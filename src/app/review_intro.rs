//! A one-time, non-modal explanation of the Review view.
use super::*;

impl PdfEditorApp {
    pub(super) fn draw_review_intro(&mut self, ctx: &Context, panel: Rect) {
        if self.settings.review_intro_dismissed || self.review_reveal.is_some() {
            return;
        }
        let width = (panel.width() - 24.0).min(300.0);
        if width < 160.0 {
            return;
        }
        let position = Pos2::new(panel.right() - width - 12.0, panel.top() + 12.0);
        egui::Area::new(egui::Id::new("review-introduction"))
            .order(egui::Order::Foreground)
            .fixed_pos(position)
            .show(ctx, |ui| {
                egui::Frame::NONE
                    .fill(theme::for_context(ctx).bar)
                    .stroke(Stroke::new(1.0_f32, theme::for_context(ctx).rule))
                    .corner_radius(6)
                    .inner_margin(12)
                    .show(ui, |ui| {
                        ui.set_width(width - 24.0);
                        ui.heading("Reading in Review Mode");
                        ui.label("Review reflows the text for reading. Choose Original above to return to the printed PDF.");
                        ui.label("Footnote numbers open notes; notes can also appear in the margin or below the text.");
                        if ui.button("Got it").clicked() {
                            self.settings.review_intro_dismissed = true;
                            self.queue_settings_save();
                        }
                    });
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_center(shape: &egui::Shape, text: &str) -> Option<Pos2> {
        match shape {
            egui::Shape::Text(shape) if shape.galley.text() == text => {
                Some(shape.pos + shape.galley.size() * 0.5)
            }
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| text_center(shape, text)),
            _ => None,
        }
    }

    #[test]
    fn dismissing_intro_hides_it_and_queues_a_preference_save() {
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        app.settings.review_intro_dismissed = false;
        app.settings_save_due = None;
        let panel = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
        let draw = |app: &mut PdfEditorApp, time, events| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(panel),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ctx| app.draw_review_intro(ctx, panel),
            )
        };
        // A newly created egui Area first measures itself before it paints.
        let _ = draw(&mut app, 0.0, vec![]);
        let output = draw(&mut app, 0.1, vec![]);
        let point = output
            .shapes
            .iter()
            .find_map(|shape| text_center(&shape.shape, "Got it"))
            .expect("dismiss button");
        draw(&mut app, 0.2, vec![egui::Event::PointerMoved(point)]);
        for (time, pressed) in [(0.3, true), (0.4, false)] {
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
        assert!(app.settings.review_intro_dismissed);
        assert!(app.settings_save_due.is_some());
        let output = draw(&mut app, 0.5, vec![]);
        assert!(
            output
                .shapes
                .iter()
                .all(|shape| text_center(&shape.shape, "Got it").is_none())
        );
    }
}
