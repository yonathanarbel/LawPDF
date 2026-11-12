use super::*;

const BADGE_SIZE: Vec2 = Vec2::new(27.0, 25.0);

impl PdfEditorApp {
    /// Keep the native cursor's precise hot spot and add a non-interactive
    /// badge beside it only while an editing tool is over the PDF itself.
    pub(super) fn draw_tool_cursor(&self, ctx: &Context) {
        if self.view_mode != DocumentViewMode::Pdf
            || self.sbs_mode.is_some()
            || self.document.is_none()
            || self.active_tool == Tool::Select
            || self.editing_text_box.is_some()
            || self.editing_comment.is_some()
        {
            return;
        }
        let Some(pointer) = ctx.input(|input| input.pointer.hover_pos()) else {
            return;
        };
        if !self
            .chrome
            .document_rect
            .is_some_and(|rect| rect.contains(pointer))
            || !self
                .pdf_page_placements
                .iter()
                .any(|(_, placement)| placement.rect.contains(pointer))
        {
            return;
        }
        if ctx
            .layer_id_at(pointer)
            .is_some_and(|layer| layer.order > egui::Order::Middle)
        {
            return;
        }
        // Do not cover a link, annotation handle, or other control that has
        // already selected a different native pointer.
        let expected_cursor = tool_cursor(self.active_tool);
        if ctx.output(|output| output.cursor_icon) != expected_cursor {
            let dragging_page = ctx.input(|input| input.pointer.primary_down())
                && match self.active_tool {
                    Tool::Marker => self.selection_state.anchor.is_some(),
                    Tool::TextBox | Tool::Signature => self.active_drag_page.is_some(),
                    Tool::Select => false,
                };
            if !dragging_page {
                return;
            }
            // Egui keeps the drag on its starting page, even after the pointer
            // crosses onto another page or the starting page scrolls away.
            ctx.set_cursor_icon(expected_cursor);
        }

        let rect = badge_rect(pointer, ctx.content_rect());
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Tooltip,
            egui::Id::new("pdf-tool-cursor"),
        ));
        painter.rect_filled(
            rect.translate(Vec2::new(0.0, 1.5)),
            7,
            Color32::from_black_alpha(40),
        );
        painter.rect_filled(rect, 7, Color32::from_rgba_unmultiplied(255, 253, 247, 245));
        painter.rect_stroke(
            rect,
            7,
            Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(49, 42, 37, 150)),
            egui::StrokeKind::Inside,
        );
        match self.active_tool {
            Tool::Marker => paint_marker(&painter, rect, self.marker_preset()),
            Tool::TextBox => paint_text_box(&painter, rect),
            Tool::Signature => paint_signature(&painter, rect),
            Tool::Select => {}
        }
    }
}

fn badge_rect(pointer: Pos2, bounds: Rect) -> Rect {
    let offset = Vec2::new(12.0, 14.0);
    let x = if pointer.x + offset.x + BADGE_SIZE.x <= bounds.right() {
        pointer.x + offset.x
    } else {
        pointer.x - offset.x - BADGE_SIZE.x
    };
    let y = if pointer.y + offset.y + BADGE_SIZE.y <= bounds.bottom() {
        pointer.y + offset.y
    } else {
        pointer.y - offset.y - BADGE_SIZE.y
    };
    Rect::from_min_size(
        Pos2::new(
            x.clamp(
                bounds.left(),
                (bounds.right() - BADGE_SIZE.x).max(bounds.left()),
            ),
            y.clamp(
                bounds.top(),
                (bounds.bottom() - BADGE_SIZE.y).max(bounds.top()),
            ),
        ),
        BADGE_SIZE,
    )
}

fn paint_marker(painter: &egui::Painter, rect: Rect, preset: MarkerPreset) {
    let at = |x, y| rect.min + Vec2::new(x, y);
    let ink = Color32::from_rgb(54, 48, 43);
    let color = color_from_rgb(preset.color_rgb, 255);
    // A little angled marker with its actual selected colour at the nib.
    painter.add(egui::Shape::convex_polygon(
        vec![at(8.0, 15.0), at(15.0, 5.0), at(20.0, 8.0), at(13.0, 18.0)],
        Color32::from_rgb(250, 247, 237),
        Stroke::new(1.1_f32, ink),
    ));
    painter.add(egui::Shape::convex_polygon(
        vec![at(7.0, 15.0), at(12.0, 18.0), at(8.0, 20.0), at(5.0, 19.0)],
        color,
        Stroke::new(0.8_f32, ink),
    ));
    let width: f32 = if preset.style == MarkerStyle::Highlight {
        3.2
    } else {
        1.6
    };
    painter.line_segment([at(5.0, 21.0), at(18.0, 21.0)], Stroke::new(width, color));
}

fn paint_text_box(painter: &egui::Painter, rect: Rect) {
    let ink = Color32::from_rgb(54, 48, 43);
    let frame = Rect::from_center_size(rect.center(), Vec2::new(17.0, 16.0));
    painter.rect_stroke(
        frame,
        2,
        Stroke::new(1.0_f32, ink),
        egui::StrokeKind::Inside,
    );
    painter.text(
        frame.center(),
        Align2::CENTER_CENTER,
        "T",
        FontId::proportional(13.0),
        ink,
    );
}

fn paint_signature(painter: &egui::Painter, rect: Rect) {
    let at = |x, y| rect.min + Vec2::new(x, y);
    let ink = Color32::from_rgb(54, 48, 43);
    painter.add(egui::Shape::convex_polygon(
        vec![
            at(6.0, 19.0),
            at(10.0, 9.0),
            at(16.0, 4.0),
            at(21.0, 9.0),
            at(16.0, 15.0),
        ],
        Color32::from_rgb(239, 225, 196),
        Stroke::new(1.0_f32, ink),
    ));
    painter.line_segment([at(6.0, 19.0), at(15.0, 10.0)], Stroke::new(1.0_f32, ink));
    painter.circle_filled(at(15.0, 10.0), 1.2, ink);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::PageInfo;

    fn has_color(shape: &egui::Shape, color: Color32) -> bool {
        match shape {
            egui::Shape::Path(path) => path.fill == color,
            egui::Shape::Vec(shapes) => shapes.iter().any(|shape| has_color(shape, color)),
            _ => false,
        }
    }

    fn swatch_center(shape: &egui::Shape, color: Color32) -> Option<Pos2> {
        match shape {
            egui::Shape::Circle(circle) if circle.radius == 8.0 && circle.fill == color => {
                Some(circle.center)
            }
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| swatch_center(shape, color)),
            _ => None,
        }
    }

    #[test]
    fn badge_stays_on_screen_near_the_edges() {
        let bounds = Rect::from_min_size(Pos2::ZERO, Vec2::new(500.0, 400.0));
        for pointer in [Pos2::new(1.0, 1.0), Pos2::new(499.0, 399.0)] {
            let rect = badge_rect(pointer, bounds);
            assert!(bounds.contains_rect(rect));
            assert!(!rect.contains(pointer));
        }
    }

    #[test]
    fn marker_cursor_shows_the_active_color_over_the_pdf_only() {
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        app.settings.sidebar_in_pdf = false;
        app.document.as_mut().unwrap().text_chars[0] = Some(Vec::new());
        app.active_tool = Tool::Marker;
        app.marker_preset_index = 2;
        let texture = ctx.load_texture(
            "tool-cursor-test-page",
            egui::ColorImage::filled([4, 4], Color32::WHITE),
            TextureOptions::LINEAR,
        );
        app.page_textures.insert(
            0,
            PageTexture {
                _zoom: app.zoom,
                render_scale: app.page_render_scale(&ctx, 612.0, 792.0),
                last_used: 1,
                texture,
                fade_started: None,
            },
        );
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 900.0));
        let frame = |time, events: Vec<egui::Event>, app: &mut PdfEditorApp| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(screen),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    app.draw_document(ctx);
                    app.draw_markup_bar(ctx);
                    app.draw_tool_cursor(ctx);
                },
            )
        };
        let _ = frame(0.0, vec![], &mut app);
        let point = app.pdf_page_placements[0]
            .1
            .rect
            .intersect(app.chrome.document_rect.unwrap())
            .center();
        let output = frame(0.1, vec![egui::Event::PointerMoved(point)], &mut app);
        let color = color_from_rgb(MARKER_PRESETS[2].color_rgb, 255);
        assert_eq!(
            output.platform_output.cursor_icon,
            CursorIcon::Text,
            "pointer {point:?}, page {:?}, document {:?}",
            app.pdf_page_placements[0].1.rect,
            app.chrome.document_rect
        );
        assert!(
            output
                .shapes
                .iter()
                .any(|shape| has_color(&shape.shape, color))
        );

        let swatch = output
            .shapes
            .iter()
            .find_map(|shape| {
                swatch_center(
                    &shape.shape,
                    color_from_rgb(MARKER_PRESETS[2].color_rgb, 230),
                )
            })
            .expect("marker swatch is visible");
        let output = frame(0.2, vec![egui::Event::PointerMoved(swatch)], &mut app);
        assert!(
            !output
                .shapes
                .iter()
                .any(|shape| has_color(&shape.shape, color))
        );

        app.active_tool = Tool::Select;
        let output = frame(0.3, vec![egui::Event::PointerMoved(point)], &mut app);
        assert!(
            !output
                .shapes
                .iter()
                .any(|shape| has_color(&shape.shape, color))
        );

        app.active_tool = Tool::Marker;
        let output = frame(
            0.4,
            vec![egui::Event::PointerMoved(Pos2::new(2.0, 2.0))],
            &mut app,
        );
        assert!(
            !output
                .shapes
                .iter()
                .any(|shape| has_color(&shape.shape, color))
        );
    }

    #[test]
    fn marker_cursor_stays_visible_during_a_cross_page_drag() {
        let chars = |text: &str, bottom: f32| {
            text.chars()
                .enumerate()
                .map(|(index, ch)| PageTextChar {
                    ch,
                    rect: Some(PdfRect::new(
                        72.0 + index as f32 * 9.0,
                        bottom,
                        80.0 + index as f32 * 9.0,
                        bottom + 16.0,
                    )),
                    font_size: Some(12.0),
                    bold: false,
                    italic: false,
                })
                .collect::<Vec<_>>()
        };
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        let document = app.document.as_mut().unwrap();
        document.page_count = 2;
        document.pages = vec![PageInfo::new(612.0, 792.0); 2];
        document.native_text = vec!["tail".to_owned(), "head".to_owned()];
        document.native_text_loaded = vec![true, true];
        document.text_chars = vec![Some(chars("tail", 30.0)), Some(chars("head", 742.0))];
        document.links = vec![Vec::new(), Vec::new()];
        app.settings.sidebar_in_pdf = false;
        app.active_tool = Tool::Marker;
        app.zoom = 0.4;
        app.target_zoom = 0.4;
        app.scroll_target_page = None;
        let texture = ctx.load_texture(
            "cross-page-cursor-test",
            egui::ColorImage::filled([4, 4], Color32::WHITE),
            TextureOptions::LINEAR,
        );
        let render_scale = app.page_render_scale(&ctx, 612.0, 792.0);
        for page in 0..2 {
            app.page_textures.insert(
                page,
                PageTexture {
                    _zoom: app.zoom,
                    render_scale,
                    last_used: 1,
                    texture: texture.clone(),
                    fade_started: None,
                },
            );
        }
        let frame = |time, events: Vec<egui::Event>, app: &mut PdfEditorApp| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 900.0))),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    app.draw_document(ctx);
                    app.draw_markup_bar(ctx);
                    app.draw_tool_cursor(ctx);
                },
            )
        };
        let _ = frame(0.0, vec![], &mut app);
        let first = app.pdf_page_placements[0].1.pdf_to_screen((76.0, 38.0));
        let second = app.pdf_page_placements[1].1.pdf_to_screen((103.0, 750.0));
        let _ = frame(
            0.1,
            vec![
                egui::Event::PointerMoved(first),
                egui::Event::PointerButton {
                    pos: first,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            &mut app,
        );
        let output = frame(0.2, vec![egui::Event::PointerMoved(second)], &mut app);
        assert!(app.selection_state.anchor.is_some());
        assert_eq!(output.platform_output.cursor_icon, CursorIcon::Text);
        let color = color_from_rgb(MARKER_PRESETS[0].color_rgb, 255);
        assert!(
            output
                .shapes
                .iter()
                .any(|shape| has_color(&shape.shape, color))
        );
    }
}
