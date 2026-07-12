use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use super::{
    LM2_SCHEMA_VERSION, LiquidDocument, app_data_dir, attach_footnote_links,
    load_cached_lm2_document,
};

pub fn load_fast_cached_liquid_mode2_document(
    path: &Path,
    use_pymupdf_blocks: bool,
    use_pp_footnote_regions: bool,
) -> Option<LiquidDocument> {
    let pointer_path = lm2_fast_cache_path(path, use_pymupdf_blocks, use_pp_footnote_regions)?;
    let source_signature = std::fs::read_to_string(pointer_path).ok()?;
    let mut document = load_cached_lm2_document(source_signature.trim())?;
    if document.blocks.is_empty() || document.block_source_lines.is_empty() {
        return None;
    }
    attach_footnote_links(&mut document);
    Some(document)
}

pub fn save_fast_cached_lm2_document(
    source_path: &Path,
    use_pymupdf_blocks: bool,
    use_pp_footnote_regions: bool,
    document: &LiquidDocument,
) -> Result<(), String> {
    let path = lm2_fast_cache_path(source_path, use_pymupdf_blocks, use_pp_footnote_regions)
        .ok_or_else(|| "Could not find LiquidMode2 fast-cache directory.".to_owned())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create LiquidMode2 fast cache: {error}"))?;
    }
    std::fs::write(path, format!("{}\n", document.source_signature))
        .map_err(|error| error.to_string())
}

pub(super) fn lm2_fast_cache_path(
    source_path: &Path,
    use_pymupdf_blocks: bool,
    use_pp_footnote_regions: bool,
) -> Option<PathBuf> {
    let metadata = std::fs::metadata(source_path).ok()?;
    let modified = metadata
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_nanos();
    let canonical =
        std::fs::canonicalize(source_path).unwrap_or_else(|_| source_path.to_path_buf());
    let mut hasher = DefaultHasher::new();
    LM2_SCHEMA_VERSION.hash(&mut hasher);
    canonical.hash(&mut hasher);
    metadata.len().hash(&mut hasher);
    modified.hash(&mut hasher);
    use_pymupdf_blocks.hash(&mut hasher);
    use_pp_footnote_regions.hash(&mut hasher);
    let mut runtime_env = std::env::vars_os()
        .filter_map(|(name, value)| {
            let name = name.to_string_lossy();
            (name.starts_with("LAWPDF_LM2_") || name == "LAWPDF_LMV")
                .then_some((name.into_owned(), value.to_string_lossy().into_owned()))
        })
        .collect::<Vec<_>>();
    runtime_env.sort();
    runtime_env.hash(&mut hasher);
    let key = hasher.finish();
    app_data_dir().map(|dir| {
        dir.join("liquid2-fast-cache")
            .join(format!("{key:016x}.pointer"))
    })
}
