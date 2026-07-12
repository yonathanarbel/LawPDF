//! LLM HTTP client, provider handling, response parsing, and layout application.
//!
//! Reliability features:
//! - Shared reqwest::blocking::Client with compression enabled (no per-request recreation,
//!   no http1_only or no_* compression disables).
//! - Retry logic with exponential backoff; honors Retry-After on 429.
//! - Structured output via response_format + json_schema (schemars-derived from LlmLayout).
//! - Groq: reasoning_effort="low" + raised max_completion_tokens (set at call site in mod.rs prepare).

use std::sync::OnceLock;
use std::time::Duration;

use reqwest::blocking::Client;
use reqwest::{StatusCode, header};
use serde_json::Value;

use crate::liquid::model::LlmProvider;

/// Shared HTTP client for all Liquid LLM calls (connection reuse, TLS reuse, compression).
static LLM_HTTP_CLIENT: OnceLock<Client> = OnceLock::new();
const LLM_REQUEST_TIMEOUT_SECS: u64 = 180;

/// Returns a process-wide shared blocking client configured for LLM calls.
/// Compression (gzip/brotli/deflate/zstd) is enabled by default for smaller responses.
pub(crate) fn get_llm_client() -> &'static Client {
    LLM_HTTP_CLIENT.get_or_init(|| {
        Client::builder()
            .timeout(Duration::from_secs(LLM_REQUEST_TIMEOUT_SECS))
            .build()
            .expect("failed to build shared LLM HTTP client")
    })
}

/// Sends the LLM chat completion request with retries + backoff.
/// Retries on network errors and 429 (rate limit), respecting Retry-After when present.
/// Simple non-crate implementation for minimal dependency surface.
pub(crate) fn send_llm_request_with_retries(
    client: &Client,
    provider: &LlmProvider,
    api_key: &str,
    body: &Value,
) -> Result<reqwest::blocking::Response, String> {
    const MAX_ATTEMPTS: usize = 4;
    let mut attempt = 0usize;
    let mut backoff = Duration::from_millis(600);

    loop {
        attempt += 1;

        let mut rb = client
            .post(provider.url)
            .bearer_auth(api_key)
            .header("Accept", "application/json");
        // Note: deliberately do NOT set Accept-Encoding: identity.
        // Compression is beneficial and now handled by the shared client defaults.

        if provider.openrouter_headers {
            rb = rb
                .header("HTTP-Referer", "https://github.com/yonathanarbel/LawPDF")
                .header("X-Title", "LawPDF");
        }

        match rb.json(body).send() {
            Ok(resp) => {
                if resp.status() == StatusCode::TOO_MANY_REQUESTS && attempt < MAX_ATTEMPTS {
                    // Honor Retry-After if provided (seconds or http-date; simple parse for seconds)
                    let retry_delay = resp
                        .headers()
                        .get(header::RETRY_AFTER)
                        .and_then(|hv| hv.to_str().ok())
                        .and_then(|s| s.parse::<u64>().ok())
                        .map(Duration::from_secs)
                        .unwrap_or(backoff);

                    std::thread::sleep(retry_delay);
                    backoff = (backoff * 2).min(Duration::from_secs(8));
                    continue;
                }
                return Ok(resp);
            }
            Err(e) if attempt < MAX_ATTEMPTS && is_transient_error(&e) => {
                std::thread::sleep(backoff);
                backoff = (backoff * 2).min(Duration::from_secs(8));
                continue;
            }
            Err(e) => {
                return Err(format!("{} request failed: {}", provider.name, e));
            }
        }
    }
}

fn is_transient_error(err: &reqwest::Error) -> bool {
    err.is_timeout() || err.is_connect() || err.is_request()
}
