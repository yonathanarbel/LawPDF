use super::*;
use crate::printing::{PrintCopy, PrintJob, prepare_copy};
#[cfg(target_os = "linux")]
use crate::printing::{PrintOptions, Printer, page_range_is_valid};

pub(super) struct PrintUi {
    pub busy: bool,
    active_job: Option<PrintJob>,
    tx: Sender<Result<PrintCopy, String>>,
    rx: Receiver<Result<PrintCopy, String>>,
    /// macOS opens AppKit's own print panel, so only the CUPS path needs a
    /// confirmation window of its own.
    #[cfg(target_os = "linux")]
    confirm: Option<PendingPrint>,
}

/// A prepared copy waiting for the user to confirm its destination.
#[cfg(target_os = "linux")]
struct PendingPrint {
    copy: PrintCopy,
    printers: Vec<Printer>,
    options: PrintOptions,
    error: Option<String>,
}

impl Default for PrintUi {
    fn default() -> Self {
        let (tx, rx) = unbounded();
        Self {
            busy: false,
            active_job: None,
            tx,
            rx,
            #[cfg(target_os = "linux")]
            confirm: None,
        }
    }
}

impl PdfEditorApp {
    pub(super) fn request_print(&mut self, ctx: &Context) {
        if self.print_ui.busy {
            return;
        }
        if self.pending_close.is_some() {
            self.status = "Finish closing the document before printing.".to_owned();
            return;
        }
        let Some(document) = &self.document else {
            self.status = "Open a PDF to print.".to_owned();
            return;
        };
        if self.page_rotation_in_flight || self.recovery_ui.is_writing() {
            self.push_error_notice("Wait for the PDF operation to finish before printing.");
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
        // Capture the selected document and edits now. Switching tabs or an
        // autosave completing must not change the requested print job.
        let source = document.path.clone();
        let revision = session.revision.clone();
        let annotations = self.annotations.clone();
        let tx = self.print_ui.tx.clone();
        let ctx = ctx.clone();
        self.print_ui.busy = true;
        self.status = "Preparing print preview…".to_owned();
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(|| {
                crate::document_store::DocumentStore::new()
                    .and_then(|store| prepare_copy(&store, &source, &revision, &annotations))
            })
            .unwrap_or_else(|_| Err("Could not prepare this PDF for printing.".to_owned()));
            let _ = tx.send(result);
            ctx.request_repaint();
        });
    }

    pub(super) fn poll_print(&mut self, ctx: &Context) {
        while let Ok(result) = self.print_ui.rx.try_recv() {
            match result {
                Ok(copy) => self.present_prepared_print(copy),
                Err(error) => {
                    self.print_ui.busy = false;
                    self.push_error_notice(error);
                }
            }
        }
        if let Some(result) = self
            .print_ui
            .active_job
            .as_ref()
            .and_then(PrintJob::take_result)
        {
            self.print_ui.active_job = None;
            self.print_ui.busy = false;
            #[cfg(target_os = "macos")]
            {
                self.status = if result {
                    "Print request completed."
                } else {
                    "Print dialog closed."
                }
                .to_owned();
            }
            #[cfg(target_os = "linux")]
            match result {
                Ok(receipt) => self.status = receipt,
                Err(error) => self.push_error_notice(error),
            }
            ctx.request_repaint();
        }
        // Only repaint while a user-requested operation needs polling.
        if self.print_ui.active_job.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    /// macOS hands the prepared copy straight to AppKit, which asks the user
    /// for a destination in its own panel.
    #[cfg(target_os = "macos")]
    fn present_prepared_print(&mut self, copy: PrintCopy) {
        match PrintJob::start(copy) {
            Ok(job) => self.print_ui.active_job = Some(job),
            Err(error) => {
                self.print_ui.busy = false;
                self.push_error_notice(error);
            }
        }
    }

    /// CUPS spools without asking, so the prepared copy waits in a confirmation
    /// window until the user picks a destination and accepts.
    #[cfg(target_os = "linux")]
    fn present_prepared_print(&mut self, copy: PrintCopy) {
        let printers = copy.printers().to_vec();
        let printer = printers.first().map(|printer| printer.name.clone());
        self.print_ui.confirm = Some(PendingPrint {
            copy,
            printers,
            options: PrintOptions {
                printer,
                ..PrintOptions::default()
            },
            error: None,
        });
        self.status = "Choose a printer to finish.".to_owned();
    }

    #[cfg(target_os = "linux")]
    pub(super) fn draw_print_dialog(&mut self, ctx: &Context) {
        let Some(mut pending) = self.print_ui.confirm.take() else {
            return;
        };
        let mut decision =
            if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                PrintDecision::Cancel
            } else {
                PrintDecision::Waiting
            };

        egui::Window::new("Print")
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .collapsible(false)
            .resizable(false)
            .title_bar(false)
            .frame(
                egui::Frame::window(&ctx.style())
                    .fill(BAR_FILL)
                    .corner_radius(8)
                    .inner_margin(Margin::symmetric(16, 14)),
            )
            .show(ctx, |ui| {
                ui.set_max_width(360.0);
                ui.label(RichText::new("Print this PDF").strong());
                ui.label(
                    RichText::new(format!(
                        "{} will be sent with its current annotations.",
                        pending.copy.title()
                    ))
                    .small()
                    .color(MUTED_INK),
                );
                ui.add_space(8.0);

                let selected = pending
                    .options
                    .printer
                    .clone()
                    .unwrap_or_else(|| "Default printer".to_owned());
                egui::ComboBox::from_label("Printer")
                    .selected_text(selected)
                    .width(220.0)
                    .show_ui(ui, |ui| {
                        for printer in &pending.printers {
                            let label = if printer.is_default {
                                format!("{} (default)", printer.name)
                            } else {
                                printer.name.clone()
                            };
                            let mut chosen =
                                pending.options.printer.as_deref() == Some(printer.name.as_str());
                            if ui.selectable_value(&mut chosen, true, label).clicked() {
                                pending.options.printer = Some(printer.name.clone());
                            }
                        }
                    });

                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label("Copies");
                    ui.add(egui::DragValue::new(&mut pending.options.copies).range(1..=99));
                });
                ui.horizontal(|ui| {
                    ui.label("Pages");
                    ui.add(
                        egui::TextEdit::singleline(&mut pending.options.page_range)
                            .hint_text("All")
                            .desired_width(150.0),
                    );
                });
                ui.label(
                    RichText::new("Leave Pages empty to print everything, or enter 1-4,7.")
                        .small()
                        .color(MUTED_INK),
                );

                if let Some(error) = &pending.error {
                    ui.add_space(4.0);
                    ui.label(RichText::new(error).small().color(egui::Color32::LIGHT_RED));
                }

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Print").clicked() {
                        decision = PrintDecision::Send;
                    }
                    if ui.button("Cancel").clicked() {
                        decision = PrintDecision::Cancel;
                    }
                });
            });

        match decision {
            PrintDecision::Waiting => self.print_ui.confirm = Some(pending),
            PrintDecision::Cancel => {
                self.print_ui.busy = false;
                self.status = "Printing cancelled.".to_owned();
            }
            PrintDecision::Send if !page_range_is_valid(&pending.options.page_range) => {
                // Keep the window and the prepared copy so the user can correct
                // the page range instead of starting over.
                pending.error =
                    Some("Enter pages as numbers and ranges, for example 1-4,7.".to_owned());
                self.print_ui.confirm = Some(pending);
            }
            PrintDecision::Send => {
                let PendingPrint { copy, options, .. } = pending;
                match PrintJob::start_with(copy, &options) {
                    Ok(job) => {
                        self.print_ui.active_job = Some(job);
                        self.status = "Sending to the printer…".to_owned();
                    }
                    Err(error) => {
                        self.print_ui.busy = false;
                        self.push_error_notice(error);
                    }
                }
            }
        }
    }
}

#[cfg(target_os = "linux")]
enum PrintDecision {
    Waiting,
    Send,
    Cancel,
}
