use super::*;
use crate::printing::{
    self, PageParity, PaperSize, PreparedPrint, PrintOptions, PrintSource, ScaleMode,
};
use crate::printing_windows::{self, Printer, SpoolOptions};
use std::sync::atomic::{AtomicU8, Ordering};

const ACCENT: Color32 = Color32::from_rgb(30, 96, 91);
const SUBTLE: Color32 = Color32::from_rgb(113, 116, 111);

enum PrintEvent {
    Printers(u64, Result<Vec<Printer>, String>),
    Preview {
        session: u64,
        generation: u64,
        annotations: bool,
        result: Result<(Arc<PreparedPrint>, image::RgbaImage), String>,
    },
    Progress(usize, usize),
    Spooling,
    Finished(Result<bool, String>),
}

pub(super) struct PrintUi {
    pub(super) open: bool,
    source: Option<PrintSource>,
    options: PrintOptions,
    current_page: usize,
    printer: String,
    printers: Vec<Printer>,
    discovering: bool,
    printer_error: Option<String>,
    duplex: String,
    session: u64,
    generation: u64,
    preview_sheet: usize,
    preview: Option<TextureHandle>,
    prepared: Option<(bool, Arc<PreparedPrint>)>,
    preview_busy: bool,
    preview_due: Option<Instant>,
    error: Option<String>,
    message: Option<String>,
    pub(super) busy: bool,
    spooling: bool,
    progress: (usize, usize),
    // 0 = preparing, 1 = canceled, 2 = handed to the spooler.
    cancel: Arc<AtomicU8>,
    tx: Sender<PrintEvent>,
    rx: Receiver<PrintEvent>,
}

impl PrintUi {
    pub(super) fn new() -> Self {
        let (tx, rx) = unbounded();
        Self {
            open: false,
            source: None,
            options: PrintOptions::default(),
            current_page: 0,
            printer: String::new(),
            printers: Vec::new(),
            discovering: false,
            printer_error: None,
            duplex: "simplex".into(),
            session: 0,
            generation: 0,
            preview_sheet: 0,
            preview: None,
            prepared: None,
            preview_busy: false,
            preview_due: None,
            error: None,
            message: None,
            busy: false,
            spooling: false,
            progress: (0, 0),
            cancel: Arc::new(AtomicU8::new(0)),
            tx,
            rx,
        }
    }

    fn invalidate(&mut self) {
        self.generation += 1;
        self.preview = None;
        self.error = None;
        self.message = None;
        self.preview_due = Some(Instant::now() + Duration::from_millis(180));
    }

    pub(super) fn close(&mut self) {
        self.open = false;
        self.generation += 1;
        self.preview = None;
        if let Some(prepared) = self.prepared.take() {
            std::thread::spawn(move || drop(prepared));
        }
        self.source = None;
        self.preview_due = None;
    }

    fn discover(&mut self, ctx: &Context) {
        self.discovering = true;
        self.printer_error = None;
        let tx = self.tx.clone();
        let session = self.session;
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(PrintEvent::Printers(session, printing_windows::printers()));
            ctx.request_repaint();
        });
    }

    fn request_preview(&mut self, ctx: &Context) {
        if self.preview_busy || self.preview_due.is_none_or(|due| due > Instant::now()) {
            return;
        }
        self.preview_due = None;
        let Some(source) = self.source.clone() else {
            return;
        };
        if let Err(error) = self.options.selected_pages(source.document.page_count) {
            self.error = Some(error);
            return;
        }
        self.preview_busy = true;
        let options = self.options.clone();
        let prepared = self
            .prepared
            .as_ref()
            .filter(|(annotations, _)| *annotations == options.include_annotations)
            .map(|(_, prepared)| prepared.clone());
        let (session, generation, sheet) = (self.session, self.generation, self.preview_sheet);
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = (|| {
                let prepared = match prepared {
                    Some(prepared) => prepared,
                    None => Arc::new(printing::prepare(&source, options.include_annotations)?),
                };
                let image = printing::render_sheet(&prepared, &options, sheet, 72.0)?;
                Ok((prepared, image))
            })();
            let _ = tx.send(PrintEvent::Preview {
                session,
                generation,
                annotations: options.include_annotations,
                result,
            });
            ctx.request_repaint();
        });
    }

    fn start_print(&mut self, system_dialog: bool, ctx: &Context) {
        if self.busy || self.preview.is_none() || self.preview_busy || self.preview_due.is_some() {
            return;
        }
        let Some((_, prepared)) = &self.prepared else {
            return;
        };
        let prepared = prepared.clone();
        let Some(source) = &self.source else {
            return;
        };
        let title = source.document.title.clone();
        let options = self.options.clone();
        let spool = SpoolOptions {
            printer: self.printer.clone(),
            copies: options.copies,
            collate: options.collate,
            duplex: self.duplex.clone(),
            paper: paper_label(options.paper).to_owned(),
            landscape: options.landscape,
            system_dialog,
        };
        self.busy = true;
        self.spooling = false;
        self.error = None;
        self.message = None;
        self.cancel = Arc::new(AtomicU8::new(0));
        self.progress = (
            0,
            options.sheet_count(source.document.page_count).unwrap_or(0),
        );
        let cancel = self.cancel.clone();
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = (|| {
                let folder = tempfile::Builder::new()
                    .prefix("lawpdf-print-sheets-")
                    .tempdir()
                    .map_err(|e| format!("Could not prepare print files: {e}"))?;
                let count = options.sheet_count(prepared.document.page_count)?;
                let mut files = Vec::with_capacity(count);
                for sheet in 0..count {
                    if cancel.load(Ordering::Acquire) == 1 {
                        return Ok(false);
                    }
                    let image = printing::render_sheet(&prepared, &options, sheet, 300.0)?;
                    let path = folder.path().join(format!("sheet-{sheet:06}.png"));
                    image
                        .save(&path)
                        .map_err(|e| format!("Could not prepare sheet {}: {e}", sheet + 1))?;
                    files.push(path);
                    let _ = tx.send(PrintEvent::Progress(sheet + 1, count));
                    ctx.request_repaint();
                }
                if cancel
                    .compare_exchange(0, 2, Ordering::AcqRel, Ordering::Acquire)
                    .is_err()
                {
                    return Ok(false);
                }
                let _ = tx.send(PrintEvent::Spooling);
                ctx.request_repaint();
                printing_windows::print_pages(&title, &files, &spool)
            })();
            let _ = tx.send(PrintEvent::Finished(result));
            ctx.request_repaint();
        });
    }
}

impl Default for PrintUi {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for PrintUi {
    fn drop(&mut self) {
        let _ = self
            .cancel
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire);
    }
}

impl PdfEditorApp {
    #[cfg(feature = "devtools")]
    pub(crate) fn print_qa_state(&self) -> Result<(bool, bool), String> {
        if let Some(error) = &self.print_ui.error {
            return Err(error.clone());
        }
        let loaded =
            self.document.is_some() && self.annotation_sessions.contains_key(&self.document_epoch);
        let ready = self.print_ui.open
            && self.print_ui.preview.is_some()
            && !self.print_ui.preview_busy
            && self.print_ui.preview_due.is_none()
            && !self.print_ui.discovering;
        Ok((loaded, ready))
    }

    pub(super) fn request_print(&mut self, ctx: &Context) {
        if self.print_ui.open {
            return;
        }
        if self.pending_close.is_some() || self.recovery_ui.is_writing() {
            self.push_error_notice("Finish the pending PDF operation before printing.");
            return;
        }
        let Some(document) = self.document.clone() else {
            self.status = "Open a PDF to print.".into();
            return;
        };
        if self.page_rotation_in_flight {
            self.push_error_notice("Wait for page rotation to finish before printing.");
            return;
        }
        let Some(session) = self.annotation_sessions.get(&self.document_epoch) else {
            self.push_error_notice(
                "The PDF is still opening. Try printing again when it is ready.",
            );
            return;
        };
        if session.recovery_pending {
            self.push_error_notice(
                "Recover or discard the earlier edits before printing this PDF.",
            );
            return;
        }
        let revision = Some(session.revision.clone());
        let mut annotations = self.annotations.clone();
        crate::comments::normalize_comment_authors(
            &mut annotations,
            &self.settings.comment_author_id,
            &self.settings.comment_author_name,
        );
        let print = &mut self.print_ui;
        print.session += 1;
        print.open = true;
        print.source = Some(PrintSource {
            document,
            annotations,
            revision,
        });
        print.current_page = self.page_index;
        print.options.range.clear();
        print.options.parity = PageParity::All;
        print.options.reverse = false;
        print.preview_sheet = 0;
        print.prepared = None;
        print.preview_busy = false;
        print.invalidate();
        print.discover(ctx);
        ctx.request_repaint();
    }

    pub(super) fn poll_print(&mut self, ctx: &Context) {
        let print = &mut self.print_ui;
        while let Ok(event) = print.rx.try_recv() {
            match event {
                PrintEvent::Printers(session, result) if session == print.session => {
                    print.discovering = false;
                    match result {
                        Ok(printers) => {
                            if !printers.iter().any(|p| p.name == print.printer) {
                                print.printer = printers
                                    .iter()
                                    .find(|p| p.is_default)
                                    .or(printers.first())
                                    .map(|p| p.name.clone())
                                    .unwrap_or_default();
                            }
                            print.printers = printers;
                            if print.printers.is_empty() {
                                print.printer_error = Some("No printers found. Add a printer in Windows Settings, then refresh.".into());
                            }
                        }
                        Err(error) => print.printer_error = Some(error),
                    }
                }
                PrintEvent::Preview {
                    session,
                    generation,
                    annotations,
                    result,
                } if session == print.session => {
                    print.preview_busy = false;
                    if print.open {
                        match result {
                            Ok((prepared, image)) => {
                                print.prepared = Some((annotations, prepared));
                                if generation == print.generation {
                                    let size = [image.width() as usize, image.height() as usize];
                                    print.preview = Some(ctx.load_texture(
                                        format!("print-preview-{session}-{generation}"),
                                        egui::ColorImage::from_rgba_unmultiplied(
                                            size,
                                            image.as_raw(),
                                        ),
                                        TextureOptions::LINEAR,
                                    ));
                                }
                            }
                            Err(error) if generation == print.generation => {
                                print.error = Some(error)
                            }
                            Err(_) => {}
                        }
                    }
                }
                PrintEvent::Progress(done, total) => print.progress = (done, total),
                PrintEvent::Spooling => print.spooling = true,
                PrintEvent::Finished(result) => {
                    print.busy = false;
                    print.spooling = false;
                    match result {
                        Ok(true) => {
                            print.message = Some("Sent to the printer. You can follow the job in the Windows print queue.".into());
                            self.status = "Print job sent to Windows.".into();
                        }
                        Ok(false) => print.message = Some("Printing canceled.".into()),
                        Err(error) => print.error = Some(error),
                    }
                }
                _ => {}
            }
        }
        if print.open && !print.busy {
            print.request_preview(ctx);
        }
        if print.open
            && (print.preview_busy
                || print.preview_due.is_some()
                || print.discovering
                || print.busy)
        {
            ctx.request_repaint_after(Duration::from_millis(60));
        }
    }

    pub(super) fn draw_print_dialog(&mut self, ctx: &Context) {
        if !self.print_ui.open {
            return;
        }
        let reduce_motion = self.settings.reduce_motion;
        let palette = self.app_palette();
        let print = &mut self.print_ui;
        let Some(source) = print.source.as_ref() else {
            return;
        };
        let title = source.document.title.clone();
        let page_count = source.document.page_count;
        let screen = ctx.content_rect();
        let width = (screen.width() - 48.0).clamp(280.0, 980.0);
        let body_height = (screen.height() - 225.0).clamp(180.0, 600.0);
        let before = print.options.clone();
        let before_sheet = print.preview_sheet;
        let mut close = false;
        let mut submit = None;
        let response = egui::Modal::new(egui::Id::new("lawpdf-print"))
            .backdrop_color(Color32::from_black_alpha(125))
            .frame(egui::Frame::NONE.fill(palette.bar).corner_radius(16)
                .inner_margin(22).stroke(Stroke::new(1.0_f32, palette.rule))
                .shadow(Shadow { offset: [0, 12], blur: 40, spread: 1, color: Color32::from_black_alpha(60) }))
            .show(ctx, |ui| {
                ui.set_width(width);
                let highlight = if ui.visuals().dark_mode { Color32::from_rgb(55, 84, 78) } else { Color32::from_rgb(216, 235, 229) };
                ui.visuals_mut().selection.bg_fill = highlight;
                ui.visuals_mut().selection.stroke = Stroke::new(1.0_f32, palette.accent);
                ui.spacing_mut().item_spacing = Vec2::new(10.0, 9.0);
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(44.0), Sense::hover());
                    paint_printer(ui.painter(), rect, palette.accent);
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Print document").size(25.0).strong().color(palette.ink));
                        ui.add(egui::Label::new(RichText::new(&title).color(palette.muted)).truncate());
                    });
                    ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                        if ui.add_enabled(!print.busy, egui::Button::new("×").frame(false))
                            .on_hover_text("Close print window · Esc").clicked() { close = true; }
                    });
                });
                ui.add_space(4.0);
                ui.separator();
                egui::ScrollArea::vertical().id_salt("print-body")
                    .max_height(body_height).auto_shrink([false, false]).show(ui, |ui| {
                    if width >= 720.0 {
                        ui.horizontal_top(|ui| {
                            let preview_width = width - 335.0;
                            ui.allocate_ui_with_layout(Vec2::new(preview_width, body_height - 10.0), egui::Layout::top_down(Align::Min), |ui| {
                                preview_panel(ui, print, preview_width, body_height - 65.0, page_count, reduce_motion);
                            });
                            ui.add_space(8.0);
                            ui.allocate_ui_with_layout(Vec2::new(305.0, body_height - 10.0), egui::Layout::top_down(Align::Min), |ui| {
                                ui.add_enabled_ui(!print.busy, |ui| controls(ui, print, page_count, ctx));
                            });
                        });
                    } else {
                        preview_panel(ui, print, width, 220.0, page_count, reduce_motion);
                        ui.add_space(10.0);
                        ui.add_enabled_ui(!print.busy, |ui| controls(ui, print, page_count, ctx));
                    }
                });
                ui.separator();
                if let Some(error) = &print.error {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
                if let Some(message) = &print.message {
                    ui.colored_label(palette.accent, message);
                }
                if print.busy {
                    let (done, total) = print.progress;
                    let label = if print.spooling { "Waiting for Windows / printer…" .to_owned() }
                        else { format!("Preparing sheet {done} of {total}…") };
                    ui.add(egui::ProgressBar::new(done as f32 / total.max(1) as f32).text(label));
                }
                ui.horizontal(|ui| {
                    let selected = print.options.selected_pages(page_count);
                    if let Ok(pages) = &selected {
                        let sides = pages.len().div_ceil(print.options.pages_per_sheet);
                        let sheets = if print.duplex == "simplex" { sides } else { sides.div_ceil(2) };
                        ui.label(RichText::new(format!("{} pages · {} sheets × {} copies",
                            pages.len(), sheets, print.options.copies)).small().color(palette.muted));
                    }
                    ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                        let ready = !print.busy && !print.discovering
                            && print.preview.is_some() && !print.preview_busy && print.preview_due.is_none()
                            && selected.is_ok() && cfg!(target_os = "windows");
                        if ui.add_enabled(ready && !print.printer.is_empty(),
                            egui::Button::new(RichText::new("Print").color(Color32::WHITE).strong())
                                .fill(ACCENT).corner_radius(7).min_size(Vec2::new(100.0, 36.0)))
                            .clicked() { submit = Some(false); }
                        if print.busy && print.cancel.load(Ordering::Acquire) == 0 {
                            if ui.button("Cancel preparation").clicked() {
                                let _ = print.cancel.compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire);
                            }
                        } else if ui.add_enabled(!print.busy, egui::Button::new("Cancel")).clicked() {
                            close = true;
                        }
                        if ui.add_enabled(ready, egui::Button::new("System dialog…").frame(false))
                            .on_hover_text("Printer properties, trays, print quality, and other device settings")
                            .clicked() { submit = Some(true); }
                    });
                });
            });
        if before != print.options {
            print.preview_sheet = 0;
            print.invalidate();
        } else if before_sheet != print.preview_sheet {
            print.invalidate();
        }
        if (close || response.should_close()) && !print.busy {
            print.close();
        } else if let Some(system_dialog) = submit {
            print.start_print(system_dialog, ctx);
        }
    }
}

fn section(ui: &mut egui::Ui, number: &str, title: &str) {
    ui.add_space(5.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(number).size(10.0).color(theme::for_ui(ui).accent).strong());
        ui.label(RichText::new(title).size(16.0).strong().color(theme::for_ui(ui).ink));
    });
}

fn controls(ui: &mut egui::Ui, print: &mut PrintUi, page_count: usize, ctx: &Context) {
    ui.set_min_width(280.0);
    section(ui, "01", "Destination");
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt("print-printer")
            .width(230.0)
            .truncate()
            .selected_text(if print.discovering {
                "Finding printers…"
            } else if print.printer.is_empty() {
                "Choose printer"
            } else {
                &print.printer
            })
            .show_ui(ui, |ui| {
                for printer in &print.printers {
                    ui.selectable_value(&mut print.printer, printer.name.clone(), &printer.name);
                }
            });
        if ui
            .add_enabled(!print.discovering, egui::Button::new("↻"))
            .on_hover_text("Refresh printers")
            .clicked()
        {
            print.discover(ctx);
        }
    });
    if let Some(error) = &print.printer_error {
        ui.label(
            RichText::new(error)
                .small()
                .color(ui.visuals().error_fg_color),
        );
    }
    ui.horizontal(|ui| {
        ui.label("Copies");
        ui.add(egui::DragValue::new(&mut print.options.copies).range(1..=999));
        ui.checkbox(&mut print.options.collate, "Collate");
    });
    ui.separator();
    section(ui, "02", "Pages");
    ui.horizontal(|ui| {
        if ui
            .selectable_label(print.options.range.is_empty(), "All")
            .clicked()
        {
            print.options.range.clear();
        }
        if ui
            .selectable_label(
                print.options.range == (print.current_page + 1).to_string(),
                "Current",
            )
            .clicked()
        {
            print.options.range = (print.current_page + 1).to_string();
        }
        ui.label(
            RichText::new(format!("{page_count} total"))
                .small()
                .color(theme::for_ui(ui).muted),
        );
    });
    ui.add(
        egui::TextEdit::singleline(&mut print.options.range)
            .desired_width(f32::INFINITY)
            .hint_text("All pages, or e.g. 1–3, 5, 8–12"),
    );
    if let Err(error) = print.options.selected_pages(page_count) {
        ui.label(
            RichText::new(error)
                .small()
                .color(ui.visuals().error_fg_color),
        );
    }
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt("print-parity")
            .selected_text(match print.options.parity {
                PageParity::All => "All selected pages",
                PageParity::Odd => "Odd pages only",
                PageParity::Even => "Even pages only",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut print.options.parity,
                    PageParity::All,
                    "All selected pages",
                );
                ui.selectable_value(&mut print.options.parity, PageParity::Odd, "Odd pages only");
                ui.selectable_value(
                    &mut print.options.parity,
                    PageParity::Even,
                    "Even pages only",
                );
            });
        ui.checkbox(&mut print.options.reverse, "Reverse");
    });
    ui.separator();
    section(ui, "03", "Paper & layout");
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt("print-paper")
            .width(125.0)
            .selected_text(paper_label(print.options.paper))
            .show_ui(ui, |ui| {
                for paper in [
                    PaperSize::Letter,
                    PaperSize::Legal,
                    PaperSize::A4,
                    PaperSize::A3,
                ] {
                    ui.selectable_value(&mut print.options.paper, paper, paper_label(paper));
                }
            });
        egui::ComboBox::from_id_salt("print-orientation")
            .width(125.0)
            .selected_text(if print.options.landscape {
                "Landscape"
            } else {
                "Portrait"
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut print.options.landscape, false, "Portrait");
                ui.selectable_value(&mut print.options.landscape, true, "Landscape");
            });
    });
    egui::ComboBox::from_id_salt("print-duplex")
        .width(270.0)
        .selected_text(match print.duplex.as_str() {
            "long-edge" => "Two-sided · long edge",
            "short-edge" => "Two-sided · short edge",
            _ => "One-sided",
        })
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut print.duplex, "simplex".into(), "One-sided");
            ui.selectable_value(
                &mut print.duplex,
                "long-edge".into(),
                "Two-sided · long edge",
            );
            ui.selectable_value(
                &mut print.duplex,
                "short-edge".into(),
                "Two-sided · short edge",
            );
        });
    ui.horizontal(|ui| {
        ui.label("Pages per sheet");
        for count in [1, 2, 4] {
            ui.selectable_value(&mut print.options.pages_per_sheet, count, count.to_string());
        }
    });
    ui.horizontal(|ui| {
        ui.label("Scale");
        egui::ComboBox::from_id_salt("print-scale")
            .width(195.0)
            .selected_text(match print.options.scale {
                ScaleMode::Fit => "Fit inside margins",
                ScaleMode::Shrink => "Shrink oversized pages",
                ScaleMode::Actual => "Actual size (100%)",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut print.options.scale,
                    ScaleMode::Fit,
                    "Fit inside margins",
                );
                ui.selectable_value(
                    &mut print.options.scale,
                    ScaleMode::Shrink,
                    "Shrink oversized pages",
                );
                ui.selectable_value(
                    &mut print.options.scale,
                    ScaleMode::Actual,
                    "Actual size (100%)",
                );
            });
    });
    ui.horizontal(|ui| {
        ui.label("Margins");
        ui.add(
            egui::DragValue::new(&mut print.options.margin_mm)
                .range(0.0..=30.0)
                .speed(0.5)
                .suffix(" mm"),
        );
    });
    if print.options.scale == ScaleMode::Actual {
        ui.label(
            RichText::new("Content beyond the selected margins is cropped.")
                .small()
                .color(theme::for_ui(ui).muted),
        );
    }
    ui.separator();
    section(ui, "04", "Appearance");
    ui.checkbox(&mut print.options.grayscale, "Black & white");
    ui.checkbox(
        &mut print.options.include_annotations,
        "Include annotations",
    );
    ui.label(RichText::new("Prints the original PDF layout, including filled forms. Preview includes your current edits.").small().color(theme::for_ui(ui).muted));
    ui.label(
        RichText::new("300 DPI · Printer margins may trim edge content.")
            .small()
            .color(theme::for_ui(ui).muted),
    );
}

fn preview_panel(
    ui: &mut egui::Ui,
    print: &mut PrintUi,
    width: f32,
    height: f32,
    page_count: usize,
    reduce_motion: bool,
) {
    let sheet_count = print.options.sheet_count(page_count).unwrap_or(0);
    let preview_background = if ui.visuals().dark_mode { theme::for_ui(ui).panel } else { Color32::from_rgb(228, 230, 225) };
    let preview_label = if ui.visuals().dark_mode { theme::for_ui(ui).muted } else { SUBTLE };
    egui::Frame::NONE
        .fill(preview_background)
        .corner_radius(10)
        .inner_margin(16)
        .show(ui, |ui| {
            let size = Vec2::new((width - 34.0).max(120.0), height.max(140.0));
            let (canvas, _) = ui.allocate_exact_size(size, Sense::hover());
            if let Some(texture) = &print.preview {
                let image_size = texture.size_vec2();
                let scale = ((size.x - 26.0) / image_size.x).min((size.y - 26.0) / image_size.y);
                let paper = Rect::from_center_size(canvas.center(), image_size * scale);
                ui.painter().rect_filled(
                    paper.translate(Vec2::new(3.0, 5.0)).expand(2.0),
                    2,
                    Color32::from_black_alpha(22),
                );
                ui.painter().rect_filled(paper, 0, Color32::WHITE);
                ui.painter().image(
                    texture.id(),
                    paper,
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
            } else {
                let label = if print.error.is_some() {
                    "Preview unavailable"
                } else {
                    "Preparing your preview…"
                };
                ui.painter().text(
                    canvas.center(),
                    Align2::CENTER_CENTER,
                    label,
                    FontId::proportional(16.0),
                    preview_label,
                );
            }
        });
    ui.horizontal(|ui| {
        ui.add_enabled_ui(!print.busy, |ui| {
            if ui
                .add_enabled(print.preview_sheet > 0, egui::Button::new("‹"))
                .on_hover_text("Previous sheet")
                .clicked()
            {
                print.preview_sheet -= 1;
            }
            ui.label(
                RichText::new(format!(
                    "Sheet {} of {}",
                    if sheet_count == 0 {
                        0
                    } else {
                        print.preview_sheet + 1
                    },
                    sheet_count
                ))
                .color(theme::for_ui(ui).muted),
            );
            if ui
                .add_enabled(
                    print.preview_sheet + 1 < sheet_count,
                    egui::Button::new("›"),
                )
                .on_hover_text("Next sheet")
                .clicked()
            {
                print.preview_sheet += 1;
            }
        });
        if print.preview_busy || print.preview_due.is_some() {
            motion::busy(ui, reduce_motion);
        }
    });
}

fn paper_label(paper: PaperSize) -> &'static str {
    match paper {
        PaperSize::Letter => "Letter",
        PaperSize::Legal => "Legal",
        PaperSize::A4 => "A4",
        PaperSize::A3 => "A3",
    }
}

fn paint_printer(painter: &egui::Painter, rect: Rect, color: Color32) {
    painter.rect_filled(rect, 10, Color32::from_rgb(223, 236, 230));
    let c = rect.center();
    let stroke = Stroke::new(1.6_f32, color);
    painter.rect_stroke(
        Rect::from_center_size(c, Vec2::new(25.0, 14.0)),
        3,
        stroke,
        egui::StrokeKind::Inside,
    );
    painter.rect_filled(
        Rect::from_center_size(c + Vec2::new(0.0, -10.0), Vec2::new(17.0, 10.0)),
        1,
        Color32::WHITE,
    );
    painter.rect_stroke(
        Rect::from_center_size(c + Vec2::new(0.0, -10.0), Vec2::new(17.0, 10.0)),
        1,
        stroke,
        egui::StrokeKind::Inside,
    );
    painter.rect_filled(
        Rect::from_center_size(c + Vec2::new(0.0, 10.0), Vec2::new(17.0, 12.0)),
        1,
        Color32::WHITE,
    );
    painter.rect_stroke(
        Rect::from_center_size(c + Vec2::new(0.0, 10.0), Vec2::new(17.0, 12.0)),
        1,
        stroke,
        egui::StrokeKind::Inside,
    );
    painter.circle_filled(c + Vec2::new(8.0, -1.0), 1.4, color);
}
