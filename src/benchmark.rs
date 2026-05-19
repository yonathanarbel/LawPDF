use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use serde::Serialize;

use crate::pdf_backend::PdfEngine;

const BENCH_ZOOM: f32 = 1.25;
const BENCH_PIXELS_PER_POINT: f32 = 1.0;
const BENCH_MAX_TEXTURE_SIDE: u32 = 8192;

#[derive(Debug, Serialize)]
struct BenchReport {
    app_version: &'static str,
    timestamp_unix_secs: u64,
    executable_path: String,
    document_count: usize,
    failures: usize,
    pages_rendered: usize,
    total_ms: f64,
    avg_document_ms: f64,
    config: BenchConfig,
    documents: Vec<BenchDocumentResult>,
}

#[derive(Debug, Serialize)]
struct BenchConfig {
    zoom: f32,
    pixels_per_point: f32,
    max_texture_side: u32,
}

#[derive(Debug, Serialize)]
struct BenchDocumentResult {
    path: String,
    page_count: usize,
    pages_rendered: usize,
    load_ms: f64,
    first_page_render_ms: Option<f64>,
    render_ms: f64,
    total_ms: f64,
    page_render_ms: Vec<f64>,
    error: Option<String>,
}

pub fn run_scroll_benchmark(args: impl IntoIterator<Item = OsString>) -> Result<()> {
    let mut output_path = None;
    let mut paths = Vec::new();
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        if arg == OsStr::new("--bench-scroll") {
            continue;
        }
        if arg == OsStr::new("--bench-output") {
            let value = args
                .next()
                .context("--bench-output needs a destination path")?;
            output_path = Some(PathBuf::from(value));
            continue;
        }
        if arg.to_string_lossy().starts_with("--") {
            bail!("unknown benchmark argument: {}", arg.to_string_lossy());
        }
        paths.push(PathBuf::from(arg));
    }

    if paths.is_empty() {
        bail!("pass at least one PDF path after --bench-scroll");
    }

    let executable_path = std::env::current_exe()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| "unknown".to_owned());
    let started = Instant::now();
    let engine = PdfEngine::new().context("failed to initialize PDF engine")?;
    let mut documents = Vec::with_capacity(paths.len());

    for path in paths {
        documents.push(benchmark_document(&engine, &path));
    }

    let total_ms = started.elapsed().as_secs_f64() * 1000.0;
    let document_count = documents.len();
    let failures = documents
        .iter()
        .filter(|document| document.error.is_some())
        .count();
    let pages_rendered = documents
        .iter()
        .map(|document| document.pages_rendered)
        .sum::<usize>();
    let report = BenchReport {
        app_version: env!("CARGO_PKG_VERSION"),
        timestamp_unix_secs: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        executable_path,
        document_count,
        failures,
        pages_rendered,
        total_ms,
        avg_document_ms: if document_count > 0 {
            total_ms / document_count as f64
        } else {
            0.0
        },
        config: BenchConfig {
            zoom: BENCH_ZOOM,
            pixels_per_point: BENCH_PIXELS_PER_POINT,
            max_texture_side: BENCH_MAX_TEXTURE_SIDE,
        },
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
        "Benchmarked {} document(s), {} page(s), {} failure(s), total {:.1} ms, avg {:.1} ms/document",
        report.document_count,
        report.pages_rendered,
        report.failures,
        report.total_ms,
        report.avg_document_ms
    );

    Ok(())
}

fn benchmark_document(engine: &PdfEngine, path: &Path) -> BenchDocumentResult {
    let total_started = Instant::now();
    let load_started = Instant::now();
    let document = match engine.load_document(path) {
        Ok(document) => document,
        Err(error) => {
            return BenchDocumentResult {
                path: path.display().to_string(),
                page_count: 0,
                pages_rendered: 0,
                load_ms: load_started.elapsed().as_secs_f64() * 1000.0,
                first_page_render_ms: None,
                render_ms: 0.0,
                total_ms: total_started.elapsed().as_secs_f64() * 1000.0,
                page_render_ms: Vec::new(),
                error: Some(error.to_string()),
            };
        }
    };
    let load_ms = load_started.elapsed().as_secs_f64() * 1000.0;

    let render_started = Instant::now();
    let mut first_page_render_ms = None;
    let mut page_render_ms = Vec::with_capacity(document.page_count);
    let mut pages_rendered = 0usize;
    let mut error = None;

    for (page_index, page) in document.pages.iter().enumerate() {
        let render_scale = benchmark_page_render_scale(page.width, page.height);
        let page_started = Instant::now();
        match engine.render_page(&document.path, page_index, render_scale) {
            Ok(_) => {
                let elapsed_ms = page_started.elapsed().as_secs_f64() * 1000.0;
                if page_index == 0 {
                    first_page_render_ms = Some(elapsed_ms);
                }
                page_render_ms.push(elapsed_ms);
                pages_rendered += 1;
            }
            Err(render_error) => {
                error = Some(format!(
                    "page {} render failed: {}",
                    page_index + 1,
                    render_error
                ));
                break;
            }
        }
    }

    BenchDocumentResult {
        path: path.display().to_string(),
        page_count: document.page_count,
        pages_rendered,
        load_ms,
        first_page_render_ms,
        render_ms: render_started.elapsed().as_secs_f64() * 1000.0,
        total_ms: total_started.elapsed().as_secs_f64() * 1000.0,
        page_render_ms,
        error,
    }
}

fn benchmark_page_render_scale(page_width: f32, page_height: f32) -> f32 {
    let raw = (BENCH_ZOOM * BENCH_PIXELS_PER_POINT * 1.25).clamp(0.75, 3.25);
    let base_scale = ((raw * 8.0).round() / 8.0).clamp(0.75, 3.25);
    let safe_texture_side = BENCH_MAX_TEXTURE_SIDE.saturating_sub(16).max(256) as f32;
    let page_side = page_width.max(page_height).max(1.0);
    let max_scale = ((safe_texture_side / page_side) * 8.0).floor() / 8.0;
    base_scale.min(max_scale.max(0.25))
}
