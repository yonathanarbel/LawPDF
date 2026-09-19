use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CloseTarget {
    Window,
    Tab(u64),
}

impl PdfEditorApp {
    fn discard_close_target(&mut self, target: CloseTarget) -> Result<(), String> {
        let store = crate::document_store::DocumentStore::new()?;
        for tab in &self.tabs {
            if tab.annotations_dirty
                && (target == CloseTarget::Window || target == CloseTarget::Tab(tab.document_epoch))
            {
                store.discard(&tab.document.path)?;
            }
        }
        Ok(())
    }

    fn save_close_target(&mut self, target: CloseTarget) -> Result<(), String> {
        let CloseTarget::Tab(epoch) = target else {
            return self.save_all_dirty_annotations();
        };
        self.save_active_tab_state();
        let Some(index) = self.tabs.iter().position(|tab| tab.document_epoch == epoch) else {
            return Ok(());
        };
        if self.active_tab == Some(index) {
            return self.save_current_annotations();
        }
        let tab = &self.tabs[index];
        self.queue_annotation_save(epoch, tab.document.path.clone(), tab.annotations.clone())
    }

    fn finish_close_target(&mut self, target: CloseTarget, ctx: &Context) {
        self.pending_close = None;
        self.pending_annotation_saves
            .retain(|_, save| match target {
                CloseTarget::Window => false,
                CloseTarget::Tab(epoch) => save.document_epoch != epoch,
            });
        match target {
            CloseTarget::Window => {
                self.allow_window_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            CloseTarget::Tab(epoch) => {
                if let Some(index) = self.tabs.iter().position(|tab| tab.document_epoch == epoch) {
                    self.close_tab_without_saving(index, ctx);
                }
            }
        }
    }

    pub(super) fn draw_unsaved_close_prompt(&mut self, ctx: &Context) {
        let Some(target) = self.pending_close else {
            return;
        };
        self.save_active_tab_state();
        if !self.close_target_is_dirty(target) && !self.close_target_is_saving(target) {
            self.finish_close_target(target, ctx);
            return;
        }
        if self.close_target_is_saving(target) {
            for save in self.pending_annotation_saves.values_mut() {
                if target == CloseTarget::Window || target == CloseTarget::Tab(save.document_epoch)
                {
                    save.due_at = Instant::now();
                }
            }
            self.start_due_annotation_saves(ctx);
            egui::Modal::new(egui::Id::new("unsaved_changes")).show(ctx, |ui| {
                ui.heading("Saving annotations before closing…");
                ui.spinner();
                if ui.button("Cancel close").clicked() {
                    self.pending_close = None;
                }
            });
            ctx.request_repaint_after(RENDER_POLL_INTERVAL);
            return;
        }
        let title = match target {
            CloseTarget::Window => "Save changes before quitting?".to_owned(),
            CloseTarget::Tab(epoch) => {
                let Some(tab) = self.tabs.iter().find(|tab| tab.document_epoch == epoch) else {
                    self.pending_close = None;
                    return;
                };
                format!("Save changes to {}?", tab.title())
            }
        };
        egui::Modal::new(egui::Id::new("unsaved_changes")).show(ctx, |ui| {
            ui.heading(title);
            ui.label(
                "Automatic saving did not finish. Your annotations have not been saved to the PDF.",
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("Save and close").clicked() {
                    match self.save_close_target(target) {
                        Ok(()) => ctx.request_repaint(),
                        Err(error) => self.push_error_notice(error),
                    }
                }
                if ui.button("Don't save").clicked() {
                    match self.discard_close_target(target) {
                        Ok(()) => self.finish_close_target(target, ctx),
                        Err(error) => self.push_error_notice(error),
                    }
                }
                if ui.button("Cancel").clicked() {
                    self.pending_close = None;
                }
            });
        });
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.pending_close = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::PageInfo;

    fn app_with_dirty_tab() -> (PdfEditorApp, Context) {
        let ctx = Context::default();
        let mut app = PdfEditorApp::new(
            &ctx,
            Vec::new(),
            unbounded().1,
            #[cfg(target_os = "macos")]
            None,
        );
        let document = LoadedDocument {
            path: std::env::temp_dir()
                .join(format!("lawpdf-close-missing-{}.pdf", std::process::id())),
            title: "Unsaved document".to_owned(),
            page_count: 1,
            pages: vec![PageInfo::new(612.0, 792.0)],
            native_text: vec![String::new()],
            native_text_loaded: vec![true],
            text_chars: vec![None],
            links: vec![Vec::new()],
            links_loaded: false,
            optimized: true,
        };
        app.document = Some(document.clone());
        app.document_epoch = 7;
        app.annotations_dirty = true;
        app.active_tab = Some(0);
        app.tabs.push(app.active_tab_snapshot(document));
        (app, ctx)
    }

    #[test]
    fn closing_a_dirty_tab_waits_for_a_decision_and_failed_save_keeps_it_open() {
        let (mut app, ctx) = app_with_dirty_tab();
        app.close_tab(0, &ctx);
        assert_eq!(app.pending_close, Some(CloseTarget::Tab(7)));
        assert_eq!(app.tabs.len(), 1);
        assert!(app.annotations_dirty);
        assert!(app.save_close_target(CloseTarget::Tab(7)).is_err());
        assert_eq!(app.tabs.len(), 1);
        assert!(app.annotations_dirty);
        assert_eq!(app.pending_close, Some(CloseTarget::Tab(7)));
        app.pending_close = None;
        assert_eq!(app.tabs.len(), 1, "Cancel leaves the document open");
        app.close_tab(0, &ctx);
        app.finish_close_target(CloseTarget::Tab(7), &ctx);
        assert!(
            app.tabs.is_empty(),
            "explicit discard closes the requested tab"
        );
        assert!(
            !app.allow_window_close,
            "closing the last tab does not quit the app"
        );
    }

    #[test]
    fn escape_dismisses_the_rendered_close_modal_without_losing_changes() {
        let (mut app, ctx) = app_with_dirty_tab();
        app.close_tab(0, &ctx);
        let first = ctx.run(egui::RawInput::default(), |ctx| {
            app.draw_unsaved_close_prompt(ctx)
        });
        assert!(!first.shapes.is_empty());
        let input = egui::RawInput {
            events: vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| app.draw_unsaved_close_prompt(ctx));
        assert!(app.pending_close.is_none());
        assert_eq!(app.tabs.len(), 1);
        assert!(app.annotations_dirty);
    }

    #[test]
    fn close_prompt_targets_the_original_tab_even_when_indices_change() {
        let (mut app, ctx) = app_with_dirty_tab();
        app.close_tab(0, &ctx);
        let mut other = app.tabs[0].clone();
        other.document_epoch = 9;
        other.annotations_dirty = false;
        app.tabs.insert(0, other);
        app.active_tab = Some(0);
        app.document_epoch = 9;
        app.annotations_dirty = false;
        app.finish_close_target(CloseTarget::Tab(7), &ctx);
        assert_eq!(app.tabs.len(), 1);
        assert_eq!(app.tabs[0].document_epoch, 9);
        assert!(app.pending_close.is_none());
    }
}
