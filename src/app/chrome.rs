//! Window chrome: the masthead at the top and the markup bar at the bottom.
//!
//! The masthead replaces the old two-row toolbar. It names the document the way
//! a law review page does (authors, title, citation over a double rule) and
//! keeps navigation beneath it: contents, the Original/Review/Side-by-side
//! switch, find, and file actions. Marking up moves to a floating bar that the
//! reader can drag anywhere over the page; it also carries zoom, page, and save
//! state, which is why the status line is gone.

use super::*;
use crate::review_masthead::{MASTHEAD_PAGES_TO_READ, MastheadIdentity, masthead_identity};

pub(super) const NAVY: Color32 = Color32::from_rgb(28, 38, 56);
pub(super) const NAVY_TEXT: Color32 = Color32::from_rgb(233, 226, 210);
pub(super) const NAVY_MUTED: Color32 = Color32::from_rgb(207, 199, 182);
pub(super) const IVORY: Color32 = Color32::from_rgb(251, 248, 241);
pub(super) const BRASS: Color32 = Color32::from_rgb(199, 165, 116);
pub(super) const OXBLOOD: Color32 = Color32::from_rgb(122, 31, 43);
pub(super) const RULE: Color32 = Color32::from_rgb(217, 207, 189);

const MASTHEAD_FONT: &str = "LawPDF Masthead";
const MASTHEAD_FONT_BYTES: &[u8] = include_bytes!("../../vendor/fonts/FrankRuhlLibre-Black.ttf");
const TAB_ROW_HEIGHT: f32 = 38.0;
const IDENTITY_ROW_HEIGHT: f32 = 46.0;
const ACTION_ROW_HEIGHT: f32 = 40.0;
const MARKUP_BAR_MARGIN: f32 = 22.0;
const STATUS_CAPTION_SECONDS: f32 = 5.0;
const MARKUP_BAR_DEFAULT_SIZE: Vec2 = Vec2::new(620.0, 52.0);

/// View state for the chrome. Lives on the app, never on a tab.
#[derive(Default)]
pub(super) struct ChromeState {
    identity: Option<(IdentityKey, MastheadIdentity)>,
    /// Central area of the window, recorded while the document draws.
    pub(super) document_rect: Option<Rect>,
    markup_bar_size: Option<Vec2>,
    markup_bar_drag: Option<Vec2>,
    last_status: String,
    status_changed_at: Option<Instant>,
    /// Review note shown in full, keyed as in `ReviewMarginEntry::key`.
    pub(super) expanded_note: Option<String>,
    /// Note under the pointer last frame, so callout and note light up together.
    pub(super) hovered_note: Option<String>,
    /// True while the current frame lays notes out in the margin.
    pub(super) margin_notes_live: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct IdentityKey {
    epoch: u64,
    text_pages: usize,
    review: usize,
}

/// Register the masthead face alongside the reading face.
pub(super) fn install_masthead_font(fonts: &mut FontDefinitions) {
    fonts.font_data.insert(
        MASTHEAD_FONT.to_owned(),
        Arc::new(FontData::from_static(MASTHEAD_FONT_BYTES)),
    );
    fonts.families.insert(
        FontFamily::Name(MASTHEAD_FONT.into()),
        vec![MASTHEAD_FONT.to_owned()],
    );
}

fn masthead_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(MASTHEAD_FONT.into()))
}

/// Small capitals, approximated: upper case, a size step down, open tracking.
pub(super) fn small_caps_job(text: &str, size: f32, color: Color32, max_width: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.append(
        &text.to_uppercase(),
        0.0,
        TextFormat {
            font_id: FontId::proportional(size),
            color,
            extra_letter_spacing: size * 0.12,
            ..Default::default()
        },
    );
    job.wrap = egui::text::TextWrapping {
        max_width,
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    job
}

fn small_caps_button(
    ui: &mut egui::Ui,
    text: &str,
    color: Color32,
    active: bool,
) -> egui::Response {
    let galley =
        ui.fonts_mut(|fonts| fonts.layout_job(small_caps_job(text, 12.5, color, f32::INFINITY)));
    let size = galley.size() + Vec2::new(10.0, 10.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    if response.hovered() && !active {
        ui.painter()
            .rect_filled(rect, 4.0, Color32::from_rgba_unmultiplied(42, 38, 32, 12));
    }
    let text_pos = rect.center() - galley.size() * 0.5;
    ui.painter().galley(text_pos, galley, color);
    if active {
        let y = rect.bottom() - 2.0;
        ui.painter().hline(
            (rect.left() + 5.0)..=(rect.right() - 5.0),
            y,
            Stroke::new(1.6_f32, color),
        );
    }
    response.on_hover_cursor(CursorIcon::PointingHand)
}

impl PdfEditorApp {
    fn masthead_identity_for_view(&mut self) -> MastheadIdentity {
        let Some(document) = self.document.as_ref() else {
            return MastheadIdentity {
                title: "LawPDF".to_owned(),
                ..Default::default()
            };
        };
        let review = match &self.liquid_mode2_state {
            LiquidState::Ready(review) => Some(Arc::clone(review)),
            _ => None,
        };
        let text_pages = document
            .native_text_loaded
            .iter()
            .take(MASTHEAD_PAGES_TO_READ + 1)
            .filter(|loaded| **loaded)
            .count();
        let key = IdentityKey {
            epoch: self.document_epoch,
            text_pages,
            review: review
                .as_ref()
                .map(|review| Arc::as_ptr(review) as usize)
                .unwrap_or_default(),
        };
        if let Some((cached_key, identity)) = &self.chrome.identity
            && *cached_key == key
        {
            return identity.clone();
        }
        let pages = document
            .native_text
            .iter()
            .zip(&document.native_text_loaded)
            .take(MASTHEAD_PAGES_TO_READ + 1)
            .map(|(text, loaded)| if *loaded { text.as_str() } else { "" })
            .collect::<Vec<_>>();
        let fallback = document
            .path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .map(|stem| stem.replace(['-', '_'], " "))
            .unwrap_or_else(|| document.title.clone());
        let identity = masthead_identity(
            &pages,
            review.as_ref().map(|review| review.title.as_str()),
            review
                .as_ref()
                .map(|review| review.blocks.as_slice())
                .unwrap_or_default(),
            &fallback,
        );
        self.chrome.identity = Some((key, identity.clone()));
        identity
    }

    /// Ask the worker for the opening pages' text the masthead reads.
    fn request_masthead_text(&mut self) {
        let Some((path, page_count)) = self
            .document
            .as_ref()
            .map(|document| (document.path.clone(), document.page_count))
        else {
            return;
        };
        for page_index in 0..page_count.min(MASTHEAD_PAGES_TO_READ + 1) {
            self.enqueue_native_text(&path, page_index);
        }
    }

    pub(super) fn sidebar_visible(&self) -> bool {
        if self.sbs_mode.is_some() {
            return false;
        }
        if self.view_mode == DocumentViewMode::LiquidMode2 && !self.review_pdf_split {
            self.settings.sidebar_in_review
        } else {
            self.settings.sidebar_in_pdf
        }
    }

    pub(super) fn set_sidebar_visible(&mut self, visible: bool) {
        if self.view_mode == DocumentViewMode::LiquidMode2 && !self.review_pdf_split {
            self.settings.sidebar_in_review = visible;
        } else {
            self.settings.sidebar_in_pdf = visible;
        }
        self.queue_settings_save();
    }

    pub(super) fn draw_masthead(&mut self, ctx: &Context) {
        self.request_masthead_text();
        let identity = self.masthead_identity_for_view();
        egui::TopBottomPanel::top("masthead")
            .frame(egui::Frame::NONE.fill(IVORY))
            .show_separator_line(false)
            .show(ctx, |ui| {
                quiet_buttons(ui);
                ui.spacing_mut().item_spacing = Vec2::new(6.0, 0.0);
                self.draw_tab_row(ui, ctx);
                ui.add_space(12.0);
                self.draw_identity_row(ui, &identity);
                self.draw_action_row(ui, ctx);
                // Brass hairline where the masthead meets the page.
                let rect = ui.max_rect();
                ui.painter().hline(
                    rect.x_range(),
                    ui.min_rect().bottom(),
                    Stroke::new(1.0_f32, BRASS),
                );
            });
    }

    fn draw_tab_row(&mut self, ui: &mut egui::Ui, ctx: &Context) {
        let (row, _) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), TAB_ROW_HEIGHT),
            Sense::hover(),
        );
        let painter = ui.painter_at(row);
        painter.rect_filled(row, 0.0, NAVY);
        // Pinstripes: faint verticals every seven points.
        let mut x = row.left() + 3.0;
        while x < row.right() {
            painter.vline(
                x,
                row.y_range(),
                Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 8)),
            );
            x += 7.0;
        }

        let mut switch_to = None;
        let mut close_tab = None;
        let mut open_new = false;
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(row.shrink2(Vec2::new(14.0, 0.0)))
                .layout(egui::Layout::left_to_right(Align::Max)),
            |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                paint_bell_mark(ui, BRASS, NAVY_TEXT);
                let brand = ui.fonts_mut(|fonts| {
                    fonts.layout_job(small_caps_job("LawPDF", 13.0, NAVY_TEXT, f32::INFINITY))
                });
                let (brand_rect, _) = ui.allocate_exact_size(
                    Vec2::new(brand.size().x + 18.0, TAB_ROW_HEIGHT),
                    Sense::hover(),
                );
                ui.painter().galley(
                    Pos2::new(
                        brand_rect.left() + 6.0,
                        brand_rect.center().y - brand.size().y * 0.5,
                    ),
                    brand,
                    NAVY_TEXT,
                );
                for index in 0..self.tabs.len() {
                    let action = self.draw_tab(ui, index);
                    match action {
                        TabAction::Switch => switch_to = Some(index),
                        TabAction::Close => close_tab = Some(index),
                        TabAction::None => {}
                    }
                }
                let plus = ui
                    .add(
                        egui::Button::new(RichText::new("+").size(17.0).color(NAVY_TEXT))
                            .frame(false)
                            .min_size(Vec2::new(30.0, 30.0)),
                    )
                    .on_hover_text("Open PDF in a new tab");
                plus.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::Button,
                        true,
                        "Open PDF in a new tab",
                    )
                });
                open_new = plus.clicked();
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    ui.label(
                        RichText::new(concat!("v", env!("CARGO_PKG_VERSION")))
                            .size(13.0)
                            .color(NAVY_MUTED),
                    )
                    .on_hover_text(APP_VERSION_LABEL);
                });
            },
        );
        if open_new {
            self.open_dialog(ctx);
        }
        if let Some(index) = close_tab {
            self.close_tab(index, ctx);
        } else if let Some(index) = switch_to {
            self.switch_to_tab(index, ctx);
        }
    }

    fn draw_tab(&mut self, ui: &mut egui::Ui, index: usize) -> TabAction {
        let tab = &self.tabs[index];
        let is_active = self.active_tab == Some(index);
        let dirty = if is_active {
            self.annotations_dirty
        } else {
            tab.annotations_dirty
        };
        let sbs_side = self.sbs_mode.and_then(|mode| {
            if mode.left_epoch == tab.document_epoch {
                Some("L")
            } else if mode.right_epoch == tab.document_epoch {
                Some("R")
            } else {
                None
            }
        });
        let full_title = tab.title();
        let path = tab.document.path.display().to_string();
        const MAX_TAB_CHARS: usize = 30;
        let mut title = full_title.clone();
        if title.chars().count() > MAX_TAB_CHARS {
            title = format!(
                "{}…",
                title.chars().take(MAX_TAB_CHARS - 1).collect::<String>()
            );
        }
        if let Some(side) = sbs_side {
            title = format!("{title}  ·  {side}");
        }
        let text_color = if is_active { INK } else { NAVY_MUTED };
        let galley = ui
            .fonts_mut(|fonts| fonts.layout_no_wrap(title, FontId::proportional(14.5), text_color));
        let dot = if dirty || is_active { 13.0 } else { 0.0 };
        let size = Vec2::new(galley.size().x + dot + 16.0 + 26.0, 32.0);
        let (rect, response) = ui.allocate_exact_size(size, Sense::click());
        let fill = if is_active {
            IVORY
        } else if response.hovered() {
            Color32::from_rgba_unmultiplied(255, 255, 255, 16)
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect_filled(
            rect,
            egui::CornerRadius {
                nw: 6,
                ne: 6,
                sw: 0,
                se: 0,
            },
            fill,
        );
        let mut x = rect.left() + 12.0;
        if dot > 0.0 {
            let dot_color = if dirty { OXBLOOD } else { BRASS };
            ui.painter()
                .circle_filled(Pos2::new(x + 3.5, rect.center().y), 3.5, dot_color);
            x += dot;
        }
        ui.painter().galley(
            Pos2::new(x, rect.center().y - galley.size().y * 0.5),
            galley,
            text_color,
        );
        let close_rect = Rect::from_center_size(
            Pos2::new(rect.right() - 15.0, rect.center().y),
            Vec2::splat(18.0),
        );
        let close = ui.interact(close_rect, response.id.with("close"), Sense::click());
        if close.hovered() {
            ui.painter().circle_filled(
                close_rect.center(),
                9.0,
                if is_active {
                    Color32::from_rgba_unmultiplied(42, 38, 32, 22)
                } else {
                    Color32::from_rgba_unmultiplied(255, 255, 255, 30)
                },
            );
        }
        let cross = if is_active { MUTED_INK } else { NAVY_MUTED };
        let c = close_rect.center();
        for (a, b) in [(-3.5, 3.5), (3.5, -3.5)] {
            ui.painter().line_segment(
                [Pos2::new(c.x + a, c.y - 3.5), Pos2::new(c.x + b, c.y + 3.5)],
                Stroke::new(1.3_f32, cross),
            );
        }
        close.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                true,
                format!("Close {full_title}"),
            )
        });
        let close = close.on_hover_text("Close tab");
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::Button, true, is_active, &full_title)
        });
        let response = response
            .on_hover_text(if dirty {
                format!("{path}\nUnsaved annotations")
            } else {
                path
            })
            .on_hover_cursor(CursorIcon::PointingHand);
        if close.clicked() {
            TabAction::Close
        } else if response.clicked() {
            TabAction::Switch
        } else {
            TabAction::None
        }
    }

    fn draw_identity_row(&mut self, ui: &mut egui::Ui, identity: &MastheadIdentity) {
        let (row, _) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), IDENTITY_ROW_HEIGHT),
            Sense::hover(),
        );
        let inner = row.shrink2(Vec2::new(32.0, 0.0));
        let title_text = identity.title.to_uppercase();
        let title_size = if inner.width() < 900.0 { 22.0 } else { 27.0 };
        let mut job = LayoutJob::default();
        job.append(
            &title_text,
            0.0,
            TextFormat {
                font_id: masthead_font(title_size),
                color: INK,
                extra_letter_spacing: title_size * 0.2,
                ..Default::default()
            },
        );
        let max_title_width = inner.width() * 0.56;
        job.wrap = egui::text::TextWrapping {
            max_width: max_title_width,
            max_rows: 1,
            break_anywhere: true,
            overflow_character: Some('…'),
        };
        let title = ui.fonts_mut(|fonts| fonts.layout_job(job));
        let baseline_y = inner.bottom() - 10.0;
        let title_pos = Pos2::new(
            inner.center().x - title.size().x * 0.5,
            baseline_y - title.size().y + 4.0,
        );
        let title_rect = Rect::from_min_size(title_pos, title.size());
        let title_truncated = title.elided;
        ui.painter().galley(title_pos, title, INK);

        let flank_width = ((inner.width() - title_rect.width()) * 0.5 - 28.0).max(60.0);
        let authors = ui.fonts_mut(|fonts| {
            fonts.layout_job(small_caps_job(
                &identity.authors,
                13.0,
                MUTED_INK,
                flank_width,
            ))
        });
        let citation = ui.fonts_mut(|fonts| {
            fonts.layout_job(small_caps_job(
                &identity.citation,
                13.0,
                MUTED_INK,
                flank_width,
            ))
        });
        let flank_y = baseline_y - authors.size().y.max(citation.size().y) + 1.0;
        let authors_rect = Rect::from_min_size(Pos2::new(inner.left(), flank_y), authors.size());
        let citation_rect = Rect::from_min_size(
            Pos2::new(inner.right() - citation.size().x, flank_y),
            citation.size(),
        );
        ui.painter().galley(authors_rect.min, authors, MUTED_INK);
        ui.painter().galley(citation_rect.min, citation, MUTED_INK);
        if !identity.authors.is_empty() {
            ui.interact(
                authors_rect,
                ui.id().with("masthead-authors"),
                Sense::hover(),
            )
            .on_hover_text(&identity.authors);
        }
        if !identity.citation.is_empty() {
            ui.interact(
                citation_rect,
                ui.id().with("masthead-citation"),
                Sense::hover(),
            )
            .on_hover_text(&identity.citation);
        }
        let title_response =
            ui.interact(title_rect, ui.id().with("masthead-title"), Sense::hover());
        title_response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &identity.title)
        });
        if title_truncated {
            title_response.on_hover_text(&identity.title);
        }

        // The double rule under the title line.
        let y = inner.bottom() - 3.0;
        ui.painter()
            .hline(inner.x_range(), y, Stroke::new(1.3_f32, INK));
        ui.painter()
            .hline(inner.x_range(), y + 3.0, Stroke::new(0.8_f32, INK));
    }

    fn draw_action_row(&mut self, ui: &mut egui::Ui, ctx: &Context) {
        let (row, _) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), ACTION_ROW_HEIGHT),
            Sense::hover(),
        );
        let inner = row.shrink2(Vec2::new(26.0, 0.0));
        let has_document = self.document.is_some();

        // Centre: the view switch.
        let switch_width = 300.0_f32.min(inner.width() * 0.34);
        let switch_rect = Rect::from_center_size(
            Pos2::new(inner.center().x, inner.center().y),
            Vec2::new(switch_width, ACTION_ROW_HEIGHT),
        );
        let left_rect = Rect::from_min_max(
            inner.left_top(),
            Pos2::new(switch_rect.left() - 12.0, inner.bottom()),
        );
        let right_rect = Rect::from_min_max(
            Pos2::new(switch_rect.right() + 12.0, inner.top()),
            inner.right_bottom(),
        );

        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(left_rect)
                .layout(egui::Layout::left_to_right(Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let visible = self.sidebar_visible();
                let toggle = toolbar_icon_button(
                    ui,
                    ToolbarIcon::Sidebar,
                    visible,
                    self.sbs_mode.is_none(),
                    if visible {
                        "Hide the side panel (pages, search, chat, notes)"
                    } else {
                        "Show the side panel (pages, search, chat, notes)"
                    },
                );
                if toggle.clicked() {
                    self.set_sidebar_visible(!visible);
                }
                self.draw_contents_menu(ui, ctx);
            },
        );

        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(switch_rect)
                .layout(egui::Layout::left_to_right(Align::Center)),
            |ui| {
                ui.add_enabled_ui(has_document, |ui| {
                    self.draw_view_switch(ui, ctx, switch_width);
                });
            },
        );

        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(right_rect)
                .layout(egui::Layout::right_to_left(Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                self.draw_more_menu(ui, ctx);
                if self.view_mode == DocumentViewMode::LiquidMode2 {
                    self.draw_reading_menu(ui);
                }
                self.draw_file_actions(ui, ctx, has_document);
                ui.add_space(8.0);
                let find_width = (ui.available_width() - 8.0).clamp(90.0, 230.0);
                self.draw_find_field(ui, ctx, has_document, find_width);
            },
        );
    }

    fn draw_view_switch(&mut self, ui: &mut egui::Ui, ctx: &Context, width: f32) {
        let sbs_active = self.sbs_mode.is_some() || self.review_pdf_split;
        let pdf_active = !sbs_active && self.view_mode == DocumentViewMode::Pdf;
        let review_active = !sbs_active && self.view_mode == DocumentViewMode::LiquidMode2;
        let item_width = [
            ("Original", pdf_active),
            ("Review", review_active),
            ("Side by side", sbs_active),
        ]
        .iter()
        .map(|(label, _)| {
            ui.fonts_mut(|fonts| {
                fonts
                    .layout_job(small_caps_job(label, 12.5, INK, f32::INFINITY))
                    .size()
                    .x
                    + 10.0
            })
        })
        .sum::<f32>()
            + 2.0 * ui.spacing().item_spacing.x;
        ui.add_space(((width - item_width) * 0.5).max(0.0));
        let color = |active: bool| if active { OXBLOOD } else { MUTED_INK };
        if small_caps_button(ui, "Original", color(pdf_active), pdf_active)
            .on_hover_text("The PDF as printed")
            .clicked()
        {
            if matches!(
                self.view_mode,
                DocumentViewMode::Liquid | DocumentViewMode::LiquidMode2
            ) {
                self.log_reflow_rejected(self.view_mode);
            }
            if self.sbs_mode.is_some() {
                self.exit_sbs_mode(ctx);
            }
            self.review_pdf_split = false;
            self.set_view_mode(DocumentViewMode::Pdf, ctx);
        }
        let review = small_caps_button(ui, "Review", color(review_active), review_active)
            .on_hover_text("Review Mode: the article reflowed, notes in the margin");
        if review.clicked() {
            if self.sbs_mode.is_some() {
                self.exit_sbs_mode(ctx);
            }
            self.review_pdf_split = false;
            self.set_view_mode(DocumentViewMode::LiquidMode2, ctx);
        }
        if small_caps_button(ui, "Side by side", color(sbs_active), sbs_active)
            .on_hover_text("Review beside the source page, or two PDFs if another tab is open")
            .clicked()
        {
            self.toggle_review_or_pdf_split(ctx);
        }
        if let Some(mut mode) = self.sbs_mode
            && toolbar_icon_button(
                ui,
                ToolbarIcon::Swap,
                false,
                true,
                "Swap the left and right PDFs",
            )
            .clicked()
        {
            std::mem::swap(&mut mode.left_epoch, &mut mode.right_epoch);
            self.sbs_mode = Some(mode);
            ctx.request_repaint();
        }
        if matches!(
            self.liquid_mode2_state,
            LiquidState::PreparingText | LiquidState::Preparing
        ) {
            ui.spinner().on_hover_text("Preparing Review Mode");
        }
    }

    fn draw_contents_menu(&mut self, ui: &mut egui::Ui, ctx: &Context) {
        let review = match &self.liquid_mode2_state {
            LiquidState::Ready(document) => Some(Arc::clone(document)),
            _ => None,
        };
        if let Some(document) = &review {
            self.refresh_review_derived(document);
        }
        let outline = review
            .as_ref()
            .map(|_| Arc::clone(&self.review_derived.outline));
        let in_review = self.view_mode == DocumentViewMode::LiquidMode2;
        let current = if in_review {
            outline.as_ref().and_then(|outline| {
                let active = self.review_active_heading?;
                outline
                    .iter()
                    .find(|item| item.block_index == active)
                    .map(|item| item.text.clone())
            })
        } else {
            None
        };
        let page_count = self
            .document
            .as_ref()
            .map(|document| document.page_count)
            .unwrap_or_default();
        let place = match (&current, in_review, page_count) {
            (Some(section), _, _) => section.clone(),
            (None, true, _) => "Beginning".to_owned(),
            (None, false, 0) => "No document".to_owned(),
            (None, false, count) => format!("Page {} of {count}", self.page_index + 1),
        };
        let mut job = small_caps_job("Contents", 12.5, OXBLOOD, f32::INFINITY);
        job.append(
            "   ",
            0.0,
            TextFormat {
                font_id: FontId::proportional(15.0),
                color: INK,
                ..Default::default()
            },
        );
        let max_place_chars = ((ui.available_width() - 150.0) / 7.5).max(8.0) as usize;
        let mut place_text = place.clone();
        if place_text.chars().count() > max_place_chars {
            place_text = format!(
                "{}…",
                place_text
                    .chars()
                    .take(max_place_chars.saturating_sub(1))
                    .collect::<String>()
            );
        }
        job.append(
            &place_text,
            0.0,
            TextFormat {
                font_id: FontId::proportional(16.0),
                color: INK,
                italics: true,
                ..Default::default()
            },
        );
        job.wrap.max_rows = 1;
        let mut clicked_target = None;
        let mut open_review = false;
        let (response, _) =
            egui::containers::menu::MenuButton::from_button(egui::Button::new(job).frame(false))
                .ui(ui, |ui| {
                    ui.set_max_width(420.0);
                    ui.set_max_height(460.0);
                    match &outline {
                        Some(outline) if !outline.is_empty() => {
                            egui::ScrollArea::vertical()
                                .id_salt("masthead_contents")
                                .max_height(440.0)
                                .show(ui, |ui| {
                                    for item in outline.iter() {
                                        let active = in_review
                                            && self.review_active_heading == Some(item.block_index);
                                        ui.horizontal(|ui| {
                                            ui.add_space(
                                                14.0 * item.level.saturating_sub(1) as f32,
                                            );
                                            let text = RichText::new(&item.text)
                                                .size(if item.level == 1 { 15.5 } else { 14.5 })
                                                .color(if active {
                                                    OXBLOOD
                                                } else if item.level == 1 {
                                                    INK
                                                } else {
                                                    MUTED_INK
                                                });
                                            if ui
                                                .add(egui::Button::new(text).frame(false).wrap())
                                                .on_hover_cursor(CursorIcon::PointingHand)
                                                .clicked()
                                            {
                                                clicked_target = Some(item.block_index);
                                                ui.close();
                                            }
                                        });
                                    }
                                });
                        }
                        Some(_) => {
                            ui.label(
                                RichText::new("No section headings detected.").color(MUTED_INK),
                            );
                        }
                        None => {
                            ui.label(
                                RichText::new(
                                    "Review Mode builds the contents from the article's headings.",
                                )
                                .color(MUTED_INK),
                            );
                            if self.document.is_some() && ui.button("Open Review Mode").clicked() {
                                open_review = true;
                                ui.close();
                            }
                        }
                    }
                });
        let (chevron_rect, _) = ui.allocate_exact_size(Vec2::new(12.0, 20.0), Sense::hover());
        let chevron = chevron_rect.center() - Vec2::new(4.0, 0.0);
        for (from, to) in [
            (Vec2::new(-4.0, -2.0), Vec2::new(0.0, 2.0)),
            (Vec2::new(0.0, 2.0), Vec2::new(4.0, -2.0)),
        ] {
            ui.painter().line_segment(
                [chevron + from, chevron + to],
                Stroke::new(1.3_f32, MUTED_INK),
            );
        }
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("Contents: {place}"))
        });
        if open_review {
            self.set_view_mode(DocumentViewMode::LiquidMode2, ctx);
        }
        if let Some(block_index) = clicked_target {
            if self.view_mode != DocumentViewMode::LiquidMode2 {
                self.set_view_mode(DocumentViewMode::LiquidMode2, ctx);
            }
            self.liquid_scroll_to_block = Some(block_index);
            self.review_active_heading = Some(block_index);
            ctx.request_repaint();
        }
    }

    fn draw_find_field(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &Context,
        has_document: bool,
        width: f32,
    ) {
        let search_enabled = has_document && self.sbs_mode.is_none();
        let hint = if self.view_mode == DocumentViewMode::LiquidMode2 {
            "Find in this article"
        } else {
            "Find in this PDF"
        };
        let search_response = ui
            .add_enabled(
                search_enabled,
                egui::TextEdit::singleline(&mut self.search_state.query)
                    .hint_text(hint)
                    .desired_width(width)
                    .margin(Margin::symmetric(8, 5)),
            )
            .on_disabled_hover_text("Open the active pane solo to search within it");
        ui.ctx().accesskit_node_builder(search_response.id, |node| {
            node.set_label("Find");
        });
        if self.search_state.focus_request {
            if search_enabled {
                search_response.request_focus();
            } else if has_document {
                self.status = "Open either SbS pane solo to search within it.".to_owned();
            }
            self.search_state.focus_request = false;
        }
        if search_response.changed() {
            self.invalidate_search_results();
        }
        let pressed_enter =
            search_response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        if pressed_enter && search_enabled {
            self.start_search(ctx);
            self.sidebar_tab = SidebarTab::Search;
            self.set_sidebar_visible(true);
        }
    }

    fn draw_file_actions(&mut self, ui: &mut egui::Ui, ctx: &Context, has_document: bool) {
        // Right-to-left: the last added sits leftmost.
        let markdown_busy = self.pending_markdown_request.is_some();
        let mut markdown_settings_changed = false;
        toolbar_icon_menu_button(
            ui,
            ToolbarIcon::More,
            has_document && !markdown_busy,
            "Markdown copy options",
            |ui| {
                ui.label(RichText::new("Markdown copy").strong());
                ui.separator();
                ui.label("Footnotes");
                markdown_settings_changed |= ui
                    .selectable_value(
                        &mut self.settings.markdown_copy_footnotes,
                        FootnoteMode::Inline,
                        "Inline",
                    )
                    .changed();
                markdown_settings_changed |= ui
                    .selectable_value(
                        &mut self.settings.markdown_copy_footnotes,
                        FootnoteMode::Endnotes,
                        "Endnotes",
                    )
                    .changed();
                markdown_settings_changed |= ui
                    .selectable_value(
                        &mut self.settings.markdown_copy_footnotes,
                        FootnoteMode::Omit,
                        "Omit",
                    )
                    .changed();
                ui.separator();
                markdown_settings_changed |= ui
                    .checkbox(
                        &mut self.settings.markdown_copy_include_tables,
                        "Include tables",
                    )
                    .changed();
                markdown_settings_changed |= ui
                    .checkbox(
                        &mut self.settings.markdown_copy_include_metadata,
                        "Include metadata",
                    )
                    .changed();
            },
        );
        if markdown_settings_changed {
            self.save_markdown_settings();
        }
        if toolbar_icon_button(
            ui,
            ToolbarIcon::Markdown,
            false,
            has_document && !markdown_busy,
            "Copy the whole article as Markdown for pasting into an AI chat\nCtrl/Cmd+Shift+C",
        )
        .clicked()
        {
            self.request_markdown_copy(ctx);
        }
        if markdown_busy {
            ui.spinner();
        }
        let review_document = match &self.liquid_mode2_state {
            LiquidState::Ready(document) if self.view_mode == DocumentViewMode::LiquidMode2 => {
                Some(Arc::clone(document))
            }
            _ => None,
        };
        toolbar_icon_menu_button(
            ui,
            ToolbarIcon::Export,
            has_document,
            "Export this PDF as another document or image format",
            |ui| {
                if ui.button("Save PDF copy").clicked() {
                    self.save_as_dialog(ctx);
                    ui.close();
                }
                if ui.button("Text").clicked() {
                    self.export_text_dialog(ctx);
                    ui.close();
                }
                if let Some(document) = &review_document
                    && ui
                        .button("Review text (.txt)")
                        .on_hover_text("The clean, page-free Review Mode text")
                        .clicked()
                {
                    self.export_review_text_dialog(document);
                    ui.close();
                }
                if ui.button("Markdown (.md)").clicked() {
                    self.export_markdown_dialog(ctx);
                    ui.close();
                }
                if ui.button("PNG").clicked() {
                    self.export_png_dialog(ctx);
                    ui.close();
                }
            },
        );
        if toolbar_icon_button(
            ui,
            ToolbarIcon::Save,
            false,
            has_document,
            "Save highlights and comments into this PDF",
        )
        .clicked()
            && let Err(error) = self.save_current_annotations()
        {
            self.push_error_notice(error);
        }
        if toolbar_icon_button(ui, ToolbarIcon::Open, false, true, "Open PDF").clicked() {
            self.open_dialog(ctx);
        }
    }

    /// Reading settings for Review Mode, formerly a row above the article.
    fn draw_reading_menu(&mut self, ui: &mut egui::Ui) {
        let review_document = match &self.liquid_mode2_state {
            LiquidState::Ready(document) => Some(Arc::clone(document)),
            _ => None,
        };
        toolbar_icon_menu_button(
            ui,
            ToolbarIcon::Reading,
            true,
            "Reading: text size, width, theme, notes, and narration",
            |ui| {
                ui.set_min_width(300.0);
                ui.label(RichText::new("Reading").strong());
                ui.add_space(4.0);
                let mut scale = self.liquid_text_scale;
                if ui
                    .add(
                        egui::Slider::new(&mut scale, 0.65..=1.85)
                            .step_by(0.05)
                            .custom_formatter(|value, _| format!("{:.0}%", value * 100.0))
                            .text("Text size"),
                    )
                    .changed()
                {
                    self.liquid_text_scale = scale.clamp(0.65, 1.85);
                }
                let mut width = self.liquid_max_width;
                if ui
                    .add(
                        egui::Slider::new(&mut width, 480.0..=1280.0)
                            .step_by(20.0)
                            .suffix(" px")
                            .text("Column width"),
                    )
                    .changed()
                {
                    self.liquid_max_width = width.clamp(480.0, 1280.0);
                }
                ui.horizontal(|ui| {
                    ui.label("Theme");
                    ui.selectable_value(&mut self.liquid_theme, LiquidTheme::Paper, "Paper");
                    ui.selectable_value(&mut self.liquid_theme, LiquidTheme::Sepia, "Sepia");
                    ui.selectable_value(&mut self.liquid_theme, LiquidTheme::Dark, "Dark");
                });
                ui.separator();
                let mut show_notes = !self.liquid_hide_footnotes;
                if ui
                    .checkbox(&mut show_notes, "Footnotes in the margin")
                    .on_hover_text("Show footnote markers, margin notes, and the notes list")
                    .changed()
                {
                    self.liquid_hide_footnotes = !show_notes;
                }
                ui.checkbox(
                    &mut self.liquid_show_hidden_furniture,
                    "Show hidden furniture",
                )
                .on_hover_text(
                    "Reveal headers, page numbers, TOC, and other hidden furniture as dimmed lines",
                );
                if ui.button("Reset reading settings").clicked() {
                    self.liquid_text_scale = 1.0;
                    self.liquid_max_width = REVIEW_DEFAULT_MAX_WIDTH;
                    self.liquid_theme = LiquidTheme::Paper;
                }
                let Some(document) = review_document.as_deref() else {
                    return;
                };
                ui.separator();
                ui.label(RichText::new("Narration").strong());
                ui.horizontal(|ui| {
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
                            egui::Button::new(paid_label),
                        )
                        .on_hover_text(
                            "Creates AI-generated narration. Sends the article text to the selected provider using your key; provider charges may apply.",
                        )
                        .clicked()
                    {
                        self.start_paid_liquid_tts(document);
                    }
                });
                let mut include = self.tts_controller.include_notes;
                if ui
                    .checkbox(&mut include, "Read footnotes after the body")
                    .changed()
                {
                    self.tts_controller.include_notes = include;
                }
                if ui
                    .button("Narration keys…")
                    .on_hover_text("Add or change OpenAI and OpenRouter API keys")
                    .clicked()
                {
                    self.settings_ui.open = true;
                    ui.close();
                }
            },
        );
    }

    /// Everything that used to crowd the toolbar but is used rarely.
    fn draw_more_menu(&mut self, ui: &mut egui::Ui, ctx: &Context) {
        let has_document = self.document.is_some();
        let review_document = match &self.liquid_mode2_state {
            LiquidState::Ready(document) if self.view_mode == DocumentViewMode::LiquidMode2 => {
                Some(Arc::clone(document))
            }
            _ => None,
        };
        toolbar_icon_menu_button(ui, ToolbarIcon::Overflow, true, "More", |ui| {
            ui.set_min_width(260.0);
            if ui
                .add_enabled(
                    has_document,
                    egui::Button::new("OCR: save a searchable copy…"),
                )
                .on_hover_text("Use OpenRouter OCR and save a searchable PDF copy")
                .clicked()
            {
                self.sidebar_tab = SidebarTab::Search;
                self.start_openrouter_ocr_save();
                ui.close();
            }
            if ui
                .add_enabled(
                    !self.recovery_is_busy(),
                    egui::Button::new("Recover edits…"),
                )
                .clicked()
            {
                self.open_recovery_dialog(ctx);
                ui.close();
            }
            #[cfg(target_os = "windows")]
            if ui.button("Make LawPDF the default PDF reader…").clicked() {
                match open_windows_default_pdf_settings() {
                    Ok(()) => {
                        self.status =
                            "Windows Settings opened. Choose LawPDF for .pdf files.".to_owned();
                    }
                    Err(error) => {
                        self.push_error_notice(format!(
                            "Could not open Windows default-app settings: {error}"
                        ));
                    }
                }
                ui.close();
            }
            #[cfg(target_os = "macos")]
            if ui.button("Make LawPDF the default PDF reader").clicked() {
                match set_macos_default_pdf_reader() {
                    Ok(()) => {
                        self.status = "LawPDF is now the default PDF reader on macOS.".to_owned();
                    }
                    Err(error) => {
                        self.push_error_notice(format!(
                            "Could not make LawPDF the default PDF reader: {error}"
                        ));
                    }
                }
                ui.close();
            }
            if let Some(document) = review_document.as_deref() {
                ui.separator();
                ui.menu_button("Review details", |ui| {
                    ui.set_max_width(520.0);
                    self.draw_liquid_review_details(ui, document);
                });
                if ui
                    .button("Report a bad review…")
                    .on_hover_text(
                        "Send this review to the LawPDF server to help improve future updates",
                    )
                    .clicked()
                {
                    self.review_feedback_dialog_open = true;
                    self.review_feedback_prompt_visible = false;
                    ui.close();
                }
                let pending = self.pending_liquid_feedback_count();
                if ui
                    .add_enabled(
                        pending > 0,
                        egui::Button::new(format!("Queue corrections for retraining ({pending})")),
                    )
                    .clicked()
                {
                    self.queue_pending_liquid_feedback_for_retraining();
                    ui.close();
                }
            }
            ui.separator();
            if ui.button("Settings…").clicked() {
                self.settings_ui.open = true;
                ui.close();
            }
            ui.label(RichText::new(APP_VERSION_LABEL).size(12.0).color(MUTED_INK));
        });
    }

    /// Floating markup bar. Drag it by its grip; double-click the grip to send it home.
    pub(super) fn draw_markup_bar(&mut self, ctx: &Context) {
        let Some(area) = self.chrome.document_rect else {
            return;
        };
        if self.document.is_none() {
            return;
        }
        let size = self
            .chrome
            .markup_bar_size
            .unwrap_or(MARKUP_BAR_DEFAULT_SIZE);
        let home = Pos2::new(
            area.center().x - size.x * 0.5,
            area.bottom() - size.y - MARKUP_BAR_MARGIN,
        );
        let saved = self
            .settings
            .markup_bar_offset
            .map(|[x, y]| Vec2::new(x, y))
            .unwrap_or(Vec2::ZERO);
        let offset = self.chrome.markup_bar_drag.unwrap_or(saved);
        let pos = clamp_bar_position(home + offset, size, area);

        let area_response = egui::Area::new(egui::Id::new("markup_bar"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .interactable(true)
            .show(ctx, |ui| {
                egui::Frame::NONE
                    .fill(IVORY)
                    .stroke(Stroke::new(1.0_f32, BRASS))
                    .corner_radius(26)
                    .shadow(egui::epaint::Shadow {
                        offset: [0, 10],
                        blur: 28,
                        spread: 0,
                        color: Color32::from_rgba_unmultiplied(42, 38, 32, 40),
                    })
                    .inner_margin(Margin::symmetric(10, 6))
                    .show(ui, |ui| {
                        quiet_buttons(ui);
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            self.draw_markup_bar_contents(ui, ctx, offset, saved);
                        });
                    });
            });
        self.chrome.markup_bar_size = Some(area_response.response.rect.size());
        self.draw_status_caption(ctx, area_response.response.rect);
    }

    fn draw_markup_bar_contents(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &Context,
        offset: Vec2,
        saved: Vec2,
    ) {
        // Grip.
        let (grip_rect, grip) =
            ui.allocate_exact_size(Vec2::new(16.0, 36.0), Sense::click_and_drag());
        for row in 0..3 {
            for column in 0..2 {
                ui.painter().circle_filled(
                    Pos2::new(
                        grip_rect.center().x - 3.0 + column as f32 * 6.0,
                        grip_rect.center().y - 6.0 + row as f32 * 6.0,
                    ),
                    1.4,
                    if grip.hovered() || grip.dragged() {
                        INK
                    } else {
                        MUTED_INK
                    },
                );
            }
        }
        grip.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Move the markup bar")
        });
        let grip = grip
            .on_hover_cursor(CursorIcon::Grab)
            .on_hover_text("Drag to move. Double-click to put it back.");
        if grip.dragged() {
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
            self.chrome.markup_bar_drag = Some(offset + grip.drag_delta());
        }
        if grip.drag_stopped() {
            let final_offset = self.chrome.markup_bar_drag.take().unwrap_or(offset);
            self.settings.markup_bar_offset = Some([final_offset.x, final_offset.y]);
            self.queue_settings_save();
        }
        if grip.double_clicked() {
            self.chrome.markup_bar_drag = None;
            if saved != Vec2::ZERO {
                self.settings.markup_bar_offset = None;
                self.queue_settings_save();
            }
        }
        bar_separator(ui);

        let annotation_enabled = self.sbs_mode.is_none();
        for tool in Tool::ALL {
            let selected = self.active_tool == tool;
            if toolbar_icon_button(
                ui,
                ToolbarIcon::for_tool(tool),
                selected,
                annotation_enabled,
                if annotation_enabled {
                    tool.tooltip()
                } else {
                    "Open either pane solo to edit or annotate it"
                },
            )
            .clicked()
            {
                self.active_tool = tool;
            }
        }
        // Swatches: picking one arms the marker in that colour.
        ui.add_space(2.0);
        for (index, preset) in MARKER_PRESETS.iter().copied().enumerate() {
            let selected = self.marker_preset_index == index;
            let (rect, response) = ui.allocate_exact_size(Vec2::new(22.0, 30.0), Sense::click());
            let center = rect.center();
            let color = color_from_rgb(preset.color_rgb, 230);
            match preset.style {
                MarkerStyle::Highlight => {
                    ui.painter().circle_filled(center, 8.0, color);
                    ui.painter().circle_stroke(
                        center,
                        8.0,
                        Stroke::new(0.8_f32, Color32::from_rgba_unmultiplied(42, 38, 32, 50)),
                    );
                }
                MarkerStyle::Underline => {
                    ui.painter().text(
                        center - Vec2::new(0.0, 2.0),
                        Align2::CENTER_CENTER,
                        "U",
                        FontId::proportional(15.0),
                        color_from_rgb(preset.color_rgb, 255),
                    );
                    ui.painter().hline(
                        (center.x - 6.0)..=(center.x + 6.0),
                        center.y + 7.0,
                        Stroke::new(1.6_f32, color_from_rgb(preset.color_rgb, 255)),
                    );
                }
            }
            if selected && self.active_tool == Tool::Marker {
                ui.painter()
                    .circle_stroke(center, 11.0, Stroke::new(1.5_f32, INK));
            } else if response.hovered() {
                ui.painter()
                    .circle_stroke(center, 11.0, Stroke::new(1.0_f32, RULE));
            }
            accessibility::name_color_choice(&response, preset.label, selected);
            if response.on_hover_text(preset.label).clicked() && annotation_enabled {
                self.marker_preset_index = index;
                self.active_tool = Tool::Marker;
                self.status = format!("Marker set to {}", preset.label);
            }
        }
        bar_separator(ui);

        self.annotation_history_buttons(ui, ctx);

        let has_document = self.document.is_some();
        if self.view_mode == DocumentViewMode::Pdf && self.sbs_mode.is_none() {
            let rotation_enabled = has_document && !self.page_rotation_in_flight;
            toolbar_icon_menu_button(
                ui,
                ToolbarIcon::Rotate,
                rotation_enabled,
                "Rotate the current page by 90° (saved in this PDF)",
                |ui| {
                    if ui
                        .button("Rotate 90° left")
                        .on_hover_text(
                            "Turn the current page 90° counterclockwise and save it in this PDF",
                        )
                        .clicked()
                    {
                        self.rotate_current_page(false);
                        ui.close();
                    }
                    if ui
                        .button("Rotate 90° right")
                        .on_hover_text(
                            "Turn the current page 90° clockwise and save it in this PDF",
                        )
                        .clicked()
                    {
                        self.rotate_current_page(true);
                        ui.close();
                    }
                },
            );
            if self.page_rotation_in_flight {
                ui.spinner().on_hover_text("Saving page rotation…");
            }
        }
        if self.view_mode == DocumentViewMode::LiquidMode2 {
            let review_document = match &self.liquid_mode2_state {
                LiquidState::Ready(document) => Some(Arc::clone(document)),
                _ => None,
            };
            let reading = self.tts_controller.child.is_some();
            if toolbar_icon_button(
                ui,
                if reading {
                    ToolbarIcon::Stop
                } else {
                    ToolbarIcon::Speaker
                },
                reading,
                review_document.is_some(),
                if reading {
                    "Stop reading"
                } else if cfg!(target_os = "windows") {
                    "Read the reflowed text aloud with Windows speech"
                } else if cfg!(target_os = "macos") {
                    "Read the reflowed text aloud with macOS speech"
                } else {
                    "Read the reflowed text aloud"
                },
            )
            .clicked()
            {
                if reading {
                    self.stop_liquid_tts();
                } else if let Some(document) = review_document.as_deref() {
                    self.start_liquid_tts(document);
                }
            }
        }
        bar_separator(ui);

        // Size: PDF zoom in the original, text size in Review Mode.
        let review = self.view_mode == DocumentViewMode::LiquidMode2 && self.sbs_mode.is_none();
        let (label, zoom_out_tip, zoom_in_tip) = if review {
            (
                format!("{:.0}%", self.liquid_text_scale * 100.0),
                "Smaller text",
                "Larger text",
            )
        } else {
            (format!("{:.0}%", self.zoom * 100.0), "Zoom out", "Zoom in")
        };
        if toolbar_icon_button(ui, ToolbarIcon::ZoomOut, false, has_document, zoom_out_tip)
            .clicked()
        {
            if review {
                self.liquid_text_scale = (self.liquid_text_scale - 0.05).clamp(0.65, 1.85);
            } else {
                self.set_zoom(self.target_zoom / 1.15);
            }
        }
        ui.add_sized(
            Vec2::new(44.0, 30.0),
            egui::Label::new(RichText::new(label).size(14.5).color(INK)),
        );
        if toolbar_icon_button(ui, ToolbarIcon::ZoomIn, false, has_document, zoom_in_tip).clicked()
        {
            if review {
                self.liquid_text_scale = (self.liquid_text_scale + 0.05).clamp(0.65, 1.85);
            } else {
                self.set_zoom(self.target_zoom * 1.15);
            }
        }

        if !review {
            bar_separator(ui);
            self.draw_bar_page_controls(ui);
        }
        bar_separator(ui);
        self.draw_bar_save_state(ui);
    }

    fn draw_bar_page_controls(&mut self, ui: &mut egui::Ui) {
        let has_document = self.document.is_some();
        let page_count = self
            .document
            .as_ref()
            .map(|document| document.page_count)
            .unwrap_or_default();
        if toolbar_icon_button(
            ui,
            ToolbarIcon::Previous,
            false,
            has_document && self.page_index > 0,
            "Previous page",
        )
        .clicked()
        {
            self.go_to_page(self.page_index.saturating_sub(1));
        }
        let page_input_id = ui
            .id()
            .with(("markup-bar-page-number", self.document_epoch));
        let input_had_focus = ui.memory(|memory| memory.has_focus(page_input_id));
        let current_page_number = if has_document { self.page_index + 1 } else { 0 };
        let mut page_number_text = ui.ctx().data_mut(|data| {
            data.get_temp::<String>(page_input_id)
                .unwrap_or_else(|| current_page_number.to_string())
        });
        if !input_had_focus {
            page_number_text = current_page_number.to_string();
        }
        let page_input_response = ui
            .add_enabled(
                has_document,
                egui::TextEdit::singleline(&mut page_number_text)
                    .id(page_input_id)
                    .desired_width(38.0)
                    .horizontal_align(Align::Center),
            )
            .on_hover_text("Type a page number and press Enter");
        ui.ctx()
            .accesskit_node_builder(page_input_response.id, |node| {
                node.set_label("Page number");
            });
        if page_input_response.changed() {
            page_number_text.retain(|character| character.is_ascii_digit());
        }
        let enter_pressed = ui.input(|input| {
            input.key_pressed(egui::Key::Enter) || input.key_pressed(egui::Key::Tab)
        });
        let submit_page_number = has_document
            && (page_input_response.lost_focus()
                || (page_input_response.has_focus() && enter_pressed));
        if submit_page_number {
            match page_index_from_input(&page_number_text, page_count) {
                Ok(page_index) => {
                    self.go_to_page(page_index);
                    page_number_text = (page_index + 1).to_string();
                }
                Err(message) => {
                    self.status = message;
                    page_number_text = current_page_number.to_string();
                }
            }
            ui.memory_mut(|memory| memory.surrender_focus(page_input_id));
        }
        ui.ctx()
            .data_mut(|data| data.insert_temp(page_input_id, page_number_text));
        ui.label(
            RichText::new(format!("of {page_count}"))
                .size(14.5)
                .color(MUTED_INK),
        )
        .on_hover_text("Total pages in this PDF");
        if toolbar_icon_button(
            ui,
            ToolbarIcon::Next,
            false,
            has_document && self.page_index + 1 < page_count,
            "Next page",
        )
        .clicked()
        {
            self.go_to_page(self.page_index + 1);
        }
    }

    fn draw_bar_save_state(&mut self, ui: &mut egui::Ui) {
        let saving = self.document.as_ref().is_some_and(|document| {
            self.pending_annotation_saves.contains_key(&document.path)
                || self.active_annotation_saves.contains_key(&document.path)
        });
        let (label, color) = if saving {
            ("Saving…", MUTED_INK)
        } else if self.annotations_dirty {
            ("Not saved", OXBLOOD)
        } else {
            ("Saved", INK)
        };
        let (rect, response) = ui.allocate_exact_size(Vec2::new(78.0, 30.0), Sense::hover());
        let mark_center = Pos2::new(rect.left() + 8.0, rect.center().y);
        if saving {
            ui.painter()
                .circle_stroke(mark_center, 5.0, Stroke::new(1.4_f32, MUTED_INK));
            ui.ctx().request_repaint_after(RENDER_POLL_INTERVAL);
        } else if self.annotations_dirty {
            ui.painter().circle_filled(mark_center, 4.0, OXBLOOD);
        } else {
            ui.painter().line_segment(
                [
                    mark_center + Vec2::new(-4.5, 0.0),
                    mark_center + Vec2::new(-1.5, 3.5),
                ],
                Stroke::new(1.8_f32, BRASS),
            );
            ui.painter().line_segment(
                [
                    mark_center + Vec2::new(-1.5, 3.5),
                    mark_center + Vec2::new(5.0, -4.0),
                ],
                Stroke::new(1.8_f32, BRASS),
            );
        }
        ui.painter().text(
            Pos2::new(rect.left() + 19.0, rect.center().y),
            Align2::LEFT_CENTER,
            label,
            FontId::proportional(15.0),
            color,
        );
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, label));
        response.on_hover_text(
            "Annotations save automatically. Keep the document open if saving fails; use Save As for a protected PDF.",
        );
    }

    /// Short-lived caption above the bar: the status line, when there is news.
    fn draw_status_caption(&mut self, ctx: &Context, bar: Rect) {
        let now = Instant::now();
        if self.status != self.chrome.last_status {
            self.chrome.last_status = self.status.clone();
            self.chrome.status_changed_at = Some(now);
        }
        let ocr = self.ocr_is_active().then(|| self.ocr_summary());
        let fresh = self
            .chrome
            .status_changed_at
            .map(|at| now.duration_since(at).as_secs_f32())
            .filter(|age| *age < STATUS_CAPTION_SECONDS);
        let text = match (&ocr, fresh) {
            (Some(ocr), _) => ocr.clone(),
            (None, Some(_)) if !self.status.trim().is_empty() && self.status != "Ready" => {
                self.status.clone()
            }
            _ => return,
        };
        let alpha = fresh
            .map(|age| ((STATUS_CAPTION_SECONDS - age) / 0.6).clamp(0.0, 1.0))
            .unwrap_or(1.0);
        if ocr.is_none() {
            ctx.request_repaint_after(Duration::from_millis(60));
        }
        let galley = ctx.fonts_mut(|fonts| {
            let mut job = LayoutJob::simple(
                text,
                FontId::proportional(14.0),
                MUTED_INK.gamma_multiply(alpha),
                (bar.width() + 120.0).max(320.0),
            );
            job.wrap.max_rows = 2;
            job.wrap.overflow_character = Some('…');
            fonts.layout_job(job)
        });
        let size = galley.size() + Vec2::new(24.0, 10.0);
        let rect = Rect::from_center_size(
            Pos2::new(bar.center().x, bar.top() - 10.0 - size.y * 0.5),
            size,
        );
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("markup_bar_caption"),
        ));
        painter.rect_filled(rect, 12.0, IVORY.gamma_multiply(0.94 * alpha));
        painter.rect_stroke(
            rect,
            12.0,
            Stroke::new(1.0_f32, RULE.gamma_multiply(alpha)),
            egui::StrokeKind::Inside,
        );
        painter.galley(rect.min + Vec2::new(12.0, 5.0), galley, MUTED_INK);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TabAction {
    None,
    Switch,
    Close,
}

/// Icon buttons without a resting fill or border: they show themselves on hover.
fn quiet_buttons(ui: &mut egui::Ui) {
    let visuals = ui.visuals_mut();
    visuals.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
    visuals.widgets.inactive.bg_stroke = Stroke::NONE;
    visuals.widgets.hovered.weak_bg_fill = Color32::from_rgba_unmultiplied(42, 38, 32, 16);
    visuals.widgets.hovered.bg_stroke = Stroke::NONE;
    visuals.widgets.active.weak_bg_fill = Color32::from_rgba_unmultiplied(42, 38, 32, 28);
    visuals.selection.bg_fill = Color32::from_rgba_unmultiplied(122, 31, 43, 30);
    visuals.selection.stroke = Stroke::new(1.0_f32, OXBLOOD);
}

fn bar_separator(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(9.0, 26.0), Sense::hover());
    ui.painter()
        .vline(rect.center().x, rect.y_range(), Stroke::new(1.0_f32, RULE));
}

/// Keep the bar fully inside the document area, whatever the window does.
pub(super) fn clamp_bar_position(pos: Pos2, size: Vec2, area: Rect) -> Pos2 {
    let min = area.min + Vec2::splat(8.0);
    let max = area.max - size - Vec2::splat(8.0);
    Pos2::new(
        pos.x.clamp(min.x, max.x.max(min.x)),
        pos.y.clamp(min.y, max.y.max(min.y)),
    )
}

/// The app mark: a bell with a serif A, drawn to sit in the navy tab row.
fn paint_bell_mark(ui: &mut egui::Ui, stroke_color: Color32, letter_color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(24.0, TAB_ROW_HEIGHT), Sense::hover());
    let c = rect.center();
    let painter = ui.painter();
    let stroke = Stroke::new(1.5_f32, stroke_color);
    let scale = 0.85;
    let p = |x: f32, y: f32| Pos2::new(c.x + (x - 12.0) * scale, c.y + (y - 12.0) * scale);
    // Bell body: straight sides, rounded crown.
    let mut points = vec![p(6.5, 18.0)];
    for step in 0..=12 {
        let t = std::f32::consts::PI * (1.0 - step as f32 / 12.0);
        points.push(p(12.0 + 5.5 * t.cos(), 10.0 - 5.5 * t.sin()));
    }
    points.push(p(17.5, 18.0));
    painter.add(egui::Shape::line(points, stroke));
    painter.line_segment([p(4.5, 18.0), p(19.5, 18.0)], stroke);
    painter.circle_stroke(p(12.0, 20.4), 1.4 * scale, stroke);
    painter.text(
        p(12.0, 13.6),
        Align2::CENTER_CENTER,
        "A",
        FontId::proportional(8.5),
        letter_color,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bar_stays_inside_the_document_area() {
        let area = Rect::from_min_size(Pos2::new(100.0, 50.0), Vec2::new(800.0, 600.0));
        let size = Vec2::new(300.0, 50.0);
        assert_eq!(
            clamp_bar_position(Pos2::new(-500.0, 2000.0), size, area),
            Pos2::new(108.0, 50.0 + 600.0 - 50.0 - 8.0)
        );
        assert_eq!(
            clamp_bar_position(Pos2::new(300.0, 300.0), size, area),
            Pos2::new(300.0, 300.0)
        );
        // A window narrower than the bar pins it to the left edge instead of panicking.
        let narrow = Rect::from_min_size(Pos2::ZERO, Vec2::new(200.0, 100.0));
        assert_eq!(
            clamp_bar_position(Pos2::new(50.0, 50.0), size, narrow),
            Pos2::new(8.0, 42.0)
        );
    }

    #[test]
    fn small_caps_are_upper_case_and_single_line() {
        let job = small_caps_job("Warren & Brandeis", 12.0, INK, 200.0);
        assert_eq!(job.text, "WARREN & BRANDEIS");
        assert_eq!(job.wrap.max_rows, 1);
        assert_eq!(job.wrap.overflow_character, Some('…'));
    }
}
