//! Layout of inline note markers inside one selectable paragraph.

use super::*;

#[derive(Debug, Clone)]
pub(super) struct MarkerRange {
    pub(super) start: usize,
    pub(super) end: usize,
    pub(super) number: u16,
}

const PARAGRAPH_CACHE_LIMIT: usize = 96 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct ParagraphKey {
    block: usize,
    offset: usize,
    len: usize,
    size: u32,
    color: [u8; 4],
    marker_color: [u8; 4],
    muted_color: [u8; 4],
    strong: bool,
    pixels_per_point: u32,
    font_hash: u64,
}

#[derive(Default)]
pub(super) struct ParagraphCache {
    document: Option<std::sync::Weak<LiquidDocument>>,
    width: Option<u32>,
    entries: HashMap<ParagraphKey, (Arc<egui::Galley>, Vec<MarkerRange>)>,
    estimated_bytes: usize,
}

impl ParagraphCache {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn get_or_layout(
        &mut self,
        document: &Arc<LiquidDocument>,
        block: usize,
        text: &str,
        size: f32,
        color: Color32,
        marker_color: Color32,
        muted_color: Color32,
        strong: bool,
        footnotes: &HashMap<u16, String>,
        ui: &mut egui::Ui,
    ) -> Option<(Arc<egui::Galley>, Vec<MarkerRange>)> {
        // The key only uses offsets into an immutable, live document. Refuse
        // to cache transient strings whose address could later be reused.
        let source = document.blocks.get(block)?.text.as_str();
        let offset = (text.as_ptr() as usize).checked_sub(source.as_ptr() as usize)?;
        if offset.checked_add(text.len())? > source.len() {
            return None;
        }
        let width = ui.available_width();
        if !width.is_finite() || width <= 0.0 {
            return None;
        }
        let same_document = self
            .document
            .as_ref()
            .and_then(std::sync::Weak::upgrade)
            .is_some_and(|cached| Arc::ptr_eq(&cached, document));
        if !same_document || self.width != Some(width.to_bits()) {
            self.entries.clear();
            self.estimated_bytes = 0;
            self.document = Some(Arc::downgrade(document));
            self.width = Some(width.to_bits());
        }
        let mut hasher = DefaultHasher::new();
        egui::FontSelection::Default
            .resolve(ui.style())
            .hash(&mut hasher);
        let key = ParagraphKey {
            block,
            offset,
            len: text.len(),
            size: size.to_bits(),
            color: color.to_array(),
            marker_color: marker_color.to_array(),
            muted_color: muted_color.to_array(),
            strong,
            pixels_per_point: ui.ctx().pixels_per_point().to_bits(),
            font_hash: hasher.finish(),
        };
        if let Some(cached) = self.entries.get(&key) {
            return Some(cached.clone());
        }
        let (job, markers) = layout_job(
            text,
            size,
            color,
            marker_color,
            muted_color,
            strong,
            footnotes,
            ui.style(),
            width,
        );
        let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
        let estimated = galley
            .num_vertices
            .saturating_mul(std::mem::size_of::<egui::epaint::Vertex>())
            .saturating_add(
                galley
                    .num_indices
                    .saturating_mul(std::mem::size_of::<u32>()),
            )
            .saturating_add(text.len().saturating_mul(48));
        if self.estimated_bytes.saturating_add(estimated) <= PARAGRAPH_CACHE_LIMIT {
            self.estimated_bytes += estimated;
            self.entries
                .insert(key, (Arc::clone(&galley), markers.clone()));
        }
        Some((galley, markers))
    }
}

/// Assemble one job instead of breaking each sentence around its callouts.
/// Marker positions are character offsets, as required by egui's cursors.
pub(super) fn layout_job(
    text: &str,
    size: f32,
    color: Color32,
    marker_color: Color32,
    muted_color: Color32,
    strong: bool,
    footnotes: &HashMap<u16, String>,
    style: &egui::Style,
    width: f32,
) -> (LayoutJob, Vec<MarkerRange>) {
    let mut job = LayoutJob::default();
    job.wrap.max_width = width;
    let mut markers = Vec::new();
    let mut visible_chars = 0;
    let mut is_marker = false;
    let mut run = String::new();

    let mut flush = |run: &mut String, is_marker: bool| {
        if run.is_empty() {
            return;
        }
        let (display, rich, number) = if is_marker {
            let number = run
                .trim()
                .parse::<u16>()
                .ok()
                .filter(|n| footnotes.contains_key(n));
            let display = review_footnote_superscript(run);
            let rich = RichText::new(&display)
                .size(size * 0.78)
                .color(if number.is_some() {
                    marker_color
                } else {
                    muted_color
                })
                .raised();
            (display, rich, number)
        } else {
            let display = std::mem::take(run);
            let mut rich = RichText::new(&display).size(size).color(color);
            if strong {
                rich = rich.strong();
            }
            (display, rich, None)
        };
        let end = visible_chars + display.chars().count();
        if let Some(number) = number {
            markers.push(MarkerRange {
                start: visible_chars,
                end,
                number,
            });
        }
        visible_chars = end;
        rich.append_to(&mut job, style, egui::FontSelection::Default, Align::BOTTOM);
        run.clear();
    };
    for ch in text.chars() {
        match ch {
            crate::layout_roles::CALLOUT_START => {
                flush(&mut run, is_marker);
                is_marker = true;
            }
            crate::layout_roles::CALLOUT_END => {
                flush(&mut run, is_marker);
                is_marker = false;
            }
            _ => run.push(ch),
        }
    }
    flush(&mut run, is_marker);
    (job, markers)
}

/// A marker can wrap between digits; return one hit rectangle per row.
pub(super) fn marker_rects(
    galley: &egui::Galley,
    origin: Pos2,
    ranges: &[MarkerRange],
) -> Vec<(Rect, u16)> {
    let mut hits: Vec<(Rect, u16)> = Vec::new();
    for marker in ranges {
        for index in marker.start..marker.end {
            let left = galley.pos_from_cursor(egui::text::CCursor {
                index,
                prefer_next_row: true,
            });
            let right = galley.pos_from_cursor(egui::text::CCursor {
                index: index + 1,
                prefer_next_row: false,
            });
            let rect = Rect::from_min_max(
                Pos2::new(left.left().min(right.left()), left.top()),
                Pos2::new(left.left().max(right.left()), left.bottom()),
            )
            .translate(origin.to_vec2());
            if let Some((last, number)) = hits.last_mut()
                && *number == marker.number
                && (last.top() - rect.top()).abs() < 0.5
                && (last.right() - rect.left()).abs() < 1.0
            {
                *last = last.union(rect);
            } else {
                hits.push((rect, marker.number));
            }
        }
    }
    hits
}

/// Locate rendered characters in the source block, retaining source byte
/// ranges when sentinel-wrapped note numbers become superscript glyphs.
pub(super) fn source_ranges(
    source: &str,
    source_offset: usize,
    rendered: &str,
    hide_markers: bool,
) -> Option<Vec<std::ops::Range<usize>>> {
    if rendered.is_empty() || !source.is_char_boundary(source_offset) {
        return None;
    }
    if let Some(relative) = source.get(source_offset..)?.find(rendered) {
        let start = source_offset + relative;
        return Some(
            rendered
                .char_indices()
                .map(|(i, c)| start + i..start + i + c.len_utf8())
                .collect(),
        );
    }

    let mut visible = String::new();
    let mut ranges = Vec::new();
    let mut marker: Option<Vec<(usize, char)>> = None;
    let append =
        |ch: char, at: usize, visible: &mut String, ranges: &mut Vec<std::ops::Range<usize>>| {
            visible.push(ch);
            ranges.push(at..at + ch.len_utf8());
        };
    let append_marker = |items: &[(usize, char)],
                         visible: &mut String,
                         ranges: &mut Vec<std::ops::Range<usize>>| {
        if hide_markers {
            return;
        }
        let first = items
            .iter()
            .position(|(_, c)| !c.is_whitespace())
            .unwrap_or(items.len());
        let last = items
            .iter()
            .rposition(|(_, c)| !c.is_whitespace())
            .map_or(first, |i| i + 1);
        for &(at, ch) in &items[first..last] {
            let display = if ch.is_whitespace() {
                ch.to_string()
            } else {
                review_footnote_superscript(&ch.to_string())
            };
            for shown in display.chars() {
                visible.push(shown);
                ranges.push(at..at + ch.len_utf8());
            }
        }
    };
    for (at, ch) in source.char_indices() {
        match ch {
            crate::layout_roles::CALLOUT_START => {
                if let Some(items) = marker.take() {
                    append_marker(&items, &mut visible, &mut ranges);
                }
                marker = Some(Vec::new());
            }
            crate::layout_roles::CALLOUT_END => {
                if let Some(items) = marker.take() {
                    append_marker(&items, &mut visible, &mut ranges);
                }
            }
            _ => {
                if let Some(items) = marker.as_mut() {
                    items.push((at, ch));
                } else {
                    append(ch, at, &mut visible, &mut ranges);
                }
            }
        }
    }
    if let Some(items) = marker {
        append_marker(&items, &mut visible, &mut ranges);
    }
    let start_char = ranges.partition_point(|range| range.start < source_offset);
    let start_byte = visible
        .char_indices()
        .nth(start_char)
        .map_or(visible.len(), |(at, _)| at);
    let relative = visible.get(start_byte..)?.find(rendered)?;
    let match_byte = start_byte + relative;
    let match_char = visible[..match_byte].chars().count();
    let count = rendered.chars().count();
    Some(ranges.get(match_char..match_char + count)?.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paragraph_job_keeps_text_together_and_marker_offsets_use_characters() {
        let ctx = Context::default();
        let text = format!(
            "Café {}12{} conclusion.",
            crate::layout_roles::CALLOUT_START,
            crate::layout_roles::CALLOUT_END
        );
        let mut output = None;
        let _ = ctx.run(Default::default(), |ctx| {
            let (job, markers) = layout_job(
                &text,
                17.0,
                Color32::BLACK,
                Color32::RED,
                Color32::GRAY,
                false,
                &HashMap::from([(12, "note".to_owned())]),
                &ctx.style(),
                120.0,
            );
            assert_eq!(job.text, "Café ¹² conclusion.");
            assert_eq!((markers[0].start, markers[0].end), (5, 7));
            let galley = ctx.fonts_mut(|fonts| fonts.layout_job(job));
            let hits = marker_rects(&galley, Pos2::new(10.0, 20.0), &markers);
            assert!(!hits.is_empty());
            assert!(
                hits.iter()
                    .all(|(rect, number)| rect.is_positive() && *number == 12)
            );
            output = Some(galley.text().to_owned());
        });
        assert_eq!(output.as_deref(), Some("Café ¹² conclusion."));
    }

    #[test]
    fn search_maps_superscript_glyphs_back_to_original_source_bytes() {
        let source = format!(
            "Café {}12{} conclusion.",
            crate::layout_roles::CALLOUT_START,
            crate::layout_roles::CALLOUT_END
        );
        let ranges = source_ranges(&source, 0, "Café ¹² conclusion.", false).unwrap();
        assert_eq!(&source[ranges[3].clone()], "é");
        assert_eq!(&source[ranges[5].clone()], "1");
        assert_eq!(&source[ranges[6].clone()], "2");
        assert_eq!(&source[ranges[8].clone()], "c");
        let hidden = source_ranges(&source, 0, "Café  conclusion.", true).unwrap();
        assert_eq!(&source[hidden[6].clone()], "c");
    }

    #[test]
    fn retained_paragraph_is_reused_and_tracks_width_and_document_changes() {
        let make_document = |text: String| {
            Arc::new(LiquidDocument {
                title: String::new(),
                blocks: vec![LiquidBlock {
                    role: LiquidBlockRole::Paragraph,
                    text,
                    label: None,
                }],
                article_spans: Vec::new(),
                block_source_lines: Vec::new(),
                footnote_links: Vec::new(),
                footnote_link_integrity: None,
                profile: None,
                noise_lines_removed: 0,
                llm_used: false,
                llm_provider: None,
                deep_liquid_used: false,
                deep_liquid_model: None,
                warnings: Vec::new(),
                source_signature: String::new(),
            })
        };
        let first = make_document(format!(
            "body {}1{} end",
            crate::layout_roles::CALLOUT_START,
            crate::layout_roles::CALLOUT_END
        ));
        let second = make_document(format!(
            "different {}1{} end",
            crate::layout_roles::CALLOUT_START,
            crate::layout_roles::CALLOUT_END
        ));
        let ctx = Context::default();
        let mut cache = ParagraphCache::default();
        let mut layouts = Vec::new();
        for (document, width) in [
            (&first, 200.0),
            (&first, 200.0),
            (&first, 300.0),
            (&second, 300.0),
        ] {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(width, 400.0))),
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let (galley, _) = cache
                        .get_or_layout(
                            document,
                            0,
                            &document.blocks[0].text,
                            17.0,
                            Color32::BLACK,
                            Color32::RED,
                            Color32::GRAY,
                            false,
                            &HashMap::from([(1, "note".to_owned())]),
                            ui,
                        )
                        .unwrap();
                    layouts.push(galley);
                });
            });
        }
        assert!(Arc::ptr_eq(&layouts[0], &layouts[1]));
        assert!(!Arc::ptr_eq(&layouts[1], &layouts[2]));
        assert_eq!(layouts[3].text(), "different ¹ end");
    }

    #[test]
    fn clicking_a_single_galley_marker_opens_its_footnote() {
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        let text = format!(
            "A sentence{}1{} continues.",
            crate::layout_roles::CALLOUT_START,
            crate::layout_roles::CALLOUT_END
        );
        let document: LiquidDocument = serde_json::from_value(serde_json::json!({
            "title":"Reader", "blocks":[{"role":"paragraph","text":text}],
            "llm_used":false,"warnings":[],"source_signature":"marker-click"
        }))
        .unwrap();
        let document = Arc::new(document);
        app.view_mode = DocumentViewMode::LiquidMode2;
        app.liquid_mode2_state = LiquidState::Ready(Arc::clone(&document));
        app.liquid_footnote_index
            .insert(1, "Footnote body".to_owned());
        let id = liquid_footnote_popup_id(
            &liquid_feedback_id(&document.source_signature, 0, &document.blocks[0]),
            1,
        );
        let mut draw = |time, events| {
            let mut hits = Vec::new();
            let input = egui::RawInput {
                time: Some(time),
                events,
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(600.0, 400.0))),
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    hits = app.draw_liquid_feedback_block(ui, &document, 0, &document.blocks[0]);
                });
            });
            hits
        };
        let point = draw(0.0, vec![])[0].0.center();
        draw(
            0.1,
            vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        draw(
            0.2,
            vec![egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(egui::Popup::is_id_open(&ctx, id));
    }
}
