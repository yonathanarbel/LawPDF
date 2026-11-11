use super::search_state::WASH;
use super::*;

impl PdfEditorApp {
    pub(super) fn landing_page(&self) -> Option<usize> {
        if !self.search_state.landing {
            return None;
        }
        let h = self
            .search_state
            .hits
            .get(self.search_state.selected_hit?)?;
        (h.source != SearchSource::ReviewText).then_some(h.page_index)
    }

    pub(super) fn draw_find_page(
        &mut self,
        ui: &egui::Ui,
        painter: &egui::Painter,
        placement: PagePlacement,
        page: usize,
    ) {
        if !self.search_state.open || !self.search_state.show_highlights {
            return;
        }
        let hits = &self.search_state.hits;
        let start = hits.partition_point(|h| h.page_index < page);
        let end = start + hits[start..].partition_point(|h| h.page_index == page);
        if start == end {
            return;
        }
        let Some(doc) = self.document.as_ref() else {
            return;
        };
        let geometry_ready = doc.text_chars.get(page).and_then(Option::as_ref).is_some();
        if geometry_ready && !self.search_state.geometry.contains_key(&page) {
            let text = &doc.native_text[page];
            let chars = doc.text_chars[page].as_ref().unwrap();
            let boundaries = text.char_indices().map(|(i, _)| i).collect::<Vec<_>>();
            let mut boxes = HashMap::new();
            for h in &hits[start..end] {
                if h.source != SearchSource::NativeText {
                    continue;
                }
                let a = boundaries
                    .partition_point(|&b| b < h.match_start)
                    .min(chars.len());
                let b = boundaries
                    .partition_point(|&b| b < h.match_end)
                    .min(chars.len());
                boxes.insert(
                    h.match_start,
                    merge_text_rects(
                        chars[a..b]
                            .iter()
                            .filter(|c| !c.ch.is_control())
                            .filter_map(|c| c.rect),
                    ),
                );
            }
            if self.search_state.geometry.len() >= 32 {
                self.search_state.geometry.clear();
            }
            self.search_state.geometry.insert(page, boxes);
        } else if !geometry_ready
            && hits[start..end]
                .iter()
                .any(|h| h.source == SearchSource::NativeText)
        {
            let path = doc.path.clone();
            self.request_text_chars(ui.ctx(), &path, page);
        }
        for i in start..end {
            let h = &self.search_state.hits[i];
            let active = self.search_state.selected_hit == Some(i);
            let boxes = self
                .search_state
                .geometry
                .get(&page)
                .and_then(|m| m.get(&h.match_start))
                .filter(|_| h.source == SearchSource::NativeText);
            if active
                && self.search_state.landing
                && (geometry_ready || h.source != SearchSource::NativeText)
            {
                if let Some(first) = boxes.and_then(|v| v.first()) {
                    scroll_to_passage(
                        ui,
                        placement.pdf_rect_to_screen(*first),
                        self.settings.reduce_motion,
                    );
                } else if !ui.clip_rect().intersects(placement.rect) {
                    let clip = ui.clip_rect();
                    ui.scroll_to_rect_animation(
                        placement.rect,
                        Some(Align::TOP),
                        motion::jump_animation(
                            placement.rect.top() - clip.top(),
                            clip.height(),
                            self.settings.reduce_motion,
                        ),
                    );
                }
                self.search_state.landing = false;
                self.search_state.activated = Some(Instant::now());
            }
            for rect in boxes.into_iter().flatten() {
                paint_find_rect(
                    painter,
                    placement.pdf_rect_to_screen(*rect),
                    active,
                    self.search_state.activated,
                    self.settings.reduce_motion,
                );
            }
        }
    }

    pub(super) fn find_rail(&self, painter: &egui::Painter, viewport: Rect, pages: usize) {
        if !self.search_state.open
            || !self.search_state.show_highlights
            || self.search_state.density.is_empty()
            || pages == 0
        {
            return;
        }
        let height = viewport.height().max(1.);
        let mut last = -4.;
        for (&p, &n) in &self.search_state.density {
            let y = height * (p as f32 + 0.5) / pages as f32;
            if y - last < 2. {
                continue;
            }
            let x = viewport.right() - 3.;
            painter.line_segment(
                [
                    Pos2::new(x - 2. - (n as f32).log2().min(5.), viewport.top() + y),
                    Pos2::new(x, viewport.top() + y),
                ],
                Stroke::new(1.4, MUTED_INK),
            );
            last = y;
        }
        if let Some(h) = self
            .search_state
            .selected_hit
            .and_then(|i| self.search_state.hits.get(i))
        {
            let y = viewport.top() + height * (h.page_index as f32 + 0.5) / pages as f32;
            painter.line_segment(
                [
                    Pos2::new(viewport.right() - 12., y),
                    Pos2::new(viewport.right() - 2., y),
                ],
                Stroke::new(3., INK),
            );
        }
    }
}

fn scroll_to_passage(ui: &egui::Ui, rect: Rect, reduce: bool) {
    let viewport = ui.clip_rect();
    let delta = passage_delta(viewport, rect);
    if delta == Vec2::ZERO {
        return;
    }
    let animation = motion::jump_animation(delta.y, viewport.height(), reduce);
    ui.scroll_with_delta_animation(delta, animation);
}
pub(super) fn passage_delta(view: Rect, rect: Rect) -> Vec2 {
    let y = if rect.top() >= view.top() && rect.bottom() <= view.bottom() {
        0.
    } else {
        view.top() + view.height() / 3. - rect.top()
    };
    let x = if rect.left() < view.left() {
        view.left() + 8. - rect.left()
    } else if rect.right() > view.right() {
        view.right() - 8. - rect.right()
    } else {
        0.
    };
    Vec2::new(x, y)
}
pub(super) fn paint_find_rect(
    painter: &egui::Painter,
    rect: Rect,
    active: bool,
    born: Option<Instant>,
    reduce: bool,
) {
    if !painter.clip_rect().intersects(rect) {
        return;
    }
    painter.rect_filled(
        rect,
        1.5,
        if active {
            Color32::from_rgba_unmultiplied(225, 169, 63, 110)
        } else {
            WASH
        },
    );
    if active {
        let t = if reduce {
            1.
        } else {
            born.map_or(1., |t| (t.elapsed().as_secs_f32() / 0.14).min(1.))
        };
        painter.line_segment(
            [
                rect.left_bottom(),
                Pos2::new(
                    rect.left() + rect.width() * (1. - (1. - t).powi(2)),
                    rect.bottom(),
                ),
            ],
            Stroke::new(2., Color32::from_rgb(121, 68, 23)),
        );
        if t < 1. {
            painter.ctx().request_repaint();
        }
    }
}

impl PdfEditorApp {
    pub(super) fn active_review_block(&self) -> Option<usize> {
        if !self.search_state.open {
            return None;
        }
        self.search_state
            .hits
            .get(self.search_state.selected_hit?)?
            .block_index
    }

    pub(super) fn draw_find_excerpt(&mut self, ui: &mut egui::Ui, block: usize) {
        if self.active_review_block() != Some(block) {
            return;
        }
        let hit = &self.search_state.hits[self.search_state.selected_hit.unwrap()];
        let label = ui.label(
            RichText::new(format!("Find · {}", hit.snippet))
                .color(self.liquid_ink_color())
                .underline(),
        );
        if self.search_state.landing {
            scroll_to_passage(ui, label.rect, self.settings.reduce_motion);
            self.search_state.landing = false;
            self.search_state.activated = Some(Instant::now());
        }
    }

    pub(super) fn draw_review_find_marks(
        &mut self,
        ui: &mut egui::Ui,
        block_index: usize,
        block: &LiquidBlock,
        start: usize,
        block_rect: Rect,
    ) {
        if !self.search_state.show_highlights {
            return;
        }
        let selected = self.active_review_block() == Some(block_index);
        // Use the galleys actually emitted by the existing selectable labels.
        // Never substitute a differently wrapped or differently sized layout.
        let labels = ui.ctx().graphics(|g| {
            g.get(ui.layer_id())
                .map(|p| {
                    p.all_entries()
                        .skip(start)
                        .filter_map(|shape| match &shape.shape {
                            egui::Shape::Text(t) => Some((t.pos, Arc::clone(&t.galley))),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        });
        let indices = &self.search_state.review_ranges[&block_index];
        let mut source_offset = 0;
        let mut active_rect = None;
        let mut visible_byte = None;
        for (pos, galley) in labels {
            let rendered = galley.text();
            if rendered.is_empty() {
                continue;
            }
            let Some(source_ranges) = review_paragraph::source_ranges(
                &block.text,
                source_offset,
                rendered,
                self.liquid_hide_footnotes,
            ) else {
                continue;
            };
            let Some(base) = source_ranges.first().map(|range| range.start) else {
                continue;
            };
            source_offset = source_ranges.last().map_or(base, |range| range.end);
            if self.search_state.review_anchor == Some(block_index) && visible_byte.is_none() {
                let mut character = 0;
                for row in &galley.rows {
                    if pos.y + row.rect().bottom() >= ui.clip_rect().top() {
                        visible_byte = source_ranges.get(character).map(|range| range.start);
                        break;
                    }
                    character += row.glyphs.len() + usize::from(row.ends_with_newline);
                }
                self.search_state.review_anchor_byte = visible_byte.unwrap_or(source_offset);
            }
            for &i in indices {
                let hit = &self.search_state.hits[i];
                if hit.match_end <= base || hit.match_start >= source_offset {
                    continue;
                }
                let active = self.search_state.selected_hit == Some(i);
                let mut character = 0;
                for row in &galley.rows {
                    let mut bounds = Rect::NOTHING;
                    for glyph in &row.glyphs {
                        if source_ranges.get(character).is_some_and(|range| {
                            range.end > hit.match_start && range.start < hit.match_end
                        }) {
                            bounds = bounds.union(
                                glyph
                                    .logical_rect()
                                    .translate(pos.to_vec2() + row.pos.to_vec2()),
                            );
                        }
                        character += 1;
                    }
                    if row.ends_with_newline {
                        character += 1;
                    }
                    if bounds.is_positive() {
                        if active && active_rect.is_none() {
                            active_rect = Some(bounds);
                        }
                        paint_find_rect(
                            ui.painter(),
                            bounds,
                            active,
                            self.search_state.activated,
                            self.settings.reduce_motion,
                        );
                    }
                }
            }
        }
        if selected && self.search_state.landing {
            if let Some(rect) = active_rect {
                scroll_to_passage(ui, rect, self.settings.reduce_motion);
                self.search_state.landing = false;
                self.search_state.activated = Some(Instant::now());
                ui.ctx().request_repaint();
            } else if !self.search_state.block_landed && !ui.clip_rect().intersects(block_rect) {
                ui.scroll_to_rect_animation(
                    block_rect,
                    Some(Align::TOP),
                    egui::style::ScrollAnimation::none(),
                );
                self.search_state.block_landed = true;
                ui.ctx().request_repaint();
            } else {
                self.search_state.block_landed = true;
                self.draw_find_excerpt(ui, block_index);
            }
        } else if selected && active_rect.is_none() && self.search_state.block_landed {
            self.draw_find_excerpt(ui, block_index);
        }
    }
}
