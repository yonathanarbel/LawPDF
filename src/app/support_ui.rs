//! Voluntary support, opened only when the reader chooses the coffee button.

use super::*;

// Keep this a fixed, public URL. Never attach document or application data.
const COFFEE_URL: &str = "https://buymeacoffee.com/lawpdf";

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
            if ui.button("Buy me a coffee ↗").clicked() {
                ui.ctx().open_url(egui::OpenUrl::new_tab(COFFEE_URL));
                ui.close();
            }
            ui.small("Opens Buy Me a Coffee in your browser.");
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
