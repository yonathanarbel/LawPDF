use super::*;

pub(super) struct SettingsUi {
    pub(super) api_key_edit: String,
    pub(super) openai_api_key_edit: String,
    pub(super) groq_api_key_edit: String,
    pub(super) open: bool,
}

impl SettingsUi {
    pub(super) fn new(settings: &AppSettings) -> Self {
        Self {
            api_key_edit: settings.openrouter_api_key.clone(),
            openai_api_key_edit: settings.openai_api_key.clone(),
            groq_api_key_edit: settings.groq_api_key.clone(),
            open: false,
        }
    }
}

impl PdfEditorApp {
    pub(super) fn draw_settings_window(&mut self, ctx: &Context) {
        if !self.settings_ui.open {
            return;
        }

        let mut open = self.settings_ui.open;
        let mut save_clicked = false;
        egui::Window::new("LawPDF settings")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(RichText::new("Groq").strong().color(INK));
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings_ui.groq_api_key_edit)
                        .password(true)
                        .hint_text("API key")
                        .desired_width(360.0),
                );
                ui.add_space(8.0);
                ui.label(RichText::new("OpenRouter").strong().color(INK));
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings_ui.api_key_edit)
                        .password(true)
                        .hint_text("API key")
                        .desired_width(360.0),
                );
                ui.add_space(8.0);
                ui.label(RichText::new("OpenAI (optional TTS)").strong().color(INK));
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings_ui.openai_api_key_edit)
                        .password(true)
                        .hint_text("API key")
                        .desired_width(360.0),
                );
                ui.label(
                    RichText::new("Keys stay in this app's local settings.")
                        .size(10.0)
                        .color(MUTED_INK),
                );
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        save_clicked = true;
                    }
                    if ui.button("Cancel").clicked() {
                        self.settings_ui.api_key_edit = self.settings.openrouter_api_key.clone();
                        self.settings_ui.openai_api_key_edit = self.settings.openai_api_key.clone();
                        self.settings_ui.groq_api_key_edit = self.settings.groq_api_key.clone();
                        self.settings_ui.open = false;
                    }
                });
            });

        self.settings_ui.open = open && self.settings_ui.open;
        if save_clicked {
            self.settings.openrouter_api_key = self.settings_ui.api_key_edit.trim().to_owned();
            self.settings.openai_api_key = self.settings_ui.openai_api_key_edit.trim().to_owned();
            self.settings.groq_api_key = self.settings_ui.groq_api_key_edit.trim().to_owned();
            match save_settings(&self.settings) {
                Ok(()) => {
                    self.settings_ui.open = false;
                    self.liquid_state = LiquidState::Idle;
                    self.status = "Settings saved.".to_owned();
                }
                Err(error) => self.status = error,
            }
        }
    }
}
