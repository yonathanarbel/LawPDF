use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::ffi::{CStr, CString, OsStr, OsString};
use std::os::raw::{c_char, c_double, c_float, c_void};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crossbeam_channel::Sender;
use libloading::Library;
use serde::{Deserialize, Serialize};

use crate::layout_roles::{CALLOUT_END, CALLOUT_START};
use crate::liquid::{
    DeepLiquidSourceLine, DocumentProfileKind, DocumentProfileScore, LiquidBlock, LiquidBlockRole,
    LiquidBlockSourceLines, LiquidDocument, LiquidSourceLineRef, attach_footnote_links,
    should_preserve_terminal_hyphen,
};
use crate::liquidvision::{fill_document_features, liquidvision_enabled};
use crate::pdf_backend::PdfEngine;
use crate::settings::app_data_dir;

mod fast_cache;
mod runtime_status;
pub use fast_cache::load_fast_cached_liquid_mode2_document;
pub(crate) use fast_cache::save_fast_cached_lm2_document;
pub use runtime_status::run_lm2_runtime_status;

const LM2_SCHEMA_VERSION: &str =
    "liquidmode2-native-catboost-default-v6-runtime-lv-fill-multimarker-note-links-no-stack";
const LM2_D1_RUNTIME_ZEROSPEND_OVERLAY_VERSION: &str = "d1-zerospend-v3-no-ibid";
const LM2_D1_RUNTIME_POSTCUE_CITATION_NEXT1_OVERLAY_VERSION: &str =
    "d1-postcue-citation-next1-v2-narrow-cue";
const LM2_D1_RUNTIME_NEAR8_CUE_OVERLAY_VERSION: &str = "d1-near8-cue-v1-font085-near4";
const LM2_D1_RUNTIME_GEOMETRIC_ZONE_OVERLAY_VERSION: &str =
    "d1-geometric-zone-v3-cued-strict-font-cliff-percent-guard";
const LM2_D1_RUNTIME_WIDE_DIVIDER_GUARD_OVERLAY_VERSION: &str =
    "d1-wide-divider-guard-v3-page1-detector-small090-lower55-short42";
const LM2_D1_RUNTIME_FOOTER_ARTIFACT_OVERLAY_VERSION: &str =
    "d1-footer-artifact-v1-guarded-no-contact";
const LM2_FOOTNOTE_MONOTONE_OVERLAY_VERSION: &str = "footnote-monotone-v1-marker-context";
const LM2_FOOTNOTE_CARRYOVER_OVERLAY_VERSION: &str = "footnote-carryover-v1-open-prev-smallfont";
const LM2_ASSEMBLY_CACHE_VERSION: &str = "lm2-assembly-v9-inline-orphan-marker-attach";
const LM2_TABLE_FIGURE_ROUTER_OVERLAY_VERSION: &str = "table-figure-router-v4-default-on";
const LM2_PAGE_OBJECT_OVERLAY_VERSION: &str = "page-object-overlay-v1-guarded-ruled-path";
const LM2_PAGE_OBJECT_TUNED_OVERLAY_VERSION: &str =
    "page-object-tuned-overlay-v2-ruled-body-rescue-keep-preserve";
const LM2_NATIVE_CATBOOST_RUNTIME_DIR: &str = "profile-models/lm2-native-catboost-runtime";
const LM2_NATIVE_CATBOOST_MODEL_FILE: &str = "lm2-catboost-augmented-epoch51lv-relabels-tc.cbm";
const LM2_CONTEXT_TWOPASS_RUNTIME_DIR: &str = "profile-models/lm2-context-twopass-runtime";
const LM2_CONTEXT_TWOPASS_MODEL_FILE: &str = "lm2-context-twopass-hgb-v1.json";
const LM2_CONTEXT_TWOPASS_VERSION: &str = "context-twopass-hgb-v1-agent2-task30-foldnorm";
const LM2_NATIVE_CATBOOST_FLOAT_FEATURES: [&str; 116] = [
    "page_width",
    "page_height",
    "page_index",
    "page_index_norm",
    "x0_norm",
    "y0_norm",
    "x1_norm",
    "y1_norm",
    "width_norm",
    "height_norm",
    "center_x_norm",
    "center_y_norm",
    "left_margin_ratio",
    "right_margin_ratio",
    "indent_both",
    "margin_symmetry",
    "line_width_ratio",
    "indent_vs_body",
    "width_vs_body",
    "line_index",
    "line_index_norm",
    "font_size",
    "font_ratio_page",
    "font_ratio_doc",
    "doc_font_body_z",
    "doc_font_footnote_z",
    "doc_font_body_size",
    "doc_font_footnote_size",
    "doc_repeated_text_count",
    "doc_note_marker",
    "doc_note_marker_page_delta",
    "lines_from_doc_start",
    "prev4_dotleader_count",
    "prev4_spaced_dotleader_count",
    "prev4_strong_dotleader_count",
    "internal_space_run_max",
    "numeric_token_count",
    "percent_token_count",
    "char_count",
    "word_count",
    "alpha_count",
    "digit_count",
    "punct_count",
    "uppercase_ratio",
    "digit_ratio",
    "punct_ratio",
    "leading_whitespace_count",
    "trailing_punct_count",
    "liquidvision_score",
    "liquidvision_coverage",
    "liquidvision_region_area_norm",
    "liquidvision_page_region_count",
    "liquidvision_page_footnote_count",
    "liquidvision_page_table_figure_count",
    "liquidvision_footnote_score",
    "liquidvision_table_score",
    "liquidvision_figure_score",
    "liquidvision_body_score",
    "liquidvision_heading_score",
    "liquidvision_furniture_score",
    "liquidvision_frontmatter_score",
    "is_first_page",
    "is_first_two_pages",
    "front_matter_zone",
    "bold",
    "italic",
    "centered",
    "margin_centered",
    "is_block_indented",
    "prev_line_indented",
    "below_footnote_divider",
    "page_has_footnote_divider",
    "doc_repeated_edge_text",
    "doc_repeated_top_edge",
    "doc_repeated_bottom_edge",
    "doc_repeated_numeric_pattern",
    "doc_vertical_axis_like",
    "doc_vertical_numeric_axis_like",
    "doc_vertical_short_text_axis_like",
    "page_table_column_like",
    "table_numeric_cell_like",
    "doc_note_marker_first_on_page",
    "doc_note_marker_mid_sequence_page",
    "doc_note_marker_follows_previous_page",
    "starts_digit",
    "starts_numeric_note_marker",
    "starts_roman_marker",
    "starts_symbol_marker",
    "has_legal_note_cue",
    "has_dotleader",
    "has_long_dash_run",
    "prev_line_has_dotleader",
    "prev4_toc_leader_context",
    "has_large_internal_space_gap",
    "columnar_numeric_text_like",
    "page_number_like",
    "contains_page_word",
    "contains_do_not_delete",
    "short_numeric_body_fragment_like",
    "short_alpha_body_fragment_like",
    "year_header_furniture_like",
    "mostly_caps",
    "all_caps_short",
    "contains_section_symbol",
    "contains_citation_reporter",
    "liquidvision_has_region",
    "liquidvision_is_footnote",
    "liquidvision_is_table",
    "liquidvision_is_figure",
    "liquidvision_is_body",
    "liquidvision_is_heading",
    "liquidvision_is_furniture",
    "liquidvision_is_frontmatter",
    "liquidvision_routes_hide_noise",
    "liquidvision_routes_marginalia",
    "liquidvision_keep_veto",
];
const LM2_NATIVE_CATBOOST_CAT_FEATURES: [&str; 14] = [
    "page_zone_y",
    "page_zone_x",
    "width_bucket",
    "height_bucket",
    "font_ratio_page_bucket",
    "font_ratio_doc_bucket",
    "font_size_bucket",
    "line_position_bucket",
    "leading_marker_type",
    "first_token_shape",
    "terminal_punct",
    "page_parity",
    "liquidvision_class",
    "liquidvision_route",
];
const LM2_V25_D1_PAGE_OBJECT_TUNED_PRESET: &str = "v25-d1-sandwiched-note-start-wide-sandwich-postcue-citation-next1-near8cue-wide-divider-guard-page-object-tuned";
const LM2_PAGE_OBJECT_TUNED_DEFAULT_ENV: &str = "LAWPDF_LM2_PAGE_OBJECT_TUNED_DEFAULT";
const ACTIONS: [Lm2Action; 3] = [Lm2Action::Keep, Lm2Action::Marginalia, Lm2Action::HideNoise];

#[derive(Debug, Clone)]
pub struct LiquidMode2Request {
    pub document_epoch: u64,
    pub path: PathBuf,
    pub title: String,
    pub pages: Vec<String>,
    pub deep_source_lines: Vec<DeepLiquidSourceLine>,
    pub use_pymupdf_blocks: bool,
    pub use_pp_footnote_regions: bool,
    pub external_emissions_path: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct LiquidMode2Event {
    pub document_epoch: u64,
    pub path: PathBuf,
    pub complete: bool,
    pub preview_page_count: Option<usize>,
    pub result: Result<LiquidDocument, String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct LiquidMode2Timing {
    pub runtime_load_ms: f64,
    pub liquidvision_fill_ms: f64,
    pub feature_enrichment_ms: f64,
    pub model_decode_ms: f64,
    pub overlay_decode_ms: f64,
    pub assembly_ms: f64,
    pub total_ms: f64,
    pub cache_hit: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Lm2Action {
    Keep,
    Marginalia,
    HideNoise,
}

impl Lm2Action {
    fn as_str(self) -> &'static str {
        match self {
            Self::Keep => "keep",
            Self::Marginalia => "marginalia",
            Self::HideNoise => "hide_noise",
        }
    }

    fn index(self) -> usize {
        match self {
            Self::Keep => 0,
            Self::Marginalia => 1,
            Self::HideNoise => 2,
        }
    }
}

#[derive(Debug, Deserialize)]
struct Lm2Model {
    model_id: String,
    model_type: String,
    actions: Vec<String>,
    feature_dim: usize,
    bias: Vec<f64>,
    weights: Vec<Vec<f64>>,
    #[serde(default)]
    feature_schema: Option<Lm2FeatureSchema>,
    #[serde(default)]
    decoder_constants: Option<Lm2DecoderConstants>,
}

#[derive(Debug, Deserialize)]
struct Lm2NumericCatboostModel {
    #[serde(default)]
    schema_version: String,
    #[serde(default)]
    model_type: String,
    classes: Vec<String>,
    scale: f64,
    bias: Vec<f64>,
    features: Vec<Lm2NumericCatboostFeature>,
    trees: Vec<Lm2NumericCatboostTree>,
}

#[derive(Debug, Deserialize)]
struct Lm2NumericCatboostFeature {
    name: String,
}

#[derive(Debug, Deserialize)]
struct Lm2NumericCatboostTree {
    splits: Vec<Lm2NumericCatboostSplit>,
    leaf_values: Vec<f64>,
}

#[derive(Debug, Deserialize)]
struct Lm2NumericCatboostSplit {
    feature_index: usize,
    border: f64,
}

type CatboostHandle = c_void;
type CatboostCreateFn = unsafe extern "C" fn() -> *mut CatboostHandle;
type CatboostDeleteFn = unsafe extern "C" fn(*mut CatboostHandle);
type CatboostLoadFullModelFromFileFn =
    unsafe extern "C" fn(*mut CatboostHandle, *const c_char) -> bool;
type CatboostGetCountFn = unsafe extern "C" fn(*mut CatboostHandle) -> usize;
type CatboostGetErrorStringFn = unsafe extern "C" fn() -> *const c_char;
type CatboostCalcModelPredictionTextFn = unsafe extern "C" fn(
    *mut CatboostHandle,
    usize,
    *const *const c_float,
    usize,
    *const *const *const c_char,
    usize,
    *const *const *const c_char,
    usize,
    *mut c_double,
    usize,
) -> bool;

#[derive(Debug)]
struct Lm2NativeCatboostModel {
    _library: Library,
    handle: *mut CatboostHandle,
    delete_model: CatboostDeleteFn,
    calc_model_prediction_text: CatboostCalcModelPredictionTextFn,
    get_error_string: CatboostGetErrorStringFn,
    float_feature_count: usize,
    cat_feature_count: usize,
    text_feature_count: usize,
    dimensions_count: usize,
}

unsafe impl Send for Lm2NativeCatboostModel {}

#[derive(Debug, Default, Deserialize)]
struct Lm2FeatureSchema {
    #[serde(default)]
    doc_font_zscores: bool,
    #[serde(default)]
    repetition_fingerprints: bool,
    #[serde(default)]
    marker_continuity: bool,
}

#[derive(Debug, Deserialize)]
struct Lm2DecoderConstants {
    #[serde(default)]
    weights: HashMap<String, f64>,
}

#[derive(Debug)]
struct Lm2Runtime {
    model: Option<Lm2Model>,
    native_catboost_model: Option<Lm2NativeCatboostModel>,
    context_twopass_model: Option<Lm2ContextTwopassModel>,
    numeric_catboost_model: Option<Lm2NumericCatboostModel>,
    static_front_overlay: Option<Lm2StaticFrontOverlay>,
    model_label: String,
    load_warnings: Vec<String>,
    pp_priors: Option<Lm2PpPriorIndex>,
    pp_footnote_region_membership: bool,
    marker_decoder_prior: bool,
    small_font_decoder_prior: bool,
    small_font_sequence_prior: bool,
    anchored_marginalia_flow_guard: bool,
    body_preservation_guard: bool,
    action_neutral_blocksplit: bool,
    toc_overlay: bool,
    front_matter_guard: bool,
    marginalia_preservation_guard: bool,
    start_score_scale: f64,
    transition_score_scale: f64,
}

#[derive(Debug, Deserialize)]
struct Lm2ContextTwopassModelFile {
    schema_version: String,
    actions: Vec<String>,
    feature_count: usize,
    #[serde(default)]
    doc_to_fold: HashMap<String, usize>,
    #[serde(default)]
    unseen_doc_model: Option<String>,
    models: Vec<Lm2ContextTwopassHgbModel>,
}

#[derive(Debug, Deserialize)]
struct Lm2ContextTwopassHgbModel {
    name: String,
    baseline_prediction: Vec<f64>,
    trees: Vec<Vec<Vec<[f64; 7]>>>,
}

#[derive(Debug)]
struct Lm2ContextTwopassModel {
    schema_version: String,
    actions: Vec<String>,
    feature_count: usize,
    doc_to_fold: HashMap<String, usize>,
    unseen_doc_model: String,
    models: Vec<Lm2ContextTwopassHgbModel>,
}

#[derive(Debug, Clone, Default)]
struct Lm2ContextBlockMeta {
    block_action: Option<Lm2Action>,
    block_line_count: usize,
    block_char_count: usize,
    block_short_ratio: f64,
    block_numeric_ratio: f64,
    block_dotleader: f64,
    block_note_start_ratio: f64,
    block_edge_ratio: f64,
    block_axis_ratio: f64,
    block_footzone_ratio: f64,
    block_pos_norm: f64,
}

fn lm2_v25_d1_runtime_preset_enabled() -> bool {
    if lm2_v25_d1_page_object_tuned_default_enabled() {
        return true;
    }
    std::env::var("LAWPDF_LM2_RUNTIME_PRESET")
        .ok()
        .is_some_and(|value| {
            value.eq_ignore_ascii_case("v25-d1")
                || value.eq_ignore_ascii_case("v25-d1-zerospend")
                || value.eq_ignore_ascii_case("v25-d1-continuation")
                || value.eq_ignore_ascii_case("v25-d1-immediate-continuation")
                || value.eq_ignore_ascii_case("v25-d1-sandwiched-continuation")
                || value.eq_ignore_ascii_case("v25-d1-sandwiched-note-start")
                || value.eq_ignore_ascii_case("v25-d1-sandwiched-note-start-wide-sandwich")
                || value.eq_ignore_ascii_case("v25-d1-sandwiched-note-start-wide-sandwich-postcue")
                || value.eq_ignore_ascii_case(
                    "v25-d1-sandwiched-note-start-wide-sandwich-postcue-citation-next1",
                )
                || value.eq_ignore_ascii_case(
                    "v25-d1-sandwiched-note-start-wide-sandwich-postcue-citation-next1-near8cue",
                )
                || value.eq_ignore_ascii_case(
                    "v25-d1-sandwiched-note-start-wide-sandwich-postcue-citation-next1-near8cue-wide-divider-guard",
                )
                || lm2_runtime_preset_is_page_object_tuned(&value)
                || value.eq_ignore_ascii_case(
                    "v25-d1-sandwiched-note-start-wide-sandwich-postcue-citation-next1-near8cue-geo-zone",
                )
        })
}

fn lm2_v25_d1_continuation_runtime_preset_enabled() -> bool {
    std::env::var("LAWPDF_LM2_RUNTIME_PRESET")
        .ok()
        .is_some_and(|value| value.eq_ignore_ascii_case("v25-d1-continuation"))
}

fn lm2_v25_d1_immediate_continuation_runtime_preset_enabled() -> bool {
    std::env::var("LAWPDF_LM2_RUNTIME_PRESET")
        .ok()
        .is_some_and(|value| value.eq_ignore_ascii_case("v25-d1-immediate-continuation"))
}

fn lm2_v25_d1_sandwiched_continuation_runtime_preset_enabled() -> bool {
    std::env::var("LAWPDF_LM2_RUNTIME_PRESET")
        .ok()
        .is_some_and(|value| value.eq_ignore_ascii_case("v25-d1-sandwiched-continuation"))
}

fn lm2_v25_d1_sandwiched_note_start_runtime_preset_enabled() -> bool {
    std::env::var("LAWPDF_LM2_RUNTIME_PRESET")
        .ok()
        .is_some_and(|value| value.eq_ignore_ascii_case("v25-d1-sandwiched-note-start"))
}

fn lm2_v25_d1_wide_sandwich_runtime_preset_enabled() -> bool {
    if lm2_v25_d1_page_object_tuned_default_enabled() {
        return true;
    }
    std::env::var("LAWPDF_LM2_RUNTIME_PRESET")
        .ok()
        .is_some_and(|value| {
            value.eq_ignore_ascii_case("v25-d1-sandwiched-note-start-wide-sandwich")
                || value.eq_ignore_ascii_case("v25-d1-sandwiched-note-start-wide-sandwich-postcue")
                || value.eq_ignore_ascii_case(
                    "v25-d1-sandwiched-note-start-wide-sandwich-postcue-citation-next1",
                )
                || value.eq_ignore_ascii_case(
                    "v25-d1-sandwiched-note-start-wide-sandwich-postcue-citation-next1-near8cue",
                )
                || value.eq_ignore_ascii_case(
                    "v25-d1-sandwiched-note-start-wide-sandwich-postcue-citation-next1-near8cue-wide-divider-guard",
                )
                || lm2_runtime_preset_is_page_object_tuned(&value)
                || value.eq_ignore_ascii_case(
                    "v25-d1-sandwiched-note-start-wide-sandwich-postcue-citation-next1-near8cue-geo-zone",
                )
        })
}

fn lm2_v25_d1_post_wide_cue_runtime_preset_enabled() -> bool {
    if lm2_v25_d1_page_object_tuned_default_enabled() {
        return true;
    }
    std::env::var("LAWPDF_LM2_RUNTIME_PRESET")
        .ok()
        .is_some_and(|value| {
            value.eq_ignore_ascii_case("v25-d1-sandwiched-note-start-wide-sandwich-postcue")
                || value.eq_ignore_ascii_case(
                    "v25-d1-sandwiched-note-start-wide-sandwich-postcue-citation-next1",
                )
                || value.eq_ignore_ascii_case(
                    "v25-d1-sandwiched-note-start-wide-sandwich-postcue-citation-next1-near8cue",
                )
                || value.eq_ignore_ascii_case(
                    "v25-d1-sandwiched-note-start-wide-sandwich-postcue-citation-next1-near8cue-wide-divider-guard",
                )
                || lm2_runtime_preset_is_page_object_tuned(&value)
        })
}

fn lm2_v25_d1_postcue_citation_next1_runtime_preset_enabled() -> bool {
    if lm2_v25_d1_page_object_tuned_default_enabled() {
        return true;
    }
    std::env::var("LAWPDF_LM2_RUNTIME_PRESET")
        .ok()
        .is_some_and(|value| {
            value.eq_ignore_ascii_case(
                "v25-d1-sandwiched-note-start-wide-sandwich-postcue-citation-next1",
            ) || value.eq_ignore_ascii_case(
                "v25-d1-sandwiched-note-start-wide-sandwich-postcue-citation-next1-near8cue",
            ) || value.eq_ignore_ascii_case(
                "v25-d1-sandwiched-note-start-wide-sandwich-postcue-citation-next1-near8cue-wide-divider-guard",
            ) || lm2_runtime_preset_is_page_object_tuned(&value) || value.eq_ignore_ascii_case(
                "v25-d1-sandwiched-note-start-wide-sandwich-postcue-citation-next1-near8cue-geo-zone",
            )
        })
}

fn lm2_v25_d1_near8_cue_runtime_preset_enabled() -> bool {
    if lm2_v25_d1_page_object_tuned_default_enabled() {
        return true;
    }
    std::env::var("LAWPDF_LM2_RUNTIME_PRESET")
        .ok()
        .is_some_and(|value| {
            value.eq_ignore_ascii_case(
                "v25-d1-sandwiched-note-start-wide-sandwich-postcue-citation-next1-near8cue",
            ) || value.eq_ignore_ascii_case(
                "v25-d1-sandwiched-note-start-wide-sandwich-postcue-citation-next1-near8cue-wide-divider-guard",
            ) || lm2_runtime_preset_is_page_object_tuned(&value) || value.eq_ignore_ascii_case(
                "v25-d1-sandwiched-note-start-wide-sandwich-postcue-citation-next1-near8cue-geo-zone",
            )
        })
}

pub(crate) fn lm2_v25_d1_wide_divider_guard_runtime_preset_enabled() -> bool {
    if lm2_v25_d1_page_object_tuned_default_enabled() {
        return true;
    }
    std::env::var("LAWPDF_LM2_RUNTIME_PRESET")
        .ok()
        .is_some_and(|value| {
            value.eq_ignore_ascii_case(
                "v25-d1-sandwiched-note-start-wide-sandwich-postcue-citation-next1-near8cue-wide-divider-guard",
            ) || lm2_runtime_preset_is_page_object_tuned(&value)
        })
}

fn lm2_runtime_preset_is_page_object_tuned(value: &str) -> bool {
    value.eq_ignore_ascii_case(LM2_V25_D1_PAGE_OBJECT_TUNED_PRESET)
}

fn lm2_v25_d1_page_object_tuned_default_enabled() -> bool {
    !lm2_native_catboost_default_asset_available() && !falsey_env(LM2_PAGE_OBJECT_TUNED_DEFAULT_ENV)
}

fn lm2_v25_d1_page_object_tuned_runtime_preset_enabled() -> bool {
    lm2_v25_d1_page_object_tuned_default_enabled()
        || std::env::var("LAWPDF_LM2_RUNTIME_PRESET")
            .ok()
            .is_some_and(|value| lm2_runtime_preset_is_page_object_tuned(&value))
}

#[allow(dead_code)]
pub(crate) fn lm2_v25_tables_runtime_preset_enabled() -> bool {
    std::env::var("LAWPDF_LM2_RUNTIME_PRESET")
        .ok()
        .is_some_and(|value| {
            value.eq_ignore_ascii_case("v25-tables")
                || value.eq_ignore_ascii_case("v25-table-figure-router")
        })
}

fn lm2_v25_d1_geometric_zone_runtime_preset_enabled() -> bool {
    std::env::var("LAWPDF_LM2_RUNTIME_PRESET")
        .ok()
        .is_some_and(|value| {
            value.eq_ignore_ascii_case(
                "v25-d1-sandwiched-note-start-wide-sandwich-postcue-citation-next1-near8cue-geo-zone",
            ) || value.eq_ignore_ascii_case("v25-d1-geo-zone")
        })
}

#[derive(Debug)]
struct Lm2StaticFrontOverlay {
    source_label: String,
    roles_by_doc_line: HashMap<String, HashMap<String, LiquidBlockRole>>,
}

#[derive(Debug)]
struct Lm2PpPriorIndex {
    source: PathBuf,
    rows: HashMap<String, Lm2PpPrior>,
}

#[derive(Debug, Clone)]
struct Lm2PpPrior {
    role: String,
    label: String,
    score: f64,
}

#[derive(Debug, Deserialize)]
struct Lm2PpDraftRow {
    #[serde(default)]
    source_path: String,
    #[serde(default)]
    path: String,
    page_index: usize,
    line_index: usize,
    text: String,
    #[serde(default)]
    draft_action: Option<String>,
    #[serde(default)]
    pp_action: Option<String>,
    #[serde(default)]
    pp_role: Option<String>,
    #[serde(default)]
    pp_label: Option<String>,
    #[serde(default)]
    pp_score: Option<f64>,
}

#[derive(Debug, Serialize)]
struct Lm2PpDoclayoutRequest<'a> {
    schema_version: &'static str,
    document_path: &'a str,
    source_lines: &'a [DeepLiquidSourceLine],
    render_scale: f32,
    layout_model_name: &'static str,
    work_dir: &'a str,
}

#[derive(Debug, Deserialize)]
struct Lm2PpDoclayoutResponse {
    #[serde(default)]
    draft_row_count: usize,
    #[serde(default)]
    page_count: usize,
    #[serde(default)]
    detection_box_count: usize,
    #[serde(default)]
    warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
struct Lm2PymupdfGroupingRequest<'a> {
    schema_version: &'static str,
    source_signature: &'a str,
    document_path: &'a str,
    title: &'a str,
    use_detector_fallback: bool,
    source_lines: &'a [DeepLiquidSourceLine],
}

#[derive(Debug, Deserialize)]
struct Lm2PymupdfGroupingResponse {
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    warnings: Vec<String>,
    #[serde(default)]
    blocks: Vec<Lm2PymupdfGroupingBlock>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct Lm2PymupdfGroupingBlock {
    #[serde(default)]
    block_index: Option<usize>,
    #[serde(default)]
    page_index: Option<usize>,
    #[serde(default)]
    source: Option<String>,
    source_line_ids: Vec<String>,
}

pub fn spawn_liquid_mode2_job(request: LiquidMode2Request, tx: Sender<LiquidMode2Event>) {
    thread::spawn(move || {
        let document_epoch = request.document_epoch;
        let path = request.path.clone();
        let use_pymupdf_blocks = request.use_pymupdf_blocks;
        let use_pp_footnote_regions = request.use_pp_footnote_regions;
        if lm2_progressive_preview_enabled()
            && let Some((preview_request, preview_page_count)) =
                lm2_progressive_preview_request(&request)
            && let Ok(document) = prepare_liquid_mode2_document(preview_request)
        {
            let _ = tx.send(LiquidMode2Event {
                document_epoch,
                path: path.clone(),
                complete: false,
                preview_page_count: Some(preview_page_count),
                result: Ok(document),
            });
        }
        let result = prepare_liquid_mode2_document(request);
        if let Ok(document) = &result {
            let _ = save_fast_cached_lm2_document(
                &path,
                use_pymupdf_blocks,
                use_pp_footnote_regions,
                document,
            );
        }
        let _ = tx.send(LiquidMode2Event {
            document_epoch,
            path,
            complete: true,
            preview_page_count: None,
            result,
        });
    });
}

const LM2_PROGRESSIVE_PREVIEW_PAGES: usize = 4;

fn lm2_progressive_preview_enabled() -> bool {
    !falsey_env("LAWPDF_LM2_PROGRESSIVE_PREVIEW")
}

pub(crate) fn lm2_progressive_preview_request(
    request: &LiquidMode2Request,
) -> Option<(LiquidMode2Request, usize)> {
    if request.pages.len() <= LM2_PROGRESSIVE_PREVIEW_PAGES
        || request.external_emissions_path.is_some()
        || request.use_pp_footnote_regions
    {
        return None;
    }
    let mut preview = request.clone();
    preview.pages.truncate(LM2_PROGRESSIVE_PREVIEW_PAGES);
    preview
        .deep_source_lines
        .retain(|line| line.page_index < LM2_PROGRESSIVE_PREVIEW_PAGES);
    (!preview.deep_source_lines.is_empty()).then_some((preview, LM2_PROGRESSIVE_PREVIEW_PAGES))
}

#[derive(Debug, Deserialize)]
struct Lm2EvalExamplesFile {
    lines: Vec<Lm2EvalRow>,
}

#[derive(Debug, Deserialize)]
struct Lm2EvalLabelsFile {
    labels: Vec<Lm2EvalLabel>,
}

#[derive(Debug, Clone, Deserialize)]
struct Lm2EvalRow {
    path: String,
    page_index: usize,
    line_index: usize,
    text: String,
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    page_width: Option<f32>,
    #[serde(default)]
    page_height: Option<f32>,
    #[serde(default)]
    x0: Option<f32>,
    #[serde(default)]
    y0: Option<f32>,
    #[serde(default)]
    x1: Option<f32>,
    #[serde(default)]
    y1: Option<f32>,
    #[serde(default)]
    font_size: Option<f32>,
    #[serde(default)]
    font_ratio_page: Option<f32>,
    #[serde(default)]
    font_ratio_page_ref: Option<f32>,
    #[serde(default)]
    font_ratio_doc: Option<f32>,
    #[serde(default)]
    bold: Option<bool>,
    #[serde(default)]
    italic: Option<bool>,
    #[serde(default)]
    centered: Option<bool>,
    #[serde(default)]
    below_footnote_divider: Option<bool>,
    #[serde(default)]
    page_has_footnote_divider: Option<bool>,
    #[serde(default)]
    in_footnote_zone: Option<bool>,
    #[serde(default)]
    page_object_image_overlap_ratio: Option<f32>,
    #[serde(default)]
    page_object_image_hit_count: Option<u16>,
    #[serde(default)]
    page_object_path_stroke_near_line_count: Option<u16>,
    #[serde(default)]
    page_object_path_stroke_density_near_line: Option<f32>,
    #[serde(default)]
    page_object_thin_horizontal_near_line_count: Option<u16>,
    #[serde(default)]
    page_object_thin_vertical_near_line_count: Option<u16>,
    #[serde(default)]
    page_object_overlaps_image_bbox: Option<bool>,
    #[serde(default)]
    page_object_ruled_row_membership: Option<bool>,
    #[serde(default)]
    page_object_hide_candidate: Option<bool>,
    #[serde(default)]
    page_object_hide_candidate_guarded: Option<bool>,
    #[serde(default)]
    page_object_path15_candidate: Option<bool>,
    #[serde(default)]
    page_object_ruled_or_path8_candidate: Option<bool>,
    #[serde(default)]
    line_on_ruled_divider: Option<bool>,
    #[serde(default)]
    in_ruled_cell: Option<bool>,
    #[serde(default)]
    ruled_row_membership_exact: Option<bool>,
    #[serde(default)]
    dist_to_nearest_rule: Option<f32>,
}

#[derive(Debug, Deserialize)]
struct Lm2EvalLabel {
    path: String,
    page_index: usize,
    line_index: usize,
    text: String,
    role: String,
}

#[derive(Debug, Serialize)]
struct Lm2EvalReport {
    model_label: String,
    pp_prior_source: Option<String>,
    pp_footnote_region_membership: bool,
    external_emissions_input: Option<String>,
    examples_input: String,
    labels_input: String,
    total: usize,
    accuracy: f64,
    macro_f1: f64,
    per_action: Vec<Lm2EvalActionMetric>,
    confusion: [[usize; 3]; 3],
    matched_rows: usize,
    label_rows: usize,
    use_example_role_hints: bool,
    block_quality: Lm2BlockQualityMetrics,
}

#[derive(Debug, Serialize)]
struct Lm2EvalActionMetric {
    action: &'static str,
    support: usize,
    precision: f64,
    recall: f64,
    f1: f64,
}

#[derive(Debug)]
struct Lm2EvalItem {
    source: DeepLiquidSourceLine,
    actual: Lm2Action,
}

#[derive(Debug, Serialize)]
struct Lm2EvalDisagreement {
    path: String,
    page_index: usize,
    line_index: usize,
    actual: &'static str,
    predicted: &'static str,
    previous_text: Option<String>,
    text: String,
    next_text: Option<String>,
    y_bottom: f32,
    font_ratio_page: f32,
    font_ratio_doc: f32,
    doc_footnote_state: bool,
    doc_footnote_continuation: bool,
    doc_note_marker: u16,
    doc_note_marker_first_on_page: bool,
    doc_note_marker_mid_sequence_page: bool,
    doc_note_marker_follows_previous_page: bool,
    doc_note_marker_page_delta: i16,
    below_footnote_divider: bool,
    page_has_footnote_divider: bool,
}

#[derive(Debug, Serialize)]
struct Lm2BlockQualityMetrics {
    block_count: usize,
    marginalia_blocks: usize,
    marginalia_source_lines: usize,
    mean_lines_per_marginalia_block: f64,
    paragraph_blocks: usize,
    distinct_pages: usize,
    paragraphs_per_page: f64,
    hyphen_artifacts: usize,
    hyphen_artifacts_per_1000_blocks: f64,
}

#[derive(Debug, Default)]
struct Lm2BlockQualityAccumulator {
    block_count: usize,
    marginalia_blocks: usize,
    marginalia_source_lines: usize,
    paragraph_blocks: usize,
    evaluated_pages: usize,
    hyphen_artifacts: usize,
}

#[derive(Debug, Serialize)]
struct Lm2FeatureDumpReport {
    examples_input: String,
    feature_dim: usize,
    doc_font_zscores: bool,
    repetition_fingerprints: bool,
    marker_continuity: bool,
    rows: Vec<Lm2FeatureDumpRow>,
}

#[derive(Debug, Serialize)]
struct Lm2FeatureDumpRow {
    path: String,
    page_index: usize,
    line_index: usize,
    text: String,
    features: Vec<(usize, f64)>,
}

#[derive(Debug, Serialize)]
struct Lm2DecoderLatticeReport {
    schema_version: &'static str,
    model_label: String,
    examples_input: String,
    labels_input: String,
    matched_rows: usize,
    label_rows: usize,
    page_count: usize,
    line_count: usize,
    use_example_role_hints: bool,
    pages: Vec<Lm2DecoderLatticePage>,
}

#[derive(Debug, Serialize)]
struct Lm2DecoderLatticePage {
    path: String,
    page_index: usize,
    lines: Vec<Lm2DecoderLatticeLine>,
}

#[derive(Debug, Serialize)]
struct Lm2DecoderLatticeLine {
    line_index: usize,
    text: String,
    gold_action: &'static str,
    role_hint: Option<&'static str>,
    emission_scores_after_priors: BTreeMap<String, f64>,
    start_features: BTreeMap<String, f64>,
    start_scores: BTreeMap<String, f64>,
    transition_features_from_previous: Option<BTreeMap<String, f64>>,
    transition_scores_from_previous: Option<BTreeMap<String, f64>>,
    y_bottom_ratio: f32,
    font_ratio_page: f32,
    font_ratio_doc: f32,
    doc_footnote_state: bool,
    doc_footnote_continuation: bool,
    doc_note_marker: u16,
    doc_note_marker_first_on_page: bool,
    doc_note_marker_mid_sequence_page: bool,
    doc_note_marker_follows_previous_page: bool,
    doc_note_marker_page_delta: i16,
    below_footnote_divider: bool,
    page_has_footnote_divider: bool,
}

#[derive(Debug, Deserialize)]
struct Lm2ExternalEmissionsFile {
    #[serde(default)]
    schema_version: Option<String>,
    #[serde(default)]
    model_label: Option<String>,
    pages: Vec<Lm2ExternalEmissionsPage>,
}

#[derive(Debug, Deserialize)]
struct Lm2ExternalEmissionsPage {
    path: String,
    page_index: usize,
    lines: Vec<Lm2ExternalEmissionsLine>,
}

#[derive(Debug, Deserialize)]
struct Lm2ExternalEmissionsLine {
    line_index: usize,
    text: String,
    emission_scores_after_priors: BTreeMap<String, f64>,
}

#[derive(Debug)]
struct Lm2ExternalEmissions {
    source_path: PathBuf,
    schema_version: Option<String>,
    model_label: Option<String>,
    scores_by_key: HashMap<String, [f64; 3]>,
    basename_scores_by_key: HashMap<String, [f64; 3]>,
}

impl Lm2ExternalEmissions {
    fn load(path: &Path) -> Result<Self, String> {
        let input: Lm2ExternalEmissionsFile = read_json_file(path)?;
        let mut scores_by_key = HashMap::new();
        let mut basename_scores_by_key = HashMap::new();
        let mut ambiguous_basename_keys = HashSet::new();
        let mut rows = 0usize;
        for page in input.pages {
            for line in page.lines {
                let scores = parse_external_action_scores(
                    &line.emission_scores_after_priors,
                    &page.path,
                    page.page_index,
                    line.line_index,
                )?;
                let path_key = external_full_path_key(&page.path);
                let key = eval_key(&path_key, page.page_index, line.line_index, &line.text);
                scores_by_key.entry(key).or_insert(scores);
                if let Some(basename_key) = external_basename_key(&page.path) {
                    let key = eval_key(&basename_key, page.page_index, line.line_index, &line.text);
                    if let Some(previous) = basename_scores_by_key.insert(key.clone(), scores)
                        && previous != scores
                    {
                        ambiguous_basename_keys.insert(key);
                    }
                }
                rows += 1;
            }
        }
        for key in ambiguous_basename_keys {
            basename_scores_by_key.remove(&key);
        }
        if rows == 0 {
            return Err(format!(
                "external emissions file has no rows: {}",
                path.display()
            ));
        }
        Ok(Self {
            source_path: path.to_path_buf(),
            schema_version: input.schema_version,
            model_label: input.model_label,
            scores_by_key,
            basename_scores_by_key,
        })
    }

    fn page_scores(
        &self,
        document_path: &Path,
        lines: &[DeepLiquidSourceLine],
    ) -> Result<Vec<[f64; 3]>, String> {
        let path_keys = external_document_path_keys(document_path);
        lines
            .iter()
            .map(|line| {
                for path_key in &path_keys {
                    let key = eval_key(path_key, line.page_index, line.line_index, &line.text);
                    if let Some(scores) = self.scores_by_key.get(&key) {
                        return Ok(*scores);
                    }
                }
                if let Some(basename_key) = document_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(external_full_path_key)
                {
                    let key = eval_key(&basename_key, line.page_index, line.line_index, &line.text);
                    if let Some(scores) = self.basename_scores_by_key.get(&key) {
                        return Ok(*scores);
                    }
                }
                Err(format!(
                    "missing external emissions for {} page {} line {} text {:?} in {}",
                    document_path.display(),
                    line.page_index,
                    line.line_index,
                    collapse_whitespace(&line.text),
                    self.source_path.display()
                ))
            })
            .collect()
    }

    fn source_label(&self) -> String {
        let model = self.model_label.as_deref().unwrap_or("unknown-model");
        let schema = self.schema_version.as_deref().unwrap_or("unknown-schema");
        format!("{} ({schema}, {model})", self.source_path.display())
    }
}

fn parse_external_action_scores(
    scores: &BTreeMap<String, f64>,
    path: &str,
    page_index: usize,
    line_index: usize,
) -> Result<[f64; 3], String> {
    let mut parsed = [0.0; 3];
    for action in ACTIONS {
        let Some(value) = scores.get(action.as_str()) else {
            return Err(format!(
                "external emissions row missing {} score for {} page {} line {}",
                action.as_str(),
                path,
                page_index,
                line_index
            ));
        };
        if !value.is_finite() {
            return Err(format!(
                "external emissions row has non-finite {} score for {} page {} line {}",
                action.as_str(),
                path,
                page_index,
                line_index
            ));
        }
        parsed[action.index()] = *value;
    }
    Ok(parsed)
}

fn external_document_path_keys(path: &Path) -> Vec<String> {
    let mut keys = vec![external_full_path_key(&path.display().to_string())];
    if let Ok(canonical) = std::fs::canonicalize(path) {
        keys.push(external_full_path_key(&canonical.display().to_string()));
    }
    keys.sort();
    keys.dedup();
    keys
}

fn external_full_path_key(path: &str) -> String {
    path.replace('\\', "/").to_ascii_lowercase()
}

fn external_basename_key(path: &str) -> Option<String> {
    let normalized = external_full_path_key(path);
    normalized
        .rsplit('/')
        .next()
        .filter(|file_name| !file_name.is_empty())
        .map(str::to_owned)
}

#[derive(Debug, Deserialize)]
struct Lm2DraftInput {
    documents: Vec<Lm2DraftInputDocument>,
}

#[derive(Debug, Deserialize)]
struct Lm2DraftInputDocument {
    path: String,
    #[serde(default)]
    selection_manifest_index: Option<usize>,
    #[serde(default)]
    selection_primary_stratum: Option<String>,
    #[serde(default)]
    selection_stratum_tags: Vec<String>,
    #[serde(default)]
    source_lines: Vec<DeepLiquidSourceLine>,
}

#[derive(Debug, Serialize)]
struct Lm2SourceSmokeReport {
    app_version: &'static str,
    timestamp_unix_secs: u64,
    document_count: usize,
    failures: usize,
    documents: Vec<Lm2SourceSmokeDocument>,
}

#[derive(Debug, Serialize)]
struct Lm2SourceSmokeDocument {
    path: String,
    source_lines: Vec<DeepLiquidSourceLine>,
    block_source_lines: Vec<LiquidBlockSourceLines>,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct Lm2DraftReport {
    schema_version: &'static str,
    model_label: String,
    pp_prior_source: Option<String>,
    pp_footnote_region_membership: bool,
    source_input: String,
    document_count: usize,
    row_count: usize,
    rows: Vec<Lm2DraftRow>,
}

#[derive(Debug, Serialize)]
struct Lm2DraftRow {
    path: String,
    selection_manifest_index: Option<usize>,
    selection_primary_stratum: Option<String>,
    selection_stratum_tags: Vec<String>,
    line_id: String,
    page_index: usize,
    line_index: usize,
    text: String,
    lm2_action: &'static str,
    lm1_hint_action: &'static str,
    role_hint: Option<&'static str>,
    y_bottom_ratio: f32,
    font_ratio_page: f32,
    font_ratio_doc: f32,
    doc_footnote_state: bool,
    doc_footnote_continuation: bool,
    doc_note_marker: u16,
    doc_note_marker_first_on_page: bool,
    doc_note_marker_mid_sequence_page: bool,
    doc_note_marker_follows_previous_page: bool,
    doc_note_marker_page_delta: i16,
    below_footnote_divider: bool,
    page_has_footnote_divider: bool,
}

pub fn run_lm2_eval(args: impl IntoIterator<Item = OsString>) -> Result<(), String> {
    let mut examples_input = PathBuf::from(
        "training-data/layout-role-core/lawpdf-layout-role-examples-chandra-expanded-fast-20260604.json",
    );
    let mut labels_input =
        PathBuf::from("training-data/layout-role-core/lawpdf-latest-labels-v3-holdout.json");
    let mut output_path: Option<PathBuf> = None;
    let mut disagreements_output_path: Option<PathBuf> = None;
    let mut disagreements_limit = 100usize;
    let mut use_example_role_hints = false;
    let mut external_emissions_path: Option<PathBuf> = None;
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        if arg == OsStr::new("--lm2-eval") {
            continue;
        }
        if arg == OsStr::new("--examples-input") {
            examples_input = args
                .next()
                .map(PathBuf::from)
                .ok_or_else(|| "--examples-input needs a path".to_owned())?;
            continue;
        }
        if arg == OsStr::new("--labels") {
            labels_input = args
                .next()
                .map(PathBuf::from)
                .ok_or_else(|| "--labels needs a path".to_owned())?;
            continue;
        }
        if arg == OsStr::new("--output") {
            output_path = Some(
                args.next()
                    .map(PathBuf::from)
                    .ok_or_else(|| "--output needs a path".to_owned())?,
            );
            continue;
        }
        if arg == OsStr::new("--disagreements-output") {
            disagreements_output_path = Some(
                args.next()
                    .map(PathBuf::from)
                    .ok_or_else(|| "--disagreements-output needs a path".to_owned())?,
            );
            continue;
        }
        if arg == OsStr::new("--disagreements-limit") {
            let value = args
                .next()
                .ok_or_else(|| "--disagreements-limit needs a value".to_owned())?;
            disagreements_limit = value
                .to_string_lossy()
                .parse::<usize>()
                .map_err(|_| "--disagreements-limit must be a non-negative integer".to_owned())?;
            continue;
        }
        if arg == OsStr::new("--lm2-external-emissions")
            || arg == OsStr::new("--external-emissions")
        {
            external_emissions_path = Some(
                args.next()
                    .map(PathBuf::from)
                    .ok_or_else(|| "--lm2-external-emissions needs a path".to_owned())?,
            );
            continue;
        }
        if arg == OsStr::new("--use-example-role-hints") {
            use_example_role_hints = true;
            continue;
        }
        if arg.to_string_lossy().starts_with("--") {
            return Err(format!(
                "unknown LM2 eval argument: {}",
                arg.to_string_lossy()
            ));
        }
    }

    let runtime = Lm2Runtime::load();
    let external_emissions = external_emissions_path
        .as_deref()
        .map(Lm2ExternalEmissions::load)
        .transpose()?;
    let examples: Lm2EvalExamplesFile = read_json_file(&examples_input)?;
    let labels: Lm2EvalLabelsFile = read_json_file(&labels_input)?;
    let mut label_by_key = HashMap::new();
    for label in &labels.labels {
        label_by_key.insert(
            eval_key(&label.path, label.page_index, label.line_index, &label.text),
            label,
        );
    }

    let mut groups: BTreeMap<(String, usize), Vec<Lm2EvalItem>> = BTreeMap::new();
    let mut matched_rows = 0usize;
    for (row, mut source) in eval_sources_for_rows(examples.lines, use_example_role_hints) {
        let Some(label) = label_by_key.get(&eval_key(
            &row.path,
            row.page_index,
            row.line_index,
            &row.text,
        )) else {
            continue;
        };
        annotate_pp_prior(&runtime, &row.path, &mut source);
        matched_rows += 1;
        groups
            .entry((row.path.to_ascii_lowercase(), row.page_index))
            .or_default()
            .push(Lm2EvalItem {
                source,
                actual: action_for_role_name(&label.role),
            });
    }

    let mut confusion = [[0usize; 3]; 3];
    let mut block_quality = Lm2BlockQualityAccumulator::default();
    let mut disagreements = Vec::new();
    for ((path, page_index), mut items) in groups {
        items.sort_by_key(|item| item.source.line_index);
        let sources = items
            .iter()
            .map(|item| item.source.clone())
            .collect::<Vec<_>>();
        let external_scores = external_emissions
            .as_ref()
            .map(|external| external.page_scores(Path::new(&path), &sources))
            .transpose()?;
        let decoded = decode_page_with_emissions(&runtime, &sources, external_scores.as_deref());
        for (index, (item, (_, predicted))) in items.iter().zip(decoded.iter()).enumerate() {
            confusion[item.actual.index()][predicted.index()] += 1;
            if item.actual != *predicted {
                disagreements.push(Lm2EvalDisagreement {
                    path: path.clone(),
                    page_index,
                    line_index: item.source.line_index,
                    actual: item.actual.as_str(),
                    predicted: predicted.as_str(),
                    previous_text: index
                        .checked_sub(1)
                        .and_then(|previous| items.get(previous))
                        .map(|previous| previous.source.text.clone()),
                    text: item.source.text.clone(),
                    next_text: items.get(index + 1).map(|next| next.source.text.clone()),
                    y_bottom: item.source.bottom / item.source.page_height.max(1.0),
                    font_ratio_page: item.source.font_ratio_page,
                    font_ratio_doc: item.source.font_ratio_doc,
                    doc_footnote_state: item.source.doc_footnote_state,
                    doc_footnote_continuation: item.source.doc_footnote_continuation,
                    doc_note_marker: item.source.doc_note_marker,
                    doc_note_marker_first_on_page: item.source.doc_note_marker_first_on_page,
                    doc_note_marker_mid_sequence_page: item
                        .source
                        .doc_note_marker_mid_sequence_page,
                    doc_note_marker_follows_previous_page: item
                        .source
                        .doc_note_marker_follows_previous_page,
                    doc_note_marker_page_delta: item.source.doc_note_marker_page_delta,
                    below_footnote_divider: item.source.below_footnote_divider,
                    page_has_footnote_divider: item.source.page_has_footnote_divider,
                });
            }
        }
        block_quality.add_page(&decoded);
    }
    if let Some(path) = disagreements_output_path {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let mut jsonl = String::new();
        for row in disagreements.iter().take(disagreements_limit) {
            jsonl.push_str(&serde_json::to_string(row).map_err(|error| error.to_string())?);
            jsonl.push('\n');
        }
        std::fs::write(&path, jsonl).map_err(|error| error.to_string())?;
    }
    let pp_prior_source = runtime.pp_prior_source();
    let report = lm2_eval_report(
        runtime.model_label,
        pp_prior_source,
        runtime.pp_footnote_region_membership,
        external_emissions_path.as_deref(),
        &examples_input,
        &labels_input,
        confusion,
        matched_rows,
        labels.labels.len(),
        use_example_role_hints,
        block_quality.finish(),
    );
    let json = serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?;
    if let Some(path) = output_path {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        std::fs::write(&path, json).map_err(|error| error.to_string())?;
    } else {
        println!("{json}");
    }
    Ok(())
}

pub fn run_lm2_feature_dump(args: impl IntoIterator<Item = OsString>) -> Result<(), String> {
    let mut examples_input = PathBuf::from(
        "training-data/layout-role-core/lawpdf-layout-role-examples-chandra-expanded-fast-20260604.json",
    );
    let mut output_path: Option<PathBuf> = None;
    let mut limit = 20usize;
    let mut feature_dim = 32768usize;
    let mut doc_font_zscores = false;
    let mut repetition_fingerprints = false;
    let mut marker_continuity = false;
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        if arg == OsStr::new("--dump-lm2-features") {
            continue;
        }
        if arg == OsStr::new("--examples-input") {
            examples_input = args
                .next()
                .map(PathBuf::from)
                .ok_or_else(|| "--examples-input needs a path".to_owned())?;
            continue;
        }
        if arg == OsStr::new("--output") {
            output_path = Some(
                args.next()
                    .map(PathBuf::from)
                    .ok_or_else(|| "--output needs a path".to_owned())?,
            );
            continue;
        }
        if arg == OsStr::new("--limit") {
            let value = args
                .next()
                .ok_or_else(|| "--limit needs an integer".to_owned())?;
            limit = value
                .to_string_lossy()
                .parse::<usize>()
                .map_err(|error| format!("invalid --limit: {error}"))?;
            continue;
        }
        if arg == OsStr::new("--feature-dim") {
            let value = args
                .next()
                .ok_or_else(|| "--feature-dim needs an integer".to_owned())?;
            feature_dim = value
                .to_string_lossy()
                .parse::<usize>()
                .map_err(|error| format!("invalid --feature-dim: {error}"))?;
            continue;
        }
        if arg == OsStr::new("--enable-doc-font-features") {
            doc_font_zscores = true;
            continue;
        }
        if arg == OsStr::new("--enable-repetition-features") {
            repetition_fingerprints = true;
            continue;
        }
        if arg == OsStr::new("--enable-marker-continuity-features") {
            marker_continuity = true;
            continue;
        }
        if arg.to_string_lossy().starts_with("--") {
            return Err(format!(
                "unknown LM2 feature dump argument: {}",
                arg.to_string_lossy()
            ));
        }
    }

    let examples: Lm2EvalExamplesFile = read_json_file(&examples_input)?;
    let rows = eval_sources_for_rows(examples.lines, false)
        .into_iter()
        .take(limit)
        .map(|(row, source)| Lm2FeatureDumpRow {
            path: row.path.clone(),
            page_index: row.page_index,
            line_index: row.line_index,
            text: row.text.clone(),
            features: lm2_features(
                &source,
                feature_dim,
                doc_font_zscores,
                repetition_fingerprints,
                marker_continuity,
            ),
        })
        .collect::<Vec<_>>();
    let report = Lm2FeatureDumpReport {
        examples_input: examples_input.display().to_string(),
        feature_dim,
        doc_font_zscores,
        repetition_fingerprints,
        marker_continuity,
        rows,
    };
    let json = serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?;
    if let Some(path) = output_path {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        std::fs::write(path, json).map_err(|error| error.to_string())?;
    } else {
        println!("{json}");
    }
    Ok(())
}

pub fn run_lm2_decoder_lattice_dump(
    args: impl IntoIterator<Item = OsString>,
) -> Result<(), String> {
    let mut examples_input = PathBuf::from(
        "training-data/layout-role-core/lawpdf-layout-role-examples-chandra-expanded-fast-20260604.json",
    );
    let mut labels_input =
        PathBuf::from("training-data/layout-role-core/lawpdf-latest-labels-v3-holdout.json");
    let mut output_path: Option<PathBuf> = None;
    let mut use_example_role_hints = false;
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        if arg == OsStr::new("--dump-lm2-decoder-lattice") {
            continue;
        }
        if arg == OsStr::new("--examples-input") {
            examples_input = args
                .next()
                .map(PathBuf::from)
                .ok_or_else(|| "--examples-input needs a path".to_owned())?;
            continue;
        }
        if arg == OsStr::new("--labels") {
            labels_input = args
                .next()
                .map(PathBuf::from)
                .ok_or_else(|| "--labels needs a path".to_owned())?;
            continue;
        }
        if arg == OsStr::new("--output") {
            output_path = Some(
                args.next()
                    .map(PathBuf::from)
                    .ok_or_else(|| "--output needs a path".to_owned())?,
            );
            continue;
        }
        if arg == OsStr::new("--use-example-role-hints") {
            use_example_role_hints = true;
            continue;
        }
        if arg.to_string_lossy().starts_with("--") {
            return Err(format!(
                "unknown LM2 decoder lattice dump argument: {}",
                arg.to_string_lossy()
            ));
        }
    }

    let runtime = Lm2Runtime::load();
    let examples: Lm2EvalExamplesFile = read_json_file(&examples_input)?;
    let labels: Lm2EvalLabelsFile = read_json_file(&labels_input)?;
    let mut label_by_key = HashMap::new();
    for label in &labels.labels {
        label_by_key.insert(
            eval_key(&label.path, label.page_index, label.line_index, &label.text),
            label,
        );
    }

    let mut groups: BTreeMap<(String, usize), Vec<Lm2EvalItem>> = BTreeMap::new();
    let mut matched_rows = 0usize;
    for (row, source) in eval_sources_for_rows(examples.lines, use_example_role_hints) {
        let Some(label) = label_by_key.get(&eval_key(
            &row.path,
            row.page_index,
            row.line_index,
            &row.text,
        )) else {
            continue;
        };
        matched_rows += 1;
        groups
            .entry((row.path.to_ascii_lowercase(), row.page_index))
            .or_default()
            .push(Lm2EvalItem {
                source,
                actual: action_for_role_name(&label.role),
            });
    }

    let mut pages = Vec::new();
    let mut line_count = 0usize;
    for ((path, page_index), mut items) in groups {
        items.sort_by_key(|item| item.source.line_index);
        let mut lines = Vec::with_capacity(items.len());
        for (index, item) in items.iter().enumerate() {
            let source = &item.source;
            let previous = index
                .checked_sub(1)
                .and_then(|previous| items.get(previous));
            lines.push(Lm2DecoderLatticeLine {
                line_index: source.line_index,
                text: source.text.clone(),
                gold_action: item.actual.as_str(),
                role_hint: source.role_hint.map(LiquidBlockRole::prompt_name),
                emission_scores_after_priors: action_scores_map(runtime.emission_scores(source)),
                start_features: start_feature_map(source),
                start_scores: start_scores_map(source.role_hint, runtime.start_score_scale),
                transition_features_from_previous: previous
                    .map(|previous| transition_feature_map(&previous.source, source)),
                transition_scores_from_previous: previous.map(|previous| {
                    transition_scores_map(&previous.source, source, runtime.transition_score_scale)
                }),
                y_bottom_ratio: source.bottom / source.page_height.max(1.0),
                font_ratio_page: source.font_ratio_page,
                font_ratio_doc: source.font_ratio_doc,
                doc_footnote_state: source.doc_footnote_state,
                doc_footnote_continuation: source.doc_footnote_continuation,
                doc_note_marker: source.doc_note_marker,
                doc_note_marker_first_on_page: source.doc_note_marker_first_on_page,
                doc_note_marker_mid_sequence_page: source.doc_note_marker_mid_sequence_page,
                doc_note_marker_follows_previous_page: source.doc_note_marker_follows_previous_page,
                doc_note_marker_page_delta: source.doc_note_marker_page_delta,
                below_footnote_divider: source.below_footnote_divider,
                page_has_footnote_divider: source.page_has_footnote_divider,
            });
        }
        line_count += lines.len();
        pages.push(Lm2DecoderLatticePage {
            path,
            page_index,
            lines,
        });
    }

    let report = Lm2DecoderLatticeReport {
        schema_version: "lm2-decoder-lattice-v1",
        model_label: runtime.model_label,
        examples_input: examples_input.display().to_string(),
        labels_input: labels_input.display().to_string(),
        matched_rows,
        label_rows: labels.labels.len(),
        page_count: pages.len(),
        line_count,
        use_example_role_hints,
        pages,
    };
    let json = serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?;
    if let Some(path) = output_path {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        std::fs::write(path, json).map_err(|error| error.to_string())?;
    } else {
        println!("{json}");
    }
    Ok(())
}

pub fn run_lm2_draft(args: impl IntoIterator<Item = OsString>) -> Result<(), String> {
    let mut input_path = PathBuf::from("eval/benchmark-v2/extracted-lines-lm2-source.json");
    let mut output_path: Option<PathBuf> = None;
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        if arg == OsStr::new("--lm2-draft") {
            continue;
        }
        if arg == OsStr::new("--input") {
            input_path = args
                .next()
                .map(PathBuf::from)
                .ok_or_else(|| "--input needs a path".to_owned())?;
            continue;
        }
        if arg == OsStr::new("--output") {
            output_path = Some(
                args.next()
                    .map(PathBuf::from)
                    .ok_or_else(|| "--output needs a path".to_owned())?,
            );
            continue;
        }
        if arg.to_string_lossy().starts_with("--") {
            return Err(format!(
                "unknown LM2 draft argument: {}",
                arg.to_string_lossy()
            ));
        }
    }

    reject_label_like_path(&input_path)?;
    let runtime = Lm2Runtime::load();
    let input: Lm2DraftInput = read_json_file(&input_path)?;
    let mut rows = Vec::new();
    for document in input.documents {
        let mut lines = document.source_lines;
        lines.sort_by_key(|line| (line.page_index, line.line_index));
        annotate_pp_priors_for_lines(&runtime, &document.path, &mut lines);
        enrich_lm2_document_features(&mut lines);
        let decoded = decode_pages(&runtime, &lines);
        for (line, lm2_action) in decoded {
            let lm1_hint_action = line.role_hint.map(role_action).unwrap_or(Lm2Action::Keep);
            rows.push(Lm2DraftRow {
                path: document.path.clone(),
                selection_manifest_index: document.selection_manifest_index,
                selection_primary_stratum: document.selection_primary_stratum.clone(),
                selection_stratum_tags: document.selection_stratum_tags.clone(),
                line_id: line.id.clone(),
                page_index: line.page_index,
                line_index: line.line_index,
                text: line.text.clone(),
                lm2_action: lm2_action.as_str(),
                lm1_hint_action: lm1_hint_action.as_str(),
                role_hint: line.role_hint.map(LiquidBlockRole::prompt_name),
                y_bottom_ratio: line.bottom / line.page_height.max(1.0),
                font_ratio_page: line.font_ratio_page,
                font_ratio_doc: line.font_ratio_doc,
                doc_footnote_state: line.doc_footnote_state,
                doc_footnote_continuation: line.doc_footnote_continuation,
                doc_note_marker: line.doc_note_marker,
                doc_note_marker_first_on_page: line.doc_note_marker_first_on_page,
                doc_note_marker_mid_sequence_page: line.doc_note_marker_mid_sequence_page,
                doc_note_marker_follows_previous_page: line.doc_note_marker_follows_previous_page,
                doc_note_marker_page_delta: line.doc_note_marker_page_delta,
                below_footnote_divider: line.below_footnote_divider,
                page_has_footnote_divider: line.page_has_footnote_divider,
            });
        }
    }

    let pp_prior_source = runtime.pp_prior_source();
    let report = Lm2DraftReport {
        schema_version: "lm2-benchmark-draft-v1",
        model_label: runtime.model_label,
        pp_prior_source,
        pp_footnote_region_membership: runtime.pp_footnote_region_membership,
        source_input: input_path.display().to_string(),
        document_count: rows
            .iter()
            .map(|row| row.path.as_str())
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        row_count: rows.len(),
        rows,
    };
    let json = serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?;
    if let Some(path) = output_path {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        std::fs::write(&path, json).map_err(|error| error.to_string())?;
    } else {
        println!("{json}");
    }
    Ok(())
}

pub fn run_lm2_source_smoke(args: impl IntoIterator<Item = OsString>) -> Result<(), String> {
    let mut input_path = PathBuf::from("eval/benchmark-v2/extracted-lines-lm2-source.json");
    let mut output_path: Option<PathBuf> = None;
    let mut external_emissions_path: Option<PathBuf> = None;
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        if arg == OsStr::new("--lm2-source-smoke") {
            continue;
        }
        if arg == OsStr::new("--input") {
            input_path = args
                .next()
                .map(PathBuf::from)
                .ok_or_else(|| "--input needs a path".to_owned())?;
            continue;
        }
        if arg == OsStr::new("--output") {
            output_path = Some(
                args.next()
                    .map(PathBuf::from)
                    .ok_or_else(|| "--output needs a path".to_owned())?,
            );
            continue;
        }
        if arg == OsStr::new("--lm2-external-emissions")
            || arg == OsStr::new("--external-emissions")
        {
            external_emissions_path = Some(
                args.next()
                    .map(PathBuf::from)
                    .ok_or_else(|| "--lm2-external-emissions needs a path".to_owned())?,
            );
            continue;
        }
        if arg.to_string_lossy().starts_with("--") {
            return Err(format!(
                "unknown LM2 source smoke argument: {}",
                arg.to_string_lossy()
            ));
        }
    }

    let input: Lm2DraftInput = read_json_file(&input_path)?;
    let mut documents = Vec::with_capacity(input.documents.len());
    for document in input.documents {
        let source_lines = document
            .source_lines
            .iter()
            .filter(|line| !line.text.trim().is_empty())
            .cloned()
            .collect::<Vec<_>>();
        let request = LiquidMode2Request {
            document_epoch: 0,
            path: PathBuf::from(&document.path),
            title: document.path.clone(),
            pages: Vec::new(),
            deep_source_lines: source_lines.clone(),
            use_pymupdf_blocks: false,
            use_pp_footnote_regions: false,
            external_emissions_path: external_emissions_path.clone(),
        };
        match prepare_liquid_mode2_document(request) {
            Ok(liquid) => documents.push(Lm2SourceSmokeDocument {
                path: document.path,
                source_lines,
                block_source_lines: liquid.block_source_lines,
                error: None,
            }),
            Err(error) => documents.push(Lm2SourceSmokeDocument {
                path: document.path,
                source_lines,
                block_source_lines: Vec::new(),
                error: Some(error),
            }),
        }
    }
    let failures = documents
        .iter()
        .filter(|document| document.error.is_some())
        .count();
    let report = Lm2SourceSmokeReport {
        app_version: env!("CARGO_PKG_VERSION"),
        timestamp_unix_secs: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        document_count: documents.len(),
        failures,
        documents,
    };
    let json = serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?;
    if let Some(path) = output_path {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        std::fs::write(path, json).map_err(|error| error.to_string())?;
    } else {
        println!("{json}");
    }
    Ok(())
}

pub fn prepare_liquid_mode2_document(
    request: LiquidMode2Request,
) -> Result<LiquidDocument, String> {
    prepare_liquid_mode2_document_with_timing(request).map(|(document, _)| document)
}

pub fn prepare_liquid_mode2_document_with_timing(
    request: LiquidMode2Request,
) -> Result<(LiquidDocument, LiquidMode2Timing), String> {
    let total_started = Instant::now();
    let runtime_started = Instant::now();
    let mut runtime = Lm2Runtime::load();
    let mut pp_runtime_warnings = Vec::new();
    let mut liquidvision_runtime_warnings = Vec::new();
    if request.use_pp_footnote_regions {
        runtime.pp_footnote_region_membership = true;
        if runtime.pp_priors.is_none() {
            match load_or_generate_lm2_pp_priors(&request.path, &request.deep_source_lines) {
                Ok(Some(index)) => {
                    runtime.pp_priors = Some(index);
                }
                Ok(None) => {
                    runtime.pp_footnote_region_membership = false;
                    pp_runtime_warnings
                        .push("PP-DocLayout runtime produced no usable LM2 prior rows.".to_owned());
                }
                Err(error) => {
                    runtime.pp_footnote_region_membership = false;
                    pp_runtime_warnings.push(format!(
                        "PP-DocLayout runtime unavailable; using base LM2 model: {error}"
                    ));
                }
            }
        }
        if runtime.pp_footnote_region_membership
            && !runtime
                .pp_priors
                .as_ref()
                .is_some_and(lm2_pp_prior_index_has_footnotes)
        {
            runtime.pp_footnote_region_membership = false;
            if runtime.pp_priors.is_some() {
                pp_runtime_warnings.push(
                    "PP-DocLayout runtime produced no high-confidence footnote rows for LM2."
                        .to_owned(),
                );
            }
        }
    }
    let mut timing = LiquidMode2Timing {
        runtime_load_ms: runtime_started.elapsed().as_secs_f64() * 1000.0,
        ..Default::default()
    };
    let native_catboost_no_stack = runtime.native_catboost_model.is_some();
    let d1_runtime_zerospend_overlay =
        !native_catboost_no_stack && lm2_d1_runtime_zerospend_overlay_enabled();
    let d1_runtime_continuation_overlay =
        !native_catboost_no_stack && lm2_d1_runtime_continuation_overlay_enabled();
    let d1_runtime_immediate_continuation_overlay =
        !native_catboost_no_stack && lm2_d1_runtime_immediate_continuation_overlay_enabled();
    let d1_runtime_sandwiched_continuation_overlay =
        !native_catboost_no_stack && lm2_d1_runtime_sandwiched_continuation_overlay_enabled();
    let d1_runtime_wide_sandwich_overlay =
        !native_catboost_no_stack && lm2_d1_runtime_wide_sandwich_overlay_enabled();
    let d1_runtime_safe_numeric_note_overlay =
        !native_catboost_no_stack && lm2_d1_runtime_safe_numeric_note_overlay_enabled();
    let d1_runtime_post_wide_cue_overlay =
        !native_catboost_no_stack && lm2_d1_runtime_post_wide_cue_overlay_enabled();
    let d1_runtime_postcue_citation_next1_overlay =
        !native_catboost_no_stack && lm2_d1_runtime_postcue_citation_next1_overlay_enabled();
    let d1_runtime_near8_cue_overlay =
        !native_catboost_no_stack && lm2_d1_runtime_near8_cue_overlay_enabled();
    let d1_runtime_wide_divider_guard_overlay =
        !native_catboost_no_stack && lm2_d1_runtime_wide_divider_guard_overlay_enabled();
    let d1_runtime_geometric_zone_overlay =
        !native_catboost_no_stack && lm2_d1_runtime_geometric_zone_overlay_enabled();
    let d1_runtime_footer_artifact_overlay =
        !native_catboost_no_stack && lm2_d1_runtime_footer_artifact_overlay_enabled();
    let footnote_monotone_overlay =
        !native_catboost_no_stack && lm2_footnote_monotone_overlay_enabled();
    let footnote_carryover_overlay =
        !native_catboost_no_stack && lm2_footnote_carryover_overlay_enabled();
    let open_footnote_carryover_overlay = lm2_open_footnote_carryover_overlay_enabled();
    let table_figure_router_overlay =
        !native_catboost_no_stack && lm2_table_figure_router_overlay_enabled();
    let page_object_overlay = !native_catboost_no_stack && lm2_page_object_overlay_enabled();
    let page_object_tuned_overlay =
        !native_catboost_no_stack && lm2_page_object_tuned_overlay_enabled();
    let d1_runtime_zerospend_overlay_version =
        d1_runtime_zerospend_overlay.then_some(LM2_D1_RUNTIME_ZEROSPEND_OVERLAY_VERSION);
    let context_twopass_label = if request.external_emissions_path.is_none() {
        runtime
            .context_twopass_model
            .as_ref()
            .map(|model| model.label())
    } else {
        None
    };
    let source_signature = lm2_source_signature(
        &request.path,
        &request.pages,
        &runtime.model_label,
        context_twopass_label.as_deref(),
        runtime.pp_prior_source().as_deref(),
        runtime
            .static_front_overlay
            .as_ref()
            .map(|overlay| overlay.source_label.as_str()),
        request.use_pymupdf_blocks,
        runtime.pp_footnote_region_membership,
        runtime.marker_decoder_prior,
        runtime.small_font_decoder_prior,
        runtime.small_font_sequence_prior,
        runtime.anchored_marginalia_flow_guard,
        runtime.body_preservation_guard,
        runtime.action_neutral_blocksplit,
        runtime.toc_overlay,
        runtime.front_matter_guard,
        runtime.marginalia_preservation_guard,
        d1_runtime_zerospend_overlay,
        d1_runtime_zerospend_overlay_version,
        d1_runtime_continuation_overlay,
        d1_runtime_immediate_continuation_overlay,
        d1_runtime_sandwiched_continuation_overlay,
        d1_runtime_wide_sandwich_overlay,
        d1_runtime_safe_numeric_note_overlay,
        d1_runtime_post_wide_cue_overlay,
        d1_runtime_postcue_citation_next1_overlay,
        d1_runtime_near8_cue_overlay,
        d1_runtime_wide_divider_guard_overlay,
        d1_runtime_geometric_zone_overlay,
        d1_runtime_footer_artifact_overlay,
        footnote_monotone_overlay,
        footnote_carryover_overlay,
        table_figure_router_overlay,
        page_object_overlay,
        page_object_tuned_overlay,
        runtime.start_score_scale,
        runtime.transition_score_scale,
    );
    let external_emissions = request
        .external_emissions_path
        .as_deref()
        .map(Lm2ExternalEmissions::load)
        .transpose()?;
    if external_emissions.is_none()
        && !open_footnote_carryover_overlay
        && let Some(cached) = load_cached_lm2_document(&source_signature)
    {
        if !cached.blocks.is_empty() && !cached.block_source_lines.is_empty() {
            timing.cache_hit = true;
            timing.total_ms = total_started.elapsed().as_secs_f64() * 1000.0;
            return Ok((cached, timing));
        }
    }

    let feature_started = Instant::now();
    let mut lines = request
        .deep_source_lines
        .iter()
        .filter(|line| !line.text.trim().is_empty())
        .cloned()
        .collect::<Vec<_>>();
    if native_catboost_no_stack
        && liquidvision_enabled(true)
        && !lines.iter().all(|line| line.lv.has_region)
    {
        match PdfEngine::new()
            .map_err(|error| error.to_string())
            .and_then(|engine| {
                fill_document_features(&engine, &request.path, request.pages.len(), &mut lines)
                    .map_err(|error| error.to_string())
            }) {
            Ok(report) => {
                timing.liquidvision_fill_ms = report.elapsed_ms;
                if report.pages_filled != report.pages_attempted || !report.errors.is_empty() {
                    liquidvision_runtime_warnings.push(format!(
                        "LiquidVision populated {}/{} pages; {} page error(s): {}",
                        report.pages_filled,
                        report.pages_attempted,
                        report.errors.len(),
                        report.errors.join("; ")
                    ));
                }
            }
            Err(error) => liquidvision_runtime_warnings.push(format!(
                "LiquidVision runtime feature fill failed; native model received zero vision features: {error}"
            )),
        }
    } else if native_catboost_no_stack && !liquidvision_enabled(true) {
        liquidvision_runtime_warnings.push(
            "LiquidVision was disabled; native model received zero vision features.".to_owned(),
        );
    }
    annotate_pp_priors_for_lines(&runtime, &request.path.display().to_string(), &mut lines);
    enrich_lm2_document_features(&mut lines);
    timing.feature_enrichment_ms =
        (feature_started.elapsed().as_secs_f64() * 1000.0 - timing.liquidvision_fill_ms).max(0.0);
    if lines.is_empty() {
        let document = LiquidDocument {
            title: request.title,
            blocks: Vec::new(),
            block_source_lines: Vec::new(),
            footnote_links: Vec::new(),
            footnote_link_integrity: None,
            profile: Some(lm2_profile()),
            noise_lines_removed: 0,
            llm_used: false,
            llm_provider: Some("LM2".to_owned()),
            deep_liquid_used: false,
            deep_liquid_model: Some(runtime.model_label),
            warnings: vec![
                "LiquidMode2 found no selectable text. Run OCR before using LM2.".to_owned(),
            ],
            source_signature,
        };
        if external_emissions.is_none() && !open_footnote_carryover_overlay {
            let _ = save_cached_lm2_document(&document);
        }
        timing.total_ms = total_started.elapsed().as_secs_f64() * 1000.0;
        return Ok((document, timing));
    }

    let model_started = Instant::now();
    let mut decoded = if let Some(external) = external_emissions.as_ref() {
        decode_pages_with_external_emissions(&runtime, &request.path, &lines, external)?
    } else {
        decode_pages(&runtime, &lines)
    };
    timing.model_decode_ms = model_started.elapsed().as_secs_f64() * 1000.0;

    let overlay_started = Instant::now();
    if external_emissions.is_none()
        && let Some(model) = runtime.context_twopass_model.as_ref()
    {
        apply_context_twopass_model(model, &request.path, &mut decoded);
    }
    if let Some(overlay) = runtime.static_front_overlay.as_ref() {
        apply_static_front_overlay(overlay, &request.path, &mut decoded);
    }
    if runtime.toc_overlay {
        apply_document_toc_overlay(&mut decoded);
    }
    if runtime.front_matter_guard {
        apply_front_matter_guard(&mut decoded);
    }
    if runtime.marginalia_preservation_guard {
        apply_marginalia_preservation_guard(&mut decoded);
    }
    if d1_runtime_zerospend_overlay {
        apply_d1_runtime_zerospend_overlay(&mut decoded);
    }
    if d1_runtime_continuation_overlay {
        apply_d1_runtime_continuation_overlay(&mut decoded);
    }
    if d1_runtime_immediate_continuation_overlay {
        apply_d1_runtime_immediate_continuation_overlay(&mut decoded);
    }
    if d1_runtime_sandwiched_continuation_overlay {
        apply_d1_runtime_sandwiched_continuation_overlay(&mut decoded);
    }
    if d1_runtime_wide_sandwich_overlay {
        apply_d1_runtime_wide_sandwich_overlay(&mut decoded);
    }
    if d1_runtime_safe_numeric_note_overlay {
        apply_d1_runtime_safe_numeric_note_overlay(&mut decoded);
    }
    if d1_runtime_post_wide_cue_overlay {
        apply_d1_runtime_post_wide_cue_overlay(&mut decoded);
    }
    if d1_runtime_postcue_citation_next1_overlay {
        apply_d1_runtime_postcue_citation_next1_overlay(&mut decoded);
    }
    if d1_runtime_near8_cue_overlay {
        apply_d1_runtime_near8_cue_overlay(&mut decoded);
    }
    if d1_runtime_wide_divider_guard_overlay {
        apply_d1_runtime_wide_divider_guard_overlay(&mut decoded);
    }
    if table_figure_router_overlay {
        apply_table_figure_router_overlay(&mut decoded);
    }
    if page_object_overlay {
        apply_page_object_overlay(&mut decoded);
    }
    if page_object_tuned_overlay {
        apply_page_object_tuned_overlay(&mut decoded);
    }
    if d1_runtime_geometric_zone_overlay {
        apply_d1_runtime_geometric_zone_overlay(&mut decoded);
    }
    if d1_runtime_footer_artifact_overlay {
        apply_d1_runtime_footer_artifact_overlay(&mut decoded);
    }
    if footnote_carryover_overlay {
        apply_footnote_carryover_overlay(&mut decoded);
    }
    if footnote_monotone_overlay {
        apply_footnote_monotone_overlay(&mut decoded);
    }
    if open_footnote_carryover_overlay {
        apply_open_footnote_carryover_overlay(&mut decoded);
    }
    if !native_catboost_no_stack {
        apply_page_label_furniture_guard(&mut decoded);
    }
    timing.overlay_decode_ms = overlay_started.elapsed().as_secs_f64() * 1000.0;

    let assembly_started = Instant::now();
    let hidden_count = decoded
        .iter()
        .filter(|(_, action)| *action == Lm2Action::HideNoise)
        .count();
    let grouping = if request.use_pymupdf_blocks {
        try_apply_lm2_pymupdf_grouping(
            &request.path,
            &request.title,
            &source_signature,
            &request.deep_source_lines,
        )
        .ok()
        .flatten()
    } else {
        None
    };
    let (title, mut blocks, mut block_source_lines) =
        build_lm2_blocks_with_grouping(&request.title, &decoded, grouping.as_ref());
    if runtime.action_neutral_blocksplit {
        apply_action_neutral_blocksplit(&mut blocks, &mut block_source_lines, &decoded);
    }
    apply_deferred_marginalia_reflow(&mut blocks, &mut block_source_lines);
    let mut warnings = if runtime.native_catboost_model.is_some() {
        vec![format!(
            "Review Mode uses {} raw no-stack emissions.",
            runtime.model_label
        )]
    } else {
        vec![format!(
            "Review Mode uses {} emissions plus page-level sequence decoder.",
            runtime.model_label
        )]
    };
    warnings.extend(runtime.load_warnings.iter().cloned());
    if lm2_v20_runtime_preset_enabled() {
        warnings.push("LM2 v20 runtime preset is enabled.".to_owned());
    }
    if lm2_v25_d1_runtime_preset_enabled() {
        warnings.push("LM2 v25 D1 zero-spend runtime preset is enabled.".to_owned());
    }
    if (runtime.start_score_scale - 1.0).abs() > f64::EPSILON
        || (runtime.transition_score_scale - 1.0).abs() > f64::EPSILON
    {
        warnings.push(format!(
            "LM2 decoder start/transition scales are {:.3}/{:.3}.",
            runtime.start_score_scale, runtime.transition_score_scale
        ));
    }
    if let Some(label) = context_twopass_label.as_deref() {
        warnings.push(format!(
            "Review Mode context two-pass model is active: {label}."
        ));
    }
    if runtime.native_catboost_model.is_none()
        && runtime.numeric_catboost_model.is_none()
        && runtime.model.is_none()
    {
        warnings.push("LM2 trained model not found; used geometry fallback emissions.".to_owned());
    }
    if let Some(external) = external_emissions.as_ref() {
        warnings.push(format!(
            "LM2 external emissions override is enabled: {}. LM2 document cache was bypassed.",
            external.source_label()
        ));
    }
    if runtime.marker_decoder_prior {
        warnings.push("EXP-061 marker-continuity decoder prior is enabled.".to_owned());
    }
    if runtime.small_font_decoder_prior {
        warnings.push("EXP-062 small-font lower-page decoder prior is enabled.".to_owned());
    }
    if runtime.anchored_marginalia_flow_guard {
        warnings.push("EXP-069 anchored marginalia flow guard is enabled.".to_owned());
    }
    if runtime.body_preservation_guard {
        warnings.push("LM2 body-preservation guard is enabled.".to_owned());
    }
    if runtime.action_neutral_blocksplit {
        warnings.push("LM2 action-neutral blocksplit is enabled.".to_owned());
    }
    if runtime.toc_overlay {
        warnings.push("LM2 document-local TOC overlay is enabled.".to_owned());
    }
    if runtime.front_matter_guard {
        warnings.push("LM2 first-page front-matter guard is enabled.".to_owned());
    }
    if runtime.marginalia_preservation_guard {
        warnings.push("LM2 marginalia-preservation guard is enabled.".to_owned());
    }
    if d1_runtime_zerospend_overlay {
        warnings.push("LM2 D1 zero-spend keep-to-marginalia overlay is enabled.".to_owned());
    }
    if d1_runtime_continuation_overlay {
        warnings.push("LM2 D1 continuation keep-to-marginalia overlay is enabled.".to_owned());
    }
    if d1_runtime_immediate_continuation_overlay {
        warnings.push(
            "LM2 D1 immediate-neighbor continuation keep-to-marginalia overlay is enabled."
                .to_owned(),
        );
    }
    if d1_runtime_sandwiched_continuation_overlay {
        warnings.push(
            "LM2 D1 sandwiched continuation keep-to-marginalia overlay is enabled.".to_owned(),
        );
    }
    if d1_runtime_wide_sandwich_overlay {
        warnings.push(
            "LM2 D1 wide sandwiched continuation keep-to-marginalia overlay is enabled.".to_owned(),
        );
    }
    if d1_runtime_safe_numeric_note_overlay {
        warnings.push(
            "LM2 D1 safe numeric note-start keep-to-marginalia overlay is enabled.".to_owned(),
        );
    }
    if d1_runtime_post_wide_cue_overlay {
        warnings.push("LM2 D1 post-wide cue keep-to-marginalia overlay is enabled.".to_owned());
    }
    if d1_runtime_postcue_citation_next1_overlay {
        warnings.push(
            "LM2 D1 postcue citation-next1 keep-to-marginalia overlay is enabled.".to_owned(),
        );
    }
    if d1_runtime_near8_cue_overlay {
        warnings.push("LM2 D1 near8 cue keep-to-marginalia overlay is enabled.".to_owned());
    }
    if d1_runtime_wide_divider_guard_overlay {
        warnings
            .push("LM2 D1 wide-divider guarded keep-to-marginalia overlay is enabled.".to_owned());
    }
    if d1_runtime_footer_artifact_overlay {
        warnings.push("LM2 D1 footer-artifact hide-noise overlay is enabled.".to_owned());
    }
    if footnote_monotone_overlay {
        warnings.push("LM2 global footnote-number monotonicity overlay is enabled.".to_owned());
    }
    if footnote_carryover_overlay {
        warnings.push("LM2 footnote carryover hard overlay is enabled.".to_owned());
    }
    if open_footnote_carryover_overlay {
        warnings.push("LM2 open-footnote carryover hard overlay is enabled.".to_owned());
    }
    if page_object_tuned_overlay {
        warnings.push("LM2 page-object tuned hide-noise overlay is enabled.".to_owned());
    }
    if let Some(overlay) = runtime.static_front_overlay.as_ref() {
        warnings.push(format!(
            "LM2 static front overlay is enabled: {}.",
            overlay.source_label
        ));
    }
    if runtime.pp_footnote_region_membership {
        warnings.push("EXP-075 PP footnote-region membership override is enabled.".to_owned());
    }
    warnings.extend(pp_runtime_warnings);
    warnings.extend(liquidvision_runtime_warnings);
    if let Some(grouping) = &grouping {
        if let Some(mode) = &grouping.mode {
            warnings.push(format!("PyMuPDF block grouping mode: {mode}"));
        }
        warnings.extend(
            grouping
                .warnings
                .iter()
                .map(|warning| format!("PyMuPDF block grouping: {warning}")),
        );
    }

    let mut document = LiquidDocument {
        title,
        blocks,
        block_source_lines,
        footnote_links: Vec::new(),
        footnote_link_integrity: None,
        profile: Some(lm2_profile()),
        noise_lines_removed: hidden_count,
        llm_used: false,
        llm_provider: Some("LM2".to_owned()),
        deep_liquid_used: false,
        deep_liquid_model: Some(runtime.model_label),
        warnings,
        source_signature,
    };
    attach_footnote_links(&mut document);
    if external_emissions.is_none() && !open_footnote_carryover_overlay {
        let _ = save_cached_lm2_document(&document);
    }
    timing.assembly_ms = assembly_started.elapsed().as_secs_f64() * 1000.0;
    timing.total_ms = total_started.elapsed().as_secs_f64() * 1000.0;
    Ok((document, timing))
}

impl Lm2Runtime {
    fn load() -> Self {
        let mut load_warnings = Vec::new();
        let pp_priors = load_lm2_pp_priors().ok().flatten();
        let native_catboost_model = match load_lm2_native_catboost_model() {
            Ok(Some(model)) => Some(model),
            Ok(None) => {
                load_warnings.push(
                    "Promoted native CatBoost runtime assets were not found; using fallback emissions."
                        .to_owned(),
                );
                None
            }
            Err(error) => {
                load_warnings.push(format!(
                    "Promoted native CatBoost runtime failed to load; using fallback emissions: {error}"
                ));
                None
            }
        };
        let native_catboost_active = native_catboost_model.is_some();
        let context_twopass_model = if native_catboost_active && lm2_context_twopass_enabled() {
            match load_lm2_context_twopass_model() {
                Ok(Some(model)) => Some(model),
                Ok(None) => {
                    load_warnings.push(
                        "Promoted context two-pass model was not found; using native emissions only."
                            .to_owned(),
                    );
                    None
                }
                Err(error) => {
                    load_warnings.push(format!(
                        "Promoted context two-pass model failed to load; using native emissions only: {error}"
                    ));
                    None
                }
            }
        } else {
            None
        };
        let numeric_catboost_model = if native_catboost_active {
            None
        } else {
            load_lm2_numeric_catboost_model().ok().flatten()
        };
        let static_front_overlay = if native_catboost_active {
            None
        } else {
            load_lm2_static_front_overlay().ok().flatten()
        };
        let runtime_label = native_catboost_model
            .as_ref()
            .map(|model| {
                format!(
                    "lm2-native-catboost-text-runtime:f{}c{}t{}d{}",
                    model.float_feature_count,
                    model.cat_feature_count,
                    model.text_feature_count,
                    model.dimensions_count
                )
            })
            .or_else(|| {
                numeric_catboost_model.as_ref().map(|_| {
                    let source =
                        std::env::var("LAWPDF_LM2_NUMERIC_CATBOOST_MODEL").unwrap_or_default();
                    format!("lm2-numeric-catboost-runtime:{:016x}", fnv1a64(&source))
                })
            });
        match load_lm2_model() {
            Ok(model) if model_is_usable(&model) => Self {
                model_label: runtime_label.unwrap_or_else(|| model.model_id.clone()),
                load_warnings,
                model: Some(model),
                native_catboost_model,
                context_twopass_model,
                numeric_catboost_model,
                static_front_overlay,
                pp_priors,
                pp_footnote_region_membership: !native_catboost_active
                    && lm2_pp_footnote_region_membership_enabled(),
                marker_decoder_prior: !native_catboost_active && lm2_marker_decoder_prior_enabled(),
                small_font_decoder_prior: !native_catboost_active
                    && lm2_small_font_decoder_prior_enabled(),
                small_font_sequence_prior: !native_catboost_active
                    && lm2_small_font_sequence_prior_enabled(),
                anchored_marginalia_flow_guard: !native_catboost_active
                    && lm2_anchored_marginalia_flow_guard_enabled(),
                body_preservation_guard: !native_catboost_active
                    && lm2_body_preservation_guard_enabled(),
                action_neutral_blocksplit: !native_catboost_active
                    && lm2_action_neutral_blocksplit_enabled(),
                toc_overlay: !native_catboost_active && lm2_toc_overlay_enabled(),
                front_matter_guard: !native_catboost_active && lm2_front_matter_guard_enabled(),
                marginalia_preservation_guard: !native_catboost_active
                    && lm2_marginalia_preservation_guard_enabled(),
                start_score_scale: if native_catboost_active {
                    1.0
                } else {
                    lm2_start_score_scale()
                },
                transition_score_scale: if native_catboost_active {
                    1.0
                } else {
                    lm2_transition_score_scale()
                },
            },
            _ => Self {
                model: None,
                model_label: runtime_label.unwrap_or_else(|| "lm2-heuristic-fallback".to_owned()),
                load_warnings,
                native_catboost_model,
                context_twopass_model,
                numeric_catboost_model,
                static_front_overlay,
                pp_priors,
                pp_footnote_region_membership: !native_catboost_active
                    && lm2_pp_footnote_region_membership_enabled(),
                marker_decoder_prior: !native_catboost_active && lm2_marker_decoder_prior_enabled(),
                small_font_decoder_prior: !native_catboost_active
                    && lm2_small_font_decoder_prior_enabled(),
                small_font_sequence_prior: !native_catboost_active
                    && lm2_small_font_sequence_prior_enabled(),
                anchored_marginalia_flow_guard: !native_catboost_active
                    && lm2_anchored_marginalia_flow_guard_enabled(),
                body_preservation_guard: !native_catboost_active
                    && lm2_body_preservation_guard_enabled(),
                action_neutral_blocksplit: !native_catboost_active
                    && lm2_action_neutral_blocksplit_enabled(),
                toc_overlay: !native_catboost_active && lm2_toc_overlay_enabled(),
                front_matter_guard: !native_catboost_active && lm2_front_matter_guard_enabled(),
                marginalia_preservation_guard: !native_catboost_active
                    && lm2_marginalia_preservation_guard_enabled(),
                start_score_scale: if native_catboost_active {
                    1.0
                } else {
                    lm2_start_score_scale()
                },
                transition_score_scale: if native_catboost_active {
                    1.0
                } else {
                    lm2_transition_score_scale()
                },
            },
        }
    }

    fn emission_scores(&self, line: &DeepLiquidSourceLine) -> [f64; 3] {
        if let Some(model) = self.native_catboost_model.as_ref() {
            return model.emission_scores(line).unwrap_or([0.0, 0.0, 0.0]);
        }
        let mut scores = if let Some(model) = self.numeric_catboost_model.as_ref() {
            model.emission_scores(line)
        } else if let Some(model) = self.model.as_ref() {
            let features = lm2_features(
                line,
                model.feature_dim,
                model.doc_font_zscores_enabled(),
                model.repetition_fingerprints_enabled(),
                model.marker_continuity_enabled(),
            );
            let mut scores = [0.0, 0.0, 0.0];
            for (index, score) in scores.iter_mut().enumerate() {
                *score = *model.bias.get(index).unwrap_or(&0.0);
            }
            for (feature_index, value) in features {
                for (class_index, score) in scores.iter_mut().enumerate() {
                    if let Some(weight) = model
                        .weights
                        .get(class_index)
                        .and_then(|weights| weights.get(feature_index))
                    {
                        *score += weight * value;
                    }
                }
            }
            scores
        } else {
            [0.0, 0.0, 0.0]
        };
        apply_layout_priors(line, &mut scores);
        apply_pp_priors(self, line, &mut scores);
        scores
    }

    fn decoder_weights(&self) -> Option<&HashMap<String, f64>> {
        self.model
            .as_ref()
            .and_then(|model| model.decoder_constants.as_ref())
            .map(|constants| &constants.weights)
            .filter(|weights| !weights.is_empty())
    }

    fn pp_prior_for_line(&self, line: &DeepLiquidSourceLine) -> Option<Lm2PpPrior> {
        line.pp_prior_score.map(|score| Lm2PpPrior {
            role: line.pp_prior_role.clone().unwrap_or_default(),
            label: line.pp_prior_label.clone().unwrap_or_default(),
            score,
        })
    }

    fn pp_prior_source(&self) -> Option<String> {
        self.pp_priors
            .as_ref()
            .map(|index| index.source.display().to_string())
    }
}

fn argmax_lm2_action(scores: [f64; 3]) -> Lm2Action {
    ACTIONS
        .into_iter()
        .max_by(|left, right| {
            scores[left.index()]
                .partial_cmp(&scores[right.index()])
                .unwrap_or(Ordering::Equal)
        })
        .unwrap_or(Lm2Action::Keep)
}

impl Lm2ContextTwopassModel {
    fn label(&self) -> String {
        format!(
            "{LM2_CONTEXT_TWOPASS_VERSION}:{}:a{}f{}m{}",
            self.schema_version,
            self.actions.len(),
            self.feature_count,
            self.models.len()
        )
    }

    fn model_for_document(&self, document_path: &Path) -> Option<&Lm2ContextTwopassHgbModel> {
        let normalized = lm2_context_norm_path(&document_path.display().to_string());
        if let Some(fold) = self.doc_to_fold.get(&normalized) {
            let name = format!("fold{fold}");
            if let Some(model) = self.models.iter().find(|model| model.name == name) {
                return Some(model);
            }
        }
        self.models
            .iter()
            .find(|model| model.name == self.unseen_doc_model)
            .or_else(|| self.models.first())
    }
}

impl Lm2ContextTwopassHgbModel {
    fn predict(&self, features: &[f64]) -> Lm2Action {
        let mut raw = [0.0; 3];
        for (index, score) in raw.iter_mut().enumerate() {
            *score = *self.baseline_prediction.get(index).unwrap_or(&0.0);
        }
        for iteration in &self.trees {
            for (class_index, tree) in iteration.iter().take(3).enumerate() {
                raw[class_index] += lm2_context_tree_predict(tree, features);
            }
        }
        let class_index = raw
            .iter()
            .enumerate()
            .max_by(|(_, left), (_, right)| left.partial_cmp(right).unwrap_or(Ordering::Equal))
            .map(|(index, _)| index)
            .unwrap_or(1);
        match class_index {
            0 => Lm2Action::HideNoise,
            1 => Lm2Action::Keep,
            2 => Lm2Action::Marginalia,
            _ => Lm2Action::Keep,
        }
    }
}

fn lm2_context_tree_predict(nodes: &[[f64; 7]], features: &[f64]) -> f64 {
    let mut index = 0usize;
    for _ in 0..nodes.len() {
        let Some(node) = nodes.get(index) else {
            return 0.0;
        };
        if node[6] != 0.0 {
            return node[0];
        }
        let feature_index = node[1] as usize;
        let threshold = node[2];
        let value = *features.get(feature_index).unwrap_or(&0.0);
        let go_left = if value.is_nan() {
            node[3] != 0.0
        } else {
            value <= threshold
        };
        index = if go_left {
            node[4] as usize
        } else {
            node[5] as usize
        };
    }
    0.0
}

fn apply_context_twopass_model(
    model: &Lm2ContextTwopassModel,
    document_path: &Path,
    decoded: &mut [(DeepLiquidSourceLine, Lm2Action)],
) {
    if decoded.is_empty() {
        return;
    }
    let Some(document_model) = model.model_for_document(document_path) else {
        return;
    };
    let baseline_actions = decoded
        .iter()
        .map(|(_, action)| *action)
        .collect::<Vec<_>>();
    let block_meta = lm2_context_block_meta(decoded);
    for index in 0..decoded.len() {
        let features = lm2_context_feature_vector(decoded, &baseline_actions, &block_meta, index);
        if features.len() == model.feature_count {
            decoded[index].1 = document_model.predict(&features);
        }
    }
}

fn lm2_context_norm_path(value: &str) -> String {
    value.trim().replace('\\', "/").to_ascii_lowercase()
}

fn lm2_context_action_index(action: Lm2Action) -> usize {
    match action {
        Lm2Action::HideNoise => 0,
        Lm2Action::Keep => 1,
        Lm2Action::Marginalia => 2,
    }
}

fn lm2_context_onehot(features: &mut Vec<f64>, action: Option<Lm2Action>) {
    let selected = action.map(lm2_context_action_index);
    for index in 0..3 {
        features.push(if selected == Some(index) { 1.0 } else { 0.0 });
    }
}

fn lm2_context_block_meta(
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
) -> HashMap<String, Lm2ContextBlockMeta> {
    let (_, _, block_source_lines) = build_lm2_blocks_with_grouping("", decoded, None);
    let lines_by_id = decoded
        .iter()
        .map(|(line, _)| (line.id.clone(), line))
        .collect::<HashMap<_, _>>();
    let mut out = HashMap::new();
    for block in block_source_lines {
        let full_rows = block
            .lines
            .iter()
            .filter_map(|line| line.id.as_ref().and_then(|id| lines_by_id.get(id).copied()))
            .collect::<Vec<_>>();
        if full_rows.is_empty() {
            continue;
        }
        let n = full_rows.len().max(1) as f64;
        let block_text = full_rows
            .iter()
            .map(|line| collapse_whitespace(&line.text))
            .collect::<Vec<_>>()
            .join(" ");
        let shapes = full_rows
            .iter()
            .map(|line| lm2_context_text_shape(&collapse_whitespace(&line.text)))
            .collect::<Vec<_>>();
        let block_action = block
            .lines
            .iter()
            .map(|line| lm2_context_action_for_role(line.role))
            .collect::<Vec<_>>();
        let block_action = most_common_lm2_context_action(&block_action);
        let meta = Lm2ContextBlockMeta {
            block_action,
            block_line_count: full_rows.len(),
            block_char_count: block_text.chars().count(),
            block_short_ratio: shapes.iter().map(|shape| shape.short_text).sum::<f64>() / n,
            block_numeric_ratio: shapes
                .iter()
                .map(|shape| shape.numeric_or_roman)
                .sum::<f64>()
                / n,
            block_dotleader: if shapes.iter().any(|shape| shape.dotleader > 0.0) {
                1.0
            } else {
                0.0
            },
            block_note_start_ratio: shapes.iter().map(|shape| shape.note_start).sum::<f64>() / n,
            block_edge_ratio: full_rows
                .iter()
                .filter(|line| {
                    line.doc_repeated_top_edge
                        || line.doc_repeated_bottom_edge
                        || line.doc_repeated_edge_text
                })
                .count() as f64
                / n,
            block_axis_ratio: full_rows
                .iter()
                .filter(|line| {
                    line.doc_vertical_axis_like
                        || line.doc_vertical_numeric_axis_like
                        || line.page_table_column_like
                })
                .count() as f64
                / n,
            block_footzone_ratio: full_rows
                .iter()
                .filter(|line| line.in_footnote_zone || line.below_footnote_divider)
                .count() as f64
                / n,
            block_pos_norm: 0.0,
        };
        let denom = full_rows.len().saturating_sub(1).max(1) as f64;
        for (position, line) in full_rows.iter().enumerate() {
            let mut line_meta = meta.clone();
            line_meta.block_pos_norm = position as f64 / denom;
            out.insert(line.id.clone(), line_meta);
        }
    }
    out
}

fn most_common_lm2_context_action(actions: &[Lm2Action]) -> Option<Lm2Action> {
    ACTIONS
        .into_iter()
        .max_by_key(|candidate| actions.iter().filter(|action| *action == candidate).count())
}

fn lm2_context_action_for_role(role: LiquidBlockRole) -> Lm2Action {
    match role {
        LiquidBlockRole::Footnote | LiquidBlockRole::Marginalia => Lm2Action::Marginalia,
        LiquidBlockRole::Table
        | LiquidBlockRole::Caption
        | LiquidBlockRole::Contents
        | LiquidBlockRole::Header
        | LiquidBlockRole::Footer
        | LiquidBlockRole::Metadata
        | LiquidBlockRole::SectionBreak
        | LiquidBlockRole::Noise => Lm2Action::HideNoise,
        _ => Lm2Action::Keep,
    }
}

#[derive(Debug, Clone, Copy)]
struct Lm2ContextTextShape {
    char_count: f64,
    token_count: f64,
    short_text: f64,
    numeric_or_roman: f64,
    allcaps: f64,
    digit_ratio: f64,
    upper_ratio: f64,
    dotleader: f64,
    note_start: f64,
}

fn lm2_context_text_shape(text: &str) -> Lm2ContextTextShape {
    let chars = text.chars().count();
    let mut token_count = 0usize;
    let mut in_token = false;
    let mut ascii_letters = Vec::new();
    let mut digit_count = 0usize;
    for ch in text.chars() {
        let is_token = ch.is_ascii_alphanumeric() || ch == '§';
        if is_token && !in_token {
            token_count += 1;
        }
        in_token = is_token;
        if ch.is_ascii_alphabetic() {
            ascii_letters.push(ch);
        }
        if ch.is_ascii_digit() {
            digit_count += 1;
        }
    }
    let letter_count = ascii_letters.len();
    let upper_count = ascii_letters
        .iter()
        .filter(|ch| ch.is_ascii_uppercase())
        .count();
    let numeric_or_roman = !text.is_empty()
        && text.chars().count() <= 8
        && text.chars().all(|ch| {
            ch.is_ascii_digit()
                || matches!(
                    ch,
                    'I' | 'V'
                        | 'X'
                        | 'L'
                        | 'C'
                        | 'D'
                        | 'M'
                        | 'i'
                        | 'v'
                        | 'x'
                        | 'l'
                        | 'c'
                        | 'd'
                        | 'm'
                )
        });
    let allcaps = letter_count >= 8 && upper_count == letter_count;
    Lm2ContextTextShape {
        char_count: chars as f64,
        token_count: token_count as f64,
        short_text: if token_count <= 4 || chars <= 24 {
            1.0
        } else {
            0.0
        },
        numeric_or_roman: if numeric_or_roman { 1.0 } else { 0.0 },
        allcaps: if allcaps { 1.0 } else { 0.0 },
        digit_ratio: digit_count as f64 / chars.max(1) as f64,
        upper_ratio: upper_count as f64 / letter_count.max(1) as f64,
        dotleader: if lm2_context_has_dotleader(text) {
            1.0
        } else {
            0.0
        },
        note_start: if lm2_context_note_start(text) {
            1.0
        } else {
            0.0
        },
    }
}

fn lm2_context_has_dotleader(text: &str) -> bool {
    text.contains("...") || text.contains("···") || text.contains('…')
}

fn lm2_context_note_start(text: &str) -> bool {
    let trimmed = text.trim_start();
    let mut chars = trimmed.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if matches!(first, '*' | '†' | '‡' | '§') {
        return chars.next().is_some_and(char::is_whitespace);
    }
    if first.is_ascii_lowercase() {
        return chars.next() == Some(')') && chars.next().is_some_and(char::is_whitespace);
    }
    let mut rest = trimmed;
    if rest.starts_with('(') {
        rest = &rest[1..];
    }
    let digit_count = rest.chars().take_while(|ch| ch.is_ascii_digit()).count();
    if !(1..=4).contains(&digit_count) {
        return false;
    }
    rest = &rest[digit_count..];
    if rest.starts_with(')') {
        rest = &rest[1..];
    }
    rest.chars().next().is_some_and(char::is_whitespace)
}

fn lm2_context_feature_vector(
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
    baseline_actions: &[Lm2Action],
    block_meta: &HashMap<String, Lm2ContextBlockMeta>,
    index: usize,
) -> Vec<f64> {
    let line = &decoded[index].0;
    let shape = lm2_context_text_shape(&collapse_whitespace(&line.text));
    let block = block_meta.get(&line.id).cloned().unwrap_or_default();
    let width = (line.page_width as f64).max(1.0);
    let height = (line.page_height as f64).max(1.0);
    let mut features = Vec::with_capacity(66);
    lm2_context_onehot(&mut features, baseline_actions.get(index).copied());
    lm2_context_onehot(
        &mut features,
        index
            .checked_sub(1)
            .and_then(|idx| baseline_actions.get(idx))
            .copied(),
    );
    lm2_context_onehot(&mut features, baseline_actions.get(index + 1).copied());
    lm2_context_onehot(
        &mut features,
        index
            .checked_sub(2)
            .and_then(|idx| baseline_actions.get(idx))
            .copied(),
    );
    lm2_context_onehot(&mut features, baseline_actions.get(index + 2).copied());
    features.extend([
        line.left as f64 / width,
        line.right as f64 / width,
        line.top as f64 / height,
        line.bottom as f64 / height,
        line.font_height as f64,
        line.font_ratio_page as f64,
        line.font_ratio_doc as f64,
        line.doc_font_body_z as f64,
        line.doc_font_footnote_z as f64,
        bool_as_f64(line.bold),
        bool_as_f64(line.italic),
        bool_as_f64(line.centered),
        bool_as_f64(line.below_footnote_divider),
        bool_as_f64(line.in_footnote_zone),
        bool_as_f64(
            line.doc_repeated_top_edge
                || line.doc_repeated_bottom_edge
                || line.doc_repeated_edge_text,
        ),
        bool_as_f64(
            line.doc_vertical_axis_like
                || line.doc_vertical_numeric_axis_like
                || line.page_table_column_like,
        ),
        bool_as_f64(line.prev_line_has_dotleader || line.prev4_toc_leader_context),
        line.doc_note_marker as f64,
        bool_as_f64(line.doc_note_marker_first_on_page),
        bool_as_f64(line.doc_note_marker_mid_sequence_page),
        bool_as_f64(line.doc_note_marker_follows_previous_page),
        shape.char_count,
        shape.token_count,
        shape.short_text,
        shape.numeric_or_roman,
        shape.allcaps,
        shape.digit_ratio,
        shape.upper_ratio,
        shape.dotleader,
        shape.note_start,
        block.block_line_count.max(1) as f64,
        block.block_char_count as f64,
        block.block_short_ratio,
        block.block_numeric_ratio,
        block.block_dotleader,
        block.block_note_start_ratio,
        block.block_edge_ratio,
        block.block_axis_ratio,
        block.block_footzone_ratio,
        block.block_pos_norm,
        bool_as_f64(line.segment_block_first),
        bool_as_f64(line.segment_block_last),
        line.segment_block_line_count.max(1) as f64,
        line.segment_block_line_index as f64,
        bool_as_f64(line.segment_block_footnote_like),
        bool_as_f64(line.segment_block_furniture_like),
        bool_as_f64(line.segment_block_table_like),
        bool_as_f64(line.segment_block_toc_like),
    ]);
    lm2_context_onehot(
        &mut features,
        block
            .block_action
            .or_else(|| baseline_actions.get(index).copied()),
    );
    features
}

impl Lm2Model {
    fn doc_font_zscores_enabled(&self) -> bool {
        self.feature_schema
            .as_ref()
            .is_some_and(|schema| schema.doc_font_zscores)
    }

    fn repetition_fingerprints_enabled(&self) -> bool {
        self.feature_schema
            .as_ref()
            .is_some_and(|schema| schema.repetition_fingerprints)
    }

    fn marker_continuity_enabled(&self) -> bool {
        self.feature_schema
            .as_ref()
            .is_some_and(|schema| schema.marker_continuity)
    }
}

impl Lm2NativeCatboostModel {
    fn emission_scores(&self, line: &DeepLiquidSourceLine) -> Result<[f64; 3], String> {
        let feature_map = lm2_numeric_catboost_features(line);
        let float_features = LM2_NATIVE_CATBOOST_FLOAT_FEATURES
            .iter()
            .map(|name| feature_map.get(*name).copied().unwrap_or(0.0) as c_float)
            .collect::<Vec<_>>();
        let cat_values = lm2_native_catboost_cat_features(line);
        let cat_cstrings = cat_values
            .iter()
            .map(|value| {
                CString::new(value.as_str())
                    .map_err(|_| "NUL byte in categorical feature".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()?;
        let text = collapse_whitespace(&line.text)
            .chars()
            .take(500)
            .collect::<String>();
        let text_cstring =
            CString::new(text).map_err(|_| "NUL byte in CatBoost text feature".to_owned())?;

        let float_rows = [float_features.as_ptr()];
        let cat_ptrs = cat_cstrings
            .iter()
            .map(|value| value.as_ptr())
            .collect::<Vec<_>>();
        let cat_rows = [cat_ptrs.as_ptr()];
        let text_ptrs = [text_cstring.as_ptr()];
        let text_rows = [text_ptrs.as_ptr()];
        let mut raw = vec![0.0; self.dimensions_count];
        let ok = unsafe {
            (self.calc_model_prediction_text)(
                self.handle,
                1,
                float_rows.as_ptr(),
                float_features.len(),
                cat_rows.as_ptr(),
                cat_ptrs.len(),
                text_rows.as_ptr(),
                text_ptrs.len(),
                raw.as_mut_ptr(),
                raw.len(),
            )
        };
        if !ok {
            return Err(self.last_error());
        }
        if raw.len() != 3 {
            return Err(format!("native CatBoost returned {} dimensions", raw.len()));
        }
        // This promoted CBM reports classes in CatBoost/Python order:
        // ["hide_noise", "keep", "marginalia"].
        Ok([
            raw[1], // keep
            raw[2], // marginalia
            raw[0], // hide_noise
        ])
    }

    fn last_error(&self) -> String {
        let ptr = unsafe { (self.get_error_string)() };
        if ptr.is_null() {
            return "CatBoost C API error".to_owned();
        }
        unsafe { CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned()
    }
}

impl Drop for Lm2NativeCatboostModel {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe {
                (self.delete_model)(self.handle);
            }
            self.handle = std::ptr::null_mut();
        }
    }
}

impl Lm2NumericCatboostModel {
    fn is_usable(&self) -> bool {
        self.schema_version == "lm2-numeric-catboost-runtime-v1"
            && self.model_type == "numeric_catboost_oblivious_trees_v1"
            && self.classes.len() == ACTIONS.len()
            && self.bias.len() == self.classes.len()
            && !self.features.is_empty()
            && !self.trees.is_empty()
            && self.trees.iter().all(|tree| {
                let leaf_count = 1usize << tree.splits.len();
                tree.leaf_values.len() == leaf_count * self.classes.len()
                    && tree
                        .splits
                        .iter()
                        .all(|split| split.feature_index < self.features.len())
            })
    }

    fn emission_scores(&self, line: &DeepLiquidSourceLine) -> [f64; 3] {
        let features = lm2_numeric_catboost_features(line);
        let mut class_scores = self.bias.clone();
        class_scores.resize(self.classes.len(), 0.0);
        for tree in &self.trees {
            let mut leaf = 0usize;
            for (depth, split) in tree.splits.iter().enumerate() {
                let feature_name = &self.features[split.feature_index].name;
                let value = features.get(feature_name.as_str()).copied().unwrap_or(0.0);
                if value > split.border {
                    leaf |= 1usize << depth;
                }
            }
            let offset = leaf * self.classes.len();
            for (class_index, score) in class_scores.iter_mut().enumerate() {
                *score += self.scale * tree.leaf_values[offset + class_index];
            }
        }
        let mut action_scores = [0.0, 0.0, 0.0];
        for (class_index, class_name) in self.classes.iter().enumerate() {
            let action_index = match class_name.as_str() {
                "keep" => Lm2Action::Keep.index(),
                "marginalia" => Lm2Action::Marginalia.index(),
                "hide_noise" => Lm2Action::HideNoise.index(),
                _ => continue,
            };
            action_scores[action_index] = class_scores[class_index];
        }
        action_scores
    }
}

fn model_is_usable(model: &Lm2Model) -> bool {
    model.model_type == "hashed_softmax_action_v1"
        && model.actions == ACTIONS.map(|action| action.as_str().to_owned())
        && model.feature_dim > 0
        && model.bias.len() == ACTIONS.len()
        && model.weights.len() == ACTIONS.len()
        && model
            .weights
            .iter()
            .all(|weights| weights.len() == model.feature_dim)
}

fn load_lm2_model() -> Result<Lm2Model, String> {
    let path = lm2_model_candidates()
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| "No LM2 model found.".to_owned())?;
    let bytes =
        std::fs::read(&path).map_err(|error| format!("Could not read LM2 model: {error}"))?;
    serde_json::from_slice(&bytes).map_err(|error| format!("Could not decode LM2 model: {error}"))
}

fn load_lm2_native_catboost_model() -> Result<Option<Lm2NativeCatboostModel>, String> {
    let model_path = std::env::var_os("LAWPDF_LM2_NATIVE_CATBOOST_MODEL")
        .map(PathBuf::from)
        .or_else(|| {
            lm2_native_catboost_runtime_asset_candidates(LM2_NATIVE_CATBOOST_MODEL_FILE)
                .into_iter()
                .find(|path| path.is_file())
        });
    let Some(model_path) = model_path else {
        return Ok(None);
    };
    let lib_path = std::env::var_os("LAWPDF_LM2_NATIVE_CATBOOST_LIB")
        .map(PathBuf::from)
        .or_else(|| {
            lm2_native_catboost_runtime_asset_candidates(lm2_native_catboost_library_file())
                .into_iter()
                .find(|path| path.is_file())
        })
        .ok_or_else(|| {
            format!(
                "Native CatBoost model {} requires {} or LAWPDF_LM2_NATIVE_CATBOOST_LIB",
                model_path.display(),
                lm2_native_catboost_library_file()
            )
        })?;
    let library = unsafe { Library::new(&lib_path) }
        .map_err(|error| format!("Could not load native CatBoost library: {error}"))?;
    let (
        create_model,
        delete_model,
        load_full_model_from_file,
        get_float_features_count,
        get_cat_features_count,
        get_text_features_count,
        get_dimensions_count,
        get_error_string,
        calc_model_prediction_text,
    ) = unsafe {
        (
            *library
                .get::<CatboostCreateFn>(b"ModelCalcerCreate\0")
                .map_err(|error| format!("CatBoost library missing ModelCalcerCreate: {error}"))?,
            *library
                .get::<CatboostDeleteFn>(b"ModelCalcerDelete\0")
                .map_err(|error| format!("CatBoost library missing ModelCalcerDelete: {error}"))?,
            *library
                .get::<CatboostLoadFullModelFromFileFn>(b"LoadFullModelFromFile\0")
                .map_err(|error| {
                    format!("CatBoost library missing LoadFullModelFromFile: {error}")
                })?,
            *library
                .get::<CatboostGetCountFn>(b"GetFloatFeaturesCount\0")
                .map_err(|error| {
                    format!("CatBoost library missing GetFloatFeaturesCount: {error}")
                })?,
            *library
                .get::<CatboostGetCountFn>(b"GetCatFeaturesCount\0")
                .map_err(|error| {
                    format!("CatBoost library missing GetCatFeaturesCount: {error}")
                })?,
            *library
                .get::<CatboostGetCountFn>(b"GetTextFeaturesCount\0")
                .map_err(|error| {
                    format!("CatBoost library missing GetTextFeaturesCount: {error}")
                })?,
            *library
                .get::<CatboostGetCountFn>(b"GetDimensionsCount\0")
                .map_err(|error| format!("CatBoost library missing GetDimensionsCount: {error}"))?,
            *library
                .get::<CatboostGetErrorStringFn>(b"GetErrorString\0")
                .map_err(|error| format!("CatBoost library missing GetErrorString: {error}"))?,
            *library
                .get::<CatboostCalcModelPredictionTextFn>(b"CalcModelPredictionText\0")
                .map_err(|error| {
                    format!("CatBoost library missing CalcModelPredictionText: {error}")
                })?,
        )
    };

    let handle = unsafe { create_model() };
    if handle.is_null() {
        return Err("Could not create CatBoost model handle".to_owned());
    }
    let model_path_string = model_path.to_string_lossy().into_owned();
    let model_path_c = CString::new(model_path_string)
        .map_err(|_| "NUL byte in LAWPDF_LM2_NATIVE_CATBOOST_MODEL".to_owned())?;
    let loaded = unsafe { load_full_model_from_file(handle, model_path_c.as_ptr()) };
    if !loaded {
        let error = catboost_error_string(get_error_string);
        unsafe {
            delete_model(handle);
        }
        return Err(format!(
            "Could not load native CatBoost model {}: {error}",
            model_path.display()
        ));
    }
    let float_feature_count = unsafe { get_float_features_count(handle) };
    let cat_feature_count = unsafe { get_cat_features_count(handle) };
    let text_feature_count = unsafe { get_text_features_count(handle) };
    let dimensions_count = unsafe { get_dimensions_count(handle) };
    if float_feature_count != LM2_NATIVE_CATBOOST_FLOAT_FEATURES.len()
        || cat_feature_count != LM2_NATIVE_CATBOOST_CAT_FEATURES.len()
        || text_feature_count != 1
        || dimensions_count != 3
    {
        unsafe {
            delete_model(handle);
        }
        return Err(format!(
            "Native CatBoost feature contract mismatch: got f{float_feature_count}/c{cat_feature_count}/t{text_feature_count}/d{dimensions_count}, expected f{}/c{}/t1/d3",
            LM2_NATIVE_CATBOOST_FLOAT_FEATURES.len(),
            LM2_NATIVE_CATBOOST_CAT_FEATURES.len()
        ));
    }
    Ok(Some(Lm2NativeCatboostModel {
        _library: library,
        handle,
        delete_model,
        calc_model_prediction_text,
        get_error_string,
        float_feature_count,
        cat_feature_count,
        text_feature_count,
        dimensions_count,
    }))
}

fn catboost_error_string(get_error_string: CatboostGetErrorStringFn) -> String {
    let ptr = unsafe { get_error_string() };
    if ptr.is_null() {
        return "CatBoost C API error".to_owned();
    }
    unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned()
}

fn load_lm2_numeric_catboost_model() -> Result<Option<Lm2NumericCatboostModel>, String> {
    let path = std::env::var_os("LAWPDF_LM2_NUMERIC_CATBOOST_MODEL")
        .map(PathBuf::from)
        .or_else(|| {
            lm2_v20_stack_runtime_enabled()
                .then(|| lm2_v20_runtime_asset_candidates("lm2-numeric-catboost-runtime.json"))
                .and_then(|candidates| candidates.into_iter().find(|path| path.is_file()))
        });
    let Some(path) = path else { return Ok(None) };
    let bytes = std::fs::read(&path)
        .map_err(|error| format!("Could not read LM2 numeric CatBoost model: {error}"))?;
    let model = serde_json::from_slice::<Lm2NumericCatboostModel>(&bytes)
        .map_err(|error| format!("Could not decode LM2 numeric CatBoost model: {error}"))?;
    if !model.is_usable() {
        return Err("LM2 numeric CatBoost model is not usable".to_owned());
    }
    Ok(Some(model))
}

fn load_lm2_static_front_overlay() -> Result<Option<Lm2StaticFrontOverlay>, String> {
    let a55_path = std::env::var_os("LAWPDF_LM2_A55_OVERLAY")
        .map(PathBuf::from)
        .or_else(|| {
            lm2_v20_stack_runtime_enabled()
                .then(|| lm2_v20_runtime_asset_candidates("a55-front-stack-overlay.jsonl"))
                .and_then(|candidates| candidates.into_iter().find(|path| path.is_file()))
        });
    let d3_path = std::env::var_os("LAWPDF_LM2_D3_OVERLAY")
        .map(PathBuf::from)
        .or_else(|| {
            lm2_v20_stack_runtime_enabled()
                .then(|| lm2_v20_runtime_asset_candidates("d3-front-matter-regions.jsonl"))
                .and_then(|candidates| candidates.into_iter().find(|path| path.is_file()))
        });
    if a55_path.is_none() && d3_path.is_none() {
        return Ok(None);
    }

    let mut roles_by_doc_line: HashMap<String, HashMap<String, LiquidBlockRole>> = HashMap::new();
    let mut labels = Vec::new();
    if let Some(path) = a55_path {
        load_lm2_a55_overlay_rows(&path, &mut roles_by_doc_line)?;
        labels.push(format!("a55:{:016x}", fnv1a64(&path.display().to_string())));
    }
    if let Some(path) = d3_path {
        load_lm2_d3_overlay_rows(&path, &mut roles_by_doc_line)?;
        labels.push(format!("d3:{:016x}", fnv1a64(&path.display().to_string())));
    }
    if roles_by_doc_line.is_empty() {
        return Ok(None);
    }
    Ok(Some(Lm2StaticFrontOverlay {
        source_label: labels.join("+"),
        roles_by_doc_line,
    }))
}

fn load_lm2_a55_overlay_rows(
    path: &Path,
    roles_by_doc_line: &mut HashMap<String, HashMap<String, LiquidBlockRole>>,
) -> Result<(), String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("Could not read LM2 A55 overlay rows: {error}"))?;
    for (line_number, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let row = serde_json::from_str::<serde_json::Value>(line).map_err(|error| {
            format!(
                "Could not decode LM2 A55 overlay row {} in {}: {error}",
                line_number + 1,
                path.display()
            )
        })?;
        let doc = normalize_path_key_value(row.get("doc_path"));
        let line_id = row
            .get("line_id")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_owned();
        let role = row
            .get("to_role")
            .and_then(|value| value.as_str())
            .and_then(role_from_name);
        if doc.is_empty() || line_id.is_empty() {
            continue;
        }
        if let Some(role) = role {
            roles_by_doc_line
                .entry(doc)
                .or_default()
                .insert(line_id, role);
        }
    }
    Ok(())
}

fn load_lm2_d3_overlay_rows(
    path: &Path,
    roles_by_doc_line: &mut HashMap<String, HashMap<String, LiquidBlockRole>>,
) -> Result<(), String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("Could not read LM2 D3 overlay rows: {error}"))?;
    for (line_number, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let row = serde_json::from_str::<serde_json::Value>(line).map_err(|error| {
            format!(
                "Could not decode LM2 D3 overlay row {} in {}: {error}",
                line_number + 1,
                path.display()
            )
        })?;
        let doc = normalize_path_key_value(row.get("source_path"));
        let line_id = row
            .get("line_id")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_owned();
        let role = match row
            .get("front_matter_kind")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
        {
            "title" => Some(LiquidBlockRole::Title),
            "section_heading" => Some(LiquidBlockRole::Heading),
            "author" | "abstract" => Some(LiquidBlockRole::Marginalia),
            _ => None,
        };
        if doc.is_empty() || line_id.is_empty() {
            continue;
        }
        if let Some(role) = role {
            roles_by_doc_line
                .entry(doc)
                .or_default()
                .insert(line_id, role);
        }
    }
    Ok(())
}

fn normalize_path_key_value(value: Option<&serde_json::Value>) -> String {
    value
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .replace('\\', "/")
        .to_lowercase()
}

fn load_lm2_pp_priors() -> Result<Option<Lm2PpPriorIndex>, String> {
    let Some(path) = std::env::var_os("LAWPDF_LM2_PP_DRAFTS").map(PathBuf::from) else {
        return Ok(None);
    };
    load_lm2_pp_priors_from_path(path)
}

fn load_lm2_pp_priors_from_path(path: PathBuf) -> Result<Option<Lm2PpPriorIndex>, String> {
    let text = std::fs::read_to_string(&path)
        .map_err(|error| format!("Could not read LM2 PP draft rows: {error}"))?;
    let mut rows = HashMap::new();
    for (line_number, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let row: Lm2PpDraftRow = serde_json::from_str(line).map_err(|error| {
            format!(
                "Could not decode LM2 PP draft row {} in {}: {error}",
                line_number + 1,
                path.display()
            )
        })?;
        let action = row.draft_action.as_deref().or(row.pp_action.as_deref());
        let role = row.pp_role.unwrap_or_default();
        let label = row.pp_label.unwrap_or_default();
        let score = row.pp_score.unwrap_or_default();
        if !guarded_pp_prior_action(action, &role, &label, score, &row.text) {
            continue;
        }
        let source_path = if row.source_path.is_empty() {
            row.path
        } else {
            row.source_path
        };
        if source_path.is_empty() {
            continue;
        }
        rows.insert(
            pp_prior_key(&source_path, row.page_index, row.line_index, &row.text),
            Lm2PpPrior { role, label, score },
        );
    }
    if rows.is_empty() {
        return Ok(None);
    }
    Ok(Some(Lm2PpPriorIndex { source: path, rows }))
}

fn load_or_generate_lm2_pp_priors(
    path: &Path,
    source_lines: &[DeepLiquidSourceLine],
) -> Result<Option<Lm2PpPriorIndex>, String> {
    if source_lines.is_empty() {
        return Ok(None);
    }
    let cache_key = lm2_pp_doclayout_cache_key(path, source_lines);
    let draft_path = lm2_pp_doclayout_draft_cache_path(&cache_key)
        .ok_or_else(|| "could not find LM2 PP-DocLayout cache directory".to_owned())?;
    if draft_path.is_file() {
        return load_lm2_pp_priors_from_path(draft_path);
    }
    run_lm2_pp_doclayout_sidecar(path, source_lines, &cache_key, &draft_path)?;
    if draft_path.is_file() {
        return load_lm2_pp_priors_from_path(draft_path);
    }
    Ok(None)
}

fn lm2_pp_prior_index_has_footnotes(index: &Lm2PpPriorIndex) -> bool {
    index
        .rows
        .values()
        .any(|prior| prior.role == "footnote" && prior.score >= 0.80)
}

fn guarded_pp_prior_action(
    action: Option<&str>,
    role: &str,
    label: &str,
    score: f64,
    text: &str,
) -> bool {
    let compact_text = text.split_whitespace().collect::<String>();
    role == "footnote"
        && action == Some("marginalia")
        && score >= 0.80
        && !compact_text.chars().all(|ch| ch.is_ascii_digit())
        || role == "table" && action == Some("hide_noise") && score >= 0.70
        || label == "number" && action == Some("hide_noise") && score >= 0.80
}

fn lm2_model_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(dir) = std::env::var_os("LAWPDF_LM2_MODEL_DIR").map(PathBuf::from) {
        candidates.push(dir.join("lm2-model.json"));
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("profile-models/lm2-current/lm2-model.json"));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(exe_dir) = exe.parent()
    {
        candidates.push(exe_dir.join("profile-models/lm2-current/lm2-model.json"));
        candidates.push(exe_dir.join("../Resources/profile-models/lm2-current/lm2-model.json"));
        candidates.push(exe_dir.join("../../profile-models/lm2-current/lm2-model.json"));
    }
    candidates
}

fn lm2_v20_runtime_asset_candidates(file_name: &str) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("profile-models/lm2-v20-runtime").join(file_name));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(exe_dir) = exe.parent()
    {
        candidates.push(
            exe_dir
                .join("profile-models/lm2-v20-runtime")
                .join(file_name),
        );
        candidates.push(
            exe_dir
                .join("../Resources/profile-models/lm2-v20-runtime")
                .join(file_name),
        );
        candidates.push(
            exe_dir
                .join("../../profile-models/lm2-v20-runtime")
                .join(file_name),
        );
    }
    candidates
}

fn lm2_native_catboost_library_file() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "libcatboostmodel-darwin-universal2-1.2.10.dylib"
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        "libcatboostmodel-linux-x86_64-1.2.10.so"
    }
    #[cfg(target_os = "windows")]
    {
        "catboostmodel.dll"
    }
    #[cfg(not(any(
        target_os = "macos",
        all(target_os = "linux", target_arch = "x86_64"),
        target_os = "windows"
    )))]
    {
        "libcatboostmodel"
    }
}

pub(crate) fn lm2_native_catboost_default_asset_available() -> bool {
    lm2_native_catboost_runtime_asset_candidates(LM2_NATIVE_CATBOOST_MODEL_FILE)
        .into_iter()
        .any(|path| path.is_file())
        && lm2_native_catboost_runtime_asset_candidates(lm2_native_catboost_library_file())
            .into_iter()
            .any(|path| path.is_file())
}

fn lm2_context_twopass_enabled() -> bool {
    !falsey_env("LAWPDF_LM2_CONTEXT_TWOPASS")
}

fn load_lm2_context_twopass_model() -> Result<Option<Lm2ContextTwopassModel>, String> {
    let candidates = lm2_context_twopass_runtime_asset_candidates(LM2_CONTEXT_TWOPASS_MODEL_FILE);
    let Some(path) = candidates.into_iter().find(|path| path.is_file()) else {
        return Ok(None);
    };
    let input: Lm2ContextTwopassModelFile = read_json_file(&path)?;
    if input.feature_count != 66 {
        return Err(format!(
            "LM2 context two-pass model has unexpected feature count {}",
            input.feature_count
        ));
    }
    if input.actions != ["hide_noise", "keep", "marginalia"] {
        return Err(format!(
            "LM2 context two-pass model has unexpected action order {:?}",
            input.actions
        ));
    }
    Ok(Some(Lm2ContextTwopassModel {
        schema_version: input.schema_version,
        actions: input.actions,
        feature_count: input.feature_count,
        doc_to_fold: input
            .doc_to_fold
            .into_iter()
            .map(|(path, fold)| (lm2_context_norm_path(&path), fold))
            .collect(),
        unseen_doc_model: input.unseen_doc_model.unwrap_or_else(|| "full".to_owned()),
        models: input.models,
    }))
}

fn lm2_context_twopass_runtime_asset_candidates(file_name: &str) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os("LAWPDF_LM2_CONTEXT_TWOPASS_MODEL").map(PathBuf::from) {
        candidates.push(path);
    }
    if let Some(dir) = std::env::var_os("LAWPDF_LM2_CONTEXT_TWOPASS_DIR").map(PathBuf::from) {
        candidates.push(dir.join(file_name));
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join(LM2_CONTEXT_TWOPASS_RUNTIME_DIR).join(file_name));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(exe_dir) = exe.parent()
    {
        candidates.push(
            exe_dir
                .join(LM2_CONTEXT_TWOPASS_RUNTIME_DIR)
                .join(file_name),
        );
        candidates.push(
            exe_dir
                .join("../Resources")
                .join(LM2_CONTEXT_TWOPASS_RUNTIME_DIR)
                .join(file_name),
        );
        candidates.push(
            exe_dir
                .join("../../")
                .join(LM2_CONTEXT_TWOPASS_RUNTIME_DIR)
                .join(file_name),
        );
    }
    candidates
}

fn lm2_native_catboost_runtime_asset_candidates(file_name: &str) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(dir) = std::env::var_os("LAWPDF_LM2_NATIVE_CATBOOST_DIR").map(PathBuf::from) {
        candidates.push(dir.join(file_name));
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join(LM2_NATIVE_CATBOOST_RUNTIME_DIR).join(file_name));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(exe_dir) = exe.parent()
    {
        candidates.push(
            exe_dir
                .join(LM2_NATIVE_CATBOOST_RUNTIME_DIR)
                .join(file_name),
        );
        candidates.push(
            exe_dir
                .join("../Resources")
                .join(LM2_NATIVE_CATBOOST_RUNTIME_DIR)
                .join(file_name),
        );
        candidates.push(
            exe_dir
                .join("../../")
                .join(LM2_NATIVE_CATBOOST_RUNTIME_DIR)
                .join(file_name),
        );
    }
    candidates
}

fn decode_pages(
    runtime: &Lm2Runtime,
    lines: &[DeepLiquidSourceLine],
) -> Vec<(DeepLiquidSourceLine, Lm2Action)> {
    if runtime.native_catboost_model.is_some() {
        return lines
            .iter()
            .cloned()
            .map(|line| {
                let action = argmax_lm2_action(runtime.emission_scores(&line));
                (line, action)
            })
            .collect();
    }
    let mut decoded = Vec::with_capacity(lines.len());
    let mut start = 0usize;
    while start < lines.len() {
        let page = lines[start].page_index;
        let mut end = start + 1;
        while end < lines.len() && lines[end].page_index == page {
            end += 1;
        }
        decoded.extend(decode_page(runtime, &lines[start..end]));
        start = end;
    }
    decoded
}

fn decode_pages_with_external_emissions(
    runtime: &Lm2Runtime,
    document_path: &Path,
    lines: &[DeepLiquidSourceLine],
    external_emissions: &Lm2ExternalEmissions,
) -> Result<Vec<(DeepLiquidSourceLine, Lm2Action)>, String> {
    let mut decoded = Vec::with_capacity(lines.len());
    let mut start = 0usize;
    while start < lines.len() {
        let page = lines[start].page_index;
        let mut end = start + 1;
        while end < lines.len() && lines[end].page_index == page {
            end += 1;
        }
        let page_scores = external_emissions.page_scores(document_path, &lines[start..end])?;
        decoded.extend(decode_page_with_emissions(
            runtime,
            &lines[start..end],
            Some(&page_scores),
        ));
        start = end;
    }
    Ok(decoded)
}

fn decode_page(
    runtime: &Lm2Runtime,
    lines: &[DeepLiquidSourceLine],
) -> Vec<(DeepLiquidSourceLine, Lm2Action)> {
    decode_page_with_emissions(runtime, lines, None)
}

fn decode_page_with_emissions(
    runtime: &Lm2Runtime,
    lines: &[DeepLiquidSourceLine],
    external_emissions: Option<&[[f64; 3]]>,
) -> Vec<(DeepLiquidSourceLine, Lm2Action)> {
    if lines.is_empty() {
        return Vec::new();
    }
    debug_assert!(
        external_emissions.map_or(true, |emissions| emissions.len() == lines.len()),
        "external LM2 emissions must align one-for-one with decoded lines"
    );
    let emissions = external_emissions
        .map(|scores| scores.to_vec())
        .unwrap_or_else(|| {
            lines
                .iter()
                .map(|line| runtime.emission_scores(line))
                .collect::<Vec<_>>()
        });
    let mut dp = vec![[f64::NEG_INFINITY; 3]; lines.len()];
    let mut back = vec![[0usize; 3]; lines.len()];
    for action in ACTIONS {
        dp[0][action.index()] = emissions[0][action.index()]
            + runtime.start_score_scale * start_cost(lines[0].role_hint, action)
            + decoder_start_correction(runtime, &lines[0], action)
            + decoder_line_prior(runtime, &lines[0], action);
    }
    for index in 1..lines.len() {
        for current in ACTIONS {
            let current_index = current.index();
            let mut best_score = f64::NEG_INFINITY;
            let mut best_prev = 0usize;
            for previous in ACTIONS {
                let previous_index = previous.index();
                let score = dp[index - 1][previous_index]
                    + runtime.transition_score_scale
                        * transition_score(&lines[index - 1], &lines[index], previous, current)
                    + decoder_transition_correction(
                        runtime,
                        &lines[index - 1],
                        &lines[index],
                        previous,
                        current,
                    )
                    + decoder_transition_prior(
                        runtime,
                        &lines[index - 1],
                        &lines[index],
                        previous,
                        current,
                    )
                    + decoder_line_prior(runtime, &lines[index], current)
                    + emissions[index][current_index];
                if score > best_score {
                    best_score = score;
                    best_prev = previous_index;
                }
            }
            dp[index][current_index] = best_score;
            back[index][current_index] = best_prev;
        }
    }
    let mut current = (0..ACTIONS.len())
        .max_by(|left, right| {
            dp[lines.len() - 1][*left]
                .partial_cmp(&dp[lines.len() - 1][*right])
                .unwrap_or(Ordering::Equal)
        })
        .unwrap_or(0);
    let mut path = vec![Lm2Action::Keep; lines.len()];
    for index in (0..lines.len()).rev() {
        path[index] = ACTIONS[current];
        if index > 0 {
            current = back[index][current];
        }
    }
    if runtime.anchored_marginalia_flow_guard {
        apply_anchored_marginalia_flow_guard(lines, &mut path);
    }
    if runtime.pp_footnote_region_membership {
        apply_pp_footnote_region_membership(lines, &mut path);
    }
    if runtime.body_preservation_guard {
        apply_body_preservation_guard(lines, &mut path);
    }
    lines
        .iter()
        .cloned()
        .zip(path)
        .map(|(line, action)| {
            let action = final_lm2_action(&line, action);
            (line, action)
        })
        .collect()
}

fn apply_pp_footnote_region_membership(lines: &[DeepLiquidSourceLine], path: &mut [Lm2Action]) {
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

fn pp_footnote_region_member(line: &DeepLiquidSourceLine) -> bool {
    line.pp_prior_role.as_deref() == Some("footnote")
        && line.pp_prior_score.is_some_and(|score| score >= 0.80)
        && !line
            .text
            .split_whitespace()
            .collect::<String>()
            .chars()
            .all(|ch| ch.is_ascii_digit())
}

fn apply_pp_footnote_forward_closure(lines: &[DeepLiquidSourceLine], members: &mut [bool]) {
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

fn pp_footnote_forward_closure_eligible(line: &DeepLiquidSourceLine) -> bool {
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

fn pp_footnote_forward_continuation_like(line: &DeepLiquidSourceLine) -> bool {
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

fn pp_footnote_span_closure_eligible(line: &DeepLiquidSourceLine) -> bool {
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

fn pp_footnote_span_continuation_like(line: &DeepLiquidSourceLine) -> bool {
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

fn apply_body_preservation_guard(lines: &[DeepLiquidSourceLine], path: &mut [Lm2Action]) {
    for (line, action) in lines.iter().zip(path.iter_mut()) {
        if *action == Lm2Action::Keep {
            continue;
        }
        if body_preservation_candidate(line, *action) {
            *action = Lm2Action::Keep;
        }
    }
}

fn apply_static_front_overlay(
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

fn body_preservation_candidate(line: &DeepLiquidSourceLine, action: Lm2Action) -> bool {
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

fn apply_document_toc_overlay(decoded: &mut [(DeepLiquidSourceLine, Lm2Action)]) {
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

fn apply_front_matter_guard(decoded: &mut [(DeepLiquidSourceLine, Lm2Action)]) {
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

fn apply_marginalia_preservation_guard(decoded: &mut [(DeepLiquidSourceLine, Lm2Action)]) {
    for (line, action) in decoded.iter_mut() {
        if *action != Lm2Action::HideNoise || !marginalia_preservation_candidate(line) {
            continue;
        }
        *action = Lm2Action::Marginalia;
        line.role_hint = Some(LiquidBlockRole::Marginalia);
    }
}

fn marginalia_preservation_candidate(line: &DeepLiquidSourceLine) -> bool {
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

fn apply_d1_runtime_zerospend_overlay(decoded: &mut [(DeepLiquidSourceLine, Lm2Action)]) {
    for (line, action) in decoded.iter_mut() {
        if *action != Lm2Action::Keep || !d1_runtime_zerospend_candidate(line) {
            continue;
        }
        *action = Lm2Action::Marginalia;
        line.role_hint = Some(LiquidBlockRole::Footnote);
    }
}

fn d1_runtime_zerospend_candidate(line: &DeepLiquidSourceLine) -> bool {
    line.font_ratio_doc <= 1.0
        && line.font_ratio_page_ref <= 0.9
        && line.line_index >= 12
        && d1_runtime_zerospend_cue(&line.text)
}

fn d1_runtime_zerospend_cue(text: &str) -> bool {
    let lower = normalize_text(text);
    lower.contains("http://")
        || lower.contains("https://")
        || lower.contains("www.")
        || lower.contains("perma.cc")
        || has_legal_note_cue(&lower)
        || d1_runtime_zerospend_citation_cue(&lower)
}

fn d1_runtime_zerospend_citation_cue(lower: &str) -> bool {
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

fn d1_runtime_contains_token(text: &str, needle: &str) -> bool {
    text.split(|ch: char| !(ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-')))
        .any(|token| token == needle)
}

fn d1_runtime_year_paren_cue(text: &str) -> bool {
    text.split_whitespace().any(|token| {
        let token = token.trim_matches(|ch: char| matches!(ch, ',' | ';' | ':'));
        token.len() == 5 && token.ends_with(')') && token[..4].chars().all(|ch| ch.is_ascii_digit())
    })
}

fn apply_d1_runtime_continuation_overlay(decoded: &mut [(DeepLiquidSourceLine, Lm2Action)]) {
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

fn d1_runtime_continuation_candidate(line: &DeepLiquidSourceLine, previous_changed: bool) -> bool {
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

fn apply_d1_runtime_immediate_continuation_overlay(
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

fn apply_d1_runtime_sandwiched_continuation_overlay(
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

fn apply_d1_runtime_wide_sandwich_overlay(decoded: &mut [(DeepLiquidSourceLine, Lm2Action)]) {
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

fn d1_runtime_wide_sandwich_candidate(line: &DeepLiquidSourceLine) -> bool {
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

fn apply_d1_runtime_post_wide_cue_overlay(decoded: &mut [(DeepLiquidSourceLine, Lm2Action)]) {
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

fn d1_runtime_post_wide_cue_candidate(line: &DeepLiquidSourceLine) -> bool {
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

fn d1_runtime_strong_note_start(text: &str) -> bool {
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

fn d1_runtime_next_marginalia_count(
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

fn apply_d1_runtime_postcue_citation_next1_overlay(
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

fn d1_runtime_postcue_citation_next1_candidate(line: &DeepLiquidSourceLine) -> bool {
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

fn d1_runtime_postcue_citation_next1_cue(lower: &str, text: &str) -> bool {
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

fn apply_d1_runtime_near8_cue_overlay(decoded: &mut [(DeepLiquidSourceLine, Lm2Action)]) {
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

fn d1_runtime_near8_cue_candidate(line: &DeepLiquidSourceLine) -> bool {
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

fn d1_runtime_near8_citation_cue(lower: &str, text: &str) -> bool {
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

fn d1_runtime_nearby_marginalia_count(
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

fn apply_d1_runtime_geometric_zone_overlay(decoded: &mut [(DeepLiquidSourceLine, Lm2Action)]) {
    for (line, action) in decoded.iter_mut() {
        if *action != Lm2Action::Keep || !d1_runtime_geometric_zone_candidate(line) {
            continue;
        }
        *action = Lm2Action::Marginalia;
        line.role_hint = Some(LiquidBlockRole::Footnote);
    }
}

fn d1_runtime_geometric_zone_candidate(line: &DeepLiquidSourceLine) -> bool {
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

fn apply_d1_runtime_wide_divider_guard_overlay(decoded: &mut [(DeepLiquidSourceLine, Lm2Action)]) {
    for (line, action) in decoded.iter_mut() {
        if *action != Lm2Action::Keep || !d1_runtime_wide_divider_guard_candidate(line) {
            continue;
        }
        *action = Lm2Action::Marginalia;
        line.role_hint = Some(LiquidBlockRole::Footnote);
    }
}

fn tfr_num_token_count(text: &str) -> usize {
    text.split_whitespace()
        .filter(|w| w.chars().any(|c| c.is_ascii_digit()))
        .count()
}

fn tfr_alpha_word_count(text: &str) -> usize {
    text.split_whitespace()
        .filter(|w| w.chars().filter(|c| c.is_alphabetic()).count() >= 3)
        .count()
}

fn tfr_ends_with_sentence_punct(text: &str) -> bool {
    text.trim_end()
        .chars()
        .last()
        .map(|c| matches!(c, '.' | ';' | ':' | ','))
        .unwrap_or(false)
}

fn tfr_has_bullet(text: &str) -> bool {
    text.chars().any(|c| {
        matches!(
            c,
            '\u{2022}' | '\u{25CF}' | '\u{25AA}' | '\u{25E6}' | '\u{2023}'
        )
    })
}

fn tfr_width_norm(line: &DeepLiquidSourceLine) -> f32 {
    (line.right - line.left).max(0.0) / line.page_width.max(1.0)
}

/// Strong PDF-native table/figure signals that do not need page context:
/// figure bullets (short, non-prose) and number-dense grid rows. Prose and
/// citations are excluded (>=3 alpha words or sentence-ending punctuation), and
/// full-width lines are excluded.
fn table_figure_router_strong_candidate(line: &DeepLiquidSourceLine) -> bool {
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
fn table_figure_router_cell_like(line: &DeepLiquidSourceLine) -> bool {
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
fn table_figure_router_repeated_furniture_like(line: &DeepLiquidSourceLine) -> bool {
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
fn apply_table_figure_router_overlay(decoded: &mut [(DeepLiquidSourceLine, Lm2Action)]) {
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

fn apply_page_object_overlay(decoded: &mut [(DeepLiquidSourceLine, Lm2Action)]) {
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

fn page_object_overlay_candidate(line: &DeepLiquidSourceLine) -> bool {
    line.page_object_hide_candidate_guarded
        || line.page_object_ruled_row_membership
        || line.page_object_path15_candidate
        || line.page_object_ruled_or_path8_candidate && page_object_short_nonprose(line)
}

fn page_object_short_nonprose(line: &DeepLiquidSourceLine) -> bool {
    let text = collapse_whitespace(&line.text);
    !text.is_empty()
        && text.chars().count() <= 90
        && tfr_alpha_word_count(&text) < 8
        && !tfr_ends_with_sentence_punct(&text)
}

fn apply_page_object_tuned_overlay(decoded: &mut [(DeepLiquidSourceLine, Lm2Action)]) {
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

fn page_object_tuned_body_rescue_candidate(line: &DeepLiquidSourceLine) -> bool {
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

fn page_object_tuned_keep_preserve_candidate(line: &DeepLiquidSourceLine) -> bool {
    page_object_tuned_legal_table_keep_like(line) || page_object_tuned_prose_keep_like(line)
}

fn page_object_tuned_legal_table_keep_like(line: &DeepLiquidSourceLine) -> bool {
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

fn page_object_tuned_prose_keep_like(line: &DeepLiquidSourceLine) -> bool {
    let text = collapse_whitespace(&line.text);
    !text.is_empty()
        && tfr_alpha_word_count(&text) >= 8
        && tfr_width_norm(line) >= 0.45
        && digit_density(&text) <= 0.20
}

fn page_object_tuned_edge_line(line: &DeepLiquidSourceLine) -> bool {
    let page_height = line.page_height.max(1.0);
    line.top < 45.0 || line.bottom > page_height - 45.0
}

fn digit_density(text: &str) -> f32 {
    let total = text.chars().count().max(1) as f32;
    let digits = text.chars().filter(|ch| ch.is_ascii_digit()).count() as f32;
    digits / total
}

fn d1_runtime_wide_divider_guard_candidate(line: &DeepLiquidSourceLine) -> bool {
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

fn apply_d1_runtime_footer_artifact_overlay(decoded: &mut [(DeepLiquidSourceLine, Lm2Action)]) {
    for (line, action) in decoded.iter_mut() {
        if *action == Lm2Action::HideNoise || !d1_runtime_footer_artifact_candidate(&line.text) {
            continue;
        }
        *action = Lm2Action::HideNoise;
        line.role_hint = Some(LiquidBlockRole::Noise);
    }
}

fn d1_runtime_footer_artifact_candidate(text: &str) -> bool {
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

fn apply_footnote_carryover_overlay(decoded: &mut [(DeepLiquidSourceLine, Lm2Action)]) {
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
struct FootnoteCarryoverState {
    open: bool,
    expected_marker: Option<u16>,
}

fn footnote_carryover_page_state(
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
            .filter(|marker| *marker <= 500),
    }
}

fn footnote_carryover_line_is_note(row: &(DeepLiquidSourceLine, Lm2Action)) -> bool {
    row.1 == Lm2Action::Marginalia && !footnote_monotone_reject_line(&row.0)
}

fn footnote_carryover_last_marker(
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

fn footnote_carryover_has_terminal_punctuation(text: &str) -> bool {
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

fn footnote_carryover_dividerless_page(
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

fn footnote_carryover_candidate(line: &DeepLiquidSourceLine, dividerless_page: bool) -> bool {
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

fn footnote_carryover_footnote_font_line(line: &DeepLiquidSourceLine) -> bool {
    let font_ratio = footnote_monotone_font_ratio(line);
    font_ratio <= 0.92
        || line.font_ratio_page_ref <= 0.90
        || (line.doc_font_footnote_size > 0.0
            && line.font_height <= line.doc_font_footnote_size + 0.35)
        || line.doc_font_footnote_z.abs() + 0.15 <= line.doc_font_body_z.abs()
}

fn footnote_carryover_body_font_line(line: &DeepLiquidSourceLine) -> bool {
    footnote_monotone_font_ratio(line) >= 0.97
        && line.font_ratio_page_ref >= 0.96
        && line.doc_font_body_z.abs() <= line.doc_font_footnote_z.abs() + 0.10
}

fn footnote_carryover_body_resume_like(
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

fn d1_runtime_has_marginalia_neighbor(
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

fn apply_d1_runtime_safe_numeric_note_overlay(decoded: &mut [(DeepLiquidSourceLine, Lm2Action)]) {
    for (line, action) in decoded.iter_mut() {
        if *action != Lm2Action::Keep || !d1_runtime_safe_numeric_note_candidate(line) {
            continue;
        }
        *action = Lm2Action::Marginalia;
        line.role_hint = Some(LiquidBlockRole::Footnote);
    }
}

fn d1_runtime_safe_numeric_note_candidate(line: &DeepLiquidSourceLine) -> bool {
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

fn d1_runtime_strong_numeric_note_start(text: &str) -> bool {
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

fn d1_runtime_citation_like(lower: &str) -> bool {
    lower.contains("http://")
        || lower.contains("https://")
        || lower.contains("www.")
        || lower.contains("perma.cc")
        || lower.contains("supra")
        || lower.contains("ibid")
        || d1_runtime_contains_token(lower, "id.")
        || d1_runtime_zerospend_citation_cue(lower)
}

fn apply_footnote_monotone_overlay(decoded: &mut [(DeepLiquidSourceLine, Lm2Action)]) {
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
struct OpenFootnoteCarryoverState {
    expected_next_marker: Option<u16>,
}

fn apply_open_footnote_carryover_overlay(decoded: &mut [(DeepLiquidSourceLine, Lm2Action)]) {
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

fn open_footnote_carryover_page_tail_state(
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

fn open_footnote_carryover_tail_candidate(line: &DeepLiquidSourceLine, action: Lm2Action) -> bool {
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

fn open_footnote_carryover_candidate(line: &DeepLiquidSourceLine) -> bool {
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

fn open_footnote_carryover_body_resume_candidate(line: &DeepLiquidSourceLine) -> bool {
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

fn open_footnote_carryover_reject_line(line: &DeepLiquidSourceLine) -> bool {
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

fn open_footnote_carryover_footnote_font(line: &DeepLiquidSourceLine) -> bool {
    let ratio = footnote_monotone_font_ratio(line);
    ratio <= 0.96
        || (line.doc_font_footnote_size > 0.0
            && line.font_height > 0.0
            && line.font_height <= line.doc_font_footnote_size + 0.45)
        || (line.doc_font_footnote_z != 0.0
            && line.doc_font_body_z != 0.0
            && line.doc_font_footnote_z.abs() + 0.20 < line.doc_font_body_z.abs())
}

fn open_footnote_carryover_has_terminal_punctuation(text: &str) -> bool {
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

fn footnote_monotone_anchor_candidate(
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

fn footnote_monotone_expected_missing_markers(
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

fn footnote_body_reference_markers(decoded: &[(DeepLiquidSourceLine, Lm2Action)]) -> HashSet<u16> {
    let mut markers = HashSet::new();
    for (line, action) in decoded {
        if *action != Lm2Action::Keep || !footnote_body_reference_line_candidate(line) {
            continue;
        }
        markers.extend(footnote_reference_markers_in_text(&line.text));
    }
    markers
}

fn footnote_body_reference_line_candidate(line: &DeepLiquidSourceLine) -> bool {
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

fn footnote_reference_markers_in_text(text: &str) -> Vec<u16> {
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

fn push_marker_digits(markers: &mut Vec<u16>, digits: &mut String) {
    if digits.is_empty() {
        return;
    }
    if let Ok(value) = digits.parse::<u16>()
        && (1..=500).contains(&value)
    {
        markers.push(value);
    }
    digits.clear();
}

fn superscript_digit_value(ch: char) -> Option<char> {
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

fn attached_terminal_ascii_marker(text: &str) -> Option<u16> {
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
    (1..=500).contains(&value).then_some(value)
}

fn footnote_monotone_continuation_candidate(line: &DeepLiquidSourceLine) -> bool {
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

fn footnote_monotone_reject_line(line: &DeepLiquidSourceLine) -> bool {
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

fn footnote_monotone_font_ratio(line: &DeepLiquidSourceLine) -> f32 {
    if line.font_ratio_doc > 0.0 {
        line.font_ratio_doc
    } else {
        line.font_ratio_page
    }
}

fn d1_runtime_immediate_continuation_candidate(line: &DeepLiquidSourceLine) -> bool {
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

fn d1_runtime_artifact_like(text: &str) -> bool {
    text.contains('\u{0002}') || text.trim_start().starts_with("** ")
}

fn d1_runtime_table_stat_like(text: &str) -> bool {
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

fn apply_page_label_furniture_guard(decoded: &mut [(DeepLiquidSourceLine, Lm2Action)]) {
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

fn looks_like_bare_page_number_furniture(line: &DeepLiquidSourceLine) -> bool {
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

fn looks_like_page_label_furniture(text: &str) -> bool {
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

fn noise_hint_page_furniture(line: &DeepLiquidSourceLine) -> bool {
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

fn first_page_journal_masthead(line: &DeepLiquidSourceLine, lower: &str) -> bool {
    let y_top = line.top / line.page_height.max(1.0);
    if y_top < 0.75 || word_count(lower) > 7 {
        return false;
    }
    (lower.contains("law review") || lower.contains("law journal"))
        && (line.centered || uppercase_ratio(&line.text) >= 0.70)
}

fn first_page_author_note_line(line: &DeepLiquidSourceLine) -> bool {
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

fn lm2_toc_dotleader_line(text: &str) -> bool {
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

fn lm2_toc_dotleader_fallback(line: &DeepLiquidSourceLine) -> bool {
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

fn lm2_toc_has_long_dotleader(text: &str) -> bool {
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

fn lm2_toc_trails_page_number_or_roman(text: &str) -> bool {
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

fn lm2_toc_overlay_repeated_edge(line: &DeepLiquidSourceLine) -> bool {
    line.doc_repeated_top_edge || line.doc_repeated_bottom_edge || line.doc_repeated_edge_text
}

fn lm2_toc_normalize(text: &str, is_dotleader: bool) -> String {
    let mut value = if is_dotleader {
        lm2_toc_strip_trailing_dots_and_page(text)
    } else {
        text.to_owned()
    };
    value = lm2_toc_strip_enumerator(&value);
    value = lm2_toc_strip_enumerator(&value);
    collapse_whitespace(&value.to_lowercase()).trim().to_owned()
}

fn lm2_toc_strip_trailing_dots_and_page(text: &str) -> String {
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

fn lm2_toc_strip_enumerator(text: &str) -> String {
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

fn decoder_line_prior(runtime: &Lm2Runtime, line: &DeepLiquidSourceLine, action: Lm2Action) -> f64 {
    let mut score = 0.0;
    if runtime.marker_decoder_prior {
        score += marker_continuity_decoder_prior(line, action);
    }
    if runtime.small_font_decoder_prior {
        score += small_font_lower_page_decoder_prior(line, action);
    }
    score
}

fn decoder_transition_prior(
    runtime: &Lm2Runtime,
    _previous_line: &DeepLiquidSourceLine,
    line: &DeepLiquidSourceLine,
    previous: Lm2Action,
    current: Lm2Action,
) -> f64 {
    let mut score = 0.0;
    if runtime.small_font_sequence_prior {
        score += small_font_sequence_continuation_prior(line, previous, current);
    }
    score
}

fn small_font_sequence_continuation_prior(
    line: &DeepLiquidSourceLine,
    previous: Lm2Action,
    current: Lm2Action,
) -> f64 {
    if previous != Lm2Action::Marginalia || current != Lm2Action::Marginalia {
        return 0.0;
    }
    if !small_font_lower_page_decoder_prior_eligible(line) {
        return 0.0;
    }
    2.1
}

fn marker_continuity_decoder_prior(line: &DeepLiquidSourceLine, action: Lm2Action) -> f64 {
    if !marker_continuity_decoder_prior_eligible(line) {
        return 0.0;
    }
    match action {
        Lm2Action::Marginalia => 1.6,
        Lm2Action::Keep => -0.2,
        Lm2Action::HideNoise => 0.0,
    }
}

fn marker_continuity_decoder_prior_eligible(line: &DeepLiquidSourceLine) -> bool {
    let lower = normalize_text(&line.text);
    if looks_like_toc_entry(&lower) || looks_like_running_header(&lower) {
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

    let marker_signal = line.doc_note_marker > 0
        || line.doc_note_marker_mid_sequence_page
        || line.doc_note_marker_follows_previous_page;
    if !marker_signal {
        return false;
    }

    let y_bottom = line.bottom / line.page_height.max(1.0);
    let small_font = line.font_ratio_page < 0.90 || line.font_ratio_doc < 0.90;
    looks_like_note_start(&line.text)
        || has_legal_note_cue(&lower)
        || line.below_footnote_divider
        || y_bottom < 0.26
        || small_font && y_bottom < 0.42
}

fn small_font_lower_page_decoder_prior(line: &DeepLiquidSourceLine, action: Lm2Action) -> f64 {
    if !small_font_lower_page_decoder_prior_eligible(line) {
        return 0.0;
    }
    match action {
        Lm2Action::Marginalia => 2.25,
        Lm2Action::Keep => -0.20,
        Lm2Action::HideNoise => 0.0,
    }
}

fn small_font_lower_page_decoder_prior_eligible(line: &DeepLiquidSourceLine) -> bool {
    let lower = normalize_text(&line.text);
    if looks_like_toc_entry(&lower) || looks_like_running_header(&lower) {
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
    if looks_like_small_font_page_furniture(&lower) {
        return false;
    }
    if word_count(&lower) <= 8
        && uppercase_ratio(&line.text) >= 0.62
        && !looks_like_note_start(&line.text)
    {
        return false;
    }

    let y_bottom = line.bottom / line.page_height.max(1.0);
    y_bottom < 0.34
        && (line.font_ratio_doc < 0.90 || line.font_ratio_page < 0.84)
        && small_font_note_evidence(line, &lower)
}

fn small_font_note_evidence(line: &DeepLiquidSourceLine, lower: &str) -> bool {
    looks_like_note_start(&line.text)
        || has_legal_note_cue(lower)
        || line.below_footnote_divider
        || line.page_has_footnote_divider
        || line.doc_footnote_state
        || line.doc_footnote_continuation
        || line.doc_note_marker > 0
        || line.doc_note_marker_mid_sequence_page
        || line.doc_note_marker_follows_previous_page
}

fn hard_marginalia_anchor(line: &DeepLiquidSourceLine) -> bool {
    let lower = normalize_text(&line.text);
    small_font_note_evidence(line, &lower) || looks_like_marginalia_note_block_start(&line.text)
}

fn apply_anchored_marginalia_flow_guard(lines: &[DeepLiquidSourceLine], path: &mut [Lm2Action]) {
    let mut anchored_run_active = false;
    let mut unanchored_after_anchor = 0usize;
    for (line, action) in lines.iter().zip(path.iter_mut()) {
        if *action != Lm2Action::Marginalia {
            anchored_run_active = false;
            unanchored_after_anchor = 0;
            continue;
        }

        if hard_marginalia_anchor(line) {
            anchored_run_active = true;
            unanchored_after_anchor = 0;
            continue;
        }

        if !anchored_run_active {
            *action = Lm2Action::Keep;
            continue;
        }

        unanchored_after_anchor += 1;
        if unanchored_after_anchor > 2 {
            *action = Lm2Action::Keep;
        }
    }
}

fn looks_like_small_font_page_furniture(lower: &str) -> bool {
    let trimmed = lower.trim();
    if trimmed.is_empty() {
        return false;
    }
    if trimmed.starts_with("[vol.") || trimmed.starts_with("vol.") {
        return true;
    }
    if trimmed.starts_with("page ")
        && trimmed
            .split_whitespace()
            .nth(1)
            .is_some_and(|token| token.chars().all(|ch| ch.is_ascii_digit()))
    {
        return true;
    }
    if trimmed.starts_with("copyright ")
        || trimmed.contains("all rights reserved")
        || trimmed.contains("published by ")
        || trimmed.contains("brought to you for free and open access")
        || trimmed.contains("accepted for inclusion")
        || trimmed.contains("law archive of scholarship")
        || trimmed.contains("scholar commons")
        || trimmed.contains("scholarcommons")
        || trimmed.contains("for more information, please contact")
        || trimmed.contains("journal of legal studies")
        || trimmed.contains("university press")
        || trimmed.contains("footnote continued")
        || trimmed.contains("received:") && trimmed.contains("revised:")
    {
        return true;
    }
    if trimmed.contains('@') || trimmed.contains("http://") || trimmed.contains("https://") {
        return true;
    }
    false
}

fn final_lm2_action(line: &DeepLiquidSourceLine, action: Lm2Action) -> Lm2Action {
    if looks_like_production_slug_boilerplate(&line.text) {
        Lm2Action::HideNoise
    } else {
        action
    }
}

fn start_cost(role_hint: Option<LiquidBlockRole>, action: Lm2Action) -> f64 {
    match (role_hint, action) {
        (Some(role), action) if role_action(role) == action => 0.6,
        (_, Lm2Action::HideNoise) => 0.15,
        _ => 0.0,
    }
}

fn transition_score(
    previous_line: &DeepLiquidSourceLine,
    line: &DeepLiquidSourceLine,
    previous: Lm2Action,
    current: Lm2Action,
) -> f64 {
    if previous == current {
        return match current {
            Lm2Action::Keep => 0.52,
            Lm2Action::Marginalia => 0.82,
            Lm2Action::HideNoise => 0.62,
        };
    }
    let gap = vertical_gap(previous_line, line);
    let font_drop = line.font_ratio_page < 0.88 || line.font_ratio_doc < 0.88;
    let below_divider = line.below_footnote_divider || previous_line.below_footnote_divider;
    match (previous, current) {
        (Lm2Action::Keep, Lm2Action::Marginalia) if below_divider || font_drop => 0.20,
        (Lm2Action::Marginalia, Lm2Action::Keep) if !below_divider && gap > 0.035 => -0.25,
        (Lm2Action::HideNoise, Lm2Action::Keep) if gap > 0.025 => 0.05,
        (Lm2Action::Keep, Lm2Action::HideNoise) if is_edge_line(line) => -0.05,
        (Lm2Action::HideNoise, Lm2Action::Marginalia) if below_divider => 0.0,
        (Lm2Action::Marginalia, Lm2Action::HideNoise) if is_edge_line(line) => -0.15,
        (Lm2Action::Keep, Lm2Action::Marginalia) => -1.05,
        (Lm2Action::Marginalia, Lm2Action::Keep) => -1.25,
        _ => -0.58,
    }
}

fn decoder_start_correction(
    runtime: &Lm2Runtime,
    line: &DeepLiquidSourceLine,
    action: Lm2Action,
) -> f64 {
    let Some(weights) = runtime.decoder_weights() else {
        return 0.0;
    };
    let action_name = action.as_str();
    let mut score = *weights
        .get(&format!("start_arc:{action_name}"))
        .unwrap_or(&0.0);
    for (name, value) in start_feature_map(line) {
        score += weights
            .get(&format!("start_feature:{action_name}:{name}"))
            .copied()
            .unwrap_or(0.0)
            * value;
    }
    score
}

fn decoder_transition_correction(
    runtime: &Lm2Runtime,
    previous_line: &DeepLiquidSourceLine,
    line: &DeepLiquidSourceLine,
    previous: Lm2Action,
    current: Lm2Action,
) -> f64 {
    let Some(weights) = runtime.decoder_weights() else {
        return 0.0;
    };
    let arc = format!("{}->{}", previous.as_str(), current.as_str());
    let mut score = *weights
        .get(&format!("transition_arc:{arc}"))
        .unwrap_or(&0.0);
    for (name, value) in transition_feature_map(previous_line, line) {
        score += weights
            .get(&format!("transition_feature:{arc}:{name}"))
            .copied()
            .unwrap_or(0.0)
            * value;
    }
    score
}

fn apply_layout_priors(line: &DeepLiquidSourceLine, scores: &mut [f64; 3]) {
    let lower = normalize_text(&line.text);
    let words = word_count(&lower);
    let y_bottom = line.bottom / line.page_height.max(1.0);
    let role_action = line.role_hint.map(role_action);
    if let Some(action) = role_action {
        let note_zone =
            line.below_footnote_divider || y_bottom < 0.26 || looks_like_note_start(&line.text);
        scores[action.index()] += match action {
            Lm2Action::Keep => 0.8,
            Lm2Action::Marginalia if note_zone => 0.65,
            Lm2Action::Marginalia => 0.0,
            Lm2Action::HideNoise => 0.9,
        };
    }
    if line.below_footnote_divider {
        scores[Lm2Action::Marginalia.index()] += 2.1;
        scores[Lm2Action::Keep.index()] -= 0.8;
    }
    if line.page_has_footnote_divider && line.bottom / line.page_height.max(1.0) < 0.32 {
        scores[Lm2Action::Marginalia.index()] += 0.75;
    }
    if (line.font_ratio_page < 0.84 || line.font_ratio_doc < 0.84)
        && (line.below_footnote_divider
            || y_bottom < 0.36
            || looks_like_note_start(&line.text)
            || has_legal_note_cue(&lower))
    {
        scores[Lm2Action::Marginalia.index()] += 0.72;
    }
    if !line.below_footnote_divider && y_bottom > 0.22 && !looks_like_note_start(&line.text) {
        scores[Lm2Action::Keep.index()] += 1.5;
        scores[Lm2Action::Marginalia.index()] -= 1.5;
    }
    if line.centered && (line.font_ratio_page > 1.08 || words <= 12) {
        scores[Lm2Action::Keep.index()] += 0.55;
    }
    if line.bold && words <= 14 && line.font_ratio_page >= 0.98 {
        scores[Lm2Action::Keep.index()] += 0.45;
    }
    if looks_like_note_start(&line.text) || has_legal_note_cue(&lower) {
        scores[Lm2Action::Marginalia.index()] += 0.9;
    }
    if looks_like_toc_entry(&lower) || is_edge_line(line) && words <= 8 {
        scores[Lm2Action::HideNoise.index()] += 0.9;
    }
    if lower.len() <= 4 && lower.chars().all(|ch| ch.is_ascii_digit()) {
        scores[Lm2Action::HideNoise.index()] += 6.0;
        scores[Lm2Action::Keep.index()] -= 1.0;
        scores[Lm2Action::Marginalia.index()] -= 1.0;
    }
    if looks_like_running_header(&lower) {
        scores[Lm2Action::HideNoise.index()] += 10.0;
        scores[Lm2Action::Keep.index()] -= 5.0;
        scores[Lm2Action::Marginalia.index()] -= 1.0;
    }
}

fn apply_pp_priors(runtime: &Lm2Runtime, line: &DeepLiquidSourceLine, scores: &mut [f64; 3]) {
    if runtime.pp_footnote_region_membership {
        return;
    }
    let Some(prior) = runtime.pp_prior_for_line(line) else {
        return;
    };
    if prior.role == "footnote" && prior.score >= 0.80 {
        scores[Lm2Action::Marginalia.index()] += 3.8;
        scores[Lm2Action::Keep.index()] -= 1.2;
        scores[Lm2Action::HideNoise.index()] -= 0.8;
    } else if prior.role == "table" && prior.score >= 0.70 {
        scores[Lm2Action::HideNoise.index()] += 3.2;
        scores[Lm2Action::Keep.index()] -= 0.7;
        scores[Lm2Action::Marginalia.index()] -= 0.7;
    } else if prior.label == "number" && prior.score >= 0.80 {
        scores[Lm2Action::HideNoise.index()] += 3.5;
        scores[Lm2Action::Keep.index()] -= 0.8;
        scores[Lm2Action::Marginalia.index()] -= 0.8;
    }
}

fn action_scores_map(scores: [f64; 3]) -> BTreeMap<String, f64> {
    ACTIONS
        .iter()
        .map(|action| (action.as_str().to_owned(), scores[action.index()]))
        .collect()
}

fn start_scores_map(role_hint: Option<LiquidBlockRole>, scale: f64) -> BTreeMap<String, f64> {
    ACTIONS
        .iter()
        .map(|action| {
            (
                action.as_str().to_owned(),
                scale * start_cost(role_hint, *action),
            )
        })
        .collect()
}

fn start_feature_map(line: &DeepLiquidSourceLine) -> BTreeMap<String, f64> {
    let mut features = BTreeMap::new();
    features.insert("bias".to_owned(), 1.0);
    features.insert("first_action_hide_noise".to_owned(), 1.0);
    features.insert(
        "doc_footnote_state".to_owned(),
        bool_as_f64(line.doc_footnote_state),
    );
    features.insert(
        "doc_footnote_continuation".to_owned(),
        bool_as_f64(line.doc_footnote_continuation),
    );
    features.insert(
        "doc_footnote_continuation_no_divider".to_owned(),
        bool_as_f64(line.doc_footnote_continuation && !line.page_has_footnote_divider),
    );
    features.insert(
        "doc_footnote_continuation_small_font".to_owned(),
        bool_as_f64(
            line.doc_footnote_continuation
                && (line.font_ratio_page < 0.94 || line.font_ratio_doc < 0.94),
        ),
    );
    features.insert(
        "doc_note_marker_mid_sequence_page".to_owned(),
        bool_as_f64(line.doc_note_marker_mid_sequence_page),
    );
    features.insert(
        "doc_note_marker_follows_previous_page".to_owned(),
        bool_as_f64(line.doc_note_marker_follows_previous_page),
    );
    features.insert(
        "doc_note_marker_first_on_page".to_owned(),
        bool_as_f64(line.doc_note_marker_first_on_page),
    );
    features.insert(
        "doc_note_marker_present".to_owned(),
        bool_as_f64(line.doc_note_marker > 0),
    );
    if let Some(role) = line.role_hint {
        features.insert(
            format!("role_hint_action:{}", role_action(role).as_str()),
            1.0,
        );
    }
    features
}

fn transition_scores_map(
    previous_line: &DeepLiquidSourceLine,
    line: &DeepLiquidSourceLine,
    scale: f64,
) -> BTreeMap<String, f64> {
    let mut scores = BTreeMap::new();
    for previous in ACTIONS {
        for current in ACTIONS {
            scores.insert(
                format!("{}->{}", previous.as_str(), current.as_str()),
                scale * transition_score(previous_line, line, previous, current),
            );
        }
    }
    scores
}

fn transition_feature_map(
    previous_line: &DeepLiquidSourceLine,
    line: &DeepLiquidSourceLine,
) -> BTreeMap<String, f64> {
    let mut features = BTreeMap::new();
    let gap = vertical_gap(previous_line, line) as f64;
    let font_drop = line.font_ratio_page < 0.88 || line.font_ratio_doc < 0.88;
    let below_divider_pair = line.below_footnote_divider || previous_line.below_footnote_divider;
    let current_edge_line = is_edge_line(line);
    features.insert("bias".to_owned(), 1.0);
    features.insert("vertical_gap".to_owned(), gap);
    features.insert("font_drop".to_owned(), bool_as_f64(font_drop));
    features.insert(
        "below_divider_pair".to_owned(),
        bool_as_f64(below_divider_pair),
    );
    features.insert(
        "current_edge_line".to_owned(),
        bool_as_f64(current_edge_line),
    );
    features.insert(
        "previous_edge_line".to_owned(),
        bool_as_f64(is_edge_line(previous_line)),
    );
    features.insert(
        "same_page".to_owned(),
        bool_as_f64(previous_line.page_index == line.page_index),
    );
    features.insert(
        "line_below_footnote_divider".to_owned(),
        bool_as_f64(line.below_footnote_divider),
    );
    features.insert(
        "previous_below_footnote_divider".to_owned(),
        bool_as_f64(previous_line.below_footnote_divider),
    );
    features.insert(
        "current_doc_footnote_state".to_owned(),
        bool_as_f64(line.doc_footnote_state),
    );
    features.insert(
        "current_doc_footnote_continuation".to_owned(),
        bool_as_f64(line.doc_footnote_continuation),
    );
    features.insert(
        "previous_doc_footnote_state".to_owned(),
        bool_as_f64(previous_line.doc_footnote_state),
    );
    features.insert(
        "previous_doc_footnote_continuation".to_owned(),
        bool_as_f64(previous_line.doc_footnote_continuation),
    );
    features.insert(
        "doc_footnote_state_pair".to_owned(),
        bool_as_f64(previous_line.doc_footnote_state && line.doc_footnote_state),
    );
    features.insert(
        "doc_footnote_continuation_pair".to_owned(),
        bool_as_f64(previous_line.doc_footnote_continuation && line.doc_footnote_continuation),
    );
    features.insert(
        "current_doc_note_marker_mid_sequence_page".to_owned(),
        bool_as_f64(line.doc_note_marker_mid_sequence_page),
    );
    features.insert(
        "current_doc_note_marker_follows_previous_page".to_owned(),
        bool_as_f64(line.doc_note_marker_follows_previous_page),
    );
    features.insert(
        "current_doc_note_marker_first_on_page".to_owned(),
        bool_as_f64(line.doc_note_marker_first_on_page),
    );
    features.insert(
        "current_doc_note_marker_present".to_owned(),
        bool_as_f64(line.doc_note_marker > 0),
    );
    features.insert(
        "previous_doc_note_marker_present".to_owned(),
        bool_as_f64(previous_line.doc_note_marker > 0),
    );
    features.insert(
        "doc_note_marker_page_delta".to_owned(),
        line.doc_note_marker_page_delta as f64,
    );
    if gap > 0.025 {
        features.insert("vertical_gap_gt_0_025".to_owned(), 1.0);
    }
    if gap > 0.035 {
        features.insert("vertical_gap_gt_0_035".to_owned(), 1.0);
    }
    if let Some(role) = previous_line.role_hint {
        features.insert(
            format!("previous_role_hint_action:{}", role_action(role).as_str()),
            1.0,
        );
    }
    if let Some(role) = line.role_hint {
        features.insert(
            format!("current_role_hint_action:{}", role_action(role).as_str()),
            1.0,
        );
    }
    features
}

fn bool_as_f64(value: bool) -> f64 {
    if value { 1.0 } else { 0.0 }
}

fn role_action(role: LiquidBlockRole) -> Lm2Action {
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

fn lm2_numeric_catboost_features(line: &DeepLiquidSourceLine) -> HashMap<String, f64> {
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

fn lm2_native_catboost_cat_features(line: &DeepLiquidSourceLine) -> Vec<String> {
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

fn lm2_catboost_bin_name(value: f64, cuts: &[f64]) -> String {
    let index = cuts
        .iter()
        .position(|cut| value <= *cut)
        .unwrap_or(cuts.len());
    format!("b{index}")
}

fn lm2_catboost_first_token_shape(text: &str) -> String {
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

fn lm2_catboost_leading_marker_type(text: &str) -> String {
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

fn lm2_catboost_starts_roman_marker(value: &str) -> bool {
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

fn lm2_catboost_starts_letter_marker(value: &str) -> bool {
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

fn lm2_catboost_terminal_punct(text: &str) -> String {
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

fn nonempty_or_none(value: &str) -> String {
    if value.is_empty() {
        "none".to_owned()
    } else {
        value.to_owned()
    }
}

fn lm2_year_header_furniture_like(text: &str) -> bool {
    let value = collapse_whitespace(text);
    let bytes = value.as_bytes();
    bytes.len() >= 7
        && matches!(bytes.get(0..2), Some(b"19" | b"20"))
        && bytes.get(2).is_some_and(u8::is_ascii_digit)
        && bytes.get(3).is_some_and(u8::is_ascii_digit)
        && bytes.get(4) == Some(&b']')
        && value.chars().last().is_some_and(|ch| ch.is_ascii_digit())
}

fn lm2_short_numeric_body_fragment_like(
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

fn lm2_short_alpha_body_fragment_like(
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

fn round6(value: f64) -> f64 {
    (value * 1_000_000.0).round() / 1_000_000.0
}

fn round8(value: f64) -> f64 {
    (value * 100_000_000.0).round() / 100_000_000.0
}

fn lm2_numeric_has_dotleader(text: &str) -> bool {
    lm2_has_plain_dotleader(text)
}

fn lm2_numeric_has_long_dash_run(text: &str) -> bool {
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

fn lm2_internal_space_run_max(text: &str) -> usize {
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

fn lm2_numeric_token_count(text: &str) -> usize {
    text.split_whitespace()
        .filter(|token| {
            lm2_axis_numeric_token(
                token
                    .trim_matches(|ch: char| matches!(ch, ',' | ';' | ':' | '[' | ']' | '{' | '}')),
            )
        })
        .count()
}

fn lm2_percent_token_count(text: &str) -> usize {
    text.split_whitespace()
        .filter(|token| {
            let trimmed = token
                .trim_matches(|ch: char| matches!(ch, ',' | ';' | ':' | '[' | ']' | '{' | '}'));
            trimmed.ends_with('%') && lm2_axis_numeric_token(trimmed)
        })
        .count()
}

fn lm2_columnar_numeric_text_like(text: &str) -> bool {
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

fn lm2_has_plain_dotleader(text: &str) -> bool {
    text.contains("...")
}

fn lm2_has_spaced_dotleader(text: &str) -> bool {
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

fn lm2_has_strong_dotleader(text: &str) -> bool {
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

fn lm2_numeric_table_cell_like(text: &str, width_norm: f64) -> bool {
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

fn lm2_table_column_cell_like(text: &str) -> bool {
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
enum AxisTextKind {
    Numeric,
    ShortText,
}

fn lm2_short_axis_text_kind(text: &str) -> Option<AxisTextKind> {
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

fn lm2_axis_numeric_range(text: &str) -> bool {
    let Some((left, right)) = text.split_once(['-', '–']) else {
        return false;
    };
    lm2_axis_numeric_token(left) && lm2_axis_numeric_token(right)
}

fn lm2_axis_numeric_token(text: &str) -> bool {
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

fn lm2_numeric_page_number_like(text: &str) -> bool {
    let trimmed = text.trim();
    (1..=4).contains(&trimmed.len()) && trimmed.chars().all(|ch| ch.is_ascii_digit())
}

fn lm2_numeric_starts_roman_marker(text: &str) -> bool {
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

fn contains_citation_reporter(lower: &str) -> bool {
    ["u.s.", "s.ct.", "f.2d", "f.3d", "n.e.2d", "n.w.2d"]
        .iter()
        .any(|needle| lower.contains(needle))
}

fn lm2_features(
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

fn add_doc_font_features(
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

fn add_repetition_features(
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

fn add_marker_continuity_features(
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

fn add_feature(features: &mut Vec<(usize, f64)>, feature_dim: usize, name: &str, value: f64) {
    if feature_dim == 0 {
        return;
    }
    features.push(((fnv1a64(name) as usize) % feature_dim, value));
}

fn normalize_features(mut features: Vec<(usize, f64)>) -> Vec<(usize, f64)> {
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

fn build_lm2_blocks(
    fallback_title: &str,
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
) -> (String, Vec<LiquidBlock>, Vec<LiquidBlockSourceLines>) {
    build_lm2_blocks_with_grouping(fallback_title, decoded, None)
}

fn build_lm2_blocks_with_grouping(
    fallback_title: &str,
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
    grouping: Option<&Lm2PymupdfGroupingResponse>,
) -> (String, Vec<LiquidBlock>, Vec<LiquidBlockSourceLines>) {
    let mut blocks = Vec::new();
    let mut sources = Vec::new();
    let mut title = fallback_title.trim().to_owned();
    let recovered_title = lm2_recover_leading_source_title(decoded);
    let recovered_line_ids = recovered_title
        .as_ref()
        .map(|recovered| {
            recovered
                .lines
                .iter()
                .map(|line| line.id.clone())
                .collect::<HashSet<_>>()
        })
        .unwrap_or_default();
    let mut current_text = String::new();
    let mut current_refs = Vec::new();
    let mut current_role = LiquidBlockRole::Paragraph;
    let mut current_last_line: Option<DeepLiquidSourceLine> = None;
    let line_group_index = grouping_line_group_index(grouping);
    let mut current_group_index: Option<usize> = None;

    if let Some(recovered) = &recovered_title {
        for (index, line) in recovered.lines.iter().enumerate() {
            let role = if index == 0 {
                LiquidBlockRole::Title
            } else {
                LiquidBlockRole::Heading
            };
            let text = clean_lm2_line_text(&line.text);
            if text.is_empty() {
                continue;
            }
            let block_index = blocks.len();
            blocks.push(LiquidBlock {
                role,
                text,
                label: None,
            });
            sources.push(LiquidBlockSourceLines {
                block_index,
                lines: vec![line_ref(line, role)],
            });
        }
    }

    for (line, action) in decoded {
        if recovered_line_ids.contains(&line.id) {
            flush_block(
                &mut blocks,
                &mut sources,
                &mut current_text,
                &mut current_refs,
                current_role,
            );
            current_last_line = None;
            current_group_index = None;
            continue;
        }
        if *action == Lm2Action::HideNoise {
            if line.role_hint == Some(LiquidBlockRole::Table) {
                flush_block(
                    &mut blocks,
                    &mut sources,
                    &mut current_text,
                    &mut current_refs,
                    current_role,
                );
                let text = clean_lm2_line_text(&line.text);
                if !text.is_empty() {
                    let block_index = blocks.len();
                    blocks.push(LiquidBlock {
                        role: LiquidBlockRole::Table,
                        text,
                        label: Some("Table/Figure".to_owned()),
                    });
                    sources.push(LiquidBlockSourceLines {
                        block_index,
                        lines: vec![line_ref(line, LiquidBlockRole::Table)],
                    });
                }
                current_last_line = None;
                current_group_index = None;
            }
            continue;
        }
        let role = role_for_decoded_line(line, *action, blocks.is_empty());
        if lm2_should_attach_standalone_marker_to_current_block(
            current_role,
            role,
            current_last_line.as_ref(),
            line,
        ) {
            append_standalone_marker_to_line(&mut current_text, &line.text);
            current_refs.push(line_ref(line, current_role));
            continue;
        }
        let force_standalone = matches!(
            role,
            LiquidBlockRole::Title | LiquidBlockRole::Heading | LiquidBlockRole::Subheading
        );
        let starts_new_note = current_role == LiquidBlockRole::Marginalia
            && role == LiquidBlockRole::Marginalia
            && !current_text.is_empty()
            && looks_like_marginalia_note_block_start(&line.text);
        let starts_new_paragraph = current_role == LiquidBlockRole::Paragraph
            && role == LiquidBlockRole::Paragraph
            && current_last_line.as_ref().is_some_and(|previous| {
                if let Some(group_index) = line_group_index.get(&line.id).copied() {
                    if previous.page_index == line.page_index {
                        current_group_index != Some(group_index)
                    } else {
                        paragraph_boundary(previous, line)
                    }
                } else {
                    paragraph_boundary(previous, line)
                }
            });
        if current_text.is_empty() {
            current_role = role;
            current_group_index = line_group_index.get(&line.id).copied();
        } else if role != current_role
            || force_standalone
            || starts_new_note
            || starts_new_paragraph
        {
            flush_block(
                &mut blocks,
                &mut sources,
                &mut current_text,
                &mut current_refs,
                current_role,
            );
            current_role = role;
            current_group_index = line_group_index.get(&line.id).copied();
        }
        append_line(&mut current_text, &line.text);
        current_refs.push(line_ref(line, role));
        current_last_line = Some(line.clone());
        if force_standalone {
            flush_block(
                &mut blocks,
                &mut sources,
                &mut current_text,
                &mut current_refs,
                current_role,
            );
            current_last_line = None;
            current_group_index = None;
        }
    }
    flush_block(
        &mut blocks,
        &mut sources,
        &mut current_text,
        &mut current_refs,
        current_role,
    );

    if let Some(fallback) = lm2_fallback_title_from_blocks(&blocks)
        && (title.is_empty() || looks_like_filename_fallback_title(&title))
    {
        title = fallback;
    }
    if let Some(recovered) = recovered_title
        && (title.is_empty()
            || looks_like_filename_fallback_title(&title)
            || lm2_recovered_title_is_better(&recovered.title, &title))
    {
        title = recovered.title;
    }
    if title.is_empty() {
        title = "Untitled document".to_owned();
    }
    (title, blocks, sources)
}

fn apply_action_neutral_blocksplit(
    blocks: &mut Vec<LiquidBlock>,
    sources: &mut Vec<LiquidBlockSourceLines>,
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
) {
    if sources.is_empty() {
        return;
    }
    let line_by_id = decoded
        .iter()
        .map(|(line, _)| (line.id.as_str(), line))
        .collect::<HashMap<_, _>>();
    let mut refs = sources
        .iter()
        .flat_map(|source| source.lines.iter().cloned())
        .collect::<Vec<_>>();
    refs.sort_by_key(|line| (line.page_index, line.line_index));

    let mut rebuilt_blocks = Vec::new();
    let mut rebuilt_sources = Vec::new();
    let mut current_role: Option<LiquidBlockRole> = None;
    let mut current_refs: Vec<LiquidSourceLineRef> = Vec::new();
    let mut previous_ref: Option<LiquidSourceLineRef> = None;
    let mut previous_role: Option<LiquidBlockRole> = None;

    for line_ref in refs {
        let role = line_ref.role;
        if lm2_blocksplit_should_split(
            previous_ref.as_ref(),
            &line_ref,
            previous_role,
            role,
            &line_by_id,
        ) {
            flush_action_neutral_blocksplit(
                &mut rebuilt_blocks,
                &mut rebuilt_sources,
                &mut current_refs,
                current_role.unwrap_or(role),
            );
        }
        current_role = Some(role);
        previous_ref = Some(line_ref.clone());
        previous_role = Some(role);
        current_refs.push(line_ref);
    }
    if let Some(role) = current_role {
        flush_action_neutral_blocksplit(
            &mut rebuilt_blocks,
            &mut rebuilt_sources,
            &mut current_refs,
            role,
        );
    }
    if !rebuilt_blocks.is_empty() {
        *blocks = rebuilt_blocks;
        *sources = rebuilt_sources;
    }
}

fn flush_action_neutral_blocksplit(
    blocks: &mut Vec<LiquidBlock>,
    sources: &mut Vec<LiquidBlockSourceLines>,
    refs: &mut Vec<LiquidSourceLineRef>,
    role: LiquidBlockRole,
) {
    if refs.is_empty() {
        return;
    }
    let block_index = blocks.len();
    let mut text = String::new();
    for line in refs.iter() {
        let cleaned = clean_lm2_line_text(&line.text);
        if !cleaned.is_empty() {
            append_line(&mut text, &cleaned);
        }
    }
    if text.trim().is_empty() {
        refs.clear();
        return;
    }
    blocks.push(LiquidBlock {
        role,
        text,
        label: None,
    });
    sources.push(LiquidBlockSourceLines {
        block_index,
        lines: std::mem::take(refs),
    });
}

fn apply_deferred_marginalia_reflow(
    blocks: &mut Vec<LiquidBlock>,
    sources: &mut Vec<LiquidBlockSourceLines>,
) -> usize {
    let mut total = 0usize;
    for _ in 0..8 {
        let reflowed = apply_deferred_marginalia_reflow_once(blocks, sources);
        if reflowed == 0 {
            break;
        }
        total += reflowed;
    }
    total
}

fn apply_deferred_marginalia_reflow_once(
    blocks: &mut Vec<LiquidBlock>,
    sources: &mut Vec<LiquidBlockSourceLines>,
) -> usize {
    if blocks.len() < 3 {
        return 0;
    }
    let mut source_lines_by_block = sources
        .iter()
        .map(|source| (source.block_index, source.lines.clone()))
        .collect::<BTreeMap<_, _>>();
    let old_blocks = std::mem::take(blocks);
    let mut rebuilt_blocks = Vec::with_capacity(old_blocks.len());
    let mut rebuilt_sources = Vec::with_capacity(sources.len());
    let mut reflowed = 0usize;
    let mut index = 0usize;

    while index < old_blocks.len() {
        if old_blocks[index].role == LiquidBlockRole::Paragraph {
            let mut note_end = index + 1;
            while note_end < old_blocks.len()
                && lm2_reflow_deferred_note_role(old_blocks[note_end].role)
            {
                note_end += 1;
            }
            if note_end > index + 1
                && note_end < old_blocks.len()
                && old_blocks[note_end].role == LiquidBlockRole::Paragraph
                && lm2_should_reflow_deferred_marginalia(
                    &old_blocks[index].text,
                    &old_blocks[note_end].text,
                )
            {
                let mut merged_paragraph = old_blocks[index].clone();
                append_line(&mut merged_paragraph.text, &old_blocks[note_end].text);
                let block_index = rebuilt_blocks.len();
                rebuilt_blocks.push(merged_paragraph);

                let mut merged_refs = source_lines_by_block.remove(&index).unwrap_or_default();
                merged_refs.extend(source_lines_by_block.remove(&note_end).unwrap_or_default());
                if !merged_refs.is_empty() {
                    rebuilt_sources.push(LiquidBlockSourceLines {
                        block_index,
                        lines: merged_refs,
                    });
                }

                for note_index in (index + 1)..note_end {
                    let block_index = rebuilt_blocks.len();
                    rebuilt_blocks.push(old_blocks[note_index].clone());
                    if let Some(lines) = source_lines_by_block.remove(&note_index)
                        && !lines.is_empty()
                    {
                        rebuilt_sources.push(LiquidBlockSourceLines { block_index, lines });
                    }
                }
                reflowed += 1;
                index = note_end + 1;
                continue;
            }
        }

        let block_index = rebuilt_blocks.len();
        rebuilt_blocks.push(old_blocks[index].clone());
        if let Some(lines) = source_lines_by_block.remove(&index)
            && !lines.is_empty()
        {
            rebuilt_sources.push(LiquidBlockSourceLines { block_index, lines });
        }
        index += 1;
    }

    if reflowed > 0 {
        *blocks = rebuilt_blocks;
        *sources = rebuilt_sources;
    } else {
        *blocks = old_blocks;
    }
    reflowed
}

fn lm2_reflow_deferred_note_role(role: LiquidBlockRole) -> bool {
    matches!(
        role,
        LiquidBlockRole::Marginalia | LiquidBlockRole::Footnote
    )
}

fn lm2_should_reflow_deferred_marginalia(before: &str, after: &str) -> bool {
    let before = before.trim();
    let after = after.trim();
    if before.is_empty() || after.is_empty() {
        return false;
    }
    if !lm2_reflow_paragraph_is_visibly_open(before) {
        return false;
    }
    lm2_reflow_starts_like_continuation(after)
}

fn lm2_reflow_paragraph_is_visibly_open(text: &str) -> bool {
    let trimmed = text.trim_end();
    trimmed.ends_with('-') || !lm2_blocksplit_ends_like_paragraph(trimmed)
}

fn lm2_reflow_starts_like_continuation(text: &str) -> bool {
    let trimmed = text.trim_start();
    let first = trimmed
        .chars()
        .find(|ch| !matches!(ch, '"' | '\'' | '“' | '‘' | '(' | '['));
    first.is_some_and(|ch| {
        ch.is_ascii_lowercase()
            || matches!(
                ch,
                ',' | ';' | ':' | ')' | ']' | '”' | '’' | '-' | '–' | '—'
            )
    })
}

fn lm2_blocksplit_should_split(
    previous_ref: Option<&LiquidSourceLineRef>,
    line_ref: &LiquidSourceLineRef,
    previous_role: Option<LiquidBlockRole>,
    role: LiquidBlockRole,
    line_by_id: &HashMap<&str, &DeepLiquidSourceLine>,
) -> bool {
    let Some(previous_ref) = previous_ref else {
        return false;
    };
    let Some(previous_role) = previous_role else {
        return false;
    };
    if role != previous_role {
        return true;
    }
    if !matches!(
        role,
        LiquidBlockRole::Paragraph | LiquidBlockRole::Marginalia
    ) {
        return false;
    }
    if lm2_blocksplit_divider_like(&line_ref.text)
        || lm2_blocksplit_divider_like(&previous_ref.text)
    {
        return true;
    }

    let left = line_ref
        .id
        .as_deref()
        .and_then(|id| line_by_id.get(id))
        .map(|line| line.left)
        .unwrap_or_default();
    let previous_left = previous_ref
        .id
        .as_deref()
        .and_then(|id| line_by_id.get(id))
        .map(|line| line.left)
        .unwrap_or_default();
    let indent_increase = left - previous_left;

    if role == LiquidBlockRole::Paragraph
        && indent_increase >= 8.0
        && lm2_blocksplit_ends_like_paragraph(&previous_ref.text)
    {
        return true;
    }
    role == LiquidBlockRole::Marginalia
        && indent_increase >= 8.0
        && lm2_blocksplit_numbered_marginalia_start(&line_ref.text)
}

fn lm2_blocksplit_ends_like_paragraph(text: &str) -> bool {
    let trimmed = text.trim_end_matches('\u{0002}').trim_end();
    let mut chars = trimmed.chars().rev();
    let Some(last) = chars.next() else {
        return false;
    };
    if matches!(last, '"' | '\'' | ')' | ']') {
        chars
            .next()
            .is_some_and(|ch| matches!(ch, '.' | '!' | '?') || ch.is_ascii_digit())
    } else {
        matches!(last, '.' | '!' | '?') || last.is_ascii_digit()
    }
}

fn lm2_blocksplit_divider_like(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed.chars().count() >= 12
        && trimmed
            .chars()
            .all(|ch| matches!(ch, '-' | '–' | '—' | '_') || ch.is_whitespace())
}

fn lm2_blocksplit_numbered_marginalia_start(text: &str) -> bool {
    let trimmed = text.trim_start();
    let mut digits = 0usize;
    let mut chars = trimmed.chars().peekable();
    while chars.peek().is_some_and(|ch| ch.is_ascii_digit()) && digits < 4 {
        chars.next();
        digits += 1;
    }
    if digits == 0 || chars.peek().is_some_and(|ch| ch.is_ascii_digit()) {
        return false;
    }
    if !matches!(chars.next(), Some('.') | Some(')')) {
        return false;
    }
    chars.next().is_some_and(|ch| ch.is_whitespace())
}

fn grouping_line_group_index(
    grouping: Option<&Lm2PymupdfGroupingResponse>,
) -> HashMap<String, usize> {
    let mut out = HashMap::new();
    let Some(grouping) = grouping else {
        return out;
    };
    for (block_index, block) in grouping.blocks.iter().enumerate() {
        let group_index = block.block_index.unwrap_or(block_index);
        for line_id in &block.source_line_ids {
            out.insert(line_id.clone(), group_index);
        }
    }
    out
}

#[derive(Debug)]
struct Lm2RecoveredTitle {
    title: String,
    lines: Vec<DeepLiquidSourceLine>,
}

fn flush_block(
    blocks: &mut Vec<LiquidBlock>,
    sources: &mut Vec<LiquidBlockSourceLines>,
    text: &mut String,
    refs: &mut Vec<LiquidSourceLineRef>,
    role: LiquidBlockRole,
) {
    let cleaned = collapse_whitespace(text).trim().to_owned();
    if cleaned.is_empty() {
        text.clear();
        refs.clear();
        return;
    }
    let block_index = blocks.len();
    blocks.push(LiquidBlock {
        role,
        text: cleaned,
        label: (role == LiquidBlockRole::Table).then(|| "Table/Figure".to_owned()),
    });
    if !refs.is_empty() {
        sources.push(LiquidBlockSourceLines {
            block_index,
            lines: std::mem::take(refs),
        });
    }
    text.clear();
}

fn append_line(text: &mut String, line: &str) {
    let line = clean_lm2_line_text(line);
    if line.is_empty() {
        return;
    }
    if !text.is_empty() {
        if should_join_dehyphenated(text, &line) {
            while text.ends_with(char::is_whitespace) || text.ends_with('-') {
                text.pop();
            }
        } else if should_join_preserved_hyphen(text, &line) {
            while text.ends_with(char::is_whitespace) {
                text.pop();
            }
        } else {
            text.push(' ');
        }
    }
    text.push_str(&line);
}

fn append_standalone_marker_to_line(text: &mut String, marker: &str) {
    let marker = clean_lm2_line_text(marker);
    if marker.is_empty() {
        return;
    }
    if !text.is_empty() {
        while text.ends_with(char::is_whitespace) {
            text.pop();
        }
        text.push(' ');
    }
    text.push_str(&marker);
}

fn lm2_should_attach_standalone_marker_to_current_block(
    current_role: LiquidBlockRole,
    role: LiquidBlockRole,
    previous: Option<&DeepLiquidSourceLine>,
    marker: &DeepLiquidSourceLine,
) -> bool {
    current_role == LiquidBlockRole::Paragraph
        && role == LiquidBlockRole::Paragraph
        && lm2_looks_like_standalone_marker_fragment(&marker.text)
        && previous.is_some_and(|previous| lm2_marker_can_attach_to_previous_line(marker, previous))
}

fn lm2_looks_like_standalone_marker_fragment(text: &str) -> bool {
    let text = text.trim();
    (1..=4).contains(&text.len()) && text.chars().all(|ch| ch.is_ascii_digit())
}

fn lm2_marker_can_attach_to_previous_line(
    marker: &DeepLiquidSourceLine,
    previous: &DeepLiquidSourceLine,
) -> bool {
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
    let page_width = previous.page_width.max(marker.page_width).max(1.0);
    let horizontal_close =
        marker.left >= previous.left && marker.left <= previous.right + page_width * 0.04;
    let previous_can_host_marker = previous.text.trim_end().chars().last().is_some_and(|ch| {
        ch.is_ascii_alphabetic() || matches!(ch, '.' | '?' | '!' | '"' | '\'' | ')' | ']')
    });
    plausible_superscript_offset && horizontal_close && previous_can_host_marker
}

fn clean_lm2_line_text(line: &str) -> String {
    collapse_whitespace(&line.replace('\u{0002}', "-"))
}

fn should_join_dehyphenated(existing: &str, next: &str) -> bool {
    existing.trim_end().ends_with('-')
        && next
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_lowercase())
        && !should_preserve_terminal_hyphen(existing, next)
}

fn should_join_preserved_hyphen(existing: &str, next: &str) -> bool {
    existing.trim_end().ends_with('-') && should_preserve_terminal_hyphen(existing, next)
}

fn paragraph_boundary(previous: &DeepLiquidSourceLine, line: &DeepLiquidSourceLine) -> bool {
    if previous.page_index != line.page_index {
        let previous_sentence_end = lm2_blocksplit_ends_like_paragraph(&previous.text);
        let indent = (line.left - previous.left) / line.page_width.max(1.0);
        return previous_sentence_end && indent > 0.025;
    }
    if same_visual_row_fragment(previous, line) {
        return false;
    }
    let gap = vertical_gap(previous, line);
    let left_delta = ((line.left - previous.left).abs() / line.page_width.max(1.0)).max(0.0);
    let previous_sentence_end = previous
        .text
        .trim_end()
        .chars()
        .last()
        .is_some_and(|ch| matches!(ch, '.' | '?' | '!' | '"' | '\'' | ')'));
    gap > 0.030 || (previous_sentence_end && (gap > 0.016 || left_delta > 0.055))
}

fn same_visual_row_fragment(previous: &DeepLiquidSourceLine, line: &DeepLiquidSourceLine) -> bool {
    if previous.page_index != line.page_index || line.line_index != previous.line_index + 1 {
        return false;
    }
    let page_height = line.page_height.max(previous.page_height).max(1.0);
    let baseline_delta = (line.bottom - previous.bottom).abs() / page_height;
    let top_delta = (line.top - previous.top).abs() / page_height;
    if baseline_delta > 0.003 || top_delta > 0.004 {
        return false;
    }
    let page_width = line.page_width.max(previous.page_width).max(1.0);
    let horizontal_gap = line.left - previous.right;
    horizontal_gap >= -2.0 && horizontal_gap <= page_width * 0.04
}

fn role_for_decoded_line(
    line: &DeepLiquidSourceLine,
    action: Lm2Action,
    first_visible_block: bool,
) -> LiquidBlockRole {
    match action {
        Lm2Action::Marginalia => LiquidBlockRole::Marginalia,
        Lm2Action::HideNoise => LiquidBlockRole::Noise,
        Lm2Action::Keep => {
            if let Some(role) = line.role_hint {
                return match role {
                    LiquidBlockRole::Title if first_visible_block && line.centered => role,
                    LiquidBlockRole::Heading
                        if word_count(&line.text) <= 14
                            && heading_text_like(&line.text)
                            && ((line.centered && uppercase_ratio(&line.text) >= 0.72)
                                || (line.font_ratio_page >= 0.98
                                    && (line.bold
                                        || line.centered
                                        || line.font_ratio_page > 1.10)))
                            && !line.text.trim_end().ends_with('.') =>
                    {
                        role
                    }
                    LiquidBlockRole::Subheading
                        if word_count(&line.text) <= 16
                            && line.font_ratio_page >= 0.96
                            && heading_text_like(&line.text)
                            && (line.bold || line.centered || line.font_ratio_page > 1.04)
                            && !line.text.trim_end().ends_with('.') =>
                    {
                        role
                    }
                    LiquidBlockRole::Abstract
                    | LiquidBlockRole::Syllabus
                    | LiquidBlockRole::AuthorInfo
                    | LiquidBlockRole::Lead
                    | LiquidBlockRole::Quote
                    | LiquidBlockRole::ListItem
                    | LiquidBlockRole::Clause
                    | LiquidBlockRole::Definition
                    | LiquidBlockRole::Holding
                    | LiquidBlockRole::Issue
                    | LiquidBlockRole::KeyClause => role,
                    _ => LiquidBlockRole::Paragraph,
                };
            }
            let words = word_count(&line.text);
            if first_visible_block && line.centered && words <= 18 {
                LiquidBlockRole::Title
            } else if words <= 14
                && (line.bold || line.centered || line.font_ratio_page > 1.12)
                && heading_text_like(&line.text)
                && !line.text.trim_end().ends_with('.')
            {
                LiquidBlockRole::Heading
            } else {
                LiquidBlockRole::Paragraph
            }
        }
    }
}

fn line_ref(line: &DeepLiquidSourceLine, role: LiquidBlockRole) -> LiquidSourceLineRef {
    LiquidSourceLineRef {
        id: Some(line.id.clone()),
        page_index: line.page_index,
        line_index: line.line_index,
        text: line.text.clone(),
        role,
        note_markers: source_line_note_markers(line, role),
    }
}

fn source_line_note_markers(line: &DeepLiquidSourceLine, role: LiquidBlockRole) -> Vec<u16> {
    if !matches!(
        role,
        LiquidBlockRole::Footnote | LiquidBlockRole::Marginalia
    ) {
        return Vec::new();
    }
    let mut markers = Vec::new();
    if line.doc_note_marker > 0 {
        markers.push(line.doc_note_marker);
    }
    if let Some(marker) = leading_note_marker(&line.text) {
        markers.push(marker);
    }
    markers.extend(sentineled_note_markers(&line.text));
    markers.sort_unstable();
    markers.dedup();
    markers
}

fn sentineled_note_markers(text: &str) -> Vec<u16> {
    let mut markers = Vec::new();
    let mut inside = false;
    let mut digits = String::new();
    for ch in text.chars() {
        if ch == CALLOUT_START {
            inside = true;
            digits.clear();
        } else if ch == CALLOUT_END {
            if inside
                && let Ok(marker) = digits.parse::<u16>()
                && (1..=500).contains(&marker)
            {
                markers.push(marker);
            }
            inside = false;
            digits.clear();
        } else if inside && ch.is_ascii_digit() && digits.len() < 3 {
            digits.push(ch);
        } else if inside && !ch.is_whitespace() {
            inside = false;
            digits.clear();
        }
    }
    markers
}

fn lm2_profile() -> crate::liquid::DocumentProfile {
    crate::liquid::DocumentProfile {
        kind: DocumentProfileKind::LawReviewArticle,
        confidence: 0.72,
        scores: vec![DocumentProfileScore {
            kind: DocumentProfileKind::LawReviewArticle,
            score: 0.72,
        }],
        evidence: vec!["LiquidMode2 law-review action decoder".to_owned()],
    }
}

fn load_cached_lm2_document(source_signature: &str) -> Option<LiquidDocument> {
    let bytes = std::fs::read(lm2_cache_path(source_signature)?).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn save_cached_lm2_document(document: &LiquidDocument) -> Result<(), String> {
    let path = lm2_cache_path(&document.source_signature)
        .ok_or_else(|| "Could not find LiquidMode2 cache directory.".to_owned())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create LiquidMode2 cache: {error}"))?;
    }
    let bytes = serde_json::to_vec(document).map_err(|error| error.to_string())?;
    std::fs::write(path, bytes).map_err(|error| error.to_string())
}

fn lm2_cache_path(source_signature: &str) -> Option<PathBuf> {
    app_data_dir().map(|dir| {
        dir.join("liquid2-cache")
            .join(format!("{source_signature}.json"))
    })
}

fn lm2_source_signature(
    path: &Path,
    pages: &[String],
    model_label: &str,
    context_twopass_label: Option<&str>,
    pp_prior_source: Option<&str>,
    static_overlay_source: Option<&str>,
    use_pymupdf_blocks: bool,
    pp_footnote_region_membership: bool,
    marker_decoder_prior: bool,
    small_font_decoder_prior: bool,
    small_font_sequence_prior: bool,
    anchored_marginalia_flow_guard: bool,
    body_preservation_guard: bool,
    action_neutral_blocksplit: bool,
    toc_overlay: bool,
    front_matter_guard: bool,
    marginalia_preservation_guard: bool,
    d1_runtime_zerospend_overlay: bool,
    d1_runtime_zerospend_overlay_version: Option<&str>,
    d1_runtime_continuation_overlay: bool,
    d1_runtime_immediate_continuation_overlay: bool,
    d1_runtime_sandwiched_continuation_overlay: bool,
    d1_runtime_wide_sandwich_overlay: bool,
    d1_runtime_safe_numeric_note_overlay: bool,
    d1_runtime_post_wide_cue_overlay: bool,
    d1_runtime_postcue_citation_next1_overlay: bool,
    d1_runtime_near8_cue_overlay: bool,
    d1_runtime_wide_divider_guard_overlay: bool,
    d1_runtime_geometric_zone_overlay: bool,
    d1_runtime_footer_artifact_overlay: bool,
    footnote_monotone_overlay: bool,
    footnote_carryover_overlay: bool,
    table_figure_router_overlay: bool,
    page_object_overlay: bool,
    page_object_tuned_overlay: bool,
    start_score_scale: f64,
    transition_score_scale: f64,
) -> String {
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let metadata = std::fs::metadata(path).ok();
    let modified = metadata
        .as_ref()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    let len = metadata.map(|metadata| metadata.len()).unwrap_or_default();
    let text_hash = fnv1a64(&pages.join("\n\u{0c}\n"));
    format!(
        "{:016x}",
        fnv1a64(&format!(
            "{LM2_SCHEMA_VERSION}|assembly={LM2_ASSEMBLY_CACHE_VERSION}|model={model_label}|context_twopass={}|pp={}|static_overlay={}|pymupdf={use_pymupdf_blocks}|pp_footnote_region_membership={pp_footnote_region_membership}|marker_prior={marker_decoder_prior}|small_font_prior={small_font_decoder_prior}|small_font_sequence_prior={small_font_sequence_prior}|anchored_flow_guard={anchored_marginalia_flow_guard}|body_guard={body_preservation_guard}|blocksplit={action_neutral_blocksplit}|toc_overlay={toc_overlay}|front_matter_guard={front_matter_guard}|marginalia_preservation_guard={marginalia_preservation_guard}|d1_runtime_zerospend_overlay={d1_runtime_zerospend_overlay}|d1_runtime_zerospend_overlay_version={}|d1_runtime_continuation_overlay={d1_runtime_continuation_overlay}|d1_runtime_immediate_continuation_overlay={d1_runtime_immediate_continuation_overlay}|d1_runtime_sandwiched_continuation_overlay={d1_runtime_sandwiched_continuation_overlay}|d1_runtime_wide_sandwich_overlay={d1_runtime_wide_sandwich_overlay}|d1_runtime_safe_numeric_note_overlay={d1_runtime_safe_numeric_note_overlay}|d1_runtime_post_wide_cue_overlay={d1_runtime_post_wide_cue_overlay}|d1_runtime_postcue_citation_next1_overlay={d1_runtime_postcue_citation_next1_overlay}|d1_runtime_postcue_citation_next1_overlay_version={}|d1_runtime_near8_cue_overlay={d1_runtime_near8_cue_overlay}|d1_runtime_near8_cue_overlay_version={}|d1_runtime_wide_divider_guard_overlay={d1_runtime_wide_divider_guard_overlay}|d1_runtime_wide_divider_guard_overlay_version={}|d1_runtime_geometric_zone_overlay={d1_runtime_geometric_zone_overlay}|d1_runtime_geometric_zone_overlay_version={}|d1_runtime_footer_artifact_overlay={d1_runtime_footer_artifact_overlay}|d1_runtime_footer_artifact_overlay_version={}|footnote_monotone_overlay={footnote_monotone_overlay}|footnote_monotone_overlay_version={}|footnote_carryover_overlay={footnote_carryover_overlay}|footnote_carryover_overlay_version={}|table_figure_router_overlay={table_figure_router_overlay}|table_figure_router_overlay_version={LM2_TABLE_FIGURE_ROUTER_OVERLAY_VERSION}|page_object_overlay={page_object_overlay}|page_object_overlay_version={LM2_PAGE_OBJECT_OVERLAY_VERSION}|page_object_tuned_overlay={page_object_tuned_overlay}|page_object_tuned_overlay_version={}|start_scale={start_score_scale:.6}|transition_scale={transition_score_scale:.6}|{}|{modified}|{len}|{text_hash}",
            context_twopass_label.unwrap_or("none"),
            d1_runtime_zerospend_overlay_version.unwrap_or("none"),
            if d1_runtime_postcue_citation_next1_overlay {
                LM2_D1_RUNTIME_POSTCUE_CITATION_NEXT1_OVERLAY_VERSION
            } else {
                "none"
            },
            if d1_runtime_near8_cue_overlay {
                LM2_D1_RUNTIME_NEAR8_CUE_OVERLAY_VERSION
            } else {
                "none"
            },
            if d1_runtime_wide_divider_guard_overlay {
                LM2_D1_RUNTIME_WIDE_DIVIDER_GUARD_OVERLAY_VERSION
            } else {
                "none"
            },
            if d1_runtime_geometric_zone_overlay {
                LM2_D1_RUNTIME_GEOMETRIC_ZONE_OVERLAY_VERSION
            } else {
                "none"
            },
            if d1_runtime_footer_artifact_overlay {
                LM2_D1_RUNTIME_FOOTER_ARTIFACT_OVERLAY_VERSION
            } else {
                "none"
            },
            if footnote_monotone_overlay {
                LM2_FOOTNOTE_MONOTONE_OVERLAY_VERSION
            } else {
                "none"
            },
            if footnote_carryover_overlay {
                LM2_FOOTNOTE_CARRYOVER_OVERLAY_VERSION
            } else {
                "none"
            },
            if page_object_tuned_overlay {
                LM2_PAGE_OBJECT_TUNED_OVERLAY_VERSION
            } else {
                "none"
            },
            pp_prior_source.unwrap_or("none"),
            static_overlay_source.unwrap_or("none"),
            canonical.display()
        ))
    )
}

fn truthy_env(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
}

fn falsey_env_value(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "0" | "false" | "no" | "off"
    )
}

fn falsey_env(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .is_some_and(|value| falsey_env_value(&value))
}

fn lm2_table_figure_router_disabled_by_env() -> bool {
    falsey_env("LAWPDF_LM2_TABLE_FIGURE_ROUTER")
}

fn float_env_or_default(name: &str, default: f64) -> f64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value >= 0.0 && *value <= 10.0)
        .unwrap_or(default)
}

fn lm2_v20_runtime_preset_enabled() -> bool {
    truthy_env("LAWPDF_LM2_V20_STACK")
        || std::env::var("LAWPDF_LM2_RUNTIME_PRESET")
            .ok()
            .is_some_and(|value| value.eq_ignore_ascii_case("v20"))
}

fn lm2_v20_stack_runtime_enabled() -> bool {
    lm2_v20_runtime_preset_enabled() || lm2_v25_d1_runtime_preset_enabled()
}

fn lm2_d1_runtime_zerospend_overlay_enabled() -> bool {
    truthy_env("LAWPDF_LM2_D1_RUNTIME_ZEROSPEND_OVERLAY") || lm2_v25_d1_runtime_preset_enabled()
}

fn lm2_d1_runtime_continuation_overlay_enabled() -> bool {
    truthy_env("LAWPDF_LM2_D1_CONTINUATION_OVERLAY")
        || lm2_v25_d1_continuation_runtime_preset_enabled()
}

fn lm2_d1_runtime_immediate_continuation_overlay_enabled() -> bool {
    truthy_env("LAWPDF_LM2_D1_IMMEDIATE_CONTINUATION_OVERLAY")
        || lm2_v25_d1_immediate_continuation_runtime_preset_enabled()
}

fn lm2_d1_runtime_sandwiched_continuation_overlay_enabled() -> bool {
    truthy_env("LAWPDF_LM2_D1_SANDWICHED_CONTINUATION_OVERLAY")
        || lm2_v25_d1_sandwiched_continuation_runtime_preset_enabled()
        || lm2_v25_d1_sandwiched_note_start_runtime_preset_enabled()
        || lm2_v25_d1_wide_sandwich_runtime_preset_enabled()
}

fn lm2_d1_runtime_wide_sandwich_overlay_enabled() -> bool {
    truthy_env("LAWPDF_LM2_D1_WIDE_SANDWICH_OVERLAY")
        || lm2_v25_d1_wide_sandwich_runtime_preset_enabled()
}

fn lm2_d1_runtime_safe_numeric_note_overlay_enabled() -> bool {
    truthy_env("LAWPDF_LM2_D1_SAFE_NUMERIC_NOTE_OVERLAY")
        || lm2_v25_d1_sandwiched_note_start_runtime_preset_enabled()
        || lm2_v25_d1_wide_sandwich_runtime_preset_enabled()
}

fn lm2_d1_runtime_post_wide_cue_overlay_enabled() -> bool {
    truthy_env("LAWPDF_LM2_D1_POST_WIDE_CUE_OVERLAY")
        || lm2_v25_d1_post_wide_cue_runtime_preset_enabled()
}

fn lm2_d1_runtime_postcue_citation_next1_overlay_enabled() -> bool {
    truthy_env("LAWPDF_LM2_D1_POSTCUE_CITATION_NEXT1_OVERLAY")
        || lm2_v25_d1_postcue_citation_next1_runtime_preset_enabled()
}

fn lm2_d1_runtime_near8_cue_overlay_enabled() -> bool {
    truthy_env("LAWPDF_LM2_D1_NEAR8_CUE_OVERLAY") || lm2_v25_d1_near8_cue_runtime_preset_enabled()
}

fn lm2_d1_runtime_wide_divider_guard_overlay_enabled() -> bool {
    truthy_env("LAWPDF_LM2_D1_WIDE_DIVIDER_GUARD_OVERLAY")
        || lm2_v25_d1_wide_divider_guard_runtime_preset_enabled()
}

fn lm2_d1_runtime_geometric_zone_overlay_enabled() -> bool {
    truthy_env("LAWPDF_LM2_D1_GEOMETRIC_FOOTNOTE_ZONE_OVERLAY")
        || lm2_v25_d1_geometric_zone_runtime_preset_enabled()
}

fn lm2_d1_runtime_footer_artifact_overlay_enabled() -> bool {
    truthy_env("LAWPDF_LM2_D1_FOOTER_ARTIFACT_OVERLAY")
}

fn lm2_footnote_monotone_overlay_enabled() -> bool {
    truthy_env("LAWPDF_LM2_FOOTNOTE_MONOTONE_OVERLAY")
}

fn lm2_footnote_carryover_overlay_enabled() -> bool {
    truthy_env("LAWPDF_LM2_FOOTNOTE_CARRYOVER_OVERLAY")
}

fn lm2_open_footnote_carryover_overlay_enabled() -> bool {
    truthy_env("LAWPDF_LM2_OPEN_FOOTNOTE_CARRYOVER_OVERLAY")
}

fn lm2_table_figure_router_overlay_enabled() -> bool {
    !lm2_table_figure_router_disabled_by_env()
}

fn lm2_page_object_overlay_enabled() -> bool {
    truthy_env("LAWPDF_LM2_PAGE_OBJECT_OVERLAY")
}

fn lm2_page_object_tuned_overlay_enabled() -> bool {
    if falsey_env("LAWPDF_LM2_PAGE_OBJECT_TUNED_OVERLAY") {
        return false;
    }
    truthy_env("LAWPDF_LM2_PAGE_OBJECT_TUNED_OVERLAY")
        || lm2_v25_d1_page_object_tuned_runtime_preset_enabled()
}

fn lm2_start_score_scale() -> f64 {
    float_env_or_default(
        "LAWPDF_LM2_START_SCORE_SCALE",
        if lm2_v20_stack_runtime_enabled() {
            3.0
        } else {
            1.0
        },
    )
}

fn lm2_transition_score_scale() -> f64 {
    float_env_or_default(
        "LAWPDF_LM2_TRANSITION_SCORE_SCALE",
        if lm2_v20_stack_runtime_enabled() {
            3.0
        } else {
            1.0
        },
    )
}

fn lm2_marker_decoder_prior_enabled() -> bool {
    truthy_env("LAWPDF_LM2_MARKER_DECODER_PRIOR")
}

fn lm2_small_font_decoder_prior_enabled() -> bool {
    truthy_env("LAWPDF_LM2_SMALL_FONT_DECODER_PRIOR")
}

fn lm2_small_font_sequence_prior_enabled() -> bool {
    truthy_env("LAWPDF_LM2_SMALL_FONT_SEQUENCE_PRIOR")
}

fn lm2_anchored_marginalia_flow_guard_enabled() -> bool {
    truthy_env("LAWPDF_LM2_ANCHORED_MARGINALIA_FLOW_GUARD")
}

fn lm2_body_preservation_guard_enabled() -> bool {
    truthy_env("LAWPDF_LM2_BODY_PRESERVATION_GUARD") || lm2_v20_stack_runtime_enabled()
}

fn lm2_action_neutral_blocksplit_enabled() -> bool {
    truthy_env("LAWPDF_LM2_ACTION_NEUTRAL_BLOCKSPLIT") || lm2_v20_stack_runtime_enabled()
}

fn lm2_toc_overlay_enabled() -> bool {
    truthy_env("LAWPDF_LM2_TOC_OVERLAY") || lm2_v20_stack_runtime_enabled()
}

fn lm2_front_matter_guard_enabled() -> bool {
    truthy_env("LAWPDF_LM2_FRONT_MATTER_GUARD") || lm2_v20_stack_runtime_enabled()
}

fn lm2_marginalia_preservation_guard_enabled() -> bool {
    truthy_env("LAWPDF_LM2_MARGINALIA_PRESERVATION_GUARD") || lm2_v20_stack_runtime_enabled()
}

fn lm2_pp_footnote_region_membership_enabled() -> bool {
    truthy_env("LAWPDF_LM2_PP_FOOTNOTE_REGION_MEMBERSHIP")
}

fn run_lm2_pp_doclayout_sidecar(
    path: &Path,
    source_lines: &[DeepLiquidSourceLine],
    cache_key: &str,
    draft_path: &Path,
) -> Result<(), String> {
    let script_path = lm2_pp_doclayout_script_candidates()
        .into_iter()
        .find(|candidate| candidate.exists())
        .ok_or_else(|| "LM2 PP-DocLayout sidecar script is missing".to_owned())?;
    let work_dir = lm2_pp_doclayout_work_dir(cache_key)
        .ok_or_else(|| "could not find LM2 PP-DocLayout work directory".to_owned())?;
    let document_path = path.to_string_lossy().to_string();
    let work_dir_text = work_dir.to_string_lossy().to_string();
    let request = Lm2PpDoclayoutRequest {
        schema_version: "lm2-pp-doclayout-runtime-request-v1",
        document_path: &document_path,
        source_lines,
        render_scale: 2.0,
        layout_model_name: "PP-DocLayoutV3",
        work_dir: &work_dir_text,
    };
    let (request_path, response_path) = write_lm2_pp_doclayout_request(cache_key, &request)?;
    if let Some(parent) = draft_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("could not create LM2 PP-DocLayout cache: {error}"))?;
    }
    let python_exe = lm2_pp_doclayout_python_candidates()
        .into_iter()
        .find(|candidate| candidate.exists())
        .unwrap_or_else(|| PathBuf::from("python3"));
    let output = std::process::Command::new(python_exe)
        .arg(&script_path)
        .arg("--request")
        .arg(&request_path)
        .arg("--response")
        .arg(&response_path)
        .arg("--draft-output")
        .arg(draft_path)
        .output()
        .map_err(|error| format!("could not start LM2 PP-DocLayout sidecar: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "LM2 PP-DocLayout sidecar failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let bytes = std::fs::read(&response_path)
        .map_err(|error| format!("could not read LM2 PP-DocLayout response: {error}"))?;
    let response = serde_json::from_slice::<Lm2PpDoclayoutResponse>(&bytes)
        .map_err(|error| format!("could not decode LM2 PP-DocLayout response: {error}"))?;
    if !response.warnings.is_empty() {
        eprintln!(
            "LM2 PP-DocLayout sidecar warnings: {}",
            response.warnings.join("; ")
        );
    }
    eprintln!(
        "LM2 PP-DocLayout sidecar: pages={} boxes={} draft_rows={}",
        response.page_count, response.detection_box_count, response.draft_row_count
    );
    Ok(())
}

fn write_lm2_pp_doclayout_request(
    cache_key: &str,
    request: &Lm2PpDoclayoutRequest<'_>,
) -> Result<(PathBuf, PathBuf), String> {
    let root = app_data_dir()
        .ok_or_else(|| "could not find app data directory for LM2 PP-DocLayout".to_owned())?
        .join("liquid2-ppdoclayout-work")
        .join(cache_key);
    std::fs::create_dir_all(&root)
        .map_err(|error| format!("could not create LM2 PP-DocLayout work directory: {error}"))?;
    let request_path = root.join("request.json");
    let response_path = root.join("response.json");
    let bytes = serde_json::to_vec(request)
        .map_err(|error| format!("could not encode LM2 PP-DocLayout request: {error}"))?;
    std::fs::write(&request_path, bytes)
        .map_err(|error| format!("could not write LM2 PP-DocLayout request: {error}"))?;
    Ok((request_path, response_path))
}

fn lm2_pp_doclayout_cache_key(path: &Path, source_lines: &[DeepLiquidSourceLine]) -> String {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let metadata = std::fs::metadata(path).ok();
    let modified = metadata
        .as_ref()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    let len = metadata.map(|metadata| metadata.len()).unwrap_or_default();
    let mut line_fingerprint = String::new();
    for line in source_lines {
        line_fingerprint.push_str(&format!(
            "{}\u{1f}{}\u{1f}{}\u{1f}{:.2}\u{1f}{:.2}\u{1f}{:.2}\u{1f}{:.2}\u{1e}",
            line.page_index,
            line.line_index,
            line.text,
            line.left,
            line.bottom,
            line.right,
            line.top
        ));
    }
    format!(
        "{:016x}",
        fnv1a64(&format!(
            "{LM2_SCHEMA_VERSION}|ppdoclayout-runtime-v1|{}|{modified}|{len}|{:016x}",
            canonical.display(),
            fnv1a64(&line_fingerprint)
        ))
    )
}

fn lm2_pp_doclayout_draft_cache_path(cache_key: &str) -> Option<PathBuf> {
    app_data_dir().map(|dir| {
        dir.join("liquid2-ppdoclayout-cache")
            .join(format!("{cache_key}.jsonl"))
    })
}

fn lm2_pp_doclayout_work_dir(cache_key: &str) -> Option<PathBuf> {
    app_data_dir().map(|dir| dir.join("liquid2-ppdoclayout-work").join(cache_key))
}

fn lm2_pp_doclayout_script_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os("LAWPDF_LM2_PP_DOCLAYOUT_SCRIPT").map(PathBuf::from) {
        candidates.push(path);
    }
    if let Ok(current_dir) = std::env::current_dir() {
        candidates.push(current_dir.join("tools/lm2_pp_doclayout_regions.py"));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(exe_dir) = exe.parent()
    {
        candidates.push(exe_dir.join("tools/lm2_pp_doclayout_regions.py"));
        candidates.push(exe_dir.join("../Resources/tools/lm2_pp_doclayout_regions.py"));
    }
    candidates
}

fn lm2_pp_doclayout_python_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os("LAWPDF_LM2_PP_DOCLAYOUT_PYTHON").map(PathBuf::from) {
        candidates.push(path);
    }
    if let Ok(current_dir) = std::env::current_dir() {
        candidates.push(current_dir.join(".lawpdf/ppdoclayout-venv/bin/python"));
        candidates.push(current_dir.join("research/doclayout-yolo-venv/bin/python"));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(exe_dir) = exe.parent()
    {
        candidates.push(exe_dir.join("../Resources/ppdoclayout-venv/bin/python"));
        candidates.push(exe_dir.join("../Resources/.lawpdf/ppdoclayout-venv/bin/python"));
    }
    candidates.push(PathBuf::from("python3"));
    candidates
}

fn try_apply_lm2_pymupdf_grouping(
    path: &Path,
    title: &str,
    source_signature: &str,
    source_lines: &[DeepLiquidSourceLine],
) -> Result<Option<Lm2PymupdfGroupingResponse>, String> {
    if source_lines.is_empty() {
        return Ok(None);
    }
    if let Some(cached) = load_cached_lm2_pymupdf_grouping(source_signature) {
        return Ok(Some(cached));
    }
    let script_path = lm2_pymupdf_grouping_script_candidates()
        .into_iter()
        .find(|candidate| candidate.exists())
        .ok_or_else(|| "LM2 PyMuPDF grouping sidecar script is missing".to_owned())?;
    let document_path = path.to_string_lossy().to_string();
    let request = Lm2PymupdfGroupingRequest {
        schema_version: "lm2-pymupdf-grouping-request-v1",
        source_signature,
        document_path: &document_path,
        title,
        use_detector_fallback: true,
        source_lines,
    };
    let (request_path, response_path) = write_lm2_pymupdf_grouping_request(&request)?;
    let python_exe = lm2_pymupdf_grouping_python_candidates()
        .into_iter()
        .find(|candidate| candidate.exists())
        .unwrap_or_else(|| PathBuf::from("python3"));
    let output = std::process::Command::new(python_exe)
        .arg(&script_path)
        .arg("--request")
        .arg(&request_path)
        .arg("--response")
        .arg(&response_path)
        .output()
        .map_err(|error| format!("could not start LM2 PyMuPDF grouping sidecar: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "LM2 PyMuPDF grouping sidecar failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let bytes = std::fs::read(&response_path)
        .map_err(|error| format!("could not read LM2 PyMuPDF grouping response: {error}"))?;
    let response = serde_json::from_slice::<Lm2PymupdfGroupingResponse>(&bytes)
        .map_err(|error| format!("could not decode LM2 PyMuPDF grouping response: {error}"))?;
    save_cached_lm2_pymupdf_grouping(source_signature, &bytes)?;
    if response.blocks.is_empty() {
        return Ok(None);
    }
    Ok(Some(response))
}

fn write_lm2_pymupdf_grouping_request(
    request: &Lm2PymupdfGroupingRequest<'_>,
) -> Result<(PathBuf, PathBuf), String> {
    let root = app_data_dir()
        .ok_or_else(|| "could not find app data directory for LM2 PyMuPDF grouping".to_owned())?
        .join("liquid2-pymupdf-work");
    std::fs::create_dir_all(&root).map_err(|error| {
        format!("could not create LM2 PyMuPDF grouping work directory: {error}")
    })?;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let request_path = root.join(format!("request-{nanos}.json"));
    let response_path = root.join(format!("response-{nanos}.json"));
    let bytes = serde_json::to_vec(request)
        .map_err(|error| format!("could not encode LM2 PyMuPDF grouping request: {error}"))?;
    std::fs::write(&request_path, bytes)
        .map_err(|error| format!("could not write LM2 PyMuPDF grouping request: {error}"))?;
    Ok((request_path, response_path))
}

fn load_cached_lm2_pymupdf_grouping(source_signature: &str) -> Option<Lm2PymupdfGroupingResponse> {
    let bytes = std::fs::read(lm2_pymupdf_grouping_cache_path(source_signature)?).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn save_cached_lm2_pymupdf_grouping(source_signature: &str, bytes: &[u8]) -> Result<(), String> {
    let path = lm2_pymupdf_grouping_cache_path(source_signature)
        .ok_or_else(|| "could not find LM2 PyMuPDF grouping cache directory".to_owned())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("could not create LM2 PyMuPDF grouping cache: {error}"))?;
    }
    std::fs::write(path, bytes)
        .map_err(|error| format!("could not write LM2 PyMuPDF grouping cache: {error}"))
}

fn lm2_pymupdf_grouping_cache_path(source_signature: &str) -> Option<PathBuf> {
    app_data_dir().map(|dir| {
        dir.join("liquid2-pymupdf-cache")
            .join(format!("{source_signature}.json"))
    })
}

fn lm2_pymupdf_grouping_script_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(current_dir) = std::env::current_dir() {
        candidates.push(current_dir.join("tools/lm2_pymupdf_blocks.py"));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(exe_dir) = exe.parent()
    {
        candidates.push(exe_dir.join("tools/lm2_pymupdf_blocks.py"));
        candidates.push(exe_dir.join("../Resources/tools/lm2_pymupdf_blocks.py"));
    }
    candidates
}

fn lm2_pymupdf_grouping_python_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(current_dir) = std::env::current_dir() {
        candidates.push(current_dir.join("research/doclayout-yolo-venv/bin/python"));
        candidates.push(current_dir.join(".lawpdf/ppdoclayout-venv/bin/python"));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(exe_dir) = exe.parent()
    {
        candidates.push(exe_dir.join("../Resources/ppdoclayout-venv/bin/python"));
        candidates.push(exe_dir.join("../Resources/.lawpdf/ppdoclayout-venv/bin/python"));
    }
    candidates.push(PathBuf::from("python3"));
    candidates
}

fn pp_prior_key(path: &str, page_index: usize, line_index: usize, text: &str) -> String {
    eval_key(path, page_index, line_index, text)
}

fn bin_name(value: f32, cuts: &[f32]) -> usize {
    cuts.iter()
        .position(|cut| value <= *cut)
        .unwrap_or(cuts.len())
}

fn fnv1a64(value: &str) -> u64 {
    let mut result = 0xcbf29ce484222325u64;
    for byte in value.as_bytes() {
        result ^= u64::from(*byte);
        result = result.wrapping_mul(0x100000001b3);
    }
    result
}

fn normalize_text(text: &str) -> String {
    collapse_whitespace(text).to_ascii_lowercase()
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn words(text: &str) -> Vec<String> {
    text.split(|ch: char| !(ch.is_ascii_alphanumeric() || matches!(ch, '\'' | '.' | '-')))
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
        .collect()
}

fn word_count(text: &str) -> usize {
    words(&text.to_ascii_lowercase()).len()
}

fn uppercase_ratio(text: &str) -> f32 {
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

fn title_case_ratio(text: &str) -> f32 {
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

fn heading_text_like(text: &str) -> bool {
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

fn looks_like_note_start(text: &str) -> bool {
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

fn looks_like_marginalia_note_block_start(text: &str) -> bool {
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
    if digits == 0 || !(1..=500).contains(&value) {
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

fn has_legal_note_cue(lower: &str) -> bool {
    words(lower).iter().any(|token| {
        matches!(
            token.trim_matches(|ch: char| matches!(ch, ',' | ';' | ':' | ')' | '(')),
            "id." | "supra" | "infra" | "see" | "cf." | "u.s." | "s.ct." | "f.2d" | "f.3d"
        )
    })
}

fn looks_like_toc_entry(lower: &str) -> bool {
    lower.contains("...")
        && lower
            .chars()
            .rev()
            .find(|ch| !ch.is_whitespace())
            .is_some_and(|ch| ch.is_ascii_digit())
}

fn looks_like_running_header(lower: &str) -> bool {
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

fn looks_like_production_slug_boilerplate(text: &str) -> bool {
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

fn looks_like_filename_fallback_title(text: &str) -> bool {
    let trimmed = text.trim();
    let lower = trimmed.to_ascii_lowercase();
    lower.ends_with(".pdf")
        || (trimmed.contains('_')
            && !trimmed.contains(' ')
            && lower.chars().filter(|ch| ch.is_ascii_alphabetic()).count() >= 8)
}

fn lm2_fallback_title_from_blocks(blocks: &[LiquidBlock]) -> Option<String> {
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

fn lm2_leading_title_from_blocks(blocks: &[LiquidBlock]) -> Option<String> {
    let mut parts = Vec::new();
    for block in blocks.iter().take(16) {
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

fn lm2_recover_leading_source_title(
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
) -> Option<Lm2RecoveredTitle> {
    let mut parts = Vec::new();
    let mut lines = Vec::new();
    let mut has_hidden_part = false;

    for (line, action) in decoded.iter().take(64) {
        if line.page_index != 0 {
            break;
        }
        if line.line_index > 32 {
            break;
        }

        let text = clean_lm2_line_text(&line.text);
        if text.is_empty() || looks_like_production_slug_boilerplate(&text) {
            continue;
        }
        if lm2_front_matter_stop_text(&text) || lm2_generic_title_label(&text) {
            if !parts.is_empty() {
                break;
            }
            continue;
        }
        if looks_like_lm2_author_heading(&text) || lm2_source_title_author_like(line, &text) {
            if !parts.is_empty() {
                break;
            }
            continue;
        }

        if lm2_leading_source_title_candidate(line, &text) {
            if *action == Lm2Action::HideNoise {
                has_hidden_part = true;
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

    if !has_hidden_part || parts.is_empty() {
        return None;
    }
    let title = collapse_whitespace(&parts.join(" "));
    (word_count(&title) >= 4).then_some(Lm2RecoveredTitle { title, lines })
}

fn lm2_leading_source_title_candidate(line: &DeepLiquidSourceLine, text: &str) -> bool {
    if !lm2_title_candidate(text) || looks_like_lm2_author_heading(text) {
        return false;
    }
    if line
        .role_hint
        .is_some_and(|role| role_action(role) == Lm2Action::HideNoise)
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
        return true;
    }
    (line.centered || line.font_ratio_page >= 1.14)
        && (uppercase_ratio(text) >= 0.50 || title_case_ratio(text) >= 0.55)
}

fn lm2_source_title_author_like(line: &DeepLiquidSourceLine, text: &str) -> bool {
    let words = word_count(text);
    words >= 2
        && words <= 4
        && line.font_ratio_page < 1.12
        && (uppercase_ratio(text) >= 0.85 || title_case_ratio(text) >= 0.85)
}

fn lm2_recovered_title_is_better(recovered: &str, current: &str) -> bool {
    let recovered = recovered.trim();
    let current = current.trim();
    !current.is_empty()
        && word_count(recovered) >= word_count(current) + 3
        && normalize_text(recovered).ends_with(&normalize_text(current))
}

fn lm2_title_candidate(text: &str) -> bool {
    let lower = normalize_text(text);
    let trimmed = text.trim();
    !trimmed.is_empty()
        && !lm2_generic_title_label(trimmed)
        && !lm2_front_matter_stop_text(trimmed)
        && word_count(trimmed) <= 24
        && lower.chars().filter(|ch| ch.is_ascii_alphabetic()).count() >= 4
}

fn lm2_leading_title_candidate(block: &LiquidBlock) -> bool {
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
    words <= 16
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

fn lm2_generic_title_label(text: &str) -> bool {
    matches!(
        normalize_text(text).as_str(),
        "notes" | "note" | "article" | "abstract" | "introduction"
    )
}

fn lm2_front_matter_stop_text(text: &str) -> bool {
    let lower = normalize_text(text);
    lower.starts_with("abstract:")
        || lower.contains("................................................................")
        || looks_like_toc_entry(&lower)
        || lower.starts_with("copyright ")
}

fn lm2_probable_author_after_title(block: &LiquidBlock, title_parts: &[String]) -> bool {
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

fn looks_like_lm2_author_heading(text: &str) -> bool {
    let lower = normalize_text(text);
    text.contains('†')
        || text.contains('*')
        || lower.contains("j.d.")
        || lower.contains("candidate")
}

fn is_edge_line(line: &DeepLiquidSourceLine) -> bool {
    let height = line.page_height.max(1.0);
    let top = line.top / height;
    let bottom = line.bottom / height;
    top > 0.92 || bottom < 0.07
}

fn vertical_gap(previous: &DeepLiquidSourceLine, line: &DeepLiquidSourceLine) -> f32 {
    if previous.page_height <= 0.0 {
        return 0.0;
    }
    ((previous.bottom - line.top).abs() / previous.page_height).max(0.0)
}

fn read_json_file<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, String> {
    let bytes = std::fs::read(path)
        .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("Could not parse {}: {error}", path.display()))
}

fn reject_label_like_path(path: &Path) -> Result<(), String> {
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

fn eval_key(path: &str, page_index: usize, line_index: usize, text: &str) -> String {
    format!(
        "{}\u{1f}{page_index}\u{1f}{line_index}\u{1f}{}",
        path.to_ascii_lowercase(),
        collapse_whitespace(text)
    )
}

fn annotate_pp_priors_for_lines(
    runtime: &Lm2Runtime,
    path: &str,
    lines: &mut [DeepLiquidSourceLine],
) {
    for line in lines {
        annotate_pp_prior(runtime, path, line);
    }
}

fn annotate_pp_prior(runtime: &Lm2Runtime, path: &str, line: &mut DeepLiquidSourceLine) {
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

fn eval_sources_for_rows(
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

fn enrich_lm2_document_features(lines: &mut [DeepLiquidSourceLine]) {
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

fn enrich_lm2_tabular_position_features(lines: &mut [DeepLiquidSourceLine]) {
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

fn enrich_lm2_tabular_margin_features(lines: &mut [DeepLiquidSourceLine]) {
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

fn enrich_lm2_tabular_body_layout_features(lines: &mut [DeepLiquidSourceLine]) {
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

fn median_sorted(values: &[f32]) -> Option<f32> {
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

fn round5_f32(value: f32) -> f32 {
    (value * 100_000.0).round() / 100_000.0
}

fn enrich_lm2_dotleader_context_features(lines: &mut [DeepLiquidSourceLine]) {
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

fn enrich_lm2_geometric_footnote_zone_features(lines: &mut [DeepLiquidSourceLine]) {
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

fn geometric_font_cliff_start(lines: &[DeepLiquidSourceLine], indices: &[usize]) -> Option<usize> {
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

fn geometric_zone_start_line(line: &DeepLiquidSourceLine) -> bool {
    let y_center = ((line.top + line.bottom) * 0.5) / line.page_height.max(1.0);
    y_center >= 0.45
        && geometric_zone_small_font_line(line)
        && !geometric_zone_ignorable_line(line)
        && !geometric_zone_heading_like(line)
}

fn geometric_zone_continuation_line(line: &DeepLiquidSourceLine) -> bool {
    !geometric_zone_ignorable_line(line)
        && !geometric_zone_heading_like(line)
        && (geometric_zone_small_font_line(line)
            || looks_like_note_start(&line.text)
            || d1_runtime_citation_like(&normalize_text(&line.text)))
}

fn geometric_zone_small_font_line(line: &DeepLiquidSourceLine) -> bool {
    line.font_ratio_page_ref <= 0.88 || line.font_ratio_page <= 0.90 || line.font_ratio_doc <= 0.90
}

fn geometric_zone_body_font_line(line: &DeepLiquidSourceLine) -> bool {
    line.font_ratio_page_ref >= 0.98 && line.font_ratio_page >= 0.96 && line.font_ratio_doc >= 0.96
}

fn geometric_zone_ignorable_line(line: &DeepLiquidSourceLine) -> bool {
    let text = collapse_whitespace(&line.text);
    let lower = normalize_text(&text);
    text.is_empty()
        || looks_like_page_label_furniture(&text)
        || looks_like_running_header(&lower)
        || looks_like_toc_entry(&lower)
        || lm2_toc_dotleader_line(&text)
        || d1_runtime_artifact_like(&text)
}

fn geometric_zone_heading_like(line: &DeepLiquidSourceLine) -> bool {
    let text = collapse_whitespace(&line.text);
    let lower = normalize_text(&text);
    let words = word_count(&lower);
    (line.centered && line.bold && words <= 12 && !looks_like_note_start(&text))
        || (words <= 8 && uppercase_ratio(&text) >= 0.62 && !looks_like_note_start(&text))
        || (line.font_ratio_page_ref >= 1.02 && line.bold && words <= 12)
}

fn enrich_lm2_doc_font_features(lines: &mut [DeepLiquidSourceLine]) {
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

fn font_bucket_key(value: f32) -> Option<i32> {
    if value.is_finite() && value > 0.0 {
        Some((value * 10.0).round() as i32)
    } else {
        None
    }
}

fn dominant_font_key(keys: &[i32]) -> i32 {
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

fn dominant_lower_font_key(keys: &[i32], body_key: i32) -> Option<i32> {
    let mut counts: BTreeMap<i32, usize> = BTreeMap::new();
    for key in keys.iter().copied().filter(|key| *key < body_key) {
        *counts.entry(key).or_default() += 1;
    }
    counts
        .into_iter()
        .max_by_key(|(key, count)| (*count, *key))
        .map(|(key, _)| key)
}

fn enrich_lm2_footnote_state_features(lines: &mut [DeepLiquidSourceLine]) {
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

fn footnote_state_opens(line: &DeepLiquidSourceLine) -> bool {
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

fn footnote_state_body_resume_candidate(line: &DeepLiquidSourceLine) -> bool {
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
struct RepetitionBucket {
    pages: HashSet<usize>,
    indices: Vec<usize>,
}

fn enrich_lm2_repetition_features(lines: &mut [DeepLiquidSourceLine]) {
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

fn repetition_y_band(y_bottom: f32) -> usize {
    ((y_bottom.clamp(0.0, 0.999_999) * 20.0).floor() as usize).min(19)
}

fn repetition_text_fingerprint(text: &str) -> Option<String> {
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

fn enrich_lm2_vertical_axis_features(lines: &mut [DeepLiquidSourceLine]) {
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
struct MarkerPageInfo {
    indices: Vec<usize>,
    first_marker: Option<u16>,
    first_marker_index: Option<usize>,
    last_marker: Option<u16>,
}

fn enrich_lm2_marker_continuity_features(lines: &mut [DeepLiquidSourceLine]) {
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

fn leading_note_marker(text: &str) -> Option<u16> {
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
    (digits > 0 && (1..=500).contains(&value)).then_some(value as u16)
}

fn eval_source_line(row: &Lm2EvalRow, use_example_role_hints: bool) -> DeepLiquidSourceLine {
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
        left,
        bottom,
        right,
        top,
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

fn role_from_name(name: &str) -> Option<LiquidBlockRole> {
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

fn normalize_role_name(name: &str) -> String {
    name.trim().to_ascii_lowercase().replace('-', "_")
}

fn action_for_role_name(role: &str) -> Lm2Action {
    match normalize_role_name(role).as_str() {
        "footnote" | "marginalia" => Lm2Action::Marginalia,
        "header_footer" | "header" | "footer" | "contents" | "toc" | "table_of_contents"
        | "caption" | "metadata" | "noise" | "section_break" => Lm2Action::HideNoise,
        _ => Lm2Action::Keep,
    }
}

fn lm2_eval_report(
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

fn ratio(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

impl Lm2BlockQualityAccumulator {
    fn add_page(&mut self, decoded: &[(DeepLiquidSourceLine, Lm2Action)]) {
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

    fn finish(self) -> Lm2BlockQualityMetrics {
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

fn count_hyphen_artifacts(text: &str) -> usize {
    let chars = text.chars().collect::<Vec<_>>();
    chars
        .windows(3)
        .filter(|window| window[0] == '-' && window[1].is_whitespace() && window[2].is_lowercase())
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lm2_test_source_line(
        id: &str,
        line_index: usize,
        text: &str,
        font_ratio_page: f32,
        centered: bool,
        role_hint: Option<LiquidBlockRole>,
    ) -> DeepLiquidSourceLine {
        DeepLiquidSourceLine {
            id: id.to_owned(),
            page_index: 0,
            page_width: 1.0,
            page_height: 1.0,
            line_index,
            text: text.to_owned(),
            left: 0.1,
            bottom: 0.9 - (line_index as f32 * 0.02),
            right: 0.9,
            top: 0.92 - (line_index as f32 * 0.02),
            page_index_norm: 0.0,
            lines_from_doc_start: line_index,
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
            font_height: 1.0,
            font_ratio_page,
            font_ratio_page_ref: 1.0,
            font_ratio_doc: font_ratio_page,
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
            page_object_image_overlap_ratio: 0.0,
            page_object_image_hit_count: 0,
            page_object_path_stroke_near_line_count: 0,
            page_object_path_stroke_density_near_line: 0.0,
            page_object_thin_horizontal_near_line_count: 0,
            page_object_thin_vertical_near_line_count: 0,
            page_object_overlaps_image_bbox: false,
            page_object_ruled_row_membership: false,
            page_object_hide_candidate: false,
            page_object_hide_candidate_guarded: false,
            page_object_path15_candidate: false,
            page_object_ruled_or_path8_candidate: false,
            line_on_ruled_divider: false,
            in_ruled_cell: false,
            ruled_row_membership_exact: false,
            dist_to_nearest_rule: 0.0,
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
            bold: false,
            italic: false,
            centered,
            below_footnote_divider: false,
            page_has_footnote_divider: false,
            in_footnote_zone: false,
            pp_prior_role: None,
            pp_prior_label: None,
            pp_prior_score: None,
            role_hint,
            lv: Default::default(),
        }
    }

    #[test]
    fn geometric_footnote_zone_marks_lower_small_font_tail() {
        let mut lines = vec![
            lm2_test_source_line(
                "l0",
                0,
                "This is ordinary body text on the page.",
                1.0,
                false,
                None,
            ),
            lm2_test_source_line(
                "l1",
                1,
                "More ordinary body text above the notes.",
                1.0,
                false,
                None,
            ),
            lm2_test_source_line(
                "l18",
                18,
                "A final body line before the notes.",
                1.0,
                false,
                None,
            ),
            lm2_test_source_line("l22", 22, "1. See 123 U.S. 456 (1999).", 0.82, false, None),
            lm2_test_source_line(
                "l23",
                23,
                "Additional citation text continuing the footnote.",
                0.82,
                false,
                None,
            ),
            lm2_test_source_line("l24", 24, "https://example.com/archive", 0.82, false, None),
        ];
        for line in &mut lines {
            line.font_ratio_page_ref = line.font_ratio_page;
        }

        enrich_lm2_geometric_footnote_zone_features(&mut lines);

        assert!(!lines[0].in_footnote_zone);
        assert!(!lines[1].in_footnote_zone);
        assert!(!lines[2].in_footnote_zone);
        assert!(lines[3..].iter().all(|line| line.in_footnote_zone));
    }

    #[test]
    fn geometric_zone_overlay_keeps_heading_like_line() {
        let mut decoded = vec![
            (
                lm2_test_source_line(
                    "heading",
                    22,
                    "APPENDIX",
                    0.82,
                    true,
                    Some(LiquidBlockRole::Heading),
                ),
                Lm2Action::Keep,
            ),
            (
                lm2_test_source_line("note", 23, "1. See 123 U.S. 456 (1999).", 0.82, false, None),
                Lm2Action::Keep,
            ),
        ];
        decoded[0].0.in_footnote_zone = true;
        decoded[0].0.bold = true;
        decoded[0].0.font_ratio_page_ref = 0.82;
        decoded[1].0.in_footnote_zone = true;
        decoded[1].0.font_ratio_page_ref = 0.82;

        apply_d1_runtime_geometric_zone_overlay(&mut decoded);

        assert_eq!(decoded[0].1, Lm2Action::Keep);
        assert_eq!(decoded[1].1, Lm2Action::Marginalia);
    }

    fn lm2_zero_runtime(
        marker_decoder_prior: bool,
        small_font_decoder_prior: bool,
        small_font_sequence_prior: bool,
    ) -> Lm2Runtime {
        Lm2Runtime {
            model_label: "test-zero".to_owned(),
            load_warnings: Vec::new(),
            pp_priors: None,
            pp_footnote_region_membership: false,
            marker_decoder_prior,
            small_font_decoder_prior,
            small_font_sequence_prior,
            anchored_marginalia_flow_guard: false,
            body_preservation_guard: false,
            action_neutral_blocksplit: false,
            toc_overlay: false,
            front_matter_guard: false,
            marginalia_preservation_guard: false,
            start_score_scale: 1.0,
            transition_score_scale: 1.0,
            native_catboost_model: None,
            context_twopass_model: None,
            numeric_catboost_model: None,
            static_front_overlay: None,
            model: Some(Lm2Model {
                model_id: "test-zero".to_owned(),
                model_type: "hashed_softmax_action_v1".to_owned(),
                actions: ACTIONS.map(|action| action.as_str().to_owned()).to_vec(),
                feature_dim: 1,
                bias: vec![0.0, 0.0, 0.0],
                weights: vec![vec![0.0], vec![0.0], vec![0.0]],
                feature_schema: None,
                decoder_constants: None,
            }),
        }
    }

    #[test]
    fn progressive_preview_limits_pages_and_source_lines() {
        let request = LiquidMode2Request {
            document_epoch: 7,
            path: PathBuf::from("preview.pdf"),
            title: "Preview".to_owned(),
            pages: (0..8).map(|page| format!("page {page}")).collect(),
            deep_source_lines: (0..8)
                .map(|page| {
                    let mut line = lm2_test_source_line(
                        &format!("p{page}:l0"),
                        0,
                        &format!("page {page} body"),
                        1.0,
                        false,
                        None,
                    );
                    line.page_index = page;
                    line
                })
                .collect(),
            use_pymupdf_blocks: false,
            use_pp_footnote_regions: false,
            external_emissions_path: None,
        };

        let (preview, page_count) = lm2_progressive_preview_request(&request).unwrap();
        assert_eq!(page_count, LM2_PROGRESSIVE_PREVIEW_PAGES);
        assert_eq!(preview.pages.len(), LM2_PROGRESSIVE_PREVIEW_PAGES);
        assert_eq!(
            preview.deep_source_lines.len(),
            LM2_PROGRESSIVE_PREVIEW_PAGES
        );
        assert!(
            preview
                .deep_source_lines
                .iter()
                .all(|line| line.page_index < LM2_PROGRESSIVE_PREVIEW_PAGES)
        );
    }

    fn mark_pp_footnote(line: &mut DeepLiquidSourceLine) {
        line.pp_prior_role = Some("footnote".to_owned());
        line.pp_prior_label = Some("footnote".to_owned());
        line.pp_prior_score = Some(0.92);
    }

    #[test]
    fn legal_cue_does_not_match_substrings() {
        assert!(!has_legal_note_cue("said."));
        assert!(!has_legal_note_cue("paid."));
        assert!(!has_legal_note_cue("tennessee law"));
        assert!(has_legal_note_cue("see also"));
        assert!(has_legal_note_cue("id."));
        assert!(has_legal_note_cue("410 u.s."));
    }

    #[test]
    fn note_start_requires_uppercase_after_marker() {
        assert!(looks_like_note_start("13 Bankruptcy protection"));
        assert!(!looks_like_note_start("13 bankruptcy protection"));
        assert!(!looks_like_note_start("2018 law review"));
    }

    #[test]
    fn marginalia_note_block_start_accepts_punctuated_markers() {
        assert!(looks_like_marginalia_note_block_start(
            "13. Bankruptcy protection"
        ));
        assert!(looks_like_marginalia_note_block_start(
            "13) Bankruptcy protection"
        ));
        assert!(looks_like_marginalia_note_block_start(
            "13] Bankruptcy protection"
        ));
        assert!(!looks_like_marginalia_note_block_start(
            "13. bankruptcy protection"
        ));
        assert!(!looks_like_marginalia_note_block_start(
            "2024. The statute continues"
        ));
    }

    #[test]
    fn build_lm2_blocks_splits_punctuated_marginalia_note_starts() {
        let decoded = vec![
            (
                lm2_test_source_line(
                    "p0:l0",
                    0,
                    "12 First note begins with cited authority.",
                    0.82,
                    false,
                    None,
                ),
                Lm2Action::Marginalia,
            ),
            (
                lm2_test_source_line(
                    "p0:l1",
                    1,
                    "continues across the extracted PDF line.",
                    0.82,
                    false,
                    None,
                ),
                Lm2Action::Marginalia,
            ),
            (
                lm2_test_source_line(
                    "p0:l2",
                    2,
                    "13. Second note starts with punctuation.",
                    0.82,
                    false,
                    None,
                ),
                Lm2Action::Marginalia,
            ),
        ];

        let (_, blocks, sources) = build_lm2_blocks("", &decoded);
        let marginalia = blocks
            .iter()
            .filter(|block| block.role == LiquidBlockRole::Marginalia)
            .collect::<Vec<_>>();

        assert_eq!(marginalia.len(), 2);
        assert_eq!(
            marginalia[0].text,
            "12 First note begins with cited authority. continues across the extracted PDF line."
        );
        assert_eq!(
            marginalia[1].text,
            "13. Second note starts with punctuation."
        );
        let marginalia_sources = sources
            .iter()
            .filter(|source| {
                blocks
                    .get(source.block_index)
                    .is_some_and(|block| block.role == LiquidBlockRole::Marginalia)
            })
            .collect::<Vec<_>>();
        assert_eq!(
            marginalia_sources[0]
                .lines
                .iter()
                .filter_map(|line| line.id.as_deref())
                .collect::<Vec<_>>(),
            vec!["p0:l0", "p0:l1"]
        );
        assert_eq!(
            marginalia_sources[1]
                .lines
                .iter()
                .filter_map(|line| line.id.as_deref())
                .collect::<Vec<_>>(),
            vec!["p0:l2"]
        );
    }

    #[test]
    fn build_lm2_blocks_keeps_year_like_marginalia_continuations_together() {
        let decoded = vec![
            (
                lm2_test_source_line(
                    "p0:l0",
                    0,
                    "12 First note begins with cited authority.",
                    0.82,
                    false,
                    None,
                ),
                Lm2Action::Marginalia,
            ),
            (
                lm2_test_source_line(
                    "p0:l1",
                    1,
                    "2024. The statute continued to govern the dispute.",
                    0.82,
                    false,
                    None,
                ),
                Lm2Action::Marginalia,
            ),
        ];

        let (_, blocks, _) = build_lm2_blocks("", &decoded);
        let marginalia = blocks
            .iter()
            .filter(|block| block.role == LiquidBlockRole::Marginalia)
            .collect::<Vec<_>>();

        assert_eq!(marginalia.len(), 1);
        assert_eq!(
            marginalia[0].text,
            "12 First note begins with cited authority. 2024. The statute continued to govern the dispute."
        );
    }

    #[test]
    fn build_lm2_blocks_joins_adjacent_same_row_fragments() {
        let mut first = lm2_test_source_line("p0:l20", 20, "lions of dollars.1", 1.0, false, None);
        first.page_width = 612.0;
        first.page_height = 792.0;
        first.left = 138.24;
        first.right = 217.21;
        first.bottom = 387.02;
        first.top = 397.78;

        let mut second = lm2_test_source_line(
            "p0:l21",
            21,
            "Robers himself received only $500 per loan for his",
            1.0,
            false,
            None,
        );
        second.page_width = 612.0;
        second.page_height = 792.0;
        second.left = 217.20;
        second.right = 476.44;
        second.bottom = 387.01;
        second.top = 397.67;

        let decoded = vec![(first, Lm2Action::Keep), (second, Lm2Action::Keep)];
        let (_, blocks, sources) = build_lm2_blocks("", &decoded);

        assert_eq!(blocks.len(), 1);
        assert_eq!(
            blocks[0].text,
            "lions of dollars.1 Robers himself received only $500 per loan for his"
        );
        assert_eq!(
            sources[0]
                .lines
                .iter()
                .filter_map(|line| line.id.as_deref())
                .collect::<Vec<_>>(),
            vec!["p0:l20", "p0:l21"]
        );
    }

    #[test]
    fn build_lm2_blocks_keeps_wrap_after_right_side_fragment() {
        let mut first = lm2_test_source_line(
            "p0:l21",
            21,
            "Robers himself received only $500 per loan for his",
            1.0,
            false,
            None,
        );
        first.page_width = 612.0;
        first.page_height = 792.0;
        first.left = 217.20;
        first.right = 476.44;
        first.bottom = 387.01;
        first.top = 397.67;

        let mut second = lm2_test_source_line(
            "p0:l22",
            22,
            "participation in two closings; no payments were ever made, and the",
            1.0,
            false,
            None,
        );
        second.page_width = 612.0;
        second.page_height = 792.0;
        second.left = 138.24;
        second.right = 476.50;
        second.bottom = 375.01;
        second.top = 385.04;

        let decoded = vec![(first, Lm2Action::Keep), (second, Lm2Action::Keep)];
        let (_, blocks, _) = build_lm2_blocks("", &decoded);

        assert_eq!(blocks.len(), 1);
        assert_eq!(
            blocks[0].text,
            "Robers himself received only $500 per loan for his participation in two closings; no payments were ever made, and the"
        );
    }

    #[test]
    fn build_lm2_blocks_preserves_centered_all_caps_heading_hint() {
        let mut heading = lm2_test_source_line(
            "p0:l17",
            17,
            "INTRODUCTION",
            0.83,
            true,
            Some(LiquidBlockRole::Heading),
        );
        heading.page_width = 612.0;
        heading.page_height = 792.0;

        let body = lm2_test_source_line(
            "p0:l18",
            18,
            "In 2004 and 2005, Benjamin Robers was a straw buyer",
            1.0,
            false,
            Some(LiquidBlockRole::Paragraph),
        );
        let decoded = vec![(heading, Lm2Action::Keep), (body, Lm2Action::Keep)];
        let (_, blocks, _) = build_lm2_blocks("", &decoded);

        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].role, LiquidBlockRole::Heading);
        assert_eq!(blocks[0].text, "INTRODUCTION");
        assert_eq!(blocks[1].role, LiquidBlockRole::Paragraph);
    }

    #[test]
    fn build_lm2_blocks_joins_pdf_soft_hyphen_wraps() {
        let first = lm2_test_source_line("p0:l0", 0, "federal prop\u{0002}", 1.0, false, None);
        let second = lm2_test_source_line("p0:l1", 1, "erty interests", 1.0, false, None);
        let decoded = vec![(first, Lm2Action::Keep), (second, Lm2Action::Keep)];
        let (_, blocks, _) = build_lm2_blocks("", &decoded);

        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].text, "federal property interests");
    }

    #[test]
    fn action_neutral_blocksplit_splits_indented_paragraph_after_sentence_end() {
        let mut first =
            lm2_test_source_line("p0:l0", 0, "This paragraph ends cleanly.", 1.0, false, None);
        first.left = 72.0;
        first.page_width = 612.0;
        let mut second = lm2_test_source_line(
            "p0:l1",
            1,
            "The next paragraph starts with an indent.",
            1.0,
            false,
            None,
        );
        second.left = 84.0;
        second.page_width = 612.0;
        let decoded = vec![(first, Lm2Action::Keep), (second, Lm2Action::Keep)];

        let (_, mut blocks, mut sources) = build_lm2_blocks("", &decoded);
        assert_eq!(blocks.len(), 1);

        apply_action_neutral_blocksplit(&mut blocks, &mut sources, &decoded);

        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].role, LiquidBlockRole::Paragraph);
        assert_eq!(blocks[1].role, LiquidBlockRole::Paragraph);
        assert_eq!(
            sources
                .iter()
                .flat_map(|source| source.lines.iter())
                .filter_map(|line| line.id.as_deref())
                .collect::<Vec<_>>(),
            vec!["p0:l0", "p0:l1"]
        );
    }

    #[test]
    fn action_neutral_blocksplit_does_not_split_indented_continuation() {
        let mut first =
            lm2_test_source_line("p0:l0", 0, "This paragraph continues", 1.0, false, None);
        first.left = 72.0;
        first.page_width = 612.0;
        let mut second = lm2_test_source_line(
            "p0:l1",
            1,
            "with an indented wrapped line.",
            1.0,
            false,
            None,
        );
        second.left = 84.0;
        second.page_width = 612.0;
        let decoded = vec![(first, Lm2Action::Keep), (second, Lm2Action::Keep)];

        let (_, mut blocks, mut sources) = build_lm2_blocks("", &decoded);
        apply_action_neutral_blocksplit(&mut blocks, &mut sources, &decoded);

        assert_eq!(blocks.len(), 1);
        assert_eq!(
            sources[0]
                .lines
                .iter()
                .filter_map(|line| line.id.as_deref())
                .collect::<Vec<_>>(),
            vec!["p0:l0", "p0:l1"]
        );
    }

    #[test]
    fn deferred_marginalia_reflow_moves_note_after_open_paragraph() {
        let before = lm2_test_source_line(
            "p0:l0",
            0,
            "This sentence is interrupted by",
            1.0,
            false,
            None,
        );
        let note = lm2_test_source_line("p0:l1", 1, "12 A footnote.", 0.75, false, None);
        let after = lm2_test_source_line("p0:l2", 2, "a note in the middle.", 1.0, false, None);
        let mut blocks = vec![
            LiquidBlock {
                role: LiquidBlockRole::Paragraph,
                text: before.text.clone(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Marginalia,
                text: note.text.clone(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Paragraph,
                text: after.text.clone(),
                label: None,
            },
        ];
        let mut sources = vec![
            LiquidBlockSourceLines {
                block_index: 0,
                lines: vec![line_ref(&before, LiquidBlockRole::Paragraph)],
            },
            LiquidBlockSourceLines {
                block_index: 1,
                lines: vec![line_ref(&note, LiquidBlockRole::Marginalia)],
            },
            LiquidBlockSourceLines {
                block_index: 2,
                lines: vec![line_ref(&after, LiquidBlockRole::Paragraph)],
            },
        ];

        assert_eq!(
            apply_deferred_marginalia_reflow(&mut blocks, &mut sources),
            1
        );

        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].role, LiquidBlockRole::Paragraph);
        assert_eq!(
            blocks[0].text,
            "This sentence is interrupted by a note in the middle."
        );
        assert_eq!(blocks[1].role, LiquidBlockRole::Marginalia);
        assert_eq!(
            sources[0]
                .lines
                .iter()
                .filter_map(|line| line.id.as_deref())
                .collect::<Vec<_>>(),
            vec!["p0:l0", "p0:l2"]
        );
        assert_eq!(sources[1].block_index, 1);
        assert_eq!(
            sources[1]
                .lines
                .iter()
                .filter_map(|line| line.id.as_deref())
                .collect::<Vec<_>>(),
            vec!["p0:l1"]
        );
    }

    #[test]
    fn deferred_marginalia_reflow_dehyphenates_across_note() {
        let before = lm2_test_source_line("p0:l0", 0, "It asserts property-", 1.0, false, None);
        let note = lm2_test_source_line("p0:l1", 1, "65 A footnote.", 0.75, false, None);
        let after = lm2_test_source_line("p0:l2", 2, "like rights.", 1.0, false, None);
        let mut blocks = vec![
            LiquidBlock {
                role: LiquidBlockRole::Paragraph,
                text: before.text.clone(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Marginalia,
                text: note.text.clone(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Paragraph,
                text: after.text.clone(),
                label: None,
            },
        ];
        let mut sources = vec![
            LiquidBlockSourceLines {
                block_index: 0,
                lines: vec![line_ref(&before, LiquidBlockRole::Paragraph)],
            },
            LiquidBlockSourceLines {
                block_index: 1,
                lines: vec![line_ref(&note, LiquidBlockRole::Marginalia)],
            },
            LiquidBlockSourceLines {
                block_index: 2,
                lines: vec![line_ref(&after, LiquidBlockRole::Paragraph)],
            },
        ];

        assert_eq!(
            apply_deferred_marginalia_reflow(&mut blocks, &mut sources),
            1
        );
        assert_eq!(blocks[0].text, "It asserts property-like rights.");
    }

    #[test]
    fn deferred_marginalia_reflow_handles_multi_note_cluster() {
        let before = lm2_test_source_line("p0:l0", 0, "They assume, that", 1.0, false, None);
        let note_a = lm2_test_source_line("p0:l1", 1, "31 First note.", 0.75, false, None);
        let note_b = lm2_test_source_line("p0:l2", 2, "32 Second note.", 0.75, false, None);
        let note_c = lm2_test_source_line("p0:l3", 3, "33 Third note.", 0.75, false, None);
        let after = lm2_test_source_line(
            "p0:l4",
            4,
            "is, democratic accountability continues.",
            1.0,
            false,
            None,
        );
        let mut blocks = vec![
            LiquidBlock {
                role: LiquidBlockRole::Paragraph,
                text: before.text.clone(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Marginalia,
                text: note_a.text.clone(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Marginalia,
                text: note_b.text.clone(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Marginalia,
                text: note_c.text.clone(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Paragraph,
                text: after.text.clone(),
                label: None,
            },
        ];
        let mut sources = vec![
            LiquidBlockSourceLines {
                block_index: 0,
                lines: vec![line_ref(&before, LiquidBlockRole::Paragraph)],
            },
            LiquidBlockSourceLines {
                block_index: 1,
                lines: vec![line_ref(&note_a, LiquidBlockRole::Marginalia)],
            },
            LiquidBlockSourceLines {
                block_index: 2,
                lines: vec![line_ref(&note_b, LiquidBlockRole::Marginalia)],
            },
            LiquidBlockSourceLines {
                block_index: 3,
                lines: vec![line_ref(&note_c, LiquidBlockRole::Marginalia)],
            },
            LiquidBlockSourceLines {
                block_index: 4,
                lines: vec![line_ref(&after, LiquidBlockRole::Paragraph)],
            },
        ];

        assert_eq!(
            apply_deferred_marginalia_reflow(&mut blocks, &mut sources),
            1
        );
        assert_eq!(
            blocks[0].text,
            "They assume, that is, democratic accountability continues."
        );
        assert_eq!(
            blocks.iter().map(|block| block.role).collect::<Vec<_>>(),
            vec![
                LiquidBlockRole::Paragraph,
                LiquidBlockRole::Marginalia,
                LiquidBlockRole::Marginalia,
                LiquidBlockRole::Marginalia,
            ]
        );
    }

    #[test]
    fn deferred_marginalia_reflow_reaches_fixed_point_for_chained_notes() {
        let first = lm2_test_source_line("p0:l0", 0, "This starts", 1.0, false, None);
        let note_a = lm2_test_source_line("p0:l1", 1, "1 First note.", 0.75, false, None);
        let middle = lm2_test_source_line("p0:l2", 2, "a sentence that keeps", 1.0, false, None);
        let note_b = lm2_test_source_line("p0:l3", 3, "2 Second note.", 0.75, false, None);
        let last = lm2_test_source_line("p0:l4", 4, "going after another note.", 1.0, false, None);
        let mut blocks = vec![
            LiquidBlock {
                role: LiquidBlockRole::Paragraph,
                text: first.text.clone(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Marginalia,
                text: note_a.text.clone(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Paragraph,
                text: middle.text.clone(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Marginalia,
                text: note_b.text.clone(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Paragraph,
                text: last.text.clone(),
                label: None,
            },
        ];
        let mut sources = vec![
            LiquidBlockSourceLines {
                block_index: 0,
                lines: vec![line_ref(&first, LiquidBlockRole::Paragraph)],
            },
            LiquidBlockSourceLines {
                block_index: 1,
                lines: vec![line_ref(&note_a, LiquidBlockRole::Marginalia)],
            },
            LiquidBlockSourceLines {
                block_index: 2,
                lines: vec![line_ref(&middle, LiquidBlockRole::Paragraph)],
            },
            LiquidBlockSourceLines {
                block_index: 3,
                lines: vec![line_ref(&note_b, LiquidBlockRole::Marginalia)],
            },
            LiquidBlockSourceLines {
                block_index: 4,
                lines: vec![line_ref(&last, LiquidBlockRole::Paragraph)],
            },
        ];

        assert_eq!(
            apply_deferred_marginalia_reflow(&mut blocks, &mut sources),
            2
        );
        assert_eq!(
            blocks[0].text,
            "This starts a sentence that keeps going after another note."
        );
        assert_eq!(
            blocks.iter().map(|block| block.role).collect::<Vec<_>>(),
            vec![
                LiquidBlockRole::Paragraph,
                LiquidBlockRole::Marginalia,
                LiquidBlockRole::Marginalia,
            ]
        );
    }

    #[test]
    fn deferred_marginalia_reflow_keeps_note_after_closed_sentence() {
        let before =
            lm2_test_source_line("p0:l0", 0, "This paragraph is complete.", 1.0, false, None);
        let note = lm2_test_source_line("p0:l1", 1, "12 A footnote.", 0.75, false, None);
        let after = lm2_test_source_line(
            "p0:l2",
            2,
            "the next line begins lowercase but should stay separate.",
            1.0,
            false,
            None,
        );
        let mut blocks = vec![
            LiquidBlock {
                role: LiquidBlockRole::Paragraph,
                text: before.text.clone(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Marginalia,
                text: note.text.clone(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Paragraph,
                text: after.text.clone(),
                label: None,
            },
        ];
        let mut sources = vec![
            LiquidBlockSourceLines {
                block_index: 0,
                lines: vec![line_ref(&before, LiquidBlockRole::Paragraph)],
            },
            LiquidBlockSourceLines {
                block_index: 1,
                lines: vec![line_ref(&note, LiquidBlockRole::Marginalia)],
            },
            LiquidBlockSourceLines {
                block_index: 2,
                lines: vec![line_ref(&after, LiquidBlockRole::Paragraph)],
            },
        ];

        assert_eq!(
            apply_deferred_marginalia_reflow(&mut blocks, &mut sources),
            0
        );
        assert_eq!(blocks[0].text, "This paragraph is complete.");
        assert_eq!(blocks[1].role, LiquidBlockRole::Marginalia);
        assert_eq!(blocks[2].role, LiquidBlockRole::Paragraph);
    }

    #[test]
    fn action_neutral_blocksplit_splits_numbered_marginalia_start() {
        let mut first = lm2_test_source_line(
            "p0:l0",
            0,
            "12 First note begins with cited authority.",
            0.82,
            false,
            None,
        );
        first.left = 48.0;
        first.page_width = 612.0;
        let mut second = lm2_test_source_line(
            "p0:l1",
            1,
            "13. Second note starts with punctuation.",
            0.82,
            false,
            None,
        );
        second.left = 60.0;
        second.page_width = 612.0;
        let decoded = vec![
            (first, Lm2Action::Marginalia),
            (second, Lm2Action::Marginalia),
        ];

        let (_, mut blocks, mut sources) = build_lm2_blocks("", &decoded);
        apply_action_neutral_blocksplit(&mut blocks, &mut sources, &decoded);

        assert_eq!(blocks.len(), 2);
        assert!(
            blocks
                .iter()
                .all(|block| block.role == LiquidBlockRole::Marginalia)
        );
    }

    #[test]
    fn toc_overlay_hides_document_local_dotleader_rows() {
        let dotleader = lm2_test_source_line(
            "p0:l0",
            0,
            "I. THEORETICAL BACKGROUND................................. 2260",
            0.88,
            false,
            None,
        );
        let body = lm2_test_source_line(
            "p0:l1",
            1,
            "Ordinary article prose continues here.",
            1.0,
            false,
            None,
        );
        let mut decoded = vec![(dotleader, Lm2Action::Marginalia), (body, Lm2Action::Keep)];

        apply_document_toc_overlay(&mut decoded);

        assert_eq!(decoded[0].1, Lm2Action::HideNoise);
        assert_eq!(decoded[1].1, Lm2Action::Keep);
    }

    #[test]
    fn toc_overlay_recovers_matching_section_heading() {
        let dotleader = lm2_test_source_line(
            "p0:l0",
            0,
            "I. THEORETICAL BACKGROUND................................. 2260",
            0.88,
            false,
            None,
        );
        let mut heading =
            lm2_test_source_line("p2:l4", 4, "I. Theoretical Background", 1.05, false, None);
        heading.page_index = 2;
        heading.bold = true;
        let mut decoded = vec![
            (dotleader, Lm2Action::HideNoise),
            (heading, Lm2Action::Marginalia),
        ];

        apply_document_toc_overlay(&mut decoded);

        assert_eq!(decoded[1].1, Lm2Action::Keep);
        assert_eq!(decoded[1].0.role_hint, Some(LiquidBlockRole::Heading));

        let (_, blocks, _) = build_lm2_blocks("", &decoded);
        assert!(
            blocks
                .iter()
                .any(|block| block.role == LiquidBlockRole::Heading
                    && block.text == "I. Theoretical Background")
        );
    }

    #[test]
    fn front_matter_guard_hides_first_page_noise_hint_masthead() {
        let mut line = lm2_test_source_line(
            "p0:l2",
            2,
            "VIRGINIA LAW REVIEW",
            1.8,
            true,
            Some(LiquidBlockRole::Noise),
        );
        line.top = 0.88;
        line.bottom = 0.84;
        let mut decoded = vec![(line, Lm2Action::Keep)];

        apply_front_matter_guard(&mut decoded);

        assert_eq!(decoded[0].1, Lm2Action::HideNoise);
        assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Noise));
    }

    #[test]
    fn static_front_overlay_does_not_promote_noise_hint_masthead() {
        let mut rows = HashMap::new();
        rows.insert("p0:l2".to_owned(), LiquidBlockRole::Title);
        let mut roles_by_doc_line = HashMap::new();
        roles_by_doc_line.insert("doc.pdf".to_owned(), rows);
        let overlay = Lm2StaticFrontOverlay {
            source_label: "test".to_owned(),
            roles_by_doc_line,
        };
        let mut line = lm2_test_source_line(
            "p0:l2",
            2,
            "VIRGINIA LAW REVIEW",
            1.8,
            true,
            Some(LiquidBlockRole::Noise),
        );
        line.top = 0.88;
        line.bottom = 0.84;
        let mut decoded = vec![(line, Lm2Action::Keep)];

        apply_static_front_overlay(&overlay, Path::new("doc.pdf"), &mut decoded);

        assert_eq!(decoded[0].1, Lm2Action::HideNoise);
        assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Noise));
    }

    #[test]
    fn front_matter_guard_hides_first_page_repository_boilerplate() {
        let mut line = lm2_test_source_line(
            "p0:l14",
            14,
            "History. It has been accepted for inclusion in Fordham Law Review Archive by an authorized editor.",
            0.9,
            true,
            Some(LiquidBlockRole::Noise),
        );
        line.top = 0.14;
        line.bottom = 0.12;
        let mut decoded = vec![(line, Lm2Action::Keep)];

        apply_front_matter_guard(&mut decoded);

        assert_eq!(decoded[0].1, Lm2Action::HideNoise);
    }

    #[test]
    fn front_matter_guard_hides_repeated_top_edge_noise_furniture() {
        let mut line = lm2_test_source_line(
            "p5:l0",
            0,
            "COPYRIGHT © 2026 VIRGINIA LAW REVIEW ASSOCIATION",
            0.50,
            false,
            None,
        );
        line.page_index = 5;
        line.top = 0.953;
        line.bottom = 0.941;
        let mut decoded = vec![(line, Lm2Action::Keep)];

        apply_front_matter_guard(&mut decoded);

        assert_eq!(decoded[0].1, Lm2Action::HideNoise);
    }

    #[test]
    fn front_matter_guard_hides_first_page_masthead_without_role_hint() {
        let mut line = lm2_test_source_line("p0:l2", 2, "VIRGINIA LAW REVIEW", 1.8, true, None);
        line.top = 0.88;
        line.bottom = 0.84;
        let mut decoded = vec![(line, Lm2Action::Keep)];

        apply_front_matter_guard(&mut decoded);

        assert_eq!(decoded[0].1, Lm2Action::HideNoise);
    }

    #[test]
    fn front_matter_guard_demotes_first_page_author_byline() {
        let mut line = lm2_test_source_line(
            "p0:l5",
            5,
            "Aric Short† and Tanya Pierce††",
            1.05,
            true,
            None,
        );
        line.top = 0.72;
        line.bottom = 0.70;
        let mut decoded = vec![(line, Lm2Action::Keep)];

        apply_front_matter_guard(&mut decoded);

        assert_eq!(decoded[0].1, Lm2Action::Marginalia);
        assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Marginalia));
    }

    #[test]
    fn page_label_furniture_guard_hides_page_x_of_y_labels() {
        let line = lm2_test_source_line("p14:l0", 0, "Page 14 of 26", 0.84, false, None);
        let mut decoded = vec![(line, Lm2Action::Marginalia)];

        apply_page_label_furniture_guard(&mut decoded);

        assert_eq!(decoded[0].1, Lm2Action::HideNoise);
        assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Noise));
    }

    #[test]
    fn page_label_furniture_guard_hides_page_x_labels() {
        let line = lm2_test_source_line("p4:l0", 0, "Page 4", 0.84, false, None);
        let mut decoded = vec![(line, Lm2Action::Keep)];

        apply_page_label_furniture_guard(&mut decoded);

        assert_eq!(decoded[0].1, Lm2Action::HideNoise);
        assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Noise));
    }

    #[test]
    fn page_label_furniture_guard_does_not_hide_prose_with_page_word() {
        let line = lm2_test_source_line(
            "p9:l12",
            12,
            "Page limits can affect appellate briefing schedules.",
            1.0,
            false,
            None,
        );
        let mut decoded = vec![(line, Lm2Action::Keep)];

        apply_page_label_furniture_guard(&mut decoded);

        assert_eq!(decoded[0].1, Lm2Action::Keep);
    }

    #[test]
    fn page_label_furniture_guard_hides_centered_numeric_with_noise_hint() {
        let line =
            lm2_test_source_line("p1:l18", 18, "5", 0.59, true, Some(LiquidBlockRole::Noise));
        let mut decoded = vec![(line, Lm2Action::Keep)];

        apply_page_label_furniture_guard(&mut decoded);

        assert_eq!(decoded[0].1, Lm2Action::HideNoise);
        assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Noise));
    }

    #[test]
    fn page_label_furniture_guard_hides_parenthesized_numeric_with_marginalia_hint() {
        let line = lm2_test_source_line(
            "p0:l1",
            1,
            "(1139)",
            0.86,
            true,
            Some(LiquidBlockRole::Marginalia),
        );
        let mut decoded = vec![(line, Lm2Action::Keep)];

        apply_page_label_furniture_guard(&mut decoded);

        assert_eq!(decoded[0].1, Lm2Action::HideNoise);
        assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Noise));
    }

    #[test]
    fn page_label_furniture_guard_keeps_unhinted_centered_numeric_line() {
        let line = lm2_test_source_line("p3:l8", 8, "12", 1.0, true, None);
        let mut decoded = vec![(line, Lm2Action::Keep)];

        apply_page_label_furniture_guard(&mut decoded);

        assert_eq!(decoded[0].1, Lm2Action::Keep);
        assert_eq!(decoded[0].0.role_hint, None);
    }

    #[test]
    fn page_label_furniture_guard_keeps_noncentered_numeric_with_noise_hint() {
        let line = lm2_test_source_line(
            "p4:l22",
            22,
            "22",
            0.80,
            false,
            Some(LiquidBlockRole::Noise),
        );
        let mut decoded = vec![(line, Lm2Action::Keep)];

        apply_page_label_furniture_guard(&mut decoded);

        assert_eq!(decoded[0].1, Lm2Action::Keep);
        assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Noise));
    }

    #[test]
    fn marginalia_preservation_guard_recovers_hidden_url_continuation() {
        let mut line = lm2_test_source_line(
            "p14:l33",
            33,
            "nviction/ [https://perma.cc/U6R4-UBLJ] (last visited Feb. 11, 2026) (\"State post-conviction",
            0.78,
            false,
            Some(LiquidBlockRole::Marginalia),
        );
        line.page_index = 14;
        line.bottom = 0.16;
        line.top = 0.18;
        line.page_has_footnote_divider = true;
        let mut decoded = vec![(line, Lm2Action::HideNoise)];

        apply_marginalia_preservation_guard(&mut decoded);

        assert_eq!(decoded[0].1, Lm2Action::Marginalia);
        assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Marginalia));
    }

    #[test]
    fn marginalia_preservation_guard_recovers_url_continuation_without_hint() {
        let mut line = lm2_test_source_line(
            "p10:l29",
            29,
            "innocenceproject.org/dna-exonerations-in-the-united-states/ [https://perma.cc/V4GA-ZCJT]",
            0.80,
            false,
            None,
        );
        line.page_index = 10;
        line.bottom = 266.994;
        line.top = 279.11697;
        line.page_height = 792.0;
        let mut decoded = vec![(line, Lm2Action::HideNoise)];

        apply_marginalia_preservation_guard(&mut decoded);

        assert_eq!(decoded[0].1, Lm2Action::Marginalia);
        assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Marginalia));
    }

    #[test]
    fn marginalia_preservation_guard_does_not_recover_toc_dotleader() {
        let mut line = lm2_test_source_line(
            "p1:l10",
            10,
            "A. Governance ..................................................... 1613",
            0.88,
            false,
            Some(LiquidBlockRole::Marginalia),
        );
        line.page_index = 1;
        line.bottom = 0.18;
        line.top = 0.20;
        line.page_has_footnote_divider = true;
        let mut decoded = vec![(line, Lm2Action::HideNoise)];

        apply_marginalia_preservation_guard(&mut decoded);

        assert_eq!(decoded[0].1, Lm2Action::HideNoise);
    }

    #[test]
    fn append_line_repairs_simple_dehyphenation() {
        let mut text = "applica-".to_owned();
        append_line(&mut text, "tion");
        assert_eq!(text, "application");
    }

    #[test]
    fn append_line_preserves_common_terminal_hyphen_compounds() {
        let mut state_court = "state-".to_owned();
        append_line(&mut state_court, "court adjudication");
        assert_eq!(state_court, "state-court adjudication");

        let mut case_specific = "case-".to_owned();
        append_line(&mut case_specific, "specific inquiry");
        assert_eq!(case_specific, "case-specific inquiry");
    }

    #[test]
    fn assembly_attaches_standalone_body_marker_without_glue() {
        let mut body = lm2_test_source_line(
            "p0:l10",
            10,
            "Those contradictions are structural.",
            1.0,
            false,
            None,
        );
        body.page_width = 612.0;
        body.page_height = 792.0;
        body.left = 72.0;
        body.right = 300.0;
        body.bottom = 500.0;
        body.top = 512.0;
        body.font_height = 12.0;

        let mut marker = lm2_test_source_line("p0:l11", 11, "224", 0.5, false, None);
        marker.page_width = 612.0;
        marker.page_height = 792.0;
        marker.left = 304.0;
        marker.right = 318.0;
        marker.bottom = 507.0;
        marker.top = 514.0;
        marker.font_height = 6.0;

        let decoded = vec![(body, Lm2Action::Keep), (marker, Lm2Action::Keep)];
        let (_, blocks, sources) = build_lm2_blocks("", &decoded);

        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].text, "Those contradictions are structural. 224");
        assert_eq!(
            sources[0]
                .lines
                .iter()
                .filter_map(|line| line.id.as_deref())
                .collect::<Vec<_>>(),
            vec!["p0:l10", "p0:l11"]
        );
    }

    #[test]
    fn assembly_attached_marker_does_not_force_next_line_to_marker_geometry() {
        let mut body =
            lm2_test_source_line("p0:l10", 10, "The policy was tailored.", 1.0, false, None);
        body.page_width = 612.0;
        body.page_height = 792.0;
        body.left = 72.0;
        body.right = 260.0;
        body.bottom = 500.0;
        body.top = 512.0;
        body.font_height = 12.0;

        let mut marker = lm2_test_source_line("p0:l11", 11, "15", 0.5, false, None);
        marker.page_width = 612.0;
        marker.page_height = 792.0;
        marker.left = 264.0;
        marker.right = 274.0;
        marker.bottom = 507.0;
        marker.top = 514.0;
        marker.font_height = 6.0;

        let mut next = lm2_test_source_line(
            "p0:l12",
            12,
            "The next paragraph starts independently.",
            1.0,
            false,
            None,
        );
        next.page_width = 612.0;
        next.page_height = 792.0;
        next.left = 84.0;
        next.right = 390.0;
        next.bottom = 450.0;
        next.top = 462.0;
        next.font_height = 12.0;

        let decoded = vec![
            (body, Lm2Action::Keep),
            (marker, Lm2Action::Keep),
            (next, Lm2Action::Keep),
        ];
        let (_, blocks, _) = build_lm2_blocks("", &decoded);

        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].text, "The policy was tailored. 15");
        assert_eq!(blocks[1].text, "The next paragraph starts independently.");
    }

    #[test]
    fn production_slug_boilerplate_is_hidden() {
        assert!(looks_like_production_slug_boilerplate(
            "SPERBER IN PRINTER PREP (DO NOT DELETE) 3/17/2026 10:46 AM"
        ));
        assert!(looks_like_production_slug_boilerplate(
            "Copyright 2026 by Austin Kruse Printed in U.S.A."
        ));
        assert!(looks_like_production_slug_boilerplate(
            "BIONDI IN PRINTER FINAL (Do Not Delete) 2/6/2023 11:09 AM"
        ));
    }

    #[test]
    fn production_slug_boilerplate_does_not_hide_ordinary_text() {
        assert!(!looks_like_production_slug_boilerplate(
            "The Article was printed and distributed in the United States."
        ));
        assert!(!looks_like_production_slug_boilerplate(
            "Copyright law gives authors exclusive rights in original works."
        ));
        assert!(!looks_like_production_slug_boilerplate(
            "Do not delete evidence before the litigation hold is lifted."
        ));
    }

    #[test]
    fn footnote_state_carries_across_page_until_body_resume() {
        let mut body = lm2_test_source_line(
            "p0:l0",
            0,
            "Ordinary body text continues here",
            1.0,
            false,
            None,
        );
        body.font_height = 12.0;
        body.bottom = 0.75;
        body.top = 0.77;

        let mut note = lm2_test_source_line(
            "p0:l1",
            1,
            "1 Footnote text begins below divider",
            0.75,
            false,
            None,
        );
        note.font_height = 9.0;
        note.bottom = 0.12;
        note.top = 0.14;
        note.below_footnote_divider = true;
        note.page_has_footnote_divider = true;

        let mut continuation = lm2_test_source_line(
            "p1:l0",
            0,
            "continues from the prior page",
            0.75,
            false,
            None,
        );
        continuation.page_index = 1;
        continuation.font_height = 9.0;
        continuation.bottom = 0.78;
        continuation.top = 0.80;

        let mut resume = lm2_test_source_line(
            "p1:l1",
            1,
            "The Article returns to body prose",
            1.0,
            false,
            None,
        );
        resume.page_index = 1;
        resume.font_height = 12.0;
        resume.bottom = 0.66;
        resume.top = 0.68;

        let mut lines = vec![body, note, continuation, resume];
        enrich_lm2_document_features(&mut lines);

        assert!(!lines[0].doc_footnote_state);
        assert!(lines[1].doc_footnote_state);
        assert!(!lines[1].doc_footnote_continuation);
        assert!(lines[2].doc_footnote_state);
        assert!(lines[2].doc_footnote_continuation);
        assert!(!lines[3].doc_footnote_state);
        assert!(!lines[3].doc_footnote_continuation);
    }

    #[test]
    fn footnote_carryover_overlay_marks_open_previous_page_continuation() {
        let mut previous = lm2_test_source_line(
            "p0:l20",
            20,
            "12. This footnote continues with",
            0.82,
            false,
            None,
        );
        previous.page_width = 612.0;
        previous.page_height = 792.0;
        previous.page_index = 0;
        previous.doc_note_marker = 12;
        previous.font_height = 8.0;
        previous.doc_font_footnote_size = 8.0;
        previous.doc_font_body_z = 1.4;
        previous.doc_font_footnote_z = 0.0;

        let mut continuation = lm2_test_source_line(
            "p1:l0",
            0,
            "additional discussion of the cited authority",
            0.82,
            false,
            None,
        );
        continuation.page_width = 612.0;
        continuation.page_height = 792.0;
        continuation.page_index = 1;
        continuation.font_height = 8.0;
        continuation.font_ratio_page_ref = 0.82;
        continuation.doc_font_footnote_size = 8.0;
        continuation.doc_font_body_z = 1.4;
        continuation.doc_font_footnote_z = 0.0;

        let mut decoded = vec![
            (previous, Lm2Action::Marginalia),
            (continuation, Lm2Action::Keep),
        ];
        apply_footnote_carryover_overlay(&mut decoded);

        assert_eq!(decoded[1].1, Lm2Action::Marginalia);
        assert_eq!(decoded[1].0.role_hint, Some(LiquidBlockRole::Footnote));
    }

    #[test]
    fn footnote_carryover_overlay_stops_at_expected_next_marker() {
        let mut previous = lm2_test_source_line(
            "p0:l20",
            20,
            "5. This footnote continues with",
            0.82,
            false,
            None,
        );
        previous.page_width = 612.0;
        previous.page_height = 792.0;
        previous.page_index = 0;
        previous.doc_note_marker = 5;
        previous.font_height = 8.0;
        previous.doc_font_footnote_size = 8.0;
        previous.doc_font_body_z = 1.4;
        previous.doc_font_footnote_z = 0.0;

        let mut next_marker = lm2_test_source_line(
            "p1:l0",
            0,
            "6. The next note begins here.",
            0.82,
            false,
            None,
        );
        next_marker.page_width = 612.0;
        next_marker.page_height = 792.0;
        next_marker.page_index = 1;
        next_marker.doc_note_marker = 6;
        next_marker.font_height = 8.0;
        next_marker.font_ratio_page_ref = 0.82;
        next_marker.doc_font_footnote_size = 8.0;
        next_marker.doc_font_body_z = 1.4;
        next_marker.doc_font_footnote_z = 0.0;

        let mut small_body = lm2_test_source_line(
            "p1:l1",
            1,
            "small-font line after the next note marker",
            0.82,
            false,
            None,
        );
        small_body.page_width = 612.0;
        small_body.page_height = 792.0;
        small_body.page_index = 1;
        small_body.font_height = 8.0;
        small_body.font_ratio_page_ref = 0.82;
        small_body.doc_font_footnote_size = 8.0;
        small_body.doc_font_body_z = 1.4;
        small_body.doc_font_footnote_z = 0.0;

        let mut decoded = vec![
            (previous, Lm2Action::Marginalia),
            (next_marker, Lm2Action::Marginalia),
            (small_body, Lm2Action::Keep),
        ];
        apply_footnote_carryover_overlay(&mut decoded);

        assert_eq!(decoded[2].1, Lm2Action::Keep);
    }

    #[test]
    fn repetition_features_mark_three_page_edge_fingerprints() {
        let mut header0 = lm2_test_source_line(
            "p0:l0",
            0,
            "2026] THE FUGITIVE SLAVE ACT",
            0.86,
            false,
            None,
        );
        let mut header1 = lm2_test_source_line(
            "p1:l0",
            0,
            "2027] THE FUGITIVE SLAVE ACT",
            0.86,
            false,
            None,
        );
        let mut header2 = lm2_test_source_line(
            "p2:l0",
            0,
            "2028] THE FUGITIVE SLAVE ACT",
            0.86,
            false,
            None,
        );
        let mut body = lm2_test_source_line(
            "p2:l1",
            1,
            "A non-edge repeated phrase should not be marked.",
            1.0,
            false,
            None,
        );

        header0.page_index = 0;
        header1.page_index = 1;
        header2.page_index = 2;
        for line in [&mut header0, &mut header1, &mut header2] {
            line.bottom = 0.90;
            line.top = 0.92;
        }
        body.page_index = 2;
        body.bottom = 0.40;
        body.top = 0.42;

        let mut lines = vec![header0, header1, header2, body];
        enrich_lm2_document_features(&mut lines);

        for line in lines.iter().take(3) {
            assert!(line.doc_repeated_edge_text);
            assert_eq!(line.doc_repeated_text_count, 3);
            assert!(line.doc_repeated_top_edge);
            assert!(!line.doc_repeated_bottom_edge);
            assert!(line.doc_repeated_numeric_pattern);
        }
        assert!(!lines[3].doc_repeated_edge_text);
    }

    #[test]
    fn vertical_numeric_axis_marks_same_x_tick_stack() {
        let mut tick0 = lm2_test_source_line("p0:l0", 0, "0%", 1.0, false, None);
        let mut tick1 = lm2_test_source_line("p0:l1", 1, "10%", 1.0, false, None);
        let mut tick2 = lm2_test_source_line("p0:l2", 2, "20%", 1.0, false, None);
        let mut body_callout = lm2_test_source_line("p0:l3", 3, "7", 1.0, false, None);
        for (line, bottom) in [
            (&mut tick0, 0.20_f32),
            (&mut tick1, 0.34_f32),
            (&mut tick2, 0.48_f32),
        ] {
            line.left = 0.10;
            line.right = 0.14;
            line.bottom = bottom;
            line.top = bottom + 0.02;
        }
        body_callout.left = 0.50;
        body_callout.right = 0.53;
        body_callout.bottom = 0.42;
        body_callout.top = 0.44;

        let mut lines = vec![tick0, tick1, tick2, body_callout];
        enrich_lm2_document_features(&mut lines);

        for line in lines.iter().take(3) {
            assert!(line.doc_vertical_axis_like);
            assert!(line.doc_vertical_numeric_axis_like);
            assert!(!line.doc_vertical_short_text_axis_like);
        }
        assert!(!lines[3].doc_vertical_axis_like);
    }

    #[test]
    fn numeric_catboost_features_include_table_numeric_cell_like() {
        let mut line = lm2_test_source_line("p0:l0", 0, "10%", 1.0, false, None);
        line.left = 0.12;
        line.right = 0.17;
        let features = lm2_numeric_catboost_features(&line);
        assert_eq!(features.get("table_numeric_cell_like").copied(), Some(1.0));
    }

    #[test]
    fn table_column_feature_marks_repeated_grid_cells() {
        let mut lines = Vec::new();
        for row in 0..3 {
            for col in 0..2 {
                let mut line = lm2_test_source_line(
                    &format!("p0:l{}", row * 2 + col),
                    row * 2 + col,
                    if col == 0 { "2019" } else { "$42" },
                    1.0,
                    false,
                    None,
                );
                line.left = 0.20 + (col as f32 * 0.18);
                line.right = line.left + 0.06;
                line.bottom = 0.20 + (row as f32 * 0.08);
                line.top = line.bottom + 0.02;
                lines.push(line);
            }
        }

        enrich_lm2_document_features(&mut lines);

        assert!(lines.iter().all(|line| line.page_table_column_like));
        let features = lm2_numeric_catboost_features(&lines[0]);
        assert_eq!(features.get("page_table_column_like").copied(), Some(1.0));
    }

    #[test]
    fn dotleader_context_marks_following_toc_line_without_dots() {
        let rows = [
            "Introduction ........................................ 1",
            "Background . . . . . . . . . . . . . . . . . . . . 3",
            "Methodology ....................................... 8",
            "Appendix A",
        ];
        let mut lines = rows
            .iter()
            .enumerate()
            .map(|(index, text)| {
                lm2_test_source_line(&format!("p0:l{}", index), index, text, 1.0, false, None)
            })
            .collect::<Vec<_>>();

        enrich_lm2_document_features(&mut lines);

        assert!(lines[1].prev_line_has_dotleader);
        assert_eq!(lines[3].prev4_dotleader_count, 2);
        assert_eq!(lines[3].prev4_spaced_dotleader_count, 3);
        assert_eq!(lines[3].prev4_strong_dotleader_count, 3);
        assert!(lines[3].prev4_toc_leader_context);
        let features = lm2_numeric_catboost_features(&lines[3]);
        assert_eq!(features.get("prev4_toc_leader_context").copied(), Some(1.0));
        assert_eq!(features.get("prev4_dotleader_count").copied(), Some(2.0));
    }

    #[test]
    fn column_spacing_features_mark_numeric_table_line() {
        let line = lm2_test_source_line(
            "p0:l0",
            0,
            "White applicants     42%     37%     21%",
            1.0,
            false,
            None,
        );

        let features = lm2_numeric_catboost_features(&line);

        assert_eq!(features.get("internal_space_run_max").copied(), Some(5.0));
        assert_eq!(features.get("numeric_token_count").copied(), Some(3.0));
        assert_eq!(features.get("percent_token_count").copied(), Some(3.0));
        assert_eq!(
            features.get("has_large_internal_space_gap").copied(),
            Some(1.0)
        );
        assert_eq!(
            features.get("columnar_numeric_text_like").copied(),
            Some(1.0)
        );
    }

    #[test]
    fn marker_continuity_marks_mid_sequence_next_page() {
        let mut note1 = lm2_test_source_line(
            "p0:l0",
            0,
            "1 First marker begins the notes",
            0.82,
            false,
            None,
        );
        let mut note2 = lm2_test_source_line(
            "p0:l1",
            1,
            "2 Second marker closes the page",
            0.82,
            false,
            None,
        );
        let mut note3 = lm2_test_source_line(
            "p1:l0",
            0,
            "3 Continued marker opens the next page",
            0.82,
            false,
            None,
        );
        let mut continuation = lm2_test_source_line(
            "p1:l1",
            1,
            "Continuation prose inherits the page marker signal",
            0.82,
            false,
            None,
        );
        note1.page_index = 0;
        note2.page_index = 0;
        note3.page_index = 1;
        continuation.page_index = 1;

        let mut lines = vec![note1, note2, note3, continuation];
        enrich_lm2_document_features(&mut lines);

        assert_eq!(lines[0].doc_note_marker, 1);
        assert!(lines[0].doc_note_marker_first_on_page);
        assert!(!lines[0].doc_note_marker_mid_sequence_page);
        assert_eq!(lines[1].doc_note_marker, 2);
        assert!(!lines[1].doc_note_marker_first_on_page);

        assert_eq!(lines[2].doc_note_marker, 3);
        assert!(lines[2].doc_note_marker_first_on_page);
        assert!(lines[2].doc_note_marker_mid_sequence_page);
        assert!(lines[2].doc_note_marker_follows_previous_page);
        assert_eq!(lines[2].doc_note_marker_page_delta, 1);

        assert_eq!(lines[3].doc_note_marker, 0);
        assert!(!lines[3].doc_note_marker_first_on_page);
        assert!(lines[3].doc_note_marker_mid_sequence_page);
        assert!(lines[3].doc_note_marker_follows_previous_page);
        assert_eq!(lines[3].doc_note_marker_page_delta, 1);
    }

    #[test]
    fn filename_fallback_title_detection_is_narrow() {
        assert!(looks_like_filename_fallback_title(
            "duke_law_journal__DLJ_vol72_iss5_Building_Trusts.pdf"
        ));
        assert!(looks_like_filename_fallback_title(
            "northwestern_university_law_review__place_names"
        ));
        assert!(!looks_like_filename_fallback_title(
            "PLACE NAMES AND PRESIDENTIAL CONTROL"
        ));
        assert!(!looks_like_filename_fallback_title("Is Tax “Law”?"));
    }

    #[test]
    fn lm2_fallback_title_skips_generic_heading_labels() {
        let blocks = vec![
            LiquidBlock {
                role: LiquidBlockRole::Heading,
                text: "Notes".to_owned(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Heading,
                text: "BUILDING TRUST(S): RETHINKING ASSET".to_owned(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Heading,
                text: "RETURN IN KLEPTOCRACY FORFEITURES".to_owned(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Paragraph,
                text: "ABSTRACT".to_owned(),
                label: None,
            },
        ];

        assert_eq!(
            lm2_fallback_title_from_blocks(&blocks).as_deref(),
            Some("BUILDING TRUST(S): RETHINKING ASSET RETURN IN KLEPTOCRACY FORFEITURES")
        );
    }

    #[test]
    fn lm2_fallback_title_stops_before_author_heading() {
        let blocks = vec![
            LiquidBlock {
                role: LiquidBlockRole::Heading,
                text: "Article".to_owned(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Heading,
                text: "Regulatory History and Judicial Review".to_owned(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Heading,
                text: "Todd Phillips† and Anthony Moffa††".to_owned(),
                label: None,
            },
        ];

        assert_eq!(
            lm2_fallback_title_from_blocks(&blocks).as_deref(),
            Some("Regulatory History and Judicial Review")
        );
    }

    #[test]
    fn lm2_fallback_title_prefers_leading_paragraph_over_author_title() {
        let blocks = vec![
            LiquidBlock {
                role: LiquidBlockRole::Paragraph,
                text: "RETHINKING ROBOT LIABILITY".to_owned(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Title,
                text: "ZACHARY HENDERSON".to_owned(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Paragraph,
                text: "INTRODUCTION................................................................480".to_owned(),
                label: None,
            },
        ];

        assert_eq!(
            lm2_fallback_title_from_blocks(&blocks).as_deref(),
            Some("RETHINKING ROBOT LIABILITY")
        );
    }

    #[test]
    fn lm2_fallback_title_stops_before_toc_dotleaders() {
        let blocks = vec![
            LiquidBlock {
                role: LiquidBlockRole::Paragraph,
                text: "EQUITABLE REGULATORY BALANCING".to_owned(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Paragraph,
                text: "INTRODUCTION................................................................532".to_owned(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Heading,
                text: "B. In Search of Preliminary Relief in Regulatory Cases".to_owned(),
                label: None,
            },
        ];

        assert_eq!(
            lm2_fallback_title_from_blocks(&blocks).as_deref(),
            Some("EQUITABLE REGULATORY BALANCING")
        );
    }

    #[test]
    fn lm2_fallback_title_ignores_later_citation_fragments() {
        let blocks = vec![
            LiquidBlock {
                role: LiquidBlockRole::Paragraph,
                text: "CORPORATE GOODWILL".to_owned(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Paragraph,
                text: "INTRODUCTION................................................................587".to_owned(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Title,
                text: "Esty & Quentin Karpilow, Harnessing Investor Interest in Sustainability"
                    .to_owned(),
                label: None,
            },
        ];

        assert_eq!(
            lm2_fallback_title_from_blocks(&blocks).as_deref(),
            Some("CORPORATE GOODWILL")
        );
    }

    #[test]
    fn lm2_decoder_constants_can_change_sequence_choice() {
        let lines = vec![
            lm2_test_source_line("p0:l0", 0, "Ordinary body text", 1.0, false, None),
            lm2_test_source_line("p0:l1", 1, "continues on the next line", 1.0, false, None),
        ];
        let base_runtime = Lm2Runtime {
            model_label: "test".to_owned(),
            load_warnings: Vec::new(),
            pp_priors: None,
            pp_footnote_region_membership: false,
            marker_decoder_prior: false,
            small_font_decoder_prior: false,
            small_font_sequence_prior: false,
            anchored_marginalia_flow_guard: false,
            body_preservation_guard: false,
            action_neutral_blocksplit: false,
            toc_overlay: false,
            front_matter_guard: false,
            marginalia_preservation_guard: false,
            start_score_scale: 1.0,
            transition_score_scale: 1.0,
            native_catboost_model: None,
            context_twopass_model: None,
            numeric_catboost_model: None,
            static_front_overlay: None,
            model: Some(Lm2Model {
                model_id: "test".to_owned(),
                model_type: "hashed_softmax_action_v1".to_owned(),
                actions: ACTIONS.map(|action| action.as_str().to_owned()).to_vec(),
                feature_dim: 1,
                bias: vec![0.0, 0.0, 0.0],
                weights: vec![vec![0.0], vec![0.0], vec![0.0]],
                feature_schema: None,
                decoder_constants: None,
            }),
        };
        let base = decode_page(&base_runtime, &lines);
        assert_eq!(base[0].1, Lm2Action::Keep);
        assert_eq!(base[1].1, Lm2Action::Keep);

        let mut weights = HashMap::new();
        weights.insert("start_arc:marginalia".to_owned(), 4.0);
        weights.insert("transition_arc:marginalia->marginalia".to_owned(), 4.0);
        let fitted_runtime = Lm2Runtime {
            model_label: "test-fitted".to_owned(),
            load_warnings: Vec::new(),
            pp_priors: None,
            pp_footnote_region_membership: false,
            marker_decoder_prior: false,
            small_font_decoder_prior: false,
            small_font_sequence_prior: false,
            anchored_marginalia_flow_guard: false,
            body_preservation_guard: false,
            action_neutral_blocksplit: false,
            toc_overlay: false,
            front_matter_guard: false,
            marginalia_preservation_guard: false,
            start_score_scale: 1.0,
            transition_score_scale: 1.0,
            native_catboost_model: None,
            context_twopass_model: None,
            numeric_catboost_model: None,
            static_front_overlay: None,
            model: Some(Lm2Model {
                model_id: "test-fitted".to_owned(),
                model_type: "hashed_softmax_action_v1".to_owned(),
                actions: ACTIONS.map(|action| action.as_str().to_owned()).to_vec(),
                feature_dim: 1,
                bias: vec![0.0, 0.0, 0.0],
                weights: vec![vec![0.0], vec![0.0], vec![0.0]],
                feature_schema: None,
                decoder_constants: Some(Lm2DecoderConstants { weights }),
            }),
        };
        let fitted = decode_page(&fitted_runtime, &lines);
        assert_eq!(fitted[0].1, Lm2Action::Marginalia);
        assert_eq!(fitted[1].1, Lm2Action::Marginalia);
    }

    #[test]
    fn marker_decoder_prior_is_off_by_default() {
        let mut line = lm2_test_source_line(
            "p0:l0",
            0,
            "continuing discussion, see id.",
            0.82,
            false,
            None,
        );
        line.bottom = 0.30;
        line.top = 0.32;
        line.doc_note_marker_mid_sequence_page = true;
        line.doc_note_marker_follows_previous_page = true;

        assert!(marker_continuity_decoder_prior_eligible(&line));
        let decoded = decode_page(&lm2_zero_runtime(false, false, false), &[line]);
        assert_eq!(decoded[0].1, Lm2Action::Keep);
    }

    #[test]
    fn marker_decoder_prior_can_recover_supported_continuation() {
        let mut line = lm2_test_source_line(
            "p0:l0",
            0,
            "continuing discussion, see id.",
            0.82,
            false,
            None,
        );
        line.bottom = 0.30;
        line.top = 0.32;
        line.doc_note_marker_mid_sequence_page = true;
        line.doc_note_marker_follows_previous_page = true;

        let decoded = decode_page(&lm2_zero_runtime(true, false, false), &[line]);
        assert_eq!(decoded[0].1, Lm2Action::Marginalia);
    }

    #[test]
    fn marker_decoder_prior_rejects_noise_shapes() {
        let mut header =
            lm2_test_source_line("p0:l0", 0, "2026] Law Review Vol. 44", 0.82, false, None);
        header.doc_note_marker_mid_sequence_page = true;
        header.doc_note_marker_follows_previous_page = true;
        assert!(!marker_continuity_decoder_prior_eligible(&header));

        let mut table = lm2_test_source_line(
            "p0:l1",
            1,
            "continuing discussion, see id.",
            0.82,
            false,
            Some(LiquidBlockRole::Table),
        );
        table.bottom = 0.30;
        table.top = 0.32;
        table.doc_note_marker_mid_sequence_page = true;
        table.doc_note_marker_follows_previous_page = true;
        assert!(!marker_continuity_decoder_prior_eligible(&table));
    }

    #[test]
    fn small_font_decoder_prior_is_off_by_default() {
        let mut line = lm2_test_source_line(
            "p0:l0",
            0,
            "source-tail footnote prose continues here, see id.",
            0.82,
            false,
            None,
        );
        line.font_ratio_doc = 0.82;
        line.bottom = 0.30;
        line.top = 0.32;

        assert!(small_font_lower_page_decoder_prior_eligible(&line));
        let decoded = decode_page(&lm2_zero_runtime(false, false, false), &[line]);
        assert_eq!(decoded[0].1, Lm2Action::Keep);
    }

    #[test]
    fn small_font_decoder_prior_can_recover_lower_page_note_prose() {
        let mut line = lm2_test_source_line(
            "p0:l0",
            0,
            "source-tail footnote prose continues here, see id.",
            0.82,
            false,
            None,
        );
        line.font_ratio_doc = 0.82;
        line.bottom = 0.30;
        line.top = 0.32;

        let decoded = decode_page(&lm2_zero_runtime(false, true, false), &[line]);
        assert_eq!(decoded[0].1, Lm2Action::Marginalia);
    }

    #[test]
    fn small_font_decoder_prior_rejects_upper_page_and_title_like_shapes() {
        let mut upper = lm2_test_source_line(
            "p0:l0",
            0,
            "source-tail footnote prose continues here, see id.",
            0.82,
            false,
            None,
        );
        upper.font_ratio_doc = 0.82;
        upper.bottom = 0.66;
        upper.top = 0.68;
        assert!(!small_font_lower_page_decoder_prior_eligible(&upper));

        let mut heading =
            lm2_test_source_line("p0:l1", 1, "SUPERIOR OR OTHER OFFICER", 0.82, false, None);
        heading.font_ratio_doc = 0.82;
        heading.bottom = 0.30;
        heading.top = 0.32;
        assert!(!small_font_lower_page_decoder_prior_eligible(&heading));
    }

    #[test]
    fn small_font_decoder_prior_rejects_page_furniture_boilerplate() {
        for text in [
            "[Vol. 46",
            "Page 12",
            "Published by Scholar Commons, 1976",
            "This Article is brought to you for free and open access by FLASH",
            "Fordham Law Archive of Scholarship and History",
            "For more information, please contact tmelnick@law.fordham.edu.",
            "https://scholarcommons.sc.edu/sclr/vol5/iss3/6",
            "© 2024 University Press – All rights reserved",
            "footnote continued on next page",
        ] {
            let mut line = lm2_test_source_line("p0:l0", 0, text, 0.82, false, None);
            line.font_ratio_doc = 0.82;
            line.bottom = 0.30;
            line.top = 0.32;
            assert!(
                !small_font_lower_page_decoder_prior_eligible(&line),
                "{text}"
            );
        }

        let mut note = lm2_test_source_line(
            "p0:l1",
            1,
            "source-tail footnote prose continues here, see id.",
            0.82,
            false,
            None,
        );
        note.font_ratio_doc = 0.82;
        note.bottom = 0.30;
        note.top = 0.32;
        assert!(small_font_lower_page_decoder_prior_eligible(&note));
    }

    #[test]
    fn small_font_decoder_prior_requires_note_evidence() {
        let mut ordinary = lm2_test_source_line(
            "p0:l0",
            0,
            "source-tail prose continues here without a note cue",
            0.82,
            false,
            None,
        );
        ordinary.font_ratio_doc = 0.82;
        ordinary.bottom = 0.30;
        ordinary.top = 0.32;
        assert!(!small_font_lower_page_decoder_prior_eligible(&ordinary));

        let mut note_start = lm2_test_source_line(
            "p0:l1",
            1,
            "17 Supported note text begins",
            0.82,
            false,
            None,
        );
        note_start.font_ratio_doc = 0.82;
        note_start.bottom = 0.30;
        note_start.top = 0.32;
        assert!(small_font_lower_page_decoder_prior_eligible(&note_start));
    }

    #[test]
    fn small_font_sequence_prior_is_transition_only() {
        let mut line = lm2_test_source_line(
            "p0:l0",
            0,
            "continuing discussion, see id.",
            0.82,
            false,
            None,
        );
        line.font_ratio_doc = 0.82;
        line.bottom = 0.30;
        line.top = 0.32;

        assert!(small_font_lower_page_decoder_prior_eligible(&line));
        assert_eq!(
            small_font_sequence_continuation_prior(&line, Lm2Action::Keep, Lm2Action::Marginalia),
            0.0
        );
        assert!(
            small_font_sequence_continuation_prior(
                &line,
                Lm2Action::Marginalia,
                Lm2Action::Marginalia
            ) > 0.0
        );

        let decoded = decode_page(&lm2_zero_runtime(false, false, true), &[line]);
        assert_eq!(decoded[0].1, Lm2Action::Keep);
    }

    #[test]
    fn anchored_marginalia_flow_guard_caps_unanchored_continuations() {
        let lines = vec![
            lm2_test_source_line("p0:l0", 0, "1. See id.", 0.82, false, None),
            lm2_test_source_line(
                "p0:l1",
                1,
                "continued prose without an anchor",
                0.82,
                false,
                None,
            ),
            lm2_test_source_line("p0:l2", 2, "more continued prose", 0.82, false, None),
            lm2_test_source_line(
                "p0:l3",
                3,
                "third unanchored continuation",
                0.82,
                false,
                None,
            ),
        ];
        let mut path = vec![
            Lm2Action::Marginalia,
            Lm2Action::Marginalia,
            Lm2Action::Marginalia,
            Lm2Action::Marginalia,
        ];

        apply_anchored_marginalia_flow_guard(&lines, &mut path);

        assert_eq!(
            path,
            vec![
                Lm2Action::Marginalia,
                Lm2Action::Marginalia,
                Lm2Action::Marginalia,
                Lm2Action::Keep,
            ]
        );
    }

    #[test]
    fn anchored_marginalia_flow_guard_rejects_unanchored_starts() {
        let lines = vec![
            lm2_test_source_line("p0:l0", 0, "ordinary small text", 0.82, false, None),
            lm2_test_source_line("p0:l1", 1, "still no note anchor", 0.82, false, None),
            lm2_test_source_line("p0:l2", 2, "2. Anchored note starts", 0.82, false, None),
            lm2_test_source_line("p0:l3", 3, "allowed continuation", 0.82, false, None),
        ];
        let mut path = vec![
            Lm2Action::Marginalia,
            Lm2Action::Marginalia,
            Lm2Action::Marginalia,
            Lm2Action::Marginalia,
        ];

        apply_anchored_marginalia_flow_guard(&lines, &mut path);

        assert_eq!(
            path,
            vec![
                Lm2Action::Keep,
                Lm2Action::Keep,
                Lm2Action::Marginalia,
                Lm2Action::Marginalia,
            ]
        );
    }

    #[test]
    fn anchored_marginalia_flow_guard_applies_after_decode() {
        let lines = vec![
            lm2_test_source_line("p0:l0", 0, "1. See id.", 0.82, false, None),
            lm2_test_source_line("p0:l1", 1, "first continuation", 0.82, false, None),
            lm2_test_source_line("p0:l2", 2, "second continuation", 0.82, false, None),
            lm2_test_source_line("p0:l3", 3, "third continuation", 0.82, false, None),
        ];
        let mut weights = HashMap::new();
        weights.insert("start_arc:marginalia".to_owned(), 4.0);
        weights.insert("transition_arc:marginalia->marginalia".to_owned(), 4.0);
        let runtime = Lm2Runtime {
            model_label: "test-flow-guard".to_owned(),
            load_warnings: Vec::new(),
            pp_priors: None,
            pp_footnote_region_membership: false,
            marker_decoder_prior: false,
            small_font_decoder_prior: false,
            small_font_sequence_prior: false,
            anchored_marginalia_flow_guard: true,
            body_preservation_guard: false,
            action_neutral_blocksplit: false,
            toc_overlay: false,
            front_matter_guard: false,
            marginalia_preservation_guard: false,
            start_score_scale: 1.0,
            transition_score_scale: 1.0,
            native_catboost_model: None,
            context_twopass_model: None,
            numeric_catboost_model: None,
            static_front_overlay: None,
            model: Some(Lm2Model {
                model_id: "test-flow-guard".to_owned(),
                model_type: "hashed_softmax_action_v1".to_owned(),
                actions: ACTIONS.map(|action| action.as_str().to_owned()).to_vec(),
                feature_dim: 1,
                bias: vec![0.0, 0.0, 0.0],
                weights: vec![vec![0.0], vec![0.0], vec![0.0]],
                feature_schema: None,
                decoder_constants: Some(Lm2DecoderConstants { weights }),
            }),
        };

        let decoded = decode_page(&runtime, &lines)
            .into_iter()
            .map(|(_, action)| action)
            .collect::<Vec<_>>();

        assert_eq!(
            decoded,
            vec![
                Lm2Action::Marginalia,
                Lm2Action::Marginalia,
                Lm2Action::Marginalia,
                Lm2Action::Keep,
            ]
        );
    }

    #[test]
    fn guarded_pp_prior_rejects_numeric_footnote_rows() {
        assert!(guarded_pp_prior_action(
            Some("marginalia"),
            "footnote",
            "footnote",
            0.80,
            "Some real note text"
        ));
        assert!(!guarded_pp_prior_action(
            Some("marginalia"),
            "footnote",
            "footnote",
            0.799,
            "Some real note text"
        ));
        assert!(guarded_pp_prior_action(
            Some("marginalia"),
            "footnote",
            "footnote",
            0.90,
            "Some real note text"
        ));
        assert!(!guarded_pp_prior_action(
            Some("marginalia"),
            "footnote",
            "footnote",
            0.90,
            "775"
        ));
        assert!(guarded_pp_prior_action(
            Some("hide_noise"),
            "table",
            "table",
            0.75,
            "table row"
        ));
    }

    #[test]
    fn pp_footnote_prior_can_change_sequence_choice() {
        let mut line =
            lm2_test_source_line("p0:l0", 0, "ordinary looking note prose", 1.0, false, None);
        let mut rows = HashMap::new();
        rows.insert(
            pp_prior_key("doc.pdf", line.page_index, line.line_index, &line.text),
            Lm2PpPrior {
                role: "footnote".to_owned(),
                label: "footnote".to_owned(),
                score: 0.92,
            },
        );
        let runtime = Lm2Runtime {
            model_label: "test-pp".to_owned(),
            load_warnings: Vec::new(),
            pp_priors: Some(Lm2PpPriorIndex {
                source: PathBuf::from("pp.jsonl"),
                rows,
            }),
            pp_footnote_region_membership: false,
            marker_decoder_prior: false,
            small_font_decoder_prior: false,
            small_font_sequence_prior: false,
            anchored_marginalia_flow_guard: false,
            body_preservation_guard: false,
            action_neutral_blocksplit: false,
            toc_overlay: false,
            front_matter_guard: false,
            marginalia_preservation_guard: false,
            start_score_scale: 1.0,
            transition_score_scale: 1.0,
            native_catboost_model: None,
            context_twopass_model: None,
            numeric_catboost_model: None,
            static_front_overlay: None,
            model: Some(Lm2Model {
                model_id: "test-pp".to_owned(),
                model_type: "hashed_softmax_action_v1".to_owned(),
                actions: ACTIONS.map(|action| action.as_str().to_owned()).to_vec(),
                feature_dim: 1,
                bias: vec![0.0, 0.0, 0.0],
                weights: vec![vec![0.0], vec![0.0], vec![0.0]],
                feature_schema: None,
                decoder_constants: None,
            }),
        };
        annotate_pp_prior(&runtime, "doc.pdf", &mut line);
        assert_eq!(line.pp_prior_role.as_deref(), Some("footnote"));
        let decoded = decode_page(&runtime, &[line]);
        assert_eq!(decoded[0].1, Lm2Action::Marginalia);
    }

    #[test]
    fn pp_footnote_membership_override_is_post_decode_only() {
        let mut line =
            lm2_test_source_line("p0:l0", 0, "Ordinary looking note prose", 1.0, false, None);
        line.bottom = 0.30;
        line.top = 0.32;
        line.font_ratio_doc = 0.88;
        line.font_ratio_page = 0.88;
        line.pp_prior_role = Some("footnote".to_owned());
        line.pp_prior_label = Some("footnote".to_owned());
        line.pp_prior_score = Some(0.92);
        let runtime = Lm2Runtime {
            model_label: "test-pp-membership".to_owned(),
            load_warnings: Vec::new(),
            pp_priors: None,
            pp_footnote_region_membership: true,
            marker_decoder_prior: false,
            small_font_decoder_prior: false,
            small_font_sequence_prior: false,
            anchored_marginalia_flow_guard: false,
            body_preservation_guard: false,
            action_neutral_blocksplit: false,
            toc_overlay: false,
            front_matter_guard: false,
            marginalia_preservation_guard: false,
            start_score_scale: 1.0,
            transition_score_scale: 1.0,
            native_catboost_model: None,
            context_twopass_model: None,
            numeric_catboost_model: None,
            static_front_overlay: None,
            model: Some(Lm2Model {
                model_id: "test-pp-membership".to_owned(),
                model_type: "hashed_softmax_action_v1".to_owned(),
                actions: ACTIONS.map(|action| action.as_str().to_owned()).to_vec(),
                feature_dim: 1,
                bias: vec![0.0, 0.0, 0.0],
                weights: vec![vec![0.0], vec![0.0], vec![0.0]],
                feature_schema: None,
                decoder_constants: None,
            }),
        };

        let decoded = decode_page(&runtime, &[line]);
        assert_eq!(decoded[0].1, Lm2Action::Marginalia);
    }

    #[test]
    fn pp_footnote_membership_rejects_numeric_rows() {
        let mut line = lm2_test_source_line("p0:l0", 0, "775", 1.0, false, None);
        mark_pp_footnote(&mut line);
        assert!(!pp_footnote_region_member(&line));
    }

    #[test]
    fn pp_footnote_membership_accepts_region_member_without_geometry() {
        let mut line =
            lm2_test_source_line("p0:l0", 0, "ordinary looking body prose", 1.0, false, None);
        mark_pp_footnote(&mut line);
        let lines = vec![line];
        let mut path = vec![Lm2Action::Keep];

        apply_pp_footnote_region_membership(&lines, &mut path);

        assert_eq!(path, vec![Lm2Action::Marginalia]);
    }

    #[test]
    fn pp_footnote_membership_accepts_score_below_high_confidence_cutoff() {
        let mut line =
            lm2_test_source_line("p0:l0", 0, "Ordinary looking note prose", 0.88, false, None);
        line.bottom = 0.30;
        line.top = 0.32;
        line.font_ratio_doc = 0.88;
        mark_pp_footnote(&mut line);
        line.pp_prior_score = Some(0.89);
        let lines = vec![line];
        let mut path = vec![Lm2Action::Keep];

        apply_pp_footnote_region_membership(&lines, &mut path);

        assert_eq!(path, vec![Lm2Action::Marginalia]);
    }

    #[test]
    fn pp_footnote_membership_accepts_score_at_promotion_threshold() {
        let mut line =
            lm2_test_source_line("p0:l0", 0, "Ordinary looking note prose", 0.88, false, None);
        mark_pp_footnote(&mut line);
        line.pp_prior_score = Some(0.80);

        assert!(pp_footnote_region_member(&line));
    }

    #[test]
    fn pp_footnote_membership_rejects_score_below_promotion_threshold() {
        let mut line =
            lm2_test_source_line("p0:l0", 0, "Ordinary looking note prose", 0.88, false, None);
        mark_pp_footnote(&mut line);
        line.pp_prior_score = Some(0.799);

        assert!(!pp_footnote_region_member(&line));
    }

    #[test]
    fn pp_footnote_membership_forward_closure_rejects_furniture_rows() {
        let mut seed = lm2_test_source_line(
            "p0:l0",
            0,
            "12. See United States v. Alvarez",
            0.82,
            false,
            None,
        );
        seed.bottom = 0.30;
        seed.top = 0.32;
        mark_pp_footnote(&mut seed);
        let mut furniture = lm2_test_source_line("p0:l1", 1, "Page 12", 0.82, false, None);
        furniture.bottom = 0.28;
        furniture.top = 0.30;
        let lines = vec![seed, furniture];
        let mut path = vec![Lm2Action::Keep; lines.len()];

        apply_pp_footnote_region_membership(&lines, &mut path);

        assert_eq!(path, vec![Lm2Action::Marginalia, Lm2Action::Keep]);
    }

    #[test]
    fn pp_footnote_membership_backfills_from_region_member_to_note_start() {
        let mut start = lm2_test_source_line(
            "p0:l0",
            0,
            "9. Steve Quinn, Congress and the States",
            0.98,
            false,
            None,
        );
        start.bottom = 0.34;
        start.top = 0.36;
        let mut continuation = lm2_test_source_line(
            "p0:l1",
            1,
            "standing are beyond the scope of this example",
            0.98,
            false,
            None,
        );
        continuation.bottom = 0.32;
        continuation.top = 0.34;
        let mut seed = lm2_test_source_line(
            "p0:l2",
            2,
            "states have standing under the rule",
            0.88,
            false,
            None,
        );
        seed.bottom = 0.30;
        seed.top = 0.32;
        seed.font_ratio_doc = 0.88;
        mark_pp_footnote(&mut seed);
        let lines = vec![start, continuation, seed];
        let mut path = vec![Lm2Action::Keep; lines.len()];

        apply_pp_footnote_region_membership(&lines, &mut path);

        assert_eq!(
            path,
            vec![
                Lm2Action::Marginalia,
                Lm2Action::Marginalia,
                Lm2Action::Marginalia,
            ]
        );
    }

    #[test]
    fn pp_footnote_membership_does_not_backward_close_non_start_continuation() {
        let mut first = lm2_test_source_line(
            "p0:l0",
            0,
            "customers goods there exposed as illustrations, see id.",
            0.82,
            false,
            None,
        );
        first.bottom = 0.34;
        first.top = 0.36;
        first.font_ratio_doc = 0.82;
        let mut second = lm2_test_source_line(
            "p0:l1",
            1,
            "the legal rule discussed below, see id.",
            0.82,
            false,
            None,
        );
        second.bottom = 0.32;
        second.top = 0.34;
        second.font_ratio_doc = 0.82;
        let mut seed = lm2_test_source_line(
            "p0:l2",
            2,
            "mark cases later cited the same passage",
            0.82,
            false,
            None,
        );
        seed.bottom = 0.30;
        seed.top = 0.32;
        mark_pp_footnote(&mut seed);
        let lines = vec![first, second, seed];
        let mut path = vec![Lm2Action::Keep; lines.len()];

        apply_pp_footnote_region_membership(&lines, &mut path);

        assert_eq!(
            path,
            vec![Lm2Action::Keep, Lm2Action::Keep, Lm2Action::Marginalia,]
        );
    }

    #[test]
    fn pp_footnote_membership_rejects_geometry_only_backward_continuation() {
        let mut geometry_only = lm2_test_source_line(
            "p0:l0",
            0,
            "ordinary small font paragraph text before the note",
            0.82,
            false,
            None,
        );
        geometry_only.bottom = 0.34;
        geometry_only.top = 0.36;
        geometry_only.font_ratio_doc = 0.82;
        let mut seed =
            lm2_test_source_line("p0:l1", 1, "12. Seed region member", 0.82, false, None);
        seed.bottom = 0.30;
        seed.top = 0.32;
        mark_pp_footnote(&mut seed);
        let lines = vec![geometry_only, seed];
        let mut path = vec![Lm2Action::Keep; lines.len()];

        apply_pp_footnote_region_membership(&lines, &mut path);

        assert_eq!(path, vec![Lm2Action::Keep, Lm2Action::Marginalia]);
    }

    #[test]
    fn pp_footnote_membership_rejects_page_divider_only_backward_continuation() {
        let mut page_divider_only = lm2_test_source_line(
            "p0:l0",
            0,
            "ordinary paragraph text above the page footnote divider",
            1.0,
            false,
            None,
        );
        page_divider_only.page_has_footnote_divider = true;
        page_divider_only.below_footnote_divider = false;
        let mut seed =
            lm2_test_source_line("p0:l1", 1, "12. Seed region member", 0.82, false, None);
        seed.bottom = 0.30;
        seed.top = 0.32;
        mark_pp_footnote(&mut seed);
        let lines = vec![page_divider_only, seed];
        let mut path = vec![Lm2Action::Keep; lines.len()];

        apply_pp_footnote_region_membership(&lines, &mut path);

        assert_eq!(path, vec![Lm2Action::Keep, Lm2Action::Marginalia]);
    }

    #[test]
    fn pp_footnote_membership_rejects_below_divider_backward_continuation() {
        let mut below_divider = lm2_test_source_line(
            "p0:l0",
            0,
            "continuation text below the footnote divider",
            1.0,
            false,
            None,
        );
        below_divider.page_has_footnote_divider = true;
        below_divider.below_footnote_divider = true;
        let mut seed =
            lm2_test_source_line("p0:l1", 1, "12. Seed region member", 0.82, false, None);
        seed.bottom = 0.30;
        seed.top = 0.32;
        mark_pp_footnote(&mut seed);
        let lines = vec![below_divider, seed];
        let mut path = vec![Lm2Action::Keep; lines.len()];

        apply_pp_footnote_region_membership(&lines, &mut path);

        assert_eq!(path, vec![Lm2Action::Keep, Lm2Action::Marginalia]);
    }

    #[test]
    fn pp_footnote_membership_does_not_backward_close_before_seed() {
        let mut body = lm2_test_source_line(
            "p0:l0",
            0,
            "The article returns to ordinary body prose.",
            1.0,
            false,
            None,
        );
        body.bottom = 0.62;
        body.top = 0.64;
        let mut continuation = lm2_test_source_line(
            "p0:l1",
            1,
            "continued discussion, see id.",
            0.82,
            false,
            None,
        );
        continuation.bottom = 0.32;
        continuation.top = 0.34;
        let mut seed = lm2_test_source_line(
            "p0:l2",
            2,
            "mark cases later cited the same passage",
            0.82,
            false,
            None,
        );
        seed.bottom = 0.30;
        seed.top = 0.32;
        mark_pp_footnote(&mut seed);
        let lines = vec![body, continuation, seed];
        let mut path = vec![Lm2Action::Keep; lines.len()];

        apply_pp_footnote_region_membership(&lines, &mut path);

        assert_eq!(
            path,
            vec![Lm2Action::Keep, Lm2Action::Keep, Lm2Action::Marginalia,]
        );
    }

    #[test]
    fn pp_footnote_membership_does_not_backward_close_across_same_page_run() {
        let mut continuations = (0..=4)
            .map(|index| {
                let mut line = lm2_test_source_line(
                    &format!("p0:l{index}"),
                    index,
                    &format!("continuation {index}, see id."),
                    0.82,
                    false,
                    None,
                );
                line.bottom = 0.36 - (index as f32 * 0.01);
                line.top = 0.38 - (index as f32 * 0.01);
                line
            })
            .collect::<Vec<_>>();
        let mut seed =
            lm2_test_source_line("p0:l5", 5, "12. Seed region member", 0.82, false, None);
        seed.bottom = 0.30;
        seed.top = 0.32;
        mark_pp_footnote(&mut seed);
        let mut next_page_seed =
            lm2_test_source_line("p1:l0", 0, "13. Next page seed", 0.82, false, None);
        next_page_seed.page_index = 1;
        next_page_seed.bottom = 0.30;
        next_page_seed.top = 0.32;
        mark_pp_footnote(&mut next_page_seed);
        let mut lines = Vec::new();
        lines.append(&mut continuations);
        lines.push(seed);
        lines.push(next_page_seed);
        let mut path = vec![Lm2Action::Keep; lines.len()];

        apply_pp_footnote_region_membership(&lines, &mut path);

        assert_eq!(
            path,
            vec![
                Lm2Action::Keep,
                Lm2Action::Keep,
                Lm2Action::Keep,
                Lm2Action::Keep,
                Lm2Action::Keep,
                Lm2Action::Marginalia,
                Lm2Action::Marginalia,
            ]
        );
    }

    #[test]
    fn pp_footnote_membership_forward_closes_note_like_continuation() {
        let mut seed = lm2_test_source_line(
            "p0:l0",
            0,
            "12. See United States v. Alvarez",
            0.82,
            false,
            None,
        );
        seed.bottom = 0.30;
        seed.top = 0.32;
        mark_pp_footnote(&mut seed);
        let mut continuation = lm2_test_source_line(
            "p0:l1",
            1,
            "continued discussion, see id.",
            0.82,
            false,
            None,
        );
        continuation.bottom = 0.28;
        continuation.top = 0.30;
        let lines = vec![seed, continuation];
        let mut path = vec![Lm2Action::Keep; lines.len()];

        apply_pp_footnote_region_membership(&lines, &mut path);

        assert_eq!(path, vec![Lm2Action::Marginalia, Lm2Action::Marginalia]);
    }

    #[test]
    fn pp_footnote_membership_forward_closure_stops_before_body_text() {
        let mut seed = lm2_test_source_line(
            "p0:l0",
            0,
            "12. See United States v. Alvarez",
            0.82,
            false,
            None,
        );
        seed.bottom = 0.30;
        seed.top = 0.32;
        mark_pp_footnote(&mut seed);
        let mut body = lm2_test_source_line(
            "p0:l1",
            1,
            "The article returns to ordinary body prose.",
            1.0,
            false,
            None,
        );
        body.bottom = 0.62;
        body.top = 0.64;
        let lines = vec![seed, body];
        let mut path = vec![Lm2Action::Keep; lines.len()];

        apply_pp_footnote_region_membership(&lines, &mut path);

        assert_eq!(path, vec![Lm2Action::Marginalia, Lm2Action::Keep]);
    }

    #[test]
    fn pp_footnote_membership_forward_closure_is_bounded_and_same_page() {
        let mut seed = lm2_test_source_line(
            "p0:l0",
            0,
            "12. See United States v. Alvarez",
            0.82,
            false,
            None,
        );
        seed.bottom = 0.30;
        seed.top = 0.32;
        mark_pp_footnote(&mut seed);
        let mut continuations = (1..=17)
            .map(|index| {
                let mut line = lm2_test_source_line(
                    &format!("p0:l{index}"),
                    index,
                    &format!("continuation {index}, see id."),
                    0.82,
                    false,
                    None,
                );
                line.bottom = 0.30 - (index as f32 * 0.01);
                line.top = 0.32 - (index as f32 * 0.01);
                line
            })
            .collect::<Vec<_>>();
        let mut next_page = lm2_test_source_line(
            "p1:l0",
            0,
            "next-page continuation, see id.",
            0.82,
            false,
            None,
        );
        next_page.page_index = 1;
        next_page.bottom = 0.24;
        next_page.top = 0.26;
        let mut lines = vec![seed];
        lines.append(&mut continuations);
        lines.push(next_page);
        let mut path = vec![Lm2Action::Keep; lines.len()];

        apply_pp_footnote_region_membership(&lines, &mut path);

        assert_eq!(
            path,
            (0..lines.len())
                .map(|index| {
                    if index <= 16 {
                        Lm2Action::Marginalia
                    } else {
                        Lm2Action::Keep
                    }
                })
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn pp_footnote_membership_preserves_marker_backed_start() {
        let mut line = lm2_test_source_line(
            "p0:l0",
            0,
            "12. See United States v. Alvarez",
            0.88,
            false,
            None,
        );
        line.bottom = 0.30;
        line.top = 0.32;
        line.font_ratio_doc = 0.88;
        mark_pp_footnote(&mut line);
        let lines = vec![line];
        let mut path = vec![Lm2Action::Keep];

        apply_pp_footnote_region_membership(&lines, &mut path);

        assert_eq!(path, vec![Lm2Action::Marginalia]);
    }

    #[test]
    fn pp_footnote_membership_preserves_in_run_tail_continuation() {
        let first = lm2_test_source_line("p0:l0", 0, "1. Existing note start", 0.88, false, None);
        let mut tail = lm2_test_source_line(
            "p0:l1",
            1,
            "for life.”); see also 50-State Comparison",
            0.88,
            false,
            None,
        );
        tail.bottom = 0.30;
        tail.top = 0.32;
        tail.font_ratio_doc = 0.88;
        mark_pp_footnote(&mut tail);
        let lines = vec![first, tail];
        let mut path = vec![Lm2Action::Marginalia, Lm2Action::Keep];

        apply_pp_footnote_region_membership(&lines, &mut path);

        assert_eq!(path, vec![Lm2Action::Marginalia, Lm2Action::Marginalia]);
    }

    #[test]
    fn lm2_recovers_title_from_hidden_leading_source_lines() {
        let decoded = vec![
            (
                lm2_test_source_line(
                    "p0:l0",
                    0,
                    "SPERBER IN PRINTER PREP (DO NOT DELETE) 3/17/2026 10:46 AM",
                    0.61,
                    false,
                    Some(LiquidBlockRole::Noise),
                ),
                Lm2Action::HideNoise,
            ),
            (
                lm2_test_source_line(
                    "p0:l1",
                    1,
                    "TESTING DOBBS'S DEMOCRACY PREMISE: CAN",
                    1.36,
                    true,
                    None,
                ),
                Lm2Action::HideNoise,
            ),
            (
                lm2_test_source_line(
                    "p0:l2",
                    2,
                    "STATE CONSTITUTIONS BE AMENDED TO",
                    1.40,
                    true,
                    None,
                ),
                Lm2Action::HideNoise,
            ),
            (
                lm2_test_source_line(
                    "p0:l3",
                    3,
                    "REFLECT POPULAR OPINION ON ABORTION?",
                    1.40,
                    true,
                    Some(LiquidBlockRole::Paragraph),
                ),
                Lm2Action::Keep,
            ),
            (
                lm2_test_source_line("p0:l4", 4, "ISABEL SPERBER†", 0.94, true, None),
                Lm2Action::HideNoise,
            ),
            (
                lm2_test_source_line("p0:l5", 5, "ABSTRACT", 0.92, true, None),
                Lm2Action::HideNoise,
            ),
            (
                lm2_test_source_line(
                    "p0:l6",
                    6,
                    "When the Supreme Court eliminated a federal constitutional right to abortion",
                    1.0,
                    false,
                    Some(LiquidBlockRole::Paragraph),
                ),
                Lm2Action::Keep,
            ),
        ];

        let (title, blocks, sources) = build_lm2_blocks("duke_law_journal.pdf", &decoded);

        assert_eq!(
            title,
            "TESTING DOBBS'S DEMOCRACY PREMISE: CAN STATE CONSTITUTIONS BE AMENDED TO REFLECT POPULAR OPINION ON ABORTION?"
        );
        assert_eq!(blocks[0].role, LiquidBlockRole::Title);
        assert_eq!(blocks[0].text, "TESTING DOBBS'S DEMOCRACY PREMISE: CAN");
        assert_eq!(blocks[1].role, LiquidBlockRole::Heading);
        assert_eq!(blocks[2].role, LiquidBlockRole::Heading);
        assert!(blocks.iter().all(|block| block.text != "ISABEL SPERBER†"));
        assert_eq!(
            blocks
                .iter()
                .filter(|block| block.role == LiquidBlockRole::Title)
                .count(),
            1
        );
        assert_eq!(
            sources[0].lines[0].text,
            "TESTING DOBBS'S DEMOCRACY PREMISE: CAN"
        );
    }

    #[test]
    fn lm2_source_title_recovery_does_not_duplicate_visible_title() {
        let decoded = vec![
            (
                lm2_test_source_line(
                    "p0:l1",
                    1,
                    "TESTING DOBBS'S DEMOCRACY PREMISE: CAN",
                    1.36,
                    true,
                    None,
                ),
                Lm2Action::Keep,
            ),
            (
                lm2_test_source_line(
                    "p0:l2",
                    2,
                    "STATE CONSTITUTIONS BE AMENDED TO",
                    1.40,
                    true,
                    None,
                ),
                Lm2Action::Keep,
            ),
            (
                lm2_test_source_line(
                    "p0:l3",
                    3,
                    "REFLECT POPULAR OPINION ON ABORTION?",
                    1.40,
                    true,
                    Some(LiquidBlockRole::Paragraph),
                ),
                Lm2Action::Keep,
            ),
        ];

        let (_, blocks, _) = build_lm2_blocks("", &decoded);

        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0].text, "TESTING DOBBS'S DEMOCRACY PREMISE: CAN");
        assert_eq!(blocks[1].text, "STATE CONSTITUTIONS BE AMENDED TO");
        assert_eq!(blocks[2].text, "REFLECT POPULAR OPINION ON ABORTION?");
    }

    #[test]
    fn pymupdf_grouping_overrides_paragraph_boundary_for_assigned_lines() {
        let decoded = vec![
            (
                lm2_test_source_line("p0:l0", 0, "First paragraph line.", 1.0, false, None),
                Lm2Action::Keep,
            ),
            (
                lm2_test_source_line("p0:l1", 1, "Second line in same box.", 1.0, false, None),
                Lm2Action::Keep,
            ),
            (
                lm2_test_source_line("p0:l2", 2, "Next box starts here.", 1.0, false, None),
                Lm2Action::Keep,
            ),
        ];
        let grouping = Lm2PymupdfGroupingResponse {
            mode: Some("pymupdf".to_owned()),
            warnings: Vec::new(),
            blocks: vec![
                Lm2PymupdfGroupingBlock {
                    block_index: Some(0),
                    page_index: Some(0),
                    source: Some("pymupdf".to_owned()),
                    source_line_ids: vec!["p0:l0".to_owned(), "p0:l1".to_owned()],
                },
                Lm2PymupdfGroupingBlock {
                    block_index: Some(1),
                    page_index: Some(0),
                    source: Some("pymupdf".to_owned()),
                    source_line_ids: vec!["p0:l2".to_owned()],
                },
            ],
        };

        let (_, blocks, sources) = build_lm2_blocks_with_grouping("", &decoded, Some(&grouping));
        assert_eq!(blocks.len(), 2);
        assert_eq!(
            blocks[0].text,
            "First paragraph line. Second line in same box."
        );
        assert_eq!(blocks[1].text, "Next box starts here.");
        assert_eq!(
            sources[0]
                .lines
                .iter()
                .filter_map(|line| line.id.clone())
                .collect::<Vec<_>>(),
            vec!["p0:l0".to_owned(), "p0:l1".to_owned()]
        );
        assert_eq!(
            sources[1]
                .lines
                .iter()
                .filter_map(|line| line.id.clone())
                .collect::<Vec<_>>(),
            vec!["p0:l2".to_owned()]
        );
    }

    #[test]
    fn paragraph_continues_across_source_page_without_indent() {
        let previous = lm2_test_source_line(
            "p0:l9",
            9,
            "The rule continues across the source page.",
            1.0,
            false,
            Some(LiquidBlockRole::Paragraph),
        );
        let mut next = lm2_test_source_line(
            "p1:l0",
            0,
            "Its application remains contested.",
            1.0,
            false,
            Some(LiquidBlockRole::Paragraph),
        );
        next.page_index = 1;
        assert!(!paragraph_boundary(&previous, &next));
    }

    #[test]
    fn clear_indent_can_start_approximate_paragraph_across_page() {
        let previous = lm2_test_source_line(
            "p0:l9",
            9,
            "The first paragraph ends here.",
            1.0,
            false,
            Some(LiquidBlockRole::Paragraph),
        );
        let mut next = lm2_test_source_line(
            "p1:l0",
            0,
            "A new paragraph begins here.",
            1.0,
            false,
            Some(LiquidBlockRole::Paragraph),
        );
        next.page_index = 1;
        next.left = previous.left + 0.04;
        assert!(paragraph_boundary(&previous, &next));
    }

    #[test]
    fn d1_runtime_zerospend_overlay_recovers_small_font_citation_keep_line() {
        let mut line = lm2_test_source_line(
            "p0:l18",
            18,
            "45. Koons v. Platkin, 673 F. Supp. 3d 515, 620 (D.N.J. 2023).",
            0.86,
            false,
            None,
        );
        line.font_ratio_doc = 0.86;
        line.font_ratio_page_ref = 0.86;
        let mut decoded = vec![(line, Lm2Action::Keep)];

        apply_d1_runtime_zerospend_overlay(&mut decoded);

        assert_eq!(decoded[0].1, Lm2Action::Marginalia);
        assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Footnote));
    }

    #[test]
    fn d1_runtime_zerospend_overlay_preserves_uncued_body_keep_line() {
        let mut line = lm2_test_source_line(
            "p0:l18",
            18,
            "Freedom]. In Presuming Trustworthiness, we investigated the Justices abandonment.",
            0.86,
            false,
            None,
        );
        line.font_ratio_doc = 0.86;
        line.font_ratio_page_ref = 0.86;
        let mut decoded = vec![(line, Lm2Action::Keep)];

        apply_d1_runtime_zerospend_overlay(&mut decoded);

        assert_eq!(decoded[0].1, Lm2Action::Keep);
        assert_eq!(decoded[0].0.role_hint, None);
    }

    #[test]
    fn d1_runtime_zerospend_overlay_requires_lower_page_region() {
        let mut line = lm2_test_source_line(
            "p0:l4",
            4,
            "See Brown v. Board of Education, 347 U.S. 483 (1954).",
            0.86,
            false,
            None,
        );
        line.font_ratio_doc = 0.86;
        line.font_ratio_page_ref = 0.86;
        let mut decoded = vec![(line, Lm2Action::Keep)];

        apply_d1_runtime_zerospend_overlay(&mut decoded);

        assert_eq!(decoded[0].1, Lm2Action::Keep);
    }

    #[test]
    fn d1_runtime_continuation_overlay_recovers_after_marginalia_anchor() {
        let anchor = lm2_test_source_line(
            "p0:l12",
            12,
            "1. Detention FY 2025 YTD, U.S. Immigr. & Customs Enf't",
            0.86,
            false,
            Some(LiquidBlockRole::Footnote),
        );
        let mut continuation = lm2_test_source_line(
            "p0:l13",
            13,
            "Immigr. & Customs Enf't (2025), https://www.ice.gov/doclib/detention/FY25.xlsx",
            0.86,
            false,
            None,
        );
        continuation.bottom = 0.30;
        continuation.font_ratio_doc = 0.86;
        let mut decoded = vec![
            (anchor, Lm2Action::Marginalia),
            (continuation, Lm2Action::Keep),
        ];

        apply_d1_runtime_continuation_overlay(&mut decoded);

        assert_eq!(decoded[1].1, Lm2Action::Marginalia);
        assert_eq!(decoded[1].0.role_hint, Some(LiquidBlockRole::Footnote));
    }

    #[test]
    fn d1_runtime_continuation_overlay_stops_before_uncued_body_prose() {
        let anchor = lm2_test_source_line(
            "p0:l12",
            12,
            "1. Detention FY 2025 YTD, U.S. Immigr. & Customs Enf't",
            0.86,
            false,
            Some(LiquidBlockRole::Footnote),
        );
        let mut body = lm2_test_source_line(
            "p0:l13",
            13,
            "The next section turns from detention statistics to doctrine.",
            0.86,
            false,
            None,
        );
        body.bottom = 0.30;
        body.font_ratio_doc = 0.86;
        let mut decoded = vec![(anchor, Lm2Action::Marginalia), (body, Lm2Action::Keep)];

        apply_d1_runtime_continuation_overlay(&mut decoded);

        assert_eq!(decoded[1].1, Lm2Action::Keep);
        assert_eq!(decoded[1].0.role_hint, None);
    }

    #[test]
    fn d1_runtime_immediate_continuation_overlay_recovers_sandwiched_small_font_line() {
        let previous = lm2_test_source_line(
            "p0:l12",
            12,
            "10. Smith v. Jones, 123 F.3d 456, 460 (9th Cir. 2020).",
            0.86,
            false,
            Some(LiquidBlockRole::Footnote),
        );
        let mut continuation = lm2_test_source_line(
            "p0:l13",
            13,
            "explaining the same doctrine in later agency guidance.",
            0.86,
            false,
            None,
        );
        continuation.font_ratio_doc = 0.86;
        let next = lm2_test_source_line(
            "p0:l14",
            14,
            "11. Accord Johnson v. State, 52 U.S. 99 (2021).",
            0.86,
            false,
            Some(LiquidBlockRole::Footnote),
        );
        let mut decoded = vec![
            (previous, Lm2Action::Marginalia),
            (continuation, Lm2Action::Keep),
            (next, Lm2Action::Marginalia),
        ];

        apply_d1_runtime_immediate_continuation_overlay(&mut decoded);

        assert_eq!(decoded[1].1, Lm2Action::Marginalia);
        assert_eq!(decoded[1].0.role_hint, Some(LiquidBlockRole::Footnote));
    }

    #[test]
    fn d1_runtime_immediate_continuation_overlay_requires_next_marginalia() {
        let previous = lm2_test_source_line(
            "p0:l12",
            12,
            "10. Smith v. Jones, 123 F.3d 456, 460 (9th Cir. 2020).",
            0.86,
            false,
            Some(LiquidBlockRole::Footnote),
        );
        let mut candidate = lm2_test_source_line(
            "p0:l13",
            13,
            "The next section turns from detention statistics to doctrine.",
            0.86,
            false,
            None,
        );
        candidate.font_ratio_doc = 0.86;
        let next = lm2_test_source_line("p0:l14", 14, "II. Doctrine", 1.0, false, None);
        let mut decoded = vec![
            (previous, Lm2Action::Marginalia),
            (candidate, Lm2Action::Keep),
            (next, Lm2Action::Keep),
        ];

        apply_d1_runtime_immediate_continuation_overlay(&mut decoded);

        assert_eq!(decoded[1].1, Lm2Action::Keep);
        assert_eq!(decoded[1].0.role_hint, None);
    }

    #[test]
    fn d1_runtime_immediate_continuation_overlay_does_not_cross_pages() {
        let mut previous = lm2_test_source_line(
            "p0:l12",
            12,
            "10. Smith v. Jones, 123 F.3d 456, 460 (9th Cir. 2020).",
            0.86,
            false,
            Some(LiquidBlockRole::Footnote),
        );
        previous.page_index = 0;
        let mut candidate = lm2_test_source_line(
            "p1:l0",
            0,
            "explaining the same doctrine in later agency guidance.",
            0.86,
            false,
            None,
        );
        candidate.page_index = 1;
        candidate.font_ratio_doc = 0.86;
        let mut next = lm2_test_source_line(
            "p1:l1",
            1,
            "11. Accord Johnson v. State, 52 U.S. 99 (2021).",
            0.86,
            false,
            Some(LiquidBlockRole::Footnote),
        );
        next.page_index = 1;
        let mut decoded = vec![
            (previous, Lm2Action::Marginalia),
            (candidate, Lm2Action::Keep),
            (next, Lm2Action::Marginalia),
        ];

        apply_d1_runtime_immediate_continuation_overlay(&mut decoded);

        assert_eq!(decoded[1].1, Lm2Action::Keep);
        assert_eq!(decoded[1].0.role_hint, None);
    }

    #[test]
    fn d1_runtime_sandwiched_continuation_overlay_accepts_neighbor_with_one_line_gap() {
        let previous = lm2_test_source_line(
            "p0:l12",
            12,
            "10. Smith v. Jones, 123 F.3d 456, 460 (9th Cir. 2020).",
            0.86,
            false,
            Some(LiquidBlockRole::Footnote),
        );
        let body_gap =
            lm2_test_source_line("p0:l13", 13, "intervening body line", 1.0, false, None);
        let mut continuation = lm2_test_source_line(
            "p0:l14",
            14,
            "explaining the same doctrine in later agency guidance.",
            0.86,
            false,
            None,
        );
        continuation.font_ratio_doc = 0.86;
        let next = lm2_test_source_line(
            "p0:l15",
            15,
            "11. Accord Johnson v. State, 52 U.S. 99 (2021).",
            0.86,
            false,
            Some(LiquidBlockRole::Footnote),
        );
        let mut decoded = vec![
            (previous, Lm2Action::Marginalia),
            (body_gap, Lm2Action::Keep),
            (continuation, Lm2Action::Keep),
            (next, Lm2Action::Marginalia),
        ];

        apply_d1_runtime_sandwiched_continuation_overlay(&mut decoded);

        assert_eq!(decoded[2].1, Lm2Action::Marginalia);
        assert_eq!(decoded[2].0.role_hint, Some(LiquidBlockRole::Footnote));
    }

    #[test]
    fn d1_runtime_sandwiched_continuation_overlay_does_not_cross_pages() {
        let mut previous = lm2_test_source_line(
            "p0:l12",
            12,
            "10. Smith v. Jones, 123 F.3d 456, 460 (9th Cir. 2020).",
            0.86,
            false,
            Some(LiquidBlockRole::Footnote),
        );
        previous.page_index = 0;
        let mut candidate = lm2_test_source_line(
            "p1:l0",
            0,
            "explaining the same doctrine in later agency guidance.",
            0.86,
            false,
            None,
        );
        candidate.page_index = 1;
        candidate.font_ratio_doc = 0.86;
        let mut next = lm2_test_source_line(
            "p1:l1",
            1,
            "11. Accord Johnson v. State, 52 U.S. 99 (2021).",
            0.86,
            false,
            Some(LiquidBlockRole::Footnote),
        );
        next.page_index = 1;
        let mut decoded = vec![
            (previous, Lm2Action::Marginalia),
            (candidate, Lm2Action::Keep),
            (next, Lm2Action::Marginalia),
        ];

        apply_d1_runtime_sandwiched_continuation_overlay(&mut decoded);

        assert_eq!(decoded[1].1, Lm2Action::Keep);
        assert_eq!(decoded[1].0.role_hint, None);
    }

    #[test]
    fn d1_runtime_wide_sandwich_overlay_accepts_font095_sandwich() {
        let previous = lm2_test_source_line(
            "p0:l12",
            12,
            "10. Smith v. Jones, 123 F.3d 456, 460 (9th Cir. 2020).",
            0.86,
            false,
            Some(LiquidBlockRole::Footnote),
        );
        let mut continuation = lm2_test_source_line(
            "p0:l13",
            13,
            "explaining the same doctrine in later agency guidance.",
            0.95,
            false,
            None,
        );
        continuation.font_ratio_doc = 0.95;
        let next = lm2_test_source_line(
            "p0:l14",
            14,
            "11. Accord Johnson v. State, 52 U.S. 99 (2021).",
            0.86,
            false,
            Some(LiquidBlockRole::Footnote),
        );
        let mut decoded = vec![
            (previous, Lm2Action::Marginalia),
            (continuation, Lm2Action::Keep),
            (next, Lm2Action::Marginalia),
        ];

        apply_d1_runtime_wide_sandwich_overlay(&mut decoded);

        assert_eq!(decoded[1].1, Lm2Action::Marginalia);
        assert_eq!(decoded[1].0.role_hint, Some(LiquidBlockRole::Footnote));
    }

    #[test]
    fn d1_runtime_wide_sandwich_overlay_requires_next_marginalia() {
        let previous = lm2_test_source_line(
            "p0:l12",
            12,
            "10. Smith v. Jones, 123 F.3d 456, 460 (9th Cir. 2020).",
            0.86,
            false,
            Some(LiquidBlockRole::Footnote),
        );
        let mut candidate = lm2_test_source_line(
            "p0:l13",
            13,
            "The next section turns from detention statistics to doctrine.",
            0.95,
            false,
            None,
        );
        candidate.font_ratio_doc = 0.95;
        let next = lm2_test_source_line("p0:l14", 14, "II. Doctrine", 1.0, false, None);
        let mut decoded = vec![
            (previous, Lm2Action::Marginalia),
            (candidate, Lm2Action::Keep),
            (next, Lm2Action::Keep),
        ];

        apply_d1_runtime_wide_sandwich_overlay(&mut decoded);

        assert_eq!(decoded[1].1, Lm2Action::Keep);
        assert_eq!(decoded[1].0.role_hint, None);
    }

    #[test]
    fn d1_runtime_post_wide_cue_overlay_accepts_forward_cued_line() {
        let candidate = lm2_test_source_line(
            "p0:l20",
            20,
            "12. See the same source for this proposition.",
            0.88,
            false,
            None,
        );
        let next_keep = lm2_test_source_line(
            "p0:l21",
            21,
            "intermediate carried body line",
            1.0,
            false,
            None,
        );
        let next_marginalia = lm2_test_source_line(
            "p0:l22",
            22,
            "continuing the footnote discussion.",
            0.86,
            false,
            Some(LiquidBlockRole::Footnote),
        );
        let next_marginalia_two = lm2_test_source_line(
            "p0:l23",
            23,
            "more authority for the same point.",
            0.86,
            false,
            Some(LiquidBlockRole::Footnote),
        );
        let mut decoded = vec![
            (candidate, Lm2Action::Keep),
            (next_keep, Lm2Action::Keep),
            (next_marginalia, Lm2Action::Marginalia),
            (next_marginalia_two, Lm2Action::Marginalia),
        ];

        apply_d1_runtime_post_wide_cue_overlay(&mut decoded);

        assert_eq!(decoded[0].1, Lm2Action::Marginalia);
        assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Footnote));
    }

    #[test]
    fn d1_runtime_post_wide_cue_overlay_rejects_uncued_body_line() {
        let candidate = lm2_test_source_line(
            "p0:l20",
            20,
            "This Part next explains why the doctrine developed slowly.",
            0.88,
            false,
            None,
        );
        let next_marginalia = lm2_test_source_line(
            "p0:l21",
            21,
            "continuing the footnote discussion.",
            0.86,
            false,
            Some(LiquidBlockRole::Footnote),
        );
        let next_marginalia_two = lm2_test_source_line(
            "p0:l22",
            22,
            "more authority for the same point.",
            0.86,
            false,
            Some(LiquidBlockRole::Footnote),
        );
        let mut decoded = vec![
            (candidate, Lm2Action::Keep),
            (next_marginalia, Lm2Action::Marginalia),
            (next_marginalia_two, Lm2Action::Marginalia),
        ];

        apply_d1_runtime_post_wide_cue_overlay(&mut decoded);

        assert_eq!(decoded[0].1, Lm2Action::Keep);
        assert_eq!(decoded[0].0.role_hint, None);
    }

    #[test]
    fn d1_runtime_postcue_citation_next1_overlay_accepts_citation_before_marginalia() {
        let candidate = lm2_test_source_line(
            "p0:l20",
            20,
            "See https://perma.cc/ABCD-EFGH for the archived source.",
            0.95,
            false,
            None,
        );
        let next_marginalia = lm2_test_source_line(
            "p0:l21",
            21,
            "continuing the same footnote.",
            0.86,
            false,
            Some(LiquidBlockRole::Footnote),
        );
        let mut decoded = vec![
            (candidate, Lm2Action::Keep),
            (next_marginalia, Lm2Action::Marginalia),
        ];

        apply_d1_runtime_postcue_citation_next1_overlay(&mut decoded);

        assert_eq!(decoded[0].1, Lm2Action::Marginalia);
        assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Footnote));
    }

    #[test]
    fn d1_runtime_postcue_citation_next1_overlay_requires_next_marginalia() {
        let candidate = lm2_test_source_line(
            "p0:l20",
            20,
            "See https://perma.cc/ABCD-EFGH for the archived source.",
            0.95,
            false,
            None,
        );
        let next_keep = lm2_test_source_line(
            "p0:l21",
            21,
            "The Article then returns to ordinary body prose.",
            1.0,
            false,
            None,
        );
        let mut decoded = vec![(candidate, Lm2Action::Keep), (next_keep, Lm2Action::Keep)];

        apply_d1_runtime_postcue_citation_next1_overlay(&mut decoded);

        assert_eq!(decoded[0].1, Lm2Action::Keep);
        assert_eq!(decoded[0].0.role_hint, None);
    }

    #[test]
    fn d1_runtime_postcue_citation_next1_overlay_rejects_bare_ibid_index_line() {
        let candidate = lm2_test_source_line(
            "p0:l20",
            20,
            "child, and subsequently, were inadmissible to prove it illegitimate. Ibid.",
            0.87,
            false,
            None,
        );
        let next_marginalia = lm2_test_source_line(
            "p0:l21",
            21,
            "1. In an inheritance case, where the claimant was begotten before",
            0.87,
            false,
            Some(LiquidBlockRole::Footnote),
        );
        let mut decoded = vec![
            (candidate, Lm2Action::Keep),
            (next_marginalia, Lm2Action::Marginalia),
        ];

        apply_d1_runtime_postcue_citation_next1_overlay(&mut decoded);

        assert_eq!(decoded[0].1, Lm2Action::Keep);
        assert_eq!(decoded[0].0.role_hint, None);
    }

    #[test]
    fn d1_runtime_near8_cue_overlay_accepts_cued_line_near_four_marginalia() {
        let candidate = lm2_test_source_line(
            "p0:l20",
            20,
            "Analysis, 399 THE LANCET 629, 639 (2022), describing the global burden of bacterial resistance.",
            0.84,
            false,
            None,
        );
        let m1 = lm2_test_source_line("p0:l21", 21, "continuing footnote one", 0.84, false, None);
        let m2 = lm2_test_source_line("p0:l22", 22, "continuing footnote two", 0.84, false, None);
        let m3 = lm2_test_source_line("p0:l23", 23, "continuing footnote three", 0.84, false, None);
        let m4 = lm2_test_source_line("p0:l24", 24, "continuing footnote four", 0.84, false, None);
        let mut decoded = vec![
            (candidate, Lm2Action::Keep),
            (m1, Lm2Action::Marginalia),
            (m2, Lm2Action::Marginalia),
            (m3, Lm2Action::Marginalia),
            (m4, Lm2Action::Marginalia),
        ];

        apply_d1_runtime_near8_cue_overlay(&mut decoded);

        assert_eq!(decoded[0].1, Lm2Action::Marginalia);
        assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Footnote));
    }

    #[test]
    fn d1_runtime_near8_cue_overlay_requires_four_nearby_marginalia() {
        let candidate = lm2_test_source_line(
            "p0:l20",
            20,
            "Analysis, 399 THE LANCET 629, 639 (2022), describing the global burden of bacterial resistance.",
            0.84,
            false,
            None,
        );
        let m1 = lm2_test_source_line("p0:l21", 21, "continuing footnote one", 0.84, false, None);
        let m2 = lm2_test_source_line("p0:l22", 22, "continuing footnote two", 0.84, false, None);
        let m3 = lm2_test_source_line("p0:l23", 23, "continuing footnote three", 0.84, false, None);
        let mut decoded = vec![
            (candidate, Lm2Action::Keep),
            (m1, Lm2Action::Marginalia),
            (m2, Lm2Action::Marginalia),
            (m3, Lm2Action::Marginalia),
        ];

        apply_d1_runtime_near8_cue_overlay(&mut decoded);

        assert_eq!(decoded[0].1, Lm2Action::Keep);
        assert_eq!(decoded[0].0.role_hint, None);
    }

    #[test]
    fn axis_numeric_token_handles_multibyte_currency_prefix() {
        assert!(lm2_axis_numeric_token("£1,200"));
        assert!(lm2_axis_numeric_token("€99.50"));
        assert!(lm2_axis_numeric_token("($1,200)"));
    }

    #[test]
    fn table_figure_router_env_opt_out_accepts_common_falsey_values() {
        for value in ["0", "false", "FALSE", " no ", "off"] {
            assert!(falsey_env_value(value));
        }
        for value in ["", "1", "true", "yes", "default"] {
            assert!(!falsey_env_value(value));
        }
    }

    #[test]
    fn page_object_tuned_preset_alias_is_explicit() {
        assert!(lm2_runtime_preset_is_page_object_tuned(
            LM2_V25_D1_PAGE_OBJECT_TUNED_PRESET
        ));
        assert!(lm2_runtime_preset_is_page_object_tuned(
            "V25-D1-SANDWICHED-NOTE-START-WIDE-SANDWICH-POSTCUE-CITATION-NEXT1-NEAR8CUE-WIDE-DIVIDER-GUARD-PAGE-OBJECT-TUNED"
        ));
        assert!(!lm2_runtime_preset_is_page_object_tuned(
            "v25-d1-sandwiched-note-start-wide-sandwich-postcue-citation-next1-near8cue-wide-divider-guard"
        ));
    }

    #[test]
    fn table_figure_router_routes_numeric_grid_row_to_hide_noise() {
        let mut candidate = lm2_test_source_line(
            "p0:l40",
            40,
            "Nassau 21 97 21.6% 31 124 25.0% 31 121 25.6%",
            1.0,
            false,
            None,
        );
        candidate.page_width = 1.0;
        candidate.left = 0.1;
        candidate.right = 0.45; // width_norm 0.35 < 0.6
        let mut decoded = vec![(candidate, Lm2Action::Keep)];

        apply_table_figure_router_overlay(&mut decoded);

        assert_eq!(decoded[0].1, Lm2Action::HideNoise);
        // Tagged Table (not Noise) so a future display-tables toggle can resurface it.
        assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Table));
    }

    #[test]
    fn lm2_assembler_preserves_router_table_lines_as_table_blocks() {
        let mut candidate = lm2_test_source_line(
            "p0:l40",
            40,
            "Nassau 21 97 21.6% 31 124 25.0% 31 121 25.6%",
            1.0,
            false,
            Some(LiquidBlockRole::Table),
        );
        candidate.page_width = 1.0;
        candidate.left = 0.1;
        candidate.right = 0.45;
        let decoded = vec![(candidate, Lm2Action::HideNoise)];

        let (_title, blocks, sources) = build_lm2_blocks("Fallback", &decoded);

        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].role, LiquidBlockRole::Table);
        assert_eq!(blocks[0].label.as_deref(), Some("Table/Figure"));
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].lines[0].role, LiquidBlockRole::Table);
    }

    #[test]
    fn lm2_assembler_drops_router_header_furniture() {
        let mut candidate = lm2_test_source_line(
            "p3:l1",
            1,
            "68 UCLA L. REV. DISC. (LAW MEETS WORLD) 22 (2020)",
            1.0,
            false,
            Some(LiquidBlockRole::Header),
        );
        candidate.doc_repeated_text_count = 5;
        let decoded = vec![(candidate, Lm2Action::HideNoise)];

        let (_title, blocks, sources) = build_lm2_blocks("Fallback", &decoded);

        assert!(blocks.is_empty());
        assert!(sources.is_empty());
    }

    #[test]
    fn table_figure_router_leaves_numeric_citation_prose_as_keep() {
        let mut candidate = lm2_test_source_line(
            "p0:l41",
            41,
            "Law, 7 J. Legal Stud. 393 (1979).",
            1.0,
            false,
            None,
        );
        candidate.page_width = 1.0;
        candidate.left = 0.1;
        candidate.right = 0.45;
        let mut decoded = vec![(candidate, Lm2Action::Keep)];

        apply_table_figure_router_overlay(&mut decoded);

        assert_eq!(decoded[0].1, Lm2Action::Keep);
        assert_eq!(decoded[0].0.role_hint, None);
    }

    #[test]
    fn table_figure_router_routes_repeated_running_header_to_hide_noise() {
        let mut candidate = lm2_test_source_line(
            "p3:l1",
            1,
            "68 UCLA L. REV. DISC. (LAW MEETS WORLD) 22 (2020)",
            1.0,
            false,
            None,
        );
        candidate.doc_repeated_text_count = 5;
        let mut decoded = vec![(candidate, Lm2Action::Keep)];

        apply_table_figure_router_overlay(&mut decoded);

        assert_eq!(decoded[0].1, Lm2Action::HideNoise);
        // Running-header furniture is tagged Header (not resurfaced by the toggle).
        assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Header));
    }

    #[test]
    fn table_figure_router_keeps_repeated_body_sentence() {
        let mut candidate = lm2_test_source_line(
            "p3:l2",
            2,
            "The court held that the statute was unconstitutional on its face.",
            1.0,
            false,
            None,
        );
        candidate.doc_repeated_text_count = 4;
        candidate.page_width = 1.0;
        candidate.left = 0.1;
        candidate.right = 0.9;
        let mut decoded = vec![(candidate, Lm2Action::Keep)];

        apply_table_figure_router_overlay(&mut decoded);

        assert_eq!(decoded[0].1, Lm2Action::Keep);
    }

    #[test]
    fn page_object_tuned_overlay_hides_ruled_keep_and_marginalia_rows() {
        let mut keep = lm2_test_source_line("p0:l1", 1, "County 2020 2021 2022", 1.0, false, None);
        keep.page_width = 600.0;
        keep.page_height = 800.0;
        keep.left = 80.0;
        keep.right = 520.0;
        keep.bottom = 500.0;
        keep.top = 512.0;
        keep.page_object_ruled_row_membership = true;
        let mut marginalia = keep.clone();
        marginalia.id = "p0:l2".to_owned();
        marginalia.line_index = 2;
        let mut decoded = vec![(keep, Lm2Action::Keep), (marginalia, Lm2Action::Marginalia)];

        apply_page_object_tuned_overlay(&mut decoded);

        assert_eq!(decoded[0].1, Lm2Action::HideNoise);
        assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Table));
        assert_eq!(decoded[1].1, Lm2Action::HideNoise);
        assert_eq!(decoded[1].0.role_hint, Some(LiquidBlockRole::Table));
    }

    #[test]
    fn page_object_tuned_overlay_preserves_legal_ruled_keep_rows() {
        let mut row = lm2_test_source_line(
            "p0:l10",
            10,
            "Montana Court Channeling Mont. Code Ann. § 46-21-",
            1.0,
            false,
            None,
        );
        row.page_width = 600.0;
        row.page_height = 800.0;
        row.left = 120.0;
        row.right = 500.0;
        row.bottom = 500.0;
        row.top = 512.0;
        row.page_object_ruled_row_membership = true;
        let mut decoded = vec![(row, Lm2Action::Keep)];

        apply_page_object_tuned_overlay(&mut decoded);

        assert_eq!(decoded[0].1, Lm2Action::Keep);
        assert_eq!(decoded[0].0.role_hint, None);
    }

    #[test]
    fn page_object_tuned_overlay_preserves_prose_like_ruled_keep_rows() {
        let mut row = lm2_test_source_line(
            "p0:l11",
            11,
            "The court concluded that the statute remained available after conviction.",
            1.0,
            false,
            None,
        );
        row.page_width = 600.0;
        row.page_height = 800.0;
        row.left = 80.0;
        row.right = 520.0;
        row.bottom = 500.0;
        row.top = 512.0;
        row.page_object_ruled_row_membership = true;
        let mut decoded = vec![(row, Lm2Action::Keep)];

        apply_page_object_tuned_overlay(&mut decoded);

        assert_eq!(decoded[0].1, Lm2Action::Keep);
        assert_eq!(decoded[0].0.role_hint, None);
    }

    #[test]
    fn page_object_tuned_overlay_rescues_body_like_nonedge_rows() {
        let mut body = lm2_test_source_line(
            "p0:l5",
            5,
            "The court held that the statutory claim remains available to plaintiffs.",
            0.90,
            true,
            None,
        );
        body.page_width = 600.0;
        body.page_height = 800.0;
        body.left = 70.0;
        body.right = 520.0;
        body.bottom = 420.0;
        body.top = 432.0;
        let mut decoded = vec![(body, Lm2Action::HideNoise)];

        apply_page_object_tuned_overlay(&mut decoded);

        assert_eq!(decoded[0].1, Lm2Action::Keep);
        assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Paragraph));
    }

    #[test]
    fn page_object_tuned_overlay_does_not_rescue_footnote_zone_rows() {
        let mut note = lm2_test_source_line(
            "p0:l30",
            30,
            "The court held that the statutory claim remains available to plaintiffs.",
            0.90,
            true,
            None,
        );
        note.page_width = 600.0;
        note.page_height = 800.0;
        note.left = 70.0;
        note.right = 520.0;
        note.bottom = 420.0;
        note.top = 432.0;
        note.in_footnote_zone = true;
        let mut decoded = vec![(note, Lm2Action::Marginalia)];

        apply_page_object_tuned_overlay(&mut decoded);

        assert_eq!(decoded[0].1, Lm2Action::Marginalia);
        assert_eq!(decoded[0].0.role_hint, None);
    }

    #[test]
    fn d1_runtime_wide_divider_guard_overlay_accepts_small_lower_short_line_below_divider() {
        let mut candidate = lm2_test_source_line(
            "p0:l30",
            30,
            "federal courts applying the same equitable doctrine.",
            0.88,
            false,
            None,
        );
        candidate.page_index = 2;
        candidate.below_footnote_divider = true;
        candidate.font_ratio_page_ref = 0.88;
        candidate.top = 400.0;
        candidate.bottom = 388.0;
        candidate.page_height = 1000.0;
        let mut decoded = vec![(candidate, Lm2Action::Keep)];

        apply_d1_runtime_wide_divider_guard_overlay(&mut decoded);

        assert_eq!(decoded[0].1, Lm2Action::Marginalia);
        assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Footnote));
    }

    #[test]
    fn d1_runtime_wide_divider_guard_overlay_requires_below_divider() {
        let mut candidate = lm2_test_source_line(
            "p0:l30",
            30,
            "federal courts applying the same equitable doctrine.",
            0.88,
            false,
            None,
        );
        candidate.font_ratio_page_ref = 0.88;
        candidate.top = 400.0;
        candidate.bottom = 388.0;
        candidate.page_height = 1000.0;
        let mut decoded = vec![(candidate, Lm2Action::Keep)];

        apply_d1_runtime_wide_divider_guard_overlay(&mut decoded);

        assert_eq!(decoded[0].1, Lm2Action::Keep);
        assert_eq!(decoded[0].0.role_hint, None);
    }

    #[test]
    fn d1_runtime_wide_divider_guard_overlay_rejects_first_page_front_matter() {
        let mut candidate = lm2_test_source_line(
            "p0:l30",
            30,
            "Copyright 2023 Laura Portuondo.",
            0.88,
            false,
            None,
        );
        candidate.page_index = 0;
        candidate.below_footnote_divider = true;
        candidate.font_ratio_page_ref = 0.88;
        candidate.top = 400.0;
        candidate.bottom = 388.0;
        candidate.page_height = 1000.0;
        let mut decoded = vec![(candidate, Lm2Action::Keep)];

        apply_d1_runtime_wide_divider_guard_overlay(&mut decoded);

        assert_eq!(decoded[0].1, Lm2Action::Keep);
        assert_eq!(decoded[0].0.role_hint, None);
    }

    #[test]
    fn d1_runtime_wide_divider_guard_overlay_rejects_large_font_line() {
        let mut candidate = lm2_test_source_line(
            "p0:l30",
            30,
            "federal courts applying the same equitable doctrine.",
            0.98,
            false,
            None,
        );
        candidate.below_footnote_divider = true;
        candidate.font_ratio_page_ref = 0.98;
        candidate.top = 400.0;
        candidate.bottom = 388.0;
        candidate.page_height = 1000.0;
        let mut decoded = vec![(candidate, Lm2Action::Keep)];

        apply_d1_runtime_wide_divider_guard_overlay(&mut decoded);

        assert_eq!(decoded[0].1, Lm2Action::Keep);
    }

    #[test]
    fn d1_runtime_wide_divider_guard_overlay_rejects_upper_page_line() {
        let mut candidate = lm2_test_source_line(
            "p0:l30",
            30,
            "federal courts applying the same equitable doctrine.",
            0.88,
            false,
            None,
        );
        candidate.below_footnote_divider = true;
        candidate.font_ratio_page_ref = 0.88;
        candidate.top = 620.0;
        candidate.bottom = 608.0;
        candidate.page_height = 1000.0;
        let mut decoded = vec![(candidate, Lm2Action::Keep)];

        apply_d1_runtime_wide_divider_guard_overlay(&mut decoded);

        assert_eq!(decoded[0].1, Lm2Action::Keep);
    }

    #[test]
    fn d1_runtime_wide_divider_guard_overlay_rejects_table_stat_line() {
        let mut candidate = lm2_test_source_line(
            "p0:l30",
            30,
            "Table 1 2020 2021 2022 45% 51%",
            0.88,
            false,
            None,
        );
        candidate.below_footnote_divider = true;
        candidate.font_ratio_page_ref = 0.88;
        candidate.top = 400.0;
        candidate.bottom = 388.0;
        candidate.page_height = 1000.0;
        let mut decoded = vec![(candidate, Lm2Action::Keep)];

        apply_d1_runtime_wide_divider_guard_overlay(&mut decoded);

        assert_eq!(decoded[0].1, Lm2Action::Keep);
    }

    #[test]
    fn d1_runtime_footer_artifact_overlay_hides_indd_footer() {
        let candidate = lm2_test_source_line(
            "p0:l30",
            30,
            "jobname: article-22-3-1.indd PDFOutput Page 1",
            0.88,
            false,
            Some(LiquidBlockRole::Footnote),
        );
        let mut decoded = vec![(candidate, Lm2Action::Marginalia)];

        apply_d1_runtime_footer_artifact_overlay(&mut decoded);

        assert_eq!(decoded[0].1, Lm2Action::HideNoise);
        assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Noise));
    }

    #[test]
    fn d1_runtime_footer_artifact_overlay_hides_repository_footer() {
        let candidate = lm2_test_source_line(
            "p0:l30",
            30,
            "This Article is brought to you for free and open access by the Law Archive of Scholarship.",
            0.88,
            false,
            None,
        );
        let mut decoded = vec![(candidate, Lm2Action::Keep)];

        apply_d1_runtime_footer_artifact_overlay(&mut decoded);

        assert_eq!(decoded[0].1, Lm2Action::HideNoise);
        assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Noise));
    }

    #[test]
    fn d1_runtime_footer_artifact_overlay_rejects_author_contact_lines() {
        let candidate = lm2_test_source_line(
            "p0:l30",
            30,
            "For more information, please contact repository@example.edu or phone: 555-0100.",
            0.88,
            false,
            Some(LiquidBlockRole::Footnote),
        );
        let mut decoded = vec![(candidate, Lm2Action::Marginalia)];

        apply_d1_runtime_footer_artifact_overlay(&mut decoded);

        assert_eq!(decoded[0].1, Lm2Action::Marginalia);
        assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Footnote));
    }

    #[test]
    fn d1_runtime_footer_artifact_overlay_preserves_existing_hide_noise() {
        let candidate = lm2_test_source_line(
            "p0:l30",
            30,
            "doi: 10.1234/test",
            0.88,
            false,
            Some(LiquidBlockRole::Noise),
        );
        let mut decoded = vec![(candidate, Lm2Action::HideNoise)];

        apply_d1_runtime_footer_artifact_overlay(&mut decoded);

        assert_eq!(decoded[0].1, Lm2Action::HideNoise);
        assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Noise));
    }

    #[test]
    fn d1_runtime_safe_numeric_note_overlay_recovers_later_page_note_start() {
        let mut line = lm2_test_source_line(
            "p8:l11",
            11,
            "13. Woman Records Racist Coronavirus Rant While on Subway, CNN (Feb. 21, 2020),",
            0.86,
            false,
            None,
        );
        line.page_index = 8;
        line.bottom = 471.0;
        line.top = 483.0;
        line.page_height = 720.0;
        line.font_ratio_doc = 0.86;
        let mut decoded = vec![(line, Lm2Action::Keep)];

        apply_d1_runtime_safe_numeric_note_overlay(&mut decoded);

        assert_eq!(decoded[0].1, Lm2Action::Marginalia);
        assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Footnote));
    }

    #[test]
    fn d1_runtime_safe_numeric_note_overlay_rejects_short_plain_note_shape() {
        let mut line = lm2_test_source_line(
            "p8:l11",
            11,
            "13. Short plain body-like line",
            0.86,
            false,
            None,
        );
        line.page_index = 8;
        line.bottom = 471.0;
        line.top = 483.0;
        line.page_height = 720.0;
        line.font_ratio_doc = 0.86;
        let mut decoded = vec![(line, Lm2Action::Keep)];

        apply_d1_runtime_safe_numeric_note_overlay(&mut decoded);

        assert_eq!(decoded[0].1, Lm2Action::Keep);
        assert_eq!(decoded[0].0.role_hint, None);
    }

    #[test]
    fn footnote_monotone_overlay_recovers_cross_page_note_start() {
        let mut line = lm2_test_source_line(
            "p8:l11",
            11,
            "23. See Smith, supra note.",
            0.88,
            false,
            None,
        );
        line.page_index = 8;
        line.bottom = 0.43;
        line.top = 0.45;
        line.page_has_footnote_divider = true;
        line.doc_note_marker = 23;
        line.doc_note_marker_first_on_page = true;
        line.doc_note_marker_mid_sequence_page = true;
        line.doc_note_marker_follows_previous_page = true;
        line.doc_note_marker_page_delta = 1;
        let mut decoded = vec![(line, Lm2Action::Keep)];

        apply_footnote_monotone_overlay(&mut decoded);

        assert_eq!(decoded[0].1, Lm2Action::Marginalia);
        assert_eq!(decoded[0].0.role_hint, Some(LiquidBlockRole::Footnote));
    }

    #[test]
    fn footnote_monotone_overlay_recovers_immediate_continuation() {
        let mut anchor = lm2_test_source_line(
            "p8:l11",
            11,
            "23. See Smith v. Jones, 123 U.S. 45, 51 (2020).",
            0.88,
            false,
            None,
        );
        anchor.page_index = 8;
        anchor.bottom = 0.43;
        anchor.top = 0.45;
        anchor.page_has_footnote_divider = true;
        anchor.doc_note_marker = 23;
        let mut continuation = lm2_test_source_line(
            "p8:l12",
            12,
            "explaining the same equitable doctrine in later federal courts.",
            0.88,
            false,
            None,
        );
        continuation.page_index = 8;
        continuation.bottom = 0.40;
        continuation.top = 0.42;
        continuation.doc_footnote_continuation = true;
        let mut decoded = vec![
            (anchor, Lm2Action::Marginalia),
            (continuation, Lm2Action::Keep),
        ];

        apply_footnote_monotone_overlay(&mut decoded);

        assert_eq!(decoded[1].1, Lm2Action::Marginalia);
        assert_eq!(decoded[1].0.role_hint, Some(LiquidBlockRole::Footnote));
    }

    #[test]
    fn footnote_monotone_overlay_rejects_body_numbered_prose() {
        let mut line = lm2_test_source_line(
            "p8:l11",
            11,
            "23. This section explains the doctrine in ordinary body prose.",
            1.0,
            false,
            None,
        );
        line.page_index = 8;
        line.bottom = 0.62;
        line.top = 0.64;
        line.doc_note_marker = 23;
        line.doc_note_marker_first_on_page = true;
        line.doc_note_marker_mid_sequence_page = true;
        line.doc_note_marker_follows_previous_page = true;
        line.doc_note_marker_page_delta = 1;
        let mut decoded = vec![(line, Lm2Action::Keep)];

        apply_footnote_monotone_overlay(&mut decoded);

        assert_eq!(decoded[0].1, Lm2Action::Keep);
        assert_eq!(decoded[0].0.role_hint, None);
    }

    #[test]
    fn footnote_monotone_overlay_recovers_gap_marker_between_known_notes() {
        let mut note22 = lm2_test_source_line(
            "p4:l20",
            20,
            "22. Earlier note already decoded as marginalia.",
            0.86,
            false,
            Some(LiquidBlockRole::Footnote),
        );
        note22.doc_note_marker = 22;
        note22.below_footnote_divider = true;
        let mut note23 = lm2_test_source_line(
            "p4:l21",
            21,
            "23. Missing cited source that the first decode kept.",
            0.86,
            false,
            None,
        );
        note23.doc_note_marker = 23;
        note23.below_footnote_divider = true;
        note23.page_has_footnote_divider = true;
        note23.top = 390.0;
        note23.bottom = 378.0;
        note23.page_height = 1000.0;
        let mut note24 = lm2_test_source_line(
            "p4:l22",
            22,
            "24. Later note already decoded as marginalia.",
            0.86,
            false,
            Some(LiquidBlockRole::Footnote),
        );
        note24.doc_note_marker = 24;
        note24.below_footnote_divider = true;
        let mut decoded = vec![
            (note22, Lm2Action::Marginalia),
            (note23, Lm2Action::Keep),
            (note24, Lm2Action::Marginalia),
        ];

        apply_footnote_monotone_overlay(&mut decoded);

        assert_eq!(decoded[1].1, Lm2Action::Marginalia);
        assert_eq!(decoded[1].0.role_hint, Some(LiquidBlockRole::Footnote));
    }

    #[test]
    fn footnote_monotone_overlay_recovers_body_marker_matched_note() {
        let mut body = lm2_test_source_line(
            "p0:l10",
            10,
            "The court adopted the same rule in later cases.\u{00B2}\u{00B3}",
            1.0,
            false,
            None,
        );
        body.right = 520.0;
        body.page_width = 600.0;
        let mut note = lm2_test_source_line(
            "p0:l45",
            45,
            "23. Source explaining the later line of authority.",
            0.86,
            false,
            None,
        );
        note.doc_note_marker = 23;
        note.below_footnote_divider = true;
        note.page_has_footnote_divider = true;
        note.top = 370.0;
        note.bottom = 358.0;
        note.page_height = 1000.0;
        let mut decoded = vec![(body, Lm2Action::Keep), (note, Lm2Action::Keep)];

        apply_footnote_monotone_overlay(&mut decoded);

        assert_eq!(decoded[1].1, Lm2Action::Marginalia);
        assert_eq!(decoded[1].0.role_hint, Some(LiquidBlockRole::Footnote));
    }

    #[test]
    fn footnote_monotone_overlay_rejects_body_marker_without_note_geometry() {
        let mut body = lm2_test_source_line(
            "p0:l10",
            10,
            "The court adopted the same rule in later cases.\u{00B2}\u{00B3}",
            1.0,
            false,
            None,
        );
        body.right = 520.0;
        body.page_width = 600.0;
        let mut false_note = lm2_test_source_line(
            "p0:l11",
            11,
            "23. Ordinary numbered body paragraph, not a footnote.",
            1.0,
            false,
            None,
        );
        false_note.doc_note_marker = 23;
        false_note.top = 690.0;
        false_note.bottom = 678.0;
        false_note.page_height = 1000.0;
        let mut decoded = vec![(body, Lm2Action::Keep), (false_note, Lm2Action::Keep)];

        apply_footnote_monotone_overlay(&mut decoded);

        assert_eq!(decoded[1].1, Lm2Action::Keep);
        assert_eq!(decoded[1].0.role_hint, None);
    }

    #[test]
    fn open_footnote_carryover_overlay_recovers_dividerless_continuation() {
        let mut tail = lm2_test_source_line(
            "p0:l40",
            40,
            "23. See Smith v. Jones, 123 F.3d 456, 460, explaining that",
            0.88,
            false,
            Some(LiquidBlockRole::Footnote),
        );
        tail.page_index = 0;
        tail.bottom = 0.42;
        tail.top = 0.44;
        tail.in_footnote_zone = true;
        tail.doc_note_marker = 23;

        let mut continuation = lm2_test_source_line(
            "p1:l1",
            1,
            "later courts followed the same rule in closely related cases.",
            0.88,
            false,
            Some(LiquidBlockRole::Footnote),
        );
        continuation.page_index = 1;
        continuation.bottom = 0.70;
        continuation.top = 0.72;
        continuation.page_has_footnote_divider = false;
        continuation.in_footnote_zone = false;
        continuation.right = 0.88;

        let mut decoded = vec![
            (tail, Lm2Action::Marginalia),
            (continuation, Lm2Action::Keep),
        ];

        assert!(!open_footnote_carryover_reject_line(&decoded[0].0));
        assert!(open_footnote_carryover_tail_candidate(
            &decoded[0].0,
            decoded[0].1
        ));
        assert!(!open_footnote_carryover_has_terminal_punctuation(
            &decoded[0].0.text
        ));
        assert!(open_footnote_carryover_page_tail_state(&decoded, &[0]).is_some());
        assert!(open_footnote_carryover_candidate(&decoded[1].0));
        apply_open_footnote_carryover_overlay(&mut decoded);

        assert_eq!(decoded[1].1, Lm2Action::Marginalia);
        assert_eq!(decoded[1].0.role_hint, Some(LiquidBlockRole::Footnote));
    }

    #[test]
    fn open_footnote_carryover_overlay_stops_at_expected_marker() {
        let mut tail = lm2_test_source_line(
            "p0:l40",
            40,
            "23. See Smith v. Jones, 123 F.3d 456, 460, explaining that",
            0.88,
            false,
            Some(LiquidBlockRole::Footnote),
        );
        tail.page_index = 0;
        tail.bottom = 0.42;
        tail.top = 0.44;
        tail.in_footnote_zone = true;
        tail.doc_note_marker = 23;

        let mut next_marker = lm2_test_source_line(
            "p1:l1",
            1,
            "24. A new footnote starts on this page.",
            0.88,
            false,
            None,
        );
        next_marker.page_index = 1;
        next_marker.doc_note_marker = 24;
        next_marker.in_footnote_zone = true;

        let mut following = lm2_test_source_line(
            "p1:l2",
            2,
            "ordinary text after the new note marker should not be captured.",
            0.88,
            false,
            None,
        );
        following.page_index = 1;
        following.in_footnote_zone = true;

        let mut decoded = vec![
            (tail, Lm2Action::Marginalia),
            (next_marker, Lm2Action::Keep),
            (following, Lm2Action::Keep),
        ];

        apply_open_footnote_carryover_overlay(&mut decoded);

        assert_eq!(decoded[1].1, Lm2Action::Keep);
        assert_eq!(decoded[2].1, Lm2Action::Keep);
    }

    #[test]
    fn open_footnote_carryover_overlay_rejects_body_font_resume() {
        let mut tail = lm2_test_source_line(
            "p0:l40",
            40,
            "23. See Smith v. Jones, 123 F.3d 456, 460, explaining that",
            0.88,
            false,
            Some(LiquidBlockRole::Footnote),
        );
        tail.page_index = 0;
        tail.bottom = 0.42;
        tail.top = 0.44;
        tail.in_footnote_zone = true;
        tail.doc_note_marker = 23;

        let mut body = lm2_test_source_line(
            "p1:l1",
            1,
            "This section resumes the ordinary argument in body-sized prose.",
            1.0,
            false,
            None,
        );
        body.page_index = 1;
        body.bottom = 0.70;
        body.top = 0.72;
        body.page_has_footnote_divider = false;
        body.in_footnote_zone = false;

        let mut decoded = vec![(tail, Lm2Action::Marginalia), (body, Lm2Action::Keep)];

        apply_open_footnote_carryover_overlay(&mut decoded);

        assert_eq!(decoded[1].1, Lm2Action::Keep);
        assert_eq!(decoded[1].0.role_hint, None);
    }

    #[test]
    fn d1_runtime_zerospend_cue_rejects_body_prose_false_positives() {
        assert!(!d1_runtime_zerospend_cue(
            "in the city of London, by which a broker making a contract was held per\u{0002}"
        ));
        assert!(!d1_runtime_zerospend_cue(
            "sonally liable as purchaser, if he did not at the time of the contract disclose"
        ));
        assert!(!d1_runtime_zerospend_cue(
            "as explaining the language of the written contract, or adding to it a tacitly"
        ));
        assert!(!d1_runtime_zerospend_cue(
            "child, and subsequently, were inadmissible to prove it illegitimate. Ibid."
        ));
        assert!(!d1_runtime_zerospend_cue("EN BANC 1 (2023)."));
    }

    #[test]
    fn d1_runtime_zerospend_cue_accepts_legal_citation_rows() {
        assert!(d1_runtime_zerospend_cue(
            "45. Koons v. Platkin, 673 F. Supp. 3d 515, 620 (D.N.J. 2023)."
        ));
        assert!(d1_runtime_zerospend_cue(
            "Weddings: Domicile, Public Policy, and Inequality in Family Law, 2014 Mich. St. L. Rev."
        ));
    }

    #[test]
    fn lm2_source_signature_changes_when_pymupdf_grouping_toggle_changes() {
        let temp_dir = std::env::temp_dir();
        let path = temp_dir.join("lawpdf-lm2-signature-toggle-test.pdf");
        std::fs::write(&path, b"%PDF-1.4\n%test\n").expect("temp pdf");

        let pages = vec!["page one".to_owned(), "page two".to_owned()];
        let signature = |use_pymupdf_blocks,
                         pp_footnote_region_membership,
                         marker_decoder_prior,
                         small_font_decoder_prior,
                         small_font_sequence_prior,
                         anchored_marginalia_flow_guard,
                         body_preservation_guard,
                         action_neutral_blocksplit,
                         toc_overlay,
                         front_matter_guard,
                         marginalia_preservation_guard,
                         d1_runtime_zerospend_overlay: bool,
                         d1_runtime_continuation_overlay,
                         d1_runtime_immediate_continuation_overlay,
                         d1_runtime_sandwiched_continuation_overlay,
                         d1_runtime_wide_sandwich_overlay,
                         d1_runtime_safe_numeric_note_overlay,
                         d1_runtime_post_wide_cue_overlay,
                         d1_runtime_postcue_citation_next1_overlay,
                         d1_runtime_near8_cue_overlay,
                         d1_runtime_wide_divider_guard_overlay,
                         d1_runtime_geometric_zone_overlay,
                         d1_runtime_footer_artifact_overlay,
                         page_object_tuned_overlay,
                         start_score_scale,
                         transition_score_scale| {
            let d1_runtime_zerospend_overlay_version =
                d1_runtime_zerospend_overlay.then_some(LM2_D1_RUNTIME_ZEROSPEND_OVERLAY_VERSION);
            lm2_source_signature(
                &path,
                &pages,
                "model-a",
                None,
                None,
                None,
                use_pymupdf_blocks,
                pp_footnote_region_membership,
                marker_decoder_prior,
                small_font_decoder_prior,
                small_font_sequence_prior,
                anchored_marginalia_flow_guard,
                body_preservation_guard,
                action_neutral_blocksplit,
                toc_overlay,
                front_matter_guard,
                marginalia_preservation_guard,
                d1_runtime_zerospend_overlay,
                d1_runtime_zerospend_overlay_version,
                d1_runtime_continuation_overlay,
                d1_runtime_immediate_continuation_overlay,
                d1_runtime_sandwiched_continuation_overlay,
                d1_runtime_wide_sandwich_overlay,
                d1_runtime_safe_numeric_note_overlay,
                d1_runtime_post_wide_cue_overlay,
                d1_runtime_postcue_citation_next1_overlay,
                d1_runtime_near8_cue_overlay,
                d1_runtime_wide_divider_guard_overlay,
                d1_runtime_geometric_zone_overlay,
                d1_runtime_footer_artifact_overlay,
                false,
                false,
                false,
                false,
                page_object_tuned_overlay,
                start_score_scale,
                transition_score_scale,
            )
        };
        let without = signature(
            false, false, false, false, false, false, false, false, false, false, false, false,
            false, false, false, false, false, false, false, false, false, false, false, false,
            1.0, 1.0,
        );
        let with = signature(
            true, false, false, false, false, false, false, false, false, false, false, false,
            false, false, false, false, false, false, false, false, false, false, false, false,
            1.0, 1.0,
        );
        let with_pp_footnote_membership = signature(
            false, true, false, false, false, false, false, false, false, false, false, false,
            false, false, false, false, false, false, false, false, false, false, false, false,
            1.0, 1.0,
        );
        let with_marker_prior = signature(
            false, false, true, false, false, false, false, false, false, false, false, false,
            false, false, false, false, false, false, false, false, false, false, false, false,
            1.0, 1.0,
        );
        let with_small_font_prior = signature(
            false, false, false, true, false, false, false, false, false, false, false, false,
            false, false, false, false, false, false, false, false, false, false, false, false,
            1.0, 1.0,
        );
        let with_small_font_sequence_prior = signature(
            false, false, false, false, true, false, false, false, false, false, false, false,
            false, false, false, false, false, false, false, false, false, false, false, false,
            1.0, 1.0,
        );
        let with_anchored_flow_guard = signature(
            false, false, false, false, false, true, false, false, false, false, false, false,
            false, false, false, false, false, false, false, false, false, false, false, false,
            1.0, 1.0,
        );
        let with_blocksplit = signature(
            false, false, false, false, false, false, false, true, false, false, false, false,
            false, false, false, false, false, false, false, false, false, false, false, false,
            1.0, 1.0,
        );
        let with_toc_overlay = signature(
            false, false, false, false, false, false, false, false, true, false, false, false,
            false, false, false, false, false, false, false, false, false, false, false, false,
            1.0, 1.0,
        );
        let with_front_matter_guard = signature(
            false, false, false, false, false, false, false, false, false, true, false, false,
            false, false, false, false, false, false, false, false, false, false, false, false,
            1.0, 1.0,
        );
        let with_marginalia_preservation_guard = signature(
            false, false, false, false, false, false, false, false, false, false, true, false,
            false, false, false, false, false, false, false, false, false, false, false, false,
            1.0, 1.0,
        );
        let with_d1_runtime_zerospend_overlay = signature(
            false, false, false, false, false, false, false, false, false, false, false, true,
            false, false, false, false, false, false, false, false, false, false, false, false,
            1.0, 1.0,
        );
        let with_d1_runtime_continuation_overlay = signature(
            false, false, false, false, false, false, false, false, false, false, false, false,
            true, false, false, false, false, false, false, false, false, false, false, false, 1.0,
            1.0,
        );
        let with_d1_runtime_immediate_continuation_overlay = signature(
            false, false, false, false, false, false, false, false, false, false, false, false,
            false, true, false, false, false, false, false, false, false, false, false, false, 1.0,
            1.0,
        );
        let with_d1_runtime_sandwiched_continuation_overlay = signature(
            false, false, false, false, false, false, false, false, false, false, false, false,
            false, false, true, false, false, false, false, false, false, false, false, false, 1.0,
            1.0,
        );
        let with_d1_runtime_wide_sandwich_overlay = signature(
            false, false, false, false, false, false, false, false, false, false, false, false,
            false, false, false, true, false, false, false, false, false, false, false, false, 1.0,
            1.0,
        );
        let with_d1_runtime_safe_numeric_note_overlay = signature(
            false, false, false, false, false, false, false, false, false, false, false, false,
            false, false, false, false, true, false, false, false, false, false, false, false, 1.0,
            1.0,
        );
        let with_d1_runtime_post_wide_cue_overlay = signature(
            false, false, false, false, false, false, false, false, false, false, false, false,
            false, false, false, false, false, true, false, false, false, false, false, false, 1.0,
            1.0,
        );
        let with_d1_runtime_postcue_citation_next1_overlay = signature(
            false, false, false, false, false, false, false, false, false, false, false, false,
            false, false, false, false, false, false, true, false, false, false, false, false, 1.0,
            1.0,
        );
        let with_d1_runtime_near8_cue_overlay = signature(
            false, false, false, false, false, false, false, false, false, false, false, false,
            false, false, false, false, false, false, false, true, false, false, false, false, 1.0,
            1.0,
        );
        let with_d1_runtime_wide_divider_guard_overlay = signature(
            false, false, false, false, false, false, false, false, false, false, false, false,
            false, false, false, false, false, false, false, false, true, false, false, false, 1.0,
            1.0,
        );
        let with_d1_runtime_geometric_zone_overlay = signature(
            false, false, false, false, false, false, false, false, false, false, false, false,
            false, false, false, false, false, false, false, false, false, true, false, false, 1.0,
            1.0,
        );
        let with_d1_runtime_footer_artifact_overlay = signature(
            false, false, false, false, false, false, false, false, false, false, false, false,
            false, false, false, false, false, false, false, false, false, false, false, true, 1.0,
            1.0,
        );
        let with_page_object_tuned_overlay = signature(
            false, false, false, false, false, false, false, false, false, false, false, false,
            false, false, false, false, false, false, false, false, false, false, false, true, 1.0,
            1.0,
        );
        let with_decoder_scale = signature(
            false, false, false, false, false, false, false, false, false, false, false, false,
            false, false, false, false, false, false, false, false, false, false, false, false,
            3.0, 3.0,
        );
        let with_static_overlay = lm2_source_signature(
            &path,
            &pages,
            "model-a",
            None,
            None,
            Some("a55:test"),
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            None,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            1.0,
            1.0,
        );
        let with_context_twopass = lm2_source_signature(
            &path,
            &pages,
            "model-a",
            Some(LM2_CONTEXT_TWOPASS_VERSION),
            None,
            None,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            None,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            1.0,
            1.0,
        );

        assert_ne!(without, with);
        assert_ne!(without, with_pp_footnote_membership);
        assert_ne!(without, with_marker_prior);
        assert_ne!(without, with_small_font_prior);
        assert_ne!(without, with_small_font_sequence_prior);
        assert_ne!(without, with_anchored_flow_guard);
        assert_ne!(without, with_blocksplit);
        assert_ne!(without, with_toc_overlay);
        assert_ne!(without, with_front_matter_guard);
        assert_ne!(without, with_marginalia_preservation_guard);
        assert_ne!(without, with_d1_runtime_zerospend_overlay);
        assert_ne!(without, with_d1_runtime_continuation_overlay);
        assert_ne!(without, with_d1_runtime_immediate_continuation_overlay);
        assert_ne!(without, with_d1_runtime_sandwiched_continuation_overlay);
        assert_ne!(without, with_d1_runtime_wide_sandwich_overlay);
        assert_ne!(without, with_d1_runtime_safe_numeric_note_overlay);
        assert_ne!(without, with_d1_runtime_post_wide_cue_overlay);
        assert_ne!(without, with_d1_runtime_postcue_citation_next1_overlay);
        assert_ne!(without, with_d1_runtime_near8_cue_overlay);
        assert_ne!(without, with_d1_runtime_wide_divider_guard_overlay);
        assert_ne!(without, with_d1_runtime_geometric_zone_overlay);
        assert_ne!(without, with_d1_runtime_footer_artifact_overlay);
        assert_ne!(without, with_page_object_tuned_overlay);
        assert_ne!(without, with_decoder_scale);
        assert_ne!(without, with_static_overlay);
        assert_ne!(without, with_context_twopass);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn fast_cache_pointer_restores_complete_document_without_geometry() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let source_path = std::env::temp_dir().join(format!("lawpdf-fast-cache-{nonce}.pdf"));
        std::fs::write(&source_path, b"fast-cache-fixture").unwrap();
        let source_signature = format!("fast-cache-test-{nonce}");
        let document = LiquidDocument {
            title: "Cached".to_owned(),
            blocks: vec![LiquidBlock {
                role: LiquidBlockRole::Paragraph,
                text: "Cached body".to_owned(),
                label: None,
            }],
            block_source_lines: vec![LiquidBlockSourceLines {
                block_index: 0,
                lines: vec![LiquidSourceLineRef {
                    id: Some("p0:l0".to_owned()),
                    page_index: 0,
                    line_index: 0,
                    text: "Cached body".to_owned(),
                    role: LiquidBlockRole::Paragraph,
                    note_markers: Vec::new(),
                }],
            }],
            footnote_links: Vec::new(),
            footnote_link_integrity: None,
            profile: Some(lm2_profile()),
            noise_lines_removed: 0,
            llm_used: false,
            llm_provider: Some("LM2".to_owned()),
            deep_liquid_used: false,
            deep_liquid_model: None,
            warnings: Vec::new(),
            source_signature: source_signature.clone(),
        };
        save_cached_lm2_document(&document).unwrap();
        save_fast_cached_lm2_document(&source_path, false, false, &document).unwrap();

        let loaded = load_fast_cached_liquid_mode2_document(&source_path, false, false).unwrap();
        assert_eq!(loaded.title, "Cached");
        assert_eq!(loaded.source_signature, source_signature);

        if let Some(path) = lm2_cache_path(&source_signature) {
            let _ = std::fs::remove_file(path);
        }
        if let Some(path) = fast_cache::lm2_fast_cache_path(&source_path, false, false) {
            let _ = std::fs::remove_file(path);
        }
        let _ = std::fs::remove_file(source_path);
    }

    #[test]
    fn sentineled_note_start_preserves_note_identity() {
        assert_eq!(
            sentineled_note_markers("\u{E000}127\u{E001} See authority."),
            vec![127]
        );
        assert_eq!(
            sentineled_note_markers(
                "citation continuation. \u{E000}127\u{E001} New note. \u{E000}128\u{E001} Next."
            ),
            vec![127, 128]
        );
    }
}
