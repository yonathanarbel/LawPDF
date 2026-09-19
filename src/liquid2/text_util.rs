//! Text helpers, hashing, and title/author recovery.

use super::*;

pub(super) fn pp_prior_key(path: &str, page_index: usize, line_index: usize, text: &str) -> String {
    eval_key(path, page_index, line_index, text)
}

pub(super) fn bin_name(value: f32, cuts: &[f32]) -> usize {
    cuts.iter()
        .position(|cut| value <= *cut)
        .unwrap_or(cuts.len())
}

pub(super) fn fnv1a64(value: &str) -> u64 {
    let mut result = 0xcbf29ce484222325u64;
    for byte in value.as_bytes() {
        result ^= u64::from(*byte);
        result = result.wrapping_mul(0x100000001b3);
    }
    result
}

pub(super) fn normalize_text(text: &str) -> String {
    collapse_whitespace(text).to_ascii_lowercase()
}

pub(super) fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(super) fn words(text: &str) -> Vec<String> {
    text.split(|ch: char| !(ch.is_ascii_alphanumeric() || matches!(ch, '\'' | '.' | '-')))
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
        .collect()
}

pub(super) fn word_count(text: &str) -> usize {
    words(&text.to_ascii_lowercase()).len()
}

pub(super) fn uppercase_ratio(text: &str) -> f32 {
    let mut letters = 0usize;
    let mut upper = 0usize;
    for ch in text.chars().filter(|ch| ch.is_alphabetic()) {
        letters += 1;
        if ch.is_uppercase() {
            upper += 1;
        }
    }
    upper as f32 / letters.max(1) as f32
}

pub(super) fn title_case_ratio(text: &str) -> f32 {
    let tokens = words(&text.to_ascii_lowercase());
    if tokens.is_empty() {
        return 0.0;
    }
    let title_words = text
        .split_whitespace()
        .filter_map(|word| {
            word.trim_matches(|ch: char| !ch.is_alphabetic())
                .chars()
                .next()
        })
        .filter(|ch| ch.is_uppercase())
        .count();
    title_words as f32 / tokens.len().max(1) as f32
}

pub(super) fn heading_text_like(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    uppercase_ratio(trimmed) >= 0.62
        || title_case_ratio(trimmed) >= 0.55
        || trimmed
            .split_whitespace()
            .next()
            .is_some_and(|word| matches!(word, "I." | "II." | "III." | "IV." | "V."))
}

pub(super) fn looks_like_note_start(text: &str) -> bool {
    let mut chars = text.trim_start().chars().peekable();
    let mut digits = 0usize;
    while chars.peek().is_some_and(|ch| ch.is_ascii_digit()) {
        digits += 1;
        let _ = chars.next();
    }
    digits > 0
        && digits <= 4
        && chars.peek().is_some_and(|ch| ch.is_whitespace())
        && chars
            .skip_while(|ch| ch.is_whitespace())
            .next()
            .is_some_and(|ch| ch.is_ascii_uppercase())
}

pub(super) fn looks_like_marginalia_note_block_start(text: &str) -> bool {
    if looks_like_note_start(text) {
        return true;
    }
    let mut chars = text.trim_start().chars().peekable();
    let mut digits = 0usize;
    let mut value = 0u32;
    while chars.peek().is_some_and(|ch| ch.is_ascii_digit()) {
        let Some(digit) = chars.next().and_then(|ch| ch.to_digit(10)) else {
            return false;
        };
        digits += 1;
        value = value * 10 + digit;
        if digits > 4 {
            return false;
        }
    }
    if digits == 0 || !(1..=u32::from(LM2_MAX_NOTE_MARKER)).contains(&value) {
        return false;
    }
    if !chars.next().is_some_and(|ch| matches!(ch, '.' | ')' | ']')) {
        return false;
    }
    if !chars.peek().is_some_and(|ch| ch.is_whitespace()) {
        return false;
    }
    chars
        .skip_while(|ch| ch.is_whitespace())
        .next()
        .is_some_and(|ch| ch.is_ascii_uppercase())
}

pub(super) fn has_legal_note_cue(lower: &str) -> bool {
    words(lower).iter().any(|token| {
        matches!(
            token.trim_matches(|ch: char| matches!(ch, ',' | ';' | ':' | ')' | '(')),
            "id." | "supra" | "infra" | "see" | "cf." | "u.s." | "s.ct." | "f.2d" | "f.3d"
        )
    })
}

pub(super) fn looks_like_toc_entry(lower: &str) -> bool {
    lower.contains("...")
        && lower
            .chars()
            .rev()
            .find(|ch| !ch.is_whitespace())
            .is_some_and(|ch| ch.is_ascii_digit())
}

pub(super) fn looks_like_running_header(lower: &str) -> bool {
    let words = words(lower);
    if words.len() < 4 || words.len() > 14 {
        return false;
    }
    let starts_with_page = words
        .first()
        .is_some_and(|word| word.chars().all(|ch| ch.is_ascii_digit()) && word.len() <= 4);
    let ends_with_page = words
        .last()
        .is_some_and(|word| word.chars().all(|ch| ch.is_ascii_digit()) && word.len() <= 4);
    (starts_with_page || ends_with_page)
        && (lower.contains("law review")
            || lower.contains("vol.")
            || lower.contains('[')
            || lower.contains(']'))
}

pub(super) fn looks_like_production_slug_boilerplate(text: &str) -> bool {
    let lower = normalize_text(text);
    lower.contains("printed in u.s.a")
        || (lower.contains("do not delete")
            && (lower.contains("(do not delete")
                || lower.contains("do not delete)")
                || lower.contains("printer")
                || lower.contains("proof")
                || lower.contains("_fmt")
                || lower.contains("5fmt")))
}

pub(super) fn looks_like_filename_fallback_title(text: &str) -> bool {
    let trimmed = text.trim();
    let lower = trimmed.to_ascii_lowercase();
    lower.ends_with(".pdf")
        || (trimmed.contains('_')
            && !trimmed.contains(' ')
            && lower.chars().filter(|ch| ch.is_ascii_alphabetic()).count() >= 8)
}

pub(super) fn lm2_fallback_title_from_blocks(blocks: &[LiquidBlock]) -> Option<String> {
    if let Some(title) = lm2_leading_title_from_blocks(blocks) {
        return Some(title);
    }

    if let Some(block) = blocks.iter().find(|block| {
        block.role == LiquidBlockRole::Title
            && lm2_title_candidate(&block.text)
            && !looks_like_lm2_author_heading(&block.text)
            && !lm2_front_matter_stop_text(&block.text)
    }) {
        return Some(block.text.clone());
    }

    let mut parts = Vec::new();
    let mut in_run = false;
    for block in blocks {
        if !matches!(
            block.role,
            LiquidBlockRole::Title | LiquidBlockRole::Heading | LiquidBlockRole::Subheading
        ) {
            if in_run {
                break;
            }
            continue;
        }
        if lm2_front_matter_stop_text(&block.text) {
            if in_run {
                break;
            }
            continue;
        }
        if !lm2_title_candidate(&block.text) {
            if in_run {
                break;
            }
            continue;
        }
        if looks_like_lm2_author_heading(&block.text) {
            break;
        }
        parts.push(block.text.clone());
        in_run = true;
        if parts.len() >= 3 {
            break;
        }
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}

pub(super) fn lm2_leading_title_from_blocks(blocks: &[LiquidBlock]) -> Option<String> {
    let mut parts = Vec::new();
    for (index, block) in blocks.iter().take(16).enumerate() {
        let text = block.text.trim();
        if text.is_empty() {
            continue;
        }
        if lm2_front_matter_stop_text(text) {
            if !parts.is_empty() {
                break;
            }
            continue;
        }
        if !parts.is_empty() && lm2_substantive_section_heading(text) {
            break;
        }
        if lm2_generic_title_label(text) {
            if !parts.is_empty() {
                break;
            }
            continue;
        }
        if looks_like_lm2_author_heading(text) || lm2_probable_author_after_title(block, &parts) {
            if !parts.is_empty() {
                break;
            }
            continue;
        }
        if lm2_probable_author_before_copyright(block, blocks.get(index + 1)) {
            continue;
        }
        if lm2_leading_title_candidate(block) {
            parts.push(text.to_owned());
            if parts.iter().map(|part| word_count(part)).sum::<usize>() >= 32 {
                break;
            }
        } else if !parts.is_empty() {
            break;
        }
    }
    (!parts.is_empty()).then(|| collapse_whitespace(&parts.join(" ")))
}

pub(super) fn lm2_probable_author_before_copyright(
    block: &LiquidBlock,
    next: Option<&LiquidBlock>,
) -> bool {
    if !matches!(
        block.role,
        LiquidBlockRole::Paragraph | LiquidBlockRole::Heading | LiquidBlockRole::AuthorInfo
    ) || !(2..=6).contains(&word_count(&block.text))
        || title_case_ratio(&block.text) < 0.75
    {
        return false;
    }
    let Some(next) = next else {
        return false;
    };
    let next = normalize_text(&next.text);
    let name = normalize_text(&block.text);
    next.starts_with("copyright ") && next.contains(&name)
}

pub(super) fn lm2_recover_leading_source_title(
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
) -> Option<Lm2RecoveredTitle> {
    let repository_covers = repository_cover_pages(decoded);
    let mut recovered = (0..=1)
        .filter(|page_index| !repository_covers.contains(page_index))
        .filter_map(|page_index| lm2_recover_source_title_on_page(decoded, page_index))
        .max_by_key(|recovered| word_count(&recovered.title))?;
    if let Some(mut metadata_title) = repository_citation_title(decoded, &repository_covers)
        && title_word_overlap(&metadata_title, &recovered.title) >= 0.70
    {
        // The repository citation is often a cleaner text object than the
        // display title rendered in an old embedded font. Keep the real title
        // lines as provenance, but use the cleaner metadata spelling.
        if recovered.title.trim_end().ends_with('?')
            && !metadata_title
                .trim_end()
                .chars()
                .next_back()
                .is_some_and(|ch| matches!(ch, '?' | '!' | '.'))
        {
            metadata_title.push('?');
        }
        recovered.title = metadata_title;
    }
    Some(recovered)
}

pub(super) fn repository_citation_title(
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
    cover_pages: &HashSet<usize>,
) -> Option<String> {
    for page_index in cover_pages {
        let mut lines = decoded
            .iter()
            .filter(|(line, _)| line.page_index == *page_index)
            .map(|(line, _)| line)
            .collect::<Vec<_>>();
        lines.sort_by_key(|line| line.line_index);
        let Some(heading) = lines.iter().position(|line| {
            normalize_text(&collapse_whitespace(&line.text)) == "recommended citation"
        }) else {
            continue;
        };
        let mut citation_lines = Vec::new();
        for line in lines.iter().skip(heading + 1).take(6) {
            let normalized = normalize_text(&line.text);
            if normalized.starts_with("available at")
                || normalized.starts_with("follow this")
                || normalized.starts_with("this article")
            {
                break;
            }
            citation_lines.push(clean_lm2_line_text(&line.text));
        }
        let citation = collapse_whitespace(&citation_lines.join(" "));
        let Some(suffix_comma) = citation.char_indices().rev().find_map(|(index, ch)| {
            if ch != ',' {
                return None;
            }
            citation[index + 1..]
                .trim_start()
                .chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_digit())
                .then_some(index)
        }) else {
            continue;
        };
        let title_and_author = citation[..suffix_comma].trim();
        // Repository citations commonly list several authors separated by
        // commas before the article title.  The title is the final citation
        // field before the volume/page suffix, not the field after the first
        // author.
        let Some(author_comma) = title_and_author.rfind(',') else {
            continue;
        };
        let title = collapse_whitespace(title_and_author[author_comma + 1..].trim());
        if (4..=40).contains(&word_count(&title)) && !title.contains("http") {
            return Some(title);
        }
    }
    None
}

pub(super) fn title_word_overlap(left: &str, right: &str) -> f32 {
    let words = |value: &str| {
        normalize_text(value)
            .split_whitespace()
            .filter(|word| word.len() >= 2)
            .map(str::to_owned)
            .collect::<HashSet<_>>()
    };
    let left = words(left);
    let right = words(right);
    let denominator = left.len().min(right.len());
    if denominator == 0 {
        return 0.0;
    }
    left.intersection(&right).count() as f32 / denominator as f32
}

pub(super) fn lm2_recover_source_title_on_page(
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
    page_index: usize,
) -> Option<Lm2RecoveredTitle> {
    let mut parts = Vec::new();
    let mut lines = Vec::new();
    let mut has_hidden_part = false;
    let mut article_label_seen = false;

    for (line, action) in decoded.iter().take(128) {
        if line.page_index < page_index {
            continue;
        }
        if line.page_index > page_index {
            break;
        }
        if line.line_index > 32 {
            break;
        }

        let mut text = clean_lm2_line_text(&line.text);
        if text.is_empty() || looks_like_production_slug_boilerplate(&text) {
            continue;
        }
        if lm2_front_matter_stop_text(&text) || lm2_generic_title_label(&text) {
            if matches!(normalize_text(&text).as_str(), "article" | "articles") {
                parts.clear();
                lines.clear();
                has_hidden_part = false;
                article_label_seen = true;
                continue;
            }
            let preceding_run = normalize_text(&parts.join(" "));
            let journal_masthead_before_article_label = lm2_generic_title_label(&text)
                && (preceding_run.ends_with(" law review")
                    || preceding_run.ends_with(" law journal"));
            if journal_masthead_before_article_label {
                parts.clear();
                lines.clear();
                has_hidden_part = false;
                continue;
            }
            if !parts.is_empty() {
                break;
            }
            continue;
        }
        let post_article_uppercase_title = article_label_seen
            && line.page_index == 0
            && line.line_index <= 8
            && (2..=16).contains(&word_count(&text))
            && uppercase_ratio(&text) >= 0.82
            && !text.contains('&')
            && !text.trim_end().ends_with('.')
            && lm2_title_candidate(&text);
        let symbolic_title_continuation =
            !parts.is_empty() && (text.ends_with('*') || text.ends_with('\u{2217}')) && {
                let without_symbol = text
                    .trim_end_matches(|ch| matches!(ch, '*' | '\u{2217}'))
                    .trim_end();
                lm2_leading_source_title_candidate(line, without_symbol)
                    && (uppercase_ratio(without_symbol) >= 0.72
                        || !lm2_source_title_author_like(line, without_symbol))
            };
        if symbolic_title_continuation {
            text = text
                .trim_end_matches(|ch| matches!(ch, '*' | '\u{2217}'))
                .trim_end()
                .to_owned();
        }
        if !symbolic_title_continuation
            && !post_article_uppercase_title
            && (looks_like_lm2_author_heading(&text) || lm2_source_title_author_like(line, &text))
        {
            if !parts.is_empty() {
                break;
            }
            continue;
        }
        if lm2_leading_source_title_candidate(line, &text) || post_article_uppercase_title {
            if *action == Lm2Action::HideNoise {
                has_hidden_part = true;
            }
            if parts.last().is_some_and(|part| part == &text) {
                continue;
            }
            parts.push(text);
            lines.push(line.clone());
            if parts.iter().map(|part| word_count(part)).sum::<usize>() >= 32 || parts.len() >= 4 {
                break;
            }
        } else if !parts.is_empty() {
            break;
        }
    }

    if parts.is_empty() {
        return None;
    }
    let title = join_lm2_title_parts(&parts);
    let title_words = word_count(&title);
    let leading_uppercase_paragraph_title = lines.first().is_some_and(|line| {
        line.page_index == 0
            && line.line_index <= 8
            && uppercase_ratio(&title) >= 0.82
            && lines.iter().all(|line| {
                line.page_index == 0
                    && line.line_index <= 8
                    && matches!(
                        line.role_hint,
                        None | Some(
                            LiquidBlockRole::Title
                                | LiquidBlockRole::Heading
                                | LiquidBlockRole::Paragraph
                        )
                    )
            })
    });
    let authoritative_display = leading_uppercase_paragraph_title
        || lines.first().is_some_and(|line| {
            line.page_index <= 1
                && line.line_index <= 8
                && line.centered
                && (line.font_ratio_page >= 1.18 || line.font_ratio_page_ref >= 1.18)
                && (uppercase_ratio(&title) >= 0.50 || title_case_ratio(&title) >= 0.55)
        }) && (2..=32).contains(&title_words);
    let strong_short_title = lines.len() == 1
        && lines[0].centered
        && lines[0].font_ratio_page >= 1.25
        && (lines[0].bold || uppercase_ratio(&lines[0].text) >= 0.72);
    (has_hidden_part || authoritative_display)
        .then_some(())
        .filter(|_| title_words >= 4 || title_words >= 2 && strong_short_title)
        .map(|_| Lm2RecoveredTitle {
            title,
            lines,
            authoritative_display,
        })
}

/// Join physical title lines without inventing a space after a discretionary
/// line-end hyphen.  The source lines distinguish `Court-` + `Defying` from a
/// deliberate in-line `word - word`, so this is safer than a global Markdown
/// replacement.
pub(super) fn join_lm2_title_parts(parts: &[String]) -> String {
    let mut joined = String::new();
    for part in parts {
        if !joined.is_empty() && !joined.trim_end().ends_with('-') {
            joined.push(' ');
        }
        joined.push_str(part.trim());
    }
    collapse_whitespace(&joined)
}

/// Recover a journal byline whose display letterspacing was flattened into
/// short lowercase chunks (`na di a banteka`).  This is restricted to a
/// first-page Noise block immediately before the recovered title and allows
/// only a bare folio alongside it.
pub(super) fn apply_leading_letterspaced_author_recovery(
    blocks: &mut [LiquidBlock],
    sources: &mut [LiquidBlockSourceLines],
) -> usize {
    let title_line = sources
        .iter()
        .filter(|source| {
            blocks
                .get(source.block_index)
                .is_some_and(|block| block.role == LiquidBlockRole::Title)
        })
        .flat_map(|source| source.lines.iter())
        .filter(|line| line.page_index == 0)
        .map(|line| line.line_index)
        .min()
        .unwrap_or(usize::MAX);

    for source in sources.iter_mut() {
        let Some(block) = blocks.get_mut(source.block_index) else {
            continue;
        };
        if block.role != LiquidBlockRole::Noise
            || !source.lines.iter().all(|line| {
                line.page_index == 0
                    && (line.text.trim().chars().all(|ch| ch.is_ascii_digit())
                        || compact_letterspaced_author_name(&line.text).is_some())
            })
        {
            continue;
        }
        let Some((line_position, author)) =
            source
                .lines
                .iter()
                .enumerate()
                .find_map(|(position, line)| {
                    (line.line_index < title_line)
                        .then(|| compact_letterspaced_author_name(&line.text))
                        .flatten()
                        .map(|author| (position, author))
                })
        else {
            continue;
        };
        let mut author_line = source.lines[line_position].clone();
        author_line.text.clone_from(&author);
        author_line.role = LiquidBlockRole::AuthorInfo;
        block.role = LiquidBlockRole::AuthorInfo;
        block.text = author;
        block.label = None;
        source.lines = vec![author_line];
        return 1;
    }
    0
}

pub(super) fn compact_letterspaced_author_name(text: &str) -> Option<String> {
    let tokens = text.split_whitespace().collect::<Vec<_>>();
    if !(3..=6).contains(&tokens.len())
        || !tokens
            .iter()
            .all(|token| token.chars().all(|ch| ch.is_ascii_alphabetic()))
        || !tokens[..tokens.len() - 1]
            .iter()
            .all(|token| token.len() <= 2)
        || !(3..=20).contains(&tokens.last()?.len())
    {
        return None;
    }
    let first = tokens[..tokens.len() - 1].join("");
    if !(3..=12).contains(&first.len()) {
        return None;
    }
    let title_case = |word: &str| {
        let mut chars = word.chars();
        chars
            .next()
            .map(|first| {
                first
                    .to_uppercase()
                    .chain(chars.flat_map(char::to_lowercase))
                    .collect::<String>()
            })
            .unwrap_or_default()
    };
    Some(format!(
        "{} {}",
        title_case(&first),
        title_case(tokens.last().copied().unwrap_or_default())
    ))
}

pub(super) fn lm2_leading_source_title_candidate(line: &DeepLiquidSourceLine, text: &str) -> bool {
    if !lm2_title_candidate(text) || looks_like_lm2_author_heading(text) {
        return false;
    }
    let strong_title_geometry = line.page_index <= 1
        && line.line_index <= 8
        && line.centered
        && line.bold
        && line.font_ratio_page >= 1.25;
    let synthetic_ocr_heading = line.synthetic_text_geometry
        && line.page_index == 0
        && line.line_index <= 4
        && line.role_hint == Some(LiquidBlockRole::Heading)
        && (4..=16).contains(&word_count(text))
        && uppercase_ratio(text) >= 0.72;
    if line
        .role_hint
        .is_some_and(|role| role_action(role) == Lm2Action::HideNoise)
        && !strong_title_geometry
        && !synthetic_ocr_heading
    {
        return false;
    }
    let words = word_count(text);
    if words > 16 || text.trim_end().ends_with('.') {
        return false;
    }
    if matches!(
        line.role_hint,
        Some(LiquidBlockRole::Title | LiquidBlockRole::Heading | LiquidBlockRole::Subheading)
    ) {
        return !line.synthetic_text_geometry || synthetic_ocr_heading;
    }
    (line.centered || line.font_ratio_page >= 1.14)
        && (uppercase_ratio(text) >= 0.50 || title_case_ratio(text) >= 0.55)
}

pub(super) fn lm2_source_title_author_like(line: &DeepLiquidSourceLine, text: &str) -> bool {
    if line.synthetic_text_geometry && line.role_hint == Some(LiquidBlockRole::Heading) {
        return false;
    }
    let words = word_count(text);
    let punctuated_name = (text.contains('.') || text.contains('\'') || text.contains('’'))
        && title_case_ratio(text) >= 0.55;
    let conjoined_names = text.split_once('&').is_some_and(|(left, right)| {
        [left, right].iter().all(|part| {
            let words = word_count(part);
            (2..=4).contains(&words) && title_case_ratio(part) >= 0.72
        })
    });
    words >= 2
        && ((words <= 4 && punctuated_name)
            || words <= 8 && conjoined_names
            || line.font_ratio_page < 1.12
                && words <= 4
                && (uppercase_ratio(text) >= 0.85 || title_case_ratio(text) >= 0.85))
}

pub(super) fn lm2_recovered_title_is_better(recovered: &str, current: &str) -> bool {
    let recovered = recovered.trim();
    let current = current.trim();
    let recovered_normalized = normalize_text(recovered);
    let current_normalized = normalize_text(current);
    let current_appends_author = current_normalized
        .strip_prefix(&recovered_normalized)
        .map(str::trim)
        .is_some_and(|suffix| !suffix.is_empty() && word_count(suffix) <= 8);
    !current.is_empty()
        && (current_appends_author
            || (title_word_overlap(recovered, current) >= 0.80
                && title_case_ratio(recovered) >= 0.55
                && uppercase_ratio(current) >= 0.70)
            || (word_count(recovered) >= word_count(current) + 2
                && normalize_text(recovered).starts_with(&normalize_text(current)))
            || word_count(recovered) >= word_count(current) + 3
                && normalize_text(recovered).ends_with(&normalize_text(current)))
}

pub(super) fn lm2_title_candidate(text: &str) -> bool {
    let lower = normalize_text(text);
    let trimmed = text.trim();
    !trimmed.is_empty()
        && !lm2_generic_title_label(trimmed)
        && !lm2_front_matter_stop_text(trimmed)
        && !lm2_journal_issue_masthead_text(trimmed)
        && word_count(trimmed) <= 24
        && lower.chars().filter(|ch| ch.is_ascii_alphabetic()).count() >= 4
}

pub(super) fn lm2_journal_issue_masthead_text(text: &str) -> bool {
    let lower = normalize_text(text);
    lower.starts_with("volume ")
        && (lower.contains(" number ")
            || lower.contains(" issue ")
            || lower.split_whitespace().any(|word| word == "no"))
}

pub(super) fn lm2_leading_title_candidate(block: &LiquidBlock) -> bool {
    if !matches!(
        block.role,
        LiquidBlockRole::Title
            | LiquidBlockRole::Heading
            | LiquidBlockRole::Subheading
            | LiquidBlockRole::Paragraph
    ) {
        return false;
    }
    let text = block.text.trim();
    if !lm2_title_candidate(text) || looks_like_lm2_author_heading(text) {
        return false;
    }
    let lower = normalize_text(text);
    let words = word_count(text);
    (words <= 16 || words <= 24 && uppercase_ratio(text) >= 0.62)
        && !text.trim_end().ends_with('.')
        && !lower.starts_with("abstract:")
        && !lower.starts_with("copyright ")
        && (uppercase_ratio(text) >= 0.62
            || title_case_ratio(text) >= 0.55
            || matches!(
                block.role,
                LiquidBlockRole::Title | LiquidBlockRole::Heading | LiquidBlockRole::Subheading
            ))
}

pub(super) fn lm2_substantive_section_heading(text: &str) -> bool {
    let lower = normalize_text(text);
    lower == "introduction"
        || lower == "conclusion"
        || ["i. ", "ii. ", "iii. ", "iv. ", "v. ", "vi. "]
            .iter()
            .any(|prefix| lower.starts_with(prefix))
}

pub(super) fn lm2_generic_title_label(text: &str) -> bool {
    matches!(
        normalize_text(text).as_str(),
        "notes"
            | "note"
            | "article"
            | "articles"
            | "abstract"
            | "contents"
            | "table of contents"
            | "introduction"
    )
}

pub(super) fn lm2_front_matter_stop_text(text: &str) -> bool {
    let lower = normalize_text(text);
    lower.starts_with("abstract:")
        || lower.contains("................................................................")
        || looks_like_toc_entry(&lower)
        || lower.starts_with("copyright ")
}

pub(super) fn lm2_probable_author_after_title(block: &LiquidBlock, title_parts: &[String]) -> bool {
    if title_parts.is_empty() || block.role != LiquidBlockRole::Title {
        return false;
    }
    let text = block.text.trim();
    let count = word_count(text);
    count >= 2
        && count <= 4
        && !text.contains(':')
        && !text.contains('?')
        && !text.contains('!')
        && (uppercase_ratio(text) >= 0.85 || title_case_ratio(text) >= 0.85)
}

pub(super) fn looks_like_lm2_author_heading(text: &str) -> bool {
    let lower = normalize_text(text);
    text.contains('†')
        || text.contains('*')
        || text.contains('∗')
        || lower.contains("j.d.")
        || lower.contains("candidate")
}

pub(super) fn is_edge_line(line: &DeepLiquidSourceLine) -> bool {
    let height = line.page_height.max(1.0);
    let top = line.top / height;
    let bottom = line.bottom / height;
    top > 0.92 || bottom < 0.07
}

pub(super) fn vertical_gap(previous: &DeepLiquidSourceLine, line: &DeepLiquidSourceLine) -> f32 {
    if previous.page_height <= 0.0 {
        return 0.0;
    }
    ((previous.bottom - line.top).abs() / previous.page_height).max(0.0)
}

pub(super) fn read_json_file<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, String> {
    let bytes = std::fs::read(path)
        .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("Could not parse {}: {error}", path.display()))
}

#[cfg(any(feature = "devtools", test))]
pub(super) fn reject_label_like_path(path: &Path) -> Result<(), String> {
    let lower = path
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase();
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let in_eval_or_training = lower.starts_with("eval/") || lower.starts_with("training-data/");
    let label_like = file_name.contains("label")
        || file_name.contains("adjudication")
        || file_name.contains("gold");
    if in_eval_or_training && label_like {
        return Err(format!(
            "refusing label-like LM2 draft input: {}",
            path.display()
        ));
    }
    Ok(())
}

pub(super) fn eval_key(path: &str, page_index: usize, line_index: usize, text: &str) -> String {
    format!(
        "{}\u{1f}{page_index}\u{1f}{line_index}\u{1f}{}",
        path.to_ascii_lowercase(),
        collapse_whitespace(text)
    )
}

pub(super) fn annotate_pp_priors_for_lines(
    runtime: &Lm2Runtime,
    path: &str,
    lines: &mut [DeepLiquidSourceLine],
) {
    for line in lines {
        annotate_pp_prior(runtime, path, line);
    }
}
