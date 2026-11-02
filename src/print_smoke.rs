//! Native print-dialog QA. Compiled only for Windows devtools builds.
//! Exercises Ctrl+P and captures the application's own rendered viewport.
//! This harness never clicks Print or invokes a printer backend.

use eframe::App;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(30);

struct ScreenshotReport {
    width: usize,
    height: usize,
}

type Outcome = Arc<Mutex<Option<Result<ScreenshotReport, String>>>>;

struct PrintSmoke {
    app: crate::app::PdfEditorApp,
    opened: Instant,
    destination: PathBuf,
    outcome: Outcome,
    key_sent: bool,
    key_released: bool,
    preview_ready_at: Option<Instant>,
    screenshot_requested: bool,
    finished: bool,
}

impl PrintSmoke {
    fn finish(&mut self, ctx: &egui::Context, result: Result<ScreenshotReport, String>) {
        if !self.finished {
            *self
                .outcome
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = Some(result);
            self.finished = true;
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

impl App for PrintSmoke {
    fn raw_input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        self.app.raw_input_hook(ctx, input);
        if self.finished {
            return;
        }
        if !self.key_sent {
            match self.app.print_qa_state() {
                Ok((true, _)) => {
                    let modifiers = egui::Modifiers {
                        ctrl: true,
                        command: true,
                        ..Default::default()
                    };
                    input.modifiers = modifiers;
                    input.events.push(egui::Event::Key {
                        key: egui::Key::P,
                        physical_key: Some(egui::Key::P),
                        pressed: true,
                        repeat: false,
                        modifiers,
                    });
                    self.key_sent = true;
                    crate::diagnostics::event("print_smoke_ctrl_p_injected");
                }
                Err(error) => self.finish(ctx, Err(error)),
                Ok((false, _)) => {}
            }
        } else if !self.key_released {
            input.modifiers = egui::Modifiers::default();
            input.events.push(egui::Event::Key {
                key: egui::Key::P,
                physical_key: Some(egui::Key::P),
                pressed: false,
                repeat: false,
                modifiers: egui::Modifiers::default(),
            });
            self.key_released = true;
        }
    }

    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.app.update(ctx, frame);
        if self.finished {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if self.screenshot_requested {
            let screenshot = ctx.input(|input| {
                input.events.iter().find_map(|event| match event {
                    egui::Event::Screenshot {
                        viewport_id, image, ..
                    } if *viewport_id == egui::ViewportId::ROOT => Some(image.clone()),
                    _ => None,
                })
            });
            if let Some(image) = screenshot {
                let result = save_screenshot(&self.destination, &image);
                self.finish(ctx, result);
                return;
            }
        }
        if self.opened.elapsed() >= TIMEOUT {
            let stage = if !self.key_sent {
                "opening the fixture document"
            } else if !self.screenshot_requested {
                "waiting for Ctrl+P to open a complete print preview"
            } else {
                "waiting for the viewport screenshot"
            };
            self.finish(
                ctx,
                Err(format!(
                    "Print UI QA timed out after 30 seconds while {stage}."
                )),
            );
            return;
        }
        if self.key_sent && !self.screenshot_requested {
            match self.app.print_qa_state() {
                Ok((_, true)) => {
                    let ready = self.preview_ready_at.get_or_insert_with(Instant::now);
                    // Allow the completed preview and printer list to paint.
                    if ready.elapsed() >= Duration::from_millis(200) {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(
                            egui::UserData::default(),
                        ));
                        self.screenshot_requested = true;
                    }
                }
                Ok((_, false)) => self.preview_ready_at = None,
                Err(error) => {
                    self.finish(ctx, Err(error));
                    return;
                }
            }
        }
        ctx.request_repaint_after(Duration::from_millis(30));
    }

    fn on_exit(&mut self, gl: Option<&eframe::glow::Context>) {
        self.app.on_exit(gl);
    }
}

fn save_screenshot(path: &Path, image: &egui::ColorImage) -> Result<ScreenshotReport, String> {
    if image.size[0] == 0 || image.size[1] == 0 {
        return Err("The print UI screenshot was empty.".to_owned());
    }
    let rgba: Vec<u8> = image
        .pixels
        .iter()
        .flat_map(|pixel| pixel.to_srgba_unmultiplied())
        .collect();
    image::save_buffer_with_format(
        path,
        &rgba,
        image.size[0] as u32,
        image.size[1] as u32,
        image::ColorType::Rgba8,
        image::ImageFormat::Png,
    )
    .map_err(|error| format!("Could not save print UI screenshot: {error}"))?;
    Ok(ScreenshotReport {
        width: image.size[0],
        height: image.size[1],
    })
}

fn qa_error(message: impl Into<String>) -> eframe::Error {
    eframe::Error::AppCreation(Box::new(std::io::Error::other(message.into())))
}

/// Usage: --smoke-print document.pdf screenshot.png [width] [height]
pub fn run() -> eframe::Result<()> {
    let args: Vec<_> = std::env::args_os()
        .skip_while(|arg| arg != "--smoke-print")
        .skip(1)
        .collect();
    if !(2..=4).contains(&args.len()) {
        return Err(qa_error(
            "Usage: --smoke-print document.pdf screenshot.png [width] [height]",
        ));
    }
    let source = std::fs::canonicalize(PathBuf::from(&args[0]))
        .map_err(|error| qa_error(format!("Could not open print QA fixture: {error}")))?;
    let destination = PathBuf::from(&args[1]);
    if !source.is_file()
        || destination
            .extension()
            .is_none_or(|extension| !extension.eq_ignore_ascii_case("png"))
    {
        return Err(qa_error(
            "Supply an existing PDF and a .png screenshot destination.",
        ));
    }
    let dimension = |index: usize, fallback: u32| -> Result<u32, eframe::Error> {
        match args.get(index) {
            Some(value) => value
                .to_string_lossy()
                .parse::<u32>()
                .ok()
                .filter(|value| (600..=2400).contains(value))
                .ok_or_else(|| qa_error("Window dimensions must be between 600 and 2400 pixels.")),
            None => Ok(fallback),
        }
    };
    let width = dimension(2, 1280)?;
    let height = dimension(3, if width < 900 { 740 } else { 900 })?;
    if let Some(parent) = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)
            .map_err(|error| qa_error(format!("Could not create screenshot directory: {error}")))?;
    }
    crate::shutdown_smoke::begin_qa();
    crate::diagnostics::start_session();
    let outcome: Outcome = Arc::new(Mutex::new(None));
    let app_outcome = outcome.clone();
    let app_destination = destination.clone();
    let (_tx, incoming) = crossbeam_channel::unbounded();
    eframe::run_native(
        "LawPDF print preview verification",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([width as f32, height as f32])
                .with_min_inner_size([600.0, 600.0]),
            run_and_return: true,
            ..Default::default()
        },
        Box::new(move |cc| {
            Ok(Box::new(PrintSmoke {
                app: crate::app::PdfEditorApp::new(&cc.egui_ctx, vec![source], incoming),
                opened: Instant::now(),
                destination: app_destination,
                outcome: app_outcome,
                key_sent: false,
                key_released: false,
                preview_ready_at: None,
                screenshot_requested: false,
                finished: false,
            }))
        }),
    )?;
    let result = outcome
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take()
        .unwrap_or_else(|| {
            Err("The print QA window closed before a screenshot was captured.".to_owned())
        });
    match result {
        Ok(report) => {
            println!(
                "{}",
                serde_json::json!({
                    "success": true, "shortcut": "Ctrl+P", "preview_ready": true,
                    "screenshot": destination, "image_width": report.width, "image_height": report.height,
                    "window_width": width, "window_height": height, "printer_jobs_submitted": 0,
                })
            );
            crate::diagnostics::finish_session("print_smoke_complete");
            Ok(())
        }
        Err(error) => {
            println!(
                "{}",
                serde_json::json!({"success": false, "error": error, "printer_jobs_submitted": 0})
            );
            crate::diagnostics::finish_session("print_smoke_failed");
            Err(qa_error(error))
        }
    }
}
