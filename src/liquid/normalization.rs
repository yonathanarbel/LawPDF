//! Local post-classification normalization passes for Liquid Mode.
//!
//! This module owns the ordered local cleanup pipeline after paragraph
//! classification: abstract/front-matter folding, reader-aid folding,
//! caption/source cleanup, end-matter/reference collapsing, duplicate pull
//! quote suppression, lead promotion, and section break insertion.

use crate::liquid::classification::{
    looks_like_abstract, looks_like_author_info, looks_like_caption, looks_like_clause,
    looks_like_dissent_or_concurrence_heading, looks_like_front_matter_metadata,
    looks_like_heading, looks_like_news_kicker_metadata, looks_like_standalone_author_line,
    looks_like_toc_entry, starts_article_transition, starts_with_lettered_heading,
    starts_with_numbered_heading,
};
use crate::liquid::cleaning::{
    looks_like_citation_footnote_line, looks_like_footnote_line, split_note_marker,
};
use crate::liquid::config::{MAX_KEY_TERM_SECTION_BLOCKS, MAX_READER_AID_SECTION_BLOCKS};
use crate::liquid::model::{DocumentProfileKind, LiquidBlock, LiquidBlockRole};
use crate::liquid::util::{
    should_preserve_terminal_hyphen, starts_with_roman_heading, title_case_ratio, uppercase_ratio,
    word_count,
};

use super::{
    contains_reference_year, end_matter_label, front_matter_label_for_text,
    is_non_title_heading_text, normalize_reference_heading, normalize_title_key,
    push_section_break_if_needed,
};

pub(super) fn run_local_normalization(blocks: Vec<LiquidBlock>) -> Vec<LiquidBlock> {
    let blocks = normalize_local_abstract_sections(blocks);
    let blocks = normalize_local_front_matter_metadata_sections(blocks);
    let blocks = collapse_local_table_of_contents_sections(blocks);
    let blocks = normalize_local_reader_aid_sections(blocks);
    let blocks = normalize_local_key_term_sections(blocks);
    let blocks = normalize_local_caption_source_lines(blocks);
    let blocks = normalize_local_tables(blocks);
    let blocks = clean_inline_footnote_markers_in_body_text(blocks);
    let blocks = normalize_inline_footnote_reference_fragments(blocks);
    let blocks = collapse_local_end_matter_sections(blocks);
    let blocks = collapse_local_reference_sections(blocks);
    let blocks = suppress_duplicate_pull_quotes(blocks);
    let blocks = promote_local_standfirst(blocks);
    let blocks = promote_local_lead(blocks);
    insert_local_section_breaks(blocks)
}

pub(super) fn run_profile_specific_normalization(
    blocks: Vec<LiquidBlock>,
    kind: DocumentProfileKind,
) -> Vec<LiquidBlock> {
    let blocks = normalize_general_structured_blocks(blocks);
    match kind {
        DocumentProfileKind::CvOrAcademicPacket => normalize_cv_academic_blocks(blocks),
        DocumentProfileKind::CourseOrExamMaterial => normalize_course_blocks(blocks),
        DocumentProfileKind::BookOrChapter => normalize_book_chapter_blocks(blocks),
        DocumentProfileKind::Contract => normalize_contract_blocks(blocks),
        DocumentProfileKind::LawReviewArticle => normalize_law_review_blocks(blocks),
        DocumentProfileKind::LegalFilingOrOpinion => normalize_legal_filing_blocks(blocks),
        DocumentProfileKind::ReceiptInvoiceFinancial => normalize_receipt_financial_blocks(blocks),
        DocumentProfileKind::PolicyReport => normalize_policy_report_blocks(blocks),
        DocumentProfileKind::FormReceiptAdmin => blocks,
        DocumentProfileKind::GeneralDocument | DocumentProfileKind::Other => blocks,
        _ => blocks,
    }
}

fn normalize_law_review_blocks(blocks: Vec<LiquidBlock>) -> Vec<LiquidBlock> {
    let mut blocks = blocks
        .into_iter()
        .map(|mut block| {
            if block.role == LiquidBlockRole::Footnote {
                block.role = LiquidBlockRole::Marginalia;
                block.label = Some("Footnote".to_owned());
            }
            block
        })
        .collect::<Vec<_>>();
    repair_interrupted_law_review_marginalia_runs(&mut blocks);
    repair_law_review_citation_runs_before_marginalia(&mut blocks);
    repair_repeated_marker_law_review_marginalia_lines(&mut blocks);
    let blocks = repair_law_review_inline_body_note_fragments(blocks);
    let blocks = strip_law_review_repository_front_matter(blocks);
    let mut blocks = repair_law_review_visible_role_noise(blocks);
    repair_law_review_author_note_continuation_runs(&mut blocks);
    let blocks = remove_section_breaks_inside_marginalia_runs(blocks);
    merge_law_review_marginalia_note_blocks(blocks)
}

fn merge_law_review_marginalia_note_blocks(blocks: Vec<LiquidBlock>) -> Vec<LiquidBlock> {
    let mut output = Vec::new();
    let mut pending: Option<(LiquidBlock, Option<usize>)> = None;

    for block in blocks {
        if block.role != LiquidBlockRole::Marginalia {
            flush_pending_marginalia(&mut output, &mut pending);
            output.push(block);
            continue;
        }

        let marker = law_review_marginalia_note_marker(&block.text);
        match pending.as_mut() {
            None => pending = Some((block, marker)),
            Some((current, current_marker)) => {
                if marginalia_block_starts_new_note(marker, *current_marker) {
                    flush_pending_marginalia(&mut output, &mut pending);
                    pending = Some((block, marker));
                } else {
                    append_marginalia_continuation(current, &block.text);
                    if current_marker.is_none() {
                        *current_marker = marker;
                    }
                }
            }
        }
    }

    flush_pending_marginalia(&mut output, &mut pending);
    output
}

fn flush_pending_marginalia(
    output: &mut Vec<LiquidBlock>,
    pending: &mut Option<(LiquidBlock, Option<usize>)>,
) {
    if let Some((mut block, _)) = pending.take() {
        block.label = Some("Footnote".to_owned());
        output.push(block);
    }
}

fn marginalia_block_starts_new_note(marker: Option<usize>, current_marker: Option<usize>) -> bool {
    let Some(marker) = marker else {
        return false;
    };
    let Some(current_marker) = current_marker else {
        return true;
    };
    marker != current_marker
}

fn append_marginalia_continuation(current: &mut LiquidBlock, continuation: &str) {
    let next = continuation.trim();
    if next.is_empty() {
        return;
    }
    let current_text = current.text.trim_end();
    if current_text.is_empty() {
        current.text = next.to_owned();
        return;
    }
    if current_text.ends_with('-') && !should_preserve_terminal_hyphen(current_text, next) {
        current.text.truncate(current_text.len() - 1);
        current.text.push_str(next);
    } else {
        current.text.truncate(current_text.len());
        current.text.push(' ');
        current.text.push_str(next);
    }
}

fn law_review_marginalia_note_marker(text: &str) -> Option<usize> {
    let trimmed = text.trim();
    if trimmed.starts_with('*') {
        return Some(0);
    }
    let (Some(marker), body) = split_note_marker(trimmed) else {
        return None;
    };
    let marker = marker.parse::<usize>().ok()?;
    if marker == 0 {
        return None;
    }
    if looks_like_footnote_line(trimmed) || marginalia_note_body_can_start_short_note(body) {
        return Some(marker);
    }
    None
}

fn marginalia_note_body_can_start_short_note(body: &str) -> bool {
    let body = body.trim();
    if body.is_empty() {
        return false;
    }
    let lower = body.to_ascii_lowercase();
    if [
        "see ",
        "see, ",
        "see e.g.",
        "see, e.g.",
        "cf. ",
        "accord ",
        "but see ",
        "id.",
        "id ",
        "supra ",
        "infra ",
    ]
    .iter()
    .any(|prefix| lower.starts_with(prefix))
    {
        return true;
    }
    body.chars()
        .find(|ch| ch.is_alphabetic())
        .is_some_and(|ch| ch.is_uppercase())
        && word_count(body) >= 2
}

fn repair_repeated_marker_law_review_marginalia_lines(blocks: &mut [LiquidBlock]) {
    let mut current_marker: Option<usize> = None;

    for block in blocks {
        if block.role == LiquidBlockRole::SectionBreak {
            continue;
        }

        if block.role == LiquidBlockRole::Marginalia {
            current_marker = law_review_marginalia_note_marker(&block.text)
                .or_else(|| leading_note_marker_value(&block.text));
            continue;
        }

        let Some(marker) = current_marker else {
            continue;
        };

        if !is_law_review_marginalia_run_candidate(block)
            && !is_law_review_citation_note_continuation_candidate(block)
        {
            current_marker = None;
            continue;
        }

        if leading_note_marker_value(&block.text) == Some(marker) {
            block.role = LiquidBlockRole::Marginalia;
            block.label = Some("Footnote".to_owned());
            continue;
        }

        current_marker = None;
    }
}

fn leading_note_marker_value(text: &str) -> Option<usize> {
    let (Some(marker), body) = split_note_marker(text.trim()) else {
        return None;
    };
    if body.trim().is_empty() {
        return None;
    }
    let marker = marker.parse::<usize>().ok()?;
    (marker > 0).then_some(marker)
}

fn repair_interrupted_law_review_marginalia_runs(blocks: &mut [LiquidBlock]) {
    let mut index = 0usize;
    while index < blocks.len() {
        if !is_law_review_marginalia_run_candidate(&blocks[index])
            || !nearest_non_section_before_is(blocks, index, LiquidBlockRole::Marginalia)
        {
            index += 1;
            continue;
        }

        let start = index;
        let mut end = index;
        let mut candidate_count = 0usize;
        while end < blocks.len()
            && (blocks[end].role == LiquidBlockRole::SectionBreak
                || is_law_review_marginalia_run_candidate(&blocks[end]))
        {
            if blocks[end].role != LiquidBlockRole::SectionBreak {
                candidate_count += 1;
            }
            end += 1;
        }

        if (1..=8).contains(&candidate_count)
            && nearest_non_section_at_or_after_is(blocks, end, LiquidBlockRole::Marginalia)
        {
            for block in blocks[start..end]
                .iter_mut()
                .filter(|block| block.role != LiquidBlockRole::SectionBreak)
            {
                block.role = LiquidBlockRole::Marginalia;
                block.label = Some("Footnote".to_owned());
            }
        }

        index = end.max(index + 1);
    }
}

fn is_law_review_marginalia_run_candidate(block: &LiquidBlock) -> bool {
    if !matches!(
        block.role,
        LiquidBlockRole::Paragraph
            | LiquidBlockRole::Table
            | LiquidBlockRole::ListItem
            | LiquidBlockRole::Heading
            | LiquidBlockRole::Subheading
    ) {
        return false;
    }

    let text = block.text.trim();
    !text.is_empty()
        && text.chars().count() <= 240
        && word_count(text) <= 34
        && !looks_like_front_matter_metadata(text)
        && !looks_like_toc_entry(text)
        && !looks_like_abstract(text)
}

fn repair_law_review_author_note_continuation_runs(blocks: &mut [LiquidBlock]) {
    let mut index = 0usize;
    while index < blocks.len() {
        if blocks[index].role != LiquidBlockRole::Marginalia
            || !looks_like_law_review_symbol_note_fragment(&blocks[index].text)
        {
            index += 1;
            continue;
        }

        let mut cursor = index + 1;
        let mut candidates = Vec::new();
        let mut has_author_note_cue =
            looks_like_law_review_author_note_continuation(&blocks[index].text);
        while cursor < blocks.len() && candidates.len() < 8 {
            if blocks[cursor].role == LiquidBlockRole::SectionBreak {
                cursor += 1;
                continue;
            }
            if blocks[cursor].role == LiquidBlockRole::Marginalia {
                break;
            }
            if !is_law_review_author_note_continuation_candidate(&blocks[cursor]) {
                candidates.clear();
                break;
            }
            has_author_note_cue |=
                looks_like_law_review_author_note_continuation(&blocks[cursor].text);
            candidates.push(cursor);
            cursor += 1;
        }

        if !candidates.is_empty()
            && has_author_note_cue
            && blocks
                .get(cursor)
                .is_some_and(|block| block.role == LiquidBlockRole::Marginalia)
        {
            for candidate in candidates {
                blocks[candidate].role = LiquidBlockRole::Marginalia;
                blocks[candidate].label = Some("Footnote".to_owned());
            }
        }

        index = cursor.max(index + 1);
    }
}

fn is_law_review_author_note_continuation_candidate(block: &LiquidBlock) -> bool {
    if !matches!(
        block.role,
        LiquidBlockRole::Paragraph | LiquidBlockRole::Heading | LiquidBlockRole::Subheading
    ) {
        return false;
    }
    let text = block.text.trim();
    !text.is_empty()
        && text.chars().count() <= 220
        && word_count(text) <= 28
        && !looks_like_front_matter_metadata(text)
        && !looks_like_toc_entry(text)
        && !looks_like_abstract(text)
        && !starts_with_roman_heading(text)
        && !starts_with_lettered_heading(text)
        && !starts_with_numbered_heading(text)
        && !starts_article_transition(text)
        && end_matter_label(text).is_none()
}

fn looks_like_law_review_author_note_continuation(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    [
        "colleague",
        "comment",
        "conversation",
        "faculty workshop",
        "grateful",
        "helpful",
        "law school",
        "research assistance",
        "thank",
        "university",
        "workshop",
    ]
    .iter()
    .any(|cue| lower.contains(cue))
}

fn repair_law_review_citation_runs_before_marginalia(blocks: &mut [LiquidBlock]) {
    let mut index = 0usize;
    while index < blocks.len() {
        if !is_law_review_citation_note_start_candidate(&blocks[index]) {
            index += 1;
            continue;
        }

        let start = index;
        let mut end = index + 1;
        let mut candidate_count = 1usize;
        while end < blocks.len()
            && (blocks[end].role == LiquidBlockRole::SectionBreak
                || is_law_review_citation_note_continuation_candidate(&blocks[end]))
        {
            if blocks[end].role != LiquidBlockRole::SectionBreak {
                candidate_count += 1;
            }
            end += 1;
        }

        if (1..=8).contains(&candidate_count)
            && nearest_non_section_at_or_after_is(blocks, end, LiquidBlockRole::Marginalia)
        {
            for block in blocks[start..end]
                .iter_mut()
                .filter(|block| block.role != LiquidBlockRole::SectionBreak)
            {
                block.role = LiquidBlockRole::Marginalia;
                block.label = Some("Footnote".to_owned());
            }
        }

        index = end.max(index + 1);
    }
}

fn is_law_review_citation_note_start_candidate(block: &LiquidBlock) -> bool {
    if !matches!(
        block.role,
        LiquidBlockRole::Paragraph
            | LiquidBlockRole::Table
            | LiquidBlockRole::ListItem
            | LiquidBlockRole::Heading
            | LiquidBlockRole::Subheading
            | LiquidBlockRole::Header
    ) {
        return false;
    }
    let text = block.text.trim();
    looks_like_citation_footnote_line(text)
        || looks_like_general_citation_footnote_start(text)
        || looks_like_multidigit_footnote_start(text)
}

fn is_law_review_citation_note_continuation_candidate(block: &LiquidBlock) -> bool {
    if !matches!(
        block.role,
        LiquidBlockRole::Paragraph
            | LiquidBlockRole::Table
            | LiquidBlockRole::ListItem
            | LiquidBlockRole::Heading
            | LiquidBlockRole::Subheading
            | LiquidBlockRole::Header
    ) {
        return false;
    }
    let text = block.text.trim();
    !text.is_empty()
        && text.chars().count() <= 260
        && word_count(text) <= 42
        && !looks_like_front_matter_metadata(text)
        && !looks_like_toc_entry(text)
        && !looks_like_abstract(text)
}

fn looks_like_general_citation_footnote_start(text: &str) -> bool {
    if !looks_like_footnote_line(text) {
        return false;
    }
    let (_, body) = split_note_marker(text);
    let lower = body.to_ascii_lowercase();
    let citation_cue = [
        "http://",
        "https://",
        "www.",
        "journal",
        "l. rev",
        "law review",
        "rev.",
        "univ.",
        "press",
        "forbes",
        "pew res",
        "res.ctr",
        "res. ctr",
        "news",
        "times",
        "post",
    ]
    .iter()
    .any(|needle| lower.contains(needle));
    let year_or_date = [
        "(19", "(20", " 19", " 20", "(jan.", "(feb.", "(mar.", "(apr.", "(may", "(jun.", "(jul.",
        "(aug.", "(sep.", "(sept.", "(oct.", "(nov.", "(dec.",
    ]
    .iter()
    .any(|needle| lower.contains(needle));
    citation_cue || year_or_date && body.contains(',')
}

fn looks_like_multidigit_footnote_start(text: &str) -> bool {
    if !looks_like_footnote_line(text) {
        return false;
    }
    split_note_marker(text)
        .0
        .is_some_and(|marker| marker.chars().filter(|ch| ch.is_ascii_digit()).count() >= 2)
}

fn repair_law_review_inline_body_note_fragments(blocks: Vec<LiquidBlock>) -> Vec<LiquidBlock> {
    let mut blocks = blocks;
    let mut remove = vec![false; blocks.len()];

    for index in 0..blocks.len() {
        let Some(body) = inline_body_note_bridge_fragment(&blocks[index]).map(str::to_owned) else {
            continue;
        };
        let Some(next_index) = next_non_section_index(&blocks, index + 1) else {
            continue;
        };
        if !matches!(
            blocks[next_index].role,
            LiquidBlockRole::Paragraph | LiquidBlockRole::Lead | LiquidBlockRole::Quote
        ) {
            continue;
        }
        if inject_inline_body_note_bridge(&mut blocks[next_index].text, &body) {
            remove[index] = true;
        }
    }

    blocks
        .into_iter()
        .enumerate()
        .filter_map(|(index, block)| (!remove[index]).then_some(block))
        .collect()
}

fn inline_body_note_bridge_fragment(block: &LiquidBlock) -> Option<&str> {
    if !matches!(
        block.role,
        LiquidBlockRole::Paragraph
            | LiquidBlockRole::Table
            | LiquidBlockRole::ListItem
            | LiquidBlockRole::Header
            | LiquidBlockRole::Subheading
    ) {
        return None;
    }
    let text = block.text.trim();
    let (marker, body) = split_note_marker(text);
    let marker = marker?;
    if marker.chars().filter(|ch| ch.is_ascii_digit()).count() < 2 {
        return None;
    }
    if text[marker.len()..]
        .chars()
        .next()
        .is_some_and(|ch| matches!(ch, '.' | ')' | ']'))
    {
        return None;
    }
    let body = body.trim();
    if word_count(body) != 1 {
        return None;
    }
    let lower = body.to_ascii_lowercase();
    matches!(lower.as_str(), "for").then_some(body)
}

fn inject_inline_body_note_bridge(text: &mut String, body: &str) -> bool {
    if !body.eq_ignore_ascii_case("for") {
        return false;
    }
    let lower = text.to_ascii_lowercase();
    for pattern in [". example", "? example", "! example"] {
        if let Some(index) = lower.find(pattern) {
            let insert_at = index + 2;
            text.insert_str(insert_at, "For ");
            return true;
        }
    }
    false
}

fn strip_law_review_repository_front_matter(blocks: Vec<LiquidBlock>) -> Vec<LiquidBlock> {
    if !has_law_review_repository_front_matter_context(&blocks) {
        return blocks;
    }

    let title_key = blocks
        .iter()
        .find(|block| block.role == LiquidBlockRole::Title)
        .map(|block| normalize_title_key(&block.text))
        .unwrap_or_default();
    let mut output = Vec::with_capacity(blocks.len());

    for (index, mut block) in blocks.into_iter().enumerate() {
        if index <= 45 && should_strip_law_review_repository_front_block(&block, &title_key) {
            continue;
        }
        if index <= 45 && should_demote_repository_front_heading_to_author_info(&block) {
            block.role = LiquidBlockRole::AuthorInfo;
            block.label = None;
            block.text = block
                .text
                .trim()
                .trim_start_matches("by ")
                .trim_start_matches("By ")
                .trim()
                .to_owned();
        }
        output.push(block);
    }

    remove_redundant_section_breaks(output)
}

fn has_law_review_repository_front_matter_context(blocks: &[LiquidBlock]) -> bool {
    let mut issue_descriptor_count = 0usize;
    let mut has_non_article_issue_descriptor = false;
    for block in blocks.iter().take(45) {
        let text = block.text.trim();
        if looks_like_repository_scaffold_text(text) {
            return true;
        }
        if looks_like_repository_issue_descriptor(text) {
            issue_descriptor_count += 1;
            if !text.to_ascii_lowercase().starts_with("article ") {
                has_non_article_issue_descriptor = true;
            }
        }
    }

    issue_descriptor_count >= 2 && has_non_article_issue_descriptor
}

fn should_strip_law_review_repository_front_block(block: &LiquidBlock, title_key: &str) -> bool {
    if matches!(
        block.role,
        LiquidBlockRole::Title | LiquidBlockRole::SectionBreak
    ) {
        return false;
    }
    let text = block.text.trim();
    if text.is_empty() {
        return true;
    }
    if looks_like_repository_scaffold_text(text)
        || looks_like_repository_issue_descriptor(text)
        || looks_like_repository_citation_block(text)
    {
        return true;
    }
    if text.eq_ignore_ascii_case("and") {
        return true;
    }
    let key = normalize_title_key(text);
    !title_key.is_empty() && key.len() >= 4 && title_key.contains(&key)
}

fn should_demote_repository_front_heading_to_author_info(block: &LiquidBlock) -> bool {
    if !matches!(
        block.role,
        LiquidBlockRole::Heading | LiquidBlockRole::Subheading
    ) {
        return false;
    }
    let text = block.text.trim();
    let lower = text.to_ascii_lowercase();
    (lower.starts_with("by ") && word_count(text) <= 12)
        || (text.ends_with('*') && word_count(text) <= 10)
        || (lower.contains(" law school") && word_count(text) <= 8)
        || looks_like_standalone_author_line(text, 0)
}

fn looks_like_repository_scaffold_text(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower == "recommended citation"
        || lower == "repository citation"
        || lower.starts_with("follow this and additional works at:")
        || lower.starts_with("available at:")
            && (lower.contains("digitalcommons")
                || lower.contains("ecommons")
                || lower.contains("/lawreview/")
                || lower.contains("lawreview/"))
        || lower.starts_with("part of the") && lower.contains(" commons")
        || lower.contains("brought to you for free and open access")
        || lower.contains("accepted for inclusion")
        || lower.contains("authorized administrator")
        || lower.contains("authorized editor")
        || lower.contains("repository@")
        || lower.contains("law-library@")
        || lower.contains("digital commons")
        || lower.contains("law ecommons")
        || lower.contains("ecommons. for more information")
}

fn looks_like_repository_issue_descriptor(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    if lower.starts_with("volume ") && word_count(text) <= 4 {
        return true;
    }
    if lower.starts_with("issue ") && lower.contains("article") && word_count(text) <= 8 {
        return true;
    }
    if lower.starts_with("number ") && word_count(text) <= 8 {
        return true;
    }
    lower.starts_with("article ")
        && word_count(text) <= 3
        && text.chars().any(|ch| ch.is_ascii_digit())
}

fn looks_like_repository_citation_block(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains(" law review:") && lower.contains("article")
        || lower.contains("loy. u. chi. l. j.")
        || lower.contains("available at:")
        || lower.contains("recommended citation")
}

fn remove_redundant_section_breaks(blocks: Vec<LiquidBlock>) -> Vec<LiquidBlock> {
    let mut output = Vec::with_capacity(blocks.len());
    for block in blocks {
        if block.role == LiquidBlockRole::SectionBreak
            && (output.is_empty()
                || output.last().is_some_and(|previous: &LiquidBlock| {
                    previous.role == LiquidBlockRole::SectionBreak
                }))
        {
            continue;
        }
        output.push(block);
    }
    while output
        .last()
        .is_some_and(|block| block.role == LiquidBlockRole::SectionBreak)
    {
        output.pop();
    }
    output
}

fn repair_law_review_visible_role_noise(blocks: Vec<LiquidBlock>) -> Vec<LiquidBlock> {
    let title_key = blocks
        .iter()
        .find(|block| block.role == LiquidBlockRole::Title)
        .map(|block| normalize_title_key(&block.text))
        .unwrap_or_default();

    blocks
        .into_iter()
        .map(|mut block| {
            let text = block.text.trim();
            if text.is_empty() {
                return block;
            }
            if !title_key.is_empty()
                && matches!(
                    block.role,
                    LiquidBlockRole::Heading | LiquidBlockRole::Subheading
                )
                && normalize_title_key(text) == title_key
            {
                block.role = LiquidBlockRole::Header;
                block.label = None;
                return block;
            }
            if looks_like_law_review_running_page_cite_artifact(text)
                && matches!(
                    block.role,
                    LiquidBlockRole::Table
                        | LiquidBlockRole::Paragraph
                        | LiquidBlockRole::ListItem
                        | LiquidBlockRole::Heading
                        | LiquidBlockRole::Subheading
                        | LiquidBlockRole::Header
                )
            {
                block.role = LiquidBlockRole::Header;
                block.label = None;
                return block;
            }
            if block.role == LiquidBlockRole::Issue
                && looks_like_law_review_lettered_question_subheading(text)
            {
                block.role = LiquidBlockRole::Subheading;
                block.label = None;
                return block;
            }
            if block.role == LiquidBlockRole::Paragraph
                && looks_like_law_review_symbol_note_fragment(text)
            {
                block.role = LiquidBlockRole::Marginalia;
                block.label = Some("Footnote".to_owned());
                return block;
            }
            if is_law_review_visible_noise_role(block.role)
                && (looks_like_law_review_standalone_footnote_fragment(text)
                    || looks_like_law_review_symbol_note_fragment(text)
                    || looks_like_law_review_split_numeric_note_marker(text)
                    || looks_like_law_review_short_numeric_note_fragment(text)
                    || looks_like_law_review_mangled_footnote_fragment(text)
                    || looks_like_law_review_citation_tail_fragment(text)
                    || looks_like_law_review_citation_title_fragment(text)
                    || looks_like_law_review_unmarked_citation_note_fragment(text, block.role)
                    || (block.role == LiquidBlockRole::Table
                        && looks_like_law_review_single_digit_table_note_fragment(text)))
            {
                block.role = LiquidBlockRole::Marginalia;
                block.label = Some("Footnote".to_owned());
                return block;
            }
            if is_law_review_visible_noise_role(block.role)
                && looks_like_law_review_single_marker_body_fragment(text)
            {
                block.role = LiquidBlockRole::Paragraph;
                block.label = None;
                return block;
            }
            if is_law_review_visible_noise_role(block.role)
                && looks_like_law_review_table_fragment(text)
            {
                block.role = LiquidBlockRole::Table;
                block.label = Some("Table".to_owned());
                return block;
            }
            if block.role == LiquidBlockRole::Takeaway {
                block.role = LiquidBlockRole::Paragraph;
                block.label = None;
                return block;
            }
            if is_law_review_visible_noise_role(block.role)
                && looks_like_law_review_body_flow_fragment(text, block.role)
            {
                block.role = LiquidBlockRole::Paragraph;
                block.label = None;
            }
            block
        })
        .collect()
}

fn is_law_review_visible_noise_role(role: LiquidBlockRole) -> bool {
    matches!(
        role,
        LiquidBlockRole::Heading
            | LiquidBlockRole::Subheading
            | LiquidBlockRole::Header
            | LiquidBlockRole::Table
            | LiquidBlockRole::Noise
            | LiquidBlockRole::ListItem
            | LiquidBlockRole::Issue
            | LiquidBlockRole::Takeaway
            | LiquidBlockRole::Definition
            | LiquidBlockRole::Holding
            | LiquidBlockRole::KeyClause
            | LiquidBlockRole::Clause
    )
}

fn looks_like_law_review_standalone_footnote_fragment(text: &str) -> bool {
    let (Some(marker), body) = split_note_marker(text) else {
        return false;
    };
    let digit_count = marker.chars().filter(|ch| ch.is_ascii_digit()).count();
    if digit_count < 2 || looks_like_year_marker(marker) {
        return false;
    }

    let lower_body = body.trim_start().to_ascii_lowercase();
    if lower_body.starts_with("u.s.c")
        || lower_body.starts_with("c.f.r")
        || lower_body.starts_with("bankruptcy protection")
        || lower_body.starts_with("bankruptcy petition")
    {
        return false;
    }

    looks_like_footnote_line(text)
        || looks_like_citation_footnote_line(text)
        || (word_count(body) >= 7
            && [
                "furthermore",
                "however",
                "moreover",
                "nevertheless",
                "notwithstanding",
                "although",
            ]
            .iter()
            .any(|prefix| lower_body.starts_with(prefix)))
}

fn looks_like_law_review_symbol_note_fragment(text: &str) -> bool {
    let trimmed = text.trim_start();
    if !trimmed.starts_with('*') {
        return false;
    };
    let body = trimmed.trim_start_matches('*').trim_start();
    if word_count(body) < 6 || word_count(body) > 60 {
        return false;
    }
    let lower = body.to_ascii_lowercase();
    lower.contains("university")
        || lower.contains("college")
        || lower.contains("law school")
        || lower.contains("school of law")
        || lower.contains("j.d.")
        || lower.contains("ll.m")
        || lower.contains("b.a.")
}

fn looks_like_law_review_split_numeric_note_marker(text: &str) -> bool {
    let mut parts = text.split_whitespace();
    let Some(first) = parts.next() else {
        return false;
    };
    let Some(second) = parts.next() else {
        return false;
    };
    if first.len() != 1 || !first.chars().all(|ch| ch.is_ascii_digit()) {
        return false;
    }
    let second_digits = second.trim_end_matches('.');
    if second_digits.is_empty()
        || second_digits.len() > 3
        || !second.ends_with('.')
        || !second_digits.chars().all(|ch| ch.is_ascii_digit())
    {
        return false;
    }
    let rest = parts.collect::<Vec<_>>().join(" ");
    if word_count(&rest) < 3 {
        return false;
    }
    let lower = rest.to_ascii_lowercase();
    lower.starts_with("see ")
        || lower.starts_with("see, ")
        || lower.starts_with("as ")
        || lower.starts_with("cf. ")
        || lower.contains("supra")
        || contains_reference_year(&rest)
}

fn looks_like_law_review_short_numeric_note_fragment(text: &str) -> bool {
    let (Some(marker), body) = split_note_marker(text) else {
        return false;
    };
    if marker.len() > 4
        || marker.chars().filter(|ch| ch.is_ascii_digit()).count() < 2
        || looks_like_year_marker(marker)
    {
        return false;
    }
    let body = body.trim_start();
    if body.is_empty() || word_count(body) > 12 {
        return false;
    }
    let lower = body.to_ascii_lowercase();
    [
        "accord ", "at ", "but see ", "cf. ", "contra ", "e.g.", "id.", "id ", "infra ", "see ",
        "see, ", "supra ",
    ]
    .iter()
    .any(|prefix| lower.starts_with(prefix))
}

fn looks_like_law_review_mangled_footnote_fragment(text: &str) -> bool {
    let (Some(marker), body) = split_note_marker(text) else {
        return false;
    };
    if marker.chars().filter(|ch| ch.is_ascii_digit()).count() < 2 || looks_like_year_marker(marker)
    {
        return false;
    }
    let body = body.trim_start();
    if word_count(body) < 6 {
        return false;
    }
    let lower_body = body.to_ascii_lowercase();
    lower_body.contains("sign-in-wrap")
        || lower_body.contains("browsewrap")
        || lower_body.contains("clickwrap")
        || lower_body.contains("wrap agreement")
        || lower_body.contains("website") && lower_body.contains("contract")
}

fn looks_like_law_review_single_marker_body_fragment(text: &str) -> bool {
    if looks_like_single_marker_enumerated_item(text) {
        return false;
    }
    let (Some(marker), body) = split_note_marker(text) else {
        return false;
    };
    if marker.chars().filter(|ch| ch.is_ascii_digit()).count() > 2 || looks_like_year_marker(marker)
    {
        return false;
    }
    let body = body.trim_start();
    if word_count(body) < 5 {
        return false;
    }
    let lower = body.to_ascii_lowercase();
    [
        "once ",
        "though ",
        "that ",
        "the ",
        "these ",
        "this ",
        "students ",
        "understand ",
        "professors ",
    ]
    .iter()
    .any(|prefix| lower.starts_with(prefix))
}

fn looks_like_law_review_citation_tail_fragment(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.len() > 80 || word_count(trimmed) > 8 {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("perma.cc") || lower.contains("http://") || lower.contains("https://") {
        return true;
    }
    let digit_count = trimmed.chars().filter(|ch| ch.is_ascii_digit()).count();
    digit_count >= 2
        && trimmed.ends_with("].")
        && trimmed.chars().any(|ch| matches!(ch, '-' | '/'))
        && trimmed
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '/' | '[' | ']' | '.' | ' '))
}

fn looks_like_law_review_citation_title_fragment(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.len() < 35
        || trimmed.len() > 220
        || starts_with_lettered_heading(trimmed)
        || starts_with_roman_heading(trimmed)
        || starts_with_numbered_heading(trimmed)
    {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("http://")
        || lower.contains("https://")
        || lower.contains("perma.cc")
        || lower.contains("law blog")
        || lower.contains("l. rev.")
        || lower.contains("law review")
        || lower.contains("lexis")
    {
        return true;
    }
    let trailing_comma_page = trimmed
        .rsplit_once(',')
        .is_some_and(|(_, tail)| tail.trim().chars().all(|ch| ch.is_ascii_digit()));
    trailing_comma_page
        && (lower.contains("clickwrap")
            || lower.contains("browsewrap")
            || lower.contains("reasonably communicated"))
}

fn looks_like_law_review_unmarked_citation_note_fragment(
    text: &str,
    role: LiquidBlockRole,
) -> bool {
    if !matches!(
        role,
        LiquidBlockRole::Heading
            | LiquidBlockRole::Subheading
            | LiquidBlockRole::Header
            | LiquidBlockRole::Table
            | LiquidBlockRole::Issue
            | LiquidBlockRole::Takeaway
    ) || starts_with_lettered_heading(text)
        || starts_with_roman_heading(text)
        || starts_with_numbered_heading(text)
    {
        return false;
    }
    let trimmed = text.trim();
    if trimmed.len() > 260 || word_count(trimmed) > 34 {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    let starts_with_citation_signal = [
        "see ",
        "see, ",
        "see e.g.",
        "see, e.g.",
        "but see ",
        "cf. ",
        "accord ",
    ]
    .iter()
    .any(|prefix| lower.starts_with(prefix));
    if starts_with_citation_signal
        && word_count(trimmed) >= 5
        && (contains_reference_year(trimmed)
            || lower.contains("supra")
            || lower.contains(" at ")
            || trimmed.contains(','))
    {
        return true;
    }

    let digit_count = trimmed.chars().filter(|ch| ch.is_ascii_digit()).count();
    digit_count >= 3
        && (contains_reference_year(trimmed) || lower.contains(" supra ") || lower.contains(" at "))
        && (trimmed.contains(';') || trimmed.matches(',').count() >= 2)
        && uppercase_letter_ratio(trimmed) >= 0.45
}

fn uppercase_letter_ratio(text: &str) -> f32 {
    let mut letters = 0usize;
    let mut uppercase = 0usize;
    for ch in text.chars().filter(|ch| ch.is_ascii_alphabetic()) {
        letters += 1;
        if ch.is_ascii_uppercase() {
            uppercase += 1;
        }
    }
    if letters == 0 {
        0.0
    } else {
        uppercase as f32 / letters as f32
    }
}

fn looks_like_law_review_lettered_question_subheading(text: &str) -> bool {
    text.trim_end().ends_with('?')
        && word_count(text) <= 18
        && (starts_with_lettered_heading(text)
            || starts_with_numbered_heading(text)
            || starts_with_roman_heading(text))
}

fn looks_like_law_review_table_fragment(text: &str) -> bool {
    let trimmed = text.trim();
    if split_note_marker(trimmed).0.is_some() {
        return false;
    }
    if trimmed.len() > 180
        || word_count(trimmed) > 18
        || starts_with_lettered_heading(trimmed)
        || starts_with_numbered_heading(trimmed)
        || starts_with_roman_heading(trimmed)
    {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("mean") && lower.contains("median") && lower.contains("standard deviation") {
        return true;
    }
    let digit_count = trimmed.chars().filter(|ch| ch.is_ascii_digit()).count();
    if digit_count < 4 {
        return false;
    }
    let numeric_tokens = trimmed
        .split_whitespace()
        .filter(|token| token.chars().any(|ch| ch.is_ascii_digit()))
        .count();
    numeric_tokens >= 2
        && (lower.contains("visitors")
            || lower.contains("pageviews")
            || lower.contains("average")
            || lower.contains("median")
            || lower.contains("standard")
            || lower.contains("score")
            || lower.contains('%'))
}

fn looks_like_law_review_body_flow_fragment(text: &str, role: LiquidBlockRole) -> bool {
    let case_name_continuation = looks_like_law_review_case_name_continuation(text);
    if looks_like_law_review_running_page_cite_artifact(text)
        || looks_like_front_matter_metadata(text)
        || looks_like_toc_entry(text)
        || looks_like_abstract(text)
        || (!case_name_continuation && starts_with_roman_heading(text))
        || (!case_name_continuation && starts_with_lettered_heading(text))
        || starts_article_transition(text)
        || end_matter_label(text).is_some()
    {
        return false;
    }
    if starts_with_numbered_heading(text) && !looks_like_numeric_body_continuation(text) {
        return false;
    }
    if role == LiquidBlockRole::ListItem && looks_like_single_marker_enumerated_item(text) {
        return false;
    }

    if matches!(
        role,
        LiquidBlockRole::Definition
            | LiquidBlockRole::Holding
            | LiquidBlockRole::KeyClause
            | LiquidBlockRole::Clause
    ) {
        return word_count(text) >= 5;
    }

    if looks_like_numeric_body_continuation(text) {
        return true;
    }
    if first_alphabetic_is_lowercase(text) {
        return true;
    }
    if looks_like_short_inline_reference_tail(text) {
        return true;
    }
    if matches!(role, LiquidBlockRole::Heading | LiquidBlockRole::Subheading)
        && (looks_like_law_review_body_question_fragment(text)
            || looks_like_law_review_attached_note_marker_fragment(text))
    {
        return true;
    }
    if role == LiquidBlockRole::Table {
        let (marker, body) = split_note_marker(text);
        if marker.is_some()
            && marker
                .is_some_and(|value| value.chars().filter(|ch| ch.is_ascii_digit()).count() == 1)
            && word_count(body) >= 4
            && first_alphabetic_is_uppercase(body)
        {
            return true;
        }
    }
    if matches!(role, LiquidBlockRole::Heading | LiquidBlockRole::Subheading)
        && ((word_count(text) >= 8
            && (text.ends_with(',')
                || text.ends_with(",\"")
                || text.ends_with(",'")
                || text.contains(" v. ")
                || text.contains(" (In re ")))
            || looks_like_law_review_sentence_fragment_heading(text))
    {
        return true;
    }

    false
}

fn looks_like_law_review_body_question_fragment(text: &str) -> bool {
    word_count(text) >= 6
        && text.contains('?')
        && text.contains(',')
        && !starts_with_lettered_heading(text)
        && !starts_with_numbered_heading(text)
        && !starts_with_roman_heading(text)
}

fn looks_like_law_review_attached_note_marker_fragment(text: &str) -> bool {
    let trimmed = text.trim();
    if word_count(trimmed) > 12
        || starts_with_lettered_heading(trimmed)
        || starts_with_numbered_heading(trimmed)
        || starts_with_roman_heading(trimmed)
    {
        return false;
    }
    let mut previous = None;
    for ch in trimmed.chars() {
        if ch.is_ascii_digit() && previous.is_some_and(|prev: char| prev.is_ascii_alphabetic()) {
            return true;
        }
        previous = Some(ch);
    }
    false
}

fn looks_like_law_review_single_digit_table_note_fragment(text: &str) -> bool {
    let (Some(marker), body) = split_note_marker(text) else {
        return false;
    };
    marker.chars().filter(|ch| ch.is_ascii_digit()).count() == 1
        && word_count(body) >= 2
        && word_count(body) <= 12
        && first_alphabetic_is_uppercase(body)
        && [
            "accord ",
            "although ",
            "applying ",
            "at ",
            "but see ",
            "cf. ",
            "however ",
            "id.",
            "see ",
        ]
        .iter()
        .any(|prefix| body.trim_start().to_ascii_lowercase().starts_with(prefix))
}

fn looks_like_law_review_case_name_continuation(text: &str) -> bool {
    let lower = text.trim_start().to_ascii_lowercase();
    lower.starts_with("v. ")
        || lower.starts_with("v. ")
        || lower.contains(" (in re ")
        || lower.contains("(in re ")
}

fn looks_like_law_review_running_page_cite_artifact(text: &str) -> bool {
    let trimmed = text.trim();
    if !(4..=90).contains(&trimmed.len()) || word_count(trimmed) > 10 {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    let digit_count = trimmed.chars().filter(|ch| ch.is_ascii_digit()).count();
    (digit_count >= 4 && lower.contains("[vol."))
        || (digit_count >= 2 && lower.contains(" law review") && lower.contains("[vol."))
        || (lower.starts_with("19") || lower.starts_with("20"))
            && lower.contains(']')
            && digit_count >= 4
}

fn looks_like_numeric_body_continuation(text: &str) -> bool {
    if leading_four_digit_year(text).is_some() {
        return true;
    }
    let (Some(marker), body) = split_note_marker(text) else {
        return false;
    };
    if looks_like_year_marker(marker) {
        return true;
    }
    let lower_body = body.trim_start().to_ascii_lowercase();
    lower_body.starts_with("u.s.c")
        || lower_body.starts_with("c.f.r")
        || lower_body.starts_with("bankruptcy protection")
        || lower_body.starts_with("bankruptcy petition")
}

fn leading_four_digit_year(text: &str) -> Option<u16> {
    let trimmed = text.trim_start();
    let year_text = trimmed.get(..4)?;
    if !year_text.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    let next = trimmed.chars().nth(4)?;
    if !matches!(next, '.' | ',' | ';' | ':' | ')' | ']' | ' ') {
        return None;
    }
    year_text
        .parse::<u16>()
        .ok()
        .filter(|year| (1800..=2099).contains(year))
}

fn looks_like_single_marker_enumerated_item(text: &str) -> bool {
    let trimmed = text.trim_start();
    if let Some(rest) = trimmed.strip_prefix('(') {
        let marker_len = rest
            .chars()
            .take_while(|ch| ch.is_ascii_alphanumeric())
            .count();
        return (1..=3).contains(&marker_len) && rest.chars().nth(marker_len) == Some(')');
    }

    let digit_count = trimmed.chars().take_while(|ch| ch.is_ascii_digit()).count();
    digit_count == 1
        && trimmed
            .chars()
            .nth(digit_count)
            .is_some_and(|ch| matches!(ch, '.' | ')'))
        && word_count(trimmed) >= 3
}

fn looks_like_year_marker(marker: &str) -> bool {
    marker.len() == 4
        && marker.chars().all(|ch| ch.is_ascii_digit())
        && marker
            .parse::<u16>()
            .is_ok_and(|year| (1800..=2099).contains(&year))
}

fn first_alphabetic_is_lowercase(text: &str) -> bool {
    text.chars()
        .find(|ch| ch.is_ascii_alphabetic())
        .is_some_and(|ch| ch.is_ascii_lowercase())
}

fn first_alphabetic_is_uppercase(text: &str) -> bool {
    text.chars()
        .find(|ch| ch.is_ascii_alphabetic())
        .is_some_and(|ch| ch.is_ascii_uppercase())
}

fn looks_like_short_inline_reference_tail(text: &str) -> bool {
    word_count(text) <= 8
        && text
            .as_bytes()
            .windows(2)
            .any(|window| window[0] == b'.' && window[1].is_ascii_digit())
}

fn looks_like_law_review_sentence_fragment_heading(text: &str) -> bool {
    if word_count(text) < 7 || !text.chars().any(|ch| ch.is_ascii_lowercase()) {
        return false;
    }
    let lower = text.to_ascii_lowercase();
    [
        " had ", " has ", " was ", " were ", " would ", " could ", " should ", " the ", " to ",
        " of ",
    ]
    .iter()
    .filter(|needle| lower.contains(**needle))
    .count()
        >= 2
}

fn remove_section_breaks_inside_marginalia_runs(blocks: Vec<LiquidBlock>) -> Vec<LiquidBlock> {
    let mut output = Vec::with_capacity(blocks.len());
    for index in 0..blocks.len() {
        if blocks[index].role == LiquidBlockRole::SectionBreak
            && nearest_non_section_before_is(&blocks, index, LiquidBlockRole::Marginalia)
            && nearest_non_section_at_or_after_is(&blocks, index + 1, LiquidBlockRole::Marginalia)
        {
            continue;
        }
        output.push(blocks[index].clone());
    }
    output
}

fn nearest_non_section_before_is(
    blocks: &[LiquidBlock],
    index: usize,
    role: LiquidBlockRole,
) -> bool {
    blocks[..index]
        .iter()
        .rev()
        .find(|block| block.role != LiquidBlockRole::SectionBreak)
        .is_some_and(|block| block.role == role)
}

fn nearest_non_section_at_or_after_is(
    blocks: &[LiquidBlock],
    index: usize,
    role: LiquidBlockRole,
) -> bool {
    blocks[index..]
        .iter()
        .find(|block| block.role != LiquidBlockRole::SectionBreak)
        .is_some_and(|block| block.role == role)
}

fn next_non_section_index(blocks: &[LiquidBlock], index: usize) -> Option<usize> {
    (index..blocks.len()).find(|candidate| blocks[*candidate].role != LiquidBlockRole::SectionBreak)
}

fn normalize_legal_filing_blocks(blocks: Vec<LiquidBlock>) -> Vec<LiquidBlock> {
    if !has_legal_opinion_context(&blocks) {
        return blocks;
    }

    blocks
        .into_iter()
        .map(|mut block| {
            if matches!(
                block.role,
                LiquidBlockRole::Heading
                    | LiquidBlockRole::Subheading
                    | LiquidBlockRole::Paragraph
                    | LiquidBlockRole::ListItem
                    | LiquidBlockRole::Explainer
            ) {
                if looks_like_legal_holding_block(&block.text) {
                    block.role = LiquidBlockRole::Holding;
                    block.label = None;
                } else if looks_like_legal_issue_block(&block.text) {
                    block.role = LiquidBlockRole::Issue;
                    block.label = None;
                } else if looks_like_legal_syllabus_block(&block.text) {
                    block.role = LiquidBlockRole::Syllabus;
                    block.label = None;
                }
            }
            block
        })
        .collect()
}

fn has_legal_opinion_context(blocks: &[LiquidBlock]) -> bool {
    let text = blocks
        .iter()
        .take(80)
        .map(|block| block.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
        .to_ascii_lowercase();
    (text.contains("supreme court") || text.contains("court of appeals"))
        && (text.contains("opinion")
            || text.contains("appeal from")
            || text.contains("reversed and remanded"))
}

fn looks_like_legal_holding_block(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("we hold that")
        || lower.contains("we further conclude")
        || lower.contains("we conclude that")
        || lower.contains("therefore, we conclude")
        || lower.contains("the court held")
}

fn looks_like_legal_issue_block(text: &str) -> bool {
    let lower = text.trim_start().to_ascii_lowercase();
    lower.starts_with("at issue ")
        || lower.starts_with("the issue ")
        || lower.starts_with("question presented")
}

fn looks_like_legal_syllabus_block(text: &str) -> bool {
    let lower = text.trim_start().to_ascii_lowercase();
    lower.starts_with("appeal from ")
        || lower.contains("reversed and remanded")
            && (lower.contains("appeal") || word_count(text) <= 80)
}

fn normalize_book_chapter_blocks(blocks: Vec<LiquidBlock>) -> Vec<LiquidBlock> {
    if !has_restatement_context(&blocks) {
        return blocks;
    }

    let mut in_subjects_covered = false;
    blocks
        .into_iter()
        .map(|mut block| {
            let normalized = normalize_reference_heading(&block.text);
            if normalized == "subjects covered" {
                in_subjects_covered = true;
            } else if in_subjects_covered && normalized == "appendix" {
                in_subjects_covered = false;
            } else if in_subjects_covered
                && matches!(
                    block.role,
                    LiquidBlockRole::Heading
                        | LiquidBlockRole::Subheading
                        | LiquidBlockRole::Paragraph
                        | LiquidBlockRole::ListItem
                )
                && looks_like_restatement_subject_entry(&block.text)
            {
                block.role = LiquidBlockRole::ListItem;
                block.label = None;
            }
            block
        })
        .collect()
}

fn has_restatement_context(blocks: &[LiquidBlock]) -> bool {
    let text = blocks
        .iter()
        .take(120)
        .map(|block| block.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
        .to_ascii_lowercase();
    text.contains("restatement")
        && (text.contains("american law institute")
            || text.contains("tentative draft")
            || text.contains("subjects covered"))
}

fn looks_like_restatement_subject_entry(text: &str) -> bool {
    let trimmed = text.trim();
    let rest = trimmed
        .trim_start_matches(|ch| matches!(ch, '§' | '?' | '*' | '-'))
        .trim_start();
    starts_with_numbered_heading(rest) && (3..=18).contains(&word_count(rest))
}

fn normalize_policy_report_blocks(blocks: Vec<LiquidBlock>) -> Vec<LiquidBlock> {
    if !has_policy_basic_statistics_context(&blocks) {
        return blocks;
    }

    let mut in_basic_statistics = false;
    blocks
        .into_iter()
        .map(|mut block| {
            let lower = block.text.trim().to_ascii_lowercase();
            if lower.starts_with("basic statistics") {
                in_basic_statistics = true;
            } else if in_basic_statistics && lower.starts_with("executive summary") {
                in_basic_statistics = false;
            } else if in_basic_statistics
                && matches!(
                    block.role,
                    LiquidBlockRole::Heading
                        | LiquidBlockRole::Subheading
                        | LiquidBlockRole::Paragraph
                        | LiquidBlockRole::ListItem
                )
                && looks_like_policy_basic_statistics_table_row(&block.text)
            {
                block.role = LiquidBlockRole::Table;
                block.label = None;
            }
            block
        })
        .collect()
}

fn has_policy_basic_statistics_context(blocks: &[LiquidBlock]) -> bool {
    let text = blocks
        .iter()
        .take(180)
        .map(|block| block.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
        .to_ascii_lowercase();
    text.contains("basic statistics")
        && (text.contains("the land")
            || text.contains("the people")
            || text.contains("the production")
            || text.contains("the government"))
}

fn looks_like_policy_basic_statistics_table_row(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.len() > 120
        || trimmed.ends_with('.')
        || word_count(trimmed) > 14
        || is_policy_basic_statistics_section_heading(trimmed)
    {
        return false;
    }
    trimmed.chars().any(|ch| ch.is_ascii_digit())
        && trimmed.chars().any(|ch| ch.is_ascii_alphabetic())
}

fn is_policy_basic_statistics_section_heading(text: &str) -> bool {
    let letters = text.chars().filter(|ch| ch.is_ascii_alphabetic()).count();
    letters >= 4 && word_count(text) <= 5 && title_case_ratio(text) == 0.0
}

fn normalize_cv_academic_blocks(blocks: Vec<LiquidBlock>) -> Vec<LiquidBlock> {
    let has_faculty_application_context = has_faculty_application_context(&blocks);
    let has_cv_context = blocks.iter().any(|block| {
        let lower = block.text.to_ascii_lowercase();
        lower.contains("curriculum vitae")
            || lower.contains("academic c.v.")
            || lower.contains("academic cv")
            || lower.contains("selected publications")
            || lower.contains("academic appointments")
            || lower.contains("biographical information")
            || lower.contains("academic background")
            || lower.contains("academic experience")
            || lower.contains("faculty application")
            || lower.contains("present position & prior academic employment")
            || lower.contains("prior academic employment")
            || lower.contains("resume / curriculum")
            || lower.contains("courses taught")
            || lower.contains("publications and contributions")
            || lower.contains("professional certifications")
            || lower == "publications"
            || lower == "education"
    }) || has_faculty_application_context;
    if !has_cv_context {
        return blocks;
    }

    let mut in_cv_body = false;
    let normalized = blocks
        .into_iter()
        .map(|mut block| {
            if matches!(
                block.role,
                LiquidBlockRole::Heading | LiquidBlockRole::Subheading
            ) && looks_like_cv_section_heading(&block.text)
            {
                in_cv_body = true;
            } else if in_cv_body
                && matches!(
                    block.role,
                    LiquidBlockRole::Heading | LiquidBlockRole::Subheading
                )
                && looks_like_cv_body_record_heading(&block.text)
            {
                block.role = LiquidBlockRole::ListItem;
                block.label = None;
            }

            if has_faculty_application_context
                && matches!(
                    block.role,
                    LiquidBlockRole::Heading
                        | LiquidBlockRole::Subheading
                        | LiquidBlockRole::Paragraph
                        | LiquidBlockRole::Issue
                        | LiquidBlockRole::KeyClause
                        | LiquidBlockRole::ListItem
                        | LiquidBlockRole::Marginalia
                )
                && looks_like_faculty_application_form_row(&block.text)
            {
                block.role = LiquidBlockRole::Table;
                block.label = None;
            }

            if block.role == LiquidBlockRole::Footnote
                && looks_like_cv_publication_or_record(&block.text)
            {
                block.role = LiquidBlockRole::ListItem;
                block.label = None;
            }
            block
        })
        .collect::<Vec<_>>();

    let normalized = remove_cv_record_section_breaks(normalized);
    remove_section_breaks_before_role(normalized, LiquidBlockRole::Table)
}

fn has_faculty_application_context(blocks: &[LiquidBlock]) -> bool {
    let text = blocks
        .iter()
        .take(80)
        .map(|block| block.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
        .to_ascii_lowercase();
    text.contains("faculty application")
        || text.contains("required documents")
            && (text.contains("resume / curriculum") || text.contains("cover letter"))
        || text.contains("application:")
            && text.contains("posting number")
            && text.contains("submitted:")
}

fn looks_like_faculty_application_form_row(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.len() > 220 {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "personal information"
            | "supplemental questions"
            | "certify"
            | "required fields are indicated with an asterisk (*)."
    ) {
        return false;
    }
    if matches!(
        lower.as_str(),
        "required documents"
            | "optional documents"
            | "kind name conversion status"
            | "resume / curriculum"
            | "vitae"
            | "pdf complete"
            | "(cdt)"
            | "teaching philosophy - -"
    ) {
        return true;
    }
    if lower.starts_with("cover letter ") || lower.starts_with("resume / curriculum vitae ") {
        return true;
    }
    if lower.contains(" pdf complete") && word_count(trimmed) <= 14 {
        return true;
    }
    if lower.starts_with("submitted on ") && lower.contains(" by ") {
        return true;
    }
    if lower == "yes" || lower == "no" {
        return true;
    }
    if lower.contains('?') && word_count(trimmed) <= 22 {
        return true;
    }

    let Some((label, _value)) = trimmed.split_once(':') else {
        return faculty_application_label_without_colon(&lower);
    };
    let label = label.trim().to_ascii_lowercase();
    matches!(
        label.as_str(),
        "posting number"
            | "posting"
            | "form"
            | "submitted"
            | "salutation"
            | "first name"
            | "middle or maiden name"
            | "last name"
            | "suffix"
            | "address"
            | "city"
            | "state"
            | "zip code"
            | "secondary phone"
            | "work phone"
            | "international contact information"
            | "email address"
    ) || (word_count(&label) <= 5 && lower.contains("application") && !lower.ends_with('.'))
}

fn faculty_application_label_without_colon(lower: &str) -> bool {
    matches!(
        lower,
        "address 2"
            | "country of residence"
            | "primary contact number"
            | "kind"
            | "name"
            | "conversion status"
    ) || lower.starts_with("country of residence ")
        || lower.starts_with("primary contact number ")
}

fn remove_cv_record_section_breaks(blocks: Vec<LiquidBlock>) -> Vec<LiquidBlock> {
    let mut output = Vec::with_capacity(blocks.len());
    for index in 0..blocks.len() {
        let block = &blocks[index];
        if block.role == LiquidBlockRole::SectionBreak {
            let previous_role = output.last().map(|previous: &LiquidBlock| previous.role);
            let next = blocks.get(index + 1);
            if next.is_some_and(|next| {
                next.role == LiquidBlockRole::ListItem
                    && looks_like_cv_publication_or_record(&next.text)
                    && matches!(
                        previous_role,
                        Some(
                            LiquidBlockRole::Heading
                                | LiquidBlockRole::Subheading
                                | LiquidBlockRole::ListItem
                                | LiquidBlockRole::Paragraph
                        )
                    )
            }) {
                continue;
            }
        }
        output.push(block.clone());
    }
    output
}

fn looks_like_cv_section_heading(text: &str) -> bool {
    let lower = text.trim().trim_matches(':').to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "education"
            | "employment"
            | "experience"
            | "academic appointments"
            | "academic employment"
            | "present position & prior academic employment"
            | "prior academic employment"
            | "professional appointments"
            | "publications"
            | "selected publications"
            | "articles"
            | "articles and essays"
            | "selected articles"
            | "books"
            | "book chapters"
            | "works in progress"
            | "working papers"
            | "other writing"
            | "teaching"
            | "courses taught"
            | "academic background"
            | "academic experience"
            | "biographical information"
            | "publications and contributions"
            | "professional certifications"
            | "professional certifications exams and licenses"
            | "certifications"
            | "licenses"
            | "service"
            | "professional service"
            | "university service"
            | "bar admissions"
            | "awards"
            | "honors"
            | "grants"
            | "fellowships"
            | "presentations"
            | "selected presentations"
            | "conferences"
            | "workshops"
            | "invited talks"
            | "media"
            | "media appearances"
            | "selected media appearances"
            | "consulting"
            | "references"
    )
}

fn looks_like_cv_body_record_heading(text: &str) -> bool {
    let trimmed = text.trim();
    if looks_like_cv_section_heading(trimmed) {
        return false;
    }
    looks_like_cv_publication_or_record(trimmed) || word_count(trimmed) >= 2 && trimmed.len() >= 12
}

fn looks_like_cv_publication_or_record(text: &str) -> bool {
    let (_, body) = split_note_marker(text);
    let lower = body.to_ascii_lowercase();
    if lower.len() < 8 {
        return false;
    }
    lower.contains("law review")
        || lower.contains("l. rev")
        || lower.contains("journal")
        || lower.contains("university press")
        || lower.contains("working paper")
        || lower.contains("forthcoming")
        || lower.contains("ssrn")
        || lower.contains("presented at")
        || lower.contains("invited talk")
        || lower.contains("teaching:")
        || lower.contains("service:")
        || lower.contains("award")
        || lower.contains("grant")
        || lower.contains("conference")
        || lower.contains("workshop")
        || lower.contains("university")
        || lower.contains("school of law")
        || contains_reference_year(body)
}

fn normalize_course_blocks(blocks: Vec<LiquidBlock>) -> Vec<LiquidBlock> {
    let has_evaluation_context = blocks
        .iter()
        .any(|block| looks_like_course_evaluation_context(&block.text));
    let has_syllabus_context = has_course_syllabus_context(&blocks);

    let normalized = blocks
        .into_iter()
        .map(|mut block| {
            if has_evaluation_context && looks_like_course_evaluation_question(&block.text) {
                block.role = LiquidBlockRole::ListItem;
                block.label = None;
            } else if has_evaluation_context
                && matches!(
                    block.role,
                    LiquidBlockRole::Heading
                        | LiquidBlockRole::Subheading
                        | LiquidBlockRole::Paragraph
                        | LiquidBlockRole::ListItem
                        | LiquidBlockRole::Issue
                        | LiquidBlockRole::Definition
                        | LiquidBlockRole::Marginalia
                )
                && looks_like_course_evaluation_table_line(&block.text)
            {
                block.role = LiquidBlockRole::Table;
                block.label = None;
            } else if matches!(
                block.role,
                LiquidBlockRole::Paragraph
                    | LiquidBlockRole::Explainer
                    | LiquidBlockRole::Takeaway
                    | LiquidBlockRole::Issue
            ) && looks_like_course_question_or_prompt(&block.text)
            {
                block.role = LiquidBlockRole::ListItem;
                block.label = None;
            } else if has_syllabus_context
                && matches!(
                    block.role,
                    LiquidBlockRole::Heading
                        | LiquidBlockRole::Subheading
                        | LiquidBlockRole::Paragraph
                        | LiquidBlockRole::ListItem
                        | LiquidBlockRole::Marginalia
                )
                && looks_like_syllabus_assignment_table_row(&block.text)
            {
                block.role = LiquidBlockRole::Table;
                block.label = None;
            }
            block
        })
        .collect::<Vec<_>>();

    if has_evaluation_context || has_syllabus_context {
        remove_course_table_section_breaks(normalized)
    } else {
        normalized
    }
}

fn has_course_syllabus_context(blocks: &[LiquidBlock]) -> bool {
    let text = blocks
        .iter()
        .take(100)
        .map(|block| block.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
        .to_ascii_lowercase();
    text.contains("syllabus")
        && (text.contains("course materials")
            || text.contains("assignments")
            || text.contains("problem set")
            || text.contains("casebook"))
}

fn looks_like_syllabus_assignment_table_row(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.len() > 180 {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("course materials:") || lower == "assignments" {
        return false;
    }
    let without_bullet = lower
        .trim_start_matches(|ch: char| matches!(ch, '•' | '-' | '*' | ' '))
        .trim();
    without_bullet.starts_with("problem set ")
        || without_bullet.contains(" problem set ")
        || without_bullet.contains("lopucki")
        || without_bullet.contains("statutory supplement")
        || without_bullet.contains("secured credit:")
        || (starts_with_numbered_material_marker(trimmed)
            && (lower.contains("casebook")
                || lower.contains("supplement")
                || lower.contains("secured credit")
                || lower.contains("warren")))
}

fn starts_with_numbered_material_marker(text: &str) -> bool {
    let trimmed = text.trim_start();
    let Some(rest) = trimmed.strip_prefix('(') else {
        return false;
    };
    let Some((digits, after)) = rest.split_once(')') else {
        return false;
    };
    (1..=3).contains(&digits.len())
        && digits.chars().all(|ch| ch.is_ascii_digit())
        && after.starts_with(' ')
}

fn remove_course_table_section_breaks(blocks: Vec<LiquidBlock>) -> Vec<LiquidBlock> {
    let mut output = Vec::with_capacity(blocks.len());
    for index in 0..blocks.len() {
        let block = &blocks[index];
        if block.role == LiquidBlockRole::SectionBreak
            && blocks
                .get(index + 1)
                .is_some_and(|next| next.role == LiquidBlockRole::Table)
        {
            continue;
        }
        output.push(block.clone());
    }
    output
}

fn looks_like_course_evaluation_context(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("survey comparisons")
        || lower.contains("responses / expected")
        || lower.contains("overall mean")
        || lower.contains("responsible faculty")
        || lower.contains("response option")
        || lower.contains("response rate")
        || lower.contains("percent responses")
        || lower.contains("strongly agree")
        || lower.contains("strongly disagree")
        || lower.contains("pct rnk")
        || lower.contains("vms imr")
        || lower.contains("web link")
        || lower.contains("time spent:")
        || lower.contains("written comments")
        || lower.contains("how often did you attend class")
        || lower.contains("instructor's mastery")
        || lower.contains("overall teaching effectiveness")
}

fn looks_like_course_evaluation_question(text: &str) -> bool {
    let trimmed = text.trim();
    let mut chars = trimmed.chars();
    (matches!(chars.next(), Some('Q' | 'q'))
        && chars.next().is_some_and(|ch| ch.is_ascii_digit())
        && (trimmed.contains('?') || word_count(trimmed) >= 6))
        || trimmed.split_once('-').is_some_and(|(prefix, rest)| {
            prefix.trim().chars().all(|ch| ch.is_ascii_digit())
                && (rest.contains('?') || word_count(rest) >= 6)
        })
}

fn looks_like_course_evaluation_table_line(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() || looks_like_course_evaluation_question(trimmed) {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    if looks_like_short_course_response_row(trimmed)
        || looks_like_course_evaluation_answer_value(trimmed)
        || looks_like_course_evaluation_metadata_row(trimmed)
    {
        return true;
    }
    if lower.contains("survey comparisons")
        || lower.starts_with("responses:")
        || lower.starts_with("pct rnk")
        || lower.contains("responses / expected")
        || lower.contains("overall mean")
        || lower.contains("responsible faculty")
        || lower.contains("response option")
        || lower.contains("response rate")
        || lower.contains("percent responses")
        || lower.contains("strongly agree")
        || lower.contains("strongly disagree")
        || lower.contains("vms imr")
        || lower.contains("dev n mean")
    {
        return true;
    }
    if trimmed.contains(':')
        && lower.split(':').next().is_some_and(|key| {
            matches!(
                key.trim(),
                "course" | "department" | "responses / expected" | "overall mean"
            )
        })
    {
        return true;
    }
    let allowed_numeric = trimmed.chars().all(|ch| {
        ch.is_ascii_digit() || matches!(ch, '.' | ',' | '/' | '%' | '(' | ')' | '-' | ' ' | '\t')
    });
    if allowed_numeric && trimmed.chars().any(|ch| ch.is_ascii_digit()) && trimmed.len() <= 40 {
        return true;
    }
    matches!(
        lower.as_str(),
        "responses"
            | "course"
            | "law"
            | "all"
            | "rnk"
            | "mean"
            | "question"
            | "school"
            | "weight"
            | "frequency"
            | "percent"
            | "means"
            | "std"
            | "median"
            | "agree"
            | "disagree"
            | "not applicable"
    )
}

fn looks_like_short_course_response_row(text: &str) -> bool {
    let Some(remainder) = course_question_code_remainder(text) else {
        return false;
    };
    if remainder.is_empty() {
        return true;
    }
    word_count(remainder) <= 4 && looks_like_course_evaluation_answer_value(remainder)
}

fn course_question_code_remainder(text: &str) -> Option<&str> {
    let trimmed = text.trim();
    let mut chars = trimmed.char_indices();
    let (_, first) = chars.next()?;
    if !matches!(first, 'Q' | 'q') {
        return None;
    }

    let mut end = first.len_utf8();
    let mut saw_digit = false;
    for (index, ch) in chars {
        if ch.is_ascii_digit() {
            saw_digit = true;
            end = index + ch.len_utf8();
        } else {
            break;
        }
    }
    if !saw_digit {
        return None;
    }

    Some(
        trimmed[end..].trim_start_matches(|ch: char| {
            ch.is_whitespace() || matches!(ch, '.' | ':' | ')' | '-')
        }),
    )
}

fn looks_like_course_evaluation_answer_value(text: &str) -> bool {
    let trimmed = text.trim().trim_matches(':');
    if trimmed.is_empty() || trimmed.len() > 80 {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "excellent"
            | "very good"
            | "good"
            | "satisfactory"
            | "fair"
            | "poor"
            | "very poor"
            | "always"
            | "usually"
            | "sometimes"
            | "rarely"
            | "never"
            | "strongly agree"
            | "agree"
            | "neither agree nor disagree"
            | "disagree"
            | "strongly disagree"
            | "not applicable"
            | "not observed"
            | "no answer"
    ) {
        return true;
    }
    lower.contains("hours")
        && lower.chars().any(|ch| ch.is_ascii_digit())
        && word_count(trimmed) <= 5
}

fn looks_like_course_evaluation_metadata_row(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.len() > 90 {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    lower == "complete"
        || lower.starts_with("collector:")
        || lower.starts_with("started:")
        || lower.starts_with("last modified:")
        || lower.starts_with("time spent:")
        || lower.starts_with("ip address:")
        || lower.contains("web link")
            && trimmed
                .chars()
                .next()
                .is_some_and(|ch| ch == '#' || ch.is_ascii_digit())
}

fn looks_like_course_question_or_prompt(text: &str) -> bool {
    let trimmed = text.trim();
    let lower = trimmed.to_ascii_lowercase();
    lower.starts_with("question ")
        || lower.starts_with("problem ")
        || lower.starts_with("assignment ")
        || lower.starts_with("hypothetical ")
        || lower.contains("anonymous number")
        || lower.contains("answer the following")
        || lower.contains("questions below")
        || trimmed.ends_with('?') && word_count(trimmed) >= 4
        || starts_with_numbered_heading(trimmed) && trimmed.ends_with('?')
}

fn normalize_general_structured_blocks(blocks: Vec<LiquidBlock>) -> Vec<LiquidBlock> {
    let blocks = normalize_codebook_blocks(blocks);
    let blocks = normalize_property_deal_sheet_blocks(blocks);
    let blocks = normalize_event_logistics_blocks(blocks);
    let blocks = normalize_grant_application_blocks(blocks);
    let blocks = normalize_expense_report_blocks(blocks);
    normalize_reference_contact_blocks(blocks)
}

fn normalize_grant_application_blocks(blocks: Vec<LiquidBlock>) -> Vec<LiquidBlock> {
    if !has_grant_application_context(&blocks) {
        return blocks;
    }

    let normalized = blocks
        .into_iter()
        .map(|mut block| {
            if matches!(
                block.role,
                LiquidBlockRole::Heading
                    | LiquidBlockRole::Subheading
                    | LiquidBlockRole::Paragraph
                    | LiquidBlockRole::ListItem
                    | LiquidBlockRole::Marginalia
                    | LiquidBlockRole::KeyClause
            ) && looks_like_grant_application_table_row(&block.text)
            {
                block.role = LiquidBlockRole::Table;
                block.label = None;
            }
            block
        })
        .collect::<Vec<_>>();

    remove_section_breaks_before_role(normalized, LiquidBlockRole::Table)
}

fn has_grant_application_context(blocks: &[LiquidBlock]) -> bool {
    let text = blocks
        .iter()
        .take(100)
        .map(|block| block.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
        .to_ascii_lowercase();
    (text.contains("research grant application") || text.contains("personal research grants"))
        && (text.contains("application no.") || text.contains("general application information"))
}

fn looks_like_grant_application_table_row(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.len() > 180 {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    if matches!(
        normalize_title_key(trimmed).as_str(),
        "role name academic rank department institute"
            | "research title"
            | "keywords"
            | "requested budget in nis"
            | "no of years average annual budget"
    ) {
        return true;
    }
    lower.starts_with("pi.")
        || lower.starts_with("pi1 name:")
        || lower.starts_with("application no.")
        || lower.contains(" budget")
            && lower.chars().any(|ch| ch.is_ascii_digit())
            && word_count(trimmed) <= 16
}

fn normalize_expense_report_blocks(blocks: Vec<LiquidBlock>) -> Vec<LiquidBlock> {
    if !has_expense_report_context(&blocks) {
        return blocks;
    }

    let normalized = blocks
        .into_iter()
        .map(|mut block| {
            if matches!(
                block.role,
                LiquidBlockRole::Heading
                    | LiquidBlockRole::Subheading
                    | LiquidBlockRole::Paragraph
                    | LiquidBlockRole::ListItem
                    | LiquidBlockRole::Marginalia
                    | LiquidBlockRole::KeyClause
            ) && looks_like_expense_report_table_row(&block.text)
            {
                block.role = LiquidBlockRole::Table;
                block.label = None;
            }
            block
        })
        .collect::<Vec<_>>();

    remove_section_breaks_before_role(normalized, LiquidBlockRole::Table)
}

fn has_expense_report_context(blocks: &[LiquidBlock]) -> bool {
    let text = blocks
        .iter()
        .take(140)
        .map(|block| block.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
        .to_ascii_lowercase();
    text.contains("expense report")
        && (text.contains("expense register")
            || text.contains("receipt evidence")
            || text.contains("spend by category")
            || text.contains("project overview"))
}

fn looks_like_expense_report_table_row(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.len() > 260 {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    if matches!(
        normalize_title_key(trimmed).as_str(),
        "field value"
            | "category total"
            | "date vendor description category amount"
            | "description qty unit amount"
            | "line items"
            | "receipt details"
    ) {
        return true;
    }
    if lower.starts_with("expense #") {
        return true;
    }
    if starts_with_iso_date(trimmed) {
        return true;
    }
    if lower.contains(" usd ") && lower.chars().any(|ch| ch.is_ascii_digit()) {
        return true;
    }
    if lower.contains('$') && lower.chars().any(|ch| ch.is_ascii_digit()) {
        return true;
    }
    let normalized = normalize_title_key(trimmed);
    [
        "project",
        "location",
        "dates",
        "status",
        "total expenses",
        "total amount",
        "notes",
        "date",
        "vendor",
        "invoice",
        "description",
        "amount",
        "category",
        "confirmation",
        "transaction date",
        "vendor type",
        "destination city",
        "service dates",
        "route",
        "itinerary",
    ]
    .iter()
    .any(|label| {
        normalized == *label
            || normalized
                .strip_prefix(label)
                .is_some_and(|rest| !rest.trim().is_empty() && word_count(rest) <= 12)
    })
}

fn starts_with_iso_date(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() >= 10
        && bytes[0..4].iter().all(u8::is_ascii_digit)
        && bytes[4] == b'-'
        && bytes[5..7].iter().all(u8::is_ascii_digit)
        && bytes[7] == b'-'
        && bytes[8..10].iter().all(u8::is_ascii_digit)
}

fn normalize_event_logistics_blocks(blocks: Vec<LiquidBlock>) -> Vec<LiquidBlock> {
    if !has_event_logistics_context(&blocks) {
        return blocks;
    }

    blocks
        .into_iter()
        .map(|mut block| {
            if matches!(
                block.role,
                LiquidBlockRole::Heading
                    | LiquidBlockRole::Subheading
                    | LiquidBlockRole::Paragraph
                    | LiquidBlockRole::ListItem
                    | LiquidBlockRole::Marginalia
            ) && looks_like_event_logistics_table_row(&block.text)
            {
                block.role = LiquidBlockRole::Table;
                block.label = None;
            }
            block
        })
        .collect()
}

fn has_event_logistics_context(blocks: &[LiquidBlock]) -> bool {
    let text = blocks
        .iter()
        .take(80)
        .map(|block| block.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
        .to_ascii_lowercase();
    [
        "roundtable",
        "conference",
        "symposium",
        "workshop",
        "seminar",
        "summit",
    ]
    .iter()
    .any(|marker| text.contains(marker))
        && [
            "logistics",
            "accommodation options",
            "booking deadline",
            "schedule highlights",
            "contact information for logistical issues",
        ]
        .iter()
        .any(|marker| text.contains(marker))
}

fn looks_like_event_logistics_table_row(text: &str) -> bool {
    let trimmed = text.trim();
    let Some((key, value)) = trimmed.split_once(':') else {
        return false;
    };
    if value.trim().is_empty() || word_count(key) > 5 {
        return false;
    }
    matches!(
        normalize_title_key(key).as_str(),
        "assistance"
            | "block name"
            | "booking deadline"
            | "booking link"
            | "contact"
            | "dates"
            | "directions"
            | "group name"
            | "hotel"
            | "phone booking"
    )
}

fn normalize_reference_contact_blocks(blocks: Vec<LiquidBlock>) -> Vec<LiquidBlock> {
    if !has_reference_contact_context(&blocks) {
        return blocks;
    }

    blocks
        .into_iter()
        .map(|mut block| {
            if matches!(
                block.role,
                LiquidBlockRole::AuthorInfo
                    | LiquidBlockRole::Heading
                    | LiquidBlockRole::Subheading
                    | LiquidBlockRole::Paragraph
                    | LiquidBlockRole::ListItem
            ) && looks_like_numbered_reference_contact_entry(&block.text)
            {
                block.role = LiquidBlockRole::ListItem;
                block.label = None;
            }
            block
        })
        .collect()
}

fn has_reference_contact_context(blocks: &[LiquidBlock]) -> bool {
    let text = blocks
        .iter()
        .take(80)
        .map(|block| block.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
        .to_ascii_lowercase();
    let numbered_contacts = blocks
        .iter()
        .take(80)
        .filter(|block| looks_like_numbered_reference_contact_entry(&block.text))
        .count();
    text.contains("references") && text.matches('@').count() >= 3 && numbered_contacts >= 2
}

fn looks_like_numbered_reference_contact_entry(text: &str) -> bool {
    let trimmed = text.trim();
    let Some((marker, rest)) = trimmed.split_once('.') else {
        return false;
    };
    let marker = marker.trim();
    (1..=2).contains(&marker.len())
        && marker.chars().all(|ch| ch.is_ascii_digit())
        && word_count(rest) >= 3
        && (rest.contains(',') || rest.to_ascii_lowercase().contains("professor"))
}

fn normalize_codebook_blocks(blocks: Vec<LiquidBlock>) -> Vec<LiquidBlock> {
    if !has_codebook_context(&blocks) {
        return blocks;
    }

    let normalized = blocks
        .into_iter()
        .map(|mut block| {
            if looks_like_codebook_section_heading(&block.text) {
                block.role = LiquidBlockRole::Heading;
                block.label = None;
            } else if matches!(
                block.role,
                LiquidBlockRole::Heading
                    | LiquidBlockRole::Subheading
                    | LiquidBlockRole::Paragraph
                    | LiquidBlockRole::Issue
                    | LiquidBlockRole::KeyClause
                    | LiquidBlockRole::Header
                    | LiquidBlockRole::ListItem
                    | LiquidBlockRole::Marginalia
            ) && looks_like_codebook_table_text(&block.text)
            {
                block.role = LiquidBlockRole::Table;
                block.label = None;
            } else if matches!(
                block.role,
                LiquidBlockRole::Heading
                    | LiquidBlockRole::Subheading
                    | LiquidBlockRole::Paragraph
                    | LiquidBlockRole::Header
            ) && looks_like_codebook_variable_heading(&block.text)
            {
                block.role = LiquidBlockRole::Subheading;
                block.label = None;
            }
            block
        })
        .collect::<Vec<_>>();

    remove_section_breaks_before_role(normalized, LiquidBlockRole::Table)
}

fn normalize_property_deal_sheet_blocks(blocks: Vec<LiquidBlock>) -> Vec<LiquidBlock> {
    if !has_property_deal_sheet_context(&blocks) {
        return blocks;
    }

    let normalized = blocks
        .into_iter()
        .map(|mut block| {
            if matches!(
                block.role,
                LiquidBlockRole::Heading
                    | LiquidBlockRole::Subheading
                    | LiquidBlockRole::Paragraph
                    | LiquidBlockRole::Header
                    | LiquidBlockRole::KeyClause
                    | LiquidBlockRole::ListItem
                    | LiquidBlockRole::Marginalia
            ) && looks_like_property_deal_field(&block.text)
            {
                block.role = LiquidBlockRole::Table;
                block.label = None;
            }
            block
        })
        .collect::<Vec<_>>();

    remove_section_breaks_before_role(normalized, LiquidBlockRole::Table)
}

fn has_property_deal_sheet_context(blocks: &[LiquidBlock]) -> bool {
    let mut field_count = 0usize;
    for block in blocks {
        let lower = block.text.to_ascii_lowercase();
        if lower.contains("off market deals")
            || lower.contains("cash or hard money")
            || lower.contains("seller financing")
        {
            return true;
        }
        if looks_like_property_deal_field(&block.text) {
            field_count += 1;
        }
    }
    field_count >= 8
}

fn looks_like_property_deal_field(text: &str) -> bool {
    let stripped = strip_leading_non_field_chars(text);
    let lower = stripped.to_ascii_lowercase();
    let Some((label, value)) = lower.split_once(':') else {
        return matches!(
            lower.trim(),
            "no hoa" | "hoa none" | "cash only" | "tenant occupied" | "vacant at closing"
        );
    };
    if value.trim().is_empty() {
        return false;
    }
    matches!(
        label.trim(),
        "price"
            | "arv"
            | "beds"
            | "baths"
            | "sqft"
            | "year built"
            | "layout"
            | "lot size"
            | "occupancy"
            | "construction"
            | "title"
            | "folio"
            | "condition"
            | "updates"
            | "notes"
            | "home"
            | "pool"
            | "laundry"
            | "hoa"
            | "location"
            | "subdivision"
            | "estimated repairs"
    )
}

fn strip_leading_non_field_chars(text: &str) -> &str {
    let trimmed = text.trim();
    let start = trimmed
        .char_indices()
        .find_map(|(index, ch)| ch.is_ascii_alphabetic().then_some(index))
        .unwrap_or(0);
    &trimmed[start..]
}

fn has_codebook_context(blocks: &[LiquidBlock]) -> bool {
    let mut field_blocks = 0usize;
    for block in blocks {
        let lower = block.text.to_ascii_lowercase();
        if lower.contains("codebook")
            || lower.contains("data dictionary")
            || lower.contains("list of variables")
            || lower.contains("variable names")
            || lower.contains("each variable to be coded")
            || lower.contains("measurement level:")
            || lower.contains("print format:")
            || lower.contains("write format:")
        {
            return true;
        }
        if looks_like_codebook_table_text(&block.text) {
            field_blocks += 1;
        }
    }
    field_blocks >= 5
}

fn looks_like_codebook_section_heading(text: &str) -> bool {
    let lower = text.trim().trim_matches(':').to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "definitions"
            | "colors"
            | "format"
            | "old variables"
            | "new variables"
            | "value label"
            | "value labels"
            | "list of variables on the working file"
            | "name (position) label"
    )
}

fn looks_like_codebook_table_text(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    if starts_with_codebook_field_label(&lower)
        || lower.contains(" description:")
        || lower.contains(" type:")
        || lower.contains(" format:")
        || lower.contains(" notes:")
        || lower.contains(" measurement level:")
        || lower.contains(" print format:")
        || lower.contains(" write format:")
    {
        return true;
    }
    looks_like_codebook_value_row(trimmed)
}

fn starts_with_codebook_field_label(lower: &str) -> bool {
    [
        "description:",
        "type:",
        "format:",
        "notes:",
        "example:",
        "measurement level:",
        "column width:",
        "print format:",
        "write format:",
        "name (position) label",
    ]
    .iter()
    .any(|prefix| lower.starts_with(prefix))
}

fn looks_like_codebook_value_row(text: &str) -> bool {
    let mut parts = text.split_whitespace();
    let Some(code) = parts.next() else {
        return false;
    };
    let rest = parts.collect::<Vec<_>>().join(" ");
    !rest.is_empty()
        && code.len() <= 4
        && code.chars().all(|ch| ch.is_ascii_digit())
        && word_count(&rest) <= 8
}

fn looks_like_codebook_variable_heading(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.len() < 2 || trimmed.len() > 90 || looks_like_codebook_section_heading(trimmed) {
        return false;
    }
    let mut parts = trimmed.split_whitespace();
    let Some(name) = parts.next() else {
        return false;
    };
    if !name.chars().any(|ch| ch.is_ascii_alphabetic())
        || !name
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
    {
        return false;
    }
    let rest = parts.collect::<Vec<_>>().join(" ");
    rest.is_empty()
        || rest.starts_with('(')
        || rest.starts_with("Description:")
        || rest.starts_with("description:")
}

fn remove_section_breaks_before_role(
    blocks: Vec<LiquidBlock>,
    role: LiquidBlockRole,
) -> Vec<LiquidBlock> {
    let mut output = Vec::with_capacity(blocks.len());
    for index in 0..blocks.len() {
        let block = &blocks[index];
        if block.role == LiquidBlockRole::SectionBreak
            && blocks.get(index + 1).is_some_and(|next| next.role == role)
        {
            continue;
        }
        output.push(block.clone());
    }
    output
}

fn normalize_contract_blocks(blocks: Vec<LiquidBlock>) -> Vec<LiquidBlock> {
    blocks
        .into_iter()
        .map(|mut block| {
            if matches!(
                block.role,
                LiquidBlockRole::Paragraph
                    | LiquidBlockRole::ListItem
                    | LiquidBlockRole::Marginalia
            ) && looks_like_contract_clause_text(&block.text)
            {
                block.role = LiquidBlockRole::Clause;
                block.label = None;
            }
            block
        })
        .collect()
}

fn looks_like_contract_clause_text(text: &str) -> bool {
    let trimmed = text.trim();
    let lower = trimmed.to_ascii_lowercase();
    if word_count(trimmed) < 6 {
        return false;
    }
    lower.starts_with("section ")
        || lower.starts_with("article ")
        || lower.starts_with("whereas")
        || lower.contains(" party shall ")
        || lower.contains(" parties shall ")
        || lower.contains(" agrees to ")
        || lower.contains(" governing law")
        || lower.contains("effective date")
        || looks_like_clause(trimmed)
}

fn normalize_receipt_financial_blocks(blocks: Vec<LiquidBlock>) -> Vec<LiquidBlock> {
    blocks
        .into_iter()
        .map(|mut block| {
            if matches!(
                block.role,
                LiquidBlockRole::Heading
                    | LiquidBlockRole::Subheading
                    | LiquidBlockRole::Paragraph
                    | LiquidBlockRole::ListItem
                    | LiquidBlockRole::Marginalia
                    | LiquidBlockRole::KeyClause
            ) && looks_like_financial_field_or_table(&block.text)
            {
                block.role = if block
                    .text
                    .lines()
                    .filter(|line| looks_like_financial_field(line))
                    .count()
                    >= 3
                {
                    LiquidBlockRole::Table
                } else {
                    LiquidBlockRole::Marginalia
                };
                block.label = None;
            }
            block
        })
        .collect()
}

fn looks_like_financial_field_or_table(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    looks_like_financial_field(text)
        || lower.contains("payment method")
        || lower.contains("amount due")
        || lower.contains("subtotal")
        || lower.contains("total $")
        || lower.contains("invoice #")
        || lower.contains("receipt #")
}

fn looks_like_financial_field(text: &str) -> bool {
    let trimmed = text.trim();
    let lower = trimmed.to_ascii_lowercase();
    (trimmed.contains(':')
        && lower.split(':').next().is_some_and(|key| {
            matches!(
                key.trim(),
                "date"
                    | "total"
                    | "subtotal"
                    | "tax"
                    | "amount"
                    | "balance"
                    | "invoice"
                    | "receipt"
                    | "payment"
            )
        }))
        || lower.contains('$') && lower.chars().any(|ch| ch.is_ascii_digit())
}

#[cfg(test)]
mod profile_policy_tests {
    use super::*;

    fn block(role: LiquidBlockRole, text: &str) -> LiquidBlock {
        LiquidBlock {
            role,
            text: text.to_owned(),
            label: None,
        }
    }

    fn marginalia_containing<'a>(blocks: &'a [LiquidBlock], text: &str) -> &'a LiquidBlock {
        blocks
            .iter()
            .find(|block| {
                block.role == LiquidBlockRole::Marginalia
                    && block.label.as_deref() == Some("Footnote")
                    && block.text.contains(text)
            })
            .unwrap_or_else(|| panic!("missing marginalia text: {text}"))
    }

    #[test]
    fn profile_policy_keeps_cv_publications_in_flow() {
        let blocks = run_profile_specific_normalization(
            vec![
                block(LiquidBlockRole::Title, "Curriculum Vitae"),
                block(LiquidBlockRole::Heading, "Publications"),
                block(
                    LiquidBlockRole::Footnote,
                    "1. Contracting Around Privacy, Journal of Law and Technology (2024).",
                ),
            ],
            DocumentProfileKind::CvOrAcademicPacket,
        );

        assert_eq!(blocks[2].role, LiquidBlockRole::ListItem);
    }

    #[test]
    fn profile_policy_keeps_cv_service_records_in_flow() {
        let blocks = run_profile_specific_normalization(
            vec![
                block(LiquidBlockRole::Title, "Curriculum Vitae"),
                block(LiquidBlockRole::Heading, "Professional Service"),
                block(
                    LiquidBlockRole::Footnote,
                    "2. Workshop on contracts, University of Alabama School of Law, 2024.",
                ),
            ],
            DocumentProfileKind::CvOrAcademicPacket,
        );

        assert_eq!(blocks[2].role, LiquidBlockRole::ListItem);
    }

    #[test]
    fn profile_policy_surfaces_law_review_footnotes_as_marginalia() {
        let blocks = run_profile_specific_normalization(
            vec![
                block(LiquidBlockRole::Title, "Article Example"),
                block(
                    LiquidBlockRole::Paragraph,
                    "Main text remains in the reading flow.",
                ),
                block(
                    LiquidBlockRole::Footnote,
                    "42 See Example v. State, 123 U.S. 456 (2026).",
                ),
            ],
            DocumentProfileKind::LawReviewArticle,
        );

        assert_eq!(blocks[1].role, LiquidBlockRole::Paragraph);
        assert_eq!(blocks[2].role, LiquidBlockRole::Marginalia);
        assert_eq!(blocks[2].label.as_deref(), Some("Footnote"));
    }

    #[test]
    fn profile_policy_merges_visual_lines_from_single_law_review_note() {
        let blocks = run_profile_specific_normalization(
            vec![
                block(LiquidBlockRole::Title, "Article Example"),
                block(
                    LiquidBlockRole::Paragraph,
                    "Main text remains in the reading flow.",
                ),
                block(
                    LiquidBlockRole::Marginalia,
                    "44 That is, the website usually does not attempt to bring the contract terms to the user's attention.",
                ),
                block(
                    LiquidBlockRole::Marginalia,
                    "\"Wraps,\" LITE DEPALMA GREENBERG LAW BLOG (Dec. 1, 2016), http://www.litedepalma.com/",
                ),
                block(
                    LiquidBlockRole::Marginalia,
                    "internet-agreements-to-arbitrate-know-the-four-wraps [https://perma.cc/SZR7-V7QU]",
                ),
                block(
                    LiquidBlockRole::Paragraph,
                    "The next main-text paragraph remains in the reading flow.",
                ),
            ],
            DocumentProfileKind::LawReviewArticle,
        );

        let marginalia = blocks
            .iter()
            .filter(|block| block.role == LiquidBlockRole::Marginalia)
            .collect::<Vec<_>>();
        assert_eq!(marginalia.len(), 1);
        assert_eq!(marginalia[0].label.as_deref(), Some("Footnote"));
        assert!(marginalia[0].text.starts_with("44 That is"));
        assert!(marginalia[0].text.contains("\"Wraps,\""));
        assert!(
            marginalia[0]
                .text
                .contains("internet-agreements-to-arbitrate")
        );
    }

    #[test]
    fn profile_policy_merges_repeated_marker_lines_from_single_law_review_note() {
        let blocks = run_profile_specific_normalization(
            vec![
                block(LiquidBlockRole::Title, "Article Example"),
                block(
                    LiquidBlockRole::Paragraph,
                    "Main text remains in the reading flow.",
                ),
                block(
                    LiquidBlockRole::Marginalia,
                    "44 That is, the website usually does not attempt to bring the contract terms to the user's attention.",
                ),
                block(
                    LiquidBlockRole::Paragraph,
                    "44 \"Wraps,\" LITE DEPALMA GREENBERG LAW BLOG (Dec. 1, 2016), http://www.litedepalma.com/",
                ),
                block(
                    LiquidBlockRole::Table,
                    "44 internet-agreements-to-arbitrate-know-the-four-wraps [https://perma.cc/SZR7-V7QU]",
                ),
                block(
                    LiquidBlockRole::Paragraph,
                    "The next main-text paragraph remains in the reading flow.",
                ),
            ],
            DocumentProfileKind::LawReviewArticle,
        );

        let marginalia = blocks
            .iter()
            .filter(|block| block.role == LiquidBlockRole::Marginalia)
            .collect::<Vec<_>>();
        assert_eq!(marginalia.len(), 1);
        assert_eq!(marginalia[0].label.as_deref(), Some("Footnote"));
        assert!(marginalia[0].text.starts_with("44 That is"));
        assert!(marginalia[0].text.contains("\"Wraps,\""));
        assert!(
            marginalia[0]
                .text
                .contains("internet-agreements-to-arbitrate")
        );
        assert!(blocks.iter().any(|block| {
            block.role == LiquidBlockRole::Paragraph
                && block.text.starts_with("The next main-text paragraph")
        }));
    }

    #[test]
    fn profile_policy_keeps_different_law_review_note_numbers_separate() {
        let blocks = run_profile_specific_normalization(
            vec![
                block(LiquidBlockRole::Title, "Article Example"),
                block(
                    LiquidBlockRole::Marginalia,
                    "44 That is, the website usually does not attempt to bring the contract terms to the user's attention.",
                ),
                block(
                    LiquidBlockRole::Marginalia,
                    "44 continued citation material from the same note.",
                ),
                block(
                    LiquidBlockRole::Marginalia,
                    "45 See Example v. State, 123 U.S. 456 (2026).",
                ),
                block(
                    LiquidBlockRole::Marginalia,
                    "continued citation material from the next note.",
                ),
            ],
            DocumentProfileKind::LawReviewArticle,
        );

        let marginalia = blocks
            .iter()
            .filter(|block| block.role == LiquidBlockRole::Marginalia)
            .collect::<Vec<_>>();
        assert_eq!(marginalia.len(), 2);
        assert!(marginalia[0].text.starts_with("44 That is"));
        assert!(marginalia[0].text.contains("44 continued"));
        assert!(marginalia[1].text.starts_with("45 See Example"));
        assert!(marginalia[1].text.contains("continued citation"));
    }

    #[test]
    fn profile_policy_repairs_interrupted_law_review_marginalia_runs() {
        let blocks = run_profile_specific_normalization(
            vec![
                block(LiquidBlockRole::Title, "Article Example"),
                block(
                    LiquidBlockRole::Marginalia,
                    "Kreiczer-Levy, Pablo Lerner, Orly Lobel, Gideon Parchomovsky, Ariel Porat, Arie Reichel, Yaad",
                ),
                block(LiquidBlockRole::SectionBreak, ""),
                block(
                    LiquidBlockRole::Paragraph,
                    "Rotem, Roee Sarel, Kate Tokeley, Lauren Willis, and Eyal Zamir for excellent comments on a previ",
                ),
                block(
                    LiquidBlockRole::Paragraph,
                    "ous version; Victoria Business School and the College of Law & Business for generous financial sup",
                ),
                block(
                    LiquidBlockRole::Paragraph,
                    "port; William Britton, Shira Halbertal, and Dor Mordechai for able research assistance; and the partic",
                ),
                block(
                    LiquidBlockRole::Marginalia,
                    "ipants at the workshop contributed helpful comments.",
                ),
                block(
                    LiquidBlockRole::Paragraph,
                    "The next main-text paragraph remains in the reading flow.",
                ),
            ],
            DocumentProfileKind::LawReviewArticle,
        );

        assert!(
            !blocks
                .iter()
                .any(|block| block.role == LiquidBlockRole::SectionBreak)
        );
        for text in ["Rotem, Roee", "ous version", "port; William"] {
            marginalia_containing(&blocks, text);
        }
        assert!(blocks.iter().any(|block| {
            block.role == LiquidBlockRole::Paragraph
                && block.text.starts_with("The next main-text paragraph")
        }));
    }

    #[test]
    fn profile_policy_repairs_citation_run_before_law_review_marginalia() {
        let blocks = run_profile_specific_normalization(
            vec![
                block(LiquidBlockRole::Title, "Article Example"),
                block(
                    LiquidBlockRole::Paragraph,
                    "Main text before the note run remains in the reading flow.",
                ),
                block(
                    LiquidBlockRole::ListItem,
                    "164 Aaron Smith & Monica Anderson, Online Shopping and E-Commerce, PEW RES.CTR. 2 (Dec.",
                ),
                block(
                    LiquidBlockRole::Table,
                    "19 2016), http://assets.pewresearch.org/wp-content/uploads/sites/14/2016/12/",
                ),
                block(LiquidBlockRole::SectionBreak, ""),
                block(
                    LiquidBlockRole::Subheading,
                    "19_Online-Shopping_FINAL.pdf [https://perma.cc/LNR2-YH7A] (\"[R]oughly eight-in-ten Ameri-",
                ),
                block(
                    LiquidBlockRole::Header,
                    "cans are now online shoppers . . . .\").",
                ),
                block(
                    LiquidBlockRole::Marginalia,
                    "166 Kevin Murnane, Which Social Media Platform Is the Most Popular in the US?, FORBES (Mar.",
                ),
            ],
            DocumentProfileKind::LawReviewArticle,
        );

        assert!(blocks.iter().any(|block| {
            block.role == LiquidBlockRole::Paragraph && block.text.starts_with("Main text before")
        }));
        for text in ["164 Aaron", "19 2016", "19_Online", "cans are now"] {
            marginalia_containing(&blocks, text);
        }
    }

    #[test]
    fn profile_policy_repairs_inline_body_marker_before_for_example() {
        let blocks = run_profile_specific_normalization(
            vec![
                block(LiquidBlockRole::Title, "Article Example"),
                block(
                    LiquidBlockRole::Paragraph,
                    "The F-K test produces a score that estimates the grade level.",
                ),
                block(LiquidBlockRole::Table, "117 For"),
                block(
                    LiquidBlockRole::Paragraph,
                    "a score that estimates the grade level required to understand the text. example, an F-K score of seven indicates a seventh-grade education.",
                ),
                block(
                    LiquidBlockRole::Marginalia,
                    "118 A related readability scale reaches the same conclusion.",
                ),
            ],
            DocumentProfileKind::LawReviewArticle,
        );

        assert!(!blocks.iter().any(|block| block.text == "117 For"));
        let repaired = blocks
            .iter()
            .find(|block| block.text.contains("For example, an F-K score"))
            .expect("the bridge word should be restored into the following paragraph");
        assert_eq!(repaired.role, LiquidBlockRole::Paragraph);
    }

    #[test]
    fn profile_policy_does_not_repair_citation_note_start_as_body_marker() {
        let blocks = run_profile_specific_normalization(
            vec![
                block(LiquidBlockRole::Title, "Article Example"),
                block(LiquidBlockRole::Table, "24 Id."),
                block(
                    LiquidBlockRole::Paragraph,
                    "example, this ordinary paragraph should not receive a citation marker.",
                ),
                block(
                    LiquidBlockRole::Marginalia,
                    "25 See Example v. State, 123 U.S. 456 (2026).",
                ),
            ],
            DocumentProfileKind::LawReviewArticle,
        );

        assert!(blocks.iter().any(|block| block.text == "24 Id."));
        assert!(
            !blocks
                .iter()
                .any(|block| block.text.contains("Id. example"))
        );
    }

    #[test]
    fn profile_policy_repairs_multidigit_note_run_before_marginalia() {
        let blocks = run_profile_specific_normalization(
            vec![
                block(LiquidBlockRole::Title, "Article Example"),
                block(
                    LiquidBlockRole::Paragraph,
                    "The body paragraph explains sign-in-wrap contracts.",
                ),
                block(
                    LiquidBlockRole::Paragraph,
                    "44 That is, the website usually does not attempt to bring the contract terms to the user's attention.",
                ),
                block(LiquidBlockRole::SectionBreak, ""),
                block(
                    LiquidBlockRole::Subheading,
                    "\"Wraps,\" LITE DEPALMA GREENBERG LAW BLOG (Dec. 1, 2016), http://www.litedepalma.com/",
                ),
                block(
                    LiquidBlockRole::Header,
                    "internet-agreements-to-arbitrate-know-the-four-wraps [https://perma.cc/SZR7-V7QU]",
                ),
                block(
                    LiquidBlockRole::Marginalia,
                    "wrap agreements require users to select \"I agree\" following a showing of the terms.",
                ),
            ],
            DocumentProfileKind::LawReviewArticle,
        );

        for text in ["44 That is", "\"Wraps,\"", "internet-agreements"] {
            marginalia_containing(&blocks, text);
        }
        assert!(blocks.iter().any(|block| {
            block.role == LiquidBlockRole::Paragraph && block.text.starts_with("The body paragraph")
        }));
    }

    #[test]
    fn profile_policy_does_not_repair_one_digit_body_lists_before_marginalia() {
        let blocks = run_profile_specific_normalization(
            vec![
                block(LiquidBlockRole::Title, "Article Example"),
                block(
                    LiquidBlockRole::ListItem,
                    "1) an intellectual property clause",
                ),
                block(LiquidBlockRole::ListItem, "2) a prohibited use clause"),
                block(
                    LiquidBlockRole::Marginalia,
                    "53 This footnote supports the preceding list.",
                ),
            ],
            DocumentProfileKind::LawReviewArticle,
        );

        assert_eq!(blocks[1].role, LiquidBlockRole::ListItem);
        assert_eq!(blocks[2].role, LiquidBlockRole::ListItem);
        assert_eq!(blocks[3].role, LiquidBlockRole::Marginalia);
    }

    #[test]
    fn profile_policy_repairs_ssrn_heading_fragment_noise() {
        let blocks = run_profile_specific_normalization(
            vec![
                block(LiquidBlockRole::Title, "THE DUTY TO READ THE UNREADABLE"),
                block(LiquidBlockRole::Heading, "THE DUTY TO READ THE UNREADABLE"),
                block(
                    LiquidBlockRole::Heading,
                    "Google, Facebook, Uber, and Amazon-readable? Can American consumers",
                ),
                block(LiquidBlockRole::Heading, "United States Supreme Court3"),
                block(LiquidBlockRole::Heading, "95NY-YQB6]."),
                block(
                    LiquidBlockRole::Subheading,
                    "Consumer Clickwrap and Browsewrap Agreements and the Reasonably Communicated Test, 77",
                ),
                block(
                    LiquidBlockRole::Subheading,
                    "\"Wraps,\" LITE DEPALMA GREENBERG LAW BLOG (Dec. 1, 2016), http://www.litedepalma.com/",
                ),
                block(
                    LiquidBlockRole::Issue,
                    "B. Can Readability Indeed Make a Difference?",
                ),
                block(
                    LiquidBlockRole::Issue,
                    "C. Can Market Forces Discipline Firms?",
                ),
                block(
                    LiquidBlockRole::Takeaway,
                    "50 Sign-in-wraps also differ ing up to the website the user agrees to the contract. from browsewraps in that they explicitly notify the user that signing up to the website means",
                ),
                block(LiquidBlockRole::Heading, "I. THEORETICAL BACKGROUND"),
            ],
            DocumentProfileKind::LawReviewArticle,
        );

        let duplicate_title = blocks
            .iter()
            .find(|block| {
                block.text == "THE DUTY TO READ THE UNREADABLE"
                    && block.role != LiquidBlockRole::Title
            })
            .expect("duplicate title heading");
        assert_eq!(duplicate_title.role, LiquidBlockRole::Header);

        for text in [
            "Google, Facebook, Uber, and Amazon-readable? Can American consumers",
            "United States Supreme Court3",
        ] {
            let repaired = blocks
                .iter()
                .find(|block| block.text == text)
                .unwrap_or_else(|| panic!("missing repaired body fragment: {text}"));
            assert_eq!(repaired.role, LiquidBlockRole::Paragraph, "{text}");
        }

        for text in [
            "95NY-YQB6].",
            "Consumer Clickwrap and Browsewrap Agreements and the Reasonably Communicated Test, 77",
            "\"Wraps,\" LITE DEPALMA GREENBERG LAW BLOG (Dec. 1, 2016), http://www.litedepalma.com/",
            "50 Sign-in-wraps also differ ing up to the website the user agrees to the contract. from browsewraps in that they explicitly notify the user that signing up to the website means",
        ] {
            marginalia_containing(&blocks, text);
        }

        for text in [
            "B. Can Readability Indeed Make a Difference?",
            "C. Can Market Forces Discipline Firms?",
        ] {
            let repaired = blocks
                .iter()
                .find(|block| block.text == text)
                .unwrap_or_else(|| panic!("missing question subheading: {text}"));
            assert_eq!(repaired.role, LiquidBlockRole::Subheading, "{text}");
            assert_eq!(repaired.label, None);
        }

        let real_heading = blocks
            .iter()
            .find(|block| block.text == "I. THEORETICAL BACKGROUND")
            .expect("real heading");
        assert_eq!(real_heading.role, LiquidBlockRole::Heading);
    }

    #[test]
    fn profile_policy_repairs_law_review_table_header_and_symbol_note_noise() {
        let blocks = run_profile_specific_normalization(
            vec![
                block(LiquidBlockRole::Title, "Mend It, Bend It, and Extend It"),
                block(LiquidBlockRole::Heading, "I. INTRODUCTION"),
                block(
                    LiquidBlockRole::Heading,
                    "1996] The Fate of Traditional Law School Methodology 455",
                ),
                block(LiquidBlockRole::Table, "MERCER LAW REVIEW [Vol. 51"),
                block(LiquidBlockRole::Heading, "Mean Median Standard Deviation"),
                block(
                    LiquidBlockRole::Heading,
                    "Unique Visitors 10,169,272 7,860,347 11,246,053",
                ),
                block(
                    LiquidBlockRole::Heading,
                    "108 See Bernstam et al., supra note 102, at 16 (The higher the Flesch reading ease score, the",
                ),
                block(
                    LiquidBlockRole::Heading,
                    "HAV. RES. METHODS, INSTRUMENTS, & COMPUTERS 193, 199 (2004); Nicola J. Kalk & David D.",
                ),
                block(
                    LiquidBlockRole::Heading,
                    "See HEALTH & SAFETY LABORATORY, EVALUATION OF PRODUCT DOCUMENTATION PRO",
                ),
                block(LiquidBlockRole::Table, "1 33. As John W. Wade observes:"),
                block(LiquidBlockRole::Table, "88 Id."),
                block(LiquidBlockRole::Table, "346 At that"),
                block(
                    LiquidBlockRole::ListItem,
                    "7 1. See Carney, supra note 64, at 19 (describing the proverbial grind).",
                ),
                block(
                    LiquidBlockRole::ListItem,
                    "5 Once students understand the goals of the Langdellian method, they can focus on learning.",
                ),
                block(
                    LiquidBlockRole::ListItem,
                    "1. The Langdellian Method Compels Students to Analyze",
                ),
                block(
                    LiquidBlockRole::Subheading,
                    "* B.A. 1986, Summa Cum Laude, Loyola University Chicago; J.D. 1989, Loyola",
                ),
                block(
                    LiquidBlockRole::Paragraph,
                    "** Robert E. Scott Distinguished Professor of Law, Columbia Law School. I thank colleagues for helpful comments.",
                ),
                block(
                    LiquidBlockRole::Paragraph,
                    "Richard Fallon, Noah Feldman, Kim Forde-Mazrui, Sherif Girgis, and Daphna Renan for conversations.",
                ),
                block(
                    LiquidBlockRole::Marginalia,
                    "1 Title VI provides that no person shall be excluded from participation in a federally funded program.",
                ),
                block(
                    LiquidBlockRole::Takeaway,
                    "In sum, scholars note that law schools tailored the Langdellian method to a prior era.",
                ),
            ],
            DocumentProfileKind::LawReviewArticle,
        );

        let intro = blocks
            .iter()
            .find(|block| block.text == "I. INTRODUCTION")
            .expect("intro heading");
        assert_eq!(intro.role, LiquidBlockRole::Heading);

        let running = blocks
            .iter()
            .find(|block| block.text.starts_with("1996] The Fate"))
            .expect("running header");
        assert_eq!(running.role, LiquidBlockRole::Header);
        let journal_header = blocks
            .iter()
            .find(|block| block.text == "MERCER LAW REVIEW [Vol. 51")
            .expect("journal running header");
        assert_eq!(journal_header.role, LiquidBlockRole::Header);

        for text in [
            "Mean Median Standard Deviation",
            "Unique Visitors 10,169,272 7,860,347 11,246,053",
        ] {
            let repaired = blocks
                .iter()
                .find(|block| block.text == text)
                .unwrap_or_else(|| panic!("missing table fragment: {text}"));
            assert_eq!(repaired.role, LiquidBlockRole::Table, "{text}");
            assert_eq!(repaired.label.as_deref(), Some("Table"), "{text}");
        }

        for text in [
            "108 See Bernstam et al., supra note 102, at 16 (The higher the Flesch reading ease score, the",
            "HAV. RES. METHODS, INSTRUMENTS, & COMPUTERS 193, 199 (2004); Nicola J. Kalk & David D.",
            "See HEALTH & SAFETY LABORATORY, EVALUATION OF PRODUCT DOCUMENTATION PRO",
        ] {
            marginalia_containing(&blocks, text);
        }

        for text in [
            "1 33. As John W. Wade observes:",
            "88 Id.",
            "346 At that",
            "7 1. See Carney, supra note 64, at 19 (describing the proverbial grind).",
        ] {
            marginalia_containing(&blocks, text);
        }

        let body_marker = blocks
            .iter()
            .find(|block| block.text.starts_with("5 Once students"))
            .expect("body marker");
        assert_eq!(body_marker.role, LiquidBlockRole::Paragraph);

        let list_item = blocks
            .iter()
            .find(|block| block.text.starts_with("1. The Langdellian Method"))
            .expect("enumerated list item");
        assert_eq!(list_item.role, LiquidBlockRole::ListItem);

        let symbol_note = blocks
            .iter()
            .find(|block| block.text.starts_with("* B.A. 1986"))
            .expect("symbol note");
        assert_eq!(symbol_note.role, LiquidBlockRole::Marginalia);
        assert_eq!(symbol_note.label.as_deref(), Some("Footnote"));
        let paragraph_symbol_note = marginalia_containing(&blocks, "** Robert E. Scott");
        assert_eq!(paragraph_symbol_note.role, LiquidBlockRole::Marginalia);
        assert_eq!(paragraph_symbol_note.label.as_deref(), Some("Footnote"));
        let author_note_continuation = marginalia_containing(&blocks, "Richard Fallon");
        assert_eq!(author_note_continuation.role, LiquidBlockRole::Marginalia);
        assert_eq!(author_note_continuation.label.as_deref(), Some("Footnote"));

        let takeaway = blocks
            .iter()
            .find(|block| block.text.starts_with("In sum, scholars"))
            .expect("takeaway sentence");
        assert_eq!(takeaway.role, LiquidBlockRole::Paragraph);
        assert_eq!(takeaway.label, None);
    }

    #[test]
    fn profile_policy_repairs_mercer_style_visible_role_noise() {
        let blocks = run_profile_specific_normalization(
            vec![
                block(LiquidBlockRole::Title, "Bankruptcy"),
                block(LiquidBlockRole::Heading, "I. INTRODUCTION"),
                block(
                    LiquidBlockRole::Heading,
                    "II. Gamble v. Brown (In re Gamble)",
                ),
                block(
                    LiquidBlockRole::Heading,
                    "v. Brown (In re Gamble),' called into question the propriety of failing to",
                ),
                block(LiquidBlockRole::Header, "property.3 4"),
                block(LiquidBlockRole::Table, "1064 [Vol. 51"),
                block(LiquidBlockRole::Table, "8 At best,"),
                block(
                    LiquidBlockRole::ListItem,
                    "11 bankruptcy protection, the joint debtors formed a partnership with",
                ),
                block(
                    LiquidBlockRole::ListItem,
                    "1993. On May 3, 1996, the debtors appealed the decision of the Tax",
                ),
                block(LiquidBlockRole::Heading, "Court to the Eleventh Circuit.45"),
                block(
                    LiquidBlockRole::Heading,
                    "Section 108(b)(2) had the effect of extending the filing deadline to",
                ),
                block(
                    LiquidBlockRole::Definition,
                    "49 Furthermore, notwithstanding the debtors' argument to the contrary, their October 1990 petition for redetermination was not pending before the Tax Court when they filed their bankruptcy petition.",
                ),
                block(
                    LiquidBlockRole::Holding,
                    "payment of a tax.69 Then, relying upon the absence of the phrase \"or the payment thereof\" from Section 523(a)(1)(C), the court concluded that federal bankruptcy law precludes discharge.",
                ),
                block(
                    LiquidBlockRole::KeyClause,
                    "20001 1073 stock in the underlying corporations and certain other assets to himself and his new wife as tenants by the entirety.",
                ),
            ],
            DocumentProfileKind::LawReviewArticle,
        );

        assert_eq!(
            blocks
                .iter()
                .find(|block| block.text == "I. INTRODUCTION")
                .expect("intro heading")
                .role,
            LiquidBlockRole::Heading
        );
        assert_eq!(
            blocks
                .iter()
                .find(|block| block.text == "II. Gamble v. Brown (In re Gamble)")
                .expect("numbered heading")
                .role,
            LiquidBlockRole::Heading
        );
        for text in [
            "v. Brown (In re Gamble),' called into question the propriety of failing to",
            "property.3 4",
            "11 bankruptcy protection, the joint debtors formed a partnership with",
            "1993. On May 3, 1996, the debtors appealed the decision of the Tax",
            "Court to the Eleventh Circuit.45",
            "Section 108(b)(2) had the effect of extending the filing deadline to",
            "payment of a tax.69 Then, relying upon the absence of the phrase \"or the payment thereof\" from Section 523(a)(1)(C), the court concluded that federal bankruptcy law precludes discharge.",
            "20001 1073 stock in the underlying corporations and certain other assets to himself and his new wife as tenants by the entirety.",
        ] {
            let repaired = blocks
                .iter()
                .find(|block| block.text == text)
                .unwrap_or_else(|| panic!("missing repaired body fragment: {text}"));
            assert_eq!(repaired.role, LiquidBlockRole::Paragraph, "{text}");
        }

        let page_cite = blocks
            .iter()
            .find(|block| block.text == "1064 [Vol. 51")
            .expect("running page cite");
        assert_eq!(page_cite.role, LiquidBlockRole::Header);

        let footnote = blocks
            .iter()
            .find(|block| block.text.starts_with("49 Furthermore"))
            .expect("footnote fragment");
        assert_eq!(footnote.role, LiquidBlockRole::Marginalia);
        assert_eq!(footnote.label.as_deref(), Some("Footnote"));

        let short_note = blocks
            .iter()
            .find(|block| block.text == "8 At best,")
            .expect("short table note fragment");
        assert_eq!(short_note.role, LiquidBlockRole::Marginalia);
        assert_eq!(short_note.label.as_deref(), Some("Footnote"));
    }

    #[test]
    fn profile_policy_strips_law_review_repository_cover_scaffold() {
        let blocks = run_profile_specific_normalization(
            vec![
                block(LiquidBlockRole::Title, "Bankruptcy"),
                block(LiquidBlockRole::Metadata, "Volume 51"),
                block(LiquidBlockRole::Heading, "Article 5"),
                block(LiquidBlockRole::Heading, "Number 4 Eleventh Circuit Survey"),
                block(
                    LiquidBlockRole::Marginalia,
                    "Drake, W.H. Jr. and Strickland, Christopher S. (2000) \"Bankruptcy,\" Mercer Law Review: Vol. 51: No. 4, Article 5.",
                ),
                block(
                    LiquidBlockRole::Metadata,
                    "This Survey Article is brought to you for free and open access by the Journals at Mercer Law School Digital",
                ),
                block(
                    LiquidBlockRole::Metadata,
                    "Digital Commons. For more information, please contact repository@law.mercer.edu.",
                ),
                block(
                    LiquidBlockRole::Heading,
                    "by The Honorable W.H. Drake, Jr.*",
                ),
                block(LiquidBlockRole::Subheading, "and"),
                block(LiquidBlockRole::Heading, "Christopher S. Strickland**"),
                block(LiquidBlockRole::Heading, "I. INTRODUCTION"),
                block(
                    LiquidBlockRole::Paragraph,
                    "The article body begins here and should remain visible.",
                ),
            ],
            DocumentProfileKind::LawReviewArticle,
        );

        assert!(!blocks.iter().any(|block| {
            matches!(
                block.text.as_str(),
                "Volume 51" | "Article 5" | "Number 4 Eleventh Circuit Survey" | "and"
            ) || block.text.contains("brought to you")
                || block.text.contains("repository@")
                || block.text.contains("Mercer Law Review: Vol. 51")
        }));
        assert!(blocks.iter().any(|block| {
            block.role == LiquidBlockRole::AuthorInfo
                && block.text == "The Honorable W.H. Drake, Jr.*"
        }));
        assert!(blocks.iter().any(|block| {
            block.role == LiquidBlockRole::AuthorInfo && block.text == "Christopher S. Strickland**"
        }));
        assert!(blocks.iter().any(|block| {
            block.role == LiquidBlockRole::Heading && block.text == "I. INTRODUCTION"
        }));
    }

    #[test]
    fn profile_policy_strips_duplicate_repository_title_fragments() {
        let title = "Mend It, Bend It, and Extend It: The Fate of Traditional Law School Methodology in the 21st Century";
        let blocks = run_profile_specific_normalization(
            vec![
                block(LiquidBlockRole::Title, title),
                block(LiquidBlockRole::Metadata, "Volume 27"),
                block(LiquidBlockRole::Metadata, "Issue 3 Spring 1996 Article 2"),
                block(
                    LiquidBlockRole::Heading,
                    "Mend It, Bend It, and Extend It: The Fate of",
                ),
                block(
                    LiquidBlockRole::AuthorInfo,
                    "Traditional Law School Methodology in the 21st",
                ),
                block(LiquidBlockRole::Heading, "Century"),
                block(LiquidBlockRole::Heading, "Ruta K. Stropus"),
                block(LiquidBlockRole::Heading, "Northern Illinois Law School"),
                block(
                    LiquidBlockRole::Marginalia,
                    "Ruta K. Stropus, Mend It, Bend It, and Extend It: The Fate of Traditional Law School Methodology in the 21st Century, 27 Loy. U. Chi. L. J. 449 (1996).",
                ),
                block(
                    LiquidBlockRole::Metadata,
                    "Journal by an authorized administrator of LAW eCommons. For more information, please contact law-library@luc.edu.",
                ),
                block(LiquidBlockRole::Heading, "I. INTRODUCTION"),
                block(
                    LiquidBlockRole::Paragraph,
                    "The article body begins here and should remain visible.",
                ),
            ],
            DocumentProfileKind::LawReviewArticle,
        );

        assert!(!blocks.iter().any(|block| {
            block.text == "Volume 27"
                || block.text == "Issue 3 Spring 1996 Article 2"
                || block.text == "Mend It, Bend It, and Extend It: The Fate of"
                || block.text == "Traditional Law School Methodology in the 21st"
                || block.text == "Century"
                || block.text.contains("LAW eCommons")
                || block.text.contains("Loy. U. Chi. L. J.")
        }));
        assert!(blocks.iter().any(|block| {
            block.role == LiquidBlockRole::AuthorInfo && block.text == "Ruta K. Stropus"
        }));
        assert!(blocks.iter().any(|block| {
            block.role == LiquidBlockRole::AuthorInfo
                && block.text == "Northern Illinois Law School"
        }));
        assert!(blocks.iter().any(|block| {
            block.role == LiquidBlockRole::Heading && block.text == "I. INTRODUCTION"
        }));
    }

    #[test]
    fn profile_policy_keeps_article_heading_without_repository_context() {
        let blocks = run_profile_specific_normalization(
            vec![
                block(LiquidBlockRole::Title, "Contract Structure"),
                block(LiquidBlockRole::Heading, "Article 5"),
                block(
                    LiquidBlockRole::Paragraph,
                    "This is a real section heading in a non-repository article.",
                ),
            ],
            DocumentProfileKind::LawReviewArticle,
        );

        assert!(
            blocks.iter().any(|block| {
                block.role == LiquidBlockRole::Heading && block.text == "Article 5"
            })
        );
    }

    #[test]
    fn profile_policy_demotes_cv_records_misclassified_as_headings() {
        let blocks = run_profile_specific_normalization(
            vec![
                block(LiquidBlockRole::Title, "Curriculum Vitae"),
                block(LiquidBlockRole::Heading, "Education"),
                block(LiquidBlockRole::SectionBreak, ""),
                block(
                    LiquidBlockRole::Heading,
                    "University of Alabama School of Law, Assistant Professor, 2024.",
                ),
            ],
            DocumentProfileKind::CvOrAcademicPacket,
        );

        assert_eq!(blocks[1].role, LiquidBlockRole::Heading);
        assert_eq!(blocks[2].role, LiquidBlockRole::ListItem);
    }

    #[test]
    fn profile_policy_activates_on_practice_cv_section_headings() {
        let blocks = run_profile_specific_normalization(
            vec![
                block(LiquidBlockRole::Title, "Harvey A. Hutchinson"),
                block(LiquidBlockRole::Heading, "BIOGRAPHICAL INFORMATION"),
                block(
                    LiquidBlockRole::Paragraph,
                    "Experienced teacher and attorney.",
                ),
                block(LiquidBlockRole::Heading, "ACADEMIC BACKGROUND"),
                block(LiquidBlockRole::Heading, "MASTER OF LAWS, TAXATION 2005"),
                block(LiquidBlockRole::Heading, "COURSES TAUGHT"),
                block(
                    LiquidBlockRole::Heading,
                    "ACCOUNTING PRINCIPLES FEDERAL INCOME TAXATION",
                ),
                block(LiquidBlockRole::Heading, "PUBLICATIONS AND CONTRIBUTIONS"),
                block(
                    LiquidBlockRole::Heading,
                    "Hutchinson, Harvey. Call for Adaptive Estate Plans. NAEPC Journal 2025.",
                ),
            ],
            DocumentProfileKind::CvOrAcademicPacket,
        );

        assert_eq!(blocks[4].role, LiquidBlockRole::ListItem);
        assert_eq!(blocks[6].role, LiquidBlockRole::ListItem);
        assert_eq!(blocks[8].role, LiquidBlockRole::ListItem);
    }

    #[test]
    fn profile_policy_structures_faculty_application_fields() {
        let blocks = run_profile_specific_normalization(
            vec![
                block(LiquidBlockRole::Title, "Application: Christian Johnson"),
                block(LiquidBlockRole::Paragraph, "Posting number: 0812950"),
                block(
                    LiquidBlockRole::Subheading,
                    "Posting: D. Paul Jones, Jr. & Charlene Jones Chairholder of Law (Faculty)",
                ),
                block(LiquidBlockRole::Heading, "Form: Faculty Application"),
                block(
                    LiquidBlockRole::Subheading,
                    "Submitted: June 17, 2021 at 11:57 AM (CDT)",
                ),
                block(LiquidBlockRole::Heading, "Personal Information"),
                block(LiquidBlockRole::Heading, "First Name: Christian"),
                block(LiquidBlockRole::Heading, "Last Name: Johnson"),
                block(
                    LiquidBlockRole::Heading,
                    "Country of Residence United States of America",
                ),
                block(LiquidBlockRole::Heading, "Required Documents"),
                block(LiquidBlockRole::Heading, "Kind Name Conversion Status"),
                block(
                    LiquidBlockRole::Heading,
                    "Resume / Curriculum Vitae 06-17-21 11:51:09",
                ),
                block(LiquidBlockRole::Paragraph, "PDF complete"),
                block(
                    LiquidBlockRole::Heading,
                    "PRESENT POSITION & PRIOR ACADEMIC EMPLOYMENT",
                ),
                block(LiquidBlockRole::Heading, "Professor of Law"),
            ],
            DocumentProfileKind::CvOrAcademicPacket,
        );

        assert_eq!(blocks[1].role, LiquidBlockRole::Table);
        assert_eq!(blocks[2].role, LiquidBlockRole::Table);
        assert_eq!(blocks[3].role, LiquidBlockRole::Table);
        assert_eq!(blocks[4].role, LiquidBlockRole::Table);
        assert_eq!(blocks[6].role, LiquidBlockRole::Table);
        assert_eq!(blocks[8].role, LiquidBlockRole::Table);
        assert_eq!(blocks[9].role, LiquidBlockRole::Table);
        assert_eq!(blocks[10].role, LiquidBlockRole::Table);
        assert_eq!(blocks[11].role, LiquidBlockRole::Table);
        assert_eq!(blocks[12].role, LiquidBlockRole::Table);
        assert_eq!(blocks[13].role, LiquidBlockRole::Heading);
        assert_eq!(blocks[14].role, LiquidBlockRole::ListItem);
    }

    #[test]
    fn profile_policy_demotes_short_cv_article_titles_after_section_heading() {
        let blocks = run_profile_specific_normalization(
            vec![
                block(LiquidBlockRole::Title, "Curriculum Vitae"),
                block(LiquidBlockRole::Heading, "Selected Publications"),
                block(LiquidBlockRole::Heading, "Contracting Over Privacy"),
            ],
            DocumentProfileKind::CvOrAcademicPacket,
        );

        assert_eq!(blocks[1].role, LiquidBlockRole::Heading);
        assert_eq!(blocks[2].role, LiquidBlockRole::ListItem);
    }

    #[test]
    fn profile_policy_preserves_course_questions_as_list_items() {
        let blocks = run_profile_specific_normalization(
            vec![block(
                LiquidBlockRole::Paragraph,
                "Question 1. What result if the offer was revoked before acceptance?",
            )],
            DocumentProfileKind::CourseOrExamMaterial,
        );

        assert_eq!(blocks[0].role, LiquidBlockRole::ListItem);
    }

    #[test]
    fn profile_policy_marks_syllabus_assignment_rows_as_tables() {
        let blocks = run_profile_specific_normalization(
            vec![
                block(LiquidBlockRole::Title, "Secured Transactions"),
                block(LiquidBlockRole::Syllabus, "SYLLABUS"),
                block(LiquidBlockRole::Heading, "Assignments"),
                block(
                    LiquidBlockRole::Subheading,
                    "A. Remedies of Unsecured Creditors Under State Law",
                ),
                block(LiquidBlockRole::Heading, "• LoPucki & Warren 3-18"),
                block(LiquidBlockRole::Heading, "• Problem Set 1"),
                block(LiquidBlockRole::Subheading, "B. Security and Foreclosure"),
                block(
                    LiquidBlockRole::Heading,
                    "• Problem Set 2: Problems 2.1-2.6",
                ),
            ],
            DocumentProfileKind::CourseOrExamMaterial,
        );

        assert_eq!(blocks[4].role, LiquidBlockRole::Table);
        assert_eq!(blocks[5].role, LiquidBlockRole::Table);
        assert_eq!(blocks[7].role, LiquidBlockRole::Table);
    }

    #[test]
    fn profile_policy_marks_course_evaluation_rows_as_tables() {
        let blocks = run_profile_specific_normalization(
            vec![
                block(LiquidBlockRole::Heading, "--- Survey Comparisons ---"),
                block(LiquidBlockRole::Heading, "Response Option"),
                block(
                    LiquidBlockRole::Heading,
                    "VMS IMR GE IMR TD N Mean Med. Mode Std",
                ),
                block(LiquidBlockRole::Heading, "5.0"),
                block(LiquidBlockRole::Heading, "Q2 Always"),
                block(LiquidBlockRole::Heading, "Excellent"),
                block(LiquidBlockRole::Heading, "COMPLETE"),
                block(
                    LiquidBlockRole::Heading,
                    "1 - Was this course, all things considered, successful?",
                ),
                block(LiquidBlockRole::Heading, "Q13 Written Comments"),
            ],
            DocumentProfileKind::CourseOrExamMaterial,
        );

        assert_eq!(blocks[0].role, LiquidBlockRole::Table);
        assert_eq!(blocks[1].role, LiquidBlockRole::Table);
        assert_eq!(blocks[2].role, LiquidBlockRole::Table);
        assert_eq!(blocks[3].role, LiquidBlockRole::Table);
        assert_eq!(blocks[4].role, LiquidBlockRole::Table);
        assert_eq!(blocks[5].role, LiquidBlockRole::Table);
        assert_eq!(blocks[6].role, LiquidBlockRole::Table);
        assert_eq!(blocks[7].role, LiquidBlockRole::ListItem);
        assert_eq!(blocks[8].role, LiquidBlockRole::Heading);
    }

    #[test]
    fn profile_policy_marks_individual_course_eval_answers_as_tables() {
        let blocks = run_profile_specific_normalization(
            vec![
                block(LiquidBlockRole::Heading, "#1 Web Link 1 (Web Link)"),
                block(LiquidBlockRole::Heading, "Q2 Always"),
                block(LiquidBlockRole::Heading, "Q3 Excellent"),
                block(
                    LiquidBlockRole::Paragraph,
                    "How often did you attend class?",
                ),
                block(LiquidBlockRole::Heading, "Written Comments"),
                block(
                    LiquidBlockRole::Paragraph,
                    "Professor Shelby was approachable and engaging.",
                ),
            ],
            DocumentProfileKind::CourseOrExamMaterial,
        );

        assert_eq!(blocks[0].role, LiquidBlockRole::Table);
        assert_eq!(blocks[1].role, LiquidBlockRole::Table);
        assert_eq!(blocks[2].role, LiquidBlockRole::Table);
        assert_eq!(blocks[3].role, LiquidBlockRole::ListItem);
        assert_eq!(blocks[4].role, LiquidBlockRole::Heading);
        assert_eq!(blocks[5].role, LiquidBlockRole::Paragraph);
    }

    #[test]
    fn profile_policy_structures_codebook_variable_fields() {
        let blocks = run_profile_specific_normalization(
            vec![
                block(
                    LiquidBlockRole::Heading,
                    "Draft Codebook - Nondisclosure / Confidentiality Agreements",
                ),
                block(LiquidBlockRole::Heading, "OLD VARIABLES"),
                block(LiquidBlockRole::Paragraph, "plaintiff"),
                block(LiquidBlockRole::SectionBreak, ""),
                block(LiquidBlockRole::Heading, "Description: Name of plaintiff"),
                block(LiquidBlockRole::Heading, "Type: Text string"),
                block(
                    LiquidBlockRole::Heading,
                    "Format: Full name, abbreviated per Bluebook Example: Dayton Superior Corp.",
                ),
                block(LiquidBlockRole::Heading, "Notes: Imported from CNC dataset"),
                block(LiquidBlockRole::ListItem, "1 STR. AGREE"),
            ],
            DocumentProfileKind::GeneralDocument,
        );

        assert_eq!(blocks[1].role, LiquidBlockRole::Heading);
        assert_eq!(blocks[2].role, LiquidBlockRole::Subheading);
        assert_eq!(blocks[3].role, LiquidBlockRole::Table);
        assert_eq!(blocks[4].role, LiquidBlockRole::Table);
        assert_eq!(blocks[5].role, LiquidBlockRole::Table);
        assert_eq!(blocks[6].role, LiquidBlockRole::Table);
        assert_eq!(blocks[7].role, LiquidBlockRole::Table);
    }

    #[test]
    fn profile_policy_structures_property_deal_fields() {
        let blocks = run_profile_specific_normalization(
            vec![
                block(
                    LiquidBlockRole::Title,
                    "Join Our Whatsapp Community to Get First Access to Our Off Market Deals!",
                ),
                block(LiquidBlockRole::Heading, "Welcome!"),
                block(LiquidBlockRole::Heading, "1738 SW 17th St, Miami,"),
                block(LiquidBlockRole::SectionBreak, ""),
                block(LiquidBlockRole::Heading, "$ Price: $815,000"),
                block(LiquidBlockRole::Heading, "^ ARV: $1.3m+"),
                block(LiquidBlockRole::Heading, "* Beds: 4"),
                block(LiquidBlockRole::Heading, "* Baths: 2"),
                block(LiquidBlockRole::Heading, "* Sqft: 1,938"),
                block(
                    LiquidBlockRole::Header,
                    "Layout: 3/2 Main + 1/1 Attached Efficiency",
                ),
                block(LiquidBlockRole::Heading, "Occupancy: Vacant"),
            ],
            DocumentProfileKind::GeneralDocument,
        );

        assert_eq!(blocks[2].role, LiquidBlockRole::Heading);
        assert_eq!(blocks[3].role, LiquidBlockRole::Table);
        assert_eq!(blocks[4].role, LiquidBlockRole::Table);
        assert_eq!(blocks[5].role, LiquidBlockRole::Table);
        assert_eq!(blocks[6].role, LiquidBlockRole::Table);
        assert_eq!(blocks[7].role, LiquidBlockRole::Table);
        assert_eq!(blocks[8].role, LiquidBlockRole::Table);
        assert_eq!(blocks[9].role, LiquidBlockRole::Table);
    }

    #[test]
    fn profile_policy_structures_event_logistics_fields() {
        let blocks = run_profile_specific_normalization(
            vec![
                block(LiquidBlockRole::Title, "Inaugural AI Law Safety Roundtable"),
                block(LiquidBlockRole::Heading, "1. Logistics"),
                block(LiquidBlockRole::Heading, "2. Accommodation Options"),
                block(LiquidBlockRole::Heading, "Booking Deadline: April 10, 2025"),
                block(
                    LiquidBlockRole::Heading,
                    "Hotel: Homewood Suites by Hilton Tuscaloosa Downtown",
                ),
                block(
                    LiquidBlockRole::Heading,
                    "Group Name: Inaugural AI Safety Roundtable",
                ),
                block(LiquidBlockRole::Heading, "Booking Link: link"),
                block(
                    LiquidBlockRole::Paragraph,
                    "Assistance: Contact Adrea Bishop (205.498.0178)",
                ),
            ],
            DocumentProfileKind::GeneralDocument,
        );

        assert_eq!(blocks[1].role, LiquidBlockRole::Heading);
        assert_eq!(blocks[2].role, LiquidBlockRole::Heading);
        assert_eq!(blocks[3].role, LiquidBlockRole::Table);
        assert_eq!(blocks[4].role, LiquidBlockRole::Table);
        assert_eq!(blocks[5].role, LiquidBlockRole::Table);
        assert_eq!(blocks[6].role, LiquidBlockRole::Table);
        assert_eq!(blocks[7].role, LiquidBlockRole::Table);
    }

    #[test]
    fn profile_policy_marks_restatement_subjects_as_list_items() {
        let blocks = run_profile_specific_normalization(
            vec![
                block(LiquidBlockRole::Title, "Restatement Consumer Contracts"),
                block(LiquidBlockRole::Paragraph, "The American Law Institute"),
                block(LiquidBlockRole::Heading, "SUBJECTS COVERED"),
                block(LiquidBlockRole::Heading, "§ 1. Definitions and Scope"),
                block(
                    LiquidBlockRole::Heading,
                    "§ 2. Adoption of Standard Contract Terms",
                ),
                block(LiquidBlockRole::Heading, "APPENDIX"),
                block(LiquidBlockRole::Heading, "Black Letter of Tentative Draft"),
            ],
            DocumentProfileKind::BookOrChapter,
        );

        assert_eq!(blocks[2].role, LiquidBlockRole::Heading);
        assert_eq!(blocks[3].role, LiquidBlockRole::ListItem);
        assert_eq!(blocks[4].role, LiquidBlockRole::ListItem);
        assert_eq!(blocks[5].role, LiquidBlockRole::Heading);
        assert_eq!(blocks[6].role, LiquidBlockRole::Heading);
    }

    #[test]
    fn profile_policy_structures_policy_report_basic_statistics() {
        let blocks = run_profile_specific_normalization(
            vec![
                block(LiquidBlockRole::Title, "OECD Economic Surveys"),
                block(LiquidBlockRole::Heading, "BASIC STATISTICS OF DENMARK"),
                block(LiquidBlockRole::Heading, "THE LAND"),
                block(LiquidBlockRole::Heading, "Area (km2)"),
                block(LiquidBlockRole::Heading, "Total 43 098"),
                block(
                    LiquidBlockRole::Heading,
                    "Population of major urban areas (thousands, 2011)",
                ),
                block(LiquidBlockRole::Heading, "Copenhagen 1 199"),
                block(LiquidBlockRole::Heading, "THE PEOPLE"),
                block(LiquidBlockRole::Heading, "Population (thousands, 2011)"),
                block(LiquidBlockRole::Heading, "EXECUTIVE SUMMARY"),
            ],
            DocumentProfileKind::PolicyReport,
        );

        assert_eq!(blocks[1].role, LiquidBlockRole::Heading);
        assert_eq!(blocks[2].role, LiquidBlockRole::Heading);
        assert_eq!(blocks[3].role, LiquidBlockRole::Table);
        assert_eq!(blocks[4].role, LiquidBlockRole::Table);
        assert_eq!(blocks[5].role, LiquidBlockRole::Table);
        assert_eq!(blocks[6].role, LiquidBlockRole::Table);
        assert_eq!(blocks[7].role, LiquidBlockRole::Heading);
        assert_eq!(blocks[8].role, LiquidBlockRole::Table);
        assert_eq!(blocks[9].role, LiquidBlockRole::Heading);
    }

    #[test]
    fn profile_policy_marks_reference_contacts_as_list_items() {
        let input = vec![
            block(LiquidBlockRole::Title, "References"),
            block(LiquidBlockRole::Heading, "REFERENCES - YONATHAN ARBEL"),
            block(LiquidBlockRole::AuthorInfo, "Research References"),
            block(
                LiquidBlockRole::AuthorInfo,
                "1. Steven Shavell, Samuel R. Rosenthal Professor of Law and Economics",
            ),
            block(
                LiquidBlockRole::Heading,
                "Harvard Law School, Cambridge, MA - Hauser 508 - (617) 495-3668",
            ),
            block(LiquidBlockRole::Paragraph, "shavell@law.harvard.edu"),
            block(
                LiquidBlockRole::Heading,
                "2. Henry Smith, Fessenden Professor of Law",
            ),
            block(LiquidBlockRole::Paragraph, "hesmith@law.harvard.edu"),
            block(
                LiquidBlockRole::Heading,
                "10. George Fisher, Judge John Crown Professor of Law (general)",
            ),
            block(LiquidBlockRole::Paragraph, "fisherg@stanford.edu"),
        ];
        let blocks = run_profile_specific_normalization(input.clone(), DocumentProfileKind::Other);

        assert_eq!(blocks[3].role, LiquidBlockRole::ListItem);
        assert_eq!(blocks[6].role, LiquidBlockRole::ListItem);
        assert_eq!(blocks[8].role, LiquidBlockRole::ListItem);

        let cv_blocks =
            run_profile_specific_normalization(input, DocumentProfileKind::CvOrAcademicPacket);
        assert_eq!(cv_blocks[3].role, LiquidBlockRole::ListItem);
        assert_eq!(cv_blocks[6].role, LiquidBlockRole::ListItem);
        assert_eq!(cv_blocks[8].role, LiquidBlockRole::ListItem);
    }

    #[test]
    fn profile_policy_marks_contract_clauses() {
        let blocks = run_profile_specific_normalization(
            vec![block(
                LiquidBlockRole::Paragraph,
                "Section 5. Payment. The party shall pay all undisputed invoices within thirty days.",
            )],
            DocumentProfileKind::Contract,
        );

        assert_eq!(blocks[0].role, LiquidBlockRole::Clause);
    }

    #[test]
    fn profile_policy_marks_legal_opinion_reader_aids() {
        let blocks = run_profile_specific_normalization(
            vec![
                block(
                    LiquidBlockRole::Title,
                    "IN THE SUPREME COURT OF THE STATE OF NEVADA",
                ),
                block(
                    LiquidBlockRole::Paragraph,
                    "Appeal from a district court order denying an anti-SLAPP special motion. Reversed and remanded with instructions.",
                ),
                block(LiquidBlockRole::Heading, "OPINION"),
                block(
                    LiquidBlockRole::Paragraph,
                    "At issue in this case are allegedly defamatory campaign statements. We hold that the district court applied the wrong test, and we further conclude that remand is required.",
                ),
            ],
            DocumentProfileKind::LegalFilingOrOpinion,
        );

        assert_eq!(blocks[1].role, LiquidBlockRole::Syllabus);
        assert_eq!(blocks[3].role, LiquidBlockRole::Holding);
    }

    #[test]
    fn profile_policy_keeps_receipt_fields_structured() {
        let blocks = run_profile_specific_normalization(
            vec![block(
                LiquidBlockRole::Paragraph,
                "Subtotal: $90.00\nTax: $7.20\nTotal: $97.20",
            )],
            DocumentProfileKind::ReceiptInvoiceFinancial,
        );

        assert_eq!(blocks[0].role, LiquidBlockRole::Table);
    }

    #[test]
    fn profile_policy_structures_expense_report_rows_from_headings() {
        let blocks = run_profile_specific_normalization(
            vec![
                block(LiquidBlockRole::Title, "Stanford Law and Economics Seminar"),
                block(LiquidBlockRole::Heading, "Project Expense Report"),
                block(LiquidBlockRole::Heading, "Expense Register"),
                block(
                    LiquidBlockRole::Heading,
                    "Date Vendor Description Category Amount",
                ),
                block(
                    LiquidBlockRole::Heading,
                    "2026-02-11 Uber Ground Transport USD 37.92",
                ),
                block(LiquidBlockRole::Heading, "Total Amount USD 945.09"),
            ],
            DocumentProfileKind::ReceiptInvoiceFinancial,
        );

        assert_eq!(blocks[3].role, LiquidBlockRole::Table);
        assert_eq!(blocks[4].role, LiquidBlockRole::Table);
        assert_eq!(blocks[5].role, LiquidBlockRole::Table);
    }

    #[test]
    fn profile_policy_structures_grant_application_rows_despite_profile_guess() {
        let blocks = run_profile_specific_normalization(
            vec![
                block(
                    LiquidBlockRole::Title,
                    "Research Grant Application no. 762/25",
                ),
                block(LiquidBlockRole::Heading, "Personal Research Grants"),
                block(LiquidBlockRole::Heading, "General application information"),
                block(
                    LiquidBlockRole::Heading,
                    "Role Name Academic Rank Department Institute",
                ),
                block(
                    LiquidBlockRole::Heading,
                    "PI.1 Uri Benoliel Associate Professor Law Academic College of Law",
                ),
                block(LiquidBlockRole::Heading, "Requested Budget in NIS"),
            ],
            DocumentProfileKind::LawReviewArticle,
        );

        assert_eq!(blocks[3].role, LiquidBlockRole::Table);
        assert_eq!(blocks[4].role, LiquidBlockRole::Table);
        assert_eq!(blocks[5].role, LiquidBlockRole::Table);
    }

    #[test]
    fn local_normalization_promotes_dense_key_value_tables() {
        let blocks = run_local_normalization(vec![block(
            LiquidBlockRole::Paragraph,
            "Course LAW211 A\nResponses / Expected: 20 / 56\nOverall Mean: 4.0\nMedian: 4\nMode: 5",
        )]);

        assert_eq!(blocks[0].role, LiquidBlockRole::Table);
    }

    #[test]
    fn local_normalization_keeps_ordinary_colon_prose_as_paragraph() {
        let blocks = run_local_normalization(vec![block(
            LiquidBlockRole::Paragraph,
            "The point is simple: courts should avoid converting every extracted label into a table. This sentence provides ordinary prose with a colon.",
        )]);

        assert_eq!(blocks[0].role, LiquidBlockRole::Paragraph);
    }
}
fn normalize_local_abstract_sections(blocks: Vec<LiquidBlock>) -> Vec<LiquidBlock> {
    let mut output = Vec::with_capacity(blocks.len());
    let mut index = 0usize;

    while index < blocks.len() {
        let block = &blocks[index];
        if is_standalone_abstract_heading(block) {
            let mut abstract_parts = Vec::new();
            let mut cursor = index + 1;

            while let Some(next) = blocks.get(cursor) {
                if !can_fold_into_standalone_abstract(next) {
                    break;
                }
                abstract_parts.push(next.text.trim().to_owned());
                cursor += 1;
            }

            if let Some(first_part) = abstract_parts.first() {
                output.push(LiquidBlock {
                    role: LiquidBlockRole::Abstract,
                    text: if abstract_parts.len() == 1 {
                        first_part.clone()
                    } else {
                        abstract_parts.join("\n\n")
                    },
                    label: None,
                });
                index = cursor;
                continue;
            }
        }

        output.push(block.clone());
        index += 1;
    }

    output
}

fn can_fold_into_standalone_abstract(block: &LiquidBlock) -> bool {
    let text = block.text.trim();
    if text.is_empty()
        || is_standalone_abstract_heading(block)
        || standalone_front_matter_metadata_label(block).is_some()
        || looks_like_front_matter_metadata(text)
        || looks_like_toc_entry(text)
        || looks_like_footnote_line(text)
        || end_matter_label(text).is_some()
    {
        return false;
    }

    matches!(
        block.role,
        LiquidBlockRole::Abstract
            | LiquidBlockRole::Syllabus
            | LiquidBlockRole::Paragraph
            | LiquidBlockRole::Lead
            | LiquidBlockRole::ListItem
            | LiquidBlockRole::Explainer
            | LiquidBlockRole::Takeaway
            | LiquidBlockRole::Holding
            | LiquidBlockRole::Issue
            | LiquidBlockRole::Definition
            | LiquidBlockRole::Quote
    )
}

fn is_standalone_abstract_heading(block: &LiquidBlock) -> bool {
    if block.role != LiquidBlockRole::Heading {
        return false;
    }
    matches!(
        block.text.trim().to_ascii_lowercase().as_str(),
        "abstract" | "summary"
    )
}

fn normalize_local_front_matter_metadata_sections(blocks: Vec<LiquidBlock>) -> Vec<LiquidBlock> {
    let mut output = Vec::with_capacity(blocks.len());
    let mut index = 0usize;

    while index < blocks.len() {
        let block = &blocks[index];
        if let Some(label) = standalone_front_matter_metadata_label(block) {
            let mut parts = Vec::new();
            let mut cursor = index + 1;

            while let Some(next) = blocks.get(cursor) {
                if !can_fold_front_matter_metadata_body(label, next) {
                    break;
                }
                parts.push(next.text.trim().to_owned());
                cursor += 1;
                if !front_matter_label_allows_multiple_bodies(label) {
                    break;
                }
            }

            if !parts.is_empty() {
                output.push(LiquidBlock {
                    role: LiquidBlockRole::Metadata,
                    text: front_matter_metadata_text(label, &parts),
                    label: None,
                });
                index = cursor;
                continue;
            }

            output.push(LiquidBlock {
                role: LiquidBlockRole::Metadata,
                text: block.text.clone(),
                label: None,
            });
            index += 1;
            continue;
        }

        output.push(block.clone());
        index += 1;
    }

    output
}

fn normalize_local_reader_aid_sections(blocks: Vec<LiquidBlock>) -> Vec<LiquidBlock> {
    let mut output = Vec::with_capacity(blocks.len());
    let mut index = 0usize;

    while index < blocks.len() {
        let block = &blocks[index];
        if let Some(label) = standalone_reader_aid_section_label(block) {
            let mut parts = Vec::new();
            let mut cursor = index + 1;

            while parts.len() < MAX_READER_AID_SECTION_BLOCKS {
                let Some(next) = blocks.get(cursor) else {
                    break;
                };
                if !can_fold_into_reader_aid_section(next) {
                    break;
                }
                parts.push((next.text.trim().to_owned(), next.role));
                cursor += 1;
            }

            let overflowed = blocks
                .get(cursor)
                .is_some_and(can_fold_into_reader_aid_section);
            if !overflowed && reader_aid_parts_are_compact(label, &parts) {
                output.push(LiquidBlock {
                    role: LiquidBlockRole::Takeaway,
                    text: reader_aid_callout_text(&parts),
                    label: Some(label.to_owned()),
                });
                index = cursor;
                continue;
            }
        }

        output.push(block.clone());
        index += 1;
    }

    output
}

fn standalone_reader_aid_section_label(block: &LiquidBlock) -> Option<&'static str> {
    match normalize_reference_heading(&block.text).as_str() {
        "highlights" | "article highlights" => Some("Highlights"),
        "key point" | "key points" => Some("Key points"),
        "key takeaway" | "key takeaways" | "takeaways" => Some("Key takeaways"),
        "quick take" | "quick takes" => Some("Quick take"),
        "at a glance" => Some("At a glance"),
        "what to know" => Some("What to know"),
        "factbox" | "fact box" => Some("Factbox"),
        "key fact" | "key facts" | "fast facts" => Some("Key facts"),
        "why it matters" => Some("Why it matters"),
        "the latest" => Some("The latest"),
        "at stake" | "whats at stake" | "what's at stake" => Some("At stake"),
        "what we know" => Some("What we know"),
        "what we dont know" | "what we don't know" => Some("What we don't know"),
        "timeline" | "key dates" | "important dates" | "chronology" => Some("Key dates"),
        "how we got here" => Some("How we got here"),
        "key finding" | "key findings" => Some("Key findings"),
        _ => None,
    }
}

fn can_fold_into_reader_aid_section(block: &LiquidBlock) -> bool {
    let text = block.text.trim();
    if text.is_empty()
        || looks_like_front_matter_metadata(text)
        || looks_like_abstract(text)
        || looks_like_toc_entry(text)
        || looks_like_footnote_line(text)
        || looks_like_caption(text, 0)
        || end_matter_label(text).is_some()
        || is_reference_section_heading(block)
        || standalone_front_matter_metadata_label(block).is_some()
        || standalone_reader_aid_section_label(block).is_some()
    {
        return false;
    }

    matches!(
        block.role,
        LiquidBlockRole::Paragraph
            | LiquidBlockRole::ListItem
            | LiquidBlockRole::Takeaway
            | LiquidBlockRole::Explainer
            | LiquidBlockRole::Holding
            | LiquidBlockRole::Issue
            | LiquidBlockRole::Definition
            | LiquidBlockRole::Quote
            | LiquidBlockRole::Syllabus
    )
}

fn reader_aid_parts_are_compact(label: &str, parts: &[(String, LiquidBlockRole)]) -> bool {
    if parts.is_empty() {
        return false;
    }

    let has_list_like_part = parts.iter().any(|(_, role)| {
        matches!(
            role,
            LiquidBlockRole::ListItem
                | LiquidBlockRole::Takeaway
                | LiquidBlockRole::Explainer
                | LiquidBlockRole::Issue
                | LiquidBlockRole::Holding
        )
    });
    if label == "Key findings" && !has_list_like_part {
        return false;
    }

    let total_words = parts
        .iter()
        .map(|(text, _)| word_count(text))
        .sum::<usize>();

    if has_list_like_part {
        return total_words <= 220;
    }

    parts.len() <= 4 && total_words <= 140 && parts.iter().all(|(text, _)| word_count(text) <= 55)
}

fn reader_aid_callout_text(parts: &[(String, LiquidBlockRole)]) -> String {
    if parts.len() == 1 {
        return strip_reader_aid_list_marker(&parts[0].0).to_owned();
    }

    parts
        .iter()
        .map(|(text, _)| format!("- {}", strip_reader_aid_list_marker(text)))
        .collect::<Vec<_>>()
        .join("\n")
}

fn strip_reader_aid_list_marker(text: &str) -> &str {
    let trimmed = text.trim();
    if let Some(rest) = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
        .or_else(|| trimmed.strip_prefix("â€¢ "))
        .or_else(|| trimmed.strip_prefix("• "))
    {
        return rest.trim_start();
    }

    if let Some(rest) = trimmed
        .strip_prefix('(')
        .and_then(|value| value.get(1..))
        .and_then(|value| value.strip_prefix(") "))
    {
        return rest.trim_start();
    }

    let Some(marker_end) = trimmed
        .char_indices()
        .take(8)
        .find_map(|(index, ch)| matches!(ch, '.' | ')').then_some(index + ch.len_utf8()))
    else {
        return trimmed;
    };
    let marker = &trimmed[..marker_end - 1];
    if marker.is_empty() || !marker.chars().all(|ch| ch.is_ascii_digit()) {
        return trimmed;
    }
    trimmed[marker_end..].trim_start()
}

fn standalone_key_term_section_label(block: &LiquidBlock) -> Option<&'static str> {
    if !matches!(
        block.role,
        LiquidBlockRole::Heading | LiquidBlockRole::Subheading
    ) {
        return None;
    }

    match normalize_reference_heading(&block.text).as_str() {
        "key term" | "key terms" | "terms to know" => Some("Key terms"),
        "glossary" => Some("Glossary"),
        "definition" | "definitions" | "defined terms" => Some("Definitions"),
        _ => None,
    }
}

fn key_term_section_has_enough_entries(label: &str, count: usize) -> bool {
    if label == "Definitions" {
        count >= 2
    } else {
        count >= 1
    }
}

fn normalize_local_key_term_sections(blocks: Vec<LiquidBlock>) -> Vec<LiquidBlock> {
    let mut output = Vec::with_capacity(blocks.len());
    let mut index = 0usize;

    while index < blocks.len() {
        let block = &blocks[index];
        if let Some(label) = standalone_key_term_section_label(block) {
            let mut definitions = Vec::new();
            let mut cursor = index + 1;

            while definitions.len() < MAX_KEY_TERM_SECTION_BLOCKS {
                let Some(next) = blocks.get(cursor) else {
                    break;
                };
                let Some(definition) = definition_block_from_key_term_entry(next) else {
                    break;
                };
                definitions.push(definition);
                cursor += 1;
            }

            let overflowed = blocks
                .get(cursor)
                .and_then(definition_block_from_key_term_entry)
                .is_some();
            if !overflowed && key_term_section_has_enough_entries(label, definitions.len()) {
                output.extend(definitions);
                index = cursor;
                continue;
            }
        }

        output.push(block.clone());
        index += 1;
    }

    output
}

fn definition_block_from_key_term_entry(block: &LiquidBlock) -> Option<LiquidBlock> {
    if !matches!(
        block.role,
        LiquidBlockRole::Paragraph | LiquidBlockRole::ListItem | LiquidBlockRole::Definition
    ) {
        return None;
    }
    let text = block.text.trim();
    if text.is_empty()
        || looks_like_front_matter_metadata(text)
        || looks_like_abstract(text)
        || looks_like_toc_entry(text)
        || looks_like_footnote_line(text)
        || looks_like_caption(text, 0)
        || looks_like_reference_entry(text)
    {
        return None;
    }

    let normalized = strip_reader_aid_list_marker(text).trim();
    let (term, body) = split_key_term_definition(normalized)?;
    if !looks_like_key_term(term) || !looks_like_key_term_definition_body(body) {
        return None;
    }

    Some(LiquidBlock {
        role: LiquidBlockRole::Definition,
        text: format!("{}: {}", term.trim(), body.trim()),
        label: Some(term.trim().to_owned()),
    })
}

fn split_key_term_definition(text: &str) -> Option<(&str, &str)> {
    text.split_once(':')
        .or_else(|| text.split_once(" - "))
        .map(|(term, body)| (term.trim(), body.trim()))
        .filter(|(term, body)| !term.is_empty() && !body.is_empty())
}

fn looks_like_key_term(term: &str) -> bool {
    let trimmed = term.trim();
    let words = word_count(trimmed);
    (1..=8).contains(&words)
        && trimmed.chars().count() <= 80
        && trimmed.chars().any(char::is_alphabetic)
        && !trimmed.ends_with('.')
        && !looks_like_front_matter_metadata(trimmed)
        && !is_non_title_heading_text(trimmed)
}

fn looks_like_key_term_definition_body(body: &str) -> bool {
    let words = word_count(body);
    (4..=80).contains(&words)
        && body.chars().any(char::is_alphabetic)
        && !looks_like_toc_entry(body)
        && !looks_like_footnote_line(body)
}

fn has_adjacent_visual_or_table_caption(caption_neighbors: &[bool], index: usize) -> bool {
    (1..=2).any(|offset| {
        index
            .checked_sub(offset)
            .and_then(|nearby| caption_neighbors.get(nearby))
            .copied()
            .unwrap_or(false)
            || caption_neighbors
                .get(index + offset)
                .copied()
                .unwrap_or(false)
    })
}

fn is_visual_or_table_caption_block(block: &LiquidBlock) -> bool {
    if block.role != LiquidBlockRole::Caption {
        return false;
    }
    matches!(
        block.label.as_deref(),
        Some("Figure")
            | Some("Table")
            | Some("Chart")
            | Some("Map")
            | Some("Photo")
            | Some("Caption")
    )
}

pub(super) fn is_source_or_credit_line(text: &str) -> bool {
    let lower = text.trim_start().to_ascii_lowercase();
    lower.starts_with("source:")
        || lower.starts_with("sources:")
        || lower.starts_with("credit:")
        || lower.starts_with("credits:")
}

fn normalize_local_caption_source_lines(blocks: Vec<LiquidBlock>) -> Vec<LiquidBlock> {
    let caption_neighbors = blocks
        .iter()
        .map(is_visual_or_table_caption_block)
        .collect::<Vec<_>>();

    blocks
        .into_iter()
        .enumerate()
        .map(|(index, mut block)| {
            if is_source_or_credit_line(&block.text) {
                if has_adjacent_visual_or_table_caption(&caption_neighbors, index) {
                    block.role = LiquidBlockRole::Caption;
                    block.label = Some("Source".to_owned());
                } else {
                    block.role = LiquidBlockRole::Metadata;
                    block.label = None;
                }
            }
            block
        })
        .collect()
}

fn normalize_local_tables(blocks: Vec<LiquidBlock>) -> Vec<LiquidBlock> {
    blocks
        .into_iter()
        .map(|mut block| {
            if matches!(
                block.role,
                LiquidBlockRole::Paragraph
                    | LiquidBlockRole::ListItem
                    | LiquidBlockRole::Marginalia
            ) && looks_like_dense_table_block(&block.text)
            {
                block.role = LiquidBlockRole::Table;
                block.label = None;
            }
            block
        })
        .collect()
}

fn looks_like_dense_table_block(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.len() < 40 || trimmed.len() > 3_500 || word_count(trimmed) > 420 {
        return false;
    }
    if looks_like_footnote_line(trimmed) || looks_like_caption(trimmed, 0) {
        return false;
    }

    let lines = trimmed
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines.len() < 3 || lines.len() > 80 {
        return false;
    }

    let structured = lines
        .iter()
        .filter(|line| looks_like_table_row_line(line))
        .count();
    structured >= 3 && structured * 2 >= lines.len()
}

fn looks_like_table_row_line(line: &str) -> bool {
    if line.len() > 220 || word_count(line) > 28 {
        return false;
    }
    let lower = line.to_ascii_lowercase();
    if line.contains('\t') || line.contains('|') || line.matches("  ").count() >= 2 {
        return true;
    }
    if line.contains('$') && line.chars().any(|ch| ch.is_ascii_digit()) {
        return true;
    }
    if line.contains(':') {
        let Some((key, value)) = line.split_once(':') else {
            return false;
        };
        let key = key.trim();
        let value = value.trim();
        return !key.is_empty()
            && !value.is_empty()
            && word_count(key) <= 6
            && value
                .chars()
                .any(|ch| ch.is_ascii_digit() || ch.is_ascii_alphabetic());
    }
    lower.contains(" total")
        || lower.starts_with("total ")
        || lower.contains(" subtotal")
        || lower.starts_with("subtotal ")
        || lower.contains(" amount ")
        || lower.contains(" balance ")
}

/// Applies generalized inline footnote marker cleanup (via clean_body_text_...)
/// to Paragraph/Lead/Quote roles early in the pipeline. Skips citation-heavy,
/// heading-like, and other guarded blocks entirely. Placed before the legacy
/// fragment demotion pass.
fn clean_inline_footnote_markers_in_body_text(blocks: Vec<LiquidBlock>) -> Vec<LiquidBlock> {
    blocks
        .into_iter()
        .map(|mut block| {
            if matches!(
                block.role,
                LiquidBlockRole::Paragraph | LiquidBlockRole::Lead | LiquidBlockRole::Quote
            ) && !block.text.trim().is_empty()
                && !should_skip_inline_marker_cleanup(&block.text)
            {
                block.text = clean_body_text_inline_footnote_markers(&block.text);
            }
            block
        })
        .collect()
}

fn normalize_inline_footnote_reference_fragments(blocks: Vec<LiquidBlock>) -> Vec<LiquidBlock> {
    let mut output = Vec::with_capacity(blocks.len());
    let mut before_footnotes = true;
    let mut previous_main_flow = false;

    for mut block in blocks {
        if matches!(
            block.role,
            LiquidBlockRole::Footnote | LiquidBlockRole::Footer
        ) {
            before_footnotes = false;
        }

        if before_footnotes
            && previous_main_flow
            && block.role == LiquidBlockRole::ListItem
            && let Some(body) = inline_footnote_reference_fragment_body(&block.text)
        {
            block.role = LiquidBlockRole::Paragraph;
            block.label = None;
            block.text = strip_embedded_inline_note_markers(body);
        }

        previous_main_flow = matches!(
            block.role,
            LiquidBlockRole::Paragraph | LiquidBlockRole::Lead | LiquidBlockRole::ListItem
        );
        output.push(block);
    }

    output
}

fn inline_footnote_reference_fragment_body(text: &str) -> Option<&str> {
    let trimmed = text.trim_start();
    let (marker, body) = split_note_marker(trimmed);
    let marker = marker?;
    if marker.len() > 2 || !marker.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    if marker.parse::<usize>().ok()? < 3 {
        return None;
    }
    if trimmed[marker.len()..]
        .chars()
        .next()
        .is_some_and(|ch| matches!(ch, '.' | ')' | ']'))
    {
        return None;
    }
    if word_count(body) < 2 || !starts_with_prose_continuation(body) {
        return None;
    }

    Some(body)
}

fn starts_with_prose_continuation(text: &str) -> bool {
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
        "see ", "see, ", "cf. ", "accord ", "but see ", "id. ", "supra ", "infra ",
    ]
    .iter()
    .any(|prefix| lower.starts_with(prefix))
    {
        return false;
    }

    word_count(trimmed) >= 12
        || contains_embedded_inline_note_marker(trimmed)
        || [
            "and ",
            "but ",
            "of course,",
            "obviously,",
            "recycling ",
            "so ",
            "still,",
            "sure,",
            "you ",
        ]
        .iter()
        .any(|prefix| lower.starts_with(prefix))
}

fn strip_embedded_inline_note_markers(text: &str) -> String {
    // Delegated to generalized implementation (covers legacy post-punct + new
    // word-adjacent 1-3 digit after letters + space-bounded prose digits, with
    // full guard set). This extends the strip logic for Task 2.
    clean_body_text_inline_footnote_markers(text)
}

fn contains_embedded_inline_note_marker(text: &str) -> bool {
    // Delegated to broadened detector (supports fragment prose continuation
    // heuristic for both legacy and new marker forms).
    contains_body_text_inline_footnote_marker(text)
}

/// Returns true for blocks that should be entirely skipped for inline footnote
/// marker cleanup (strong citation/structural guards).
fn should_skip_inline_marker_cleanup(text: &str) -> bool {
    if text.len() < 4 {
        return true;
    }
    let lower = text.to_ascii_lowercase();
    if contains_reference_year(text) {
        return true;
    }
    if looks_like_citation_footnote_line(text) {
        return true;
    }
    if looks_like_reference_entry(text) {
        return true;
    }
    if looks_like_footnote_line(text) {
        return true;
    }
    if lower.contains(" v. ")
        || lower.contains(" v ")
        || lower.contains("u.s.")
        || lower.contains("f.3d")
        || lower.contains("f.2d")
        || lower.contains("§")
        || lower.contains(" no.")
        || lower.contains(" no ")
        || lower.contains(" id.")
        || lower.contains("supra")
        || lower.contains("infra")
        || lower.contains(" et al.")
    {
        return true;
    }
    if lower.contains("fig.")
        || lower.contains("tbl.")
        || lower.contains(" p.")
        || lower.contains(" pp.")
        || lower.contains("vol.")
        || lower.contains("ch.")
    {
        return true;
    }
    // Conservative decimal/version guard via local patterns
    let cs: Vec<char> = text.chars().collect();
    for window in cs.windows(3) {
        if window[0].is_ascii_digit() && window[1] == '.' && window[2].is_ascii_digit() {
            return true;
        }
    }
    false
}

/// Core conservative predicate: is this 1-3 digit run a likely inline footnote marker
/// (word-adjacent after letter, post-punct, or space-bounded in prose) given strong
/// local context guards. If any doubt, return false (leave digits).
fn is_likely_inline_footnote_marker(text: &str, start: usize, end: usize) -> bool {
    let len = end - start;
    if len == 0 || len > 3 {
        return false;
    }
    let digits = &text[start..end];
    if digits == "0" || digits == "00" || digits == "000" {
        return false;
    }
    let before = &text[..start];
    let after_full = &text[end..];
    let lower_before = before.to_ascii_lowercase();
    let lower_after = after_full.to_ascii_lowercase().trim_start().to_owned();
    let last_ch = before.chars().last();
    let preceded_by_letter = last_ch.is_some_and(|c| c.is_alphabetic());
    let last = last_ch.unwrap_or('\0');
    let is_marker_punct = last == '.'
        || last == '?'
        || last == '!'
        || last == ')'
        || last == ']'
        || last == '"'
        || last == '\u{201d}'
        || last == '“'
        || last == '”';
    let preceded_by_punct = is_marker_punct || last == ',';
    let preceded_by_boundary = last.is_whitespace()
        || preceded_by_punct
        || last == '('
        || last == ':'
        || last == ';'
        || last == '—'
        || last == '–'
        || last == '-'
        || last == '§'
        || last == '¶';
    let followed_by_boundary =
        after_full.chars().next().is_some_and(|c| {
            c.is_whitespace() || c.is_ascii_uppercase() || c.is_ascii_punctuation()
        }) || after_full.trim().is_empty();

    let is_attached = preceded_by_letter;
    let is_post_punct = preceded_by_punct;
    let is_bounded = !preceded_by_letter && preceded_by_boundary && followed_by_boundary;
    if !(is_attached || is_post_punct || is_bounded) {
        return false;
    }

    // Strong local prefix guards (reuse abbrev/fig/tbl/no/§/id/v./citation patterns + prose number contexts)
    let suffix = lower_before
        .chars()
        .rev()
        .take(10)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    if suffix.contains("fig")
        || suffix.contains("tbl")
        || suffix.contains(" p.")
        || suffix.contains("pp.")
        || suffix.contains("no.")
        || suffix.contains("§")
        || suffix.contains(" id.")
        || suffix.contains(" v.")
        || suffix.contains("u.s.")
        || suffix.contains("f.3")
        || suffix.contains("f.2")
        || suffix.contains("rule ")
        || suffix.contains("sec.")
        || suffix.contains("sec ")
        || suffix.contains("art.")
        || suffix.contains("ch.")
        || suffix.contains("para")
        || suffix.contains("page ")
        || suffix.contains("ex.")
        || suffix.contains("step ")
        || suffix.contains("count ")
        || suffix.contains("item ")
        || suffix.contains("point ")
        || suffix.contains("example")
        || suffix.contains("note ")
        || suffix.contains("see ")
        || suffix.contains("case ")
        || suffix.contains("cases ")
        || suffix.contains("matter ")
    {
        return false;
    }
    // v/rev/version contexts for attached (e.g. "v2", "rev3")
    if suffix.ends_with('v')
        || suffix.contains(" rev")
        || suffix.contains("ver ")
        || suffix.contains(" v.")
    {
        return false;
    }
    // Ordinal / unit / scale suffixes after the digit run (leave real numbers)
    if lower_after.starts_with("st ")
        || lower_after.starts_with("nd ")
        || lower_after.starts_with("rd ")
        || lower_after.starts_with("th ")
        || lower_after.starts_with("%")
        || lower_after.starts_with(" percent")
        || lower_after.starts_with(" times")
        || lower_after.starts_with(" million")
        || lower_after.starts_with(" billion")
        || lower_after.starts_with("d ")
    {
        return false;
    }
    // Decimal fragment guard (e.g. "3." of "3.14")
    if after_full.starts_with('.') {
        return false;
    }
    // Trailing digit (part of longer run already filtered by caller, but belt-and-suspenders)
    if after_full
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_digit())
    {
        return false;
    }
    true
}

/// Generalized cleanup for body text (Paragraph/Lead/Quote). Handles original
/// post-punct cases plus word-adjacent lone 1-3 digits after letters (e.g. "doctrine14")
/// and space/punct-bounded lone digits in prose (e.g. "held 3 that").
/// Conservative: skips whole blocks via guards; per-occurrence if doubt, leaves digits.
fn clean_body_text_inline_footnote_markers(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    if should_skip_inline_marker_cleanup(text) {
        return text.to_owned();
    }
    let mut output = String::with_capacity(text.len());
    let char_indices: Vec<(usize, char)> = text.char_indices().collect();
    let mut idx = 0usize;
    while idx < char_indices.len() {
        let (byte_pos, ch) = char_indices[idx];
        if ch.is_ascii_digit() {
            // Measure full consecutive digit run
            let mut run_end = idx;
            while run_end < char_indices.len() && char_indices[run_end].1.is_ascii_digit() {
                run_end += 1;
            }
            let full_len = run_end - idx;
            if (1..=3).contains(&full_len) {
                let end_byte = if run_end < char_indices.len() {
                    char_indices[run_end].0
                } else {
                    text.len()
                };
                if is_likely_inline_footnote_marker(text, byte_pos, end_byte) {
                    if inline_marker_needs_boundary_space(text, byte_pos, end_byte, &output) {
                        output.push(' ');
                    }
                    idx = run_end;
                    continue; // drop the marker digits
                }
            }
            output.push(ch);
            idx += 1;
            continue;
        }
        output.push(ch);
        idx += 1;
    }
    output
}

fn inline_marker_needs_boundary_space(text: &str, start: usize, end: usize, output: &str) -> bool {
    if output.chars().last().is_none_or(|ch| ch.is_whitespace()) {
        return false;
    }
    let previous = text[..start].chars().last();
    if !previous.is_some_and(|ch| matches!(ch, '.' | '?' | '!' | ')' | ']' | '"' | '\u{201d}')) {
        return false;
    }
    text[end..]
        .chars()
        .next()
        .is_some_and(|ch| ch.is_alphabetic() || matches!(ch, '"' | '\u{201c}' | '\'' | '\u{2018}'))
}

/// Updated contains that detects both legacy and broadened marker patterns
/// (used by prose continuation heuristic for fragment demotion).
fn contains_body_text_inline_footnote_marker(text: &str) -> bool {
    if text.len() < 4 || should_skip_inline_marker_cleanup(text) {
        return false;
    }
    let char_indices: Vec<(usize, char)> = text.char_indices().collect();
    let mut idx = 0usize;
    while idx < char_indices.len() {
        let (byte_pos, ch) = char_indices[idx];
        if ch.is_ascii_digit() {
            let mut run_end = idx;
            while run_end < char_indices.len() && char_indices[run_end].1.is_ascii_digit() {
                run_end += 1;
            }
            let full_len = run_end - idx;
            if (1..=3).contains(&full_len) {
                let end_byte = if run_end < char_indices.len() {
                    char_indices[run_end].0
                } else {
                    text.len()
                };
                if is_likely_inline_footnote_marker(text, byte_pos, end_byte) {
                    return true;
                }
            }
            idx = run_end;
            continue;
        }
        idx += 1;
    }
    false
}

fn standalone_front_matter_metadata_label(block: &LiquidBlock) -> Option<&'static str> {
    if !matches!(
        block.role,
        LiquidBlockRole::Heading | LiquidBlockRole::Subheading | LiquidBlockRole::Metadata
    ) {
        return None;
    }
    front_matter_label_for_text(&block.text)
}

fn can_fold_front_matter_metadata_body(label: &str, block: &LiquidBlock) -> bool {
    matches!(
        block.role,
        LiquidBlockRole::Paragraph
            | LiquidBlockRole::AuthorInfo
            | LiquidBlockRole::ListItem
            | LiquidBlockRole::Metadata
            | LiquidBlockRole::Heading
            | LiquidBlockRole::Subheading
    ) && !block.text.trim().is_empty()
        && (!looks_like_author_info(&block.text, 0)
            || can_fold_author_like_front_matter_body(label, &block.text))
        && !looks_like_abstract(&block.text)
        && (!looks_like_toc_entry(&block.text)
            || can_fold_toc_like_front_matter_body(label, &block.text))
        && (!looks_like_footnote_line(&block.text)
            || can_fold_footnote_like_front_matter_body(label, &block.text))
        && !starts_with_roman_heading(&block.text)
        && !starts_with_lettered_heading(&block.text)
        && !starts_with_numbered_heading(&block.text)
        && !is_non_title_heading_text(&block.text)
        && front_matter_label_for_text(&block.text).is_none()
}

fn can_fold_author_like_front_matter_body(label: &str, text: &str) -> bool {
    label == "ORCID" && text.to_ascii_lowercase().contains("orcid")
}

fn can_fold_footnote_like_front_matter_body(label: &str, text: &str) -> bool {
    label == "DOI" && text.trim_start().starts_with("10.")
}

fn can_fold_toc_like_front_matter_body(label: &str, text: &str) -> bool {
    label == "Article history" && looks_like_front_matter_metadata(text)
}

fn front_matter_label_allows_multiple_bodies(label: &str) -> bool {
    matches!(label, "Article history")
}

fn front_matter_metadata_text(label: &str, parts: &[String]) -> String {
    if parts.len() == 1 {
        let text = parts[0].trim();
        if front_matter_body_has_label(text, label) {
            text.to_owned()
        } else {
            format!("{label}: {text}")
        }
    } else {
        let body = parts
            .iter()
            .map(|part| part.trim())
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("; ");
        format!("{label}: {body}")
    }
}

fn front_matter_body_has_label(text: &str, label: &str) -> bool {
    normalize_reference_heading(text).starts_with(&normalize_reference_heading(label))
}

pub(super) fn collapse_local_table_of_contents_sections(
    blocks: Vec<LiquidBlock>,
) -> Vec<LiquidBlock> {
    let mut output = Vec::with_capacity(blocks.len());
    let mut index = 0usize;

    while index < blocks.len() {
        if let Some(end) = table_of_contents_section_end(&blocks, index) {
            for mut block in blocks[index..end].iter().cloned() {
                block.role = LiquidBlockRole::Contents;
                block.label = None;
                output.push(block);
            }
            index = end;
            continue;
        }

        output.push(blocks[index].clone());
        index += 1;
    }

    output
}

pub(super) fn table_of_contents_section_mask(blocks: &[LiquidBlock]) -> Vec<bool> {
    let mut mask = vec![false; blocks.len()];
    let mut index = 0usize;

    while index < blocks.len() {
        if let Some(end) = table_of_contents_section_end(blocks, index) {
            for hidden in index..end {
                mask[hidden] = true;
            }
            index = end;
            continue;
        }

        index += 1;
    }

    mask
}

fn table_of_contents_section_end(blocks: &[LiquidBlock], heading_index: usize) -> Option<usize> {
    let heading = blocks.get(heading_index)?;
    let web_navigation = is_web_navigation_contents_heading(heading);
    if !is_table_of_contents_heading(heading) {
        return None;
    }
    let preceding_body_blocks = preceding_body_block_count(blocks, heading_index);
    if preceding_body_blocks > 0
        && !table_of_contents_may_follow_front_matter(blocks, heading_index)
    {
        return None;
    }

    let mut end = heading_index + 1;
    let mut entries = 0usize;
    let mut split_entries = 0usize;
    let mut plain_entries = 0usize;
    let mut entry_keys = Vec::new();
    while end < blocks.len() {
        let block = &blocks[end];
        if entries + split_entries + plain_entries >= 2
            && (toc_entry_key_repeats(block, &entry_keys)
                || repeats_prior_toc_entry_before_body(blocks, end, &entry_keys))
        {
            break;
        }
        if looks_like_toc_entry(&block.text)
            || (web_navigation && looks_like_web_navigation_entry(block))
        {
            entries += 1;
            push_toc_entry_key(&mut entry_keys, &block.text);
            end += 1;
            continue;
        }
        if let Some(split_end) = split_toc_entry_end(blocks, end) {
            split_entries += 1;
            push_toc_entry_key(&mut entry_keys, &blocks[end].text);
            end = split_end;
            continue;
        }
        if entries + split_entries > 0
            && looks_like_toc_group_heading(block)
            && next_block_can_continue_toc(blocks, end + 1)
        {
            end += 1;
            continue;
        }
        if entries + split_entries >= 2 && looks_like_plain_toc_outline_entry(block) {
            break;
        }
        if looks_like_plain_toc_outline_entry(block) {
            plain_entries += 1;
            push_toc_entry_key(&mut entry_keys, &block.text);
            end += 1;
            continue;
        }
        if entries == 0
            && split_entries == 0
            && matches!(
                block.role,
                LiquidBlockRole::AuthorInfo | LiquidBlockRole::Metadata
            )
        {
            end += 1;
            continue;
        }
        break;
    }

    let paged_entries = entries + split_entries;
    (paged_entries >= 2
        || (paged_entries == 0 && plain_entries >= 2)
        || paged_entries + plain_entries >= 3)
        .then_some(end)
}

fn split_toc_entry_end(blocks: &[LiquidBlock], index: usize) -> Option<usize> {
    let title = blocks.get(index)?;
    if !looks_like_split_toc_title_entry(title) {
        return None;
    }
    let page_index = index + 1;
    let page = blocks.get(page_index)?;
    looks_like_standalone_toc_page_locator(page).then_some(page_index + 1)
}

fn next_block_can_continue_toc(blocks: &[LiquidBlock], index: usize) -> bool {
    blocks
        .get(index)
        .is_some_and(|block| looks_like_toc_entry(&block.text))
        || split_toc_entry_end(blocks, index).is_some()
        || blocks
            .get(index)
            .is_some_and(looks_like_plain_toc_outline_entry)
}

fn looks_like_plain_toc_outline_entry(block: &LiquidBlock) -> bool {
    if !matches!(
        block.role,
        LiquidBlockRole::Title
            | LiquidBlockRole::Heading
            | LiquidBlockRole::Subheading
            | LiquidBlockRole::Paragraph
            | LiquidBlockRole::ListItem
            | LiquidBlockRole::Table
            | LiquidBlockRole::Metadata
    ) {
        return false;
    }

    let text = block.text.trim();
    if text.len() < 3
        || text.len() > 140
        || !text.chars().any(char::is_alphabetic)
        || text.ends_with(['.', '?', '!', '"', '\u{201d}'])
    {
        return false;
    }
    let words = word_count(text);
    if words == 0 || words > 16 {
        return false;
    }
    if looks_like_front_matter_metadata(text)
        || looks_like_abstract(text)
        || looks_like_footnote_line(text)
        || looks_like_caption(text, 0)
        || end_matter_label(text).is_some()
        || is_reference_section_heading(block)
    {
        return false;
    }

    starts_with_roman_heading(text)
        || starts_with_numbered_heading(text)
        || starts_with_lettered_heading(text)
        || title_case_ratio(text) > 0.42
        || uppercase_ratio(text) > 0.72
}

fn repeats_prior_toc_entry_before_body(
    blocks: &[LiquidBlock],
    index: usize,
    entry_keys: &[String],
) -> bool {
    let Some(block) = blocks.get(index) else {
        return false;
    };
    if !looks_like_plain_toc_outline_entry(block) && !looks_like_toc_entry(&block.text) {
        return false;
    }
    let key = toc_entry_title_key(&block.text);
    !key.is_empty()
        && entry_keys.iter().any(|entry| entry == &key)
        && next_non_section_index(blocks, index + 1)
            .and_then(|next| blocks.get(next))
            .is_some_and(looks_like_toc_body_start)
}

fn toc_entry_key_repeats(block: &LiquidBlock, entry_keys: &[String]) -> bool {
    if !looks_like_plain_toc_outline_entry(block) && !looks_like_toc_entry(&block.text) {
        return false;
    }
    let key = toc_entry_title_key(&block.text);
    !key.is_empty() && entry_keys.iter().any(|entry| entry == &key)
}

fn looks_like_toc_body_start(block: &LiquidBlock) -> bool {
    if !matches!(
        block.role,
        LiquidBlockRole::Paragraph
            | LiquidBlockRole::Lead
            | LiquidBlockRole::Abstract
            | LiquidBlockRole::Quote
            | LiquidBlockRole::Explainer
            | LiquidBlockRole::Takeaway
            | LiquidBlockRole::Holding
            | LiquidBlockRole::Issue
    ) {
        return false;
    }
    let text = block.text.trim();
    word_count(text) >= 8 || text.ends_with(['.', '?', '!', '"', '\u{201d}'])
}

fn push_toc_entry_key(keys: &mut Vec<String>, text: &str) {
    let key = toc_entry_title_key(text);
    if !key.is_empty() && !keys.iter().any(|existing| existing == &key) {
        keys.push(key);
    }
}

fn toc_entry_title_key(text: &str) -> String {
    let mut title = text
        .split(['\u{2026}'])
        .next()
        .unwrap_or(text)
        .split("...")
        .next()
        .unwrap_or(text)
        .trim()
        .to_owned();

    if let Some((prefix, rest)) = title.rsplit_once(char::is_whitespace) {
        let locator = rest.trim_matches(|ch: char| matches!(ch, '.' | ',' | ';' | ':' | '(' | ')'));
        if is_toc_page_locator_token(locator) {
            title = prefix.trim().to_owned();
        }
    }

    let trimmed = title.trim();
    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let first = parts.next().unwrap_or_default();
    let rest = parts.next().unwrap_or_default().trim();
    let marker = first.trim_matches(|ch: char| matches!(ch, '.' | ')' | '('));
    let marker_like = !rest.is_empty()
        && (marker.chars().all(|ch| ch.is_ascii_digit())
            || marker.len() == 1 && marker.chars().all(|ch| ch.is_ascii_alphabetic())
            || marker
                .chars()
                .all(|ch| matches!(ch.to_ascii_uppercase(), 'I' | 'V' | 'X')));
    if marker_like {
        title = rest.to_owned();
    }

    normalize_reference_heading(&title)
}

fn looks_like_split_toc_title_entry(block: &LiquidBlock) -> bool {
    if !matches!(
        block.role,
        LiquidBlockRole::Title
            | LiquidBlockRole::Heading
            | LiquidBlockRole::Subheading
            | LiquidBlockRole::Paragraph
            | LiquidBlockRole::ListItem
            | LiquidBlockRole::Table
            | LiquidBlockRole::Metadata
    ) {
        return false;
    }

    let text = block.text.trim();
    if text.len() < 3
        || text.len() > 140
        || text.ends_with('.')
        || !text.chars().any(char::is_alphabetic)
    {
        return false;
    }
    let words = word_count(text);
    if words > 16 {
        return false;
    }
    if looks_like_front_matter_metadata(text)
        || looks_like_abstract(text)
        || looks_like_footnote_line(text)
        || looks_like_caption(text, 0)
        || end_matter_label(text).is_some()
        || is_reference_section_heading(block)
    {
        return false;
    }

    starts_with_roman_heading(text)
        || starts_with_numbered_heading(text)
        || starts_with_lettered_heading(text)
        || title_case_ratio(text) > 0.42
}

fn looks_like_standalone_toc_page_locator(block: &LiquidBlock) -> bool {
    if matches!(
        block.role,
        LiquidBlockRole::Marginalia | LiquidBlockRole::Footnote | LiquidBlockRole::Caption
    ) {
        return false;
    }
    is_toc_page_locator_token(block.text.trim())
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

fn looks_like_toc_group_heading(block: &LiquidBlock) -> bool {
    if !matches!(
        block.role,
        LiquidBlockRole::Heading
            | LiquidBlockRole::Subheading
            | LiquidBlockRole::Paragraph
            | LiquidBlockRole::Table
            | LiquidBlockRole::Metadata
    ) {
        return false;
    }
    let normalized = normalize_reference_heading(&block.text);
    matches!(
        normalized.as_str(),
        "articles"
            | "essays"
            | "notes"
            | "comments"
            | "book reviews"
            | "symposium"
            | "foreword"
            | "introduction"
            | "preface"
    )
}

fn table_of_contents_may_follow_front_matter(blocks: &[LiquidBlock], heading_index: usize) -> bool {
    let body_blocks = blocks[..heading_index]
        .iter()
        .filter(|block| {
            matches!(
                block.role,
                LiquidBlockRole::Paragraph
                    | LiquidBlockRole::Lead
                    | LiquidBlockRole::Abstract
                    | LiquidBlockRole::Quote
                    | LiquidBlockRole::Explainer
                    | LiquidBlockRole::Takeaway
                    | LiquidBlockRole::Holding
                    | LiquidBlockRole::Issue
            )
        })
        .collect::<Vec<_>>();

    !body_blocks.is_empty()
        && body_blocks.len() <= 2
        && body_blocks
            .iter()
            .all(|block| block.role == LiquidBlockRole::Abstract)
}

fn is_table_of_contents_heading(block: &LiquidBlock) -> bool {
    if is_web_navigation_contents_heading(block) {
        return true;
    }

    let normalized = normalize_reference_heading(&block.text);
    is_table_of_contents_heading_text(&normalized)
        && !matches!(
            block.role,
            LiquidBlockRole::Marginalia
                | LiquidBlockRole::Footnote
                | LiquidBlockRole::Caption
                | LiquidBlockRole::Header
                | LiquidBlockRole::Footer
                | LiquidBlockRole::Noise
                | LiquidBlockRole::SectionBreak
        )
}

fn is_table_of_contents_heading_text(normalized: &str) -> bool {
    matches!(
        normalized,
        "contents"
            | "table of contents"
            | "brief contents"
            | "brief table of contents"
            | "contents of this article"
            | "contents of the article"
            | "article outline"
    ) || normalized.starts_with("table of contents ")
        || normalized.starts_with("contents ")
}

fn is_web_navigation_contents_heading(block: &LiquidBlock) -> bool {
    matches!(
        normalize_reference_heading(&block.text).as_str(),
        "in this article"
            | "on this page"
            | "article contents"
            | "article navigation"
            | "jump to"
            | "jump to section"
            | "jump to sections"
    )
}

fn looks_like_web_navigation_entry(block: &LiquidBlock) -> bool {
    if !matches!(
        block.role,
        LiquidBlockRole::Heading
            | LiquidBlockRole::Subheading
            | LiquidBlockRole::Paragraph
            | LiquidBlockRole::Explainer
            | LiquidBlockRole::Takeaway
            | LiquidBlockRole::Issue
    ) {
        return false;
    }

    let trimmed = block.text.trim();
    trimmed.len() <= 120
        && (1..=12).contains(&word_count(trimmed))
        && !trimmed.ends_with('.')
        && trimmed.chars().any(char::is_alphabetic)
        && !looks_like_front_matter_metadata(trimmed)
        && !looks_like_abstract(trimmed)
        && !looks_like_footnote_line(trimmed)
        && end_matter_label(trimmed).is_none()
        && !is_reference_section_heading(block)
}

fn collapse_local_end_matter_sections(blocks: Vec<LiquidBlock>) -> Vec<LiquidBlock> {
    let mut output = Vec::with_capacity(blocks.len());
    let mut index = 0usize;

    while index < blocks.len() {
        if let Some(end) = end_matter_section_end(&blocks, index) {
            let label = end_matter_label(&blocks[index].text).unwrap_or("Note");
            for mut block in blocks[index + 1..end].iter().cloned() {
                if can_fold_end_matter_block(label, &block) {
                    block.role = LiquidBlockRole::Footnote;
                    block.text = prefix_end_matter_note(label, &block.text);
                    block.label = None;
                    output.push(block);
                }
            }
            index = end;
            continue;
        }

        output.push(blocks[index].clone());
        index += 1;
    }

    output
}

fn end_matter_section_end(blocks: &[LiquidBlock], heading_index: usize) -> Option<usize> {
    let heading = blocks.get(heading_index)?;
    let label = end_matter_label(&heading.text)?;
    if !is_end_matter_section_heading(heading)
        || preceding_body_block_count(blocks, heading_index) < 2
    {
        return None;
    }

    let mut end = heading_index + 1;
    let mut convertible = 0usize;
    while end < blocks.len() {
        let block = &blocks[end];
        if matches!(
            block.role,
            LiquidBlockRole::Heading | LiquidBlockRole::Subheading | LiquidBlockRole::Title
        ) {
            if !can_fold_end_matter_block(label, block) {
                break;
            }
        }
        if can_fold_end_matter_block(label, block) {
            convertible += 1;
        }
        end += 1;
    }

    (convertible > 0).then_some(end)
}

fn is_end_matter_section_heading(block: &LiquidBlock) -> bool {
    if !matches!(
        block.role,
        LiquidBlockRole::Heading | LiquidBlockRole::Subheading
    ) {
        return false;
    }
    end_matter_label(&block.text).is_some()
}

fn can_fold_end_matter_block(label: &str, block: &LiquidBlock) -> bool {
    if matches!(
        block.role,
        LiquidBlockRole::Paragraph
            | LiquidBlockRole::ListItem
            | LiquidBlockRole::Footnote
            | LiquidBlockRole::Clause
            | LiquidBlockRole::Metadata
            | LiquidBlockRole::AuthorInfo
    ) {
        return true;
    }

    is_author_bio_label(label)
        && matches!(
            block.role,
            LiquidBlockRole::Heading | LiquidBlockRole::Subheading
        )
        && looks_like_author_bio_name_heading(&block.text)
        || is_further_reading_label(label)
            && matches!(
                block.role,
                LiquidBlockRole::Heading | LiquidBlockRole::Subheading
            )
            && looks_like_related_link_title(&block.text)
}

fn is_author_bio_label(label: &str) -> bool {
    label == "About the author"
}

fn is_further_reading_label(label: &str) -> bool {
    label == "Further reading"
}

fn looks_like_author_bio_name_heading(text: &str) -> bool {
    looks_like_standalone_author_line(text, 0)
}

fn looks_like_related_link_title(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed.len() <= 140
        && word_count(trimmed) <= 18
        && trimmed.chars().any(char::is_alphabetic)
        && !matches!(
            normalize_reference_heading(trimmed).as_str(),
            "introduction"
                | "background"
                | "overview"
                | "analysis"
                | "discussion"
                | "conclusion"
                | "conclusions"
                | "methods"
                | "results"
                | "findings"
        )
}

fn prefix_end_matter_note(label: &str, text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() || front_matter_body_has_label(trimmed, label) {
        trimmed.to_owned()
    } else {
        format!("{label}: {trimmed}")
    }
}

fn collapse_local_reference_sections(blocks: Vec<LiquidBlock>) -> Vec<LiquidBlock> {
    let mut output = Vec::with_capacity(blocks.len());
    let mut index = 0usize;

    while index < blocks.len() {
        if let Some(end) = reference_section_end(&blocks, index) {
            for mut block in blocks[index + 1..end].iter().cloned() {
                if matches!(
                    block.role,
                    LiquidBlockRole::Paragraph
                        | LiquidBlockRole::ListItem
                        | LiquidBlockRole::Footnote
                        | LiquidBlockRole::Clause
                        | LiquidBlockRole::Metadata
                ) {
                    block.role = LiquidBlockRole::Footnote;
                    block.label = None;
                    output.push(block);
                }
            }
            index = end;
            continue;
        }

        output.push(blocks[index].clone());
        index += 1;
    }

    output
}

fn reference_section_end(blocks: &[LiquidBlock], heading_index: usize) -> Option<usize> {
    let heading = blocks.get(heading_index)?;
    if !is_reference_section_heading(heading)
        || preceding_body_block_count(blocks, heading_index) < 2
    {
        return None;
    }

    let mut end = heading_index + 1;
    while end < blocks.len()
        && !matches!(
            blocks[end].role,
            LiquidBlockRole::Heading | LiquidBlockRole::Subheading | LiquidBlockRole::Title
        )
    {
        end += 1;
    }
    if end <= heading_index + 1 || end < blocks.len() {
        return None;
    }

    let entries = &blocks[heading_index + 1..end];
    let convertible = entries
        .iter()
        .filter(|block| {
            matches!(
                block.role,
                LiquidBlockRole::Paragraph
                    | LiquidBlockRole::ListItem
                    | LiquidBlockRole::Footnote
                    | LiquidBlockRole::Clause
                    | LiquidBlockRole::Metadata
            )
        })
        .count();
    let reference_like = entries
        .iter()
        .filter(|block| looks_like_reference_entry(&block.text))
        .count();

    (convertible > 0 && reference_like >= 2).then_some(end)
}

fn is_reference_section_heading(block: &LiquidBlock) -> bool {
    if !matches!(
        block.role,
        LiquidBlockRole::Heading | LiquidBlockRole::Subheading
    ) {
        return false;
    }
    matches!(
        normalize_reference_heading(&block.text).as_str(),
        "references" | "bibliography" | "works cited" | "reference list" | "notes" | "endnotes"
    )
}

fn preceding_body_block_count(blocks: &[LiquidBlock], end: usize) -> usize {
    blocks[..end]
        .iter()
        .filter(|block| {
            matches!(
                block.role,
                LiquidBlockRole::Paragraph
                    | LiquidBlockRole::Lead
                    | LiquidBlockRole::Abstract
                    | LiquidBlockRole::Quote
                    | LiquidBlockRole::Explainer
                    | LiquidBlockRole::Takeaway
                    | LiquidBlockRole::Holding
                    | LiquidBlockRole::Issue
            )
        })
        .count()
}

fn looks_like_reference_entry(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    word_count(text) >= 4
        && (contains_reference_year(text)
            || looks_like_citation_footnote_line(text)
            || lower.contains("doi:")
            || lower.contains("http://")
            || lower.contains("https://")
            || lower.contains("journal")
            || lower.contains("law review")
            || lower.contains("l. rev.")
            || lower.contains("university press")
            || lower.contains("vol."))
}

fn suppress_duplicate_pull_quotes(blocks: Vec<LiquidBlock>) -> Vec<LiquidBlock> {
    if blocks.len() < 4 {
        return blocks;
    }

    let body_keys = blocks
        .iter()
        .map(|block| {
            is_pull_quote_body_search_block(block)
                .then(|| normalize_pull_quote_key(&block.text))
                .filter(|key| !key.is_empty())
        })
        .collect::<Vec<_>>();

    blocks
        .iter()
        .enumerate()
        .filter(|(index, block)| {
            !is_duplicate_pull_quote_artifact(block, *index, &blocks, &body_keys)
        })
        .map(|(_, block)| block.clone())
        .collect()
}

fn is_duplicate_pull_quote_artifact(
    block: &LiquidBlock,
    index: usize,
    blocks: &[LiquidBlock],
    body_keys: &[Option<String>],
) -> bool {
    if !is_pull_quote_candidate(block) || !has_pull_quote_body_neighbors(blocks, index) {
        return false;
    }

    let key = normalize_pull_quote_key(&block.text);
    let key_words = word_count(&key);
    if key_words < 6 {
        return false;
    }
    let candidate_words = word_count(&block.text);
    let candidate_chars = block.text.chars().count();

    body_keys
        .iter()
        .enumerate()
        .any(|(other_index, other_key)| {
            if other_index == index {
                return false;
            }
            let Some(other_key) = other_key else {
                return false;
            };
            if !normalized_key_contains_phrase(other_key, &key) {
                return false;
            }

            let other = &blocks[other_index];
            if block.role == LiquidBlockRole::Quote && other_key == &key {
                return true;
            }

            word_count(&other.text) >= candidate_words + 5
                && other.text.chars().count() >= candidate_chars + 24
        })
}

fn is_pull_quote_candidate(block: &LiquidBlock) -> bool {
    let text = block.text.trim();
    let words = word_count(text);
    let chars = text.chars().count();
    if !(6..=45).contains(&words) || !(35..=260).contains(&chars) {
        return false;
    }

    match block.role {
        LiquidBlockRole::Quote => true,
        LiquidBlockRole::Paragraph => {
            words <= 32
                && !text.contains(':')
                && text.ends_with(['.', '?', '!', '"', '”'])
                && !looks_like_heading(text)
                && !looks_like_caption(text, 0)
                && !looks_like_front_matter_metadata(text)
                && !looks_like_reference_entry(text)
        }
        _ => false,
    }
}

fn is_pull_quote_body_search_block(block: &LiquidBlock) -> bool {
    matches!(
        block.role,
        LiquidBlockRole::Paragraph | LiquidBlockRole::Lead | LiquidBlockRole::Abstract
    )
}

fn has_pull_quote_body_neighbors(blocks: &[LiquidBlock], index: usize) -> bool {
    has_substantive_body_before(blocks, index) && has_substantive_body_after(blocks, index)
}

fn has_substantive_body_before(blocks: &[LiquidBlock], index: usize) -> bool {
    blocks[..index]
        .iter()
        .rev()
        .take(4)
        .any(is_substantive_pull_quote_neighbor)
}

fn has_substantive_body_after(blocks: &[LiquidBlock], index: usize) -> bool {
    blocks[index + 1..]
        .iter()
        .take(4)
        .any(is_substantive_pull_quote_neighbor)
}

fn is_substantive_pull_quote_neighbor(block: &LiquidBlock) -> bool {
    matches!(
        block.role,
        LiquidBlockRole::Paragraph
            | LiquidBlockRole::Lead
            | LiquidBlockRole::Abstract
            | LiquidBlockRole::Syllabus
            | LiquidBlockRole::Explainer
            | LiquidBlockRole::Takeaway
            | LiquidBlockRole::Holding
            | LiquidBlockRole::Issue
    )
}

fn normalize_pull_quote_key(text: &str) -> String {
    let trimmed = text
        .trim()
        .trim_matches(|ch: char| {
            ch.is_whitespace() || matches!(ch, '"' | '\'' | '“' | '”' | '‘' | '’')
        })
        .trim();
    normalize_title_key(trimmed)
}

fn normalized_key_contains_phrase(body_key: &str, phrase_key: &str) -> bool {
    if body_key == phrase_key {
        return true;
    }
    body_key
        .strip_prefix(phrase_key)
        .is_some_and(|rest| rest.starts_with(' '))
        || body_key
            .strip_suffix(phrase_key)
            .is_some_and(|rest| rest.ends_with(' '))
        || body_key.contains(&format!(" {phrase_key} "))
}

fn promote_local_standfirst(mut blocks: Vec<LiquidBlock>) -> Vec<LiquidBlock> {
    if blocks.iter().any(|block| {
        matches!(
            block.role,
            LiquidBlockRole::Abstract | LiquidBlockRole::Lead
        )
    }) {
        return blocks;
    }

    for block in blocks.iter_mut().skip(1).take(10) {
        match block.role {
            LiquidBlockRole::Title
            | LiquidBlockRole::AuthorInfo
            | LiquidBlockRole::Metadata
            | LiquidBlockRole::Caption
            | LiquidBlockRole::Table
            | LiquidBlockRole::Contents
            | LiquidBlockRole::Header
            | LiquidBlockRole::Footer
            | LiquidBlockRole::Noise
            | LiquidBlockRole::Footnote
            | LiquidBlockRole::SectionBreak => continue,
            LiquidBlockRole::Paragraph if looks_like_standfirst_paragraph(&block.text) => {
                block.role = LiquidBlockRole::Lead;
                break;
            }
            _ => break,
        }
    }

    blocks
}

fn looks_like_standfirst_paragraph(text: &str) -> bool {
    let trimmed = text.trim();
    let words = word_count(trimmed);
    (8..=48).contains(&words)
        && trimmed.chars().count() <= 320
        && !trimmed.ends_with(':')
        && !looks_like_footnote_line(trimmed)
        && !looks_like_clause(trimmed)
        && !looks_like_toc_entry(trimmed)
        && !looks_like_news_kicker_metadata(trimmed)
        && !is_non_title_heading_text(trimmed)
        && (trimmed.ends_with(['.', '?', '!', '"', '\u{201d}']) || title_case_ratio(trimmed) < 0.48)
}

fn promote_local_lead(mut blocks: Vec<LiquidBlock>) -> Vec<LiquidBlock> {
    if blocks
        .iter()
        .any(|block| block.role == LiquidBlockRole::Abstract)
    {
        return blocks;
    }

    for block in blocks.iter_mut().skip(1) {
        match block.role {
            LiquidBlockRole::Title
            | LiquidBlockRole::Heading
            | LiquidBlockRole::Subheading
            | LiquidBlockRole::AuthorInfo
            | LiquidBlockRole::Metadata
            | LiquidBlockRole::Caption
            | LiquidBlockRole::Table
            | LiquidBlockRole::Contents
            | LiquidBlockRole::Header
            | LiquidBlockRole::Footer
            | LiquidBlockRole::Noise
            | LiquidBlockRole::Footnote
            | LiquidBlockRole::SectionBreak => continue,
            LiquidBlockRole::Paragraph if looks_like_lead_paragraph(&block.text) => {
                block.role = LiquidBlockRole::Lead;
                break;
            }
            _ => break,
        }
    }

    blocks
}

fn looks_like_lead_paragraph(text: &str) -> bool {
    let words = word_count(text);
    (12..=120).contains(&words)
        && !starts_article_transition(text)
        && !looks_like_footnote_line(text)
        && !looks_like_clause(text)
}

fn insert_local_section_breaks(blocks: Vec<LiquidBlock>) -> Vec<LiquidBlock> {
    let mut output = Vec::with_capacity(blocks.len() + blocks.len() / 8);
    let mut body_blocks_seen = 0usize;
    let mut body_blocks_since_break = 0usize;

    for block in blocks {
        if should_insert_local_section_break(
            &block,
            &output,
            body_blocks_seen,
            body_blocks_since_break,
        ) {
            push_section_break_if_needed(&mut output);
            body_blocks_since_break = 0;
        }
        if !matches!(
            block.role,
            LiquidBlockRole::Title
                | LiquidBlockRole::Contents
                | LiquidBlockRole::Header
                | LiquidBlockRole::Footer
                | LiquidBlockRole::Noise
                | LiquidBlockRole::Footnote
                | LiquidBlockRole::SectionBreak
        ) {
            body_blocks_seen += 1;
            body_blocks_since_break += 1;
        } else if block.role == LiquidBlockRole::SectionBreak {
            body_blocks_since_break = 0;
        }
        output.push(block);
    }

    output
}

fn should_insert_local_section_break(
    block: &LiquidBlock,
    output: &[LiquidBlock],
    body_blocks_seen: usize,
    body_blocks_since_break: usize,
) -> bool {
    if body_blocks_seen < 2 {
        return false;
    }

    let Some(previous) = output
        .iter()
        .rev()
        .find(|candidate| candidate.role != LiquidBlockRole::SectionBreak)
    else {
        return false;
    };

    if matches!(
        previous.role,
        LiquidBlockRole::Title | LiquidBlockRole::Heading | LiquidBlockRole::Subheading
    ) {
        return false;
    }

    if matches!(
        block.role,
        LiquidBlockRole::Heading | LiquidBlockRole::Subheading
    ) {
        return true;
    }
    if looks_like_dissent_or_concurrence_heading(&block.text) {
        // Light court boundary: dissent/concurrence headings force section break
        // (even if not detected as Heading by other rules).
        return true;
    }
    if block.role == LiquidBlockRole::Metadata {
        return body_blocks_seen >= 4;
    }

    if body_blocks_seen >= 4
        && body_blocks_since_break >= 4
        && starts_article_transition(&block.text)
    {
        return true;
    }

    body_blocks_seen >= 8
        && body_blocks_since_break >= 6
        && matches!(
            block.role,
            LiquidBlockRole::Paragraph | LiquidBlockRole::Quote
        )
}
