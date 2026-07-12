//! Liquid Mode cache keys and persistence helpers.

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::settings::app_data_dir;

use super::config::{LIQUID_LAYOUT_MODEL_VERSION, LIQUID_SCHEMA_VERSION};
use super::model::LiquidDocument;
use super::util::stable_hash;

pub(super) fn load_cached_document(source_signature: &str) -> Option<LiquidDocument> {
    let path = cache_path(source_signature)?;
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice::<LiquidDocument>(&bytes).ok()
}

pub(super) fn save_cached_document(document: &LiquidDocument) -> Result<(), String> {
    let path = cache_path(&document.source_signature)
        .ok_or_else(|| "Could not find Review Mode cache directory.".to_owned())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create Review Mode cache: {error}"))?;
    }
    let bytes = serde_json::to_vec(document).map_err(|error| error.to_string())?;
    std::fs::write(path, bytes).map_err(|error| error.to_string())
}

fn cache_path(source_signature: &str) -> Option<PathBuf> {
    app_data_dir().map(|dir| {
        dir.join("liquid-cache")
            .join(format!("{source_signature}.json"))
    })
}

pub(super) fn source_signature(path: &Path, pages: &[String]) -> String {
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let metadata = std::fs::metadata(path).ok();
    let modified = metadata
        .as_ref()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    let len = metadata.map(|metadata| metadata.len()).unwrap_or_default();
    let page_text_signature = stable_hash(&pages.join("\n\u{0c}\n"));
    stable_hash(&format!(
        "v{LIQUID_SCHEMA_VERSION}|layout={LIQUID_LAYOUT_MODEL_VERSION}|{}|{modified}|{len}|{page_text_signature}",
        canonical.display()
    ))
}
