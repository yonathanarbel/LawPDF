//! Small, pure, reusable string and text utilities.
//!
//! These functions have minimal dependencies and are excellent candidates
//! for unit testing. Extracted as part of Phase 1.

use std::time::{SystemTime, UNIX_EPOCH};

/// Count words using whitespace splitting (matches original behavior).
pub fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

/// Ratio of uppercase alphabetic characters to total alphabetic characters.
pub fn uppercase_ratio(text: &str) -> f32 {
    let letters = text.chars().filter(|ch| ch.is_alphabetic()).count();
    if letters == 0 {
        return 0.0;
    }
    let uppercase = text.chars().filter(|ch| ch.is_uppercase()).count();
    uppercase as f32 / letters as f32
}

/// Ratio of words that start with an uppercase letter.
pub fn title_case_ratio(text: &str) -> f32 {
    let mut words = 0usize;
    let mut title_words = 0usize;
    for word in text.split_whitespace() {
        let mut chars = word.trim_matches(|ch: char| !ch.is_alphabetic()).chars();
        let Some(first) = chars.next() else {
            continue;
        };
        words += 1;
        if first.is_uppercase() {
            title_words += 1;
        }
    }
    if words == 0 {
        0.0
    } else {
        title_words as f32 / words as f32
    }
}

/// Detects common Roman numeral + dot style headings (I., II., etc.).
pub fn starts_with_roman_heading(text: &str) -> bool {
    if let Some((prefix, rest)) = text.split_once('.') {
        return !rest.trim().is_empty() && is_roman_heading_marker(prefix);
    }

    split_bare_outline_marker(text).is_some_and(|(prefix, rest)| {
        is_roman_heading_marker(prefix) && looks_like_heading_remainder(rest)
    })
}

fn looks_like_heading_remainder(rest: &str) -> bool {
    let rest = rest.trim();
    if rest.is_empty() || rest.ends_with('.') || word_count(rest) > 12 {
        return false;
    }
    title_case_ratio(rest) > 0.58 && rest.chars().any(char::is_alphabetic)
}

pub fn split_bare_outline_marker(text: &str) -> Option<(&str, &str)> {
    let trimmed = text.trim_start();
    let marker_end = trimmed.find(|ch: char| ch.is_whitespace())?;
    let prefix = &trimmed[..marker_end];
    let rest = trimmed[marker_end..].trim_start();
    (!prefix.is_empty() && !rest.is_empty()).then_some((prefix, rest))
}

pub fn is_roman_heading_marker(prefix: &str) -> bool {
    !prefix.is_empty()
        && prefix.len() <= 8
        && prefix
            .chars()
            .all(|ch| matches!(ch.to_ascii_uppercase(), 'I' | 'V' | 'X'))
}

pub fn is_letter_heading_marker(prefix: &str) -> bool {
    prefix.len() == 1 && prefix.chars().all(|ch| ch.is_ascii_uppercase())
}

pub fn is_superscript_digit(ch: char) -> bool {
    matches!(
        ch,
        '⁰' | '¹' | '²' | '³' | '⁴' | '⁵' | '⁶' | '⁷' | '⁸' | '⁹'
    )
}

/// Truncates a string for logging/preview purposes.
pub fn preview(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_owned();
    }
    let mut truncated = value.chars().take(max_chars).collect::<String>();
    truncated.push_str("...");
    truncated
}

/// Extracts the first top-level JSON object from a string (handles ```json fences).
pub fn extract_json_object(content: &str) -> String {
    let trimmed = content.trim();
    let without_fence = if trimmed.starts_with("```") {
        trimmed
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim()
            .trim_end_matches("```")
            .trim()
    } else {
        trimmed
    };

    let Some(start) = without_fence.find('{') else {
        return without_fence.to_owned();
    };
    let Some(end) = without_fence.rfind('}') else {
        return without_fence.to_owned();
    };
    without_fence[start..=end].to_owned()
}

/// Creates a compact version of text for LLM prompts (head + tail).
pub fn compact_for_prompt(text: &str, max_chars: usize) -> String {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= max_chars {
        return normalized;
    }
    let head_chars = max_chars.saturating_mul(2) / 3;
    let tail_chars = max_chars.saturating_sub(head_chars).saturating_sub(5);
    let head = normalized.chars().take(head_chars).collect::<String>();
    let tail = normalized
        .chars()
        .rev()
        .take(tail_chars)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    format!("{head} ... {tail}")
}

/// Current unix timestamp in seconds.
pub fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

/// Simple stable 64-bit hash (FNV-1a variant) used for cache keys.
pub fn stable_hash(value: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

pub(crate) fn should_preserve_terminal_hyphen(left: &str, next_line: &str) -> bool {
    let Some(prefix) = terminal_hyphen_prefix(left) else {
        return false;
    };
    let Some(next_word) = first_word(next_line) else {
        return false;
    };

    if !next_word
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_lowercase())
    {
        return false;
    }

    let prefix = prefix.to_ascii_lowercase();
    let next_word = next_word.to_ascii_lowercase();
    let preserve_prefix = matches!(
        prefix.as_str(),
        "well"
            | "case"
            | "fact"
            | "time"
            | "rule"
            | "policy"
            | "decision"
            | "record"
            | "court"
            | "state"
            | "party"
            | "agency"
            | "rights"
            | "due"
            | "long"
            | "short"
            | "cross"
            | "self"
    );
    let preserve_next = matches!(
        next_word.as_str(),
        "based"
            | "bound"
            | "driven"
            | "like"
            | "making"
            | "process"
            | "related"
            | "settled"
            | "specific"
            | "term"
            | "wide"
    );

    preserve_prefix || preserve_next
}

fn terminal_hyphen_prefix(text: &str) -> Option<&str> {
    let trimmed = text.trim_end();
    let without_hyphen = trimmed.strip_suffix('-')?;
    let start = without_hyphen
        .char_indices()
        .rev()
        .find_map(|(index, ch)| (!ch.is_ascii_alphabetic()).then_some(index + ch.len_utf8()))
        .unwrap_or(0);
    let prefix = without_hyphen[start..].trim();
    (!prefix.is_empty()).then_some(prefix)
}

fn first_word(text: &str) -> Option<&str> {
    let trimmed = text.trim_start();
    let end = trimmed
        .char_indices()
        .find_map(|(index, ch)| (!ch.is_ascii_alphabetic()).then_some(index))
        .unwrap_or(trimmed.len());
    (end > 0).then_some(&trimmed[..end])
}
