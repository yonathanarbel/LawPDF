//! Original-page footnote previews where the PDF and Review mapping agree.
use super::*;

fn digit(ch: char) -> Option<char> {
    match ch {
        '0'..='9' => Some(ch),
        '⁰' => Some('0'),
        '¹' => Some('1'),
        '²' => Some('2'),
        '³' => Some('3'),
        '⁴' => Some('4'),
        '⁵' => Some('5'),
        '⁶' => Some('6'),
        '⁷' => Some('7'),
        '⁸' => Some('8'),
        '⁹' => Some('9'),
        _ => None,
    }
}

fn marker_in_link(chars: &[PageTextChar], rect: PdfRect) -> Option<u16> {
    let mut digits = String::new();
    for ch in chars {
        let Some(bounds) = ch.rect else {
            continue;
        };
        let center = (
            (bounds.left + bounds.right) * 0.5,
            (bounds.bottom + bounds.top) * 0.5,
        );
        if !point_in_pdf_rect(center, rect) || ch.ch.is_whitespace() {
            continue;
        }
        digits.push(digit(ch.ch)?);
        if digits.len() > 3 {
            return None;
        }
    }
    digits.parse::<u16>().ok().filter(|number| *number > 0)
}

fn preview_note(review: &LiquidDocument, block_index: usize, number: u16) -> Option<String> {
    let block = review.blocks.get(block_index)?;
    if !is_review_note_display_block(block) {
        return None;
    }
    let mut matches = split_fused_review_notes(&block.text)
        .into_iter()
        .filter_map(|(marker, body)| (marker.parse::<u16>().ok() == Some(number)).then_some(body));
    let body = matches.next()?;
    if matches.next().is_some() || body.trim().is_empty() {
        return None;
    }
    let body = body.trim();
    let mut preview = body.chars().take(800).collect::<String>();
    if body.chars().count() > 800 {
        preview.push('…');
    }
    Some(preview)
}

fn resolve_footnote(
    review: &LiquidDocument,
    chars: &[PageTextChar],
    page: usize,
    link: &PageLink,
) -> Option<(u16, String)> {
    let target = link.destination_page?;
    let number = marker_in_link(chars, link.rect)?;
    let mut matches = review
        .footnote_links
        .iter()
        .filter(|candidate| {
            candidate.marker == number
                && candidate.body_page_index == Some(page)
                && candidate.note_page_index == Some(target)
        })
        .map(|candidate| candidate.note_block_index)
        .collect::<Vec<_>>();
    matches.sort_unstable();
    matches.dedup();
    if matches.len() != 1 {
        return None;
    }
    Some((number, preview_note(review, matches[0], number)?))
}

#[derive(Clone, Copy, Debug)]
struct MarkerGlyphs {
    first: usize,
    end: usize,
    number: u16,
    rect: PdfRect,
    unicode_superscript: bool,
}

/// The PDF's character order must contain a contiguous, visually adjacent
/// group of at most three digits. We do not join across whitespace or lines.
fn marker_glyphs(chars: &[PageTextChar], index: usize) -> Option<MarkerGlyphs> {
    let adjacent = |a: &PageTextChar, b: &PageTextChar| {
        let (Some(a), Some(b)) = (a.rect, b.rect) else {
            return false;
        };
        let gap = b.left - a.right;
        gap >= -1.0
            && gap <= a.height().max(b.height()).max(1.0)
            && a.bottom < b.top
            && b.bottom < a.top
    };
    digit(chars.get(index)?.ch)?;
    let mut first = index;
    while first > 0 && digit(chars[first - 1].ch).is_some() {
        if index - first >= 3 {
            return None;
        }
        if !adjacent(&chars[first - 1], &chars[first]) {
            return None;
        }
        first -= 1;
    }
    let mut end = index + 1;
    while end < chars.len() && digit(chars[end].ch).is_some() {
        if end - first >= 3 {
            return None;
        }
        if !adjacent(&chars[end - 1], &chars[end]) {
            return None;
        }
        end += 1;
    }
    if end - first > 3 {
        return None;
    }
    let mut digits = String::new();
    let mut rect = None::<PdfRect>;
    let mut unicode_superscript = true;
    for ch in &chars[first..end] {
        digits.push(digit(ch.ch)?);
        unicode_superscript &= !ch.ch.is_ascii_digit();
        let bounds = ch.rect?;
        if ![bounds.left, bounds.bottom, bounds.right, bounds.top]
            .into_iter()
            .all(f32::is_finite)
            || bounds.width() <= 0.0
            || bounds.height() <= 0.0
        {
            return None;
        }
        rect = Some(match rect {
            Some(previous) => PdfRect::new(
                previous.left.min(bounds.left),
                previous.bottom.min(bounds.bottom),
                previous.right.max(bounds.right),
                previous.top.max(bounds.top),
            ),
            None => bounds,
        });
    }
    if digits.starts_with('0') {
        return None;
    }
    Some(MarkerGlyphs {
        first,
        end,
        number: digits.parse::<u16>().ok().filter(|n| *n > 0)?,
        rect: rect?,
        unicode_superscript,
    })
}

fn center(rect: PdfRect) -> (f32, f32) {
    (
        (rect.left + rect.right) * 0.5,
        (rect.bottom + rect.top) * 0.5,
    )
}

/// Ordinary digits need physical evidence of a raised, smaller glyph. Unicode
/// superscript digits already carry that evidence in the extracted text.
fn is_raised_marker(chars: &[PageTextChar], marker: MarkerGlyphs, line: PdfRect) -> bool {
    if marker.unicode_superscript {
        return true;
    }
    let nearest = chars
        .iter()
        .enumerate()
        .filter_map(|(index, character)| {
            if (marker.first..marker.end).contains(&index) || !character.ch.is_alphabetic() {
                return None;
            }
            let rect = character.rect?;
            if !point_in_pdf_rect(center(rect), line) || rect.height() <= 0.0 {
                return None;
            }
            let distance = if rect.right < marker.rect.left {
                marker.rect.left - rect.right
            } else if marker.rect.right < rect.left {
                rect.left - marker.rect.right
            } else {
                0.0
            };
            (distance <= rect.height() * 3.0).then_some((distance, rect))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0));
    nearest.is_some_and(|(_, body)| {
        marker.rect.height() <= body.height() * 0.85
            && marker.rect.bottom >= body.bottom + body.height() * 0.10
    })
}

/// Source lines can retain ordinary digits even when Review has independently
/// mapped the same glyph as a callout in its body block.
fn source_marker_count(text: &str, number: u16) -> usize {
    let wanted = number.to_string();
    let mut run = String::new();
    let mut count = 0;
    for ch in text.chars().chain(std::iter::once(' ')) {
        if let Some(digit) = digit(ch) {
            run.push(digit);
        } else {
            if run == wanted {
                count += 1;
            }
            run.clear();
        }
    }
    count
}

fn resolve_unlinked(
    review: &LiquidDocument,
    chars: &[PageTextChar],
    geometry: &HashMap<(usize, usize), PdfRect>,
    page: usize,
    page_count: usize,
    point: (f32, f32),
) -> Option<(u16, String)> {
    let mut under_pointer = chars.iter().enumerate().filter(|(_, character)| {
        digit(character.ch).is_some()
            && character
                .rect
                .is_some_and(|rect| point_in_pdf_rect(point, rect))
    });
    let index = under_pointer.next()?.0;
    if under_pointer.next().is_some() {
        return None;
    }
    let marker = marker_glyphs(chars, index)?;
    let mut links = review
        .footnote_links
        .iter()
        .filter(|link| link.marker == marker.number && link.body_page_index == Some(page));
    let link = links.next()?;
    if links.next().is_some()
        || !link
            .note_page_index
            .is_some_and(|target| target < page_count)
    {
        return None;
    }
    let token = format!(
        "{}{}{}",
        layout_roles::CALLOUT_START,
        marker.number,
        layout_roles::CALLOUT_END
    );
    let block = review.blocks.get(link.body_block_index)?;
    if is_review_note_display_block(block) || block.text.match_indices(&token).count() != 1 {
        return None;
    }
    let mut sources = review
        .block_source_lines
        .iter()
        .filter(|source| source.block_index == link.body_block_index);
    let source = sources.next()?;
    if sources.next().is_some() {
        return None;
    }
    let mut matching_lines = source.lines.iter().filter(|line| {
        line.page_index == page
            && source_marker_count(&line.text, marker.number) > 0
            && geometry.get(&(page, line.line_index)).is_some_and(|rect| {
                point_in_pdf_rect(center(marker.rect), expand_pdf_rect(*rect, 1.0))
            })
    });
    let line = matching_lines.next()?;
    if source_marker_count(&line.text, marker.number) != 1 || matching_lines.next().is_some() {
        return None;
    }
    let line_rect = *geometry.get(&(page, line.line_index))?;
    if !point_in_pdf_rect(center(marker.rect), expand_pdf_rect(line_rect, 1.0))
        || !is_raised_marker(chars, marker, line_rect)
    {
        return None;
    }
    // Reject another visually plausible copy of the same marker on this line.
    let mut count = 0;
    let mut candidates = 0;
    for index in 0..chars.len() {
        let Some(other) = marker_glyphs(chars, index) else {
            continue;
        };
        if other.first != index
            || other.number != marker.number
            || !point_in_pdf_rect(center(other.rect), expand_pdf_rect(line_rect, 1.0))
        {
            continue;
        }
        candidates += 1;
        if candidates > 8 {
            return None;
        }
        if is_raised_marker(chars, other, line_rect) {
            count += 1;
            if count > 1 {
                return None;
            }
        }
    }
    if count != 1 {
        return None;
    }
    Some((
        marker.number,
        preview_note(review, link.note_block_index, marker.number)?,
    ))
}

impl PdfEditorApp {
    pub(super) fn original_footnote_peek(
        &mut self,
        page: usize,
        link: &PageLink,
    ) -> Option<(u16, String)> {
        if link.destination_page.is_none() {
            return None;
        }
        let review = match &self.liquid_mode2_state {
            LiquidState::Ready(review) => Arc::clone(review),
            _ => return None,
        };
        let chars = self.ensure_text_chars(page)?;
        resolve_footnote(&review, chars, page, link)
    }

    pub(super) fn original_unlinked_footnote_peek(
        &mut self,
        page: usize,
        point: (f32, f32),
    ) -> Option<(u16, String)> {
        let review = match &self.liquid_mode2_state {
            LiquidState::Ready(review) => Arc::clone(review),
            _ => return None,
        };
        // Do not build the source-line index just because the pointer is over a page.
        let chars = self.document.as_ref()?.text_chars.get(page)?.as_ref()?;
        let index = chars.iter().position(|character| {
            digit(character.ch).is_some()
                && character
                    .rect
                    .is_some_and(|rect| point_in_pdf_rect(point, rect))
        })?;
        let marker = marker_glyphs(chars, index)?;
        if !review
            .footnote_links
            .iter()
            .any(|link| link.body_page_index == Some(page) && link.marker == marker.number)
        {
            return None;
        }
        let geometry = self.review_source_geometry_for_current_document()?;
        let document = self.document.as_ref()?;
        let chars = document.text_chars.get(page)?.as_ref()?;
        resolve_unlinked(&review, chars, &geometry, page, document.page_count, point)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::liquid::{LiquidBlockSourceLines, LiquidFootnoteLink, LiquidSourceLineRef};

    fn character(ch: char, x: f32) -> PageTextChar {
        PageTextChar {
            ch,
            rect: Some(PdfRect::new(x, 10.0, x + 4.0, 18.0)),
            font_size: Some(8.0),
            bold: false,
            italic: false,
        }
    }

    fn review() -> LiquidDocument {
        LiquidDocument {
            title: "Test".into(),
            blocks: vec![
                LiquidBlock {
                    role: LiquidBlockRole::Paragraph,
                    text: "Text".into(),
                    label: None,
                },
                LiquidBlock {
                    role: LiquidBlockRole::Footnote,
                    text: "12. A source in the note.".into(),
                    label: None,
                },
            ],
            article_spans: vec![],
            block_source_lines: vec![],
            footnote_links: vec![LiquidFootnoteLink {
                body_block_index: 0,
                body_marker_ordinal: 0,
                marker: 12,
                note_block_index: 1,
                body_page_index: Some(2),
                note_page_index: Some(3),
            }],
            footnote_link_integrity: None,
            profile: None,
            noise_lines_removed: 0,
            llm_used: false,
            llm_provider: None,
            deep_liquid_used: false,
            deep_liquid_model: None,
            warnings: vec![],
            source_signature: String::new(),
        }
    }

    fn link() -> PageLink {
        PageLink {
            rect: PdfRect::new(9.0, 9.0, 20.0, 19.0),
            url: String::new(),
            destination_page: Some(3),
            destination_fraction: None,
        }
    }

    fn unlinked_review(page: usize) -> LiquidDocument {
        let mut review = review();
        let text = format!(
            "A sentence.{}12{}",
            layout_roles::CALLOUT_START,
            layout_roles::CALLOUT_END
        );
        review.blocks[0].text = text.clone();
        review.footnote_links[0].body_page_index = Some(page);
        review.block_source_lines = vec![LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![LiquidSourceLineRef {
                id: None,
                page_index: page,
                line_index: 7,
                text,
                role: LiquidBlockRole::Paragraph,
                note_markers: vec![],
            }],
        }];
        review
    }

    fn unlinked_characters(unicode: bool) -> Vec<PageTextChar> {
        let mut letter = character('e', 5.0);
        letter.rect = Some(PdfRect::new(5.0, 10.0, 10.0, 22.0));
        letter.font_size = Some(12.0);
        let mut first = character(if unicode { '¹' } else { '1' }, 10.0);
        first.rect = Some(PdfRect::new(10.0, 16.0, 13.0, 22.0));
        let mut second = character(if unicode { '²' } else { '2' }, 13.0);
        second.rect = Some(PdfRect::new(13.0, 16.0, 16.0, 22.0));
        vec![letter, first, second]
    }

    fn source_geometry(page: usize) -> HashMap<(usize, usize), PdfRect> {
        HashMap::from([((page, 7), PdfRect::new(4.0, 9.0, 45.0, 23.0))])
    }

    #[test]
    fn unlinked_preview_requires_unambiguous_source_and_superscript_evidence() {
        let review = unlinked_review(2);
        let geometry = source_geometry(2);
        let chars = unlinked_characters(false);
        assert_eq!(
            resolve_unlinked(&review, &chars, &geometry, 2, 4, (11.0, 19.0)),
            Some((12, "A source in the note.".into()))
        );
        assert_eq!(
            resolve_unlinked(
                &review,
                &unlinked_characters(true),
                &geometry,
                2,
                4,
                (11.0, 19.0)
            ),
            Some((12, "A source in the note.".into()))
        );
        let mut plain_source = review.clone();
        plain_source.block_source_lines[0].lines[0].text = "A sentence. 12".into();
        assert_eq!(
            resolve_unlinked(&plain_source, &chars, &geometry, 2, 4, (11.0, 19.0)),
            Some((12, "A source in the note.".into()))
        );
        plain_source.block_source_lines[0].lines[0].text = "A sentence. 112".into();
        assert!(resolve_unlinked(&plain_source, &chars, &geometry, 2, 4, (11.0, 19.0)).is_none());
        let mut ordinary = chars.clone();
        ordinary[1].rect = Some(PdfRect::new(10.0, 10.0, 13.0, 22.0));
        ordinary[2].rect = Some(PdfRect::new(13.0, 10.0, 16.0, 22.0));
        assert!(resolve_unlinked(&review, &ordinary, &geometry, 2, 4, (11.0, 19.0)).is_none());
        assert!(resolve_unlinked(&review, &chars, &HashMap::new(), 2, 4, (11.0, 19.0)).is_none());
        assert!(resolve_unlinked(&review, &chars, &geometry, 1, 4, (11.0, 19.0)).is_none());
        assert!(resolve_unlinked(&review, &chars, &geometry, 2, 3, (11.0, 19.0)).is_none());
        let mut wrong = review.clone();
        wrong.blocks[0].text = "No matching callout".into();
        assert!(resolve_unlinked(&wrong, &chars, &geometry, 2, 4, (11.0, 19.0)).is_none());
        let mut duplicate_link = review.clone();
        duplicate_link
            .footnote_links
            .push(duplicate_link.footnote_links[0].clone());
        assert!(resolve_unlinked(&duplicate_link, &chars, &geometry, 2, 4, (11.0, 19.0)).is_none());
        duplicate_link.footnote_links[1].note_page_index = None;
        assert!(resolve_unlinked(&duplicate_link, &chars, &geometry, 2, 4, (11.0, 19.0)).is_none());
        let mut duplicate_source = review.clone();
        duplicate_source
            .block_source_lines
            .push(duplicate_source.block_source_lines[0].clone());
        assert!(
            resolve_unlinked(&duplicate_source, &chars, &geometry, 2, 4, (11.0, 19.0)).is_none()
        );
        let mut duplicate_callout = review.clone();
        let token = format!(
            "{}12{}",
            layout_roles::CALLOUT_START,
            layout_roles::CALLOUT_END
        );
        duplicate_callout.block_source_lines[0].lines[0]
            .text
            .push_str(&token);
        assert!(
            resolve_unlinked(&duplicate_callout, &chars, &geometry, 2, 4, (11.0, 19.0)).is_none()
        );
        let mut duplicate_glyphs = chars.clone();
        duplicate_glyphs.push(character(' ', 20.0));
        let mut other = unlinked_characters(true);
        other[1].rect = Some(PdfRect::new(30.0, 16.0, 33.0, 22.0));
        other[2].rect = Some(PdfRect::new(33.0, 16.0, 36.0, 22.0));
        duplicate_glyphs.extend(other.into_iter().skip(1));
        assert!(
            resolve_unlinked(&review, &duplicate_glyphs, &geometry, 2, 4, (11.0, 19.0)).is_none()
        );
        let mut distant_digit = chars.clone();
        distant_digit[2].rect = Some(PdfRect::new(40.0, 16.0, 43.0, 22.0));
        assert!(resolve_unlinked(&review, &distant_digit, &geometry, 2, 4, (11.0, 19.0)).is_none());
    }

    #[test]
    fn unlinked_preview_uses_real_pdfium_character_and_source_geometry() {
        use lopdf::{Document, Object, Stream, dictionary};
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("unlinked-marker.pdf");
        let mut pdf = Document::with_version("1.5");
        let catalog = pdf.new_object_id();
        let pages = pdf.new_object_id();
        let page = pdf.new_object_id();
        let font = pdf.add_object(
            dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" },
        );
        let contents = pdf.add_object(Stream::new(dictionary! {}, b"BT /F1 12 Tf 1 0 0 1 72 700 Tm (A sentence about the law.) Tj /F1 7 Tf 5 Ts (12) Tj ET".to_vec()));
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
        let lines = crate::layout_roles::deep_source_lines_for_pages(
            &[crate::model::PageInfo::new(612.0, 792.0)],
            &[Some(chars.clone())],
        );
        let token = format!(
            "{}12{}",
            layout_roles::CALLOUT_START,
            layout_roles::CALLOUT_END
        );
        let line = lines
            .iter()
            .find(|line| line.text.contains(&token))
            .expect("PDFium did not identify the synthetic raised callout");
        let mut review = unlinked_review(0);
        review.blocks[0].text = line.text.clone();
        // Review can retain plain digits in a source line while the block's
        // independently mapped callout carries explicit marker delimiters.
        review.block_source_lines[0].lines[0].text = line
            .text
            .replace(layout_roles::CALLOUT_START, "")
            .replace(layout_roles::CALLOUT_END, "");
        review.block_source_lines[0].lines[0].line_index = line.line_index;
        review.footnote_links[0].note_page_index = Some(0);
        let geometry = HashMap::from([(
            (0, line.line_index),
            PdfRect::new(line.left, line.bottom, line.right, line.top),
        )]);
        let marker = chars
            .iter()
            .find(|character| character.ch == '1')
            .unwrap()
            .rect
            .unwrap();
        assert_eq!(
            resolve_unlinked(&review, &chars, &geometry, 0, 1, center(marker)),
            Some((12, "A source in the note.".into()))
        );
    }

    #[cfg(feature = "devtools")]
    #[test]
    #[ignore = "requires the native Review models"]
    fn unlinked_preview_uses_a_real_prepared_review_mapping() {
        use lopdf::{Document, Object, Stream, dictionary};
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("synthetic-article.pdf");
        let mut pdf = Document::with_version("1.5");
        let catalog = pdf.new_object_id();
        let pages_id = pdf.new_object_id();
        let page_id = pdf.new_object_id();
        let regular = pdf.add_object(
            dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Times-Roman" },
        );
        let bold = pdf.add_object(
            dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Times-Bold" },
        );
        let mut content = String::from(
            "BT /F2 18 Tf 72 744 Td (A SYNTHETIC STUDY OF CONTRACT INTERPRETATION) Tj ET\nBT /F2 11 Tf 72 690 Td (I. INTRODUCTION) Tj ET\n",
        );
        let lines = [
            "Courts often interpret agreements by examining their text, structure, and purpose.",
            "This synthetic article illustrates a reading interface and a footnote reference.",
            "The choice between ordinary meaning and context shapes the work of interpretation.",
            "Readers also compare related provisions to understand how the agreement operates.",
            "No sentence in this test document describes a real case or a real legal authority.",
            "The purpose of the page is to test geometry, navigation, and source-line matching.",
            "A second paragraph adds ordinary text so that the page resembles an article.",
            "Written contracts can include definitions, conditions, remedies, and other terms.",
            "The reader may select a phrase or consult the note below without leaving the page.",
            "These examples are generated solely for local software testing and contain no data.",
            "The final line gives the classifier enough surrounding prose to recognize a body.",
        ];
        for (index, line) in lines.iter().enumerate() {
            content.push_str(&format!(
                "BT /F1 12 Tf 72 {} Td ({line}) Tj ",
                665 - 16 * index
            ));
            if index == 1 {
                content.push_str("/F1 7 Tf 5 Ts (1) Tj ");
            }
            content.push_str("ET\n");
        }
        content.push_str("72 125 m 225 125 l S\nBT /F1 9 Tf 72 111 Td (1. This generated note is an example for testing a footnote preview.) Tj ET\nBT /F1 9 Tf 72 100 Td (It has no source and makes no claim about an actual court or publication.) Tj ET\n");
        let contents = pdf.add_object(Stream::new(dictionary! {}, content.into_bytes()));
        pdf.objects.insert(
            catalog,
            Object::Dictionary(dictionary! { "Type" => "Catalog", "Pages" => pages_id }),
        );
        pdf.objects.insert(pages_id, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => vec![Object::Reference(page_id)], "Count" => 1 }));
        pdf.objects.insert(page_id, Object::Dictionary(dictionary! { "Type" => "Page", "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => regular, "F2" => bold } }, "Contents" => contents }));
        pdf.trailer.set("Root", catalog);
        pdf.save(&path).unwrap();
        let engine = PdfEngine::new().unwrap();
        let document = engine.load_document(&path).unwrap();
        let page_count = document.page_count;
        let chars = (0..page_count)
            .map(|page| Some(engine.load_page_text_chars(&path, page).unwrap()))
            .collect::<Vec<_>>();
        let pages = crate::layout_roles::source_pages_from_text_chars(&document.pages, &chars)
            .into_iter()
            .enumerate()
            .map(|(page, text)| text.unwrap_or_else(|| engine.load_page_text(&path, page).unwrap()))
            .collect();
        let deep = crate::layout_roles::deep_source_lines_for_pages(&document.pages, &chars);
        let geometry = deep
            .iter()
            .map(|line| {
                (
                    (line.page_index, line.line_index),
                    PdfRect::new(line.left, line.bottom, line.right, line.top),
                )
            })
            .collect::<HashMap<_, _>>();
        let review =
            crate::liquid2::prepare_liquid_mode2_document(crate::liquid2::LiquidMode2Request {
                document_epoch: 0,
                path,
                title: document.title,
                pages,
                deep_source_lines: deep,
                use_pymupdf_blocks: false,
                use_pp_footnote_regions: false,
                external_emissions_path: None,
                runtime_choice: crate::liquid2::Lm2RuntimeChoice::Automatic,
                preview_only: false,
                skip_progressive_preview: false,
            })
            .unwrap();
        let previews = chars
            .iter()
            .enumerate()
            .flat_map(|(page, chars)| {
                let review = &review;
                let geometry = &geometry;
                let chars = chars.as_ref().unwrap();
                chars.iter().filter_map(move |character| {
                    let rect = character.rect?;
                    digit(character.ch)?;
                    resolve_unlinked(review, chars, geometry, page, page_count, center(rect))
                })
            })
            .collect::<Vec<_>>();
        assert!(
            previews
                .iter()
                .any(|(number, body)| *number == 1 && body.contains("generated note")),
            "prepared Review did not expose the synthetic note"
        );
    }

    #[test]
    fn hovering_unlinked_marker_shows_preview_without_changing_navigation() {
        fn contains_text(shape: &egui::Shape, expected: &str) -> bool {
            match shape {
                egui::Shape::Text(text) => text.galley.text().contains(expected),
                egui::Shape::Vec(shapes) => {
                    shapes.iter().any(|shape| contains_text(shape, expected))
                }
                _ => false,
            }
        }
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        let document = app.document.as_mut().unwrap();
        document.page_count = 4;
        document.pages = vec![crate::model::PageInfo::new(612.0, 792.0); 4];
        document.links = vec![vec![], vec![], vec![], vec![]];
        document.text_chars = vec![Some(unlinked_characters(false)), None, None, None];
        document.native_text = vec![String::new(); 4];
        document.native_text_loaded = vec![true; 4];
        app.liquid_mode2_state = LiquidState::Ready(Arc::new(unlinked_review(0)));
        app.review_source_geometry = Some(ReviewSourceGeometry {
            document_epoch: app.document_epoch,
            loaded_pages: 1,
            rects: Arc::new(source_geometry(0)),
        });
        app.settings.sidebar_in_pdf = false;
        app.zoom = 0.7;
        app.target_zoom = 0.7;
        app.scroll_target_page = None;
        let texture = ctx.load_texture(
            "unlinked-footnote-test",
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
        let draw = |app: &mut PdfEditorApp, time, events| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 900.0))),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ctx| app.draw_document(ctx),
            )
        };
        let _ = draw(&mut app, 0.0, vec![]);
        let point = app
            .pdf_page_placements
            .iter()
            .find(|(page, _)| *page == 0)
            .unwrap()
            .1
            .pdf_to_screen((11.0, 19.0));
        let _ = draw(&mut app, 0.1, vec![egui::Event::PointerMoved(point)]);
        let mut seen = false;
        for time in [0.3, 0.6, 1.0, 1.4, 1.8] {
            let output = draw(&mut app, time, vec![]);
            seen |= output
                .shapes
                .iter()
                .any(|shape| contains_text(&shape.shape, "A source in the note."));
        }
        assert!(seen, "unlinked marker preview did not appear");
        assert_eq!(app.page_index, 0);
        let _ = draw(
            &mut app,
            1.9,
            vec![egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        let _ = draw(
            &mut app,
            2.0,
            vec![egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert_eq!(app.page_index, 0);
    }

    #[test]
    fn preview_requires_the_marker_destination_and_review_mapping_to_agree() {
        let chars = vec![character('¹', 10.0), character('²', 15.0)];
        assert_eq!(
            resolve_footnote(&review(), &chars, 2, &link()),
            Some((12, "A source in the note.".into()))
        );
        assert!(resolve_footnote(&review(), &chars, 1, &link()).is_none());
        let mut wrong = link();
        wrong.destination_page = Some(4);
        assert!(resolve_footnote(&review(), &chars, 2, &wrong).is_none());
        assert!(resolve_footnote(&review(), &[character('x', 10.0)], 2, &link()).is_none());
        let mut ambiguous = review();
        ambiguous.blocks.push(ambiguous.blocks[1].clone());
        let mut duplicate = ambiguous.footnote_links[0].clone();
        duplicate.note_block_index = 2;
        ambiguous.footnote_links.push(duplicate);
        assert!(resolve_footnote(&ambiguous, &chars, 2, &link()).is_none());
    }

    #[test]
    fn preview_waits_for_review_and_never_treats_external_links_as_notes() {
        let (mut app, _ctx, _worker) = annotation_saving::tests::fixture();
        app.document.as_mut().unwrap().text_chars[0] =
            Some(vec![character('¹', 10.0), character('²', 15.0)]);
        assert!(app.original_footnote_peek(0, &link()).is_none());
        let mut ready = review();
        ready.footnote_links[0].body_page_index = Some(0);
        app.liquid_mode2_state = LiquidState::Ready(Arc::new(ready));
        assert_eq!(
            app.original_footnote_peek(0, &link()),
            Some((12, "A source in the note.".into()))
        );
        let mut external = link();
        external.destination_page = None;
        external.url = "https://example.org".into();
        assert!(app.original_footnote_peek(0, &external).is_none());
    }

    #[test]
    fn hovering_a_matched_marker_shows_the_note_and_keeps_the_link_clickable() {
        fn contains_text(shape: &egui::Shape, expected: &str) -> bool {
            match shape {
                egui::Shape::Text(text) => text.galley.text().contains(expected),
                egui::Shape::Vec(shapes) => {
                    shapes.iter().any(|shape| contains_text(shape, expected))
                }
                _ => false,
            }
        }
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        let document = app.document.as_mut().unwrap();
        document.page_count = 4;
        document.pages = vec![crate::model::PageInfo::new(612.0, 792.0); 4];
        document.links = vec![vec![link()], vec![], vec![], vec![]];
        document.text_chars = vec![
            Some(vec![character('¹', 10.0), character('²', 15.0)]),
            None,
            None,
            None,
        ];
        document.native_text = vec![String::new(); 4];
        document.native_text_loaded = vec![true; 4];
        let mut ready = review();
        ready.footnote_links[0].body_page_index = Some(0);
        app.liquid_mode2_state = LiquidState::Ready(Arc::new(ready));
        app.settings.sidebar_in_pdf = false;
        app.zoom = 0.7;
        app.target_zoom = 0.7;
        app.scroll_target_page = None;
        let texture = ctx.load_texture(
            "footnote-peek-test",
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
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(bounds),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ctx| app.draw_document(ctx),
            )
        };
        let _ = draw(&mut app, 0.0, vec![]);
        let point = app
            .pdf_page_placements
            .iter()
            .find(|(page, _)| *page == 0)
            .unwrap()
            .1
            .pdf_to_screen((12.0, 14.0));
        let _ = draw(&mut app, 0.1, vec![egui::Event::PointerMoved(point)]);
        let mut seen = false;
        for time in [0.3, 0.6, 1.0, 1.4, 1.8] {
            let output = draw(&mut app, time, vec![]);
            seen |= output
                .shapes
                .iter()
                .any(|shape| contains_text(&shape.shape, "A source in the note."));
        }
        assert!(seen, "note preview did not appear");
        assert_eq!(app.page_index, 0);
        let _ = draw(
            &mut app,
            1.9,
            vec![egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        let _ = draw(
            &mut app,
            2.0,
            vec![egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert_eq!(app.page_index, 3);
    }
}
