use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use image::ColorType;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::pdf_backend::PdfEngine;

const BENCH_ZOOM: f32 = 1.25;
const BENCH_PIXELS_PER_POINT: f32 = 1.0;
const BENCH_MAX_TEXTURE_SIDE: u32 = 8192;

/// Time the actual open/recovery pipeline through the first CPU raster.
/// The caller supplies a fresh, absolute LAWPDF_BENCH_DATA_DIR so a benchmark
/// never writes to or prunes the reader's normal recovery and performance data.
pub fn run_open_pipeline_benchmark(args: Vec<OsString>) -> Result<()> {
    use crate::render_worker::{PageRenderKey, RenderEvent, RenderRequest, spawn_render_worker};
    use std::time::Duration;

    let (source, scale) = parse_open_pipeline_args(&args)?;
    let source = source
        .canonicalize()
        .context("Could not resolve the PDF path")?;
    let metadata = std::fs::metadata(&source).context("Could not inspect the PDF")?;
    anyhow::ensure!(metadata.is_file(), "The benchmark input must be a file");
    let data_dir = std::env::var_os("LAWPDF_BENCH_DATA_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .context("Set LAWPDF_BENCH_DATA_DIR to a fresh absolute directory path")?;
    std::fs::create_dir(&data_dir).with_context(|| {
        format!(
            "Benchmark data directory must not already exist: {}",
            data_dir.display()
        )
    })?;

    let start = Instant::now();
    let (worker, events) = spawn_render_worker(None);
    worker
        .send(RenderRequest::OpenDocument {
            path: source.clone(),
            optimize_large_documents: true,
            preview_page: Some(0),
        })
        .map_err(|error| anyhow::anyhow!("Could not queue document opening: {error}"))?;
    let mut preview_ready_ms = None;
    let opened = loop {
        let remaining = Duration::from_secs(60).saturating_sub(start.elapsed());
        match events
            .recv_timeout(remaining)
            .context("Timed out opening the PDF")?
        {
            RenderEvent::OpeningPreview { .. } => {
                preview_ready_ms = Some(start.elapsed().as_secs_f64() * 1000.0);
            }
            RenderEvent::DocumentOpened { result, .. } => {
                break result.map_err(anyhow::Error::msg)?;
            }
            _ => bail!("The worker returned an unexpected opening event"),
        }
    };
    let open_ms = start.elapsed().as_secs_f64() * 1000.0;
    anyhow::ensure!(opened.document.page_count > 0, "The PDF has no pages");
    worker.set_live_documents([1]);
    worker.set_viewport(1, 0, 0, 0);
    let key = PageRenderKey::new(1, 0, scale);
    let render_started = Instant::now();
    worker
        .send(RenderRequest::Page {
            key,
            path: source.clone(),
            zoom: 1.0,
            render_scale: scale,
            fast: false,
        })
        .map_err(|error| anyhow::anyhow!("Could not queue the first page: {error}"))?;
    let page = loop {
        let remaining = Duration::from_secs(30).saturating_sub(render_started.elapsed());
        match events
            .recv_timeout(remaining)
            .context("Timed out rendering the first page")?
        {
            RenderEvent::Page { result, .. } => break result.map_err(anyhow::Error::msg)?,
            // A preview arriving after the verified document is no longer useful.
            RenderEvent::OpeningPreview { .. } => {}
            _ => bail!("The worker returned an unexpected page event"),
        }
    };
    let render_ms = render_started.elapsed().as_secs_f64() * 1000.0;
    let ready_ms = start.elapsed().as_secs_f64() * 1000.0;
    opened
        .revision
        .require_current(&source)
        .map_err(anyhow::Error::msg)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "schema_version": "lawpdf-open-pipeline-benchmark-v1",
            "app_version": env!("CARGO_PKG_VERSION"),
            "input": source, "input_bytes": metadata.len(), "page_count": opened.document.page_count,
            "optimized_open": true, "render_scale": scale,
            "open_ms": open_ms, "first_page_render_ms": render_ms, "first_page_ready_ms": ready_ms,
            "read_only_preview_ready_ms": preview_ready_ms,
            "open_phase_ms": {
                "recovery_snapshot": opened.timings.snapshot.as_secs_f64() * 1000.0,
                "pdf_metadata": opened.timings.metadata.as_secs_f64() * 1000.0,
                "annotations": opened.timings.annotations.as_secs_f64() * 1000.0,
                "revision_and_recovery": opened.timings.revision_and_recovery.as_secs_f64() * 1000.0
            },
            "first_page_pixels": [page.width, page.height], "source_unchanged": true,
            "data_directory": data_dir,
            "scope": "Recovery snapshot, metadata, annotation read, first CPU raster and UI pixel preparation; excludes window, GPU upload and presentation."
        }))?
    );
    drop(worker);
    crate::native_process::shutdown();
    Ok(())
}

fn parse_open_pipeline_args(args: &[OsString]) -> Result<(PathBuf, f32)> {
    anyhow::ensure!(
        args.len() == 2 || args.len() == 3,
        "Usage: --bench-open-pipeline PDF [render-scale]"
    );
    anyhow::ensure!(
        args[0] == OsStr::new("--bench-open-pipeline"),
        "Expected --bench-open-pipeline"
    );
    let scale = if args.len() == 3 {
        args[2]
            .to_string_lossy()
            .parse::<f32>()
            .context("Render scale must be a number")?
    } else {
        1.0
    };
    anyhow::ensure!(
        scale.is_finite() && (0.25..=4.0).contains(&scale),
        "Render scale must be between 0.25 and 4.0"
    );
    Ok((PathBuf::from(&args[1]), scale))
}

#[cfg(test)]
mod open_pipeline_tests {
    use super::*;

    #[test]
    fn open_pipeline_arguments_require_a_path_and_safe_scale() {
        let args = |values: &[&str]| values.iter().map(OsString::from).collect::<Vec<_>>();
        assert_eq!(
            parse_open_pipeline_args(&args(&["--bench-open-pipeline", "paper.pdf"]))
                .unwrap()
                .1,
            1.0
        );
        assert_eq!(
            parse_open_pipeline_args(&args(&["--bench-open-pipeline", "paper.pdf", "2"]))
                .unwrap()
                .1,
            2.0
        );
        for invalid in [
            &["--bench-open-pipeline"][..],
            &["--bench-open-pipeline", "paper.pdf", "NaN"][..],
            &["--bench-open-pipeline", "paper.pdf", "8"][..],
        ] {
            assert!(parse_open_pipeline_args(&args(invalid)).is_err());
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum BenchLoadMode {
    Full,
    Adaptive,
    Metadata,
}

impl BenchLoadMode {
    fn label(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Adaptive => "adaptive",
            Self::Metadata => "metadata",
        }
    }
}

#[derive(Debug, Serialize)]
struct BenchReport {
    app_version: &'static str,
    timestamp_unix_secs: u64,
    executable_path: String,
    document_count: usize,
    failures: usize,
    pages_rendered: usize,
    wall_ms: f64,
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
    max_pages: Option<usize>,
    bitmap_conversion: &'static str,
    performance_cache_disabled: bool,
    load_mode: &'static str,
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
    page_dimensions: Vec<[usize; 2]>,
    page_rgba_sha256: Vec<String>,
    page_images: Vec<String>,
    error: Option<String>,
}

pub fn run_scroll_benchmark(args: impl IntoIterator<Item = OsString>) -> Result<()> {
    let mut output_path = None;
    let mut image_dir = None;
    let mut max_pages = None;
    let mut load_mode = BenchLoadMode::Adaptive;
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
        if arg == OsStr::new("--bench-images") {
            let value = args
                .next()
                .context("--bench-images needs a destination directory")?;
            image_dir = Some(PathBuf::from(value));
            continue;
        }
        if arg == OsStr::new("--bench-max-pages") {
            let value = args.next().context("--bench-max-pages needs a number")?;
            let parsed = value
                .to_string_lossy()
                .parse::<usize>()
                .context("--bench-max-pages must be a positive integer")?;
            if parsed == 0 {
                bail!("--bench-max-pages must be greater than zero");
            }
            max_pages = Some(parsed);
            continue;
        }
        if arg == OsStr::new("--bench-load") {
            let value = args
                .next()
                .context("--bench-load needs one of: full, adaptive, metadata")?;
            load_mode = match value.to_string_lossy().as_ref() {
                "full" => BenchLoadMode::Full,
                "adaptive" => BenchLoadMode::Adaptive,
                "metadata" => BenchLoadMode::Metadata,
                other => {
                    bail!("unknown --bench-load mode {other:?}; use full, adaptive, or metadata")
                }
            };
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
    if let Some(image_dir) = &image_dir {
        std::fs::create_dir_all(image_dir)
            .with_context(|| format!("failed to create {}", image_dir.display()))?;
    }

    let executable_path = std::env::current_exe()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| "unknown".to_owned());
    let started = Instant::now();
    let engine = PdfEngine::new().context("failed to initialize PDF engine")?;
    let mut documents = Vec::with_capacity(paths.len());

    for (document_index, path) in paths.into_iter().enumerate() {
        documents.push(benchmark_document(
            &engine,
            &path,
            document_index,
            max_pages,
            image_dir.as_deref(),
            load_mode,
        ));
    }

    let wall_ms = started.elapsed().as_secs_f64() * 1000.0;
    let document_count = documents.len();
    let failures = documents
        .iter()
        .filter(|document| document.error.is_some())
        .count();
    let pages_rendered = documents
        .iter()
        .map(|document| document.pages_rendered)
        .sum::<usize>();
    let total_ms = documents
        .iter()
        .map(|document| document.total_ms)
        .sum::<f64>();
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
        wall_ms,
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
            max_pages,
            bitmap_conversion: if cfg!(feature = "bench-image-conversion") {
                "image_to_rgba8"
            } else {
                "direct_rgba_bytes"
            },
            performance_cache_disabled: std::env::var_os("LAWPDF_DISABLE_PERFORMANCE_CACHE")
                .is_some(),
            load_mode: load_mode.label(),
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

fn benchmark_document(
    engine: &PdfEngine,
    path: &Path,
    document_index: usize,
    max_pages: Option<usize>,
    image_dir: Option<&Path>,
    load_mode: BenchLoadMode,
) -> BenchDocumentResult {
    let total_started = Instant::now();
    let load_started = Instant::now();
    let document_result = match load_mode {
        BenchLoadMode::Full => engine.load_document(path),
        BenchLoadMode::Adaptive => engine.load_document_adaptive(path, true),
        BenchLoadMode::Metadata => engine.load_document_metadata_only(path),
    };
    let document = match document_result {
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
                page_dimensions: Vec::new(),
                page_rgba_sha256: Vec::new(),
                page_images: Vec::new(),
                error: Some(error.to_string()),
            };
        }
    };
    let load_ms = load_started.elapsed().as_secs_f64() * 1000.0;

    let mut first_page_render_ms = None;
    let mut page_render_ms = Vec::with_capacity(document.page_count);
    let mut page_dimensions = Vec::with_capacity(document.page_count);
    let mut page_rgba_sha256 = Vec::with_capacity(document.page_count);
    let mut page_images = Vec::new();
    let mut pages_rendered = 0usize;
    let mut error = None;

    let pages_to_render = max_pages
        .unwrap_or(document.page_count)
        .min(document.page_count);
    for (page_index, page) in document.pages.iter().take(pages_to_render).enumerate() {
        let render_scale = benchmark_page_render_scale(page.width, page.height);
        let page_started = Instant::now();
        match engine.render_page(&document.path, page_index, render_scale) {
            Ok(rendered) => {
                let elapsed_ms = page_started.elapsed().as_secs_f64() * 1000.0;
                if page_index == 0 {
                    first_page_render_ms = Some(elapsed_ms);
                }
                page_render_ms.push(elapsed_ms);
                page_dimensions.push([rendered.width, rendered.height]);
                page_rgba_sha256.push(format!("{:x}", Sha256::digest(rendered.rgba.as_slice())));
                if let Some(image_dir) = image_dir {
                    let image_path =
                        image_dir.join(format!("d{document_index:03}-p{page_index:04}.png"));
                    if let Err(save_error) = image::save_buffer(
                        &image_path,
                        &rendered.rgba,
                        rendered.width as u32,
                        rendered.height as u32,
                        ColorType::Rgba8,
                    ) {
                        error = Some(format!(
                            "page {} image export failed: {save_error}",
                            page_index + 1
                        ));
                        break;
                    }
                    page_images.push(image_path.display().to_string());
                }
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

    let render_ms = page_render_ms.iter().sum::<f64>();
    BenchDocumentResult {
        path: path.display().to_string(),
        page_count: document.page_count,
        pages_rendered,
        load_ms,
        first_page_render_ms,
        render_ms,
        total_ms: load_ms + render_ms,
        page_render_ms,
        page_dimensions,
        page_rgba_sha256,
        page_images,
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
