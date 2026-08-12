//! Pure Review Mode helpers: opening-page preview, margin layout, article
//! search, live corrections, and table/figure crop geometry.
//!
//! These functions are the shipped decisions used by the UI and LM2 prepare
//! path. Keep them free of Pdfium and native-model I/O so tests can drive them
//! directly.

use std::collections::HashSet;

use crate::liquid::{
    hidden_contents_mask_for_display, ArticleSpan, LiquidBlock, LiquidBlockRole, LiquidDocument,
};

pub const REVIEW_OPENING_PAGE_LIMIT: usize = 4;
pub const REVIEW_MARGIN_MIN_WIDTH: f32 = 168.0;
pub const REVIEW_MARGIN_MAX_WIDTH: f32 = 260.0;
pub const REVIEW_MARGIN_GAP: f32 = 16.0;
pub const REVIEW_HIGH_CONFIDENCE_ARTICLE_SPAN: f32 = 3.0;
pub const REVIEW_OUTLINE_RAIL_DEFAULT_WIDTH: f32 = 236.0;

/// Hide printed tables of contents from the Review reading column.
pub fn review_hidden_display_mask(blocks: &[LiquidBlock]) -> Vec<bool> {
    let mut mask = hidden_contents_mask_for_display(blocks);
    for (index, block) in blocks.iter().enumerate() {
        if is_review_table_of_contents_text(&block.text) {
            mask[index] = true;
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
            .find(|(_, ch)| {
                !ch.is_ascii_digit()
                    && !matches!(ch.to_ascii_lowercase(), 'i' | 'v' | 'x' | 'l' | 'c')
            })
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
    if available_width <= 0.0 || body_width <= 0.0 {
        return 0.0;
    }
    let needed = body_width + 2.0 * (REVIEW_MARGIN_MIN_WIDTH + REVIEW_MARGIN_GAP);
    if available_width + 0.5 < needed {
        return 0.0;
    }
    let leftover = (available_width - body_width) / 2.0 - REVIEW_MARGIN_GAP;
    leftover.clamp(REVIEW_MARGIN_MIN_WIDTH, REVIEW_MARGIN_MAX_WIDTH)
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
}
