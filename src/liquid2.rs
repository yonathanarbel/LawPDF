use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::ffi::{CStr, CString};
#[cfg(any(feature = "devtools", test))]
use std::ffi::{OsStr, OsString};
use std::os::raw::{c_char, c_double, c_float, c_void};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Mutex, OnceLock};
use std::thread;
#[cfg(any(feature = "devtools", test))]
use std::time::SystemTime;
use std::time::{Instant, UNIX_EPOCH};

use crossbeam_channel::Sender;
use libloading::Library;
use serde::{Deserialize, Serialize};

use crate::article_segments::{
    ArticleBoundaryCandidateTrace, ArticleSegmentationLine, detect_article_spans,
    detect_article_spans_with_trace,
};
use crate::hashing::sha256_hex_of_file;
use crate::layout_roles::{CALLOUT_END, CALLOUT_START};
use crate::liquid::{
    ArticleSpan, DeepLiquidSourceLine, DocumentProfileKind, DocumentProfileScore, LiquidBlock,
    LiquidBlockRole, LiquidBlockSourceLines, LiquidDocument, LiquidSourceLineRef,
    attach_footnote_links, should_preserve_terminal_hyphen,
};
use crate::liquidvision::{fill_document_features, liquidvision_enabled};
use crate::pdf_backend::PdfEngine;
use crate::review_reading::{merge_article_and_global_note_starts, omitted_keep_source_ids};
use crate::settings::app_data_dir;

mod fast_cache;
mod fasttab;
mod runtime_status;
pub use fast_cache::load_fast_cached_liquid_mode2_document;
pub(crate) use fast_cache::save_fast_cached_lm2_document;
use fasttab::{Lm2FastTabModel, fasttab_enabled};
pub use runtime_status::run_lm2_runtime_status;

const LM2_SCHEMA_VERSION: &str = "liquidmode2-catboost-v4-max-default-v1-line-article-segmentation";
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
const LM2_ASSEMBLY_CACHE_VERSION: &str = "lm2-assembly-v156-runtime-isolation-and-inference-errors";
const LM2_MAX_NOTE_MARKER: u16 = 999;
const LM2_TABLE_FIGURE_ROUTER_OVERLAY_VERSION: &str = "table-figure-router-v4-default-on";
const LM2_PAGE_OBJECT_OVERLAY_VERSION: &str = "page-object-overlay-v1-guarded-ruled-path";
const LM2_PAGE_OBJECT_TUNED_OVERLAY_VERSION: &str =
    "page-object-tuned-overlay-v2-ruled-body-rescue-keep-preserve";
const LM2_NATIVE_CATBOOST_RUNTIME_DIR: &str = "profile-models/lm2-native-catboost-runtime";
const LM2_NATIVE_CATBOOST_MODEL_FILE: &str = "lm2-catboost-augmented-epoch51lv-relabels-tc.cbm";
const LM2_FASTTAB_RUNTIME_DIR: &str = "profile-models/lm2-fasttab-runtime";
const LM2_FASTTAB_MODEL_FILE: &str = "fasttab-v1.onnx";
const LM2_CONTEXT_TWOPASS_RUNTIME_DIR: &str = "profile-models/lm2-context-twopass-runtime";
const LM2_CONTEXT_TWOPASS_MODEL_FILE: &str = "lm2-context-twopass-hgb-v1.json";
const LM2_CONTEXT_ARBITER_MODEL_FILE: &str = "lm2-runtime-residual-rescue-v7.json";
const LM2_NOTE_HEAD_RUNTIME_DIR: &str = "profile-models/lm2-note-head-runtime";
const LM2_NOTE_HEAD_MODEL_FILE: &str = "lm2-note-head-hgb-v1.json";
const LM2_NOTE_HEAD_SCHEMA_V1: &str = "lawpdf-footnote-head-hgb-v1";
const LM2_NOTE_HEAD_SCHEMA_V2: &str = "lawpdf-footnote-head-stacked-hgb-v2";
const LM2_NOTE_HEAD_SCHEMA_V3: &str = "lawpdf-footnote-head-sequence-hgb-v3";
const LM2_NOTE_HEAD_RUNTIME_VERSION: &str = "note-head-runtime-v3.0-sequence-stacked";
const LM2_NOTE_HEAD_FEATURE_COUNT_V1: usize = 131;
const LM2_NOTE_HEAD_FEATURE_COUNT_V2: usize = 134;
const LM2_NOTE_HEAD_FEATURE_COUNT_V3: usize = 150;
const LM2_NOTE_HEAD_NEIGHBOR_OFFSETS_V3: [isize; 2] = [-1, 1];
const LM2_LINK_RANKER_RUNTIME_DIR: &str = "profile-models/lm2-link-ranker-runtime";
const LM2_LINK_RANKER_MODEL_FILE: &str = "lm2-footnote-link-ranker-hgb-v1.json";
const LM2_LINK_RANKER_SCHEMA_V1: &str = "lawpdf-footnote-link-ranker-hgb-v1";
const LM2_LINK_RANKER_RUNTIME_VERSION: &str = "footnote-link-ranker-runtime-v1";
const LM2_LINK_RANKER_FEATURE_COUNT_V1: usize = 89;
const LM2_LINK_RANKER_FEATURES_V1: [&str; LM2_LINK_RANKER_FEATURE_COUNT_V1] = [
    "marker_norm",
    "marker_log",
    "marker_digits_norm",
    "marker_le_5",
    "marker_ge_100",
    "page_delta_clip",
    "same_page",
    "next_page",
    "body_page_norm",
    "candidate_page_norm",
    "reference_line_known",
    "reference_starts_with_marker",
    "reference_embeds_marker",
    "reference_word_count_norm",
    "reference_font_ratio_page",
    "same_source_line_as_reference",
    "candidate_after_reference",
    "same_page_line_delta",
    "line_index_norm",
    "top_norm",
    "bottom_norm",
    "center_y_norm",
    "width_norm",
    "left_norm",
    "right_margin_norm",
    "font_ratio_page",
    "font_ratio_page_ref",
    "font_ratio_doc",
    "font_height_norm",
    "bold",
    "italic",
    "centered",
    "margin_centered",
    "below_footnote_divider",
    "page_has_footnote_divider",
    "in_footnote_zone",
    "doc_footnote_state",
    "doc_footnote_continuation",
    "segment_footnote_like",
    "segment_furniture_like",
    "segment_table_like",
    "segment_toc_like",
    "segment_first",
    "segment_last",
    "segment_line_position",
    "segment_count_norm",
    "text_length_norm",
    "word_count_norm",
    "digit_ratio",
    "alpha_ratio",
    "uppercase_ratio",
    "punctuation_ratio",
    "space_ratio",
    "suffix_space",
    "suffix_dot",
    "suffix_paren",
    "suffix_bracket",
    "remainder_upper",
    "remainder_lower",
    "remainder_digit",
    "has_url",
    "terminal_punctuation",
    "mostly_upper",
    "local_same_marker_count",
    "document_same_marker_count",
    "prev_numeric_present",
    "prev_numeric_marker_delta",
    "prev_numeric_page_delta",
    "prev_numeric_line_gap",
    "prev_numeric_same_page",
    "prev_numeric_is_current_note",
    "next_numeric_present",
    "next_numeric_marker_delta",
    "next_numeric_page_delta",
    "next_numeric_line_gap",
    "next_numeric_same_page",
    "next_numeric_is_current_note",
    "previous_note_present",
    "previous_note_marker_delta",
    "previous_note_page_delta",
    "previous_note_line_gap",
    "next_note_present",
    "next_note_marker_delta",
    "next_note_page_delta",
    "next_note_line_gap",
    "previous_note_is_marker_minus_one",
    "next_note_is_marker_plus_one",
    "bracketed_by_consecutive_notes",
    "candidate_auth_probability",
];
const LM2_CONTEXT_TWOPASS_VERSION: &str = "context-twopass-hgb-v1-agent2-task30-foldnorm";
const LM2_CONTEXT_ARBITER_SCHEMA_V2: &str = "lawpdf-lm2-context-arbiter-hgb-v2";
const LM2_CONTEXT_RESIDUAL_SCHEMA_V3: &str = "lawpdf-lm2-runtime-residual-hgb-v3";
const LM2_CONTEXT_ARBITER_RUNTIME_VERSION: &str = "context-arbiter-runtime-v2.1-gated-nonkeep";
const LM2_CONTEXT_RESIDUAL_RUNTIME_VERSION: &str = "runtime-residual-v3-rescue-only";
const LM2_CONTEXT_ARBITER_FEATURE_COUNT_V2: usize = 139;
const LM2_CONTEXT_RESIDUAL_FEATURE_COUNT_V3: usize = 142;
const LM2_CONTEXT_ARBITER_NEIGHBOR_OFFSETS: [isize; 4] = [-2, -1, 1, 2];
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
/// Fewer candidate note heads than this and the sequence says nothing.
const NOTE_SEQUENCE_MIN_CANDIDATES: usize = 8;
/// A compact run of explicit document markers is already strong provenance.
const DOCUMENT_NOTE_SEQUENCE_MIN_CANDIDATES: usize = 4;
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Lm2RuntimeChoice {
    #[default]
    Automatic,
    CatBoost,
    FastTab,
}

impl Lm2RuntimeChoice {
    fn fasttab_requested(self) -> bool {
        match self {
            Self::Automatic => fasttab_enabled(),
            Self::CatBoost => false,
            Self::FastTab => true,
        }
    }

    fn cache_key(self) -> u8 {
        match self {
            Self::Automatic => 0,
            Self::CatBoost => 1,
            Self::FastTab => 2,
        }
    }
}

static LM2_RUNTIME_LOADS: AtomicU64 = AtomicU64::new(0);
static LM2_RUNTIME_CACHE: OnceLock<Lm2RuntimeCache> = OnceLock::new();

/// Process-wide home for loaded runtimes, one slot per runtime choice.
///
/// A slot is `busy` while a lease holds (or is loading) its runtime. A second
/// lease for the same choice waits on `released` instead of loading another
/// copy of the model stack: the opening-pages preview and the full document
/// job are spawned back to back, and before this they each loaded, hashed,
/// and parsed every model asset, then the last one to finish overwrote the
/// other's runtime.
struct Lm2RuntimeCache {
    slots: Mutex<HashMap<u8, Lm2RuntimeSlot>>,
    released: std::sync::Condvar,
}

#[derive(Default)]
struct Lm2RuntimeSlot {
    fingerprint: u64,
    runtime: Option<Lm2Runtime>,
    busy: bool,
}

impl Lm2RuntimeCache {
    fn global() -> &'static Self {
        LM2_RUNTIME_CACHE.get_or_init(|| Self {
            slots: Mutex::new(HashMap::new()),
            released: std::sync::Condvar::new(),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<u8, Lm2RuntimeSlot>> {
        self.slots
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

struct Lm2RuntimeLease {
    choice: Lm2RuntimeChoice,
    runtime: Option<Lm2Runtime>,
    fingerprint: u64,
    request_state: Lm2RequestState,
}

struct Lm2RequestState {
    had_pp_priors: bool,
    pp_footnote_region_membership: bool,
}

impl Lm2RequestState {
    fn capture(runtime: &Lm2Runtime) -> Self {
        Self {
            had_pp_priors: runtime.pp_priors.is_some(),
            pp_footnote_region_membership: runtime.pp_footnote_region_membership,
        }
    }

    fn restore(&self, runtime: &mut Lm2Runtime) {
        if !self.had_pp_priors {
            runtime.pp_priors = None;
        }
        runtime.pp_footnote_region_membership = self.pp_footnote_region_membership;
    }
}

impl Lm2RuntimeLease {
    fn acquire(choice: Lm2RuntimeChoice) -> Self {
        let fingerprint = fast_cache::runtime_fingerprint();
        let cache = Lm2RuntimeCache::global();
        let key = choice.cache_key();
        let cached = {
            let mut guard = cache.lock();
            while guard.entry(key).or_default().busy {
                guard = cache
                    .released
                    .wait(guard)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
            }
            let slot = guard.entry(key).or_default();
            slot.busy = true;
            let matches = slot.fingerprint == fingerprint;
            slot.runtime.take().filter(|_| matches)
        };
        let runtime = cached.unwrap_or_else(|| {
            LM2_RUNTIME_LOADS.fetch_add(1, AtomicOrdering::SeqCst);
            Lm2Runtime::load(choice)
        });
        let request_state = Lm2RequestState::capture(&runtime);
        Self {
            choice,
            runtime: Some(runtime),
            fingerprint,
            request_state,
        }
    }

    fn runtime(&mut self) -> &mut Lm2Runtime {
        self.runtime
            .as_mut()
            .expect("LM2 runtime lease is still held")
    }
}

impl Drop for Lm2RuntimeLease {
    fn drop(&mut self) {
        let Some(mut runtime) = self.runtime.take() else {
            return;
        };
        self.request_state.restore(&mut runtime);
        let cache = Lm2RuntimeCache::global();
        {
            let mut guard = cache.lock();
            let slot = guard.entry(self.choice.cache_key()).or_default();
            slot.fingerprint = self.fingerprint;
            slot.runtime = Some(runtime);
            slot.busy = false;
        }
        cache.released.notify_all();
    }
}

/// Number of times the native Review runtime was actually constructed.
pub fn lm2_runtime_process_load_count() -> u64 {
    LM2_RUNTIME_LOADS.load(AtomicOrdering::SeqCst)
}

/// Drive two prepares through the process cache and report how many new loads
/// each one caused. The second value is 0 when the runtime is reused.
pub fn lm2_runtime_reuse_across_two_prepares(choice: Lm2RuntimeChoice) -> (u64, u64) {
    let before = lm2_runtime_process_load_count();
    drop(Lm2RuntimeLease::acquire(choice));
    let mid = lm2_runtime_process_load_count();
    drop(Lm2RuntimeLease::acquire(choice));
    let after = lm2_runtime_process_load_count();
    (mid - before, after - mid)
}

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
    pub runtime_choice: Lm2RuntimeChoice,
    pub preview_only: bool,
    /// The caller already ran (or is running) a separate opening-pages preview
    /// job, so the full job must not repeat that work before the full pass.
    pub skip_progressive_preview: bool,
}

#[derive(Debug, Clone)]
pub struct LiquidMode2Event {
    pub document_epoch: u64,
    pub path: PathBuf,
    pub complete: bool,
    pub preview_page_count: Option<usize>,
    pub runtime_choice: Lm2RuntimeChoice,
    pub result: Result<LiquidDocument, String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct LiquidMode2Timing {
    pub runtime_load_ms: f64,
    pub liquidvision_fill_ms: f64,
    pub feature_enrichment_ms: f64,
    pub model_decode_ms: f64,
    pub overlay_decode_ms: f64,
    pub footnote_linker_ms: f64,
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
    model_sha256: String,
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
    fasttab_model: Option<Lm2FastTabModel>,
    native_catboost_model: Option<Lm2NativeCatboostModel>,
    context_twopass_model: Option<Lm2ContextTwopassModel>,
    context_arbiter_model: Option<Lm2ContextTwopassModel>,
    note_head_model: Option<Lm2NoteHeadModel>,
    link_ranker_model: Option<Lm2LinkRankerModel>,
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
    numeric_feature_count: Option<usize>,
    #[serde(default)]
    primary_probability_order: Vec<String>,
    #[serde(default)]
    primary_model_sha256: Option<String>,
    #[serde(default)]
    baseline_context_model_sha256: Option<String>,
    #[serde(default)]
    baseline_action_order: Vec<String>,
    #[serde(default)]
    neighbor_offsets: Vec<isize>,
    #[serde(default)]
    calibration: Option<Lm2ContextArbiterCalibration>,
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

#[derive(Debug, Clone, Deserialize)]
struct Lm2ContextArbiterCalibration {
    rescue_keep_threshold: f64,
    demote_to_marginalia_threshold: f64,
    demote_to_noise_threshold: f64,
    #[serde(default = "lm2_context_default_closed_threshold")]
    reclassify_nonkeep_threshold: f64,
}

fn lm2_context_default_closed_threshold() -> f64 {
    1.0
}

#[derive(Debug)]
struct Lm2ContextTwopassModel {
    schema_version: String,
    actions: Vec<String>,
    feature_count: usize,
    calibration: Option<Lm2ContextArbiterCalibration>,
    primary_model_sha256: Option<String>,
    asset_sha256: String,
    doc_to_fold: HashMap<String, usize>,
    unseen_doc_model: String,
    models: Vec<Lm2ContextTwopassHgbModel>,
}

#[derive(Debug, Deserialize)]
struct Lm2NoteHeadModelFile {
    schema_version: String,
    feature_count: usize,
    #[serde(default)]
    numeric_feature_count: Option<usize>,
    #[serde(default)]
    primary_probability_order: Vec<String>,
    #[serde(default)]
    primary_model_sha256: Option<String>,
    #[serde(default)]
    candidate_neighbor_offsets: Vec<isize>,
    baseline_prediction: Vec<f64>,
    trees: Vec<Vec<[f64; 7]>>,
    threshold: f64,
}

#[derive(Debug)]
struct Lm2NoteHeadModel {
    schema_version: String,
    feature_count: usize,
    primary_model_sha256: Option<String>,
    baseline_prediction: f64,
    trees: Vec<Vec<[f64; 7]>>,
    threshold: f64,
    asset_sha256: String,
}

#[derive(Debug, Deserialize)]
struct Lm2LinkRankerModelFile {
    schema_version: String,
    feature_count: usize,
    feature_names: Vec<String>,
    baseline_prediction: Vec<f64>,
    trees: Vec<Vec<[f64; 7]>>,
    threshold: f64,
    auth_threshold: f64,
    auth_model_sha256: String,
    candidate_page_offsets: Vec<i32>,
}

#[derive(Debug)]
struct Lm2LinkRankerModel {
    baseline_prediction: f64,
    trees: Vec<Vec<[f64; 7]>>,
    threshold: f64,
    auth_threshold: f64,
    auth_model_sha256: String,
    asset_sha256: String,
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
    !lm2_native_line_default_asset_available() && !falsey_env(LM2_PAGE_OBJECT_TUNED_DEFAULT_ENV)
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
    thread::Builder::new()
        .name("lawpdf-review-mode".to_owned())
        .spawn(move || {
            let document_epoch = request.document_epoch;
            let path = request.path.clone();
            let runtime_choice = request.runtime_choice;
            // A panic inside the pipeline must surface as a failed Review, not
            // as a dropped channel: the UI keeps showing "Preparing Review
            // Mode..." forever when no event ever arrives.
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run_liquid_mode2_job(request, &tx);
            }));
            if let Err(payload) = outcome {
                let message = format!(
                    "Review Mode stopped unexpectedly: {}",
                    panic_payload_message(payload.as_ref())
                );
                eprintln!("{message}");
                let _ = tx.send(LiquidMode2Event {
                    document_epoch,
                    path,
                    complete: true,
                    preview_page_count: None,
                    runtime_choice,
                    result: Err(message),
                });
            }
        })
        .expect("spawn Review Mode worker thread");
}

/// Best-effort text for a caught panic payload (`panic!` with a `&str` or a
/// `String`, or something else entirely).
pub(crate) fn panic_payload_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "internal error (no panic message)".to_owned()
    }
}

fn run_liquid_mode2_job(request: LiquidMode2Request, tx: &Sender<LiquidMode2Event>) {
    {
        let document_epoch = request.document_epoch;
        let path = request.path.clone();
        let use_pymupdf_blocks = request.use_pymupdf_blocks;
        let use_pp_footnote_regions = request.use_pp_footnote_regions;
        let runtime_choice = request.runtime_choice;
        if request.preview_only {
            let preview_page_count = request.pages.len();
            let result = prepare_liquid_mode2_document(request);
            let _ = tx.send(LiquidMode2Event {
                document_epoch,
                path,
                complete: false,
                preview_page_count: Some(preview_page_count),
                runtime_choice,
                result,
            });
            return;
        }
        if !request.skip_progressive_preview
            && lm2_progressive_preview_enabled()
            && let Some((preview_request, preview_page_count)) =
                lm2_progressive_preview_request(&request)
            && let Ok(document) = prepare_liquid_mode2_document(preview_request)
        {
            let _ = tx.send(LiquidMode2Event {
                document_epoch,
                path: path.clone(),
                complete: false,
                preview_page_count: Some(preview_page_count),
                runtime_choice,
                result: Ok(document),
            });
        }
        let result = prepare_liquid_mode2_document(request);
        if let Ok(document) = &result {
            let _ = save_fast_cached_lm2_document(
                &path,
                use_pymupdf_blocks,
                use_pp_footnote_regions,
                runtime_choice,
                document,
            );
        }
        let _ = tx.send(LiquidMode2Event {
            document_epoch,
            path,
            complete: true,
            preview_page_count: None,
            runtime_choice,
            result,
        });
    }
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
    preview.preview_only = true;
    (!preview.deep_source_lines.is_empty()).then_some((preview, LM2_PROGRESSIVE_PREVIEW_PAGES))
}

// Pipeline modules (mechanical split of the former single file).
mod assembly;
mod cache;
mod decoder;
mod devtools;
mod enrichment;
mod features;
mod overlays;
mod overlays_pre_assembly;
mod passes;
mod pipeline;
mod priors;
mod runtime;
#[cfg(test)]
mod tests;
mod text_repairs;
mod text_util;

// Re-export the public CLI entry points and runtime types under `liquid2::`,
// and pull every module's `pub(super)` items into this namespace so sibling
// modules (and `tests`) reach them through `use super::*`.
use assembly::*;
use cache::*;
use decoder::*;
#[cfg(feature = "devtools")]
pub use devtools::*;
#[cfg(not(feature = "devtools"))]
use devtools::*;
use enrichment::*;
use features::*;
use overlays::*;
use overlays_pre_assembly::*;
use passes::*;
pub use pipeline::*;
use priors::*;
pub(crate) use runtime::*;
use text_repairs::*;
use text_util::*;
