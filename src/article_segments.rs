//! Conservative article-boundary detection for PDFs that may contain bound
//! journal volumes.
//!
//! The detector operates on page-local observations rather than LM2 internals
//! so the same scoring contract can be reproduced by corpus tooling. Page zero
//! is always a boundary. Later boundaries require independent evidence or a
//! strong footnote-number reset anchored to a plausible title page.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::liquid::{ArticleBoundaryEvidence, ArticleSpan};

const MIN_EDGE_ARTICLE_PAGES: usize = 3;
const MIN_BETWEEN_BOUNDARIES_PAGES: usize = 2;
const BOUNDARY_SCORE_THRESHOLD: f32 = 3.0;
const RESET_LOOKBACK_PAGES: usize = 6;
const PRIOR_NOTE_LOOKBACK_PAGES: usize = 12;
const MAX_NOTE_MARKER: u16 = 999;
pub(crate) const ARTICLE_SEGMENTATION_VERSION: &str = "article-segments-v1";

#[derive(Debug, Clone)]
pub(crate) struct ArticleSegmentationLine {
    pub page_index: usize,
    pub line_index: usize,
    pub text: String,
    pub font_ratio_page: f32,
    pub font_ratio_doc: f32,
    pub margin_centered: bool,
    pub line_width_ratio: f32,
    pub top: f32,
    pub page_height: f32,
    pub repeated_edge_text: bool,
    pub toc_like: bool,
    pub note_marker: Option<u16>,
    pub marginalia: bool,
}

#[derive(Debug, Clone)]
struct PageObservation {
    page_index: usize,
    title_score: f32,
    title_hint: Option<String>,
    title_evidence: Vec<ArticleBoundaryEvidence>,
    note_markers: Vec<u16>,
}

#[derive(Debug, Clone)]
struct BoundaryCandidate {
    page_index: usize,
    line_index: usize,
    score: f32,
    title_hint: Option<String>,
    evidence: Vec<ArticleBoundaryEvidence>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ArticleBoundaryCandidateTrace {
    pub page_index: usize,
    pub line_index: usize,
    pub score: f32,
    pub selected: bool,
    pub title_hint: Option<String>,
    pub evidence: Vec<ArticleBoundaryEvidence>,
}

pub(crate) fn detect_article_spans(
    lines: &[ArticleSegmentationLine],
    page_count: usize,
) -> Vec<ArticleSpan> {
    detect_article_spans_with_trace(lines, page_count).0
}

pub(crate) fn detect_article_spans_with_trace(
    lines: &[ArticleSegmentationLine],
    page_count: usize,
) -> (Vec<ArticleSpan>, Vec<ArticleBoundaryCandidateTrace>) {
    if page_count == 0 {
        return (Vec::new(), Vec::new());
    }
    if lines.is_empty() {
        return (vec![single_span(page_count)], Vec::new());
    }

    let pages = observe_pages(lines, page_count);
    let mut candidates = pages
        .iter()
        .skip(1)
        .map(|page| BoundaryCandidate {
            page_index: page.page_index,
            line_index: 0,
            score: page.title_score,
            title_hint: page.title_hint.clone(),
            evidence: page.title_evidence.clone(),
        })
        .collect::<Vec<_>>();
    apply_note_reset_evidence(&pages, &mut candidates);
    candidates.extend(midpage_boundary_candidates(lines));
    attach_opening_front_matter_to_first_article(lines, &mut candidates, page_count);
    require_journal_issue_identity(lines, &mut candidates);
    let boundaries = select_boundaries(candidates.clone(), page_count);
    let traces = candidates
        .into_iter()
        .map(|candidate| ArticleBoundaryCandidateTrace {
            page_index: candidate.page_index,
            line_index: candidate.line_index,
            score: candidate.score,
            selected: boundaries.iter().any(|selected| {
                selected.page_index == candidate.page_index
                    && selected.line_index == candidate.line_index
            }),
            title_hint: candidate.title_hint,
            evidence: candidate.evidence,
        })
        .collect();
    (
        spans_from_boundaries(&pages, &boundaries, page_count),
        traces,
    )
}

fn single_span(page_count: usize) -> ArticleSpan {
    ArticleSpan {
        article_index: 0,
        start_page_index: 0,
        start_line_index: 0,
        end_page_index: page_count,
        end_line_index: 0,
        confidence: 1.0,
        title_hint: None,
        evidence: vec![ArticleBoundaryEvidence {
            kind: "document_start".to_owned(),
            score: 1.0,
            detail: "PDF starts here".to_owned(),
        }],
    }
}

fn require_journal_issue_identity(
    lines: &[ArticleSegmentationLine],
    candidates: &mut [BoundaryCandidate],
) {
    if lines.iter().any(|line| {
        let lower = collapse_whitespace(&line.text).to_ascii_lowercase();
        [
            "law review",
            "law journal",
            "legal journal",
            "law quarterly",
            "bar journal",
        ]
        .iter()
        .any(|identity| lower.contains(identity))
    }) {
        return;
    }
    for candidate in candidates
        .iter_mut()
        .filter(|candidate| candidate.score >= BOUNDARY_SCORE_THRESHOLD)
    {
        let removed_score = candidate.score;
        candidate.score = 0.0;
        candidate.evidence.push(ArticleBoundaryEvidence {
            kind: "non_journal_document".to_owned(),
            score: -removed_score,
            detail: "article segmentation requires positive journal-issue identity".to_owned(),
        });
    }
}

fn observe_pages(lines: &[ArticleSegmentationLine], page_count: usize) -> Vec<PageObservation> {
    let mut by_page: BTreeMap<usize, Vec<&ArticleSegmentationLine>> = BTreeMap::new();
    for line in lines {
        by_page.entry(line.page_index).or_default().push(line);
    }
    let recurring_top_text = recurring_top_text(lines);

    (0..page_count)
        .map(|page_index| {
            let mut page_lines = by_page.remove(&page_index).unwrap_or_default();
            page_lines.sort_by_key(|line| line.line_index);
            observe_page(page_index, &page_lines, &recurring_top_text)
        })
        .collect()
}

fn observe_page(
    page_index: usize,
    page_lines: &[&ArticleSegmentationLine],
    recurring_top_text: &BTreeMap<String, (f32, f32)>,
) -> PageObservation {
    let content = page_lines
        .iter()
        .copied()
        .filter(|line| !line.text.trim().is_empty() && !furniture_like(line, recurring_top_text))
        .take(14)
        .collect::<Vec<_>>();
    let mut evidence = Vec::new();
    let mut score = 0.0f32;

    let title_lines = content
        .iter()
        .copied()
        .take(8)
        .filter(|line| title_line_like(line))
        .collect::<Vec<_>>();
    let title_hint = title_lines
        .iter()
        .take(3)
        .map(|line| collapse_whitespace(&line.text))
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let title_hint = (!title_hint.is_empty()).then_some(title_hint);

    let publication_section = content
        .iter()
        .take(3)
        .filter(|line| uppercase_ratio(&line.text) >= 0.68)
        .map(|line| collapse_whitespace(&line.text))
        .find_map(|text| publication_section_heading(&text).map(|_| text));
    if let Some(section) = publication_section.as_ref() {
        score += 3.1;
        evidence.push(ArticleBoundaryEvidence {
            kind: "publication_section".to_owned(),
            score: 3.1,
            detail: section.clone(),
        });
    }

    if !title_lines.is_empty() {
        let strongest_ratio = title_lines
            .iter()
            .map(|line| effective_font_ratio(line))
            .fold(0.0f32, f32::max);
        let typography_score = if strongest_ratio >= 1.55 {
            2.0
        } else if strongest_ratio >= 1.30 {
            1.5
        } else if strongest_ratio >= 1.14 {
            1.0
        } else {
            0.5
        };
        score += typography_score;
        evidence.push(ArticleBoundaryEvidence {
            kind: "title_typography".to_owned(),
            score: typography_score,
            detail: format!(
                "{} title-like top line(s), strongest font ratio {:.2}",
                title_lines.len(),
                strongest_ratio
            ),
        });
    }

    let centered_titles = title_lines
        .iter()
        .filter(|line| line.margin_centered)
        .count();
    if centered_titles > 0 {
        let centered_score = if centered_titles >= 2 { 1.0 } else { 0.65 };
        score += centered_score;
        evidence.push(ArticleBoundaryEvidence {
            kind: "centered_title".to_owned(),
            score: centered_score,
            detail: format!("{centered_titles} centered title line(s)"),
        });
    }

    let uppercase_titles = title_lines
        .iter()
        .filter(|line| uppercase_ratio(&line.text) >= 0.72)
        .count();
    if uppercase_titles > 0 {
        let uppercase_score = if uppercase_titles >= 2 { 0.9 } else { 0.55 };
        score += uppercase_score;
        evidence.push(ArticleBoundaryEvidence {
            kind: "uppercase_title".to_owned(),
            score: uppercase_score,
            detail: format!("{uppercase_titles} uppercase title line(s)"),
        });
    }

    if title_lines.len() >= 2
        && title_lines
            .iter()
            .take(2)
            .all(|line| uppercase_ratio(&line.text) >= 0.68)
    {
        score += 1.15;
        evidence.push(ArticleBoundaryEvidence {
            kind: "stacked_uppercase_title".to_owned(),
            score: 1.15,
            detail: "two title-like uppercase lines at the top of the page".to_owned(),
        });
    }

    if let Some(first_title) = title_lines.first()
        && content
            .first()
            .is_some_and(|first| std::ptr::eq(*first, *first_title))
        && normalized_top(first_title) <= 0.82
        && uppercase_ratio(&first_title.text) >= 0.68
    {
        score += 0.6;
        evidence.push(ArticleBoundaryEvidence {
            kind: "title_top_margin".to_owned(),
            score: 0.6,
            detail: "substantial top margin before an opening uppercase title".to_owned(),
        });
    }

    if let Some(byline) = content.iter().copied().skip(1).take(9).find(|line| {
        !title_line_like(line)
            && (byline_like(&line.text) || author_signature_like(line))
            && effective_font_ratio(line) <= 1.18
            && !line.repeated_edge_text
    }) {
        score += 0.8;
        evidence.push(ArticleBoundaryEvidence {
            kind: "byline".to_owned(),
            score: 0.8,
            detail: collapse_whitespace(&byline.text),
        });
        if !title_lines.is_empty() {
            score += 0.75;
            evidence.push(ArticleBoundaryEvidence {
                kind: "title_byline_pair".to_owned(),
                score: 0.75,
                detail: "title typography followed by a plausible author line".to_owned(),
            });
        }
    }

    if let Some(author) = content.iter().copied().skip(1).take(6).find(|line| {
        collapse_whitespace(&line.text)
            .to_ascii_lowercase()
            .starts_with("by ")
            && word_count(&line.text) <= 8
    }) {
        score += 1.1;
        evidence.push(ArticleBoundaryEvidence {
            kind: "explicit_author_line".to_owned(),
            score: 1.1,
            detail: collapse_whitespace(&author.text),
        });
    }

    let large_top_gap = content.windows(2).any(|pair| {
        let first = pair[0];
        let second = pair[1];
        title_line_like(first)
            && normalized_top(first) >= 0.62
            && (first.top - second.top).abs() / first.page_height.max(1.0) >= 0.055
    });
    if large_top_gap {
        score += 0.45;
        evidence.push(ArticleBoundaryEvidence {
            kind: "title_body_gap".to_owned(),
            score: 0.45,
            detail: "large vertical gap below a title-like line".to_owned(),
        });
    }

    if let Some(first) = content.first()
        && continuation_opening_like(first)
    {
        score -= 1.4;
        evidence.push(ArticleBoundaryEvidence {
            kind: "continuation_penalty".to_owned(),
            score: -1.4,
            detail: "page opens with body-sized prose".to_owned(),
        });
    }

    if title_hint.as_deref().is_some_and(internal_heading_only) && title_lines.len() <= 1 {
        score -= 1.8;
        evidence.push(ArticleBoundaryEvidence {
            kind: "internal_heading_penalty".to_owned(),
            score: -1.8,
            detail: "top line resembles an internal section heading".to_owned(),
        });
    }

    if title_lines
        .first()
        .is_some_and(|line| section_prefixed_heading(&line.text))
    {
        score -= 5.0;
        evidence.push(ArticleBoundaryEvidence {
            kind: "section_prefix_penalty".to_owned(),
            score: -5.0,
            detail: "title begins with an internal section number".to_owned(),
        });
    }

    if title_hint.as_deref().is_some_and(running_folio_heading) {
        score -= 5.0;
        evidence.push(ArticleBoundaryEvidence {
            kind: "running_folio_heading_penalty".to_owned(),
            score: -5.0,
            detail: "running folio is not an article title".to_owned(),
        });
    }

    if title_hint.as_deref().is_some_and(appendix_heading) {
        score -= 5.0;
        evidence.push(ArticleBoundaryEvidence {
            kind: "appendix_penalty".to_owned(),
            score: -5.0,
            detail: "appendix material continues the current article".to_owned(),
        });
    }

    if title_hint
        .as_deref()
        .is_some_and(index_or_bibliography_heading)
    {
        score -= 5.0;
        evidence.push(ArticleBoundaryEvidence {
            kind: "index_or_bibliography_penalty".to_owned(),
            score: -5.0,
            detail: "index or bibliography material remains in the current article tail".to_owned(),
        });
    }

    if title_hint.as_deref().is_some_and(lowercase_prose_heading) {
        score -= 5.0;
        evidence.push(ArticleBoundaryEvidence {
            kind: "lowercase_prose_heading_penalty".to_owned(),
            score: -5.0,
            detail: "lowercase prose continuation is not an article title".to_owned(),
        });
    }

    if title_hint.as_deref().is_some_and(table_or_figure_heading) {
        score -= 4.0;
        evidence.push(ArticleBoundaryEvidence {
            kind: "table_or_figure_penalty".to_owned(),
            score: -4.0,
            detail: "table or figure heading is internal article material".to_owned(),
        });
    }

    if title_hint.as_deref().is_some_and(|hint| {
        matches!(
            collapse_whitespace(hint)
                .trim_matches(|ch: char| matches!(ch, ' ' | '.' | ':'))
                .to_ascii_lowercase()
                .as_str(),
            "rule" | "figure" | "table"
        )
    }) {
        score -= 1.5;
        evidence.push(ArticleBoundaryEvidence {
            kind: "generic_heading_penalty".to_owned(),
            score: -1.5,
            detail: "one-word diagram or table heading".to_owned(),
        });
    }
    if publication_section.is_none()
        && title_hint
            .as_deref()
            .is_some_and(|hint| word_count(hint) <= 2)
    {
        score -= 3.0;
        evidence.push(ArticleBoundaryEvidence {
            kind: "short_generic_heading_penalty".to_owned(),
            score: -3.0,
            detail: "very short heading lacks article-level evidence".to_owned(),
        });
    }

    let masthead_lines = content
        .iter()
        .take(14)
        .map(|line| collapse_whitespace(&line.text).to_ascii_lowercase())
        .collect::<Vec<_>>();
    let masthead = masthead_lines
        .iter()
        .any(|text| text.contains("board of editors") || text.contains("editorial board"))
        || masthead_lines
            .iter()
            .filter(|text| text.contains("editor"))
            .count()
            >= 2;
    let journal_identity = masthead_lines
        .iter()
        .take(6)
        .any(|text| text.contains("law journal") || text.contains("law review"));
    if page_index <= 5 && masthead {
        score -= 4.0;
        evidence.push(ArticleBoundaryEvidence {
            kind: "opening_masthead_penalty".to_owned(),
            score: -4.0,
            detail: "opening pages contain an editorial-board masthead".to_owned(),
        });
    } else if page_index > 5 && masthead && journal_identity {
        score += 5.0;
        evidence.push(ArticleBoundaryEvidence {
            kind: "inner_issue_masthead".to_owned(),
            score: 5.0,
            detail: "interior journal masthead starts a new bound issue".to_owned(),
        });
    }

    let first_title_position = content
        .iter()
        .position(|line| title_line_like(line))
        .unwrap_or(content.len());
    if publication_section.is_none()
        && content.iter().take(first_title_position).any(|line| {
            word_count(&line.text) >= 6
                && uppercase_ratio(&line.text) < 0.55
                && !line.margin_centered
        })
    {
        score -= 2.0;
        evidence.push(ArticleBoundaryEvidence {
            kind: "body_before_title_penalty".to_owned(),
            score: -2.0,
            detail: "body prose precedes the title-like material on the page".to_owned(),
        });
    }

    let toc_lines = content.iter().filter(|line| line.toc_like).count();
    let strongest_title_ratio = title_lines
        .iter()
        .map(|line| effective_font_ratio(line))
        .fold(0.0f32, f32::max);
    let continued_toc_folio = content.first().is_some_and(|line| {
        let normalized = collapse_whitespace(&line.text).to_ascii_lowercase();
        normalized
            .split_whitespace()
            .next()
            .is_some_and(|word| word.chars().all(|ch| ch.is_ascii_digit()))
            && (normalized.contains("law review") || normalized.contains("law journal"))
    });
    if content.len() >= 5
        && toc_lines * 2 >= content.len()
        && (strongest_title_ratio < 1.25 || continued_toc_folio)
    {
        score -= 4.0;
        evidence.push(ArticleBoundaryEvidence {
            kind: "toc_continuation_penalty".to_owned(),
            score: -4.0,
            detail: "page is dominated by a continued table of contents".to_owned(),
        });
    }

    if advertisement_or_directory_page(page_lines) && !(masthead && journal_identity) {
        let removed_score = score;
        score = 0.0;
        evidence.push(ArticleBoundaryEvidence {
            kind: "advertisement_exclusion".to_owned(),
            score: -removed_score,
            detail: "advertisement or commercial directory pages cannot start articles".to_owned(),
        });
    }

    if front_matter_directory_page(page_lines) && !(masthead && journal_identity) {
        let removed_score = score;
        score = 0.0;
        evidence.push(ArticleBoundaryEvidence {
            kind: "front_matter_directory_exclusion".to_owned(),
            score: -removed_score,
            detail:
                "faculty, officer, instructor, or advisory-board directories cannot start articles"
                    .to_owned(),
        });
    }

    if continuing_publication_section_header(page_lines, publication_section.is_some()) {
        score -= 2.5;
        evidence.push(ArticleBoundaryEvidence {
            kind: "continuing_publication_section_penalty".to_owned(),
            score: -2.5,
            detail: "page continues an existing notes, cases, decisions, or reviews section"
                .to_owned(),
        });
    }

    let stacked_uppercase_opening = title_lines.len() >= 2
        && title_lines
            .iter()
            .take(2)
            .all(|line| uppercase_ratio(&line.text) >= 0.68);
    let prose_like_titles = title_lines
        .iter()
        .filter(|line| word_count(&line.text) >= 7 && uppercase_ratio(&line.text) < 0.55)
        .count();
    if publication_section.is_none()
        && !stacked_uppercase_opening
        && title_lines.len() >= 3
        && prose_like_titles * 2 >= title_lines.len()
    {
        score -= 2.0;
        evidence.push(ArticleBoundaryEvidence {
            kind: "prose_density_penalty".to_owned(),
            score: -2.0,
            detail: "most title candidates are ordinary mixed-case prose".to_owned(),
        });
    }

    let dense_numbered_diagram = page_lines
        .iter()
        .filter(|line| {
            line.note_marker.is_some()
                && word_count(&line.text) <= 8
                && effective_font_ratio(line) >= 0.98
        })
        .count()
        >= 4;
    let mut note_markers = if dense_numbered_diagram {
        Vec::new()
    } else {
        page_lines
            .iter()
            .filter_map(|line| {
                (line.marginalia || footnote_geometry_like(line))
                    .then_some(line.note_marker)
                    .flatten()
            })
            .filter(|marker| (1..=MAX_NOTE_MARKER).contains(marker))
            .collect::<Vec<_>>()
    };
    note_markers.sort_unstable();
    note_markers.dedup();

    PageObservation {
        page_index,
        title_score: score.max(0.0),
        title_hint,
        title_evidence: evidence,
        note_markers,
    }
}

fn midpage_boundary_candidates(lines: &[ArticleSegmentationLine]) -> Vec<BoundaryCandidate> {
    let mut by_page: BTreeMap<usize, Vec<&ArticleSegmentationLine>> = BTreeMap::new();
    for line in lines {
        by_page.entry(line.page_index).or_default().push(line);
    }
    let mut candidates = Vec::new();
    for (page_index, mut page_lines) in by_page {
        page_lines.sort_by_key(|line| line.line_index);
        if advertisement_or_directory_page(&page_lines) || front_matter_directory_page(&page_lines)
        {
            continue;
        }
        for position in 3..page_lines.len().saturating_sub(2) {
            let first = page_lines[position];
            if normalized_top(first) >= 0.84
                || normalized_top(first) <= 0.30
                || !midpage_title_line(first)
                || internal_heading_only(&first.text)
                || first.toc_like
            {
                continue;
            }
            let second = page_lines.get(position + 1).copied();
            let title_end = if second.is_some_and(|line| {
                midpage_title_line(line) && line.line_index <= first.line_index.saturating_add(2)
            }) {
                position + 2
            } else {
                position + 1
            };
            let title_words = page_lines[position..title_end]
                .iter()
                .map(|line| word_count(&line.text))
                .sum::<usize>();
            if title_words < 6 {
                continue;
            }
            let has_signature = author_signature_like(page_lines[position - 1]);
            if !has_signature || page_lines[position - 1].toc_like {
                continue;
            }
            let body = page_lines.iter().skip(title_end).take(3).find(|line| {
                !line.toc_like && word_count(&line.text) >= 8 && uppercase_ratio(&line.text) < 0.55
            });
            if body.is_none() {
                continue;
            }
            let title_hint = page_lines[position..title_end]
                .iter()
                .map(|line| collapse_whitespace(&line.text))
                .collect::<Vec<_>>()
                .join(" ");
            candidates.push(BoundaryCandidate {
                page_index,
                line_index: first.line_index,
                score: 5.4,
                title_hint: Some(title_hint.clone()),
                evidence: vec![ArticleBoundaryEvidence {
                    kind: "midpage_title_after_signature".to_owned(),
                    score: 5.4,
                    detail: format!(
                        "author signature is followed by a new title and body prose: {title_hint}"
                    ),
                }],
            });
        }
    }
    candidates
}

fn midpage_title_line(line: &ArticleSegmentationLine) -> bool {
    let words = word_count(&line.text);
    (2..=18).contains(&words)
        && uppercase_ratio(&line.text) >= 0.68
        && effective_font_ratio(line) >= 0.88
        && !line.text.chars().any(|ch| ch.is_ascii_digit())
        && !line
            .text
            .trim_start()
            .starts_with(|ch: char| matches!(ch, '•' | '*' | '†' | '‡'))
}

fn author_signature_like(line: &ArticleSegmentationLine) -> bool {
    let text = collapse_whitespace(&line.text)
        .trim_matches(|ch: char| !ch.is_alphanumeric() && ch != ' ')
        .to_owned();
    let lower = text.to_ascii_lowercase();
    let words = word_count(&text);
    let small_caps_signature = (2..=6).contains(&words)
        && text.chars().count() <= 80
        && !text.chars().any(|ch| ch.is_ascii_digit())
        && uppercase_ratio(&text) >= 0.68
        && effective_font_ratio(line) <= 0.90
        && !lower.contains("law review")
        && !lower.contains("law journal")
        && publication_section_heading(&text).is_none()
        && !section_prefixed_heading(&text)
        && !internal_heading_only(&text);
    small_caps_signature
        || ((2..=6).contains(&words)
            && byline_like(&line.text)
            && signature_digits_are_class_year_only(&line.text)
            && publication_section_heading(&text).is_none()
            && !internal_heading_only(&text))
}

fn signature_digits_are_class_year_only(text: &str) -> bool {
    if !text.chars().any(|ch| ch.is_ascii_digit()) {
        return true;
    }
    text.split_whitespace().last().is_some_and(|token| {
        let token = token.trim_matches(|ch: char| matches!(ch, '\'' | '’' | '*' | '†'));
        matches!(token.len(), 2 | 4) && token.chars().all(|ch| ch.is_ascii_digit())
    })
}

fn apply_note_reset_evidence(pages: &[PageObservation], candidates: &mut [BoundaryCandidate]) {
    for reset_page in 1..pages.len() {
        let Some(current_low) = pages[reset_page].note_markers.first().copied() else {
            continue;
        };
        if current_low > 2 {
            continue;
        }
        let prior_start = reset_page.saturating_sub(PRIOR_NOTE_LOOKBACK_PAGES);
        let prior_high = pages[prior_start..reset_page]
            .iter()
            .flat_map(|page| page.note_markers.iter().copied())
            .max()
            .unwrap_or(0);
        if prior_high < 8 || prior_high.saturating_sub(current_low) < 6 {
            continue;
        }

        let anchor_start = reset_page.saturating_sub(RESET_LOOKBACK_PAGES).max(1);
        let anchor_page = (anchor_start..=reset_page)
            .max_by(|left, right| {
                pages[*left]
                    .title_score
                    .total_cmp(&pages[*right].title_score)
                    .then_with(|| left.cmp(right))
            })
            .unwrap_or(reset_page);
        let distance = reset_page - anchor_page;
        let uppercase_anchor = pages[anchor_page]
            .title_evidence
            .iter()
            .any(|item| item.kind == "uppercase_title");
        let reset_score = if pages[anchor_page].title_score < 1.4 {
            0.9
        } else if pages[anchor_page].title_score < 1.8 && uppercase_anchor {
            1.5
        } else if pages[anchor_page].title_score < 1.8 {
            0.9
        } else if distance == 0 {
            3.2
        } else if pages[anchor_page].title_score >= 1.0 {
            2.9
        } else {
            2.1
        };
        let candidate = &mut candidates[anchor_page - 1];
        if candidate
            .evidence
            .iter()
            .any(|item| item.kind == "footnote_reset")
        {
            continue;
        }
        candidate.score += reset_score;
        candidate.evidence.push(ArticleBoundaryEvidence {
            kind: "footnote_reset".to_owned(),
            score: reset_score,
            detail: format!(
                "note sequence falls from at least {prior_high} to {current_low} on page {}; anchored {} page(s) earlier",
                reset_page + 1,
                distance
            ),
        });
    }
}

fn select_boundaries(
    mut candidates: Vec<BoundaryCandidate>,
    page_count: usize,
) -> Vec<BoundaryCandidate> {
    candidates.retain(|candidate| {
        candidate.page_index >= MIN_EDGE_ARTICLE_PAGES
            && page_count.saturating_sub(candidate.page_index) >= MIN_EDGE_ARTICLE_PAGES
            && candidate.score >= BOUNDARY_SCORE_THRESHOLD
    });
    candidates.sort_by(|left, right| {
        candidate_has_evidence(right, "inner_issue_masthead")
            .cmp(&candidate_has_evidence(left, "inner_issue_masthead"))
            .then_with(|| right.score.total_cmp(&left.score))
            .then_with(|| {
                (left.page_index, left.line_index).cmp(&(right.page_index, right.line_index))
            })
    });

    let mut selected: Vec<BoundaryCandidate> = Vec::new();
    for candidate in candidates {
        let too_close = selected.iter().any(|chosen| {
            chosen.page_index.abs_diff(candidate.page_index) < MIN_BETWEEN_BOUNDARIES_PAGES
        });
        if !too_close {
            selected.push(candidate);
        }
    }
    selected.sort_by_key(|candidate| (candidate.page_index, candidate.line_index));
    selected
}

fn candidate_has_evidence(candidate: &BoundaryCandidate, kind: &str) -> bool {
    candidate.evidence.iter().any(|item| item.kind == kind)
}

fn attach_opening_front_matter_to_first_article(
    lines: &[ArticleSegmentationLine],
    candidates: &mut [BoundaryCandidate],
    page_count: usize,
) {
    let Some(position) = candidates
        .iter()
        .enumerate()
        .filter(|(_, candidate)| {
            candidate.page_index >= MIN_EDGE_ARTICLE_PAGES
                && page_count.saturating_sub(candidate.page_index) >= MIN_EDGE_ARTICLE_PAGES
                && candidate.score >= BOUNDARY_SCORE_THRESHOLD
        })
        .min_by_key(|(_, candidate)| (candidate.page_index, candidate.line_index))
        .map(|(position, _)| position)
    else {
        return;
    };
    let candidate_page = candidates[position].page_index;
    if substantive_body_pages_before(lines, candidate_page) >= 2 {
        return;
    }
    if candidate_page > 4 && !opening_front_matter_evidence(lines, candidate_page) {
        return;
    }
    let removed_score = candidates[position].score;
    candidates[position].score = 0.0;
    candidates[position].evidence.push(ArticleBoundaryEvidence {
        kind: "opening_article_attached_to_front_matter".to_owned(),
        score: -removed_score,
        detail: "the first article remains in the document-start span with opening front matter"
            .to_owned(),
    });
}

fn opening_front_matter_evidence(lines: &[ArticleSegmentationLine], end_page: usize) -> bool {
    let mut by_page: BTreeMap<usize, Vec<&ArticleSegmentationLine>> = BTreeMap::new();
    for line in lines.iter().filter(|line| line.page_index < end_page) {
        by_page.entry(line.page_index).or_default().push(line);
    }
    by_page.into_values().any(|page_lines| {
        page_lines.iter().any(|line| line.toc_like)
            || front_matter_directory_page(&page_lines)
            || page_lines.iter().take(16).any(|line| {
                let normalized = collapse_whitespace(&line.text).to_ascii_lowercase();
                normalized.contains("editorial board")
                    || normalized.contains("board of editors")
                    || normalized.contains("editor in chief")
            })
    })
}

fn substantive_body_pages_before(lines: &[ArticleSegmentationLine], end_page: usize) -> usize {
    let mut by_page: BTreeMap<usize, Vec<&ArticleSegmentationLine>> = BTreeMap::new();
    for line in lines.iter().filter(|line| line.page_index < end_page) {
        by_page.entry(line.page_index).or_default().push(line);
    }
    by_page
        .into_values()
        .filter(|page_lines| {
            !advertisement_or_directory_page(page_lines)
                && !front_matter_directory_page(page_lines)
                && page_lines
                    .iter()
                    .filter(|line| {
                        !line.toc_like
                            && !line.repeated_edge_text
                            && word_count(&line.text) >= 7
                            && uppercase_ratio(&line.text) < 0.55
                            && effective_font_ratio(line) <= 1.20
                    })
                    .count()
                    >= 6
        })
        .count()
}

fn spans_from_boundaries(
    pages: &[PageObservation],
    boundaries: &[BoundaryCandidate],
    page_count: usize,
) -> Vec<ArticleSpan> {
    if boundaries.is_empty() {
        let mut span = single_span(page_count);
        span.title_hint = pages.first().and_then(|page| page.title_hint.clone());
        return vec![span];
    }

    let mut starts = Vec::with_capacity(boundaries.len() + 1);
    starts.push(((0usize, 0usize), None));
    starts.extend(boundaries.iter().map(|candidate| {
        (
            (candidate.page_index, candidate.line_index),
            Some(candidate),
        )
    }));
    starts
        .iter()
        .enumerate()
        .map(|(article_index, ((start_page, start_line), candidate))| {
            let (end_page, end_line) = starts
                .get(article_index + 1)
                .map(|(next, _)| *next)
                .unwrap_or((page_count, 0));
            let (confidence, evidence) = if let Some(candidate) = candidate {
                (
                    score_confidence(candidate.score),
                    candidate.evidence.clone(),
                )
            } else {
                (
                    1.0,
                    vec![ArticleBoundaryEvidence {
                        kind: "document_start".to_owned(),
                        score: 1.0,
                        detail: "PDF starts here".to_owned(),
                    }],
                )
            };
            ArticleSpan {
                article_index,
                start_page_index: *start_page,
                start_line_index: *start_line,
                end_page_index: end_page,
                end_line_index: end_line,
                confidence,
                title_hint: candidate
                    .and_then(|boundary| boundary.title_hint.clone())
                    .or_else(|| {
                        pages
                            .get(*start_page)
                            .and_then(|page| page.title_hint.clone())
                    }),
                evidence,
            }
        })
        .collect()
}

fn score_confidence(score: f32) -> f32 {
    ((score - 2.0) / 4.0).clamp(0.5, 0.99)
}

fn effective_font_ratio(line: &ArticleSegmentationLine) -> f32 {
    if line.font_ratio_doc > 0.0 {
        line.font_ratio_doc.max(line.font_ratio_page)
    } else {
        line.font_ratio_page
    }
}

fn normalized_top(line: &ArticleSegmentationLine) -> f32 {
    line.top / line.page_height.max(1.0)
}

fn furniture_like(
    line: &ArticleSegmentationLine,
    recurring_top_text: &BTreeMap<String, (f32, f32)>,
) -> bool {
    if line.repeated_edge_text {
        return true;
    }
    let text = collapse_whitespace(&line.text);
    if normalized_top(line) >= 0.84 && numbered_journal_folio(&text) {
        return true;
    }
    let canonical = canonical_edge_text(&text);
    if normalized_top(line) >= 0.87
        && recurring_top_text
            .get(&canonical)
            .is_some_and(|(median_ratio, median_top)| {
                (normalized_top(line) - median_top).abs() <= 0.025
                    && effective_font_ratio(line) <= median_ratio * 1.30
            })
    {
        return true;
    }
    text.is_empty()
        || text.chars().all(|ch| ch.is_ascii_digit())
        || (word_count(&text) <= 8
            && normalized_top(line) >= 0.93
            && effective_font_ratio(line) <= 1.12
            && running_header_phrase(&text))
}

fn numbered_journal_folio(text: &str) -> bool {
    let lower = collapse_whitespace(text).to_ascii_lowercase();
    lower
        .split_whitespace()
        .next()
        .is_some_and(|word| word.chars().all(|ch| ch.is_ascii_digit()))
        && (lower.contains("law review") || lower.contains("law journal"))
}

fn recurring_top_text(lines: &[ArticleSegmentationLine]) -> BTreeMap<String, (f32, f32)> {
    let mut occurrences: BTreeMap<String, Vec<(usize, f32, f32)>> = BTreeMap::new();
    for line in lines {
        if normalized_top(line) < 0.87 {
            continue;
        }
        let canonical = canonical_edge_text(&line.text);
        if canonical.is_empty() || canonical.split_whitespace().count() > 14 {
            continue;
        }
        occurrences.entry(canonical).or_default().push((
            line.page_index,
            effective_font_ratio(line),
            normalized_top(line),
        ));
    }
    occurrences
        .into_iter()
        .filter_map(|(text, mut values)| {
            values.sort_by(|left, right| {
                left.0
                    .cmp(&right.0)
                    .then_with(|| left.1.total_cmp(&right.1))
            });
            values.dedup_by_key(|value| value.0);
            if values.len() < 3 {
                return None;
            }
            let mut ratios = values
                .iter()
                .map(|(_, ratio, _)| *ratio)
                .collect::<Vec<_>>();
            ratios.sort_by(f32::total_cmp);
            let median = ratios[ratios.len() / 2].max(0.1);
            let mut tops = values
                .into_iter()
                .map(|(_, _, top)| top)
                .collect::<Vec<_>>();
            tops.sort_by(f32::total_cmp);
            let median_top = tops[tops.len() / 2];
            Some((text, (median, median_top)))
        })
        .collect()
}

fn canonical_edge_text(text: &str) -> String {
    collapse_whitespace(
        &text
            .chars()
            .map(|ch| {
                if ch.is_alphabetic() {
                    ch.to_ascii_lowercase()
                } else {
                    ' '
                }
            })
            .collect::<String>(),
    )
    .split_whitespace()
    .filter(|word| !matches!(*word, "vol" | "volume"))
    .collect::<Vec<_>>()
    .join(" ")
}

fn running_header_phrase(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("law journal")
        || lower.contains("law review")
        || lower.starts_with("the georgetown")
        || lower.starts_with("vol.")
        || lower.starts_with("[vol.")
}

fn title_line_like(line: &ArticleSegmentationLine) -> bool {
    let text = collapse_whitespace(&line.text);
    let words = word_count(&text);
    if !(1..=22).contains(&words) || normalized_top(line) < 0.55 {
        return false;
    }
    let ratio = effective_font_ratio(line);
    let upper = uppercase_ratio(&text);
    let width_ok = line.line_width_ratio <= 0.0 || line.line_width_ratio <= 0.92;
    width_ok
        && (ratio >= 1.14
            || line.margin_centered
            || (upper >= 0.70 && words <= 16 && ratio >= 0.90))
}

fn footnote_geometry_like(line: &ArticleSegmentationLine) -> bool {
    line.note_marker.is_some() && (line.font_ratio_doc <= 0.96 || line.font_ratio_page <= 0.94)
}

fn byline_like(text: &str) -> bool {
    let text = collapse_whitespace(text)
        .trim_matches('*')
        .trim()
        .to_owned();
    let lower = text.to_ascii_lowercase();
    let words = text
        .split_whitespace()
        .filter(|word| word.chars().any(char::is_alphabetic))
        .collect::<Vec<_>>();
    if !(2..=7).contains(&words.len())
        || text.chars().count() > 100
        || uppercase_ratio(&text) >= 0.62
        || lower.starts_with("volume ")
        || lower.contains("law review")
        || lower.contains("law journal")
        || section_prefixed_heading(&text)
        || internal_heading_only(&lower)
    {
        return false;
    }
    let capped = words
        .iter()
        .filter(|word| {
            word.trim_matches(|ch: char| !ch.is_alphabetic())
                .chars()
                .next()
                .is_some_and(char::is_uppercase)
        })
        .count();
    capped * 4 >= words.len() * 3
}

fn publication_section_heading(text: &str) -> Option<&'static str> {
    let normalized = collapse_whitespace(text)
        .trim_matches(|ch: char| !ch.is_alphanumeric() && ch != ' ')
        .to_ascii_lowercase();
    match normalized.as_str() {
        "notes" => Some("notes"),
        "comments" => Some("comments"),
        "decisions" => Some("decisions"),
        "recent decisions" => Some("recent decisions"),
        "recent cases" => Some("recent cases"),
        "case notes" => Some("case notes"),
        "book reviews" => Some("book reviews"),
        "book review" => Some("book review"),
        "recent statute" => Some("recent statute"),
        "recent statutes" => Some("recent statutes"),
        "editorial" => Some("editorial"),
        "federal legislation" => Some("federal legislation"),
        "state legislation" => Some("state legislation"),
        "legislation" => Some("legislation"),
        "symposium" => Some("symposium"),
        _ => None,
    }
}

fn advertisement_or_directory_page(content: &[&ArticleSegmentationLine]) -> bool {
    let lower = content
        .iter()
        .take(32)
        .map(|line| collapse_whitespace(&line.text).to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join(" ");
    [
        "legal directory",
        "university alumni",
        "our representative will be at",
        "pictures and prices",
        "law students are provided",
        "law book publishers",
        "lunch and restaurant",
        "madison avenue",
        "subscription price",
        "deferred payment plan",
        "please mention the journal",
        "when dealing with our advertisers",
        "order your set today",
        "printers, stationers, engravers",
        "supplies for law students",
        "university lunch",
    ]
    .iter()
    .any(|cue| lower.contains(cue))
        || lower.matches("bldg").count() >= 3
        || (lower.contains("every student") && lower.contains("law school"))
}

fn front_matter_directory_page(content: &[&ArticleSegmentationLine]) -> bool {
    let lower = content
        .iter()
        .take(32)
        .map(|line| collapse_whitespace(&line.text).to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join(" ");
    [
        "administrative officers",
        "legal research and legal writing instructors",
        "law school advisory board",
        "faculty and staff",
        "board of advisors",
    ]
    .iter()
    .any(|cue| lower.contains(cue))
}

fn continuing_publication_section_header(
    page_lines: &[&ArticleSegmentationLine],
    opens_publication_section: bool,
) -> bool {
    if opens_publication_section {
        return false;
    }
    page_lines.iter().take(2).any(|line| {
        let normalized = collapse_whitespace(&line.text).to_ascii_lowercase();
        [
            "notes ",
            "notes on recent cases ",
            "recent cases ",
            "recent decisions ",
            "book reviews ",
        ]
        .iter()
        .any(|prefix| normalized.starts_with(prefix))
            && normalized.chars().any(|ch| ch.is_ascii_digit())
    })
}

fn running_folio_heading(text: &str) -> bool {
    let text = collapse_whitespace(text);
    let Some(first) = text.split_whitespace().next() else {
        return false;
    };
    let trimmed = first.trim_start_matches('[');
    let year_prefix = trimmed
        .get(..4)
        .is_some_and(|year| year.chars().all(|ch| ch.is_ascii_digit()))
        && trimmed.get(4..5) == Some("]");
    year_prefix
}

fn appendix_heading(text: &str) -> bool {
    collapse_whitespace(text)
        .trim_matches(|ch: char| !ch.is_alphanumeric() && ch != ' ')
        .to_ascii_lowercase()
        .starts_with("appendix ")
}

fn index_or_bibliography_heading(text: &str) -> bool {
    let normalized = collapse_whitespace(text)
        .trim_matches(|ch: char| !ch.is_alphanumeric() && ch != ' ')
        .to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "index" | "index by authors" | "author index" | "subject index" | "bibliography"
    ) || normalized.contains("index to volume")
        || normalized.starts_with("index by ")
        || normalized.starts_with("table of indexes")
}

fn lowercase_prose_heading(text: &str) -> bool {
    word_count(text) >= 9
        && text
            .chars()
            .find(|ch| ch.is_alphabetic())
            .is_some_and(char::is_lowercase)
}

fn table_or_figure_heading(text: &str) -> bool {
    let upper = format!(" {} ", collapse_whitespace(text).to_ascii_uppercase());
    [
        " TABLE 1 ",
        " TABLE 2 ",
        " TABLE 3 ",
        " FIGURE 1 ",
        " FIGURE 2 ",
    ]
    .iter()
    .any(|cue| upper.contains(cue))
}

fn continuation_opening_like(line: &ArticleSegmentationLine) -> bool {
    let text = collapse_whitespace(&line.text);
    let words = word_count(&text);
    words >= 9
        && effective_font_ratio(line) <= 1.10
        && uppercase_ratio(&text) < 0.55
        && !line.margin_centered
}

fn internal_heading_only(text: &str) -> bool {
    let normalized = collapse_whitespace(text)
        .trim_matches(|ch: char| !ch.is_alphanumeric() && ch != ' ')
        .to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "introduction"
            | "conclusion"
            | "background"
            | "methodology"
            | "methods"
            | "results"
            | "discussion"
            | "appendix"
            | "table of contents"
    ) || normalized.starts_with("appendix ")
        || normalized.starts_with("part ")
        || normalized.starts_with("chapter ")
        || starts_with_roman_section(&normalized)
}

fn starts_with_roman_section(text: &str) -> bool {
    let Some((prefix, _)) = text.split_once(['.', ' ']) else {
        return false;
    };
    !prefix.is_empty()
        && prefix.len() <= 6
        && prefix
            .chars()
            .all(|ch| matches!(ch.to_ascii_lowercase(), 'i' | 'v' | 'x'))
}

fn section_prefixed_heading(text: &str) -> bool {
    let text = collapse_whitespace(text);
    let Some((prefix, _)) = text.split_once(". ") else {
        return false;
    };
    (prefix.len() == 1 && prefix.chars().all(|ch| ch.is_ascii_uppercase()))
        || (!prefix.is_empty()
            && prefix.len() <= 6
            && prefix
                .chars()
                .all(|ch| matches!(ch.to_ascii_lowercase(), 'i' | 'v' | 'x')))
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn word_count(text: &str) -> usize {
    text.split_whitespace()
        .filter(|word| word.chars().any(char::is_alphabetic))
        .count()
}

fn uppercase_ratio(text: &str) -> f32 {
    let letters = text.chars().filter(|ch| ch.is_alphabetic()).count();
    if letters == 0 {
        return 0.0;
    }
    text.chars().filter(|ch| ch.is_uppercase()).count() as f32 / letters as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(page: usize, index: usize, text: &str) -> ArticleSegmentationLine {
        ArticleSegmentationLine {
            page_index: page,
            line_index: index,
            text: text.to_owned(),
            font_ratio_page: 1.0,
            font_ratio_doc: 1.0,
            margin_centered: false,
            line_width_ratio: 0.8,
            top: 700.0 - index as f32 * 14.0,
            page_height: 792.0,
            repeated_edge_text: false,
            toc_like: false,
            note_marker: None,
            marginalia: false,
        }
    }

    fn add_journal_identity(lines: &mut Vec<ArticleSegmentationLine>) {
        let mut identity = line(0, usize::MAX, "THE EXAMPLE LAW REVIEW");
        identity.repeated_edge_text = true;
        lines.push(identity);
    }

    #[test]
    fn single_article_defaults_to_one_span() {
        let lines = (0..12)
            .map(|page| {
                line(
                    page,
                    0,
                    "Ordinary body text continues across this page normally.",
                )
            })
            .collect::<Vec<_>>();
        let spans = detect_article_spans(&lines, 12);
        assert_eq!(spans.len(), 1);
        assert_eq!(
            (spans[0].start_page_index, spans[0].end_page_index),
            (0, 12)
        );
    }

    #[test]
    fn non_journal_document_does_not_split_on_title_like_sections() {
        let mut lines = (0..14)
            .filter(|page| *page != 7)
            .map(|page| {
                line(
                    page,
                    0,
                    "Ordinary catalog prose continues across this page normally.",
                )
            })
            .collect::<Vec<_>>();
        let mut title = line(7, 0, "DEPARTMENT OF PUBLIC ADMINISTRATION");
        title.font_ratio_doc = 1.5;
        lines.push(title);
        lines.push(line(7, 1, "Jane Q. Scholar"));

        let (spans, traces) = detect_article_spans_with_trace(&lines, 14);
        assert_eq!(spans.len(), 1);
        assert!(traces.iter().any(|candidate| {
            candidate.page_index == 7
                && candidate
                    .evidence
                    .iter()
                    .any(|evidence| evidence.kind == "non_journal_document")
        }));
    }

    #[test]
    fn opening_title_after_cover_does_not_split_single_article() {
        let mut lines = (0..12)
            .filter(|page| *page != 1)
            .map(|page| {
                line(
                    page,
                    0,
                    "Ordinary body text continues across this page normally.",
                )
            })
            .collect::<Vec<_>>();
        let mut title = line(1, 0, "COMMENTS");
        title.font_ratio_doc = 1.5;
        title.margin_centered = true;
        lines.push(title);
        let mut subtitle = line(1, 1, "A NEW ARTICLE ABOUT CONTRACT LAW");
        subtitle.font_ratio_doc = 1.5;
        subtitle.margin_centered = true;
        lines.push(subtitle);
        lines.push(line(1, 2, "Jane Q. Scholar"));

        let spans = detect_article_spans(&lines, 12);
        assert_eq!(spans.len(), 1);
    }

    #[test]
    fn terminal_books_received_list_stays_with_preceding_article() {
        let mut lines = (0..12)
            .filter(|page| *page != 11)
            .map(|page| {
                line(
                    page,
                    0,
                    "Ordinary body text continues across this page normally.",
                )
            })
            .collect::<Vec<_>>();
        let mut heading = line(11, 0, "BOOKS RECEIVED");
        heading.font_ratio_doc = 1.6;
        heading.margin_centered = true;
        lines.push(heading);
        lines.push(line(11, 1, "Administrative Law. By Jane Scholar."));

        let spans = detect_article_spans(&lines, 12);
        assert_eq!(spans.len(), 1);
    }

    #[test]
    fn volume_index_title_stays_with_preceding_article() {
        let mut lines = (0..14)
            .filter(|page| *page != 8)
            .map(|page| {
                line(
                    page,
                    0,
                    "Ordinary body text continues across this page normally.",
                )
            })
            .collect::<Vec<_>>();
        for (index, text) in ["Creighton", "Law Review", "Index to Volume 8"]
            .into_iter()
            .enumerate()
        {
            let mut row = line(8, index, text);
            row.font_ratio_doc = if index < 2 { 3.0 } else { 1.5 };
            row.toc_like = true;
            lines.push(row);
        }

        let spans = detect_article_spans(&lines, 14);
        assert_eq!(spans.len(), 1);
    }

    #[test]
    fn footnote_reset_anchors_to_preceding_title_page() {
        let mut lines = Vec::new();
        for page in 0..18 {
            lines.push(line(
                page,
                0,
                "Ordinary body text continues across this page normally.",
            ));
        }
        for marker in 8..=20 {
            let mut note = line(7, marker as usize, &format!("{marker}. Authority."));
            note.font_ratio_doc = 0.78;
            note.font_ratio_page = 0.78;
            note.note_marker = Some(marker);
            note.marginalia = true;
            lines.push(note);
        }
        let mut title = line(10, 0, "A NEW ARTICLE ABOUT CONTRACT LAW");
        title.font_ratio_doc = 1.45;
        title.margin_centered = true;
        lines.push(title);
        lines.push(line(10, 1, "Jane Q. Scholar*"));
        let mut reset = line(12, 20, "1. New authority.");
        reset.font_ratio_doc = 0.78;
        reset.font_ratio_page = 0.78;
        reset.note_marker = Some(1);
        reset.marginalia = true;
        lines.push(reset);

        add_journal_identity(&mut lines);
        let spans = detect_article_spans(&lines, 18);
        assert_eq!(
            spans
                .iter()
                .map(|span| span.start_page_index)
                .collect::<Vec<_>>(),
            vec![0, 10]
        );
    }

    #[test]
    fn dense_numbered_diagram_does_not_supply_note_reset_evidence() {
        let mut lines = (0..16)
            .map(|page| {
                line(
                    page,
                    0,
                    "Ordinary body text continues across this page normally.",
                )
            })
            .collect::<Vec<_>>();
        for marker in 8..=20 {
            let mut note = line(6, marker as usize, &format!("{marker}. Authority."));
            note.font_ratio_doc = 0.78;
            note.font_ratio_page = 0.78;
            note.note_marker = Some(marker);
            note.marginalia = true;
            lines.push(note);
        }
        let mut weak_heading = line(8, 0, "MODEL OF ADMINISTRATIVE REVIEW");
        weak_heading.font_ratio_doc = 1.23;
        lines.push(weak_heading);
        for marker in 1..=4 {
            let mut node = line(8, marker + 1, &format!("{marker}. Agency"));
            node.note_marker = Some(marker as u16);
            node.marginalia = true;
            lines.push(node);
        }

        let spans = detect_article_spans(&lines, 16);
        assert_eq!(spans.len(), 1);
    }

    #[test]
    fn repeated_low_markers_do_not_multiply_reset_evidence() {
        let mut lines = (0..18)
            .filter(|page| *page != 8)
            .map(|page| {
                line(
                    page,
                    0,
                    "Ordinary body text continues across this page normally.",
                )
            })
            .collect::<Vec<_>>();
        for marker in 8..=20 {
            let mut note = line(6, marker as usize, &format!("{marker}. Authority."));
            note.font_ratio_doc = 0.78;
            note.font_ratio_page = 0.78;
            note.note_marker = Some(marker);
            note.marginalia = true;
            lines.push(note);
        }
        lines.push(line(8, 0, "ADMINISTRATIVE REVIEW STANDARDS"));
        lines.push(line(
            8,
            1,
            "This ordinary prose prevents a synthetic title-to-note geometry gap.",
        ));
        for page in [8, 10, 12] {
            let mut false_reset = line(page, 20, "1. Agency");
            false_reset.font_ratio_doc = 0.78;
            false_reset.font_ratio_page = 0.78;
            false_reset.note_marker = Some(1);
            false_reset.marginalia = true;
            lines.push(false_reset);
        }

        let spans = detect_article_spans(&lines, 18);
        assert_eq!(spans.len(), 1);
    }

    #[test]
    fn internal_heading_does_not_split_article() {
        let mut lines = (0..12)
            .map(|page| {
                line(
                    page,
                    0,
                    "Ordinary body text continues across this page normally.",
                )
            })
            .collect::<Vec<_>>();
        let mut heading = line(6, 0, "III. DISCUSSION");
        heading.font_ratio_doc = 1.35;
        heading.margin_centered = true;
        lines.push(heading);
        let spans = detect_article_spans(&lines, 12);
        assert_eq!(spans.len(), 1);
    }

    #[test]
    fn detects_article_starting_midpage_after_signature() {
        let mut lines = (0..12)
            .map(|page| {
                line(
                    page,
                    0,
                    "Ordinary body text continues across this page normally.",
                )
            })
            .collect::<Vec<_>>();
        lines.push(line(
            5,
            1,
            "The preceding article continues toward its concluding signature.",
        ));
        lines.push(line(
            5,
            2,
            "Its final paragraph supplies realistic lines above the transition.",
        ));
        let mut signature = line(5, 10, "RICHARD L. STILL");
        signature.font_ratio_doc = 0.74;
        signature.font_ratio_page = 0.74;
        lines.push(signature);
        lines.push(line(5, 11, "THE ABILITY OF A UNION TO CAUSE A DISCHARGE"));
        lines.push(line(
            5,
            12,
            "FOR NONPAYMENT OF DUES UNDER THE TAFT HARTLEY ACT",
        ));
        lines.push(line(
            5,
            13,
            "One of the most debated subjects in labor relations concerns union membership.",
        ));

        add_journal_identity(&mut lines);
        let spans = detect_article_spans(&lines, 12);
        assert_eq!(spans.len(), 2);
        assert_eq!(
            (spans[1].start_page_index, spans[1].start_line_index),
            (5, 11)
        );
        assert_eq!((spans[0].end_page_index, spans[0].end_line_index), (5, 11));
    }

    #[test]
    fn detects_midpage_start_after_title_case_signature() {
        let mut lines = (0..12)
            .map(|page| {
                line(
                    page,
                    0,
                    "Ordinary body text continues across this page normally.",
                )
            })
            .collect::<Vec<_>>();
        lines.push(line(
            5,
            1,
            "The preceding article continues toward its concluding signature.",
        ));
        lines.push(line(
            5,
            2,
            "Its final paragraph supplies realistic lines above the transition.",
        ));
        lines.push(line(5, 10, "Lyle A. Rodenburg '68"));
        lines.push(line(
            5,
            11,
            "CRIMINAL LAW ROBBERY AND THE FEDERAL BANK ROBBERY STATUTE",
        ));
        lines.push(line(
            5,
            12,
            "The new case asks whether false pretenses fall within the federal statute.",
        ));
        lines.push(line(
            5,
            13,
            "The discussion then turns to the elements required by the statute.",
        ));

        add_journal_identity(&mut lines);
        let spans = detect_article_spans(&lines, 12);
        assert_eq!(
            (spans[1].start_page_index, spans[1].start_line_index),
            (5, 11)
        );
    }

    #[test]
    fn flat_font_scan_can_use_stacked_uppercase_title() {
        let mut lines = (0..14)
            .filter(|page| *page != 7)
            .map(|page| {
                line(
                    page,
                    0,
                    "Ordinary body text continues across this page normally.",
                )
            })
            .collect::<Vec<_>>();
        let mut title_a = line(7, 0, "PROCEDURE BY DEFAULT IN SUPREME COURT");
        title_a.font_ratio_doc = 0.98;
        title_a.top = 640.0;
        let mut title_b = line(7, 1, "AGAINST DEFENDANT STATES");
        title_b.font_ratio_doc = 0.96;
        title_b.top = 625.0;
        lines.extend([title_a, title_b]);
        lines.push(line(
            7,
            2,
            "This article discusses the constitutional history of default proceedings.",
        ));

        add_journal_identity(&mut lines);
        let spans = detect_article_spans(&lines, 14);
        assert_eq!(
            spans
                .iter()
                .map(|span| span.start_page_index)
                .collect::<Vec<_>>(),
            vec![0, 7]
        );
    }

    #[test]
    fn advertisement_page_is_not_an_article() {
        let mut lines = (0..14)
            .filter(|page| *page != 7)
            .map(|page| {
                line(
                    page,
                    0,
                    "Ordinary body text continues across this page normally.",
                )
            })
            .collect::<Vec<_>>();
        let mut display = line(7, 0, "DINNER COATS AND THEIR ACCESSORIES");
        display.font_ratio_doc = 1.8;
        lines.push(display);
        lines.push(line(7, 1, "MADISON AVENUE NEW YORK"));
        lines.push(line(7, 2, "Our Representative Will Be At The Mayflower"));
        lines.push(line(7, 3, "Pictures and Prices of Hats"));

        let spans = detect_article_spans(&lines, 14);
        assert_eq!(spans.len(), 1);
    }

    #[test]
    fn title_page_beats_adjacent_toc_continuation() {
        let mut lines = (0..16)
            .filter(|page| !matches!(*page, 7 | 8))
            .map(|page| {
                line(
                    page,
                    0,
                    "Ordinary body text continues across this page normally.",
                )
            })
            .collect::<Vec<_>>();
        for (index, text) in [
            "CORROSION BY CODIFICATION",
            "THE DEFICIENCIES IN THE STATUTORY VERSIONS",
            "OF THE IMPLIED WARRANTY",
            "TABLE OF CONTENTS",
            "INTRODUCTION ................................ 104",
        ]
        .into_iter()
        .enumerate()
        {
            let mut row = line(7, index, text);
            row.font_ratio_doc = if index < 3 { 1.45 } else { 1.0 };
            row.toc_like = true;
            lines.push(row);
        }
        for (index, text) in [
            "COVERAGE OF COMMERCIAL BUILDINGS",
            "OPPOSED TO RESIDENTIAL .............. 124",
            "DEFENSES BY CONTRACTORS ............. 125",
            "DAMAGES ............................. 126",
            "CONCLUSION .......................... 127",
        ]
        .into_iter()
        .enumerate()
        {
            let mut row = line(8, index, text);
            row.font_ratio_doc = 1.0;
            row.toc_like = true;
            lines.push(row);
        }

        add_journal_identity(&mut lines);
        let spans = detect_article_spans(&lines, 16);
        assert_eq!(
            spans
                .iter()
                .map(|span| span.start_page_index)
                .collect::<Vec<_>>(),
            vec![0, 7]
        );
    }

    #[test]
    fn running_journal_folio_does_not_make_a_contents_continuation_a_boundary() {
        let mut lines = (0..14)
            .filter(|page| *page != 7)
            .map(|page| {
                line(
                    page,
                    0,
                    "Ordinary body text continues across this page normally.",
                )
            })
            .collect::<Vec<_>>();
        for (index, text) in [
            "222 CREIGHTON LAW REVIEW [Vol. 49",
            "D. INDETERMINACY ........................ 271",
            "VIII. FURTHER MEANING BEYOND THE MESSAGE:",
            "GAPS AND OMISSIONS ...................... 274",
            "A. ILLUSORY GAPS ......................... 276",
        ]
        .into_iter()
        .enumerate()
        {
            let mut row = line(7, index, text);
            row.font_ratio_doc = if index == 2 { 1.42 } else { 1.0 };
            row.toc_like = true;
            lines.push(row);
        }

        let spans = detect_article_spans(&lines, 14);
        assert_eq!(spans.len(), 1);
    }

    #[test]
    fn interior_issue_masthead_is_a_boundary() {
        let mut lines = (0..16)
            .filter(|page| *page != 8)
            .map(|page| {
                line(
                    page,
                    0,
                    "Ordinary body text continues across this page normally.",
                )
            })
            .collect::<Vec<_>>();
        lines.push(line(8, 0, "THE GEORGETOWN LAW JOURNAL"));
        lines.push(line(8, 1, "THE BOARD OF EDITORS"));
        lines.push(line(8, 2, "Jane Scholar, Editor in Chief"));
        lines.push(line(8, 3, "John Writer, Notes Editor"));

        add_journal_identity(&mut lines);
        let (spans, traces) = detect_article_spans_with_trace(&lines, 16);
        assert_eq!(
            spans
                .iter()
                .map(|span| span.start_page_index)
                .collect::<Vec<_>>(),
            vec![0, 8],
            "{traces:#?}"
        );
    }

    #[test]
    fn weak_uppercase_title_plus_note_reset_is_a_boundary() {
        let mut lines = (0..16)
            .filter(|page| *page != 8)
            .map(|page| {
                line(
                    page,
                    0,
                    "Ordinary body text continues across this page normally.",
                )
            })
            .collect::<Vec<_>>();
        for marker in 8..=20 {
            let mut note = line(6, marker as usize, &format!("{marker}. Authority."));
            note.font_ratio_doc = 0.78;
            note.font_ratio_page = 0.78;
            note.note_marker = Some(marker);
            note.marginalia = true;
            lines.push(note);
        }
        let mut title = line(8, 0, "LOOPHOLE TO EXECUTION FORD V. WAINWRIGHT");
        title.font_ratio_doc = 1.23;
        lines.push(title);
        let mut reset = line(8, 20, "1. New authority.");
        reset.font_ratio_doc = 0.78;
        reset.font_ratio_page = 0.78;
        reset.note_marker = Some(1);
        reset.marginalia = true;
        lines.push(reset);

        add_journal_identity(&mut lines);
        let (spans, traces) = detect_article_spans_with_trace(&lines, 16);
        assert_eq!(
            spans
                .iter()
                .map(|span| span.start_page_index)
                .collect::<Vec<_>>(),
            vec![0, 8],
            "{traces:#?}"
        );
    }

    #[test]
    fn preserves_verified_two_page_editorial_between_boundaries() {
        let mut lines = (0..14)
            .filter(|page| !matches!(*page, 6 | 8))
            .map(|page| {
                line(
                    page,
                    0,
                    "Ordinary body text continues across this page normally.",
                )
            })
            .collect::<Vec<_>>();
        lines.push(line(6, 0, "THE GEORGETOWN LAW JOURNAL"));
        lines.push(line(6, 1, "EDITORIAL BOARD"));
        lines.push(line(6, 2, "Jane Scholar, Editor in Chief"));
        lines.push(line(6, 3, "John Writer, Notes Editor"));
        let mut notes = line(8, 0, "NOTES");
        notes.font_ratio_doc = 1.5;
        lines.push(notes);

        add_journal_identity(&mut lines);
        let spans = detect_article_spans(&lines, 14);
        assert_eq!(
            spans
                .iter()
                .map(|span| span.start_page_index)
                .collect::<Vec<_>>(),
            vec![0, 6, 8]
        );
    }
}
