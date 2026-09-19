use super::*;

pub(super) struct SettingsUi {
    pub(super) api_key_edit: String,
    pub(super) openai_api_key_edit: String,
    pub(super) groq_api_key_edit: String,
    pub(super) open: bool,
    saving_credentials: bool,
    credential_tx: Sender<Result<AppSettings, String>>,
    credential_rx: Receiver<Result<AppSettings, String>>,
    maintenance_busy: bool,
    maintenance_message: Option<String>,
    maintenance_tx: Sender<Result<crate::storage_maintenance::StorageReport, String>>,
    maintenance_rx: Receiver<Result<crate::storage_maintenance::StorageReport, String>>,
}

impl SettingsUi {
    pub(super) fn new(settings: &AppSettings) -> Self {
        let (credential_tx, credential_rx) = unbounded();
        let (maintenance_tx, maintenance_rx) = unbounded();
        Self {
            api_key_edit: settings.openrouter_api_key.clone(),
            openai_api_key_edit: settings.openai_api_key.clone(),
            groq_api_key_edit: settings.groq_api_key.clone(),
            open: false,
            saving_credentials: false,
            credential_tx,
            credential_rx,
            maintenance_busy: false,
            maintenance_message: None,
            maintenance_tx,
            maintenance_rx,
        }
    }
}

impl PdfEditorApp {
    pub(super) fn start_storage_maintenance(&mut self, clear_caches: bool, ctx: &Context) {
        if self.settings_ui.maintenance_busy {
            return;
        }
        self.settings_ui.maintenance_busy = true;
        let live = self
            .annotation_sessions
            .values()
            .map(|session| session.revision.clone())
            .collect::<Vec<_>>();
        let days = self.settings.cache_retention_days;
        let tx = self.settings_ui.maintenance_tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(crate::storage_maintenance::maintain(
                clear_caches,
                days,
                &live,
            ));
            ctx.request_repaint();
        });
    }

    fn export_support_diagnostics(&mut self) {
        let Some(path) = self.pick_save_path(
            "Save support diagnostics",
            "LawPDF-diagnostics.json",
            "JSON",
            &["json"],
        ) else {
            return;
        };
        // Allowlisted fields only. No document paths/text, provider keys, raw
        // errors, environment variables, or crash payloads enter this export.
        let diagnostic = serde_json::json!({
            "schema": 1, "version": env!("CARGO_PKG_VERSION"),
            "os": std::env::consts::OS, "architecture": std::env::consts::ARCH,
            "open_documents": self.tabs.len(), "has_unsaved_annotations": self.has_unsaved_annotations(),
            "pending_saves": self.pending_annotation_saves.len(), "active_saves": self.active_annotation_saves.len(),
            "pending_opens": self.pending_document_opens.len(),
            "credential_store_error": self.settings.credential_error.is_some(),
            "update_check_error": self.update_ui.last_check_error.is_some(),
        });
        match serde_json::to_vec_pretty(&diagnostic)
            .map_err(|error| error.to_string())
            .and_then(|bytes| {
                crate::atomic_file::write(&path, &bytes).map_err(|error| error.to_string())
            }) {
            Ok(()) => {
                self.status =
                    "Support diagnostics saved. Document contents and keys are excluded.".to_owned()
            }
            Err(error) => self.push_error_notice(error),
        }
    }

    pub(super) fn poll_settings_credentials(&mut self, ctx: &Context) {
        while let Ok(result) = self.settings_ui.maintenance_rx.try_recv() {
            self.settings_ui.maintenance_busy = false;
            self.settings_ui.maintenance_message = Some(match result {
                Ok(report) => format!(
                    "Cleared {} cached files ({:.1} MB).",
                    report.removed_files,
                    report.removed_bytes as f64 / (1024.0 * 1024.0)
                ),
                Err(error) => error,
            });
        }
        while let Ok(result) = self.settings_ui.credential_rx.try_recv() {
            self.settings_ui.saving_credentials = false;
            match result {
                Ok(saved) => {
                    self.settings.openrouter_api_key = saved.openrouter_api_key;
                    self.settings.openai_api_key = saved.openai_api_key;
                    self.settings.groq_api_key = saved.groq_api_key;
                    self.settings.credential_error = None;
                    match save_settings(&self.settings) {
                        Ok(()) => {
                            self.settings_ui.open = false;
                            self.liquid_state = LiquidState::Idle;
                            self.status = "Settings saved. API keys are protected by the system credential store.".to_owned();
                        }
                        Err(error) => self.push_error_notice(error),
                    }
                }
                Err(error) => self.push_error_notice(error),
            }
            ctx.request_repaint();
        }
    }

    pub(super) fn draw_settings_window(&mut self, ctx: &Context) {
        if !self.settings_ui.open {
            return;
        }

        let mut open = self.settings_ui.open;
        let mut save_clicked = false;
        egui::Window::new("LawPDF settings")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(460.0)
            .vscroll(true)
            .show(ctx, |ui| {
                if let Some(error) = &self.settings.credential_error {
                    ui.colored_label(Color32::DARK_RED, error);
                }
                if self.settings_ui.saving_credentials { ui.disable(); }
                ui.label(RichText::new(APP_VERSION_LABEL).size(12.0).color(MUTED_INK));
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            !self.update_ui.check_in_flight,
                            egui::Button::new("Check for updates"),
                        )
                        .clicked()
                    {
                        self.update_ui.manual_check = true;
                        self.update_ui.last_check_error = None;
                        self.update_ui.check_in_flight = true;
                        self.update_ui.next_check = None;
                        self.update_ui.notice = Some(UpdateNotice::persistent(
                            "Checking for updates…",
                            UpdateNoticeKind::Working,
                        ));
                        updater::spawn_update_check(self.update_ui.tx.clone());
                    }
                    if self.update_ui.check_in_flight {
                        ui.spinner();
                    }
                });
                if let Some(error) = &self.update_ui.last_check_error {
                    ui.label("Could not check for updates. LawPDF will retry automatically.")
                        .on_hover_text(error);
                }
                ui.hyperlink_to("Download LawPDF from GitHub", updater::RELEASES_PAGE);
                ui.add_space(12.0);
                ui.label(RichText::new("Local document data").strong());
                ui.label("Cached text and images and old backup copies expire after the selected period. Unsaved edits and their source PDFs stay until you recover or discard them.");
                let retention_changed = ui.add(egui::Slider::new(&mut self.settings.cache_retention_days, 1..=365).text("Days to keep cached data")).changed();
                if retention_changed {
                    if let Err(error) = save_settings(&self.settings) { self.push_error_notice(error); }
                }
                if ui.add_enabled(!self.settings_ui.maintenance_busy, egui::Button::new("Clear cached text and images")).clicked() {
                    self.start_storage_maintenance(true, ctx);
                }
                if let Some(message) = &self.settings_ui.maintenance_message { ui.label(message); }
                if ui.button("Save support diagnostics…").clicked() { self.export_support_diagnostics(); }
                ui.hyperlink_to("Privacy and data storage", "https://github.com/yonathanarbel/LawPDF/blob/main/docs/PRIVACY.md");
                ui.add_space(12.0);
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
                    RichText::new("Keys are stored in your system credential store. Blank a field to remove its key.")
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
            let mut candidate = self.settings.clone();
            candidate.openrouter_api_key = self.settings_ui.api_key_edit.trim().to_owned();
            candidate.openai_api_key = self.settings_ui.openai_api_key_edit.trim().to_owned();
            candidate.groq_api_key = self.settings_ui.groq_api_key_edit.trim().to_owned();
            self.settings_ui.saving_credentials = true;
            self.status = "Saving API keys securely…".to_owned();
            let tx = self.settings_ui.credential_tx.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let result = crate::credentials::save(&candidate).map(|()| candidate);
                let _ = tx.send(result);
                ctx.request_repaint();
            });
        }
    }
}
