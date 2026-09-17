#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod article_segments;
mod atomic_file;
#[cfg(feature = "devtools")]
mod benchmark;
mod chat;
mod credentials;
mod document_store;
mod hashing;
mod layout_roles;
mod liquid;
mod liquid2;
#[cfg(feature = "devtools")]
mod liquid_smoke;
mod liquidvision;
#[cfg(target_os = "macos")]
mod macos_open_files;
mod model;
mod native_process;
mod ocr;
mod page_geometry;
mod pdf_backend;
mod performance_cache;
mod render_worker;
mod review_reading;
mod settings;
#[cfg(feature = "devtools")]
mod shutdown_smoke;
mod single_instance;
mod storage_maintenance;
mod text_conversion;
mod text_search;
mod tts;
mod update_trust;
mod updater;

use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

use app::PdfEditorApp;

const APP_TITLE: &str = concat!(
    "LawPDF v",
    env!("CARGO_PKG_VERSION"),
    " - Y. Arbel design (2026)"
);

fn main() -> eframe::Result<()> {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    // The child has no UI, update service, credentials, or document-write
    // commands. Anonymous pipes carry its bounded request/response protocol.
    if args.len() == 1 && args[0] == "--pdf-worker" {
        if native_process::run_worker().is_err() {
            std::process::exit(1);
        }
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    macos_open_files::install_appkit_crash_workarounds();
    install_panic_log_hook();

    #[cfg(feature = "devtools")]
    if args.iter().any(|arg| arg == "--smoke-shutdown") {
        return shutdown_smoke::run();
    }

    if args.iter().any(|arg| arg == "--lm2-runtime-status") {
        if let Err(error) = liquid2::run_lm2_runtime_status(args.clone().into_iter()) {
            eprintln!("LiquidMode2 runtime verification failed: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    #[cfg(feature = "devtools")]
    if let Some(result) = dispatch_dev_command(&args) {
        if let Err(error) = result {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return Ok(());
    }

    let startup_paths = if let Some(source_paths) = convert_sources_from_args(&args) {
        match text_conversion::convert_sources_to_pdf(&source_paths) {
            Ok(outputs) => {
                let converted_paths = outputs
                    .into_iter()
                    .map(|output| output.destination)
                    .collect::<Vec<_>>();
                if args
                    .iter()
                    .any(|arg| arg == OsStr::new("--open-after-convert"))
                {
                    converted_paths
                } else {
                    return Ok(());
                }
            }
            Err(error) => {
                eprintln!("Conversion failed: {error:#}");
                show_conversion_error(&format!("{error:#}"));
                std::process::exit(1);
            }
        }
    } else {
        args.iter()
            .filter(|arg| !arg.to_string_lossy().starts_with("--"))
            .map(PathBuf::from)
            .collect::<Vec<_>>()
    };
    let single_instance = single_instance::initialize(&startup_paths);
    let (incoming_paths_tx, incoming_paths_rx) = match single_instance {
        single_instance::InstanceMode::Primary {
            incoming_paths_tx,
            incoming_paths_rx,
        } => (incoming_paths_tx, incoming_paths_rx),
        single_instance::InstanceMode::SecondarySent => return Ok(()),
        single_instance::InstanceMode::Unavailable(message) => {
            rfd::MessageDialog::new()
                .set_title("LawPDF is already open")
                .set_description(message)
                .set_level(rfd::MessageLevel::Error)
                .show();
            return Ok(());
        }
    };
    #[cfg(target_os = "macos")]
    let macos_open_files = macos_open_files::install(incoming_paths_tx);
    #[cfg(not(target_os = "macos"))]
    let _ = incoming_paths_tx;
    if let Some(pending_update) = updater::load_pending_update() {
        let relaunch_args = std::env::args_os().skip(1).collect::<Vec<_>>();
        match updater::start_update_helper(&pending_update, &relaunch_args) {
            Ok(()) => return Ok(()),
            Err(error) => eprintln!("Failed to start pending LawPDF update: {error}"),
        }
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(APP_TITLE)
            .with_icon(std::sync::Arc::new(load_app_icon()))
            .with_inner_size([1280.0, 860.0])
            .with_min_inner_size([720.0, 520.0]),
        ..Default::default()
    };

    eframe::run_native(
        APP_TITLE,
        options,
        Box::new(move |cc| {
            #[cfg(target_os = "macos")]
            {
                macos_open_files.register();
            }
            single_instance::set_repaint_context(&cc.egui_ctx);
            Ok(Box::new(PdfEditorApp::new(
                &cc.egui_ctx,
                startup_paths.clone(),
                incoming_paths_rx.clone(),
                #[cfg(target_os = "macos")]
                Some(macos_open_files),
            )))
        }),
    )
}

#[cfg(feature = "devtools")]
type DevCommandHandler = fn(Vec<OsString>) -> Result<(), String>;

#[cfg(feature = "devtools")]
struct DevCommand {
    flags: &'static [&'static str],
    handler: DevCommandHandler,
}

#[cfg(feature = "devtools")]
const DEV_COMMANDS: &[DevCommand] = &[
    DevCommand {
        flags: &["--smoke-credential-store"],
        handler: |_| credentials::verify_native_store(),
    },
    DevCommand {
        flags: &["--verify-update-manifest"],
        handler: |args| {
            let index = args.iter().position(|arg| arg == "--verify-update-manifest").unwrap();
            let manifest = args.get(index + 1).ok_or("Provide a manifest and signature path")?;
            let signature = args.get(index + 2).ok_or("Provide a manifest and signature path")?;
            let manifest = std::fs::read_to_string(manifest).map_err(|error| error.to_string())?;
            let signature = std::fs::read_to_string(signature).map_err(|error| error.to_string())?;
            let release = update_trust::verify(&manifest, &signature)?;
            println!("Authenticated LawPDF {} manifest for {}", release.version, release.commit);
            Ok(())
        },
    },
    DevCommand {
        flags: &["--smoke-pdf-isolation"],
        handler: |_| {
            native_process::verify_supervision(
                &smoke_pdf_path().map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())
        },
    },
    DevCommand {
        flags: &["--smoke-open-default"],
        handler: dev_smoke_open_default,
    },
    DevCommand {
        flags: &["--smoke-render-worker"],
        handler: dev_smoke_render_worker,
    },
    DevCommand {
        flags: &["--bench-pdf-search"],
        handler: dev_bench_pdf_search,
    },
    DevCommand {
        flags: &["--bench-scroll"],
        handler: dev_bench_scroll,
    },
    DevCommand {
        flags: &["--smoke-liquid", "--smoke-liquid2"],
        handler: dev_smoke_liquid,
    },
    DevCommand {
        flags: &["--segment-smoke-report"],
        handler: dev_segment_smoke_report,
    },
    DevCommand {
        flags: &["--lm2-assemble-markdown"],
        handler: dev_lm2_assemble_markdown,
    },
    DevCommand {
        flags: &["--lm2-copy-md"],
        handler: dev_lm2_copy_md,
    },
    DevCommand {
        flags: &["--lm2-timing-baseline"],
        handler: dev_lm2_timing_baseline,
    },
    DevCommand {
        flags: &["--lm2-eval"],
        handler: dev_lm2_eval,
    },
    DevCommand {
        flags: &["--dump-char-metrics"],
        handler: dev_char_metrics_dump,
    },
    DevCommand {
        flags: &["--lm2-source-smoke"],
        handler: dev_lm2_source_smoke,
    },
];

#[cfg(feature = "devtools")]
fn dispatch_dev_command(args: &[OsString]) -> Option<Result<(), String>> {
    let command = DEV_COMMANDS.iter().find(|command| {
        command
            .flags
            .iter()
            .any(|flag| args.iter().any(|arg| arg == OsStr::new(flag)))
    })?;
    let flag = command
        .flags
        .iter()
        .find(|flag| args.iter().any(|arg| arg == OsStr::new(flag)))
        .expect("matched dev command has a matching flag");
    Some((command.handler)(args.to_vec()).map_err(|error| format!("{flag} failed: {error}")))
}

#[cfg(feature = "devtools")]
fn dev_smoke_open_default(_args: Vec<OsString>) -> Result<(), String> {
    smoke_open_default();
    Ok(())
}

#[cfg(feature = "devtools")]
fn dev_smoke_render_worker(_args: Vec<OsString>) -> Result<(), String> {
    smoke_render_worker();
    Ok(())
}

#[cfg(feature = "devtools")]
fn dev_bench_pdf_search(_args: Vec<OsString>) -> Result<(), String> {
    bench_pdf_search()
}

#[cfg(feature = "devtools")]
fn dev_bench_scroll(args: Vec<OsString>) -> Result<(), String> {
    benchmark::run_scroll_benchmark(args.into_iter()).map_err(|error| format!("{error:#}"))
}

#[cfg(feature = "devtools")]
fn dev_smoke_liquid(args: Vec<OsString>) -> Result<(), String> {
    liquid_smoke::run_liquid_smoke(args.into_iter()).map_err(|error| format!("{error:#}"))
}

#[cfg(feature = "devtools")]
fn dev_segment_smoke_report(args: Vec<OsString>) -> Result<(), String> {
    liquid_smoke::run_article_segment_report(args.into_iter()).map_err(|error| format!("{error:#}"))
}

#[cfg(feature = "devtools")]
fn dev_lm2_assemble_markdown(args: Vec<OsString>) -> Result<(), String> {
    liquid_smoke::run_lm2_assemble_markdown(args.into_iter()).map_err(|error| format!("{error:#}"))
}

#[cfg(feature = "devtools")]
fn dev_lm2_copy_md(args: Vec<OsString>) -> Result<(), String> {
    liquid_smoke::run_lm2_copy_md(args.into_iter()).map_err(|error| format!("{error:#}"))
}

#[cfg(feature = "devtools")]
fn dev_lm2_timing_baseline(args: Vec<OsString>) -> Result<(), String> {
    liquid_smoke::run_lm2_timing_baseline(args.into_iter()).map_err(|error| format!("{error:#}"))
}

#[cfg(feature = "devtools")]
fn dev_lm2_eval(args: Vec<OsString>) -> Result<(), String> {
    liquid2::run_lm2_eval(args.into_iter())
}

#[cfg(feature = "devtools")]
fn dev_char_metrics_dump(args: Vec<OsString>) -> Result<(), String> {
    liquid_smoke::run_char_metrics_dump(args.into_iter()).map_err(|error| format!("{error:#}"))
}

#[cfg(feature = "devtools")]
fn dev_lm2_source_smoke(args: Vec<OsString>) -> Result<(), String> {
    liquid2::run_lm2_source_smoke(args.into_iter())
}

fn convert_sources_from_args(args: &[OsString]) -> Option<Vec<PathBuf>> {
    let mut saw_flag = false;
    let mut paths = Vec::new();

    for arg in args {
        if arg == OsStr::new("--convert-to-pdf") {
            saw_flag = true;
            continue;
        }
        if saw_flag && !arg.to_string_lossy().starts_with("--") {
            paths.push(PathBuf::from(arg));
        }
    }

    saw_flag.then_some(paths)
}

fn show_conversion_error(message: &str) {
    let _ = rfd::MessageDialog::new()
        .set_title("LawPDF conversion failed")
        .set_description(message)
        .set_level(rfd::MessageLevel::Error)
        .show();
}

fn load_app_icon() -> egui::IconData {
    eframe::icon_data::from_png_bytes(include_bytes!("../assets/lawpdf.png"))
        .expect("bundled LawPDF icon should be a valid PNG")
}

#[cfg(feature = "devtools")]
fn smoke_open_default() {
    match pdf_backend::PdfEngine::new().and_then(|engine| {
        let path = smoke_pdf_path()?;
        let document = engine.load_document(&path)?;
        let first_page = engine.render_page(&document.path, 0, 1.0)?;
        Ok((document, first_page))
    }) {
        Ok((document, first_page)) => {
            println!(
                "Opened default PDF: {} ({} pages); rendered page 1 at {}x{}",
                document.path.display(),
                document.page_count,
                first_page.width,
                first_page.height
            );
        }
        Err(error) => {
            eprintln!("Failed to open default PDF: {error:#}");
            std::process::exit(1);
        }
    }
}

#[cfg(feature = "devtools")]
fn smoke_render_worker() {
    use std::time::Duration;

    let path = match smoke_pdf_path() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    };
    let (render_tx, render_rx) = render_worker::spawn_render_worker(None);
    let (load_tx, load_rx) = crossbeam_channel::unbounded();
    if let Err(error) = render_tx.send(render_worker::RenderRequest::LoadDocument {
        path: path.clone(),
        optimize_large_documents: true,
        reply: load_tx,
    }) {
        eprintln!("Failed to send load request: {error}");
        std::process::exit(1);
    }
    match load_rx.recv_timeout(Duration::from_secs(10)) {
        Ok(Ok(document)) => println!(
            "Worker opened default PDF metadata: {} page(s)",
            document.page_count
        ),
        Ok(Err(error)) => {
            eprintln!("Worker failed to open default PDF: {error}");
            std::process::exit(1);
        }
        Err(error) => {
            eprintln!("Timed out waiting for worker load result: {error}");
            std::process::exit(1);
        }
    }

    let render_scale = std::env::var("LAWPDF_SMOKE_SCALE")
        .ok()
        .and_then(|value| value.parse::<f32>().ok())
        .unwrap_or(1.0);
    let key = render_worker::PageRenderKey::new(1, 0, render_scale);
    if let Err(error) = render_tx.send(render_worker::RenderRequest::Page {
        key,
        path,
        zoom: 1.0,
        render_scale,
        fast: false,
    }) {
        eprintln!("Failed to send render request: {error}");
        std::process::exit(1);
    }

    match render_rx.recv_timeout(Duration::from_secs(10)) {
        Ok(render_worker::RenderEvent::Page { result, .. }) => match result {
            Ok(page) => {
                println!("Worker rendered page 1 at {}x{}", page.width, page.height);
            }
            Err(error) => {
                eprintln!("Worker failed to render page 1: {error}");
                std::process::exit(1);
            }
        },
        Ok(other) => {
            eprintln!("Unexpected worker event: {other:?}");
            std::process::exit(1);
        }
        Err(error) => {
            eprintln!("Timed out waiting for worker render result: {error}");
            std::process::exit(1);
        }
    }
}

#[cfg(feature = "devtools")]
fn bench_pdf_search() -> Result<(), String> {
    use std::time::Instant;

    let path = smoke_pdf_path().map_err(|error| format!("{error:#}"))?;
    let query = std::env::var("LAWPDF_SEARCH_QUERY")
        .unwrap_or_else(|_| "instrument".to_owned())
        .to_lowercase();
    let engine = pdf_backend::PdfEngine::new().map_err(|error| format!("{error:#}"))?;
    let opened_at = Instant::now();
    let document = engine
        .load_document_adaptive(&path, true)
        .map_err(|error| format!("{error:#}"))?;
    let metadata_seconds = opened_at.elapsed().as_secs_f64();

    for pass in 1..=2 {
        let started = Instant::now();
        let mut text_bytes = 0usize;
        let mut matches = 0usize;
        let mut first_hit_page = None;
        for page_index in 0..document.page_count {
            let text = engine
                .load_page_text(&path, page_index)
                .map_err(|error| format!("{error:#}"))?;
            text_bytes += text.len();
            let page_matches = text.to_lowercase().matches(&query).count();
            matches += page_matches;
            if page_matches > 0 && first_hit_page.is_none() {
                first_hit_page = Some(page_index);
            }
        }
        if let Some(page_index) = first_hit_page {
            let text = engine
                .load_page_text(&path, page_index)
                .map_err(|error| format!("{error:#}"))?;
            let chars = engine
                .load_page_text_chars(&path, page_index)
                .map_err(|error| format!("{error:#}"))?;
            if let Some(start) = text.to_lowercase().find(&query) {
                let end = start + query.len();
                let start_char = text[..start].chars().count();
                let end_char = text[..end].chars().count();
                let geometry_text = chars
                    .get(start_char..end_char)
                    .unwrap_or_default()
                    .iter()
                    .map(|char| char.ch)
                    .collect::<String>();
                println!(
                    "Search pass {pass}: first hit page={} geometry_text={geometry_text:?}",
                    page_index + 1
                );
            }
        }
        println!(
            "Search pass {pass}: pages={} text_bytes={text_bytes} matches={matches} elapsed={:.3}s",
            document.page_count,
            started.elapsed().as_secs_f64()
        );
    }
    let links_started = Instant::now();
    let links = engine
        .load_document_links(&path, document.page_count)
        .map_err(|error| format!("{error:#}"))?;
    let link_count = links.iter().map(Vec::len).sum::<usize>();
    println!(
        "Hyperlinks: count={link_count} elapsed={:.3}s",
        links_started.elapsed().as_secs_f64()
    );
    println!("Metadata elapsed={metadata_seconds:.3}s");
    Ok(())
}

#[cfg(feature = "devtools")]
fn smoke_pdf_path() -> anyhow::Result<std::path::PathBuf> {
    std::env::var("LAWPDF_SMOKE_PDF")
        .or_else(|_| std::env::var("LAWPDF_DEFAULT_PDF"))
        .map(std::path::PathBuf::from)
        .map_err(|_| {
            anyhow::anyhow!("Set LAWPDF_SMOKE_PDF to a PDF path before running smoke tests.")
        })
}

/// Record every panic to `<app data>/panic.log` before the default hook runs.
///
/// Release builds on Windows use the `windows` subsystem, so a panic message
/// printed to stderr goes nowhere and the app simply vanishes (or, for a
/// worker thread, silently stops answering). The log gives the user, and a bug
/// report, something to point at.
fn install_panic_log_hook() {
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let location = info
            .location()
            .map(|location| format!("{}:{}", location.file(), location.line()))
            .unwrap_or_else(|| "unknown location".to_owned());
        let line = format!(
            "{} LawPDF {} thread={} at {}\n",
            time::OffsetDateTime::now_utc()
                .format(&time::format_description::well_known::Rfc3339)
                .unwrap_or_else(|_| "unknown-time".to_owned()),
            env!("CARGO_PKG_VERSION"),
            thread.name().unwrap_or("unnamed"),
            location
        );
        if let Some(dir) = settings::app_data_dir() {
            let _ = std::fs::create_dir_all(&dir);
            let path = dir.join("panic.log");
            // Never persist panic payloads: they can contain PDF text or keys.
            if std::fs::metadata(&path).is_ok_and(|metadata| metadata.len() > 64 * 1024) {
                let _ = std::fs::remove_file(&path);
            }
            let mut options = std::fs::OpenOptions::new();
            options.create(true).append(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            if let Ok(mut file) = options.open(path) {
                use std::io::Write;
                let _ = file.write_all(line.as_bytes());
            }
        }
        eprintln!("LawPDF encountered an unexpected error at {location}. Check document recovery when reopening the app.");
    }));
}
