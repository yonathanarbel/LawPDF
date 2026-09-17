//! Semantics for custom-painted PDF pages and compact controls.
use super::*;

impl PdfEditorApp {
    pub(super) fn expose_page_text(
        &mut self,
        response: &egui::Response,
        document_epoch: u64,
        path: &Path,
        page_index: usize,
    ) {
        // Text extraction is lazy and stays on the PDF worker. Do not pay for
        // accessibility-only page text when no accessibility client is active.
        if response
            .ctx
            .accesskit_node_builder(response.id, |_| ())
            .is_none()
        {
            return;
        }
        let current = self.is_current_document(document_epoch, path);
        if current {
            self.enqueue_native_text(path, page_index);
        } else {
            let needs_text = self
                .tabs
                .iter()
                .find(|tab| tab.document_epoch == document_epoch && tab.document.path == path)
                .and_then(|tab| tab.document.native_text_loaded.get(page_index))
                .is_some_and(|loaded| !*loaded);
            if needs_text
                && self
                    .pending_accessible_text
                    .insert((document_epoch, page_index))
            {
                if self
                    .render_tx
                    .send(RenderRequest::TextPageAsync {
                        document_epoch,
                        path: path.to_path_buf(),
                        page_index,
                    })
                    .is_err()
                {
                    self.pending_accessible_text
                        .remove(&(document_epoch, page_index));
                }
            }
        }
        let source = if current {
            self.document
                .as_ref()
                .map(|document| (document, self.ocr_states.as_slice()))
        } else {
            self.tabs
                .iter()
                .find(|tab| tab.document_epoch == document_epoch && tab.document.path == path)
                .map(|tab| (&tab.document, tab.ocr_states.as_slice()))
        };
        let Some((document, ocr_states)) = source else {
            return;
        };
        response.widget_info(|| {
            let native = document.native_text.get(page_index).map(String::as_str).unwrap_or("");
            let ocr = ocr_states.get(page_index).and_then(OcrPageState::text).unwrap_or("");
            let text = if !native.trim().is_empty() { native } else { ocr };
            let label = if !text.trim().is_empty() {
                format!("Page {} of {}. {}", page_index + 1, document.page_count, text)
            } else if !document.native_text_loaded.get(page_index).copied().unwrap_or(true) {
                format!("Page {} of {}. Loading page text.", page_index + 1, document.page_count)
            } else {
                format!("Page {} of {}. No selectable text is available. Use OCR to recognize scanned text.", page_index + 1, document.page_count)
            };
            egui::WidgetInfo::labeled(egui::WidgetType::Label, true, label)
        });
    }
}

pub(super) fn name_color_choice(response: &egui::Response, label: &str, selected: bool) {
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::Button,
            response.enabled(),
            selected,
            label,
        )
    });
}
