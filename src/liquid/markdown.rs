use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::{LiquidBlockRole, LiquidDocument, LiquidFootnoteLink};

const CALLOUT_START: char = '\u{E000}';
const CALLOUT_END: char = '\u{E001}';
const MARKDOWN_MARKER_START: char = '\u{E100}';
const MARKDOWN_MARKER_END: char = '\u{E101}';
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
    let linked_note_indices = document
        .footnote_links
        .iter()
        .map(|link| link.note_block_index)
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
    let available_author_notes = collect_author_notes(document, &linked_note_indices);
    let has_non_author_notes = note_indices
        .iter()
        .any(|index| !available_author_notes.note_blocks.contains(index));

    let mut warnings = Vec::new();
    let mut inline_blocks = BTreeMap::new();
    let footnotes_inlined = match options.footnotes {
        FootnoteMode::Inline if !has_non_author_notes && document.footnote_links.is_empty() => true,
        FootnoteMode::Inline => {
            let integrity_is_usable = document
                .footnote_link_integrity
                .as_ref()
                .is_some_and(|integrity| integrity.landing_rate >= 0.9);
            if !integrity_is_usable {
                warnings.push(LOW_LINK_CONFIDENCE_WARNING.to_owned());
                false
            } else {
                let placement = rewrite_inline_blocks(document);
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
    let title = resolved_title(document);
    let author_lines = document
        .blocks
        .iter()
        .enumerate()
        .filter(|(_, block)| block.role == LiquidBlockRole::AuthorInfo)
        .filter_map(|(index, block)| {
            let text = normalize_whitespace(&block.text);
            if text.is_empty() {
                return None;
            }
            let markers = author_notes
                .by_author_block
                .get(&index)
                .map(|markers| {
                    markers
                        .iter()
                        .map(|marker| format!("[^{marker}]"))
                        .collect::<String>()
                })
                .unwrap_or_default();
            Some(format!("*{text}*{markers}"))
        })
        .collect::<Vec<_>>();

    let mut writer = MarkdownWriter::default();
    if let Some(title) = title {
        writer.push(format!("# {title}"), BlockJoin::Loose);
    }
    for author in author_lines {
        writer.push(author, BlockJoin::Loose);
    }

    let mut heading_context = HeadingContext::default();
    let mut last_source_text: Option<String> = None;
    let mut last_special_section = None;
    for (index, block) in document.blocks.iter().enumerate() {
        if matches!(
            block.role,
            LiquidBlockRole::Title | LiquidBlockRole::AuthorInfo
        ) || linked_note_indices.contains(&index)
            || author_notes.note_blocks.contains(&index)
        {
            continue;
        }

        let raw_text = inline_blocks
            .get(&index)
            .map(String::as_str)
            .unwrap_or(&block.text);
        let source_key = normalize_whitespace(&strip_callout_sentinels(raw_text));
        if !source_key.is_empty()
            && last_source_text
                .as_ref()
                .is_some_and(|previous| previous.eq_ignore_ascii_case(&source_key))
        {
            continue;
        }

        let emitted = match block.role {
            LiquidBlockRole::Heading | LiquidBlockRole::Subheading => {
                let level_text = normalize_whitespace(&strip_callout_sentinels(raw_text));
                let text = normalize_heading_text(raw_text);
                if text.is_empty() {
                    false
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
                let text = normalize_and_escape_body(raw_text);
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
                    writer.push(text, BlockJoin::Loose);
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
                let text = normalize_and_escape_body(raw_text);
                if text.is_empty() {
                    false
                } else {
                    writer.push(text, BlockJoin::Loose);
                    last_special_section = None;
                    true
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
            LiquidBlockRole::Footnote
            | LiquidBlockRole::Marginalia
            | LiquidBlockRole::Header
            | LiquidBlockRole::Footer
            | LiquidBlockRole::Contents
            | LiquidBlockRole::Noise
            | LiquidBlockRole::Table
            | LiquidBlockRole::Metadata
            | LiquidBlockRole::Title
            | LiquidBlockRole::AuthorInfo => false,
        };
        if emitted && !source_key.is_empty() {
            last_source_text = Some(source_key);
        }
    }

    let footnote_count = match options.footnotes {
        FootnoteMode::Inline if footnotes_inlined => {
            let notes = build_inline_notes(
                document,
                &note_indices,
                &linked_note_indices,
                author_notes,
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

    let text = writer.finish();
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockJoin {
    Loose,
    ListItem,
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
        if !self.text.is_empty() {
            if self.last_join == Some(BlockJoin::ListItem) && join == BlockJoin::ListItem {
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
        }
    }
}

fn heading_level(text: &str, role: LiquidBlockRole) -> u8 {
    match leading_heading_enumerator(text) {
        Some(HeadingEnumerator::Arabic) => 4,
        Some(HeadingEnumerator::Letter(_)) => 3,
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
    None
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

#[derive(Default)]
struct PlacementOutcome {
    blocks: BTreeMap<usize, String>,
    attempted: usize,
    failed: usize,
    appended: usize,
}

fn rewrite_inline_blocks(document: &LiquidDocument) -> PlacementOutcome {
    let mut by_block: BTreeMap<usize, Vec<&LiquidFootnoteLink>> = BTreeMap::new();
    for link in &document.footnote_links {
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
            if let Some(occurrence) = occurrences.get(link.body_marker_ordinal)
                && occurrence.marker == link.marker
                && !replacements.contains_key(&occurrence.start)
            {
                replacements.insert(occurrence.start, (occurrence.end, occurrence.marker));
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
                rewritten.push(MARKDOWN_MARKER_START);
                rewritten.push_str(&link.marker.to_string());
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

fn replace_marker_occurrences(text: &str, replacements: &BTreeMap<usize, (usize, u16)>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0usize;
    for (start, (end, marker)) in replacements {
        if *start < cursor || *end > text.len() {
            continue;
        }
        out.push_str(&text[cursor..*start]);
        out.push(MARKDOWN_MARKER_START);
        out.push_str(&marker.to_string());
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
    for ch in text.chars() {
        match ch {
            MARKDOWN_MARKER_START => {
                inside = true;
                marker.clear();
            }
            MARKDOWN_MARKER_END if inside => {
                out.push_str("[^");
                out.push_str(&marker);
                out.push(']');
                inside = false;
                marker.clear();
            }
            _ if inside => marker.push(ch),
            _ => out.push(ch),
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
                    && (1..=500).contains(&marker)
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
            && (1..=500).contains(&marker)
        {
            occurrences.push(MarkerOccurrence { start, end, marker });
        }
    }
    occurrences
}

#[derive(Clone, Default)]
struct AuthorNotes {
    by_author_block: BTreeMap<usize, Vec<String>>,
    note_blocks: BTreeSet<usize>,
    definitions: Vec<FootnoteDefinition>,
}

fn collect_author_notes(
    document: &LiquidDocument,
    linked_note_indices: &BTreeSet<usize>,
) -> AuthorNotes {
    let mut notes = AuthorNotes::default();
    for (index, block) in document.blocks.iter().enumerate() {
        if linked_note_indices.contains(&index) {
            continue;
        }
        let Some((marker, text)) = leading_symbol_note(&block.text) else {
            continue;
        };
        let Some(author_index) = index.checked_sub(1).filter(|author_index| {
            document.blocks[*author_index].role == LiquidBlockRole::AuthorInfo
        }) else {
            continue;
        };
        notes
            .by_author_block
            .entry(author_index)
            .or_default()
            .push(marker.clone());
        notes.note_blocks.insert(index);
        notes.definitions.push(FootnoteDefinition {
            label: marker,
            text,
            note_index: index,
        });
    }
    notes
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
    linked_note_indices: &BTreeSet<usize>,
    author_notes: AuthorNotes,
    warnings: &mut Vec<String>,
) -> InlineNotes {
    let mut definitions = author_notes.definitions;
    let mut links = document.footnote_links.iter().collect::<Vec<_>>();
    links.sort_by_key(|link| (link.note_block_index, link.marker));
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

    for link in links {
        let label = link.marker.to_string();
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
                "footnote {} has no readable note text",
                link.marker
            ));
            continue;
        }
        definitions.push(FootnoteDefinition {
            label,
            text,
            note_index: link.note_block_index,
        });
    }
    definitions.sort_by_key(|definition| definition.note_index);

    let mut unlinked = Vec::new();
    let mut continuation_target: Option<(usize, usize)> = None;
    for index in note_indices {
        if linked_note_indices.contains(index) || author_notes.note_blocks.contains(index) {
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
        let has_marker =
            leading_numeric_note_marker(&text).is_some() || leading_symbol_note(&text).is_some();
        if !has_marker
            && let Some((previous_index, definition_index)) = continuation_target
            && previous_index.saturating_add(1) == *index
            && let Some(definition) = definitions.get_mut(definition_index)
        {
            definition.text.push(' ');
            definition.text.push_str(&escape_footnote_text(&text));
            continuation_target = Some((*index, definition_index));
            warnings.push(
                "a stray footnote continuation was appended to the previous definition".to_owned(),
            );
            continue;
        }
        unlinked.push(escape_footnote_text(&text));
        continuation_target = None;
    }

    InlineNotes {
        definitions,
        unlinked,
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

fn note_text_for_marker(text: &str, marker: u16, block_markers: &[u16]) -> String {
    let normalized = normalize_whitespace(&strip_callout_sentinels(text));
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
        if !expected.contains(&marker) {
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
    (1..=500).contains(&marker).then_some(marker)
}

fn leading_symbol_note(text: &str) -> Option<(String, String)> {
    let trimmed = text.trim_start();
    let marker = trimmed.chars().next()?;
    if !matches!(marker, '*' | '†' | '‡') {
        return None;
    }
    let rest = trimmed[marker.len_utf8()..]
        .trim_start_matches(['.', ')', ']', ':'])
        .trim();
    (!rest.is_empty()).then(|| (marker.to_string(), escape_footnote_text(rest)))
}

fn is_note_candidate(role: LiquidBlockRole, text: &str, linked: bool) -> bool {
    role == LiquidBlockRole::Footnote
        || (role == LiquidBlockRole::Marginalia
            && (linked
                || leading_numeric_note_marker(text).is_some()
                || leading_symbol_note(text).is_some()))
}

fn resolved_title(document: &LiquidDocument) -> Option<String> {
    let document_title = normalize_whitespace(&document.title);
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
    let text = normalize_whitespace(&strip_callout_sentinels(text));
    render_marker_placeholders(&escape_body_text(&text))
}

fn normalize_heading_text(text: &str) -> String {
    let text = normalize_whitespace(&strip_callout_sentinels(text));
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
    let text = strip_callout_sentinels(text);
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
        LiquidBlock, LiquidBlockSourceLines, LiquidFootnoteLinkIntegrity, LiquidSourceLineRef,
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
            ("Introduction", LiquidBlockRole::Subheading, 2),
            ("Conclusion", LiquidBlockRole::Heading, 2),
            ("Background", LiquidBlockRole::Heading, 2),
            ("Background", LiquidBlockRole::Subheading, 3),
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
        low_integrity.footnote_link_integrity = Some(integrity(0.89));
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
}
