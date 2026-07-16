mod chat_ui;
mod settings_ui;
mod search_state;
mod selection_state;
mod tts_controller;
mod update_ui;

use chat_ui::{ChatState, ChatUi};
use settings_ui::SettingsUi;
use search_state::SearchState;
use selection_state::SelectionState;
use tts_controller::TtsController;
use update_ui::UpdateUi;

use std::collections::{HashMap, HashSet, VecDeque, hash_map::DefaultHasher};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crossbeam_channel::{Receiver, Sender, unbounded};
use eframe::egui::{
    self, Align, Align2, Color32, Context, CursorIcon, FontData, FontDefinitions, FontFamily,
    FontId, Margin, Pos2, Rect, RichText, Sense, Shadow, Stroke, TextureHandle, TextureId,
    TextureOptions, Vec2,
};
use rfd::FileDialog;
use serde::{Deserialize, Serialize};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::chat::{CHAT_MODELS, ChatEvent, ChatMessage, ChatRequest, ChatRole, spawn_chat_job};
use crate::layout_roles;
use crate::liquid::{
    DeepLiquidConfig, DocumentProfileKind, LiquidBlock, LiquidBlockRole, LiquidDocument,
    LiquidEvent, LiquidRequest, LiquidSourceLineRef, hidden_contents_mask_for_display,
    should_hide_contents_block_for_display, should_prefer_ocr_page_text, spawn_liquid_job,
};
use crate::liquid2::{
    LiquidMode2Event, LiquidMode2Request, load_fast_cached_liquid_mode2_document,
    spawn_liquid_mode2_job,
};
use crate::model::{
    AnnotationKind, EditorAnnotation, LoadedDocument, MarkerStyle, OcrPageState, PageLink,
    PageTextChar, PdfRect, RenderedPage, SearchHit, SearchSource, Tool,
};
use crate::ocr::{
    OcrEvent, load_ocr_cache, save_ocr_cache, spawn_ocr_job, spawn_openrouter_ocr_save_job,
};
use crate::pdf_backend::{
    LAWPDF_COMMENT_ID_PREFIX, export_text, load_lawpdf_annotations, save_with_annotations,
    sidecar_path_for_export,
};
use crate::render_worker::{
    PageRenderKey, RenderEvent, RenderRequest, ThumbnailRenderKey, spawn_render_worker,
};
use crate::settings::{
    AppSettings, app_data_dir, effective_groq_api_key, effective_openai_api_key,
    effective_openrouter_api_key, load_settings, normalized_pdf_zoom, save_settings,
};
use crate::text_conversion;
use crate::tts::{PaidTtsEvent, PaidTtsProvider, PaidTtsRequest, spawn_paid_tts_job};
use crate::updater::{self, UpdateEvent};

const CANVAS_FILL: Color32 = Color32::from_rgb(232, 230, 224);
const PANEL_FILL: Color32 = Color32::from_rgb(246, 244, 239);
const BAR_FILL: Color32 = Color32::from_rgb(250, 249, 245);
const PAPER_FILL: Color32 = Color32::from_rgb(255, 254, 250);
const PAPER_STROKE: Color32 = Color32::from_rgb(205, 201, 192);
const INK: Color32 = Color32::from_rgb(42, 38, 32);
const MUTED_INK: Color32 = Color32::from_rgb(105, 99, 90);
const RENDER_POLL_INTERVAL: Duration = Duration::from_millis(16);
const ZOOM_RENDER_DEBOUNCE: Duration = Duration::from_millis(180);
const ZOOM_ANIMATION_EPSILON: f32 = 0.001;
const ZOOM_ANIMATION_SPEED: f32 = 18.0;
const MAX_LIQUID_OUTLINE_ITEMS: usize = 80;
const THUMBNAIL_SCROLL_SECONDS: f32 = 0.28;
const DOCUMENT_PAGE_GAP: f32 = 24.0;
const PAGE_PREFETCH_RADIUS: usize = 3;
const PAGE_TEXTURE_CACHE_CAP: usize = 32;
const SMALL_DOCUMENT_PREFETCH_LIMIT: usize = 6;
const UPDATE_CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
const UPDATE_RETRY_INTERVAL: Duration = Duration::from_secs(30 * 60);
const UPDATE_NOTICE_DURATION: Duration = Duration::from_secs(4);
const NOTICE_CAPACITY: usize = 5;
const INFO_NOTICE_DURATION: Duration = Duration::from_secs(6);
const COMMENT_AUTOSAVE_DELAY: Duration = Duration::from_millis(650);
const COMMENT_CARD_WIDTH: f32 = 324.0;
const COMMENT_CARD_GAP: f32 = 14.0;
const LIQUID_MARGIN_NOTE_GAP: f32 = 16.0;
const LIQUID_MARGIN_NOTE_MAX_CHARS: usize = 260;
// "Laying-down ink" highlight animation. The stroke wipes on left-to-right in
// reading direction, then a brief sheen relaxes as the ink "dries". Multi-line
// selections stagger so a paragraph fills like a hand sweeping down the page.
const MARKER_WIPE_DURATION: Duration = Duration::from_millis(260);
const MARKER_SETTLE_DURATION: Duration = Duration::from_millis(160);
const MARKER_STAGGER: Duration = Duration::from_millis(65);
const MARKER_CORNER_RADIUS: u8 = 2;
// Settled highlights stay gently alive: a long, low-amplitude opacity drift and
// an occasional faint sheen. Each mark gets its own deterministic phase/cycle,
// so a page never pulses in unison. Disabled under reduced motion.
const MARKER_BREATH_RATE: f32 = 0.66;
const MARKER_BREATH_AMPLITUDE: f32 = 0.045;
const MARKER_SHEEN_CYCLE_BASE: f32 = 12.0;
const MARKER_SHEEN_CYCLE_VARIANCE: f32 = 5.0;
const MARKER_SHEEN_ALPHA: f32 = 0.11;
const MARKER_BREATH_REPAINT: Duration = Duration::from_millis(80);
const MARKER_PRESETS: [MarkerPreset; 6] = [
    MarkerPreset {
        label: "Yellow",
        color_rgb: [1.0, 0.93, 0.45],
        opacity: 0.42,
        style: MarkerStyle::Highlight,
    },
    MarkerPreset {
        label: "Pink",
        color_rgb: [1.0, 0.72, 0.80],
        opacity: 0.38,
        style: MarkerStyle::Highlight,
    },
    MarkerPreset {
        label: "Mint",
        color_rgb: [0.68, 0.92, 0.75],
        opacity: 0.38,
        style: MarkerStyle::Highlight,
    },
    MarkerPreset {
        label: "Blue",
        color_rgb: [0.70, 0.84, 1.0],
        opacity: 0.38,
        style: MarkerStyle::Highlight,
    },
    MarkerPreset {
        label: "Lavender",
        color_rgb: [0.82, 0.76, 1.0],
        opacity: 0.38,
        style: MarkerStyle::Highlight,
    },
    MarkerPreset {
        label: "Crimson underline",
        color_rgb: [0.72, 0.05, 0.14],
        opacity: 1.0,
        style: MarkerStyle::Underline,
    },
];
const COMMENT_COLOR_PRESETS: [CommentColorPreset; 5] = [
    CommentColorPreset {
        label: "Amber",
        color_rgb: [1.0, 0.78, 0.28],
    },
    CommentColorPreset {
        label: "Rose",
        color_rgb: [1.0, 0.56, 0.64],
    },
    CommentColorPreset {
        label: "Sky",
        color_rgb: [0.46, 0.70, 1.0],
    },
    CommentColorPreset {
        label: "Mint",
        color_rgb: [0.42, 0.78, 0.58],
    },
    CommentColorPreset {
        label: "Violet",
        color_rgb: [0.66, 0.58, 1.0],
    },
];

fn install_eb_garamond(ctx: &Context) {
    let Some(font_bytes) = eb_garamond_candidates()
        .into_iter()
        .find_map(|path| std::fs::read(path).ok())
    else {
        return;
    };

    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "EB Garamond".to_owned(),
        Arc::new(FontData::from_owned(font_bytes)),
    );
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "EB Garamond".to_owned());
    ctx.set_fonts(fonts);
}

fn eb_garamond_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            candidates.push(exe_dir.join("fonts").join("EBGaramond.ttf"));
            candidates.push(exe_dir.join("vendor").join("fonts").join("EBGaramond.ttf"));
        }
    }
    if let Ok(current_dir) = std::env::current_dir() {
        candidates.push(current_dir.join("fonts").join("EBGaramond.ttf"));
        candidates.push(
            current_dir
                .join("vendor")
                .join("fonts")
                .join("EBGaramond.ttf"),
        );
    }
    candidates.push(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("vendor")
            .join("fonts")
            .join("EBGaramond.ttf"),
    );
    candidates
}

fn install_paper_theme(ctx: &Context) {
    let mut style = (*ctx.style()).clone();
    style.visuals = egui::Visuals::light();
    style.visuals.panel_fill = PANEL_FILL;
    style.visuals.window_fill = BAR_FILL;
    style.visuals.extreme_bg_color = Color32::from_rgb(238, 236, 231);
    style.visuals.faint_bg_color = Color32::from_rgb(235, 232, 225);
    style.visuals.widgets.noninteractive.fg_stroke.color = INK;
    style.visuals.widgets.inactive.fg_stroke.color = INK;
    style.visuals.widgets.hovered.fg_stroke.color = INK;
    style.visuals.widgets.active.fg_stroke.color = INK;
    style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(242, 239, 232);
    style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(232, 227, 216);
    style.visuals.widgets.active.bg_fill = Color32::from_rgb(220, 214, 202);
    style.visuals.selection.bg_fill = Color32::from_rgb(185, 143, 84);
    style.scroll_animation = egui::style::ScrollAnimation::duration(THUMBNAIL_SCROLL_SECONDS);
    style.spacing.scroll.bar_width = 13.0;
    style.spacing.scroll.floating_width = 3.5;
    style.spacing.scroll.active_handle_opacity = 0.75;
    style.spacing.scroll.interact_handle_opacity = 0.95;
    style.spacing.item_spacing = Vec2::new(8.0, 8.0);
    style.spacing.button_padding = Vec2::new(9.0, 5.0);
    ctx.set_style(style);
}

pub struct PdfEditorApp {
    startup_error: Option<String>,
    tabs: Vec<DocumentTab>,
    active_tab: Option<usize>,
    next_document_epoch: u64,
    document: Option<LoadedDocument>,
    page_index: usize,
    document_epoch: u64,
    view_mode: DocumentViewMode,
    liquid_state: LiquidState,
    liquid_mode2_state: LiquidState,
    liquid_notice_dismissed: bool,
    liquid_text_scale: f32,
    liquid_max_width: f32,
    liquid_theme: LiquidTheme,
    liquid_tx: Sender<LiquidEvent>,
    liquid_rx: Receiver<LiquidEvent>,
    liquid_mode2_tx: Sender<LiquidMode2Event>,
    liquid_mode2_rx: Receiver<LiquidMode2Event>,
    chat_ui: ChatUi,
    update_ui: UpdateUi,
    notices: VecDeque<Notice>,
    zoom: f32,
    target_zoom: f32,
    page_textures: HashMap<usize, PageTexture>,
    thumbnail_textures: HashMap<usize, ThumbnailTexture>,
    render_tx: Sender<RenderRequest>,
    render_rx: Receiver<RenderEvent>,
    incoming_paths_rx: Receiver<Vec<PathBuf>>,
    queued_open_paths: VecDeque<PathBuf>,
    pending_page_renders: HashMap<usize, PageRenderKey>,
    pending_thumbnail_renders: HashMap<usize, ThumbnailRenderKey>,
    pending_native_text: HashSet<usize>,
    pending_text_chars: HashSet<usize>,
    texture_access_counter: u64,
    last_zoom_change: Option<Instant>,
    annotations: Vec<EditorAnnotation>,
    annotations_dirty: bool,
    liquid_feedback: Vec<LiquidFeedback>,
    editing_liquid_feedback: Option<String>,
    /// #27 footnote popovers: body superscript marker number -> footnote text,
    /// rebuilt each frame from the rendered document's Footnote/Marginalia blocks.
    liquid_footnote_index: HashMap<u16, String>,
    /// #31 outline navigation: pending scroll target (heading level, compacted text)
    /// set when an outline entry is clicked; consumed by the matching heading block.
    liquid_scroll_to_heading: Option<(usize, String)>,
    /// #29 provenance dual-view: source bboxes (per page) to highlight in the
    /// fixed-layout view after the reader ⌘-clicks a reflowed block. Empty = none.
    liquid_provenance_highlight: Vec<(usize, PdfRect)>,
    /// #29 show-hidden-furniture toggle: when true, the reader renders the blocks
    /// normally hidden for display (headers/footers/TOC/noise/tables) as dimmed,
    /// role-tagged lines instead of dropping them.
    liquid_show_hidden_furniture: bool,
    tts_controller: TtsController,
    marker_animations: Vec<MarkerAnim>,
    active_tool: Tool,
    sidebar_tab: SidebarTab,
    selection_state: SelectionState,
    selected_text_box: Option<usize>,
    editing_text_box: Option<usize>,
    text_box_focus_request: Option<usize>,
    text_box_action_rect: Option<Rect>,
    text_box_drag: Option<TextBoxDrag>,
    selected_comment: Option<usize>,
    editing_comment: Option<usize>,
    comment_focus_request: Option<usize>,
    comment_action_rect: Option<Rect>,
    comment_drag: Option<CommentDrag>,
    active_drag_page: Option<usize>,
    drag_start_pdf: Option<(f32, f32)>,
    drag_preview: Option<PdfRect>,
    active_signature_stroke: Vec<(f32, f32)>,
    context_menu_pdf: Option<(usize, (f32, f32))>,
    marker_opacity: f32,
    marker_preset_index: usize,
    comment_color_index: usize,
    pending_comment_saves: HashMap<PathBuf, PendingCommentSave>,
    active_comment_saves: HashMap<PathBuf, u64>,
    text_box_text: String,
    signer_name: String,
    search_state: SearchState,
    ocr_states: Vec<OcrPageState>,
    ocr_progress: Option<OcrProgress>,
    ocr_tx: Sender<OcrEvent>,
    ocr_rx: Receiver<OcrEvent>,
    scroll_target_page: Option<usize>,
    thumbnail_scroll_target: Option<usize>,
    pending_document_scroll_offset: Option<Vec2>,
    visible_page_ranges: Vec<VisiblePageRange>,
    settings: AppSettings,
    settings_ui: SettingsUi,
    status: String,
    show_unsaved_close_prompt: bool,
    allow_window_close: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TextSelection {
    start_page: usize,
    start: usize,
    end_page: usize,
    end: usize,
}

impl TextSelection {
    fn new(page_index: usize, start: usize, end: usize) -> Self {
        Self::range(page_index, start, page_index, end)
    }

    fn range(start_page: usize, start: usize, end_page: usize, end: usize) -> Self {
        if (start_page, start) <= (end_page, end) {
            Self {
                start_page,
                start,
                end_page,
                end,
            }
        } else {
            Self {
                start_page: end_page,
                start: end,
                end_page: start_page,
                end: start,
            }
        }
    }

    fn contains(self, page_index: usize) -> bool {
        self.start_page <= page_index && page_index <= self.end_page
    }

    fn action_page(self) -> usize {
        self.start_page
    }

    fn page_range(self) -> std::ops::RangeInclusive<usize> {
        self.start_page..=self.end_page
    }

    fn bounds_for_page(self, page_index: usize, page_len: usize) -> Option<(usize, usize)> {
        if !self.contains(page_index) || page_len == 0 {
            return None;
        }

        let last = page_len.saturating_sub(1);
        let start = if page_index == self.start_page {
            self.start.min(last)
        } else {
            0
        };
        let end = if page_index == self.end_page {
            self.end.min(last)
        } else {
            last
        };

        (start <= end).then_some((start, end))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DocumentViewMode {
    Pdf,
    Liquid,
    LiquidMode2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum LiquidTheme {
    #[default]
    Paper,
    Sepia,
    Dark,
}

#[derive(Debug, Clone)]
enum LiquidState {
    Idle,
    PreparingText,
    Preparing,
    Ready(LiquidDocument),
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LiquidOutlineItem {
    level: usize,
    text: String,
}

impl LiquidState {
    fn label(&self) -> &'static str {
        match self {
            Self::Idle => "Liquid idle",
            Self::PreparingText => "Liquid text preparing",
            Self::Preparing => "Liquid preparing",
            Self::Ready(_) => "Liquid ready",
            Self::Failed(_) => "Liquid failed",
        }
    }
}

#[derive(Debug, Clone)]
enum UpdateUiState {
    Idle,
    Checking,
    Downloading,
    Ready,
    Failed { shown_at: Instant },
}

impl UpdateUiState {
    fn is_busy(&self) -> bool {
        matches!(self, Self::Checking | Self::Downloading)
    }

    fn has_ready_update(&self) -> bool {
        matches!(self, Self::Ready)
    }
}

#[derive(Debug, Clone)]
struct UpdateNotice {
    message: String,
    kind: UpdateNoticeKind,
    shown_at: Instant,
    duration: Duration,
}

impl UpdateNotice {
    fn new(message: impl Into<String>, kind: UpdateNoticeKind) -> Self {
        Self {
            message: message.into(),
            kind,
            shown_at: Instant::now(),
            duration: UPDATE_NOTICE_DURATION,
        }
    }

    fn is_expired(&self) -> bool {
        self.shown_at.elapsed() >= self.duration
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpdateNoticeKind {
    Working,
    Success,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NoticeSeverity {
    Info,
    Error,
}

#[derive(Debug, Clone)]
struct Notice {
    message: String,
    severity: NoticeSeverity,
    created_at: Instant,
}

impl Notice {
    fn new(message: impl Into<String>, severity: NoticeSeverity) -> Self {
        Self {
            message: message.into(),
            severity,
            created_at: Instant::now(),
        }
    }

    fn is_expired_at(&self, now: Instant) -> bool {
        self.severity == NoticeSeverity::Info
            && now.saturating_duration_since(self.created_at) >= INFO_NOTICE_DURATION
    }
}

fn enqueue_notice(notices: &mut VecDeque<Notice>, notice: Notice) {
    while notices.len() >= NOTICE_CAPACITY {
        notices.pop_front();
    }
    notices.push_back(notice);
}

fn prune_notices_at(notices: &mut VecDeque<Notice>, now: Instant) {
    notices.retain(|notice| !notice.is_expired_at(now));
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SidebarTab {
    Pages,
    Outline,
    Search,
    Chat,
    Notes,
}

impl SidebarTab {
    const ALL: [Self; 5] = [
        Self::Pages,
        Self::Outline,
        Self::Search,
        Self::Chat,
        Self::Notes,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Pages => "Pages",
            Self::Outline => "Outline",
            Self::Search => "Search",
            Self::Chat => "Chat",
            Self::Notes => "Notes",
        }
    }
}

#[derive(Clone)]
struct PageTexture {
    _zoom: f32,
    render_scale: f32,
    last_used: u64,
    texture: TextureHandle,
}

#[derive(Clone)]
struct ThumbnailTexture {
    texture: TextureHandle,
}

#[derive(Clone, Copy)]
struct ThumbnailTextureView {
    texture_id: TextureId,
}

#[derive(Clone, Copy)]
struct PageTextureView {
    texture_id: TextureId,
}

#[derive(Debug, Clone, Copy)]
struct PagePlacement {
    rect: Rect,
    page_width: f32,
    page_height: f32,
}

#[derive(Debug, Clone, Copy)]
struct TextBoxDrag {
    annotation_index: usize,
    start_pdf: (f32, f32),
    original_rect: PdfRect,
}

#[derive(Debug, Clone, Copy)]
struct CommentDrag {
    annotation_index: usize,
    start_pdf: (f32, f32),
    original_rect: PdfRect,
}

#[derive(Debug, Clone, Copy)]
struct MarkerPreset {
    label: &'static str,
    color_rgb: [f32; 3],
    opacity: f32,
    style: MarkerStyle,
}

/// Transient state driving a single highlight's "laying-down" animation. Keyed
/// by page + rect rather than annotation index so it survives annotation
/// removal/reordering; entries are pruned once the animation finishes.
#[derive(Debug, Clone, Copy)]
struct MarkerAnim {
    page_index: usize,
    rect: PdfRect,
    born: Instant,
    delay: Duration,
}

#[derive(Debug, Clone, Copy)]
struct CommentColorPreset {
    label: &'static str,
    color_rgb: [f32; 3],
}

#[derive(Debug, Clone)]
struct PendingCommentSave {
    document_epoch: u64,
    path: PathBuf,
    generation: u64,
    comments: Vec<EditorAnnotation>,
    due_at: Instant,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LiquidFeedback {
    id: String,
    document_path: PathBuf,
    document_title: String,
    source_signature: String,
    block_index: usize,
    original_role: LiquidBlockRole,
    expected_role: Option<LiquidBlockRole>,
    block_text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    source_lines: Vec<LiquidSourceLineRef>,
    note: String,
    created_at: String,
    updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    submitted_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LiquidFeedbackFile {
    document_path: PathBuf,
    document_title: String,
    entries: Vec<LiquidFeedback>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LiquidRetrainQueue {
    created_at: String,
    entries: Vec<LiquidFeedback>,
}

#[derive(Debug, Clone, Copy)]
struct VisiblePageRange {
    page_index: usize,
    top_fraction: f32,
    bottom_fraction: f32,
    coverage: f32,
}

#[derive(Debug, Clone, Copy)]
struct OcrProgress {
    started_at: Instant,
    initial_completed: usize,
}

#[derive(Clone)]
struct DocumentTab {
    document: LoadedDocument,
    page_index: usize,
    document_epoch: u64,
    view_mode: DocumentViewMode,
    liquid_state: LiquidState,
    liquid_mode2_state: LiquidState,
    liquid_notice_dismissed: bool,
    zoom: f32,
    target_zoom: f32,
    page_textures: HashMap<usize, PageTexture>,
    thumbnail_textures: HashMap<usize, ThumbnailTexture>,
    texture_access_counter: u64,
    last_zoom_change: Option<Instant>,
    annotations: Vec<EditorAnnotation>,
    annotations_dirty: bool,
    liquid_feedback: Vec<LiquidFeedback>,
    editing_liquid_feedback: Option<String>,
    text_selection: Option<TextSelection>,
    liquid_all_selected: bool,
    selection_anchor: Option<(usize, usize)>,
    selection_toolbar_rect: Option<Rect>,
    selected_text_box: Option<usize>,
    editing_text_box: Option<usize>,
    text_box_focus_request: Option<usize>,
    text_box_action_rect: Option<Rect>,
    text_box_drag: Option<TextBoxDrag>,
    selected_comment: Option<usize>,
    editing_comment: Option<usize>,
    comment_focus_request: Option<usize>,
    comment_action_rect: Option<Rect>,
    comment_drag: Option<CommentDrag>,
    active_drag_page: Option<usize>,
    drag_start_pdf: Option<(f32, f32)>,
    drag_preview: Option<PdfRect>,
    active_signature_stroke: Vec<(f32, f32)>,
    pending_select_all_text: bool,
    search_query: String,
    search_hits: Vec<SearchHit>,
    selected_hit: Option<usize>,
    show_search_highlights: bool,
    ocr_states: Vec<OcrPageState>,
    ocr_progress: Option<OcrProgress>,
    chat_state: ChatState,
    scroll_target_page: Option<usize>,
    thumbnail_scroll_target: Option<usize>,
    pending_document_scroll_offset: Option<Vec2>,
    visible_page_ranges: Vec<VisiblePageRange>,
    status: String,
}

impl DocumentTab {
    fn title(&self) -> String {
        tab_title(&self.document)
    }
}

impl PdfEditorApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        startup_paths: Vec<PathBuf>,
        incoming_paths_rx: Receiver<Vec<PathBuf>>,
    ) -> Self {
        install_eb_garamond(&cc.egui_ctx);
        install_paper_theme(&cc.egui_ctx);

        let (ocr_tx, ocr_rx) = unbounded();
        let (render_tx, render_rx) = spawn_render_worker();
        let (liquid_tx, liquid_rx) = unbounded();
        let (liquid_mode2_tx, liquid_mode2_rx) = unbounded();
        let chat_ui = ChatUi::new();
        let (update_tx, update_rx) = unbounded();
        let tts_controller = TtsController::new();
        updater::spawn_update_check(update_tx.clone());
        let update_installed = updater::take_installed_update().is_some();
        let update_notice = update_installed
            .then(|| UpdateNotice::new("Update installed", UpdateNoticeKind::Success));
        let update_ui = UpdateUi::new(update_tx, update_rx, update_notice);
        let initial_status = if update_installed {
            "Update installed."
        } else {
            "Ready"
        };
        let settings = load_settings();
        let initial_zoom = normalized_pdf_zoom(settings.last_pdf_zoom);
        let settings_ui = SettingsUi::new(&settings);
        let mut app = Self {
            startup_error: None,
            tabs: Vec::new(),
            active_tab: None,
            next_document_epoch: 1,
            document: None,
            page_index: 0,
            document_epoch: 0,
            view_mode: DocumentViewMode::Pdf,
            liquid_state: LiquidState::Idle,
            liquid_mode2_state: LiquidState::Idle,
            liquid_notice_dismissed: false,
            liquid_text_scale: 1.0,
            liquid_max_width: 920.0,
            liquid_theme: LiquidTheme::Paper,
            liquid_tx,
            liquid_rx,
            liquid_mode2_tx,
            liquid_mode2_rx,
            chat_ui,
            update_ui,
            notices: VecDeque::new(),
            zoom: initial_zoom,
            target_zoom: initial_zoom,
            page_textures: HashMap::new(),
            thumbnail_textures: HashMap::new(),
            render_tx,
            render_rx,
            incoming_paths_rx,
            queued_open_paths: VecDeque::new(),
            pending_page_renders: HashMap::new(),
            pending_thumbnail_renders: HashMap::new(),
            pending_native_text: HashSet::new(),
            pending_text_chars: HashSet::new(),
            texture_access_counter: 0,
            last_zoom_change: None,
            annotations: Vec::new(),
            annotations_dirty: false,
            liquid_feedback: Vec::new(),
            editing_liquid_feedback: None,
            liquid_footnote_index: HashMap::new(),
            liquid_scroll_to_heading: None,
            liquid_provenance_highlight: Vec::new(),
            liquid_show_hidden_furniture: false,
            tts_controller,
            marker_animations: Vec::new(),
            active_tool: Tool::Select,
            sidebar_tab: SidebarTab::Pages,
            selection_state: SelectionState::default(),
            selected_text_box: None,
            editing_text_box: None,
            text_box_focus_request: None,
            text_box_action_rect: None,
            text_box_drag: None,
            selected_comment: None,
            editing_comment: None,
            comment_focus_request: None,
            comment_action_rect: None,
            comment_drag: None,
            active_drag_page: None,
            drag_start_pdf: None,
            drag_preview: None,
            active_signature_stroke: Vec::new(),
            context_menu_pdf: None,
            marker_opacity: 0.45,
            marker_preset_index: 0,
            comment_color_index: 0,
            pending_comment_saves: HashMap::new(),
            active_comment_saves: HashMap::new(),
            text_box_text: String::new(),
            signer_name: String::new(),
            search_state: SearchState::default(),
            ocr_states: Vec::new(),
            ocr_progress: None,
            ocr_tx,
            ocr_rx,
            scroll_target_page: Some(0),
            thumbnail_scroll_target: Some(0),
            pending_document_scroll_offset: None,
            visible_page_ranges: Vec::new(),
            settings,
            settings_ui,
            status: initial_status.to_owned(),
            show_unsaved_close_prompt: false,
            allow_window_close: false,
        };

        if !startup_paths.is_empty() {
            app.open_paths_in_tabs(startup_paths, &cc.egui_ctx, true);
        }

        app.apply_startup_view_mode(&cc.egui_ctx);

        app
    }

    fn allocate_document_epoch(&mut self) -> u64 {
        let epoch = self.next_document_epoch;
        self.next_document_epoch = self.next_document_epoch.wrapping_add(1).max(1);
        epoch
    }

    fn active_tab_snapshot(&self, document: LoadedDocument) -> DocumentTab {
        DocumentTab {
            document,
            page_index: self.page_index,
            document_epoch: self.document_epoch,
            view_mode: self.view_mode,
            liquid_state: self.liquid_state.clone(),
            liquid_mode2_state: self.liquid_mode2_state.clone(),
            liquid_notice_dismissed: self.liquid_notice_dismissed,
            zoom: self.zoom,
            target_zoom: self.target_zoom,
            page_textures: self.page_textures.clone(),
            thumbnail_textures: self.thumbnail_textures.clone(),
            texture_access_counter: self.texture_access_counter,
            last_zoom_change: self.last_zoom_change,
            annotations: self.annotations.clone(),
            annotations_dirty: self.annotations_dirty,
            liquid_feedback: self.liquid_feedback.clone(),
            editing_liquid_feedback: self.editing_liquid_feedback.clone(),
            text_selection: self.selection_state.text,
            liquid_all_selected: self.selection_state.liquid_all,
            selection_anchor: self.selection_state.anchor,
            selection_toolbar_rect: self.selection_state.toolbar_rect,
            selected_text_box: self.selected_text_box,
            editing_text_box: self.editing_text_box,
            text_box_focus_request: self.text_box_focus_request,
            text_box_action_rect: self.text_box_action_rect,
            text_box_drag: self.text_box_drag,
            selected_comment: self.selected_comment,
            editing_comment: self.editing_comment,
            comment_focus_request: self.comment_focus_request,
            comment_action_rect: self.comment_action_rect,
            comment_drag: self.comment_drag,
            active_drag_page: self.active_drag_page,
            drag_start_pdf: self.drag_start_pdf,
            drag_preview: self.drag_preview,
            active_signature_stroke: self.active_signature_stroke.clone(),
            pending_select_all_text: self.selection_state.pending_select_all,
            search_query: self.search_state.query.clone(),
            search_hits: self.search_state.hits.clone(),
            selected_hit: self.search_state.selected_hit,
            show_search_highlights: self.search_state.show_highlights,
            ocr_states: self.ocr_states.clone(),
            ocr_progress: self.ocr_progress,
            chat_state: self.chat_ui.state.clone(),
            scroll_target_page: self.scroll_target_page,
            thumbnail_scroll_target: self.thumbnail_scroll_target,
            pending_document_scroll_offset: self.pending_document_scroll_offset,
            visible_page_ranges: self.visible_page_ranges.clone(),
            status: self.status.clone(),
        }
    }

    fn save_active_tab_state(&mut self) {
        let Some(tab_index) = self.active_tab else {
            return;
        };
        let Some(document) = self.document.clone() else {
            return;
        };
        let snapshot = self.active_tab_snapshot(document);
        if let Some(tab) = self.tabs.get_mut(tab_index) {
            *tab = snapshot;
        }
    }

    fn apply_tab_state(&mut self, tab: DocumentTab, ctx: &Context) {
        let tab_zoom = normalized_pdf_zoom(tab.zoom);
        let tab_target_zoom = normalized_pdf_zoom(tab.target_zoom);
        self.document = Some(tab.document);
        self.page_index = tab.page_index;
        self.document_epoch = tab.document_epoch;
        self.view_mode = tab.view_mode;
        self.liquid_state = tab.liquid_state;
        self.liquid_mode2_state = tab.liquid_mode2_state;
        self.liquid_notice_dismissed = tab.liquid_notice_dismissed;
        self.zoom = tab_zoom;
        self.target_zoom = tab_target_zoom;
        self.remember_pdf_zoom(tab_target_zoom);
        self.page_textures = tab.page_textures;
        self.thumbnail_textures = tab.thumbnail_textures;
        self.pending_page_renders.clear();
        self.pending_thumbnail_renders.clear();
        self.pending_native_text.clear();
        self.pending_text_chars.clear();
        self.texture_access_counter = tab.texture_access_counter;
        self.last_zoom_change = tab.last_zoom_change;
        self.annotations = tab.annotations;
        self.annotations_dirty = tab.annotations_dirty;
        self.liquid_feedback = tab.liquid_feedback;
        self.editing_liquid_feedback = tab.editing_liquid_feedback;
        self.selection_state.text = tab.text_selection;
        self.selection_state.liquid_all = tab.liquid_all_selected;
        self.selection_state.anchor = tab.selection_anchor;
        self.selection_state.toolbar_rect = tab.selection_toolbar_rect;
        self.selected_text_box = tab.selected_text_box;
        self.editing_text_box = tab.editing_text_box;
        self.text_box_focus_request = tab.text_box_focus_request;
        self.text_box_action_rect = tab.text_box_action_rect;
        self.text_box_drag = tab.text_box_drag;
        self.selected_comment = tab.selected_comment;
        self.editing_comment = tab.editing_comment;
        self.comment_focus_request = tab.comment_focus_request;
        self.comment_action_rect = tab.comment_action_rect;
        self.comment_drag = tab.comment_drag;
        self.active_drag_page = tab.active_drag_page;
        self.drag_start_pdf = tab.drag_start_pdf;
        self.drag_preview = tab.drag_preview;
        self.active_signature_stroke = tab.active_signature_stroke;
        self.selection_state.pending_select_all = tab.pending_select_all_text;
        self.search_state.query = tab.search_query;
        self.search_state.hits = tab.search_hits;
        self.search_state.selected_hit = tab.selected_hit;
        self.search_state.show_highlights = tab.show_search_highlights;
        self.ocr_states = tab.ocr_states;
        self.ocr_progress = tab.ocr_progress;
        self.chat_ui.state = tab.chat_state;
        self.scroll_target_page = tab.scroll_target_page.or(Some(self.page_index));
        self.thumbnail_scroll_target = tab.thumbnail_scroll_target.or(Some(self.page_index));
        self.pending_document_scroll_offset = tab.pending_document_scroll_offset;
        self.visible_page_ranges = tab.visible_page_ranges;
        self.status = tab.status;
        ctx.request_repaint();
    }

    fn switch_to_tab(&mut self, tab_index: usize, ctx: &Context) {
        if self.active_tab == Some(tab_index) || tab_index >= self.tabs.len() {
            return;
        }

        self.save_active_tab_state();
        let tab = self.tabs[tab_index].clone();
        self.active_tab = Some(tab_index);
        self.startup_error = None;
        self.apply_tab_state(tab, ctx);
    }

    fn close_tab(&mut self, tab_index: usize, ctx: &Context) {
        if tab_index >= self.tabs.len() {
            return;
        }

        let closing_active = self.active_tab == Some(tab_index);
        if !closing_active {
            self.save_active_tab_state();
        }

        self.tabs.remove(tab_index);
        if self.tabs.is_empty() {
            self.active_tab = None;
            self.clear_document_state();
            ctx.request_repaint();
            return;
        }

        if closing_active {
            let next_index = tab_index.min(self.tabs.len() - 1);
            let tab = self.tabs[next_index].clone();
            self.active_tab = Some(next_index);
            self.apply_tab_state(tab, ctx);
        } else if let Some(active_tab) = self.active_tab {
            self.active_tab = Some(if active_tab > tab_index {
                active_tab - 1
            } else {
                active_tab
            });
        }
    }

    fn clear_document_state(&mut self) {
        let zoom = self.default_zoom_for_new_document();
        self.document = None;
        self.page_index = 0;
        self.document_epoch = 0;
        self.view_mode = DocumentViewMode::Pdf;
        self.stop_liquid_tts();
        self.liquid_state = LiquidState::Idle;
        self.liquid_mode2_state = LiquidState::Idle;
        self.liquid_notice_dismissed = false;
        self.zoom = zoom;
        self.target_zoom = zoom;
        self.page_textures.clear();
        self.thumbnail_textures.clear();
        self.pending_page_renders.clear();
        self.pending_thumbnail_renders.clear();
        self.pending_native_text.clear();
        self.pending_text_chars.clear();
        self.texture_access_counter = 0;
        self.last_zoom_change = None;
        self.annotations.clear();
        self.annotations_dirty = false;
        self.liquid_feedback.clear();
        self.editing_liquid_feedback = None;
        self.selection_state.text = None;
        self.selection_state.liquid_all = false;
        self.selection_state.anchor = None;
        self.selection_state.toolbar_rect = None;
        self.clear_text_box_selection();
        self.text_box_drag = None;
        self.clear_comment_selection();
        self.comment_drag = None;
        self.active_drag_page = None;
        self.drag_start_pdf = None;
        self.drag_preview = None;
        self.active_signature_stroke.clear();
        self.selection_state.pending_select_all = false;
        self.search_state = SearchState::default();
        self.ocr_states.clear();
        self.ocr_progress = None;
        self.chat_ui.state = ChatState::default();
        self.scroll_target_page = Some(0);
        self.thumbnail_scroll_target = Some(0);
        self.visible_page_ranges.clear();
        self.status = "Ready".to_owned();
    }

    fn tab_index_for_path(&self, path: &Path) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.document.path == path)
    }

    fn pick_save_path(
        &self,
        title: &str,
        file_name: &str,
        filter_name: &str,
        extensions: &[&str],
    ) -> Option<PathBuf> {
        FileDialog::new()
            .set_title(title)
            .add_filter(filter_name, extensions)
            .set_file_name(file_name)
            .save_file()
    }

    fn open_dialog(&mut self, ctx: &Context) {
        if let Some(paths) = FileDialog::new()
            .add_filter(
                "Documents",
                &[
                    "pdf", "docx", "md", "markdown", "txt", "text", "log", "csv", "json",
                ],
            )
            .add_filter("PDF", &["pdf"])
            .add_filter(
                "Convertible text",
                &[
                    "docx", "md", "markdown", "txt", "text", "log", "csv", "json",
                ],
            )
            .pick_files()
        {
            self.open_paths_in_tabs(paths, ctx, true);
        }
    }

    fn open_paths_in_tabs(&mut self, paths: Vec<PathBuf>, ctx: &Context, defer_background: bool) {
        let (mut paths, converted, conversion_errors) = prepare_open_paths(paths);
        if !conversion_errors.is_empty() {
            self.status = conversion_errors.join("; ");
        } else if converted > 0 {
            self.status = format!("Converted {converted} document(s) to PDF.");
        }
        if paths.is_empty() {
            return;
        }

        let total = paths.len();
        let first = paths.remove(0);
        self.load_document_with_options(first, ctx, true, true, total == 1);

        if defer_background {
            self.queued_open_paths.extend(paths);
            if total > 1 {
                self.status = format!("Opening {total} PDFs in tabs...");
                ctx.request_repaint_after(RENDER_POLL_INTERVAL);
            }
            return;
        }

        let mut opened = 1;
        for path in paths {
            if self.load_document_with_options(path, ctx, false, false, false) {
                opened += 1;
            }
        }
        if opened > 1 {
            self.status = format!("Opened {opened} PDFs in tabs.");
        }
    }

    fn poll_queued_open_paths(&mut self, ctx: &Context) {
        let Some(path) = self.queued_open_paths.pop_front() else {
            return;
        };

        self.load_document_with_options(path, ctx, false, false, false);
        let remaining = self.queued_open_paths.len();
        if remaining == 0 {
            self.prefetch_small_document_pages(ctx);
            self.status = format!("Opened {} PDFs in tabs.", self.tabs.len());
        } else {
            self.status = format!("Opening PDFs in tabs... {remaining} remaining");
            ctx.request_repaint_after(RENDER_POLL_INTERVAL);
        }
    }

    fn poll_incoming_paths(&mut self, ctx: &Context) {
        let mut saw_message = false;
        let mut paths = Vec::new();
        while let Ok(mut batch) = self.incoming_paths_rx.try_recv() {
            saw_message = true;
            paths.append(&mut batch);
        }

        if !saw_message {
            return;
        }

        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(
            egui::UserAttentionType::Informational,
        ));

        if paths.is_empty() {
            self.status = "LawPDF is already open.".to_owned();
            ctx.request_repaint();
        } else {
            self.open_paths_in_tabs(paths, ctx, true);
        }
    }

    fn poll_update_events(&mut self, ctx: &Context) {
        while let Ok(event) = self.update_ui.rx.try_recv() {
            match event {
                UpdateEvent::Checking => {
                    self.update_ui.check_in_flight = true;
                    if !self.update_ui.state.has_ready_update() {
                        self.update_ui.state = UpdateUiState::Checking;
                    }
                }
                UpdateEvent::Detected { version } => {
                    self.update_ui.check_in_flight = true;
                    self.update_ui.state = UpdateUiState::Downloading;
                    self.update_ui.notice = Some(UpdateNotice::new(
                        "New update detected, updating in background",
                        UpdateNoticeKind::Working,
                    ));
                    self.status = format!("Downloading LawPDF {version} in the background.");
                    ctx.request_repaint();
                }
                UpdateEvent::NotAvailable => {
                    self.update_ui.check_in_flight = false;
                    self.update_ui.next_check = Some(Instant::now() + UPDATE_CHECK_INTERVAL);
                    if matches!(self.update_ui.state, UpdateUiState::Checking) {
                        self.update_ui.state = UpdateUiState::Idle;
                    }
                }
                UpdateEvent::Downloading => {
                    self.update_ui.check_in_flight = true;
                    self.update_ui.state = UpdateUiState::Downloading;
                }
                UpdateEvent::Ready(pending) => {
                    self.update_ui.check_in_flight = false;
                    self.update_ui.next_check = Some(Instant::now() + UPDATE_CHECK_INTERVAL);
                    self.status = format!(
                        "LawPDF {} is ready and will install on next launch.",
                        pending.version
                    );
                    self.update_ui.state = UpdateUiState::Ready;
                    ctx.request_repaint();
                }
                UpdateEvent::Failed(message) => {
                    self.update_ui.check_in_flight = false;
                    self.update_ui.next_check = Some(Instant::now() + UPDATE_RETRY_INTERVAL);
                    self.push_error_notice(message.clone());
                    if matches!(
                        self.update_ui.state,
                        UpdateUiState::Checking | UpdateUiState::Downloading
                    ) {
                        self.update_ui.state = UpdateUiState::Failed {
                            shown_at: Instant::now(),
                        };
                    }
                }
            }
        }

        if let UpdateUiState::Failed { shown_at, .. } = &self.update_ui.state {
            if shown_at.elapsed() < Duration::from_secs(12) {
                ctx.request_repaint_after(Duration::from_secs(1));
            } else {
                self.update_ui.state = UpdateUiState::Idle;
            }
        }

        if self
            .update_ui
            .notice
            .as_ref()
            .is_some_and(UpdateNotice::is_expired)
        {
            self.update_ui.notice = None;
            ctx.request_repaint();
        } else if self.update_ui.notice.is_some() {
            ctx.request_repaint_after(Duration::from_millis(250));
        }

        if !self.update_ui.check_in_flight
            && !self.update_ui.state.is_busy()
            && !self.update_ui.state.has_ready_update()
            && self
                .update_ui
                .next_check
                .is_some_and(|next_check| Instant::now() >= next_check)
        {
            self.update_ui.check_in_flight = true;
            self.update_ui.next_check = None;
            updater::spawn_update_check(self.update_ui.tx.clone());
        }

        if matches!(self.update_ui.state, UpdateUiState::Downloading) {
            ctx.request_repaint_after(RENDER_POLL_INTERVAL);
        }
    }

    fn load_document_with_options(
        &mut self,
        path: PathBuf,
        ctx: &Context,
        activate: bool,
        render_first_page: bool,
        prefetch_pages: bool,
    ) -> bool {
        if let Some(tab_index) = self.tab_index_for_path(&path) {
            if activate {
                self.switch_to_tab(tab_index, ctx);
                self.status = format!("Switched to {}", path.display());
            } else {
                self.status = format!("Already open {}", path.display());
            }
            return true;
        }

        let result = self.load_document_on_worker(path.clone());

        match result {
            Ok(document) => {
                let title = document.title.clone();
                let tab = self.tab_for_new_document(document);
                let should_activate = activate || self.active_tab.is_none();
                if should_activate {
                    self.save_active_tab_state();
                }
                self.startup_error = None;
                self.tabs.push(tab);
                let tab_index = self.tabs.len() - 1;
                if should_activate {
                    let tab = self.tabs[tab_index].clone();
                    self.active_tab = Some(tab_index);
                    self.apply_tab_state(tab, ctx);
                    if render_first_page {
                        self.render_first_page_before_repaint(ctx);
                    }
                    if prefetch_pages {
                        self.prefetch_small_document_pages(ctx);
                    }
                } else {
                    self.status = format!("Added {title} to tabs");
                }
                ctx.request_repaint();
                true
            }
            Err(error) => {
                self.startup_error = Some(error.clone());
                self.visible_page_ranges.clear();
                self.push_error_notice(error);
                false
            }
        }
    }

    fn tab_for_new_document(&mut self, document: LoadedDocument) -> DocumentTab {
        let page_count = document.page_count;
        let title = document.title.clone();
        let zoom = self.default_zoom_for_new_document();
        let ocr_states = load_ocr_cache(&document.path, page_count)
            .unwrap_or_else(|| vec![OcrPageState::Idle; page_count]);
        let annotations = load_lawpdf_annotations(&document.path).unwrap_or_default();
        let liquid_feedback = load_liquid_feedback(&document.path).unwrap_or_default();
        let comment_count = annotations
            .iter()
            .filter(|annotation| matches!(annotation.kind, AnnotationKind::Comment { .. }))
            .count();
        let feedback_count = liquid_feedback
            .iter()
            .filter(|entry| entry.submitted_at.is_none())
            .count();
        let cached_ocr_pages = ocr_states
            .iter()
            .filter(|state| matches!(state, OcrPageState::Done(_)))
            .count();
        DocumentTab {
            document,
            page_index: 0,
            document_epoch: self.allocate_document_epoch(),
            view_mode: DocumentViewMode::Pdf,
            liquid_state: LiquidState::Idle,
            liquid_mode2_state: LiquidState::Idle,
            liquid_notice_dismissed: false,
            zoom,
            target_zoom: zoom,
            page_textures: HashMap::new(),
            thumbnail_textures: HashMap::new(),
            texture_access_counter: 0,
            last_zoom_change: None,
            annotations,
            annotations_dirty: false,
            liquid_feedback,
            editing_liquid_feedback: None,
            text_selection: None,
            liquid_all_selected: false,
            selection_anchor: None,
            selection_toolbar_rect: None,
            selected_text_box: None,
            editing_text_box: None,
            text_box_focus_request: None,
            text_box_action_rect: None,
            text_box_drag: None,
            selected_comment: None,
            editing_comment: None,
            comment_focus_request: None,
            comment_action_rect: None,
            comment_drag: None,
            active_drag_page: None,
            drag_start_pdf: None,
            drag_preview: None,
            active_signature_stroke: Vec::new(),
            pending_select_all_text: false,
            search_query: String::new(),
            search_hits: Vec::new(),
            selected_hit: None,
            show_search_highlights: true,
            ocr_states,
            ocr_progress: None,
            chat_state: ChatState::default(),
            scroll_target_page: Some(0),
            thumbnail_scroll_target: Some(0),
            pending_document_scroll_offset: None,
            visible_page_ranges: Vec::new(),
            status: document_opened_status(&title, cached_ocr_pages, comment_count, feedback_count),
        }
    }

    fn save_as_dialog(&mut self) {
        let Some(document) = self.document.as_ref() else {
            return;
        };

        let file_name = default_output_name(&document.path, "edited", "pdf");
        if let Some(destination) =
            self.pick_save_path("Save edited PDF", &file_name, "PDF", &["pdf"])
        {
            match save_with_annotations(&document.path, &destination, &self.annotations) {
                Ok(()) => self.status = format!("Saved {}", destination.display()),
                Err(error) => self.push_error_notice(error.to_string()),
            }
        }
    }

    fn save_current_annotations(&mut self) -> Result<(), String> {
        let Some(document) = self.document.as_ref() else {
            return Ok(());
        };
        let path = document.path.clone();
        save_with_annotations(&path, &path, &self.annotations)
            .map_err(|error| error.to_string())?;
        self.annotations_dirty = false;
        self.pending_comment_saves.remove(&path);
        self.active_comment_saves.remove(&path);
        if let Some(tab_index) = self.active_tab
            && let Some(tab) = self.tabs.get_mut(tab_index)
        {
            tab.annotations = self.annotations.clone();
            tab.annotations_dirty = false;
        }
        self.status = format!("Saved annotations to {}", path.display());
        Ok(())
    }

    fn save_all_dirty_annotations(&mut self) -> Result<(), String> {
        self.save_active_tab_state();
        for tab in &mut self.tabs {
            if !tab.annotations_dirty {
                continue;
            }
            let path = tab.document.path.clone();
            save_with_annotations(&path, &path, &tab.annotations)
                .map_err(|error| format!("Could not save {}: {error}", path.display()))?;
            tab.annotations_dirty = false;
            self.pending_comment_saves.remove(&path);
            self.active_comment_saves.remove(&path);
        }
        if let Some(tab_index) = self.active_tab {
            self.annotations_dirty = self
                .tabs
                .get(tab_index)
                .is_some_and(|tab| tab.annotations_dirty);
        }
        self.status = "Saved annotations to PDF.".to_owned();
        Ok(())
    }

    fn has_unsaved_annotations(&self) -> bool {
        self.annotations_dirty || self.tabs.iter().any(|tab| tab.annotations_dirty)
    }

    fn export_text_dialog(&mut self, ctx: &Context) {
        if self.document.is_none() {
            return;
        }

        if !self.ensure_native_text_loaded_for_all(ctx, "Preparing PDF text for export") {
            self.status = "Preparing PDF text for export; export again when ready.".to_owned();
            return;
        }

        let Some(document) = self.document.as_ref() else {
            return;
        };

        let default = sidecar_path_for_export(&document.path, "text", "txt");
        let file_name = default
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("document-text.txt");

        if let Some(destination) =
            self.pick_save_path("Export document text", file_name, "Text", &["txt"])
        {
            let ocr_text = self.collect_ocr_text();
            match export_text(&destination, document, &ocr_text) {
                Ok(()) => self.status = format!("Exported {}", destination.display()),
                Err(error) => self.push_error_notice(error.to_string()),
            }
        }
    }

    fn export_review_text_dialog(&mut self, document: &LiquidDocument) {
        let Some(text) = liquid_document_copy_text(document) else {
            self.status = "Review Mode has no readable text to export.".to_owned();
            return;
        };
        let file_name = self
            .document
            .as_ref()
            .map(|source| default_output_name(&source.path, "review-mode", "txt"))
            .unwrap_or_else(|| "review-mode.txt".to_owned());
        if let Some(destination) =
            self.pick_save_path("Download Review Mode text", &file_name, "Text", &["txt"])
        {
            let contents = format!("{}\n", text.trim_end());
            match std::fs::write(&destination, contents) {
                Ok(()) => self.status = format!("Downloaded {}", destination.display()),
                Err(error) => {
                    self.push_error_notice(format!("Could not save Review Mode text: {error}"))
                }
            }
        }
    }

    fn export_png_dialog(&mut self) {
        let Some(document) = self.document.as_ref() else {
            return;
        };

        let path = document.path.clone();
        let page_index = self.page_index;
        let file_name = default_output_name(&path, &format!("page-{}", page_index + 1), "png");

        if let Some(destination) =
            self.pick_save_path("Export page image", &file_name, "PNG", &["png"])
        {
            match self.export_page_png_on_worker(path, page_index, destination.clone(), 2.0) {
                Ok(()) => self.status = format!("Exported {}", destination.display()),
                Err(error) => self.push_error_notice(error),
            }
        }
    }

    fn start_ocr(&mut self) {
        let Some(document) = self.document.as_ref() else {
            return;
        };

        let page_count = document.page_count;
        let pdf_path = document.path.clone();
        self.ocr_states = vec![OcrPageState::Pending; page_count];
        self.ocr_progress = Some(OcrProgress {
            started_at: Instant::now(),
            initial_completed: 0,
        });
        spawn_ocr_job(
            self.document_epoch,
            pdf_path,
            page_count,
            self.ocr_tx.clone(),
            self.render_tx.clone(),
        );
        self.status = "OCR started in the background.".to_owned();
    }

    fn start_openrouter_ocr_save(&mut self) {
        let Some(document) = self.document.as_ref() else {
            return;
        };

        let Some(api_key) = effective_openrouter_api_key(&self.settings) else {
            self.status = "OpenRouter OCR needs an API key.".to_owned();
            self.settings_ui.open = true;
            return;
        };

        let pdf_path = document.path.clone();
        let page_count = document.page_count;
        let page_sizes = document
            .pages
            .iter()
            .map(|page| (page.width, page.height))
            .collect::<Vec<_>>();

        let default = sidecar_path_for_export(&pdf_path, "ocr", "pdf");
        let file_name = default
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("document-ocr.pdf");

        let Some(destination) = self.pick_save_path("Save OCR PDF", file_name, "PDF", &["pdf"])
        else {
            return;
        };

        if self.ocr_states.len() != page_count {
            self.ocr_states = vec![OcrPageState::Idle; page_count];
        }
        let initial_ocr_text = self
            .ocr_states
            .iter()
            .map(|state| state.text().map(str::to_owned))
            .collect::<Vec<_>>();
        let cached_pages = initial_ocr_text
            .iter()
            .filter(|text| text.is_some())
            .count();
        for state in &mut self.ocr_states {
            if !matches!(state, OcrPageState::Done(_)) {
                *state = OcrPageState::Pending;
            }
        }
        self.ocr_progress = Some(OcrProgress {
            started_at: Instant::now(),
            initial_completed: cached_pages,
        });
        spawn_openrouter_ocr_save_job(
            self.document_epoch,
            pdf_path,
            page_count,
            page_sizes,
            destination.clone(),
            initial_ocr_text,
            api_key,
            self.ocr_tx.clone(),
            self.render_tx.clone(),
        );
        self.status = if cached_pages > 0 {
            format!(
                "OpenRouter OCR started with {cached_pages} cached page(s); saving {}",
                destination.display()
            )
        } else {
            format!("OpenRouter OCR started; saving {}", destination.display())
        };
    }

    fn poll_ocr(&mut self, ctx: &Context) {
        let mut should_rebuild_search = false;
        let mut current_ocr_text_changed = false;

        while let Ok(event) = self.ocr_rx.try_recv() {
            let mut error_notice = match &event.state {
                OcrPageState::Failed(error) => Some(format!(
                    "OCR failed on page {}: {error}",
                    event.page_index + 1
                )),
                _ => None,
            };
            if let Some(status) = event.status.as_ref() {
                self.status = status.clone();
            }
            if self.is_current_document(event.document_epoch, &event.path) {
                if let Some(state) = self.ocr_states.get_mut(event.page_index) {
                    let was_done = matches!(event.state, OcrPageState::Done(_));
                    *state = event.state;
                    should_rebuild_search |=
                        was_done && !self.search_state.query.trim().is_empty();
                    current_ocr_text_changed |= was_done;
                    if was_done {
                        if let Some(document) = self.document.as_ref() {
                            if let Err(error) = save_ocr_cache(&document.path, &self.ocr_states) {
                                error_notice = Some(format!("Could not save OCR cache: {error}"));
                            }
                        }
                        if self.chat_ui.state.messages.is_empty() {
                            self.chat_ui.state.document_context = None;
                            self.chat_ui.state.context_estimated_tokens = None;
                            self.chat_ui.state.context_warning = None;
                        }
                    }
                }
            } else if let Some(tab) = self.tabs.iter_mut().find(|tab| {
                tab.document_epoch == event.document_epoch && tab.document.path == event.path
            }) {
                if let Some(state) = tab.ocr_states.get_mut(event.page_index) {
                    let was_done = matches!(event.state, OcrPageState::Done(_));
                    *state = event.state;
                    if was_done {
                        if let Err(error) = save_ocr_cache(&tab.document.path, &tab.ocr_states) {
                            error_notice = Some(format!("Could not save OCR cache: {error}"));
                        }
                        if tab.chat_state.messages.is_empty() {
                            tab.chat_state.document_context = None;
                            tab.chat_state.context_estimated_tokens = None;
                            tab.chat_state.context_warning = None;
                        }
                    }
                }
            }
            if let Some(error) = error_notice {
                self.push_error_notice(error);
            }
        }

        if should_rebuild_search {
            self.rebuild_search();
        }

        if current_ocr_text_changed
            && liquid_state_needs_ocr(&self.liquid_state)
            && !self.ocr_is_active()
            && has_usable_ocr_text(&self.ocr_states)
        {
            self.liquid_state = LiquidState::Idle;
            self.liquid_notice_dismissed = false;
            self.status = "OCR ready; rebuilding Review Mode...".to_owned();
            self.ensure_liquid_started(ctx);
        }
        if current_ocr_text_changed
            && liquid_state_needs_ocr(&self.liquid_mode2_state)
            && !self.ocr_is_active()
            && has_usable_ocr_text(&self.ocr_states)
        {
            self.liquid_mode2_state = LiquidState::Idle;
            self.status = "OCR ready; rebuilding Review Mode...".to_owned();
            self.ensure_liquid_mode2_started(ctx);
        }
    }

    fn poll_render_results(&mut self, ctx: &Context) {
        while let Ok(event) = self.render_rx.try_recv() {
            match event {
                RenderEvent::Page {
                    key,
                    path,
                    _zoom: _,
                    render_scale,
                    result,
                } => {
                    if self
                        .pending_page_renders
                        .get(&key.page_index)
                        .is_some_and(|pending_key| *pending_key == key)
                    {
                        self.pending_page_renders.remove(&key.page_index);
                    }
                    if !self.is_current_document(key.document_epoch, &path) {
                        continue;
                    }

                    match result {
                        Ok(rendered) => {
                            let current_render_scale = self
                                .document
                                .as_ref()
                                .and_then(|document| document.pages.get(key.page_index))
                                .map(|page| self.page_render_scale(ctx, page.width, page.height))
                                .unwrap_or(render_scale);
                            let existing_render_scale = self
                                .page_textures
                                .get(&key.page_index)
                                .map(|texture| texture.render_scale);
                            let incoming_matches_current =
                                (render_scale - current_render_scale).abs() < f32::EPSILON;
                            if !incoming_matches_current
                                && existing_render_scale.is_some_and(|scale| {
                                    (scale - current_render_scale).abs() < f32::EPSILON
                                        || scale >= render_scale
                                })
                            {
                                continue;
                            }

                            self.install_page_texture(
                                ctx,
                                key.document_epoch,
                                rendered,
                                self.zoom,
                                render_scale,
                            );
                            ctx.request_repaint();
                        }
                        Err(error) => {
                            self.push_error_notice(error);
                            self.page_textures.remove(&key.page_index);
                        }
                    }
                }
                RenderEvent::Thumbnail { key, path, result } => {
                    if self
                        .pending_thumbnail_renders
                        .get(&key.page_index)
                        .is_some_and(|pending_key| *pending_key == key)
                    {
                        self.pending_thumbnail_renders.remove(&key.page_index);
                    }
                    if !self.is_current_document(key.document_epoch, &path) {
                        continue;
                    }

                    match result {
                        Ok(rendered) => {
                            let image = egui::ColorImage::from_rgba_unmultiplied(
                                [rendered.width, rendered.height],
                                &rendered.rgba,
                            );
                            let texture = ctx.load_texture(
                                format!(
                                    "thumb-e{}-{}",
                                    key.document_epoch,
                                    rendered.page_index + 1
                                ),
                                image,
                                TextureOptions::LINEAR,
                            );
                            self.thumbnail_textures
                                .insert(key.page_index, ThumbnailTexture { texture });
                            ctx.request_repaint();
                        }
                        Err(error) => {
                            self.push_error_notice(error);
                        }
                    }
                }
                RenderEvent::TextChars {
                    document_epoch,
                    path,
                    page_index,
                    result,
                } => {
                    self.pending_text_chars.remove(&page_index);
                    if !self.is_current_document(document_epoch, &path) {
                        continue;
                    }

                    match result {
                        Ok(chars) => {
                            if let Some(document) = self.document.as_mut() {
                                if let Some(slot) = document.text_chars.get_mut(page_index) {
                                    *slot = Some(chars);
                                    ctx.request_repaint();
                                }
                            }
                        }
                        Err(error) => {
                            self.push_error_notice(error);
                            if let Some(document) = self.document.as_mut() {
                                if let Some(slot) = document.text_chars.get_mut(page_index) {
                                    *slot = Some(Vec::new());
                                }
                            }
                        }
                    }
                    if matches!(self.liquid_mode2_state, LiquidState::PreparingText)
                        && self.pending_text_chars.is_empty()
                        && self.pending_native_text.is_empty()
                    {
                        self.liquid_mode2_state = LiquidState::Idle;
                        self.ensure_liquid_mode2_started(ctx);
                    }
                }
                RenderEvent::TextPage {
                    document_epoch,
                    path,
                    page_index,
                    result,
                } => {
                    if !self.is_current_document(document_epoch, &path) {
                        continue;
                    }
                    self.pending_native_text.remove(&page_index);

                    match result {
                        Ok(text) => {
                            if let Some(document) = self.document.as_mut() {
                                if let Some(slot) = document.native_text.get_mut(page_index) {
                                    *slot = text;
                                }
                                if let Some(slot) = document.native_text_loaded.get_mut(page_index)
                                {
                                    *slot = true;
                                }
                            }
                            if !self.search_state.query.trim().is_empty() {
                                self.rebuild_search();
                            }
                            ctx.request_repaint();
                        }
                        Err(error) => {
                            self.push_error_notice(error);
                            if let Some(document) = self.document.as_mut() {
                                if let Some(slot) = document.native_text_loaded.get_mut(page_index)
                                {
                                    *slot = true;
                                }
                            }
                        }
                    }
                    if matches!(self.liquid_state, LiquidState::PreparingText)
                        && self.pending_native_text.is_empty()
                    {
                        self.liquid_state = LiquidState::Idle;
                        self.ensure_liquid_started(ctx);
                    }
                    if matches!(self.liquid_mode2_state, LiquidState::PreparingText)
                        && self.pending_native_text.is_empty()
                    {
                        self.liquid_mode2_state = LiquidState::Idle;
                        self.ensure_liquid_mode2_started(ctx);
                    }
                }
                RenderEvent::CommentsSaved {
                    document_epoch,
                    path,
                    generation,
                    result,
                } => {
                    self.active_comment_saves.remove(&path);
                    let newer_pending = self
                        .pending_comment_saves
                        .get(&path)
                        .is_some_and(|save| save.generation > generation);
                    if !self.is_current_document(document_epoch, &path) && newer_pending {
                        continue;
                    }

                    match result {
                        Ok(count) => {
                            if !newer_pending {
                                self.status = format!("Saved {count} comment(s) to PDF.");
                                self.page_textures.clear();
                                self.thumbnail_textures.clear();
                            }
                        }
                        Err(error) => {
                            if !newer_pending {
                                self.push_error_notice(format!("Could not save comments: {error}"));
                            }
                        }
                    }
                    ctx.request_repaint();
                }
            }
        }
    }

    fn load_document_on_worker(&self, path: PathBuf) -> Result<LoadedDocument, String> {
        let (reply_tx, reply_rx) = unbounded();
        self.render_tx
            .send(RenderRequest::LoadDocument {
                path,
                reply: reply_tx,
            })
            .map_err(|error| format!("PDF worker is not available: {error}"))?;
        reply_rx
            .recv_timeout(Duration::from_secs(30))
            .map_err(|error| format!("Timed out opening PDF: {error}"))?
    }

    fn render_page_immediate_on_worker(
        &self,
        path: PathBuf,
        page_index: usize,
        render_scale: f32,
    ) -> Result<RenderedPage, String> {
        let (reply_tx, reply_rx) = unbounded();
        self.render_tx
            .send(RenderRequest::PageImmediate {
                path,
                page_index,
                render_scale,
                reply: reply_tx,
            })
            .map_err(|error| format!("PDF worker is not available: {error}"))?;
        reply_rx
            .recv_timeout(Duration::from_secs(30))
            .map_err(|error| format!("Timed out rendering first page: {error}"))?
    }

    fn render_first_page_before_repaint(&mut self, ctx: &Context) {
        let Some((path, page_width, page_height)) = self.document.as_ref().and_then(|document| {
            document
                .pages
                .first()
                .map(|page| (document.path.clone(), page.width, page.height))
        }) else {
            return;
        };

        let render_scale = self.page_render_scale(ctx, page_width, page_height);
        match self.render_page_immediate_on_worker(path, 0, render_scale) {
            Ok(rendered) => {
                self.install_page_texture(
                    ctx,
                    self.document_epoch,
                    rendered,
                    self.zoom,
                    render_scale,
                );
                self.pending_page_renders.remove(&0);
            }
            Err(error) => {
                self.push_error_notice(error);
            }
        }
    }

    fn next_texture_access(&mut self) -> u64 {
        self.texture_access_counter = self.texture_access_counter.wrapping_add(1);
        self.texture_access_counter
    }

    fn install_page_texture(
        &mut self,
        ctx: &Context,
        document_epoch: u64,
        rendered: RenderedPage,
        zoom: f32,
        render_scale: f32,
    ) {
        let max_texture_side = ctx.input(|input| input.max_texture_side);
        if rendered.width > max_texture_side || rendered.height > max_texture_side {
            self.status = format!(
                "Skipped oversized page texture {}x{}; max side is {}",
                rendered.width, rendered.height, max_texture_side
            );
            return;
        }

        let image = egui::ColorImage::from_rgba_unmultiplied(
            [rendered.width, rendered.height],
            &rendered.rgba,
        );
        let texture = ctx.load_texture(
            format!(
                "page-e{}-{}-z{:.3}-r{:.3}",
                document_epoch,
                rendered.page_index + 1,
                zoom,
                render_scale
            ),
            image,
            TextureOptions::LINEAR,
        );
        let last_used = self.next_texture_access();
        self.page_textures.insert(
            rendered.page_index,
            PageTexture {
                _zoom: zoom,
                render_scale,
                last_used,
                texture,
            },
        );
        self.prune_page_texture_cache();
    }

    fn prefetch_small_document_pages(&mut self, ctx: &Context) {
        let Some((path, pages)) = self.document.as_ref().map(|document| {
            let pages = document
                .pages
                .iter()
                .enumerate()
                .skip(1)
                .map(|(page_index, page)| (page_index, page.width, page.height))
                .collect::<Vec<_>>();
            (document.path.clone(), pages)
        }) else {
            return;
        };
        let page_count = pages.len() + 1;
        if page_count <= 1 || page_count > PAGE_TEXTURE_CACHE_CAP {
            return;
        }

        for (page_index, page_width, page_height) in pages
            .into_iter()
            .take(SMALL_DOCUMENT_PREFETCH_LIMIT.saturating_sub(1))
        {
            let render_scale = self.page_render_scale(ctx, page_width, page_height);
            let is_current = self
                .page_textures
                .get(&page_index)
                .is_some_and(|texture| (texture.render_scale - render_scale).abs() < f32::EPSILON);
            if !is_current {
                self.request_page_render(ctx, &path, page_index, render_scale);
            }
        }
    }

    fn export_page_png_on_worker(
        &self,
        path: PathBuf,
        page_index: usize,
        destination: PathBuf,
        scale: f32,
    ) -> Result<(), String> {
        let (reply_tx, reply_rx) = unbounded();
        self.render_tx
            .send(RenderRequest::ExportPagePng {
                path,
                page_index,
                destination,
                scale,
                reply: reply_tx,
            })
            .map_err(|error| format!("PDF worker is not available: {error}"))?;
        reply_rx
            .recv_timeout(Duration::from_secs(30))
            .map_err(|error| format!("Timed out exporting PNG: {error}"))?
    }

    fn is_current_document(&self, document_epoch: u64, path: &Path) -> bool {
        self.document_epoch == document_epoch
            && self
                .document
                .as_ref()
                .is_some_and(|document| document.path == path)
    }

    fn collect_ocr_text(&self) -> Vec<String> {
        self.ocr_states
            .iter()
            .map(|state| state.text().unwrap_or_default().to_owned())
            .collect()
    }

    fn collect_liquid_source_pages(&self, document: &LoadedDocument) -> Vec<String> {
        (0..document.page_count)
            .map(|page_index| {
                let native = document
                    .native_text
                    .get(page_index)
                    .map(|text| text.trim())
                    .unwrap_or_default();
                let ocr = self
                    .ocr_states
                    .get(page_index)
                    .and_then(OcrPageState::text)
                    .unwrap_or_default();
                if should_prefer_ocr_page_text(native, ocr) {
                    ocr.to_owned()
                } else if !native.is_empty() {
                    native.to_owned()
                } else {
                    ocr.to_owned()
                }
            })
            .collect()
    }

    fn collect_liquid_source_pages_with_layout_text(
        &self,
        document: &LoadedDocument,
    ) -> Vec<String> {
        let layout_pages =
            layout_roles::source_pages_from_text_chars(&document.pages, &document.text_chars);
        (0..document.page_count)
            .map(|page_index| {
                let native = document
                    .native_text
                    .get(page_index)
                    .map(|text| text.trim())
                    .unwrap_or_default();
                let layout = layout_pages
                    .get(page_index)
                    .and_then(Option::as_deref)
                    .map(str::trim)
                    .unwrap_or_default();
                let ocr = self
                    .ocr_states
                    .get(page_index)
                    .and_then(OcrPageState::text)
                    .unwrap_or_default();
                if should_prefer_ocr_page_text(native, ocr) {
                    ocr.to_owned()
                } else if !layout.is_empty() {
                    layout.to_owned()
                } else if !native.is_empty() {
                    native.to_owned()
                } else {
                    ocr.to_owned()
                }
            })
            .collect()
    }

    fn set_view_mode(&mut self, mode: DocumentViewMode, ctx: &Context) {
        // #29: returning to a reading view clears any stale source-provenance highlight.
        if mode != DocumentViewMode::Pdf {
            self.liquid_provenance_highlight.clear();
        }
        if self.view_mode == mode {
            if mode == DocumentViewMode::Liquid {
                self.ensure_liquid_started(ctx);
            } else if mode == DocumentViewMode::LiquidMode2 {
                self.ensure_liquid_mode2_started(ctx);
            }
            return;
        }
        self.clear_text_selection();
        self.view_mode = mode;
        match mode {
            DocumentViewMode::Pdf => {
                self.status = "PDF view".to_owned();
            }
            DocumentViewMode::Liquid => {
                self.ensure_liquid_started(ctx);
            }
            DocumentViewMode::LiquidMode2 => {
                self.ensure_liquid_mode2_started(ctx);
            }
        }
        ctx.request_repaint();
    }

    fn apply_startup_view_mode(&mut self, ctx: &Context) {
        let Ok(value) = std::env::var("LAWPDF_START_VIEW") else {
            return;
        };
        let mode = match value.trim().to_ascii_lowercase().as_str() {
            "liquid" | "lm2" | "liquid2" | "liquidmode2" => DocumentViewMode::LiquidMode2,
            "legacy-liquid" => DocumentViewMode::Liquid,
            _ => return,
        };
        if self.document.is_some() {
            self.set_view_mode(mode, ctx);
        }
    }

    fn ensure_liquid_started(&mut self, ctx: &Context) {
        if !matches!(
            self.liquid_state,
            LiquidState::Idle | LiquidState::PreparingText
        ) {
            return;
        }

        if self.document.is_none() {
            return;
        }

        if !self.ensure_native_text_loaded_for_all(ctx, "Preparing PDF text for Review Mode") {
            self.liquid_state = LiquidState::PreparingText;
            self.liquid_notice_dismissed = false;
            self.status = "Preparing PDF text for Review Mode...".to_owned();
            return;
        }

        if !self.ensure_text_chars_loaded_for_all(ctx, "Preparing PDF layout for Review Mode") {
            self.liquid_state = LiquidState::PreparingText;
            self.liquid_notice_dismissed = false;
            self.status = "Preparing PDF layout for Review Mode...".to_owned();
            return;
        }

        let Some(document) = self.document.as_ref() else {
            return;
        };
        let pages = self.collect_liquid_source_pages_with_layout_text(document);
        let (layout_hints, source_line_hints) =
            layout_roles::layout_hints_and_source_lines_for_pages(
                &document.pages,
                &document.text_chars,
            );
        let deep_source_lines =
            layout_roles::deep_source_lines_for_pages(&document.pages, &document.text_chars);
        let request = LiquidRequest {
            document_epoch: self.document_epoch,
            path: document.path.clone(),
            title: document.title.clone(),
            pages,
            layout_hints,
            source_line_hints,
            deep_source_lines,
            deep_liquid: effective_deep_liquid_config(&self.settings),
            groq_api_key: effective_groq_api_key(&self.settings),
            openrouter_api_key: effective_openrouter_api_key(&self.settings),
        };
        self.liquid_state = LiquidState::Preparing;
        self.liquid_notice_dismissed = false;
        self.status = "Preparing Review Mode...".to_owned();
        spawn_liquid_job(request, self.liquid_tx.clone());
        ctx.request_repaint_after(RENDER_POLL_INTERVAL);
    }

    fn poll_liquid_results(&mut self, ctx: &Context) {
        while let Ok(event) = self.liquid_rx.try_recv() {
            let mut error_notice = None;
            let next_state = match event.result {
                Ok(document) => {
                    let status = if let Some(warning) = document
                        .warnings
                        .first()
                        .filter(|warning| warning.contains("No selectable text found"))
                    {
                        warning.clone()
                    } else if document.llm_used || document.deep_liquid_used {
                        format!(
                            "Review Mode ready; {} noise line(s) removed.",
                            document.noise_lines_removed
                        )
                    } else {
                        format!(
                            "Review Mode ready locally; {} noise line(s) removed.",
                            document.noise_lines_removed
                        )
                    };
                    (LiquidState::Ready(document), status)
                }
                Err(error) => {
                    error_notice = Some(format!("Review Mode failed: {error}"));
                    (LiquidState::Failed(error.clone()), error)
                }
            };

            if self.is_current_document(event.document_epoch, &event.path) {
                self.liquid_state = next_state.0;
                self.liquid_notice_dismissed = false;
                self.status = next_state.1;
                ctx.request_repaint();
            } else if let Some(tab) = self.tabs.iter_mut().find(|tab| {
                tab.document_epoch == event.document_epoch && tab.document.path == event.path
            }) {
                tab.liquid_state = next_state.0;
                tab.liquid_notice_dismissed = false;
                tab.status = next_state.1;
            }
            if let Some(error) = error_notice {
                self.push_error_notice(error);
            }
        }
    }

    fn ensure_liquid_mode2_started(&mut self, ctx: &Context) {
        if !matches!(
            self.liquid_mode2_state,
            LiquidState::Idle | LiquidState::PreparingText
        ) {
            return;
        }

        if self.document.is_none() {
            return;
        }

        if let Some(document) = self.document.as_ref().and_then(|source| {
            load_fast_cached_liquid_mode2_document(
                &source.path,
                self.settings.liquid_mode2_use_pymupdf_blocks,
                self.settings.liquid_mode2_use_pp_footnote_regions,
            )
        }) {
            self.liquid_mode2_state = LiquidState::Ready(document);
            self.status = "LM2 ready from cache.".to_owned();
            ctx.request_repaint();
            return;
        }

        if !self.ensure_native_text_loaded_for_all(ctx, "Preparing PDF text for Review Mode") {
            self.liquid_mode2_state = LiquidState::PreparingText;
            self.status = "Preparing PDF text for Review Mode...".to_owned();
            return;
        }

        if !self.ensure_text_chars_loaded_for_all(ctx, "Preparing PDF layout for Review Mode") {
            self.liquid_mode2_state = LiquidState::PreparingText;
            self.status = "Preparing PDF layout for Review Mode...".to_owned();
            return;
        }

        let Some(document) = self.document.as_ref() else {
            return;
        };
        let pages = self.collect_liquid_source_pages_with_layout_text(document);
        let deep_source_lines =
            layout_roles::deep_source_lines_for_pages(&document.pages, &document.text_chars);
        let request = LiquidMode2Request {
            document_epoch: self.document_epoch,
            path: document.path.clone(),
            title: document.title.clone(),
            pages,
            deep_source_lines,
            use_pymupdf_blocks: self.settings.liquid_mode2_use_pymupdf_blocks,
            use_pp_footnote_regions: self.settings.liquid_mode2_use_pp_footnote_regions,
            external_emissions_path: None,
        };
        self.liquid_mode2_state = LiquidState::Preparing;
        self.status = "Preparing Review Mode...".to_owned();
        spawn_liquid_mode2_job(request, self.liquid_mode2_tx.clone());
        ctx.request_repaint_after(RENDER_POLL_INTERVAL);
    }

    fn poll_liquid_mode2_results(&mut self, ctx: &Context) {
        while let Ok(event) = self.liquid_mode2_rx.try_recv() {
            let complete = event.complete;
            let preview_page_count = event.preview_page_count;
            let mut error_notice = None;
            let next_state = match event.result {
                Ok(document) => {
                    let status = if complete {
                        document
                            .warnings
                            .iter()
                            .find(|warning| {
                                warning.contains("Promoted native CatBoost runtime failed")
                                    || warning
                                        .contains("Promoted context two-pass model failed")
                            })
                            .cloned()
                            .unwrap_or_else(|| {
                                format!(
                                    "Review Mode ready; {} noise line(s) removed.",
                                    document.noise_lines_removed
                                )
                            })
                    } else {
                        format!(
                            "First {} page(s) ready; finishing the full Liquid document in the background...",
                            preview_page_count.unwrap_or(0)
                        )
                    };
                    (LiquidState::Ready(document), status)
                }
                Err(error) => {
                    error_notice = Some(format!("Review Mode failed: {error}"));
                    (LiquidState::Failed(error.clone()), error)
                }
            };

            if self.is_current_document(event.document_epoch, &event.path) {
                self.liquid_mode2_state = next_state.0;
                self.status = next_state.1;
                ctx.request_repaint();
            } else if let Some(tab) = self.tabs.iter_mut().find(|tab| {
                tab.document_epoch == event.document_epoch && tab.document.path == event.path
            }) {
                tab.liquid_mode2_state = next_state.0;
                tab.status = next_state.1;
            }
            if let Some(error) = error_notice {
                self.push_error_notice(error);
            }
        }
    }

    fn start_search(&mut self, ctx: &Context) {
        if !self.search_state.query.trim().is_empty()
            && !self.ensure_native_text_loaded_for_all(ctx, "Preparing searchable PDF text")
        {
            self.rebuild_search();
            return;
        }
        self.rebuild_search();
    }

    fn focus_search(&mut self, ctx: &Context) {
        self.sidebar_tab = SidebarTab::Search;
        self.search_state.focus_request = true;
        if self.document.is_none() {
            self.status = "Open a PDF to search.".to_owned();
        }
        ctx.request_repaint();
    }

    fn rebuild_search(&mut self) {
        self.search_state.hits.clear();
        self.search_state.selected_hit = None;

        let Some(document) = self.document.as_ref() else {
            return;
        };

        let query = self.search_state.query.trim();
        if query.is_empty() {
            return;
        }

        for page_index in 0..document.page_count {
            if let Some(text) = document.native_text.get(page_index) {
                self.search_state.hits.extend(find_hits(
                    text,
                    query,
                    page_index,
                    SearchSource::NativeText,
                ));
            }

            if let Some(text) = self.ocr_states.get(page_index).and_then(OcrPageState::text) {
                self.search_state
                    .hits
                    .extend(find_hits(text, query, page_index, SearchSource::OcrText));
            }
        }

        if let Some(first) = self.search_state.hits.first() {
            self.search_state.selected_hit = Some(0);
            self.page_index = first.page_index;
            self.scroll_target_page = Some(first.page_index);
            self.thumbnail_scroll_target = Some(first.page_index);
        }

        self.status = format!("{} match(es)", self.search_state.hits.len());
    }

    fn add_search_highlights(&mut self) {
        let Some(document) = self.document.as_ref() else {
            return;
        };

        let preset = self.marker_preset();
        let mut added = 0usize;
        for hit in &self.search_state.hits {
            if let Some(rect) = self.estimated_hit_rect(document, hit) {
                self.annotations.push(EditorAnnotation {
                    page_index: hit.page_index,
                    rect,
                    kind: AnnotationKind::Marker {
                        color_rgb: preset.color_rgb,
                        opacity: self.marker_opacity_for(preset),
                        style: preset.style,
                    },
                });
                added += 1;
            }
        }

        if added > 0 {
            self.annotations_dirty = true;
        }
        self.status = format!("Added {added} highlight annotation(s)");
    }

    fn estimated_hit_rect(&self, document: &LoadedDocument, hit: &SearchHit) -> Option<PdfRect> {
        let page = document.pages.get(hit.page_index)?;
        let text = match hit.source {
            SearchSource::NativeText => document.native_text.get(hit.page_index)?.as_str(),
            SearchSource::OcrText => self.ocr_states.get(hit.page_index)?.text()?,
        };

        let safe_start = floor_char_boundary(text, hit.match_start.min(text.len()));
        let line_index = text[..safe_start].chars().filter(|ch| *ch == '\n').count();
        let line_height = 14.0;
        let top =
            (page.height - 54.0 - line_index as f32 * line_height).clamp(36.0, page.height - 24.0);
        let bottom = (top - 12.0).max(24.0);

        Some(PdfRect::new(
            48.0,
            bottom,
            (page.width - 48.0).max(96.0),
            top,
        ))
    }

    fn ensure_page_texture(
        &mut self,
        ctx: &Context,
        path: &Path,
        page_index: usize,
    ) -> Option<PageTextureView> {
        let render_scale = self
            .document
            .as_ref()
            .and_then(|document| document.pages.get(page_index))
            .map(|page| self.page_render_scale(ctx, page.width, page.height))
            .unwrap_or_else(|| self.render_scale(ctx));
        let is_current = self
            .page_textures
            .get(&page_index)
            .is_some_and(|texture| (texture.render_scale - render_scale).abs() < f32::EPSILON);

        if !is_current {
            let has_stale_texture = self.page_textures.contains_key(&page_index);
            if !has_stale_texture || !self.zoom_render_is_debounced() {
                self.request_page_render(ctx, path, page_index, render_scale);
            } else {
                ctx.request_repaint_after(ZOOM_RENDER_DEBOUNCE);
            }
        }

        let last_used = self.next_texture_access();
        self.page_textures.get_mut(&page_index).map(|texture| {
            texture.last_used = last_used;
            PageTextureView {
                texture_id: texture.texture.id(),
            }
        })
    }

    fn ensure_thumbnail_texture(
        &mut self,
        ctx: &Context,
        path: &Path,
        page_index: usize,
        page_width: f32,
    ) -> Option<ThumbnailTextureView> {
        if !self.thumbnail_textures.contains_key(&page_index) {
            let display_width = 118.0;
            let render_scale = (display_width / page_width).clamp(0.12, 0.32);
            self.request_thumbnail_render(ctx, path, page_index, render_scale);
        }

        self.thumbnail_textures
            .get(&page_index)
            .map(|texture| ThumbnailTextureView {
                texture_id: texture.texture.id(),
            })
    }

    fn render_scale(&self, ctx: &Context) -> f32 {
        let device_pixels_per_point = ctx.pixels_per_point().max(1.0);
        let raw = (self.zoom * device_pixels_per_point * 1.25).clamp(0.75, 3.25);
        ((raw * 8.0).round() / 8.0).clamp(0.75, 3.25)
    }

    fn page_render_scale(&self, ctx: &Context, page_width: f32, page_height: f32) -> f32 {
        let base_scale = self.render_scale(ctx);
        let max_texture_side = ctx.input(|input| input.max_texture_side);
        let safe_texture_side = max_texture_side.saturating_sub(16).max(256) as f32;
        let page_side = page_width.max(page_height).max(1.0);
        let max_scale = ((safe_texture_side / page_side) * 8.0).floor() / 8.0;
        base_scale.min(max_scale.max(0.25))
    }

    fn zoom_is_animating(&self) -> bool {
        (self.target_zoom - self.zoom).abs() > ZOOM_ANIMATION_EPSILON
    }

    fn request_page_render(
        &mut self,
        ctx: &Context,
        path: &Path,
        page_index: usize,
        render_scale: f32,
    ) {
        let key = PageRenderKey::new(self.document_epoch, page_index, render_scale);
        if self
            .pending_page_renders
            .get(&page_index)
            .is_some_and(|pending_key| *pending_key == key)
        {
            return;
        }
        self.pending_page_renders.insert(page_index, key);

        let request = RenderRequest::Page {
            key,
            path: path.to_path_buf(),
            zoom: self.zoom,
            render_scale,
        };
        if self.render_tx.send(request).is_err() {
            if self
                .pending_page_renders
                .get(&page_index)
                .is_some_and(|pending_key| *pending_key == key)
            {
                self.pending_page_renders.remove(&page_index);
            }
            self.push_error_notice("PDF render worker stopped.");
        } else {
            ctx.request_repaint_after(RENDER_POLL_INTERVAL);
        }
    }

    fn request_thumbnail_render(
        &mut self,
        ctx: &Context,
        path: &Path,
        page_index: usize,
        render_scale: f32,
    ) {
        let key = ThumbnailRenderKey {
            document_epoch: self.document_epoch,
            page_index,
        };
        if self
            .pending_thumbnail_renders
            .get(&page_index)
            .is_some_and(|pending_key| *pending_key == key)
        {
            return;
        }
        self.pending_thumbnail_renders.insert(page_index, key);

        let request = RenderRequest::Thumbnail {
            key,
            path: path.to_path_buf(),
            render_scale,
        };
        if self.render_tx.send(request).is_err() {
            if self
                .pending_thumbnail_renders
                .get(&page_index)
                .is_some_and(|pending_key| *pending_key == key)
            {
                self.pending_thumbnail_renders.remove(&page_index);
            }
            self.push_error_notice("PDF render worker stopped.");
        } else {
            ctx.request_repaint_after(RENDER_POLL_INTERVAL);
        }
    }

    fn request_text_chars(&mut self, ctx: &Context, path: &Path, page_index: usize) {
        if self.enqueue_text_chars(path, page_index) {
            ctx.request_repaint_after(RENDER_POLL_INTERVAL);
        }
    }

    fn ensure_native_text_loaded_for_all(&mut self, ctx: &Context, status: &str) -> bool {
        let Some((path, page_count)) = self
            .document
            .as_ref()
            .map(|document| (document.path.clone(), document.page_count))
        else {
            return false;
        };

        let loaded_flags = self
            .document
            .as_ref()
            .map(|document| document.native_text_loaded.clone())
            .unwrap_or_default();
        let mut waiting = 0usize;
        let mut requested = 0usize;
        for page_index in 0..page_count {
            let is_loaded = loaded_flags.get(page_index).copied().unwrap_or(false);
            if is_loaded {
                continue;
            }
            waiting += 1;
            if self.enqueue_native_text(&path, page_index) {
                requested += 1;
            }
        }

        if waiting > 0 {
            self.status = if requested > 0 {
                format!("{status} ({requested} page(s) queued)")
            } else {
                format!("{status} ({waiting} page(s) pending)")
            };
            ctx.request_repaint_after(RENDER_POLL_INTERVAL);
            false
        } else {
            true
        }
    }

    fn ensure_text_chars_loaded_for_all(&mut self, ctx: &Context, status: &str) -> bool {
        let Some((path, page_count)) = self
            .document
            .as_ref()
            .map(|document| (document.path.clone(), document.page_count))
        else {
            return false;
        };

        let loaded_flags = self
            .document
            .as_ref()
            .map(|document| {
                document
                    .text_chars
                    .iter()
                    .map(Option::is_some)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut waiting = 0usize;
        let mut requested = 0usize;
        for page_index in 0..page_count {
            let is_loaded = loaded_flags.get(page_index).copied().unwrap_or(false);
            if is_loaded {
                continue;
            }
            waiting += 1;
            if self.enqueue_text_chars(&path, page_index) {
                requested += 1;
            }
        }

        if waiting > 0 {
            self.status = if requested > 0 {
                format!("{status} ({requested} page(s) queued)")
            } else {
                format!("{status} ({waiting} page(s) pending)")
            };
            ctx.request_repaint_after(RENDER_POLL_INTERVAL);
            false
        } else {
            true
        }
    }

    fn enqueue_native_text(&mut self, path: &Path, page_index: usize) -> bool {
        let should_request = self
            .document
            .as_ref()
            .and_then(|document| document.native_text_loaded.get(page_index))
            .is_some_and(|loaded| !*loaded);
        if !should_request || !self.pending_native_text.insert(page_index) {
            return false;
        }

        let request = RenderRequest::TextPageAsync {
            document_epoch: self.document_epoch,
            path: path.to_path_buf(),
            page_index,
        };
        if self.render_tx.send(request).is_err() {
            self.pending_native_text.remove(&page_index);
            self.push_error_notice("PDF render worker stopped.");
            false
        } else {
            true
        }
    }

    fn enqueue_text_chars(&mut self, path: &Path, page_index: usize) -> bool {
        let should_request = self
            .document
            .as_ref()
            .and_then(|document| document.text_chars.get(page_index))
            .is_some_and(Option::is_none);
        if !should_request || !self.pending_text_chars.insert(page_index) {
            return false;
        }

        let request = RenderRequest::TextCharsAsync {
            document_epoch: self.document_epoch,
            path: path.to_path_buf(),
            page_index,
        };
        if self.render_tx.send(request).is_err() {
            self.pending_text_chars.remove(&page_index);
            self.push_error_notice("PDF render worker stopped.");
            false
        } else {
            true
        }
    }

    fn zoom_render_is_debounced(&self) -> bool {
        if self.zoom_is_animating() {
            return true;
        }

        self.last_zoom_change
            .is_some_and(|changed_at| changed_at.elapsed() < ZOOM_RENDER_DEBOUNCE)
    }

    fn ensure_text_chars(&mut self, page_index: usize) -> Option<&[PageTextChar]> {
        let needs_load = {
            let document = self.document.as_ref()?;
            document
                .text_chars
                .get(page_index)
                .is_some_and(Option::is_none)
        };

        if needs_load {
            if let Some(path) = self.document.as_ref().map(|document| document.path.clone()) {
                self.enqueue_text_chars(&path, page_index);
            }
            return None;
        }

        self.document
            .as_ref()?
            .text_chars
            .get(page_index)?
            .as_deref()
    }

    fn default_zoom_for_new_document(&self) -> f32 {
        zoom_for_new_document(
            self.document.as_ref().map(|_| self.target_zoom),
            &self.settings,
        )
    }

    fn remember_pdf_zoom(&mut self, zoom: f32) {
        let zoom = normalized_pdf_zoom(zoom);
        if (self.settings.last_pdf_zoom - zoom).abs() <= f32::EPSILON {
            return;
        }

        self.settings.last_pdf_zoom = zoom;
        if let Err(error) = save_settings(&self.settings) {
            self.push_error_notice(format!("Could not save zoom setting: {error}"));
        }
    }

    fn set_zoom(&mut self, zoom: f32) {
        let zoom = normalized_pdf_zoom(zoom);
        if (self.target_zoom - zoom).abs() > f32::EPSILON {
            self.target_zoom = zoom;
            self.remember_pdf_zoom(zoom);
            self.last_zoom_change = Some(Instant::now());
        }
    }

    fn advance_zoom_animation(&mut self, ctx: &Context) {
        if !self.zoom_is_animating() {
            self.zoom = self.target_zoom;
            return;
        }

        let dt = ctx
            .input(|input| input.stable_dt)
            .clamp(1.0 / 240.0, 1.0 / 20.0);
        let blend = 1.0 - (-ZOOM_ANIMATION_SPEED * dt).exp();
        self.zoom += (self.target_zoom - self.zoom) * blend;

        if !self.zoom_is_animating() {
            self.zoom = self.target_zoom;
        }

        ctx.request_repaint_after(RENDER_POLL_INTERVAL);
    }

    fn go_to_page(&mut self, page_index: usize) {
        let Some(document) = self.document.as_ref() else {
            return;
        };

        self.page_index = page_index.min(document.page_count.saturating_sub(1));
        self.scroll_target_page = Some(self.page_index);
        self.thumbnail_scroll_target = Some(self.page_index);
    }

    fn visible_range_for_page(&self, page_index: usize) -> Option<VisiblePageRange> {
        self.visible_page_ranges
            .iter()
            .copied()
            .find(|range| range.page_index == page_index)
    }

    fn marker_preset(&self) -> MarkerPreset {
        MARKER_PRESETS[self.marker_preset_index.min(MARKER_PRESETS.len() - 1)]
    }

    fn marker_opacity_for(&self, preset: MarkerPreset) -> f32 {
        match preset.style {
            MarkerStyle::Highlight => self.marker_opacity,
            MarkerStyle::Underline => preset.opacity,
        }
    }

    fn add_text_box_annotation(&mut self, page_index: usize, rect: PdfRect) {
        let annotation_index = self.annotations.len();
        self.annotations.push(EditorAnnotation {
            page_index,
            rect,
            kind: AnnotationKind::TextBox {
                text: self.text_box_text.trim().to_owned(),
                font_size: 12.0,
                color_rgb: [0.05, 0.05, 0.05],
            },
        });
        self.start_text_box_edit(annotation_index);
        self.status = "Text box added. Type in the box.".to_owned();
    }

    fn add_comment_annotation(
        &mut self,
        ctx: &Context,
        page_index: usize,
        anchor_pdf: (f32, f32),
        page_width: f32,
        page_height: f32,
    ) {
        let color = COMMENT_COLOR_PRESETS
            .get(self.comment_color_index)
            .unwrap_or(&COMMENT_COLOR_PRESETS[0])
            .color_rgb;
        let now = comment_timestamp();
        let annotation_index = self.annotations.len();
        // Alternate margins so stacked comments don't pile onto one side.
        let comments_on_page = self
            .annotations
            .iter()
            .filter(|annotation| {
                annotation.page_index == page_index
                    && matches!(annotation.kind, AnnotationKind::Comment { .. })
            })
            .count();
        let side = if comments_on_page % 2 == 0 {
            CommentSide::Right
        } else {
            CommentSide::Left
        };
        self.annotations.push(EditorAnnotation {
            page_index,
            rect: comment_card_rect(page_height, page_width, anchor_pdf, side),
            kind: AnnotationKind::Comment {
                id: new_comment_id(),
                text: String::new(),
                color_rgb: color,
                created_at: now.clone(),
                updated_at: now,
                anchor: anchor_pdf,
            },
        });
        self.start_comment_edit(annotation_index);
        self.schedule_comment_autosave_now(ctx);
        self.status = "Comment added.".to_owned();
    }

    fn select_comment(&mut self, annotation_index: usize) {
        self.clear_text_box_selection();
        self.selected_comment = Some(annotation_index);
        self.clear_text_selection();
    }

    fn start_comment_edit(&mut self, annotation_index: usize) {
        self.select_comment(annotation_index);
        self.editing_comment = Some(annotation_index);
        self.comment_focus_request = Some(annotation_index);
    }

    fn finish_comment_edit(&mut self) {
        self.editing_comment = None;
        self.comment_focus_request = None;
        self.status = "Comment updated.".to_owned();
    }

    fn clear_comment_selection(&mut self) {
        self.selected_comment = None;
        self.editing_comment = None;
        self.comment_focus_request = None;
        self.comment_action_rect = None;
        self.comment_drag = None;
    }

    fn delete_comment(&mut self, ctx: &Context, annotation_index: usize) {
        if annotation_index >= self.annotations.len()
            || !matches!(
                self.annotations[annotation_index].kind,
                AnnotationKind::Comment { .. }
            )
        {
            return;
        }

        self.annotations.remove(annotation_index);
        self.clear_comment_selection();
        self.clear_text_box_selection();
        self.schedule_comment_autosave_now(ctx);
        self.status = "Comment deleted.".to_owned();
    }

    fn schedule_comment_autosave(&mut self, ctx: &Context) {
        self.schedule_comment_autosave_with_delay(ctx, COMMENT_AUTOSAVE_DELAY);
    }

    fn schedule_comment_autosave_now(&mut self, ctx: &Context) {
        self.schedule_comment_autosave_with_delay(ctx, Duration::ZERO);
        self.start_due_comment_saves(ctx);
    }

    fn schedule_comment_autosave_with_delay(&mut self, ctx: &Context, delay: Duration) {
        let _ = delay;
        self.annotations_dirty = true;
        ctx.request_repaint();
    }

    fn start_due_comment_saves(&mut self, ctx: &Context) {
        let now = Instant::now();
        let due_paths = self
            .pending_comment_saves
            .iter()
            .filter(|(path, save)| {
                save.due_at <= now && !self.active_comment_saves.contains_key(*path)
            })
            .map(|(path, _)| path.clone())
            .collect::<Vec<_>>();

        for path in due_paths {
            let Some(save) = self.pending_comment_saves.remove(&path) else {
                continue;
            };
            let generation = save.generation;
            self.active_comment_saves.insert(path.clone(), generation);
            if self
                .render_tx
                .send(RenderRequest::SyncComments {
                    document_epoch: save.document_epoch,
                    path: save.path,
                    generation,
                    comments: save.comments,
                })
                .is_err()
            {
                self.active_comment_saves.remove(&path);
                self.push_error_notice("PDF worker is not available; comment was not saved.");
            } else {
                self.status = "Saving comments...".to_owned();
                ctx.request_repaint_after(RENDER_POLL_INTERVAL);
            }
        }

        if let Some(next_due) = self
            .pending_comment_saves
            .values()
            .map(|save| save.due_at)
            .min()
        {
            ctx.request_repaint_after(next_due.saturating_duration_since(now));
        }
    }

    fn select_text_box(&mut self, annotation_index: usize) {
        self.clear_comment_selection();
        self.selected_text_box = Some(annotation_index);
        self.clear_text_selection();
    }

    fn start_text_box_edit(&mut self, annotation_index: usize) {
        self.select_text_box(annotation_index);
        self.editing_text_box = Some(annotation_index);
        self.text_box_focus_request = Some(annotation_index);
        self.status = "Editing text box.".to_owned();
    }

    fn finish_text_box_edit(&mut self) {
        self.editing_text_box = None;
        self.text_box_focus_request = None;
        self.status = "Text box updated.".to_owned();
    }

    fn clear_text_box_selection(&mut self) {
        self.selected_text_box = None;
        self.editing_text_box = None;
        self.text_box_focus_request = None;
        self.text_box_action_rect = None;
        self.text_box_drag = None;
    }

    fn delete_selected_text_box(&mut self) {
        if let Some(annotation_index) = self.selected_text_box {
            self.delete_text_box(annotation_index);
        }
    }

    fn delete_text_box(&mut self, annotation_index: usize) {
        if annotation_index >= self.annotations.len()
            || !matches!(
                self.annotations[annotation_index].kind,
                AnnotationKind::TextBox { .. }
            )
        {
            return;
        }

        self.annotations.remove(annotation_index);
        self.clear_text_box_selection();
        self.status = "Text box deleted.".to_owned();
    }

    fn draw_tab_strip(&mut self, ui: &mut egui::Ui, ctx: &Context) {
        let mut switch_to = None;
        let mut close_tab = None;

        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("LawPDF").strong().color(INK));
            ui.add_space(4.0);

            for (index, tab) in self.tabs.iter().enumerate() {
                let is_active = self.active_tab == Some(index);
                let tab_fill = if is_active {
                    Color32::from_rgb(255, 254, 250)
                } else {
                    Color32::from_rgb(229, 224, 214)
                };
                let tab_stroke = if is_active {
                    Stroke::new(1.4, Color32::from_rgb(151, 105, 48))
                } else {
                    Stroke::new(1.0, Color32::from_rgb(204, 198, 187))
                };
                let title_color = if is_active { INK } else { MUTED_INK };

                egui::Frame::NONE
                    .fill(tab_fill)
                    .stroke(tab_stroke)
                    .corner_radius(6)
                    .inner_margin(if is_active {
                        Margin::symmetric(10, 5)
                    } else {
                        Margin::symmetric(9, 4)
                    })
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let mut title = tab.title();
                            let dirty = if is_active {
                                self.annotations_dirty
                            } else {
                                tab.annotations_dirty
                            };
                            if dirty {
                                title.push_str(" •");
                            }
                            const MAX_TAB_CHARS: usize = 28;
                            if title.chars().count() > MAX_TAB_CHARS {
                                title = format!(
                                    "{}...",
                                    title.chars().take(MAX_TAB_CHARS - 3).collect::<String>()
                                );
                            }
                            let title = if is_active {
                                RichText::new(title).strong().color(title_color)
                            } else {
                                RichText::new(title).color(title_color)
                            };
                            if ui
                                .add(egui::Label::new(title).sense(Sense::click()))
                                .on_hover_text(tab.document.path.display().to_string())
                                .clicked()
                            {
                                switch_to = Some(index);
                            }
                            let close_text = RichText::new("x").color(if is_active {
                                Color32::from_rgb(104, 75, 43)
                            } else {
                                Color32::from_rgb(128, 122, 112)
                            });
                            if ui
                                .add(egui::Button::new(close_text).small().frame(false))
                                .on_hover_text("Close tab")
                                .clicked()
                            {
                                close_tab = Some(index);
                            }
                        });
                    });
            }

            if ui.small_button("+").on_hover_text("Open PDF").clicked() {
                self.open_dialog(ctx);
            }
        });

        if let Some(index) = close_tab {
            self.close_tab(index, ctx);
        } else if let Some(index) = switch_to {
            self.switch_to_tab(index, ctx);
        }
    }

    fn draw_toolbar(&mut self, ctx: &Context) {
        egui::TopBottomPanel::top("toolbar")
            .frame(
                egui::Frame::NONE
                    .fill(BAR_FILL)
                    .inner_margin(Margin::symmetric(10, 8))
                    .stroke(Stroke::new(1.0, Color32::from_rgb(222, 218, 208))),
            )
            .show(ctx, |ui| {
                self.draw_tab_strip(ui, ctx);
                ui.add_space(6.0);

                let has_document = self.document.is_some();
                let page_count = self
                    .document
                    .as_ref()
                    .map(|document| document.page_count)
                    .unwrap_or_default();

                ui.horizontal_wrapped(|ui| {
                    toolbar_group(ui, |ui| {
                        if ui.button("Open").on_hover_text("Open PDF").clicked() {
                            self.open_dialog(ctx);
                        }
                        if ui
                            .add_enabled(has_document, egui::Button::new("Save"))
                            .on_hover_text("Save highlights and comments into this PDF")
                            .clicked()
                        {
                            if let Err(error) = self.save_current_annotations() {
                                self.push_error_notice(error);
                            }
                        }
                        ui.add_enabled_ui(has_document, |ui| {
                            ui.menu_button("Export", |ui| {
                                if ui.button("Save PDF copy").clicked() {
                                    self.save_as_dialog();
                                    ui.close();
                                }
                                if ui.button("Text").clicked() {
                                    self.export_text_dialog(ctx);
                                    ui.close();
                                }
                                if ui.button("PNG").clicked() {
                                    self.export_png_dialog();
                                    ui.close();
                                }
                            });
                        });
                        #[cfg(target_os = "windows")]
                        if ui
                            .button("Set as default")
                            .on_hover_text(
                                "Open Windows Settings to choose LawPDF as the default PDF reader",
                            )
                            .clicked()
                        {
                            match open_windows_default_pdf_settings() {
                                Ok(()) => {
                                    self.status =
                                        "Windows Settings opened. Choose LawPDF for .pdf files."
                                            .to_owned();
                                }
                                Err(error) => {
                                    self.push_error_notice(format!(
                                        "Could not open Windows default-app settings: {error}"
                                    ));
                                }
                            }
                        }
                    });

                    ui.add_space(6.0);

                    toolbar_group(ui, |ui| {
                        if ui
                            .add_enabled(has_document, egui::Button::new("OCR PDF"))
                            .on_hover_text("Use OpenRouter OCR and save a searchable PDF copy")
                            .clicked()
                        {
                            self.sidebar_tab = SidebarTab::Search;
                            self.start_openrouter_ocr_save();
                        }
                    });

                    ui.add_space(6.0);

                    toolbar_group(ui, |ui| {
                        let pdf_active = self.view_mode == DocumentViewMode::Pdf;
                        if ui
                            .add_enabled(
                                has_document,
                                egui::Button::new("PDF").selected(pdf_active),
                            )
                            .on_hover_text("Original PDF view")
                            .clicked()
                        {
                            // #30: leaving a reflow view for the fixed layout is a doc-level
                            // "reflow rejected" signal.
                            if matches!(
                                self.view_mode,
                                DocumentViewMode::Liquid | DocumentViewMode::LiquidMode2
                            ) {
                                self.log_reflow_rejected(self.view_mode);
                            }
                            self.set_view_mode(DocumentViewMode::Pdf, ctx);
                        }
                        let liquid_active = self.view_mode == DocumentViewMode::LiquidMode2;
                        if ui
                            .add_enabled(
                                has_document,
                                egui::Button::new("Review Mode").selected(liquid_active),
                            )
                            .on_hover_text(
                                "Converts law review articles to a smooth reading experience.",
                            )
                            .clicked()
                        {
                            self.set_view_mode(DocumentViewMode::LiquidMode2, ctx);
                        }
                        if matches!(
                            self.liquid_mode2_state,
                            LiquidState::PreparingText | LiquidState::Preparing
                        ) {
                            ui.spinner();
                        }
                    });

                    ui.add_space(6.0);

                    toolbar_group(ui, |ui| {
                        ui.vertical(|ui| {
                            ui.horizontal(|ui| {
                                for (tool, label) in [
                                    (Tool::Select, "V"),
                                    (Tool::Marker, "M"),
                                    (Tool::TextBox, "T"),
                                    (Tool::Signature, "S"),
                                ] {
                                    ui.selectable_value(&mut self.active_tool, tool, label)
                                        .on_hover_text(tool.label());
                                }
                            });
                            if self.active_tool == Tool::Marker {
                                ui.add_space(2.0);
                                ui.horizontal(|ui| {
                                    self.draw_marker_default_palette(ui);
                                });
                            }
                        });
                    });

                    ui.add_space(6.0);

                    toolbar_group(ui, |ui| {
                        if ui
                            .add_enabled(has_document, egui::Button::new("-"))
                            .on_hover_text("Zoom out")
                            .clicked()
                        {
                            self.set_zoom(self.target_zoom / 1.15);
                        }
                        ui.label(format!("{:.0}%", self.zoom * 100.0));
                        if ui
                            .add_enabled(has_document, egui::Button::new("+"))
                            .on_hover_text("Zoom in")
                            .clicked()
                        {
                            self.set_zoom(self.target_zoom * 1.15);
                        }
                    });

                    ui.add_space(6.0);

                    toolbar_group(ui, |ui| {
                        if ui
                            .add_enabled(
                                has_document && self.page_index > 0,
                                egui::Button::new("<"),
                            )
                            .clicked()
                        {
                            self.go_to_page(self.page_index.saturating_sub(1));
                        }
                        ui.label(format!(
                            "{} / {}",
                            if has_document { self.page_index + 1 } else { 0 },
                            page_count
                        ));
                        if ui
                            .add_enabled(
                                has_document && self.page_index + 1 < page_count,
                                egui::Button::new(">"),
                            )
                            .clicked()
                        {
                            self.go_to_page(self.page_index + 1);
                        }
                    });

                    ui.add_space(6.0);

                    toolbar_group(ui, |ui| {
                        let search_response = ui.add_enabled(
                            has_document,
                            egui::TextEdit::singleline(&mut self.search_state.query)
                                .hint_text("Find")
                                .desired_width(190.0),
                        );
                        let pressed_enter = search_response.lost_focus()
                            && ui.input(|input| input.key_pressed(egui::Key::Enter));
                        if ui
                            .add_enabled(has_document, egui::Button::new("Find"))
                            .clicked()
                            || pressed_enter
                        {
                            self.start_search(ctx);
                            self.sidebar_tab = SidebarTab::Search;
                        }
                    });
                });
            });
    }

    fn draw_side_panel(&mut self, ctx: &Context) {
        egui::SidePanel::left("side_panel")
            .resizable(true)
            .default_width(292.0)
            .width_range(248.0..=430.0)
            .frame(
                egui::Frame::NONE
                    .fill(PANEL_FILL)
                    .inner_margin(Margin::symmetric(10, 10))
                    .stroke(Stroke::new(1.0, Color32::from_rgb(220, 216, 207))),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    for tab in SidebarTab::ALL {
                        if ui
                            .selectable_label(self.sidebar_tab == tab, tab.label())
                            .clicked()
                        {
                            self.sidebar_tab = tab;
                            if tab == SidebarTab::Pages {
                                self.thumbnail_scroll_target = Some(self.page_index);
                            }
                        }
                    }
                });
                ui.add_space(8.0);

                match self.sidebar_tab {
                    SidebarTab::Pages => self.draw_pages_tab(ui, ctx),
                    SidebarTab::Outline => self.draw_outline_tab(ui),
                    SidebarTab::Search => self.draw_search_tab(ui),
                    SidebarTab::Chat => self.draw_chat_tab(ui),
                    SidebarTab::Notes => self.draw_notes_tab(ui),
                }
            });
    }

    fn draw_pages_tab(&mut self, ui: &mut egui::Ui, ctx: &Context) {
        let Some(document) = self.document.as_ref() else {
            ui.label(RichText::new("No PDF loaded").color(MUTED_INK));
            return;
        };

        let path = document.path.clone();
        let pages = document.pages.clone();

        let requested_thumbnail = self.thumbnail_scroll_target;
        egui::ScrollArea::vertical().show(ui, |ui| {
            let viewport = ui.clip_rect();
            let render_window = viewport.expand2(Vec2::new(0.0, 700.0));

            for (page_index, page_info) in pages.iter().enumerate() {
                let visible_range = self.visible_range_for_page(page_index);
                let target_visible = visible_range
                    .map(|range| range.coverage.max(0.24))
                    .unwrap_or_else(|| {
                        if self.visible_page_ranges.is_empty() && page_index == self.page_index {
                            1.0
                        } else {
                            0.0
                        }
                    });
                let visible_t = ui.ctx().animate_value_with_time(
                    ui.id().with(("thumb-visible", page_index)),
                    target_visible,
                    0.14,
                );
                let fill = lerp_color(
                    Color32::from_rgb(250, 248, 242),
                    Color32::from_rgb(231, 224, 210),
                    visible_t,
                );
                let stroke = Stroke::new(
                    1.0 + 1.0 * visible_t,
                    lerp_color(
                        Color32::from_rgb(213, 208, 198),
                        Color32::from_rgb(146, 103, 52),
                        visible_t,
                    ),
                );

                let inner = egui::Frame::NONE
                    .fill(fill)
                    .stroke(stroke)
                    .corner_radius(6)
                    .inner_margin(Margin::same(8))
                    .show(ui, |ui| {
                        ui.vertical_centered(|ui| {
                            let display_width = 118.0;
                            let display_height =
                                display_width * page_info.height / page_info.width.max(1.0);
                            let display_size = Vec2::new(display_width, display_height);
                            let (thumb_rect, response) =
                                ui.allocate_exact_size(display_size, Sense::click());
                            let should_render = visible_t > 0.01
                                || requested_thumbnail == Some(page_index)
                                || thumb_rect.intersects(render_window);
                            let painter = ui.painter_at(thumb_rect);

                            painter.rect_filled(thumb_rect, 2, PAPER_FILL);
                            painter.rect_stroke(
                                thumb_rect,
                                2,
                                Stroke::new(1.0, PAPER_STROKE),
                                egui::StrokeKind::Inside,
                            );

                            if should_render {
                                if let Some(thumbnail) = self.ensure_thumbnail_texture(
                                    ctx,
                                    &path,
                                    page_index,
                                    page_info.width,
                                ) {
                                    painter.image(
                                        thumbnail.texture_id,
                                        thumb_rect,
                                        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                                        Color32::WHITE,
                                    );
                                } else {
                                    painter.text(
                                        thumb_rect.center(),
                                        Align2::CENTER_CENTER,
                                        "Rendering",
                                        FontId::proportional(13.0),
                                        MUTED_INK,
                                    );
                                }
                            } else {
                                painter.text(
                                    thumb_rect.center(),
                                    Align2::CENTER_CENTER,
                                    format!("{}", page_index + 1),
                                    FontId::proportional(18.0),
                                    MUTED_INK,
                                );
                            }
                            if let Some(range) = visible_range {
                                let top = thumb_rect.top()
                                    + thumb_rect.height() * range.top_fraction.clamp(0.0, 1.0);
                                let bottom = thumb_rect.top()
                                    + thumb_rect.height() * range.bottom_fraction.clamp(0.0, 1.0);
                                let bottom = bottom.max(top + 3.0).min(thumb_rect.bottom());
                                let visible_rect = Rect::from_min_max(
                                    Pos2::new(thumb_rect.left(), top),
                                    Pos2::new(thumb_rect.right(), bottom),
                                );
                                let alpha =
                                    (42.0 + 86.0 * range.coverage).round().clamp(0.0, 160.0) as u8;
                                painter.rect_filled(
                                    visible_rect,
                                    1,
                                    Color32::from_rgba_unmultiplied(207, 150, 64, alpha),
                                );
                                painter.rect_stroke(
                                    visible_rect,
                                    1,
                                    Stroke::new(
                                        1.4,
                                        Color32::from_rgba_unmultiplied(146, 103, 52, 190),
                                    ),
                                    egui::StrokeKind::Inside,
                                );
                            }
                            response
                        })
                        .inner
                    });

                if self.thumbnail_scroll_target == Some(page_index) {
                    inner.response.scroll_to_me_animation(
                        Some(Align::Center),
                        egui::style::ScrollAnimation::duration(THUMBNAIL_SCROLL_SECONDS),
                    );
                    self.thumbnail_scroll_target = None;
                }

                ui.horizontal(|ui| {
                    ui.add_space(6.0);
                    ui.label(RichText::new(format!("Page {}", page_index + 1)).color(INK));
                });

                if inner.response.clicked() || inner.inner.clicked() {
                    self.go_to_page(page_index);
                }
                ui.add_space(10.0);
            }
        });
    }

    fn draw_outline_tab(&mut self, ui: &mut egui::Ui) {
        ui.heading("Outline");
        ui.add_space(6.0);
        ui.label(RichText::new("No outline detected.").color(MUTED_INK));
        if let Some(document) = self.document.as_ref() {
            ui.add_space(10.0);
            ui.label(RichText::new(&document.title).strong());
            ui.label(RichText::new(format!("{} pages", document.page_count)).color(MUTED_INK));
        }
    }

    fn draw_search_tab(&mut self, ui: &mut egui::Ui) {
        let has_document = self.document.is_some();

        ui.horizontal(|ui| {
            let search_response = ui.add_enabled(
                has_document,
                egui::TextEdit::singleline(&mut self.search_state.query)
                    .hint_text("Find")
                    .desired_width(168.0),
            );
            if self.search_state.focus_request {
                if has_document {
                    search_response.request_focus();
                }
                self.search_state.focus_request = false;
            }
            let pressed_enter = search_response.lost_focus()
                && ui.input(|input| input.key_pressed(egui::Key::Enter));
            if ui
                .add_enabled(has_document, egui::Button::new("Find"))
                .clicked()
                || pressed_enter
            {
                self.start_search(ui.ctx());
            }
        });

        ui.horizontal(|ui| {
            ui.checkbox(&mut self.search_state.show_highlights, "show hits");
            if ui
                .add_enabled(
                    has_document && !self.search_state.hits.is_empty(),
                    egui::Button::new("Annotate"),
                )
                .clicked()
            {
                self.add_search_highlights();
            }
        });

        ui.add_space(6.0);
        ui.label(
            RichText::new(format!("{} result(s)", self.search_state.hits.len()))
                .color(MUTED_INK),
        );
        egui::ScrollArea::vertical()
            .max_height(280.0)
            .show(ui, |ui| {
                let hits = self.search_state.hits.clone();
                for (index, hit) in hits.iter().enumerate() {
                    let label = format!(
                        "p{} [{}] {}",
                        hit.page_index + 1,
                        hit.source.label(),
                        hit.snippet
                    );
                    if ui
                        .selectable_label(self.search_state.selected_hit == Some(index), label)
                        .clicked()
                    {
                        self.search_state.selected_hit = Some(index);
                        self.go_to_page(hit.page_index);
                    }
                }
            });

        ui.add_space(10.0);
        ui.separator();
        ui.add_space(8.0);
        if ui
            .add_enabled(has_document, egui::Button::new("Run OCR"))
            .clicked()
        {
            self.start_ocr();
        }
        if ui
            .add_enabled(has_document, egui::Button::new("OCR PDF"))
            .on_hover_text("Use OpenRouter OCR and save a searchable PDF copy")
            .clicked()
        {
            self.start_openrouter_ocr_save();
        }
        self.draw_ocr_status(ui);
    }

    fn draw_notes_tab(&mut self, ui: &mut egui::Ui) {
        ui.heading("Tools");
        ui.horizontal_wrapped(|ui| {
            for tool in Tool::ALL {
                ui.selectable_value(&mut self.active_tool, tool, tool.label());
            }
        });

        ui.add_space(8.0);
        ui.label("Marker opacity");
        ui.add(egui::Slider::new(&mut self.marker_opacity, 0.1..=0.9));

        if ui
            .checkbox(&mut self.settings.reduce_motion, "Reduce highlight motion")
            .on_hover_text("Highlights appear instantly instead of animating the ink stroke.")
            .changed()
        {
            if let Err(error) = save_settings(&self.settings) {
                self.push_error_notice(format!("Could not save motion setting: {error}"));
            }
        }
        if ui
            .checkbox(
                &mut self.settings.liquid_mode2_use_pymupdf_blocks,
                "Use PyMuPDF block grouping for Review Mode",
            )
            .on_hover_text("Use PyMuPDF paragraph boxes for Review Mode block assembly; detector fallback remains available when the sidecar cannot find text blocks.")
            .changed()
        {
            if let Err(error) = save_settings(&self.settings) {
                self.push_error_notice(format!(
                    "Could not save Review Mode grouping setting: {error}"
                ));
            }
        }
        if ui
            .checkbox(
                &mut self.settings.liquid_mode2_use_pp_footnote_regions,
                "Use PP footnote regions for Review Mode",
            )
            .on_hover_text("Use high-confidence PP footnote-region membership as a default-off Review Mode marginalia override.")
            .changed()
        {
            if let Err(error) = save_settings(&self.settings) {
                self.push_error_notice(format!(
                    "Could not save Review Mode footnote-region setting: {error}"
                ));
            }
        }

        ui.add_space(8.0);
        ui.label("Comment color");
        ui.horizontal(|ui| {
            for (index, preset) in COMMENT_COLOR_PRESETS.iter().enumerate() {
                let selected = self.comment_color_index == index;
                let stroke = if selected {
                    Stroke::new(2.0, Color32::from_rgb(72, 48, 26))
                } else {
                    Stroke::new(1.0, Color32::from_rgb(210, 200, 184))
                };
                if ui
                    .add(
                        egui::Button::new("")
                            .fill(color_from_rgb(preset.color_rgb, 235))
                            .stroke(stroke)
                            .min_size(Vec2::splat(20.0)),
                    )
                    .on_hover_text(preset.label)
                    .clicked()
                {
                    self.comment_color_index = index;
                }
            }
        });

        ui.add_space(8.0);
        ui.label("Text box");
        ui.add(
            egui::TextEdit::multiline(&mut self.text_box_text)
                .desired_rows(3)
                .lock_focus(true),
        );

        ui.add_space(8.0);
        ui.label("Signer");
        ui.text_edit_singleline(&mut self.signer_name);

        ui.add_space(12.0);
        ui.separator();
        ui.add_space(8.0);
        ui.heading("Comments");
        let comments = self
            .annotations
            .iter()
            .enumerate()
            .filter_map(|(index, annotation)| {
                if let AnnotationKind::Comment {
                    text, color_rgb, ..
                } = &annotation.kind
                {
                    Some((index, annotation.page_index, text.clone(), *color_rgb))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        if comments.is_empty() {
            ui.label(RichText::new("No comments").color(MUTED_INK));
        } else {
            egui::ScrollArea::vertical()
                .max_height(180.0)
                .show(ui, |ui| {
                    for (index, page_index, text, color_rgb) in comments {
                        ui.horizontal(|ui| {
                            let (swatch_rect, _) =
                                ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
                            ui.painter().rect_filled(
                                swatch_rect,
                                2,
                                color_from_rgb(color_rgb, 230),
                            );
                            let label = format!(
                                "p{} {}",
                                page_index + 1,
                                comment_preview(&text).unwrap_or_else(|| "Comment".to_owned())
                            );
                            if ui
                                .selectable_label(self.selected_comment == Some(index), label)
                                .clicked()
                            {
                                self.go_to_page(page_index);
                                self.start_comment_edit(index);
                            }
                        });
                    }
                });
        }

        ui.add_space(12.0);
        ui.separator();
        ui.add_space(8.0);
        ui.heading("Document");
        if let Some(document) = self.document.as_ref() {
            ui.label(&document.title);
            ui.label(RichText::new(format!("{} page(s)", document.page_count)).color(MUTED_INK));
            ui.label(
                RichText::new(format!("{} annotation(s)", self.annotations.len())).color(MUTED_INK),
            );
        } else {
            ui.label(RichText::new("No PDF loaded").color(MUTED_INK));
        }
    }

    fn draw_ocr_status(&self, ui: &mut egui::Ui) {
        if self.ocr_states.is_empty() {
            ui.label("OCR idle");
            return;
        }

        let total = self.ocr_states.len();
        let done = self
            .ocr_states
            .iter()
            .filter(|state| matches!(state, OcrPageState::Done(_)))
            .count();
        let running = self
            .ocr_states
            .iter()
            .filter(|state| matches!(state, OcrPageState::Running))
            .count();
        let failed = self
            .ocr_states
            .iter()
            .filter(|state| matches!(state, OcrPageState::Failed(_)))
            .count();
        let queued = self
            .ocr_states
            .iter()
            .filter(|state| matches!(state, OcrPageState::Pending))
            .count();
        let idle = self
            .ocr_states
            .iter()
            .filter(|state| matches!(state, OcrPageState::Idle))
            .count();
        if done == 0 && running == 0 && queued == 0 && failed == 0 {
            ui.label("OCR idle");
            return;
        }

        let completed = done + failed;
        let progress = completed as f32 / total.max(1) as f32;
        let eta = if running > 0 || queued > 0 {
            self.ocr_eta_label(completed, total)
        } else if completed >= total {
            "complete".to_owned()
        } else if failed > 0 {
            "stopped".to_owned()
        } else {
            "cached".to_owned()
        };

        ui.add(
            egui::ProgressBar::new(progress)
                .desired_width(ui.available_width())
                .show_percentage()
                .text(format!("{completed}/{total} pages - {eta}")),
        );
        ui.label(
            RichText::new(format!(
                "{done} done, {running} running, {queued} queued, {idle} idle, {failed} failed"
            ))
            .color(MUTED_INK),
        );

        if let Some(state) = self.ocr_states.get(self.page_index) {
            ui.label(format!("Current page: {}", state.label()));
        }

        let completed_pages = self
            .ocr_states
            .iter()
            .enumerate()
            .filter_map(|(page_index, state)| state.text().map(|text| (page_index, text)))
            .collect::<Vec<_>>();
        if completed_pages.is_empty() {
            return;
        }

        ui.add_space(8.0);
        ui.label(RichText::new("Recognized text").strong().color(INK));
        egui::ScrollArea::vertical()
            .max_height(220.0)
            .show(ui, |ui| {
                for (page_index, text) in completed_pages.iter().rev().take(8).rev() {
                    ui.collapsing(
                        format!("Page {} - {} chars", page_index + 1, text.chars().count()),
                        |ui| {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(text_preview(text, 900)).size(13.0).color(INK),
                                )
                                .wrap(),
                            );
                        },
                    );
                }
            });
    }

    fn draw_status_bar(&mut self, ctx: &Context) {
        egui::TopBottomPanel::bottom("status_bar")
            .frame(
                egui::Frame::NONE
                    .fill(BAR_FILL)
                    .inner_margin(Margin::symmetric(10, 6))
                    .stroke(Stroke::new(1.0, Color32::from_rgb(222, 218, 208))),
            )
            .show(ctx, |ui| {
                let page_count = self
                    .document
                    .as_ref()
                    .map(|document| document.page_count)
                    .unwrap_or_default();
                let page_text = if page_count > 0 && self.view_mode == DocumentViewMode::Pdf {
                    format!("Page {} of {}", self.page_index + 1, page_count)
                } else if page_count > 0 && self.view_mode == DocumentViewMode::LiquidMode2 {
                    self.liquid_mode2_state.label().to_owned()
                } else if page_count > 0 {
                    self.liquid_state.label().to_owned()
                } else {
                    "No document".to_owned()
                };

                ui.horizontal(|ui| {
                    ui.label(RichText::new(page_text).color(INK));
                    ui.separator();
                    let mode_text = match self.view_mode {
                        DocumentViewMode::Pdf => format!("{:.0}% zoom", self.zoom * 100.0),
                        DocumentViewMode::Liquid => "Liquid view".to_owned(),
                        DocumentViewMode::LiquidMode2 => "Review Mode".to_owned(),
                    };
                    ui.label(RichText::new(mode_text).color(INK));
                    ui.separator();
                    ui.label(RichText::new(self.active_tool.label()).color(INK));
                    ui.separator();
                    ui.label(RichText::new(self.ocr_summary()).color(MUTED_INK));
                    ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                        ui.label(RichText::new(&self.status).color(MUTED_INK));
                    });
                });
            });
    }

    fn draw_unsaved_close_prompt(&mut self, ctx: &Context) {
        if !self.show_unsaved_close_prompt {
            return;
        }
        egui::Window::new("Save changes before closing?")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label("Highlights or comments have not been saved into their PDF files.");
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("Save and close").clicked() {
                        match self.save_all_dirty_annotations() {
                            Ok(()) => {
                                self.show_unsaved_close_prompt = false;
                                self.allow_window_close = true;
                                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                            }
                            Err(error) => self.push_error_notice(error),
                        }
                    }
                    if ui.button("Don't save").clicked() {
                        self.show_unsaved_close_prompt = false;
                        self.allow_window_close = true;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    if ui.button("Cancel").clicked() {
                        self.show_unsaved_close_prompt = false;
                    }
                });
            });
    }

    fn push_notice(&mut self, message: impl Into<String>, severity: NoticeSeverity) {
        enqueue_notice(&mut self.notices, Notice::new(message, severity));
    }

    fn push_error_notice(&mut self, message: impl Into<String>) {
        self.push_notice(message, NoticeSeverity::Error);
    }

    fn push_info_notice(&mut self, message: impl Into<String>) {
        self.push_notice(message, NoticeSeverity::Info);
    }

    fn draw_notices(&mut self, ctx: &Context) {
        let now = Instant::now();
        prune_notices_at(&mut self.notices, now);
        if self.notices.is_empty() {
            return;
        }

        let reduce_motion = self.settings.reduce_motion;
        let notices = self.notices.iter().cloned().collect::<Vec<_>>();
        let mut dismiss = None;
        let mut animating = false;
        egui::Area::new(egui::Id::new("general_notices"))
            .order(egui::Order::Foreground)
            .anchor(Align2::RIGHT_BOTTOM, Vec2::new(-24.0, -56.0))
            .show(ctx, |ui| {
                ui.vertical(|ui| {
                    for (index, notice) in notices.iter().enumerate().rev() {
                        let age = now.saturating_duration_since(notice.created_at);
                        let visibility = if reduce_motion {
                            1.0
                        } else {
                            let visibility = (age.as_secs_f32() / 0.18).clamp(0.0, 1.0);
                            animating |= visibility < 1.0;
                            visibility
                        };
                        let alpha = (246.0 * visibility) as u8;
                        let accent = match notice.severity {
                            NoticeSeverity::Info => Color32::from_rgb(55, 104, 137),
                            NoticeSeverity::Error => Color32::from_rgb(164, 54, 54),
                        };
                        egui::Frame::NONE
                            .fill(Color32::from_rgba_unmultiplied(255, 254, 250, alpha))
                            .stroke(Stroke::new(
                                1.0,
                                Color32::from_rgba_unmultiplied(
                                    accent.r(),
                                    accent.g(),
                                    accent.b(),
                                    alpha,
                                ),
                            ))
                            .corner_radius(8)
                            .inner_margin(Margin::symmetric(14, 10))
                            .show(ui, |ui| {
                                ui.set_width(360.0);
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(match notice.severity {
                                            NoticeSeverity::Info => "i",
                                            NoticeSeverity::Error => "!",
                                        })
                                        .strong()
                                        .color(accent),
                                    );
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(&notice.message).color(INK).size(13.5),
                                        )
                                        .wrap(),
                                    );
                                    if ui
                                        .small_button(RichText::new("x").color(MUTED_INK))
                                        .on_hover_text("Dismiss")
                                        .clicked()
                                    {
                                        dismiss = Some(index);
                                    }
                                });
                            });
                        ui.add_space(6.0);
                    }
                });
            });

        if let Some(index) = dismiss {
            self.notices.remove(index);
        }
        if animating {
            ctx.request_repaint_after(Duration::from_millis(16));
        } else if self
            .notices
            .iter()
            .any(|notice| notice.severity == NoticeSeverity::Info)
        {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    fn draw_update_notice(&mut self, ctx: &Context) {
        let Some(notice) = self.update_ui.notice.as_ref() else {
            return;
        };
        if notice.is_expired() {
            return;
        }

        egui::Area::new(egui::Id::new("update_notice"))
            .order(egui::Order::Foreground)
            .anchor(Align2::RIGHT_TOP, Vec2::new(-24.0, 24.0))
            .show(ctx, |ui| {
                egui::Frame::NONE
                    .fill(Color32::from_rgba_unmultiplied(255, 254, 250, 246))
                    .stroke(Stroke::new(1.0, Color32::from_rgb(205, 197, 181)))
                    .corner_radius(8)
                    .inner_margin(Margin::symmetric(16, 12))
                    .show(ui, |ui| {
                        ui.set_width(330.0);
                        ui.horizontal(|ui| {
                            if matches!(notice.kind, UpdateNoticeKind::Working) {
                                ui.spinner();
                            }
                            ui.label(
                                RichText::new(&notice.message)
                                    .strong()
                                    .size(15.0)
                                    .color(INK),
                            );
                        });
                    });
            });

        ctx.request_repaint_after(Duration::from_millis(250));
    }

    fn draw_liquid_status_popover(&mut self, ctx: &Context) {
        if self.document.is_none() || self.view_mode != DocumentViewMode::Pdf {
            return;
        }

        let state = self.liquid_state.clone();
        let should_show = matches!(state, LiquidState::PreparingText | LiquidState::Preparing)
            || (!self.liquid_notice_dismissed
                && matches!(state, LiquidState::Ready(_) | LiquidState::Failed(_)));
        if !should_show {
            return;
        }

        let mut open_liquid = false;
        let mut retry = false;
        let mut dismiss = false;
        egui::Area::new(egui::Id::new("liquid_status_popover"))
            .order(egui::Order::Foreground)
            .anchor(Align2::RIGHT_TOP, Vec2::new(-24.0, 118.0))
            .show(ctx, |ui| {
                egui::Frame::NONE
                    .fill(Color32::from_rgba_unmultiplied(255, 254, 250, 238))
                    .stroke(Stroke::new(1.0, Color32::from_rgb(205, 197, 181)))
                    .corner_radius(8)
                    .inner_margin(Margin::symmetric(14, 12))
                    .show(ui, |ui| {
                        ui.set_width(310.0);
                        match state {
                            LiquidState::PreparingText => {
                                ui.horizontal(|ui| {
                                    ui.spinner();
                                    ui.label(RichText::new("Preparing PDF Text").strong());
                                });
                                ui.add_space(8.0);
                                ui.add(
                                    egui::ProgressBar::new(0.30)
                                        .animate(true)
                                        .desired_width(286.0)
                                        .text("Loading page text..."),
                                );
                                ui.label(
                                    RichText::new("The LLM request has not been sent yet.")
                                        .color(MUTED_INK)
                                        .size(13.0),
                                );
                            }
                            LiquidState::Preparing => {
                                ui.horizontal(|ui| {
                                    ui.spinner();
                                    ui.label(RichText::new("Preparing Review Mode").strong());
                                });
                                ui.add_space(8.0);
                                ui.add(
                                    egui::ProgressBar::new(0.55)
                                        .animate(true)
                                        .desired_width(286.0)
                                        .text("LLM layout running..."),
                                );
                                ui.label(
                                    RichText::new("You can keep reading the PDF.")
                                        .color(MUTED_INK)
                                        .size(13.0),
                                );
                            }
                            LiquidState::Ready(document) => {
                                let engine =
                                    document.llm_provider.as_deref().unwrap_or_else(|| {
                                        if document.llm_used {
                                            "LLM"
                                        } else {
                                            "local fallback"
                                        }
                                    });
                                ui.label(RichText::new("Review Mode Ready").strong().color(INK));
                                ui.label(
                                    RichText::new(format!("Prepared with {engine}."))
                                        .color(MUTED_INK)
                                        .size(13.0),
                                );
                                if let Some(warning) = document.warnings.first() {
                                    ui.label(
                                        RichText::new(text_preview(warning, 160))
                                            .color(Color32::from_rgb(134, 92, 34))
                                            .size(12.5),
                                    );
                                }
                                ui.add_space(8.0);
                                ui.horizontal(|ui| {
                                    if ui.button("Open").clicked() {
                                        open_liquid = true;
                                    }
                                    if ui.button("Hide").clicked() {
                                        dismiss = true;
                                    }
                                });
                            }
                            LiquidState::Failed(error) => {
                                ui.label(
                                    RichText::new("Review Mode Failed")
                                        .strong()
                                        .color(Color32::from_rgb(132, 49, 42)),
                                );
                                ui.label(
                                    RichText::new(text_preview(&error, 180))
                                        .color(MUTED_INK)
                                        .size(12.5),
                                );
                                ui.add_space(8.0);
                                ui.horizontal(|ui| {
                                    if ui.button("Retry").clicked() {
                                        retry = true;
                                    }
                                    if ui.button("Hide").clicked() {
                                        dismiss = true;
                                    }
                                });
                            }
                            LiquidState::Idle => {}
                        }
                    });
            });

        if open_liquid {
            self.set_view_mode(DocumentViewMode::Liquid, ctx);
        }
        if retry {
            self.liquid_state = LiquidState::Idle;
            self.liquid_notice_dismissed = false;
            self.ensure_liquid_started(ctx);
        }
        if dismiss {
            self.liquid_notice_dismissed = true;
        }
    }

    fn draw_empty_state(&mut self, ui: &mut egui::Ui, ctx: &Context) {
        let available = ui.available_rect_before_wrap();
        let drop_rect = Rect::from_center_size(
            available.center(),
            Vec2::new(available.width().min(520.0), 260.0),
        );
        let response = ui.allocate_rect(drop_rect, Sense::click());
        if response.clicked() {
            self.open_dialog(ctx);
        }

        let hovering_drop = ctx.input(|input| !input.raw.hovered_files.is_empty());
        let fill = if hovering_drop {
            Color32::from_rgb(246, 241, 229)
        } else {
            Color32::from_rgb(250, 248, 242)
        };
        let stroke = if hovering_drop {
            Stroke::new(1.8, Color32::from_rgb(164, 119, 63))
        } else {
            Stroke::new(1.0, Color32::from_rgb(211, 205, 193))
        };

        let painter = ui.painter();
        painter.add(
            Shadow {
                offset: [0, 10],
                blur: 28,
                spread: 0,
                color: Color32::from_black_alpha(32),
            }
            .as_shape(drop_rect, 8),
        );
        painter.rect_filled(drop_rect, 8, fill);
        painter.rect_stroke(drop_rect, 8, stroke, egui::StrokeKind::Inside);
        painter.text(
            drop_rect.center_top() + Vec2::new(0.0, 74.0),
            Align2::CENTER_CENTER,
            "Drop PDF here",
            FontId::proportional(30.0),
            INK,
        );
        painter.text(
            drop_rect.center_top() + Vec2::new(0.0, 118.0),
            Align2::CENTER_CENTER,
            "or click to open",
            FontId::proportional(18.0),
            MUTED_INK,
        );

        if let Some(error) = self.startup_error.as_ref() {
            painter.text(
                drop_rect.center_top() + Vec2::new(0.0, 170.0),
                Align2::CENTER_CENTER,
                error,
                FontId::proportional(14.0),
                Color32::from_rgb(164, 58, 46),
            );
        }
    }

    fn handle_dropped_files(&mut self, ctx: &Context) {
        let dropped_files = ctx.input(|input| input.raw.dropped_files.clone());
        let paths = dropped_files
            .into_iter()
            .filter_map(|file| file.path)
            .collect::<Vec<_>>();
        if !paths.is_empty() {
            self.open_paths_in_tabs(paths, ctx, true);
        }
    }

    fn ocr_summary(&self) -> String {
        if self.ocr_states.is_empty() {
            return "OCR idle".to_owned();
        }

        let done = self
            .ocr_states
            .iter()
            .filter(|state| matches!(state, OcrPageState::Done(_)))
            .count();
        let running = self
            .ocr_states
            .iter()
            .filter(|state| matches!(state, OcrPageState::Running))
            .count();
        let failed = self
            .ocr_states
            .iter()
            .filter(|state| matches!(state, OcrPageState::Failed(_)))
            .count();

        if running > 0 || done > 0 || failed > 0 {
            format!("OCR {done}/{}, {failed} failed", self.ocr_states.len())
        } else {
            "OCR idle".to_owned()
        }
    }

    fn ocr_is_active(&self) -> bool {
        self.ocr_states
            .iter()
            .any(|state| matches!(state, OcrPageState::Pending | OcrPageState::Running))
    }

    fn ocr_eta_label(&self, completed: usize, total: usize) -> String {
        if total == 0 || completed >= total {
            return "complete".to_owned();
        }

        let Some(progress) = self.ocr_progress else {
            return "ETA calculating".to_owned();
        };
        let newly_completed = completed.saturating_sub(progress.initial_completed);
        if newly_completed == 0 {
            return "ETA calculating".to_owned();
        }

        let elapsed = progress.started_at.elapsed().as_secs_f64();
        if elapsed <= 0.0 {
            return "ETA calculating".to_owned();
        }

        let seconds_per_page = elapsed / newly_completed as f64;
        let remaining = total.saturating_sub(completed) as f64;
        format!("ETA {}", human_duration(seconds_per_page * remaining))
    }

    fn draw_liquid_document(&mut self, ui: &mut egui::Ui, ctx: &Context) {
        let state = self.liquid_state.clone();
        egui::ScrollArea::vertical()
            .id_salt("liquid_document")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let available = ui.available_width();
                let margin_width = 0.0;
                let width = available
                    .min(self.liquid_max_width)
                    .max(360.0)
                    .min(available.max(360.0));
                let side = ((available - width) * 0.5).max(0.0);
                ui.horizontal(|ui| {
                    ui.add_space(side);
                    ui.vertical(|ui| {
                        ui.set_width(width);
                        ui.add_space(28.0);
                        match state {
                            LiquidState::Idle => {
                                ui.label(RichText::new("Review Mode").size(26.0).strong());
                                if ui.button("Prepare").clicked() {
                                    self.ensure_liquid_started(ctx);
                                }
                            }
                            LiquidState::PreparingText => {
                                ui.add_space(80.0);
                                ui.spinner();
                                ui.label(RichText::new("Preparing PDF text").size(20.0));
                                ui.label(
                                    RichText::new("The LLM request will send after text loads.")
                                        .color(self.liquid_muted_color()),
                                );
                            }
                            LiquidState::Preparing => {
                                ui.add_space(80.0);
                                ui.spinner();
                                ui.label(RichText::new("Preparing Review Mode").size(20.0));
                                ui.label(
                                    RichText::new("Formatting text...")
                                        .color(self.liquid_muted_color()),
                                );
                            }
                            LiquidState::Failed(error) => {
                                ui.add_space(48.0);
                                ui.label(
                                    RichText::new("Review Mode unavailable")
                                        .size(22.0)
                                        .strong()
                                        .color(Color32::from_rgb(132, 49, 42)),
                                );
                                ui.label(RichText::new(error).color(self.liquid_muted_color()));
                                if ui.button("Retry").clicked() {
                                    self.liquid_state = LiquidState::Idle;
                                    self.ensure_liquid_started(ctx);
                                }
                            }
                            LiquidState::Ready(document) => {
                                self.draw_liquid_controls(ui);
                                self.draw_liquid_tts_controls(ui, &document);
                                self.liquid_footnote_index =
                                    build_liquid_footnote_index(&document.blocks);
                                self.draw_liquid_header(ui, &document);
                                if liquid_document_needs_ocr(&document) {
                                    self.draw_liquid_ocr_actions(ui, ctx);
                                } else if let Some(hint) = liquid_reflow_low_confidence(&document) {
                                    // #23: confidence-gated fallback affordance.
                                    self.draw_liquid_reflow_gate(ui, ctx, hint);
                                }
                                let outline = liquid_outline_items(&document.blocks);
                                self.draw_liquid_outline(ui, &outline);
                                let notes = liquid_note_blocks(&document.blocks);
                                let hidden_contents =
                                    hidden_contents_mask_for_display(&document.blocks);
                                let mut block_index = 0usize;
                                while block_index < document.blocks.len() {
                                    let block = &document.blocks[block_index];
                                    if hidden_contents.get(block_index).copied().unwrap_or(false)
                                        || should_hide_contents_block_for_display(block)
                                    {
                                        // #29: reveal hidden furniture (dimmed) when toggled on.
                                        if self.liquid_show_hidden_furniture {
                                            self.draw_liquid_hidden_furniture_block(ui, block);
                                        }
                                        block_index += 1;
                                        continue;
                                    }
                                    if block.role == LiquidBlockRole::Marginalia {
                                        let mut margin_notes = Vec::new();
                                        while block_index < document.blocks.len() {
                                            let note = &document.blocks[block_index];
                                            if hidden_contents
                                                .get(block_index)
                                                .copied()
                                                .unwrap_or(false)
                                                || should_hide_contents_block_for_display(note)
                                            {
                                                block_index += 1;
                                                continue;
                                            }
                                            if note.role != LiquidBlockRole::Marginalia {
                                                break;
                                            }
                                            margin_notes.push((block_index, note));
                                            block_index += 1;
                                        }
                                        self.draw_liquid_reader_row(
                                            ui,
                                            &document,
                                            None,
                                            None,
                                            &margin_notes,
                                            width,
                                            margin_width,
                                        );
                                        continue;
                                    }
                                    if block.role == LiquidBlockRole::Metadata {
                                        let start = block_index;
                                        while block_index < document.blocks.len()
                                            && document.blocks[block_index].role
                                                == LiquidBlockRole::Metadata
                                        {
                                            block_index += 1;
                                        }
                                        let visible_metadata = document.blocks[start..block_index]
                                            .iter()
                                            .enumerate()
                                            .filter(|(offset, block)| {
                                                !hidden_contents
                                                    .get(start + offset)
                                                    .copied()
                                                    .unwrap_or(false)
                                                    && !should_hide_contents_block_for_display(
                                                        block,
                                                    )
                                            })
                                            .map(|(_, block)| block.clone())
                                            .collect::<Vec<_>>();
                                        if !visible_metadata.is_empty() {
                                            self.draw_liquid_metadata_group(ui, &visible_metadata);
                                        }
                                        continue;
                                    }
                                    let mut margin_notes = Vec::new();
                                    let mut next_index = block_index + 1;
                                    while next_index < document.blocks.len() {
                                        let note = &document.blocks[next_index];
                                        if hidden_contents.get(next_index).copied().unwrap_or(false)
                                            || should_hide_contents_block_for_display(note)
                                        {
                                            next_index += 1;
                                            continue;
                                        }
                                        if note.role != LiquidBlockRole::Marginalia {
                                            break;
                                        }
                                        margin_notes.push((next_index, note));
                                        next_index += 1;
                                    }
                                    self.draw_liquid_reader_row(
                                        ui,
                                        &document,
                                        Some(block_index),
                                        Some(block),
                                        &margin_notes,
                                        width,
                                        margin_width,
                                    );
                                    block_index = next_index;
                                }
                                self.draw_liquid_notes(ui, &notes);
                                ui.add_space(40.0);
                            }
                        }
                    });
                });
            });
    }

    fn draw_liquid_mode2_document(&mut self, ui: &mut egui::Ui, ctx: &Context) {
        let state = self.liquid_mode2_state.clone();
        egui::ScrollArea::vertical()
            .id_salt("liquid_mode2_document")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let available = ui.available_width();
                let margin_width = 0.0;
                let width = available
                    .min(self.liquid_max_width)
                    .max(360.0)
                    .min(available.max(360.0));
                let side = ((available - width) * 0.5).max(0.0);
                ui.horizontal(|ui| {
                    ui.add_space(side);
                    ui.vertical(|ui| {
                        ui.set_width(width);
                        ui.add_space(28.0);
                        match state {
                            LiquidState::Idle => {
                                ui.label(RichText::new("Review Mode").size(26.0).strong());
                                if ui.button("Prepare Review Mode").clicked() {
                                    self.ensure_liquid_mode2_started(ctx);
                                }
                            }
                            LiquidState::PreparingText => {
                                ui.add_space(80.0);
                                ui.spinner();
                                ui.label(RichText::new("Preparing PDF layout").size(20.0));
                                ui.label(
                                    RichText::new("Review Mode is extracting line geometry.")
                                        .color(self.liquid_muted_color()),
                                );
                                self.draw_liquid_loading_fallback(ui, ctx);
                            }
                            LiquidState::Preparing => {
                                ui.add_space(80.0);
                                ui.spinner();
                                ui.label(RichText::new("Preparing Review Mode").size(20.0));
                                ui.label(
                                    RichText::new("Decoding page-level reading flow...")
                                        .color(self.liquid_muted_color()),
                                );
                                self.draw_liquid_loading_fallback(ui, ctx);
                            }
                            LiquidState::Failed(error) => {
                                ui.add_space(48.0);
                                ui.label(
                                    RichText::new("Review Mode unavailable")
                                        .size(22.0)
                                        .strong()
                                        .color(Color32::from_rgb(132, 49, 42)),
                                );
                                ui.label(RichText::new(error).color(self.liquid_muted_color()));
                                if ui.button("Retry Review Mode").clicked() {
                                    self.liquid_mode2_state = LiquidState::Idle;
                                    self.ensure_liquid_mode2_started(ctx);
                                }
                            }
                            LiquidState::Ready(document) => {
                                self.draw_liquid_controls(ui);
                                self.draw_liquid_tts_controls(ui, &document);
                                self.liquid_footnote_index =
                                    build_liquid_footnote_index(&document.blocks);
                                self.draw_liquid_header(ui, &document);
                                if liquid_document_needs_ocr(&document) {
                                    self.draw_liquid_ocr_actions(ui, ctx);
                                } else if let Some(hint) = liquid_reflow_low_confidence(&document) {
                                    // #23: confidence-gated fallback affordance.
                                    self.draw_liquid_reflow_gate(ui, ctx, hint);
                                }
                                let outline = liquid_outline_items(&document.blocks);
                                self.draw_liquid_outline(ui, &outline);
                                let notes = liquid_note_blocks(&document.blocks);
                                let hidden_contents =
                                    hidden_contents_mask_for_display(&document.blocks);
                                let mut block_index = 0usize;
                                while block_index < document.blocks.len() {
                                    let block = &document.blocks[block_index];
                                    if hidden_contents.get(block_index).copied().unwrap_or(false)
                                        || should_hide_contents_block_for_display(block)
                                    {
                                        // #29: reveal hidden furniture (dimmed) when toggled on.
                                        if self.liquid_show_hidden_furniture {
                                            self.draw_liquid_hidden_furniture_block(ui, block);
                                        }
                                        block_index += 1;
                                        continue;
                                    }
                                    if block.role == LiquidBlockRole::Marginalia {
                                        let mut margin_notes = Vec::new();
                                        while block_index < document.blocks.len() {
                                            let note = &document.blocks[block_index];
                                            if hidden_contents
                                                .get(block_index)
                                                .copied()
                                                .unwrap_or(false)
                                                || should_hide_contents_block_for_display(note)
                                            {
                                                block_index += 1;
                                                continue;
                                            }
                                            if note.role != LiquidBlockRole::Marginalia {
                                                break;
                                            }
                                            margin_notes.push((block_index, note));
                                            block_index += 1;
                                        }
                                        self.draw_liquid_reader_row(
                                            ui,
                                            &document,
                                            None,
                                            None,
                                            &margin_notes,
                                            width,
                                            margin_width,
                                        );
                                        continue;
                                    }
                                    if block.role == LiquidBlockRole::Metadata {
                                        let start = block_index;
                                        while block_index < document.blocks.len()
                                            && document.blocks[block_index].role
                                                == LiquidBlockRole::Metadata
                                        {
                                            block_index += 1;
                                        }
                                        let visible_metadata = document.blocks[start..block_index]
                                            .iter()
                                            .enumerate()
                                            .filter(|(offset, block)| {
                                                !hidden_contents
                                                    .get(start + offset)
                                                    .copied()
                                                    .unwrap_or(false)
                                                    && !should_hide_contents_block_for_display(
                                                        block,
                                                    )
                                            })
                                            .map(|(_, block)| block.clone())
                                            .collect::<Vec<_>>();
                                        if !visible_metadata.is_empty() {
                                            self.draw_liquid_metadata_group(ui, &visible_metadata);
                                        }
                                        continue;
                                    }
                                    let mut margin_notes = Vec::new();
                                    let mut next_index = block_index + 1;
                                    while next_index < document.blocks.len() {
                                        let note = &document.blocks[next_index];
                                        if hidden_contents.get(next_index).copied().unwrap_or(false)
                                            || should_hide_contents_block_for_display(note)
                                        {
                                            next_index += 1;
                                            continue;
                                        }
                                        if note.role != LiquidBlockRole::Marginalia {
                                            break;
                                        }
                                        margin_notes.push((next_index, note));
                                        next_index += 1;
                                    }
                                    self.draw_liquid_reader_row(
                                        ui,
                                        &document,
                                        Some(block_index),
                                        Some(block),
                                        &margin_notes,
                                        width,
                                        margin_width,
                                    );
                                    block_index = next_index;
                                }
                                self.draw_liquid_notes(ui, &notes);
                                ui.add_space(40.0);
                            }
                        }
                    });
                });
            });
    }

    fn draw_liquid_header(&self, ui: &mut egui::Ui, document: &LiquidDocument) {
        ui.horizontal_wrapped(|ui| {
            let engine = document
                .llm_provider
                .as_deref()
                .unwrap_or_else(|| if document.llm_used { "LLM" } else { "Local" });
            ui.label(
                RichText::new(format!("Review Mode · {engine}"))
                    .strong()
                    .color(self.liquid_ink_color()),
            );
            ui.separator();
            ui.label(
                RichText::new(format!(
                    "{} noise line(s) removed",
                    document.noise_lines_removed
                ))
                .color(self.liquid_muted_color()),
            );
            if let Some(profile) = &document.profile {
                ui.separator();
                ui.label(
                    RichText::new(format!(
                        "{} · {:.0}%",
                        profile_display_name(profile.kind),
                        profile.confidence * 100.0
                    ))
                    .color(self.liquid_muted_color()),
                );
            }
            if let Some(integrity) = &document.footnote_link_integrity {
                ui.separator();
                let color = if integrity.ambiguous == 0 && integrity.landing_rate >= 0.95 {
                    self.liquid_muted_color()
                } else {
                    Color32::from_rgb(154, 91, 35)
                };
                ui.label(
                    RichText::new(format!(
                        "{} / {} note markers linked",
                        integrity.landed, integrity.detectable_markers
                    ))
                    .color(color),
                );
            }
        });
        for warning in &document.warnings {
            ui.label(RichText::new(warning).color(Color32::from_rgb(134, 92, 34)));
        }
        ui.add_space(12.0);
    }

    fn draw_liquid_feedback_block(
        &mut self,
        ui: &mut egui::Ui,
        document: &LiquidDocument,
        block_index: usize,
        block: &LiquidBlock,
    ) {
        let feedback_id = liquid_feedback_id(&document.source_signature, block_index, block);
        let has_feedback = self
            .liquid_feedback
            .iter()
            .any(|entry| entry.id == feedback_id && entry.submitted_at.is_none());
        let inner = egui::Frame::NONE.show(ui, |ui| self.draw_liquid_block(ui, block));
        let response = inner.response;
        // #27: footnote marker rects (in screen space) collected while drawing the body text.
        let marker_hits = inner.inner;
        // #30: right-click a block for a quick correction menu that feeds the label pipeline.
        let mut pending_correction: Option<(LiquidBlockRole, &'static str, &'static str)> = None;
        response.context_menu(|ui| {
            ui.label(
                RichText::new("Teach the reader")
                    .size(10.0)
                    .color(self.liquid_muted_color()),
            );
            if ui.button("This is a footnote").clicked() {
                pending_correction = Some((LiquidBlockRole::Marginalia, "footnote", "marginalia"));
                ui.close();
            }
            if ui.button("Keep as body text").clicked() {
                pending_correction = Some((LiquidBlockRole::Paragraph, "body", "keep"));
                ui.close();
            }
            if ui.button("Hide this (furniture)").clicked() {
                pending_correction = Some((LiquidBlockRole::Noise, "header_footer", "hide_noise"));
                ui.close();
            }
        });
        if let Some((role, gold_role, action)) = pending_correction {
            self.apply_reader_correction(document, block_index, block, role, gold_role, action);
        }
        // #31: if this heading is the pending outline scroll target, bring it into view.
        if let Some((target_level, target_text)) = self.liquid_scroll_to_heading.clone() {
            let level = match block.role {
                LiquidBlockRole::Heading => 1,
                LiquidBlockRole::Subheading => 2,
                _ => 0,
            };
            if level == target_level && compact_liquid_outline_text(&block.text) == target_text {
                response.scroll_to_me(Some(Align::TOP));
                self.liquid_scroll_to_heading = None;
            }
        }
        // Click a block to open/close its correction toolbar. (It used to appear on hover, which made
        // reading jumpy — touching any line popped the toolbar — and the buttons were effectively
        // unreachable, since moving the pointer toward them left the hover area and dismissed it.)
        // The block-wide overlay fully contains the inline markers, so egui's hit-test always
        // routes the click here (a fully-occluded smaller widget can't win). We therefore resolve
        // marker taps ourselves against `marker_hits` before falling back to the feedback toggle.
        let toggle = ui
            .interact(
                response.rect,
                ui.id().with(("liquid-fb-toggle", feedback_id.as_str())),
                Sense::click(),
            )
            .on_hover_cursor(CursorIcon::PointingHand);
        let mut marker_clicked = false;
        let mut provenance_clicked = false;
        if toggle.clicked() {
            // #29: ⌘/Ctrl-click a reflowed block jumps to the fixed-layout page view and
            // highlights the source bboxes this block was assembled from.
            if ui.input(|i| i.modifiers.command) {
                let rects = self.liquid_block_provenance_rects(document, block_index);
                if let Some((page, _)) = rects.first().copied() {
                    self.liquid_provenance_highlight = rects;
                    self.scroll_target_page = Some(page);
                    self.set_view_mode(DocumentViewMode::Pdf, ui.ctx());
                }
                provenance_clicked = true;
            } else if let Some(pos) = toggle.interact_pointer_pos() {
                if let Some((_, number)) = marker_hits.iter().find(|(rect, _)| rect.contains(pos)) {
                    let popup_id = liquid_footnote_popup_id(&feedback_id, *number);
                    egui::Popup::toggle_id(ui.ctx(), popup_id);
                    marker_clicked = true;
                }
            }
        }
        if toggle.clicked() && !marker_clicked && !provenance_clicked {
            if self.editing_liquid_feedback.as_deref() == Some(feedback_id.as_str()) {
                self.editing_liquid_feedback = None;
            } else {
                self.editing_liquid_feedback = Some(feedback_id.clone());
            }
        }
        // Render any open footnote popovers, anchored at their markers.
        for (rect, number) in &marker_hits {
            let popup_id = liquid_footnote_popup_id(&feedback_id, *number);
            if !egui::Popup::is_id_open(ui.ctx(), popup_id) {
                continue;
            }
            if let Some(body) = self.liquid_footnote_index.get(number) {
                self.draw_liquid_footnote_popover(ui, popup_id, *rect, *number, body);
            }
        }
        let editing = self.editing_liquid_feedback.as_deref() == Some(feedback_id.as_str());
        if has_feedback || editing {
            self.draw_liquid_feedback_toolbar(ui, document, block_index, block, &feedback_id);
        }
    }

    /// #29: source bboxes (paired with their page index) for a reflowed block, joined from the
    /// block's source-line refs to freshly extracted per-line geometry. Empty if the loaded
    /// document or its geometry is unavailable.
    fn liquid_block_provenance_rects(
        &self,
        document: &LiquidDocument,
        block_index: usize,
    ) -> Vec<(usize, PdfRect)> {
        let Some(loaded) = self.document.as_ref() else {
            return Vec::new();
        };
        let refs = liquid_block_source_lines(document, block_index);
        if refs.is_empty() {
            return Vec::new();
        }
        let deep =
            crate::layout_roles::deep_source_lines_for_pages(&loaded.pages, &loaded.text_chars);
        let mut rects = Vec::new();
        for source in &refs {
            if let Some(line) = deep.iter().find(|line| {
                line.page_index == source.page_index && line.line_index == source.line_index
            }) {
                rects.push((
                    line.page_index,
                    PdfRect::new(line.left, line.bottom, line.right, line.top),
                ));
            }
        }
        rects
    }

    /// #29: render a normally-hidden "furniture" block (header/footer/TOC/noise/table) as a
    /// dimmed, role-tagged line, so the reader can see what the reflow dropped.
    fn draw_liquid_hidden_furniture_block(&self, ui: &mut egui::Ui, block: &LiquidBlock) {
        let text = block.text.trim();
        if text.is_empty() {
            return;
        }
        let scale = self.liquid_text_scale;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 5.0;
            ui.add(egui::Label::new(
                RichText::new(format!("[{}]", block.role.prompt_name()))
                    .size(10.0 * scale)
                    .monospace()
                    .color(self.liquid_footnote_marker_color()),
            ));
            ui.add(
                egui::Label::new(
                    RichText::new(text)
                        .size(12.0 * scale)
                        .italics()
                        .color(self.liquid_muted_color()),
                )
                .wrap()
                .selectable(true),
            );
        });
        ui.add_space(4.0);
    }

    /// #29: paint the provenance source-line highlights on `page_index` in the fixed-layout view.
    fn draw_liquid_provenance_overlay(
        &self,
        painter: &egui::Painter,
        placement: &PagePlacement,
        page_index: usize,
    ) {
        for (_, pdf_rect) in self
            .liquid_provenance_highlight
            .iter()
            .filter(|(page, _)| *page == page_index)
        {
            let rect = placement.pdf_rect_to_screen(*pdf_rect);
            painter.rect_filled(rect, 2.0, Color32::from_rgba_unmultiplied(56, 132, 200, 56));
            painter.rect_stroke(
                rect,
                2.0,
                Stroke::new(1.5, Color32::from_rgba_unmultiplied(38, 92, 158, 190)),
                egui::StrokeKind::Inside,
            );
        }
    }

    fn draw_liquid_reader_row(
        &mut self,
        ui: &mut egui::Ui,
        document: &LiquidDocument,
        block_index: Option<usize>,
        block: Option<&LiquidBlock>,
        margin_notes: &[(usize, &LiquidBlock)],
        body_width: f32,
        margin_width: f32,
    ) {
        if margin_width <= 0.0 {
            if let (Some(block_index), Some(block)) = (block_index, block) {
                self.draw_liquid_feedback_block(ui, document, block_index, block);
            }
            return;
        }

        let mut left_notes = Vec::new();
        let mut right_notes = Vec::new();
        for (note_index, note) in margin_notes {
            if liquid_margin_note_goes_left(*note_index, note) {
                left_notes.push((*note_index, *note));
            } else {
                right_notes.push((*note_index, *note));
            }
        }

        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                Vec2::new(margin_width, 0.0),
                egui::Layout::top_down(Align::RIGHT),
                |ui| {
                    self.draw_liquid_margin_note_stack(ui, &left_notes, margin_width, true);
                },
            );
            ui.add_space(LIQUID_MARGIN_NOTE_GAP);
            ui.allocate_ui_with_layout(
                Vec2::new(body_width, 0.0),
                egui::Layout::top_down(Align::LEFT),
                |ui| {
                    ui.set_width(body_width);
                    if let (Some(block_index), Some(block)) = (block_index, block) {
                        self.draw_liquid_feedback_block(ui, document, block_index, block);
                    } else {
                        ui.add_space(2.0);
                    }
                },
            );
            ui.add_space(LIQUID_MARGIN_NOTE_GAP);
            ui.allocate_ui_with_layout(
                Vec2::new(margin_width, 0.0),
                egui::Layout::top_down(Align::LEFT),
                |ui| {
                    self.draw_liquid_margin_note_stack(ui, &right_notes, margin_width, false);
                },
            );
        });
    }

    fn draw_liquid_margin_note_stack(
        &self,
        ui: &mut egui::Ui,
        notes: &[(usize, &LiquidBlock)],
        width: f32,
        left_side: bool,
    ) {
        for (position, (_, note)) in notes.iter().enumerate() {
            if position > 0 {
                ui.add_space(7.0);
            }
            self.draw_liquid_margin_note_card(ui, note, width, left_side);
        }
    }

    fn draw_liquid_margin_note_card(
        &self,
        ui: &mut egui::Ui,
        note: &LiquidBlock,
        width: f32,
        left_side: bool,
    ) {
        let (marker, body) = split_liquid_note_marker(&note.text);
        let label = marker.unwrap_or("cont.");
        let body = compact_liquid_margin_note_text(callout_body_text(
            note.label.as_deref().unwrap_or("Footnote"),
            body,
        ));
        let (fill, stroke, label_color, body_color) = match self.liquid_theme {
            LiquidTheme::Paper => (
                Color32::from_rgb(252, 249, 242),
                Color32::from_rgb(213, 202, 181),
                Color32::from_rgb(118, 86, 48),
                Color32::from_rgb(82, 73, 61),
            ),
            LiquidTheme::Sepia => (
                Color32::from_rgb(245, 235, 214),
                Color32::from_rgb(201, 179, 139),
                Color32::from_rgb(112, 76, 39),
                Color32::from_rgb(78, 59, 38),
            ),
            LiquidTheme::Dark => (
                Color32::from_rgb(45, 43, 39),
                Color32::from_rgb(96, 88, 73),
                Color32::from_rgb(220, 190, 137),
                Color32::from_rgb(202, 197, 187),
            ),
        };
        let align = if left_side { Align::RIGHT } else { Align::LEFT };
        ui.allocate_ui_with_layout(Vec2::new(width, 0.0), egui::Layout::top_down(align), |ui| {
            egui::Frame::NONE
                .fill(fill)
                .stroke(Stroke::new(1.0, stroke))
                .corner_radius(4)
                .inner_margin(Margin::symmetric(8, 7))
                .show(ui, |ui| {
                    ui.set_width((width - 18.0).max(72.0));
                    ui.label(
                        RichText::new(label)
                            .size(10.5 * self.liquid_text_scale)
                            .strong()
                            .color(label_color),
                    );
                    ui.add_space(2.0);
                    ui.add(
                        egui::Label::new(
                            RichText::new(body)
                                .size(11.5 * self.liquid_text_scale)
                                .color(body_color),
                        )
                        .wrap(),
                    );
                });
        });
    }

    fn draw_liquid_feedback_toolbar(
        &mut self,
        ui: &mut egui::Ui,
        document: &LiquidDocument,
        block_index: usize,
        block: &LiquidBlock,
        feedback_id: &str,
    ) {
        let mut save_after_edit = false;
        ui.horizontal_wrapped(|ui| {
            ui.add_space(18.0);
            ui.label(
                RichText::new("Mark as")
                    .size(10.0)
                    .color(self.liquid_muted_color()),
            );
            for (label, role) in [
                ("Body", LiquidBlockRole::Paragraph),
                ("Footnote", LiquidBlockRole::Footnote),
                ("Title", LiquidBlockRole::Title),
                ("Heading", LiquidBlockRole::Heading),
                ("Noise", LiquidBlockRole::Noise),
            ] {
                if ui
                    .small_button(label)
                    .on_hover_text(format!("Record this block as {label} training feedback"))
                    .clicked()
                {
                    self.upsert_liquid_feedback(document, block_index, block, Some(role));
                    self.editing_liquid_feedback = Some(feedback_id.to_owned());
                }
            }
            if ui
                .small_button("Note")
                .on_hover_text("Add a freeform Review Mode training note")
                .clicked()
            {
                self.upsert_liquid_feedback(document, block_index, block, None);
                self.editing_liquid_feedback = Some(feedback_id.to_owned());
            }
        });

        if let Some(index) = self
            .liquid_feedback
            .iter()
            .position(|entry| entry.id == feedback_id)
            && self.editing_liquid_feedback.as_deref() == Some(feedback_id)
        {
            ui.horizontal(|ui| {
                ui.add_space(18.0);
                ui.label(
                    RichText::new("Annotation")
                        .size(10.0)
                        .color(self.liquid_muted_color()),
                );
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.liquid_feedback[index].note)
                        .desired_width(360.0)
                        .hint_text("what looked wrong?"),
                );
                if response.changed() {
                    self.liquid_feedback[index].updated_at = comment_timestamp();
                    save_after_edit = true;
                }
                if ui.small_button("Done").clicked() {
                    self.editing_liquid_feedback = None;
                    save_after_edit = true;
                }
            });
        }
        if save_after_edit {
            self.save_current_liquid_feedback();
        }
    }

    /// #33: read the reflowed text aloud with native macOS speech (`say`), reading order
    /// preserved, page furniture skipped, footnotes as a separate pass. Replaces any current
    /// playback. No-op with a note on non-macOS.
    fn start_liquid_tts(&mut self, document: &LiquidDocument) {
        self.stop_liquid_tts();
        let text = liquid_tts_text(document, self.tts_controller.include_notes);
        if text.is_empty() {
            self.status = "Nothing to read aloud.".to_owned();
            return;
        }
        #[cfg(target_os = "macos")]
        {
            let tmp = std::env::temp_dir().join("lawpdf-tts.txt");
            if let Err(error) = std::fs::write(&tmp, &text) {
                self.push_error_notice(format!("Could not prepare speech: {error}"));
                return;
            }
            match std::process::Command::new("say")
                .arg("-f")
                .arg(&tmp)
                .spawn()
            {
                Ok(child) => {
                    self.tts_controller.child = Some(child);
                    self.status = "Reading aloud…".to_owned();
                }
                Err(error) => {
                    self.push_error_notice(format!("Text-to-speech unavailable: {error}"));
                }
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = text;
            self.status = "Read aloud is available on macOS.".to_owned();
        }
    }

    /// #33: stop any in-progress speech playback.
    fn stop_liquid_tts(&mut self) {
        if let Some(mut child) = self.tts_controller.child.take() {
            // Best-effort process cleanup: playback is already detached from app state.
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    fn start_paid_liquid_tts(&mut self, document: &LiquidDocument) {
        if self.tts_controller.progress.is_some() {
            return;
        }
        let api_key = match self.tts_controller.provider {
            PaidTtsProvider::OpenRouter => effective_openrouter_api_key(&self.settings),
            PaidTtsProvider::OpenAi => effective_openai_api_key(&self.settings),
        };
        let Some(api_key) = api_key else {
            self.settings_ui.open = true;
            self.status = format!(
                "Add your {} API key to create an MP3.",
                self.tts_controller.provider.label()
            );
            return;
        };
        let text = liquid_tts_text(document, self.tts_controller.include_notes);
        if text.is_empty() {
            self.status = "Nothing to narrate.".to_owned();
            return;
        }
        let file_name = self
            .document
            .as_ref()
            .map(|source| default_output_name(&source.path, "review-mode", "mp3"))
            .unwrap_or_else(|| "review-mode.mp3".to_owned());
        let Some(destination) =
            self.pick_save_path("Save narration", &file_name, "MP3 audio", &["mp3"])
        else {
            return;
        };
        spawn_paid_tts_job(
            PaidTtsRequest {
                provider: self.tts_controller.provider,
                api_key,
                voice: "nova".to_owned(),
                text,
                destination,
            },
            self.tts_controller.tx.clone(),
        );
        self.tts_controller.progress = Some((0, 0));
        self.status = format!(
            "Creating MP3 with {}…",
            self.tts_controller.provider.label()
        );
    }

    fn poll_paid_tts(&mut self, ctx: &Context) {
        while let Ok(event) = self.tts_controller.rx.try_recv() {
            match event {
                PaidTtsEvent::Progress { completed, total } => {
                    self.tts_controller.progress = Some((completed, total));
                    self.status = format!("Creating MP3… {completed}/{total}");
                }
                PaidTtsEvent::Complete(path) => {
                    self.tts_controller.progress = None;
                    self.status = format!("Created {}", path.display());
                    self.push_info_notice(format!("Created {}", path.display()));
                }
                PaidTtsEvent::Failed(error) => {
                    self.tts_controller.progress = None;
                    self.push_error_notice(error);
                }
            }
            ctx.request_repaint();
        }
    }

    /// Reader actions: clean text download is the default; narration is optional.
    fn draw_liquid_tts_controls(&mut self, ui: &mut egui::Ui, document: &LiquidDocument) {
        // Clear the handle if playback finished on its own.
        let finished = self
            .tts_controller
            .child
            .as_mut()
            .map(|child| matches!(child.try_wait(), Ok(Some(_))))
            .unwrap_or(false);
        if finished {
            self.tts_controller.child = None;
        }
        ui.horizontal_wrapped(|ui| {
            if ui
                .button(RichText::new("Download text").size(11.0).strong())
                .on_hover_text("Save the clean, page-free Review Mode text as a .txt file")
                .clicked()
            {
                self.export_review_text_dialog(document);
            }
            if self.tts_controller.child.is_some() {
                if ui
                    .button(RichText::new("⏹ Stop reading").size(11.0))
                    .clicked()
                {
                    self.stop_liquid_tts();
                }
            } else if ui
                .button(RichText::new("▶ Read aloud").size(11.0))
                .on_hover_text("Read the reflowed text aloud (macOS speech)")
                .clicked()
            {
                self.start_liquid_tts(document);
            }
            ui.separator();
            egui::ComboBox::from_id_salt("paid_tts_provider")
                .selected_text(self.tts_controller.provider.label())
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.tts_controller.provider,
                        PaidTtsProvider::OpenRouter,
                        "OpenRouter",
                    );
                    ui.selectable_value(
                        &mut self.tts_controller.provider,
                        PaidTtsProvider::OpenAi,
                        "OpenAI",
                    );
                });
            let paid_label = self
                .tts_controller
                .progress
                .map(|(completed, total)| {
                    if total == 0 {
                        "Creating MP3…".to_owned()
                    } else {
                        format!("Creating MP3… {completed}/{total}")
                    }
                })
                .unwrap_or_else(|| "Create AI MP3".to_owned());
            if ui
                .add_enabled(
                    self.tts_controller.progress.is_none(),
                    egui::Button::new(RichText::new(paid_label).size(11.0)),
                )
                .on_hover_text(
                    "Creates AI-generated narration. Sends the article text to the selected provider using your key; provider charges may apply.",
                )
                .clicked()
            {
                self.start_paid_liquid_tts(document);
            }
            if ui
                .small_button("TTS keys")
                .on_hover_text("Add or change OpenAI and OpenRouter API keys")
                .clicked()
            {
                self.settings_ui.open = true;
            }
            let mut include = self.tts_controller.include_notes;
            if ui
                .checkbox(&mut include, RichText::new("footnotes").size(11.0))
                .on_hover_text("Read footnotes as a separate pass after the body")
                .changed()
            {
                self.tts_controller.include_notes = include;
            }
        });
        ui.add_space(6.0);
    }

    /// #32: while the (non-blocking, background) reflow finishes, give the reader instant
    /// one-click access to the original layout so opening a document never strands them on a
    /// blank spinner. NOTE: true per-page streaming reflow (first page paints, rest streams in)
    /// requires the LM2 job to emit partial documents — a bounded pipeline/event change in
    /// liquid2.rs, outside this src/app.rs UI lane; flagged to the coordinator.
    fn draw_liquid_loading_fallback(&mut self, ui: &mut egui::Ui, ctx: &Context) {
        ui.add_space(14.0);
        if ui
            .button("Read the original layout while this loads")
            .clicked()
        {
            self.set_view_mode(DocumentViewMode::Pdf, ctx);
        }
        ui.add_space(4.0);
        ui.label(
            RichText::new("The reflow keeps processing in the background — switch back to Review Mode anytime.")
                .size(11.0)
                .color(self.liquid_muted_color()),
        );
    }

    /// #23: warn when a document is low-confidence to reflow, and offer the fixed layout —
    /// a wrong reflow costs more trust than no reflow. Non-blocking: the reflow still renders
    /// below, but the reader is told and given a one-click escape.
    fn draw_liquid_reflow_gate(&mut self, ui: &mut egui::Ui, ctx: &Context, hint: &str) {
        let scale = self.liquid_text_scale;
        let (fill, stroke, ink) = match self.liquid_theme {
            LiquidTheme::Dark => (
                Color32::from_rgb(58, 48, 30),
                Color32::from_rgb(140, 112, 58),
                Color32::from_rgb(232, 205, 150),
            ),
            _ => (
                Color32::from_rgb(252, 244, 222),
                Color32::from_rgb(219, 190, 122),
                Color32::from_rgb(122, 88, 28),
            ),
        };
        let muted = self.liquid_muted_color();
        let mut go_fixed = false;
        egui::Frame::NONE
            .fill(fill)
            .stroke(Stroke::new(1.0, stroke))
            .corner_radius(6)
            .inner_margin(Margin::symmetric(12, 10))
            .show(ui, |ui| {
                ui.label(
                    RichText::new("This document may not reflow cleanly")
                        .size(13.0 * scale)
                        .strong()
                        .color(ink),
                );
                ui.add_space(3.0);
                ui.label(RichText::new(hint).size(11.5 * scale).color(muted));
                ui.add_space(7.0);
                if ui
                    .button(RichText::new("View fixed layout").size(11.5 * scale))
                    .on_hover_text("Open the original page layout instead")
                    .clicked()
                {
                    go_fixed = true;
                }
            });
        ui.add_space(10.0);
        if go_fixed {
            self.set_view_mode(DocumentViewMode::Pdf, ctx);
        }
    }

    fn draw_liquid_ocr_actions(&mut self, ui: &mut egui::Ui, ctx: &Context) {
        ui.horizontal_wrapped(|ui| {
            let ocr_active = self.ocr_is_active();
            if ui
                .add_enabled(!ocr_active, egui::Button::new("Run OCR"))
                .clicked()
            {
                self.start_ocr();
                ctx.request_repaint_after(RENDER_POLL_INTERVAL);
            }
            if ui
                .add_enabled(!ocr_active, egui::Button::new("OCR PDF"))
                .on_hover_text("Use OpenRouter OCR and save a searchable PDF copy")
                .clicked()
            {
                self.start_openrouter_ocr_save();
                ctx.request_repaint_after(RENDER_POLL_INTERVAL);
            }
            if has_usable_ocr_text(&self.ocr_states) && ui.button("Rebuild Liquid").clicked() {
                self.liquid_state = LiquidState::Idle;
                self.liquid_notice_dismissed = false;
                self.ensure_liquid_started(ctx);
            }
            if ocr_active {
                ui.spinner();
                ui.label(RichText::new(self.ocr_summary()).color(self.liquid_muted_color()));
            }
        });
        ui.add_space(12.0);
    }

    fn draw_liquid_outline(&mut self, ui: &mut egui::Ui, outline: &[LiquidOutlineItem]) {
        if outline.is_empty() {
            return;
        }

        let ink = self.liquid_ink_color();
        let muted = self.liquid_muted_color();
        let scale = self.liquid_text_scale;
        let mut clicked_target: Option<(usize, String)> = None;
        egui::CollapsingHeader::new(format!("Sections ({})", outline.len()))
            .default_open(outline.len() <= 6)
            .show(ui, |ui| {
                ui.add_space(2.0);
                for item in outline {
                    ui.horizontal_wrapped(|ui| {
                        ui.add_space(match item.level {
                            0 | 1 => 0.0,
                            _ => 18.0,
                        });
                        // #31: clicking an entry scrolls the reading view to its heading.
                        let response = ui
                            .add(
                                egui::Label::new(
                                    RichText::new(&item.text)
                                        .size(if item.level <= 1 {
                                            14.0 * scale
                                        } else {
                                            13.0 * scale
                                        })
                                        .strong()
                                        .color(if item.level <= 1 { ink } else { muted }),
                                )
                                .sense(Sense::click()),
                            )
                            .on_hover_cursor(CursorIcon::PointingHand);
                        if response.clicked() {
                            clicked_target = Some((item.level, item.text.clone()));
                        }
                    });
                }
            });
        if let Some(target) = clicked_target {
            self.liquid_scroll_to_heading = Some(target);
        }
        ui.add_space(10.0);
    }

    /// Draws a body paragraph. Inline footnote markers (wrapped in CALLOUT sentinels
    /// during extraction) are rendered as raised superscripts; those whose number
    /// resolves against `liquid_footnote_index` are drawn as tappable links and their
    /// screen rects are returned so the caller can anchor a footnote popover.
    fn draw_liquid_paragraph(
        &self,
        ui: &mut egui::Ui,
        text: &str,
        size: f32,
        color: Color32,
    ) -> Vec<(Rect, u16)> {
        let has_callout = text.contains(crate::layout_roles::CALLOUT_START);
        if !has_callout {
            ui.add(
                egui::Label::new(RichText::new(text).size(size).color(color))
                    .wrap()
                    .selectable(true),
            );
            return Vec::new();
        }

        // Split into (is_callout, run) segments at the sentinels, then flow them inline
        // via horizontal_wrapped so each marker owns a rect while body text still wraps.
        let mut segments: Vec<(bool, String)> = Vec::new();
        let mut in_callout = false;
        let mut buf = String::new();
        for ch in text.chars() {
            match ch {
                crate::layout_roles::CALLOUT_START => {
                    if !buf.is_empty() {
                        segments.push((false, std::mem::take(&mut buf)));
                    }
                    in_callout = true;
                }
                crate::layout_roles::CALLOUT_END => {
                    if !buf.is_empty() {
                        segments.push((true, std::mem::take(&mut buf)));
                    }
                    in_callout = false;
                }
                _ => buf.push(ch),
            }
        }
        if !buf.is_empty() {
            segments.push((in_callout, buf));
        }

        let muted = self.liquid_muted_color();
        let marker_color = self.liquid_footnote_marker_color();
        let mut marker_hits = Vec::new();
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            for (is_callout, run) in &segments {
                if !is_callout {
                    ui.add(
                        egui::Label::new(RichText::new(run).size(size).color(color))
                            .wrap()
                            .selectable(true),
                    );
                    continue;
                }
                let number = run
                    .trim()
                    .parse::<u16>()
                    .ok()
                    .filter(|n| self.liquid_footnote_index.contains_key(n));
                if let Some(number) = number {
                    let response = ui
                        .add(
                            egui::Label::new(
                                RichText::new(run)
                                    .size(size * 0.66)
                                    .color(marker_color)
                                    .raised(),
                            )
                            .sense(Sense::click()),
                        )
                        .on_hover_cursor(CursorIcon::PointingHand);
                    marker_hits.push((response.rect, number));
                } else {
                    ui.add(egui::Label::new(
                        RichText::new(run).size(size * 0.66).color(muted).raised(),
                    ));
                }
            }
        });
        marker_hits
    }

    /// Theme-aware accent for tappable footnote markers in the Liquid view.
    fn liquid_footnote_marker_color(&self) -> Color32 {
        match self.liquid_theme {
            LiquidTheme::Paper => Color32::from_rgb(38, 92, 158),
            LiquidTheme::Sepia => Color32::from_rgb(120, 74, 38),
            LiquidTheme::Dark => Color32::from_rgb(126, 176, 222),
        }
    }

    /// Renders an open footnote popover anchored at a body marker's rect. Open state
    /// lives in egui memory (keyed by `popup_id`); it closes on a click outside.
    fn draw_liquid_footnote_popover(
        &self,
        ui: &egui::Ui,
        popup_id: egui::Id,
        anchor: Rect,
        number: u16,
        body: &str,
    ) {
        let (fill, stroke, label_color, body_color) = match self.liquid_theme {
            LiquidTheme::Paper => (
                Color32::from_rgb(252, 249, 242),
                Color32::from_rgb(213, 202, 181),
                self.liquid_footnote_marker_color(),
                Color32::from_rgb(60, 55, 48),
            ),
            LiquidTheme::Sepia => (
                Color32::from_rgb(245, 235, 214),
                Color32::from_rgb(201, 179, 139),
                self.liquid_footnote_marker_color(),
                Color32::from_rgb(70, 54, 36),
            ),
            LiquidTheme::Dark => (
                Color32::from_rgb(45, 43, 39),
                Color32::from_rgb(96, 88, 73),
                self.liquid_footnote_marker_color(),
                Color32::from_rgb(210, 205, 196),
            ),
        };
        let scale = self.liquid_text_scale;
        egui::Popup::new(popup_id, ui.ctx().clone(), anchor, ui.layer_id())
            .open_memory(None)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .gap(4.0)
            .width(320.0)
            .frame(
                egui::Frame::popup(ui.style())
                    .fill(fill)
                    .stroke(Stroke::new(1.0, stroke)),
            )
            .show(|ui| {
                ui.set_max_width(320.0);
                ui.label(
                    RichText::new(format!("Footnote {number}"))
                        .size(11.0 * scale)
                        .strong()
                        .color(label_color),
                );
                ui.add_space(3.0);
                ui.add(
                    egui::Label::new(RichText::new(body).size(13.0 * scale).color(body_color))
                        .wrap(),
                );
            });
    }

    /// Returns footnote-marker hit rects (screen space) so the caller can drive
    /// tap-to-view popovers; empty for blocks with no clickable markers.
    fn draw_liquid_block(&self, ui: &mut egui::Ui, block: &LiquidBlock) -> Vec<(Rect, u16)> {
        let s = self.liquid_text_scale; // scale factor for all body text in Liquid view
        let ink = self.liquid_ink_color();
        let muted = self.liquid_muted_color();
        let mut marker_hits = Vec::new();
        match block.role {
            LiquidBlockRole::Title => {
                ui.add_space(8.0);
                ui.add(
                    egui::Label::new(
                        RichText::new(&block.text)
                            .size(30.0 * s)
                            .strong()
                            .color(ink),
                    )
                    .wrap(),
                );
                ui.add_space(12.0);
            }
            LiquidBlockRole::Heading => {
                ui.add_space(18.0);
                ui.add(
                    egui::Label::new(
                        RichText::new(&block.text)
                            .size(24.0 * s)
                            .strong()
                            .color(ink),
                    )
                    .wrap(),
                );
                ui.add_space(4.0);
            }
            LiquidBlockRole::Subheading => {
                ui.add_space(12.0);
                ui.add(
                    egui::Label::new(
                        RichText::new(&block.text)
                            .size(19.0 * s)
                            .strong()
                            .color(ink),
                    )
                    .wrap(),
                );
                ui.add_space(2.0);
            }
            LiquidBlockRole::Definition => {
                self.draw_liquid_callout_block(
                    ui,
                    block.label.as_deref().unwrap_or("Definition"),
                    &block.text,
                    Color32::from_rgb(116, 77, 30),
                    Color32::from_rgb(255, 248, 232),
                    Color32::from_rgb(224, 183, 102),
                );
            }
            LiquidBlockRole::KeyClause => {
                self.draw_liquid_callout_block(
                    ui,
                    block.label.as_deref().unwrap_or("Key clause"),
                    &block.text,
                    Color32::from_rgb(53, 95, 58),
                    Color32::from_rgb(239, 249, 239),
                    Color32::from_rgb(148, 194, 151),
                );
            }
            LiquidBlockRole::Marginalia => {
                self.draw_liquid_marginalia_block(
                    ui,
                    block.label.as_deref().unwrap_or("Note"),
                    &block.text,
                );
            }
            LiquidBlockRole::Explainer => {
                self.draw_liquid_callout_block(
                    ui,
                    block.label.as_deref().unwrap_or("Explainer"),
                    &block.text,
                    Color32::from_rgb(42, 84, 131),
                    Color32::from_rgb(238, 246, 255),
                    Color32::from_rgb(148, 188, 229),
                );
            }
            LiquidBlockRole::Takeaway => {
                self.draw_liquid_callout_block(
                    ui,
                    block.label.as_deref().unwrap_or("Takeaway"),
                    &block.text,
                    Color32::from_rgb(45, 102, 81),
                    Color32::from_rgb(236, 249, 244),
                    Color32::from_rgb(127, 193, 168),
                );
            }
            LiquidBlockRole::Holding => {
                self.draw_liquid_callout_block(
                    ui,
                    block.label.as_deref().unwrap_or("Holding"),
                    &block.text,
                    Color32::from_rgb(111, 76, 37),
                    Color32::from_rgb(255, 244, 224),
                    Color32::from_rgb(222, 172, 98),
                );
            }
            LiquidBlockRole::Issue => {
                self.draw_liquid_callout_block(
                    ui,
                    block.label.as_deref().unwrap_or("Issue"),
                    &block.text,
                    Color32::from_rgb(122, 55, 72),
                    Color32::from_rgb(254, 240, 244),
                    Color32::from_rgb(222, 148, 166),
                );
            }
            LiquidBlockRole::Clause => {
                self.draw_liquid_list_item(ui, &block.text);
            }
            LiquidBlockRole::ListItem => {
                self.draw_liquid_list_item(ui, &block.text);
            }
            LiquidBlockRole::Quote => {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    ui.separator();
                    ui.add_space(10.0);
                    ui.add(
                        egui::Label::new(
                            RichText::new(&block.text)
                                .size(17.0 * s)
                                .italics()
                                .color(Color32::from_rgb(69, 63, 55)),
                        )
                        .wrap(),
                    );
                });
                ui.add_space(6.0);
            }
            LiquidBlockRole::Caption => {
                self.draw_liquid_caption_block(
                    ui,
                    block.label.as_deref().unwrap_or("Caption"),
                    &block.text,
                );
            }
            LiquidBlockRole::Table => {
                self.draw_liquid_caption_block(ui, "Table", &block.text);
            }
            LiquidBlockRole::Paragraph => {
                marker_hits = self.draw_liquid_paragraph(ui, &block.text, 17.0 * s, ink);
                ui.add_space(8.0);
            }
            LiquidBlockRole::Lead => {
                ui.add_space(2.0);
                ui.add(
                    egui::Label::new(
                        RichText::new(&block.text)
                            .size(20.0 * s)
                            .color(Color32::from_rgb(42, 44, 45)),
                    )
                    .wrap()
                    .selectable(true),
                );
                ui.add_space(14.0);
            }
            LiquidBlockRole::Abstract => {
                self.draw_liquid_callout_block(
                    ui,
                    "Abstract",
                    &block.text,
                    Color32::from_rgb(50, 80, 130),
                    Color32::from_rgb(240, 244, 250),
                    Color32::from_rgb(176, 196, 222),
                );
            }
            LiquidBlockRole::Syllabus => {
                self.draw_liquid_callout_block(
                    ui,
                    "Syllabus",
                    &block.text,
                    Color32::from_rgb(50, 80, 130),
                    Color32::from_rgb(240, 244, 250),
                    Color32::from_rgb(176, 196, 222),
                );
            }
            LiquidBlockRole::AuthorInfo => {
                ui.add_space(4.0);
                ui.add(
                    egui::Label::new(
                        RichText::new(&block.text)
                            .size(15.0 * s)
                            .italics()
                            .color(muted),
                    )
                    .wrap(),
                );
                ui.add_space(4.0);
            }
            LiquidBlockRole::Metadata => {
                self.draw_liquid_metadata_group(ui, std::slice::from_ref(block));
            }
            LiquidBlockRole::Header
            | LiquidBlockRole::Footer
            | LiquidBlockRole::Footnote
            | LiquidBlockRole::Contents
            | LiquidBlockRole::Noise => {}
            LiquidBlockRole::SectionBreak => {
                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    ui.add_space((ui.available_width() - 120.0).max(0.0) / 2.0);
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(120.0, 1.0), egui::Sense::hover());
                    ui.painter()
                        .hline(rect.x_range(), rect.center().y, Stroke::new(1.0, muted));
                });
                ui.add_space(16.0);
            }
        }
        marker_hits
    }

    fn draw_liquid_caption_block(&self, ui: &mut egui::Ui, label: &str, text: &str) {
        let s = self.liquid_text_scale;
        ui.add_space(2.0);
        ui.horizontal_top(|ui| {
            ui.add_space(12.0);
            ui.separator();
            ui.add_space(8.0);
            ui.vertical(|ui| {
                ui.label(
                    RichText::new(label)
                        .size(11.0 * s)
                        .strong()
                        .color(Color32::from_rgb(100, 93, 83)),
                );
                ui.add_space(1.0);
                ui.add(
                    egui::Label::new(
                        RichText::new(text)
                            .size(14.0 * s)
                            .italics()
                            .color(self.liquid_muted_color()),
                    )
                    .wrap()
                    .selectable(true),
                );
            });
        });
        ui.add_space(8.0);
    }

    fn draw_liquid_marginalia_block(&self, ui: &mut egui::Ui, label: &str, text: &str) {
        let body = callout_body_text(label, text);
        ui.add_space(2.0);
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                Vec2::new(128.0, 18.0),
                egui::Layout::top_down(Align::RIGHT),
                |ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new(label)
                                .size(12.0 * self.liquid_text_scale)
                                .strong()
                                .color(Color32::from_rgb(94, 86, 75)),
                        )
                        .wrap(),
                    );
                },
            );
            ui.add_space(12.0);
            ui.add(
                egui::Label::new(
                    RichText::new(body)
                        .size(16.0 * self.liquid_text_scale)
                        .color(self.liquid_ink_color()),
                )
                .wrap()
                .selectable(true),
            );
        });
        ui.add_space(6.0);
    }

    fn draw_liquid_list_item(&self, ui: &mut egui::Ui, text: &str) {
        let item = liquid_list_item_parts(text);
        ui.horizontal_top(|ui| {
            ui.add_space((item.indent_level as f32) * 22.0);
            ui.allocate_ui_with_layout(
                Vec2::new(42.0, 18.0),
                egui::Layout::top_down(Align::RIGHT),
                |ui| {
                    ui.label(
                        RichText::new(item.marker)
                            .size(16.0)
                            .strong()
                            .color(MUTED_INK),
                    );
                },
            );
            ui.add_space(8.0);
            ui.add(
                egui::Label::new(
                    RichText::new(item.body)
                        .size(17.0 * self.liquid_text_scale)
                        .color(self.liquid_ink_color()),
                )
                .wrap()
                .selectable(true),
            );
        });
        ui.add_space(5.0);
    }

    fn draw_liquid_callout_block(
        &self,
        ui: &mut egui::Ui,
        label: &str,
        text: &str,
        label_color: Color32,
        fill: Color32,
        stroke_color: Color32,
    ) {
        let body_text = callout_body_text(label, text);
        egui::Frame::NONE
            .fill(fill)
            .stroke(Stroke::new(1.0, stroke_color))
            .corner_radius(6)
            .inner_margin(Margin::symmetric(14, 10))
            .show(ui, |ui| {
                ui.label(
                    RichText::new(label)
                        .size(12.0 * self.liquid_text_scale)
                        .strong()
                        .color(label_color),
                );
                ui.add_space(3.0);
                ui.add(
                    egui::Label::new(
                        RichText::new(body_text)
                            .size(17.0 * self.liquid_text_scale)
                            .color(self.liquid_ink_color()),
                    )
                    .wrap()
                    .selectable(true),
                );
            });
        ui.add_space(10.0);
    }

    fn draw_liquid_metadata_group(&self, ui: &mut egui::Ui, metadata: &[LiquidBlock]) {
        let parts = compact_liquid_metadata_parts(metadata);
        if parts.is_empty() {
            return;
        }

        ui.add_space(2.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new("Context")
                    .size(12.0 * self.liquid_text_scale)
                    .strong()
                    .color(self.liquid_muted_color()),
            );
            for (index, part) in parts.iter().enumerate() {
                if index > 0 {
                    ui.label(
                        RichText::new("·")
                            .size(13.0 * self.liquid_text_scale)
                            .color(self.liquid_muted_color()),
                    );
                }
                ui.label(
                    RichText::new(part)
                        .size(14.0 * self.liquid_text_scale)
                        .color(self.liquid_muted_color()),
                );
            }
        });
        ui.add_space(7.0);
    }

    fn draw_liquid_notes(&self, ui: &mut egui::Ui, notes: &[&LiquidBlock]) {
        if notes.is_empty() {
            return;
        }

        ui.add_space(22.0);
        ui.separator();
        ui.add_space(6.0);
        egui::CollapsingHeader::new(format!("Notes ({})", notes.len()))
            .default_open(false)
            .show(ui, |ui| {
                ui.add_space(4.0);
                for (index, note) in notes.iter().enumerate() {
                    let is_footnote = note.role == LiquidBlockRole::Footnote;
                    let (marker, body) = if is_footnote {
                        split_liquid_note_marker(&note.text)
                    } else {
                        (None, note.text.as_str())
                    };
                    let marker = if is_footnote {
                        marker
                            .map(str::to_owned)
                            .unwrap_or_else(|| (index + 1).to_string())
                    } else {
                        // Show role for mis-classified blocks now surfaced in Notes
                        format!("{:?}", note.role)
                    };
                    ui.horizontal_top(|ui| {
                        ui.allocate_ui_with_layout(
                            Vec2::new(52.0, 18.0),
                            egui::Layout::top_down(Align::RIGHT),
                            |ui| {
                                ui.label(
                                    RichText::new(marker)
                                        .size(11.0)
                                        .strong()
                                        .color(self.liquid_muted_color()),
                                );
                            },
                        );
                        ui.add_space(8.0);
                        ui.add(
                            egui::Label::new(
                                RichText::new(body)
                                    .size(14.0)
                                    .color(self.liquid_muted_color()),
                            )
                            .wrap()
                            .selectable(true),
                        );
                    });
                    ui.add_space(7.0);
                }
            });
    }

    /// Theme-aware main text color for Liquid view reading controls and body text.
    fn liquid_ink_color(&self) -> Color32 {
        match self.liquid_theme {
            LiquidTheme::Paper => INK,
            LiquidTheme::Sepia => Color32::from_rgb(65, 48, 32),
            LiquidTheme::Dark => Color32::from_rgb(232, 228, 220),
        }
    }

    /// Theme-aware secondary text color for Liquid view.
    fn liquid_muted_color(&self) -> Color32 {
        match self.liquid_theme {
            LiquidTheme::Paper => MUTED_INK,
            LiquidTheme::Sepia => Color32::from_rgb(125, 100, 75),
            LiquidTheme::Dark => Color32::from_rgb(170, 165, 155),
        }
    }

    fn draw_liquid_controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new("Text size")
                    .size(11.0)
                    .color(self.liquid_muted_color()),
            );
            let mut scale = self.liquid_text_scale;
            let scale_label = format!("{:.0}%", scale * 100.0);
            if ui
                .add(
                    egui::Slider::new(&mut scale, 0.65..=1.85)
                        .step_by(0.05)
                        .show_value(false)
                        .text(scale_label),
                )
                .changed()
            {
                self.liquid_text_scale = scale.clamp(0.65, 1.85);
            }
            ui.add_space(12.0);

            ui.label(
                RichText::new("Max width")
                    .size(11.0)
                    .color(self.liquid_muted_color()),
            );
            let mut w = self.liquid_max_width;
            if ui
                .add(
                    egui::Slider::new(&mut w, 480.0..=1280.0)
                        .step_by(20.0)
                        .show_value(true)
                        .text("px"),
                )
                .changed()
            {
                self.liquid_max_width = w.clamp(480.0, 1280.0);
            }
            ui.add_space(12.0);

            let theme_label = match self.liquid_theme {
                LiquidTheme::Paper => "Paper",
                LiquidTheme::Sepia => "Sepia",
                LiquidTheme::Dark => "Dark",
            };
            if ui
                .button(
                    RichText::new(format!("Theme: {}", theme_label))
                        .size(11.0)
                        .color(self.liquid_ink_color()),
                )
                .clicked()
            {
                self.liquid_theme = match self.liquid_theme {
                    LiquidTheme::Paper => LiquidTheme::Sepia,
                    LiquidTheme::Sepia => LiquidTheme::Dark,
                    LiquidTheme::Dark => LiquidTheme::Paper,
                };
            }
            if ui.button(RichText::new("Reset").size(11.0)).clicked() {
                self.liquid_text_scale = 1.0;
                self.liquid_max_width = 920.0;
                self.liquid_theme = LiquidTheme::Paper;
            }
            ui.add_space(12.0);
            // #29: reveal normally-hidden furniture (headers/footers/TOC/noise/tables).
            let furniture_label = if self.liquid_show_hidden_furniture {
                "Hide furniture"
            } else {
                "Show furniture"
            };
            if ui
                .button(
                    RichText::new(furniture_label)
                        .size(11.0)
                        .color(self.liquid_ink_color()),
                )
                .on_hover_text(
                    "Reveal headers, page numbers, TOC, and other hidden furniture as dimmed lines",
                )
                .clicked()
            {
                self.liquid_show_hidden_furniture = !self.liquid_show_hidden_furniture;
            }
            ui.add_space(12.0);
            let pending = self.pending_liquid_feedback_count();
            if ui
                .add_enabled(
                    pending > 0,
                    egui::Button::new(
                        RichText::new(format!("Retrain Liquid ({pending})")).size(11.0),
                    ),
                )
                .on_hover_text("Export pending Liquid annotations into the retraining queue")
                .clicked()
            {
                self.queue_pending_liquid_feedback_for_retraining();
            }
        });
        ui.add_space(6.0);
    }

    fn pending_liquid_feedback_count(&self) -> usize {
        self.liquid_feedback
            .iter()
            .filter(|entry| entry.submitted_at.is_none())
            .count()
    }

    fn upsert_liquid_feedback(
        &mut self,
        liquid_document: &LiquidDocument,
        block_index: usize,
        block: &LiquidBlock,
        expected_role: Option<LiquidBlockRole>,
    ) {
        let Some(document) = self.document.as_ref() else {
            return;
        };
        let id = liquid_feedback_id(&liquid_document.source_signature, block_index, block);
        let now = comment_timestamp();
        if let Some(entry) = self.liquid_feedback.iter_mut().find(|entry| entry.id == id) {
            entry.expected_role = expected_role;
            entry.original_role = block.role;
            entry.block_text = block.text.clone();
            entry.source_lines = liquid_block_source_lines(liquid_document, block_index);
            entry.updated_at = now;
            entry.submitted_at = None;
        } else {
            self.liquid_feedback.push(LiquidFeedback {
                id,
                document_path: document.path.clone(),
                document_title: document.title.clone(),
                source_signature: liquid_document.source_signature.clone(),
                block_index,
                original_role: block.role,
                expected_role,
                block_text: block.text.clone(),
                source_lines: liquid_block_source_lines(liquid_document, block_index),
                note: String::new(),
                created_at: now.clone(),
                updated_at: now,
                submitted_at: None,
            });
        }
        self.save_current_liquid_feedback();
        self.status = format!(
            "{} Liquid annotation(s) pending.",
            self.pending_liquid_feedback_count()
        );
    }

    /// #30: apply a reader's quick correction — records the block-level feedback (existing
    /// retrain path) AND appends per-source-line records in the human-gold audit schema
    /// (path/page_index/line_index/text/gold_role) so reader corrections feed the label pipeline.
    fn apply_reader_correction(
        &mut self,
        liquid_document: &LiquidDocument,
        block_index: usize,
        block: &LiquidBlock,
        expected_role: LiquidBlockRole,
        gold_role: &str,
        action: &str,
    ) {
        self.upsert_liquid_feedback(liquid_document, block_index, block, Some(expected_role));
        let source_lines = liquid_block_source_lines(liquid_document, block_index);
        let Some(document) = self.document.as_ref() else {
            return;
        };
        let ts = comment_timestamp();
        match append_reader_corrections(document, &source_lines, gold_role, action, &ts) {
            Ok(count) => {
                self.status = format!("Correction logged ({action}, {count} line(s)).");
            }
            Err(error) => {
                self.push_error_notice(format!("Correction saved; audit log failed: {error}"));
            }
        }
    }

    /// #30: log a doc-level "reflow rejected" signal when the reader leaves a Liquid view for
    /// the fixed layout — a cheap negative signal about reflow quality for the whole document.
    fn log_reflow_rejected(&mut self, from: DocumentViewMode) {
        let Some(document) = self.document.as_ref() else {
            return;
        };
        let ts = comment_timestamp();
        let from = match from {
            DocumentViewMode::Liquid => "liquid",
            DocumentViewMode::LiquidMode2 => "liquid_mode2",
            DocumentViewMode::Pdf => "pdf",
        };
        if let Err(error) = append_reader_event(document, "reflow_rejected", from, &ts) {
            self.push_error_notice(format!("Could not log reflow feedback: {error}"));
        }
    }

    fn save_current_liquid_feedback(&mut self) {
        let Some(document) = self.document.as_ref() else {
            return;
        };
        match save_liquid_feedback(document, &self.liquid_feedback) {
            Ok(()) => {}
            Err(error) => {
                self.push_error_notice(error);
            }
        }
    }

    fn queue_pending_liquid_feedback_for_retraining(&mut self) {
        let pending = self
            .liquid_feedback
            .iter()
            .filter(|entry| entry.submitted_at.is_none())
            .cloned()
            .collect::<Vec<_>>();
        if pending.is_empty() {
            return;
        }
        let submitted_at = comment_timestamp();
        match save_liquid_retrain_queue(&pending, &submitted_at) {
            Ok(path) => {
                for entry in &mut self.liquid_feedback {
                    if entry.submitted_at.is_none() {
                        entry.submitted_at = Some(submitted_at.clone());
                        entry.updated_at = submitted_at.clone();
                    }
                }
                self.save_current_liquid_feedback();
                self.status = format!(
                    "Queued {} Liquid annotation(s) for retraining: {}",
                    pending.len(),
                    path.display()
                );
            }
            Err(error) => {
                self.push_error_notice(error);
            }
        }
    }

    fn draw_document(&mut self, ctx: &Context) {
        self.text_box_action_rect = None;
        self.comment_action_rect = None;
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(CANVAS_FILL))
            .show(ctx, |ui| {
                if self.document.is_none() {
                    self.draw_empty_state(ui, ctx);
                    return;
                }

                if self.view_mode == DocumentViewMode::Liquid {
                    self.ensure_liquid_started(ctx);
                    self.draw_liquid_document(ui, ctx);
                    return;
                }
                if self.view_mode == DocumentViewMode::LiquidMode2 {
                    self.ensure_liquid_mode2_started(ctx);
                    self.draw_liquid_mode2_document(ui, ctx);
                    return;
                }

                let Some(document) = self.document.as_ref() else {
                    return;
                };
                let path = document.path.clone();
                let pages = document.pages.clone();

                let mut document_scroll_area = egui::ScrollArea::both()
                    .id_salt("document_pages")
                    .auto_shrink([false, false]);
                if let Some(offset) = self.pending_document_scroll_offset.take() {
                    document_scroll_area = document_scroll_area.scroll_offset(offset);
                }

                document_scroll_area.show_viewport(ui, |ui, viewport| {
                    let requested_scroll_target = self.scroll_target_page;
                    let mut closest_visible_page =
                        requested_scroll_target.unwrap_or(self.page_index);
                    let mut closest_visible_distance = f32::MAX;

                    let mut content_width = viewport.width().max(ui.available_width()).max(1.0);
                    let mut content_height = 0.0;
                    let mut page_tops = Vec::with_capacity(pages.len());
                    let mut display_sizes = Vec::with_capacity(pages.len());
                    for page_info in &pages {
                        content_height += DOCUMENT_PAGE_GAP;
                        page_tops.push(content_height);
                        let display_size =
                            Vec2::new(page_info.width * self.zoom, page_info.height * self.zoom);
                        content_width = content_width.max(display_size.x);
                        content_height += display_size.y;
                        display_sizes.push(display_size);
                    }

                    let (content_rect, _) = ui.allocate_exact_size(
                        Vec2::new(content_width, content_height),
                        Sense::hover(),
                    );

                    let zoom_delta = ctx.input(|input| {
                        if input.modifiers.ctrl {
                            input.zoom_delta()
                        } else {
                            1.0
                        }
                    });
                    let clip_rect = ui.clip_rect();
                    if (zoom_delta - 1.0).abs() > f32::EPSILON {
                        if let Some(pointer) = ctx.input(|input| input.pointer.hover_pos()) {
                            if clip_rect.contains(pointer) && !display_sizes.is_empty() {
                                let pointer_in_viewport = pointer - clip_rect.min;
                                let anchor_content = Pos2::new(
                                    viewport.left() + pointer_in_viewport.x,
                                    viewport.top() + pointer_in_viewport.y,
                                );
                                let mut anchor_page = 0usize;
                                let mut anchor_distance = f32::MAX;
                                for (page_index, display_size) in display_sizes.iter().enumerate() {
                                    let top = page_tops[page_index];
                                    let bottom = top + display_size.y;
                                    if anchor_content.y >= top && anchor_content.y <= bottom {
                                        anchor_page = page_index;
                                        break;
                                    }
                                    let distance = (anchor_content.y - top)
                                        .abs()
                                        .min((anchor_content.y - bottom).abs());
                                    if distance < anchor_distance {
                                        anchor_distance = distance;
                                        anchor_page = page_index;
                                    }
                                }

                                let old_size = display_sizes[anchor_page];
                                let old_top = page_tops[anchor_page];
                                let old_left = (content_width - old_size.x).max(0.0) * 0.5;
                                let y_fraction = if old_size.y > 0.0 {
                                    ((anchor_content.y - old_top) / old_size.y).clamp(0.0, 1.0)
                                } else {
                                    0.0
                                };
                                let x_fraction = if old_size.x > 0.0 {
                                    ((anchor_content.x - old_left) / old_size.x).clamp(0.0, 1.0)
                                } else {
                                    0.5
                                };

                                let new_zoom = normalized_pdf_zoom(self.target_zoom * zoom_delta);
                                let mut new_content_width =
                                    viewport.width().max(ui.available_width()).max(1.0);
                                let mut new_content_height = 0.0;
                                let mut new_page_top = 0.0;
                                let mut new_page_size = Vec2::ZERO;
                                for (page_index, page_info) in pages.iter().enumerate() {
                                    new_content_height += DOCUMENT_PAGE_GAP;
                                    let display_size = Vec2::new(
                                        page_info.width * new_zoom,
                                        page_info.height * new_zoom,
                                    );
                                    if page_index == anchor_page {
                                        new_page_top = new_content_height;
                                        new_page_size = display_size;
                                    }
                                    new_content_width = new_content_width.max(display_size.x);
                                    new_content_height += display_size.y;
                                }
                                let new_page_left =
                                    (new_content_width - new_page_size.x).max(0.0) * 0.5;
                                let desired_anchor = Pos2::new(
                                    new_page_left + new_page_size.x * x_fraction,
                                    new_page_top + new_page_size.y * y_fraction,
                                );
                                let max_offset = Vec2::new(
                                    (new_content_width - clip_rect.width()).max(0.0),
                                    (new_content_height - clip_rect.height()).max(0.0),
                                );
                                let viewport_center =
                                    Vec2::new(clip_rect.width() * 0.5, clip_rect.height() * 0.5);
                                self.pending_document_scroll_offset = Some(Vec2::new(
                                    (desired_anchor.x - viewport_center.x).clamp(0.0, max_offset.x),
                                    (desired_anchor.y - viewport_center.y).clamp(0.0, max_offset.y),
                                ));
                                self.scroll_target_page = None;
                                self.set_zoom(new_zoom);
                                self.status = format!("{:.0}% zoom", self.target_zoom * 100.0);
                                ctx.request_repaint_after(RENDER_POLL_INTERVAL);
                            }
                        }
                    }

                    let render_window = viewport.expand2(Vec2::new(220.0, 900.0));
                    let mut visible_pages = Vec::new();
                    let mut visible_page_ranges = Vec::new();

                    for (page_index, display_size) in display_sizes.iter().enumerate() {
                        let top = page_tops[page_index];
                        let bottom = top + display_size.y;
                        let intersects_render_window =
                            bottom >= render_window.top() && top <= render_window.bottom();
                        if intersects_render_window || requested_scroll_target == Some(page_index) {
                            visible_pages.push(page_index);
                        }

                        let visible_top = top.max(viewport.top());
                        let visible_bottom = bottom.min(viewport.bottom());
                        let visible_height = (visible_bottom - visible_top).max(0.0);
                        if visible_height > 0.5 && display_size.y > 0.0 {
                            visible_page_ranges.push(VisiblePageRange {
                                page_index,
                                top_fraction: ((visible_top - top) / display_size.y)
                                    .clamp(0.0, 1.0),
                                bottom_fraction: ((visible_bottom - top) / display_size.y)
                                    .clamp(0.0, 1.0),
                                coverage: (visible_height / display_size.y).clamp(0.0, 1.0),
                            });
                        }

                        if requested_scroll_target.is_none()
                            && bottom >= viewport.top()
                            && top <= viewport.bottom()
                        {
                            let distance =
                                ((top + display_size.y * 0.5) - viewport.center().y).abs();
                            if distance < closest_visible_distance {
                                closest_visible_distance = distance;
                                closest_visible_page = page_index;
                            }
                        }
                    }

                    for page_index in visible_pages {
                        let page_info = &pages[page_index];
                        let display_size = display_sizes[page_index];
                        let left =
                            content_rect.left() + (content_width - display_size.x).max(0.0) * 0.5;
                        let top = content_rect.top() + page_tops[page_index];
                        let rect = Rect::from_min_size(Pos2::new(left, top), display_size);
                        let placement = PagePlacement {
                            rect,
                            page_width: page_info.width,
                            page_height: page_info.height,
                        };
                        let response = ui.interact(
                            rect,
                            ui.id().with(("document-page", page_index)),
                            Sense::click_and_drag(),
                        );
                        let response = if let Some(url) =
                            self.hovered_web_link_url(&response, &placement, page_index)
                        {
                            response
                                .on_hover_cursor(CursorIcon::PointingHand)
                                .on_hover_text(url)
                        } else {
                            response.on_hover_cursor(tool_cursor(self.active_tool))
                        };
                        if requested_scroll_target == Some(page_index) {
                            response.scroll_to_me(Some(Align::Center));
                        }

                        let painter = ui.painter();
                        let page_shadow = Shadow {
                            offset: [0, 8],
                            blur: 24,
                            spread: 0,
                            color: Color32::from_black_alpha(48),
                        };
                        painter.add(page_shadow.as_shape(rect, 3));
                        painter.rect_filled(rect, 3, PAPER_FILL);
                        painter.rect_stroke(
                            rect,
                            3,
                            Stroke::new(1.0, PAPER_STROKE),
                            egui::StrokeKind::Inside,
                        );

                        if let Some(page_texture) = self.ensure_page_texture(ctx, &path, page_index)
                        {
                            painter.image(
                                page_texture.texture_id,
                                rect,
                                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                                Color32::WHITE,
                            );
                        } else {
                            self.draw_page_loading_placeholder(ctx, painter, rect, page_index);
                        }
                        if self.active_tool == Tool::Select || self.active_tool == Tool::Marker {
                            self.request_text_chars(ctx, &path, page_index);
                        }

                        let chip_rect = Rect::from_min_size(
                            rect.left_top() + Vec2::new(12.0, 12.0),
                            Vec2::new(78.0, 26.0),
                        );
                        painter.rect_filled(
                            chip_rect,
                            13,
                            Color32::from_rgba_unmultiplied(45, 39, 32, 168),
                        );
                        painter.text(
                            chip_rect.center(),
                            Align2::CENTER_CENTER,
                            format!("Page {}", page_index + 1),
                            FontId::proportional(14.0),
                            Color32::from_rgb(255, 252, 244),
                        );

                        let painter = ui.painter_at(rect);
                        self.draw_search_overlay(&painter, &placement, page_index);
                        self.draw_liquid_provenance_overlay(&painter, &placement, page_index);
                        self.draw_text_selection(&painter, &placement, page_index);
                        self.draw_annotations(&painter, &placement, page_index);
                        self.draw_drag_preview(&painter, &placement, page_index);
                        self.draw_hovered_web_link(&painter, &response, &placement, page_index);
                        let mut annotation_interacted =
                            self.draw_text_box_controls(ui, ctx, &placement, page_index);
                        annotation_interacted |=
                            self.draw_text_box_action_palette(ctx, &placement, page_index);
                        annotation_interacted |=
                            self.draw_comment_controls(ui, ctx, &placement, page_index);
                        self.handle_page_interaction(
                            ctx,
                            &response,
                            &placement,
                            page_index,
                            annotation_interacted,
                        );
                        if !annotation_interacted {
                            self.draw_page_context_menu(ctx, &response, &placement, page_index);
                        }
                        self.draw_selection_action_palette(ctx, &placement, page_index);
                    }

                    self.visible_page_ranges = visible_page_ranges;
                    self.prune_marker_animations();

                    let previous_page = self.page_index;
                    self.page_index = closest_visible_page;
                    if self.page_index != previous_page {
                        self.thumbnail_scroll_target = Some(self.page_index);
                        self.prune_page_texture_cache();
                    }
                    self.prefetch_neighbor_pages(ctx, &path);
                    if requested_scroll_target.is_some() {
                        self.scroll_target_page = None;
                    }
                });
            });
        self.draw_liquid_status_popover(ctx);
    }

    fn draw_page_loading_placeholder(
        &self,
        ctx: &Context,
        painter: &egui::Painter,
        rect: Rect,
        page_index: usize,
    ) {
        let time = ctx.input(|input| input.time) as f32;
        let travel = rect.width() + 180.0;
        let phase = (time * 150.0 + page_index as f32 * 37.0).rem_euclid(travel);
        let stripe_center = rect.left() - 90.0 + phase;
        let stripe = Rect::from_center_size(
            Pos2::new(stripe_center, rect.center().y),
            Vec2::new(72.0, rect.height() * 1.25),
        )
        .intersect(rect.shrink(6.0));

        painter.rect_filled(
            stripe,
            2,
            Color32::from_rgba_unmultiplied(255, 255, 255, 46),
        );
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            "Rendering page",
            FontId::proportional(18.0),
            MUTED_INK,
        );
        ctx.request_repaint_after(RENDER_POLL_INTERVAL);
    }

    fn prefetch_neighbor_pages(&mut self, ctx: &Context, path: &Path) {
        if self.zoom_render_is_debounced() {
            return;
        }

        let pages = self
            .document
            .as_ref()
            .map(|document| {
                document
                    .pages
                    .iter()
                    .map(|page| (page.width, page.height))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let page_count = pages.len();
        if page_count == 0 {
            return;
        }

        let start = self.page_index.saturating_sub(PAGE_PREFETCH_RADIUS);
        let end = (self.page_index + PAGE_PREFETCH_RADIUS).min(page_count - 1);
        let mut candidates = (start..=end).collect::<Vec<_>>();
        candidates.sort_by_key(|page_index| page_index.abs_diff(self.page_index));

        for page_index in candidates {
            let (page_width, page_height) = pages[page_index];
            let render_scale = self.page_render_scale(ctx, page_width, page_height);
            let is_current = self
                .page_textures
                .get(&page_index)
                .is_some_and(|texture| (texture.render_scale - render_scale).abs() < f32::EPSILON);
            if !is_current {
                self.request_page_render(ctx, path, page_index, render_scale);
            }
        }
    }

    fn draw_search_overlay(
        &self,
        painter: &egui::Painter,
        placement: &PagePlacement,
        page_index: usize,
    ) {
        if !self.search_state.show_highlights {
            return;
        }

        let Some(document) = self.document.as_ref() else {
            return;
        };

        for hit in self
            .search_state
            .hits
            .iter()
            .filter(|hit| hit.page_index == page_index)
        {
            if let Some(pdf_rect) = self.estimated_hit_rect(document, hit) {
                let rect = placement.pdf_rect_to_screen(pdf_rect);
                painter.rect_filled(rect, 0.0, Color32::from_rgba_unmultiplied(255, 232, 64, 72));
            }
        }
    }

    fn prune_page_texture_cache(&mut self) {
        let page_count = self
            .document
            .as_ref()
            .map(|document| document.page_count)
            .unwrap_or_default();
        if page_count <= PAGE_TEXTURE_CACHE_CAP
            || self.page_textures.len() <= PAGE_TEXTURE_CACHE_CAP
        {
            return;
        }

        let current_page = self.page_index;
        while self.page_textures.len() > PAGE_TEXTURE_CACHE_CAP {
            let Some(victim_page) = self
                .page_textures
                .iter()
                .filter(|(page_index, _)| **page_index != current_page)
                .min_by_key(|(_, texture)| texture.last_used)
                .map(|(page_index, _)| *page_index)
            else {
                break;
            };
            self.page_textures.remove(&victim_page);
        }
    }

    /// Draw a highlight, animating the ink "laying down" if a live animation is
    /// registered for it. Falls back to a flat fill once settled (or when the
    /// user has opted into reduced motion), so the resting look is unchanged.
    fn draw_highlight_marker(
        &self,
        painter: &egui::Painter,
        page_index: usize,
        pdf_rect: PdfRect,
        rect: Rect,
        color_rgb: [f32; 3],
        target_alpha: u8,
    ) {
        let seed = marker_seed(pdf_rect);

        // No live animation for this exact segment => settled. Breathe gently
        // unless the user has opted into reduced motion.
        let Some(anim) = self
            .marker_animations
            .iter()
            .find(|anim| anim.page_index == page_index && anim.rect == pdf_rect)
        else {
            if self.settings.reduce_motion {
                paint_marker_stroke(painter, rect, seed, color_rgb, target_alpha);
                return;
            }
            let time = painter.ctx().input(|input| input.time) as f32;
            let motion = settled_marker_motion(time, seed, target_alpha);
            painter.ctx().request_repaint_after(MARKER_BREATH_REPAINT);
            paint_marker_stroke(painter, rect, seed, color_rgb, motion.alpha);

            // A narrow glint passes over the existing textured stroke. It fades
            // in and out at the ends of its long cycle, so it never flashes.
            if motion.sheen_alpha > 0 && rect.width() > 8.0 {
                let band_width = (rect.width() * 0.14).clamp(10.0, 38.0);
                let travel = rect.width() + band_width * 2.0;
                let center = rect.left() - band_width + travel * motion.sheen_progress;
                let band = Rect::from_min_max(
                    Pos2::new(center - band_width * 0.5, rect.top()),
                    Pos2::new(center + band_width * 0.5, rect.bottom()),
                );
                let sheen = painter.with_clip_rect(band.intersect(painter.clip_rect()));
                paint_marker_stroke(
                    &sheen,
                    rect,
                    seed,
                    marker_sheen_rgb(color_rgb),
                    motion.sheen_alpha,
                );
            }
            return;
        };

        let elapsed = anim.born.elapsed();
        if elapsed < anim.delay {
            // Still waiting its turn in the cascade: no ink down yet. Keep the
            // clock running so it starts on time, but draw nothing.
            painter.ctx().request_repaint();
            return;
        }

        let wipe = MARKER_WIPE_DURATION.as_secs_f32();
        let settle = MARKER_SETTLE_DURATION.as_secs_f32();
        let l = (elapsed - anim.delay).as_secs_f32();
        if l >= wipe + settle {
            paint_marker_stroke(painter, rect, seed, color_rgb, target_alpha);
            return;
        }

        // Animation in flight: keep the frame clock running.
        painter.ctx().request_repaint();

        if l < wipe {
            // Lay the ink down left-to-right, easing out as the tip slows. The
            // textured stroke is revealed by clipping to the wiped width.
            let t = ease_out_cubic((l / wipe).clamp(0.0, 1.0));
            let reveal_w = rect.width() * t;
            if reveal_w <= 0.5 {
                return;
            }
            let clip =
                Rect::from_min_max(rect.min, Pos2::new(rect.left() + reveal_w, rect.bottom()));
            let revealed = painter.with_clip_rect(clip.intersect(painter.clip_rect()));
            paint_marker_stroke(&revealed, rect, seed, color_rgb, target_alpha);

            // Wet leading pool: a darker band of pigment at the marker tip.
            let pool_w = (rect.width() * 0.06).clamp(3.0, 14.0).min(reveal_w);
            let pool = Rect::from_min_max(
                Pos2::new(rect.left() + reveal_w - pool_w, rect.top()),
                Pos2::new(rect.left() + reveal_w, rect.bottom()),
            );
            let pool_alpha = ((target_alpha as f32) * 1.6).min(235.0) as u8;
            revealed.rect_filled(
                pool,
                MARKER_CORNER_RADIUS,
                color_from_rgb(color_rgb, pool_alpha),
            );
        } else {
            // Ink dries: the full stroke with a sheen that quickly relaxes away.
            paint_marker_stroke(painter, rect, seed, color_rgb, target_alpha);
            let s = ((l - wipe) / settle).clamp(0.0, 1.0);
            let sheen_alpha = ((target_alpha as f32) * 0.45 * (1.0 - ease_in_cubic(s))) as u8;
            if sheen_alpha > 0 {
                painter.rect_filled(
                    rect,
                    MARKER_CORNER_RADIUS,
                    color_from_rgb(color_rgb, sheen_alpha),
                );
            }
        }
    }

    /// Drop highlight animations once their stroke has finished settling.
    fn prune_marker_animations(&mut self) {
        if self.marker_animations.is_empty() {
            return;
        }
        let life = MARKER_WIPE_DURATION + MARKER_SETTLE_DURATION;
        self.marker_animations
            .retain(|anim| anim.born.elapsed() < anim.delay + life);
    }

    fn draw_annotations(
        &self,
        painter: &egui::Painter,
        placement: &PagePlacement,
        page_index: usize,
    ) {
        for (annotation_index, annotation) in self
            .annotations
            .iter()
            .enumerate()
            .filter(|(_, annotation)| annotation.page_index == page_index)
        {
            let rect = placement.pdf_rect_to_screen(annotation.rect);
            match &annotation.kind {
                AnnotationKind::Marker {
                    color_rgb,
                    opacity,
                    style,
                } => {
                    let alpha = (opacity.clamp(0.0, 1.0) * 255.0) as u8;
                    let color = color_from_rgb(*color_rgb, alpha);
                    match style {
                        MarkerStyle::Highlight => {
                            self.draw_highlight_marker(
                                painter,
                                page_index,
                                annotation.rect,
                                rect,
                                *color_rgb,
                                alpha,
                            );
                        }
                        MarkerStyle::Underline => {
                            let y = rect.bottom() - 2.0;
                            painter.line_segment(
                                [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
                                Stroke::new(2.0, color),
                            );
                        }
                    }
                }
                AnnotationKind::TextBox { text, .. } => {
                    painter.rect_filled(
                        rect,
                        0.0,
                        Color32::from_rgba_unmultiplied(255, 255, 255, 220),
                    );
                    painter.rect_stroke(
                        rect,
                        0.0,
                        Stroke::new(1.0, Color32::from_rgb(72, 106, 180)),
                        egui::StrokeKind::Inside,
                    );
                    painter.text(
                        rect.left_top() + Vec2::new(6.0, 6.0),
                        Align2::LEFT_TOP,
                        text,
                        FontId::proportional(14.0),
                        Color32::BLACK,
                    );
                }
                AnnotationKind::Comment {
                    color_rgb, anchor, ..
                } => {
                    // The marker sits on the referenced text (the anchor), not
                    // on the margin card stored in `annotation.rect`.
                    let anchor_point = placement.pdf_to_screen(*anchor);
                    let marker_rect = comment_marker_screen_rect(Rect::from_center_size(
                        anchor_point,
                        Vec2::ZERO,
                    ));
                    let selected = self.selected_comment == Some(annotation_index);
                    draw_comment_marker(
                        painter,
                        marker_rect,
                        *color_rgb,
                        self.comment_ordinal(annotation_index),
                        selected,
                    );
                }
                AnnotationKind::Signature {
                    strokes, signer, ..
                } => {
                    for stroke in strokes {
                        draw_pdf_stroke(
                            painter,
                            placement,
                            stroke,
                            Stroke::new(2.0, Color32::BLACK),
                        );
                    }
                    if !signer.trim().is_empty() {
                        painter.text(
                            rect.left_bottom() + Vec2::new(0.0, 4.0),
                            Align2::LEFT_TOP,
                            signer,
                            FontId::proportional(12.0),
                            Color32::DARK_GRAY,
                        );
                    }
                }
            }
        }
    }

    fn draw_text_selection(
        &self,
        painter: &egui::Painter,
        placement: &PagePlacement,
        page_index: usize,
    ) {
        for pdf_rect in self.selection_rects_for_page(page_index) {
            let rect = placement.pdf_rect_to_screen(pdf_rect);
            painter.rect_filled(rect, 1.0, Color32::from_rgba_unmultiplied(84, 132, 212, 92));
        }
    }

    fn draw_drag_preview(
        &self,
        painter: &egui::Painter,
        placement: &PagePlacement,
        page_index: usize,
    ) {
        if self.active_drag_page != Some(page_index) {
            return;
        }

        if let Some(preview) = self.drag_preview {
            let rect = placement.pdf_rect_to_screen(preview);
            match self.active_tool {
                Tool::TextBox => {
                    painter.rect_stroke(
                        rect,
                        0.0,
                        Stroke::new(1.0, Color32::from_rgb(72, 106, 180)),
                        egui::StrokeKind::Inside,
                    );
                }
                _ => {}
            }
        }

        if !self.active_signature_stroke.is_empty() {
            draw_pdf_stroke(
                painter,
                placement,
                &self.active_signature_stroke,
                Stroke::new(2.0, Color32::BLACK),
            );
        }
    }

    fn draw_text_box_controls(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &Context,
        placement: &PagePlacement,
        page_index: usize,
    ) -> bool {
        let text_box_indices = self
            .annotations
            .iter()
            .enumerate()
            .filter(|(_, annotation)| {
                annotation.page_index == page_index
                    && matches!(annotation.kind, AnnotationKind::TextBox { .. })
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();

        let mut interacted = false;
        for annotation_index in text_box_indices {
            let Some(annotation) = self.annotations.get(annotation_index) else {
                continue;
            };
            let annotation_rect = annotation.rect;
            let screen_rect = placement.pdf_rect_to_screen(annotation_rect);
            let selected = self.selected_text_box == Some(annotation_index);
            let editing = self.editing_text_box == Some(annotation_index);
            let response = ui
                .interact(
                    screen_rect,
                    ui.id().with(("text-box-annotation", annotation_index)),
                    Sense::click_and_drag(),
                )
                .on_hover_cursor(if response_is_dragging(ctx) {
                    CursorIcon::Grabbing
                } else {
                    CursorIcon::Grab
                });

            let pointer_down = ctx.input(|input| input.pointer.any_down());
            interacted |= response.clicked()
                || response.double_clicked()
                || response.drag_started()
                || response.dragged()
                || response.drag_stopped()
                || (response.hovered() && pointer_down);

            let mut edit_requested = false;
            let mut delete_requested = false;
            response.context_menu(|ui| {
                if ui.button("Edit").clicked() {
                    edit_requested = true;
                    ui.close();
                }
                if ui.button("Delete").clicked() {
                    delete_requested = true;
                    ui.close();
                }
            });

            if response.clicked() {
                self.select_text_box(annotation_index);
            }
            if response.double_clicked() {
                edit_requested = true;
            }
            if response.drag_started() {
                if let Some(pos) = response.interact_pointer_pos() {
                    if let Some(start_pdf) = placement.screen_to_pdf(pos) {
                        self.select_text_box(annotation_index);
                        self.editing_text_box = None;
                        self.text_box_drag = Some(TextBoxDrag {
                            annotation_index,
                            start_pdf,
                            original_rect: annotation_rect,
                        });
                    }
                }
            }
            if response.dragged() {
                if let Some(drag) = self.text_box_drag {
                    if drag.annotation_index == annotation_index {
                        if let Some(pos) = response.interact_pointer_pos() {
                            if let Some(pdf) = placement.screen_to_pdf(pos) {
                                let dx = pdf.0 - drag.start_pdf.0;
                                let dy = pdf.1 - drag.start_pdf.1;
                                if let Some(annotation) = self.annotations.get_mut(annotation_index)
                                {
                                    annotation.rect = translate_pdf_rect_clamped(
                                        drag.original_rect,
                                        dx,
                                        dy,
                                        placement.page_width,
                                        placement.page_height,
                                    );
                                }
                            }
                        }
                    }
                }
            }
            if response.drag_stopped()
                && self
                    .text_box_drag
                    .is_some_and(|drag| drag.annotation_index == annotation_index)
            {
                self.text_box_drag = None;
                self.status = "Text box moved.".to_owned();
            }

            if selected {
                let painter = ui.painter();
                painter.rect_stroke(
                    screen_rect.expand(3.0),
                    2,
                    Stroke::new(1.6, Color32::from_rgb(146, 103, 52)),
                    egui::StrokeKind::Outside,
                );
            }

            if edit_requested {
                self.start_text_box_edit(annotation_index);
            }
            if editing {
                interacted |= self.draw_text_box_editor(ctx, annotation_index, screen_rect);
            }
            if delete_requested {
                self.delete_text_box(annotation_index);
                return true;
            }
        }

        interacted
    }

    fn draw_comment_controls(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &Context,
        placement: &PagePlacement,
        page_index: usize,
    ) -> bool {
        let comment_indices = self
            .annotations
            .iter()
            .enumerate()
            .filter(|(_, annotation)| {
                annotation.page_index == page_index
                    && matches!(annotation.kind, AnnotationKind::Comment { .. })
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();

        let mut interacted = false;
        for annotation_index in comment_indices {
            let Some(annotation) = self.annotations.get(annotation_index) else {
                continue;
            };
            let anchor = match &annotation.kind {
                AnnotationKind::Comment { anchor, .. } => *anchor,
                _ => continue,
            };
            let card_rect = annotation.rect;
            let anchor_screen = placement.pdf_to_screen(anchor);
            let marker_rect =
                comment_marker_screen_rect(Rect::from_center_size(anchor_screen, Vec2::ZERO));
            let selected = self.selected_comment == Some(annotation_index);
            let editing = self.editing_comment == Some(annotation_index);

            // The on-text marker is a click target only; the card it opens (in
            // the margin) is what the user drags.
            let response = ui
                .interact(
                    marker_rect.expand(5.0),
                    ui.id().with(("comment-annotation", annotation_index)),
                    Sense::click(),
                )
                .on_hover_cursor(CursorIcon::PointingHand);

            interacted |= response.clicked() || response.secondary_clicked() || response.hovered();

            let mut edit_requested = false;
            let mut delete_requested = false;
            response.context_menu(|ui| {
                if ui.button("Edit").clicked() {
                    edit_requested = true;
                    ui.close();
                }
                if ui.button("Delete").clicked() {
                    delete_requested = true;
                    ui.close();
                }
            });

            if response.clicked() {
                edit_requested = true;
            }
            if edit_requested {
                self.start_comment_edit(annotation_index);
            }

            if selected || editing {
                interacted |= self.draw_comment_editor(
                    ctx,
                    ui,
                    placement,
                    annotation_index,
                    anchor_screen,
                    card_rect,
                );
            }
            if delete_requested {
                self.delete_comment(ctx, annotation_index);
                return true;
            }
        }

        interacted
    }

    fn draw_comment_editor(
        &mut self,
        ctx: &Context,
        ui: &mut egui::Ui,
        placement: &PagePlacement,
        annotation_index: usize,
        anchor_screen: Pos2,
        card_rect: PdfRect,
    ) -> bool {
        let mut text_changed = false;
        let mut selected_color = None;
        let mut done_clicked = false;
        let mut delete_clicked = false;
        let mut editor_active = false;
        let mut card_moved = false;
        let comment_number = self.comment_ordinal(annotation_index);

        // The card lives in the page margin on its stored side; the editor
        // opens away from the page body so it doesn't cover the text.
        let side = comment_card_side(card_rect, placement.page_width);
        let card_edge = placement.pdf_to_screen((card_rect.left, card_rect.top));
        let position_x = match side {
            CommentSide::Left => card_edge.x - COMMENT_CARD_WIDTH - COMMENT_CARD_GAP,
            CommentSide::Right => card_edge.x + COMMENT_CARD_GAP,
        };
        let position = Pos2::new(position_x, (card_edge.y - 16.0).max(52.0));

        let area = egui::Area::new(egui::Id::new(("comment-editor", annotation_index)))
            .order(egui::Order::Foreground)
            .fixed_pos(position)
            .show(ctx, |ui| {
                egui::Frame::NONE
                    .fill(Color32::from_rgba_unmultiplied(255, 253, 247, 248))
                    .stroke(Stroke::new(1.3, Color32::from_rgb(184, 141, 68)))
                    .corner_radius(7)
                    .inner_margin(Margin::symmetric(10, 9))
                    .show(ui, |ui| {
                        ui.set_min_width(300.0);
                        ui.set_max_width(330.0);
                        ui.horizontal(|ui| {
                            // Drag handle: moves the card, not the anchor.
                            let grip = ui
                                .add(
                                    egui::Label::new(
                                        RichText::new("\u{2059}").strong().color(MUTED_INK),
                                    )
                                    .sense(Sense::drag()),
                                )
                                .on_hover_cursor(CursorIcon::Grab);
                            if grip.drag_started() {
                                if let Some(pos) = grip.interact_pointer_pos() {
                                    self.comment_drag = Some(CommentDrag {
                                        annotation_index,
                                        start_pdf: placement.screen_to_pdf_unclamped(pos),
                                        original_rect: card_rect,
                                    });
                                }
                            }
                            if grip.dragged() {
                                if let (Some(drag), Some(pos)) =
                                    (self.comment_drag, grip.interact_pointer_pos())
                                {
                                    if drag.annotation_index == annotation_index {
                                        let now = placement.screen_to_pdf_unclamped(pos);
                                        let dx = now.0 - drag.start_pdf.0;
                                        let dy = now.1 - drag.start_pdf.1;
                                        if let Some(annotation) =
                                            self.annotations.get_mut(annotation_index)
                                        {
                                            annotation.rect =
                                                translate_pdf_rect_free(drag.original_rect, dx, dy);
                                        }
                                    }
                                }
                            }
                            if grip.drag_stopped() {
                                self.comment_drag = None;
                                card_moved = true;
                            }
                            ui.label(
                                RichText::new(format!("Comment {comment_number}"))
                                    .strong()
                                    .color(INK),
                            );
                            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                                if ui.button("Delete").clicked() {
                                    delete_clicked = true;
                                }
                                if ui.button("Done").clicked() {
                                    done_clicked = true;
                                }
                            });
                        });
                        ui.add_space(5.0);

                        if let Some(EditorAnnotation {
                            kind:
                                AnnotationKind::Comment {
                                    text, color_rgb, ..
                                },
                            ..
                        }) = self.annotations.get_mut(annotation_index)
                        {
                            let response = ui.add_sized(
                                Vec2::new(300.0, 118.0),
                                egui::TextEdit::multiline(text)
                                    .font(egui::TextStyle::Body)
                                    .hint_text("Comment")
                                    .desired_rows(5)
                                    .lock_focus(true),
                            );
                            if self.comment_focus_request == Some(annotation_index) {
                                response.request_focus();
                            }
                            text_changed |= response.changed();
                            editor_active |= response.has_focus() || response.hovered();

                            ui.add_space(6.0);
                            ui.horizontal(|ui| {
                                for (index, preset) in COMMENT_COLOR_PRESETS.iter().enumerate() {
                                    let color = color_from_rgb(preset.color_rgb, 235);
                                    let selected = close_rgb(*color_rgb, preset.color_rgb);
                                    let stroke = if selected {
                                        Stroke::new(2.0, Color32::from_rgb(72, 48, 26))
                                    } else {
                                        Stroke::new(1.0, Color32::from_rgb(210, 200, 184))
                                    };
                                    if ui
                                        .add(
                                            egui::Button::new("")
                                                .fill(color)
                                                .stroke(stroke)
                                                .min_size(Vec2::splat(20.0)),
                                        )
                                        .on_hover_text(preset.label)
                                        .clicked()
                                    {
                                        selected_color = Some((index, preset.color_rgb));
                                    }
                                }
                            });
                        }
                    });
            });

        let editor_rect = area.response.rect;
        self.comment_action_rect = Some(editor_rect);
        if self.comment_focus_request == Some(annotation_index) {
            self.comment_focus_request = None;
        }

        // Dotted leader from the on-text marker to the margin card.
        let leader_y = anchor_screen
            .y
            .clamp(editor_rect.top() + 6.0, editor_rect.bottom() - 6.0);
        let leader_end = match side {
            CommentSide::Left => Pos2::new(editor_rect.right(), leader_y),
            CommentSide::Right => Pos2::new(editor_rect.left(), leader_y),
        };
        let leader_stroke = Stroke::new(1.3, Color32::from_rgb(150, 120, 70));
        ui.painter().extend(egui::Shape::dashed_line(
            &[anchor_screen, leader_end],
            leader_stroke,
            5.0,
            4.0,
        ));
        ui.painter()
            .circle_filled(anchor_screen, 2.5, leader_stroke.color);

        if card_moved {
            if let Some(EditorAnnotation {
                kind: AnnotationKind::Comment { updated_at, .. },
                ..
            }) = self.annotations.get_mut(annotation_index)
            {
                *updated_at = comment_timestamp();
            }
            self.schedule_comment_autosave_now(ctx);
            self.status = "Comment moved.".to_owned();
        }
        if self.comment_drag.is_some() {
            ctx.request_repaint();
        }

        let color_changed = selected_color.is_some();
        if text_changed || color_changed {
            if let Some(EditorAnnotation {
                kind:
                    AnnotationKind::Comment {
                        color_rgb,
                        updated_at,
                        ..
                    },
                ..
            }) = self.annotations.get_mut(annotation_index)
            {
                if let Some((index, color)) = selected_color {
                    *color_rgb = color;
                    self.comment_color_index = index;
                }
                *updated_at = comment_timestamp();
            }
            if color_changed {
                self.schedule_comment_autosave_now(ctx);
            } else {
                self.schedule_comment_autosave(ctx);
            }
        }

        if done_clicked
            || (self.editing_comment == Some(annotation_index)
                && ctx.input(|input| input.key_pressed(egui::Key::Escape)))
        {
            self.finish_comment_edit();
            self.schedule_comment_autosave_now(ctx);
        }
        if delete_clicked {
            self.delete_comment(ctx, annotation_index);
            return true;
        }

        editor_active || area.response.hovered()
    }

    fn draw_text_box_editor(
        &mut self,
        ctx: &Context,
        annotation_index: usize,
        screen_rect: Rect,
    ) -> bool {
        let mut editor_response = None;
        let area = egui::Area::new(egui::Id::new(("text-box-editor", annotation_index)))
            .order(egui::Order::Foreground)
            .fixed_pos(screen_rect.left_top())
            .show(ctx, |ui| {
                egui::Frame::NONE
                    .fill(Color32::from_rgba_unmultiplied(255, 254, 250, 245))
                    .stroke(Stroke::new(1.4, Color32::from_rgb(72, 106, 180)))
                    .corner_radius(2)
                    .inner_margin(Margin::same(5))
                    .show(ui, |ui| {
                        let edit_size = Vec2::new(
                            (screen_rect.width() - 10.0).max(48.0),
                            (screen_rect.height() - 10.0).max(32.0),
                        );
                        if let Some(EditorAnnotation {
                            kind: AnnotationKind::TextBox { text, .. },
                            ..
                        }) = self.annotations.get_mut(annotation_index)
                        {
                            let response = ui.add_sized(
                                edit_size,
                                egui::TextEdit::multiline(text)
                                    .hint_text("Type here")
                                    .lock_focus(true),
                            );
                            if self.text_box_focus_request == Some(annotation_index) {
                                response.request_focus();
                            }
                            editor_response = Some(response);
                        }
                    });
            });

        if self.text_box_focus_request == Some(annotation_index) {
            self.text_box_focus_request = None;
        }
        if self.editing_text_box == Some(annotation_index)
            && ctx.input(|input| input.key_pressed(egui::Key::Escape))
        {
            self.finish_text_box_edit();
        }

        let editor_active = editor_response.as_ref().is_some_and(|response| {
            response.has_focus() || response.hovered() || response.changed()
        });
        editor_active || area.response.hovered()
    }

    fn draw_text_box_action_palette(
        &mut self,
        ctx: &Context,
        placement: &PagePlacement,
        page_index: usize,
    ) -> bool {
        let Some(annotation_index) = self.selected_text_box else {
            return false;
        };
        let Some(annotation) = self.annotations.get(annotation_index) else {
            return false;
        };
        if annotation.page_index != page_index
            || !matches!(annotation.kind, AnnotationKind::TextBox { .. })
        {
            return false;
        }

        let screen_rect = placement.pdf_rect_to_screen(annotation.rect);
        let mut position = screen_rect.left_top() + Vec2::new(0.0, -34.0);
        if position.y < 8.0 {
            position = screen_rect.left_bottom() + Vec2::new(0.0, 8.0);
        }

        let editing = self.editing_text_box == Some(annotation_index);
        let mut edit_clicked = false;
        let mut done_clicked = false;
        let mut delete_clicked = false;
        let area = egui::Area::new(egui::Id::new(("text-box-actions", annotation_index)))
            .order(egui::Order::Foreground)
            .fixed_pos(position)
            .show(ctx, |ui| {
                egui::Frame::NONE
                    .fill(Color32::from_rgba_unmultiplied(250, 249, 245, 240))
                    .stroke(Stroke::new(1.0, Color32::from_rgb(205, 198, 184)))
                    .corner_radius(6)
                    .inner_margin(Margin::symmetric(6, 4))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            if editing {
                                if ui.button("Done").clicked() {
                                    done_clicked = true;
                                }
                            } else if ui.button("Edit").clicked() {
                                edit_clicked = true;
                            }
                            if ui.button("Delete").clicked() {
                                delete_clicked = true;
                            }
                        });
                    });
            });

        self.text_box_action_rect = Some(area.response.rect);
        if edit_clicked {
            self.start_text_box_edit(annotation_index);
        }
        if done_clicked {
            self.finish_text_box_edit();
        }
        if delete_clicked {
            self.delete_text_box(annotation_index);
            return true;
        }

        area.response.hovered() && ctx.input(|input| input.pointer.any_down())
    }

    fn draw_hovered_web_link(
        &self,
        painter: &egui::Painter,
        response: &egui::Response,
        placement: &PagePlacement,
        page_index: usize,
    ) {
        let Some(link) = self.hovered_web_link(response, placement, page_index) else {
            return;
        };
        let rect = placement
            .pdf_rect_to_screen(link.rect)
            .expand(2.0)
            .intersect(placement.rect);
        if rect.width() <= 0.0 || rect.height() <= 0.0 {
            return;
        }

        painter.rect_filled(rect, 2, Color32::from_rgba_unmultiplied(58, 112, 180, 24));
        painter.rect_stroke(
            rect,
            2,
            Stroke::new(1.0, Color32::from_rgb(58, 112, 180)),
            egui::StrokeKind::Inside,
        );
    }

    fn hovered_web_link_url(
        &self,
        response: &egui::Response,
        placement: &PagePlacement,
        page_index: usize,
    ) -> Option<String> {
        self.hovered_web_link(response, placement, page_index)
            .map(|link| link.url.clone())
    }

    fn clicked_web_link_url(
        &self,
        response: &egui::Response,
        placement: &PagePlacement,
        page_index: usize,
    ) -> Option<String> {
        if !response.clicked() || self.active_tool != Tool::Select {
            return None;
        }

        let pos = response.interact_pointer_pos().or(response.hover_pos())?;
        self.web_link_at_screen(page_index, pos, placement)
            .map(|link| link.url.clone())
    }

    fn hovered_web_link(
        &self,
        response: &egui::Response,
        placement: &PagePlacement,
        page_index: usize,
    ) -> Option<&PageLink> {
        if self.active_tool != Tool::Select {
            return None;
        }

        self.web_link_at_screen(page_index, response.hover_pos()?, placement)
    }

    fn web_link_at_screen(
        &self,
        page_index: usize,
        pos: Pos2,
        placement: &PagePlacement,
    ) -> Option<&PageLink> {
        let pdf = placement.screen_to_pdf(pos)?;
        self.web_link_at_pdf(page_index, pdf)
    }

    fn web_link_at_pdf(&self, page_index: usize, point: (f32, f32)) -> Option<&PageLink> {
        let document = self.document.as_ref()?;
        document
            .links
            .get(page_index)?
            .iter()
            .filter(|link| point_in_pdf_rect(point, expand_pdf_rect(link.rect, 2.0)))
            .min_by(|left, right| {
                let left_area = left.rect.width() * left.rect.height();
                let right_area = right.rect.width() * right.rect.height();
                left_area
                    .partial_cmp(&right_area)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    }

    fn handle_page_interaction(
        &mut self,
        ctx: &Context,
        response: &egui::Response,
        placement: &PagePlacement,
        page_index: usize,
        text_box_interacted: bool,
    ) {
        if text_box_interacted {
            return;
        }

        if self.active_tool == Tool::Select {
            if let Some(url) = self.clicked_web_link_url(response, placement, page_index) {
                ctx.open_url(egui::OpenUrl::new_tab(url.clone()));
                self.clear_text_selection();
                self.status = format!("Opened {url}");
                return;
            }
        }

        if response.clicked() && !self.click_hits_annotation_action(response) {
            self.clear_text_box_selection();
            self.clear_comment_selection();
        }

        if self.active_tool == Tool::Select || self.active_tool == Tool::Marker {
            self.handle_text_selection(response, placement, page_index);
            return;
        }

        if self.active_tool == Tool::TextBox && response.clicked() {
            if let Some(pos) = response.interact_pointer_pos() {
                if let Some(pdf) = placement.screen_to_pdf(pos) {
                    let rect = default_text_box_rect(*placement, pdf);
                    self.add_text_box_annotation(page_index, rect);
                }
            }
            return;
        }

        if response.drag_started() {
            if let Some(pos) = response.interact_pointer_pos() {
                if let Some(pdf) = placement.screen_to_pdf(pos) {
                    self.active_drag_page = Some(page_index);
                    match self.active_tool {
                        Tool::Signature => {
                            self.active_signature_stroke.clear();
                            self.active_signature_stroke.push(pdf);
                        }
                        Tool::TextBox => {
                            self.drag_start_pdf = Some(pdf);
                            self.drag_preview = Some(PdfRect::new(pdf.0, pdf.1, pdf.0, pdf.1));
                        }
                        Tool::Marker | Tool::Select => {}
                    }
                }
            }
        }

        if response.dragged() {
            if let Some(pos) = response.interact_pointer_pos() {
                if let Some(pdf) = placement.screen_to_pdf(pos) {
                    match self.active_tool {
                        Tool::Signature => {
                            let should_push = self
                                .active_signature_stroke
                                .last()
                                .map(|last| distance(*last, pdf) > 0.75)
                                .unwrap_or(true);
                            if should_push {
                                self.active_signature_stroke.push(pdf);
                            }
                        }
                        Tool::TextBox => {
                            if let Some(start) = self.drag_start_pdf {
                                self.drag_preview =
                                    Some(PdfRect::new(start.0, start.1, pdf.0, pdf.1));
                            }
                        }
                        Tool::Marker | Tool::Select => {}
                    }
                }
            }
        }

        if response.drag_stopped() {
            match self.active_tool {
                Tool::Marker => {}
                Tool::TextBox => {
                    if let Some(rect) = self.drag_preview.take() {
                        if rect.width() > 12.0 && rect.height() > 12.0 {
                            self.add_text_box_annotation(page_index, rect);
                        }
                    }
                    self.drag_start_pdf = None;
                }
                Tool::Signature => {
                    if self.active_signature_stroke.len() > 1 {
                        let stroke = std::mem::take(&mut self.active_signature_stroke);
                        let rect = rect_for_points(&stroke);
                        self.annotations.push(EditorAnnotation {
                            page_index,
                            rect,
                            kind: AnnotationKind::Signature {
                                signer: self.signer_name.trim().to_owned(),
                                signed_at: signature_timestamp(),
                                strokes: vec![stroke],
                            },
                        });
                        self.status = "Signature added.".to_owned();
                    } else {
                        self.active_signature_stroke.clear();
                    }
                }
                Tool::Select => {}
            }

            self.active_drag_page = None;
        }
    }

    fn handle_text_selection(
        &mut self,
        response: &egui::Response,
        placement: &PagePlacement,
        page_index: usize,
    ) {
        if response.clicked()
            && !self.click_hits_selection_or_toolbar(response, placement, page_index)
        {
            self.clear_text_selection();
            return;
        }

        if response.drag_started() {
            if let Some(pos) = response.interact_pointer_pos() {
                if let Some(pdf) = placement.screen_to_pdf(pos) {
                    if let Some(char_index) = self.text_char_index_at(page_index, pdf) {
                        self.selection_state.anchor = Some((page_index, char_index));
                        self.selection_state.text =
                            Some(TextSelection::new(page_index, char_index, char_index));
                    } else {
                        self.selection_state.anchor = None;
                        self.selection_state.text = None;
                    }
                }
            }
        }

        if response.dragged() {
            let Some((anchor_page, anchor_index)) = self.selection_state.anchor else {
                return;
            };
            if anchor_page != page_index {
                return;
            }

            if let Some(pos) = response.interact_pointer_pos() {
                if let Some(pdf) = placement.screen_to_pdf(pos) {
                    if let Some(char_index) = self.text_char_index_at(page_index, pdf) {
                        self.selection_state.text =
                            Some(TextSelection::new(page_index, anchor_index, char_index));
                    }
                }
            }
        }

        if response.drag_stopped() {
            self.selection_state.anchor = None;
            if self.active_tool == Tool::Marker {
                if self.selected_text().is_some() {
                    self.mark_selection(self.marker_preset());
                }
            } else if let Some(text) = self.selected_text() {
                self.status = format!("Selected {} character(s)", text.chars().count());
            }
        }
    }

    fn draw_page_context_menu(
        &mut self,
        ctx: &Context,
        response: &egui::Response,
        placement: &PagePlacement,
        page_index: usize,
    ) {
        let has_selection = self
            .selection_state
            .text
            .is_some_and(|selection| selection.contains(page_index));

        // Remember where the menu was opened: `interact_pointer_pos` is empty
        // once the pointer moves onto the menu itself, so capture it on the
        // right-click frame and reuse it when "Add comment" is finally clicked.
        if response.secondary_clicked() {
            self.context_menu_pdf = response
                .interact_pointer_pos()
                .or(response.hover_pos())
                .and_then(|pos| placement.screen_to_pdf(pos))
                .map(|pdf| (page_index, pdf));
        }
        let menu_pdf_pos = self
            .context_menu_pdf
            .filter(|(page, _)| *page == page_index)
            .map(|(_, pdf)| pdf);

        response.context_menu(|ui| {
            if ui
                .add_enabled(has_selection, egui::Button::new("Copy"))
                .clicked()
            {
                self.copy_selection(ctx);
                ui.close();
            }
            if ui
                .add_enabled(has_selection, egui::Button::new("Highlight selection"))
                .clicked()
            {
                self.mark_selection(self.marker_preset());
                ui.close();
            }
            if ui
                .add_enabled(menu_pdf_pos.is_some(), egui::Button::new("Add comment"))
                .clicked()
            {
                if let Some(pdf) = menu_pdf_pos {
                    self.add_comment_annotation(
                        ctx,
                        page_index,
                        pdf,
                        placement.page_width,
                        placement.page_height,
                    );
                }
                ui.close();
            }
        });
    }

    fn draw_selection_action_palette(
        &mut self,
        ctx: &Context,
        placement: &PagePlacement,
        page_index: usize,
    ) {
        let Some(selection) = self.selection_state.text else {
            self.selection_state.toolbar_rect = None;
            return;
        };
        if !selection.contains(page_index)
            || page_index != selection.action_page()
            || self.selected_text().is_none()
        {
            return;
        }
        if self.selection_state.anchor.is_some() {
            self.selection_state.toolbar_rect = None;
            return;
        }
        if self.active_tool == Tool::Marker {
            self.selection_state.toolbar_rect = None;
            return;
        }

        let Some(selection_rect) = self.selection_screen_rect(placement, page_index) else {
            return;
        };

        let mut position = selection_rect.left_top() + Vec2::new(0.0, -34.0);
        if position.y < 8.0 {
            position = selection_rect.left_bottom() + Vec2::new(0.0, 8.0);
        }

        let inner = egui::Area::new(egui::Id::new(("selection-actions", page_index)))
            .order(egui::Order::Foreground)
            .fixed_pos(position)
            .show(ctx, |ui| {
                egui::Frame::NONE
                    .fill(Color32::from_rgba_unmultiplied(250, 249, 245, 238))
                    .stroke(Stroke::new(1.0, Color32::from_rgb(205, 198, 184)))
                    .corner_radius(6)
                    .inner_margin(Margin::symmetric(6, 4))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            if ui.button("Copy").clicked() {
                                self.copy_selection(ctx);
                            }
                            if ui.button("Highlight").clicked() {
                                self.mark_selection(self.marker_preset());
                            }
                        });
                    });
            });

        self.selection_state.toolbar_rect = Some(inner.response.rect);
    }

    fn copy_selection(&mut self, ctx: &Context) {
        let Some(text) = self.selected_text() else {
            self.status = "No selected text to copy.".to_owned();
            return;
        };

        self.copy_text_to_clipboard(ctx, text);
    }

    fn copy_liquid_selection(&mut self, ctx: &Context) {
        let Some(text) = self.current_liquid_copy_text() else {
            self.selection_state.liquid_all = false;
            self.status = "No Liquid text to copy.".to_owned();
            return;
        };

        self.selection_state.liquid_all = true;
        self.copy_text_to_clipboard(ctx, text);
    }

    fn copy_text_to_clipboard(&mut self, ctx: &Context, text: String) {
        ctx.copy_text(text.clone());
        let chars = text.chars().count();
        self.status =
            match arboard::Clipboard::new().and_then(|mut clipboard| clipboard.set_text(text)) {
                Ok(()) => copy_status_message(chars, None),
                Err(error) => {
                    let error = error.to_string();
                    eprintln!("Copy failed: {error}");
                    copy_status_message(chars, Some(&error))
                }
            };
    }

    fn select_all_current_view_text(&mut self, ctx: &Context) {
        match self.view_mode {
            DocumentViewMode::Pdf => self.select_all_text(ctx),
            DocumentViewMode::Liquid | DocumentViewMode::LiquidMode2 => {
                self.select_all_liquid_text()
            }
        }
    }

    fn select_all_liquid_text(&mut self) {
        let Some(text) = self.current_liquid_copy_text() else {
            self.clear_text_selection();
            self.status = match self.view_mode {
                DocumentViewMode::LiquidMode2 => "No Review Mode text ready to select.".to_owned(),
                DocumentViewMode::Liquid => "No Liquid text ready to select.".to_owned(),
                DocumentViewMode::Pdf => "No selectable PDF text found.".to_owned(),
            };
            return;
        };

        let chars = text.chars().count();
        self.clear_text_selection();
        self.clear_text_box_selection();
        self.selection_state.liquid_all = true;
        self.status = match self.view_mode {
            DocumentViewMode::LiquidMode2 => {
                format!("Selected all Review Mode text ({chars} character(s))")
            }
            DocumentViewMode::Liquid => format!("Selected all Liquid text ({chars} character(s))"),
            DocumentViewMode::Pdf => format!("Selected all text ({chars} character(s))"),
        };
    }

    fn current_liquid_copy_text(&self) -> Option<String> {
        let document = match self.view_mode {
            DocumentViewMode::Liquid => match &self.liquid_state {
                LiquidState::Ready(document) => document,
                _ => return None,
            },
            DocumentViewMode::LiquidMode2 => match &self.liquid_mode2_state {
                LiquidState::Ready(document) => document,
                _ => return None,
            },
            DocumentViewMode::Pdf => return None,
        };
        liquid_document_copy_text(document)
    }

    fn select_all_text(&mut self, ctx: &Context) {
        if self.document.is_none() {
            return;
        }

        self.clear_text_box_selection();
        self.selection_state.text = None;
        self.selection_state.liquid_all = false;
        self.selection_state.anchor = None;
        self.selection_state.toolbar_rect = None;
        self.selection_state.pending_select_all = true;
        self.finish_pending_select_all_text(ctx);
    }

    fn finish_pending_select_all_text(&mut self, ctx: &Context) {
        if !self.selection_state.pending_select_all {
            return;
        }

        let Some((path, page_count)) = self
            .document
            .as_ref()
            .map(|document| (document.path.clone(), document.page_count))
        else {
            self.selection_state.pending_select_all = false;
            return;
        };

        let missing_pages = self
            .document
            .as_ref()
            .map(|document| {
                (0..page_count)
                    .filter(|page_index| {
                        document
                            .text_chars
                            .get(*page_index)
                            .is_some_and(Option::is_none)
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        for page_index in &missing_pages {
            self.enqueue_text_chars(&path, *page_index);
        }

        if !missing_pages.is_empty() {
            self.status = format!(
                "Preparing selectable text for {} page(s)...",
                missing_pages.len()
            );
            ctx.request_repaint_after(RENDER_POLL_INTERVAL);
            return;
        }

        let Some(document) = self.document.as_ref() else {
            self.selection_state.pending_select_all = false;
            return;
        };

        let mut first_page = None;
        let mut last_page = None;
        let mut character_count = 0usize;
        for (page_index, chars) in document
            .text_chars
            .iter()
            .enumerate()
            .filter_map(|(page_index, chars)| chars.as_ref().map(|chars| (page_index, chars)))
        {
            if chars.is_empty() {
                continue;
            }
            first_page.get_or_insert((page_index, chars.len()));
            last_page = Some((page_index, chars.len()));
            character_count += chars.len();
        }

        let (Some((start_page, _)), Some((end_page, end_len))) = (first_page, last_page) else {
            self.selection_state.pending_select_all = false;
            self.status = "No selectable PDF text found.".to_owned();
            return;
        };

        self.selection_state.text = Some(TextSelection::range(
            start_page,
            0,
            end_page,
            end_len.saturating_sub(1),
        ));
        self.selection_state.anchor = None;
        self.selection_state.toolbar_rect = None;
        self.selection_state.pending_select_all = false;
        self.status = format!("Selected all text ({character_count} character(s))");
        ctx.request_repaint();
    }

    fn draw_marker_default_palette(&mut self, ui: &mut egui::Ui) {
        for (index, preset) in MARKER_PRESETS.iter().copied().enumerate() {
            let color = color_from_rgb(preset.color_rgb, 210);
            let selected = self.marker_preset_index == index;
            let stroke = if selected {
                Stroke::new(1.6, Color32::from_rgb(88, 66, 38))
            } else {
                Stroke::new(1.0, Color32::from_rgb(210, 202, 188))
            };
            let button = match preset.style {
                MarkerStyle::Highlight => egui::Button::new("")
                    .fill(color)
                    .stroke(stroke)
                    .min_size(Vec2::splat(18.0)),
                MarkerStyle::Underline => egui::Button::new(
                    RichText::new("U")
                        .strong()
                        .color(color_from_rgb(preset.color_rgb, 255)),
                )
                .fill(Color32::from_rgb(255, 252, 246))
                .stroke(stroke)
                .min_size(Vec2::new(24.0, 18.0)),
            };
            if ui.add(button).on_hover_text(preset.label).clicked() {
                self.marker_preset_index = index;
                self.status = format!("Marker set to {}", preset.label);
            }
        }
    }

    fn mark_selection(&mut self, preset: MarkerPreset) {
        let Some(selection) = self.selection_state.text else {
            return;
        };

        let animate = preset.style == MarkerStyle::Highlight && !self.settings.reduce_motion;
        let born = Instant::now();
        let mut count = 0u32;
        for page_index in selection.page_range() {
            for rect in self.selection_rects_for_page(page_index) {
                self.annotations.push(EditorAnnotation {
                    page_index,
                    rect,
                    kind: AnnotationKind::Marker {
                        color_rgb: preset.color_rgb,
                        opacity: self.marker_opacity_for(preset),
                        style: preset.style,
                    },
                });
                if animate {
                    // Each line segment lands a beat after the previous one, so a
                    // multi-line selection fills like a hand sweeping down the page.
                    self.marker_animations.push(MarkerAnim {
                        page_index,
                        rect,
                        born,
                        delay: MARKER_STAGGER * count,
                    });
                }
                count += 1;
            }
        }

        self.status = match preset.style {
            MarkerStyle::Highlight => {
                format!("Highlighted selected text ({count} line segment(s))")
            }
            MarkerStyle::Underline => format!("Underlined selected text ({count} line segment(s))"),
        };
        if count > 0 {
            self.annotations_dirty = true;
        }
        self.clear_text_selection();
    }

    fn clear_text_selection(&mut self) {
        self.selection_state = SelectionState::default();
    }

    fn selected_text(&self) -> Option<String> {
        let document = self.document.as_ref()?;
        let selection = self.selection_state.text?;

        let mut text = String::new();
        for page_index in selection.page_range() {
            let Some(chars) = document.text_chars.get(page_index).and_then(Option::as_ref) else {
                return None;
            };
            let Some((start, end)) = selection.bounds_for_page(page_index, chars.len()) else {
                continue;
            };
            if !text.is_empty() {
                text.push('\n');
            }
            text.extend(chars[start..=end].iter().map(|char| char.ch));
        }

        (!text.is_empty()).then_some(text)
    }

    fn selection_rects_for_page(&self, page_index: usize) -> Vec<PdfRect> {
        let Some(document) = self.document.as_ref() else {
            return Vec::new();
        };
        let Some(selection) = self.selection_state.text else {
            return Vec::new();
        };
        if !selection.contains(page_index) {
            return Vec::new();
        }
        let Some(chars) = document.text_chars.get(page_index).and_then(Option::as_ref) else {
            return Vec::new();
        };
        if chars.is_empty() {
            return Vec::new();
        }

        let Some((start, end)) = selection.bounds_for_page(page_index, chars.len()) else {
            return Vec::new();
        };
        merge_text_rects(
            chars[start..=end]
                .iter()
                .filter(|char| !char.ch.is_control())
                .filter_map(|char| char.rect),
        )
    }

    fn selection_screen_rect(&self, placement: &PagePlacement, page_index: usize) -> Option<Rect> {
        let mut rects = self
            .selection_rects_for_page(page_index)
            .into_iter()
            .map(|pdf_rect| placement.pdf_rect_to_screen(pdf_rect));
        let first = rects.next()?;
        Some(rects.fold(first, |acc, rect| acc.union(rect)))
    }

    fn click_hits_selection_or_toolbar(
        &self,
        response: &egui::Response,
        placement: &PagePlacement,
        page_index: usize,
    ) -> bool {
        let Some(pos) = response.interact_pointer_pos() else {
            return false;
        };

        if self
            .selection_state
            .toolbar_rect
            .is_some_and(|rect| rect.expand(4.0).contains(pos))
        {
            return true;
        }

        self.selection_rects_for_page(page_index)
            .into_iter()
            .map(|pdf_rect| placement.pdf_rect_to_screen(pdf_rect).expand(2.0))
            .any(|rect| rect.contains(pos))
    }

    fn click_hits_annotation_action(&self, response: &egui::Response) -> bool {
        let Some(pos) = response.interact_pointer_pos() else {
            return false;
        };

        self.text_box_action_rect
            .is_some_and(|rect| rect.expand(4.0).contains(pos))
            || self
                .comment_action_rect
                .is_some_and(|rect| rect.expand(4.0).contains(pos))
    }

    fn comment_ordinal(&self, annotation_index: usize) -> usize {
        self.annotations
            .iter()
            .take(annotation_index + 1)
            .filter(|annotation| matches!(annotation.kind, AnnotationKind::Comment { .. }))
            .count()
            .max(1)
    }

    fn text_char_index_at(&mut self, page_index: usize, point: (f32, f32)) -> Option<usize> {
        let Some(chars) = self.ensure_text_chars(page_index) else {
            self.status = "Preparing selectable text for this page.".to_owned();
            return None;
        };

        let mut best: Option<(usize, f32)> = None;
        for (index, text_char) in chars.iter().enumerate() {
            let Some(rect) = text_char.rect else {
                continue;
            };
            if text_char.ch.is_control() {
                continue;
            }

            let expanded = expand_pdf_rect(rect, 2.5);
            if point_in_pdf_rect(point, expanded) {
                return Some(index);
            }

            let distance = distance_to_pdf_rect(point, rect);
            if distance < 10.0 && best.is_none_or(|(_, best_distance)| distance < best_distance) {
                best = Some((index, distance));
            }
        }

        best.map(|(index, _)| index)
    }
}

impl eframe::App for PdfEditorApp {
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.stop_liquid_tts();
    }

    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        if ctx.input(|input| input.viewport().close_requested()) && !self.allow_window_close {
            self.save_active_tab_state();
            if self.has_unsaved_annotations() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.show_unsaved_close_prompt = true;
            }
        }
        if consume_command_shortcut(ctx, egui::Key::W) {
            if let Some(active_tab) = self.active_tab {
                self.close_tab(active_tab, ctx);
            }
        }

        self.poll_incoming_paths(ctx);
        self.poll_queued_open_paths(ctx);
        self.poll_render_results(ctx);
        self.start_due_comment_saves(ctx);
        self.finish_pending_select_all_text(ctx);
        self.poll_ocr(ctx);
        self.poll_chat_results(ctx);
        self.poll_liquid_results(ctx);
        self.poll_liquid_mode2_results(ctx);
        self.poll_paid_tts(ctx);
        self.poll_update_events(ctx);
        self.handle_dropped_files(ctx);

        let zoom_delta = ctx.input(|input| input.zoom_delta());
        let pdf_ctrl_wheel_zoom = self.view_mode == DocumentViewMode::Pdf
            && ctx.input(|input| input.modifiers.ctrl)
            && ctx.input(|input| input.pointer.hover_pos().is_some());
        if (zoom_delta - 1.0).abs() > f32::EPSILON && !pdf_ctrl_wheel_zoom {
            self.set_zoom(self.target_zoom * zoom_delta);
            self.status = format!("{:.0}% zoom", self.target_zoom * 100.0);
            ctx.request_repaint_after(ZOOM_RENDER_DEBOUNCE);
        }

        if consume_command_shortcut(ctx, egui::Key::O) {
            self.open_dialog(ctx);
        }
        if consume_command_shortcut(ctx, egui::Key::S)
            && let Err(error) = self.save_current_annotations()
        {
            self.push_error_notice(error);
        }
        if consume_command_shortcut(ctx, egui::Key::F) {
            self.focus_search(ctx);
        }
        if consume_command_shortcut(ctx, egui::Key::Tab) && self.tabs.len() > 1 {
            let active = self.active_tab.unwrap_or(0);
            let next = if ctx.input(|input| input.modifiers.shift) {
                active.checked_sub(1).unwrap_or(self.tabs.len() - 1)
            } else {
                (active + 1) % self.tabs.len()
            };
            self.switch_to_tab(next, ctx);
        }
        if consume_command_shortcut(ctx, egui::Key::Plus)
            || consume_command_shortcut(ctx, egui::Key::Equals)
        {
            self.set_zoom(self.target_zoom * 1.15);
        }
        if consume_command_shortcut(ctx, egui::Key::Minus) {
            self.set_zoom(self.target_zoom / 1.15);
        }
        let wants_keyboard_input = ctx.wants_keyboard_input();
        let focused_text_edit = focused_widget_is_text_edit(ctx);
        let liquid_shortcuts_allowed =
            liquid_document_shortcuts_allowed(self.view_mode, focused_text_edit);
        if liquid_shortcuts_allowed && consume_command_shortcut_or_key_event(ctx, egui::Key::A) {
            surrender_focused_non_text_edit(ctx);
            self.select_all_current_view_text(ctx);
        }
        let document_shortcuts_allowed =
            document_shortcuts_allowed(self.view_mode, wants_keyboard_input, focused_text_edit);
        if !liquid_shortcuts_allowed
            && document_shortcuts_allowed
            && consume_command_shortcut(ctx, egui::Key::A)
        {
            self.select_all_current_view_text(ctx);
        }
        let copy_requested = copy_shortcut_requested(ctx);
        if document_shortcuts_allowed || liquid_shortcuts_allowed {
            match self.view_mode {
                DocumentViewMode::Pdf => {
                    if should_copy_pdf_selection_on_shortcut(
                        self.selection_state.text.is_some(),
                        copy_requested,
                    ) {
                        consume_copy_shortcut(ctx);
                        self.copy_selection(ctx);
                    }
                }
                DocumentViewMode::Liquid | DocumentViewMode::LiquidMode2 => {
                    if should_copy_liquid_selection_on_shortcut(
                        self.selection_state.liquid_all,
                        self.current_liquid_copy_text().is_some(),
                        copy_requested,
                    ) {
                        consume_copy_shortcut(ctx);
                        surrender_focused_non_text_edit(ctx);
                        self.copy_liquid_selection(ctx);
                    }
                }
            }
        }
        if self.editing_text_box.is_some()
            && ctx.input(|input| input.key_pressed(egui::Key::Escape))
        {
            self.finish_text_box_edit();
        }
        if self.selected_text_box.is_some()
            && self.editing_text_box.is_none()
            && !ctx.wants_keyboard_input()
            && ctx.input(|input| {
                input.key_pressed(egui::Key::Delete) || input.key_pressed(egui::Key::Backspace)
            })
        {
            self.delete_selected_text_box();
        }
        if let Some(comment_index) = self.selected_comment {
            if self.editing_comment.is_none()
                && !ctx.wants_keyboard_input()
                && ctx.input(|input| {
                    input.key_pressed(egui::Key::Delete) || input.key_pressed(egui::Key::Backspace)
                })
            {
                self.delete_comment(ctx, comment_index);
            }
        }

        if !ctx.wants_keyboard_input() {
            if ctx.input(|input| input.key_pressed(egui::Key::V)) {
                self.active_tool = Tool::Select;
            }
            if ctx.input(|input| input.key_pressed(egui::Key::M)) {
                self.active_tool = Tool::Marker;
            }
            if ctx.input(|input| input.key_pressed(egui::Key::T)) {
                self.active_tool = Tool::TextBox;
            }
            if ctx.input(|input| input.key_pressed(egui::Key::S) && !input.modifiers.ctrl) {
                self.active_tool = Tool::Signature;
            }
        }

        self.advance_zoom_animation(ctx);

        self.draw_toolbar(ctx);
        self.draw_side_panel(ctx);
        self.draw_status_bar(ctx);
        self.draw_document(ctx);
        self.draw_settings_window(ctx);
        self.draw_unsaved_close_prompt(ctx);
        self.draw_update_notice(ctx);
        self.draw_notices(ctx);

        if self.zoom_is_animating()
            || !self.pending_page_renders.is_empty()
            || !self.pending_thumbnail_renders.is_empty()
            || !self.pending_native_text.is_empty()
            || !self.pending_text_chars.is_empty()
            || self.selection_state.pending_select_all
            || !self.pending_comment_saves.is_empty()
            || !self.active_comment_saves.is_empty()
            || !self.queued_open_paths.is_empty()
            || self.update_ui.state.is_busy()
            || self.chat_ui.state.in_flight
            || self.ocr_is_active()
            || matches!(
                self.liquid_state,
                LiquidState::PreparingText | LiquidState::Preparing
            )
            || matches!(
                self.liquid_mode2_state,
                LiquidState::PreparingText | LiquidState::Preparing
            )
        {
            ctx.request_repaint_after(RENDER_POLL_INTERVAL);
        }
    }
}

#[cfg(target_os = "windows")]
fn open_windows_default_pdf_settings() -> Result<(), String> {
    const DEFAULT_APPS_URI: &str = "ms-settings:defaultapps?registeredAppMachine=LawPDF";

    std::process::Command::new("explorer.exe")
        .arg(DEFAULT_APPS_URI)
        .spawn()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn toolbar_group(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::NONE
        .fill(Color32::from_rgb(244, 241, 235))
        .stroke(Stroke::new(1.0, Color32::from_rgb(224, 219, 209)))
        .corner_radius(6)
        .inner_margin(Margin::symmetric(6, 4))
        .show(ui, |ui| {
            ui.horizontal(add_contents);
        });
}

fn consume_command_shortcut(ctx: &Context, key: egui::Key) -> bool {
    ctx.input_mut(|input| {
        input_has_command_shortcut(input, key) || consume_command_key_event(input, key)
    })
}

fn consume_command_shortcut_or_key_event(ctx: &Context, key: egui::Key) -> bool {
    ctx.input_mut(|input| {
        input_has_command_shortcut(input, key) || consume_command_key_event(input, key)
    })
}

fn input_has_command_shortcut(input: &mut egui::InputState, key: egui::Key) -> bool {
    input.consume_shortcut(&command_shortcut(key))
}

fn input_has_command_shortcut_event(input: &egui::InputState, key: egui::Key) -> bool {
    let shortcut = command_shortcut(key);
    input.events.iter().any(|event| {
        matches!(
            event,
            egui::Event::Key {
                key: event_key,
                modifiers,
                pressed: true,
                ..
            } if *event_key == shortcut.logical_key
                && command_modifiers_match(*modifiers, shortcut.modifiers)
        )
    })
}

fn command_shortcut(key: egui::Key) -> egui::KeyboardShortcut {
    egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, key)
}

fn copy_shortcut_requested(ctx: &Context) -> bool {
    ctx.input(|input| {
        input_has_copy_event(input) || input_has_command_shortcut_event(input, egui::Key::C)
    })
}

fn consume_copy_shortcut(ctx: &Context) -> bool {
    ctx.input_mut(|input| {
        let mut consumed = false;
        input.events.retain(|event| {
            let is_copy = matches!(event, egui::Event::Copy);
            consumed |= is_copy;
            !is_copy
        });
        input_has_command_shortcut(input, egui::Key::C)
            || consume_command_key_event(input, egui::Key::C)
            || consumed
    })
}

fn consume_command_key_event(input: &mut egui::InputState, key: egui::Key) -> bool {
    let shortcut = command_shortcut(key);
    let mut consumed = false;
    input.events.retain(|event| {
        let is_shortcut = matches!(
            event,
            egui::Event::Key {
                key: event_key,
                modifiers,
                pressed: true,
                ..
            } if *event_key == shortcut.logical_key
                && command_modifiers_match(*modifiers, shortcut.modifiers)
        );
        consumed |= is_shortcut;
        !is_shortcut
    });
    consumed
}

fn command_modifiers_match(
    modifiers: egui::Modifiers,
    shortcut_modifiers: egui::Modifiers,
) -> bool {
    modifiers.matches_logically(shortcut_modifiers)
        || (shortcut_modifiers == egui::Modifiers::COMMAND && modifiers.mac_cmd)
}

fn focused_widget_is_text_edit(ctx: &Context) -> bool {
    let focused = ctx.memory(|memory| memory.focused());
    focused.is_some_and(|id| egui::TextEdit::load_state(ctx, id).is_some())
}

fn surrender_focused_non_text_edit(ctx: &Context) {
    if focused_widget_is_text_edit(ctx) {
        return;
    }
    if let Some(id) = ctx.memory(|memory| memory.focused()) {
        ctx.memory_mut(|memory| memory.surrender_focus(id));
    }
}

fn document_shortcuts_allowed(
    view_mode: DocumentViewMode,
    wants_keyboard_input: bool,
    focused_text_edit: bool,
) -> bool {
    !wants_keyboard_input
        || (matches!(
            view_mode,
            DocumentViewMode::Liquid | DocumentViewMode::LiquidMode2
        ) && !focused_text_edit)
}

fn liquid_document_shortcuts_allowed(view_mode: DocumentViewMode, focused_text_edit: bool) -> bool {
    matches!(
        view_mode,
        DocumentViewMode::Liquid | DocumentViewMode::LiquidMode2
    ) && !focused_text_edit
}

fn should_copy_pdf_selection_on_shortcut(has_pdf_selection: bool, copy_requested: bool) -> bool {
    has_pdf_selection && copy_requested
}

fn should_copy_liquid_selection_on_shortcut(
    has_liquid_selection: bool,
    has_liquid_copy_text: bool,
    copy_requested: bool,
) -> bool {
    (has_liquid_selection || has_liquid_copy_text) && copy_requested
}

fn input_has_copy_event(input: &egui::InputState) -> bool {
    input
        .events
        .iter()
        .any(|event| matches!(event, egui::Event::Copy))
}

fn copy_status_message(chars: usize, error: Option<&str>) -> String {
    match error {
        Some(error) => format!("Copy failed: {error}"),
        None => format!("Copied {chars} character(s)"),
    }
}

fn human_duration(seconds: f64) -> String {
    if !seconds.is_finite() || seconds <= 0.0 {
        return "less than 1s".to_owned();
    }

    let rounded = seconds.round() as u64;
    let minutes = rounded / 60;
    let seconds = rounded % 60;
    if minutes >= 60 {
        let hours = minutes / 60;
        let minutes = minutes % 60;
        format!("{hours}h {minutes}m")
    } else if minutes > 0 {
        format!("{minutes}m {seconds}s")
    } else {
        format!("{seconds}s")
    }
}

fn text_preview(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }

    let mut preview = text.chars().take(max_chars).collect::<String>();
    preview.push_str("...");
    preview
}

fn comment_preview(text: &str) -> Option<String> {
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.is_empty() {
        None
    } else if compact.chars().count() > 44 {
        Some(format!(
            "{}...",
            compact.chars().take(41).collect::<String>()
        ))
    } else {
        Some(compact)
    }
}

fn new_comment_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{LAWPDF_COMMENT_ID_PREFIX}{}-{nanos}", std::process::id())
}

fn comment_timestamp() -> String {
    signature_timestamp()
}

fn document_opened_status(
    title: &str,
    cached_ocr_pages: usize,
    comment_count: usize,
    liquid_feedback_count: usize,
) -> String {
    let mut restored = Vec::new();
    if cached_ocr_pages > 0 {
        restored.push(format!("OCR for {cached_ocr_pages} page(s)"));
    }
    if comment_count > 0 {
        restored.push(format!("{comment_count} comment(s)"));
    }
    if liquid_feedback_count > 0 {
        restored.push(format!("{liquid_feedback_count} Liquid annotation(s)"));
    }
    if restored.is_empty() {
        format!("Opened {title}")
    } else {
        format!("Opened {title}; restored {}.", restored.join(", "))
    }
}

fn effective_deep_liquid_config(_settings: &AppSettings) -> Option<DeepLiquidConfig> {
    let enabled = std::env::var("LAWPDF_DEEP_LIQUID")
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false);
    if !enabled {
        return None;
    }

    let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let python_exe = std::env::var_os("LAWPDF_DEEP_LIQUID_PYTHON")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("python"));
    let script_path = std::env::var_os("LAWPDF_DEEP_LIQUID_SCRIPT")
        .map(PathBuf::from)
        .unwrap_or_else(|| current_dir.join("tools").join("liquid_deep_infer.py"));
    let model_dir = std::env::var_os("LAWPDF_DEEP_LIQUID_MODEL_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            let promoted = current_dir
                .join("profile-models")
                .join("deep-liquid-current");
            promoted.exists().then_some(promoted)
        });
    let model_id = std::env::var("LAWPDF_DEEP_LIQUID_MODEL_ID")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "lawreview-liquidnet-span-baseline-v0".to_owned());
    let timeout_secs = std::env::var("LAWPDF_DEEP_LIQUID_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(180);

    Some(DeepLiquidConfig {
        python_exe,
        script_path,
        model_dir,
        model_id,
        timeout_secs,
    })
}

fn liquid_feedback_id(source_signature: &str, block_index: usize, block: &LiquidBlock) -> String {
    let mut hasher = DefaultHasher::new();
    source_signature.hash(&mut hasher);
    block_index.hash(&mut hasher);
    block.role.prompt_name().hash(&mut hasher);
    block.text.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn liquid_block_source_lines(
    document: &LiquidDocument,
    block_index: usize,
) -> Vec<LiquidSourceLineRef> {
    document
        .block_source_lines
        .iter()
        .find(|sources| sources.block_index == block_index)
        .map(|sources| sources.lines.clone())
        .unwrap_or_default()
}

#[derive(Debug, Clone, Serialize)]
struct ReaderCorrectionRecord {
    path: String,
    page_index: usize,
    line_index: usize,
    text: String,
    gold_role: String,
    source_role: String,
    action: String,
    origin: String,
    ts: String,
}

#[derive(Debug, Clone, Serialize)]
struct ReaderEventRecord {
    path: String,
    event: String,
    detail: String,
    origin: String,
    ts: String,
}

fn reader_corrections_path() -> Option<PathBuf> {
    app_data_dir().map(|dir| dir.join("liquid-feedback").join("reader-corrections.jsonl"))
}

fn reader_events_path() -> Option<PathBuf> {
    app_data_dir().map(|dir| dir.join("liquid-feedback").join("reader-events.jsonl"))
}

/// #30: append per-source-line reader corrections (human-gold audit schema:
/// path/page_index/line_index/text/gold_role, plus source_role/action/origin/ts) to a local
/// append-only JSONL log so in-app corrections feed the label pipeline. Returns lines written.
fn append_reader_corrections(
    document: &LoadedDocument,
    source_lines: &[LiquidSourceLineRef],
    gold_role: &str,
    action: &str,
    ts: &str,
) -> Result<usize, String> {
    if source_lines.is_empty() {
        return Ok(0);
    }
    let path = reader_corrections_path()
        .ok_or_else(|| "Could not find reader corrections directory.".to_owned())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create corrections folder: {error}"))?;
    }
    let doc_path = document.path.to_string_lossy().into_owned();
    let mut buf = String::new();
    for line in source_lines {
        let record = ReaderCorrectionRecord {
            path: doc_path.clone(),
            page_index: line.page_index,
            line_index: line.line_index,
            text: line.text.clone(),
            gold_role: gold_role.to_owned(),
            source_role: line.role.prompt_name().to_owned(),
            action: action.to_owned(),
            origin: "reader_correction".to_owned(),
            ts: ts.to_owned(),
        };
        let encoded = serde_json::to_string(&record)
            .map_err(|error| format!("Could not encode correction: {error}"))?;
        buf.push_str(&encoded);
        buf.push('\n');
    }
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|error| format!("Could not open corrections log: {error}"))?;
    file.write_all(buf.as_bytes())
        .map_err(|error| format!("Could not write corrections log: {error}"))?;
    Ok(source_lines.len())
}

/// #30: append a doc-level reader event (e.g. reflow_rejected) to a local append-only JSONL log.
fn append_reader_event(
    document: &LoadedDocument,
    event: &str,
    detail: &str,
    ts: &str,
) -> Result<(), String> {
    let path =
        reader_events_path().ok_or_else(|| "Could not find reader events directory.".to_owned())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create events folder: {error}"))?;
    }
    let record = ReaderEventRecord {
        path: document.path.to_string_lossy().into_owned(),
        event: event.to_owned(),
        detail: detail.to_owned(),
        origin: "reader_event".to_owned(),
        ts: ts.to_owned(),
    };
    let encoded = serde_json::to_string(&record)
        .map_err(|error| format!("Could not encode event: {error}"))?;
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|error| format!("Could not open events log: {error}"))?;
    writeln!(file, "{encoded}").map_err(|error| format!("Could not write events log: {error}"))
}

fn liquid_feedback_path(document_path: &Path) -> Option<PathBuf> {
    let mut hasher = DefaultHasher::new();
    document_path.to_string_lossy().hash(&mut hasher);
    app_data_dir().map(|dir| {
        dir.join("liquid-feedback")
            .join(format!("{:016x}.json", hasher.finish()))
    })
}

fn load_liquid_feedback(document_path: &Path) -> Result<Vec<LiquidFeedback>, String> {
    let Some(path) = liquid_feedback_path(document_path) else {
        return Ok(Vec::new());
    };
    if !path.exists() {
        return Ok(Vec::new());
    }
    let bytes =
        std::fs::read(&path).map_err(|error| format!("Could not read Liquid feedback: {error}"))?;
    let file: LiquidFeedbackFile = serde_json::from_slice(&bytes)
        .map_err(|error| format!("Could not decode Liquid feedback: {error}"))?;
    Ok(file.entries)
}

fn save_liquid_feedback(
    document: &LoadedDocument,
    entries: &[LiquidFeedback],
) -> Result<(), String> {
    let path = liquid_feedback_path(&document.path)
        .ok_or_else(|| "Could not find Liquid feedback directory.".to_owned())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create Liquid feedback folder: {error}"))?;
    }
    let file = LiquidFeedbackFile {
        document_path: document.path.clone(),
        document_title: document.title.clone(),
        entries: entries.to_vec(),
    };
    let bytes = serde_json::to_vec_pretty(&file)
        .map_err(|error| format!("Could not encode Liquid feedback: {error}"))?;
    std::fs::write(&path, bytes).map_err(|error| format!("Could not save Liquid feedback: {error}"))
}

fn save_liquid_retrain_queue(
    entries: &[LiquidFeedback],
    created_at: &str,
) -> Result<PathBuf, String> {
    let dir = app_data_dir()
        .ok_or_else(|| "Could not find Liquid feedback directory.".to_owned())?
        .join("liquid-feedback")
        .join("retrain-queue");
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("Could not create Liquid retrain queue: {error}"))?;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = dir.join(format!("liquid-feedback-{nanos}.json"));
    let queue = LiquidRetrainQueue {
        created_at: created_at.to_owned(),
        entries: entries.to_vec(),
    };
    let bytes = serde_json::to_vec_pretty(&queue)
        .map_err(|error| format!("Could not encode Liquid retrain queue: {error}"))?;
    std::fs::write(&path, bytes)
        .map_err(|error| format!("Could not save Liquid retrain queue: {error}"))?;
    Ok(path)
}

#[cfg(test)]
fn comment_annotations_for_save_from(annotations: &[EditorAnnotation]) -> Vec<EditorAnnotation> {
    annotations
        .iter()
        .filter(|annotation| matches!(annotation.kind, AnnotationKind::Comment { .. }))
        .cloned()
        .collect()
}

fn lerp_color(from: Color32, to: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let lerp_channel = |from: u8, to: u8| {
        (from as f32 + (to as f32 - from as f32) * t)
            .round()
            .clamp(0.0, 255.0) as u8
    };

    Color32::from_rgba_unmultiplied(
        lerp_channel(from.r(), to.r()),
        lerp_channel(from.g(), to.g()),
        lerp_channel(from.b(), to.b()),
        lerp_channel(from.a(), to.a()),
    )
}

/// Decelerating ease (fast start, gentle stop) — the marker tip slowing as the
/// stroke completes.
fn ease_out_cubic(t: f32) -> f32 {
    let u = 1.0 - t;
    1.0 - u * u * u
}

/// Accelerating ease, used so the drying sheen lingers then fades quickly.
fn ease_in_cubic(t: f32) -> f32 {
    t * t * t
}

#[derive(Debug, Clone, Copy)]
struct SettledMarkerMotion {
    alpha: u8,
    sheen_progress: f32,
    sheen_alpha: u8,
}

fn settled_marker_motion(time: f32, seed: u32, target_alpha: u8) -> SettledMarkerMotion {
    let phase = hash01(seed ^ 0xA511_E9B3, 17) * std::f32::consts::TAU;
    let breath = (time * MARKER_BREATH_RATE + phase).sin();
    let alpha = ((target_alpha as f32) * (1.0 + MARKER_BREATH_AMPLITUDE * breath))
        .round()
        .clamp(0.0, 255.0) as u8;

    let cycle =
        MARKER_SHEEN_CYCLE_BASE + MARKER_SHEEN_CYCLE_VARIANCE * hash01(seed ^ 0x63D8_3595, 29);
    let offset = cycle * hash01(seed ^ 0xB529_7A4D, 43);
    let sheen_progress = (time + offset).rem_euclid(cycle) / cycle;
    let fade = (std::f32::consts::PI * sheen_progress).sin().powi(4);
    let sheen_alpha = ((target_alpha as f32) * MARKER_SHEEN_ALPHA * fade)
        .round()
        .clamp(0.0, 255.0) as u8;

    SettledMarkerMotion {
        alpha,
        sheen_progress,
        sheen_alpha,
    }
}

fn marker_sheen_rgb(color_rgb: [f32; 3]) -> [f32; 3] {
    color_rgb.map(|channel| channel + (1.0 - channel) * 0.34)
}

/// Paint a highlight as a felt-tip marker stroke rather than a flat box: wavy
/// (non-straight) top and bottom edges plus uneven ink density — darker pools
/// where the tip lingered, lighter streaks where it skipped. Everything is
/// derived deterministically from `seed`, so the texture stays put frame to
/// frame while the opacity is free to animate (wipe / settle / breathing).
fn paint_marker_stroke(
    painter: &egui::Painter,
    rect: Rect,
    seed: u32,
    color_rgb: [f32; 3],
    base_alpha: u8,
) {
    if base_alpha == 0 {
        return;
    }
    let width = rect.width();
    let height = rect.height();
    if width <= 2.0 || height <= 1.0 {
        painter.rect_filled(
            rect,
            MARKER_CORNER_RADIUS,
            color_from_rgb(color_rgb, base_alpha),
        );
        return;
    }

    // A column strip with jittered top/bottom edges and per-column alpha.
    let cols = (width / 7.0).clamp(6.0, 72.0) as usize;
    let edge = (height * 0.16).clamp(0.6, 3.0);
    let mut mesh = egui::epaint::Mesh::default();
    for i in 0..=cols {
        let f = i as f32 / cols as f32;
        let x = rect.left() + width * f;
        let top_j = (hash01(seed, i as u32) - 0.5) * 2.0 * edge;
        let bot_j = (hash01(seed ^ 0x9E37_79B9, i as u32) - 0.5) * 2.0 * edge;
        let density = 0.78 + 0.34 * hash01(seed ^ 0x85EB_CA6B, i as u32);
        let col = color_from_rgb(
            color_rgb,
            ((base_alpha as f32) * density).clamp(0.0, 255.0) as u8,
        );
        let base_idx = mesh.vertices.len() as u32;
        mesh.colored_vertex(Pos2::new(x, rect.top() + top_j), col);
        mesh.colored_vertex(Pos2::new(x, rect.bottom() + bot_j), col);
        if i > 0 {
            mesh.add_triangle(base_idx - 2, base_idx - 1, base_idx);
            mesh.add_triangle(base_idx - 1, base_idx + 1, base_idx);
        }
    }
    painter.add(egui::Shape::mesh(mesh));

    // A couple of wet pools: soft darker blobs where pigment gathered.
    for k in 0..2u32 {
        let px = rect.left() + width * hash01(seed ^ (0x1234 + k), 101);
        let py = rect.center().y + (hash01(seed ^ (0x5678 + k), 101) - 0.5) * height * 0.5;
        let pr = height * (0.28 + 0.22 * hash01(seed ^ (0x9ABC + k), 101));
        let pa = ((base_alpha as f32) * 0.30).clamp(0.0, 110.0) as u8;
        painter.circle_filled(Pos2::new(px, py), pr, color_from_rgb(color_rgb, pa));
    }
}

/// Stable per-highlight seed from its PDF rectangle.
fn marker_seed(rect: PdfRect) -> u32 {
    rect.left.to_bits() ^ rect.top.to_bits().rotate_left(11) ^ rect.right.to_bits().rotate_left(21)
}

/// Cheap deterministic hash → f32 in [0, 1] (PCG-style bit mixing).
fn hash01(seed: u32, i: u32) -> f32 {
    let mut h = seed
        .wrapping_mul(747_796_405)
        .wrapping_add(i.wrapping_mul(2_891_336_453))
        .wrapping_add(1);
    h ^= h >> 16;
    h = h.wrapping_mul(2_246_822_519);
    h ^= h >> 13;
    h = h.wrapping_mul(3_266_489_917);
    h ^= h >> 16;
    (h as f32) / (u32::MAX as f32)
}

fn color_from_rgb(rgb: [f32; 3], alpha: u8) -> Color32 {
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgba_unmultiplied(channel(rgb[0]), channel(rgb[1]), channel(rgb[2]), alpha)
}

fn close_rgb(left: [f32; 3], right: [f32; 3]) -> bool {
    left.iter()
        .zip(right)
        .all(|(left, right)| (*left - right).abs() <= 0.01)
}

fn comment_marker_screen_rect(rect: Rect) -> Rect {
    let min_size = 24.0;
    if rect.width() >= min_size && rect.height() >= min_size {
        rect
    } else {
        Rect::from_center_size(rect.center(), Vec2::splat(min_size))
    }
}

fn draw_comment_marker(
    painter: &egui::Painter,
    rect: Rect,
    color_rgb: [f32; 3],
    ordinal: usize,
    selected: bool,
) {
    let fill = color_from_rgb(color_rgb, 238);
    let stroke_color = if selected {
        Color32::from_rgb(80, 52, 24)
    } else {
        Color32::from_rgb(132, 96, 45)
    };
    painter.rect_filled(rect, 5, fill);
    painter.rect_stroke(
        rect,
        5,
        Stroke::new(if selected { 2.0 } else { 1.1 }, stroke_color),
        egui::StrokeKind::Inside,
    );
    let fold = 7.0_f32.min(rect.width() * 0.34).min(rect.height() * 0.34);
    painter.line_segment(
        [
            Pos2::new(rect.right() - fold, rect.top()),
            Pos2::new(rect.right(), rect.top() + fold),
        ],
        Stroke::new(1.0, Color32::from_rgba_unmultiplied(88, 60, 30, 130)),
    );
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        ordinal.to_string(),
        FontId::proportional(13.0),
        Color32::from_rgb(42, 31, 18),
    );
}

fn tool_cursor(tool: Tool) -> CursorIcon {
    match tool {
        Tool::Select => CursorIcon::Text,
        Tool::Marker => CursorIcon::Text,
        Tool::TextBox => CursorIcon::Text,
        Tool::Signature => CursorIcon::Cell,
    }
}

fn merge_text_rects(rects: impl IntoIterator<Item = PdfRect>) -> Vec<PdfRect> {
    let mut merged: Vec<PdfRect> = Vec::new();

    for rect in rects {
        if rect.width() <= 0.0 || rect.height() <= 0.0 {
            continue;
        }

        if let Some(last) = merged.last_mut() {
            let last_center = (last.top + last.bottom) * 0.5;
            let rect_center = (rect.top + rect.bottom) * 0.5;
            let same_line_threshold = last.height().max(rect.height()).max(8.0) * 0.7;

            if (last_center - rect_center).abs() <= same_line_threshold {
                *last = PdfRect::new(
                    last.left.min(rect.left),
                    last.bottom.min(rect.bottom),
                    last.right.max(rect.right),
                    last.top.max(rect.top),
                );
                continue;
            }
        }

        merged.push(rect);
    }

    merged
}

fn expand_pdf_rect(rect: PdfRect, amount: f32) -> PdfRect {
    PdfRect::new(
        rect.left - amount,
        rect.bottom - amount,
        rect.right + amount,
        rect.top + amount,
    )
}

fn point_in_pdf_rect(point: (f32, f32), rect: PdfRect) -> bool {
    point.0 >= rect.left && point.0 <= rect.right && point.1 >= rect.bottom && point.1 <= rect.top
}

fn distance_to_pdf_rect(point: (f32, f32), rect: PdfRect) -> f32 {
    let dx = if point.0 < rect.left {
        rect.left - point.0
    } else if point.0 > rect.right {
        point.0 - rect.right
    } else {
        0.0
    };
    let dy = if point.1 < rect.bottom {
        rect.bottom - point.1
    } else if point.1 > rect.top {
        point.1 - rect.top
    } else {
        0.0
    };

    (dx * dx + dy * dy).sqrt()
}

fn default_text_box_rect(placement: PagePlacement, top_left: (f32, f32)) -> PdfRect {
    let width = (180.0 * placement.page_width / placement.rect.width())
        .clamp(72.0, placement.page_width.max(72.0));
    let height = (64.0 * placement.page_height / placement.rect.height())
        .clamp(32.0, placement.page_height.max(32.0));
    let left = top_left.0.min((placement.page_width - width).max(0.0));
    let top = top_left.1.min(placement.page_height).max(height);
    PdfRect::new(left, top - height, left + width, top)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommentSide {
    Left,
    Right,
}

/// Where a comment's margin card is pinned. The stored rect is a zero-size
/// point at the page's left or right edge, vertically level with the anchor;
/// the editor itself renders at a fixed pixel size beside that point, so it
/// lives in the page margin / gutter rather than over the body text.
fn comment_card_rect(
    page_height: f32,
    page_width: f32,
    anchor: (f32, f32),
    side: CommentSide,
) -> PdfRect {
    let y = anchor.1.clamp(0.0, page_height);
    let x = match side {
        CommentSide::Left => 0.0,
        CommentSide::Right => page_width,
    };
    PdfRect::new(x, y, x, y)
}

/// Recover which margin a card sits in from its stored position.
fn comment_card_side(card: PdfRect, page_width: f32) -> CommentSide {
    if (card.left + card.right) * 0.5 <= page_width * 0.5 {
        CommentSide::Left
    } else {
        CommentSide::Right
    }
}

fn translate_pdf_rect_clamped(
    rect: PdfRect,
    dx: f32,
    dy: f32,
    page_width: f32,
    page_height: f32,
) -> PdfRect {
    let width = rect.width();
    let height = rect.height();
    let mut left = rect.left + dx;
    let mut bottom = rect.bottom + dy;
    left = left.clamp(0.0, (page_width - width).max(0.0));
    bottom = bottom.clamp(0.0, (page_height - height).max(0.0));
    PdfRect::new(left, bottom, left + width, bottom + height)
}

/// Translate a rect with no page clamping — used for margin cards, which are
/// allowed to live in the gutter outside the page body.
fn translate_pdf_rect_free(rect: PdfRect, dx: f32, dy: f32) -> PdfRect {
    PdfRect::new(
        rect.left + dx,
        rect.bottom + dy,
        rect.right + dx,
        rect.top + dy,
    )
}

fn response_is_dragging(ctx: &Context) -> bool {
    ctx.input(|input| input.pointer.primary_down())
}

impl PagePlacement {
    fn screen_to_pdf(self, pos: Pos2) -> Option<(f32, f32)> {
        if !self.rect.contains(pos) {
            return None;
        }

        let x = (pos.x - self.rect.left()) * self.page_width / self.rect.width();
        let y_from_top = (pos.y - self.rect.top()) * self.page_height / self.rect.height();
        let y = self.page_height - y_from_top;
        Some((
            x.clamp(0.0, self.page_width),
            y.clamp(0.0, self.page_height),
        ))
    }

    fn pdf_to_screen(self, point: (f32, f32)) -> Pos2 {
        let x = self.rect.left() + point.0 * self.rect.width() / self.page_width;
        let y =
            self.rect.top() + (self.page_height - point.1) * self.rect.height() / self.page_height;
        Pos2::new(x, y)
    }

    /// Inverse of `pdf_to_screen` without clamping to the page bounds, so a
    /// margin card dragged into the gutter still maps back to PDF coordinates.
    fn screen_to_pdf_unclamped(self, pos: Pos2) -> (f32, f32) {
        let x = (pos.x - self.rect.left()) * self.page_width / self.rect.width();
        let y_from_top = (pos.y - self.rect.top()) * self.page_height / self.rect.height();
        (x, self.page_height - y_from_top)
    }

    fn pdf_rect_to_screen(self, pdf_rect: PdfRect) -> Rect {
        Rect::from_min_max(
            self.pdf_to_screen((pdf_rect.left, pdf_rect.top)),
            self.pdf_to_screen((pdf_rect.right, pdf_rect.bottom)),
        )
    }
}

fn draw_pdf_stroke(
    painter: &egui::Painter,
    placement: &PagePlacement,
    stroke_points: &[(f32, f32)],
    stroke: Stroke,
) {
    for points in stroke_points.windows(2) {
        painter.line_segment(
            [
                placement.pdf_to_screen(points[0]),
                placement.pdf_to_screen(points[1]),
            ],
            stroke,
        );
    }
}

fn prepare_open_paths(paths: Vec<PathBuf>) -> (Vec<PathBuf>, usize, Vec<String>) {
    let mut seen = HashSet::new();
    let mut clean = Vec::new();
    let mut converted = 0usize;
    let mut conversion_errors = Vec::new();

    for path in paths {
        let is_pdf = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"));
        let normalized = if is_pdf {
            std::fs::canonicalize(&path).unwrap_or(path)
        } else if text_conversion::is_convertible_source(&path) {
            match text_conversion::convert_source_to_pdf(&path) {
                Ok(destination) => {
                    converted += 1;
                    std::fs::canonicalize(&destination).unwrap_or(destination)
                }
                Err(error) => {
                    conversion_errors
                        .push(format!("Could not convert {}: {error:#}", path.display()));
                    continue;
                }
            }
        } else {
            continue;
        };

        if seen.insert(normalized.clone()) {
            clean.push(normalized);
        }
    }

    (clean, converted, conversion_errors)
}

fn find_hits(text: &str, query: &str, page_index: usize, source: SearchSource) -> Vec<SearchHit> {
    let haystack = text.to_lowercase();
    let needle = query.to_lowercase();

    haystack
        .match_indices(&needle)
        .map(|(start, value)| {
            let end = start + value.len();
            SearchHit {
                page_index,
                source,
                match_start: start,
                snippet: snippet(text, start, end),
            }
        })
        .collect()
}

fn snippet(text: &str, start: usize, end: usize) -> String {
    let start_chars = text[..floor_char_boundary(text, start.min(text.len()))]
        .chars()
        .count();
    let end_chars = text[..floor_char_boundary(text, end.min(text.len()))]
        .chars()
        .count();
    let chars = text.chars().collect::<Vec<_>>();
    let left = start_chars.saturating_sub(42);
    let right = (end_chars + 42).min(chars.len());
    let mut value = chars[left..right].iter().collect::<String>();
    value = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if left > 0 {
        value.insert_str(0, "...");
    }
    if right < chars.len() {
        value.push_str("...");
    }
    value
}

fn floor_char_boundary(value: &str, index: usize) -> usize {
    let mut index = index.min(value.len());
    while index > 0 && !value.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn rect_for_points(points: &[(f32, f32)]) -> PdfRect {
    let mut left = f32::MAX;
    let mut right = f32::MIN;
    let mut bottom = f32::MAX;
    let mut top = f32::MIN;

    for (x, y) in points {
        left = left.min(*x);
        right = right.max(*x);
        bottom = bottom.min(*y);
        top = top.max(*y);
    }

    PdfRect::new(left - 4.0, bottom - 4.0, right + 4.0, top + 4.0)
}

fn distance(left: (f32, f32), right: (f32, f32)) -> f32 {
    let dx = left.0 - right.0;
    let dy = left.1 - right.1;
    (dx * dx + dy * dy).sqrt()
}

fn signature_timestamp() -> String {
    OffsetDateTime::now_local()
        .unwrap_or_else(|_| OffsetDateTime::now_utc())
        .format(&Rfc3339)
        .unwrap_or_else(|_| "unknown time".to_owned())
}

fn compact_liquid_metadata(text: &str) -> String {
    let mut compact = text.trim().to_owned();
    if let Some((_, rest)) = compact.split_once("Contracts Exam - ") {
        compact = rest.trim().to_owned();
    }
    for prefix in [
        "Date:",
        "Source:",
        "Published:",
        "Published",
        "Updated:",
        "Keywords:",
        "Key words:",
        "JEL Classification:",
        "JEL Classifications:",
        "Received:",
        "Accepted:",
        "Revised:",
    ] {
        if let Some(rest) = compact.strip_prefix(prefix) {
            compact = rest.trim().to_owned();
            break;
        }
    }
    compact = compact.replace(" | ", "  ");
    compact
}

fn compact_liquid_metadata_parts(metadata: &[LiquidBlock]) -> Vec<String> {
    let mut parts = Vec::new();
    for block in metadata {
        if block.role != LiquidBlockRole::Metadata {
            continue;
        }
        let compact = compact_liquid_metadata(&block.text);
        if compact.is_empty()
            || parts
                .iter()
                .any(|part: &String| part.eq_ignore_ascii_case(&compact))
        {
            continue;
        }
        parts.push(compact);
    }
    parts
}

fn liquid_document_copy_text(document: &LiquidDocument) -> Option<String> {
    let hidden_contents = hidden_contents_mask_for_display(&document.blocks);
    let mut parts = Vec::new();
    push_liquid_copy_part(&mut parts, document.title.trim());

    for (index, block) in document.blocks.iter().enumerate() {
        if hidden_contents.get(index).copied().unwrap_or(false)
            || should_hide_contents_block_for_display(block)
        {
            continue;
        }
        let text = match block.role {
            LiquidBlockRole::Header
            | LiquidBlockRole::Footer
            | LiquidBlockRole::Contents
            | LiquidBlockRole::Noise
            | LiquidBlockRole::SectionBreak => continue,
            LiquidBlockRole::Metadata => compact_liquid_metadata(&block.text),
            _ => block.text.split_whitespace().collect::<Vec<_>>().join(" "),
        };
        push_liquid_copy_part(&mut parts, text.trim());
    }

    (!parts.is_empty()).then(|| parts.join("\n\n"))
}

fn push_liquid_copy_part(parts: &mut Vec<String>, text: &str) {
    if text.is_empty() {
        return;
    }
    if parts
        .last()
        .is_some_and(|previous| previous.eq_ignore_ascii_case(text))
    {
        return;
    }
    parts.push(text.to_owned());
}

fn liquid_outline_items(blocks: &[LiquidBlock]) -> Vec<LiquidOutlineItem> {
    let mut outline = Vec::new();
    let hidden_contents = hidden_contents_mask_for_display(blocks);
    for (index, block) in blocks.iter().enumerate() {
        if hidden_contents.get(index).copied().unwrap_or(false)
            || should_hide_contents_block_for_display(block)
        {
            continue;
        }
        let level = match block.role {
            LiquidBlockRole::Heading => 1,
            LiquidBlockRole::Subheading => 2,
            _ => continue,
        };
        let text = compact_liquid_outline_text(&block.text);
        if text.is_empty()
            || outline
                .last()
                .is_some_and(|item: &LiquidOutlineItem| item.level == level && item.text == text)
        {
            continue;
        }
        outline.push(LiquidOutlineItem { level, text });
        if outline.len() >= MAX_LIQUID_OUTLINE_ITEMS {
            break;
        }
    }
    outline
}

fn compact_liquid_outline_text(text: &str) -> String {
    const MAX_OUTLINE_CHARS: usize = 96;
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= MAX_OUTLINE_CHARS {
        return normalized;
    }
    let mut compact = normalized
        .chars()
        .take(MAX_OUTLINE_CHARS.saturating_sub(3))
        .collect::<String>();
    compact.push_str("...");
    compact
}

fn profile_display_name(kind: DocumentProfileKind) -> &'static str {
    match kind {
        DocumentProfileKind::LawReviewArticle => "Law review",
        DocumentProfileKind::ScienceArticle => "Science article",
        DocumentProfileKind::Contract => "Contract",
        DocumentProfileKind::LegalFilingOrOpinion => "Legal filing",
        DocumentProfileKind::NewsArticle => "News article",
        DocumentProfileKind::FreeProse => "Free prose",
        DocumentProfileKind::CvOrAcademicPacket => "CV/academic packet",
        DocumentProfileKind::ReceiptInvoiceFinancial => "Receipt/financial",
        DocumentProfileKind::CourseOrExamMaterial => "Course/exam",
        DocumentProfileKind::BookOrChapter => "Book/chapter",
        DocumentProfileKind::PolicyReport => "Policy report",
        DocumentProfileKind::FormReceiptAdmin => "Form/receipt",
        DocumentProfileKind::GeneralDocument => "General document",
        DocumentProfileKind::ScannedImageOnly => "Scanned/image-only",
        DocumentProfileKind::Other => "Other",
    }
}

fn liquid_state_needs_ocr(state: &LiquidState) -> bool {
    match state {
        LiquidState::Ready(document) => liquid_document_needs_ocr(document),
        LiquidState::Failed(error) => error.contains("No selectable text found"),
        _ => false,
    }
}

fn liquid_document_needs_ocr(document: &LiquidDocument) -> bool {
    document
        .profile
        .as_ref()
        .is_some_and(|profile| profile.kind == DocumentProfileKind::ScannedImageOnly)
        || document
            .warnings
            .iter()
            .any(|warning| warning.contains("No selectable text found"))
}

/// #23: is a block readable body content (as opposed to a note or page furniture)?
/// #23: is a block page furniture (headers/footers/noise/TOC/tables/breaks)?
fn liquid_role_is_furniture(role: LiquidBlockRole) -> bool {
    matches!(
        role,
        LiquidBlockRole::Header
            | LiquidBlockRole::Footer
            | LiquidBlockRole::Noise
            | LiquidBlockRole::Contents
            | LiquidBlockRole::Table
            | LiquidBlockRole::SectionBreak
    )
}

/// #23: agent2's R1-calibrated `reader_yield_collapse` gate (recall/precision 1.0 on the R1-200
/// set, flagging 13/200 near-empty docs). Returns a user-facing hint when the reflow yields too
/// little usable text to trust (so the reader warns + offers the fixed layout instead of showing
/// a broken reflow), else None. Computed from the assembled document:
/// `markdown_bytes` ~= visible non-furniture block-text bytes (reflow-output proxy);
/// `source_lines`/`input_bytes` from block_source_lines (classified input);
/// `yield_ratio` = output/input.
fn liquid_reflow_low_confidence(document: &LiquidDocument) -> Option<&'static str> {
    let blocks = document.blocks.len();
    let markdown_bytes: usize = document
        .blocks
        .iter()
        .filter(|block| !liquid_role_is_furniture(block.role))
        .map(|block| block.text.len())
        .sum();
    let source_lines: usize = document
        .block_source_lines
        .iter()
        .map(|sources| sources.lines.len())
        .sum();
    let input_bytes: usize = document
        .block_source_lines
        .iter()
        .flat_map(|sources| sources.lines.iter())
        .map(|line| line.text.len())
        .sum();
    let yield_ratio = if input_bytes == 0 {
        0.0
    } else {
        markdown_bytes as f32 / input_bytes as f32
    };
    let collapse = markdown_bytes < 1100
        || (yield_ratio < 0.001 && blocks < 20)
        || (blocks <= 3 && source_lines >= 30);
    if collapse {
        return Some(
            "This document produced very little reflowable text, so the fixed layout is more reliable.",
        );
    }
    None
}

/// #33: drop inline footnote-marker callouts (the private-use sentinel spans) from text so TTS
/// doesn't read superscript reference numbers aloud mid-sentence.
fn strip_liquid_callouts(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_callout = false;
    for ch in text.chars() {
        match ch {
            crate::layout_roles::CALLOUT_START => in_callout = true,
            crate::layout_roles::CALLOUT_END => in_callout = false,
            _ if in_callout => {}
            _ => out.push(ch),
        }
    }
    out
}

/// #33: assemble the reading text for TTS in reading order — body blocks first (furniture and
/// hidden blocks skipped, marker callouts stripped), then footnotes as a separate pass.
fn liquid_tts_text(document: &LiquidDocument, include_notes: bool) -> String {
    let hidden = hidden_contents_mask_for_display(&document.blocks);
    let mut body = String::new();
    let mut notes = String::new();
    for (index, block) in document.blocks.iter().enumerate() {
        if hidden.get(index).copied().unwrap_or(false)
            || should_hide_contents_block_for_display(block)
            || liquid_role_is_furniture(block.role)
        {
            continue;
        }
        let cleaned = strip_liquid_callouts(&block.text);
        let cleaned = cleaned.trim();
        if cleaned.is_empty() {
            continue;
        }
        if matches!(
            block.role,
            LiquidBlockRole::Marginalia | LiquidBlockRole::Footnote
        ) {
            notes.push_str(cleaned);
            notes.push_str("\n\n");
        } else {
            body.push_str(cleaned);
            body.push_str("\n\n");
        }
    }
    let mut out = body;
    if include_notes {
        let notes = notes.trim();
        if !notes.is_empty() {
            out.push_str("\n\nFootnotes.\n\n");
            out.push_str(notes);
        }
    }
    out.trim().to_owned()
}

fn has_usable_ocr_text(states: &[OcrPageState]) -> bool {
    states
        .iter()
        .filter_map(OcrPageState::text)
        .any(|text| !text.trim().is_empty())
}

fn zoom_for_new_document(active_target_zoom: Option<f32>, settings: &AppSettings) -> f32 {
    active_target_zoom
        .map(normalized_pdf_zoom)
        .unwrap_or_else(|| normalized_pdf_zoom(settings.last_pdf_zoom))
}

fn liquid_note_blocks(blocks: &[LiquidBlock]) -> Vec<&LiquidBlock> {
    blocks
        .iter()
        .filter(|block| {
            matches!(
                block.role,
                LiquidBlockRole::Footnote | LiquidBlockRole::Marginalia
            )
        })
        .collect()
}

fn liquid_margin_note_goes_left(block_index: usize, note: &LiquidBlock) -> bool {
    split_liquid_note_marker(&note.text)
        .0
        .and_then(|marker| marker.parse::<usize>().ok())
        .map(|marker| marker % 2 == 0)
        .unwrap_or(block_index % 2 == 0)
}

fn compact_liquid_margin_note_text(text: &str) -> String {
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut output = compact
        .chars()
        .take(LIQUID_MARGIN_NOTE_MAX_CHARS)
        .collect::<String>();
    if compact.chars().count() > LIQUID_MARGIN_NOTE_MAX_CHARS {
        output = output
            .trim_end_matches(['.', ',', ';', ':', '-'])
            .to_owned();
        output.push_str("...");
    }
    output
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LiquidListItemParts<'a> {
    marker: String,
    body: &'a str,
    indent_level: usize,
}

fn liquid_list_item_parts(text: &str) -> LiquidListItemParts<'_> {
    let trimmed = text.trim_start();
    for prefix in ["- ", "* ", "\u{2022} ", "â€¢ "] {
        if let Some(body) = trimmed.strip_prefix(prefix) {
            return LiquidListItemParts {
                marker: "\u{2022}".to_owned(),
                body: body.trim_start(),
                indent_level: 1,
            };
        }
    }

    if let Some((marker, body)) = split_parenthesized_list_marker(trimmed) {
        return LiquidListItemParts {
            marker: marker.to_owned(),
            body,
            indent_level: 2,
        };
    }

    if let Some((marker, body, indent_level)) = split_numbered_list_marker(trimmed) {
        return LiquidListItemParts {
            marker: marker.to_owned(),
            body,
            indent_level,
        };
    }

    LiquidListItemParts {
        marker: "\u{2022}".to_owned(),
        body: trimmed,
        indent_level: 0,
    }
}

fn split_parenthesized_list_marker(text: &str) -> Option<(&str, &str)> {
    let rest = text.strip_prefix('(')?;
    let close = rest.find(')')?;
    let marker_end = close + 2;
    let marker = &text[..marker_end];
    let body = text[marker_end..].trim_start();
    let inner = &rest[..close];
    (!body.is_empty() && inner.len() <= 3 && inner.chars().all(|ch| ch.is_ascii_alphanumeric()))
        .then_some((marker, body))
}

fn split_numbered_list_marker(text: &str) -> Option<(&str, &str, usize)> {
    let marker_end = text.find(char::is_whitespace)?;
    let marker = &text[..marker_end];
    let body = text[marker_end..].trim_start();
    if marker.is_empty() || body.is_empty() {
        return None;
    }

    let normalized_marker = marker.trim_end_matches(['.', ')']);
    if normalized_marker.is_empty() {
        return None;
    }

    if normalized_marker.split('.').all(|part| {
        !part.is_empty() && part.len() <= 3 && part.chars().all(|ch| ch.is_ascii_digit())
    }) {
        let components = normalized_marker.split('.').count();
        return Some((marker, body, components.saturating_sub(1).min(4)));
    }

    if normalized_marker.len() <= 2 && normalized_marker.chars().all(|ch| ch.is_ascii_alphabetic())
    {
        return Some((marker, body, 2));
    }

    None
}

/// #27: Build a footnote-number -> footnote-text index from a rendered document's
/// note blocks (Footnote or Marginalia roles). Body superscript markers resolve
/// against this map to drive the tap-to-view popover. First occurrence of a number
/// wins — footnote numbering can restart per section, but the rendered `LiquidBlock`
/// carries no page/order to disambiguate on (only upstream source lines do).
fn build_liquid_footnote_index(blocks: &[LiquidBlock]) -> HashMap<u16, String> {
    let mut index = HashMap::new();
    for block in blocks {
        if !matches!(
            block.role,
            LiquidBlockRole::Footnote | LiquidBlockRole::Marginalia
        ) {
            continue;
        }
        let (marker, body) = split_liquid_note_marker(&block.text);
        let Some(marker) = marker else { continue };
        let Ok(number) = marker.parse::<u16>() else {
            continue;
        };
        if number == 0 {
            continue;
        }
        let body = body.trim();
        if body.is_empty() {
            continue;
        }
        index.entry(number).or_insert_with(|| body.to_owned());
    }
    index
}

/// Stable egui id for a body marker's footnote popover, unique per block+number.
fn liquid_footnote_popup_id(feedback_id: &str, number: u16) -> egui::Id {
    egui::Id::new(("liquid-fn-popover", feedback_id, number))
}

fn split_liquid_note_marker(text: &str) -> (Option<&str>, &str) {
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

fn callout_body_text<'a>(label: &str, text: &'a str) -> &'a str {
    let trimmed = text.trim_start();
    let Some((prefix, rest)) = trimmed.split_once(':') else {
        return text;
    };
    if prefix.chars().count() > 48 || prefix.split_whitespace().count() > 6 {
        return text;
    }
    if normalize_callout_label(prefix) == normalize_callout_label(label) {
        rest.trim_start()
    } else {
        text
    }
}

fn normalize_callout_label(value: &str) -> String {
    let normalized = value
        .trim()
        .trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != ' ')
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    match normalized.as_str() {
        "q" => "question".to_owned(),
        "a" => "answer".to_owned(),
        _ => normalized,
    }
}

fn default_output_name(source: &Path, suffix: &str, extension: &str) -> String {
    let stem = source
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("document");
    format!("{stem}-{suffix}.{extension}")
}

fn tab_title(document: &LoadedDocument) -> String {
    if !document.title.trim().is_empty() {
        return document.title.clone();
    }

    document
        .path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("Untitled PDF")
        .to_owned()
}

#[cfg(test)]
mod app_tests {
    use super::*;

    #[test]
    fn notice_queue_keeps_only_the_newest_five_items() {
        let mut notices = VecDeque::new();
        for index in 0..7 {
            enqueue_notice(
                &mut notices,
                Notice::new(format!("notice-{index}"), NoticeSeverity::Info),
            );
        }
        assert_eq!(notices.len(), NOTICE_CAPACITY);
        assert_eq!(
            notices.front().map(|notice| notice.message.as_str()),
            Some("notice-2")
        );
        assert_eq!(
            notices.back().map(|notice| notice.message.as_str()),
            Some("notice-6")
        );
    }

    #[test]
    fn notice_expiry_removes_old_info_but_keeps_errors() {
        let now = Instant::now();
        let old = now - INFO_NOTICE_DURATION - Duration::from_millis(1);
        let mut notices = VecDeque::from([
            Notice {
                message: "old info".to_owned(),
                severity: NoticeSeverity::Info,
                created_at: old,
            },
            Notice {
                message: "old error".to_owned(),
                severity: NoticeSeverity::Error,
                created_at: old,
            },
            Notice {
                message: "new info".to_owned(),
                severity: NoticeSeverity::Info,
                created_at: now,
            },
        ]);
        prune_notices_at(&mut notices, now);
        assert_eq!(
            notices
                .iter()
                .map(|notice| notice.message.as_str())
                .collect::<Vec<_>>(),
            vec!["old error", "new info"]
        );
    }

    fn key_press_input(key: egui::Key, modifiers: egui::Modifiers) -> egui::InputState {
        let mut input = egui::InputState::default();
        input.events.push(egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers,
        });
        input
    }

    fn test_liquid_document_with_blocks(blocks: Vec<LiquidBlock>) -> LiquidDocument {
        let mut doc = test_liquid_document(None, Vec::new());
        doc.blocks = blocks;
        doc
    }

    fn test_liquid_document(
        profile_kind: Option<DocumentProfileKind>,
        warnings: Vec<&str>,
    ) -> LiquidDocument {
        LiquidDocument {
            title: "Test Document".to_owned(),
            blocks: Vec::new(),
            block_source_lines: Vec::new(),
            footnote_links: Vec::new(),
            footnote_link_integrity: None,
            profile: profile_kind.map(|kind| crate::liquid::DocumentProfile {
                kind,
                confidence: 0.9,
                scores: vec![crate::liquid::DocumentProfileScore { kind, score: 1.0 }],
                evidence: Vec::new(),
            }),
            noise_lines_removed: 0,
            llm_used: false,
            llm_provider: None,
            deep_liquid_used: false,
            deep_liquid_model: None,
            warnings: warnings.into_iter().map(str::to_owned).collect(),
            source_signature: "test".to_owned(),
        }
    }

    fn test_liquid_block(role: LiquidBlockRole, text: &str) -> LiquidBlock {
        LiquidBlock {
            role,
            text: text.to_owned(),
            label: None,
        }
    }

    #[test]
    fn callout_body_text_strips_matching_leading_label() {
        assert_eq!(
            callout_body_text(
                "Why it matters",
                "Why it matters: The holding changes the stakes."
            ),
            "The holding changes the stakes."
        );
        assert_eq!(
            callout_body_text("Bottom line", "Bottom line: Preserve objections early."),
            "Preserve objections early."
        );
        assert_eq!(
            callout_body_text("Question", "Q: What changed in the rule?"),
            "What changed in the rule?"
        );
        assert_eq!(
            callout_body_text("Answer", "A: The rule changed quickly."),
            "The rule changed quickly."
        );
    }

    #[test]
    fn callout_body_text_preserves_nonmatching_prefixes() {
        let text = "Context differs: this is the real sentence.";
        assert_eq!(callout_body_text("Why it matters", text), text);
    }

    #[test]
    fn compact_liquid_metadata_strips_common_context_prefixes() {
        assert_eq!(
            compact_liquid_metadata("Contracts Exam - Part IV | Character Limit: 10,000"),
            "Part IV  Character Limit: 10,000"
        );
        assert_eq!(
            compact_liquid_metadata("Source: The Example Times"),
            "The Example Times"
        );
        assert_eq!(
            compact_liquid_metadata("Date: May 28, 2026"),
            "May 28, 2026"
        );
        assert_eq!(
            compact_liquid_metadata("Updated: May 28, 2026"),
            "May 28, 2026"
        );
        assert_eq!(
            compact_liquid_metadata("Keywords: administrative law; agencies"),
            "administrative law; agencies"
        );
        assert_eq!(
            compact_liquid_metadata("JEL Classification: K23, K41"),
            "K23, K41"
        );
    }

    #[test]
    fn compact_liquid_metadata_parts_deduplicates_metadata_runs() {
        let blocks = vec![
            LiquidBlock {
                role: LiquidBlockRole::Metadata,
                text: "Source: The Example Times".to_owned(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Metadata,
                text: "The Example Times".to_owned(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Metadata,
                text: "5 min read".to_owned(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Paragraph,
                text: "Body text".to_owned(),
                label: None,
            },
        ];

        assert_eq!(
            compact_liquid_metadata_parts(&blocks),
            vec!["The Example Times".to_owned(), "5 min read".to_owned()]
        );
    }

    #[test]
    fn liquid_document_copy_text_uses_visible_reading_text() {
        let mut document = test_liquid_document(None, vec![]);
        document.title = "Example Article".to_owned();
        document.blocks = vec![
            test_liquid_block(LiquidBlockRole::Title, "Example Article"),
            test_liquid_block(LiquidBlockRole::Header, "EXAMPLE JOURNAL"),
            test_liquid_block(LiquidBlockRole::Metadata, "Source: SSRN"),
            test_liquid_block(LiquidBlockRole::Paragraph, "This is the body   paragraph."),
            test_liquid_block(LiquidBlockRole::Marginalia, "1. A margin note."),
            test_liquid_block(LiquidBlockRole::Footnote, "2. A footnote."),
            test_liquid_block(LiquidBlockRole::Noise, "Downloaded from repository"),
            test_liquid_block(LiquidBlockRole::Contents, "Contents ........ 1"),
        ];

        let copied = liquid_document_copy_text(&document).unwrap();
        assert!(copied.contains("Example Article"));
        assert!(copied.contains("SSRN"));
        assert!(copied.contains("This is the body paragraph."));
        assert!(copied.contains("1. A margin note."));
        assert!(copied.contains("2. A footnote."));
        assert!(!copied.contains("EXAMPLE JOURNAL"));
        assert!(!copied.contains("Downloaded from repository"));
        assert!(!copied.contains("Contents ........ 1"));
        assert_eq!(copied.matches("Example Article").count(), 1);
    }

    #[test]
    fn liquid_outline_items_extracts_heading_hierarchy() {
        let blocks = vec![
            LiquidBlock {
                role: LiquidBlockRole::Title,
                text: "Document".to_owned(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Heading,
                text: "I. Introduction".to_owned(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Subheading,
                text: "A. Agency Reliance Interests".to_owned(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Paragraph,
                text: "Body".to_owned(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Subheading,
                text: "A. Agency Reliance Interests".to_owned(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Heading,
                text: "II. Conclusion".to_owned(),
                label: None,
            },
        ];

        assert_eq!(
            liquid_outline_items(&blocks),
            vec![
                LiquidOutlineItem {
                    level: 1,
                    text: "I. Introduction".to_owned()
                },
                LiquidOutlineItem {
                    level: 2,
                    text: "A. Agency Reliance Interests".to_owned()
                },
                LiquidOutlineItem {
                    level: 1,
                    text: "II. Conclusion".to_owned()
                }
            ]
        );
    }

    #[test]
    fn liquid_outline_items_skip_table_of_contents_clutter() {
        let blocks = vec![
            LiquidBlock {
                role: LiquidBlockRole::Title,
                text: "Document".to_owned(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Heading,
                text: "Table of Contents".to_owned(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Heading,
                text: "INTRODUCTION ........................................................................ 1"
                    .to_owned(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Subheading,
                text: "Consumer Sign-in-Wrap Contracts 2264".to_owned(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Heading,
                text: "I. Introduction".to_owned(),
                label: None,
            },
        ];

        assert_eq!(
            liquid_outline_items(&blocks),
            vec![LiquidOutlineItem {
                level: 1,
                text: "I. Introduction".to_owned()
            }]
        );
    }

    #[test]
    fn liquid_outline_items_skip_long_cached_table_of_contents_clutter() {
        let mut blocks = vec![
            LiquidBlock {
                role: LiquidBlockRole::Title,
                text: "Document".to_owned(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Heading,
                text: "Table of Contents".to_owned(),
                label: None,
            },
        ];
        for index in 1..=32 {
            blocks.push(LiquidBlock {
                role: LiquidBlockRole::Heading,
                text: format!("Chapter {index} ........................................ {index}"),
                label: None,
            });
        }
        blocks.push(LiquidBlock {
            role: LiquidBlockRole::Heading,
            text: "I. Introduction".to_owned(),
            label: None,
        });

        assert_eq!(
            liquid_outline_items(&blocks),
            vec![LiquidOutlineItem {
                level: 1,
                text: "I. Introduction".to_owned()
            }]
        );
    }

    #[test]
    fn liquid_outline_items_skip_table_tagged_table_of_contents_clutter() {
        let blocks = vec![
            LiquidBlock {
                role: LiquidBlockRole::Title,
                text: "Document".to_owned(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Table,
                text: "Table of Contents".to_owned(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Table,
                text: "INTRODUCTION ........................................................................ 1"
                    .to_owned(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Heading,
                text: "I. Introduction".to_owned(),
                label: None,
            },
        ];

        assert_eq!(
            liquid_outline_items(&blocks),
            vec![LiquidOutlineItem {
                level: 1,
                text: "I. Introduction".to_owned()
            }]
        );
    }

    #[test]
    fn liquid_outline_items_skip_page_less_cached_table_of_contents_clutter() {
        let blocks = vec![
            LiquidBlock {
                role: LiquidBlockRole::Title,
                text: "Document".to_owned(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Heading,
                text: "Table of Contents".to_owned(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Heading,
                text: "Introduction".to_owned(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Heading,
                text: "Theoretical Background".to_owned(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Subheading,
                text: "Consumer Sign-in-Wrap Contracts".to_owned(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Heading,
                text: "Introduction".to_owned(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Paragraph,
                text: "The article begins here with actual body prose after the hidden contents outline."
                    .to_owned(),
                label: None,
            },
        ];

        assert_eq!(
            liquid_outline_items(&blocks),
            vec![LiquidOutlineItem {
                level: 1,
                text: "Introduction".to_owned()
            }]
        );
    }

    #[test]
    fn liquid_outline_items_skip_short_page_less_cached_table_of_contents_clutter() {
        let blocks = vec![
            LiquidBlock {
                role: LiquidBlockRole::Title,
                text: "Document".to_owned(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Heading,
                text: "Table of Contents".to_owned(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Heading,
                text: "Introduction".to_owned(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Heading,
                text: "Conclusion".to_owned(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Heading,
                text: "Introduction".to_owned(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Paragraph,
                text: "The article begins here with actual body prose after the hidden short contents outline."
                    .to_owned(),
                label: None,
            },
        ];

        assert_eq!(
            liquid_outline_items(&blocks),
            vec![LiquidOutlineItem {
                level: 1,
                text: "Introduction".to_owned()
            }]
        );
    }

    #[test]
    fn compact_liquid_outline_text_truncates_long_sections() {
        let text = "A Very Long Section Heading That Keeps Going With More Detail Than A Compact Outline Should Show In Full";
        let compact = compact_liquid_outline_text(text);

        assert!(compact.chars().count() <= 96);
        assert!(compact.ends_with("..."));
    }

    #[test]
    fn profile_display_name_uses_short_user_facing_labels() {
        assert_eq!(
            profile_display_name(DocumentProfileKind::LawReviewArticle),
            "Law review"
        );
        assert_eq!(
            profile_display_name(DocumentProfileKind::Contract),
            "Contract"
        );
        assert_eq!(
            profile_display_name(DocumentProfileKind::ScannedImageOnly),
            "Scanned/image-only"
        );
        assert_eq!(
            profile_display_name(DocumentProfileKind::CvOrAcademicPacket),
            "CV/academic packet"
        );
    }

    #[test]
    fn liquid_document_needs_ocr_detects_scanned_outputs() {
        let by_profile = test_liquid_document(Some(DocumentProfileKind::ScannedImageOnly), vec![]);
        assert!(liquid_document_needs_ocr(&by_profile));

        let by_warning = test_liquid_document(
            Some(DocumentProfileKind::GeneralDocument),
            vec!["No selectable text found. Run OCR to create a searchable copy."],
        );
        assert!(liquid_document_needs_ocr(&by_warning));

        let ordinary = test_liquid_document(Some(DocumentProfileKind::GeneralDocument), vec![]);
        assert!(!liquid_document_needs_ocr(&ordinary));
    }

    #[test]
    fn has_usable_ocr_text_requires_nonblank_done_text() {
        assert!(!has_usable_ocr_text(&[
            OcrPageState::Idle,
            OcrPageState::Done("   ".to_owned())
        ]));
        assert!(has_usable_ocr_text(&[
            OcrPageState::Failed("missing engine".to_owned()),
            OcrPageState::Done("Recognized text".to_owned())
        ]));
    }

    #[test]
    fn zoom_for_new_document_uses_active_zoom_when_document_is_open() {
        let mut settings = AppSettings::default();
        settings.last_pdf_zoom = 1.0;

        assert_eq!(zoom_for_new_document(Some(1.75), &settings), 1.75);
    }

    #[test]
    fn zoom_for_new_document_uses_saved_zoom_without_active_document() {
        let mut settings = AppSettings::default();
        settings.last_pdf_zoom = 1.65;

        assert_eq!(zoom_for_new_document(None, &settings), 1.65);
    }

    #[test]
    fn mac_command_a_consumes_pdf_select_all_shortcut() {
        let mut input = key_press_input(
            egui::Key::A,
            egui::Modifiers::MAC_CMD | egui::Modifiers::COMMAND,
        );

        assert!(input_has_command_shortcut(&mut input, egui::Key::A));
        assert!(!input_has_command_shortcut(&mut input, egui::Key::A));
    }

    #[test]
    fn mac_command_c_consumes_pdf_copy_shortcut() {
        let mut input = key_press_input(
            egui::Key::C,
            egui::Modifiers::MAC_CMD | egui::Modifiers::COMMAND,
        );

        assert!(input_has_command_shortcut(&mut input, egui::Key::C));
        assert!(!input_has_command_shortcut(&mut input, egui::Key::C));
    }

    #[test]
    fn platform_copy_event_is_detected() {
        let mut input = egui::InputState::default();
        assert!(!input_has_copy_event(&input));

        input.events.push(egui::Event::Copy);
        assert!(input_has_copy_event(&input));
    }

    #[test]
    fn copy_shortcut_peek_does_not_consume_command_c() {
        let mut input = key_press_input(
            egui::Key::C,
            egui::Modifiers::MAC_CMD | egui::Modifiers::COMMAND,
        );

        assert!(input_has_command_shortcut_event(&input, egui::Key::C));
        assert!(input_has_command_shortcut(&mut input, egui::Key::C));
    }

    #[test]
    fn copy_shortcut_peek_detects_native_mac_command_c() {
        let input = key_press_input(egui::Key::C, egui::Modifiers::MAC_CMD);

        assert!(input_has_command_shortcut_event(&input, egui::Key::C));
    }

    #[test]
    fn copy_shortcut_consumer_removes_copy_event() {
        let ctx = egui::Context::default();
        ctx.input_mut(|input| input.events.push(egui::Event::Copy));

        assert!(copy_shortcut_requested(&ctx));
        assert!(consume_copy_shortcut(&ctx));
        assert!(!copy_shortcut_requested(&ctx));
    }

    #[test]
    fn copy_shortcut_consumer_removes_native_mac_command_c() {
        let ctx = egui::Context::default();
        ctx.input_mut(|input| {
            *input = key_press_input(egui::Key::C, egui::Modifiers::MAC_CMD);
        });

        assert!(copy_shortcut_requested(&ctx));
        assert!(consume_copy_shortcut(&ctx));
        assert!(!copy_shortcut_requested(&ctx));
    }

    #[test]
    fn command_shortcut_event_fallback_consumes_key_event() {
        let ctx = egui::Context::default();
        ctx.input_mut(|input| {
            input.events.push(egui::Event::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::MAC_CMD | egui::Modifiers::COMMAND,
            });
        });

        assert!(consume_command_shortcut_or_key_event(&ctx, egui::Key::A));
        assert!(!consume_command_shortcut_or_key_event(&ctx, egui::Key::A));
    }

    #[test]
    fn command_shortcut_consumes_native_mac_command_a() {
        let ctx = egui::Context::default();
        ctx.input_mut(|input| {
            *input = key_press_input(egui::Key::A, egui::Modifiers::MAC_CMD);
        });

        assert!(consume_command_shortcut(&ctx, egui::Key::A));
        assert!(!consume_command_shortcut(&ctx, egui::Key::A));
    }

    #[test]
    fn document_shortcuts_allow_liquid_read_only_focus_but_not_text_edit_focus() {
        assert!(document_shortcuts_allowed(
            DocumentViewMode::Liquid,
            true,
            false
        ));
        assert!(document_shortcuts_allowed(
            DocumentViewMode::LiquidMode2,
            true,
            false
        ));
        assert!(!document_shortcuts_allowed(
            DocumentViewMode::LiquidMode2,
            true,
            true
        ));
        assert!(!document_shortcuts_allowed(
            DocumentViewMode::Pdf,
            true,
            false
        ));
    }

    #[test]
    fn liquid_shortcuts_are_allowed_for_read_only_liquid_views_only() {
        assert!(liquid_document_shortcuts_allowed(
            DocumentViewMode::Liquid,
            false
        ));
        assert!(liquid_document_shortcuts_allowed(
            DocumentViewMode::LiquidMode2,
            false
        ));
        assert!(!liquid_document_shortcuts_allowed(
            DocumentViewMode::Liquid,
            true
        ));
        assert!(!liquid_document_shortcuts_allowed(
            DocumentViewMode::Pdf,
            false
        ));
    }

    #[test]
    fn pdf_copy_shortcut_depends_on_selection_and_copy_event_only() {
        assert!(should_copy_pdf_selection_on_shortcut(true, true));
        assert!(!should_copy_pdf_selection_on_shortcut(false, true));
        assert!(!should_copy_pdf_selection_on_shortcut(true, false));
    }

    #[test]
    fn liquid_copy_shortcut_depends_on_selection_and_copy_event_only() {
        assert!(should_copy_liquid_selection_on_shortcut(true, false, true));
        assert!(should_copy_liquid_selection_on_shortcut(false, true, true));
        assert!(!should_copy_liquid_selection_on_shortcut(
            false, false, true
        ));
        assert!(!should_copy_liquid_selection_on_shortcut(true, true, false));
    }

    #[test]
    fn copy_status_message_records_success_and_errors() {
        assert_eq!(copy_status_message(12, None), "Copied 12 character(s)");
        let failed = copy_status_message(12, Some("clipboard unavailable"));
        assert!(failed.contains("Copy failed"));
        assert!(failed.contains("clipboard unavailable"));
    }

    #[test]
    fn comment_card_pins_to_alternating_margins() {
        let anchor = (300.0, 500.0);
        let right = comment_card_rect(792.0, 612.0, anchor, CommentSide::Right);
        let left = comment_card_rect(792.0, 612.0, anchor, CommentSide::Left);

        assert_eq!(right.left, 612.0);
        assert_eq!(left.left, 0.0);
        assert_eq!(comment_card_side(right, 612.0), CommentSide::Right);
        assert_eq!(comment_card_side(left, 612.0), CommentSide::Left);
    }

    #[test]
    fn comment_preview_compacts_and_truncates_text() {
        assert_eq!(comment_preview("   \n\t  "), None);
        let preview = comment_preview(
            "This   comment\ncontains enough words to require a short sidebar preview.",
        )
        .unwrap();

        assert!(preview.ends_with("..."));
        assert!(preview.chars().count() <= 44);
        assert!(!preview.contains('\n'));
    }

    #[test]
    fn autosave_payload_keeps_only_comments() {
        let annotations = vec![
            EditorAnnotation {
                page_index: 0,
                rect: PdfRect::new(1.0, 2.0, 3.0, 4.0),
                kind: AnnotationKind::Marker {
                    color_rgb: [1.0, 1.0, 0.0],
                    opacity: 0.4,
                    style: MarkerStyle::Highlight,
                },
            },
            EditorAnnotation {
                page_index: 0,
                rect: PdfRect::new(10.0, 10.0, 38.0, 38.0),
                kind: AnnotationKind::Comment {
                    id: "LawPDF-comment-test".to_owned(),
                    text: "Review this cite.".to_owned(),
                    color_rgb: [1.0, 0.78, 0.28],
                    created_at: "now".to_owned(),
                    updated_at: "now".to_owned(),
                    anchor: (24.0, 24.0),
                },
            },
        ];

        let comments = comment_annotations_for_save_from(&annotations);

        assert_eq!(comments.len(), 1);
        assert!(matches!(comments[0].kind, AnnotationKind::Comment { .. }));
    }

    #[test]
    fn liquid_list_item_parts_use_existing_markers_and_hierarchy() {
        assert_eq!(
            liquid_list_item_parts("1. Performance. Artist agrees to perform."),
            LiquidListItemParts {
                marker: "1.".to_owned(),
                body: "Performance. Artist agrees to perform.",
                indent_level: 0
            }
        );
        assert_eq!(
            liquid_list_item_parts("2.1 Agency is authorized to collect the Artist Fee."),
            LiquidListItemParts {
                marker: "2.1".to_owned(),
                body: "Agency is authorized to collect the Artist Fee.",
                indent_level: 1
            }
        );
        assert_eq!(
            liquid_list_item_parts("- Instagram feed post from Artist"),
            LiquidListItemParts {
                marker: "\u{2022}".to_owned(),
                body: "Instagram feed post from Artist",
                indent_level: 1
            }
        );
        assert_eq!(
            liquid_list_item_parts("(a) First nested condition"),
            LiquidListItemParts {
                marker: "(a)".to_owned(),
                body: "First nested condition",
                indent_level: 2
            }
        );
    }

    #[test]
    fn liquid_note_blocks_collects_footnotes_and_marginalia() {
        let blocks = vec![
            LiquidBlock {
                role: LiquidBlockRole::Paragraph,
                text: "Main text".to_owned(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Footnote,
                text: "1. See Example v. State, 1 U.S. 1 (2026).".to_owned(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Footer,
                text: "Page 2".to_owned(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Marginalia,
                text: "Sidebar note that should not reserve body width.".to_owned(),
                label: Some("Footnote".to_owned()),
            },
            LiquidBlock {
                role: LiquidBlockRole::Footnote,
                text: "2 Additional note.".to_owned(),
                label: None,
            },
        ];

        let notes = liquid_note_blocks(&blocks);

        assert_eq!(notes.len(), 3);
        assert!(notes[0].text.starts_with("1."));
        assert_eq!(notes[1].role, LiquidBlockRole::Marginalia);
        assert!(notes[2].text.starts_with("2 "));
    }

    #[test]
    fn split_liquid_note_marker_parses_numbered_prefix() {
        assert_eq!(
            split_liquid_note_marker("12. See Restatement (Second) of Contracts."),
            (Some("12"), "See Restatement (Second) of Contracts.")
        );
        assert_eq!(
            split_liquid_note_marker("No marker here."),
            (None, "No marker here.")
        );
    }

    #[test]
    fn build_liquid_footnote_index_maps_numbered_notes() {
        let blocks = vec![
            LiquidBlock {
                role: LiquidBlockRole::Paragraph,
                text: "Body text.".to_owned(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Footnote,
                text: "1. First note.".to_owned(),
                label: None,
            },
            LiquidBlock {
                role: LiquidBlockRole::Marginalia,
                text: "2 Second note.".to_owned(),
                label: None,
            },
            // Duplicate number: first occurrence wins.
            LiquidBlock {
                role: LiquidBlockRole::Footnote,
                text: "1. Restarted-section note.".to_owned(),
                label: None,
            },
            // No leading number: skipped.
            LiquidBlock {
                role: LiquidBlockRole::Footnote,
                text: "See supra note 4.".to_owned(),
                label: None,
            },
        ];

        let index = build_liquid_footnote_index(&blocks);

        assert_eq!(index.get(&1).map(String::as_str), Some("First note."));
        assert_eq!(index.get(&2).map(String::as_str), Some("Second note."));
        assert_eq!(index.len(), 2);
    }

    #[test]
    fn reader_correction_record_matches_gold_audit_schema() {
        let record = ReaderCorrectionRecord {
            path: "/docs/example.pdf".to_owned(),
            page_index: 3,
            line_index: 7,
            text: "A source line.".to_owned(),
            gold_role: "footnote".to_owned(),
            source_role: "paragraph".to_owned(),
            action: "marginalia".to_owned(),
            origin: "reader_correction".to_owned(),
            ts: "2026-07-05T00:00:00Z".to_owned(),
        };
        let value: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&record).unwrap()).unwrap();
        // The five canonical human-gold audit columns must all be present.
        for key in ["path", "page_index", "line_index", "text", "gold_role"] {
            assert!(value.get(key).is_some(), "missing audit column {key}");
        }
        assert_eq!(value["gold_role"], "footnote");
        assert_eq!(value["page_index"], 3);
        assert_eq!(value["line_index"], 7);
    }

    #[test]
    fn liquid_reflow_low_confidence_flags_low_yield_docs() {
        let mk = |role: LiquidBlockRole, text: &str| LiquidBlock {
            role,
            text: text.to_owned(),
            label: None,
        };
        // Healthy article: plenty of body text (>1100 bytes) across many blocks -> confident.
        let para =
            "This is a substantial paragraph of body text that a reader can follow. ".repeat(2);
        let healthy_blocks: Vec<_> = (0..20)
            .map(|_| mk(LiquidBlockRole::Paragraph, &para))
            .collect();
        let healthy = test_liquid_document_with_blocks(healthy_blocks);
        assert!(liquid_reflow_low_confidence(&healthy).is_none());

        // Near-empty reflow output (markdown_bytes < 1100) -> flagged.
        let sparse = test_liquid_document_with_blocks(vec![
            mk(LiquidBlockRole::Paragraph, "short"),
            mk(LiquidBlockRole::Header, "running header"),
        ]);
        assert!(liquid_reflow_low_confidence(&sparse).is_some());

        // Empty document -> flagged.
        assert!(liquid_reflow_low_confidence(&test_liquid_document_with_blocks(vec![])).is_some());
    }

    #[test]
    fn liquid_tts_text_orders_body_then_notes_and_skips_furniture() {
        let mk = |role: LiquidBlockRole, text: &str| LiquidBlock {
            role,
            text: text.to_owned(),
            label: None,
        };
        let callout = format!(
            "As held{}12{}, the rule applies.",
            crate::layout_roles::CALLOUT_START,
            crate::layout_roles::CALLOUT_END
        );
        let blocks = vec![
            mk(LiquidBlockRole::Header, "Running header 42"),
            mk(LiquidBlockRole::Paragraph, &callout),
            mk(LiquidBlockRole::Marginalia, "12. See Example v. State."),
            mk(LiquidBlockRole::Noise, "junk"),
        ];
        let doc = test_liquid_document_with_blocks(blocks);

        let with_notes = liquid_tts_text(&doc, true);
        // Furniture skipped, marker callout stripped (no "12" read mid-sentence).
        assert!(with_notes.contains("As held, the rule applies."));
        assert!(!with_notes.contains("Running header"));
        assert!(!with_notes.contains("junk"));
        // Body precedes the separate footnotes pass.
        let body_pos = with_notes.find("As held").unwrap();
        let notes_pos = with_notes.find("Footnotes.").unwrap();
        assert!(body_pos < notes_pos);
        assert!(with_notes.contains("See Example v. State."));

        // Without notes, footnotes are omitted.
        let body_only = liquid_tts_text(&doc, false);
        assert!(!body_only.contains("Footnotes."));
        assert!(!body_only.contains("See Example v. State."));
    }

    #[test]
    fn settled_marker_motion_stays_subtle_and_seeded() {
        let target = 110;
        let first = settled_marker_motion(3.0, 17, target);
        let later = settled_marker_motion(7.0, 17, target);
        let other = settled_marker_motion(3.0, 99, target);

        let allowed_delta = (target as f32 * MARKER_BREATH_AMPLITUDE).ceil() as i16 + 1;
        for motion in [first, later, other] {
            assert!((motion.alpha as i16 - target as i16).abs() <= allowed_delta);
            assert!((0.0..1.0).contains(&motion.sheen_progress));
            assert!(motion.sheen_alpha <= 13);
        }
        assert_ne!(first.sheen_progress, later.sheen_progress);
        assert_ne!(first.sheen_progress, other.sheen_progress);
    }

    #[test]
    fn marker_sheen_only_gently_lifts_color() {
        let lifted = marker_sheen_rgb([0.68, 0.42, 1.0]);
        assert!(lifted[0] > 0.68 && lifted[0] < 0.8);
        assert!(lifted[1] > 0.42 && lifted[1] < 0.7);
        assert_eq!(lifted[2], 1.0);
    }
}
