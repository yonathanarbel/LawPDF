use super::*;
use crate::document_store::{DocumentStore, RecoveryItem, RecoveryScan};

fn recovery_job<T>(work: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)).unwrap_or_else(|_| {
        Err(
            "The document task stopped unexpectedly. Your original and recovery files were kept."
                .to_owned(),
        )
    })
}

enum RecoveryEvent {
    Loaded(Result<RecoveryScan, String>),
    Copied {
        source: PathBuf,
        destination: PathBuf,
        recovered: bool,
        result: Result<(), String>,
    },
    Exported {
        destination: PathBuf,
        result: Result<(), String>,
    },
}

pub(super) struct RecoveryUi {
    items: Vec<RecoveryItem>,
    error: Option<String>,
    visible: bool,
    busy: bool,
    writing: bool,
    tx: Sender<RecoveryEvent>,
    rx: Receiver<RecoveryEvent>,
}

impl RecoveryUi {
    pub fn new(ctx: &Context, isolated: bool) -> Self {
        let (tx, rx) = unbounded();
        let mut state = Self {
            items: Vec::new(),
            error: None,
            visible: false,
            busy: false,
            writing: false,
            tx,
            rx,
        };
        if !isolated {
            state.refresh(ctx);
        }
        state
    }

    fn refresh(&mut self, ctx: &Context) {
        if self.busy {
            return;
        }
        self.busy = true;
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = recovery_job(|| DocumentStore::new().and_then(|store| store.pending()));
            let _ = tx.send(RecoveryEvent::Loaded(result));
            ctx.request_repaint();
        });
    }

    pub(super) fn is_writing(&self) -> bool {
        self.writing
    }
}

impl PdfEditorApp {
    pub(super) fn export_png_dialog(&mut self, ctx: &Context) {
        if self.recovery_ui.busy {
            self.status = "Wait for the current export to finish.".to_owned();
            return;
        }
        let Some(document) = &self.document else {
            return;
        };
        let path = document.path.clone();
        let Some(session) = self.annotation_sessions.get(&self.document_epoch) else {
            return;
        };
        if session.recovery_pending || self.page_rotation_in_flight {
            self.status = "Resolve recovery or finish rotating before exporting.".to_owned();
            return;
        }
        let revision = session.revision.clone();
        let annotations = self.annotations.clone();
        let page = self.page_index;
        let name = default_output_name(&path, &format!("page-{}", page + 1), "png");
        let Some(destination) = self.pick_save_path("Export page image", &name, "PNG", &["png"])
        else {
            return;
        };
        self.recovery_ui.busy = true;
        self.recovery_ui.writing = true;
        self.status = "Exporting page image…".to_owned();
        let tx = self.recovery_ui.tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = recovery_job(|| {
                let temporary = tempfile::Builder::new()
                    .prefix("lawpdf-page-export-")
                    .tempdir()
                    .map_err(|error| error.to_string())?;
                let copy = temporary.path().join("annotated.pdf");
                DocumentStore::new()?.export_annotations(&path, &revision, &annotations, &copy)?;
                let engine = PdfEngine::new().map_err(|error| error.to_string())?;
                let result = engine
                    .export_page_png(&copy, page, &destination, 2.0)
                    .map_err(|error| format!("{error:#}"));
                engine.close_document(&copy);
                result
            });
            let _ = tx.send(RecoveryEvent::Exported {
                destination,
                result,
            });
            ctx.request_repaint();
        });
    }

    pub(super) fn save_as_dialog(&mut self, ctx: &Context) {
        if self.recovery_ui.busy || self.page_rotation_in_flight {
            return;
        }
        let Some(document) = &self.document else {
            return;
        };
        let Some(session) = self.annotation_sessions.get(&self.document_epoch) else {
            return;
        };
        if session.recovery_pending {
            self.recovery_ui.visible = true;
            self.recovery_ui.refresh(ctx);
            return;
        }
        let source = document.path.clone();
        let revision = session.revision.clone();
        let annotations = self.annotations.clone();
        let file_name = default_output_name(&source, "edited", "pdf");
        let Some(destination) =
            self.pick_save_path("Save an annotated PDF copy", &file_name, "PDF", &["pdf"])
        else {
            return;
        };
        let tx = self.recovery_ui.tx.clone();
        let ctx = ctx.clone();
        self.recovery_ui.busy = true;
        self.recovery_ui.writing = true;
        self.status = "Saving PDF copy…".to_owned();
        std::thread::spawn(move || {
            let result = recovery_job(|| {
                DocumentStore::new().and_then(|store| {
                    store.export_annotations(&source, &revision, &annotations, &destination)
                })
            });
            let _ = tx.send(RecoveryEvent::Copied {
                source,
                destination,
                recovered: false,
                result,
            });
            ctx.request_repaint();
        });
    }

    pub(super) fn poll_recovery(&mut self, ctx: &Context) {
        while let Ok(event) = self.recovery_ui.rx.try_recv() {
            self.recovery_ui.busy = false;
            self.recovery_ui.writing = false;
            match event {
                RecoveryEvent::Exported {
                    destination,
                    result,
                } => match result {
                    Ok(()) => self.status = format!("Exported {}", destination.display()),
                    Err(error) => self.push_error_notice(error),
                },
                RecoveryEvent::Loaded(result) => match result {
                    Ok(mut scan) => {
                        // A refresh must never offer to discard the journal of
                        // an edit still active in this process.
                        scan.items.retain(|item| {
                            !self.tabs.iter().any(|tab| {
                                tab.document.path == item.record.original_path
                                    && self
                                        .annotation_sessions
                                        .get(&tab.document_epoch)
                                        .is_some_and(|session| !session.recovery_pending)
                            })
                        });
                        self.recovery_ui.error =
                            (!scan.warnings.is_empty()).then(|| scan.warnings.join("\n"));
                        self.recovery_ui.visible =
                            !scan.items.is_empty() || self.recovery_ui.error.is_some();
                        self.recovery_ui.items = scan.items;
                    }
                    Err(error) => {
                        self.recovery_ui.error = Some(error);
                        self.recovery_ui.visible = true;
                    }
                },
                RecoveryEvent::Copied {
                    source,
                    destination,
                    recovered,
                    result,
                } => match result {
                    Ok(()) => {
                        if recovered {
                            // The copied journal is no longer pending. The worker
                            // removes it only after the new PDF is durable.
                            for tab in &self.tabs {
                                if tab.document.path == source
                                    && let Some(session) =
                                        self.annotation_sessions.get_mut(&tab.document_epoch)
                                {
                                    session.recovery_pending = false;
                                }
                            }
                            self.recovery_ui
                                .items
                                .retain(|item| item.record.original_path != source);
                            self.recovery_ui.visible = !self.recovery_ui.items.is_empty();
                            self.load_document_with_options(
                                destination.clone(),
                                ctx,
                                true,
                                false,
                                true,
                            );
                        }
                        self.status = format!(
                            "Saved PDF copy to {}. The original was preserved.",
                            destination.display()
                        );
                    }
                    Err(error) => {
                        self.recovery_ui.error = Some(error.clone());
                        self.push_error_notice(error);
                    }
                },
            }
        }
    }

    pub(super) fn draw_recovery(&mut self, ctx: &Context) {
        if !self.recovery_ui.visible {
            return;
        }
        let mut recover = None;
        let mut discard = None;
        egui::Modal::new(egui::Id::new("annotation_recovery")).show(ctx, |ui| {
            ui.heading("Recover unsaved annotations");
            ui.label("LawPDF kept your edits and the PDF they belong to. Save a recovered copy to preserve both versions.");
            ui.label("Recovered copies may remove password protection or invalidate a digital signature. Your original stays unchanged.");
            if let Some(error) = &self.recovery_ui.error { ui.colored_label(Color32::DARK_RED, error); }
            if self.recovery_ui.busy { ui.spinner(); }
            egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                for (index, item) in self.recovery_ui.items.iter().enumerate() {
                    ui.group(|ui| {
                        ui.label(item.record.original_path.file_name().unwrap_or_default().to_string_lossy());
                        ui.small(format!("{} annotations", item.record.annotations.len()));
                        ui.add_enabled_ui(!self.recovery_ui.busy, |ui| {
                            if ui.button("Save recovered copy…").clicked() { recover = Some(index); }
                            if ui.button("Discard these recovered edits").clicked() { discard = Some(index); }
                        });
                    });
                }
            });
            if ui.button("Keep for later").clicked() { self.recovery_ui.visible = false; }
        });
        if let Some(index) = recover {
            let item = self.recovery_ui.items[index].clone();
            let name = default_output_name(&item.record.original_path, "recovered", "pdf");
            if let Some(destination) =
                self.pick_save_path("Save recovered PDF", &name, "PDF", &["pdf"])
            {
                self.recovery_ui.busy = true;
                self.recovery_ui.writing = true;
                let tx = self.recovery_ui.tx.clone();
                let ctx = ctx.clone();
                std::thread::spawn(move || {
                    let source = item.record.original_path.clone();
                    let result = recovery_job(|| {
                        DocumentStore::new().and_then(|store| {
                            store.export_recovery(&item, &destination)?;
                            store.discard_recovery(&item)
                        })
                    });
                    let _ = tx.send(RecoveryEvent::Copied {
                        source,
                        destination,
                        recovered: true,
                        result,
                    });
                    ctx.request_repaint();
                });
            }
        }
        if let Some(index) = discard {
            let item = self.recovery_ui.items[index].clone();
            let source = item.record.original_path.clone();
            match DocumentStore::new().and_then(|store| store.discard_recovery(&item)) {
                Ok(()) => {
                    for tab in &self.tabs {
                        if tab.document.path == source
                            && let Some(session) =
                                self.annotation_sessions.get_mut(&tab.document_epoch)
                        {
                            session.recovery_pending = false;
                        }
                    }
                    self.recovery_ui.items.remove(index);
                    self.recovery_ui.visible = !self.recovery_ui.items.is_empty();
                }
                Err(error) => self.recovery_ui.error = Some(error),
            }
        }
    }

    pub(super) fn recovery_is_busy(&self) -> bool {
        self.recovery_ui.busy
    }

    pub(super) fn open_recovery_dialog(&mut self, ctx: &Context) {
        self.recovery_ui.error = None;
        self.recovery_ui.visible = true;
        self.recovery_ui.refresh(ctx);
    }
}
