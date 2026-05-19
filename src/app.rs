use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, unbounded};
use eframe::egui::{
    self, Align, Align2, Color32, Context, CursorIcon, FontData, FontDefinitions, FontFamily,
    FontId, Margin, Pos2, Rect, RichText, Sense, Shadow, Stroke, TextureHandle, TextureId,
    TextureOptions, Vec2,
};
use rfd::FileDialog;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::chat::{CHAT_MODELS, ChatEvent, ChatMessage, ChatRequest, ChatRole, spawn_chat_job};
use crate::liquid::{
    LiquidBlock, LiquidBlockRole, LiquidDocument, LiquidEvent, LiquidRequest, spawn_liquid_job,
};
use crate::model::{
    AnnotationKind, EditorAnnotation, LoadedDocument, MarkerStyle, OcrPageState, PageTextChar,
    PdfRect, RenderedPage, SearchHit, SearchSource, Tool,
};
use crate::ocr::{
    OcrEvent, load_ocr_cache, save_ocr_cache, spawn_ocr_job, spawn_openrouter_ocr_save_job,
};
use crate::pdf_backend::{export_text, save_with_annotations, sidecar_path_for_export};
use crate::render_worker::{
    PageRenderKey, RenderEvent, RenderRequest, ThumbnailRenderKey, spawn_render_worker,
};
use crate::settings::{
    AppSettings, effective_groq_api_key, effective_openrouter_api_key, load_settings, save_settings,
};
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
const THUMBNAIL_SCROLL_SECONDS: f32 = 0.28;
const DOCUMENT_PAGE_GAP: f32 = 24.0;
const PAGE_PREFETCH_RADIUS: usize = 3;
const PAGE_TEXTURE_CACHE_CAP: usize = 32;
const SMALL_DOCUMENT_PREFETCH_LIMIT: usize = 6;
const UPDATE_CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
const UPDATE_RETRY_INTERVAL: Duration = Duration::from_secs(30 * 60);
const UPDATE_NOTICE_DURATION: Duration = Duration::from_secs(4);
const CHAT_CONTEXT_TOKEN_LIMIT: usize = 64_000;
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
    liquid_notice_dismissed: bool,
    liquid_tx: Sender<LiquidEvent>,
    liquid_rx: Receiver<LiquidEvent>,
    chat_tx: Sender<ChatEvent>,
    chat_rx: Receiver<ChatEvent>,
    update_tx: Sender<UpdateEvent>,
    update_rx: Receiver<UpdateEvent>,
    update_state: UpdateUiState,
    update_check_in_flight: bool,
    update_notice: Option<UpdateNotice>,
    next_update_check: Option<Instant>,
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
    active_tool: Tool,
    sidebar_tab: SidebarTab,
    text_selection: Option<TextSelection>,
    selection_anchor: Option<(usize, usize)>,
    selection_toolbar_rect: Option<Rect>,
    selected_text_box: Option<usize>,
    editing_text_box: Option<usize>,
    text_box_focus_request: Option<usize>,
    text_box_action_rect: Option<Rect>,
    text_box_drag: Option<TextBoxDrag>,
    active_drag_page: Option<usize>,
    drag_start_pdf: Option<(f32, f32)>,
    drag_preview: Option<PdfRect>,
    active_signature_stroke: Vec<(f32, f32)>,
    marker_opacity: f32,
    marker_preset_index: usize,
    text_box_text: String,
    signer_name: String,
    pending_select_all_text: bool,
    search_query: String,
    search_focus_request: bool,
    search_hits: Vec<SearchHit>,
    selected_hit: Option<usize>,
    show_search_highlights: bool,
    ocr_states: Vec<OcrPageState>,
    ocr_progress: Option<OcrProgress>,
    chat_state: ChatState,
    ocr_tx: Sender<OcrEvent>,
    ocr_rx: Receiver<OcrEvent>,
    scroll_target_page: Option<usize>,
    thumbnail_scroll_target: Option<usize>,
    pending_document_scroll_offset: Option<Vec2>,
    visible_page_ranges: Vec<VisiblePageRange>,
    settings: AppSettings,
    settings_api_key_edit: String,
    settings_groq_api_key_edit: String,
    show_settings: bool,
    status: String,
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
}

#[derive(Debug, Clone)]
enum LiquidState {
    Idle,
    PreparingText,
    Preparing,
    Ready(LiquidDocument),
    Failed(String),
}

#[derive(Debug, Clone)]
struct ChatState {
    messages: Vec<ChatMessage>,
    input: String,
    model_index: usize,
    in_flight: bool,
    document_context: Option<String>,
    context_estimated_tokens: Option<usize>,
    context_warning: Option<String>,
}

impl Default for ChatState {
    fn default() -> Self {
        Self {
            messages: Vec::new(),
            input: String::new(),
            model_index: 0,
            in_flight: false,
            document_context: None,
            context_estimated_tokens: None,
            context_warning: None,
        }
    }
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
    Failed { message: String, shown_at: Instant },
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
struct MarkerPreset {
    label: &'static str,
    color_rgb: [f32; 3],
    opacity: f32,
    style: MarkerStyle,
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
    liquid_notice_dismissed: bool,
    zoom: f32,
    target_zoom: f32,
    page_textures: HashMap<usize, PageTexture>,
    thumbnail_textures: HashMap<usize, ThumbnailTexture>,
    texture_access_counter: u64,
    last_zoom_change: Option<Instant>,
    annotations: Vec<EditorAnnotation>,
    text_selection: Option<TextSelection>,
    selection_anchor: Option<(usize, usize)>,
    selection_toolbar_rect: Option<Rect>,
    selected_text_box: Option<usize>,
    editing_text_box: Option<usize>,
    text_box_focus_request: Option<usize>,
    text_box_action_rect: Option<Rect>,
    text_box_drag: Option<TextBoxDrag>,
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
        let (chat_tx, chat_rx) = unbounded();
        let (update_tx, update_rx) = unbounded();
        let updates_enabled = updater::updates_enabled();
        if updates_enabled {
            updater::spawn_update_check(update_tx.clone());
        }
        let update_installed = updater::take_installed_update().is_some();
        let update_notice = update_installed
            .then(|| UpdateNotice::new("Update installed", UpdateNoticeKind::Success));
        let initial_status = if update_installed {
            "Update installed."
        } else {
            "Ready"
        };
        let settings = load_settings();
        let settings_api_key_edit = settings.openrouter_api_key.clone();
        let settings_groq_api_key_edit = settings.groq_api_key.clone();
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
            liquid_notice_dismissed: false,
            liquid_tx,
            liquid_rx,
            chat_tx,
            chat_rx,
            update_tx,
            update_rx,
            update_state: UpdateUiState::Idle,
            update_check_in_flight: updates_enabled,
            update_notice,
            next_update_check: None,
            zoom: 1.25,
            target_zoom: 1.25,
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
            active_tool: Tool::Select,
            sidebar_tab: SidebarTab::Pages,
            text_selection: None,
            selection_anchor: None,
            selection_toolbar_rect: None,
            selected_text_box: None,
            editing_text_box: None,
            text_box_focus_request: None,
            text_box_action_rect: None,
            text_box_drag: None,
            active_drag_page: None,
            drag_start_pdf: None,
            drag_preview: None,
            active_signature_stroke: Vec::new(),
            marker_opacity: 0.45,
            marker_preset_index: 0,
            text_box_text: String::new(),
            signer_name: String::new(),
            pending_select_all_text: false,
            search_query: String::new(),
            search_focus_request: false,
            search_hits: Vec::new(),
            selected_hit: None,
            show_search_highlights: true,
            ocr_states: Vec::new(),
            ocr_progress: None,
            chat_state: ChatState::default(),
            ocr_tx,
            ocr_rx,
            scroll_target_page: Some(0),
            thumbnail_scroll_target: Some(0),
            pending_document_scroll_offset: None,
            visible_page_ranges: Vec::new(),
            settings,
            settings_api_key_edit,
            settings_groq_api_key_edit,
            show_settings: false,
            status: initial_status.to_owned(),
        };

        if !startup_paths.is_empty() {
            app.open_paths_in_tabs(startup_paths, &cc.egui_ctx, true);
        }

        if app.tabs.is_empty() {
            if let Ok(path) = std::env::var("LAWPDF_DEFAULT_PDF") {
                if !path.trim().is_empty() {
                    app.status = format!("Opening {}", path);
                    app.load_document(PathBuf::from(path), &cc.egui_ctx);
                }
            }
        }

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
            liquid_notice_dismissed: self.liquid_notice_dismissed,
            zoom: self.zoom,
            target_zoom: self.target_zoom,
            page_textures: self.page_textures.clone(),
            thumbnail_textures: self.thumbnail_textures.clone(),
            texture_access_counter: self.texture_access_counter,
            last_zoom_change: self.last_zoom_change,
            annotations: self.annotations.clone(),
            text_selection: self.text_selection,
            selection_anchor: self.selection_anchor,
            selection_toolbar_rect: self.selection_toolbar_rect,
            selected_text_box: self.selected_text_box,
            editing_text_box: self.editing_text_box,
            text_box_focus_request: self.text_box_focus_request,
            text_box_action_rect: self.text_box_action_rect,
            text_box_drag: self.text_box_drag,
            active_drag_page: self.active_drag_page,
            drag_start_pdf: self.drag_start_pdf,
            drag_preview: self.drag_preview,
            active_signature_stroke: self.active_signature_stroke.clone(),
            pending_select_all_text: self.pending_select_all_text,
            search_query: self.search_query.clone(),
            search_hits: self.search_hits.clone(),
            selected_hit: self.selected_hit,
            show_search_highlights: self.show_search_highlights,
            ocr_states: self.ocr_states.clone(),
            ocr_progress: self.ocr_progress,
            chat_state: self.chat_state.clone(),
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
        self.document = Some(tab.document);
        self.page_index = tab.page_index;
        self.document_epoch = tab.document_epoch;
        self.view_mode = tab.view_mode;
        self.liquid_state = tab.liquid_state;
        self.liquid_notice_dismissed = tab.liquid_notice_dismissed;
        self.zoom = tab.zoom;
        self.target_zoom = tab.target_zoom;
        self.page_textures = tab.page_textures;
        self.thumbnail_textures = tab.thumbnail_textures;
        self.pending_page_renders.clear();
        self.pending_thumbnail_renders.clear();
        self.pending_native_text.clear();
        self.pending_text_chars.clear();
        self.texture_access_counter = tab.texture_access_counter;
        self.last_zoom_change = tab.last_zoom_change;
        self.annotations = tab.annotations;
        self.text_selection = tab.text_selection;
        self.selection_anchor = tab.selection_anchor;
        self.selection_toolbar_rect = tab.selection_toolbar_rect;
        self.selected_text_box = tab.selected_text_box;
        self.editing_text_box = tab.editing_text_box;
        self.text_box_focus_request = tab.text_box_focus_request;
        self.text_box_action_rect = tab.text_box_action_rect;
        self.text_box_drag = tab.text_box_drag;
        self.active_drag_page = tab.active_drag_page;
        self.drag_start_pdf = tab.drag_start_pdf;
        self.drag_preview = tab.drag_preview;
        self.active_signature_stroke = tab.active_signature_stroke;
        self.pending_select_all_text = tab.pending_select_all_text;
        self.search_query = tab.search_query;
        self.search_hits = tab.search_hits;
        self.selected_hit = tab.selected_hit;
        self.show_search_highlights = tab.show_search_highlights;
        self.ocr_states = tab.ocr_states;
        self.ocr_progress = tab.ocr_progress;
        self.chat_state = tab.chat_state;
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
        self.document = None;
        self.page_index = 0;
        self.document_epoch = 0;
        self.view_mode = DocumentViewMode::Pdf;
        self.liquid_state = LiquidState::Idle;
        self.liquid_notice_dismissed = false;
        self.zoom = 1.25;
        self.target_zoom = 1.25;
        self.page_textures.clear();
        self.thumbnail_textures.clear();
        self.pending_page_renders.clear();
        self.pending_thumbnail_renders.clear();
        self.pending_native_text.clear();
        self.pending_text_chars.clear();
        self.texture_access_counter = 0;
        self.last_zoom_change = None;
        self.annotations.clear();
        self.text_selection = None;
        self.selection_anchor = None;
        self.selection_toolbar_rect = None;
        self.clear_text_box_selection();
        self.text_box_drag = None;
        self.active_drag_page = None;
        self.drag_start_pdf = None;
        self.drag_preview = None;
        self.active_signature_stroke.clear();
        self.pending_select_all_text = false;
        self.search_query.clear();
        self.search_focus_request = false;
        self.search_hits.clear();
        self.selected_hit = None;
        self.show_search_highlights = true;
        self.ocr_states.clear();
        self.ocr_progress = None;
        self.chat_state = ChatState::default();
        self.scroll_target_page = Some(0);
        self.thumbnail_scroll_target = Some(0);
        self.visible_page_ranges.clear();
        self.status = "Ready".to_owned();
    }

    fn tab_index_for_path(&self, path: &Path) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.document.path == path)
    }

    fn open_dialog(&mut self, ctx: &Context) {
        if let Some(paths) = FileDialog::new().add_filter("PDF", &["pdf"]).pick_files() {
            self.open_paths_in_tabs(paths, ctx, true);
        }
    }

    fn load_document(&mut self, path: PathBuf, ctx: &Context) {
        self.load_document_with_options(path, ctx, true, true, true);
    }

    fn open_paths_in_tabs(&mut self, paths: Vec<PathBuf>, ctx: &Context, defer_background: bool) {
        let mut paths = clean_pdf_paths(paths);
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
        while let Ok(event) = self.update_rx.try_recv() {
            match event {
                UpdateEvent::Checking => {
                    self.update_check_in_flight = true;
                    if !self.update_state.has_ready_update() {
                        self.update_state = UpdateUiState::Checking;
                    }
                }
                UpdateEvent::Detected { version } => {
                    self.update_check_in_flight = true;
                    self.update_state = UpdateUiState::Downloading;
                    self.update_notice = Some(UpdateNotice::new(
                        "New update detected, updating in background",
                        UpdateNoticeKind::Working,
                    ));
                    self.status = format!("Downloading LawPDF {version} in the background.");
                    ctx.request_repaint();
                }
                UpdateEvent::NotAvailable => {
                    self.update_check_in_flight = false;
                    self.next_update_check = Some(Instant::now() + UPDATE_CHECK_INTERVAL);
                    if matches!(self.update_state, UpdateUiState::Checking) {
                        self.update_state = UpdateUiState::Idle;
                    }
                }
                UpdateEvent::Downloading => {
                    self.update_check_in_flight = true;
                    self.update_state = UpdateUiState::Downloading;
                }
                UpdateEvent::Ready(pending) => {
                    self.update_check_in_flight = false;
                    self.next_update_check = Some(Instant::now() + UPDATE_CHECK_INTERVAL);
                    self.status = format!(
                        "LawPDF {} is ready and will install on next launch.",
                        pending.version
                    );
                    self.update_state = UpdateUiState::Ready;
                    ctx.request_repaint();
                }
                UpdateEvent::Failed(message) => {
                    self.update_check_in_flight = false;
                    self.next_update_check = Some(Instant::now() + UPDATE_RETRY_INTERVAL);
                    self.status = message.clone();
                    if matches!(
                        self.update_state,
                        UpdateUiState::Checking | UpdateUiState::Downloading
                    ) {
                        self.update_state = UpdateUiState::Failed {
                            message,
                            shown_at: Instant::now(),
                        };
                    }
                }
            }
        }

        if let UpdateUiState::Failed { message, shown_at } = &self.update_state {
            if shown_at.elapsed() < Duration::from_secs(12) {
                self.status = message.clone();
                ctx.request_repaint_after(Duration::from_secs(1));
            } else {
                self.update_state = UpdateUiState::Idle;
            }
        }

        if self
            .update_notice
            .as_ref()
            .is_some_and(UpdateNotice::is_expired)
        {
            self.update_notice = None;
            ctx.request_repaint();
        } else if self.update_notice.is_some() {
            ctx.request_repaint_after(Duration::from_millis(250));
        }

        if updater::updates_enabled()
            && !self.update_check_in_flight
            && !self.update_state.is_busy()
            && !self.update_state.has_ready_update()
            && self
                .next_update_check
                .is_some_and(|next_check| Instant::now() >= next_check)
        {
            self.update_check_in_flight = true;
            self.next_update_check = None;
            updater::spawn_update_check(self.update_tx.clone());
        }

        if matches!(self.update_state, UpdateUiState::Downloading) {
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
                self.status = error;
                false
            }
        }
    }

    fn tab_for_new_document(&mut self, document: LoadedDocument) -> DocumentTab {
        let page_count = document.page_count;
        let title = document.title.clone();
        let ocr_states = load_ocr_cache(&document.path, page_count)
            .unwrap_or_else(|| vec![OcrPageState::Idle; page_count]);
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
            liquid_notice_dismissed: false,
            zoom: 1.25,
            target_zoom: 1.25,
            page_textures: HashMap::new(),
            thumbnail_textures: HashMap::new(),
            texture_access_counter: 0,
            last_zoom_change: None,
            annotations: Vec::new(),
            text_selection: None,
            selection_anchor: None,
            selection_toolbar_rect: None,
            selected_text_box: None,
            editing_text_box: None,
            text_box_focus_request: None,
            text_box_action_rect: None,
            text_box_drag: None,
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
            status: if cached_ocr_pages > 0 {
                format!("Opened {title}; restored OCR for {cached_ocr_pages} page(s).")
            } else {
                format!("Opened {title}")
            },
        }
    }

    fn save_as_dialog(&mut self) {
        let Some(document) = self.document.as_ref() else {
            return;
        };

        let file_name = default_output_name(&document.path, "edited", "pdf");
        if let Some(destination) = FileDialog::new()
            .add_filter("PDF", &["pdf"])
            .set_file_name(file_name)
            .save_file()
        {
            match save_with_annotations(&document.path, &destination, &self.annotations) {
                Ok(()) => self.status = format!("Saved {}", destination.display()),
                Err(error) => self.status = error.to_string(),
            }
        }
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

        if let Some(destination) = FileDialog::new()
            .add_filter("Text", &["txt"])
            .set_file_name(file_name)
            .save_file()
        {
            let ocr_text = self.collect_ocr_text();
            match export_text(&destination, document, &ocr_text) {
                Ok(()) => self.status = format!("Exported {}", destination.display()),
                Err(error) => self.status = error.to_string(),
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

        if let Some(destination) = FileDialog::new()
            .add_filter("PNG", &["png"])
            .set_file_name(file_name)
            .save_file()
        {
            match self.export_page_png_on_worker(path, page_index, destination.clone(), 2.0) {
                Ok(()) => self.status = format!("Exported {}", destination.display()),
                Err(error) => self.status = error,
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
            self.show_settings = true;
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

        let Some(destination) = FileDialog::new()
            .add_filter("PDF", &["pdf"])
            .set_file_name(file_name)
            .save_file()
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

    fn poll_ocr(&mut self) {
        let mut should_rebuild_search = false;

        while let Ok(event) = self.ocr_rx.try_recv() {
            if let Some(status) = event.status.as_ref() {
                self.status = status.clone();
            }
            if self.is_current_document(event.document_epoch, &event.path) {
                if let Some(state) = self.ocr_states.get_mut(event.page_index) {
                    let was_done = matches!(event.state, OcrPageState::Done(_));
                    *state = event.state;
                    should_rebuild_search |= was_done && !self.search_query.trim().is_empty();
                    if was_done {
                        if let Some(document) = self.document.as_ref() {
                            let _ = save_ocr_cache(&document.path, &self.ocr_states);
                        }
                        if self.chat_state.messages.is_empty() {
                            self.chat_state.document_context = None;
                            self.chat_state.context_estimated_tokens = None;
                            self.chat_state.context_warning = None;
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
                        let _ = save_ocr_cache(&tab.document.path, &tab.ocr_states);
                        if tab.chat_state.messages.is_empty() {
                            tab.chat_state.document_context = None;
                            tab.chat_state.context_estimated_tokens = None;
                            tab.chat_state.context_warning = None;
                        }
                    }
                }
            }
        }

        if should_rebuild_search {
            self.rebuild_search();
        }
    }

    fn send_chat_message(&mut self, ctx: &Context) {
        if self.document.is_none() {
            return;
        }
        if self.chat_state.in_flight {
            return;
        }

        let user_message = self.chat_state.input.trim().to_owned();
        if user_message.is_empty() {
            return;
        }

        let Some(api_key) = effective_openrouter_api_key(&self.settings) else {
            self.status = "Chat needs an OpenRouter API key.".to_owned();
            self.show_settings = true;
            return;
        };

        if self.chat_state.messages.is_empty() && self.chat_state.document_context.is_none() {
            if !self.ensure_native_text_loaded_for_all(ctx, "Preparing PDF text for chat") {
                self.status = "Preparing PDF text for chat; send again when ready.".to_owned();
                return;
            }

            let Some(document) = self.document.as_ref() else {
                return;
            };
            let (context, estimated_tokens) = self.chat_context_for_document(document);
            self.chat_state.context_estimated_tokens = Some(estimated_tokens);
            if context.trim().is_empty() {
                self.status = "Chat needs PDF text or OCR text first.".to_owned();
                return;
            }
            if estimated_tokens <= CHAT_CONTEXT_TOKEN_LIMIT {
                self.chat_state.document_context = Some(context);
                self.chat_state.context_warning = None;
            } else {
                self.chat_state.context_warning = Some(format!(
                    "PDF text is about {estimated_tokens} tokens, so it was not attached to the first chat message."
                ));
            }
        }

        self.chat_state.messages.push(ChatMessage {
            role: ChatRole::User,
            content: user_message,
        });
        self.chat_state.input.clear();
        self.chat_state.in_flight = true;

        let Some(document) = self.document.as_ref() else {
            return;
        };
        let document_path = document.path.clone();
        let model = CHAT_MODELS
            .get(self.chat_state.model_index)
            .unwrap_or(&CHAT_MODELS[0])
            .id
            .to_owned();
        spawn_chat_job(
            ChatRequest {
                document_epoch: self.document_epoch,
                path: document_path,
                api_key,
                model,
                visible_messages: self.chat_state.messages.clone(),
                document_context: self.chat_state.document_context.clone(),
            },
            self.chat_tx.clone(),
        );
        self.status = "Chat request sent.".to_owned();
    }

    fn poll_chat_results(&mut self, ctx: &Context) {
        while let Ok(event) = self.chat_rx.try_recv() {
            if self.is_current_document(event.document_epoch, &event.path) {
                self.chat_state.in_flight = false;
                match event.result {
                    Ok(content) => {
                        self.chat_state.messages.push(ChatMessage {
                            role: ChatRole::Assistant,
                            content,
                        });
                        self.status = "Chat response ready.".to_owned();
                    }
                    Err(error) => {
                        self.chat_state.messages.push(ChatMessage {
                            role: ChatRole::Assistant,
                            content: format!("Chat failed: {error}"),
                        });
                        self.status = error;
                    }
                }
                ctx.request_repaint();
            } else if let Some(tab) = self.tabs.iter_mut().find(|tab| {
                tab.document_epoch == event.document_epoch && tab.document.path == event.path
            }) {
                tab.chat_state.in_flight = false;
                match event.result {
                    Ok(content) => {
                        tab.chat_state.messages.push(ChatMessage {
                            role: ChatRole::Assistant,
                            content,
                        });
                        tab.status = "Chat response ready.".to_owned();
                    }
                    Err(error) => {
                        tab.chat_state.messages.push(ChatMessage {
                            role: ChatRole::Assistant,
                            content: format!("Chat failed: {error}"),
                        });
                        tab.status = error;
                    }
                }
            }
        }
    }

    fn chat_context_for_document(&self, document: &LoadedDocument) -> (String, usize) {
        let pages = self.collect_liquid_source_pages(document);
        let mut context = String::new();
        for (page_index, page) in pages.iter().enumerate() {
            let text = page.trim();
            if text.is_empty() {
                continue;
            }
            context.push_str(&format!("\n\n--- Page {} ---\n", page_index + 1));
            context.push_str(text);
        }
        let estimated_tokens = estimate_tokens(&context);
        (context, estimated_tokens)
    }

    fn chat_context_summary(&self) -> (usize, usize) {
        let Some(document) = self.document.as_ref() else {
            return (0, 0);
        };
        let pages = self.collect_liquid_source_pages(document);
        let mut page_count = 0usize;
        let mut chars = 0usize;
        for page in pages {
            let text = page.trim();
            if !text.is_empty() {
                page_count += 1;
                chars += text.chars().count();
            }
        }
        (page_count, (chars / 4).max(1))
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
                            self.status = error;
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
                            self.status = error;
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
                            self.status = error;
                            if let Some(document) = self.document.as_mut() {
                                if let Some(slot) = document.text_chars.get_mut(page_index) {
                                    *slot = Some(Vec::new());
                                }
                            }
                        }
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
                            if !self.search_query.trim().is_empty() {
                                self.rebuild_search();
                            }
                            ctx.request_repaint();
                        }
                        Err(error) => {
                            self.status = error;
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
                self.status = error;
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
                if !native.is_empty() {
                    native.to_owned()
                } else {
                    self.ocr_states
                        .get(page_index)
                        .and_then(OcrPageState::text)
                        .unwrap_or_default()
                        .to_owned()
                }
            })
            .collect()
    }

    fn set_view_mode(&mut self, mode: DocumentViewMode, ctx: &Context) {
        if self.view_mode == mode {
            if mode == DocumentViewMode::Liquid {
                self.ensure_liquid_started(ctx);
            }
            return;
        }
        self.view_mode = mode;
        match mode {
            DocumentViewMode::Pdf => {
                self.status = "PDF view".to_owned();
            }
            DocumentViewMode::Liquid => {
                self.ensure_liquid_started(ctx);
            }
        }
        ctx.request_repaint();
    }

    fn start_llm_liquid_mode(&mut self, ctx: &Context) {
        if self.document.is_none() {
            self.status = "Open a PDF before starting LLM Liquid Mode.".to_owned();
            return;
        }

        if matches!(
            self.liquid_state,
            LiquidState::Failed(_) | LiquidState::Ready(_)
        ) {
            self.liquid_state = LiquidState::Idle;
        }
        self.liquid_notice_dismissed = false;
        self.ensure_liquid_started(ctx);
        ctx.request_repaint();
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

        if !self.ensure_native_text_loaded_for_all(ctx, "Preparing PDF text for Liquid Mode") {
            self.liquid_state = LiquidState::PreparingText;
            self.liquid_notice_dismissed = false;
            self.status = "Preparing PDF text for Liquid Mode...".to_owned();
            return;
        }

        let Some(document) = self.document.as_ref() else {
            return;
        };
        let pages = self.collect_liquid_source_pages(document);
        let has_text = pages.iter().any(|page| !page.trim().is_empty());
        if !has_text {
            self.liquid_state = LiquidState::Failed(
                "Liquid Mode needs native PDF text or completed OCR text.".to_owned(),
            );
            self.status = "Liquid Mode has no text to format.".to_owned();
            return;
        }

        let request = LiquidRequest {
            document_epoch: self.document_epoch,
            path: document.path.clone(),
            title: document.title.clone(),
            pages,
            groq_api_key: effective_groq_api_key(&self.settings),
            openrouter_api_key: effective_openrouter_api_key(&self.settings),
        };
        self.liquid_state = LiquidState::Preparing;
        self.liquid_notice_dismissed = false;
        self.status = "Preparing Liquid Mode...".to_owned();
        spawn_liquid_job(request, self.liquid_tx.clone());
        ctx.request_repaint_after(RENDER_POLL_INTERVAL);
    }

    fn poll_liquid_results(&mut self, ctx: &Context) {
        while let Ok(event) = self.liquid_rx.try_recv() {
            let next_state = match event.result {
                Ok(document) => {
                    let status = if document.llm_used {
                        format!(
                            "Liquid Mode ready; {} footnote line(s) removed.",
                            document.footnotes_removed
                        )
                    } else {
                        format!(
                            "Liquid Mode ready locally; {} footnote line(s) removed.",
                            document.footnotes_removed
                        )
                    };
                    (LiquidState::Ready(document), status)
                }
                Err(error) => (LiquidState::Failed(error.clone()), error),
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
        }
    }

    fn start_search(&mut self, ctx: &Context) {
        if !self.search_query.trim().is_empty()
            && !self.ensure_native_text_loaded_for_all(ctx, "Preparing searchable PDF text")
        {
            self.rebuild_search();
            return;
        }
        self.rebuild_search();
    }

    fn focus_search(&mut self, ctx: &Context) {
        self.sidebar_tab = SidebarTab::Search;
        self.search_focus_request = true;
        if self.document.is_none() {
            self.status = "Open a PDF to search.".to_owned();
        }
        ctx.request_repaint();
    }

    fn rebuild_search(&mut self) {
        self.search_hits.clear();
        self.selected_hit = None;

        let Some(document) = self.document.as_ref() else {
            return;
        };

        let query = self.search_query.trim();
        if query.is_empty() {
            return;
        }

        for page_index in 0..document.page_count {
            if let Some(text) = document.native_text.get(page_index) {
                self.search_hits.extend(find_hits(
                    text,
                    query,
                    page_index,
                    SearchSource::NativeText,
                ));
            }

            if let Some(text) = self.ocr_states.get(page_index).and_then(OcrPageState::text) {
                self.search_hits
                    .extend(find_hits(text, query, page_index, SearchSource::OcrText));
            }
        }

        if let Some(first) = self.search_hits.first() {
            self.selected_hit = Some(0);
            self.page_index = first.page_index;
            self.scroll_target_page = Some(first.page_index);
            self.thumbnail_scroll_target = Some(first.page_index);
        }

        self.status = format!("{} match(es)", self.search_hits.len());
    }

    fn add_search_highlights(&mut self) {
        let Some(document) = self.document.as_ref() else {
            return;
        };

        let preset = self.marker_preset();
        let mut added = 0usize;
        for hit in &self.search_hits {
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
        let key = PageRenderKey::new(self.document_epoch, page_index, self.zoom, render_scale);
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
            self.status = "PDF render worker stopped.".to_owned();
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
            self.status = "PDF render worker stopped.".to_owned();
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
            self.status = "PDF render worker stopped.".to_owned();
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
            self.status = "PDF render worker stopped.".to_owned();
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

    fn set_zoom(&mut self, zoom: f32) {
        let zoom = zoom.clamp(0.35, 5.0);
        if (self.target_zoom - zoom).abs() > f32::EPSILON {
            self.target_zoom = zoom;
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

    fn select_text_box(&mut self, annotation_index: usize) {
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
                            .on_hover_text("Save edited copy")
                            .clicked()
                        {
                            self.save_as_dialog();
                        }
                        ui.add_enabled_ui(has_document, |ui| {
                            ui.menu_button("Export", |ui| {
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
                            self.set_view_mode(DocumentViewMode::Pdf, ctx);
                        }
                        let liquid_active = self.view_mode == DocumentViewMode::Liquid;
                        if ui
                            .add_enabled(
                                has_document,
                                egui::Button::new("Liquid").selected(liquid_active),
                            )
                            .on_hover_text("Reflowed reading view")
                            .clicked()
                        {
                            self.set_view_mode(DocumentViewMode::Liquid, ctx);
                        }
                        let llm_busy = matches!(
                            self.liquid_state,
                            LiquidState::PreparingText | LiquidState::Preparing
                        );
                        let llm_text = match self.liquid_state {
                            LiquidState::PreparingText => "LLM: text...",
                            LiquidState::Preparing => "LLM: sent...",
                            LiquidState::Ready(_) => "LLM ready",
                            LiquidState::Failed(_) => "LLM retry",
                            LiquidState::Idle => "LLM",
                        };
                        let llm_response = ui
                            .add_enabled(!llm_busy && has_document, egui::Button::new(llm_text))
                            .on_hover_text(match self.liquid_state {
                                LiquidState::PreparingText => {
                                    "Extracting PDF text before sending the LLM request"
                                }
                                LiquidState::Preparing => {
                                    "LLM request sent; waiting for OpenRouter"
                                }
                                _ => "Run LLM-powered Liquid Mode with the beta OpenRouter key",
                            });
                        if llm_response.clicked() {
                            self.start_llm_liquid_mode(ctx);
                        }
                        if llm_busy {
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
                            egui::TextEdit::singleline(&mut self.search_query)
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
                egui::TextEdit::singleline(&mut self.search_query)
                    .hint_text("Find")
                    .desired_width(168.0),
            );
            if self.search_focus_request {
                if has_document {
                    search_response.request_focus();
                }
                self.search_focus_request = false;
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
            ui.checkbox(&mut self.show_search_highlights, "show hits");
            if ui
                .add_enabled(
                    has_document && !self.search_hits.is_empty(),
                    egui::Button::new("Annotate"),
                )
                .clicked()
            {
                self.add_search_highlights();
            }
        });

        ui.add_space(6.0);
        ui.label(RichText::new(format!("{} result(s)", self.search_hits.len())).color(MUTED_INK));
        egui::ScrollArea::vertical()
            .max_height(280.0)
            .show(ui, |ui| {
                let hits = self.search_hits.clone();
                for (index, hit) in hits.iter().enumerate() {
                    let label = format!(
                        "p{} [{}] {}",
                        hit.page_index + 1,
                        hit.source.label(),
                        hit.snippet
                    );
                    if ui
                        .selectable_label(self.selected_hit == Some(index), label)
                        .clicked()
                    {
                        self.selected_hit = Some(index);
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

    fn draw_chat_tab(&mut self, ui: &mut egui::Ui) {
        let has_document = self.document.is_some();
        if !has_document {
            ui.label(RichText::new("No PDF loaded").color(MUTED_INK));
            return;
        }

        ui.horizontal(|ui| {
            ui.label(RichText::new("Model").color(INK));
            let selected = CHAT_MODELS
                .get(self.chat_state.model_index)
                .unwrap_or(&CHAT_MODELS[0]);
            egui::ComboBox::from_id_salt("chat_model_selector")
                .selected_text(selected.label)
                .width(180.0)
                .show_ui(ui, |ui| {
                    for (index, model) in CHAT_MODELS.iter().enumerate() {
                        ui.selectable_value(&mut self.chat_state.model_index, index, model.label)
                            .on_hover_text(model.id);
                    }
                });
        });

        let (context_pages, context_estimate) = self.chat_context_summary();
        let context_status = if let Some(tokens) = self.chat_state.context_estimated_tokens {
            if self.chat_state.document_context.is_some() {
                format!("PDF context attached: ~{tokens} tokens")
            } else {
                format!("PDF context not attached: ~{tokens} tokens")
            }
        } else {
            format!("PDF text available: {context_pages} page(s), ~{context_estimate} tokens")
        };
        ui.label(RichText::new(context_status).color(MUTED_INK));
        if let Some(warning) = self.chat_state.context_warning.as_ref() {
            ui.label(RichText::new(warning).color(Color32::from_rgb(134, 92, 34)));
        }

        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !self.chat_state.messages.is_empty() || self.chat_state.in_flight,
                    egui::Button::new("Clear"),
                )
                .clicked()
            {
                self.chat_state = ChatState {
                    model_index: self.chat_state.model_index,
                    ..ChatState::default()
                };
            }
            if self.chat_state.in_flight {
                ui.spinner();
            }
        });

        ui.add_space(8.0);
        egui::ScrollArea::vertical()
            .id_salt("chat_messages")
            .max_height(340.0)
            .stick_to_bottom(true)
            .show(ui, |ui| {
                if self.chat_state.messages.is_empty() {
                    ui.label(RichText::new("Ask a question about this PDF.").color(MUTED_INK));
                }
                for message in &self.chat_state.messages {
                    let (label, color) = match message.role {
                        ChatRole::User => ("You", Color32::from_rgb(93, 68, 37)),
                        ChatRole::Assistant => ("LawPDF", INK),
                    };
                    ui.label(RichText::new(label).strong().color(color));
                    egui::Frame::NONE
                        .fill(if message.role == ChatRole::User {
                            Color32::from_rgb(247, 243, 235)
                        } else {
                            Color32::from_rgb(255, 254, 250)
                        })
                        .stroke(Stroke::new(1.0, Color32::from_rgb(222, 216, 205)))
                        .corner_radius(6)
                        .inner_margin(Margin::symmetric(8, 6))
                        .show(ui, |ui| {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&message.content).size(14.0).color(INK),
                                )
                                .wrap(),
                            );
                        });
                    ui.add_space(8.0);
                }
            });

        ui.add_space(8.0);
        ui.add(
            egui::TextEdit::multiline(&mut self.chat_state.input)
                .hint_text("Ask about the PDF")
                .desired_rows(4)
                .lock_focus(true),
        );
        if ui
            .add_enabled(!self.chat_state.in_flight, egui::Button::new("Send"))
            .clicked()
        {
            self.send_chat_message(ui.ctx());
        }
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

    fn draw_settings_window(&mut self, ctx: &Context) {
        if !self.show_settings {
            return;
        }

        let mut open = self.show_settings;
        let mut save_clicked = false;
        egui::Window::new("LawPDF settings")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(RichText::new("Groq").strong().color(INK));
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings_groq_api_key_edit)
                        .password(true)
                        .hint_text("API key")
                        .desired_width(360.0),
                );
                ui.add_space(8.0);
                ui.label(RichText::new("OpenRouter").strong().color(INK));
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings_api_key_edit)
                        .password(true)
                        .hint_text("API key")
                        .desired_width(360.0),
                );
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        save_clicked = true;
                    }
                    if ui.button("Cancel").clicked() {
                        self.settings_api_key_edit = self.settings.openrouter_api_key.clone();
                        self.settings_groq_api_key_edit = self.settings.groq_api_key.clone();
                        self.show_settings = false;
                    }
                });
            });

        self.show_settings = open && self.show_settings;
        if save_clicked {
            self.settings.openrouter_api_key = self.settings_api_key_edit.trim().to_owned();
            self.settings.groq_api_key = self.settings_groq_api_key_edit.trim().to_owned();
            match save_settings(&self.settings) {
                Ok(()) => {
                    self.show_settings = false;
                    self.liquid_state = LiquidState::Idle;
                    self.status = "Settings saved.".to_owned();
                }
                Err(error) => self.status = error,
            }
        }
    }

    fn draw_update_notice(&mut self, ctx: &Context) {
        let Some(notice) = self.update_notice.as_ref() else {
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
                                    ui.label(RichText::new("Preparing Liquid Mode").strong());
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
                                let engine = if document.llm_used {
                                    "OpenRouter"
                                } else {
                                    "local fallback"
                                };
                                ui.label(RichText::new("Liquid Mode Ready").strong().color(INK));
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
                                    RichText::new("Liquid Mode Failed")
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
                let width = ui.available_width().min(920.0);
                let side = ((ui.available_width() - width) * 0.5).max(0.0);
                ui.horizontal(|ui| {
                    ui.add_space(side);
                    ui.vertical(|ui| {
                        ui.set_width(width);
                        ui.add_space(28.0);
                        match state {
                            LiquidState::Idle => {
                                ui.label(RichText::new("Liquid Mode").size(26.0).strong());
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
                                        .color(MUTED_INK),
                                );
                            }
                            LiquidState::Preparing => {
                                ui.add_space(80.0);
                                ui.spinner();
                                ui.label(RichText::new("Preparing Liquid Mode").size(20.0));
                                ui.label(RichText::new("Formatting text...").color(MUTED_INK));
                            }
                            LiquidState::Failed(error) => {
                                ui.add_space(48.0);
                                ui.label(
                                    RichText::new("Liquid Mode unavailable")
                                        .size(22.0)
                                        .strong()
                                        .color(Color32::from_rgb(132, 49, 42)),
                                );
                                ui.label(RichText::new(error).color(MUTED_INK));
                                if ui.button("Retry").clicked() {
                                    self.liquid_state = LiquidState::Idle;
                                    self.ensure_liquid_started(ctx);
                                }
                            }
                            LiquidState::Ready(document) => {
                                self.draw_liquid_header(ui, &document);
                                for block in &document.blocks {
                                    self.draw_liquid_block(ui, block);
                                }
                                ui.add_space(40.0);
                            }
                        }
                    });
                });
            });
    }

    fn draw_liquid_header(&self, ui: &mut egui::Ui, document: &LiquidDocument) {
        ui.horizontal_wrapped(|ui| {
            let engine = if document.llm_used {
                "OpenRouter"
            } else {
                "Local"
            };
            ui.label(
                RichText::new(format!("Liquid Mode · {engine}"))
                    .strong()
                    .color(INK),
            );
            ui.separator();
            ui.label(
                RichText::new(format!(
                    "{} footnote line(s) removed",
                    document.footnotes_removed
                ))
                .color(MUTED_INK),
            );
        });
        for warning in &document.warnings {
            ui.label(RichText::new(warning).color(Color32::from_rgb(134, 92, 34)));
        }
        ui.add_space(12.0);
    }

    fn draw_liquid_block(&self, ui: &mut egui::Ui, block: &LiquidBlock) {
        match block.role {
            LiquidBlockRole::Title => {
                ui.add_space(8.0);
                ui.add(
                    egui::Label::new(RichText::new(&block.text).size(30.0).strong().color(INK))
                        .wrap(),
                );
                ui.add_space(12.0);
            }
            LiquidBlockRole::Heading => {
                ui.add_space(18.0);
                ui.add(
                    egui::Label::new(RichText::new(&block.text).size(24.0).strong().color(INK))
                        .wrap(),
                );
                ui.add_space(4.0);
            }
            LiquidBlockRole::Subheading => {
                ui.add_space(12.0);
                ui.add(
                    egui::Label::new(RichText::new(&block.text).size(19.0).strong().color(INK))
                        .wrap(),
                );
                ui.add_space(2.0);
            }
            LiquidBlockRole::Definition => {
                self.draw_liquid_marginalia_block(
                    ui,
                    block.label.as_deref().unwrap_or("Definition"),
                    &block.text,
                    Color32::from_rgb(116, 77, 30),
                );
            }
            LiquidBlockRole::KeyClause => {
                self.draw_liquid_marginalia_block(
                    ui,
                    block.label.as_deref().unwrap_or("Key clause"),
                    &block.text,
                    Color32::from_rgb(53, 95, 58),
                );
            }
            LiquidBlockRole::Clause => {
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(8.0);
                    ui.add(egui::Label::new(RichText::new(&block.text).size(17.0)).wrap());
                });
                ui.add_space(7.0);
            }
            LiquidBlockRole::ListItem => {
                ui.horizontal_top(|ui| {
                    ui.label(RichText::new("•").size(18.0).strong().color(MUTED_INK));
                    ui.add(egui::Label::new(RichText::new(&block.text).size(17.0)).wrap());
                });
                ui.add_space(4.0);
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
                                .size(17.0)
                                .italics()
                                .color(Color32::from_rgb(69, 63, 55)),
                        )
                        .wrap(),
                    );
                });
                ui.add_space(6.0);
            }
            LiquidBlockRole::Paragraph => {
                ui.add(egui::Label::new(RichText::new(&block.text).size(17.0).color(INK)).wrap());
                ui.add_space(8.0);
            }
            LiquidBlockRole::Abstract => {
                egui::Frame::NONE
                    .fill(Color32::from_rgb(240, 244, 250))
                    .stroke(Stroke::new(1.0, Color32::from_rgb(176, 196, 222)))
                    .corner_radius(6)
                    .inner_margin(Margin::symmetric(12, 9))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new("Abstract")
                                .strong()
                                .color(Color32::from_rgb(50, 80, 130)),
                        );
                        ui.add(egui::Label::new(RichText::new(&block.text).size(17.0)).wrap());
                    });
                ui.add_space(8.0);
            }
            LiquidBlockRole::AuthorInfo => {
                ui.add_space(4.0);
                ui.add(
                    egui::Label::new(
                        RichText::new(&block.text)
                            .size(15.0)
                            .italics()
                            .color(MUTED_INK),
                    )
                    .wrap(),
                );
                ui.add_space(4.0);
            }
            LiquidBlockRole::Metadata => {
                self.draw_liquid_marginalia_block(
                    ui,
                    "Context",
                    &compact_exam_metadata(&block.text),
                    MUTED_INK,
                );
            }
            LiquidBlockRole::Header | LiquidBlockRole::Footer | LiquidBlockRole::Footnote => {}
            LiquidBlockRole::SectionBreak => {
                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    ui.add_space((ui.available_width() - 120.0).max(0.0) / 2.0);
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(120.0, 1.0), egui::Sense::hover());
                    ui.painter().hline(
                        rect.x_range(),
                        rect.center().y,
                        Stroke::new(1.0, MUTED_INK),
                    );
                });
                ui.add_space(16.0);
            }
        }
    }

    fn draw_liquid_marginalia_block(
        &self,
        ui: &mut egui::Ui,
        label: &str,
        text: &str,
        label_color: Color32,
    ) {
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                Vec2::new(92.0, 18.0),
                egui::Layout::top_down(Align::RIGHT),
                |ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new(label).size(11.0).strong().color(label_color),
                        )
                        .wrap(),
                    );
                },
            );
            ui.add_space(10.0);
            ui.add(
                egui::Label::new(RichText::new(text).size(17.0).color(INK))
                    .wrap()
                    .selectable(true),
            );
        });
        ui.add_space(8.0);
    }

    fn draw_document(&mut self, ctx: &Context) {
        self.text_box_action_rect = None;
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

                                let new_zoom = (self.target_zoom * zoom_delta).clamp(0.35, 5.0);
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
                        let response = ui
                            .interact(
                                rect,
                                ui.id().with(("document-page", page_index)),
                                Sense::click_and_drag(),
                            )
                            .on_hover_cursor(tool_cursor(self.active_tool));
                        let placement = PagePlacement {
                            rect,
                            page_width: page_info.width,
                            page_height: page_info.height,
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
                        self.draw_text_selection(&painter, &placement, page_index);
                        self.draw_annotations(&painter, &placement, page_index);
                        self.draw_drag_preview(&painter, &placement, page_index);
                        let mut text_box_interacted =
                            self.draw_text_box_controls(ui, ctx, &placement, page_index);
                        text_box_interacted |=
                            self.draw_text_box_action_palette(ctx, &placement, page_index);
                        self.handle_page_interaction(
                            &response,
                            &placement,
                            page_index,
                            text_box_interacted,
                        );
                        self.draw_selection_context_menu(ctx, &response, page_index);
                        self.draw_selection_action_palette(ctx, &placement, page_index);
                    }

                    self.visible_page_ranges = visible_page_ranges;

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
        if !self.show_search_highlights {
            return;
        }

        let Some(document) = self.document.as_ref() else {
            return;
        };

        for hit in self
            .search_hits
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

    fn draw_annotations(
        &self,
        painter: &egui::Painter,
        placement: &PagePlacement,
        page_index: usize,
    ) {
        for annotation in self
            .annotations
            .iter()
            .filter(|annotation| annotation.page_index == page_index)
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
                            painter.rect_filled(rect, 0.0, color);
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

    fn handle_page_interaction(
        &mut self,
        response: &egui::Response,
        placement: &PagePlacement,
        page_index: usize,
        text_box_interacted: bool,
    ) {
        if text_box_interacted {
            return;
        }

        if response.clicked() && !self.click_hits_text_box_action(response) {
            self.clear_text_box_selection();
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
                        self.selection_anchor = Some((page_index, char_index));
                        self.text_selection =
                            Some(TextSelection::new(page_index, char_index, char_index));
                    } else {
                        self.selection_anchor = None;
                        self.text_selection = None;
                    }
                }
            }
        }

        if response.dragged() {
            let Some((anchor_page, anchor_index)) = self.selection_anchor else {
                return;
            };
            if anchor_page != page_index {
                return;
            }

            if let Some(pos) = response.interact_pointer_pos() {
                if let Some(pdf) = placement.screen_to_pdf(pos) {
                    if let Some(char_index) = self.text_char_index_at(page_index, pdf) {
                        self.text_selection =
                            Some(TextSelection::new(page_index, anchor_index, char_index));
                    }
                }
            }
        }

        if response.drag_stopped() {
            self.selection_anchor = None;
            if self.active_tool == Tool::Marker {
                if self.selected_text().is_some() {
                    self.mark_selection(self.marker_preset());
                }
            } else if let Some(text) = self.selected_text() {
                self.status = format!("Selected {} character(s)", text.chars().count());
            }
        }
    }

    fn draw_selection_context_menu(
        &mut self,
        ctx: &Context,
        response: &egui::Response,
        page_index: usize,
    ) {
        let has_selection = self
            .text_selection
            .is_some_and(|selection| selection.contains(page_index));
        if !has_selection {
            return;
        }

        response.context_menu(|ui| {
            if ui.button("Copy").clicked() {
                self.copy_selection(ctx);
                ui.close();
            }
            if ui.button("Highlight selection").clicked() {
                self.mark_selection(self.marker_preset());
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
        let Some(selection) = self.text_selection else {
            self.selection_toolbar_rect = None;
            return;
        };
        if !selection.contains(page_index)
            || page_index != selection.action_page()
            || self.selected_text().is_none()
        {
            return;
        }
        if self.selection_anchor.is_some() {
            self.selection_toolbar_rect = None;
            return;
        }
        if self.active_tool == Tool::Marker {
            self.selection_toolbar_rect = None;
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

        self.selection_toolbar_rect = Some(inner.response.rect);
    }

    fn copy_selection(&mut self, ctx: &Context) {
        let Some(text) = self.selected_text() else {
            self.status = "No selected text to copy.".to_owned();
            return;
        };

        ctx.copy_text(text.clone());
        let chars = text.chars().count();
        match arboard::Clipboard::new().and_then(|mut clipboard| clipboard.set_text(text)) {
            Ok(()) => {
                self.status = format!("Copied {chars} character(s)");
            }
            Err(error) => {
                self.status = format!("Copy failed: {error}");
            }
        }
    }

    fn select_all_text(&mut self, ctx: &Context) {
        if self.document.is_none() {
            return;
        }

        self.clear_text_box_selection();
        self.text_selection = None;
        self.selection_anchor = None;
        self.selection_toolbar_rect = None;
        self.pending_select_all_text = true;
        self.finish_pending_select_all_text(ctx);
    }

    fn finish_pending_select_all_text(&mut self, ctx: &Context) {
        if !self.pending_select_all_text {
            return;
        }

        let Some((path, page_count)) = self
            .document
            .as_ref()
            .map(|document| (document.path.clone(), document.page_count))
        else {
            self.pending_select_all_text = false;
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
            self.pending_select_all_text = false;
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
            self.pending_select_all_text = false;
            self.status = "No selectable PDF text found.".to_owned();
            return;
        };

        self.text_selection = Some(TextSelection::range(
            start_page,
            0,
            end_page,
            end_len.saturating_sub(1),
        ));
        self.selection_anchor = None;
        self.selection_toolbar_rect = None;
        self.pending_select_all_text = false;
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
        let Some(selection) = self.text_selection else {
            return;
        };

        let mut count = 0;
        for page_index in selection.page_range() {
            for rect in self.selection_rects_for_page(page_index) {
                count += 1;
                self.annotations.push(EditorAnnotation {
                    page_index,
                    rect,
                    kind: AnnotationKind::Marker {
                        color_rgb: preset.color_rgb,
                        opacity: self.marker_opacity_for(preset),
                        style: preset.style,
                    },
                });
            }
        }

        self.status = match preset.style {
            MarkerStyle::Highlight => {
                format!("Highlighted selected text ({count} line segment(s))")
            }
            MarkerStyle::Underline => format!("Underlined selected text ({count} line segment(s))"),
        };
        self.clear_text_selection();
    }

    fn clear_text_selection(&mut self) {
        self.text_selection = None;
        self.selection_anchor = None;
        self.selection_toolbar_rect = None;
        self.pending_select_all_text = false;
    }

    fn selected_text(&self) -> Option<String> {
        let document = self.document.as_ref()?;
        let selection = self.text_selection?;

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
        let Some(selection) = self.text_selection else {
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
            .selection_toolbar_rect
            .is_some_and(|rect| rect.expand(4.0).contains(pos))
        {
            return true;
        }

        self.selection_rects_for_page(page_index)
            .into_iter()
            .map(|pdf_rect| placement.pdf_rect_to_screen(pdf_rect).expand(2.0))
            .any(|rect| rect.contains(pos))
    }

    fn click_hits_text_box_action(&self, response: &egui::Response) -> bool {
        let Some(pos) = response.interact_pointer_pos() else {
            return false;
        };

        self.text_box_action_rect
            .is_some_and(|rect| rect.expand(4.0).contains(pos))
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
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        if consume_command_shortcut(ctx, egui::Key::W) {
            if let Some(active_tab) = self.active_tab {
                self.close_tab(active_tab, ctx);
            }
        }

        self.poll_incoming_paths(ctx);
        self.poll_queued_open_paths(ctx);
        self.poll_render_results(ctx);
        self.finish_pending_select_all_text(ctx);
        self.poll_ocr();
        self.poll_chat_results(ctx);
        self.poll_liquid_results(ctx);
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
        if !wants_keyboard_input && consume_command_shortcut(ctx, egui::Key::A) {
            self.select_all_text(ctx);
        }
        if !wants_keyboard_input
            && self.text_selection.is_some()
            && consume_command_shortcut(ctx, egui::Key::C)
        {
            self.copy_selection(ctx);
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
        self.draw_update_notice(ctx);

        if self.zoom_is_animating()
            || !self.pending_page_renders.is_empty()
            || !self.pending_thumbnail_renders.is_empty()
            || !self.pending_native_text.is_empty()
            || !self.pending_text_chars.is_empty()
            || self.pending_select_all_text
            || !self.queued_open_paths.is_empty()
            || self.update_state.is_busy()
            || self.chat_state.in_flight
            || self.ocr_is_active()
            || matches!(
                self.liquid_state,
                LiquidState::PreparingText | LiquidState::Preparing
            )
        {
            ctx.request_repaint_after(RENDER_POLL_INTERVAL);
        }
    }
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
        input.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, key))
    })
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

fn estimate_tokens(text: &str) -> usize {
    (text.chars().count() / 4).max(1)
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

fn color_from_rgb(rgb: [f32; 3], alpha: u8) -> Color32 {
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgba_unmultiplied(channel(rgb[0]), channel(rgb[1]), channel(rgb[2]), alpha)
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

fn clean_pdf_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut clean = Vec::new();
    for path in paths {
        let is_pdf = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"));
        if !is_pdf {
            continue;
        }

        let normalized = std::fs::canonicalize(&path).unwrap_or(path);
        if seen.insert(normalized.clone()) {
            clean.push(normalized);
        }
    }
    clean
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

fn compact_exam_metadata(text: &str) -> String {
    let mut compact = text.trim().to_owned();
    if let Some((_, rest)) = compact.split_once("Contracts Exam - ") {
        compact = rest.trim().to_owned();
    }
    compact = compact.replace(" | ", "  ");
    compact
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
