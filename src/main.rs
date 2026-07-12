#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod benchmark;
mod chat;
mod layout_roles;
mod liquid;
mod liquid2;
mod liquid_smoke;
mod liquidvision;
mod model;
mod ocr;
mod pdf_backend;
mod profile_dataset;
mod render_worker;
mod settings;
mod single_instance;
mod text_conversion;
mod tts;
mod updater;

use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

use app::PdfEditorApp;

const APP_TITLE: &str = "LawPDF - Y. Arbel design (2026)";

fn main() -> eframe::Result<()> {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();

    if args.iter().any(|arg| arg == "--smoke-open-default") {
        smoke_open_default();
        return Ok(());
    }
    if args.iter().any(|arg| arg == "--smoke-render-worker") {
        smoke_render_worker();
        return Ok(());
    }
    if args.iter().any(|arg| arg == "--lm2-runtime-status") {
        if let Err(error) = liquid2::run_lm2_runtime_status(args.clone().into_iter()) {
            eprintln!("LiquidMode2 runtime verification failed: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if args.iter().any(|arg| arg == "--bench-scroll") {
        if let Err(error) = benchmark::run_scroll_benchmark(args.clone().into_iter()) {
            eprintln!("Benchmark failed: {error:#}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if args.iter().any(|arg| arg == "--smoke-liquid") {
        if let Err(error) = liquid_smoke::run_liquid_smoke(args.clone().into_iter()) {
            eprintln!("Liquid smoke failed: {error:#}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if args.iter().any(|arg| arg == "--smoke-liquid2") {
        if let Err(error) = liquid_smoke::run_liquid_smoke(args.clone().into_iter()) {
            eprintln!("LiquidMode2 smoke failed: {error:#}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if args.iter().any(|arg| arg == "--lm2-assemble-markdown") {
        if let Err(error) = liquid_smoke::run_lm2_assemble_markdown(args.clone().into_iter()) {
            eprintln!("LiquidMode2 Markdown export failed: {error:#}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if args.iter().any(|arg| arg == "--lm2-timing-baseline") {
        if let Err(error) = liquid_smoke::run_lm2_timing_baseline(args.clone().into_iter()) {
            eprintln!("LiquidMode2 timing baseline failed: {error:#}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if args.iter().any(|arg| arg == "--lm2-eval") {
        if let Err(error) = liquid2::run_lm2_eval(args.clone().into_iter()) {
            eprintln!("LiquidMode2 eval failed: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if args.iter().any(|arg| arg == "--dump-lm2-features") {
        if let Err(error) = liquid2::run_lm2_feature_dump(args.clone().into_iter()) {
            eprintln!("LiquidMode2 feature dump failed: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if args.iter().any(|arg| arg == "--dump-lm2-decoder-lattice") {
        if let Err(error) = liquid2::run_lm2_decoder_lattice_dump(args.clone().into_iter()) {
            eprintln!("LiquidMode2 decoder lattice dump failed: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if args.iter().any(|arg| arg == "--lm2-draft") {
        if let Err(error) = liquid2::run_lm2_draft(args.clone().into_iter()) {
            eprintln!("LiquidMode2 draft failed: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if args.iter().any(|arg| arg == "--lm2-source-smoke") {
        if let Err(error) = liquid2::run_lm2_source_smoke(args.clone().into_iter()) {
            eprintln!("LiquidMode2 source smoke failed: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if args.iter().any(|arg| arg == "--profile-dataset") {
        if let Err(error) = profile_dataset::run_profile_dataset(args.clone().into_iter()) {
            eprintln!("Profile dataset failed: {error:#}");
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
    let incoming_paths_rx = match single_instance {
        single_instance::InstanceMode::Primary { incoming_paths_rx } => incoming_paths_rx,
        single_instance::InstanceMode::SecondarySent => return Ok(()),
    };
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
            .with_min_inner_size([980.0, 640.0]),
        ..Default::default()
    };

    eframe::run_native(
        APP_TITLE,
        options,
        Box::new(move |cc| {
            Ok(Box::new(PdfEditorApp::new(
                cc,
                startup_paths.clone(),
                incoming_paths_rx.clone(),
            )))
        }),
    )
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

fn smoke_render_worker() {
    use std::time::Duration;

    let path = match smoke_pdf_path() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    };
    let (render_tx, render_rx) = render_worker::spawn_render_worker();
    let (load_tx, load_rx) = crossbeam_channel::unbounded();
    if let Err(error) = render_tx.send(render_worker::RenderRequest::LoadDocument {
        path: path.clone(),
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

    let key = render_worker::PageRenderKey::new(1, 0, 1.0, 1.0);
    if let Err(error) = render_tx.send(render_worker::RenderRequest::Page {
        key,
        path,
        zoom: 1.0,
        render_scale: 1.0,
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

fn smoke_pdf_path() -> anyhow::Result<std::path::PathBuf> {
    std::env::var("LAWPDF_SMOKE_PDF")
        .or_else(|_| std::env::var("LAWPDF_DEFAULT_PDF"))
        .map(std::path::PathBuf::from)
        .map_err(|_| {
            anyhow::anyhow!("Set LAWPDF_SMOKE_PDF to a PDF path before running smoke tests.")
        })
}
