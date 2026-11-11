//! Reproducible headless Review-frame measurements. The synthetic text is not
//! a research corpus and contains no user documents.
use super::*;

fn synthetic_article(paragraphs: usize) -> LiquidDocument {
    let sentence = "The court considered the agreement and explained that the parties must give practical effect to each relevant term in the instrument. ";
    let mut blocks = Vec::new();
    for index in 0..paragraphs {
        if index % 12 == 0 {
            blocks.push(LiquidBlock {
                role: LiquidBlockRole::Heading,
                text: format!("Section {}", index / 12 + 1),
                label: None,
            });
        }
        let note = index + 1;
        let text = format!(
            "{}{}{}{}{} {}{}",
            sentence.repeat(2),
            crate::layout_roles::CALLOUT_START,
            note,
            crate::layout_roles::CALLOUT_END,
            sentence.repeat(2),
            sentence.repeat(2),
            sentence
        );
        blocks.push(LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text,
            label: None,
        });
        blocks.push(LiquidBlock {
            role: LiquidBlockRole::Footnote,
            text: format!("{note}. {}", sentence.repeat(2)),
            label: None,
        });
    }
    LiquidDocument {
        title: "Synthetic Review benchmark".to_owned(),
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
        source_signature: format!("synthetic-{paragraphs}"),
    }
}

fn draw_frame(app: &mut PdfEditorApp, ctx: &Context, width: f32, time: f64) -> Duration {
    let input = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(width, 900.0))),
        time: Some(time),
        ..Default::default()
    };
    let start = Instant::now();
    let _ = ctx.run(input, |ctx| {
        app.draw_masthead(ctx);
        app.draw_document(ctx);
        app.draw_markup_bar(ctx);
    });
    start.elapsed()
}

fn median(values: &mut [Duration]) -> Duration {
    values.sort_unstable();
    values[values.len() / 2]
}

/// Run with `cargo test --release --locked review_frame_cost -- --ignored --nocapture`.
/// Set LAWPDF_ENFORCE_REVIEW_BUDGETS=1 to enforce the synthetic median budgets.
/// This is a headless CPU check, not a real-window or worst-frame guarantee.
#[test]
#[ignore = "performance baseline; run in release mode on a reference machine"]
fn review_frame_cost() {
    for paragraphs in [108, 448, 807] {
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        let document = synthetic_article(paragraphs);
        let original = serde_json::to_vec(&document).unwrap();
        let count = document.blocks.len();
        app.liquid_mode2_state = LiquidState::Ready(Arc::new(document));
        app.liquid_mode2_complete = true;
        app.view_mode = DocumentViewMode::LiquidMode2;
        app.liquid_mode2_reveal_at = Some(-100.0);
        app.settings.sidebar_in_review = false;
        for frame in 0..10 {
            draw_frame(&mut app, &ctx, 1440.0, frame as f64 / 60.0);
        }
        let mut steady = (10..50)
            .map(|frame| draw_frame(&mut app, &ctx, 1440.0, frame as f64 / 60.0))
            .collect::<Vec<_>>();
        let mut resize = (0..12)
            .map(|frame| {
                draw_frame(
                    &mut app,
                    &ctx,
                    1440.0 - 9.0 * frame as f32,
                    1.0 + frame as f64 / 60.0,
                )
            })
            .collect::<Vec<_>>();
        // At wide sizes the body column may already be at its maximum width.
        // Also exercise a narrower window where every resize changes wrapping.
        let mut resize_reflow = (0..12)
            .map(|frame| {
                draw_frame(
                    &mut app,
                    &ctx,
                    1060.0 - 9.0 * frame as f32,
                    2.0 + frame as f64 / 60.0,
                )
            })
            .collect::<Vec<_>>();
        if let LiquidState::Ready(document) = &app.liquid_mode2_state {
            assert_eq!(serde_json::to_vec(document.as_ref()).unwrap(), original);
        } else {
            panic!("benchmark document was unexpectedly replaced");
        }
        let steady_ms = median(&mut steady).as_secs_f64() * 1000.0;
        let resize_ms = median(&mut resize).as_secs_f64() * 1000.0;
        let reflow_ms = median(&mut resize_reflow).as_secs_f64() * 1000.0;
        println!(
            "paragraphs={paragraphs} blocks={count} steady_ms={:.3} resize_ms={:.3} resize_reflow_ms={:.3}",
            steady_ms, resize_ms, reflow_ms
        );
        if std::env::var_os("LAWPDF_ENFORCE_REVIEW_BUDGETS").as_deref()
            == Some(std::ffi::OsStr::new("1"))
        {
            assert!(
                steady_ms < 1.5,
                "{paragraphs} paragraphs: median steady frame {steady_ms:.3} ms exceeds 1.5 ms"
            );
            assert!(
                resize_ms < 8.0 && reflow_ms < 8.0,
                "{paragraphs} paragraphs: median resize frames {resize_ms:.3}/{reflow_ms:.3} ms exceed 8 ms"
            );
        }
    }
}
