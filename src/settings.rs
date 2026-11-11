use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::liquid::FootnoteMode;

pub const DEFAULT_PDF_ZOOM: f32 = 1.25;
pub const MIN_PDF_ZOOM: f32 = 0.35;
pub const MAX_PDF_ZOOM: f32 = 5.0;
pub const MAX_RECENT_DOCUMENTS: usize = 500;
pub const MAX_LAST_SESSION_DOCUMENTS: usize = 256;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadingView {
    #[default]
    Pdf,
    Review,
    LegacyReview,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecentDocument {
    pub path: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub authors: String,
    #[serde(default)]
    pub citation: String,
    #[serde(default)]
    pub page: usize,
    #[serde(default)]
    pub page_fraction: f32,
    #[serde(default)]
    pub view: ReadingView,
    #[serde(default)]
    pub review_block: Option<usize>,
    #[serde(default)]
    pub last_seen_unix: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowPlacement {
    pub inner_size: [f32; 2],
    #[serde(default)]
    pub outer_position: Option<[f32; 2]>,
    #[serde(default)]
    pub maximized: bool,
}

impl WindowPlacement {
    fn is_valid(&self) -> bool {
        self.inner_size[0].is_finite()
            && (720.0..=7680.0).contains(&self.inner_size[0])
            && self.inner_size[1].is_finite()
            && (520.0..=4320.0).contains(&self.inner_size[1])
            && self.outer_position.is_none_or(|position| {
                position
                    .iter()
                    .all(|value| value.is_finite() && value.abs() <= 16384.0)
            })
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct AppSettings {
    #[serde(default)]
    pub comment_author_name: String,
    #[serde(default)]
    pub comment_author_id: String,
    #[serde(default)]
    pub review_margin_mode: crate::comments::ReviewMarginMode,
    #[serde(default = "default_openrouter_api_key", skip_serializing)]
    pub openrouter_api_key: String,
    #[serde(default, skip_serializing)]
    pub openai_api_key: String,
    #[serde(default, skip_serializing)]
    pub groq_api_key: String,
    #[serde(default, skip_serializing)]
    pub elevenlabs_api_key: String,
    #[serde(default)]
    pub speech_provider: crate::tts::SpeechProvider,
    #[serde(default = "default_openrouter_speech_model")]
    pub openrouter_speech_model: String,
    #[serde(default = "default_openrouter_speech_voice")]
    pub openrouter_speech_voice: String,
    #[serde(default = "default_elevenlabs_speech_model")]
    pub elevenlabs_speech_model: String,
    #[serde(default = "default_elevenlabs_speech_voice")]
    pub elevenlabs_speech_voice: String,
    #[serde(default)]
    pub speech_include_notes: bool,
    /// Offset for the installed system voice; cloud voices use their own defaults.
    #[serde(default)]
    pub system_speech_rate: i32,
    #[serde(default = "default_true")]
    pub speech_follow_along: bool,
    #[serde(skip)]
    pub credential_error: Option<String>,
    #[serde(default = "default_retention_days")]
    pub cache_retention_days: u16,
    #[serde(default = "default_pdf_zoom")]
    pub last_pdf_zoom: f32,
    /// Last zoom selected for each PDF, keyed by its normalized absolute path.
    #[serde(default)]
    pub pdf_zoom_by_document: BTreeMap<String, f32>,
    /// Local reading history, newest first; bounded and subject to retention.
    #[serde(default)]
    pub recent_documents: Vec<RecentDocument>,
    /// Optional local session restore; explicit startup files take precedence.
    #[serde(default)]
    pub reopen_last_session: bool,
    #[serde(default)]
    pub last_session_paths: Vec<String>,
    #[serde(default)]
    pub last_session_saved_unix: u64,
    #[serde(default)]
    pub window_placement: Option<WindowPlacement>,
    /// Window appearance follows the operating system until the reader chooses otherwise.
    #[serde(default)]
    pub appearance: Appearance,
    /// User preference for reduced motion, initialized from the system on first use.
    #[serde(default = "system_reduce_motion")]
    pub reduce_motion: bool,
    /// Optional display-only dimming for the original PDF pages.
    #[serde(default)]
    pub dim_original_pages: bool,
    /// Automatically use the cached, low-latency path when opening PDFs.
    #[serde(default = "default_true")]
    pub optimize_large_documents: bool,
    #[serde(default)]
    pub liquid_mode2_use_pymupdf_blocks: bool,
    #[serde(default)]
    pub liquid_mode2_use_pp_footnote_regions: bool,
    #[serde(default)]
    pub markdown_copy_footnotes: FootnoteMode,
    #[serde(default = "default_true")]
    pub markdown_copy_include_tables: bool,
    #[serde(default)]
    pub markdown_copy_include_metadata: bool,
    /// Show the Review Mode explanation until the reader dismisses it.
    #[serde(default)]
    pub review_intro_dismissed: bool,
    /// Suppress the one-time, non-modal macOS default-reader suggestion.
    #[serde(default)]
    pub macos_default_reader_prompt_dismissed: bool,
    /// Side panel (pages, search, chat, notes) in the original PDF view.
    #[serde(default = "default_true")]
    pub sidebar_in_pdf: bool,
    /// Side panel in Review Mode. Off by default: the margins hold the notes.
    #[serde(default)]
    pub sidebar_in_review: bool,
    /// Where the reader dragged the markup bar, as an offset from its home at
    /// the bottom centre of the page area. `None` keeps it at home.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub markup_bar_offset: Option<[f32; 2]>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            comment_author_name: String::new(),
            comment_author_id: String::new(),
            review_margin_mode: crate::comments::ReviewMarginMode::default(),
            openrouter_api_key: default_openrouter_api_key(),
            openai_api_key: String::new(),
            groq_api_key: String::new(),
            elevenlabs_api_key: String::new(),
            speech_provider: crate::tts::SpeechProvider::System,
            openrouter_speech_model: default_openrouter_speech_model(),
            openrouter_speech_voice: default_openrouter_speech_voice(),
            elevenlabs_speech_model: default_elevenlabs_speech_model(),
            elevenlabs_speech_voice: default_elevenlabs_speech_voice(),
            speech_include_notes: false,
            system_speech_rate: 0,
            speech_follow_along: true,
            credential_error: None,
            cache_retention_days: default_retention_days(),
            last_pdf_zoom: DEFAULT_PDF_ZOOM,
            pdf_zoom_by_document: BTreeMap::new(),
            recent_documents: Vec::new(),
            reopen_last_session: false,
            last_session_paths: Vec::new(),
            last_session_saved_unix: 0,
            window_placement: None,
            appearance: Appearance::System,
            reduce_motion: system_reduce_motion(),
            dim_original_pages: false,
            optimize_large_documents: true,
            liquid_mode2_use_pymupdf_blocks: false,
            liquid_mode2_use_pp_footnote_regions: false,
            markdown_copy_footnotes: FootnoteMode::Inline,
            markdown_copy_include_tables: true,
            markdown_copy_include_metadata: false,
            review_intro_dismissed: false,
            macos_default_reader_prompt_dismissed: false,
            sidebar_in_pdf: true,
            sidebar_in_review: false,
            markup_bar_offset: None,
        }
    }
}

fn default_retention_days() -> u16 {
    30
}

/// Read the operating-system preference only when no LawPDF choice exists.
#[cfg(target_os = "windows")]
fn system_reduce_motion() -> bool {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn SystemParametersInfoW(
            action: u32,
            parameter: u32,
            value: *mut std::ffi::c_void,
            flags: u32,
        ) -> i32;
    }
    const SPI_GETCLIENTAREAANIMATION: u32 = 4162;
    let mut animations_enabled: i32 = 1;
    // The documented query writes a BOOL to the supplied pointer; no setting is changed.
    let success = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            (&mut animations_enabled as *mut i32).cast(),
            0,
        )
    };
    success != 0 && animations_enabled == 0
}

#[cfg(target_os = "macos")]
fn system_reduce_motion() -> bool {
    objc2_app_kit::NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
}

/// Parse only the standardized portal value. Zero means no preference, not a
/// request to enable motion, so it may fall back to the desktop's setting.
#[cfg(any(target_os = "linux", test))]
fn portal_reduced_motion(output: &str) -> Option<bool> {
    let compact: String = output.chars().filter(|ch| !ch.is_whitespace()).collect();
    let marker = "'reduced-motion':<uint32";
    let mut matches = compact.match_indices(marker);
    let (start, _) = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    let tail = &compact[start + marker.len()..];
    let digits = tail.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 || !tail[digits..].starts_with('>') {
        return None;
    }
    match &tail[..digits] {
        "1" => Some(true),
        _ => None,
    }
}

#[cfg(any(target_os = "linux", test))]
fn gnome_reduced_motion(output: &str) -> Option<bool> {
    match output.trim() {
        "false" => Some(true),
        "true" => Some(false),
        _ => None,
    }
}

/// Bound optional desktop queries so an unavailable portal cannot hold up
/// startup indefinitely. The child is terminated and reaped on timeout.
#[cfg(any(target_os = "linux", test))]
fn bounded_desktop_query(
    program: &str,
    args: &[&str],
    deadline: std::time::Instant,
) -> Option<String> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    use std::time::Duration;

    if std::time::Instant::now() >= deadline {
        return None;
    }
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    if !status.success() {
        return None;
    }
    let mut bytes = Vec::new();
    child
        .stdout
        .take()?
        .take(4096)
        .read_to_end(&mut bytes)
        .ok()?;
    String::from_utf8(bytes).ok()
}

#[cfg(any(target_os = "linux", test))]
fn linux_reduce_motion_from_desktop() -> bool {
    use std::sync::OnceLock;
    use std::time::{Duration, Instant};
    static SYSTEM_PREFERENCE: OnceLock<bool> = OnceLock::new();
    *SYSTEM_PREFERENCE.get_or_init(|| {
        let started = Instant::now();
        // Reserve time for GNOME's setting if the optional portal is slow.
        let portal_deadline = started + Duration::from_millis(250);
        let deadline = started + Duration::from_millis(500);
        // ReadAll tolerates portal backends that do not implement this key.
        let portal = bounded_desktop_query(
            "gdbus",
            &[
                "call",
                "--session",
                "--dest",
                "org.freedesktop.portal.Desktop",
                "--object-path",
                "/org/freedesktop/portal/desktop",
                "--method",
                "org.freedesktop.portal.Settings.ReadAll",
                "--timeout",
                "1",
                "['org.freedesktop.appearance']",
            ],
            portal_deadline,
        )
        .and_then(|output| portal_reduced_motion(&output));
        if portal == Some(true) {
            return true;
        }
        let is_gnome = std::env::var("XDG_CURRENT_DESKTOP")
            .ok()
            .is_some_and(|desktop| desktop.to_ascii_lowercase().contains("gnome"));
        if is_gnome {
            return bounded_desktop_query(
                "gsettings",
                &["get", "org.gnome.desktop.interface", "enable-animations"],
                deadline,
            )
            .and_then(|output| gnome_reduced_motion(&output))
            .unwrap_or(false);
        }
        false
    })
}

#[cfg(target_os = "linux")]
fn system_reduce_motion() -> bool {
    linux_reduce_motion_from_desktop()
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
fn system_reduce_motion() -> bool {
    false
}

impl std::fmt::Debug for AppSettings {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AppSettings")
            .field("provider_keys", &"[redacted]")
            .field("last_pdf_zoom", &self.last_pdf_zoom)
            .field("remembered_documents", &self.pdf_zoom_by_document.len())
            .finish_non_exhaustive()
    }
}

pub fn effective_openai_api_key(settings: &AppSettings) -> Option<String> {
    let configured = settings.openai_api_key.trim();
    if !configured.is_empty() {
        return Some(configured.to_owned());
    }
    std::env::var("OPENAI_API_KEY")
        .or_else(|_| std::env::var("LAWPDF_OPENAI_API_KEY"))
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

pub fn effective_openrouter_api_key(settings: &AppSettings) -> Option<String> {
    let configured = settings.openrouter_api_key.trim();
    if configured.is_empty() {
        std::env::var("LAWPDF_OPENROUTER_API_KEY")
            .or_else(|_| std::env::var("OPENROUTER_API_KEY"))
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    } else {
        Some(configured.to_owned())
    }
}

pub fn effective_groq_api_key(settings: &AppSettings) -> Option<String> {
    let configured = settings.groq_api_key.trim();
    if !configured.is_empty() {
        return Some(configured.to_owned());
    }
    std::env::var("LAWPDF_GROQ_API_KEY")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

pub fn effective_elevenlabs_api_key(settings: &AppSettings) -> Option<String> {
    let configured = settings.elevenlabs_api_key.trim();
    if !configured.is_empty() {
        return Some(configured.to_owned());
    }
    std::env::var("LAWPDF_ELEVENLABS_API_KEY")
        .or_else(|_| std::env::var("ELEVENLABS_API_KEY"))
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn default_openrouter_speech_model() -> String {
    "openai/gpt-4o-mini-tts-2025-12-15".to_owned()
}

fn default_openrouter_speech_voice() -> String {
    "nova".to_owned()
}

fn default_elevenlabs_speech_model() -> String {
    "eleven_flash_v2_5".to_owned()
}

fn default_elevenlabs_speech_voice() -> String {
    "JBFqnCBsd6RMkjVDRZzb".to_owned()
}

fn default_openrouter_api_key() -> String {
    String::new()
}

fn default_pdf_zoom() -> f32 {
    DEFAULT_PDF_ZOOM
}

fn default_true() -> bool {
    true
}

pub fn normalized_pdf_zoom(zoom: f32) -> f32 {
    if zoom.is_finite() {
        zoom.clamp(MIN_PDF_ZOOM, MAX_PDF_ZOOM)
    } else {
        DEFAULT_PDF_ZOOM
    }
}

pub fn pdf_document_key(path: &Path) -> String {
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    path_key_without_io(&path)
}

fn path_key_without_io(path: &Path) -> String {
    let key = path.to_string_lossy().replace('\\', "/");
    if cfg!(windows) {
        key.to_lowercase()
    } else {
        key
    }
}

fn bounded_session_paths(paths: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    paths
        .into_iter()
        .filter(|path| {
            let parsed = Path::new(path);
            !path.trim().is_empty()
                && parsed.is_absolute()
                && seen.insert(path_key_without_io(parsed))
        })
        .take(MAX_LAST_SESSION_DOCUMENTS)
        .collect()
}

impl AppSettings {
    pub fn remember_session_paths(
        &mut self,
        paths: impl IntoIterator<Item = String>,
        now_unix: u64,
    ) {
        self.last_session_paths = bounded_session_paths(paths);
        self.last_session_saved_unix = if self.last_session_paths.is_empty() {
            0
        } else {
            now_unix
        };
    }

    pub fn startup_paths(&self, requested: Vec<PathBuf>) -> Vec<PathBuf> {
        if !requested.is_empty() || !self.reopen_last_session {
            return requested;
        }
        self.last_session_paths.iter().map(PathBuf::from).collect()
    }

    pub fn recent_document(&self, path: &Path) -> Option<&RecentDocument> {
        let key = pdf_document_key(path);
        self.recent_documents
            .iter()
            .find(|recent| path_key_without_io(Path::new(&recent.path)) == key)
    }

    pub fn recent_document_for_opened_path(&self, path: &Path) -> Option<&RecentDocument> {
        let key = path_key_without_io(path);
        self.recent_documents
            .iter()
            .find(|recent| path_key_without_io(Path::new(&recent.path)) == key)
    }

    /// Update a record without touching the filesystem; opened paths are
    /// canonicalized when they enter the document-opening pipeline.
    pub fn remember_recent_document(&mut self, mut recent: RecentDocument, touch: bool) -> bool {
        if recent.path.trim().is_empty() {
            return false;
        }
        recent.page_fraction = if recent.page_fraction.is_finite() {
            recent.page_fraction.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let key = path_key_without_io(Path::new(&recent.path));
        let existing = self
            .recent_documents
            .iter()
            .position(|item| path_key_without_io(Path::new(&item.path)) == key);
        if let Some(index) = existing {
            if !touch && self.recent_documents[index] == recent {
                return false;
            }
            if touch {
                self.recent_documents.remove(index);
                self.recent_documents.insert(0, recent);
            } else {
                self.recent_documents[index] = recent;
            }
        } else {
            self.recent_documents.insert(0, recent);
        }
        self.recent_documents.truncate(MAX_RECENT_DOCUMENTS);
        true
    }

    pub fn prune_recent_documents(&mut self, now_unix: u64) {
        let age = u64::from(self.cache_retention_days.max(1)) * 86_400;
        let mut seen = std::collections::HashSet::new();
        self.recent_documents.retain_mut(|recent| {
            recent.page_fraction = if recent.page_fraction.is_finite() {
                recent.page_fraction.clamp(0.0, 1.0)
            } else {
                0.0
            };
            !recent.path.trim().is_empty()
                && now_unix.saturating_sub(recent.last_seen_unix) <= age
                && seen.insert(path_key_without_io(Path::new(&recent.path)))
        });
        self.recent_documents.truncate(MAX_RECENT_DOCUMENTS);
        if !self.reopen_last_session
            || self.last_session_saved_unix == 0
            || now_unix.saturating_sub(self.last_session_saved_unix) > age
        {
            self.last_session_paths.clear();
            self.last_session_saved_unix = 0;
        } else {
            self.last_session_paths =
                bounded_session_paths(std::mem::take(&mut self.last_session_paths));
            if self.last_session_paths.is_empty() {
                self.last_session_saved_unix = 0;
            }
        }
    }

    pub fn zoom_for_document(&self, path: &Path) -> Option<f32> {
        self.pdf_zoom_by_document
            .get(&pdf_document_key(path))
            .copied()
            .map(normalized_pdf_zoom)
    }

    pub fn remember_document_zoom(&mut self, path: &Path, zoom: f32) -> bool {
        let key = pdf_document_key(path);
        let zoom = normalized_pdf_zoom(zoom);
        if self
            .pdf_zoom_by_document
            .get(&key)
            .is_some_and(|saved| (*saved - zoom).abs() <= f32::EPSILON)
        {
            return false;
        }
        self.pdf_zoom_by_document.insert(key, zoom);
        true
    }
}

pub fn load_settings() -> AppSettings {
    let Some(path) = settings_path() else {
        return AppSettings::default();
    };
    let mut settings = load_settings_from(&path);
    match crate::credentials::load_and_migrate(&mut settings) {
        Ok(()) => {
            // The native store has acknowledged every legacy key before this
            // sanitized rewrite. No new plaintext backup is created.
            if path.exists() {
                if let Err(error) = save_settings_to(&settings, &path) {
                    settings.credential_error = Some(error);
                }
            }
        }
        Err(error) => settings.credential_error = Some(error),
    }
    settings
}

pub(crate) fn load_settings_from(path: &Path) -> AppSettings {
    let Ok(bytes) = std::fs::read(path) else {
        return AppSettings::default();
    };
    let mut settings: AppSettings = match serde_json::from_slice(&bytes) {
        Ok(settings) => settings,
        Err(_) => {
            let backup = corrupt_settings_backup_path(path);
            // The backup may contain legacy credentials. Keep it private and
            // do not print parser payloads, filenames, or provider key text.
            let _ = crate::atomic_file::write(&backup, &bytes);
            AppSettings::default()
        }
    };
    settings.last_pdf_zoom = normalized_pdf_zoom(settings.last_pdf_zoom);
    settings.system_speech_rate = settings.system_speech_rate.clamp(-5, 5);
    settings.pdf_zoom_by_document.retain(|key, zoom| {
        if key.trim().is_empty() {
            return false;
        }
        *zoom = normalized_pdf_zoom(*zoom);
        true
    });
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    settings.prune_recent_documents(now);
    if settings
        .window_placement
        .as_ref()
        .is_some_and(|placement| !placement.is_valid())
    {
        settings.window_placement = None;
    }
    settings
}

/// Read only window geometry before creating the native window. This avoids
/// loading credentials or migrating any settings during viewport setup.
pub fn load_window_placement() -> Option<WindowPlacement> {
    #[derive(Deserialize)]
    struct WindowOnly {
        #[serde(default)]
        window_placement: Option<WindowPlacement>,
    }
    let bytes = std::fs::read(settings_path()?).ok()?;
    let settings: WindowOnly = serde_json::from_slice(&bytes).ok()?;
    settings.window_placement.filter(WindowPlacement::is_valid)
}

pub fn save_settings(settings: &AppSettings) -> Result<(), String> {
    ensure_settings_save_allowed(settings)?;
    let path = settings_path().ok_or_else(|| "Could not find settings directory.".to_owned())?;
    save_settings_to(settings, &path)
}

fn ensure_settings_save_allowed(settings: &AppSettings) -> Result<(), String> {
    // A locked/unavailable keyring must not prevent saving local preferences.
    // Preserve the legacy JSON, however, until every in-memory legacy secret
    // has successfully migrated; serialized settings intentionally omit keys.
    if let Some(error) = &settings.credential_error {
        if [
            &settings.openrouter_api_key,
            &settings.openai_api_key,
            &settings.groq_api_key,
            &settings.elevenlabs_api_key,
        ]
        .iter()
        .any(|key| !key.is_empty())
        {
            return Err(error.clone());
        }
    }
    Ok(())
}

pub(crate) fn save_settings_to(settings: &AppSettings, path: &Path) -> Result<(), String> {
    ensure_settings_save_allowed(settings)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create settings folder: {error}"))?;
    }
    let bytes = serde_json::to_vec_pretty(settings)
        .map_err(|error| format!("Could not encode settings: {error}"))?;
    crate::atomic_file::write(path, &bytes)
        .map_err(|error| format!("Could not save settings: {error}"))
}

fn corrupt_settings_backup_path(path: &Path) -> PathBuf {
    let mut backup = path.as_os_str().to_os_string();
    backup.push(".bak");
    PathBuf::from(backup)
}

pub fn app_data_dir() -> Option<PathBuf> {
    #[cfg(all(feature = "devtools", not(test)))]
    if let Some(path) = std::env::var_os("LAWPDF_BENCH_DATA_DIR") {
        return Some(PathBuf::from(path));
    }
    #[cfg(feature = "devtools")]
    if crate::shutdown_smoke::running() {
        if let Some(path) = std::env::var_os("LAWPDF_SHUTDOWN_SMOKE_DATA_DIR") {
            return Some(PathBuf::from(path));
        }
        return Some(
            std::env::temp_dir().join(format!("lawpdf-shutdown-data-{}", std::process::id())),
        );
    }
    // Unit tests must never read, overwrite, or prune a user's application data.
    if cfg!(test) {
        return Some(std::env::temp_dir().join(format!("lawpdf-unit-data-{}", std::process::id())));
    }
    #[cfg(windows)]
    {
        return std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .map(|path| path.join("LawPDF"));
    }

    #[cfg(target_os = "macos")]
    {
        return std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|path| path.join("Library/Application Support/LawPDF"));
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .map(|path| path.join("LawPDF"))
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .map(|path| path.join(".local/share/LawPDF"))
            })
    }
}

fn settings_path() -> Option<PathBuf> {
    app_data_dir().map(|path| path.join("settings.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_appearance_follows_system_and_explicit_choices_round_trip() {
        let legacy: AppSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(legacy.appearance, Appearance::System);
        for appearance in [Appearance::System, Appearance::Light, Appearance::Dark] {
            let mut settings = AppSettings::default();
            settings.appearance = appearance;
            let encoded = serde_json::to_vec(&settings).unwrap();
            let restored: AppSettings = serde_json::from_slice(&encoded).unwrap();
            assert_eq!(restored.appearance, appearance);
        }
    }
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn session_restore_is_opt_in_bounded_and_obeys_history_retention() {
        let mut settings: AppSettings = serde_json::from_str("{}").unwrap();
        assert!(!settings.reopen_last_session);
        let base = std::env::temp_dir().join("lawpdf-session-test");
        let documents = (0..260)
            .map(|index| {
                base.join(format!("{index}.pdf"))
                    .to_string_lossy()
                    .into_owned()
            })
            .collect::<Vec<_>>();
        let mut candidates = vec![
            "relative.pdf".to_owned(),
            documents[0].clone(),
            documents[0].clone(),
        ];
        candidates.extend(documents.clone());
        settings.remember_session_paths(candidates, 200_000);
        assert_eq!(
            settings.last_session_paths.len(),
            MAX_LAST_SESSION_DOCUMENTS
        );
        assert_eq!(settings.last_session_paths[0], documents[0]);
        assert!(settings.startup_paths(Vec::new()).is_empty());
        settings.reopen_last_session = true;
        let serialized = serde_json::to_vec(&settings).unwrap();
        let round_trip: AppSettings = serde_json::from_slice(&serialized).unwrap();
        assert!(round_trip.reopen_last_session);
        assert_eq!(round_trip.last_session_paths, settings.last_session_paths);
        assert_eq!(round_trip.last_session_saved_unix, 200_000);
        assert_eq!(
            settings.startup_paths(Vec::new()).len(),
            MAX_LAST_SESSION_DOCUMENTS
        );
        let explicit = vec![base.join("explicit.pdf")];
        assert_eq!(settings.startup_paths(explicit.clone()), explicit);
        settings.cache_retention_days = 1;
        settings.prune_recent_documents(200_000 + 86_400);
        assert_eq!(
            settings.last_session_paths.len(),
            MAX_LAST_SESSION_DOCUMENTS
        );
        settings.prune_recent_documents(200_000 + 86_401);
        assert!(settings.last_session_paths.is_empty());
        assert_eq!(settings.last_session_saved_unix, 0);
    }

    #[test]
    fn initial_motion_preference_uses_system_but_saved_choice_wins() {
        let initial: AppSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(initial.reduce_motion, system_reduce_motion());
        for choice in [false, true] {
            let saved: AppSettings =
                serde_json::from_value(serde_json::json!({"reduce_motion": choice})).unwrap();
            assert_eq!(saved.reduce_motion, choice);
        }
    }

    #[test]
    fn desktop_motion_parsers_accept_only_known_unambiguous_values() {
        assert_eq!(
            portal_reduced_motion(
                "({'org.freedesktop.appearance': {'reduced-motion': <uint32 1>}},)"
            ),
            Some(true)
        );
        assert_eq!(
            portal_reduced_motion("({'reduced-motion': <uint32 0>},)"),
            None
        );
        assert_eq!(
            portal_reduced_motion("({'reduced-motion': <uint32 2>},)"),
            None
        );
        assert_eq!(
            portal_reduced_motion("({'reduced-motion': <uint32 10>},)"),
            None
        );
        assert_eq!(portal_reduced_motion("({'reduced-motion': <'1'>},)"), None);
        assert_eq!(portal_reduced_motion("({'other': <uint32 1>},)"), None);
        assert_eq!(
            portal_reduced_motion(
                "({'reduced-motion': <uint32 1>, 'reduced-motion': <uint32 0>},)"
            ),
            None
        );
        assert_eq!(gnome_reduced_motion("false\n"), Some(true));
        assert_eq!(gnome_reduced_motion("true\n"), Some(false));
        assert_eq!(gnome_reduced_motion("False"), None);
        assert_eq!(gnome_reduced_motion(""), None);
    }

    #[test]
    fn desktop_query_returns_output_and_stops_a_hung_child() {
        use std::time::{Duration, Instant};
        #[cfg(windows)]
        let (program, immediate, slow): (&str, &[&str], &[&str]) = (
            "powershell.exe",
            &[
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Write-Output ready",
            ],
            &[
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Start-Sleep -Seconds 4",
            ],
        );
        #[cfg(not(windows))]
        let (program, immediate, slow): (&str, &[&str], &[&str]) =
            ("sh", &["-c", "printf ready"], &["-c", "exec sleep 4"]);
        let output =
            bounded_desktop_query(program, immediate, Instant::now() + Duration::from_secs(3));
        assert_eq!(output.as_deref().map(str::trim), Some("ready"));
        let started = Instant::now();
        assert!(
            bounded_desktop_query(program, slow, started + Duration::from_millis(300)).is_none()
        );
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    fn temp_settings_path(test_name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir()
            .join(format!(
                "lawpdf-settings-{test_name}-{}-{nonce}",
                std::process::id()
            ))
            .join("settings.json")
    }

    fn recent(path: String, at: u64) -> RecentDocument {
        RecentDocument {
            path,
            title: "Article".into(),
            authors: "Author".into(),
            citation: "Journal".into(),
            page: 3,
            page_fraction: 0.4,
            view: ReadingView::Review,
            review_block: Some(17),
            last_seen_unix: at,
        }
    }

    #[test]
    fn recent_history_is_bounded_reordered_and_expires_by_retention() {
        let mut settings = AppSettings::default();
        for index in 0..503 {
            assert!(
                settings
                    .remember_recent_document(recent(format!("paper-{index}.pdf"), 200_000), true)
            );
        }
        assert_eq!(settings.recent_documents.len(), MAX_RECENT_DOCUMENTS);
        assert_eq!(settings.recent_documents[0].path, "paper-502.pdf");
        let mut selected = settings.recent_documents[100].clone();
        selected.page_fraction = 2.0;
        assert!(settings.remember_recent_document(selected, true));
        assert_eq!(settings.recent_documents[0].page_fraction, 1.0);
        settings.cache_retention_days = 1;
        settings.prune_recent_documents(200_000 + 86_401);
        assert!(settings.recent_documents.is_empty());
    }

    #[test]
    fn reading_history_round_trips_without_provider_credentials() {
        let path = temp_settings_path("reading-history");
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let mut settings = AppSettings::default();
        settings.remember_recent_document(recent("reading.pdf".into(), now), true);
        save_settings_to(&settings, &path).unwrap();
        let restored = load_settings_from(&path);
        let record = restored.recent_document(Path::new("reading.pdf")).unwrap();
        assert_eq!(
            (
                record.page,
                record.page_fraction,
                record.view,
                record.review_block
            ),
            (3, 0.4, ReadingView::Review, Some(17))
        );
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn corrupt_json_returns_defaults_and_creates_backup() {
        let path = temp_settings_path("corrupt");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"{not valid json").unwrap();

        let settings = load_settings_from(&path);
        let backup = corrupt_settings_backup_path(&path);

        assert_eq!(settings.last_pdf_zoom, DEFAULT_PDF_ZOOM);
        assert_eq!(std::fs::read(&backup).unwrap(), b"{not valid json");
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn zoom_normalization_handles_nonfinite_and_bounds() {
        assert_eq!(normalized_pdf_zoom(f32::NAN), DEFAULT_PDF_ZOOM);
        assert_eq!(normalized_pdf_zoom(f32::INFINITY), DEFAULT_PDF_ZOOM);
        assert_eq!(normalized_pdf_zoom(f32::NEG_INFINITY), DEFAULT_PDF_ZOOM);
        assert_eq!(normalized_pdf_zoom(0.1), MIN_PDF_ZOOM);
        assert_eq!(normalized_pdf_zoom(9.0), MAX_PDF_ZOOM);
        assert_eq!(normalized_pdf_zoom(2.0), 2.0);
    }

    #[test]
    fn older_settings_do_not_dim_original_pages_by_default() {
        let settings: AppSettings = serde_json::from_str("{}").unwrap();
        assert!(!settings.dim_original_pages);
    }

    #[test]
    fn unavailable_keyring_still_allows_local_preferences_without_secrets() {
        let path = temp_settings_path("unavailable-keyring-preferences");
        let settings = AppSettings {
            credential_error: Some("keyring unavailable".to_owned()),
            speech_provider: crate::tts::SpeechProvider::ElevenLabs,
            ..AppSettings::default()
        };
        save_settings_to(&settings, &path).unwrap();
        let restored = load_settings_from(&path);
        assert_eq!(
            restored.speech_provider,
            crate::tts::SpeechProvider::ElevenLabs
        );
        assert!(restored.elevenlabs_api_key.is_empty());
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn unavailable_keyring_preserves_every_unmigrated_provider_secret() {
        let path = temp_settings_path("unavailable-keyring-legacy");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let legacy = b"existing legacy credential settings";
        std::fs::write(&path, legacy).unwrap();
        for provider in 0..4 {
            let mut settings = AppSettings {
                credential_error: Some("keyring unavailable".to_owned()),
                ..AppSettings::default()
            };
            let field = match provider {
                0 => &mut settings.openrouter_api_key,
                1 => &mut settings.openai_api_key,
                2 => &mut settings.groq_api_key,
                _ => &mut settings.elevenlabs_api_key,
            };
            *field = "non-secret-legacy-test-key".to_owned();
            assert_eq!(
                save_settings_to(&settings, &path).unwrap_err(),
                "keyring unavailable"
            );
            assert_eq!(std::fs::read(&path).unwrap(), legacy);
        }
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn settings_round_trip_through_injected_path() {
        let path = temp_settings_path("roundtrip");
        let expected = AppSettings {
            openrouter_api_key: "router".to_owned(),
            openai_api_key: "openai".to_owned(),
            groq_api_key: "groq".to_owned(),
            elevenlabs_api_key: "elevenlabs-secret".to_owned(),
            speech_provider: crate::tts::SpeechProvider::ElevenLabs,
            last_pdf_zoom: 2.25,
            pdf_zoom_by_document: BTreeMap::from([
                ("c:/docs/first.pdf".to_owned(), 1.5),
                ("c:/docs/second.pdf".to_owned(), 2.0),
            ]),
            reduce_motion: true,
            dim_original_pages: true,
            optimize_large_documents: false,
            liquid_mode2_use_pymupdf_blocks: true,
            liquid_mode2_use_pp_footnote_regions: true,
            markdown_copy_footnotes: FootnoteMode::Endnotes,
            markdown_copy_include_tables: false,
            markdown_copy_include_metadata: true,
            review_intro_dismissed: true,
            macos_default_reader_prompt_dismissed: true,
            ..AppSettings::default()
        };

        save_settings_to(&expected, &path).unwrap();
        let actual = load_settings_from(&path);

        assert!(actual.openrouter_api_key.is_empty());
        assert!(actual.openai_api_key.is_empty());
        assert!(actual.groq_api_key.is_empty());
        assert!(actual.elevenlabs_api_key.is_empty());
        assert_eq!(
            actual.speech_provider,
            crate::tts::SpeechProvider::ElevenLabs
        );
        let serialized = std::fs::read_to_string(&path).unwrap();
        assert!(!serialized.contains("api_key"));
        assert!(!serialized.contains("elevenlabs-secret"));
        assert!(!format!("{expected:?}").contains("router"));
        assert_eq!(actual.last_pdf_zoom, expected.last_pdf_zoom);
        assert_eq!(actual.pdf_zoom_by_document, expected.pdf_zoom_by_document);
        assert!(actual.review_intro_dismissed);
        assert_eq!(actual.reduce_motion, expected.reduce_motion);
        assert_eq!(actual.dim_original_pages, expected.dim_original_pages);
        assert_eq!(
            actual.optimize_large_documents,
            expected.optimize_large_documents
        );
        assert_eq!(
            actual.liquid_mode2_use_pymupdf_blocks,
            expected.liquid_mode2_use_pymupdf_blocks
        );
        assert_eq!(
            actual.liquid_mode2_use_pp_footnote_regions,
            expected.liquid_mode2_use_pp_footnote_regions
        );
        assert_eq!(
            actual.markdown_copy_footnotes,
            expected.markdown_copy_footnotes
        );
        assert_eq!(
            actual.markdown_copy_include_tables,
            expected.markdown_copy_include_tables
        );
        assert_eq!(
            actual.markdown_copy_include_metadata,
            expected.markdown_copy_include_metadata
        );
        assert_eq!(
            actual.macos_default_reader_prompt_dismissed,
            expected.macos_default_reader_prompt_dismissed
        );
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn document_zoom_is_path_specific_and_normalized() {
        let mut settings = AppSettings::default();
        let first = Path::new("C:/docs/First.pdf");
        let second = Path::new("C:/docs/Second.pdf");

        assert!(settings.remember_document_zoom(first, 1.8));
        assert!(!settings.remember_document_zoom(first, 1.8));
        assert!(settings.remember_document_zoom(second, 9.0));

        assert_eq!(settings.zoom_for_document(first), Some(1.8));
        assert_eq!(settings.zoom_for_document(second), Some(MAX_PDF_ZOOM));
        assert_eq!(
            settings.zoom_for_document(Path::new("C:/docs/Other.pdf")),
            None
        );
    }

    #[test]
    fn legacy_settings_without_document_zoom_map_still_load() {
        let path = temp_settings_path("legacy-zoom");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, br#"{"last_pdf_zoom":1.6}"#).unwrap();

        let settings = load_settings_from(&path);

        assert_eq!(settings.last_pdf_zoom, 1.6);
        assert!(settings.pdf_zoom_by_document.is_empty());
        assert_eq!(settings.markdown_copy_footnotes, FootnoteMode::Inline);
        assert!(settings.markdown_copy_include_tables);
        assert!(!settings.markdown_copy_include_metadata);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
