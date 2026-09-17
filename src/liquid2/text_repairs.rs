//! Hyphenation and spacing repairs, paragraph boundaries, and note-marker parsers.

use super::*;

/// Repair PDFium's U+0002 line-end-hyphen glyph when extraction has folded two
/// visual rows into one source line. Repeated intact spellings elsewhere in the
/// document are authoritative; the existing conservative compound heuristic is
/// only a fallback when the document supplies no unambiguous evidence.
pub(super) fn apply_document_discretionary_hyphen_repairs(
    decoded: &mut [(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    let token_evidence = decoded
        .iter()
        .flat_map(|(line, _)| line.text.split_whitespace())
        .filter(|token| !token.contains('\u{0002}'))
        .filter_map(lm2_hyphen_evidence_token)
        .collect::<HashSet<_>>();
    let mut repaired = 0usize;
    for (line, _) in decoded {
        let (text, count) = repair_internal_discretionary_hyphens(&line.text, &token_evidence);
        if count > 0 {
            line.text = text;
            repaired += count;
        }
    }
    repaired
}

pub(super) fn lm2_ascii_alpha_core(token: &str) -> Option<(usize, usize, &str)> {
    let start = token
        .char_indices()
        .find_map(|(index, ch)| ch.is_ascii_alphabetic().then_some(index))?;
    let end = token
        .char_indices()
        .rev()
        .find_map(|(index, ch)| ch.is_ascii_alphabetic().then_some(index + ch.len_utf8()))?;
    let core = &token[start..end];
    core.chars()
        .all(|ch| ch.is_ascii_alphabetic())
        .then_some((start, end, core))
}

/// Repair a fused token only when the same document repeatedly supplies the
/// exact two-word spelling. This avoids dictionary guesses while fixing font-
/// span joins such as `Modelclauses` when `Model clauses` occurs throughout the
/// article. Small-font citation rows also get two narrow typography repairs:
/// `SeeSurname` and a missing space after a single-letter initial.
pub(super) fn apply_document_fused_word_spacing_repairs(
    decoded: &mut [(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    let mut token_counts = HashMap::<String, usize>::new();
    let mut bigram_splits = HashMap::<String, HashMap<usize, usize>>::new();
    for (line, _) in decoded.iter() {
        let words = line
            .text
            .split_whitespace()
            .filter_map(|token| lm2_ascii_alpha_core(token).map(|(_, _, core)| core))
            .filter(|core| core.len() >= 2)
            .map(str::to_ascii_lowercase)
            .collect::<Vec<_>>();
        for word in &words {
            *token_counts.entry(word.clone()).or_default() += 1;
        }
        for pair in words.windows(2) {
            if pair[0].len() < 3 || pair[1].len() < 3 {
                continue;
            }
            let key = format!("{}{}", pair[0], pair[1]);
            *bigram_splits
                .entry(key)
                .or_default()
                .entry(pair[0].len())
                .or_default() += 1;
        }
    }

    let mut repaired = 0usize;
    for (line, _) in decoded {
        let note_like =
            line.in_footnote_zone || line.below_footnote_divider || line.font_ratio_doc <= 0.88;
        let mut tokens = line
            .text
            .split_whitespace()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        for token in &mut tokens {
            let Some((start, end, core)) = lm2_ascii_alpha_core(token) else {
                continue;
            };
            let lower = core.to_ascii_lowercase();
            let evidence_split = (token_counts.get(&lower).copied().unwrap_or_default() == 1)
                .then(|| bigram_splits.get(&lower))
                .flatten()
                .and_then(|splits| {
                    let supported = splits
                        .iter()
                        .filter_map(|(split, count)| (*count >= 2).then_some(*split))
                        .collect::<Vec<_>>();
                    (supported.len() == 1).then(|| supported[0])
                });
            let see_split = note_like
                && core.len() > 3
                && core[..3].eq_ignore_ascii_case("see")
                && core.as_bytes()[3].is_ascii_uppercase();
            let split = evidence_split.or_else(|| see_split.then_some(3));
            if let Some(split) = split
                && split < core.len()
            {
                token.insert(start + split, ' ');
                repaired += 1;
                let _ = end;
            }
        }

        if note_like {
            for index in 0..tokens.len() {
                let next_is_of = tokens
                    .get(index + 1)
                    .and_then(|token| lm2_ascii_alpha_core(token))
                    .is_some_and(|(_, _, core)| core.eq_ignore_ascii_case("of"));
                let Some((start, _, core)) = lm2_ascii_alpha_core(&tokens[index]) else {
                    continue;
                };
                if next_is_of
                    && core.len() >= 5
                    && core.starts_with('A')
                    && core.chars().all(|ch| ch.is_ascii_uppercase())
                {
                    tokens[index].insert(start + 1, ' ');
                    repaired += 1;
                }
            }
        }

        let mut text = tokens.join(" ");
        if note_like {
            let chars = text.char_indices().collect::<Vec<_>>();
            let mut insertions = Vec::new();
            for window in chars.windows(3) {
                let (before_index, before) = window[0];
                let (punct_index, punct) = window[1];
                let (_, after) = window[2];
                if punct == ',' && before.is_alphabetic() && after.is_ascii_uppercase() {
                    insertions.push(punct_index + punct.len_utf8());
                    continue;
                }
                if punct != '.' || !before.is_ascii_uppercase() || !after.is_ascii_uppercase() {
                    continue;
                }
                let before_is_initial = text[..before_index]
                    .chars()
                    .next_back()
                    .is_none_or(|ch| !ch.is_alphabetic());
                let uppercase_run = text[punct_index + 1..]
                    .chars()
                    .take_while(|ch| ch.is_ascii_uppercase())
                    .count();
                if before_is_initial && uppercase_run >= 3 {
                    insertions.push(punct_index + 1);
                }
            }
            for index in insertions.into_iter().rev() {
                text.insert(index, ' ');
                repaired += 1;
            }
            let mut spaced_tokens = text
                .split_whitespace()
                .map(str::to_owned)
                .collect::<Vec<_>>();
            for index in 0..spaced_tokens.len() {
                let next_is_of = spaced_tokens
                    .get(index + 1)
                    .and_then(|token| lm2_ascii_alpha_core(token))
                    .is_some_and(|(_, _, core)| core.eq_ignore_ascii_case("of"));
                let Some((start, _, core)) = lm2_ascii_alpha_core(&spaced_tokens[index]) else {
                    continue;
                };
                if next_is_of
                    && core.len() >= 5
                    && core.starts_with('A')
                    && core.chars().all(|ch| ch.is_ascii_uppercase())
                {
                    spaced_tokens[index].insert(start + 1, ' ');
                    repaired += 1;
                }
            }
            text = spaced_tokens.join(" ");
        }
        line.text = text;
    }
    repaired
}

pub(super) fn lm2_hyphen_evidence_token(token: &str) -> Option<String> {
    let token = token.trim_matches(|ch: char| !ch.is_ascii_alphabetic() && ch != '-');
    let token = token.trim_matches('-');
    (!token.is_empty() && token.chars().any(|ch| ch.is_ascii_alphabetic()))
        .then(|| token.to_ascii_lowercase())
}

pub(super) fn repair_internal_discretionary_hyphens(
    text: &str,
    token_evidence: &HashSet<String>,
) -> (String, usize) {
    if !text.contains('\u{0002}') {
        return (text.to_owned(), 0);
    }
    let mut output = String::with_capacity(text.len());
    let mut remainder = text;
    let mut repaired = 0usize;
    while let Some(marker_index) = remainder.find('\u{0002}') {
        output.push_str(&remainder[..marker_index]);
        let after_marker = &remainder[marker_index + '\u{0002}'.len_utf8()..];
        let Some(left) = terminal_hyphen_component(&output) else {
            output.push('\u{0002}');
            remainder = after_marker;
            continue;
        };
        let Some(right) = leading_hyphen_component(after_marker) else {
            // Preserve a terminal marker so `append_line` can make the
            // cross-source-line decision with the following physical row.
            output.push('\u{0002}');
            remainder = after_marker;
            continue;
        };
        if !right
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_lowercase())
        {
            output.push('\u{0002}');
            remainder = after_marker;
            continue;
        }

        let joined = format!("{left}{right}").to_ascii_lowercase();
        let hyphenated = format!("{left}-{right}").to_ascii_lowercase();
        let joined_seen = token_evidence.contains(&joined);
        let hyphenated_seen = token_evidence.contains(&hyphenated);
        let preserve = match (joined_seen, hyphenated_seen) {
            (true, false) => false,
            (false, true) => true,
            // Pass the complete continuation so contextual compounds (for
            // example `abortion-rights group`) can inspect the following word.
            _ => should_preserve_terminal_hyphen(&format!("{left}-"), after_marker),
        };
        if preserve {
            output.push('-');
        }
        repaired += 1;
        remainder = after_marker;
    }
    output.push_str(remainder);
    (output, repaired)
}

pub(super) fn terminal_hyphen_component(text: &str) -> Option<&str> {
    let trimmed = text.trim_end();
    let start = trimmed
        .char_indices()
        .rev()
        .find_map(|(index, ch)| {
            (!ch.is_ascii_alphabetic() && ch != '-').then_some(index + ch.len_utf8())
        })
        .unwrap_or(0);
    let component = trimmed[start..].trim_matches('-');
    (!component.is_empty()).then_some(component)
}

pub(super) fn leading_hyphen_component(text: &str) -> Option<&str> {
    let trimmed = text.trim_start();
    let end = trimmed
        .char_indices()
        .find_map(|(index, ch)| (!ch.is_ascii_alphabetic() && ch != '-').then_some(index))
        .unwrap_or(trimmed.len());
    let component = trimmed[..end].trim_matches('-');
    (!component.is_empty()).then_some(component)
}

pub(super) fn clean_lm2_line_text(line: &str) -> String {
    collapse_whitespace(&line.replace('\u{0002}', "-"))
}

pub(super) fn should_join_dehyphenated(existing: &str, next: &str) -> bool {
    existing.trim_end().ends_with('-')
        && next
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_lowercase())
        && !should_preserve_terminal_hyphen(existing, next)
}

pub(super) fn should_join_preserved_hyphen(existing: &str, next: &str) -> bool {
    existing.trim_end().ends_with('-') && should_preserve_terminal_hyphen(existing, next)
}

pub(super) fn paragraph_boundary(
    previous: &DeepLiquidSourceLine,
    line: &DeepLiquidSourceLine,
) -> bool {
    if previous.page_index != line.page_index {
        let previous_sentence_end =
            lm2_blocksplit_ends_like_paragraph(&strip_callout_sentinels_lm2(&previous.text));
        let indent = (line.left - previous.left) / line.page_width.max(1.0);
        return previous_sentence_end && indent > 0.025;
    }
    if same_visual_row_fragment(previous, line) {
        return false;
    }
    if lm2_tall_row_paragraph_start(previous, line) {
        return true;
    }
    let gap = vertical_gap(previous, line);
    let page_width = line.page_width.max(1.0);
    let visual_line_start = if line.first_visual_left > 0.0 {
        line.first_visual_left
    } else {
        line.left
    };
    let left_delta = ((visual_line_start - previous.left).abs() / page_width).max(0.0);
    // Signed, so a line that starts further right than its predecessor counts
    // as a first-line indent while a line returning to the body margin (the
    // line after a block quote) does not.
    let first_line_indent = (visual_line_start - previous.left) / page_width;
    let previous_sentence_end = previous
        .text
        .trim_end()
        .chars()
        .last()
        .is_some_and(|ch| matches!(ch, '.' | '?' | '!' | '"' | '\'' | ')'));
    // A paragraph's last line usually ends on a footnote marker, which leaves
    // a callout sentinel or a bare digit as the final character, so the plain
    // punctuation test above misses precisely the lines that end paragraphs.
    let previous_ends_paragraph = previous_sentence_end
        || lm2_blocksplit_ends_like_paragraph(&strip_callout_sentinels_lm2(&previous.text));
    gap > 0.030
        || (previous_sentence_end && (gap > 0.016 || left_delta > 0.055))
        || (previous_ends_paragraph && first_line_indent > PARAGRAPH_FIRST_LINE_INDENT)
}

pub(super) fn strip_callout_sentinels_lm2(text: &str) -> String {
    text.chars()
        .filter(|ch| !matches!(*ch, CALLOUT_START | CALLOUT_END))
        .collect()
}

/// Minimum signed first-line indent, as a fraction of page width, that marks a
/// new paragraph on the same page.
///
/// Law reviews mark paragraphs with a first-line indent and no extra leading.
/// Measured across ten articles the indent runs 0.015–0.059 of page width, so
/// the previous `left_delta > 0.055` test fired on only one of them; every
/// other document ran its whole body together into a single block.
pub(super) const PARAGRAPH_FIRST_LINE_INDENT: f32 = 0.012;

pub(super) fn same_visual_row_fragment(
    previous: &DeepLiquidSourceLine,
    line: &DeepLiquidSourceLine,
) -> bool {
    if previous.page_index != line.page_index || line.line_index != previous.line_index + 1 {
        return false;
    }
    same_visual_row_geometry(previous, line)
}

pub(super) fn same_visual_row_after_standalone_callout(
    previous: &DeepLiquidSourceLine,
    line: &DeepLiquidSourceLine,
) -> bool {
    previous.page_index == line.page_index
        && line.line_index == previous.line_index.saturating_add(2)
        && same_visual_row_geometry(previous, line)
}

pub(super) fn same_visual_row_leading_callout_fragment(
    previous: &DeepLiquidSourceLine,
    line: &DeepLiquidSourceLine,
) -> bool {
    if previous.page_index != line.page_index
        || line.line_index != previous.line_index.saturating_add(1)
    {
        return false;
    }
    let page_height = line.page_height.max(previous.page_height).max(1.0);
    let shared_top = (line.top - previous.top).abs() / page_height <= 0.003;
    let shared_baseline = (line.bottom - previous.bottom).abs() / page_height <= 0.003;
    let font_ratio = previous.font_height.max(line.font_height)
        / previous.font_height.min(line.font_height).max(1.0);
    let page_width = line.page_width.max(previous.page_width).max(1.0);
    let first_left = if line.first_visual_left > 0.0 {
        line.first_visual_left
    } else {
        line.left
    };
    let last_right = if previous.last_visual_right > 0.0 {
        previous.last_visual_right
    } else {
        previous.right
    };
    let horizontal_gap = first_left - last_right;
    (shared_top || shared_baseline)
        && font_ratio <= 1.20
        && horizontal_gap >= -2.0
        && horizontal_gap <= page_width * 0.04
        && line.text.chars().any(char::is_alphabetic)
}

pub(super) fn same_visual_row_geometry(
    previous: &DeepLiquidSourceLine,
    line: &DeepLiquidSourceLine,
) -> bool {
    let page_height = line.page_height.max(previous.page_height).max(1.0);
    let baseline_delta = (line.bottom - previous.bottom).abs() / page_height;
    let top_delta = (line.top - previous.top).abs() / page_height;
    if baseline_delta > 0.003 || top_delta > 0.004 {
        return false;
    }
    let page_width = line.page_width.max(previous.page_width).max(1.0);
    let horizontal_gap = line.left - previous.right;
    horizontal_gap >= -2.0 && horizontal_gap <= page_width * 0.04
}

pub(super) fn role_for_decoded_line(
    line: &DeepLiquidSourceLine,
    action: Lm2Action,
    first_visible_block: bool,
) -> LiquidBlockRole {
    if lm2_journal_issue_masthead_text(&line.text) {
        return LiquidBlockRole::Noise;
    }
    if lm2_generic_title_label(&line.text) {
        return LiquidBlockRole::Heading;
    }
    match action {
        Lm2Action::Marginalia => LiquidBlockRole::Marginalia,
        Lm2Action::HideNoise => LiquidBlockRole::Noise,
        Lm2Action::Keep => {
            if let Some(role) = line.role_hint {
                return match role {
                    LiquidBlockRole::Title if first_visible_block && line.centered => role,
                    LiquidBlockRole::Heading
                        if word_count(&line.text) <= 14
                            && heading_text_like(&line.text)
                            && ((lm2_heading_starts_new_outline_item(&line.text)
                                && line.font_ratio_doc >= 0.84
                                && !line.in_footnote_zone
                                && !line.below_footnote_divider)
                                || (line.centered && uppercase_ratio(&line.text) >= 0.72)
                                || (line.font_ratio_page >= 0.98
                                    && (line.bold
                                        || line.centered
                                        || line.font_ratio_page > 1.10
                                            && line.font_ratio_doc > 1.08)))
                            && !line.text.trim_end().ends_with('.') =>
                    {
                        role
                    }
                    LiquidBlockRole::Subheading
                        if word_count(&line.text) <= 16
                            && line.font_ratio_page >= 0.96
                            && heading_text_like(&line.text)
                            && (line.bold || line.centered || line.font_ratio_page > 1.04)
                            && !line.text.trim_end().ends_with('.') =>
                    {
                        role
                    }
                    LiquidBlockRole::Abstract
                    | LiquidBlockRole::Syllabus
                    | LiquidBlockRole::AuthorInfo
                    | LiquidBlockRole::Lead
                    | LiquidBlockRole::Quote
                    | LiquidBlockRole::ListItem
                    | LiquidBlockRole::Clause
                    | LiquidBlockRole::Definition
                    | LiquidBlockRole::Holding
                    | LiquidBlockRole::Issue
                    | LiquidBlockRole::KeyClause => role,
                    _ => LiquidBlockRole::Paragraph,
                };
            }
            let words = word_count(&line.text);
            if first_visible_block && line.centered && words <= 18 {
                LiquidBlockRole::Title
            } else if words <= 14
                && (line.bold
                    || line.centered
                    || line.font_ratio_page > 1.12 && line.font_ratio_doc > 1.08)
                && heading_text_like(&line.text)
                && !line.text.trim_end().ends_with('.')
            {
                LiquidBlockRole::Heading
            } else {
                LiquidBlockRole::Paragraph
            }
        }
    }
}

pub(super) fn line_ref(line: &DeepLiquidSourceLine, role: LiquidBlockRole) -> LiquidSourceLineRef {
    LiquidSourceLineRef {
        id: Some(line.id.clone()),
        page_index: line.page_index,
        line_index: line.line_index,
        text: line.text.clone(),
        role,
        note_markers: source_line_note_markers(line, role),
    }
}

pub(super) fn line_ref_with_note_start(
    line: &DeepLiquidSourceLine,
    role: LiquidBlockRole,
    force_note_start: bool,
) -> LiquidSourceLineRef {
    let mut reference = line_ref(line, role);
    if force_note_start
        && matches!(
            role,
            LiquidBlockRole::Footnote | LiquidBlockRole::Marginalia
        )
        && let Some(marker) = leading_numeric_token_marker(&line.text)
            .or_else(|| leading_sentineled_note_head_marker(line))
            .or_else(|| (line.doc_note_marker > 0).then_some(line.doc_note_marker))
        && !reference.note_markers.contains(&marker)
    {
        reference.note_markers.push(marker);
        reference.note_markers.sort_unstable();
    }
    reference
}

pub(super) fn source_line_note_markers(
    line: &DeepLiquidSourceLine,
    role: LiquidBlockRole,
) -> Vec<u16> {
    let hidden_marker = lm2_hidden_numbered_note_line_marker(line);
    if let Some(marker) = hidden_marker {
        return vec![marker];
    }
    if !matches!(
        role,
        LiquidBlockRole::Footnote | LiquidBlockRole::Marginalia
    ) {
        return Vec::new();
    }
    // Raw decoder hints and bare leading numbers remain candidates only.  An
    // explicitly punctuated, small-font line in the physical note region is a
    // stronger provenance contract, however, and must survive grouping even
    // when a table prior later tries to route the block away from Marginalia.
    // Reporter volumes (`119 Colum. L. Rev.`) do not satisfy the punctuation
    // gate in `lm2_hidden_numbered_note_line_marker`.
    Vec::new()
}

/// Leading note number of a footnote definition, accepting both the bare
/// `12 Text` form and the punctuated `12. Text` form.
///
/// [`leading_note_marker`] only recognises the bare form, which loses every
/// note in the many journals that print `12.`; this is applied to lines
/// already classified as a footnote or marginalia, so the punctuated form
/// cannot be confused with a numbered list item in body text.
pub(super) fn note_head_marker(text: &str) -> Option<u16> {
    if let Some(marker) = leading_note_marker(text) {
        return Some(marker);
    }
    if !looks_like_marginalia_note_block_start(text) {
        return None;
    }
    let mut value = 0u32;
    let mut digits = 0usize;
    for ch in text.trim_start().chars() {
        let Some(digit) = ch.to_digit(10) else { break };
        value = value * 10 + digit;
        digits += 1;
        if digits > 4 {
            return None;
        }
    }
    (digits > 0 && (1..=u32::from(LM2_MAX_NOTE_MARKER)).contains(&value)).then_some(value as u16)
}

pub(super) fn sentineled_note_markers(text: &str) -> Vec<u16> {
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
                && (1..=LM2_MAX_NOTE_MARKER).contains(&marker)
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
