use super::*;
use crate::document_store::{DocumentStore, FileRevision};

pub(super) struct SaveAsTask {
    epoch: u64,
    source: PathBuf,
    destination: PathBuf,
    revision: FileRevision,
    annotations: Vec<EditorAnnotation>,
    result: Option<Receiver<Result<FileRevision, String>>>,
}

impl PdfEditorApp {
    pub(super) fn save_as_dialog(&mut self, ctx: &Context) {
        if self.save_as.is_some() || self.recovery_is_busy() || self.page_rotation_in_flight {
            self.status = "Wait for the current document operation to finish.".to_owned();
            return;
        }
        let Some(document) = &self.document else {
            return;
        };
        let Some(session) = self.annotation_sessions.get(&self.document_epoch) else {
            return;
        };
        if session.recovery_pending {
            self.open_recovery_dialog(ctx);
            return;
        }
        let source = document.path.clone();
        let filename = source.file_name().unwrap_or_default().to_string_lossy();
        // Keep the attachment's filename and let the native panel choose its
        // initial folder; the user can select any destination.
        let Some(destination) = self.pick_save_path("Save PDF As", &filename, "PDF", &["pdf"])
        else {
            return;
        };
        if let Err(error) = self.begin_save_as(destination) {
            self.push_error_notice(error);
        }
        ctx.request_repaint();
    }

    fn begin_save_as(&mut self, destination: PathBuf) -> Result<(), String> {
        let destination = std::fs::canonicalize(&destination).unwrap_or(destination);
        self.normalize_document_comment_authors(self.document_epoch);
        let Some(document) = &self.document else {
            return Ok(());
        };
        if destination == document.path {
            return self.save_current_annotations();
        }
        if self.tabs.iter().any(|tab| tab.document.path == destination)
            || self.pending_document_opens.contains_key(&destination)
        {
            return Err("That PDF is already open. Choose another filename.".to_owned());
        }
        let session = self
            .annotation_sessions
            .get(&self.document_epoch)
            .ok_or_else(|| "This PDF's recovery source is unavailable.".to_owned())?;
        self.save_as = Some(SaveAsTask {
            epoch: self.document_epoch,
            source: document.path.clone(),
            destination,
            revision: session.revision.clone(),
            annotations: self.annotations.clone(),
            result: None,
        });
        self.status = "Saving PDF…".to_owned();
        Ok(())
    }

    /// Hold document interaction briefly while transferring its save destination.
    /// In-flight autosaves finish first; queued saves stay recoverable until the
    /// new file AND its recovery snapshot are durable. Failure changes no path.
    pub(super) fn poll_save_as(&mut self, ctx: &Context) -> bool {
        let Some(task) = self.save_as.as_mut() else {
            return false;
        };
        if task.result.is_none() && !self.active_annotation_saves.contains_key(&task.source) {
            let source = task.source.clone();
            let destination = task.destination.clone();
            let revision = task.revision.clone();
            let annotations = task.annotations.clone();
            let (tx, rx) = unbounded();
            task.result = Some(rx);
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let result = (|| {
                    let store = DocumentStore::new()?;
                    store.export_annotations(&source, &revision, &annotations, &destination)?;
                    store.capture(&destination)
                })();
                let _ = tx.send(result);
                ctx.request_repaint();
            });
        }
        let result = task.result.as_ref().and_then(|rx| match rx.try_recv() {
            Ok(result) => Some(result),
            Err(crossbeam_channel::TryRecvError::Empty) => None,
            Err(crossbeam_channel::TryRecvError::Disconnected) => Some(Err(
                "The Save As task stopped. Your original and edits were kept.".to_owned(),
            )),
        });
        if let Some(result) = result {
            let task = self.save_as.take().expect("active Save As");
            self.finish_save_as(task, result, ctx);
            return false;
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(48.0);
                ui.heading("Saving PDF…");
                ui.label(task.destination.display().to_string());
                ui.spinner();
            });
        });
        ctx.request_repaint_after(RENDER_POLL_INTERVAL);
        true
    }

    fn finish_save_as(
        &mut self,
        task: SaveAsTask,
        result: Result<FileRevision, String>,
        ctx: &Context,
    ) {
        let revision = match result {
            Ok(revision) => revision,
            Err(error) => {
                self.push_error_notice(format!("Save As failed: {error}"));
                return;
            }
        };
        let destination = std::fs::canonicalize(&task.destination).unwrap_or(task.destination);
        self.save_active_tab_state();
        let Some(index) = self
            .tabs
            .iter()
            .position(|tab| tab.document_epoch == task.epoch && tab.document.path == task.source)
        else {
            return;
        };
        self.pending_annotation_saves.remove(&task.source);
        let tab = &mut self.tabs[index];
        let newer_edits = (tab.annotations != task.annotations).then(|| tab.annotations.clone());
        tab.document.path = destination.clone();
        tab.document.title = destination
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        tab.annotations_dirty = newer_edits.is_some();
        tab.status = format!("Saved to {}", destination.display());
        if let Some(session) = self.annotation_sessions.get_mut(&task.epoch) {
            session.revision = revision;
            session.recovery_error = None;
        }
        if self.active_tab == Some(index) {
            // Keep the viewport and selection in place when only the path changes.
            self.document = Some(self.tabs[index].document.clone());
            self.annotations_dirty = self.tabs[index].annotations_dirty;
            self.status = self.tabs[index].status.clone();
            self.pending_page_renders.clear();
            self.pending_thumbnail_renders.clear();
            self.pending_native_text.clear();
            self.pending_text_chars.clear();
            self.remember_pdf_zoom(self.target_zoom);
        }
        ctx.request_repaint();
        // Input delivered in the frame that opened the panel may have changed
        // an annotation after the snapshot. Carry it to the chosen destination.
        if let Some(annotations) = newer_edits
            && let Err(error) = self.queue_annotation_save(task.epoch, destination, annotations)
        {
            self.push_error_notice(error);
            return; // Keep the original journal until the newer edits are protected.
        }
        if let Err(error) = DocumentStore::new().and_then(|store| store.discard(&task.source)) {
            self.push_error_notice(format!(
                "The PDF was saved, but recovery cleanup failed: {error}"
            ));
        }
    }

    #[cfg(target_os = "macos")]
    pub(super) fn reveal_current_pdf(&mut self) {
        if let Some(document) = &self.document
            && let Err(error) = std::process::Command::new("/usr/bin/open")
                .arg("-R")
                .arg(&document.path)
                .spawn()
        {
            self.push_error_notice(format!("Could not show the PDF in Finder: {error}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edited_fixture() -> (PdfEditorApp, Context, Receiver<RenderRequest>) {
        let (mut app, ctx, rx) = super::super::annotation_saving::tests::fixture();
        let path = &app.document.as_ref().unwrap().path;
        // A protected attachment also exercises saving from the immutable
        // recovery source when the original cannot be overwritten.
        std::fs::write(
            path,
            include_bytes!("../../tests/fixtures/owner-password.pdf"),
        )
        .unwrap();
        app.annotation_sessions
            .get_mut(&app.document_epoch)
            .unwrap()
            .revision = DocumentStore::new().unwrap().capture(path).unwrap();
        app.add_text_box_annotation(0, PdfRect::new(10.0, 20.0, 200.0, 60.0));
        (app, ctx, rx)
    }

    fn finish(app: &mut PdfEditorApp, ctx: &Context) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.save_as.is_some() {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                app.poll_save_as(ctx);
            });
            assert!(Instant::now() < deadline, "Save As did not finish");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn save_as_writes_current_comment_author_name_without_renaming_foreign_authors() {
        let (mut app, ctx, _) = edited_fixture();
        app.settings.comment_author_id = "this-reader".to_owned();
        app.settings.comment_author_name = "New profile name".to_owned();
        let comment = |id: &str, author_id: &str| EditorAnnotation {
            page_index: 0,
            rect: PdfRect::new(500.0, 650.0, 530.0, 680.0),
            kind: AnnotationKind::Comment {
                id: format!("LawPDF-comment-{id}"),
                text: "A note".to_owned(),
                author: "Old profile name".to_owned(),
                author_id: author_id.to_owned(),
                color_rgb: [0.2, 0.4, 0.7],
                created_at: "created".to_owned(),
                updated_at: "updated".to_owned(),
                anchor: (220.0, 660.0),
            },
        };
        app.annotations.extend([
            comment("own", "this-reader"),
            comment("foreign", "someone-else"),
        ]);
        let destination = app
            .document
            .as_ref()
            .unwrap()
            .path
            .with_file_name("renamed-author-export.pdf");
        app.begin_save_as(destination.clone()).unwrap();
        finish(&mut app, &ctx);
        assert!(
            !app.annotations_dirty,
            "normalizing a saved author must not look like a newer edit"
        );
        let annotations = crate::pdf_backend::load_lawpdf_annotations(&destination).unwrap();
        for (id, expected) in [
            ("this-reader", "New profile name"),
            ("someone-else", "Old profile name"),
        ] {
            assert!(annotations.iter().any(|annotation| matches!(&annotation.kind,
                AnnotationKind::Comment { author, author_id, .. } if author_id == id && author == expected)));
        }
    }

    #[test]
    fn save_as_preserves_attachment_and_redirects_later_saves_and_undo() {
        let (mut app, ctx, rx) = edited_fixture();
        let source = app.document.as_ref().unwrap().path.clone();
        let original = std::fs::read(&source).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let destination = directory
            .path()
            .canonicalize()
            .unwrap()
            .join("My saved attachment.pdf");
        app.begin_save_as(destination.clone()).unwrap();
        finish(&mut app, &ctx);
        assert!(destination.is_file());
        assert_eq!(
            crate::pdf_backend::load_lawpdf_annotations(&destination).unwrap(),
            app.annotations
        );
        assert_eq!(std::fs::read(&source).unwrap(), original);
        assert_eq!(app.document.as_ref().unwrap().path, destination);
        assert_eq!(app.tabs[0].document.path, destination);
        assert_eq!(app.tabs[0].title(), "My saved attachment.pdf");
        assert!(!app.annotations_dirty);
        assert!(!app.pending_annotation_saves.contains_key(&source));
        let session = &app.annotation_sessions[&app.document_epoch];
        session.revision.require_current(&destination).unwrap();
        assert!(!session.undo.is_empty());
        assert!(
            !DocumentStore::new()
                .unwrap()
                .pending()
                .unwrap()
                .items
                .iter()
                .any(|item| item.record.original_path == source)
        );
        app.add_text_box_annotation(0, PdfRect::new(10.0, 80.0, 200.0, 100.0));
        app.save_current_annotations().unwrap();
        app.start_due_annotation_saves(&ctx);
        let RenderRequest::AutosaveAnnotations {
            path,
            expected_revision,
            annotations,
            ..
        } = rx.try_recv().unwrap()
        else {
            panic!("expected autosave at new destination")
        };
        assert_eq!(path, destination);
        assert_eq!(annotations.len(), 2);
        expected_revision.require_current(&destination).unwrap();
        crate::pdf_backend::save_with_annotations_checked(
            &path,
            &path,
            &annotations,
            &expected_revision,
        )
        .unwrap();
        assert_eq!(
            crate::pdf_backend::load_lawpdf_annotations(&destination).unwrap(),
            annotations
        );
        assert_eq!(std::fs::read(&source).unwrap(), original);
    }

    #[test]
    fn edits_arriving_with_save_as_are_carried_to_the_new_destination() {
        let (mut app, ctx, _rx) = edited_fixture();
        let source = app.document.as_ref().unwrap().path.clone();
        let directory = tempfile::tempdir().unwrap();
        let destination = directory
            .path()
            .canonicalize()
            .unwrap()
            .join("newer-edits.pdf");
        app.begin_save_as(destination.clone()).unwrap();
        app.add_text_box_annotation(0, PdfRect::new(10.0, 80.0, 200.0, 100.0));
        finish(&mut app, &ctx);
        assert_eq!(app.document.as_ref().unwrap().path, destination);
        assert!(app.annotations_dirty);
        assert_eq!(app.annotations.len(), 2);
        assert_eq!(
            app.pending_annotation_saves[&destination].annotations,
            app.annotations
        );
        assert!(!app.pending_annotation_saves.contains_key(&source));
    }

    #[test]
    fn failed_save_as_keeps_original_path_edits_and_pending_save() {
        let (mut app, ctx, _rx) = edited_fixture();
        let source = app.document.as_ref().unwrap().path.clone();
        let annotations = app.annotations.clone();
        let directory = tempfile::tempdir().unwrap();
        app.begin_save_as(directory.path().join("missing/failure.pdf"))
            .unwrap();
        finish(&mut app, &ctx);
        assert_eq!(app.document.as_ref().unwrap().path, source);
        assert_eq!(app.annotations, annotations);
        assert!(app.annotations_dirty);
        assert!(app.pending_annotation_saves.contains_key(&source));
        assert!(!app.notices.is_empty());
    }

    #[test]
    fn save_as_waits_for_in_flight_save_and_prevents_close() {
        let (mut app, ctx, _rx) = edited_fixture();
        let source = app.document.as_ref().unwrap().path.clone();
        app.save_current_annotations().unwrap();
        app.start_due_annotation_saves(&ctx);
        let directory = tempfile::tempdir().unwrap();
        let destination = directory
            .path()
            .canonicalize()
            .unwrap()
            .join("after-autosave.pdf");
        app.begin_save_as(destination.clone()).unwrap();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            assert!(app.poll_save_as(ctx));
        });
        assert!(app.save_as.as_ref().unwrap().result.is_none());
        assert!(!destination.exists());
        assert!(app.close_target_is_saving(CloseTarget::Window));
        app.request_window_close(&ctx);
        assert!(!app.allow_window_close);
        let save = app.active_annotation_saves[&source].clone();
        app.finish_annotation_autosave(
            save.document_epoch,
            source,
            save.generation,
            Err("protected original".to_owned()),
            &ctx,
        );
        finish(&mut app, &ctx);
        assert_eq!(app.document.as_ref().unwrap().path, destination);
        assert!(!app.annotations_dirty);
    }

    #[test]
    fn save_as_refuses_to_overwrite_another_open_tab() {
        let (mut app, _ctx, _rx) = edited_fixture();
        let mut other = app.tabs[0].clone();
        other.document.path = PathBuf::from("other-open.pdf");
        other.document_epoch = 99;
        app.tabs.push(other);
        assert!(app.begin_save_as(PathBuf::from("other-open.pdf")).is_err());
        assert!(app.save_as.is_none());
        assert!(app.annotations_dirty);
    }

    #[test]
    fn shift_command_s_is_consumed_before_plain_save() {
        for modifiers in [
            egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
            egui::Modifiers::CTRL | egui::Modifiers::SHIFT,
            egui::Modifiers::MAC_CMD | egui::Modifiers::SHIFT,
        ] {
            let ctx = Context::default();
            ctx.begin_pass(egui::RawInput {
                events: vec![egui::Event::Key {
                    key: egui::Key::S,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers,
                }],
                ..Default::default()
            });
            assert!(consume_save_as_shortcut(&ctx));
            assert!(!consume_command_shortcut(&ctx, egui::Key::S));
            let _ = ctx.end_pass();
        }
        let ctx = Context::default();
        ctx.begin_pass(egui::RawInput {
            events: vec![egui::Event::Key {
                key: egui::Key::S,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::COMMAND,
            }],
            ..Default::default()
        });
        assert!(!consume_save_as_shortcut(&ctx));
        assert!(consume_command_shortcut(&ctx, egui::Key::S));
        let _ = ctx.end_pass();
    }
}
