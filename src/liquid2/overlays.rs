//! Line-level overlays applied to decoded lines before block assembly.

use super::*;

pub(super) fn apply_pp_footnote_region_membership(
    lines: &[DeepLiquidSourceLine],
    path: &mut [Lm2Action],
) {
    let mut members = lines
        .iter()
        .map(pp_footnote_region_member)
        .collect::<Vec<_>>();

    for cluster_start in 0..lines.len() {
        if !members[cluster_start] || cluster_start > 0 && members[cluster_start - 1] {
            continue;
        }
        let window_start = cluster_start.saturating_sub(5);
        for anchor in (window_start..cluster_start).rev() {
            if !looks_like_marginalia_note_block_start(&lines[anchor].text) {
                continue;
            }
            if !(anchor..cluster_start).all(|span_index| {
                pp_footnote_span_closure_eligible(&lines[span_index])
                    && (span_index == anchor
                        || pp_footnote_region_member(&lines[span_index])
                        || pp_footnote_span_continuation_like(&lines[span_index]))
            }) {
                continue;
            }
            for member in &mut members[anchor..cluster_start] {
                *member = true;
            }
            break;
        }
    }

    let mut index = 0usize;
    while index < lines.len() {
        if members[index] {
            index += 1;
            continue;
        }
        let gap_start = index;
        while index < lines.len() && !members[index] {
            index += 1;
        }
        let gap_end = index;
        if gap_start > 0
            && gap_end < lines.len()
            && gap_end - gap_start <= 1
            && (gap_start..gap_end)
                .all(|gap_index| pp_footnote_span_closure_eligible(&lines[gap_index]))
        {
            for member in &mut members[gap_start..gap_end] {
                *member = true;
            }
        }
    }

    apply_pp_footnote_forward_closure(lines, &mut members);

    for (index, member) in members.into_iter().enumerate() {
        if member {
            path[index] = Lm2Action::Marginalia;
        }
    }
}

pub(super) fn pp_footnote_region_member(line: &DeepLiquidSourceLine) -> bool {
    line.pp_prior_role.as_deref() == Some("footnote")
        && line.pp_prior_score.is_some_and(|score| score >= 0.80)
        && !line
            .text
            .split_whitespace()
            .collect::<String>()
            .chars()
            .all(|ch| ch.is_ascii_digit())
}

pub(super) fn apply_pp_footnote_forward_closure(
    lines: &[DeepLiquidSourceLine],
    members: &mut [bool],
) {
    let mut index = 0usize;
    while index < lines.len() {
        if !members[index] {
            index += 1;
            continue;
        }
        while index < lines.len() && members[index] {
            index += 1;
        }
        let run_end = index;
        let page_index = lines[run_end - 1].page_index;
        let mut added = 0usize;
        while index < lines.len()
            && added < 16
            && lines[index].page_index == page_index
            && pp_footnote_forward_closure_eligible(&lines[index])
        {
            members[index] = true;
            index += 1;
            added += 1;
        }
    }
}

pub(super) fn pp_footnote_forward_closure_eligible(line: &DeepLiquidSourceLine) -> bool {
    if !pp_footnote_span_closure_eligible(line) {
        return false;
    }
    if line.role_hint.is_some_and(|role| {
        matches!(
            role,
            LiquidBlockRole::Title
                | LiquidBlockRole::Heading
                | LiquidBlockRole::Subheading
                | LiquidBlockRole::Table
        )
    }) {
        return false;
    }
    pp_footnote_forward_continuation_like(line)
}

pub(super) fn pp_footnote_forward_continuation_like(line: &DeepLiquidSourceLine) -> bool {
    let lower = normalize_text(&line.text);
    let y_bottom = line.bottom / line.page_height.max(1.0);
    has_legal_note_cue(&lower)
        || line.below_footnote_divider
        || line.doc_footnote_continuation
        || line.doc_note_marker > 0
        || line.doc_note_marker_mid_sequence_page
        || line.doc_note_marker_follows_previous_page
        || y_bottom < 0.42 && (line.font_ratio_doc < 1.02 || line.font_ratio_page < 1.02)
}

pub(super) fn pp_footnote_span_closure_eligible(line: &DeepLiquidSourceLine) -> bool {
    let lower = normalize_text(&line.text);
    if lower.trim().is_empty()
        || looks_like_toc_entry(&lower)
        || looks_like_running_header(&lower)
        || looks_like_small_font_page_furniture(&lower)
    {
        return false;
    }
    if line
        .role_hint
        .is_some_and(|role| role_action(role) == Lm2Action::HideNoise)
    {
        return false;
    }
    if line.doc_repeated_edge_text && (line.doc_repeated_top_edge || line.doc_repeated_bottom_edge)
    {
        return false;
    }
    if line
        .text
        .split_whitespace()
        .collect::<String>()
        .chars()
        .all(|ch| ch.is_ascii_digit())
    {
        return false;
    }
    true
}

pub(super) fn pp_footnote_span_continuation_like(line: &DeepLiquidSourceLine) -> bool {
    let lower = normalize_text(&line.text);
    let y_bottom = line.bottom / line.page_height.max(1.0);
    has_legal_note_cue(&lower)
        || line.below_footnote_divider
        || line.page_has_footnote_divider
        || line.doc_footnote_state
        || line.doc_footnote_continuation
        || line.doc_note_marker > 0
        || line.doc_note_marker_mid_sequence_page
        || line.doc_note_marker_follows_previous_page
        || y_bottom < 0.42 && (line.font_ratio_doc < 1.02 || line.font_ratio_page < 1.02)
}

pub(super) fn apply_body_preservation_guard(
    lines: &[DeepLiquidSourceLine],
    path: &mut [Lm2Action],
) {
    for (line, action) in lines.iter().zip(path.iter_mut()) {
        if *action == Lm2Action::Keep {
            continue;
        }
        if body_preservation_candidate(line, *action) {
            *action = Lm2Action::Keep;
        }
    }
}

pub(super) fn apply_static_front_overlay(
    overlay: &Lm2StaticFrontOverlay,
    path: &Path,
    decoded: &mut [(DeepLiquidSourceLine, Lm2Action)],
) {
    let doc_key = path.display().to_string().replace('\\', "/").to_lowercase();
    let Some(rows) = overlay.roles_by_doc_line.get(&doc_key) else {
        return;
    };
    for (line, action) in decoded.iter_mut() {
        let Some(role) = rows.get(&line.id).copied() else {
            continue;
        };
        if matches!(
            role,
            LiquidBlockRole::Title | LiquidBlockRole::Heading | LiquidBlockRole::Subheading
        ) && noise_hint_page_furniture(line)
        {
            *action = Lm2Action::HideNoise;
            line.role_hint = Some(LiquidBlockRole::Noise);
            continue;
        }
        match role {
            LiquidBlockRole::Title | LiquidBlockRole::Heading | LiquidBlockRole::Subheading => {
                *action = Lm2Action::Keep;
                line.role_hint = Some(role);
            }
            LiquidBlockRole::Marginalia | LiquidBlockRole::Footnote => {
                *action = Lm2Action::Marginalia;
                line.role_hint = Some(LiquidBlockRole::Marginalia);
            }
            role if role_action(role) == Lm2Action::HideNoise => {
                *action = Lm2Action::HideNoise;
                line.role_hint = Some(role);
            }
            _ => {}
        }
    }
}

pub(super) fn body_preservation_candidate(line: &DeepLiquidSourceLine, action: Lm2Action) -> bool {
    if line.below_footnote_divider || line.doc_footnote_state || line.doc_footnote_continuation {
        return false;
    }
    let lower = normalize_text(&line.text);
    if lower.trim().is_empty()
        || looks_like_note_start(&line.text)
        || has_legal_note_cue(&lower)
        || looks_like_toc_entry(&lower)
        || looks_like_running_header(&lower)
        || looks_like_small_font_page_furniture(&lower)
    {
        return false;
    }
    if line.doc_repeated_edge_text && (line.doc_repeated_top_edge || line.doc_repeated_bottom_edge)
    {
        return false;
    }
    let words = word_count(&lower);
    if words < 7 {
        return false;
    }
    let y_bottom = line.bottom / line.page_height.max(1.0);
    if !(0.12..=0.90).contains(&y_bottom) {
        return false;
    }
    let width = (line.right - line.left).max(0.0) / line.page_width.max(1.0);
    if width < 0.28 {
        return false;
    }
    let body_font_like = line.font_ratio_page >= 0.92
        && line.font_ratio_doc >= 0.92
        && line.doc_font_body_z.abs() <= line.doc_font_footnote_z.abs() + 0.20;
    if !body_font_like {
        return false;
    }
    match action {
        Lm2Action::HideNoise => words >= 8 && uppercase_ratio(&line.text) < 0.72,
        Lm2Action::Marginalia => {
            words >= 10
                && y_bottom >= 0.30
                && !line.doc_note_marker_mid_sequence_page
                && !line.doc_note_marker_follows_previous_page
        }
        Lm2Action::Keep => false,
    }
}

pub(super) fn apply_document_toc_overlay(decoded: &mut [(DeepLiquidSourceLine, Lm2Action)]) {
    let toc_titles = decoded
        .iter()
        .filter_map(|(line, _)| {
            if lm2_toc_dotleader_line(&line.text) {
                let title = lm2_toc_normalize(&line.text, true);
                (title.len() >= 3).then_some(title)
            } else {
                None
            }
        })
        .collect::<HashSet<_>>();
    if toc_titles.is_empty() {
        return;
    }

    for (line, action) in decoded.iter_mut() {
        let is_dotleader = lm2_toc_dotleader_line(&line.text);
        let normalized = lm2_toc_normalize(&line.text, is_dotleader);
        let in_toc = !normalized.is_empty() && toc_titles.contains(&normalized);
        if is_dotleader {
            if in_toc || lm2_toc_dotleader_fallback(line) {
                *action = Lm2Action::HideNoise;
            }
            continue;
        }
        if in_toc && *action != Lm2Action::Keep && !lm2_toc_overlay_repeated_edge(line) {
            *action = Lm2Action::Keep;
            line.role_hint = Some(LiquidBlockRole::Heading);
        }
    }
}

/// Hide an early contents page that uses aligned terminal page locators rather
/// than dot leaders.  This guard is intentionally page-bounded: a literal
/// contents header plus several outline rows must be present, and only the run
/// from that header through the final locator is affected.  A real article
/// paragraph later on the same page is therefore preserved.
pub(super) fn apply_strict_no_dot_toc_page_guard(
    decoded: &mut [(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    let mut by_page = BTreeMap::<usize, Vec<(usize, usize)>>::new();
    for (decoded_index, (line, _)) in decoded.iter().enumerate() {
        if line.page_index <= 4 {
            by_page
                .entry(line.page_index)
                .or_default()
                .push((line.line_index, decoded_index));
        }
    }

    let mut ranges = Vec::<(usize, usize, usize)>::new();
    for (page_index, mut lines) in by_page {
        lines.sort_by_key(|(line_index, _)| *line_index);
        let Some((header_position, (header_line, _))) =
            lines.iter().enumerate().find(|(_, (_, decoded_index))| {
                matches!(
                    normalize_text(&decoded[*decoded_index].0.text).as_str(),
                    "contents" | "article contents" | "table of contents"
                )
            })
        else {
            continue;
        };

        let mut locator_rows = 0usize;
        let mut outline_rows = 0usize;
        let mut last_locator_line = None;
        for (line_index, decoded_index) in lines.iter().skip(header_position + 1) {
            let text = collapse_whitespace(&decoded[*decoded_index].0.text);
            let Some(title) = lm2_toc_strip_trailing_locator(&text) else {
                continue;
            };
            locator_rows += 1;
            outline_rows += usize::from(lm2_toc_outline_row(&title));
            last_locator_line = Some(*line_index);
        }
        if locator_rows >= 4
            && outline_rows >= 3
            && let Some(last_locator_line) = last_locator_line
        {
            ranges.push((page_index, *header_line, last_locator_line));
        }
    }

    let mut changed = 0usize;
    for (line, action) in decoded.iter_mut() {
        if ranges.iter().any(|(page_index, first, last)| {
            line.page_index == *page_index && (*first..=*last).contains(&line.line_index)
        }) {
            if *action != Lm2Action::HideNoise || line.role_hint != Some(LiquidBlockRole::Contents)
            {
                changed += 1;
            }
            *action = Lm2Action::HideNoise;
            line.role_hint = Some(LiquidBlockRole::Contents);
        }
    }
    changed
}

/// Hide an early dotleader contents run as a page sequence, including title
/// fragments that were emitted on the line immediately before their locator.
/// Some journals omit a literal `Contents` header and continue the final two
/// locator rows onto the next page before the real `INTRODUCTION` heading.
pub(super) fn apply_dense_dotleader_toc_run_guard(
    decoded: &mut [(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    let mut by_page = BTreeMap::<usize, Vec<(usize, usize)>>::new();
    for (decoded_index, (line, _)) in decoded.iter().enumerate() {
        if line.page_index <= 2 {
            by_page
                .entry(line.page_index)
                .or_default()
                .push((line.line_index, decoded_index));
        }
    }
    for lines in by_page.values_mut() {
        lines.sort_by_key(|(line_index, _)| *line_index);
    }

    let dense_pages = by_page
        .iter()
        .filter_map(|(page_index, lines)| {
            let leaders = lines
                .iter()
                .filter(|(_, decoded_index)| {
                    lm2_toc_dotleader_line(&decoded[*decoded_index].0.text)
                })
                .count();
            (leaders >= 4).then_some(*page_index)
        })
        .collect::<HashSet<_>>();
    if dense_pages.is_empty() {
        return 0;
    }

    let mut ranges = Vec::<(usize, usize, usize)>::new();
    for page_index in dense_pages.iter().copied() {
        let Some(lines) = by_page.get(&page_index) else {
            continue;
        };
        let Some(first) = lines
            .iter()
            .find(|(_, decoded_index)| lm2_toc_dotleader_line(&decoded[*decoded_index].0.text))
            .map(|(line_index, _)| *line_index)
        else {
            continue;
        };
        let last = lines
            .iter()
            .rev()
            .find(|(_, decoded_index)| lm2_toc_dotleader_line(&decoded[*decoded_index].0.text))
            .map(|(line_index, _)| *line_index)
            .unwrap_or(first);
        ranges.push((page_index, first, last));

        let next_page = page_index + 1;
        let Some(next_lines) = by_page.get(&next_page) else {
            continue;
        };
        let first_body_heading = next_lines.iter().find(|(_, decoded_index)| {
            matches!(
                normalize_text(&decoded[*decoded_index].0.text).as_str(),
                "introduction" | "conclusion"
            )
        });
        let Some((body_line, _)) = first_body_heading else {
            continue;
        };
        let continuation_leaders = next_lines
            .iter()
            .filter(|(line_index, decoded_index)| {
                *line_index < *body_line && lm2_toc_dotleader_line(&decoded[*decoded_index].0.text)
            })
            .map(|(line_index, _)| *line_index)
            .collect::<Vec<_>>();
        if let (Some(first), Some(last)) = (
            continuation_leaders.first().copied(),
            continuation_leaders.last().copied(),
        ) {
            ranges.push((next_page, first, last));
        }
    }

    let mut changed = 0usize;
    for (line, action) in decoded.iter_mut() {
        if ranges.iter().any(|(page_index, first, last)| {
            line.page_index == *page_index && (*first..=*last).contains(&line.line_index)
        }) {
            if *action != Lm2Action::HideNoise || line.role_hint != Some(LiquidBlockRole::Contents)
            {
                changed += 1;
            }
            *action = Lm2Action::HideNoise;
            line.role_hint = Some(LiquidBlockRole::Contents);
        }
    }
    changed
}

pub(super) fn lm2_toc_strip_trailing_locator(text: &str) -> Option<String> {
    let trimmed = text.trim();
    let split = trimmed.rfind(char::is_whitespace)?;
    let title = trimmed[..split].trim_end();
    let locator = trimmed[split..]
        .trim()
        .trim_matches(|ch: char| matches!(ch, '.' | ',' | ')' | ']' | ':'));
    if title.is_empty() || locator.is_empty() || locator.len() > 8 {
        return None;
    }
    let numeric = locator.len() <= 4 && locator.chars().all(|ch| ch.is_ascii_digit());
    let roman = locator.chars().all(|ch| {
        matches!(
            ch.to_ascii_lowercase(),
            'i' | 'v' | 'x' | 'l' | 'c' | 'd' | 'm'
        )
    });
    (numeric || roman).then(|| title.to_owned())
}

pub(super) fn lm2_toc_outline_row(title: &str) -> bool {
    let lower = normalize_text(title);
    if lower.starts_with("introduction") || lower.starts_with("conclusion") {
        return true;
    }
    let token = title.split_whitespace().next().unwrap_or_default();
    let marker = token.trim_matches(|ch| matches!(ch, '(' | ')' | '.' | ':' | ';'));
    (!marker.is_empty()
        && marker.len() <= 8
        && marker.chars().all(|ch| {
            matches!(
                ch.to_ascii_lowercase(),
                'i' | 'v' | 'x' | 'l' | 'c' | 'd' | 'm'
            )
        }))
        || (marker.len() == 1 && marker.chars().all(|ch| ch.is_ascii_alphabetic()))
        || (marker.len() <= 2 && marker.chars().all(|ch| ch.is_ascii_digit()))
}

pub(super) fn apply_front_matter_guard(decoded: &mut [(DeepLiquidSourceLine, Lm2Action)]) {
    for (line, action) in decoded.iter_mut() {
        if *action != Lm2Action::Keep {
            continue;
        }
        if noise_hint_page_furniture(line) {
            *action = Lm2Action::HideNoise;
            line.role_hint = Some(LiquidBlockRole::Noise);
        } else if line.page_index == 0 && first_page_author_note_line(line) {
            *action = Lm2Action::Marginalia;
            line.role_hint = Some(LiquidBlockRole::Marginalia);
        }
    }
}

pub(super) fn apply_marginalia_preservation_guard(
    decoded: &mut [(DeepLiquidSourceLine, Lm2Action)],
) {
    for (line, action) in decoded.iter_mut() {
        if *action != Lm2Action::HideNoise || !marginalia_preservation_candidate(line) {
            continue;
        }
        *action = Lm2Action::Marginalia;
        line.role_hint = Some(LiquidBlockRole::Marginalia);
    }
}

pub(super) fn marginalia_preservation_candidate(line: &DeepLiquidSourceLine) -> bool {
    let lower = normalize_text(&line.text);
    if lower.trim().is_empty()
        || looks_like_toc_entry(&lower)
        || lm2_toc_dotleader_line(&line.text)
        || lm2_toc_dotleader_fallback(line)
        || looks_like_running_header(&lower)
        || looks_like_production_slug_boilerplate(&line.text)
        || first_page_journal_masthead(line, &lower)
    {
        return false;
    }
    let y_bottom = line.bottom / line.page_height.max(1.0);
    let marginalia_hint = line
        .role_hint
        .is_some_and(|role| role_action(role) == Lm2Action::Marginalia);
    let url_or_citation_continuation = line.page_index > 0
        && y_bottom < 0.44
        && (line.font_ratio_doc < 0.90 || line.font_ratio_page < 0.90)
        && (has_legal_note_cue(&lower)
            || lower.contains("http://")
            || lower.contains("https://")
            || lower.contains("perma.cc/")
            || lower.contains("supra note")
            || lower.contains("id."));
    if !marginalia_hint && !url_or_citation_continuation {
        return false;
    }
    let note_context = line.below_footnote_divider
        || line.page_has_footnote_divider
        || line.doc_footnote_state
        || line.doc_footnote_continuation
        || line.doc_note_marker > 0
        || line.doc_note_marker_mid_sequence_page
        || line.doc_note_marker_follows_previous_page
        || has_legal_note_cue(&lower)
        || (y_bottom < 0.44 && (line.font_ratio_doc < 1.02 || line.font_ratio_page < 1.02));
    if !note_context {
        return false;
    }
    if word_count(&lower) <= 8
        && uppercase_ratio(&line.text) >= 0.62
        && !looks_like_note_start(&line.text)
    {
        return false;
    }
    true
}

pub(super) fn apply_d1_runtime_zerospend_overlay(
    decoded: &mut [(DeepLiquidSourceLine, Lm2Action)],
) {
    for (line, action) in decoded.iter_mut() {
        if *action != Lm2Action::Keep || !d1_runtime_zerospend_candidate(line) {
            continue;
        }
        *action = Lm2Action::Marginalia;
        line.role_hint = Some(LiquidBlockRole::Footnote);
    }
}

pub(super) fn d1_runtime_zerospend_candidate(line: &DeepLiquidSourceLine) -> bool {
    line.font_ratio_doc <= 1.0
        && line.font_ratio_page_ref <= 0.9
        && line.line_index >= 12
        && d1_runtime_zerospend_cue(&line.text)
}

pub(super) fn d1_runtime_zerospend_cue(text: &str) -> bool {
    let lower = normalize_text(text);
    lower.contains("http://")
        || lower.contains("https://")
        || lower.contains("www.")
        || lower.contains("perma.cc")
        || has_legal_note_cue(&lower)
        || d1_runtime_zerospend_citation_cue(&lower)
}

pub(super) fn d1_runtime_zerospend_citation_cue(lower: &str) -> bool {
    d1_runtime_contains_token(lower, "accord")
        || d1_runtime_contains_token(lower, "contra")
        || d1_runtime_contains_token(lower, "l.j.")
        || lower.contains("u.s.")
        || lower.contains("f.2d")
        || lower.contains("f.3d")
        || lower.contains("f.4th")
        || lower.contains("f. supp")
        || lower.contains("l. rev.")
        || d1_runtime_contains_token(lower, "rev.")
        || d1_runtime_contains_token(lower, "reg.")
        || d1_runtime_contains_token(lower, "stat.")
        || d1_runtime_contains_token(lower, "wl")
        || d1_runtime_contains_token(lower, "no.")
        || d1_runtime_year_paren_cue(lower)
}

pub(super) fn d1_runtime_contains_token(text: &str, needle: &str) -> bool {
    text.split(|ch: char| !(ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-')))
        .any(|token| token == needle)
}

pub(super) fn d1_runtime_year_paren_cue(text: &str) -> bool {
    text.split_whitespace().any(|token| {
        let token = token.trim_matches(|ch: char| matches!(ch, ',' | ';' | ':'));
        token.len() == 5 && token.ends_with(')') && token[..4].chars().all(|ch| ch.is_ascii_digit())
    })
}

pub(super) fn apply_d1_runtime_continuation_overlay(
    decoded: &mut [(DeepLiquidSourceLine, Lm2Action)],
) {
    let mut active = false;
    let mut previous_changed = false;
    let mut hidden_gap = 0usize;
    let mut previous_page: Option<usize> = None;

    for (line, action) in decoded.iter_mut() {
        if previous_page != Some(line.page_index) {
            active = false;
            previous_changed = false;
            hidden_gap = 0;
            previous_page = Some(line.page_index);
        }

        match *action {
            Lm2Action::Marginalia => {
                active = true;
                previous_changed = false;
                hidden_gap = 0;
            }
            Lm2Action::HideNoise => {
                if active {
                    hidden_gap += 1;
                    if hidden_gap > 2 {
                        active = false;
                        previous_changed = false;
                    }
                }
            }
            Lm2Action::Keep => {
                if active && d1_runtime_continuation_candidate(line, previous_changed) {
                    *action = Lm2Action::Marginalia;
                    line.role_hint = Some(LiquidBlockRole::Footnote);
                    previous_changed = true;
                    hidden_gap = 0;
                } else {
                    active = false;
                    previous_changed = false;
                    hidden_gap = 0;
                }
            }
        }
    }
}

pub(super) fn d1_runtime_continuation_candidate(
    line: &DeepLiquidSourceLine,
    previous_changed: bool,
) -> bool {
    let lower = normalize_text(&line.text);
    let citation_cue = d1_runtime_zerospend_cue(&line.text);
    if looks_like_toc_entry(&lower)
        || lm2_toc_dotleader_line(&line.text)
        || looks_like_running_header(&lower)
    {
        return false;
    }
    if looks_like_small_font_page_furniture(&lower) && !citation_cue {
        return false;
    }
    if word_count(&lower) <= 8
        && uppercase_ratio(&line.text) >= 0.62
        && !looks_like_note_start(&line.text)
    {
        return false;
    }

    let small_font = line.font_ratio_doc <= 0.96 || line.font_ratio_page <= 0.96;
    let lower_page = line.bottom / line.page_height.max(1.0) <= 0.45;
    line.below_footnote_divider
        || line.doc_footnote_continuation
        || (small_font && lower_page && (previous_changed || citation_cue))
}

pub(super) fn apply_d1_runtime_immediate_continuation_overlay(
    decoded: &mut [(DeepLiquidSourceLine, Lm2Action)],
) {
    if decoded.len() < 3 {
        return;
    }
    let accepted = (1..decoded.len() - 1)
        .filter(|index| {
            let (line, action) = &decoded[*index];
            *action == Lm2Action::Keep
                && decoded[*index - 1].0.page_index == line.page_index
                && decoded[*index + 1].0.page_index == line.page_index
                && decoded[*index - 1].1 == Lm2Action::Marginalia
                && decoded[*index + 1].1 == Lm2Action::Marginalia
                && d1_runtime_immediate_continuation_candidate(line)
        })
        .collect::<Vec<_>>();

    for index in accepted {
        decoded[index].1 = Lm2Action::Marginalia;
        decoded[index].0.role_hint = Some(LiquidBlockRole::Footnote);
    }
}

pub(super) fn apply_d1_runtime_sandwiched_continuation_overlay(
    decoded: &mut [(DeepLiquidSourceLine, Lm2Action)],
) {
    if decoded.len() < 3 {
        return;
    }
    let accepted = (0..decoded.len())
        .filter(|index| {
            let (line, action) = &decoded[*index];
            *action == Lm2Action::Keep
                && d1_runtime_has_marginalia_neighbor(decoded, *index, -2)
                && d1_runtime_has_marginalia_neighbor(decoded, *index, 2)
                && d1_runtime_immediate_continuation_candidate(line)
        })
        .collect::<Vec<_>>();

    for index in accepted {
        decoded[index].1 = Lm2Action::Marginalia;
        decoded[index].0.role_hint = Some(LiquidBlockRole::Footnote);
    }
}

pub(super) fn apply_d1_runtime_wide_sandwich_overlay(
    decoded: &mut [(DeepLiquidSourceLine, Lm2Action)],
) {
    if decoded.len() < 3 {
        return;
    }
    let accepted = (0..decoded.len())
        .filter(|index| {
            let (line, action) = &decoded[*index];
            *action == Lm2Action::Keep
                && d1_runtime_has_marginalia_neighbor(decoded, *index, -4)
                && d1_runtime_has_marginalia_neighbor(decoded, *index, 4)
                && d1_runtime_wide_sandwich_candidate(line)
        })
        .collect::<Vec<_>>();

    for index in accepted {
        decoded[index].1 = Lm2Action::Marginalia;
        decoded[index].0.role_hint = Some(LiquidBlockRole::Footnote);
    }
}

pub(super) fn d1_runtime_wide_sandwich_candidate(line: &DeepLiquidSourceLine) -> bool {
    let text = collapse_whitespace(&line.text);
    let font_ratio = if line.font_ratio_doc > 0.0 {
        line.font_ratio_doc
    } else {
        line.font_ratio_page
    };
    font_ratio <= 0.95
        && !d1_runtime_artifact_like(&text)
        && !text.contains("....")
        && !d1_runtime_table_stat_like(&text)
}

pub(super) fn apply_d1_runtime_post_wide_cue_overlay(
    decoded: &mut [(DeepLiquidSourceLine, Lm2Action)],
) {
    if decoded.len() < 3 {
        return;
    }
    let accepted = (0..decoded.len())
        .filter(|index| {
            let (line, action) = &decoded[*index];
            *action == Lm2Action::Keep
                && d1_runtime_next_marginalia_count(decoded, *index, 4) >= 2
                && d1_runtime_post_wide_cue_candidate(line)
        })
        .collect::<Vec<_>>();

    for index in accepted {
        decoded[index].1 = Lm2Action::Marginalia;
        decoded[index].0.role_hint = Some(LiquidBlockRole::Footnote);
    }
}

pub(super) fn d1_runtime_post_wide_cue_candidate(line: &DeepLiquidSourceLine) -> bool {
    let text = collapse_whitespace(&line.text);
    let lower = normalize_text(&text);
    let font_ratio = if line.font_ratio_doc > 0.0 {
        line.font_ratio_doc
    } else {
        line.font_ratio_page
    };
    font_ratio <= 0.88
        && (d1_runtime_strong_note_start(&text) || d1_runtime_citation_like(&lower))
        && !d1_runtime_artifact_like(&text)
        && !d1_runtime_table_stat_like(&text)
}

pub(super) fn d1_runtime_strong_note_start(text: &str) -> bool {
    let trimmed = text.trim_start();
    if trimmed.starts_with('*')
        || trimmed.starts_with('†')
        || trimmed.starts_with('‡')
        || trimmed.starts_with('§')
    {
        return trimmed.chars().nth(1).is_some_and(|ch| ch.is_whitespace());
    }
    d1_runtime_strong_numeric_note_start(trimmed)
}

pub(super) fn d1_runtime_next_marginalia_count(
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
    index: usize,
    window: usize,
) -> usize {
    let page_index = decoded[index].0.page_index;
    let end = (index + window + 1).min(decoded.len());
    (index + 1..end)
        .filter(|neighbor| {
            decoded[*neighbor].0.page_index == page_index
                && decoded[*neighbor].1 == Lm2Action::Marginalia
        })
        .count()
}

pub(super) fn apply_d1_runtime_postcue_citation_next1_overlay(
    decoded: &mut [(DeepLiquidSourceLine, Lm2Action)],
) {
    if decoded.len() < 2 {
        return;
    }
    let accepted = (0..decoded.len())
        .filter(|index| {
            let (line, action) = &decoded[*index];
            *action == Lm2Action::Keep
                && d1_runtime_next_marginalia_count(decoded, *index, 4) >= 1
                && d1_runtime_postcue_citation_next1_candidate(line)
        })
        .collect::<Vec<_>>();

    for index in accepted {
        decoded[index].1 = Lm2Action::Marginalia;
        decoded[index].0.role_hint = Some(LiquidBlockRole::Footnote);
    }
}

pub(super) fn d1_runtime_postcue_citation_next1_candidate(line: &DeepLiquidSourceLine) -> bool {
    let text = collapse_whitespace(&line.text);
    let lower = normalize_text(&text);
    let font_ratio = if line.font_ratio_doc > 0.0 {
        line.font_ratio_doc
    } else {
        line.font_ratio_page
    };
    font_ratio <= 0.95
        && d1_runtime_postcue_citation_next1_cue(&lower, &text)
        && !d1_runtime_artifact_like(&text)
        && !d1_runtime_table_stat_like(&text)
}

pub(super) fn d1_runtime_postcue_citation_next1_cue(lower: &str, text: &str) -> bool {
    lower.contains("http://")
        || lower.contains("https://")
        || lower.contains("perma.cc")
        || text.contains('§')
        || d1_runtime_contains_token(lower, "u.s.")
        || d1_runtime_contains_token(lower, "s. ct.")
        || d1_runtime_contains_token(lower, "f.2d")
        || d1_runtime_contains_token(lower, "f.3d")
        || d1_runtime_contains_token(lower, "f. supp")
        || d1_runtime_contains_token(lower, "l. rev.")
        || d1_runtime_contains_token(lower, "j.")
}

pub(super) fn apply_d1_runtime_near8_cue_overlay(
    decoded: &mut [(DeepLiquidSourceLine, Lm2Action)],
) {
    if decoded.len() < 2 {
        return;
    }
    let accepted = (0..decoded.len())
        .filter(|index| {
            let (line, action) = &decoded[*index];
            *action == Lm2Action::Keep
                && d1_runtime_nearby_marginalia_count(decoded, *index, 8) >= 4
                && d1_runtime_near8_cue_candidate(line)
        })
        .collect::<Vec<_>>();

    for index in accepted {
        decoded[index].1 = Lm2Action::Marginalia;
        decoded[index].0.role_hint = Some(LiquidBlockRole::Footnote);
    }
}

pub(super) fn d1_runtime_near8_cue_candidate(line: &DeepLiquidSourceLine) -> bool {
    let text = collapse_whitespace(&line.text);
    let lower = normalize_text(&text);
    let font_ratio = if line.font_ratio_doc > 0.0 {
        line.font_ratio_doc
    } else {
        line.font_ratio_page
    };
    let center_y = ((line.top + line.bottom) * 0.5) / line.page_height.max(1.0);
    font_ratio <= 0.85
        && center_y >= 0.30
        && (looks_like_marginalia_note_block_start(&text)
            || d1_runtime_near8_citation_cue(&lower, &text))
        && !d1_runtime_artifact_like(&text)
        && !d1_runtime_table_stat_like(&text)
}

pub(super) fn d1_runtime_near8_citation_cue(lower: &str, text: &str) -> bool {
    lower.contains("http://")
        || lower.contains("https://")
        || lower.contains("perma.cc")
        || text.contains('§')
        || lower.contains("(18")
        || lower.contains("(19")
        || lower.contains("(20")
        || d1_runtime_contains_token(lower, "u.s.")
        || d1_runtime_contains_token(lower, "s. ct.")
        || d1_runtime_contains_token(lower, "f.2d")
        || d1_runtime_contains_token(lower, "f.3d")
        || d1_runtime_contains_token(lower, "f. supp")
        || d1_runtime_contains_token(lower, "l. rev.")
        || d1_runtime_contains_token(lower, "rev.")
        || d1_runtime_contains_token(lower, "j.")
}

pub(super) fn d1_runtime_nearby_marginalia_count(
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
    index: usize,
    window: usize,
) -> usize {
    let page_index = decoded[index].0.page_index;
    let start = index.saturating_sub(window);
    let end = (index + window + 1).min(decoded.len());
    (start..end)
        .filter(|neighbor| {
            *neighbor != index
                && decoded[*neighbor].0.page_index == page_index
                && decoded[*neighbor].1 == Lm2Action::Marginalia
        })
        .count()
}

pub(super) fn apply_d1_runtime_geometric_zone_overlay(
    decoded: &mut [(DeepLiquidSourceLine, Lm2Action)],
) {
    for (line, action) in decoded.iter_mut() {
        if *action != Lm2Action::Keep || !d1_runtime_geometric_zone_candidate(line) {
            continue;
        }
        *action = Lm2Action::Marginalia;
        line.role_hint = Some(LiquidBlockRole::Footnote);
    }
}

pub(super) fn d1_runtime_geometric_zone_candidate(line: &DeepLiquidSourceLine) -> bool {
    if !line.in_footnote_zone || d1_runtime_artifact_like(&line.text) {
        return false;
    }
    let text = collapse_whitespace(&line.text);
    let lower = normalize_text(&text);
    if text.contains('%') {
        return false;
    }
    if looks_like_toc_entry(&lower)
        || lm2_toc_dotleader_line(&text)
        || looks_like_running_header(&lower)
        || looks_like_small_font_page_furniture(&lower)
        || looks_like_page_label_furniture(&text)
        || (d1_runtime_table_stat_like(&text)
            && !looks_like_note_start(&text)
            && !d1_runtime_citation_like(&lower))
    {
        return false;
    }
    let word_count = word_count(&lower);
    if word_count <= 8 && uppercase_ratio(&text) >= 0.62 && !looks_like_note_start(&text) {
        return false;
    }
    if line.centered
        && line.bold
        && word_count <= 12
        && !looks_like_note_start(&text)
        && !d1_runtime_citation_like(&lower)
    {
        return false;
    }
    let note_evidence = looks_like_note_start(&text)
        || d1_runtime_citation_like(&lower)
        || has_legal_note_cue(&lower);
    if !note_evidence {
        return false;
    }
    let font_ratio = if line.font_ratio_doc > 0.0 {
        line.font_ratio_doc
    } else {
        line.font_ratio_page
    };
    let explicit_divider_context = line.below_footnote_divider || line.page_has_footnote_divider;
    let font_threshold = if explicit_divider_context { 0.90 } else { 0.84 };
    let page_ref_threshold = if explicit_divider_context { 0.86 } else { 0.82 };
    let footnote_size = font_ratio <= font_threshold
        || line.font_ratio_page_ref <= page_ref_threshold
        || (explicit_divider_context
            && line.doc_font_footnote_size > 0.0
            && line.font_height <= line.doc_font_footnote_size + 0.35);
    let y_center = ((line.top + line.bottom) * 0.5) / line.page_height.max(1.0);
    footnote_size && y_center >= 0.30
}

pub(super) fn apply_d1_runtime_wide_divider_guard_overlay(
    decoded: &mut [(DeepLiquidSourceLine, Lm2Action)],
) {
    for (line, action) in decoded.iter_mut() {
        if *action != Lm2Action::Keep || !d1_runtime_wide_divider_guard_candidate(line) {
            continue;
        }
        *action = Lm2Action::Marginalia;
        line.role_hint = Some(LiquidBlockRole::Footnote);
    }
}

pub(super) fn tfr_num_token_count(text: &str) -> usize {
    text.split_whitespace()
        .filter(|w| w.chars().any(|c| c.is_ascii_digit()))
        .count()
}

pub(super) fn tfr_alpha_word_count(text: &str) -> usize {
    text.split_whitespace()
        .filter(|w| w.chars().filter(|c| c.is_alphabetic()).count() >= 3)
        .count()
}

pub(super) fn tfr_ends_with_sentence_punct(text: &str) -> bool {
    text.trim_end()
        .chars()
        .last()
        .map(|c| matches!(c, '.' | ';' | ':' | ','))
        .unwrap_or(false)
}

pub(super) fn tfr_has_bullet(text: &str) -> bool {
    text.chars().any(|c| {
        matches!(
            c,
            '\u{2022}' | '\u{25CF}' | '\u{25AA}' | '\u{25E6}' | '\u{2023}'
        )
    })
}

pub(super) fn tfr_width_norm(line: &DeepLiquidSourceLine) -> f32 {
    (line.right - line.left).max(0.0) / line.page_width.max(1.0)
}

/// Strong PDF-native table/figure signals that do not need page context:
/// figure bullets (short, non-prose) and number-dense grid rows. Prose and
/// citations are excluded (>=3 alpha words or sentence-ending punctuation), and
/// full-width lines are excluded.
pub(super) fn table_figure_router_strong_candidate(line: &DeepLiquidSourceLine) -> bool {
    let text = collapse_whitespace(&line.text);
    if text.is_empty() {
        return false;
    }
    let aw = tfr_alpha_word_count(&text);
    let wn = tfr_width_norm(line);
    if tfr_has_bullet(&text) && aw <= 4 && wn < 0.45 {
        return true;
    }
    let prose = aw >= 3 || tfr_ends_with_sentence_punct(&text);
    if !prose && wn < 0.6 {
        let nt = tfr_num_token_count(&text);
        if nt >= 3 {
            return true;
        }
        if nt >= 2 && (text.contains('%') || wn < 0.5) {
            return true;
        }
    }
    false
}

/// A short, narrow numeric cell — routed only when it sits in a column of
/// several left-aligned narrow siblings (otherwise it is a footnote marker or a
/// page number). The column check is applied by the caller.
pub(super) fn table_figure_router_cell_like(line: &DeepLiquidSourceLine) -> bool {
    let text = collapse_whitespace(&line.text);
    if text.is_empty() || text.chars().count() > 12 {
        return false;
    }
    let aw = tfr_alpha_word_count(&text);
    if aw >= 3 || tfr_ends_with_sentence_punct(&text) {
        return false;
    }
    if !text.chars().any(|c| c.is_ascii_digit() || c == '%') {
        return false;
    }
    tfr_width_norm(line) < 0.20
}

/// A short line repeated across the document (>=3 times) is running-header /
/// repeated-furniture noise. Guarded against repeated body sentences.
pub(super) fn table_figure_router_repeated_furniture_like(line: &DeepLiquidSourceLine) -> bool {
    if line.doc_repeated_text_count < 3 {
        return false;
    }
    let text = collapse_whitespace(&line.text);
    let n = text.chars().count();
    if text.is_empty() || n > 70 {
        return false;
    }
    // Don't hide a genuine repeated body sentence.
    if tfr_ends_with_sentence_punct(&text) && tfr_alpha_word_count(&text) >= 6 {
        return false;
    }
    true
}

/// Routes confident in-text table/figure lines from Keep to HideNoise. Enabled
/// by default; set `LAWPDF_LM2_TABLE_FIGURE_ROUTER=0` to disable. Attacks the
/// dominant `hide_noise->keep` error where TRAIN has ~no table/noise examples;
/// uses PDF-native geometry, no runtime vision model.
pub(super) fn apply_table_figure_router_overlay(decoded: &mut [(DeepLiquidSourceLine, Lm2Action)]) {
    let mut narrow_lefts: BTreeMap<usize, Vec<f32>> = BTreeMap::new();
    for (line, _) in decoded.iter() {
        if tfr_width_norm(line) < 0.30 {
            narrow_lefts
                .entry(line.page_index)
                .or_default()
                .push(line.left);
        }
    }
    for (line, action) in decoded.iter_mut() {
        if *action != Lm2Action::Keep {
            continue;
        }
        // Table/figure content is tagged `Table` so a future "display tables/figures"
        // toggle can resurface it; repeated running-header furniture is tagged
        // `Header` (never resurfaced). Both map to HideNoise, so they are hidden now.
        let role = if table_figure_router_strong_candidate(line) {
            Some(LiquidBlockRole::Table)
        } else if table_figure_router_repeated_furniture_like(line) {
            Some(LiquidBlockRole::Header)
        } else if table_figure_router_cell_like(line)
            && narrow_lefts
                .get(&line.page_index)
                .map(|lefts| {
                    lefts
                        .iter()
                        .filter(|l| (**l - line.left).abs() < 6.0)
                        .count()
                })
                .unwrap_or(0)
                >= 4
        {
            // self + >=3 siblings sharing the same left edge => a real column.
            Some(LiquidBlockRole::Table)
        } else {
            None
        };
        if let Some(role) = role {
            *action = Lm2Action::HideNoise;
            line.role_hint = Some(role);
        }
    }

    // Phase 2 — region contiguity. A table/figure is a compact vertical block, so
    // fill short non-prose Keep lines that fall inside a compact band of >=3
    // HideNoise lines on the same page (text cells the per-line rules missed). The
    // compactness guard (< 0.45 of page height) excludes header+footer whole-page
    // spans so body is not swept up.
    let mut hn_band: BTreeMap<usize, (f32, f32, f32, usize)> = BTreeMap::new();
    for (line, action) in decoded.iter() {
        if *action == Lm2Action::HideNoise {
            let e = hn_band.entry(line.page_index).or_insert((
                f32::MAX,
                f32::MIN,
                line.page_height.max(1.0),
                0,
            ));
            e.0 = e.0.min(line.top);
            e.1 = e.1.max(line.bottom);
            e.3 += 1;
        }
    }
    for (line, action) in decoded.iter_mut() {
        if *action != Lm2Action::Keep {
            continue;
        }
        let Some(&(top, bottom, page_height, count)) = hn_band.get(&line.page_index) else {
            continue;
        };
        if count < 3 || (bottom - top) / page_height > 0.45 {
            continue;
        }
        if line.top < top - 2.0 || line.bottom > bottom + 2.0 {
            continue;
        }
        let text = collapse_whitespace(&line.text);
        if text.is_empty()
            || tfr_alpha_word_count(&text) >= 6
            || tfr_ends_with_sentence_punct(&text)
            || tfr_width_norm(line) > 0.6
        {
            continue;
        }
        *action = Lm2Action::HideNoise;
        line.role_hint = Some(LiquidBlockRole::Table);
    }
}

pub(super) fn apply_page_object_overlay(decoded: &mut [(DeepLiquidSourceLine, Lm2Action)]) {
    for (line, action) in decoded.iter_mut() {
        if *action != Lm2Action::Keep {
            continue;
        }
        if !page_object_overlay_candidate(line) {
            continue;
        }
        *action = Lm2Action::HideNoise;
        line.role_hint = Some(LiquidBlockRole::Table);
    }
}

pub(super) fn page_object_overlay_candidate(line: &DeepLiquidSourceLine) -> bool {
    line.page_object_hide_candidate_guarded
        || line.page_object_ruled_row_membership
        || line.page_object_path15_candidate
        || line.page_object_ruled_or_path8_candidate && page_object_short_nonprose(line)
}

pub(super) fn page_object_short_nonprose(line: &DeepLiquidSourceLine) -> bool {
    let text = collapse_whitespace(&line.text);
    !text.is_empty()
        && text.chars().count() <= 90
        && tfr_alpha_word_count(&text) < 8
        && !tfr_ends_with_sentence_punct(&text)
}

pub(super) fn apply_page_object_tuned_overlay(decoded: &mut [(DeepLiquidSourceLine, Lm2Action)]) {
    for (line, action) in decoded.iter_mut() {
        if !matches!(*action, Lm2Action::Keep | Lm2Action::Marginalia) {
            continue;
        }
        if !line.page_object_ruled_row_membership {
            continue;
        }
        if page_object_tuned_keep_preserve_candidate(line) {
            continue;
        }
        *action = Lm2Action::HideNoise;
        line.role_hint = Some(LiquidBlockRole::Table);
    }

    for (line, action) in decoded.iter_mut() {
        if *action == Lm2Action::Keep {
            continue;
        }
        if !page_object_tuned_body_rescue_candidate(line) {
            continue;
        }
        *action = Lm2Action::Keep;
        line.role_hint = Some(LiquidBlockRole::Paragraph);
    }
}

pub(super) fn page_object_tuned_body_rescue_candidate(line: &DeepLiquidSourceLine) -> bool {
    if line.in_footnote_zone
        || line.below_footnote_divider
        || line.doc_footnote_state
        || line.doc_footnote_continuation
    {
        return false;
    }
    if line.doc_repeated_edge_text
        || line.doc_repeated_top_edge
        || line.doc_repeated_bottom_edge
        || page_object_tuned_edge_line(line)
    {
        return false;
    }
    if line.page_object_hide_candidate
        || line.page_object_ruled_or_path8_candidate
        || line.page_table_column_like
    {
        return false;
    }
    let text = collapse_whitespace(&line.text);
    !text.is_empty()
        && tfr_alpha_word_count(&text) >= 7
        && tfr_width_norm(line) >= 0.50
        && line.font_ratio_doc >= 0.80
        && digit_density(&text) <= 0.25
}

pub(super) fn page_object_tuned_keep_preserve_candidate(line: &DeepLiquidSourceLine) -> bool {
    page_object_tuned_legal_table_keep_like(line) || page_object_tuned_prose_keep_like(line)
}

pub(super) fn page_object_tuned_legal_table_keep_like(line: &DeepLiquidSourceLine) -> bool {
    let text = collapse_whitespace(&line.text);
    if tfr_alpha_word_count(&text) < 3 {
        return false;
    }
    let lower = text.to_ascii_lowercase();
    let padded = format!(" {lower} ");
    text.contains('§')
        || padded.contains(" code ")
        || lower.contains("code ann")
        || lower.contains("ct. r.")
        || lower.contains("rev. code")
        || padded.contains(" court ")
        || lower.contains("ann.")
        || lower.contains("stat.")
        || padded.contains(" rule ")
        || padded.contains(" rules ")
}

pub(super) fn page_object_tuned_prose_keep_like(line: &DeepLiquidSourceLine) -> bool {
    let text = collapse_whitespace(&line.text);
    !text.is_empty()
        && tfr_alpha_word_count(&text) >= 8
        && tfr_width_norm(line) >= 0.45
        && digit_density(&text) <= 0.20
}

pub(super) fn page_object_tuned_edge_line(line: &DeepLiquidSourceLine) -> bool {
    let page_height = line.page_height.max(1.0);
    line.top < 45.0 || line.bottom > page_height - 45.0
}

pub(super) fn digit_density(text: &str) -> f32 {
    let total = text.chars().count().max(1) as f32;
    let digits = text.chars().filter(|ch| ch.is_ascii_digit()).count() as f32;
    digits / total
}

pub(super) fn d1_runtime_wide_divider_guard_candidate(line: &DeepLiquidSourceLine) -> bool {
    if line.page_index == 0 || !line.below_footnote_divider || d1_runtime_artifact_like(&line.text)
    {
        return false;
    }
    let text = collapse_whitespace(&line.text);
    if text.is_empty() || text.contains('%') || d1_runtime_table_stat_like(&text) {
        return false;
    }
    let lower = normalize_text(&text);
    if looks_like_toc_entry(&lower)
        || lm2_toc_dotleader_line(&text)
        || looks_like_running_header(&lower)
        || looks_like_small_font_page_furniture(&lower)
        || looks_like_page_label_furniture(&text)
    {
        return false;
    }
    if line.font_ratio_page_ref > 0.90 {
        return false;
    }
    let y_from_top = (line.page_height - line.top) / line.page_height.max(1.0);
    if y_from_top < 0.55 {
        return false;
    }
    word_count(&lower) <= 42
}

pub(super) fn apply_d1_runtime_footer_artifact_overlay(
    decoded: &mut [(DeepLiquidSourceLine, Lm2Action)],
) {
    for (line, action) in decoded.iter_mut() {
        if *action == Lm2Action::HideNoise || !d1_runtime_footer_artifact_candidate(&line.text) {
            continue;
        }
        *action = Lm2Action::HideNoise;
        line.role_hint = Some(LiquidBlockRole::Noise);
    }
}

pub(super) fn d1_runtime_footer_artifact_candidate(text: &str) -> bool {
    let text = collapse_whitespace(text);
    if text.is_empty() {
        return false;
    }
    let lower = normalize_text(&text);
    if lower.contains("email:") || lower.contains("phone:") || lower.contains("tel:") {
        return false;
    }
    if text.contains('@') {
        return false;
    }
    lower.contains(".indd")
        || lower.contains("doi:")
        || lower.contains("doi.org/")
        || lower.contains("accepted for inclusion")
        || lower.contains("law archive of scholarship")
        || lower.contains("available at:")
        || lower.contains("for more information, please contact")
        || lower.contains("brought to you for free and open access")
}

/// Enforce only the high-confidence role boundaries needed for safe block
/// assembly. Learned decoders may intentionally replace layout hints, but a
/// divider, a recognisable running head at the first line of a page, or
/// repository boilerplate at the physical page edge must never be flattened
/// into surrounding prose. Likewise, a small-font line already identified by
/// the layout pass as marginalia and placed inside the footnote zone must not
/// be promoted back into body text by a later overlay.
pub(super) fn apply_final_assembly_safety_guards(
    decoded: &mut [(DeepLiquidSourceLine, Lm2Action)],
) {
    for (line, action) in decoded.iter_mut() {
        let text = collapse_whitespace(&line.text);
        let lower = normalize_text(&text);
        let page_height = line.page_height.max(1.0);
        let at_physical_edge = line.top <= page_height * 0.08 || line.bottom >= page_height * 0.88;
        let hard_furniture = lm2_blocksplit_divider_like(&text)
            || lower == "footnote continued on next page"
            || (looks_like_running_header(&lower) && (line.line_index <= 2 || at_physical_edge))
            || (d1_runtime_footer_artifact_candidate(&text) && at_physical_edge)
            || (line.doc_repeated_edge_text
                && (line.doc_repeated_top_edge || line.doc_repeated_bottom_edge))
            || looks_like_page_label_furniture(&text);
        if hard_furniture {
            *action = Lm2Action::HideNoise;
            line.role_hint = Some(LiquidBlockRole::Noise);
            continue;
        }

        // `segment_block_toc_like` is deliberately not authoritative here.
        // PyMuPDF can place the final contents rows and the first real article
        // paragraph in one segment; hiding the whole segment then deletes body
        // text and any footnotes on that page.  The line-local leader tests are
        // strong enough for this final safety pass.  No-leader contents pages
        // are handled by the bounded page-level overlay instead.
        if lm2_toc_dotleader_line(&text) || lm2_toc_dotleader_fallback(line) {
            *action = Lm2Action::HideNoise;
            line.role_hint = Some(LiquidBlockRole::Contents);
            continue;
        }

        if *action == Lm2Action::HideNoise
            && line.role_hint != Some(LiquidBlockRole::Contents)
            && !at_physical_edge
            && word_count(&text) <= 20
            && uppercase_ratio(&text) >= 0.55
            && lm2_substantive_section_heading(&text)
        {
            *action = Lm2Action::Keep;
            line.role_hint = Some(LiquidBlockRole::Heading);
            continue;
        }

        let marginalia_hint = line.role_hint.is_some_and(|role| {
            matches!(
                role,
                LiquidBlockRole::Footnote | LiquidBlockRole::Marginalia
            )
        });
        let footnote_geometry = line.in_footnote_zone || line.below_footnote_divider;
        let footnote_font = line.font_ratio_page_ref <= 0.92
            || line.font_ratio_page <= 0.92
            || line.font_ratio_doc <= 0.92;
        if marginalia_hint && footnote_geometry && footnote_font {
            *action = Lm2Action::Marginalia;
            line.role_hint = Some(LiquidBlockRole::Footnote);
        }
    }
}

/// Recover a displayed statutory subdivision that the line model mistakes for
/// footnote prose merely because it uses a smaller font.  The contract is the
/// physical one visible in the source: an ordinary body line ending in a
/// colon, followed in the very same body segment by a parenthesized letter and
/// then a parenthesized number.  A footnote-zone or ruled/table line is never
/// eligible.
pub(super) fn apply_same_segment_statutory_subdivision_body_rescue(
    decoded: &mut [(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    let mut by_page = BTreeMap::<usize, Vec<usize>>::new();
    for (index, (line, _)) in decoded.iter().enumerate() {
        by_page.entry(line.page_index).or_default().push(index);
    }
    for indices in by_page.values_mut() {
        indices.sort_by_key(|index| decoded[*index].0.line_index);
    }

    let mut rescue = BTreeSet::new();
    for indices in by_page.values() {
        for anchor_position in 0..indices.len().saturating_sub(2) {
            let anchor_index = indices[anchor_position];
            let letter_index = indices[anchor_position + 1];
            let (anchor, anchor_action) = &decoded[anchor_index];
            let letter = &decoded[letter_index].0;
            if *anchor_action != Lm2Action::Keep
                || !anchor.text.trim_end().ends_with(':')
                || !lm2_body_segment_line(anchor)
                || !lm2_body_segment_line(letter)
                || anchor.segment_block_id != letter.segment_block_id
                || letter.line_index != anchor.line_index + 1
                || !lm2_parenthesized_alpha_lead(&letter.text)
                || letter.font_height > anchor.font_height * 0.92
            {
                continue;
            }

            let numeric_position = indices
                .iter()
                .enumerate()
                .skip(anchor_position + 2)
                .take(6)
                .find_map(|(position, index)| {
                    let line = &decoded[*index].0;
                    (line.segment_block_id == anchor.segment_block_id
                        && lm2_parenthesized_numeric_lead(&line.text))
                    .then_some(position)
                });
            let Some(numeric_position) = numeric_position else {
                continue;
            };

            let subdivision_font = letter.font_height.max(1.0);
            for index in indices
                .iter()
                .skip(anchor_position + 1)
                .take_while(|index| {
                    let line = &decoded[**index].0;
                    line.segment_block_id == anchor.segment_block_id
                        && (line.line_index <= decoded[indices[numeric_position]].0.line_index
                            || line.font_height <= subdivision_font * 1.10)
                })
            {
                rescue.insert(*index);
            }
        }
    }

    let mut repaired = 0usize;
    for index in rescue {
        let (line, action) = &mut decoded[index];
        if *action != Lm2Action::Keep || line.role_hint != Some(LiquidBlockRole::Paragraph) {
            repaired += 1;
        }
        *action = Lm2Action::Keep;
        line.role_hint = Some(LiquidBlockRole::Paragraph);
    }
    repaired
}

pub(super) fn lm2_body_segment_line(line: &DeepLiquidSourceLine) -> bool {
    line.segment_block_shape.eq_ignore_ascii_case("body")
        && !line.in_footnote_zone
        && !line.below_footnote_divider
        && !line.in_ruled_cell
        && !line.ruled_row_membership_exact
        && !line.page_table_column_like
        && !line.page_object_overlaps_image_bbox
}

/// A late note-continuation overlay can mark a small-font quotation or
/// displayed definition as being in the note zone even when the page
/// segmenter still proves that it belongs to an ordinary body block. This
/// predicate deliberately ignores that mutable note-state, but retains the
/// immutable body-shape and table/figure exclusion gates.
pub(super) fn lm2_source_backed_body_display_line(line: &DeepLiquidSourceLine) -> bool {
    line.segment_block_id != 0
        && line.segment_block_shape.eq_ignore_ascii_case("body")
        && !line.segment_block_footnote_like
        && !line.below_footnote_divider
        && !line.in_ruled_cell
        && !line.ruled_row_membership_exact
        && !line.page_table_column_like
        && !line.page_object_overlaps_image_bbox
        && line.font_ratio_doc >= 0.84
}

pub(super) fn lm2_parenthesized_alpha_lead(text: &str) -> bool {
    let bytes = text.trim_start().as_bytes();
    bytes.len() >= 4
        && bytes[0] == b'('
        && bytes[1].is_ascii_alphabetic()
        && bytes[2] == b')'
        && bytes[3].is_ascii_whitespace()
}

pub(super) fn lm2_parenthesized_numeric_lead(text: &str) -> bool {
    let trimmed = text.trim_start();
    let Some(remainder) = trimmed.strip_prefix('(') else {
        return false;
    };
    let digits = remainder
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .count();
    digits > 0 && remainder[digits..].starts_with(") ")
}

/// Restore a short final body row that a geometric segmenter isolated as table
/// noise.  This is deliberately narrower than a general Noise rescue: the row
/// must be the immediate continuation of an open body line, use the same left
/// edge and font, be physically outside the note band, and end the sentence.
/// A common instance is a final date row (`October 7, 2025.`) immediately above
/// the page's real footnotes.
pub(super) fn apply_isolated_body_noise_tail_recovery(
    decoded: &mut [(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    let mut repaired = Vec::new();
    for index in 1..decoded.len() {
        let (previous_line, previous_action) = &decoded[index - 1];
        let (line, action) = &decoded[index];
        if *previous_action != Lm2Action::Keep
            || *action != Lm2Action::HideNoise
            || previous_line.page_index != line.page_index
            || previous_line.line_index.checked_add(1) != Some(line.line_index)
            || line.segment_block_line_count != 1
            || line.in_footnote_zone
            || line.below_footnote_divider
            || line.doc_footnote_state
            || line.doc_footnote_continuation
            || line.doc_repeated_edge_text
            || line.doc_repeated_top_edge
            || line.doc_repeated_bottom_edge
            || line.page_object_overlaps_image_bbox
            || line.in_ruled_cell
            || line.ruled_row_membership_exact
            || line.page_object_path_stroke_near_line_count > 0
            || lm2_numbered_table_figure_caption(&line.text)
            || leading_explicit_numbered_note_marker(&line.text).is_some()
            || note_head_marker(&line.text).is_some()
            || looks_like_running_header(&normalize_text(&line.text))
            || looks_like_small_font_page_furniture(&normalize_text(&line.text))
            || lm2_blocksplit_ends_like_paragraph(&previous_line.text)
            || !lm2_blocksplit_ends_like_paragraph(&line.text)
            || !(2..=12).contains(&word_count(&line.text))
        {
            continue;
        }
        let page_width = line.page_width.max(previous_line.page_width).max(1.0);
        let same_left = (line.left - previous_line.left).abs() / page_width <= 0.012;
        let font_ratio = if previous_line.font_height > 0.0 {
            line.font_height / previous_line.font_height
        } else {
            0.0
        };
        let body_sized = (0.96..=1.04).contains(&font_ratio)
            && line.font_ratio_page_ref >= 0.94
            && line.font_ratio_page >= 0.94
            && line.font_ratio_doc >= 0.94;
        if same_left && body_sized {
            repaired.push(index);
        }
    }
    for index in repaired.iter().copied() {
        decoded[index].1 = Lm2Action::Keep;
        decoded[index].0.role_hint = Some(LiquidBlockRole::Paragraph);
    }
    repaired.len()
}

/// Preserve numbered table/figure bands as structured non-body content.  The
/// caption itself is protected independently of band detection so a running
/// header cannot swallow bare `Table 1` / `Figure 1` labels.  For tables, use
/// source geometry to stop at either a real numbered note head or the familiar
/// first-line-indent/body-margin return after the final row.  This also ignores
/// a table rule that PDF extraction mislabeled as a footnote divider.
pub(super) fn apply_numbered_table_figure_band_role_hints(
    decoded: &mut [(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    let mut by_page = BTreeMap::<usize, Vec<usize>>::new();
    for (decoded_index, (line, _)) in decoded.iter().enumerate() {
        by_page
            .entry(line.page_index)
            .or_default()
            .push(decoded_index);
    }
    for indices in by_page.values_mut() {
        indices.sort_by_key(|index| decoded[*index].0.line_index);
    }

    let mut forced_roles = BTreeMap::<usize, LiquidBlockRole>::new();
    let mut continuation_pages = BTreeSet::<usize>::new();

    for (page_index, indices) in &by_page {
        let captions = indices
            .iter()
            .enumerate()
            .filter(|(_, index)| lm2_display_table_figure_caption(&decoded[**index].0))
            .map(|(position, _)| position)
            .collect::<Vec<_>>();
        for caption_position in captions {
            let caption_index = indices[caption_position];
            let caption = &decoded[caption_index].0;
            forced_roles.insert(caption_index, LiquidBlockRole::Caption);

            let mut content_start = caption_position + 1;
            if word_count(&caption.text) <= 3
                && let Some(next_index) = indices.get(content_start).copied()
                && lm2_table_figure_subtitle(&decoded[next_index].0)
            {
                forced_roles.insert(next_index, LiquidBlockRole::Caption);
                content_start += 1;
            }

            if lm2_display_table_caption(caption) {
                let (end_position, stopped_at_note) =
                    lm2_numbered_table_band_end(decoded, indices, content_start);
                for index in indices
                    .iter()
                    .take(end_position)
                    .skip(content_start)
                    .copied()
                {
                    forced_roles.insert(index, LiquidBlockRole::Table);
                }
                if stopped_at_note && end_position > content_start {
                    continuation_pages.insert(page_index.saturating_add(1));
                }
                continue;
            }

            // Preserve the existing false-divider figure recovery for vector
            // labels. Raster figures often have no source lines in the image;
            // in that case the caption/subtitle above are still retained.
            let divider = indices
                .iter()
                .enumerate()
                .skip(content_start)
                .find(|(_, index)| decoded[**index].0.below_footnote_divider);
            let Some((divider_position, divider_index)) = divider else {
                continue;
            };
            let first_below = &decoded[*divider_index].0;
            let first_below_is_body = leading_explicit_numbered_note_marker(&first_below.text)
                .is_none()
                && note_head_marker(&first_below.text).is_none()
                && word_count(&first_below.text) >= 6
                && (first_below.font_ratio_page_ref >= 0.94
                    || first_below.font_ratio_page >= 0.94
                    || first_below.font_ratio_doc >= 0.94);
            if first_below_is_body {
                for index in indices
                    .iter()
                    .take(divider_position)
                    .skip(content_start)
                    .copied()
                {
                    forced_roles.insert(index, LiquidBlockRole::Table);
                }
            }
        }
    }

    // A table can continue on the next page after page-specific notes.  Skip
    // only the leading repeated header/folio, then require several independent
    // table-geometry votes near the page top before carrying the band forward.
    let mut pending = continuation_pages.into_iter().collect::<Vec<_>>();
    while let Some(page_index) = pending.pop() {
        let Some(indices) = by_page.get(&page_index) else {
            continue;
        };
        if indices
            .iter()
            .any(|index| lm2_display_table_figure_caption(&decoded[*index].0))
        {
            continue;
        }
        let content_start = indices
            .iter()
            .position(|index| !lm2_table_continuation_furniture(&decoded[*index].0))
            .unwrap_or(indices.len());
        if content_start == indices.len() {
            continue;
        }
        let geometry_votes = indices
            .iter()
            .skip(content_start)
            .take(10)
            .filter(|index| lm2_table_geometry_vote(&decoded[**index].0))
            .count();
        if geometry_votes < 2 {
            continue;
        }
        let (end_position, stopped_at_note) =
            lm2_numbered_table_band_end(decoded, indices, content_start);
        for index in indices
            .iter()
            .take(end_position)
            .skip(content_start)
            .copied()
        {
            forced_roles.insert(index, LiquidBlockRole::Table);
        }
        if stopped_at_note && end_position > content_start {
            pending.push(page_index.saturating_add(1));
        }
    }

    let mut changed = 0usize;
    for (index, role) in forced_roles {
        if decoded[index].1 != Lm2Action::HideNoise || decoded[index].0.role_hint != Some(role) {
            changed += 1;
        }
        decoded[index].1 = Lm2Action::HideNoise;
        decoded[index].0.role_hint = Some(role);
    }
    changed
}

pub(super) fn lm2_display_table_figure_caption(line: &DeepLiquidSourceLine) -> bool {
    if line.in_footnote_zone
        || line.doc_footnote_state
        || line.doc_footnote_continuation
        || !lm2_numbered_table_figure_caption(&line.text)
    {
        return false;
    }
    let mut alphabetic = line.text.chars().filter(|ch| ch.is_alphabetic());
    let alphabetic_count = alphabetic.clone().count();
    let display_caps = alphabetic_count >= 4 && alphabetic.all(|ch| ch.is_uppercase());
    word_count(&line.text) <= 3
        || (word_count(&line.text) <= 16 && (line.centered || line.margin_centered || display_caps))
}

pub(super) fn lm2_display_table_caption(line: &DeepLiquidSourceLine) -> bool {
    lm2_display_table_figure_caption(line)
        && normalize_text(&line.text)
            .split_whitespace()
            .next()
            .is_some_and(|word| word == "table")
}

pub(super) fn lm2_table_figure_subtitle(line: &DeepLiquidSourceLine) -> bool {
    let text = clean_lm2_line_text(&line.text);
    !text.is_empty()
        && word_count(&text) <= 12
        && (line.centered || line.margin_centered)
        && leading_explicit_numbered_note_marker(&text).is_none()
}

pub(super) fn lm2_numbered_table_band_end(
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
    indices: &[usize],
    content_start: usize,
) -> (usize, bool) {
    for position in content_start..indices.len() {
        let line = &decoded[indices[position]].0;
        if leading_explicit_numbered_note_marker(&line.text).is_some()
            || note_head_marker(&line.text).is_some()
        {
            return (position, true);
        }
        if position + 1 < indices.len()
            && lm2_numbered_table_body_start(line, &decoded[indices[position + 1]].0)
        {
            return (position, false);
        }
    }
    (indices.len(), false)
}

pub(super) fn lm2_numbered_table_body_start(
    line: &DeepLiquidSourceLine,
    next: &DeepLiquidSourceLine,
) -> bool {
    if line.page_index != next.page_index
        || line.line_index.checked_add(1) != Some(next.line_index)
        || word_count(&line.text) < 6
        || word_count(&next.text) < 4
        || tfr_width_norm(line) < 0.62
        || tfr_width_norm(next) < 0.62
        || next.page_object_path_stroke_near_line_count > 0
        || next.segment_block_table_like
        || next.page_table_column_like
        || next.in_ruled_cell
        || next.ruled_row_membership_exact
    {
        return false;
    }
    let page_width = line.page_width.max(next.page_width).max(1.0);
    let first_line_indent = (line.left - next.left) / page_width;
    (0.015..=0.080).contains(&first_line_indent)
}

pub(super) fn lm2_table_continuation_furniture(line: &DeepLiquidSourceLine) -> bool {
    line.line_index <= 4
        && (table_figure_router_repeated_furniture_like(line)
            || looks_like_bare_page_number_furniture(line)
            || looks_like_running_header(&normalize_text(&line.text)))
}

pub(super) fn lm2_table_geometry_vote(line: &DeepLiquidSourceLine) -> bool {
    line.page_object_path_stroke_near_line_count > 0
        || line.segment_block_table_like
        || line.page_table_column_like
        || line.in_ruled_cell
        || line.ruled_row_membership_exact
        || line.page_object_ruled_row_membership
}

/// Restore short numbered note heads that the line classifier rejected as
/// noise.  The combination of explicit note punctuation, small note font, and
/// physical footnote-zone membership is much stronger than the learned line
/// label for terse definitions such as `113. Id.`.  Keeping this deliberately
/// narrower than [`leading_numeric_token_marker`] avoids treating reporter
/// citations (`106 VA. L. REV. ...`) as note heads.
pub(super) fn apply_hidden_numbered_note_head_recovery(
    decoded: &mut [(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    let mut repaired = 0usize;
    for (line, action) in decoded.iter_mut() {
        if *action != Lm2Action::HideNoise
            || line.role_hint == Some(LiquidBlockRole::Contents)
            || if line.page_has_footnote_divider {
                !line.below_footnote_divider
            } else {
                !(line.in_footnote_zone || line.doc_footnote_state)
            }
            || (line.font_ratio_page_ref > 0.94
                && line.font_ratio_page > 0.94
                && line.font_ratio_doc > 0.94)
            || line.in_ruled_cell
            || line.page_table_column_like
            // A journal may print several distinct terse definitions such as
            // `5. Id.` at the same bottom-edge position.  The repeated-text
            // detector quite reasonably marks those lines as edge repeats,
            // but an explicit divider plus a leading note number is stronger
            // evidence than that furniture prior.
            || (line.doc_repeated_edge_text && !line.page_has_footnote_divider)
        {
            continue;
        }
        let Some(marker) = leading_explicit_numbered_note_marker(&line.text) else {
            continue;
        };
        *action = Lm2Action::Marginalia;
        line.role_hint = Some(LiquidBlockRole::Footnote);
        line.doc_note_marker = marker;
        repaired += 1;
    }
    repaired
}

pub(super) fn leading_explicit_numbered_note_marker(text: &str) -> Option<u16> {
    let trimmed = text.trim_start();
    let digits_len = trimmed
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .take(3)
        .count();
    if digits_len == 0 {
        return None;
    }
    let marker = trimmed[..digits_len].parse::<u16>().ok()?;
    let remainder = &trimmed[digits_len..];
    let punctuation = remainder.chars().next()?;
    if !matches!(punctuation, '.' | ')' | ']') {
        return None;
    }
    let after = &remainder[punctuation.len_utf8()..];
    if after.chars().next().is_some_and(|ch| !ch.is_whitespace()) {
        return None;
    }
    (1..=LM2_MAX_NOTE_MARKER)
        .contains(&marker)
        .then_some(marker)
}

/// Remove a repository-generated cover sheet when it is unmistakably present.
/// Such sheets duplicate article metadata and otherwise become false body text.
/// The guard requires multiple Digital Commons-style cues on the same opening
/// page; ordinary first pages and isolated `available at` citations are left
/// alone. Some repositories concatenate the final contact sentence with the
/// first title line of the real article, so preserve that trailing title.
pub(super) fn apply_repository_cover_guard(
    decoded: &mut [(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    let cover_pages = repository_cover_pages(decoded);
    if cover_pages.is_empty() {
        return 0;
    }

    let mut changed = 0usize;
    for (line, action) in decoded.iter_mut() {
        if cover_pages.contains(&line.page_index) {
            if *action != Lm2Action::HideNoise || line.role_hint != Some(LiquidBlockRole::Noise) {
                changed += 1;
            }
            *action = Lm2Action::HideNoise;
            line.role_hint = Some(LiquidBlockRole::Noise);
            continue;
        }
        let follows_cover = line
            .page_index
            .checked_sub(1)
            .is_some_and(|page| cover_pages.contains(&page));
        if follows_cover && let Some(title) = title_after_repository_contact_prefix(&line.text) {
            line.text = title;
            *action = Lm2Action::Keep;
            line.role_hint = Some(LiquidBlockRole::Heading);
            changed += 1;
        }
    }
    changed
}

pub(super) fn repository_cover_pages(
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
) -> HashSet<usize> {
    let mut cues_by_page: BTreeMap<usize, HashSet<&'static str>> = BTreeMap::new();
    for (line, _) in decoded {
        if line.page_index > 1 {
            continue;
        }
        let lower = normalize_text(&line.text);
        for (label, cue) in [
            ("works", "follow this and additional works at"),
            ("citation", "recommended citation"),
            ("access", "brought to you for free and open access"),
            ("inclusion", "accepted for inclusion"),
            ("review", "review by an authorized administrator"),
        ] {
            if lower.contains(cue) {
                cues_by_page
                    .entry(line.page_index)
                    .or_default()
                    .insert(label);
            }
        }
    }
    cues_by_page
        .into_iter()
        .filter_map(|(page_index, cues)| (cues.len() >= 2).then_some(page_index))
        .collect()
}

pub(super) fn title_after_repository_contact_prefix(text: &str) -> Option<String> {
    let lower = normalize_text(text);
    if !lower.contains("for more information, please contact") {
        return None;
    }
    let at = text.find('@')?;
    let end_of_email = text[at..]
        .find(char::is_whitespace)
        .map(|offset| at + offset)
        .unwrap_or(text.len());
    let title = collapse_whitespace(
        text.get(end_of_email..)?
            .trim_start_matches(|ch: char| ch.is_whitespace() || matches!(ch, '.' | ':' | ';')),
    );
    (word_count(&title) >= 4 && uppercase_ratio(&title) >= 0.55).then_some(title)
}

/// Some repository-produced PDFs move every footnote into a full-size,
/// full-page section headed `Footnotes` or `Endnotes`. Geometry-based note
/// classifiers correctly see those lines as body-sized, while the standalone
/// note numbers are often classified as noise. Recover this explicit document
/// structure only when the heading is followed by a substantial monotone
/// numeric sequence. This keeps ordinary numbered prose and incidental
/// headings outside the recovery path.
pub(super) fn apply_explicit_endnote_section_guard(
    decoded: &mut [(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    if decoded.is_empty() {
        return 0;
    }

    let mut ordered = (0..decoded.len()).collect::<Vec<_>>();
    ordered.sort_by_key(|index| {
        let line = &decoded[*index].0;
        (line.page_index, line.line_index)
    });

    let Some((heading_position, _)) = ordered.iter().enumerate().find(|(_, index)| {
        matches!(
            normalize_text(&collapse_whitespace(&decoded[**index].0.text)).as_str(),
            "footnotes" | "endnotes"
        )
    }) else {
        return 0;
    };

    let markers = ordered
        .iter()
        .skip(heading_position + 1)
        .filter_map(|index| {
            let line = &decoded[*index].0;
            if looks_like_bare_page_number_furniture(line) {
                return None;
            }
            collapse_whitespace(&line.text).parse::<u16>().ok()
        })
        .filter(|marker| *marker > 0 && *marker <= 999)
        .collect::<Vec<_>>();
    let monotone = markers
        .windows(2)
        .all(|pair| pair[1] > pair[0] && pair[1] <= pair[0].saturating_add(3));
    if markers.len() < 4 || markers[0] > 5 || !monotone {
        return 0;
    }

    let body_indices = &ordered[..heading_position];
    let mut replacements: BTreeMap<usize, Vec<(usize, usize, u16)>> = BTreeMap::new();
    let mut body_position = 0usize;
    let mut byte_position = 0usize;
    let mut all_callouts_recovered = true;
    for marker in &markers {
        let mut found = None;
        while body_position < body_indices.len() {
            let index = body_indices[body_position];
            if let Some((start, end)) =
                attached_ascii_callout_range(&decoded[index].0.text, *marker, byte_position)
            {
                found = Some((index, start, end));
                byte_position = end;
                break;
            }
            body_position += 1;
            byte_position = 0;
        }
        let Some((index, start, end)) = found else {
            all_callouts_recovered = false;
            break;
        };
        replacements
            .entry(index)
            .or_default()
            .push((start, end, *marker));
    }
    if all_callouts_recovered {
        for (index, mut ranges) in replacements {
            ranges.sort_by_key(|(start, _, _)| *start);
            for (start, end, marker) in ranges.into_iter().rev() {
                decoded[index]
                    .0
                    .text
                    .replace_range(start..end, &format!("{CALLOUT_START}{marker}{CALLOUT_END}"));
            }
            decoded[index].1 = Lm2Action::Keep;
            decoded[index].0.role_hint = Some(LiquidBlockRole::Paragraph);
        }
    }

    let heading_index = ordered[heading_position];
    decoded[heading_index].1 = Lm2Action::HideNoise;
    decoded[heading_index].0.role_hint = Some(LiquidBlockRole::Noise);

    let mut recovered = 0usize;
    for index in ordered.into_iter().skip(heading_position + 1) {
        let (line, action) = &mut decoded[index];
        let text = collapse_whitespace(&line.text);
        if text.is_empty() || looks_like_bare_page_number_furniture(line) {
            continue;
        }
        *action = Lm2Action::Marginalia;
        line.role_hint = Some(LiquidBlockRole::Marginalia);
        // Marker-continuity features are intentionally permissive and may have
        // labeled citation continuations such as `55 Fed. Reg. ...` before the
        // explicit section was recognized. Inside this guarded section, only
        // the standalone numeric lines are authoritative note heads.
        line.doc_note_marker = 0;
        if let Ok(marker) = text.parse::<u16>()
            && markers.binary_search(&marker).is_ok()
        {
            line.doc_note_marker = marker;
        }
        recovered += 1;
    }
    recovered
}

pub(super) fn attached_ascii_callout_range(
    text: &str,
    marker: u16,
    from: usize,
) -> Option<(usize, usize)> {
    let digits = marker.to_string();
    let mut cursor = from.min(text.len());
    while let Some(relative) = text.get(cursor..)?.find(&digits) {
        let start = cursor + relative;
        let end = start + digits.len();
        let before = text.get(..start)?.chars().next_back();
        let after = text.get(end..)?.chars().next();
        let attached = before.is_some_and(|ch| {
            !ch.is_whitespace()
                && !ch.is_ascii_digit()
                && !matches!(ch, '§' | '-' | '*' | CALLOUT_START | CALLOUT_END)
        });
        let bounded = after.is_none_or(|ch| !ch.is_ascii_digit());
        if attached && bounded {
            return Some((start, end));
        }
        cursor = end;
    }
    None
}

pub(super) fn apply_footnote_carryover_overlay(decoded: &mut [(DeepLiquidSourceLine, Lm2Action)]) {
    if decoded.is_empty() {
        return;
    }

    let mut pages: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (index, (line, _)) in decoded.iter().enumerate() {
        pages.entry(line.page_index).or_default().push(index);
    }
    for indices in pages.values_mut() {
        indices.sort_by_key(|index| decoded[*index].0.line_index);
    }

    let mut carryover_open = false;
    let mut expected_marker: Option<u16> = None;
    for indices in pages.values() {
        if carryover_open {
            let marker_stop = expected_marker
                .and_then(|marker| {
                    indices
                        .iter()
                        .position(|index| decoded[*index].0.doc_note_marker == marker)
                })
                .unwrap_or(indices.len());
            let dividerless_page = footnote_carryover_dividerless_page(decoded, indices);

            for index in indices.iter().take(marker_stop).copied() {
                let (line, action) = &mut decoded[index];
                if *action != Lm2Action::Keep {
                    continue;
                }
                if footnote_carryover_candidate(line, dividerless_page) {
                    *action = Lm2Action::Marginalia;
                    line.role_hint = Some(LiquidBlockRole::Footnote);
                }
            }
        }

        let state = footnote_carryover_page_state(decoded, indices);
        carryover_open = state.open;
        expected_marker = state.expected_marker;
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct FootnoteCarryoverState {
    pub(super) open: bool,
    pub(super) expected_marker: Option<u16>,
}

pub(super) fn footnote_carryover_page_state(
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
    indices: &[usize],
) -> FootnoteCarryoverState {
    let Some(last_index) = indices
        .iter()
        .copied()
        .rev()
        .find(|index| footnote_carryover_line_is_note(&decoded[*index]))
    else {
        return FootnoteCarryoverState::default();
    };
    let line = &decoded[last_index].0;
    FootnoteCarryoverState {
        open: !footnote_carryover_has_terminal_punctuation(&line.text),
        expected_marker: footnote_carryover_last_marker(decoded, indices)
            .and_then(|marker| marker.checked_add(1))
            .filter(|marker| *marker <= LM2_MAX_NOTE_MARKER),
    }
}

pub(super) fn footnote_carryover_line_is_note(row: &(DeepLiquidSourceLine, Lm2Action)) -> bool {
    row.1 == Lm2Action::Marginalia && !footnote_monotone_reject_line(&row.0)
}

pub(super) fn footnote_carryover_last_marker(
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
    indices: &[usize],
) -> Option<u16> {
    indices
        .iter()
        .copied()
        .rev()
        .filter(|index| decoded[*index].1 == Lm2Action::Marginalia)
        .map(|index| decoded[index].0.doc_note_marker)
        .find(|marker| *marker > 0)
}

pub(super) fn footnote_carryover_has_terminal_punctuation(text: &str) -> bool {
    let collapsed = collapse_whitespace(text);
    let mut chars = collapsed.chars().rev().peekable();
    while chars
        .peek()
        .is_some_and(|ch| matches!(ch, ')' | ']' | '}' | '"' | '\'' | '\u{201D}' | '\u{2019}'))
    {
        chars.next();
    }
    chars
        .next()
        .is_some_and(|ch| matches!(ch, '.' | '?' | '!' | ';' | ':'))
}

pub(super) fn footnote_carryover_dividerless_page(
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
    indices: &[usize],
) -> bool {
    if indices
        .iter()
        .any(|index| decoded[*index].0.page_has_footnote_divider)
    {
        return false;
    }
    let candidates = indices
        .iter()
        .copied()
        .filter(|index| !footnote_monotone_reject_line(&decoded[*index].0))
        .collect::<Vec<_>>();
    if candidates.len() < 2 {
        return false;
    }
    let small = candidates
        .iter()
        .filter(|index| footnote_carryover_footnote_font_line(&decoded[**index].0))
        .count();
    let body = candidates
        .iter()
        .filter(|index| footnote_carryover_body_font_line(&decoded[**index].0))
        .count();
    small * 2 >= candidates.len() && body <= 1
}

pub(super) fn footnote_carryover_candidate(
    line: &DeepLiquidSourceLine,
    dividerless_page: bool,
) -> bool {
    if footnote_monotone_reject_line(line) {
        return false;
    }
    if line
        .role_hint
        .is_some_and(|role| role_action(role) == Lm2Action::HideNoise)
    {
        return false;
    }
    if !footnote_carryover_footnote_font_line(line) {
        return false;
    }

    let text = collapse_whitespace(&line.text);
    let lower = normalize_text(&text);
    let words = word_count(&lower);
    let cue = line.doc_footnote_continuation
        || line.in_footnote_zone
        || line.below_footnote_divider
        || has_legal_note_cue(&lower)
        || d1_runtime_citation_like(&lower);
    let early_page = line.line_index <= 8;
    if !(dividerless_page || cue || early_page) {
        return false;
    }
    if footnote_carryover_body_resume_like(line, words, cue) {
        return false;
    }
    words >= 3 || cue
}

pub(super) fn footnote_carryover_footnote_font_line(line: &DeepLiquidSourceLine) -> bool {
    let font_ratio = footnote_monotone_font_ratio(line);
    font_ratio <= 0.92
        || line.font_ratio_page_ref <= 0.90
        || (line.doc_font_footnote_size > 0.0
            && line.font_height <= line.doc_font_footnote_size + 0.35)
        || line.doc_font_footnote_z.abs() + 0.15 <= line.doc_font_body_z.abs()
}

pub(super) fn footnote_carryover_body_font_line(line: &DeepLiquidSourceLine) -> bool {
    footnote_monotone_font_ratio(line) >= 0.97
        && line.font_ratio_page_ref >= 0.96
        && line.doc_font_body_z.abs() <= line.doc_font_footnote_z.abs() + 0.10
}

pub(super) fn footnote_carryover_body_resume_like(
    line: &DeepLiquidSourceLine,
    words: usize,
    cue: bool,
) -> bool {
    if cue {
        return false;
    }
    let width_norm = (line.right - line.left).max(0.0) / line.page_width.max(1.0);
    footnote_carryover_body_font_line(line) && words >= 6 && width_norm >= 0.45
}

pub(super) fn d1_runtime_has_marginalia_neighbor(
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
    index: usize,
    signed_window: isize,
) -> bool {
    let page_index = decoded[index].0.page_index;
    if signed_window < 0 {
        let window = signed_window.unsigned_abs();
        let start = index.saturating_sub(window);
        return (start..index).any(|neighbor| {
            decoded[neighbor].0.page_index == page_index
                && decoded[neighbor].1 == Lm2Action::Marginalia
        });
    }
    let end = (index + signed_window as usize + 1).min(decoded.len());
    (index + 1..end).any(|neighbor| {
        decoded[neighbor].0.page_index == page_index && decoded[neighbor].1 == Lm2Action::Marginalia
    })
}

pub(super) fn apply_d1_runtime_safe_numeric_note_overlay(
    decoded: &mut [(DeepLiquidSourceLine, Lm2Action)],
) {
    for (line, action) in decoded.iter_mut() {
        if *action != Lm2Action::Keep || !d1_runtime_safe_numeric_note_candidate(line) {
            continue;
        }
        *action = Lm2Action::Marginalia;
        line.role_hint = Some(LiquidBlockRole::Footnote);
    }
}

pub(super) fn d1_runtime_safe_numeric_note_candidate(line: &DeepLiquidSourceLine) -> bool {
    let text = collapse_whitespace(&line.text);
    if line.page_index < 3 || d1_runtime_artifact_like(&text) {
        return false;
    }
    if !d1_runtime_strong_numeric_note_start(&text) {
        return false;
    }
    let lower = normalize_text(&text);
    let small_font = line.font_ratio_doc <= 0.92 || line.font_ratio_page <= 0.92;
    let lower_half = ((line.top + line.bottom) * 0.5) / line.page_height.max(1.0) >= 0.45;
    let short = text.chars().count() <= 65;
    small_font && lower_half && (!short || d1_runtime_citation_like(&lower) || text.contains('§'))
}

pub(super) fn d1_runtime_strong_numeric_note_start(text: &str) -> bool {
    let trimmed = text.trim_start();
    let mut chars = trimmed.chars().peekable();
    let mut digits = 0usize;
    while chars.peek().is_some_and(|ch| ch.is_ascii_digit()) && digits < 4 {
        chars.next();
        digits += 1;
    }
    if digits == 0 || digits > 4 {
        return false;
    }
    if !chars.next().is_some_and(|ch| matches!(ch, '.' | ')')) {
        return false;
    }
    chars.next().is_some_and(|ch| ch.is_whitespace())
}

pub(super) fn d1_runtime_citation_like(lower: &str) -> bool {
    lower.contains("http://")
        || lower.contains("https://")
        || lower.contains("www.")
        || lower.contains("perma.cc")
        || lower.contains("supra")
        || lower.contains("ibid")
        || d1_runtime_contains_token(lower, "id.")
        || d1_runtime_zerospend_citation_cue(lower)
}

pub(super) fn apply_footnote_monotone_overlay(
    decoded: &mut [(DeepLiquidSourceLine, Lm2Action)],
    article_spans: &[ArticleSpan],
) {
    if decoded.is_empty() {
        return;
    }
    if article_spans.len() > 1 {
        let mut start = 0usize;
        while start < decoded.len() {
            let article = article_index_at(
                article_spans,
                decoded[start].0.page_index,
                decoded[start].0.line_index,
            );
            let mut end = start + 1;
            while end < decoded.len()
                && article_index_at(
                    article_spans,
                    decoded[end].0.page_index,
                    decoded[end].0.line_index,
                ) == article
            {
                end += 1;
            }
            apply_footnote_monotone_overlay_unscoped(&mut decoded[start..end]);
            start = end;
        }
        return;
    }
    apply_footnote_monotone_overlay_unscoped(decoded);
}

pub(super) fn apply_footnote_monotone_overlay_unscoped(
    decoded: &mut [(DeepLiquidSourceLine, Lm2Action)],
) {
    if decoded.is_empty() {
        return;
    }

    let expected_markers = footnote_monotone_expected_missing_markers(decoded);
    let body_reference_markers = footnote_body_reference_markers(decoded);
    let anchors = (0..decoded.len())
        .filter(|index| {
            decoded[*index].1 == Lm2Action::Keep
                && footnote_monotone_anchor_candidate(
                    decoded,
                    *index,
                    &expected_markers,
                    &body_reference_markers,
                )
        })
        .collect::<Vec<_>>();
    for index in anchors {
        decoded[index].1 = Lm2Action::Marginalia;
        decoded[index].0.role_hint = Some(LiquidBlockRole::Footnote);
    }

    let mut active_page: Option<usize> = None;
    let mut active = false;
    let mut added_after_anchor = 0usize;
    for index in 0..decoded.len() {
        let page_index = decoded[index].0.page_index;
        if active_page != Some(page_index) {
            active_page = Some(page_index);
            active = false;
            added_after_anchor = 0;
        }

        if decoded[index].1 == Lm2Action::Marginalia && decoded[index].0.doc_note_marker > 0 {
            active = true;
            added_after_anchor = 0;
            continue;
        }

        if decoded[index].1 != Lm2Action::Keep {
            if active && decoded[index].1 == Lm2Action::HideNoise {
                added_after_anchor += 1;
                if added_after_anchor > 1 {
                    active = false;
                    added_after_anchor = 0;
                }
            } else {
                active = false;
                added_after_anchor = 0;
            }
            continue;
        }

        if active
            && added_after_anchor < 4
            && footnote_monotone_continuation_candidate(&decoded[index].0)
        {
            decoded[index].1 = Lm2Action::Marginalia;
            decoded[index].0.role_hint = Some(LiquidBlockRole::Footnote);
            added_after_anchor += 1;
        } else {
            active = false;
            added_after_anchor = 0;
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct OpenFootnoteCarryoverState {
    pub(super) expected_next_marker: Option<u16>,
}

pub(super) fn apply_open_footnote_carryover_overlay(
    decoded: &mut [(DeepLiquidSourceLine, Lm2Action)],
) {
    if decoded.is_empty() {
        return;
    }

    let mut pages: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (index, (line, _)) in decoded.iter().enumerate() {
        pages.entry(line.page_index).or_default().push(index);
    }

    let mut carryover: Option<OpenFootnoteCarryoverState> = None;
    for indices in pages.values() {
        let mut active = carryover;
        let mut fired_on_page = 0usize;

        for &index in indices {
            let marker = decoded[index]
                .0
                .doc_note_marker
                .max(leading_note_marker(&decoded[index].0.text).unwrap_or(0));
            if active
                .and_then(|state| state.expected_next_marker)
                .is_some_and(|expected| marker == expected)
            {
                active = None;
            }
            if active.is_none() {
                continue;
            }
            if open_footnote_carryover_body_resume_candidate(&decoded[index].0) {
                active = None;
                continue;
            }
            if decoded[index].1 == Lm2Action::Keep
                && fired_on_page < 12
                && open_footnote_carryover_candidate(&decoded[index].0)
            {
                decoded[index].1 = Lm2Action::Marginalia;
                decoded[index].0.role_hint = Some(LiquidBlockRole::Footnote);
                fired_on_page += 1;
            }
        }

        carryover = open_footnote_carryover_page_tail_state(decoded, indices);
    }
}

/// A long note can continue at the bottom of the next page immediately above
/// that page's first numbered note. Some PDFs omit a divider above the
/// continuation, so geometric zone detection begins only at the numbered head
/// and the small-font continuation is otherwise spliced into body prose.
/// Recover only a contiguous run of at least two clearly footnote-sized lines
/// directly preceding a recognized note head.
pub(super) fn apply_preceding_small_font_note_continuation_guard(
    decoded: &mut [(DeepLiquidSourceLine, Lm2Action)],
) -> usize {
    let mut pages = BTreeMap::<usize, Vec<usize>>::new();
    for (index, (line, _)) in decoded.iter().enumerate() {
        pages.entry(line.page_index).or_default().push(index);
    }
    for indices in pages.values_mut() {
        indices.sort_by_key(|index| decoded[*index].0.line_index);
    }

    let mut repairs = Vec::<usize>::new();
    for indices in pages.values() {
        for (position, index) in indices.iter().copied().enumerate() {
            let head = &decoded[index].0;
            let marker = numeric_note_head_candidate(&decoded[index]).or_else(|| {
                (head.in_footnote_zone || head.below_footnote_divider)
                    .then(|| leading_numeric_token_marker(&head.text))
                    .flatten()
            });
            if marker.is_none() || position < 2 {
                continue;
            }

            let mut run = Vec::new();
            let mut next_line_index = head.line_index;
            for candidate_index in indices[..position].iter().copied().rev().take(12) {
                let candidate = &decoded[candidate_index].0;
                if candidate.line_index.saturating_add(1) != next_line_index
                    || candidate.in_footnote_zone
                    || candidate.below_footnote_divider
                    || candidate.doc_note_marker > 0
                    || leading_note_marker(&candidate.text).is_some()
                    || candidate.doc_repeated_edge_text
                    || footnote_monotone_font_ratio(candidate) > 0.86
                    || word_count(&candidate.text) < 2
                    || matches!(
                        candidate.role_hint,
                        Some(
                            LiquidBlockRole::Heading
                                | LiquidBlockRole::Subheading
                                | LiquidBlockRole::Title
                                | LiquidBlockRole::Table
                                | LiquidBlockRole::Noise
                        )
                    )
                {
                    break;
                }
                run.push(candidate_index);
                next_line_index = candidate.line_index;
            }
            if run.len() >= 2 {
                repairs.extend(run);
            }
        }
    }

    repairs.sort_unstable();
    repairs.dedup();
    for index in &repairs {
        decoded[*index].1 = Lm2Action::Marginalia;
        decoded[*index].0.role_hint = Some(LiquidBlockRole::Footnote);
        decoded[*index].0.in_footnote_zone = true;
        decoded[*index].0.doc_footnote_continuation = true;
    }
    repairs.len()
}

pub(super) fn open_footnote_carryover_page_tail_state(
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
    indices: &[usize],
) -> Option<OpenFootnoteCarryoverState> {
    for &index in indices.iter().rev() {
        let (line, action) = &decoded[index];
        if !open_footnote_carryover_tail_candidate(line, *action) {
            continue;
        }
        if open_footnote_carryover_has_terminal_punctuation(&line.text) {
            return None;
        }
        let marker = line
            .doc_note_marker
            .max(leading_note_marker(&line.text).unwrap_or(0));
        return Some(OpenFootnoteCarryoverState {
            expected_next_marker: (marker > 0 && marker < 500).then_some(marker + 1),
        });
    }
    None
}

pub(super) fn open_footnote_carryover_tail_candidate(
    line: &DeepLiquidSourceLine,
    action: Lm2Action,
) -> bool {
    if action != Lm2Action::Marginalia || open_footnote_carryover_reject_line(line) {
        return false;
    }
    let text = collapse_whitespace(&line.text);
    let lower = normalize_text(&text);
    let center_y = ((line.top + line.bottom) * 0.5) / line.page_height.max(1.0);
    let small_or_note_font = open_footnote_carryover_footnote_font(line);
    line.below_footnote_divider
        || line.in_footnote_zone
        || line.doc_footnote_state
        || line.doc_footnote_continuation
        || (center_y >= 0.34
            && small_or_note_font
            && (has_legal_note_cue(&lower) || d1_runtime_citation_like(&lower)))
}

pub(super) fn open_footnote_carryover_candidate(line: &DeepLiquidSourceLine) -> bool {
    if open_footnote_carryover_reject_line(line) {
        return false;
    }
    if line
        .role_hint
        .is_some_and(|role| role_action(role) == Lm2Action::HideNoise)
    {
        return false;
    }
    if line
        .role_hint
        .is_some_and(|role| role_action(role) == Lm2Action::Keep)
    {
        return false;
    }
    if line.doc_note_marker > 0 || leading_note_marker(&line.text).is_some() {
        return false;
    }
    let text = collapse_whitespace(&line.text);
    let lower = normalize_text(&text);
    let center_y = ((line.top + line.bottom) * 0.5) / line.page_height.max(1.0);
    let width_norm = (line.right - line.left).max(0.0) / line.page_width.max(1.0);
    let note_font = open_footnote_carryover_footnote_font(line);
    let note_zone = line.below_footnote_divider
        || line.in_footnote_zone
        || line.doc_footnote_continuation
        || line.doc_footnote_state;
    let note_hint = line
        .role_hint
        .is_some_and(|role| role_action(role) == Lm2Action::Marginalia);
    if !note_zone && !note_hint {
        return false;
    }
    let dividerless_continuation = !line.page_has_footnote_divider
        && note_hint
        && note_font
        && center_y >= 0.18
        && word_count(&lower) >= 5
        && uppercase_ratio(&text) < 0.72
        && width_norm >= 0.24;
    let cited_tail = note_hint
        && note_font
        && center_y >= 0.18
        && (has_legal_note_cue(&lower) || d1_runtime_citation_like(&lower));

    note_zone || dividerless_continuation || cited_tail
}

pub(super) fn open_footnote_carryover_body_resume_candidate(line: &DeepLiquidSourceLine) -> bool {
    if line.below_footnote_divider || line.in_footnote_zone {
        return false;
    }
    if open_footnote_carryover_footnote_font(line) {
        return false;
    }
    let text = collapse_whitespace(&line.text);
    let lower = normalize_text(&text);
    let center_y = ((line.top + line.bottom) * 0.5) / line.page_height.max(1.0);
    center_y >= 0.18
        && word_count(&lower) >= 5
        && uppercase_ratio(&text) < 0.72
        && !has_legal_note_cue(&lower)
        && !d1_runtime_citation_like(&lower)
}

pub(super) fn open_footnote_carryover_reject_line(line: &DeepLiquidSourceLine) -> bool {
    let text = collapse_whitespace(&line.text);
    let lower = normalize_text(&text);
    let citation_like = has_legal_note_cue(&lower) || d1_runtime_citation_like(&lower);
    lower.trim().is_empty()
        || looks_like_toc_entry(&lower)
        || lm2_toc_dotleader_line(&text)
        || looks_like_running_header(&lower)
        || looks_like_small_font_page_furniture(&lower)
        || d1_runtime_artifact_like(&text)
        || (d1_runtime_table_stat_like(&text) && !citation_like)
}

pub(super) fn open_footnote_carryover_footnote_font(line: &DeepLiquidSourceLine) -> bool {
    let ratio = footnote_monotone_font_ratio(line);
    ratio <= 0.96
        || (line.doc_font_footnote_size > 0.0
            && line.font_height > 0.0
            && line.font_height <= line.doc_font_footnote_size + 0.45)
        || (line.doc_font_footnote_z != 0.0
            && line.doc_font_body_z != 0.0
            && line.doc_font_footnote_z.abs() + 0.20 < line.doc_font_body_z.abs())
}

pub(super) fn open_footnote_carryover_has_terminal_punctuation(text: &str) -> bool {
    let trimmed = text.trim_end_matches(|ch: char| {
        ch.is_whitespace()
            || matches!(
                ch,
                '"' | '\'' | ')' | ']' | '}' | '\u{2019}' | '\u{201D}' | '\u{00BB}' | '\u{203A}'
            )
    });
    trimmed
        .chars()
        .last()
        .is_some_and(|ch| matches!(ch, '.' | '?' | '!' | ';' | ':'))
}

pub(super) fn footnote_monotone_anchor_candidate(
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
    index: usize,
    expected_markers: &HashSet<u16>,
    body_reference_markers: &HashSet<u16>,
) -> bool {
    let line = &decoded[index].0;
    if line.doc_note_marker == 0 || !d1_runtime_strong_numeric_note_start(&line.text) {
        return false;
    }
    if footnote_monotone_reject_line(line) {
        return false;
    }
    if line
        .role_hint
        .is_some_and(|role| role_action(role) == Lm2Action::HideNoise)
    {
        return false;
    }

    let text = collapse_whitespace(&line.text);
    let lower = normalize_text(&text);
    let font_ratio = footnote_monotone_font_ratio(line);
    let center_y = ((line.top + line.bottom) * 0.5) / line.page_height.max(1.0);
    let small_font = font_ratio <= 0.95;
    let lower_half = center_y >= 0.38;
    let monotone_evidence = line.doc_note_marker_follows_previous_page
        || (line.doc_note_marker_mid_sequence_page
            && (0..=3).contains(&line.doc_note_marker_page_delta))
        || line.doc_note_marker_first_on_page && line.page_index > 0 && line.doc_note_marker > 1;
    let gap_or_body_marker_evidence = expected_markers.contains(&line.doc_note_marker)
        || body_reference_markers.contains(&line.doc_note_marker);
    let local_note_context = line.below_footnote_divider
        || line.page_has_footnote_divider
        || line.doc_footnote_state
        || line.doc_footnote_continuation
        || d1_runtime_nearby_marginalia_count(decoded, index, 6) >= 1;

    (monotone_evidence || gap_or_body_marker_evidence)
        && local_note_context
        && (small_font || line.below_footnote_divider || d1_runtime_citation_like(&lower))
        && (lower_half || line.below_footnote_divider || line.doc_footnote_state)
}

pub(super) fn footnote_monotone_expected_missing_markers(
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
) -> HashSet<u16> {
    let mut known = decoded
        .iter()
        .filter_map(|(line, action)| {
            (*action == Lm2Action::Marginalia
                && line.doc_note_marker > 0
                && !footnote_monotone_reject_line(line))
            .then_some(line.doc_note_marker)
        })
        .collect::<Vec<_>>();
    known.sort_unstable();
    known.dedup();

    let mut expected = HashSet::new();
    for pair in known.windows(2) {
        let left = pair[0];
        let right = pair[1];
        if right <= left || right - left > 4 {
            continue;
        }
        for marker in (left + 1)..right {
            expected.insert(marker);
        }
    }
    expected
}

pub(super) fn footnote_body_reference_markers(
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
) -> HashSet<u16> {
    let mut markers = HashSet::new();
    for (line, action) in decoded {
        if *action != Lm2Action::Keep || !footnote_body_reference_line_candidate(line) {
            continue;
        }
        markers.extend(footnote_reference_markers_in_text(&line.text));
    }
    markers
}

pub(super) fn footnote_body_reference_line_candidate(line: &DeepLiquidSourceLine) -> bool {
    if footnote_monotone_reject_line(line) || line.in_footnote_zone || line.below_footnote_divider {
        return false;
    }
    if line
        .role_hint
        .is_some_and(|role| role_action(role) != Lm2Action::Keep)
    {
        return false;
    }
    let text = collapse_whitespace(&line.text);
    let lower = normalize_text(&text);
    let width_norm = (line.right - line.left).max(0.0) / line.page_width.max(1.0);
    let font_ratio = footnote_monotone_font_ratio(line);
    width_norm >= 0.32
        && (0.86..=1.22).contains(&font_ratio)
        && word_count(&lower) >= 5
        && uppercase_ratio(&text) < 0.72
        && !d1_runtime_table_stat_like(&text)
}

pub(super) fn footnote_reference_markers_in_text(text: &str) -> Vec<u16> {
    let mut markers = Vec::new();
    let mut superscript_value = String::new();
    for ch in text.chars() {
        if let Some(digit) = superscript_digit_value(ch) {
            superscript_value.push(digit);
            continue;
        }
        push_marker_digits(&mut markers, &mut superscript_value);
    }
    push_marker_digits(&mut markers, &mut superscript_value);

    if let Some(marker) = attached_terminal_ascii_marker(text) {
        markers.push(marker);
    }
    markers.sort_unstable();
    markers.dedup();
    markers
}

pub(super) fn push_marker_digits(markers: &mut Vec<u16>, digits: &mut String) {
    if digits.is_empty() {
        return;
    }
    if let Ok(value) = digits.parse::<u16>()
        && (1..=LM2_MAX_NOTE_MARKER).contains(&value)
    {
        markers.push(value);
    }
    digits.clear();
}

pub(super) fn superscript_digit_value(ch: char) -> Option<char> {
    match ch {
        '\u{2070}' => Some('0'),
        '\u{00B9}' => Some('1'),
        '\u{00B2}' => Some('2'),
        '\u{00B3}' => Some('3'),
        '\u{2074}' => Some('4'),
        '\u{2075}' => Some('5'),
        '\u{2076}' => Some('6'),
        '\u{2077}' => Some('7'),
        '\u{2078}' => Some('8'),
        '\u{2079}' => Some('9'),
        _ => None,
    }
}

pub(super) fn attached_terminal_ascii_marker(text: &str) -> Option<u16> {
    let token = text.split_whitespace().last()?.trim_matches(|ch: char| {
        matches!(ch, ')' | ']' | '}' | '"' | '\'' | '\u{201D}' | '\u{2019}')
    });
    if token.chars().count() < 3 {
        return None;
    }
    let mut digits = String::new();
    for ch in token.chars().rev() {
        if ch.is_ascii_digit() && digits.len() < 3 {
            digits.insert(0, ch);
        } else {
            break;
        }
    }
    if digits.is_empty() || digits.len() == token.chars().count() {
        return None;
    }
    let prefix = &token[..token.len() - digits.len()];
    if !prefix.chars().any(|ch| ch.is_alphabetic()) {
        return None;
    }
    if prefix
        .chars()
        .last()
        .is_some_and(|ch| ch.is_ascii_digit() || ch == '-')
    {
        return None;
    }
    let value = digits.parse::<u16>().ok()?;
    (1..=LM2_MAX_NOTE_MARKER).contains(&value).then_some(value)
}

/// Like `attached_terminal_ascii_marker`, but also returns the byte range of
/// the marker digits inside `text`.
///
/// Callers that rewrite the line must slice on this range rather than assume
/// the digits sit at the very end of the string: the parser ignores closing
/// punctuation such as `)` or a curly quote, and a curly quote is three bytes
/// wide, so `text.len() - digits.len()` lands inside it and panics.
pub(super) fn attached_terminal_ascii_marker_span(
    text: &str,
) -> Option<(u16, std::ops::Range<usize>)> {
    let marker = attached_terminal_ascii_marker(text)?;
    let trimmed_end = text.trim_end();
    let token_start = trimmed_end.rfind(char::is_whitespace).map_or(0, |index| {
        index
            + trimmed_end[index..]
                .chars()
                .next()
                .map_or(1, char::len_utf8)
    });
    let token = &trimmed_end[token_start..];
    let core_len = token
        .trim_end_matches(|ch: char| {
            matches!(ch, ')' | ']' | '}' | '"' | '\'' | '\u{201D}' | '\u{2019}')
        })
        .len();
    let digit_len = token[..core_len]
        .chars()
        .rev()
        .take_while(|ch| ch.is_ascii_digit())
        .take(3)
        .count();
    let start = token_start + core_len - digit_len;
    let end = token_start + core_len;
    if digit_len == 0 || text.get(start..end).is_none() {
        return None;
    }
    Some((marker, start..end))
}

/// Rewrite a body line whose terminal note number was matched by
/// `attached_terminal_ascii_marker_span`, keeping any closing punctuation and
/// trailing whitespace that followed the digits.
pub(super) fn rewrite_terminal_marker_as_callout(
    text: &str,
    marker: u16,
    span: &std::ops::Range<usize>,
) -> Option<String> {
    let prefix = text.get(..span.start)?;
    let suffix = text.get(span.end..)?;
    Some(format!(
        "{prefix}{CALLOUT_START}{marker}{CALLOUT_END}{suffix}"
    ))
}

pub(super) fn attached_terminal_body_marker(text: &str) -> Option<u16> {
    if let Some(marker) = attached_terminal_ascii_marker(text) {
        return Some(marker);
    }
    let trimmed = text.trim_end();
    if !trimmed.ends_with(CALLOUT_END) {
        return None;
    }
    let start = trimmed.rfind(CALLOUT_START)?;
    let prefix = trimmed[..start].trim_end();
    if !prefix.chars().any(char::is_alphabetic) {
        return None;
    }
    let digits = &trimmed[start + CALLOUT_START.len_utf8()..trimmed.len() - CALLOUT_END.len_utf8()];
    let marker = digits.parse::<u16>().ok()?;
    (1..=LM2_MAX_NOTE_MARKER)
        .contains(&marker)
        .then_some(marker)
}

pub(super) fn footnote_monotone_continuation_candidate(line: &DeepLiquidSourceLine) -> bool {
    if footnote_monotone_reject_line(line) {
        return false;
    }
    if line
        .role_hint
        .is_some_and(|role| role_action(role) == Lm2Action::HideNoise)
    {
        return false;
    }
    let text = collapse_whitespace(&line.text);
    let lower = normalize_text(&text);
    let center_y = ((line.top + line.bottom) * 0.5) / line.page_height.max(1.0);
    let small_font = footnote_monotone_font_ratio(line) <= 0.96;
    let cued_continuation = line.doc_footnote_continuation
        || line.below_footnote_divider
        || has_legal_note_cue(&lower)
        || d1_runtime_citation_like(&lower);
    if !cued_continuation && center_y > 0.40 {
        return false;
    }
    let continuation_signal = line.doc_footnote_continuation
        || line.below_footnote_divider
        || has_legal_note_cue(&lower)
        || d1_runtime_citation_like(&lower)
        || (small_font && word_count(&lower) >= 5 && uppercase_ratio(&text) < 0.62);
    small_font && center_y >= 0.34 && continuation_signal
}

pub(super) fn footnote_monotone_reject_line(line: &DeepLiquidSourceLine) -> bool {
    let text = collapse_whitespace(&line.text);
    let lower = normalize_text(&text);
    lower.trim().is_empty()
        || looks_like_toc_entry(&lower)
        || lm2_toc_dotleader_line(&text)
        || looks_like_running_header(&lower)
        || looks_like_small_font_page_furniture(&lower)
        || d1_runtime_artifact_like(&text)
        || d1_runtime_table_stat_like(&text)
}

pub(super) fn footnote_monotone_font_ratio(line: &DeepLiquidSourceLine) -> f32 {
    if line.font_ratio_doc > 0.0 {
        line.font_ratio_doc
    } else {
        line.font_ratio_page
    }
}

pub(super) fn d1_runtime_immediate_continuation_candidate(line: &DeepLiquidSourceLine) -> bool {
    let text = collapse_whitespace(&line.text);
    let font_ratio = if line.font_ratio_doc > 0.0 {
        line.font_ratio_doc
    } else {
        line.font_ratio_page
    };
    font_ratio <= 0.92
        && !d1_runtime_artifact_like(&text)
        && !text.contains("....")
        && !d1_runtime_table_stat_like(&text)
}

pub(super) fn d1_runtime_artifact_like(text: &str) -> bool {
    text.contains('\u{0002}') || text.trim_start().starts_with("** ")
}

pub(super) fn d1_runtime_table_stat_like(text: &str) -> bool {
    let digit_count = text.chars().filter(|ch| ch.is_ascii_digit()).count();
    let alpha_count = text.chars().filter(|ch| ch.is_alphabetic()).count();
    let punct_count = text
        .chars()
        .filter(|ch| !ch.is_alphanumeric() && !ch.is_whitespace())
        .count();
    text.len() <= 90
        && digit_count >= 4
        && (punct_count >= 3 || text.contains('%'))
        && alpha_count <= 40
}

pub(super) fn apply_page_label_furniture_guard(decoded: &mut [(DeepLiquidSourceLine, Lm2Action)]) {
    for (line, action) in decoded.iter_mut() {
        if !looks_like_page_label_furniture(&line.text)
            && !looks_like_bare_page_number_furniture(line)
        {
            continue;
        }
        *action = Lm2Action::HideNoise;
        line.role_hint = Some(LiquidBlockRole::Noise);
    }
}

pub(super) fn looks_like_bare_page_number_furniture(line: &DeepLiquidSourceLine) -> bool {
    if !line.centered {
        return false;
    }
    let hinted_non_body = line
        .role_hint
        .is_some_and(|role| role_action(role) != Lm2Action::Keep);
    if !hinted_non_body {
        return false;
    }
    let text = collapse_whitespace(&line.text);
    let stripped = text
        .trim()
        .trim_start_matches('(')
        .trim_end_matches(')')
        .trim();
    !stripped.is_empty() && stripped.len() <= 4 && stripped.chars().all(|ch| ch.is_ascii_digit())
}

pub(super) fn looks_like_page_label_furniture(text: &str) -> bool {
    let mut words = text.split_whitespace();
    let Some(page) = words.next() else {
        return false;
    };
    if !page.eq_ignore_ascii_case("page") {
        return false;
    }
    let Some(number) = words.next() else {
        return false;
    };
    if !number.chars().all(|ch| ch.is_ascii_digit()) {
        return false;
    }
    match (words.next(), words.next(), words.next()) {
        (None, None, None) => true,
        (Some(of), Some(total), None) => {
            of.eq_ignore_ascii_case("of") && total.chars().all(|ch| ch.is_ascii_digit())
        }
        _ => false,
    }
}

pub(super) fn noise_hint_page_furniture(line: &DeepLiquidSourceLine) -> bool {
    let has_noise_hint = line
        .role_hint
        .is_some_and(|role| role_action(role) == Lm2Action::HideNoise);
    let lower = normalize_text(&line.text);
    if lower.trim().is_empty() {
        return false;
    }
    let y_top = line.top / line.page_height.max(1.0);
    let y_bottom = line.bottom / line.page_height.max(1.0);
    let strong_edge_furniture = looks_like_small_font_page_furniture(&lower)
        && (y_top > 0.90 || y_bottom < 0.12 || line.font_ratio_page < 0.82);
    let first_page_masthead = line.page_index == 0 && first_page_journal_masthead(line, &lower);
    strong_edge_furniture
        || first_page_masthead
        || has_noise_hint
            && (looks_like_small_font_page_furniture(&lower)
                || looks_like_running_header(&lower)
                || looks_like_production_slug_boilerplate(&line.text)
                || y_top > 0.93 && line.font_ratio_page < 0.82
                || y_bottom < 0.08 && line.font_ratio_page < 0.82
                || line.page_index == 0
                    && y_top > 0.78
                    && (line.centered || uppercase_ratio(&line.text) >= 0.70)
                || line.page_index == 0
                    && y_bottom < 0.18
                    && (looks_like_small_font_page_furniture(&lower)
                        || line.centered
                        || lower.contains("law archive")))
}

pub(super) fn first_page_journal_masthead(line: &DeepLiquidSourceLine, lower: &str) -> bool {
    let y_top = line.top / line.page_height.max(1.0);
    if y_top < 0.75 || word_count(lower) > 7 {
        return false;
    }
    (lower.contains("law review") || lower.contains("law journal"))
        && (line.centered || uppercase_ratio(&line.text) >= 0.70)
}

pub(super) fn first_page_author_note_line(line: &DeepLiquidSourceLine) -> bool {
    if !looks_like_lm2_author_heading(&line.text) {
        return false;
    }
    let lower = normalize_text(&line.text);
    let words = word_count(&lower);
    let y_top = line.top / line.page_height.max(1.0);
    words >= 2
        && words <= 10
        && y_top > 0.45
        && !line.below_footnote_divider
        && !looks_like_note_start(&line.text)
        && !has_legal_note_cue(&lower)
}

pub(super) fn lm2_toc_dotleader_line(text: &str) -> bool {
    let mut run = 0usize;
    for ch in text.chars() {
        if ch == '.' {
            run += 1;
            if run >= 4 {
                return true;
            }
        } else {
            run = 0;
        }
    }
    false
}

pub(super) fn lm2_toc_dotleader_fallback(line: &DeepLiquidSourceLine) -> bool {
    if line.page_index > 2 || !lm2_toc_has_long_dotleader(&line.text) {
        return false;
    }
    let trimmed = line.text.trim();
    if !lm2_toc_trails_page_number_or_roman(trimmed) {
        return false;
    }
    let mut letters = 0usize;
    let mut uppercase = 0usize;
    for ch in trimmed.chars().filter(|ch| ch.is_alphabetic()) {
        letters += 1;
        if ch.is_uppercase() {
            uppercase += 1;
        }
    }
    letters > 0 && uppercase as f64 / letters as f64 >= 0.45
}

pub(super) fn lm2_toc_has_long_dotleader(text: &str) -> bool {
    let mut run = 0usize;
    for ch in text.chars() {
        if ch == '.' {
            run += 1;
            if run >= 8 {
                return true;
            }
        } else {
            run = 0;
        }
    }
    false
}

pub(super) fn lm2_toc_trails_page_number_or_roman(text: &str) -> bool {
    let tail = text
        .split_whitespace()
        .last()
        .unwrap_or_default()
        .trim_matches(|ch: char| matches!(ch, '.' | ',' | ';' | ':' | ')' | ']'));
    if tail.is_empty() {
        return true;
    }
    tail.chars().all(|ch| ch.is_ascii_digit())
        || tail.chars().all(|ch| {
            matches!(
                ch.to_ascii_lowercase(),
                'i' | 'v' | 'x' | 'l' | 'c' | 'd' | 'm'
            )
        })
}

pub(super) fn lm2_toc_overlay_repeated_edge(line: &DeepLiquidSourceLine) -> bool {
    line.doc_repeated_top_edge || line.doc_repeated_bottom_edge || line.doc_repeated_edge_text
}

pub(super) fn lm2_toc_normalize(text: &str, is_dotleader: bool) -> String {
    let mut value = if is_dotleader {
        lm2_toc_strip_trailing_dots_and_page(text)
    } else {
        text.to_owned()
    };
    value = lm2_toc_strip_enumerator(&value);
    value = lm2_toc_strip_enumerator(&value);
    collapse_whitespace(&value.to_lowercase()).trim().to_owned()
}

pub(super) fn lm2_toc_strip_trailing_dots_and_page(text: &str) -> String {
    let mut chars = text.trim_end().chars().collect::<Vec<_>>();
    while chars.last().is_some_and(|ch| ch.is_whitespace()) {
        chars.pop();
    }
    while chars.last().is_some_and(|ch| {
        ch.is_ascii_digit()
            || matches!(
                ch.to_ascii_lowercase(),
                'i' | 'v' | 'x' | 'l' | 'c' | 'd' | 'm'
            )
    }) {
        chars.pop();
    }
    while chars.last().is_some_and(|ch| ch.is_whitespace()) {
        chars.pop();
    }
    while chars.last().is_some_and(|ch| *ch == '.') {
        chars.pop();
    }
    chars.into_iter().collect::<String>().trim_end().to_owned()
}

pub(super) fn lm2_toc_strip_enumerator(text: &str) -> String {
    let trimmed = text.trim_start();
    let chars = trimmed.chars().collect::<Vec<_>>();
    let mut index = 0usize;
    while index < chars.len()
        && (chars[index].is_ascii_digit()
            || matches!(
                chars[index].to_ascii_lowercase(),
                'i' | 'v' | 'x' | 'l' | 'c' | 'd' | 'm'
            ))
    {
        index += 1;
    }
    if index == 0 || index >= chars.len() || !matches!(chars[index], '.' | ')' | ']') {
        return trimmed.to_owned();
    }
    index += 1;
    while index < chars.len() && chars[index].is_whitespace() {
        index += 1;
    }
    chars[index..].iter().collect()
}
