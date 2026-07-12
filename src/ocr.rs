use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crossbeam_channel::Sender;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::model::OcrPageState;
use crate::pdf_backend::{PdfEngine, save_with_ocr_text};
use crate::render_worker::RenderRequest;
use crate::settings::app_data_dir;

const OCR_CACHE_SCHEMA_VERSION: u32 = 1;
const OPENROUTER_URL: &str = "https://openrouter.ai/api/v1/chat/completions";
const OPENROUTER_OCR_MODEL: &str = "baidu/qianfan-ocr-fast:free";
const OCR_RENDER_SCALE: f32 = 2.2;
const LOCAL_TESSERACT_PSM: &str = "4";
const OPENROUTER_OCR_BATCH_PAGES: usize = 3;
const OPENROUTER_REQUEST_SPACING: Duration = Duration::from_millis(3200);
const OPENROUTER_MAX_RETRIES: usize = 3;
const LOCAL_TESSERACT_TIMEOUT: Duration = Duration::from_secs(90);

#[derive(Debug, Clone)]
pub struct OcrEvent {
    pub document_epoch: u64,
    pub path: PathBuf,
    pub page_index: usize,
    pub state: OcrPageState,
    pub status: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OcrCache {
    schema_version: u32,
    page_count: usize,
    pages: Vec<Option<String>>,
}

pub fn spawn_ocr_job(
    document_epoch: u64,
    pdf_path: PathBuf,
    page_count: usize,
    tx: Sender<OcrEvent>,
    render_tx: Sender<RenderRequest>,
) {
    thread::spawn(move || {
        for page_index in 0..page_count {
            let _ = tx.send(OcrEvent {
                document_epoch,
                path: pdf_path.clone(),
                page_index,
                state: OcrPageState::Running,
                status: None,
            });

            let state = match ocr_page(&render_tx, &pdf_path, page_index) {
                Ok(text) => OcrPageState::Done(text),
                Err(error) => OcrPageState::Failed(error),
            };

            let _ = tx.send(OcrEvent {
                document_epoch,
                path: pdf_path.clone(),
                page_index,
                state,
                status: None,
            });
        }
    });
}

pub fn load_ocr_cache(path: &Path, page_count: usize) -> Option<Vec<OcrPageState>> {
    let cache_path = ocr_cache_path(path)?;
    let bytes = std::fs::read(cache_path).ok()?;
    let cache = serde_json::from_slice::<OcrCache>(&bytes).ok()?;
    if cache.schema_version != OCR_CACHE_SCHEMA_VERSION
        || cache.page_count != page_count
        || cache.pages.len() != page_count
    {
        return None;
    }

    Some(
        cache
            .pages
            .into_iter()
            .map(|page| match page {
                Some(text) if !text.trim().is_empty() => OcrPageState::Done(text),
                _ => OcrPageState::Idle,
            })
            .collect(),
    )
}

pub fn save_ocr_cache(path: &Path, states: &[OcrPageState]) -> Result<(), String> {
    let cache_path =
        ocr_cache_path(path).ok_or_else(|| "Could not find OCR cache directory.".to_owned())?;
    if let Some(parent) = cache_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create OCR cache: {error}"))?;
    }

    let pages = states
        .iter()
        .map(|state| state.text().map(str::to_owned))
        .collect::<Vec<_>>();
    let cache = OcrCache {
        schema_version: OCR_CACHE_SCHEMA_VERSION,
        page_count: states.len(),
        pages,
    };
    let bytes = serde_json::to_vec_pretty(&cache)
        .map_err(|error| format!("Could not encode OCR cache: {error}"))?;
    std::fs::write(cache_path, bytes).map_err(|error| format!("Could not save OCR cache: {error}"))
}

pub fn ocr_page_with_engine(
    engine: &PdfEngine,
    pdf_path: &Path,
    page_index: usize,
) -> Result<String, String> {
    ocr_page_with_engine_scale(engine, pdf_path, page_index, OCR_RENDER_SCALE)
}

pub fn ocr_page_with_engine_scale(
    engine: &PdfEngine,
    pdf_path: &Path,
    page_index: usize,
    scale: f32,
) -> Result<String, String> {
    let image_path = temp_image_path(page_index);
    let export_result = engine
        .export_page_png(pdf_path, page_index, &image_path, scale)
        .map_err(|error| format!("Could not render OCR page image: {error:#}"));
    if let Err(error) = export_result {
        let _ = std::fs::remove_file(&image_path);
        return Err(error);
    }

    let result = run_tesseract_image(&image_path);
    let _ = std::fs::remove_file(&image_path);
    result
}

pub fn spawn_openrouter_ocr_save_job(
    document_epoch: u64,
    pdf_path: PathBuf,
    page_count: usize,
    page_sizes: Vec<(f32, f32)>,
    destination: PathBuf,
    initial_ocr_text: Vec<Option<String>>,
    openrouter_api_key: String,
    tx: Sender<OcrEvent>,
    render_tx: Sender<RenderRequest>,
) {
    thread::spawn(move || {
        let client = match reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
        {
            Ok(client) => client,
            Err(error) => {
                send_ocr_status(
                    &tx,
                    document_epoch,
                    &pdf_path,
                    format!("Could not create OpenRouter OCR client: {error}"),
                );
                return;
            }
        };

        let status_target = OcrStatusTarget {
            tx: &tx,
            document_epoch,
            pdf_path: &pdf_path,
        };
        let mut ocr_text = initial_ocr_text
            .into_iter()
            .take(page_count)
            .map(|text| text.unwrap_or_default())
            .collect::<Vec<_>>();
        ocr_text.resize(page_count, String::new());
        let pages_to_ocr = ocr_text
            .iter()
            .enumerate()
            .filter_map(|(page_index, text)| {
                if text.trim().is_empty() {
                    Some(page_index)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        let cached = page_count.saturating_sub(pages_to_ocr.len());
        let mut failed = 0usize;
        let mut completed_requested = 0usize;
        let mut last_request_at = None;

        if cached > 0 {
            send_ocr_status(
                &tx,
                document_epoch,
                &pdf_path,
                format!(
                    "Using cached OCR for {cached} page(s); {} page(s) left.",
                    pages_to_ocr.len()
                ),
            );
        }

        for batch in pages_to_ocr.chunks(OPENROUTER_OCR_BATCH_PAGES) {
            for page_index in batch {
                let _ = tx.send(OcrEvent {
                    document_epoch,
                    path: pdf_path.clone(),
                    page_index: *page_index,
                    state: OcrPageState::Running,
                    status: Some(format!(
                        "Preparing OCR page {} of {}...",
                        page_index + 1,
                        page_count
                    )),
                });
            }

            match openrouter_ocr_pages(
                &client,
                &render_tx,
                &pdf_path,
                batch,
                &openrouter_api_key,
                &mut last_request_at,
                &status_target,
            ) {
                Ok(texts) => {
                    for (page_index, text) in batch.iter().copied().zip(texts) {
                        completed_requested += 1;
                        ocr_text[page_index] = text.clone();
                        let _ = tx.send(OcrEvent {
                            document_epoch,
                            path: pdf_path.clone(),
                            page_index,
                            state: OcrPageState::Done(text),
                            status: Some(format!(
                                "OCR page {} ready ({}/{})",
                                page_index + 1,
                                completed_requested,
                                pages_to_ocr.len()
                            )),
                        });
                    }
                }
                Err(error) => {
                    for page_index in batch {
                        failed += 1;
                        let _ = tx.send(OcrEvent {
                            document_epoch,
                            path: pdf_path.clone(),
                            page_index: *page_index,
                            state: OcrPageState::Failed(error.clone()),
                            status: None,
                        });
                    }
                }
            }
        }

        if failed > 0 {
            send_ocr_status(
                &tx,
                document_epoch,
                &pdf_path,
                format!("OpenRouter OCR stopped; {failed} page(s) failed."),
            );
            return;
        }

        send_ocr_status(
            &tx,
            document_epoch,
            &pdf_path,
            format!("Embedding OCR text in {}", destination.display()),
        );
        match save_with_ocr_text(&pdf_path, &destination, &page_sizes, &ocr_text) {
            Ok(()) => send_ocr_status(
                &tx,
                document_epoch,
                &pdf_path,
                format!("Saved OCR PDF {}", destination.display()),
            ),
            Err(error) => send_ocr_status(
                &tx,
                document_epoch,
                &pdf_path,
                format!("Could not save OCR PDF: {error}"),
            ),
        }
    });
}

fn ocr_page(
    render_tx: &Sender<RenderRequest>,
    pdf_path: &PathBuf,
    page_index: usize,
) -> Result<String, String> {
    let image_path = temp_image_path(page_index);
    export_ocr_image(render_tx, pdf_path.clone(), page_index, image_path.clone())?;

    let result = run_tesseract_image(&image_path);
    let _ = std::fs::remove_file(&image_path);
    result
}

fn run_tesseract_image(image_path: &Path) -> Result<String, String> {
    let mut child = Command::new("tesseract")
        .arg(&image_path)
        .arg("stdout")
        .arg("-l")
        .arg("eng")
        .arg("--psm")
        .arg(LOCAL_TESSERACT_PSM)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
        format!(
            "failed to run tesseract; install Tesseract OCR and make sure it is on PATH: {error}"
        )
    })?;

    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => break,
            Ok(None) => {
                if started.elapsed() >= LOCAL_TESSERACT_TIMEOUT {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!(
                        "tesseract timed out after {} seconds",
                        LOCAL_TESSERACT_TIMEOUT.as_secs()
                    ));
                }
                thread::sleep(Duration::from_millis(100));
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("failed while waiting for tesseract: {error}"));
            }
        }
    }

    let output = child
        .wait_with_output()
        .map_err(|error| format!("failed to read tesseract output: {error}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(if stderr.is_empty() {
            format!("tesseract exited with {}", output.status)
        } else {
            stderr
        });
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn export_ocr_image(
    render_tx: &Sender<RenderRequest>,
    pdf_path: PathBuf,
    page_index: usize,
    image_path: PathBuf,
) -> Result<(), String> {
    let (reply_tx, reply_rx) = crossbeam_channel::unbounded();
    render_tx
        .send(RenderRequest::ExportPagePng {
            path: pdf_path,
            page_index,
            destination: image_path,
            scale: OCR_RENDER_SCALE,
            reply: reply_tx,
        })
        .map_err(|error| format!("PDF worker is not available: {error}"))?;
    reply_rx
        .recv_timeout(Duration::from_secs(120))
        .map_err(|error| format!("timed out rendering OCR page image: {error}"))?
}

fn temp_image_path(page_index: usize) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();

    std::env::temp_dir().join(format!(
        "rust_pdf_editor_ocr_{stamp}_p{}.png",
        page_index + 1
    ))
}

struct OcrStatusTarget<'a> {
    tx: &'a Sender<OcrEvent>,
    document_epoch: u64,
    pdf_path: &'a PathBuf,
}

fn openrouter_ocr_pages(
    client: &reqwest::blocking::Client,
    render_tx: &Sender<RenderRequest>,
    pdf_path: &PathBuf,
    page_indices: &[usize],
    api_key: &str,
    last_request_at: &mut Option<Instant>,
    status_target: &OcrStatusTarget<'_>,
) -> Result<Vec<String>, String> {
    if page_indices.is_empty() {
        return Ok(Vec::new());
    }

    let page_list = page_indices
        .iter()
        .map(|page_index| (page_index + 1).to_string())
        .collect::<Vec<_>>()
        .join(", ");
    send_ocr_status(
        status_target.tx,
        status_target.document_epoch,
        status_target.pdf_path,
        format!("OpenRouter OCR pages {page_list}..."),
    );

    let mut content = vec![json!({
        "type": "text",
        "text": format!(
            "Extract every readable word from each attached PDF page image. Preserve natural line breaks and reading order. Return only valid JSON in this exact shape: {{\"pages\":[{{\"page\":{},\"text\":\"...\"}}]}}. Include one object for each of these page numbers, in order: {page_list}. Do not summarize or add commentary.",
            page_indices[0] + 1
        )
    })];
    for page_index in page_indices {
        content.push(json!({
            "type": "text",
            "text": format!("PDF page {}", page_index + 1)
        }));
        content.push(json!({
            "type": "image_url",
            "image_url": { "url": page_data_url(render_tx, pdf_path, *page_index)? }
        }));
    }

    let body = json!({
        "model": OPENROUTER_OCR_MODEL,
        "messages": [
            {
                "role": "user",
                "content": content
            }
        ],
        "temperature": 0
    });

    let response_text =
        post_openrouter_ocr(client, api_key, &body, last_request_at, Some(status_target))?;
    match parse_ocr_batch_response(&response_text, page_indices) {
        Ok(texts) => Ok(texts),
        Err(error) if page_indices.len() > 1 => {
            send_ocr_status(
                status_target.tx,
                status_target.document_epoch,
                status_target.pdf_path,
                format!("OCR batch parse failed; retrying pages one at a time. {error}"),
            );
            page_indices
                .iter()
                .map(|page_index| {
                    openrouter_ocr_single_page(
                        client,
                        render_tx,
                        pdf_path,
                        *page_index,
                        api_key,
                        last_request_at,
                        status_target,
                    )
                })
                .collect()
        }
        Err(error) => Err(error),
    }
}

fn openrouter_ocr_single_page(
    client: &reqwest::blocking::Client,
    render_tx: &Sender<RenderRequest>,
    pdf_path: &PathBuf,
    page_index: usize,
    api_key: &str,
    last_request_at: &mut Option<Instant>,
    status_target: &OcrStatusTarget<'_>,
) -> Result<String, String> {
    send_ocr_status(
        status_target.tx,
        status_target.document_epoch,
        status_target.pdf_path,
        format!("OpenRouter OCR page {}...", page_index + 1),
    );
    let body = json!({
        "model": OPENROUTER_OCR_MODEL,
        "messages": [
            {
                "role": "user",
                "content": [
                    {
                        "type": "text",
                        "text": "Extract every readable word from this PDF page. Preserve natural line breaks and reading order. Return only the OCR text, with no commentary."
                    },
                    {
                        "type": "image_url",
                        "image_url": { "url": page_data_url(render_tx, pdf_path, page_index)? }
                    }
                ]
            }
        ],
        "temperature": 0
    });

    let response_text =
        post_openrouter_ocr(client, api_key, &body, last_request_at, Some(status_target))?;
    let response_json =
        serde_json::from_str::<serde_json::Value>(&response_text).map_err(|error| {
            format!(
                "OpenRouter OCR response was not JSON: {error}; {}",
                preview(&response_text, 1200)
            )
        })?;
    let content = response_json
        .pointer("/choices/0/message/content")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "OpenRouter OCR response did not include message content.".to_owned())?;

    Ok(strip_text_fence(content).trim().to_owned())
}

fn page_data_url(
    render_tx: &Sender<RenderRequest>,
    pdf_path: &PathBuf,
    page_index: usize,
) -> Result<String, String> {
    let image_path = temp_image_path(page_index);
    export_ocr_image(render_tx, pdf_path.clone(), page_index, image_path.clone())?;
    let image_bytes = std::fs::read(&image_path)
        .map_err(|error| format!("Could not read rendered OCR image: {error}"))?;
    let _ = std::fs::remove_file(&image_path);

    Ok(format!(
        "data:image/png;base64,{}",
        base64_encode(&image_bytes)
    ))
}

fn post_openrouter_ocr(
    client: &reqwest::blocking::Client,
    api_key: &str,
    body: &serde_json::Value,
    last_request_at: &mut Option<Instant>,
    status_target: Option<&OcrStatusTarget<'_>>,
) -> Result<String, String> {
    for attempt in 0..=OPENROUTER_MAX_RETRIES {
        if let Some(last_request) = *last_request_at {
            let wait = OPENROUTER_REQUEST_SPACING.saturating_sub(last_request.elapsed());
            if !wait.is_zero() {
                if let Some(target) = status_target {
                    send_ocr_status(
                        target.tx,
                        target.document_epoch,
                        target.pdf_path,
                        format!(
                            "Waiting {:.1}s to respect OpenRouter free-model limits...",
                            wait.as_secs_f32()
                        ),
                    );
                }
                thread::sleep(wait);
            }
        }
        *last_request_at = Some(Instant::now());

        let response = client
            .post(OPENROUTER_URL)
            .bearer_auth(api_key)
            .header("Accept", "application/json")
            .header("Accept-Encoding", "identity")
            .header("HTTP-Referer", "https://github.com/yonathanarbel/LawPDF")
            .header("X-Title", "LawPDF")
            .json(body)
            .send()
            .map_err(|error| format!("OpenRouter OCR request failed: {error}"))?;

        let status = response.status();
        let retry_after = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
            .map(Duration::from_secs);
        let response_text = response
            .text()
            .map_err(|error| format!("Could not read OpenRouter OCR response: {error}"))?;
        if status.is_success() {
            return Ok(response_text);
        }

        if status.as_u16() == 429 && attempt < OPENROUTER_MAX_RETRIES {
            let wait =
                retry_after.unwrap_or_else(|| Duration::from_secs(2_u64.pow((attempt + 1) as u32)));
            if let Some(target) = status_target {
                send_ocr_status(
                    target.tx,
                    target.document_epoch,
                    target.pdf_path,
                    format!(
                        "OpenRouter rate-limited OCR; retrying in {}...",
                        duration_label(wait)
                    ),
                );
            }
            thread::sleep(wait);
            continue;
        }

        return Err(format!(
            "OpenRouter OCR returned HTTP {status}: {}",
            preview(&response_text, 1200)
        ));
    }

    Err("OpenRouter OCR retry limit reached.".to_owned())
}

fn parse_ocr_batch_response(
    response_text: &str,
    page_indices: &[usize],
) -> Result<Vec<String>, String> {
    let response_json =
        serde_json::from_str::<serde_json::Value>(response_text).map_err(|error| {
            format!(
                "OpenRouter OCR response was not JSON: {error}; {}",
                preview(response_text, 1200)
            )
        })?;
    let content = response_json
        .pointer("/choices/0/message/content")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "OpenRouter OCR response did not include message content.".to_owned())?;
    let content = strip_text_fence(content);
    let value = serde_json::from_str::<serde_json::Value>(&content).map_err(|error| {
        format!(
            "OpenRouter OCR content was not JSON: {error}; {}",
            preview(&content, 1200)
        )
    })?;
    let pages = value
        .get("pages")
        .and_then(serde_json::Value::as_array)
        .or_else(|| value.as_array())
        .ok_or_else(|| "OpenRouter OCR JSON did not include a pages array.".to_owned())?;

    let mut texts = Vec::with_capacity(page_indices.len());
    for (ordinal, page_index) in page_indices.iter().enumerate() {
        let page_number = (*page_index + 1) as u64;
        let page_value = pages
            .iter()
            .find(|page| {
                page.get("page")
                    .and_then(serde_json::Value::as_u64)
                    .is_some_and(|value| value == page_number)
            })
            .or_else(|| pages.get(ordinal))
            .ok_or_else(|| format!("OpenRouter OCR JSON omitted page {}.", page_index + 1))?;
        let text = page_value
            .get("text")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                format!(
                    "OpenRouter OCR JSON omitted text for page {}.",
                    page_index + 1
                )
            })?;
        texts.push(text.trim().to_owned());
    }
    Ok(texts)
}

fn send_ocr_status(tx: &Sender<OcrEvent>, document_epoch: u64, pdf_path: &PathBuf, status: String) {
    let _ = tx.send(OcrEvent {
        document_epoch,
        path: pdf_path.clone(),
        page_index: usize::MAX,
        state: OcrPageState::Idle,
        status: Some(status),
    });
}

fn strip_text_fence(content: &str) -> String {
    let trimmed = content.trim();
    if !trimmed.starts_with("```") {
        return trimmed.to_owned();
    }
    let mut without_start = trimmed.trim_start_matches("```").trim();
    if let Some(newline) = without_start.find('\n') {
        let language = without_start[..newline].trim();
        if !language.is_empty() && language.chars().all(|ch| ch.is_ascii_alphabetic()) {
            without_start = without_start[newline + 1..].trim();
        }
    }
    without_start.trim_end_matches("```").trim().to_owned()
}

fn preview(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_owned();
    }
    let mut value = value.chars().take(max_chars).collect::<String>();
    value.push_str("...");
    value
}

fn duration_label(duration: Duration) -> String {
    let seconds = duration.as_secs();
    if seconds >= 60 {
        format!("{}m {}s", seconds / 60, seconds % 60)
    } else {
        format!("{seconds}s")
    }
}

fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(bytes.len().div_ceil(3) * 4);

    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = *chunk.get(1).unwrap_or(&0);
        let b2 = *chunk.get(2).unwrap_or(&0);

        output.push(TABLE[(b0 >> 2) as usize] as char);
        output.push(TABLE[(((b0 & 0b0000_0011) << 4) | (b1 >> 4)) as usize] as char);
        if chunk.len() > 1 {
            output.push(TABLE[(((b1 & 0b0000_1111) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            output.push('=');
        }
        if chunk.len() > 2 {
            output.push(TABLE[(b2 & 0b0011_1111) as usize] as char);
        } else {
            output.push('=');
        }
    }

    output
}

fn ocr_cache_path(path: &Path) -> Option<PathBuf> {
    app_data_dir().map(|dir| {
        dir.join("ocr-cache")
            .join(format!("{}.json", source_signature(path)))
    })
}

fn source_signature(path: &Path) -> String {
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let metadata = std::fs::metadata(path).ok();
    let modified = metadata
        .as_ref()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    let len = metadata.map(|metadata| metadata.len()).unwrap_or_default();
    stable_hash(&format!(
        "ocr-v{OCR_CACHE_SCHEMA_VERSION}|{}|{modified}|{len}",
        canonical.display()
    ))
}

fn stable_hash(value: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}
