//! Explicit links in citation text. These helpers perform no network or PDF work
//! and do not infer an article's identity from a citation.

use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use reqwest::Url;

const MAX_LINKS: usize = 8;
const MAX_URL_BYTES: usize = 8192;

/// Accept a public HTTP(S) destination from citation text or an untrusted search
/// result. User information and local network destinations are not source links.
/// This is a link filter, not an SSRF boundary: it does not resolve hostnames.
pub fn source_url(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > MAX_URL_BYTES
        || value
            .chars()
            .any(|ch| ch.is_control() || ch.is_whitespace())
    {
        return None;
    }
    let (scheme, rest) = value.split_once("://")?;
    if !scheme.eq_ignore_ascii_case("https") && !scheme.eq_ignore_ascii_case("http") {
        return None;
    }
    // Check the original authority too: URL normalization can remove empty
    // user information such as `https://@example.com`.
    let authority = rest.split(['/', '?', '#']).next()?;
    if authority.contains('@') || authority.contains('\\') {
        return None;
    }
    let url = Url::parse(value).ok()?;
    if !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    let host = url.host_str()?.trim_end_matches('.');
    if host.is_empty() {
        return None;
    }
    let lower = host.to_ascii_lowercase();
    if lower == "localhost"
        || lower.ends_with(".localhost")
        || lower.ends_with(".local")
        || lower == "home.arpa"
        || lower.ends_with(".home.arpa")
    {
        return None;
    }
    let ip_text = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = ip_text.parse::<IpAddr>()
        && !public_ip(ip)
    {
        return None;
    }
    Some(url.to_string())
}

fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => public_ipv4(ip),
        IpAddr::V6(ip) => public_ipv6(ip),
    }
}

fn public_ipv4(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    !ip.is_unspecified()
        && !ip.is_loopback()
        && !ip.is_private()
        && !ip.is_link_local()
        && !ip.is_multicast()
        && !ip.is_broadcast()
        && octets[0] != 0
        && octets[0] < 240
        // Shared address space is used by carrier and mesh-network addresses.
        && !(octets[0] == 100 && (64..=127).contains(&octets[1]))
}

fn public_ipv6(ip: Ipv6Addr) -> bool {
    if let Some(ipv4) = ip.to_ipv4_mapped() {
        return public_ipv4(ipv4);
    }
    let first = ip.segments()[0];
    !ip.is_unspecified()
        && !ip.is_loopback()
        && !ip.is_multicast()
        && first & 0xfe00 != 0xfc00
        && first & 0xffc0 != 0xfe80
}

/// Extract only explicit HTTP(S), `www.`, and DOI identifiers from the footnote.
/// Incomplete or ambiguous citations remain available for an explicit lookup.
/// Results preserve text order, are deduplicated, and never exceed eight links.
pub fn explicit_links(text: &str) -> Vec<String> {
    // ASCII folding preserves the byte offsets of UTF-8 text.
    let mut lower = text.to_owned();
    lower.make_ascii_lowercase();
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    let mut at = 0;
    while at < text.len() && result.len() < MAX_LINKS {
        if !text.is_char_boundary(at) {
            at += 1;
            continue;
        }
        let tail = &lower[at..];
        let kind = if tail.starts_with("https://") || tail.starts_with("http://") {
            Some(LinkKind::Web)
        } else if tail.starts_with("www.") {
            Some(LinkKind::Www)
        } else if tail.starts_with("doi:10.") {
            Some(LinkKind::DoiLabel)
        } else if tail.starts_with("10.") {
            Some(LinkKind::Doi)
        } else {
            None
        };
        let Some(kind) = kind.filter(|_| link_boundary(text, at)) else {
            at += text[at..].chars().next().map_or(1, char::len_utf8);
            continue;
        };
        let end = text[at..]
            .char_indices()
            .find_map(|(offset, ch)| token_end(ch).then_some(at + offset))
            .unwrap_or(text.len());
        if end - at > MAX_URL_BYTES {
            at = end;
            continue;
        }
        let candidate = trim_citation_punctuation(&text[at..end]);
        let candidate = match kind {
            LinkKind::Web => source_url(candidate),
            LinkKind::Www => source_url(&format!("https://{candidate}")),
            LinkKind::DoiLabel => doi_url(candidate.get(4..).unwrap_or_default()),
            LinkKind::Doi => doi_url(candidate),
        };
        if let Some(url) = candidate {
            let key = dedup_key(&url);
            if seen.insert(key) {
                result.push(url);
            }
        }
        at = end.max(at + 1);
    }
    result
}

#[derive(Clone, Copy)]
enum LinkKind {
    Web,
    Www,
    DoiLabel,
    Doi,
}

fn link_boundary(text: &str, at: usize) -> bool {
    text[..at]
        .chars()
        .next_back()
        .is_none_or(|ch| !ch.is_alphanumeric() && !matches!(ch, '_' | '/' | ':' | '.' | '-' | '@'))
}

fn token_end(ch: char) -> bool {
    ch.is_whitespace() || ch.is_control() || matches!(ch, '<' | '>' | '"' | '“' | '”' | '‘' | '’')
}

fn trim_citation_punctuation(mut value: &str) -> &str {
    let pairs = [('(', ')'), ('[', ']'), ('{', '}')];
    let mut counts = pairs.map(|(open, close)| {
        (
            value.chars().filter(|&ch| ch == open).count(),
            value.chars().filter(|&ch| ch == close).count(),
        )
    });
    while let Some(last) = value.chars().next_back() {
        if matches!(last, '.' | ',' | ';' | ':' | '\'') {
            value = &value[..value.len() - last.len_utf8()];
            continue;
        }
        let Some(pair) = pairs.iter().position(|(_, close)| *close == last) else {
            break;
        };
        if counts[pair].1 <= counts[pair].0 {
            break;
        }
        counts[pair].1 -= 1;
        value = &value[..value.len() - last.len_utf8()];
    }
    value
}

fn doi_url(value: &str) -> Option<String> {
    let (registrant, suffix) = value.strip_prefix("10.")?.split_once('/')?;
    if !(4..=9).contains(&registrant.len())
        || !registrant.bytes().all(|byte| byte.is_ascii_digit())
        || suffix.is_empty()
        || !suffix.chars().any(char::is_alphanumeric)
    {
        return None;
    }
    let mut url = Url::parse("https://doi.org/").ok()?;
    // A DOI is one identifier, so '?' and '#' are part of its path rather
    // than URL query/fragment delimiters.
    url.set_path(value);
    source_url(url.as_str())
}

fn dedup_key(value: &str) -> String {
    // DOI names are case insensitive, unlike general URL paths.
    if Url::parse(value)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .is_some_and(|host| host.eq_ignore_ascii_case("doi.org"))
    {
        value.to_ascii_lowercase()
    } else {
        value.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_urls_accept_only_web_destinations_without_user_information() {
        assert_eq!(
            source_url("HTTPS://Example.COM:443/paper?q=1#section"),
            Some("https://example.com/paper?q=1#section".to_owned())
        );
        for value in [
            "javascript:alert(1)",
            "file:///tmp/paper.pdf",
            "data:text/html,test",
            "ftp://example.com/paper",
            "https://",
            "https://@example.com/paper",
            "https://user@example.com/paper",
            "https://user:password@example.com/paper",
            "https://example.com\\@evil.example/paper",
            "https://example.com/white space",
            "https://example.com/line\nbreak",
        ] {
            assert_eq!(source_url(value), None, "{value:?}");
        }
    }

    #[test]
    fn source_urls_reject_local_and_private_destinations() {
        for value in [
            "http://localhost/paper",
            "http://LOCALHOST./paper",
            "https://app.localhost/paper",
            "http://printer.local/paper",
            "http://router.home.arpa/paper",
            "http://127.0.0.1/paper",
            "http://2130706433/paper",
            "http://0x7f000001/paper",
            "http://10.1.2.3/paper",
            "http://172.16.0.1/paper",
            "http://192.168.1.1/paper",
            "http://169.254.169.254/paper",
            "http://100.115.220.99/paper",
            "http://[::1]/paper",
            "http://[fc00::123]/paper",
            "http://[fe80::1]/paper",
            "http://[::ffff:192.168.1.1]/paper",
        ] {
            assert_eq!(source_url(value), None, "{value:?}");
        }
        assert!(source_url("https://localhost.example.com/paper").is_some());
        assert!(source_url("https://8.8.8.8/paper").is_some());
        assert!(source_url("https://[2606:4700:4700::1111]/paper").is_some());
    }

    #[test]
    fn explicit_links_preserve_balanced_parentheses_and_trim_citation_punctuation() {
        assert_eq!(
            explicit_links(
                "See (https://example.com/article_(law)). Also <www.example.org/paper>, \
                 DOI:10.1002/(SICI)1099-0739(199811)12:6(1)."
            ),
            vec![
                "https://example.com/article_(law)",
                "https://www.example.org/paper",
                "https://doi.org/10.1002/(SICI)1099-0739(199811)12:6(1)",
            ]
        );
    }

    #[test]
    fn explicit_links_accept_spaced_doi_labels_and_reliable_bare_identifiers() {
        assert_eq!(
            explicit_links("doi: 10.1234/Abc. See also 10.12345/test(foo)), at 11."),
            vec![
                "https://doi.org/10.1234/Abc",
                "https://doi.org/10.12345/test(foo)"
            ]
        );
        assert!(explicit_links("10.12/no, 10.1234567890/no, 10.1234/, v10.1234/no").is_empty());
        assert!(explicit_links("Author, Article, 123 Law Review 45 (2026)").is_empty());
        assert_eq!(
            explicit_links("doi:10.1234/foo?bar#baz"),
            vec!["https://doi.org/10.1234/foo%3Fbar%23baz"]
        );
    }

    #[test]
    fn explicit_links_deduplicate_urls_and_doi_case_without_folding_general_paths() {
        assert_eq!(
            explicit_links(
                "https://EXAMPLE.com:443/A https://example.com/A https://example.com/a \
                 doi:10.1234/AbC https://doi.org/10.1234/abc 10.1234/ABC"
            ),
            vec![
                "https://example.com/A",
                "https://example.com/a",
                "https://doi.org/10.1234/AbC",
            ]
        );
    }

    #[test]
    fn explicit_links_do_not_extract_nested_urls_or_local_links() {
        assert!(
            explicit_links(
                "javascript:https://evil.example, http://localhost/x, \
                 https://user@example.com/x, mail@example.com"
            )
            .is_empty()
        );
        assert_eq!(
            explicit_links("https://example.com/10.1234/abc"),
            vec!["https://example.com/10.1234/abc"]
        );
    }

    #[test]
    fn explicit_links_are_bounded() {
        let input = (0..30)
            .map(|number| format!("https://example.com/{number}"))
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(explicit_links(&input).len(), MAX_LINKS);
        assert_eq!(
            source_url(&format!(
                "https://example.com/{}",
                "x".repeat(MAX_URL_BYTES)
            )),
            None
        );
    }
}
