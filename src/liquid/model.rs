//! Core public data model for Liquid Mode documents.
//!
//! This module owns:
//! - LiquidBlockRole (the central semantic taxonomy)
//! - LiquidBlock, LiquidDocument
//! - LiquidRequest, LiquidEvent (the job protocol)
//!
//! Private LLM-related types that were co-located are also here temporarily
//! during the refactor; they will likely migrate into llm/ later.

use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// =============================================================================
// Public API Types (the stable surface used by app.rs and worker entry points)
// =============================================================================

#[derive(Debug, Clone)]
pub struct LiquidRequest {
    pub document_epoch: u64,
    pub path: PathBuf,
    pub title: String,
    pub pages: Vec<String>,
    pub layout_hints: Vec<LiquidLayoutHint>,
    pub source_line_hints: Vec<LiquidSourceLineRef>,
    pub deep_source_lines: Vec<DeepLiquidSourceLine>,
    pub deep_liquid: Option<DeepLiquidConfig>,
    pub groq_api_key: Option<String>,
    pub openrouter_api_key: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LiquidEvent {
    pub document_epoch: u64,
    pub path: PathBuf,
    pub result: Result<LiquidDocument, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiquidDocument {
    pub title: String,
    pub blocks: Vec<LiquidBlock>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub block_source_lines: Vec<LiquidBlockSourceLines>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub footnote_links: Vec<LiquidFootnoteLink>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub footnote_link_integrity: Option<LiquidFootnoteLinkIntegrity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<DocumentProfile>,
    #[serde(default, alias = "footnotes_removed")]
    pub noise_lines_removed: usize,
    pub llm_used: bool,
    /// Which LLM provider produced the layout (e.g. "Groq", "OpenRouter"), if any.
    /// None means local heuristic fallback (or legacy cache entry).
    #[serde(default)]
    pub llm_provider: Option<String>,
    #[serde(default)]
    pub deep_liquid_used: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deep_liquid_model: Option<String>,
    pub warnings: Vec<String>,
    pub source_signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LiquidFootnoteLink {
    pub body_block_index: usize,
    pub body_marker_ordinal: usize,
    pub marker: u16,
    pub note_block_index: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_page_index: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note_page_index: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LiquidFootnoteLinkIntegrity {
    pub detectable_markers: usize,
    pub landed: usize,
    pub unmatched: usize,
    pub ambiguous: usize,
    pub note_heads: usize,
    pub landing_rate: f32,
    pub ambiguous_rate: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentProfile {
    pub kind: DocumentProfileKind,
    pub confidence: f32,
    pub scores: Vec<DocumentProfileScore>,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentProfileScore {
    pub kind: DocumentProfileKind,
    pub score: f32,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
#[schemars(rename_all = "snake_case")]
pub enum DocumentProfileKind {
    LawReviewArticle,
    ScienceArticle,
    Contract,
    LegalFilingOrOpinion,
    NewsArticle,
    FreeProse,
    CvOrAcademicPacket,
    ReceiptInvoiceFinancial,
    CourseOrExamMaterial,
    BookOrChapter,
    PolicyReport,
    FormReceiptAdmin,
    GeneralDocument,
    ScannedImageOnly,
    Other,
}

impl DocumentProfileKind {
    #[allow(dead_code)]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LawReviewArticle => "law_review_article",
            Self::ScienceArticle => "science_article",
            Self::Contract => "contract",
            Self::LegalFilingOrOpinion => "legal_filing_or_opinion",
            Self::NewsArticle => "news_article",
            Self::FreeProse => "free_prose",
            Self::CvOrAcademicPacket => "cv_or_academic_packet",
            Self::ReceiptInvoiceFinancial => "receipt_invoice_financial",
            Self::CourseOrExamMaterial => "course_or_exam_material",
            Self::BookOrChapter => "book_or_chapter",
            Self::PolicyReport => "policy_report",
            Self::FormReceiptAdmin => "form_receipt_admin",
            Self::GeneralDocument => "general_document",
            Self::ScannedImageOnly => "scanned_image_only",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiquidBlock {
    pub role: LiquidBlockRole,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LiquidLayoutHint {
    pub text: String,
    pub role: LiquidBlockRole,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiquidBlockSourceLines {
    pub block_index: usize,
    pub lines: Vec<LiquidSourceLineRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiquidSourceLineRef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub page_index: usize,
    pub line_index: usize,
    pub text: String,
    pub role: LiquidBlockRole,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub note_markers: Vec<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeepLiquidConfig {
    pub python_exe: PathBuf,
    pub script_path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_dir: Option<PathBuf>,
    pub model_id: String,
    pub timeout_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeepLiquidSourceLine {
    pub id: String,
    pub page_index: usize,
    pub page_width: f32,
    pub page_height: f32,
    pub line_index: usize,
    pub text: String,
    pub left: f32,
    pub bottom: f32,
    pub right: f32,
    pub top: f32,
    #[serde(default)]
    pub page_index_norm: f32,
    #[serde(default)]
    pub lines_from_doc_start: usize,
    #[serde(default)]
    pub left_margin_ratio: f32,
    #[serde(default)]
    pub right_margin_ratio: f32,
    #[serde(default)]
    pub indent_both: f32,
    #[serde(default = "default_margin_symmetry")]
    pub margin_symmetry: f32,
    #[serde(default)]
    pub line_width_ratio: f32,
    #[serde(default)]
    pub indent_vs_body: f32,
    #[serde(default = "default_width_vs_body")]
    pub width_vs_body: f32,
    #[serde(default)]
    pub front_matter_zone: bool,
    #[serde(default)]
    pub margin_centered: bool,
    #[serde(default)]
    pub is_block_indented: bool,
    #[serde(default)]
    pub prev_line_indented: bool,
    pub font_height: f32,
    pub font_ratio_page: f32,
    pub font_ratio_page_ref: f32,
    pub font_ratio_doc: f32,
    #[serde(default)]
    pub doc_font_body_z: f32,
    #[serde(default)]
    pub doc_font_footnote_z: f32,
    #[serde(default)]
    pub doc_font_body_size: f32,
    #[serde(default)]
    pub doc_font_footnote_size: f32,
    #[serde(default)]
    pub doc_footnote_state: bool,
    #[serde(default)]
    pub doc_footnote_continuation: bool,
    #[serde(default)]
    pub doc_repeated_edge_text: bool,
    #[serde(default)]
    pub doc_repeated_text_count: u16,
    #[serde(default)]
    pub doc_repeated_top_edge: bool,
    #[serde(default)]
    pub doc_repeated_bottom_edge: bool,
    #[serde(default)]
    pub doc_repeated_numeric_pattern: bool,
    #[serde(default)]
    pub doc_vertical_axis_like: bool,
    #[serde(default)]
    pub doc_vertical_numeric_axis_like: bool,
    #[serde(default)]
    pub doc_vertical_short_text_axis_like: bool,
    #[serde(default)]
    pub page_table_column_like: bool,
    #[serde(default)]
    pub segment_block_id: usize,
    #[serde(default)]
    pub segment_block_line_index: usize,
    #[serde(default)]
    pub segment_block_line_count: usize,
    #[serde(default)]
    pub segment_block_first: bool,
    #[serde(default)]
    pub segment_block_last: bool,
    #[serde(default)]
    pub segment_block_shape: String,
    #[serde(default)]
    pub segment_block_toc_like: bool,
    #[serde(default)]
    pub segment_block_table_like: bool,
    #[serde(default)]
    pub segment_block_footnote_like: bool,
    #[serde(default)]
    pub segment_block_furniture_like: bool,
    #[serde(default)]
    pub page_object_image_overlap_ratio: f32,
    #[serde(default)]
    pub page_object_image_hit_count: u16,
    #[serde(default)]
    pub page_object_path_stroke_near_line_count: u16,
    #[serde(default)]
    pub page_object_path_stroke_density_near_line: f32,
    #[serde(default)]
    pub page_object_thin_horizontal_near_line_count: u16,
    #[serde(default)]
    pub page_object_thin_vertical_near_line_count: u16,
    #[serde(default)]
    pub page_object_overlaps_image_bbox: bool,
    #[serde(default)]
    pub page_object_ruled_row_membership: bool,
    #[serde(default)]
    pub page_object_hide_candidate: bool,
    #[serde(default)]
    pub page_object_hide_candidate_guarded: bool,
    #[serde(default)]
    pub page_object_path15_candidate: bool,
    #[serde(default)]
    pub page_object_ruled_or_path8_candidate: bool,
    #[serde(default)]
    pub line_on_ruled_divider: bool,
    #[serde(default)]
    pub in_ruled_cell: bool,
    #[serde(default)]
    pub ruled_row_membership_exact: bool,
    #[serde(default)]
    pub dist_to_nearest_rule: f32,
    #[serde(default)]
    pub prev_line_has_dotleader: bool,
    #[serde(default)]
    pub prev4_dotleader_count: u8,
    #[serde(default)]
    pub prev4_spaced_dotleader_count: u8,
    #[serde(default)]
    pub prev4_strong_dotleader_count: u8,
    #[serde(default)]
    pub prev4_toc_leader_context: bool,
    #[serde(default)]
    pub doc_note_marker: u16,
    #[serde(default)]
    pub doc_note_marker_first_on_page: bool,
    #[serde(default)]
    pub doc_note_marker_mid_sequence_page: bool,
    #[serde(default)]
    pub doc_note_marker_follows_previous_page: bool,
    #[serde(default)]
    pub doc_note_marker_page_delta: i16,
    pub bold: bool,
    pub italic: bool,
    pub centered: bool,
    pub below_footnote_divider: bool,
    pub page_has_footnote_divider: bool,
    #[serde(default)]
    pub in_footnote_zone: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pp_prior_role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pp_prior_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pp_prior_score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role_hint: Option<LiquidBlockRole>,
    /// LmV (vision) per-line features. Default = all-zero (Lm tier / un-rendered).
    #[serde(default)]
    pub lv: crate::liquidvision::LvLineFeatures,
}

fn default_margin_symmetry() -> f32 {
    1.0
}

fn default_width_vs_body() -> f32 {
    1.0
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
#[schemars(rename_all = "snake_case")]
pub enum LiquidBlockRole {
    Title,
    Heading,
    Subheading,
    Abstract,
    Syllabus,
    AuthorInfo,
    Lead,
    Paragraph,
    Explainer,
    Takeaway,
    Holding,
    Issue,
    Definition,
    Marginalia,
    Clause,
    ListItem,
    Quote,
    Caption,
    Table,
    Contents,
    KeyClause,
    Header,
    Footer,
    Footnote,
    Metadata,
    SectionBreak,
    Noise,
}

impl LiquidBlockRole {
    pub fn prompt_name(self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::Heading => "heading",
            Self::Subheading => "subheading",
            Self::Abstract => "abstract",
            Self::Syllabus => "syllabus",
            Self::AuthorInfo => "author_info",
            Self::Lead => "lead",
            Self::Paragraph => "paragraph",
            Self::Explainer => "explainer",
            Self::Takeaway => "takeaway",
            Self::Holding => "holding",
            Self::Issue => "issue",
            Self::Definition => "definition",
            Self::Marginalia => "marginalia",
            Self::Clause => "clause",
            Self::ListItem => "list_item",
            Self::Quote => "quote",
            Self::Caption => "caption",
            Self::Table => "table",
            Self::Contents => "contents",
            Self::KeyClause => "key_clause",
            Self::Header => "header",
            Self::Footer => "footer",
            Self::Footnote => "footnote",
            Self::Metadata => "metadata",
            Self::SectionBreak => "section_break",
            Self::Noise => "noise",
        }
    }
}

// =============================================================================
// LLM-internal types (used by the LLM refinement pass)
// These were private in the original monolithic file.
// They are kept here for now to minimize churn; they can move to llm/types.rs
// once the LLM module is more fully extracted.
// =============================================================================

#[derive(Debug, Clone, Copy)]
pub(crate) struct LlmProvider {
    pub name: &'static str,
    pub url: &'static str,
    pub model: &'static str,
    pub max_tokens_field: &'static str,
    pub max_completion_tokens: usize,
    pub reasoning_effort: Option<&'static str>,
    pub openrouter_headers: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct LlmLayout {
    #[serde(default)]
    pub blocks: Vec<LlmBlock>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct LlmBlock {
    pub source_index: usize,
    // Accepted for compatibility with LLM responses that echo an unused block identifier.
    #[serde(default, rename = "block")]
    pub _block: Option<String>,
    #[serde(default, rename = "type")]
    pub style_type: Option<String>,
    #[serde(default)]
    pub role: Option<LiquidBlockRole>,
    #[serde(default = "default_keep")]
    pub action: LlmAction,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub visual_break_before: bool,
    // Prompt no longer advertises box/background/text color fields. Extraction
    // produces no inline styles; renderer uses only role, label, and visual breaks.
}

#[derive(Debug, Deserialize, PartialEq, Clone, Copy, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(rename_all = "snake_case")]
pub(crate) enum LlmAction {
    Keep,
    Remove,
}

pub(crate) fn default_keep() -> LlmAction {
    LlmAction::Keep
}

#[derive(Debug, Serialize)]
pub(crate) struct LiquidLlmLog {
    pub timestamp_unix_secs: u64,
    pub title: String,
    pub source_signature: String,
    pub provider: String,
    pub model: String,
    pub block_count: usize,
    pub prompt_block_count: usize,
    pub system_prompt: Option<String>,
    pub user_prompt: Option<String>,
    pub request_body: Option<serde_json::Value>,
    pub http_status: Option<u16>,
    pub success: bool,
    pub error: Option<String>,
    pub generation_id: Option<String>,
    pub response_preview: Option<String>,
    pub assistant_content_preview: Option<String>,
    pub response_text: Option<String>,
    pub assistant_content: Option<String>,
    pub parsed_layout_blocks: Option<usize>,
}
