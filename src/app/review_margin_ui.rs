//! Footnotes in the Review Mode margin.
//!
//! The body column is drawn first and records where each callout landed. The
//! notes are then placed beside those lines by `review_margin::place_margin_notes`,
//! which shortens long or crowded notes instead of pushing the text apart.
//! A shortened note ends in "more"; clicking it, or its callout, opens the whole
//! note over the margin without moving anything else.

use super::chrome::{BRASS, OXBLOOD, RULE, small_caps_job};
use super::*;
use crate::review_margin::{
    MarginNoteMetrics, MarginNotePlacement, MarginNoteRequest, MarginSide, assign_margin_sides,
    margin_notes_bottom, place_margin_notes,
};

const NOTE_TEXT_INSET: f32 = 12.0;
const NOTE_MAX_ROWS: usize = 9;
const NOTE_MIN_ROWS: usize = 3;
const NOTE_OPEN_MAX_WIDTH: f32 = 440.0;

/// One note waiting for a place, in document order.
#[derive(Debug, Clone)]
pub(super) struct ReviewMarginEntry {
    /// The note number as printed, or a synthetic key for unnumbered notes.
    pub(super) key: String,
    pub(super) marker: String,
    pub(super) body: String,
    pub(super) anchor_top: f32,
    /// The callout in the body text, when the note was reached through one.
    pub(super) callout: Option<Rect>,
}

#[derive(Debug, Default)]
pub(super) struct ReviewMarginCollector {
    entries: Vec<ReviewMarginEntry>,
    placed: HashSet<String>,
}

impl ReviewMarginCollector {
    /// Callouts found while drawing a body block. Each note appears once, at
    /// its first callout.
    pub(super) fn add_callouts(&mut self, hits: &[(Rect, u16)], index: &HashMap<u16, String>) {
        for (rect, number) in hits {
            let key = number.to_string();
            let Some(body) = index.get(number) else {
                continue;
            };
            if !self.placed.insert(key.clone()) {
                continue;
            }
            self.entries.push(ReviewMarginEntry {
                marker: key.clone(),
                key,
                body: body.clone(),
                anchor_top: rect.top(),
                callout: Some(*rect),
            });
        }
    }

    /// A note block met in reading order. Notes already placed through a
    /// callout are skipped; the rest sit where the block would have been.
    pub(super) fn add_note_block(&mut self, block_index: usize, text: &str, anchor_top: f32) {
        // The splitter labels any unnumbered note "*". Keep the star only when
        // the note really carries one; otherwise print no number at all.
        let starred = text.trim_start().starts_with(['*', '∗', '†', '‡']);
        for (ordinal, (marker, body)) in split_fused_review_notes(text).into_iter().enumerate() {
            let marker = if marker == "*" && !starred {
                String::new()
            } else {
                marker
            };
            let key = if marker.parse::<u16>().is_ok() {
                marker.clone()
            } else {
                format!("b{block_index}.{ordinal}")
            };
            let body = callout_body_text("Footnote", &body).trim().to_owned();
            if body.is_empty() {
                continue;
            }
            // An unnumbered fragment that starts mid-sentence is the rest of the
            // note before it, split by a page break. Rejoin it.
            if marker.parse::<u16>().is_err()
                && reads_as_continuation(&body)
                && let Some(previous) = self.entries.last_mut()
            {
                previous.body.push(' ');
                previous.body.push_str(&body);
                continue;
            }
            if !self.placed.insert(key.clone()) {
                continue;
            }
            self.entries.push(ReviewMarginEntry {
                key,
                marker,
                body,
                anchor_top,
                callout: None,
            });
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Text that continues a sentence rather than starting a note.
fn reads_as_continuation(body: &str) -> bool {
    body.chars().next().is_some_and(|first| {
        !(first.is_uppercase() || matches!(first, '“' | '"' | '‘' | '\'' | '*' | '†'))
    })
}

struct NoteColors {
    body: Color32,
    number: Color32,
    rule: Color32,
    hover_fill: Color32,
    open_fill: Color32,
}

impl PdfEditorApp {
    fn review_note_colors(&self) -> NoteColors {
        match self.liquid_theme {
            LiquidTheme::Paper => NoteColors {
                body: Color32::from_rgb(59, 54, 46),
                number: OXBLOOD,
                rule: RULE,
                hover_fill: Color32::from_rgba_unmultiplied(199, 165, 116, 26),
                open_fill: Color32::from_rgb(255, 253, 248),
            },
            LiquidTheme::Sepia => NoteColors {
                body: Color32::from_rgb(78, 59, 38),
                number: Color32::from_rgb(120, 52, 32),
                rule: Color32::from_rgb(214, 196, 160),
                hover_fill: Color32::from_rgba_unmultiplied(160, 120, 60, 30),
                open_fill: Color32::from_rgb(250, 242, 224),
            },
            LiquidTheme::Dark => NoteColors {
                body: Color32::from_rgb(207, 199, 182),
                number: Color32::from_rgb(216, 185, 127),
                rule: Color32::from_rgb(62, 70, 86),
                hover_fill: Color32::from_rgba_unmultiplied(216, 185, 127, 22),
                open_fill: Color32::from_rgb(34, 40, 52),
            },
        }
    }

    fn review_note_job(
        &self,
        entry: &ReviewMarginEntry,
        colors: &NoteColors,
        width: f32,
        max_rows: usize,
    ) -> LayoutJob {
        let scale = self.liquid_text_scale;
        let mut job = LayoutJob::default();
        if !entry.marker.is_empty() {
            job.append(
                &entry.marker,
                0.0,
                TextFormat {
                    font_id: FontId::proportional(11.0 * scale),
                    color: colors.number,
                    valign: Align::TOP,
                    ..Default::default()
                },
            );
        }
        job.append(
            &entry.body,
            if entry.marker.is_empty() { 0.0 } else { 4.0 },
            TextFormat {
                font_id: FontId::proportional(13.5 * scale),
                color: colors.body,
                line_height: Some(18.0 * scale),
                ..Default::default()
            },
        );
        job.wrap = egui::text::TextWrapping {
            max_width: width,
            max_rows,
            break_anywhere: false,
            overflow_character: Some('…'),
        };
        job
    }

    /// Place and paint the collected notes: right margin first, left when crowded.
    pub(super) fn draw_review_margin_notes(
        &mut self,
        ui: &mut egui::Ui,
        collector: ReviewMarginCollector,
        right_left: f32,
        left_left: f32,
        width: f32,
    ) {
        if collector.is_empty() || width <= 0.0 {
            return;
        }
        let scale = self.liquid_text_scale;
        let colors = self.review_note_colors();
        let text_width = (width - NOTE_TEXT_INSET).max(40.0);
        let mut entries = collector.entries;
        entries.sort_by(|a, b| a.anchor_top.total_cmp(&b.anchor_top));

        let full_rows = entries
            .iter()
            .map(|entry| {
                let job = self.review_note_job(entry, &colors, text_width, usize::MAX);
                ui.fonts_mut(|fonts| fonts.layout_job(job))
                    .rows
                    .len()
                    .max(1)
            })
            .collect::<Vec<_>>();
        let row_height = 18.0 * scale;
        let metrics = MarginNoteMetrics {
            row_height,
            chrome_height: 4.0,
            more_height: 17.0 * scale,
            gap: 12.0 * scale,
            max_rows: NOTE_MAX_ROWS,
            min_rows: NOTE_MIN_ROWS,
            drift_limit: row_height * 14.0,
        };
        let requests = entries
            .iter()
            .zip(&full_rows)
            .map(|(entry, rows)| MarginNoteRequest {
                anchor_top: entry.anchor_top,
                total_rows: *rows,
            })
            .collect::<Vec<_>>();
        let sides = assign_margin_sides(&requests, &metrics, row_height * 3.0);
        let mut placements = vec![
            MarginNotePlacement {
                top: 0.0,
                height: 0.0,
                rows: 1,
                shortened: false,
            };
            requests.len()
        ];
        for side in [MarginSide::Right, MarginSide::Left] {
            let indices = (0..requests.len())
                .filter(|index| sides[*index] == side)
                .collect::<Vec<_>>();
            let side_requests = indices
                .iter()
                .map(|index| requests[*index])
                .collect::<Vec<_>>();
            for (index, placement) in indices
                .iter()
                .zip(place_margin_notes(&side_requests, &metrics))
            {
                placements[*index] = placement;
            }
        }

        let pointer = ui.input(|input| input.pointer.hover_pos());
        let clip = ui.clip_rect();
        let mut hovered = None;
        let mut toggled = None;
        let previous_hover = self.chrome.hovered_note.clone();
        let expanded = self.chrome.expanded_note.clone();
        let mut note_rects = Vec::with_capacity(entries.len());

        for ((entry, placement), side) in entries.iter().zip(&placements).zip(&sides) {
            let left_side = *side == MarginSide::Left;
            let left = if left_side { left_left } else { right_left };
            let rect = Rect::from_min_size(
                Pos2::new(left, placement.top),
                Vec2::new(width, placement.height),
            );
            note_rects.push(rect);
            if !rect.intersects(clip.expand(200.0)) {
                continue;
            }
            let id = ui.id().with(("review-margin-note", &entry.key));
            let response = ui.interact(rect, id, Sense::click());
            let over_callout = entry
                .callout
                .zip(pointer)
                .is_some_and(|(callout, pointer)| callout.expand(3.0).contains(pointer));
            let lit = response.hovered()
                || over_callout
                || previous_hover.as_deref() == Some(entry.key.as_str())
                || expanded.as_deref() == Some(entry.key.as_str());
            if response.hovered() || over_callout {
                hovered = Some(entry.key.clone());
            }
            let painter = ui.painter();
            if response.hovered() {
                painter.rect_filled(rect.expand2(Vec2::new(4.0, 3.0)), 4.0, colors.hover_fill);
            }
            // The hairline always faces the text column.
            painter.vline(
                if left_side {
                    rect.right() - 0.5
                } else {
                    rect.left() + 0.5
                },
                rect.top() + 3.0..=rect.bottom() - 3.0,
                if lit {
                    Stroke::new(2.0_f32, colors.number)
                } else {
                    Stroke::new(1.0_f32, colors.rule)
                },
            );
            let job = self.review_note_job(entry, &colors, text_width, placement.rows);
            let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
            let text_left = if left_side {
                rect.left()
            } else {
                rect.left() + NOTE_TEXT_INSET
            };
            painter.galley(Pos2::new(text_left, rect.top() + 2.0), galley, colors.body);
            if placement.shortened {
                let more = ui.fonts_mut(|fonts| {
                    fonts.layout_job(small_caps_job("more", 10.5 * scale, colors.number, 80.0))
                });
                painter.galley(
                    Pos2::new(text_left, rect.bottom() - metrics.more_height + 3.0),
                    more,
                    colors.number,
                );
            }
            if lit && let Some(callout) = entry.callout {
                painter.rect_stroke(
                    callout.expand2(Vec2::new(2.5, 1.0)),
                    3.0,
                    Stroke::new(1.2_f32, colors.number),
                    egui::StrokeKind::Outside,
                );
            }
            response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Button,
                    true,
                    format!("Footnote {}: {}", entry.marker, entry.body),
                )
            });
            let response = if placement.shortened {
                response
                    .on_hover_cursor(CursorIcon::PointingHand)
                    .on_hover_text("Show the whole note")
            } else {
                response
            };
            if response.clicked() {
                toggled = Some(entry.key.clone());
            }
        }

        if let Some(bottom) = margin_notes_bottom(&placements) {
            let below = bottom - ui.cursor().top();
            if below > 0.0 {
                ui.add_space(below + 16.0);
            }
        }
        if hovered != self.chrome.hovered_note {
            self.chrome.hovered_note = hovered;
            ui.ctx().request_repaint();
        }
        let toggled_now = toggled.is_some();
        if let Some(key) = toggled {
            self.chrome.expanded_note = if self.chrome.expanded_note.as_deref() == Some(&key) {
                None
            } else {
                Some(key)
            };
        }
        let open = self.chrome.expanded_note.clone().and_then(|key| {
            entries
                .iter()
                .zip(&note_rects)
                .zip(&sides)
                .find(|((entry, _), _)| entry.key == key)
                .map(|((entry, rect), side)| (entry.clone(), *rect, *side))
        });
        if let Some((entry, rect, side)) = open {
            let just_opened = toggled_now || expanded.as_deref() != Some(entry.key.as_str());
            self.draw_open_review_note(ui, &entry, rect, side, &colors, just_opened);
        }
    }

    /// The whole note, floated over the margin and anchored to its short form.
    fn draw_open_review_note(
        &mut self,
        ui: &mut egui::Ui,
        entry: &ReviewMarginEntry,
        anchor: Rect,
        side: MarginSide,
        colors: &NoteColors,
        just_opened: bool,
    ) {
        let clip = ui.clip_rect();
        if anchor.bottom() < clip.top() - 40.0 || anchor.top() > clip.bottom() + 40.0 {
            self.chrome.expanded_note = None;
            return;
        }
        let scale = self.liquid_text_scale;
        let screen = ui.ctx().content_rect();
        // Open outward, over the margin and the canvas, never across the text.
        let (left, width) = match side {
            MarginSide::Right => {
                let width = NOTE_OPEN_MAX_WIDTH
                    .min(screen.right() - anchor.left() - 4.0)
                    .max(anchor.width() + 20.0);
                (anchor.left() - 10.0, width)
            }
            MarginSide::Left => {
                let right = anchor.right() + 10.0;
                let width = NOTE_OPEN_MAX_WIDTH
                    .min(right - screen.left() - 4.0)
                    .max(anchor.width() + 20.0);
                ((right - width).max(screen.left() + 4.0), width)
            }
        };
        let top = anchor.top().max(clip.top() + 8.0) - 8.0;
        let max_height = (clip.bottom() - top - 24.0).max(120.0);
        let area = egui::Area::new(ui.id().with(("review-note-open", &entry.key)))
            .order(egui::Order::Foreground)
            .fixed_pos(Pos2::new(left, top))
            .show(ui.ctx(), |ui| {
                egui::Frame::NONE
                    .fill(colors.open_fill)
                    .stroke(Stroke::new(1.0_f32, BRASS))
                    .corner_radius(6)
                    .shadow(egui::epaint::Shadow {
                        offset: [0, 8],
                        blur: 24,
                        spread: 0,
                        color: Color32::from_rgba_unmultiplied(28, 24, 18, 46),
                    })
                    .inner_margin(Margin::symmetric(14, 10))
                    .show(ui, |ui| {
                        ui.set_width(width - 28.0);
                        ui.horizontal(|ui| {
                            let heading = ui.fonts_mut(|fonts| {
                                fonts.layout_job(small_caps_job(
                                    &if entry.marker.is_empty() {
                                        "Note".to_owned()
                                    } else {
                                        format!("Note {}", entry.marker)
                                    },
                                    11.0 * scale,
                                    colors.number,
                                    width,
                                ))
                            });
                            ui.label(heading);
                            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                                let close = ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new("×").size(16.0).color(colors.body),
                                        )
                                        .frame(false),
                                    )
                                    .on_hover_text("Close (Esc)");
                                close.widget_info(|| {
                                    egui::WidgetInfo::labeled(
                                        egui::WidgetType::Button,
                                        true,
                                        "Close note",
                                    )
                                });
                                if close.clicked() {
                                    self.chrome.expanded_note = None;
                                }
                            });
                        });
                        ui.add_space(4.0);
                        egui::ScrollArea::vertical()
                            .id_salt(("review-note-open-scroll", &entry.key))
                            .max_height(max_height)
                            .show(ui, |ui| {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&entry.body)
                                            .size(15.0 * scale)
                                            .color(colors.body),
                                    )
                                    .wrap()
                                    .selectable(true),
                                );
                            });
                    });
            });
        let area_rect = area.response.rect;
        let callout = entry.callout.map(|rect| rect.expand(3.0));
        let clicked_outside = !just_opened
            && ui.input(|input| {
                input.pointer.any_pressed()
                    && input.pointer.interact_pos().is_some_and(|pos| {
                        !area_rect.contains(pos)
                            && !anchor.contains(pos)
                            && !callout.is_some_and(|rect| rect.contains(pos))
                    })
            });
        let escape = ui.input(|input| input.key_pressed(egui::Key::Escape));
        if clicked_outside || escape {
            self.chrome.expanded_note = None;
        }
    }
}
