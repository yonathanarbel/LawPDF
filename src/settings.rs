use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppSettings {
    #[serde(default)]
    pub openrouter_api_key: String,
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
