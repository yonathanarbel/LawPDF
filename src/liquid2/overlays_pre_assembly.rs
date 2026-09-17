//! Callout and sequence overlays that run between decoding and block assembly.

use super::*;

/// Recover an oversized initial that PDFium emitted after the paragraph it
/// visually begins. Law-review drop caps are commonly separate PDF text
/// objects, so extraction order can place the letter after author notes even
/// though its box sits immediately left of the opening body line.
pub(super) fn apply_drop_cap_recovery_overlay(
    decoded: &mut [(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    let mut repairs = Vec::new();
    let mut claimed_targets = HashSet::new();

    for (cap_index, (cap, _)) in decoded.iter().enumerate() {
        let mut chars = cap.text.trim().chars();
        let Some(letter) = chars.next() else { continue };
        if chars.next().is_some()
            || !letter.is_alphabetic()
            || !letter.is_uppercase()
            || cap.synthetic_text_geometry
            || cap.in_footnote_zone
            || cap.below_footnote_divider
            || cap.font_ratio_page < 1.75
            || cap.font_ratio_doc < 1.55
        {
            continue;
        }

        let mut best: Option<(usize, usize, f32)> = None;
        for (target_index, (target, action)) in decoded.iter().enumerate() {
            if target_index == cap_index
                || claimed_targets.contains(&target_index)
                || target.page_index != cap.page_index
                || *action != Lm2Action::Keep
                || target.in_footnote_zone
                || target.below_footnote_divider
                || target
                    .text
                    .trim_start()
                    .chars()
                    .next()
                    .is_none_or(|ch| !ch.is_lowercase())
            {
                continue;
            }
            let horizontal_gap = target.left - cap.right;
            let max_gap = target.page_width.max(cap.page_width).max(1.0) * 0.012;
            if horizontal_gap < -1.5 || horizontal_gap > max_gap {
                continue;
            }
            let overlap = target.top.min(cap.top) - target.bottom.max(cap.bottom);
            let target_height = (target.top - target.bottom).abs().max(1.0);
            if overlap < target_height * 0.55 || cap.font_height < target.font_height * 1.65 {
                continue;
            }
            let score = horizontal_gap.abs()
                + ((target.bottom + target.top) * 0.5 - (cap.bottom + cap.top) * 0.5).abs() * 0.05;
            let candidate = (target_index, target.line_index, score);
            if best.is_none_or(|(_, best_line_index, best_score)| {
                target.line_index < best_line_index
                    || (target.line_index == best_line_index && score < best_score)
            }) {
                best = Some(candidate);
            }
        }
        if let Some((target_index, _, _)) = best {
            claimed_targets.insert(target_index);
            repairs.push((cap_index, target_index, letter));
        }
    }

    for (cap_index, target_index, letter) in &repairs {
        let target = decoded[*target_index].0.text.trim_start().to_owned();
        decoded[*target_index].0.text = format!("{letter}{target}");
        decoded[*target_index].0.role_hint = Some(LiquidBlockRole::Paragraph);
        // Retain the original source object as explicit Noise so provenance is
        // preserved without duplicating the recovered letter in Review text.
        decoded[*cap_index].1 = Lm2Action::HideNoise;
        decoded[*cap_index].0.role_hint = Some(LiquidBlockRole::Noise);
    }
    repairs.len()
}

/// A mixed-font body line can arrive as adjacent PDF text objects. If the
/// first object is italic and short, the local classifier may style it as a
/// heading even though the next object continues on the very same baseline.
pub(super) fn apply_same_row_body_fragment_overlay(
    decoded: &mut [(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    let mut repairs = Vec::new();
    for index in 0..decoded.len().saturating_sub(1) {
        let (line, action) = &decoded[index];
        let (next, next_action) = &decoded[index + 1];
        if *action != Lm2Action::Keep
            || *next_action != Lm2Action::Keep
            || line.centered
            || uppercase_ratio(&line.text) >= 0.72
            || !same_visual_baseline_fragment(line, next)
            || !matches!(
                line.text.trim_end().chars().last(),
                Some(',') | Some(';') | Some(':')
            )
            || !leading_inline_marker_then_lowercase(&next.text)
        {
            continue;
        }
        repairs.push(index);
    }
    for index in &repairs {
        decoded[*index].0.role_hint = Some(LiquidBlockRole::Paragraph);
    }
    repairs.len()
}

/// Recover body callouts that PDF extraction left as plain digits. Matching a
/// plausible definition on the same page makes these otherwise ambiguous
/// fragments safe to reinterpret, including when the classifier called the
/// tiny fragment Noise or Marginalia.
pub(super) fn apply_same_page_body_callout_overlay(
    decoded: &mut [(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    let sequence_repairs = apply_decoded_page_sequence_callout_recovery(decoded);
    let mut definition_markers: BTreeMap<usize, HashSet<u16>> = BTreeMap::new();
    for item @ (line, _) in decoded.iter() {
        let marker = numeric_note_head_candidate(item).or_else(|| {
            (line.in_footnote_zone || line.below_footnote_divider)
                .then(|| leading_numeric_token_marker(&line.text))
                .flatten()
        });
        if let Some(marker) = marker {
            definition_markers
                .entry(line.page_index)
                .or_default()
                .insert(marker);
        }
    }

    let mut standalone_repairs = Vec::new();
    for index in 1..decoded.len() {
        let (marker_line, _) = &decoded[index];
        let Some(marker) = marker_line.text.trim().parse::<u16>().ok() else {
            continue;
        };
        if !(1..=LM2_MAX_NOTE_MARKER).contains(&marker)
            || marker_line.in_footnote_zone
            || marker_line.below_footnote_divider
            || marker_line.doc_footnote_state
            || !definition_markers
                .get(&marker_line.page_index)
                .is_some_and(|markers| markers.contains(&marker))
        {
            continue;
        }
        let (previous, _) = &decoded[index - 1];
        if !previous.in_footnote_zone
            && !previous.below_footnote_divider
            && !previous.doc_footnote_state
            && lm2_marker_can_attach_to_previous_line(marker_line, previous)
        {
            standalone_repairs.push((index - 1, index, marker));
        }
    }

    let mut repaired = sequence_repairs;
    for (previous_index, marker_index, marker) in standalone_repairs {
        if previous_index >= 2 {
            let continuation_index = previous_index - 1;
            let lead_index = previous_index - 2;
            let (lead, lead_action) = &decoded[lead_index];
            let (continuation, continuation_action) = &decoded[continuation_index];
            if *lead_action == Lm2Action::Keep
                && *continuation_action != Lm2Action::Keep
                && lead.page_index == continuation.page_index
                && continuation.page_index == decoded[previous_index].0.page_index
                && continuation.line_index == lead.line_index + 1
                && !continuation.in_footnote_zone
                && !continuation.below_footnote_divider
                && should_join_dehyphenated(&lead.text, &continuation.text)
            {
                decoded[continuation_index].1 = Lm2Action::Keep;
                decoded[continuation_index].0.role_hint = Some(LiquidBlockRole::Paragraph);
                repaired += 1;
            }
        }
        decoded[previous_index].0.text = format!(
            "{}{}{}{}",
            decoded[previous_index].0.text.trim_end(),
            CALLOUT_START,
            marker,
            CALLOUT_END
        );
        decoded[previous_index].1 = Lm2Action::Keep;
        decoded[previous_index].0.role_hint = Some(LiquidBlockRole::Paragraph);
        decoded[marker_index].1 = Lm2Action::HideNoise;
        decoded[marker_index].0.role_hint = Some(LiquidBlockRole::Noise);
        repaired += 1;
    }

    for (line, action) in decoded.iter_mut() {
        if *action != Lm2Action::Keep
            || line.in_footnote_zone
            || line.below_footnote_divider
            || line.doc_footnote_state
        {
            continue;
        }
        let Some(page_markers) = definition_markers.get(&line.page_index) else {
            continue;
        };
        if let Some((marker, digit_len)) = leading_numeric_token_marker_with_len(&line.text)
            && page_markers.contains(&marker)
        {
            let trimmed = line.text.trim_start();
            let remainder = trimmed[digit_len..].trim_start();
            if remainder.chars().next().is_some_and(char::is_lowercase) {
                line.text = format!("{CALLOUT_START}{marker}{CALLOUT_END} {remainder}");
                line.role_hint = Some(LiquidBlockRole::Paragraph);
                repaired += 1;
                continue;
            }
        }
        if let Some((marker, span)) = attached_terminal_ascii_marker_span(&line.text)
            && page_markers.contains(&marker)
            && let Some(rewritten) = rewrite_terminal_marker_as_callout(&line.text, marker, &span)
        {
            line.text = rewritten;
            repaired += 1;
        }
    }
    repaired
}

/// Bridge older law-review PDFs whose embedded font draws a superscript note
/// number but maps that glyph to an unrelated character. These files can
/// still expose two independent, exact page-local signals:
///
/// * body text contains one unique consecutive run of attached callout digits;
/// * footnote first rows form an equally long, tightly aligned indent column.
///
/// The bridge is deliberately all-or-nothing. It rejects synthetic OCR,
/// readable-but-conflicting note numbers, ordinary indented quotations, and
/// any ambiguous body run. It only assigns markers to physical source rows;
/// `apply_decoded_page_sequence_callout_recovery` still has to find every
/// corresponding body occurrence before it rewrites any text.
pub(super) fn apply_scanned_glyph_note_sequence_bridge(
    document_path: &Path,
    decoded: &mut [(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    let mut indices_by_page = BTreeMap::<usize, Vec<usize>>::new();
    for (index, (line, _)) in decoded.iter().enumerate() {
        indices_by_page
            .entry(line.page_index)
            .or_default()
            .push(index);
    }

    let trace_path = std::env::var_os("LAWPDF_LM2_TRACE_SCANNED_GLYPH_FILE").map(PathBuf::from);
    let mut trace_rows = Vec::new();
    let mut assignments = Vec::<(usize, u16, bool)>::new();
    for page_indices in indices_by_page.values_mut() {
        page_indices.sort_by_key(|index| decoded[*index].0.line_index);
        let body_indices = page_indices
            .iter()
            .copied()
            .filter(|index| {
                let line = &decoded[*index].0;
                !line.in_footnote_zone
                    && !line.below_footnote_divider
                    && !line.doc_footnote_state
                    && !line.doc_repeated_edge_text
            })
            .collect::<Vec<_>>();
        let Some(markers) = scanned_glyph_body_marker_sequence(decoded, &body_indices) else {
            continue;
        };
        let Some(head_indices) =
            scanned_glyph_note_head_rows(decoded, page_indices, markers.as_slice())
        else {
            continue;
        };
        if trace_path.is_some() {
            trace_rows.push(serde_json::json!({
                "document": document_path.display().to_string(),
                "page_index": decoded[head_indices[0]].0.page_index,
                "markers": markers,
                "heads": head_indices.iter().map(|index| serde_json::json!({
                    "id": decoded[*index].0.id,
                    "line_index": decoded[*index].0.line_index,
                    "left": decoded[*index].0.left,
                    "text": decoded[*index].0.text,
                })).collect::<Vec<_>>(),
            }));
        }
        assignments.extend(
            head_indices
                .into_iter()
                .zip(markers)
                .enumerate()
                .map(|(position, (index, marker))| (index, marker, position == 0)),
        );
    }

    for (index, marker, first_on_page) in &assignments {
        let (line, action) = &mut decoded[*index];
        if leading_sentineled_marker(&line.text) != Some(*marker) {
            line.text = format!(
                "{CALLOUT_START}{marker}{CALLOUT_END} {}",
                line.text.trim_start()
            );
        }
        line.doc_note_marker = *marker;
        line.doc_note_marker_first_on_page = *first_on_page;
        line.doc_note_marker_mid_sequence_page = *marker > 1;
        line.doc_note_marker_follows_previous_page = false;
        line.doc_note_marker_page_delta = 0;
        line.role_hint = Some(LiquidBlockRole::Marginalia);
        *action = Lm2Action::Marginalia;
    }
    let mut marker_rows_by_page = BTreeMap::<usize, Vec<(usize, u16)>>::new();
    for (index, marker, _) in &assignments {
        marker_rows_by_page
            .entry(decoded[*index].0.page_index)
            .or_default()
            .push((decoded[*index].0.line_index, *marker));
    }
    apply_decoded_page_sequence_callout_recovery_from_rows(decoded, marker_rows_by_page);
    if let Some(path) = trace_path
        && !trace_rows.is_empty()
        && let Ok(file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
    {
        use std::io::Write as _;
        let mut writer = std::io::BufWriter::new(file);
        for row in trace_rows {
            if let Ok(encoded) = serde_json::to_string(&row) {
                let _ = writeln!(writer, "{encoded}");
            }
        }
        let _ = writer.flush();
    }
    assignments.len()
}

pub(super) fn scanned_glyph_body_marker_sequence(
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
    body_indices: &[usize],
) -> Option<Vec<u16>> {
    let mut candidates = body_indices
        .iter()
        .flat_map(|index| {
            scanned_glyph_body_marker_candidates(&decoded[*index].0.text)
                .into_iter()
                .map(move |(byte_index, marker)| (decoded[*index].0.line_index, byte_index, marker))
        })
        .collect::<Vec<_>>();
    candidates.sort_unstable();
    candidates.dedup();

    let mut runs = Vec::<(usize, usize)>::new();
    let mut start = 0usize;
    while start < candidates.len() {
        let mut end = start + 1;
        while end < candidates.len() && candidates[end].2 == candidates[end - 1].2.saturating_add(1)
        {
            end += 1;
        }
        runs.push((start, end));
        start = end;
    }
    let longest = runs
        .iter()
        .map(|(start, end)| end - start)
        .max()
        .unwrap_or(0);
    if longest < 4
        || runs
            .iter()
            .filter(|(start, end)| end - start == longest)
            .count()
            != 1
    {
        return None;
    }
    let (start, end) = runs
        .into_iter()
        .find(|(start, end)| end - start == longest)?;
    let markers = candidates[start..end]
        .iter()
        .map(|(_, _, marker)| *marker)
        .collect::<Vec<_>>();
    let counts = candidates.iter().fold(
        HashMap::<u16, usize>::new(),
        |mut counts, (_, _, marker)| {
            *counts.entry(*marker).or_default() += 1;
            counts
        },
    );
    markers
        .iter()
        .all(|marker| counts.get(marker) == Some(&1))
        .then_some(markers)
}

pub(super) fn scanned_glyph_body_marker_candidates(text: &str) -> Vec<(usize, u16)> {
    let chars = text.char_indices().collect::<Vec<_>>();
    let mut candidates = Vec::new();
    let mut position = 0usize;
    while position < chars.len() {
        if chars[position].1 == CALLOUT_START {
            let start = chars[position].0;
            let mut cursor = position + 1;
            let mut digits = String::new();
            while cursor < chars.len() && chars[cursor].1.is_ascii_digit() && digits.len() < 3 {
                digits.push(chars[cursor].1);
                cursor += 1;
            }
            if chars.get(cursor).is_some_and(|(_, ch)| *ch == CALLOUT_END)
                && let Ok(marker) = digits.parse::<u16>()
                && (1..=LM2_MAX_NOTE_MARKER).contains(&marker)
            {
                candidates.push((start, marker));
                position = cursor + 1;
                continue;
            }
        }
        if !chars[position].1.is_ascii_digit()
            || position > 0 && chars[position - 1].1.is_ascii_digit()
        {
            position += 1;
            continue;
        }
        let start_position = position;
        let start = chars[position].0;
        while position < chars.len() && chars[position].1.is_ascii_digit() {
            position += 1;
        }
        if position - start_position > 3 {
            continue;
        }
        let end = chars
            .get(position)
            .map(|(offset, _)| *offset)
            .unwrap_or(text.len());
        let before = start_position
            .checked_sub(1)
            .and_then(|index| chars.get(index))
            .map(|(_, ch)| *ch);
        let after = chars.get(position).map(|(_, ch)| *ch);
        if before.is_some_and(|ch| matches!(ch, '\u{00A7}' | '-' | '*' | CALLOUT_START))
            || after.is_some_and(|ch| {
                !ch.is_whitespace()
                    && !matches!(ch, '"' | '\'' | '\u{2019}' | '\u{201D}' | CALLOUT_END)
            })
        {
            continue;
        }
        let left = text[..start].trim_end();
        let attached = before.is_some_and(|ch| !ch.is_whitespace() && !ch.is_ascii_digit());
        let spaced_after_punctuation = before.is_some_and(char::is_whitespace)
            && left
                .chars()
                .next_back()
                .is_some_and(|ch| matches!(ch, '.' | '!' | '?' | ',' | ':' | ';' | ')' | ']'));
        if !(attached || spaced_after_punctuation) || scanned_glyph_abbreviation_before_marker(left)
        {
            continue;
        }
        if let Ok(marker) = text[start..end].parse::<u16>()
            && (1..=LM2_MAX_NOTE_MARKER).contains(&marker)
        {
            candidates.push((start, marker));
        }
    }
    candidates
}

pub(super) fn scanned_glyph_abbreviation_before_marker(left: &str) -> bool {
    let Some(stem) = left.strip_suffix('.') else {
        return false;
    };
    let token = stem
        .split(|ch: char| !ch.is_ascii_alphabetic())
        .next_back()
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(
        token.as_str(),
        "p" | "pp" | "no" | "nos" | "vol" | "sec" | "art" | "cl" | "fig"
    )
}

pub(super) fn scanned_glyph_note_head_rows(
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
    page_indices: &[usize],
    markers: &[u16],
) -> Option<Vec<usize>> {
    let footnote_indices = page_indices
        .iter()
        .copied()
        .filter(|index| {
            let line = &decoded[*index].0;
            (line.in_footnote_zone || line.below_footnote_divider || line.doc_footnote_state)
                && !line.synthetic_text_geometry
                && !line.doc_repeated_edge_text
                && !line.in_ruled_cell
                && !line.ruled_row_membership_exact
                && !line.page_table_column_like
        })
        .collect::<Vec<_>>();
    if footnote_indices.len() < markers.len() || markers.len() < 4 {
        return None;
    }
    let page_width = footnote_indices
        .iter()
        .map(|index| decoded[*index].0.page_width)
        .fold(1.0f32, f32::max);
    let mut lefts = footnote_indices
        .iter()
        .map(|index| decoded[*index].0.left)
        .collect::<Vec<_>>();
    lefts.sort_by(f32::total_cmp);
    lefts.dedup_by(|left, right| (*left - *right).abs() < 0.25);
    let (gap, split) = lefts
        .windows(2)
        .map(|pair| (pair[1] - pair[0], (pair[0] + pair[1]) * 0.5))
        .max_by(|left, right| left.0.total_cmp(&right.0))?;
    if gap < (page_width * 0.009).max(4.0) || gap > page_width * 0.05 {
        return None;
    }
    let mut heads = footnote_indices
        .into_iter()
        .filter(|index| decoded[*index].0.left > split)
        .collect::<Vec<_>>();
    heads.sort_by_key(|index| decoded[*index].0.line_index);
    if heads.len() == markers.len() + 1
        && markers.first() == Some(&1)
        && heads
            .first()
            .is_some_and(|index| scanned_glyph_author_note_candidate(&decoded[*index].0.text))
    {
        heads.remove(0);
    }
    if heads.len() != markers.len() {
        return None;
    }
    let (min_left, max_left) = heads
        .iter()
        .map(|index| decoded[*index].0.left)
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(min, max), left| {
            (min.min(left), max.max(left))
        });
    if max_left - min_left > (page_width * 0.008).max(2.5)
        || heads.iter().any(|index| {
            let line = &decoded[*index].0;
            line.font_ratio_page_ref > 0.93 || !line.segment_block_footnote_like
        })
        || heads
            .iter()
            .filter(|index| decoded[**index].0.segment_block_first)
            .count()
            * 2
            < heads.len()
    {
        return None;
    }
    let parsed = heads
        .iter()
        .map(|index| leading_numeric_token_marker(&decoded[*index].0.text))
        .collect::<Vec<_>>();
    if parsed
        .iter()
        .zip(markers)
        .any(|(parsed, marker)| parsed.is_some_and(|parsed| parsed != *marker))
        || parsed.iter().filter(|marker| marker.is_some()).count() * 2 >= markers.len()
    {
        return None;
    }
    Some(heads)
}

pub(super) fn scanned_glyph_author_note_candidate(text: &str) -> bool {
    let lower = normalize_text(text);
    [
        "b.a.",
        "j.d.",
        "ll.m.",
        "s.j.d.",
        "professor of law",
        "university",
    ]
    .iter()
    .any(|cue| lower.contains(cue))
}

/// Recover flattened law-review superscripts before the permissive local
/// callout overlay sees them as one-digit notes. Performing the guarded page
/// sequence alignment on source lines preserves the corrected marker through
/// block assembly and prevents `26` rendered as `2 6` from becoming two links.
pub(super) fn apply_decoded_page_sequence_callout_recovery(
    decoded: &mut [(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    let mut marker_rows_by_page = BTreeMap::<usize, Vec<(usize, u16)>>::new();
    for item @ (line, _) in decoded.iter() {
        let Some(primary) = numeric_note_head_candidate(item).or_else(|| {
            (line.in_footnote_zone || line.below_footnote_divider)
                .then(|| leading_numeric_token_marker(&line.text))
                .flatten()
        }) else {
            continue;
        };
        let rows = marker_rows_by_page.entry(line.page_index).or_default();
        rows.push((line.line_index, primary));
        for marker in embedded_numeric_note_head_markers(&line.text) {
            rows.push((line.line_index, marker));
        }
    }

    apply_decoded_page_sequence_callout_recovery_from_rows(decoded, marker_rows_by_page)
}

pub(super) fn apply_decoded_page_sequence_callout_recovery_from_rows(
    decoded: &mut [(DeepLiquidSourceLine, Lm2Action)],
    marker_rows_by_page: BTreeMap<usize, Vec<(usize, u16)>>,
) -> usize {
    let mut body_indices_by_page = BTreeMap::<usize, Vec<usize>>::new();
    for (index, (line, _)) in decoded.iter().enumerate() {
        if !line.in_footnote_zone
            && !line.below_footnote_divider
            && !line.doc_footnote_state
            && !line.doc_repeated_edge_text
        {
            body_indices_by_page
                .entry(line.page_index)
                .or_default()
                .push(index);
        }
    }
    for indices in body_indices_by_page.values_mut() {
        indices.sort_by_key(|index| decoded[*index].0.line_index);
    }

    let mut repaired = 0usize;
    for (page_index, mut marker_rows) in marker_rows_by_page {
        marker_rows.sort_unstable();
        marker_rows.dedup();
        let candidate_count = marker_rows.len();
        let markers = strongest_local_marker_sequence(&marker_rows);
        if markers.is_empty()
            || markers.len() == 1 && candidate_count != 1
            || markers.len() == 1 && markers[0] < 10
        {
            continue;
        }
        let Some(body_indices) = body_indices_by_page.get(&page_index) else {
            continue;
        };

        let mut body_position = 0usize;
        let mut byte_position = 0usize;
        let mut replacements = BTreeMap::<usize, Vec<(usize, usize, u16)>>::new();
        let mut cleanup_indices = Vec::<usize>::new();
        let mut complete = true;
        for marker in markers {
            let mut found: Option<(usize, usize, usize, usize, PageSequenceMatchKind)> = None;
            for matched_position in body_position..body_indices.len() {
                let index = body_indices[matched_position];
                let text = &decoded[index].0.text;
                let from = if matched_position == body_position {
                    byte_position
                } else {
                    0
                };
                if let Some((start, end, kind)) = page_sequence_callout_range(text, marker, from) {
                    if candidate_count == 1 && !kind.singleton_safe() {
                        continue;
                    }
                    let candidate = (matched_position, index, start, end, kind);
                    if found.as_ref().is_none_or(
                        |(best_position, best_index, best_start, best_end, best_kind)| {
                            (
                                sequence_selection_priority(kind, text, start, end),
                                matched_position,
                                start,
                            ) < (
                                sequence_selection_priority(
                                    *best_kind,
                                    &decoded[*best_index].0.text,
                                    *best_start,
                                    *best_end,
                                ),
                                *best_position,
                                *best_start,
                            )
                        },
                    ) {
                        found = Some(candidate);
                    }
                }
            }
            let Some((matched_position, index, start, end, kind)) = found else {
                complete = false;
                break;
            };
            body_position = matched_position;
            if kind == PageSequenceMatchKind::InferredQuoteBoundary {
                body_position += 1;
                byte_position = 0;
            } else {
                byte_position = end;
            }
            if kind.replaces_text() {
                if kind == PageSequenceMatchKind::PartialDigit
                    && let Some(next_index) = body_indices.get(matched_position + 1).copied()
                    && decoded[next_index].0.line_index
                        == decoded[index].0.line_index.saturating_add(1)
                    && split_callout_fragment_completes_marker(
                        &decoded[index].0.text[start..end],
                        &decoded[next_index].0.text,
                        marker,
                    )
                {
                    cleanup_indices.push(next_index);
                }
                replacements
                    .entry(index)
                    .or_default()
                    .push((start, end, marker));
            }
        }
        if !complete || replacements.is_empty() {
            continue;
        }
        for (index, mut ranges) in replacements {
            ranges.sort_by_key(|(start, _, _)| *start);
            for (start, end, marker) in ranges.into_iter().rev() {
                decoded[index]
                    .0
                    .text
                    .replace_range(start..end, &format!("{CALLOUT_START}{marker}{CALLOUT_END}"));
                repaired += 1;
            }
            decoded[index].1 = Lm2Action::Keep;
            decoded[index].0.role_hint = Some(LiquidBlockRole::Paragraph);
        }
        cleanup_indices.sort_unstable();
        cleanup_indices.dedup();
        for index in cleanup_indices {
            decoded[index].0.text.clear();
            decoded[index].1 = Lm2Action::HideNoise;
            decoded[index].0.role_hint = Some(LiquidBlockRole::Noise);
        }
    }
    repaired
}

pub(super) fn strongest_local_marker_sequence(rows: &[(usize, u16)]) -> Vec<u16> {
    if rows.is_empty() {
        return Vec::new();
    }
    let mut best = vec![1usize; rows.len()];
    let mut previous = vec![usize::MAX; rows.len()];
    let mut end = 0usize;
    for i in 0..rows.len() {
        for j in 0..i {
            let delta = rows[i].1.saturating_sub(rows[j].1);
            if (1..=3).contains(&delta) && best[j] + 1 > best[i] {
                best[i] = best[j] + 1;
                previous[i] = j;
            }
        }
        if best[i] > best[end] {
            end = i;
        }
    }
    let mut positions = Vec::new();
    let mut cursor = end;
    while cursor != usize::MAX {
        positions.push(cursor);
        cursor = previous[cursor];
    }
    positions.reverse();
    positions
        .into_iter()
        .map(|position| rows[position].1)
        .collect()
}

pub(super) fn same_visual_baseline_fragment(
    previous: &DeepLiquidSourceLine,
    line: &DeepLiquidSourceLine,
) -> bool {
    if previous.page_index != line.page_index || line.line_index != previous.line_index + 1 {
        return false;
    }
    let page_height = line.page_height.max(previous.page_height).max(1.0);
    if (line.bottom - previous.bottom).abs() / page_height > 0.003 {
        return false;
    }
    let overlap = line.top.min(previous.top) - line.bottom.max(previous.bottom);
    let shorter_height = (line.top - line.bottom)
        .abs()
        .min((previous.top - previous.bottom).abs())
        .max(1.0);
    if overlap < shorter_height * 0.60 {
        return false;
    }
    let page_width = line.page_width.max(previous.page_width).max(1.0);
    let horizontal_gap = line.left - previous.right;
    horizontal_gap >= -2.0 && horizontal_gap <= page_width * 0.04
}

pub(super) fn leading_inline_marker_then_lowercase(text: &str) -> bool {
    let trimmed = text.trim_start_matches(|ch: char| {
        ch.is_whitespace() || matches!(ch, CALLOUT_START | CALLOUT_END)
    });
    let digits = trimmed.chars().take_while(|ch| ch.is_ascii_digit()).count();
    if digits == 0 || digits > 3 {
        return false;
    }
    trimmed[digits..]
        .trim_start_matches(|ch: char| {
            ch.is_whitespace() || matches!(ch, CALLOUT_START | CALLOUT_END)
        })
        .chars()
        .next()
        .is_some_and(|ch| ch.is_lowercase())
}
