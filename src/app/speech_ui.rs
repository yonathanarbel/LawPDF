use super::*;
use crate::settings::effective_elevenlabs_api_key;
use crate::tts::{
    ApiSpeechConfig, SpeechEventKind, SpeechProvider, SpeechRequest, spawn_speech_job,
};

impl PdfEditorApp {
    fn selected_api_speech(&mut self) -> Option<ApiSpeechConfig> {
        let provider = self.settings.speech_provider;
        let (api_key, model, voice) = match provider {
            SpeechProvider::ElevenLabs => (
                effective_elevenlabs_api_key(&self.settings),
                self.settings.elevenlabs_speech_model.clone(),
                self.settings.elevenlabs_speech_voice.clone(),
            ),
            SpeechProvider::OpenRouter => (
                effective_openrouter_api_key(&self.settings),
                self.settings.openrouter_speech_model.clone(),
                self.settings.openrouter_speech_voice.clone(),
            ),
            SpeechProvider::System => return None,
        };
        let Some(api_key) = api_key else {
            self.settings_ui.open = true;
            self.status = format!(
                "Add your {} API key in Settings to narrate this text.",
                provider.label()
            );
            return None;
        };
        Some(ApiSpeechConfig {
            provider,
            api_key,
            model: model.trim().to_owned(),
            voice: voice.trim().to_owned(),
        })
    }

    fn launch_speech(&mut self, request: SpeechRequest, exporting: bool) {
        self.stop_liquid_tts();
        self.tts_controller.job_id = self.tts_controller.job_id.wrapping_add(1);
        self.tts_controller.exporting = exporting;
        self.tts_controller.progress = Some((0, 0));
        self.tts_controller.active = Some(spawn_speech_job(
            self.tts_controller.job_id,
            request,
            self.tts_controller.tx.clone(),
        ));
        self.status = if exporting {
            format!(
                "Creating MP3 with {}…",
                self.settings.speech_provider.label()
            )
        } else {
            format!("Preparing {}…", self.settings.speech_provider.label())
        };
    }

    pub(super) fn start_liquid_tts(&mut self, document: &LiquidDocument) {
        let text = liquid_tts_text(document, self.settings.speech_include_notes);
        if text.trim().is_empty() {
            self.status = "Nothing to read aloud.".to_owned();
            return;
        }
        let request = if self.settings.speech_provider == SpeechProvider::System {
            SpeechRequest::System(text)
        } else {
            let Some(config) = self.selected_api_speech() else {
                return;
            };
            SpeechRequest::Api {
                config,
                text,
                destination: None,
            }
        };
        self.launch_speech(request, false);
    }

    pub(super) fn stop_liquid_tts(&mut self) {
        // Cancellation kills only our process, without waiting. Network work,
        // process reaping and temporary-file cleanup stay on the worker.
        if let Some(job) = self.tts_controller.active.take() {
            job.cancel();
        }
        self.tts_controller.progress = None;
        self.tts_controller.job_id = self.tts_controller.job_id.wrapping_add(1);
    }

    pub(super) fn start_paid_liquid_tts(&mut self, document: &LiquidDocument) {
        if self.tts_controller.is_active() {
            return;
        }
        let Some(config) = self.selected_api_speech() else {
            return;
        };
        let text = liquid_tts_text(document, self.settings.speech_include_notes);
        if text.trim().is_empty() {
            self.status = "Nothing to narrate.".to_owned();
            return;
        }
        let file_name = self
            .document
            .as_ref()
            .map(|source| default_output_name(&source.path, "review-mode", "mp3"))
            .unwrap_or_else(|| "review-mode.mp3".to_owned());
        let Some(destination) =
            self.pick_save_path("Save narration", &file_name, "MP3 audio", &["mp3"])
        else {
            return;
        };
        self.launch_speech(
            SpeechRequest::Api {
                config,
                text,
                destination: Some(destination),
            },
            true,
        );
    }

    pub(super) fn poll_paid_tts(&mut self, ctx: &Context) {
        while let Ok(event) = self.tts_controller.rx.try_recv() {
            if event.job_id != self.tts_controller.job_id || !self.tts_controller.is_active() {
                continue;
            }
            match event.kind {
                SpeechEventKind::Progress { completed, total } => {
                    self.tts_controller.progress = Some((completed, total));
                    self.status = if self.tts_controller.exporting {
                        format!("Creating MP3… {completed}/{total}")
                    } else if completed == 0 {
                        "Preparing narration…".to_owned()
                    } else {
                        format!("Reading aloud… {completed}/{total}")
                    };
                }
                SpeechEventKind::Playing => self.status = "Reading aloud…".to_owned(),
                SpeechEventKind::Complete(path) => {
                    self.tts_controller.active = None;
                    self.tts_controller.progress = None;
                    if let Some(path) = path {
                        self.status = format!("Created {}", path.display());
                        self.push_info_notice(self.status.clone());
                    } else {
                        self.status = "Finished reading aloud.".to_owned();
                    }
                }
                SpeechEventKind::Failed(error) => {
                    self.tts_controller.active = None;
                    self.tts_controller.progress = None;
                    self.push_error_notice(error);
                }
            }
            ctx.request_repaint();
        }
        if self.tts_controller.is_active() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    pub(super) fn draw_speech_source(&mut self, ui: &mut egui::Ui) {
        let before = self.settings.speech_provider;
        egui::ComboBox::from_id_salt("speech_source")
            .selected_text(self.settings.speech_provider.label())
            .show_ui(ui, |ui| {
                for source in [
                    SpeechProvider::System,
                    SpeechProvider::ElevenLabs,
                    SpeechProvider::OpenRouter,
                ] {
                    ui.selectable_value(&mut self.settings.speech_provider, source, source.label());
                }
            });
        if before != self.settings.speech_provider {
            self.queue_settings_save();
        }
    }

    pub(super) fn draw_speech_settings(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Read aloud").strong());
        ui.push_id("settings_speech", |ui| self.draw_speech_source(ui));
        let mut save = false;
        match self.settings.speech_provider {
            SpeechProvider::System => {
                ui.label("Uses your computer's installed speech engine. Document text stays on this computer.");
                #[cfg(target_os = "linux")]
                ui.label(
                    "Requires espeak-ng or espeak. Only LawPDF's speech stops when you click Stop.",
                );
            }
            SpeechProvider::ElevenLabs => {
                ui.label("ElevenLabs model");
                save |= ui
                    .text_edit_singleline(&mut self.settings.elevenlabs_speech_model)
                    .lost_focus();
                ui.label("ElevenLabs voice ID");
                save |= ui
                    .text_edit_singleline(&mut self.settings.elevenlabs_speech_voice)
                    .lost_focus();
                ui.hyperlink_to(
                    "Find ElevenLabs voices",
                    "https://elevenlabs.io/app/voice-library",
                );
            }
            SpeechProvider::OpenRouter => {
                ui.label("OpenRouter speech model");
                save |= ui
                    .text_edit_singleline(&mut self.settings.openrouter_speech_model)
                    .lost_focus();
                ui.label("Voice supported by that model");
                save |= ui
                    .text_edit_singleline(&mut self.settings.openrouter_speech_voice)
                    .lost_focus();
                ui.hyperlink_to(
                    "OpenRouter speech models",
                    "https://openrouter.ai/models?output_modalities=audio",
                );
            }
        }
        if self.settings.speech_provider != SpeechProvider::System {
            ui.label("Read aloud and Create MP3 send the selected article text to this provider using your key. Provider charges apply.");
            #[cfg(target_os = "linux")]
            ui.label("Playback requires ffplay (FFmpeg) or mpg123. MP3 export works without either player.");
        }
        save |= ui
            .checkbox(
                &mut self.settings.speech_include_notes,
                "Read footnotes after the body",
            )
            .changed();
        if save {
            self.queue_settings_save();
        }
    }

    pub(super) fn draw_speech_menu(&mut self, ui: &mut egui::Ui, document: &LiquidDocument) {
        ui.label(RichText::new("Narration").strong());
        self.draw_speech_source(ui);
        ui.horizontal_wrapped(|ui| self.draw_speech_actions(ui, document));
        if ui
            .checkbox(
                &mut self.settings.speech_include_notes,
                "Read footnotes after the body",
            )
            .changed()
        {
            self.queue_settings_save();
        }
        if ui.button("Speech settings…").clicked() {
            self.settings_ui.open = true;
            ui.close();
        }
    }

    fn draw_speech_actions(&mut self, ui: &mut egui::Ui, document: &LiquidDocument) {
        if self.tts_controller.is_active() {
            if ui.button("Stop narration").clicked() {
                self.stop_liquid_tts();
                self.status = "Narration stopped.".to_owned();
            }
            if let Some((completed, total)) = self.tts_controller.progress {
                if total > 0 {
                    ui.label(format!("{completed}/{total}"));
                } else {
                    ui.spinner();
                }
            }
        } else {
            let tip = if self.settings.speech_provider == SpeechProvider::System {
                "Read using the installed system voice. Text stays on this computer."
            } else {
                "Send article text to the selected speech provider and play its narration. Provider charges apply."
            };
            if ui.button("Read aloud").on_hover_text(tip).clicked() {
                self.start_liquid_tts(document);
                ui.ctx().request_repaint();
            }
            if self.settings.speech_provider != SpeechProvider::System && ui.button("Create MP3…")
                .on_hover_text("Send article text to the selected speech provider and save an MP3. Provider charges apply.").clicked() {
                self.start_paid_liquid_tts(document);
                ui.ctx().request_repaint();
            }
        }
    }

    pub(super) fn draw_liquid_tts_controls(
        &mut self,
        ui: &mut egui::Ui,
        document: &LiquidDocument,
    ) {
        ui.horizontal_wrapped(|ui| {
            if ui
                .button("Download text")
                .on_hover_text("Save the clean Review Mode text as a .txt file")
                .clicked()
            {
                self.export_review_text_dialog(document);
            }
            self.draw_speech_source(ui);
            self.draw_speech_actions(ui, document);
            if ui.small_button("Speech settings…").clicked() {
                self.settings_ui.open = true;
            }
        });
        ui.add_space(6.0);
    }
}
