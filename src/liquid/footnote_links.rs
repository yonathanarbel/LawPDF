use std::collections::BTreeMap;

use super::{
    LiquidBlock, LiquidBlockRole, LiquidBlockSourceLines, LiquidDocument, LiquidFootnoteLink,
    LiquidFootnoteLinkIntegrity,
};

const CALLOUT_START: char = '\u{E000}';
const CALLOUT_END: char = '\u{E001}';

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
    let (links, integrity) = resolve_footnote_links(&document.blocks, &document.block_source_lines);
    document.footnote_links = links;
    document.footnote_link_integrity = (integrity.detectable_markers > 0).then_some(integrity);
}

pub fn resolve_footnote_links(
    blocks: &[LiquidBlock],
    block_source_lines: &[LiquidBlockSourceLines],
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
            if let Some(block_heads) = block_heads
                && !block_heads.is_empty()
            {
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

    let mut links = Vec::new();
    let mut unmatched = 0usize;
    let mut ambiguous = 0usize;
    for reference in &references {
        let same_marker = notes
            .iter()
            .filter(|note| note.marker == reference.marker)
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
            (!heads.is_empty()).then_some((source.block_index, heads))
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
                && (1..=500).contains(&marker)
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
    (1..=500).contains(&marker).then_some(marker)
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
    )
}

fn note_role(role: LiquidBlockRole) -> bool {
    matches!(
        role,
        LiquidBlockRole::Footnote | LiquidBlockRole::Marginalia
    )
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
        LiquidBlockSourceLines {
            block_index,
            lines: vec![LiquidSourceLineRef {
                id: None,
                page_index,
                line_index: 0,
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
}
