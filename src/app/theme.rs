//! Semantic colors for LawPDF's light and dark window surfaces.
use super::*;

#[derive(Clone, Copy)]
pub(super) struct Palette {
    pub canvas: Color32,
    pub panel: Color32,
    pub bar: Color32,
    pub ink: Color32,
    pub muted: Color32,
    pub rule: Color32,
    pub accent: Color32,
    pub hover: Color32,
}

pub(super) const LIGHT: Palette = Palette {
    canvas: CANVAS_FILL,
    panel: PANEL_FILL,
    bar: BAR_FILL,
    ink: INK,
    muted: MUTED_INK,
    rule: chrome::RULE,
    accent: chrome::OXBLOOD,
    hover: Color32::from_rgb(232, 227, 216),
};
pub(super) const DARK: Palette = Palette {
    canvas: Color32::from_rgb(18, 23, 31),
    panel: Color32::from_rgb(30, 36, 47),
    bar: Color32::from_rgb(35, 41, 52),
    ink: Color32::from_rgb(239, 234, 224),
    muted: Color32::from_rgb(190, 185, 174),
    rule: Color32::from_rgb(81, 89, 101),
    accent: Color32::from_rgb(241, 163, 155),
    hover: Color32::from_rgb(55, 63, 76),
};

pub(super) fn for_context(ctx: &Context) -> Palette {
    if ctx.theme() == egui::Theme::Dark {
        DARK
    } else {
        LIGHT
    }
}

pub(super) fn for_ui(ui: &egui::Ui) -> Palette {
    if ui.visuals().dark_mode { DARK } else { LIGHT }
}

pub(super) fn set_preference(ctx: &Context, choice: crate::settings::Appearance) {
    let preference = match choice {
        crate::settings::Appearance::System => egui::ThemePreference::System,
        crate::settings::Appearance::Light => egui::ThemePreference::Light,
        crate::settings::Appearance::Dark => egui::ThemePreference::Dark,
    };
    ctx.set_theme(preference);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn luminance(color: Color32) -> f64 {
        let linear = |channel: u8| {
            let value = f64::from(channel) / 255.0;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(color.r()) + 0.7152 * linear(color.g()) + 0.0722 * linear(color.b())
    }
    fn contrast(a: Color32, b: Color32) -> f64 {
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn core_text_and_accent_have_readable_contrast_on_their_surfaces() {
        for palette in [LIGHT, DARK] {
            for surface in [palette.canvas, palette.panel, palette.bar, palette.hover] {
                assert!(contrast(palette.ink, surface) >= 4.5);
                assert!(contrast(palette.muted, surface) >= 4.5);
                assert!(contrast(palette.accent, surface) >= 4.5);
            }
        }
    }

    #[test]
    fn system_preference_falls_back_to_light_and_explicit_choice_wins() {
        let ctx = Context::default();
        ctx.options_mut(|options| options.fallback_theme = egui::Theme::Light);
        set_preference(&ctx, crate::settings::Appearance::System);
        assert_eq!(ctx.theme(), egui::Theme::Light);
        set_preference(&ctx, crate::settings::Appearance::Dark);
        assert_eq!(ctx.theme(), egui::Theme::Dark);
        set_preference(&ctx, crate::settings::Appearance::Light);
        assert_eq!(ctx.theme(), egui::Theme::Light);
    }

    #[test]
    fn review_follows_system_until_the_reader_chooses_a_review_theme() {
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        set_preference(&ctx, crate::settings::Appearance::System);
        let _ = ctx.run(
            egui::RawInput {
                system_theme: Some(egui::Theme::Dark),
                ..Default::default()
            },
            |_| {},
        );
        app.sync_review_appearance(&ctx);
        assert_eq!(app.liquid_theme, LiquidTheme::Dark);
        app.liquid_theme = LiquidTheme::Sepia;
        app.liquid_theme_follows_appearance = false;
        let _ = ctx.run(
            egui::RawInput {
                system_theme: Some(egui::Theme::Light),
                ..Default::default()
            },
            |_| {},
        );
        app.sync_review_appearance(&ctx);
        assert_eq!(app.liquid_theme, LiquidTheme::Sepia);
        app.liquid_theme_follows_appearance = true;
        app.sync_review_appearance(&ctx);
        assert_eq!(app.liquid_theme, LiquidTheme::Paper);
    }

    #[test]
    fn window_appearance_does_not_change_pdf_pixels_without_the_dim_setting() {
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        app.settings.dim_original_pages = false;
        set_preference(&ctx, crate::settings::Appearance::Dark);
        app.sync_review_appearance(&ctx);
        assert_eq!(app.original_page_tint(), Color32::WHITE);
        app.settings.dim_original_pages = true;
        assert_eq!(app.original_page_tint(), Color32::from_rgb(190, 180, 162));
    }

    fn contains_fill(shape: &egui::Shape, color: Color32) -> bool {
        match shape {
            egui::Shape::Rect(rect) => rect.fill == color,
            egui::Shape::Vec(shapes) => shapes.iter().any(|shape| contains_fill(shape, color)),
            _ => false,
        }
    }

    fn text_color(shape: &egui::Shape, wanted: &str) -> Option<Color32> {
        match shape {
            egui::Shape::Text(text) if text.galley.text().contains(wanted) => Some(
                text.galley
                    .job
                    .sections
                    .first()
                    .map_or(text.fallback_color, |section| {
                        if section.format.color == Color32::PLACEHOLDER {
                            text.fallback_color
                        } else {
                            section.format.color
                        }
                    }),
            ),
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| text_color(shape, wanted)),
            _ => None,
        }
    }

    fn text_center(shape: &egui::Shape, wanted: &str) -> Option<Pos2> {
        match shape {
            egui::Shape::Text(text) if text.galley.text() == wanted => {
                Some(text.pos + text.galley.size() * 0.5)
            }
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| text_center(shape, wanted)),
            _ => None,
        }
    }

    #[test]
    fn settings_choice_changes_theme_and_queues_persistence() {
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        app.settings_ui.open = true;
        app.settings_save_due = None;
        let draw = |app: &mut PdfEditorApp, time, events| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1100.0, 800.0))),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ctx| app.draw_settings_window(ctx),
            )
        };
        let _ = draw(&mut app, 0.0, vec![]);
        let output = draw(&mut app, 0.1, vec![]);
        let point = output
            .shapes
            .iter()
            .find_map(|shape| text_center(&shape.shape, "Dark"))
            .expect("dark appearance control");
        let _ = draw(&mut app, 0.2, vec![egui::Event::PointerMoved(point)]);
        let _ = draw(
            &mut app,
            0.3,
            vec![egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        let _ = draw(
            &mut app,
            0.4,
            vec![egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert_eq!(app.settings.appearance, crate::settings::Appearance::Dark);
        assert_eq!(ctx.theme(), egui::Theme::Dark);
        assert_eq!(app.app_palette().ink, DARK.ink);
        assert!(app.settings_save_due.is_some());
    }

    #[test]
    fn dark_window_paints_readable_masthead_and_empty_state() {
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        app.document = None;
        app.settings.recent_documents.clear();
        set_preference(&ctx, crate::settings::Appearance::Dark);
        app.sync_review_appearance(&ctx);
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1100.0, 800.0))),
            ..Default::default()
        };
        let output = ctx.run(input, |ctx| {
            app.draw_masthead(ctx);
            app.draw_document(ctx);
        });
        assert!(
            output
                .shapes
                .iter()
                .any(|shape| contains_fill(&shape.shape, DARK.bar))
        );
        assert!(
            output
                .shapes
                .iter()
                .any(|shape| contains_fill(&shape.shape, DARK.canvas))
        );
        assert_eq!(
            output
                .shapes
                .iter()
                .find_map(|shape| text_color(&shape.shape, "Drop PDF here")),
            Some(DARK.ink)
        );
    }

    #[test]
    fn dark_reader_uses_a_dark_sidebar_and_a_light_pdf_placeholder() {
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        app.settings.sidebar_in_pdf = true;
        set_preference(&ctx, crate::settings::Appearance::Dark);
        app.sync_review_appearance(&ctx);
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1100.0, 800.0))),
                ..Default::default()
            },
            |ctx| {
                app.draw_side_panel(ctx);
                app.draw_document(ctx);
            },
        );
        assert!(
            output
                .shapes
                .iter()
                .any(|shape| contains_fill(&shape.shape, DARK.panel))
        );
        assert!(
            output
                .shapes
                .iter()
                .any(|shape| contains_fill(&shape.shape, PAPER_FILL))
        );
    }

    #[test]
    fn dark_review_callout_and_manually_selected_paper_keep_contrasting_text() {
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        let review = LiquidDocument {
            title: "Synthetic article".into(),
            blocks: vec![LiquidBlock {
                role: LiquidBlockRole::Definition,
                text: "A definition for the reader.".into(),
                label: None,
            }],
            article_spans: vec![],
            block_source_lines: vec![],
            footnote_links: vec![],
            footnote_link_integrity: None,
            profile: None,
            noise_lines_removed: 0,
            llm_used: false,
            llm_provider: None,
            deep_liquid_used: false,
            deep_liquid_model: None,
            warnings: vec![],
            source_signature: "theme-test".into(),
        };
        app.liquid_mode2_state = LiquidState::Ready(Arc::new(review));
        app.liquid_mode2_complete = true;
        app.view_mode = DocumentViewMode::LiquidMode2;
        app.settings.review_intro_dismissed = true;
        app.liquid_theme = LiquidTheme::Dark;
        set_preference(&ctx, crate::settings::Appearance::Dark);
        app.sync_review_appearance(&ctx);
        let draw = |app: &mut PdfEditorApp| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1100.0, 800.0))),
                    ..Default::default()
                },
                |ctx| app.draw_document(ctx),
            )
        };
        let output = draw(&mut app);
        assert!(
            output
                .shapes
                .iter()
                .any(|shape| contains_fill(&shape.shape, Color32::from_rgb(38, 45, 57)))
        );
        assert_eq!(
            output
                .shapes
                .iter()
                .find_map(|shape| text_color(&shape.shape, "A definition for the reader.")),
            Some(app.liquid_ink_color())
        );
        app.liquid_theme = LiquidTheme::Paper;
        let output = draw(&mut app);
        assert!(
            output
                .shapes
                .iter()
                .any(|shape| contains_fill(&shape.shape, PAPER_FILL))
        );
        assert_eq!(
            output
                .shapes
                .iter()
                .find_map(|shape| text_color(&shape.shape, "A definition for the reader.")),
            Some(INK)
        );
    }
}
