//! Retained thumbnail geometry for documents with many differently sized pages.
use super::*;

const THUMBNAIL_MAX_WIDTH: f32 = 118.0;
const THUMBNAIL_MAX_HEIGHT: f32 = 360.0;

#[derive(Clone, Copy, Debug)]
pub(super) struct ThumbnailRow {
    pub top: f32,
    pub height: f32,
    pub page_width: f32,
    pub display_size: Vec2,
}

#[derive(Debug)]
pub(super) struct ThumbnailLayout {
    pub document_epoch: u64,
    pub overhead: f32,
    pub rows: Vec<ThumbnailRow>,
    pub total_height: f32,
}

impl ThumbnailLayout {
    pub fn new(document_epoch: u64, pages: &[crate::model::PageInfo], overhead: f32) -> Self {
        let mut rows = Vec::with_capacity(pages.len());
        let mut top = 0.0;
        for page in pages {
            let width = if page.width.is_finite() && page.width > 0.0 {
                page.width
            } else {
                612.0
            };
            let height = if page.height.is_finite() && page.height > 0.0 {
                page.height
            } else {
                792.0
            };
            let scale = (THUMBNAIL_MAX_WIDTH / width).min(THUMBNAIL_MAX_HEIGHT / height);
            let display_size = Vec2::new(width * scale, height * scale);
            let row_height = display_size.y + overhead;
            rows.push(ThumbnailRow {
                top,
                height: row_height,
                page_width: width,
                display_size,
            });
            top += row_height;
        }
        Self {
            document_epoch,
            overhead,
            rows,
            total_height: top,
        }
    }

    pub fn visible_rows(&self, top: f32, bottom: f32) -> std::ops::Range<usize> {
        let first = self.rows.partition_point(|row| row.top + row.height < top);
        let end = self.rows.partition_point(|row| row.top <= bottom);
        first.min(end)..end
    }
}

impl PdfEditorApp {
    pub(super) fn thumbnail_layout(&mut self, overhead: f32) -> Option<Arc<ThumbnailLayout>> {
        let document = self.document.as_ref()?;
        if let Some(layout) = &self.thumbnail_layout_cache
            && layout.document_epoch == self.document_epoch
            && layout.rows.len() == document.pages.len()
            && (layout.overhead - overhead).abs() < 0.1
        {
            return Some(Arc::clone(layout));
        }
        let layout = Arc::new(ThumbnailLayout::new(
            self.document_epoch,
            &document.pages,
            overhead,
        ));
        self.thumbnail_layout_cache = Some(Arc::clone(&layout));
        Some(layout)
    }

    pub(super) fn draw_pages_tab(&mut self, ui: &mut egui::Ui, ctx: &Context) {
        let Some(path) = self.document.as_ref().map(|document| document.path.clone()) else {
            ui.label(RichText::new("No PDF loaded").color(self.app_palette().muted));
            return;
        };
        // Include a little slack so different fonts and item spacing cannot
        // make adjacent rows overlap when the sidebar is resized.
        let overhead =
            32.0 + ui.text_style_height(&egui::TextStyle::Body) + 3.0 * ui.spacing().item_spacing.y;
        let Some(layout) = self.thumbnail_layout(overhead) else {
            return;
        };
        let requested = self.thumbnail_scroll_target.take();
        egui::ScrollArea::vertical()
            .id_salt(("page-thumbnails", self.document_epoch))
            .show_viewport(ui, |ui, viewport| {
                let width = ui.available_width().max(1.0);
                let (canvas, canvas_response) = ui.allocate_exact_size(
                    Vec2::new(width, layout.total_height.max(1.0)),
                    Sense::hover(),
                );
                let visible =
                    layout.visible_rows(viewport.top() - 700.0, viewport.bottom() + 700.0);
                // AccessKit needs every page represented in the widget tree.
                // Keep raster requests restricted to the nearby range.
                let accesskit_active = ctx
                    .accesskit_node_builder(canvas_response.id, |_| ())
                    .is_some();
                let rows_to_draw =
                    if accesskit_active || ctx.memory(|memory| memory.options.screen_reader) {
                        0..layout.rows.len()
                    } else {
                        visible.clone()
                    };
                if let Some(index) = requested
                    && let Some(row) = layout.rows.get(index)
                {
                    let rect = Rect::from_min_size(
                        canvas.min + Vec2::new(0.0, row.top),
                        Vec2::new(width, row.height),
                    );
                    let animation = if self.settings.reduce_motion {
                        egui::style::ScrollAnimation::none()
                    } else {
                        egui::style::ScrollAnimation::duration(THUMBNAIL_SCROLL_SECONDS)
                    };
                    ui.scroll_to_rect_animation(rect, Some(Align::Center), animation);
                    if !visible.contains(&index) {
                        self.ensure_thumbnail_texture(ctx, &path, index, row.page_width);
                    }
                    ctx.request_repaint();
                }
                for page_index in rows_to_draw {
                    let row = layout.rows[page_index];
                    let rect = Rect::from_min_size(
                        canvas.min + Vec2::new(0.0, row.top),
                        Vec2::new(width, row.height),
                    );
                    let mut child = ui.new_child(
                        egui::UiBuilder::new()
                            .id_salt(("thumbnail-row", self.document_epoch, page_index))
                            .max_rect(rect),
                    );
                    {
                        let ui = &mut child;
                        let visible_range = self.visible_range_for_page(page_index);
                        let target_visible = visible_range
                            .map(|range| range.coverage.max(0.24))
                            .unwrap_or_else(|| {
                                if self.visible_page_ranges.is_empty()
                                    && page_index == self.page_index
                                {
                                    1.0
                                } else {
                                    0.0
                                }
                            });
                        let visible_t =
                            if self.settings.reduce_motion || !visible.contains(&page_index) {
                                target_visible
                            } else {
                                ctx.animate_value_with_time(
                                    ui.id().with("thumb-visible"),
                                    target_visible,
                                    0.14,
                                )
                            };
                        let fill = if ui.visuals().dark_mode {
                            lerp_color(self.app_palette().bar, self.app_palette().hover, visible_t)
                        } else {
                            lerp_color(Color32::from_rgb(250, 248, 242), Color32::from_rgb(231, 224, 210), visible_t)
                        };
                        let stroke = Stroke::new(
                            1.0 + visible_t,
                            if ui.visuals().dark_mode {
                                lerp_color(self.app_palette().rule, self.app_palette().accent, visible_t)
                            } else {
                                lerp_color(Color32::from_rgb(213, 208, 198), Color32::from_rgb(146, 103, 52), visible_t)
                            },
                        );
                        let inner = egui::Frame::NONE
                            .fill(fill)
                            .stroke(stroke)
                            .corner_radius(6)
                            .inner_margin(Margin::same(8))
                            .show(ui, |ui| {
                                ui.vertical_centered(|ui| {
                                    let (thumb_rect, response) =
                                        ui.allocate_exact_size(row.display_size, Sense::click());
                                    response.widget_info(|| {
                                        egui::WidgetInfo::labeled(
                                            egui::WidgetType::Button,
                                            true,
                                            format!("Go to page {}", page_index + 1),
                                        )
                                    });
                                    let painter = ui.painter_at(thumb_rect);
                                    painter.rect_filled(thumb_rect, 2, PAPER_FILL);
                                    painter.rect_stroke(
                                        thumb_rect,
                                        2,
                                        Stroke::new(1.0_f32, PAPER_STROKE),
                                        egui::StrokeKind::Inside,
                                    );
                                    if let Some(thumbnail) = visible
                                        .contains(&page_index)
                                        .then(|| {
                                            self.ensure_thumbnail_texture(
                                                ctx,
                                                &path,
                                                page_index,
                                                row.page_width,
                                            )
                                        })
                                        .flatten()
                                    {
                                        painter.image(
                                            thumbnail.texture_id,
                                            thumb_rect,
                                            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                                            Color32::WHITE,
                                        );
                                    } else if visible.contains(&page_index) {
                                        painter.text(
                                            thumb_rect.center(),
                                            Align2::CENTER_CENTER,
                                            "Rendering",
                                            FontId::proportional(13.0),
                                            MUTED_INK,
                                        );
                                    } else {
                                        painter.text(
                                            thumb_rect.center(),
                                            Align2::CENTER_CENTER,
                                            format!("{}", page_index + 1),
                                            FontId::proportional(18.0),
                                            MUTED_INK,
                                        );
                                    }
                                    if let Some(range) = visible_range {
                                        let top = thumb_rect.top()
                                            + thumb_rect.height()
                                                * range.top_fraction.clamp(0.0, 1.0);
                                        let bottom = (thumb_rect.top()
                                            + thumb_rect.height()
                                                * range.bottom_fraction.clamp(0.0, 1.0))
                                        .max(top + 3.0)
                                        .min(thumb_rect.bottom());
                                        let area = Rect::from_min_max(
                                            Pos2::new(thumb_rect.left(), top),
                                            Pos2::new(thumb_rect.right(), bottom),
                                        );
                                        let alpha = (42.0 + 86.0 * range.coverage)
                                            .round()
                                            .clamp(0.0, 160.0)
                                            as u8;
                                        painter.rect_filled(
                                            area,
                                            1,
                                            Color32::from_rgba_unmultiplied(207, 150, 64, alpha),
                                        );
                                        painter.rect_stroke(
                                            area,
                                            1,
                                            Stroke::new(
                                                1.4_f32,
                                                Color32::from_rgba_unmultiplied(146, 103, 52, 190),
                                            ),
                                            egui::StrokeKind::Inside,
                                        );
                                    }
                                    response
                                })
                                .inner
                            });
                        ui.horizontal(|ui| {
                            ui.add_space(6.0);
                            ui.label(RichText::new(format!("Page {}", page_index + 1)).color(self.app_palette().ink));
                        });
                        if inner.response.clicked() || inner.inner.clicked() {
                            self.jump_to_page(page_index);
                        }
                    }
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::PageInfo;

    #[test]
    fn variable_page_sizes_produce_correct_visible_ranges() {
        let pages = vec![
            PageInfo::new(600.0, 800.0),
            PageInfo::new(800.0, 600.0),
            PageInfo::new(100.0, 2000.0),
        ];
        let layout = ThumbnailLayout::new(7, &pages, 60.0);
        assert!(layout.rows[2].display_size.y <= 360.0);
        assert!(layout.rows[0].display_size.y > layout.rows[1].display_size.y);
        let middle = layout.rows[1];
        assert_eq!(
            layout.visible_rows(middle.top + 1.0, middle.top + middle.height - 1.0),
            1..2
        );
        assert_eq!(
            layout.visible_rows(layout.total_height + 1.0, layout.total_height + 100.0),
            3..3
        );
    }

    #[test]
    fn long_document_only_selects_nearby_rows() {
        let layout = ThumbnailLayout::new(7, &vec![PageInfo::new(612.0, 792.0); 10_000], 60.0);
        let top = layout.rows[5000].top;
        let range = layout.visible_rows(top - 700.0, top + 1400.0);
        assert!(range.contains(&5000));
        assert!(range.len() < 20);
    }

    #[test]
    fn sidebar_scrolls_to_a_distant_page_and_only_requests_nearby_thumbnails() {
        let (mut app, ctx, requests) = annotation_saving::tests::fixture();
        let document = app.document.as_mut().unwrap();
        document.page_count = 500;
        document.pages = vec![PageInfo::new(612.0, 792.0); 500];
        app.thumbnail_scroll_target = None;
        app.settings.reduce_motion = true;
        let draw = |app: &mut PdfEditorApp, time| {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(300.0, 600.0))),
                time: Some(time),
                ..Default::default()
            };
            ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| app.draw_pages_tab(ui, ctx));
            })
        };
        draw(&mut app, 0.0);
        let first_requests: Vec<_> = requests
            .try_iter()
            .filter_map(|request| match request {
                RenderRequest::Thumbnail { key, .. } => Some(key.page_index),
                _ => None,
            })
            .collect();
        assert!(!first_requests.is_empty() && first_requests.len() < 20);
        assert!(first_requests.iter().all(|page| *page < 20));
        app.thumbnail_scroll_target = Some(450);
        for frame in 1..5 {
            draw(&mut app, frame as f64 * 0.2);
        }
        let later_requests: Vec<_> = requests
            .try_iter()
            .filter_map(|request| match request {
                RenderRequest::Thumbnail { key, .. } => Some(key.page_index),
                _ => None,
            })
            .collect();
        assert!(later_requests.contains(&450));
        assert!(
            later_requests
                .iter()
                .any(|page| *page >= 447 && *page <= 453 && *page != 450),
            "the viewport did not reach nearby rows: {later_requests:?}"
        );
        assert!(later_requests.len() < 20);
    }

    #[test]
    fn clicking_a_visible_thumbnail_jumps_to_that_page() {
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
        let document = app.document.as_mut().unwrap();
        document.page_count = 3;
        document.pages = vec![PageInfo::new(612.0, 792.0); 3];
        app.page_index = 1;
        app.thumbnail_scroll_target = None;
        app.settings.reduce_motion = true;
        let draw = |app: &mut PdfEditorApp, time, events| {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(300.0, 600.0))),
                time: Some(time),
                events,
                ..Default::default()
            };
            ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| app.draw_pages_tab(ui, ctx));
            })
        };
        let output = draw(&mut app, 0.0, vec![]);
        let point = output
            .shapes
            .iter()
            .find_map(|shape| text_center(&shape.shape, "Rendering"))
            .expect("first thumbnail is painted");
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
        assert_eq!(app.page_index, 0);
        assert_eq!(app.scroll_target_page, Some(0));
    }

    #[test]
    fn mixed_size_thumbnails_have_space_for_labels_and_keep_aspect_ratio() {
        fn collect_paper_rects(shape: &egui::Shape, result: &mut Vec<Rect>) {
            match shape {
                egui::Shape::Rect(shape) if shape.fill == PAPER_FILL => result.push(shape.rect),
                egui::Shape::Vec(shapes) => shapes
                    .iter()
                    .for_each(|shape| collect_paper_rects(shape, result)),
                _ => {}
            }
        }
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        let document = app.document.as_mut().unwrap();
        document.page_count = 3;
        document.pages = vec![
            PageInfo::new(612.0, 792.0),
            PageInfo::new(800.0, 600.0),
            PageInfo::new(612.0, 792.0),
        ];
        app.thumbnail_scroll_target = None;
        app.settings.reduce_motion = true;
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(300.0, 800.0))),
            ..Default::default()
        };
        let output = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| app.draw_pages_tab(ui, ctx));
        });
        let mut rects = Vec::new();
        for shape in &output.shapes {
            collect_paper_rects(&shape.shape, &mut rects);
        }
        rects.sort_by(|a, b| a.top().total_cmp(&b.top()));
        assert_eq!(rects.len(), 3);
        assert!((rects[0].width() / rects[0].height() - 612.0 / 792.0).abs() < 0.01);
        assert!((rects[1].width() / rects[1].height() - 800.0 / 600.0).abs() < 0.01);
        assert!(
            rects
                .windows(2)
                .all(|pair| pair[1].top() - pair[0].bottom() > 25.0)
        );
    }

    #[test]
    fn screen_reader_sees_distant_pages_without_rendering_every_thumbnail() {
        let (mut app, ctx, requests) = annotation_saving::tests::fixture();
        let document = app.document.as_mut().unwrap();
        document.page_count = 100;
        document.pages = vec![PageInfo::new(612.0, 792.0); 100];
        app.thumbnail_scroll_target = None;
        app.settings.reduce_motion = true;
        ctx.enable_accesskit();
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(300.0, 600.0))),
            ..Default::default()
        };
        let output = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| app.draw_pages_tab(ui, ctx));
        });
        let tree = output
            .platform_output
            .accesskit_update
            .expect("accessibility tree");
        let page_node = tree
            .nodes
            .iter()
            .find_map(|(id, node)| (node.label() == Some("Go to page 100")).then_some(*id))
            .expect("distant page control");
        let count = requests
            .try_iter()
            .filter(|request| matches!(request, RenderRequest::Thumbnail { .. }))
            .count();
        assert!(
            count > 0 && count < 20,
            "offscreen pages must not all render: {count}"
        );
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(300.0, 600.0))),
            events: vec![egui::Event::AccessKitActionRequest(
                egui::accesskit::ActionRequest {
                    action: egui::accesskit::Action::Click,
                    target: page_node,
                    data: None,
                },
            )],
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| app.draw_pages_tab(ui, ctx));
        });
        assert_eq!(app.scroll_target_page, Some(99));
    }
}
