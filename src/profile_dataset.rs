use std::collections::{BTreeMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use serde::Serialize;

use crate::liquid::{
    LiquidBlockRole, LiquidRequest, prepare_liquid_document, should_prefer_ocr_page_text,
    should_try_ocr_page_text,
};
use crate::ocr::ocr_page_with_engine;
use crate::pdf_backend::PdfEngine;

const LIQUID_SOURCE_TEXT_LIMIT: usize = 100_000;

#[derive(Debug, Serialize)]
struct ProfileDatasetManifest {
    app_version: &'static str,
    timestamp_unix_secs: u64,
    seed: u64,
    candidate_count: usize,
    sample_count: usize,
    skipped_prediction_error_count: usize,
    roots: Vec<String>,
    items: Vec<ProfileDatasetItem>,
}

#[derive(Debug, Serialize)]
struct ProfileDatasetItem {
    path: String,
    name: String,
    folder_bucket: String,
    size_bytes: u64,
    modified_unix_secs: Option<u64>,
    label_primary: Option<String>,
    label_secondary: Option<String>,
    label_confidence: Option<String>,
    flags: Vec<String>,
    reviewer_notes: Option<String>,
    predicted_profile: Option<String>,
    predicted_confidence: Option<f32>,
    page_count: Option<usize>,
    extracted_pages: Option<usize>,
    extracted_chars: Option<usize>,
    ocr_pages: Option<usize>,
    ocr_chars: Option<usize>,
    ocr_errors: Vec<String>,
    detected_title: Option<String>,
    liquid_source_text: Option<String>,
    liquid_block_count: Option<usize>,
    liquid_role_counts: BTreeMap<String, usize>,
    liquid_noise_lines_removed: Option<usize>,
    liquid_warnings: Vec<String>,
    liquid_samples: Vec<String>,
    prediction_error: Option<String>,
}

pub fn run_profile_dataset(args: impl IntoIterator<Item = OsString>) -> Result<()> {
    let mut output_path = None;
    let mut roots = Vec::new();
    let mut limit = 500usize;
    let mut seed = 20260529u64;
    let mut predict = false;
    let mut max_predict_pages = 12usize;
    let mut ocr_empty_pages = false;
    let mut ocr_sparse_pages = false;
    let mut max_ocr_pages = 3usize;
    let mut max_file_bytes = 80 * 1024 * 1024u64;
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        if arg == OsStr::new("--profile-dataset") {
            continue;
        }
        if arg == OsStr::new("--profile-dataset-output") {
            output_path = Some(PathBuf::from(
                args.next()
                    .context("--profile-dataset-output needs a destination path")?,
            ));
            continue;
        }
        if arg == OsStr::new("--profile-dataset-root") {
            roots.push(PathBuf::from(
                args.next()
                    .context("--profile-dataset-root needs a directory path")?,
            ));
            continue;
        }
        if arg == OsStr::new("--profile-dataset-limit") {
            let value = args
                .next()
                .context("--profile-dataset-limit needs a positive integer")?;
            limit = value
                .to_string_lossy()
                .parse::<usize>()
                .context("invalid --profile-dataset-limit")?;
            continue;
        }
        if arg == OsStr::new("--profile-dataset-seed") {
            let value = args
                .next()
                .context("--profile-dataset-seed needs a positive integer")?;
            seed = value
                .to_string_lossy()
                .parse::<u64>()
                .context("invalid --profile-dataset-seed")?;
            continue;
        }
        if arg == OsStr::new("--profile-dataset-predict") {
            predict = true;
            continue;
        }
        if arg == OsStr::new("--profile-dataset-ocr-empty-pages") {
            predict = true;
            ocr_empty_pages = true;
            continue;
        }
        if arg == OsStr::new("--profile-dataset-ocr-sparse-pages") {
            predict = true;
            ocr_sparse_pages = true;
            continue;
        }
        if arg == OsStr::new("--profile-dataset-max-predict-pages") {
            let value = args
                .next()
                .context("--profile-dataset-max-predict-pages needs a positive integer")?;
            max_predict_pages = value
                .to_string_lossy()
                .parse::<usize>()
                .context("invalid --profile-dataset-max-predict-pages")?;
            continue;
        }
        if arg == OsStr::new("--profile-dataset-max-ocr-pages") {
            let value = args
                .next()
                .context("--profile-dataset-max-ocr-pages needs a positive integer")?;
            max_ocr_pages = value
                .to_string_lossy()
                .parse::<usize>()
                .context("invalid --profile-dataset-max-ocr-pages")?;
            continue;
        }
        if arg == OsStr::new("--profile-dataset-max-file-mb") {
            let value = args
                .next()
                .context("--profile-dataset-max-file-mb needs a positive integer")?;
            let megabytes = value
                .to_string_lossy()
                .parse::<u64>()
                .context("invalid --profile-dataset-max-file-mb")?;
            max_file_bytes = megabytes.saturating_mul(1024 * 1024);
            continue;
        }
        if arg.to_string_lossy().starts_with("--") {
            bail!(
                "unknown profile dataset argument: {}",
                arg.to_string_lossy()
            );
        }
        roots.push(PathBuf::from(arg));
    }

    if roots.is_empty() {
        roots = default_roots();
    }
    roots.retain(|root| root.is_dir());
    roots = roots
        .into_iter()
        .map(|root| fs::canonicalize(&root).unwrap_or(root))
        .collect::<Vec<_>>();
    if roots.is_empty() {
        bail!("no existing profile dataset roots found");
    }
    if limit == 0 {
        bail!("--profile-dataset-limit must be greater than zero");
    }

    let candidates = collect_pdf_candidates(&roots, max_file_bytes);
    let sampled_limit = if predict { candidates.len() } else { limit };
    let sampled = sample_paths(candidates.clone(), &roots, sampled_limit, seed);
    let engine = if predict {
        Some(PdfEngine::new().context("failed to initialize PDF engine")?)
    } else {
        None
    };
    let mut skipped_prediction_error_count = 0usize;
    let mut items = Vec::new();
    for path in sampled {
        let Some(mut item) = item_for_path(&roots, &path) else {
            continue;
        };
        if let Some(engine) = engine.as_ref() {
            predict_item(
                engine,
                &mut item,
                PredictionOptions {
                    max_pages: max_predict_pages,
                    ocr_empty_pages,
                    ocr_sparse_pages,
                    max_ocr_pages,
                },
            );
            if item.predicted_profile.is_none() && item.prediction_error.is_some() {
                skipped_prediction_error_count += 1;
                continue;
            }
        }
        items.push(item);
        if items.len() >= limit {
            break;
        }
    }
    let manifest = ProfileDatasetManifest {
        app_version: env!("CARGO_PKG_VERSION"),
        timestamp_unix_secs: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        seed,
        candidate_count: candidates.len(),
        sample_count: items.len(),
        skipped_prediction_error_count,
        roots: roots
            .iter()
            .map(|root| root.display().to_string())
            .collect::<Vec<_>>(),
        items,
    };

    let json = serde_json::to_string_pretty(&manifest)?;
    if let Some(output_path) = output_path {
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        fs::write(&output_path, json)
            .with_context(|| format!("failed to write {}", output_path.display()))?;
        println!(
            "Profile dataset sampled {} of {} PDF candidate(s) -> {}",
            manifest.sample_count,
            manifest.candidate_count,
            output_path.display()
        );
    } else {
        println!("{json}");
    }

    Ok(())
}

fn default_roots() -> Vec<PathBuf> {
    let home = std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Users\yonat"));
    [
        home.join("Box"),
        home.join("Downloads"),
        home.join("Documents"),
        home.join("Desktop"),
        home.join("OneDrive"),
    ]
    .into_iter()
    .collect()
}

fn collect_pdf_candidates(roots: &[PathBuf], max_file_bytes: u64) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut output = Vec::new();
    for root in roots {
        collect_pdf_candidates_from(root, max_file_bytes, &mut seen, &mut output);
    }
    output
}

fn collect_pdf_candidates_from(
    path: &Path,
    max_file_bytes: u64,
    seen: &mut HashSet<PathBuf>,
    output: &mut Vec<PathBuf>,
) {
    if should_skip_dir(path) {
        return;
    }
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            collect_pdf_candidates_from(&path, max_file_bytes, seen, output);
        } else if file_type.is_file() && is_pdf_path(&path) {
            if entry
                .metadata()
                .is_ok_and(|metadata| metadata.len() > max_file_bytes)
            {
                continue;
            }
            let canonical = fs::canonicalize(&path).unwrap_or(path);
            if seen.insert(canonical.clone()) {
                output.push(canonical);
            }
        }
    }
}

fn should_skip_dir(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    matches!(
        name.to_ascii_lowercase().as_str(),
        ".git"
            | "target"
            | "appdata"
            | ".cargo"
            | "anaconda3"
            | "node_modules"
            | ".cache"
            | "cache"
            | "temp"
            | "tmp"
            | "__pycache__"
            | "site-packages"
            | "pkgs"
    )
}

fn is_pdf_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))
}

fn sample_paths(paths: Vec<PathBuf>, roots: &[PathBuf], limit: usize, seed: u64) -> Vec<PathBuf> {
    let mut buckets: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for path in paths {
        buckets
            .entry(folder_bucket(roots, &path))
            .or_default()
            .push(path);
    }
    let mut buckets = buckets
        .into_values()
        .map(|mut paths| {
            paths.sort_by_key(|path| stable_sample_key(path, seed));
            paths
        })
        .collect::<Vec<_>>();
    buckets.sort_by_key(|paths| std::cmp::Reverse(paths.len()));

    let mut output = Vec::new();
    let mut round = 0usize;
    while output.len() < limit {
        let mut added = false;
        for bucket in &buckets {
            if let Some(path) = bucket.get(round) {
                output.push(path.clone());
                added = true;
                if output.len() >= limit {
                    break;
                }
            }
        }
        if !added {
            break;
        }
        round += 1;
    }
    output
}

fn stable_sample_key(path: &Path, seed: u64) -> u64 {
    let mut hash = 0xcbf29ce484222325u64 ^ seed;
    for byte in path.display().to_string().bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn item_for_path(roots: &[PathBuf], path: &Path) -> Option<ProfileDatasetItem> {
    let metadata = fs::metadata(path).ok()?;
    let modified_unix_secs = metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs());
    Some(ProfileDatasetItem {
        path: path.display().to_string(),
        name: path.file_name()?.to_string_lossy().to_string(),
        folder_bucket: folder_bucket(roots, path),
        size_bytes: metadata.len(),
        modified_unix_secs,
        label_primary: None,
        label_secondary: None,
        label_confidence: None,
        flags: Vec::new(),
        reviewer_notes: None,
        predicted_profile: None,
        predicted_confidence: None,
        page_count: None,
        extracted_pages: None,
        extracted_chars: None,
        ocr_pages: None,
        ocr_chars: None,
        ocr_errors: Vec::new(),
        detected_title: None,
        liquid_source_text: None,
        liquid_block_count: None,
        liquid_role_counts: BTreeMap::new(),
        liquid_noise_lines_removed: None,
        liquid_warnings: Vec::new(),
        liquid_samples: Vec::new(),
        prediction_error: None,
    })
}

fn folder_bucket(roots: &[PathBuf], path: &Path) -> String {
    for root in roots {
        if let Ok(relative) = path.strip_prefix(root) {
            let first = relative
                .components()
                .next()
                .map(|component| component.as_os_str().to_string_lossy().to_string())
                .unwrap_or_else(|| "(root)".to_owned());
            let root_name = root
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| root.display().to_string());
            return format!("{root_name}\\{first}");
        }
    }
    "other".to_owned()
}

#[derive(Debug, Clone, Copy)]
struct PredictionOptions {
    max_pages: usize,
    ocr_empty_pages: bool,
    ocr_sparse_pages: bool,
    max_ocr_pages: usize,
}

fn predict_item(engine: &PdfEngine, item: &mut ProfileDatasetItem, options: PredictionOptions) {
    let path = Path::new(&item.path);
    let document = match engine.load_document(path) {
        Ok(document) => document,
        Err(error) => {
            item.prediction_error = Some(error.to_string());
            return;
        }
    };
    item.page_count = Some(document.page_count);

    let mut pages = Vec::new();
    let mut extracted_pages = 0usize;
    let mut extracted_chars = 0usize;
    let mut ocr_pages = 0usize;
    let mut ocr_chars = 0usize;
    let mut ocr_attempts = 0usize;
    let mut ocr_errors = Vec::new();
    for page_index in 0..document.page_count.min(options.max_pages.max(1)) {
        let mut text = match engine.load_page_text(path, page_index) {
            Ok(text) => text,
            Err(error) => {
                item.prediction_error = Some(error.to_string());
                String::new()
            }
        };

        if should_try_ocr_page_text(&text, options.ocr_empty_pages, options.ocr_sparse_pages)
            && ocr_attempts < options.max_ocr_pages
        {
            ocr_attempts += 1;
            match ocr_page_with_engine(engine, path, page_index) {
                Ok(ocr_text) => {
                    if should_prefer_ocr_page_text(&text, &ocr_text) {
                        ocr_pages += 1;
                        ocr_chars += ocr_text.chars().count();
                        text = ocr_text;
                    }
                }
                Err(error) => {
                    ocr_errors.push(format!("page {}: {error}", page_index + 1));
                }
            }
        }

        if !text.trim().is_empty() {
            extracted_pages += 1;
            extracted_chars += text.chars().count();
        }
        pages.push(text);
    }
    item.extracted_pages = Some(extracted_pages);
    item.extracted_chars = Some(extracted_chars);
    item.ocr_pages = Some(ocr_pages);
    item.ocr_chars = Some(ocr_chars);
    item.ocr_errors = ocr_errors;
    item.liquid_source_text = Some(compact_source_text(&pages));

    match prepare_liquid_document(LiquidRequest {
        document_epoch: 0,
        path: path.to_path_buf(),
        title: document.title,
        pages,
        layout_hints: Vec::new(),
        source_line_hints: Vec::new(),
        deep_source_lines: Vec::new(),
        deep_liquid: None,
        groq_api_key: None,
        openrouter_api_key: None,
    }) {
        Ok(liquid) => {
            item.detected_title = Some(liquid.title);
            if let Some(profile) = liquid.profile {
                item.predicted_profile = Some(profile.kind.as_str().to_owned());
                item.predicted_confidence = Some(profile.confidence);
            }
            item.liquid_block_count = Some(liquid.blocks.len());
            item.liquid_noise_lines_removed = Some(liquid.noise_lines_removed);
            item.liquid_warnings = liquid.warnings;
            for block in &liquid.blocks {
                *item
                    .liquid_role_counts
                    .entry(block.role.prompt_name().to_owned())
                    .or_default() += 1;
            }
            item.liquid_samples = liquid
                .blocks
                .iter()
                .filter(|block| {
                    !matches!(
                        block.role,
                        LiquidBlockRole::Title
                            | LiquidBlockRole::Header
                            | LiquidBlockRole::Footer
                            | LiquidBlockRole::SectionBreak
                            | LiquidBlockRole::Contents
                            | LiquidBlockRole::Table
                            | LiquidBlockRole::Syllabus
                    )
                })
                .take(8)
                .map(|block| compact_sample(&block.text))
                .collect();
        }
        Err(error) => item.prediction_error = Some(error),
    }
}

fn compact_source_text(pages: &[String]) -> String {
    let mut output = String::new();
    for (page_index, page) in pages.iter().enumerate() {
        let text = page.trim();
        if text.is_empty() {
            continue;
        }
        if !output.is_empty() {
            output.push_str("\n\n");
        }
        output.push_str(&format!("--- Page {} ---\n", page_index + 1));
        output.push_str(text);
        if output.chars().count() >= LIQUID_SOURCE_TEXT_LIMIT {
            return output
                .chars()
                .take(LIQUID_SOURCE_TEXT_LIMIT)
                .collect::<String>();
        }
    }
    output
}

fn compact_sample(text: &str) -> String {
    const MAX_CHARS: usize = 220;
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= MAX_CHARS {
        return normalized;
    }
    let mut sample = normalized
        .chars()
        .take(MAX_CHARS.saturating_sub(3))
        .collect::<String>();
    sample.push_str("...");
    sample
}
