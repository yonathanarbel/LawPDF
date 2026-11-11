//! Export the reader's annotations without guessing missing printed pages.
use super::*;
use crate::review_masthead::{CitationDetails, citation_details, printed_folio};

pub(super) struct PendingDigest {
    epoch: u64,
    path: PathBuf,
    annotations: Vec<EditorAnnotation>,
    foreign: Arc<Vec<ForeignAnnotation>>,
    pages: Vec<usize>,
}

struct Entry {
    page: usize,
    kind: &'static str,
    foreign: bool,
    text: String,
    printed_page: Option<u32>,
}

pub(super) struct DigestDraft {
    epoch: u64,
    path: PathBuf,
    title: String,
    annotations: Vec<EditorAnnotation>,
    foreign: Arc<Vec<ForeignAnnotation>>,
    entries: Vec<Entry>,
    printed_pages: std::collections::BTreeMap<usize, String>,
    details: CitationDetails,
    detected_journal: String,
    designation: String,
    include_citations: bool,
}

fn marked_text(annotation: &EditorAnnotation, document: &LoadedDocument) -> Option<String> {
    marked_text_in_rect(annotation.page_index, annotation.rect, document)
}

fn marked_text_in_rect(page: usize, rect: PdfRect, document: &LoadedDocument) -> Option<String> {
    let chars = document.text_chars.get(page)?.as_deref()?;
    let bounds = expand_pdf_rect(rect, 0.5);
    let mut indices = chars.iter().enumerate().filter_map(|(index, ch)| {
        let rect = ch.rect?;
        let center = (
            (rect.left + rect.right) * 0.5,
            (rect.bottom + rect.top) * 0.5,
        );
        (!ch.ch.is_control() && point_in_pdf_rect(center, bounds)).then_some(index)
    });
    let first = indices.next()?;
    let last = indices.last().unwrap_or(first);
    let mut text = String::new();
    for ch in &chars[first..=last] {
        if let Some(rect) = ch.rect {
            let center = (
                (rect.left + rect.right) * 0.5,
                (rect.bottom + rect.top) * 0.5,
            );
            if !ch.ch.is_whitespace() && !ch.ch.is_control() && !point_in_pdf_rect(center, bounds) {
                return None;
            }
        }
        text.push(ch.ch);
    }
    (!text.trim().is_empty()).then(|| text.trim().to_owned())
}

fn digest_entries(
    annotations: &[EditorAnnotation],
    foreign: &[ForeignAnnotation],
    document: &LoadedDocument,
    year: Option<u32>,
) -> Vec<Entry> {
    let mut order = annotations
        .iter()
        .enumerate()
        .map(|(index, mark)| (mark.page_index, mark.rect.top, false, index))
        .chain(foreign.iter().enumerate().filter_map(|(index, mark)| {
            mark.rects
                .iter()
                .map(|rect| rect.top)
                .max_by(f32::total_cmp)
                .map(|top| (mark.page_index, top, true, index))
        }))
        .collect::<Vec<_>>();
    order.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| right.1.total_cmp(&left.1))
            .then_with(|| left.2.cmp(&right.2))
            .then_with(|| left.3.cmp(&right.3))
    });
    let mut folios = HashMap::new();
    order
        .into_iter()
        .map(|(page, _, is_foreign, index)| {
            let (kind, text) = if is_foreign {
                let annotation = &foreign[index];
                let kind = match annotation.kind {
                    ForeignAnnotationKind::Highlight => "Highlight",
                    ForeignAnnotationKind::Underline => "Underline",
                    ForeignAnnotationKind::Squiggly => "Squiggly underline",
                    ForeignAnnotationKind::StrikeOut => "Strikeout",
                    ForeignAnnotationKind::Comment => "Comment",
                    ForeignAnnotationKind::TextBox => "Text box",
                };
                let mut text = if matches!(
                    annotation.kind,
                    ForeignAnnotationKind::Comment | ForeignAnnotationKind::TextBox
                ) {
                    annotation.text.clone()
                } else if annotation.precise_text_geometry {
                    annotation
                        .rects
                        .iter()
                        .map(|rect| marked_text_in_rect(page, *rect, document))
                        .collect::<Option<Vec<_>>>()
                        .map(|parts| parts.join("\n"))
                        .unwrap_or_else(|| "[Marked text unavailable or ambiguous.]".to_owned())
                } else {
                    "[Marked text unavailable or ambiguous.]".to_owned()
                };
                if !matches!(
                    annotation.kind,
                    ForeignAnnotationKind::Comment | ForeignAnnotationKind::TextBox
                ) && !annotation.text.trim().is_empty()
                {
                    text.push_str("\nNote: ");
                    text.push_str(&annotation.text);
                }
                if !annotation.author.trim().is_empty() {
                    text.push_str("\nAuthor: ");
                    text.push_str(&annotation.author);
                }
                (kind, text)
            } else {
                match &annotations[index].kind {
                    AnnotationKind::Marker { style, .. } => (
                        match style {
                            MarkerStyle::Highlight => "Highlight",
                            MarkerStyle::Underline => "Underline",
                        },
                        marked_text(&annotations[index], document).unwrap_or_else(|| {
                            "[Marked text unavailable or ambiguous.]".to_owned()
                        }),
                    ),
                    AnnotationKind::TextBox { text, .. } => ("Text box", text.clone()),
                    AnnotationKind::Comment { text, .. } => ("Comment", text.clone()),
                    AnnotationKind::Signature { signer, .. } => ("Signature", signer.clone()),
                }
            };
            let printed_page = *folios.entry(page).or_insert_with(|| {
                citation_copy::page_text(document, page).and_then(|text| printed_folio(&text, year))
            });
            Entry {
                page,
                kind,
                foreign: is_foreign,
                text,
                printed_page,
            }
        })
        .collect()
}

fn citation_fields_complete(details: &CitationDetails, designation: &str) -> bool {
    [
        &details.title,
        &details.volume,
        &details.journal,
        &details.first_page,
        &details.year,
    ]
    .into_iter()
    .all(|field| !field.trim().is_empty())
        && (!details.authors.trim().is_empty() || !designation.trim().is_empty())
}

fn markdown_citation(details: &CitationDetails, designation: &str, page: &str) -> Option<String> {
    if !citation_fields_complete(details, designation) || page.trim().is_empty() {
        return None;
    }
    let clean = |s: &str| {
        citation_copy::escape_markdown(&s.split_whitespace().collect::<Vec<_>>().join(" "))
    };
    let prefix = [details.authors.as_str(), designation]
        .into_iter()
        .map(|value| clean(value))
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join(", ");
    let first = clean(&details.first_page);
    let pinpoint = if page.trim() == details.first_page.trim() {
        String::new()
    } else {
        format!(", {}", clean(page))
    };
    Some(format!(
        "{}, *{}*, {} {} {}{} ({}).",
        prefix,
        clean(&details.title),
        clean(&details.volume),
        clean(&details.journal),
        first,
        pinpoint,
        clean(&details.year)
    ))
}

fn digest_markdown(draft: &DigestDraft) -> String {
    let mut out = format!(
        "# Annotations: {}\n\n",
        citation_copy::escape_markdown(&draft.title)
    );
    out.push_str("LawPDF marks and supported highlights, underlines, strikeouts, comments, and text boxes from other PDF apps. Other-app marks are read-only; other annotation types may be omitted. Verify printed pages and citation details against the source.\n\n");
    let mut current_page = None;
    for entry in &draft.entries {
        let printed = draft
            .printed_pages
            .get(&entry.page)
            .map(|page| page.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|page| !page.is_empty());
        if current_page != Some(entry.page) {
            current_page = Some(entry.page);
            match printed.as_deref() {
                Some(page) => out.push_str(&format!(
                    "## PDF page {} · printed page {} (check against source)\n\n",
                    entry.page + 1,
                    citation_copy::escape_markdown(page)
                )),
                None => out.push_str(&format!(
                    "## PDF page {} · printed page not verified\n\n",
                    entry.page + 1
                )),
            }
        }
        let body = citation_copy::escape_markdown(entry.text.trim())
            .replace('\r', "")
            .replace('\n', "\n  ");
        let origin = if entry.foreign {
            " (other app, read-only)"
        } else {
            ""
        };
        out.push_str(&format!("- **{}{}:** {}", entry.kind, origin, body));
        if draft.include_citations {
            if let Some(citation) = printed
                .as_deref()
                .and_then(|page| markdown_citation(&draft.details, &draft.designation, page))
            {
                out.push_str(&format!("\n  {citation}"));
            } else {
                out.push_str("\n  Printed pinpoint unavailable; check the source.");
            }
        }
        out.push_str("\n\n");
    }
    out
}

fn safe_destination(source: &Path, destination: &Path) -> bool {
    let source = std::fs::canonicalize(source).unwrap_or_else(|_| source.to_path_buf());
    let destination =
        std::fs::canonicalize(destination).unwrap_or_else(|_| destination.to_path_buf());
    source != destination
}

impl PdfEditorApp {
    pub(super) fn start_annotation_digest(&mut self, ctx: &Context) {
        if self.pending_annotation_digest.is_some() || self.annotation_digest_draft.is_some() {
            return;
        }
        let Some(foreign) = self.foreign_annotations() else {
            self.ensure_foreign_annotations();
            self.status =
                "Wait for other applications' annotations to load before exporting.".to_owned();
            return;
        };
        let Some(document) = self.document.as_ref() else {
            return;
        };
        if self.annotations.is_empty() && foreign.is_empty() {
            self.status = "No annotations to export.".to_owned();
            return;
        }
        let mut pages = self
            .annotations
            .iter()
            .map(|annotation| annotation.page_index)
            .chain(foreign.iter().map(|annotation| annotation.page_index))
            .collect::<Vec<_>>();
        pages.sort_unstable();
        pages.dedup();
        if pages
            .iter()
            .any(|page| *page >= document.page_count || *page >= document.text_chars.len())
        {
            self.push_error_notice("An annotation refers to a page that is not in the document.");
            return;
        }
        let path = document.path.clone();
        let retry = pages
            .iter()
            .copied()
            .filter(|page| self.text_char_errors.remove(&(self.document_epoch, *page)))
            .collect::<Vec<_>>();
        if let Some(document) = self.document.as_mut() {
            for page in retry {
                document.text_chars[page] = None;
            }
        }
        self.pending_annotation_digest = Some(PendingDigest {
            epoch: self.document_epoch,
            path,
            annotations: self.annotations.clone(),
            foreign,
            pages,
        });
        self.poll_annotation_digest(ctx);
    }

    pub(super) fn annotation_digest_text_failed(&mut self, epoch: u64, path: &Path, page: usize) {
        if self
            .pending_annotation_digest
            .as_ref()
            .is_some_and(|pending| {
                pending.epoch == epoch && pending.path == path && pending.pages.contains(&page)
            })
        {
            self.pending_annotation_digest = None;
            self.status =
                "Could not prepare the annotation digest. Try again after checking the PDF."
                    .to_owned();
        }
    }

    pub(super) fn poll_annotation_digest(&mut self, ctx: &Context) {
        let Some(pending) = self.pending_annotation_digest.take() else {
            return;
        };
        if !self.is_current_document(pending.epoch, &pending.path)
            || self.annotations != pending.annotations
            || !self
                .foreign_annotations()
                .is_some_and(|marks| Arc::ptr_eq(&marks, &pending.foreign))
        {
            self.status = "Annotations changed; start the digest again.".to_owned();
            return;
        }
        let document = self.document.as_ref().expect("current document");
        let missing = pending
            .pages
            .iter()
            .copied()
            .filter(|page| document.text_chars.get(*page).is_some_and(Option::is_none))
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            let mut retry_queue = false;
            for page in missing {
                if self.pending_text_chars.contains(&page) {
                    continue;
                }
                let request = RenderRequest::TextCharsAsync {
                    document_epoch: pending.epoch,
                    path: pending.path.clone(),
                    page_index: page,
                };
                match self.render_tx.send(request) {
                    Ok(()) => {
                        self.pending_text_chars.insert(page);
                    }
                    Err(crossbeam_channel::TrySendError::Full(_)) => {
                        retry_queue = true;
                        break;
                    }
                    Err(crossbeam_channel::TrySendError::Disconnected(_)) => {
                        self.push_error_notice(
                            "The PDF worker stopped while preparing the annotation digest.",
                        );
                        return;
                    }
                }
            }
            self.status = "Preparing marked pages for the annotation digest…".to_owned();
            self.pending_annotation_digest = Some(pending);
            if retry_queue {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
            return;
        }
        let pages = document
            .native_text
            .iter()
            .zip(&document.native_text_loaded)
            .take(crate::review_masthead::MASTHEAD_PAGES_TO_READ + 1)
            .map(|(text, loaded)| if *loaded { text.as_str() } else { "" })
            .collect::<Vec<_>>();
        let review = match &self.liquid_mode2_state {
            LiquidState::Ready(review) => Some(review.as_ref()),
            _ => None,
        };
        let mut details = citation_details(
            &pages,
            review.map(|review| review.title.as_str()),
            review
                .map(|review| review.blocks.as_slice())
                .unwrap_or_default(),
        );
        let detected_journal = std::mem::take(&mut details.journal);
        let entries = digest_entries(
            &pending.annotations,
            &pending.foreign,
            document,
            details.year.parse().ok(),
        );
        let printed_pages = entries
            .iter()
            .map(|entry| {
                (
                    entry.page,
                    entry
                        .printed_page
                        .map(|page| page.to_string())
                        .unwrap_or_default(),
                )
            })
            .collect();
        self.annotation_digest_draft = Some(DigestDraft {
            epoch: pending.epoch,
            path: pending.path,
            title: document.title.clone(),
            annotations: pending.annotations,
            foreign: pending.foreign,
            entries,
            printed_pages,
            details,
            detected_journal,
            designation: String::new(),
            include_citations: true,
        });
        self.status = "Review citation details for the annotation digest.".to_owned();
    }

    pub(super) fn draw_annotation_digest_window(&mut self, ctx: &Context) {
        let Some(mut draft) = self.annotation_digest_draft.take() else {
            return;
        };
        if !self.is_current_document(draft.epoch, &draft.path)
            || self.annotations != draft.annotations
            || !self
                .foreign_annotations()
                .is_some_and(|marks| Arc::ptr_eq(&marks, &draft.foreign))
        {
            self.status = "Annotations changed; start the digest again.".to_owned();
            return;
        }
        let mut open = true;
        let mut save = false;
        egui::Window::new("Export annotation digest").open(&mut open).collapsible(false).default_width(480.0).show(ctx, |ui| {
            ui.label(format!("{} supported marks", draft.entries.len()));
            ui.label("Includes LawPDF marks and supported highlights, underlines, strikeouts, comments, and text boxes from other apps. Other types may be omitted; other-app marks are read-only. Check printed pages against the PDF; image-only marks may have no selectable text.");
            egui::CollapsingHeader::new("Review printed page numbers").default_open(true).show(ui, |ui| {
                ui.label("Correct a suggestion, fill a blank, or leave it blank when unknown.");
                let pages = draft.printed_pages.keys().copied().collect::<Vec<_>>();
                let accesskit = ui.ctx().accesskit_node_builder(ui.id(), |_| ()).is_some();
                let draw_page = |ui: &mut egui::Ui, page: usize, values: &mut std::collections::BTreeMap<usize, String>| {
                    ui.horizontal(|ui| {
                        ui.label(format!("PDF page {}", page + 1));
                        if let Some(value) = values.get_mut(&page) { ui.text_edit_singleline(value); }
                    });
                };
                let scroll = egui::ScrollArea::vertical().id_salt("digest-printed-pages").max_height(160.0);
                if accesskit || ui.ctx().memory(|memory| memory.options.screen_reader) {
                    scroll.show(ui, |ui| { for page in pages.iter().copied() { draw_page(ui, page, &mut draft.printed_pages); } });
                } else {
                    let row_height = ui.spacing().interact_size.y.max(20.0);
                    scroll.show_rows(ui, row_height, pages.len(), |ui, range| {
                        for index in range { draw_page(ui, pages[index], &mut draft.printed_pages); }
                    });
                }
            });
            ui.checkbox(&mut draft.include_citations, "Include journal-article citations when a printed page is found");
            if draft.include_citations {
                if !draft.detected_journal.is_empty() { ui.label(format!("Journal detected: {}", draft.detected_journal)); }
                ui.label("Check the inferred details and enter the journal abbreviation.");
                egui::Grid::new("digest-citation-fields").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                    for (label, field) in [("Author(s)", &mut draft.details.authors), ("Designation (optional)", &mut draft.designation),
                        ("Article title", &mut draft.details.title), ("Volume", &mut draft.details.volume),
                        ("Journal abbreviation", &mut draft.details.journal), ("First page", &mut draft.details.first_page), ("Year", &mut draft.details.year)] {
                        ui.label(label); ui.text_edit_singleline(field); ui.end_row();
                    }
                });
            }
            let ready = !draft.include_citations || citation_fields_complete(&draft.details, &draft.designation);
            if !ready { ui.label("Fill the citation fields or turn off citations."); }
            if ui.add_enabled(ready, egui::Button::new("Choose Markdown file…")).clicked() { save = true; }
        });
        if save {
            let file_name = default_output_name(&draft.path, "annotations", "md");
            if let Some(destination) =
                self.pick_save_path("Export annotation digest", &file_name, "Markdown", &["md"])
            {
                if !safe_destination(&draft.path, &destination) {
                    self.push_error_notice("Choose a Markdown file other than the original PDF.");
                } else {
                    match crate::atomic_file::write(
                        &destination,
                        digest_markdown(&draft).as_bytes(),
                    ) {
                        Ok(()) => {
                            self.status =
                                format!("Exported annotation digest to {}", destination.display());
                            return;
                        }
                        Err(error) => self.push_error_notice(format!(
                            "Could not export annotation digest: {error}"
                        )),
                    }
                }
            }
        }
        if open {
            self.annotation_digest_draft = Some(draft);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mark(page: usize, top: f32) -> EditorAnnotation {
        EditorAnnotation {
            page_index: page,
            rect: PdfRect::new(50.0, top - 12.0, 150.0, top),
            kind: AnnotationKind::Marker {
                color_rgb: [1.0, 0.9, 0.2],
                opacity: 0.4,
                style: MarkerStyle::Highlight,
            },
        }
    }

    fn text_chars(text: &str) -> Vec<PageTextChar> {
        text.chars()
            .enumerate()
            .map(|(index, ch)| PageTextChar {
                ch,
                rect: Some(PdfRect::new(
                    50.0 + index as f32 * 5.0,
                    688.0,
                    54.0 + index as f32 * 5.0,
                    700.0,
                )),
                font_size: Some(12.0),
                bold: false,
                italic: false,
            })
            .collect()
    }

    fn details() -> CitationDetails {
        CitationDetails {
            authors: "A. Author".into(),
            title: "A Title".into(),
            journal: "N.Y.U. L. Rev.".into(),
            volume: "99".into(),
            first_page: "451".into(),
            year: "2024".into(),
        }
    }

    #[test]
    fn digest_preserves_mark_text_order_and_identifies_unverified_pages() {
        let (mut app, _ctx, _worker) = annotation_saving::tests::fixture();
        let document = app.document.as_mut().unwrap();
        document.page_count = 2;
        document.native_text = vec!["453\nText".into(), "No folio".into()];
        document.native_text_loaded = vec![true, true];
        document.text_chars = vec![Some(text_chars("Quoted text")), Some(vec![])];
        let comment = EditorAnnotation {
            page_index: 0,
            rect: PdfRect::new(20.0, 200.0, 40.0, 210.0),
            kind: AnnotationKind::Comment {
                id: "comment-1".into(),
                text: "A *note*\nsecond line".into(),
                author: "Reader".into(),
                author_id: "reader-1".into(),
                color_rgb: [0.0; 3],
                created_at: String::new(),
                updated_at: String::new(),
                anchor: (50.0, 200.0),
            },
        };
        let annotations = vec![mark(1, 700.0), comment, mark(0, 700.0)];
        let entries = digest_entries(&annotations, &[], document, Some(2024));
        assert_eq!(
            entries.iter().map(|entry| entry.page).collect::<Vec<_>>(),
            vec![0, 0, 1]
        );
        assert_eq!(entries[0].text, "Quoted text");
        assert!(entries[2].text.contains("unavailable or ambiguous"));
        let mut draft = DigestDraft {
            epoch: 7,
            path: document.path.clone(),
            title: "Article".into(),
            annotations,
            foreign: Arc::new(Vec::new()),
            entries,
            printed_pages: std::collections::BTreeMap::from([
                (0, "453".into()),
                (1, String::new()),
            ]),
            details: details(),
            detected_journal: String::new(),
            designation: String::new(),
            include_citations: true,
        };
        let markdown = digest_markdown(&draft);
        assert!(markdown.contains("printed page 453 (check against source)"));
        assert!(markdown.contains("A. Author, *A Title*, 99 N.Y.U. L. Rev. 451, 453 (2024)."));
        assert!(markdown.contains("A \\*note\\*\n  second line"));
        assert!(markdown.contains("**Comment:**"));
        assert!(markdown.contains("PDF page 2 · printed page not verified"));
        assert!(markdown.contains("Printed pinpoint unavailable"));
        draft.printed_pages.insert(0, "454".into());
        let corrected = digest_markdown(&draft);
        assert!(corrected.contains("printed page 454 (check against source)"));
        assert!(corrected.contains("99 N.Y.U. L. Rev. 451, 454 (2024)."));
        draft.printed_pages.insert(0, String::new());
        assert!(digest_markdown(&draft).contains("PDF page 1 · printed page not verified"));
    }

    #[test]
    fn digest_loads_marked_page_text_before_opening_the_export_dialog() {
        let (mut app, ctx, requests) = annotation_saving::tests::fixture();
        app.set_foreign_annotations_for_test(Vec::new());
        app.document.as_mut().unwrap().text_chars[0] = None;
        app.annotations.push(mark(0, 700.0));
        let path = app.document.as_ref().unwrap().path.clone();
        let _ = ctx.run(Default::default(), |ctx| app.start_annotation_digest(ctx));
        assert!(matches!(
            requests.try_recv().unwrap(),
            RenderRequest::TextCharsAsync { page_index: 0, .. }
        ));
        assert!(app.annotation_digest_draft.is_none());
        let (sender, receiver) = crossbeam_channel::unbounded();
        app.render_rx = receiver;
        sender
            .send(RenderEvent::TextChars {
                document_epoch: 7,
                path,
                page_index: 0,
                result: Ok(text_chars("Quoted text")),
            })
            .unwrap();
        let _ = ctx.run(Default::default(), |ctx| {
            app.poll_render_results(ctx);
            app.poll_annotation_digest(ctx);
        });
        let draft = app.annotation_digest_draft.as_ref().unwrap();
        assert_eq!(draft.entries[0].text, "Quoted text");
        assert!(app.pending_annotation_digest.is_none());
    }

    #[test]
    fn printed_page_can_be_corrected_in_the_export_window() {
        fn text_center(shape: &egui::Shape, expected: &str) -> Option<Pos2> {
            match shape {
                egui::Shape::Text(text) if text.galley.text() == expected => {
                    Some(text.pos + text.galley.size() * 0.5)
                }
                egui::Shape::Vec(shapes) => {
                    shapes.iter().find_map(|shape| text_center(shape, expected))
                }
                _ => None,
            }
        }
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        app.set_foreign_annotations_for_test(Vec::new());
        let document = app.document.as_mut().unwrap();
        document.native_text[0] = "453\nText".into();
        document.text_chars[0] = Some(text_chars("Quoted text"));
        app.annotations.push(mark(0, 700.0));
        let _ = ctx.run(Default::default(), |ctx| app.start_annotation_digest(ctx));
        app.annotation_digest_draft.as_mut().unwrap().details = details();
        let draw = |app: &mut PdfEditorApp, time, events, modifiers| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 800.0))),
                    time: Some(time),
                    events,
                    modifiers,
                    ..Default::default()
                },
                |ctx| app.draw_annotation_digest_window(ctx),
            )
        };
        let _ = draw(&mut app, 0.0, vec![], egui::Modifiers::NONE);
        let output = draw(&mut app, 0.1, vec![], egui::Modifiers::NONE);
        let point = output
            .shapes
            .iter()
            .find_map(|shape| text_center(&shape.shape, "453"))
            .expect("printed page editor");
        let _ = draw(
            &mut app,
            0.2,
            vec![egui::Event::PointerMoved(point)],
            egui::Modifiers::NONE,
        );
        for (time, pressed) in [(0.3, true), (0.4, false)] {
            let _ = draw(
                &mut app,
                time,
                vec![egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }],
                egui::Modifiers::NONE,
            );
        }
        let command = egui::Modifiers::COMMAND;
        let _ = draw(
            &mut app,
            0.5,
            vec![
                egui::Event::Key {
                    key: egui::Key::A,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: command,
                },
                egui::Event::Text("454".into()),
            ],
            command,
        );
        let draft = app.annotation_digest_draft.as_ref().unwrap();
        assert_eq!(draft.printed_pages[&0], "454");
        assert!(digest_markdown(draft).contains("451, 454 (2024)"));
    }

    #[test]
    fn digest_cancels_when_annotations_change_or_page_extraction_fails() {
        let (mut app, ctx, _requests) = annotation_saving::tests::fixture();
        app.set_foreign_annotations_for_test(Vec::new());
        app.document.as_mut().unwrap().text_chars[0] = None;
        app.annotations.push(mark(0, 700.0));
        let path = app.document.as_ref().unwrap().path.clone();
        let _ = ctx.run(Default::default(), |ctx| app.start_annotation_digest(ctx));
        app.annotations.push(mark(0, 650.0));
        let _ = ctx.run(Default::default(), |ctx| app.poll_annotation_digest(ctx));
        assert!(app.pending_annotation_digest.is_none());
        assert!(app.annotation_digest_draft.is_none());
        app.pending_text_chars.clear();
        let _ = ctx.run(Default::default(), |ctx| app.start_annotation_digest(ctx));
        let (sender, receiver) = crossbeam_channel::unbounded();
        app.render_rx = receiver;
        sender
            .send(RenderEvent::TextChars {
                document_epoch: 7,
                path: path.clone(),
                page_index: 0,
                result: Err("test extraction failure".into()),
            })
            .unwrap();
        let _ = ctx.run(Default::default(), |ctx| app.poll_render_results(ctx));
        assert!(app.pending_annotation_digest.is_none());
        assert!(app.annotation_digest_draft.is_none());
        assert!(app.text_char_errors.contains(&(7, 0)));
        let _ = ctx.run(Default::default(), |ctx| app.start_annotation_digest(ctx));
        assert!(app.pending_annotation_digest.is_some());
        assert!(app.document.as_ref().unwrap().text_chars[0].is_none());
    }

    #[test]
    fn digest_does_not_include_interleaved_text_outside_a_mark() {
        let (mut app, _ctx, _worker) = annotation_saving::tests::fixture();
        let mut chars = text_chars("abc");
        chars[1].rect = Some(PdfRect::new(300.0, 688.0, 304.0, 700.0));
        app.document.as_mut().unwrap().text_chars[0] = Some(chars);
        assert!(marked_text(&mark(0, 700.0), app.document.as_ref().unwrap()).is_none());
    }

    #[test]
    fn foreign_only_digest_preserves_each_quad_and_rejects_stale_revision() {
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        let mut chars = text_chars("First");
        let mut second = text_chars("Second");
        for ch in &mut second {
            let rect = ch.rect.as_mut().unwrap();
            rect.top -= 30.0;
            rect.bottom -= 30.0;
        }
        chars.extend(second);
        app.document.as_mut().unwrap().text_chars[0] = Some(chars);
        app.set_foreign_annotations_for_test(vec![ForeignAnnotation {
            page_index: 0,
            rects: vec![
                PdfRect::new(49.0, 687.0, 90.0, 701.0),
                PdfRect::new(49.0, 657.0, 90.0, 671.0),
            ],
            precise_text_geometry: true,
            kind: ForeignAnnotationKind::Highlight,
            text: "External note".into(),
            author: "Reader".into(),
            color_rgb: [1.0, 1.0, 0.0],
            key: "other".into(),
        }]);
        let _ = ctx.run(Default::default(), |ctx| app.start_annotation_digest(ctx));
        let draft = app.annotation_digest_draft.as_ref().unwrap();
        assert_eq!(
            draft.entries[0].text,
            "First\nSecond\nNote: External note\nAuthor: Reader"
        );
        assert!(digest_markdown(draft).contains("Highlight (other app, read-only)"));
        app.annotation_sessions.get_mut(&7).unwrap().revision.sha256 = "changed".into();
        let _ = ctx.run(Default::default(), |ctx| {
            app.draw_annotation_digest_window(ctx)
        });
        assert!(app.annotation_digest_draft.is_none());
        assert!(app.status.contains("changed"));
    }

    #[test]
    fn foreign_bounding_rectangle_does_not_guess_a_quotation() {
        let (mut app, _ctx, _worker) = annotation_saving::tests::fixture();
        app.document.as_mut().unwrap().text_chars[0] = Some(text_chars("Unrelated text"));
        let mark = ForeignAnnotation {
            page_index: 0,
            rects: vec![PdfRect::new(49.0, 687.0, 150.0, 701.0)],
            precise_text_geometry: false,
            kind: ForeignAnnotationKind::Highlight,
            text: "Reader note".into(),
            author: String::new(),
            color_rgb: [1.0; 3],
            key: "bbox".into(),
        };
        let entries = digest_entries(&[], &[mark], app.document.as_ref().unwrap(), None);
        assert_eq!(
            entries[0].text,
            "[Marked text unavailable or ambiguous.]\nNote: Reader note"
        );
    }

    #[test]
    fn digest_never_overwrites_the_source_pdf() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source.pdf");
        std::fs::write(&source, b"source").unwrap();
        assert!(!safe_destination(&source, &source));
        assert!(safe_destination(
            &source,
            &directory.path().join("notes.md")
        ));
    }
}
