//! Text cleaning, running header detection, noise removal, and cross-page paragraph joining.
//!
//! One of the highest-value and most self-contained pieces of the pipeline.

use std::collections::{HashMap, HashSet};

// =============================================================================
// Running header & page noise detection
// =============================================================================

pub fn detect_running_headers(pages: &[String]) -> HashMap<String, ()> {
    if pages.len() < 2 {
        return HashMap::new();
    }
    let threshold = ((pages.len() as f32 * 0.20).ceil() as usize).max(2);
    let mut freq: HashMap<String, usize> = HashMap::new();
    for page in pages {
        let mut seen = HashSet::new();
        for candidate in page_edge_candidates(page) {
            if seen.insert(candidate.clone()) {
                *freq.entry(candidate).or_default() += 1;
            }
        }
    }
    freq.into_iter()
        .filter(|(_, count)| *count >= threshold)
        .map(|(key, _)| (key, ()))
        .collect()
}

fn page_edge_candidates(page: &str) -> Vec<String> {
    let lines = page
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    let mut candidates = Vec::new();
    for line in lines.iter().take(4).chain(lines.iter().rev().take(4)) {
        if line.len() >= 120 || is_lone_page_number(line) {
            continue;
        }
        let normalized = normalize_noise_key(line);
        if !normalized.is_empty() {
            candidates.push(normalized);
        }
        if let Some(variable_key) = normalize_variable_page_edge_key(line) {
            candidates.push(variable_key);
        }
    }
    candidates
}

fn is_running_header(line: &str, headers: &HashMap<String, ()>) -> bool {
    headers.contains_key(&normalize_noise_key(line))
        || normalize_variable_page_edge_key(line).is_some_and(|key| headers.contains_key(&key))
}

pub fn normalize_noise_key(line: &str) -> String {
    line.trim()
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_variable_page_edge_key(line: &str) -> Option<String> {
    let normalized = normalize_noise_key(line);
    if normalized.len() < 12
        || !normalized.chars().any(|ch| ch.is_ascii_digit())
        || !normalized.chars().any(|ch| ch.is_ascii_alphabetic())
    {
        return None;
    }

    let mut key = String::with_capacity(normalized.len());
    let mut in_digits = false;
    for ch in normalized.chars() {
        if ch.is_ascii_digit() {
            if !in_digits {
                key.push('#');
                in_digits = true;
            }
        } else {
            key.push(ch);
            in_digits = false;
        }
    }
    let alpha_count = key.chars().filter(|ch| ch.is_ascii_alphabetic()).count();
    (alpha_count >= 6 && key != normalized).then_some(key)
}

fn is_lone_page_number(line: &str) -> bool {
    let t = line.trim();
    if t.chars().all(|c| c.is_ascii_digit()) && !t.is_empty() {
        return true;
    }
    let lower = t.to_ascii_lowercase();
    if lower.starts_with("page ") && lower.split_whitespace().count() <= 4 {
        return true;
    }
    if t.starts_with("- ") && t.ends_with(" -") && t.len() <= 10 {
        return true;
    }
    false
}

// =============================================================================
// Main cleaning pipeline
// =============================================================================

pub fn clean_source_text(pages: &[String]) -> (String, usize) {
    let headers = detect_running_headers(pages);
    let mut output = String::new();
    let mut removed = 0usize;

    for page in pages {
        let (cleaned, page_removed) = clean_page_text(page, &headers);
        removed += page_removed;
        let trimmed = cleaned.trim();
        if trimmed.is_empty() {
            continue;
        }
        if output.is_empty() {
            output.push_str(trimmed);
        } else {
            let last_line = output
                .lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("")
                .trim_end();
            if last_line.ends_with('-') {
                let new_len = output.trim_end_matches(|c: char| c.is_whitespace()).len();
                output.truncate(new_len);
                if !crate::liquid::util::should_preserve_terminal_hyphen(&output, trimmed) {
                    output.pop();
                }
                output.push_str(trimmed);
            } else if last_line.ends_with(['.', '?', '!', ':', '"', '\u{201d}']) {
                output.push_str("\n\n");
                output.push_str(trimmed);
            } else {
                output.push('\n');
                output.push_str(trimmed);
            }
        }
    }

    (output, removed)
}

fn clean_page_text(page: &str, headers: &HashMap<String, ()>) -> (String, usize) {
    let mut lines = Vec::new();
    let mut removed = 0usize;

    for raw_line in page.lines() {
        let normalized_line = normalize_extracted_line(raw_line);
        let line = normalized_line.trim();
        if line.is_empty() {
            if !lines.is_empty() && !lines.last().is_some_and(|line: &String| line.is_empty()) {
                lines.push(String::new());
            }
            continue;
        }

        if is_running_header(line, headers) || is_lone_page_number(line) || is_noise_line(line) {
            removed += 1;
            continue;
        }
        lines.push(line.to_owned());
    }

    while lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }

    (lines.join("\n"), removed)
}

fn normalize_extracted_line(line: &str) -> String {
    repair_common_pdf_mojibake(line)
        .chars()
        .filter(|ch| !ch.is_control() || ch.is_whitespace())
        .collect()
}

fn repair_common_pdf_mojibake(line: &str) -> String {
    let replacements = [
        ("\u{2217}", "*"),
        ("\u{2212}", "-"),
        ("âˆ—", "*"),
        ("âˆ’", "-"),
        ("â€“", "\u{2013}"),
        ("â€”", "\u{2014}"),
        ("â€˜", "\u{2018}"),
        ("â€™", "\u{2019}"),
        ("â€œ", "\u{201c}"),
        ("â€", "\u{201d}"),
        ("â€¢", "\u{2022}"),
        ("Â©", "\u{00a9}"),
        ("Â§", "\u{00a7}"),
        ("Â¶", "\u{00b6}"),
        ("Â®", "\u{00ae}"),
        ("Â ", " "),
    ];

    let mut repaired = line.to_owned();
    for (from, to) in replacements {
        if repaired.contains(from) {
            repaired = repaired.replace(from, to);
        }
    }
    repaired
}

// =============================================================================
// The giant noise list + helpers (the heart of web/SSRN/HeinOnline cleaning)
// =============================================================================

// Table-driven noise list for easy maintenance and testing.
// Exact matches (after normalize_noise_key).
static NOISE_EXACT: &[&str] = &[
    "advertisement",
    "advertisements",
    "subscribe",
    "subscribe now",
    "become a subscriber",
    "sign in",
    "log in",
    "register",
    "register for free",
    "share",
    "save",
    "print",
    "email",
    "facebook",
    "x",
    "linkedin",
    "whatsapp",
    "reddit",
    "threads",
    "bluesky",
    "mastodon",
    "pocket",
    "telegram",
    "flipboard",
    "copy link",
    "copy link copied",
    "copy article link",
    "gift article",
    "open in app",
    "read in app",
    "open the app",
    "listen",
    "listen to article",
    "listen to this article",
    "listen to story",
    "share full article",
    "share this article",
    "save article",
    "save this article",
    "read more",
    "read more:",
    "read next",
    "related",
    "related articles",
    "related stories",
    "more on this story",
    "most popular",
    "most read",
    "top stories",
    "latest news",
    "recommended",
    "recommended articles",
    "recommended stories",
    "comments",
    "view comments",
    "show comments",
    "leave a comment",
    "join the conversation",
    "newsletter",
    "notifications",
    "enable notifications",
    "allow notifications",
    "turn on notifications",
    "more from this author",
    "follow us",
    "privacy policy",
    "terms of service",
    "terms of use",
    "cookie policy",
    "cookie settings",
    "accept all cookies",
    "accept cookies",
    "accept and continue",
    "accept & continue",
    "reject all",
    "reject all cookies",
    "manage cookies",
    "manage consent",
    "manage consent preferences",
    "manage privacy preferences",
    "privacy settings",
    "your privacy choices",
    "customize choices",
    "save choices",
    "save preferences",
    "do not sell or share my personal information",
    "skip to main content",
    "continue reading",
    "continue reading the main story",
    "advertisement - scroll to continue",
    "advertisement -- scroll to continue",
    "advertisement scroll to continue",
    "skip advertisement",
    "skip ad",
    "sponsored content",
    "paid post",
    "all rights reserved",
    "recommended citation",
    "repository citation",
];

// Prefixes that indicate noise when they start the normalized line.
static NOISE_PREFIXES: &[&str] = &[
    "downloaded from ",
    "electronic copy available at",
    "© ",
    "(c) ",
    "copyright ",
    "available at: http",
    "already a subscriber",
    "create a free account",
    "create your free account",
    "create an account",
    "create your account",
    "subscribe to",
    "subscribe now",
    "subscribe today",
    "subscribe for",
    "become a subscriber",
    "sign up for",
    "sign up to",
    "sign up here",
    "sign in to unlock",
    "sign in for",
    "sign in to continue",
    "log in to continue",
    "log in or create",
    "register for free",
    "thanks for reading",
    "unlock more",
    "support our journalism",
    "support independent journalism",
    "support quality journalism",
    "continue with google",
    "continue with apple",
    "continue with facebook",
    "open in app",
    "read in app",
    "listen to article",
    "listen to this article",
    "listen to story",
    "share on ",
    "share via ",
    "share to ",
    "share full article",
    "share this article",
    "save article",
    "save this article",
    "copy link",
    "send any friend a story",
    "send this article to",
    "read next:",
    "related:",
    "related article",
    "related story",
    "more from ",
    "more about ",
    "more on ",
    "recommended for you",
    "recommended from ",
    "most popular",
    "most read",
    "top stories",
    "latest news",
    "view comments",
    "read comments",
    "join the conversation",
    "advertisement - scroll to continue",
    "advertisement: scroll to continue",
    "advertisement scroll to continue",
    "skip advertisement",
    "skip ad",
    "allow notifications",
    "enable notifications",
    "turn on notifications",
    "we use cookies to",
    "this site uses cookies",
    "by clicking accept",
    "by selecting accept",
    "do not sell or share",
    "this story has been shared",
    "this article is brought to you",
    "this article is available at",
    "a version of this article appears in print",
    "a version of this article appeared in print",
    "this article appears in print",
    "this article appeared in print",
    "this article was originally published",
    "this story was originally published",
    "originally published",
    "read the original article",
    "it has been accepted for inclusion",
    "follow this and additional works at",
    "for more information, please contact",
];

// Additional table-driven sets for contains checks and URL/copyright patterns.
// These + EXACT + PREFIXES make is_noise_line fully data-driven for easy testing/extension.
static NOISE_CONTAINS: &[&str] = &["digitalcommons", "bepress", "heinonline", "ssrn.com"];
static NOISE_URL_PREFIXES: &[&str] = &["http://", "https://"];

fn is_noise_line(line: &str) -> bool {
    let lower = normalize_noise_key(line);

    if NOISE_EXACT.contains(&lower.as_str()) {
        return true;
    }

    if NOISE_PREFIXES.iter().any(|p| lower.starts_with(p)) {
        return true;
    }

    if NOISE_CONTAINS.iter().any(|s| lower.contains(s)) {
        return true;
    }

    if NOISE_URL_PREFIXES.iter().any(|p| lower.starts_with(p)) {
        return true;
    }

    if lower.starts_with('\u{00a9}') {
        return true;
    }

    if lower.starts_with("part of the ") && lower.ends_with(" commons") {
        return true;
    }

    looks_like_comment_count_noise(&lower)
}

fn looks_like_comment_count_noise(lower: &str) -> bool {
    let mut words = lower.split_whitespace();
    let Some(first) = words.next() else {
        return false;
    };
    let first = first.trim_matches(|ch: char| matches!(ch, '(' | ')' | '[' | ']' | ','));
    if first.parse::<u32>().is_err() {
        return false;
    }
    matches!(
        words.next(),
        Some("comment") | Some("comments") | Some("responses") | Some("reply") | Some("replies")
    )
}

// =============================================================================
// Footnote detection (used by both cleaning and classification)
// =============================================================================

pub fn looks_like_footnote_line(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.len() < 18 {
        return false;
    }
    let mut chars = trimmed.chars().peekable();
    let Some(first) = chars.peek().copied() else {
        return false;
    };
    if crate::liquid::util::is_superscript_digit(first) {
        return true;
    }
    if first == '*' {
        let body = trimmed[first.len_utf8()..].trim_start();
        return looks_like_symbol_footnote_body(body);
    }
    if !first.is_ascii_digit() {
        return false;
    }
    let mut digits = 0usize;
    while chars.peek().is_some_and(|ch| ch.is_ascii_digit()) {
        digits += 1;
        chars.next();
    }
    if digits == 0 || digits > 3 {
        return false;
    }
    if !chars
        .peek()
        .is_some_and(|ch| ch.is_whitespace() || matches!(ch, '.' | ')' | ']'))
    {
        return false;
    }
    let (_, body) = split_note_marker(trimmed);
    !looks_like_numeric_lowercase_body_fragment(body)
}

fn looks_like_numeric_lowercase_body_fragment(body: &str) -> bool {
    if body_starts_with_legal_note_cue(body) {
        return false;
    }
    body.chars()
        .find(|ch| ch.is_alphabetic())
        .is_some_and(char::is_lowercase)
}

fn body_starts_with_legal_note_cue(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    lower.starts_with("see")
        || lower.starts_with("cf.")
        || lower.starts_with("cf ")
        || lower.starts_with("id.")
        || lower.starts_with("id ")
        || lower.starts_with("accord")
        || lower.starts_with("but see")
        || lower.starts_with("supra")
        || lower.starts_with("infra")
}

fn looks_like_symbol_footnote_body(body: &str) -> bool {
    if body.len() < 18 {
        return false;
    }
    let lower = body.to_ascii_lowercase();
    lower.contains(" l. rev")
        || lower.contains(" law review")
        || lower.contains(" legal writing")
        || lower.contains(" v. ")
        || lower.contains("u.s.")
        || lower.contains("supra")
        || lower.contains("infra")
        || lower.contains("university press")
        || lower.contains("statute l. rev")
        || lower.contains("legal usage")
}

pub fn looks_like_citation_footnote_line(line: &str) -> bool {
    if !looks_like_footnote_line(line) {
        return false;
    }
    let (_, body) = split_note_marker(line);
    let lower = body.to_ascii_lowercase();
    lower.starts_with("see ")
        || lower.starts_with("see, ")
        || lower.starts_with("see e.g.")
        || lower.starts_with("see, e.g.")
        || lower.starts_with("cf. ")
        || lower.starts_with("accord ")
        || lower.starts_with("but see ")
        || lower.contains(" v. ")
        || lower.contains("u.s.")
        || lower.contains("f.3d")
        || lower.contains("f.2d")
        || lower.contains("restatement")
        || lower.contains("law review")
        || lower.contains("l. rev.")
        || lower.contains("supra")
        || lower.contains("infra")
        || lower.contains(" id.")
}

pub fn split_note_marker(text: &str) -> (Option<&str>, &str) {
    let trimmed = text.trim_start();
    let mut marker_end = 0usize;
    let mut digits = 0usize;

    for (index, ch) in trimmed.char_indices() {
        if ch.is_ascii_digit() && digits < 3 {
            digits += 1;
            marker_end = index + ch.len_utf8();
            continue;
        }
        break;
    }

    if digits == 0 {
        return (None, trimmed);
    }

    let rest = &trimmed[marker_end..];
    let body = rest
        .trim_start_matches(|ch: char| matches!(ch, '.' | ')' | ']' | ' ' | '\t'))
        .trim_start();
    (Some(&trimmed[..marker_end]), body)
}
