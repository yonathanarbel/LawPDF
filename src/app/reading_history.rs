//! Local reading position, recent-document identity, and resume controls.

use super::*;
use crate::settings::{ReadingView, RecentDocument, WindowPlacement};

const RESUME_NOTICE_DURATION: Duration = Duration::from_secs(8);

pub(super) struct ResumeNotice {
    shown_at: Instant,
    document_epoch: u64,
    page: usize,
}

impl PdfEditorApp {
    pub(super) fn clear_reading_history(&mut self) {
        self.settings.recent_documents.clear();
        self.settings.last_session_paths.clear();
        self.settings.last_session_saved_unix = 0;
        // Closing this same window must not silently recreate what was cleared.
        self.session_history_suppressed = true;
        self.recent_suppressed_epoch = self.document.as_ref().map(|_| self.document_epoch);
        self.queue_settings_save();
        self.status = "Reading history cleared.".to_owned();
    }

    /// Record only after the close/save decision has succeeded. The active tab
    /// opens first, followed by the remaining tabs in their existing order.
    pub(super) fn remember_open_session(&mut self) {
        if !self.settings.reopen_last_session || self.session_history_suppressed {
            return;
        }
        let active = self.active_tab.filter(|index| *index < self.tabs.len());
        let paths = active
            .into_iter()
            .chain((0..self.tabs.len()).filter(|index| Some(*index) != active))
            .map(|index| {
                self.tabs[index]
                    .document
                    .path
                    .to_string_lossy()
                    .into_owned()
            })
            .collect::<Vec<_>>();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.settings.remember_session_paths(paths, now);
        self.queue_settings_save();
    }

    pub(super) fn remember_window_placement(&mut self, ctx: &Context) {
        let viewport = ctx.input(|input| input.viewport().clone());
        if viewport.minimized == Some(true) || viewport.fullscreen == Some(true) {
            return;
        }
        let previous = self.settings.window_placement.clone();
        let Some(inner) = viewport.inner_rect else {
            return;
        };
        let mut placement = previous.clone().unwrap_or(WindowPlacement {
            inner_size: [inner.width(), inner.height()],
            outer_position: None,
            maximized: false,
        });
        placement.maximized = viewport.maximized.unwrap_or(placement.maximized);
        if !placement.maximized {
            let size = [inner.width().round(), inner.height().round()];
            if !size[0].is_finite()
                || !size[1].is_finite()
                || !(720.0..=7680.0).contains(&size[0])
                || !(520.0..=4320.0).contains(&size[1])
            {
                return;
            }
            placement.inner_size = size;
            if let Some(outer) = viewport.outer_rect {
                let position = [outer.min.x.round(), outer.min.y.round()];
                if position
                    .iter()
                    .all(|value| value.is_finite() && value.abs() <= 16384.0)
                {
                    placement.outer_position = Some(position);
                }
            }
        }
        if previous.as_ref() != Some(&placement) {
            self.settings.window_placement = Some(placement);
            self.queue_settings_save();
            ctx.request_repaint_after(SETTINGS_SAVE_DEBOUNCE);
        }
    }

    pub(super) fn draw_recent_documents(&mut self, ui: &mut egui::Ui, ctx: &Context, rect: Rect) {
        let recent = self.settings.recent_documents.clone();
        let mut open = None;
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            ui.label(RichText::new("Recent documents").strong().size(15.0));
            egui::ScrollArea::vertical()
                .id_salt("recent-documents")
                .auto_shrink([false, false])
                .show_rows(ui, 62.0, recent.len(), |ui, rows| {
                    for index in rows {
                        let item = &recent[index];
                        let fallback = Path::new(&item.path)
                            .file_stem()
                            .and_then(|value| value.to_str())
                            .unwrap_or("Untitled PDF");
                        let title = if item.title.trim().is_empty() {
                            fallback
                        } else {
                            &item.title
                        };
                        let response = ui
                            .add(
                                egui::Label::new(RichText::new(title).strong().size(15.0))
                                    .truncate()
                                    .sense(Sense::click()),
                            )
                            .on_hover_cursor(CursorIcon::PointingHand)
                            .on_hover_text(&item.path);
                        if response.clicked() {
                            open = Some(PathBuf::from(&item.path));
                        }
                        let identity = [item.authors.trim(), item.citation.trim()]
                            .into_iter()
                            .filter(|s| !s.is_empty())
                            .collect::<Vec<_>>()
                            .join(" · ");
                        let detail = if identity.is_empty() {
                            format!("Page {}", item.page.saturating_add(1))
                        } else {
                            format!("{identity} · page {}", item.page.saturating_add(1))
                        };
                        ui.add(
                            egui::Label::new(RichText::new(detail).size(12.0).color(self.app_palette().muted))
                                .truncate(),
                        );
                        ui.add_space(5.0);
                    }
                });
        });
        if let Some(path) = open {
            self.open_paths_in_tabs(vec![path], ctx, true);
        }
    }

    pub(super) fn apply_pending_reading_resume(&mut self, ctx: &Context) {
        let Some(saved) = self.pending_reading_resume.take() else {
            return;
        };
        let Some(document) = self.document.as_ref() else {
            return;
        };
        let page = saved.page.min(document.page_count.saturating_sub(1));
        self.page_index = page;
        self.scroll_target_page = Some(page);
        self.thumbnail_scroll_target = Some(page);
        let explicit_start_view = std::env::var_os("LAWPDF_START_VIEW").is_some();
        let view = if explicit_start_view {
            self.view_mode
        } else {
            match saved.view {
                ReadingView::Pdf => DocumentViewMode::Pdf,
                ReadingView::Review => DocumentViewMode::LiquidMode2,
                ReadingView::LegacyReview => DocumentViewMode::Liquid,
            }
        };
        if view == DocumentViewMode::Pdf {
            self.pending_resume_fraction = Some((page, saved.page_fraction));
        } else {
            self.set_view_mode(view, ctx);
            self.pending_resume_review_block = saved.review_block;
        }
        if page > 0 || saved.page_fraction > 0.01 || view != DocumentViewMode::Pdf {
            self.resume_notice = Some(ResumeNotice {
                shown_at: Instant::now(),
                document_epoch: self.document_epoch,
                page,
            });
        }
        ctx.request_repaint();
    }

    pub(super) fn finish_pending_review_resume(&mut self, ctx: &Context) {
        let Some(block) = self.pending_resume_review_block else {
            return;
        };
        let (ready, complete) = match self.view_mode {
            DocumentViewMode::LiquidMode2 => (&self.liquid_mode2_state, self.liquid_mode2_complete),
            DocumentViewMode::Liquid => (&self.liquid_state, true),
            DocumentViewMode::Pdf => {
                self.pending_resume_review_block = None;
                return;
            }
        };
        let LiquidState::Ready(document) = ready else {
            return;
        };
        if block >= document.blocks.len() && !complete {
            self.request_full_review_prepare(ctx);
            return;
        }
        let mask = review_hidden_display_mask(&document.blocks);
        let target = document
            .blocks
            .iter()
            .enumerate()
            .filter(|(index, candidate)| {
                candidate.role != LiquidBlockRole::Metadata
                    && !is_review_note_display_block(candidate)
                    && !review_skips_block_as_furniture(
                        candidate,
                        mask.get(*index).copied().unwrap_or(false),
                    )
            })
            .map(|(index, _)| index)
            .min_by_key(|index| index.abs_diff(block));
        self.pending_resume_review_block = None;
        if let Some(target) = target {
            self.liquid_scroll_to_block = Some(target);
            self.liquid_scroll_instant = true;
            ctx.request_repaint();
        }
    }

    pub(super) fn remember_reading_position(&mut self, ctx: &Context) {
        if self.pending_reading_resume.is_some()
            || self.pending_resume_fraction.is_some()
            || self.pending_resume_review_block.is_some()
            || self.liquid_scroll_to_block.is_some()
            || self.recent_suppressed_epoch == Some(self.document_epoch)
        {
            return;
        }
        let Some(path) = self.document.as_ref().map(|doc| doc.path.clone()) else {
            return;
        };
        let prior = self
            .settings
            .recent_document_for_opened_path(&path)
            .cloned();
        let (page, fraction, view, review_block) = match self.view_mode {
            DocumentViewMode::Pdf => {
                let (page, fraction) = if self.sbs_mode.is_some() {
                    (
                        self.page_index,
                        prior
                            .as_ref()
                            .filter(|item| item.page == self.page_index)
                            .map_or(0.0, |item| item.page_fraction),
                    )
                } else {
                    let Some(visible) = self.visible_page_ranges.first() else {
                        return;
                    };
                    (
                        visible.page_index,
                        (visible.top_fraction.clamp(0.0, 1.0) * 200.0).round() / 200.0,
                    )
                };
                (
                    page,
                    fraction,
                    ReadingView::Pdf,
                    prior.as_ref().and_then(|item| item.review_block),
                )
            }
            DocumentViewMode::Liquid | DocumentViewMode::LiquidMode2 => {
                let state = match self.view_mode {
                    DocumentViewMode::Liquid => &self.liquid_state,
                    _ => &self.liquid_mode2_state,
                };
                let LiquidState::Ready(review) = state else {
                    return;
                };
                let view = if self.view_mode == DocumentViewMode::Liquid {
                    ReadingView::LegacyReview
                } else {
                    ReadingView::Review
                };
                let anchor = self
                    .search_state
                    .review_anchor
                    .or_else(|| prior.as_ref().and_then(|item| item.review_block));
                let page = anchor
                    .and_then(|anchor| {
                        review
                            .block_source_lines
                            .iter()
                            .find(|source| source.block_index == anchor)
                            .and_then(|source| source.lines.first())
                            .map(|line| line.page_index)
                    })
                    .unwrap_or_else(|| prior.as_ref().map_or(self.page_index, |item| item.page))
                    .min(
                        self.document
                            .as_ref()
                            .map_or(0, |document| document.page_count.saturating_sub(1)),
                    );
                (
                    page,
                    prior
                        .as_ref()
                        .filter(|item| item.page == page)
                        .map_or(0.0, |item| item.page_fraction),
                    view,
                    anchor,
                )
            }
        };
        let identity = self.masthead_identity_for_view();
        let mut record = RecentDocument {
            path: path.to_string_lossy().into_owned(),
            title: identity.title,
            authors: identity.authors,
            citation: identity.citation,
            page,
            page_fraction: fraction,
            view,
            review_block,
            last_seen_unix: prior.as_ref().map_or(0, |item| item.last_seen_unix),
        };
        let touch = self.recent_needs_touch;
        if !touch && prior.as_ref() == Some(&record) {
            return;
        }
        record.last_seen_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.recent_needs_touch = false;
        if self.settings.remember_recent_document(record, touch) {
            self.queue_settings_save();
            ctx.request_repaint_after(SETTINGS_SAVE_DEBOUNCE);
        }
    }

    pub(super) fn draw_resume_notice(&mut self, ctx: &Context) {
        let Some(notice) = self.resume_notice.as_ref() else {
            return;
        };
        let elapsed = notice.shown_at.elapsed();
        if notice.document_epoch != self.document_epoch || elapsed >= RESUME_NOTICE_DURATION {
            self.resume_notice = None;
            return;
        }
        let page = notice.page;
        let mut undo = false;
        egui::Area::new(egui::Id::new("reading-resume-notice"))
            .order(egui::Order::Foreground)
            .anchor(Align2::CENTER_TOP, Vec2::new(0.0, 76.0))
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(format!("Resumed at page {}", page + 1));
                        undo = ui.button("Undo").clicked();
                    });
                });
            });
        if undo {
            self.pending_resume_fraction = None;
            self.pending_resume_review_block = None;
            self.set_view_mode(DocumentViewMode::Pdf, ctx);
            self.go_to_page(0);
            self.resume_notice = None;
        } else {
            ctx.request_repaint_after(RESUME_NOTICE_DURATION - elapsed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::PageInfo;

    fn text_center(shape: &egui::Shape, text: &str) -> Option<Pos2> {
        match shape {
            egui::Shape::Text(shape) if shape.galley.text() == text => {
                Some(shape.pos + shape.galley.size() * 0.5)
            }
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| text_center(shape, text)),
            _ => None,
        }
    }

    fn input(time: f64, events: Vec<egui::Event>) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 700.0))),
            time: Some(time),
            events,
            ..Default::default()
        }
    }

    fn click_events(point: Pos2, pressed: bool) -> Vec<egui::Event> {
        vec![egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }]
    }

    #[test]
    fn session_saves_active_tab_first_only_on_clean_close_and_respects_clear_history() {
        let (mut app, _ctx, _worker) = annotation_saving::tests::fixture();
        app.settings.reopen_last_session = true;
        let first = app.tabs[0].document.path.clone();
        let second = first.with_file_name("second-session.pdf");
        let mut tab = app.tabs[0].clone();
        tab.document.path = second.clone();
        tab.document_epoch = 8;
        app.tabs.push(tab);
        app.active_tab = Some(1);
        app.remember_open_session();
        assert_eq!(
            app.settings.last_session_paths,
            vec![
                second.to_string_lossy().into_owned(),
                first.to_string_lossy().into_owned()
            ]
        );
        assert!(app.settings.last_session_saved_unix > 0);
        app.clear_reading_history();
        app.remember_open_session();
        assert!(app.settings.last_session_paths.is_empty());
        assert_eq!(app.settings.last_session_saved_unix, 0);
        assert!(app.session_history_suppressed);

        let (mut clean, ctx2, _worker2) = annotation_saving::tests::fixture();
        clean.settings.reopen_last_session = true;
        let _ = ctx2.run(Default::default(), |ctx| clean.request_window_close(ctx));
        assert!(clean.allow_window_close);
        assert_eq!(clean.settings.last_session_paths.len(), 1);
        let (mut dirty, ctx3, _worker3) = annotation_saving::tests::fixture();
        dirty.settings.reopen_last_session = true;
        dirty.annotations_dirty = true;
        let _ = ctx3.run(Default::default(), |ctx| dirty.request_window_close(ctx));
        assert!(!dirty.allow_window_close);
        assert!(
            dirty.settings.last_session_paths.is_empty(),
            "opening the unsaved prompt must not save a session"
        );
    }

    #[test]
    fn resume_restores_pdf_page_fraction_and_records_stable_position() {
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        let doc = app.document.as_mut().unwrap();
        doc.page_count = 3;
        doc.pages = vec![PageInfo::new(612.0, 792.0); 3];
        doc.native_text = vec!["one".into(), "two".into(), "three".into()];
        doc.native_text_loaded = vec![true; 3];
        doc.text_chars = vec![Some(Vec::new()); 3];
        doc.links = vec![Vec::new(); 3];
        let path = doc.path.to_string_lossy().into_owned();
        app.settings.sidebar_in_pdf = false;
        app.pending_reading_resume = Some(RecentDocument {
            path,
            title: "Article".into(),
            authors: String::new(),
            citation: String::new(),
            page: 1,
            page_fraction: 0.4,
            view: ReadingView::Pdf,
            review_block: None,
            last_seen_unix: 1,
        });
        for frame in 0..4 {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 600.0))),
                time: Some(frame as f64 * 0.2),
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                app.apply_pending_reading_resume(ctx);
                app.draw_document(ctx);
                app.remember_reading_position(ctx);
            });
        }
        let visible = app.visible_page_ranges.first().unwrap();
        assert_eq!(visible.page_index, 1);
        assert!(
            (visible.top_fraction - 0.4).abs() < 0.02,
            "restored fraction was {}",
            visible.top_fraction
        );
        assert!(app.resume_notice.is_some());
        let saved = app.settings.recent_documents.first().unwrap();
        assert_eq!(saved.page, 1);
        assert!((saved.page_fraction - 0.4).abs() < 0.02);
        app.settings_save_due = None;
        let _ = ctx.run(Default::default(), |ctx| app.remember_reading_position(ctx));
        assert!(
            app.settings_save_due.is_none(),
            "an unchanged reading position must not schedule writes"
        );
    }

    #[test]
    fn window_state_ignores_maximized_geometry_and_remembers_normal_size() {
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        let mut viewport = egui::ViewportInfo {
            inner_rect: Some(Rect::from_min_size(
                Pos2::new(100.0, 100.0),
                Vec2::new(1000.0, 700.0),
            )),
            outer_rect: Some(Rect::from_min_size(
                Pos2::new(90.0, 70.0),
                Vec2::new(1020.0, 740.0),
            )),
            maximized: Some(false),
            ..Default::default()
        };
        let mut frame = |viewport: egui::ViewportInfo, app: &mut PdfEditorApp| {
            let mut input = egui::RawInput::default();
            input.viewports.insert(egui::ViewportId::ROOT, viewport);
            let _ = ctx.run(input, |ctx| app.remember_window_placement(ctx));
        };
        frame(viewport.clone(), &mut app);
        viewport.maximized = Some(true);
        viewport.inner_rect = Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1920.0, 1080.0)));
        frame(viewport, &mut app);
        let saved = app.settings.window_placement.unwrap();
        assert_eq!(saved.inner_size, [1000.0, 700.0]);
        assert_eq!(saved.outer_position, Some([90.0, 70.0]));
        assert!(saved.maximized);
    }

    #[test]
    fn clicking_recent_identity_queues_that_document() {
        let (mut app, ctx, requests) = annotation_saving::tests::fixture();
        let path = app.document.as_ref().unwrap().path.clone();
        app.tabs.clear();
        app.active_tab = None;
        app.clear_document_state();
        app.settings.recent_documents = vec![RecentDocument {
            path: path.to_string_lossy().into_owned(),
            title: "A Recent Article".into(),
            authors: "Jane Author".into(),
            citation: "Journal 1 (2026)".into(),
            page: 0,
            page_fraction: 0.0,
            view: ReadingView::Pdf,
            review_block: None,
            last_seen_unix: 1,
        }];
        let draw = |app: &mut PdfEditorApp, time, events| {
            ctx.run(input(time, events), |ctx| app.draw_document(ctx))
        };
        draw(&mut app, 0.0, vec![]);
        let output = draw(&mut app, 0.05, vec![]);
        let point = output
            .shapes
            .iter()
            .find_map(|shape| text_center(&shape.shape, "A Recent Article"))
            .expect("recent document identity should be visible");
        draw(&mut app, 0.1, vec![egui::Event::PointerMoved(point)]);
        draw(&mut app, 0.2, click_events(point, true));
        draw(&mut app, 0.3, click_events(point, false));
        assert!(requests.try_iter().any(|request| matches!(request,
            RenderRequest::PrepareSources { paths, .. } if paths == vec![path.clone()])));
    }

    #[test]
    fn resume_notice_undo_returns_to_the_first_page() {
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        let doc = app.document.as_mut().unwrap();
        doc.page_count = 3;
        doc.pages = vec![PageInfo::new(612.0, 792.0); 3];
        let path = doc.path.to_string_lossy().into_owned();
        app.pending_reading_resume = Some(RecentDocument {
            path,
            title: "Article".into(),
            authors: String::new(),
            citation: String::new(),
            page: 2,
            page_fraction: 0.2,
            view: ReadingView::Pdf,
            review_block: None,
            last_seen_unix: 1,
        });
        app.apply_pending_reading_resume(&ctx);
        let draw = |app: &mut PdfEditorApp, time, events| {
            ctx.run(input(time, events), |ctx| app.draw_resume_notice(ctx))
        };
        draw(&mut app, 0.0, vec![]);
        let output = draw(&mut app, 0.05, vec![]);
        let point = output
            .shapes
            .iter()
            .find_map(|shape| text_center(&shape.shape, "Undo"))
            .expect("resume notice should offer Undo");
        draw(&mut app, 0.1, vec![egui::Event::PointerMoved(point)]);
        draw(&mut app, 0.2, click_events(point, true));
        draw(&mut app, 0.3, click_events(point, false));
        assert_eq!(app.page_index, 0);
        assert!(app.pending_resume_fraction.is_none());
        assert!(app.resume_notice.is_none());
    }

    #[test]
    fn review_resume_scrolls_to_the_remembered_block() {
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        let blocks = (0..40)
            .map(|index| LiquidBlock {
                role: LiquidBlockRole::Paragraph,
                text: format!(
                    "Paragraph {index}: {}",
                    "A long paragraph of ordinary article text. ".repeat(5)
                ),
                label: None,
            })
            .collect::<Vec<_>>();
        let document = LiquidDocument {
            title: "Article".into(),
            blocks,
            article_spans: Vec::new(),
            block_source_lines: Vec::new(),
            footnote_links: Vec::new(),
            footnote_link_integrity: None,
            profile: None,
            noise_lines_removed: 0,
            llm_used: false,
            llm_provider: None,
            deep_liquid_used: false,
            deep_liquid_model: None,
            warnings: Vec::new(),
            source_signature: "resume-review".into(),
        };
        app.view_mode = DocumentViewMode::LiquidMode2;
        app.liquid_mode2_state = LiquidState::Ready(Arc::new(document));
        app.liquid_mode2_complete = true;
        app.liquid_hide_footnotes = true;
        app.pending_resume_review_block = Some(20);
        for frame in 0..5 {
            let _ = ctx.run(input(frame as f64 * 0.2, vec![]), |ctx| {
                app.finish_pending_review_resume(ctx);
                egui::CentralPanel::default()
                    .show(ctx, |ui| app.draw_liquid_mode2_document(ui, ctx));
            });
        }
        let anchor = app
            .search_state
            .review_anchor
            .expect("a Review block should be visible");
        assert!(
            (18..=21).contains(&anchor),
            "Review resumed at block {anchor} rather than block 20"
        );
    }

    #[test]
    fn review_history_records_the_source_page_of_the_visible_block() {
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        app.document.as_mut().unwrap().page_count = 3;
        let review: LiquidDocument = serde_json::from_value(serde_json::json!({
            "title":"Article", "blocks":[{"role":"paragraph","text":"A passage"}],
            "block_source_lines":[{"block_index":0,"lines":[{"page_index":2,"line_index":0,"text":"A passage","role":"paragraph"}]}],
            "llm_used":false,"warnings":[],"source_signature":"source-page"
        })).unwrap();
        app.view_mode = DocumentViewMode::LiquidMode2;
        app.liquid_mode2_state = LiquidState::Ready(Arc::new(review));
        app.search_state.review_anchor = Some(0);
        let _ = ctx.run(input(0.0, vec![]), |ctx| app.remember_reading_position(ctx));
        let saved = app.settings.recent_documents.first().unwrap();
        assert_eq!(saved.page, 2);
        assert_eq!(saved.review_block, Some(0));
        assert_eq!(saved.view, ReadingView::Review);
    }
}
