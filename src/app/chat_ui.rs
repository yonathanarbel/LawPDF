use super::*;

const CHAT_CONTEXT_TOKEN_LIMIT: usize = 64_000;

#[derive(Debug, Clone)]
pub(super) struct ChatState {
    pub(super) messages: Vec<ChatMessage>,
    pub(super) input: String,
    pub(super) model_index: usize,
    pub(super) in_flight: bool,
    pending_request: Option<u64>,
    pub(super) document_context: Option<String>,
    pub(super) context_estimated_tokens: Option<usize>,
    pub(super) context_warning: Option<String>,
}

impl Default for ChatState {
    fn default() -> Self {
        Self {
            messages: Vec::new(),
            input: String::new(),
            model_index: 0,
            in_flight: false,
            pending_request: None,
            document_context: None,
            context_estimated_tokens: None,
            context_warning: None,
        }
    }
}

impl ChatState {
    fn attach_context(&mut self, context: String, estimated_tokens: usize) -> Result<(), String> {
        self.context_estimated_tokens = Some(estimated_tokens);
        self.document_context = None;
        let error = if context.trim().is_empty() {
            Some("Chat needs PDF text or OCR text first.".to_owned())
        } else if estimated_tokens > CHAT_CONTEXT_TOKEN_LIMIT {
            Some(format!(
                "This PDF is about {estimated_tokens} tokens, above the {CHAT_CONTEXT_TOKEN_LIMIT}-token chat limit. Open a shorter PDF or extract the relevant pages to chat about them. Your question has not been sent."
            ))
        } else {
            None
        };
        self.context_warning = error.clone();
        if let Some(error) = error {
            return Err(error);
        }
        self.document_context = Some(context);
        Ok(())
    }

    fn accepts(&self, event: &ChatEvent) -> bool {
        self.in_flight && self.pending_request == Some(event.request_id)
    }
}

pub(super) struct ChatUi {
    next_request_id: u64,
    tx: Sender<ChatEvent>,
    rx: Receiver<ChatEvent>,
    pub(super) state: ChatState,
}

impl ChatUi {
    pub(super) fn new() -> Self {
        let (tx, rx) = unbounded();
        Self {
            next_request_id: 0,
            tx,
            rx,
            state: ChatState::default(),
        }
    }
}

impl PdfEditorApp {
    fn send_chat_message(&mut self, ctx: &Context) {
        if self.document.is_none() {
            return;
        }
        if self.chat_ui.state.in_flight {
            return;
        }

        let user_message = self.chat_ui.state.input.trim().to_owned();
        if user_message.is_empty() {
            return;
        }

        let Some(api_key) = effective_openrouter_api_key(&self.settings) else {
            self.status = "Chat needs an OpenRouter API key.".to_owned();
            self.settings_ui.open = true;
            return;
        };

        if self.chat_ui.state.messages.is_empty() && self.chat_ui.state.document_context.is_none() {
            if !self.ensure_native_text_loaded_for_all(ctx, "Preparing PDF text for chat") {
                self.status = "Preparing PDF text for chat; send again when ready.".to_owned();
                return;
            }

            let Some(document) = self.document.as_ref() else {
                return;
            };
            let (context, estimated_tokens) = self.chat_context_for_document(document);
            if let Err(error) = self.chat_ui.state.attach_context(context, estimated_tokens) {
                self.status = error;
                return;
            }
        }

        self.chat_ui.state.messages.push(ChatMessage {
            role: ChatRole::User,
            content: user_message,
        });
        self.chat_ui.state.input.clear();
        self.chat_ui.state.in_flight = true;
        self.chat_ui.next_request_id = self.chat_ui.next_request_id.wrapping_add(1);
        let request_id = self.chat_ui.next_request_id;
        self.chat_ui.state.pending_request = Some(request_id);

        let Some(document) = self.document.as_ref() else {
            return;
        };
        let document_path = document.path.clone();
        let model = CHAT_MODELS
            .get(self.chat_ui.state.model_index)
            .unwrap_or(&CHAT_MODELS[0])
            .id
            .to_owned();
        spawn_chat_job(
            ChatRequest {
                request_id,
                document_epoch: self.document_epoch,
                path: document_path,
                api_key,
                model,
                visible_messages: self.chat_ui.state.messages.clone(),
                document_context: self.chat_ui.state.document_context.clone(),
            },
            self.chat_ui.tx.clone(),
        );
        self.status = "Chat request sent.".to_owned();
    }

    pub(super) fn poll_chat_results(&mut self, ctx: &Context) {
        while let Ok(event) = self.chat_ui.rx.try_recv() {
            let mut error_notice = None;
            if self.is_current_document(event.document_epoch, &event.path) {
                if !self.chat_ui.state.accepts(&event) {
                    continue;
                }
                self.chat_ui.state.in_flight = false;
                self.chat_ui.state.pending_request = None;
                match event.result {
                    Ok(content) => {
                        self.chat_ui.state.messages.push(ChatMessage {
                            role: ChatRole::Assistant,
                            content,
                        });
                        self.status = "Chat response ready.".to_owned();
                    }
                    Err(error) => {
                        let message = format!("Chat failed: {error}");
                        self.chat_ui.state.messages.push(ChatMessage {
                            role: ChatRole::Assistant,
                            content: message.clone(),
                        });
                        error_notice = Some(message);
                    }
                }
                ctx.request_repaint();
            } else if let Some(tab) = self.tabs.iter_mut().find(|tab| {
                tab.document_epoch == event.document_epoch && tab.document.path == event.path
            }) {
                if !tab.chat_state.accepts(&event) {
                    continue;
                }
                tab.chat_state.in_flight = false;
                tab.chat_state.pending_request = None;
                match event.result {
                    Ok(content) => {
                        tab.chat_state.messages.push(ChatMessage {
                            role: ChatRole::Assistant,
                            content,
                        });
                        tab.status = "Chat response ready.".to_owned();
                    }
                    Err(error) => {
                        let message = format!("Chat failed: {error}");
                        tab.chat_state.messages.push(ChatMessage {
                            role: ChatRole::Assistant,
                            content: message.clone(),
                        });
                        tab.status = error;
                        error_notice = Some(message);
                    }
                }
            }
            if let Some(error) = error_notice {
                self.push_error_notice(error);
            }
        }
    }

    fn chat_context_for_document(&self, document: &LoadedDocument) -> (String, usize) {
        // Original page text retains page provenance and cannot be truncated to a
        // progressive Review preview or lose content hidden by a classifier.
        let pages = self.collect_liquid_source_pages(document);
        let mut context = String::new();
        for (page_index, page) in pages.iter().enumerate() {
            let text = page.trim();
            if text.is_empty() {
                continue;
            }
            context.push_str(&format!("\n\n--- Page {} ---\n", page_index + 1));
            context.push_str(text);
        }
        let estimated_tokens = estimate_tokens(&context);
        (context, estimated_tokens)
    }

    fn chat_context_summary(&self) -> (usize, usize) {
        let Some(document) = self.document.as_ref() else {
            return (0, 0);
        };
        let pages = self.collect_liquid_source_pages(document);
        let mut page_count = 0usize;
        let mut chars = 0usize;
        for page in pages {
            let text = page.trim();
            if !text.is_empty() {
                page_count += 1;
                chars += text.chars().count();
            }
        }
        (page_count, (chars / 4).max(1))
    }

    pub(super) fn draw_chat_tab(&mut self, ui: &mut egui::Ui) {
        let has_document = self.document.is_some();
        if !has_document {
            ui.label(RichText::new("No PDF loaded").color(MUTED_INK));
            return;
        }

        ui.horizontal(|ui| {
            ui.label(RichText::new("Model").color(INK));
            let selected = CHAT_MODELS
                .get(self.chat_ui.state.model_index)
                .unwrap_or(&CHAT_MODELS[0]);
            egui::ComboBox::from_id_salt("chat_model_selector")
                .selected_text(selected.label)
                .width(180.0)
                .show_ui(ui, |ui| {
                    for (index, model) in CHAT_MODELS.iter().enumerate() {
                        ui.selectable_value(
                            &mut self.chat_ui.state.model_index,
                            index,
                            model.label,
                        )
                        .on_hover_text(model.id);
                    }
                });
        });

        let (context_pages, context_estimate) = self.chat_context_summary();
        let context_status = if let Some(tokens) = self.chat_ui.state.context_estimated_tokens {
            if self.chat_ui.state.document_context.is_some() {
                format!("PDF context attached: ~{tokens} tokens")
            } else {
                format!("PDF context not attached: ~{tokens} tokens")
            }
        } else {
            format!("PDF text available: {context_pages} page(s), ~{context_estimate} tokens")
        };
        ui.label(RichText::new(context_status).color(MUTED_INK));
        if let Some(warning) = self.chat_ui.state.context_warning.as_ref() {
            ui.label(RichText::new(warning).color(Color32::from_rgb(134, 92, 34)));
        }

        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !self.chat_ui.state.messages.is_empty() || self.chat_ui.state.in_flight,
                    egui::Button::new("Clear"),
                )
                .clicked()
            {
                self.chat_ui.state = ChatState {
                    model_index: self.chat_ui.state.model_index,
                    ..ChatState::default()
                };
            }
            if self.chat_ui.state.in_flight {
                ui.spinner();
            }
        });

        ui.add_space(8.0);
        egui::ScrollArea::vertical()
            .id_salt("chat_messages")
            .max_height(340.0)
            .stick_to_bottom(true)
            .show(ui, |ui| {
                if self.chat_ui.state.messages.is_empty() {
                    ui.label(RichText::new("Ask a question about this PDF.").color(MUTED_INK));
                }
                for message in &self.chat_ui.state.messages {
                    let (label, color) = match message.role {
                        ChatRole::User => ("You", Color32::from_rgb(93, 68, 37)),
                        ChatRole::Assistant => ("LawPDF", INK),
                    };
                    ui.label(RichText::new(label).strong().color(color));
                    egui::Frame::NONE
                        .fill(if message.role == ChatRole::User {
                            Color32::from_rgb(247, 243, 235)
                        } else {
                            Color32::from_rgb(255, 254, 250)
                        })
                        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(222, 216, 205)))
                        .corner_radius(6)
                        .inner_margin(Margin::symmetric(8, 6))
                        .show(ui, |ui| {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&message.content).size(14.0).color(INK),
                                )
                                .wrap(),
                            );
                        });
                    ui.add_space(8.0);
                }
            });

        ui.add_space(8.0);
        ui.add(
            egui::TextEdit::multiline(&mut self.chat_ui.state.input)
                .hint_text("Ask about the PDF")
                .desired_rows(4)
                .lock_focus(true),
        );
        if ui
            .add_enabled(!self.chat_ui.state.in_flight, egui::Button::new("Send"))
            .clicked()
        {
            self.send_chat_message(ui.ctx());
        }
    }
}

pub(super) fn estimate_tokens(text: &str) -> usize {
    (text.chars().count() / 4).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oversized_or_empty_context_keeps_the_unsent_question() {
        let mut state = ChatState {
            input: "What does the last page say?".to_owned(),
            ..ChatState::default()
        };
        assert!(
            state
                .attach_context("large PDF".to_owned(), CHAT_CONTEXT_TOKEN_LIMIT + 1)
                .is_err()
        );
        assert!(state.document_context.is_none());
        assert!(state.messages.is_empty());
        assert!(!state.in_flight);
        assert_eq!(state.input, "What does the last page say?");
        assert!(
            state
                .context_warning
                .as_ref()
                .unwrap()
                .contains("not been sent")
        );
        assert!(state.attach_context(" ".to_owned(), 1).is_err());
        state
            .attach_context("complete PDF text".to_owned(), CHAT_CONTEXT_TOKEN_LIMIT)
            .unwrap();
        assert!(state.context_warning.is_none());
        assert_eq!(state.document_context.as_deref(), Some("complete PDF text"));
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn chat_uses_all_source_pages_even_when_review_is_only_a_preview() {
        let ctx = Context::default();
        let mut app = PdfEditorApp::new(&ctx, Vec::new(), unbounded().1);
        let document = LoadedDocument {
            path: PathBuf::from("context-fixture.pdf"),
            title: "Three pages".to_owned(),
            page_count: 3,
            pages: vec![crate::model::PageInfo::new(612.0, 792.0); 3],
            native_text: vec![
                "Opening text".to_owned(),
                String::new(),
                "Last-page conclusion".to_owned(),
            ],
            native_text_loaded: vec![true; 3],
            text_chars: vec![None; 3],
            links: vec![Vec::new(); 3],
            links_loaded: false,
            optimized: true,
        };
        let mut review: LiquidDocument = serde_json::from_value(serde_json::json!({
            "title": "Preview", "blocks": [], "llm_used": false, "warnings": [], "source_signature": "preview"
        })).unwrap();
        review.blocks.push(LiquidBlock {
            role: LiquidBlockRole::Paragraph,
            text: "Only an opening preview".to_owned(),
            label: None,
        });
        app.liquid_mode2_state = LiquidState::Ready(std::sync::Arc::new(review));
        app.liquid_mode2_complete = false;
        let (context, _) = app.chat_context_for_document(&document);
        assert!(context.contains("--- Page 1 ---\nOpening text"));
        assert!(context.contains("--- Page 3 ---\nLast-page conclusion"));
        assert!(!context.contains("opening preview"));
        assert!(!context.contains("--- Page 2 ---"));
    }

    #[test]
    fn cleared_or_restarted_chats_reject_old_responses() {
        let event = ChatEvent {
            request_id: 1,
            document_epoch: 7,
            path: PathBuf::from("test.pdf"),
            result: Ok("stale answer".to_owned()),
        };
        let mut state = ChatState {
            in_flight: true,
            pending_request: Some(1),
            ..ChatState::default()
        };
        assert!(state.accepts(&event));
        state = ChatState::default();
        assert!(!state.accepts(&event));
        state.in_flight = true;
        state.pending_request = Some(2);
        assert!(!state.accepts(&event));
    }
}
