use super::*;
use crate::render_worker::OpenedDocument;

pub(super) struct OpenOptions {
    activate: bool,
    prefetch: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_app() -> (
        PdfEditorApp,
        Context,
        crossbeam_channel::Receiver<RenderRequest>,
    ) {
        let ctx = Context::default();
        let mut app = PdfEditorApp::new(
            &ctx,
            Vec::new(),
            crossbeam_channel::unbounded().1,
            #[cfg(target_os = "macos")]
            None,
        );
        let (sender, receiver) = crossbeam_channel::unbounded();
        app.render_tx = sender.into();
        (app, ctx, receiver)
    }

    fn preview() -> PreparedPage {
        PreparedPage::try_from(RenderedPage {
            page_index: 0,
            width: 2,
            height: 2,
            rgba: vec![255; 16],
        })
        .unwrap()
    }

    #[test]
    fn opening_preview_is_read_only_and_clears_on_open_failure() {
        let source = tempfile::NamedTempFile::new().unwrap();
        let path = source.path().canonicalize().unwrap();
        let (mut app, ctx, requests) = empty_app();
        assert!(app.load_document_with_options(path.clone(), &ctx, true, true, false));
        assert!(matches!(
            requests.try_recv().unwrap(),
            RenderRequest::OpenDocument {
                preview_page: Some(0),
                ..
            }
        ));
        let _ = ctx.run(Default::default(), |ctx| {
            app.accept_opening_preview(path.clone(), preview(), ctx)
        });
        assert!(app.opening_preview.is_some());
        assert!(app.document.is_none());
        assert!(app.tabs.is_empty());
        assert!(app.annotation_sessions.is_empty());
        let preview_texture = app.opening_preview.as_ref().unwrap().texture.id();
        app.settings.dim_original_pages = true;
        let preview_output = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| app.draw_opening_preview(ui));
        });
        assert!(preview_output.shapes.iter().any(|shape| matches!(
            &shape.shape,
            egui::Shape::Mesh(mesh)
                if mesh.texture_id == preview_texture
                    && mesh.vertices[0].color == Color32::from_rgb(190, 180, 162)
        )));
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 700.0))),
            ..Default::default()
        };
        let output = ctx.run(input, |ctx| app.draw_document(ctx));
        fn has_message(shape: &egui::Shape) -> bool {
            match shape {
                egui::Shape::Text(text) => {
                    text.galley.text().contains("Editing available when ready")
                }
                egui::Shape::Vec(shapes) => shapes.iter().any(has_message),
                _ => false,
            }
        }
        assert!(output.shapes.iter().any(|shape| has_message(&shape.shape)));
        let _ = ctx.run(Default::default(), |ctx| {
            app.finish_document_open(path.clone(), Err("test failure".into()), ctx)
        });
        assert!(app.opening_preview.is_none());
        assert!(app.document.is_none());
        assert!(app.tabs.is_empty());
    }

    #[test]
    fn open_failure_offers_an_accessible_retry_for_the_same_file() {
        let source = tempfile::NamedTempFile::new().unwrap();
        let path = source.path().canonicalize().unwrap();
        let (mut app, ctx, requests) = empty_app();
        assert!(app.load_document_with_options(path.clone(), &ctx, true, true, false));
        assert!(matches!(
            requests.try_recv().unwrap(),
            RenderRequest::OpenDocument { .. }
        ));
        let _ = ctx.run(Default::default(), |ctx| {
            app.finish_document_open(
                path.clone(),
                Err("The file is temporarily unavailable.".into()),
                ctx,
            );
        });
        assert!(app.document.is_none());
        assert!(app.pending_document_opens.is_empty());
        assert!(
            app.notices[0]
                .message
                .starts_with("Could not open this PDF:")
        );
        ctx.enable_accesskit();
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 700.0));
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(screen),
                ..Default::default()
            },
            |ctx| {
                app.draw_notices(ctx);
            },
        );
        let tree = output.platform_output.accesskit_update.unwrap();
        let target = tree
            .nodes
            .iter()
            .find_map(|(id, node)| (node.label() == Some("Retry opening")).then_some(*id))
            .expect("accessible retry action");
        let input = egui::RawInput {
            screen_rect: Some(screen),
            events: vec![egui::Event::AccessKitActionRequest(
                egui::accesskit::ActionRequest {
                    action: egui::accesskit::Action::Click,
                    target,
                    data: None,
                },
            )],
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| app.draw_notices(ctx));
        match requests.try_recv().expect("retry request") {
            RenderRequest::OpenDocument {
                path: retried,
                preview_page,
                ..
            } => {
                assert_eq!(retried, path);
                assert_eq!(preview_page, Some(0));
            }
            _ => panic!("retry did not request a document open"),
        }
        assert!(app.pending_document_opens.contains_key(&path));
        assert!(app.notices.is_empty());
        assert!(app.startup_error.is_none());
    }

    #[test]
    fn failed_background_open_keeps_its_retry_options() {
        let source = tempfile::NamedTempFile::new().unwrap();
        let path = source.path().canonicalize().unwrap();
        let (mut app, ctx, _requests) = empty_app();
        assert!(app.load_document_with_options(path.clone(), &ctx, false, false, true));
        let _ = ctx.run(Default::default(), |ctx| {
            app.finish_document_open(path.clone(), Err("File unavailable.".into()), ctx);
        });
        assert!(matches!(
            &app.notices[0].action,
            Some(NoticeAction::RetryOpen { path: retry_path, activate: false, prefetch: true })
                if retry_path == &path
        ));
    }

    #[test]
    fn stale_invalid_and_background_previews_are_ignored() {
        let first = tempfile::NamedTempFile::new().unwrap();
        let second = tempfile::NamedTempFile::new().unwrap();
        let first = first.path().canonicalize().unwrap();
        let second = second.path().canonicalize().unwrap();
        let (mut app, ctx, _requests) = empty_app();
        assert!(app.load_document_with_options(first.clone(), &ctx, true, true, false));
        assert!(app.load_document_with_options(second.clone(), &ctx, true, true, false));
        let _ = ctx.run(Default::default(), |ctx| {
            app.accept_opening_preview(first.clone(), preview(), ctx);
            assert!(
                PreparedPage::try_from(RenderedPage {
                    page_index: 0,
                    width: 2,
                    height: 2,
                    rgba: vec![0; 3]
                })
                .is_err()
            );
        });
        assert!(app.opening_preview.is_none());
        let _ = ctx.run(Default::default(), |ctx| {
            app.accept_opening_preview(second.clone(), preview(), ctx)
        });
        assert!(app.opening_preview.is_some());
        let (with_document, _ctx, _worker) = annotation_saving::tests::fixture();
        app.document = with_document.document;
        app.opening_preview = None;
        let _ = ctx.run(Default::default(), |ctx| {
            app.accept_opening_preview(second.clone(), preview(), ctx)
        });
        assert!(app.opening_preview.is_none());
    }
}

pub(super) struct OpeningPreview {
    texture: egui::TextureHandle,
    size: Vec2,
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
        if activate {
            self.first_page_open_started = Some(Instant::now());
        }
        let path = std::fs::canonicalize(&path).unwrap_or(path);
        if activate {
            self.opening_preview_target = Some(path.clone());
            self.opening_preview = None;
        }
        if let Some(tab_index) = self.tab_index_for_path(&path) {
            if activate {
                self.opening_preview_target = None;
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
            preview_page: (activate && self.document.is_none()).then(|| {
                self.settings
                    .recent_document_for_opened_path(&path)
                    .map_or(0, |recent| recent.page)
            }),
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
                if self.opening_preview_target.as_ref() == Some(&path) {
                    self.opening_preview_target = None;
                    self.opening_preview = None;
                }
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
        if self.opening_preview_target.as_ref() == Some(&path) {
            self.opening_preview_target = None;
            // The preview predates the verified snapshot and may depict an
            // older source revision. Never reuse it after editing is enabled.
            self.opening_preview = None;
        }
        let opened = match result {
            Ok(opened) => opened,
            Err(error) => {
                self.open_into_sbs = false;
                let message = format!("Could not open this PDF: {error}");
                self.startup_error = Some(message.clone());
                self.push_retry_open_notice(message, path, options.activate, options.prefetch);
                return;
            }
        };
        let tab = match self.tab_for_new_document(opened) {
            Ok(tab) => tab,
            Err(error) => {
                self.open_into_sbs = false;
                self.push_retry_open_notice(
                    format!("Could not open this PDF: {error}"),
                    path,
                    options.activate,
                    options.prefetch,
                );
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
        self.render_tx
            .set_live_documents(self.tabs.iter().map(|tab| tab.document_epoch));
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

    pub(super) fn accept_opening_preview(
        &mut self,
        path: PathBuf,
        page: PreparedPage,
        ctx: &Context,
    ) {
        if self.document.is_some()
            || self.opening_preview_target.as_ref() != Some(&path)
            || !self
                .pending_document_opens
                .get(&path)
                .is_some_and(|options| options.activate)
        {
            return;
        }
        let texture = ctx.load_texture(
            "opening-pdf-preview",
            page.image,
            egui::TextureOptions::LINEAR,
        );
        self.opening_preview = Some(OpeningPreview {
            texture,
            size: Vec2::new(page.width as f32, page.height as f32),
        });
        if let Some(started) = self.first_page_open_started {
            crate::diagnostics::timing(
                "document_first_preview",
                "request_to_preview",
                started.elapsed(),
            );
        }
        ctx.request_repaint();
    }

    pub(super) fn draw_opening_preview(&self, ui: &mut egui::Ui) {
        let Some(preview) = &self.opening_preview else {
            return;
        };
        let area = ui.available_rect_before_wrap().shrink(18.0);
        let available = Vec2::new(area.width().max(1.0), (area.height() - 36.0).max(1.0));
        let scale = (available.x / preview.size.x).min(available.y / preview.size.y);
        let size = preview.size * scale;
        let rect =
            Rect::from_center_size(Pos2::new(area.center().x, area.top() + size.y * 0.5), size);
        ui.painter().image(
            preview.texture.id(),
            rect,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            self.original_page_tint(),
        );
        ui.painter().text(
            Pos2::new(area.center().x, rect.bottom() + 18.0),
            Align2::CENTER_CENTER,
            "Opening PDF… Editing available when ready",
            FontId::proportional(13.0),
            self.app_palette().muted,
        );
    }
}
