use std::collections::HashMap;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::liquid::{DeepLiquidSourceLine, LiquidBlockRole, LiquidLayoutHint, LiquidSourceLineRef};
use crate::model::{PageInfo, PageTextChar, PdfRect};

const FOOTNOTE_SPECIALIST_RUNTIME_BIAS: f64 = -6.0;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct ExtractionStats {
    pub lines_split: usize,
    pub markers_attached: usize,
    pub markers_attached_backward: usize,
    pub markers_dropped: usize,
    pub inline_splits_merged: usize,
}

impl ExtractionStats {
    fn merge(&mut self, other: Self) {
        self.lines_split += other.lines_split;
        self.markers_attached += other.markers_attached;
        self.markers_attached_backward += other.markers_attached_backward;
        self.markers_dropped += other.markers_dropped;
        self.inline_splits_merged += other.inline_splits_merged;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractionReport {
    pub extraction_version: String,
    pub stats: ExtractionStats,
    pub events: Vec<ExtractionEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractionEvent {
    pub kind: String,
    pub page_index: usize,
    pub before_text: Vec<String>,
    pub after_text: Vec<String>,
}

#[derive(Debug, Default)]
struct ExtractionTrace {
    stats: ExtractionStats,
    events: Vec<ExtractionEvent>,
}

impl ExtractionTrace {
    fn merge(&mut self, mut other: Self) {
        self.stats.merge(other.stats);
        self.events.append(&mut other.events);
    }
}

pub fn extraction_version() -> &'static str {
    if extraction_v2_enabled() { "v2" } else { "v1" }
}

#[derive(Debug, Clone)]
struct LayoutLine {
    text: String,
    page_index: usize,
    page_width: f32,
    page_height: f32,
    line_index: usize,
    left: f32,
    bottom: f32,
    right: f32,
    top: f32,
    font_height: f32,
    font_ratio_page: f32,
    font_ratio_page_ref: f32,
    font_ratio_doc: f32,
    bold: bool,
    italic: bool,
    centered: bool,
    below_footnote_divider: bool,
    distance_below_divider: f32,
    page_has_footnote_divider: bool,
    sequence_footnote_zone: bool,
    prev_line_present: bool,
    prev_sequence_footnote_zone: bool,
    prev_below_footnote_divider: bool,
    prev_small_font: bool,
    prev_note_marker: bool,
    prev_legal_note_cue: bool,
    next_line_present: bool,
    next_sequence_footnote_zone: bool,
    next_below_footnote_divider: bool,
    next_small_font: bool,
    next_note_marker: bool,
    next_legal_note_cue: bool,
    prev_y_gap_ratio: f32,
    prev_left_delta_ratio: f32,
    prev_font_delta_ratio: f32,
    next_y_gap_ratio: f32,
    next_left_delta_ratio: f32,
    next_font_delta_ratio: f32,
    body_left_delta_ratio: f32,
    width_to_body_ratio: f32,
    prev_gap_to_median_ratio: f32,
    next_gap_to_median_ratio: f32,
    signed_body_left_delta_ratio: f32,
    right_indent_ratio: f32,
    center_offset_ratio: f32,
    font_ratio_body: f32,
    max_internal_space_run: usize,
    space_density: f32,
    leading_space_count: usize,
    trailing_space_count: usize,
    body_column_like: bool,
    narrow_measure_like: bool,
    hanging_indent_like: bool,
    vertically_isolated_like: bool,
    heading_geometry_like: bool,
    follows_hanging_note_marker: bool,
    repeated_header_footer: bool,
    segment_block_id: usize,
    segment_block_line_index: usize,
    segment_block_line_count: usize,
    segment_block_first: bool,
    segment_block_last: bool,
    segment_block_shape: String,
    segment_block_toc_like: bool,
    segment_block_table_like: bool,
    segment_block_footnote_like: bool,
    segment_block_furniture_like: bool,
    page_contents_like: bool,
    contents_or_index_entry: bool,
}

#[derive(Debug, Default)]
struct LineBuilder {
    text: String,
    left: f32,
    bottom: f32,
    right: f32,
    top: f32,
    height_total: f32,
    rect_count: usize,
    font_size_total: f32,
    font_size_count: usize,
    bold_count: usize,
    italic_count: usize,
    char_meta: Vec<CharMeta>,
    // Wrap inline superscript footnote callouts in sentinels at line finalize. Only enabled on the
    // shipped v1 extraction path; extraction-v2 has its own marker attach/merge logic that expects
    // bare digits, so callout sentinels would corrupt it.
    wrap_callouts: bool,
}

/// Per-glyph record kept while a line accumulates, used to detect inline superscript footnote
/// callouts (smaller font than the surrounding body) and wrap them in private-use sentinels.
#[derive(Clone, Debug, Default)]
struct CharMeta {
    start: usize, // byte offset of this char in the raw (pre-normalize) line text
    len: usize,   // byte length of this char
    ch: char,
    font_size: Option<f32>,
}

/// Private-use sentinels bracketing a detected inline footnote callout. They ride through
/// `normalize_line_text` and the assembly layer (both preserve non-whitespace chars) so the reflow
/// renderer can style them as superscript (and, later, hide them for TTS).
pub const CALLOUT_START: char = '\u{E000}';
pub const CALLOUT_END: char = '\u{E001}';

pub fn layout_hints_for_pages(
    pages: &[PageInfo],
    text_chars: &[Option<Vec<PageTextChar>>],
) -> Vec<LiquidLayoutHint> {
    layout_hints_and_source_lines_for_pages(pages, text_chars).0
}

pub fn deep_source_lines_for_pages(
    pages: &[PageInfo],
    text_chars: &[Option<Vec<PageTextChar>>],
) -> Vec<DeepLiquidSourceLine> {
    deep_source_lines_for_pages_with_extraction_report(pages, text_chars).0
}

pub fn deep_source_lines_for_pages_with_extraction_report(
    pages: &[PageInfo],
    text_chars: &[Option<Vec<PageTextChar>>],
) -> (Vec<DeepLiquidSourceLine>, ExtractionReport) {
    let mut all_lines = Vec::new();
    let mut page_ranges = Vec::new();
    let mut all_heights = Vec::new();
    let mut trace = ExtractionTrace::default();

    for (page_index, page) in pages.iter().enumerate() {
        let start = all_lines.len();
        if let Some(chars) = text_chars.get(page_index).and_then(Option::as_deref) {
            let (mut lines, page_trace) = extract_lines_with_trace(page_index, page, chars);
            trace.merge(page_trace);
            normalize_lines_to_page_coordinates(page, &mut lines);
            all_heights.extend(lines.iter().map(|line| line.font_height));
            all_lines.append(&mut lines);
        }
        page_ranges.push(start..all_lines.len());
    }

    enrich_line_features(pages, &page_ranges, &mut all_lines, &all_heights);
    let hints = hints_for_enriched_lines(pages, &page_ranges, &all_lines);
    let source_lines = all_lines
        .iter()
        .map(|line| {
            let page_object = pages
                .get(line.page_index)
                .map(|page| page_object_features_for_line(page, line))
                .unwrap_or_default();
            DeepLiquidSourceLine {
                id: deep_source_line_id(line.page_index, line.line_index),
                page_index: line.page_index,
                page_width: line.page_width,
                page_height: line.page_height,
                line_index: line.line_index,
                text: line.text.clone(),
                left: line.left,
                bottom: line.bottom,
                right: line.right,
                top: line.top,
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
                font_height: line.font_height,
                font_ratio_page: line.font_ratio_page,
                font_ratio_page_ref: line.font_ratio_page_ref,
                font_ratio_doc: line.font_ratio_doc,
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
                segment_block_id: line.segment_block_id,
                segment_block_line_index: line.segment_block_line_index,
                segment_block_line_count: line.segment_block_line_count,
                segment_block_first: line.segment_block_first,
                segment_block_last: line.segment_block_last,
                segment_block_shape: line.segment_block_shape.clone(),
                segment_block_toc_like: line.segment_block_toc_like,
                segment_block_table_like: line.segment_block_table_like,
                segment_block_footnote_like: line.segment_block_footnote_like,
                segment_block_furniture_like: line.segment_block_furniture_like,
                page_object_image_overlap_ratio: page_object.image_overlap_ratio,
                page_object_image_hit_count: page_object.image_hit_count,
                page_object_path_stroke_near_line_count: page_object.path_stroke_near_line_count,
                page_object_path_stroke_density_near_line: page_object
                    .path_stroke_density_near_line,
                page_object_thin_horizontal_near_line_count: page_object
                    .thin_horizontal_near_line_count,
                page_object_thin_vertical_near_line_count: page_object
                    .thin_vertical_near_line_count,
                page_object_overlaps_image_bbox: page_object.overlaps_image_bbox,
                page_object_ruled_row_membership: page_object.ruled_row_membership,
                page_object_hide_candidate: page_object.hide_candidate,
                page_object_hide_candidate_guarded: page_object.hide_candidate_guarded,
                page_object_path15_candidate: page_object.path15_candidate,
                page_object_ruled_or_path8_candidate: page_object.ruled_or_path8_candidate,
                line_on_ruled_divider: page_object.line_on_ruled_divider,
                in_ruled_cell: page_object.in_ruled_cell,
                ruled_row_membership_exact: page_object.ruled_row_membership_exact,
                dist_to_nearest_rule: page_object.dist_to_nearest_rule,
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
                bold: line.bold,
                italic: line.italic,
                centered: line.centered,
                below_footnote_divider: line.below_footnote_divider,
                page_has_footnote_divider: line.page_has_footnote_divider,
                in_footnote_zone: line.below_footnote_divider || line.sequence_footnote_zone,
                pp_prior_role: None,
                pp_prior_label: None,
                pp_prior_score: None,
                role_hint: hint_role_for_line(&hints, line),
                lv: Default::default(),
            }
        })
        .collect();
    (
        source_lines,
        ExtractionReport {
            extraction_version: extraction_version().to_owned(),
            stats: trace.stats,
            events: trace.events,
        },
    )
}

fn deep_source_line_id(page_index: usize, line_index: usize) -> String {
    format!("p{page_index}:l{line_index}")
}

#[derive(Debug, Clone, Copy, Default)]
struct PageObjectLineFeatures {
    image_overlap_ratio: f32,
    image_hit_count: u16,
    path_stroke_near_line_count: u16,
    path_stroke_density_near_line: f32,
    thin_horizontal_near_line_count: u16,
    thin_vertical_near_line_count: u16,
    overlaps_image_bbox: bool,
    ruled_row_membership: bool,
    hide_candidate: bool,
    hide_candidate_guarded: bool,
    path15_candidate: bool,
    ruled_or_path8_candidate: bool,
    line_on_ruled_divider: bool,
    in_ruled_cell: bool,
    ruled_row_membership_exact: bool,
    dist_to_nearest_rule: f32,
}

fn page_object_features_for_line(page: &PageInfo, line: &LayoutLine) -> PageObjectLineFeatures {
    const CANDIDATE_PATH_COUNT: u16 = 3;
    const CANDIDATE_IMAGE_OVERLAP: f32 = 0.15;
    let rect = PdfRect::new(line.left, line.bottom, line.right, line.top);
    let area = rect.width().max(0.0) * rect.height().max(0.0);
    let area = area.max(1.0);
    let pad_y = 3.0f32.max(rect.height() * 1.25);
    let pad_x = 2.0f32.max(page.width.max(1.0) * 0.01);
    let near = inflate_rect(rect, pad_x, pad_y);

    let mut image_hit_count = 0u16;
    let mut image_overlap_ratio = 0.0f32;
    for image_rect in &page.image_object_rects {
        let overlap = rect_intersection_area(rect, *image_rect) / area;
        if overlap > 0.0 {
            image_hit_count = image_hit_count.saturating_add(1);
            image_overlap_ratio = image_overlap_ratio.max(overlap);
        }
    }

    let mut path_stroke_near_line_count = 0u16;
    let mut near_path_length = 0.0f32;
    for path_rect in &page.path_object_rects {
        if rect_intersects(near, *path_rect) {
            path_stroke_near_line_count = path_stroke_near_line_count.saturating_add(1);
            near_path_length += path_rect.width().max(path_rect.height());
        }
    }

    let thin_horizontal_near_line_count = page
        .thin_horizontal_object_rects
        .iter()
        .filter(|rect| rect_intersects(near, **rect))
        .count()
        .min(u16::MAX as usize) as u16;
    let thin_vertical_near_line_count = page
        .thin_vertical_object_rects
        .iter()
        .filter(|rect| rect_intersects(near, **rect))
        .count()
        .min(u16::MAX as usize) as u16;
    let ruled_row_membership = thin_horizontal_near_line_count >= 1
        && (thin_vertical_near_line_count >= 1
            || path_stroke_near_line_count >= CANDIDATE_PATH_COUNT);
    let overlaps_image_bbox = image_overlap_ratio >= CANDIDATE_IMAGE_OVERLAP;
    let hide_candidate = overlaps_image_bbox
        || ruled_row_membership
        || path_stroke_near_line_count >= CANDIDATE_PATH_COUNT;
    let hide_candidate_guarded = hide_candidate && page_object_text_guard_allows_hide(&line.text);
    let vector_rule_features = vector_rule_features_for_line(page, rect);
    PageObjectLineFeatures {
        image_overlap_ratio: round6_f32(image_overlap_ratio),
        image_hit_count,
        path_stroke_near_line_count,
        path_stroke_density_near_line: round6_f32(near_path_length / page.width.max(1.0)),
        thin_horizontal_near_line_count,
        thin_vertical_near_line_count,
        overlaps_image_bbox,
        ruled_row_membership,
        hide_candidate,
        hide_candidate_guarded,
        path15_candidate: path_stroke_near_line_count >= 15,
        ruled_or_path8_candidate: ruled_row_membership || path_stroke_near_line_count >= 8,
        line_on_ruled_divider: vector_rule_features.line_on_ruled_divider,
        in_ruled_cell: vector_rule_features.in_ruled_cell,
        ruled_row_membership_exact: vector_rule_features.ruled_row_membership_exact,
        dist_to_nearest_rule: vector_rule_features.dist_to_nearest_rule,
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct VectorRuleLineFeatures {
    line_on_ruled_divider: bool,
    in_ruled_cell: bool,
    ruled_row_membership_exact: bool,
    dist_to_nearest_rule: f32,
}

fn vector_rule_features_for_line(page: &PageInfo, rect: PdfRect) -> VectorRuleLineFeatures {
    let has_rules = !page.vector_horizontal_rule_rects.is_empty()
        || !page.vector_vertical_rule_rects.is_empty();
    if !has_rules {
        return VectorRuleLineFeatures::default();
    }
    let center_x = (rect.left + rect.right) * 0.5;
    let center_y = (rect.bottom + rect.top) * 0.5;
    let line_width = rect.width().max(1.0);
    let line_height = rect.height().max(1.0);
    let near_y_pad = 2.0f32.max(line_height * 0.35);
    let near_x_pad = 2.0f32.max(line_width * 0.02);
    let inflated = inflate_rect(rect, near_x_pad, near_y_pad);

    let line_on_ruled_divider = page.vector_horizontal_rule_rects.iter().any(|rule| {
        let overlap = horizontal_overlap_width(rect, *rule);
        overlap >= (line_width * 0.20).min(18.0)
            && (rule_center_y(*rule) - center_y).abs() <= near_y_pad
    });
    let in_explicit_cell = page
        .vector_ruled_cell_rects
        .iter()
        .any(|cell| rect_inside_with_tolerance(rect, *cell, 2.0));
    let horizontal_below = page.vector_horizontal_rule_rects.iter().any(|rule| {
        rule_center_y(*rule) <= rect.bottom + near_y_pad
            && horizontal_spans_x(*rule, center_x)
            && rect.bottom - rule_center_y(*rule) <= line_height.max(24.0)
    });
    let horizontal_above = page.vector_horizontal_rule_rects.iter().any(|rule| {
        rule_center_y(*rule) >= rect.top - near_y_pad
            && horizontal_spans_x(*rule, center_x)
            && rule_center_y(*rule) - rect.top <= line_height.max(24.0)
    });
    let vertical_left = page.vector_vertical_rule_rects.iter().any(|rule| {
        rule_center_x(*rule) <= rect.left + near_x_pad
            && vertical_spans_y(*rule, center_y)
            && rect.left - rule_center_x(*rule) <= line_width.max(96.0)
    });
    let vertical_right = page.vector_vertical_rule_rects.iter().any(|rule| {
        rule_center_x(*rule) >= rect.right - near_x_pad
            && vertical_spans_y(*rule, center_y)
            && rule_center_x(*rule) - rect.right <= line_width.max(96.0)
    });
    let in_inferred_cell = horizontal_below && horizontal_above && vertical_left && vertical_right;
    let in_ruled_cell = in_explicit_cell || in_inferred_cell;
    let ruled_row_membership_exact = in_ruled_cell
        || (page
            .vector_horizontal_rule_rects
            .iter()
            .filter(|rule| {
                rect_intersects(inflated, **rule)
                    || (horizontal_spans_x(**rule, center_x)
                        && (rule_center_y(**rule) - center_y).abs() <= line_height.max(18.0))
            })
            .count()
            >= 1
            && !page.vector_vertical_rule_rects.is_empty());
    let dist_to_nearest_rule = nearest_vector_rule_distance(page, rect);
    VectorRuleLineFeatures {
        line_on_ruled_divider,
        in_ruled_cell,
        ruled_row_membership_exact,
        dist_to_nearest_rule: round6_f32(dist_to_nearest_rule),
    }
}

fn nearest_vector_rule_distance(page: &PageInfo, rect: PdfRect) -> f32 {
    page.vector_horizontal_rule_rects
        .iter()
        .chain(page.vector_vertical_rule_rects.iter())
        .map(|rule| rect_distance(rect, *rule))
        .fold(f32::INFINITY, f32::min)
        .min(999.0)
}

fn rect_inside_with_tolerance(inner: PdfRect, outer: PdfRect, tolerance: f32) -> bool {
    inner.left >= outer.left - tolerance
        && inner.right <= outer.right + tolerance
        && inner.bottom >= outer.bottom - tolerance
        && inner.top <= outer.top + tolerance
}

fn horizontal_spans_x(rect: PdfRect, x: f32) -> bool {
    x >= rect.left - 2.0 && x <= rect.right + 2.0
}

fn vertical_spans_y(rect: PdfRect, y: f32) -> bool {
    y >= rect.bottom - 2.0 && y <= rect.top + 2.0
}

fn rule_center_x(rect: PdfRect) -> f32 {
    (rect.left + rect.right) * 0.5
}

fn rule_center_y(rect: PdfRect) -> f32 {
    (rect.bottom + rect.top) * 0.5
}

fn horizontal_overlap_width(a: PdfRect, b: PdfRect) -> f32 {
    (a.right.min(b.right) - a.left.max(b.left)).max(0.0)
}

fn rect_distance(a: PdfRect, b: PdfRect) -> f32 {
    let dx = if a.right < b.left {
        b.left - a.right
    } else if b.right < a.left {
        a.left - b.right
    } else {
        0.0
    };
    let dy = if a.top < b.bottom {
        b.bottom - a.top
    } else if b.top < a.bottom {
        a.bottom - b.top
    } else {
        0.0
    };
    (dx * dx + dy * dy).sqrt()
}

fn inflate_rect(rect: PdfRect, pad_x: f32, pad_y: f32) -> PdfRect {
    PdfRect::new(
        rect.left - pad_x,
        rect.bottom - pad_y,
        rect.right + pad_x,
        rect.top + pad_y,
    )
}

fn rect_intersects(a: PdfRect, b: PdfRect) -> bool {
    a.left <= b.right && a.right >= b.left && a.bottom <= b.top && a.top >= b.bottom
}

fn rect_intersection_area(a: PdfRect, b: PdfRect) -> f32 {
    let left = a.left.max(b.left);
    let right = a.right.min(b.right);
    let bottom = a.bottom.max(b.bottom);
    let top = a.top.min(b.top);
    if right <= left || top <= bottom {
        0.0
    } else {
        (right - left) * (top - bottom)
    }
}

fn page_object_text_guard_allows_hide(text: &str) -> bool {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() {
        return false;
    }
    let words = text
        .split(|ch: char| !ch.is_ascii_alphabetic())
        .filter(|word| !word.is_empty())
        .count();
    let digits = text.chars().filter(|ch| ch.is_ascii_digit()).count();
    let punct = text
        .chars()
        .filter(|ch| !ch.is_alphanumeric() && !ch.is_whitespace())
        .count();
    if words >= 12 && digits <= 2 && punct <= 4 {
        return false;
    }
    if text.len() > 140 && digits <= 3 {
        return false;
    }
    true
}

fn round6_f32(value: f32) -> f32 {
    (value * 1_000_000.0).round() / 1_000_000.0
}

fn normalize_lines_to_page_coordinates(page: &PageInfo, lines: &mut [LayoutLine]) {
    if page.coord_offset_left == 0.0 && page.coord_offset_bottom == 0.0 {
        return;
    }
    for line in lines {
        let (left, right) = clamp_interval_keep_extent(
            line.left - page.coord_offset_left,
            line.right - page.coord_offset_left,
            page.width,
        );
        let (bottom, top) = clamp_interval_keep_extent(
            line.bottom - page.coord_offset_bottom,
            line.top - page.coord_offset_bottom,
            page.height,
        );
        line.left = left;
        line.right = right;
        line.bottom = bottom;
        line.top = top;
    }
}

fn clamp_interval_keep_extent(start: f32, end: f32, limit: f32) -> (f32, f32) {
    if !limit.is_finite() || limit <= 0.0 {
        return (0.0, 0.0);
    }
    let low = start.min(end);
    let high = start.max(end);
    let extent = (high - low).max(0.0).min(limit);
    if high < 0.0 {
        return (0.0, extent);
    }
    if low > limit {
        return (limit - extent, limit);
    }
    let clamped_low = low.clamp(0.0, limit);
    let clamped_high = high.clamp(0.0, limit);
    if clamped_high >= clamped_low {
        (clamped_low, clamped_high)
    } else {
        let center = ((low + high) * 0.5).clamp(0.0, limit);
        let shifted_low = (center - extent * 0.5).clamp(0.0, (limit - extent).max(0.0));
        (shifted_low, shifted_low + extent)
    }
}

pub fn layout_hints_and_source_lines_for_pages(
    pages: &[PageInfo],
    text_chars: &[Option<Vec<PageTextChar>>],
) -> (Vec<LiquidLayoutHint>, Vec<LiquidSourceLineRef>) {
    let mut all_lines = Vec::new();
    let mut page_ranges = Vec::new();
    let mut all_heights = Vec::new();

    for (page_index, page) in pages.iter().enumerate() {
        let start = all_lines.len();
        if let Some(chars) = text_chars.get(page_index).and_then(Option::as_deref) {
            let mut lines = extract_lines(page_index, page, chars);
            normalize_lines_to_page_coordinates(page, &mut lines);
            all_heights.extend(lines.iter().map(|line| line.font_height));
            all_lines.append(&mut lines);
        }
        page_ranges.push(start..all_lines.len());
    }

    enrich_line_features(pages, &page_ranges, &mut all_lines, &all_heights);

    let hints = hints_for_enriched_lines(pages, &page_ranges, &all_lines);
    let source_lines = all_lines
        .iter()
        .filter_map(|line| {
            hint_role_for_line(&hints, line).map(|role| LiquidSourceLineRef {
                id: None,
                page_index: line.page_index,
                line_index: line.line_index,
                text: line.text.clone(),
                role,
                note_markers: Vec::new(),
            })
        })
        .collect();
    (hints, source_lines)
}

fn hints_for_enriched_lines(
    pages: &[PageInfo],
    page_ranges: &[std::ops::Range<usize>],
    all_lines: &[LayoutLine],
) -> Vec<LiquidLayoutHint> {
    let mut hints = Vec::new();
    for (page_index, page) in pages.iter().enumerate() {
        let lines = page_ranges
            .get(page_index)
            .map(|range| &all_lines[range.clone()])
            .unwrap_or(&[]);
        extend_repository_cover_hints(&mut hints, lines);
        extend_page_contents_noise_hints(&mut hints, lines);
        if let Some(model) = layout_role_model() {
            extend_model_hints(&mut hints, page, lines, model);
        }
        if let Some(model) = liquid_core_role_model() {
            extend_liquid_core_model_hints(&mut hints, page, lines, model);
        }
        if let Some(model) = body_role_model() {
            extend_body_specialist_hints(&mut hints, lines, model);
        }
        if let Some(model) = heading_role_model() {
            extend_heading_specialist_hints(&mut hints, lines, model);
        }
        if let Some(model) = header_footer_role_model() {
            extend_header_footer_specialist_hints(&mut hints, page, lines, model);
        }
        if let Some(model) = footnote_role_model() {
            extend_footnote_specialist_hints(&mut hints, lines, model);
        }
        extend_heuristic_footnote_hints(&mut hints, page, lines);
        extend_decoded_footnote_run_hints(&mut hints, lines);
    }
    hints
}

pub fn source_pages_from_text_chars(
    pages: &[PageInfo],
    text_chars: &[Option<Vec<PageTextChar>>],
) -> Vec<Option<String>> {
    pages
        .iter()
        .enumerate()
        .map(|(page_index, page)| {
            let chars = text_chars.get(page_index).and_then(Option::as_deref)?;
            let mut lines = extract_lines(page_index, page, chars);
            normalize_lines_to_page_coordinates(page, &mut lines);
            lines.sort_by(|a, b| {
                y_from_top(page, a)
                    .total_cmp(&y_from_top(page, b))
                    .then_with(|| a.left.total_cmp(&b.left))
            });
            let text = lines
                .into_iter()
                .map(|line| line.text)
                .filter(|text| !text.trim().is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            (!text.trim().is_empty()).then_some(text)
        })
        .collect()
}

fn extract_lines(page_index: usize, page: &PageInfo, chars: &[PageTextChar]) -> Vec<LayoutLine> {
    extract_lines_with_stats(page_index, page, chars).0
}

fn extract_lines_with_stats(
    page_index: usize,
    page: &PageInfo,
    chars: &[PageTextChar],
) -> (Vec<LayoutLine>, ExtractionStats) {
    let (lines, trace) = extract_lines_with_trace(page_index, page, chars);
    (lines, trace.stats)
}

fn extract_lines_with_trace(
    page_index: usize,
    page: &PageInfo,
    chars: &[PageTextChar],
) -> (Vec<LayoutLine>, ExtractionTrace) {
    extract_lines_with_options(page_index, page, chars, extraction_v2_enabled())
}

fn extraction_v2_enabled() -> bool {
    std::env::var("LAWPDF_EXTRACT_V2")
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}

fn extract_lines_with_options(
    page_index: usize,
    page: &PageInfo,
    chars: &[PageTextChar],
    extraction_v2: bool,
) -> (Vec<LayoutLine>, ExtractionTrace) {
    let mut lines = Vec::new();
    let mut current = LineBuilder::default();
    current.wrap_callouts = !extraction_v2;
    let mut trace = ExtractionTrace::default();

    for item in chars {
        if matches!(item.ch, '\n' | '\r') {
            flush_line(page, &mut current, &mut lines);
            continue;
        }

        if let Some(rect) = item.rect {
            if current.has_rect()
                && !current.text.trim().is_empty()
                && is_new_visual_line(&current, rect)
            {
                flush_line(page, &mut current, &mut lines);
            }
            if extraction_v2
                && current.has_rect()
                && !current.text.trim().is_empty()
                && should_split_font_step_down(&current, item, rect)
            {
                trace.stats.lines_split += 1;
                trace.events.push(ExtractionEvent {
                    kind: "lines_split".to_owned(),
                    page_index,
                    before_text: vec![normalize_line_text(&current.text)],
                    after_text: vec![format!("next fragment starts with {:?}", item.ch)],
                });
                flush_line(page, &mut current, &mut lines);
            }
            current.push(item.ch, Some(rect));
            current.push_style(item);
        } else {
            current.push(item.ch, None);
            current.push_style(item);
        }
    }

    flush_line(page, &mut current, &mut lines);
    if extraction_v2 {
        trace.merge(merge_or_drop_standalone_marker_fragments(
            page_index, &mut lines,
        ));
    }
    for (line_index, line) in lines.iter_mut().enumerate() {
        line.page_index = page_index;
        line.line_index = line_index;
    }
    (lines, trace)
}

fn should_split_font_step_down(current: &LineBuilder, item: &PageTextChar, rect: PdfRect) -> bool {
    let Some(font_size) = item
        .font_size
        .filter(|size| size.is_finite() && *size > 0.0)
    else {
        return false;
    };
    let current_font_size = current.average_font_size();
    if current_font_size <= 0.0 || font_size > current_font_size * 0.86 {
        return false;
    }
    let visible_chars = current
        .text
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .count();
    if visible_chars < 18 {
        return false;
    }
    let current_center = (current.top + current.bottom) * 0.5;
    let next_center = (rect.top + rect.bottom) * 0.5;
    let same_visual_line_tolerance = current.average_height().max(rect.height()).max(1.0) * 0.45;
    if (current_center - next_center).abs() > same_visual_line_tolerance {
        return false;
    }
    let next_starts_note_like = item.ch.is_ascii_digit() || item.ch.is_ascii_alphabetic();
    next_starts_note_like && current.text.trim_end().chars().last().is_some()
}

fn merge_or_drop_standalone_marker_fragments(
    page_index: usize,
    lines: &mut Vec<LayoutLine>,
) -> ExtractionTrace {
    let mut merged = Vec::with_capacity(lines.len());
    let mut index = 0usize;
    let mut trace = ExtractionTrace::default();
    while index < lines.len() {
        if looks_like_standalone_marker_fragment(&lines[index]) {
            if let Some(next) = lines.get(index + 1)
                && marker_can_attach_to_next(&lines[index], next)
                && !marker_has_previous_line_host(&lines[index], merged.last())
            {
                let mut next = next.clone();
                let after_text = join_marker_with_anchor_text(&lines[index].text, &next.text);
                trace.events.push(ExtractionEvent {
                    kind: "markers_attached_forward".to_owned(),
                    page_index,
                    before_text: vec![
                        normalize_line_text(&lines[index].text),
                        normalize_line_text(&next.text),
                    ],
                    after_text: vec![after_text.clone()],
                });
                next.text = after_text;
                next.left = lines[index].left.min(next.left);
                next.bottom = lines[index].bottom.min(next.bottom);
                next.right = lines[index].right.max(next.right);
                next.top = lines[index].top.max(next.top);
                merged.push(next);
                trace.stats.markers_attached += 1;
                index += 2;
                continue;
            }
            if let Some(previous) = merged.last_mut()
                && marker_can_attach_to_previous(&lines[index], previous)
            {
                let after_text = join_anchor_with_marker_text(&previous.text, &lines[index].text);
                trace.events.push(ExtractionEvent {
                    kind: "markers_attached_backward".to_owned(),
                    page_index,
                    before_text: vec![
                        normalize_line_text(&previous.text),
                        normalize_line_text(&lines[index].text),
                    ],
                    after_text: vec![after_text.clone()],
                });
                previous.text = after_text;
                previous.left = previous.left.min(lines[index].left);
                previous.bottom = previous.bottom.min(lines[index].bottom);
                previous.right = previous.right.max(lines[index].right);
                previous.top = previous.top.max(lines[index].top);
                previous.font_height = ((previous.font_height + lines[index].font_height) * 0.5)
                    .max(previous.font_height.min(lines[index].font_height));
                trace.stats.markers_attached += 1;
                trace.stats.markers_attached_backward += 1;
                index += 1;
                continue;
            }
            trace.stats.markers_dropped += 1;
            trace.events.push(ExtractionEvent {
                kind: "markers_dropped".to_owned(),
                page_index,
                before_text: vec![normalize_line_text(&lines[index].text)],
                after_text: Vec::new(),
            });
            index += 1;
            continue;
        }
        merged.push(lines[index].clone());
        index += 1;
    }
    trace.stats.inline_splits_merged +=
        merge_same_visual_line_inline_marker_splits(page_index, &mut merged, &mut trace.events);
    *lines = merged;
    trace
}

fn merge_same_visual_line_inline_marker_splits(
    page_index: usize,
    lines: &mut Vec<LayoutLine>,
    events: &mut Vec<ExtractionEvent>,
) -> usize {
    let mut merged: Vec<LayoutLine> = Vec::with_capacity(lines.len());
    let mut merge_count = 0usize;
    for line in lines.drain(..) {
        if let Some(previous) = merged.last_mut()
            && looks_like_inline_marker_split(&line)
            && same_visual_line(previous, &line)
            && previous.right <= line.right
            && previous
                .text
                .trim_end()
                .chars()
                .last()
                .is_some_and(|ch| matches!(ch, '.' | '?' | '!' | '"' | '\'' | ')' | ']'))
        {
            let after_text = normalize_line_text(&format!("{}{}", previous.text, line.text));
            events.push(ExtractionEvent {
                kind: "inline_splits_merged".to_owned(),
                page_index,
                before_text: vec![
                    normalize_line_text(&previous.text),
                    normalize_line_text(&line.text),
                ],
                after_text: vec![after_text.clone()],
            });
            previous.text = after_text;
            previous.left = previous.left.min(line.left);
            previous.bottom = previous.bottom.min(line.bottom);
            previous.right = previous.right.max(line.right);
            previous.top = previous.top.max(line.top);
            previous.font_height = ((previous.font_height + line.font_height) * 0.5)
                .max(previous.font_height.min(line.font_height));
            merge_count += 1;
            continue;
        }
        merged.push(line);
    }
    *lines = merged;
    merge_count
}

fn looks_like_standalone_marker_fragment(line: &LayoutLine) -> bool {
    let text = line.text.trim();
    (1..=4).contains(&text.len()) && text.chars().all(|ch| ch.is_ascii_digit())
}

fn marker_can_attach_to_next(marker: &LayoutLine, next: &LayoutLine) -> bool {
    if marker.page_index != next.page_index {
        return false;
    }
    if marker.font_height > next.font_height * 1.35 {
        return false;
    }
    let marker_center = (marker.top + marker.bottom) * 0.5;
    let next_center = (next.top + next.bottom) * 0.5;
    let vertical_delta = (marker_center - next_center).abs();
    let plausible_superscript_offset =
        marker_center > next_center && vertical_delta <= next.font_height * 2.0;
    let horizontal_close =
        marker.right <= next.right && marker.left <= next.left + next.page_width * 0.20;
    plausible_superscript_offset
        && horizontal_close
        && next
            .text
            .trim_start()
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_alphabetic() || matches!(ch, '.' | ',' | ';' | ':'))
}

fn marker_can_attach_to_previous(marker: &LayoutLine, previous: &LayoutLine) -> bool {
    if marker.page_index != previous.page_index {
        return false;
    }
    if marker.font_height > previous.font_height * 1.35 {
        return false;
    }
    let marker_center = (marker.top + marker.bottom) * 0.5;
    let previous_center = (previous.top + previous.bottom) * 0.5;
    let vertical_delta = (marker_center - previous_center).abs();
    let plausible_superscript_offset =
        marker_center > previous_center && vertical_delta <= previous.font_height * 2.0;
    let horizontal_close = marker.left >= previous.left
        && marker.left <= previous.right + previous.page_width * 0.04
        && marker.right >= previous.right;
    let previous_can_host_marker = previous.text.trim_end().chars().last().is_some_and(|ch| {
        ch.is_ascii_alphabetic() || matches!(ch, '.' | '?' | '!' | '"' | '\'' | ')' | ']')
    });
    plausible_superscript_offset && horizontal_close && previous_can_host_marker
}

fn marker_has_previous_line_host(marker: &LayoutLine, previous: Option<&LayoutLine>) -> bool {
    previous
        .filter(|line| marker_can_attach_to_previous(marker, line))
        .is_some()
}

fn join_marker_with_anchor_text(marker: &str, anchor: &str) -> String {
    let anchor = anchor.trim_start();
    if anchor
        .chars()
        .next()
        .is_some_and(|ch| matches!(ch, '.' | ',' | ';' | ':'))
    {
        normalize_line_text(&format!("{}{anchor}", marker.trim()))
    } else {
        normalize_line_text(&format!("{} {anchor}", marker.trim()))
    }
}

fn join_anchor_with_marker_text(anchor: &str, marker: &str) -> String {
    normalize_line_text(&format!("{} {}", anchor.trim_end(), marker.trim()))
}

fn looks_like_inline_marker_split(line: &LayoutLine) -> bool {
    let text = line.text.trim_start();
    let digit_count = text.chars().take_while(|ch| ch.is_ascii_digit()).count();
    if !(1..=4).contains(&digit_count) {
        return false;
    }
    let rest = text[digit_count..].trim_start();
    !rest.is_empty()
        && rest
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_uppercase() || matches!(ch, '"' | '\'' | '('))
}

fn same_visual_line(left: &LayoutLine, right: &LayoutLine) -> bool {
    if left.page_index != right.page_index {
        return false;
    }
    let left_center = (left.top + left.bottom) * 0.5;
    let right_center = (right.top + right.bottom) * 0.5;
    (left_center - right_center).abs() <= left.font_height.max(right.font_height).max(1.0) * 0.35
}

fn flush_line(page: &PageInfo, current: &mut LineBuilder, lines: &mut Vec<LayoutLine>) {
    if let Some(line) = current.take_line(page) {
        lines.push(line);
    }
}

fn is_new_visual_line(current: &LineBuilder, rect: PdfRect) -> bool {
    let current_center = (current.top + current.bottom) * 0.5;
    let next_center = (rect.top + rect.bottom) * 0.5;
    let tolerance = current
        .average_height()
        .max(rect.height())
        .mul_add(0.85, 0.0)
        .max(3.0);
    (current_center - next_center).abs() > tolerance
}

impl LineBuilder {
    fn has_rect(&self) -> bool {
        self.rect_count > 0
    }

    fn average_height(&self) -> f32 {
        if self.rect_count == 0 {
            0.0
        } else {
            self.height_total / self.rect_count as f32
        }
    }

    fn average_font_size(&self) -> f32 {
        if self.font_size_count == 0 {
            0.0
        } else {
            self.font_size_total / self.font_size_count as f32
        }
    }

    fn push(&mut self, ch: char, rect: Option<PdfRect>) {
        self.text.push(ch);
        let Some(rect) = rect else {
            return;
        };
        if self.rect_count == 0 {
            self.left = rect.left;
            self.bottom = rect.bottom;
            self.right = rect.right;
            self.top = rect.top;
        } else {
            self.left = self.left.min(rect.left);
            self.bottom = self.bottom.min(rect.bottom);
            self.right = self.right.max(rect.right);
            self.top = self.top.max(rect.top);
        }
        self.height_total += rect.height();
        self.rect_count += 1;
    }

    fn push_style(&mut self, item: &PageTextChar) {
        if let Some(size) = item
            .font_size
            .filter(|size| size.is_finite() && *size > 0.0)
        {
            self.font_size_total += size;
            self.font_size_count += 1;
        }
        self.bold_count += usize::from(item.bold);
        self.italic_count += usize::from(item.italic);
        // `push_style` is called immediately after `push(item.ch, …)`, so the char just appended
        // occupies the tail of `self.text`; record its byte span + font size for callout detection.
        let len = item.ch.len_utf8();
        let start = self.text.len().saturating_sub(len);
        self.char_meta.push(CharMeta {
            start,
            len,
            ch: item.ch,
            font_size: item.font_size,
        });
    }

    fn take_line(&mut self, page: &PageInfo) -> Option<LayoutLine> {
        let max_internal_space_run = line_internal_space_run(&self.text);
        let space_density = line_space_density(&self.text);
        let leading_space_count = leading_space_count(&self.text);
        let trailing_space_count = trailing_space_count(&self.text);
        if self.wrap_callouts {
            wrap_superscript_callouts(&mut self.text, &self.char_meta);
        }
        let text = normalize_line_text(&self.text);
        if text.is_empty() || self.rect_count == 0 || page.width <= 0.0 || page.height <= 0.0 {
            self.clear();
            return None;
        }
        let font_height = if self.font_size_count > 0 {
            self.font_size_total / self.font_size_count as f32
        } else {
            self.average_height()
        };
        let style_count = self.font_size_count.max(self.rect_count).max(1);
        let line = LayoutLine {
            text,
            page_index: 0,
            page_width: page.width,
            page_height: page.height,
            line_index: 0,
            left: self.left,
            bottom: self.bottom,
            right: self.right,
            top: self.top,
            font_height,
            font_ratio_page: 1.0,
            font_ratio_page_ref: 1.0,
            font_ratio_doc: 1.0,
            bold: self.bold_count * 2 >= style_count,
            italic: self.italic_count * 2 >= style_count,
            centered: false,
            below_footnote_divider: false,
            distance_below_divider: 0.0,
            page_has_footnote_divider: page.footnote_divider_y_from_top.is_some(),
            sequence_footnote_zone: false,
            prev_line_present: false,
            prev_sequence_footnote_zone: false,
            prev_below_footnote_divider: false,
            prev_small_font: false,
            prev_note_marker: false,
            prev_legal_note_cue: false,
            next_line_present: false,
            next_sequence_footnote_zone: false,
            next_below_footnote_divider: false,
            next_small_font: false,
            next_note_marker: false,
            next_legal_note_cue: false,
            prev_y_gap_ratio: 0.0,
            prev_left_delta_ratio: 0.0,
            prev_font_delta_ratio: 0.0,
            next_y_gap_ratio: 0.0,
            next_left_delta_ratio: 0.0,
            next_font_delta_ratio: 0.0,
            body_left_delta_ratio: 0.0,
            width_to_body_ratio: 1.0,
            prev_gap_to_median_ratio: 1.0,
            next_gap_to_median_ratio: 1.0,
            signed_body_left_delta_ratio: 0.0,
            right_indent_ratio: 0.0,
            center_offset_ratio: 0.0,
            font_ratio_body: 1.0,
            max_internal_space_run,
            space_density,
            leading_space_count,
            trailing_space_count,
            body_column_like: false,
            narrow_measure_like: false,
            hanging_indent_like: false,
            vertically_isolated_like: false,
            heading_geometry_like: false,
            follows_hanging_note_marker: false,
            repeated_header_footer: false,
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
            page_contents_like: false,
            contents_or_index_entry: false,
        };
        self.clear();
        Some(line)
    }

    fn clear(&mut self) {
        self.text.clear();
        self.left = 0.0;
        self.bottom = 0.0;
        self.right = 0.0;
        self.top = 0.0;
        self.height_total = 0.0;
        self.rect_count = 0;
        self.font_size_total = 0.0;
        self.font_size_count = 0;
        self.bold_count = 0;
        self.italic_count = 0;
        self.char_meta.clear();
    }
}

/// Detect inline footnote callouts — short digit runs set in a font clearly smaller than the line's
/// body text (i.e. superscript reference markers) — and wrap them in `CALLOUT_START`/`CALLOUT_END`
/// sentinels on the raw line text (before whitespace normalization). Conservative by design:
/// requires real body content on the line, so standalone/marginal number lines are left alone.
fn wrap_superscript_callouts(text: &mut String, meta: &[CharMeta]) {
    if meta.len() < 6 {
        return;
    }
    let mut sizes: Vec<f32> = meta
        .iter()
        .filter_map(|m| m.font_size)
        .filter(|s| s.is_finite() && *s > 0.0)
        .collect();
    if sizes.len() < 6 {
        return;
    }
    sizes.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let body = sizes[sizes.len() / 2]; // median font size = the body run
    if body <= 0.0 {
        return;
    }
    let small = |m: &CharMeta| {
        m.font_size
            .is_some_and(|s| s.is_finite() && s < body * 0.80)
    };
    // Require substantial body content (chars at/above body size) so we don't superscript a line
    // that is itself a small standalone marker/footnote line.
    let body_visible = meta
        .iter()
        .filter(|m| !m.ch.is_whitespace() && !small(m))
        .count();
    if body_visible < 12 {
        return;
    }
    // Collect maximal runs of small numeric chars (1–4 digits) = superscript callouts.
    let mut runs: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < meta.len() {
        if small(&meta[i]) && meta[i].ch.is_ascii_digit() {
            let start = meta[i].start;
            let mut j = i;
            while j < meta.len() && small(&meta[j]) && meta[j].ch.is_ascii_digit() {
                j += 1;
            }
            let end = meta[j - 1].start + meta[j - 1].len;
            if (1..=4).contains(&(j - i)) {
                runs.push((start, end));
            }
            i = j;
        } else {
            i += 1;
        }
    }
    // Insert sentinels back-to-front so earlier byte offsets stay valid.
    for (s, e) in runs.into_iter().rev() {
        if e <= text.len() && s < e && text.is_char_boundary(s) && text.is_char_boundary(e) {
            text.insert(e, CALLOUT_END);
            text.insert(s, CALLOUT_START);
        }
    }
}

fn normalize_line_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn line_internal_space_run(text: &str) -> usize {
    let mut current = 0usize;
    let mut best = 1usize;
    for ch in text.chars() {
        if ch == ' ' || ch == '\t' {
            current += 1;
            best = best.max(current);
        } else {
            current = 0;
        }
    }
    best
}

fn line_space_density(text: &str) -> f32 {
    let visible = text.chars().filter(|ch| !ch.is_whitespace()).count();
    if visible == 0 {
        0.0
    } else {
        text.chars().filter(|ch| ch.is_whitespace()).count() as f32 / visible as f32
    }
}

fn leading_space_count(text: &str) -> usize {
    text.chars()
        .take_while(|ch| *ch == ' ' || *ch == '\t')
        .count()
}

fn trailing_space_count(text: &str) -> usize {
    text.chars()
        .rev()
        .take_while(|ch| *ch == ' ' || *ch == '\t')
        .count()
}

fn enrich_line_features(
    pages: &[PageInfo],
    page_ranges: &[std::ops::Range<usize>],
    lines: &mut [LayoutLine],
    all_heights: &[f32],
) {
    let mut doc_heights = all_heights
        .iter()
        .copied()
        .filter(|height| height.is_finite() && *height > 1.0)
        .collect::<Vec<_>>();
    doc_heights.sort_by(f32::total_cmp);
    let doc_median = median_sorted(&doc_heights).max(1.0);

    let mut text_counts: HashMap<String, usize> = HashMap::new();
    for line in lines.iter() {
        let key = normalize_model_text(&line.text);
        if !key.is_empty() {
            *text_counts.entry(key).or_default() += 1;
        }
    }

    for (page_index, range) in page_ranges.iter().enumerate() {
        let Some(page) = pages.get(page_index) else {
            continue;
        };
        let mut page_heights = lines[range.clone()]
            .iter()
            .map(|line| line.font_height)
            .filter(|height| height.is_finite() && *height > 1.0 && *height < page.height * 0.08)
            .collect::<Vec<_>>();
        page_heights.sort_by(f32::total_cmp);
        let page_median = if page_heights.is_empty() {
            doc_median
        } else {
            median_sorted(&page_heights).max(1.0)
        };
        let page_ref = if page_heights.is_empty() {
            doc_median
        } else {
            percentile_sorted(&page_heights, 0.75).max(1.0)
        };

        for line in &mut lines[range.clone()] {
            line.font_ratio_page = line.font_height / page_median.max(0.1);
            line.font_ratio_page_ref = line.font_height / page_ref.max(0.1);
            line.font_ratio_doc = line.font_height / doc_median.max(0.1);
            line.centered = ((((line.left + line.right) * 0.5) - (page.width * 0.5)).abs()
                / page.width.max(1.0))
                < 0.09;
            let y0 = y_from_top(page, line);
            line.page_has_footnote_divider = page.footnote_divider_y_from_top.is_some();
            if let Some(divider) = page.footnote_divider_y_from_top {
                line.below_footnote_divider = y0 >= divider + 1.0;
                line.distance_below_divider = if line.below_footnote_divider {
                    (y0 - divider) / page.height.max(1.0)
                } else {
                    0.0
                };
            }
            line.repeated_header_footer =
                is_repeated_header_footer(line, &text_counts, page.height);
        }
        mark_page_context_features(&mut lines[range.clone()]);
        mark_block_geometry_features(&mut lines[range.clone()]);
        mark_sequence_footnote_zones(page, &mut lines[range.clone()]);
        mark_previous_line_context(page, &mut lines[range.clone()]);
        mark_segment_blocks(page, &mut lines[range.clone()]);
    }
}

fn mark_block_geometry_features(lines: &mut [LayoutLine]) {
    for line in lines.iter_mut() {
        line.body_left_delta_ratio = 0.0;
        line.width_to_body_ratio = 1.0;
        line.prev_gap_to_median_ratio = 1.0;
        line.next_gap_to_median_ratio = 1.0;
        line.signed_body_left_delta_ratio = 0.0;
        line.right_indent_ratio = 0.0;
        line.center_offset_ratio = 0.0;
        line.font_ratio_body = 1.0;
        line.body_column_like = false;
        line.narrow_measure_like = false;
        line.hanging_indent_like = false;
        line.vertically_isolated_like = false;
        line.heading_geometry_like = false;
        line.follows_hanging_note_marker = false;
    }

    let mut indices = (0..lines.len()).collect::<Vec<_>>();
    indices.sort_by(|a, b| {
        lines[*a]
            .y0_ratio()
            .total_cmp(&lines[*b].y0_ratio())
            .then_with(|| lines[*a].left.total_cmp(&lines[*b].left))
    });

    let mut body_lefts = Vec::new();
    let mut body_widths = Vec::new();
    let mut body_fonts = Vec::new();
    for index in indices.iter().copied() {
        let line = &lines[index];
        let y0 = line.y0_ratio();
        let width_ratio = (line.right - line.left) / line.page_width.max(1.0);
        if !line.repeated_header_footer
            && !line.page_contents_like
            && !is_plain_page_number_line(&line.text)
            && !is_repository_cover_boilerplate(line)
            && !is_repository_cover_identifier(line)
            && (0.10..=0.88).contains(&y0)
            && (0.88..=1.18).contains(&line.font_ratio_page_ref)
            && width_ratio >= 0.34
        {
            body_lefts.push(line.left);
            body_widths.push(line.right - line.left);
            body_fonts.push(line.font_height);
        }
    }
    body_lefts.sort_by(f32::total_cmp);
    body_widths.sort_by(f32::total_cmp);
    body_fonts.sort_by(f32::total_cmp);
    let body_left = if body_lefts.len() >= 3 {
        median_sorted(&body_lefts)
    } else {
        let mut lefts = indices
            .iter()
            .map(|index| lines[*index].left)
            .collect::<Vec<_>>();
        lefts.sort_by(f32::total_cmp);
        median_sorted(&lefts)
    };
    let body_width = if body_widths.len() >= 3 {
        median_sorted(&body_widths).max(1.0)
    } else {
        indices
            .iter()
            .map(|index| lines[*index].right - lines[*index].left)
            .max_by(f32::total_cmp)
            .unwrap_or(1.0)
            .max(1.0)
    };
    let body_font = if body_fonts.len() >= 3 {
        median_sorted(&body_fonts).max(0.1)
    } else {
        let mut heights = indices
            .iter()
            .map(|index| lines[*index].font_height)
            .filter(|height| height.is_finite() && *height > 0.0)
            .collect::<Vec<_>>();
        heights.sort_by(f32::total_cmp);
        median_sorted(&heights).max(0.1)
    };

    let mut gaps = Vec::new();
    for pair in indices.windows(2) {
        let prev = &lines[pair[0]];
        let current = &lines[pair[1]];
        if current.y0_ratio() >= prev.y0_ratio() {
            gaps.push(((current.y0_ratio() - prev.y0_ratio()) * current.page_height).max(0.0));
        }
    }
    gaps.sort_by(f32::total_cmp);
    let median_gap = median_sorted(&gaps).max(1.0);

    for (position, index) in indices.iter().copied().enumerate() {
        let width = (lines[index].right - lines[index].left).max(0.0);
        let page_width = lines[index].page_width.max(1.0);
        lines[index].signed_body_left_delta_ratio = (lines[index].left - body_left) / page_width;
        lines[index].body_left_delta_ratio = (lines[index].left - body_left).abs() / page_width;
        lines[index].width_to_body_ratio = width / body_width.max(1.0);
        lines[index].right_indent_ratio = (page_width - lines[index].right).max(0.0) / page_width;
        lines[index].center_offset_ratio =
            (((lines[index].left + lines[index].right) * 0.5) - (page_width * 0.5)).abs()
                / page_width;
        lines[index].font_ratio_body = lines[index].font_height / body_font.max(0.1);
        lines[index].body_column_like = lines[index].body_left_delta_ratio <= 0.045
            && (0.72..=1.25).contains(&lines[index].width_to_body_ratio);
        lines[index].narrow_measure_like = lines[index].width_to_body_ratio <= 0.72
            && width / lines[index].page_width.max(1.0) <= 0.50;
        let mut prev_gap = 1.0;
        let mut next_gap = 1.0;
        if position > 0 {
            let prev_index = indices[position - 1];
            let prev = lines[prev_index].clone();
            let line = &mut lines[index];
            let gap = ((line.y0_ratio() - prev.y0_ratio()) * line.page_height).max(0.0);
            prev_gap = gap / median_gap;
            line.prev_gap_to_median_ratio = prev_gap;
            let prev_indent = (line.left - prev.left) / line.page_width.max(1.0);
            line.hanging_indent_like = (0.018..=0.12).contains(&prev_indent)
                && (line.font_ratio_page_ref - prev.font_ratio_page_ref).abs() <= 0.22
                && gap / median_gap <= 2.2
                && (starts_with_note_marker(&prev.text)
                    || starts_with_legal_note_marker(&prev.text));
            line.follows_hanging_note_marker = line.hanging_indent_like
                && prev.font_ratio_page_ref <= 1.02
                && line.font_ratio_page_ref <= 1.02;
        }
        if position + 1 < indices.len() {
            let next_index = indices[position + 1];
            let next_line = lines[next_index].clone();
            let line = &mut lines[index];
            let gap = ((next_line.y0_ratio() - line.y0_ratio()) * line.page_height).max(0.0);
            next_gap = gap / median_gap;
            line.next_gap_to_median_ratio = next_gap;
        }
        let words = model_word_count(&lines[index].text);
        lines[index].vertically_isolated_like = prev_gap >= 1.45 && next_gap >= 1.05;
        lines[index].heading_geometry_like = (1..=18).contains(&words)
            && lines[index].font_ratio_body >= 1.06
            && (lines[index].vertically_isolated_like
                || lines[index].bold
                || lines[index].centered)
            && lines[index].width_to_body_ratio <= 1.05
            && !lines[index].below_footnote_divider
            && !lines[index].sequence_footnote_zone;
    }
}

fn mark_previous_line_context(page: &PageInfo, lines: &mut [LayoutLine]) {
    for line in lines.iter_mut() {
        line.prev_line_present = false;
        line.prev_sequence_footnote_zone = false;
        line.prev_below_footnote_divider = false;
        line.prev_small_font = false;
        line.prev_note_marker = false;
        line.prev_legal_note_cue = false;
        line.next_line_present = false;
        line.next_sequence_footnote_zone = false;
        line.next_below_footnote_divider = false;
        line.next_small_font = false;
        line.next_note_marker = false;
        line.next_legal_note_cue = false;
        line.prev_y_gap_ratio = 0.0;
        line.prev_left_delta_ratio = 0.0;
        line.prev_font_delta_ratio = 0.0;
        line.next_y_gap_ratio = 0.0;
        line.next_left_delta_ratio = 0.0;
        line.next_font_delta_ratio = 0.0;
    }

    let mut indices = (0..lines.len()).collect::<Vec<_>>();
    indices.sort_by(|a, b| {
        y_from_top(page, &lines[*a])
            .total_cmp(&y_from_top(page, &lines[*b]))
            .then_with(|| lines[*a].left.total_cmp(&lines[*b].left))
    });

    for pair in indices.windows(2) {
        let prev_index = pair[0];
        let index = pair[1];
        let prev = lines[prev_index].clone();
        let line = &mut lines[index];
        line.prev_line_present = true;
        line.prev_sequence_footnote_zone = prev.sequence_footnote_zone;
        line.prev_below_footnote_divider = prev.below_footnote_divider;
        line.prev_small_font = prev.font_ratio_page_ref <= 0.92;
        line.prev_note_marker = starts_with_note_marker(&prev.text);
        line.prev_legal_note_cue = contains_legal_note_cue(&prev.text);
        line.prev_y_gap_ratio =
            ((y_from_top(page, line) - y_from_top(page, &prev)) / page.height.max(1.0)).max(0.0);
        line.prev_left_delta_ratio = ((line.left - prev.left).abs() / page.width.max(1.0)).max(0.0);
        line.prev_font_delta_ratio =
            ((line.font_ratio_page_ref - prev.font_ratio_page_ref).abs()).max(0.0);
    }

    for pair in indices.windows(2) {
        let index = pair[0];
        let next_index = pair[1];
        let next = lines[next_index].clone();
        let line = &mut lines[index];
        line.next_line_present = true;
        line.next_sequence_footnote_zone = next.sequence_footnote_zone;
        line.next_below_footnote_divider = next.below_footnote_divider;
        line.next_small_font = next.font_ratio_page_ref <= 0.92;
        line.next_note_marker = starts_with_note_marker(&next.text);
        line.next_legal_note_cue = contains_legal_note_cue(&next.text);
        line.next_y_gap_ratio =
            ((y_from_top(page, &next) - y_from_top(page, line)) / page.height.max(1.0)).max(0.0);
        line.next_left_delta_ratio = ((line.left - next.left).abs() / page.width.max(1.0)).max(0.0);
        line.next_font_delta_ratio =
            ((line.font_ratio_page_ref - next.font_ratio_page_ref).abs()).max(0.0);
    }
}

#[derive(Debug, Clone)]
struct SegmentBlockSummary {
    id: usize,
    start: usize,
    end: usize,
    shape: String,
    toc_like: bool,
    table_like: bool,
    footnote_like: bool,
    furniture_like: bool,
}

fn mark_segment_blocks(page: &PageInfo, lines: &mut [LayoutLine]) {
    for line in lines.iter_mut() {
        line.segment_block_id = 0;
        line.segment_block_line_index = 0;
        line.segment_block_line_count = 1;
        line.segment_block_first = true;
        line.segment_block_last = true;
        line.segment_block_shape = "unknown".to_owned();
        line.segment_block_toc_like = false;
        line.segment_block_table_like = false;
        line.segment_block_footnote_like = false;
        line.segment_block_furniture_like = false;
    }
    if lines.is_empty() {
        return;
    }

    let mut order = (0..lines.len()).collect::<Vec<_>>();
    order.sort_by(|a, b| {
        y_from_top(page, &lines[*a])
            .total_cmp(&y_from_top(page, &lines[*b]))
            .then_with(|| lines[*a].left.total_cmp(&lines[*b].left))
    });

    let mut gaps = Vec::new();
    for pair in order.windows(2) {
        let previous = &lines[pair[0]];
        let current = &lines[pair[1]];
        let gap = (y_from_top(page, current) - y_from_top(page, previous)).max(0.0);
        if gap.is_finite() && gap > 0.1 {
            gaps.push(gap);
        }
    }
    gaps.sort_by(f32::total_cmp);
    let median_gap = median_sorted(&gaps).max(1.0);

    let mut blocks: Vec<Vec<usize>> = Vec::new();
    let mut current = vec![order[0]];
    for pair in order.windows(2) {
        let previous_index = pair[0];
        let current_index = pair[1];
        if starts_new_segment_block(
            &lines[previous_index],
            &lines[current_index],
            page,
            median_gap,
        ) {
            blocks.push(current);
            current = Vec::new();
        }
        current.push(current_index);
    }
    blocks.push(current);

    for (block_id, block) in blocks.iter().enumerate() {
        let summary = summarize_segment_block(block_id, block, lines);
        let count = summary.end.saturating_sub(summary.start);
        for (position, index) in block.iter().copied().enumerate() {
            lines[index].segment_block_id = summary.id;
            lines[index].segment_block_line_index = position;
            lines[index].segment_block_line_count = count;
            lines[index].segment_block_first = position == 0;
            lines[index].segment_block_last = position + 1 == count;
            lines[index].segment_block_shape = summary.shape.clone();
            lines[index].segment_block_toc_like = summary.toc_like;
            lines[index].segment_block_table_like = summary.table_like;
            lines[index].segment_block_footnote_like = summary.footnote_like;
            lines[index].segment_block_furniture_like = summary.furniture_like;
        }
    }
}

fn starts_new_segment_block(
    previous: &LayoutLine,
    current: &LayoutLine,
    page: &PageInfo,
    median_gap: f32,
) -> bool {
    let gap = (y_from_top(page, current) - y_from_top(page, previous)).max(0.0);
    let gap_ratio = gap / median_gap.max(1.0);
    let left_delta = (current.left - previous.left).abs() / page.width.max(1.0);
    let font_delta = (current.font_ratio_page_ref - previous.font_ratio_page_ref).abs();
    let same_row_column_jump = gap <= median_gap * 0.35 && left_delta >= 0.18;
    let large_gap = gap_ratio >= 1.75 || gap / page.height.max(1.0) >= 0.028;
    let major_alignment_change = left_delta >= 0.14 && gap_ratio >= 0.75;
    let font_run_break = font_delta >= 0.22 && gap_ratio >= 0.65;
    let footnote_transition = previous.sequence_footnote_zone != current.sequence_footnote_zone
        || previous.below_footnote_divider != current.below_footnote_divider;
    let furniture_boundary = previous.repeated_header_footer != current.repeated_header_footer;
    let contents_boundary = previous.page_contents_like != current.page_contents_like
        || previous.contents_or_index_entry != current.contents_or_index_entry;
    let table_boundary =
        segment_table_line_like(previous) != segment_table_line_like(current) && gap_ratio >= 0.65;

    same_row_column_jump
        || large_gap
        || major_alignment_change
        || font_run_break
        || footnote_transition
        || furniture_boundary
        || contents_boundary
        || table_boundary
}

fn segment_table_line_like(line: &LayoutLine) -> bool {
    is_table_line(&line.text)
        || looks_like_numeric_table_cell_fragment(line)
        || (line.max_internal_space_run >= 5 && line.space_density >= 0.18)
        || (line.narrow_measure_like
            && line.leading_space_count > 0
            && line.trailing_space_count > 0)
}

fn summarize_segment_block(
    block_id: usize,
    block: &[usize],
    lines: &[LayoutLine],
) -> SegmentBlockSummary {
    let count = block.len().max(1);
    let toc_count = block
        .iter()
        .filter(|index| {
            let line = &lines[**index];
            line.page_contents_like
                || line.contents_or_index_entry
                || looks_like_dot_leader_contents_line(&line.text)
                || looks_like_dot_leader_contents_fragment(&line.text)
        })
        .count();
    let table_count = block
        .iter()
        .filter(|index| {
            let line = &lines[**index];
            segment_table_line_like(line)
        })
        .count();
    let footnote_count = block
        .iter()
        .filter(|index| {
            let line = &lines[**index];
            line.below_footnote_divider
                || line.sequence_footnote_zone
                || starts_with_note_marker(&line.text)
                || contains_legal_note_cue(&line.text)
        })
        .count();
    let furniture_count = block
        .iter()
        .filter(|index| {
            let line = &lines[**index];
            line.repeated_header_footer
                || is_plain_page_number_line(&line.text)
                || is_repository_cover_boilerplate(line)
                || is_repository_cover_identifier(line)
        })
        .count();
    let body_count = block
        .iter()
        .filter(|index| {
            let line = &lines[**index];
            line.body_column_like && !line.page_contents_like && !line.repeated_header_footer
        })
        .count();
    let heading_count = block
        .iter()
        .filter(|index| lines[**index].heading_geometry_like)
        .count();

    let toc_like = toc_count * 2 >= count;
    let table_like = table_count * 2 >= count;
    let footnote_like = footnote_count * 2 >= count;
    let furniture_like = furniture_count * 2 >= count;
    let body_like = body_count * 2 >= count;
    let heading_like = heading_count > 0 && count <= 3;
    let shape = if furniture_like {
        "furniture"
    } else if toc_like {
        "toc_or_index"
    } else if table_like {
        "table"
    } else if footnote_like {
        "footnote"
    } else if heading_like {
        "heading"
    } else if body_like {
        "body"
    } else {
        "mixed"
    }
    .to_owned();

    SegmentBlockSummary {
        id: block_id,
        start: 0,
        end: count,
        shape,
        toc_like,
        table_like,
        footnote_like,
        furniture_like,
    }
}

fn mark_page_context_features(lines: &mut [LayoutLine]) {
    let mut entries = 0usize;
    let mut dot_entries = 0usize;
    let mut dot_fragments = 0usize;
    let mut case_entries = 0usize;
    let mut split_name_index_entries = 0usize;
    let mut headings = 0usize;
    let mut right_numbers = 0usize;

    for line in lines.iter_mut() {
        line.page_contents_like = false;
        let standard_entry = looks_like_contents_or_index_entry_text(&line.text);
        let split_name_entry = looks_like_split_name_index_entry(&line.text);
        line.contents_or_index_entry = standard_entry || split_name_entry;
        entries += usize::from(standard_entry);
        split_name_index_entries += usize::from(split_name_entry);
        dot_entries += usize::from(looks_like_dot_leader_contents_line(&line.text));
        dot_fragments += usize::from(looks_like_dot_leader_contents_fragment(&line.text));
        case_entries += usize::from(looks_like_case_index_entry(&line.text));
        headings += usize::from(looks_like_contents_heading_text(&line.text));
        right_numbers += usize::from(
            is_plain_page_number_line(&line.text) && line.left / line.page_width.max(1.0) >= 0.62,
        );
    }

    let page_like = entries >= 5
        || dot_entries >= 3
        || dot_fragments >= 6
        || dot_fragments >= 3 && right_numbers >= 2
        || case_entries >= 5
        || headings >= 1 && entries >= 2
        || headings >= 1 && dot_fragments >= 2
        || right_numbers >= 4 && entries >= 2
        || split_name_index_entries >= 4 && right_numbers >= 3;
    if page_like {
        for line in lines {
            line.page_contents_like = true;
        }
    }
}

fn mark_sequence_footnote_zones(page: &PageInfo, lines: &mut [LayoutLine]) {
    for line in lines.iter_mut() {
        line.sequence_footnote_zone = false;
    }

    let mut indices = (0..lines.len()).collect::<Vec<_>>();
    indices.sort_by(|a, b| {
        y_from_top(page, &lines[*a])
            .total_cmp(&y_from_top(page, &lines[*b]))
            .then_with(|| lines[*a].left.total_cmp(&lines[*b].left))
    });

    let mut in_footnote_zone = false;
    for (position, index) in indices.iter().copied().enumerate() {
        if lines[index].repeated_header_footer {
            continue;
        }
        let starts_zone = starts_sequence_footnote_zone(&lines[index])
            || starts_split_numeric_footnote_zone(lines, &indices, position)
            || starts_midpage_indented_note_quote_run(lines, &indices, position)
            || starts_small_font_legal_note_run(lines, &indices, position)
            || starts_contextual_citation_footnote_zone(lines, &indices, position)
            || starts_fragmented_publication_footnote_zone(lines, &indices, position);
        if starts_zone {
            in_footnote_zone = true;
        }
        if in_footnote_zone && can_continue_sequence_footnote_zone(&lines[index]) {
            lines[index].sequence_footnote_zone = true;
        }
    }
}

fn starts_sequence_footnote_zone(line: &LayoutLine) -> bool {
    let note_marker = starts_with_note_marker(&line.text);
    let symbol_note_marker = starts_with_symbol_note_marker(&line.text);
    let compact_legal_note_marker = starts_with_compact_legal_note_marker(&line.text);
    let legal_note_marker = starts_with_legal_note_marker(&line.text);
    let general_citation_note_start = looks_like_general_citation_note_start(&line.text);
    if line.y0_ratio() < 0.22 {
        return false;
    }
    if line.y0_ratio() < 0.24
        && !(note_marker
            || compact_legal_note_marker
            || legal_note_marker
            || general_citation_note_start)
    {
        return false;
    }
    if is_repository_cover_boilerplate(line)
        || is_repository_cover_identifier(line)
        || line.page_contents_like
        || is_disposable_contents_or_index_line(line)
        || is_plain_page_number_line(&line.text)
        || starts_with_numeric_lowercase_body_fragment(&line.text)
        || (!note_marker && looks_like_clear_section_heading(&line.text))
    {
        return false;
    }
    let legal_note_cue = contains_legal_note_cue(&line.text);
    let strong_legal_note_cue = contains_strong_legal_note_cue(&line.text);
    if line.page_index == 0
        && symbol_note_marker
        && line.y0_ratio() >= 0.35
        && line.font_ratio_page_ref <= 0.98
    {
        return true;
    }
    if line.below_footnote_divider {
        return line.font_ratio_page_ref <= 1.05 && (note_marker || legal_note_cue);
    }
    if compact_legal_note_marker && line.y0_ratio() >= 0.25 && line.font_ratio_page_ref <= 1.02 {
        return true;
    }
    if legal_note_marker && line.y0_ratio() >= 0.25 && line.font_ratio_page_ref <= 1.02 {
        return true;
    }
    if general_citation_note_start && line.y0_ratio() >= 0.22 && line.font_ratio_page_ref <= 1.02 {
        return true;
    }
    if general_citation_note_start && line.y0_ratio() >= 0.25 && line.font_ratio_page_ref <= 1.02 {
        return true;
    }
    if line.font_ratio_page_ref > 0.98 {
        return false;
    }
    if !line.below_footnote_divider && line.y0_ratio() < 0.55 {
        return false;
    }
    note_marker || strong_legal_note_cue
}

fn starts_split_numeric_footnote_zone(
    lines: &[LayoutLine],
    indices: &[usize],
    position: usize,
) -> bool {
    let line = &lines[indices[position]];
    if !is_bare_numeric_note_marker(&line.text)
        || line.page_has_footnote_divider
        || line.repeated_header_footer
        || is_repository_cover_boilerplate(line)
        || is_repository_cover_identifier(line)
        || line.page_contents_like
        || is_disposable_contents_or_index_line(line)
        || line.y0_ratio() < 0.20
        || line.y0_ratio() > 0.90
        || line.font_ratio_page_ref > 1.02
    {
        return false;
    }

    let mut saw_citation_confirmation = false;
    for next_index in indices.iter().skip(position + 1).take(10).copied() {
        let next = &lines[next_index];
        if next.repeated_header_footer {
            continue;
        }
        if is_repository_cover_boilerplate(next)
            || is_repository_cover_identifier(next)
            || next.page_contents_like
            || is_disposable_contents_or_index_line(next)
            || looks_like_clear_section_heading(&next.text)
            || is_table_line(&next.text)
            || is_caption_line(&next.text)
            || next.y0_ratio() - line.y0_ratio() > 0.18
            || next.font_ratio_page_ref > 1.12
        {
            break;
        }
        if is_plain_page_number_line(&next.text) {
            continue;
        }

        let bridge = can_bridge_generic_contextual_footnote_line(next)
            || starts_with_note_marker(&next.text)
            || looks_like_citation_continuation_text(&next.text)
            || looks_like_footnote_bibliographic_lead_text(&next.text);
        if !bridge {
            break;
        }

        saw_citation_confirmation |= starts_with_legal_note_marker(&next.text)
            || looks_like_general_citation_note_start(&next.text)
            || looks_like_citation_continuation_text(&next.text)
            || looks_like_publication_citation_continuation_text(&next.text)
            || contains_strong_legal_note_cue(&next.text);

        if saw_citation_confirmation {
            return true;
        }
    }
    false
}

fn starts_midpage_indented_note_quote_run(
    lines: &[LayoutLine],
    indices: &[usize],
    position: usize,
) -> bool {
    let line = &lines[indices[position]];
    if line.page_has_footnote_divider
        || line.sequence_footnote_zone
        || line.repeated_header_footer
        || is_repository_cover_boilerplate(line)
        || is_repository_cover_identifier(line)
        || line.page_contents_like
        || is_disposable_contents_or_index_line(line)
        || is_plain_page_number_line(&line.text)
        || starts_with_numeric_lowercase_body_fragment(&line.text)
        || !starts_with_note_marker(&line.text)
        || line.y0_ratio() < 0.45
        || line.y0_ratio() > 0.72
        || line.font_ratio_page_ref > 0.86
    {
        return false;
    }

    let mut run_lines = 0usize;
    let mut indented_lines = 0usize;
    let mut saw_confirmation = looks_like_general_citation_note_start(&line.text)
        || looks_like_citation_continuation_text(&line.text)
        || looks_like_footnote_bibliographic_lead_text(&line.text)
        || contains_strong_legal_note_cue(&line.text);
    for next_index in indices.iter().skip(position + 1).take(12).copied() {
        let next = &lines[next_index];
        if next.repeated_header_footer
            || is_repository_cover_boilerplate(next)
            || is_repository_cover_identifier(next)
            || next.page_contents_like
            || is_disposable_contents_or_index_line(next)
            || is_plain_page_number_line(&next.text)
            || next.y0_ratio() - line.y0_ratio() > 0.18
            || next.font_ratio_page_ref > 0.92
        {
            break;
        }
        let confirmation = starts_with_legal_note_marker(&next.text)
            || looks_like_general_citation_note_start(&next.text)
            || looks_like_citation_continuation_text(&next.text)
            || looks_like_publication_citation_continuation_text(&next.text)
            || looks_like_footnote_bibliographic_lead_text(&next.text)
            || contains_strong_legal_note_cue(&next.text);
        if (looks_like_clear_section_heading(&next.text)
            || is_table_line(&next.text)
            || is_caption_line(&next.text))
            && !confirmation
        {
            break;
        }
        if !can_bridge_generic_contextual_footnote_line(next) && !confirmation {
            break;
        }

        run_lines += 1;
        if (next.left - line.left) / line.page_width.max(1.0) >= 0.035 {
            indented_lines += 1;
        }
        saw_confirmation |= confirmation;
        if saw_confirmation && run_lines >= 3 && indented_lines >= 2 {
            return true;
        }
    }
    false
}

fn can_continue_sequence_footnote_zone(line: &LayoutLine) -> bool {
    !line.page_contents_like
        && !is_disposable_contents_or_index_line(line)
        && line.y0_ratio() < 0.94
        && line.font_ratio_page_ref <= 1.02
}

fn starts_small_font_legal_note_run(
    lines: &[LayoutLine],
    indices: &[usize],
    position: usize,
) -> bool {
    let line = &lines[indices[position]];
    if line.page_has_footnote_divider
        || line.sequence_footnote_zone
        || line.repeated_header_footer
        || is_repository_cover_boilerplate(line)
        || is_repository_cover_identifier(line)
        || line.page_contents_like
        || is_disposable_contents_or_index_line(line)
        || looks_like_clear_section_heading(&line.text)
        || line.y0_ratio() < 0.30
        || line.y0_ratio() > 0.62
        || line.font_ratio_page_ref > 0.82
    {
        return false;
    }

    let mut run_lines = 0usize;
    let mut small_lines = 0usize;
    let mut evidence_lines = 0usize;
    let mut early_evidence = false;
    let mut early_marker_evidence = false;
    for (offset, next_index) in indices.iter().skip(position).take(14).copied().enumerate() {
        let next = &lines[next_index];
        if next.y0_ratio() - line.y0_ratio() > 0.22
            || next.repeated_header_footer
            || is_repository_cover_boilerplate(next)
            || is_repository_cover_identifier(next)
            || next.page_contents_like
            || is_disposable_contents_or_index_line(next)
            || looks_like_clear_section_heading(&next.text)
            || next.font_ratio_page_ref > 1.02
        {
            break;
        }

        run_lines += 1;
        small_lines += usize::from(next.font_ratio_page_ref <= 0.92);
        if has_small_font_legal_note_run_evidence(next) {
            evidence_lines += 1;
            early_evidence |= offset < 5;
        }
        early_marker_evidence |= offset < 6
            && (starts_with_legal_note_marker(&next.text)
                || starts_with_symbol_note_marker(&next.text)
                || is_bare_numeric_note_marker(&next.text)
                || plain_numeric_line_digit_count(&next.text).is_some_and(|digits| digits <= 3));
    }

    run_lines >= 4
        && small_lines >= 4
        && evidence_lines >= 2
        && early_evidence
        && early_marker_evidence
}

fn has_small_font_legal_note_run_evidence(line: &LayoutLine) -> bool {
    if plain_numeric_line_digit_count(&line.text).is_some_and(|digits| digits <= 3) {
        return true;
    }
    let lower = line.text.to_ascii_lowercase();
    looks_like_general_citation_note_start(&line.text)
        || looks_like_publication_citation_continuation_text(&line.text)
        || contains_strong_legal_note_cue(&line.text)
        || looks_like_reporter_citation_text(&line.text)
        || looks_like_statutory_citation_fragment(&line.text)
        || lower.contains("http://")
        || lower.contains("https://")
        || lower.contains("perma.cc")
        || lower.contains("supra note")
        || lower.contains("hereinafter")
}

fn starts_contextual_citation_footnote_zone(
    lines: &[LayoutLine],
    indices: &[usize],
    position: usize,
) -> bool {
    let line = &lines[indices[position]];
    if line.page_has_footnote_divider
        || line.repeated_header_footer
        || is_repository_cover_boilerplate(line)
        || is_repository_cover_identifier(line)
        || line.page_contents_like
        || is_disposable_contents_or_index_line(line)
        || is_plain_page_number_line(&line.text)
        || looks_like_clear_section_heading(&line.text)
        || starts_with_quoted_citation_fragment(&line.text)
        || (starts_with_lowercase_letter(&line.text)
            && !looks_like_citation_continuation_text(&line.text))
        || line.y0_ratio() < 0.24
        || line.y0_ratio() > 0.72
        || line.font_ratio_page_ref > 1.02
        || (!looks_like_citation_continuation_text(&line.text)
            && !looks_like_footnote_bibliographic_lead_text(&line.text))
    {
        return false;
    }

    let mut saw_citation_confirmation = looks_like_citation_continuation_text(&line.text);
    let mut generic_bridge_count = 0usize;
    for next_index in indices.iter().skip(position + 1).take(13).copied() {
        let next = &lines[next_index];
        if next.repeated_header_footer {
            continue;
        }
        if is_repository_cover_boilerplate(next)
            || is_repository_cover_identifier(next)
            || next.page_contents_like
            || is_disposable_contents_or_index_line(next)
        {
            return false;
        }
        if starts_sequence_footnote_zone(next)
            || starts_with_legal_note_marker(&next.text)
            || looks_like_general_citation_note_start(&next.text)
        {
            return true;
        }
        if can_bridge_contextual_citation_footnote_zone(next) {
            saw_citation_confirmation |= looks_like_citation_continuation_text(&next.text);
            continue;
        }
        if saw_citation_confirmation && can_bridge_generic_contextual_footnote_line(next) {
            generic_bridge_count += 1;
            if generic_bridge_count <= 4 {
                continue;
            }
        }
        return false;
    }
    false
}

fn starts_with_quoted_citation_fragment(text: &str) -> bool {
    let stripped = text.trim_start();
    let Some(rest) = stripped
        .strip_prefix('"')
        .or_else(|| stripped.strip_prefix('\''))
    else {
        return false;
    };
    let rest = rest.trim_start();
    let lower = rest.to_ascii_lowercase();
    if lower.starts_with("id.") || lower.starts_with("ibid") {
        return true;
    }
    if starts_with_reporter_abbreviation(&lower) {
        return true;
    }
    if let Some(body) = numeric_note_marker_body(rest) {
        return starts_with_reporter_abbreviation(&body.to_ascii_lowercase());
    }
    false
}

fn starts_fragmented_publication_footnote_zone(
    lines: &[LayoutLine],
    indices: &[usize],
    position: usize,
) -> bool {
    let line = &lines[indices[position]];
    if line.page_has_footnote_divider
        || line.repeated_header_footer
        || line.page_contents_like
        || is_repository_cover_boilerplate(line)
        || is_repository_cover_identifier(line)
        || is_disposable_contents_or_index_line(line)
        || is_plain_page_number_line(&line.text)
        || looks_like_clear_section_heading(&line.text)
        || line.y0_ratio() < 0.54
        || line.y0_ratio() > 0.90
        || line.font_ratio_page_ref > 0.92
        || !looks_like_fragmented_publication_note_piece(&line.text)
    {
        return false;
    }

    let mut saw_confirmation = looks_like_fragmented_publication_confirmation(&line.text);
    let mut bridge_count = 0usize;
    for next_index in indices.iter().skip(position + 1).take(11).copied() {
        let next = &lines[next_index];
        if next.repeated_header_footer {
            continue;
        }
        if next.page_contents_like
            || is_repository_cover_boilerplate(next)
            || is_repository_cover_identifier(next)
            || is_disposable_contents_or_index_line(next)
            || is_plain_page_number_line(&next.text)
            || looks_like_clear_section_heading(&next.text)
            || next.y0_ratio() < 0.50
            || next.y0_ratio() > 0.94
            || next.font_ratio_page_ref > 0.95
        {
            break;
        }
        if looks_like_fragmented_publication_confirmation(&next.text) {
            saw_confirmation = true;
        }
        if looks_like_fragmented_publication_note_piece(&next.text) {
            bridge_count += 1;
            if saw_confirmation && bridge_count >= 1 {
                return true;
            }
            if bridge_count <= 8 {
                continue;
            }
        }
        break;
    }
    saw_confirmation && bridge_count >= 2
}

fn looks_like_fragmented_publication_note_piece(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.len() > 140 || is_contents_line(trimmed) {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    let words = word_count(trimmed);
    let letters = trimmed
        .chars()
        .filter(|ch| ch.is_ascii_alphabetic())
        .count();
    let uppercase = trimmed.chars().filter(|ch| ch.is_ascii_uppercase()).count();
    let upper_ratio = if letters > 0 {
        uppercase as f32 / letters as f32
    } else {
        0.0
    };
    looks_like_fragmented_publication_confirmation(trimmed)
        || [
            "jan.", "feb.", "mar.", "apr.", "may", "jun.", "jul.", "aug.", "sep.", "sept.", "oct.",
            "nov.", "dec.",
        ]
        .iter()
        .any(|prefix| lower.starts_with(prefix) || lower.starts_with(&format!("({prefix}")))
        || looks_like_short_numeric_date_fragment(trimmed)
        || (words <= 5 && upper_ratio >= 0.65 && trimmed.contains('.'))
        || (words <= 3 && upper_ratio >= 0.85 && letters >= 4)
}

fn looks_like_fragmented_publication_confirmation(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("http://")
        || lower.contains("https://")
        || lower.contains("perma.cc")
        || lower.contains("washingtonpost")
        || looks_like_publication_citation_continuation_text(text)
}

fn looks_like_short_numeric_date_fragment(text: &str) -> bool {
    let trimmed = text.trim().trim_matches(|ch| matches!(ch, ',' | ')' | '('));
    let digits = trimmed.chars().filter(|ch| ch.is_ascii_digit()).count();
    !trimmed.is_empty()
        && digits == trimmed.chars().count()
        && (digits <= 2 || digits == 4 && matches!(trimmed.chars().next(), Some('1' | '2')))
}

fn can_bridge_contextual_citation_footnote_zone(line: &LayoutLine) -> bool {
    line.font_ratio_page_ref <= 1.08
        && !line.page_contents_like
        && !is_disposable_contents_or_index_line(line)
        && !is_plain_page_number_line(&line.text)
        && (looks_like_citation_continuation_text(&line.text)
            || looks_like_publication_citation_continuation_text(&line.text)
            || looks_like_footnote_bibliographic_lead_text(&line.text))
}

fn can_bridge_generic_contextual_footnote_line(line: &LayoutLine) -> bool {
    line.font_ratio_page_ref <= 1.08
        && !line.page_contents_like
        && !is_disposable_contents_or_index_line(line)
        && line.y0_ratio() < 0.94
        && !is_plain_page_number_line(&line.text)
        && word_count(&line.text) <= 34
        && !is_repository_cover_boilerplate(line)
        && !is_repository_cover_identifier(line)
        && !looks_like_clear_section_heading(&line.text)
        && !is_table_line(&line.text)
        && !is_caption_line(&line.text)
}

fn extend_heuristic_footnote_hints(
    hints: &mut Vec<LiquidLayoutHint>,
    page: &PageInfo,
    lines: &[LayoutLine],
) {
    let mut sorted = lines
        .iter()
        .filter(|line| !line.text.trim().is_empty())
        .collect::<Vec<_>>();
    sorted.sort_by(|a, b| {
        y_from_top(page, a)
            .total_cmp(&y_from_top(page, b))
            .then_with(|| a.left.total_cmp(&b.left))
    });

    let mut heights = sorted
        .iter()
        .map(|line| line.font_height)
        .filter(|height| height.is_finite() && *height > 1.0 && *height < page.height * 0.08)
        .collect::<Vec<_>>();
    if heights.len() < 4 {
        return;
    }
    heights.sort_by(f32::total_cmp);
    let body_ref = percentile_sorted(&heights, 0.75).max(median_sorted(&heights));
    if body_ref <= 0.0 {
        return;
    }

    let mut in_footnote_zone = false;
    for line in sorted {
        let line_y_from_top = y_from_top(page, line);
        let y_frac = line_y_from_top / page.height.max(1.0);
        let small = line.font_height <= body_ref * 0.91;
        let smallish = line.font_height <= body_ref * 0.97;
        let below_divider = page
            .footnote_divider_y_from_top
            .is_some_and(|divider| line_y_from_top >= divider + 1.0);
        let footer_like = y_frac > 0.94
            && word_count(&line.text) <= 5
            && !starts_with_note_marker(&line.text)
            && !contains_legal_note_cue(&line.text);
        let plain_page_number = is_plain_page_number_line(&line.text);
        let repository_boilerplate = is_repository_cover_boilerplate(line);
        let contents_like = line.page_contents_like;
        let clear_heading = looks_like_clear_section_heading(&line.text)
            && !starts_with_note_marker(&line.text)
            && !contains_legal_note_cue(&line.text);

        if in_footnote_zone {
            if repository_boilerplate
                || contents_like
                || footer_like
                || plain_page_number
                || clear_heading
                || y_frac < 0.20
                || (!smallish && !below_divider)
            {
                in_footnote_zone = false;
            }
        }

        if !in_footnote_zone
            && !repository_boilerplate
            && !contents_like
            && !plain_page_number
            && !clear_heading
            && (is_probable_footnote_start(&line.text, small, y_frac, below_divider)
                || is_probable_divider_footnote_line(&line.text, smallish, below_divider))
        {
            in_footnote_zone = true;
        }

        if in_footnote_zone
            && !repository_boilerplate
            && !contents_like
            && !footer_like
            && !plain_page_number
            && !clear_heading
            && line.text.chars().count() >= 3
        {
            push_unique_hint(hints, &line.text, LiquidBlockRole::Marginalia);
        }
    }
}

fn is_probable_footnote_start(text: &str, small: bool, y_frac: f32, below_divider: bool) -> bool {
    if !small || y_frac <= 0.22 {
        return false;
    }
    let note_marker = starts_with_note_marker(text);
    let compact_legal_note_marker = starts_with_compact_legal_note_marker(text);
    let general_citation_note_start = looks_like_general_citation_note_start(text);
    let legal_note_cue = contains_legal_note_cue(text);
    legal_note_cue
        || general_citation_note_start
        || note_marker && (below_divider || y_frac >= 0.55 || compact_legal_note_marker)
}

fn is_probable_divider_footnote_line(text: &str, smallish: bool, below_divider: bool) -> bool {
    below_divider && smallish && word_count(text) <= 42
}

fn extend_repository_cover_hints(hints: &mut Vec<LiquidLayoutHint>, lines: &[LayoutLine]) {
    for line in lines {
        if is_repository_cover_boilerplate(line) || is_repository_cover_identifier(line) {
            push_unique_hint(hints, &line.text, LiquidBlockRole::Noise);
        }
    }
}

fn extend_page_contents_noise_hints(hints: &mut Vec<LiquidLayoutHint>, lines: &[LayoutLine]) {
    let first_contents_line = lines
        .iter()
        .filter(|line| line.page_contents_like)
        .filter(|line| {
            line.contents_or_index_entry
                || looks_like_contents_heading_text(&line.text)
                || looks_like_dot_leader_contents_fragment(&line.text)
        })
        .map(|line| line.line_index)
        .min();
    let Some(first_contents_line) = first_contents_line else {
        return;
    };
    for line in lines {
        if line.page_contents_like
            && line.line_index >= first_contents_line
            && !line.text.trim().is_empty()
        {
            push_unique_hint(hints, &line.text, LiquidBlockRole::Noise);
        }
    }
}

fn extend_model_hints(
    hints: &mut Vec<LiquidLayoutHint>,
    page: &PageInfo,
    lines: &[LayoutLine],
    model: &LayoutRoleModel,
) {
    for line in lines {
        if looks_like_edge_running_header_footer_fragment(line) {
            push_unique_hint(hints, &line.text, LiquidBlockRole::Noise);
            continue;
        }
        if is_repository_cover_boilerplate(line) {
            continue;
        }
        if looks_like_running_law_review_cite_line(&line.text) {
            push_unique_hint(hints, &line.text, LiquidBlockRole::Noise);
            continue;
        }
        if looks_like_nonlegal_study_prompt_noise(line) {
            push_unique_hint(hints, &line.text, LiquidBlockRole::Noise);
            continue;
        }
        let Some(role) = model.predict(line) else {
            continue;
        };
        let hinted_role = if model_line_should_be_marginalia(line) {
            Some(LiquidBlockRole::Marginalia)
        } else {
            match role {
                "footnote" if footnote_specialist_line_can_be_marginalia(line) => {
                    Some(LiquidBlockRole::Marginalia)
                }
                "footnote" => None,
                "noise" if model_noise_line_can_be_hidden(page, line) => {
                    Some(LiquidBlockRole::Noise)
                }
                "noise" => None,
                "front_matter" => Some(LiquidBlockRole::Metadata),
                "visual" => Some(LiquidBlockRole::Table),
                "contents" => Some(LiquidBlockRole::Contents),
                "header_footer" => Some(if y_from_top(page, line) / page.height.max(1.0) <= 0.50 {
                    LiquidBlockRole::Header
                } else {
                    LiquidBlockRole::Footer
                }),
                "table" => Some(LiquidBlockRole::Table),
                "caption" => Some(LiquidBlockRole::Caption),
                "list_item" => Some(LiquidBlockRole::ListItem),
                "metadata" => Some(LiquidBlockRole::Metadata),
                _ => None,
            }
        };
        if let Some(role) = hinted_role {
            push_unique_hint(hints, &line.text, role);
        }
    }
}

fn extend_liquid_core_model_hints(
    hints: &mut Vec<LiquidLayoutHint>,
    page: &PageInfo,
    lines: &[LayoutLine],
    model: &LayoutRoleModel,
) {
    for line in lines {
        if looks_like_nonlegal_study_prompt_noise(line) {
            push_unique_hint(hints, &line.text, LiquidBlockRole::Noise);
            continue;
        }
        let Some(role) = model.predict(line) else {
            continue;
        };
        match role {
            "noise" if model_noise_line_can_be_hidden(page, line) => {
                push_unique_hint(hints, &line.text, LiquidBlockRole::Noise);
            }
            "footnote" if footnote_specialist_line_can_be_marginalia(line) => {
                push_unique_hint(hints, &line.text, LiquidBlockRole::Marginalia);
            }
            _ => {}
        }
    }
}

fn extend_footnote_specialist_hints(
    hints: &mut Vec<LiquidLayoutHint>,
    lines: &[LayoutLine],
    model: &LayoutRoleModel,
) {
    for line in lines {
        if !footnote_specialist_line_can_be_marginalia(line) {
            continue;
        }
        if model.predict_footnote_specialist(line) == Some("footnote") {
            push_unique_hint(hints, &line.text, LiquidBlockRole::Marginalia);
        }
    }
}

fn extend_body_specialist_hints(
    hints: &mut Vec<LiquidLayoutHint>,
    lines: &[LayoutLine],
    model: &LayoutRoleModel,
) {
    for line in lines {
        if !body_specialist_line_can_be_paragraph(line) {
            continue;
        }
        if model.predict(line) == Some("body") {
            push_unique_hint(hints, &line.text, LiquidBlockRole::Paragraph);
        }
    }
}

fn extend_heading_specialist_hints(
    hints: &mut Vec<LiquidLayoutHint>,
    lines: &[LayoutLine],
    model: &LayoutRoleModel,
) {
    for line in lines {
        if !heading_specialist_line_can_be_heading(line) {
            continue;
        }
        let mut tokens = feature_tokens(line);
        tokens.extend(heading_specialist_stack_tokens(line));
        if model.predict_from_tokens(tokens) == Some("heading") {
            push_unique_hint(hints, &line.text, LiquidBlockRole::Heading);
        }
    }
}

fn extend_header_footer_specialist_hints(
    hints: &mut Vec<LiquidLayoutHint>,
    page: &PageInfo,
    lines: &[LayoutLine],
    model: &LayoutRoleModel,
) {
    for line in lines {
        if !header_footer_specialist_line_can_be_header_footer(line) {
            continue;
        }
        if model.predict_header_footer_specialist(line) == Some("header_footer") {
            let role = if y_from_top(page, line) / page.height.max(1.0) <= 0.50 {
                LiquidBlockRole::Header
            } else {
                LiquidBlockRole::Footer
            };
            push_unique_hint(hints, &line.text, role);
        }
    }
}

fn extend_decoded_footnote_run_hints(hints: &mut Vec<LiquidLayoutHint>, lines: &[LayoutLine]) {
    let mut indices = (0..lines.len()).collect::<Vec<_>>();
    indices.sort_by(|a, b| {
        lines[*a]
            .y0_ratio()
            .total_cmp(&lines[*b].y0_ratio())
            .then_with(|| lines[*a].left.total_cmp(&lines[*b].left))
    });

    for (position, index) in indices.iter().copied().enumerate() {
        let line = &lines[index];
        let prev = position
            .checked_sub(1)
            .and_then(|prev_position| indices.get(prev_position))
            .map(|prev_index| &lines[*prev_index]);
        let next = indices
            .get(position + 1)
            .map(|next_index| &lines[*next_index]);
        if let Some(role) = hint_role_for_line(hints, line) {
            let metadata_credential_repair = role == LiquidBlockRole::Metadata
                && contents_like_credential_before_note_run_can_be_marginalia(hints, line, next);
            let metadata_credential_continuation_repair = role == LiquidBlockRole::Metadata
                && metadata_credential_continuation_after_marginalia_can_be_marginalia(
                    hints, line, prev,
                );
            if !matches!(
                role,
                LiquidBlockRole::Header
                    | LiquidBlockRole::Footer
                    | LiquidBlockRole::Table
                    | LiquidBlockRole::Caption
                    | LiquidBlockRole::ListItem
            ) && !metadata_credential_repair
                && !metadata_credential_continuation_repair
            {
                continue;
            }
        }
        let window_start = position.saturating_sub(4);
        let window_end = (position + 5).min(indices.len());
        let nearby = indices[window_start..window_end]
            .iter()
            .copied()
            .filter(|nearby_index| *nearby_index != index)
            .map(|nearby_index| &lines[nearby_index])
            .collect::<Vec<_>>();
        if dense_citation_prelude_row_above_footnote_sequence(line, lines)
            || sequence_citation_row_fragment_can_be_marginalia(line, lines)
            || midpage_numbered_note_run_above_sequence_can_be_marginalia(line, lines)
            || previous_sequence_small_font_continuation_can_be_marginalia(line)
            || outside_sequence_model_supported_continuation_can_be_marginalia(line)
            || outside_sequence_specific_citation_fragment_can_be_marginalia(line)
            || previous_sequence_publication_continuation_can_be_marginalia(line)
            || previous_sequence_quote_continuation_can_be_marginalia(line)
            || previous_sequence_normal_font_citation_or_marker_can_be_marginalia(line)
            || early_numbered_note_pair_can_be_marginalia(line, next)
            || decoded_adjacent_continuation_can_be_marginalia(hints, line, prev)
            || same_row_right_fragment_after_marginalia_can_be_marginalia(hints, line, prev)
            || list_item_core_footnote_continuation_can_be_marginalia(hints, line)
            || small_font_line_before_next_note_run_can_be_marginalia(hints, line, next)
            || contents_like_credential_before_note_run_can_be_marginalia(hints, line, next)
            || metadata_credential_continuation_after_marginalia_can_be_marginalia(
                hints, line, prev,
            )
            || tiny_numeric_note_cluster_can_be_marginalia(hints, line, nearby.iter().copied())
            || should_decode_keep_as_marginalia(hints, line, prev, next)
            || plain_numeric_fragment_near_marginalia(hints, line, nearby.into_iter())
        {
            push_unique_hint(hints, &line.text, LiquidBlockRole::Marginalia);
        }
    }
}

fn previous_sequence_small_font_continuation_can_be_marginalia(line: &LayoutLine) -> bool {
    if is_repository_cover_boilerplate(line)
        || is_repository_cover_identifier(line)
        || is_disposable_contents_or_index_line(line)
        || looks_like_nonlegal_study_prompt_noise(line)
        || looks_like_running_law_review_cite_line(&line.text)
        || is_plain_page_number_line(&line.text)
        || starts_with_numeric_lowercase_body_fragment(&line.text)
        || line.text.trim_end().ends_with(':')
        || is_bare_numeric_fragment(&line.text)
        || (!line.prev_sequence_footnote_zone && !line.prev_note_marker)
        || line.font_ratio_page_ref > 0.85
        || line.y0_ratio() < 0.45
        || line.prev_y_gap_ratio > 0.018
        || line.prev_left_delta_ratio > 0.16
    {
        return false;
    }

    starts_with_note_marker(&line.text) || !looks_like_clear_section_heading(&line.text)
}

fn is_bare_numeric_fragment(text: &str) -> bool {
    let trimmed = text.trim();
    let mut digits = 0usize;
    let mut chars = trimmed.chars().peekable();
    while chars.peek().is_some_and(|ch| ch.is_ascii_digit()) {
        digits += 1;
        chars.next();
    }
    if !(1..=4).contains(&digits) {
        return false;
    }
    match chars.next() {
        None => true,
        Some('.' | ')') => chars.next().is_none(),
        _ => false,
    }
}

fn line_should_never_be_marginalia_by_body_geometry(line: &LayoutLine) -> bool {
    old_law_review_topic_heading_like_body_line(line)
        || normal_font_body_citation_continuation_like_body_line(line)
        || sequence_zone_quoted_body_excerpt_like_body_line(line)
        || looks_like_sequence_zone_orphan_contents_fragment_noise(line)
        || body_quote_or_rule_leadin_after_citation_like_body_line(line)
}

fn old_law_review_topic_heading_like_body_line(line: &LayoutLine) -> bool {
    let text = line.text.trim();
    let y0 = line.y0_ratio();
    let width_ratio = (line.right - line.left) / line.page_width.max(1.0);
    line.sequence_footnote_zone
        && !line.below_footnote_divider
        && (0.22..=0.42).contains(&y0)
        && !starts_with_note_marker(text)
        && !contains_legal_note_cue(text)
        && width_ratio >= 0.44
        && line.font_ratio_page_ref <= 0.88
        && (2..=11).contains(&word_count(text))
        && text.chars().filter(|ch| ch.is_alphabetic()).count() >= 8
        && uppercase_ratio(text) >= 0.55
        && (text.contains('-') || text.ends_with('-') || text.chars().any(|ch| ch.is_ascii_digit()))
}

fn normal_font_body_citation_continuation_like_body_line(line: &LayoutLine) -> bool {
    let text = line.text.trim();
    let y0 = line.y0_ratio();
    let width_ratio = (line.right - line.left) / line.page_width.max(1.0);
    !line.sequence_footnote_zone
        && !line.below_footnote_divider
        && !line.prev_sequence_footnote_zone
        && !line.prev_note_marker
        && !starts_with_note_marker(text)
        && line.font_ratio_page_ref >= 0.95
        && width_ratio >= 0.55
        && (0.35..=0.82).contains(&y0)
        && text
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_lowercase())
        && looks_like_citation_continuation_text(text)
}

fn sequence_zone_quoted_body_excerpt_like_body_line(line: &LayoutLine) -> bool {
    let text = line.text.trim();
    let y0 = line.y0_ratio();
    if !line.sequence_footnote_zone
        || line.below_footnote_divider
        || line.prev_sequence_footnote_zone
        || line.next_sequence_footnote_zone
        || line.prev_note_marker
        || line.next_note_marker
        || !(0.45..=0.66).contains(&y0)
        || starts_with_note_marker(text)
    {
        return false;
    }
    if text.starts_with('"') && line.font_ratio_page_ref >= 0.78 {
        return true;
    }
    word_count(text) == 1
        && text
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_lowercase())
        && line.font_ratio_page_ref >= 0.80
}

fn body_quote_or_rule_leadin_after_citation_like_body_line(line: &LayoutLine) -> bool {
    let text = line.text.trim();
    let width_ratio = (line.right - line.left) / line.page_width.max(1.0);
    !line.sequence_footnote_zone
        && !line.below_footnote_divider
        && line.prev_sequence_footnote_zone
        && !line.prev_note_marker
        && (0.58..=0.70).contains(&line.y0_ratio())
        && line.font_ratio_page_ref <= 0.90
        && (0.24..=0.48).contains(&width_ratio)
        && text.ends_with(':')
        && text
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_uppercase())
        && !starts_with_note_marker(text)
        && !contains_legal_note_cue(text)
        && !looks_like_citation_continuation_text(text)
}

fn outside_sequence_decode_repair_excluded(line: &LayoutLine) -> bool {
    let text = line.text.trim();
    is_repository_cover_boilerplate(line)
        || is_repository_cover_identifier(line)
        || is_disposable_contents_or_index_line(line)
        || looks_like_nonlegal_study_prompt_noise(line)
        || line_should_never_be_marginalia_by_body_geometry(line)
        || looks_like_running_law_review_cite_line(text)
        || looks_like_clear_section_heading(text)
        || text.ends_with(':')
        || (is_plain_page_number_line(text) && !looks_like_plain_numeric_citation_fragment(text))
}

fn runtime_main_and_core_predict_footnote(line: &LayoutLine) -> bool {
    layout_role_model().and_then(|model| model.predict(line)) == Some("footnote")
        && liquid_core_role_model().and_then(|model| model.predict(line)) == Some("footnote")
}

fn runtime_footnote_specialist_predicts_footnote(line: &LayoutLine) -> bool {
    footnote_role_model().and_then(|model| model.predict_footnote_specialist(line))
        == Some("footnote")
}

fn outside_sequence_model_supported_continuation_can_be_marginalia(line: &LayoutLine) -> bool {
    let text = line.text.trim();
    let y0 = line.y0_ratio();
    let width_ratio = (line.right - line.left) / line.page_width.max(1.0);
    if outside_sequence_decode_repair_excluded(line)
        || !line.prev_sequence_footnote_zone
        || y0 < 0.55
        || y0 > 0.91
        || line.font_ratio_page_ref > 0.83
        || line.prev_y_gap_ratio > 0.018
        || width_ratio > 0.72
        || line.body_column_like
    {
        return false;
    }
    let model_supported = runtime_main_and_core_predict_footnote(line);
    let continuation_shape = line.prev_left_delta_ratio <= 0.18
        || (line.prev_left_delta_ratio <= 0.55
            && (runtime_footnote_specialist_predicts_footnote(line)
                || looks_like_citation_continuation_text(text)
                || text.contains('"')));
    let small_font_run_shape = line.prev_left_delta_ratio <= 0.55
        && line.prev_small_font
        && line.next_small_font
        && line.font_ratio_doc <= 0.84
        && (looks_like_citation_continuation_text(text) || text.contains('"'));
    (model_supported && continuation_shape) || small_font_run_shape
}

fn outside_sequence_specific_citation_fragment_can_be_marginalia(line: &LayoutLine) -> bool {
    let text = line.text.trim();
    let y0 = line.y0_ratio();
    let width_ratio = (line.right - line.left) / line.page_width.max(1.0);
    !outside_sequence_decode_repair_excluded(line)
        && line.prev_sequence_footnote_zone
        && y0 >= 0.55
        && line.prev_y_gap_ratio <= 0.018
        && line.font_ratio_page_ref <= 0.88
        && width_ratio <= 0.38
        && runtime_footnote_specialist_predicts_footnote(line)
        && (looks_like_citation_continuation_text(text)
            || looks_like_publication_citation_continuation_text(text)
            || looks_like_seq_comma_numeric_fragment(text))
}

fn previous_sequence_publication_continuation_can_be_marginalia(line: &LayoutLine) -> bool {
    let text = line.text.trim();
    let y0 = line.y0_ratio();
    !outside_sequence_decode_repair_excluded(line)
        && line.prev_sequence_footnote_zone
        && line.prev_note_marker
        && (0.55..=0.80).contains(&y0)
        && line.font_ratio_page_ref <= 0.82
        && line.prev_y_gap_ratio <= 0.018
        && line.prev_left_delta_ratio <= 0.08
        && runtime_footnote_specialist_predicts_footnote(line)
        && (contains_month_abbreviation(text)
            || looks_like_publication_citation_continuation_text(text)
            || contains_specific_citation_material_cue(text))
}

fn previous_sequence_quote_continuation_can_be_marginalia(line: &LayoutLine) -> bool {
    let text = line.text.trim();
    let y0 = line.y0_ratio();
    !outside_sequence_decode_repair_excluded(line)
        && line.prev_sequence_footnote_zone
        && (0.70..=0.88).contains(&y0)
        && line.font_ratio_page_ref <= 0.89
        && line.prev_y_gap_ratio <= 0.018
        && line.prev_left_delta_ratio <= 0.04
        && text.starts_with('"')
        && runtime_main_and_core_predict_footnote(line)
}

fn previous_sequence_normal_font_citation_or_marker_can_be_marginalia(line: &LayoutLine) -> bool {
    let text = line.text.trim();
    let y0 = line.y0_ratio();
    let width_ratio = (line.right - line.left) / line.page_width.max(1.0);
    if outside_sequence_decode_repair_excluded(line)
        || !line.prev_sequence_footnote_zone
        || !(0.36..=0.62).contains(&y0)
        || line.prev_y_gap_ratio > 0.004
        || line.prev_left_delta_ratio > 0.08
        || line.font_ratio_page_ref > 1.02
    {
        return false;
    }
    if is_bare_numeric_fragment(text) {
        return y0 <= 0.55
            && line.font_ratio_page_ref <= 0.75
            && width_ratio <= 0.04
            && runtime_main_and_core_predict_footnote(line);
    }
    let clear_note_text = starts_with_note_marker(text)
        || looks_like_citation_continuation_text(text)
        || contains_specific_citation_material_cue(text);
    clear_note_text
        && (runtime_main_and_core_predict_footnote(line)
            || contains_legal_note_cue(text)
            || starts_with_note_marker(text))
}

fn early_numbered_note_pair_can_be_marginalia(
    line: &LayoutLine,
    next: Option<&LayoutLine>,
) -> bool {
    let text = line.text.trim();
    let y0 = line.y0_ratio();
    let width_ratio = (line.right - line.left) / line.page_width.max(1.0);
    if outside_sequence_decode_repair_excluded(line)
        || !(0.34..=0.45).contains(&y0)
        || line.font_ratio_page_ref > 1.02
        || width_ratio > 0.75
        || leading_numbered_note_marker_value(text).is_none_or(|value| value < 10)
    {
        return false;
    }
    let Some(next) = next else {
        return false;
    };
    let left_delta = ((next.left - line.left).abs() / line.page_width.max(1.0)).max(0.0);
    (next.y0_ratio() - y0).abs() <= 0.025
        && left_delta <= 0.04
        && next.prev_note_marker
        && next.font_ratio_page_ref <= 1.03
        && next
            .text
            .trim()
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_lowercase())
}

fn decoded_adjacent_continuation_can_be_marginalia(
    hints: &[LiquidLayoutHint],
    line: &LayoutLine,
    prev: Option<&LayoutLine>,
) -> bool {
    let Some(prev) = prev else {
        return false;
    };
    if outside_sequence_decode_repair_excluded(line)
        || !matches!(
            hint_role_for_line(hints, prev),
            Some(LiquidBlockRole::Marginalia)
        )
    {
        return false;
    }
    let text = line.text.trim();
    let y0 = line.y0_ratio();
    let prev_y0 = prev.y0_ratio();
    let left_delta = ((line.left - prev.left).abs() / line.page_width.max(1.0)).max(0.0);
    let width_ratio = (line.right - line.left) / line.page_width.max(1.0);
    if (0.34..=0.46).contains(&y0)
        && line.font_ratio_page_ref <= 1.03
        && width_ratio <= 0.75
        && line.prev_note_marker
        && text
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_lowercase())
        && (y0 - prev_y0).abs() <= 0.025
        && left_delta <= 0.04
    {
        return true;
    }
    (0.36..=0.48).contains(&y0)
        && line.font_ratio_page_ref <= 1.03
        && width_ratio <= 0.08
        && looks_like_short_acronym_fragment(text)
        && (y0 - prev_y0).abs() <= 0.04
        && left_delta <= 0.12
}

fn same_row_right_fragment_after_marginalia_can_be_marginalia(
    hints: &[LiquidLayoutHint],
    line: &LayoutLine,
    prev: Option<&LayoutLine>,
) -> bool {
    let Some(prev) = prev else {
        return false;
    };
    if outside_sequence_decode_repair_excluded(line)
        || !matches!(
            hint_role_for_line(hints, prev),
            Some(LiquidBlockRole::Marginalia)
        )
    {
        return false;
    }
    let y0 = line.y0_ratio();
    let prev_y0 = prev.y0_ratio();
    let width_ratio = (line.right - line.left) / line.page_width.max(1.0);
    (0.50..=0.86).contains(&y0)
        && line.font_ratio_page_ref <= 0.86
        && width_ratio <= 0.08
        && word_count(&line.text) <= 2
        && (y0 - prev_y0).abs() <= 0.004
        && line.left > prev.left
        && line.prev_left_delta_ratio >= 0.40
        && runtime_main_and_core_predict_footnote(line)
}

fn list_item_core_footnote_continuation_can_be_marginalia(
    hints: &[LiquidLayoutHint],
    line: &LayoutLine,
) -> bool {
    let y0 = line.y0_ratio();
    let width_ratio = (line.right - line.left) / line.page_width.max(1.0);
    !outside_sequence_decode_repair_excluded(line)
        && line.prev_sequence_footnote_zone
        && (0.55..=0.75).contains(&y0)
        && line.prev_y_gap_ratio <= 0.018
        && line.prev_left_delta_ratio <= 0.025
        && line.font_ratio_page_ref <= 1.02
        && width_ratio <= 0.18
        && word_count(&line.text) <= 4
        && liquid_core_role_model().and_then(|model| model.predict(line)) == Some("footnote")
        && matches!(
            hint_role_for_line(hints, line),
            Some(LiquidBlockRole::ListItem)
        )
}

fn small_font_line_before_next_note_run_can_be_marginalia(
    hints: &[LiquidLayoutHint],
    line: &LayoutLine,
    next: Option<&LayoutLine>,
) -> bool {
    let Some(next) = next else {
        return false;
    };
    if outside_sequence_decode_repair_excluded(line)
        || !matches!(
            hint_role_for_line(hints, next),
            Some(LiquidBlockRole::Marginalia)
        )
    {
        return false;
    }
    let y0 = line.y0_ratio();
    let next_y0 = next.y0_ratio();
    let width_ratio = (line.right - line.left) / line.page_width.max(1.0);
    let left_delta = ((line.left - next.left).abs() / line.page_width.max(1.0)).max(0.0);
    (0.52..=0.62).contains(&y0)
        && line.font_ratio_page_ref <= 0.80
        && (0.25..=0.65).contains(&width_ratio)
        && next_y0 > y0
        && next_y0 - y0 <= 0.08
        && left_delta <= 0.09
        && layout_role_model().and_then(|model| model.predict(line)) == Some("body")
        && liquid_core_role_model().and_then(|model| model.predict(line)) == Some("body")
}

fn contents_like_credential_before_note_run_can_be_marginalia(
    hints: &[LiquidLayoutHint],
    line: &LayoutLine,
    next: Option<&LayoutLine>,
) -> bool {
    let text = line.text.trim();
    let y0 = line.y0_ratio();
    let width_ratio = (line.right - line.left) / line.page_width.max(1.0);
    if !line.page_contents_like
        || !(0.68..=0.78).contains(&y0)
        || line.font_ratio_page_ref > 0.80
        || width_ratio > 0.75
        || !looks_like_author_credential_fragment(text)
    {
        return false;
    }
    let Some(next) = next else {
        return false;
    };
    matches!(
        hint_role_for_line(hints, next),
        Some(LiquidBlockRole::Marginalia)
    ) && next.y0_ratio() > y0
        && next.y0_ratio() - y0 <= 0.12
        && next.font_ratio_page_ref <= 0.82
}

fn looks_like_author_credential_fragment(text: &str) -> bool {
    [
        "J.D.",
        "L.L.M.",
        "LL.M",
        "B.B.A.",
        "University",
        "Univer-",
        "Law Center",
        "College of Law",
    ]
    .iter()
    .any(|cue| text.contains(cue))
}

fn metadata_credential_continuation_after_marginalia_can_be_marginalia(
    hints: &[LiquidLayoutHint],
    line: &LayoutLine,
    prev: Option<&LayoutLine>,
) -> bool {
    let Some(prev) = prev else {
        return false;
    };
    let text = line.text.trim();
    let y0 = line.y0_ratio();
    let width_ratio = (line.right - line.left) / line.page_width.max(1.0);
    if !matches!(
        hint_role_for_line(hints, prev),
        Some(LiquidBlockRole::Marginalia)
    ) || !(0.68..=0.78).contains(&y0)
        || line.font_ratio_page_ref > 0.82
        || !(0.45..=0.75).contains(&width_ratio)
        || !looks_like_author_credential_fragment(text)
    {
        return false;
    }
    let prev_y0 = prev.y0_ratio();
    let left_delta = (line.left - prev.left).abs() / line.page_width.max(1.0);
    y0 > prev_y0 && y0 - prev_y0 <= 0.035 && left_delta <= 0.075
}

fn tiny_numeric_note_cluster_can_be_marginalia<'a>(
    hints: &[LiquidLayoutHint],
    line: &LayoutLine,
    nearby: impl Iterator<Item = &'a LayoutLine>,
) -> bool {
    let text = line.text.trim();
    let y0 = line.y0_ratio();
    let width_ratio = (line.right - line.left) / line.page_width.max(1.0);
    let x0 = line.left / line.page_width.max(1.0);
    if !(1..=3).contains(&text.len())
        || !text.chars().all(|ch| ch.is_ascii_digit())
        || !(0.82..=0.90).contains(&y0)
        || line.font_ratio_page_ref > 0.72
        || width_ratio > 0.025
        || x0 < 0.85
    {
        return false;
    }
    nearby
        .filter(|neighbor| (neighbor.y0_ratio() - y0).abs() <= 0.04)
        .filter(|neighbor| {
            matches!(
                hint_role_for_line(hints, neighbor),
                Some(LiquidBlockRole::Marginalia)
            )
        })
        .take(2)
        .count()
        >= 2
}

fn looks_like_seq_comma_numeric_fragment(text: &str) -> bool {
    let trimmed = text.trim();
    let mut chars = trimmed.chars().peekable();
    let digits = chars.by_ref().take_while(|ch| ch.is_ascii_digit()).count();
    if !(1..=4).contains(&digits) {
        return false;
    }
    let rest = chars.collect::<String>();
    let rest = rest.trim_start();
    rest.starts_with("seq.,")
        && rest
            .trim_start_matches("seq.,")
            .trim_start()
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_digit())
}

fn looks_like_short_acronym_fragment(text: &str) -> bool {
    let trimmed = text.trim();
    (2..=8).contains(&trimmed.len()) && trimmed.chars().all(|ch| ch.is_ascii_uppercase())
}

fn dense_citation_prelude_row_above_footnote_sequence(
    line: &LayoutLine,
    lines: &[LayoutLine],
) -> bool {
    if is_repository_cover_boilerplate(line)
        || is_repository_cover_identifier(line)
        || is_disposable_contents_or_index_line(line)
        || line.page_contents_like
        || line.repeated_header_footer
        || line.sequence_footnote_zone
    {
        return false;
    }
    let y0 = line.y0_ratio();
    let width_ratio = (line.right - line.left) / line.page_width.max(1.0);
    if !(0.62..=0.86).contains(&y0) || line.font_ratio_page_ref > 0.88 || width_ratio > 0.25 {
        return false;
    }

    let row = lines
        .iter()
        .filter(|candidate| {
            !candidate.sequence_footnote_zone
                && !candidate.repeated_header_footer
                && (candidate.y0_ratio() - y0).abs() <= 0.0045
        })
        .collect::<Vec<_>>();
    if row.len() < 6 {
        return false;
    }
    let small_fragments = row
        .iter()
        .filter(|candidate| candidate.font_ratio_page_ref <= 0.88)
        .count();
    if small_fragments < 5 || small_fragments * 100 < row.len() * 65 {
        return false;
    }

    let mut sorted_row = row;
    sorted_row.sort_by(|a, b| a.left.total_cmp(&b.left));
    let row_text = sorted_row
        .iter()
        .map(|candidate| candidate.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    if !looks_like_dense_citation_prelude_row(&row_text) {
        return false;
    }

    lines
        .iter()
        .filter(|candidate| {
            let delta = candidate.y0_ratio() - y0;
            delta > 0.0
                && delta <= 0.04
                && candidate.sequence_footnote_zone
                && candidate.font_ratio_page_ref <= 0.92
        })
        .count()
        >= 2
}

fn looks_like_dense_citation_prelude_row(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let has_versus = lower.contains(" v.") || lower.contains(" v ");
    let has_reporter = looks_like_reporter_citation_text(text)
        || lower.contains(" supp")
        || lower.contains(" u.s.")
        || lower.contains(" u. s.")
        || lower.contains(" cir.")
        || lower.contains(" stat.");
    (has_versus && has_reporter) || lower.contains("compare") && has_versus
}

fn sequence_citation_row_fragment_can_be_marginalia(
    line: &LayoutLine,
    lines: &[LayoutLine],
) -> bool {
    if is_repository_cover_boilerplate(line)
        || is_repository_cover_identifier(line)
        || is_disposable_contents_or_index_line(line)
        || line.page_contents_like
        || line.repeated_header_footer
        || !line.sequence_footnote_zone
    {
        return false;
    }
    let y0 = line.y0_ratio();
    if !(0.30..=0.86).contains(&y0) || line.font_ratio_page_ref > 1.05 {
        return false;
    }

    let row = lines
        .iter()
        .filter(|candidate| {
            candidate.sequence_footnote_zone
                && !candidate.repeated_header_footer
                && (candidate.y0_ratio() - y0).abs() <= 0.0045
                && candidate.font_ratio_page_ref <= 1.05
        })
        .collect::<Vec<_>>();
    if row.len() < 4 {
        return false;
    }
    let mut sorted_row = row;
    sorted_row.sort_by(|a, b| a.left.total_cmp(&b.left));
    let row_text = sorted_row
        .iter()
        .map(|candidate| candidate.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    if !looks_like_sequence_citation_fragment_row(&row_text) {
        return false;
    }

    lines
        .iter()
        .filter(|candidate| {
            (candidate.y0_ratio() - y0).abs() <= 0.035
                && candidate.sequence_footnote_zone
                && candidate.font_ratio_page_ref <= 1.05
        })
        .count()
        >= 4
}

fn looks_like_sequence_citation_fragment_row(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let has_versus = lower.contains(" v.") || lower.contains(" v ");
    let has_case_context = lower.contains(" lcia")
        || lower.contains(" case no")
        || lower.contains(" award")
        || lower.contains("http")
        || lower.contains(" yale l.j")
        || lower.contains(" l.j.")
        || looks_like_reporter_citation_text(text)
        || lower.contains(" supp")
        || lower.contains(" u.s.")
        || lower.contains(" f.");
    has_versus && has_case_context
}

fn midpage_numbered_note_run_above_sequence_can_be_marginalia(
    line: &LayoutLine,
    lines: &[LayoutLine],
) -> bool {
    if is_repository_cover_boilerplate(line)
        || is_repository_cover_identifier(line)
        || is_disposable_contents_or_index_line(line)
        || line.page_contents_like
        || line.repeated_header_footer
        || line.sequence_footnote_zone
        || line.below_footnote_divider
    {
        return false;
    }
    let y0 = line.y0_ratio();
    if !(0.30..=0.55).contains(&y0) || line.font_ratio_page_ref > 1.05 {
        return false;
    }

    let is_run_member = leading_numbered_note_marker_value(&line.text).is_some()
        || previous_numbered_note_start_supports_continuation(line, lines);
    if !is_run_member {
        return false;
    }

    let mut note_numbers = lines
        .iter()
        .filter(|candidate| {
            let candidate_y = candidate.y0_ratio();
            (y0 - 0.08..=y0 + 0.16).contains(&candidate_y) && candidate.font_ratio_page_ref <= 1.08
        })
        .filter_map(|candidate| leading_numbered_note_marker_value(&candidate.text))
        .filter(|number| *number >= 10)
        .collect::<Vec<_>>();
    note_numbers.sort_unstable();
    note_numbers.dedup();
    if longest_near_consecutive_run(&note_numbers) < 3 {
        return false;
    }

    lines
        .iter()
        .filter(|candidate| {
            let delta = candidate.y0_ratio() - y0;
            delta > 0.0 && delta <= 0.18 && candidate.sequence_footnote_zone
        })
        .count()
        >= 3
}

fn previous_numbered_note_start_supports_continuation(
    line: &LayoutLine,
    lines: &[LayoutLine],
) -> bool {
    lines
        .iter()
        .filter(|candidate| {
            candidate.line_index != line.line_index
                && leading_numbered_note_marker_value(&candidate.text).is_some()
                && candidate.font_ratio_page_ref <= 1.08
        })
        .filter_map(|candidate| {
            let y_gap = line.y0_ratio() - candidate.y0_ratio();
            if y_gap > 0.0 && y_gap <= 0.025 {
                Some((y_gap, candidate))
            } else {
                None
            }
        })
        .min_by(|(gap_a, _), (gap_b, _)| gap_a.total_cmp(gap_b))
        .is_some_and(|(_, candidate)| {
            ((candidate.left - line.left).abs() / line.page_width.max(1.0)) <= 0.04
        })
}

fn leading_numbered_note_marker_value(text: &str) -> Option<usize> {
    let trimmed = text.trim_start();
    let mut digits = 0usize;
    let mut value = 0usize;
    let mut marker_end = 0usize;
    for (index, ch) in trimmed.char_indices() {
        if ch.is_ascii_digit() {
            digits += 1;
            value = value
                .saturating_mul(10)
                .saturating_add((ch as u8 - b'0') as usize);
            marker_end = index + ch.len_utf8();
            continue;
        }
        break;
    }
    if !(1..=4).contains(&digits) || trimmed.len() <= marker_end {
        return None;
    }
    let mut rest = trimmed[marker_end..].chars();
    let separator = rest.next()?;
    if !matches!(separator, '.' | ')' | ']') {
        return None;
    }
    if rest.next().is_some_and(|ch| !ch.is_whitespace()) {
        return None;
    }
    Some(value)
}

fn longest_near_consecutive_run(numbers: &[usize]) -> usize {
    if numbers.is_empty() {
        return 0;
    }
    let mut current = 1usize;
    let mut best = 1usize;
    for pair in numbers.windows(2) {
        let gap = pair[1].saturating_sub(pair[0]);
        if (1..=2).contains(&gap) {
            current += 1;
            best = best.max(current);
        } else {
            current = 1;
        }
    }
    best
}

fn should_decode_keep_as_marginalia(
    hints: &[LiquidLayoutHint],
    line: &LayoutLine,
    prev: Option<&LayoutLine>,
    next: Option<&LayoutLine>,
) -> bool {
    if is_repository_cover_boilerplate(line) || is_repository_cover_identifier(line) {
        return false;
    }
    if is_disposable_contents_or_index_line(line) {
        return false;
    }
    if looks_like_nonlegal_study_prompt_noise(line) {
        return false;
    }
    if line_should_never_be_marginalia_by_body_geometry(line) {
        return false;
    }
    if sequence_zone_numeric_marker_in_note_run_can_be_marginalia(line, prev, next) {
        return true;
    }
    if sequence_zone_body_enumeration_marker(line) {
        return false;
    }
    if normal_font_top_half_sequence_body_line(line) {
        return false;
    }
    if normal_font_inline_body_citation_continuation(line) {
        return false;
    }
    if normal_font_body_fragment_before_inline_note_marker(line) {
        return false;
    }
    if looks_like_administrative_status_or_update_line(&line.text) {
        return false;
    }
    if line_should_never_be_marginalia_by_body_geometry(line) {
        return false;
    }
    if line.font_ratio_page_ref >= 0.90 && looks_like_uncited_all_caps_topic_heading(&line.text) {
        return false;
    }
    if starts_with_quoted_citation_fragment(&line.text)
        && !line.below_footnote_divider
        && !line.sequence_footnote_zone
    {
        return false;
    }
    if small_font_bibliographic_lead_can_be_marginalia(line) {
        return true;
    }
    if contents_like_author_credential_continuation_can_be_marginalia(line)
        || contents_like_tiny_citation_fragment_can_be_marginalia(line)
    {
        return true;
    }
    if line.page_contents_like
        && !contents_like_page_line_can_be_marginalia(line)
        && !fragmented_publication_piece_adjacent_to_marginalia(hints, line, prev, next)
    {
        return false;
    }
    if below_divider_small_font_continuation_can_be_marginalia(line, prev, next) {
        return true;
    }
    if concrete_sequence_citation_fragment_can_be_marginalia(line) {
        return true;
    }
    if small_font_sequence_citation_material_can_be_marginalia(line) {
        return true;
    }
    if small_font_note_run_continuation_can_be_marginalia(line, prev, next) {
        return true;
    }
    if numeric_citation_fragment_in_note_run_can_be_marginalia(line, prev, next) {
        return true;
    }
    if numeric_note_fragment_geometry_can_be_marginalia(line) {
        return true;
    }
    if numeric_lowercase_publication_note_start_can_be_marginalia(line) {
        return true;
    }
    if numeric_year_parenthetical_continuation_can_be_marginalia(hints, line, prev, next) {
        return true;
    }
    if numeric_page_parenthetical_citation_continuation_can_be_marginalia(hints, line, prev, next) {
        return true;
    }
    if looks_like_running_law_review_cite_line(&line.text)
        || (is_plain_page_number_line(&line.text)
            && !plain_numeric_fragment_adjacent_to_marginalia(hints, line, prev, next))
        || looks_like_clear_section_heading(&line.text)
        || starts_with_numeric_lowercase_body_fragment(&line.text)
    {
        return false;
    }
    if line.y0_ratio() < 0.45 && !line.below_footnote_divider && !line.sequence_footnote_zone {
        return false;
    }
    if line.font_ratio_page_ref > 1.02 {
        return false;
    }
    if plain_numeric_fragment_adjacent_to_marginalia(hints, line, prev, next) {
        return true;
    }

    let note_marker = starts_with_note_marker(&line.text);
    let current_note_text = note_marker
        || contains_legal_note_cue(&line.text)
        || looks_like_citation_continuation_text(&line.text)
        || looks_like_publication_citation_continuation_text(&line.text)
        || looks_like_footnote_bibliographic_lead_text(&line.text);
    if line.below_footnote_divider && line.font_ratio_page_ref <= 1.05 && current_note_text {
        return true;
    }
    if line.sequence_footnote_zone && line.font_ratio_page_ref <= 1.02 {
        if line.font_ratio_page_ref <= 0.92 || current_note_text {
            return true;
        }
        let width_ratio = (line.right - line.left) / line.page_width.max(1.0);
        if width_ratio > 0.55 {
            return false;
        }
        if let Some(prev) = prev
            && adjacent_note_context_supports_continuation(hints, prev, line)
        {
            return true;
        }
        if let Some(next) = next
            && matches!(
                hint_role_for_line(hints, next),
                Some(LiquidBlockRole::Marginalia)
            )
            && adjacent_geometry_supports_continuation(next, line)
        {
            return true;
        }
    }
    if note_marker
        && line.y0_ratio() >= 0.50
        && line.font_ratio_page_ref <= 0.90
        && let Some(next) = next
        && adjacent_geometry_supports_continuation(line, next)
        && (contains_legal_note_cue(&next.text)
            || looks_like_citation_continuation_text(&next.text)
            || looks_like_publication_citation_continuation_text(&next.text)
            || looks_like_footnote_bibliographic_lead_text(&next.text))
    {
        return true;
    }

    if let Some(prev) = prev
        && adjacent_note_context_supports_continuation(hints, prev, line)
        && (current_note_text
            || line.y0_ratio() >= 0.68
            || (line.font_ratio_page_ref <= 0.88 && line.y0_ratio() >= 0.62))
    {
        return true;
    }
    if let Some(next) = next
        && matches!(
            hint_role_for_line(hints, next),
            Some(LiquidBlockRole::Marginalia)
        )
        && adjacent_geometry_supports_continuation(next, line)
        && current_note_text
        && line.font_ratio_page_ref <= 0.92
    {
        return true;
    }
    false
}

fn fragmented_publication_piece_adjacent_to_marginalia(
    hints: &[LiquidLayoutHint],
    line: &LayoutLine,
    prev: Option<&LayoutLine>,
    next: Option<&LayoutLine>,
) -> bool {
    if line.y0_ratio() < 0.54
        || line.y0_ratio() > 0.92
        || line.font_ratio_page_ref > 0.92
        || !looks_like_fragmented_publication_note_piece(&line.text)
    {
        return false;
    }
    [prev, next].into_iter().flatten().any(|neighbor| {
        matches!(
            hint_role_for_line(hints, neighbor),
            Some(LiquidBlockRole::Marginalia)
        ) && adjacent_geometry_supports_continuation(neighbor, line)
    })
}

fn plain_numeric_fragment_adjacent_to_marginalia(
    hints: &[LiquidLayoutHint],
    line: &LayoutLine,
    prev: Option<&LayoutLine>,
    next: Option<&LayoutLine>,
) -> bool {
    plain_numeric_fragment_near_marginalia(hints, line, [prev, next].into_iter().flatten())
}

fn plain_numeric_fragment_near_marginalia<'a>(
    hints: &[LiquidLayoutHint],
    line: &LayoutLine,
    neighbors: impl IntoIterator<Item = &'a LayoutLine>,
) -> bool {
    if !plain_numeric_fragment_can_be_marginalia(line) {
        return false;
    }
    let neighbors = neighbors.into_iter().collect::<Vec<_>>();
    neighbors.iter().any(|neighbor| {
        matches!(
            hint_role_for_line(hints, *neighbor),
            Some(LiquidBlockRole::Marginalia)
        ) && adjacent_numeric_fragment_geometry_supports_continuation(neighbor, line)
    }) || plain_numeric_fragment_in_dense_marginalia_run(hints, line, &neighbors)
}

fn plain_numeric_fragment_in_dense_marginalia_run(
    hints: &[LiquidLayoutHint],
    line: &LayoutLine,
    neighbors: &[&LayoutLine],
) -> bool {
    if line.font_ratio_page_ref > 0.72 {
        return false;
    }
    let y0 = line.y0_ratio();
    let width_ratio = (line.right - line.left) / line.page_width.max(1.0);
    if !(0.84..=0.90).contains(&y0) || width_ratio > 0.035 {
        return false;
    }
    neighbors
        .iter()
        .filter(|neighbor| {
            (neighbor.y0_ratio() - y0).abs() <= 0.025
                && matches!(
                    hint_role_for_line(hints, neighbor),
                    Some(LiquidBlockRole::Marginalia)
                )
        })
        .count()
        >= 3
}

fn plain_numeric_fragment_can_be_marginalia(line: &LayoutLine) -> bool {
    if is_repository_cover_boilerplate(line) || is_repository_cover_identifier(line) {
        return false;
    }
    if is_disposable_contents_or_index_line(line) {
        return false;
    }
    if looks_like_nonlegal_study_prompt_noise(line) {
        return false;
    }
    if line.page_contents_like && !contents_like_page_line_can_be_marginalia(line) {
        return false;
    }
    if !looks_like_plain_numeric_citation_fragment(&line.text)
        || line.y0_ratio() < 0.65
        || line.y0_ratio() > 0.90
        || line.font_ratio_page_ref > 0.84
    {
        return false;
    }
    let width_ratio = (line.right - line.left) / line.page_width.max(1.0);
    width_ratio <= 0.08
}

fn concrete_sequence_citation_fragment_can_be_marginalia(line: &LayoutLine) -> bool {
    if is_repository_cover_boilerplate(line) || is_repository_cover_identifier(line) {
        return false;
    }
    if is_disposable_contents_or_index_line(line) {
        return false;
    }
    if looks_like_nonlegal_study_prompt_noise(line) {
        return false;
    }
    if line.page_contents_like && !contents_like_page_line_can_be_marginalia(line) {
        return false;
    }
    let y0 = line.y0_ratio();
    let width_ratio = (line.right - line.left) / line.page_width.max(1.0);
    line.sequence_footnote_zone
        && (0.58..=0.91).contains(&y0)
        && line.font_ratio_page_ref <= 0.96
        && width_ratio <= 0.55
        && looks_like_concrete_citation_fragment(&line.text)
}

fn small_font_sequence_citation_material_can_be_marginalia(line: &LayoutLine) -> bool {
    if is_repository_cover_boilerplate(line) || is_repository_cover_identifier(line) {
        return false;
    }
    if is_disposable_contents_or_index_line(line) || line.repeated_header_footer {
        return false;
    }
    if line.page_contents_like && !contents_like_page_line_can_be_marginalia(line) {
        return false;
    }
    let y0 = line.y0_ratio();
    let width_ratio = (line.right - line.left) / line.page_width.max(1.0);
    if line.below_footnote_divider
        || !line.sequence_footnote_zone
        || !(0.45..=0.90).contains(&y0)
        || line.font_ratio_page_ref > 0.92
        || width_ratio > 0.80
        || is_plain_page_number_line(&line.text)
    {
        return false;
    }
    let citation_material = looks_like_citation_continuation_text(&line.text)
        || looks_like_publication_citation_continuation_text(&line.text)
        || looks_like_footnote_bibliographic_lead_text(&line.text)
        || looks_like_concrete_citation_fragment(&line.text)
        || contains_specific_citation_material_cue(&line.text);
    if !citation_material {
        return false;
    }
    !looks_like_clear_section_heading(&line.text)
        || contains_specific_citation_material_cue(&line.text)
}

fn sequence_zone_numeric_marker_in_note_run_can_be_marginalia(
    line: &LayoutLine,
    prev: Option<&LayoutLine>,
    next: Option<&LayoutLine>,
) -> bool {
    if is_repository_cover_boilerplate(line) || is_repository_cover_identifier(line) {
        return false;
    }
    if is_disposable_contents_or_index_line(line) || line.repeated_header_footer {
        return false;
    }
    if line.page_contents_like && !contents_like_page_line_can_be_marginalia(line) {
        return false;
    }
    let y0 = line.y0_ratio();
    let width_ratio = (line.right - line.left) / line.page_width.max(1.0);
    let bare_marker = is_bare_numeric_note_marker(&line.text);
    let narrow_numeric_fragment = starts_with_note_marker(&line.text)
        && starts_with_numeric_lowercase_body_fragment(&line.text)
        && width_ratio <= 0.28;
    let small_font_numeric_fragment = starts_with_note_marker(&line.text)
        && starts_with_numeric_lowercase_body_fragment(&line.text)
        && line.font_ratio_page_ref <= 0.82
        && width_ratio <= 0.80
        && y0 >= 0.45;
    if !(bare_marker || narrow_numeric_fragment || small_font_numeric_fragment)
        || !line.sequence_footnote_zone
        || line.below_footnote_divider
        || !(0.20..=0.92).contains(&y0)
        || line.font_ratio_page_ref > 0.94
        || (bare_marker && width_ratio > 0.035)
    {
        return false;
    }
    [prev, next].into_iter().flatten().any(|neighbor| {
        let vertical_gap = (line.y0_ratio() - neighbor.y0_ratio()).abs();
        let left_delta = (line.left - neighbor.left).abs() / line.page_width.max(1.0);
        let font_delta = (line.font_ratio_page_ref - neighbor.font_ratio_page_ref).abs();
        vertical_gap <= 0.045
            && left_delta <= 0.22
            && font_delta <= 0.35
            && sequence_note_run_text_evidence(neighbor)
    })
}

fn sequence_note_run_text_evidence(line: &LayoutLine) -> bool {
    if is_repository_cover_boilerplate(line)
        || is_repository_cover_identifier(line)
        || is_disposable_contents_or_index_line(line)
        || line.repeated_header_footer
    {
        return false;
    }
    let lower = line.text.trim_start().to_ascii_lowercase();
    starts_with_note_marker(&line.text)
        || is_bare_numeric_note_marker(&line.text)
        || lower.starts_with("see ")
        || lower.starts_with("see,")
        || lower.starts_with("cf. ")
        || lower.starts_with("id.")
        || lower.starts_with("ibid")
        || contains_legal_note_cue(&line.text)
        || looks_like_citation_continuation_text(&line.text)
        || looks_like_publication_citation_continuation_text(&line.text)
        || looks_like_footnote_bibliographic_lead_text(&line.text)
        || looks_like_concrete_citation_fragment(&line.text)
        || contains_specific_citation_material_cue(&line.text)
}

fn numeric_citation_fragment_in_note_run_can_be_marginalia(
    line: &LayoutLine,
    prev: Option<&LayoutLine>,
    next: Option<&LayoutLine>,
) -> bool {
    if is_repository_cover_boilerplate(line)
        || is_repository_cover_identifier(line)
        || is_disposable_contents_or_index_line(line)
        || line.repeated_header_footer
    {
        return false;
    }
    if line.page_contents_like && !contents_like_page_line_can_be_marginalia(line) {
        return false;
    }
    let y0 = line.y0_ratio();
    let width_ratio = (line.right - line.left) / line.page_width.max(1.0);
    if !line.sequence_footnote_zone
        || line.below_footnote_divider
        || !(0.45..=0.93).contains(&y0)
        || line.font_ratio_page_ref > 0.90
        || width_ratio > 0.60
        || !looks_like_numeric_citation_fragment(&line.text)
    {
        return false;
    }
    [prev, next].into_iter().flatten().any(|neighbor| {
        let vertical_gap = (line.y0_ratio() - neighbor.y0_ratio()).abs();
        let left_delta = (line.left - neighbor.left).abs() / line.page_width.max(1.0);
        vertical_gap <= 0.06
            && left_delta <= 0.22
            && (sequence_note_run_text_evidence(neighbor)
                || contains_statutory_context_cue(&neighbor.text))
    })
}

fn below_divider_small_font_continuation_can_be_marginalia(
    line: &LayoutLine,
    prev: Option<&LayoutLine>,
    next: Option<&LayoutLine>,
) -> bool {
    if is_repository_cover_boilerplate(line)
        || is_repository_cover_identifier(line)
        || is_disposable_contents_or_index_line(line)
        || line.repeated_header_footer
        || looks_like_short_all_caps_note_run_clutter(&line.text)
    {
        return false;
    }
    if line.page_contents_like && !contents_like_page_line_can_be_marginalia(line) {
        return false;
    }
    let y0 = line.y0_ratio();
    let width_ratio = (line.right - line.left) / line.page_width.max(1.0);
    if !line.below_footnote_divider
        || !(0.55..=0.92).contains(&y0)
        || line.font_ratio_page_ref > 0.86
        || width_ratio > 0.82
        || word_count(&line.text) <= 1
    {
        return false;
    }
    [prev, next].into_iter().flatten().any(|neighbor| {
        (line.y0_ratio() - neighbor.y0_ratio()).abs() <= 0.035
            && neighbor.font_ratio_page_ref <= 0.90
    })
}

fn small_font_note_run_continuation_can_be_marginalia(
    line: &LayoutLine,
    prev: Option<&LayoutLine>,
    next: Option<&LayoutLine>,
) -> bool {
    if is_repository_cover_boilerplate(line)
        || is_repository_cover_identifier(line)
        || is_disposable_contents_or_index_line(line)
        || line.repeated_header_footer
    {
        return false;
    }
    if line.page_contents_like && !contents_like_page_line_can_be_marginalia(line) {
        return false;
    }
    let y0 = line.y0_ratio();
    let width_ratio = (line.right - line.left) / line.page_width.max(1.0);
    if !line.sequence_footnote_zone
        || line.below_footnote_divider
        || !(0.45..=0.91).contains(&y0)
        || line.font_ratio_page_ref > 0.82
        || width_ratio > 0.80
        || looks_like_short_all_caps_note_run_clutter(&line.text)
    {
        return false;
    }
    [prev, next].into_iter().flatten().any(|neighbor| {
        let vertical_gap = (line.y0_ratio() - neighbor.y0_ratio()).abs();
        vertical_gap <= 0.045
            && (sequence_note_run_text_evidence(neighbor)
                || contains_statutory_context_cue(&neighbor.text))
    })
}

fn looks_like_short_all_caps_note_run_clutter(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return true;
    }
    let words = word_count(trimmed);
    if words <= 3
        && uppercase_ratio(trimmed) >= 0.80
        && !trimmed.chars().any(|ch| ch.is_ascii_digit())
    {
        let lower = trimmed.to_ascii_lowercase();
        return !(lower.contains("u.s")
            || lower.contains("rev")
            || lower.contains("stat")
            || lower.contains("dep't"));
    }
    let compact = trimmed.replace(' ', "");
    compact.len() <= 3
        && compact
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '~')
}

fn looks_like_numeric_citation_fragment(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() || !trimmed.starts_with(|ch: char| ch.is_ascii_digit()) {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    let dotted_numeric = starts_with_dotted_numeric_fragment(trimmed);
    let statute_parenthetical = starts_with_number_then_parenthetical_section(trimmed);
    if dotted_numeric {
        return trimmed.contains('(')
            || trimmed.contains(',')
            || trimmed.contains('-')
            || lower.contains(" at ")
            || lower.contains("supp.")
            || dotted_numeric_fragment_count(trimmed) >= 2;
    }
    statute_parenthetical
}

fn starts_with_dotted_numeric_fragment(text: &str) -> bool {
    let mut chars = text.chars().peekable();
    let mut digits = 0usize;
    while chars.peek().is_some_and(|ch| ch.is_ascii_digit()) {
        digits += 1;
        chars.next();
    }
    if !(1..=5).contains(&digits) || chars.next() != Some('.') {
        return false;
    }
    chars.peek().is_some_and(|ch| ch.is_ascii_digit())
}

fn dotted_numeric_fragment_count(text: &str) -> usize {
    let bytes = text.as_bytes();
    bytes
        .windows(3)
        .filter(|window| {
            window[0].is_ascii_digit() && window[1] == b'.' && window[2].is_ascii_digit()
        })
        .count()
}

fn starts_with_number_then_parenthetical_section(text: &str) -> bool {
    let mut chars = text.chars().peekable();
    let mut digits = 0usize;
    while chars.peek().is_some_and(|ch| ch.is_ascii_digit()) {
        digits += 1;
        chars.next();
    }
    if !(1..=4).contains(&digits) {
        return false;
    }
    if !chars.next().is_some_and(|ch| ch.is_whitespace()) {
        return false;
    }
    let mut section_digits = 0usize;
    while chars.peek().is_some_and(|ch| ch.is_ascii_digit()) {
        section_digits += 1;
        chars.next();
    }
    (2..=5).contains(&section_digits) && chars.peek() == Some(&'(')
}

fn contains_statutory_context_cue(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    (lower.contains("section ") && lower.chars().any(|ch| ch.is_ascii_digit()))
        || lower.contains("reg.")
        || lower.contains("rev. stat")
        || lower.contains("civil code")
        || lower.contains("u.s.c")
}

fn contains_specific_citation_material_cue(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    text.contains('\u{00a7}')
        || [
            "rev. stat",
            "c.f.r",
            "u.s.c",
            "code ann",
            "ann. ",
            " para.",
            " ch. ",
            " ed.",
            " eds.",
            "c. wright",
            "a. miller",
            "l. rev",
            "l. j.",
            "law review",
            "hereinafter",
            "http://",
            "https://",
            "www.",
            "perma.cc",
        ]
        .iter()
        .any(|cue| lower.contains(cue))
}

fn numeric_note_fragment_geometry_can_be_marginalia(line: &LayoutLine) -> bool {
    if is_repository_cover_boilerplate(line) || is_repository_cover_identifier(line) {
        return false;
    }
    if is_disposable_contents_or_index_line(line) {
        return false;
    }
    if line.page_contents_like && !contents_like_page_line_can_be_marginalia(line) {
        return false;
    }
    let y0 = line.y0_ratio();
    let width_ratio = (line.right - line.left) / line.page_width.max(1.0);
    if !starts_with_note_marker(&line.text)
        || !starts_with_numeric_lowercase_body_fragment(&line.text)
    {
        return false;
    }
    if line.below_footnote_divider && line.font_ratio_page_ref <= 1.05 {
        return true;
    }
    !line.below_footnote_divider
        && line.sequence_footnote_zone
        && (0.54..=0.90).contains(&y0)
        && line.font_ratio_page_ref <= 0.86
        && width_ratio <= 0.70
}

fn numeric_lowercase_publication_note_start_can_be_marginalia(line: &LayoutLine) -> bool {
    if is_repository_cover_boilerplate(line)
        || is_repository_cover_identifier(line)
        || is_disposable_contents_or_index_line(line)
    {
        return false;
    }
    let Some(body) = numeric_note_marker_body(&line.text) else {
        return false;
    };
    if !starts_with_note_marker(&line.text)
        || !body
            .chars()
            .find(|ch| ch.is_alphabetic())
            .is_some_and(char::is_lowercase)
    {
        return false;
    }
    let y0 = line.y0_ratio();
    if !line.sequence_footnote_zone
        || !(0.55..=0.90).contains(&y0)
        || line.font_ratio_page_ref > 0.86
    {
        return false;
    }
    let uppercase = body.chars().filter(|ch| ch.is_ascii_uppercase()).count();
    uppercase >= 5
        && (body.contains(',') || body.contains('.'))
        && contains_month_abbreviation(body)
}

fn numeric_year_parenthetical_continuation_can_be_marginalia(
    hints: &[LiquidLayoutHint],
    line: &LayoutLine,
    prev: Option<&LayoutLine>,
    next: Option<&LayoutLine>,
) -> bool {
    if is_repository_cover_boilerplate(line)
        || is_repository_cover_identifier(line)
        || is_disposable_contents_or_index_line(line)
        || line.page_contents_like
        || !line.sequence_footnote_zone
        || line.below_footnote_divider
        || line.font_ratio_page_ref > 0.92
        || !(0.55..=0.90).contains(&line.y0_ratio())
        || !looks_like_numeric_year_parenthetical_continuation(&line.text)
    {
        return false;
    }

    let prev_supports = prev
        .is_some_and(|neighbor| adjacent_note_context_supports_continuation(hints, neighbor, line));
    let next_supports = next.is_some_and(|neighbor| {
        matches!(
            hint_role_for_line(hints, neighbor),
            Some(LiquidBlockRole::Marginalia)
        ) && adjacent_geometry_supports_continuation(neighbor, line)
    });
    prev_supports && next_supports
}

fn looks_like_numeric_year_parenthetical_continuation(text: &str) -> bool {
    let trimmed = text.trim_start();
    let mut chars = trimmed.char_indices();
    let mut year = 0usize;
    let mut digit_count = 0usize;
    let mut marker_end = 0usize;
    for (index, ch) in chars.by_ref() {
        if ch.is_ascii_digit() {
            digit_count += 1;
            year = year
                .saturating_mul(10)
                .saturating_add((ch as u8 - b'0') as usize);
            marker_end = index + ch.len_utf8();
            continue;
        }
        break;
    }
    if digit_count != 4 || !(1900..=2099).contains(&year) {
        return false;
    }
    if trimmed[marker_end..].chars().next() != Some(')') {
        return false;
    }
    let rest = trimmed[marker_end + 1..].trim_start();
    rest.starts_with('(') || rest.starts_with(|ch: char| ch.is_ascii_lowercase())
}

fn numeric_page_parenthetical_citation_continuation_can_be_marginalia(
    hints: &[LiquidLayoutHint],
    line: &LayoutLine,
    prev: Option<&LayoutLine>,
    next: Option<&LayoutLine>,
) -> bool {
    if is_repository_cover_boilerplate(line)
        || is_repository_cover_identifier(line)
        || is_disposable_contents_or_index_line(line)
        || line.page_contents_like
        || !line.sequence_footnote_zone
        || line.below_footnote_divider
        || line.font_ratio_page_ref > 0.92
        || !(0.55..=0.90).contains(&line.y0_ratio())
        || !looks_like_numeric_page_parenthetical_citation_continuation(&line.text)
    {
        return false;
    }

    let prev_supports = prev
        .is_some_and(|neighbor| adjacent_note_context_supports_continuation(hints, neighbor, line));
    let next_supports = next.is_some_and(|neighbor| {
        matches!(
            hint_role_for_line(hints, neighbor),
            Some(LiquidBlockRole::Marginalia)
        ) && adjacent_geometry_supports_continuation(neighbor, line)
    });
    prev_supports && next_supports
}

fn looks_like_numeric_page_parenthetical_citation_continuation(text: &str) -> bool {
    let trimmed = text.trim_start();
    let mut digits = 0usize;
    let mut marker_end = 0usize;
    for (index, ch) in trimmed.char_indices() {
        if ch.is_ascii_digit() {
            digits += 1;
            marker_end = index + ch.len_utf8();
            continue;
        }
        break;
    }
    if !(1..=4).contains(&digits) {
        return false;
    }
    let rest = trimmed[marker_end..].trim_start();
    let Some(parenthetical) = rest.strip_prefix('(') else {
        return false;
    };
    let Some(close_index) = parenthetical.find(')') else {
        return false;
    };
    let inside = parenthetical[..close_index].trim().to_ascii_lowercase();
    if inside.is_empty() || inside.len() > 40 {
        return false;
    }
    inside.contains("19")
        || inside.contains("20")
        || ["ed.", "cir.", "dist.", "app.", "supp.", "dept.", "dept"]
            .iter()
            .any(|cue| inside.contains(cue))
}

fn contents_like_author_credential_continuation_can_be_marginalia(line: &LayoutLine) -> bool {
    if !line.page_contents_like || line.contents_or_index_entry {
        return false;
    }
    let y0 = line.y0_ratio();
    let width_ratio = (line.right - line.left) / line.page_width.max(1.0);
    if !(0.62..=0.88).contains(&y0) || line.font_ratio_page_ref > 0.82 || width_ratio > 0.75 {
        return false;
    }
    let lower = line.text.to_ascii_lowercase();
    line.text.contains(';')
        && (lower.contains("j.d.") || lower.contains("l.l.m") || lower.contains("ll.m"))
        && (lower.contains("univer") || lower.contains("law center"))
}

fn contents_like_tiny_citation_fragment_can_be_marginalia(line: &LayoutLine) -> bool {
    if !line.page_contents_like || line.contents_or_index_entry {
        return false;
    }
    let y0 = line.y0_ratio();
    let width_ratio = (line.right - line.left) / line.page_width.max(1.0);
    (0.65..=0.90).contains(&y0)
        && line.font_ratio_page_ref <= 0.84
        && width_ratio <= 0.08
        && looks_like_concrete_citation_fragment(&line.text)
}

fn contains_month_abbreviation(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    [
        " jan.", " feb.", " mar.", " apr.", " may ", " jun.", " jul.", " aug.", " sep.", " sept.",
        " oct.", " nov.", " dec.",
    ]
    .iter()
    .any(|month| lower.contains(month))
}

fn looks_like_administrative_status_or_update_line(text: &str) -> bool {
    let trimmed = text.trim();
    let lower = trimmed.to_ascii_lowercase();
    if [
        "passed:", "absent:", "present:", "aye:", "ayes:", "nay:", "nays:", "vote:",
    ]
    .iter()
    .any(|prefix| lower.starts_with(prefix))
    {
        return true;
    }

    if lower.starts_with("notification to the jurisdiction")
        || (lower.contains("plan amendment")
            && (lower.contains("jurisdiction") || lower.contains("acknowledged")))
        || lower.contains("regional representative")
        || lower.contains(" dlcd ")
        || lower.starts_with("dlcd ")
        || lower.ends_with(" dlcd")
    {
        return !contains_legal_note_cue(trimmed);
    }

    lower.starts_with("updated ")
        && contains_full_month_name_or_abbreviation(trimmed)
        && contains_four_digit_year(trimmed)
}

fn looks_like_uncited_all_caps_topic_heading(text: &str) -> bool {
    let trimmed = text.trim();
    let words = word_count(trimmed);
    if !(2..=8).contains(&words)
        || starts_with_note_marker(trimmed)
        || contains_legal_note_cue(trimmed)
        || looks_like_reporter_citation_text(trimmed)
        || trimmed.chars().any(|ch| ch.is_ascii_digit())
        || trimmed.chars().any(|ch| {
            matches!(
                ch,
                '.' | ',' | ';' | ':' | '(' | ')' | '[' | ']' | '&' | '§'
            )
        })
    {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    if ["journal", "review", "revue", "press", "university", "univ."]
        .iter()
        .any(|cue| lower.contains(cue))
    {
        return false;
    }
    uppercase_ratio(trimmed) >= 0.75
}

fn contains_full_month_name_or_abbreviation(text: &str) -> bool {
    let lower = format!(" {} ", text.to_ascii_lowercase());
    [
        " january ",
        " jan ",
        " jan. ",
        " february ",
        " feb ",
        " feb. ",
        " march ",
        " mar ",
        " mar. ",
        " april ",
        " apr ",
        " apr. ",
        " may ",
        " june ",
        " jun ",
        " jun. ",
        " july ",
        " jul ",
        " jul. ",
        " august ",
        " aug ",
        " aug. ",
        " september ",
        " sep ",
        " sep. ",
        " sept ",
        " sept. ",
        " october ",
        " oct ",
        " oct. ",
        " november ",
        " nov ",
        " nov. ",
        " december ",
        " dec ",
        " dec. ",
    ]
    .iter()
    .any(|month| lower.contains(month))
}

fn contains_four_digit_year(text: &str) -> bool {
    text.split(|ch: char| !ch.is_ascii_digit())
        .filter(|part| part.len() == 4)
        .any(|part| {
            part.parse::<u16>()
                .is_ok_and(|year| (1900..=2099).contains(&year))
        })
}

fn adjacent_note_context_supports_continuation(
    hints: &[LiquidLayoutHint],
    neighbor: &LayoutLine,
    line: &LayoutLine,
) -> bool {
    matches!(
        hint_role_for_line(hints, neighbor),
        Some(LiquidBlockRole::Marginalia)
    ) && adjacent_geometry_supports_continuation(neighbor, line)
}

fn adjacent_geometry_supports_continuation(neighbor: &LayoutLine, line: &LayoutLine) -> bool {
    let vertical_gap = (line.y0_ratio() - neighbor.y0_ratio()).abs();
    let left_delta = ((line.left - neighbor.left).abs() / line.page_width.max(1.0)).max(0.0);
    let font_delta = (line.font_ratio_page_ref - neighbor.font_ratio_page_ref).abs();
    vertical_gap <= 0.045 && left_delta <= 0.14 && font_delta <= 0.30
}

fn adjacent_numeric_fragment_geometry_supports_continuation(
    neighbor: &LayoutLine,
    line: &LayoutLine,
) -> bool {
    let vertical_gap = (line.y0_ratio() - neighbor.y0_ratio()).abs();
    let left_delta = ((line.left - neighbor.left).abs() / line.page_width.max(1.0)).max(0.0);
    let font_delta = (line.font_ratio_page_ref - neighbor.font_ratio_page_ref).abs();
    vertical_gap <= 0.045 && left_delta <= 0.22 && font_delta <= 0.35
}

fn footnote_specialist_line_can_be_marginalia(line: &LayoutLine) -> bool {
    if is_repository_cover_boilerplate(line) || is_repository_cover_identifier(line) {
        return false;
    }
    if looks_like_edge_case_name_running_header_fragment(line) {
        return false;
    }
    if is_disposable_contents_or_index_line(line) {
        return false;
    }
    if line.page_contents_like && !contents_like_page_line_can_be_marginalia(line) {
        return false;
    }
    if looks_like_running_law_review_cite_line(&line.text) {
        return false;
    }
    if is_plain_page_number_line(&line.text) && !is_plain_numeric_footnote_marker_candidate(line) {
        return false;
    }
    if sequence_zone_body_enumeration_marker(line) {
        return false;
    }
    if normal_font_top_half_sequence_body_line(line) {
        return false;
    }
    if normal_font_inline_body_citation_continuation(line) {
        return false;
    }
    if normal_font_body_fragment_before_inline_note_marker(line) {
        return false;
    }
    if looks_like_administrative_status_or_update_line(&line.text) {
        return false;
    }
    if line_should_never_be_marginalia_by_body_geometry(line) {
        return false;
    }
    if line.font_ratio_page_ref >= 0.90 && looks_like_uncited_all_caps_topic_heading(&line.text) {
        return false;
    }
    if starts_with_quoted_citation_fragment(&line.text)
        && !line.below_footnote_divider
        && !line.sequence_footnote_zone
    {
        return false;
    }
    if small_font_bibliographic_lead_can_be_marginalia(line) {
        return true;
    }
    if small_font_sequence_citation_material_can_be_marginalia(line) {
        return true;
    }
    if numeric_note_fragment_geometry_can_be_marginalia(line) {
        return true;
    }
    if starts_with_numeric_lowercase_body_fragment(&line.text) {
        return false;
    }
    let note_marker = starts_with_note_marker(&line.text);
    let note_cue = note_marker || contains_legal_note_cue(&line.text);
    if word_count(&line.text) <= 1
        && !note_cue
        && !line.sequence_footnote_zone
        && !line.below_footnote_divider
    {
        return false;
    }
    if !has_plausible_footnote_geometry(line) {
        return false;
    }
    if !note_marker && looks_like_clear_section_heading(&line.text) {
        return false;
    }
    if line.y0_ratio() > 0.94 && word_count(&line.text) <= 5 && !note_cue {
        return false;
    }
    if line.sequence_footnote_zone && !line.below_footnote_divider {
        return line.font_ratio_page_ref <= 0.92 || note_cue;
    }
    true
}

fn model_line_should_be_marginalia(line: &LayoutLine) -> bool {
    if is_repository_cover_boilerplate(line) || is_repository_cover_identifier(line) {
        return false;
    }
    if looks_like_edge_case_name_running_header_fragment(line) {
        return false;
    }
    if is_disposable_contents_or_index_line(line) {
        return false;
    }
    if line.page_contents_like && !contents_like_page_line_can_be_marginalia(line) {
        return false;
    }
    if looks_like_running_law_review_cite_line(&line.text) {
        return false;
    }
    if is_plain_page_number_line(&line.text) && !is_plain_numeric_footnote_marker_candidate(line) {
        return false;
    }
    if sequence_zone_body_enumeration_marker(line) {
        return false;
    }
    if normal_font_top_half_sequence_body_line(line) {
        return false;
    }
    if normal_font_inline_body_citation_continuation(line) {
        return false;
    }
    if normal_font_body_fragment_before_inline_note_marker(line) {
        return false;
    }
    if looks_like_administrative_status_or_update_line(&line.text) {
        return false;
    }
    if line_should_never_be_marginalia_by_body_geometry(line) {
        return false;
    }
    if line.font_ratio_page_ref >= 0.90 && looks_like_uncited_all_caps_topic_heading(&line.text) {
        return false;
    }
    if starts_with_quoted_citation_fragment(&line.text)
        && !line.below_footnote_divider
        && !line.sequence_footnote_zone
    {
        return false;
    }
    if small_font_bibliographic_lead_can_be_marginalia(line) {
        return true;
    }
    if small_font_sequence_citation_material_can_be_marginalia(line) {
        return true;
    }
    if numeric_note_fragment_geometry_can_be_marginalia(line) {
        return true;
    }
    if starts_with_numeric_lowercase_body_fragment(&line.text) {
        return false;
    }
    let note_marker = starts_with_note_marker(&line.text);
    let note_cue = note_marker || contains_legal_note_cue(&line.text);
    if word_count(&line.text) <= 1
        && !note_cue
        && !line.sequence_footnote_zone
        && !line.below_footnote_divider
    {
        return false;
    }
    if !note_marker && looks_like_clear_section_heading(&line.text) {
        return false;
    }
    if line.y0_ratio() > 0.94 && word_count(&line.text) <= 5 && !note_cue {
        return false;
    }
    if !has_plausible_footnote_geometry(line) {
        return false;
    }
    if line.below_footnote_divider && line.font_ratio_page_ref <= 1.05 {
        return true;
    }
    if line.sequence_footnote_zone && line.font_ratio_page_ref <= 1.02 {
        return line.font_ratio_page_ref <= 0.92 || note_cue;
    }
    if note_cue {
        return true;
    }
    false
}

fn has_plausible_footnote_geometry(line: &LayoutLine) -> bool {
    let note_marker = starts_with_note_marker(&line.text);
    let symbol_note_marker = starts_with_symbol_note_marker(&line.text);
    let compact_legal_note_marker = starts_with_compact_legal_note_marker(&line.text);
    let general_citation_note_start = looks_like_general_citation_note_start(&line.text);
    let strong_legal_note_cue = contains_strong_legal_note_cue(&line.text);
    if line.below_footnote_divider && line.font_ratio_page_ref <= 1.05 {
        return true;
    }
    if line.sequence_footnote_zone && line.font_ratio_page_ref <= 1.02 {
        return true;
    }
    if general_citation_note_start && line.y0_ratio() >= 0.22 && line.font_ratio_page_ref <= 0.95 {
        return true;
    }
    if compact_legal_note_marker && line.y0_ratio() >= 0.25 && line.font_ratio_page_ref <= 1.02 {
        return true;
    }
    if symbol_note_marker
        && line.page_index == 0
        && line.y0_ratio() >= 0.35
        && line.font_ratio_page_ref <= 0.98
    {
        return true;
    }
    if strong_legal_note_cue && line.font_ratio_page_ref <= 0.98 {
        return true;
    }
    if note_marker && line.y0_ratio() >= 0.55 && line.font_ratio_page_ref <= 0.98 {
        return true;
    }
    if strong_legal_note_cue && line.y0_ratio() >= 0.55 {
        return true;
    }
    false
}

fn sequence_zone_body_enumeration_marker(line: &LayoutLine) -> bool {
    is_bare_numeric_note_marker(&line.text)
        && line.page_index > 0
        && line.sequence_footnote_zone
        && !line.below_footnote_divider
        && line.y0_ratio() < 0.70
        && line.font_ratio_page_ref > 0.86
}

fn normal_font_top_half_sequence_body_line(line: &LayoutLine) -> bool {
    if !line.sequence_footnote_zone
        || line.page_index == 0
        || line.below_footnote_divider
        || line.y0_ratio() >= 0.55
        || line.font_ratio_page_ref < 0.95
    {
        return false;
    }
    let width_ratio = (line.right - line.left) / line.page_width.max(1.0);
    if width_ratio <= 0.50 || starts_with_note_marker(&line.text) {
        return false;
    }
    let lower = line.text.trim_start().to_ascii_lowercase();
    !(lower.starts_with("see ")
        || lower.starts_with("see,")
        || lower.starts_with("cf. ")
        || lower.starts_with("id.")
        || lower.starts_with("id ")
        || lower.starts_with("ibid"))
}

fn normal_font_inline_body_citation_continuation(line: &LayoutLine) -> bool {
    if line.page_index == 0
        || line.below_footnote_divider
        || line.sequence_footnote_zone
        || line.font_ratio_page_ref < 0.95
        || !(0.35..=0.82).contains(&line.y0_ratio())
        || line.width_to_body_ratio < 0.60
        || !starts_with_lowercase_letter(&line.text)
        || !looks_like_citation_continuation_text(&line.text)
        || starts_with_note_marker(&line.text)
        || looks_like_general_citation_note_start(&line.text)
    {
        return false;
    }

    line.prev_line_present
        && line.next_line_present
        && !line.prev_sequence_footnote_zone
        && !line.next_sequence_footnote_zone
        && !line.prev_below_footnote_divider
        && !line.next_below_footnote_divider
        && line.prev_y_gap_ratio <= 0.035
        && line.next_y_gap_ratio <= 0.035
        && line.prev_left_delta_ratio <= 0.06
        && line.next_left_delta_ratio <= 0.06
}

fn normal_font_body_fragment_before_inline_note_marker(line: &LayoutLine) -> bool {
    if line.page_index == 0
        || !line.sequence_footnote_zone
        || line.below_footnote_divider
        || !(0.45..=0.72).contains(&line.y0_ratio())
        || line.font_ratio_page_ref < 0.95
        || (line.right - line.left) / line.page_width.max(1.0) > 0.35
        || !starts_with_lowercase_letter(&line.text)
        || contains_legal_note_cue(&line.text)
        || starts_with_note_marker(&line.text)
    {
        return false;
    }

    line.prev_line_present
        && line.prev_sequence_footnote_zone
        && line.prev_note_marker
        && line.prev_y_gap_ratio <= 0.006
        && (0.12..=0.35).contains(&line.prev_left_delta_ratio)
}

fn small_font_bibliographic_lead_can_be_marginalia(line: &LayoutLine) -> bool {
    let y0 = line.y0_ratio();
    (0.55..=0.90).contains(&y0)
        && line.font_ratio_page_ref <= 0.75
        && !looks_like_clear_section_heading(&line.text)
        && looks_like_footnote_bibliographic_lead_text(&line.text)
}

fn contents_like_page_line_can_be_marginalia(line: &LayoutLine) -> bool {
    if !line.page_contents_like {
        return true;
    }
    let y0 = line.y0_ratio();
    if line.below_footnote_divider && y0 >= 0.45 && line.font_ratio_page_ref <= 1.05 {
        return true;
    }
    if line.sequence_footnote_zone
        && y0 >= 0.55
        && line.font_ratio_page_ref <= 0.90
        && (starts_with_note_marker(&line.text) || contains_legal_note_cue(&line.text))
    {
        return true;
    }
    if !line.contents_or_index_entry
        && y0 >= 0.62
        && line.font_ratio_page_ref <= 0.84
        && !is_contents_line(&line.text)
        && !looks_like_clear_section_heading(&line.text)
    {
        return true;
    }
    if starts_with_note_marker(&line.text) && y0 >= 0.62 && line.font_ratio_page_ref <= 0.92 {
        return true;
    }
    false
}

fn is_repository_cover_boilerplate(line: &LayoutLine) -> bool {
    if starts_with_note_marker(&line.text) {
        return false;
    }
    let lower = normalize_model_text(&line.text);
    if lower.ends_with("law review") && word_count(&line.text) <= 6 {
        return true;
    }
    if line.page_index > 1 {
        return false;
    }
    lower == "recommended citation"
        || lower == "repository citation"
        || lower.starts_with("electronic copy available at:")
        || lower.starts_with("electronic copy available at ")
        || lower.contains("law review:") && (lower.contains("vol.") || lower.contains(" no."))
        || lower.strip_prefix("article ").is_some_and(|rest| {
            rest.trim_end_matches('.')
                .chars()
                .all(|ch| ch.is_ascii_digit())
        })
        || lower.contains("librarian@")
        || lower.contains("repository@")
        || lower.contains("law-library@")
        || lower.contains("commons. for more information, please contact")
        || lower.contains("law reviews and journals")
            && (lower.contains("brought to you") || lower.contains("accepted for"))
        || lower.contains("law digital commons")
        || lower.contains("law school digital commons")
        || lower.contains("law ecommons")
        || lower.contains("scholar commons")
            && (lower.contains("brought to you") || lower.contains("accepted for"))
        || lower.contains("inclusion in") && lower.contains("scholar commons")
        || lower.starts_with("contact ") && lower.contains('@')
        || lower.contains("please contact ") && lower.contains('@')
        || lower.starts_with("follow this and additional works at")
        || lower.starts_with("part of the") && lower.contains(" commons")
        || lower.starts_with("available at:")
            && (lower.contains("digitalcommons")
                || lower.contains("scholarship.law")
                || lower.contains("/lawreview/")
                || lower.contains("lawreview/"))
        || lower.contains(" law review:") && lower.contains("article") && lower.contains('(')
        || lower.starts_with("recent decisions,")
            && lower.contains(" l. rev.")
            && lower.contains(',')
            && lower.contains('(')
        || line.below_footnote_divider
            && (lower.contains(" law review,")
                || lower.contains(" law review:")
                || lower.contains(" law review by "))
            && (lower.contains('(') || lower.contains("available at:") || lower.contains("lawyer"))
        || lower.contains("brought to you for free and open access")
        || lower.contains("brought to you by") && lower.contains("scholar commons")
        || lower.contains("accepted for inclusion")
        || lower.contains("authorized administrator")
            && (lower.contains("digital commons")
                || lower.contains("scholarly commons")
                || lower.contains("scholar commons")
                || lower.contains("ecommons"))
}

fn model_noise_line_can_be_hidden(page: &PageInfo, line: &LayoutLine) -> bool {
    if looks_like_running_law_review_cite_line(&line.text) {
        return true;
    }
    if is_repository_cover_boilerplate(line) || is_repository_cover_identifier(line) {
        return true;
    }
    if is_disposable_contents_or_index_line(line) {
        return true;
    }
    if is_plain_numeric_footnote_marker_candidate(line) {
        return false;
    }
    if looks_like_edge_running_header_footer_fragment(line) {
        return true;
    }
    if starts_with_note_marker(&line.text) {
        return false;
    }
    if line.repeated_header_footer {
        return true;
    }
    if contains_legal_note_cue(&line.text) {
        return false;
    }
    if looks_like_nonlegal_study_prompt_noise(line) {
        return true;
    }
    if is_contents_line(&line.text) {
        return true;
    }

    let y0 = y_from_top(page, line) / page.height.max(1.0);
    let y1 = (page.height - line.bottom).clamp(0.0, page.height.max(1.0)) / page.height.max(1.0);
    let words = word_count(&line.text);
    if is_plain_page_number(&normalize_model_text(&line.text)) && (y0 <= 0.12 || y1 >= 0.88) {
        return true;
    }

    let edge_line = y0 <= 0.08 || y1 >= 0.94;
    edge_line && words <= 8 && line.font_ratio_page_ref <= 1.05
}

fn is_repository_cover_identifier(line: &LayoutLine) -> bool {
    if line.page_index > 1 || starts_with_note_marker(&line.text) {
        return false;
    }
    let lower = normalize_model_text(&line.text);
    if lower == "recommended citation" || lower == "repository citation" {
        return true;
    }
    let mut parts = lower.split_whitespace();
    let Some(first) = parts.next() else {
        return false;
    };
    let Some(second) = parts.next() else {
        return false;
    };
    matches!(
        first,
        "volume" | "vol." | "issue" | "number" | "no." | "article"
    ) && second
        .trim_end_matches('.')
        .chars()
        .all(|ch| ch.is_ascii_digit())
        && word_count(&line.text) <= 8
}

fn is_disposable_contents_or_index_line(line: &LayoutLine) -> bool {
    let text = &line.text;
    if looks_like_punctuation_rule_noise(text) {
        return true;
    }
    if looks_like_contents_heading_text(text) || looks_like_dot_leader_contents_line(text) {
        return true;
    }
    if is_contents_line(text) && !contains_legal_note_cue(text) {
        return true;
    }
    if looks_like_orphan_contents_page_fragment_noise(line) {
        return true;
    }
    if looks_like_sequence_zone_orphan_contents_fragment_noise(line) {
        return true;
    }
    if looks_like_table_index_page_number_noise(line) {
        return true;
    }
    if looks_like_index_cross_reference_name_noise(line) {
        return true;
    }
    line.page_contents_like
        && (line.contents_or_index_entry
            || looks_like_dot_leader_contents_fragment(text)
            || is_plain_page_number_line(text))
}

fn looks_like_orphan_contents_page_fragment_noise(line: &LayoutLine) -> bool {
    if !line.page_contents_like
        || line.sequence_footnote_zone
        || line.below_footnote_divider
        || line.font_ratio_page_ref < 0.86
        || starts_with_note_marker(&line.text)
        || contains_legal_note_cue(&line.text)
        || word_count(&line.text) > 4
    {
        return false;
    }

    let lower = normalize_model_text(&line.text);
    (lower.starts_with("and ") && word_count(&line.text) <= 4)
        || (lower.ends_with('?') && word_count(&line.text) <= 2)
}

fn looks_like_sequence_zone_orphan_contents_fragment_noise(line: &LayoutLine) -> bool {
    let text = line.text.trim();
    let words = word_count(text);
    if !line.sequence_footnote_zone
        || line.below_footnote_divider
        || line.prev_note_marker
        || line.font_ratio_page_ref < 0.88
        || line.font_ratio_page_ref > 1.02
        || !(0.55..=0.85).contains(&line.y0_ratio())
        || (line.right - line.left) / line.page_width.max(1.0) > 0.35
        || starts_with_note_marker(text)
        || contains_legal_note_cue(text)
        || words > 4
    {
        return false;
    }
    let lower = normalize_model_text(text);
    if lower.starts_with("and ") && words <= 4 {
        let rest = text
            .split_whitespace()
            .skip(1)
            .collect::<Vec<_>>()
            .join(" ");
        return title_case_ratio(&rest) >= 0.45;
    }
    lower.ends_with('?') && words <= 2
}

fn looks_like_table_index_page_number_noise(line: &LayoutLine) -> bool {
    let text = line.text.trim();
    let width_ratio = (line.right - line.left) / line.page_width.max(1.0);
    is_plain_page_number_line(text)
        && (0.80..=0.88).contains(&line.font_ratio_page_ref)
        && (0.68..=0.78).contains(&line.y0_ratio())
        && width_ratio <= 0.05
        && line.prev_sequence_footnote_zone
        && !line.prev_note_marker
        && !line.below_footnote_divider
}

fn looks_like_index_cross_reference_name_noise(line: &LayoutLine) -> bool {
    let text = line.text.trim();
    let width_ratio = (line.right - line.left) / line.page_width.max(1.0);
    if !line.sequence_footnote_zone
        || line.below_footnote_divider
        || line.prev_sequence_footnote_zone
        || line.prev_note_marker
        || !(0.55..=0.70).contains(&line.y0_ratio())
        || !(0.90..=0.98).contains(&line.font_ratio_page_ref)
        || width_ratio > 0.30
    {
        return false;
    }
    let Some(rest) = text.strip_prefix("See ") else {
        return false;
    };
    let Some((surname, given)) = rest.split_once(", ") else {
        return false;
    };
    valid_index_name_token(surname) && valid_index_name_token(given.trim_end_matches('.'))
}

fn valid_index_name_token(text: &str) -> bool {
    let mut chars = text.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_uppercase()
        && text.chars().count() >= 3
        && chars.all(|ch| {
            ch.is_ascii_alphabetic() || matches!(ch, '\'' | '\u{2019}' | '\u{2013}' | '-')
        })
}

fn looks_like_punctuation_rule_noise(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() || starts_with_symbol_note_marker(trimmed) {
        return false;
    }
    let mut rule_chars = 0usize;
    for ch in trimmed.chars() {
        if ch.is_ascii_alphanumeric() {
            return false;
        }
        if matches!(
            ch,
            '.' | '-' | '_' | '=' | '\u{2013}' | '\u{2014}' | '\u{2026}'
        ) {
            rule_chars += 1;
            continue;
        }
        if ch.is_whitespace() || ch.is_ascii_punctuation() {
            continue;
        }
        return false;
    }
    rule_chars > 0
}

fn looks_like_nonlegal_study_prompt_noise(line: &LayoutLine) -> bool {
    if line.below_footnote_divider
        || line.font_ratio_page_ref < 0.93
        || line.font_ratio_page_ref > 1.08
        || line.y0_ratio() < 0.45
        || line.y0_ratio() > 0.95
        || starts_with_note_marker(&line.text)
        || contains_legal_note_cue(&line.text)
        || word_count(&line.text) > 8
    {
        return false;
    }

    let lower = normalize_model_text(&line.text);
    let philosophical_prompt_term = lower.contains("truth")
        || lower.contains("coherence")
        || lower.contains("validity")
        || lower.contains("utility")
        || lower.contains("pragmatic criterion");
    let ocr_prompt_artifact = lower.contains("t1")
        || lower.contains("ra,t")
        || lower.contains("-:")
        || lower.contains(" oi ")
        || lower.contains("vori")
        || lower.contains("ll?");
    let short_question_prompt = lower.ends_with('?')
        && (lower.starts_with("what ")
            || lower.starts_with("why ")
            || lower.starts_with("how ")
            || lower.starts_with("when ")
            || lower.starts_with("where "))
        && (philosophical_prompt_term || ocr_prompt_artifact);
    let stage_prompt = lower.contains("stage")
        && (lower.contains("coherence") || lower.contains("criterion") || ocr_prompt_artifact);

    short_question_prompt || stage_prompt || (ocr_prompt_artifact && philosophical_prompt_term)
}

fn looks_like_clear_section_heading(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() || word_count(trimmed) > 14 || trimmed.contains('@') {
        return false;
    }
    if looks_like_known_section_heading_text(trimmed) {
        return true;
    }
    let Some((marker, rest)) = trimmed.split_once(char::is_whitespace) else {
        return false;
    };
    let marker = marker.trim_matches(['.', ')', ']', '(']);
    (is_roman_heading_marker(marker) || is_letter_heading_marker(marker))
        && looks_like_section_heading_remainder(rest)
}

fn looks_like_known_section_heading_text(text: &str) -> bool {
    matches!(
        normalize_model_text(text).as_str(),
        "abstract"
            | "background"
            | "conclusion"
            | "conclusions"
            | "discussion"
            | "introduction"
            | "methodology"
            | "overview"
            | "references"
            | "table of contents"
    )
}

fn looks_like_section_heading_remainder(text: &str) -> bool {
    let trimmed = text.trim().trim_end_matches(':');
    if trimmed.is_empty() || word_count(trimmed) > 12 || trimmed.ends_with('.') {
        return false;
    }
    let normalized = normalize_model_text(trimmed);
    looks_like_known_section_heading_text(&normalized)
        || uppercase_ratio(trimmed) >= 0.45
        || title_case_ratio(trimmed) >= 0.45
}

fn is_roman_heading_marker(text: &str) -> bool {
    let trimmed = text.trim();
    !trimmed.is_empty()
        && trimmed.len() <= 8
        && trimmed
            .chars()
            .all(|ch| matches!(ch, 'I' | 'V' | 'X' | 'L' | 'C' | 'D' | 'M'))
}

fn is_letter_heading_marker(text: &str) -> bool {
    text.len() == 1 && text.chars().all(|ch| ch.is_ascii_uppercase())
}

fn header_footer_specialist_line_can_be_header_footer(line: &LayoutLine) -> bool {
    if line.page_index == 0 || is_repository_cover_boilerplate(line) {
        return false;
    }
    let note_cue = starts_with_note_marker(&line.text) || contains_legal_note_cue(&line.text);
    if note_cue && line.font_ratio_page_ref <= 0.98 {
        return false;
    }
    let y0 = line.y0_ratio();
    let y1 = line.y1_ratio();
    let words = word_count(&line.text);
    line.repeated_header_footer
        || (y0 <= 0.10 && words <= 14)
        || (y1 >= 0.92 && words <= 14)
        || (y0 <= 0.14 && line.font_ratio_page_ref <= 1.05 && words <= 10)
}

fn body_specialist_line_can_be_paragraph(line: &LayoutLine) -> bool {
    if line.text.trim().is_empty()
        || line.repeated_header_footer
        || line.page_contents_like
        || line.below_footnote_divider
        || line.sequence_footnote_zone
        || starts_with_note_marker(&line.text)
        || looks_like_clear_section_heading(&line.text)
    {
        return false;
    }
    line.body_column_like
        && !line.heading_geometry_like
        && line.font_ratio_page_ref >= 0.90
        && line.y0_ratio() >= 0.08
        && line.y1_ratio() <= 0.94
}

fn heading_specialist_line_can_be_heading(line: &LayoutLine) -> bool {
    if line.text.trim().is_empty()
        || line.repeated_header_footer
        || (line.page_contents_like && !page_contents_clear_section_heading_can_be_heading(line))
        || line.below_footnote_divider
        || line.sequence_footnote_zone
        || starts_with_note_marker(&line.text)
        || line_ends_with_period(&line.text)
        || looks_like_law_review_journal_masthead_line(line)
        || early_article_display_title_fragment_should_not_be_heading(line)
        || looks_like_probable_author_line(line)
        || heading_specialist_fragment_should_not_be_heading(line)
        || sentence_case_body_continuation_should_not_be_heading(line)
    {
        return false;
    }
    let words = model_word_count(&line.text);
    (1..=16).contains(&words)
        && (line.heading_geometry_like
            || looks_like_clear_section_heading(&line.text)
            || (line.font_ratio_page_ref >= 0.90
                && looks_like_uncited_all_caps_topic_heading(&line.text))
            || (line.centered && line.font_ratio_body >= 1.02)
            || (line.bold && line.font_ratio_body >= 1.02))
}

fn page_contents_clear_section_heading_can_be_heading(line: &LayoutLine) -> bool {
    if !line.page_contents_like {
        return false;
    }
    let words = model_word_count(&line.text);
    if !(1..=16).contains(&words) {
        return false;
    }
    if line_ends_with_period(&line.text)
        || looks_like_dot_leader_contents_fragment(&line.text)
        || looks_like_dot_leader_contents_line(&line.text)
        || !looks_like_clear_section_heading(&line.text)
    {
        return false;
    }
    if looks_like_common_section_heading_label(&line.text) {
        return true;
    }
    line.heading_geometry_like
        || (line.centered && line.font_ratio_body >= 1.02)
        || (line.bold && line.font_ratio_body >= 1.02)
}

fn looks_like_common_section_heading_label(text: &str) -> bool {
    let normalized = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "introduction"
            | "abstract"
            | "background"
            | "analysis"
            | "discussion"
            | "conclusion"
            | "appendix"
    )
}

fn heading_specialist_fragment_should_not_be_heading(line: &LayoutLine) -> bool {
    if looks_like_clear_section_heading(&line.text) {
        return false;
    }
    if looks_like_numeric_table_cell_fragment(line) {
        return true;
    }
    let words = model_word_count(&line.text);
    if (1..=2).contains(&words) && text_is_all_lowercase_alpha_fragment(&line.text) {
        return true;
    }
    false
}

fn centered_prose_continuation_should_not_be_heading(line: &LayoutLine) -> bool {
    if line.page_index < 2 || looks_like_clear_section_heading(&line.text) {
        return false;
    }
    let words = model_word_count(&line.text);
    if words < 6 || !line.centered || line_ends_with_period(&line.text) {
        return false;
    }
    let width_ratio = (line.right - line.left) / line.page_width.max(1.0);
    if width_ratio < 0.40 || uppercase_ratio(&line.text) > 0.40 {
        return false;
    }
    let trimmed = line.text.trim();
    let digit_count = trimmed.chars().filter(|ch| ch.is_ascii_digit()).count();
    let title_ratio = title_case_ratio(trimmed);
    if title_ratio >= 0.65 {
        return false;
    }
    let prose_punctuation = trimmed.contains(',') || trimmed.contains(';');
    let continuation_shape =
        starts_with_lowercase_letter(trimmed) || trimmed.ends_with('-') || digit_count >= 1;
    prose_punctuation || continuation_shape
}

fn sentence_case_body_continuation_should_not_be_heading(line: &LayoutLine) -> bool {
    if line.page_index < 2 || looks_like_clear_section_heading(&line.text) {
        return false;
    }
    let words = model_word_count(&line.text);
    if !(6..=18).contains(&words) || line_ends_with_period(&line.text) {
        return false;
    }
    if !(line.centered || line.heading_geometry_like) {
        return false;
    }
    let trimmed = line.text.trim();
    if uppercase_ratio(trimmed) > 0.40 || title_case_ratio(trimmed) >= 0.55 {
        return false;
    }
    let digit_count = trimmed.chars().filter(|ch| ch.is_ascii_digit()).count();
    let prose_punctuation = trimmed.contains(',') || trimmed.contains(';');
    let continuation_shape =
        starts_with_lowercase_letter(trimmed) || trimmed.ends_with('-') || digit_count >= 1;
    prose_punctuation || continuation_shape
}

fn early_article_display_title_fragment_should_not_be_heading(line: &LayoutLine) -> bool {
    if line.page_index > 1 || looks_like_clear_section_heading(&line.text) {
        return false;
    }
    let words = model_word_count(&line.text);
    if !(2..=24).contains(&words) || line.line_index > 12 || line.y0_ratio() > 0.42 {
        return false;
    }
    let width_ratio = (line.right - line.left) / line.page_width.max(1.0);
    let display_like = line.centered
        || line.bold
        || line.heading_geometry_like
        || line.font_ratio_body >= 1.10
        || line.font_ratio_page_ref >= 1.08;
    display_like
        && width_ratio <= 0.78
        && !line_ends_with_period(&line.text)
        && !starts_with_note_marker(&line.text)
}

fn looks_like_numeric_table_cell_fragment(line: &LayoutLine) -> bool {
    let trimmed = line.text.trim();
    let digit_count = trimmed.chars().filter(|ch| ch.is_ascii_digit()).count();
    digit_count >= 3
        && digit_count <= 5
        && trimmed
            .chars()
            .all(|ch| ch.is_ascii_digit() || matches!(ch, ',' | '.' | ' ' | '\u{00a0}'))
        && (line.narrow_measure_like || line.centered || line.width_to_body_ratio <= 0.55)
}

fn text_is_all_lowercase_alpha_fragment(text: &str) -> bool {
    let mut saw_alpha = false;
    for ch in text.chars() {
        if ch.is_ascii_alphabetic() {
            saw_alpha = true;
            if !ch.is_ascii_lowercase() {
                return false;
            }
        }
    }
    saw_alpha
}

fn push_unique_hint(hints: &mut Vec<LiquidLayoutHint>, text: &str, role: LiquidBlockRole) {
    let key = normalize_model_text(text);
    if key.is_empty() {
        return;
    }
    if let Some(existing) = hints
        .iter_mut()
        .find(|hint| normalize_model_text(&hint.text) == key)
    {
        if hint_priority(role) > hint_priority(existing.role) {
            existing.role = role;
            existing.text = text.to_owned();
        }
        return;
    }
    hints.push(LiquidLayoutHint {
        text: text.to_owned(),
        role,
    });
}

fn hint_role_for_line(hints: &[LiquidLayoutHint], line: &LayoutLine) -> Option<LiquidBlockRole> {
    let key = normalize_model_text(&line.text);
    if key.is_empty() {
        return None;
    }
    hints
        .iter()
        .find(|hint| normalize_model_text(&hint.text) == key)
        .map(|hint| hint.role)
}

fn hint_priority(role: LiquidBlockRole) -> u8 {
    match role {
        LiquidBlockRole::Marginalia => 100,
        LiquidBlockRole::Noise => 90,
        LiquidBlockRole::Contents => 80,
        LiquidBlockRole::Header | LiquidBlockRole::Footer => 70,
        LiquidBlockRole::Caption | LiquidBlockRole::Table | LiquidBlockRole::ListItem => 50,
        LiquidBlockRole::Heading | LiquidBlockRole::Subheading => 45,
        LiquidBlockRole::Metadata => 40,
        LiquidBlockRole::Paragraph | LiquidBlockRole::Lead => 20,
        _ => 10,
    }
}

#[derive(Debug, Deserialize)]
struct LayoutRoleModel {
    roles: Vec<String>,
    feature_dim: usize,
    log_priors: Vec<f64>,
    log_likelihood: Vec<Vec<f64>>,
}

impl LayoutRoleModel {
    fn predict(&self, line: &LayoutLine) -> Option<&str> {
        self.predict_from_tokens(feature_tokens(line))
    }

    fn predict_footnote_specialist(&self, line: &LayoutLine) -> Option<&str> {
        self.predict_from_tokens_with_bias(
            footnote_specialist_feature_tokens(line),
            &[("footnote", FOOTNOTE_SPECIALIST_RUNTIME_BIAS)],
        )
    }

    fn predict_header_footer_specialist(&self, line: &LayoutLine) -> Option<&str> {
        self.predict_from_tokens(header_footer_specialist_feature_tokens(line))
    }

    fn predict_from_tokens(&self, tokens: Vec<String>) -> Option<&str> {
        self.score_from_tokens_with_bias(tokens, &[])
            .map(|(role, _)| role)
    }

    fn predict_from_tokens_with_bias(
        &self,
        tokens: Vec<String>,
        role_biases: &[(&str, f64)],
    ) -> Option<&str> {
        self.score_from_tokens_with_bias(tokens, role_biases)
            .map(|(role, _)| role)
    }

    fn score(&self, line: &LayoutLine) -> Option<(&str, f64)> {
        self.score_from_tokens_with_bias(feature_tokens(line), &[])
    }

    fn score_from_tokens_with_bias(
        &self,
        tokens: Vec<String>,
        role_biases: &[(&str, f64)],
    ) -> Option<(&str, f64)> {
        if self.roles.is_empty()
            || self.feature_dim == 0
            || self.log_priors.len() != self.roles.len()
            || self.log_likelihood.len() != self.roles.len()
        {
            return None;
        }

        let mut scores = self.log_priors.clone();
        for (role, bias) in role_biases {
            if let Some(index) = self.roles.iter().position(|candidate| candidate == role)
                && let Some(score) = scores.get_mut(index)
            {
                *score += *bias;
            }
        }
        for token in tokens {
            let index = (stable_hash(&token) as usize) % self.feature_dim;
            for (role_index, score) in scores.iter_mut().enumerate() {
                let value = self
                    .log_likelihood
                    .get(role_index)
                    .and_then(|row| row.get(index))
                    .copied()?;
                *score += value;
            }
        }

        let best = scores
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.total_cmp(b))
            .map(|(index, _)| index)?;
        let best_score = *scores.get(best)?;
        let runner_up = scores
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != best)
            .map(|(_, score)| *score)
            .max_by(f64::total_cmp)
            .unwrap_or(best_score);
        self.roles
            .get(best)
            .map(|role| (role.as_str(), best_score - runner_up))
    }
}

fn heading_specialist_stack_tokens(line: &LayoutLine) -> Vec<String> {
    let mut tokens = Vec::new();
    for (name, model) in [
        ("main", layout_role_model()),
        ("liquid", liquid_core_role_model()),
        ("doclaynet_main", doclaynet_main_role_model()),
        ("doclaynet_liquid", doclaynet_liquid_core_role_model()),
        ("body", body_legacy_role_model()),
        ("body_chandra", body_role_model()),
        ("heading_chandra", heading_prior_stack_role_model()),
    ] {
        let Some(model) = model else {
            continue;
        };
        let Some((predicted, margin)) = model.score(line) else {
            continue;
        };
        for _ in 0..8 {
            tokens.push(format!("stack={name}:role={predicted}"));
        }
        let margin_bucket = bucket(
            margin as f32,
            &[
                (2.0, "2_0"),
                (5.0, "5_0"),
                (10.0, "10_0"),
                (20.0, "20_0"),
                (40.0, "40_0"),
            ],
        );
        tokens.push(format!("stack={name}:margin={margin_bucket}",));
        tokens.push(format!(
            "stack={name}:role_margin={predicted}:{margin_bucket}"
        ));
        if margin >= 20.0 {
            tokens.push(format!("stack={name}:strong_role={predicted}"));
        }
        if line.heading_geometry_like {
            tokens.push(format!("stack={name}:role_heading_geometry={predicted}"));
            if margin >= 20.0 {
                tokens.push(format!(
                    "stack={name}:strong_role_heading_geometry={predicted}"
                ));
            }
        }
        if line.centered {
            tokens.push(format!("stack={name}:role_centered={predicted}"));
        }
        if name.starts_with("body") && predicted == "body" && margin >= 20.0 {
            tokens.push("stack=body_family:strong_body".to_owned());
            if line.heading_geometry_like {
                tokens.push("stack=body_family:strong_body_heading_geometry_conflict".to_owned());
            }
            if line.centered {
                tokens.push("stack=body_family:strong_body_centered_conflict".to_owned());
            }
        }
    }
    tokens
}

fn layout_role_model() -> Option<&'static LayoutRoleModel> {
    static MODEL: OnceLock<Option<LayoutRoleModel>> = OnceLock::new();
    MODEL
        .get_or_init(|| {
            serde_json::from_str(include_str!(
                "../profile-models/layout-role-v45-pp-grok-scaleup-0507-candidate/layout-role-model.json"
            ))
            .ok()
        })
        .as_ref()
}

fn footnote_role_model() -> Option<&'static LayoutRoleModel> {
    static MODEL: OnceLock<Option<LayoutRoleModel>> = OnceLock::new();
    MODEL
        .get_or_init(|| {
            serde_json::from_str(include_str!(
                "../profile-models/layout-footnote-v68-chandra-filtered3-w035-20260603-candidate/layout-role-model.json"
            ))
            .ok()
        })
        .as_ref()
}

fn body_role_model() -> Option<&'static LayoutRoleModel> {
    static MODEL: OnceLock<Option<LayoutRoleModel>> = OnceLock::new();
    MODEL
        .get_or_init(|| {
            serde_json::from_str(include_str!(
                "../profile-models/layout-body-chandra-structure-disputes-20260604-interim-104950-cycle064-candidate/layout-role-model.json"
            ))
            .ok()
        })
        .as_ref()
}

fn body_legacy_role_model() -> Option<&'static LayoutRoleModel> {
    static MODEL: OnceLock<Option<LayoutRoleModel>> = OnceLock::new();
    MODEL
        .get_or_init(|| {
            serde_json::from_str(include_str!(
                "../profile-models/layout-body-v5-unseen-highconf-calibrated-20260602-candidate/layout-role-model.json"
            ))
            .ok()
        })
        .as_ref()
}

fn heading_role_model() -> Option<&'static LayoutRoleModel> {
    static MODEL: OnceLock<Option<LayoutRoleModel>> = OnceLock::new();
    MODEL
        .get_or_init(|| {
            serde_json::from_str(include_str!(
                "../profile-models/layout-heading-cycle058-faststack-expanded-goldnoise-20260605-candidate/layout-role-model.json"
            ))
            .ok()
        })
        .as_ref()
}

fn heading_prior_stack_role_model() -> Option<&'static LayoutRoleModel> {
    static MODEL: OnceLock<Option<LayoutRoleModel>> = OnceLock::new();
    MODEL
        .get_or_init(|| {
            serde_json::from_str(include_str!(
                "../profile-models/layout-heading-chandra-feature-hardneg-backfill-allaccum-20260605-night-0001-000039-cycle263-seed-candidate/layout-role-model.json"
            ))
            .ok()
        })
        .as_ref()
}

fn header_footer_role_model() -> Option<&'static LayoutRoleModel> {
    static MODEL: OnceLock<Option<LayoutRoleModel>> = OnceLock::new();
    MODEL
        .get_or_init(|| {
            serde_json::from_str(include_str!(
                "../profile-models/layout-header-footer-v1-grok-lawreview-bias-neg20/layout-role-model.json"
            ))
            .ok()
        })
        .as_ref()
}

fn doclaynet_main_role_model() -> Option<&'static LayoutRoleModel> {
    static MODEL: OnceLock<Option<LayoutRoleModel>> = OnceLock::new();
    MODEL
        .get_or_init(|| {
            serde_json::from_str(include_str!(
                "../profile-models/layout-role-v64-doclaynet-stream-balanced-20260603-candidate/layout-role-model.json"
            ))
            .ok()
        })
        .as_ref()
}

fn liquid_core_role_model() -> Option<&'static LayoutRoleModel> {
    static MODEL: OnceLock<Option<LayoutRoleModel>> = OnceLock::new();
    MODEL
        .get_or_init(|| {
            serde_json::from_str(include_str!(
                "../profile-models/layout-role-liquid-core-v34-block-geometry-20260601-candidate/layout-role-model.json"
            ))
            .ok()
        })
        .as_ref()
}

fn doclaynet_liquid_core_role_model() -> Option<&'static LayoutRoleModel> {
    static MODEL: OnceLock<Option<LayoutRoleModel>> = OnceLock::new();
    MODEL
        .get_or_init(|| {
            serde_json::from_str(include_str!(
                "../profile-models/layout-role-liquid-core-v35-doclaynet-stream-balanced-20260603-candidate/layout-role-model.json"
            ))
            .ok()
        })
        .as_ref()
}

fn feature_tokens(line: &LayoutLine) -> Vec<String> {
    let mut tokens = Vec::new();
    let words = model_words(&line.text);
    let word_count = model_word_count(&line.text);
    let ends_with_period = line_ends_with_period(&line.text);
    let terminal_punctuation = terminal_punctuation_name(&line.text);
    let contains_l_rev = contains_l_rev_cue(&line.text);
    for token in words.iter().take(8) {
        tokens.push(format!("w={token}"));
    }
    add_lexical_ngram_tokens(&line.text, &mut tokens);
    if let Some(first) = words.first() {
        tokens.push(format!("first={first}"));
    }
    if let Some(last) = words.last() {
        tokens.push(format!("last={last}"));
    }

    push_weighted(
        &mut tokens,
        format!(
            "page={}",
            bucket(
                line.page_index as f32,
                &[(0.0, "0"), (1.0, "1"), (2.0, "2"), (4.0, "4"), (9.0, "9")]
            )
        ),
        2,
    );
    push_weighted(
        &mut tokens,
        format!("page_exact={}", line.page_index.min(20)),
        1,
    );
    if line.page_index <= 1 {
        push_weighted(&mut tokens, "early_article_page".to_owned(), 3);
    }
    if line.page_index >= 2 {
        push_weighted(&mut tokens, "post_front_matter_page".to_owned(), 2);
    }
    push_weighted(
        &mut tokens,
        format!(
            "y0={}",
            bucket(
                line.y0_ratio(),
                &[
                    (0.06, "0_06"),
                    (0.12, "0_12"),
                    (0.25, "0_25"),
                    (0.45, "0_45"),
                    (0.65, "0_65"),
                    (0.78, "0_78"),
                    (0.9, "0_9"),
                ],
            )
        ),
        4,
    );
    push_weighted(
        &mut tokens,
        format!(
            "x0={}",
            bucket(
                line.left / line.page_width.max(1.0),
                &[(0.08, "0_08"), (0.15, "0_15"), (0.25, "0_25"), (0.4, "0_4")]
            )
        ),
        2,
    );
    push_weighted(
        &mut tokens,
        format!(
            "width={}",
            bucket(
                (line.right - line.left) / line.page_width.max(1.0),
                &[
                    (0.12, "0_12"),
                    (0.25, "0_25"),
                    (0.45, "0_45"),
                    (0.7, "0_7"),
                    (0.9, "0_9"),
                ],
            )
        ),
        2,
    );
    push_weighted(
        &mut tokens,
        format!(
            "fs_page={}",
            bucket(
                line.font_ratio_page,
                &[
                    (0.75, "0_75"),
                    (0.9, "0_9"),
                    (1.0, "1_0"),
                    (1.1, "1_1"),
                    (1.25, "1_25"),
                    (1.5, "1_5"),
                ],
            )
        ),
        4,
    );
    push_weighted(
        &mut tokens,
        format!(
            "fs_page_ref={}",
            bucket(
                line.font_ratio_page_ref,
                &[
                    (0.7, "0_7"),
                    (0.82, "0_82"),
                    (0.92, "0_92"),
                    (1.0, "1_0"),
                    (1.1, "1_1"),
                ],
            )
        ),
        5,
    );
    push_weighted(
        &mut tokens,
        format!(
            "fs_doc={}",
            bucket(
                line.font_ratio_doc,
                &[
                    (0.75, "0_75"),
                    (0.9, "0_9"),
                    (1.0, "1_0"),
                    (1.1, "1_1"),
                    (1.25, "1_25"),
                    (1.5, "1_5"),
                ],
            )
        ),
        3,
    );
    push_weighted(
        &mut tokens,
        format!(
            "wc={}",
            bucket(
                word_count as f32,
                &[
                    (1.0, "1"),
                    (2.0, "2"),
                    (3.0, "3"),
                    (5.0, "5"),
                    (8.0, "8"),
                    (12.0, "12"),
                    (18.0, "18"),
                    (28.0, "28"),
                    (40.0, "40")
                ]
            )
        ),
        3,
    );
    push_weighted(
        &mut tokens,
        format!(
            "line_word_count_bucket={}",
            bucket(
                word_count as f32,
                &[
                    (1.0, "1"),
                    (2.0, "2"),
                    (3.0, "3"),
                    (5.0, "5"),
                    (8.0, "8"),
                    (12.0, "12"),
                    (18.0, "18"),
                    (28.0, "28"),
                    (40.0, "40")
                ]
            )
        ),
        3,
    );
    push_weighted(&mut tokens, format!("wc_exact={}", word_count.min(40)), 1);
    push_weighted(
        &mut tokens,
        format!("line_word_count_exact={}", word_count.min(40)),
        1,
    );
    push_weighted(
        &mut tokens,
        format!(
            "lex_wc={}",
            bucket(
                words.len() as f32,
                &[
                    (1.0, "1"),
                    (3.0, "3"),
                    (8.0, "8"),
                    (16.0, "16"),
                    (30.0, "30")
                ]
            )
        ),
        1,
    );
    push_weighted(
        &mut tokens,
        format!("terminal_punct={terminal_punctuation}"),
        2,
    );
    if word_count <= 18 {
        push_weighted(
            &mut tokens,
            format!("short_terminal_punct={terminal_punctuation}"),
            2,
        );
    }
    if ends_with_period {
        push_weighted(&mut tokens, "ends_with_period".to_owned(), 2);
        push_weighted(&mut tokens, "line_ends_with_period".to_owned(), 2);
    } else {
        push_weighted(&mut tokens, "does_not_end_with_period".to_owned(), 1);
        push_weighted(&mut tokens, "line_does_not_end_with_period".to_owned(), 1);
    }
    if contains_l_rev {
        push_weighted(&mut tokens, "contains_l_rev".to_owned(), 8);
        push_weighted(&mut tokens, "contains_l_rev_citation".to_owned(), 8);
    }
    if word_count <= 5 {
        push_weighted(&mut tokens, "very_short_line_words".to_owned(), 2);
    }
    if word_count <= 12 {
        push_weighted(&mut tokens, "short_line_words".to_owned(), 1);
        if ends_with_period {
            push_weighted(&mut tokens, "short_line_period".to_owned(), 2);
        } else {
            push_weighted(&mut tokens, "short_line_no_period".to_owned(), 2);
        }
    }
    if word_count >= 24 {
        push_weighted(&mut tokens, "long_line_words".to_owned(), 4);
    }
    if word_count >= 12 && ends_with_period {
        push_weighted(&mut tokens, "sentence_length_period_line".to_owned(), 4);
    }
    if looks_like_numeric_table_cell_fragment(line) {
        push_weighted(&mut tokens, "numeric_table_cell_fragment".to_owned(), 8);
        push_weighted(&mut tokens, "numeric_fragment_not_heading".to_owned(), 8);
    }
    if (1..=2).contains(&word_count)
        && text_is_all_lowercase_alpha_fragment(&line.text)
        && !looks_like_clear_section_heading(&line.text)
    {
        push_weighted(&mut tokens, "lowercase_body_fragment".to_owned(), 8);
        if line.centered || line.heading_geometry_like {
            push_weighted(
                &mut tokens,
                "lowercase_fragment_heading_shape_conflict".to_owned(),
                8,
            );
        }
    }
    let upper_ratio = uppercase_ratio(&line.text);
    push_weighted(
        &mut tokens,
        format!(
            "upper={}",
            bucket(
                upper_ratio,
                &[(0.1, "0_1"), (0.35, "0_35"), (0.65, "0_65"), (0.9, "0_9")]
            )
        ),
        2,
    );
    if upper_ratio >= 0.82 {
        push_weighted(&mut tokens, "all_caps_line".to_owned(), 5);
        if word_count <= 12 {
            push_weighted(&mut tokens, "short_all_caps_line".to_owned(), 5);
        }
        if !ends_with_period {
            push_weighted(&mut tokens, "all_caps_no_period".to_owned(), 4);
        }
    } else if upper_ratio >= 0.55 {
        push_weighted(&mut tokens, "mostly_caps_line".to_owned(), 3);
        if word_count <= 12 && !ends_with_period {
            push_weighted(&mut tokens, "short_mostly_caps_no_period".to_owned(), 3);
        }
    }
    tokens.push(format!(
        "zone={}",
        bucket(
            line.y0_ratio(),
            &[
                (0.08, "0_08"),
                (0.18, "0_18"),
                (0.5, "0_5"),
                (0.68, "0_68"),
                (0.82, "0_82")
            ]
        )
    ));

    push_weighted(
        &mut tokens,
        format!(
            "line_index={}",
            bucket(
                line.line_index as f32,
                &[
                    (0.0, "0"),
                    (1.0, "1"),
                    (2.0, "2"),
                    (3.0, "3"),
                    (5.0, "5"),
                    (10.0, "10"),
                    (20.0, "20")
                ]
            )
        ),
        3,
    );
    let width_ratio = (line.right - line.left) / line.page_width.max(1.0);
    push_weighted(
        &mut tokens,
        format!(
            "body_left_delta={}",
            bucket(
                line.body_left_delta_ratio,
                &[
                    (0.02, "0_02"),
                    (0.05, "0_05"),
                    (0.10, "0_10"),
                    (0.18, "0_18"),
                    (0.30, "0_30"),
                ],
            )
        ),
        3,
    );
    push_weighted(
        &mut tokens,
        format!(
            "signed_body_indent={}",
            bucket(
                line.signed_body_left_delta_ratio,
                &[
                    (-0.18, "-0_18"),
                    (-0.08, "-0_08"),
                    (-0.025, "-0_025"),
                    (0.025, "0_025"),
                    (0.08, "0_08"),
                    (0.18, "0_18"),
                ],
            )
        ),
        4,
    );
    push_weighted(
        &mut tokens,
        format!(
            "width_to_body={}",
            bucket(
                line.width_to_body_ratio,
                &[
                    (0.45, "0_45"),
                    (0.72, "0_72"),
                    (0.95, "0_95"),
                    (1.20, "1_20"),
                    (1.60, "1_60"),
                ],
            )
        ),
        3,
    );
    push_weighted(
        &mut tokens,
        format!(
            "prev_gap_body={}",
            bucket(
                line.prev_gap_to_median_ratio,
                &[
                    (0.8, "0_8"),
                    (1.2, "1_2"),
                    (1.8, "1_8"),
                    (2.8, "2_8"),
                    (4.5, "4_5"),
                ],
            )
        ),
        2,
    );
    push_weighted(
        &mut tokens,
        format!(
            "next_gap_body={}",
            bucket(
                line.next_gap_to_median_ratio,
                &[
                    (0.8, "0_8"),
                    (1.2, "1_2"),
                    (1.8, "1_8"),
                    (2.8, "2_8"),
                    (4.5, "4_5"),
                ],
            )
        ),
        2,
    );
    push_weighted(
        &mut tokens,
        format!(
            "right_indent={}",
            bucket(
                line.right_indent_ratio,
                &[
                    (0.08, "0_08"),
                    (0.16, "0_16"),
                    (0.28, "0_28"),
                    (0.42, "0_42"),
                    (0.60, "0_60"),
                ],
            )
        ),
        2,
    );
    push_weighted(
        &mut tokens,
        format!(
            "center_offset={}",
            bucket(
                line.center_offset_ratio,
                &[
                    (0.025, "0_025"),
                    (0.06, "0_06"),
                    (0.12, "0_12"),
                    (0.22, "0_22")
                ],
            )
        ),
        3,
    );
    push_weighted(
        &mut tokens,
        format!(
            "fs_body={}",
            bucket(
                line.font_ratio_body,
                &[
                    (0.78, "0_78"),
                    (0.92, "0_92"),
                    (1.0, "1_0"),
                    (1.06, "1_06"),
                    (1.14, "1_14"),
                    (1.28, "1_28"),
                    (1.55, "1_55"),
                ],
            )
        ),
        5,
    );
    push_weighted(
        &mut tokens,
        format!(
            "space_run={}",
            bucket(
                line.max_internal_space_run as f32,
                &[(1.0, "1"), (2.0, "2"), (4.0, "4"), (8.0, "8"), (16.0, "16")],
            )
        ),
        2,
    );
    push_weighted(
        &mut tokens,
        format!(
            "space_density={}",
            bucket(
                line.space_density,
                &[
                    (0.08, "0_08"),
                    (0.16, "0_16"),
                    (0.28, "0_28"),
                    (0.42, "0_42")
                ],
            )
        ),
        2,
    );
    push_weighted(
        &mut tokens,
        format!(
            "leading_spaces={}",
            bucket(
                line.leading_space_count as f32,
                &[
                    (0.0, "0"),
                    (1.0, "1"),
                    (2.0, "2"),
                    (4.0, "4"),
                    (8.0, "8"),
                    (16.0, "16")
                ],
            )
        ),
        1,
    );
    push_weighted(
        &mut tokens,
        format!(
            "trailing_spaces={}",
            bucket(
                line.trailing_space_count as f32,
                &[
                    (0.0, "0"),
                    (1.0, "1"),
                    (2.0, "2"),
                    (4.0, "4"),
                    (8.0, "8"),
                    (16.0, "16")
                ],
            )
        ),
        1,
    );
    if line.leading_space_count >= 2 {
        push_weighted(&mut tokens, "raw_text_leading_spaces".to_owned(), 2);
    }
    if line.trailing_space_count >= 2 {
        push_weighted(&mut tokens, "raw_text_trailing_spaces".to_owned(), 1);
    }
    if line.font_ratio_body >= 1.06 {
        push_weighted(&mut tokens, "larger_than_body_font".to_owned(), 5);
        if word_count <= 18 {
            push_weighted(&mut tokens, "larger_than_body_short_line".to_owned(), 5);
        }
        if word_count <= 12 && !ends_with_period {
            push_weighted(
                &mut tokens,
                "larger_than_body_short_no_period".to_owned(),
                5,
            );
        }
    }
    if line.font_ratio_body >= 1.14 {
        push_weighted(&mut tokens, "much_larger_than_body_font".to_owned(), 4);
        if word_count <= 12 && !ends_with_period {
            push_weighted(&mut tokens, "much_larger_short_no_period".to_owned(), 5);
        }
    }
    if line.vertically_isolated_like {
        push_weighted(&mut tokens, "vertically_isolated_line".to_owned(), 5);
        if word_count <= 18 {
            push_weighted(&mut tokens, "isolated_short_line".to_owned(), 5);
        }
        if word_count <= 12 && !ends_with_period {
            push_weighted(&mut tokens, "isolated_short_no_period".to_owned(), 5);
        }
    }
    if word_count <= 12
        && !ends_with_period
        && matches!(terminal_punctuation, "none" | "colon" | "close_paren")
        && (line.heading_geometry_like || line.centered || line.bold)
        && line.font_ratio_body >= 1.02
        && line.width_to_body_ratio <= 1.12
        && !line.body_column_like
        && !line.below_footnote_divider
        && !line.sequence_footnote_zone
    {
        push_weighted(&mut tokens, "heading_shape_short_nonperiod".to_owned(), 8);
    }
    if word_count <= 8
        && !ends_with_period
        && (line.centered || line.bold)
        && line.font_ratio_body >= 1.08
        && line.right_indent_ratio >= 0.16
    {
        push_weighted(
            &mut tokens,
            "heading_shape_short_indented_right_edge".to_owned(),
            6,
        );
    }
    if word_count <= 12
        && !ends_with_period
        && !line.heading_geometry_like
        && !line.vertically_isolated_like
        && !line.centered
        && line.body_column_like
        && line.font_ratio_body < 1.06
    {
        push_weighted(&mut tokens, "short_no_period_in_body_flow".to_owned(), 8);
    }
    if word_count <= 8
        && !ends_with_period
        && line.body_column_like
        && line.font_ratio_body <= 1.04
        && line.prev_gap_to_median_ratio <= 1.25
        && line.next_gap_to_median_ratio <= 1.45
    {
        push_weighted(
            &mut tokens,
            "short_no_period_continuous_body_geometry".to_owned(),
            8,
        );
    }
    if line.heading_geometry_like {
        push_weighted(&mut tokens, "heading_geometry_like".to_owned(), 10);
    }
    if line.heading_geometry_like
        && (title_case_ratio(&line.text) >= 0.65 || uppercase_ratio(&line.text) >= 0.35)
    {
        push_weighted(&mut tokens, "heading_geometry_case_match".to_owned(), 8);
    }
    if line.heading_geometry_like && line.centered {
        push_weighted(&mut tokens, "heading_geometry_centered".to_owned(), 6);
    }
    if line.heading_geometry_like && line.bold {
        push_weighted(&mut tokens, "heading_geometry_bold".to_owned(), 6);
    }
    if line.max_internal_space_run >= 4 {
        push_weighted(&mut tokens, "wide_internal_space_run".to_owned(), 4);
    }
    if line.max_internal_space_run >= 8 {
        push_weighted(&mut tokens, "table_or_toc_space_run".to_owned(), 5);
    }
    if line.body_column_like {
        push_weighted(&mut tokens, "body_column_like".to_owned(), 5);
        if line.font_ratio_page_ref >= 0.92 {
            push_weighted(&mut tokens, "body_column_normal_font".to_owned(), 5);
        }
        if word_count >= 10 && ends_with_period && line.font_ratio_body <= 1.08 {
            push_weighted(&mut tokens, "body_column_sentence_period".to_owned(), 7);
        }
        if word_count >= 16 && line.font_ratio_body <= 1.08 {
            push_weighted(&mut tokens, "body_column_long_line".to_owned(), 6);
        }
    }
    if line.width_to_body_ratio >= 0.82
        && (0.94..=1.08).contains(&line.font_ratio_body)
        && word_count >= 8
    {
        push_weighted(&mut tokens, "body_measure_normal_words".to_owned(), 5);
    }
    if line.width_to_body_ratio >= 0.82
        && (0.94..=1.08).contains(&line.font_ratio_body)
        && word_count >= 10
        && ends_with_period
    {
        push_weighted(&mut tokens, "body_measure_sentence_period".to_owned(), 7);
    }
    if line.narrow_measure_like {
        push_weighted(&mut tokens, "narrow_measure_like".to_owned(), 4);
    }
    if line.hanging_indent_like {
        push_weighted(&mut tokens, "hanging_indent_like".to_owned(), 6);
    }
    if line.follows_hanging_note_marker {
        push_weighted(&mut tokens, "follows_hanging_note_marker".to_owned(), 8);
    }
    if line.page_index <= 1 && line.y0_ratio() <= 0.22 {
        push_weighted(&mut tokens, "front_matter_top_band".to_owned(), 3);
    }
    if line.page_index >= 2 {
        push_weighted(&mut tokens, "after_front_matter".to_owned(), 2);
    }
    if line.page_index == 0
        && line.line_index <= 12
        && line.y0_ratio() <= 0.42
        && line.centered
        && (2..=24).contains(&word_count)
        && !ends_with_period
        && line.font_ratio_doc >= 1.05
    {
        push_weighted(
            &mut tokens,
            "first_page_centered_display_context".to_owned(),
            8,
        );
        if line.heading_geometry_like {
            push_weighted(
                &mut tokens,
                "first_page_display_heading_geometry_context".to_owned(),
                8,
            );
        }
        if word_count <= 10 && width_ratio <= 0.55 {
            push_weighted(
                &mut tokens,
                "first_page_title_fragment_context".to_owned(),
                7,
            );
        }
    }
    if line.page_index <= 1
        && line.line_index <= 12
        && line.y0_ratio() <= 0.42
        && (2..=24).contains(&word_count)
        && !ends_with_period
        && (line.centered
            || line.bold
            || line.heading_geometry_like
            || line.font_ratio_body >= 1.10
            || line.font_ratio_page_ref >= 1.08)
    {
        push_weighted(&mut tokens, "early_article_display_context".to_owned(), 8);
        if width_ratio <= 0.78 {
            push_weighted(
                &mut tokens,
                "early_article_display_metadata_context".to_owned(),
                8,
            );
        }
        if word_count <= 10 && width_ratio <= 0.55 {
            push_weighted(
                &mut tokens,
                "early_article_title_fragment_context".to_owned(),
                8,
            );
        }
    }
    if line.page_index == 0
        && line.line_index <= 6
        && line.y0_ratio() <= 0.28
        && line.centered
        && line.font_ratio_doc >= 1.05
    {
        push_weighted(&mut tokens, "first_page_title_band".to_owned(), 6);
    }
    if line.page_index >= 1 && line.repeated_header_footer {
        push_weighted(&mut tokens, "later_repeated_edge_text".to_owned(), 6);
    }
    if first_page_repeated_line_can_be_title(line, words.len(), line.y0_ratio()) {
        push_weighted(
            &mut tokens,
            "first_page_repeated_title_candidate".to_owned(),
            4,
        );
    }
    if looks_like_probable_author_line(line) {
        push_weighted(&mut tokens, "probable_author_line".to_owned(), 8);
        push_weighted(
            &mut tokens,
            "early_article_probable_author_line".to_owned(),
            8,
        );
    }
    if line.page_index >= 3
        && line.centered
        && width_ratio >= 0.45
        && line.y0_ratio() <= 0.32
        && words.len() >= 6
    {
        push_weighted(&mut tokens, "late_centered_prose_band".to_owned(), 6);
    }
    if line.page_index >= 3
        && line.centered
        && width_ratio >= 0.45
        && uppercase_ratio(&line.text) <= 0.35
        && words.len() >= 8
    {
        push_weighted(&mut tokens, "late_centered_sentence".to_owned(), 6);
    }
    if centered_prose_continuation_should_not_be_heading(line) {
        push_weighted(&mut tokens, "centered_prose_continuation".to_owned(), 8);
        push_weighted(
            &mut tokens,
            "centered_body_clause_not_heading".to_owned(),
            8,
        );
    }
    if sentence_case_body_continuation_should_not_be_heading(line) {
        push_weighted(&mut tokens, "sentence_case_body_continuation".to_owned(), 8);
        push_weighted(
            &mut tokens,
            "heading_shape_sentence_fragment_conflict".to_owned(),
            8,
        );
    }
    if line.bold {
        push_weighted(&mut tokens, "is_bold".to_owned(), 3);
    }
    if line.italic {
        push_weighted(&mut tokens, "is_italic".to_owned(), 2);
    }
    if line.centered {
        push_weighted(&mut tokens, "is_centered".to_owned(), 3);
    }
    if line.below_footnote_divider {
        push_weighted(&mut tokens, "below_footnote_divider".to_owned(), 8);
        push_weighted(
            &mut tokens,
            format!(
                "dist_divider={}",
                bucket(
                    line.distance_below_divider,
                    &[(0.02, "0_02"), (0.05, "0_05"), (0.1, "0_1"), (0.2, "0_2")]
                )
            ),
            3,
        );
    }
    if line.page_has_footnote_divider {
        push_weighted(&mut tokens, "page_has_footnote_divider".to_owned(), 2);
    }
    if line.sequence_footnote_zone {
        push_weighted(&mut tokens, "sequence_footnote_zone".to_owned(), 8);
    }
    add_previous_line_context_tokens(line, &mut tokens);
    add_footnote_geometry_interaction_tokens(line, &mut tokens);
    if line.repeated_header_footer {
        push_weighted(&mut tokens, "repeated_edge_text".to_owned(), 6);
    }
    if starts_with_note_marker(&line.text) {
        push_weighted(&mut tokens, "note_marker".to_owned(), 5);
    }
    if is_bare_numeric_note_marker(&line.text) {
        push_weighted(&mut tokens, "bare_numeric_note_marker".to_owned(), 5);
    }
    if starts_with_compact_legal_note_marker(&line.text) {
        push_weighted(&mut tokens, "compact_legal_note_marker".to_owned(), 8);
    }
    if starts_with_legal_note_marker(&line.text) {
        push_weighted(&mut tokens, "legal_note_marker".to_owned(), 8);
    }
    if looks_like_general_citation_note_start(&line.text) {
        push_weighted(&mut tokens, "general_citation_note_start".to_owned(), 8);
    }
    if looks_like_citation_continuation_text(&line.text) {
        push_weighted(&mut tokens, "citation_continuation_text".to_owned(), 4);
    }
    if contains_short_form_citation_cue(&line.text) {
        push_weighted(&mut tokens, "short_form_citation_cue".to_owned(), 8);
    }
    if looks_like_publication_citation_continuation_text(&line.text) {
        push_weighted(&mut tokens, "publication_citation_text".to_owned(), 4);
    }
    if looks_like_footnote_bibliographic_lead_text(&line.text) {
        push_weighted(&mut tokens, "bibliographic_lead_text".to_owned(), 4);
    }
    if line.page_contents_like {
        push_weighted(&mut tokens, "page_contents_like".to_owned(), 8);
    }
    if line.contents_or_index_entry {
        push_weighted(&mut tokens, "contents_or_index_entry".to_owned(), 8);
    }
    if line.page_contents_like && line.contents_or_index_entry {
        push_weighted(&mut tokens, "page_contents_entry".to_owned(), 10);
    }
    if is_plain_page_number_line(&line.text) {
        push_weighted(&mut tokens, "plain_page_number_line".to_owned(), 6);
        if line.y0_ratio() <= 0.12 || line.y1_ratio() >= 0.88 {
            push_weighted(&mut tokens, "edge_plain_page_number".to_owned(), 8);
        } else {
            push_weighted(&mut tokens, "midpage_plain_number_cell".to_owned(), 3);
        }
    }
    if looks_like_running_law_review_cite_line(&line.text) {
        push_weighted(&mut tokens, "running_law_review_cite_text".to_owned(), 8);
        if line.y0_ratio() <= 0.12 || line.y1_ratio() >= 0.88 {
            push_weighted(&mut tokens, "edge_running_law_review_cite".to_owned(), 6);
        }
    }
    if looks_like_law_review_journal_masthead_line(line) {
        push_weighted(&mut tokens, "law_review_journal_masthead".to_owned(), 10);
        if line.y0_ratio() <= 0.18 {
            push_weighted(
                &mut tokens,
                "edge_law_review_journal_masthead".to_owned(),
                8,
            );
        }
    }
    if looks_like_enum_marker(&line.text) {
        push_weighted(&mut tokens, "enum_marker".to_owned(), 3);
    }
    if is_contents_line(&line.text) {
        push_weighted(&mut tokens, "dot_leader_contents".to_owned(), 6);
    }
    if is_table_line(&line.text) {
        push_weighted(&mut tokens, "tableish".to_owned(), 5);
    }
    if is_caption_line(&line.text) {
        push_weighted(&mut tokens, "captionish".to_owned(), 5);
    }

    tokens
}

fn add_previous_line_context_tokens(line: &LayoutLine, tokens: &mut Vec<String>) {
    if line.prev_line_present {
        push_weighted(tokens, "prev_line_present".to_owned(), 2);
        push_weighted(
            tokens,
            format!(
                "prev_y_gap={}",
                bucket(
                    line.prev_y_gap_ratio,
                    &[
                        (0.006, "0_006"),
                        (0.014, "0_014"),
                        (0.03, "0_03"),
                        (0.06, "0_06")
                    ]
                )
            ),
            3,
        );
        push_weighted(
            tokens,
            format!(
                "prev_left_delta={}",
                bucket(
                    line.prev_left_delta_ratio,
                    &[
                        (0.015, "0_015"),
                        (0.04, "0_04"),
                        (0.09, "0_09"),
                        (0.18, "0_18")
                    ]
                )
            ),
            2,
        );
        push_weighted(
            tokens,
            format!(
                "prev_font_delta={}",
                bucket(
                    line.prev_font_delta_ratio,
                    &[
                        (0.04, "0_04"),
                        (0.10, "0_10"),
                        (0.18, "0_18"),
                        (0.32, "0_32")
                    ]
                )
            ),
            2,
        );
        if line.prev_sequence_footnote_zone {
            push_weighted(tokens, "prev_sequence_footnote_zone".to_owned(), 8);
            if line.font_ratio_page_ref <= 1.02 {
                push_weighted(tokens, "prev_sequence_current_note_font".to_owned(), 6);
            }
        }
        if line.prev_below_footnote_divider {
            push_weighted(tokens, "prev_below_footnote_divider".to_owned(), 5);
        }
        if line.prev_small_font {
            push_weighted(tokens, "prev_small_font".to_owned(), 4);
        }
        if line.prev_note_marker {
            push_weighted(tokens, "prev_note_marker".to_owned(), 5);
        }
        if line.prev_legal_note_cue {
            push_weighted(tokens, "prev_legal_note_cue".to_owned(), 4);
        }
        if (line.prev_sequence_footnote_zone || line.prev_below_footnote_divider)
            && line.prev_y_gap_ratio <= 0.03
            && line.prev_left_delta_ratio <= 0.09
            && line.font_ratio_page_ref <= 1.02
        {
            push_weighted(tokens, "prev_context_footnote_continuation".to_owned(), 8);
        }
    }
    if line.next_line_present {
        push_weighted(tokens, "next_line_present".to_owned(), 1);
        push_weighted(
            tokens,
            format!(
                "next_y_gap={}",
                bucket(
                    line.next_y_gap_ratio,
                    &[
                        (0.006, "0_006"),
                        (0.014, "0_014"),
                        (0.03, "0_03"),
                        (0.06, "0_06")
                    ]
                )
            ),
            2,
        );
    }
    if line.next_sequence_footnote_zone {
        push_weighted(tokens, "next_sequence_footnote_zone".to_owned(), 5);
        if line.font_ratio_page_ref <= 1.02 {
            push_weighted(tokens, "next_sequence_current_note_font".to_owned(), 5);
        }
    }
    if line.next_below_footnote_divider {
        push_weighted(tokens, "next_below_footnote_divider".to_owned(), 4);
    }
    if line.next_note_marker {
        push_weighted(tokens, "next_note_marker".to_owned(), 5);
    }
    if line.next_legal_note_cue {
        push_weighted(tokens, "next_legal_note_cue".to_owned(), 4);
    }
    if (line.next_sequence_footnote_zone || line.next_below_footnote_divider)
        && line.next_y_gap_ratio <= 0.03
        && line.next_left_delta_ratio <= 0.09
        && line.font_ratio_page_ref <= 1.02
    {
        push_weighted(tokens, "next_context_footnote_continuation".to_owned(), 7);
    }
    if line.sequence_footnote_zone
        && line.font_ratio_page_ref >= 0.95
        && !line.next_note_marker
        && !line.next_legal_note_cue
        && !line.prev_note_marker
        && !line.prev_legal_note_cue
        && !contains_legal_note_cue(&line.text)
        && !looks_like_citation_continuation_text(&line.text)
    {
        push_weighted(
            tokens,
            "sequence_zone_without_local_citation_confirmation".to_owned(),
            6,
        );
    }
}

fn add_footnote_geometry_interaction_tokens(line: &LayoutLine, tokens: &mut Vec<String>) {
    let no_divider = !line.page_has_footnote_divider;
    let small_note_font = line.font_ratio_page_ref <= 0.92;
    let note_marker = starts_with_note_marker(&line.text);
    let compact_note_marker = starts_with_compact_legal_note_marker(&line.text);
    let legal_note_marker = starts_with_legal_note_marker(&line.text);
    let general_citation_note_start = looks_like_general_citation_note_start(&line.text);
    let strong_legal_note_cue = contains_strong_legal_note_cue(&line.text);
    let symbol_note_marker = starts_with_symbol_note_marker(&line.text);
    let bibliographic_lead = looks_like_footnote_bibliographic_lead_text(&line.text);

    if line.below_footnote_divider && line.font_ratio_page_ref <= 1.05 {
        push_weighted(tokens, "geom_below_divider_note_font".to_owned(), 8);
    }
    if no_divider && line.y0_ratio() >= 0.55 && small_note_font && note_marker {
        push_weighted(tokens, "geom_no_divider_note_start".to_owned(), 8);
    }
    if no_divider
        && line.y0_ratio() >= 0.25
        && line.font_ratio_page_ref <= 1.02
        && compact_note_marker
    {
        push_weighted(tokens, "geom_no_divider_compact_note_start".to_owned(), 8);
    }
    if no_divider
        && line.y0_ratio() >= 0.25
        && line.font_ratio_page_ref <= 1.02
        && legal_note_marker
    {
        push_weighted(
            tokens,
            "geom_no_divider_legal_marker_note_start".to_owned(),
            8,
        );
    }
    if no_divider
        && line.y0_ratio() >= 0.25
        && line.font_ratio_page_ref <= 0.95
        && general_citation_note_start
    {
        push_weighted(
            tokens,
            "geom_no_divider_general_citation_note_start".to_owned(),
            8,
        );
    }
    if no_divider
        && line.y0_ratio() >= 0.25
        && line.font_ratio_page_ref <= 1.02
        && general_citation_note_start
    {
        push_weighted(
            tokens,
            "geom_no_divider_general_citation_note_start_relaxed".to_owned(),
            8,
        );
    }
    if no_divider
        && line.y0_ratio() >= 0.22
        && line.y0_ratio() < 0.55
        && line.font_ratio_page_ref <= 0.95
        && general_citation_note_start
    {
        push_weighted(
            tokens,
            "geom_no_divider_general_cite_midpage".to_owned(),
            10,
        );
    }
    if no_divider
        && line.y0_ratio() >= 0.25
        && line.y0_ratio() < 0.55
        && line.font_ratio_page_ref <= 0.98
        && compact_note_marker
    {
        push_weighted(tokens, "geom_no_divider_compact_see_midpage".to_owned(), 10);
    }
    if no_divider && line.y0_ratio() >= 0.55 && small_note_font && strong_legal_note_cue {
        push_weighted(tokens, "geom_no_divider_legal_note".to_owned(), 8);
    }
    if no_divider
        && line.page_index == 0
        && line.y0_ratio() >= 0.35
        && line.font_ratio_page_ref <= 0.98
        && symbol_note_marker
    {
        push_weighted(tokens, "geom_first_page_symbol_author_note".to_owned(), 8);
    }
    if no_divider
        && (0.24..=0.72).contains(&line.y0_ratio())
        && line.font_ratio_page_ref <= 1.02
        && bibliographic_lead
    {
        push_weighted(tokens, "geom_no_divider_bibliographic_lead".to_owned(), 8);
    }
    if no_divider && line.sequence_footnote_zone && line.y0_ratio() >= 0.55 && small_note_font {
        push_weighted(tokens, "geom_no_divider_sequence_note".to_owned(), 8);
    }
    if no_divider
        && line.y0_ratio() < 0.55
        && small_note_font
        && !note_marker
        && !strong_legal_note_cue
    {
        push_weighted(tokens, "geom_no_divider_small_mid_body".to_owned(), 4);
    }
}

fn add_lexical_ngram_tokens(text: &str, tokens: &mut Vec<String>) {
    let words = model_words(text);
    for pair in words.windows(2).take(7) {
        tokens.push(format!("wb={}_{}", pair[0], pair[1]));
    }
    let compact = compact_lexical_ngram_text(text);
    if compact.is_empty() {
        return;
    }
    let chars: Vec<char> = compact.chars().take(64).collect();
    for index in 0..chars.len().saturating_sub(2).min(20) {
        let gram: String = chars[index..index + 3].iter().collect();
        tokens.push(format!("c3={gram}"));
    }
    for index in 0..chars.len().saturating_sub(3).min(16) {
        let gram: String = chars[index..index + 4].iter().collect();
        tokens.push(format!("c4={gram}"));
    }
}

fn compact_lexical_ngram_text(text: &str) -> String {
    let mut output = String::new();
    let mut last_was_sep = true;
    for ch in text.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            output.push(ch);
            last_was_sep = false;
        } else if !last_was_sep {
            output.push('_');
            last_was_sep = true;
        }
        if output.len() >= 80 {
            break;
        }
    }
    output.trim_matches('_').to_owned()
}

fn footnote_specialist_feature_tokens(line: &LayoutLine) -> Vec<String> {
    feature_tokens(line)
}

fn header_footer_specialist_feature_tokens(line: &LayoutLine) -> Vec<String> {
    feature_tokens(line)
}

fn first_page_repeated_line_can_be_title(
    line: &LayoutLine,
    word_count: usize,
    y_ratio: f32,
) -> bool {
    line.page_index == 0
        && y_ratio <= 0.22
        && (2..=24).contains(&word_count)
        && !is_metadata_line(line)
        && !is_contents_line(&line.text)
        && !is_caption_line(&line.text)
        && !looks_like_probable_author_line(line)
}

fn is_metadata_line(line: &LayoutLine) -> bool {
    let lower = line.text.to_ascii_lowercase();
    if line.page_index <= 1 && looks_like_probable_author_line(line) {
        return true;
    }
    if line.page_index <= 1
        && [
            "ssrn",
            "doi",
            "abstract id",
            "keywords:",
            "jel",
            "university",
            "school of law",
        ]
        .iter()
        .any(|needle| lower.contains(needle))
    {
        return true;
    }
    lower.starts_with("by ")
        || lower.starts_with("author:")
        || lower.starts_with("authors:")
        || lower.starts_with("draft:")
        || lower.starts_with("version:")
}

fn looks_like_probable_author_line(line: &LayoutLine) -> bool {
    if line.page_index > 1 {
        return false;
    }
    let words = model_words(&line.text)
        .into_iter()
        .map(|word| word.trim_matches('.').to_owned())
        .collect::<Vec<_>>();
    if !(2..=5).contains(&words.len()) {
        return false;
    }
    let lower = line.text.to_ascii_lowercase();
    if [
        "review",
        "journal",
        "law",
        "university",
        "school",
        "abstract",
    ]
    .iter()
    .any(|token| lower.contains(token))
    {
        return false;
    }
    let title_terms = [
        "agency",
        "canon",
        "contract",
        "contracts",
        "duty",
        "drift",
        "ideological",
        "meaning",
        "ordinary",
        "practice",
        "read",
        "testing",
        "transparency",
        "unreadable",
    ];
    if words.iter().any(|word| {
        title_terms.iter().any(|term| {
            word.trim_matches(['.', '\'', '-'])
                .eq_ignore_ascii_case(term)
        })
    }) {
        return false;
    }
    let initialish = line.text.split_whitespace().any(|word| {
        word.trim_matches(|ch: char| !ch.is_ascii_alphabetic())
            .chars()
            .count()
            == 1
    });
    let short_width = (line.right - line.left) / line.page_width.max(1.0) <= 0.55;
    short_width
        && (uppercase_ratio(&line.text) >= 0.82
            || (initialish && title_case_ratio(&line.text) >= 0.70))
}

fn title_case_ratio(text: &str) -> f32 {
    let words = text
        .split_whitespace()
        .map(|word| word.trim_matches(|ch: char| !ch.is_alphabetic()))
        .filter(|word| word.chars().count() > 2)
        .collect::<Vec<_>>();
    if words.is_empty() {
        return 0.0;
    }
    let title_case = words
        .iter()
        .filter(|word| word.chars().next().is_some_and(char::is_uppercase))
        .count();
    title_case as f32 / words.len() as f32
}

impl LayoutLine {
    fn y0_ratio(&self) -> f32 {
        (self.page_height - self.top).clamp(0.0, self.page_height.max(1.0))
            / self.page_height.max(1.0)
    }

    fn y1_ratio(&self) -> f32 {
        (self.page_height - self.bottom).clamp(0.0, self.page_height.max(1.0))
            / self.page_height.max(1.0)
    }
}

fn push_weighted(tokens: &mut Vec<String>, token: String, weight: usize) {
    for _ in 0..weight.max(1) {
        tokens.push(token.clone());
    }
}

fn bucket(value: f32, cuts: &[(f32, &'static str)]) -> &'static str {
    for (cut, label) in cuts {
        if value <= *cut {
            return label;
        }
    }
    "hi"
}

fn stable_hash(token: &str) -> u64 {
    let mut result = 0xcbf29ce484222325u64;
    for byte in token.as_bytes() {
        result ^= u64::from(*byte);
        result = result.wrapping_mul(0x100000001b3);
    }
    result
}

fn model_words(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let lower = text.to_ascii_lowercase();
    let mut current = String::new();
    for ch in lower.chars() {
        if current.is_empty() {
            if ch.is_ascii_lowercase() {
                current.push(ch);
            }
        } else if ch.is_ascii_alphanumeric() || ch == '\'' || ch == '-' {
            current.push(ch);
        } else {
            if current.len() >= 2 {
                words.push(std::mem::take(&mut current));
            }
            current.clear();
        }
    }
    if current.len() >= 2 {
        words.push(current);
    }
    words
}

fn model_word_count(text: &str) -> usize {
    let mut count = 0usize;
    let mut in_word = false;
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() {
            if !in_word {
                count += 1;
                in_word = true;
            }
        } else {
            in_word = false;
        }
    }
    count
}

fn strip_trailing_closers(text: &str) -> &str {
    text.trim_end()
        .trim_end_matches(['"', '\'', ')', ']', '}', '\u{2019}', '\u{201d}'])
        .trim_end()
}

fn line_ends_with_period(text: &str) -> bool {
    strip_trailing_closers(text).ends_with('.')
}

fn contains_l_rev_cue(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let compact = lower.split_whitespace().collect::<Vec<_>>().join(" ");
    compact.contains("l. rev.") || compact.contains("l.rev.")
}

fn terminal_punctuation_name(text: &str) -> &'static str {
    let stripped = text.trim_end();
    if stripped.is_empty() {
        return "none";
    }
    let without_closers = strip_trailing_closers(stripped);
    let candidate = if !without_closers.is_empty() && without_closers != stripped {
        without_closers
    } else {
        stripped
    };
    match candidate.chars().next_back() {
        Some('.') => "period",
        Some(':') => "colon",
        Some('?') => "question",
        Some('!') => "exclamation",
        Some(';') => "semicolon",
        Some(',') => "comma",
        Some('-' | '\u{2010}' | '\u{2011}' | '\u{2012}' | '\u{2013}' | '\u{2014}') => "dash",
        Some(')' | ']' | '}') => "close_paren",
        _ => "none",
    }
}

fn is_repeated_header_footer(
    line: &LayoutLine,
    counts: &HashMap<String, usize>,
    page_height: f32,
) -> bool {
    let key = normalize_model_text(&line.text);
    if key.is_empty() {
        return false;
    }
    let edge = line.y0_ratio() <= 0.08 || line.y1_ratio() >= 0.93;
    edge && (counts.get(&key).copied().unwrap_or(0) >= 3 || is_plain_page_number(&key))
        && page_height > 0.0
}

fn normalize_model_text(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn is_plain_page_number(text: &str) -> bool {
    let trimmed = text.trim_matches(['-', ' ', '\u{2013}', '\u{2014}']);
    !trimmed.is_empty() && trimmed.len() <= 4 && trimmed.chars().all(|ch| ch.is_ascii_digit())
}

fn is_plain_page_number_line(text: &str) -> bool {
    is_plain_page_number(&normalize_model_text(text))
}

fn looks_like_plain_numeric_citation_fragment(text: &str) -> bool {
    let trimmed = text
        .trim()
        .trim_matches(|ch: char| matches!(ch, ',' | ';' | '.' | ')' | '(' | ' '));
    !trimmed.is_empty() && trimmed.len() <= 3 && trimmed.chars().all(|ch| ch.is_ascii_digit())
}

fn looks_like_concrete_citation_fragment(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    looks_like_case_name_continuation_fragment(trimmed)
        || contains_year_parenthetical(trimmed)
        || looks_like_comma_pincite_fragment(trimmed)
        || lower.contains(" seq.")
        || lower.starts_with("seq.")
}

fn looks_like_case_name_continuation_fragment(text: &str) -> bool {
    let lower = text.trim_start().to_ascii_lowercase();
    if !lower.starts_with("v. ") && !lower.contains(" v. ") {
        return false;
    }
    text.chars().any(|ch| ch.is_ascii_uppercase())
}

fn contains_year_parenthetical(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.windows(6).any(|window| {
        window[0] == b'('
            && (window[1] == b'1' || window[1] == b'2')
            && window[2].is_ascii_digit()
            && window[3].is_ascii_digit()
            && window[4].is_ascii_digit()
            && (window[5] == b')' || window[5].is_ascii_alphabetic())
    })
}

fn looks_like_comma_pincite_fragment(text: &str) -> bool {
    let trimmed = text.trim();
    let Some(digits) = trimmed.strip_suffix(',').map(str::trim) else {
        return false;
    };
    (1..=3).contains(&digits.len()) && digits.chars().all(|ch| ch.is_ascii_digit())
}

fn looks_like_edge_running_header_footer_fragment(line: &LayoutLine) -> bool {
    if line.page_index == 0 || !(line.y0_ratio() <= 0.12 || line.y1_ratio() >= 0.88) {
        return false;
    }
    if is_plain_numeric_footnote_marker_candidate(line) {
        return false;
    }
    let trimmed = line.text.trim();
    if trimmed.is_empty() {
        return false;
    }
    if looks_like_edge_case_name_running_header_fragment(line) {
        return true;
    }
    let legal_running_header = looks_like_edge_legal_running_header_fragment(line);
    if word_count(trimmed) > 8 && !legal_running_header {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    if is_plain_page_number_line(trimmed)
        || looks_like_edge_year_fragment(trimmed)
        || looks_like_edge_numeric_fragment(trimmed)
        || lower.starts_with("[vol.")
        || lower.starts_with("vol. ")
        || legal_running_header
    {
        return true;
    }
    if starts_with_note_marker(&line.text) || contains_legal_note_cue(&line.text) {
        return false;
    }
    word_count(trimmed) >= 2
        && (uppercase_ratio(trimmed) >= 0.72 || title_case_ratio(trimmed) >= 0.65)
}

fn looks_like_edge_case_name_running_header_fragment(line: &LayoutLine) -> bool {
    if line.page_index == 0 || !(line.y0_ratio() <= 0.11 || line.y1_ratio() >= 0.93) {
        return false;
    }
    let trimmed = line.text.trim();
    if trimmed.chars().count() < 6 || word_count(trimmed) > 8 || starts_with_note_marker(trimmed) {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    lower.contains(" v. ") || lower.starts_with("in re ")
}

fn looks_like_edge_legal_running_header_fragment(line: &LayoutLine) -> bool {
    let trimmed = line.text.trim();
    if word_count(trimmed) > 14 {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    let width_ratio = (line.right - line.left) / line.page_width.max(1.0);
    (lower.contains("law review")
        && (lower.contains("vol.") || lower.contains("iss.") || lower.contains("art.")))
        || (uppercase_ratio(trimmed) >= 0.75 && lower.contains(" v. ") && width_ratio < 0.55)
}

fn looks_like_edge_year_fragment(text: &str) -> bool {
    let trimmed = text.trim().trim_start_matches('[').trim_end_matches(']');
    trimmed.len() == 4
        && trimmed.chars().all(|ch| ch.is_ascii_digit())
        && (trimmed.starts_with("19") || trimmed.starts_with("20"))
}

fn looks_like_edge_numeric_fragment(text: &str) -> bool {
    let trimmed = text.trim().trim_end_matches(']');
    (3..=5).contains(&trimmed.len()) && trimmed.chars().all(|ch| ch.is_ascii_digit())
}

fn plain_numeric_line_digit_count(text: &str) -> Option<usize> {
    let normalized = normalize_model_text(text);
    let trimmed = normalized.trim_matches(['-', ' ', '\u{2013}', '\u{2014}']);
    if trimmed.is_empty() || !trimmed.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    Some(trimmed.len())
}

fn is_plain_numeric_footnote_marker_candidate(line: &LayoutLine) -> bool {
    let Some(digits) = plain_numeric_line_digit_count(&line.text) else {
        return false;
    };
    if digits > 3 {
        return false;
    }
    (line.below_footnote_divider && line.font_ratio_page_ref <= 0.92)
        || (line.sequence_footnote_zone && line.font_ratio_page_ref <= 0.92)
        || (line.y0_ratio() >= 0.80 && line.font_ratio_page_ref <= 0.75)
}

fn is_bare_numeric_note_marker(text: &str) -> bool {
    let trimmed = text.trim();
    let digits = trimmed.chars().take_while(|ch| ch.is_ascii_digit()).count();
    if !(1..=4).contains(&digits) {
        return false;
    }
    let rest = trimmed[digits..].trim();
    matches!(rest, "." | ")" | "]")
}

fn looks_like_enum_marker(text: &str) -> bool {
    let trimmed = text.trim_start();
    if trimmed.starts_with(['*', '-', '\u{2022}']) {
        return true;
    }
    let Some((marker, body)) = trimmed.split_once(char::is_whitespace) else {
        return false;
    };
    if body.trim().is_empty() {
        return false;
    }
    let normalized = marker.trim_matches(['(', ')', '.', ']']);
    if normalized.is_empty() || normalized.len() > 5 {
        return false;
    }
    normalized.chars().all(|ch| ch.is_ascii_alphanumeric())
}

fn is_contents_line(text: &str) -> bool {
    text.matches('.').count() >= 8
        && text
            .trim_end()
            .chars()
            .last()
            .is_some_and(|ch| ch.is_ascii_digit())
}

fn looks_like_contents_heading_text(text: &str) -> bool {
    let lower = normalize_model_text(text);
    matches!(
        lower.as_str(),
        "contents"
            | "table of contents"
            | "index"
            | "index of cases"
            | "table of cases"
            | "index to annual survey"
    ) || lower.starts_with("index to ")
        || lower.starts_with("table of ")
}

fn looks_like_dot_leader_contents_line(text: &str) -> bool {
    let trimmed = text.trim();
    if !ends_with_page_number(trimmed) {
        return false;
    }
    if !has_dot_leader_run(trimmed)
        && !(has_spaced_dot_leader_run(trimmed) && !contains_legal_note_cue(trimmed))
    {
        return false;
    }
    trimmed.matches('.').count() + trimmed.matches('…').count() + trimmed.matches("â€¦").count()
        >= 3
}

fn looks_like_dot_leader_contents_fragment(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.chars().count() > 180 || word_count(trimmed) > 24 {
        return false;
    }
    if !has_dot_leader_run(trimmed) && !has_spaced_dot_leader_run(trimmed) {
        return false;
    }
    trimmed.matches('.').count() >= 3
}

fn has_dot_leader_run(text: &str) -> bool {
    let mut run = 0usize;
    for ch in text.chars() {
        if ch == '.' {
            run += 1;
            if run >= 3 {
                return true;
            }
        } else {
            run = 0;
        }
    }
    false
}

fn has_spaced_dot_leader_run(text: &str) -> bool {
    let mut dots = 0usize;
    let mut saw_space_after_dot = false;
    for ch in text.chars() {
        if ch == '.' {
            dots += 1;
            if dots >= 4 && saw_space_after_dot {
                return true;
            }
            saw_space_after_dot = false;
        } else if ch.is_whitespace() && dots > 0 {
            saw_space_after_dot = true;
        } else if !ch.is_whitespace() {
            dots = 0;
            saw_space_after_dot = false;
        }
    }
    false
}

fn looks_like_case_index_entry(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.chars().count() > 140 || word_count(trimmed) > 16 || !ends_with_page_number(trimmed)
    {
        return false;
    }
    let lower = normalize_model_text(trimmed);
    lower.contains(" v. ")
        || lower.starts_with("in re ")
        || lower.starts_with("ex parte ")
        || lower.starts_with("state v.")
        || lower.starts_with("united states v.")
        || looks_like_reversed_name_index_entry(trimmed)
}

fn looks_like_contents_or_index_entry_text(text: &str) -> bool {
    is_contents_line(text)
        || looks_like_dot_leader_contents_line(text)
        || looks_like_case_index_entry(text)
}

fn looks_like_split_name_index_entry(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.chars().count() > 90 || word_count(trimmed) > 10 || contains_legal_note_cue(trimmed)
    {
        return false;
    }
    let Some((surname, rest)) = trimmed.split_once(',') else {
        return false;
    };
    let surname = surname.trim();
    let rest = rest.trim_start();
    if surname.chars().count() < 2
        || !surname
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_uppercase())
    {
        return false;
    }
    let Some(colon_index) = rest.find(':') else {
        return false;
    };
    let given = rest[..colon_index].trim();
    !given.is_empty()
        && given.chars().count() <= 28
        && given
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_uppercase())
        && given.chars().any(|ch| ch.is_ascii_alphabetic())
}

fn ends_with_page_number(text: &str) -> bool {
    let trimmed = text.trim_end();
    let suffix = trimmed
        .rsplit_once(char::is_whitespace)
        .map(|(_, suffix)| suffix)
        .unwrap_or(trimmed)
        .trim_end_matches([',', ';']);
    if suffix.is_empty() {
        return false;
    }
    let mut parts = suffix.split(',');
    parts.all(|part| {
        let part = part.trim();
        (1..=4).contains(&part.len()) && part.chars().all(|ch| ch.is_ascii_digit())
    })
}

fn looks_like_reversed_name_index_entry(text: &str) -> bool {
    let Some((first, rest)) = text.split_once(',') else {
        return false;
    };
    let first = first.trim();
    let rest = rest.trim_start();
    first
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_uppercase())
        && rest
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_uppercase())
}

fn is_table_line(text: &str) -> bool {
    if is_plain_page_number_line(text) {
        return false;
    }
    if is_contents_line(text) {
        return false;
    }
    if looks_like_running_law_review_cite_line(text) {
        return false;
    }
    let digit_count = text.chars().filter(|ch| ch.is_ascii_digit()).count();
    digit_count >= 4
        && (digit_count as f32 / text.chars().count().max(1) as f32) >= 0.18
        && word_count(text) <= 16
}

fn looks_like_running_law_review_cite_line(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() || word_count(trimmed) > 18 {
        return false;
    }
    let lower = normalize_model_text(trimmed);
    let leading_digits = trimmed.chars().take_while(|ch| ch.is_ascii_digit()).count();

    if (3..=4).contains(&leading_digits) && trimmed.len() > leading_digits {
        let rest = trimmed[leading_digits..].trim_start();
        let starts_with_text = rest
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_alphabetic());
        let journal_name = lower.contains(" law review")
            || lower.contains(" law journal")
            || lower.contains(" journal of");
        let volume_cite =
            lower.contains("[vol.") || lower.contains(" vol.") || lower.contains(" vol ");
        let colon_pagination = lower.split_whitespace().any(|part| {
            part.chars().any(|ch| ch == ':') && part.chars().any(|ch| ch.is_ascii_digit())
        });
        if starts_with_text && journal_name && volume_cite && colon_pagination {
            return true;
        }
    }

    if leading_digits == 4 && trimmed[leading_digits..].starts_with(']') {
        let rest = trimmed[leading_digits + 1..].trim();
        let trailing_page = rest
            .split_whitespace()
            .last()
            .is_some_and(is_plain_page_number);
        if trailing_page
            && (4..=14).contains(&word_count(rest))
            && title_case_ratio(rest) >= 0.45
            && !contains_strong_legal_note_cue(rest)
        {
            return true;
        }
    }

    false
}

fn looks_like_law_review_journal_masthead_line(line: &LayoutLine) -> bool {
    let trimmed = line.text.trim();
    if trimmed.is_empty() {
        return false;
    }
    let words = trimmed
        .split(|ch: char| !ch.is_ascii_alphabetic())
        .filter(|word| !word.is_empty())
        .count();
    if !(2..=8).contains(&words) || trimmed.chars().any(|ch| ch.is_ascii_digit()) {
        return false;
    }
    let lower = normalize_model_text(trimmed);
    let journal_name = lower.ends_with(" law review")
        || lower.ends_with(" law journal")
        || (lower.starts_with("journal of ") && lower.contains(" law"))
        || matches!(
            lower.as_str(),
            "georgetown law journal"
                | "minnesota law review"
                | "mercer law review"
                | "oregon law review"
                | "creighton law review"
        );
    if !journal_name {
        return false;
    }
    if uppercase_ratio(trimmed) < 0.65 && title_case_ratio(trimmed) < 0.75 {
        return false;
    }
    let width_ratio = (line.right - line.left) / line.page_width.max(1.0);
    let in_top_band = line.y0_ratio() <= 0.18 || line.page_index <= 1;
    in_top_band
        && (width_ratio <= 0.72
            || (line.centered && line.y0_ratio() <= 0.18 && width_ratio <= 0.90))
}

fn is_caption_line(text: &str) -> bool {
    let lower = text.trim_start().to_ascii_lowercase();
    lower.starts_with("figure ")
        || lower.starts_with("fig. ")
        || lower.starts_with("table ")
        || lower.starts_with("source:")
        || lower.starts_with("sources:")
        || lower.starts_with("photo:")
        || lower.starts_with("credit:")
}

fn uppercase_ratio(text: &str) -> f32 {
    let mut letters = 0usize;
    let mut uppercase = 0usize;
    for ch in text.chars().filter(|ch| ch.is_alphabetic()) {
        letters += 1;
        uppercase += usize::from(ch.is_uppercase());
    }
    if letters == 0 {
        0.0
    } else {
        uppercase as f32 / letters as f32
    }
}

fn y_from_top(page: &PageInfo, line: &LayoutLine) -> f32 {
    (page.height - line.top).clamp(0.0, page.height.max(1.0))
}

fn starts_with_note_marker(text: &str) -> bool {
    let trimmed = text.trim_start();
    if starts_with_symbol_note_marker(trimmed) {
        return true;
    }

    if let Some(rest) = trimmed.strip_prefix('[') {
        let digits = rest.chars().take_while(|ch| ch.is_ascii_digit()).count();
        if (1..=4).contains(&digits) && rest.chars().nth(digits) == Some(']') {
            return true;
        }
    }

    if let Some(rest) = trimmed.strip_prefix('(') {
        let digits = rest.chars().take_while(|ch| ch.is_ascii_digit()).count();
        if (1..=4).contains(&digits) && rest.chars().nth(digits) == Some(')') {
            return true;
        }
    }

    let digits = trimmed.chars().take_while(|ch| ch.is_ascii_digit()).count();
    if !(1..=4).contains(&digits) {
        return false;
    }
    if starts_with_compact_legal_note_marker(trimmed) {
        return true;
    }
    let Some(separator) = trimmed.chars().nth(digits) else {
        return false;
    };
    if separator.is_whitespace() || separator == ')' {
        return true;
    }
    if separator != '.' {
        return false;
    }
    let after_dot = &trimmed[digits + separator.len_utf8()..];
    after_dot
        .chars()
        .next()
        .is_none_or(|ch| ch.is_whitespace() || matches!(ch, ')' | ']'))
}

fn starts_with_symbol_note_marker(text: &str) -> bool {
    text.trim_start()
        .chars()
        .next()
        .is_some_and(|ch| matches!(ch, '*' | '∗' | '†' | '‡'))
}

fn starts_with_compact_legal_note_marker(text: &str) -> bool {
    let trimmed = text.trim_start();
    let digits = trimmed.chars().take_while(|ch| ch.is_ascii_digit()).count();
    if !(1..=4).contains(&digits) || trimmed.chars().count() <= digits {
        return false;
    }
    let rest = trimmed[digits..]
        .trim_start_matches('.')
        .to_ascii_lowercase();
    rest.starts_with("see")
        || rest.starts_with("cf.")
        || rest.starts_with("id.")
        || rest.starts_with("id ")
        || rest.starts_with("accord")
        || rest.starts_with("butsee")
        || rest.starts_with("but see")
}

fn starts_with_legal_note_marker(text: &str) -> bool {
    if starts_with_compact_legal_note_marker(text) {
        return true;
    }
    let Some(body) = numeric_note_marker_body(text) else {
        return false;
    };
    let lower = body.to_ascii_lowercase();
    lower.starts_with("see")
        || lower.starts_with("cf.")
        || lower.starts_with("cf ")
        || lower.starts_with("id.")
        || lower.starts_with("id ")
        || lower.starts_with("accord")
        || lower.starts_with("but see")
}

fn starts_with_lowercase_letter(text: &str) -> bool {
    text.trim_start()
        .chars()
        .find(|ch| ch.is_alphabetic())
        .is_some_and(char::is_lowercase)
}

fn starts_with_numeric_lowercase_body_fragment(text: &str) -> bool {
    let Some(body) = numeric_note_marker_body(text) else {
        return false;
    };
    if starts_with_legal_note_marker(text) {
        return false;
    }
    let starts_lowercase = body
        .chars()
        .find(|ch| ch.is_alphabetic())
        .is_some_and(char::is_lowercase);
    starts_lowercase && !contains_strong_legal_note_cue(body)
}

fn looks_like_general_citation_note_start(text: &str) -> bool {
    let Some(body) = numeric_note_marker_body(text) else {
        return false;
    };
    let lower = body.to_ascii_lowercase();
    if starts_with_reporter_abbreviation(&lower) {
        return false;
    }
    let citation_cue = [
        "http://",
        "https://",
        "www.",
        "journal",
        "l. rev",
        "law review",
        "rev.",
        "univ.",
        "press",
        "forbes",
        "pew res",
        "res.ctr",
        "res. ctr",
        "news",
        "times",
        "post",
    ]
    .iter()
    .any(|needle| lower.contains(needle));
    let year_or_date = [
        "(19", "(20", " 19", " 20", "(jan.", "(feb.", "(mar.", "(apr.", "(may", "(jun.", "(jul.",
        "(aug.", "(sep.", "(sept.", "(oct.", "(nov.", "(dec.",
    ]
    .iter()
    .any(|needle| lower.contains(needle));
    let source_author_cue = (lower.contains(" et al") || body.contains(" & "))
        && body.contains(',')
        && title_case_ratio(body) >= 0.35;
    citation_cue || source_author_cue || year_or_date && body.contains(',')
}

fn starts_with_reporter_abbreviation(lower: &str) -> bool {
    let stripped = lower
        .trim_start_matches(|ch: char| ch.is_whitespace() || matches!(ch, '"' | '\'' | '(' | '['));
    [
        "u.s.", "u. s.", "s.ct.", "s. ct.", "f.2d", "f.3d", "f. supp", "n.e.", "n. e.", "n.w.",
        "n. w.", "s.e.", "s. e.", "s.w.", "s. w.", "p.2d", "p. 2d", "a.2d", "a. 2d", "n.y.",
        "n. y.",
    ]
    .iter()
    .any(|prefix| stripped.starts_with(prefix))
}

fn numeric_note_marker_body(text: &str) -> Option<&str> {
    let trimmed = text.trim_start();
    let mut digits = 0usize;
    let mut marker_end = 0usize;
    for (index, ch) in trimmed.char_indices() {
        if ch.is_ascii_digit() {
            digits += 1;
            marker_end = index + ch.len_utf8();
            continue;
        }
        break;
    }
    if !(1..=4).contains(&digits) || trimmed.len() <= marker_end {
        return None;
    }
    let rest = &trimmed[marker_end..];
    let separator_is_valid = rest
        .chars()
        .next()
        .is_some_and(|ch| ch.is_whitespace() || matches!(ch, '.' | ')' | ']'));
    if !separator_is_valid {
        return None;
    }
    Some(
        rest.trim_start_matches(|ch: char| matches!(ch, '.' | ')' | ']' | ' ' | '\t'))
            .trim_start(),
    )
}

fn contains_legal_note_cue(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    if contains_short_form_citation_cue(&lower) {
        return true;
    }
    if contains_us_reporter_cue(&lower) {
        return true;
    }
    [
        " see ",
        " cf. ",
        " id.",
        " ibid",
        " supra",
        " infra",
        " v. ",
        " u.s.c.",
        " s. ct.",
        " f.2d",
        " f.3d",
        " f. supp",
        " l. rev",
        " law review",
        " restatement",
        " statute",
    ]
    .iter()
    .any(|cue| lower.contains(cue))
}

fn contains_us_reporter_cue(lower_text: &str) -> bool {
    let mut offset = 0usize;
    while let Some(relative) = lower_text[offset..].find("u.") {
        let index = offset + relative;
        let before = lower_text[..index].chars().next_back();
        let before_ok = before.map_or(true, |ch| !ch.is_ascii_alphabetic());
        let mut cursor = index + "u.".len();
        while let Some(ch) = lower_text[cursor..].chars().next() {
            if !ch.is_whitespace() {
                break;
            }
            cursor += ch.len_utf8();
        }
        let rest = &lower_text[cursor..];
        if let Some(after_s) = rest.strip_prefix("s.") {
            let after = after_s.chars().next();
            let after_ok = after.map_or(true, |ch| !ch.is_ascii_alphabetic() && ch != '.');
            if before_ok && after_ok {
                return true;
            }
        }
        offset = index + "u.".len();
    }
    false
}

fn contains_short_form_citation_cue(text: &str) -> bool {
    let lower = text.trim_start().to_ascii_lowercase();
    let Some(rest) = leading_short_form_citation_rest(&lower)
        .or_else(|| loose_leading_short_form_citation_rest(&lower))
    else {
        return lower.contains(" id.") || lower.contains(" ibid.") || lower.contains(" ibid ");
    };
    rest.is_empty()
        || rest
            .chars()
            .next()
            .is_some_and(|ch| ch.is_whitespace() || matches!(ch, ',' | ';' | ')' | ']'))
}

fn leading_short_form_citation_rest(lower: &str) -> Option<&str> {
    lower
        .strip_prefix("id.")
        .or_else(|| lower.strip_prefix("id "))
        .or_else(|| lower.strip_prefix("ibid."))
        .or_else(|| lower.strip_prefix("ibid "))
}

fn loose_leading_short_form_citation_rest(lower: &str) -> Option<&str> {
    let stripped = lower.trim_start_matches(|ch: char| !ch.is_ascii_alphanumeric());
    for skip_chars in 0..=2 {
        let Some((byte_index, _)) = stripped.char_indices().nth(skip_chars) else {
            break;
        };
        let candidate =
            stripped[byte_index..].trim_start_matches(|ch: char| !ch.is_ascii_alphanumeric());
        if let Some(rest) = leading_short_form_citation_rest(candidate) {
            return Some(rest);
        }
    }
    None
}

fn looks_like_citation_continuation_text(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    contains_legal_note_cue(text)
        || lower.contains("supra note")
        || lower.contains("hereinafter")
        || lower.contains("perma.cc")
        || lower.contains("http://")
        || lower.contains("https://")
        || lower.contains(" l.j.")
        || lower.contains(".l.j.")
        || lower.contains(" j.")
        || lower.contains(".j.")
        || lower.contains(" rev.")
        || looks_like_publication_citation_continuation_text(text)
}

fn looks_like_publication_citation_continuation_text(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty()
        || trimmed.contains('@')
        || word_count(trimmed) > 34
        || is_contents_line(trimmed)
    {
        return false;
    }
    let uppercase = trimmed.chars().filter(|ch| ch.is_ascii_uppercase()).count();
    let digits = trimmed.chars().filter(|ch| ch.is_ascii_digit()).count();
    if uppercase < 5 || digits < 2 {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    let has_year = [
        "(19", "(20", " 19", " 20", "(jan.", "(feb.", "(mar.", "(apr.", "(may", "(jun.", "(jul.",
        "(aug.", "(sep.", "(sept.", "(oct.", "(nov.", "(dec.",
    ]
    .iter()
    .any(|cue| lower.contains(cue));
    let has_page_cite = trimmed.contains(',') && digits >= 3;
    let publication_shape = trimmed.contains('.') || trimmed.contains('&');
    publication_shape && (has_year || has_page_cite)
}

fn looks_like_footnote_bibliographic_lead_text(text: &str) -> bool {
    let trimmed = text.trim();
    let words = word_count(trimmed);
    if !(2..=24).contains(&words)
        || trimmed.chars().count() > 180
        || starts_with_note_marker(trimmed)
        || starts_with_lowercase_letter(trimmed)
        || is_contents_line(trimmed)
        || is_table_line(trimmed)
        || is_caption_line(trimmed)
        || contains_legal_note_cue(trimmed)
    {
        return false;
    }

    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("this article ")
        || lower.starts_with("this essay ")
        || lower.starts_with("this part ")
        || lower.starts_with("this section ")
        || lower.starts_with("the court ")
        || lower.starts_with("the parties ")
    {
        return false;
    }

    let title_ratio = title_case_ratio(trimmed);
    let upper_ratio = uppercase_ratio(trimmed);
    let title_like = title_ratio >= 0.45 || upper_ratio >= 0.55;
    let author_like = (lower.contains(" et al") || trimmed.contains(" & "))
        && trimmed.contains(',')
        && title_ratio >= 0.30;
    let edition_or_work_cue = [
        " ed.",
        " eds.",
        " trans.",
        "forthcoming",
        "working paper",
        "press",
        "university",
    ]
    .iter()
    .any(|cue| lower.contains(cue));
    let ends_like_body_sentence =
        trimmed.ends_with('.') && !author_like && !edition_or_work_cue && !lower.contains(',');

    (title_like || author_like || edition_or_work_cue) && !ends_like_body_sentence
}

fn contains_strong_legal_note_cue(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    contains_short_form_citation_cue(&lower)
        || lower.starts_with("see ")
        || lower.starts_with("see, ")
        || lower.starts_with("cf. ")
        || lower.starts_with("accord ")
        || lower.starts_with("but see ")
        || [
            " see ",
            " cf. ",
            " id.",
            " ibid",
            " supra",
            " infra",
            " v. ",
            " s. ct.",
            " f.2d",
            " f.3d",
            " f. supp",
            " l. rev",
            " restatement",
        ]
        .iter()
        .any(|cue| lower.contains(cue))
}

fn looks_like_reporter_citation_text(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    [
        " n. e.",
        " n.e.",
        " n. y.",
        " n.y.",
        " n. w.",
        " n.w.",
        " s. e.",
        " s.e.",
        " s. w.",
        " s.w.",
        " so. 2d",
        " so.2d",
        " p.2d",
        " p. 2d",
        " a.2d",
        " a. 2d",
        " ga. app",
        " cal. app",
        " s. c.",
        " s.c.",
        " neb.",
        " cal.",
        " ill.",
        " mass.",
    ]
    .iter()
    .any(|cue| lower.contains(cue))
}

fn looks_like_statutory_citation_fragment(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    text.contains('§')
        || lower.contains("rev. stat")
        || lower.contains("penal code")
        || lower.contains("gen. stats")
        || lower.contains(" gen. stat")
}

fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

fn median_sorted(values: &[f32]) -> f32 {
    percentile_sorted(values, 0.5)
}

fn percentile_sorted(values: &[f32], percentile: f32) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    let index = ((values.len() - 1) as f32 * percentile).round() as usize;
    values[index.min(values.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_small_law_review_footnote_zone() {
        let page = PageInfo::new(612.0, 792.0);
        let mut chars = Vec::new();
        push_line(
            &mut chars,
            &page,
            120.0,
            11.0,
            "This is ordinary body text on the page.",
        );
        push_line(
            &mut chars,
            &page,
            144.0,
            11.0,
            "A second ordinary body line supplies the body font size.",
        );
        push_line(
            &mut chars,
            &page,
            520.0,
            8.0,
            "1 See Example v. State, 123 U.S. 456 (2020).",
        );
        push_line(
            &mut chars,
            &page,
            532.0,
            8.0,
            "continued citation material in the same small font.",
        );

        let hints = layout_hints_for_pages(&[page], &[Some(chars)]);
        let footnotes = hints
            .iter()
            .filter(|hint| hint.role == LiquidBlockRole::Marginalia)
            .collect::<Vec<_>>();

        assert_eq!(footnotes.len(), 2);
        assert!(footnotes[0].text.starts_with("1 See Example"));
        assert!(footnotes[1].text.starts_with("continued citation"));
    }

    #[test]
    fn split_numeric_marker_starts_old_law_review_footnote_zone() {
        let page = PageInfo::new(612.0, 792.0);
        let mut lines = vec![
            test_line(
                0,
                0,
                120.0,
                650.0,
                500.0,
                660.0,
                "This is ordinary body text above the footnotes.",
            ),
            test_line(
                0,
                1,
                120.0,
                635.0,
                500.0,
                645.0,
                "Another body line establishes the main text region.",
            ),
            test_line(0, 2, 138.0, 602.0, 158.0, 610.0, "15."),
            test_line(
                0,
                3,
                160.0,
                602.0,
                520.0,
                610.0,
                "Rosales-Lopez, 451 U.S. at 190 (discussing jury bias).",
            ),
            test_line(
                0,
                4,
                120.0,
                590.0,
                520.0,
                598.0,
                "continued citation material in the same footnote block.",
            ),
        ];
        lines[0].font_ratio_page_ref = 1.30;
        lines[1].font_ratio_page_ref = 1.30;
        for line in lines.iter_mut().skip(2) {
            line.font_ratio_page_ref = 0.96;
        }

        mark_sequence_footnote_zones(&page, &mut lines);

        assert!(!lines[0].sequence_footnote_zone);
        assert!(lines[2].sequence_footnote_zone);
        assert!(lines[3].sequence_footnote_zone);
        assert!(lines[4].sequence_footnote_zone);
        assert!(model_line_should_be_marginalia(&lines[2]));
        assert!(model_line_should_be_marginalia(&lines[3]));
    }

    #[test]
    fn midpage_indented_note_quote_run_starts_footnote_sequence() {
        let page = PageInfo::new(612.0, 792.0);
        let mut lines = vec![
            test_line(
                5,
                0,
                72.0,
                420.0,
                520.0,
                430.0,
                "This is ordinary body text before a mid-page note.",
            ),
            test_line(
                5,
                1,
                80.0,
                370.0,
                250.0,
                380.0,
                "26. The court stated in Follett:",
            ),
            test_line(
                5,
                2,
                120.0,
                360.0,
                420.0,
                370.0,
                "freedom of religion is not merely reserved for those with a",
            ),
            test_line(
                5,
                3,
                120.0,
                350.0,
                420.0,
                360.0,
                "long purse. preachers are not engaged in commercial",
            ),
            test_line(5, 4, 120.0, 340.0, 420.0, 350.0, "id. at 576-577."),
            test_line(5, 5, 80.0, 330.0, 220.0, 340.0, "Id. at 576-577."),
        ];
        lines[0].font_ratio_page_ref = 1.12;
        for line in lines.iter_mut().skip(1) {
            line.font_ratio_page_ref = 0.78;
        }

        let indices = (0..lines.len()).collect::<Vec<_>>();
        assert!(starts_with_note_marker(&lines[1].text));
        assert!(!starts_with_numeric_lowercase_body_fragment(&lines[1].text));
        assert!(can_bridge_generic_contextual_footnote_line(&lines[2]));
        assert!(can_bridge_generic_contextual_footnote_line(&lines[3]));
        assert!(looks_like_citation_continuation_text(&lines[4].text));
        assert!(starts_midpage_indented_note_quote_run(&lines, &indices, 1));

        mark_sequence_footnote_zones(&page, &mut lines);

        assert!(!lines[0].sequence_footnote_zone);
        assert!(
            lines.iter().skip(1).all(|line| line.sequence_footnote_zone),
            "{:?}",
            lines
                .iter()
                .map(|line| (
                    line.line_index,
                    line.y0_ratio(),
                    line.sequence_footnote_zone
                ))
                .collect::<Vec<_>>()
        );
        assert!(model_line_should_be_marginalia(&lines[1]));
        assert!(should_decode_keep_as_marginalia(
            &[],
            &lines[3],
            Some(&lines[2]),
            Some(&lines[4]),
        ));
    }

    #[test]
    fn small_font_legal_note_run_starts_old_law_review_sequence() {
        let page = PageInfo::new(612.0, 792.0);
        let mut lines = vec![
            test_line(
                0,
                0,
                96.0,
                676.0,
                520.0,
                686.0,
                "This is ordinary body text above the old law review note run.",
            ),
            test_line(
                0,
                1,
                84.0,
                450.0,
                520.0,
                460.0,
                "tion of the court or judge thereof, of the relief granted or denied in an action.",
            ),
            test_line(
                0,
                2,
                84.0,
                438.0,
                320.0,
                448.0,
                "REV. STAT. § 25-1301 (Reissue 1975).",
            ),
            test_line(0, 3, 84.0, 426.0, 104.0, 436.0, "27."),
            test_line(
                0,
                4,
                84.0,
                414.0,
                360.0,
                424.0,
                "199 Neb. at 712, 255 N.W.2d at 266.",
            ),
            test_line(
                0,
                5,
                84.0,
                402.0,
                520.0,
                412.0,
                "Id. In reaching this conclusion the court extended the holding of Pallas v.",
            ),
        ];
        for line in &mut lines[1..] {
            line.font_ratio_page_ref = 0.78;
        }
        let indices = (0..lines.len()).collect::<Vec<_>>();
        assert!(starts_small_font_legal_note_run(&lines, &indices, 1));

        mark_sequence_footnote_zones(&page, &mut lines);
        assert!(!lines[0].sequence_footnote_zone);
        assert!(lines[1..].iter().all(|line| line.sequence_footnote_zone));
        assert!(model_line_should_be_marginalia(&lines[1]));
    }

    #[test]
    fn small_font_body_quote_without_citation_run_does_not_start_sequence() {
        let page = PageInfo::new(612.0, 792.0);
        let mut lines = vec![
            test_line(
                0,
                0,
                96.0,
                676.0,
                520.0,
                686.0,
                "This is ordinary body text above the indented quotation.",
            ),
            test_line(
                0,
                1,
                96.0,
                240.0,
                520.0,
                250.0,
                "\"Louisiana emphasizes the fact that the usual drunken driving statutes do",
            ),
            test_line(
                0,
                2,
                96.0,
                228.0,
                520.0,
                238.0,
                "not require an injury, by providing in a different statute, with a heavier penalty",
            ),
            test_line(
                0,
                3,
                96.0,
                216.0,
                520.0,
                226.0,
                "than usual (GEN. STATS. §§ 5292, 5293) as follows:",
            ),
            test_line(
                0,
                4,
                96.0,
                204.0,
                520.0,
                214.0,
                "\"Section 5292. Operating motor vehicle while intoxicated and causing injury.",
            ),
        ];
        for line in &mut lines {
            line.font_ratio_page_ref = 0.78;
        }
        let indices = (0..lines.len()).collect::<Vec<_>>();
        assert!(!starts_small_font_legal_note_run(&lines, &indices, 1));

        mark_sequence_footnote_zones(&page, &mut lines);
        assert!(!lines.iter().any(|line| line.sequence_footnote_zone));
    }

    #[test]
    fn small_font_quote_citation_run_needs_early_note_marker() {
        let page = PageInfo::new(612.0, 792.0);
        let mut lines = vec![
            test_line(
                0,
                0,
                96.0,
                410.0,
                520.0,
                420.0,
                "ordinary body text before a small-font quotation.",
            ),
            test_line(0, 1, 84.0, 360.0, 170.0, 370.0, "\"Id. at 461."),
            test_line(
                0,
                2,
                84.0,
                348.0,
                290.0,
                358.0,
                "\"292 U. S. 360, 368-9 (1934).",
            ),
            test_line(
                0,
                3,
                84.0,
                336.0,
                540.0,
                346.0,
                "\"Prior to the enactment of the Revenue Act of 1926, the Treasury made this distinction,",
            ),
            test_line(0, 4, 84.0, 324.0, 120.0, 334.0, "but"),
            test_line(
                0,
                5,
                120.0,
                312.0,
                540.0,
                322.0,
                "1211 of the Act was broad enough to exclude all such compensation from taxation,",
            ),
            test_line(
                0,
                6,
                84.0,
                300.0,
                540.0,
                310.0,
                "whether in connection with either kind of state activity.",
            ),
            test_line(
                0,
                7,
                84.0,
                288.0,
                520.0,
                298.0,
                "See Mim. 3397, V-1 Cum. Bull. 36 (1926).",
            ),
        ];
        for line in &mut lines[1..] {
            line.font_ratio_page_ref = 0.80;
        }
        let indices = (0..lines.len()).collect::<Vec<_>>();
        assert!(!looks_like_general_citation_note_start(&lines[2].text));
        assert!(!starts_small_font_legal_note_run(&lines, &indices, 1));

        mark_sequence_footnote_zones(&page, &mut lines);
        let leaked = lines[1..6]
            .iter()
            .filter(|line| line.sequence_footnote_zone)
            .map(|line| format!("{}:{}", line.line_index, line.text))
            .collect::<Vec<_>>();
        assert!(leaked.is_empty(), "unexpected sequence lines: {leaked:?}");
        let hints = vec![LiquidLayoutHint {
            text: lines[1].text.clone(),
            role: LiquidBlockRole::Marginalia,
        }];
        assert!(!model_line_should_be_marginalia(&lines[2]));
        assert!(!should_decode_keep_as_marginalia(
            &hints,
            &lines[2],
            Some(&lines[1]),
            Some(&lines[3]),
        ));
    }

    #[test]
    fn split_numeric_marker_without_citation_confirmation_does_not_start_footnotes() {
        let page = PageInfo::new(612.0, 792.0);
        let mut lines = vec![
            test_line(
                0,
                0,
                120.0,
                650.0,
                500.0,
                660.0,
                "This is ordinary body text above the numbered line.",
            ),
            test_line(0, 1, 138.0, 602.0, 158.0, 610.0, "15."),
            test_line(
                0,
                2,
                160.0,
                602.0,
                520.0,
                610.0,
                "This numbered item continues as ordinary prose without legal citation cues.",
            ),
            test_line(
                0,
                3,
                120.0,
                590.0,
                520.0,
                598.0,
                "The next sentence still has no footnote-style confirmation.",
            ),
        ];
        lines[0].font_ratio_page_ref = 1.30;
        for line in lines.iter_mut().skip(1) {
            line.font_ratio_page_ref = 0.96;
        }

        mark_sequence_footnote_zones(&page, &mut lines);

        assert!(!lines.iter().any(|line| line.sequence_footnote_zone));
    }

    #[test]
    fn split_numeric_marker_before_section_heading_does_not_start_footnotes() {
        let page = PageInfo::new(612.0, 792.0);
        let mut lines = vec![
            test_line(
                4,
                19,
                92.0,
                496.0,
                520.0,
                506.0,
                "federal gift tax return. 45",
            ),
            test_line(4, 20, 92.0, 466.0, 116.0, 476.0, "2."),
            test_line(
                4,
                21,
                114.0,
                466.0,
                320.0,
                476.0,
                "The Lingering Lien Problem",
            ),
            test_line(
                4,
                22,
                116.0,
                446.0,
                520.0,
                456.0,
                "Prior to the floor debates of the Nebraska Legislature,",
            ),
        ];
        for line in &mut lines {
            line.font_ratio_page_ref = 1.0;
        }

        mark_sequence_footnote_zones(&page, &mut lines);

        assert!(!lines.iter().any(|line| line.sequence_footnote_zone));
    }

    #[test]
    fn contextual_bridge_does_not_pull_body_paragraph_into_later_footnotes() {
        let page = PageInfo::new(612.0, 792.0);
        let mut lines = vec![
            test_line(
                1,
                13,
                76.0,
                448.0,
                520.0,
                456.0,
                "Mr. Justice Black, concurring in result only in Interna-",
            ),
            test_line(
                1,
                14,
                48.0,
                432.0,
                520.0,
                440.0,
                "tional Shoe Co. v. Washington,1 criticized the majority's crea-",
            ),
            test_line(
                1,
                15,
                48.0,
                416.0,
                520.0,
                424.0,
                "tion from thin air of a due process-based test for personal juris-",
            ),
            test_line(
                1,
                16,
                48.0,
                400.0,
                520.0,
                408.0,
                "diction and predicted that the minimum contacts analysis",
            ),
            test_line(
                1,
                17,
                48.0,
                384.0,
                520.0,
                392.0,
                "would produce an entirely ad hoc jurisprudence.2 In his view,",
            ),
            test_line(
                1,
                18,
                48.0,
                368.0,
                520.0,
                376.0,
                "every imaginable fact pattern would have to be decided, as a",
            ),
            test_line(
                1,
                19,
                48.0,
                352.0,
                520.0,
                360.0,
                "matter of constitutional magnitude, by the Supreme Court be-",
            ),
            test_line(
                1,
                23,
                72.0,
                266.0,
                520.0,
                274.0,
                "* B.A., 1973, the State University of New York at Stonybrook;",
            ),
            test_line(
                1,
                24,
                48.0,
                250.0,
                520.0,
                258.0,
                "1. 326 U.S. 310, 322-26 (1945) (Black, J., concurring).",
            ),
        ];
        for line in &mut lines[..7] {
            line.font_ratio_page_ref = 1.0;
        }
        for line in &mut lines[7..] {
            line.font_ratio_page_ref = 0.74;
        }

        mark_sequence_footnote_zones(&page, &mut lines);

        assert!(!lines[..7].iter().any(|line| line.sequence_footnote_zone));
        assert!(lines[7..].iter().all(|line| line.sequence_footnote_zone));
    }

    #[test]
    fn repository_cover_boilerplate_breaks_contextual_sequence_bridge() {
        let page = PageInfo::new(612.0, 792.0);
        let mut lines = vec![
            test_line(
                0,
                6,
                72.0,
                552.0,
                540.0,
                562.0,
                "Diversions from the Great Lakes: Out of the Watershed and in",
            ),
            test_line(0, 10, 72.0, 496.0, 300.0, 506.0, "Christina L. Wabiszewski"),
            test_line(
                0,
                12,
                72.0,
                230.0,
                540.0,
                240.0,
                "Follow this and additional works at: https://scholarship.law.marquette.edu/mulr",
            ),
            test_line(0, 14, 72.0, 170.0, 240.0, 180.0, "Repository Citation"),
            test_line(
                0,
                17,
                72.0,
                145.0,
                540.0,
                155.0,
                "the Compact, 100 Marq. L. Rev. 627 (2016).",
            ),
        ];
        for line in &mut lines {
            line.font_ratio_page_ref = 0.92;
        }

        mark_sequence_footnote_zones(&page, &mut lines);

        assert!(!lines[0].sequence_footnote_zone);
        assert!(!lines[1].sequence_footnote_zone);
    }

    #[test]
    fn contents_like_page_does_not_start_split_numeric_footnote_zone() {
        let page = PageInfo::new(612.0, 792.0);
        let mut lines = vec![
            test_line(0, 0, 120.0, 650.0, 500.0, 660.0, "TABLE OF CASES"),
            test_line(0, 1, 120.0, 602.0, 145.0, 610.0, "15."),
            test_line(
                0,
                2,
                160.0,
                602.0,
                520.0,
                610.0,
                "Rosales-Lopez, 451 U.S. at 190",
            ),
            test_line(
                0,
                3,
                120.0,
                590.0,
                520.0,
                598.0,
                "Ristaino v. Ross, 424 U.S. 589, 597",
            ),
        ];
        for line in &mut lines {
            line.font_ratio_page_ref = 0.96;
            line.page_contents_like = true;
        }

        mark_sequence_footnote_zones(&page, &mut lines);

        assert!(!lines.iter().any(|line| line.sequence_footnote_zone));
        assert!(!lines.iter().any(model_line_should_be_marginalia));
    }

    #[test]
    fn single_word_low_page_fragment_without_sequence_is_not_marginalia() {
        let mut fragment = test_line(3, 12, 110.0, 250.0, 180.0, 258.0, "merce");
        fragment.font_ratio_page_ref = 0.82;

        assert!(!footnote_specialist_line_can_be_marginalia(&fragment));
        assert!(!model_line_should_be_marginalia(&fragment));

        fragment.sequence_footnote_zone = true;
        assert!(footnote_specialist_line_can_be_marginalia(&fragment));
    }

    #[test]
    fn divider_extends_footnote_zone_without_citation_marker() {
        let page = PageInfo::with_footnote_divider_y_from_top(612.0, 792.0, Some(500.0));
        let mut chars = Vec::new();
        push_line(
            &mut chars,
            &page,
            120.0,
            11.0,
            "This is ordinary body text above the notes.",
        );
        push_line(
            &mut chars,
            &page,
            146.0,
            11.0,
            "A second body line defines the larger reference font.",
        );
        push_line(
            &mut chars,
            &page,
            520.0,
            8.0,
            "This continuation line is below the divider but has no marker.",
        );
        push_line(
            &mut chars,
            &page,
            534.0,
            8.0,
            "It should still stay out of the main paragraph stream.",
        );

        let hints = layout_hints_for_pages(&[page], &[Some(chars)]);
        let footnotes = hints
            .iter()
            .filter(|hint| hint.role == LiquidBlockRole::Marginalia)
            .collect::<Vec<_>>();

        assert_eq!(footnotes.len(), 2);
        assert!(footnotes[0].text.starts_with("This continuation"));
        assert!(footnotes[1].text.starts_with("It should still"));
    }

    #[test]
    fn small_legal_note_continuation_near_page_top_stays_marginalia() {
        let page = PageInfo::new(612.0, 792.0);
        let mut chars = Vec::new();
        push_line(
            &mut chars,
            &page,
            70.0,
            8.0,
            "101 See Example v. State, 123 U.S. 456 (2020).",
        );
        push_line(
            &mut chars,
            &page,
            150.0,
            11.0,
            "This is ordinary body text on the page.",
        );
        push_line(
            &mut chars,
            &page,
            176.0,
            11.0,
            "A second ordinary body line supplies the body font size.",
        );

        let hints = layout_hints_for_pages(&[page], &[Some(chars)]);

        assert!(hints.iter().any(|hint| {
            hint.role == LiquidBlockRole::Marginalia && hint.text.starts_with("101 See Example")
        }));
    }

    #[test]
    fn repository_cover_boilerplate_below_divider_becomes_noise() {
        let page = PageInfo::with_footnote_divider_y_from_top(612.0, 792.0, Some(500.0));
        let mut chars = Vec::new();
        push_line(&mut chars, &page, 120.0, 11.0, "Example Article Title");
        push_line(
            &mut chars,
            &page,
            146.0,
            11.0,
            "A second body line defines the larger reference font.",
        );
        push_line(&mut chars, &page, 520.0, 8.0, "Recommended Citation");
        push_line(
            &mut chars,
            &page,
            534.0,
            8.0,
            "Santa Clara Law Review, Salute, 6 Santa Clara Lawyer 115 (1965).",
        );
        push_line(
            &mut chars,
            &page,
            541.0,
            8.0,
            "Ross, Michael Eric (2000) \"Antitrust,\" Mercer Law Review: Vol. 51 : No. 4 , Article 3.",
        );
        push_line(
            &mut chars,
            &page,
            548.0,
            8.0,
            "Available at: http://digitalcommons.law.scu.edu/lawreview/vol6/iss2/1",
        );
        push_line(&mut chars, &page, 562.0, 8.0, "sculawlibrarian@gmail.com.");
        push_line(
            &mut chars,
            &page,
            569.0,
            8.0,
            "Digital Commons. For more information, please contact repository@law.mercer.edu.",
        );
        push_line(&mut chars, &page, 583.0, 8.0, "South Carolina Law Review");
        push_line(
            &mut chars,
            &page,
            590.0,
            8.0,
            "Recent Decisions, 20 S. C. L. Rev. 507 (1968).",
        );
        push_line(
            &mut chars,
            &page,
            597.0,
            8.0,
            "inclusion in South Carolina Law Review by an authorized editor of Scholar Commons.",
        );
        push_line(
            &mut chars,
            &page,
            604.0,
            8.0,
            "1 See Example v. State, 123 U.S. 456 (2020).",
        );

        let hints = layout_hints_for_pages(&[page], &[Some(chars)]);

        assert!(hints.iter().any(|hint| {
            hint.role == LiquidBlockRole::Noise && hint.text == "Recommended Citation"
        }));
        assert!(hints.iter().any(|hint| {
            hint.role == LiquidBlockRole::Noise && hint.text.starts_with("Available at:")
        }));
        assert!(hints.iter().any(|hint| {
            hint.role == LiquidBlockRole::Noise && hint.text.contains("Santa Clara Law Review")
        }));
        assert!(hints.iter().any(|hint| {
            hint.role == LiquidBlockRole::Noise && hint.text.contains("Mercer Law Review:")
        }));
        assert!(hints.iter().any(|hint| {
            hint.role == LiquidBlockRole::Noise && hint.text == "sculawlibrarian@gmail.com."
        }));
        assert!(hints.iter().any(|hint| {
            hint.role == LiquidBlockRole::Noise && hint.text.contains("repository@law.mercer.edu")
        }));
        assert!(hints.iter().any(|hint| {
            hint.role == LiquidBlockRole::Noise && hint.text == "South Carolina Law Review"
        }));
        assert!(hints.iter().any(|hint| {
            hint.role == LiquidBlockRole::Noise && hint.text.contains("S. C. L. Rev.")
        }));
        assert!(hints.iter().any(|hint| {
            hint.role == LiquidBlockRole::Noise && hint.text.contains("Scholar Commons")
        }));
        assert!(!hints.iter().any(|hint| {
            hint.role == LiquidBlockRole::Marginalia && hint.text == "Recommended Citation"
        }));
        assert!(hints.iter().any(|hint| {
            hint.role == LiquidBlockRole::Marginalia && hint.text.starts_with("1 See Example")
        }));
    }

    #[test]
    fn clear_section_headings_below_divider_do_not_become_marginalia() {
        let page = PageInfo::with_footnote_divider_y_from_top(612.0, 792.0, Some(500.0));
        let mut chars = Vec::new();
        push_line(&mut chars, &page, 120.0, 11.0, "Example Article Title");
        push_line(
            &mut chars,
            &page,
            146.0,
            11.0,
            "A second body line defines the larger reference font.",
        );
        push_line(&mut chars, &page, 520.0, 8.0, "I. INTRODUCTION");
        push_line(
            &mut chars,
            &page,
            536.0,
            8.0,
            "1 See Example v. State, 123 U.S. 456 (2020).",
        );

        let hints = layout_hints_for_pages(&[page], &[Some(chars)]);

        assert!(!hints.iter().any(|hint| {
            hint.role == LiquidBlockRole::Marginalia && hint.text == "I. INTRODUCTION"
        }));
        assert!(hints.iter().any(|hint| {
            hint.role == LiquidBlockRole::Marginalia && hint.text.starts_with("1 See Example")
        }));
    }

    #[test]
    fn header_footer_specialist_gate_skips_first_page_title_candidates() {
        let mut title = test_line(0, 1, 72.0, 690.0, 540.0, 705.0, "THE DUTY TO READ");
        title.centered = true;
        title.font_ratio_doc = 1.6;

        let mut running_header = test_line(2, 0, 72.0, 742.0, 260.0, 752.0, "THE DUTY TO READ");
        running_header.repeated_header_footer = true;
        running_header.font_ratio_page_ref = 0.95;

        assert!(!header_footer_specialist_line_can_be_header_footer(&title));
        assert!(header_footer_specialist_line_can_be_header_footer(
            &running_header
        ));
    }

    #[test]
    fn footnote_specialist_gate_requires_plausible_note_geometry() {
        let body = test_line(
            0,
            5,
            72.0,
            590.0,
            540.0,
            602.0,
            "I never thought it would happen to me.",
        );
        assert!(!footnote_specialist_line_can_be_marginalia(&body));

        let mut small_note = test_line(
            0,
            20,
            72.0,
            290.0,
            540.0,
            300.0,
            "1 Spears-Gilbert Professor of Law, University of Kentucky College of Law.",
        );
        small_note.font_ratio_page_ref = 0.90;
        assert!(footnote_specialist_line_can_be_marginalia(&small_note));

        let mut divider_continuation = test_line(
            0,
            21,
            72.0,
            250.0,
            540.0,
            260.0,
            "continued citation material in the note zone.",
        );
        divider_continuation.below_footnote_divider = true;
        divider_continuation.font_ratio_page_ref = 0.95;
        assert!(footnote_specialist_line_can_be_marginalia(
            &divider_continuation
        ));
    }

    #[test]
    fn ordinary_body_prose_at_does_not_start_footnote_sequence() {
        let mut body = test_line(
            0,
            5,
            72.0,
            580.0,
            540.0,
            592.0,
            "I never thought it would happen to me at a typical public law school.",
        );
        body.font_ratio_page_ref = 0.90;

        assert!(!contains_legal_note_cue(&body.text));
        assert!(!starts_sequence_footnote_zone(&body));

        let mut citation = test_line(
            0,
            20,
            72.0,
            280.0,
            540.0,
            290.0,
            "See Example v. State, 123 U.S. 456, at 460 (2020).",
        );
        citation.font_ratio_page_ref = 0.90;

        assert!(contains_legal_note_cue(&citation.text));
        assert!(starts_sequence_footnote_zone(&citation));
    }

    #[test]
    fn inline_body_note_marker_does_not_start_midpage_footnote_sequence() {
        let mut inline_marker = test_line(0, 12, 72.0, 430.0, 200.0, 442.0, "5 And the");
        inline_marker.font_ratio_page_ref = 0.90;

        assert!(starts_with_note_marker(&inline_marker.text));
        assert!(!starts_sequence_footnote_zone(&inline_marker));

        let mut below_divider = inline_marker.clone();
        below_divider.below_footnote_divider = true;
        assert!(starts_sequence_footnote_zone(&below_divider));
        assert!(!model_line_should_be_marginalia(&inline_marker));
        assert!(model_line_should_be_marginalia(&below_divider));

        assert!(!is_probable_footnote_start(
            &inline_marker.text,
            true,
            inline_marker.y0_ratio(),
            false
        ));
        assert!(is_probable_footnote_start(
            &below_divider.text,
            true,
            below_divider.y0_ratio(),
            true
        ));
    }

    #[test]
    fn weak_legal_cue_does_not_start_no_divider_sequence_above_lower_note_band() {
        let mut body = test_line(
            3,
            12,
            72.0,
            520.0,
            540.0,
            530.0,
            "operated by the U.S. Department of Education.",
        );
        body.font_ratio_page_ref = 0.75;

        assert!(contains_legal_note_cue(&body.text));
        assert!(!body.below_footnote_divider);
        assert!(body.y0_ratio() < 0.55);
        assert!(!starts_sequence_footnote_zone(&body));

        let mut lower_note = body.clone();
        lower_note.top = 300.0;
        lower_note.bottom = 290.0;
        assert!(lower_note.y0_ratio() >= 0.55);
        assert!(!contains_strong_legal_note_cue(&lower_note.text));
        assert!(!starts_sequence_footnote_zone(&lower_note));

        let mut citation = lower_note.clone();
        citation.text = "financial harm as a result of the agreement's anticompetitive effects. 171 F.3d at 1281.".to_owned();
        assert!(contains_strong_legal_note_cue(&citation.text));
        assert!(starts_sequence_footnote_zone(&citation));
    }

    #[test]
    fn compact_legal_note_marker_starts_early_no_divider_sequence() {
        let mut compact_note = test_line(
            0,
            0,
            72.0,
            546.0,
            520.0,
            554.0,
            "22See Becher, supra note 2, at 729 (discussing the duty to read).",
        );
        compact_note.font_ratio_page_ref = 0.95;

        assert!(starts_with_note_marker(&compact_note.text));
        assert!(starts_sequence_footnote_zone(&compact_note));
        compact_note.sequence_footnote_zone = true;
        assert!(model_line_should_be_marginalia(&compact_note));

        let mut inline_body_fragment = test_line(
            0,
            1,
            72.0,
            546.0,
            520.0,
            554.0,
            "22Without such a duty, ordinary body prose should not start a note zone.",
        );
        inline_body_fragment.font_ratio_page_ref = 0.95;

        assert!(!starts_with_note_marker(&inline_body_fragment.text));
        assert!(!starts_sequence_footnote_zone(&inline_body_fragment));
        assert!(!model_line_should_be_marginalia(&inline_body_fragment));
    }

    #[test]
    fn small_general_citation_note_starts_midpage_no_divider_sequence() {
        let mut citation_note = test_line(
            29,
            24,
            144.0,
            423.0,
            468.0,
            431.0,
            "164 Aaron Smith & Monica Anderson, Online Shopping and E-Commerce, PEW RES.CTR. 2 (Dec.",
        );
        citation_note.font_ratio_page_ref = 0.78;

        assert!(looks_like_general_citation_note_start(&citation_note.text));
        assert!(starts_sequence_footnote_zone(&citation_note));
        citation_note.sequence_footnote_zone = true;
        assert!(model_line_should_be_marginalia(&citation_note));

        let mut inline_body_fragment = test_line(18, 12, 144.0, 423.0, 468.0, 431.0, "117 For");
        inline_body_fragment.font_ratio_page_ref = 0.78;

        assert!(!looks_like_general_citation_note_start(
            &inline_body_fragment.text
        ));
        assert!(!starts_sequence_footnote_zone(&inline_body_fragment));
        assert!(!model_line_should_be_marginalia(&inline_body_fragment));
    }

    #[test]
    fn midpage_general_citation_note_start_allows_slightly_larger_note_font() {
        let mut citation_note = test_line(
            29,
            24,
            144.0,
            498.0,
            468.0,
            506.0,
            "164 Aaron Smith & Monica Anderson, Online Shopping and E-Commerce, PEW RES.CTR. 2 (Dec.",
        );
        citation_note.font_ratio_page_ref = 0.91;

        assert!(citation_note.y0_ratio() < 0.55);
        assert!(looks_like_general_citation_note_start(&citation_note.text));
        assert!(starts_sequence_footnote_zone(&citation_note));
        assert!(model_line_should_be_marginalia(&citation_note));

        let tokens = feature_tokens(&citation_note);
        assert_token_count(&tokens, "geom_no_divider_general_citation_note_start", 8);
        assert_token_count(&tokens, "geom_no_divider_general_cite_midpage", 10);

        let mut inline_body_fragment = test_line(18, 12, 144.0, 498.0, 468.0, 506.0, "117 For");
        inline_body_fragment.font_ratio_page_ref = 0.91;

        assert!(!looks_like_general_citation_note_start(
            &inline_body_fragment.text
        ));
        assert!(!starts_sequence_footnote_zone(&inline_body_fragment));
        assert!(!model_line_should_be_marginalia(&inline_body_fragment));
    }

    #[test]
    fn midpage_compact_legal_note_marker_survives_larger_note_font() {
        let mut compact_note = test_line(
            3,
            14,
            72.0,
            498.0,
            520.0,
            506.0,
            "22See Becher, supra note 2, at 54.",
        );
        compact_note.font_ratio_page_ref = 0.97;

        assert!(compact_note.y0_ratio() < 0.55);
        assert!(starts_with_compact_legal_note_marker(&compact_note.text));
        assert!(starts_sequence_footnote_zone(&compact_note));
        assert!(model_line_should_be_marginalia(&compact_note));

        let tokens = feature_tokens(&compact_note);
        assert_token_count(&tokens, "compact_legal_note_marker", 8);
        assert_token_count(&tokens, "geom_no_divider_compact_see_midpage", 10);
    }

    #[test]
    fn spaced_legal_note_marker_starts_midpage_no_divider_sequence() {
        let mut note = test_line(
            3,
            27,
            72.0,
            420.0,
            520.0,
            430.0,
            "5 See,e.g., Ayres & Schwartz, supra note 2, at 548 n.10.",
        );
        note.font_ratio_page_ref = 0.97;

        assert!(starts_with_note_marker(&note.text));
        assert!(starts_with_legal_note_marker(&note.text));
        assert!(starts_sequence_footnote_zone(&note));
        assert!(model_line_should_be_marginalia(&note));

        let tokens = feature_tokens(&note);
        assert_token_count(&tokens, "legal_note_marker", 8);
        assert_token_count(&tokens, "geom_no_divider_legal_marker_note_start", 8);
    }

    #[test]
    fn numeric_lowercase_body_fragment_does_not_start_footnote_sequence() {
        let mut body = test_line(
            3,
            18,
            72.0,
            350.0,
            520.0,
            360.0,
            "13 bankruptcy protection on October 11, 1996 in the Southern District",
        );
        body.font_ratio_page_ref = 0.92;

        assert!(starts_with_note_marker(&body.text));
        assert!(starts_with_numeric_lowercase_body_fragment(&body.text));
        assert!(!starts_sequence_footnote_zone(&body));
        assert!(!model_line_should_be_marginalia(&body));
    }

    #[test]
    fn citation_continuation_before_marker_starts_contextual_sequence() {
        let page = PageInfo::new(612.0, 792.0);
        let mut lines = vec![
            test_line(
                17,
                10,
                72.0,
                640.0,
                540.0,
                650.0,
                "the formula that underlies the FRE test is: 206.835 - average number",
            ),
            test_line(
                17,
                11,
                72.0,
                580.0,
                540.0,
                590.0,
                "F. Christensen, Does the Readability of Your Brief Affect Your Chance of Winning an Appeal?, 12 J.",
            ),
            test_line(
                17,
                18,
                72.0,
                510.0,
                540.0,
                520.0,
                "101 See, e.g., ReadabilityStatistics Object (Word), MICROSOFT (June 7, 2019).",
            ),
        ];
        lines[0].font_ratio_page_ref = 1.28;
        lines[1].font_ratio_page_ref = 1.0;
        lines[2].font_ratio_page_ref = 1.0;

        assert!(!starts_sequence_footnote_zone(&lines[1]));
        assert!(starts_sequence_footnote_zone(&lines[2]));

        mark_sequence_footnote_zones(&page, &mut lines);

        assert!(!lines[0].sequence_footnote_zone);
        assert!(lines[1].sequence_footnote_zone);
        assert!(lines[2].sequence_footnote_zone);
    }

    #[test]
    fn bibliographic_lead_before_marker_starts_contextual_sequence() {
        let page = PageInfo::new(612.0, 792.0);
        let mut lines = vec![
            test_line(
                5,
                9,
                72.0,
                650.0,
                540.0,
                660.0,
                "This ordinary body paragraph remains above the note zone.",
            ),
            test_line(
                5,
                10,
                72.0,
                510.0,
                540.0,
                520.0,
                "The Law of Standard Form Contracts",
            ),
            test_line(
                5,
                11,
                72.0,
                498.0,
                540.0,
                508.0,
                "Alan Schwartz & Robert E. Scott, Contract Theory and the Limits",
            ),
            test_line(
                5,
                12,
                72.0,
                486.0,
                540.0,
                496.0,
                "113 Yale L.J. 541, 550 (2003).",
            ),
            test_line(
                5,
                13,
                72.0,
                474.0,
                540.0,
                484.0,
                "likely to read typical consumer contracts ex ante and related materials;",
            ),
            test_line(
                5,
                14,
                72.0,
                462.0,
                540.0,
                472.0,
                "5 See Restatement (Second) of Contracts section 211.",
            ),
        ];
        lines[0].font_ratio_page_ref = 1.22;
        for line in &mut lines[1..] {
            line.font_ratio_page_ref = 1.0;
        }

        assert!(looks_like_footnote_bibliographic_lead_text(&lines[1].text));
        assert!(!starts_sequence_footnote_zone(&lines[1]));
        assert!(starts_sequence_footnote_zone(&lines[5]));

        mark_sequence_footnote_zones(&page, &mut lines);

        assert!(!lines[0].sequence_footnote_zone);
        assert!(lines[1].sequence_footnote_zone);
        assert!(lines[2].sequence_footnote_zone);
        assert!(lines[3].sequence_footnote_zone);
        assert!(lines[4].sequence_footnote_zone);
        assert!(lines[5].sequence_footnote_zone);

        let tokens = feature_tokens(&lines[1]);
        assert_token_count(&tokens, "bibliographic_lead_text", 4);
        assert_token_count(&tokens, "geom_no_divider_bibliographic_lead", 8);
    }

    #[test]
    fn bibliographic_lead_shape_does_not_bridge_through_body_prose() {
        let page = PageInfo::new(612.0, 792.0);
        let mut lines = vec![
            test_line(
                5,
                10,
                72.0,
                510.0,
                540.0,
                520.0,
                "The Law of Standard Form Contracts",
            ),
            test_line(
                5,
                11,
                72.0,
                498.0,
                540.0,
                508.0,
                "is central to the dispute and remains ordinary body prose.",
            ),
            test_line(
                5,
                12,
                72.0,
                486.0,
                540.0,
                496.0,
                "6 See Restatement (Second) of Contracts section 211.",
            ),
        ];
        for line in &mut lines {
            line.font_ratio_page_ref = 1.0;
        }

        mark_sequence_footnote_zones(&page, &mut lines);

        assert!(!lines[0].sequence_footnote_zone);
        assert!(!lines[1].sequence_footnote_zone);
        assert!(lines[2].sequence_footnote_zone);
    }

    #[test]
    fn publication_abbreviation_lines_bridge_contextual_sequence() {
        let page = PageInfo::new(612.0, 792.0);
        let mut lines = vec![
            test_line(
                17,
                11,
                72.0,
                590.0,
                540.0,
                600.0,
                "F. Christensen, Does the Readability of Your Brief Affect Your Chance of Winning an Appeal?, 12 J.",
            ),
            test_line(
                17,
                12,
                72.0,
                578.0,
                540.0,
                588.0,
                "APP.PRAC.&PROCESS 145, 147 (2011) (using these tests to analyze readability),",
            ),
            test_line(
                17,
                13,
                72.0,
                566.0,
                540.0,
                576.0,
                "ers: Comprehension and Coverage, 31 LAW &HUM.BEHAV. 177, 181, 185 (2007)",
            ),
            test_line(
                17,
                18,
                72.0,
                500.0,
                540.0,
                510.0,
                "101 See, e.g., ReadabilityStatistics Object (Word), MICROSOFT (June 7, 2019).",
            ),
        ];
        for line in &mut lines {
            line.font_ratio_page_ref = 1.0;
        }

        assert!(looks_like_publication_citation_continuation_text(
            &lines[1].text
        ));
        assert!(looks_like_publication_citation_continuation_text(
            &lines[2].text
        ));

        mark_sequence_footnote_zones(&page, &mut lines);

        assert!(lines.iter().all(|line| line.sequence_footnote_zone));
        let tokens = feature_tokens(&lines[1]);
        assert_token_count(&tokens, "publication_citation_text", 4);
    }

    #[test]
    fn url_citation_continuation_bridges_to_next_note_marker() {
        let page = PageInfo::new(612.0, 792.0);
        let mut lines = vec![
            test_line(
                29,
                20,
                72.0,
                492.0,
                540.0,
                502.0,
                "be considered in breach of this duty.170 In such cases, consumers should be re-",
            ),
            test_line(
                29,
                22,
                72.0,
                440.0,
                540.0,
                450.0,
                "uploads/sites/8/2017/05/PJ_2017.05.10_Media-Attitudes_FINAL.pdf [https://perma.cc/6K4A-KVAC]",
            ),
            test_line(
                29,
                23,
                72.0,
                428.0,
                540.0,
                438.0,
                "(noting that 85% of Americans get news via \"a desktop computer\").",
            ),
            test_line(
                29,
                24,
                72.0,
                416.0,
                540.0,
                426.0,
                "164 Aaron Smith & Monica Anderson, Online Shopping and E-Commerce, PEW RES.CTR. 2 (Dec.",
            ),
        ];
        lines[0].font_ratio_page_ref = 1.22;
        for line in &mut lines[1..] {
            line.font_ratio_page_ref = 0.78;
        }

        assert!(looks_like_citation_continuation_text(&lines[1].text));

        mark_sequence_footnote_zones(&page, &mut lines);

        assert!(!lines[0].sequence_footnote_zone);
        assert!(lines[1].sequence_footnote_zone);
        assert!(lines[2].sequence_footnote_zone);
        assert!(lines[3].sequence_footnote_zone);
    }

    #[test]
    fn ussr_body_text_is_not_us_reporter_citation() {
        assert!(!contains_legal_note_cue(
            "U.S.S.R. and prohibited the importation of several kinds of fur from the"
        ));
        assert!(!contains_legal_note_cue(
            "the export to the U.S.S.R. and related markets"
        ));
        assert!(contains_legal_note_cue("123 U.S. 456 (2020)."));
        assert!(contains_legal_note_cue("292 U. S. 360, 368-9 (1934)."));
    }

    #[test]
    fn state_reporter_fragments_are_legal_citation_evidence() {
        assert!(looks_like_reporter_citation_text(
            "Id. at 418-19, 224 S.E.2d at 344."
        ));
        assert!(looks_like_reporter_citation_text(
            "See Smith v. Jones, 12 Ga. App. 44, 48."
        ));
        assert!(looks_like_reporter_citation_text(
            "The court reached the same result in 99 S.C. 21."
        ));
        assert!(!looks_like_reporter_citation_text(
            "The southeast corner of the map was revised."
        ));
    }

    #[test]
    fn fragmented_publication_piece_can_follow_marginalia_on_contents_like_page() {
        let mut hints = vec![LiquidLayoutHint {
            text: "2. Jena McGregor, The Number of New Female Board Members Actually Dropped Last Year,"
                .to_owned(),
            role: LiquidBlockRole::Marginalia,
        }];
        let mut prev = test_line(2, 29, 148.0, 254.0, 476.0, 264.0, &hints[0].text);
        let mut line = test_line(2, 30, 130.0, 243.0, 154.0, 253.0, "WASH.");
        prev.font_ratio_page_ref = 0.77;
        line.font_ratio_page_ref = 0.77;
        prev.page_contents_like = true;
        line.page_contents_like = true;

        assert!(looks_like_fragmented_publication_note_piece(&line.text));
        assert!(fragmented_publication_piece_adjacent_to_marginalia(
            &hints,
            &line,
            Some(&prev),
            None
        ));
        assert!(should_decode_keep_as_marginalia(
            &hints,
            &line,
            Some(&prev),
            None
        ));

        hints.clear();
        assert!(!fragmented_publication_piece_adjacent_to_marginalia(
            &hints,
            &line,
            Some(&prev),
            None
        ));
    }

    #[test]
    fn footnote_specialist_rejects_normal_font_sequence_without_note_cue() {
        let mut body = test_line(
            3,
            31,
            72.0,
            318.0,
            470.0,
            328.0,
            "was satisfied and service of process was perfected.",
        );
        body.sequence_footnote_zone = true;
        body.font_ratio_page_ref = 0.98;

        assert!(!contains_legal_note_cue(&body.text));
        assert!(!starts_with_note_marker(&body.text));
        assert!(!footnote_specialist_line_can_be_marginalia(&body));

        let mut small_continuation = body.clone();
        small_continuation.font_ratio_page_ref = 0.82;
        assert!(footnote_specialist_line_can_be_marginalia(
            &small_continuation
        ));

        let mut cited_continuation = body.clone();
        cited_continuation.text = "review. Id. at 1348.".to_owned();
        assert!(contains_legal_note_cue(&cited_continuation.text));
        assert!(footnote_specialist_line_can_be_marginalia(
            &cited_continuation
        ));
    }

    #[test]
    fn decoder_rejects_wide_normal_font_sequence_continuation_without_note_cue() {
        let hints = vec![LiquidLayoutHint {
            text: "10 Lenders traditionally".to_owned(),
            role: LiquidBlockRole::Marginalia,
        }];
        let mut prev = test_line(2, 30, 148.0, 340.0, 260.0, 350.0, &hints[0].text);
        prev.sequence_footnote_zone = true;
        prev.font_ratio_page_ref = 0.78;

        let mut wide = test_line(
            2,
            31,
            150.0,
            328.0,
            520.0,
            338.0,
            "offered to borrowers with better credit.",
        );
        wide.sequence_footnote_zone = true;
        wide.font_ratio_page_ref = 1.0;

        assert!(!contains_legal_note_cue(&wide.text));
        assert!(!should_decode_keep_as_marginalia(
            &hints,
            &wide,
            Some(&prev),
            None
        ));

        let mut narrow = wide.clone();
        narrow.right = narrow.left + narrow.page_width * 0.40;
        assert!(should_decode_keep_as_marginalia(
            &hints,
            &narrow,
            Some(&prev),
            None
        ));
    }

    #[test]
    fn high_page_general_citation_note_can_start_sequence() {
        let mut line = test_line(
            18,
            11,
            72.0,
            600.0,
            540.0,
            610.5,
            "106 Arthur C. Graesser et al., Coh-Metrix: Analysis of Text on Cohesion and Language, 36 BE-",
        );
        line.font_ratio_page_ref = 1.0;

        assert!(looks_like_general_citation_note_start(&line.text));
        assert!(starts_sequence_footnote_zone(&line));
    }

    #[test]
    fn first_page_symbol_author_note_starts_sequence() {
        let page = PageInfo::new(612.0, 792.0);
        let mut lines = vec![
            test_line(
                0,
                16,
                72.0,
                405.0,
                420.0,
                415.0,
                "∗Professor of Law, Harvard Law School.",
            ),
            test_line(
                0,
                17,
                72.0,
                392.0,
                540.0,
                402.0,
                "helpful comments and conversations, we thank colleagues and workshop participants.",
            ),
            test_line(
                0,
                23,
                72.0,
                360.0,
                540.0,
                370.0,
                "1Title VI provides that no person shall be excluded on the ground of race.",
            ),
        ];
        for line in &mut lines {
            line.font_ratio_page_ref = 0.72;
        }

        assert!(starts_with_note_marker(&lines[0].text));
        assert!(starts_sequence_footnote_zone(&lines[0]));
        assert!(has_plausible_footnote_geometry(&lines[0]));

        mark_sequence_footnote_zones(&page, &mut lines);

        assert!(lines.iter().all(|line| line.sequence_footnote_zone));
        let tokens = feature_tokens(&lines[0]);
        assert_token_count(&tokens, "geom_first_page_symbol_author_note", 8);
    }

    #[test]
    fn next_line_context_marks_confirming_footnote_continuations() {
        let page = PageInfo::new(612.0, 792.0);
        let mut lines = vec![
            test_line(
                2,
                31,
                72.0,
                230.0,
                430.0,
                240.0,
                "promised to perform the obligation after the statute changed.",
            ),
            test_line(
                2,
                32,
                72.0,
                216.0,
                460.0,
                226.0,
                "18 See Brown v. Board, 347 U.S. 483, 488 (1954).",
            ),
        ];
        for line in &mut lines {
            line.font_ratio_page_ref = 0.78;
        }

        mark_sequence_footnote_zones(&page, &mut lines);
        mark_previous_line_context(&page, &mut lines);

        assert!(lines[0].next_sequence_footnote_zone);
        assert!(lines[0].next_legal_note_cue);
        let tokens = feature_tokens(&lines[0]);
        assert_token_count(&tokens, "next_sequence_footnote_zone", 5);
        assert_token_count(&tokens, "next_context_footnote_continuation", 7);
    }

    #[test]
    fn leading_id_and_ibid_are_short_form_note_cues() {
        assert!(contains_short_form_citation_cue(
            "Id. at 418-19, 224 S.E.2d at 344."
        ));
        assert!(contains_short_form_citation_cue("Ibid. at 122."));
        assert!(contains_short_form_citation_cue("'sIbid. (4th ed. 1942)."));
        assert!(contains_short_form_citation_cue("X\" Ibid., § 509.18."));
        assert!(contains_legal_note_cue("Id. at 418-19, 224 S.E.2d at 344."));
        assert!(contains_strong_legal_note_cue("Ibid. at 122."));
        assert!(!contains_short_form_citation_cue(
            "Identity is contested in the literature."
        ));
        assert!(!contains_short_form_citation_cue(
            "Siblings are discussed in the next section."
        ));
        assert!(!contains_legal_note_cue(
            "Identity is contested in the literature."
        ));
    }

    #[test]
    fn compact_numbered_list_items_are_not_note_markers() {
        assert!(!starts_with_note_marker(
            "3.Freshman experiences, including Freshman Interest Groups"
        ));
        assert!(!starts_with_note_marker(
            "8.Background information, including year began at UO"
        ));
        assert!(starts_with_note_marker(
            "3. See Brown v. Board, 347 U.S. 483."
        ));
        assert!(starts_with_note_marker(
            "3.See Brown v. Board, 347 U.S. 483."
        ));
        assert!(starts_with_note_marker(
            "3 See Brown v. Board, 347 U.S. 483."
        ));
    }

    #[test]
    fn previous_line_context_marks_footnote_continuations_without_labels() {
        let page = PageInfo::new(612.0, 792.0);
        let mut lines = vec![
            test_line(
                2,
                41,
                108.0,
                190.0,
                560.0,
                200.0,
                "33. See Peter Applebone, With Inmates at Record High, Sentence Policy is Reas-",
            ),
            test_line(
                2,
                42,
                72.0,
                176.0,
                430.0,
                186.0,
                "sessed, N.Y. TIMES, Apr. 25, 1988, at A1, C4.",
            ),
        ];
        for line in &mut lines {
            line.font_ratio_page_ref = 0.78;
        }

        mark_sequence_footnote_zones(&page, &mut lines);
        mark_previous_line_context(&page, &mut lines);

        assert!(lines[0].sequence_footnote_zone);
        assert!(lines[1].prev_line_present);
        assert!(lines[1].prev_sequence_footnote_zone);
        assert!(lines[1].prev_note_marker);
        let tokens = feature_tokens(&lines[1]);
        assert_token_count(&tokens, "prev_sequence_footnote_zone", 8);
        assert_token_count(&tokens, "prev_context_footnote_continuation", 8);
    }

    #[test]
    fn decoded_footnote_run_promotes_adjacent_continuation_only_when_unhinted() {
        let mut lines = vec![
            test_line(
                2,
                41,
                108.0,
                190.0,
                560.0,
                200.0,
                "33. See Peter Applebone, With Inmates at Record High, Sentence Policy is Reas-",
            ),
            test_line(
                2,
                42,
                72.0,
                176.0,
                430.0,
                186.0,
                "sessed, N.Y. TIMES, Apr. 25, 1988, at A1, C4.",
            ),
        ];
        for line in &mut lines {
            line.font_ratio_page_ref = 0.78;
        }
        let mut hints = vec![LiquidLayoutHint {
            text: lines[0].text.clone(),
            role: LiquidBlockRole::Marginalia,
        }];

        extend_decoded_footnote_run_hints(&mut hints, &lines);

        assert_eq!(
            hint_role_for_line(&hints, &lines[1]),
            Some(LiquidBlockRole::Marginalia)
        );
    }

    #[test]
    fn decoded_footnote_run_promotes_adjacent_numeric_citation_fragments() {
        let mut note = test_line(
            5,
            91,
            72.0,
            112.0,
            480.0,
            122.0,
            "See, e.g., Martin Kaste, Prison Population Continues to Grow,",
        );
        let mut numeric = test_line(5, 92, 96.0, 108.0, 116.0, 118.0, "393");
        let mut punctuated = test_line(5, 93, 96.0, 108.0, 112.0, 118.0, "13,");
        let mut page_number = test_line(5, 94, 96.0, 108.0, 126.0, 118.0, "2256");
        let distant = test_line(5, 95, 96.0, 320.0, 116.0, 330.0, "393");
        for line in [&mut note, &mut numeric, &mut punctuated, &mut page_number] {
            line.font_ratio_page_ref = 0.76;
        }
        let hints = vec![LiquidLayoutHint {
            text: note.text.clone(),
            role: LiquidBlockRole::Marginalia,
        }];

        assert!(is_plain_page_number_line(&numeric.text));
        assert!(!is_plain_page_number_line(&punctuated.text));
        assert!(should_decode_keep_as_marginalia(
            &hints,
            &numeric,
            Some(&note),
            None,
        ));
        assert!(should_decode_keep_as_marginalia(
            &hints,
            &punctuated,
            Some(&note),
            None,
        ));
        assert!(!should_decode_keep_as_marginalia(
            &hints,
            &page_number,
            Some(&note),
            None,
        ));
        assert!(!should_decode_keep_as_marginalia(
            &hints,
            &distant,
            Some(&note),
            None,
        ));
    }

    #[test]
    fn decoded_footnote_run_uses_nearby_window_for_numeric_fragments() {
        let mut lines = vec![
            test_line(
                5,
                91,
                72.0,
                112.0,
                480.0,
                122.0,
                "See, e.g., Martin Kaste, Prison Population Continues to Grow,",
            ),
            test_line(5, 92, 260.0, 110.0, 276.0, 120.0, "A1"),
            test_line(5, 93, 96.0, 108.0, 116.0, 118.0, "393"),
        ];
        for line in &mut lines {
            line.font_ratio_page_ref = 0.76;
        }
        let mut hints = vec![LiquidLayoutHint {
            text: lines[0].text.clone(),
            role: LiquidBlockRole::Marginalia,
        }];

        assert!(!should_decode_keep_as_marginalia(
            &hints,
            &lines[2],
            Some(&lines[1]),
            None,
        ));

        extend_decoded_footnote_run_hints(&mut hints, &lines);

        assert_eq!(
            hint_role_for_line(&hints, &lines[2]),
            Some(LiquidBlockRole::Marginalia)
        );
    }

    #[test]
    fn dense_marginalia_run_recovers_tiny_numeric_fragments() {
        let mut lines = vec![
            test_line(5, 100, 72.0, 108.0, 220.0, 118.0, "18 F. Supp. 62"),
            test_line(5, 101, 236.0, 107.0, 372.0, 117.0, "(N. D. Ga. 1937),"),
            test_line(5, 102, 388.0, 106.0, 492.0, 116.0, "rev'd,"),
            test_line(5, 103, 72.0, 92.0, 82.0, 102.0, "81"),
            test_line(5, 104, 190.0, 84.0, 238.0, 94.0, "a finding"),
        ];
        for line in &mut lines {
            line.font_ratio_page_ref = 0.76;
        }
        lines[3].font_ratio_page_ref = 0.67;
        let mut hints = vec![
            LiquidLayoutHint {
                text: lines[0].text.clone(),
                role: LiquidBlockRole::Marginalia,
            },
            LiquidLayoutHint {
                text: lines[1].text.clone(),
                role: LiquidBlockRole::Marginalia,
            },
            LiquidLayoutHint {
                text: lines[2].text.clone(),
                role: LiquidBlockRole::Marginalia,
            },
            LiquidLayoutHint {
                text: lines[4].text.clone(),
                role: LiquidBlockRole::Marginalia,
            },
        ];

        extend_decoded_footnote_run_hints(&mut hints, &lines);

        assert_eq!(
            hint_role_for_line(&hints, &lines[3]),
            Some(LiquidBlockRole::Marginalia)
        );
    }

    #[test]
    fn dense_citation_prelude_row_recovers_tableish_name_fragments() {
        let mut lines = vec![
            test_line(5, 43, 42.0, 190.0, 114.0, 200.0, "!Compare"),
            test_line(5, 44, 124.0, 190.0, 230.0, 200.0, "Westor Theatres,"),
            test_line(5, 45, 241.0, 190.0, 263.0, 200.0, "Inc."),
            test_line(5, 46, 273.0, 190.0, 284.0, 200.0, "v."),
            test_line(5, 47, 294.0, 190.0, 338.0, 200.0, "Warner"),
            test_line(5, 48, 349.0, 190.0, 379.0, 200.0, "Bros."),
            test_line(5, 49, 389.0, 190.0, 439.0, 200.0, "Pictures,"),
            test_line(5, 50, 449.0, 190.0, 473.0, 200.0, "Inc.,"),
            test_line(5, 51, 484.0, 190.0, 497.0, 200.0, "41"),
            test_line(5, 52, 507.0, 190.0, 519.0, 200.0, "F."),
            test_line(5, 53, 528.0, 190.0, 560.0, 200.0, "Supp."),
            test_line(5, 54, 570.0, 190.0, 588.0, 200.0, "757"),
            test_line(5, 55, 42.0, 176.0, 126.0, 186.0, "(D.N.J. 1941)"),
            test_line(5, 56, 134.0, 176.0, 205.0, 186.0, "with Giusti"),
            test_line(5, 58, 404.0, 176.0, 484.0, 186.0, "156 F.2d 351"),
        ];
        for line in &mut lines {
            line.font_ratio_page_ref = 0.78;
        }
        for line in &mut lines[12..] {
            line.sequence_footnote_zone = true;
        }

        assert!(dense_citation_prelude_row_above_footnote_sequence(
            &lines[4], &lines
        ));

        let mut hints = vec![LiquidLayoutHint {
            text: lines[12].text.clone(),
            role: LiquidBlockRole::Marginalia,
        }];
        extend_decoded_footnote_run_hints(&mut hints, &lines);

        assert_eq!(
            hint_role_for_line(&hints, &lines[4]),
            Some(LiquidBlockRole::Marginalia)
        );
        assert_eq!(
            hint_role_for_line(&hints, &lines[11]),
            Some(LiquidBlockRole::Marginalia)
        );
    }

    #[test]
    fn sequence_citation_row_recovers_normal_font_case_fragments() {
        let mut lines = vec![
            test_line(
                4,
                15,
                164.0,
                480.0,
                476.0,
                490.0,
                "See, e.g., EnCana Corp. v. Ecuador, LCIA Case No. UN 3481, Award (Feb. 3, 2006),",
            ),
            test_line(
                4,
                16,
                139.0,
                468.0,
                476.0,
                478.0,
                "http://www.italaw.com/sites/default/files/case-documents/ita0285_0.pdf; Occidental Expl. & Prod.",
            ),
            test_line(4, 17, 139.0, 456.0, 153.0, 466.0, "Co."),
            test_line(4, 18, 161.0, 456.0, 170.0, 466.0, "v."),
            test_line(4, 19, 177.0, 456.0, 209.0, 466.0, "Ecuador,"),
            test_line(4, 20, 217.0, 456.0, 239.0, 466.0, "LCIA"),
            test_line(4, 21, 247.0, 456.0, 266.0, 466.0, "Case"),
            test_line(4, 22, 274.0, 456.0, 289.0, 466.0, "No."),
            test_line(4, 23, 296.0, 456.0, 310.0, 466.0, "UN"),
            test_line(4, 24, 318.0, 456.0, 340.0, 466.0, "3467,"),
            test_line(4, 25, 348.0, 456.0, 368.0, 466.0, "Final"),
            test_line(4, 26, 376.0, 456.0, 401.0, 466.0, "Award"),
        ];
        for line in &mut lines {
            line.sequence_footnote_zone = true;
            line.font_ratio_page_ref = 1.0;
        }

        assert!(sequence_citation_row_fragment_can_be_marginalia(
            &lines[5], &lines
        ));

        let mut hints = vec![LiquidLayoutHint {
            text: lines[0].text.clone(),
            role: LiquidBlockRole::Marginalia,
        }];
        extend_decoded_footnote_run_hints(&mut hints, &lines);

        assert_eq!(
            hint_role_for_line(&hints, &lines[5]),
            Some(LiquidBlockRole::Marginalia)
        );
    }

    #[test]
    fn midpage_numbered_note_run_recovers_lines_above_detected_sequence() {
        let mut lines = vec![
            test_line(
                7,
                13,
                82.0,
                516.0,
                492.0,
                526.0,
                "on an individual already convicted of a violation, other potential offenders will refrain from",
            ),
            test_line(
                7,
                16,
                92.0,
                482.0,
                494.0,
                492.0,
                "33. Specific deterrence means that if an individual defendant is subject to criminal sanc-",
            ),
            test_line(
                7,
                17,
                82.0,
                470.0,
                492.0,
                480.0,
                "tions, that particular individual will not commit subsequent violations after being released",
            ),
            test_line(
                7,
                18,
                82.0,
                458.0,
                205.0,
                468.0,
                "from prison. Id. at 619 n.1.",
            ),
            test_line(
                7,
                19,
                92.0,
                446.0,
                494.0,
                456.0,
                "34. Incapacitation refers to the idea that if an individual defendant is imprisoned, that",
            ),
            test_line(
                7,
                24,
                92.0,
                376.0,
                494.0,
                386.0,
                "35. Punishment encompasses the theory of retaliatory sanctions imposed by society for",
            ),
            test_line(
                7,
                25,
                82.0,
                364.0,
                274.0,
                374.0,
                "impermissible conduct. See supra note 31.",
            ),
            test_line(
                7,
                26,
                92.0,
                352.0,
                494.0,
                362.0,
                "36. Rehabilitation means that the imposition of criminal penalties will modify the atti-",
            ),
            test_line(
                7,
                27,
                82.0,
                340.0,
                492.0,
                350.0,
                "tude and behavior of the individual sentenced, and that individual will be restored to a",
            ),
        ];
        for line in &mut lines {
            line.font_ratio_page_ref = 0.99;
        }
        for line in &mut lines[6..] {
            line.sequence_footnote_zone = true;
        }

        assert!(midpage_numbered_note_run_above_sequence_can_be_marginalia(
            &lines[1], &lines
        ));
        assert!(midpage_numbered_note_run_above_sequence_can_be_marginalia(
            &lines[2], &lines
        ));
        assert!(!midpage_numbered_note_run_above_sequence_can_be_marginalia(
            &lines[0], &lines
        ));

        let mut hints = vec![LiquidLayoutHint {
            text: lines[6].text.clone(),
            role: LiquidBlockRole::Marginalia,
        }];
        extend_decoded_footnote_run_hints(&mut hints, &lines);

        assert_eq!(
            hint_role_for_line(&hints, &lines[1]),
            Some(LiquidBlockRole::Marginalia)
        );
        assert_eq!(
            hint_role_for_line(&hints, &lines[2]),
            Some(LiquidBlockRole::Marginalia)
        );
    }

    #[test]
    fn administrative_status_lines_do_not_become_marginalia() {
        for text in [
            "PASSED: 7:0",
            "ABSENT: Taylor",
            "Updated December 6, 2012",
            "Notification to the jurisdiction of an appeal by the deadline, this Plan Amendment is acknowledged.",
            "Grant Young, DLCD Regional Representative",
        ] {
            let mut line = test_line(4, 90, 72.0, 184.0, 154.0, 194.0, text);
            line.sequence_footnote_zone = true;
            line.font_ratio_page_ref = 0.82;

            assert!(looks_like_administrative_status_or_update_line(text));
            assert!(!should_decode_keep_as_marginalia(&[], &line, None, None));
            assert!(!footnote_specialist_line_can_be_marginalia(&line));
            assert!(!model_line_should_be_marginalia(&line));
        }

        assert!(!looks_like_administrative_status_or_update_line(
            "Updated discussion of legal doctrine"
        ));
        assert!(!looks_like_administrative_status_or_update_line(
            "4 See Regional Representatives Ass'n v. State, 123 U.S. 456 (2020)."
        ));
    }

    #[test]
    fn uncited_all_caps_topic_headings_do_not_become_marginalia() {
        let mut heading = test_line(
            1,
            26,
            96.0,
            360.0,
            410.0,
            370.0,
            "FEDERAL RULES OF CIVIL PROCEDURE UNDER",
        );
        heading.sequence_footnote_zone = true;
        heading.font_ratio_page_ref = 0.99;

        assert!(looks_like_uncited_all_caps_topic_heading(&heading.text));
        assert!(!should_decode_keep_as_marginalia(&[], &heading, None, None,));
        assert!(!model_line_should_be_marginalia(&heading));
        assert!(!footnote_specialist_line_can_be_marginalia(&heading));

        assert!(!looks_like_uncited_all_caps_topic_heading(
            "THE FEDERALIST NO. 32 (Alexander Hamilton)."
        ));
        assert!(!looks_like_uncited_all_caps_topic_heading(
            "HARVARD LAW REVIEW"
        ));
        assert!(!looks_like_uncited_all_caps_topic_heading(
            "HENRY MAINE, ANCIENT LAW: ITS CONNECTION TO HISTORY"
        ));
        assert!(!looks_like_uncited_all_caps_topic_heading("STATE v. HUNT"));

        let mut small_note_fragment = heading.clone();
        small_note_fragment.text = "OF WORKING GROUP".to_owned();
        small_note_fragment.font_ratio_page_ref = 0.64;
        assert!(looks_like_uncited_all_caps_topic_heading(
            &small_note_fragment.text
        ));
        assert!(should_decode_keep_as_marginalia(
            &[],
            &small_note_fragment,
            None,
            None,
        ));
    }

    #[test]
    fn sequence_zone_concrete_citation_fragments_decode_as_marginalia() {
        let mut year_fragment = test_line(
            2,
            89,
            72.0,
            184.0,
            328.0,
            194.0,
            "and Municipal Officers and Employees (1930)",
        );
        let mut case_fragment =
            test_line(2, 99, 72.0, 158.0, 192.0, 168.0, "v. Baltic Mining Co.,");
        let mut pincite = test_line(2, 106, 72.0, 142.0, 92.0, 152.0, "165,");
        let mut sequence_fragment = test_line(4, 89, 72.0, 184.0, 148.0, 194.0, "44 seq., 69");
        let mut broad_noise = test_line(4, 90, 72.0, 184.0, 154.0, 194.0, "ABSENT: Taylor");
        for line in [
            &mut year_fragment,
            &mut case_fragment,
            &mut pincite,
            &mut sequence_fragment,
            &mut broad_noise,
        ] {
            line.sequence_footnote_zone = true;
            line.font_ratio_page_ref = 0.94;
        }
        sequence_fragment.font_ratio_page_ref = 0.74;

        for line in [&year_fragment, &case_fragment, &pincite, &sequence_fragment] {
            assert!(should_decode_keep_as_marginalia(&[], line, None, None));
        }
        assert!(!concrete_sequence_citation_fragment_can_be_marginalia(
            &broad_noise
        ));
    }

    #[test]
    fn small_font_previous_sequence_continuation_decodes_as_marginalia() {
        let mut continuation = test_line(
            4,
            35,
            124.0,
            302.0,
            336.0,
            312.0,
            "of an individual's capacities or propensities.",
        );
        continuation.prev_sequence_footnote_zone = true;
        continuation.prev_y_gap_ratio = 0.0107;
        continuation.prev_left_delta_ratio = 0.0;
        continuation.font_ratio_page_ref = 0.81;

        assert!(previous_sequence_small_font_continuation_can_be_marginalia(
            &continuation
        ));
    }

    #[test]
    fn previous_sequence_body_lead_in_does_not_decode_as_marginalia() {
        let mut body = test_line(
            7,
            32,
            109.0,
            302.0,
            323.0,
            312.0,
            "A peremptory writ of mandamus must be filed:",
        );
        body.prev_sequence_footnote_zone = true;
        body.prev_y_gap_ratio = 0.0131;
        body.prev_left_delta_ratio = 0.0317;
        body.font_ratio_page_ref = 0.787;

        assert!(!previous_sequence_small_font_continuation_can_be_marginalia(&body));

        body.text = "5.".to_owned();
        assert!(!previous_sequence_small_font_continuation_can_be_marginalia(&body));
    }

    #[test]
    fn sequence_zone_lowercase_publication_note_start_decodes_as_marginalia() {
        let mut note = test_line(
            3,
            29,
            56.0,
            304.0,
            558.0,
            314.0,
            "2 reasons why the drought in California won't open the door to Great Lakes water, MICH. RADIO, Apr.",
        );
        note.sequence_footnote_zone = true;
        note.font_ratio_page_ref = 0.78;

        assert!(starts_with_numeric_lowercase_body_fragment(&note.text));
        assert!(numeric_lowercase_publication_note_start_can_be_marginalia(
            &note
        ));
        assert!(should_decode_keep_as_marginalia(&[], &note, None, None));

        let mut bodyish = note.clone();
        bodyish.text =
            "2 reasons why the drought did not change the parties' obligations.".to_owned();
        assert!(!numeric_lowercase_publication_note_start_can_be_marginalia(
            &bodyish
        ));
    }

    #[test]
    fn numeric_note_fragments_with_footnote_geometry_survive_body_fragment_guard() {
        let mut note = test_line(
            18,
            44,
            72.0,
            248.0,
            362.0,
            258.0,
            "10 (arguing that software licenses are federal law-enabled servitudes on chattels).",
        );
        note.sequence_footnote_zone = true;
        note.font_ratio_page_ref = 0.73;

        assert!(starts_with_note_marker(&note.text));
        assert!(starts_with_numeric_lowercase_body_fragment(&note.text));
        assert!(numeric_note_fragment_geometry_can_be_marginalia(&note));
        assert!(footnote_specialist_line_can_be_marginalia(&note));
        assert!(model_line_should_be_marginalia(&note));
        assert!(should_decode_keep_as_marginalia(&[], &note, None, None));

        let mut bodyish = note.clone();
        bodyish.font_ratio_page_ref = 0.96;
        assert!(!numeric_note_fragment_geometry_can_be_marginalia(&bodyish));
        assert!(!footnote_specialist_line_can_be_marginalia(&bodyish));
        assert!(!model_line_should_be_marginalia(&bodyish));

        let mut below_divider = test_line(7, 32, 72.0, 290.0, 112.0, 300.0, "69. ld.");
        below_divider.below_footnote_divider = true;
        below_divider.font_ratio_page_ref = 0.82;

        assert!(starts_with_numeric_lowercase_body_fragment(
            &below_divider.text
        ));
        assert!(numeric_note_fragment_geometry_can_be_marginalia(
            &below_divider
        ));
        assert!(footnote_specialist_line_can_be_marginalia(&below_divider));
        assert!(model_line_should_be_marginalia(&below_divider));
    }

    #[test]
    fn small_font_sequence_citation_material_survives_heading_and_body_guards() {
        let mut statutory = test_line(
            2,
            35,
            72.0,
            280.0,
            452.0,
            290.0,
            "ILL. REV. STAT. ch. 48, para. 138.5(b) (1987). Paragraph 138.5(b) provides in",
        );
        let mut treatise = test_line(
            1,
            26,
            72.0,
            338.0,
            276.0,
            348.0,
            "54.77[2] (2d ed. 1980); 10 C. WRIGHT & A.",
        );
        for line in [&mut statutory, &mut treatise] {
            line.sequence_footnote_zone = true;
            line.font_ratio_page_ref = 0.74;
        }

        assert!(looks_like_clear_section_heading(&statutory.text));
        assert!(starts_with_numeric_lowercase_body_fragment(&treatise.text));
        for line in [&statutory, &treatise] {
            assert!(small_font_sequence_citation_material_can_be_marginalia(
                line
            ));
            assert!(footnote_specialist_line_can_be_marginalia(line));
            assert!(model_line_should_be_marginalia(line));
            assert!(should_decode_keep_as_marginalia(&[], line, None, None));
        }

        let mut body = test_line(
            2,
            36,
            72.0,
            280.0,
            452.0,
            290.0,
            "ILL. REV. STAT. title for a table of contents entry",
        );
        body.sequence_footnote_zone = true;
        body.font_ratio_page_ref = 1.0;
        assert!(!small_font_sequence_citation_material_can_be_marginalia(
            &body
        ));
    }

    #[test]
    fn edge_case_name_running_headers_are_noise_not_marginalia() {
        let mut header = test_line(
            5,
            0,
            72.0,
            732.0,
            212.0,
            742.0,
            "Costello v. Capital Cities",
        );
        header.font_ratio_page_ref = 0.96;

        assert!(looks_like_edge_case_name_running_header_fragment(&header));
        assert!(looks_like_edge_running_header_footer_fragment(&header));
        assert!(!footnote_specialist_line_can_be_marginalia(&header));
        assert!(!model_line_should_be_marginalia(&header));

        let mut footnote = test_line(
            5,
            88,
            72.0,
            64.0,
            256.0,
            74.0,
            "952, 40 Cal. Rptr. 264 (1964).",
        );
        footnote.font_ratio_page_ref = 0.72;
        footnote.sequence_footnote_zone = true;

        assert!(!looks_like_edge_case_name_running_header_fragment(
            &footnote
        ));
        assert!(footnote_specialist_line_can_be_marginalia(&footnote));
    }

    #[test]
    fn numeric_year_parenthetical_continuation_decodes_inside_marginalia_run() {
        let mut prev = test_line(
            22,
            44,
            72.0,
            190.0,
            540.0,
            200.0,
            "140 S. Ct. 1731, 1740 (2020). But cf. Thomas v. Eastman Kodak Co., 183 F.3d 38, 57-61 (1st Cir.",
        );
        let mut line = test_line(
            22,
            45,
            72.0,
            178.0,
            190.0,
            188.0,
            "1999) (suggesting a broader view).",
        );
        let mut next = test_line(
            22,
            46,
            72.0,
            166.0,
            540.0,
            176.0,
            "138The issue has much the same structure as the challenge of disentangling race and politics.",
        );
        for item in [&mut prev, &mut line, &mut next] {
            item.sequence_footnote_zone = true;
            item.font_ratio_page_ref = 0.72;
        }
        let hints = vec![
            LiquidLayoutHint {
                text: prev.text.clone(),
                role: LiquidBlockRole::Marginalia,
            },
            LiquidLayoutHint {
                text: next.text.clone(),
                role: LiquidBlockRole::Marginalia,
            },
        ];

        assert!(starts_with_note_marker(&line.text));
        assert!(starts_with_numeric_lowercase_body_fragment(&line.text));
        assert!(looks_like_numeric_year_parenthetical_continuation(
            &line.text
        ));
        assert!(numeric_year_parenthetical_continuation_can_be_marginalia(
            &hints,
            &line,
            Some(&prev),
            Some(&next),
        ));
        assert!(should_decode_keep_as_marginalia(
            &hints,
            &line,
            Some(&prev),
            Some(&next),
        ));
    }

    #[test]
    fn numeric_page_parenthetical_citation_continuation_decodes_inside_marginalia_run() {
        let mut prev = test_line(
            9,
            81,
            72.0,
            260.0,
            540.0,
            270.0,
            "142See U.C.C. section 2-302, cmt. 1; Kessler, Contracts of Adhesion, 43 COLUM. L. REV.",
        );
        let mut line = test_line(
            9,
            82,
            72.0,
            248.0,
            520.0,
            258.0,
            "43 (1983) (explaining how contracts of adhesion confer advantages on drafting parties).",
        );
        let mut next = test_line(
            9,
            83,
            72.0,
            236.0,
            540.0,
            246.0,
            "Courts have used unconscionability doctrine to police similar bargaining problems.",
        );
        for item in [&mut prev, &mut line, &mut next] {
            item.sequence_footnote_zone = true;
            item.font_ratio_page_ref = 0.72;
        }
        let hints = vec![
            LiquidLayoutHint {
                text: prev.text.clone(),
                role: LiquidBlockRole::Marginalia,
            },
            LiquidLayoutHint {
                text: next.text.clone(),
                role: LiquidBlockRole::Marginalia,
            },
        ];

        assert!(starts_with_note_marker(&line.text));
        assert!(starts_with_numeric_lowercase_body_fragment(&line.text));
        assert!(!looks_like_numeric_year_parenthetical_continuation(
            &line.text
        ));
        assert!(looks_like_numeric_page_parenthetical_citation_continuation(
            &line.text
        ));
        assert!(
            numeric_page_parenthetical_citation_continuation_can_be_marginalia(
                &hints,
                &line,
                Some(&prev),
                Some(&next),
            )
        );
        assert!(should_decode_keep_as_marginalia(
            &hints,
            &line,
            Some(&prev),
            Some(&next),
        ));
    }

    #[test]
    fn contents_like_author_and_tiny_citation_fragments_can_be_marginalia() {
        let mut credential = test_line(
            1,
            47,
            54.0,
            210.0,
            464.0,
            220.0,
            "cinnati; J.D., 1980, Georgetown University Law Center; L.L.M., 1985, DePaul Univer-",
        );
        credential.page_contents_like = true;
        credential.font_ratio_page_ref = 0.75;

        let mut tiny = test_line(1, 48, 380.0, 224.0, 393.0, 234.0, "13,");
        tiny.page_contents_like = true;
        tiny.font_ratio_page_ref = 0.77;

        let mut contents_entry = test_line(
            1,
            49,
            72.0,
            224.0,
            420.0,
            234.0,
            "Article Title ........ 13",
        );
        contents_entry.page_contents_like = true;
        contents_entry.contents_or_index_entry = true;
        contents_entry.font_ratio_page_ref = 0.77;

        assert!(should_decode_keep_as_marginalia(
            &[],
            &credential,
            None,
            None
        ));
        assert!(should_decode_keep_as_marginalia(&[], &tiny, None, None));
        assert!(!contents_like_author_credential_continuation_can_be_marginalia(&contents_entry));
        assert!(!contents_like_tiny_citation_fragment_can_be_marginalia(
            &contents_entry
        ));
    }

    #[test]
    fn decoded_footnote_run_does_not_override_existing_noise_hint() {
        let mut lines = vec![
            test_line(
                0,
                13,
                72.0,
                190.0,
                560.0,
                200.0,
                "1. See Smith v. Jones, 123 U.S. 456 (1901).",
            ),
            test_line(
                0,
                14,
                72.0,
                176.0,
                560.0,
                186.0,
                "Available at: https://scholarship.law.example/lawreview/vol1/iss1/1",
            ),
        ];
        for line in &mut lines {
            line.font_ratio_page_ref = 0.78;
        }
        let mut hints = vec![
            LiquidLayoutHint {
                text: lines[0].text.clone(),
                role: LiquidBlockRole::Marginalia,
            },
            LiquidLayoutHint {
                text: lines[1].text.clone(),
                role: LiquidBlockRole::Noise,
            },
        ];

        extend_decoded_footnote_run_hints(&mut hints, &lines);

        assert_eq!(
            hint_role_for_line(&hints, &lines[1]),
            Some(LiquidBlockRole::Noise)
        );
    }

    #[test]
    fn decoded_footnote_run_can_upgrade_footer_hint_to_marginalia() {
        let mut lines = vec![
            test_line(3, 24, 72.0, 358.0, 92.0, 368.0, "9."),
            test_line(
                3,
                25,
                72.0,
                344.0,
                520.0,
                354.0,
                "Id. Previously in Georgia, all covenants within an agreement were unenforceable.",
            ),
        ];
        for line in &mut lines {
            line.font_ratio_page_ref = 0.78;
        }
        let mut hints = vec![LiquidLayoutHint {
            text: lines[0].text.clone(),
            role: LiquidBlockRole::Footer,
        }];

        extend_decoded_footnote_run_hints(&mut hints, &lines);

        assert_eq!(
            hint_role_for_line(&hints, &lines[0]),
            Some(LiquidBlockRole::Marginalia)
        );
    }

    #[test]
    fn repository_and_ssrn_boilerplate_do_not_become_marginalia() {
        let repository = test_line(
            0,
            10,
            72.0,
            90.0,
            540.0,
            100.0,
            "This Comment is brought to you for free and open access by the Journals at Santa Clara Law Digital Commons.",
        );
        assert!(is_repository_cover_boilerplate(&repository));
        assert!(!footnote_specialist_line_can_be_marginalia(&repository));
        assert!(!model_line_should_be_marginalia(&repository));

        let ssrn = test_line(
            0,
            41,
            72.0,
            14.0,
            540.0,
            24.0,
            "Electronic copy available at: https://ssrn.com/abstract=3912101",
        );
        assert!(is_repository_cover_boilerplate(&ssrn));
        assert!(!footnote_specialist_line_can_be_marginalia(&ssrn));
        assert!(!model_line_should_be_marginalia(&ssrn));

        let contact = test_line(
            0,
            16,
            72.0,
            92.0,
            380.0,
            102.0,
            "please contact digres@mailbox.sc.edu.",
        );
        assert!(is_repository_cover_boilerplate(&contact));
        assert!(!footnote_specialist_line_can_be_marginalia(&contact));
        assert!(!model_line_should_be_marginalia(&contact));

        let taxonomy = test_line(
            0,
            17,
            72.0,
            112.0,
            420.0,
            122.0,
            "Part of the Legal Education Commons",
        );
        assert!(is_repository_cover_boilerplate(&taxonomy));
        assert!(!footnote_specialist_line_can_be_marginalia(&taxonomy));
        assert!(!model_line_should_be_marginalia(&taxonomy));
    }

    #[test]
    fn plain_page_numbers_near_footnotes_do_not_become_marginalia() {
        let page = PageInfo::with_footnote_divider_y_from_top(612.0, 792.0, Some(500.0));
        let mut chars = Vec::new();
        push_line(&mut chars, &page, 120.0, 11.0, "Example Article Title");
        push_line(
            &mut chars,
            &page,
            148.0,
            11.0,
            "Ordinary body text establishes the larger reference font.",
        );
        push_line(&mut chars, &page, 520.0, 8.0, "2256");
        push_line(
            &mut chars,
            &page,
            536.0,
            8.0,
            "1 See Example v. State, 123 U.S. 456 (2020).",
        );

        let mut page_number = test_line(0, 3, 72.0, 264.0, 96.0, 272.0, "2256");
        page_number.below_footnote_divider = true;
        page_number.font_ratio_page_ref = 0.82;
        assert!(!footnote_specialist_line_can_be_marginalia(&page_number));
        assert!(!model_line_should_be_marginalia(&page_number));

        let hints = layout_hints_for_pages(&[page], &[Some(chars)]);
        assert!(!hints.iter().any(|hint| {
            hint.role == LiquidBlockRole::Marginalia && hint.text.trim() == "2256"
        }));
        assert!(hints.iter().any(|hint| {
            hint.role == LiquidBlockRole::Marginalia && hint.text.starts_with("1 See Example")
        }));
    }

    #[test]
    fn bare_numeric_footnote_markers_are_not_hidden_as_page_noise() {
        let page = PageInfo::with_footnote_divider_y_from_top(612.0, 792.0, Some(500.0));
        let mut marker = test_line(4, 61, 72.0, 88.0, 90.0, 96.0, "35");
        marker.below_footnote_divider = true;
        marker.font_ratio_page_ref = 0.60;

        assert!(is_plain_page_number_line(&marker.text));
        assert!(is_plain_numeric_footnote_marker_candidate(&marker));
        assert!(!model_noise_line_can_be_hidden(&page, &marker));
        assert!(footnote_specialist_line_can_be_marginalia(&marker));

        let mut page_number = test_line(4, 62, 72.0, 88.0, 96.0, 96.0, "2256");
        page_number.below_footnote_divider = true;
        page_number.font_ratio_page_ref = 0.60;
        assert!(!is_plain_numeric_footnote_marker_candidate(&page_number));
        assert!(!footnote_specialist_line_can_be_marginalia(&page_number));
    }

    #[test]
    fn sequence_zone_body_enumeration_markers_do_not_become_marginalia() {
        let mut body_marker = test_line(5, 14, 60.0, 570.0, 72.0, 580.0, "3.");
        body_marker.sequence_footnote_zone = true;
        body_marker.font_ratio_page_ref = 0.90;

        assert!(is_bare_numeric_note_marker(&body_marker.text));
        assert!(sequence_zone_body_enumeration_marker(&body_marker));
        assert!(!footnote_specialist_line_can_be_marginalia(&body_marker));
        assert!(!model_line_should_be_marginalia(&body_marker));
        assert!(!sequence_zone_numeric_marker_in_note_run_can_be_marginalia(
            &body_marker,
            None,
            None
        ));

        let mut true_marker = test_line(5, 15, 60.0, 210.0, 72.0, 220.0, "9.");
        let mut note_text = test_line(
            5,
            16,
            60.0,
            196.0,
            420.0,
            206.0,
            "See Example v. State, 123 U.S. 456 (2020).",
        );
        for line in [&mut true_marker, &mut note_text] {
            line.sequence_footnote_zone = true;
            line.font_ratio_page_ref = 0.78;
        }
        assert!(!sequence_zone_body_enumeration_marker(&true_marker));
        assert!(sequence_zone_numeric_marker_in_note_run_can_be_marginalia(
            &true_marker,
            None,
            Some(&note_text),
        ));
        assert!(should_decode_keep_as_marginalia(
            &[],
            &true_marker,
            None,
            Some(&note_text),
        ));

        let mut enum_fragment =
            test_line(5, 17, 60.0, 485.0, 178.0, 495.0, "3) taken for the city");
        let mut prev_fragment =
            test_line(5, 16, 60.0, 475.0, 210.0, 485.0, "2) unconstitutional act");
        let mut next_fragment =
            test_line(5, 18, 60.0, 495.0, 190.0, 505.0, "4) by a city official");
        for line in [&mut enum_fragment, &mut prev_fragment, &mut next_fragment] {
            line.sequence_footnote_zone = true;
            line.font_ratio_page_ref = 0.86;
        }
        assert!(starts_with_numeric_lowercase_body_fragment(
            &enum_fragment.text
        ));
        assert!(sequence_zone_numeric_marker_in_note_run_can_be_marginalia(
            &enum_fragment,
            Some(&prev_fragment),
            Some(&next_fragment),
        ));
        assert!(should_decode_keep_as_marginalia(
            &[],
            &enum_fragment,
            Some(&prev_fragment),
            Some(&next_fragment),
        ));

        let mut parenthetical_fragment = test_line(
            5,
            19,
            60.0,
            382.0,
            452.0,
            392.0,
            "35 (describing the probable cause requirement as it relates to Fourth Amendment jurisprudence).",
        );
        let mut previous_citation = test_line(
            5,
            18,
            60.0,
            372.0,
            440.0,
            382.0,
            "34. Nat'l Treasury Employees Union v. Von Raab, 489 U.S. 656 (1989).",
        );
        let mut next_citation = test_line(
            5,
            20,
            68.0,
            392.0,
            430.0,
            402.0,
            "Katz v. United States, 389 U.S. 347 (1967).",
        );
        for line in [
            &mut parenthetical_fragment,
            &mut previous_citation,
            &mut next_citation,
        ] {
            line.sequence_footnote_zone = true;
            line.font_ratio_page_ref = 0.74;
        }
        assert!(starts_with_numeric_lowercase_body_fragment(
            &parenthetical_fragment.text
        ));
        assert!(sequence_zone_numeric_marker_in_note_run_can_be_marginalia(
            &parenthetical_fragment,
            Some(&previous_citation),
            Some(&next_citation),
        ));
        assert!(should_decode_keep_as_marginalia(
            &[],
            &parenthetical_fragment,
            Some(&previous_citation),
            Some(&next_citation),
        ));

        let mut numeric_citation = test_line(5, 21, 60.0, 330.0, 208.0, 340.0, "2.44, at 43.");
        let mut citation_lead = test_line(
            5,
            20,
            60.0,
            318.0,
            420.0,
            328.0,
            "See Johnson v. Transworld Airlines, Inc., 149 Cal. App. 3d 518, 528.",
        );
        for line in [&mut numeric_citation, &mut citation_lead] {
            line.sequence_footnote_zone = true;
            line.font_ratio_page_ref = 0.70;
        }
        assert!(looks_like_numeric_citation_fragment(&numeric_citation.text));
        assert!(numeric_citation_fragment_in_note_run_can_be_marginalia(
            &numeric_citation,
            Some(&citation_lead),
            None,
        ));
        assert!(should_decode_keep_as_marginalia(
            &[],
            &numeric_citation,
            Some(&citation_lead),
            None,
        ));

        let mut author_title = test_line(
            5,
            22,
            60.0,
            210.0,
            430.0,
            220.0,
            "M. Kaplan, Cognitive Processes in the Individual Juror, in THE PSYCHOLOGY OF THE",
        );
        let mut note_marker = test_line(5, 21, 60.0, 200.0, 78.0, 210.0, "19.");
        for line in [&mut author_title, &mut note_marker] {
            line.sequence_footnote_zone = true;
            line.font_ratio_page_ref = 0.81;
        }
        assert!(small_font_note_run_continuation_can_be_marginalia(
            &author_title,
            Some(&note_marker),
            None,
        ));
        assert!(should_decode_keep_as_marginalia(
            &[],
            &author_title,
            Some(&note_marker),
            None,
        ));

        let mut acknowledgement = test_line(
            0,
            31,
            60.0,
            202.0,
            455.0,
            212.0,
            "I would like to mention: Georgetown University Law Center, Critical Perspectives on Law",
        );
        let mut acknowledgement_prev = test_line(
            0,
            30,
            60.0,
            192.0,
            430.0,
            202.0,
            "to the organizers and participants for the opportunity and for their comments.",
        );
        for line in [&mut acknowledgement, &mut acknowledgement_prev] {
            line.below_footnote_divider = true;
            line.page_contents_like = true;
            line.font_ratio_page_ref = 0.78;
        }
        assert!(below_divider_small_font_continuation_can_be_marginalia(
            &acknowledgement,
            Some(&acknowledgement_prev),
            None,
        ));
        assert!(should_decode_keep_as_marginalia(
            &[],
            &acknowledgement,
            Some(&acknowledgement_prev),
            None,
        ));
    }

    #[test]
    fn normal_font_top_half_sequence_body_lines_do_not_become_marginalia() {
        let mut body_citation = test_line(
            5,
            27,
            45.0,
            394.0,
            485.0,
            404.0,
            "Carter v. Harper, 182 Wis. 148, where a milk depot was driven out",
        );
        body_citation.sequence_footnote_zone = true;
        body_citation.font_ratio_page_ref = 1.0;

        let mut true_note = test_line(
            5,
            28,
            45.0,
            380.0,
            357.0,
            390.0,
            "See, e.g., EnCana Corp. v. Ecuador, LCIA Case No. UN 3481, Award (Feb. 3, 2006),",
        );
        true_note.sequence_footnote_zone = true;
        true_note.font_ratio_page_ref = 1.0;

        assert!(normal_font_top_half_sequence_body_line(&body_citation));
        assert!(!model_line_should_be_marginalia(&body_citation));
        assert!(!should_decode_keep_as_marginalia(
            &[],
            &body_citation,
            None,
            None,
        ));

        assert!(!normal_font_top_half_sequence_body_line(&true_note));
        assert!(should_decode_keep_as_marginalia(
            &[],
            &true_note,
            None,
            None,
        ));
    }

    #[test]
    fn normal_font_inline_body_citations_do_not_become_marginalia() {
        let mut body_citation = test_line(
            4,
            30,
            72.0,
            270.0,
            492.0,
            280.0,
            "of the donee's possession. Owsley v. Owsley, 117 Ky. 47, 77 S.",
        );
        body_citation.font_ratio_page_ref = 0.98;
        body_citation.width_to_body_ratio = 0.69;
        body_citation.prev_line_present = true;
        body_citation.next_line_present = true;
        body_citation.prev_y_gap_ratio = 0.019;
        body_citation.next_y_gap_ratio = 0.019;
        body_citation.prev_left_delta_ratio = 0.002;
        body_citation.next_left_delta_ratio = 0.002;

        assert!(looks_like_citation_continuation_text(&body_citation.text));
        assert!(normal_font_inline_body_citation_continuation(
            &body_citation
        ));
        assert!(!model_line_should_be_marginalia(&body_citation));
        assert!(!footnote_specialist_line_can_be_marginalia(&body_citation));
        assert!(!should_decode_keep_as_marginalia(
            &[],
            &body_citation,
            None,
            None,
        ));

        let mut note = body_citation.clone();
        note.text = "See Owsley v. Owsley, 117 Ky. 47, 77 S.W. 397 (1903).".to_owned();
        assert!(!normal_font_inline_body_citation_continuation(&note));
        assert!(model_line_should_be_marginalia(&note));
    }

    #[test]
    fn old_law_review_topic_titles_do_not_become_marginalia() {
        let mut title = test_line(
            1,
            9,
            86.6,
            538.3,
            481.8,
            548.3,
            "CONSTITUTIONAL LAW-LEGISLATIVE POWERS-IMPAIRMENT OF",
        );
        title.sequence_footnote_zone = true;
        title.font_ratio_page_ref = 0.816;

        assert!(old_law_review_topic_heading_like_body_line(&title));
        assert!(line_should_never_be_marginalia_by_body_geometry(&title));
        assert!(!model_line_should_be_marginalia(&title));
        assert!(!footnote_specialist_line_can_be_marginalia(&title));
        assert!(!should_decode_keep_as_marginalia(&[], &title, None, None));

        let mut note = title.clone();
        note.text = "1. See generally Home Bldg. & Loan Ass'n v. Blaisdell.".to_owned();
        assert!(!old_law_review_topic_heading_like_body_line(&note));
        assert!(model_line_should_be_marginalia(&note));
    }

    #[test]
    fn normal_font_body_citation_continuation_without_sequence_is_not_marginalia() {
        let mut body_citation = test_line(
            4,
            30,
            54.0,
            270.3,
            475.4,
            280.3,
            "of the donee's possession. Owsley v. Owsley, 117 Ky. 47, 77 S.",
        );
        body_citation.font_ratio_page_ref = 0.9802;

        assert!(normal_font_body_citation_continuation_like_body_line(
            &body_citation
        ));
        assert!(line_should_never_be_marginalia_by_body_geometry(
            &body_citation
        ));
        assert!(!model_line_should_be_marginalia(&body_citation));
        assert!(!footnote_specialist_line_can_be_marginalia(&body_citation));
        assert!(!should_decode_keep_as_marginalia(
            &[],
            &body_citation,
            None,
            None,
        ));

        let mut note = body_citation.clone();
        note.text = "See Owsley v. Owsley, 117 Ky. 47, 77 S.W. 397 (1903).".to_owned();
        assert!(!normal_font_body_citation_continuation_like_body_line(
            &note
        ));
        assert!(model_line_should_be_marginalia(&note));
    }

    #[test]
    fn sequence_zone_quoted_body_excerpt_does_not_decode_as_marginalia() {
        let mut quoted_citation = test_line(
            4,
            52,
            71.8,
            341.9,
            241.2,
            351.9,
            "\"\"292 U. S. 360, 368-9 (1934).",
        );
        quoted_citation.sequence_footnote_zone = true;
        quoted_citation.font_ratio_page_ref = 0.8108;

        let mut quoted_prose = test_line(
            4,
            53,
            71.5,
            329.8,
            572.4,
            339.8,
            "\"Prior to the enactment of the Revenue Act of 1926, the Treasury made this distinction,",
        );
        quoted_prose.sequence_footnote_zone = true;
        quoted_prose.font_ratio_page_ref = 0.8538;

        let mut short_continuation = test_line(4, 54, 60.6, 316.3, 78.0, 326.3, "but");
        short_continuation.sequence_footnote_zone = true;
        short_continuation.font_ratio_page_ref = 0.8562;

        for line in [&quoted_citation, &quoted_prose, &short_continuation] {
            assert!(sequence_zone_quoted_body_excerpt_like_body_line(line));
            assert!(line_should_never_be_marginalia_by_body_geometry(line));
            assert!(!model_line_should_be_marginalia(line));
            assert!(!footnote_specialist_line_can_be_marginalia(line));
            assert!(!should_decode_keep_as_marginalia(&[], line, None, None));
        }

        let mut quoted_note = test_line(
            3,
            51,
            85.2,
            128.3,
            487.6,
            138.3,
            "\" The result, in the instant case, would seem to indicate that even shoes",
        );
        quoted_note.prev_sequence_footnote_zone = true;
        quoted_note.font_ratio_page_ref = 0.8719;
        assert!(!sequence_zone_quoted_body_excerpt_like_body_line(
            &quoted_note
        ));
    }

    #[test]
    fn outside_sequence_model_supported_continuations_decode_as_marginalia() {
        let mut shifted_small = test_line(
            4,
            33,
            123.6,
            292.2,
            533.6,
            302.2,
            "deemed \"suspect\" are those involving characteristics over which the individual has",
        );
        shifted_small.font_ratio_page_ref = 0.81;
        shifted_small.font_ratio_page = 0.9972;
        shifted_small.font_ratio_doc = 0.81;
        shifted_small.prev_line_present = true;
        shifted_small.prev_sequence_footnote_zone = true;
        shifted_small.prev_small_font = true;
        shifted_small.prev_y_gap_ratio = 0.0094;
        shifted_small.prev_left_delta_ratio = 0.4872;
        shifted_small.next_line_present = true;
        shifted_small.next_small_font = true;
        assert!(outside_sequence_model_supported_continuation_can_be_marginalia(&shifted_small));

        assert!(looks_like_seq_comma_numeric_fragment("44 seq., 69"));

        let mut body_colon = test_line(
            5,
            32,
            72.0,
            272.0,
            286.0,
            282.0,
            "A peremptory writ of mandamus must be filed:",
        );
        body_colon.font_ratio_page_ref = 0.787;
        body_colon.font_ratio_doc = 0.787;
        body_colon.prev_line_present = true;
        body_colon.prev_sequence_footnote_zone = true;
        body_colon.prev_small_font = true;
        body_colon.prev_y_gap_ratio = 0.009;
        body_colon.prev_left_delta_ratio = 0.0317;
        assert!(!outside_sequence_model_supported_continuation_can_be_marginalia(&body_colon));
    }

    #[test]
    fn outside_sequence_specific_footnote_shapes_decode_as_marginalia() {
        let mut publication = test_line(
            3,
            29,
            56.7,
            299.4,
            559.8,
            309.4,
            "2 reasons why the drought in California won’t open the door to Great Lakes water, MICH. RADIO, Apr.",
        );
        publication.font_ratio_page_ref = 0.7778;
        publication.font_ratio_doc = 0.7778;
        publication.prev_line_present = true;
        publication.prev_sequence_footnote_zone = true;
        publication.prev_small_font = true;
        publication.prev_note_marker = true;
        publication.prev_y_gap_ratio = 0.0151;
        publication.prev_left_delta_ratio = 0.0402;
        assert!(previous_sequence_publication_continuation_can_be_marginalia(&publication));

        let mut quote = test_line(
            3,
            51,
            85.2,
            128.3,
            487.6,
            138.3,
            "\" The result, in the instant case, would seem to indicate that even shoes",
        );
        quote.font_ratio_page_ref = 0.8719;
        quote.font_ratio_page = 0.8827;
        quote.font_ratio_doc = 0.8719;
        quote.prev_line_present = true;
        quote.prev_sequence_footnote_zone = true;
        quote.prev_small_font = true;
        quote.prev_y_gap_ratio = 0.012;
        quote.prev_left_delta_ratio = 0.0162;
        assert!(previous_sequence_quote_continuation_can_be_marginalia(
            &quote
        ));

        let mut normal_citation = test_line(
            4,
            15,
            163.7,
            471.6,
            475.0,
            481.6,
            "See, e.g., EnCana Corp. v. Ecuador, LCIA Case No. UN 3481, Award (Feb. 3, 2006),",
        );
        normal_citation.font_ratio_page_ref = 1.0;
        normal_citation.prev_line_present = true;
        normal_citation.prev_sequence_footnote_zone = true;
        normal_citation.prev_small_font = false;
        normal_citation.prev_y_gap_ratio = 0.0;
        normal_citation.prev_left_delta_ratio = 0.029;
        assert!(
            previous_sequence_normal_font_citation_or_marker_can_be_marginalia(&normal_citation)
        );

        let mut numeric_marker = test_line(6, 23, 98.2, 401.1, 112.2, 411.1, "31.");
        numeric_marker.font_ratio_page_ref = 0.6923;
        numeric_marker.font_ratio_doc = 0.6923;
        numeric_marker.prev_line_present = true;
        numeric_marker.prev_sequence_footnote_zone = true;
        numeric_marker.prev_small_font = true;
        numeric_marker.prev_y_gap_ratio = 0.0013;
        numeric_marker.prev_left_delta_ratio = 0.0396;
        assert!(
            previous_sequence_normal_font_citation_or_marker_can_be_marginalia(&numeric_marker)
        );
    }

    #[test]
    fn early_numbered_note_pair_and_decoded_continuations_become_marginalia() {
        let mut lead = test_line(
            7,
            16,
            92.1,
            483.0,
            493.7,
            493.0,
            "33. Specific deterrence means that if an individual defendant is subject to criminal sanc-",
        );
        lead.font_ratio_page_ref = 0.9834;
        let mut continuation = test_line(
            7,
            17,
            82.4,
            471.8,
            492.3,
            481.8,
            "tions, that particular individual will not commit subsequent violations after being released",
        );
        continuation.font_ratio_page_ref = 0.9889;
        continuation.prev_note_marker = true;
        assert!(early_numbered_note_pair_can_be_marginalia(
            &lead,
            Some(&continuation)
        ));

        let hints = vec![LiquidLayoutHint {
            text: lead.text.clone(),
            role: LiquidBlockRole::Marginalia,
        }];
        assert!(decoded_adjacent_continuation_can_be_marginalia(
            &hints,
            &continuation,
            Some(&lead)
        ));

        let mut acronym = test_line(4, 20, 217.5, 451.4, 239.3, 461.4, "LCIA");
        acronym.font_ratio_page_ref = 1.0;
        let mut citation = test_line(
            4,
            15,
            163.7,
            471.6,
            475.0,
            481.6,
            "See, e.g., EnCana Corp. v. Ecuador, LCIA Case No. UN 3481, Award (Feb. 3, 2006),",
        );
        citation.font_ratio_page_ref = 1.0;
        let hints = vec![LiquidLayoutHint {
            text: citation.text.clone(),
            role: LiquidBlockRole::Marginalia,
        }];
        assert!(decoded_adjacent_continuation_can_be_marginalia(
            &hints,
            &acronym,
            Some(&citation)
        ));
    }

    #[test]
    fn decoded_repair_recovers_right_fragments_and_short_list_continuations() {
        let mut previous = test_line(
            4,
            40,
            123.6,
            247.4,
            503.0,
            257.4,
            "rights\", are those rights which attach to a person by the mere fact of citizenship.",
        );
        previous.font_ratio_page_ref = 0.8054;
        let mut right_fragment = test_line(4, 41, 512.8, 247.2, 533.4, 257.2, "Such");
        right_fragment.font_ratio_page_ref = 0.7915;
        right_fragment.prev_sequence_footnote_zone = true;
        right_fragment.prev_y_gap_ratio = 0.0002;
        right_fragment.prev_left_delta_ratio = 0.6357;
        right_fragment.prev_line_present = true;
        right_fragment.prev_small_font = true;

        let hints = vec![LiquidLayoutHint {
            text: previous.text.clone(),
            role: LiquidBlockRole::Marginalia,
        }];
        assert!(same_row_right_fragment_after_marginalia_can_be_marginalia(
            &hints,
            &right_fragment,
            Some(&previous)
        ));

        let mut list_item = test_line(1, 37, 107.9, 282.4, 191.0, 292.4, "paid or promised");
        list_item.font_ratio_page_ref = 0.994;
        list_item.prev_line_present = true;
        list_item.prev_sequence_footnote_zone = true;
        list_item.prev_y_gap_ratio = 0.0114;
        list_item.prev_left_delta_ratio = 0.002;
        list_item.prev_gap_to_median_ratio = 0.8;
        list_item.width_to_body_ratio = 0.2;
        let hints = vec![LiquidLayoutHint {
            text: list_item.text.clone(),
            role: LiquidBlockRole::ListItem,
        }];
        assert!(list_item_core_footnote_continuation_can_be_marginalia(
            &hints, &list_item
        ));
    }

    #[test]
    fn decoded_repair_recovers_small_line_before_next_note_run() {
        let mut line = test_line(
            5,
            28,
            109.1,
            335.6,
            401.4,
            345.6,
            "engaged in commercial undertakings because they are dependent",
        );
        line.font_ratio_page_ref = 0.7659;

        let mut next = test_line(
            5,
            36,
            61.8,
            280.4,
            286.3,
            290.4,
            "Burstyn, Inc. v. Wilson, 343 U.S. 495, 501-02 (1952).",
        );
        next.font_ratio_page_ref = 0.7399;
        let hints = vec![LiquidLayoutHint {
            text: next.text.clone(),
            role: LiquidBlockRole::Marginalia,
        }];

        assert!(small_font_line_before_next_note_run_can_be_marginalia(
            &hints,
            &line,
            Some(&next)
        ));
    }

    #[test]
    fn decoded_repair_recovers_contents_credential_before_notes() {
        let mut credential = test_line(
            1,
            47,
            54.5,
            207.9,
            464.5,
            217.9,
            "cinnati; J.D., 1980, Georgetown University Law Center; L.L.M., 1985, DePaul Univer-",
        );
        credential.page_contents_like = true;
        credential.font_ratio_page_ref = 0.7479;

        let mut note = test_line(
            1,
            52,
            67.9,
            161.8,
            293.3,
            171.8,
            "1. See infra notes 5-63 and accompanying text.",
        );
        note.page_contents_like = true;
        note.font_ratio_page_ref = 0.7479;
        let hints = vec![LiquidLayoutHint {
            text: note.text.clone(),
            role: LiquidBlockRole::Marginalia,
        }];

        assert!(contents_like_credential_before_note_run_can_be_marginalia(
            &hints,
            &credential,
            Some(&note)
        ));

        let mut previous = test_line(
            1,
            46,
            86.0,
            221.3,
            465.0,
            231.3,
            "Partner, Chapman and Cutler, Chicago, Illinois; B.B.A., 1977, University of Cin-",
        );
        previous.font_ratio_page_ref = 0.75;
        let mut continuation = credential.clone();
        continuation.page_contents_like = false;
        continuation.left = 54.5;
        continuation.right = 464.5;
        continuation.bottom = 207.9;
        continuation.top = 217.9;
        let hints = vec![LiquidLayoutHint {
            text: previous.text.clone(),
            role: LiquidBlockRole::Marginalia,
        }];

        assert!(
            metadata_credential_continuation_after_marginalia_can_be_marginalia(
                &hints,
                &continuation,
                Some(&previous)
            )
        );
    }

    #[test]
    fn decoded_repair_recovers_tiny_numeric_note_cluster() {
        let mut note_a = test_line(5, 86, 401.0, 109.6, 422.2, 119.6, "389.");
        note_a.font_ratio_page_ref = 0.7164;
        let mut note_b = test_line(5, 90, 356.7, 109.1, 376.3, 119.1, "105.");
        note_b.font_ratio_page_ref = 0.7169;
        let mut target = test_line(5, 108, 553.5, 82.3, 564.1, 92.3, "81");
        target.font_ratio_page_ref = 0.6701;

        let hints = vec![
            LiquidLayoutHint {
                text: note_a.text.clone(),
                role: LiquidBlockRole::Marginalia,
            },
            LiquidLayoutHint {
                text: note_b.text.clone(),
                role: LiquidBlockRole::Marginalia,
            },
        ];
        assert!(tiny_numeric_note_cluster_can_be_marginalia(
            &hints,
            &target,
            [&note_a, &note_b].into_iter()
        ));
    }

    #[test]
    fn body_fragments_before_inline_note_markers_do_not_become_marginalia() {
        let mut body_fragment = test_line(5, 26, 72.0, 346.0, 149.0, 356.0, "those limits.");
        body_fragment.sequence_footnote_zone = true;
        body_fragment.font_ratio_page_ref = 0.99;
        body_fragment.prev_line_present = true;
        body_fragment.prev_sequence_footnote_zone = true;
        body_fragment.prev_note_marker = true;
        body_fragment.prev_y_gap_ratio = 0.001;
        body_fragment.prev_left_delta_ratio = 0.13;

        assert!(normal_font_body_fragment_before_inline_note_marker(
            &body_fragment
        ));
        assert!(!model_line_should_be_marginalia(&body_fragment));
        assert!(!footnote_specialist_line_can_be_marginalia(&body_fragment));
        assert!(!should_decode_keep_as_marginalia(
            &[],
            &body_fragment,
            None,
            None,
        ));

        let mut continuation = body_fragment.clone();
        continuation.text =
            "authority of municipal officers by only permitting these officers to make".to_owned();
        continuation.prev_left_delta_ratio = 0.0;
        assert!(!normal_font_body_fragment_before_inline_note_marker(
            &continuation
        ));
    }

    #[test]
    fn body_quote_leadin_after_citation_does_not_decode_as_marginalia() {
        let mut leadin = test_line(
            7,
            32,
            109.1,
            272.0,
            323.0,
            282.0,
            "A peremptory writ of mandamus must be filed:",
        );
        leadin.prev_sequence_footnote_zone = true;
        leadin.font_ratio_page_ref = 0.787;

        assert!(body_quote_or_rule_leadin_after_citation_like_body_line(
            &leadin
        ));
        assert!(line_should_never_be_marginalia_by_body_geometry(&leadin));
        assert!(!should_decode_keep_as_marginalia(&[], &leadin, None, None));
    }

    #[test]
    fn small_font_bibliographic_lead_decodes_as_marginalia() {
        let mut book_lead = test_line(
            7,
            28,
            120.0,
            318.0,
            518.0,
            328.0,
            "HENRY MAINE, ANCIENT LAW: ITS CONNECTION TO THE HISTORY OF EARLY SOCIETY",
        );
        book_lead.font_ratio_page_ref = 0.66;

        assert!(looks_like_footnote_bibliographic_lead_text(&book_lead.text));
        assert!(small_font_bibliographic_lead_can_be_marginalia(&book_lead));
        assert!(should_decode_keep_as_marginalia(
            &[],
            &book_lead,
            None,
            None,
        ));

        let mut normal_title = book_lead.clone();
        normal_title.font_ratio_page_ref = 1.0;
        assert!(!small_font_bibliographic_lead_can_be_marginalia(
            &normal_title
        ));
    }

    #[test]
    fn runtime_layout_features_match_trainer_geometry_tokens() {
        let mut title = test_line(
            0,
            1,
            72.0,
            690.0,
            540.0,
            705.0,
            "THE DUTY TO READ THE UNREADABLE",
        );
        title.centered = true;
        title.font_ratio_doc = 1.25;
        title.bold = true;

        let title_tokens = feature_tokens(&title);
        assert_token_count(&title_tokens, "wc=8", 3);
        assert_token_count(&title_tokens, "line_word_count_bucket=8", 3);
        assert_token_count(&title_tokens, "wc_exact=6", 1);
        assert_token_count(&title_tokens, "line_word_count_exact=6", 1);
        assert_token_count(&title_tokens, "page_exact=0", 1);
        assert_token_count(&title_tokens, "early_article_page", 3);
        assert_token_count(&title_tokens, "all_caps_line", 5);
        assert_token_count(&title_tokens, "short_all_caps_line", 5);
        assert_token_count(&title_tokens, "all_caps_no_period", 4);
        assert_token_count(&title_tokens, "line_does_not_end_with_period", 1);
        assert_token_count(&title_tokens, "line_index=1", 3);
        assert_token_count(&title_tokens, "front_matter_top_band", 3);
        assert_token_count(&title_tokens, "first_page_centered_display_context", 8);
        assert_token_count(&title_tokens, "early_article_display_context", 8);
        assert_token_count(&title_tokens, "early_article_display_metadata_context", 8);
        assert_token_count(
            &title_tokens,
            "first_page_display_heading_geometry_context",
            0,
        );
        assert_token_count(&title_tokens, "first_page_title_band", 6);
        assert_token_count(&title_tokens, "is_bold", 3);

        let mut title_fragment = test_line(0, 3, 205.0, 635.0, 405.0, 650.0, "FAIR-CROSS-SECTION");
        title_fragment.centered = true;
        title_fragment.heading_geometry_like = true;
        title_fragment.font_ratio_doc = 1.23;
        title_fragment.width_to_body_ratio = 0.38;
        let title_fragment_tokens = feature_tokens(&title_fragment);
        assert_token_count(
            &title_fragment_tokens,
            "first_page_centered_display_context",
            8,
        );
        assert_token_count(
            &title_fragment_tokens,
            "first_page_display_heading_geometry_context",
            8,
        );
        assert_token_count(
            &title_fragment_tokens,
            "first_page_title_fragment_context",
            7,
        );
        assert_token_count(
            &title_fragment_tokens,
            "early_article_title_fragment_context",
            8,
        );

        let mut section_heading =
            test_line(4, 10, 86.0, 500.0, 430.0, 514.0, "The Public Meaning Canon");
        section_heading.bold = true;
        section_heading.heading_geometry_like = true;
        section_heading.font_ratio_body = 1.12;
        section_heading.width_to_body_ratio = 0.72;
        let section_tokens = feature_tokens(&section_heading);
        assert_token_count(&section_tokens, "wc=5", 3);
        assert_token_count(&section_tokens, "line_word_count_exact=4", 1);
        assert_token_count(&section_tokens, "heading_shape_short_nonperiod", 8);
        assert_token_count(&section_tokens, "larger_than_body_short_no_period", 5);

        let mut author_line = test_line(1, 4, 230.0, 640.0, 380.0, 654.0, "Frank L. Fine**");
        author_line.centered = true;
        author_line.bold = true;
        author_line.font_ratio_body = 1.14;
        author_line.heading_geometry_like = true;
        assert!(looks_like_probable_author_line(&author_line));
        assert!(early_article_display_title_fragment_should_not_be_heading(
            &author_line
        ));
        assert!(!heading_specialist_line_can_be_heading(&author_line));
        let author_tokens = feature_tokens(&author_line);
        assert_token_count(&author_tokens, "probable_author_line", 8);
        assert_token_count(&author_tokens, "early_article_probable_author_line", 8);
        assert_token_count(&author_tokens, "early_article_display_context", 8);

        let mut numeric_cell = test_line(5, 56, 480.0, 120.0, 520.0, 134.0, "20081");
        numeric_cell.centered = true;
        numeric_cell.bold = true;
        numeric_cell.narrow_measure_like = true;
        numeric_cell.heading_geometry_like = true;
        assert!(looks_like_numeric_table_cell_fragment(&numeric_cell));
        assert!(heading_specialist_fragment_should_not_be_heading(
            &numeric_cell
        ));
        assert!(!heading_specialist_line_can_be_heading(&numeric_cell));
        let numeric_tokens = feature_tokens(&numeric_cell);
        assert_token_count(&numeric_tokens, "numeric_table_cell_fragment", 8);
        assert_token_count(&numeric_tokens, "numeric_fragment_not_heading", 8);

        let mut lowercase_fragment = test_line(5, 27, 260.0, 430.0, 300.0, 444.0, "do");
        lowercase_fragment.centered = true;
        lowercase_fragment.heading_geometry_like = true;
        lowercase_fragment.font_ratio_body = 1.15;
        assert!(text_is_all_lowercase_alpha_fragment(
            &lowercase_fragment.text
        ));
        assert!(heading_specialist_fragment_should_not_be_heading(
            &lowercase_fragment
        ));
        assert!(!heading_specialist_line_can_be_heading(&lowercase_fragment));
        let lowercase_tokens = feature_tokens(&lowercase_fragment);
        assert_token_count(&lowercase_tokens, "lowercase_body_fragment", 8);
        assert_token_count(
            &lowercase_tokens,
            "lowercase_fragment_heading_shape_conflict",
            8,
        );

        let mut prose_continuation = test_line(
            7,
            12,
            72.0,
            300.0,
            540.0,
            314.0,
            "Division of the Justice Department reflect this sentiment.42 The",
        );
        prose_continuation.centered = true;
        prose_continuation.heading_geometry_like = true;
        prose_continuation.font_ratio_body = 1.35;
        assert!(centered_prose_continuation_should_not_be_heading(
            &prose_continuation
        ));
        let prose_tokens = feature_tokens(&prose_continuation);
        assert_token_count(&prose_tokens, "centered_prose_continuation", 8);
        assert_token_count(&prose_tokens, "centered_body_clause_not_heading", 8);

        let mut sentence_fragment = test_line(
            7,
            12,
            96.0,
            300.0,
            516.0,
            314.0,
            "Division of the Justice Department reflect this sentiment.42 The",
        );
        sentence_fragment.centered = true;
        sentence_fragment.heading_geometry_like = true;
        sentence_fragment.font_ratio_body = 1.40;
        assert!(sentence_case_body_continuation_should_not_be_heading(
            &sentence_fragment
        ));
        assert!(!heading_specialist_line_can_be_heading(&sentence_fragment));
        let sentence_tokens = feature_tokens(&sentence_fragment);
        assert_token_count(&sentence_tokens, "sentence_case_body_continuation", 8);
        assert_token_count(
            &sentence_tokens,
            "heading_shape_sentence_fragment_conflict",
            8,
        );

        let mut journal_header = test_line(4, 0, 185.0, 734.0, 425.0, 748.0, "MERCER LAW REVIEW");
        journal_header.centered = true;
        journal_header.heading_geometry_like = true;
        journal_header.font_ratio_body = 1.16;
        assert!(looks_like_law_review_journal_masthead_line(&journal_header));
        assert!(!heading_specialist_line_can_be_heading(&journal_header));
        let journal_tokens = feature_tokens(&journal_header);
        assert_token_count(&journal_tokens, "law_review_journal_masthead", 10);
        assert_token_count(&journal_tokens, "edge_law_review_journal_masthead", 8);

        let mut wide_journal_header =
            test_line(0, 1, 45.0, 685.0, 567.0, 699.0, "GEORGETOWN LAW JOURNAL");
        wide_journal_header.centered = true;
        wide_journal_header.heading_geometry_like = true;
        wide_journal_header.font_ratio_body = 1.16;
        assert!(looks_like_law_review_journal_masthead_line(
            &wide_journal_header
        ));
        assert!(!heading_specialist_line_can_be_heading(
            &wide_journal_header
        ));

        let mut all_caps_topic = test_line(4, 2, 96.0, 690.0, 276.0, 704.0, "MAPS AND CHARTS");
        all_caps_topic.font_ratio_page_ref = 1.0;
        all_caps_topic.font_ratio_body = 1.0;
        assert!(looks_like_uncited_all_caps_topic_heading(
            &all_caps_topic.text
        ));
        assert!(heading_specialist_line_can_be_heading(&all_caps_topic));

        let mut letter_heading = test_line(
            26,
            29,
            210.0,
            300.0,
            402.0,
            314.0,
            "B. Pressure in the System",
        );
        letter_heading.centered = true;
        letter_heading.vertically_isolated_like = true;
        letter_heading.font_ratio_body = 1.08;
        assert!(looks_like_clear_section_heading(&letter_heading.text));
        assert!(!centered_prose_continuation_should_not_be_heading(
            &letter_heading
        ));
        assert!(heading_specialist_line_can_be_heading(&letter_heading));

        let mut contents_tagged_section = test_line(
            0,
            14,
            96.0,
            280.0,
            516.0,
            294.0,
            "III. Defining Meaningful Involvement and Fair",
        );
        contents_tagged_section.page_contents_like = true;
        contents_tagged_section.centered = true;
        contents_tagged_section.heading_geometry_like = true;
        contents_tagged_section.font_ratio_body = 1.12;
        assert!(page_contents_clear_section_heading_can_be_heading(
            &contents_tagged_section
        ));
        assert!(heading_specialist_line_can_be_heading(
            &contents_tagged_section
        ));

        let mut contents_dot_leader = contents_tagged_section.clone();
        contents_dot_leader.text = "III. Defining Meaningful Involvement .......... 12".to_owned();
        assert!(looks_like_dot_leader_contents_fragment(
            &contents_dot_leader.text
        ));
        assert!(!page_contents_clear_section_heading_can_be_heading(
            &contents_dot_leader
        ));
        assert!(!heading_specialist_line_can_be_heading(
            &contents_dot_leader
        ));

        let mut contents_intro = test_line(1, 24, 248.0, 410.0, 366.0, 424.0, "INTRODUCTION");
        contents_intro.page_contents_like = true;
        contents_intro.heading_geometry_like = false;
        contents_intro.centered = false;
        contents_intro.bold = false;
        contents_intro.font_ratio_body = 1.0;
        assert!(looks_like_common_section_heading_label(
            &contents_intro.text
        ));
        assert!(page_contents_clear_section_heading_can_be_heading(
            &contents_intro
        ));
        assert!(heading_specialist_line_can_be_heading(&contents_intro));

        let mut table_of_contents =
            test_line(1, 3, 220.0, 120.0, 392.0, 134.0, "TABLE OF CONTENTS");
        table_of_contents.page_contents_like = true;
        table_of_contents.centered = true;
        table_of_contents.heading_geometry_like = true;
        table_of_contents.font_ratio_body = 1.12;
        assert!(!looks_like_common_section_heading_label(
            &table_of_contents.text
        ));
        assert!(!heading_specialist_line_can_be_heading(&table_of_contents));

        let mut contents_abstract = test_line(1, 29, 250.0, 410.0, 362.0, 424.0, "ABSTRACT");
        contents_abstract.page_contents_like = true;
        contents_abstract.heading_geometry_like = false;
        contents_abstract.centered = false;
        contents_abstract.bold = false;
        contents_abstract.font_ratio_body = 0.85;
        assert!(looks_like_common_section_heading_label(
            &contents_abstract.text
        ));
        assert!(page_contents_clear_section_heading_can_be_heading(
            &contents_abstract
        ));
        assert!(heading_specialist_line_can_be_heading(&contents_abstract));

        let mut footnote = test_line(
            3,
            12,
            72.0,
            165.0,
            540.0,
            174.0,
            "continued citation material in the same small font.",
        );
        footnote.sequence_footnote_zone = true;
        footnote.repeated_header_footer = true;
        footnote.italic = true;
        footnote.font_ratio_page_ref = 0.88;

        let base_tokens = feature_tokens(&footnote);
        assert_token_count(&base_tokens, "wc=8", 3);
        assert_token_count(&base_tokens, "line_word_count_bucket=8", 3);
        assert_token_count(&base_tokens, "line_word_count_exact=8", 1);
        assert_token_count(&base_tokens, "line_ends_with_period", 2);
        assert_token_count(&base_tokens, "line_index=20", 3);
        assert_token_count(&base_tokens, "after_front_matter", 2);
        assert_token_count(&base_tokens, "sequence_footnote_zone", 8);
        assert_token_count(&base_tokens, "geom_no_divider_sequence_note", 8);
        assert_token_count(&base_tokens, "later_repeated_edge_text", 6);
        assert_token_count(&base_tokens, "repeated_edge_text", 6);
        assert_token_count(&base_tokens, "is_italic", 2);

        let specialist_tokens = footnote_specialist_feature_tokens(&footnote);
        assert_token_count(&specialist_tokens, "sequence_footnote_zone", 8);
        assert_token_count(&specialist_tokens, "geom_no_divider_sequence_note", 8);

        let mut no_divider_start = test_line(
            2,
            25,
            72.0,
            300.0,
            540.0,
            309.0,
            "12. See Example v. State, 123 U.S. 456 (2020).",
        );
        no_divider_start.font_ratio_page_ref = 0.84;
        let start_tokens = feature_tokens(&no_divider_start);
        assert_token_count(&start_tokens, "geom_no_divider_note_start", 8);
        assert_token_count(&start_tokens, "geom_no_divider_legal_note", 8);

        let l_rev = test_line(
            2,
            26,
            72.0,
            286.0,
            540.0,
            295.0,
            "13. See Example, 120 Harv. L. Rev. 456 (2020).",
        );
        let l_rev_tokens = feature_tokens(&l_rev);
        assert_token_count(&l_rev_tokens, "contains_l_rev", 8);
        assert_token_count(&l_rev_tokens, "contains_l_rev_citation", 8);

        let mut small_mid_body = test_line(
            2,
            8,
            72.0,
            420.0,
            540.0,
            430.0,
            "ordinary small-font body material before any footnote zone",
        );
        small_mid_body.font_ratio_page_ref = 0.84;
        let body_tokens = feature_tokens(&small_mid_body);
        assert_token_count(&body_tokens, "geom_no_divider_small_mid_body", 4);
        assert_token_count(&body_tokens, "geom_no_divider_note_start", 0);

        let mut contents_entry = test_line(
            1,
            6,
            72.0,
            560.0,
            540.0,
            570.0,
            "A. Data ................................................................ 2270",
        );
        contents_entry.page_contents_like = true;
        contents_entry.contents_or_index_entry = true;
        let contents_tokens = feature_tokens(&contents_entry);
        assert_token_count(&contents_tokens, "page_contents_like", 8);
        assert_token_count(&contents_tokens, "contents_or_index_entry", 8);
        assert_token_count(&contents_tokens, "page_contents_entry", 10);
    }

    #[test]
    fn heading_specialist_stack_tokens_include_training_stack_context() {
        let mut line = test_line(4, 10, 86.0, 500.0, 430.0, 514.0, "The Public Meaning Canon");
        line.bold = true;
        line.heading_geometry_like = true;
        line.font_ratio_body = 1.12;
        line.width_to_body_ratio = 0.72;

        let tokens = heading_specialist_stack_tokens(&line);
        for name in [
            "main",
            "liquid",
            "doclaynet_main",
            "doclaynet_liquid",
            "body",
            "body_chandra",
            "heading_chandra",
        ] {
            let role_prefix = format!("stack={name}:role=");
            let margin_prefix = format!("stack={name}:margin=");
            let role_margin_prefix = format!("stack={name}:role_margin=");
            let role_count = tokens
                .iter()
                .filter(|token| token.starts_with(&role_prefix))
                .count();
            let margin_count = tokens
                .iter()
                .filter(|token| token.starts_with(&margin_prefix))
                .count();
            let role_margin_count = tokens
                .iter()
                .filter(|token| token.starts_with(&role_margin_prefix))
                .count();
            assert_eq!(role_count, 8, "role tokens for {name}: {tokens:?}");
            assert_eq!(margin_count, 1, "margin token for {name}: {tokens:?}");
            assert_eq!(
                role_margin_count, 1,
                "role-margin token for {name}: {tokens:?}"
            );
        }
    }

    #[test]
    fn line_extraction_prefers_pdf_font_size_over_bounds_height() {
        let page = PageInfo::new(612.0, 792.0);
        let mut chars = Vec::new();
        for (index, ch) in "Bold line".chars().enumerate() {
            let left = 72.0 + index as f32 * 5.0;
            chars.push(PageTextChar {
                ch,
                rect: Some(PdfRect::new(left, 680.0, left + 5.0, 700.0)),
                font_size: Some(9.0),
                bold: true,
                italic: false,
            });
        }

        let lines = extract_lines(0, &page, &chars);
        assert_eq!(lines.len(), 1);
        assert!(
            (lines[0].font_height - 9.0).abs() < 0.01,
            "expected true font size, got {}",
            lines[0].font_height
        );
        assert!(lines[0].bold);
        assert!(!lines[0].italic);
    }

    #[test]
    fn extraction_v2_splits_midline_font_step_note_prose() {
        let page = PageInfo::new(612.0, 792.0);
        let mut chars = Vec::new();
        push_text_run(
            &mut chars,
            72.0,
            680.0,
            12.0,
            12.0,
            "This is ordinary body prose before a note ",
        );
        push_text_run(&mut chars, 292.0, 683.0, 8.0, 8.0, "44 That is a note.");

        let (v1, v1_trace) = extract_lines_with_options(0, &page, &chars, false);
        assert_eq!(v1_trace.stats.lines_split, 0);
        assert_eq!(v1.len(), 1);
        // v1 wraps the small inline marker "44" as a superscript footnote callout.
        assert_eq!(
            v1[0].text,
            "This is ordinary body prose before a note \u{e000}44\u{e001} That is a note."
        );

        let (v2, v2_trace) = extract_lines_with_options(0, &page, &chars, true);
        assert_eq!(v2.len(), 2);
        assert_eq!(v2_trace.stats.lines_split, 1);
        assert_eq!(v2_trace.events[0].kind, "lines_split");
        assert_eq!(v2[0].text, "This is ordinary body prose before a note");
        assert_eq!(v2[1].text, "44 That is a note.");
        assert!(v2[1].font_height < v2[0].font_height);
    }

    #[test]
    fn extraction_v2_attaches_raised_numeric_marker_to_anchor_line() {
        let page = PageInfo::new(612.0, 792.0);
        let mut chars = Vec::new();
        push_text_run(&mut chars, 72.0, 700.0, 6.0, 6.0, "148");
        push_text_run(
            &mut chars,
            78.0,
            680.0,
            12.0,
            12.0,
            "Recycling the argument begins here.",
        );

        let (v1, _) = extract_lines_with_options(0, &page, &chars, false);
        assert_eq!(v1.len(), 2);
        assert_eq!(v1[0].text, "148");
        assert_eq!(v1[1].text, "Recycling the argument begins here.");

        let (v2, trace) = extract_lines_with_options(0, &page, &chars, true);
        assert_eq!(v2.len(), 1);
        assert_eq!(trace.stats.markers_attached, 1);
        assert_eq!(trace.stats.markers_attached_backward, 0);
        assert_eq!(trace.stats.markers_dropped, 0);
        assert_eq!(trace.events[0].kind, "markers_attached_forward");
        assert_eq!(v2[0].text, "148 Recycling the argument begins here.");
        assert_eq!(v2[0].line_index, 0);
    }

    #[test]
    fn extraction_v2_attaches_marker_to_punctuation_started_note_line() {
        let page = PageInfo::new(612.0, 792.0);
        let mut chars = Vec::new();
        push_text_run(&mut chars, 72.0, 700.0, 6.0, 6.0, "123");
        push_text_run(&mut chars, 102.0, 686.0, 8.0, 8.0, ". Id. at 1245.");

        let (v2, trace) = extract_lines_with_options(0, &page, &chars, true);
        assert_eq!(v2.len(), 1);
        assert_eq!(trace.stats.markers_attached, 1);
        assert_eq!(v2[0].text, "123. Id. at 1245.");
    }

    #[test]
    fn extraction_v2_attaches_line_final_marker_backward_to_body_line() {
        let page = PageInfo::new(612.0, 792.0);
        let mut chars = Vec::new();
        push_text_run(
            &mut chars,
            72.0,
            680.0,
            12.0,
            12.0,
            "This sentence ends with a citation.",
        );
        push_text_run(&mut chars, 235.0, 686.0, 6.0, 6.0, "95");

        let (v1, _) = extract_lines_with_options(0, &page, &chars, false);
        assert_eq!(v1.len(), 1);
        // v1 wraps the small inline marker "95" as a superscript footnote callout.
        assert_eq!(
            v1[0].text,
            "This sentence ends with a citation.\u{e000}95\u{e001}"
        );

        let (v2, trace) = extract_lines_with_options(0, &page, &chars, true);
        assert_eq!(v2.len(), 1);
        assert_eq!(v2[0].text, "This sentence ends with a citation. 95");
        assert_eq!(trace.stats.lines_split, 1);
        assert_eq!(trace.stats.markers_attached, 1);
        assert_eq!(trace.stats.markers_attached_backward, 1);
        assert_eq!(trace.stats.markers_dropped, 0);
        assert_eq!(trace.events[1].kind, "markers_attached_backward");
    }

    #[test]
    fn extraction_v2_does_not_forward_attach_line_final_marker_to_next_paragraph() {
        let page = PageInfo::new(612.0, 792.0);
        let mut chars = Vec::new();
        push_text_run(
            &mut chars,
            72.0,
            700.0,
            12.0,
            12.0,
            "Appropriately tailored.",
        );
        push_text_run(&mut chars, 185.0, 706.0, 6.0, 6.0, "15");
        push_text_run(
            &mut chars,
            72.0,
            680.0,
            12.0,
            12.0,
            "The conflict among courts starts a new paragraph.",
        );

        let (v2, trace) = extract_lines_with_options(0, &page, &chars, true);
        assert_eq!(v2.len(), 2);
        assert_eq!(v2[0].text, "Appropriately tailored. 15");
        assert_eq!(
            v2[1].text,
            "The conflict among courts starts a new paragraph."
        );
        assert_eq!(trace.stats.markers_attached, 1);
        assert_eq!(trace.stats.markers_attached_backward, 1);
        assert_eq!(trace.stats.markers_dropped, 0);
        assert_eq!(trace.events[1].kind, "markers_attached_backward");
    }

    #[test]
    fn extraction_v2_counts_dropped_unattached_numeric_marker() {
        let page = PageInfo::new(612.0, 792.0);
        let mut chars = Vec::new();
        push_text_run(&mut chars, 72.0, 700.0, 6.0, 6.0, "12");
        push_text_run(&mut chars, 420.0, 650.0, 12.0, 12.0, "Remote body line.");

        let (v2, trace) = extract_lines_with_options(0, &page, &chars, true);
        assert_eq!(v2.len(), 1);
        assert_eq!(v2[0].text, "Remote body line.");
        assert_eq!(trace.stats.markers_attached, 0);
        assert_eq!(trace.stats.markers_dropped, 1);
        assert_eq!(trace.events[0].kind, "markers_dropped");
    }

    #[test]
    fn extraction_v2_merges_inline_superscript_marker_split_back_to_body_line() {
        let page = PageInfo::new(612.0, 792.0);
        let mut chars = Vec::new();
        push_text_run(
            &mut chars,
            72.0,
            680.0,
            12.0,
            12.0,
            "the purpose of buying a bakery.",
        );
        push_text_run(&mut chars, 240.0, 683.0, 6.0, 6.0, "131");
        push_text_run(
            &mut chars,
            258.0,
            680.0,
            12.0,
            12.0,
            " Rund argued on appeal.",
        );

        let (v2, trace) = extract_lines_with_options(0, &page, &chars, true);
        assert_eq!(v2.len(), 1);
        assert_eq!(trace.stats.inline_splits_merged, 1);
        assert_eq!(trace.events[1].kind, "inline_splits_merged");
        assert_eq!(
            v2[0].text,
            "the purpose of buying a bakery.131 Rund argued on appeal."
        );
    }

    #[test]
    fn model_noise_guard_keeps_body_continuations_and_citations() {
        let page = PageInfo::new(612.0, 792.0);
        let body_top = page.height - 86.7;
        let body = test_line(
            1,
            1,
            72.0,
            body_top - 8.0,
            160.0,
            body_top,
            "the difference.10",
        );
        assert!(!model_noise_line_can_be_hidden(&page, &body));

        let citation_top = page.height - 271.8;
        let citation = test_line(
            5,
            18,
            72.0,
            citation_top - 10.0,
            520.0,
            citation_top,
            "contents before signing it . . . .\"); Rosenfeld v. JPMorgan Chase Bank, N.A., 732 F. Supp. 2d 952, 965",
        );
        assert!(!model_noise_line_can_be_hidden(&page, &citation));
    }

    #[test]
    fn model_noise_guard_accepts_disposable_layout_clutter() {
        let page = PageInfo::new(612.0, 792.0);
        let mut running_header = test_line(
            3,
            0,
            72.0,
            page.height - 42.0,
            300.0,
            page.height - 34.0,
            "Harvard Law Review",
        );
        running_header.repeated_header_footer = true;
        assert!(model_noise_line_can_be_hidden(&page, &running_header));

        let toc_top = page.height - 220.0;
        let toc = test_line(
            1,
            12,
            72.0,
            toc_top - 10.0,
            520.0,
            toc_top,
            "Introduction ........................................ 1",
        );
        assert!(model_noise_line_can_be_hidden(&page, &toc));

        let volume_top = page.height - 166.0;
        let volume = test_line(
            0,
            2,
            72.0,
            volume_top - 10.0,
            160.0,
            volume_top,
            "Volume 52",
        );
        assert!(model_noise_line_can_be_hidden(&page, &volume));
    }

    #[test]
    fn nonlegal_study_prompt_noise_does_not_become_marginalia() {
        let page = PageInfo::new(612.0, 792.0);
        let prompt_top = page.height - 476.0;
        let mut prompt = test_line(
            5,
            30,
            72.0,
            prompt_top - 10.0,
            180.0,
            prompt_top,
            "What is t1Truthll?",
        );
        prompt.sequence_footnote_zone = true;
        prompt.font_ratio_page_ref = 0.9811;

        assert!(looks_like_nonlegal_study_prompt_noise(&prompt));
        assert!(model_noise_line_can_be_hidden(&page, &prompt));
        assert!(!footnote_specialist_line_can_be_marginalia(&prompt));
        assert!(!model_line_should_be_marginalia(&prompt));
        assert!(!should_decode_keep_as_marginalia(&[], &prompt, None, None));

        let mut non_sequence_prompt = prompt.clone();
        non_sequence_prompt.sequence_footnote_zone = false;
        non_sequence_prompt.prev_sequence_footnote_zone = true;
        assert!(looks_like_nonlegal_study_prompt_noise(&non_sequence_prompt));
        assert!(model_noise_line_can_be_hidden(&page, &non_sequence_prompt));

        let stage_top = page.height - 729.0;
        let mut stage = test_line(
            5,
            48,
            72.0,
            stage_top - 10.0,
            260.0,
            stage_top,
            "Completed Stage - ra,t ional coherence",
        );
        stage.sequence_footnote_zone = true;
        stage.font_ratio_page_ref = 0.9811;
        assert!(looks_like_nonlegal_study_prompt_noise(&stage));
        assert!(model_noise_line_can_be_hidden(&page, &stage));

        let mut legal_question = test_line(
            5,
            31,
            72.0,
            prompt_top - 10.0,
            260.0,
            prompt_top,
            "What is the holding of Marbury v. Madison?",
        );
        legal_question.sequence_footnote_zone = true;
        legal_question.font_ratio_page_ref = 0.9811;
        assert!(!looks_like_nonlegal_study_prompt_noise(&legal_question));
        assert!(!model_noise_line_can_be_hidden(&page, &legal_question));
    }

    #[test]
    fn model_noise_guard_hides_edge_running_header_fragments() {
        let page = PageInfo::new(612.0, 792.0);
        let year = test_line(7, 42, 72.0, 738.0, 112.0, 746.0, "1996]");
        let volume = test_line(5, 4, 72.0, 704.0, 170.0, 714.0, "[Vol. 87, 311");
        let title_header = test_line(
            7,
            1,
            72.0,
            710.0,
            260.0,
            720.0,
            "Torture, Ethics, Accountability?",
        );
        let numeric_fragment = test_line(1, 72, 72.0, 740.0, 104.0, 748.0, "19351");
        let bracketed_year = test_line(3, 52, 72.0, 736.0, 112.0, 744.0, "1989]");

        assert!(looks_like_edge_running_header_footer_fragment(&year));
        assert!(looks_like_edge_running_header_footer_fragment(&volume));
        assert!(looks_like_edge_running_header_footer_fragment(
            &title_header
        ));
        assert!(looks_like_edge_running_header_footer_fragment(
            &numeric_fragment
        ));
        assert!(looks_like_edge_running_header_footer_fragment(
            &bracketed_year
        ));
        assert!(model_noise_line_can_be_hidden(&page, &year));
        assert!(model_noise_line_can_be_hidden(&page, &volume));
        assert!(model_noise_line_can_be_hidden(&page, &title_header));
        assert!(model_noise_line_can_be_hidden(&page, &numeric_fragment));
        assert!(model_noise_line_can_be_hidden(&page, &bracketed_year));
    }

    #[test]
    fn edge_running_header_fragment_rule_preserves_first_page_title() {
        let page = PageInfo::new(612.0, 792.0);
        let title = test_line(
            0,
            1,
            72.0,
            710.0,
            360.0,
            720.0,
            "Torture, Ethics, Accountability?",
        );

        assert!(!looks_like_edge_running_header_footer_fragment(&title));
        assert!(!model_noise_line_can_be_hidden(&page, &title));
    }

    #[test]
    fn numbered_dot_leader_contents_entry_is_noise_not_marginalia() {
        let page = PageInfo::new(612.0, 792.0);
        let toc_top = page.height - 580.0;
        let mut toc = test_line(
            3,
            31,
            72.0,
            toc_top - 10.0,
            520.0,
            toc_top,
            "1. Implicit Bias and Tools to Measure It ............................. 552",
        );
        toc.font_ratio_page_ref = 0.84;

        assert!(is_disposable_contents_or_index_line(&toc));
        assert!(model_noise_line_can_be_hidden(&page, &toc));
        assert!(!model_line_should_be_marginalia(&toc));
        assert!(!footnote_specialist_line_can_be_marginalia(&toc));
        assert!(!starts_sequence_footnote_zone(&toc));
    }

    #[test]
    fn contents_like_page_footnote_below_divider_can_still_be_marginalia() {
        let page = PageInfo::new(612.0, 792.0);
        let mut note = test_line(
            3,
            44,
            72.0,
            page.height - 640.0,
            430.0,
            page.height - 630.0,
            "37. Id. at 236, 581 S.E.2d at 582.",
        );
        note.page_contents_like = true;
        note.below_footnote_divider = true;
        note.font_ratio_page_ref = 0.80;

        assert!(!is_disposable_contents_or_index_line(&note));
        assert!(contents_like_page_line_can_be_marginalia(&note));
        assert!(model_line_should_be_marginalia(&note));
        assert!(footnote_specialist_line_can_be_marginalia(&note));

        let mut toc = note.clone();
        toc.text =
            "1. Implicit Bias and Tools to Measure It ............................. 552".to_owned();
        toc.contents_or_index_entry = true;

        assert!(is_disposable_contents_or_index_line(&toc));
        assert!(!model_line_should_be_marginalia(&toc));
        assert!(!footnote_specialist_line_can_be_marginalia(&toc));
    }

    #[test]
    fn contents_like_page_small_bottom_note_continuation_can_be_marginalia() {
        let page = PageInfo::new(612.0, 792.0);
        let mut continuation = test_line(
            2,
            35,
            120.0,
            page.height - 540.0,
            455.0,
            page.height - 530.0,
            "in this issue of the Loyola University Chicago Law Journal. See generally Robin W. Lovin,",
        );
        continuation.page_contents_like = true;
        continuation.font_ratio_page_ref = 0.74;

        assert!(contents_like_page_line_can_be_marginalia(&continuation));
        assert!(model_line_should_be_marginalia(&continuation));
        assert!(footnote_specialist_line_can_be_marginalia(&continuation));

        let mut citation_continuation = test_line(
            1,
            54,
            90.0,
            page.height - 590.0,
            445.0,
            page.height - 580.0,
            "See, e.g., Martin Kaste, Arrested for Resisting Arrest-Yes, It's Possible, NPR (Jan.",
        );
        citation_continuation.page_contents_like = true;
        citation_continuation.font_ratio_page_ref = 0.827;

        assert!(contents_like_page_line_can_be_marginalia(
            &citation_continuation
        ));
        let mut marker = test_line(1, 53, 82.0, 200.0, 92.0, 210.0, "2.");
        marker.page_contents_like = true;
        marker.font_ratio_page_ref = 0.74;
        let hints = vec![LiquidLayoutHint {
            text: marker.text.clone(),
            role: LiquidBlockRole::Marginalia,
        }];
        assert!(should_decode_keep_as_marginalia(
            &hints,
            &citation_continuation,
            Some(&marker),
            None
        ));

        let mut contents_entry = continuation.clone();
        contents_entry.text =
            "1. Implicit Bias and Tools to Measure It ............................. 552".to_owned();
        contents_entry.contents_or_index_entry = true;

        assert!(is_disposable_contents_or_index_line(&contents_entry));
        assert!(!model_line_should_be_marginalia(&contents_entry));
        assert!(!footnote_specialist_line_can_be_marginalia(&contents_entry));
    }

    #[test]
    fn orphan_contents_page_fragments_are_noise_not_marginalia() {
        let page = PageInfo::new(612.0, 792.0);
        let mut criminal_procedure = test_line(
            5,
            91,
            130.0,
            page.height - 516.0,
            260.0,
            page.height - 506.0,
            "and Criminal Procedure",
        );
        criminal_procedure.page_contents_like = true;
        criminal_procedure.font_ratio_page_ref = 0.91;

        assert!(looks_like_orphan_contents_page_fragment_noise(
            &criminal_procedure
        ));
        assert!(is_disposable_contents_or_index_line(&criminal_procedure));
        assert!(model_noise_line_can_be_hidden(&page, &criminal_procedure));
        assert!(!model_line_should_be_marginalia(&criminal_procedure));
        assert!(!footnote_specialist_line_can_be_marginalia(
            &criminal_procedure
        ));
        assert!(!should_decode_keep_as_marginalia(
            &[],
            &criminal_procedure,
            None,
            None
        ));

        let mut sequence_criminal_procedure = criminal_procedure.clone();
        sequence_criminal_procedure.page_contents_like = false;
        sequence_criminal_procedure.sequence_footnote_zone = true;
        sequence_criminal_procedure.prev_sequence_footnote_zone = true;

        assert!(looks_like_sequence_zone_orphan_contents_fragment_noise(
            &sequence_criminal_procedure
        ));
        assert!(is_disposable_contents_or_index_line(
            &sequence_criminal_procedure
        ));
        assert!(model_noise_line_can_be_hidden(
            &page,
            &sequence_criminal_procedure
        ));
        assert!(!model_line_should_be_marginalia(
            &sequence_criminal_procedure
        ));

        let mut system_question = test_line(
            5,
            128,
            130.0,
            page.height - 648.0,
            170.0,
            page.height - 638.0,
            "System?",
        );
        system_question.page_contents_like = true;
        system_question.font_ratio_page_ref = 1.0;

        assert!(looks_like_orphan_contents_page_fragment_noise(
            &system_question
        ));
        assert!(is_disposable_contents_or_index_line(&system_question));
        assert!(model_noise_line_can_be_hidden(&page, &system_question));
        assert!(!model_line_should_be_marginalia(&system_question));

        let mut sequence_system_question = system_question.clone();
        sequence_system_question.page_contents_like = false;
        sequence_system_question.sequence_footnote_zone = true;

        assert!(looks_like_sequence_zone_orphan_contents_fragment_noise(
            &sequence_system_question
        ));
        assert!(is_disposable_contents_or_index_line(
            &sequence_system_question
        ));
        assert!(model_noise_line_can_be_hidden(
            &page,
            &sequence_system_question
        ));

        let mut table_page = test_line(
            5,
            113,
            130.0,
            page.height - 578.0,
            150.0,
            page.height - 568.0,
            "880",
        );
        table_page.prev_sequence_footnote_zone = true;
        table_page.font_ratio_page_ref = 0.8357;

        assert!(looks_like_table_index_page_number_noise(&table_page));
        assert!(is_disposable_contents_or_index_line(&table_page));
        assert!(model_noise_line_can_be_hidden(&page, &table_page));
        assert!(!model_line_should_be_marginalia(&table_page));

        let mut cross_reference = test_line(
            5,
            87,
            130.0,
            page.height - 486.0,
            268.0,
            page.height - 476.0,
            "See Magill, Roswell.",
        );
        cross_reference.sequence_footnote_zone = true;
        cross_reference.font_ratio_page_ref = 0.9425;

        assert!(looks_like_index_cross_reference_name_noise(
            &cross_reference
        ));
        assert!(is_disposable_contents_or_index_line(&cross_reference));
        assert!(model_noise_line_can_be_hidden(&page, &cross_reference));
        assert!(!model_line_should_be_marginalia(&cross_reference));

        let mut legal_question = system_question.clone();
        legal_question.text = "What is the holding of Marbury v. Madison?".to_owned();
        assert!(!looks_like_orphan_contents_page_fragment_noise(
            &legal_question
        ));
        assert!(!is_disposable_contents_or_index_line(&legal_question));
    }

    #[test]
    fn punctuation_rule_fragments_are_noise_not_marginalia() {
        let page = PageInfo::new(612.0, 792.0);
        let mut dot = test_line(4, 117, 312.0, 98.0, 313.0, 102.0, ".");
        dot.sequence_footnote_zone = true;
        dot.font_ratio_page_ref = 0.20;

        assert!(looks_like_punctuation_rule_noise(&dot.text));
        assert!(is_disposable_contents_or_index_line(&dot));
        assert!(model_noise_line_can_be_hidden(&page, &dot));
        assert!(!model_line_should_be_marginalia(&dot));
        assert!(!footnote_specialist_line_can_be_marginalia(&dot));

        let mut rule = test_line(
            4,
            118,
            90.0,
            252.0,
            420.0,
            258.0,
            ".............-------------------------------",
        );
        rule.sequence_footnote_zone = true;
        rule.font_ratio_page_ref = 0.52;

        assert!(looks_like_punctuation_rule_noise(&rule.text));
        assert!(is_disposable_contents_or_index_line(&rule));
        assert!(model_noise_line_can_be_hidden(&page, &rule));
        assert!(!model_line_should_be_marginalia(&rule));

        let mut author_marker = test_line(0, 18, 72.0, 430.0, 76.0, 438.0, "*");
        author_marker.font_ratio_page_ref = 0.82;

        assert!(!looks_like_punctuation_rule_noise(&author_marker.text));
        assert!(!is_disposable_contents_or_index_line(&author_marker));
    }

    #[test]
    fn repository_cover_identifiers_get_deterministic_noise_hints() {
        let lines = vec![
            test_line(0, 2, 72.0, 620.0, 160.0, 632.0, "Volume 36 | Number 3"),
            test_line(0, 3, 72.0, 600.0, 160.0, 612.0, "Article 5"),
        ];
        let mut hints = Vec::new();
        extend_repository_cover_hints(&mut hints, &lines);

        assert!(
            hints
                .iter()
                .any(|hint| hint.text == "Volume 36 | Number 3"
                    && hint.role == LiquidBlockRole::Noise)
        );
        assert!(
            hints
                .iter()
                .any(|hint| hint.text == "Article 5" && hint.role == LiquidBlockRole::Noise)
        );
    }

    #[test]
    fn running_law_review_volume_cite_becomes_noise_not_table_or_marginalia() {
        let page = PageInfo::new(612.0, 792.0);
        let running_cite = test_line(
            12,
            0,
            72.0,
            page.height - 50.0,
            360.0,
            page.height - 42.0,
            "2266 Boston College Law Review [Vol. 60:2255",
        );

        assert!(looks_like_running_law_review_cite_line(&running_cite.text));
        assert!(!is_table_line(&running_cite.text));
        assert!(!model_line_should_be_marginalia(&running_cite));
        assert!(model_noise_line_can_be_hidden(&page, &running_cite));

        let mut chars = Vec::new();
        push_line(
            &mut chars,
            &page,
            42.0,
            8.0,
            "2266 Boston College Law Review [Vol. 60:2255",
        );
        let hints = layout_hints_for_pages(&[page], &[Some(chars)]);
        assert!(hints.iter().any(|hint| {
            hint.role == LiquidBlockRole::Noise
                && hint.text.starts_with("2266 Boston College Law Review")
        }));
        assert!(!hints.iter().any(|hint| {
            hint.text.starts_with("2266 Boston College Law Review")
                && (hint.role == LiquidBlockRole::Table || hint.role == LiquidBlockRole::Marginalia)
        }));
    }

    #[test]
    fn running_year_title_page_header_is_noise_not_heading_or_marginalia() {
        let page = PageInfo::new(612.0, 792.0);
        let running_cite = test_line(
            12,
            0,
            72.0,
            page.height - 50.0,
            360.0,
            page.height - 42.0,
            "2019] The Duty to Read the Unreadable 2257",
        );

        assert!(looks_like_running_law_review_cite_line(&running_cite.text));
        assert!(!is_table_line(&running_cite.text));
        assert!(!looks_like_clear_section_heading(&running_cite.text));
        assert!(!model_line_should_be_marginalia(&running_cite));
        assert!(model_noise_line_can_be_hidden(&page, &running_cite));
    }

    #[test]
    fn table_of_contents_rows_are_not_tableish_visuals() {
        let toc = "I. THEORETICAL BACKGROUND................................................................................................. 2260";

        assert!(is_contents_line(toc));
        assert!(!is_table_line(toc));
        assert!(is_plain_page_number_line("2255"));
        assert!(!is_table_line("2255"));
    }

    #[test]
    fn split_legacy_name_index_pages_do_not_start_marginalia() {
        let page = PageInfo::new(612.0, 792.0);
        let mut lines = vec![
            test_line(5, 0, 72.0, 450.0, 210.0, 460.0, "Hamilton,Robert P.:"),
            test_line(5, 1, 212.0, 450.0, 360.0, 460.0, "See Magill, Roswell."),
            test_line(5, 2, 500.0, 450.0, 520.0, 460.0, "116"),
            test_line(5, 3, 72.0, 432.0, 210.0, 442.0, "Harno,Albert J.:"),
            test_line(5, 4, 212.0, 432.0, 360.0, 442.0, "Cases and Other Material"),
            test_line(5, 5, 362.0, 432.0, 480.0, 442.0, "on Criminal Law"),
            test_line(5, 6, 500.0, 432.0, 520.0, 442.0, "885"),
            test_line(5, 7, 72.0, 414.0, 210.0, 424.0, "Hopkins,James Love:"),
            test_line(
                5,
                8,
                212.0,
                414.0,
                430.0,
                424.0,
                "The New Federal Equity Rules",
            ),
            test_line(5, 9, 500.0, 414.0, 520.0, 424.0, "134"),
            test_line(5, 10, 72.0, 396.0, 210.0, 406.0, "Glueck,Sheldon:"),
            test_line(5, 11, 500.0, 396.0, 520.0, 406.0, "384"),
        ];
        for line in &mut lines {
            line.font_ratio_page_ref = 0.94;
        }

        mark_page_context_features(&mut lines);
        mark_sequence_footnote_zones(&page, &mut lines);

        assert!(lines.iter().all(|line| line.page_contents_like));
        assert!(lines[0].contents_or_index_entry);
        assert!(!lines.iter().any(|line| line.sequence_footnote_zone));
        assert!(!model_line_should_be_marginalia(&lines[1]));
        assert!(!footnote_specialist_line_can_be_marginalia(&lines[5]));

        let mut hints = Vec::new();
        extend_page_contents_noise_hints(&mut hints, &lines);
        assert!(
            hints
                .iter()
                .any(|hint| hint.text == "See Magill, Roswell."
                    && hint.role == LiquidBlockRole::Noise)
        );
    }

    #[test]
    fn case_index_pages_do_not_start_footnote_marginalia() {
        let page = PageInfo::new(612.0, 792.0);
        let mut lines = vec![
            test_line(1, 0, 72.0, 700.0, 260.0, 710.0, "TABLE OF CASES"),
            test_line(
                1,
                1,
                72.0,
                680.0,
                260.0,
                690.0,
                "Atlanta Funtown, Inc. v. Crouch, 415",
            ),
            test_line(1, 2, 72.0, 660.0, 260.0, 670.0, "Baker v. State, 182"),
            test_line(1, 3, 72.0, 640.0, 260.0, 650.0, "Bagley v. Shortt, 60"),
            test_line(
                1,
                4,
                72.0,
                620.0,
                260.0,
                630.0,
                "Balkcom v. Jones County, 336",
            ),
            test_line(
                1,
                5,
                72.0,
                600.0,
                260.0,
                610.0,
                "Bowdish v. Johns Creek Associates, 155",
            ),
        ];
        mark_page_context_features(&mut lines);

        assert!(lines[1].page_contents_like);
        assert!(lines[1].contents_or_index_entry);
        assert!(model_noise_line_can_be_hidden(&page, &lines[1]));
        assert!(!starts_sequence_footnote_zone(&lines[1]));
        assert!(!model_line_should_be_marginalia(&lines[1]));
        assert!(!footnote_specialist_line_can_be_marginalia(&lines[1]));
    }

    #[test]
    fn split_table_of_contents_pages_are_hidden_as_noise() {
        let mut lines = vec![
            test_line(
                1,
                0,
                58.0,
                500.0,
                430.0,
                510.0,
                "THE JUDGE I KNEW ............................. Douglas E. Baker",
            ),
            test_line(1, 1, 440.0, 500.0, 452.0, 510.0, "9"),
            test_line(
                1,
                2,
                58.0,
                470.0,
                430.0,
                480.0,
                "THE UNITED STATES DISTRICT JUDGE ..... Honorable Lyle E. Strom",
            ),
            test_line(1, 3, 436.0, 470.0, 448.0, 480.0, "11"),
            test_line(
                1,
                4,
                58.0,
                440.0,
                260.0,
                450.0,
                "VOLUME ONE .................................",
            ),
            test_line(
                1,
                5,
                58.0,
                410.0,
                260.0,
                420.0,
                "VOLUME TWO ...............................",
            ),
        ];
        mark_page_context_features(&mut lines);

        assert!(lines.iter().all(|line| line.page_contents_like));
        let mut hints = Vec::new();
        extend_page_contents_noise_hints(&mut hints, &lines);
        assert_eq!(hints.len(), lines.len());
        assert!(hints.iter().all(|hint| hint.role == LiquidBlockRole::Noise));
    }

    #[test]
    fn mixed_front_matter_before_contents_region_is_not_hidden_as_noise() {
        let mut lines = vec![
            test_line(
                0,
                0,
                120.0,
                740.0,
                490.0,
                760.0,
                "THE DUTY TO READ THE UNREADABLE",
            ),
            test_line(0, 1, 230.0, 710.0, 380.0, 725.0, "URI BENOLIEL"),
            test_line(0, 2, 220.0, 690.0, 390.0, 705.0, "SHMUEL I. BECHER"),
            test_line(0, 3, 210.0, 650.0, 400.0, 662.0, "TABLE OF CONTENTS"),
            test_line(
                0,
                4,
                58.0,
                620.0,
                520.0,
                632.0,
                "INTRODUCTION ................................................................ 2257",
            ),
            test_line(
                0,
                5,
                58.0,
                595.0,
                520.0,
                607.0,
                "I. THEORETICAL BACKGROUND ............................................. 2260",
            ),
        ];
        mark_page_context_features(&mut lines);

        assert!(lines.iter().all(|line| line.page_contents_like));
        let mut hints = Vec::new();
        extend_page_contents_noise_hints(&mut hints, &lines);
        assert!(
            !hints
                .iter()
                .any(|hint| hint.text == "THE DUTY TO READ THE UNREADABLE")
        );
        assert!(!hints.iter().any(|hint| hint.text == "URI BENOLIEL"));
        assert!(hints
            .iter()
            .any(|hint| hint.text == "TABLE OF CONTENTS" && hint.role == LiquidBlockRole::Noise));
        assert!(hints.iter().any(
            |hint| hint.text.starts_with("INTRODUCTION") && hint.role == LiquidBlockRole::Noise
        ));
    }

    #[test]
    fn spaced_dot_leader_contents_pages_are_hidden_as_noise() {
        let mut lines = vec![
            test_line(2, 0, 210.0, 700.0, 400.0, 712.0, "TABLE OF CONTENTS"),
            test_line(
                2,
                1,
                72.0,
                650.0,
                420.0,
                662.0,
                "THE DORMANT COMMERCE CLAUSE:",
            ),
            test_line(
                2,
                2,
                72.0,
                632.0,
                520.0,
                644.0,
                "1824 TO 1945 . . . . James M. McGoldrick, Jr.",
            ),
            test_line(
                2,
                3,
                72.0,
                610.0,
                520.0,
                622.0,
                "THE MODERN RULE . . . . Jane Doe",
            ),
        ];
        mark_page_context_features(&mut lines);

        assert!(lines.iter().all(|line| line.page_contents_like));
        assert!(looks_like_dot_leader_contents_fragment(&lines[2].text));
        assert!(!lines.iter().any(model_line_should_be_marginalia));

        let mut hints = Vec::new();
        extend_page_contents_noise_hints(&mut hints, &lines);
        assert_eq!(hints.len(), lines.len());
        assert!(hints.iter().all(|hint| hint.role == LiquidBlockRole::Noise));
    }

    fn push_line(
        chars: &mut Vec<PageTextChar>,
        page: &PageInfo,
        y_from_top: f32,
        height: f32,
        text: &str,
    ) {
        let top = page.height - y_from_top;
        let bottom = top - height;
        let mut left = 72.0;
        for ch in text.chars() {
            chars.push(PageTextChar {
                ch,
                rect: Some(PdfRect::new(left, bottom, left + 5.0, top)),
                font_size: Some(height),
                bold: false,
                italic: false,
            });
            left += 5.0;
        }
        chars.push(PageTextChar {
            ch: '\n',
            rect: None,
            font_size: None,
            bold: false,
            italic: false,
        });
    }

    fn push_text_run(
        chars: &mut Vec<PageTextChar>,
        mut left: f32,
        bottom: f32,
        height: f32,
        font_size: f32,
        text: &str,
    ) {
        for ch in text.chars() {
            let width = if ch.is_whitespace() { 3.0 } else { 5.0 };
            chars.push(PageTextChar {
                ch,
                rect: Some(PdfRect::new(left, bottom, left + width, bottom + height)),
                font_size: Some(font_size),
                bold: false,
                italic: false,
            });
            left += width;
        }
    }

    fn test_line(
        page_index: usize,
        line_index: usize,
        left: f32,
        bottom: f32,
        right: f32,
        top: f32,
        text: &str,
    ) -> LayoutLine {
        LayoutLine {
            text: text.to_owned(),
            page_index,
            page_width: 612.0,
            page_height: 792.0,
            line_index,
            left,
            bottom,
            right,
            top,
            font_height: top - bottom,
            font_ratio_page: 1.0,
            font_ratio_page_ref: 1.0,
            font_ratio_doc: 1.0,
            bold: false,
            italic: false,
            centered: false,
            below_footnote_divider: false,
            distance_below_divider: 0.0,
            page_has_footnote_divider: false,
            sequence_footnote_zone: false,
            prev_line_present: false,
            prev_sequence_footnote_zone: false,
            prev_below_footnote_divider: false,
            prev_small_font: false,
            prev_note_marker: false,
            prev_legal_note_cue: false,
            next_line_present: false,
            next_sequence_footnote_zone: false,
            next_below_footnote_divider: false,
            next_small_font: false,
            next_note_marker: false,
            next_legal_note_cue: false,
            prev_y_gap_ratio: 0.0,
            prev_left_delta_ratio: 0.0,
            prev_font_delta_ratio: 0.0,
            next_y_gap_ratio: 0.0,
            next_left_delta_ratio: 0.0,
            next_font_delta_ratio: 0.0,
            body_left_delta_ratio: 0.0,
            width_to_body_ratio: 1.0,
            prev_gap_to_median_ratio: 1.0,
            next_gap_to_median_ratio: 1.0,
            signed_body_left_delta_ratio: 0.0,
            right_indent_ratio: 0.0,
            center_offset_ratio: 0.0,
            font_ratio_body: 1.0,
            max_internal_space_run: line_internal_space_run(text),
            space_density: line_space_density(text),
            leading_space_count: leading_space_count(text),
            trailing_space_count: trailing_space_count(text),
            body_column_like: false,
            narrow_measure_like: false,
            hanging_indent_like: false,
            vertically_isolated_like: false,
            heading_geometry_like: false,
            follows_hanging_note_marker: false,
            repeated_header_footer: false,
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
            page_contents_like: false,
            contents_or_index_entry: false,
        }
    }

    fn assert_token_count(tokens: &[String], token: &str, expected: usize) {
        let actual = tokens
            .iter()
            .filter(|candidate| candidate.as_str() == token)
            .count();
        assert_eq!(actual, expected, "token {token:?} in {tokens:?}");
    }
}
