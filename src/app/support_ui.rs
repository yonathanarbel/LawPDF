//! Voluntary support, opened only when the reader chooses the coffee button.

use super::*;

// Release blocker: replace None with the creator's verified, working HTTPS
// payment page before publishing. Never guess a recipient or publish a dead link.
const COFFEE_URL: Option<&str> = None;

pub(super) fn draw_coffee_button(ui: &mut egui::Ui) {
    let (response, _) = egui::containers::menu::MenuButton::from_button(
        egui::Button::new("")
            .frame(false)
            .min_size(Vec2::splat(TOOLBAR_ICON_SIZE)),
    )
    .ui(ui, |ui| {
        ui.set_width(280.0);
        ui.with_layout(egui::Layout::top_down(Align::Min), |ui| {
            ui.label(RichText::new("A little coffee, a lot of gratitude.").strong());
            ui.add_space(6.0);
            ui.add(egui::Label::new("LawPDF is free. If you enjoy using it and want to support my coffee addiction, you can buy me a coffee.").wrap());
            ui.add_space(8.0);
            if let Some(url) = COFFEE_URL {
                if ui.button("Buy me a coffee ↗").clicked() {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(url));
                    ui.close();
                }
                ui.small("Opens Buy Me a Coffee in your browser.");
            } else {
                ui.add_enabled(false, egui::Button::new("Buy me a coffee ↗"));
                ui.small("The coffee page is being prepared.");
            }
            ui.add_space(6.0);
            ui.label(RichText::new("A thank-you, never a requirement.").weak());
            ui.add_space(4.0);
            ui.small("With thanks, Professor Yonathan Arbel");
        });
    });
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            true,
            "Buy me a coffee — support LawPDF",
        )
    });
    paint_toolbar_icon(
        ui.painter(),
        response.rect,
        ToolbarIcon::Coffee,
        chrome::NAVY_TEXT,
    );
    toolbar_tooltip(response, "Buy me a coffee — support LawPDF");
}
