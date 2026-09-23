use std::fs;
use std::io::{Read, Write};
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
            let _ = tx.send(PaidTtsEvent::Failed(error));
        }
    });
}

fn run_paid_tts_job(request: &PaidTtsRequest, tx: &Sender<PaidTtsEvent>) -> Result<(), String> {
    let chunks = split_for_tts(&request.text, MAX_CHUNK_CHARS);
    if chunks.is_empty() {
        return Err("Nothing to narrate.".to_owned());
    }
    if chunks.len() > 1000 {
        return Err(
            "This narration exceeds the supported length. Export a shorter selection.".to_owned(),
        );
    }
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|_| "Could not start the speech service.".to_owned())?;
    crate::atomic_file::replace_with(&request.destination, |output| {
        let mut total_bytes = 0usize;
        for (index, chunk) in chunks.iter().enumerate() {
            let response = client.post(request.provider.endpoint())
                .bearer_auth(&request.api_key)
                .header("HTTP-Referer", "https://github.com/yonathanarbel/LawPDF")
                .header("X-Title", "LawPDF")
                .json(&json!({"model": request.provider.model(), "input": chunk, "voice": request.voice, "response_format": "mp3"}))
                .send().map_err(|_| std::io::Error::other("The speech service could not be reached. Check your connection and retry."))?;
            if !response.status().is_success() {
                return Err(std::io::Error::other(format!("{} speech service returned HTTP {}. Check the provider account and voice settings.", request.provider.label(), response.status().as_u16())));
            }
            let mut bytes = Vec::new();
            response.take(32 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
            total_bytes = total_bytes.saturating_add(bytes.len());
            if bytes.is_empty() || bytes.len() > 32 * 1024 * 1024 || total_bytes > 1024 * 1024 * 1024 {
                return Err(std::io::Error::other("The speech response exceeded the supported audio size."));
            }
            output.write_all(&bytes)?;
            tx.send(PaidTtsEvent::Progress { completed: index + 1, total: chunks.len() })
                .map_err(|_| std::io::Error::other("Audio export was canceled."))?;
        }
        Ok(())
    }).map_err(|error| format!("Could not export audio: {error}"))?;
    let _ = tx.send(PaidTtsEvent::Complete(request.destination.clone()));
    Ok(())
}

pub fn write_private_speech_text(text: &str) -> std::io::Result<PathBuf> {
    if text.len() > 16 * 1024 * 1024 {
        return Err(std::io::Error::other("Select less text to read aloud."));
    }
    let root = crate::settings::app_data_dir()
        .ok_or_else(|| std::io::Error::other("No private speech folder is available."))?
        .join("speech-cache");
    fs::create_dir_all(&root)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
    }
    let mut file = tempfile::Builder::new()
        .prefix("speech-")
        .suffix(".txt")
        .tempfile_in(root)?;
    file.write_all(text.as_bytes())?;
    file.as_file().sync_all()?;
    let (file, path) = file.keep().map_err(|error| error.error)?;
    drop(file);
    Ok(path)
}

/// Run speech in a disposable copy of our own executable. This works in an
/// MSIX installation without launching PowerShell or interpreting document text.
#[cfg(target_os = "windows")]
pub fn run_windows_speech_worker() -> Result<(), String> {
    use windows::Win32::Media::Speech::{ISpVoice, SPF_IS_NOT_XML, SpVoice};
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
        CoUninitialize,
    };
    use windows::core::PCWSTR;

    let path = std::env::var_os("LAWPDF_TTS_TEXT_PATH")
        .ok_or("No speech text was provided.")?;
    let bytes = crate::document_store::read_limited(
        &PathBuf::from(path), 16 * 1024 * 1024,
    ).map_err(|error| format!("Could not read speech text: {error}"))?;
    let text = String::from_utf8(bytes).map_err(|_| "Speech text is not valid UTF-8.")?;
    let wide: Vec<u16> = text.replace('\0', " ").encode_utf16().chain(Some(0)).collect();
    // SAFETY: This dedicated child initializes COM on its sole main thread.
    // All COM interfaces are dropped before the matching CoUninitialize call.
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.ok()
        .map_err(|error| format!("Could not initialize Windows speech: {error}"))?;
    let result = (|| -> windows::core::Result<()> {
        let voice: ISpVoice = unsafe { CoCreateInstance(&SpVoice, None, CLSCTX_INPROC_SERVER)? };
        // Speak synchronously; the parent can stop speech by terminating this
        // child. Force plain text so PDF content cannot act as SAPI markup.
        unsafe { voice.Speak(PCWSTR(wide.as_ptr()), SPF_IS_NOT_XML.0 as u32, None) }
    })();
    unsafe { CoUninitialize() };
    result.map_err(|error| format!("Windows speech failed: {error}"))
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
