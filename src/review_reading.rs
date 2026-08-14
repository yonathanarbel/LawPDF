//! Pure Review Mode helpers: opening-page preview, margin layout, article
//! search, live corrections, and table/figure crop geometry.
//!
//! These functions are the shipped decisions used by the UI and LM2 prepare
//! path. Keep them free of Pdfium and native-model I/O so tests can drive them
//! directly.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use crate::liquid::{
    ArticleSpan, LiquidBlock, LiquidBlockRole, LiquidDocument, hidden_contents_mask_for_display,
    should_hide_contents_block_for_display,
};

pub const REVIEW_OPENING_PAGE_LIMIT: usize = 4;
/// Opening a PDF never starts a full Review job. Cache hits are free; anything
/// larger waits until the user actually opens Review.
pub const REVIEW_AUTO_PRECOMPUTE_PAGE_LIMIT_EXCLUSIVE: usize = 1;
/// Full-document Review is automatic only for short files. Longer PDFs stay on
/// the opening-page preview until the user asks for the rest.
pub const REVIEW_AUTO_FULL_PAGE_LIMIT_EXCLUSIVE: usize = 32;
pub const REVIEW_PARAGRAPH_GAP: f32 = 16.0;
pub const REVIEW_MARGIN_MIN_WIDTH: f32 = 168.0;
pub const REVIEW_MARGIN_MAX_WIDTH: f32 = 260.0;
pub const REVIEW_MARGIN_GAP: f32 = 16.0;
// `article_segments::score_confidence` normalizes boundary scores to 0.50..0.99.
// Keep this threshold on that same scale; the former raw-score value `3.0`
// made every detected bound volume look low-confidence and silently unioned
// article-local note starts with the unsafe file-global result.
pub const REVIEW_HIGH_CONFIDENCE_ARTICLE_SPAN: f32 = 0.85;
pub const REVIEW_OUTLINE_RAIL_DEFAULT_WIDTH: f32 = 236.0;

/// Hide printed tables of contents from the Review reading column.
pub fn review_hidden_display_mask(blocks: &[LiquidBlock]) -> Vec<bool> {
    let mut mask = hidden_contents_mask_for_display(blocks);
    let title_keys = blocks
        .iter()
        .filter(|block| block.role == LiquidBlockRole::Title)
        .map(|block| normalize_review_title_key(&block.text))
        .filter(|key| !key.is_empty())
        .collect::<HashSet<_>>();
    for (index, block) in blocks.iter().enumerate() {
        if is_review_note_display_block(block) {
            mask[index] = false;
            continue;
        }
        if is_review_table_of_contents_text(&block.text) {
            mask[index] = true;
            continue;
        }
        if matches!(
            block.role,
            LiquidBlockRole::Heading | LiquidBlockRole::Subheading
        ) {
            let key = normalize_review_title_key(&block.text);
            if !key.is_empty() && title_keys.contains(&key) {
                mask[index] = true;
            }
        }
    }
    mask
}

pub fn is_review_table_of_contents_text(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    if is_explicit_contents_heading(trimmed) {
        return true;
    }
    if fused_toc_page_locator_count(trimmed) >= 2 {
        return true;
    }
    if compound_toc_entries(trimmed).len() >= 2 {
        return true;
    }
    is_single_dotleader_toc_row(trimmed)
}

fn is_explicit_contents_heading(text: &str) -> bool {
    let normalized = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "contents"
            | "table of contents"
            | "brief contents"
            | "brief table of contents"
            | "contents of this article"
            | "contents of the article"
            | "article outline"
            | "article contents"
    ) || normalized.starts_with("table of contents ")
        || normalized.starts_with("contents ")
}

fn is_single_dotleader_toc_row(text: &str) -> bool {
    if fused_toc_page_locator_count(text) != 1 {
        return false;
    }
    let words = text.split_whitespace().count();
    words > 1 && words <= 36
}

/// Count "long leader + page number" pairs. Law-review TOCs look like
/// `A. Two Entitlements................ 13`. Ordinary prose almost never has
/// five or more consecutive dots followed by a page number.
pub fn fused_toc_page_locator_count(text: &str) -> usize {
    let chars = text.chars().collect::<Vec<_>>();
    let mut count = 0usize;
    let mut index = 0usize;
    while index < chars.len() {
        let start = index;
        let solid = consume_solid_leader(&chars, &mut index);
        let spaced = if solid == 0 {
            consume_spaced_leader(&chars, &mut index)
        } else {
            0
        };
        if solid >= 5 || spaced >= 4 {
            while index < chars.len() && chars[index].is_whitespace() {
                index += 1;
            }
            let digits = consume_page_digits(&chars, &mut index);
            if (1..=3).contains(&digits) {
                count += 1;
                continue;
            }
        }
        if index == start {
            index += 1;
        }
    }
    count
}

fn consume_solid_leader(chars: &[char], index: &mut usize) -> usize {
    let start = *index;
    while *index < chars.len() && matches!(chars[*index], '.' | '·' | '•' | '…' | '⋯') {
        *index += 1;
    }
    *index - start
}

fn consume_spaced_leader(chars: &[char], index: &mut usize) -> usize {
    let mut dots = 0usize;
    let mut cursor = *index;
    while cursor + 1 < chars.len() && chars[cursor] == '.' && chars[cursor + 1].is_whitespace() {
        dots += 1;
        cursor += 2;
        while cursor < chars.len() && chars[cursor].is_whitespace() {
            cursor += 1;
        }
    }
    if dots >= 4 {
        *index = cursor;
        dots
    } else {
        0
    }
}

fn consume_page_digits(chars: &[char], index: &mut usize) -> usize {
    let start = *index;
    while *index < chars.len() && chars[*index].is_ascii_digit() {
        *index += 1;
    }
    *index - start
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewContentsEntry {
    pub source_block_index: usize,
    pub title: String,
}

/// Titles from a hidden printed TOC, in document order. The Review rail uses
/// these as the navigation spec and resolves each title to a real body block.
pub fn review_contents_navigation_entries(blocks: &[LiquidBlock]) -> Vec<ReviewContentsEntry> {
    let mask = review_hidden_display_mask(blocks);
    let mut entries = Vec::new();
    for (index, block) in blocks.iter().enumerate() {
        if !mask.get(index).copied().unwrap_or(false) {
            continue;
        }
        if is_explicit_contents_heading(block.text.trim()) {
            continue;
        }
        let compound = compound_toc_entries(&block.text);
        if !compound.is_empty() {
            for title in compound {
                push_navigation_entry(&mut entries, index, title);
            }
            continue;
        }
        if is_single_dotleader_toc_row(block.text.trim()) {
            push_navigation_entry(&mut entries, index, toc_entry_display_title(&block.text));
        }
    }
    entries
}

fn push_navigation_entry(entries: &mut Vec<ReviewContentsEntry>, index: usize, title: String) {
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.is_empty() || is_explicit_contents_heading(&title) {
        return;
    }
    let key = normalize_contents_title(&title);
    if key.is_empty()
        || entries
            .iter()
            .any(|existing| normalize_contents_title(&existing.title) == key)
    {
        return;
    }
    entries.push(ReviewContentsEntry {
        source_block_index: index,
        title,
    });
}

fn normalize_contents_title(text: &str) -> String {
    text.chars()
        .map(|ch| {
            if ch.is_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn compound_toc_entries(text: &str) -> Vec<String> {
    let mut entries = Vec::new();
    let mut cursor = 0usize;
    while cursor < text.len() {
        let Some((leader_start, leader_end)) = next_dot_leader(text, cursor) else {
            break;
        };
        let title = clean_compound_toc_title(&text[cursor..leader_start]);
        let locator_start = text[leader_end..]
            .char_indices()
            .find(|(_, ch)| !ch.is_whitespace())
            .map_or(leader_end, |(offset, _)| leader_end + offset);
        let locator_end = text[locator_start..]
            .char_indices()
            .find(|(_, ch)| ch.is_whitespace())
            .map_or(text.len(), |(offset, _)| locator_start + offset);
        let locator = text[locator_start..locator_end].trim();
        if !title.is_empty() && is_toc_page_locator_token(locator) {
            entries.push(title);
            cursor = locator_end;
        } else {
            cursor = leader_end;
        }
    }
    entries
}

fn next_dot_leader(text: &str, cursor: usize) -> Option<(usize, usize)> {
    let tail = &text[cursor..];
    let ascii = tail.find("...").map(|offset| cursor + offset);
    let spaced = tail.find(". .").map(|offset| cursor + offset);
    let ellipsis = tail.find('\u{2026}').map(|offset| cursor + offset);
    let start = [ascii, spaced, ellipsis].into_iter().flatten().min()?;
    let first = text[start..].chars().next()?;
    if first == '\u{2026}' {
        return Some((start, start + first.len_utf8()));
    }
    let end = text[start..]
        .char_indices()
        .find(|(_, ch)| *ch != '.' && !ch.is_whitespace())
        .map_or(text.len(), |(offset, _)| start + offset);
    Some((start, end))
}

fn clean_compound_toc_title(text: &str) -> String {
    let title = text
        .trim()
        .trim_start_matches(|ch: char| matches!(ch, '.' | ',' | ';' | ':' | '-'))
        .trim();
    let Some((first, rest)) = title.split_once(char::is_whitespace) else {
        return title.to_owned();
    };
    if first.chars().all(|ch| ch.is_ascii_digit()) && !rest.trim().is_empty() {
        rest.trim().to_owned()
    } else {
        title.to_owned()
    }
}

fn toc_entry_display_title(text: &str) -> String {
    let before_leader = text
        .split('\u{2026}')
        .next()
        .unwrap_or(text)
        .split("...")
        .next()
        .unwrap_or(text)
        .trim();
    if before_leader != text.trim() {
        return before_leader.to_owned();
    }
    let mut parts = text.rsplitn(2, char::is_whitespace);
    let locator = parts
        .next()
        .unwrap_or_default()
        .trim_matches(|ch: char| matches!(ch, '.' | ',' | ';' | ':' | '(' | ')'));
    let title = parts.next().unwrap_or_default().trim();
    if is_toc_page_locator_token(locator) {
        title.to_owned()
    } else {
        text.trim().to_owned()
    }
}

fn is_toc_page_locator_token(text: &str) -> bool {
    let token = text.trim_matches(|ch: char| matches!(ch, '.' | ',' | ';' | ':' | '(' | ')'));
    if token.is_empty() {
        return false;
    }
    if token.len() <= 4 && token.chars().all(|ch| ch.is_ascii_digit()) {
        return true;
    }
    token.len() <= 8
        && token
            .chars()
            .all(|ch| matches!(ch.to_ascii_lowercase(), 'i' | 'v' | 'x' | 'l' | 'c'))
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReviewSearchHit {
    pub block_index: usize,
    pub match_start: usize,
    pub match_end: usize,
    pub snippet: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReviewSourceCrop {
    pub page_index: usize,
    pub left: f32,
    pub bottom: f32,
    pub right: f32,
    pub top: f32,
}

pub fn review_opening_page_count(total_pages: usize) -> usize {
    total_pages.min(REVIEW_OPENING_PAGE_LIMIT)
}

/// Background Review on file-open is cache-only. Starting extract + model work
/// here is what stalls the UI and wastes cost on PDFs the user never Reviews.
pub fn should_precompute_review_on_open(page_count: usize) -> bool {
    (1..REVIEW_AUTO_PRECOMPUTE_PAGE_LIMIT_EXCLUSIVE).contains(&page_count)
}

pub fn review_allows_automatic_full_prepare(page_count: usize) -> bool {
    page_count > 0 && page_count < REVIEW_AUTO_FULL_PAGE_LIMIT_EXCLUSIVE
}

/// Law-review style superscripts. Regular digits plus `.raised()` look like a
/// floating callout chip; these sit on the line like a print footnote.
pub fn review_footnote_superscript(marker: &str) -> String {
    marker
        .trim()
        .chars()
        .map(|ch| match ch {
            '0' => '⁰',
            '1' => '¹',
            '2' => '²',
            '3' => '³',
            '4' => '⁴',
            '5' => '⁵',
            '6' => '⁶',
            '7' => '⁷',
            '8' => '⁸',
            '9' => '⁹',
            '*' | '∗' => '⁎',
            _ => ch,
        })
        .collect()
}

/// Visual paragraph breaks already present in a single Review block.
pub fn review_paragraph_display_parts(text: &str) -> Vec<&str> {
    let parts = text
        .split("\n\n")
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.len() > 1 {
        parts
    } else if text.is_empty() {
        Vec::new()
    } else {
        vec![text]
    }
}

/// True when the first readable Review screen can be built from the pages that
/// are already extracted. A longer document must not wait for later pages.
pub fn opening_pages_ready_for_review(
    native_text_loaded: &[bool],
    text_chars_present: &[bool],
    total_pages: usize,
) -> bool {
    if total_pages == 0 {
        return false;
    }
    let limit = review_opening_page_count(total_pages);
    (0..limit).all(|page| {
        native_text_loaded.get(page).copied().unwrap_or(false)
            && text_chars_present.get(page).copied().unwrap_or(false)
    })
}

pub fn review_all_pages_ready(
    native_text_loaded: &[bool],
    text_chars_present: &[bool],
    total_pages: usize,
) -> bool {
    total_pages > 0
        && (0..total_pages).all(|page| {
            native_text_loaded.get(page).copied().unwrap_or(false)
                && text_chars_present.get(page).copied().unwrap_or(false)
        })
}

/// What `ensure_liquid_mode2_started` should do next. Short documents never
/// get a separate preview job; a finished document ignores a late preview.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewPrepareAction {
    WaitForPages,
    SpawnPreview { page_count: usize },
    SpawnFull { page_count: usize },
    Nothing,
}

pub fn review_prepare_next_action(
    total_pages: usize,
    opening_ready: bool,
    all_ready: bool,
    preview_spawned: bool,
    full_spawned: bool,
) -> ReviewPrepareAction {
    if total_pages == 0 {
        return ReviewPrepareAction::Nothing;
    }
    let opening = review_opening_page_count(total_pages);
    let preview_is_distinct = opening < total_pages;
    if preview_is_distinct {
        if !preview_spawned {
            if opening_ready {
                return ReviewPrepareAction::SpawnPreview {
                    page_count: opening,
                };
            }
            return ReviewPrepareAction::WaitForPages;
        }
        if !full_spawned {
            if all_ready {
                return ReviewPrepareAction::SpawnFull {
                    page_count: total_pages,
                };
            }
            return ReviewPrepareAction::WaitForPages;
        }
        return ReviewPrepareAction::Nothing;
    }
    if full_spawned {
        return ReviewPrepareAction::Nothing;
    }
    if all_ready {
        ReviewPrepareAction::SpawnFull {
            page_count: total_pages,
        }
    } else {
        ReviewPrepareAction::WaitForPages
    }
}

/// Drop a full-document spawn when the file is large and the user has not asked.
pub fn review_gate_automatic_full(
    action: ReviewPrepareAction,
    total_pages: usize,
    allow_full: bool,
) -> ReviewPrepareAction {
    if allow_full || total_pages <= review_opening_page_count(total_pages) {
        return action;
    }
    match action {
        ReviewPrepareAction::SpawnFull { .. } => ReviewPrepareAction::Nothing,
        other => other,
    }
}

/// A late opening-pages event must not replace a finished full Review.
pub fn should_apply_review_event(already_complete: bool, event_complete: bool) -> bool {
    event_complete || !already_complete
}

/// Flags stored per tab so switching documents cannot leak the previous job.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReviewPrepareFlags {
    pub preview_spawned: bool,
    pub full_spawned: bool,
    pub complete: bool,
    pub pdf_split: bool,
}

pub fn review_prepare_flags_after_restart() -> ReviewPrepareFlags {
    ReviewPrepareFlags::default()
}

/// Side-note column width. Zero means the three-column margin layout stays off.
pub fn review_margin_width(available_width: f32, body_width: f32) -> f32 {
    review_column_layout(available_width, body_width, true).margin_width
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReviewColumnLayout {
    pub body_width: f32,
    pub margin_width: f32,
    pub row_width: f32,
    pub side: f32,
    pub body_indent: f32,
}

/// Center the reading row in the remaining Review area. Note columns live
/// *inside* `row_width`, so leftover grey is equal on both sides of the
/// whole row — not a wide left gutter plus a body jammed against CONTENTS.
pub fn review_column_layout(
    available_width: f32,
    max_body_width: f32,
    show_note_margins: bool,
) -> ReviewColumnLayout {
    let body_width = available_width
        .min(max_body_width)
        .max(360.0)
        .min(available_width.max(360.0));
    if !show_note_margins || available_width <= 0.0 || body_width <= 0.0 {
        let side = ((available_width - body_width) * 0.5).max(0.0);
        return ReviewColumnLayout {
            body_width,
            margin_width: 0.0,
            row_width: body_width,
            side,
            body_indent: 0.0,
        };
    }
    let needed = body_width + 2.0 * (REVIEW_MARGIN_MIN_WIDTH + REVIEW_MARGIN_GAP);
    if available_width + 0.5 < needed {
        let side = ((available_width - body_width) * 0.5).max(0.0);
        return ReviewColumnLayout {
            body_width,
            margin_width: 0.0,
            row_width: body_width,
            side,
            body_indent: 0.0,
        };
    }
    let leftover = (available_width - body_width) / 2.0 - REVIEW_MARGIN_GAP;
    let margin_width = leftover.clamp(REVIEW_MARGIN_MIN_WIDTH, REVIEW_MARGIN_MAX_WIDTH);
    let row_width = body_width + 2.0 * (margin_width + REVIEW_MARGIN_GAP);
    let side = ((available_width - row_width) * 0.5).max(0.0);
    ReviewColumnLayout {
        body_width,
        margin_width,
        row_width,
        side,
        body_indent: margin_width + REVIEW_MARGIN_GAP,
    }
}

fn normalize_review_title_key(text: &str) -> String {
    text.chars()
        .map(|ch| {
            if ch.is_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Split a fused law-review note block (`24 See … 25 See … 26 This …`)
/// into one entry per note number so the rail and popovers stay sequential.
pub fn split_fused_review_notes(text: &str) -> Vec<(String, String)> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let (first_marker, first_body) = split_leading_note_marker(trimmed);
    let leading_number = first_marker.and_then(|marker| marker.parse::<u16>().ok());
    let mut starts = Vec::new();
    if let Some(number) = leading_number {
        starts.push((0usize, number.to_string()));
        let mut expected = number.saturating_add(1);
        let mut search_from = first_marker.map(|marker| marker.len()).unwrap_or(0);
        while let Some((at, found)) = find_next_fused_note_number(trimmed, search_from, expected) {
            starts.push((at, found.to_string()));
            expected = found.saturating_add(1);
            search_from = at + found.to_string().len();
        }
        if starts.len() == 1 {
            if let Some(run) = first_fused_note_run(trimmed, search_from) {
                if false_leading_note_head(number, &run) {
                    starts = run
                        .into_iter()
                        .map(|(at, found)| (at, found.to_string()))
                        .collect();
                }
            }
        }
    } else {
        if trimmed.starts_with(['*', '∗']) {
            starts.push((0usize, "*".to_owned()));
        }
        if let Some(run) = first_fused_note_run(trimmed, 0) {
            for (at, found) in run {
                starts.push((at, found.to_string()));
            }
        }
        if starts.is_empty() {
            return vec![("*".to_owned(), first_body.trim().to_owned())];
        }
    }
    emit_fused_note_parts(trimmed, &starts, first_body)
}

fn emit_fused_note_parts(
    trimmed: &str,
    starts: &[(usize, String)],
    first_body: &str,
) -> Vec<(String, String)> {
    let mut parts = Vec::new();
    let first_at = starts.first().map(|(at, _)| *at).unwrap_or(0);
    if first_at > 0 {
        let continuation = continuation_body_from_prefix(trimmed[..first_at].trim());
        if !continuation.is_empty() {
            parts.push(("*".to_owned(), continuation));
        }
    } else if starts.len() == 1 {
        let body = first_body.trim().to_owned();
        return vec![(starts[0].1.clone(), body)];
    }
    for (index, (start, marker)) in starts.iter().enumerate() {
        let end = starts
            .get(index + 1)
            .map(|(next, _)| *next)
            .unwrap_or(trimmed.len());
        let chunk = trimmed[*start..end].trim();
        let body = chunk
            .strip_prefix(marker.as_str())
            .unwrap_or(chunk)
            .trim_start_matches(['.', ')', ':'])
            .trim()
            .to_owned();
        if !body.is_empty() || marker != "*" {
            parts.push((marker.clone(), body));
        }
    }
    if parts.is_empty() {
        return vec![("*".to_owned(), first_body.trim().to_owned())];
    }
    parts
}

fn continuation_body_from_prefix(prefix: &str) -> String {
    let (marker, body) = split_leading_note_marker(prefix);
    if marker.is_some() {
        body.trim().to_owned()
    } else {
        prefix.trim().to_owned()
    }
}

fn first_fused_note_run(text: &str, from: usize) -> Option<Vec<(usize, u16)>> {
    let (at, number) = find_first_plausible_note_head(text, from)?;
    let mut run = vec![(at, number)];
    let mut expected = number.saturating_add(1);
    let mut search = at + number.to_string().len();
    while let Some((next_at, found)) = find_next_fused_note_number(text, search, expected) {
        run.push((next_at, found));
        expected = found.saturating_add(1);
        search = next_at + found.to_string().len();
    }
    Some(run)
}

fn false_leading_note_head(lead: u16, run: &[(usize, u16)]) -> bool {
    run.len() >= 2 && run[0].1.saturating_add(15) < lead
}

fn find_first_plausible_note_head(text: &str, from: usize) -> Option<(usize, u16)> {
    let tail = text.get(from..)?;
    let mut search = 0usize;
    while search < tail.len() {
        let rest = &tail[search..];
        let Some(digit_at) = rest.find(|ch: char| ch.is_ascii_digit()) else {
            return None;
        };
        let abs = from + search + digit_at;
        let mut end = abs;
        let mut digits = 0usize;
        for (index, ch) in text[abs..].char_indices() {
            if ch.is_ascii_digit() && digits < 3 {
                digits += 1;
                end = abs + index + ch.len_utf8();
                continue;
            }
            break;
        }
        let parsed = text[abs..end].parse::<u16>().ok();
        let next = text.get(end..).and_then(|suffix| suffix.chars().next());
        if let Some(number) = parsed {
            if (1..=399).contains(&number)
                && next.is_some_and(|ch| ch.is_whitespace())
                && !note_number_has_citation_prefix(text, abs)
                && (abs == 0 || note_number_follows_sentence_break(text, abs))
                && !note_number_is_citation_token(text, abs, number)
            {
                return Some((abs, number));
            }
        }
        search = end.saturating_sub(from).max(search + 1);
    }
    None
}

fn find_next_fused_note_number(text: &str, from: usize, expected: u16) -> Option<(usize, u16)> {
    for candidate in expected..=expected.saturating_add(2) {
        if let Some(at) = find_note_number_at_boundary(text, from, candidate) {
            return Some((at, candidate));
        }
    }
    None
}

fn find_note_number_at_boundary(text: &str, from: usize, number: u16) -> Option<usize> {
    if from >= text.len() {
        return None;
    }
    let needle = number.to_string();
    let mut search = from;
    while let Some(rel) = text.get(search..)?.find(&needle) {
        let at = search + rel;
        let after = at + needle.len();
        let prev = text
            .get(..at)
            .and_then(|prefix| prefix.chars().rev().next());
        if prev.is_some_and(|ch| ch.is_ascii_digit()) {
            search = after;
            continue;
        }
        let next = text.get(after..).and_then(|suffix| suffix.chars().next());
        if !next.is_some_and(|ch| ch.is_whitespace()) {
            search = after;
            continue;
        }
        if note_number_has_citation_prefix(text, at)
            || !note_number_follows_sentence_break(text, at)
        {
            search = after;
            continue;
        }
        return Some(at);
    }
    None
}

fn split_leading_note_marker(text: &str) -> (Option<&str>, &str) {
    let trimmed = text.trim_start();
    let mut end = 0usize;
    let mut digits = 0usize;
    for (index, ch) in trimmed.char_indices() {
        if ch.is_ascii_digit() && digits < 3 {
            digits += 1;
            end = index + ch.len_utf8();
            continue;
        }
        break;
    }
    if digits == 0 {
        return (None, trimmed);
    }
    let rest = &trimmed[end..];
    if rest.starts_with(char::is_whitespace) || rest.starts_with(['.', ')', ':']) || rest.is_empty()
    {
        (
            Some(&trimmed[..end]),
            rest.trim_start_matches(['.', ')', ':']).trim_start(),
        )
    } else {
        (None, trimmed)
    }
}

fn note_number_has_citation_prefix(text: &str, at: usize) -> bool {
    let prefix = text
        .get(..at)
        .unwrap_or("")
        .rsplit_once(char::is_whitespace)
        .map(|(_, last)| last)
        .unwrap_or("")
        .trim_end_matches(['.', ',', ';', ':'])
        .to_ascii_lowercase();
    matches!(
        prefix.as_str(),
        "note"
            | "notes"
            | "n"
            | "nn"
            | "p"
            | "pp"
            | "at"
            | "supra"
            | "infra"
            | "§"
            | "rev"
            | "vol"
            | "volume"
            | "id"
            | "ibid"
            | "cir"
            | "app"
    )
}

fn note_number_follows_sentence_break(text: &str, at: usize) -> bool {
    let prefix = text.get(..at).unwrap_or("").trim_end();
    if prefix.is_empty() {
        return true;
    }
    prefix.ends_with([
        '.', '?', '!', ';', ':', ')', '…', '"', '\u{201C}', '\u{201D}', '\u{2018}', '\u{2019}',
    ])
}

fn note_number_is_citation_token(text: &str, at: usize, number: u16) -> bool {
    let after = at + number.to_string().len();
    let rest = text.get(after..).unwrap_or("").trim_start();
    if rest.starts_with('(') {
        let year: String = rest.chars().skip(1).take(4).collect();
        if year.len() == 4 && year.chars().all(|ch| ch.is_ascii_digit()) {
            if let Ok(parsed) = year.parse::<u16>() {
                if (1600..=2099).contains(&parsed) {
                    return true;
                }
            }
        }
    }
    if looks_like_reporter_start(rest) || looks_like_volume_title(rest) {
        return true;
    }
    false
}

fn looks_like_reporter_start(rest: &str) -> bool {
    let token = rest
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_end_matches([',', ';', ':']);
    let compact: String = token
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .map(|ch| ch.to_ascii_lowercase())
        .collect();
    matches!(
        compact.as_str(),
        "p2d"
            | "p3d"
            | "a2d"
            | "a3d"
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
            | "so3d"
            | "f2d"
            | "f3d"
            | "f4th"
            | "fsupp"
            | "fsupp2d"
            | "us"
            | "sct"
            | "led"
            | "led2d"
            | "eng"
    )
}

fn looks_like_volume_title(rest: &str) -> bool {
    let mut words = rest.split_whitespace();
    let Some(first) = words.next() else {
        return false;
    };
    let Some(second) = words.next() else {
        return false;
    };
    is_smallcaps_title_word(first) && is_smallcaps_title_word(second)
}

fn is_smallcaps_title_word(word: &str) -> bool {
    let letters: String = word.chars().filter(|ch| ch.is_ascii_alphabetic()).collect();
    !letters.is_empty() && letters.chars().all(|ch| ch.is_ascii_uppercase())
}

/// Footnote/marginalia blocks, plus Noise that is actually a dropped note.
pub fn is_review_note_display_block(block: &LiquidBlock) -> bool {
    matches!(
        block.role,
        LiquidBlockRole::Footnote | LiquidBlockRole::Marginalia
    ) || (block.role == LiquidBlockRole::Noise && noise_block_is_review_note(&block.text))
}

/// Marginalia and rescued Noise notes sit in the side rail, not the body.
pub fn is_review_margin_note_block(block: &LiquidBlock) -> bool {
    block.role == LiquidBlockRole::Marginalia
        || (block.role == LiquidBlockRole::Noise && noise_block_is_review_note(&block.text))
}

pub fn noise_block_is_review_note(text: &str) -> bool {
    let (marker, body) = split_leading_note_marker(text.trim());
    let Some(marker) = marker else {
        return false;
    };
    let Ok(number) = marker.parse::<u16>() else {
        return false;
    };
    if !(1..=399).contains(&number) {
        return false;
    }
    let body = body.trim();
    if body.is_empty() || body.starts_with(']') || is_review_table_of_contents_text(body) {
        return false;
    }
    body.chars().next().is_some_and(|ch| ch.is_alphabetic()) && body.split_whitespace().count() >= 6
}

/// Furniture skip for the Review column. Rescued notes are never furniture.
pub fn review_skips_block_as_furniture(block: &LiquidBlock, hidden_by_mask: bool) -> bool {
    if is_review_note_display_block(block) {
        return false;
    }
    hidden_by_mask || should_hide_contents_block_for_display(block)
}

/// Consecutive margin notes from `start`, skipping true furniture only.
pub fn review_collect_margin_note_indices(
    blocks: &[LiquidBlock],
    mut index: usize,
    hidden: &[bool],
) -> (Vec<usize>, usize) {
    let mut notes = Vec::new();
    while index < blocks.len() {
        let block = &blocks[index];
        let is_hidden = hidden.get(index).copied().unwrap_or(false);
        if review_skips_block_as_furniture(block, is_hidden) {
            index += 1;
            continue;
        }
        if !is_review_margin_note_block(block) {
            break;
        }
        notes.push(index);
        index += 1;
    }
    (notes, index)
}

pub fn article_spans_may_revoke_global_note_starts(spans: &[ArticleSpan]) -> bool {
    spans.len() > 1
        && spans
            .iter()
            .all(|span| span.confidence >= REVIEW_HIGH_CONFIDENCE_ARTICLE_SPAN)
}

pub fn merge_article_and_global_note_starts(
    scoped: HashSet<String>,
    global: HashSet<String>,
    spans: &[ArticleSpan],
) -> HashSet<String> {
    if article_spans_may_revoke_global_note_starts(spans) {
        scoped
    } else if spans.is_empty() {
        scoped
    } else {
        scoped.union(&global).cloned().collect()
    }
}

pub fn find_hits_in_review_blocks(blocks: &[LiquidBlock], query: &str) -> Vec<ReviewSearchHit> {
    let needle = query.trim();
    if needle.is_empty() {
        return Vec::new();
    }
    let needle_lower = needle.to_ascii_lowercase();
    let mut hits = Vec::new();
    for (block_index, block) in blocks.iter().enumerate() {
        if matches!(
            block.role,
            LiquidBlockRole::Header
                | LiquidBlockRole::Footer
                | LiquidBlockRole::Contents
                | LiquidBlockRole::Noise
                | LiquidBlockRole::SectionBreak
        ) || is_review_table_of_contents_text(&block.text)
        {
            continue;
        }
        let haystack = block.text.to_ascii_lowercase();
        let mut from = 0usize;
        while let Some(rel) = haystack[from..].find(&needle_lower) {
            let match_start = from + rel;
            let match_end = match_start + needle_lower.len();
            hits.push(ReviewSearchHit {
                block_index,
                match_start,
                match_end,
                snippet: review_hit_snippet(&block.text, match_start, match_end),
            });
            from = match_end;
            if from >= haystack.len() {
                break;
            }
        }
    }
    hits
}

fn review_hit_snippet(text: &str, match_start: usize, match_end: usize) -> String {
    let start = match_start.saturating_sub(24);
    let end = (match_end + 24).min(text.len());
    let mut snippet = String::new();
    if start > 0 {
        snippet.push('…');
    }
    snippet.push_str(text.get(start..end).unwrap_or(text));
    if end < text.len() {
        snippet.push('…');
    }
    snippet
}

pub fn review_document_plain_text(document: &LiquidDocument) -> String {
    let mut parts = Vec::new();
    let title = document.title.trim();
    if !title.is_empty() {
        parts.push(title.to_owned());
    }
    for block in &document.blocks {
        if matches!(
            block.role,
            LiquidBlockRole::Header
                | LiquidBlockRole::Footer
                | LiquidBlockRole::Contents
                | LiquidBlockRole::Noise
                | LiquidBlockRole::SectionBreak
        ) || is_review_table_of_contents_text(&block.text)
        {
            continue;
        }
        let text = block.text.split_whitespace().collect::<Vec<_>>().join(" ");
        if !text.is_empty() {
            parts.push(text);
        }
    }
    parts.join("\n\n")
}

pub fn apply_live_review_correction(
    document: &mut LiquidDocument,
    block_index: usize,
    expected_role: LiquidBlockRole,
) -> bool {
    let Some(block) = document.blocks.get_mut(block_index) else {
        return false;
    };
    block.role = expected_role;
    if let Some(source) = document
        .block_source_lines
        .iter_mut()
        .find(|source| source.block_index == block_index)
    {
        for line in &mut source.lines {
            line.role = expected_role;
        }
    }
    true
}

pub fn review_table_figure_crop(
    role: LiquidBlockRole,
    source_rects: &[(usize, f32, f32, f32, f32)],
) -> Option<ReviewSourceCrop> {
    if !matches!(role, LiquidBlockRole::Table | LiquidBlockRole::Caption) {
        return None;
    }
    let (page_index, _, _, _, _) = *source_rects.first()?;
    let page_rects = source_rects
        .iter()
        .copied()
        .filter(|(page, _, _, _, _)| *page == page_index)
        .collect::<Vec<_>>();
    if page_rects.is_empty() {
        return None;
    }
    let left = page_rects
        .iter()
        .map(|rect| rect.1)
        .fold(f32::INFINITY, f32::min);
    let bottom = page_rects
        .iter()
        .map(|rect| rect.2)
        .fold(f32::INFINITY, f32::min);
    let right = page_rects
        .iter()
        .map(|rect| rect.3)
        .fold(f32::NEG_INFINITY, f32::max);
    let top = page_rects
        .iter()
        .map(|rect| rect.4)
        .fold(f32::NEG_INFINITY, f32::max);
    if !left.is_finite() || right <= left || top <= bottom {
        return None;
    }
    Some(ReviewSourceCrop {
        page_index,
        left,
        bottom,
        right,
        top,
    })
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ReviewDisplayBenchmark {
    pub document: String,
    pub source_retention: f64,
    pub visible_words: usize,
    pub retained_source_words: usize,
    pub criticals_by_category: BTreeMap<String, usize>,
    pub fused_note_first_only: Vec<u16>,
    pub fused_note_visible: Vec<u16>,
    pub fused_note_longest_consecutive: usize,
    pub fused_note_gap_count: usize,
    pub hidden_toc_blocks: usize,
    pub hidden_reprint_titles: usize,
    pub keep_lines_omitted: usize,
}

/// Display-layer Review metrics used as the 3-hour before/after snapshot.
/// Source retention is visible Review words over non-furniture words.
pub fn review_display_benchmark(
    document: &str,
    blocks: &[LiquidBlock],
    keep_line_ids: &[&str],
    assembled_line_ids: &HashSet<String>,
) -> ReviewDisplayBenchmark {
    let hidden = review_hidden_display_mask(blocks);
    let first_only = first_only_note_markers(blocks);
    let visible = visible_review_note_sequence(blocks);
    let (longest, gaps) = note_sequence_run_and_gaps(&visible);
    let mut criticals = BTreeMap::new();
    criticals.insert("note.sequence_gap".to_owned(), gaps);
    criticals.insert(
        "note.collapsed_vs_split".to_owned(),
        visible.len().saturating_sub(first_only.len()),
    );
    let hidden_toc = blocks
        .iter()
        .zip(hidden.iter())
        .filter(|(block, hide)| {
            **hide
                && (is_review_table_of_contents_text(&block.text)
                    || block.role == LiquidBlockRole::Contents)
        })
        .count();
    let hidden_reprints = blocks
        .iter()
        .enumerate()
        .filter(|(index, block)| {
            hidden.get(*index).copied().unwrap_or(false)
                && matches!(
                    block.role,
                    LiquidBlockRole::Heading | LiquidBlockRole::Subheading
                )
                && !is_review_table_of_contents_text(&block.text)
        })
        .count();
    criticals.insert("furniture.toc_leaked".to_owned(), {
        blocks
            .iter()
            .zip(hidden.iter())
            .filter(|(block, hide)| !**hide && is_review_table_of_contents_text(&block.text))
            .count()
    });
    let omitted = omitted_keep_source_ids(keep_line_ids.iter().copied(), assembled_line_ids);
    let (visible_words, retained_source_words) = review_retention_words(blocks, &hidden);
    let source_retention = if retained_source_words == 0 {
        1.0
    } else {
        visible_words as f64 / retained_source_words as f64
    };
    ReviewDisplayBenchmark {
        document: document.to_owned(),
        source_retention,
        visible_words,
        retained_source_words,
        criticals_by_category: criticals,
        fused_note_first_only: first_only,
        fused_note_visible: visible,
        fused_note_longest_consecutive: longest,
        fused_note_gap_count: gaps,
        hidden_toc_blocks: hidden_toc,
        hidden_reprint_titles: hidden_reprints,
        keep_lines_omitted: omitted.len(),
    }
}

pub fn write_review_display_benchmark_json(
    path: &Path,
    snapshot: &ReviewDisplayBenchmark,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let bytes = serde_json::to_vec_pretty(snapshot).map_err(|error| error.to_string())?;
    std::fs::write(path, bytes).map_err(|error| error.to_string())
}

/// Leading marker only — the old Review notes-list collapse.
pub fn first_only_note_markers(blocks: &[LiquidBlock]) -> Vec<u16> {
    blocks
        .iter()
        .filter(|block| is_review_note_display_block(block))
        .filter_map(|block| split_leading_note_marker(block.text.trim()).0)
        .filter_map(|marker| marker.parse().ok())
        .filter(|number| *number > 0)
        .collect()
}

/// Visible Review note numbers after fused-block splitting, in document order.
pub fn visible_review_note_sequence(blocks: &[LiquidBlock]) -> Vec<u16> {
    let hidden = review_hidden_display_mask(blocks);
    let mut numbers = Vec::new();
    for (index, block) in blocks.iter().enumerate() {
        if hidden.get(index).copied().unwrap_or(false) {
            continue;
        }
        if !is_review_note_display_block(block) {
            continue;
        }
        for (marker, _) in split_fused_review_notes(&block.text) {
            if let Ok(number) = marker.parse::<u16>() {
                if number > 0 {
                    numbers.push(number);
                }
            }
        }
    }
    numbers
}

pub fn note_sequence_run_and_gaps(numbers: &[u16]) -> (usize, usize) {
    if numbers.is_empty() {
        return (0, 0);
    }
    let mut longest = 1usize;
    let mut run = 1usize;
    let mut gaps = 0usize;
    for window in numbers.windows(2) {
        if window[1] == window[0].saturating_add(1) {
            run += 1;
            longest = longest.max(run);
        } else {
            if window[1] > window[0].saturating_add(1) {
                gaps += 1;
            }
            run = 1;
        }
    }
    (longest, gaps)
}

fn review_retention_words(blocks: &[LiquidBlock], hidden: &[bool]) -> (usize, usize) {
    let mut visible = 0usize;
    let mut source = 0usize;
    for (index, block) in blocks.iter().enumerate() {
        let furniture = matches!(
            block.role,
            LiquidBlockRole::Header
                | LiquidBlockRole::Footer
                | LiquidBlockRole::Contents
                | LiquidBlockRole::Noise
                | LiquidBlockRole::SectionBreak
        ) || is_review_table_of_contents_text(&block.text);
        let words = block.text.split_whitespace().count();
        if !furniture {
            source += words;
            if !hidden.get(index).copied().unwrap_or(false) {
                visible += words;
            }
        }
    }
    (visible, source)
}

pub fn omitted_keep_source_ids<'a>(
    keep_line_ids: impl IntoIterator<Item = &'a str>,
    assembled_line_ids: &HashSet<String>,
) -> Vec<String> {
    keep_line_ids
        .into_iter()
        .filter(|id| !id.is_empty() && !assembled_line_ids.contains(*id))
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::liquid::LiquidBlockSourceLines;
    use crate::liquid::LiquidSourceLineRef;

    fn span(article_index: usize, confidence: f32) -> ArticleSpan {
        ArticleSpan {
            article_index,
            start_page_index: article_index,
            start_line_index: 0,
            end_page_index: article_index + 1,
            end_line_index: 0,
            confidence,
            title_hint: None,
            evidence: Vec::new(),
        }
    }

    fn block(role: LiquidBlockRole, text: &str) -> LiquidBlock {
        LiquidBlock {
            role,
            text: text.to_owned(),
            label: None,
        }
    }

    #[test]
    fn opening_pages_preview_does_not_require_later_pages() {
        let native = vec![true, true, true, true, false, false];
        let chars = vec![true, true, true, true, false, false];
        assert!(opening_pages_ready_for_review(&native, &chars, 6));
        assert!(!review_all_pages_ready(&native, &chars, 6));
        assert_eq!(review_opening_page_count(6), 4);
        assert!(!opening_pages_ready_for_review(
            &[true, false],
            &[true, false],
            6
        ));
    }

    #[test]
    fn short_documents_need_every_page_before_review_is_ready() {
        let native = vec![true, true];
        let chars = vec![true, true];
        assert!(opening_pages_ready_for_review(&native, &chars, 2));
        assert!(review_all_pages_ready(&native, &chars, 2));
        assert_eq!(review_opening_page_count(2), 2);
    }

    #[test]
    fn wide_window_uses_margin_note_layout() {
        let width = review_margin_width(1600.0, 920.0);
        assert!(width > 0.0);
        assert!(width >= REVIEW_MARGIN_MIN_WIDTH);
        assert_eq!(review_margin_width(900.0, 920.0), 0.0);
    }

    #[test]
    fn review_column_balances_grey_outside_the_full_row() {
        let layout = review_column_layout(1424.0, 920.0, true);
        assert!(layout.margin_width >= REVIEW_MARGIN_MIN_WIDTH);
        assert!(
            (layout.row_width
                - (layout.body_width + 2.0 * (layout.margin_width + REVIEW_MARGIN_GAP)))
                .abs()
                < 0.01
        );
        assert!((layout.side * 2.0 + layout.row_width - 1424.0).abs() < 0.5);
        let hidden = review_column_layout(1424.0, 920.0, false);
        assert_eq!(hidden.margin_width, 0.0);
        assert_eq!(hidden.row_width, hidden.body_width);
    }

    #[test]
    fn fused_harvard_notes_split_into_sequential_entries() {
        let fused = "24 See RIPSTEIN, supra note 17, at 200. 25 See GOLDBERG & ZIPURSKY, supra note 6, at 154–55. 26 This Article shares with the Palsgraf perspective the basic assumption.";
        let parts = split_fused_review_notes(fused);
        assert_eq!(
            parts
                .iter()
                .map(|(marker, _)| marker.as_str())
                .collect::<Vec<_>>(),
            vec!["24", "25", "26"]
        );
        assert!(parts[0].1.starts_with("See RIPSTEIN"));
        assert!(parts[1].1.starts_with("See GOLDBERG"));
        assert!(!parts.iter().any(|(marker, _)| marker == "17"));
        let eight = split_fused_review_notes(
            "7 See BURROWS. 8 162 N.E. 99 (N.Y. 1928). 9 Palsgraf, 162 N.E. at 99.",
        );
        assert_eq!(
            eight
                .iter()
                .map(|(marker, _)| marker.as_str())
                .collect::<Vec<_>>(),
            vec!["7", "8", "9"]
        );
        let continuation = split_fused_review_notes(
            "among representative persons, with respect to the kinds of dangers that we might reasonably foresee happening.”). 49 For a structurally similar example involving battery, see infra notes 173–75.",
        );
        assert_eq!(
            continuation
                .iter()
                .map(|(marker, _)| marker.as_str())
                .collect::<Vec<_>>(),
            vec!["*", "49"]
        );
        assert!(continuation[0].1.contains("among representative persons"));
        let page_glitch = split_fused_review_notes(
            "542. But the law conspicuously declines to impose such liability. 280 See Smith. 281 See Jones. 282 See Brown.",
        );
        assert_eq!(
            page_glitch
                .iter()
                .map(|(marker, _)| marker.as_str())
                .collect::<Vec<_>>(),
            vec!["*", "280", "281", "282"]
        );
        assert!(page_glitch[0].1.contains("conspicuously declines"));
        assert!(!page_glitch[0].1.contains("542"));
        let mid = split_fused_review_notes(
            "foundation of our negligence law.” Id. at 564. 54 See William L. Prosser. 55 See id. 56 See Jones.",
        );
        assert_eq!(
            mid.iter()
                .map(|(marker, _)| marker.as_str())
                .collect::<Vec<_>>(),
            vec!["*", "54", "55", "56"]
        );
        assert!(mid[0].1.contains("foundation of our negligence law"));
        let star = split_fused_review_notes(
            "∗ I am grateful to many friends. This piece is much better than it was before their work. 1 See Sharkey. 2 See Calabresi. 3 Smith New Ct.",
        );
        assert_eq!(star[0].0, "*");
        assert_eq!(
            star.iter()
                .map(|(marker, _)| marker.as_str())
                .collect::<Vec<_>>(),
            vec!["*", "1", "2", "3"]
        );
        let volume_in_body = split_fused_review_notes(
            "125 Compare Frederick Pollock and Frederic William Maitland’s famous remarks. “We and our fathers have got on well enough without such an action.” 2 FREDERICK POLLOCK & FREDERIC WILLIAM MAITLAND, THE HISTORY OF ENGLISH LAW BEFORE THE TIME OF EDWARD I, at 186 (2d ed. 2010).",
        );
        assert_eq!(
            volume_in_body
                .iter()
                .map(|(marker, _)| marker.as_str())
                .collect::<Vec<_>>(),
            vec!["125"]
        );
        let harvard_249 = split_fused_review_notes(
            "249 See Winter. 250 See RIPSTEIN, supra note 17, at 192–95. 251 See generally Bryson Kern, Note, Reputational Injury, 83 FORDHAM L. REV. 253 (2014) (surveying). 252 See, e.g., Kennedy v. McKesson Co., 448 N.E.2d 1332 (N.Y. 1983). 253 See Kern, supra note 251, at 255.",
        );
        assert_eq!(
            harvard_249
                .iter()
                .map(|(marker, _)| marker.as_str())
                .collect::<Vec<_>>(),
            vec!["249", "250", "251", "252", "253"]
        );
        let restatement = split_fused_review_notes(
            "308 See RESTATEMENT (SECOND) OF TORTS § 217 cmt. c (A.L.I. 1965) (“The intention required.”). The defendant has committed trespass to chattels and conversion. 309 See supra notes 49–51 and accompanying text. 310 For versions of this idea, see WEINRIB, supra note 17, at 169 n.53. 311 For a structurally similar case, see Lewinsohn, supra note 48, at 199.",
        );
        assert_eq!(
            restatement
                .iter()
                .map(|(marker, _)| marker.as_str())
                .collect::<Vec<_>>(),
            vec!["308", "309", "310", "311"]
        );
        assert!(noise_block_is_review_note(
            "279 There are, of course, moral accounts of tort law that seek to explain the scope and incidence of both fault-based and strict liability."
        ));
        assert!(!noise_block_is_review_note("2026] WHAT IS A TORT? 1071"));
        assert!(!noise_block_is_review_note(
            "–––––––––––––––––––––––––––––––––––––––––––––––––––––––––––––"
        ));
        assert!(is_review_note_display_block(&block(
            LiquidBlockRole::Noise,
            "279 There are, of course, moral accounts of tort law that seek to explain the scope and incidence of both fault-based and strict liability.",
        )));
    }

    #[test]
    fn fused_split_keeps_hlr_continuation_tails() {
        const BLOCK_47: &str = "among representative persons, with respect to the kinds of dangers that we might reasonably foresee happening.”). For a precise articulation of this generic understanding of the Palsgraf principle, see Jed Lewinsohn, “I Didn’t Know It Was You”: The Impersonal Grounds of Relational Normativity, 59 NOÛS 191, 194–96 (2025). 49 For a structurally similar example involving battery, see infra notes 173–75 and accompanying text.";
        const BLOCK_58: &str = "foundation of our negligence law.” Id. at 564. This position is, on a natural interpretation, congruent with this Article’s claim that the normative principles underlying tort law are substantially continuous across common law and civil law systems. 54 See infra section II.A, pp. 1035–46. 55 See William L. Prosser, Transferred Intent, 45 TEX. L. REV. 650, 650 (1967) (quoting State v. Batson, 96 S.W.2d 384, 389 (Mo. 1936)). 56 See Palsgraf, 162 N.E. at 101. 57 See id. at 100. 58 Prosser, supra note 55, at 650.";
        const BLOCK_112: &str = "principles they implement, are considered in section II.F, pp. 1067–76. Until then, this Article principally focuses on the paradigm of liability-for-rights infringement through culpable wrongdoing that (it argues) underlies both civil law general clauses such as CC 2043 and BGB section 823(1) and much of common law negligence and battery doctrine. 104 On the centrality of negligence and negligence-like forms of products liability in common law tort, see generally James A. Henderson, Jr., Why Negligence Dominates Tort, 50 UCLA L. REV. 377 (2002).";
        const BLOCK_330: &str = "542. But the law conspicuously declines to impose such liability except in limited domains. 280 See, e.g., Richard A. Epstein, A Theory of Strict Liability, 2 J. LEGAL STUD. 151, 160–61 (1973). 281 See, e.g., Larry Alexander & Kimberly Kessler Ferzan, Confused Culpability, Contrived Causation, and the Collapse of Tort Theory, in PHILOSOPHICAL FOUNDATIONS OF THE LAW OF TORTS, supra note 39, at 416–25. 282 WIEACKER, supra note 124, at 257.";
        const NOTE_279: &str = "279 There are, of course, moral accounts of tort law that seek to explain the scope and incidence of both fault-based and strict liability on the basis of a unified, consistent set of moral principles. See, e.g., Fletcher, supra note 272, at 556; Stein, supra note 272, at 611. But such accounts do not seem to succeed in capturing even the broad contours of the doctrine. Fletcher’s account, for example, implies that justifiably imposing a five percent risk of serious property damage on another person should incur liability, given that it is a “nonreciprocal” risk. See Fletcher, supra note 272, at";

        let forty_eight = split_fused_review_notes(BLOCK_47);
        assert_eq!(forty_eight[0].0, "*");
        assert!(
            forty_eight[0].1.contains("Lewinsohn"),
            "note 48 tail must stay visible: {:?}",
            forty_eight[0].1
        );
        assert!(forty_eight[0].1.contains("among representative persons"));
        assert_eq!(forty_eight[1].0, "49");
        assert!(forty_eight[1].1.starts_with("For a structurally similar"));

        let fifty_three = split_fused_review_notes(BLOCK_58);
        assert_eq!(fifty_three[0].0, "*");
        assert!(
            fifty_three[0]
                .1
                .contains("foundation of our negligence law"),
            "note 53 tail must stay visible: {:?}",
            fifty_three[0].1
        );
        assert_eq!(
            fifty_three
                .iter()
                .map(|(marker, _)| marker.as_str())
                .collect::<Vec<_>>(),
            vec!["*", "54", "55", "56", "57", "58"]
        );

        let one_oh_three = split_fused_review_notes(BLOCK_112);
        assert_eq!(one_oh_three[0].0, "*");
        assert!(
            one_oh_three[0].1.contains("abnormally dangerous activity")
                || one_oh_three[0].1.contains("principles they implement"),
            "note 103 tail must stay visible: {:?}",
            one_oh_three[0].1
        );
        assert_eq!(one_oh_three[1].0, "104");

        let glitch = split_fused_review_notes(BLOCK_330);
        assert_eq!(
            glitch
                .iter()
                .map(|(marker, _)| marker.as_str())
                .collect::<Vec<_>>(),
            vec!["*", "280", "281", "282"]
        );
        assert!(
            glitch[0]
                .1
                .contains("conspicuously declines to impose such liability"),
            "542-glitch body is the tail of 279: {:?}",
            glitch[0].1
        );
        assert!(!glitch.iter().any(|(marker, _)| marker == "542"));

        let rescued = block(LiquidBlockRole::Noise, NOTE_279);
        let follow = block(LiquidBlockRole::Marginalia, BLOCK_330);
        let header = block(LiquidBlockRole::Noise, "2026] WHAT IS A TORT? 1071");
        let body = block(
            LiquidBlockRole::Paragraph,
            "Defenders of the Palsgraf perspective.",
        );
        assert!(!review_skips_block_as_furniture(&rescued, true));
        assert!(review_skips_block_as_furniture(&header, false));
        let blocks = vec![rescued, follow, header, body];
        let hidden = review_hidden_display_mask(&blocks);
        let (notes, next) = review_collect_margin_note_indices(&blocks, 0, &hidden);
        assert_eq!(
            notes,
            vec![0, 1],
            "rescued 279 must enter the margin run with 280–282"
        );
        assert_eq!(next, 3);
        assert!(is_review_margin_note_block(&blocks[0]));
    }

    #[test]
    fn repeated_article_title_is_hidden_from_review_display() {
        let blocks = vec![
            block(LiquidBlockRole::Title, "WHAT IS A TORT? Ketan Ramakrishnan"),
            block(LiquidBlockRole::Heading, "ARTICLE"),
            block(
                LiquidBlockRole::Heading,
                "WHAT IS A TORT? Ketan Ramakrishnan∗",
            ),
            block(LiquidBlockRole::Heading, "INTRODUCTION"),
        ];
        assert_eq!(
            review_hidden_display_mask(&blocks),
            vec![false, false, true, false]
        );
    }

    #[test]
    fn high_confidence_spans_revoke_global_note_starts() {
        let scoped = HashSet::from(["a1:n1".to_owned()]);
        let global = HashSet::from(["a1:n1".to_owned(), "impostor".to_owned()]);
        let spans = vec![
            span(0, REVIEW_HIGH_CONFIDENCE_ARTICLE_SPAN),
            span(1, REVIEW_HIGH_CONFIDENCE_ARTICLE_SPAN),
        ];
        let merged = merge_article_and_global_note_starts(scoped, global, &spans);
        assert!(merged.contains("a1:n1"));
        assert!(!merged.contains("impostor"));
    }

    #[test]
    fn single_article_keeps_global_note_starts() {
        let scoped = HashSet::from(["n1".to_owned(), "n2".to_owned()]);
        let global = scoped.clone();
        let spans = vec![span(0, REVIEW_HIGH_CONFIDENCE_ARTICLE_SPAN)];
        let merged = merge_article_and_global_note_starts(scoped.clone(), global, &spans);
        assert_eq!(merged, scoped);
        assert!(!article_spans_may_revoke_global_note_starts(&spans));
    }

    #[test]
    fn low_confidence_spans_do_not_revoke() {
        let scoped = HashSet::from(["keep".to_owned()]);
        let global = HashSet::from(["keep".to_owned(), "global".to_owned()]);
        let spans = vec![span(0, 0.4), span(1, 0.4)];
        let merged = merge_article_and_global_note_starts(scoped, global, &spans);
        assert!(merged.contains("global"));
    }

    #[test]
    fn find_and_plain_text_use_review_blocks() {
        let document = LiquidDocument {
            title: "Title".to_owned(),
            blocks: vec![
                block(LiquidBlockRole::Paragraph, "The holding on standing."),
                block(LiquidBlockRole::Noise, "Page 12"),
                block(LiquidBlockRole::Marginalia, "12 See standing cases."),
            ],
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
            source_signature: "test".to_owned(),
        };
        let hits = find_hits_in_review_blocks(&document.blocks, "standing");
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].block_index, 0);
        assert_eq!(hits[1].block_index, 2);
        let text = review_document_plain_text(&document);
        assert!(text.contains("The holding on standing."));
        assert!(text.contains("12 See standing cases."));
        assert!(!text.contains("Page 12"));
    }

    #[test]
    fn live_correction_mutates_the_assembled_review_document() {
        let mut document = LiquidDocument {
            title: "Title".to_owned(),
            blocks: vec![block(
                LiquidBlockRole::Paragraph,
                "1 See the cited statute.",
            )],
            article_spans: Vec::new(),
            block_source_lines: vec![LiquidBlockSourceLines {
                block_index: 0,
                lines: vec![LiquidSourceLineRef {
                    id: Some("p0:l0".to_owned()),
                    page_index: 0,
                    line_index: 0,
                    text: "1 See the cited statute.".to_owned(),
                    role: LiquidBlockRole::Paragraph,
                    note_markers: vec![1],
                }],
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
            source_signature: "test".to_owned(),
        };
        assert!(apply_live_review_correction(
            &mut document,
            0,
            LiquidBlockRole::Marginalia
        ));
        assert_eq!(document.blocks[0].role, LiquidBlockRole::Marginalia);
        assert_eq!(
            document.block_source_lines[0].lines[0].role,
            LiquidBlockRole::Marginalia
        );
    }

    #[test]
    fn table_figure_crop_unions_source_rects_on_the_same_page() {
        let crop = review_table_figure_crop(
            LiquidBlockRole::Table,
            &[(2, 0.10, 0.20, 0.40, 0.35), (2, 0.12, 0.18, 0.80, 0.50)],
        )
        .expect("table crop");
        assert_eq!(crop.page_index, 2);
        assert!((crop.left - 0.10).abs() < 1e-6);
        assert!((crop.bottom - 0.18).abs() < 1e-6);
        assert!((crop.right - 0.80).abs() < 1e-6);
        assert!((crop.top - 0.50).abs() < 1e-6);
        assert!(
            review_table_figure_crop(LiquidBlockRole::Paragraph, &[(2, 0.1, 0.2, 0.3, 0.4)])
                .is_none()
        );
    }

    #[test]
    fn omitted_keep_ids_are_the_keep_lines_missing_from_assembly() {
        let assembled = HashSet::from(["keep-a".to_owned()]);
        let omitted = omitted_keep_source_ids(["keep-a", "keep-b", ""], &assembled);
        assert_eq!(omitted, vec!["keep-b".to_owned()]);
    }

    #[test]
    fn fused_law_review_toc_is_hidden_from_review_display() {
        let fused = "A. Two Entitlements................................................................................................. 13 B. A Typology of Nationalization............................................................................. 15 C. Control and claim as spectrums ........................................................................... 16 II. Should We Nationalize AI?....................................................................................... 18";
        assert!(fused_toc_page_locator_count(fused) >= 3);
        assert!(is_review_table_of_contents_text(fused));
        let blocks = vec![
            block(LiquidBlockRole::Title, "AI Nationalization"),
            block(LiquidBlockRole::Paragraph, fused),
            block(LiquidBlockRole::Heading, "Introduction"),
        ];
        let mask = review_hidden_display_mask(&blocks);
        assert_eq!(mask, vec![false, true, false]);
        assert!(!is_review_table_of_contents_text(
            "Introduction. This Article fills the gap without any leader dots."
        ));
        assert!(
            !is_review_table_of_contents_text(
                "308 See RESTATEMENT (SECOND) OF TORTS § 217 cmt. c (A.L.I. 1965) (“The intention required to make an actor liable for trespass to a chattel . . . is present when an act is done for the purpose of using or otherwise intermeddling with a chattel . . . .”)."
            ),
            "legal ellipses must not hide a fused note as a TOC"
        );
        assert_eq!(
            compound_toc_entries(fused),
            vec![
                "A. Two Entitlements",
                "B. A Typology of Nationalization",
                "C. Control and claim as spectrums",
                "II. Should We Nationalize AI?",
            ]
        );
        let entries = review_contents_navigation_entries(&blocks);
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.title.as_str())
                .collect::<Vec<_>>(),
            vec![
                "A. Two Entitlements",
                "B. A Typology of Nationalization",
                "C. Control and claim as spectrums",
                "II. Should We Nationalize AI?",
            ]
        );
    }

    #[test]
    fn single_dot_leader_toc_row_is_hidden() {
        assert!(is_review_table_of_contents_text(
            "III. A Design for Minimalist AI Nationalization........................................................ 49"
        ));
        assert!(is_review_table_of_contents_text("Table of Contents"));
    }

    #[test]
    fn short_documents_spawn_one_full_job_never_a_preview() {
        assert_eq!(
            review_prepare_next_action(3, true, true, false, false),
            ReviewPrepareAction::SpawnFull { page_count: 3 }
        );
        assert_eq!(
            review_prepare_next_action(3, true, true, false, true),
            ReviewPrepareAction::Nothing
        );
        assert_eq!(
            review_prepare_next_action(3, false, false, false, false),
            ReviewPrepareAction::WaitForPages
        );
    }

    #[test]
    fn long_documents_preview_then_full_without_double_preview() {
        assert_eq!(
            review_prepare_next_action(20, true, false, false, false),
            ReviewPrepareAction::SpawnPreview { page_count: 4 }
        );
        assert_eq!(
            review_prepare_next_action(20, true, false, true, false),
            ReviewPrepareAction::WaitForPages
        );
        assert_eq!(
            review_prepare_next_action(20, true, true, true, false),
            ReviewPrepareAction::SpawnFull { page_count: 20 }
        );
        assert_eq!(
            review_prepare_next_action(20, true, true, true, true),
            ReviewPrepareAction::Nothing
        );
    }

    #[test]
    fn finished_review_ignores_a_late_preview_event() {
        assert!(!should_apply_review_event(true, false));
        assert!(should_apply_review_event(false, false));
        assert!(should_apply_review_event(true, true));
        assert!(should_apply_review_event(false, true));
    }

    #[test]
    fn review_open_does_not_precompute_and_large_files_wait_for_full() {
        assert!(!should_precompute_review_on_open(0));
        assert!(!should_precompute_review_on_open(1));
        assert!(!should_precompute_review_on_open(40));
        assert!(!should_precompute_review_on_open(500));
        assert!(review_allows_automatic_full_prepare(12));
        assert!(!review_allows_automatic_full_prepare(32));
        assert!(!review_allows_automatic_full_prepare(80));
        assert_eq!(
            review_gate_automatic_full(
                ReviewPrepareAction::SpawnFull { page_count: 80 },
                80,
                false
            ),
            ReviewPrepareAction::Nothing
        );
        assert_eq!(
            review_gate_automatic_full(ReviewPrepareAction::SpawnFull { page_count: 80 }, 80, true),
            ReviewPrepareAction::SpawnFull { page_count: 80 }
        );
        assert_eq!(
            review_gate_automatic_full(
                ReviewPrepareAction::SpawnPreview { page_count: 4 },
                80,
                false
            ),
            ReviewPrepareAction::SpawnPreview { page_count: 4 }
        );
        assert_eq!(review_footnote_superscript("1"), "¹");
        assert_eq!(review_footnote_superscript("12"), "¹²");
        assert_eq!(
            review_paragraph_display_parts("First paragraph.\n\nSecond paragraph."),
            vec!["First paragraph.", "Second paragraph."]
        );
    }

    #[test]
    fn retry_clears_spawn_flags_so_prepare_can_run_again() {
        let after_full = ReviewPrepareFlags {
            preview_spawned: true,
            full_spawned: true,
            complete: true,
            pdf_split: true,
        };
        let reset = review_prepare_flags_after_restart();
        assert_eq!(reset, ReviewPrepareFlags::default());
        assert_eq!(
            review_prepare_next_action(3, true, true, reset.preview_spawned, reset.full_spawned),
            ReviewPrepareAction::SpawnFull { page_count: 3 }
        );
        assert_eq!(
            review_prepare_next_action(
                20,
                true,
                true,
                after_full.preview_spawned,
                after_full.full_spawned
            ),
            ReviewPrepareAction::Nothing
        );
    }

    fn harvard_tort_fixture_blocks() -> Vec<LiquidBlock> {
        if let Ok(path) = std::env::var("LAWPDF_FIXTURE_JSON") {
            if let Ok(bytes) = std::fs::read(&path) {
                if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                    if let Some(blocks) = value.get("blocks").and_then(|item| item.as_array()) {
                        return blocks
                            .iter()
                            .filter_map(|entry| {
                                let role = match entry.get("role")?.as_str()? {
                                    "title" | "Title" => LiquidBlockRole::Title,
                                    "heading" | "Heading" => LiquidBlockRole::Heading,
                                    "subheading" | "Subheading" => LiquidBlockRole::Subheading,
                                    "paragraph" | "Paragraph" => LiquidBlockRole::Paragraph,
                                    "marginalia" | "Marginalia" => LiquidBlockRole::Marginalia,
                                    "footnote" | "Footnote" => LiquidBlockRole::Footnote,
                                    "noise" | "Noise" => LiquidBlockRole::Noise,
                                    "contents" | "Contents" => LiquidBlockRole::Contents,
                                    _ => LiquidBlockRole::Paragraph,
                                };
                                Some(block(role, entry.get("text")?.as_str()?))
                            })
                            .collect();
                    }
                }
            }
        }
        vec![
            block(LiquidBlockRole::Title, "WHAT IS A TORT? Ketan Ramakrishnan"),
            block(LiquidBlockRole::Heading, "ARTICLE"),
            block(
                LiquidBlockRole::Heading,
                "WHAT IS A TORT? Ketan Ramakrishnan∗",
            ),
            block(
                LiquidBlockRole::Paragraph,
                "INTRODUCTION ........................................................................ 1 II. THE PALSGRAF PERSPECTIVE ........................ 8",
            ),
            block(
                LiquidBlockRole::Marginalia,
                "24 See RIPSTEIN, supra note 17, at 200. 25 See GOLDBERG & ZIPURSKY, supra note 6, at 154–55. 26 This Article shares the assumption.",
            ),
            block(
                LiquidBlockRole::Marginalia,
                "7 See BURROWS. 8 162 N.E. 99 (N.Y. 1928). 9 Palsgraf, 162 N.E. at 99.",
            ),
        ]
    }

    #[test]
    fn review_display_benchmark_drives_shipped_helpers() {
        let blocks = harvard_tort_fixture_blocks();
        let assembled = HashSet::from(["keep-a".to_owned()]);
        let snapshot = review_display_benchmark(
            "harvard-tort-fixture",
            &blocks,
            &["keep-a", "keep-b"],
            &assembled,
        );
        assert!(snapshot.source_retention > 0.0);
        assert!(!snapshot.fused_note_visible.is_empty());
        assert!(
            snapshot.fused_note_visible.contains(&25),
            "shipped splitter must expose 25, not only 24: {:?}",
            snapshot.fused_note_visible
        );
        assert!(
            snapshot
                .criticals_by_category
                .contains_key("note.sequence_gap")
        );
        if let Ok(path) = std::env::var("LAWPDF_BENCH_OUT") {
            write_review_display_benchmark_json(Path::new(&path), &snapshot)
                .expect("write snapshot");
        }
    }
}
