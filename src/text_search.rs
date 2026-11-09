use std::ops::Range;

/// Case-insensitive matches expressed as byte ranges in the original text.
pub fn match_ranges(text: &str, query: &str) -> Vec<Range<usize>> {
    if query.is_empty() {
        return Vec::new();
    }
    if text.is_ascii() && query.is_ascii() {
        return text
            .to_ascii_lowercase()
            .match_indices(&query.to_ascii_lowercase())
            .map(|(start, value)| start..start + value.len())
            .collect();
    }
    let needle = query
        .chars()
        .flat_map(char::to_lowercase)
        .collect::<String>();
    let mut folded = String::with_capacity(text.len());
    let mut boundaries = vec![(0, 0)];
    for (start, ch) in text.char_indices() {
        folded.extend(ch.to_lowercase());
        boundaries.push((folded.len(), start + ch.len_utf8()));
    }
    folded
        .match_indices(&needle)
        .map(|(start, value)| {
            let first = boundaries.partition_point(|(offset, _)| *offset <= start) - 1;
            let last = boundaries.partition_point(|(offset, _)| *offset < start + value.len());
            boundaries[first].1..boundaries[last].1
        })
        .collect()
}

/// Bound context by characters without rescanning or copying the whole page per hit.
pub fn snippet_with_match(
    text: &str,
    range: Range<usize>,
    context_chars: usize,
) -> (String, Range<usize>) {
    let left = text[..range.start]
        .char_indices()
        .rev()
        .take(context_chars)
        .last()
        .map_or(range.start, |(offset, _)| offset);
    let right = text[range.end..]
        .char_indices()
        .nth(context_chars)
        .map_or(text.len(), |(offset, _)| range.end + offset);
    let mut result = String::new();
    if left > 0 {
        result.push('…');
    }
    let before = &text[left..range.start];
    let after = &text[range.end..right];
    let words = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let prefix = words(before);
    result.push_str(&prefix);
    if !prefix.is_empty() && before.ends_with(char::is_whitespace) {
        result.push(' ');
    }
    let start = result.len();
    result.push_str(&words(&text[range]));
    let matched = start..result.len();
    let suffix = words(after);
    if !suffix.is_empty() && after.starts_with(char::is_whitespace) {
        result.push(' ');
    }
    result.push_str(&suffix);
    if right < text.len() {
        result.push('…');
    }
    (result, matched)
}

#[cfg(test)]
fn snippet(text: &str, range: Range<usize>, context_chars: usize) -> String {
    snippet_with_match(text, range, context_chars).0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snippet_emphasis_identifies_the_actual_repeat_after_whitespace_folding() {
        let text = "漢法   LAW\nlaw\tÅ";
        let range = match_ranges(text, "law").pop().unwrap();
        let (snippet, found) = snippet_with_match(text, range, 40);
        assert_eq!(snippet, "漢法 LAW law Å");
        assert_eq!(&snippet[found], "law");
    }

    #[test]
    fn unicode_matches_keep_original_offsets_and_case() {
        let text = "İ ÉCOLE éCoLe 漢字";
        let ranges = match_ranges(text, "école");
        assert_eq!(
            ranges.iter().map(|r| &text[r.clone()]).collect::<Vec<_>>(),
            ["ÉCOLE", "éCoLe"]
        );
        assert_eq!(&text[match_ranges(text, "i")[0].clone()], "İ");
        assert_eq!(&text[match_ranges(text, "漢字")[0].clone()], "漢字");
        assert!(match_ranges(text, "").is_empty());
    }

    #[test]
    fn ascii_search_preserves_nonoverlapping_match_order() {
        assert_eq!(match_ranges("LAW law lawful", "LaW"), [0..3, 4..7, 8..11]);
        assert_eq!(match_ranges("aaaa", "aa"), [0..2, 2..4]);
    }

    #[test]
    fn snippets_stay_bounded_at_multibyte_boundaries() {
        let text = format!("{}ÉCOLE{}", "界".repeat(10_000), "é".repeat(10_000));
        let found = match_ranges(&text, "école").remove(0);
        let result = snippet(&text, found, 24);
        assert_eq!(
            result,
            format!("…{}ÉCOLE{}…", "界".repeat(24), "é".repeat(24))
        );
        assert_eq!(snippet("a law b", 2..5, 42), "a law b");
    }
}
