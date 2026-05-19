use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    #[serde(default = "default_openrouter_api_key")]
    pub openrouter_api_key: String,
    #[serde(default)]
    pub groq_api_key: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            openrouter_api_key: default_openrouter_api_key(),
            groq_api_key: String::new(),
        }
    }
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

pub fn load_settings() -> AppSettings {
    let Some(path) = settings_path() else {
        return AppSettings::default();
    };
    let Ok(bytes) = std::fs::read(path) else {
        return AppSettings::default();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
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
