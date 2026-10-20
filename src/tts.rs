//! Speech initializes only after a user action. A background worker owns every
//! process and temporary file; Stop never cancels another application's audio.
use crossbeam_channel::Sender;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

const MAX_CHUNK_CHARS: usize = 3_800;
const FIRST_PLAYBACK_CHARS: usize = 600;
const MAX_AUDIO_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpeechProvider {
    #[default]
    System,
    ElevenLabs,
    OpenRouter,
}
impl SpeechProvider {
    pub fn label(self) -> &'static str {
        match self {
            Self::System => "System speech",
            Self::ElevenLabs => "ElevenLabs",
            Self::OpenRouter => "OpenRouter",
        }
    }
}
// No Debug: keys must never enter logs or diagnostics.
pub struct ApiSpeechConfig {
    pub provider: SpeechProvider,
    pub api_key: String,
    pub model: String,
    pub voice: String,
}
pub enum SpeechRequest {
    System(String),
    Api {
        config: ApiSpeechConfig,
        text: String,
        destination: Option<PathBuf>,
    },
}
pub struct SpeechEvent {
    pub job_id: u64,
    pub kind: SpeechEventKind,
}
pub enum SpeechEventKind {
    Progress { completed: usize, total: usize },
    Playing,
    Complete(Option<PathBuf>),
    Failed(String),
}
type ActiveProcess = Arc<Mutex<Option<Child>>>;
pub struct SpeechCancellation {
    canceled: Arc<AtomicBool>,
    process: ActiveProcess,
}
impl SpeechCancellation {
    pub fn cancel(&self) {
        self.canceled.store(true, Ordering::Release);
        // Kill has no wait. Doing it before app exit avoids orphaned speech;
        // the worker later reaps the child and removes its temporary files.
        if let Ok(mut process) = self.process.lock() {
            if let Some(child) = process.as_mut() {
                let _ = child.kill();
            }
        }
    }
}
impl Drop for SpeechCancellation {
    fn drop(&mut self) {
        self.cancel();
    }
}
pub fn spawn_speech_job(
    job_id: u64,
    request: SpeechRequest,
    tx: Sender<SpeechEvent>,
) -> SpeechCancellation {
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = Arc::clone(&cancel);
    let process = Arc::new(Mutex::new(None));
    let worker_process = Arc::clone(&process);
    thread::spawn(move || {
        let send = |kind| {
            let _ = tx.send(SpeechEvent { job_id, kind });
        };
        let result = match request {
            SpeechRequest::System(text) => {
                run_system_speech(&text, &worker_cancel, &worker_process, &send)
            }
            SpeechRequest::Api {
                config,
                text,
                destination,
            } => run_api_speech(
                &config,
                &text,
                destination.as_deref(),
                &worker_cancel,
                &worker_process,
                &send,
            ),
        };
        if let Err(error) = result {
            // The UI also checks job identity, so superseded jobs stay silent.
            if error != "Speech canceled." {
                send(SpeechEventKind::Failed(error));
            }
        }
    });
    SpeechCancellation {
        canceled: cancel,
        process,
    }
}
fn check_cancel(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err("Speech canceled.".to_owned())
    } else {
        Ok(())
    }
}
fn private_file(suffix: &str) -> Result<tempfile::NamedTempFile, String> {
    let root = crate::settings::app_data_dir()
        .ok_or("No private speech folder is available.")?
        .join("speech-cache");
    fs::create_dir_all(&root).map_err(|_| "Could not prepare the speech folder.")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700))
            .map_err(|_| "Could not protect the speech folder.")?;
    }
    tempfile::Builder::new()
        .prefix("speech-")
        .suffix(suffix)
        .tempfile_in(root)
        .map_err(|_| "Could not prepare temporary speech data.".to_owned())
}
fn private_text(text: &str) -> Result<tempfile::NamedTempFile, String> {
    if text.len() > 16 * 1024 * 1024 {
        return Err("Select less text to read aloud.".to_owned());
    }
    let mut file = private_file(".txt")?;
    file.write_all(text.as_bytes())
        .and_then(|()| file.flush())
        .map_err(|_| "Could not prepare speech text.")?;
    Ok(file)
}
struct RegisteredChild<'a>(&'a ActiveProcess);
impl Drop for RegisteredChild<'_> {
    fn drop(&mut self) {
        let child = self.0.lock().ok().and_then(|mut slot| {
            if let Some(child) = slot.as_mut() {
                let _ = child.kill();
            }
            slot.take()
        });
        if let Some(mut child) = child {
            let _ = child.wait();
        }
    }
}
fn quiet_command(command: &mut Command) {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
}
fn run_child(
    mut command: Command,
    cancel: &AtomicBool,
    process: &ActiveProcess,
) -> Result<(), String> {
    {
        // Cancellation cannot land between spawn and registration. Only this
        // tiny OS operation shares a lock with Stop; network never holds it.
        let mut slot = process
            .lock()
            .map_err(|_| "Could not start the speech player.")?;
        check_cancel(cancel)?;
        *slot = Some(command.spawn().map_err(
            |_| "Could not start the speech or audio player. Check that it is installed.",
        )?);
    }
    let _registered = RegisteredChild(process);
    loop {
        check_cancel(cancel)?;
        let status = {
            let mut slot = process
                .lock()
                .map_err(|_| "Could not check the speech player.")?;
            slot.as_mut()
                .ok_or("The speech player stopped.")?
                .try_wait()
        };
        match status {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(_)) => return Err(
                "The speech or audio player failed. Check your system audio and selected voice."
                    .to_owned(),
            ),
            Ok(None) => thread::sleep(Duration::from_millis(40)),
            Err(_) => return Err("Could not check the speech player.".to_owned()),
        }
    }
}
fn run_system_speech(
    text: &str,
    cancel: &AtomicBool,
    process: &ActiveProcess,
    send: &impl Fn(SpeechEventKind),
) -> Result<(), String> {
    check_cancel(cancel)?;
    if text.trim().is_empty() {
        return Err("Nothing to read aloud.".to_owned());
    }
    let file = private_text(text)?;
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut c = Command::new("/usr/bin/say");
        c.arg("-f").arg(file.path());
        c
    };
    #[cfg(target_os = "windows")]
    let mut command = {
        let executable =
            std::env::current_exe().map_err(|_| "Could not locate the speech worker.")?;
        let mut c = Command::new(executable);
        c.arg("--speech-worker")
            .env("LAWPDF_TTS_TEXT_PATH", file.path());
        c
    };
    #[cfg(target_os = "linux")]
    let mut command = {
        // A private synthesizer can be stopped without affecting other apps.
        // spd-say --cancel would cancel all Speech Dispatcher clients.
        let program = find_program(&["espeak-ng", "espeak"]).ok_or("System speech needs espeak-ng or espeak. Install one, or choose ElevenLabs or OpenRouter in Settings.")?;
        let mut c = Command::new(program);
        c.arg("-f").arg(file.path());
        c
    };
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    return Err("System speech is unavailable. Choose ElevenLabs or OpenRouter.".to_owned());
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    {
        quiet_command(&mut command);
        check_cancel(cancel)?;
        send(SpeechEventKind::Playing);
        run_child(command, cancel, process)?;
        send(SpeechEventKind::Complete(None));
        Ok(())
    }
}
#[cfg(target_os = "linux")]
fn find_program(names: &[&str]) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    names.iter().find_map(|name| {
        std::env::split_paths(&path)
            .map(|root| root.join(name))
            .find(|candidate| candidate.is_file())
    })
}
enum AudioPlayer {
    #[cfg(target_os = "macos")]
    Afplay,
    #[cfg(target_os = "windows")]
    Windows(PathBuf),
    #[cfg(target_os = "linux")]
    Ffplay(PathBuf),
    #[cfg(target_os = "linux")]
    Mpg123(PathBuf),
}
impl AudioPlayer {
    fn discover() -> Result<Self, String> {
        #[cfg(target_os = "macos")]
        {
            Ok(Self::Afplay)
        }
        #[cfg(target_os = "windows")]
        {
            std::env::current_exe()
                .map(Self::Windows)
                .map_err(|_| "Could not locate the audio worker.".to_owned())
        }
        #[cfg(target_os = "linux")]
        {
            if let Some(path) = find_program(&["ffplay"]) {
                return Ok(Self::Ffplay(path));
            }
            if let Some(path) = find_program(&["mpg123"]) {
                return Ok(Self::Mpg123(path));
            }
            Err("API read aloud needs ffplay (FFmpeg) or mpg123. Install either, or use Create MP3 to save the audio.".to_owned())
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        Err("Audio playback is unavailable. Use Create MP3 to save the audio.".to_owned())
    }
    fn play(
        &self,
        path: &Path,
        cancel: &AtomicBool,
        process: &ActiveProcess,
    ) -> Result<(), String> {
        check_cancel(cancel)?;
        let mut command = match self {
            #[cfg(target_os = "macos")]
            Self::Afplay => {
                let mut c = Command::new("/usr/bin/afplay");
                c.arg(path);
                c
            }
            #[cfg(target_os = "windows")]
            Self::Windows(executable) => {
                let mut c = Command::new(executable);
                c.arg("--audio-worker").env("LAWPDF_TTS_AUDIO_PATH", path);
                c
            }
            #[cfg(target_os = "linux")]
            Self::Ffplay(executable) => {
                let mut c = Command::new(executable);
                c.args(["-nodisp", "-autoexit", "-loglevel", "error", "-i"])
                    .arg(path);
                c
            }
            #[cfg(target_os = "linux")]
            Self::Mpg123(executable) => {
                let mut c = Command::new(executable);
                c.arg("-q").arg(path);
                c
            }
        };
        quiet_command(&mut command);
        run_child(command, cancel, process)
    }
}
fn client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(90))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "Could not start the speech service.".to_owned())
}
fn speech_endpoint(config: &ApiSpeechConfig) -> Result<String, String> {
    if config.api_key.trim().is_empty() {
        return Err("Add your speech API key in Settings.".to_owned());
    }
    if config.model.trim().is_empty()
        || config.model.len() > 160
        || config.voice.trim().is_empty()
        || config.voice.len() > 160
    {
        return Err("Set a speech model and voice in Settings.".to_owned());
    }
    match config.provider {
        SpeechProvider::OpenRouter => Ok("https://openrouter.ai/api/v1/audio/speech".to_owned()),
        SpeechProvider::ElevenLabs => {
            if !config
                .voice
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
            {
                return Err("The ElevenLabs voice ID must contain only letters, numbers, underscores, or hyphens.".to_owned());
            }
            Ok(format!(
                "https://api.elevenlabs.io/v1/text-to-speech/{}?output_format=mp3_44100_128",
                config.voice
            ))
        }
        SpeechProvider::System => Err("Choose ElevenLabs or OpenRouter for API speech.".to_owned()),
    }
}
fn synthesize_chunk(
    client: &reqwest::blocking::Client,
    endpoint: &str,
    config: &ApiSpeechConfig,
    text: &str,
    cancel: &AtomicBool,
) -> Result<Vec<u8>, String> {
    check_cancel(cancel)?;
    let request = match config.provider {
        SpeechProvider::OpenRouter => client.post(endpoint).bearer_auth(&config.api_key)
            .header("HTTP-Referer", "https://github.com/yonathanarbel/LawPDF").header("X-OpenRouter-Title", "LawPDF")
            .json(&json!({"model": config.model, "input": text, "voice": config.voice, "response_format": "mp3"})),
        SpeechProvider::ElevenLabs => client.post(endpoint).header("xi-api-key", &config.api_key).json(&json!({"model_id": config.model, "text": text})),
        SpeechProvider::System => return Err("System speech does not use an API.".to_owned()),
    };
    // No automatic retry: a timed-out paid synthesis may already be billable.
    let response = request.send().map_err(|_| {
        "The speech service could not be reached or timed out. Check your connection and retry."
            .to_owned()
    })?;
    check_cancel(cancel)?;
    if !response.status().is_success() {
        return Err(format!(
            "{} speech returned HTTP {}. Check your API key, account balance, model, and voice in Settings.",
            config.provider.label(),
            response.status().as_u16()
        ));
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !content_type.starts_with("audio/") && !content_type.starts_with("application/octet-stream")
    {
        return Err("The speech service returned data that is not audio.".to_owned());
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_AUDIO_BYTES)
    {
        return Err("The speech response exceeded the supported audio size.".to_owned());
    }
    let mut bytes = Vec::new();
    response
        .take(MAX_AUDIO_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "The speech audio download was interrupted.".to_owned())?;
    check_cancel(cancel)?;
    if bytes.len() as u64 > MAX_AUDIO_BYTES || !looks_like_mp3(&bytes) {
        return Err("The speech service did not return a valid MP3 response.".to_owned());
    }
    Ok(bytes)
}
fn looks_like_mp3(bytes: &[u8]) -> bool {
    bytes.starts_with(b"ID3") || (bytes.len() > 2 && bytes[0] == 0xff && bytes[1] & 0xe0 == 0xe0)
}
fn run_api_speech(
    config: &ApiSpeechConfig,
    text: &str,
    destination: Option<&Path>,
    cancel: &Arc<AtomicBool>,
    process: &ActiveProcess,
    send: &impl Fn(SpeechEventKind),
) -> Result<(), String> {
    let endpoint = speech_endpoint(config)?;
    // Check playback before making a potentially billable request.
    let player = if destination.is_none() {
        Some(AudioPlayer::discover()?)
    } else {
        None
    };
    let client = client()?;
    let chunks = if destination.is_none() {
        playback_chunks(text)
    } else {
        split_for_tts(text, MAX_CHUNK_CHARS)
    };
    if chunks.is_empty() {
        return Err("Nothing to narrate.".to_owned());
    }
    if chunks.len() > 1000 {
        return Err("Select less text to narrate.".to_owned());
    }
    let total = chunks.len();
    send(SpeechEventKind::Progress {
        completed: 0,
        total,
    });
    if let Some(destination) = destination {
        export_mp3(
            &client,
            &endpoint,
            config,
            &chunks,
            destination,
            cancel,
            send,
        )?;
        send(SpeechEventKind::Complete(Some(destination.to_path_buf())));
    } else {
        let player = player.ok_or("No audio player is available.")?;
        // Prepare one next chunk while the current one plays. A zero-slot
        // channel bounds prefetch and a short first request reduces startup wait.
        thread::scope(|scope| -> Result<(), String> {
            let (tx, rx) = crossbeam_channel::bounded(0);
            let producer_cancel = Arc::clone(cancel);
            scope.spawn(move || {
                for chunk in chunks {
                    if producer_cancel.load(Ordering::Acquire) {
                        break;
                    }
                    let audio =
                        synthesize_chunk(&client, &endpoint, config, &chunk, &producer_cancel);
                    let failed = audio.is_err();
                    let mut pending = audio;
                    loop {
                        if producer_cancel.load(Ordering::Acquire) {
                            return;
                        }
                        match tx.send_timeout(pending, Duration::from_millis(40)) {
                            Ok(()) => break,
                            Err(crossbeam_channel::SendTimeoutError::Timeout(value)) => {
                                pending = value
                            }
                            Err(crossbeam_channel::SendTimeoutError::Disconnected(_)) => return,
                        }
                    }
                    if failed {
                        break;
                    }
                }
            });
            let result = (|| {
                for completed in 1..=total {
                    let bytes = loop {
                        check_cancel(cancel)?;
                        match rx.recv_timeout(Duration::from_millis(40)) {
                            Ok(result) => break result?,
                            Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
                            Err(_) => {
                                return Err(
                                    "The speech worker ended before narration finished.".to_owned()
                                );
                            }
                        }
                    };
                    let mut file = private_file(".mp3")?;
                    file.write_all(&bytes)
                        .and_then(|()| file.flush())
                        .map_err(|_| "Could not prepare narration audio.")?;
                    send(SpeechEventKind::Playing);
                    player.play(file.path(), cancel, process)?;
                    send(SpeechEventKind::Progress { completed, total });
                }
                Ok(())
            })();
            if let Err(error) = &result {
                // Report a local playback failure immediately. The scoped
                // producer may still be finishing its one bounded HTTP call.
                if error != "Speech canceled." {
                    send(SpeechEventKind::Failed(error.clone()));
                }
                cancel.store(true, Ordering::Release);
            }
            drop(rx);
            result
        })?;
        send(SpeechEventKind::Complete(None));
    }
    Ok(())
}
fn export_mp3(
    client: &reqwest::blocking::Client,
    endpoint: &str,
    config: &ApiSpeechConfig,
    chunks: &[String],
    destination: &Path,
    cancel: &AtomicBool,
    send: &impl Fn(SpeechEventKind),
) -> Result<(), String> {
    crate::atomic_file::replace_with_guard(
        destination,
        |output| {
            let mut total_bytes = 0usize;
            for (index, chunk) in chunks.iter().enumerate() {
                let bytes = synthesize_chunk(client, endpoint, config, chunk, cancel)
                    .map_err(std::io::Error::other)?;
                total_bytes = total_bytes.saturating_add(bytes.len());
                if total_bytes > 1024 * 1024 * 1024 {
                    return Err(std::io::Error::other(
                        "Narration exceeds the supported audio size.",
                    ));
                }
                output.write_all(&bytes)?;
                send(SpeechEventKind::Progress {
                    completed: index + 1,
                    total: chunks.len(),
                });
            }
            Ok(())
        },
        || check_cancel(cancel).map_err(std::io::Error::other),
    )
    .map_err(|error| format!("Could not export narration: {error}"))
}
fn playback_chunks(text: &str) -> Vec<String> {
    let text = text.trim();
    if text.is_empty() {
        return Vec::new();
    }
    let (start, remainder) = text.split_at(chunk_boundary(text, FIRST_PLAYBACK_CHARS));
    let mut chunks = vec![start.trim().to_owned()];
    chunks.extend(split_for_tts(remainder, MAX_CHUNK_CHARS));
    chunks
}
fn chunk_boundary(text: &str, max_chars: usize) -> usize {
    match text.char_indices().nth(max_chars) {
        None => text.len(),
        Some((byte_limit, _)) => {
            let window = &text[..byte_limit];
            window
                .rfind("\n\n")
                .map(|i| i + 2)
                .or_else(|| window.rfind(['.', '?', '!']).map(|i| i + 1))
                .or_else(|| window.rfind(char::is_whitespace))
                .filter(|i| window[..*i].chars().count() > max_chars / 2)
                .unwrap_or(byte_limit)
        }
    }
}
fn split_for_tts(text: &str, max_chars: usize) -> Vec<String> {
    let mut remaining = text.trim();
    if remaining.is_empty() || max_chars == 0 {
        return Vec::new();
    }
    let mut chunks = Vec::new();
    while !remaining.is_empty() {
        let (chunk, rest) = remaining.split_at(chunk_boundary(remaining, max_chars));
        if !chunk.trim().is_empty() {
            chunks.push(chunk.trim().to_owned());
        }
        remaining = rest.trim_start();
    }
    chunks
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

    let path = std::env::var_os("LAWPDF_TTS_TEXT_PATH").ok_or("No speech text was provided.")?;
    let bytes = crate::document_store::read_limited(&PathBuf::from(path), 16 * 1024 * 1024)
        .map_err(|error| format!("Could not read speech text: {error}"))?;
    let text = String::from_utf8(bytes).map_err(|_| "Speech text is not valid UTF-8.")?;
    let wide: Vec<u16> = text
        .replace('\0', " ")
        .encode_utf16()
        .chain(Some(0))
        .collect();
    // SAFETY: This dedicated child initializes COM on its sole main thread.
    // All COM interfaces are dropped before the matching CoUninitialize call.
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
        .ok()
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

/// Built-in Windows MP3 playback, isolated so Stop can terminate just this
/// narration. Paths pass through Unicode WinMM directly, never a shell.
#[cfg(target_os = "windows")]
pub fn run_windows_audio_worker() -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "winmm")]
    unsafe extern "system" {
        fn mciSendStringW(
            command: *const u16,
            result: *mut u16,
            length: u32,
            callback: isize,
        ) -> u32;
    }
    let path =
        std::env::var_os("LAWPDF_TTS_AUDIO_PATH").ok_or("No narration audio was provided.")?;
    let encoded = path.encode_wide().collect::<Vec<_>>();
    if encoded.iter().any(|&c| c == 0 || c == u16::from(b'"')) {
        return Err("The narration audio path is invalid.".to_owned());
    }
    let mut open = "open \"".encode_utf16().collect::<Vec<_>>();
    open.extend(encoded);
    open.extend("\" type mpegvideo alias lawpdf_audio\0".encode_utf16());
    // SAFETY: Commands are nul-terminated UTF-16 and WinMM receives no output
    // buffer or callback. The only variable field is a validated quoted path.
    if unsafe { mciSendStringW(open.as_ptr(), std::ptr::null_mut(), 0, 0) } != 0 {
        return Err("Windows could not open the narration MP3.".to_owned());
    }
    let play = "play lawpdf_audio wait\0"
        .encode_utf16()
        .collect::<Vec<_>>();
    let close = "close lawpdf_audio\0".encode_utf16().collect::<Vec<_>>();
    let result = unsafe { mciSendStringW(play.as_ptr(), std::ptr::null_mut(), 0, 0) };
    unsafe { mciSendStringW(close.as_ptr(), std::ptr::null_mut(), 0, 0) };
    if result == 0 {
        Ok(())
    } else {
        Err("Windows could not play the narration MP3.".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn config(provider: SpeechProvider) -> ApiSpeechConfig {
        ApiSpeechConfig {
            provider,
            api_key: "non-secret-test-key".to_owned(),
            model: "test-model".to_owned(),
            voice: "test-voice".to_owned(),
        }
    }

    fn mock_server(
        responses: Vec<(u16, &'static str, &'static [u8])>,
    ) -> (String, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/speech", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let mut requests = Vec::new();
            for (status, content_type, body) in responses {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut received = Vec::new();
                loop {
                    let mut buf = [0; 4096];
                    let count = stream.read(&mut buf).unwrap();
                    if count == 0 {
                        break;
                    }
                    received.extend_from_slice(&buf[..count]);
                    if let Some(end) = received.windows(4).position(|w| w == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&received[..end]);
                        let length = headers
                            .lines()
                            .find_map(|line| {
                                line.to_lowercase()
                                    .strip_prefix("content-length: ")
                                    .and_then(|n| n.parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if received.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                requests.push(String::from_utf8(received).unwrap());
                write!(stream, "HTTP/1.1 {status} Mock\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
                stream.write_all(body).unwrap();
            }
            requests
        });
        (endpoint, handle)
    }

    #[test]
    fn speech_apis_use_their_own_auth_and_payload() {
        for provider in [SpeechProvider::OpenRouter, SpeechProvider::ElevenLabs] {
            let (endpoint, server) = mock_server(vec![(200, "audio/mpeg", b"ID3mock-audio")]);
            let bytes = synthesize_chunk(
                &client().unwrap(),
                &endpoint,
                &config(provider),
                "Hello from LawPDF.",
                &AtomicBool::new(false),
            )
            .unwrap();
            assert_eq!(bytes, b"ID3mock-audio");
            let requests = server.join().unwrap();
            let request = &requests[0];
            let headers = request.to_lowercase();
            let body: serde_json::Value =
                serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
            match provider {
                SpeechProvider::OpenRouter => {
                    assert!(headers.contains("authorization: bearer non-secret-test-key"));
                    assert_eq!(body["input"], "Hello from LawPDF.");
                    assert_eq!(body["response_format"], "mp3");
                    assert_eq!(body["voice"], "test-voice");
                    assert!(body.get("model_id").is_none());
                }
                SpeechProvider::ElevenLabs => {
                    assert!(headers.contains("xi-api-key: non-secret-test-key"));
                    assert!(!headers.contains("authorization:"));
                    assert_eq!(body["text"], "Hello from LawPDF.");
                    assert_eq!(body["model_id"], "test-model");
                    assert!(body.get("voice").is_none());
                }
                SpeechProvider::System => unreachable!(),
            }
        }
    }

    #[test]
    fn failed_later_chunk_preserves_existing_export_and_redacts_provider_error() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("existing.mp3");
        fs::write(&destination, b"old recording").unwrap();
        let (endpoint, server) = mock_server(vec![
            (200, "audio/mpeg", b"ID3first"),
            (
                401,
                "application/json",
                b"non-secret-test-key confidential document",
            ),
        ]);
        let error = export_mp3(
            &client().unwrap(),
            &endpoint,
            &config(SpeechProvider::ElevenLabs),
            &["first".to_owned(), "second".to_owned()],
            &destination,
            &AtomicBool::new(false),
            &|_| {},
        )
        .unwrap_err();
        assert!(error.contains("HTTP 401"));
        assert!(!error.contains("non-secret-test-key"));
        assert!(!error.contains("confidential document"));
        assert_eq!(fs::read(&destination).unwrap(), b"old recording");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
        assert_eq!(server.join().unwrap().len(), 2);
    }

    #[test]
    fn json_success_is_rejected_instead_of_being_saved_as_audio() {
        let (endpoint, server) = mock_server(vec![(
            200,
            "application/json",
            b"{\"error\":\"not audio\"}",
        )]);
        assert!(
            synthesize_chunk(
                &client().unwrap(),
                &endpoint,
                &config(SpeechProvider::OpenRouter),
                "hello",
                &AtomicBool::new(false)
            )
            .unwrap_err()
            .contains("not audio")
        );
        server.join().unwrap();
    }

    #[test]
    fn cancellation_prevents_any_http_request() {
        let cancel = AtomicBool::new(true);
        let error = synthesize_chunk(
            &client().unwrap(),
            "http://127.0.0.1:1",
            &config(SpeechProvider::OpenRouter),
            "hello",
            &cancel,
        )
        .unwrap_err();
        assert_eq!(error, "Speech canceled.");
    }

    #[cfg(unix)]
    #[test]
    fn cancellation_kills_only_our_player() {
        struct TestChild(Child);
        impl Drop for TestChild {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let mut unrelated = TestChild(Command::new("/bin/sleep").arg("20").spawn().unwrap());
        let canceled = Arc::new(AtomicBool::new(false));
        let process = Arc::new(Mutex::new(None));
        let cancellation = SpeechCancellation {
            canceled: Arc::clone(&canceled),
            process: Arc::clone(&process),
        };
        let worker_process = Arc::clone(&process);
        let worker = thread::spawn(move || {
            let mut command = Command::new("/bin/sleep");
            command.arg("20");
            run_child(command, &canceled, &worker_process)
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while process.lock().unwrap().is_none() {
            assert!(std::time::Instant::now() < deadline, "player did not start");
            thread::sleep(Duration::from_millis(1));
        }
        cancellation.cancel();
        assert!(worker.join().unwrap().is_err());
        assert!(process.lock().unwrap().is_none());
        assert!(unrelated.0.try_wait().unwrap().is_none());
    }

    #[test]
    fn chunks_keep_unicode_text_and_start_with_a_short_request() {
        let text = "An article: café. 判決と脚注。 ".repeat(600);
        let chunks = playback_chunks(&text);
        assert!(chunks[0].chars().count() <= FIRST_PLAYBACK_CHARS);
        assert!(
            chunks
                .iter()
                .all(|chunk| chunk.chars().count() <= MAX_CHUNK_CHARS)
        );
        assert_eq!(
            chunks.join(" ").split_whitespace().collect::<Vec<_>>(),
            text.split_whitespace().collect::<Vec<_>>()
        );
    }

    #[test]
    fn endpoints_reject_path_injection_in_elevenlabs_voice() {
        let mut cfg = config(SpeechProvider::ElevenLabs);
        cfg.voice = "voice?redirect=elsewhere".to_owned();
        assert!(speech_endpoint(&cfg).is_err());
    }
}
