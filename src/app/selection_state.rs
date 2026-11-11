use super::*;

#[derive(Default)]
pub(super) struct SelectionState {
    pub(super) text: Option<TextSelection>,
    pub(super) liquid_all: bool,
    pub(super) anchor: Option<(usize, usize)>,
    /// Fixed end for Shift-click, including when the original drag went up.
    pub(super) extend_anchor: Option<(usize, usize)>,
    pub(super) toolbar_rect: Option<Rect>,
    pub(super) pending_select_all: bool,
    pub(super) pending_mark: bool,
}

pub(super) fn selected_text(
    pages: &[Option<Vec<PageTextChar>>],
    selection: TextSelection,
) -> Option<String> {
    let mut text = String::new();
    for page_index in selection.page_range() {
        let chars = pages.get(page_index)?.as_ref()?;
        let Some((start, end)) = selection.bounds_for_page(page_index, chars.len()) else {
            continue;
        };
        if !text.is_empty() && !text.ends_with('\n') && chars[start].ch != '\n' {
            text.push('\n');
        }
        text.extend(chars[start..=end].iter().map(|character| character.ch));
    }
    (!text.is_empty()).then_some(text)
}

pub(super) fn word_bounds(chars: &[PageTextChar], index: usize) -> Option<(usize, usize)> {
    let character = chars.get(index)?.ch;
    let in_word = |c: char| c.is_alphanumeric() || matches!(c, '_' | '\'' | '’');
    if !in_word(character) {
        return (!character.is_whitespace()).then_some((index, index));
    }
    let mut start = index;
    let mut end = index;
    while start > 0 && in_word(chars[start - 1].ch) {
        start -= 1;
    }
    while end + 1 < chars.len() && in_word(chars[end + 1].ch) {
        end += 1;
    }
    Some((start, end))
}

#[derive(Clone, Copy)]
struct TextLine {
    start: usize,
    end: usize,
    breaks_before: usize,
    rect: Option<PdfRect>,
}

/// Estimate a page-local paragraph when no matching Review block is available.
/// PDF text does not carry reliable paragraph boundaries, so use visible gaps,
/// indentation, and explicit blank lines; leave unlocated lines separate when
/// there is no layout evidence.
pub(super) fn paragraph_bounds(chars: &[PageTextChar], index: usize) -> Option<(usize, usize)> {
    let mut lines = Vec::<TextLine>::new();
    let mut at = 0;
    let mut breaks = 0;
    while at < chars.len() {
        while at < chars.len() && chars[at].ch.is_whitespace() {
            if chars[at].ch == '\r' {
                breaks += 1;
                if chars.get(at + 1).is_some_and(|ch| ch.ch == '\n') {
                    at += 1;
                }
            } else if chars[at].ch == '\n' {
                breaks += 1;
            }
            at += 1;
        }
        if at == chars.len() {
            break;
        }
        let start = at;
        while at < chars.len() && !matches!(chars[at].ch, '\r' | '\n') {
            at += 1;
        }
        let mut end = at - 1;
        while end > start && chars[end].ch.is_whitespace() {
            end -= 1;
        }
        let rect = chars[start..=end]
            .iter()
            .filter_map(|ch| ch.rect)
            .reduce(|a, b| {
                PdfRect::new(
                    a.left.min(b.left),
                    a.bottom.min(b.bottom),
                    a.right.max(b.right),
                    a.top.max(b.top),
                )
            });
        lines.push(TextLine {
            start,
            end,
            breaks_before: breaks,
            rect,
        });
        breaks = 0;
    }
    let selected = lines
        .iter()
        .position(|line| line.start <= index && index <= line.end)?;
    let mut heights = lines
        .iter()
        .filter_map(|line| line.rect.map(PdfRect::height))
        .filter(|height| height.is_finite() && *height > 0.0)
        .collect::<Vec<_>>();
    heights.sort_by(f32::total_cmp);
    let height = heights.get(heights.len() / 2).copied().unwrap_or(10.0);
    let max_width = lines
        .iter()
        .filter_map(|line| line.rect.map(PdfRect::width))
        .filter(|width| width.is_finite())
        .fold(0.0_f32, f32::max);
    let left = lines
        .iter()
        .filter_map(|line| line.rect.map(|rect| rect.left))
        .filter(|left| left.is_finite())
        .fold(f32::INFINITY, f32::min);
    let explicit_paragraphs = lines.iter().any(|line| line.breaks_before > 1);
    let boundary = |previous: TextLine, next: TextLine| {
        if next.breaks_before > 1 {
            return true;
        }
        let (Some(a), Some(b)) = (previous.rect, next.rect) else {
            return !explicit_paragraphs;
        };
        if a.bottom - b.top > (height * 0.65).max(5.0)
            || b.top - a.top > height * 0.6
            || (b.left - a.left).abs() > max_width * 0.5
        {
            return true;
        }
        if b.left - left > (height * 0.85).max(7.0) && a.left - left < (height * 0.55).max(4.0) {
            return true;
        }
        let ending = chars[previous.start..=previous.end]
            .iter()
            .rev()
            .map(|ch| ch.ch)
            .find(|ch| {
                !ch.is_whitespace() && !ch.is_ascii_digit() && !matches!(ch, ')' | ']' | '”' | '’')
            });
        max_width > 0.0
            && a.width() < max_width * 0.68
            && ending.is_some_and(|ch| matches!(ch, '.' | '!' | '?' | ':'))
    };
    let mut first = selected;
    while first > 0 && !boundary(lines[first - 1], lines[first]) {
        first -= 1;
    }
    let mut last = selected;
    while last + 1 < lines.len() && !boundary(lines[last], lines[last + 1]) {
        last += 1;
    }
    Some((lines[first].start, lines[last].end))
}

/// Use Review's mapped source lines when they identify one unambiguous block
/// and every page of that block already has character geometry.
fn mapped_paragraph(
    loaded: &LoadedDocument,
    review: &LiquidDocument,
    geometry: &HashMap<(usize, usize), PdfRect>,
    page: usize,
    index: usize,
) -> Option<TextSelection> {
    let clicked = loaded.text_chars.get(page)?.as_ref()?.get(index)?.rect?;
    let center = (
        (clicked.left + clicked.right) * 0.5,
        (clicked.bottom + clicked.top) * 0.5,
    );
    let mut matching = review.block_source_lines.iter().filter(|source| {
        review.blocks.get(source.block_index).is_some_and(|block| {
            matches!(
                block.role,
                LiquidBlockRole::Paragraph
                    | LiquidBlockRole::Lead
                    | LiquidBlockRole::Abstract
                    | LiquidBlockRole::Quote
                    | LiquidBlockRole::ListItem
                    | LiquidBlockRole::Clause
                    | LiquidBlockRole::Footnote
                    | LiquidBlockRole::Marginalia
            )
        }) && source.lines.iter().any(|line| {
            line.page_index == page
                && geometry
                    .get(&(line.page_index, line.line_index))
                    .is_some_and(|rect| point_in_pdf_rect(center, *rect))
        })
    });
    let source = matching.next()?;
    if matching.next().is_some() {
        return None;
    }
    let mut positions = Vec::new();
    let mut pages = source
        .lines
        .iter()
        .map(|line| line.page_index)
        .collect::<Vec<_>>();
    pages.sort_unstable();
    pages.dedup();
    if pages
        .windows(2)
        .any(|pair| pair[1] != pair[0].saturating_add(1))
    {
        return None;
    }
    for page_index in pages {
        let rects = source
            .lines
            .iter()
            .filter(|line| line.page_index == page_index)
            .map(|line| geometry.get(&(page_index, line.line_index)).copied())
            .collect::<Option<Vec<_>>>()?;
        let chars = loaded.text_chars.get(page_index)?.as_ref()?;
        let matched = chars
            .iter()
            .enumerate()
            .filter_map(|(index, ch)| {
                let rect = ch.rect?;
                let center = (
                    (rect.left + rect.right) * 0.5,
                    (rect.bottom + rect.top) * 0.5,
                );
                (!ch.ch.is_control() && rects.iter().any(|rect| point_in_pdf_rect(center, *rect)))
                    .then_some(index)
            })
            .collect::<Vec<_>>();
        positions.push((page_index, *matched.first()?));
        positions.push((page_index, *matched.last()?));
    }
    let start = *positions.iter().min()?;
    let end = *positions.iter().max()?;
    ((start..=end).contains(&(page, index)))
        .then_some(TextSelection::range(start.0, start.1, end.0, end.1))
}

impl PdfEditorApp {
    pub(super) fn mapped_paragraph_selection(
        &mut self,
        page: usize,
        index: usize,
    ) -> Option<TextSelection> {
        let review = match &self.liquid_mode2_state {
            LiquidState::Ready(document) => Arc::clone(document),
            _ => return None,
        };
        let geometry = self.review_source_geometry_for_current_document()?;
        mapped_paragraph(self.document.as_ref()?, &review, &geometry, page, index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chars(text: &str) -> Vec<PageTextChar> {
        text.chars()
            .map(|ch| PageTextChar {
                ch,
                rect: None,
                font_size: None,
                bold: false,
                italic: false,
            })
            .collect()
    }

    #[test]
    fn selection_copies_across_pages_in_both_drag_directions() {
        let pages = vec![
            Some(chars("first page ending")),
            Some(chars("next page begins")),
        ];
        let forward = TextSelection::range(0, 11, 1, 3);
        let backward = TextSelection::range(1, 3, 0, 11);
        assert_eq!(
            selected_text(&pages, forward).as_deref(),
            Some("ending\nnext")
        );
        assert_eq!(
            selected_text(&pages, backward),
            selected_text(&pages, forward)
        );
        assert!(selected_text(&[pages[0].clone(), None], forward).is_none());
    }

    #[test]
    fn double_click_word_bounds_respect_whitespace_and_unicode() {
        let text = chars("The court’s ruling.");
        let (start, end) = word_bounds(&text, 6).unwrap();
        assert_eq!(
            text[start..=end].iter().map(|c| c.ch).collect::<String>(),
            "court’s"
        );
        assert_eq!(word_bounds(&text, 3), None);
    }

    #[test]
    fn paragraph_bounds_respect_explicit_blank_lines_without_geometry() {
        let text = chars("First line\r\nstill first.\r\n\r\nSecond paragraph.");
        let (start, end) = paragraph_bounds(&text, 2).unwrap();
        assert_eq!(
            text[start..=end].iter().map(|c| c.ch).collect::<String>(),
            "First line\r\nstill first."
        );
        let unlocated = chars("one line\nanother line");
        let (start, end) = paragraph_bounds(&unlocated, 2).unwrap();
        assert_eq!(
            unlocated[start..=end]
                .iter()
                .map(|c| c.ch)
                .collect::<String>(),
            "one line"
        );
    }

    #[test]
    fn paragraph_bounds_use_indent_and_line_geometry() {
        let rows = [
            (
                72.0,
                700.0,
                "First paragraph is long enough to fill the line",
            ),
            (60.0, 686.0, "and ends here."),
            (72.0, 672.0, "The next paragraph also has a full first line"),
            (60.0, 658.0, "and finishes here."),
        ];
        let mut text = Vec::new();
        for (x, bottom, line) in rows {
            if !text.is_empty() {
                text.extend(chars("\r\n"));
            }
            text.extend(line.chars().enumerate().map(|(index, ch)| PageTextChar {
                ch,
                rect: Some(PdfRect::new(
                    x + index as f32 * 5.0,
                    bottom,
                    x + (index + 1) as f32 * 5.0,
                    bottom + 10.0,
                )),
                font_size: Some(10.0),
                bold: false,
                italic: false,
            }));
        }
        let cursor = text.iter().position(|ch| ch.ch == 'n').unwrap();
        let (start, end) = paragraph_bounds(&text, cursor).unwrap();
        assert_eq!(
            text[start..=end].iter().map(|ch| ch.ch).collect::<String>(),
            "First paragraph is long enough to fill the line\r\nand ends here."
        );
        let cursor = text.iter().rposition(|ch| ch.ch == 'f').unwrap();
        let (start, end) = paragraph_bounds(&text, cursor).unwrap();
        assert_eq!(
            text[start..=end].iter().map(|ch| ch.ch).collect::<String>(),
            "The next paragraph also has a full first line\r\nand finishes here."
        );
    }

    #[test]
    fn paragraph_estimate_works_with_pdfium_extracted_text() {
        use lopdf::{Document, Object, Stream, dictionary};
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("paragraphs.pdf");
        let mut pdf = Document::with_version("1.5");
        let catalog = pdf.new_object_id();
        let pages = pdf.new_object_id();
        let page = pdf.new_object_id();
        let font = pdf.add_object(
            dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" },
        );
        let contents = pdf.add_object(Stream::new(dictionary! {}, b"BT /F1 12 Tf 1 0 0 1 72 700 Tm (First paragraph has a long opening line for selection.) Tj 1 0 0 1 60 685 Tm (It ends here.) Tj 1 0 0 1 72 670 Tm (Second paragraph begins on its own indented line.) Tj 1 0 0 1 60 655 Tm (It also ends here.) Tj ET".to_vec()));
        pdf.objects.insert(
            catalog,
            Object::Dictionary(dictionary! { "Type" => "Catalog", "Pages" => pages }),
        );
        pdf.objects.insert(pages, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => vec![Object::Reference(page)], "Count" => 1 }));
        pdf.objects.insert(page, Object::Dictionary(dictionary! { "Type" => "Page", "Parent" => pages,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } }, "Contents" => contents }));
        pdf.trailer.set("Root", catalog);
        pdf.save(&path).unwrap();
        let chars = PdfEngine::new()
            .unwrap()
            .load_page_text_chars(&path, 0)
            .unwrap();
        let index = chars.iter().position(|ch| ch.ch == 'F').unwrap();
        let (start, end) = paragraph_bounds(&chars, index).unwrap();
        let selected = chars[start..=end]
            .iter()
            .map(|ch| ch.ch)
            .collect::<String>();
        assert!(
            selected.contains("First paragraph has a long opening line"),
            "{selected:?}"
        );
        assert!(selected.contains("It ends here."), "{selected:?}");
        assert!(!selected.contains("Second paragraph"), "{selected:?}");
    }

    #[test]
    fn mapped_paragraph_uses_all_available_source_pages_and_rejects_ambiguity() {
        use crate::liquid::LiquidBlockSourceLines;
        let (mut app, _ctx, _worker) = annotation_saving::tests::fixture();
        let loaded = app.document.as_mut().unwrap();
        loaded.page_count = 2;
        loaded.text_chars = vec![Some(chars("abc")), Some(chars("def"))];
        for page in 0..2 {
            for (index, ch) in loaded.text_chars[page]
                .as_mut()
                .unwrap()
                .iter_mut()
                .enumerate()
            {
                ch.rect = Some(PdfRect::new(
                    70.0 + index as f32 * 6.0,
                    600.0,
                    75.0 + index as f32 * 6.0,
                    610.0,
                ));
            }
        }
        let line = |page_index| LiquidSourceLineRef {
            id: None,
            page_index,
            line_index: 0,
            text: String::new(),
            role: LiquidBlockRole::Paragraph,
            note_markers: vec![],
        };
        let mut review = LiquidDocument {
            title: String::new(),
            blocks: vec![LiquidBlock {
                role: LiquidBlockRole::Paragraph,
                text: "abcdef".into(),
                label: None,
            }],
            article_spans: vec![],
            block_source_lines: vec![LiquidBlockSourceLines {
                block_index: 0,
                lines: vec![line(0), line(1)],
            }],
            footnote_links: vec![],
            footnote_link_integrity: None,
            profile: None,
            noise_lines_removed: 0,
            llm_used: false,
            llm_provider: None,
            deep_liquid_used: false,
            deep_liquid_model: None,
            warnings: vec![],
            source_signature: String::new(),
        };
        let geometry = HashMap::from([
            ((0, 0), PdfRect::new(69.0, 599.0, 90.0, 611.0)),
            ((1, 0), PdfRect::new(69.0, 599.0, 90.0, 611.0)),
        ]);
        assert_eq!(
            mapped_paragraph(loaded, &review, &geometry, 0, 1),
            Some(TextSelection::range(0, 0, 1, 2))
        );
        loaded.text_chars[1] = None;
        assert!(mapped_paragraph(loaded, &review, &geometry, 0, 1).is_none());
        loaded.text_chars[1] = Some(chars("def"));
        review.blocks.push(review.blocks[0].clone());
        review.block_source_lines.push(LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![line(0)],
        });
        assert!(mapped_paragraph(loaded, &review, &geometry, 0, 1).is_none());
    }

    #[test]
    fn triple_click_selects_the_paragraph_in_the_original_view() {
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        let rows = [
            (
                72.0,
                700.0,
                "First paragraph is long enough to fill the line",
            ),
            (60.0, 686.0, "and ends here."),
            (72.0, 672.0, "The next paragraph also has a full first line"),
            (60.0, 658.0, "and finishes here."),
        ];
        let mut text = Vec::new();
        for (x, bottom, line) in rows {
            if !text.is_empty() {
                text.extend(chars("\r\n"));
            }
            text.extend(line.chars().enumerate().map(|(index, ch)| PageTextChar {
                ch,
                rect: Some(PdfRect::new(
                    x + index as f32 * 5.0,
                    bottom,
                    x + (index + 1) as f32 * 5.0,
                    bottom + 10.0,
                )),
                font_size: Some(10.0),
                bold: false,
                italic: false,
            }));
        }
        app.document.as_mut().unwrap().text_chars[0] = Some(text);
        app.settings.sidebar_in_pdf = false;
        app.zoom = 0.7;
        app.target_zoom = 0.7;
        app.scroll_target_page = None;
        let texture = ctx.load_texture(
            "triple-click-test",
            egui::ColorImage::filled([4, 4], Color32::WHITE),
            TextureOptions::LINEAR,
        );
        let render_scale = app.page_render_scale(&ctx, 612.0, 792.0);
        app.page_textures.insert(
            0,
            PageTexture {
                _zoom: app.zoom,
                render_scale,
                last_used: 1,
                texture,
                fade_started: None,
            },
        );
        let bounds = Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 900.0));
        let draw = |app: &mut PdfEditorApp, time, events| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(bounds),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ctx| app.draw_document(ctx),
            );
        };
        draw(&mut app, 0.0, vec![]);
        let point = app
            .pdf_page_placements
            .iter()
            .find(|(page, _)| *page == 0)
            .unwrap()
            .1
            .pdf_to_screen((84.0, 705.0));
        draw(&mut app, 0.02, vec![egui::Event::PointerMoved(point)]);
        for (time, pressed) in [
            (0.10, true),
            (0.12, false),
            (0.20, true),
            (0.22, false),
            (0.30, true),
            (0.32, false),
        ] {
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
        assert_eq!(
            app.selected_text().as_deref(),
            Some("First paragraph is long enough to fill the line\r\nand ends here.")
        );
    }
}
