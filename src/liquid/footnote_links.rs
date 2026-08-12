use std::collections::{BTreeMap, BTreeSet};

use super::{
    LiquidBlock, LiquidBlockRole, LiquidBlockSourceLines, LiquidDocument, LiquidFootnoteLink,
    LiquidFootnoteLinkIntegrity,
};

const CALLOUT_START: char = '\u{E000}';
const CALLOUT_END: char = '\u{E001}';
const MAX_NOTE_MARKER: u16 = 999;

#[derive(Debug, Clone, Copy)]
struct Reference {
    block_index: usize,
    ordinal: usize,
    marker: u16,
    page_index: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct NoteHead {
    block_index: usize,
    marker: u16,
    page_index: Option<usize>,
}

pub fn attach_footnote_links(document: &mut LiquidDocument) {
    restore_source_backed_plain_callouts(&mut document.blocks, &document.block_source_lines);
    let (links, integrity) = resolve_footnote_links_in_articles(
        &document.blocks,
        &document.block_source_lines,
        &document.article_spans,
    );
    document.footnote_links = links;
    document.footnote_link_integrity = (integrity.detectable_markers > 0).then_some(integrity);
}

/// A late paragraph merge can occasionally preserve an inline superscript as
/// plain digits even though its source row still carries the exact callout
/// sentinels. Restore only a uniquely located `marker + following word` pair
/// proved by that same source row; ordinary prose numbers are untouched.
fn restore_source_backed_plain_callouts(
    blocks: &mut [LiquidBlock],
    block_source_lines: &[LiquidBlockSourceLines],
) {
    for source in block_source_lines {
        let Some(block) = blocks.get_mut(source.block_index) else {
            continue;
        };
        if !body_role(block.role) {
            continue;
        }
        let mut existing = callout_markers(&block.text).into_iter().fold(
            BTreeMap::<u16, usize>::new(),
            |mut counts, marker| {
                *counts.entry(marker).or_default() += 1;
                counts
            },
        );
        for line in &source.lines {
            for (marker, anchor) in source_callout_anchors(&line.text) {
                if existing.get_mut(&marker).is_some_and(|count| {
                    if *count > 0 {
                        *count -= 1;
                        true
                    } else {
                        false
                    }
                }) {
                    continue;
                }
                restore_plain_callout_before_anchor(&mut block.text, marker, &anchor);
            }
        }
    }
}

fn source_callout_anchors(text: &str) -> Vec<(u16, String)> {
    let mut anchors = Vec::new();
    let mut cursor = 0usize;
    while let Some(relative) = text[cursor..].find(CALLOUT_START) {
        let start = cursor + relative;
        let tail_start = start + CALLOUT_START.len_utf8();
        let Some(end_relative) = text[tail_start..].find(CALLOUT_END) else {
            break;
        };
        let end = tail_start + end_relative;
        let after = end + CALLOUT_END.len_utf8();
        if let Ok(marker) = text[tail_start..end].parse::<u16>()
            && (1..=MAX_NOTE_MARKER).contains(&marker)
        {
            let anchor = text[after..]
                .trim_start()
                .chars()
                .skip_while(|ch| !ch.is_alphabetic())
                .take_while(|ch| ch.is_alphabetic())
                .collect::<String>();
            if anchor.len() >= 2 {
                anchors.push((marker, anchor));
            }
        }
        cursor = after;
    }
    anchors
}

fn restore_plain_callout_before_anchor(text: &mut String, marker: u16, anchor: &str) -> bool {
    let digits = marker.to_string();
    let candidates = text
        .match_indices(&digits)
        .filter_map(|(start, _)| {
            let end = start + digits.len();
            let previous = text[..start].chars().next_back();
            let boundary_before = previous.is_none_or(|ch| {
                ch.is_whitespace() || ch.is_ascii_punctuation() || matches!(ch, '”' | '’')
            });
            let boundary_after = text[end..].chars().next().is_some_and(char::is_whitespace);
            let remainder = text[end..].trim_start();
            (boundary_before && boundary_after && remainder.starts_with(anchor))
                .then_some((start, end))
        })
        .collect::<Vec<_>>();
    let [(start, end)] = candidates.as_slice() else {
        return false;
    };
    text.replace_range(
        *start..*end,
        &format!("{CALLOUT_START}{marker}{CALLOUT_END}"),
    );
    true
}

pub fn resolve_footnote_links(
    blocks: &[LiquidBlock],
    block_source_lines: &[LiquidBlockSourceLines],
) -> (Vec<LiquidFootnoteLink>, LiquidFootnoteLinkIntegrity) {
    resolve_footnote_links_in_articles(blocks, block_source_lines, &[])
}

pub fn resolve_footnote_links_in_articles(
    blocks: &[LiquidBlock],
    block_source_lines: &[LiquidBlockSourceLines],
    article_spans: &[super::ArticleSpan],
) -> (Vec<LiquidFootnoteLink>, LiquidFootnoteLinkIntegrity) {
    let pages = block_pages(block_source_lines);
    let reference_pages = block_reference_pages(block_source_lines);
    let source_note_heads = block_note_heads(block_source_lines);
    let mut references = Vec::new();
    let mut notes = Vec::new();
    for (block_index, block) in blocks.iter().enumerate() {
        let page_index = pages.get(&block_index).copied();
        if body_role(block.role) {
            for (ordinal, marker) in callout_markers(&block.text).into_iter().enumerate() {
                let marker_page = reference_pages
                    .get(&block_index)
                    .and_then(|markers| markers.get(ordinal))
                    .filter(|(source_marker, _)| *source_marker == marker)
                    .map(|(_, page)| *page)
                    .or(page_index);
                references.push(Reference {
                    block_index,
                    ordinal,
                    marker,
                    page_index: marker_page,
                });
            }
        }
        if note_role(block.role) {
            let block_heads = source_note_heads.get(&block_index);
            if let Some(block_heads) = block_heads {
                notes.extend(block_heads.iter().copied());
            } else if let Some(marker) = leading_note_marker(&block.text) {
                notes.push(NoteHead {
                    block_index,
                    marker,
                    page_index,
                });
            }
        }
    }
    notes.sort_unstable();
    notes.dedup();
    discard_redundant_marker_only_note_heads(&mut notes, blocks);

    let mut links = Vec::new();
    let mut unmatched = 0usize;
    let mut ambiguous = 0usize;
    for reference in &references {
        let reference_article =
            block_article_index(reference.block_index, block_source_lines, article_spans);
        let same_marker = notes
            .iter()
            .filter(|note| {
                note.marker == reference.marker
                    && (article_spans.is_empty()
                        || block_article_index(
                            note.block_index,
                            block_source_lines,
                            article_spans,
                        ) == reference_article)
            })
            .copied()
            .collect::<Vec<_>>();
        let candidates = conservative_candidates(reference, &same_marker);
        if candidates.len() == 1 {
            let note = candidates[0];
            links.push(LiquidFootnoteLink {
                body_block_index: reference.block_index,
                body_marker_ordinal: reference.ordinal,
                marker: reference.marker,
                note_block_index: note.block_index,
                body_page_index: reference.page_index,
                note_page_index: note.page_index,
            });
        } else if candidates.is_empty() {
            unmatched += 1;
        } else {
            ambiguous += 1;
        }
    }
    let detectable = references.len();
    let integrity = LiquidFootnoteLinkIntegrity {
        detectable_markers: detectable,
        landed: links.len(),
        unmatched,
        ambiguous,
        note_heads: notes.len(),
        landing_rate: rate(links.len(), detectable),
        ambiguous_rate: rate(ambiguous, detectable),
    };
    (links, integrity)
}

/// A flattened PDF can emit an inline callout twice: once as the sentinel in
/// body prose and once as a standalone number-only Marginalia row.  When the
/// same page also contains the substantive definition for that marker, the
/// number-only row is not a second footnote head and must not make the link
/// ambiguous.  Preserve number-only heads when no fuller same-page definition
/// exists because some PDFs genuinely split the marker from its continuation.
fn discard_redundant_marker_only_note_heads(notes: &mut Vec<NoteHead>, blocks: &[LiquidBlock]) {
    let substantive = notes
        .iter()
        .filter(|note| {
            blocks
                .get(note.block_index)
                .is_some_and(|block| !marker_only_note_head(&block.text, note.marker))
        })
        .map(|note| (note.marker, note.page_index))
        .collect::<BTreeSet<_>>();
    notes.retain(|note| {
        !blocks
            .get(note.block_index)
            .is_some_and(|block| marker_only_note_head(&block.text, note.marker))
            || !substantive.contains(&(note.marker, note.page_index))
    });
}

fn marker_only_note_head(text: &str, marker: u16) -> bool {
    let trimmed = text.trim();
    trimmed.parse::<u16>() == Ok(marker)
        || trimmed
            .strip_suffix('.')
            .is_some_and(|digits| digits.trim().parse::<u16>() == Ok(marker))
}

fn block_note_heads(
    block_source_lines: &[LiquidBlockSourceLines],
) -> BTreeMap<usize, Vec<NoteHead>> {
    block_source_lines
        .iter()
        .filter_map(|source| {
            let heads = source
                .lines
                .iter()
                .flat_map(|line| {
                    line.note_markers.iter().copied().map(|marker| NoteHead {
                        block_index: source.block_index,
                        marker,
                        page_index: Some(line.page_index),
                    })
                })
                .collect::<Vec<_>>();
            source
                .lines
                .iter()
                .any(|line| !line.text.trim().is_empty())
                .then_some((source.block_index, heads))
        })
        .collect()
}

fn block_reference_pages(
    block_source_lines: &[LiquidBlockSourceLines],
) -> BTreeMap<usize, Vec<(u16, usize)>> {
    block_source_lines
        .iter()
        .filter_map(|source| {
            let markers = source
                .lines
                .iter()
                .flat_map(|line| {
                    callout_markers(&line.text)
                        .into_iter()
                        .map(|marker| (marker, line.page_index))
                })
                .collect::<Vec<_>>();
            (!markers.is_empty()).then_some((source.block_index, markers))
        })
        .collect()
}

fn conservative_candidates<'a>(reference: &Reference, notes: &'a [NoteHead]) -> Vec<&'a NoteHead> {
    if notes.len() <= 1 {
        return notes.iter().collect();
    }
    let Some(body_page) = reference.page_index else {
        return Vec::new();
    };
    for page in [body_page, body_page.saturating_add(1)] {
        let local = notes
            .iter()
            .filter(|note| note.page_index == Some(page))
            .collect::<Vec<_>>();
        if !local.is_empty() {
            return local;
        }
    }
    Vec::new()
}

fn block_pages(block_source_lines: &[LiquidBlockSourceLines]) -> BTreeMap<usize, usize> {
    block_source_lines
        .iter()
        .filter_map(|source| {
            source
                .lines
                .iter()
                .map(|line| line.page_index)
                .min()
                .map(|page| (source.block_index, page))
        })
        .collect()
}

fn callout_markers(text: &str) -> Vec<u16> {
    let mut markers = Vec::new();
    let mut inside = false;
    let mut digits = String::new();
    for ch in text.chars() {
        if ch == CALLOUT_START {
            inside = true;
            digits.clear();
        } else if ch == CALLOUT_END {
            if inside
                && let Ok(marker) = digits.parse::<u16>()
                && (1..=MAX_NOTE_MARKER).contains(&marker)
            {
                markers.push(marker);
            }
            inside = false;
            digits.clear();
        } else if inside && ch.is_ascii_digit() && digits.len() < 3 {
            digits.push(ch);
        } else if inside && !ch.is_whitespace() {
            inside = false;
            digits.clear();
        }
    }
    markers
}

fn leading_note_marker(text: &str) -> Option<u16> {
    let trimmed = text.trim_start();
    let digits = trimmed
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .take(3)
        .collect::<String>();
    if digits.is_empty() {
        return None;
    }
    let marker = digits.parse::<u16>().ok()?;
    (1..=MAX_NOTE_MARKER).contains(&marker).then_some(marker)
}

fn body_role(role: LiquidBlockRole) -> bool {
    matches!(
        role,
        LiquidBlockRole::Paragraph
            | LiquidBlockRole::Lead
            | LiquidBlockRole::Heading
            | LiquidBlockRole::Subheading
            | LiquidBlockRole::Quote
            | LiquidBlockRole::ListItem
            | LiquidBlockRole::Table
            | LiquidBlockRole::Caption
    )
}

fn note_role(role: LiquidBlockRole) -> bool {
    matches!(
        role,
        LiquidBlockRole::Footnote | LiquidBlockRole::Marginalia
    )
}

fn block_article_index(
    block_index: usize,
    block_source_lines: &[LiquidBlockSourceLines],
    article_spans: &[super::ArticleSpan],
) -> Option<usize> {
    if article_spans.is_empty() {
        return Some(0);
    }
    let source = block_source_lines
        .iter()
        .find(|source| source.block_index == block_index)?;
    let coordinate = source
        .lines
        .iter()
        .map(|line| (line.page_index, line.line_index))
        .min()?;
    article_spans.iter().find_map(|span| {
        (coordinate >= (span.start_page_index, span.start_line_index)
            && coordinate < (span.end_page_index, span.end_line_index))
        .then_some(span.article_index)
    })
}

fn rate(numerator: usize, denominator: usize) -> f32 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f32 / denominator as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::liquid::LiquidSourceLineRef;

    fn block(role: LiquidBlockRole, text: &str) -> LiquidBlock {
        LiquidBlock {
            role,
            text: text.to_owned(),
            label: None,
        }
    }

    fn source(block_index: usize, page_index: usize) -> LiquidBlockSourceLines {
        source_at(block_index, page_index, 0)
    }

    fn source_at(block_index: usize, page_index: usize, line_index: usize) -> LiquidBlockSourceLines {
        LiquidBlockSourceLines {
            block_index,
            lines: vec![LiquidSourceLineRef {
                id: None,
                page_index,
                line_index,
                text: String::new(),
                role: LiquidBlockRole::Paragraph,
                note_markers: Vec::new(),
            }],
        }
    }

    #[test]
    fn resolves_exact_inline_markers_to_numbered_notes() {
        let blocks = vec![
            block(LiquidBlockRole::Paragraph, "Claim.\u{E000}12\u{E001}"),
            block(LiquidBlockRole::Marginalia, "12 Authority."),
        ];
        let (links, integrity) = resolve_footnote_links(&blocks, &[source(0, 3), source(1, 3)]);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].marker, 12);
        assert_eq!(links[0].note_block_index, 1);
        assert_eq!(integrity.landing_rate, 1.0);
    }

    #[test]
    fn resolves_note_markers_above_five_hundred() {
        let blocks = vec![
            block(LiquidBlockRole::Paragraph, "Claim.\u{E000}532\u{E001}"),
            block(LiquidBlockRole::Marginalia, "532 Authority."),
        ];
        let (links, integrity) = resolve_footnote_links(&blocks, &[source(0, 78), source(1, 78)]);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].marker, 532);
        assert_eq!(integrity.note_heads, 1);
        assert_eq!(integrity.landing_rate, 1.0);
    }

    #[test]
    fn repeated_full_issue_numbers_use_same_page_note() {
        let blocks = vec![
            block(LiquidBlockRole::Marginalia, "1 Old article note."),
            block(LiquidBlockRole::Paragraph, "New claim.\u{E000}1\u{E001}"),
            block(LiquidBlockRole::Marginalia, "1 New article note."),
        ];
        let (links, integrity) =
            resolve_footnote_links(&blocks, &[source(0, 1), source(1, 20), source(2, 20)]);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].note_block_index, 2);
        assert_eq!(integrity.ambiguous, 0);
    }

    #[test]
    fn duplicate_same_page_note_heads_are_ambiguous() {
        let blocks = vec![
            block(LiquidBlockRole::Paragraph, "Claim.\u{E000}2\u{E001}"),
            block(LiquidBlockRole::Marginalia, "2 First."),
            block(LiquidBlockRole::Marginalia, "2 Second."),
        ];
        let (links, integrity) =
            resolve_footnote_links(&blocks, &[source(0, 4), source(1, 4), source(2, 4)]);
        assert!(links.is_empty());
        assert_eq!(integrity.ambiguous, 1);
    }

    #[test]
    fn redundant_marker_only_head_defers_to_full_same_page_definition() {
        let blocks = vec![
            block(LiquidBlockRole::Paragraph, "Claim.\u{E000}21\u{E001}"),
            block(LiquidBlockRole::Marginalia, "21"),
            block(LiquidBlockRole::Marginalia, "21 Full authority."),
        ];
        let (links, integrity) =
            resolve_footnote_links(&blocks, &[source(0, 4), source(1, 4), source(2, 4)]);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].note_block_index, 2);
        assert_eq!(integrity.note_heads, 1);
        assert_eq!(integrity.ambiguous, 0);
        assert_eq!(integrity.landing_rate, 1.0);
    }

    #[test]
    fn marker_only_head_is_preserved_without_a_full_definition() {
        let blocks = vec![
            block(LiquidBlockRole::Paragraph, "Claim.\u{E000}21\u{E001}"),
            block(LiquidBlockRole::Marginalia, "21"),
        ];
        let (links, integrity) = resolve_footnote_links(&blocks, &[source(0, 4), source(1, 4)]);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].note_block_index, 1);
        assert_eq!(integrity.note_heads, 1);
        assert_eq!(integrity.ambiguous, 0);
    }

    #[test]
    fn merged_note_block_preserves_each_source_marker() {
        let blocks = vec![
            block(
                LiquidBlockRole::Paragraph,
                "First.\u{E000}1\u{E001} Second.\u{E000}2\u{E001}",
            ),
            block(LiquidBlockRole::Marginalia, "1 First note. 2 Second note."),
        ];
        let note_sources = LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![
                LiquidSourceLineRef {
                    id: Some("p0:l1".to_owned()),
                    page_index: 0,
                    line_index: 1,
                    text: "1 First note.".to_owned(),
                    role: LiquidBlockRole::Marginalia,
                    note_markers: vec![1],
                },
                LiquidSourceLineRef {
                    id: Some("p0:l2".to_owned()),
                    page_index: 0,
                    line_index: 2,
                    text: "2 Second note.".to_owned(),
                    role: LiquidBlockRole::Marginalia,
                    note_markers: vec![2],
                },
            ],
        };
        let (links, integrity) = resolve_footnote_links(&blocks, &[source(0, 0), note_sources]);
        assert_eq!(links.len(), 2);
        assert_eq!(integrity.landing_rate, 1.0);
        assert_eq!(links[0].note_block_index, 1);
        assert_eq!(links[1].note_block_index, 1);
    }

    #[test]
    fn malformed_callout_is_not_guessed() {
        assert!(callout_markers("Claim.\u{E000}12a\u{E001}").is_empty());
    }

    #[test]
    fn restores_plain_callout_only_from_matching_source_provenance() {
        let mut blocks = vec![block(
            LiquidBlockRole::Paragraph,
            "Accountability. 24 Cross-sovereign policing.\u{E000}25\u{E001}",
        )];
        let sources = vec![LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![LiquidSourceLineRef {
                id: Some("p8:l6".to_owned()),
                page_index: 8,
                line_index: 6,
                text: "\u{E000}24\u{E001} Cross\u{0002}".to_owned(),
                role: LiquidBlockRole::Paragraph,
                note_markers: vec![24],
            }],
        }];

        restore_source_backed_plain_callouts(&mut blocks, &sources);

        assert_eq!(
            blocks[0].text,
            "Accountability. \u{E000}24\u{E001} Cross-sovereign policing.\u{E000}25\u{E001}"
        );
    }

    #[test]
    fn does_not_restore_plain_number_without_matching_source_provenance() {
        let mut blocks = vec![block(
            LiquidBlockRole::Paragraph,
            "Section 24 Cross references remain ordinary prose.",
        )];
        let sources = vec![source(0, 8)];

        restore_source_backed_plain_callouts(&mut blocks, &sources);

        assert_eq!(
            blocks[0].text,
            "Section 24 Cross references remain ordinary prose."
        );
    }

    #[test]
    fn restored_source_backed_callout_links_to_its_note() {
        let mut document = LiquidDocument {
            title: "Test Article".to_owned(),
            blocks: vec![
                block(
                    LiquidBlockRole::Paragraph,
                    "Accountability. 24 Cross-sovereign policing.",
                ),
                block(LiquidBlockRole::Marginalia, "24 Authority."),
            ],
            block_source_lines: vec![
                LiquidBlockSourceLines {
                    block_index: 0,
                    lines: vec![LiquidSourceLineRef {
                        id: Some("p8:l6".to_owned()),
                        page_index: 8,
                        line_index: 6,
                        text: "\u{E000}24\u{E001} Cross\u{0002}".to_owned(),
                        role: LiquidBlockRole::Paragraph,
                        note_markers: vec![24],
                    }],
                },
                LiquidBlockSourceLines {
                    block_index: 1,
                    lines: vec![LiquidSourceLineRef {
                        id: Some("p8:l54".to_owned()),
                        page_index: 8,
                        line_index: 54,
                        text: "\u{E000}24\u{E001}. Authority.".to_owned(),
                        role: LiquidBlockRole::Marginalia,
                        note_markers: vec![24],
                    }],
                },
            ],
            article_spans: Vec::new(),
            footnote_links: Vec::new(),
            footnote_link_integrity: None,
            profile: None,
            noise_lines_removed: 0,
            llm_used: false,
            llm_provider: None,
            deep_liquid_used: false,
            deep_liquid_model: None,
            warnings: Vec::new(),
            source_signature: "test".to_owned(),
        };

        attach_footnote_links(&mut document);

        assert_eq!(document.footnote_links.len(), 1);
        assert_eq!(document.footnote_links[0].marker, 24);
        assert_eq!(document.footnote_links[0].note_block_index, 1);
    }

    #[test]
    fn source_backed_markerless_reporter_volume_is_not_a_note_head() {
        let blocks = vec![
            block(LiquidBlockRole::Paragraph, "Claim.\u{E000}121\u{E001}"),
            block(
                LiquidBlockRole::Marginalia,
                "121 COLUM. L. REV. 1659, 1675 n.72 (2021).",
            ),
        ];
        let note_source = LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![LiquidSourceLineRef {
                id: Some("p4:l18".to_owned()),
                page_index: 4,
                line_index: 18,
                text: "121 COLUM. L. REV. 1659, 1675 n.72 (2021).".to_owned(),
                role: LiquidBlockRole::Marginalia,
                note_markers: Vec::new(),
            }],
        };
        let (links, integrity) = resolve_footnote_links(&blocks, &[source(0, 4), note_source]);
        assert!(links.is_empty());
        assert_eq!(integrity.note_heads, 0);
        assert_eq!(integrity.unmatched, 1);
    }

    #[test]
    fn article_spans_keep_restarted_note_numbers_in_their_own_article() {
        use crate::liquid::ArticleSpan;
        let blocks = vec![
            block(LiquidBlockRole::Paragraph, "First.\u{E000}1\u{E001}"),
            block(LiquidBlockRole::Marginalia, "1 First article note."),
            block(LiquidBlockRole::Paragraph, "Second.\u{E000}1\u{E001}"),
            block(LiquidBlockRole::Marginalia, "1 Second article note."),
        ];
        let sources = vec![
            source_at(0, 0, 0),
            source_at(1, 0, 2),
            source_at(2, 12, 0),
            source_at(3, 12, 2),
        ];
        let spans = vec![
            ArticleSpan {
                article_index: 0,
                start_page_index: 0,
                start_line_index: 0,
                end_page_index: 12,
                end_line_index: 0,
                confidence: 3.0,
                title_hint: None,
                evidence: Vec::new(),
            },
            ArticleSpan {
                article_index: 1,
                start_page_index: 12,
                start_line_index: 0,
                end_page_index: 20,
                end_line_index: 0,
                confidence: 3.0,
                title_hint: None,
                evidence: Vec::new(),
            },
        ];
        let (links, integrity) = resolve_footnote_links_in_articles(&blocks, &sources, &spans);
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].note_block_index, 1);
        assert_eq!(links[1].note_block_index, 3);
        assert_eq!(integrity.ambiguous, 0);
        assert_eq!(integrity.landing_rate, 1.0);
    }
}
