//! Copy a selected quotation with reader-verified citation details.
use super::*;
use crate::review_masthead::{CitationDetails, citation_details, printed_folio};

#[derive(Clone, Copy, PartialEq, Eq)]
enum CitationStyle {
    Plain,
    Footnote,
    Markdown,
}

pub(super) struct CitationDraft {
    document_epoch: u64,
    source: String,
    pdf_pages: String,
    quote: String,
    details: CitationDetails,
    detected_journal: String,
    designation: String,
    pinpoint: String,
    style: CitationStyle,
}

impl CitationDraft {
    fn complete(&self) -> bool {
        [
            &self.details.title,
            &self.details.journal,
            &self.details.volume,
            &self.details.first_page,
            &self.details.year,
            &self.pinpoint,
        ]
        .into_iter()
        .all(|field| !field.trim().is_empty())
            && (!self.details.authors.trim().is_empty() || !self.designation.trim().is_empty())
            && !self.quote.trim().is_empty()
    }

    fn render(&self) -> Option<String> {
        if !self.complete() {
            return None;
        }
        let field = |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ");
        let authors = field(&self.details.authors);
        let designation = field(&self.designation);
        let title = field(&self.details.title);
        let journal = field(&self.details.journal);
        let volume = field(&self.details.volume);
        let first_page = field(&self.details.first_page);
        let pinpoint = field(&self.pinpoint);
        let year = field(&self.details.year);
        let prefix = [&authors, &designation]
            .into_iter()
            .filter(|part| !part.is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        let pin = if pinpoint == first_page {
            String::new()
        } else {
            format!(", {pinpoint}")
        };
        let citation = format!("{prefix}, {title}, {volume} {journal} {first_page}{pin} ({year}).");
        let quote = self.quote.trim();
        match self.style {
            CitationStyle::Plain => Some(format!("“{quote}”\n\n{citation}")),
            CitationStyle::Footnote => Some(format!("“{quote}”¹\n\n1. {citation}")),
            CitationStyle::Markdown => {
                let quotation = quote
                    .lines()
                    .map(|line| format!("> {}", escape_markdown(line)))
                    .collect::<Vec<_>>()
                    .join("\n");
                let markdown_pin = if pinpoint == first_page {
                    String::new()
                } else {
                    format!(", {}", escape_markdown(&pinpoint))
                };
                let citation = format!(
                    "{}, *{}*, {} {} {}{} ({}).",
                    escape_markdown(&prefix),
                    escape_markdown(&title),
                    escape_markdown(&volume),
                    escape_markdown(&journal),
                    escape_markdown(&first_page),
                    markdown_pin,
                    escape_markdown(&year)
                );
                Some(format!("{quotation}[^1]\n\n[^1]: {citation}"))
            }
        }
    }
}

pub(super) fn escape_markdown(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        if matches!(
            ch,
            '\\' | '*' | '_' | '[' | ']' | '<' | '>' | '`' | '#' | '~'
        ) {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

pub(super) fn page_text(document: &LoadedDocument, page: usize) -> Option<String> {
    if document
        .native_text_loaded
        .get(page)
        .copied()
        .unwrap_or(false)
        && let Some(text) = document.native_text.get(page)
        && !text.trim().is_empty()
    {
        return Some(text.clone());
    }
    document
        .text_chars
        .get(page)?
        .as_ref()
        .map(|chars| chars.iter().map(|ch| ch.ch).collect())
}

fn page_pinpoint(document: &LoadedDocument, selection: TextSelection, year: Option<u32>) -> String {
    let first =
        page_text(document, selection.start_page).and_then(|text| printed_folio(&text, year));
    let last = page_text(document, selection.end_page).and_then(|text| printed_folio(&text, year));
    match (first, last) {
        (Some(first), Some(last)) if first == last => first.to_string(),
        (Some(first), Some(last)) if first < last => format!("{first}–{last}"),
        _ => String::new(),
    }
}

pub(super) fn consume_citation_shortcut(ctx: &Context) -> bool {
    ctx.input_mut(|input| {
        let mut consumed = false;
        input.events.retain(|event| {
            let is_shortcut = matches!(event,
                egui::Event::Key { key: egui::Key::C, modifiers, pressed: true, repeat: false, .. }
                if modifiers.alt && !modifiers.shift && (modifiers.ctrl || modifiers.command || modifiers.mac_cmd));
            consumed |= is_shortcut;
            !is_shortcut
        });
        consumed
    })
}

// Egui owns label selections. Its built-in selection plugin emits their text as a
// CopyText command at end-of-pass. Snapshot the output just before that plugin
// runs and capture only its new command, leaving the real clipboard untouched.
struct ReviewCaptureRequest {
    document_epoch: u64,
    view: DocumentViewMode,
    review: std::sync::Weak<LiquidDocument>,
}

#[derive(Default)]
struct ReviewSelectionCapture {
    pending: Option<ReviewCaptureRequest>,
    commands_before_selection: Option<usize>,
    completed: Option<(ReviewCaptureRequest, Option<String>)>,
}

impl egui::Plugin for ReviewSelectionCapture {
    fn debug_name(&self) -> &'static str {
        "ReviewSelectionCapture"
    }

    fn on_end_pass(&mut self, ctx: &Context) {
        let Some(request) = self.pending.take() else {
            return;
        };
        let text = self.commands_before_selection.take().and_then(|start| {
            ctx.output_mut(|output| {
                let position = output.commands.iter().enumerate().skip(start).find_map(
                    |(index, command)| {
                        matches!(command, egui::OutputCommand::CopyText(_)).then_some(index)
                    },
                )?;
                match output.commands.remove(position) {
                    egui::OutputCommand::CopyText(text) => Some(text),
                    _ => unreachable!(),
                }
            })
        });
        self.completed = Some((request, text));
        ctx.request_repaint();
    }
}

fn ensure_review_capture_plugin(ctx: &Context) {
    if ctx.plugin_opt::<ReviewSelectionCapture>().is_some() {
        return;
    }
    ctx.add_plugin(ReviewSelectionCapture::default());
    // CallbackPlugin is built into egui before LabelSelectionState. This callback
    // records the boundary immediately before the latter appends its command.
    ctx.on_end_pass(
        "review citation selection boundary",
        Arc::new(|ctx| {
            let plugin = ctx.plugin::<ReviewSelectionCapture>();
            let mut capture = plugin.lock();
            if capture.pending.is_some() {
                capture.commands_before_selection =
                    Some(ctx.output_mut(|output| output.commands.len()));
            }
        }),
    );
}

pub(super) fn ensure_copy_event(ctx: &Context) {
    ctx.input_mut(|input| {
        if !input
            .events
            .iter()
            .any(|event| matches!(event, egui::Event::Copy))
        {
            input.events.push(egui::Event::Copy);
        }
    });
}

// Mirror the visible rendering of footnote callouts; private-use delimiters are
// internal markup and should never be copied into a quotation.
pub(super) fn review_display_text(source: &str, hide_markers: bool) -> String {
    let mut result = String::new();
    let mut marker: Option<String> = None;
    for ch in source.chars() {
        match ch {
            crate::layout_roles::CALLOUT_START => {
                if let Some(previous) = marker.take() {
                    if !hide_markers {
                        result.push_str(&review_footnote_superscript(&previous));
                    }
                }
                marker = Some(String::new());
            }
            crate::layout_roles::CALLOUT_END => {
                if let Some(previous) = marker.take() {
                    if !hide_markers {
                        result.push_str(&review_footnote_superscript(&previous));
                    }
                }
            }
            _ => {
                if let Some(current) = marker.as_mut() {
                    current.push(ch);
                } else {
                    result.push(ch);
                }
            }
        }
    }
    if let Some(previous) = marker {
        if !hide_markers {
            result.push_str(&review_footnote_superscript(&previous));
        }
    }
    result
}

fn normalized(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

// Selection state is global to egui. Require the captured text to occur in the
// current Review document, in order, before attaching this document's citation.
fn quote_belongs_to_review(quote: &str, review: &LiquidDocument, hide_markers: bool) -> bool {
    let source = std::iter::once(review.title.as_str())
        .chain(review.blocks.iter().map(|block| block.text.as_str()))
        .map(|text| normalized(&review_display_text(text, hide_markers)))
        .collect::<Vec<_>>()
        .join(" ");
    let mut remaining = source.as_str();
    let mut found_any = false;
    for line in quote
        .lines()
        .map(normalized)
        .filter(|line| !line.is_empty())
    {
        let Some(start) = remaining.find(&line) else {
            return false;
        };
        remaining = &remaining[start + line.len()..];
        found_any = true;
    }
    found_any
}

/// Infer a PDF page only when one Review block contains the quote and the
/// quote is independently present in that block's source lines and in the
/// extracted text of exactly one source page. Partial or ambiguous mappings
/// intentionally return no page.
fn review_quote_page(
    review: &LiquidDocument,
    document: &LoadedDocument,
    quote: &str,
    hide_markers: bool,
) -> Option<usize> {
    let quote = normalized(quote);
    if quote.is_empty() {
        return None;
    }
    let mut blocks = review.blocks.iter().enumerate().filter(|(_, block)| {
        normalized(&review_display_text(&block.text, hide_markers)).contains(&quote)
    });
    let (block_index, _) = blocks.next()?;
    if blocks.next().is_some() {
        return None;
    }
    let mut entries = review
        .block_source_lines
        .iter()
        .filter(|entry| entry.block_index == block_index);
    let source = entries.next()?;
    if entries.next().is_some() || source.lines.is_empty() {
        return None;
    }
    let mut pages: std::collections::BTreeMap<usize, Vec<String>> =
        std::collections::BTreeMap::new();
    for line in &source.lines {
        if line.page_index >= document.page_count {
            return None;
        }
        pages
            .entry(line.page_index)
            .or_default()
            .push(review_display_text(&line.text, hide_markers));
    }
    // Count source-page candidates before consulting loaded page text: an
    // unloaded competing page is still ambiguous, not evidence for a match.
    let mut matches = pages.into_iter().filter_map(|(page, lines)| {
        normalized(&lines.join(" "))
            .contains(&quote)
            .then_some(page)
    });
    let page = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    let original = page_text(document, page)?;
    normalized(&review_display_text(&original, hide_markers))
        .contains(&quote)
        .then_some(page)
}

impl PdfEditorApp {
    fn active_review_document(&self) -> Option<&Arc<LiquidDocument>> {
        match self.view_mode {
            DocumentViewMode::Liquid => match &self.liquid_state {
                LiquidState::Ready(doc) => Some(doc),
                _ => None,
            },
            DocumentViewMode::LiquidMode2 => match &self.liquid_mode2_state {
                LiquidState::Ready(doc) => Some(doc),
                _ => None,
            },
            DocumentViewMode::Pdf => None,
        }
    }

    pub(super) fn start_review_selection_citation(&mut self, ctx: &Context) {
        let has_label_selection = ctx
            .plugin::<egui::text_selection::LabelSelectionState>()
            .lock()
            .has_selection();
        if !has_label_selection {
            if self.selection_state.liquid_all {
                if let Some(text) = self.current_liquid_copy_text() {
                    self.start_review_quote_citation(review_display_text(
                        &text,
                        self.liquid_hide_footnotes,
                    ));
                    return;
                }
            }
            self.status = "Select Review text before copying with a citation.".to_owned();
            return;
        }
        let Some(review) = self.active_review_document() else {
            self.status = "Review text is not ready to copy.".to_owned();
            return;
        };
        let request = ReviewCaptureRequest {
            document_epoch: self.document_epoch,
            view: self.view_mode,
            review: Arc::downgrade(review),
        };
        ensure_review_capture_plugin(ctx);
        let plugin = ctx.plugin::<ReviewSelectionCapture>();
        let mut capture = plugin.lock();
        capture.pending = Some(request);
        capture.completed = None;
        drop(capture);
        ensure_copy_event(ctx);
    }

    pub(super) fn poll_review_selection_citation(&mut self, ctx: &Context) {
        let Some(plugin) = ctx.plugin_opt::<ReviewSelectionCapture>() else {
            return;
        };
        let Some((request, text)) = plugin.lock().completed.take() else {
            return;
        };
        let valid_document = request.document_epoch == self.document_epoch
            && request.view == self.view_mode
            && request
                .review
                .upgrade()
                .zip(self.active_review_document())
                .is_some_and(|(previous, current)| Arc::ptr_eq(&previous, current));
        if !valid_document {
            return;
        }
        let Some(text) = text.filter(|text| !text.trim().is_empty()) else {
            self.status = "The Review selection is no longer visible. Select it again before copying with a citation.".to_owned();
            return;
        };
        if !self.active_review_document().is_some_and(|review| {
            quote_belongs_to_review(&text, review, self.liquid_hide_footnotes)
        }) {
            self.status =
                "Select text from the Review document before copying with a citation.".to_owned();
            return;
        }
        self.start_review_quote_citation(text);
    }

    pub(super) fn start_review_quote_citation(&mut self, quote: String) {
        let full_review =
            self.view_mode != DocumentViewMode::LiquidMode2 || self.liquid_mode2_complete;
        let page = full_review
            .then(|| self.active_review_document().zip(self.document.as_ref()))
            .flatten()
            .and_then(|(review, document)| {
                review_quote_page(review, document, &quote, self.liquid_hide_footnotes)
            });
        let label = page.map_or_else(
            || "Review selection · printed page required".to_owned(),
            |page| format!("Review selection · PDF page {}", page + 1),
        );
        self.start_citation_for_quote(
            quote,
            page.map(|page| TextSelection::new(page, 0, 0)),
            label,
        );
    }

    pub(super) fn start_copy_with_citation(&mut self) {
        let Some((_document, selection, quote)) = self
            .document
            .as_ref()
            .zip(self.selection_state.text)
            .zip(self.selected_text())
            .map(|((document, selection), quote)| (document, selection, quote))
        else {
            self.status = "Select text in the original PDF and wait for it to load before copying with a citation.".to_owned();
            return;
        };
        let pdf_pages = if selection.start_page == selection.end_page {
            format!("PDF page {}", selection.start_page + 1)
        } else {
            format!(
                "PDF pages {}–{}",
                selection.start_page + 1,
                selection.end_page + 1
            )
        };
        self.start_citation_for_quote(quote, Some(selection), pdf_pages);
    }

    fn start_citation_for_quote(
        &mut self,
        quote: String,
        selection: Option<TextSelection>,
        pdf_pages: String,
    ) {
        let Some(document) = self.document.as_ref() else {
            return;
        };
        let pages = document
            .native_text
            .iter()
            .zip(&document.native_text_loaded)
            .take(crate::review_masthead::MASTHEAD_PAGES_TO_READ + 1)
            .map(|(text, loaded)| if *loaded { text.as_str() } else { "" })
            .collect::<Vec<_>>();
        let review = match &self.liquid_mode2_state {
            LiquidState::Ready(document) => Some(document.as_ref()),
            _ => None,
        };
        let mut details = citation_details(
            &pages,
            review.map(|document| document.title.as_str()),
            review
                .map(|document| document.blocks.as_slice())
                .unwrap_or_default(),
        );
        let detected_journal = std::mem::take(&mut details.journal);
        let pinpoint = selection
            .map(|selection| page_pinpoint(document, selection, details.year.parse().ok()))
            .unwrap_or_default();
        let source = document
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("PDF")
            .to_owned();
        self.citation_draft = Some(CitationDraft {
            document_epoch: self.document_epoch,
            source,
            pdf_pages,
            quote,
            details,
            detected_journal,
            designation: String::new(),
            pinpoint,
            style: CitationStyle::Plain,
        });
    }

    pub(super) fn draw_citation_copy_window(&mut self, ctx: &Context) {
        let Some(mut draft) = self.citation_draft.take() else {
            return;
        };
        if self.document.is_none() || draft.document_epoch != self.document_epoch {
            return;
        }
        let mut open = true;
        let mut copied = None;
        egui::Window::new("Copy with citation")
            .open(&mut open).collapsible(false).default_width(480.0)
            .show(ctx, |ui| {
                ui.label(format!("{} · {}", draft.source, draft.pdf_pages));
                ui.label("Check the inferred details, including the journal abbreviation and printed page. PDF page numbers may differ.");
                ui.label("For a published journal article. Plain text may need italics and small caps after pasting.");
                if !draft.detected_journal.is_empty() {
                    ui.label(format!("Journal detected: {}", draft.detected_journal));
                }
                ui.collapsing("Selected quotation", |ui| {
                    egui::ScrollArea::vertical().max_height(140.0).show(ui, |ui| { ui.label(&draft.quote); });
                });
                egui::Grid::new("citation-fields").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                    for (label, field) in [
                        ("Author(s)", &mut draft.details.authors),
                        ("Designation (optional)", &mut draft.designation),
                        ("Article title", &mut draft.details.title),
                        ("Volume", &mut draft.details.volume),
                        ("Journal abbreviation", &mut draft.details.journal),
                        ("First page", &mut draft.details.first_page),
                        ("Printed pinpoint page(s)", &mut draft.pinpoint),
                        ("Year", &mut draft.details.year),
                    ] {
                        ui.label(label);
                        ui.text_edit_singleline(field);
                        ui.end_row();
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Format:");
                    ui.selectable_value(&mut draft.style, CitationStyle::Plain, "Plain");
                    ui.selectable_value(&mut draft.style, CitationStyle::Footnote, "Footnote text");
                    ui.selectable_value(&mut draft.style, CitationStyle::Markdown, "Markdown");
                });
                if !draft.complete() { ui.label("Fill the required fields and provide an author or a designation."); }
                if ui.add_enabled(draft.complete(), egui::Button::new("Copy quotation and citation")).clicked() {
                    copied = draft.render();
                }
            });
        if let Some(text) = copied {
            self.copy_text_to_clipboard(ctx, text);
            self.status =
                "Copied quotation and citation. Check the details against the source.".to_owned();
        } else if open {
            self.citation_draft = Some(draft);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(style: CitationStyle) -> CitationDraft {
        CitationDraft {
            document_epoch: 7,
            source: "article.pdf".into(),
            pdf_pages: "PDF page 3".into(),
            quote: "A quotation\ncontinued.".into(),
            details: CitationDetails {
                authors: "A. Author".into(),
                title: "A Title".into(),
                journal: "N.Y.U. L. Rev.".into(),
                volume: "99".into(),
                first_page: "451".into(),
                year: "2024".into(),
            },
            detected_journal: "New York University Law Review".into(),
            designation: String::new(),
            pinpoint: "453".into(),
            style,
        }
    }

    #[test]
    fn citations_require_real_fields_and_support_three_formats() {
        let mut value = draft(CitationStyle::Plain);
        assert_eq!(
            value.render().unwrap(),
            "“A quotation\ncontinued.”\n\nA. Author, A Title, 99 N.Y.U. L. Rev. 451, 453 (2024)."
        );
        value.style = CitationStyle::Footnote;
        assert!(value.render().unwrap().contains("”¹\n\n1. A. Author"));
        value.style = CitationStyle::Markdown;
        value.details.title = "A *quoted* title".into();
        assert!(
            value
                .render()
                .unwrap()
                .contains("[^1]: A. Author, *A \\*quoted\\* title*, 99")
        );
        value.pinpoint.clear();
        assert!(value.render().is_none());
        value.style = CitationStyle::Plain;
        value.details.authors.clear();
        value.designation = "Note".into();
        value.details.title = "A Title".into();
        value.pinpoint = "451".into();
        assert!(
            value
                .render()
                .unwrap()
                .ends_with("Note, A Title, 99 N.Y.U. L. Rev. 451 (2024).")
        );
    }

    #[test]
    fn citation_shortcut_does_not_consume_plain_or_shift_copy() {
        let ctx = Context::default();
        let key = |modifiers, pressed| egui::Event::Key {
            key: egui::Key::C,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers,
        };
        let input = egui::RawInput {
            events: vec![
                key(egui::Modifiers::CTRL, true),
                key(egui::Modifiers::CTRL, false),
                key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, true),
                key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, false),
                key(egui::Modifiers::CTRL | egui::Modifiers::ALT, true),
                key(egui::Modifiers::CTRL | egui::Modifiers::ALT, false),
            ],
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            assert!(consume_citation_shortcut(ctx));
            assert_eq!(ctx.input(|input| input.events.len()), 5);
        });
    }

    #[test]
    fn review_selection_is_captured_without_touching_clipboard_and_rejects_stale_documents() {
        fn find_text(shape: &egui::Shape, wanted: &str) -> Option<Pos2> {
            match shape {
                egui::Shape::Text(text) if text.galley.text() == wanted => Some(text.pos),
                egui::Shape::Vec(shapes) => {
                    shapes.iter().find_map(|shape| find_text(shape, wanted))
                }
                _ => None,
            }
        }
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        let first = "First paragraph has selected words.";
        let second = "Second paragraph continues the quotation.";
        let blocks = [first, second]
            .into_iter()
            .map(|text| LiquidBlock {
                role: LiquidBlockRole::Paragraph,
                text: text.to_owned(),
                label: None,
            })
            .collect();
        app.view_mode = DocumentViewMode::LiquidMode2;
        app.liquid_mode2_state = LiquidState::Ready(Arc::new(LiquidDocument {
            title: "Article".into(),
            blocks,
            article_spans: Vec::new(),
            block_source_lines: Vec::new(),
            footnote_links: Vec::new(),
            footnote_link_integrity: None,
            profile: None,
            noise_lines_removed: 0,
            llm_used: false,
            llm_provider: None,
            deep_liquid_used: false,
            deep_liquid_model: None,
            warnings: Vec::new(),
            source_signature: "citation-test".into(),
        }));
        app.liquid_mode2_complete = true;
        app.liquid_hide_footnotes = true;
        let draw = |time, events, action: u8, app: &mut PdfEditorApp| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1100.0, 800.0))),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    if action == 1 {
                        app.copy_review_shortcut(ctx);
                    }
                    if action == 2 {
                        app.start_review_selection_citation(ctx);
                        ctx.copy_text("unrelated existing command".to_owned());
                    }
                    if action == 3 {
                        app.poll_review_selection_citation(ctx);
                    }
                    egui::CentralPanel::default()
                        .show(ctx, |ui| app.draw_liquid_mode2_document(ui, ctx));
                },
            )
        };
        let _ = draw(0.0, vec![], 0, &mut app);
        let output = draw(0.1, vec![], 0, &mut app);
        let start = output
            .shapes
            .iter()
            .find_map(|shape| find_text(&shape.shape, first))
            .unwrap()
            + Vec2::new(15.0, 8.0);
        let end = output
            .shapes
            .iter()
            .find_map(|shape| find_text(&shape.shape, second))
            .unwrap()
            + Vec2::new(80.0, 8.0);
        let _ = draw(0.2, vec![egui::Event::PointerMoved(start)], 0, &mut app);
        let _ = draw(
            0.3,
            vec![egui::Event::PointerButton {
                pos: start,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
            0,
            &mut app,
        );
        let _ = draw(0.4, vec![egui::Event::PointerMoved(end)], 0, &mut app);
        let _ = draw(
            0.5,
            vec![egui::Event::PointerButton {
                pos: end,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
            0,
            &mut app,
        );
        assert!(
            ctx.plugin::<egui::text_selection::LabelSelectionState>()
                .lock()
                .has_selection()
        );
        let ordinary = draw(0.6, vec![egui::Event::Copy], 1, &mut app);
        let selected = ordinary
            .platform_output
            .commands
            .iter()
            .find_map(|command| match command {
                egui::OutputCommand::CopyText(text) => Some(text.clone()),
                _ => None,
            })
            .expect("ordinary copy must produce the selected text");
        assert!(selected.contains("paragraph has selected words"));
        assert!(selected.contains("Second"));
        assert!(!selected.contains("Article"));
        let citation = draw(0.7, vec![], 2, &mut app);
        let clipboard_commands = citation
            .platform_output
            .commands
            .iter()
            .filter_map(|command| match command {
                egui::OutputCommand::CopyText(text) => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(clipboard_commands, vec!["unrelated existing command"]);
        assert!(app.citation_draft.is_none());
        let _ = draw(0.8, vec![], 3, &mut app);
        let draft = app
            .citation_draft
            .as_ref()
            .expect("selection should open a citation draft");
        assert_eq!(draft.quote, selected);
        assert!(draft.pinpoint.is_empty());
        assert!(draft.pdf_pages.contains("printed page required"));
        app.citation_draft = None;
        let _ = draw(0.9, vec![], 2, &mut app);
        app.document_epoch = app.document_epoch.wrapping_add(1);
        let _ = draw(1.0, vec![], 3, &mut app);
        assert!(
            app.citation_draft.is_none(),
            "a selection from another document must be discarded"
        );
    }

    #[test]
    fn review_pinpoint_requires_unique_review_source_and_original_page_evidence() {
        use crate::liquid::LiquidBlockSourceLines;
        let (mut app, _ctx, _worker) = annotation_saving::tests::fixture();
        let pdf = app.document.as_mut().unwrap();
        pdf.page_count = 2;
        pdf.pages.push(pdf.pages[0].clone());
        pdf.native_text = vec![
            "451\nSome other page.".into(),
            "452\nThe unique quotation appears here.".into(),
        ];
        pdf.native_text_loaded = vec![true, true];
        pdf.text_chars = vec![None, None];
        let line = |page, text: &str| LiquidSourceLineRef {
            id: None,
            page_index: page,
            line_index: 0,
            text: text.into(),
            role: LiquidBlockRole::Paragraph,
            note_markers: Vec::new(),
        };
        let review = LiquidDocument {
            title: "Article".into(),
            blocks: vec![LiquidBlock {
                role: LiquidBlockRole::Paragraph,
                text: "The unique quotation appears here.".into(),
                label: None,
            }],
            article_spans: Vec::new(),
            block_source_lines: vec![LiquidBlockSourceLines {
                block_index: 0,
                lines: vec![line(1, "The unique quotation appears here.")],
            }],
            footnote_links: Vec::new(),
            footnote_link_integrity: None,
            profile: None,
            noise_lines_removed: 0,
            llm_used: false,
            llm_provider: None,
            deep_liquid_used: false,
            deep_liquid_model: None,
            warnings: Vec::new(),
            source_signature: "citation-test".into(),
        };
        let quote = "unique quotation";
        assert_eq!(review_quote_page(&review, pdf, quote, false), Some(1));
        let mut duplicate_block = review.clone();
        duplicate_block
            .blocks
            .push(duplicate_block.blocks[0].clone());
        assert_eq!(review_quote_page(&duplicate_block, pdf, quote, false), None);
        let mut duplicate_mapping = review.clone();
        duplicate_mapping
            .block_source_lines
            .push(duplicate_mapping.block_source_lines[0].clone());
        assert_eq!(
            review_quote_page(&duplicate_mapping, pdf, quote, false),
            None
        );
        let mut repeated_on_unloaded_page = review.clone();
        repeated_on_unloaded_page.block_source_lines[0]
            .lines
            .push(line(0, "A unique quotation also appears here."));
        pdf.native_text_loaded[0] = false;
        assert_eq!(
            review_quote_page(&repeated_on_unloaded_page, pdf, quote, false),
            None
        );
        pdf.native_text_loaded[1] = false;
        assert_eq!(review_quote_page(&review, pdf, quote, false), None);
        pdf.native_text_loaded[1] = true;
        pdf.native_text[1] = "452\nDifferent source text.".into();
        assert_eq!(review_quote_page(&review, pdf, quote, false), None);
        pdf.native_text[1] = "452\nThe unique quotation appears here.".into();
        app.view_mode = DocumentViewMode::LiquidMode2;
        app.liquid_mode2_state = LiquidState::Ready(Arc::new(review));
        app.liquid_mode2_complete = false;
        app.start_review_quote_citation(quote.into());
        assert!(
            app.citation_draft.as_ref().unwrap().pinpoint.is_empty(),
            "partial Review preparation must not infer a page"
        );
        app.liquid_mode2_complete = true;
        app.start_review_quote_citation(quote.into());
        let draft = app.citation_draft.as_ref().unwrap();
        assert_eq!(draft.pinpoint, "452");
        assert!(draft.pdf_pages.contains("PDF page 2"));
        app.document.as_mut().unwrap().native_text[1] =
            "The unique quotation appears here.\nNo printed folio.".into();
        app.start_review_quote_citation(quote.into());
        assert!(
            app.citation_draft.as_ref().unwrap().pinpoint.is_empty(),
            "never substitute the PDF index for the printed folio"
        );
    }

    #[test]
    fn review_quote_matching_handles_superscripts_and_rejects_unrelated_text() {
        let (_app, _ctx, _worker) = annotation_saving::tests::fixture();
        let review = LiquidDocument {
            title: "Article".into(),
            blocks: vec![LiquidBlock {
                role: LiquidBlockRole::Paragraph,
                text: format!(
                    "Quoted{}12{} passage.",
                    crate::layout_roles::CALLOUT_START,
                    crate::layout_roles::CALLOUT_END
                ),
                label: None,
            }],
            article_spans: Vec::new(),
            block_source_lines: Vec::new(),
            footnote_links: Vec::new(),
            footnote_link_integrity: None,
            profile: None,
            noise_lines_removed: 0,
            llm_used: false,
            llm_provider: None,
            deep_liquid_used: false,
            deep_liquid_model: None,
            warnings: Vec::new(),
            source_signature: "citation-test".into(),
        };
        assert!(quote_belongs_to_review("Quoted¹² passage.", &review, false));
        assert!(quote_belongs_to_review("Quoted passage.", &review, true));
        assert!(!quote_belongs_to_review("Unrelated text", &review, false));
        assert!(!quote_belongs_to_review("passage.\nQuoted", &review, false));
    }

    #[test]
    fn pdf_index_is_never_used_as_a_printed_pinpoint() {
        let (mut app, _ctx, _worker) = annotation_saving::tests::fixture();
        let document = app.document.as_mut().unwrap();
        document.native_text[0] = "No printed page number here.".into();
        assert_eq!(
            page_pinpoint(document, TextSelection::new(0, 0, 1), Some(2024)),
            ""
        );
        document.native_text[0] = "453\nARTICLE\nQuoted text".into();
        assert_eq!(
            page_pinpoint(document, TextSelection::new(0, 0, 1), Some(2024)),
            "453"
        );
    }

    #[test]
    fn draft_uses_the_actual_selection_and_can_be_copied_from_the_window() {
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
        document.native_text[0] = "453\nNEW YORK UNIVERSITY\nLAW REVIEW\nVolume 99 May 2024 Number 2\nARTICLES\nGENERATIVE INTERPRETATION\nYonathan Arbel† & David A. Hoffman‡\nA quotation".into();
        document.text_chars[0] = Some(
            "A quotation"
                .chars()
                .map(|ch| PageTextChar {
                    ch,
                    rect: None,
                    font_size: None,
                    bold: false,
                    italic: false,
                })
                .collect(),
        );
        app.selection_state.text = Some(TextSelection::new(0, 0, 10));
        app.start_copy_with_citation();
        let initial = app.citation_draft.as_ref().unwrap();
        assert_eq!(initial.quote, "A quotation");
        assert_eq!(initial.pinpoint, "453");
        assert_eq!(initial.detected_journal, "New York University Law Review");
        assert!(initial.details.journal.is_empty());
        app.citation_draft = Some(draft(CitationStyle::Plain));

        let draw = |app: &mut PdfEditorApp, time, events| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 800.0))),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ctx| app.draw_citation_copy_window(ctx),
            )
        };
        let _ = draw(&mut app, 0.0, vec![]);
        let output = draw(&mut app, 0.1, vec![]);
        let point = output
            .shapes
            .iter()
            .find_map(|shape| text_center(&shape.shape, "Copy quotation and citation"))
            .expect("copy control");
        let _ = draw(&mut app, 0.2, vec![egui::Event::PointerMoved(point)]);
        let _ = draw(
            &mut app,
            0.3,
            vec![egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        let output = draw(
            &mut app,
            0.4,
            vec![egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(output.platform_output.commands.iter().any(|command| matches!(command,
            egui::OutputCommand::CopyText(text) if text.contains("99 N.Y.U. L. Rev. 451, 453 (2024)"))));
        assert!(app.citation_draft.is_none());
    }

    #[test]
    fn switching_documents_discards_a_citation_draft() {
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        app.citation_draft = Some(draft(CitationStyle::Plain));
        app.document_epoch += 1;
        let output = ctx.run(Default::default(), |ctx| app.draw_citation_copy_window(ctx));
        assert!(app.citation_draft.is_none());
        assert!(output.platform_output.commands.is_empty());
    }
}
