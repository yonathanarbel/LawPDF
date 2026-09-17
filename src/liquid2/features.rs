//! Per-line feature extraction for the CatBoost, FastTab, and HGB models.

use super::*;

pub(super) fn bool_as_f64(value: bool) -> f64 {
    if value { 1.0 } else { 0.0 }
}

pub(super) fn role_action(role: LiquidBlockRole) -> Lm2Action {
    match role {
        LiquidBlockRole::Footnote | LiquidBlockRole::Marginalia => Lm2Action::Marginalia,
        LiquidBlockRole::Header
        | LiquidBlockRole::Footer
        | LiquidBlockRole::Contents
        | LiquidBlockRole::Caption
        | LiquidBlockRole::Table
        | LiquidBlockRole::Metadata
        | LiquidBlockRole::SectionBreak
        | LiquidBlockRole::Noise => Lm2Action::HideNoise,
        _ => Lm2Action::Keep,
    }
}

pub(super) fn lm2_numeric_catboost_features(line: &DeepLiquidSourceLine) -> HashMap<String, f64> {
    let text = collapse_whitespace(&line.text);
    let raw_text = line.text.as_str();
    let lower = text.to_ascii_lowercase();
    let tokens = words(&lower);
    let alpha_count = text.chars().filter(|ch| ch.is_alphabetic()).count();
    let digit_count = text.chars().filter(|ch| ch.is_ascii_digit()).count();
    let punct_count = text
        .chars()
        .filter(|ch| !ch.is_alphanumeric() && !ch.is_whitespace())
        .count();
    let page_width = line.page_width.max(1.0) as f64;
    let page_height = line.page_height.max(1.0) as f64;
    let x0_norm =
        ((line.left as f64 / page_width).clamp(-0.5, 1.5) * 100_000_000.0).round() / 100_000_000.0;
    let y0_norm = ((line.bottom as f64 / page_height).clamp(-0.5, 1.5) * 100_000_000.0).round()
        / 100_000_000.0;
    let x1_norm =
        ((line.right as f64 / page_width).clamp(-0.5, 1.5) * 100_000_000.0).round() / 100_000_000.0;
    let y1_norm =
        ((line.top as f64 / page_height).clamp(-0.5, 1.5) * 100_000_000.0).round() / 100_000_000.0;
    let width_norm = (x1_norm - x0_norm).max(0.0);
    let height_norm = (y1_norm - y0_norm).max(0.0);
    let uppercase_ratio = text
        .chars()
        .filter(|ch| ch.is_alphabetic() && ch.is_uppercase())
        .count() as f64
        / alpha_count.max(1) as f64;
    let digit_ratio = digit_count as f64 / text.len().max(1) as f64;
    let punct_ratio = punct_count as f64 / text.len().max(1) as f64;
    let internal_space_run_max = lm2_internal_space_run_max(raw_text);
    let numeric_token_count = lm2_numeric_token_count(&text);
    let percent_token_count = lm2_percent_token_count(&text);
    let trailing_punct_count = text
        .trim_end()
        .chars()
        .rev()
        .take_while(|ch| matches!(ch, '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']' | '}'))
        .count();
    let mut features = HashMap::new();
    features.insert(
        "all_caps_short".to_owned(),
        bool_as_f64(uppercase_ratio >= 0.90 && (1..=8).contains(&tokens.len())),
    );
    features.insert("alpha_count".to_owned(), alpha_count as f64);
    features.insert(
        "below_footnote_divider".to_owned(),
        bool_as_f64(line.below_footnote_divider),
    );
    features.insert("bold".to_owned(), bool_as_f64(line.bold));
    features.insert(
        "center_x_norm".to_owned(),
        round8((x0_norm + x1_norm) / 2.0),
    );
    features.insert(
        "center_y_norm".to_owned(),
        round8((y0_norm + y1_norm) / 2.0),
    );
    features.insert("centered".to_owned(), bool_as_f64(line.centered));
    features.insert("char_count".to_owned(), text.len() as f64);
    features.insert(
        "contains_citation_reporter".to_owned(),
        bool_as_f64(contains_citation_reporter(&lower)),
    );
    features.insert(
        "contains_section_symbol".to_owned(),
        bool_as_f64(text.contains('§') || lower.contains("section ")),
    );
    features.insert("digit_count".to_owned(), digit_count as f64);
    features.insert("digit_ratio".to_owned(), round8(digit_ratio));
    features.insert(
        "doc_font_body_size".to_owned(),
        round6(line.doc_font_body_size as f64),
    );
    features.insert(
        "doc_font_body_z".to_owned(),
        round8(line.doc_font_body_z as f64),
    );
    features.insert(
        "doc_font_footnote_size".to_owned(),
        round6(line.doc_font_footnote_size as f64),
    );
    features.insert(
        "doc_font_footnote_z".to_owned(),
        round8(line.doc_font_footnote_z as f64),
    );
    features.insert("doc_note_marker".to_owned(), line.doc_note_marker as f64);
    features.insert(
        "doc_note_marker_first_on_page".to_owned(),
        bool_as_f64(line.doc_note_marker_first_on_page),
    );
    features.insert(
        "doc_note_marker_follows_previous_page".to_owned(),
        bool_as_f64(line.doc_note_marker_follows_previous_page),
    );
    features.insert(
        "doc_note_marker_mid_sequence_page".to_owned(),
        bool_as_f64(line.doc_note_marker_mid_sequence_page),
    );
    features.insert(
        "doc_note_marker_page_delta".to_owned(),
        line.doc_note_marker_page_delta as f64,
    );
    features.insert(
        "doc_repeated_bottom_edge".to_owned(),
        bool_as_f64(line.doc_repeated_bottom_edge),
    );
    features.insert(
        "doc_repeated_edge_text".to_owned(),
        bool_as_f64(line.doc_repeated_edge_text),
    );
    features.insert(
        "doc_repeated_numeric_pattern".to_owned(),
        bool_as_f64(line.doc_repeated_numeric_pattern),
    );
    features.insert(
        "doc_vertical_axis_like".to_owned(),
        bool_as_f64(line.doc_vertical_axis_like),
    );
    features.insert(
        "doc_vertical_numeric_axis_like".to_owned(),
        bool_as_f64(line.doc_vertical_numeric_axis_like),
    );
    features.insert(
        "doc_vertical_short_text_axis_like".to_owned(),
        bool_as_f64(line.doc_vertical_short_text_axis_like),
    );
    features.insert(
        "page_table_column_like".to_owned(),
        bool_as_f64(line.page_table_column_like),
    );
    features.insert(
        "prev_line_has_dotleader".to_owned(),
        bool_as_f64(line.prev_line_has_dotleader),
    );
    features.insert(
        "prev4_dotleader_count".to_owned(),
        line.prev4_dotleader_count as f64,
    );
    features.insert(
        "prev4_spaced_dotleader_count".to_owned(),
        line.prev4_spaced_dotleader_count as f64,
    );
    features.insert(
        "prev4_strong_dotleader_count".to_owned(),
        line.prev4_strong_dotleader_count as f64,
    );
    features.insert(
        "internal_space_run_max".to_owned(),
        internal_space_run_max as f64,
    );
    features.insert("numeric_token_count".to_owned(), numeric_token_count as f64);
    features.insert("percent_token_count".to_owned(), percent_token_count as f64);
    features.insert(
        "prev4_toc_leader_context".to_owned(),
        bool_as_f64(line.prev4_toc_leader_context),
    );
    features.insert(
        "doc_repeated_text_count".to_owned(),
        line.doc_repeated_text_count as f64,
    );
    features.insert(
        "doc_repeated_top_edge".to_owned(),
        bool_as_f64(line.doc_repeated_top_edge),
    );
    features.insert(
        "font_ratio_doc".to_owned(),
        round8(line.font_ratio_doc as f64),
    );
    features.insert(
        "font_ratio_page".to_owned(),
        round8(line.font_ratio_page as f64),
    );
    features.insert("font_size".to_owned(), round6(line.font_height as f64));
    features.insert(
        "has_dotleader".to_owned(),
        bool_as_f64(lm2_numeric_has_dotleader(&text)),
    );
    features.insert(
        "has_long_dash_run".to_owned(),
        bool_as_f64(lm2_numeric_has_long_dash_run(&text)),
    );
    features.insert(
        "has_legal_note_cue".to_owned(),
        bool_as_f64(has_legal_note_cue(&lower)),
    );
    features.insert(
        "has_large_internal_space_gap".to_owned(),
        bool_as_f64(internal_space_run_max >= 3),
    );
    features.insert(
        "columnar_numeric_text_like".to_owned(),
        bool_as_f64(lm2_columnar_numeric_text_like(raw_text)),
    );
    features.insert("height_norm".to_owned(), height_norm);
    features.insert(
        "left_margin_ratio".to_owned(),
        round6(line.left_margin_ratio as f64),
    );
    features.insert(
        "right_margin_ratio".to_owned(),
        round6(line.right_margin_ratio as f64),
    );
    features.insert("indent_both".to_owned(), round6(line.indent_both as f64));
    features.insert(
        "margin_symmetry".to_owned(),
        round6(line.margin_symmetry as f64),
    );
    features.insert(
        "line_width_ratio".to_owned(),
        round6(line.line_width_ratio as f64),
    );
    features.insert(
        "indent_vs_body".to_owned(),
        round6(line.indent_vs_body as f64),
    );
    features.insert(
        "width_vs_body".to_owned(),
        round6(line.width_vs_body as f64),
    );
    features.insert("italic".to_owned(), bool_as_f64(line.italic));
    features.insert(
        "leading_whitespace_count".to_owned(),
        line.text.len().saturating_sub(line.text.trim_start().len()) as f64,
    );
    features.insert("line_index".to_owned(), line.line_index as f64);
    features.insert(
        "line_index_norm".to_owned(),
        (line.line_index as f64 / 120.0).min(1.0),
    );
    features.insert(
        "mostly_caps".to_owned(),
        bool_as_f64(uppercase_ratio >= 0.75 && alpha_count >= 3),
    );
    features.insert(
        "page_has_footnote_divider".to_owned(),
        bool_as_f64(line.page_has_footnote_divider),
    );
    features.insert("page_height".to_owned(), page_height);
    features.insert("page_index".to_owned(), line.page_index as f64);
    features.insert(
        "page_index_norm".to_owned(),
        round8(line.page_index_norm as f64),
    );
    features.insert(
        "lines_from_doc_start".to_owned(),
        line.lines_from_doc_start as f64,
    );
    features.insert(
        "is_first_page".to_owned(),
        bool_as_f64(line.page_index == 0),
    );
    features.insert(
        "is_first_two_pages".to_owned(),
        bool_as_f64(line.page_index <= 1),
    );
    features.insert(
        "front_matter_zone".to_owned(),
        bool_as_f64(line.front_matter_zone),
    );
    features.insert(
        "margin_centered".to_owned(),
        bool_as_f64(line.margin_centered),
    );
    features.insert(
        "is_block_indented".to_owned(),
        bool_as_f64(line.is_block_indented),
    );
    features.insert(
        "prev_line_indented".to_owned(),
        bool_as_f64(line.prev_line_indented),
    );
    features.insert(
        "page_number_like".to_owned(),
        bool_as_f64(lm2_numeric_page_number_like(&text)),
    );
    features.insert(
        "contains_page_word".to_owned(),
        bool_as_f64(
            lower
                .split_whitespace()
                .any(|token| token.trim_matches(|ch: char| !ch.is_ascii_alphanumeric()) == "page"),
        ),
    );
    features.insert(
        "contains_do_not_delete".to_owned(),
        bool_as_f64(lower.contains("do not delete")),
    );
    let leading_marker_type = lm2_catboost_leading_marker_type(&text);
    features.insert(
        "short_numeric_body_fragment_like".to_owned(),
        bool_as_f64(lm2_short_numeric_body_fragment_like(
            &text,
            &leading_marker_type,
            tokens.len(),
            alpha_count,
            line.font_ratio_doc,
        )),
    );
    features.insert(
        "short_alpha_body_fragment_like".to_owned(),
        bool_as_f64(lm2_short_alpha_body_fragment_like(
            &text,
            &leading_marker_type,
            tokens.len(),
            alpha_count,
            numeric_token_count,
            line.font_ratio_doc,
        )),
    );
    features.insert(
        "year_header_furniture_like".to_owned(),
        bool_as_f64(lm2_year_header_furniture_like(&text)),
    );
    features.insert("page_width".to_owned(), page_width);
    features.insert("punct_count".to_owned(), punct_count as f64);
    features.insert("punct_ratio".to_owned(), round8(punct_ratio));
    features.insert(
        "table_numeric_cell_like".to_owned(),
        bool_as_f64(lm2_numeric_table_cell_like(&text, width_norm)),
    );
    features.insert(
        "starts_digit".to_owned(),
        bool_as_f64(text.chars().next().is_some_and(|ch| ch.is_ascii_digit())),
    );
    features.insert(
        "starts_numeric_note_marker".to_owned(),
        bool_as_f64(leading_note_marker(&text).is_some()),
    );
    features.insert(
        "starts_roman_marker".to_owned(),
        bool_as_f64(lm2_numeric_starts_roman_marker(&text)),
    );
    features.insert(
        "starts_symbol_marker".to_owned(),
        bool_as_f64(
            text.trim_start()
                .chars()
                .next()
                .is_some_and(|ch| matches!(ch, '*' | '†' | '‡' | '§')),
        ),
    );
    features.insert(
        "trailing_punct_count".to_owned(),
        trailing_punct_count as f64,
    );
    features.insert("uppercase_ratio".to_owned(), round8(uppercase_ratio));
    features.insert("width_norm".to_owned(), width_norm);
    features.insert("word_count".to_owned(), tokens.len() as f64);
    features.insert("x0_norm".to_owned(), x0_norm);
    features.insert("x1_norm".to_owned(), x1_norm);
    features.insert("y0_norm".to_owned(), y0_norm);
    features.insert("y1_norm".to_owned(), y1_norm);
    // LmV (vision) features. Zero-valued for the Lm tier (the Lm model does not
    // list them, so they are ignored); populated when the LiquidVision pre-pass ran.
    let lv = &line.lv;
    features.insert("liquidvision_score".to_owned(), lv.score);
    features.insert("liquidvision_coverage".to_owned(), lv.coverage);
    features.insert(
        "liquidvision_region_area_norm".to_owned(),
        lv.region_area_norm,
    );
    features.insert(
        "liquidvision_page_region_count".to_owned(),
        lv.page_region_count,
    );
    features.insert(
        "liquidvision_page_footnote_count".to_owned(),
        lv.page_footnote_count,
    );
    features.insert(
        "liquidvision_page_table_figure_count".to_owned(),
        lv.page_table_figure_count,
    );
    features.insert("liquidvision_footnote_score".to_owned(), lv.footnote_score);
    features.insert("liquidvision_table_score".to_owned(), lv.table_score);
    features.insert("liquidvision_figure_score".to_owned(), lv.figure_score);
    features.insert("liquidvision_body_score".to_owned(), lv.body_score);
    features.insert("liquidvision_heading_score".to_owned(), lv.heading_score);
    features.insert(
        "liquidvision_furniture_score".to_owned(),
        lv.furniture_score,
    );
    features.insert(
        "liquidvision_frontmatter_score".to_owned(),
        lv.frontmatter_score,
    );
    features.insert(
        "liquidvision_has_region".to_owned(),
        bool_as_f64(lv.has_region),
    );
    features.insert(
        "liquidvision_is_footnote".to_owned(),
        bool_as_f64(lv.class == "footnote"),
    );
    features.insert(
        "liquidvision_is_table".to_owned(),
        bool_as_f64(lv.class == "table"),
    );
    features.insert(
        "liquidvision_is_figure".to_owned(),
        bool_as_f64(lv.class == "figure"),
    );
    features.insert(
        "liquidvision_is_body".to_owned(),
        bool_as_f64(lv.class == "body"),
    );
    features.insert(
        "liquidvision_is_heading".to_owned(),
        bool_as_f64(lv.class == "heading"),
    );
    features.insert(
        "liquidvision_is_furniture".to_owned(),
        bool_as_f64(lv.class == "furniture"),
    );
    features.insert(
        "liquidvision_is_frontmatter".to_owned(),
        bool_as_f64(lv.class == "frontmatter"),
    );
    features.insert(
        "liquidvision_routes_hide_noise".to_owned(),
        bool_as_f64(lv.route == "hide_noise"),
    );
    features.insert(
        "liquidvision_routes_marginalia".to_owned(),
        bool_as_f64(lv.route == "marginalia"),
    );
    features.insert(
        "liquidvision_keep_veto".to_owned(),
        bool_as_f64(lv.route == "keep_veto"),
    );
    features.insert(
        "page_object_image_overlap_ratio".to_owned(),
        round8(line.page_object_image_overlap_ratio as f64),
    );
    features.insert(
        "page_object_image_hit_count".to_owned(),
        line.page_object_image_hit_count as f64,
    );
    features.insert(
        "page_object_path_stroke_near_line_count".to_owned(),
        line.page_object_path_stroke_near_line_count as f64,
    );
    features.insert(
        "page_object_path_stroke_density_near_line".to_owned(),
        round8(line.page_object_path_stroke_density_near_line as f64),
    );
    features.insert(
        "page_object_thin_horizontal_near_line_count".to_owned(),
        line.page_object_thin_horizontal_near_line_count as f64,
    );
    features.insert(
        "page_object_thin_vertical_near_line_count".to_owned(),
        line.page_object_thin_vertical_near_line_count as f64,
    );
    features.insert(
        "page_object_overlaps_image_bbox".to_owned(),
        bool_as_f64(line.page_object_overlaps_image_bbox),
    );
    features.insert(
        "page_object_ruled_row_membership".to_owned(),
        bool_as_f64(line.page_object_ruled_row_membership),
    );
    features.insert(
        "page_object_hide_candidate".to_owned(),
        bool_as_f64(line.page_object_hide_candidate),
    );
    features.insert(
        "page_object_hide_candidate_guarded".to_owned(),
        bool_as_f64(line.page_object_hide_candidate_guarded),
    );
    features.insert(
        "page_object_path15_candidate".to_owned(),
        bool_as_f64(line.page_object_path15_candidate),
    );
    features.insert(
        "page_object_ruled_or_path8_candidate".to_owned(),
        bool_as_f64(line.page_object_ruled_or_path8_candidate),
    );
    features.insert(
        "line_on_ruled_divider".to_owned(),
        bool_as_f64(line.line_on_ruled_divider),
    );
    features.insert("in_ruled_cell".to_owned(), bool_as_f64(line.in_ruled_cell));
    features.insert(
        "ruled_row_membership_exact".to_owned(),
        bool_as_f64(line.ruled_row_membership_exact),
    );
    features.insert(
        "dist_to_nearest_rule".to_owned(),
        round8(line.dist_to_nearest_rule as f64),
    );
    features
}

pub(super) fn lm2_native_catboost_cat_features(line: &DeepLiquidSourceLine) -> Vec<String> {
    let page_width = line.page_width.max(1.0) as f64;
    let page_height = line.page_height.max(1.0) as f64;
    let x0_norm = (line.left as f64 / page_width).clamp(-0.5, 1.5);
    let y0_norm = (line.bottom as f64 / page_height).clamp(-0.5, 1.5);
    let x1_norm = (line.right as f64 / page_width).clamp(-0.5, 1.5);
    let y1_norm = (line.top as f64 / page_height).clamp(-0.5, 1.5);
    let width_norm = (x1_norm - x0_norm).max(0.0);
    let height_norm = (y1_norm - y0_norm).max(0.0);
    let line_index_norm = (line.line_index as f64 / 120.0).min(1.0);
    let text = collapse_whitespace(&line.text);
    vec![
        lm2_catboost_bin_name(y0_norm, &[0.08, 0.16, 0.28, 0.45, 0.62, 0.78, 0.90]),
        lm2_catboost_bin_name(x0_norm, &[0.08, 0.16, 0.28, 0.45, 0.65]),
        lm2_catboost_bin_name(width_norm, &[0.08, 0.18, 0.35, 0.58, 0.82]),
        lm2_catboost_bin_name(height_norm, &[0.008, 0.014, 0.020, 0.032]),
        lm2_catboost_bin_name(
            line.font_ratio_page as f64,
            &[0.72, 0.84, 0.92, 1.02, 1.16, 1.35],
        ),
        lm2_catboost_bin_name(
            line.font_ratio_doc as f64,
            &[0.72, 0.84, 0.92, 1.02, 1.16, 1.35],
        ),
        lm2_catboost_bin_name(line.font_height as f64, &[6.0, 8.0, 10.0, 12.0, 16.0, 22.0]),
        lm2_catboost_bin_name(line_index_norm, &[0.05, 0.12, 0.25, 0.45, 0.70, 0.90]),
        lm2_catboost_leading_marker_type(&text),
        lm2_catboost_first_token_shape(&text),
        lm2_catboost_terminal_punct(&text),
        if line.page_index % 2 == 1 {
            "odd".to_owned()
        } else {
            "even".to_owned()
        },
        nonempty_or_none(&line.lv.class),
        nonempty_or_none(&line.lv.route),
    ]
}

pub(super) fn lm2_catboost_bin_name(value: f64, cuts: &[f64]) -> String {
    let index = cuts
        .iter()
        .position(|cut| value <= *cut)
        .unwrap_or(cuts.len());
    format!("b{index}")
}

pub(super) fn lm2_catboost_first_token_shape(text: &str) -> String {
    let Some(token) = words(text).into_iter().next() else {
        return "none".to_owned();
    };
    let mut shape = String::new();
    for ch in token.chars().take(24) {
        if ch.is_ascii_digit() {
            shape.push('d');
        } else if ch.is_alphabetic() && ch.is_uppercase() {
            shape.push('A');
        } else if ch.is_alphabetic() {
            shape.push('a');
        } else {
            shape.push('p');
        }
        if shape.len() >= 12 {
            break;
        }
    }
    if shape.is_empty() {
        "none".to_owned()
    } else {
        shape
    }
}

pub(super) fn lm2_catboost_leading_marker_type(text: &str) -> String {
    let value = text.trim_start();
    if leading_note_marker(value).is_some() {
        return "numeric_note".to_owned();
    }
    if lm2_catboost_starts_roman_marker(value) {
        return "roman".to_owned();
    }
    if value
        .chars()
        .next()
        .is_some_and(|ch| matches!(ch, '*' | '\u{2020}' | '\u{2021}' | '\u{00a7}'))
    {
        return "symbol".to_owned();
    }
    if lm2_catboost_starts_letter_marker(value) {
        return "letter".to_owned();
    }
    if value.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
        return "digit_other".to_owned();
    }
    "none".to_owned()
}

pub(super) fn lm2_catboost_starts_roman_marker(value: &str) -> bool {
    let marker = value
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_matches(|ch| matches!(ch, '(' | ')' | '.' | ';' | ':'));
    !marker.is_empty()
        && marker.len() <= 8
        && marker.chars().all(|ch| {
            matches!(
                ch.to_ascii_lowercase(),
                'i' | 'v' | 'x' | 'l' | 'c' | 'd' | 'm'
            )
        })
}

pub(super) fn lm2_catboost_starts_letter_marker(value: &str) -> bool {
    let mut chars = value.chars();
    match (chars.next(), chars.next(), chars.next()) {
        (Some(letter), Some(punct), Some(space))
            if letter.is_ascii_alphabetic()
                && matches!(punct, ')' | '.')
                && space.is_whitespace() =>
        {
            true
        }
        (Some('('), Some(letter), Some(punct)) if letter.is_ascii_alphabetic() => {
            matches!(punct, ')')
        }
        _ => false,
    }
}

pub(super) fn lm2_catboost_terminal_punct(text: &str) -> String {
    let Some(ch) = text.trim_end().chars().next_back() else {
        return "none".to_owned();
    };
    if ".:;?!,)]}".contains(ch) {
        ch.to_string()
    } else if ch.is_ascii_digit() {
        "digit".to_owned()
    } else if ch.is_alphabetic() {
        "alpha".to_owned()
    } else {
        "other".to_owned()
    }
}

pub(super) fn nonempty_or_none(value: &str) -> String {
    if value.is_empty() {
        "none".to_owned()
    } else {
        value.to_owned()
    }
}

pub(super) fn lm2_year_header_furniture_like(text: &str) -> bool {
    let value = collapse_whitespace(text);
    let bytes = value.as_bytes();
    bytes.len() >= 7
        && matches!(bytes.get(0..2), Some(b"19" | b"20"))
        && bytes.get(2).is_some_and(u8::is_ascii_digit)
        && bytes.get(3).is_some_and(u8::is_ascii_digit)
        && bytes.get(4) == Some(&b']')
        && value.chars().last().is_some_and(|ch| ch.is_ascii_digit())
}

pub(super) fn lm2_short_numeric_body_fragment_like(
    text: &str,
    marker_type: &str,
    word_count: usize,
    alpha_count: usize,
    font_ratio_doc: f32,
) -> bool {
    !lm2_year_header_furniture_like(text)
        && matches!(marker_type, "numeric_note" | "digit_other")
        && (1..=4).contains(&word_count)
        && alpha_count >= 2
        && (0.60..=1.20).contains(&font_ratio_doc)
}

pub(super) fn lm2_short_alpha_body_fragment_like(
    text: &str,
    marker_type: &str,
    word_count: usize,
    alpha_count: usize,
    numeric_count: usize,
    font_ratio_doc: f32,
) -> bool {
    !lm2_year_header_furniture_like(text)
        && matches!(marker_type, "none" | "numeric_note" | "digit_other")
        && (1..=4).contains(&word_count)
        && alpha_count >= 2
        && numeric_count <= 1
        && (0.65..=1.15).contains(&font_ratio_doc)
        && !lm2_has_plain_dotleader(text)
        && !lm2_has_strong_dotleader(text)
}

pub(super) fn round6(value: f64) -> f64 {
    (value * 1_000_000.0).round() / 1_000_000.0
}

pub(super) fn round8(value: f64) -> f64 {
    (value * 100_000_000.0).round() / 100_000_000.0
}

pub(super) fn lm2_numeric_has_dotleader(text: &str) -> bool {
    lm2_has_plain_dotleader(text)
}

pub(super) fn lm2_numeric_has_long_dash_run(text: &str) -> bool {
    let mut run = 0usize;
    for ch in text.chars() {
        if matches!(
            ch,
            '-' | '\u{2010}' | '\u{2011}' | '\u{2012}' | '\u{2013}' | '\u{2014}' | '\u{2015}'
        ) {
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

pub(super) fn lm2_internal_space_run_max(text: &str) -> usize {
    let mut best = 0usize;
    let mut current = 0usize;
    for ch in text.trim().chars() {
        if ch == ' ' {
            current += 1;
        } else if ch == '\t' {
            current += 4;
        } else {
            best = best.max(current);
            current = 0;
        }
    }
    best.max(current)
}

pub(super) fn lm2_numeric_token_count(text: &str) -> usize {
    text.split_whitespace()
        .filter(|token| {
            lm2_axis_numeric_token(
                token
                    .trim_matches(|ch: char| matches!(ch, ',' | ';' | ':' | '[' | ']' | '{' | '}')),
            )
        })
        .count()
}

pub(super) fn lm2_percent_token_count(text: &str) -> usize {
    text.split_whitespace()
        .filter(|token| {
            let trimmed = token
                .trim_matches(|ch: char| matches!(ch, ',' | ';' | ':' | '[' | ']' | '{' | '}'));
            trimmed.ends_with('%') && lm2_axis_numeric_token(trimmed)
        })
        .count()
}

pub(super) fn lm2_columnar_numeric_text_like(text: &str) -> bool {
    let collapsed = collapse_whitespace(text);
    if collapsed.is_empty() {
        return false;
    }
    let gap = lm2_internal_space_run_max(text);
    let numeric_count = lm2_numeric_token_count(&collapsed);
    let percent_count = lm2_percent_token_count(&collapsed);
    let digit_count = collapsed.chars().filter(|ch| ch.is_ascii_digit()).count();
    let alpha_count = collapsed.chars().filter(|ch| ch.is_alphabetic()).count();
    let word_count = collapsed
        .split_whitespace()
        .filter(|token| token.chars().any(|ch| ch.is_alphanumeric()))
        .count();
    let digit_ratio = digit_count as f64 / collapsed.len().max(1) as f64;
    if gap >= 3 && (numeric_count >= 1 || digit_ratio >= 0.18) {
        return true;
    }
    if gap >= 2 && numeric_count >= 2 {
        return true;
    }
    if percent_count >= 2 && numeric_count >= 2 && word_count <= 12 {
        return true;
    }
    numeric_count >= 4 && digit_ratio >= 0.30 && alpha_count <= 24 && word_count <= 10
}

pub(super) fn lm2_has_plain_dotleader(text: &str) -> bool {
    text.contains("...")
}

pub(super) fn lm2_has_spaced_dotleader(text: &str) -> bool {
    let mut dot_count = 0usize;
    for ch in text.chars() {
        if ch == '.' {
            dot_count += 1;
            if dot_count >= 3 {
                return true;
            }
        } else if ch.is_whitespace() {
            continue;
        } else {
            dot_count = 0;
        }
    }
    false
}

pub(super) fn lm2_has_strong_dotleader(text: &str) -> bool {
    if text.contains(".....") {
        return true;
    }
    let mut dot_count = 0usize;
    for ch in text.chars() {
        if ch == '.' {
            dot_count += 1;
            if dot_count >= 5 {
                return true;
            }
        } else if ch.is_whitespace() {
            continue;
        } else {
            dot_count = 0;
        }
    }
    false
}

pub(super) fn lm2_numeric_table_cell_like(text: &str, width_norm: f64) -> bool {
    if width_norm > 0.24 {
        return false;
    }
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    if lm2_short_axis_text_kind(trimmed).is_some_and(|kind| kind == AxisTextKind::Numeric) {
        return true;
    }
    let mut numeric_tokens = 0usize;
    for token in trimmed.split_whitespace() {
        if !lm2_axis_numeric_token(token) {
            return false;
        }
        numeric_tokens += 1;
    }
    (2..=6).contains(&numeric_tokens)
}

pub(super) fn lm2_table_column_cell_like(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.chars().count() > 48 {
        return false;
    }
    let word_count = trimmed
        .split_whitespace()
        .filter(|token| token.chars().any(|ch| ch.is_alphanumeric()))
        .count();
    if word_count > 6 {
        return false;
    }
    if lm2_numeric_table_cell_like(trimmed, 0.0) {
        return true;
    }
    let alpha_count = trimmed.chars().filter(|ch| ch.is_alphabetic()).count();
    let digit_count = trimmed.chars().filter(|ch| ch.is_ascii_digit()).count();
    let punct_count = trimmed
        .chars()
        .filter(|ch| !ch.is_alphanumeric() && !ch.is_whitespace())
        .count();
    if digit_count > 0 && trimmed.chars().count() <= 24 {
        return true;
    }
    (1..=4).contains(&word_count) && alpha_count >= 2 && punct_count <= 4
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum AxisTextKind {
    Numeric,
    ShortText,
}

pub(super) fn lm2_short_axis_text_kind(text: &str) -> Option<AxisTextKind> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    if lm2_axis_numeric_token(trimmed) || lm2_axis_numeric_range(trimmed) {
        return Some(AxisTextKind::Numeric);
    }
    let alpha_count = trimmed.chars().filter(|ch| ch.is_alphabetic()).count();
    let digit_count = trimmed.chars().filter(|ch| ch.is_ascii_digit()).count();
    let word_count = trimmed
        .split_whitespace()
        .filter(|token| token.chars().any(|ch| ch.is_alphanumeric()))
        .count();
    if trimmed.chars().count() <= 8 && word_count <= 2 && alpha_count + digit_count > 0 {
        return Some(AxisTextKind::ShortText);
    }
    None
}

pub(super) fn lm2_axis_numeric_range(text: &str) -> bool {
    let Some((left, right)) = text.split_once(['-', '–']) else {
        return false;
    };
    lm2_axis_numeric_token(left) && lm2_axis_numeric_token(right)
}

pub(super) fn lm2_axis_numeric_token(text: &str) -> bool {
    let mut value = text.trim();
    if value.is_empty() {
        return false;
    }
    if let Some(stripped) = value
        .strip_prefix('(')
        .and_then(|inner| inner.strip_suffix(')'))
        .filter(|inner| !inner.is_empty())
    {
        value = stripped;
    }
    if let Some(stripped) = value.strip_prefix('-') {
        value = stripped;
    }
    if let Some(stripped) = value
        .strip_prefix('$')
        .or_else(|| value.strip_prefix('€'))
        .or_else(|| value.strip_prefix('£'))
    {
        value = stripped;
    }
    if let Some(stripped) = value.strip_suffix('%') {
        value = stripped;
    }
    let mut saw_digit = false;
    let mut saw_decimal = false;
    for ch in value.chars() {
        if ch.is_ascii_digit() {
            saw_digit = true;
        } else if ch == ',' {
            continue;
        } else if ch == '.' && !saw_decimal {
            saw_decimal = true;
        } else {
            return false;
        }
    }
    saw_digit
}

pub(super) fn lm2_numeric_page_number_like(text: &str) -> bool {
    let trimmed = text.trim();
    (1..=4).contains(&trimmed.len()) && trimmed.chars().all(|ch| ch.is_ascii_digit())
}

pub(super) fn lm2_numeric_starts_roman_marker(text: &str) -> bool {
    let token = text
        .trim_start()
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .trim_matches(|ch| matches!(ch, '(' | ')' | '.'));
    !token.is_empty()
        && token.len() <= 8
        && token.chars().all(|ch| {
            matches!(
                ch.to_ascii_lowercase(),
                'i' | 'v' | 'x' | 'l' | 'c' | 'd' | 'm'
            )
        })
}

pub(super) fn contains_citation_reporter(lower: &str) -> bool {
    ["u.s.", "s.ct.", "f.2d", "f.3d", "n.e.2d", "n.w.2d"]
        .iter()
        .any(|needle| lower.contains(needle))
}

pub(super) fn lm2_features(
    line: &DeepLiquidSourceLine,
    feature_dim: usize,
    doc_font_zscores: bool,
    repetition_fingerprints: bool,
    marker_continuity: bool,
) -> Vec<(usize, f64)> {
    let text = collapse_whitespace(&line.text);
    let lower = text.to_ascii_lowercase();
    let words = words(&lower);
    let mut features = Vec::new();
    add_feature(&mut features, feature_dim, "bias", 1.0);
    for token in words.iter().take(32) {
        add_feature(&mut features, feature_dim, &format!("w:{token}"), 1.0);
    }
    let compact = collapse_whitespace(&lower)
        .chars()
        .take(120)
        .collect::<String>();
    for size in [3usize, 4usize] {
        let chars = compact.chars().collect::<Vec<_>>();
        let mut index = 0usize;
        while index + size <= chars.len() {
            let gram = chars[index..index + size].iter().collect::<String>();
            if !gram.trim().is_empty() {
                add_feature(&mut features, feature_dim, &format!("c{size}:{gram}"), 0.35);
            }
            index += 2;
        }
    }
    for token in words.iter().take(4) {
        add_feature(&mut features, feature_dim, &format!("leadw:{token}"), 0.8);
    }

    let page_width = line.page_width.max(1.0);
    let page_height = line.page_height.max(1.0);
    let x0 = line.left / page_width;
    let y0 = line.bottom / page_height;
    let x1 = line.right / page_width;
    let y1 = line.top / page_height;
    let width = (x1 - x0).max(0.0);
    let height = (y1 - y0).max(0.0);

    add_feature(
        &mut features,
        feature_dim,
        &format!(
            "len:{}",
            bin_name(text.len() as f32, &[0.0, 2.0, 5.0, 12.0, 30.0, 80.0, 180.0])
        ),
        1.0,
    );
    add_feature(
        &mut features,
        feature_dim,
        &format!(
            "words:{}",
            bin_name(words.len() as f32, &[0.0, 1.0, 3.0, 7.0, 15.0, 35.0])
        ),
        1.0,
    );
    add_feature(
        &mut features,
        feature_dim,
        &format!("x0:{}", bin_name(x0, &[0.08, 0.16, 0.28, 0.45, 0.65])),
        1.0,
    );
    add_feature(
        &mut features,
        feature_dim,
        &format!(
            "y0:{}",
            bin_name(y0, &[0.08, 0.16, 0.28, 0.45, 0.62, 0.78, 0.90])
        ),
        1.0,
    );
    add_feature(
        &mut features,
        feature_dim,
        &format!("wbin:{}", bin_name(width, &[0.08, 0.18, 0.35, 0.58, 0.82])),
        1.0,
    );
    add_feature(
        &mut features,
        feature_dim,
        &format!("hbin:{}", bin_name(height, &[0.008, 0.014, 0.020, 0.032])),
        1.0,
    );
    add_feature(
        &mut features,
        feature_dim,
        &format!(
            "fpage:{}",
            bin_name(line.font_ratio_page, &[0.72, 0.84, 0.92, 1.02, 1.16, 1.35])
        ),
        1.0,
    );
    add_feature(
        &mut features,
        feature_dim,
        &format!(
            "fdoc:{}",
            bin_name(line.font_ratio_doc, &[0.72, 0.84, 0.92, 1.02, 1.16, 1.35])
        ),
        1.0,
    );
    add_feature(
        &mut features,
        feature_dim,
        &format!(
            "fsize:{}",
            bin_name(line.font_height, &[6.0, 8.0, 10.0, 12.0, 16.0, 22.0])
        ),
        1.0,
    );
    if doc_font_zscores {
        add_doc_font_features(&mut features, feature_dim, line);
    }
    if repetition_fingerprints {
        add_repetition_features(&mut features, feature_dim, line);
    }
    if marker_continuity {
        add_marker_continuity_features(&mut features, feature_dim, line);
    }
    if line.bold {
        add_feature(&mut features, feature_dim, "bold", 1.0);
    }
    if line.italic {
        add_feature(&mut features, feature_dim, "italic", 1.0);
    }
    if line.centered {
        add_feature(&mut features, feature_dim, "centered", 1.0);
    }
    if line.below_footnote_divider {
        add_feature(&mut features, feature_dim, "below_divider", 1.0);
    }
    if line.page_has_footnote_divider {
        add_feature(&mut features, feature_dim, "page_has_divider", 1.0);
    }
    if line.line_on_ruled_divider {
        add_feature(&mut features, feature_dim, "line_on_ruled_divider", 1.0);
    }
    if line.in_ruled_cell {
        add_feature(&mut features, feature_dim, "in_ruled_cell", 1.0);
    }
    if line.ruled_row_membership_exact {
        add_feature(
            &mut features,
            feature_dim,
            "ruled_row_membership_exact",
            1.0,
        );
    }
    add_feature(
        &mut features,
        feature_dim,
        &format!(
            "dist_to_nearest_rule:{}",
            bin_name(
                line.dist_to_nearest_rule,
                &[0.0, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0, 128.0, 256.0]
            )
        ),
        1.0,
    );
    if uppercase_ratio(&text) >= 0.75 && text.chars().filter(|ch| ch.is_alphabetic()).count() >= 3 {
        add_feature(&mut features, feature_dim, "mostly_caps", 1.0);
    }
    if text.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
        add_feature(&mut features, feature_dim, "starts_digit", 1.0);
    }
    if looks_like_note_start(&text) {
        add_feature(&mut features, feature_dim, "numeric_note_start", 1.0);
    }
    if looks_like_toc_entry(&lower) {
        add_feature(&mut features, feature_dim, "dotleader", 1.0);
    }
    if has_legal_note_cue(&lower) {
        add_feature(&mut features, feature_dim, "legal_citation_cue", 1.0);
    }
    normalize_features(features)
}

pub(super) fn add_doc_font_features(
    features: &mut Vec<(usize, f64)>,
    feature_dim: usize,
    line: &DeepLiquidSourceLine,
) {
    add_feature(
        features,
        feature_dim,
        &format!(
            "docf_body_z:{}",
            bin_name(
                line.doc_font_body_z,
                &[-2.0, -1.0, -0.5, 0.0, 0.5, 1.0, 2.0]
            )
        ),
        1.0,
    );
    add_feature(
        features,
        feature_dim,
        &format!(
            "docf_note_z:{}",
            bin_name(
                line.doc_font_footnote_z,
                &[-2.0, -1.0, -0.5, 0.0, 0.5, 1.0, 2.0]
            )
        ),
        1.0,
    );
    let body_distance = line.doc_font_body_z.abs();
    let footnote_distance = line.doc_font_footnote_z.abs();
    let closer = if (body_distance - footnote_distance).abs() <= 0.10 {
        "equal"
    } else if body_distance < footnote_distance {
        "body"
    } else {
        "footnote"
    };
    add_feature(features, feature_dim, &format!("docf_closer:{closer}"), 1.0);
}

pub(super) fn add_repetition_features(
    features: &mut Vec<(usize, f64)>,
    feature_dim: usize,
    line: &DeepLiquidSourceLine,
) {
    if !line.doc_repeated_edge_text {
        return;
    }
    add_feature(features, feature_dim, "docrep_edge", 1.0);
    add_feature(
        features,
        feature_dim,
        &format!(
            "docrep_count:{}",
            bin_name(line.doc_repeated_text_count as f32, &[3.0, 5.0, 10.0, 20.0])
        ),
        1.0,
    );
    if line.doc_repeated_top_edge {
        add_feature(features, feature_dim, "docrep_top", 1.0);
    }
    if line.doc_repeated_bottom_edge {
        add_feature(features, feature_dim, "docrep_bottom", 1.0);
    }
    if line.doc_repeated_numeric_pattern {
        add_feature(features, feature_dim, "docrep_numeric", 1.0);
    }
}

pub(super) fn add_marker_continuity_features(
    features: &mut Vec<(usize, f64)>,
    feature_dim: usize,
    line: &DeepLiquidSourceLine,
) {
    if line.doc_note_marker > 0 {
        add_feature(features, feature_dim, "docmk_present", 1.0);
        add_feature(
            features,
            feature_dim,
            &format!(
                "docmk_number:{}",
                bin_name(
                    line.doc_note_marker as f32,
                    &[1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0]
                )
            ),
            1.0,
        );
    }
    if line.doc_note_marker_first_on_page {
        add_feature(features, feature_dim, "docmk_first_on_page", 1.0);
    }
    if line.doc_note_marker_mid_sequence_page {
        add_feature(features, feature_dim, "docmk_mid_sequence_page", 1.0);
    }
    if line.doc_note_marker_follows_previous_page {
        add_feature(features, feature_dim, "docmk_follows_previous_page", 1.0);
    }
    if line.doc_note_marker_mid_sequence_page || line.doc_note_marker_follows_previous_page {
        add_feature(
            features,
            feature_dim,
            &format!(
                "docmk_page_delta:{}",
                bin_name(
                    line.doc_note_marker_page_delta as f32,
                    &[-5.0, -1.0, 0.0, 1.0, 2.0, 5.0]
                )
            ),
            1.0,
        );
    }
}

pub(super) fn add_feature(
    features: &mut Vec<(usize, f64)>,
    feature_dim: usize,
    name: &str,
    value: f64,
) {
    if feature_dim == 0 {
        return;
    }
    features.push(((fnv1a64(name) as usize) % feature_dim, value));
}

pub(super) fn normalize_features(mut features: Vec<(usize, f64)>) -> Vec<(usize, f64)> {
    features.sort_by_key(|(index, _)| *index);
    let mut merged: Vec<(usize, f64)> = Vec::with_capacity(features.len());
    for (index, value) in features {
        if let Some((last_index, last_value)) = merged.last_mut()
            && *last_index == index
        {
            *last_value += value;
            continue;
        }
        merged.push((index, value));
    }
    let norm = merged
        .iter()
        .map(|(_, value)| value * value)
        .sum::<f64>()
        .sqrt()
        .max(1.0);
    for (_, value) in &mut merged {
        *value /= norm;
    }
    merged
}
