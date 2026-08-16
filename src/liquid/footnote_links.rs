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
    allow_unpaged_singleton: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct NoteHead {
    block_index: usize,
    marker: u16,
    page_index: Option<usize>,
    line_index: Option<usize>,
    explicit_source: bool,
}

pub fn attach_footnote_links(document: &mut LiquidDocument) {
    reflow_false_marginalia_body_continuations(
        &mut document.blocks,
        &mut document.block_source_lines,
    );
    restore_source_backed_plain_callouts(&mut document.blocks, &document.block_source_lines);
    repair_truncated_body_callout_markers(
        &mut document.blocks,
        &mut document.block_source_lines,
        &document.article_spans,
    );
    restore_local_plain_callout_markers(
        &mut document.blocks,
        &document.block_source_lines,
        &document.article_spans,
    );
    restore_visual_callout_hints(document);
    let (links, integrity) = resolve_footnote_links_in_articles(
        &document.blocks,
        &document.block_source_lines,
        &document.article_spans,
    );
    document.footnote_links = links;
    document.footnote_link_integrity = (integrity.detectable_markers > 0).then_some(integrity);
}

/// A quoted body paragraph can cross a row whose font resembles the footnote
/// zone. Rejoin only the unmistakable case: an adjacent Marginalia block has
/// no note-head provenance, begins with the lowercase continuation of a source
/// soft-hyphen, ends in a callout with a distinct later same-page note head,
/// and is geometrically bracketed by body rows. Block indices stay stable; its
/// source rows are reassigned to the preceding block and the donor is emptied.
fn reflow_false_marginalia_body_continuations(
    blocks: &mut [LiquidBlock],
    block_source_lines: &mut [LiquidBlockSourceLines],
) -> usize {
    let note_heads = block_note_heads(block_source_lines);
    let mut repairs = Vec::<(usize, usize, usize)>::new();
    for block_index in 1..blocks.len().saturating_sub(1) {
        if !note_role(blocks[block_index].role)
            || !body_role(blocks[block_index - 1].role)
            || !body_role(blocks[block_index + 1].role)
            || !blocks[block_index]
                .text
                .trim_start()
                .chars()
                .next()
                .is_some_and(char::is_lowercase)
        {
            continue;
        }
        let source_positions = |candidate_index| {
            block_source_lines
                .iter()
                .enumerate()
                .filter(|(_, source)| source.block_index == candidate_index)
                .map(|(position, _)| position)
                .collect::<Vec<_>>()
        };
        let previous_positions = source_positions(block_index - 1);
        let current_positions = source_positions(block_index);
        let next_positions = source_positions(block_index + 1);
        let ([previous_position], [current_position], [next_position]) = (
            previous_positions.as_slice(),
            current_positions.as_slice(),
            next_positions.as_slice(),
        ) else {
            continue;
        };
        let previous_source = &block_source_lines[*previous_position];
        let current_source = &block_source_lines[*current_position];
        let next_source = &block_source_lines[*next_position];
        if current_source.lines.is_empty()
            || current_source
                .lines
                .iter()
                .any(|line| !line.note_markers.is_empty() || !note_role(line.role))
        {
            continue;
        }
        let Some(previous_line) = previous_source.lines.last() else {
            continue;
        };
        let Some(current_first) = current_source.lines.first() else {
            continue;
        };
        let Some(current_last) = current_source.lines.last() else {
            continue;
        };
        let Some(next_line) = next_source.lines.first() else {
            continue;
        };
        if !previous_line.text.trim_end().ends_with('\u{0002}')
            || previous_line.page_index != current_first.page_index
            || current_first.line_index != previous_line.line_index.saturating_add(1)
            || current_last.page_index != next_line.page_index
            || next_line.line_index != current_last.line_index.saturating_add(1)
        {
            continue;
        }
        let Some(terminal) = raw_callout_occurrences(&blocks[block_index].text)
            .into_iter()
            .find(|occurrence| blocks[block_index].text[occurrence.end..].trim().is_empty())
        else {
            continue;
        };
        let has_later_head = note_heads.values().flatten().any(|head| {
            head.marker == terminal.marker
                && head.block_index > block_index
                && head.page_index == Some(current_last.page_index)
                && head
                    .line_index
                    .is_some_and(|line| line > current_last.line_index)
        });
        if has_later_head {
            repairs.push((block_index - 1, block_index, *current_position));
        }
    }

    let mut repaired = 0usize;
    for (previous_index, donor_index, source_position) in repairs {
        let donor = blocks[donor_index].text.trim().to_owned();
        let separator = if blocks[previous_index]
            .text
            .chars()
            .next_back()
            .is_some_and(char::is_alphabetic)
            && donor.chars().next().is_some_and(char::is_lowercase)
        {
            ""
        } else {
            " "
        };
        blocks[previous_index].text.push_str(separator);
        blocks[previous_index].text.push_str(&donor);
        blocks[donor_index].text.clear();
        blocks[donor_index].role = LiquidBlockRole::Paragraph;
        blocks[donor_index].label = None;
        block_source_lines[source_position].block_index = previous_index;
        repaired += 1;
    }
    repaired
}

#[derive(Debug, Clone)]
struct PlainCalloutCandidate {
    block_index: usize,
    block_start: usize,
    block_end: usize,
    source_position: usize,
    source_start: usize,
    marker: u16,
    page_index: usize,
    line_index: usize,
    article_index: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct BodyMarkerEvent {
    article_index: Option<usize>,
    page_index: usize,
    line_index: usize,
    source_position: usize,
    source_start: usize,
    marker: u16,
    candidate_index: Option<usize>,
}

/// Restore a superscript that the text layer flattened to ordinary digits.
/// This path intentionally covers only punctuation-attached callouts.  The
/// source row and assembled paragraph must identify the same unique marker +
/// following-word pair, a matching note head must be on the same/next page,
/// and an already trusted neighboring callout must prove N-1 or N+1.  Chains
/// can grow from a trusted marker, but an unanchored run never self-approves.
fn restore_local_plain_callout_markers(
    blocks: &mut [LiquidBlock],
    block_source_lines: &[LiquidBlockSourceLines],
    article_spans: &[super::ArticleSpan],
) -> usize {
    let source_note_heads = block_note_heads(block_source_lines);
    let mut notes = source_note_heads
        .values()
        .flat_map(|heads| heads.iter().copied())
        .collect::<Vec<_>>();
    notes.sort_unstable();
    notes.dedup();
    discard_redundant_marker_only_note_heads(&mut notes, blocks);
    discard_isolated_inferred_note_heads(&mut notes, block_source_lines, article_spans);
    if notes.is_empty() {
        return 0;
    }

    let mut trusted = BTreeSet::<BodyMarkerEvent>::new();
    let mut proposed = BTreeMap::<(usize, usize, u16), Vec<PlainCalloutCandidate>>::new();
    for (source_position, source) in block_source_lines.iter().enumerate() {
        let Some(block) = blocks.get(source.block_index) else {
            continue;
        };
        if !body_role(block.role) {
            continue;
        }
        let article_index =
            block_article_index(source.block_index, block_source_lines, article_spans);
        for (line_position, line) in source.lines.iter().enumerate() {
            let local_markers = notes
                .iter()
                .filter(|note| {
                    note.page_index.is_some_and(|page| {
                        page == line.page_index || page == line.page_index.saturating_add(1)
                    }) && (article_spans.is_empty()
                        || block_article_index(note.block_index, block_source_lines, article_spans)
                            == article_index)
                })
                .map(|note| note.marker)
                .collect::<BTreeSet<_>>();
            if local_markers.is_empty() {
                continue;
            }

            for (source_ordinal, occurrence) in
                raw_callout_occurrences(&line.text).into_iter().enumerate()
            {
                if !local_markers.contains(&occurrence.marker)
                    || !plausible_body_callout_context(&line.text, &occurrence)
                    || source_body_callout_block_ordinal(
                        &block.text,
                        &line.text,
                        source_ordinal,
                        occurrence.marker,
                    )
                    .is_none()
                {
                    continue;
                }
                trusted.insert(BodyMarkerEvent {
                    article_index,
                    page_index: line.page_index,
                    line_index: line.line_index,
                    source_position,
                    source_start: occurrence.start,
                    marker: occurrence.marker,
                    candidate_index: None,
                });
            }

            let source_candidate_text = source
                .lines
                .get(line_position + 1)
                .filter(|next| {
                    next.page_index == line.page_index
                        && next.line_index == line.line_index.saturating_add(1)
                })
                .map(|next| format!("{} {}", line.text, next.text))
                .unwrap_or_else(|| line.text.clone());
            for marker in local_markers {
                let source_candidates =
                    punctuation_plain_callout_candidates(&source_candidate_text, marker)
                        .into_iter()
                        .filter(|(start, _, _, _)| *start < line.text.len())
                        .collect::<Vec<_>>();
                let [(source_start, _, anchor, _source_leading)] = source_candidates.as_slice()
                else {
                    continue;
                };
                let existing_block_markers = raw_callout_occurrences(&block.text)
                    .into_iter()
                    .filter(|occurrence| {
                        occurrence.marker == marker
                            && plausible_body_callout_context(&block.text, occurrence)
                            && following_anchor_word(&block.text[occurrence.end..]) == *anchor
                    })
                    .collect::<Vec<_>>();
                if existing_block_markers.len() == 1 {
                    trusted.insert(BodyMarkerEvent {
                        article_index,
                        page_index: line.page_index,
                        line_index: line.line_index,
                        source_position,
                        source_start: *source_start,
                        marker,
                        candidate_index: None,
                    });
                    continue;
                }
                let block_candidates = punctuation_plain_callout_candidates(&block.text, marker)
                    .into_iter()
                    .filter(|(_, _, block_anchor, _)| block_anchor == anchor)
                    .collect::<Vec<_>>();
                let [(block_start, block_end, _, _)] = block_candidates.as_slice() else {
                    continue;
                };
                proposed
                    .entry((source.block_index, *block_start, marker))
                    .or_default()
                    .push(PlainCalloutCandidate {
                        block_index: source.block_index,
                        block_start: *block_start,
                        block_end: *block_end,
                        source_position,
                        source_start: *source_start,
                        marker,
                        page_index: line.page_index,
                        line_index: line.line_index,
                        article_index,
                    });
            }
        }
    }

    let candidates = proposed
        .into_values()
        .filter_map(|mut matches| (matches.len() == 1).then(|| matches.pop()).flatten())
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return 0;
    }

    let mut events = trusted.into_iter().collect::<Vec<_>>();
    events.extend(
        candidates
            .iter()
            .enumerate()
            .map(|(candidate_index, candidate)| BodyMarkerEvent {
                article_index: candidate.article_index,
                page_index: candidate.page_index,
                line_index: candidate.line_index,
                source_position: candidate.source_position,
                source_start: candidate.source_start,
                marker: candidate.marker,
                candidate_index: Some(candidate_index),
            }),
    );
    events.sort_unstable();

    let mut accepted = vec![false; candidates.len()];
    loop {
        let mut pass = Vec::new();
        for (event_index, event) in events.iter().enumerate() {
            let Some(candidate_index) = event.candidate_index else {
                continue;
            };
            if accepted[candidate_index] {
                continue;
            }
            let is_trusted = |candidate: &BodyMarkerEvent| {
                candidate
                    .candidate_index
                    .is_none_or(|index| accepted[index])
            };
            let previous = events[..event_index].iter().rev().find(|candidate| {
                candidate.article_index == event.article_index
                    && candidate.page_index.abs_diff(event.page_index) <= 1
                    && is_trusted(candidate)
            });
            let next = events[event_index + 1..].iter().find(|candidate| {
                candidate.article_index == event.article_index
                    && candidate.page_index.abs_diff(event.page_index) <= 1
                    && is_trusted(candidate)
            });
            let candidate = &candidates[candidate_index];
            let block_neighbor_proof = blocks
                .get(candidate.block_index)
                .and_then(|block| {
                    raw_callout_occurrences(&block.text)
                        .into_iter()
                        .filter(|occurrence| {
                            occurrence.start < candidate.block_start
                                && plausible_body_callout_context(&block.text, occurrence)
                        })
                        .next_back()
                })
                .is_some_and(|occurrence| occurrence.marker.saturating_add(1) == event.marker);
            if previous.is_some_and(|item| item.marker.saturating_add(1) == event.marker)
                || next.is_some_and(|item| event.marker.saturating_add(1) == item.marker)
                || block_neighbor_proof
            {
                pass.push(candidate_index);
            }
        }
        if pass.is_empty() {
            break;
        }
        for index in pass {
            accepted[index] = true;
        }
    }

    let mut approved = candidates
        .iter()
        .enumerate()
        .filter(|(index, _)| accepted[*index])
        .map(|(_, candidate)| candidate)
        .collect::<Vec<_>>();
    approved.sort_by_key(|candidate| (candidate.block_index, candidate.block_start));
    let mut restored = 0usize;
    for candidate in approved.into_iter().rev() {
        let Some(block) = blocks.get_mut(candidate.block_index) else {
            continue;
        };
        let marker_text = candidate.marker.to_string();
        if block.text.get(candidate.block_start..candidate.block_end) == Some(marker_text.as_str())
        {
            block.text.replace_range(
                candidate.block_start..candidate.block_end,
                &format!("{CALLOUT_START}{}{CALLOUT_END}", candidate.marker),
            );
            restored += 1;
        }
    }
    restored
}

#[derive(Debug, Clone, Copy)]
struct SourceCalloutOccurrence {
    source_position: usize,
    line_position: usize,
    line_ordinal: usize,
    block_index: usize,
    page_index: usize,
    line_index: usize,
    article_index: Option<usize>,
    raw_marker: u16,
    leaked_marker: Option<u16>,
}

/// Some OCR text layers preserve only a suffix of a printed callout (`8`
/// instead of `18`, or `1 1 6` instead of `116`).  A suffix is dangerous:
/// the ordinary exact-number linker can silently send it to note 8 or note 1
/// many pages away and still count that as a landed link.
///
/// Repair only when the body and note apparatus independently agree.  The
/// strongest form is a leaked digit tail plus a matching same/next-page note
/// head.  Otherwise a unique local note-head suffix also needs an immediately
/// adjacent, already trusted N-1 or N+1 body callout.  The operation is
/// deliberately iterative so one proved repair can become the anchor for the
/// next, while unproved suffixes remain untouched.
fn repair_truncated_body_callout_markers(
    blocks: &mut [LiquidBlock],
    block_source_lines: &mut [LiquidBlockSourceLines],
    article_spans: &[super::ArticleSpan],
) -> usize {
    let source_note_heads = block_note_heads(block_source_lines);
    let mut notes = source_note_heads
        .values()
        .flat_map(|heads| heads.iter().copied())
        .collect::<Vec<_>>();
    notes.sort_unstable();
    notes.dedup();
    discard_redundant_marker_only_note_heads(&mut notes, blocks);
    discard_isolated_inferred_note_heads(&mut notes, block_source_lines, article_spans);

    let mut occurrences = Vec::<SourceCalloutOccurrence>::new();
    for (source_position, source) in block_source_lines.iter().enumerate() {
        if !blocks
            .get(source.block_index)
            .is_some_and(|block| body_role(block.role))
        {
            continue;
        }
        let article_index =
            block_article_index(source.block_index, block_source_lines, article_spans);
        for (line_position, line) in source.lines.iter().enumerate() {
            for (line_ordinal, occurrence) in
                raw_callout_occurrences(&line.text).into_iter().enumerate()
            {
                occurrences.push(SourceCalloutOccurrence {
                    source_position,
                    line_position,
                    line_ordinal,
                    block_index: source.block_index,
                    page_index: line.page_index,
                    line_index: line.line_index,
                    article_index,
                    raw_marker: occurrence.marker,
                    leaked_marker: leaked_callout_marker(
                        &line.text,
                        occurrence.end,
                        occurrence.marker,
                    ),
                });
            }
        }
    }
    occurrences.sort_by_key(|item| {
        (
            item.page_index,
            item.line_index,
            item.block_index,
            item.line_ordinal,
        )
    });
    if occurrences.is_empty() || notes.is_empty() {
        return 0;
    }

    let local_note_markers = |occurrence: &SourceCalloutOccurrence| {
        notes
            .iter()
            .filter(|note| {
                note.page_index.is_some_and(|page| {
                    page == occurrence.page_index || page == occurrence.page_index.saturating_add(1)
                }) && (article_spans.is_empty()
                    || block_article_index(note.block_index, block_source_lines, article_spans)
                        == occurrence.article_index)
            })
            .map(|note| note.marker)
            .collect::<BTreeSet<_>>()
    };

    let mut accepted = occurrences
        .iter()
        .map(|occurrence| {
            let local = local_note_markers(occurrence);
            (local.contains(&occurrence.raw_marker)
                && source_body_callout_block_ordinal(
                    &blocks[occurrence.block_index].text,
                    &block_source_lines[occurrence.source_position].lines[occurrence.line_position]
                        .text,
                    occurrence.line_ordinal,
                    occurrence.raw_marker,
                )
                .is_some())
            .then_some(occurrence.raw_marker)
        })
        .collect::<Vec<_>>();
    let mut repairs = BTreeMap::<usize, (u16, bool)>::new();

    loop {
        let mut pass = Vec::<(usize, u16, bool)>::new();
        for (index, occurrence) in occurrences.iter().enumerate() {
            if accepted[index].is_some() {
                continue;
            }
            if source_body_callout_block_ordinal(
                &blocks[occurrence.block_index].text,
                &block_source_lines[occurrence.source_position].lines[occurrence.line_position]
                    .text,
                occurrence.line_ordinal,
                occurrence.raw_marker,
            )
            .is_none()
            {
                continue;
            }
            let local = local_note_markers(occurrence);
            if local.is_empty() {
                continue;
            }
            let suffixes = local
                .iter()
                .copied()
                .filter(|marker| marker_has_strict_suffix(*marker, occurrence.raw_marker))
                .collect::<Vec<_>>();
            if let Some(leaked) = occurrence.leaked_marker
                && leaked > occurrence.raw_marker
                && local.contains(&leaked)
            {
                pass.push((index, leaked, true));
                continue;
            }
            let [candidate] = suffixes.as_slice() else {
                continue;
            };
            let previous_proof = index.checked_sub(1).is_some_and(|previous| {
                occurrences[previous].article_index == occurrence.article_index
                    && occurrences[previous]
                        .page_index
                        .abs_diff(occurrence.page_index)
                        <= 1
                    && accepted[previous] == Some(candidate.saturating_sub(1))
            });
            let next_proof = occurrences.get(index + 1).is_some_and(|next| {
                next.article_index == occurrence.article_index
                    && next.page_index.abs_diff(occurrence.page_index) <= 1
                    && accepted[index + 1] == Some(candidate.saturating_add(1))
            });
            if previous_proof || next_proof {
                pass.push((index, *candidate, false));
            }
        }
        if pass.is_empty() {
            break;
        }
        for (index, marker, strip_leaked) in pass {
            accepted[index] = Some(marker);
            repairs.insert(index, (marker, strip_leaked));
        }
    }

    let mut repaired = 0usize;
    for (&index, &(marker, strip_leaked)) in repairs.iter().rev() {
        let occurrence = occurrences[index];
        let current_line =
            &block_source_lines[occurrence.source_position].lines[occurrence.line_position].text;
        let Some(block_ordinal) = source_body_callout_block_ordinal(
            &blocks[occurrence.block_index].text,
            current_line,
            occurrence.line_ordinal,
            occurrence.raw_marker,
        ) else {
            continue;
        };
        let mut rewritten_line = current_line.clone();
        let mut rewritten_block = blocks[occurrence.block_index].text.clone();
        let line_changed = rewrite_callout_occurrence(
            &mut rewritten_line,
            occurrence.line_ordinal,
            occurrence.raw_marker,
            marker,
            strip_leaked,
        );
        let block_changed = rewrite_callout_occurrence(
            &mut rewritten_block,
            block_ordinal,
            occurrence.raw_marker,
            marker,
            strip_leaked,
        );
        let rewritten_line_is_body = raw_callout_occurrences(&rewritten_line)
            .get(occurrence.line_ordinal)
            .is_some_and(|item| plausible_body_callout_context(&rewritten_line, item));
        let rewritten_block_is_body = raw_callout_occurrences(&rewritten_block)
            .get(block_ordinal)
            .is_some_and(|item| plausible_body_callout_context(&rewritten_block, item));
        if line_changed && block_changed && rewritten_line_is_body && rewritten_block_is_body {
            block_source_lines[occurrence.source_position].lines[occurrence.line_position].text =
                rewritten_line;
            blocks[occurrence.block_index].text = rewritten_block;
            repaired += 1;
        }
    }
    repaired
}

#[derive(Debug, Clone, Copy)]
struct RawCalloutOccurrence {
    start: usize,
    end: usize,
    marker: u16,
}

fn raw_callout_occurrences(text: &str) -> Vec<RawCalloutOccurrence> {
    let mut occurrences = Vec::new();
    let mut cursor = 0usize;
    while let Some(relative) = text[cursor..].find(CALLOUT_START) {
        let start = cursor + relative;
        let marker_start = start + CALLOUT_START.len_utf8();
        let Some(relative_end) = text[marker_start..].find(CALLOUT_END) else {
            break;
        };
        let marker_end = marker_start + relative_end;
        let end = marker_end + CALLOUT_END.len_utf8();
        if let Ok(marker) = text[marker_start..marker_end].parse::<u16>()
            && marker <= MAX_NOTE_MARKER
        {
            occurrences.push(RawCalloutOccurrence { start, end, marker });
        }
        cursor = end;
    }
    occurrences
}

/// Map one source-line callout to one occurrence in its assembled body block.
/// Source and block ordinals are not interchangeable: late paragraph assembly
/// can restore or drop a neighboring marker. Prefer an exact embedded source
/// line, then a singleton raw marker, then require a unique three-token local
/// context. Anything less specific abstains.
fn source_callout_block_ordinal(
    block_text: &str,
    source_line: &str,
    source_ordinal: usize,
    expected_raw: u16,
) -> Option<usize> {
    let source_occurrence = raw_callout_occurrences(source_line)
        .get(source_ordinal)
        .copied()?;
    if source_occurrence.marker != expected_raw {
        return None;
    }
    let block_occurrences = raw_callout_occurrences(block_text);

    let mut exact_lines = block_text.match_indices(source_line);
    if let Some((line_start, _)) = exact_lines.next()
        && exact_lines.next().is_none()
    {
        let exact_start = line_start + source_occurrence.start;
        if let Some((ordinal, _)) = block_occurrences
            .iter()
            .enumerate()
            .find(|(_, occurrence)| {
                occurrence.start == exact_start && occurrence.marker == expected_raw
            })
        {
            return Some(ordinal);
        }
    }

    let raw_candidates = block_occurrences
        .iter()
        .enumerate()
        .filter(|(_, occurrence)| occurrence.marker == expected_raw)
        .collect::<Vec<_>>();
    if let [(ordinal, _)] = raw_candidates.as_slice() {
        return Some(*ordinal);
    }

    let source_before = context_tokens(&source_line[..source_occurrence.start]);
    let source_after = context_tokens(&source_line[source_occurrence.end..]);
    let matches = raw_candidates
        .into_iter()
        .filter(|(_, occurrence)| {
            let block_before = context_tokens(&block_text[..occurrence.start]);
            let block_after = context_tokens(&block_text[occurrence.end..]);
            let before = common_suffix_len(&source_before, &block_before).min(3);
            let after = common_prefix_len(&source_after, &block_after).min(3);
            before + after >= 3
        })
        .map(|(ordinal, _)| ordinal)
        .collect::<Vec<_>>();
    let [ordinal] = matches.as_slice() else {
        return None;
    };
    Some(*ordinal)
}

fn source_body_callout_block_ordinal(
    block_text: &str,
    source_line: &str,
    source_ordinal: usize,
    expected_raw: u16,
) -> Option<usize> {
    let raw_ordinal =
        source_callout_block_ordinal(block_text, source_line, source_ordinal, expected_raw)?;
    raw_callout_occurrences(block_text)
        .get(raw_ordinal)
        .filter(|occurrence| plausible_body_callout_context(block_text, occurrence))
        .map(|_| raw_ordinal)
}

fn plausible_body_callout_context(text: &str, occurrence: &RawCalloutOccurrence) -> bool {
    let before = &text[..occurrence.start];
    let mut previous = before.chars().rev();
    let immediately_before = previous.next();
    let before_that = previous.next();
    let remainder = &text[occurrence.end..];
    let next_visible = remainder.chars().find(|ch| !ch.is_whitespace());
    if immediately_before == Some('.') && before_that.is_some_and(|ch| ch.is_ascii_digit()) {
        let next_word = following_anchor_word(remainder);
        let credible_terminal_callout = remainder.trim().is_empty() && occurrence.marker >= 10;
        if !credible_terminal_callout && !next_word.chars().next().is_some_and(char::is_uppercase) {
            return false;
        }
    }
    if matches!(
        (immediately_before, next_visible),
        (Some('('), Some(')')) | (Some('['), Some(']'))
    ) {
        return false;
    }

    if immediately_before.is_some_and(char::is_alphabetic) {
        let preceding_word = before
            .chars()
            .rev()
            .take_while(|ch| ch.is_alphabetic())
            .collect::<String>();
        let variable = immediately_before.filter(|ch| ch.is_uppercase());
        let paired_small_subscript = occurrence.marker <= 9
            && variable.is_some_and(|variable| {
                raw_callout_occurrences(text).into_iter().any(|other| {
                    other.start != occurrence.start
                        && other.marker <= 9
                        && text[..other.start].chars().next_back() == Some(variable)
                })
            });
        if preceding_word.chars().count() == 1 && paired_small_subscript {
            return false;
        }
        let after_digit_tail = remainder
            .chars()
            .skip_while(|ch| ch.is_whitespace() || ch.is_ascii_digit())
            .next();
        if preceding_word.chars().count() == 1
            && (next_visible.is_some_and(|ch| matches!(ch, ',' | ';' | ':' | ')' | ']'))
                || (next_visible.is_some_and(|ch| ch.is_ascii_digit())
                    && after_digit_tail
                        .is_some_and(|ch| matches!(ch, ',' | ';' | ':' | ')' | ']'))))
        {
            return false;
        }
    }
    true
}

fn body_callout_occurrences(text: &str) -> Vec<(usize, RawCalloutOccurrence)> {
    raw_callout_occurrences(text)
        .into_iter()
        .enumerate()
        .filter(|(_, occurrence)| {
            occurrence.marker > 0 && plausible_body_callout_context(text, occurrence)
        })
        .collect()
}

fn context_tokens(text: &str) -> Vec<String> {
    text.split(|ch: char| !ch.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn common_suffix_len(left: &[String], right: &[String]) -> usize {
    left.iter()
        .rev()
        .zip(right.iter().rev())
        .take_while(|(left, right)| left == right)
        .count()
}

fn common_prefix_len(left: &[String], right: &[String]) -> usize {
    left.iter()
        .zip(right.iter())
        .take_while(|(left, right)| left == right)
        .count()
}

fn marker_has_strict_suffix(marker: u16, suffix: u16) -> bool {
    marker > suffix && marker.to_string().ends_with(&suffix.to_string())
}

fn leaked_callout_marker(text: &str, after: usize, raw_marker: u16) -> Option<u16> {
    let (_, leaked, _) = leaked_callout_tail(text, after)?;
    format!("{raw_marker}{leaked}")
        .parse::<u16>()
        .ok()
        .filter(|marker| *marker <= MAX_NOTE_MARKER)
}

/// Return the byte end, digits without spaces, and whether the next visible
/// character is prose.  At most two leaked digits are accepted because a
/// legal note marker is capped at three digits.
fn leaked_callout_tail(text: &str, after: usize) -> Option<(usize, String, bool)> {
    let mut cursor = after;
    let mut saw_space = false;
    while let Some(ch) = text[cursor..].chars().next()
        && ch.is_whitespace()
    {
        saw_space = true;
        cursor += ch.len_utf8();
    }
    if !saw_space {
        return None;
    }
    let mut digits = String::new();
    let mut end = cursor;
    while digits.len() < 2 {
        let Some(ch) = text[end..].chars().next() else {
            break;
        };
        if ch.is_ascii_digit() {
            digits.push(ch);
            end += ch.len_utf8();
            while let Some(space) = text[end..].chars().next()
                && space.is_whitespace()
            {
                end += space.len_utf8();
            }
        } else {
            break;
        }
    }
    if digits.is_empty() {
        return None;
    }
    let prose_follows = text[end..].chars().next().is_some_and(char::is_alphabetic);
    prose_follows.then_some((end, digits, true))
}

fn rewrite_callout_occurrence(
    text: &mut String,
    ordinal: usize,
    expected_raw: u16,
    marker: u16,
    strip_leaked: bool,
) -> bool {
    let occurrences = raw_callout_occurrences(text);
    let Some(occurrence) = occurrences.get(ordinal).copied() else {
        return false;
    };
    if occurrence.marker != expected_raw {
        return false;
    }
    let replacement = format!("{CALLOUT_START}{marker}{CALLOUT_END}");
    if strip_leaked {
        let Some((tail_end, leaked, _)) = leaked_callout_tail(text, occurrence.end) else {
            return false;
        };
        let Some(expected_leaked) = marker
            .to_string()
            .strip_prefix(&expected_raw.to_string())
            .map(str::to_owned)
        else {
            return false;
        };
        if leaked != expected_leaked {
            return false;
        }
        text.replace_range(occurrence.start..tail_end, &format!("{replacement} "));
    } else {
        text.replace_range(occurrence.start..occurrence.end, &replacement);
    }
    true
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
            let anchor = following_anchor_word(&text[after..]);
            if anchor.len() >= 2 {
                anchors.push((marker, anchor));
            }
        }
        cursor = after;
    }
    anchors
}

fn following_anchor_word(text: &str) -> String {
    text.trim_start()
        .chars()
        .skip_while(|ch| !ch.is_alphabetic())
        .take_while(|ch| ch.is_alphabetic())
        .collect()
}

/// Optional high-precision visual/callout hints from
/// `LAWPDF_VISUAL_CALLOUT_HINTS` (JSONL). Each line is
/// `{"pdf":"...","page_index":0,"line_index":4,"marker":12,"anchor":"Though"}`.
/// A hint is applied only when `marker` + following `anchor` word is unique
/// in both the named source line (or page, for legacy hints) and its body block.
/// Years, citation volumes, and other pages in a merged block stay untouched.
fn restore_visual_callout_hints(document: &mut LiquidDocument) {
    let Ok(path) = std::env::var("LAWPDF_VISUAL_CALLOUT_HINTS") else {
        return;
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return;
    };
    let hints = parse_visual_callout_hints(&text);
    apply_visual_callout_hints(
        &mut document.blocks,
        &document.block_source_lines,
        &document.source_signature,
        &hints,
    );
}

#[derive(Debug, Clone)]
struct VisualCalloutHint {
    pdf_stem: String,
    page_index: usize,
    line_index: Option<usize>,
    marker: u16,
    anchor: String,
}

fn parse_visual_callout_hints(text: &str) -> Vec<VisualCalloutHint> {
    let mut hints = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(marker) = value
            .get("marker")
            .and_then(|v| v.as_u64())
            .and_then(|v| u16::try_from(v).ok())
        else {
            continue;
        };
        if !(1..=MAX_NOTE_MARKER).contains(&marker) {
            continue;
        }
        let anchor = value
            .get("anchor")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .chars()
            .filter(|ch| ch.is_alphabetic())
            .take(24)
            .collect::<String>();
        if anchor.len() < 2 {
            continue;
        }
        let pdf = value.get("pdf").and_then(|v| v.as_str()).unwrap_or("");
        let stem = std::path::Path::new(pdf)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(pdf)
            .to_owned();
        let page_index = value
            .get("page_index")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        let line_index = value
            .get("line_index")
            .and_then(|v| v.as_u64())
            .and_then(|v| usize::try_from(v).ok());
        hints.push(VisualCalloutHint {
            pdf_stem: stem,
            page_index,
            line_index,
            marker,
            anchor,
        });
    }
    hints
}

fn apply_visual_callout_hints(
    blocks: &mut [LiquidBlock],
    block_source_lines: &[LiquidBlockSourceLines],
    source_signature: &str,
    hints: &[VisualCalloutHint],
) -> usize {
    let signature_l = source_signature.to_ascii_lowercase();
    let mut applied = 0usize;
    for hint in hints {
        if !hint.pdf_stem.is_empty() {
            let stem_l = hint.pdf_stem.to_ascii_lowercase();
            if !signature_l.contains(&stem_l) && !stem_l.is_empty() {
                continue;
            }
        }
        for source in block_source_lines {
            let source_occurrences = source
                .lines
                .iter()
                .filter(|line| {
                    line.page_index == hint.page_index
                        && hint.line_index.is_none_or(|index| line.line_index == index)
                })
                .map(|line| plain_callout_candidates(&line.text, hint.marker, &hint.anchor).len())
                .sum::<usize>();
            if source_occurrences != 1 {
                continue;
            }
            let Some(block) = blocks.get_mut(source.block_index) else {
                continue;
            };
            if !body_role(block.role) {
                continue;
            }
            if callout_markers(&block.text).contains(&hint.marker) {
                continue;
            }
            if restore_plain_callout_before_anchor(&mut block.text, hint.marker, &hint.anchor) {
                applied += 1;
                break;
            }
        }
    }
    applied
}

fn restore_plain_callout_before_anchor(text: &mut String, marker: u16, anchor: &str) -> bool {
    let candidates = plain_callout_candidates(text, marker, anchor);
    let [(start, end)] = candidates.as_slice() else {
        return false;
    };
    text.replace_range(
        *start..*end,
        &format!("{CALLOUT_START}{marker}{CALLOUT_END}"),
    );
    true
}

fn plain_callout_candidates(text: &str, marker: u16, anchor: &str) -> Vec<(usize, usize)> {
    let digits = marker.to_string();
    text.match_indices(&digits)
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
        .collect()
}

fn punctuation_plain_callout_candidates(
    text: &str,
    marker: u16,
) -> Vec<(usize, usize, String, bool)> {
    let digits = marker.to_string();
    text.match_indices(&digits)
        .filter_map(|(start, _)| {
            let end = start + digits.len();
            let source_leading = text[..start].trim().is_empty();
            let immediately_before = text[..start].chars().next_back();
            let line_terminal = end == text.len();
            let boundary_after =
                line_terminal || text[end..].chars().next().is_some_and(char::is_whitespace);
            let anchor = following_anchor_word(&text[end..]);
            if !boundary_after || (!line_terminal && anchor.len() < 2) {
                return None;
            }

            let (punctuation, before_punctuation) = if source_leading {
                ('\0', "")
            } else if immediately_before.is_some_and(char::is_whitespace) {
                let trimmed = text[..start].trim_end_matches(char::is_whitespace);
                let punctuation = trimmed.chars().next_back()?;
                let punctuation_start = trimmed.len().saturating_sub(punctuation.len_utf8());
                (punctuation, &trimmed[..punctuation_start])
            } else {
                let immediately_before = immediately_before?;
                let punctuation_start = start.saturating_sub(immediately_before.len_utf8());
                (immediately_before, &text[..punctuation_start])
            };
            let punctuation_attached = punctuation.is_ascii_punctuation()
                || matches!(punctuation, '”' | '’' | '»' | '」' | '』');
            if !source_leading && !punctuation_attached {
                return None;
            }

            if punctuation == '.'
                && before_punctuation
                    .chars()
                    .next_back()
                    .is_some_and(|ch| ch.is_ascii_digit())
            {
                let preceding_digits = before_punctuation
                    .chars()
                    .rev()
                    .take_while(|ch| ch.is_ascii_digit())
                    .count();
                let anchor_starts_uppercase = anchor.chars().next().is_some_and(char::is_uppercase);
                let credible_terminal_callout = line_terminal && marker >= 10;
                if !credible_terminal_callout && (preceding_digits < 4 || !anchor_starts_uppercase)
                {
                    return None;
                }
            }
            Some((start, end, anchor, source_leading))
        })
        .collect()
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
    let reference_pages = block_reference_pages(blocks, block_source_lines);
    let single_pages = block_single_pages(block_source_lines);
    let sourced_blocks = block_source_lines
        .iter()
        .map(|source| source.block_index)
        .collect::<BTreeSet<_>>();
    let source_note_heads = block_note_heads(block_source_lines);
    let mut references = Vec::new();
    let mut notes = Vec::new();
    for (block_index, block) in blocks.iter().enumerate() {
        let page_index = pages.get(&block_index).copied();
        if body_role(block.role) {
            for (ordinal, occurrence) in body_callout_occurrences(&block.text) {
                let marker = occurrence.marker;
                let marker_page = reference_pages
                    .get(&(block_index, ordinal, marker))
                    .copied()
                    .or_else(|| single_pages.get(&block_index).copied());
                references.push(Reference {
                    block_index,
                    ordinal,
                    marker,
                    page_index: marker_page,
                    allow_unpaged_singleton: !sourced_blocks.contains(&block_index),
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
                    line_index: None,
                    explicit_source: true,
                });
            }
        } else if let Some(marker) = super::markdown::numeric_url_note_marker(&block.text) {
            notes.push(NoteHead {
                block_index,
                marker,
                page_index,
                line_index: None,
                explicit_source: true,
            });
        }
    }
    notes.sort_unstable();
    notes.dedup();
    discard_redundant_marker_only_note_heads(&mut notes, blocks);
    discard_isolated_inferred_note_heads(&mut notes, block_source_lines, article_spans);

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
                        || block_article_index(note.block_index, block_source_lines, article_spans)
                            == reference_article)
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
                        line_index: Some(line.line_index),
                        explicit_source: leading_source_note_marker(&line.text) == Some(marker),
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

/// Source-level marker inference is valuable for corrupt-font note heads, but
/// an isolated inferred marker can also be a reporter page or Congressional
/// Record page number. Keep printed/sentineled heads directly. An inferred
/// head must participate in an exact three-note run in physical source order,
/// within its detected article, before it can become a link target.
fn discard_isolated_inferred_note_heads(
    notes: &mut Vec<NoteHead>,
    block_source_lines: &[LiquidBlockSourceLines],
    article_spans: &[super::ArticleSpan],
) {
    let mut groups = BTreeMap::<Option<usize>, Vec<usize>>::new();
    for (index, note) in notes.iter().enumerate() {
        let article_index =
            block_article_index(note.block_index, block_source_lines, article_spans);
        groups.entry(article_index).or_default().push(index);
    }
    let mut sequence_backed = BTreeSet::<usize>::new();
    for indices in groups.values_mut() {
        indices.sort_by_key(|index| {
            let note = notes[*index];
            (
                note.page_index.unwrap_or(usize::MAX),
                note.line_index.unwrap_or(usize::MAX),
                note.block_index,
                note.marker,
            )
        });
        for window in indices.windows(3) {
            let [first, second, third] = window else {
                continue;
            };
            if notes[*second].marker == notes[*first].marker.saturating_add(1)
                && notes[*third].marker == notes[*second].marker.saturating_add(1)
            {
                sequence_backed.extend([*first, *second, *third]);
            }
        }
    }
    let mut index = 0usize;
    notes.retain(|note| {
        let keep = note.explicit_source || sequence_backed.contains(&index);
        index += 1;
        keep
    });
}

fn leading_source_note_marker(text: &str) -> Option<u16> {
    let trimmed = text.trim_start();
    if let Some(tail) = trimmed.strip_prefix(CALLOUT_START) {
        let end = tail.find(CALLOUT_END)?;
        let marker = tail[..end].parse::<u16>().ok()?;
        return (1..=MAX_NOTE_MARKER).contains(&marker).then_some(marker);
    }
    let marker = leading_note_marker(trimmed)?;
    let digits = marker.to_string();
    let remainder = &trimmed[digits.len()..];
    if remainder
        .chars()
        .next()
        .is_some_and(|ch| matches!(ch, '.' | ')' | ']'))
    {
        return Some(marker);
    }
    (remainder.starts_with(char::is_whitespace)
        && !starts_with_reporter_continuation(remainder.trim_start()))
    .then_some(marker)
}

fn starts_with_reporter_continuation(text: &str) -> bool {
    let tokens = text
        .split_whitespace()
        .take(4)
        .map(|token| {
            token
                .chars()
                .filter(|ch| ch.is_ascii_alphanumeric())
                .map(|ch| ch.to_ascii_lowercase())
                .collect::<String>()
        })
        .collect::<Vec<_>>();
    let Some(first) = tokens.first().map(String::as_str) else {
        return false;
    };
    if matches!(
        first,
        "us" | "sct"
            | "f2d"
            | "f3d"
            | "f4th"
            | "fsupp"
            | "fsupp2d"
            | "fsupp3d"
            | "ne"
            | "ne2d"
            | "nw"
            | "nw2d"
            | "se"
            | "se2d"
            | "sw"
            | "sw2d"
            | "so"
            | "so2d"
            | "a2d"
            | "a3d"
            | "p2d"
            | "p3d"
            | "led"
            | "led2d"
            | "eng"
    ) {
        return true;
    }
    matches!(tokens.as_slice(), [first, second, ..]
        if (first == "u" && second == "s")
            || (first == "s" && second == "ct")
            || (first == "sup" && second == "ct")
            || (matches!(first.as_str(), "a" | "f" | "p")
                && (second.starts_with('2') || second.starts_with('3'))))
        || tokens.len() >= 4 && tokens[..4].iter().all(|token| token.len() == 1)
}

fn block_reference_pages(
    blocks: &[LiquidBlock],
    block_source_lines: &[LiquidBlockSourceLines],
) -> BTreeMap<(usize, usize, u16), usize> {
    let mut candidates = BTreeMap::<(usize, usize, u16), BTreeSet<usize>>::new();
    for source in block_source_lines {
        let Some(block) = blocks.get(source.block_index) else {
            continue;
        };
        for line in &source.lines {
            for (source_ordinal, occurrence) in
                raw_callout_occurrences(&line.text).into_iter().enumerate()
            {
                if occurrence.marker == 0 {
                    continue;
                }
                let Some(raw_ordinal) = source_body_callout_block_ordinal(
                    &block.text,
                    &line.text,
                    source_ordinal,
                    occurrence.marker,
                ) else {
                    continue;
                };
                candidates
                    .entry((source.block_index, raw_ordinal, occurrence.marker))
                    .or_default()
                    .insert(line.page_index);
            }
        }
    }
    // Paragraph assembly sometimes promotes a visibly superscript marker that
    // the source row retained only as ordinary digits (`10 The ...`). Recover
    // its physical page only when both sides have one unique marker + following
    // word pair inside the same assembled block. This is page provenance, not
    // a new marker guess: the body block already contains the exact callout.
    for (block_index, block) in blocks.iter().enumerate() {
        if !body_role(block.role) {
            continue;
        }
        let block_occurrences = raw_callout_occurrences(&block.text);
        for (raw_ordinal, occurrence) in body_callout_occurrences(&block.text) {
            let key = (block_index, raw_ordinal, occurrence.marker);
            if candidates.contains_key(&key) {
                continue;
            }
            let anchor = following_anchor_word(&block.text[occurrence.end..]);
            let terminal = block.text[occurrence.end..].trim().is_empty();
            if anchor.len() < 2 && !terminal {
                continue;
            }
            let same_block_pair_count = block_occurrences
                .iter()
                .enumerate()
                .filter(|(candidate_ordinal, item)| {
                    plausible_body_callout_context(&block.text, item)
                        && *candidate_ordinal != raw_ordinal
                        && item.marker == occurrence.marker
                        && following_anchor_word(&block.text[item.end..]) == anchor
                })
                .count()
                + 1;
            if same_block_pair_count != 1 {
                continue;
            }
            let source_matches = block_source_lines
                .iter()
                .filter(|source| source.block_index == block_index)
                .flat_map(|source| source.lines.iter())
                .flat_map(|line| {
                    if anchor.is_empty() {
                        punctuation_plain_callout_candidates(&line.text, occurrence.marker)
                            .into_iter()
                            .filter(|(_, _, source_anchor, _)| source_anchor.is_empty())
                            .map(|_| line.page_index)
                            .collect::<Vec<_>>()
                    } else {
                        plain_callout_candidates(&line.text, occurrence.marker, &anchor)
                            .into_iter()
                            .map(|_| line.page_index)
                            .collect::<Vec<_>>()
                    }
                })
                .collect::<Vec<_>>();
            if let [page] = source_matches.as_slice() {
                candidates.entry(key).or_default().insert(*page);
            }
        }
    }
    candidates
        .into_iter()
        .filter_map(|(key, pages)| {
            (pages.len() == 1)
                .then(|| pages.into_iter().next().map(|page| (key, page)))
                .flatten()
        })
        .collect()
}

fn conservative_candidates<'a>(reference: &Reference, notes: &'a [NoteHead]) -> Vec<&'a NoteHead> {
    let Some(body_page) = reference.page_index else {
        return (reference.allow_unpaged_singleton && notes.len() == 1)
            .then(|| notes.iter().collect())
            .unwrap_or_default();
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
    // A globally unique number is not sufficient provenance. OCR frequently
    // drops leading marker digits, so linking a printed 116 (extracted as 1)
    // to note 1 dozens of pages earlier is a high-confidence wrong target.
    // With physical page evidence available, abstain rather than reward that
    // jump as a successful landing.
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

fn block_single_pages(block_source_lines: &[LiquidBlockSourceLines]) -> BTreeMap<usize, usize> {
    let mut pages = BTreeMap::<usize, BTreeSet<usize>>::new();
    for source in block_source_lines {
        let block_pages = pages.entry(source.block_index).or_default();
        block_pages.extend(source.lines.iter().map(|line| line.page_index));
    }
    pages
        .into_iter()
        .filter_map(|(block_index, pages)| {
            (pages.len() == 1)
                .then(|| pages.into_iter().next().map(|page| (block_index, page)))
                .flatten()
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

    fn source_at(
        block_index: usize,
        page_index: usize,
        line_index: usize,
    ) -> LiquidBlockSourceLines {
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
    fn reflows_soft_hyphen_body_continuation_misclassified_as_marginalia() {
        let mut blocks = vec![
            block(LiquidBlockRole::Paragraph, "terms and condi"),
            block(
                LiquidBlockRole::Marginalia,
                "tions, the consumer accepts.\u{E000}295\u{E001}",
            ),
            block(LiquidBlockRole::Paragraph, "The court continued."),
            block(LiquidBlockRole::Marginalia, "295 Authority."),
        ];
        let mut sources = vec![
            LiquidBlockSourceLines {
                block_index: 0,
                lines: vec![LiquidSourceLineRef {
                    id: Some("p4:l10".to_owned()),
                    page_index: 4,
                    line_index: 10,
                    text: "terms and condi\u{0002}".to_owned(),
                    role: LiquidBlockRole::Paragraph,
                    note_markers: Vec::new(),
                }],
            },
            LiquidBlockSourceLines {
                block_index: 1,
                lines: vec![
                    LiquidSourceLineRef {
                        id: Some("p4:l11".to_owned()),
                        page_index: 4,
                        line_index: 11,
                        text: "tions, the consumer".to_owned(),
                        role: LiquidBlockRole::Marginalia,
                        note_markers: Vec::new(),
                    },
                    LiquidSourceLineRef {
                        id: Some("p4:l12".to_owned()),
                        page_index: 4,
                        line_index: 12,
                        text: "accepts.295".to_owned(),
                        role: LiquidBlockRole::Marginalia,
                        note_markers: Vec::new(),
                    },
                ],
            },
            LiquidBlockSourceLines {
                block_index: 2,
                lines: vec![LiquidSourceLineRef {
                    id: Some("p4:l13".to_owned()),
                    page_index: 4,
                    line_index: 13,
                    text: "The court continued.".to_owned(),
                    role: LiquidBlockRole::Paragraph,
                    note_markers: Vec::new(),
                }],
            },
            note_source(3, 4, 295),
        ];

        let repaired = reflow_false_marginalia_body_continuations(&mut blocks, &mut sources);

        assert_eq!(repaired, 1);
        assert_eq!(
            blocks[0].text,
            "terms and conditions, the consumer accepts.\u{E000}295\u{E001}"
        );
        assert!(blocks[1].text.is_empty());
        assert_eq!(sources[1].block_index, 0);
    }

    #[test]
    fn visual_callout_hint_restores_unique_marker_anchor_pair() {
        let mut blocks = vec![
            block(
                LiquidBlockRole::Paragraph,
                "salaries rose since 1999.63 Though data is scarce.",
            ),
            block(LiquidBlockRole::Marginalia, "63 Biglaw salary scale."),
        ];
        let sources = vec![
            LiquidBlockSourceLines {
                block_index: 0,
                lines: vec![LiquidSourceLineRef {
                    id: None,
                    page_index: 7,
                    line_index: 4,
                    text: blocks[0].text.clone(),
                    role: LiquidBlockRole::Paragraph,
                    note_markers: Vec::new(),
                }],
            },
            source(1, 7),
        ];
        let hints = [VisualCalloutHint {
            pdf_stem: "chicago_art6306".to_owned(),
            page_index: 7,
            line_index: Some(4),
            marker: 63,
            anchor: "Though".to_owned(),
        }];
        let applied =
            apply_visual_callout_hints(&mut blocks, &sources, "sig|chicago_art6306|page7", &hints);
        assert_eq!(applied, 1);
        assert!(
            blocks[0]
                .text
                .contains(&format!("{CALLOUT_START}63{CALLOUT_END}"))
        );
        assert!(!blocks[0].text.contains("1999.63 Though"));
    }

    #[test]
    fn visual_callout_hint_ignores_years_without_matching_anchor() {
        let mut blocks = vec![block(
            LiquidBlockRole::Paragraph,
            "The 1999 study and the 2001 follow-up disagree.",
        )];
        let sources = vec![LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![LiquidSourceLineRef {
                id: None,
                page_index: 2,
                line_index: 0,
                text: blocks[0].text.clone(),
                role: LiquidBlockRole::Paragraph,
                note_markers: Vec::new(),
            }],
        }];
        let hints = [VisualCalloutHint {
            pdf_stem: String::new(),
            page_index: 2,
            line_index: Some(0),
            marker: 99,
            anchor: "Though".to_owned(),
        }];
        let applied = apply_visual_callout_hints(&mut blocks, &sources, "doc", &hints);
        assert_eq!(applied, 0);
        assert_eq!(
            blocks[0].text,
            "The 1999 study and the 2001 follow-up disagree."
        );
    }

    #[test]
    fn visual_callout_hint_does_not_cross_pages_inside_a_merged_block() {
        let mut blocks = vec![block(
            LiquidBlockRole::Paragraph,
            "The first page ends here. 63 Though the next page differs.",
        )];
        let sources = vec![LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![
                LiquidSourceLineRef {
                    id: None,
                    page_index: 4,
                    line_index: 22,
                    text: "The first page ends here.".to_owned(),
                    role: LiquidBlockRole::Paragraph,
                    note_markers: Vec::new(),
                },
                LiquidSourceLineRef {
                    id: None,
                    page_index: 5,
                    line_index: 0,
                    text: "63 Though the next page differs.".to_owned(),
                    role: LiquidBlockRole::Paragraph,
                    note_markers: Vec::new(),
                },
            ],
        }];
        let hints = [VisualCalloutHint {
            pdf_stem: String::new(),
            page_index: 4,
            line_index: Some(22),
            marker: 63,
            anchor: "Though".to_owned(),
        }];

        let applied = apply_visual_callout_hints(&mut blocks, &sources, "doc", &hints);

        assert_eq!(applied, 0);
        assert_eq!(
            blocks[0].text,
            "The first page ends here. 63 Though the next page differs."
        );
    }

    #[test]
    fn caption_url_note_is_a_note_head() {
        let blocks = vec![
            block(
                LiquidBlockRole::Paragraph,
                "Salaries rose.\u{E000}63\u{E001} Though data is scarce.",
            ),
            block(
                LiquidBlockRole::Caption,
                "63 Biglaw Investor, Biglaw Salary Scale, https://www.biglawinvestor.com/biglaw-salary-scale/",
            ),
        ];
        let (links, integrity) = resolve_footnote_links(&blocks, &[]);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].marker, 63);
        assert_eq!(links[0].note_block_index, 1);
        assert_eq!(integrity.unmatched, 0);
        assert_eq!(integrity.note_heads, 1);
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

    fn note_source(block_index: usize, page_index: usize, marker: u16) -> LiquidBlockSourceLines {
        LiquidBlockSourceLines {
            block_index,
            lines: vec![LiquidSourceLineRef {
                id: Some(format!("p{page_index}:note{marker}")),
                page_index,
                line_index: marker as usize,
                text: format!("{marker} Authority."),
                role: LiquidBlockRole::Marginalia,
                note_markers: vec![marker],
            }],
        }
    }

    #[test]
    fn leaked_digits_and_local_note_head_restore_full_marker() {
        let mut blocks = vec![
            block(
                LiquidBlockRole::Paragraph,
                "Claim. \u{E000}1\u{E001} 1 6 The court agreed.",
            ),
            block(LiquidBlockRole::Marginalia, "116 Authority."),
        ];
        let mut sources = vec![
            LiquidBlockSourceLines {
                block_index: 0,
                lines: vec![LiquidSourceLineRef {
                    id: Some("p7:l4".to_owned()),
                    page_index: 7,
                    line_index: 4,
                    text: blocks[0].text.clone(),
                    role: LiquidBlockRole::Paragraph,
                    note_markers: Vec::new(),
                }],
            },
            note_source(1, 7, 116),
        ];
        let raw = raw_callout_occurrences(&blocks[0].text);
        assert_eq!(raw.len(), 1);
        assert_eq!(raw[0].marker, 1);
        assert_eq!(
            leaked_callout_marker(&blocks[0].text, raw[0].end, 1),
            Some(116)
        );
        let heads = block_note_heads(&sources);
        assert_eq!(heads.get(&1).map(Vec::len), Some(1));
        assert!(heads[&1][0].explicit_source);
        let mut filtered = heads.values().flatten().copied().collect::<Vec<_>>();
        discard_redundant_marker_only_note_heads(&mut filtered, &blocks);
        discard_isolated_inferred_note_heads(&mut filtered, &sources, &[]);
        assert_eq!(filtered.len(), 1);
        let mut trial = blocks[0].text.clone();
        assert!(rewrite_callout_occurrence(&mut trial, 0, 1, 116, true));
        assert_eq!(trial, "Claim. \u{E000}116\u{E001} The court agreed.");

        let repaired = repair_truncated_body_callout_markers(&mut blocks, &mut sources, &[]);

        assert_eq!(repaired, 1);
        assert_eq!(
            blocks[0].text,
            "Claim. \u{E000}116\u{E001} The court agreed."
        );
        assert_eq!(sources[0].lines[0].text, blocks[0].text);
    }

    #[test]
    fn unique_local_suffix_needs_trusted_sequence_neighbor() {
        let mut blocks = vec![
            block(
                LiquidBlockRole::Paragraph,
                "Prior.\u{E000}17\u{E001} Target.\u{E000}8\u{E001}",
            ),
            block(LiquidBlockRole::Marginalia, "17 Prior authority."),
            block(LiquidBlockRole::Marginalia, "18 Target authority."),
        ];
        let mut sources = vec![
            LiquidBlockSourceLines {
                block_index: 0,
                lines: vec![LiquidSourceLineRef {
                    id: Some("p4:l2".to_owned()),
                    page_index: 4,
                    line_index: 2,
                    text: blocks[0].text.clone(),
                    role: LiquidBlockRole::Paragraph,
                    note_markers: Vec::new(),
                }],
            },
            note_source(1, 4, 17),
            note_source(2, 4, 18),
        ];

        let repaired = repair_truncated_body_callout_markers(&mut blocks, &mut sources, &[]);

        assert_eq!(repaired, 1);
        assert_eq!(callout_markers(&blocks[0].text), vec![17, 18]);
    }

    #[test]
    fn unbracketed_suffix_match_abstains() {
        let mut blocks = vec![
            block(LiquidBlockRole::Paragraph, "Target.\u{E000}8\u{E001}"),
            block(LiquidBlockRole::Marginalia, "18 Target authority."),
        ];
        let original = blocks[0].text.clone();
        let mut sources = vec![
            LiquidBlockSourceLines {
                block_index: 0,
                lines: vec![LiquidSourceLineRef {
                    id: Some("p4:l2".to_owned()),
                    page_index: 4,
                    line_index: 2,
                    text: original.clone(),
                    role: LiquidBlockRole::Paragraph,
                    note_markers: Vec::new(),
                }],
            },
            note_source(1, 4, 18),
        ];

        let repaired = repair_truncated_body_callout_markers(&mut blocks, &mut sources, &[]);

        assert_eq!(repaired, 0);
        assert_eq!(blocks[0].text, original);
    }

    #[test]
    fn formula_subscript_is_not_repaired_as_a_note() {
        let mut blocks = vec![
            block(
                LiquidBlockRole::Paragraph,
                "Prior.\u{E000}50\u{E001} sector C\u{E000}1\u{E001}, remains constant.",
            ),
            block(LiquidBlockRole::Marginalia, "50 Prior authority."),
            block(LiquidBlockRole::Marginalia, "51 Target authority."),
        ];
        let original = blocks[0].text.clone();
        let mut sources = vec![
            LiquidBlockSourceLines {
                block_index: 0,
                lines: vec![LiquidSourceLineRef {
                    id: Some("p4:l2".to_owned()),
                    page_index: 4,
                    line_index: 2,
                    text: original.clone(),
                    role: LiquidBlockRole::Paragraph,
                    note_markers: Vec::new(),
                }],
            },
            note_source(1, 4, 50),
            note_source(2, 4, 51),
        ];

        let repaired = repair_truncated_body_callout_markers(&mut blocks, &mut sources, &[]);

        assert_eq!(repaired, 0);
        assert_eq!(blocks[0].text, original);
    }

    #[test]
    fn formula_subscript_with_digit_tail_is_not_repaired_from_next_neighbor() {
        let mut blocks = vec![
            block(
                LiquidBlockRole::Paragraph,
                "sector C\u{E000}2\u{E001} 5 2, remains constant.\u{E000}53\u{E001}",
            ),
            block(LiquidBlockRole::Marginalia, "52 Formula authority."),
            block(LiquidBlockRole::Marginalia, "53 Actual authority."),
        ];
        let original = blocks[0].text.clone();
        let mut sources = vec![
            LiquidBlockSourceLines {
                block_index: 0,
                lines: vec![LiquidSourceLineRef {
                    id: Some("p4:l2".to_owned()),
                    page_index: 4,
                    line_index: 2,
                    text: original.clone(),
                    role: LiquidBlockRole::Paragraph,
                    note_markers: Vec::new(),
                }],
            },
            note_source(1, 4, 52),
            note_source(2, 4, 53),
        ];

        let repaired = repair_truncated_body_callout_markers(&mut blocks, &mut sources, &[]);

        assert_eq!(repaired, 0);
        assert_eq!(blocks[0].text, original);
    }

    #[test]
    fn parenthesized_enumeration_is_not_repaired_as_a_note() {
        let mut blocks = vec![
            block(
                LiquidBlockRole::Paragraph,
                "Two examples are (1) the first and (\u{E000}2\u{E001}) the second.\u{E000}93\u{E001}",
            ),
            block(LiquidBlockRole::Marginalia, "92 Unrelated authority."),
            block(LiquidBlockRole::Marginalia, "93 Actual authority."),
        ];
        let original = blocks[0].text.clone();
        let mut sources = vec![
            LiquidBlockSourceLines {
                block_index: 0,
                lines: vec![LiquidSourceLineRef {
                    id: Some("p4:l2".to_owned()),
                    page_index: 4,
                    line_index: 2,
                    text: original.clone(),
                    role: LiquidBlockRole::Paragraph,
                    note_markers: Vec::new(),
                }],
            },
            note_source(1, 4, 92),
            note_source(2, 4, 93),
        ];

        let repaired = repair_truncated_body_callout_markers(&mut blocks, &mut sources, &[]);

        assert_eq!(repaired, 0);
        assert_eq!(blocks[0].text, original);
    }

    #[test]
    fn decimal_and_formula_sentinels_are_not_body_references() {
        let blocks = vec![
            block(
                LiquidBlockRole::Paragraph,
                "The rate was 16.\u{E000}95\u{E001} percent and C\u{E000}52\u{E001}, remained stable.",
            ),
            block(LiquidBlockRole::Marginalia, "52 Formula authority."),
            block(LiquidBlockRole::Marginalia, "95 Percentage authority."),
        ];

        let (links, integrity) = resolve_footnote_links(
            &blocks,
            &[source(0, 4), note_source(1, 4, 52), note_source(2, 4, 95)],
        );

        assert!(links.is_empty());
        assert_eq!(integrity.detectable_markers, 0);
    }

    #[test]
    fn paired_single_letter_subscripts_are_not_body_references() {
        let blocks = vec![
            block(
                LiquidBlockRole::Paragraph,
                "Should a contract at T\u{E000}1\u{E001} become waste at T\u{E000}2\u{E001}?",
            ),
            block(LiquidBlockRole::Marginalia, "1 Unrelated authority."),
            block(LiquidBlockRole::Marginalia, "2 Unrelated authority."),
        ];

        let (links, integrity) = resolve_footnote_links(
            &blocks,
            &[source(0, 4), note_source(1, 4, 1), note_source(2, 4, 2)],
        );

        assert!(links.is_empty());
        assert_eq!(integrity.detectable_markers, 0);
    }

    #[test]
    fn year_adjacent_sentinel_with_prose_anchor_is_a_body_reference() {
        let blocks = vec![
            block(
                LiquidBlockRole::Paragraph,
                "The study ended in 1992.\u{E000}85\u{E001} But later work disagreed.",
            ),
            block(LiquidBlockRole::Marginalia, "85 Authority."),
        ];

        let (links, integrity) =
            resolve_footnote_links(&blocks, &[source(0, 4), note_source(1, 4, 85)]);

        assert_eq!(links.len(), 1);
        assert_eq!(links[0].marker, 85);
        assert_eq!(integrity.detectable_markers, 1);
    }

    #[test]
    fn number_adjacent_terminal_sentinel_is_a_body_reference() {
        let blocks = vec![
            block(
                LiquidBlockRole::Paragraph,
                "The decline dates to 1800.\u{E000}105\u{E001}",
            ),
            block(LiquidBlockRole::Marginalia, "105 Authority."),
        ];

        let (links, integrity) =
            resolve_footnote_links(&blocks, &[source(0, 4), note_source(1, 4, 105)]);

        assert_eq!(links.len(), 1);
        assert_eq!(links[0].marker, 105);
        assert_eq!(integrity.detectable_markers, 1);
    }

    #[test]
    fn excluded_semantic_sentinels_do_not_shift_later_reference_ordinals() {
        let blocks = vec![
            block(
                LiquidBlockRole::Paragraph,
                "The rate was 16.\u{E000}95\u{E001} percent, C\u{E000}52\u{E001}, then changed.\u{E000}53\u{E001}",
            ),
            block(LiquidBlockRole::Marginalia, "52 Formula authority."),
            block(LiquidBlockRole::Marginalia, "53 Actual authority."),
            block(LiquidBlockRole::Marginalia, "95 Percentage authority."),
        ];

        let (links, integrity) = resolve_footnote_links(
            &blocks,
            &[
                source(0, 4),
                note_source(1, 4, 52),
                note_source(2, 4, 53),
                note_source(3, 4, 95),
            ],
        );

        assert_eq!(links.len(), 1);
        assert_eq!(links[0].marker, 53);
        assert_eq!(links[0].body_marker_ordinal, 2);
        assert_eq!(integrity.detectable_markers, 1);
        assert_eq!(integrity.landing_rate, 1.0);
    }

    #[test]
    fn failed_block_rewrite_does_not_mutate_source_line() {
        let mut blocks = vec![
            block(
                LiquidBlockRole::Paragraph,
                "Claim without a preserved callout.",
            ),
            block(LiquidBlockRole::Marginalia, "116 Authority."),
        ];
        let source_text = "Claim. \u{E000}1\u{E001} 1 6 The court agreed.".to_owned();
        let mut sources = vec![
            LiquidBlockSourceLines {
                block_index: 0,
                lines: vec![LiquidSourceLineRef {
                    id: Some("p7:l4".to_owned()),
                    page_index: 7,
                    line_index: 4,
                    text: source_text.clone(),
                    role: LiquidBlockRole::Paragraph,
                    note_markers: Vec::new(),
                }],
            },
            note_source(1, 7, 116),
        ];

        let repaired = repair_truncated_body_callout_markers(&mut blocks, &mut sources, &[]);

        assert_eq!(repaired, 0);
        assert_eq!(sources[0].lines[0].text, source_text);
    }

    #[test]
    fn duplicate_source_records_share_their_block_ordinal() {
        let mut blocks = vec![
            block(
                LiquidBlockRole::Paragraph,
                "Prior.\u{E000}17\u{E001} Target.\u{E000}8\u{E001}",
            ),
            block(LiquidBlockRole::Marginalia, "17 Prior authority."),
            block(LiquidBlockRole::Marginalia, "18 Target authority."),
        ];
        let mut sources = vec![
            LiquidBlockSourceLines {
                block_index: 0,
                lines: vec![LiquidSourceLineRef {
                    id: Some("p4:l1".to_owned()),
                    page_index: 4,
                    line_index: 1,
                    text: "Prior.\u{E000}17\u{E001}".to_owned(),
                    role: LiquidBlockRole::Paragraph,
                    note_markers: Vec::new(),
                }],
            },
            LiquidBlockSourceLines {
                block_index: 0,
                lines: vec![LiquidSourceLineRef {
                    id: Some("p4:l2".to_owned()),
                    page_index: 4,
                    line_index: 2,
                    text: "Target.\u{E000}8\u{E001}".to_owned(),
                    role: LiquidBlockRole::Paragraph,
                    note_markers: Vec::new(),
                }],
            },
            note_source(1, 4, 17),
            note_source(2, 4, 18),
        ];

        let repaired = repair_truncated_body_callout_markers(&mut blocks, &mut sources, &[]);

        assert_eq!(repaired, 1);
        assert_eq!(callout_markers(&blocks[0].text), vec![17, 18]);
        assert_eq!(callout_markers(&sources[1].lines[0].text), vec![18]);
    }

    #[test]
    fn repeated_raw_markers_use_unique_source_context() {
        let mut blocks = vec![
            block(
                LiquidBlockRole::Paragraph,
                "Unrelated claim.\u{E000}8\u{E001} Prior.\u{E000}17\u{E001} Target claim.\u{E000}8\u{E001}",
            ),
            block(LiquidBlockRole::Marginalia, "17 Prior authority."),
            block(LiquidBlockRole::Marginalia, "18 Target authority."),
        ];
        let mut sources = vec![
            LiquidBlockSourceLines {
                block_index: 0,
                lines: vec![
                    LiquidSourceLineRef {
                        id: Some("p4:l1".to_owned()),
                        page_index: 4,
                        line_index: 1,
                        text: "Unrelated claim.\u{E000}8\u{E001}".to_owned(),
                        role: LiquidBlockRole::Paragraph,
                        note_markers: Vec::new(),
                    },
                    LiquidSourceLineRef {
                        id: Some("p4:l2".to_owned()),
                        page_index: 4,
                        line_index: 2,
                        text: "Prior.\u{E000}17\u{E001}".to_owned(),
                        role: LiquidBlockRole::Paragraph,
                        note_markers: Vec::new(),
                    },
                    LiquidSourceLineRef {
                        id: Some("p4:l3".to_owned()),
                        page_index: 4,
                        line_index: 3,
                        text: "Target claim.\u{E000}8\u{E001}".to_owned(),
                        role: LiquidBlockRole::Paragraph,
                        note_markers: Vec::new(),
                    },
                ],
            },
            note_source(1, 4, 17),
            note_source(2, 4, 18),
        ];

        let repaired = repair_truncated_body_callout_markers(&mut blocks, &mut sources, &[]);

        assert_eq!(repaired, 1);
        assert_eq!(callout_markers(&blocks[0].text), vec![8, 17, 18]);
        assert_eq!(callout_markers(&sources[0].lines[2].text), vec![18]);
    }

    #[test]
    fn identical_repeated_source_context_abstains() {
        let block = "Target.\u{E000}8\u{E001} Target.\u{E000}8\u{E001}";
        let source = "Target.\u{E000}8\u{E001}";

        assert_eq!(source_callout_block_ordinal(block, source, 0, 8), None);
    }

    #[test]
    fn globally_unique_far_note_is_not_a_valid_target() {
        let blocks = vec![
            block(LiquidBlockRole::Paragraph, "Claim.\u{E000}1\u{E001}"),
            block(LiquidBlockRole::Marginalia, "1 Other article authority."),
        ];

        let (links, integrity) = resolve_footnote_links(&blocks, &[source(0, 12), source(1, 0)]);

        assert!(links.is_empty());
        assert_eq!(integrity.unmatched, 1);
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
    fn isolated_inferred_reporter_page_is_not_a_note_head() {
        let blocks = vec![
            block(
                LiquidBlockRole::Paragraph,
                "Debate continued.\u{E000}875\u{E001}",
            ),
            block(
                LiquidBlockRole::Marginalia,
                "ra89 Cong. Rec. 875 (Mr. Halleck).",
            ),
        ];
        let inferred_note = LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![LiquidSourceLineRef {
                id: Some("p10:l34".to_owned()),
                page_index: 10,
                line_index: 34,
                text: "ra89 Cong. Rec. 875 (Mr. Halleck).".to_owned(),
                role: LiquidBlockRole::Marginalia,
                note_markers: vec![875],
            }],
        };

        let (links, integrity) = resolve_footnote_links(&blocks, &[source(0, 10), inferred_note]);

        assert!(links.is_empty());
        assert_eq!(integrity.note_heads, 0);
        assert_eq!(integrity.unmatched, 1);
    }

    #[test]
    fn citation_volume_prefixes_are_not_explicit_note_heads() {
        for (marker, text) in [
            (232, "232 U. S. 619, 625 (1914)."),
            (24, "24 A. (2d) 1 (Pa. 1942)."),
            (62, "62 Sup. Ct. 608 (1942)."),
            (27, "27 A. B. A. J. 547."),
        ] {
            assert_eq!(leading_source_note_marker(text), None, "{text}");
            assert_eq!(leading_note_marker(text), Some(marker));
        }
        assert_eq!(
            leading_source_note_marker("27 A useful authority."),
            Some(27)
        );
        assert_eq!(leading_source_note_marker("27. 232 U.S. 619."), Some(27));
        assert_eq!(
            leading_source_note_marker("\u{E000}27\u{E001} corrupt glyph authority"),
            Some(27)
        );
    }

    #[test]
    fn consecutive_inferred_source_heads_remain_linkable() {
        let mut blocks = Vec::new();
        let mut sources = Vec::new();
        for marker in 39..=41 {
            let body_index = blocks.len();
            blocks.push(block(
                LiquidBlockRole::Paragraph,
                &format!("Claim.\u{E000}{marker}\u{E001}"),
            ));
            sources.push(source_at(body_index, 8, marker as usize));
            let note_index = blocks.len();
            blocks.push(block(
                LiquidBlockRole::Marginalia,
                &format!("corrupt glyph authority for note {marker}"),
            ));
            sources.push(LiquidBlockSourceLines {
                block_index: note_index,
                lines: vec![LiquidSourceLineRef {
                    id: Some(format!("p8:l{}", marker + 20)),
                    page_index: 8,
                    line_index: (marker + 20) as usize,
                    text: "corrupt glyph authority".to_owned(),
                    role: LiquidBlockRole::Marginalia,
                    note_markers: vec![marker],
                }],
            });
        }

        let (links, integrity) = resolve_footnote_links(&blocks, &sources);

        assert_eq!(links.len(), 3);
        assert_eq!(integrity.note_heads, 3);
        assert_eq!(integrity.landing_rate, 1.0);
    }

    #[test]
    fn merged_multi_page_block_maps_reference_pages_by_source_context() {
        let blocks = vec![
            block(
                LiquidBlockRole::Paragraph,
                "Early.\u{E000}1\u{E001} Unproved.\u{E000}2\u{E001} Later claim.\u{E000}7\u{E001}",
            ),
            block(LiquidBlockRole::Marginalia, "1 Early authority."),
            block(LiquidBlockRole::Marginalia, "2 Unproved authority."),
            block(LiquidBlockRole::Marginalia, "7 Later authority."),
        ];
        let sources = vec![
            LiquidBlockSourceLines {
                block_index: 0,
                lines: vec![LiquidSourceLineRef {
                    id: Some("p0:l1".to_owned()),
                    page_index: 0,
                    line_index: 1,
                    text: "Early.\u{E000}1\u{E001}".to_owned(),
                    role: LiquidBlockRole::Paragraph,
                    note_markers: Vec::new(),
                }],
            },
            LiquidBlockSourceLines {
                block_index: 0,
                lines: vec![LiquidSourceLineRef {
                    id: Some("p5:l1".to_owned()),
                    page_index: 5,
                    line_index: 1,
                    text: "Later claim.\u{E000}7\u{E001}".to_owned(),
                    role: LiquidBlockRole::Paragraph,
                    note_markers: Vec::new(),
                }],
            },
            note_source(1, 0, 1),
            note_source(2, 0, 2),
            note_source(3, 5, 7),
        ];

        let (links, integrity) = resolve_footnote_links(&blocks, &sources);

        assert_eq!(
            links.iter().map(|link| link.marker).collect::<Vec<_>>(),
            vec![1, 7]
        );
        assert_eq!(links[1].body_page_index, Some(5));
        assert_eq!(links[1].note_page_index, Some(5));
        assert_eq!(integrity.unmatched, 1);
    }

    #[test]
    fn merged_multi_page_block_maps_plain_source_marker_by_anchor() {
        let blocks = vec![
            block(
                LiquidBlockRole::Paragraph,
                "Earlier prose. Claim.\u{E000}10\u{E001} The analysis continues onto another page.",
            ),
            block(LiquidBlockRole::Marginalia, "10 Authority."),
        ];
        let sources = vec![
            LiquidBlockSourceLines {
                block_index: 0,
                lines: vec![
                    LiquidSourceLineRef {
                        id: Some("p3:l30".to_owned()),
                        page_index: 3,
                        line_index: 30,
                        text: "Earlier prose. Claim.".to_owned(),
                        role: LiquidBlockRole::Paragraph,
                        note_markers: Vec::new(),
                    },
                    LiquidSourceLineRef {
                        id: Some("p3:l31".to_owned()),
                        page_index: 3,
                        line_index: 31,
                        text: "10 The analysis continues".to_owned(),
                        role: LiquidBlockRole::Marginalia,
                        note_markers: Vec::new(),
                    },
                    LiquidSourceLineRef {
                        id: Some("p4:l1".to_owned()),
                        page_index: 4,
                        line_index: 1,
                        text: "onto another page.".to_owned(),
                        role: LiquidBlockRole::Paragraph,
                        note_markers: Vec::new(),
                    },
                ],
            },
            note_source(1, 3, 10),
        ];

        let (links, integrity) = resolve_footnote_links(&blocks, &sources);

        assert_eq!(links.len(), 1);
        assert_eq!(links[0].body_page_index, Some(3));
        assert_eq!(links[0].note_page_index, Some(3));
        assert_eq!(integrity.unmatched, 0);
    }

    #[test]
    fn merged_multi_page_block_maps_terminal_plain_source_marker() {
        let blocks = vec![
            block(
                LiquidBlockRole::Paragraph,
                "Earlier prose crosses a page. The decline dates to 1800.\u{E000}105\u{E001}",
            ),
            block(LiquidBlockRole::Marginalia, "105 Authority."),
        ];
        let sources = vec![
            LiquidBlockSourceLines {
                block_index: 0,
                lines: vec![
                    LiquidSourceLineRef {
                        id: Some("p3:l30".to_owned()),
                        page_index: 3,
                        line_index: 30,
                        text: "Earlier prose crosses a page.".to_owned(),
                        role: LiquidBlockRole::Paragraph,
                        note_markers: Vec::new(),
                    },
                    LiquidSourceLineRef {
                        id: Some("p4:l1".to_owned()),
                        page_index: 4,
                        line_index: 1,
                        text: "The decline dates to 1800.105".to_owned(),
                        role: LiquidBlockRole::Paragraph,
                        note_markers: Vec::new(),
                    },
                ],
            },
            note_source(1, 4, 105),
        ];

        let (links, integrity) = resolve_footnote_links(&blocks, &sources);

        assert_eq!(links.len(), 1);
        assert_eq!(links[0].body_page_index, Some(4));
        assert_eq!(links[0].note_page_index, Some(4));
        assert_eq!(integrity.unmatched, 0);
    }

    #[test]
    fn duplicate_plain_source_marker_anchor_abstains() {
        let blocks = vec![
            block(
                LiquidBlockRole::Paragraph,
                "Claim.\u{E000}10\u{E001} The analysis continues.",
            ),
            block(LiquidBlockRole::Marginalia, "10 Authority."),
        ];
        let sources = vec![
            LiquidBlockSourceLines {
                block_index: 0,
                lines: vec![
                    LiquidSourceLineRef {
                        id: Some("p3:l1".to_owned()),
                        page_index: 3,
                        line_index: 1,
                        text: "10 The analysis".to_owned(),
                        role: LiquidBlockRole::Paragraph,
                        note_markers: Vec::new(),
                    },
                    LiquidSourceLineRef {
                        id: Some("p4:l1".to_owned()),
                        page_index: 4,
                        line_index: 1,
                        text: "10 The analysis".to_owned(),
                        role: LiquidBlockRole::Paragraph,
                        note_markers: Vec::new(),
                    },
                ],
            },
            note_source(1, 3, 10),
        ];

        let (links, integrity) = resolve_footnote_links(&blocks, &sources);

        assert!(links.is_empty());
        assert_eq!(integrity.unmatched, 1);
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
    fn restores_punctuation_attached_plain_callout_from_local_sequence() {
        let mut blocks = vec![
            block(
                LiquidBlockRole::Paragraph,
                "Prior.\u{E000}51\u{E001} The quotation ends.”52 Then the discussion continues.\u{E000}53\u{E001}",
            ),
            block(LiquidBlockRole::Marginalia, "51 Prior authority."),
            block(LiquidBlockRole::Marginalia, "52 Restored authority."),
            block(LiquidBlockRole::Marginalia, "53 Later authority."),
        ];
        let body_text = blocks[0].text.clone();
        let sources = vec![
            LiquidBlockSourceLines {
                block_index: 0,
                lines: vec![LiquidSourceLineRef {
                    id: Some("p8:l6".to_owned()),
                    page_index: 8,
                    line_index: 6,
                    text: body_text,
                    role: LiquidBlockRole::Paragraph,
                    note_markers: Vec::new(),
                }],
            },
            note_source(1, 8, 51),
            note_source(2, 8, 52),
            note_source(3, 8, 53),
        ];

        let restored = restore_local_plain_callout_markers(&mut blocks, &sources, &[]);

        assert_eq!(restored, 1);
        assert!(blocks[0].text.contains("ends.”\u{E000}52\u{E001} Then"));
    }

    #[test]
    fn restores_year_adjacent_plain_callout_from_local_sequence() {
        let mut blocks = vec![
            block(
                LiquidBlockRole::Paragraph,
                "Prior.\u{E000}62\u{E001} Salaries rose in 1999.63 Though data is scarce.\u{E000}64\u{E001}",
            ),
            block(LiquidBlockRole::Marginalia, "62 Prior authority."),
            block(LiquidBlockRole::Marginalia, "63 Restored authority."),
            block(LiquidBlockRole::Marginalia, "64 Later authority."),
        ];
        let body_text = blocks[0].text.clone();
        let sources = vec![
            LiquidBlockSourceLines {
                block_index: 0,
                lines: vec![LiquidSourceLineRef {
                    id: Some("p8:l6".to_owned()),
                    page_index: 8,
                    line_index: 6,
                    text: body_text,
                    role: LiquidBlockRole::Paragraph,
                    note_markers: Vec::new(),
                }],
            },
            note_source(1, 8, 62),
            note_source(2, 8, 63),
            note_source(3, 8, 64),
        ];

        let restored = restore_local_plain_callout_markers(&mut blocks, &sources, &[]);

        assert_eq!(restored, 1);
        assert!(blocks[0].text.contains("1999.\u{E000}63\u{E001} Though"));
    }

    #[test]
    fn restores_spaced_punctuation_callout_from_local_sequence() {
        let mut blocks = vec![
            block(
                LiquidBlockRole::Paragraph,
                "Prior.\u{E000}234\u{E001} ChatGPT has “plugins.” 235 For example.\u{E000}236\u{E001}",
            ),
            block(LiquidBlockRole::Marginalia, "234 Prior authority."),
            block(LiquidBlockRole::Marginalia, "235 Restored authority."),
            block(LiquidBlockRole::Marginalia, "236 Later authority."),
        ];
        let body_text = blocks[0].text.clone();
        let sources = vec![
            LiquidBlockSourceLines {
                block_index: 0,
                lines: vec![LiquidSourceLineRef {
                    id: Some("p8:l6".to_owned()),
                    page_index: 8,
                    line_index: 6,
                    text: body_text,
                    role: LiquidBlockRole::Paragraph,
                    note_markers: Vec::new(),
                }],
            },
            note_source(1, 8, 234),
            note_source(2, 8, 235),
            note_source(3, 8, 236),
        ];

        let restored = restore_local_plain_callout_markers(&mut blocks, &sources, &[]);

        assert_eq!(restored, 1);
        assert!(blocks[0].text.contains("plugins.” \u{E000}235\u{E001} For"));
    }

    #[test]
    fn restores_leading_source_line_callout_from_local_sequence() {
        let mut blocks = vec![
            block(
                LiquidBlockRole::Paragraph,
                "Prior.\u{E000}112\u{E001} McCleskey v. Kemp. 113 There, the Court ruled.\u{E000}114\u{E001}",
            ),
            block(LiquidBlockRole::Marginalia, "112 Prior authority."),
            block(LiquidBlockRole::Marginalia, "113 Restored authority."),
            block(LiquidBlockRole::Marginalia, "114 Later authority."),
        ];
        let sources = vec![
            LiquidBlockSourceLines {
                block_index: 0,
                lines: vec![
                    LiquidSourceLineRef {
                        id: Some("p8:l5".to_owned()),
                        page_index: 8,
                        line_index: 5,
                        text: "Prior.\u{E000}112\u{E001} McCleskey v. Kemp.".to_owned(),
                        role: LiquidBlockRole::Paragraph,
                        note_markers: Vec::new(),
                    },
                    LiquidSourceLineRef {
                        id: Some("p8:l6".to_owned()),
                        page_index: 8,
                        line_index: 6,
                        text: "113 There, the Court ruled.\u{E000}114\u{E001}".to_owned(),
                        role: LiquidBlockRole::Paragraph,
                        note_markers: Vec::new(),
                    },
                ],
            },
            note_source(1, 8, 112),
            note_source(2, 8, 113),
            note_source(3, 8, 114),
        ];

        let restored = restore_local_plain_callout_markers(&mut blocks, &sources, &[]);

        assert_eq!(restored, 1);
        assert!(blocks[0].text.contains("Kemp. \u{E000}113\u{E001} There"));
    }

    #[test]
    fn restores_line_terminal_callout_using_next_source_anchor() {
        let mut blocks = vec![
            block(
                LiquidBlockRole::Paragraph,
                "Prior.\u{E000}104\u{E001} The decline dates to 1800.105 Curiously, no one connected it.\u{E000}106\u{E001}",
            ),
            block(LiquidBlockRole::Marginalia, "104 Prior authority."),
            block(LiquidBlockRole::Marginalia, "105 Restored authority."),
            block(LiquidBlockRole::Marginalia, "106 Later authority."),
        ];
        let sources = vec![
            LiquidBlockSourceLines {
                block_index: 0,
                lines: vec![
                    LiquidSourceLineRef {
                        id: Some("p8:l5".to_owned()),
                        page_index: 8,
                        line_index: 5,
                        text: "Prior.\u{E000}104\u{E001} The decline dates to 1800.105".to_owned(),
                        role: LiquidBlockRole::Paragraph,
                        note_markers: Vec::new(),
                    },
                    LiquidSourceLineRef {
                        id: Some("p8:l6".to_owned()),
                        page_index: 8,
                        line_index: 6,
                        text: "Curiously, no one connected it.\u{E000}106\u{E001}".to_owned(),
                        role: LiquidBlockRole::Paragraph,
                        note_markers: Vec::new(),
                    },
                ],
            },
            note_source(1, 8, 104),
            note_source(2, 8, 105),
            note_source(3, 8, 106),
        ];

        let restored = restore_local_plain_callout_markers(&mut blocks, &sources, &[]);

        assert_eq!(restored, 1);
        assert!(
            blocks[0]
                .text
                .contains("1800.\u{E000}105\u{E001} Curiously")
        );
    }

    #[test]
    fn restores_callout_at_end_of_paragraph_from_previous_sequence_marker() {
        let mut blocks = vec![
            block(
                LiquidBlockRole::Paragraph,
                "Prior.\u{E000}104\u{E001} The decline dates to 1800.105",
            ),
            block(LiquidBlockRole::Marginalia, "104 Prior authority."),
            block(LiquidBlockRole::Marginalia, "105 Restored authority."),
        ];
        let body_text = blocks[0].text.clone();
        let sources = vec![
            LiquidBlockSourceLines {
                block_index: 0,
                lines: vec![LiquidSourceLineRef {
                    id: Some("p8:l6".to_owned()),
                    page_index: 8,
                    line_index: 6,
                    text: body_text,
                    role: LiquidBlockRole::Paragraph,
                    note_markers: Vec::new(),
                }],
            },
            note_source(1, 8, 104),
            note_source(2, 8, 105),
        ];

        let restored = restore_local_plain_callout_markers(&mut blocks, &sources, &[]);

        assert_eq!(restored, 1);
        assert!(blocks[0].text.ends_with("1800.\u{E000}105\u{E001}"));
    }

    #[test]
    fn repaired_block_neighbors_prove_plain_source_callout_sequence() {
        let mut blocks = vec![
            block(
                LiquidBlockRole::Paragraph,
                "Expensive.\u{E000}126\u{E001} Today, trial by paper,”127 illustrates the point.\u{E000}128\u{E001} Summary follows.",
            ),
            block(LiquidBlockRole::Marginalia, "126 Prior authority."),
            block(LiquidBlockRole::Marginalia, "127 Restored authority."),
            block(LiquidBlockRole::Marginalia, "128 Later authority."),
        ];
        let sources = vec![
            LiquidBlockSourceLines {
                block_index: 0,
                lines: vec![
                    LiquidSourceLineRef {
                        id: Some("p8:l5".to_owned()),
                        page_index: 8,
                        line_index: 5,
                        text: "126 Today, trial by".to_owned(),
                        role: LiquidBlockRole::Paragraph,
                        note_markers: Vec::new(),
                    },
                    LiquidSourceLineRef {
                        id: Some("p8:l6".to_owned()),
                        page_index: 8,
                        line_index: 6,
                        text: "paper,”127 illustrates the point.".to_owned(),
                        role: LiquidBlockRole::Paragraph,
                        note_markers: Vec::new(),
                    },
                    LiquidSourceLineRef {
                        id: Some("p8:l7".to_owned()),
                        page_index: 8,
                        line_index: 7,
                        text: "128 Summary follows.".to_owned(),
                        role: LiquidBlockRole::Paragraph,
                        note_markers: Vec::new(),
                    },
                ],
            },
            note_source(1, 8, 126),
            note_source(2, 8, 127),
            note_source(3, 8, 128),
        ];

        let restored = restore_local_plain_callout_markers(&mut blocks, &sources, &[]);

        assert_eq!(restored, 1);
        assert!(
            blocks[0]
                .text
                .contains("paper,”\u{E000}127\u{E001} illustrates")
        );
    }

    #[test]
    fn local_sequence_does_not_restore_decimal_digit() {
        let mut blocks = vec![
            block(
                LiquidBlockRole::Paragraph,
                "Prior.\u{E000}4\u{E001} The rate is 16.5 percent. Later.\u{E000}6\u{E001}",
            ),
            block(LiquidBlockRole::Marginalia, "4 Prior authority."),
            block(LiquidBlockRole::Marginalia, "5 Unrelated authority."),
            block(LiquidBlockRole::Marginalia, "6 Later authority."),
        ];
        let body_text = blocks[0].text.clone();
        let sources = vec![
            LiquidBlockSourceLines {
                block_index: 0,
                lines: vec![LiquidSourceLineRef {
                    id: Some("p8:l6".to_owned()),
                    page_index: 8,
                    line_index: 6,
                    text: body_text,
                    role: LiquidBlockRole::Paragraph,
                    note_markers: Vec::new(),
                }],
            },
            note_source(1, 8, 4),
            note_source(2, 8, 5),
            note_source(3, 8, 6),
        ];

        let restored = restore_local_plain_callout_markers(&mut blocks, &sources, &[]);

        assert_eq!(restored, 0);
        assert!(blocks[0].text.contains("16.5 percent"));
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
