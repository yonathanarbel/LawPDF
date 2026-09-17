//! Document-level feature enrichment and evaluation reports.

use super::*;

pub(super) fn annotate_pp_prior(runtime: &Lm2Runtime, path: &str, line: &mut DeepLiquidSourceLine) {
    let Some(index) = runtime.pp_priors.as_ref() else {
        return;
    };
    let Some(prior) = index.rows.get(&pp_prior_key(
        path,
        line.page_index,
        line.line_index,
        &line.text,
    )) else {
        return;
    };
    line.pp_prior_role = Some(prior.role.clone());
    line.pp_prior_label = Some(prior.label.clone());
    line.pp_prior_score = Some(prior.score);
}

#[cfg(any(feature = "devtools", test))]
pub(super) fn eval_sources_for_rows(
    rows: Vec<Lm2EvalRow>,
    use_example_role_hints: bool,
) -> Vec<(Lm2EvalRow, DeepLiquidSourceLine)> {
    let mut entries = rows
        .into_iter()
        .map(|row| {
            let source = eval_source_line(&row, use_example_role_hints);
            (row, source)
        })
        .collect::<Vec<_>>();
    let mut doc_indices: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (index, (row, _)) in entries.iter().enumerate() {
        doc_indices
            .entry(row.path.trim().to_ascii_lowercase())
            .or_default()
            .push(index);
    }
    for indices in doc_indices.values() {
        let mut doc_lines = indices
            .iter()
            .map(|index| entries[*index].1.clone())
            .collect::<Vec<_>>();
        enrich_lm2_document_features(&mut doc_lines);
        for (index, source) in indices.iter().zip(doc_lines) {
            entries[*index].1 = source;
        }
    }
    entries
}

pub(super) fn enrich_lm2_document_features(lines: &mut [DeepLiquidSourceLine]) {
    enrich_lm2_tabular_position_features(lines);
    enrich_lm2_tabular_margin_features(lines);
    enrich_lm2_tabular_body_layout_features(lines);
    enrich_lm2_geometric_footnote_zone_features(lines);
    enrich_lm2_doc_font_features(lines);
    enrich_lm2_footnote_state_features(lines);
    enrich_lm2_repetition_features(lines);
    enrich_lm2_dotleader_context_features(lines);
    enrich_lm2_vertical_axis_features(lines);
    enrich_lm2_marker_continuity_features(lines);
}

pub(super) fn enrich_lm2_tabular_position_features(lines: &mut [DeepLiquidSourceLine]) {
    let page_count = lines
        .iter()
        .map(|line| line.page_index)
        .max()
        .map(|page| page + 1)
        .unwrap_or(1)
        .max(1);
    let mut indices = (0..lines.len()).collect::<Vec<_>>();
    indices.sort_by(|left, right| {
        let lhs = &lines[*left];
        let rhs = &lines[*right];
        lhs.page_index
            .cmp(&rhs.page_index)
            .then_with(|| lhs.line_index.cmp(&rhs.line_index))
            .then_with(|| lhs.bottom.total_cmp(&rhs.bottom))
            .then_with(|| lhs.left.total_cmp(&rhs.left))
    });
    for (position, index) in indices.into_iter().enumerate() {
        let line = &mut lines[index];
        line.page_index_norm = if page_count <= 1 {
            0.0
        } else {
            (line.page_index as f32 / (page_count - 1) as f32).clamp(0.0, 1.0)
        };
        line.lines_from_doc_start = position;
        line.front_matter_zone = line.page_index <= 1 && position < 80;
    }
}

pub(super) fn enrich_lm2_tabular_margin_features(lines: &mut [DeepLiquidSourceLine]) {
    for line in lines.iter_mut() {
        line.left_margin_ratio = 0.0;
        line.right_margin_ratio = 0.0;
        line.indent_both = 0.0;
        line.margin_symmetry = 1.0;
        line.line_width_ratio = 0.0;
        line.margin_centered = false;
    }

    let mut pages: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (index, line) in lines.iter().enumerate() {
        pages.entry(line.page_index).or_default().push(index);
    }
    for indices in pages.values() {
        if indices.is_empty() {
            continue;
        }
        let mut x0s = indices
            .iter()
            .map(|index| lines[*index].left)
            .collect::<Vec<_>>();
        let mut x1s = indices
            .iter()
            .map(|index| lines[*index].right)
            .collect::<Vec<_>>();
        x0s.sort_by(f32::total_cmp);
        x1s.sort_by(f32::total_cmp);
        let last = x0s.len().saturating_sub(1);
        let lcol = x0s[((0.05 * last as f32) as usize).min(last)];
        let rcol = x1s[((0.95 * last as f32) as usize).min(last)];
        let colw = (rcol - lcol).max(1.0);
        for index in indices {
            let line = &mut lines[*index];
            let left = ((line.left - lcol) / colw).max(0.0);
            let right = ((rcol - line.right) / colw).max(0.0);
            let indent_both = left.min(right);
            let symmetry = (1.0 - (left - right).abs()).max(0.0);
            let width = ((line.right - line.left) / colw).max(0.0);
            line.left_margin_ratio = round5_f32(left);
            line.right_margin_ratio = round5_f32(right);
            line.indent_both = round5_f32(indent_both);
            line.margin_symmetry = round5_f32(symmetry);
            line.line_width_ratio = round5_f32(width);
            line.margin_centered = indent_both > 0.12 && symmetry > 0.7 && width < 0.75;
        }
    }
}

pub(super) fn enrich_lm2_tabular_body_layout_features(lines: &mut [DeepLiquidSourceLine]) {
    for line in lines.iter_mut() {
        line.indent_vs_body = 0.0;
        line.width_vs_body = 1.0;
        line.is_block_indented = false;
        line.prev_line_indented = false;
    }
    if lines.is_empty() {
        return;
    }
    let mut ref_indices = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| {
            let width = (line.right - line.left) / line.page_width.max(1.0);
            width > 0.5 && (0.9..=1.15).contains(&line.font_ratio_doc)
        })
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if ref_indices.is_empty() {
        ref_indices = (0..lines.len()).collect();
    }
    let mut lefts = ref_indices
        .iter()
        .map(|index| lines[*index].left / lines[*index].page_width.max(1.0))
        .collect::<Vec<_>>();
    let mut widths = ref_indices
        .iter()
        .map(|index| {
            (lines[*index].right - lines[*index].left).max(0.0) / lines[*index].page_width.max(1.0)
        })
        .collect::<Vec<_>>();
    lefts.sort_by(f32::total_cmp);
    widths.sort_by(f32::total_cmp);
    let body_left = median_sorted(&lefts).unwrap_or(0.0);
    let body_width = median_sorted(&widths).unwrap_or(1.0).max(1e-6);

    let mut indices = (0..lines.len()).collect::<Vec<_>>();
    indices.sort_by(|left, right| {
        let lhs = &lines[*left];
        let rhs = &lines[*right];
        lhs.page_index
            .cmp(&rhs.page_index)
            .then_with(|| lhs.line_index.cmp(&rhs.line_index))
    });
    let mut previous_indented = false;
    for index in indices {
        let line_width =
            (lines[index].right - lines[index].left).max(0.0) / lines[index].page_width.max(1.0);
        let indent = ((lines[index].left / lines[index].page_width.max(1.0)) - body_left).max(0.0);
        let width_vs_body = (line_width / body_width).min(2.0);
        let block_indented = indent > 0.015 && indent < 0.22 && width_vs_body < 0.97;
        lines[index].indent_vs_body = round5_f32(indent);
        lines[index].width_vs_body = round5_f32(width_vs_body);
        lines[index].is_block_indented = block_indented;
        lines[index].prev_line_indented = previous_indented;
        previous_indented = block_indented;
    }
}

pub(super) fn median_sorted(values: &[f32]) -> Option<f32> {
    if values.is_empty() {
        return None;
    }
    let mid = values.len() / 2;
    if values.len() % 2 == 1 {
        Some(values[mid])
    } else {
        Some((values[mid - 1] + values[mid]) * 0.5)
    }
}

pub(super) fn round5_f32(value: f32) -> f32 {
    (value * 100_000.0).round() / 100_000.0
}

pub(super) fn enrich_lm2_dotleader_context_features(lines: &mut [DeepLiquidSourceLine]) {
    for line in lines.iter_mut() {
        line.prev_line_has_dotleader = false;
        line.prev4_dotleader_count = 0;
        line.prev4_spaced_dotleader_count = 0;
        line.prev4_strong_dotleader_count = 0;
        line.prev4_toc_leader_context = false;
    }

    let mut pages: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (index, line) in lines.iter().enumerate() {
        pages.entry(line.page_index).or_default().push(index);
    }

    for indices in pages.values_mut() {
        indices.sort_by(|left, right| {
            let lhs = &lines[*left];
            let rhs = &lines[*right];
            lhs.line_index
                .cmp(&rhs.line_index)
                .then_with(|| lhs.bottom.total_cmp(&rhs.bottom))
                .then_with(|| lhs.left.total_cmp(&rhs.left))
        });
        let plain = indices
            .iter()
            .map(|index| lm2_has_plain_dotleader(&lines[*index].text))
            .collect::<Vec<_>>();
        let spaced = indices
            .iter()
            .map(|index| lm2_has_spaced_dotleader(&lines[*index].text))
            .collect::<Vec<_>>();
        let strong = indices
            .iter()
            .map(|index| lm2_has_strong_dotleader(&lines[*index].text))
            .collect::<Vec<_>>();
        for pos in 0..indices.len() {
            let start = pos.saturating_sub(4);
            let plain_count = plain[start..pos].iter().filter(|value| **value).count() as u8;
            let spaced_count = spaced[start..pos].iter().filter(|value| **value).count() as u8;
            let strong_count = strong[start..pos].iter().filter(|value| **value).count() as u8;
            let line = &mut lines[indices[pos]];
            line.prev_line_has_dotleader = pos > 0 && plain[pos - 1];
            line.prev4_dotleader_count = plain_count;
            line.prev4_spaced_dotleader_count = spaced_count;
            line.prev4_strong_dotleader_count = strong_count;
            line.prev4_toc_leader_context = plain_count >= 2 || strong_count >= 1;
        }
    }
}

pub(super) fn enrich_lm2_geometric_footnote_zone_features(lines: &mut [DeepLiquidSourceLine]) {
    for line in lines.iter_mut() {
        line.in_footnote_zone |= line.below_footnote_divider;
    }

    let mut pages: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (index, line) in lines.iter().enumerate() {
        pages.entry(line.page_index).or_default().push(index);
    }

    for (_page_index, mut indices) in pages {
        indices.sort_by_key(|index| lines[*index].line_index);
        if indices.len() < 4 {
            continue;
        }
        let page_has_divider = indices
            .iter()
            .any(|index| lines[*index].page_has_footnote_divider);
        if page_has_divider {
            continue;
        }
        let Some(start_position) = geometric_font_cliff_start(lines, &indices) else {
            continue;
        };
        for index in indices.into_iter().skip(start_position) {
            if geometric_zone_continuation_line(&lines[index]) {
                lines[index].in_footnote_zone = true;
            }
        }
    }
}

pub(super) fn geometric_font_cliff_start(
    lines: &[DeepLiquidSourceLine],
    indices: &[usize],
) -> Option<usize> {
    for (position, index) in indices.iter().copied().enumerate() {
        let line = &lines[index];
        if !geometric_zone_start_line(line) {
            continue;
        }
        let tail = indices
            .iter()
            .skip(position)
            .copied()
            .filter(|candidate| !geometric_zone_ignorable_line(&lines[*candidate]))
            .take(8)
            .collect::<Vec<_>>();
        if tail.len() < 2 {
            continue;
        }
        let small = tail
            .iter()
            .filter(|candidate| geometric_zone_small_font_line(&lines[**candidate]))
            .count();
        let body_like = tail
            .iter()
            .filter(|candidate| geometric_zone_body_font_line(&lines[**candidate]))
            .count();
        if small >= 2 && small * 2 >= tail.len() && body_like <= 1 {
            return Some(position);
        }
    }
    None
}

pub(super) fn geometric_zone_start_line(line: &DeepLiquidSourceLine) -> bool {
    let y_center = ((line.top + line.bottom) * 0.5) / line.page_height.max(1.0);
    y_center >= 0.45
        && geometric_zone_small_font_line(line)
        && !geometric_zone_ignorable_line(line)
        && !geometric_zone_heading_like(line)
}

pub(super) fn geometric_zone_continuation_line(line: &DeepLiquidSourceLine) -> bool {
    !geometric_zone_ignorable_line(line)
        && !geometric_zone_heading_like(line)
        && (geometric_zone_small_font_line(line)
            || looks_like_note_start(&line.text)
            || d1_runtime_citation_like(&normalize_text(&line.text)))
}

pub(super) fn geometric_zone_small_font_line(line: &DeepLiquidSourceLine) -> bool {
    line.font_ratio_page_ref <= 0.88 || line.font_ratio_page <= 0.90 || line.font_ratio_doc <= 0.90
}

pub(super) fn geometric_zone_body_font_line(line: &DeepLiquidSourceLine) -> bool {
    line.font_ratio_page_ref >= 0.98 && line.font_ratio_page >= 0.96 && line.font_ratio_doc >= 0.96
}

pub(super) fn geometric_zone_ignorable_line(line: &DeepLiquidSourceLine) -> bool {
    let text = collapse_whitespace(&line.text);
    let lower = normalize_text(&text);
    text.is_empty()
        || looks_like_page_label_furniture(&text)
        || looks_like_running_header(&lower)
        || looks_like_toc_entry(&lower)
        || lm2_toc_dotleader_line(&text)
        || d1_runtime_artifact_like(&text)
}

pub(super) fn geometric_zone_heading_like(line: &DeepLiquidSourceLine) -> bool {
    let text = collapse_whitespace(&line.text);
    let lower = normalize_text(&text);
    let words = word_count(&lower);
    (line.centered && line.bold && words <= 12 && !looks_like_note_start(&text))
        || (words <= 8 && uppercase_ratio(&text) >= 0.62 && !looks_like_note_start(&text))
        || (line.font_ratio_page_ref >= 1.02 && line.bold && words <= 12)
}

pub(super) fn enrich_lm2_doc_font_features(lines: &mut [DeepLiquidSourceLine]) {
    for line in lines.iter_mut() {
        line.doc_font_body_z = 0.0;
        line.doc_font_footnote_z = 0.0;
        line.doc_font_body_size = 0.0;
        line.doc_font_footnote_size = 0.0;
    }
    let font_keys = lines
        .iter()
        .filter_map(|line| font_bucket_key(line.font_height))
        .collect::<Vec<_>>();
    if font_keys.is_empty() {
        return;
    }
    let body_key = dominant_font_key(&font_keys);
    let footnote_key = dominant_lower_font_key(&font_keys, body_key).unwrap_or(body_key);
    let body_size = body_key as f32 / 10.0;
    let footnote_size = footnote_key as f32 / 10.0;
    let mean_key = font_keys.iter().map(|key| *key as f32).sum::<f32>() / font_keys.len() as f32;
    let spread = (font_keys
        .iter()
        .map(|key| {
            let delta = *key as f32 - mean_key;
            delta * delta
        })
        .sum::<f32>()
        / font_keys.len() as f32)
        .sqrt()
        / 10.0;
    let spread = spread.max(0.5);
    for line in lines.iter_mut() {
        line.doc_font_body_size = body_size;
        line.doc_font_footnote_size = footnote_size;
        if let Some(key) = font_bucket_key(line.font_height) {
            line.doc_font_body_z = ((key - body_key) as f32 / 10.0) / spread;
            line.doc_font_footnote_z = ((key - footnote_key) as f32 / 10.0) / spread;
        }
    }
}

pub(super) fn font_bucket_key(value: f32) -> Option<i32> {
    if value.is_finite() && value > 0.0 {
        Some((value * 10.0).round() as i32)
    } else {
        None
    }
}

pub(super) fn dominant_font_key(keys: &[i32]) -> i32 {
    let mut counts: BTreeMap<i32, usize> = BTreeMap::new();
    for key in keys {
        *counts.entry(*key).or_default() += 1;
    }
    counts
        .into_iter()
        .max_by_key(|(key, count)| (*count, *key))
        .map(|(key, _)| key)
        .unwrap_or(0)
}

pub(super) fn dominant_lower_font_key(keys: &[i32], body_key: i32) -> Option<i32> {
    let mut counts: BTreeMap<i32, usize> = BTreeMap::new();
    for key in keys.iter().copied().filter(|key| *key < body_key) {
        *counts.entry(key).or_default() += 1;
    }
    counts
        .into_iter()
        .max_by_key(|(key, count)| (*count, *key))
        .map(|(key, _)| key)
}

pub(super) fn enrich_lm2_footnote_state_features(lines: &mut [DeepLiquidSourceLine]) {
    for line in lines.iter_mut() {
        line.doc_footnote_state = false;
        line.doc_footnote_continuation = false;
    }
    let mut indices = (0..lines.len()).collect::<Vec<_>>();
    indices.sort_by_key(|index| (lines[*index].page_index, lines[*index].line_index));

    let mut active = false;
    let mut current_page: Option<usize> = None;
    let mut inherited_on_page = false;

    for index in indices {
        let page = lines[index].page_index;
        if current_page != Some(page) {
            current_page = Some(page);
            inherited_on_page = active;
        }

        if active && inherited_on_page && footnote_state_body_resume_candidate(&lines[index]) {
            active = false;
            inherited_on_page = false;
        }

        let opens_here = footnote_state_opens(&lines[index]);
        if opens_here {
            active = true;
        }

        lines[index].doc_footnote_state = active;
        lines[index].doc_footnote_continuation = active && inherited_on_page && !opens_here;
    }
}

pub(super) fn footnote_state_opens(line: &DeepLiquidSourceLine) -> bool {
    if line.below_footnote_divider {
        return true;
    }
    let y_bottom = line.bottom / line.page_height.max(1.0);
    line.page_has_footnote_divider
        && y_bottom < 0.32
        && (line.font_ratio_page < 0.94
            || line.font_ratio_doc < 0.94
            || looks_like_note_start(&line.text)
            || has_legal_note_cue(&normalize_text(&line.text)))
}

pub(super) fn footnote_state_body_resume_candidate(line: &DeepLiquidSourceLine) -> bool {
    if line.below_footnote_divider {
        return false;
    }
    let lower = normalize_text(&line.text);
    if looks_like_toc_entry(&lower) || looks_like_running_header(&lower) {
        return true;
    }
    let y_bottom = line.bottom / line.page_height.max(1.0);
    let body_cluster_like = line.font_ratio_page >= 0.94
        && line.font_ratio_doc >= 0.94
        && line.doc_font_body_z.abs() <= line.doc_font_footnote_z.abs() + 0.10;
    y_bottom > 0.35
        && body_cluster_like
        && word_count(&lower) >= 4
        && !looks_like_note_start(&line.text)
        && !has_legal_note_cue(&lower)
}

#[derive(Debug, Default)]
pub(super) struct RepetitionBucket {
    pub(super) pages: HashSet<usize>,
    pub(super) indices: Vec<usize>,
}

pub(super) fn enrich_lm2_repetition_features(lines: &mut [DeepLiquidSourceLine]) {
    for line in lines.iter_mut() {
        line.doc_repeated_edge_text = false;
        line.doc_repeated_text_count = 0;
        line.doc_repeated_top_edge = false;
        line.doc_repeated_bottom_edge = false;
        line.doc_repeated_numeric_pattern = false;
    }

    let mut buckets: HashMap<(String, usize), RepetitionBucket> = HashMap::new();
    for (index, line) in lines.iter().enumerate() {
        let y_bottom = line.bottom / line.page_height.max(1.0);
        if !(y_bottom <= 0.16 || y_bottom >= 0.84) {
            continue;
        }
        let Some(fingerprint) = repetition_text_fingerprint(&line.text) else {
            continue;
        };
        let y_band = repetition_y_band(y_bottom);
        let bucket = buckets.entry((fingerprint, y_band)).or_default();
        bucket.pages.insert(line.page_index);
        bucket.indices.push(index);
    }

    for ((fingerprint, _), bucket) in buckets {
        if bucket.pages.len() < 3 {
            continue;
        }
        let count = bucket.pages.len().min(u16::MAX as usize) as u16;
        let numeric = fingerprint.contains('#');
        for index in bucket.indices {
            let y_bottom = lines[index].bottom / lines[index].page_height.max(1.0);
            lines[index].doc_repeated_edge_text = true;
            lines[index].doc_repeated_text_count = count;
            lines[index].doc_repeated_top_edge = y_bottom >= 0.84;
            lines[index].doc_repeated_bottom_edge = y_bottom <= 0.16;
            lines[index].doc_repeated_numeric_pattern = numeric;
        }
    }
}

pub(super) fn repetition_y_band(y_bottom: f32) -> usize {
    ((y_bottom.clamp(0.0, 0.999_999) * 20.0).floor() as usize).min(19)
}

pub(super) fn repetition_text_fingerprint(text: &str) -> Option<String> {
    let mut normalized = String::new();
    let mut last_was_space = true;
    let mut last_was_digit_marker = false;
    for ch in text.chars() {
        if ch.is_ascii_alphabetic() {
            normalized.push(ch.to_ascii_lowercase());
            last_was_space = false;
            last_was_digit_marker = false;
        } else if ch.is_ascii_digit() {
            if !last_was_digit_marker {
                normalized.push('#');
            }
            last_was_space = false;
            last_was_digit_marker = true;
        } else {
            if !last_was_space {
                normalized.push(' ');
            }
            last_was_space = true;
            last_was_digit_marker = false;
        }
    }
    let normalized = normalized.trim().to_owned();
    if normalized.len() < 2 {
        return None;
    }
    let tokens = normalized.split_whitespace().collect::<Vec<_>>();
    if tokens.is_empty() || tokens.len() > 16 {
        return None;
    }
    let has_alpha = normalized.chars().any(|ch| ch.is_ascii_alphabetic());
    let has_digit_marker = normalized.contains('#');
    if !has_alpha && !has_digit_marker {
        return None;
    }
    if tokens.len() == 1 && !has_digit_marker && tokens[0].len() < 4 {
        return None;
    }
    Some(normalized)
}

pub(super) fn enrich_lm2_vertical_axis_features(lines: &mut [DeepLiquidSourceLine]) {
    for line in lines.iter_mut() {
        line.doc_vertical_axis_like = false;
        line.doc_vertical_numeric_axis_like = false;
        line.doc_vertical_short_text_axis_like = false;
        line.page_table_column_like = false;
    }

    let mut pages: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (index, line) in lines.iter().enumerate() {
        pages.entry(line.page_index).or_default().push(index);
    }

    for indices in pages.values() {
        let mut buckets: BTreeMap<i32, Vec<(usize, AxisTextKind)>> = BTreeMap::new();
        let mut table_candidates: Vec<(usize, i32, i32)> = Vec::new();
        for &index in indices {
            let line = &lines[index];
            let page_width = line.page_width.max(1.0);
            let page_height = line.page_height.max(1.0);
            let width_norm = ((line.right - line.left) / page_width).max(0.0);
            let height_norm = ((line.top - line.bottom) / page_height).max(0.0);
            if width_norm <= 0.30 && height_norm <= 0.060 && lm2_table_column_cell_like(&line.text)
            {
                let center_x = ((line.left + line.right) * 0.5 / page_width).clamp(0.0, 1.0);
                let center_y = ((line.bottom + line.top) * 0.5 / page_height).clamp(0.0, 1.0);
                table_candidates.push((
                    index,
                    (center_x * 100.0).round() as i32,
                    (center_y * 120.0).round() as i32,
                ));
            }
            if width_norm > 0.14 || height_norm > 0.055 {
                continue;
            }
            let Some(kind) = lm2_short_axis_text_kind(&line.text) else {
                continue;
            };
            let center_x = ((line.left + line.right) * 0.5 / page_width).clamp(0.0, 1.0);
            let bucket = (center_x * 80.0).round() as i32;
            buckets.entry(bucket).or_default().push((index, kind));
        }

        for bucket in buckets.values() {
            if bucket.len() < 3 {
                continue;
            }
            let mut min_y = f32::INFINITY;
            let mut max_y = f32::NEG_INFINITY;
            let mut numeric_count = 0usize;
            for &(index, kind) in bucket {
                let line = &lines[index];
                let center_y =
                    ((line.bottom + line.top) * 0.5 / line.page_height.max(1.0)).clamp(0.0, 1.0);
                min_y = min_y.min(center_y);
                max_y = max_y.max(center_y);
                if kind == AxisTextKind::Numeric {
                    numeric_count += 1;
                }
            }
            if max_y - min_y < 0.08 {
                continue;
            }
            for &(index, kind) in bucket {
                lines[index].doc_vertical_axis_like = true;
                lines[index].doc_vertical_numeric_axis_like = numeric_count >= 2;
                lines[index].doc_vertical_short_text_axis_like = kind == AxisTextKind::ShortText;
            }
        }

        let mut x_to_items: BTreeMap<i32, Vec<(usize, i32)>> = BTreeMap::new();
        for &(index, x_bucket, y_bucket) in &table_candidates {
            x_to_items
                .entry(x_bucket)
                .or_default()
                .push((index, y_bucket));
        }
        let mut active_x: HashSet<i32> = HashSet::new();
        for (&x_bucket, items) in &x_to_items {
            if items.len() < 3 {
                continue;
            }
            let min_y = items.iter().map(|(_, y)| *y).min().unwrap_or(0);
            let max_y = items.iter().map(|(_, y)| *y).max().unwrap_or(0);
            if max_y - min_y >= 5 {
                active_x.insert(x_bucket);
            }
        }
        if active_x.len() < 2 {
            continue;
        }
        let mut y_to_x: BTreeMap<i32, HashSet<i32>> = BTreeMap::new();
        for &(_index, x_bucket, y_bucket) in &table_candidates {
            if active_x.contains(&x_bucket) {
                y_to_x.entry(y_bucket).or_default().insert(x_bucket);
            }
        }
        let active_y: HashSet<i32> = y_to_x
            .iter()
            .filter_map(|(&y_bucket, x_buckets)| (x_buckets.len() >= 2).then_some(y_bucket))
            .collect();
        if active_y.len() < 3 {
            continue;
        }
        for &(index, x_bucket, y_bucket) in &table_candidates {
            if active_x.contains(&x_bucket) && active_y.contains(&y_bucket) {
                lines[index].page_table_column_like = true;
            }
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct MarkerPageInfo {
    pub(super) indices: Vec<usize>,
    pub(super) first_marker: Option<u16>,
    pub(super) first_marker_index: Option<usize>,
    pub(super) last_marker: Option<u16>,
}

pub(super) fn enrich_lm2_marker_continuity_features(lines: &mut [DeepLiquidSourceLine]) {
    for line in lines.iter_mut() {
        line.doc_note_marker = 0;
        line.doc_note_marker_first_on_page = false;
        line.doc_note_marker_mid_sequence_page = false;
        line.doc_note_marker_follows_previous_page = false;
        line.doc_note_marker_page_delta = 0;
    }

    let mut indices = (0..lines.len()).collect::<Vec<_>>();
    indices.sort_by_key(|index| (lines[*index].page_index, lines[*index].line_index));

    let mut pages: BTreeMap<usize, MarkerPageInfo> = BTreeMap::new();
    for index in indices {
        let page = lines[index].page_index;
        let marker = leading_note_marker(&lines[index].text);
        let info = pages.entry(page).or_default();
        info.indices.push(index);
        if let Some(marker) = marker {
            lines[index].doc_note_marker = marker;
            if info.first_marker.is_none() {
                info.first_marker = Some(marker);
                info.first_marker_index = Some(index);
            }
            info.last_marker = Some(marker);
        }
    }

    let mut previous_page: Option<usize> = None;
    let mut previous_last_marker: Option<u16> = None;
    for (page, info) in pages {
        if let Some(first_marker) = info.first_marker {
            let mid_sequence = page > 0 && first_marker > 1;
            let delta = if previous_page.is_some_and(|previous| previous + 1 == page) {
                previous_last_marker
                    .map(|previous| first_marker as i32 - previous as i32)
                    .unwrap_or(0)
            } else {
                0
            };
            let follows_previous = mid_sequence && (0..=3).contains(&delta);
            let delta = delta.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
            for index in info.indices.iter().copied() {
                lines[index].doc_note_marker_mid_sequence_page = mid_sequence;
                lines[index].doc_note_marker_follows_previous_page = follows_previous;
                lines[index].doc_note_marker_page_delta = delta;
            }
            if let Some(first_index) = info.first_marker_index {
                lines[first_index].doc_note_marker_first_on_page = true;
            }
        }
        previous_page = Some(page);
        previous_last_marker = info.last_marker;
    }
}

pub(super) fn leading_note_marker(text: &str) -> Option<u16> {
    if !looks_like_note_start(text) {
        return None;
    }
    let mut value = 0u32;
    let mut digits = 0usize;
    for ch in text.trim_start().chars() {
        if let Some(digit) = ch.to_digit(10) {
            value = value * 10 + digit;
            digits += 1;
            if digits > 4 {
                return None;
            }
        } else {
            break;
        }
    }
    (digits > 0 && (1..=u32::from(LM2_MAX_NOTE_MARKER)).contains(&value)).then_some(value as u16)
}

#[cfg(any(feature = "devtools", test))]
pub(super) fn eval_source_line(
    row: &Lm2EvalRow,
    use_example_role_hints: bool,
) -> DeepLiquidSourceLine {
    let page_width = row.page_width.unwrap_or(1.0).max(1.0);
    let page_height = row.page_height.unwrap_or(1.0).max(1.0);
    let left = row.x0.unwrap_or(0.0);
    let bottom = row.y0.unwrap_or(0.0);
    let right = row.x1.unwrap_or(left);
    let top = row.y1.unwrap_or(bottom);
    let font_height = row
        .font_size
        .unwrap_or_else(|| (top - bottom).abs().max(1.0));
    DeepLiquidSourceLine {
        id: format!("p{}:l{}", row.page_index, row.line_index),
        page_index: row.page_index,
        page_width,
        page_height,
        line_index: row.line_index,
        text: row.text.clone(),
        synthetic_text_geometry: false,
        left,
        bottom,
        right,
        top,
        first_visual_left: left,
        last_visual_right: right,
        page_index_norm: 0.0,
        lines_from_doc_start: 0,
        left_margin_ratio: 0.0,
        right_margin_ratio: 0.0,
        indent_both: 0.0,
        margin_symmetry: 1.0,
        line_width_ratio: 0.0,
        indent_vs_body: 0.0,
        width_vs_body: 1.0,
        front_matter_zone: false,
        margin_centered: false,
        is_block_indented: false,
        prev_line_indented: false,
        font_height,
        font_ratio_page: row.font_ratio_page.unwrap_or(1.0),
        font_ratio_page_ref: row
            .font_ratio_page_ref
            .unwrap_or(row.font_ratio_page.unwrap_or(1.0)),
        font_ratio_doc: row.font_ratio_doc.unwrap_or(1.0),
        doc_font_body_z: 0.0,
        doc_font_footnote_z: 0.0,
        doc_font_body_size: 0.0,
        doc_font_footnote_size: 0.0,
        doc_footnote_state: false,
        doc_footnote_continuation: false,
        doc_repeated_edge_text: false,
        doc_repeated_text_count: 0,
        doc_repeated_top_edge: false,
        doc_repeated_bottom_edge: false,
        doc_repeated_numeric_pattern: false,
        doc_vertical_axis_like: false,
        doc_vertical_numeric_axis_like: false,
        doc_vertical_short_text_axis_like: false,
        page_table_column_like: false,
        segment_block_id: 0,
        segment_block_line_index: 0,
        segment_block_line_count: 1,
        segment_block_first: true,
        segment_block_last: true,
        segment_block_shape: "unknown".to_owned(),
        segment_block_toc_like: false,
        segment_block_table_like: false,
        segment_block_footnote_like: false,
        segment_block_furniture_like: false,
        page_object_image_overlap_ratio: row.page_object_image_overlap_ratio.unwrap_or(0.0),
        page_object_image_hit_count: row.page_object_image_hit_count.unwrap_or(0),
        page_object_path_stroke_near_line_count: row
            .page_object_path_stroke_near_line_count
            .unwrap_or(0),
        page_object_path_stroke_density_near_line: row
            .page_object_path_stroke_density_near_line
            .unwrap_or(0.0),
        page_object_thin_horizontal_near_line_count: row
            .page_object_thin_horizontal_near_line_count
            .unwrap_or(0),
        page_object_thin_vertical_near_line_count: row
            .page_object_thin_vertical_near_line_count
            .unwrap_or(0),
        page_object_overlaps_image_bbox: row.page_object_overlaps_image_bbox.unwrap_or(false),
        page_object_ruled_row_membership: row.page_object_ruled_row_membership.unwrap_or(false),
        page_object_hide_candidate: row.page_object_hide_candidate.unwrap_or(false),
        page_object_hide_candidate_guarded: row.page_object_hide_candidate_guarded.unwrap_or(false),
        page_object_path15_candidate: row.page_object_path15_candidate.unwrap_or(false),
        page_object_ruled_or_path8_candidate: row
            .page_object_ruled_or_path8_candidate
            .unwrap_or(false),
        line_on_ruled_divider: row.line_on_ruled_divider.unwrap_or(false),
        in_ruled_cell: row.in_ruled_cell.unwrap_or(false),
        ruled_row_membership_exact: row.ruled_row_membership_exact.unwrap_or(false),
        dist_to_nearest_rule: row.dist_to_nearest_rule.unwrap_or(0.0),
        prev_line_has_dotleader: false,
        prev4_dotleader_count: 0,
        prev4_spaced_dotleader_count: 0,
        prev4_strong_dotleader_count: 0,
        prev4_toc_leader_context: false,
        doc_note_marker: 0,
        doc_note_marker_first_on_page: false,
        doc_note_marker_mid_sequence_page: false,
        doc_note_marker_follows_previous_page: false,
        doc_note_marker_page_delta: 0,
        bold: row.bold.unwrap_or(false),
        italic: row.italic.unwrap_or(false),
        centered: row.centered.unwrap_or(false),
        below_footnote_divider: row.below_footnote_divider.unwrap_or(false),
        page_has_footnote_divider: row
            .page_has_footnote_divider
            .unwrap_or_else(|| row.below_footnote_divider.unwrap_or(false)),
        in_footnote_zone: row
            .in_footnote_zone
            .unwrap_or_else(|| row.below_footnote_divider.unwrap_or(false)),
        pp_prior_role: None,
        pp_prior_label: None,
        pp_prior_score: None,
        role_hint: use_example_role_hints
            .then(|| row.role.as_deref().and_then(role_from_name))
            .flatten(),
        lv: Default::default(),
    }
}

pub(super) fn role_from_name(name: &str) -> Option<LiquidBlockRole> {
    match normalize_role_name(name).as_str() {
        "title" => Some(LiquidBlockRole::Title),
        "heading" => Some(LiquidBlockRole::Heading),
        "subheading" => Some(LiquidBlockRole::Subheading),
        "abstract" => Some(LiquidBlockRole::Abstract),
        "syllabus" => Some(LiquidBlockRole::Syllabus),
        "author_info" => Some(LiquidBlockRole::AuthorInfo),
        "lead" | "body" | "paragraph" => Some(LiquidBlockRole::Paragraph),
        "quote" => Some(LiquidBlockRole::Quote),
        "list_item" => Some(LiquidBlockRole::ListItem),
        "clause" => Some(LiquidBlockRole::Clause),
        "definition" => Some(LiquidBlockRole::Definition),
        "holding" => Some(LiquidBlockRole::Holding),
        "issue" => Some(LiquidBlockRole::Issue),
        "key_clause" => Some(LiquidBlockRole::KeyClause),
        "footnote" => Some(LiquidBlockRole::Footnote),
        "marginalia" => Some(LiquidBlockRole::Marginalia),
        "header_footer" | "header" => Some(LiquidBlockRole::Header),
        "footer" => Some(LiquidBlockRole::Footer),
        "contents" | "toc" | "table_of_contents" => Some(LiquidBlockRole::Contents),
        "caption" => Some(LiquidBlockRole::Caption),
        "table" => Some(LiquidBlockRole::Table),
        "metadata" => Some(LiquidBlockRole::Metadata),
        "section_break" => Some(LiquidBlockRole::SectionBreak),
        "noise" => Some(LiquidBlockRole::Noise),
        _ => None,
    }
}

pub(super) fn normalize_role_name(name: &str) -> String {
    name.trim().to_ascii_lowercase().replace('-', "_")
}

#[cfg(any(feature = "devtools", test))]
pub(super) fn action_for_role_name(role: &str) -> Lm2Action {
    match normalize_role_name(role).as_str() {
        "footnote" | "marginalia" => Lm2Action::Marginalia,
        "header_footer" | "header" | "footer" | "contents" | "toc" | "table_of_contents"
        | "caption" | "metadata" | "noise" | "section_break" => Lm2Action::HideNoise,
        _ => Lm2Action::Keep,
    }
}

#[cfg(any(feature = "devtools", test))]
pub(super) fn lm2_eval_report(
    model_label: String,
    pp_prior_source: Option<String>,
    pp_footnote_region_membership: bool,
    external_emissions_input: Option<&Path>,
    examples_input: &Path,
    labels_input: &Path,
    confusion: [[usize; 3]; 3],
    matched_rows: usize,
    label_rows: usize,
    use_example_role_hints: bool,
    block_quality: Lm2BlockQualityMetrics,
) -> Lm2EvalReport {
    let total = confusion.iter().flatten().copied().sum::<usize>();
    let correct = (0..ACTIONS.len())
        .map(|index| confusion[index][index])
        .sum::<usize>();
    let mut per_action = Vec::new();
    for action in ACTIONS {
        let index = action.index();
        let support = confusion[index].iter().sum::<usize>();
        let predicted = confusion.iter().map(|row| row[index]).sum::<usize>();
        let true_positive = confusion[index][index];
        let precision = ratio(true_positive, predicted);
        let recall = ratio(true_positive, support);
        let f1 = if precision + recall > 0.0 {
            2.0 * precision * recall / (precision + recall)
        } else {
            0.0
        };
        per_action.push(Lm2EvalActionMetric {
            action: action.as_str(),
            support,
            precision,
            recall,
            f1,
        });
    }
    let macro_f1 = per_action.iter().map(|metric| metric.f1).sum::<f64>() / ACTIONS.len() as f64;
    Lm2EvalReport {
        model_label,
        pp_prior_source,
        pp_footnote_region_membership,
        external_emissions_input: external_emissions_input.map(|path| path.display().to_string()),
        examples_input: examples_input.display().to_string(),
        labels_input: labels_input.display().to_string(),
        total,
        accuracy: ratio(correct, total),
        macro_f1,
        per_action,
        confusion,
        matched_rows,
        label_rows,
        use_example_role_hints,
        block_quality,
    }
}

#[cfg(any(feature = "devtools", test))]
pub(super) fn ratio(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

#[cfg(any(feature = "devtools", test))]
impl Lm2BlockQualityAccumulator {
    pub(super) fn add_page(&mut self, decoded: &[(DeepLiquidSourceLine, Lm2Action)]) {
        if decoded.is_empty() {
            return;
        }
        self.evaluated_pages += 1;
        let (_, blocks, source_lines) = build_lm2_blocks("", decoded);
        self.block_count += blocks.len();
        for block in &blocks {
            match block.role {
                LiquidBlockRole::Marginalia => self.marginalia_blocks += 1,
                LiquidBlockRole::Paragraph => self.paragraph_blocks += 1,
                _ => {}
            }
            self.hyphen_artifacts += count_hyphen_artifacts(&block.text);
        }
        for row in &source_lines {
            let block_role = blocks
                .get(row.block_index)
                .map(|block| block.role)
                .unwrap_or(LiquidBlockRole::Noise);
            if block_role == LiquidBlockRole::Marginalia {
                self.marginalia_source_lines += row.lines.len();
            }
        }
    }

    pub(super) fn finish(self) -> Lm2BlockQualityMetrics {
        Lm2BlockQualityMetrics {
            block_count: self.block_count,
            marginalia_blocks: self.marginalia_blocks,
            marginalia_source_lines: self.marginalia_source_lines,
            mean_lines_per_marginalia_block: ratio(
                self.marginalia_source_lines,
                self.marginalia_blocks,
            ),
            paragraph_blocks: self.paragraph_blocks,
            distinct_pages: self.evaluated_pages,
            paragraphs_per_page: ratio(self.paragraph_blocks, self.evaluated_pages),
            hyphen_artifacts: self.hyphen_artifacts,
            hyphen_artifacts_per_1000_blocks: if self.block_count == 0 {
                0.0
            } else {
                self.hyphen_artifacts as f64 * 1000.0 / self.block_count as f64
            },
        }
    }
}

#[cfg(any(feature = "devtools", test))]
pub(super) fn count_hyphen_artifacts(text: &str) -> usize {
    let chars = text.chars().collect::<Vec<_>>();
    chars
        .windows(3)
        .filter(|window| window[0] == '-' && window[1].is_whitespace() && window[2].is_lowercase())
        .count()
}
