use super::*;
use crate::printing::{PrintCopy, PrintJob, prepare_copy};

pub(super) struct PrintUi {
    pub busy: bool,
    active_job: Option<PrintJob>,
    tx: Sender<Result<PrintCopy, String>>,
    rx: Receiver<Result<PrintCopy, String>>,
}

impl Default for PrintUi {
    fn default() -> Self {
        let (tx, rx) = unbounded();
        Self {
            busy: false,
            active_job: None,
            tx,
            rx,
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
            match result.and_then(PrintJob::start) {
                Ok(job) => self.print_ui.active_job = Some(job),
                Err(error) => {
                    self.print_ui.busy = false;
                    self.push_error_notice(error);
                }
            }
        }
        if let Some(success) = self
            .print_ui
            .active_job
            .as_ref()
            .and_then(PrintJob::take_result)
        {
            self.print_ui.active_job = None;
            self.print_ui.busy = false;
            self.status = if success {
                "Print request completed."
            } else {
                "Print dialog closed."
            }
            .to_owned();
            ctx.request_repaint();
        }
    }
}
