use super::*;
use crate::pdf_backend::SaveReport;

impl PdfEditorApp {
    pub(super) fn queue_annotation_save(
        &mut self,
        epoch: u64,
        path: PathBuf,
        annotations: Vec<EditorAnnotation>,
    ) -> Result<(), String> {
        let session = self
            .annotation_sessions
            .get(&epoch)
            .ok_or_else(|| "No recovery source is available; save a copy instead.".to_owned())?;
        if session.recovery_pending {
            return Err("Resolve the earlier recovered edits before saving this PDF.".to_owned());
        }
        self.annotation_save_generation += 1;
        let generation = self.annotation_save_generation;
        crate::document_store::DocumentStore::new()?.journal(
            &path,
            &session.revision,
            generation,
            &annotations,
        )?;
        self.pending_annotation_saves.insert(
            path.clone(),
            PendingAnnotationSave {
                document_epoch: epoch,
                path,
                generation,
                annotations,
                due_at: Instant::now(),
            },
        );
        self.status = "Saving annotations…".to_owned();
        Ok(())
    }

    /// Every annotation mutation enters the same queue. Snapshots belong to a
    /// document epoch, so a late completion cannot acknowledge a different tab.
    pub(super) fn mark_annotations_changed(&mut self) {
        if self.page_rotation_in_flight {
            if let Some(session) = self.annotation_sessions.get(&self.document_epoch) {
                self.annotations = session.current.clone();
            }
            self.push_error_notice("Wait for page rotation to finish before changing annotations.");
            return;
        }
        if let Some(session) = self.annotation_sessions.get(&self.document_epoch)
            && session.recovery_pending
        {
            self.annotations = session.current.clone();
            self.push_error_notice("Recover or discard the earlier edits before editing this PDF.");
            return;
        }
        self.annotations_dirty = true;
        let Some(path) = self.document.as_ref().map(|document| document.path.clone()) else {
            return;
        };
        self.annotation_save_generation += 1;
        if let Err(error) = self.journal_active_annotations(self.annotation_save_generation) {
            self.push_error_notice(error);
            return;
        }
        self.pending_annotation_saves.insert(
            path.clone(),
            PendingAnnotationSave {
                document_epoch: self.document_epoch,
                path,
                generation: self.annotation_save_generation,
                annotations: self.annotations.clone(),
                due_at: Instant::now() + ANNOTATION_AUTOSAVE_DELAY,
            },
        );
    }

    pub(super) fn schedule_annotation_autosave(&mut self, ctx: &Context) {
        self.mark_annotations_changed();
        ctx.request_repaint_after(ANNOTATION_AUTOSAVE_DELAY);
    }

    pub(super) fn schedule_annotation_autosave_now(&mut self, ctx: &Context) {
        self.schedule_annotation_autosave(ctx);
        if let Some(document) = &self.document
            && let Some(save) = self.pending_annotation_saves.get_mut(&document.path)
        {
            save.due_at = Instant::now();
        }
        self.start_due_annotation_saves(ctx);
    }

    pub(super) fn start_due_annotation_saves(&mut self, ctx: &Context) {
        let now = Instant::now();
        let paths = self
            .pending_annotation_saves
            .iter()
            .filter(|(path, save)| {
                save.due_at <= now && !self.active_annotation_saves.contains_key(*path)
            })
            .map(|(path, _)| path.clone())
            .collect::<Vec<_>>();
        for path in paths {
            let Some(save) = self.pending_annotation_saves.remove(&path) else {
                continue;
            };
            let Some(session) = self.annotation_sessions.get(&save.document_epoch) else {
                self.push_error_notice(
                    "Could not find this PDF's recovery state. Keep it open and save a copy.",
                );
                continue;
            };
            let request = RenderRequest::AutosaveAnnotations {
                document_epoch: save.document_epoch,
                path: save.path.clone(),
                generation: save.generation,
                annotations: save.annotations.clone(),
                expected_revision: session.revision.clone(),
            };
            match self.render_tx.send(request) {
                Err(error) if error.is_full() => {
                    let mut save = save;
                    save.due_at = Instant::now() + RENDER_POLL_INTERVAL;
                    self.pending_annotation_saves.insert(path, save);
                    ctx.request_repaint_after(RENDER_POLL_INTERVAL);
                }
                Err(_) => self.push_error_notice("PDF worker is not available. Your annotations are still unsaved; keep this document open."),
                Ok(()) => {
                    self.active_annotation_saves.insert(path, save);
                    self.status = "Saving annotations…".to_owned();
                    ctx.request_repaint_after(RENDER_POLL_INTERVAL);
                }
            }
        }
        if let Some(next) = self
            .pending_annotation_saves
            .values()
            .map(|save| save.due_at)
            .min()
        {
            ctx.request_repaint_after(next.saturating_duration_since(now));
        }
    }

    pub(super) fn finish_annotation_autosave(
        &mut self,
        epoch: u64,
        path: PathBuf,
        generation: u64,
        result: Result<SaveReport, String>,
        ctx: &Context,
    ) {
        // Explicit Save may already have superseded this completion.
        if !self
            .active_annotation_saves
            .get(&path)
            .is_some_and(|save| save.document_epoch == epoch && save.generation == generation)
        {
            return;
        }
        let saved = self
            .active_annotation_saves
            .remove(&path)
            .expect("matched save");
        match result {
            Ok(report) => {
                if let Some(revision) = report.revision
                    && let Some(session) = self.annotation_sessions.get_mut(&epoch)
                {
                    session.revision = revision.clone();
                    if let Some(pending) = self.pending_annotation_saves.get(&path)
                        && let Err(error) =
                            crate::document_store::DocumentStore::new().and_then(|store| {
                                store.journal(
                                    &path,
                                    &revision,
                                    pending.generation,
                                    &pending.annotations,
                                )
                            })
                    {
                        self.push_error_notice(error);
                    }
                }
                self.save_active_tab_state();
                let newer_pending = self.pending_annotation_saves.contains_key(&path);
                for tab in &mut self.tabs {
                    if tab.document_epoch == epoch
                        && tab.document.path == path
                        && !newer_pending
                        && tab.annotations == saved.annotations
                    {
                        tab.annotations_dirty = false;
                    }
                }
                if self.is_current_document(epoch, &path)
                    && !newer_pending
                    && self.annotations == saved.annotations
                {
                    self.annotations_dirty = false;
                    self.status = "All annotations saved to PDF.".to_owned();
                }
                if let Some(warning) = report.recovery_warning {
                    self.push_error_notice(format!(
                        "The PDF was saved, but recovery cleanup needs attention: {warning}"
                    ));
                }
            }
            Err(error) => self.push_error_notice(format!(
                "Could not save annotations to {}: {error}. Your changes are still unsaved.",
                path.display()
            )),
        }
        ctx.request_repaint();
    }

    pub(super) fn close_target_is_saving(&self, target: CloseTarget) -> bool {
        self.page_rotation_in_flight
            || self.recovery_ui.is_writing()
            || self
                .pending_annotation_saves
                .values()
                .chain(self.active_annotation_saves.values())
                .any(|save| match target {
                    CloseTarget::Window => true,
                    CloseTarget::Tab(epoch) => save.document_epoch == epoch,
                })
    }

    pub(super) fn close_target_is_dirty(&self, target: CloseTarget) -> bool {
        match target {
            CloseTarget::Window => self.has_unsaved_annotations(),
            CloseTarget::Tab(epoch) => self
                .tabs
                .iter()
                .any(|tab| tab.document_epoch == epoch && tab.annotations_dirty),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::PageInfo;

    pub(crate) fn fixture() -> (PdfEditorApp, Context, Receiver<RenderRequest>) {
        let ctx = Context::default();
        let mut app = PdfEditorApp::new(
            &ctx,
            Vec::new(),
            unbounded().1,
            #[cfg(target_os = "macos")]
            None,
        );
        let (tx, rx) = unbounded();
        app.render_tx = tx.into();
        let source = tempfile::tempdir()
            .unwrap()
            .keep()
            .join("autosave-fixture.pdf");
        std::fs::write(&source, b"initial test document").unwrap();
        let revision = crate::document_store::DocumentStore::new()
            .unwrap()
            .capture(&source)
            .unwrap();
        let document = LoadedDocument {
            path: source,
            title: "Fixture".to_owned(),
            page_count: 1,
            pages: vec![PageInfo::new(612.0, 792.0)],
            native_text: vec!["Read this".to_owned()],
            native_text_loaded: vec![true],
            text_chars: vec![None],
            links: vec![Vec::new()],
            links_loaded: true,
            optimized: true,
        };
        app.document = Some(document.clone());
        app.document_epoch = 7;
        app.annotation_sessions.insert(
            7,
            super::super::annotation_session::AnnotationSession::new(revision, Vec::new()),
        );
        app.active_tab = Some(0);
        app.tabs.push(app.active_tab_snapshot(document));
        (app, ctx, rx)
    }

    fn start_save(
        app: &mut PdfEditorApp,
        ctx: &Context,
        rx: &Receiver<RenderRequest>,
    ) -> PendingAnnotationSave {
        for save in app.pending_annotation_saves.values_mut() {
            save.due_at = Instant::now();
        }
        app.start_due_annotation_saves(ctx);
        let RenderRequest::AutosaveAnnotations {
            path,
            document_epoch,
            generation,
            annotations,
            ..
        } = rx.try_recv().unwrap()
        else {
            panic!("expected annotation save")
        };
        PendingAnnotationSave {
            path,
            document_epoch,
            generation,
            annotations,
            due_at: Instant::now(),
        }
    }

    #[test]
    fn text_box_creation_edit_and_deletion_all_queue_complete_snapshots() {
        let (mut app, ctx, rx) = fixture();
        app.add_text_box_annotation(0, PdfRect::new(10.0, 20.0, 200.0, 60.0));
        let first = start_save(&mut app, &ctx, &rx);
        assert_eq!(first.annotations, app.annotations);
        assert_eq!(first.annotations.len(), 1);
        app.finish_annotation_autosave(
            7,
            first.path.clone(),
            first.generation,
            Ok(SaveReport::default()),
            &ctx,
        );
        assert!(!app.annotations_dirty);
        app.delete_text_box(0);
        assert!(app.annotations_dirty);
        let deleted = start_save(&mut app, &ctx, &rx);
        assert!(
            deleted.annotations.is_empty(),
            "deleting the last annotation must also be saved"
        );
    }

    #[test]
    fn highlighting_text_queues_a_save_without_pressing_save() {
        let (mut app, ctx, rx) = fixture();
        app.document.as_mut().unwrap().text_chars[0] = Some(vec![
            crate::model::PageTextChar {
                ch: 'A',
                rect: Some(PdfRect::new(10.0, 20.0, 20.0, 32.0)),
                font_size: Some(12.0),
                bold: false,
                italic: false,
            },
            crate::model::PageTextChar {
                ch: 'B',
                rect: Some(PdfRect::new(20.0, 20.0, 30.0, 32.0)),
                font_size: Some(12.0),
                bold: false,
                italic: false,
            },
        ]);
        app.selection_state.text = Some(TextSelection::range(0, 0, 0, 1));
        app.mark_selection(MARKER_PRESETS[0]);
        assert!(app.annotations_dirty);
        let saved = start_save(&mut app, &ctx, &rx);
        assert!(!saved.annotations.is_empty());
        assert!(
            saved
                .annotations
                .iter()
                .all(|annotation| matches!(annotation.kind, AnnotationKind::Marker { .. }))
        );
    }

    #[test]
    fn older_save_cannot_acknowledge_newer_edits_or_supersede_explicit_save() {
        let (mut app, ctx, rx) = fixture();
        app.add_text_box_annotation(0, PdfRect::new(10.0, 20.0, 200.0, 60.0));
        let first = start_save(&mut app, &ctx, &rx);
        app.add_text_box_annotation(0, PdfRect::new(10.0, 80.0, 200.0, 100.0));
        app.finish_annotation_autosave(
            7,
            first.path.clone(),
            first.generation,
            Ok(SaveReport::default()),
            &ctx,
        );
        assert!(app.annotations_dirty);
        let second = start_save(&mut app, &ctx, &rx);
        assert_eq!(second.annotations.len(), 2);
        // A delayed duplicate event cannot remove the newer active save.
        app.finish_annotation_autosave(
            7,
            first.path,
            first.generation,
            Ok(SaveReport::default()),
            &ctx,
        );
        assert_eq!(app.active_annotation_saves.len(), 1);
        app.finish_annotation_autosave(
            7,
            second.path,
            second.generation,
            Ok(SaveReport::default()),
            &ctx,
        );
        assert!(!app.annotations_dirty);
        assert!(!app.tabs[0].annotations_dirty);
    }

    #[test]
    fn closing_waits_for_autosave_and_failed_save_keeps_changes_open() {
        let (mut app, ctx, rx) = fixture();
        app.add_text_box_annotation(0, PdfRect::new(10.0, 20.0, 200.0, 60.0));
        app.close_tab(0, &ctx);
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            app.draw_unsaved_close_prompt(ctx)
        });
        let RenderRequest::AutosaveAnnotations {
            path, generation, ..
        } = rx.try_recv().unwrap()
        else {
            panic!("expected save before close")
        };
        assert_eq!(app.tabs.len(), 1);
        app.finish_annotation_autosave(7, path, generation, Err("read-only file".to_owned()), &ctx);
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            app.draw_unsaved_close_prompt(ctx)
        });
        assert_eq!(app.tabs.len(), 1);
        assert!(app.annotations_dirty);
        assert_eq!(app.pending_close, Some(CloseTarget::Tab(7)));
        assert!(!app.notices.is_empty());
    }

    #[test]
    fn successful_autosave_finishes_close_without_discarding_other_tabs() {
        let (mut app, ctx, rx) = fixture();
        app.add_text_box_annotation(0, PdfRect::new(10.0, 20.0, 200.0, 60.0));
        app.save_active_tab_state();
        let mut other = app.tabs[0].clone();
        other.document_epoch = 9;
        other.document.path = PathBuf::from("other.pdf");
        other.annotations_dirty = false;
        app.tabs.push(other);
        app.close_tab(0, &ctx);
        let saved = start_save(&mut app, &ctx, &rx);
        app.finish_annotation_autosave(
            7,
            saved.path,
            saved.generation,
            Ok(SaveReport::default()),
            &ctx,
        );
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            app.draw_unsaved_close_prompt(ctx)
        });
        assert_eq!(app.tabs.len(), 1);
        assert_eq!(app.tabs[0].document_epoch, 9);
        assert!(app.pending_close.is_none());
    }

    #[test]
    fn save_completion_updates_the_original_tab_after_switching_documents() {
        let (mut app, ctx, rx) = fixture();
        app.add_text_box_annotation(0, PdfRect::new(10.0, 20.0, 200.0, 60.0));
        let saved = start_save(&mut app, &ctx, &rx);
        app.save_active_tab_state();
        let mut other = app.tabs[0].clone();
        other.document_epoch = 9;
        other.document.path = PathBuf::from("other.pdf");
        app.tabs.push(other.clone());
        app.document = Some(other.document);
        app.document_epoch = 9;
        app.active_tab = Some(1);
        app.finish_annotation_autosave(
            7,
            saved.path,
            saved.generation,
            Ok(SaveReport::default()),
            &ctx,
        );
        assert!(!app.tabs[0].annotations_dirty);
        assert!(app.tabs[1].annotations_dirty);
        assert!(
            app.annotations_dirty,
            "saving another tab cannot mark this tab saved"
        );
    }

    #[test]
    fn stopped_worker_does_not_leave_close_waiting_forever_for_a_save() {
        let (mut app, ctx, rx) = fixture();
        app.add_text_box_annotation(0, PdfRect::new(10.0, 20.0, 200.0, 60.0));
        let _ = start_save(&mut app, &ctx, &rx);
        let (events, results) = unbounded();
        app.render_rx = results;
        drop(events);
        app.poll_render_results(&ctx);
        assert!(app.active_annotation_saves.is_empty());
        assert!(app.annotations_dirty);
        assert!(!app.notices.is_empty());
    }

    #[cfg(not(feature = "microsoft-store"))]
    #[test]
    fn background_update_network_failure_is_quiet_but_integrity_failure_is_visible() {
        let (mut app, ctx, _rx) = fixture();
        app.update_ui.tx.send(UpdateEvent::Checking).unwrap();
        app.update_ui
            .tx
            .send(UpdateEvent::CheckDeferred("network unavailable".to_owned()))
            .unwrap();
        app.poll_update_events(&ctx);
        assert!(app.notices.is_empty());
        assert!(app.update_ui.notice.is_none());
        assert!(app.update_ui.next_check.is_some());
        assert!(!app.update_ui.check_in_flight);
        assert!(app.update_ui.last_check_error.is_some());
        app.update_ui
            .tx
            .send(UpdateEvent::Failed("checksum mismatch".to_owned()))
            .unwrap();
        app.poll_update_events(&ctx);
        assert!(!app.notices.is_empty());
        assert!(app.update_ui.notice.is_some());
    }

    #[cfg(not(feature = "microsoft-store"))]
    #[test]
    fn manual_update_check_always_finishes_with_a_visible_result() {
        let (mut app, ctx, _rx) = fixture();
        app.update_ui.manual_check = true;
        app.update_ui.tx.send(UpdateEvent::Checking).unwrap();
        app.update_ui
            .tx
            .send(UpdateEvent::CheckDeferred("offline".to_owned()))
            .unwrap();
        app.poll_update_events(&ctx);
        assert!(!app.update_ui.manual_check);
        assert!(!app.update_ui.check_in_flight);
        assert!(app.update_ui.notice.is_some());
        app.update_ui.manual_check = true;
        app.update_ui.tx.send(UpdateEvent::NotAvailable).unwrap();
        app.poll_update_events(&ctx);
        assert!(app.update_ui.last_check_error.is_none());
        assert!(!app.update_ui.manual_check);
        assert!(app.update_ui.notice.is_some());
    }

    #[cfg(feature = "microsoft-store")]
    #[test]
    fn store_build_ignores_direct_update_events_without_disrupting_edits() {
        let (mut app, ctx, _rx) = fixture();
        app.add_text_box_annotation(0, PdfRect::new(10.0, 20.0, 200.0, 60.0));
        app.update_ui.tx.send(UpdateEvent::Checking).unwrap();
        app.update_ui.tx.send(UpdateEvent::Failed("direct channel must be ignored".to_owned())).unwrap();
        app.poll_update_events(&ctx);
        assert!(app.annotations_dirty);
        assert!(app.update_ui.notice.is_none());
        assert!(app.update_ui.next_check.is_none());
        assert!(!app.update_ui.check_in_flight);
        assert!(app.notices.is_empty());
    }
}
