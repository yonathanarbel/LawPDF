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
    /// The check is animated only after this document's confirmed save.
    save_confirmation: Option<(u64, Instant)>,
    /// Review note shown in full, keyed as in `ReviewMarginEntry::key`.
    pub(super) expanded_note: Option<String>,
    /// Note under the pointer last frame, so callout and note light up together.
    pub(super) hovered_note: Option<String>,
    /// True while the current frame lays notes out in the margin.
    pub(super) margin_notes_live: bool,
}

impl ChromeState {
    pub(super) fn confirm_save(&mut self, epoch: u64) {
        self.save_confirmation = Some((epoch, Instant::now()));
    }
}

fn draw_save_check(painter: &egui::Painter, center: Pos2, progress: f32) {
    let start = center + Vec2::new(-4.5, 0.0);
    let corner = center + Vec2::new(-1.5, 3.5);
    let end = center + Vec2::new(5.0, -4.0);
    let first = start.distance(corner);
    let second = corner.distance(end);
    let length = progress.clamp(0.0, 1.0) * (first + second);
    let stroke = Stroke::new(1.8_f32, BRASS);
    if length > 0.0 {
        painter.line_segment(
            [start, start.lerp(corner, (length / first).min(1.0))],
            stroke,
        );
    }
    if length > first {
        painter.line_segment(
            [
                corner,
                corner.lerp(end, ((length - first) / second).min(1.0)),
            ],
            stroke,
        );
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct IdentityKey {
    epoch: u64,
    path: PathBuf,
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

pub(super) fn masthead_font(size: f32) -> FontId {
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
            .rect_filled(rect, 4.0, if ui.visuals().dark_mode {
                Color32::from_white_alpha(20)
            } else {
                Color32::from_rgba_unmultiplied(42, 38, 32, 12)
            });
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
    pub(super) fn masthead_identity_for_view(&mut self) -> MastheadIdentity {
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
            path: document.path.clone(),
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
            return self.search_state.open && self.search_state.split_results_open;
        }
        if self.view_mode == DocumentViewMode::LiquidMode2 && !self.review_pdf_split {
            self.settings.sidebar_in_review
        } else {
            self.settings.sidebar_in_pdf
        }
    }

    pub(super) fn set_sidebar_visible(&mut self, visible: bool) {
        if self.sbs_mode.is_some() {
            self.search_state.split_results_open = visible;
            return;
        }
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
            .frame(egui::Frame::NONE.fill(self.app_palette().bar))
            .show_separator_line(false)
            .show(ctx, |ui| {
                quiet_buttons(ui);
                ui.spacing_mut().item_spacing = Vec2::new(6.0, 0.0);
                self.draw_tab_row(ui, ctx);
                ui.add_space(12.0);
                self.draw_identity_row(ui, &identity);
                self.draw_action_row(ui, ctx);
                if self.view_mode == DocumentViewMode::LiquidMode2 {
                    self.draw_review_feedback_switch(ui);
                }
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
        let mut close_others = None;
        let mut copy_path = None;
        let mut reveal_path = None;
        let mut drop_tab = None;
        let mut tab_rects = Vec::new();
        let mut open_new = false;
        let tab_width = ((row.width() - 220.0) / self.tabs.len().max(1) as f32).clamp(84.0, 250.0);
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
                    let epoch = self.tabs[index].document_epoch;
                    let (action, rect) = ui
                        .push_id(("document-tab", epoch), |ui| {
                            self.draw_tab(ui, index, tab_width)
                        })
                        .inner;
                    tab_rects.push(rect);
                    match action {
                        TabAction::Switch => switch_to = Some(index),
                        TabAction::Close => close_tab = Some(index),
                        TabAction::CloseOthers => close_others = Some(epoch),
                        TabAction::CopyPath => copy_path = Some(index),
                        TabAction::RevealPath => reveal_path = Some(index),
                        TabAction::Drop => drop_tab = Some(index),
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
                    support_ui::draw_coffee_button(ui);
                    ui.add_space(6.0);
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
        } else if let Some(epoch) = close_others {
            self.close_other_tabs(epoch, ctx);
        } else if let Some(index) = copy_path {
            ctx.copy_text(self.tabs[index].document.path.display().to_string());
            self.status = "Copied document path.".to_owned();
        } else if let Some(index) = reveal_path {
            if let Err(error) = reveal_document_in_file_manager(&self.tabs[index].document.path) {
                self.push_error_notice(error);
            }
        } else if let Some(index) = drop_tab {
            if let Some(target) = ctx
                .input(|input| input.pointer.latest_pos())
                .and_then(|pos| tab_rects.iter().position(|rect| rect.contains(pos)))
            {
                self.reorder_tab(index, target);
            }
        } else if let Some(index) = switch_to {
            self.switch_to_tab(index, ctx);
        }
    }

    fn reorder_tab(&mut self, source: usize, target: usize) {
        if source >= self.tabs.len() || target >= self.tabs.len() || source == target {
            return;
        }
        let active = self
            .active_tab
            .and_then(|index| self.tabs.get(index))
            .map(|tab| tab.document_epoch);
        let tab = self.tabs.remove(source);
        self.tabs.insert(target, tab);
        self.active_tab =
            active.and_then(|epoch| self.tabs.iter().position(|tab| tab.document_epoch == epoch));
    }

    fn draw_tab(&mut self, ui: &mut egui::Ui, index: usize, max_width: f32) -> (TabAction, Rect) {
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
        let text_color = if is_active { self.app_palette().ink } else { NAVY_MUTED };
        let dot = if dirty || is_active { 13.0 } else { 0.0 };
        let side_galley = sbs_side.map(|side| {
            ui.fonts_mut(|fonts| {
                fonts.layout_no_wrap(
                    format!("  ·  {side}"),
                    FontId::proportional(14.5),
                    text_color,
                )
            })
        });
        let side_width = side_galley.as_ref().map_or(0.0, |galley| galley.size().x);
        let mut job = LayoutJob::simple_singleline(
            full_title.clone(),
            FontId::proportional(14.5),
            text_color,
        );
        job.wrap.max_width = (max_width - dot - side_width - 42.0).max(30.0);
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
        let size = Vec2::new(galley.size().x + side_width + dot + 16.0 + 26.0, 32.0);
        let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
        let fill = if is_active {
            self.app_palette().bar
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
            let dot_color = if dirty { self.app_palette().accent } else { BRASS };
            ui.painter()
                .circle_filled(Pos2::new(x + 3.5, rect.center().y), 3.5, dot_color);
            x += dot;
        }
        let title_width = galley.size().x;
        ui.painter().galley(
            Pos2::new(x, rect.center().y - galley.size().y * 0.5),
            galley,
            text_color,
        );
        if let Some(side) = side_galley {
            ui.painter().galley(
                Pos2::new(x + title_width, rect.center().y - side.size().y * 0.5),
                side,
                text_color,
            );
        }
        let close_rect = Rect::from_center_size(
            Pos2::new(rect.right() - 15.0, rect.center().y),
            Vec2::splat(18.0),
        );
        let close = ui.interact(close_rect, response.id.with("close"), Sense::click());
        if close.hovered() {
            ui.painter().circle_filled(
                close_rect.center(),
                9.0,
                if is_active && !ui.visuals().dark_mode {
                    Color32::from_rgba_unmultiplied(42, 38, 32, 22)
                } else {
                    Color32::from_rgba_unmultiplied(255, 255, 255, 30)
                },
            );
        }
        let cross = if is_active { self.app_palette().muted } else { NAVY_MUTED };
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
        let mut menu_action = TabAction::None;
        response.context_menu(|ui| {
            if ui.button("Close other tabs").clicked() {
                menu_action = TabAction::CloseOthers;
                ui.close();
            }
            if ui.button("Show in folder").clicked() {
                menu_action = TabAction::RevealPath;
                ui.close();
            }
            if ui.button("Copy path").clicked() {
                menu_action = TabAction::CopyPath;
                ui.close();
            }
        });
        let response = response
            .on_hover_text(if dirty {
                format!("{path}\nUnsaved annotations")
            } else {
                path
            })
            .on_hover_cursor(CursorIcon::PointingHand);
        let action = if menu_action != TabAction::None {
            menu_action
        } else if close.clicked()
            || close.clicked_by(egui::PointerButton::Middle)
            || response.clicked_by(egui::PointerButton::Middle)
        {
            TabAction::Close
        } else if response.drag_stopped_by(egui::PointerButton::Primary) {
            TabAction::Drop
        } else if response.clicked() {
            TabAction::Switch
        } else {
            TabAction::None
        };
        (action, rect)
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
                color: self.app_palette().ink,
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
        ui.painter().galley(title_pos, title, self.app_palette().ink);

        let flank_width = ((inner.width() - title_rect.width()) * 0.5 - 28.0).max(60.0);
        let authors = ui.fonts_mut(|fonts| {
            fonts.layout_job(small_caps_job(
                &identity.authors,
                13.0,
                self.app_palette().muted,
                flank_width,
            ))
        });
        let citation = ui.fonts_mut(|fonts| {
            fonts.layout_job(small_caps_job(
                &identity.citation,
                13.0,
                self.app_palette().muted,
                flank_width,
            ))
        });
        let flank_y = baseline_y - authors.size().y.max(citation.size().y) + 1.0;
        let authors_rect = Rect::from_min_size(Pos2::new(inner.left(), flank_y), authors.size());
        let citation_rect = Rect::from_min_size(
            Pos2::new(inner.right() - citation.size().x, flank_y),
            citation.size(),
        );
        ui.painter().galley(authors_rect.min, authors, self.app_palette().muted);
        ui.painter().galley(citation_rect.min, citation, self.app_palette().muted);
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
            .hline(inner.x_range(), y, Stroke::new(1.3_f32, self.app_palette().ink));
        ui.painter()
            .hline(inner.x_range(), y + 3.0, Stroke::new(0.8_f32, self.app_palette().ink));
    }

    fn draw_action_row(&mut self, ui: &mut egui::Ui, ctx: &Context) {
        let (row, _) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), ACTION_ROW_HEIGHT),
            Sense::hover(),
        );
        let inner = row.shrink2(Vec2::new(26.0, 0.0));
        let has_document = self.document.is_some();

        if self.search_state.open && has_document {
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(inner)
                    .layout(egui::Layout::left_to_right(Align::Center)),
                |ui| {
                    ui.spacing_mut().item_spacing.x = 5.;
                    let scope = if self.sbs_mode.is_some() {
                        if self
                            .sbs_mode
                            .is_some_and(|m| m.left_epoch == self.document_epoch)
                        {
                            "Left PDF"
                        } else {
                            "Right PDF"
                        }
                    } else if self.find_scope() == search_state::FindScope::Review {
                        "Article"
                    } else {
                        "PDF"
                    };
                    ui.label(
                        RichText::new(format!("Find in {scope}"))
                            .size(13.)
                            .color(self.app_palette().muted),
                    );
                    let width = (inner.width() * 0.24).clamp(54., 300.);
                    self.draw_find_field(ui, ctx, true, width);
                    let count = self.find_progress();
                    ui.add_sized(
                        [if inner.width() > 700. { 125. } else { 78. }, 26.],
                        egui::Label::new(RichText::new(&count).size(13.).color(self.app_palette().muted))
                            .truncate(),
                    )
                    .on_hover_text(&count);
                    for (back, icon, tip) in [
                        (true, "↑", "Previous match (Shift+Enter)"),
                        (false, "↓", "Next match (Enter)"),
                    ] {
                        let button = ui
                            .add_enabled(
                                !self.search_state.query.trim().is_empty(),
                                egui::Button::new(RichText::new(icon).size(17.))
                                    .min_size(Vec2::new(28., 26.)),
                            )
                            .on_hover_text(tip);
                        ui.ctx()
                            .accesskit_node_builder(button.id, |node| node.set_label(tip));
                        if button.clicked() {
                            self.find_step(ctx, back);
                            ui.memory_mut(|m| m.request_focus(egui::Id::new("lawpdf-find")));
                        }
                    }
                    if ui
                        .small_button(if inner.width() < 590. {
                            "List"
                        } else {
                            "All results"
                        })
                        .clicked()
                    {
                        let already =
                            self.sidebar_visible() && self.sidebar_tab == SidebarTab::Search;
                        if !already {
                            self.search_state.follow_result = true;
                            self.search_state.close_sidebar = !self.sidebar_visible();
                            self.sidebar_tab = SidebarTab::Search;
                        }
                        self.set_sidebar_visible(!already);
                    }
                    let close = ui
                        .add(
                            egui::Button::new(RichText::new("×").size(17.))
                                .min_size(Vec2::new(28., 26.)),
                        )
                        .on_hover_text("Close Find (Esc)");
                    ui.ctx()
                        .accesskit_node_builder(close.id, |n| n.set_label("Close Find"));
                    if close.clicked() {
                        self.dismiss_search(ctx);
                    }
                },
            );
            return;
        }
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
                    .layout_job(small_caps_job(label, 12.5, self.app_palette().ink, f32::INFINITY))
                    .size()
                    .x
                    + 10.0
            })
        })
        .sum::<f32>()
            + 2.0 * ui.spacing().item_spacing.x;
        ui.add_space(((width - item_width) * 0.5).max(0.0));
        let palette = self.app_palette();
        let color = |active: bool| if active { palette.accent } else { palette.muted };
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
            motion::busy(ui, self.settings.reduce_motion).on_hover_text("Preparing Review Mode");
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
        let pdf_outline = (!in_review)
            .then(|| self.pdf_outlines.get(&self.document_epoch).cloned())
            .flatten()
            .filter(|outline| !outline.is_empty());
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
        let mut job = small_caps_job("Contents", 12.5, self.app_palette().accent, f32::INFINITY);
        job.append(
            "   ",
            0.0,
            TextFormat {
                font_id: FontId::proportional(15.0),
                color: self.app_palette().ink,
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
                color: self.app_palette().ink,
                italics: true,
                ..Default::default()
            },
        );
        job.wrap.max_rows = 1;
        let mut clicked_target = None;
        let mut clicked_pdf_target = None;
        let mut open_review = false;
        let (response, _) =
            egui::containers::menu::MenuButton::from_button(egui::Button::new(job).frame(false))
                .ui(ui, |ui| {
                    ui.set_max_width(420.0);
                    ui.set_max_height(460.0);
                    if let Some(outline) = &pdf_outline {
                        egui::ScrollArea::vertical()
                            .id_salt("masthead_pdf_contents")
                            .max_height(440.0)
                            .show(ui, |ui| {
                                for item in outline.iter() {
                                    ui.horizontal(|ui| {
                                        ui.add_space(14.0 * item.level.min(8) as f32);
                                        let button = egui::Button::new(
                                            RichText::new(&item.title).size(14.5),
                                        )
                                        .frame(false)
                                        .wrap();
                                        if ui
                                            .add_enabled(item.page_index.is_some(), button)
                                            .clicked()
                                        {
                                            clicked_pdf_target = item
                                                .page_index
                                                .map(|page| (page, item.page_fraction));
                                            ui.close();
                                        }
                                    });
                                }
                            });
                    } else {
                        match &outline {
                            Some(outline) if !outline.is_empty() => {
                                egui::ScrollArea::vertical()
                                    .id_salt("masthead_contents")
                                    .max_height(440.0)
                                    .show(ui, |ui| {
                                        for item in outline.iter() {
                                            let active = in_review
                                                && self.review_active_heading
                                                    == Some(item.block_index);
                                            ui.horizontal(|ui| {
                                                ui.add_space(
                                                    14.0 * item.level.saturating_sub(1) as f32,
                                                );
                                                let text = RichText::new(&item.text)
                                                    .size(if item.level == 1 { 15.5 } else { 14.5 })
                                                    .color(if active {
                                                        self.app_palette().accent
                                                    } else if item.level == 1 {
                                                        self.app_palette().ink
                                                    } else {
                                                        self.app_palette().muted
                                                    });
                                                if ui
                                                    .add(
                                                        egui::Button::new(text).frame(false).wrap(),
                                                    )
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
                                    RichText::new("No section headings detected.").color(self.app_palette().muted),
                                );
                            }
                            None => {
                                ui.label(
                                RichText::new(
                                    "Review Mode builds the contents from the article's headings.",
                                )
                                .color(self.app_palette().muted),
                            );
                                if self.document.is_some()
                                    && ui.button("Open Review Mode").clicked()
                                {
                                    open_review = true;
                                    ui.close();
                                }
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
                Stroke::new(1.3_f32, self.app_palette().muted),
            );
        }
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("Contents: {place}"))
        });
        if open_review {
            self.set_view_mode(DocumentViewMode::LiquidMode2, ctx);
        }
        if let Some(block_index) = clicked_target {
            self.record_jump_origin();
            if self.view_mode != DocumentViewMode::LiquidMode2 {
                self.set_view_mode(DocumentViewMode::LiquidMode2, ctx);
            }
            self.liquid_scroll_to_block = Some(block_index);
            self.review_active_heading = Some(block_index);
            ctx.request_repaint();
        }
        if let Some((page_index, fraction)) = clicked_pdf_target {
            self.jump_to_pdf_destination(page_index, fraction);
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
        let id = egui::Id::new("lawpdf-find");
        let focus_request = self.search_state.focus_request;
        if focus_request {
            ui.memory_mut(|m| m.request_focus(id));
        }
        let enter = if ui.memory(|m| m.has_focus(id)) {
            ui.input_mut(|i| {
                if i.consume_key(egui::Modifiers::SHIFT, egui::Key::Enter) {
                    Some(true)
                } else if i.consume_key(egui::Modifiers::NONE, egui::Key::Enter) {
                    Some(false)
                } else {
                    None
                }
            })
        } else {
            None
        };
        let mut output = egui::TextEdit::singleline(&mut self.search_state.query)
            .id(id)
            .hint_text("Find…")
            .desired_width(width)
            .interactive(has_document)
            .margin(Margin::symmetric(8, 5))
            .show(ui);
        ui.ctx()
            .accesskit_node_builder(id, |n| n.set_label("Find in the current view"));
        if focus_request {
            let end = egui::text::CCursor::new(self.search_state.query.chars().count());
            output
                .state
                .cursor
                .set_char_range(Some(egui::text::CCursorRange::two(
                    egui::text::CCursor::new(0),
                    end,
                )));
            output.state.store(ctx, id);
            self.search_state.focus_request = false;
        }
        if has_document && (output.response.has_focus() || output.response.clicked()) {
            self.search_state.open = true;
        }
        if output.response.changed() {
            self.invalidate_search_results();
            self.search_state.open = true;
            self.search_state.show_highlights = true;
            self.search_state.due = Some(Instant::now() + Duration::from_millis(65));
            ctx.request_repaint_after(Duration::from_millis(65));
        }
        if let Some(back) = enter {
            self.find_step(ctx, back);
        }
    }

    fn draw_file_actions(&mut self, ui: &mut egui::Ui, ctx: &Context, has_document: bool) {
        // Right-to-left: the last added sits leftmost.
        #[cfg(target_os = "windows")]
        if ui
            .add_enabled(
                has_document && !self.print_ui.busy,
                egui::Button::new("Print…").min_size(Vec2::new(64.0, 30.0)),
            )
            .on_hover_text("Print this PDF · Ctrl+P")
            .clicked()
        {
            self.request_print(ctx);
        }
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
            motion::busy(ui, self.settings.reduce_motion);
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
                #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
                if ui
                    .add_enabled(
                        !self.print_ui.busy,
                        egui::Button::new(if cfg!(target_os = "macos") {
                            "Print…   ⌘P"
                        } else {
                            "Print…   Ctrl+P"
                        }),
                    )
                    .on_hover_text("Print the original PDF with its current annotations")
                    .clicked()
                {
                    self.request_print(ctx);
                    ui.close();
                }
                if ui.button("Save PDF copy").clicked() {
                    self.save_copy_dialog(ctx);
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
        if ui
            .add_enabled(
                has_document && !self.recovery_is_busy() && !self.page_rotation_in_flight,
                egui::Button::new("Save As…"),
            )
            .on_hover_text(
                "Choose a filename and folder, then continue editing there\nCtrl/Cmd+Shift+S",
            )
            .clicked()
        {
            self.save_as_dialog(ctx);
        }
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
                    let paper = ui.selectable_value(&mut self.liquid_theme, LiquidTheme::Paper, "Paper").clicked();
                    let sepia = ui.selectable_value(&mut self.liquid_theme, LiquidTheme::Sepia, "Sepia").clicked();
                    let dark = ui.selectable_value(&mut self.liquid_theme, LiquidTheme::Dark, "Dark").clicked();
                    if paper || sepia || dark { self.liquid_theme_follows_appearance = false; }
                });
                ui.separator();
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
                    self.liquid_theme_follows_appearance = true;
                    self.liquid_theme = if ui.ctx().theme() == egui::Theme::Dark { LiquidTheme::Dark } else { LiquidTheme::Paper };
                }
                let Some(document) = review_document.as_deref() else {
                    return;
                };
                ui.separator();
                self.draw_speech_menu(ui, document);
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
            let ocr_active = self.ocr_is_active();
            if ui
                .add_enabled(
                    has_document && !ocr_active,
                    egui::Button::new("OCR: save a searchable copy…"),
                )
                .on_hover_text(super::LOCAL_OCR_SAVE_HOVER)
                .clicked()
            {
                self.sidebar_tab = SidebarTab::Search;
                self.start_local_ocr_save();
                ui.close();
            }
            if ui
                .add_enabled(
                    has_document && !ocr_active,
                    egui::Button::new("Cloud OCR (OpenRouter): save a searchable copy…"),
                )
                .on_hover_text(super::CLOUD_OCR_SAVE_HOVER)
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
            if ui.button("Comment name…").clicked() {
                self.open_comment_profile();
                ui.close();
            }
            #[cfg(target_os = "linux")]
            if ui.button("Make LawPDF the default PDF reader").clicked() {
                self.request_linux_default_reader(ctx);
                ui.close();
            }
            if ui.button("Settings…").clicked() {
                self.settings_ui.open = true;
                ui.close();
            }
            if ui.button("Keyboard shortcuts…    ?").clicked() {
                self.shortcuts_open = true;
                ui.close();
            }
            ui.label(RichText::new(APP_VERSION_LABEL).size(12.0).color(self.app_palette().muted));
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
                    .fill(self.app_palette().bar)
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
                        self.app_palette().ink
                    } else {
                        self.app_palette().muted
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
                    .circle_stroke(center, 11.0, Stroke::new(1.5_f32, self.app_palette().ink));
            } else if response.hovered() {
                ui.painter()
                    .circle_stroke(center, 11.0, Stroke::new(1.0_f32, self.app_palette().rule));
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
                motion::busy(ui, self.settings.reduce_motion)
                    .on_hover_text("Saving page rotation…");
            }
        }
        if self.view_mode == DocumentViewMode::LiquidMode2 {
            let review_document = match &self.liquid_mode2_state {
                LiquidState::Ready(document) => Some(Arc::clone(document)),
                _ => None,
            };
            let reading = self.tts_controller.is_active();
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
                } else {
                    "Read aloud using the selected speech source"
                },
            )
            .clicked()
            {
                if reading {
                    self.stop_liquid_tts();
                } else if let Some(document) = review_document.as_deref() {
                    self.start_liquid_tts(document);
                    ui.ctx().request_repaint();
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
            egui::Label::new(RichText::new(label).size(14.5).color(self.app_palette().ink)),
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
            self.jump_to_page(self.page_index.saturating_sub(1));
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
        if ui.ctx().data_mut(|data| {
            data.remove_temp::<bool>(egui::Id::new("focus-page-number"))
                .unwrap_or(false)
        }) {
            page_input_response.request_focus();
        }
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
                    self.jump_to_page(page_index);
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
                .color(self.app_palette().muted),
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
            self.jump_to_page(self.page_index + 1);
        }
    }

    fn draw_bar_save_state(&mut self, ui: &mut egui::Ui) {
        let saving = self.document.as_ref().is_some_and(|document| {
            self.pending_annotation_saves.contains_key(&document.path)
                || self.active_annotation_saves.contains_key(&document.path)
        });
        let (label, color) = if saving {
            ("Saving…", self.app_palette().muted)
        } else if self.annotations_dirty {
            ("Not saved", self.app_palette().accent)
        } else {
            ("Saved", self.app_palette().ink)
        };
        let (rect, response) = ui.allocate_exact_size(Vec2::new(78.0, 30.0), Sense::hover());
        let mark_center = Pos2::new(rect.left() + 8.0, rect.center().y);
        if saving {
            ui.painter()
                .circle_stroke(mark_center, 5.0, Stroke::new(1.4_f32, self.app_palette().muted));
        } else if self.annotations_dirty {
            ui.painter().circle_filled(mark_center, 4.0, self.app_palette().accent);
        } else {
            let progress = self
                .chrome
                .save_confirmation
                .filter(|(epoch, _)| *epoch == self.document_epoch)
                .map_or(1.0, |(_, started)| {
                    motion::save_progress(ui.ctx(), started, self.settings.reduce_motion)
                });
            draw_save_check(ui.painter(), mark_center, progress);
        }
        ui.painter().text(
            Pos2::new(rect.left() + 19.0, rect.center().y),
            Align2::LEFT_CENTER,
            label,
            FontId::proportional(15.0),
            color,
        );
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, label));
        let location = self
            .document
            .as_ref()
            .map(|document| document.path.display().to_string())
            .unwrap_or_default();
        response.on_hover_text(format!("Saving to: {location}\nAnnotations save automatically. Use Save As to choose another folder."));
    }

    /// Short-lived caption above the bar: the status line, when there is news.
    fn draw_status_caption(&mut self, ctx: &Context, bar: Rect) {
        let now = Instant::now();
        if self.status != self.chrome.last_status {
            self.chrome.last_status = self.status.clone();
            self.chrome.status_changed_at = Some(now);
        }
        #[cfg(target_os = "windows")]
        if self.print_ui.open {
            return;
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
                self.app_palette().muted.gamma_multiply(alpha),
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
        painter.rect_filled(rect, 12.0, self.app_palette().bar.gamma_multiply(0.94 * alpha));
        painter.rect_stroke(
            rect,
            12.0,
            Stroke::new(1.0_f32, self.app_palette().rule.gamma_multiply(alpha)),
            egui::StrokeKind::Inside,
        );
        painter.galley(rect.min + Vec2::new(12.0, 5.0), galley, self.app_palette().muted);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TabAction {
    None,
    Switch,
    Close,
    CloseOthers,
    CopyPath,
    RevealPath,
    Drop,
}

fn reveal_document_in_file_manager(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = std::process::Command::new("explorer.exe");
        command.arg("/select,").arg(path);
        command
    };
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = std::process::Command::new("open");
        command.arg("-R").arg(path);
        command
    };
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let mut command = {
        let mut command = std::process::Command::new("xdg-open");
        command.arg(path.parent().unwrap_or(path));
        command
    };
    command
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("Could not show the document in its folder: {error}"))
}

/// Icon buttons without a resting fill or border: they show themselves on hover.
fn quiet_buttons(ui: &mut egui::Ui) {
    let dark = ui.visuals().dark_mode;
    let accent = theme::for_ui(ui).accent;
    let visuals = ui.visuals_mut();
    visuals.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
    visuals.widgets.inactive.bg_stroke = Stroke::NONE;
    visuals.widgets.hovered.weak_bg_fill = if dark { Color32::from_white_alpha(24) } else { Color32::from_rgba_unmultiplied(42, 38, 32, 16) };
    visuals.widgets.hovered.bg_stroke = Stroke::NONE;
    visuals.widgets.active.weak_bg_fill = if dark { Color32::from_white_alpha(40) } else { Color32::from_rgba_unmultiplied(42, 38, 32, 28) };
    visuals.selection.bg_fill = if dark { Color32::from_rgba_unmultiplied(241, 163, 155, 48) } else { Color32::from_rgba_unmultiplied(122, 31, 43, 30) };
    visuals.selection.stroke = Stroke::new(1.0_f32, accent);
}

fn bar_separator(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(9.0, 26.0), Sense::hover());
    ui.painter()
        .vline(rect.center().x, rect.y_range(), Stroke::new(1.0_f32, theme::for_ui(ui).rule));
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
    fn save_check_only_animates_after_the_active_documents_latest_success() {
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        let path = app.document.as_ref().unwrap().path.clone();
        let mut save = PendingAnnotationSave {
            document_epoch: app.document_epoch,
            path: path.clone(),
            generation: 1,
            annotations: app.annotations.clone(),
            due_at: Instant::now(),
        };
        app.annotations_dirty = true;
        app.active_annotation_saves
            .insert(path.clone(), save.clone());
        let mut newer = save.clone();
        newer.generation = 2;
        app.pending_annotation_saves
            .insert(path.clone(), newer.clone());
        app.finish_annotation_autosave(
            save.document_epoch,
            path.clone(),
            1,
            Ok(crate::pdf_backend::SaveReport::default()),
            &ctx,
        );
        assert!(app.annotations_dirty);
        assert!(app.chrome.save_confirmation.is_none());
        app.pending_annotation_saves.remove(&path);
        app.active_annotation_saves.insert(path.clone(), newer);
        app.finish_annotation_autosave(
            save.document_epoch,
            path.clone(),
            2,
            Err("test failure".to_owned()),
            &ctx,
        );
        assert!(app.annotations_dirty);
        assert!(app.chrome.save_confirmation.is_none());
        save.generation = 3;
        app.active_annotation_saves
            .insert(path.clone(), save.clone());
        app.finish_annotation_autosave(
            save.document_epoch,
            path.clone(),
            3,
            Ok(crate::pdf_backend::SaveReport::default()),
            &ctx,
        );
        assert!(!app.annotations_dirty);
        assert_eq!(
            app.chrome.save_confirmation.map(|(epoch, _)| epoch),
            Some(app.document_epoch)
        );

        // A completion for another tab, or a stale completion, cannot replay
        // the active tab's check mark.
        app.chrome.save_confirmation = None;
        let other_path = PathBuf::from("other.pdf");
        save.document_epoch += 1;
        save.path = other_path.clone();
        app.active_annotation_saves
            .insert(other_path.clone(), save.clone());
        app.finish_annotation_autosave(
            save.document_epoch,
            other_path,
            3,
            Ok(crate::pdf_backend::SaveReport::default()),
            &ctx,
        );
        assert!(app.chrome.save_confirmation.is_none());
    }

    #[test]
    fn save_check_draws_in_and_reduced_motion_shows_the_complete_mark() {
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        let frame = |app: &mut PdfEditorApp| {
            ctx.run(Default::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| app.draw_bar_save_state(ui));
            })
        };
        let segments = |output: &egui::FullOutput| {
            output
                .shapes
                .iter()
                .filter(|shape| matches!(shape.shape, egui::Shape::LineSegment { .. }))
                .count()
        };
        app.chrome.save_confirmation =
            Some((app.document_epoch, Instant::now() + Duration::from_secs(1)));
        app.settings.reduce_motion = false;
        assert_eq!(segments(&frame(&mut app)), 0);
        app.chrome.save_confirmation = Some((
            app.document_epoch,
            Instant::now() - Duration::from_millis(150),
        ));
        assert_eq!(segments(&frame(&mut app)), 2);
        app.settings.reduce_motion = true;
        app.chrome.save_confirmation =
            Some((app.document_epoch, Instant::now() + Duration::from_secs(1)));
        assert_eq!(segments(&frame(&mut app)), 2);
        for index in 0..8 {
            let output = frame(&mut app);
            if index >= 4 {
                assert_eq!(
                    output.viewport_output[&egui::ViewportId::ROOT].repaint_delay,
                    Duration::MAX
                );
            }
        }
    }

    #[test]
    fn static_saving_indicator_does_not_schedule_repaints() {
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        let path = app.document.as_ref().unwrap().path.clone();
        app.active_annotation_saves.insert(
            path.clone(),
            PendingAnnotationSave {
                document_epoch: 7,
                path,
                generation: 1,
                annotations: Vec::new(),
                due_at: Instant::now(),
            },
        );
        for frame in 0..8 {
            let output = ctx.run(
                egui::RawInput {
                    time: Some(frame as f64 * 0.2),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| app.draw_bar_save_state(ui));
                },
            );
            if frame >= 4 {
                assert_eq!(
                    output.viewport_output[&egui::ViewportId::ROOT].repaint_delay,
                    Duration::MAX
                );
            }
        }
    }

    fn tab_label_center(shape: &egui::Shape, title: &str) -> Option<Pos2> {
        match shape {
            egui::Shape::Text(shape) if shape.galley.text() == title => {
                Some(shape.pos + shape.galley.size() * 0.5)
            }
            egui::Shape::Vec(shapes) => shapes
                .iter()
                .find_map(|shape| tab_label_center(shape, title)),
            _ => None,
        }
    }

    fn tab_frame(
        app: &mut PdfEditorApp,
        ctx: &Context,
        time: f64,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 500.0))),
            time: Some(time),
            events,
            ..Default::default()
        };
        ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| app.draw_tab_row(ui, ctx));
        })
    }

    fn pointer_button(pos: Pos2, button: egui::PointerButton, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    #[test]
    fn middle_click_a_dirty_tab_keeps_it_open_for_a_save_decision() {
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        app.annotations_dirty = true;
        let output = tab_frame(&mut app, &ctx, 0.0, vec![]);
        let point = output
            .shapes
            .iter()
            .find_map(|shape| tab_label_center(&shape.shape, "Fixture"))
            .unwrap();
        tab_frame(&mut app, &ctx, 0.1, vec![egui::Event::PointerMoved(point)]);
        tab_frame(
            &mut app,
            &ctx,
            0.2,
            vec![pointer_button(point, egui::PointerButton::Middle, true)],
        );
        tab_frame(
            &mut app,
            &ctx,
            0.3,
            vec![pointer_button(point, egui::PointerButton::Middle, false)],
        );
        assert_eq!(app.pending_close, Some(CloseTarget::Tab(7)));
        assert_eq!(app.tabs.len(), 1);
        assert!(app.annotations_dirty);
    }

    #[test]
    fn tab_context_menu_copies_the_selected_documents_path() {
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        let expected = app.tabs[0].document.path.display().to_string();
        let output = tab_frame(&mut app, &ctx, 0.0, vec![]);
        let point = output
            .shapes
            .iter()
            .find_map(|shape| tab_label_center(&shape.shape, "Fixture"))
            .unwrap();
        tab_frame(&mut app, &ctx, 0.1, vec![egui::Event::PointerMoved(point)]);
        tab_frame(
            &mut app,
            &ctx,
            0.2,
            vec![pointer_button(point, egui::PointerButton::Secondary, true)],
        );
        tab_frame(
            &mut app,
            &ctx,
            0.3,
            vec![pointer_button(point, egui::PointerButton::Secondary, false)],
        );
        let output = tab_frame(&mut app, &ctx, 0.35, vec![]);
        let menu_point = output
            .shapes
            .iter()
            .find_map(|shape| tab_label_center(&shape.shape, "Copy path"))
            .expect("context menu opens");
        tab_frame(
            &mut app,
            &ctx,
            0.4,
            vec![egui::Event::PointerMoved(menu_point)],
        );
        tab_frame(
            &mut app,
            &ctx,
            0.5,
            vec![pointer_button(
                menu_point,
                egui::PointerButton::Primary,
                true,
            )],
        );
        let output = tab_frame(
            &mut app,
            &ctx,
            0.6,
            vec![pointer_button(
                menu_point,
                egui::PointerButton::Primary,
                false,
            )],
        );
        assert!(output.platform_output.commands.iter().any(
            |command| matches!(command, egui::OutputCommand::CopyText(text) if *text == expected)
        ));
    }

    #[test]
    fn tab_titles_are_elided_by_rendered_width() {
        fn elision(shape: &egui::Shape, text: &str) -> Option<bool> {
            match shape {
                egui::Shape::Text(shape) if shape.galley.text() == text => {
                    Some(shape.galley.elided)
                }
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| elision(shape, text)),
                _ => None,
            }
        }
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        for (title, expected) in [
            ("iiiiiiiiiiiiiiiiiiiiiiiii", false),
            ("WWWWWWWWWWWWWWWWWWWWWWWWW", true),
        ] {
            app.tabs[0].document.title = title.into();
            let output = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    app.draw_tab(ui, 0, 190.0);
                });
            });
            assert_eq!(
                output
                    .shapes
                    .iter()
                    .find_map(|shape| elision(&shape.shape, title)),
                Some(expected)
            );
        }
    }

    #[test]
    fn dragging_a_tab_reorders_it_without_changing_the_active_document() {
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        app.document.as_mut().unwrap().title = "Alpha".into();
        app.tabs[0].document.title = "Alpha".into();
        for (epoch, title) in [(8, "Bravo"), (9, "Charlie")] {
            let mut tab = app.tabs[0].clone();
            tab.document_epoch = epoch;
            tab.document.title = title.into();
            app.tabs.push(tab);
        }
        let output = tab_frame(&mut app, &ctx, 0.0, vec![]);
        let center = |title| {
            output
                .shapes
                .iter()
                .find_map(|shape| tab_label_center(&shape.shape, title))
                .unwrap()
        };
        let from = center("Alpha");
        let to = center("Charlie");
        tab_frame(&mut app, &ctx, 0.1, vec![egui::Event::PointerMoved(from)]);
        tab_frame(
            &mut app,
            &ctx,
            0.2,
            vec![pointer_button(from, egui::PointerButton::Primary, true)],
        );
        tab_frame(&mut app, &ctx, 0.3, vec![egui::Event::PointerMoved(to)]);
        tab_frame(
            &mut app,
            &ctx,
            0.4,
            vec![pointer_button(to, egui::PointerButton::Primary, false)],
        );
        assert_eq!(
            app.tabs
                .iter()
                .map(|tab| tab.document_epoch)
                .collect::<Vec<_>>(),
            vec![8, 9, 7]
        );
        assert_eq!(app.active_tab, Some(2));
        assert_eq!(app.document_epoch, 7);
    }

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
