//! Local semantic classification and display labels for Liquid Mode blocks.
//!
//! This module is a verified move from the former monolithic `mod.rs`.
//! It owns text-role predicates and label derivation, while `mod.rs` still
//! owns the broader normalization pipeline that calls into these predicates.

use crate::liquid::cleaning::{looks_like_citation_footnote_line, looks_like_footnote_line};
use crate::liquid::model::LiquidBlockRole;
use crate::liquid::util::{
    is_letter_heading_marker, split_bare_outline_marker, starts_with_roman_heading,
    title_case_ratio, uppercase_ratio, word_count,
};

use super::{
    contains_reference_year, end_matter_label, front_matter_label_for_text,
    is_non_title_heading_text, looks_like_short_all_caps_person_name, normalize_reference_heading,
};
pub(super) fn classify_block(text: &str, index: usize) -> LiquidBlockRole {
    if looks_like_letter_salutation(text) || looks_like_letter_closing(text) {
        return LiquidBlockRole::Paragraph;
    }
    if looks_like_author_info(text, index) {
        return LiquidBlockRole::AuthorInfo;
    }
    if looks_like_abstract(text) {
        return LiquidBlockRole::Abstract;
    }
    if looks_like_syllabus(text) {
        return LiquidBlockRole::Syllabus;
    }
    if looks_like_article_metadata(text, index) {
        return LiquidBlockRole::Metadata;
    }
    if looks_like_exam_metadata(text) {
        return LiquidBlockRole::Metadata;
    }
    if looks_like_issue(text) {
        return LiquidBlockRole::Issue;
    }
    if looks_like_answer(text) {
        return LiquidBlockRole::Explainer;
    }
    if looks_like_holding(text) {
        return LiquidBlockRole::Holding;
    }
    if looks_like_takeaway(text) {
        return LiquidBlockRole::Takeaway;
    }
    if looks_like_explainer(text) {
        return LiquidBlockRole::Explainer;
    }
    if looks_like_caption(text, index) {
        return LiquidBlockRole::Caption;
    }
    if looks_like_table(text) {
        return LiquidBlockRole::Table;
    }
    if end_matter_label(text).is_some() {
        return LiquidBlockRole::Heading;
    }
    if index == 0 && text.len() < 160 && uppercase_ratio(text) > 0.55 {
        return LiquidBlockRole::Heading;
    }
    if looks_like_marginalia(text) {
        return LiquidBlockRole::Marginalia;
    }
    if let Some(role) = heading_role_for_text(text) {
        return role;
    }
    if looks_like_definition(text) {
        return LiquidBlockRole::Definition;
    }
    if looks_like_citation_footnote_line(text) {
        return LiquidBlockRole::Footnote;
    }
    if looks_like_list_item(text) {
        return LiquidBlockRole::ListItem;
    }
    if looks_like_footnote_line(text) {
        return LiquidBlockRole::Footnote;
    }
    if looks_like_clause(text) {
        return LiquidBlockRole::Clause;
    }
    if text.starts_with('"') || text.starts_with('“') {
        return LiquidBlockRole::Quote;
    }
    if contains_key_clause_language(text) {
        return LiquidBlockRole::KeyClause;
    }
    LiquidBlockRole::Paragraph
}

pub(super) fn looks_like_heading(text: &str) -> bool {
    heading_role_for_text(text).is_some()
}

fn heading_role_for_text(text: &str) -> Option<LiquidBlockRole> {
    let trimmed = text.trim();
    if looks_like_citation_footnote_line(trimmed) {
        return None;
    }
    if looks_like_dissent_or_concurrence_heading(trimmed) {
        return Some(LiquidBlockRole::Heading);
    }
    let lower = trimmed.to_ascii_lowercase();
    let known_heading = matches!(
        lower.as_str(),
        "abstract"
            | "introduction"
            | "background"
            | "overview"
            | "analysis"
            | "discussion"
            | "conclusion"
            | "conclusions"
            | "methodology"
            | "methods"
            | "materials and methods"
            | "literature review"
            | "related work"
            | "results"
            | "findings"
            | "implications"
            | "limitations"
            | "future research"
            | "future work"
            | "notes"
            | "references"
    );
    let starts_with_legal_heading = looks_like_structural_legal_heading(trimmed);
    let title_like = trimmed.len() <= 92
        && word_count(trimmed) <= 12
        && !trimmed.ends_with('.')
        && title_case_ratio(trimmed) > 0.58
        && trimmed.chars().any(char::is_alphabetic);

    if known_heading || starts_with_legal_heading || starts_with_roman_heading(trimmed) {
        return Some(LiquidBlockRole::Heading);
    }
    if starts_with_lettered_heading(trimmed) {
        return Some(LiquidBlockRole::Subheading);
    }
    if starts_with_numbered_heading(trimmed) {
        return Some(LiquidBlockRole::Heading);
    }
    if trimmed.len() <= 92
        && uppercase_ratio(trimmed) > 0.72
        && trimmed.chars().any(char::is_alphabetic)
    {
        return Some(LiquidBlockRole::Heading);
    }
    if title_like {
        return Some(if trimmed.len() < 70 {
            LiquidBlockRole::Heading
        } else {
            LiquidBlockRole::Subheading
        });
    }
    None
}

pub(super) fn starts_with_lettered_heading(text: &str) -> bool {
    let trimmed = text.trim_start();
    if let Some(rest) = trimmed
        .strip_prefix('(')
        .and_then(|value| value.get(1..))
        .and_then(|value| value.strip_prefix(") "))
    {
        return trimmed
            .chars()
            .nth(1)
            .is_some_and(|ch| ch.is_ascii_uppercase())
            && looks_like_heading_remainder(rest);
    }

    if let Some((prefix, rest)) = trimmed.split_once('.') {
        if is_letter_heading_marker(prefix) && looks_like_heading_remainder(rest) {
            return true;
        }
    }

    split_bare_outline_marker(trimmed).is_some_and(|(prefix, rest)| {
        is_letter_heading_marker(prefix) && looks_like_heading_remainder(rest)
    })
}

pub(super) fn starts_with_numbered_heading(text: &str) -> bool {
    let trimmed = text.trim_start();
    if looks_like_citation_footnote_line(trimmed) {
        return false;
    }

    if let Some((prefix, rest)) = trimmed.split_once('.').or_else(|| trimmed.split_once(')')) {
        if is_simple_number_heading_marker(prefix) && looks_like_heading_remainder(rest) {
            return true;
        }
    }

    split_bare_numbered_heading_marker(trimmed).is_some_and(|(prefix, rest)| {
        is_numbered_heading_marker(prefix) && looks_like_heading_remainder(rest)
    })
}

fn split_bare_numbered_heading_marker(text: &str) -> Option<(&str, &str)> {
    let trimmed = text.trim_start();
    let marker_end = trimmed.find(|ch: char| ch.is_whitespace())?;
    let prefix = &trimmed[..marker_end];
    let rest = trimmed[marker_end..].trim_start();
    (!prefix.is_empty() && !rest.is_empty()).then_some((prefix, rest))
}

fn is_simple_number_heading_marker(prefix: &str) -> bool {
    !prefix.is_empty() && prefix.len() <= 3 && prefix.chars().all(|ch| ch.is_ascii_digit())
}

fn is_numbered_heading_marker(prefix: &str) -> bool {
    if prefix.len() > 8 || prefix.starts_with('.') || prefix.ends_with('.') {
        return false;
    }
    let mut components = 0usize;
    for part in prefix.split('.') {
        if part.is_empty() || part.len() > 3 || !part.chars().all(|ch| ch.is_ascii_digit()) {
            return false;
        }
        components += 1;
    }
    (1..=4).contains(&components)
}

fn looks_like_structural_legal_heading(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.ends_with('.') || word_count(trimmed) > 12 {
        return false;
    }

    let upper = trimmed.to_ascii_uppercase();
    if matches!(upper.as_str(), "RECITALS" | "WHEREAS") {
        return starts_with_uppercase_word(trimmed);
    }

    let Some((prefix, rest)) = trimmed.split_once(char::is_whitespace) else {
        return false;
    };
    if !matches!(
        prefix.to_ascii_uppercase().as_str(),
        "ARTICLE" | "SECTION" | "PART" | "EXHIBIT" | "SCHEDULE" | "APPENDIX"
    ) || !starts_with_uppercase_word(prefix)
    {
        return false;
    }

    let rest = rest.trim();
    if rest.is_empty() {
        return false;
    }
    let first = rest
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_matches(|ch: char| matches!(ch, '.' | ',' | ':' | ';' | ')' | '(' | '[' | ']'));
    if first.is_empty() {
        return false;
    }
    if first.chars().any(|ch| ch.is_ascii_digit()) {
        return true;
    }
    let first_upper = first.to_ascii_uppercase();
    if first == first_upper
        && first_upper
            .chars()
            .all(|ch| matches!(ch, 'I' | 'V' | 'X' | 'L' | 'C' | 'D' | 'M' | 'A'..='Z'))
    {
        return true;
    }
    title_case_ratio(rest) > 0.58 || uppercase_ratio(rest) > 0.72
}

fn starts_with_uppercase_word(text: &str) -> bool {
    text.chars()
        .find(|ch| ch.is_ascii_alphabetic())
        .is_some_and(|ch| ch.is_ascii_uppercase())
}

fn looks_like_heading_remainder(rest: &str) -> bool {
    let rest = rest.trim();
    if rest.is_empty() || rest.ends_with('.') || word_count(rest) > 12 {
        return false;
    }
    title_case_ratio(rest) > 0.58 && rest.chars().any(char::is_alphabetic)
}

pub(super) fn looks_like_toc_entry(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.len() < 4 || trimmed.len() > 180 || word_count(trimmed) > 24 {
        return false;
    }
    if !ends_with_page_locator(trimmed) {
        return false;
    }
    if has_dot_leader(trimmed) {
        return true;
    }

    let starts_with_outline_marker = starts_with_roman_heading(trimmed)
        || starts_with_numbered_heading(trimmed)
        || starts_with_lettered_heading(trimmed);
    let title_like = title_case_ratio(trimmed) > 0.45
        && !trimmed.ends_with('.')
        && trimmed.chars().any(char::is_alphabetic);
    starts_with_outline_marker || title_like
}

fn has_dot_leader(text: &str) -> bool {
    text.contains("...") || text.contains(". .") || text.contains('\u{2026}')
}

fn ends_with_page_locator(text: &str) -> bool {
    let Some(token) = text.split_whitespace().last() else {
        return false;
    };
    let token = token.trim_matches(|ch: char| matches!(ch, '.' | ',' | ';' | ':' | '(' | ')'));
    if is_page_locator_token(token) {
        return true;
    }

    has_dot_leader(token)
        && token
            .rsplit('.')
            .find(|part| !part.is_empty())
            .is_some_and(is_page_locator_token)
}

fn is_page_locator_token(token: &str) -> bool {
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

pub(super) fn looks_like_exam_metadata(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("contracts exam - part")
        && lower.contains("character limit:")
        && lower.contains("characters:")
}

pub(super) fn looks_like_article_metadata(text: &str, index: usize) -> bool {
    if index > 12 {
        return false;
    }
    let lower = text.to_ascii_lowercase();
    if looks_like_front_matter_metadata(text) {
        return true;
    }
    if index <= 4 && looks_like_news_kicker_metadata(text) {
        return true;
    }
    if index > 8 || (text.len() > 140 && !looks_like_repository_citation(text)) {
        return false;
    }
    if lower.starts_with("published ")
        || lower.starts_with("updated ")
        || lower.starts_with("last updated")
        || lower.starts_with("filed ")
        || lower.starts_with("posted ")
        || lower.starts_with("date:")
        || lower.starts_with("source:")
        || lower.starts_with("doi:")
        || lower.starts_with("volume ")
        || lower.starts_with("vol. ")
        || lower.starts_with("issue ")
        || lower.ends_with(" min read")
        || lower.ends_with(" minute read")
        || lower == "reuters"
        || lower == "associated press"
        || lower == "ap"
    {
        return true;
    }

    looks_like_standalone_date(text)
        || looks_like_publication_name(text)
        || looks_like_repository_citation(text)
}

pub(super) fn looks_like_news_kicker_metadata(text: &str) -> bool {
    let normalized = normalize_reference_heading(text);
    matches!(
        normalized.as_str(),
        "opinion"
            | "analysis"
            | "news analysis"
            | "commentary"
            | "editorial"
            | "essay"
            | "guest essay"
            | "review"
            | "general article"
            | "books"
            | "politics"
            | "world"
            | "us"
            | "u.s"
            | "u s"
            | "business"
            | "technology"
            | "tech"
            | "science"
            | "health"
            | "sports"
            | "arts"
            | "style"
            | "travel"
            | "magazine"
            | "the daily"
    )
}

pub(super) fn looks_like_front_matter_metadata(text: &str) -> bool {
    if text.len() > 320 {
        return false;
    }

    let lower = text.to_ascii_lowercase();
    front_matter_label_for_text(text).is_some()
        || lower.starts_with("keywords:")
        || lower.starts_with("keywords ")
        || lower.starts_with("key words:")
        || lower.starts_with("key words ")
        || lower.starts_with("doi:")
        || lower.starts_with("to:")
        || lower.starts_with("from:")
        || lower.starts_with("re:")
        || lower.starts_with("date:")
        || lower.starts_with("doi ")
        || lower.starts_with("https://doi.org/")
        || lower.starts_with("http://doi.org/")
        || lower.starts_with("orcid:")
        || lower.starts_with("orcid ")
        || lower.starts_with("jel classification")
        || lower.starts_with("jel classifications")
        || lower.starts_with("received:")
        || lower.starts_with("received ")
        || lower.starts_with("accepted:")
        || lower.starts_with("accepted ")
        || lower.starts_with("revised:")
        || lower.starts_with("revised ")
        || lower.starts_with("published online")
        || lower.starts_with("available online")
        || lower.starts_with("article history")
        || lower.starts_with("publication history")
        || lower.starts_with("citation:")
        || lower.starts_with("recommended citation")
        || lower.starts_with("suggested citation")
        || lower.starts_with("corresponding author")
        || lower.starts_with("correspondence:")
        || lower.starts_with("funding:")
        || lower.starts_with("conflict of interest")
        || lower.starts_with("conflicts of interest")
}

fn looks_like_repository_citation(text: &str) -> bool {
    if text.len() > 260 || word_count(text) < 6 || !contains_reference_year(text) {
        return false;
    }

    let lower = text.to_ascii_lowercase();
    [
        " law review",
        " law journal",
        " l. rev.",
        " journal",
        " review ",
        "available at:",
        "doi:",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn looks_like_standalone_date(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.len() > 80 || trimmed.ends_with('.') {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    let starts_with_month = [
        "january ",
        "february ",
        "march ",
        "april ",
        "may ",
        "june ",
        "july ",
        "august ",
        "september ",
        "october ",
        "november ",
        "december ",
    ]
    .iter()
    .any(|month| lower.starts_with(month));
    (starts_with_month && trimmed.chars().any(|ch| ch.is_ascii_digit()))
        || looks_like_numeric_date(trimmed)
}

fn looks_like_numeric_date(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.len() < 6
        || trimmed.len() > 16
        || !trimmed
            .chars()
            .all(|ch| ch.is_ascii_digit() || matches!(ch, '-' | '/' | '.'))
    {
        return false;
    }

    let parts = trimmed
        .split(['-', '/', '.'])
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.len() != 3 {
        return false;
    }

    let Ok(first) = parts[0].parse::<u32>() else {
        return false;
    };
    let Ok(second) = parts[1].parse::<u32>() else {
        return false;
    };
    let Ok(third) = parts[2].parse::<u32>() else {
        return false;
    };

    let plausible_year = |value: u32, width: usize| {
        (width == 4 && (1800..=2099).contains(&value)) || (width == 2 && value <= 99)
    };
    let plausible_month = |value: u32| (1..=12).contains(&value);
    let plausible_day = |value: u32| (1..=31).contains(&value);

    if parts[0].len() == 4 {
        return plausible_year(first, parts[0].len())
            && plausible_month(second)
            && plausible_day(third);
    }

    plausible_month(first) && plausible_day(second) && plausible_year(third, parts[2].len())
}

pub(super) fn looks_like_publication_name(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.len() > 72 || trimmed.ends_with('.') || word_count(trimmed) > 8 {
        return false;
    }
    if trimmed.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    if looks_like_letter_salutation(trimmed)
        || lower.starts_with("letter to ")
        || lower.starts_with("reply to ")
        || lower.starts_with("response to ")
    {
        return false;
    }
    let publication_words = [
        "times",
        "post",
        "journal",
        "tribune",
        "herald",
        "gazette",
        "magazine",
        "law review",
        "law journal",
    ];
    title_case_ratio(trimmed) > 0.65 && publication_words.iter().any(|word| lower.contains(word))
}

fn looks_like_letter_salutation(text: &str) -> bool {
    let trimmed = text.trim();
    let lower = trimmed.to_ascii_lowercase();
    lower.starts_with("dear ") && trimmed.ends_with(',') && word_count(trimmed) <= 12
}

fn looks_like_letter_closing(text: &str) -> bool {
    let normalized = text
        .trim()
        .trim_end_matches(',')
        .trim()
        .to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "sincerely"
            | "respectfully"
            | "yours truly"
            | "very truly yours"
            | "best"
            | "best regards"
            | "kind regards"
            | "regards"
    )
}

fn looks_like_definition(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains(" means ") || lower.contains(" shall mean ") || lower.contains(" is defined as ")
}

pub(super) fn looks_like_marginalia(text: &str) -> bool {
    split_marginalia_field(text).is_some()
}

fn split_marginalia_field(text: &str) -> Option<(&str, &str)> {
    let trimmed = text.trim();
    if trimmed.len() < 5 || trimmed.len() > 700 || trimmed.ends_with('.') {
        return None;
    }
    if starts_with_reader_aid_prefix(trimmed) || looks_like_caption(trimmed, 0) {
        return None;
    }

    let (raw_label, raw_body) = trimmed.split_once(':')?;
    let label = raw_label.trim();
    let body = raw_body.trim();
    if label.is_empty()
        || body.is_empty()
        || label.chars().count() > 48
        || !(1..=7).contains(&word_count(label))
        || body.chars().count() > 620
    {
        return None;
    }

    let normalized = normalize_reference_heading(label);
    let known_contract_field = matches!(
        normalized.as_str(),
        "engagement"
            | "artist"
            | "artists"
            | "performance date"
            | "performance dates"
            | "location venue"
            | "duration of performance"
            | "radius clause"
            | "artist fee"
            | "first deposit"
            | "second deposit"
            | "bank name"
            | "name on account"
            | "aba routing number"
            | "account number"
            | "swift code"
            | "address"
            | "adress"
            | "name"
            | "date"
    );
    let aligned_colon = raw_label
        .chars()
        .last()
        .is_some_and(|ch| ch.is_whitespace());
    let label_is_title_like =
        label.chars().any(char::is_alphabetic) && title_case_ratio(label) > 0.62;

    (known_contract_field || (aligned_colon && label_is_title_like)).then_some((label, body))
}

pub(super) fn looks_like_author_info(text: &str, index: usize) -> bool {
    if text.len() > 220 {
        return false;
    }
    if index > 4 {
        return index <= 80 && looks_like_late_all_caps_author_line(text);
    }
    if front_matter_label_for_text(text).is_some() {
        return false;
    }
    let lower = text.to_ascii_lowercase();
    if lower.starts_with("by the numbers") {
        return false;
    }
    lower.starts_with("by ")
        || lower.starts_with("byline")
        || lower.contains("professor of")
        || lower.contains("associate professor")
        || lower.contains("assistant professor")
        || lower.contains("university school of law")
        || lower.contains("law school")
        || lower.contains("journalist")
        || looks_like_author_affiliation(text)
        || looks_like_standalone_author_line(text, index)
}

fn looks_like_late_all_caps_author_line(text: &str) -> bool {
    let name = text
        .trim()
        .trim_end_matches(|ch: char| matches!(ch, '*' | '†' | '‡') || ch.is_ascii_digit())
        .trim_end();
    !name.is_empty()
        && !name.chars().any(|ch| ch.is_ascii_digit())
        && looks_like_short_all_caps_person_name(name)
}

fn looks_like_author_affiliation(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.len() < 5 || trimmed.len() > 240 {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains('@') || lower.contains("orcid") {
        return true;
    }
    if lower.starts_with("affiliation:") || lower.starts_with("affiliations:") {
        return true;
    }
    if lower.contains("faculty of law")
        || lower.contains("school of law")
        || lower.contains("college of law")
        || lower.contains("law faculty")
    {
        return true;
    }

    let has_role = [
        "professor",
        "lecturer",
        "fellow",
        "candidate",
        "researcher",
        "journalist",
        "editor",
        "doctoral student",
        "ph.d.",
        "phd",
        "j.d.",
        "jd ",
    ]
    .iter()
    .any(|needle| lower.contains(needle));
    let has_institution = [
        "university",
        "college",
        "school",
        "department",
        "faculty",
        "institute",
        "center",
        "centre",
    ]
    .iter()
    .any(|needle| lower.contains(needle));

    has_role && has_institution
}

pub(super) fn looks_like_standalone_author_line(text: &str, index: usize) -> bool {
    if index > 4 {
        return false;
    }
    let trimmed = text.trim();
    if trimmed.len() < 5
        || trimmed.len() > 120
        || trimmed.ends_with('.')
        || trimmed.contains(':')
        || looks_like_news_kicker_metadata(trimmed)
        || is_non_title_heading_text(trimmed)
        || starts_with_roman_heading(trimmed)
        || starts_with_lettered_heading(trimmed)
        || starts_with_numbered_heading(trimmed)
    {
        return false;
    }

    let without_note_marker = trimmed
        .trim_end_matches(|ch: char| matches!(ch, '*' | '†' | '‡') || ch.is_ascii_digit())
        .trim_end();
    if without_note_marker.is_empty() || without_note_marker.chars().any(|ch| ch.is_ascii_digit()) {
        return false;
    }

    let name_part = without_note_marker
        .split(['|', ','])
        .next()
        .unwrap_or(without_note_marker)
        .trim();
    let words = name_part.split_whitespace().collect::<Vec<_>>();
    if !(2..=7).contains(&words.len()) {
        return false;
    }
    if looks_like_short_all_caps_person_name(name_part) {
        return true;
    }

    let mut name_tokens = 0usize;
    for word in words {
        if word.eq_ignore_ascii_case("and") || word == "&" {
            continue;
        }
        let normalized =
            word.trim_matches(|ch: char| !ch.is_ascii_alphabetic() && ch != '-' && ch != '\'');
        if normalized.is_empty() || is_common_non_author_title_word(normalized) {
            return false;
        }
        if !looks_like_name_token(normalized) {
            return false;
        }
        name_tokens += 1;
    }

    name_tokens >= 2
}

fn looks_like_name_token(token: &str) -> bool {
    let mut chars = token.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_uppercase() {
        return false;
    }
    chars.all(|ch| ch.is_ascii_lowercase() || ch == '-' || ch == '\'' || ch == '.')
}

fn is_common_non_author_title_word(word: &str) -> bool {
    matches!(
        word.to_ascii_lowercase().as_str(),
        "a" | "an"
            | "the"
            | "how"
            | "why"
            | "what"
            | "when"
            | "where"
            | "who"
            | "court"
            | "courts"
            | "law"
            | "laws"
            | "review"
            | "journal"
            | "agency"
            | "agencies"
            | "power"
            | "privacy"
            | "artificial"
            | "intelligence"
            | "values"
            | "alignment"
            | "master"
            | "service"
            | "services"
            | "agreement"
            | "agreements"
            | "engagement"
            | "performance"
            | "contract"
            | "contracts"
            | "constitutional"
            | "administrative"
            | "judicial"
            | "federal"
            | "state"
            | "states"
            | "supreme"
            | "politics"
            | "business"
            | "technology"
            | "science"
            | "health"
            | "sports"
            | "analysis"
            | "opinion"
            | "background"
            | "introduction"
            | "conclusion"
            | "discussion"
            | "findings"
            | "implications"
            | "key"
            | "methodology"
            | "methods"
            | "overview"
            | "results"
            | "summary"
    )
}

pub(super) fn looks_like_abstract(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.starts_with("abstract ")
        || lower.starts_with("abstract.")
        || lower.starts_with("abstract:")
        || lower.starts_with("summary ")
        || lower.starts_with("summary:")
}

pub(super) fn starts_with_reader_aid_prefix(text: &str) -> bool {
    let lower = text.trim_start().to_ascii_lowercase();
    leading_reader_label(text).is_some()
        || lower.starts_with("the court held")
        || lower.starts_with("held that")
        || lower.starts_with("this article argues")
        || lower.starts_with("this essay argues")
        || lower.starts_with("this note argues")
        || lower.starts_with("we argue")
        || lower.starts_with("i argue")
}

pub(super) fn starts_article_transition(text: &str) -> bool {
    let lower = text.trim_start().to_ascii_lowercase();
    [
        "however,",
        "however ",
        "but ",
        "yet ",
        "still,",
        "at the same time",
        "the problem",
        "the case",
        "the decision",
        "the dispute",
        "the result",
        "as a result",
        "for years",
        "in recent years",
        "for critics",
        "supporters say",
        "opponents say",
        "experts say",
        "in the end",
    ]
    .iter()
    .any(|prefix| lower.starts_with(prefix))
}

fn looks_like_explainer(text: &str) -> bool {
    if leading_explainer_label(text).is_some() {
        return true;
    }

    let lower = text.to_ascii_lowercase();
    [
        "why it matters",
        "the big picture",
        "what to know",
        "context:",
        "background:",
        "in brief",
        "at issue",
        "key point",
        "here is why",
        "state of play",
        "what happened",
        "what's next",
        "whats next",
        "what they're saying",
        "what theyre saying",
        "by the numbers",
        "between the lines",
        "zoom in",
        "zoom out",
        "yes, but",
        "yes but",
        "reality check",
        "worth noting",
    ]
    .iter()
    .any(|needle| lower.starts_with(needle) || lower.contains(&format!(". {needle}")))
}

fn looks_like_answer(text: &str) -> bool {
    let trimmed = text.trim_start();
    let Some((prefix, body)) = trimmed.split_once(':') else {
        return false;
    };
    matches!(normalize_reader_label(prefix).as_str(), "a" | "answer")
        && word_count(body) >= 3
        && body.chars().any(char::is_alphabetic)
}

pub(super) fn looks_like_caption(text: &str, index: usize) -> bool {
    let trimmed = text.trim();
    if trimmed.len() < 6 || trimmed.len() > 420 || word_count(trimmed) > 70 {
        return false;
    }

    if starts_with_caption_label(trimmed) {
        return true;
    }

    let lower = trimmed.to_ascii_lowercase();
    if index > 8
        && (lower.starts_with("source:")
            || lower.starts_with("sources:")
            || lower.starts_with("credit:")
            || lower.starts_with("credits:"))
    {
        return true;
    }

    lower.starts_with("photo:")
        || lower.starts_with("image:")
        || lower.starts_with("graphic:")
        || lower.starts_with("graphics:")
        || lower.starts_with("photograph by ")
        || lower.starts_with("photo by ")
        || lower.starts_with("illustration by ")
        || lower.starts_with("graphic by ")
}

fn starts_with_caption_label(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    if lower.starts_with("table of ") {
        return false;
    }

    for label in [
        "figure", "fig.", "fig", "table", "chart", "map", "image", "photo",
    ] {
        let Some(rest) = lower.strip_prefix(label) else {
            continue;
        };
        if !rest
            .chars()
            .next()
            .is_some_and(|ch| ch.is_whitespace() || matches!(ch, '.' | ':' | '-'))
        {
            continue;
        }
        if caption_label_has_marker(rest) {
            return true;
        }
    }

    false
}

fn caption_label_has_marker(rest: &str) -> bool {
    let trimmed = rest.trim_start_matches(|ch: char| matches!(ch, '.' | ':' | '-' | ' '));
    let mut marker_chars = 0usize;
    for ch in trimmed.chars() {
        if ch.is_ascii_digit() || ch.is_ascii_uppercase() || matches!(ch, '.' | '-') {
            marker_chars += 1;
            continue;
        }
        break;
    }
    marker_chars > 0
}

fn looks_like_takeaway(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    [
        "this article argues",
        "we argue",
        "i argue",
        "the central claim",
        "the takeaway",
        "bottom line",
        "in sum",
        "in short",
        "this essay argues",
        "this note argues",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn looks_like_holding(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("the court held")
        || lower.contains("held that")
        || lower.starts_with("holding:")
        || lower.starts_with("holding ")
}

fn looks_like_issue(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.starts_with("issue:")
        || lower.starts_with("q:")
        || lower.starts_with("question:")
        || lower.starts_with("question presented")
        || lower.starts_with("whether ")
        || (text.trim_end().ends_with('?') && word_count(text) <= 28)
}

pub(super) fn looks_like_list_item(text: &str) -> bool {
    let trimmed = text.trim_start();
    trimmed.starts_with("- ")
        || trimmed.starts_with("• ")
        || starts_with_enumerator(trimmed, '(')
        || starts_with_numbered_prefix(trimmed)
        || starts_with_bare_numbered_list_prefix(trimmed)
}

pub(super) fn looks_like_clause(text: &str) -> bool {
    starts_with_numbered_prefix(text.trim_start())
}

fn starts_with_bare_numbered_list_prefix(text: &str) -> bool {
    split_bare_numbered_heading_marker(text).is_some_and(|(prefix, rest)| {
        is_simple_number_heading_marker(prefix) && !looks_like_heading_remainder(rest)
    })
}

fn starts_with_numbered_prefix(text: &str) -> bool {
    let mut seen_digit = false;
    for ch in text.chars().take(8) {
        if ch.is_ascii_digit() {
            seen_digit = true;
            continue;
        }
        if seen_digit && matches!(ch, '.' | ')') {
            return true;
        }
        if !matches!(ch, '.' | ' ') {
            return false;
        }
    }
    false
}

fn starts_with_enumerator(text: &str, open: char) -> bool {
    let mut chars = text.chars();
    chars.next() == Some(open)
        && chars.next().is_some_and(|ch| ch.is_ascii_alphanumeric())
        && chars.next() == Some(')')
}

fn contains_key_clause_language(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    if word_count(text) > 85 {
        return false;
    }

    let has_obligation = ["shall", "must", "may not", "is required to"]
        .iter()
        .any(|needle| lower.contains(needle));
    let has_contract_topic = [
        "deadline",
        "terminate",
        "termination",
        "confidential",
        "indemn",
        "payment",
        "invoice",
        "fee",
        "notice",
        "breach",
        "liable",
        "liability",
    ]
    .iter()
    .any(|needle| lower.contains(needle));
    let has_contract_actor = [
        "agreement",
        "contract",
        "party",
        "parties",
        "buyer",
        "seller",
        "tenant",
        "landlord",
        "borrower",
        "lender",
        "provider",
        "client",
        "customer",
        "supplier",
        "contractor",
        "employee",
        "employer",
        "licensee",
        "licensor",
    ]
    .iter()
    .any(|needle| lower.contains(needle));

    has_contract_actor && (has_obligation || has_contract_topic)
}

pub(super) fn label_for_block(role: LiquidBlockRole, text: &str) -> Option<String> {
    match role {
        LiquidBlockRole::Definition => text
            .split_once(" means ")
            .or_else(|| text.split_once(" shall mean "))
            .map(|(term, _)| term.trim_matches(['"', '“', '”', ' ', '.']).to_owned())
            .filter(|term| !term.is_empty() && term.len() <= 80)
            .or_else(|| Some("Definition".to_owned())),
        LiquidBlockRole::Marginalia => split_marginalia_field(text)
            .map(|(label, _)| label.to_owned())
            .filter(|label| !label.is_empty()),
        LiquidBlockRole::KeyClause => key_clause_label(text).map(str::to_owned),
        LiquidBlockRole::Explainer => {
            leading_reader_label(text).or_else(|| Some("Explainer".to_owned()))
        }
        LiquidBlockRole::Takeaway => {
            leading_reader_label(text).or_else(|| Some("Takeaway".to_owned()))
        }
        LiquidBlockRole::Holding => {
            leading_reader_label(text).or_else(|| Some("Holding".to_owned()))
        }
        LiquidBlockRole::Issue => leading_reader_label(text).or_else(|| Some("Issue".to_owned())),
        LiquidBlockRole::Caption => caption_label(text),
        LiquidBlockRole::Table => Some("Table".to_owned()),
        LiquidBlockRole::Syllabus => Some("Syllabus".to_owned()),
        _ => None,
    }
}

fn caption_label(text: &str) -> Option<String> {
    let lower = text.trim_start().to_ascii_lowercase();
    if lower.starts_with("table ") {
        Some("Table".to_owned())
    } else if lower.starts_with("figure ") || lower.starts_with("fig.") || lower.starts_with("fig ")
    {
        Some("Figure".to_owned())
    } else if lower.starts_with("chart ") {
        Some("Chart".to_owned())
    } else if lower.starts_with("map ") {
        Some("Map".to_owned())
    } else if lower.starts_with("photo")
        || lower.starts_with("image")
        || lower.starts_with("photograph by ")
    {
        Some("Photo".to_owned())
    } else if lower.starts_with("source:")
        || lower.starts_with("sources:")
        || lower.starts_with("credit:")
        || lower.starts_with("credits:")
    {
        Some("Source".to_owned())
    } else {
        Some("Caption".to_owned())
    }
}

fn leading_reader_label(text: &str) -> Option<String> {
    let (prefix, _) = text.trim_start().split_once(':')?;
    if prefix.chars().count() > 48 || prefix.split_whitespace().count() > 6 {
        return None;
    }

    let label = match normalize_reader_label(prefix).as_str() {
        "why it matters" => "Why it matters",
        "the big picture" => "The big picture",
        "what to know" => "What to know",
        "factbox" | "fact box" => "Factbox",
        "key fact" | "key facts" | "fast facts" => "Key facts",
        "the latest" => "The latest",
        "at stake" | "whats at stake" => "At stake",
        "what we know" => "What we know",
        "what we dont know" => "What we don't know",
        "state of play" => "State of play",
        "what happened" => "What happened",
        "timeline" => "Timeline",
        "key dates" | "important dates" | "chronology" => "Key dates",
        "how we got here" => "How we got here",
        "whats next" => "What's next",
        "what theyre saying" => "What they're saying",
        "by the numbers" => "By the numbers",
        "between the lines" => "Between the lines",
        "zoom in" => "Zoom in",
        "zoom out" => "Zoom out",
        "yes but" => "Yes, but",
        "reality check" => "Reality check",
        "worth noting" => "Worth noting",
        "context" => "Context",
        "background" => "Background",
        "in brief" => "In brief",
        "at issue" => "At issue",
        "key point" => "Key point",
        "here is why" => "Here is why",
        "bottom line" => "Bottom line",
        "the takeaway" | "takeaway" => "Takeaway",
        "holding" => "Holding",
        "issue" => "Issue",
        "q" | "question" => "Question",
        "a" | "answer" => "Answer",
        "question presented" => "Question presented",
        _ => return None,
    };
    Some(label.to_owned())
}

fn leading_explainer_label(text: &str) -> Option<String> {
    let label = leading_reader_label(text)?;
    matches!(
        label.as_str(),
        "Why it matters"
            | "The big picture"
            | "What to know"
            | "Factbox"
            | "Key facts"
            | "The latest"
            | "At stake"
            | "What we know"
            | "What we don't know"
            | "State of play"
            | "What happened"
            | "Timeline"
            | "Key dates"
            | "How we got here"
            | "What's next"
            | "What they're saying"
            | "By the numbers"
            | "Between the lines"
            | "Zoom in"
            | "Zoom out"
            | "Yes, but"
            | "Reality check"
            | "Worth noting"
            | "Context"
            | "Background"
            | "In brief"
            | "At issue"
            | "Key point"
            | "Here is why"
    )
    .then_some(label)
}

fn normalize_reader_label(value: &str) -> String {
    value
        .trim()
        .trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != ' ')
        .chars()
        .filter_map(|ch| {
            if ch.is_ascii_alphanumeric() {
                Some(ch.to_ascii_lowercase())
            } else if ch.is_whitespace() || matches!(ch, '-' | '_' | '/') {
                Some(' ')
            } else if matches!(ch, '\'' | '’' | '‘') {
                None
            } else {
                None
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn key_clause_label(text: &str) -> Option<&'static str> {
    let lower = text.to_ascii_lowercase();
    if lower.contains("termination") || lower.contains("terminate") {
        Some("Termination")
    } else if lower.contains("payment") || lower.contains("fee") || lower.contains("invoice") {
        Some("Payment")
    } else if lower.contains("confidential") {
        Some("Confidentiality")
    } else if lower.contains("notice") {
        Some("Notice")
    } else if lower.contains("indemn") || lower.contains("liability") || lower.contains("liable") {
        Some("Risk")
    } else if lower.contains("shall") || lower.contains("must") {
        Some("Obligation")
    } else {
        None
    }
}

pub(super) fn looks_like_syllabus(text: &str) -> bool {
    let trimmed = text.trim_start();
    let lower = trimmed.to_ascii_lowercase();
    lower.starts_with("syllabus")
        || lower.starts_with("syllabus:")
        || trimmed.starts_with("Question Presented")
        || lower.starts_with("held:")
}

pub(super) fn looks_like_table(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.len() < 30 || trimmed.len() > 900 {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    if starts_with_caption_label(trimmed)
        || lower.starts_with("table of ")
        || looks_like_marginalia(trimmed)
        || looks_like_front_matter_metadata(trimmed)
    {
        return false;
    }
    let multi_ws_count = trimmed.matches("  ").count() + trimmed.matches('\t').count();
    if multi_ws_count < 2 {
        return false;
    }
    let digits = trimmed.chars().filter(|c| c.is_ascii_digit()).count();
    let digit_density = digits as f32 / trimmed.len() as f32;
    let has_struct = trimmed.contains('|')
        || trimmed.contains(" $")
        || trimmed.contains("$ ")
        || lower.contains("total")
        || lower.contains("amount");
    (digit_density >= 0.05 || has_struct) && multi_ws_count >= 2 && word_count(trimmed) >= 3
}

pub(super) fn looks_like_dissent_or_concurrence_heading(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.len() > 120 || word_count(trimmed) > 15 {
        return false;
    }
    let upper = trimmed.to_ascii_uppercase();
    let lower = trimmed.to_ascii_lowercase();
    if upper.starts_with("JUSTICE ")
        && (lower.contains("dissent") || lower.contains("concurr") || lower.contains("opinion"))
    {
        return true;
    }
    if lower.starts_with("dissenting")
        || lower.starts_with("concurring")
        || lower.starts_with("dissent ")
        || lower.starts_with("concurrence")
        || lower.contains("dissenting opinion")
        || lower.contains("concurring opinion")
    {
        return true;
    }
    false
}

pub(super) fn style_type_to_role(style_type: &str) -> LiquidBlockRole {
    match style_type.trim().to_ascii_lowercase().as_str() {
        "heading1" | "heading2" | "heading3" => LiquidBlockRole::Heading,
        "heading4" | "heading5" | "heading6" | "heading7" | "heading8" | "heading9" => {
            LiquidBlockRole::Subheading
        }
        "abstract" => LiquidBlockRole::Abstract,
        "syllabus" => LiquidBlockRole::Syllabus,
        "author_info" | "byline" => LiquidBlockRole::AuthorInfo,
        "lead" | "lede" | "standfirst" => LiquidBlockRole::Lead,
        "explainer" | "callout" | "context" | "background" => LiquidBlockRole::Explainer,
        "takeaway" | "key_takeaway" | "bottom_line" => LiquidBlockRole::Takeaway,
        "holding" => LiquidBlockRole::Holding,
        "issue" | "question" => LiquidBlockRole::Issue,
        "definition" => LiquidBlockRole::Definition,
        "marginalia" | "field" | "field_row" | "key_value" => LiquidBlockRole::Marginalia,
        "key_clause" => LiquidBlockRole::KeyClause,
        "quote_para" => LiquidBlockRole::Quote,
        "caption" | "figure_caption" | "table_caption" | "photo_caption" | "credit" => {
            LiquidBlockRole::Caption
        }
        "table" | "table_data" => LiquidBlockRole::Table,
        "contents" | "toc" | "table_of_contents" => LiquidBlockRole::Contents,
        "header" => LiquidBlockRole::Header,
        "footer" => LiquidBlockRole::Footer,
        "footnote" => LiquidBlockRole::Footnote,
        "metadata" => LiquidBlockRole::Metadata,
        "noise" | "discard" | "junk" => LiquidBlockRole::Noise,
        "paragraph" => LiquidBlockRole::Paragraph,
        _ => LiquidBlockRole::Paragraph,
    }
}
