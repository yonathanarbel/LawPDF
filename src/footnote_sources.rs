//! Citation discovery is explicitly requested by the reader. One bounded worker
//! owns HTTP and cache I/O; opening a document never searches or loads this cache.

use crossbeam_channel::Sender;
use reqwest::Url;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const DEFAULT_SEARCH_URL: &str = "https://bbgpt.battleoftheforms.com/api/search";
const MAX_QUERY_CHARS: usize = 2_000;
const MAX_RESPONSE_BYTES: u64 = 1024 * 1024;
const MAX_CACHE_RECORD_BYTES: u64 = 256 * 1024;
const MAX_CACHE_FILES: usize = 256;
const MAX_CACHE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_CANDIDATES: usize = 20;
const RESULT_TTL: u64 = 14 * 24 * 60 * 60;
const EMPTY_TTL: u64 = 24 * 60 * 60;
const CACHE_VERSION: u8 = 1;

/// Candidates are discovery results, never a claim that an article matches the
/// footnote. The reader selects the appropriate source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceCandidate {
    pub title: String,
    pub url: String,
    pub authors: Vec<String>,
    pub year: Option<u16>,
    pub doi: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceSearch {
    pub candidates: Vec<SourceCandidate>,
    pub partial_errors: Vec<String>,
    pub cached: bool,
}

pub struct LookupRequest {
    pub key: String,
    pub query: String,
    /// Only an explicit reader retry bypasses the local result cache.
    pub refresh: bool,
}

pub struct LookupEvent {
    pub key: String,
    pub result: Result<SourceSearch, String>,
}

/// Allow a deployment or QA server override without accepting embedded secrets,
/// fragments, or preloaded query parameters. The HTTP override supports local
/// protocol tests; source-result links still use the stricter public-link filter.
pub fn configured_endpoint() -> Result<String, String> {
    let value = std::env::var("LAWPDF_CITATION_SEARCH_URL")
        .unwrap_or_else(|_| DEFAULT_SEARCH_URL.to_owned());
    validated_endpoint(&value).map(|url| url.to_string())
}

fn validated_endpoint(value: &str) -> Result<Url, String> {
    let invalid = || {
        "The citation search address must be an HTTP(S) URL without credentials, query parameters, or a fragment.".to_owned()
    };
    let value = value.trim();
    if value.len() > 8_192
        || value
            .chars()
            .any(|ch| ch.is_control() || ch.is_whitespace())
    {
        return Err(invalid());
    }
    let (_, authority) = value.split_once("://").ok_or_else(invalid)?;
    if authority
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .contains(['@', '\\'])
    {
        return Err(invalid());
    }
    let url = Url::parse(value).map_err(|_| invalid())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid());
    }
    Ok(url)
}

pub fn normalize_query(text: &str) -> String {
    let mut output = String::new();
    let mut length = 0;
    let mut pending_space = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            pending_space = !output.is_empty();
        } else if !ch.is_control() {
            if pending_space && length < MAX_QUERY_CHARS {
                output.push(' ');
                length += 1;
            }
            pending_space = false;
            if length == MAX_QUERY_CHARS {
                break;
            }
            output.push(ch);
            length += 1;
        }
        if length == MAX_QUERY_CHARS {
            break;
        }
    }
    output.trim_end().to_owned()
}

pub fn query_key(endpoint: &str, query: &str) -> String {
    let endpoint = validated_endpoint(endpoint)
        .map(|url| url.to_string())
        .unwrap_or_else(|_| endpoint.to_owned());
    let payload = serde_json::json!({
        "version": CACHE_VERSION,
        "endpoint": endpoint,
        "query": normalize_query(query).to_lowercase(),
        "engines": "crossref",
        "use_llm": false,
        "top_per_engine": 5,
        "timeout": 10,
    });
    format!("{:x}", Sha256::digest(payload.to_string().as_bytes()))
}

/// Callers enqueue with `try_send`; the sixteenth queued item is the last one
/// accepted while a lookup is in progress. Dropping every sender ends the worker.
pub fn spawn_lookup_worker(
    endpoint: String,
    tx: Sender<LookupEvent>,
    ctx: egui::Context,
) -> Sender<LookupRequest> {
    let (request_tx, request_rx) = crossbeam_channel::bounded::<LookupRequest>(16);
    let _ = std::thread::Builder::new()
        .name("citation-source-search".to_owned())
        .spawn(move || {
            let cache_dir =
                crate::settings::app_data_dir().map(|root| root.join("citation-sources"));
            let service = LookupService::new(&endpoint, cache_dir);
            for request in request_rx {
                let result = service.lookup(&request.query, request.refresh);
                if tx
                    .send(LookupEvent {
                        key: request.key,
                        result,
                    })
                    .is_err()
                {
                    break;
                }
                ctx.request_repaint();
            }
        });
    request_tx
}

/// An explicit command-line diagnostic uses the same bounded HTTP path as the
/// reader, with no persistent cache and no document access.
pub fn probe(query: &str) -> Result<SourceSearch, String> {
    let endpoint = configured_endpoint()?;
    LookupService::new(&endpoint, None).lookup(query, true)
}

struct LookupService {
    endpoint: Result<Url, String>,
    client: Result<Client, String>,
    cache_dir: Option<PathBuf>,
}

impl LookupService {
    fn new(endpoint: &str, cache_dir: Option<PathBuf>) -> Self {
        Self {
            endpoint: validated_endpoint(endpoint),
            client: Client::builder()
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(15))
                .redirect(reqwest::redirect::Policy::none())
                .user_agent(concat!(
                    "LawPDF/",
                    env!("CARGO_PKG_VERSION"),
                    " citation-sources"
                ))
                .build()
                .map_err(|_| "Could not initialize citation search.".to_owned()),
            cache_dir,
        }
    }

    fn lookup(&self, query: &str, refresh: bool) -> Result<SourceSearch, String> {
        let query = normalize_query(query);
        if query.is_empty() {
            return Err("This footnote has no text to search.".to_owned());
        }
        let endpoint = self.endpoint.as_ref().map_err(Clone::clone)?;
        let key = query_key(endpoint.as_str(), &query);
        let now = unix_seconds();
        if !refresh
            && let Some(cached) = self
                .cache_dir
                .as_deref()
                .and_then(|root| read_cache(root, &key, now))
        {
            return Ok(cached);
        }
        let client = self.client.as_ref().map_err(Clone::clone)?;
        let response = client
            .get(endpoint.clone())
            .query(&[
                ("q", query.as_str()),
                ("engines", "crossref"),
                ("use_llm", "false"),
                ("top_per_engine", "5"),
                ("timeout", "10"),
            ])
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .map_err(http_error)?;
        if !response.status().is_success() {
            // Provider bodies and reqwest errors can echo query text, credentials,
            // or internal addresses. Never put them in a reader-facing error.
            return Err(format!(
                "Citation search returned HTTP {}. Try again later.",
                response.status().as_u16()
            ));
        }
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if content_type != "application/json" && !content_type.ends_with("+json") {
            return Err("Citation search did not return a JSON result.".to_owned());
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_RESPONSE_BYTES)
        {
            return Err("Citation search returned too much data.".to_owned());
        }
        let bytes = read_bounded(response, MAX_RESPONSE_BYTES)
            .map_err(|_| "Citation search returned too much or incomplete data.".to_owned())?;
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|_| "Citation search returned an unreadable result.".to_owned())?;
        let result = parse_response(&value)?;
        if result.partial_errors.is_empty() {
            if let Some(root) = self.cache_dir.as_deref() {
                write_cache(root, &key, &result, now);
            }
        }
        Ok(result)
    }
}

fn http_error(error: reqwest::Error) -> String {
    if error.is_timeout() {
        "Citation search timed out. Try this footnote again later."
    } else if error.is_connect() {
        "Could not connect to citation search. Check your connection and try again."
    } else {
        "Citation search could not complete the request. Try again later."
    }
    .to_owned()
}

fn read_bounded(reader: impl Read, maximum: u64) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.take(maximum + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > maximum {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "size limit",
        ));
    }
    Ok(bytes)
}

fn text_field(value: &Value, maximum: usize) -> Option<String> {
    let text = value
        .as_str()?
        .trim()
        .chars()
        .filter(|ch| !ch.is_control())
        .take(maximum)
        .collect::<String>();
    (!text.is_empty()).then_some(text)
}

fn doi_field(value: &Value) -> Option<String> {
    let text = value.as_str()?.trim();
    let doi = text
        .strip_prefix("https://doi.org/")
        .or_else(|| text.strip_prefix("http://doi.org/"))
        .or_else(|| text.strip_prefix("doi:"))
        .unwrap_or(text)
        .trim();
    let (prefix, suffix) = doi.split_once('/')?;
    let registrant = prefix.strip_prefix("10.")?;
    if doi.len() > 2_048
        || suffix.is_empty()
        || !(4..=9).contains(&registrant.len())
        || !registrant.bytes().all(|ch| ch.is_ascii_digit())
        || doi.chars().any(|ch| ch.is_control() || ch.is_whitespace())
    {
        return None;
    }
    Some(doi.to_owned())
}

fn doi_source_url(doi: &str) -> Option<String> {
    let mut url = Url::parse("https://doi.org/").ok()?;
    // '?' and '#' can belong to the DOI itself. Keep the complete identifier
    // in the URL path instead of turning its tail into a query or fragment.
    url.set_path(doi);
    crate::citation_links::source_url(url.as_str())
}

fn parse_response(value: &Value) -> Result<SourceSearch, String> {
    if value.get("success").and_then(Value::as_bool) != Some(true) {
        return Err("Citation search could not complete the lookup. Try again later.".to_owned());
    }
    let results = value
        .get("results")
        .and_then(Value::as_array)
        .ok_or_else(|| "Citation search returned an unreadable result.".to_owned())?;
    let mut candidates = Vec::new();
    let mut seen = HashSet::new();
    for item in results.iter().take(100) {
        let Some(title) = item.get("title").and_then(|value| text_field(value, 600)) else {
            continue;
        };
        let doi = item.get("doi").and_then(doi_field);
        let url = item
            .get("url")
            .and_then(Value::as_str)
            .and_then(crate::citation_links::source_url)
            .or_else(|| doi.as_deref().and_then(doi_source_url));
        let Some(url) = url else { continue };
        let identity = doi
            .as_ref()
            .map(|doi| format!("doi:{}", doi.to_lowercase()))
            .unwrap_or_else(|| url.clone());
        if !seen.insert(identity) {
            continue;
        }
        let authors = match item.get("authors") {
            Some(Value::Array(authors)) => authors
                .iter()
                .take(16)
                .filter_map(|author| text_field(author, 160))
                .collect(),
            Some(author) => text_field(author, 160).into_iter().collect(),
            None => Vec::new(),
        };
        let year = item
            .get("year")
            .and_then(|year| {
                year.as_u64()
                    .or_else(|| year.as_str().and_then(|year| year.parse::<u64>().ok()))
            })
            .filter(|year| (1_000..=9_999).contains(year))
            .map(|year| year as u16);
        candidates.push(SourceCandidate {
            title,
            url,
            authors,
            year,
            doi,
        });
        if candidates.len() == MAX_CANDIDATES {
            break;
        }
    }
    let mut partial_errors = Vec::new();
    if let Some(errors) = value.get("engine_errors").and_then(Value::as_object) {
        for (engine, error) in errors.iter().take(12) {
            if !error.is_null() {
                partial_errors.push(format!("{} search did not complete.", engine_label(engine)));
            }
        }
    }
    if let Some(deferred) = value.get("deferred_engines").and_then(Value::as_array) {
        for engine in deferred.iter().take(12).filter_map(Value::as_str) {
            partial_errors.push(format!("{} search was deferred.", engine_label(engine)));
        }
    }
    if value
        .get("error")
        .is_some_and(|error| !error.is_null() && error != "")
    {
        partial_errors.push("The search service reported an incomplete result.".to_owned());
    }
    partial_errors.sort();
    partial_errors.dedup();
    Ok(SourceSearch {
        candidates,
        partial_errors,
        cached: value
            .get("cached")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

fn engine_label(engine: &str) -> &'static str {
    match engine.to_ascii_lowercase().as_str() {
        "crossref" => "CrossRef",
        "openalex" => "OpenAlex",
        "ssrn" => "SSRN",
        "hollis" => "HOLLIS",
        "courtlistener" => "CourtListener",
        "annas" => "Bibliographic",
        _ => "An upstream",
    }
}

#[derive(Serialize, Deserialize)]
struct CacheRecord {
    version: u8,
    key: String,
    saved_at: u64,
    expires_at: u64,
    result: SourceSearch,
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn valid_key(key: &str) -> bool {
    key.len() == 64 && key.bytes().all(|ch| ch.is_ascii_hexdigit())
}

fn cache_path(root: &Path, key: &str) -> Option<PathBuf> {
    valid_key(key).then(|| root.join(format!("{key}.json")))
}

fn read_cache(root: &Path, key: &str, now: u64) -> Option<SourceSearch> {
    let path = cache_path(root, key)?;
    if !fs::symlink_metadata(&path).ok()?.file_type().is_file() {
        return None;
    }
    let bytes = read_bounded(fs::File::open(&path).ok()?, MAX_CACHE_RECORD_BYTES).ok()?;
    let record: CacheRecord = serde_json::from_slice(&bytes).ok()?;
    let ttl = if record.result.candidates.is_empty() {
        EMPTY_TTL
    } else {
        RESULT_TTL
    };
    if record.version != CACHE_VERSION
        || record.key != key
        || record.saved_at > now.saturating_add(300)
        || record.expires_at <= now
        || record.expires_at > record.saved_at.saturating_add(ttl)
        || !record.result.partial_errors.is_empty()
        || record.result.candidates.len() > MAX_CANDIDATES
        || record
            .result
            .candidates
            .iter()
            .any(|item| crate::citation_links::source_url(&item.url).is_none())
    {
        return None;
    }
    let mut result = record.result;
    result.cached = true;
    Some(result)
}

fn write_cache(root: &Path, key: &str, result: &SourceSearch, now: u64) {
    if !result.partial_errors.is_empty() {
        return;
    }
    let Some(path) = cache_path(root, key) else {
        return;
    };
    if fs::create_dir_all(root).is_err() {
        return;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if fs::set_permissions(root, fs::Permissions::from_mode(0o700)).is_err() {
            return;
        }
    }
    let ttl = if result.candidates.is_empty() {
        EMPTY_TTL
    } else {
        RESULT_TTL
    };
    let record = CacheRecord {
        version: CACHE_VERSION,
        key: key.to_owned(),
        saved_at: now,
        expires_at: now.saturating_add(ttl),
        result: result.clone(),
    };
    let Ok(bytes) = serde_json::to_vec(&record) else {
        return;
    };
    if bytes.len() as u64 > MAX_CACHE_RECORD_BYTES {
        return;
    }
    let Ok(mut file) = tempfile::Builder::new().prefix("source-").tempfile_in(root) else {
        return;
    };
    if file.write_all(&bytes).is_err() || file.flush().is_err() {
        return;
    }
    if file.persist(path).is_ok() {
        prune_cache(root);
    }
}

fn prune_cache(root: &Path) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    let mut files = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            if !entry.file_type().ok()?.is_file() {
                return None;
            }
            let name = entry.file_name();
            let key = name.to_str()?.strip_suffix(".json")?;
            if !valid_key(key) {
                return None;
            }
            let metadata = entry.metadata().ok()?;
            Some((
                metadata.modified().unwrap_or(UNIX_EPOCH),
                entry.path(),
                metadata.len(),
            ))
        })
        .collect::<Vec<_>>();
    let mut bytes = files.iter().map(|file| file.2).sum::<u64>();
    let mut count = files.len();
    files.sort_by_key(|file| file.0);
    for (_, path, length) in files {
        if count <= MAX_CACHE_FILES && bytes <= MAX_CACHE_BYTES {
            break;
        }
        if fs::remove_file(path).is_ok() {
            count -= 1;
            bytes = bytes.saturating_sub(length);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;

    fn mock_response(
        status: &str,
        body: Vec<u8>,
        content_length: Option<u64>,
    ) -> (
        String,
        std::sync::mpsc::Receiver<String>,
        thread::JoinHandle<()>,
    ) {
        mock_responses(vec![(status.to_owned(), body, content_length)])
    }

    fn mock_responses(
        responses: Vec<(String, Vec<u8>, Option<u64>)>,
    ) -> (
        String,
        std::sync::mpsc::Receiver<String>,
        thread::JoinHandle<()>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/api/search", listener.local_addr().unwrap());
        let (tx, rx) = std::sync::mpsc::channel();
        let handle = thread::spawn(move || {
            for (status, body, content_length) in responses {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0_u8; 2_048];
                while request.len() < 16_384 {
                    let length = stream.read(&mut buffer).unwrap();
                    if length == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..length]);
                    if request.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                let _ = tx.send(String::from_utf8(request).unwrap());
                let headers = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    content_length.unwrap_or(body.len() as u64)
                );
                let _ = stream.write_all(headers.as_bytes());
                let _ = stream.write_all(&body);
            }
        });
        (endpoint, rx, handle)
    }

    fn candidate() -> SourceCandidate {
        SourceCandidate {
            title: "An Article".to_owned(),
            url: "https://example.org/article".to_owned(),
            authors: vec!["A. Scholar".to_owned()],
            year: Some(2024),
            doi: None,
        }
    }

    #[test]
    fn queries_are_bounded_and_cache_keys_include_deployment_and_configuration() {
        assert_eq!(normalize_query("  One\n two\tthree\0 "), "One two three");
        assert_eq!(
            normalize_query(&"é".repeat(3_000)).chars().count(),
            MAX_QUERY_CHARS
        );
        assert_eq!(
            query_key(DEFAULT_SEARCH_URL, " One\nTwo "),
            query_key(DEFAULT_SEARCH_URL, "one two")
        );
        assert_ne!(
            query_key(DEFAULT_SEARCH_URL, "one"),
            query_key("https://other.example/api/search", "one")
        );
        assert!(valid_key(&query_key(DEFAULT_SEARCH_URL, "one")));
        assert!(cache_path(Path::new("cache"), "../../other").is_none());
    }

    #[test]
    fn endpoints_reject_secrets_parameters_and_non_http_schemes() {
        for invalid in [
            "file:///tmp/data",
            "https://user:secret@example.org/api",
            "https://@example.org/api",
            "https://example.org/api?key=secret",
            "https://example.org/api#key",
            "https://example.org/a\nb",
        ] {
            assert!(validated_endpoint(invalid).is_err(), "{invalid}");
        }
        assert!(validated_endpoint(DEFAULT_SEARCH_URL).is_ok());
        assert!(validated_endpoint("http://127.0.0.1:8000/api/search").is_ok());
    }

    #[test]
    fn protocol_uses_one_get_with_explicit_deterministic_parameters_and_caches_duplicates() {
        let body = serde_json::json!({"success":true,"results":[{"title":"A title","authors":["A. Scholar"],"year":"2024","doi":"10.1234/article","url":"https://doi.org/10.1234/article"}]}).to_string().into_bytes();
        let (endpoint, requests, server) = mock_response("200 OK", body, None);
        let cache = tempfile::tempdir().unwrap();
        let service = LookupService::new(&endpoint, Some(cache.path().to_owned()));
        let first = service.lookup("A title & another", false).unwrap();
        assert!(!first.cached);
        assert_eq!(first.candidates[0].year, Some(2024));
        let request = requests.recv_timeout(Duration::from_secs(3)).unwrap();
        let first_line = request.lines().next().unwrap();
        assert!(first_line.starts_with("GET /api/search?"));
        let request_url = Url::parse(&format!(
            "http://localhost{}",
            first_line.split_whitespace().nth(1).unwrap()
        ))
        .unwrap();
        let parameters = request_url
            .query_pairs()
            .collect::<std::collections::HashMap<_, _>>();
        for (key, value) in [
            ("q", "A title & another"),
            ("engines", "crossref"),
            ("use_llm", "false"),
            ("top_per_engine", "5"),
            ("timeout", "10"),
        ] {
            assert_eq!(parameters.get(key).unwrap(), value);
        }
        server.join().unwrap();
        let second = service.lookup("  A title & another\n", false).unwrap();
        assert!(second.cached);
        assert_eq!(first.candidates, second.candidates);
    }

    #[test]
    fn malformed_and_oversized_responses_are_bounded_and_never_cached() {
        for (body, length) in [
            (b"not json".to_vec(), None),
            (Vec::new(), Some(MAX_RESPONSE_BYTES + 1)),
        ] {
            let (endpoint, _, server) = mock_response("200 OK", body, length);
            let cache = tempfile::tempdir().unwrap();
            let error = LookupService::new(&endpoint, Some(cache.path().to_owned()))
                .lookup("title", false)
                .unwrap_err();
            assert!(!error.contains(&endpoint));
            assert_eq!(fs::read_dir(cache.path()).unwrap().count(), 0);
            server.join().unwrap();
        }
        assert!(read_bounded(&[0_u8; 100][..], 20).is_err());
    }

    #[test]
    fn an_explicit_refresh_can_replace_a_cached_empty_result() {
        let empty = serde_json::json!({"success":true,"results":[]})
            .to_string()
            .into_bytes();
        let found = serde_json::json!({"success":true,"results":[{"title":"Found article","url":"https://example.org/found"}]}).to_string().into_bytes();
        let (endpoint, requests, server) = mock_responses(vec![
            ("200 OK".to_owned(), empty, None),
            ("200 OK".to_owned(), found, None),
        ]);
        let cache = tempfile::tempdir().unwrap();
        let service = LookupService::new(&endpoint, Some(cache.path().to_owned()));
        assert!(
            service
                .lookup("article", false)
                .unwrap()
                .candidates
                .is_empty()
        );
        let cached = service.lookup("article", false).unwrap();
        assert!(cached.cached && cached.candidates.is_empty());
        let refreshed = service.lookup("article", true).unwrap();
        assert!(!refreshed.cached);
        assert_eq!(refreshed.candidates[0].title, "Found article");
        server.join().unwrap();
        assert_eq!(requests.try_iter().count(), 2);
        assert_eq!(
            service.lookup("article", false).unwrap().candidates,
            refreshed.candidates
        );
    }

    #[test]
    fn provider_errors_do_not_expose_their_body_or_query() {
        let secret = "token=never-display-this-query";
        let (endpoint, _, server) =
            mock_response("429 Too Many Requests", secret.as_bytes().to_vec(), None);
        let error = LookupService::new(&endpoint, None)
            .lookup(secret, false)
            .unwrap_err();
        assert!(error.contains("429"));
        assert!(!error.contains(secret));
        server.join().unwrap();
        let value = serde_json::json!({"success": false, "error": secret});
        assert!(!parse_response(&value).unwrap_err().contains(secret));
    }

    #[test]
    fn partial_failures_remain_distinct_from_no_match_and_are_not_cached() {
        let value = serde_json::json!({"success":true,"results":[],"engine_errors":{"crossref":"internal credential=secret"},"deferred_engines":["ssrn"]});
        let result = parse_response(&value).unwrap();
        assert!(result.candidates.is_empty());
        assert_eq!(result.partial_errors.len(), 2);
        assert!(!result.partial_errors.join(" ").contains("secret"));
        let cache = tempfile::tempdir().unwrap();
        write_cache(cache.path(), &"a".repeat(64), &result, 100);
        assert_eq!(fs::read_dir(cache.path()).unwrap().count(), 0);
    }

    #[test]
    fn unsafe_results_are_dropped_and_repeated_dois_deduplicated() {
        let result = parse_response(&serde_json::json!({"success":true,"results":[
            {"title":"Bad","url":"javascript:alert(1)"},
            {"title":"Local","url":"http://127.0.0.1/private"},
            {"title":"Source","doi":"10.1234/ARTICLE","url":"https://example.org/first"},
            {"title":"Duplicate","doi":"10.1234/article","url":"https://example.org/second"},
            {"title":"DOI only","doi":"10.5555/other"}
        ]}))
        .unwrap();
        assert_eq!(result.candidates.len(), 2);
        assert_eq!(result.candidates[1].url, "https://doi.org/10.5555/other");
    }

    #[test]
    fn doi_only_results_preserve_query_and_fragment_characters_in_the_identifier() {
        let result = parse_response(&serde_json::json!({"success":true,"results":[
            {"title":"An article","doi":"10.1234/article?version#part"}
        ]}))
        .unwrap();
        assert_eq!(result.candidates.len(), 1);
        assert_eq!(
            result.candidates[0].url,
            "https://doi.org/10.1234/article%3Fversion%23part"
        );
        let url = Url::parse(&result.candidates[0].url).unwrap();
        assert!(url.query().is_none());
        assert!(url.fragment().is_none());
    }

    #[test]
    fn positive_and_empty_cache_entries_expire_on_their_respective_deadlines() {
        let cache = tempfile::tempdir().unwrap();
        let positive_key = "a".repeat(64);
        let empty_key = "b".repeat(64);
        let result = SourceSearch {
            candidates: vec![candidate()],
            partial_errors: Vec::new(),
            cached: false,
        };
        let empty = SourceSearch {
            candidates: Vec::new(),
            ..result.clone()
        };
        write_cache(cache.path(), &positive_key, &result, 1_000);
        write_cache(cache.path(), &empty_key, &empty, 1_000);
        assert!(read_cache(cache.path(), &positive_key, 1_000 + RESULT_TTL - 1).is_some());
        assert!(read_cache(cache.path(), &positive_key, 1_000 + RESULT_TTL).is_none());
        assert!(read_cache(cache.path(), &empty_key, 1_000 + EMPTY_TTL - 1).is_some());
        assert!(read_cache(cache.path(), &empty_key, 1_000 + EMPTY_TTL).is_none());
    }

    #[test]
    fn cache_pruning_is_bounded_and_leaves_unrelated_files_alone() {
        let cache = tempfile::tempdir().unwrap();
        fs::write(cache.path().join("keep.txt"), "untouched").unwrap();
        for index in 0..MAX_CACHE_FILES + 4 {
            fs::write(cache.path().join(format!("{index:064x}.json")), "{}").unwrap();
        }
        prune_cache(cache.path());
        assert_eq!(
            fs::read_dir(cache.path()).unwrap().count(),
            MAX_CACHE_FILES + 1
        );
        assert_eq!(
            fs::read_to_string(cache.path().join("keep.txt")).unwrap(),
            "untouched"
        );
    }
}
