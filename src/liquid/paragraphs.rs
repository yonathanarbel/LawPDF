//! Paragraph and sentence splitting for Liquid Mode local extraction.
//!
//! This module owns line joining, de-hyphenation decisions, glued heading
//! splitting, long paragraph splitting, and abbreviation-aware sentence scans.

use crate::liquid::classification::{
    looks_like_abstract, looks_like_article_metadata, looks_like_author_info, looks_like_caption,
    looks_like_heading, looks_like_list_item, looks_like_marginalia, looks_like_table,
    looks_like_toc_entry, starts_with_reader_aid_prefix,
};
use crate::liquid::cleaning::{looks_like_footnote_line, split_note_marker};
use crate::liquid::model::{LiquidBlockRole, LiquidLayoutHint};
use crate::liquid::util::{
    is_letter_heading_marker, is_roman_heading_marker, should_preserve_terminal_hyphen,
    split_bare_outline_marker, word_count,
};

use super::contains_reference_year;
pub(super) fn split_paragraphs(source_text: &str) -> Vec<String> {
    split_paragraphs_with_layout_hints(source_text, &[])
}

pub(super) fn split_paragraphs_with_layout_hints(
    source_text: &str,
    hints: &[LiquidLayoutHint],
) -> Vec<String> {
    let mut paragraphs = Vec::new();
    let mut current = String::new();
    let mut pending_inline_note_fragments = Vec::new();

    for raw_line in source_text.lines() {
        let mut line = raw_line.trim().to_owned();
        if line.is_empty() {
            append_pending_inline_note_fragments(&mut current, &mut pending_inline_note_fragments);
            flush_paragraph(&mut current, &mut paragraphs);
            continue;
        }

        if let Some((marker, body)) = leading_inline_note_fragment(&line) {
            let should_delay_fragment = if current.is_empty() {
                looks_like_page_start_inline_note_fragment(marker, body)
            } else {
                looks_like_page_start_inline_note_fragment(marker, body)
                    || !ends_with_sentence_terminal(&current)
            };
            if should_delay_fragment {
                pending_inline_note_fragments.push(body.to_owned());
                continue;
            }
        }

        let hinted_role = layout_hint_role(&line, hints);
        let structural_boundary = hinted_role.is_some_and(is_structural_layout_hint_role)
            || starts_accumulatable_block(&line)
            || should_split_standalone_line(&line);
        if structural_boundary {
            append_pending_inline_note_fragments(&mut current, &mut pending_inline_note_fragments);
        } else if !pending_inline_note_fragments.is_empty() {
            line.push(' ');
            line.push_str(&pending_inline_note_fragments.join(" "));
            pending_inline_note_fragments.clear();
        }

        if hinted_role.is_some_and(is_structural_layout_hint_role) {
            flush_paragraph(&mut current, &mut paragraphs);
            paragraphs.push(line);
            continue;
        }

        if starts_accumulatable_block(&line) {
            flush_paragraph(&mut current, &mut paragraphs);
            current.push_str(&line);
            continue;
        }

        if should_split_standalone_line(&line) {
            flush_paragraph(&mut current, &mut paragraphs);
            paragraphs.push(line);
            continue;
        }

        if current.is_empty() {
            current.push_str(&line);
        } else if current.ends_with('-') {
            if !should_preserve_terminal_hyphen(&current, &line) {
                current.pop();
            }
            current.push_str(&line);
        } else {
            current.push(' ');
            current.push_str(&line);
        }
    }

    append_pending_inline_note_fragments(&mut current, &mut pending_inline_note_fragments);
    flush_paragraph(&mut current, &mut paragraphs);
    paragraphs
        .into_iter()
        .flat_map(expand_dense_paragraph)
        .collect()
}

fn append_pending_inline_note_fragments(current: &mut String, pending: &mut Vec<String>) {
    if pending.is_empty() {
        return;
    }
    if !current.is_empty() {
        current.push(' ');
    }
    current.push_str(&pending.join(" "));
    pending.clear();
}

fn ends_with_sentence_terminal(text: &str) -> bool {
    text.trim_end_matches(|ch: char| {
        ch.is_whitespace()
            || matches!(
                ch,
                '"' | '\'' | ')' | ']' | '\u{2019}' | '\u{201d}' | '\u{2018}' | '\u{201c}'
            )
    })
    .chars()
    .last()
    .is_some_and(|ch| matches!(ch, '.' | '?' | '!'))
}

fn leading_inline_note_fragment(text: &str) -> Option<(usize, &str)> {
    let trimmed = text.trim_start();
    let (marker, body) = split_note_marker(trimmed);
    let marker = marker?;
    if marker.len() > 2 || !marker.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    let marker_value = marker.parse::<usize>().ok()?;
    if marker_value < 3 {
        return None;
    }
    if trimmed[marker.len()..]
        .chars()
        .next()
        .is_some_and(|ch| matches!(ch, '.' | ')' | ']'))
    {
        return None;
    }
    if body.trim().is_empty() || word_count(body) > 18 || contains_reference_year(body) {
        return None;
    }
    starts_with_inline_prose_continuation(body).then_some((marker_value, body))
}

fn looks_like_page_start_inline_note_fragment(marker: usize, body: &str) -> bool {
    marker >= 10 && word_count(body) <= 4
}

fn starts_with_inline_prose_continuation(text: &str) -> bool {
    let trimmed = text.trim_start();
    if !trimmed
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_uppercase())
    {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    if [
        "see ", "see, ", "cf. ", "accord ", "but see ", "id. ", "supra ", "infra ", "e.g., ",
    ]
    .iter()
    .any(|prefix| lower.starts_with(prefix))
        || lower.contains(" v. ")
        || lower.contains("u.s.")
        || lower.contains("l. rev.")
        || lower.contains("law review")
    {
        return false;
    }
    [
        "and ",
        "but ",
        "so ",
        "still",
        "sure",
        "obviously",
        "recycling ",
        "one ",
        "you ",
    ]
    .iter()
    .any(|prefix| lower.starts_with(prefix))
}

pub(super) fn layout_hint_role(text: &str, hints: &[LiquidLayoutHint]) -> Option<LiquidBlockRole> {
    let key = normalize_hint_key(text);
    if key.is_empty() {
        return None;
    }
    hints
        .iter()
        .filter(|hint| normalize_hint_key(&hint.text) == key)
        .max_by_key(|hint| layout_hint_priority(hint.role))
        .map(|hint| hint.role)
}

fn normalize_hint_key(text: &str) -> String {
    let normalized = text
        .trim_end_matches(|ch: char| {
            matches!(
                ch,
                '-' | '\u{2010}' | '\u{2011}' | '\u{2012}' | '\u{2013}' | '\u{2014}'
            )
        })
        .trim_end();
    normalized
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn layout_hint_priority(role: LiquidBlockRole) -> u8 {
    match role {
        LiquidBlockRole::Marginalia => 100,
        LiquidBlockRole::Noise => 90,
        LiquidBlockRole::Contents => 80,
        LiquidBlockRole::Header | LiquidBlockRole::Footer => 70,
        LiquidBlockRole::Caption | LiquidBlockRole::Table | LiquidBlockRole::ListItem => 50,
        LiquidBlockRole::Metadata => 40,
        _ => 10,
    }
}

fn is_structural_layout_hint_role(role: LiquidBlockRole) -> bool {
    !matches!(role, LiquidBlockRole::Paragraph | LiquidBlockRole::Lead)
}

fn starts_accumulatable_block(line: &str) -> bool {
    looks_like_footnote_line(line)
        || looks_like_abstract(line)
        || starts_with_reader_aid_prefix(line)
        || looks_like_marginalia(line)
        || looks_like_table(line)
        || (looks_like_list_item(line) && !looks_like_heading(line))
}

fn should_split_standalone_line(line: &str) -> bool {
    looks_like_heading(line)
        || looks_like_list_item(line)
        || looks_like_author_info(line, 0)
        || looks_like_article_metadata(line, 0)
        || looks_like_marginalia(line)
        || looks_like_abstract(line)
        || looks_like_caption(line, 0)
        || looks_like_table(line)
        || looks_like_toc_entry(line)
        || looks_like_footnote_line(line)
        || starts_with_reader_aid_prefix(line)
}

fn flush_paragraph(current: &mut String, paragraphs: &mut Vec<String>) {
    let value = current.split_whitespace().collect::<Vec<_>>().join(" ");
    if !value.is_empty() {
        paragraphs.push(value);
    }
    current.clear();
}

fn expand_dense_paragraph(text: String) -> Vec<String> {
    let mut output = Vec::new();
    let mut rest = text.trim().to_owned();
    if rest.is_empty() {
        return output;
    }

    if let Some((header, body)) = split_pdf_export_header(&rest) {
        output.push(header);
        rest = body;
    }

    if let Some((heading, body)) = split_leading_heading_prefix(&rest) {
        output.push(heading);
        rest = body;
    }

    for segment in split_long_paragraph(&rest) {
        let segment = segment.trim();
        if !segment.is_empty() {
            output.push(segment.to_owned());
        }
    }
    output
}

fn split_pdf_export_header(text: &str) -> Option<(String, String)> {
    let marker = "Characters:";
    let marker_pos = text.find(marker)?;
    let after_marker = marker_pos + marker.len();
    let slash = text[after_marker..].find('/')?;
    let mut split_at = after_marker + slash + 1;

    while let Some(ch) = text[split_at..].chars().next() {
        if ch.is_ascii_digit() || ch == ',' || ch.is_whitespace() {
            split_at += ch.len_utf8();
        } else {
            break;
        }
    }

    let header = text[..split_at].trim();
    let body = text[split_at..].trim();
    if header.len() < 40 || body.len() < 40 {
        return None;
    }
    Some((header.to_owned(), body.to_owned()))
}

fn split_leading_heading_prefix(text: &str) -> Option<(String, String)> {
    let trimmed = text.trim();
    if trimmed.chars().count() < 60 {
        return None;
    }

    if let Some((marker_end, after_marker)) = leading_roman_or_letter_marker(trimmed) {
        if let Some(label_end) = leading_known_heading_len(after_marker) {
            let heading_end = marker_end + label_end;
            let heading = trimmed[..heading_end].trim().trim_end_matches(':');
            let body = trimmed[heading_end..]
                .trim_start_matches([':', '.', '-', ' '])
                .trim();
            if looks_like_body_after_glued_heading(body) {
                return Some((heading.to_owned(), body.to_owned()));
            }
        }
    }

    let label_end = leading_known_heading_len(trimmed)?;
    let heading = trimmed[..label_end].trim().trim_end_matches(':');
    let body = trimmed[label_end..]
        .trim_start_matches([':', '.', '-', ' '])
        .trim();
    if looks_like_body_after_glued_heading(body) {
        Some((heading.to_owned(), body.to_owned()))
    } else {
        None
    }
}

fn leading_roman_or_letter_marker(text: &str) -> Option<(usize, &str)> {
    if let Some((prefix, rest)) = text.split_once('.') {
        let rest = rest.trim_start();
        if !rest.is_empty() && (is_roman_heading_marker(prefix) || is_letter_heading_marker(prefix))
        {
            let marker_end = text.len().saturating_sub(rest.len());
            return Some((marker_end, &text[marker_end..]));
        }
    }

    let (prefix, rest) = split_bare_outline_marker(text)?;
    if is_roman_heading_marker(prefix) || is_letter_heading_marker(prefix) {
        let marker_end = text.len().saturating_sub(rest.len());
        Some((marker_end, &text[marker_end..]))
    } else {
        None
    }
}

fn leading_known_heading_len(text: &str) -> Option<usize> {
    const HEADINGS: &[&str] = &[
        "introduction",
        "background",
        "overview",
        "analysis",
        "discussion",
        "conclusion",
        "conclusions",
        "methodology",
        "methods",
        "results",
        "findings",
        "implications",
        "notes",
        "references",
    ];

    let trimmed_start = text.len().saturating_sub(text.trim_start().len());
    let candidate = text.trim_start();
    let lower = candidate.to_ascii_lowercase();
    for heading in HEADINGS {
        if lower == *heading {
            return None;
        }
        if lower.starts_with(&format!("{heading}:")) {
            return Some(trimmed_start + heading.len());
        }
        if lower.starts_with(&format!("{heading} ")) {
            return Some(trimmed_start + heading.len());
        }
    }
    None
}

fn looks_like_body_after_glued_heading(body: &str) -> bool {
    if body.chars().count() < 40 || word_count(body) < 7 {
        return false;
    }
    body.chars()
        .find(|ch| ch.is_alphabetic())
        .is_some_and(|ch| ch.is_uppercase())
}

fn split_long_paragraph(text: &str) -> Vec<String> {
    const TARGET_CHARS: usize = 620;
    const MAX_CHARS: usize = 1_050;

    if text.chars().count() <= MAX_CHARS {
        return vec![text.to_owned()];
    }

    let mut parts = Vec::new();
    let mut current = String::new();
    for sentence in split_sentences(text) {
        let sentence = sentence.trim();
        if sentence.is_empty() {
            continue;
        }
        let current_len = current.chars().count();
        let sentence_len = sentence.chars().count();
        if !current.is_empty()
            && current_len >= TARGET_CHARS
            && current_len + sentence_len > TARGET_CHARS
        {
            parts.push(current.trim().to_owned());
            current.clear();
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(sentence);
    }
    if !current.trim().is_empty() {
        parts.push(current.trim().to_owned());
    }

    if parts.len() <= 1 {
        vec![text.to_owned()]
    } else {
        parts
    }
}

/// Returns true if the . at punct_idx is part of a known legal/name/etc abbreviation
/// (e.g. "U.S.", "Inc.", "Fed.", "No.", "e.g.") and thus should not end a sentence.
fn is_abbrev_terminator(text: &str, punct_idx: usize) -> bool {
    if punct_idx == 0 {
        return false;
    }
    // Scan a small window before the punct for the preceding token.
    let look_start = floor_char_boundary(text, punct_idx.saturating_sub(18));
    let preceding = &text[look_start..punct_idx];
    let token = preceding
        .rsplit(|c: char| {
            c.is_whitespace()
                || matches!(
                    c,
                    '.' | ',' | ';' | ':' | '(' | ')' | '[' | ']' | '"' | '“' | '”' | '‘' | '’'
                )
        })
        .next()
        .unwrap_or("")
        .trim_matches(|c: char| !c.is_alphanumeric() && c != '.')
        .to_ascii_lowercase();
    if token.is_empty() {
        return false;
    }

    // Focused list of common legal, corporate, title, and reference abbreviations.
    // Extend here for more coverage; case-insensitive match after lower.
    static LEGAL_ABBREVS: &[&str] = &[
        "u.s.",
        "u.s.c.",
        "inc.",
        "ltd.",
        "co.",
        "corp.",
        "llc.",
        "llp.",
        "no.",
        "nos.",
        "vol.",
        "p.",
        "pp.",
        "fig.",
        "tbl.",
        "ch.",
        "sec.",
        "art.",
        "mr.",
        "mrs.",
        "ms.",
        "dr.",
        "prof.",
        "rev.",
        "st.",
        "sr.",
        "jr.",
        "hon.",
        "etc.",
        "e.g.",
        "i.e.",
        "cf.",
        "viz.",
        "et al.",
        "ca.",
        "approx.",
        "v.",
        "vs.",
        "ex rel.",
        "ex parte.",
        "fed.",
        "f.2d",
        "f.3d",
        "f.supp.",
        "f.supp.2d",
        "s.ct.",
        "l.ed.",
        "l.ed.2d",
        "n.d.",
        "s.d.",
        "e.d.",
        "w.d.",
        "c.d.",
        "cir.",
        "ct.",
        "app.",
        "sup.",
        "cert.",
    ];
    LEGAL_ABBREVS.iter().any(|&a| {
        token == a
            || token.ends_with(a)
            || a.strip_suffix('.')
                .map_or(false, |b| token == b || token.ends_with(b))
    })
}

fn floor_char_boundary(value: &str, index: usize) -> usize {
    let mut index = index.min(value.len());
    while index > 0 && !value.is_char_boundary(index) {
        index -= 1;
    }
    index
}

pub(super) fn split_sentences(text: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut start = 0usize;
    let mut chars = text.char_indices().peekable();
    while let Some((idx, ch)) = chars.next() {
        if !matches!(ch, '.' | '?' | '!') {
            continue;
        }
        let Some((next_idx, next)) = chars.peek().copied() else {
            continue;
        };
        if !next.is_whitespace() {
            continue;
        }
        if ch == '.' && is_abbrev_terminator(text, idx) {
            continue;
        }
        let end = idx + ch.len_utf8();
        let sentence = text[start..end].trim();
        if !sentence.is_empty() {
            sentences.push(sentence.to_owned());
        }
        start = next_idx;
    }
    let tail = text[start..].trim();
    if !tail.is_empty() {
        sentences.push(tail.to_owned());
    }
    sentences
}
