use super::*;
use crate::document_store::{DocumentStore, FileRevision};

/// Per-document editing state survives tab switches without borrowing native
/// PDF handles. Every command, including undo/redo, uses the same journal.
#[derive(Clone)]
pub(super) struct AnnotationSession {
    pub revision: FileRevision,
    pub current: Vec<EditorAnnotation>,
    pub undo: Vec<Vec<EditorAnnotation>>,
    pub redo: Vec<Vec<EditorAnnotation>>,
    pub recovery_error: Option<String>,
    pub recovery_pending: bool,
}

impl AnnotationSession {
    pub fn new(revision: FileRevision, annotations: Vec<EditorAnnotation>) -> Self {
        Self {
            revision,
            current: annotations,
            undo: Vec::new(),
            redo: Vec::new(),
            recovery_error: None,
            recovery_pending: false,
        }
    }

    pub fn record(&mut self, annotations: &[EditorAnnotation]) {
        if self.current == annotations {
            return;
        }
        self.undo
            .push(std::mem::replace(&mut self.current, annotations.to_vec()));
        // Cap both the number of steps and their combined annotation payload.
        // Journal recovery is independent of this bounded interactive history.
        while self.undo.len() > 100
            || self
                .undo
                .iter()
                .map(|snapshot| annotation_bytes(snapshot))
                .sum::<usize>()
                > 16 * 1024 * 1024
        {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    pub fn undo(&mut self) -> Option<Vec<EditorAnnotation>> {
        let previous = self.undo.pop()?;
        self.redo
            .push(std::mem::replace(&mut self.current, previous));
        Some(self.current.clone())
    }

    pub fn redo(&mut self) -> Option<Vec<EditorAnnotation>> {
        let next = self.redo.pop()?;
        self.undo.push(std::mem::replace(&mut self.current, next));
        Some(self.current.clone())
    }
}

fn annotation_bytes(annotations: &[EditorAnnotation]) -> usize {
    annotations
        .iter()
        .map(|annotation| {
            std::mem::size_of::<EditorAnnotation>()
                + match &annotation.kind {
                    AnnotationKind::Marker { .. } => 0,
                    AnnotationKind::TextBox { text, .. } => text.len(),
                    AnnotationKind::Comment {
                        text,
                        id,
                        created_at,
                        updated_at,
                        ..
                    } => text.len() + id.len() + created_at.len() + updated_at.len(),
                    AnnotationKind::Signature {
                        signer,
                        signed_at,
                        strokes,
                    } => {
                        signer.len()
                            + signed_at.len()
                            + strokes
                                .iter()
                                .map(|stroke| stroke.len() * std::mem::size_of::<(f32, f32)>())
                                .sum::<usize>()
                    }
                }
        })
        .sum()
}

impl PdfEditorApp {
    pub(super) fn restore_annotation_history(&mut self, redo: bool, ctx: &Context) {
        if self.page_rotation_in_flight {
            return;
        }
        let Some(session) = self.annotation_sessions.get_mut(&self.document_epoch) else {
            return;
        };
        if session.recovery_pending {
            return;
        }
        let snapshot = if redo { session.redo() } else { session.undo() };
        if let Some(snapshot) = snapshot {
            self.annotations = snapshot;
            self.clear_text_box_selection();
            self.clear_comment_selection();
            self.text_box_drag = None;
            self.comment_drag = None;
            self.mark_annotations_changed();
            ctx.request_repaint();
        }
    }

    pub(super) fn annotation_history_buttons(&mut self, ui: &mut egui::Ui, ctx: &Context) {
        let session = self.annotation_sessions.get(&self.document_epoch);
        let enabled = session.is_some_and(|session| !session.recovery_pending)
            && !self.page_rotation_in_flight;
        let undo = enabled && session.is_some_and(|session| !session.undo.is_empty());
        let redo = enabled && session.is_some_and(|session| !session.redo.is_empty());
        if ui
            .add_enabled(undo, egui::Button::new("Undo"))
            .on_hover_text("Undo annotation change (⌘Z / Ctrl+Z)")
            .clicked()
        {
            self.restore_annotation_history(false, ctx);
        }
        if ui
            .add_enabled(redo, egui::Button::new("Redo"))
            .on_hover_text("Redo annotation change (⇧⌘Z / Ctrl+Y)")
            .clicked()
        {
            self.restore_annotation_history(true, ctx);
        }
    }

    pub(super) fn journal_active_annotations(&mut self, generation: u64) -> Result<(), String> {
        let Some(document) = &self.document else {
            return Ok(());
        };
        let session = self
            .annotation_sessions
            .get_mut(&self.document_epoch)
            .ok_or_else(|| {
                "The recovery source is not ready. Keep the document open.".to_owned()
            })?;
        if session.recovery_pending {
            self.annotations = session.current.clone();
            return Err("Resolve the earlier recovered edits before editing this PDF.".to_owned());
        }
        session.record(&self.annotations);
        let result = DocumentStore::new()?.journal(
            &document.path,
            &session.revision,
            generation,
            &self.annotations,
        );
        session.recovery_error = result.as_ref().err().cloned();
        result
    }
}
