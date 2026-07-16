use std::path::{Path, PathBuf};

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
    load_settings_from(&path)
}

pub(crate) fn load_settings_from(path: &Path) -> AppSettings {
    let Ok(bytes) = std::fs::read(path) else {
        return AppSettings::default();
    };
    let mut settings: AppSettings = match serde_json::from_slice(&bytes) {
        Ok(settings) => settings,
        Err(error) => {
            let backup = corrupt_settings_backup_path(path);
            match std::fs::copy(path, &backup) {
                Ok(_) => eprintln!(
                    "LawPDF settings parse failed ({error}); copied corrupt file to {}.",
                    backup.display()
                ),
                Err(backup_error) => eprintln!(
                    "LawPDF settings parse failed ({error}); backup to {} failed: {backup_error}.",
                    backup.display()
                ),
            }
            AppSettings::default()
        }
    };
    settings.last_pdf_zoom = normalized_pdf_zoom(settings.last_pdf_zoom);
    settings
}

pub fn save_settings(settings: &AppSettings) -> Result<(), String> {
    let path = settings_path().ok_or_else(|| "Could not find settings directory.".to_owned())?;
    save_settings_to(settings, &path)
}

pub(crate) fn save_settings_to(settings: &AppSettings, path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create settings folder: {error}"))?;
    }
    let bytes = serde_json::to_vec_pretty(settings)
        .map_err(|error| format!("Could not encode settings: {error}"))?;
    std::fs::write(path, bytes).map_err(|error| format!("Could not save settings: {error}"))
}

fn corrupt_settings_backup_path(path: &Path) -> PathBuf {
    let mut backup = path.as_os_str().to_os_string();
    backup.push(".bak");
    PathBuf::from(backup)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

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
    fn settings_round_trip_through_injected_path() {
        let path = temp_settings_path("roundtrip");
        let expected = AppSettings {
            openrouter_api_key: "router".to_owned(),
            openai_api_key: "openai".to_owned(),
            groq_api_key: "groq".to_owned(),
            last_pdf_zoom: 2.25,
            reduce_motion: true,
            liquid_mode2_use_pymupdf_blocks: true,
            liquid_mode2_use_pp_footnote_regions: true,
        };

        save_settings_to(&expected, &path).unwrap();
        let actual = load_settings_from(&path);

        assert_eq!(actual.openrouter_api_key, expected.openrouter_api_key);
        assert_eq!(actual.openai_api_key, expected.openai_api_key);
        assert_eq!(actual.groq_api_key, expected.groq_api_key);
        assert_eq!(actual.last_pdf_zoom, expected.last_pdf_zoom);
        assert_eq!(actual.reduce_motion, expected.reduce_motion);
        assert_eq!(
            actual.liquid_mode2_use_pymupdf_blocks,
            expected.liquid_mode2_use_pymupdf_blocks
        );
        assert_eq!(
            actual.liquid_mode2_use_pp_footnote_regions,
            expected.liquid_mode2_use_pp_footnote_regions
        );
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
