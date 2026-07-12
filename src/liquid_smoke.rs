use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::Value;

use crate::layout_roles;
use crate::liquid::{
    DeepLiquidSourceLine, DocumentProfile, LiquidBlock, LiquidBlockRole, LiquidBlockSourceLines,
    LiquidFootnoteLinkIntegrity, LiquidRequest, prepare_liquid_document,
    should_prefer_ocr_page_text, should_try_ocr_page_text,
};
use crate::liquid2::{
    LiquidMode2Request, LiquidMode2Timing, lm2_progressive_preview_request,
    load_fast_cached_liquid_mode2_document, prepare_liquid_mode2_document,
    prepare_liquid_mode2_document_with_timing, save_fast_cached_lm2_document,
};
use crate::liquidvision::{fill_document_features, liquidvision_enabled};
use crate::ocr::ocr_page_with_engine;
use crate::pdf_backend::PdfEngine;

/// LmV (vision) tier toggle. Lm (default) is completely unaffected when unset.
pub(crate) fn lmv_enabled() -> bool {
    liquidvision_enabled(crate::liquid2::lm2_native_catboost_default_asset_available())
}

/// LmV pre-pass: render each page, run the LiquidVision nano, and attach
/// per-line vision features to `lines` (mirrors the Python sidecar schema).
fn apply_liquidvision_features(
    engine: &PdfEngine,
    path: &Path,
    page_count: usize,
    lines: &mut [DeepLiquidSourceLine],
) {
    if let Err(error) = fill_document_features(engine, path, page_count, lines) {
        eprintln!("[LmV] document feature fill failed: {error}");
    }
}

#[derive(Debug, Serialize)]
struct LiquidSmokeReport {
    app_version: &'static str,
    timestamp_unix_secs: u64,
    document_count: usize,
    failures: usize,
    total_ms: f64,
    documents: Vec<LiquidSmokeDocument>,
}

#[derive(Debug, Serialize)]
struct LiquidSmokeDocument {
    path: String,
    extraction_version: String,
    extraction_stats: layout_roles::ExtractionStats,
    extraction_events: Vec<layout_roles::ExtractionEvent>,
    page_count: usize,
    extracted_pages: usize,
    extracted_chars: usize,
    ocr_pages: usize,
    ocr_chars: usize,
    ocr_errors: Vec<String>,
    footnote_divider_pages: Vec<usize>,
    layout_hint_count: usize,
    layout_hint_role_counts: BTreeMap<String, usize>,
    layout_hint_samples: BTreeMap<String, Vec<String>>,
    title: Option<String>,
    profile: Option<DocumentProfile>,
    block_count: usize,
    footnote_link_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    footnote_link_integrity: Option<LiquidFootnoteLinkIntegrity>,
    role_counts: BTreeMap<String, usize>,
    role_samples: BTreeMap<String, Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    blocks: Option<Vec<LiquidSmokeBlock>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    block_source_lines: Option<Vec<LiquidBlockSourceLines>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_lines: Option<Vec<DeepLiquidSourceLine>>,
    noise_lines_removed: usize,
    warnings: Vec<String>,
    samples: Vec<String>,
    elapsed_ms: f64,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct LiquidSmokeBlock {
    role: LiquidBlockRole,
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    source_line_ids: Vec<String>,
}

pub fn run_liquid_smoke(args: impl IntoIterator<Item = OsString>) -> Result<()> {
    let mut output_path = None;
    let mut ocr_empty_pages = false;
    let mut ocr_sparse_pages = false;
    let mut max_ocr_pages = 3usize;
    let mut include_blocks = false;
    let mut include_source_lines = false;
    let mut source_lines_only = false;
    let mut use_lm2 = false;
    let mut use_pymupdf_blocks = false;
    let mut use_pp_footnote_regions = false;
    let mut lm2_external_emissions_path: Option<PathBuf> = None;
    let mut paths = Vec::new();
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        if arg == OsStr::new("--smoke-liquid") {
            continue;
        }
        if arg == OsStr::new("--smoke-liquid2") {
            use_lm2 = true;
            continue;
        }
        if arg == OsStr::new("--liquid-smoke-use-pymupdf-blocks") {
            use_pymupdf_blocks = true;
            continue;
        }
        if arg == OsStr::new("--liquid-smoke-use-pp-footnote-regions") {
            use_pp_footnote_regions = true;
            continue;
        }
        if arg == OsStr::new("--lm2-external-emissions")
            || arg == OsStr::new("--external-emissions")
        {
            let value = args
                .next()
                .context("--lm2-external-emissions needs a path")?;
            lm2_external_emissions_path = Some(PathBuf::from(value));
            continue;
        }
        if arg == OsStr::new("--liquid-smoke-output") {
            let value = args
                .next()
                .context("--liquid-smoke-output needs a destination path")?;
            output_path = Some(PathBuf::from(value));
            continue;
        }
        if arg == OsStr::new("--liquid-smoke-ocr-empty-pages") {
            ocr_empty_pages = true;
            continue;
        }
        if arg == OsStr::new("--liquid-smoke-ocr-sparse-pages") {
            ocr_sparse_pages = true;
            continue;
        }
        if arg == OsStr::new("--liquid-smoke-include-blocks") {
            include_blocks = true;
            continue;
        }
        if arg == OsStr::new("--liquid-smoke-include-source-lines") {
            include_source_lines = true;
            continue;
        }
        if arg == OsStr::new("--liquid-smoke-source-lines-only") {
            include_source_lines = true;
            source_lines_only = true;
            continue;
        }
        if arg == OsStr::new("--liquid-smoke-max-ocr-pages") {
            let value = args
                .next()
                .context("--liquid-smoke-max-ocr-pages needs a positive integer")?;
            max_ocr_pages = value
                .to_string_lossy()
                .parse::<usize>()
                .context("invalid --liquid-smoke-max-ocr-pages")?;
            continue;
        }
        if arg.to_string_lossy().starts_with("--") {
            bail!("unknown liquid smoke argument: {}", arg.to_string_lossy());
        }
        paths.push(PathBuf::from(arg));
    }

    if paths.is_empty() {
        bail!("pass at least one PDF path after --smoke-liquid");
    }

    let started = Instant::now();
    let engine = PdfEngine::new().context("failed to initialize PDF engine")?;
    let documents = paths
        .iter()
        .map(|path| {
            smoke_document(
                &engine,
                path,
                ocr_empty_pages,
                ocr_sparse_pages,
                max_ocr_pages,
                include_blocks,
                include_source_lines,
                source_lines_only,
                use_lm2,
                use_pymupdf_blocks,
                use_pp_footnote_regions,
                lm2_external_emissions_path.as_deref(),
            )
        })
        .collect::<Vec<_>>();
    let failures = documents
        .iter()
        .filter(|document| document.error.is_some())
        .count();

    let report = LiquidSmokeReport {
        app_version: env!("CARGO_PKG_VERSION"),
        timestamp_unix_secs: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        document_count: documents.len(),
        failures,
        total_ms: started.elapsed().as_secs_f64() * 1000.0,
        documents,
    };

    let json = serde_json::to_string_pretty(&report)?;
    if let Some(output_path) = output_path {
        std::fs::write(&output_path, json)
            .with_context(|| format!("failed to write {}", output_path.display()))?;
    } else {
        println!("{json}");
    }

    println!(
        "Liquid-smoked {} document(s), {} failure(s), total {:.1} ms",
        report.document_count, report.failures, report.total_ms
    );
    Ok(())
}

#[derive(Debug, Serialize)]
struct Lm2MarkdownExportReport {
    app_version: &'static str,
    timestamp_unix_secs: u64,
    document_count: usize,
    failures: usize,
    output_dir: String,
    documents: Vec<Lm2MarkdownExportDocument>,
}

#[derive(Debug, Serialize)]
struct Lm2MarkdownExportDocument {
    input_path: String,
    markdown_path: Option<String>,
    sidecar_path: Option<String>,
    block_count: usize,
    source_line_count: usize,
    warnings: Vec<String>,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct Lm2TimingBaselineReport {
    app_version: &'static str,
    timestamp_unix_secs: u64,
    document_count: usize,
    documents: Vec<Lm2TimingDocument>,
}

#[derive(Debug, Serialize)]
struct Lm2TimingDocument {
    input_path: String,
    page_count: usize,
    source_line_count: usize,
    block_count: usize,
    extracted_chars: usize,
    load_document_ms: f64,
    first_page_render_ms: f64,
    text_extraction_ms: f64,
    line_geometry_ms: f64,
    time_to_preview_ms: f64,
    preview_page_count: usize,
    preview_lm2: Option<LiquidMode2Timing>,
    liquify_total_ms: f64,
    fast_reopen_ms: f64,
    fast_reopen_hit: bool,
    lm2: LiquidMode2Timing,
    markdown_render_ms: f64,
    warnings: Vec<String>,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct Lm2MarkdownSidecar<'a> {
    input_path: &'a str,
    title: Option<&'a str>,
    markdown_path: &'a str,
    source_signature: Option<&'a str>,
    warnings: &'a [String],
    blocks: Vec<Lm2MarkdownBlockAnchor>,
    source_lines: Option<&'a [DeepLiquidSourceLine]>,
}

#[derive(Debug, Serialize)]
struct Lm2MarkdownBlockAnchor {
    block_index: usize,
    markdown_block_index: Option<usize>,
    markdown_anchor: Option<String>,
    role: LiquidBlockRole,
    text: String,
    source_line_ids: Vec<String>,
    source_lines: Vec<crate::liquid::LiquidSourceLineRef>,
}

pub fn run_lm2_assemble_markdown(args: impl IntoIterator<Item = OsString>) -> Result<()> {
    let mut output_dir = None;
    let mut input_paths = Vec::new();
    let mut ocr_empty_pages = false;
    let mut ocr_sparse_pages = false;
    let mut max_ocr_pages = 3usize;
    let mut use_pymupdf_blocks = false;
    let mut use_pp_footnote_regions = false;
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        if arg == OsStr::new("--lm2-assemble-markdown") {
            continue;
        }
        if arg == OsStr::new("--input") {
            let value = args.next().context("--input needs a PDF or JSON path")?;
            input_paths.push(PathBuf::from(value));
            continue;
        }
        if arg == OsStr::new("--output") {
            let value = args.next().context("--output needs a directory path")?;
            output_dir = Some(PathBuf::from(value));
            continue;
        }
        if arg == OsStr::new("--liquid-smoke-ocr-empty-pages") {
            ocr_empty_pages = true;
            continue;
        }
        if arg == OsStr::new("--liquid-smoke-ocr-sparse-pages") {
            ocr_sparse_pages = true;
            continue;
        }
        if arg == OsStr::new("--liquid-smoke-max-ocr-pages") {
            let value = args
                .next()
                .context("--liquid-smoke-max-ocr-pages needs a positive integer")?;
            max_ocr_pages = value
                .to_string_lossy()
                .parse::<usize>()
                .context("invalid --liquid-smoke-max-ocr-pages")?;
            continue;
        }
        if arg == OsStr::new("--liquid-smoke-use-pymupdf-blocks") {
            use_pymupdf_blocks = true;
            continue;
        }
        if arg == OsStr::new("--liquid-smoke-use-pp-footnote-regions") {
            use_pp_footnote_regions = true;
            continue;
        }
        if arg.to_string_lossy().starts_with("--") {
            bail!(
                "unknown LiquidMode2 Markdown export argument: {}",
                arg.to_string_lossy()
            );
        }
        input_paths.push(PathBuf::from(arg));
    }

    let output_dir = output_dir.context("--output is required")?;
    if input_paths.is_empty() {
        bail!("pass at least one --input path or positional PDF path");
    }
    std::fs::create_dir_all(&output_dir)
        .with_context(|| format!("failed to create {}", output_dir.display()))?;

    let engine = PdfEngine::new().context("failed to initialize PDF engine")?;
    let mut documents = Vec::new();
    for (index, input_path) in input_paths.iter().enumerate() {
        let result = if input_path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
        {
            export_assembled_smoke_json_to_markdown(input_path, &output_dir, index)
        } else {
            let smoke = smoke_document(
                &engine,
                input_path,
                ocr_empty_pages,
                ocr_sparse_pages,
                max_ocr_pages,
                true,
                true,
                false,
                true,
                use_pymupdf_blocks,
                use_pp_footnote_regions,
                None,
            );
            export_smoke_document_to_markdown(&smoke, &output_dir, index)
        };
        documents.push(match result {
            Ok(document) => document,
            Err(error) => Lm2MarkdownExportDocument {
                input_path: input_path.display().to_string(),
                markdown_path: None,
                sidecar_path: None,
                block_count: 0,
                source_line_count: 0,
                warnings: Vec::new(),
                error: Some(format!("{error:#}")),
            },
        });
    }

    let failures = documents
        .iter()
        .filter(|document| document.error.is_some())
        .count();
    let report = Lm2MarkdownExportReport {
        app_version: env!("CARGO_PKG_VERSION"),
        timestamp_unix_secs: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        document_count: documents.len(),
        failures,
        output_dir: output_dir.display().to_string(),
        documents,
    };
    let manifest_path = output_dir.join("manifest.json");
    std::fs::write(&manifest_path, serde_json::to_string_pretty(&report)?)
        .with_context(|| format!("failed to write {}", manifest_path.display()))?;
    println!(
        "Exported LiquidMode2 Markdown for {} document(s), {} failure(s): {}",
        report.document_count,
        report.failures,
        output_dir.display()
    );
    if failures > 0 {
        bail!(
            "{failures} Markdown export(s) failed; see {}",
            manifest_path.display()
        );
    }
    Ok(())
}

pub fn run_lm2_timing_baseline(args: impl IntoIterator<Item = OsString>) -> Result<()> {
    let mut output_path = None;
    let mut input_paths = Vec::new();
    let mut use_pymupdf_blocks = false;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if arg == OsStr::new("--lm2-timing-baseline") {
            continue;
        }
        if arg == OsStr::new("--input") {
            input_paths.push(PathBuf::from(
                args.next().context("--input needs a PDF path")?,
            ));
            continue;
        }
        if arg == OsStr::new("--output") {
            output_path = Some(PathBuf::from(
                args.next().context("--output needs a JSON path")?,
            ));
            continue;
        }
        if arg == OsStr::new("--liquid-smoke-use-pymupdf-blocks") {
            use_pymupdf_blocks = true;
            continue;
        }
        if arg.to_string_lossy().starts_with("--") {
            bail!(
                "unknown LiquidMode2 timing argument: {}",
                arg.to_string_lossy()
            );
        }
        input_paths.push(PathBuf::from(arg));
    }
    if input_paths.is_empty() {
        bail!("pass at least one --input path or positional PDF path");
    }

    let engine = PdfEngine::new().context("failed to initialize PDF engine")?;
    let documents = input_paths
        .iter()
        .map(|path| measure_lm2_timing_document(&engine, path, use_pymupdf_blocks))
        .collect::<Vec<_>>();
    let report = Lm2TimingBaselineReport {
        app_version: env!("CARGO_PKG_VERSION"),
        timestamp_unix_secs: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0),
        document_count: documents.len(),
        documents,
    };
    let json = serde_json::to_string_pretty(&report)?;
    if let Some(path) = output_path {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        std::fs::write(&path, format!("{json}\n"))
            .with_context(|| format!("failed to write {}", path.display()))?;
        println!("Wrote LiquidMode2 timing baseline: {}", path.display());
    } else {
        println!("{json}");
    }
    Ok(())
}

fn measure_lm2_timing_document(
    engine: &PdfEngine,
    path: &Path,
    use_pymupdf_blocks: bool,
) -> Lm2TimingDocument {
    let load_started = Instant::now();
    let document = match engine.load_document(path) {
        Ok(document) => document,
        Err(error) => return failed_timing_document(path, error.to_string()),
    };
    let load_document_ms = load_started.elapsed().as_secs_f64() * 1000.0;

    let render_started = Instant::now();
    if document.page_count > 0 {
        let _ = engine.render_page(path, 0, 1.0);
    }
    let first_page_render_ms = if document.page_count > 0 {
        render_started.elapsed().as_secs_f64() * 1000.0
    } else {
        0.0
    };

    let extraction_started = Instant::now();
    let mut text_chars = Vec::with_capacity(document.page_count);
    let mut pages = Vec::with_capacity(document.page_count);
    for page_index in 0..document.page_count {
        text_chars.push(engine.load_page_text_chars(path, page_index).ok());
        pages.push(
            engine
                .load_page_text(path, page_index)
                .unwrap_or_else(|error| format!("[Liquid timing extraction failed: {error}]")),
        );
    }
    let layout_pages = layout_roles::source_pages_from_text_chars(&document.pages, &text_chars);
    for (page_index, layout_text) in layout_pages.into_iter().enumerate() {
        if let Some(layout_text) = layout_text
            && !layout_text.trim().is_empty()
        {
            pages[page_index] = layout_text;
        }
    }
    let text_extraction_ms = extraction_started.elapsed().as_secs_f64() * 1000.0;
    let extracted_chars = pages.iter().map(|page| page.chars().count()).sum();

    let geometry_started = Instant::now();
    let deep_source_lines = layout_roles::deep_source_lines_for_pages(&document.pages, &text_chars);
    let line_geometry_ms = geometry_started.elapsed().as_secs_f64() * 1000.0;
    let raw_source_line_count = deep_source_lines.len();

    let request = LiquidMode2Request {
        document_epoch: 0,
        path: path.to_path_buf(),
        title: document.title.clone(),
        pages,
        deep_source_lines,
        use_pymupdf_blocks,
        use_pp_footnote_regions: false,
        external_emissions_path: None,
    };
    let (preview_page_count, preview_lm2) = lm2_progressive_preview_request(&request)
        .and_then(|(preview_request, page_count)| {
            prepare_liquid_mode2_document_with_timing(preview_request)
                .ok()
                .map(|(_, timing)| (page_count, timing))
        })
        .map_or((document.page_count, None), |(page_count, timing)| {
            (page_count, Some(timing))
        });
    let time_to_preview_ms = load_document_ms
        + text_extraction_ms
        + line_geometry_ms
        + preview_lm2.as_ref().map_or(0.0, |timing| timing.total_ms);

    let (liquid, lm2) = match prepare_liquid_mode2_document_with_timing(request) {
        Ok(result) => result,
        Err(error) => {
            let mut failed = failed_timing_document(path, error);
            failed.page_count = document.page_count;
            failed.source_line_count = raw_source_line_count;
            failed.extracted_chars = extracted_chars;
            failed.load_document_ms = load_document_ms;
            failed.first_page_render_ms = first_page_render_ms;
            failed.text_extraction_ms = text_extraction_ms;
            failed.line_geometry_ms = line_geometry_ms;
            return failed;
        }
    };
    let markdown_started = Instant::now();
    let _ = render_markdown_blocks(
        Some(liquid.title.as_str()),
        liquid
            .blocks
            .iter()
            .map(|block| (block.role.prompt_name(), block.text.as_str())),
    );
    let markdown_render_ms = markdown_started.elapsed().as_secs_f64() * 1000.0;
    let liquify_total_ms =
        text_extraction_ms + line_geometry_ms + lm2.total_ms + markdown_render_ms;
    let _ = save_fast_cached_lm2_document(path, use_pymupdf_blocks, false, &liquid);
    let fast_reopen_started = Instant::now();
    let fast_reopen_hit =
        load_fast_cached_liquid_mode2_document(path, use_pymupdf_blocks, false).is_some();
    let fast_reopen_ms = fast_reopen_started.elapsed().as_secs_f64() * 1000.0;

    Lm2TimingDocument {
        input_path: path.display().to_string(),
        page_count: document.page_count,
        source_line_count: raw_source_line_count,
        block_count: liquid.blocks.len(),
        extracted_chars,
        load_document_ms,
        first_page_render_ms,
        text_extraction_ms,
        line_geometry_ms,
        time_to_preview_ms,
        preview_page_count,
        preview_lm2,
        liquify_total_ms,
        fast_reopen_ms,
        fast_reopen_hit,
        lm2,
        markdown_render_ms,
        warnings: liquid.warnings,
        error: None,
    }
}

fn failed_timing_document(path: &Path, error: String) -> Lm2TimingDocument {
    Lm2TimingDocument {
        input_path: path.display().to_string(),
        page_count: 0,
        source_line_count: 0,
        block_count: 0,
        extracted_chars: 0,
        load_document_ms: 0.0,
        first_page_render_ms: 0.0,
        text_extraction_ms: 0.0,
        line_geometry_ms: 0.0,
        time_to_preview_ms: 0.0,
        preview_page_count: 0,
        preview_lm2: None,
        liquify_total_ms: 0.0,
        fast_reopen_ms: 0.0,
        fast_reopen_hit: false,
        lm2: LiquidMode2Timing::default(),
        markdown_render_ms: 0.0,
        warnings: Vec::new(),
        error: Some(error),
    }
}

fn export_smoke_document_to_markdown(
    document: &LiquidSmokeDocument,
    output_dir: &Path,
    ordinal: usize,
) -> Result<Lm2MarkdownExportDocument> {
    if let Some(error) = &document.error {
        bail!("{}: {error}", document.path);
    }
    let blocks = document
        .blocks
        .as_ref()
        .context("assembled blocks missing; run with block export enabled")?;
    let block_source_lines = document
        .block_source_lines
        .as_ref()
        .context("block source-line sidecar missing; run with source-line export enabled")?;
    let stem = export_stem(&document.path, ordinal);
    let markdown_path = output_dir.join(format!("{stem}.md"));
    let sidecar_path = output_dir.join(format!("{stem}.sidecar.json"));
    let (markdown, markdown_indices) =
        render_markdown_blocks(document.title.as_deref(), blocks.iter().map(block_parts));
    std::fs::write(&markdown_path, markdown)
        .with_context(|| format!("failed to write {}", markdown_path.display()))?;

    let source_lines_by_block = block_source_lines
        .iter()
        .map(|entry| (entry.block_index, entry.lines.clone()))
        .collect::<BTreeMap<_, _>>();
    let anchors = blocks
        .iter()
        .enumerate()
        .map(|(block_index, block)| {
            let source_lines = source_lines_by_block
                .get(&block_index)
                .cloned()
                .unwrap_or_default();
            let source_line_ids = source_lines
                .iter()
                .filter_map(|line| line.id.clone())
                .collect::<Vec<_>>();
            let markdown_block_index = markdown_indices.get(&block_index).copied();
            Lm2MarkdownBlockAnchor {
                block_index,
                markdown_block_index,
                markdown_anchor: markdown_block_index.map(|index| format!("block-{index:04}")),
                role: block.role,
                text: clean_markdown_text(&block.text),
                source_line_ids,
                source_lines,
            }
        })
        .collect::<Vec<_>>();
    let markdown_path_string = markdown_path.display().to_string();
    let sidecar = Lm2MarkdownSidecar {
        input_path: &document.path,
        title: document.title.as_deref(),
        markdown_path: &markdown_path_string,
        source_signature: None,
        warnings: &document.warnings,
        blocks: anchors,
        source_lines: document.source_lines.as_deref(),
    };
    std::fs::write(&sidecar_path, serde_json::to_string_pretty(&sidecar)?)
        .with_context(|| format!("failed to write {}", sidecar_path.display()))?;
    Ok(Lm2MarkdownExportDocument {
        input_path: document.path.clone(),
        markdown_path: Some(markdown_path.display().to_string()),
        sidecar_path: Some(sidecar_path.display().to_string()),
        block_count: blocks.len(),
        source_line_count: document.source_lines.as_ref().map_or(0, Vec::len),
        warnings: document.warnings.clone(),
        error: None,
    })
}

fn export_assembled_smoke_json_to_markdown(
    input_path: &Path,
    output_dir: &Path,
    ordinal: usize,
) -> Result<Lm2MarkdownExportDocument> {
    let bytes = std::fs::read(input_path)
        .with_context(|| format!("failed to read {}", input_path.display()))?;
    let value = serde_json::from_slice::<Value>(&bytes)
        .with_context(|| format!("failed to parse {}", input_path.display()))?;
    let document = value
        .get("documents")
        .and_then(Value::as_array)
        .and_then(|documents| documents.first())
        .unwrap_or(&value);
    let blocks = document
        .get("blocks")
        .and_then(Value::as_array)
        .context("JSON input must contain assembled blocks")?;
    let title = document.get("title").and_then(Value::as_str);
    let stem = export_stem(&input_path.display().to_string(), ordinal);
    let markdown_path = output_dir.join(format!("{stem}.md"));
    let sidecar_path = output_dir.join(format!("{stem}.sidecar.json"));
    let (markdown, _) = render_markdown_blocks(
        title,
        blocks.iter().map(|block| {
            let role = block
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or("paragraph");
            let text = block.get("text").and_then(Value::as_str).unwrap_or("");
            (role, text)
        }),
    );
    std::fs::write(&markdown_path, markdown)
        .with_context(|| format!("failed to write {}", markdown_path.display()))?;
    let sidecar = serde_json::json!({
        "input_path": input_path.display().to_string(),
        "title": title,
        "markdown_path": markdown_path.display().to_string(),
        "source": "assembled_smoke_json",
        "document": document,
    });
    std::fs::write(&sidecar_path, serde_json::to_string_pretty(&sidecar)?)
        .with_context(|| format!("failed to write {}", sidecar_path.display()))?;
    Ok(Lm2MarkdownExportDocument {
        input_path: input_path.display().to_string(),
        markdown_path: Some(markdown_path.display().to_string()),
        sidecar_path: Some(sidecar_path.display().to_string()),
        block_count: blocks.len(),
        source_line_count: document
            .get("source_lines")
            .and_then(Value::as_array)
            .map_or(0, Vec::len),
        warnings: Vec::new(),
        error: None,
    })
}

fn block_parts(block: &LiquidSmokeBlock) -> (&'static str, &str) {
    (block.role.prompt_name(), block.text.as_str())
}

fn render_markdown_blocks<I, R, T>(
    title: Option<&str>,
    blocks: I,
) -> (String, BTreeMap<usize, usize>)
where
    I: IntoIterator<Item = (R, T)>,
    R: AsRef<str>,
    T: AsRef<str>,
{
    let mut out = String::new();
    let mut markdown_indices = BTreeMap::new();
    let mut markdown_block_index = 0usize;
    let mut saw_title = false;
    let mut deferred_notes = Vec::new();
    let mut pending_paragraph_text = String::new();
    let mut pending_paragraph_indices: Vec<usize> = Vec::new();
    let fallback_title = title
        .map(clean_markdown_text)
        .filter(|title| !title.is_empty());

    for (block_index, (role, text)) in blocks.into_iter().enumerate() {
        let role = role.as_ref();
        let text = clean_markdown_text(text.as_ref());
        if text.is_empty() || markdown_suppressed_role(role) {
            continue;
        }
        if markdown_note_role(role) {
            let rendered = render_markdown_block(role, &text, saw_title);
            if !rendered.is_empty() {
                deferred_notes.push((block_index, rendered));
            }
            continue;
        }
        if markdown_paragraph_role(role) {
            if pending_paragraph_text.is_empty() {
                pending_paragraph_text = text;
                pending_paragraph_indices.push(block_index);
                continue;
            }
            if markdown_should_join_paragraphs(&pending_paragraph_text, &text) {
                append_markdown_paragraph_text(&mut pending_paragraph_text, &text);
                pending_paragraph_indices.push(block_index);
                continue;
            }
            flush_markdown_paragraph(
                &mut out,
                &mut markdown_indices,
                &mut markdown_block_index,
                &mut pending_paragraph_text,
                &mut pending_paragraph_indices,
            );
            pending_paragraph_text = text;
            pending_paragraph_indices.push(block_index);
            continue;
        }
        flush_markdown_paragraph(
            &mut out,
            &mut markdown_indices,
            &mut markdown_block_index,
            &mut pending_paragraph_text,
            &mut pending_paragraph_indices,
        );
        let rendered = render_markdown_block(role, &text, saw_title);
        if rendered.is_empty() {
            continue;
        }
        markdown_indices.insert(block_index, markdown_block_index);
        markdown_block_index += 1;
        if role == "title" {
            saw_title = true;
        }
        out.push_str(&rendered);
        if !out.ends_with("\n\n") {
            out.push_str("\n\n");
        }
    }
    flush_markdown_paragraph(
        &mut out,
        &mut markdown_indices,
        &mut markdown_block_index,
        &mut pending_paragraph_text,
        &mut pending_paragraph_indices,
    );
    if !deferred_notes.is_empty() {
        if !out.trim().is_empty() {
            if !out.ends_with("\n\n") {
                out.push_str("\n\n");
            }
            out.push_str("## Notes\n\n");
        }
        for (block_index, rendered) in deferred_notes {
            markdown_indices.insert(block_index, markdown_block_index);
            markdown_block_index += 1;
            out.push_str(&rendered);
            if !out.ends_with("\n\n") {
                out.push_str("\n\n");
            }
        }
    }
    if out.trim().is_empty() {
        if let Some(title) = fallback_title {
            out.push_str("# ");
            out.push_str(&title);
            out.push('\n');
        } else {
            out.push_str("(No readable Review Mode content.)\n");
        }
    }
    (out.trim_end().to_owned() + "\n", markdown_indices)
}

fn render_markdown_block(role: &str, text: &str, saw_title: bool) -> String {
    match role {
        "title" if saw_title => format!("## {text}\n\n"),
        "title" => format!("# {text}\n\n"),
        "heading" => format!("## {text}\n\n"),
        "subheading" => format!("### {text}\n\n"),
        "abstract" => format!("## Abstract\n\n{text}\n\n"),
        "author_info" | "metadata" => format!("_{text}_\n\n"),
        "lead" | "explainer" | "takeaway" | "holding" | "issue" | "definition" | "key_clause" => {
            format!("**{}:** {text}\n\n", role.replace('_', " "))
        }
        "list_item" | "clause" => format!("- {text}\n"),
        "quote" => format!("> {text}\n\n"),
        "caption" => format!("_{text}_\n\n"),
        "table" => format!("```text\n{text}\n```\n\n"),
        "marginalia" | "footnote" => format!("> [note] {text}\n\n"),
        "section_break" => "---\n\n".to_owned(),
        _ => format!("{text}\n\n"),
    }
}

fn markdown_suppressed_role(role: &str) -> bool {
    matches!(role, "noise" | "header" | "footer" | "contents")
}

fn markdown_note_role(role: &str) -> bool {
    matches!(role, "marginalia" | "footnote")
}

fn markdown_paragraph_role(role: &str) -> bool {
    matches!(role, "paragraph")
}

fn flush_markdown_paragraph(
    out: &mut String,
    markdown_indices: &mut BTreeMap<usize, usize>,
    markdown_block_index: &mut usize,
    pending_text: &mut String,
    pending_indices: &mut Vec<usize>,
) {
    if pending_text.trim().is_empty() {
        pending_text.clear();
        pending_indices.clear();
        return;
    }
    let rendered = render_markdown_block("paragraph", pending_text, false);
    if rendered.is_empty() {
        pending_text.clear();
        pending_indices.clear();
        return;
    }
    for block_index in pending_indices.iter().copied() {
        markdown_indices.insert(block_index, *markdown_block_index);
    }
    *markdown_block_index += 1;
    out.push_str(&rendered);
    if !out.ends_with("\n\n") {
        out.push_str("\n\n");
    }
    pending_text.clear();
    pending_indices.clear();
}

fn markdown_should_join_paragraphs(before: &str, after: &str) -> bool {
    let before = before.trim_end();
    let after = after.trim_start();
    if before.is_empty() || after.is_empty() {
        return false;
    }
    if markdown_ends_like_complete_sentence(before) {
        return false;
    }
    markdown_starts_like_continuation(after)
}

fn markdown_ends_like_complete_sentence(text: &str) -> bool {
    let trimmed = text.trim_end();
    trimmed
        .chars()
        .rev()
        .find(|ch| !matches!(ch, '"' | '\'' | ')' | ']' | '”' | '’'))
        .is_some_and(|ch| matches!(ch, '.' | '!' | '?' | ':'))
}

fn markdown_starts_like_continuation(text: &str) -> bool {
    let first = text
        .trim_start()
        .chars()
        .find(|ch| !matches!(ch, '"' | '\'' | '“' | '‘' | '(' | '['));
    first.is_some_and(|ch| ch.is_ascii_lowercase() || matches!(ch, ',' | ';' | ')' | ']'))
}

fn append_markdown_paragraph_text(text: &mut String, line: &str) {
    let line = line.trim();
    if line.is_empty() {
        return;
    }
    if text.ends_with('-') {
        let before = text
            .chars()
            .rev()
            .nth(1)
            .is_some_and(|ch| ch.is_ascii_alphabetic());
        let after = line
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_lowercase());
        if before && after {
            text.pop();
            text.push_str(line);
            return;
        }
    }
    if !text.is_empty() && !text.ends_with(char::is_whitespace) {
        text.push(' ');
    }
    text.push_str(line);
}

fn collapse_markdown_space(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn clean_markdown_text(text: &str) -> String {
    let mut text = text.to_owned();
    strip_callout_sentinels(&mut text);
    collapse_markdown_space(&text)
}

fn export_stem(path: &str, ordinal: usize) -> String {
    let raw_stem = Path::new(path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("document");
    let mut slug = String::new();
    for ch in raw_stem.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
        } else if (ch == '-' || ch == '_' || ch.is_whitespace()) && !slug.ends_with('-') {
            slug.push('-');
        }
        if slug.len() >= 96 {
            break;
        }
    }
    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        format!("{:04}-document", ordinal + 1)
    } else {
        format!("{:04}-{slug}", ordinal + 1)
    }
}

fn smoke_document(
    engine: &PdfEngine,
    path: &Path,
    ocr_empty_pages: bool,
    ocr_sparse_pages: bool,
    max_ocr_pages: usize,
    include_blocks: bool,
    include_source_lines: bool,
    source_lines_only: bool,
    use_lm2: bool,
    use_pymupdf_blocks: bool,
    use_pp_footnote_regions: bool,
    lm2_external_emissions_path: Option<&Path>,
) -> LiquidSmokeDocument {
    let started = Instant::now();
    let document = match engine.load_document(path) {
        Ok(document) => document,
        Err(error) => {
            return LiquidSmokeDocument {
                path: path.display().to_string(),
                extraction_version: layout_roles::extraction_version().to_owned(),
                extraction_stats: Default::default(),
                extraction_events: Vec::new(),
                page_count: 0,
                extracted_pages: 0,
                extracted_chars: 0,
                ocr_pages: 0,
                ocr_chars: 0,
                ocr_errors: Vec::new(),
                footnote_divider_pages: Vec::new(),
                layout_hint_count: 0,
                layout_hint_role_counts: BTreeMap::new(),
                layout_hint_samples: BTreeMap::new(),
                title: None,
                profile: None,
                block_count: 0,
                footnote_link_count: 0,
                footnote_link_integrity: None,
                role_counts: BTreeMap::new(),
                role_samples: BTreeMap::new(),
                blocks: None,
                block_source_lines: None,
                source_lines: None,
                noise_lines_removed: 0,
                warnings: Vec::new(),
                samples: Vec::new(),
                elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
                error: Some(error.to_string()),
            };
        }
    };

    let mut pages = Vec::with_capacity(document.page_count);
    let mut text_chars = Vec::with_capacity(document.page_count);
    let mut page_uses_ocr = Vec::with_capacity(document.page_count);
    let mut ocr_pages = 0usize;
    let mut ocr_chars = 0usize;
    let mut ocr_attempts = 0usize;
    let mut ocr_errors = Vec::new();
    for page_index in 0..document.page_count {
        text_chars.push(engine.load_page_text_chars(path, page_index).ok());
        let mut text = match engine.load_page_text(path, page_index) {
            Ok(text) => text,
            Err(error) => {
                format!("[Liquid smoke text extraction failed: {error}]")
            }
        };

        let mut uses_ocr = false;
        if should_try_ocr_page_text(&text, ocr_empty_pages, ocr_sparse_pages)
            && ocr_attempts < max_ocr_pages
        {
            ocr_attempts += 1;
            match ocr_page_with_engine(engine, path, page_index) {
                Ok(ocr_text) => {
                    if should_prefer_ocr_page_text(&text, &ocr_text) {
                        ocr_pages += 1;
                        ocr_chars += ocr_text.chars().count();
                        text = ocr_text;
                        uses_ocr = true;
                    }
                }
                Err(error) => {
                    ocr_errors.push(format!("page {}: {error}", page_index + 1));
                }
            }
        }

        page_uses_ocr.push(uses_ocr);
        pages.push(text);
    }

    let layout_pages = layout_roles::source_pages_from_text_chars(&document.pages, &text_chars);
    for (page_index, layout_text) in layout_pages.into_iter().enumerate() {
        if page_uses_ocr.get(page_index).copied().unwrap_or(false) {
            continue;
        }
        let Some(layout_text) = layout_text else {
            continue;
        };
        if !layout_text.trim().is_empty() {
            pages[page_index] = layout_text;
        }
    }
    let extracted_pages = pages.iter().filter(|page| !page.trim().is_empty()).count();
    let extracted_chars = pages.iter().map(|page| page.chars().count()).sum();

    let (mut deep_source_lines, extraction_report) =
        layout_roles::deep_source_lines_for_pages_with_extraction_report(
            &document.pages,
            &text_chars,
        );
    if lmv_enabled() && (source_lines_only || !use_lm2) {
        apply_liquidvision_features(engine, path, document.page_count, &mut deep_source_lines);
    }
    let source_lines = include_source_lines.then(|| smoke_deep_source_lines(&deep_source_lines));
    if source_lines_only {
        let source_line_count = source_lines.as_ref().map_or(0, Vec::len);
        return LiquidSmokeDocument {
            path: path.display().to_string(),
            extraction_version: extraction_report.extraction_version.clone(),
            extraction_stats: extraction_report.stats,
            extraction_events: extraction_report.events,
            page_count: document.page_count,
            extracted_pages,
            extracted_chars,
            ocr_pages,
            ocr_chars,
            ocr_errors,
            footnote_divider_pages: document
                .pages
                .iter()
                .enumerate()
                .filter_map(|(index, page)| page.footnote_divider_y_from_top.map(|_| index + 1))
                .collect(),
            layout_hint_count: source_line_count,
            layout_hint_role_counts: BTreeMap::new(),
            layout_hint_samples: BTreeMap::new(),
            title: Some(document.title),
            profile: None,
            block_count: 0,
            footnote_link_count: 0,
            footnote_link_integrity: None,
            role_counts: BTreeMap::new(),
            role_samples: BTreeMap::new(),
            blocks: None,
            block_source_lines: None,
            source_lines,
            noise_lines_removed: 0,
            warnings: Vec::new(),
            samples: Vec::new(),
            elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
            error: None,
        };
    }
    let layout_hints = layout_roles::layout_hints_for_pages(&document.pages, &text_chars);
    let layout_hint_count = layout_hints.len();
    let mut layout_hint_role_counts = BTreeMap::new();
    let mut layout_hint_samples: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for hint in &layout_hints {
        let role = hint.role.prompt_name().to_owned();
        *layout_hint_role_counts.entry(role.clone()).or_insert(0) += 1;
        let samples = layout_hint_samples.entry(role).or_default();
        if samples.len() < 10 {
            samples.push(compact_sample(&hint.text));
        }
    }
    let footnote_divider_pages = document
        .pages
        .iter()
        .enumerate()
        .filter_map(|(index, page)| page.footnote_divider_y_from_top.map(|_| index + 1))
        .collect::<Vec<_>>();
    let result = if use_lm2 {
        prepare_liquid_mode2_document(LiquidMode2Request {
            document_epoch: 0,
            path: path.to_path_buf(),
            title: document.title.clone(),
            pages,
            deep_source_lines,
            use_pymupdf_blocks,
            use_pp_footnote_regions,
            external_emissions_path: lm2_external_emissions_path.map(Path::to_path_buf),
        })
    } else {
        prepare_liquid_document(LiquidRequest {
            document_epoch: 0,
            path: path.to_path_buf(),
            title: document.title.clone(),
            pages,
            layout_hints,
            source_line_hints: Vec::new(),
            deep_source_lines: deep_source_lines.clone(),
            deep_liquid: None,
            groq_api_key: None,
            openrouter_api_key: None,
        })
    };

    match result {
        Ok(liquid) => {
            let mut role_counts = BTreeMap::new();
            let mut role_samples: BTreeMap<String, Vec<String>> = BTreeMap::new();
            for block in &liquid.blocks {
                let role = block.role.prompt_name().to_owned();
                *role_counts.entry(role.clone()).or_insert(0) += 1;
                let samples = role_samples.entry(role).or_default();
                if samples.len() < 5 {
                    samples.push(compact_sample(&block.text));
                }
            }
            let samples = liquid
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
                            | LiquidBlockRole::Noise
                            | LiquidBlockRole::Table
                            | LiquidBlockRole::Syllabus
                    )
                })
                .take(6)
                .map(|block| compact_sample(&block.text))
                .collect::<Vec<_>>();
            let profile = liquid.profile.clone();
            let blocks = include_blocks.then(|| {
                smoke_blocks_with_source_line_ids(&liquid.blocks, &liquid.block_source_lines)
            });
            let block_source_lines =
                include_source_lines.then(|| smoke_block_source_lines(&liquid.block_source_lines));

            LiquidSmokeDocument {
                path: path.display().to_string(),
                extraction_version: extraction_report.extraction_version.clone(),
                extraction_stats: extraction_report.stats,
                extraction_events: extraction_report.events,
                page_count: document.page_count,
                extracted_pages,
                extracted_chars,
                ocr_pages,
                ocr_chars,
                ocr_errors,
                footnote_divider_pages,
                layout_hint_count,
                layout_hint_role_counts,
                layout_hint_samples,
                title: Some(liquid.title),
                profile,
                block_count: liquid.blocks.len(),
                footnote_link_count: liquid.footnote_links.len(),
                footnote_link_integrity: liquid.footnote_link_integrity,
                role_counts,
                role_samples,
                blocks,
                block_source_lines,
                source_lines,
                noise_lines_removed: liquid.noise_lines_removed,
                warnings: liquid.warnings,
                samples,
                elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
                error: None,
            }
        }
        Err(error) => LiquidSmokeDocument {
            path: path.display().to_string(),
            extraction_version: extraction_report.extraction_version.clone(),
            extraction_stats: extraction_report.stats,
            extraction_events: extraction_report.events,
            page_count: document.page_count,
            extracted_pages,
            extracted_chars,
            ocr_pages,
            ocr_chars,
            ocr_errors,
            footnote_divider_pages,
            layout_hint_count,
            layout_hint_role_counts,
            layout_hint_samples,
            title: Some(document.title),
            profile: None,
            block_count: 0,
            footnote_link_count: 0,
            footnote_link_integrity: None,
            role_counts: BTreeMap::new(),
            role_samples: BTreeMap::new(),
            blocks: None,
            block_source_lines: None,
            source_lines,
            noise_lines_removed: 0,
            warnings: Vec::new(),
            samples: Vec::new(),
            elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
            error: Some(error),
        },
    }
}

fn compact_sample(text: &str) -> String {
    const MAX_CHARS: usize = 180;
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

fn smoke_blocks_with_source_line_ids(
    blocks: &[LiquidBlock],
    block_source_lines: &[LiquidBlockSourceLines],
) -> Vec<LiquidSmokeBlock> {
    let mut source_line_ids_by_block = BTreeMap::new();
    for entry in block_source_lines {
        source_line_ids_by_block.insert(
            entry.block_index,
            entry
                .lines
                .iter()
                .filter_map(|line| line.id.as_ref().map(ToOwned::to_owned))
                .collect::<Vec<_>>(),
        );
    }

    blocks
        .iter()
        .enumerate()
        .map(|(index, block)| LiquidSmokeBlock {
            role: block.role,
            text: block.text.clone(),
            label: block.label.clone(),
            source_line_ids: source_line_ids_by_block.remove(&index).unwrap_or_default(),
        })
        .collect()
}

fn smoke_deep_source_lines(lines: &[DeepLiquidSourceLine]) -> Vec<DeepLiquidSourceLine> {
    let mut lines = lines.to_vec();
    for line in &mut lines {
        strip_callout_sentinels(&mut line.text);
    }
    lines
}

fn smoke_block_source_lines(lines: &[LiquidBlockSourceLines]) -> Vec<LiquidBlockSourceLines> {
    let mut lines = lines.to_vec();
    for entry in &mut lines {
        for line in &mut entry.lines {
            strip_callout_sentinels(&mut line.text);
        }
    }
    lines
}

fn strip_callout_sentinels(text: &mut String) {
    if text.contains(layout_roles::CALLOUT_START) || text.contains(layout_roles::CALLOUT_END) {
        text.retain(|ch| ch != layout_roles::CALLOUT_START && ch != layout_roles::CALLOUT_END);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::liquid::LiquidSourceLineRef;

    #[test]
    fn smoke_blocks_include_source_line_ids_from_block_sources() {
        let blocks = vec![
            LiquidBlock {
                role: LiquidBlockRole::Title,
                text: "Title".to_owned(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Paragraph,
                text: "Body".to_owned(),
                label: Some("Body".to_owned()),
            },
        ];
        let block_source_lines = vec![LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![LiquidSourceLineRef {
                id: Some("p0:l0".to_owned()),
                page_index: 0,
                line_index: 0,
                text: "Body".to_owned(),
                role: LiquidBlockRole::Paragraph,
                note_markers: Vec::new(),
            }],
        }];

        let smoke_blocks = smoke_blocks_with_source_line_ids(&blocks, &block_source_lines);
        assert_eq!(smoke_blocks[0].source_line_ids, Vec::<String>::new());
        assert_eq!(smoke_blocks[1].source_line_ids, vec!["p0:l0".to_owned()]);
    }

    #[test]
    fn markdown_export_renders_reading_blocks_and_tracks_indices() {
        let blocks = [
            ("header", "YALE LAW JOURNAL"),
            ("title", "A Useful Article"),
            ("author_info", "JANE DOE"),
            ("heading", "I. Introduction"),
            ("paragraph", "This is the first paragraph."),
            ("marginalia", "1 This is a footnote."),
            ("noise", "123"),
        ];

        let (markdown, indices) = render_markdown_blocks(Some("A Useful Article"), blocks);

        assert!(markdown.contains("# A Useful Article"));
        assert!(!markdown.contains("## A Useful Article"));
        assert!(markdown.contains("_JANE DOE_"));
        assert!(markdown.contains("## I. Introduction"));
        assert!(markdown.contains("> [note] 1 This is a footnote."));
        assert!(!markdown.contains("YALE LAW JOURNAL"));
        assert!(!markdown.contains("\n123\n"));
        assert_eq!(indices.get(&0), None);
        assert_eq!(indices.get(&1), Some(&0));
        assert_eq!(indices.get(&4), Some(&3));
        assert_eq!(indices.get(&6), None);
    }

    #[test]
    fn markdown_export_strips_private_callout_sentinels() {
        let text = format!(
            "Body{}12{} text",
            layout_roles::CALLOUT_START,
            layout_roles::CALLOUT_END
        );
        let (markdown, _) = render_markdown_blocks(None, [("paragraph", text.as_str())]);
        assert_eq!(markdown, "Body12 text\n");
    }

    #[test]
    fn markdown_export_stem_is_stable_and_safe() {
        assert_eq!(
            export_stem("/tmp/A Big Law Review PDF.pdf", 6),
            "0007-a-big-law-review-pdf"
        );
        assert_eq!(export_stem("/tmp/!!!.pdf", 0), "0001-document");
    }
}
