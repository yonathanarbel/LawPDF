//! Native-window lifecycle check for release QA; excluded from production builds.
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

static RUNNING: AtomicBool = AtomicBool::new(false);

pub fn running() -> bool {
    RUNNING.load(Ordering::Relaxed)
}

pub fn run() -> eframe::Result<()> {
    RUNNING.store(true, Ordering::Relaxed);
    let paths: Vec<_> = std::env::args_os().skip_while(|arg| arg != "--smoke-shutdown")
        .skip(1).map(std::path::PathBuf::from).collect();
    let delay = if paths.is_empty() { Duration::from_millis(350) } else { Duration::from_secs(3) };
    let (tx, rx) = crossbeam_channel::unbounded();
    #[cfg(target_os = "macos")]
    let registration = crate::macos_open_files::install(tx);
    #[cfg(not(target_os = "macos"))]
    let _ = tx;
    struct Smoke {
        app: crate::app::PdfEditorApp,
        opened: Instant,
        delay: Duration,
    }
    impl eframe::App for Smoke {
        fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
            self.app.update(ctx, frame);
            if self.opened.elapsed() >= self.delay {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            } else {
                ctx.request_repaint_after(Duration::from_millis(50));
            }
        }
        fn on_exit(&mut self, gl: Option<&eframe::glow::Context>) {
            self.app.on_exit(gl);
        }
    }
    eframe::run_native(
        "LawPDF shutdown verification",
        eframe::NativeOptions::default(),
        Box::new(move |cc| {
            #[cfg(target_os = "macos")]
            registration.register();
            Ok(Box::new(Smoke {
                app: crate::app::PdfEditorApp::new(
                    &cc.egui_ctx,
                    paths,
                    rx,
                    #[cfg(target_os = "macos")]
                    Some(registration),
                ),
                opened: Instant::now(),
                delay,
            }))
        }),
    )?;
    println!("LawPDF native shutdown completed cleanly");
    Ok(())
}
