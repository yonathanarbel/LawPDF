use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::thread;

use crossbeam_channel::Sender;
use serde_json::json;

const MAX_CHUNK_CHARS: usize = 3_800;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaidTtsProvider {
    OpenRouter,
    OpenAi,
}

impl PaidTtsProvider {
    pub fn label(self) -> &'static str {
        match self {
            Self::OpenRouter => "OpenRouter",
            Self::OpenAi => "OpenAI",
        }
    }

    fn endpoint(self) -> &'static str {
        match self {
            Self::OpenRouter => "https://openrouter.ai/api/v1/audio/speech",
            Self::OpenAi => "https://api.openai.com/v1/audio/speech",
        }
    }

    fn model(self) -> &'static str {
        match self {
            Self::OpenRouter => "openai/gpt-4o-mini-tts-2025-12-15",
            Self::OpenAi => "gpt-4o-mini-tts",
        }
    }
}

pub struct PaidTtsRequest {
    pub provider: PaidTtsProvider,
    pub api_key: String,
    pub voice: String,
    pub text: String,
    pub destination: PathBuf,
}

#[derive(Debug)]
pub enum PaidTtsEvent {
    Progress { completed: usize, total: usize },
    Complete(PathBuf),
    Failed(String),
}

pub fn spawn_paid_tts_job(request: PaidTtsRequest, tx: Sender<PaidTtsEvent>) {
    thread::spawn(move || {
        if let Err(error) = run_paid_tts_job(&request, &tx) {
            let _ = fs::remove_file(request.destination.with_extension("mp3.part"));
            let _ = tx.send(PaidTtsEvent::Failed(error));
        }
    });
}

fn run_paid_tts_job(request: &PaidTtsRequest, tx: &Sender<PaidTtsEvent>) -> Result<(), String> {
    let chunks = split_for_tts(&request.text, MAX_CHUNK_CHARS);
    if chunks.is_empty() {
        return Err("Nothing to narrate.".to_owned());
    }
    let part_path = request.destination.with_extension("mp3.part");
    let _ = fs::remove_file(&part_path);
    let mut output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&part_path)
        .map_err(|error| format!("Could not create audio file: {error}"))?;
    let client = reqwest::blocking::Client::builder()
        .build()
        .map_err(|error| format!("Could not start TTS client: {error}"))?;

    for (index, chunk) in chunks.iter().enumerate() {
        let response = client
            .post(request.provider.endpoint())
            .bearer_auth(&request.api_key)
            .header("Content-Type", "application/json")
            .header("HTTP-Referer", "https://github.com/yonathanarbel/lawpdf")
            .header("X-Title", "Review Mode")
            .json(&json!({
                "model": request.provider.model(),
                "input": chunk,
                "voice": request.voice,
                "response_format": "mp3"
            }))
            .send()
            .map_err(|error| format!("{} TTS request failed: {error}", request.provider.label()))?;
        let status = response.status();
        if !status.is_success() {
            let detail = response.text().unwrap_or_default();
            let _ = fs::remove_file(&part_path);
            return Err(format!(
                "{} TTS returned {status}: {}",
                request.provider.label(),
                compact_error(&detail)
            ));
        }
        let bytes = response
            .bytes()
            .map_err(|error| format!("Could not read TTS audio: {error}"))?;
        output
            .write_all(&bytes)
            .map_err(|error| format!("Could not save TTS audio: {error}"))?;
        let _ = tx.send(PaidTtsEvent::Progress {
            completed: index + 1,
            total: chunks.len(),
        });
    }
    output
        .flush()
        .map_err(|error| format!("Could not finish TTS audio: {error}"))?;
    drop(output);
    if request.destination.exists() {
        fs::remove_file(&request.destination)
            .map_err(|error| format!("Could not replace audio file: {error}"))?;
    }
    fs::rename(&part_path, &request.destination)
        .map_err(|error| format!("Could not finish audio file: {error}"))?;
    let _ = tx.send(PaidTtsEvent::Complete(request.destination.clone()));
    Ok(())
}

fn split_for_tts(text: &str, max_chars: usize) -> Vec<String> {
    let normalized = text.trim();
    if normalized.is_empty() || max_chars == 0 {
        return Vec::new();
    }
    let mut chunks = Vec::new();
    let mut remaining = normalized;
    while remaining.chars().count() > max_chars {
        let byte_limit = remaining
            .char_indices()
            .nth(max_chars)
            .map(|(index, _)| index)
            .unwrap_or(remaining.len());
        let window = &remaining[..byte_limit];
        let split = window
            .rfind("\n\n")
            .map(|index| index + 2)
            .or_else(|| window.rfind(['.', '?', '!']).map(|index| index + 1))
            .or_else(|| window.rfind(char::is_whitespace))
            .filter(|index| *index > max_chars / 2)
            .unwrap_or(byte_limit);
        let (chunk, rest) = remaining.split_at(split);
        if !chunk.trim().is_empty() {
            chunks.push(chunk.trim().to_owned());
        }
        remaining = rest.trim_start();
    }
    if !remaining.trim().is_empty() {
        chunks.push(remaining.trim().to_owned());
    }
    chunks
}

fn compact_error(value: &str) -> String {
    let one_line = value.split_whitespace().collect::<Vec<_>>().join(" ");
    one_line.chars().take(240).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_long_text_at_readable_boundaries() {
        let text = format!(
            "{}\n\n{}",
            "First sentence. ".repeat(20),
            "Second paragraph. ".repeat(20)
        );
        let chunks = split_for_tts(&text, 180);
        assert!(chunks.len() > 2);
        assert!(chunks.iter().all(|chunk| chunk.chars().count() <= 180));
        assert_eq!(
            chunks.join(" ").split_whitespace().collect::<Vec<_>>(),
            text.split_whitespace().collect::<Vec<_>>()
        );
    }

    #[test]
    fn provider_defaults_match_each_api() {
        assert_eq!(
            PaidTtsProvider::OpenAi.endpoint(),
            "https://api.openai.com/v1/audio/speech"
        );
        assert_eq!(PaidTtsProvider::OpenAi.model(), "gpt-4o-mini-tts");
        assert!(PaidTtsProvider::OpenRouter.model().starts_with("openai/"));
    }
}
