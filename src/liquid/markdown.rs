use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::{
    LiquidBlockRole, LiquidBlockSourceLines, LiquidDocument, LiquidFootnoteLink,
    should_preserve_terminal_hyphen,
};

const CALLOUT_START: char = '\u{E000}';
const CALLOUT_END: char = '\u{E001}';
const MARKDOWN_MARKER_START: char = '\u{E100}';
const MARKDOWN_MARKER_END: char = '\u{E101}';
const MAX_NOTE_MARKER: u16 = 999;
/// Consecutive dash-like glyphs that mark a printed footnote separator rule.
const FOOTNOTE_SEPARATOR_MIN_RUN: usize = 24;
/// A rejected line repeating at least this often, and no longer than
/// [`FURNITURE_MAX_WORDS`], is a running head or folio rather than prose.
const FURNITURE_MIN_REPEATS: usize = 3;
const FURNITURE_MAX_WORDS: usize = 14;
/// Words a rejected block needs before it is rescued as prose rather than
/// left out as furniture.
const RESCUED_NOISE_MIN_WORDS: usize = 20;
/// Table-classified text this long must still look tabular before Markdown
/// fencing is allowed. OCR-layered scans can make every line overlap a
/// page-sized image, which is useful classification evidence for recovering
/// notes but not enough reason to display ordinary prose as code.
const TABLE_PROSE_MIN_WORDS: usize = 24;
/// Longest all-digit line still treated as a folio or accession stamp.
const MAX_BARE_NUMBER_LEN: usize = 12;
/// Below this share of markers matched to a note, the document is probably
/// being misread rather than merely under-linked, so fall back to endnotes.
///
/// Measured over 100 law review articles, this floor is what decides whether a
/// document gets linked footnotes at all, and it was set far too high. At 0.75
/// it silently discarded 22 documents whose markers were mostly matched; at
/// 0.50 it still discarded 13 more between 0.25 and 0.50.
///
/// An unmatched marker does not become a wrong link, it becomes no link, so
/// coverage is the only thing a low rate costs. What can attach a citation to
/// the wrong sentence is an ambiguous match, gated by
/// [`MAX_FOOTNOTE_AMBIGUOUS_RATE`], or a placement failure, gated separately
/// below. Only three documents in the hundred contain a single ambiguous
/// match. A quarter of the citations linked is worth more to a reader than a
/// wall of notes with nothing pointing into it.
const MIN_FOOTNOTE_LANDING_RATE: f32 = 0.20;
/// Ambiguous matches are the ones that can attach a citation to the wrong
/// sentence, so they are gated far more tightly than unmatched markers.
const MAX_FOOTNOTE_AMBIGUOUS_RATE: f32 = 0.02;
const LOW_LINK_CONFIDENCE_WARNING: &str =
    "footnote linking below confidence threshold; notes appended as a section";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownOptions {
    pub footnotes: FootnoteMode,
    pub include_tables: bool,
    pub include_metadata: bool,
}

impl Default for MarkdownOptions {
    fn default() -> Self {
        Self {
            footnotes: FootnoteMode::Inline,
            include_tables: true,
            include_metadata: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FootnoteMode {
    Inline,
    Endnotes,
    Omit,
}

impl Default for FootnoteMode {
    fn default() -> Self {
        Self::Inline
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownExport {
    pub text: String,
    pub word_count: usize,
    pub footnote_count: usize,
    pub footnotes_inlined: bool,
    pub warnings: Vec<String>,
}

pub fn liquid_document_markdown(
    document: &LiquidDocument,
    options: &MarkdownOptions,
) -> MarkdownExport {
    let markdown_links = safe_markdown_footnote_links(document);
    let linked_note_indices = markdown_links
        .iter()
        .map(|link| link.note_block_index)
        .collect::<BTreeSet<_>>();
    let title = resolved_title(document);
    let first_body_index = front_matter_body_index(document);
    let front_author_indices = document
        .blocks
        .iter()
        .enumerate()
        .filter_map(|(index, block)| {
            let repeats_title = title
                .as_ref()
                .is_some_and(|title| redundant_title_heading(title, &block.text));
            (!repeats_title
                && (block.role == LiquidBlockRole::AuthorInfo
                    || ((index < first_body_index
                        || (index < 16 && has_trailing_front_matter_author_marker(&block.text)))
                        && !matches!(
                            block.role,
                            LiquidBlockRole::Title
                                | LiquidBlockRole::Footnote
                                | LiquidBlockRole::Marginalia
                        )
                        && looks_like_front_matter_byline(&block.text))))
            .then_some(index)
        })
        .collect::<BTreeSet<_>>();
    let note_indices = document
        .blocks
        .iter()
        .enumerate()
        .filter_map(|(index, block)| {
            is_note_candidate(
                block.role,
                &block.text,
                linked_note_indices.contains(&index),
            )
            .then_some(index)
        })
        .collect::<Vec<_>>();
    let available_author_notes =
        collect_author_notes(document, &linked_note_indices, &front_author_indices);
    let has_non_author_notes = note_indices
        .iter()
        .any(|index| !available_author_notes.note_blocks.contains(index));

    let mut warnings = Vec::new();
    let suppressed_unsafe_links = document
        .footnote_links
        .len()
        .saturating_sub(markdown_links.len());
    if suppressed_unsafe_links > 0 {
        warnings.push(format!(
            "{suppressed_unsafe_links} footnote link(s) sharing a printed number across unscoped note blocks were listed without links"
        ));
    }
    let mut inline_blocks = BTreeMap::new();
    let footnotes_inlined = match options.footnotes {
        FootnoteMode::Inline if !has_non_author_notes && markdown_links.is_empty() => true,
        FootnoteMode::Inline => {
            // Gate on correctness, not coverage. A marker that found no note
            // head simply does not become a link, so a low landing rate costs
            // reach; what would put a citation on the wrong sentence is an
            // ambiguous match, or a placement failure, which `too_many_failed`
            // below already catches. Gating on landing alone discarded 416
            // correct links in one held-out article because 51 of its 468
            // markers had no matching note.
            let integrity_is_usable =
                document
                    .footnote_link_integrity
                    .as_ref()
                    .is_some_and(|integrity| {
                        integrity.landing_rate >= MIN_FOOTNOTE_LANDING_RATE
                            && integrity.ambiguous_rate <= MAX_FOOTNOTE_AMBIGUOUS_RATE
                    });
            if !integrity_is_usable {
                warnings.push(LOW_LINK_CONFIDENCE_WARNING.to_owned());
                false
            } else {
                let placement = rewrite_inline_blocks(document, &markdown_links);
                let too_many_failed = placement.attempted > 0
                    && placement.failed.saturating_mul(100) > placement.attempted.saturating_mul(5);
                if too_many_failed {
                    warnings.push(LOW_LINK_CONFIDENCE_WARNING.to_owned());
                    false
                } else {
                    inline_blocks = placement.blocks;
                    if placement.appended > 0 {
                        warnings.push(format!(
                            "{} footnote marker(s) were appended to their paragraph because their exact positions were unavailable",
                            placement.appended
                        ));
                    }
                    true
                }
            }
        }
        FootnoteMode::Endnotes | FootnoteMode::Omit => false,
    };

    let author_notes = if footnotes_inlined {
        available_author_notes
    } else {
        AuthorNotes::default()
    };
    let mut author_lines = Vec::new();
    let mut author_position = BTreeMap::<String, usize>::new();
    for (index, block) in document.blocks.iter().enumerate().filter(|(index, _)| {
        front_author_indices.contains(index) && !author_notes.note_blocks.contains(index)
    }) {
        let text = author_display_text(&block.text);
        if text.is_empty() {
            continue;
        }
        let markers = author_notes
            .by_author_block
            .get(&index)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let rendered = render_author_byline(&block.text, markers);
        let key = normalize_whitespace(&text).to_ascii_lowercase();
        if let Some(position) = author_position.get(&key).copied() {
            // Prefer the repeated printed byline that retains its author-note
            // marker over an unmarked repository cover duplicate.
            if !markers.is_empty() {
                author_lines[position] = rendered;
            }
        } else {
            author_position.insert(key, author_lines.len());
            author_lines.push(rendered);
        }
    }

    let mut writer = MarkdownWriter::default();
    if let Some(title) = &title {
        let markers = author_notes
            .title_markers
            .iter()
            .map(|marker| format!("[^{marker}]"))
            .collect::<String>();
        writer.push(format!("# {title}{markers}"), BlockJoin::Loose);
    }
    for author in author_lines {
        writer.push(author, BlockJoin::Loose);
    }

    let mut heading_context = HeadingContext::default();
    let mut last_source_text: Option<String> = None;
    let mut last_special_section = None;
    let mut omitted_footnote_separator_fragments = 0usize;
    let mut omitted_footnote_separator_rules = 0usize;
    let mut discarded_furniture = 0usize;
    // Running heads and folios repeat on every page; body text does not. This
    // is what separates furniture from content the classifier merely disliked.
    let repeated_noise = repeated_noise_texts(document);
    let cross_page_tight_joins = cross_page_body_continuation_indices(document);
    let misclassified_heading_tight_joins =
        misclassified_heading_prose_tight_join_indices(document);
    let standalone_display_quotes = standalone_display_quote_indices(document);
    let inline_display_quote_cues = inline_display_quote_cues(document);
    let (figure_exports, figure_label_blocks) = figure_export_plan(document, &inline_blocks);
    let (table_run_text, table_run_continuations, prose_table_blocks) =
        adjacent_table_run_text(document, &inline_blocks, &figure_label_blocks);
    let mut visual_figure_count = 0usize;
    let mut prose_table_fallbacks = 0usize;
    for (index, block) in document.blocks.iter().enumerate() {
        if matches!(
            block.role,
            LiquidBlockRole::Title | LiquidBlockRole::AuthorInfo
        ) || front_author_indices.contains(&index)
            || linked_note_indices.contains(&index)
            || author_notes.note_blocks.contains(&index)
        {
            continue;
        }
        if figure_label_blocks.contains(&index) {
            continue;
        }
        if table_run_continuations.contains(&index) {
            continue;
        }

        let mut raw_text = table_run_text
            .get(&index)
            .or_else(|| inline_blocks.get(&index))
            .map(String::as_str)
            .unwrap_or(&block.text);
        if looks_like_repository_cover_block(raw_text) {
            discarded_furniture += 1;
            continue;
        }
        // Contents detection must inspect the source block, not the inline
        // rendering. Rewritten footnote placeholders are numeric, and a body
        // paragraph with several callouts plus a phrase such as "we have
        // concluded" otherwise looks like a no-dotleader contents row.
        if looks_like_contents_block(&block.text) {
            continue;
        }
        if (index < first_body_index || index < 16)
            && (front_matter_genre_label(raw_text)
                || title
                    .as_ref()
                    .is_some_and(|title| redundant_title_heading(title, raw_text)))
        {
            continue;
        }
        // A block can begin with the PDF's footnote-separator rule and then
        // continue into real content. Drop the rule, not the block: discarding
        // the whole block silently deletes body prose.
        let separator_trimmed;
        if block.role != LiquidBlockRole::SectionBreak {
            if let Some(prefix) = footnote_separator_prefix_len(raw_text) {
                let remainder = raw_text[prefix..].trim_start();
                if normalize_whitespace(&strip_callout_sentinels(remainder)).is_empty() {
                    omitted_footnote_separator_fragments += 1;
                    continue;
                }
                separator_trimmed = remainder.to_string();
                raw_text = &separator_trimmed;
                omitted_footnote_separator_rules += 1;
            }
        }
        let source_key = normalize_whitespace(&strip_callout_sentinels(raw_text));
        if !source_key.is_empty()
            && last_source_text
                .as_ref()
                .is_some_and(|previous| previous.eq_ignore_ascii_case(&source_key))
        {
            continue;
        }

        let emitted = if standalone_display_quotes.contains(&index) {
            let continuation = index > 0 && standalone_display_quotes.contains(&(index - 1));
            let text = if continuation {
                normalize_and_escape_body(raw_text)
            } else {
                render_quote(raw_text)
            };
            if text.is_empty() {
                false
            } else {
                writer.push(
                    text,
                    if continuation {
                        BlockJoin::Tight
                    } else {
                        BlockJoin::Loose
                    },
                );
                last_special_section = None;
                true
            }
        } else {
            match block.role {
                LiquidBlockRole::Heading | LiquidBlockRole::Subheading => {
                    let level_text = normalize_whitespace(&strip_callout_sentinels(raw_text));
                    let text = normalize_heading_text(raw_text);
                    let begins_with_callout = block.text.trim_start().starts_with(CALLOUT_START);
                    if text.is_empty() {
                        false
                    } else if index < first_body_index
                        && title
                            .as_ref()
                            .is_some_and(|title| redundant_title_heading(title, &level_text))
                    {
                        false
                    } else if numbered_outline_heading_without_body(raw_text) {
                        let level = heading_context.level(&level_text, block.role);
                        writer.push(
                            format!("{} {text}", "#".repeat(level as usize)),
                            BlockJoin::Loose,
                        );
                        last_special_section = None;
                        true
                    } else if let Some((heading, body)) = numbered_outline_run_in(raw_text) {
                        let level = heading_context.level(&heading, block.role);
                        writer.push(
                            format!(
                                "{} {}",
                                "#".repeat(level as usize),
                                normalize_heading_text(&heading)
                            ),
                            BlockJoin::Loose,
                        );
                        writer.push(normalize_and_escape_body(&body), BlockJoin::Loose);
                        last_special_section = None;
                        true
                    } else if begins_with_callout
                        || person_name_continuation_misclassified_as_heading(&level_text)
                        || sentence_like_prose_misclassified_as_heading(&level_text)
                        || inline_callout_followed_by_prose(raw_text)
                        || !reads_like_heading(&level_text)
                    {
                        // Law reviews italicise case names, and a stray italic
                        // fragment upstream is easily mistaken for a heading. An
                        // outline entry never ends mid-clause, so render the text
                        // as the prose it is rather than emit `## Raich,`.
                        let body = normalize_and_escape_body(raw_text);
                        if body.is_empty() {
                            false
                        } else {
                            writer.push(
                                body,
                                if misclassified_heading_tight_joins.contains(&index) {
                                    BlockJoin::Tight
                                } else {
                                    BlockJoin::Loose
                                },
                            );
                            last_special_section = None;
                            true
                        }
                    } else {
                        let level = heading_context.level(&level_text, block.role);
                        writer.push(
                            format!("{} {text}", "#".repeat(level as usize)),
                            BlockJoin::Loose,
                        );
                        last_special_section = None;
                        true
                    }
                }
                LiquidBlockRole::Abstract | LiquidBlockRole::Syllabus => {
                    let labeled_text = if block.role == LiquidBlockRole::Abstract {
                        strip_leading_abstract_label(raw_text)
                    } else {
                        raw_text
                    };
                    let text = normalize_and_escape_body(labeled_text);
                    if text.is_empty() {
                        false
                    } else {
                        let section = if block.role == LiquidBlockRole::Abstract {
                            "Abstract"
                        } else {
                            "Syllabus"
                        };
                        if last_special_section != Some(block.role) {
                            writer.push(format!("## {section}"), BlockJoin::Loose);
                        }
                        writer.push(
                            text,
                            if cross_page_tight_joins.contains(&index) {
                                BlockJoin::Tight
                            } else {
                                BlockJoin::Loose
                            },
                        );
                        last_special_section = Some(block.role);
                        true
                    }
                }
                LiquidBlockRole::Paragraph
                | LiquidBlockRole::Lead
                | LiquidBlockRole::Explainer
                | LiquidBlockRole::Takeaway
                | LiquidBlockRole::Holding
                | LiquidBlockRole::Issue
                | LiquidBlockRole::Definition
                | LiquidBlockRole::Clause
                | LiquidBlockRole::KeyClause => {
                    let front_abstract = (block.role == LiquidBlockRole::Paragraph
                        && index < first_body_index)
                        .then(|| strip_leading_abstract_label(raw_text))
                        .filter(|text| text.len() < raw_text.trim_start().len())
                        .map(normalize_and_escape_body)
                        .filter(|text| !text.is_empty());
                    if let Some(text) = front_abstract {
                        writer.push("## Abstract".to_owned(), BlockJoin::Loose);
                        writer.push(
                            text,
                            if cross_page_tight_joins.contains(&index) {
                                BlockJoin::Tight
                            } else {
                                BlockJoin::Loose
                            },
                        );
                        last_special_section = Some(LiquidBlockRole::Abstract);
                        true
                    } else if block.role == LiquidBlockRole::Paragraph
                        && let Some(cue) = inline_display_quote_cues.get(&index)
                        && let Some((introduction, quotation)) =
                            split_after_case_insensitive(raw_text, cue)
                    {
                        let introduction = normalize_and_escape_body(introduction);
                        let quotation = render_quote(quotation);
                        if introduction.is_empty() || quotation.is_empty() {
                            false
                        } else {
                            writer.push(introduction, BlockJoin::Loose);
                            writer.push(quotation, BlockJoin::Loose);
                            last_special_section = None;
                            true
                        }
                    } else if block.role == LiquidBlockRole::Paragraph
                        && let Some(text) = standalone_uppercase_roman_outline_heading(raw_text)
                    {
                        let level = heading_context.level(&text, LiquidBlockRole::Heading);
                        writer.push(
                            format!(
                                "{} {}",
                                "#".repeat(level as usize),
                                normalize_heading_text(&text)
                            ),
                            BlockJoin::Loose,
                        );
                        last_special_section = None;
                        true
                    } else if block.role == LiquidBlockRole::Paragraph
                        && let Some((heading, body)) = numbered_outline_run_in(raw_text)
                    {
                        let level = heading_context.level(&heading, LiquidBlockRole::Subheading);
                        writer.push(
                            format!(
                                "{} {}",
                                "#".repeat(level as usize),
                                normalize_heading_text(&heading)
                            ),
                            BlockJoin::Loose,
                        );
                        writer.push(normalize_and_escape_body(&body), BlockJoin::Loose);
                        last_special_section = None;
                        true
                    } else if block.role == LiquidBlockRole::Paragraph
                        && let Some(text) = star_paginated_heading(raw_text)
                    {
                        let level = heading_context.level(&text, LiquidBlockRole::Heading);
                        writer.push(
                            format!(
                                "{} {}",
                                "#".repeat(level as usize),
                                normalize_heading_text(&text)
                            ),
                            BlockJoin::Loose,
                        );
                        last_special_section = None;
                        true
                    } else {
                        let text = normalize_and_escape_body(raw_text);
                        if text.is_empty() {
                            false
                        } else {
                            let join = if cross_page_tight_joins.contains(&index)
                                || misclassified_heading_tight_joins.contains(&index)
                            {
                                BlockJoin::Tight
                            } else {
                                BlockJoin::Loose
                            };
                            writer.push(text, join);
                            last_special_section = None;
                            true
                        }
                    }
                }
                LiquidBlockRole::Quote => {
                    let text = render_quote(raw_text);
                    if text.is_empty() {
                        false
                    } else {
                        writer.push(text, BlockJoin::Loose);
                        last_special_section = None;
                        true
                    }
                }
                LiquidBlockRole::ListItem => {
                    let text = normalize_and_escape_body(raw_text);
                    if text.is_empty() {
                        false
                    } else {
                        writer.push(format!("- {text}"), BlockJoin::ListItem);
                        last_special_section = None;
                        true
                    }
                }
                LiquidBlockRole::Caption => {
                    let text = normalize_whitespace(raw_text);
                    if text.is_empty() {
                        false
                    } else if let Some(figure) = figure_exports.get(&index) {
                        writer.push(render_figure_notice(&text, figure), BlockJoin::Loose);
                        visual_figure_count += 1;
                        last_special_section = None;
                        true
                    } else {
                        writer.push(format!("*{text}*"), BlockJoin::Loose);
                        last_special_section = None;
                        true
                    }
                }
                LiquidBlockRole::Table if options.include_tables => {
                    let text = raw_text.trim();
                    if text.is_empty() {
                        false
                    } else if prose_table_blocks.contains(&index)
                        || table_run_is_sentence_prose(text)
                    {
                        let body = normalize_and_escape_body(text);
                        if body.is_empty() {
                            false
                        } else {
                            writer.push(body, BlockJoin::Loose);
                            prose_table_fallbacks += 1;
                            last_special_section = None;
                            true
                        }
                    } else {
                        let fence = if text.contains("```") { "````" } else { "```" };
                        writer.push(format!("{fence}\n{text}\n{fence}"), BlockJoin::Loose);
                        last_special_section = None;
                        true
                    }
                }
                LiquidBlockRole::Metadata if options.include_metadata => {
                    let text = normalize_and_escape_body(&compact_liquid_metadata(raw_text));
                    if text.is_empty() {
                        false
                    } else {
                        writer.push(text, BlockJoin::Loose);
                        last_special_section = None;
                        true
                    }
                }
                LiquidBlockRole::SectionBreak => {
                    writer.push("***".to_owned(), BlockJoin::Loose);
                    last_special_section = None;
                    true
                }
                // Text the classifier rejected. Furniture is dropped, deliberately
                // and by an explicit test; anything else is kept, because a role
                // decision should not silently destroy content.
                LiquidBlockRole::Noise => {
                    let text = normalize_whitespace(&strip_callout_sentinels(raw_text));
                    // Only substantial prose is worth rescuing. Furniture is
                    // short by nature -- folios, running heads, contents lines --
                    // and a length floor separates the two far more reliably than
                    // any pattern, at the cost of leaving short stray lines out.
                    let words = text.split_whitespace().count();
                    if text.is_empty()
                        || words < RESCUED_NOISE_MIN_WORDS
                        || is_discardable_furniture(&text, &repeated_noise)
                    {
                        discarded_furniture += 1;
                        false
                    } else {
                        let body = normalize_and_escape_body(raw_text);
                        if body.is_empty() {
                            false
                        } else {
                            writer.push(body, BlockJoin::Loose);
                            last_special_section = None;
                            true
                        }
                    }
                }
                LiquidBlockRole::Footnote
                | LiquidBlockRole::Marginalia
                | LiquidBlockRole::Header
                | LiquidBlockRole::Footer
                | LiquidBlockRole::Contents
                | LiquidBlockRole::Table
                | LiquidBlockRole::Metadata
                | LiquidBlockRole::Title
                | LiquidBlockRole::AuthorInfo => false,
            }
        };
        if emitted && !source_key.is_empty() {
            last_source_text = Some(source_key);
        }
    }
    if visual_figure_count > 0 {
        let figure_word = if visual_figure_count == 1 {
            "figure"
        } else {
            "figures"
        };
        warnings.push(format!(
            "{visual_figure_count} visual {figure_word} referenced; visual content is not included in this text-only export; source PDF locations are shown where available"
        ));
    }
    if omitted_footnote_separator_fragments > 0 {
        warnings.push(format!(
            "omitted {omitted_footnote_separator_fragments} standalone footnote-separator rule(s) from the article body"
        ));
    }
    if discarded_furniture > 0 {
        warnings.push(format!(
            "dropped {discarded_furniture} block(s) of page furniture such as running heads and folios"
        ));
    }
    if prose_table_fallbacks > 0 {
        warnings.push(format!(
            "rendered {prose_table_fallbacks} sentence-like table-classified block(s) as ordinary prose"
        ));
    }
    if omitted_footnote_separator_rules > 0 {
        warnings.push(format!(
            "stripped a leading footnote-separator rule from {omitted_footnote_separator_rules} block(s), keeping the text that followed"
        ));
    }

    let footnote_count = match options.footnotes {
        FootnoteMode::Inline if footnotes_inlined => {
            let notes = build_inline_notes(
                document,
                &note_indices,
                author_notes,
                &markdown_links,
                &mut warnings,
            );
            append_inline_notes(&mut writer, &notes);
            notes.definitions.len() + notes.unlinked.len()
        }
        FootnoteMode::Inline | FootnoteMode::Endnotes => {
            let notes = collect_endnotes(document, &note_indices);
            append_endnotes(&mut writer, &notes);
            notes.len()
        }
        FootnoteMode::Omit => 0,
    };

    let text = finalize_markdown(writer.finish());
    let word_count = text
        .split_whitespace()
        .filter(|word| word.chars().any(char::is_alphanumeric))
        .count();
    MarkdownExport {
        text,
        word_count,
        footnote_count,
        footnotes_inlined,
        warnings,
    }
}

#[derive(Debug, Clone, Default)]
struct FigureExportInfo {
    page_index: Option<usize>,
    labels: Vec<String>,
}

/// Copy Markdown is deliberately portable text, so it cannot carry a PDF
/// image or a local sidecar path. Preserve each explicit figure as a concise,
/// page-addressed notice instead. Short same-page Table/Figure runs are labels
/// from vector artwork, not a rectangular table; retain their words while
/// explicitly declining to invent their spatial relationship.
fn figure_export_plan(
    document: &LiquidDocument,
    inline_blocks: &BTreeMap<usize, String>,
) -> (BTreeMap<usize, FigureExportInfo>, BTreeSet<usize>) {
    const MAX_FIGURE_LABEL_BLOCK_DISTANCE: usize = 6;
    const MIN_FIGURE_LABELS: usize = 3;

    let source_by_block = document
        .block_source_lines
        .iter()
        .map(|source| (source.block_index, source))
        .collect::<BTreeMap<_, _>>();
    let figures = document
        .blocks
        .iter()
        .enumerate()
        .filter(|(_, block)| figure_caption_block(block))
        .map(|(block_index, _)| {
            let source = source_by_block.get(&block_index).copied();
            (
                block_index,
                source.and_then(block_source_page),
                source.and_then(block_source_line),
            )
        })
        .collect::<Vec<_>>();
    let mut exports = figures
        .iter()
        .map(|(block_index, page_index, _)| {
            (
                *block_index,
                FigureExportInfo {
                    page_index: *page_index,
                    labels: Vec::new(),
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut assignments = BTreeMap::<usize, Vec<(usize, usize, Vec<String>)>>::new();

    for (block_index, block) in document.blocks.iter().enumerate() {
        if block.role != LiquidBlockRole::Table
            || !block
                .label
                .as_deref()
                .is_some_and(|label| label.eq_ignore_ascii_case("Table/Figure"))
        {
            continue;
        }
        let Some(source) = source_by_block.get(&block_index).copied() else {
            continue;
        };
        let Some(page_index) = block_source_page(source) else {
            continue;
        };
        if source
            .lines
            .iter()
            .any(|line| line.page_index != page_index || !line.note_markers.is_empty())
        {
            continue;
        }
        // Link placement operates on the assembled block text, where exact
        // callout sentinels survive even when the raw source-line sidecar only
        // contains flattened text such as `Action15`. Prefer that rewritten
        // text for a one-line visual label so the figure notice retains the
        // real Markdown note marker instead of printing the digits as prose.
        let labels = if source.lines.len() == 1 {
            inline_blocks
                .get(&block_index)
                .map(|text| vec![normalize_whitespace(text)])
                .unwrap_or_else(|| {
                    vec![normalize_whitespace(&strip_callout_sentinels(
                        &source.lines[0].text,
                    ))]
                })
        } else {
            source
                .lines
                .iter()
                .map(|line| normalize_whitespace(&strip_callout_sentinels(&line.text)))
                .collect::<Vec<_>>()
        }
        .into_iter()
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>();
        if labels.is_empty() || labels.iter().any(|label| !figure_label_text_like(label)) {
            continue;
        }
        let Some((figure_index, _, _)) = figures
            .iter()
            .filter(|(figure_index, figure_page, _)| {
                *figure_page == Some(page_index)
                    && figure_index.abs_diff(block_index) <= MAX_FIGURE_LABEL_BLOCK_DISTANCE
            })
            .min_by_key(|(figure_index, _, figure_line)| {
                (
                    figure_index.abs_diff(block_index),
                    figure_line
                        .map(|line| line.abs_diff(source.lines[0].line_index))
                        .unwrap_or(usize::MAX),
                )
            })
        else {
            continue;
        };
        assignments.entry(*figure_index).or_default().push((
            block_index,
            source.lines[0].line_index,
            labels,
        ));
    }

    let mut suppressed = BTreeSet::new();
    for (figure_index, mut assigned) in assignments {
        let label_count = assigned
            .iter()
            .map(|(_, _, labels)| labels.len())
            .sum::<usize>();
        if label_count < MIN_FIGURE_LABELS {
            continue;
        }
        assigned.sort_by_key(|(_, first_line, _)| *first_line);
        let mut seen = BTreeSet::new();
        let mut labels = Vec::new();
        for (block_index, _, block_labels) in assigned {
            suppressed.insert(block_index);
            for label in block_labels {
                let key = label.to_ascii_lowercase();
                if seen.insert(key) {
                    labels.push(label);
                }
            }
        }
        if let Some(info) = exports.get_mut(&figure_index) {
            info.labels = labels;
        }
    }

    (exports, suppressed)
}

fn figure_caption_block(block: &crate::liquid::LiquidBlock) -> bool {
    block.role == LiquidBlockRole::Caption
        && (block
            .label
            .as_deref()
            .is_some_and(|label| label.eq_ignore_ascii_case("Figure"))
            || figure_caption_text(&block.text))
}

fn figure_caption_text(text: &str) -> bool {
    normalize_whitespace(&strip_callout_sentinels(text))
        .split_whitespace()
        .next()
        .map(|word| {
            word.trim_matches(|ch: char| !ch.is_ascii_alphabetic())
                .to_ascii_lowercase()
        })
        .is_some_and(|word| matches!(word.as_str(), "figure" | "fig"))
}

fn block_source_page(source: &LiquidBlockSourceLines) -> Option<usize> {
    let page_index = source.lines.first()?.page_index;
    source
        .lines
        .iter()
        .all(|line| line.page_index == page_index)
        .then_some(page_index)
}

fn block_source_line(source: &LiquidBlockSourceLines) -> Option<usize> {
    source.lines.iter().map(|line| line.line_index).min()
}

fn figure_label_text_like(text: &str) -> bool {
    let words = text.split_whitespace().count();
    (1..=5).contains(&words)
        && text.len() <= 64
        && !text
            .chars()
            .any(|ch| matches!(ch, '.' | ';' | ':' | '?' | '!'))
}

fn render_figure_notice(caption: &str, figure: &FigureExportInfo) -> String {
    let caption = normalize_and_escape_body(caption).replace('*', "\\*");
    let location = figure.page_index.map_or_else(
        || "see the source PDF".to_owned(),
        |page_index| format!("see PDF page {}", page_index + 1),
    );
    let mut notice = format!(
        "> **{caption}**\n> Visual content is not included in this text-only export; {location}."
    );
    if !figure.labels.is_empty() {
        let labels = figure
            .labels
            .iter()
            .map(|label| normalize_and_escape_body(label).replace('*', "\\*"))
            .collect::<Vec<_>>()
            .join("; ");
        notice.push_str(&format!(
            "\n> Extracted labels (unordered; spatial relationships are not represented): {labels}."
        ));
    }
    notice
}

/// Whether a run carrying the semantic `Table` role is actually sentence
/// prose for Markdown presentation.
///
/// This is intentionally a serialization guard. The underlying role remains
/// available to note recovery and layout diagnostics. Dense numeric grids stay
/// fenced, while long prose and citation paragraphs are emitted as readable
/// body text even when an OCR-layered scan's page image supplied a false table
/// overlap signal.
fn table_run_is_sentence_prose(text: &str) -> bool {
    let lines = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    let total_words = lines
        .iter()
        .map(|line| line.split_whitespace().count())
        .sum::<usize>();
    if total_words < TABLE_PROSE_MIN_WORDS || lines.is_empty() {
        return false;
    }

    let numeric_rows = lines
        .iter()
        .filter(|line| {
            let tokens = line
                .split_whitespace()
                .map(|token| token.trim_matches(|ch: char| !ch.is_alphanumeric()))
                .filter(|token| !token.is_empty())
                .collect::<Vec<_>>();
            let numeric = tokens
                .iter()
                .filter(|token| token.chars().any(|ch| ch.is_ascii_digit()))
                .count();
            tokens.len() >= 3 && numeric >= 2 && numeric * 2 >= tokens.len()
        })
        .count();
    if numeric_rows >= 3 && numeric_rows * 2 >= lines.len() {
        return false;
    }

    let prose_lines = lines
        .iter()
        .filter(|line| {
            let alphabetic_words = line
                .split_whitespace()
                .filter(|word| word.chars().any(char::is_alphabetic))
                .count();
            alphabetic_words >= 6
                || (alphabetic_words >= 4
                    && line
                        .chars()
                        .last()
                        .is_some_and(|ch| matches!(ch, '.' | '!' | '?' | ')' | ']' | '"')))
        })
        .count();

    prose_lines * 2 >= lines.len()
}

/// PyMuPDF commonly emits each visual table row (and sometimes each wrapped
/// cell) as its own semantic Table block. Keep those source boundaries in the
/// document model for navigation, but render an adjacent run as one code block
/// so copied Markdown remains a readable table rather than dozens of one-line
/// fences.
fn adjacent_table_run_text(
    document: &LiquidDocument,
    inline_blocks: &BTreeMap<usize, String>,
    excluded_blocks: &BTreeSet<usize>,
) -> (BTreeMap<usize, String>, BTreeSet<usize>, BTreeSet<usize>) {
    let mut starts = BTreeMap::new();
    let mut continuations = BTreeSet::new();
    let mut prose_blocks = BTreeSet::new();
    let mut index = 0usize;
    while index < document.blocks.len() {
        if document.blocks[index].role != LiquidBlockRole::Table || excluded_blocks.contains(&index)
        {
            index += 1;
            continue;
        }
        let start = index;
        let mut rows = Vec::new();
        let mut run_indices = Vec::new();
        while index < document.blocks.len()
            && document.blocks[index].role == LiquidBlockRole::Table
            && !excluded_blocks.contains(&index)
        {
            run_indices.push(index);
            let text = inline_blocks
                .get(&index)
                .map(String::as_str)
                .unwrap_or(&document.blocks[index].text)
                .trim();
            if !text.is_empty() {
                rows.push(text);
            }
            index += 1;
        }
        if index - start > 1 && !rows.is_empty() {
            let combined = rows.join("\n");
            if table_run_is_sentence_prose(&combined) {
                prose_blocks.extend(run_indices);
            } else {
                starts.insert(start, combined);
                continuations.extend(run_indices.into_iter().skip(1));
            }
        }
    }
    (starts, continuations, prose_blocks)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockJoin {
    Loose,
    ListItem,
    Tight,
}

#[derive(Default)]
struct MarkdownWriter {
    text: String,
    last_join: Option<BlockJoin>,
    last_rendered: Option<String>,
}

impl MarkdownWriter {
    fn push(&mut self, text: String, join: BlockJoin) {
        let text = text.trim().to_owned();
        if text.is_empty()
            || self
                .last_rendered
                .as_ref()
                .is_some_and(|previous| previous.eq_ignore_ascii_case(&text))
        {
            return;
        }
        let join = if join == BlockJoin::Loose
            && self.last_rendered.as_ref().is_some_and(|previous| {
                markdown_plain_body_boundary(previous, &text)
                    && markdown_paragraph_is_visibly_open(previous)
                    && markdown_starts_with_lowercase_prose(&text)
            }) {
            BlockJoin::Tight
        } else {
            join
        };
        if !self.text.is_empty() {
            if join == BlockJoin::Tight {
                if self.text.ends_with('-')
                    && text
                        .chars()
                        .find(|ch| ch.is_alphabetic())
                        .is_some_and(char::is_lowercase)
                {
                    self.text.pop();
                } else if !self.text.chars().last().is_some_and(char::is_whitespace) {
                    self.text.push(' ');
                }
            } else if self.last_join == Some(BlockJoin::ListItem) && join == BlockJoin::ListItem {
                self.text.push('\n');
            } else {
                self.text.push_str("\n\n");
            }
        }
        self.text.push_str(&text);
        self.last_join = Some(join);
        self.last_rendered = Some(text);
    }

    fn finish(self) -> String {
        self.text
    }
}

fn markdown_plain_body_boundary(previous: &str, current: &str) -> bool {
    [previous, current]
        .into_iter()
        .all(|text| !markdown_block_is_structural(text))
}

fn markdown_block_is_structural(text: &str) -> bool {
    let trimmed = text.trim_start();
    trimmed.starts_with('#')
        || trimmed.starts_with('>')
        || trimmed.starts_with("```")
        || trimmed.starts_with("~~~")
        || trimmed.starts_with("[^")
        || trimmed.starts_with("- ")
        || trimmed.starts_with("* ")
        || trimmed.starts_with("+ ")
        || matches!(trimmed, "***" | "---" | "___")
}

fn markdown_paragraph_is_visibly_open(text: &str) -> bool {
    let mut trimmed = text.trim_end();
    while trimmed.ends_with(']') {
        let Some(marker_start) = trimmed.rfind("[^") else {
            break;
        };
        let marker = &trimmed[marker_start + 2..trimmed.len() - 1];
        if marker.is_empty()
            || !marker
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '*'))
        {
            break;
        }
        trimmed = trimmed[..marker_start].trim_end();
    }
    let trimmed = trimmed.trim_end_matches(|ch: char| {
        matches!(
            ch,
            '*' | '_' | '`' | '"' | '\'' | '\u{2019}' | '\u{201d}' | ')' | ']' | '}'
        )
    });
    trimmed
        .chars()
        .last()
        .is_some_and(|ch| !matches!(ch, '.' | '!' | '?'))
}

fn markdown_starts_with_lowercase_prose(text: &str) -> bool {
    text.trim_start()
        .trim_start_matches(|ch: char| {
            matches!(
                ch,
                '*' | '_' | '`' | '"' | '\'' | '\u{201c}' | '\u{2018}' | '(' | '[' | '{'
            )
        })
        .chars()
        .next()
        .is_some_and(char::is_lowercase)
}

#[derive(Default)]
struct HeadingContext {
    last_roman: Option<u16>,
    last_letter: Option<u8>,
    saw_multi_roman: bool,
}

impl HeadingContext {
    fn level(&mut self, text: &str, role: LiquidBlockRole) -> u8 {
        let Some(enumerator) = leading_heading_enumerator(text) else {
            return heading_level(text, role);
        };
        match enumerator {
            HeadingEnumerator::Arabic => 4,
            HeadingEnumerator::Roman { value, len } => {
                if len > 1 {
                    self.last_roman = Some(value);
                    self.saw_multi_roman = true;
                    2
                } else {
                    let letter = text
                        .trim_start()
                        .as_bytes()
                        .first()
                        .copied()
                        .unwrap_or_default();
                    let continues_letters = self
                        .last_letter
                        .is_some_and(|previous| previous.saturating_add(1) == letter);
                    let continues_roman = self.saw_multi_roman
                        && self
                            .last_roman
                            .is_some_and(|previous| previous.saturating_add(1) == value);
                    let prefer_roman = matches!(letter, b'I' | b'V' | b'X');
                    if continues_letters || (!continues_roman && !prefer_roman) {
                        self.last_letter = Some(letter);
                        3
                    } else {
                        self.last_roman = Some(value);
                        2
                    }
                }
            }
            HeadingEnumerator::Letter(letter) => {
                self.last_letter = Some(letter);
                3
            }
            HeadingEnumerator::LowerLetter => {
                if role == LiquidBlockRole::Heading
                    && lowercase_roman_heading_enumerator(text).is_some()
                {
                    2
                } else {
                    4
                }
            }
        }
    }
}

fn heading_level(text: &str, role: LiquidBlockRole) -> u8 {
    match leading_heading_enumerator(text) {
        Some(HeadingEnumerator::Arabic) => 4,
        Some(HeadingEnumerator::Letter(_)) => 3,
        Some(HeadingEnumerator::LowerLetter) => 4,
        Some(HeadingEnumerator::Roman { value, len: 1 }) => {
            let letter = text
                .trim_start()
                .as_bytes()
                .first()
                .copied()
                .unwrap_or_default();
            let _ = value;
            if matches!(letter, b'I' | b'V' | b'X') {
                2
            } else {
                3
            }
        }
        Some(HeadingEnumerator::Roman { .. }) => 2,
        None if matches!(
            text.trim(),
            value if value.eq_ignore_ascii_case("introduction")
                || value.eq_ignore_ascii_case("conclusion")
        ) =>
        {
            2
        }
        None if role == LiquidBlockRole::Subheading => 3,
        None => 2,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HeadingEnumerator {
    Roman { value: u16, len: usize },
    Letter(u8),
    LowerLetter,
    Arabic,
}

fn leading_heading_enumerator(text: &str) -> Option<HeadingEnumerator> {
    let trimmed = text.trim_start();
    let token_end = trimmed.find(char::is_whitespace).unwrap_or(trimmed.len());
    let token = &trimmed[..token_end];
    let (token, dotted) = token
        .strip_suffix('.')
        .map_or((token, false), |token| (token, true));
    if token.is_empty() {
        return None;
    }
    if dotted && token.chars().all(|ch| ch.is_ascii_digit()) {
        return Some(HeadingEnumerator::Arabic);
    }
    if dotted
        && token.len() > 1
        && token.chars().all(|ch| ch.is_ascii_lowercase())
        && let Some(value) = lowercase_roman_heading_enumerator(trimmed)
    {
        return Some(HeadingEnumerator::Roman {
            value,
            len: token.len(),
        });
    }
    if token.chars().all(|ch| ch.is_ascii_uppercase())
        && let Some(value) = roman_value(token)
    {
        return Some(HeadingEnumerator::Roman {
            value,
            len: token.len(),
        });
    }
    if dotted && token.len() == 1 && token.as_bytes()[0].is_ascii_uppercase() {
        return Some(HeadingEnumerator::Letter(token.as_bytes()[0]));
    }
    if dotted && token.len() == 1 && token.as_bytes()[0].is_ascii_lowercase() {
        return Some(HeadingEnumerator::LowerLetter);
    }
    None
}

fn lowercase_roman_heading_enumerator(text: &str) -> Option<u16> {
    let token = text.trim_start().split_whitespace().next()?;
    let token = token.strip_suffix('.')?;
    if token.is_empty()
        || token.len() > 8
        || !token.chars().all(|ch| {
            ch.is_ascii_lowercase() && matches!(ch, 'i' | 'v' | 'x' | 'l' | 'c' | 'd' | 'm')
        })
    {
        return None;
    }
    roman_value(&token.to_ascii_uppercase())
}

fn roman_value(value: &str) -> Option<u16> {
    if value.is_empty()
        || !value
            .chars()
            .all(|ch| matches!(ch, 'I' | 'V' | 'X' | 'L' | 'C' | 'D' | 'M'))
    {
        return None;
    }
    let mut total = 0u16;
    let mut previous = 0u16;
    for ch in value.chars().rev() {
        let current = match ch {
            'I' => 1,
            'V' => 5,
            'X' => 10,
            'L' => 50,
            'C' => 100,
            'D' => 500,
            'M' => 1000,
            _ => return None,
        };
        if current < previous {
            total = total.checked_sub(current)?;
        } else {
            total = total.checked_add(current)?;
            previous = current;
        }
    }
    (roman_string(total).as_deref() == Some(value)).then_some(total)
}

fn roman_string(mut value: u16) -> Option<String> {
    if !(1..=3999).contains(&value) {
        return None;
    }
    let mut out = String::new();
    for (number, numeral) in [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ] {
        while value >= number {
            out.push_str(numeral);
            value -= number;
        }
    }
    Some(out)
}

#[derive(Debug, Clone)]
struct MarkerOccurrence {
    start: usize,
    end: usize,
    marker: u16,
}

/// Markdown footnote identifiers are file-global, while a bound journal volume
/// restarts its printed note numbers for every article.  Keep the familiar
/// numeric label unless the same marker is linked to distinct note blocks.
/// Scope those collisions to their detected articles so a callout in article A
/// cannot silently land on article B's note.  Links whose colliding blocks do
/// not have distinct article provenance are filtered before this function; the
/// block fallback is retained only as a defensive uniqueness guarantee.
fn numeric_footnote_labels(
    document: &LiquidDocument,
    links: &[&LiquidFootnoteLink],
) -> BTreeMap<(u16, usize), String> {
    let mut note_blocks_by_marker = BTreeMap::<u16, BTreeSet<usize>>::new();
    for link in links {
        note_blocks_by_marker
            .entry(link.marker)
            .or_default()
            .insert(link.note_block_index);
    }

    let mut labels = BTreeMap::new();
    for (marker, note_blocks) in note_blocks_by_marker {
        if note_blocks.len() == 1 {
            let note_block_index = *note_blocks.first().expect("one note block");
            labels.insert((marker, note_block_index), marker.to_string());
            continue;
        }

        let proposed = note_blocks
            .iter()
            .map(|note_block_index| {
                let label = markdown_article_index_for_block(document, *note_block_index)
                    .map(|article_index| format!("a{}-{marker}", article_index + 1))
                    .unwrap_or_else(|| format!("n{}-{marker}", note_block_index + 1));
                (*note_block_index, label)
            })
            .collect::<Vec<_>>();
        let proposed_are_unique = proposed
            .iter()
            .map(|(_, label)| label)
            .collect::<BTreeSet<_>>()
            .len()
            == proposed.len();
        for (note_block_index, proposed_label) in proposed {
            let label = if proposed_are_unique {
                proposed_label
            } else {
                format!("n{}-{marker}", note_block_index + 1)
            };
            labels.insert((marker, note_block_index), label);
        }
    }
    labels
}

fn safe_markdown_footnote_links(document: &LiquidDocument) -> Vec<&LiquidFootnoteLink> {
    let mut note_blocks_by_marker = BTreeMap::<u16, BTreeSet<usize>>::new();
    for link in &document.footnote_links {
        note_blocks_by_marker
            .entry(link.marker)
            .or_default()
            .insert(link.note_block_index);
    }
    let mut allowed_note_blocks = BTreeMap::<u16, BTreeSet<usize>>::new();
    for (marker, note_blocks) in note_blocks_by_marker {
        if note_blocks.len() <= 1 {
            allowed_note_blocks.insert(marker, note_blocks);
            continue;
        }
        let articles = note_blocks
            .iter()
            .filter_map(|note_block_index| {
                markdown_article_index_for_block(document, *note_block_index)
            })
            .collect::<BTreeSet<_>>();
        if articles.len() == note_blocks.len() {
            allowed_note_blocks.insert(marker, note_blocks);
            continue;
        }

        // Without distinct article provenance, Markdown cannot represent two
        // definitions with the same printed label safely. Keep only the block
        // supported by the most resolved body references, breaking ties by
        // source order, and leave every competing block unlinked in `## Notes`.
        let preferred = note_blocks
            .iter()
            .copied()
            .max_by_key(|note_block_index| {
                let support = document
                    .footnote_links
                    .iter()
                    .filter(|link| {
                        link.marker == marker && link.note_block_index == *note_block_index
                    })
                    .count();
                let coordinate = markdown_block_coordinate(document, *note_block_index)
                    .unwrap_or((*note_block_index, 0));
                (support, std::cmp::Reverse(coordinate))
            })
            .expect("colliding marker has a note block");
        allowed_note_blocks.insert(marker, BTreeSet::from([preferred]));
    }
    document
        .footnote_links
        .iter()
        .filter(|link| {
            allowed_note_blocks
                .get(&link.marker)
                .is_some_and(|blocks| blocks.contains(&link.note_block_index))
        })
        .collect()
}

fn markdown_article_index_for_block(
    document: &LiquidDocument,
    block_index: usize,
) -> Option<usize> {
    let coordinate = markdown_block_coordinate(document, block_index)?;
    document
        .article_spans
        .iter()
        .find(|span| {
            coordinate >= (span.start_page_index, span.start_line_index)
                && coordinate < (span.end_page_index, span.end_line_index)
        })
        .map(|span| span.article_index)
}

fn markdown_block_coordinate(
    document: &LiquidDocument,
    block_index: usize,
) -> Option<(usize, usize)> {
    document
        .block_source_lines
        .iter()
        .find(|source| source.block_index == block_index)?
        .lines
        .iter()
        .map(|line| (line.page_index, line.line_index))
        .min()
}

#[derive(Default)]
struct PlacementOutcome {
    blocks: BTreeMap<usize, String>,
    attempted: usize,
    failed: usize,
    appended: usize,
}

fn rewrite_inline_blocks(
    document: &LiquidDocument,
    markdown_links: &[&LiquidFootnoteLink],
) -> PlacementOutcome {
    let labels = numeric_footnote_labels(document, markdown_links);
    let mut by_block: BTreeMap<usize, Vec<&LiquidFootnoteLink>> = BTreeMap::new();
    for link in markdown_links {
        by_block
            .entry(link.body_block_index)
            .or_default()
            .push(link);
    }

    let mut outcome = PlacementOutcome::default();
    for (block_index, mut links) in by_block {
        outcome.attempted += links.len();
        let Some(block) = document.blocks.get(block_index) else {
            outcome.failed += links.len();
            continue;
        };
        links.sort_by_key(|link| link.body_marker_ordinal);
        let sentinels = sentinel_marker_occurrences(&block.text);
        let plain = plausible_digit_occurrences(&block.text);
        let occurrences = if sentinels.is_empty() {
            &plain
        } else {
            &sentinels
        };
        let mut replacements = BTreeMap::new();
        for link in &links {
            let ordinal_match = occurrences
                .get(link.body_marker_ordinal)
                .filter(|occurrence| occurrence.marker == link.marker);
            let identity_match = ordinal_match.or_else(|| {
                let mut matching = occurrences
                    .iter()
                    .filter(|occurrence| occurrence.marker == link.marker);
                let occurrence = matching.next()?;
                matching.next().is_none().then_some(occurrence)
            });
            if let Some(occurrence) = identity_match
                && !replacements.contains_key(&occurrence.start)
            {
                let label = labels
                    .get(&(link.marker, link.note_block_index))
                    .cloned()
                    .unwrap_or_else(|| link.marker.to_string());
                replacements.insert(occurrence.start, (occurrence.end, label));
            }
        }

        let placed = replacements.len();
        let missing = links.len().saturating_sub(placed);
        let can_append_missing = occurrences.is_empty()
            && links.iter().all(|link| {
                source_marker_matches(document, block_index, link.body_marker_ordinal, link.marker)
            });
        if can_append_missing {
            let mut rewritten = replace_marker_occurrences(&block.text, &replacements);
            for link in &links {
                let label = labels
                    .get(&(link.marker, link.note_block_index))
                    .cloned()
                    .unwrap_or_else(|| link.marker.to_string());
                rewritten.push(MARKDOWN_MARKER_START);
                rewritten.push_str(&label);
                rewritten.push(MARKDOWN_MARKER_END);
            }
            outcome.appended += missing;
            outcome.blocks.insert(block_index, rewritten);
        } else {
            outcome.failed += missing;
            let rewritten = replace_marker_occurrences(&block.text, &replacements);
            outcome.blocks.insert(block_index, rewritten);
        }
    }
    outcome
}

fn source_marker_matches(
    document: &LiquidDocument,
    block_index: usize,
    ordinal: usize,
    marker: u16,
) -> bool {
    document
        .block_source_lines
        .iter()
        .find(|source| source.block_index == block_index)
        .map(|source| {
            source
                .lines
                .iter()
                .flat_map(|line| sentinel_marker_occurrences(&line.text))
                .nth(ordinal)
                .is_some_and(|occurrence| occurrence.marker == marker)
        })
        .unwrap_or(false)
}

fn replace_marker_occurrences(
    text: &str,
    replacements: &BTreeMap<usize, (usize, String)>,
) -> String {
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0usize;
    for (start, (end, label)) in replacements {
        if *start < cursor || *end > text.len() {
            continue;
        }
        out.push_str(&text[cursor..*start]);
        out.push(MARKDOWN_MARKER_START);
        out.push_str(label);
        out.push(MARKDOWN_MARKER_END);
        cursor = *end;
    }
    out.push_str(&text[cursor..]);
    out
}

fn render_marker_placeholders(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut marker = String::new();
    let mut inside = false;
    let mut just_closed_marker = false;
    for ch in text.chars() {
        match ch {
            MARKDOWN_MARKER_START => {
                inside = true;
                marker.clear();
                just_closed_marker = false;
            }
            MARKDOWN_MARKER_END if inside => {
                out.push_str("[^");
                out.push_str(&marker);
                out.push(']');
                inside = false;
                marker.clear();
                just_closed_marker = true;
            }
            _ if inside => marker.push(ch),
            _ => {
                if just_closed_marker && ch.is_alphabetic() {
                    out.push(' ');
                }
                out.push(ch);
                just_closed_marker = false;
            }
        }
    }
    if inside {
        out.push(MARKDOWN_MARKER_START);
        out.push_str(&marker);
    }
    out
}

fn sentinel_marker_occurrences(text: &str) -> Vec<MarkerOccurrence> {
    let mut occurrences = Vec::new();
    let mut start = None;
    let mut digits = String::new();
    for (index, ch) in text.char_indices() {
        match ch {
            CALLOUT_START => {
                start = Some(index);
                digits.clear();
            }
            CALLOUT_END => {
                if let Some(start) = start.take()
                    && let Ok(marker) = digits.parse::<u16>()
                    && (1..=MAX_NOTE_MARKER).contains(&marker)
                {
                    occurrences.push(MarkerOccurrence {
                        start,
                        end: index + ch.len_utf8(),
                        marker,
                    });
                }
                digits.clear();
            }
            _ if start.is_some() && ch.is_ascii_digit() && digits.len() < 3 => digits.push(ch),
            _ if start.is_some() && !ch.is_whitespace() => {
                start = None;
                digits.clear();
            }
            _ => {}
        }
    }
    occurrences
}

fn plausible_digit_occurrences(text: &str) -> Vec<MarkerOccurrence> {
    let chars = text.char_indices().collect::<Vec<_>>();
    let mut occurrences = Vec::new();
    let mut index = 0usize;
    while index < chars.len() {
        if !chars[index].1.is_ascii_digit() {
            index += 1;
            continue;
        }
        let start_index = index;
        while index < chars.len() && chars[index].1.is_ascii_digit() {
            index += 1;
        }
        let end_index = index;
        let start = chars[start_index].0;
        let end = chars
            .get(end_index)
            .map(|(offset, _)| *offset)
            .unwrap_or(text.len());
        let digits = &text[start..end];
        let previous = start_index.checked_sub(1).map(|value| chars[value].1);
        let next = chars.get(end_index).map(|(_, ch)| *ch);
        let plausible = digits.len() <= 3
            && previous.is_some_and(|ch| !ch.is_whitespace() && !ch.is_ascii_digit())
            && next.is_none_or(|ch| ch.is_whitespace() || ch.is_ascii_punctuation());
        if plausible
            && let Ok(marker) = digits.parse::<u16>()
            && (1..=MAX_NOTE_MARKER).contains(&marker)
        {
            occurrences.push(MarkerOccurrence { start, end, marker });
        }
    }
    occurrences
}

#[derive(Clone, Default)]
struct AuthorNotes {
    by_author_block: BTreeMap<usize, Vec<String>>,
    title_markers: Vec<String>,
    note_blocks: BTreeSet<usize>,
    definitions: Vec<FootnoteDefinition>,
}

fn collect_author_notes(
    document: &LiquidDocument,
    linked_note_indices: &BTreeSet<usize>,
    front_author_indices: &BTreeSet<usize>,
) -> AuthorNotes {
    let mut notes = AuthorNotes::default();
    let mut author_by_marker = BTreeMap::<String, usize>::new();
    for author_index in front_author_indices {
        if !looks_like_front_matter_byline(&document.blocks[*author_index].text) {
            continue;
        }
        for marker in symbolic_marker_occurrences(&document.blocks[*author_index].text)
            .into_iter()
            .map(|occurrence| occurrence.label)
        {
            // Prefer the later, printed byline over an earlier repository-cover
            // duplicate. The printed form is the one carrying the symbols.
            author_by_marker.insert(marker, *author_index);
        }
    }
    let expected_markers = author_by_marker.keys().cloned().collect::<BTreeSet<_>>();
    let mut seen_labels = BTreeSet::new();

    for (index, block) in document.blocks.iter().enumerate() {
        if linked_note_indices.contains(&index) {
            continue;
        }
        if !matches!(
            block.role,
            LiquidBlockRole::Footnote | LiquidBlockRole::Marginalia | LiquidBlockRole::AuthorInfo
        ) {
            continue;
        }

        // Yale and a few other journals print an explicit `AUTHOR.` label
        // instead of a symbol. That is metadata about the preceding byline,
        // not a second author name.
        if block.role == LiquidBlockRole::AuthorInfo
            && let Some(text) = explicit_author_note(&block.text)
            && let Some(author_index) = front_author_indices.range(..index).next_back().copied()
            && seen_labels.insert("author".to_owned())
        {
            notes
                .by_author_block
                .entry(author_index)
                .or_default()
                .push("author".to_owned());
            notes.note_blocks.insert(index);
            notes.definitions.push(FootnoteDefinition {
                label: "author".to_owned(),
                text,
                note_index: index,
            });
            continue;
        }

        let segments = split_symbol_note_definitions(&block.text, &expected_markers);
        if segments.is_empty() {
            continue;
        }

        let first_is_unclaimed_title_note = block.role == LiquidBlockRole::AuthorInfo
            && segments
                .first()
                .is_some_and(|segment| !author_by_marker.contains_key(&segment.label))
            && segments
                .iter()
                .skip(1)
                .any(|segment| author_by_marker.contains_key(&segment.label));
        let fallback_author = front_author_indices.range(..index).next_back().copied();
        let mut assignments = Vec::with_capacity(segments.len());
        for (position, segment) in segments.iter().enumerate() {
            let direct_author = author_by_marker
                .get(&segment.label)
                .copied()
                .filter(|author_index| *author_index < index)
                .filter(|author_index| {
                    author_note_is_on_author_page(document, *author_index, index)
                });
            if let Some(author_index) = direct_author {
                assignments.push(Some(AuthorNoteTarget::Author(author_index)));
            } else if position == 0 && first_is_unclaimed_title_note {
                assignments.push(Some(AuthorNoteTarget::Title));
            } else if segments.len() == 1 {
                assignments.push(
                    fallback_author
                        .filter(|author_index| {
                            author_note_is_on_author_page(document, *author_index, index)
                        })
                        .map(AuthorNoteTarget::Author),
                );
            } else {
                assignments.push(None);
            }
        }
        // Partial consumption would hide the unassigned remainder because the
        // whole source block is skipped once it becomes an author note. Keep
        // the legacy visible-note behavior unless every recovered head has a
        // defensible destination.
        if assignments.iter().any(Option::is_none) {
            continue;
        }

        let mut continuation_blocks = Vec::new();
        let mut segments = segments;
        let copyright_embedded =
            block.role == LiquidBlockRole::AuthorInfo && contains_copyright_notice(&block.text);
        if copyright_embedded
            && segments
                .last()
                .is_some_and(|segment| !author_note_visibly_closed(&segment.text))
        {
            for continuation_index in index + 1..document.blocks.len() {
                let continuation = &document.blocks[continuation_index];
                if matches!(
                    continuation.role,
                    LiquidBlockRole::Heading | LiquidBlockRole::Subheading
                ) && substantive_section_heading(&continuation.text)
                {
                    break;
                }
                if matches!(
                    continuation.role,
                    LiquidBlockRole::Noise
                        | LiquidBlockRole::Contents
                        | LiquidBlockRole::Header
                        | LiquidBlockRole::Footer
                        | LiquidBlockRole::Metadata
                        | LiquidBlockRole::SectionBreak
                ) {
                    continue;
                }
                if continuation.role != LiquidBlockRole::Paragraph {
                    break;
                }
                let continuation_text =
                    normalize_whitespace(&strip_callout_sentinels(&continuation.text));
                if continuation_text.is_empty() {
                    continue;
                }
                let last = segments
                    .last_mut()
                    .expect("nonempty symbol segments checked above");
                last.text.push(' ');
                last.text
                    .push_str(&escape_footnote_text(&continuation_text));
                continuation_blocks.push(continuation_index);
                break;
            }
        }

        notes.note_blocks.insert(index);
        notes.note_blocks.extend(continuation_blocks);
        for (segment, target) in segments.into_iter().zip(assignments) {
            if !seen_labels.insert(segment.label.clone()) {
                continue;
            }
            match target.expect("all author-note assignments checked above") {
                AuthorNoteTarget::Author(author_index) => notes
                    .by_author_block
                    .entry(author_index)
                    .or_default()
                    .push(segment.label.clone()),
                AuthorNoteTarget::Title => notes.title_markers.push(segment.label.clone()),
            }
            notes.definitions.push(FootnoteDefinition {
                label: segment.label,
                text: segment.text,
                note_index: index,
            });
        }
    }
    notes
}

#[derive(Clone, Copy)]
enum AuthorNoteTarget {
    Author(usize),
    Title,
}

#[derive(Clone)]
struct SymbolNoteSegment {
    label: String,
    text: String,
}

fn explicit_author_note(text: &str) -> Option<String> {
    let normalized = normalize_whitespace(&strip_callout_sentinels(text));
    let prefix = normalized.get(.."author.".len())?;
    if !prefix.eq_ignore_ascii_case("author.") {
        return None;
    }
    let text = normalized["author.".len()..].trim();
    (!text.is_empty()).then(|| escape_footnote_text(text))
}

fn contains_copyright_notice(text: &str) -> bool {
    let normalized = normalize_whitespace(&strip_callout_sentinels(text));
    normalized.to_ascii_lowercase().contains("copyright") || normalized.contains('\u{00a9}')
}

fn author_note_is_on_author_page(
    document: &LiquidDocument,
    author_index: usize,
    note_index: usize,
) -> bool {
    !block_page_span(document, author_index)
        .zip(block_page_span(document, note_index))
        .is_some_and(|((_, author_page), (note_page, _))| author_page != note_page)
}

fn split_symbol_note_definitions(
    text: &str,
    expected_markers: &BTreeSet<String>,
) -> Vec<SymbolNoteSegment> {
    let normalized = normalize_whitespace(&strip_callout_sentinels(text));
    let occurrences = symbolic_marker_occurrences(&normalized)
        .into_iter()
        .filter(|occurrence| {
            let leading = occurrence.start == 0;
            let preceded_by_space = normalized[..occurrence.start]
                .chars()
                .next_back()
                .is_some_and(char::is_whitespace);
            let followed_by_boundary = normalized[occurrence.end..]
                .chars()
                .next()
                .is_none_or(|ch| ch.is_whitespace() || matches!(ch, '.' | ')' | ']' | ':'));
            (leading || preceded_by_space)
                && followed_by_boundary
                && (leading || expected_markers.contains(&occurrence.label))
        })
        .collect::<Vec<_>>();
    let mut segments = Vec::new();
    for (position, occurrence) in occurrences.iter().enumerate() {
        let suffix = &normalized[occurrence.end..];
        let content = suffix.trim_start_matches(|ch: char| {
            ch.is_whitespace() || matches!(ch, '.' | ')' | ']' | ':')
        });
        let content_start = occurrence.end + suffix.len().saturating_sub(content.len());
        let content_end = occurrences
            .get(position + 1)
            .map(|next| next.start)
            .unwrap_or(normalized.len());
        if content_start >= content_end {
            continue;
        }
        let text = normalized[content_start..content_end].trim();
        if text.is_empty() {
            continue;
        }
        segments.push(SymbolNoteSegment {
            label: occurrence.label.clone(),
            text: escape_footnote_text(text),
        });
    }
    segments
}

fn author_note_visibly_closed(text: &str) -> bool {
    let text = text
        .trim_end()
        .trim_end_matches(['"', '\'', '\u{2019}', '\u{201d}', ')', ']', '}'])
        .trim_end();
    text.chars()
        .next_back()
        .is_some_and(|ch| matches!(ch, '.' | '!' | '?'))
}

struct InlineNotes {
    definitions: Vec<FootnoteDefinition>,
    unlinked: Vec<String>,
}

#[derive(Clone)]
struct FootnoteDefinition {
    label: String,
    text: String,
    note_index: usize,
}

fn build_inline_notes(
    document: &LiquidDocument,
    note_indices: &[usize],
    author_notes: AuthorNotes,
    markdown_links: &[&LiquidFootnoteLink],
    warnings: &mut Vec<String>,
) -> InlineNotes {
    let labels = numeric_footnote_labels(document, markdown_links);
    let mut definitions = author_notes.definitions;
    let mut links = markdown_links.to_vec();
    links.sort_by_key(|link| (link.note_block_index, link.marker));
    let mut consumed_ranges: BTreeMap<usize, Vec<(usize, usize)>> = BTreeMap::new();
    let mut seen_labels = definitions
        .iter()
        .map(|definition| definition.label.clone())
        .collect::<BTreeSet<_>>();
    let mut markers_by_note =
        links
            .iter()
            .fold(BTreeMap::<usize, Vec<u16>>::new(), |mut markers, link| {
                markers
                    .entry(link.note_block_index)
                    .or_default()
                    .push(link.marker);
                markers
            });
    for markers in markers_by_note.values_mut() {
        markers.sort_unstable();
        markers.dedup();
    }

    // Diagnostic only. A repeated marker is normally a second body reference to
    // the same note, which the dedup below is meant to collapse. It is a
    // deletion only when the repeat points at a *different* note block, which is
    // what a bound volume produces when each article restarts numbering at 1.
    let mut block_for_label = BTreeMap::<String, usize>::new();
    let mut dropped_distinct_notes = 0usize;

    for link in links {
        let label = labels
            .get(&(link.marker, link.note_block_index))
            .cloned()
            .unwrap_or_else(|| link.marker.to_string());
        match block_for_label.get(&label) {
            Some(seen) if *seen != link.note_block_index => dropped_distinct_notes += 1,
            None => {
                block_for_label.insert(label.clone(), link.note_block_index);
            }
            _ => {}
        }
        if !seen_labels.insert(label.clone()) {
            continue;
        }
        let Some(block) = document.blocks.get(link.note_block_index) else {
            warnings.push(format!(
                "footnote {} points to a missing note block",
                link.marker
            ));
            continue;
        };
        let text = note_text_for_marker(
            &block.text,
            link.marker,
            markers_by_note
                .get(&link.note_block_index)
                .map(Vec::as_slice)
                .unwrap_or(&[]),
        );
        if text.is_empty() {
            warnings.push(format!(
                "footnote {} has no readable note text in its head block; a following continuation may supply it",
                link.marker
            ));
        }
        let normalized_block = normalize_whitespace(&strip_callout_sentinels(&block.text));
        if let Some(range) = note_range_for_marker(
            &normalized_block,
            link.marker,
            markers_by_note
                .get(&link.note_block_index)
                .map(Vec::as_slice)
                .unwrap_or(&[]),
        ) {
            consumed_ranges
                .entry(link.note_block_index)
                .or_insert_with(Vec::new)
                .push(range);
        }
        definitions.push(FootnoteDefinition {
            label,
            text,
            note_index: link.note_block_index,
        });
    }
    definitions.sort_by_key(|definition| definition.note_index);

    // A block that was linked but never emitted a definition — because another
    // block claimed its marker first — must still reach the reader. Skipping on
    // `linked_note_indices` below would drop it from the unlinked pass too, and
    // its text would be written nowhere at all.
    let emitted_note_blocks = definitions
        .iter()
        .map(|definition| definition.note_index)
        .collect::<BTreeSet<_>>();

    let mut unlinked = Vec::new();
    let mut continuation_target: Option<(usize, usize)> = None;
    for index in note_indices {
        if emitted_note_blocks.contains(index) || author_notes.note_blocks.contains(index) {
            // Definitions are cut per marker and those cuts do not tile the
            // block. Emit every stretch of the block that no definition
            // claimed, so a slicing gap cannot become a deletion.
            // Only blocks whose definitions reported a consumed range can have
            // a residual computed. A symbol-marked author note reports none, and
            // treating its whole block as unclaimed would emit it twice.
            if let (Some(block), Some(recorded)) =
                (document.blocks.get(*index), consumed_ranges.get(index))
            {
                let normalized = normalize_whitespace(&strip_callout_sentinels(&block.text));
                let mut ranges = recorded.clone();
                ranges.sort_unstable();
                let mut cursor = 0usize;
                for (start, end) in ranges {
                    if start > cursor {
                        let gap = normalized[cursor..start].trim();
                        if gap.chars().any(char::is_alphanumeric) {
                            let appended_to_previous = cursor == 0
                                && continuation_target.is_some_and(
                                    |(previous_index, definition_index)| {
                                        if !note_continuation_follows(
                                            document,
                                            previous_index,
                                            *index,
                                        ) {
                                            return false;
                                        }
                                        let Some(definition) =
                                            definitions.get_mut(definition_index)
                                        else {
                                            return false;
                                        };
                                        append_footnote_continuation(
                                            &mut definition.text,
                                            &escape_footnote_text(gap),
                                        );
                                        true
                                    },
                                );
                            if appended_to_previous {
                                warnings.push(
                                    "a footnote prefix before the next embedded head was appended to the previous definition".to_owned(),
                                );
                            } else {
                                unlinked.push(escape_footnote_text(gap));
                            }
                        }
                    }
                    cursor = cursor.max(end);
                }
                if cursor < normalized.len() {
                    let tail = normalized[cursor..].trim();
                    if tail.chars().any(char::is_alphanumeric) {
                        unlinked.push(escape_footnote_text(tail));
                    }
                }
            }
            let target = definitions
                .iter()
                .enumerate()
                .rev()
                .find(|(_, definition)| definition.note_index == *index)
                .map(|(definition_index, _)| (*index, definition_index));
            continuation_target = target;
            continue;
        }
        let Some(block) = document.blocks.get(*index) else {
            continue;
        };
        let text = normalize_whitespace(&strip_callout_sentinels(&block.text));
        if text.is_empty() {
            continue;
        }
        // Prefer the source-line marker decision when it exists. Text alone is
        // ambiguous here: a continuation can legitimately begin with a section
        // number such as `211(c)(1)`, which must not strand it as a new note.
        // Documents assembled without provenance retain the legacy fallback.
        let has_marker = block_source_note_marker(document, *index)
            .unwrap_or_else(|| leading_numeric_note_marker(&text).is_some())
            || leading_symbol_note(&text).is_some();
        if !has_marker
            && let Some((previous_index, definition_index)) = continuation_target
            && note_continuation_follows(document, previous_index, *index)
            && let Some(definition) = definitions.get_mut(definition_index)
        {
            append_footnote_continuation(&mut definition.text, &escape_footnote_text(&text));
            continuation_target = Some((*index, definition_index));
            warnings.push(
                "a stray footnote continuation was appended to the previous definition".to_owned(),
            );
            continue;
        }
        unlinked.push(escape_footnote_text(&text));
        continuation_target = None;
    }

    if dropped_distinct_notes > 0 {
        warnings.push(format!(
            "{dropped_distinct_notes} note(s) shared a marker number with a different note; they are listed without links rather than dropped"
        ));
    }

    let empty_labels = definitions
        .iter()
        .filter(|definition| definition.text.trim().is_empty())
        .map(|definition| definition.label.clone())
        .collect::<Vec<_>>();
    definitions.retain(|definition| !definition.text.trim().is_empty());
    for label in empty_labels {
        unlinked.push(escape_footnote_text(&label));
    }

    repair_glued_note_boundaries(&mut definitions, &unlinked);

    InlineNotes {
        definitions,
        unlinked,
    }
}

fn append_footnote_continuation(text: &mut String, continuation: &str) {
    let continuation = continuation.trim_start();
    if continuation.is_empty() {
        return;
    }
    if !text.is_empty() {
        let dehyphenate = text.trim_end().ends_with('-')
            && continuation
                .chars()
                .find(|ch| ch.is_alphabetic())
                .is_some_and(char::is_lowercase)
            && !should_preserve_terminal_hyphen(text, continuation);
        if dehyphenate {
            while text.ends_with(char::is_whitespace) || text.ends_with('-') {
                text.pop();
            }
        } else if !text.chars().last().is_some_and(char::is_whitespace) {
            text.push(' ');
        }
    }
    text.push_str(continuation);
}

/// Remove a marker glued to the previous definition only when the same marker
/// survives as a separate unlinked note. Without an exact body occurrence it
/// stays unlinked: placing a synthetic reference after marker N-1 or at the end
/// of the body would silently move or invent a citation.
fn repair_glued_note_boundaries(definitions: &mut [FootnoteDefinition], unlinked: &[String]) {
    let existing: BTreeSet<String> = definitions
        .iter()
        .map(|definition| definition.label.clone())
        .collect();
    for definition in definitions.iter_mut() {
        let Some((kept, marker)) = peel_glued_trailing_note_number(&definition.text) else {
            continue;
        };
        let label = marker.to_string();
        if existing.contains(&label) {
            continue;
        }
        let has_bare_partner = unlinked.iter().any(|note| {
            recover_bare_numeric_note_line(note).is_some_and(|(found, _)| found == marker)
        });
        if !has_bare_partner {
            continue;
        }
        definition.text = kept;
    }
}

/// Whether a block has an explicitly recovered numeric note head in its source
/// provenance. `None` means the block has no provenance and callers should use
/// a textual compatibility fallback; `Some(false)` is affirmative evidence that
/// a leading number belongs to continuation prose rather than a note marker.
fn block_source_note_marker(document: &LiquidDocument, block_index: usize) -> Option<bool> {
    document
        .block_source_lines
        .iter()
        .find(|source| source.block_index == block_index)
        .map(|source| {
            source
                .lines
                .iter()
                .any(|line| !line.note_markers.is_empty())
        })
}

/// Whether an unnumbered note block can continue the preceding definition.
///
/// Block indices are not reading-order adjacency: body flow and page furniture
/// commonly sit between a note at the bottom of one page and its continuation
/// at the bottom of the next. Source-page provenance is the stable boundary.
/// Keep raw adjacency as the compatibility fallback for documents without
/// source-line provenance.
fn note_continuation_follows(
    document: &LiquidDocument,
    previous_index: usize,
    continuation_index: usize,
) -> bool {
    if previous_index.saturating_add(1) == continuation_index {
        return true;
    }
    let Some((_, previous_last_page)) = block_page_span(document, previous_index) else {
        return false;
    };
    let Some((continuation_first_page, _)) = block_page_span(document, continuation_index) else {
        return false;
    };
    if continuation_first_page < previous_last_page
        || continuation_first_page > previous_last_page.saturating_add(1)
    {
        return false;
    }
    same_article(document, previous_index, continuation_index)
}

fn block_page_span(document: &LiquidDocument, block_index: usize) -> Option<(usize, usize)> {
    let source = document
        .block_source_lines
        .iter()
        .find(|source| source.block_index == block_index)?;
    Some((
        source.lines.iter().map(|line| line.page_index).min()?,
        source.lines.iter().map(|line| line.page_index).max()?,
    ))
}

fn same_article(document: &LiquidDocument, left_index: usize, right_index: usize) -> bool {
    if document.article_spans.is_empty() {
        return true;
    }
    let coordinate = |block_index: usize| {
        document
            .block_source_lines
            .iter()
            .find(|source| source.block_index == block_index)
            .and_then(|source| {
                source
                    .lines
                    .iter()
                    .map(|line| (line.page_index, line.line_index))
                    .min()
            })
    };
    let article = |coordinate: (usize, usize)| {
        document.article_spans.iter().find(|span| {
            coordinate >= (span.start_page_index, span.start_line_index)
                && coordinate < (span.end_page_index, span.end_line_index)
        })
    };
    match (coordinate(left_index), coordinate(right_index)) {
        (Some(left), Some(right)) => match (article(left), article(right)) {
            (Some(left), Some(right)) => left.article_index == right.article_index,
            _ => true,
        },
        _ => true,
    }
}

fn append_inline_notes(writer: &mut MarkdownWriter, notes: &InlineNotes) {
    if notes.definitions.is_empty() && notes.unlinked.is_empty() {
        return;
    }
    writer.push("---".to_owned(), BlockJoin::Loose);
    for definition in &notes.definitions {
        writer.push(
            format!("[^{}]: {}", definition.label, definition.text),
            BlockJoin::Loose,
        );
    }
    if !notes.unlinked.is_empty() {
        writer.push("## Notes".to_owned(), BlockJoin::Loose);
        for note in &notes.unlinked {
            writer.push(note.clone(), BlockJoin::Loose);
        }
    }
}

fn collect_endnotes(document: &LiquidDocument, note_indices: &[usize]) -> Vec<String> {
    note_indices
        .iter()
        .filter_map(|index| document.blocks.get(*index))
        .filter_map(|block| {
            let text = normalize_whitespace(&strip_callout_sentinels(&block.text));
            (!text.is_empty()).then(|| escape_footnote_text(&text))
        })
        .collect()
}

fn append_endnotes(writer: &mut MarkdownWriter, notes: &[String]) {
    if notes.is_empty() {
        return;
    }
    writer.push("---".to_owned(), BlockJoin::Loose);
    writer.push("## Notes".to_owned(), BlockJoin::Loose);
    for note in notes {
        writer.push(note.clone(), BlockJoin::Loose);
    }
}

/// The byte range of `text` that [`note_text_for_marker`] turns into a
/// definition, in the coordinates of the normalized text.
///
/// Definitions are cut per marker, and the cuts are not guaranteed to tile the
/// block: a marker already emitted from an earlier block is skipped, and text
/// belonging to notes that never linked sits between the cuts. Reporting the
/// consumed ranges lets the caller emit whatever is left instead of discarding
/// it.
fn note_range_for_marker(
    normalized: &str,
    marker: u16,
    block_markers: &[u16],
) -> Option<(usize, usize)> {
    if block_markers == [marker] && leading_numeric_note_marker(normalized) == Some(marker) {
        return Some((0, normalized.len()));
    }
    let heads = numbered_note_heads(normalized, block_markers);
    let (head_index, (_, content_start)) = heads
        .iter()
        .enumerate()
        .find(|(_, (found, _))| *found == marker)?;
    let content_end = heads
        .get(head_index + 1)
        .map(|(_, start)| note_head_start(normalized, *start))
        .unwrap_or(normalized.len());
    Some((note_head_start(normalized, *content_start), content_end))
}

fn note_text_for_marker(text: &str, marker: u16, block_markers: &[u16]) -> String {
    let normalized = normalize_whitespace(&strip_callout_sentinels(text));
    if block_markers == [marker] && leading_numeric_note_marker(&normalized) == Some(marker) {
        return escape_footnote_text(strip_leading_numeric_marker(&normalized, marker));
    }
    let heads = numbered_note_heads(&normalized, block_markers);
    if let Some((head_index, (_, content_start))) = heads
        .iter()
        .enumerate()
        .find(|(_, (found_marker, _))| *found_marker == marker)
    {
        let content_end = heads
            .get(head_index + 1)
            .map(|(_, start)| note_head_start(&normalized, *start))
            .unwrap_or(normalized.len());
        return escape_footnote_text(normalized[*content_start..content_end].trim());
    }
    escape_footnote_text(strip_leading_numeric_marker(&normalized, marker))
}

fn numbered_note_heads(text: &str, expected: &[u16]) -> Vec<(u16, usize)> {
    let bytes = text.as_bytes();
    let mut heads = Vec::new();
    let mut index = 0usize;
    while index < bytes.len() {
        if !bytes[index].is_ascii_digit() || (index > 0 && !bytes[index - 1].is_ascii_whitespace())
        {
            index += 1;
            continue;
        }
        let digit_start = index;
        while index < bytes.len() && bytes[index].is_ascii_digit() && index - digit_start < 3 {
            index += 1;
        }
        let Ok(marker) = text[digit_start..index].parse::<u16>() else {
            continue;
        };
        let contextual_boundary = text[..digit_start].trim().is_empty() || {
            let mut previous = text[..digit_start].trim_end().chars().rev();
            previous.next().is_some_and(|ch| {
                matches!(ch, '.' | '?' | '!' | ')' | ']')
                    || matches!(ch, '\'' | '"' | '\u{2019}' | '\u{201d}')
                        && previous
                            .next()
                            .is_some_and(|before_quote| matches!(before_quote, '.' | '?' | '!'))
            })
        };
        if !expected.contains(&marker)
            || !contextual_boundary
            || heads
                .last()
                .is_some_and(|(previous_marker, _)| marker <= *previous_marker)
        {
            continue;
        }
        let mut content_start = index;
        if bytes
            .get(content_start)
            .is_some_and(|byte| matches!(*byte, b'.' | b')' | b']'))
        {
            content_start += 1;
        }
        while bytes
            .get(content_start)
            .is_some_and(|byte| byte.is_ascii_whitespace())
        {
            content_start += 1;
        }
        heads.push((marker, content_start));
    }
    heads
}

fn note_head_start(text: &str, content_start: usize) -> usize {
    let bytes = text.as_bytes();
    let mut index = content_start;
    while index > 0 && bytes[index - 1].is_ascii_whitespace() {
        index -= 1;
    }
    while index > 0 && matches!(bytes[index - 1], b'.' | b')' | b']') {
        index -= 1;
    }
    while index > 0 && bytes[index - 1].is_ascii_digit() {
        index -= 1;
    }
    index
}

fn strip_leading_numeric_marker(text: &str, marker: u16) -> &str {
    let trimmed = text.trim_start();
    let marker = marker.to_string();
    let Some(mut rest) = trimmed.strip_prefix(&marker) else {
        return trimmed;
    };
    if rest.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
        return trimmed;
    }
    rest = rest.trim_start_matches(['.', ')', ']', ':']);
    rest.trim_start()
}

fn leading_numeric_note_marker(text: &str) -> Option<u16> {
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

#[derive(Clone)]
struct SymbolMarkerOccurrence {
    start: usize,
    end: usize,
    label: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SymbolMarkerKind {
    Star,
    Dagger,
    DoubleDagger,
}

fn symbol_marker_kind(ch: char) -> Option<SymbolMarkerKind> {
    match ch {
        '*' | '\u{2217}' => Some(SymbolMarkerKind::Star),
        '\u{2020}' => Some(SymbolMarkerKind::Dagger),
        '\u{2021}' => Some(SymbolMarkerKind::DoubleDagger),
        _ => None,
    }
}

fn symbolic_marker_occurrences(text: &str) -> Vec<SymbolMarkerOccurrence> {
    let chars = text.char_indices().collect::<Vec<_>>();
    let mut occurrences = Vec::new();
    let mut position = 0usize;
    while position < chars.len() {
        let (start, ch) = chars[position];
        let Some(kind) = symbol_marker_kind(ch) else {
            position += 1;
            continue;
        };
        let mut end_position = position + 1;
        while end_position < chars.len() && symbol_marker_kind(chars[end_position].1) == Some(kind)
        {
            end_position += 1;
        }
        let end = chars
            .get(end_position)
            .map(|(offset, _)| *offset)
            .unwrap_or(text.len());
        let count = end_position - position;
        let symbol = match kind {
            SymbolMarkerKind::Star => '*',
            SymbolMarkerKind::Dagger => '\u{2020}',
            SymbolMarkerKind::DoubleDagger => '\u{2021}',
        };
        occurrences.push(SymbolMarkerOccurrence {
            start,
            end,
            label: std::iter::repeat(symbol).take(count).collect(),
        });
        position = end_position;
    }
    occurrences
}

fn leading_symbol_note(text: &str) -> Option<(String, String)> {
    let trimmed = text.trim_start();
    let occurrence = symbolic_marker_occurrences(trimmed).into_iter().next()?;
    if occurrence.start != 0 {
        return None;
    }
    let rest = trimmed[occurrence.end..]
        .trim_start_matches(['.', ')', ']', ':'])
        .trim();
    (!rest.is_empty()).then(|| (occurrence.label, escape_footnote_text(rest)))
}

/// Whether a block's text belongs in the notes apparatus.
///
/// Every marginalia block qualifies, because the alternative is deletion.
/// Marginalia is skipped by the body loop, so a block that is not a note
/// candidate appears nowhere at all — and the requirement used to be a link or
/// a recognisable leading note number. On one 1941 volume that discarded 157
/// of 171 marginalia blocks, 27,297 words, including plainly readable body
/// prose that the classifier had merely mislabelled.
///
/// A footnote's continuation lines do not repeat its number, so they never met
/// the old test either. `build_inline_notes` reattaches those to the
/// definition above them and lists whatever is left, which is the right place
/// for text whose role is uncertain: visible to the reader, out of the body
/// flow, and not silently destroyed.
fn is_note_candidate(role: LiquidBlockRole, text: &str, linked: bool) -> bool {
    match role {
        LiquidBlockRole::Footnote | LiquidBlockRole::Marginalia => {
            linked || !marker_only_unlinked_note_head(text)
        }
        // A URL-only footnote can be classified as a caption or body line, but
        // text shape alone is not provenance. Move it into the notes apparatus
        // only when a real body callout was linked; otherwise preserve it in
        // place rather than moving substantive numbered prose or inventing a
        // citation.
        LiquidBlockRole::Caption | LiquidBlockRole::Paragraph => linked,
        // Rejected text that opens with a note number is footnote material the
        // classifier mislabelled. It belongs with the notes, not loose in the
        // body where it reads as a numbered fragment interrupting the prose.
        LiquidBlockRole::Noise => false,
        _ => false,
    }
}

/// Italic or plain URL-only note left in the body/caption stream.
///
/// Matches `*63 Biglaw Investor, … https://…*` and the same text without
/// Markdown italics. Rejects years (`2024 See https://…`) by requiring the
/// whole leading number to be a 1–3 digit marker followed by a title word.
pub(crate) fn numeric_url_note_marker(text: &str) -> Option<u16> {
    recover_numeric_url_note_line(text).map(|(marker, _)| marker)
}

fn recover_numeric_url_note_line(text: &str) -> Option<(u16, String)> {
    let trimmed = text.trim();
    let inner = unwrap_italic_line(trimmed);
    let marker = leading_numeric_note_marker(inner)?;
    let rest = strip_leading_numeric_marker(inner, marker).trim();
    if rest.is_empty()
        || rest.chars().next().is_some_and(|ch| ch.is_ascii_digit())
        || !rest.chars().next().is_some_and(|ch| ch.is_alphabetic())
        || rest.split_whitespace().count() > 24
    {
        return None;
    }
    let has_url = rest.contains("https://") || rest.contains("http://") || rest.contains("www.");
    has_url.then(|| (marker, escape_footnote_text(rest)))
}

fn unwrap_italic_line(text: &str) -> &str {
    let trimmed = text.trim();
    if trimmed.len() > 2 && trimmed.starts_with('*') && trimmed.ends_with('*') {
        trimmed[1..trimmed.len() - 1].trim()
    } else {
        trimmed
    }
}

/// `…conditions.295` → keep `…conditions.` and the glued marker 295.
fn peel_glued_trailing_note_number(text: &str) -> Option<(String, u16)> {
    let trimmed = text.trim_end();
    let digit_start = trimmed
        .char_indices()
        .rev()
        .take_while(|(_, ch)| ch.is_ascii_digit())
        .last()
        .map(|(index, _)| index)?;
    if digit_start == 0 || trimmed.len() - digit_start > 3 {
        return None;
    }
    let marker = trimmed[digit_start..].parse::<u16>().ok()?;
    if !(1..=MAX_NOTE_MARKER).contains(&marker) {
        return None;
    }
    let kept = &trimmed[..digit_start];
    kept.ends_with('.').then(|| (kept.to_owned(), marker))
}

/// Leftover `295 Id. at 682–83 …` line that never became `[^295]:`.
fn recover_bare_numeric_note_line(text: &str) -> Option<(u16, String)> {
    let trimmed = unwrap_italic_line(text);
    let marker = leading_numeric_note_marker(trimmed)?;
    let rest = strip_leading_numeric_marker(trimmed, marker).trim();
    if rest.is_empty() || rest.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
        return None;
    }
    let citation_open = rest.starts_with("Id.")
        || rest.starts_with("id.")
        || rest.starts_with("See ")
        || rest.starts_with("See,")
        || rest.starts_with("Ibid")
        || rest.starts_with("supra")
        || rest.starts_with("Supra");
    citation_open.then(|| (marker, escape_footnote_text(rest)))
}

fn marker_only_unlinked_note_head(text: &str) -> bool {
    let text = normalize_whitespace(&strip_callout_sentinels(text));
    let digits = text.trim_end_matches('.').trim();
    digits
        .parse::<u16>()
        .is_ok_and(|marker| (1..=MAX_NOTE_MARKER).contains(&marker))
}

fn body_flow_role(role: LiquidBlockRole) -> bool {
    matches!(
        role,
        LiquidBlockRole::Paragraph
            | LiquidBlockRole::Lead
            | LiquidBlockRole::Abstract
            | LiquidBlockRole::Syllabus
            | LiquidBlockRole::Quote
            | LiquidBlockRole::ListItem
            | LiquidBlockRole::Explainer
            | LiquidBlockRole::Takeaway
            | LiquidBlockRole::Holding
            | LiquidBlockRole::Issue
            | LiquidBlockRole::Definition
            | LiquidBlockRole::Clause
            | LiquidBlockRole::KeyClause
    )
}

/// Italic case-name fragments can be classified as headings one physical line
/// at a time. A leading footnote callout is conclusive prose evidence. Join
/// these false headings to contiguous body lines on both sides so the output
/// preserves the sentence instead of creating headings or paragraph breaks.
fn misclassified_heading_prose_tight_join_indices(document: &LiquidDocument) -> BTreeSet<usize> {
    let source_by_block = document
        .block_source_lines
        .iter()
        .map(|source| (source.block_index, source))
        .collect::<BTreeMap<_, _>>();
    let mut joins = BTreeSet::new();

    for (index, block) in document.blocks.iter().enumerate() {
        if !matches!(
            block.role,
            LiquidBlockRole::Heading | LiquidBlockRole::Subheading
        ) {
            continue;
        }
        let cleaned = normalize_whitespace(&strip_callout_sentinels(&block.text));
        let false_heading = block.text.trim_start().starts_with(CALLOUT_START)
            || person_name_continuation_misclassified_as_heading(&cleaned)
            || sentence_like_prose_misclassified_as_heading(&cleaned)
            || inline_callout_followed_by_prose(&block.text)
            || (!reads_like_heading(&cleaned)
                && !numbered_outline_heading_without_body(&block.text));
        if !false_heading {
            continue;
        }

        if index > 0
            && contiguous_source_blocks(index - 1, index, &source_by_block)
            && (body_flow_role(document.blocks[index - 1].role)
                || matches!(
                    document.blocks[index - 1].role,
                    LiquidBlockRole::Heading | LiquidBlockRole::Subheading
                ))
        {
            joins.insert(index);
        }
        if index + 1 < document.blocks.len()
            && contiguous_source_blocks(index, index + 1, &source_by_block)
            && (body_flow_role(document.blocks[index + 1].role)
                || matches!(
                    document.blocks[index + 1].role,
                    LiquidBlockRole::Heading | LiquidBlockRole::Subheading
                ))
        {
            joins.insert(index + 1);
        }
    }
    joins
}

fn contiguous_source_blocks(
    previous_index: usize,
    current_index: usize,
    source_by_block: &BTreeMap<usize, &LiquidBlockSourceLines>,
) -> bool {
    let (Some(previous), Some(current)) = (
        source_by_block.get(&previous_index),
        source_by_block.get(&current_index),
    ) else {
        return false;
    };
    let (Some(previous_line), Some(current_line)) = (previous.lines.last(), current.lines.first())
    else {
        return false;
    };
    previous_line.page_index == current_line.page_index
        && previous_line.line_index.checked_add(1) == Some(current_line.line_index)
}

fn cross_page_body_continuation_indices(document: &LiquidDocument) -> BTreeSet<usize> {
    let source_by_block = document
        .block_source_lines
        .iter()
        .map(|source| (source.block_index, source))
        .collect::<BTreeMap<_, _>>();
    let mut joins = BTreeSet::new();
    let mut previous_body: Option<usize> = None;

    for (index, block) in document.blocks.iter().enumerate() {
        if matches!(
            block.role,
            LiquidBlockRole::Paragraph | LiquidBlockRole::Abstract
        ) {
            if let Some(previous_index) = previous_body
                && (document.blocks[previous_index].role == block.role
                    || (block.role == LiquidBlockRole::Paragraph
                        && matches!(
                            document.blocks[previous_index].role,
                            LiquidBlockRole::Heading | LiquidBlockRole::Subheading
                        )
                        && numbered_outline_run_in(&document.blocks[previous_index].text)
                            .is_some()))
                && cross_page_body_continuation(previous_index, index, document, &source_by_block)
            {
                joins.insert(index);
            }
            previous_body = Some(index);
        } else if matches!(
            block.role,
            LiquidBlockRole::Heading | LiquidBlockRole::Subheading
        ) && numbered_outline_run_in(&block.text).is_some()
        {
            // The renderer splits this upstream combined block into a heading
            // and body paragraph. Retain it as a body-continuation candidate
            // so a lowercase next-page Paragraph can join the run-in prose.
            previous_body = Some(index);
        } else if !matches!(
            block.role,
            LiquidBlockRole::Marginalia
                | LiquidBlockRole::Footnote
                | LiquidBlockRole::Header
                | LiquidBlockRole::Footer
                | LiquidBlockRole::Noise
                | LiquidBlockRole::Metadata
                | LiquidBlockRole::SectionBreak
        ) {
            previous_body = None;
        }
    }
    joins
}

/// A reporting clause ending in a colon followed by a contiguous multi-line
/// paragraph is a conservative signal for a displayed quotation. This catches
/// contract forms and statutory extracts that the role model flattened into
/// ordinary prose without treating every paragraph after a colon as quoted.
fn standalone_display_quote_indices(document: &LiquidDocument) -> BTreeSet<usize> {
    let source_by_block = document
        .block_source_lines
        .iter()
        .map(|source| (source.block_index, source))
        .collect::<BTreeMap<_, _>>();
    let mut quotes = BTreeSet::new();

    for index in 1..document.blocks.len() {
        let previous = &document.blocks[index - 1];
        let current = &document.blocks[index];
        let (Some(previous_source), Some(current_source)) = (
            source_by_block.get(&(index - 1)),
            source_by_block.get(&index),
        ) else {
            continue;
        };
        let (Some(previous_line), Some(current_line)) =
            (previous_source.lines.last(), current_source.lines.first())
        else {
            continue;
        };
        let contiguous = previous_line.page_index == current_line.page_index
            && previous_line.line_index.checked_add(1) == Some(current_line.line_index);
        if !contiguous {
            continue;
        }
        let starts_quote = previous.role == LiquidBlockRole::Paragraph
            && introduces_standalone_display_quote(&previous.text)
            && !matches!(
                current.role,
                LiquidBlockRole::Footnote
                    | LiquidBlockRole::Marginalia
                    | LiquidBlockRole::Header
                    | LiquidBlockRole::Footer
                    | LiquidBlockRole::Metadata
                    | LiquidBlockRole::Contents
            )
            && (current_source.lines.len() >= 2 || display_quote_all_caps(&current.text));
        let continues_quote = quotes.contains(&(index - 1))
            && (previous.text.trim_end().ends_with('-') || display_quote_all_caps(&current.text));
        if starts_quote || continues_quote {
            quotes.insert(index);
        }
    }
    quotes
}

fn introduces_standalone_display_quote(text: &str) -> bool {
    let lower = normalize_whitespace(&strip_callout_sentinels(text)).to_ascii_lowercase();
    if !lower.ends_with(':') {
        return false;
    }
    (lower.contains("following")
        && ["agreement", "notice", "provision", "language", "terms"]
            .into_iter()
            .any(|cue| lower.contains(cue)))
        || lower.contains("provides in relevant part:")
        || lower.contains("reads as follows:")
}

fn display_quote_all_caps(text: &str) -> bool {
    let text = normalize_whitespace(&strip_callout_sentinels(text));
    let letters = text.chars().filter(|ch| ch.is_alphabetic()).count();
    letters >= 4 && !text.chars().any(char::is_lowercase)
}

/// Return the reporting phrase used to introduce a displayed quotation when
/// the quotation starts later in the same assembled Paragraph block.
fn inline_display_quote_cues(document: &LiquidDocument) -> BTreeMap<usize, &'static str> {
    let mut cues = BTreeMap::new();
    for source in &document.block_source_lines {
        let Some(block) = document.blocks.get(source.block_index) else {
            continue;
        };
        if block.role != LiquidBlockRole::Paragraph || source.lines.len() < 3 {
            continue;
        }
        for line_index in 0..source.lines.len().saturating_sub(2) {
            let line = &source.lines[line_index];
            let next = &source.lines[line_index + 1];
            let Some(cue) = display_quote_cue(line.text.trim_end()) else {
                continue;
            };
            if line.page_index == next.page_index
                && line.line_index.checked_add(1) == Some(next.line_index)
                && next
                    .text
                    .chars()
                    .find(|ch| ch.is_alphabetic())
                    .is_some_and(char::is_uppercase)
            {
                cues.insert(source.block_index, cue);
                break;
            }
        }
    }
    cues
}

fn display_quote_cue(text: &str) -> Option<&'static str> {
    let lower = text.to_ascii_lowercase();
    ["as follows:", "the following:", "provides:", "reads:"]
        .into_iter()
        .find(|cue| lower.ends_with(cue))
}

fn split_after_case_insensitive<'a>(text: &'a str, cue: &str) -> Option<(&'a str, &'a str)> {
    let start = text.to_ascii_lowercase().find(cue)?;
    let split = start + cue.len();
    let introduction = text[..split].trim_end();
    let quotation = text[split..].trim_start();
    (!introduction.is_empty() && quotation.split_whitespace().count() >= 12)
        .then_some((introduction, quotation))
}

fn cross_page_body_continuation(
    previous_index: usize,
    current_index: usize,
    document: &LiquidDocument,
    source_by_block: &BTreeMap<usize, &LiquidBlockSourceLines>,
) -> bool {
    let Some(previous_source) = source_by_block.get(&previous_index) else {
        return false;
    };
    let Some(current_source) = source_by_block.get(&current_index) else {
        return false;
    };
    let (Some(previous_line), Some(current_line)) =
        (previous_source.lines.last(), current_source.lines.first())
    else {
        return false;
    };
    if previous_line.page_index.checked_add(1) != Some(current_line.page_index) {
        return false;
    }

    let previous = text_without_callout_payload(&document.blocks[previous_index].text);
    let current = text_without_callout_payload(&document.blocks[current_index].text);
    let previous = previous
        .trim_end()
        .trim_end_matches(['"', '\'', '’', '”', ')', ']', '}']);
    let previous_is_open = !previous.is_empty()
        && !matches!(
            previous.chars().last(),
            Some('.') | Some('?') | Some('!') | Some(':') | Some(';')
        );
    let current_begins_lowercase = current
        .trim_start()
        .chars()
        .find(|ch| ch.is_alphabetic())
        .is_some_and(char::is_lowercase);
    let short_sentence_fragment = current.split_whitespace().count() <= 12
        && current
            .trim_end()
            .trim_end_matches(['"', '\'', '\u{2019}', '\u{201d}', ')', ']', '}'])
            .chars()
            .next_back()
            .is_some_and(|ch| matches!(ch, '.' | '?' | '!'));
    previous_is_open && (current_begins_lowercase || short_sentence_fragment)
}

fn text_without_callout_payload(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut inside = false;
    for ch in text.chars() {
        match ch {
            CALLOUT_START => inside = true,
            CALLOUT_END if inside => inside = false,
            _ if !inside => output.push(ch),
            _ => {}
        }
    }
    output
}

fn front_matter_body_index(document: &LiquidDocument) -> usize {
    if let Some(index) = document.blocks.iter().take(24).position(|block| {
        matches!(
            block.role,
            LiquidBlockRole::Heading | LiquidBlockRole::Subheading
        ) && substantive_section_heading(&block.text)
    }) {
        return index;
    }
    document
        .blocks
        .iter()
        .position(|block| body_flow_role(block.role) && !looks_like_contents_block(&block.text))
        .unwrap_or(document.blocks.len())
}

fn substantive_section_heading(text: &str) -> bool {
    let text = strip_star_pagination_markers(text);
    let text = normalize_whitespace(&strip_callout_sentinels(&text));
    leading_heading_enumerator(&text).is_some()
        || matches!(
            text.to_ascii_lowercase().as_str(),
            "introduction" | "conclusion"
        )
}

fn front_matter_genre_label(text: &str) -> bool {
    matches!(
        normalize_whitespace(&strip_callout_sentinels(text))
            .to_ascii_lowercase()
            .as_str(),
        "article" | "articles" | "essay" | "essays" | "note" | "notes" | "comment" | "comments"
    )
}

fn looks_like_contents_block(text: &str) -> bool {
    // Inline footnote callouts are not page locators. Remove both sentinel
    // brackets and their numeric payload before counting contents-like page
    // numbers; otherwise a citation-rich paragraph ending on a callout can be
    // mistaken for a no-dotleader table of contents and deleted wholesale.
    let text = normalize_whitespace(&text_without_callout_payload(text));
    if text.is_empty() {
        return false;
    }
    let lower = text.to_ascii_lowercase();
    let dotleader = text.contains(".....");
    let ends_with_page_locator = text
        .chars()
        .rev()
        .find(|ch| !ch.is_whitespace())
        .is_some_and(|ch| ch.is_ascii_digit());
    let page_locators = text
        .split_whitespace()
        .filter_map(|token| {
            token
                .trim_matches(|ch: char| !ch.is_ascii_digit())
                .parse::<u16>()
                .is_ok()
                .then(|| {
                    token
                        .trim_matches(|ch: char| !ch.is_ascii_digit())
                        .parse::<u16>()
                        .unwrap_or_default()
                })
        })
        .filter(|value| (100..=9999).contains(value))
        .count();
    let explicit_contents = lower.contains("article contents")
        || lower.contains("table of contents")
        || lower.starts_with("contents ");
    dotleader && (explicit_contents || page_locators >= 2 || ends_with_page_locator)
        || explicit_contents && page_locators >= 2
        || page_locators >= 5 && ends_with_page_locator && lower.contains("conclusion ")
}

fn looks_like_front_matter_byline(text: &str) -> bool {
    let text = normalize_whitespace(&strip_callout_sentinels(text));
    let words = text.split_whitespace().count();
    if words == 0 || words > 20 || text.ends_with(['.', ':', ';']) {
        return false;
    }
    let has_author_marker = text
        .chars()
        .any(|ch| matches!(ch, '*' | '\u{2217}' | '\u{2020}' | '\u{2021}'));
    let starts_with_by = text
        .get(..3)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("by "));
    (has_author_marker || starts_with_by)
        && text.chars().filter(|ch| ch.is_alphabetic()).count() >= 4
        || looks_like_plain_person_byline(&text)
}

fn has_trailing_front_matter_author_marker(text: &str) -> bool {
    let text = normalize_whitespace(&strip_callout_sentinels(text));
    text.chars()
        .next_back()
        .is_some_and(|ch| matches!(ch, '*' | '\u{2217}' | '\u{2020}' | '\u{2021}'))
        || text
            .get(..3)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("by "))
}

fn looks_like_repository_cover_block(text: &str) -> bool {
    let lower = normalize_whitespace(&strip_callout_sentinels(text)).to_ascii_lowercase();
    [
        "follow this and additional works at",
        "recommended citation",
        "brought to you for free and open access",
        "accepted for inclusion",
        "review by an authorized administrator",
    ]
    .iter()
    .filter(|cue| lower.contains(**cue))
    .count()
        >= 2
}

fn looks_like_plain_person_byline(text: &str) -> bool {
    let text = normalize_whitespace(&strip_callout_sentinels(text));
    let words = text.split_whitespace().collect::<Vec<_>>();
    if !(2..=8).contains(&words.len())
        || text.ends_with(['.', ':', ';'])
        || text.chars().filter(|ch| ch.is_alphabetic()).count() < 4
    {
        return false;
    }
    let lower = text.to_ascii_lowercase();
    if [
        "article",
        "chapter",
        "introduction",
        "conclusion",
        "abstract",
        "contents",
        "guide",
        "review",
    ]
    .iter()
    .any(|word| lower.split_whitespace().any(|token| token == *word))
    {
        return false;
    }
    let connectors = ["and", "&", "de", "del", "van", "von", "of", "the"];
    let names_are_title_cased = words.iter().all(|word| {
        let word = word.trim_matches(|ch: char| !ch.is_alphanumeric());
        word.is_empty()
            || connectors
                .iter()
                .any(|connector| word.eq_ignore_ascii_case(connector))
            || word.chars().next().is_some_and(|ch| ch.is_uppercase())
    });
    let alphabetic = text.chars().filter(|ch| ch.is_alphabetic()).count();
    let uppercase = text.chars().filter(|ch| ch.is_uppercase()).count();
    names_are_title_cased && uppercase * 100 < alphabetic.max(1) * 80
}

fn author_display_text(text: &str) -> String {
    let normalized = normalize_whitespace(
        &strip_callout_sentinels(text)
            .chars()
            .filter(|ch| !matches!(*ch, '*' | '\u{2217}' | '\u{2020}' | '\u{2021}'))
            .collect::<String>(),
    );
    normalized
        .strip_prefix("author. ")
        .or_else(|| normalized.strip_prefix("Author. "))
        .unwrap_or(&normalized)
        .to_owned()
}

fn render_author_byline(text: &str, markers: &[String]) -> String {
    let normalized = normalize_whitespace(&strip_callout_sentinels(text));
    let expected = markers.iter().cloned().collect::<BTreeSet<_>>();
    let occurrences = symbolic_marker_occurrences(&normalized)
        .into_iter()
        .filter(|occurrence| expected.contains(&occurrence.label))
        .collect::<Vec<_>>();
    if occurrences.is_empty() {
        let text = author_display_text(&normalized);
        let suffix = markers
            .iter()
            .map(|marker| format!("[^{marker}]"))
            .collect::<String>();
        return format!("*{text}*{suffix}");
    }

    let mut rendered = String::new();
    let mut emitted = BTreeSet::new();
    let mut cursor = 0usize;
    for occurrence in occurrences {
        let segment = author_display_text(&normalized[cursor..occurrence.start]);
        if !segment.is_empty() {
            if !rendered.is_empty() {
                rendered.push(' ');
            }
            rendered.push('*');
            rendered.push_str(&segment);
            rendered.push('*');
        }
        if emitted.insert(occurrence.label.clone()) {
            rendered.push_str(&format!("[^{}]", occurrence.label));
        }
        cursor = occurrence.end;
    }
    let tail = author_display_text(&normalized[cursor..]);
    if !tail.is_empty() {
        if !rendered.is_empty() {
            rendered.push(' ');
        }
        rendered.push('*');
        rendered.push_str(&tail);
        rendered.push('*');
    }
    for marker in markers {
        if emitted.insert(marker.clone()) {
            rendered.push_str(&format!("[^{marker}]"));
        }
    }
    rendered
}

fn strip_leading_abstract_label(text: &str) -> &str {
    let trimmed = text.trim_start();
    let Some(prefix) = trimmed.get(.."abstract".len()) else {
        return trimmed;
    };
    if !prefix.eq_ignore_ascii_case("abstract") {
        return trimmed;
    }
    let rest = &trimmed["abstract".len()..];
    if rest.is_empty()
        || rest
            .chars()
            .next()
            .is_some_and(|ch| ch.is_whitespace() || matches!(ch, '.' | ':' | '\u{2014}' | '-'))
    {
        rest.trim_start_matches(|ch: char| {
            ch.is_whitespace() || matches!(ch, '.' | ':' | '\u{2014}' | '-')
        })
    } else {
        trimmed
    }
}

fn redundant_title_heading(title: &str, heading: &str) -> bool {
    let key = |text: &str| {
        normalize_whitespace(
            &text
                .chars()
                .map(|ch| {
                    if ch.is_alphanumeric() {
                        ch.to_ascii_lowercase()
                    } else {
                        ' '
                    }
                })
                .collect::<String>(),
        )
    };
    let title = key(title);
    let mut heading = key(heading);
    for prefix in ["article ", "essay ", "note "] {
        if let Some(remainder) = heading.strip_prefix(prefix) {
            heading = remainder.to_owned();
            break;
        }
    }
    !heading.is_empty()
        && (title == heading
            || (heading.split_whitespace().count() >= 3 && title.contains(&heading))
            || fuzzy_title_prefix_match(&title, &heading))
}

fn fuzzy_title_prefix_match(title: &str, candidate: &str) -> bool {
    let title_words = title.split_whitespace().collect::<Vec<_>>();
    let candidate_words = candidate.split_whitespace().collect::<Vec<_>>();
    if candidate_words.len() < 6 || candidate_words.len() > title_words.len() {
        return false;
    }
    let exact = candidate_words
        .iter()
        .zip(&title_words)
        .filter(|(candidate, title)| candidate == title)
        .count();
    exact * 100 >= candidate_words.len() * 85
}

fn resolved_title(document: &LiquidDocument) -> Option<String> {
    let mut document_title = normalize_whitespace(&document.title);
    let front_matter_end = front_matter_body_index(document);
    for block in document.blocks.iter().take(front_matter_end) {
        if !matches!(
            block.role,
            LiquidBlockRole::Heading | LiquidBlockRole::Subheading
        ) {
            continue;
        }
        let byline = author_display_text(&block.text);
        if !looks_like_plain_person_byline(&byline) || document_title.eq_ignore_ascii_case(&byline)
        {
            continue;
        }
        if document_title
            .to_ascii_lowercase()
            .ends_with(&byline.to_ascii_lowercase())
        {
            let prefix_len = document_title.len().saturating_sub(byline.len());
            let trimmed = document_title[..prefix_len]
                .trim_end_matches([' ', '-', ':', ',', ';'])
                .trim();
            if trimmed.split_whitespace().count() >= 2 {
                document_title = trimmed.to_owned();
                break;
            }
        }
    }
    let title_block = document
        .blocks
        .iter()
        .find(|block| block.role == LiquidBlockRole::Title)
        .map(|block| normalize_whitespace(&block.text))
        .filter(|title| !title.is_empty());
    if document_title.is_empty() {
        return title_block;
    }
    if looks_like_pdf_filename(&document_title) {
        return title_block.or_else(|| {
            Path::new(&document_title)
                .file_stem()
                .and_then(|stem| stem.to_str())
                .map(normalize_whitespace)
                .filter(|title| !title.is_empty())
        });
    }
    Some(document_title)
}

fn looks_like_pdf_filename(text: &str) -> bool {
    Path::new(text)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))
}

fn normalize_and_escape_body(text: &str) -> String {
    let text = strip_star_pagination_markers(text);
    let text = normalize_whitespace(&strip_callout_sentinels(&text));
    render_marker_placeholders(&escape_body_text(&text))
}

/// Westlaw-style star pagination is location metadata, not prose or Markdown
/// emphasis. Remove tokens such as `*740` wherever they occur in body text.
fn strip_star_pagination_markers(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut cursor = 0usize;
    while let Some(relative) = text[cursor..].find('*') {
        let start = cursor + relative;
        let bytes = text.as_bytes();
        let left_bounded = start == 0 || bytes[start - 1].is_ascii_whitespace();
        let mut end = start + 1;
        while end < bytes.len() && bytes[end].is_ascii_digit() && end - start <= 4 {
            end += 1;
        }
        let digit_count = end.saturating_sub(start + 1);
        let right_bounded = end == bytes.len() || bytes[end].is_ascii_whitespace();
        if left_bounded && (2..=4).contains(&digit_count) && right_bounded {
            output.push_str(&text[cursor..start]);
            cursor = end;
        } else {
            output.push_str(&text[cursor..start + 1]);
            cursor = start + 1;
        }
    }
    output.push_str(&text[cursor..]);
    output
}

fn star_paginated_heading(text: &str) -> Option<String> {
    let trimmed = text.trim_start();
    let star = trimmed.strip_prefix('*')?;
    let digits = star.chars().take_while(|ch| ch.is_ascii_digit()).count();
    if !(2..=4).contains(&digits)
        || !star[digits..]
            .chars()
            .next()
            .is_some_and(|ch| ch.is_whitespace())
    {
        return None;
    }
    let cleaned = normalize_whitespace(&strip_star_pagination_markers(trimmed));
    (cleaned.split_whitespace().count() <= 14
        && substantive_section_heading(&cleaned)
        && reads_like_heading(&cleaned))
    .then_some(cleaned)
}

fn numbered_outline_run_in(text: &str) -> Option<(String, String)> {
    let stripped = strip_star_pagination_markers(text);
    let trimmed = stripped.trim_start();
    let marker = trimmed.split_whitespace().next()?;
    let digits = marker.strip_suffix('.')?;
    if digits.is_empty()
        || digits.len() > 2
        || !digits.chars().all(|ch| ch.is_ascii_digit())
        || digits.parse::<u8>().ok()? == 0
    {
        return None;
    }

    let after_marker = &trimmed[marker.len()..];
    let period = [". â€”", ". —", ". -", ".-"]
        .iter()
        .filter_map(|delimiter| after_marker.find(delimiter))
        .min()
        .or_else(|| after_marker.find('.'))?;
    let heading_end = marker.len() + period + 1;
    let heading = trimmed[..heading_end].trim().trim_end_matches('.');
    let mut body = trimmed[heading_end..].trim_start();
    for delimiter in ["—", "-"] {
        if let Some(remainder) = body.strip_prefix(delimiter) {
            body = remainder.trim_start();
            break;
        }
    }
    let heading_words = heading.split_whitespace().count();
    if !(2..=18).contains(&heading_words)
        || heading.chars().count() > 140
        || body.is_empty()
        || heading[marker.len()..]
            .chars()
            .find(|ch| ch.is_alphabetic())
            .is_none_or(char::is_lowercase)
        || !body
            .chars()
            .find(|ch| ch.is_alphabetic())
            .is_some_and(char::is_uppercase)
    {
        return None;
    }
    Some((heading.to_owned(), body.to_owned()))
}

fn standalone_uppercase_roman_outline_heading(text: &str) -> Option<String> {
    let cleaned = normalize_whitespace(&strip_star_pagination_markers(text));
    let HeadingEnumerator::Roman { len, .. } = leading_heading_enumerator(&cleaned)? else {
        return None;
    };
    (len > 1
        && (2..=24).contains(&cleaned.split_whitespace().count())
        && cleaned.chars().count() <= 180
        && !cleaned
            .chars()
            .last()
            .is_some_and(|ch| matches!(ch, '.' | '!' | '?'))
        && reads_like_heading(&cleaned))
    .then_some(cleaned)
}

fn person_name_continuation_misclassified_as_heading(text: &str) -> bool {
    let trimmed = text.trim();
    let marker = trimmed.split_whitespace().next().unwrap_or_default();
    marker.len() == 2
        && marker.ends_with('.')
        && marker.as_bytes()[0].is_ascii_uppercase()
        && trimmed.contains(',')
        && (2..=48).contains(&trimmed.split_whitespace().count())
        && trimmed.chars().count() <= 420
        && ["professor", "judge", "justice", "member", "director"]
            .iter()
            .any(|cue| trimmed.to_ascii_lowercase().contains(cue))
}

/// A short prose sentence can be promoted to a heading when an isolated
/// italic or bold run dominates the line.  Ordinary unnumbered law-review
/// headings rarely contain a finite linking verb in the middle of a six-plus
/// word phrase; require that narrow cue so genuine title-case headings remain
/// untouched.
fn sentence_like_prose_misclassified_as_heading(text: &str) -> bool {
    let trimmed = text.trim();
    let has_finite_verb = trimmed.split(|ch: char| !ch.is_alphabetic()).any(|word| {
        matches!(
            word.to_ascii_lowercase().as_str(),
            "have" | "has" | "is" | "are" | "was" | "were"
        )
    });
    let ordinal_prose_lead = trimmed.split_once(',').is_some_and(|(lead, remainder)| {
        matches!(
            lead.trim().to_ascii_lowercase().as_str(),
            "first" | "second" | "third" | "fourth" | "finally"
        ) && remainder
            .chars()
            .find(|ch| ch.is_alphabetic())
            .is_some_and(char::is_lowercase)
    });
    leading_heading_enumerator(trimmed).is_none()
        && (6..=24).contains(&trimmed.split_whitespace().count())
        && (has_finite_verb || ordinal_prose_lead)
}

/// A real heading may end with a callout, but prose often contains a callout
/// mid-sentence. A heading-classified line with two or more words after a
/// sentinel is body prose and should join its neighbours.
fn inline_callout_followed_by_prose(text: &str) -> bool {
    text.char_indices()
        .filter(|(_, ch)| matches!(*ch, CALLOUT_END | MARKDOWN_MARKER_END))
        .any(|(index, ch)| {
            text.get(index + ch.len_utf8()..).is_some_and(|remainder| {
                strip_callout_sentinels(remainder)
                    .split_whitespace()
                    .filter(|word| word.chars().any(char::is_alphabetic))
                    .take(2)
                    .count()
                    >= 2
            })
        })
}

fn numbered_outline_heading_without_body(text: &str) -> bool {
    let stripped = strip_star_pagination_markers(text);
    let trimmed = stripped.trim();
    let marker = trimmed.split_whitespace().next().unwrap_or_default();
    let digits = marker.strip_suffix('.').unwrap_or_default();
    !digits.is_empty()
        && digits.len() <= 2
        && digits.chars().all(|ch| ch.is_ascii_digit())
        && digits.parse::<u8>().is_ok_and(|number| number > 0)
        && ![".-", ". -", ".—", ". —", ".â€”"]
            .iter()
            .any(|delimiter| trimmed.contains(delimiter))
        && (2..=18).contains(&trimmed.split_whitespace().count())
        && trimmed.chars().count() <= 140
        && !trimmed
            .chars()
            .last()
            .is_some_and(|ch| matches!(ch, '.' | '?' | '!'))
        && trimmed[marker.len()..]
            .chars()
            .find(|ch| ch.is_alphabetic())
            .is_some_and(char::is_uppercase)
}

/// Length in bytes of a leading PDF footnote-separator rule, if `text` opens
/// with one.
///
/// The rule is a run of at least [`FOOTNOTE_SEPARATOR_MIN_RUN`] dash-like
/// glyphs, optionally preceded by whitespace or callout sentinels. Returns the
/// offset just past the run so the caller can keep whatever follows it.
fn footnote_separator_prefix_len(text: &str) -> Option<usize> {
    let mut run = 0usize;
    let mut end = 0usize;
    for (offset, ch) in text.char_indices() {
        match ch {
            CALLOUT_START | CALLOUT_END => continue,
            ch if ch.is_whitespace() && run == 0 => continue,
            '-' | '_' | '\u{2010}'..='\u{2015}' => {
                run += 1;
                end = offset + ch.len_utf8();
            }
            _ => break,
        }
    }
    (run >= FOOTNOTE_SEPARATOR_MIN_RUN).then_some(end)
}

/// Whether a block classified as a heading actually reads like one.
///
/// A section heading is a complete label: it does not trail off in a comma or
/// semicolon, and it is not a case citation. Italic case names inside body and
/// footnote prose are the common false positive, and emitting them as `##`
/// breaks the document outline.
fn reads_like_heading(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    if trimmed.starts_with('§') || trimmed.starts_with("Â§") {
        return false;
    }
    if trimmed.ends_with([',', ';', ':']) {
        return false;
    }
    // "Brown v. Board", "Reed v. Goertz" - a reporter-style party separator in
    // a short line is a citation fragment, not a section title.
    let words = trimmed.split_whitespace().count();
    let lower = trimmed.to_ascii_lowercase();
    let special_section = matches!(
        lower.trim_matches(|ch: char| !ch.is_alphanumeric() && !ch.is_whitespace()),
        "abstract" | "acknowledgments" | "appendix" | "conclusion" | "introduction"
    );
    let has_outline_marker = leading_heading_enumerator(trimmed).is_some();
    if words > 24
        || (!special_section
            && !has_outline_marker
            && trimmed
                .chars()
                .find(|ch| ch.is_alphabetic())
                .is_some_and(char::is_lowercase))
        || (!special_section && !has_outline_marker && trimmed.ends_with('.'))
    {
        return false;
    }
    if words <= 2
        && trimmed
            .split_whitespace()
            .map(|word| {
                word.trim_matches(|ch: char| !ch.is_alphanumeric())
                    .to_ascii_lowercase()
            })
            .all(|word| {
                matches!(
                    word.as_str(),
                    "a" | "an"
                        | "and"
                        | "as"
                        | "at"
                        | "but"
                        | "by"
                        | "for"
                        | "from"
                        | "in"
                        | "is"
                        | "it"
                        | "of"
                        | "on"
                        | "or"
                        | "the"
                        | "to"
                )
            })
    {
        return false;
    }
    if words <= 12
        && trimmed
            .split_whitespace()
            .any(|token| matches!(token, "v." | "v" | "vs."))
    {
        return false;
    }
    true
}

/// Normalised texts that appear on enough pages to be page furniture.
fn repeated_noise_texts(document: &LiquidDocument) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for block in &document.blocks {
        if block.role != LiquidBlockRole::Noise {
            continue;
        }
        let key = normalize_whitespace(&strip_callout_sentinels(&block.text)).to_lowercase();
        if !key.is_empty() {
            *counts.entry(key).or_insert(0usize) += 1;
        }
    }
    counts
}

/// Whether a rejected line is page furniture rather than content.
///
/// Deliberately narrow. Everything it does not recognise is kept, because the
/// cost of keeping a stray running head is one noisy line, while the cost of
/// dropping a paragraph is a hole nothing downstream can repair.
fn is_discardable_furniture(text: &str, repeated: &BTreeMap<String, usize>) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return true;
    }
    // A bare folio, optionally bracketed.
    // A bare number on its own line: a folio, a volume year, or a scanner's
    // accession stamp. Prose does not consist solely of digits.
    let folio = trimmed.trim_matches(|ch: char| matches!(ch, '[' | ']' | '(' | ')' | '.'));
    if !folio.is_empty()
        && folio.len() <= MAX_BARE_NUMBER_LEN
        && folio.chars().all(|ch| ch.is_ascii_digit())
    {
        return true;
    }
    // Repeats across pages and is short enough to be a running head.
    let words = trimmed.split_whitespace().count();
    if words <= FURNITURE_MAX_WORDS
        && repeated
            .get(&trimmed.to_lowercase())
            .is_some_and(|count| *count >= FURNITURE_MIN_REPEATS)
    {
        return true;
    }
    // A table-of-contents entry: title, a run of dot leaders, a page number.
    if trimmed.contains("..") && trimmed.ends_with(|ch: char| ch.is_ascii_digit()) {
        return true;
    }
    // A printed rule.
    trimmed.chars().count() >= 6
        && trimmed
            .chars()
            .all(|ch| matches!(ch, '-' | '_' | ' ') || ('\u{2010}'..='\u{2015}').contains(&ch))
}

fn normalize_heading_text(text: &str) -> String {
    let text = strip_star_pagination_markers(text);
    let text = normalize_whitespace(&strip_callout_sentinels(&text));
    render_marker_placeholders(&text.replace("[^", "\\[^"))
}

fn normalize_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn strip_callout_sentinels(text: &str) -> String {
    text.chars()
        .filter(|ch| !matches!(*ch, CALLOUT_START | CALLOUT_END))
        .collect()
}

fn finalize_markdown(text: String) -> String {
    let text = render_marker_placeholders(&strip_callout_sentinels(&text));
    text.chars()
        .filter(|ch| {
            !matches!(
                *ch,
                CALLOUT_START | CALLOUT_END | MARKDOWN_MARKER_START | MARKDOWN_MARKER_END
            )
        })
        .map(map_symbol_font_glyph)
        .collect()
}

/// Map the Symbol/Wingdings list glyphs that PDFs encode in the private-use
/// area onto real Unicode. Left unmapped they render as tofu in every viewer.
fn map_symbol_font_glyph(ch: char) -> char {
    match ch {
        '\u{F0B7}' | '\u{F0A7}' | '\u{F0B0}' => '\u{2022}', // Symbol bullets
        '\u{F0FC}' => '\u{2713}',                           // Wingdings check
        '\u{F0D8}' | '\u{F0E0}' => '\u{2192}',              // Wingdings arrows
        other => other,
    }
}

fn escape_body_text(text: &str) -> String {
    text.lines()
        .map(escape_body_line)
        .collect::<Vec<_>>()
        .join("\n")
}

fn escape_body_line(line: &str) -> String {
    let mut escaped = line.replace("[^", "\\[^");
    let bytes = escaped.as_bytes();
    let structural = matches!(bytes.first(), Some(b'#' | b'>'))
        || (matches!(bytes.first(), Some(b'-' | b'*' | b'+'))
            && bytes.get(1).is_some_and(|byte| byte.is_ascii_whitespace()))
        || starts_digit_dot_space(bytes);
    if structural {
        escaped.insert(0, '\\');
    }
    escaped
}

fn starts_digit_dot_space(bytes: &[u8]) -> bool {
    let digit_count = bytes
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    digit_count > 0
        && bytes.get(digit_count) == Some(&b'.')
        && bytes
            .get(digit_count + 1)
            .is_some_and(|byte| byte.is_ascii_whitespace())
}

fn escape_footnote_text(text: &str) -> String {
    normalize_whitespace(text).replace("[^", "\\[^")
}

fn render_quote(text: &str) -> String {
    let text = strip_star_pagination_markers(&strip_callout_sentinels(text));
    let lines = text
        .lines()
        .map(normalize_whitespace)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines.is_empty() {
        return String::new();
    }
    lines
        .iter()
        .map(|line| format!("> {}", render_marker_placeholders(&escape_body_line(line))))
        .collect::<Vec<_>>()
        .join("\n")
}

fn compact_liquid_metadata(text: &str) -> String {
    let mut compact = text.trim().to_owned();
    if let Some((_, rest)) = compact.split_once("Contracts Exam - ") {
        compact = rest.trim().to_owned();
    }
    for prefix in [
        "Date:",
        "Source:",
        "Published:",
        "Published",
        "Updated:",
        "Keywords:",
        "Key words:",
        "JEL Classification:",
        "JEL Classifications:",
        "Received:",
        "Accepted:",
        "Revised:",
    ] {
        if let Some(rest) = compact.strip_prefix(prefix) {
            compact = rest.trim().to_owned();
            break;
        }
    }
    compact.replace(" | ", "  ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::liquid::{
        ArticleSpan, LiquidBlock, LiquidBlockSourceLines, LiquidFootnoteLinkIntegrity,
        LiquidSourceLineRef,
    };

    fn block(role: LiquidBlockRole, text: &str) -> LiquidBlock {
        LiquidBlock {
            role,
            text: text.to_owned(),
            label: None,
        }
    }

    fn document(blocks: Vec<LiquidBlock>) -> LiquidDocument {
        LiquidDocument {
            title: "Test Article".to_owned(),
            article_spans: Vec::new(),
            blocks,
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
        }
    }

    fn integrity(landing_rate: f32) -> LiquidFootnoteLinkIntegrity {
        LiquidFootnoteLinkIntegrity {
            detectable_markers: 1,
            landed: usize::from(landing_rate >= 0.9),
            unmatched: usize::from(landing_rate < 0.9),
            ambiguous: 0,
            note_heads: 1,
            landing_rate,
            ambiguous_rate: 0.0,
        }
    }

    fn add_link(
        document: &mut LiquidDocument,
        body_block_index: usize,
        ordinal: usize,
        marker: u16,
        note_block_index: usize,
    ) {
        document.footnote_links.push(LiquidFootnoteLink {
            body_block_index,
            body_marker_ordinal: ordinal,
            marker,
            note_block_index,
            body_page_index: Some(0),
            note_page_index: Some(0),
        });
    }

    #[test]
    fn defaults_match_the_copy_markdown_contract() {
        assert_eq!(
            MarkdownOptions::default(),
            MarkdownOptions {
                footnotes: FootnoteMode::Inline,
                include_tables: true,
                include_metadata: false,
            }
        );
    }

    #[test]
    fn bound_volume_repeated_numbers_get_article_scoped_markdown_labels() {
        let mut document = document(vec![
            block(
                LiquidBlockRole::Paragraph,
                "First article proposition.\u{E000}1\u{E001}",
            ),
            block(LiquidBlockRole::Footnote, "1 First article authority."),
            block(
                LiquidBlockRole::Paragraph,
                "Second article proposition.\u{E000}1\u{E001}",
            ),
            block(LiquidBlockRole::Footnote, "1 Second article authority."),
        ]);
        document.article_spans = vec![
            ArticleSpan {
                article_index: 0,
                start_page_index: 0,
                start_line_index: 0,
                end_page_index: 1,
                end_line_index: 0,
                confidence: 0.99,
                title_hint: None,
                evidence: Vec::new(),
            },
            ArticleSpan {
                article_index: 1,
                start_page_index: 1,
                start_line_index: 0,
                end_page_index: 2,
                end_line_index: 0,
                confidence: 0.99,
                title_hint: None,
                evidence: Vec::new(),
            },
        ];
        document.block_source_lines = (0..4)
            .map(|block_index| LiquidBlockSourceLines {
                block_index,
                lines: vec![LiquidSourceLineRef {
                    id: None,
                    page_index: block_index / 2,
                    line_index: block_index % 2,
                    text: document.blocks[block_index].text.clone(),
                    role: document.blocks[block_index].role,
                    note_markers: Vec::new(),
                }],
            })
            .collect();
        add_link(&mut document, 0, 0, 1, 1);
        add_link(&mut document, 2, 0, 1, 3);
        document.footnote_link_integrity = Some(LiquidFootnoteLinkIntegrity {
            detectable_markers: 2,
            landed: 2,
            unmatched: 0,
            ambiguous: 0,
            note_heads: 2,
            landing_rate: 1.0,
            ambiguous_rate: 0.0,
        });

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert!(export.text.contains("First article proposition.[^a1-1]"));
        assert!(export.text.contains("Second article proposition.[^a2-1]"));
        assert!(export.text.contains("[^a1-1]: First article authority."));
        assert!(export.text.contains("[^a2-1]: Second article authority."));
        assert!(!export.text.contains("[^1]:"));
    }

    #[test]
    fn repeated_number_without_distinct_article_provenance_keeps_only_one_landing() {
        let mut document = document(vec![
            block(
                LiquidBlockRole::Paragraph,
                "One possible callout.\u{E000}1\u{E001}",
            ),
            block(LiquidBlockRole::Footnote, "1 First possible authority."),
            block(
                LiquidBlockRole::Paragraph,
                "Another possible callout.\u{E000}1\u{E001}",
            ),
            block(LiquidBlockRole::Footnote, "1 Second possible authority."),
        ]);
        add_link(&mut document, 0, 0, 1, 1);
        add_link(&mut document, 2, 0, 1, 3);
        document.footnote_link_integrity = Some(LiquidFootnoteLinkIntegrity {
            detectable_markers: 2,
            landed: 2,
            unmatched: 0,
            ambiguous: 0,
            note_heads: 2,
            landing_rate: 1.0,
            ambiguous_rate: 0.0,
        });

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert!(export.text.contains("One possible callout.[^1]"));
        assert!(export.text.contains("Another possible callout."));
        assert!(!export.text.contains("Another possible callout.[^1]"));
        assert!(export.text.contains("[^1]: First possible authority."));
        assert!(export.text.contains("1 Second possible authority."));
        assert!(
            export
                .warnings
                .iter()
                .any(|warning| warning.contains("sharing a printed number"))
        );
    }

    #[test]
    fn heading_levels_cover_roman_letter_arabic_and_plain_headings() {
        let cases = [
            ("I. Introduction", LiquidBlockRole::Heading, 2),
            ("II. Background", LiquidBlockRole::Heading, 2),
            ("XIV Reform", LiquidBlockRole::Heading, 2),
            ("A. Scope", LiquidBlockRole::Heading, 3),
            ("B. Limits", LiquidBlockRole::Subheading, 3),
            ("C. Remedies", LiquidBlockRole::Heading, 3),
            ("D. Damages", LiquidBlockRole::Heading, 3),
            ("1. Rule", LiquidBlockRole::Heading, 4),
            ("a. First application", LiquidBlockRole::Heading, 4),
            ("b. Second application", LiquidBlockRole::Subheading, 4),
            ("Introduction", LiquidBlockRole::Subheading, 2),
            ("Conclusion", LiquidBlockRole::Heading, 2),
            ("Background", LiquidBlockRole::Heading, 2),
            ("Background", LiquidBlockRole::Subheading, 3),
            ("ii. Lowercase parent", LiquidBlockRole::Heading, 2),
            ("iii. Lowercase parent", LiquidBlockRole::Heading, 2),
            ("iv. Lowercase parent", LiquidBlockRole::Heading, 2),
        ];
        for (text, role, expected) in cases {
            assert_eq!(heading_level(text, role), expected, "{text}");
        }
    }

    #[test]
    fn heading_context_resolves_single_character_ambiguity() {
        let mut roman = HeadingContext::default();
        assert_eq!(roman.level("I. First", LiquidBlockRole::Heading), 2);
        assert_eq!(roman.level("II. Second", LiquidBlockRole::Heading), 2);
        assert_eq!(roman.level("III. Third", LiquidBlockRole::Heading), 2);
        assert_eq!(roman.level("IV. Fourth", LiquidBlockRole::Heading), 2);
        assert_eq!(roman.level("V. Fifth", LiquidBlockRole::Heading), 2);

        let mut letters = HeadingContext::default();
        assert_eq!(letters.level("A. First", LiquidBlockRole::Heading), 3);
        assert_eq!(letters.level("B. Second", LiquidBlockRole::Heading), 3);
        assert_eq!(letters.level("C. Third", LiquidBlockRole::Heading), 3);
        assert_eq!(letters.level("D. Fourth", LiquidBlockRole::Heading), 3);

        let mut uncertain = HeadingContext::default();
        assert_eq!(uncertain.level("I. First", LiquidBlockRole::Heading), 2);
        assert_eq!(uncertain.level("C. First", LiquidBlockRole::Heading), 3);

        let mut nested = HeadingContext::default();
        assert_eq!(nested.level("A. First", LiquidBlockRole::Heading), 3);
        assert_eq!(nested.level("a. Nested first", LiquidBlockRole::Heading), 4);
        assert_eq!(
            nested.level("b. Nested second", LiquidBlockRole::Subheading),
            4
        );

        let mut lowercase_parents = HeadingContext::default();
        assert_eq!(
            lowercase_parents.level("i. First parent", LiquidBlockRole::Heading),
            2
        );
        assert_eq!(
            lowercase_parents.level("a. Nested child", LiquidBlockRole::Subheading),
            4
        );
    }

    #[test]
    fn inline_markers_replace_multiple_sentinels_and_marker_at_block_end() {
        let mut document = document(vec![
            block(
                LiquidBlockRole::Paragraph,
                "First.\u{E000}12\u{E001} Second\u{E000}13\u{E001}",
            ),
            block(LiquidBlockRole::Footnote, "12. First authority."),
            block(LiquidBlockRole::Footnote, "13 Second authority."),
        ]);
        add_link(&mut document, 0, 0, 12, 1);
        add_link(&mut document, 0, 1, 13, 2);
        document.footnote_link_integrity = Some(integrity(1.0));

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert!(export.text.contains("First.[^12] Second[^13]"));
        assert!(export.text.contains("[^12]: First authority."));
        assert!(export.text.contains("[^13]: Second authority."));
        assert_eq!(export.footnote_count, 2);
        assert!(export.footnotes_inlined);
    }

    #[test]
    fn inline_marker_inserts_missing_space_before_following_word() {
        let mut document = document(vec![
            block(
                LiquidBlockRole::Paragraph,
                "The historical example played out.\u{E000}418\u{E001}And public opinion matters.",
            ),
            block(LiquidBlockRole::Footnote, "418. Historical authority."),
        ]);
        add_link(&mut document, 0, 0, 418, 1);
        document.footnote_link_integrity = Some(integrity(1.0));

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert!(
            export
                .text
                .contains("played out.[^418] And public opinion matters.")
        );
    }

    #[test]
    fn unique_marker_identity_survives_an_ordinal_shift() {
        let mut document = document(vec![
            block(
                LiquidBlockRole::Paragraph,
                "Enumerated text.\u{E000}1\u{E001} Supported claim.\u{E000}98\u{E001}",
            ),
            block(LiquidBlockRole::Footnote, "98. Controlling authority."),
        ]);
        // Upstream semantic filtering can remove the first raw occurrence
        // after links were resolved. A unique marker identity remains a safe
        // placement key even when its raw ordinal has shifted.
        add_link(&mut document, 0, 0, 98, 1);
        document.footnote_link_integrity = Some(integrity(1.0));

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert!(export.footnotes_inlined);
        assert!(export.text.contains("Supported claim.[^98]"));
        assert!(export.text.contains("Enumerated text.1"));
    }

    #[test]
    fn legacy_digit_matching_ignores_years_and_spaced_page_cites() {
        let mut document = document(vec![
            block(
                LiquidBlockRole::Paragraph,
                "The 2020 article cites 304 U.S. 64. This proposition.12",
            ),
            block(LiquidBlockRole::Footnote, "12 Authority."),
        ]);
        add_link(&mut document, 0, 0, 12, 1);
        document.footnote_link_integrity = Some(integrity(1.0));

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert!(export.text.contains("2020 article cites 304 U.S. 64."));
        assert!(export.text.contains("proposition.[^12]"));
    }

    #[test]
    fn source_sentinels_allow_missing_block_markers_to_append_safely() {
        let mut document = document(vec![
            block(LiquidBlockRole::Paragraph, "A cleaned paragraph."),
            block(LiquidBlockRole::Footnote, "7 Authority."),
        ]);
        add_link(&mut document, 0, 0, 7, 1);
        document.footnote_link_integrity = Some(integrity(1.0));
        document.block_source_lines = vec![LiquidBlockSourceLines {
            block_index: 0,
            lines: vec![LiquidSourceLineRef {
                id: None,
                page_index: 0,
                line_index: 0,
                text: "A cleaned paragraph.\u{E000}7\u{E001}".to_owned(),
                role: LiquidBlockRole::Paragraph,
                note_markers: Vec::new(),
            }],
        }];

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert!(export.text.contains("A cleaned paragraph.[^7]"));
        assert!(
            export
                .warnings
                .iter()
                .any(|warning| warning.contains("appended"))
        );
    }

    #[test]
    fn low_integrity_and_failed_placements_use_the_endnotes_fallback() {
        let mut low_integrity = document(vec![
            block(LiquidBlockRole::Paragraph, "Claim.\u{E000}4\u{E001}"),
            block(LiquidBlockRole::Footnote, "4 Authority."),
        ]);
        add_link(&mut low_integrity, 0, 0, 4, 1);
        low_integrity.footnote_link_integrity = Some(integrity(0.10));
        let export = liquid_document_markdown(&low_integrity, &MarkdownOptions::default());
        assert!(!export.footnotes_inlined);
        assert!(export.text.contains("## Notes"));
        assert!(export.text.contains("4 Authority."));
        assert!(
            export
                .warnings
                .contains(&LOW_LINK_CONFIDENCE_WARNING.to_owned())
        );

        let mut failed = low_integrity;
        failed.blocks[0].text = "Claim without marker.".to_owned();
        failed.footnote_link_integrity = Some(integrity(1.0));
        let export = liquid_document_markdown(&failed, &MarkdownOptions::default());
        assert!(!export.footnotes_inlined);
        assert!(export.text.contains("## Notes"));
    }

    /// Unmatched markers cost reach, not correctness, so a partially linked
    /// document still links. An ambiguous match can attach a citation to the
    /// wrong sentence, so it falls back to endnotes.
    #[test]
    fn partial_coverage_links_but_ambiguity_falls_back() {
        let mut partial = document(vec![
            block(LiquidBlockRole::Paragraph, "Claim.\u{E000}4\u{E001}"),
            block(LiquidBlockRole::Footnote, "4 Authority."),
        ]);
        add_link(&mut partial, 0, 0, 4, 1);
        let mut under_linked = integrity(0.30);
        under_linked.ambiguous_rate = 0.0;
        partial.footnote_link_integrity = Some(under_linked);
        let export = liquid_document_markdown(&partial, &MarkdownOptions::default());
        assert!(export.footnotes_inlined);
        assert!(
            !export
                .warnings
                .contains(&LOW_LINK_CONFIDENCE_WARNING.to_owned())
        );

        let mut ambiguous = integrity(0.98);
        ambiguous.ambiguous_rate = 0.10;
        partial.footnote_link_integrity = Some(ambiguous);
        let export = liquid_document_markdown(&partial, &MarkdownOptions::default());
        assert!(!export.footnotes_inlined);
        assert!(export.text.contains("## Notes"));
    }

    #[test]
    fn all_footnote_modes_have_distinct_output() {
        let mut document = document(vec![
            block(LiquidBlockRole::Paragraph, "Claim.\u{E000}1\u{E001}"),
            block(LiquidBlockRole::Footnote, "1 Authority."),
        ]);
        add_link(&mut document, 0, 0, 1, 1);
        document.footnote_link_integrity = Some(integrity(1.0));

        let inline = liquid_document_markdown(&document, &MarkdownOptions::default());
        assert!(inline.text.contains("[^1]"));
        assert!(inline.text.contains("[^1]: Authority."));

        let endnotes = liquid_document_markdown(
            &document,
            &MarkdownOptions {
                footnotes: FootnoteMode::Endnotes,
                ..MarkdownOptions::default()
            },
        );
        assert!(endnotes.text.contains("Claim.1"));
        assert!(endnotes.text.contains("## Notes"));
        assert!(!endnotes.text.contains("[^1]:"));

        let omitted = liquid_document_markdown(
            &document,
            &MarkdownOptions {
                footnotes: FootnoteMode::Omit,
                ..MarkdownOptions::default()
            },
        );
        assert!(omitted.text.contains("Claim.1"));
        assert!(!omitted.text.contains("Authority."));
        assert_eq!(omitted.footnote_count, 0);
    }

    #[test]
    fn body_escaping_is_deliberately_limited() {
        let document = document(vec![
            block(LiquidBlockRole::Paragraph, "# Heading-like body"),
            block(LiquidBlockRole::Paragraph, "> Quote-like body"),
            block(LiquidBlockRole::Paragraph, "- List-like body"),
            block(LiquidBlockRole::Paragraph, "12. Number-like body"),
            block(
                LiquidBlockRole::Paragraph,
                "Keep § 2, mid_word, and *stars*.",
            ),
            block(LiquidBlockRole::Paragraph, "Literal [^collision] marker."),
        ]);
        let export = liquid_document_markdown(
            &document,
            &MarkdownOptions {
                footnotes: FootnoteMode::Omit,
                ..MarkdownOptions::default()
            },
        );
        assert!(export.text.contains("\\# Heading-like body"));
        assert!(export.text.contains("\\> Quote-like body"));
        assert!(export.text.contains("\\- List-like body"));
        assert!(export.text.contains("\\12. Number-like body"));
        assert!(export.text.contains("Keep § 2, mid_word, and *stars*."));
        assert!(export.text.contains("Literal \\[^collision] marker."));
    }

    #[test]
    fn final_export_strips_private_marker_sentinels_from_fenced_tables() {
        let document = document(vec![block(
            LiquidBlockRole::Table,
            "Cell\u{E000}3\u{E001} Other\u{E100}4\u{E101} Broken\u{E100}5",
        )]);

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert!(export.text.contains("Cell3 Other[^4] Broken5"));
        assert!(
            !export
                .text
                .chars()
                .any(|ch| matches!(ch, '\u{E000}' | '\u{E001}' | '\u{E100}' | '\u{E101}'))
        );
    }

    /// Italic case names inside prose are the common false heading. They must
    /// render as body text, while real section titles keep their level.
    #[test]
    fn citation_fragments_do_not_become_headings() {
        let document = document(vec![
            block(LiquidBlockRole::Heading, "I. INTRODUCTION"),
            block(LiquidBlockRole::Heading, "Raich,"),
            block(LiquidBlockRole::Heading, "Alito in Reed v. Goertz,"),
            block(LiquidBlockRole::Heading, "Massachusetts v. Feeney"),
            block(LiquidBlockRole::Paragraph, "Body."),
        ]);

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert!(export.text.contains("# I. INTRODUCTION"));
        assert!(export.text.contains("Raich,"));
        assert!(!export.text.contains("# Raich,"));
        assert!(!export.text.contains("# Alito in Reed v. Goertz,"));
        assert!(!export.text.contains("# Massachusetts v. Feeney"));
    }

    #[test]
    fn standalone_roman_paragraph_becomes_main_heading() {
        let document = document(vec![block(
            LiquidBlockRole::Paragraph,
            "II. The Common Law of Quorums",
        )]);

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert!(export.text.contains("## II. The Common Law of Quorums"));
    }

    #[test]
    fn person_name_continuation_is_not_an_outline_heading() {
        assert!(person_name_continuation_misclassified_as_heading(
            "H. Webster, Professor Frank Remington, and Professor Wayne LaFave jointly stated that the conflicts rules were consistent with Harris, in that they recognize and affirm the discretionary power of courts to use their judgment in promoting the ends of justice."
        ));
        assert!(!person_name_continuation_misclassified_as_heading(
            "H. Historical Development"
        ));
    }

    #[test]
    fn short_prose_sentences_are_not_rendered_as_unnumbered_headings() {
        assert!(sentence_like_prose_misclassified_as_heading(
            "Professors Timothy Meyer and Ganesh Sitaraman have recently termed this strategy legalistic noncompliance"
        ));
        assert!(sentence_like_prose_misclassified_as_heading(
            "Third, some scholars—notably Macey and Strine—have defended statutes of this kind"
        ));
        assert!(!sentence_like_prose_misclassified_as_heading(
            "Third, Some Important Implications for Administrative Law"
        ));
        assert!(!sentence_like_prose_misclassified_as_heading(
            "IV. Courts Have Long Resisted Political Pressure"
        ));
    }

    #[test]
    fn ordinal_prose_fragment_without_verb_is_not_a_heading() {
        assert!(sentence_like_prose_misclassified_as_heading(
            "Third, some scholars-notably Jack Balkin, Katharine Bartlett, Felipe"
        ));
    }

    #[test]
    fn heading_line_with_mid_sentence_callout_is_prose() {
        let text = format!(
            "Sides With Wrongly Deported Migrant.{CALLOUT_START}311{CALLOUT_END} In the USAID case, the headline"
        );
        assert!(inline_callout_followed_by_prose(&text));

        let linked_text = format!(
            "Sides With Wrongly Deported Migrant.{MARKDOWN_MARKER_START}311{MARKDOWN_MARKER_END} In the USAID case, the headline"
        );
        assert!(inline_callout_followed_by_prose(&linked_text));

        let true_heading =
            format!("IV. The Rule Against Legalistic Noncompliance{CALLOUT_START}311{CALLOUT_END}");
        assert!(!inline_callout_followed_by_prose(&true_heading));
    }

    #[test]
    fn final_export_omits_standalone_footnote_separator_rules() {
        let separator = "\u{2013}".repeat(61);
        let document = document(vec![
            block(LiquidBlockRole::Paragraph, "Body paragraph."),
            block(LiquidBlockRole::Paragraph, &separator),
            block(LiquidBlockRole::SectionBreak, &separator),
            block(LiquidBlockRole::Paragraph, "Following paragraph."),
        ]);

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert!(export.text.contains("Body paragraph."));
        assert!(!export.text.contains(&separator));
        assert!(export.text.contains("***"));
        assert!(export.text.contains("Following paragraph."));
        assert!(
            export
                .warnings
                .iter()
                .any(|warning| warning.contains("omitted 1 standalone"))
        );
    }

    /// A block that opens with the separator rule and then continues into body
    /// prose must keep the prose. Discarding the whole block dropped 11,463
    /// tokens across the five-article review corpus.
    #[test]
    fn final_export_keeps_body_text_after_a_leading_footnote_separator() {
        let separator = "\u{2013}".repeat(61);
        let document = document(vec![
            block(LiquidBlockRole::Paragraph, "Body paragraph."),
            block(
                LiquidBlockRole::Paragraph,
                &format!(
                    "{separator} In keeping with the analysis above, we will \
                     describe the relevant protected characteristic here as \
                     racial Jewishness."
                ),
            ),
            block(LiquidBlockRole::Paragraph, "Following paragraph."),
        ]);

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert!(export.text.contains("In keeping with the analysis above"));
        assert!(export.text.contains("racial Jewishness."));
        assert!(!export.text.contains(&separator));
        assert!(export.text.contains("Following paragraph."));
        assert!(
            export.warnings.iter().any(
                |warning| warning.contains("stripped a leading footnote-separator rule from 1")
            )
        );
    }

    #[test]
    fn symbol_author_note_attaches_to_the_byline() {
        let document = document(vec![
            block(LiquidBlockRole::Title, "Test Article"),
            block(LiquidBlockRole::AuthorInfo, "Ada Scholar"),
            block(LiquidBlockRole::Footnote, "* Thanks to the editors."),
            block(LiquidBlockRole::Paragraph, "Body."),
        ]);
        let mut document = document;
        document.footnote_link_integrity = Some(integrity(1.0));

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert!(export.text.contains("*Ada Scholar*[^*]"));
        assert!(export.text.contains("[^*]: Thanks to the editors."));
    }

    #[test]
    fn merged_star_notes_split_and_land_at_their_combined_byline_markers() {
        let document = document(vec![
            block(LiquidBlockRole::Title, "Habeas Class Actions"),
            block(
                LiquidBlockRole::Heading,
                "Lee Kovarsky\u{2217} & D. Theodore Rave\u{2217}\u{2217}",
            ),
            block(
                LiquidBlockRole::Marginalia,
                "\u{2217} Lee Kovarsky is a Visiting Professor of Law. \u{2217}\u{2217} D. Theodore Rave is the Ward Centennial Professor. Thanks to the editors.",
            ),
            block(LiquidBlockRole::Paragraph, "Body."),
        ]);

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert!(
            export
                .text
                .contains("*Lee Kovarsky*[^*] *& D. Theodore Rave*[^**]")
        );
        assert!(
            export
                .text
                .contains("[^*]: Lee Kovarsky is a Visiting Professor of Law.")
        );
        assert!(export.text.contains(
            "[^**]: D. Theodore Rave is the Ward Centennial Professor. Thanks to the editors."
        ));
        assert!(!export.text.contains("Law. \u{2217}\u{2217} D. Theodore"));
    }

    #[test]
    fn merged_mixed_symbol_notes_map_to_separate_repeated_bylines() {
        let document = document(vec![
            block(LiquidBlockRole::Title, "Expressive Association at Work"),
            block(LiquidBlockRole::Heading, "Elizabeth Sepper"),
            block(LiquidBlockRole::Heading, "James D. Nelson"),
            block(LiquidBlockRole::Heading, "Charlotte Garden"),
            block(LiquidBlockRole::Heading, "Elizabeth Sepper*"),
            block(LiquidBlockRole::Heading, "James D. Nelson**"),
            block(LiquidBlockRole::Heading, "Charlotte Garden\u{2020}"),
            block(
                LiquidBlockRole::Marginalia,
                "* First professorship. ** Second professorship. \u{2020} Third professorship. We thank the workshop participants.",
            ),
            block(LiquidBlockRole::Paragraph, "Body."),
        ]);

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert!(export.text.contains("*Elizabeth Sepper*[^*]"));
        assert!(export.text.contains("*James D. Nelson*[^**]"));
        assert!(export.text.contains("*Charlotte Garden*[^\u{2020}]"));
        assert_eq!(export.text.matches("Elizabeth Sepper").count(), 1);
        assert_eq!(export.text.matches("James D. Nelson").count(), 1);
        assert_eq!(export.text.matches("Charlotte Garden").count(), 1);
        assert!(export.text.contains("[^*]: First professorship."));
        assert!(export.text.contains("[^**]: Second professorship."));
        assert!(
            export
                .text
                .contains("[^\u{2020}]: Third professorship. We thank the workshop participants.")
        );
    }

    #[test]
    fn copyright_prefixed_dagger_notes_split_and_attach_to_joint_byline() {
        let document = document(vec![
            block(LiquidBlockRole::Title, "Legalistic Noncompliance"),
            block(
                LiquidBlockRole::Heading,
                "Daniel T. Deacon\u{2020} & Leah M. Litman\u{2020}\u{2020}",
            ),
            block(
                LiquidBlockRole::Marginalia,
                "Copyright \u{00a9} 2026 Daniel T. Deacon & Leah M. Litman. \u{2020} Assistant Professor of Law. \u{2020}\u{2020} Professor of Law. For helpful comments, we thank our colleagues.",
            ),
            block(LiquidBlockRole::Paragraph, "Body."),
        ]);

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert!(
            export
                .text
                .contains("*Daniel T. Deacon*[^\u{2020}] *& Leah M. Litman*[^\u{2020}\u{2020}]")
        );
        assert!(
            export
                .text
                .contains("[^\u{2020}]: Assistant Professor of Law.")
        );
        assert!(export.text.contains(
            "[^\u{2020}\u{2020}]: Professor of Law. For helpful comments, we thank our colleagues."
        ));
        assert!(!export.text.contains("Copyright"));
    }

    #[test]
    fn leading_article_note_attaches_to_title_before_dagger_author_notes() {
        let document = document(vec![
            block(
                LiquidBlockRole::Title,
                "Making the Party Presentation Principle Safe for Originalism",
            ),
            block(
                LiquidBlockRole::Heading,
                "Randy E. Barnett\u{2020} & Lawrence B. Solum\u{2020}\u{2020}",
            ),
            block(
                LiquidBlockRole::AuthorInfo,
                "* This Article was previously entitled Originalism and the Party Presentation Principle. \u{2020} Patrick Hotung Professor of Constitutional Law. \u{2020}\u{2020} Distinguished Professor of Law.",
            ),
            block(LiquidBlockRole::Paragraph, "Body."),
        ]);

        let mut document = document;
        document.title = "Making the Party Presentation Principle Safe for Originalism".to_owned();
        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert!(
            export
                .text
                .starts_with("# Making the Party Presentation Principle Safe for Originalism[^*]")
        );
        assert!(
            export
                .text
                .contains("*Randy E. Barnett*[^\u{2020}] *& Lawrence B. Solum*[^\u{2020}\u{2020}]")
        );
        assert!(export.text.contains(
            "[^*]: This Article was previously entitled Originalism and the Party Presentation Principle."
        ));
        assert!(
            export
                .text
                .contains("[^\u{2020}]: Patrick Hotung Professor of Constitutional Law.")
        );
        assert!(
            export
                .text
                .contains("[^\u{2020}\u{2020}]: Distinguished Professor of Law.")
        );
        assert!(!export.text.contains("*This Article was previously"));
    }

    #[test]
    fn explicit_author_label_becomes_a_named_note_not_a_second_byline() {
        let document = document(vec![
            block(LiquidBlockRole::Title, "Cross-Sovereign Policing"),
            block(LiquidBlockRole::AuthorInfo, "Nadia Banteka"),
            block(LiquidBlockRole::Abstract, "Abstract text."),
            block(LiquidBlockRole::Noise, "THE YALE LAW JOURNAL 135:2955"),
            block(
                LiquidBlockRole::AuthorInfo,
                "AUTHOR. Gary & Sallyn Pajcic Professor, Florida State University College of Law. I thank the editors.",
            ),
            block(LiquidBlockRole::Paragraph, "Body."),
        ]);

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert!(export.text.contains("*Nadia Banteka*[^author]"));
        assert!(export.text.contains(
            "[^author]: Gary & Sallyn Pajcic Professor, Florida State University College of Law. I thank the editors."
        ));
        assert!(!export.text.contains("*Gary & Sallyn Pajcic Professor"));
    }

    #[test]
    fn embedded_copyright_author_note_absorbs_front_matter_continuation() {
        let mut document = document(vec![
            block(LiquidBlockRole::Title, "Bankruptcy v. MDL"),
            block(LiquidBlockRole::Heading, "D. Theodore Rave*"),
            block(
                LiquidBlockRole::AuthorInfo,
                "Copyright © 2026 D. Theodore Rave * Professor of Law. Thanks to Jonathan",
            ),
            block(LiquidBlockRole::Noise, "840 LAW REVIEW CONTENTS 841"),
            block(
                LiquidBlockRole::Paragraph,
                "Lipson and Angela Littwin for helpful comments and discussions.",
            ),
            block(LiquidBlockRole::Heading, "INTRODUCTION"),
            block(LiquidBlockRole::Paragraph, "The Article begins here."),
        ]);
        document.footnote_link_integrity = Some(integrity(1.0));

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert!(export.text.contains("*D. Theodore Rave*[^*]"));
        assert!(export.text.contains(
            "[^*]: Professor of Law. Thanks to Jonathan Lipson and Angela Littwin for helpful comments and discussions."
        ));
        assert!(!export.text.contains("*Copyright"));
        assert_eq!(export.text.matches("Lipson and Angela Littwin").count(), 1);
    }

    #[test]
    fn abstract_label_is_not_repeated_and_split_abstract_blocks_stay_separate() {
        let document = document(vec![
            block(
                LiquidBlockRole::Abstract,
                "abstract. The opening abstract paragraph ends here.",
            ),
            block(
                LiquidBlockRole::Abstract,
                "This Article begins a second abstract paragraph.",
            ),
        ]);

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert_eq!(export.text.matches("## Abstract").count(), 1);
        assert!(!export.text.contains("abstract. The opening"));
        assert!(export.text.contains(
            "The opening abstract paragraph ends here.\n\nThis Article begins a second abstract paragraph."
        ));
    }

    #[test]
    fn cross_page_abstract_sentence_joins_across_author_note_and_furniture() {
        let mut document = document(vec![
            block(LiquidBlockRole::AuthorInfo, "Rachel Bayefsky*"),
            block(
                LiquidBlockRole::Abstract,
                "Why integrate practices from eras in which women were subject to severe political, economic, and social",
            ),
            block(
                LiquidBlockRole::Marginalia,
                "* Associate Professor of Law. Thanks to the editors.",
            ),
            block(
                LiquidBlockRole::Noise,
                "866 Virginia Law Review [Vol. 112:865",
            ),
            block(
                LiquidBlockRole::Abstract,
                "subordination? Yet the relationship depends on the form tradition takes.",
            ),
        ]);
        let source =
            |block_index, page_index, line_index, text: &str, role| LiquidBlockSourceLines {
                block_index,
                lines: vec![LiquidSourceLineRef {
                    id: Some(format!("p{page_index}:l{line_index}")),
                    page_index,
                    line_index,
                    text: text.to_owned(),
                    role,
                    note_markers: Vec::new(),
                }],
            };
        document.block_source_lines = vec![
            source(
                1,
                0,
                19,
                &document.blocks[1].text,
                LiquidBlockRole::Abstract,
            ),
            source(
                2,
                0,
                20,
                &document.blocks[2].text,
                LiquidBlockRole::Marginalia,
            ),
            source(3, 1, 0, &document.blocks[3].text, LiquidBlockRole::Noise),
            source(4, 1, 2, &document.blocks[4].text, LiquidBlockRole::Abstract),
        ];

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert_eq!(export.text.matches("## Abstract").count(), 1);
        assert!(
            export
                .text
                .contains("economic, and social subordination? Yet the relationship depends")
        );
        assert!(!export.text.contains("social\n\nsubordination?"));
    }

    #[test]
    fn symbol_author_note_attaches_to_noise_labeled_first_page_byline() {
        let mut document = document(vec![
            block(LiquidBlockRole::Title, "Test Article"),
            block(LiquidBlockRole::Noise, "Ada Scholar*"),
            block(LiquidBlockRole::Marginalia, "*. Thanks to the editors."),
            block(LiquidBlockRole::Paragraph, "Opening body paragraph."),
        ]);
        document.footnote_link_integrity = Some(integrity(1.0));

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert!(export.text.contains("*Ada Scholar*[^*]"));
        assert!(export.text.contains("[^*]: Thanks to the editors."));
        assert!(!export.text.contains("## Notes"));
    }

    #[test]
    fn unicode_author_note_can_follow_front_page_body_and_divider() {
        let document = document(vec![
            block(LiquidBlockRole::Title, "Test Article"),
            block(LiquidBlockRole::Heading, "Ada Scholar\u{2217}"),
            block(LiquidBlockRole::Paragraph, "Opening abstract."),
            block(LiquidBlockRole::Noise, &"\u{2013}".repeat(40)),
            block(
                LiquidBlockRole::Footnote,
                "\u{2217} Thanks to the editors and workshop participants.",
            ),
        ]);

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert!(export.text.contains("*Ada Scholar*[^*]"));
        assert!(
            export
                .text
                .contains("[^*]: Thanks to the editors and workshop participants.")
        );
        assert!(!export.text.contains("## Notes"));
    }

    #[test]
    fn options_control_tables_and_compacted_metadata() {
        let document = document(vec![
            block(LiquidBlockRole::Metadata, "Published: 2026 | Volume 1"),
            block(LiquidBlockRole::Table, "Term    Value\nAlpha   1"),
        ]);

        let export = liquid_document_markdown(
            &document,
            &MarkdownOptions {
                footnotes: FootnoteMode::Omit,
                include_tables: false,
                include_metadata: true,
            },
        );

        assert!(export.text.contains("2026 Volume 1"));
        assert!(!export.text.contains("Term    Value"));
    }

    #[test]
    fn adjacent_table_fragments_render_in_one_fence() {
        let document = document(vec![
            block(
                LiquidBlockRole::Paragraph,
                "The article introduces the dataset.",
            ),
            block(LiquidBlockRole::Caption, "Table 1"),
            block(LiquidBlockRole::Table, "Commission Dates Without Quorum"),
            block(LiquidBlockRole::Table, "Agency A 01/20/2025 Removal"),
            block(LiquidBlockRole::Table, "Agency B 02/28/2025 Resignation"),
            block(LiquidBlockRole::Paragraph, "The discussion resumes."),
        ]);

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert_eq!(export.text.matches("```").count(), 2, "{}", export.text);
        assert!(export.text.contains(
            "```\nCommission Dates Without Quorum\nAgency A 01/20/2025 Removal\nAgency B 02/28/2025 Resignation\n```"
        ));
        assert!(export.text.ends_with("The discussion resumes."));
    }

    #[test]
    fn adjacent_sentence_table_blocks_keep_real_paragraph_boundaries() {
        let document = document(vec![
            block(
                LiquidBlockRole::Table,
                "The first complete paragraph explains the governing rule in practical terms.",
            ),
            block(
                LiquidBlockRole::Table,
                "The second complete paragraph applies that rule to the disputed conduct.",
            ),
            block(
                LiquidBlockRole::Table,
                "The third complete paragraph states the resulting legal conclusion clearly.",
            ),
        ]);

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert!(!export.text.contains("```"), "{}", export.text);
        assert!(
            export
                .text
                .contains("practical terms.\n\nThe second complete paragraph")
        );
        assert!(
            export
                .text
                .contains("disputed conduct.\n\nThe third complete paragraph")
        );
    }

    #[test]
    fn sentence_prose_misclassified_as_table_renders_as_body_and_keeps_note() {
        let mut document = document(vec![
            block(
                LiquidBlockRole::Table,
                &format!(
                    "After examining the governing rule, the commission explained that ordinary clients need clear advice about the objectives and scope of representation.{CALLOUT_START}1{CALLOUT_END} The discussion then describes the lawyer's obligations in complete sentences and applies those duties to the facts before the court."
                ),
            ),
            block(
                LiquidBlockRole::Marginalia,
                "1 See Model Rule of Professional Conduct 1.2 and accompanying commentary.",
            ),
        ]);
        document.footnote_link_integrity = Some(integrity(1.0));
        add_link(&mut document, 0, 0, 1, 1);

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert!(!export.text.contains("```"), "{}", export.text);
        assert!(export.text.contains("scope of representation.[^1]"));
        assert!(export.text.contains("[^1]: See Model Rule"));
        assert!(
            export
                .warnings
                .iter()
                .any(|warning| warning.contains("table-classified"))
        );
    }

    #[test]
    fn table_prose_fallback_rejoins_lowercase_body_continuations() {
        let document = document(vec![
            block(LiquidBlockRole::Paragraph, "The court explained"),
            block(
                LiquidBlockRole::Table,
                "that the governing doctrine protects ordinary clients when counsel defines the objectives and scope of a limited representation. The commission reviewed the complete rule, its comments, the drafting history, and the practical consequences because",
            ),
            block(
                LiquidBlockRole::Paragraph,
                "the final interpretation depends on all of those sources.",
            ),
        ]);

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert!(!export.text.contains("```"), "{}", export.text);
        assert!(
            export.text.contains(
                "The court explained that the governing doctrine protects ordinary clients"
            )
        );
        assert!(
            export
                .text
                .contains("practical consequences because the final interpretation depends")
        );
        assert!(!export.text.contains("explained\n\nthat"));
        assert!(!export.text.contains("because\n\nthe final"));
    }

    #[test]
    fn long_numeric_table_remains_fenced() {
        let document = document(vec![block(
            LiquidBlockRole::Table,
            "State 1996 1997 1998 1999 2000\nNew York 11.06 11.13 11.13 10.71 11.19\nCalifornia 9.48 9.54 9.03 9.34 8.53\nIllinois 7.69 7.71 7.46 6.95 6.58\nMichigan 7.10 7.04 7.09 7.14 7.11\nOhio 6.30 6.25 6.38 6.40 6.51",
        )]);

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert_eq!(export.text.matches("```").count(), 2, "{}", export.text);
        assert!(export.warnings.is_empty());
    }

    #[test]
    fn figure_caption_emits_page_notice_and_folds_vector_labels_without_topology() {
        let mut figure = block(
            LiquidBlockRole::Caption,
            "Figure 1: Scope of the Party Presentation Principle",
        );
        figure.label = Some("Figure".to_owned());
        let mut labels = block(
            LiquidBlockRole::Table,
            "Claims Legal Theories Issues Reasons Evidence",
        );
        labels.label = Some("Table/Figure".to_owned());
        let mut document = document(vec![
            block(
                LiquidBlockRole::Paragraph,
                "The relationship is a nested set of circles.",
            ),
            figure,
            block(
                LiquidBlockRole::Paragraph,
                "The outermost circle is the widest formulation.",
            ),
            labels,
            block(LiquidBlockRole::Paragraph, "The discussion resumes."),
        ]);
        let source = |block_index, line_index, text: &str, role| LiquidBlockSourceLines {
            block_index,
            lines: vec![LiquidSourceLineRef {
                id: None,
                page_index: 42,
                line_index,
                text: text.to_owned(),
                role,
                note_markers: Vec::new(),
            }],
        };
        document.block_source_lines = vec![
            source(0, 0, &document.blocks[0].text, LiquidBlockRole::Paragraph),
            source(1, 1, &document.blocks[1].text, LiquidBlockRole::Caption),
            source(2, 2, &document.blocks[2].text, LiquidBlockRole::Paragraph),
            LiquidBlockSourceLines {
                block_index: 3,
                lines: ["Claims", "Legal Theories", "Issues", "Reasons", "Evidence"]
                    .into_iter()
                    .enumerate()
                    .map(|(offset, text)| LiquidSourceLineRef {
                        id: None,
                        page_index: 42,
                        line_index: 22 + offset,
                        text: text.to_owned(),
                        role: LiquidBlockRole::Marginalia,
                        note_markers: Vec::new(),
                    })
                    .collect(),
            },
            source(4, 27, &document.blocks[4].text, LiquidBlockRole::Paragraph),
        ];

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert!(export.text.contains(
            "> **Figure 1: Scope of the Party Presentation Principle**\n> Visual content is not included in this text-only export; see PDF page 43."
        ));
        assert!(export.text.contains(
            "> Extracted labels (unordered; spatial relationships are not represented): Claims; Legal Theories; Issues; Reasons; Evidence."
        ));
        assert!(!export.text.contains("```"), "{}", export.text);
        assert_eq!(export.text.matches("Claims").count(), 1, "{}", export.text);
        assert!(export.text.ends_with("The discussion resumes."));
        assert_eq!(
            export
                .warnings
                .iter()
                .filter(|warning| warning.contains("visual figure"))
                .count(),
            1
        );
        assert!(export.warnings[0].starts_with("1 visual figure referenced"));
    }

    #[test]
    fn figure_label_uses_linked_block_marker_when_source_text_is_flattened() {
        let mut figure = block(LiquidBlockRole::Caption, "Figure 1: Decision Paths");
        figure.label = Some("Figure".to_owned());
        let mut first_label = block(LiquidBlockRole::Table, "Individual");
        first_label.label = Some("Table/Figure".to_owned());
        let mut marked_label = block(
            LiquidBlockRole::Table,
            &format!("Action{CALLOUT_START}15{CALLOUT_END}"),
        );
        marked_label.label = Some("Table/Figure".to_owned());
        let mut last_label = block(LiquidBlockRole::Table, "Class claim");
        last_label.label = Some("Table/Figure".to_owned());
        let mut document = document(vec![
            figure,
            first_label,
            marked_label,
            last_label,
            block(LiquidBlockRole::Marginalia, "15. The figure's action note."),
        ]);
        let source = |block_index, line_index, text: &str, role| LiquidBlockSourceLines {
            block_index,
            lines: vec![LiquidSourceLineRef {
                id: None,
                page_index: 4,
                line_index,
                text: text.to_owned(),
                role,
                note_markers: Vec::new(),
            }],
        };
        document.block_source_lines = vec![
            source(0, 0, &document.blocks[0].text, LiquidBlockRole::Caption),
            source(1, 1, "Individual", LiquidBlockRole::Table),
            // PyMuPDF's source row flattens the superscript marker, while the
            // assembled linked block retains the exact callout sentinel.
            source(2, 2, "Action15", LiquidBlockRole::Table),
            source(3, 3, "Class claim", LiquidBlockRole::Table),
            source(
                4,
                20,
                "15. The figure's action note.",
                LiquidBlockRole::Marginalia,
            ),
        ];
        add_link(&mut document, 2, 0, 15, 4);
        document.footnote_link_integrity = Some(integrity(1.0));

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert!(
            export
                .text
                .contains("Individual; Action[^15]; Class claim."),
            "{}",
            export.text
        );
        assert!(export.text.contains("[^15]: The figure's action note."));
        assert!(!export.text.contains("Individual; Action15; Class claim."));

        // Bare digits without a real linked callout remain ordinary label text.
        document.footnote_links.clear();
        document.footnote_link_integrity = None;
        document.blocks[2].text = "Action15".to_owned();
        let unlinked = liquid_document_markdown(&document, &MarkdownOptions::default());
        assert!(
            unlinked.text.contains("Individual; Action15; Class claim."),
            "{}",
            unlinked.text
        );
        assert!(!unlinked.text.contains("Action[^15]"));
    }

    #[test]
    fn figure_label_without_prefix_is_still_an_honest_page_notice() {
        let mut figure = block(
            LiquidBlockRole::Caption,
            "Distribution of Preventive Structures",
        );
        figure.label = Some("Figure".to_owned());
        let mut document = document(vec![
            block(
                LiquidBlockRole::Paragraph,
                "The analysis precedes the chart.",
            ),
            figure,
            block(
                LiquidBlockRole::Paragraph,
                "The analysis follows the chart.",
            ),
        ]);
        document.block_source_lines = vec![LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![LiquidSourceLineRef {
                id: None,
                page_index: 53,
                line_index: 4,
                text: document.blocks[1].text.clone(),
                role: LiquidBlockRole::Caption,
                note_markers: Vec::new(),
            }],
        }];

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert!(export.text.contains(
            "> **Distribution of Preventive Structures**\n> Visual content is not included in this text-only export; see PDF page 54."
        ));
        assert!(
            !export
                .text
                .contains("\n\n*Distribution of Preventive Structures*\n\n")
        );
        assert_eq!(export.warnings.len(), 1);
    }

    #[test]
    fn ordinary_table_near_figure_is_not_suppressed_as_short_labels() {
        let figure = block(LiquidBlockRole::Caption, "Figure 2: Results");
        let mut table = block(
            LiquidBlockRole::Table,
            "Agency A 14.2%\nAgency B 9.8%\nTotal 24.0%",
        );
        table.label = Some("Table/Figure".to_owned());
        let mut document = document(vec![figure, table]);
        document.block_source_lines = vec![
            LiquidBlockSourceLines {
                block_index: 0,
                lines: vec![LiquidSourceLineRef {
                    id: None,
                    page_index: 7,
                    line_index: 1,
                    text: document.blocks[0].text.clone(),
                    role: LiquidBlockRole::Caption,
                    note_markers: Vec::new(),
                }],
            },
            LiquidBlockSourceLines {
                block_index: 1,
                lines: ["Agency A 14.2%", "Agency B 9.8%", "Total 24.0%"]
                    .into_iter()
                    .enumerate()
                    .map(|(offset, text)| LiquidSourceLineRef {
                        id: None,
                        page_index: 7,
                        line_index: 5 + offset,
                        text: text.to_owned(),
                        role: LiquidBlockRole::Table,
                        note_markers: Vec::new(),
                    })
                    .collect(),
            },
        ];

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert!(export.text.contains("see PDF page 8"));
        assert!(
            export.text.contains("```\nAgency A 14.2%"),
            "{}",
            export.text
        );
        assert!(!export.text.contains("Extracted labels (unordered"));
    }

    #[test]
    fn inline_mode_preserves_unlinked_notes_and_appends_stray_continuations() {
        let mut document = document(vec![
            block(LiquidBlockRole::Paragraph, "Claim.\u{E000}1\u{E001}"),
            block(LiquidBlockRole::Footnote, "1 Linked authority."),
            block(LiquidBlockRole::Footnote, "continued discussion."),
            block(LiquidBlockRole::Footnote, "9 Unlinked authority."),
        ]);
        add_link(&mut document, 0, 0, 1, 1);
        document.footnote_link_integrity = Some(integrity(1.0));

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert!(export.footnotes_inlined);
        assert!(
            export
                .text
                .contains("[^1]: Linked authority. continued discussion.")
        );
        assert!(export.text.contains("## Notes"));
        assert!(export.text.contains("9 Unlinked authority."));
        assert!(
            export
                .warnings
                .iter()
                .any(|warning| warning.contains("continuation"))
        );
    }

    #[test]
    fn linked_note_prefix_continues_the_previous_definition() {
        let mut document = document(vec![
            block(
                LiquidBlockRole::Paragraph,
                "First claim.\u{E000}2\u{E001} Second claim.\u{E000}3\u{E001}",
            ),
            block(LiquidBlockRole::Marginalia, "2 The first authority ends at"),
            block(
                LiquidBlockRole::Marginalia,
                "630. \u{E000}3\u{E001} Third authority.",
            ),
        ]);
        add_link(&mut document, 0, 0, 2, 1);
        add_link(&mut document, 0, 1, 3, 2);
        document.footnote_link_integrity = Some(integrity(1.0));
        document.block_source_lines = vec![
            LiquidBlockSourceLines {
                block_index: 1,
                lines: vec![LiquidSourceLineRef {
                    id: Some("p13:l30".to_owned()),
                    page_index: 13,
                    line_index: 30,
                    text: document.blocks[1].text.clone(),
                    role: LiquidBlockRole::Marginalia,
                    note_markers: vec![2],
                }],
            },
            LiquidBlockSourceLines {
                block_index: 2,
                lines: vec![LiquidSourceLineRef {
                    id: Some("p13:l31".to_owned()),
                    page_index: 13,
                    line_index: 31,
                    text: document.blocks[2].text.clone(),
                    role: LiquidBlockRole::Marginalia,
                    note_markers: vec![3],
                }],
            },
        ];

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert!(
            export
                .text
                .contains("[^2]: The first authority ends at 630.")
        );
        assert!(export.text.contains("[^3]: Third authority."));
        assert!(!export.text.contains("## Notes"));
    }

    #[test]
    fn single_linked_note_is_not_split_at_an_internal_pin_cite() {
        let mut document = document(vec![
            block(LiquidBlockRole::Paragraph, "Claim.\u{E000}6\u{E001}"),
            block(
                LiquidBlockRole::Footnote,
                "6 See CHIEF JUSTICE JOHN G. ROBERTS, JR., 2023 YEAR-END REPORT ON \
                 THE FEDERAL JUDICIARY 6 (2023), explaining that the work will change.",
            ),
        ]);
        add_link(&mut document, 0, 0, 6, 1);
        document.footnote_link_integrity = Some(integrity(1.0));

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert!(export.text.contains(
            "[^6]: See CHIEF JUSTICE JOHN G. ROBERTS, JR., 2023 YEAR-END REPORT ON \
             THE FEDERAL JUDICIARY 6 (2023), explaining that the work will change."
        ));
        assert!(!export.text.contains("## Notes"));
        assert_eq!(export.footnote_count, 1);
    }

    #[test]
    fn callout_bearing_case_name_fragments_rejoin_surrounding_prose() {
        let mut document = document(vec![
            block(
                LiquidBlockRole::Paragraph,
                "For example in Hoekman v. Tamko",
            ),
            block(LiquidBlockRole::Heading, "Building Products, Inc.,"),
            block(
                LiquidBlockRole::Heading,
                "\u{E000}372\u{E001} Skadden, Arps, Slate, Meagher & Flom",
            ),
            block(
                LiquidBlockRole::Paragraph,
                "LLP — a multinational firm represented Tamko.",
            ),
            block(
                LiquidBlockRole::Marginalia,
                "372 No. 14-cv-01581, 2015 U.S. Dist. LEXIS 113414.",
            ),
        ]);
        add_link(&mut document, 2, 0, 372, 4);
        document.footnote_link_integrity = Some(integrity(1.0));
        document.block_source_lines = (0..4)
            .map(|index| LiquidBlockSourceLines {
                block_index: index,
                lines: vec![LiquidSourceLineRef {
                    id: Some(format!("p51:l{}", index + 5)),
                    page_index: 51,
                    line_index: index + 5,
                    text: document.blocks[index].text.clone(),
                    role: document.blocks[index].role,
                    note_markers: Vec::new(),
                }],
            })
            .chain(std::iter::once(LiquidBlockSourceLines {
                block_index: 4,
                lines: vec![LiquidSourceLineRef {
                    id: Some("p51:l27".to_owned()),
                    page_index: 51,
                    line_index: 27,
                    text: document.blocks[4].text.clone(),
                    role: LiquidBlockRole::Marginalia,
                    note_markers: vec![372],
                }],
            }))
            .collect();

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert!(export.text.contains(
            "For example in Hoekman v. Tamko Building Products, Inc., [^372] Skadden, Arps, Slate, Meagher & Flom LLP — a multinational firm represented Tamko."
        ));
        assert!(!export.text.contains("## Building Products"));
        assert!(!export.text.contains("## [^372]"));
        assert!(
            export
                .text
                .contains("[^372]: No. 14-cv-01581, 2015 U.S. Dist. LEXIS 113414.")
        );
    }

    #[test]
    fn inline_footnotes_support_markers_above_five_hundred() {
        let mut document = document(vec![
            block(
                LiquidBlockRole::Paragraph,
                "The final proposition.\u{E000}532\u{E001}",
            ),
            block(
                LiquidBlockRole::Marginalia,
                "532 See Balganesh, supra note 529, at 2184.",
            ),
        ]);
        add_link(&mut document, 0, 0, 532, 1);
        document.footnote_link_integrity = Some(integrity(1.0));
        document.block_source_lines = vec![LiquidBlockSourceLines {
            block_index: 1,
            lines: vec![LiquidSourceLineRef {
                id: Some("p78:l33".to_owned()),
                page_index: 78,
                line_index: 33,
                text: document.blocks[1].text.clone(),
                role: LiquidBlockRole::Marginalia,
                note_markers: vec![532],
            }],
        }];

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert!(export.text.contains("The final proposition.[^532]"));
        assert!(
            export
                .text
                .contains("[^532]: See Balganesh, supra note 529, at 2184.")
        );
    }

    #[test]
    fn numbered_note_slicing_ignores_citation_numbers_inside_a_linked_note() {
        let text = "24 First authority. 25 See the discussion, supra note 24, at 15; Ordered Liberty, 35 J. Am. Acad. Matrim. Laws 623. 26 Next authority.";
        let heads = numbered_note_heads(text, &[24, 25, 26]);

        assert_eq!(
            heads.iter().map(|(marker, _)| *marker).collect::<Vec<_>>(),
            vec![24, 25, 26]
        );
        assert_eq!(
            note_text_for_marker(text, 25, &[24, 25, 26]),
            "See the discussion, supra note 24, at 15; Ordered Liberty, 35 J. Am. Acad. Matrim. Laws 623."
        );
    }

    #[test]
    fn next_page_note_continuation_can_cross_intervening_body_blocks() {
        let mut document = document(vec![
            block(LiquidBlockRole::Paragraph, "Claim.\u{E000}45\u{E001}"),
            block(
                LiquidBlockRole::Footnote,
                "45 The authority begins here and",
            ),
            block(LiquidBlockRole::Paragraph, "Body text from the next page."),
            block(LiquidBlockRole::Noise, "8 LAW REVIEW [Vol. 1:1"),
            block(
                LiquidBlockRole::Footnote,
                "continues on the next page before the next numbered note.",
            ),
        ]);
        add_link(&mut document, 0, 0, 45, 1);
        document.footnote_link_integrity = Some(integrity(1.0));
        document.block_source_lines = vec![
            LiquidBlockSourceLines {
                block_index: 1,
                lines: vec![LiquidSourceLineRef {
                    id: None,
                    page_index: 6,
                    line_index: 40,
                    text: "45 The authority begins here and".to_owned(),
                    role: LiquidBlockRole::Footnote,
                    note_markers: vec![45],
                }],
            },
            LiquidBlockSourceLines {
                block_index: 4,
                lines: vec![LiquidSourceLineRef {
                    id: None,
                    page_index: 7,
                    line_index: 38,
                    text: "continues on the next page before the next numbered note.".to_owned(),
                    role: LiquidBlockRole::Footnote,
                    note_markers: Vec::new(),
                }],
            },
        ];

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert!(export.text.contains(
            "[^45]: The authority begins here and continues on the next page before the next numbered note."
        ));
        assert!(!export.text.contains("## Notes"));
    }

    #[test]
    fn next_page_note_continuation_dehyphenates_across_note_blocks() {
        let mut document = document(vec![
            block(LiquidBlockRole::Paragraph, "Claim.\u{E000}287\u{E001}"),
            block(
                LiquidBlockRole::Footnote,
                "287 Press Release, Padilla, Blumenthal Intro-",
            ),
            block(LiquidBlockRole::Paragraph, "Intervening next-page body."),
            block(
                LiquidBlockRole::Marginalia,
                "duce Bill to Provide Victims of Abuse.",
            ),
        ]);
        add_link(&mut document, 0, 0, 287, 1);
        document.footnote_link_integrity = Some(integrity(1.0));
        document.block_source_lines = vec![
            LiquidBlockSourceLines {
                block_index: 1,
                lines: vec![LiquidSourceLineRef {
                    id: None,
                    page_index: 56,
                    line_index: 46,
                    text: document.blocks[1].text.clone(),
                    role: LiquidBlockRole::Footnote,
                    note_markers: vec![287],
                }],
            },
            LiquidBlockSourceLines {
                block_index: 3,
                lines: vec![LiquidSourceLineRef {
                    id: None,
                    page_index: 57,
                    line_index: 18,
                    text: document.blocks[3].text.clone(),
                    role: LiquidBlockRole::Marginalia,
                    note_markers: Vec::new(),
                }],
            },
        ];

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert!(export.text.contains(
            "[^287]: Press Release, Padilla, Blumenthal Introduce Bill to Provide Victims of Abuse."
        ));
        assert!(!export.text.contains("Intro- duce"));
        assert!(!export.text.contains("## Notes"));
    }

    #[test]
    fn marker_only_note_head_can_take_its_next_page_continuation() {
        let mut document = document(vec![
            block(LiquidBlockRole::Paragraph, "Claim.\u{E000}45\u{E001}"),
            block(LiquidBlockRole::Footnote, "45"),
            block(LiquidBlockRole::Noise, "7"),
            block(
                LiquidBlockRole::Footnote,
                "The authority begins at the top of the next endnote page.",
            ),
        ]);
        add_link(&mut document, 0, 0, 45, 1);
        document.footnote_link_integrity = Some(integrity(1.0));
        document.block_source_lines = vec![
            LiquidBlockSourceLines {
                block_index: 1,
                lines: vec![LiquidSourceLineRef {
                    id: None,
                    page_index: 6,
                    line_index: 40,
                    text: "45".to_owned(),
                    role: LiquidBlockRole::Footnote,
                    note_markers: vec![45],
                }],
            },
            LiquidBlockSourceLines {
                block_index: 3,
                lines: vec![LiquidSourceLineRef {
                    id: None,
                    page_index: 7,
                    line_index: 1,
                    text: "The authority begins at the top of the next endnote page.".to_owned(),
                    role: LiquidBlockRole::Footnote,
                    note_markers: Vec::new(),
                }],
            },
        ];

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert!(
            export
                .text
                .contains("[^45]: The authority begins at the top of the next endnote page.")
        );
        assert!(!export.text.contains("## Notes"));
    }

    #[test]
    fn unlinked_marker_only_marginalia_is_not_emitted_as_note_furniture() {
        let document = document(vec![
            block(LiquidBlockRole::Paragraph, "The article continues."),
            block(LiquidBlockRole::Marginalia, "304"),
        ]);

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert!(export.text.ends_with("The article continues."));
        assert!(!export.text.contains("\n304"));
        assert!(!export.text.contains("## Notes"));
    }

    #[test]
    fn numbered_legal_section_can_continue_a_note_when_source_has_no_marker() {
        let mut document = document(vec![
            block(LiquidBlockRole::Paragraph, "Claim.\u{E000}123\u{E001}"),
            block(
                LiquidBlockRole::Footnote,
                "123 The cancellation rule continues on the next page",
            ),
            block(
                LiquidBlockRole::Footnote,
                "211(c)(1), if a receiving bank consents to cancellation.",
            ),
        ]);
        add_link(&mut document, 0, 0, 123, 1);
        document.footnote_link_integrity = Some(integrity(1.0));
        document.block_source_lines = vec![
            LiquidBlockSourceLines {
                block_index: 1,
                lines: vec![LiquidSourceLineRef {
                    id: None,
                    page_index: 6,
                    line_index: 40,
                    text: "123 The cancellation rule continues on the next page".to_owned(),
                    role: LiquidBlockRole::Footnote,
                    note_markers: vec![123],
                }],
            },
            LiquidBlockSourceLines {
                block_index: 2,
                lines: vec![LiquidSourceLineRef {
                    id: None,
                    page_index: 7,
                    line_index: 1,
                    text: "211(c)(1), if a receiving bank consents to cancellation.".to_owned(),
                    role: LiquidBlockRole::Footnote,
                    note_markers: Vec::new(),
                }],
            },
        ];

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert!(export.text.contains(
            "[^123]: The cancellation rule continues on the next page 211(c)(1), if a receiving bank consents to cancellation."
        ));
        assert!(!export.text.contains("## Notes"));
    }

    #[test]
    fn filename_title_prefers_a_real_title_block() {
        let mut document = document(vec![
            block(LiquidBlockRole::Title, "Recovered Article Title"),
            block(LiquidBlockRole::Paragraph, "Body."),
        ]);
        document.title = "scan_0042.pdf".to_owned();

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert!(export.text.starts_with("# Recovered Article Title"));
        assert!(!export.text.contains("scan_0042"));
    }

    #[test]
    fn title_suffix_and_marked_byline_are_not_emitted_as_section_headings() {
        let mut document = document(vec![
            block(
                LiquidBlockRole::Title,
                "ANTISEMITISM, ANTI-ZIONISM, AND TITLE VI:",
            ),
            block(LiquidBlockRole::Heading, "A GUIDE FOR THE PERPLEXED"),
            block(
                LiquidBlockRole::Heading,
                "Benjamin Eidelson\u{2217} & Deborah Hellman\u{2217}\u{2217}",
            ),
            block(LiquidBlockRole::Paragraph, "The article begins here."),
        ]);
        document.title =
            "ANTISEMITISM, ANTI-ZIONISM, AND TITLE VI: A GUIDE FOR THE PERPLEXED".to_owned();

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert!(export.text.starts_with(
            "# ANTISEMITISM, ANTI-ZIONISM, AND TITLE VI: A GUIDE FOR THE PERPLEXED\n\n\
             *Benjamin Eidelson & Deborah Hellman*"
        ));
        assert!(!export.text.contains("## A GUIDE FOR THE PERPLEXED"));
        assert!(!export.text.contains("## Benjamin Eidelson"));
    }

    #[test]
    fn exact_title_repeat_before_body_is_suppressed() {
        let mut document = document(vec![
            block(
                LiquidBlockRole::Title,
                "The Cost of Justice at the Dawn of AI",
            ),
            block(
                LiquidBlockRole::Heading,
                "The Cost of Justice at the Dawn of AI",
            ),
            block(LiquidBlockRole::Heading, "Michael Abramowicz*"),
            block(LiquidBlockRole::Paragraph, "Justice is not free."),
        ]);
        document.title = "The Cost of Justice at the Dawn of AI".to_owned();

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert_eq!(
            export
                .text
                .matches("# The Cost of Justice at the Dawn of AI")
                .count(),
            1
        );
        assert!(export.text.contains("*Michael Abramowicz*"));
        assert!(!export.text.contains("## Michael Abramowicz"));
    }

    #[test]
    fn ocr_damaged_partial_title_repeat_before_body_is_suppressed() {
        let mut document = document(vec![
            block(
                LiquidBlockRole::Paragraph,
                "THE UCC DRAFJTING PROCESS AND SIX QUESTIONS ABOUT ARTICLE 4A: IS THERE A",
            ),
            block(
                LiquidBlockRole::Paragraph,
                "Article 4A governs commercial wire transfers.",
            ),
        ]);
        document.title = "The UCC Drafting Process and Six Questions about Article 4A: Is There a Need for Revisions to the Uniform Funds Transfers Law?".to_owned();

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert!(!export.text.contains("DRAFJTING"));
        assert!(
            export
                .text
                .contains("Article 4A governs commercial wire transfers.")
        );
    }

    #[test]
    fn cross_page_lowercase_body_continuation_is_tightly_joined_and_dehyphenated() {
        let mut document = document(vec![
            block(LiquidBlockRole::Paragraph, "The rule applies com-"),
            block(LiquidBlockRole::Noise, "352 LAW REVIEW"),
            block(
                LiquidBlockRole::Paragraph,
                "mercially reasonable procedures to every transfer.",
            ),
        ]);
        document.block_source_lines = vec![
            LiquidBlockSourceLines {
                block_index: 0,
                lines: vec![LiquidSourceLineRef {
                    id: None,
                    page_index: 0,
                    line_index: 40,
                    text: "The rule applies com-".to_owned(),
                    role: LiquidBlockRole::Paragraph,
                    note_markers: Vec::new(),
                }],
            },
            LiquidBlockSourceLines {
                block_index: 2,
                lines: vec![LiquidSourceLineRef {
                    id: None,
                    page_index: 1,
                    line_index: 1,
                    text: "mercially reasonable procedures to every transfer.".to_owned(),
                    role: LiquidBlockRole::Paragraph,
                    note_markers: Vec::new(),
                }],
            },
        ];

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert!(
            export
                .text
                .contains("The rule applies commercially reasonable procedures to every transfer.")
        );
        assert!(!export.text.contains("com-\n\nmercially"));
    }

    #[test]
    fn numbered_run_in_heading_body_tightly_joins_next_page_paragraph() {
        let mut document = document(vec![
            block(
                LiquidBlockRole::Heading,
                "1. Unilateral Remedies Across Contexts. \u{2014} Unilateral remedies, in which defendants provide relief without plaintiff consent, present the clearest case for applying the exception to a putative class challenging the",
            ),
            block(LiquidBlockRole::Marginalia, "300. Supporting authority."),
            block(LiquidBlockRole::Noise, "1089 LAW REVIEW"),
            block(
                LiquidBlockRole::Paragraph,
                "delays, the government processed each named plaintiff's application.",
            ),
        ]);
        document.block_source_lines = vec![
            LiquidBlockSourceLines {
                block_index: 0,
                lines: vec![LiquidSourceLineRef {
                    id: None,
                    page_index: 51,
                    line_index: 29,
                    text: document.blocks[0].text.clone(),
                    role: LiquidBlockRole::Paragraph,
                    note_markers: Vec::new(),
                }],
            },
            LiquidBlockSourceLines {
                block_index: 1,
                lines: vec![LiquidSourceLineRef {
                    id: None,
                    page_index: 51,
                    line_index: 45,
                    text: document.blocks[1].text.clone(),
                    role: LiquidBlockRole::Marginalia,
                    note_markers: vec![300],
                }],
            },
            LiquidBlockSourceLines {
                block_index: 2,
                lines: vec![LiquidSourceLineRef {
                    id: None,
                    page_index: 52,
                    line_index: 0,
                    text: document.blocks[2].text.clone(),
                    role: LiquidBlockRole::Noise,
                    note_markers: Vec::new(),
                }],
            },
            LiquidBlockSourceLines {
                block_index: 3,
                lines: vec![LiquidSourceLineRef {
                    id: None,
                    page_index: 52,
                    line_index: 1,
                    text: document.blocks[3].text.clone(),
                    role: LiquidBlockRole::Paragraph,
                    note_markers: Vec::new(),
                }],
            },
        ];

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert!(
            export.text.contains(
                "exception to a putative class challenging the delays, the government processed"
            ),
            "{}",
            export.text
        );
        assert!(!export.text.contains("challenging the\n\ndelays,"));
    }

    #[test]
    fn numbered_run_in_cross_page_join_requires_run_in_and_open_body() {
        let source = |block_index, page_index, text: &str, role| LiquidBlockSourceLines {
            block_index,
            lines: vec![LiquidSourceLineRef {
                id: None,
                page_index,
                line_index: if page_index == 0 { 40 } else { 1 },
                text: text.to_owned(),
                role,
                note_markers: Vec::new(),
            }],
        };
        let mut closed = document(vec![
            block(
                LiquidBlockRole::Heading,
                "1. First Question. The first discussion is complete.",
            ),
            block(
                LiquidBlockRole::Paragraph,
                "another paragraph begins on the next page.",
            ),
        ]);
        closed.block_source_lines = vec![
            source(0, 0, &closed.blocks[0].text, LiquidBlockRole::Paragraph),
            source(1, 1, &closed.blocks[1].text, LiquidBlockRole::Paragraph),
        ];
        let closed_export = liquid_document_markdown(&closed, &MarkdownOptions::default());
        assert!(
            closed_export
                .text
                .contains("The first discussion is complete.\n\nanother paragraph begins")
        );

        let mut heading_only = document(vec![
            block(LiquidBlockRole::Heading, "1. First Question"),
            block(
                LiquidBlockRole::Paragraph,
                "another paragraph begins on the next page.",
            ),
        ]);
        heading_only.block_source_lines = vec![
            source(0, 0, &heading_only.blocks[0].text, LiquidBlockRole::Heading),
            source(
                1,
                1,
                &heading_only.blocks[1].text,
                LiquidBlockRole::Paragraph,
            ),
        ];
        let heading_export = liquid_document_markdown(&heading_only, &MarkdownOptions::default());
        assert!(
            heading_export
                .text
                .contains("1. First Question\n\nanother paragraph begins")
        );
    }

    #[test]
    fn cross_page_short_capitalized_sentence_fragment_is_tightly_joined() {
        let mut document = document(vec![
            block(
                LiquidBlockRole::Paragraph,
                "The litigation became the largest MDL in history, the 3M Combat",
            ),
            block(LiquidBlockRole::Noise, "842 LAW REVIEW"),
            block(
                LiquidBlockRole::Paragraph,
                "Arms Earplug Litigation.\u{E000}5\u{E001}",
            ),
        ]);
        document.block_source_lines = vec![
            LiquidBlockSourceLines {
                block_index: 0,
                lines: vec![LiquidSourceLineRef {
                    id: None,
                    page_index: 0,
                    line_index: 40,
                    text: document.blocks[0].text.clone(),
                    role: LiquidBlockRole::Paragraph,
                    note_markers: Vec::new(),
                }],
            },
            LiquidBlockSourceLines {
                block_index: 2,
                lines: vec![LiquidSourceLineRef {
                    id: None,
                    page_index: 1,
                    line_index: 1,
                    text: document.blocks[2].text.clone(),
                    role: LiquidBlockRole::Paragraph,
                    note_markers: vec![5],
                }],
            },
        ];

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert!(
            export
                .text
                .contains("the 3M Combat Arms Earplug Litigation.")
        );
        assert!(!export.text.contains("3M Combat\n\nArms Earplug"));
    }

    #[test]
    fn cross_page_complete_sentence_stays_a_separate_paragraph() {
        let mut document = document(vec![
            block(LiquidBlockRole::Paragraph, "The first paragraph ends."),
            block(
                LiquidBlockRole::Paragraph,
                "another paragraph begins with a damaged lowercase letter.",
            ),
        ]);
        document.block_source_lines = vec![
            LiquidBlockSourceLines {
                block_index: 0,
                lines: vec![LiquidSourceLineRef {
                    id: None,
                    page_index: 0,
                    line_index: 40,
                    text: "The first paragraph ends.".to_owned(),
                    role: LiquidBlockRole::Paragraph,
                    note_markers: Vec::new(),
                }],
            },
            LiquidBlockSourceLines {
                block_index: 1,
                lines: vec![LiquidSourceLineRef {
                    id: None,
                    page_index: 1,
                    line_index: 1,
                    text: "another paragraph begins with a damaged lowercase letter.".to_owned(),
                    role: LiquidBlockRole::Paragraph,
                    note_markers: Vec::new(),
                }],
            },
        ];

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert!(
            export
                .text
                .contains("The first paragraph ends.\n\nanother paragraph begins")
        );
    }

    #[test]
    fn reporting_clause_promotes_contiguous_multiline_paragraph_to_quote() {
        let mut document = document(vec![
            block(
                LiquidBlockRole::Paragraph,
                "The customer's agreement might look like the following:",
            ),
            block(
                LiquidBlockRole::Paragraph,
                "Customer's Acknowledgment. Customer agrees to be bound by the selected procedure.",
            ),
            block(LiquidBlockRole::Paragraph, "The analysis then resumes."),
        ]);
        document.block_source_lines = vec![
            LiquidBlockSourceLines {
                block_index: 0,
                lines: vec![LiquidSourceLineRef {
                    id: None,
                    page_index: 0,
                    line_index: 10,
                    text: "The customer's agreement might look like the following:".to_owned(),
                    role: LiquidBlockRole::Paragraph,
                    note_markers: Vec::new(),
                }],
            },
            LiquidBlockSourceLines {
                block_index: 1,
                lines: vec![
                    LiquidSourceLineRef {
                        id: None,
                        page_index: 0,
                        line_index: 11,
                        text: "Customer's Acknowledgment. Customer agrees".to_owned(),
                        role: LiquidBlockRole::Paragraph,
                        note_markers: Vec::new(),
                    },
                    LiquidSourceLineRef {
                        id: None,
                        page_index: 0,
                        line_index: 12,
                        text: "to be bound by the selected procedure.".to_owned(),
                        role: LiquidBlockRole::Paragraph,
                        note_markers: Vec::new(),
                    },
                ],
            },
        ];

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert!(export.text.contains(
            "the following:\n\n> Customer's Acknowledgment. Customer agrees to be bound"
        ));
        assert!(export.text.contains("\n\nThe analysis then resumes."));
    }

    #[test]
    fn reporting_clause_splits_same_block_quote_and_keeps_page_continuation_tight() {
        let mut document = document(vec![
            block(
                LiquidBlockRole::Paragraph,
                "The parties' rights are affected. Accordingly, the third requirement provides: \
                 With respect to a payment order accepted by the beneficiary's bank, cancellation \
                 is not effective unless the order was unauthorized, or",
            ),
            block(
                LiquidBlockRole::Paragraph,
                "beneficiary consent is obtained *758 to the extent allowed by law.",
            ),
            block(
                LiquidBlockRole::Paragraph,
                "Whether an order is authorized is governed separately.",
            ),
        ]);
        document.block_source_lines = vec![
            LiquidBlockSourceLines {
                block_index: 0,
                lines: vec![
                    LiquidSourceLineRef {
                        id: None,
                        page_index: 0,
                        line_index: 4,
                        text: "The parties' rights are affected.".to_owned(),
                        role: LiquidBlockRole::Paragraph,
                        note_markers: Vec::new(),
                    },
                    LiquidSourceLineRef {
                        id: None,
                        page_index: 0,
                        line_index: 5,
                        text: "Accordingly, the third requirement provides:".to_owned(),
                        role: LiquidBlockRole::Paragraph,
                        note_markers: Vec::new(),
                    },
                    LiquidSourceLineRef {
                        id: None,
                        page_index: 0,
                        line_index: 6,
                        text: "With respect to a payment order accepted by the beneficiary's bank,"
                            .to_owned(),
                        role: LiquidBlockRole::Paragraph,
                        note_markers: Vec::new(),
                    },
                    LiquidSourceLineRef {
                        id: None,
                        page_index: 0,
                        line_index: 7,
                        text: "cancellation is not effective unless the order was unauthorized, or"
                            .to_owned(),
                        role: LiquidBlockRole::Paragraph,
                        note_markers: Vec::new(),
                    },
                ],
            },
            LiquidBlockSourceLines {
                block_index: 1,
                lines: vec![LiquidSourceLineRef {
                    id: None,
                    page_index: 1,
                    line_index: 1,
                    text: "beneficiary consent is obtained to the extent allowed by law."
                        .to_owned(),
                    role: LiquidBlockRole::Paragraph,
                    note_markers: Vec::new(),
                }],
            },
        ];

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert!(export.text.contains(
            "requirement provides:\n\n> With respect to a payment order accepted by the beneficiary's bank"
        ));
        assert!(export.text.contains(
            "unauthorized, or beneficiary consent is obtained to the extent allowed by law."
        ));
        assert!(!export.text.contains("*758"));
        assert!(export.text.contains("\n\nWhether an order is authorized"));
    }

    #[test]
    fn following_notice_reflows_multiblock_all_caps_quote_without_headings() {
        let mut document = document(vec![
            block(
                LiquidBlockRole::Paragraph,
                "The following notice was molded into each individual shingle:",
            ),
            block(
                LiquidBlockRole::Heading,
                "PURCHASE OF THIS PRODUCT IS SUBJECT TO THE TERMS AND LIMITATIONS TRANS-",
            ),
            block(
                LiquidBlockRole::Heading,
                "ACTION. THERE ARE NO OTHER WARRANTIES FOR THIS PRODUCT.",
            ),
            block(
                LiquidBlockRole::Noise,
                "CALL THE DISTRIBUTOR AT 1-800-555-0100, OR",
            ),
            block(LiquidBlockRole::Paragraph, "VISIT WWW.EXAMPLE.COM."),
            block(
                LiquidBlockRole::Paragraph,
                "These shingles were then affixed to the roof.",
            ),
        ]);
        document.block_source_lines = (0..document.blocks.len())
            .map(|block_index| LiquidBlockSourceLines {
                block_index,
                lines: vec![LiquidSourceLineRef {
                    id: None,
                    page_index: 0,
                    line_index: 10 + block_index,
                    text: document.blocks[block_index].text.clone(),
                    role: document.blocks[block_index].role,
                    note_markers: Vec::new(),
                }],
            })
            .collect();

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert!(export.text.contains(
            "> PURCHASE OF THIS PRODUCT IS SUBJECT TO THE TERMS AND LIMITATIONS TRANS- ACTION. \
             THERE ARE NO OTHER WARRANTIES FOR THIS PRODUCT. CALL THE DISTRIBUTOR AT \
             1-800-555-0100, OR VISIT WWW.EXAMPLE.COM."
        ));
        assert!(!export.text.contains("## PURCHASE OF THIS PRODUCT"));
        assert!(!export.text.contains("## ACTION."));
        assert!(export.text.contains("\n\nThese shingles were then affixed"));
    }

    #[test]
    fn single_page_locator_dotleader_is_contents_furniture() {
        assert!(looks_like_contents_block(
            "A. Two Possibilities for Law's Near-Term Future ....................................37"
        ));
        assert!(looks_like_contents_block(
            "IV. CONCLUSION........................................................................72"
        ));
    }

    #[test]
    fn no_dotleader_contents_continuation_requires_terminal_page_locator() {
        assert!(looks_like_contents_block(
            "1. Authority Attribution 3003 2. Recalibrating Immunity 3006 3. Federal Cause of Action 3010 4. Interstate Compacts 3027 conclusion 3034"
        ));
        assert!(!looks_like_contents_block(
            "Courts considered five authorities before they concluded that the statutory rule controlled."
        ));
    }

    #[test]
    fn five_inline_callouts_and_concluded_do_not_suppress_body_paragraph() {
        let mut body = String::from(
            "Courts have also interpreted statutes to permit delegated authority absent a quorum.",
        );
        for marker in 223..=226 {
            body.push_str(&format!(" Authority{CALLOUT_START}{marker}{CALLOUT_END}."));
        }
        body.push_str(&format!(
            " We have reached the conclusion that the statutory rule controls.{CALLOUT_START}227{CALLOUT_END}"
        ));
        let mut blocks = vec![block(LiquidBlockRole::Paragraph, &body)];
        for marker in 223..=227 {
            blocks.push(block(
                LiquidBlockRole::Marginalia,
                &format!("{CALLOUT_START}{marker}{CALLOUT_END}. Citation {marker}."),
            ));
        }
        let mut document = document(blocks);
        for (ordinal, marker) in (223..=227).enumerate() {
            add_link(&mut document, 0, ordinal, marker, ordinal + 1);
        }
        document.footnote_link_integrity = Some(LiquidFootnoteLinkIntegrity {
            detectable_markers: 5,
            landed: 5,
            unmatched: 0,
            ambiguous: 0,
            note_heads: 5,
            landing_rate: 1.0,
            ambiguous_rate: 0.0,
        });

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert!(
            export
                .text
                .contains("Courts have also interpreted statutes")
        );
        assert!(
            export
                .text
                .contains("We have reached the conclusion that the statutory rule controls.")
        );
        for marker in 223..=227 {
            assert!(export.text.contains(&format!("[^{marker}]")));
        }
    }

    #[test]
    fn repeated_front_matter_title_byline_and_contents_are_compacted() {
        let mut document = document(vec![
            block(
                LiquidBlockRole::Heading,
                "ARTICLE CONTRACT-WRAPPED PROPERTY",
            ),
            block(LiquidBlockRole::Heading, "Danielle Dâ€™Onfro"),
            block(
                LiquidBlockRole::Paragraph,
                "CONTENTS INTRODUCTION ........................ 1059 I. THE PUZZLE \
                 ........................ 1064",
            ),
            block(LiquidBlockRole::Heading, "CONTRACT-WRAPPED PROPERTY"),
            block(LiquidBlockRole::Heading, "Danielle Dâ€™Onfro\u{2217}"),
            block(
                LiquidBlockRole::Abstract,
                "For nearly two centuries, the law has allowed servitudes.",
            ),
        ]);
        document.title = "CONTRACT-WRAPPED PROPERTY Danielle Dâ€™Onfro".to_owned();

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert!(
            export
                .text
                .starts_with("# CONTRACT-WRAPPED PROPERTY\n\n*Danielle Dâ€™Onfro*\n\n## Abstract")
        );
        assert_eq!(export.text.matches("CONTRACT-WRAPPED PROPERTY").count(), 1);
        assert_eq!(export.text.matches("Danielle Dâ€™Onfro").count(), 1);
        assert!(!export.text.contains("CONTENTS"));
        assert!(!export.text.contains("........................"));
    }

    #[test]
    fn short_title_case_title_is_not_repeated_as_a_plain_person_byline() {
        let mut document = document(vec![
            block(LiquidBlockRole::Heading, "ARTICLE"),
            block(LiquidBlockRole::Paragraph, "Commission Quorums"),
            block(
                LiquidBlockRole::Heading,
                "Nicholas R. Bednar & Todd Phillips*",
            ),
            block(
                LiquidBlockRole::Paragraph,
                "Abstract. Multimember commissions are a central feature of the modern administrative state.",
            ),
            block(LiquidBlockRole::Heading, "Introduction"),
            block(LiquidBlockRole::Paragraph, "The Article begins here."),
        ]);
        document.title = "Commission Quorums".to_owned();

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert!(
            export
                .text
                .starts_with("# Commission Quorums\n\n*Nicholas R. Bednar & Todd Phillips*")
        );
        assert!(
            export
                .text
                .contains("\n\n## Abstract\n\nMultimember commissions")
        );
        assert!(!export.text.contains("Abstract. Multimember"));
        assert_eq!(export.text.matches("Commission Quorums").count(), 1);
        assert!(!export.text.contains("*Commission Quorums*"));
    }

    #[test]
    fn note_front_matter_uses_article_title_and_plain_person_byline_once() {
        let mut document = document(vec![
            block(
                LiquidBlockRole::Noise,
                "1 70 Tex. L. Rev. 739 Texas Law Review February, 1992",
            ),
            block(LiquidBlockRole::Heading, "Note"),
            block(LiquidBlockRole::Paragraph, "Roger Cowie"),
            block(
                LiquidBlockRole::Noise,
                "Copyright (c) 1992 by the Texas Law Review Association",
            ),
            block(
                LiquidBlockRole::Paragraph,
                "CANCELLATION OF WIRE TRANSFERS UNDER ARTICLE 4A OF THE UNIFORM COMMERCIAL CODE",
            ),
            block(LiquidBlockRole::Heading, "I. Introduction"),
            block(LiquidBlockRole::Paragraph, "Opening body text."),
            block(
                LiquidBlockRole::Paragraph,
                "*741 II. The Mechanics of Wire Transfers",
            ),
        ]);
        document.title =
            "CANCELLATION OF WIRE TRANSFERS UNDER ARTICLE 4A OF THE UNIFORM COMMERCIAL CODE"
                .to_owned();

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert!(
            export.text.starts_with(
                "# CANCELLATION OF WIRE TRANSFERS UNDER ARTICLE 4A OF THE UNIFORM COMMERCIAL CODE\n\n*Roger Cowie*\n\n## I. Introduction"
            ),
            "{}",
            export.text
        );
        assert_eq!(
            export
                .text
                .matches("CANCELLATION OF WIRE TRANSFERS")
                .count(),
            1
        );
        assert_eq!(export.text.matches("Roger Cowie").count(), 1);
        assert!(!export.text.contains("## Note"));
        assert!(
            export
                .text
                .contains("## II. The Mechanics of Wire Transfers")
        );
        assert!(!export.text.contains("*741"));
    }

    #[test]
    fn star_pagination_is_removed_from_inline_prose() {
        assert_eq!(
            normalize_and_escape_body(
                "A transfer between *740 sophisticated institutions later reached *741 another bank."
            ),
            "A transfer between sophisticated institutions later reached another bank."
        );
    }

    #[test]
    fn numbered_outline_run_in_splits_title_from_body() {
        assert_eq!(
            numbered_outline_run_in(
                "1. Cancellation Before Acceptance.-The general rule permits cancellation."
            ),
            Some((
                "1. Cancellation Before Acceptance".to_owned(),
                "The general rule permits cancellation.".to_owned()
            ))
        );
        assert_eq!(
            numbered_outline_run_in(
                "1. Unilateral Remedies Across Contexts. — Unilateral remedies apply."
            ),
            Some((
                "1. Unilateral Remedies Across Contexts".to_owned(),
                "Unilateral remedies apply.".to_owned()
            ))
        );
        assert_eq!(
            numbered_outline_run_in("1. Law. The administration has invoked this rule."),
            Some((
                "1. Law".to_owned(),
                "The administration has invoked this rule.".to_owned()
            ))
        );
    }

    #[test]
    fn numbered_outline_run_in_rejects_decimal_and_inline_enumeration() {
        assert_eq!(
            numbered_outline_run_in("1.2 Cancellation.-The rule applies."),
            None
        );
        assert_eq!(
            numbered_outline_run_in("1. First.-then the second step follows."),
            None
        );
    }

    #[test]
    fn numbered_outline_case_title_is_not_split_at_abbreviation() {
        assert!(numbered_outline_heading_without_body(
            "1. Party Presentation in Action: United States v. Sineneng-Smith"
        ));
        assert!(numbered_outline_heading_without_body(
            "2. Party Presentation Ignored: Erie Railroad Co. v. Tompkins"
        ));
        assert!(!numbered_outline_heading_without_body(
            "1. Cancellation Before Acceptance. The general rule permits cancellation."
        ));
        assert!(!numbered_outline_heading_without_body(
            "1. Unilateral Remedies Across Contexts. — Unilateral remedies, in"
        ));
    }

    #[test]
    fn numbered_outline_run_in_block_separates_heading_and_body() {
        let document = document(vec![block(
            LiquidBlockRole::Heading,
            "1. Unilateral Remedies Across Contexts. — Unilateral remedies apply.",
        )]);

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert!(
            export.text.contains(
                "#### 1. Unilateral Remedies Across Contexts\n\nUnilateral remedies apply."
            )
        );
    }

    #[test]
    fn numbered_outline_case_title_does_not_tight_join_following_body() {
        let mut document = document(vec![
            block(
                LiquidBlockRole::Heading,
                "1. Party Presentation in Action: United States v. Sineneng-Smith",
            ),
            block(
                LiquidBlockRole::Paragraph,
                "United States v. Sineneng-Smith was a 2020 Supreme Court decision.",
            ),
        ]);
        document.block_source_lines = vec![
            LiquidBlockSourceLines {
                block_index: 0,
                lines: vec![LiquidSourceLineRef {
                    id: None,
                    page_index: 0,
                    line_index: 10,
                    text: document.blocks[0].text.clone(),
                    role: LiquidBlockRole::Heading,
                    note_markers: Vec::new(),
                }],
            },
            LiquidBlockSourceLines {
                block_index: 1,
                lines: vec![LiquidSourceLineRef {
                    id: None,
                    page_index: 0,
                    line_index: 11,
                    text: document.blocks[1].text.clone(),
                    role: LiquidBlockRole::Paragraph,
                    note_markers: Vec::new(),
                }],
            },
        ];

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert!(export.text.contains(
            "#### 1. Party Presentation in Action: United States v. Sineneng-Smith\n\nUnited States v. Sineneng-Smith was a 2020 Supreme Court decision."
        ));
    }

    #[test]
    fn heading_reader_rejects_prose_fragments_and_section_symbol_citations() {
        assert!(!reads_like_heading("In"));
        assert!(!reads_like_heading("It"));
        assert!(!reads_like_heading("§ 1983 and into the Bivens posture."));
        assert!(!reads_like_heading(
            "issue. We call this the limited option."
        ));
        assert!(reads_like_heading("introduction"));
        assert!(reads_like_heading(
            "II. EMPLOYMENT IN THE SUPREME COURT'S DOCTRINE"
        ));
    }

    #[test]
    fn fixture_goldens_match_real_pipeline_shapes() {
        let fixtures = [
            (
                include_str!("../../tests/markdown_fixtures/digital_law_review.json"),
                include_str!("../../tests/markdown_fixtures/digital_law_review.md"),
            ),
            (
                include_str!("../../tests/markdown_fixtures/scanned_ocr.json"),
                include_str!("../../tests/markdown_fixtures/scanned_ocr.md"),
            ),
            (
                include_str!("../../tests/markdown_fixtures/front_matter_heavy.json"),
                include_str!("../../tests/markdown_fixtures/front_matter_heavy.md"),
            ),
        ];
        for (fixture, expected) in fixtures {
            let document: LiquidDocument = serde_json::from_str(fixture).unwrap();
            let export = liquid_document_markdown(&document, &MarkdownOptions::default());
            assert_eq!(export.text, expected.trim_end());
        }
    }

    #[test]
    fn recovers_italic_url_note_line_shape() {
        let source = "*63 Biglaw Investor, Biglaw Salary Scale, https://www.biglawinvestor.com/biglaw-salary-scale/*";
        let (marker, text) = recover_numeric_url_note_line(source).expect("url note");
        assert_eq!(marker, 63);
        assert!(text.starts_with("Biglaw Investor"));
        assert!(text.contains("https://www.biglawinvestor.com/biglaw-salary-scale/"));
        assert_eq!(
            recover_numeric_url_note_line(
                "63 Biglaw Investor, Biglaw Salary Scale, https://www.biglawinvestor.com/biglaw-salary-scale/"
            ),
            Some((marker, text.clone()))
        );
        assert!(recover_numeric_url_note_line("2024 See https://example.com/paper").is_none());
        assert!(numeric_url_note_marker(source) == Some(63));
    }

    #[test]
    fn recovers_fused_trailing_note_and_bare_id_line() {
        let fused = "Compare id. at 152, with Dye, 908 F.3d at 678. tions, it's also eminently reasonable to assume that by opening and retaining those items a consumer necessarily accepts the accompanying terms and conditions.295";
        let (kept, marker) = peel_glued_trailing_note_number(fused).expect("glued 295");
        assert_eq!(marker, 295);
        assert!(kept.ends_with("conditions."));
        assert!(!kept.ends_with("295"));
        let leftover =
            "295 Id. at 682–83 (citing Kolodziej v. Mason, 774 F.3d 736, 742 (11th Cir. 2014)).";
        let (bare_marker, bare_text) =
            recover_bare_numeric_note_line(leftover).expect("bare 295 Id.");
        assert_eq!(bare_marker, 295);
        assert!(bare_text.starts_with("Id. at 682"));
        assert!(peel_glued_trailing_note_number("Dye, 908 F.3d at 678.").is_none());
        assert!(recover_bare_numeric_note_line("9 Unlinked authority.").is_none());
    }

    #[test]
    fn italic_url_caption_becomes_landed_footnote() {
        let mut document = document(vec![
            block(
                LiquidBlockRole::Paragraph,
                "Figure 3 shows salaries have risen faster than inflation since 1999.\u{E000}63\u{E001} Though comprehensive data is difficult to obtain.",
            ),
            block(
                LiquidBlockRole::Caption,
                "63 Biglaw Investor, Biglaw Salary Scale, https://www.biglawinvestor.com/biglaw-salary-scale/",
            ),
            block(
                LiquidBlockRole::Paragraph,
                "Median lawyer wages have also been increasing.\u{E000}64\u{E001}",
            ),
            block(
                LiquidBlockRole::Footnote,
                "64 MARC GALANTER & THOMAS PALAY, TOURNAMENT OF LAWYERS 24 (1991).",
            ),
        ]);
        crate::liquid::attach_footnote_links(&mut document);
        assert!(
            document
                .footnote_links
                .iter()
                .any(|link| link.marker == 63 && link.note_block_index == 1),
            "url caption must become the note head for 63: {:?}",
            document.footnote_links
        );

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());
        assert!(
            export.text.contains("[^63]: Biglaw Investor"),
            "{}",
            export.text
        );
        assert!(
            export.text.contains("1999.[^63]"),
            "body callout must land: {}",
            export.text
        );
        assert!(
            !export.text.contains("*63 Biglaw"),
            "caption must not stay italic body: {}",
            export.text
        );
    }

    #[test]
    fn numbered_url_paragraph_without_callout_stays_in_body() {
        let document = document(vec![block(
            LiquidBlockRole::Paragraph,
            "2 Learn more at https://example.com/clinic",
        )]);

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());

        assert!(
            export
                .text
                .contains("2 Learn more at https://example.com/clinic"),
            "{}",
            export.text
        );
        assert!(!export.text.contains("[^2]:"), "{}", export.text);
        assert_eq!(export.footnote_count, 0);
    }

    #[test]
    fn fused_continuation_number_stays_unlinked_without_body_provenance() {
        let mut document = document(vec![
            block(
                LiquidBlockRole::Paragraph,
                "The warranty was at the heart of the suit.\u{E000}293\u{E001} Homeowners accepted the terms.\u{E000}294\u{E001} The court later continued: items come with terms and condi",
            ),
            block(
                LiquidBlockRole::Footnote,
                "293 Compare id. at 152, with Dye, 908 F.3d at 678.",
            ),
            block(
                LiquidBlockRole::Footnote,
                "tions, it's also eminently reasonable to assume that by opening and retaining those items a consumer necessarily accepts the accompanying terms and conditions.295",
            ),
            block(LiquidBlockRole::Footnote, "294 Dye, 908 F.3d at 678."),
            block(
                LiquidBlockRole::Footnote,
                "295 Id. at 682–83 (citing Kolodziej v. Mason, 774 F.3d 736, 742 (11th Cir. 2014)).",
            ),
            block(
                LiquidBlockRole::Paragraph,
                "The court even said fair notice has been building.\u{E000}296\u{E001}",
            ),
            block(LiquidBlockRole::Footnote, "296 Id. at 683."),
        ]);
        add_link(&mut document, 0, 0, 293, 1);
        add_link(&mut document, 0, 1, 294, 3);
        add_link(&mut document, 5, 0, 296, 6);
        document.footnote_link_integrity = Some(integrity(1.0));

        let export = liquid_document_markdown(&document, &MarkdownOptions::default());
        let note_293 = export
            .text
            .lines()
            .find(|line| line.starts_with("[^293]:"))
            .unwrap_or("");
        assert!(
            !note_293.trim_end().ends_with("295"),
            "293 must not keep a glued 295: {note_293}"
        );
        let body = export.text.split("\n---\n").next().unwrap_or("");
        assert!(
            !body.contains("[^295]"),
            "body must not invent a location for 295: {}",
            export.text
        );
        assert!(
            !export.text.contains("[^295]:"),
            "an unlocated 295 must not become an orphan definition: {}",
            export.text
        );
        assert!(
            export.text.lines().any(|line| line.starts_with("295 Id.")),
            "the exact unlinked 295 text must remain visible: {}",
            export.text
        );
    }
}
