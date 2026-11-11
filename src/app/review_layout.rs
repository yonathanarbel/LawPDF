//! Retained geometry for static Review rows.
use super::*;

static NEXT_LAYOUT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

pub(super) fn next_layout_id() -> u64 {
    NEXT_LAYOUT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

#[derive(Clone, Copy, Debug)]
pub(super) enum RowKind {
    Block(usize),
    Metadata { start: usize, end: usize },
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Row {
    pub(super) top: f32,
    pub(super) bottom: f32,
    pub(super) kind: RowKind,
}

pub(super) struct ReviewLayout {
    pub(super) id: u64,
    pub(super) active_this_frame: bool,
    pub(super) document: std::sync::Weak<LiquidDocument>,
    pub(super) width: u32,
    pub(super) observed_width: u32,
    pub(super) width_changed_at: Option<f64>,
    pub(super) resize_factor: f32,
    pub(super) previous_resize_factor: f32,
    pub(super) resize_anchor: Option<usize>,
    pub(super) indent: u32,
    pub(super) scale: u32,
    pub(super) pixels_per_point: u32,
    pub(super) theme: LiquidTheme,
    pub(super) hide_notes: bool,
    pub(super) has_margin: bool,
    pub(super) origin: Pos2,
    pub(super) height: f32,
    pub(super) rows: Vec<Row>,
    pub(super) margin: Option<ReviewMarginCollector>,
}

impl ReviewLayout {
    pub(super) fn is_compatible(
        &self,
        document: &Arc<LiquidDocument>,
        _width: f32,
        _indent: f32,
        scale: f32,
        pixels_per_point: f32,
        theme: LiquidTheme,
        hide_notes: bool,
        has_margin: bool,
    ) -> bool {
        self.document
            .upgrade()
            .is_some_and(|cached| Arc::ptr_eq(&cached, document))
            && self.scale == scale.to_bits()
            && self.pixels_per_point == pixels_per_point.to_bits()
            && self.theme == theme
            && self.hide_notes == hide_notes
            && self.has_margin == has_margin
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find_text(shape: &egui::Shape, wanted: &str) -> Option<Pos2> {
        match shape {
            egui::Shape::Text(text) if text.galley.text() == wanted => Some(text.pos),
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| find_text(shape, wanted)),
            _ => None,
        }
    }

    fn find_text_containing(shape: &egui::Shape, wanted: &str) -> Option<Pos2> {
        match shape {
            egui::Shape::Text(text) if text.galley.text().contains(wanted) => Some(text.pos),
            egui::Shape::Vec(shapes) => shapes
                .iter()
                .find_map(|shape| find_text_containing(shape, wanted)),
            _ => None,
        }
    }

    fn document(blocks: Vec<LiquidBlock>) -> LiquidDocument {
        LiquidDocument {
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
            source_signature: "retained-test".into(),
        }
    }

    fn input(time: f64, events: Vec<egui::Event>) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1100.0, 800.0))),
            time: Some(time),
            events,
            ..Default::default()
        }
    }

    #[test]
    fn retained_rows_keep_visible_text_at_the_measured_positions() {
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        let blocks = (0..40)
            .map(|index| LiquidBlock {
                role: LiquidBlockRole::Paragraph,
                text: format!("Paragraph {index}: {}", "Ordinary article text. ".repeat(6)),
                label: None,
            })
            .collect::<Vec<_>>();
        let text = blocks[0].text.clone();
        let document = document(blocks);
        app.view_mode = DocumentViewMode::LiquidMode2;
        app.liquid_mode2_state = LiquidState::Ready(Arc::new(document));
        app.liquid_mode2_complete = true;
        app.liquid_hide_footnotes = true;
        let mut draw = |time: f64, app: &mut PdfEditorApp| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1100.0, 800.0))),
                    time: Some(time),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default()
                        .show(ctx, |ui| app.draw_liquid_mode2_document(ui, ctx));
                },
            )
        };
        let first = draw(0.0, &mut app);
        let measured = app
            .review_layout
            .as_ref()
            .expect("first frame should retain geometry")
            .rows
            .as_ptr();
        let second = draw(0.2, &mut app);
        assert_eq!(
            measured,
            app.review_layout.as_ref().unwrap().rows.as_ptr(),
            "the second frame should reuse the retained layout"
        );
        let pos = |output: &egui::FullOutput| {
            output
                .shapes
                .iter()
                .find_map(|shape| find_text(&shape.shape, &text))
        };
        let a = pos(&first).expect("first paragraph should be visible");
        let b = pos(&second).expect("retained first paragraph should be visible");
        assert!((a - b).length() < 0.5, "text moved from {a:?} to {b:?}");
    }

    #[test]
    fn retained_rows_keep_footnote_positions() {
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        let mut blocks = Vec::new();
        for number in 1..=20 {
            blocks.push(LiquidBlock {
                role: LiquidBlockRole::Paragraph,
                text: format!(
                    "The first paragraph cites{}{}{} an authority.",
                    crate::layout_roles::CALLOUT_START,
                    number,
                    crate::layout_roles::CALLOUT_END
                ),
                label: None,
            });
            blocks.push(LiquidBlock {
                role: LiquidBlockRole::Footnote,
                text: format!("{number}. Footnote body {number} explains this."),
                label: None,
            });
        }
        app.view_mode = DocumentViewMode::LiquidMode2;
        app.liquid_mode2_state = LiquidState::Ready(Arc::new(document(blocks)));
        app.liquid_mode2_complete = true;
        let draw = |time, app: &mut PdfEditorApp| {
            ctx.run(input(time, vec![]), |ctx| {
                egui::CentralPanel::default()
                    .show(ctx, |ui| app.draw_liquid_mode2_document(ui, ctx));
            })
        };
        let first = draw(0.0, &mut app);
        let retained = app.review_layout.as_ref().unwrap();
        assert!(retained.margin.is_some());
        let rows = retained.rows.as_ptr();
        let second = draw(0.2, &mut app);
        assert_eq!(rows, app.review_layout.as_ref().unwrap().rows.as_ptr());
        let position = |output: &egui::FullOutput| {
            output.shapes.iter().find_map(|shape| {
                find_text_containing(&shape.shape, "Footnote body 1 explains this.")
            })
        };
        let before = position(&first).expect("first footnote should be visible");
        let after = position(&second).expect("retained footnote should be visible");
        assert!(
            (before - after).length() < 0.5,
            "footnote moved from {before:?} to {after:?}"
        );
    }

    #[test]
    fn clicking_a_retained_margin_note_opens_it() {
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        let blocks = vec![
            LiquidBlock {
                role: LiquidBlockRole::Paragraph,
                text: format!(
                    "A paragraph cites{}1{} the note.",
                    crate::layout_roles::CALLOUT_START,
                    crate::layout_roles::CALLOUT_END
                ),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Footnote,
                text: "1. Footnote body is here.".into(),
                label: None,
            },
        ];
        app.view_mode = DocumentViewMode::LiquidMode2;
        app.liquid_mode2_state = LiquidState::Ready(Arc::new(document(blocks)));
        app.liquid_mode2_complete = true;
        let draw = |time, events, app: &mut PdfEditorApp| {
            ctx.run(input(time, events), |ctx| {
                egui::CentralPanel::default()
                    .show(ctx, |ui| app.draw_liquid_mode2_document(ui, ctx));
            })
        };
        draw(0.0, vec![], &mut app);
        let output = draw(0.1, vec![], &mut app);
        let point = output
            .shapes
            .iter()
            .find_map(|shape| find_text_containing(&shape.shape, "Footnote body is here."))
            .expect("footnote should be visible")
            + Vec2::new(30.0, 8.0);
        draw(0.2, vec![egui::Event::PointerMoved(point)], &mut app);
        draw(
            0.3,
            vec![egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
            &mut app,
        );
        draw(
            0.4,
            vec![egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
            &mut app,
        );
        assert_eq!(app.chrome.expanded_note.as_deref(), Some("1"));
    }

    #[test]
    fn retained_notes_match_full_layout_after_scrolling() {
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        let mut blocks = Vec::new();
        for number in 1..=40 {
            blocks.push(LiquidBlock {
                role: LiquidBlockRole::Paragraph,
                text: format!(
                    "Paragraph {number}: {}{}{} {}",
                    crate::layout_roles::CALLOUT_START,
                    number,
                    crate::layout_roles::CALLOUT_END,
                    "Some ordinary article text. ".repeat(5)
                ),
                label: None,
            });
            blocks.push(LiquidBlock {
                role: LiquidBlockRole::Footnote,
                text: format!("{number}. Note {number} provides a citation."),
                label: None,
            });
        }
        app.view_mode = DocumentViewMode::LiquidMode2;
        app.liquid_mode2_state = LiquidState::Ready(Arc::new(document(blocks)));
        app.liquid_mode2_complete = true;
        let draw = |time, events, app: &mut PdfEditorApp| {
            ctx.run(input(time, events), |ctx| {
                egui::CentralPanel::default()
                    .show(ctx, |ui| app.draw_liquid_mode2_document(ui, ctx));
            })
        };
        draw(0.0, vec![], &mut app);
        draw(0.1, vec![], &mut app);
        app.liquid_scroll_to_block = Some(20);
        app.liquid_scroll_instant = true;
        draw(0.2, vec![], &mut app);
        draw(0.4, vec![], &mut app);
        let retained = draw(0.6, vec![], &mut app);
        let mut visible = None;
        for number in 2..=20 {
            let text = format!("Note {number} provides a citation.");
            if let Some(position) = retained
                .shapes
                .iter()
                .find_map(|shape| find_text_containing(&shape.shape, &text))
            {
                visible = Some((text, position));
                break;
            }
        }
        let (text, position) = visible.expect("a later note should be visible after scrolling");
        app.review_layout = None;
        app.review_margin_rows = None;
        let full = draw(0.8, vec![], &mut app);
        let expected = full
            .shapes
            .iter()
            .find_map(|shape| find_text_containing(&shape.shape, &text))
            .expect("the same note should be visible in the full layout");
        assert!(
            (position - expected).length() < 0.75,
            "retained note moved from {expected:?} to {position:?}"
        );
    }

    #[test]
    fn retained_notes_follow_a_margin_resize_after_scrolling() {
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        let mut blocks = Vec::new();
        for number in 1..=40 {
            blocks.push(LiquidBlock {
                role: LiquidBlockRole::Paragraph,
                text: format!(
                    "Paragraph {number}: {}{}{} {}",
                    crate::layout_roles::CALLOUT_START,
                    number,
                    crate::layout_roles::CALLOUT_END,
                    "Some ordinary article text. ".repeat(5)
                ),
                label: None,
            });
            blocks.push(LiquidBlock {
                role: LiquidBlockRole::Footnote,
                text: format!("{number}. Note {number} provides a citation."),
                label: None,
            });
        }
        app.view_mode = DocumentViewMode::LiquidMode2;
        app.liquid_mode2_state = LiquidState::Ready(Arc::new(document(blocks)));
        app.liquid_mode2_complete = true;
        let draw = |time, events, width, app: &mut PdfEditorApp| {
            let mut raw = input(time, events);
            raw.screen_rect = Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(width, 800.0)));
            ctx.run(raw, |ctx| {
                egui::CentralPanel::default()
                    .show(ctx, |ui| app.draw_liquid_mode2_document(ui, ctx));
            })
        };
        draw(0.0, vec![], 1300.0, &mut app);
        draw(0.1, vec![], 1300.0, &mut app);
        app.liquid_scroll_to_block = Some(20);
        app.liquid_scroll_instant = true;
        draw(0.2, vec![], 1300.0, &mut app);
        draw(0.4, vec![], 1300.0, &mut app);
        draw(0.6, vec![], 1300.0, &mut app);
        let cached_id = app.review_layout.as_ref().unwrap().id;
        let resized = draw(0.8, vec![], 1250.0, &mut app);
        assert_eq!(
            cached_id,
            app.review_layout.as_ref().unwrap().id,
            "body width stays fixed while its margins resize"
        );
        let mut visible = None;
        for number in 2..=20 {
            let text = format!("Note {number} provides a citation.");
            if let Some(position) = resized
                .shapes
                .iter()
                .find_map(|shape| find_text_containing(&shape.shape, &text))
            {
                visible = Some((text, position));
                break;
            }
        }
        let (text, position) = visible.expect("a later note should be visible after scrolling");
        app.review_layout = None;
        app.review_margin_rows = None;
        let full = draw(1.0, vec![], 1250.0, &mut app);
        let expected = full
            .shapes
            .iter()
            .find_map(|shape| find_text_containing(&shape.shape, &text))
            .expect("the same note should be visible in the full layout");
        assert!(
            (position - expected).length() < 0.75,
            "resized note moved from {expected:?} to {position:?}"
        );
    }

    #[test]
    fn live_reflow_reuses_geometry_then_rebuilds_after_resize_settles() {
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        let blocks = (0..40)
            .map(|index| LiquidBlock {
                role: LiquidBlockRole::Paragraph,
                text: format!(
                    "Paragraph {index}: {}",
                    "Text that wraps with the column width. ".repeat(8)
                ),
                label: None,
            })
            .collect::<Vec<_>>();
        app.view_mode = DocumentViewMode::LiquidMode2;
        app.liquid_mode2_state = LiquidState::Ready(Arc::new(document(blocks)));
        app.liquid_mode2_complete = true;
        app.liquid_hide_footnotes = true;
        let draw = |time, width, app: &mut PdfEditorApp| {
            let mut raw = input(time, vec![]);
            raw.screen_rect = Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(width, 800.0)));
            ctx.run(raw, |ctx| {
                egui::CentralPanel::default()
                    .show(ctx, |ui| app.draw_liquid_mode2_document(ui, ctx));
            })
        };
        draw(0.0, 760.0, &mut app);
        draw(0.1, 760.0, &mut app);
        let id = app.review_layout.as_ref().unwrap().id;
        for (time, width) in [(0.2, 740.0), (0.3, 720.0), (0.4, 700.0)] {
            draw(time, width, &mut app);
            assert_eq!(
                app.review_layout.as_ref().unwrap().id,
                id,
                "live resize should reuse the document layout"
            );
        }
        draw(0.6, 700.0, &mut app);
        assert_ne!(
            app.review_layout.as_ref().unwrap().id,
            id,
            "the settled width must get an exact layout"
        );
        assert_eq!(app.review_layout.as_ref().unwrap().resize_factor, 1.0);
    }

    #[test]
    fn resizing_a_scrolled_article_keeps_the_nearby_passage_in_view() {
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        let blocks = (0..80)
            .map(|index| LiquidBlock {
                role: LiquidBlockRole::Paragraph,
                text: format!(
                    "Paragraph {index}: {}",
                    "Text that wraps with the column width. ".repeat(8)
                ),
                label: None,
            })
            .collect::<Vec<_>>();
        app.view_mode = DocumentViewMode::LiquidMode2;
        app.liquid_mode2_state = LiquidState::Ready(Arc::new(document(blocks)));
        app.liquid_mode2_complete = true;
        app.liquid_hide_footnotes = true;
        let draw = |time, width, app: &mut PdfEditorApp| {
            let mut raw = input(time, vec![]);
            raw.screen_rect = Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(width, 800.0)));
            ctx.run(raw, |ctx| {
                egui::CentralPanel::default()
                    .show(ctx, |ui| app.draw_liquid_mode2_document(ui, ctx));
            })
        };
        draw(0.0, 760.0, &mut app);
        draw(0.1, 760.0, &mut app);
        app.liquid_scroll_to_block = Some(40);
        app.liquid_scroll_instant = true;
        draw(0.2, 760.0, &mut app);
        draw(0.4, 760.0, &mut app);
        draw(0.6, 760.0, &mut app);
        let before = app.search_state.review_anchor.unwrap();
        for (time, width) in [
            (0.8, 740.0),
            (0.9, 720.0),
            (1.0, 700.0),
            (1.2, 700.0),
            (1.4, 700.0),
            (1.6, 700.0),
        ] {
            draw(time, width, &mut app);
        }
        let after = app.search_state.review_anchor.unwrap();
        assert!(
            before.abs_diff(after) <= 3,
            "resize moved the visible passage from block {before} to {after}"
        );
    }

    #[test]
    fn selecting_across_visible_paragraphs_survives_culling_fallback() {
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        let blocks = (0..25)
            .map(|index| LiquidBlock {
                role: LiquidBlockRole::Paragraph,
                text: format!("Paragraph {index}: a paragraph with several words to select."),
                label: None,
            })
            .collect::<Vec<_>>();
        let first_text = blocks[0].text.clone();
        let second_text = blocks[1].text.clone();
        app.view_mode = DocumentViewMode::LiquidMode2;
        app.liquid_mode2_state = LiquidState::Ready(Arc::new(document(blocks)));
        app.liquid_mode2_complete = true;
        app.liquid_hide_footnotes = true;
        let draw = |time, events, app: &mut PdfEditorApp| {
            ctx.run(input(time, events), |ctx| {
                egui::CentralPanel::default()
                    .show(ctx, |ui| app.draw_liquid_mode2_document(ui, ctx));
            })
        };
        draw(0.0, vec![], &mut app);
        let output = draw(0.1, vec![], &mut app);
        let start = output
            .shapes
            .iter()
            .find_map(|shape| find_text(&shape.shape, &first_text))
            .unwrap()
            + Vec2::new(15.0, 8.0);
        let end = output
            .shapes
            .iter()
            .find_map(|shape| find_text(&shape.shape, &second_text))
            .unwrap()
            + Vec2::new(80.0, 8.0);
        draw(0.2, vec![egui::Event::PointerMoved(start)], &mut app);
        draw(
            0.3,
            vec![egui::Event::PointerButton {
                pos: start,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
            &mut app,
        );
        draw(0.4, vec![egui::Event::PointerMoved(end)], &mut app);
        draw(
            0.5,
            vec![egui::Event::PointerButton {
                pos: end,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
            &mut app,
        );
        assert!(
            ctx.plugin::<egui::text_selection::LabelSelectionState>()
                .lock()
                .has_selection()
        );
        draw(0.6, vec![], &mut app);
        assert!(
            ctx.plugin::<egui::text_selection::LabelSelectionState>()
                .lock()
                .has_selection()
        );
    }
}
