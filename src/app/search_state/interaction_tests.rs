use super::*;
use crate::app::find_paint::passage_delta;
use crate::model::PageInfo;

fn reader(ctx: &Context) -> PdfEditorApp {
    let mut app = PdfEditorApp::new(
        ctx,
        vec![],
        unbounded().1,
        #[cfg(target_os = "macos")]
        None,
    );
    let text = vec![
        "law earlier".into(),
        "law one law two law three law four".into(),
        "closing".into(),
    ];
    app.document = Some(LoadedDocument {
        path: "find-test.pdf".into(),
        title: "Find test".into(),
        page_count: 3,
        pages: vec![PageInfo::with_footnote_divider_y_from_top(600., 800., None); 3],
        text_chars: text
            .iter()
            .map(|text: &String| {
                Some(
                    text.char_indices()
                        .map(|(i, ch)| PageTextChar {
                            ch,
                            rect: Some(PdfRect::new(
                                30.,
                                750. - i as f32 * 18.,
                                40.,
                                760. - i as f32 * 18.,
                            )),
                            font_size: Some(12.),
                            bold: false,
                            italic: false,
                        })
                        .collect(),
                )
            })
            .collect(),
        native_text: text,
        native_text_loaded: vec![true; 3],
        links: vec![vec![]; 3],
        links_loaded: true,
        optimized: true,
    });
    app.page_index = 1;
    app
}

fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}

fn frame(
    ctx: &Context,
    app: &mut PdfEditorApp,
    events: Vec<egui::Event>,
    modifiers: egui::Modifiers,
) {
    let _ = ctx.run(
        egui::RawInput {
            events,
            modifiers,
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(960., 700.))),
            ..Default::default()
        },
        |ctx| {
            app.handle_find_shortcuts(ctx);
            app.poll_find(ctx);
            app.draw_masthead(ctx);
        },
    );
}

fn settle(ctx: &Context, app: &mut PdfEditorApp, query: &str) {
    let deadline = Instant::now() + Duration::from_secs(4);
    while Instant::now() < deadline {
        frame(ctx, app, vec![], egui::Modifiers::NONE);
        if app
            .search_state
            .session
            .as_ref()
            .is_some_and(|s| s.query == query && s.scanned.len() == 3)
        {
            return;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    panic!("Find did not settle: {}", app.find_progress());
}

#[test]
fn find_keyboard_walks_same_page_wraps_and_replaces_the_query() {
    let ctx = Context::default();
    let mut app = reader(&ctx);
    let ctrl = egui::Modifiers {
        ctrl: true,
        command: true,
        ..Default::default()
    };
    frame(&ctx, &mut app, vec![key(egui::Key::F, ctrl)], ctrl);
    frame(
        &ctx,
        &mut app,
        vec![egui::Event::Text("law".into())],
        egui::Modifiers::NONE,
    );
    settle(&ctx, &mut app, "law");
    assert_eq!(app.search_state.hits.len(), 5);
    assert_eq!(
        app.search_state.selected_hit, None,
        "typing must not navigate"
    );
    assert_eq!(app.page_index, 1);
    assert_eq!(
        app.scroll_target_page,
        Some(0),
        "preview must leave the existing scroll target untouched"
    );
    for expected in [1, 2, 3, 4, 0] {
        frame(
            &ctx,
            &mut app,
            vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
            egui::Modifiers::NONE,
        );
        assert_eq!(app.search_state.selected_hit, Some(expected));
        assert!(ctx.memory(|m| m.has_focus(egui::Id::new("lawpdf-find"))));
    }
    assert!(app.search_state.wrapped);
    frame(
        &ctx,
        &mut app,
        vec![key(egui::Key::Enter, egui::Modifiers::SHIFT)],
        egui::Modifiers::SHIFT,
    );
    assert_eq!(app.search_state.selected_hit, Some(4));
    frame(&ctx, &mut app, vec![key(egui::Key::F, ctrl)], ctrl);
    frame(
        &ctx,
        &mut app,
        vec![
            egui::Event::Text("closing".into()),
            key(egui::Key::Enter, egui::Modifiers::NONE),
        ],
        egui::Modifiers::NONE,
    );
    settle(&ctx, &mut app, "closing");
    assert_eq!(app.search_state.hits.len(), 1);
    assert_eq!(
        app.search_state.selected_hit,
        Some(0),
        "Enter before matching finishes must land once"
    );
    assert_eq!(app.search_state.hits[0].page_index, 2);
    frame(
        &ctx,
        &mut app,
        vec![key(egui::Key::Escape, egui::Modifiers::NONE)],
        egui::Modifiers::NONE,
    );
    assert!(!app.search_state.open && app.search_state.session.is_none());
    assert!(app.annotations.is_empty());
}

#[test]
fn find_discards_replaced_work_and_keeps_ocr_page_only() {
    let ctx = Context::default();
    let mut app = reader(&ctx);
    app.search_state.open = true;
    app.search_state.query = "law".into();
    app.poll_find(&ctx);
    app.ocr_states = vec![
        OcrPageState::Idle,
        OcrPageState::Idle,
        OcrPageState::Done("scanned remedy".into()),
    ];
    app.invalidate_search_results();
    app.search_state.query = "remedy".into();
    settle(&ctx, &mut app, "remedy");
    assert_eq!(app.search_state.hits.len(), 1);
    let h = &app.search_state.hits[0];
    assert_eq!(h.source, SearchSource::OcrText);
    assert!(
        app.search_hit_rects(app.document.as_ref().unwrap(), h)
            .is_empty()
    );
    app.find_step(&ctx, false);
    assert_eq!(app.landing_page(), Some(2));
    app.dismiss_search(&ctx);
    app.update_search_page(&ctx, 0);
    app.poll_find(&ctx);
    assert!(app.search_state.hits.is_empty() && app.search_state.session.is_none());
}

#[test]
fn find_review_occurrences_remain_distinct_within_the_same_block() {
    let ctx = Context::default();
    let mut app = reader(&ctx);
    let review: LiquidDocument = serde_json::from_value(serde_json::json!({
        "title":"Reader","blocks":[{"role":"paragraph","text":"law then LAW"}],"llm_used":false,
        "warnings":[],"source_signature":"find-review"
    }))
    .unwrap();
    app.view_mode = DocumentViewMode::LiquidMode2;
    app.liquid_mode2_complete = true;
    app.liquid_mode2_state = LiquidState::Ready(Arc::new(review));
    app.search_state.query = "law".into();
    app.search_state.open = true;
    let end = Instant::now() + Duration::from_secs(4);
    while app.search_state.hits.len() != 2 && Instant::now() < end {
        frame(&ctx, &mut app, vec![], egui::Modifiers::NONE);
        std::thread::yield_now();
    }
    assert_eq!(app.search_state.hits.len(), 2);
    assert!(
        app.search_state
            .hits
            .iter()
            .all(|h| h.source == SearchSource::ReviewText && h.block_index == Some(0))
    );
    app.find_step(&ctx, false);
    app.find_step(&ctx, false);
    let target = &app.search_state.hits[app.search_state.selected_hit.unwrap()];
    assert_eq!(target.match_start, 9);
    assert_eq!(&target.snippet[target.snippet_match.clone()], "LAW");
    // Paint the actual Review paragraph and require a measured text landing.
    let doc = match &app.liquid_mode2_state {
        LiquidState::Ready(d) => Arc::clone(d),
        _ => unreachable!(),
    };
    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            app.draw_liquid_feedback_block(ui, &doc, 0, &doc.blocks[0]);
        });
    });
    assert!(!app.search_state.landing);
    assert!(
        !app.search_state.block_landed,
        "visible Review text must land on glyph geometry, without an excerpt fallback"
    );
}

#[test]
fn find_lands_on_text_after_a_review_footnote_marker() {
    let ctx = Context::default();
    let mut app = reader(&ctx);
    let text = format!("Before {}12{} the target passage.", layout_roles::CALLOUT_START, layout_roles::CALLOUT_END);
    let review: LiquidDocument = serde_json::from_value(serde_json::json!({
        "title":"Reader","blocks":[{"role":"paragraph","text":text}, {"role":"footnote","text":"12. A note."}],
        "llm_used":false,"warnings":[],"source_signature":"find-callout"
    })).unwrap();
    app.view_mode = DocumentViewMode::LiquidMode2;
    app.liquid_mode2_complete = true;
    app.liquid_mode2_state = LiquidState::Ready(Arc::new(review));
    app.search_state.query = "target".into();
    app.search_state.open = true;
    let end = Instant::now() + Duration::from_secs(4);
    while app.search_state.hits.is_empty() && Instant::now() < end {
        frame(&ctx, &mut app, vec![], egui::Modifiers::NONE);
        std::thread::yield_now();
    }
    assert_eq!(app.search_state.hits.len(), 1);
    app.find_step(&ctx, false);
    let doc = match &app.liquid_mode2_state {
        LiquidState::Ready(d) => Arc::clone(d),
        _ => unreachable!(),
    };
    app.liquid_footnote_index = build_liquid_footnote_index(&doc.blocks);
    let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(960.0, 700.0))), ..Default::default() };
    let _ = ctx.run(input, |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            app.draw_liquid_feedback_block(ui, &doc, 0, &doc.blocks[0]);
        });
    });
    assert!(!app.search_state.landing, "the visible match must land on its actual glyphs");
    assert!(!app.search_state.block_landed, "a normal match must not need a fallback excerpt");
}

#[test]
fn find_scope_follows_the_focused_split_pane() {
    let ctx = Context::default();
    let mut app = reader(&ctx);
    app.view_mode = DocumentViewMode::LiquidMode2;
    app.review_pdf_split = true;
    assert!(app.find_scope() == FindScope::Review);
    app.search_state.split_original = true;
    assert!(app.find_scope() == FindScope::Original);
    app.search_state.split_original = false;
    app.sbs_mode = Some(SbsModeState {
        left_epoch: 1,
        right_epoch: 2,
    });
    assert!(app.find_scope() == FindScope::Original);
}

#[test]
fn find_anchor_skips_above_viewport_and_visible_matches_do_not_scroll() {
    let ctx = Context::default();
    let mut app = reader(&ctx);
    app.search_state.open = true;
    app.search_state.query = "law".into();
    app.visible_page_ranges.push(VisiblePageRange {
        page_index: 1,
        top_fraction: 0.3,
        bottom_fraction: 0.7,
        coverage: 0.4,
    });
    settle(&ctx, &mut app, "law");
    app.find_step(&ctx, false);
    let hit = &app.search_state.hits[app.search_state.selected_hit.unwrap()];
    assert_eq!((hit.page_index, hit.match_start), (1, 16));
    let view = Rect::from_min_max(Pos2::new(0., 100.), Pos2::new(400., 400.));
    let line = Rect::from_min_size(Pos2::new(40., 220.), Vec2::new(50., 12.));
    assert_eq!(passage_delta(view, line), Vec2::ZERO);
    let far = line.translate(Vec2::new(0., 1400.));
    assert_eq!(far.top() + passage_delta(view, far).y, 200.);
}

#[test]
fn paired_find_results_are_local_to_the_open_search() {
    let ctx = Context::default();
    let mut app = reader(&ctx);
    let old_sidebar = app.settings.sidebar_in_pdf;
    app.sbs_mode = Some(SbsModeState { left_epoch: 1, right_epoch: 2 });
    app.focus_search(&ctx);
    assert!(!app.sidebar_visible());
    app.set_sidebar_visible(true);
    assert!(app.sidebar_visible());
    assert_eq!(app.settings.sidebar_in_pdf, old_sidebar);
    app.dismiss_search(&ctx);
    assert!(!app.sidebar_visible());
    app.sbs_mode = None;
    assert_eq!(app.sidebar_visible(), old_sidebar);
}
