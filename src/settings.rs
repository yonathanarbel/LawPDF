use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub const DEFAULT_PDF_ZOOM: f32 = 1.25;
pub const MIN_PDF_ZOOM: f32 = 0.35;
pub const MAX_PDF_ZOOM: f32 = 5.0;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    #[serde(default = "default_openrouter_api_key")]
    pub openrouter_api_key: String,
    #[serde(default)]
    pub openai_api_key: String,
    #[serde(default)]
    pub groq_api_key: String,
    #[serde(default = "default_pdf_zoom")]
    pub last_pdf_zoom: f32,
    /// When true, highlights appear instantly instead of animating the
    /// "laying-down" ink stroke. Honors users who prefer reduced motion.
    #[serde(default)]
    pub reduce_motion: bool,
    #[serde(default)]
    pub liquid_mode2_use_pymupdf_blocks: bool,
    #[serde(default)]
    pub liquid_mode2_use_pp_footnote_regions: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            openrouter_api_key: default_openrouter_api_key(),
            openai_api_key: String::new(),
            groq_api_key: String::new(),
            last_pdf_zoom: DEFAULT_PDF_ZOOM,
            reduce_motion: false,
            liquid_mode2_use_pymupdf_blocks: false,
            liquid_mode2_use_pp_footnote_regions: false,
        }
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

fn default_openrouter_api_key() -> String {
    String::new()
}

fn default_pdf_zoom() -> f32 {
    DEFAULT_PDF_ZOOM
}

pub fn normalized_pdf_zoom(zoom: f32) -> f32 {
    if zoom.is_finite() {
        zoom.clamp(MIN_PDF_ZOOM, MAX_PDF_ZOOM)
    } else {
        DEFAULT_PDF_ZOOM
    }
}

pub fn load_settings() -> AppSettings {
    let Some(path) = settings_path() else {
        return AppSettings::default();
    };
    let Ok(bytes) = std::fs::read(path) else {
        return AppSettings::default();
    };
    let mut settings: AppSettings = serde_json::from_slice(&bytes).unwrap_or_default();
    settings.last_pdf_zoom = normalized_pdf_zoom(settings.last_pdf_zoom);
    settings
}

pub fn save_settings(settings: &AppSettings) -> Result<(), String> {
    let path = settings_path().ok_or_else(|| "Could not find settings directory.".to_owned())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create settings folder: {error}"))?;
    }
    let bytes = serde_json::to_vec_pretty(settings)
        .map_err(|error| format!("Could not encode settings: {error}"))?;
    std::fs::write(path, bytes).map_err(|error| format!("Could not save settings: {error}"))
}

pub fn app_data_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .map(|path| path.join("LawPDF"))
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|path| path.join(".lawpdf"))
        })
}

fn settings_path() -> Option<PathBuf> {
    app_data_dir().map(|path| path.join("settings.json"))
}
