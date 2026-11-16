use super::*;

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub(super) use windows::NativeMarkerCursor;

impl PdfEditorApp {
    /// Identify the PDF surface where the highlighter should replace the cursor.
    fn marker_cursor_preset(&self, ctx: &Context) -> Option<MarkerPreset> {
        if self.view_mode != DocumentViewMode::Pdf
            || self.sbs_mode.is_some()
            || self.document.is_none()
            || self.active_tool != Tool::Marker
            || self.editing_text_box.is_some()
            || self.editing_comment.is_some()
        {
            return None;
        }
        let pointer = ctx.input(|input| input.pointer.hover_pos())?;
        if !self
            .chrome
            .document_rect
            .is_some_and(|rect| rect.contains(pointer))
            || !self
                .pdf_page_placements
                .iter()
                .any(|(_, placement)| placement.rect.contains(pointer))
            || ctx
                .layer_id_at(pointer)
                .is_some_and(|layer| layer != egui::LayerId::background())
        {
            return None;
        }
        let dragging_page = ctx.input(|input| input.pointer.primary_down())
            && self.selection_state.anchor.is_some();
        if ctx.output(|output| output.cursor_icon) != tool_cursor(Tool::Marker) && !dragging_page {
            return None;
        }
        Some(self.marker_preset())
    }

    pub(super) fn update_tool_cursor(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        let preset = self.marker_cursor_preset(ctx);
        #[cfg(target_os = "windows")]
        {
            if preset.is_some() && self.native_marker_cursor.is_none() {
                self.native_marker_cursor = NativeMarkerCursor::new(_frame);
            }
            if let Some(native) = self.native_marker_cursor.as_mut()
                && native.set(preset.map(|_| self.marker_preset_index))
            {
                return;
            }
        }
        if let Some(preset) = preset {
            self.draw_marker_cursor(ctx, preset);
        }
    }

    /// Cross-platform fallback: replace, rather than accompany, the pointer.
    fn draw_marker_cursor(&self, ctx: &Context, preset: MarkerPreset) {
        let Some(point) = ctx.input(|input| input.pointer.hover_pos()) else {
            return;
        };
        ctx.set_cursor_icon(CursorIcon::None);
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Tooltip,
            egui::Id::new("pdf-marker-cursor"),
        ));
        let at = |x, y| point + Vec2::new(x, y);
        let ink = Color32::from_rgb(48, 44, 42);
        let color = color_from_rgb(preset.color_rgb, 255);
        // The tip is exactly at the pointer's selection hot spot.
        painter.add(egui::Shape::convex_polygon(
            vec![
                at(0.0, 0.0),
                at(3.0, -7.0),
                at(20.0, -24.0),
                at(26.0, -18.0),
                at(9.0, -1.0),
                at(3.0, 2.0),
            ],
            color,
            Stroke::new(1.2_f32, ink),
        ));
        painter.line_segment([at(3.0, -7.0), at(9.0, -1.0)], Stroke::new(1.0_f32, ink));
    }

    #[cfg(test)]
    fn draw_tool_cursor(&self, ctx: &Context) {
        if let Some(preset) = self.marker_cursor_preset(ctx) {
            self.draw_marker_cursor(ctx, preset);
        }
    }
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
            CursorIcon::None,
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
        assert_eq!(output.platform_output.cursor_icon, CursorIcon::None);
        let color = color_from_rgb(MARKER_PRESETS[0].color_rgb, 255);
        assert!(
            output
                .shapes
                .iter()
                .any(|shape| has_color(&shape.shape, color))
        );
    }

    #[test]
    fn marker_cursor_does_not_cover_a_window_over_the_pdf() {
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
        let point = Pos2::new(300.0, 300.0);
        app.active_tool = Tool::Marker;
        app.chrome.document_rect = Some(screen);
        app.pdf_page_placements = vec![(
            0,
            PagePlacement {
                rect: screen,
                page_width: 612.0,
                page_height: 792.0,
            },
        )];
        let frame = |time, events: Vec<egui::Event>| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(screen),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |_| {});
                    egui::Area::new(egui::Id::new("cursor-test-dialog"))
                        .order(egui::Order::Middle)
                        .fixed_pos(Pos2::new(250.0, 250.0))
                        .show(ctx, |ui| ui.set_min_size(Vec2::splat(100.0)));
                    // A text field in the dialog can use the same I-beam as
                    // the document; the layer still needs to suppress the badge.
                    ctx.set_cursor_icon(CursorIcon::None);
                    app.draw_tool_cursor(ctx);
                },
            )
        };
        let _ = frame(0.0, vec![]);
        let output = frame(0.1, vec![egui::Event::PointerMoved(point)]);
        assert_eq!(ctx.layer_id_at(point).unwrap().order, egui::Order::Middle);
        let color = color_from_rgb(MARKER_PRESETS[0].color_rgb, 255);
        assert!(
            !output
                .shapes
                .iter()
                .any(|shape| has_color(&shape.shape, color))
        );
    }
}
