//! Give an open document the same visible drop target as the empty window.
use super::*;

impl PdfEditorApp {
    pub(super) fn draw_file_drop_overlay(&self, ctx: &Context) {
        if self.document.is_none() || ctx.input(|input| input.raw.hovered_files.is_empty()) {
            return;
        }
        let bounds = ctx.content_rect();
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Tooltip,
            egui::Id::new("file-drop-overlay"),
        ));
        painter.rect_filled(bounds, 0, Color32::from_rgba_unmultiplied(24, 30, 38, 200));
        painter.text(
            bounds.center(),
            Align2::CENTER_CENTER,
            "Drop files to open",
            FontId::proportional(24.0),
            Color32::WHITE,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contains_label(shape: &egui::Shape) -> bool {
        match shape {
            egui::Shape::Text(text) => text.galley.text() == "Drop files to open",
            egui::Shape::Vec(shapes) => shapes.iter().any(contains_label),
            _ => false,
        }
    }

    #[test]
    fn overlay_follows_file_hover_without_changing_the_open_document() {
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        let original = app.document.as_ref().unwrap().path.clone();
        let draw = |app: &PdfEditorApp, hovered_files| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))),
                    hovered_files,
                    ..Default::default()
                },
                |ctx| app.draw_file_drop_overlay(ctx),
            )
        };
        let output = draw(&app, vec![egui::HoveredFile::default()]);
        assert!(
            output
                .shapes
                .iter()
                .any(|shape| contains_label(&shape.shape))
        );
        assert_eq!(app.document.as_ref().unwrap().path, original);
        let output = draw(&app, vec![]);
        assert!(
            output
                .shapes
                .iter()
                .all(|shape| !contains_label(&shape.shape))
        );
        app.document = None;
        let output = draw(&app, vec![egui::HoveredFile::default()]);
        assert!(
            output
                .shapes
                .iter()
                .all(|shape| !contains_label(&shape.shape))
        );
    }
}
