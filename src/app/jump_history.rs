//! Bounded, per-document history of explicit navigation, not ordinary scrolling.
use super::*;

const MAX_JUMPS: usize = 100;

pub(super) fn consume_history_key(ctx: &Context, enabled: bool) -> Option<bool> {
    if !enabled {
        return None;
    }
    ctx.input_mut(|input| {
        if input.consume_key(egui::Modifiers::ALT, egui::Key::ArrowLeft) {
            Some(false)
        } else if input.consume_key(egui::Modifiers::ALT, egui::Key::ArrowRight) {
            Some(true)
        } else {
            None
        }
    })
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Location {
    view: DocumentViewMode,
    page: usize,
    fraction: f32,
    review_block: Option<usize>,
}

#[derive(Default)]
pub(super) struct JumpHistory {
    back: Vec<Location>,
    forward: Vec<Location>,
}

impl JumpHistory {
    fn push(stack: &mut Vec<Location>, location: Location) {
        if stack.last() == Some(&location) {
            return;
        }
        if stack.len() == MAX_JUMPS {
            stack.remove(0);
        }
        stack.push(location);
    }

    fn record(&mut self, origin: Location) {
        Self::push(&mut self.back, origin);
        self.forward.clear();
    }

    fn navigate(&mut self, current: Location, forward: bool) -> Option<Location> {
        let (source, destination) = if forward {
            (&mut self.forward, &mut self.back)
        } else {
            (&mut self.back, &mut self.forward)
        };
        let target = source.pop()?;
        Self::push(destination, current);
        Some(target)
    }
}

impl PdfEditorApp {
    fn current_jump_location(&self) -> Option<Location> {
        self.document.as_ref()?;
        let visible = self.visible_page_ranges.first();
        let (page, fraction) = if self.view_mode == DocumentViewMode::Pdf {
            let pending = if self.sbs_mode.is_some() {
                self.sbs_pane_states
                    .get(&self.document_epoch)
                    .and_then(|state| state.destination_fraction)
            } else {
                self.pending_resume_fraction
            };
            pending
                .or_else(|| self.scroll_target_page.map(|page| (page, 0.0)))
                .unwrap_or_else(|| {
                    visible.map_or((self.page_index, 0.0), |range| {
                        (range.page_index, range.top_fraction.clamp(0.0, 1.0))
                    })
                })
        } else {
            (self.page_index, 0.0)
        };
        Some(Location {
            view: self.view_mode,
            page,
            fraction,
            review_block: (self.view_mode != DocumentViewMode::Pdf)
                .then(|| {
                    self.liquid_scroll_to_block
                        .or(self.search_state.review_anchor)
                })
                .flatten(),
        })
    }

    pub(super) fn record_jump_origin(&mut self) {
        if let Some(origin) = self.current_jump_location() {
            self.jump_histories
                .entry(self.document_epoch)
                .or_default()
                .record(origin);
        }
    }

    pub(super) fn jump_to_page(&mut self, page: usize) {
        self.record_jump_origin();
        self.go_to_page(page);
    }

    pub(super) fn jump_to_pdf_destination(&mut self, page: usize, fraction: Option<f32>) {
        self.jump_to_page(page);
        if self.view_mode == DocumentViewMode::Pdf {
            self.set_pdf_destination_fraction(self.page_index, fraction);
        }
    }

    pub(super) fn set_pdf_destination_fraction(&mut self, page: usize, fraction: Option<f32>) {
        let Some(fraction) = fraction.filter(|fraction| fraction.is_finite()) else {
            return;
        };
        let destination = Some((page, fraction.clamp(0.0, 1.0)));
        if self.sbs_mode.is_some() {
            self.seed_sbs_pane_state(self.document_epoch);
            if let Some(state) = self.sbs_pane_states.get_mut(&self.document_epoch) {
                state.destination_fraction = destination;
            }
        } else {
            self.pending_resume_fraction = destination;
        }
    }

    pub(super) fn jump_to_review_block(&mut self, block: usize) {
        self.record_jump_origin();
        self.liquid_scroll_to_block = Some(block);
    }

    pub(super) fn navigate_jump_history(&mut self, ctx: &Context, forward: bool) {
        let Some(current) = self.current_jump_location() else {
            return;
        };
        let Some(target) = self
            .jump_histories
            .get_mut(&self.document_epoch)
            .and_then(|history| history.navigate(current, forward))
        else {
            return;
        };
        self.pending_reference_note = None;
        self.search_state.landing = false;
        self.search_state.follow_result = false;
        self.pending_resume_fraction = None;
        self.pending_resume_review_block = None;
        self.set_view_mode(target.view, ctx);
        self.go_to_page(target.page);
        if target.view == DocumentViewMode::Pdf {
            self.set_pdf_destination_fraction(self.page_index, Some(target.fraction));
        } else if let Some(block) = target.review_block {
            self.liquid_scroll_to_block = Some(block);
        }
        ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(page: usize) -> Location {
        Location {
            view: DocumentViewMode::Pdf,
            page,
            fraction: 0.25,
            review_block: None,
        }
    }

    #[test]
    fn back_forward_and_branching_preserve_origins() {
        let mut history = JumpHistory::default();
        history.record(page(0));
        history.record(page(4));
        assert_eq!(history.navigate(page(8), false), Some(page(4)));
        assert_eq!(history.navigate(page(4), false), Some(page(0)));
        assert_eq!(history.navigate(page(0), true), Some(page(4)));
        history.record(page(4));
        assert_eq!(history.navigate(page(9), true), None);
        assert_eq!(history.navigate(page(9), false), Some(page(4)));
    }

    #[test]
    fn history_is_bounded() {
        let mut history = JumpHistory::default();
        for n in 0..MAX_JUMPS + 5 {
            history.record(page(n));
        }
        assert_eq!(history.back.len(), MAX_JUMPS);
        assert_eq!(history.back.first(), Some(&page(5)));
    }

    #[test]
    fn new_page_jumps_clear_an_older_or_nonfinite_destination() {
        let (mut app, _ctx, _worker) = annotation_saving::tests::fixture();
        app.pending_resume_fraction = Some((0, 0.8));
        app.jump_to_pdf_destination(0, Some(f32::NAN));
        assert_eq!(app.pending_resume_fraction, None);
        app.jump_to_pdf_destination(0, Some(2.0));
        assert_eq!(app.pending_resume_fraction, Some((0, 1.0)));
        app.jump_to_page(0);
        assert_eq!(app.pending_resume_fraction, None);
    }

    #[test]
    fn clicking_pdf_link_jumps_and_back_restores_origin() {
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        let doc = app.document.as_mut().unwrap();
        doc.page_count = 4;
        doc.pages = vec![crate::model::PageInfo::new(612.0, 792.0); 4];
        doc.native_text = vec![String::new(); 4];
        doc.native_text_loaded = vec![true; 4];
        doc.text_chars = vec![Some(Vec::new()); 4];
        let rect = PdfRect::new(70.0, 580.0, 180.0, 650.0);
        doc.links = vec![
            vec![PageLink {
                rect,
                url: String::new(),
                destination_page: Some(2),
                destination_fraction: Some(0.25),
            }],
            vec![],
            vec![],
            vec![],
        ];
        app.settings.sidebar_in_pdf = false;
        app.settings.reduce_motion = true;
        app.fit_width_on_first_draw = false;
        app.scroll_target_page = None;
        let frame = |app: &mut PdfEditorApp, time, events| {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 700.0))),
                time: Some(time),
                events,
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| app.draw_document(ctx));
        };
        frame(&mut app, 0.0, vec![]);
        let placement = app
            .pdf_page_placements
            .iter()
            .find(|(page, _)| *page == 0)
            .unwrap()
            .1;
        let point = placement.pdf_rect_to_screen(rect).center();
        frame(&mut app, 0.1, vec![egui::Event::PointerMoved(point)]);
        for (time, pressed) in [(0.2, true), (0.3, false)] {
            frame(
                &mut app,
                time,
                vec![egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
        }
        assert_eq!(
            app.scroll_target_page,
            Some(2),
            "the link click must queue the destination"
        );
        assert_eq!(app.pending_resume_fraction, Some((2, 0.25)));
        frame(&mut app, 0.4, vec![]);
        assert_eq!(app.page_index, 2);
        let landed = app
            .visible_page_ranges
            .iter()
            .find(|range| range.page_index == 2)
            .unwrap();
        assert!(
            (landed.top_fraction - 0.25).abs() < 0.03,
            "landed at {}",
            landed.top_fraction
        );
        let _ = ctx.run(Default::default(), |ctx| {
            app.navigate_jump_history(ctx, false)
        });
        assert_eq!(app.page_index, 0);
        assert!(app.pending_resume_fraction.is_some());
    }

    #[test]
    fn each_document_has_its_own_jump_stack() {
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        app.record_jump_origin();
        app.document_epoch = 8;
        app.page_index = 4;
        let _ = ctx.run(Default::default(), |ctx| {
            app.navigate_jump_history(ctx, false)
        });
        assert_eq!(app.page_index, 4);
        app.document_epoch = 7;
        let _ = ctx.run(Default::default(), |ctx| {
            app.navigate_jump_history(ctx, false)
        });
        assert_eq!(app.page_index, 0);
    }

    #[test]
    fn history_shortcuts_do_not_consume_text_editor_input() {
        let ctx = Context::default();
        let event = egui::Event::Key {
            key: egui::Key::ArrowLeft,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::ALT,
        };
        let input = egui::RawInput {
            events: vec![event],
            modifiers: egui::Modifiers::ALT,
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            assert_eq!(consume_history_key(ctx, false), None);
            assert_eq!(consume_history_key(ctx, true), Some(false));
            assert_eq!(consume_history_key(ctx, true), None);
        });
    }

    #[test]
    fn stale_navigation_result_cannot_replace_newer_document_revision() {
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        let path = app.document.as_ref().unwrap().path.clone();
        app.document_navigation_revisions.insert(7, 1);
        let navigation = |title: &str| PdfNavigation {
            links: vec![vec![]],
            outline: vec![PdfOutlineItem {
                title: title.into(),
                page_index: Some(0),
                page_fraction: None,
                level: 0,
            }],
        };
        app.document_links_tx
            .send(DocumentLinksEvent {
                document_epoch: 7,
                path: path.clone(),
                revision: 0,
                result: Ok(navigation("stale")),
            })
            .unwrap();
        app.document_links_tx
            .send(DocumentLinksEvent {
                document_epoch: 7,
                path,
                revision: 1,
                result: Ok(navigation("fresh")),
            })
            .unwrap();
        let _ = ctx.run(Default::default(), |ctx| app.poll_document_links(ctx));
        assert_eq!(app.pdf_outlines[&7][0].title, "fresh");
        assert!(app.document.as_ref().unwrap().links_loaded);
    }

    #[test]
    fn pdf_bookmark_is_clickable_before_review_is_ready() {
        fn text_center(shape: &egui::Shape, text: &str) -> Option<Pos2> {
            match shape {
                egui::Shape::Text(shape) if shape.galley.text() == text => {
                    Some(shape.pos + shape.galley.size() * 0.5)
                }
                egui::Shape::Vec(shapes) => {
                    shapes.iter().find_map(|shape| text_center(shape, text))
                }
                _ => None,
            }
        }
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        let doc = app.document.as_mut().unwrap();
        doc.page_count = 3;
        doc.pages = vec![crate::model::PageInfo::new(612.0, 792.0); 3];
        app.pdf_outlines.insert(
            7,
            Arc::new(vec![PdfOutlineItem {
                title: "Chapter Two".into(),
                page_index: Some(2),
                page_fraction: Some(0.3),
                level: 0,
            }]),
        );
        let draw = |app: &mut PdfEditorApp, time, events| {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(600.0, 500.0))),
                time: Some(time),
                events,
                ..Default::default()
            };
            ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| app.draw_outline_tab(ui));
            })
        };
        let output = draw(&mut app, 0.0, vec![]);
        let point = output
            .shapes
            .iter()
            .find_map(|shape| text_center(&shape.shape, "Chapter Two"))
            .expect("bookmark is shown in PDF view");
        draw(&mut app, 0.1, vec![egui::Event::PointerMoved(point)]);
        for (time, pressed) in [(0.2, true), (0.3, false)] {
            draw(
                &mut app,
                time,
                vec![egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
        }
        assert_eq!(app.scroll_target_page, Some(2));
        assert_eq!(app.pending_resume_fraction, Some((2, 0.3)));
    }

    #[test]
    fn clicking_link_in_other_side_by_side_pane_focuses_and_navigates_it() {
        for reduce_motion in [false, true] {
            exercise_side_by_side_link(reduce_motion);
        }
    }

    fn exercise_side_by_side_link(reduce_motion: bool) {
        fn page_rect(shape: &egui::Shape) -> Option<Rect> {
            match shape {
                egui::Shape::Rect(shape) if shape.fill == PAPER_FILL => Some(shape.rect),
                egui::Shape::Vec(shapes) => shapes.iter().find_map(page_rect),
                _ => None,
            }
        }
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        app.settings.reduce_motion = reduce_motion;
        ctx.all_styles_mut(|style| {
            style.scroll_animation = if reduce_motion {
                egui::style::ScrollAnimation::none()
            } else {
                egui::style::ScrollAnimation::duration(THUMBNAIL_SCROLL_SECONDS)
            };
        });
        app.scroll_target_page = None;
        let mut second = app.document.as_ref().unwrap().clone();
        second.page_count = 4;
        second.pages = vec![crate::model::PageInfo::new(612.0, 792.0); 4];
        second.native_text = vec![String::new(); 4];
        second.native_text_loaded = vec![true; 4];
        second.text_chars = vec![Some(Vec::new()); 4];
        let link_rect = PdfRect::new(70.0, 580.0, 180.0, 650.0);
        second.links = vec![
            vec![PageLink {
                rect: link_rect,
                url: String::new(),
                destination_page: Some(2),
                destination_fraction: Some(0.25),
            }],
            vec![],
            vec![],
            vec![],
        ];
        let mut tab = app.active_tab_snapshot(second);
        tab.document_epoch = 8;
        tab.scroll_target_page = None;
        app.tabs.push(tab);
        app.pdf_outlines.insert(8, Arc::new(vec![]));
        app.sbs_mode = Some(SbsModeState {
            left_epoch: 7,
            right_epoch: 8,
        });
        app.active_tool = Tool::Select;
        let draw = |app: &mut PdfEditorApp, time, events| {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 700.0))),
                time: Some(time),
                events,
                ..Default::default()
            };
            ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| app.draw_sbs_document(ui, ctx));
            })
        };
        // The initial page can snap or animate depending on the preference.
        // Measure the link only after that positioning has settled.
        for step in 0..10 {
            draw(&mut app, step as f64 * 0.1, vec![]);
        }
        let output = draw(&mut app, 1.0, vec![]);
        let rect = output
            .shapes
            .iter()
            .filter_map(|shape| page_rect(&shape.shape))
            .filter(|rect| rect.center().x > 600.0)
            .min_by(|a, b| a.top().total_cmp(&b.top()))
            .expect("right pane's page is painted");
        let point = PagePlacement {
            rect,
            page_width: 612.0,
            page_height: 792.0,
        }
        .pdf_rect_to_screen(link_rect)
        .center();
        draw(&mut app, 1.1, vec![egui::Event::PointerMoved(point)]);
        for (time, pressed) in [(1.2, true), (1.3, false)] {
            draw(
                &mut app,
                time,
                vec![egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
        }
        assert_eq!(app.document_epoch, 8);
        assert_eq!(app.scroll_target_page, Some(2));
        assert_eq!(
            app.sbs_pane_states[&8].destination_fraction,
            Some((2, 0.25))
        );
        for step in 14..30 {
            draw(&mut app, step as f64 * 0.1, vec![]);
        }
        let landed = app
            .visible_page_ranges
            .iter()
            .find(|range| range.page_index == 2)
            .unwrap();
        assert!(
            (landed.top_fraction - 0.25).abs() < 0.03,
            "landed at {}",
            landed.top_fraction
        );
        let _ = ctx.run(
            egui::RawInput {
                time: Some(3.1),
                ..Default::default()
            },
            |ctx| app.navigate_jump_history(ctx, false),
        );
        assert_eq!(app.scroll_target_page, Some(0));
    }
}
