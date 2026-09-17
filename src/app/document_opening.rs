use super::*;
use crate::render_worker::OpenedDocument;

pub(super) struct OpenOptions {
    activate: bool,
    prefetch: bool,
}

impl PdfEditorApp {
    pub(super) fn load_document_with_options(
        &mut self,
        path: PathBuf,
        ctx: &Context,
        activate: bool,
        _render_first_page: bool,
        prefetch: bool,
    ) -> bool {
        let path = std::fs::canonicalize(&path).unwrap_or(path);
        if let Some(tab_index) = self.tab_index_for_path(&path) {
            if activate {
                self.switch_to_tab(tab_index, ctx);
            }
            return true;
        }
        if let Some(options) = self.pending_document_opens.get_mut(&path) {
            options.activate |= activate;
            return true;
        }
        if self.pending_document_opens.len() >= 8 {
            self.queued_open_paths.push_front(path);
            ctx.request_repaint_after(RENDER_POLL_INTERVAL);
            return true;
        }
        let request = RenderRequest::OpenDocument {
            path: path.clone(),
            optimize_large_documents: self.settings.optimize_large_documents,
        };
        match self.render_tx.send(request) {
            Ok(()) => {
                self.pending_document_opens
                    .insert(path, OpenOptions { activate, prefetch });
                self.status = "Opening PDF…".to_owned();
                ctx.request_repaint_after(RENDER_POLL_INTERVAL);
                true
            }
            Err(error) if error.is_full() => {
                self.queued_open_paths.push_front(path);
                ctx.request_repaint_after(RENDER_POLL_INTERVAL);
                true
            }
            Err(_) => {
                self.push_error_notice(
                    "The PDF worker is unavailable. Close and reopen LawPDF to restart it.",
                );
                false
            }
        }
    }

    pub(super) fn finish_document_open(
        &mut self,
        path: PathBuf,
        result: Result<OpenedDocument, String>,
        ctx: &Context,
    ) {
        let Some(options) = self.pending_document_opens.remove(&path) else {
            return;
        };
        let opened = match result {
            Ok(opened) => opened,
            Err(error) => {
                self.open_into_sbs = false;
                self.startup_error = Some(error.clone());
                self.push_error_notice(error);
                return;
            }
        };
        let tab = match self.tab_for_new_document(opened) {
            Ok(tab) => tab,
            Err(error) => {
                self.open_into_sbs = false;
                self.push_error_notice(error);
                return;
            }
        };
        let activate = options.activate || self.active_tab.is_none();
        let first_document = self.tabs.is_empty();
        if activate {
            self.save_active_tab_state();
        }
        self.startup_error = None;
        self.tabs.push(tab);
        self.render_tx.set_live_documents(self.tabs.iter().map(|tab| tab.document_epoch));
        if activate {
            let index = self.tabs.len() - 1;
            self.active_tab = Some(index);
            self.apply_tab_state(self.tabs[index].clone(), ctx);
            if first_document {
                self.apply_startup_view_mode(ctx);
            }
            self.request_document_links();
            // The visible-page path already queues renders asynchronously.
            if options.prefetch {
                self.prefetch_small_document_pages(ctx);
            }
            self.start_review_precompute_if_eligible(ctx);
        }
        if self.open_into_sbs && self.tabs.len() >= 2 {
            self.open_into_sbs = false;
            self.toggle_sbs_mode(ctx);
        }
        ctx.request_repaint();
    }
}
