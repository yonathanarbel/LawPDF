//! A page-ordered index of editable and read-only PDF annotations.
use super::*;

struct AnnotationRow {
    index: usize,
    foreign: bool,
    page: usize,
    top: f32,
    label: String,
    color: Color32,
}

pub(super) struct AnnotationListCache {
    epoch: u64,
    generation: u64,
    annotation_count: usize,
    foreign_marks: Option<Arc<Vec<ForeignAnnotation>>>,
    data: Arc<AnnotationListData>,
}

#[derive(Clone, Copy)]
enum ListItem {
    Page(usize),
    Annotation(usize),
}

struct AnnotationListData {
    rows: Vec<AnnotationRow>,
    items: Vec<ListItem>,
}

fn short_text(text: &str) -> String {
    short_chars(text.chars())
}

fn short_chars(chars: impl Iterator<Item = char>) -> String {
    let mut result = String::new();
    let mut count = 0;
    let mut pending_space = false;
    let mut truncated = false;
    for ch in chars {
        if ch.is_whitespace() {
            pending_space = !result.is_empty();
            continue;
        }
        if pending_space {
            if count == 96 {
                truncated = true;
                break;
            }
            result.push(' ');
            count += 1;
            pending_space = false;
        }
        if count == 96 {
            truncated = true;
            break;
        }
        result.push(ch);
        count += 1;
    }
    if truncated {
        if result.ends_with(' ') {
            result.pop();
        }
        result.push('…');
    }
    result
}

fn marker_excerpt(annotation: &EditorAnnotation, document: Option<&LoadedDocument>) -> String {
    let Some(chars) = document
        .and_then(|document| document.text_chars.get(annotation.page_index))
        .and_then(Option::as_deref)
    else {
        return String::new();
    };
    let bounds = expand_pdf_rect(annotation.rect, 2.0);
    let mut indices = chars.iter().enumerate().filter_map(|(index, character)| {
        let rect = character.rect?;
        let center = (
            (rect.left + rect.right) * 0.5,
            (rect.bottom + rect.top) * 0.5,
        );
        (!character.ch.is_control() && point_in_pdf_rect(center, bounds)).then_some(index)
    });
    let Some(first) = indices.next() else {
        return String::new();
    };
    let last = indices.last().unwrap_or(first);
    short_chars(chars[first..=last].iter().map(|character| character.ch))
}

fn annotation_rows(
    annotations: &[EditorAnnotation],
    document: Option<&LoadedDocument>,
) -> Vec<AnnotationRow> {
    let mut rows = annotations
        .iter()
        .enumerate()
        .map(|(index, annotation)| {
            let (kind, text, color) = match &annotation.kind {
                AnnotationKind::Marker {
                    style, color_rgb, ..
                } => (
                    match style {
                        MarkerStyle::Highlight => "Highlight",
                        MarkerStyle::Underline => "Underline",
                    },
                    marker_excerpt(annotation, document),
                    color_from_rgb(*color_rgb, 255),
                ),
                AnnotationKind::TextBox {
                    text, color_rgb, ..
                } => (
                    "Text box",
                    short_text(text),
                    color_from_rgb(*color_rgb, 255),
                ),
                AnnotationKind::Comment {
                    text, color_rgb, ..
                } => ("Comment", short_text(text), color_from_rgb(*color_rgb, 255)),
                AnnotationKind::Signature { signer, .. } => ("Signature", short_text(signer), INK),
            };
            let label = if text.is_empty() {
                kind.to_owned()
            } else {
                format!("{kind} · {text}")
            };
            AnnotationRow {
                index,
                foreign: false,
                page: annotation.page_index,
                top: annotation.rect.top,
                label,
                color,
            }
        })
        .collect::<Vec<_>>();
    rows.sort_by(|a, b| {
        a.page
            .cmp(&b.page)
            .then_with(|| b.top.total_cmp(&a.top))
            .then_with(|| a.index.cmp(&b.index))
    });
    rows
}

impl PdfEditorApp {
    fn annotation_list_rows(&mut self) -> Arc<AnnotationListData> {
        let foreign = self.foreign_annotations();
        if let Some(cache) = &self.annotation_list_cache
            && cache.epoch == self.document_epoch
            && cache.generation == self.annotation_save_generation
            && cache.annotation_count == self.annotations.len()
            && match (&cache.foreign_marks, &foreign) {
                (Some(left), Some(right)) => Arc::ptr_eq(left, right),
                (None, None) => true,
                _ => false,
            }
        {
            return Arc::clone(&cache.data);
        }
        let mut rows = annotation_rows(&self.annotations, self.document.as_ref());
        if let Some(marks) = &foreign {
            for (index, mark) in marks.iter().enumerate() {
                let Some(top) = mark
                    .rects
                    .iter()
                    .map(|rect| rect.top)
                    .max_by(f32::total_cmp)
                else {
                    continue;
                };
                let kind = match mark.kind {
                    ForeignAnnotationKind::Highlight => "Highlight",
                    ForeignAnnotationKind::Underline => "Underline",
                    ForeignAnnotationKind::Squiggly => "Squiggly underline",
                    ForeignAnnotationKind::StrikeOut => "Strikeout",
                    ForeignAnnotationKind::Comment => "Comment",
                    ForeignAnnotationKind::TextBox => "Text box",
                };
                let text = short_text(&mark.text);
                let label = if text.is_empty() {
                    format!("{kind} · other app")
                } else {
                    format!("{kind} · other app · {text}")
                };
                rows.push(AnnotationRow {
                    index,
                    foreign: true,
                    page: mark.page_index,
                    top,
                    label,
                    color: color_from_rgb(mark.color_rgb, 255),
                });
            }
        }
        rows.sort_by(|a, b| {
            a.page
                .cmp(&b.page)
                .then_with(|| b.top.total_cmp(&a.top))
                .then_with(|| a.foreign.cmp(&b.foreign))
                .then_with(|| a.index.cmp(&b.index))
        });
        let mut items = Vec::with_capacity(rows.len());
        let mut page = None;
        for (index, row) in rows.iter().enumerate() {
            if page != Some(row.page) {
                page = Some(row.page);
                items.push(ListItem::Page(row.page));
            }
            items.push(ListItem::Annotation(index));
        }
        let data = Arc::new(AnnotationListData { rows, items });
        self.annotation_list_cache = Some(AnnotationListCache {
            epoch: self.document_epoch,
            generation: self.annotation_save_generation,
            annotation_count: self.annotations.len(),
            foreign_marks: foreign,
            data: Arc::clone(&data),
        });
        data
    }

    pub(super) fn draw_annotation_list(&mut self, ui: &mut egui::Ui) {
        self.ensure_foreign_annotations();
        ui.heading("Annotations");
        ui.label(
            RichText::new("Other apps' marks are read-only")
                .size(11.0)
                .color(self.app_palette().muted),
        );
        let data = self.annotation_list_rows();
        let foreign_ready = self.foreign_annotations().is_some();
        if self.foreign_annotations_loading() {
            ui.label("Loading other applications' marks…");
        }
        if self.foreign_annotations_failed() {
            ui.label("Other applications' marks could not be loaded.");
            if ui.button("Retry loading marks").clicked() {
                self.retry_foreign_annotations();
            }
        }
        if data.rows.is_empty() && foreign_ready {
            ui.label(RichText::new("No annotations yet.").color(self.app_palette().muted));
            return;
        }
        if self.pending_annotation_digest.is_some() {
            ui.label("Preparing annotation text…");
            if ui.button("Cancel digest preparation").clicked() {
                self.pending_annotation_digest = None;
                self.status = "Annotation digest preparation canceled.".to_owned();
            }
        } else if ui
            .add_enabled(
                foreign_ready && !data.rows.is_empty(),
                egui::Button::new("Export annotation digest…"),
            )
            .clicked()
        {
            self.start_annotation_digest(ui.ctx());
        }
        if self.annotations.iter().any(|annotation| matches!(&annotation.kind,
            AnnotationKind::Comment { author_id, .. } if !author_id.is_empty() && *author_id == self.settings.comment_author_id))
            && ui.link("Edit my comment name").clicked() {
                self.open_comment_profile();
            }
        let mut clicked = None;
        let draw_item =
            |ui: &mut egui::Ui, item: ListItem, clicked: &mut Option<(bool, usize)>| match item {
                ListItem::Page(page) => {
                    ui.label(RichText::new(format!("Page {}", page + 1)).strong());
                }
                ListItem::Annotation(index) => {
                    let row = &data.rows[index];
                    ui.horizontal(|ui| {
                        let (swatch, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
                        ui.painter().rect_filled(swatch, 2, row.color);
                        ui.painter().rect_stroke(swatch, 2, Stroke::new(1.0_f32, theme::for_ui(ui).muted), egui::StrokeKind::Inside);
                        if ui
                            .add(egui::Button::new(&row.label).frame(false).truncate())
                            .on_hover_text(&row.label)
                            .on_hover_cursor(CursorIcon::PointingHand)
                            .clicked()
                        {
                            *clicked = Some((row.foreign, row.index));
                        }
                    });
                }
            };
        let accesskit_active = ui.ctx().accesskit_node_builder(ui.id(), |_| ()).is_some();
        let scroll = egui::ScrollArea::vertical()
            .id_salt(("annotation-list", self.document_epoch))
            .max_height(360.0);
        if accesskit_active || ui.ctx().memory(|memory| memory.options.screen_reader) {
            scroll.show(ui, |ui| {
                for item in data.items.iter().copied() {
                    draw_item(ui, item, &mut clicked);
                }
            });
        } else {
            let row_height = ui.spacing().interact_size.y.max(20.0);
            scroll.show_rows(ui, row_height, data.items.len(), |ui, range| {
                for index in range {
                    draw_item(ui, data.items[index], &mut clicked);
                }
            });
        }
        if let Some((foreign, index)) = clicked {
            if foreign {
                self.jump_to_foreign_annotation(index, ui.ctx());
            } else {
                self.jump_to_annotation(index, ui.ctx());
            }
        }
    }

    fn jump_to_foreign_annotation(&mut self, index: usize, ctx: &Context) {
        let Some(mark) = self
            .foreign_annotations()
            .and_then(|marks| marks.get(index).cloned())
        else {
            return;
        };
        let Some(page_height) = self
            .document
            .as_ref()
            .and_then(|document| document.pages.get(mark.page_index))
            .map(|page| page.height)
        else {
            return;
        };
        let Some(top) = mark
            .rects
            .iter()
            .map(|rect| rect.top)
            .max_by(f32::total_cmp)
        else {
            return;
        };
        self.record_jump_origin();
        self.set_view_mode(DocumentViewMode::Pdf, ctx);
        self.go_to_page(mark.page_index);
        if page_height.is_finite() && page_height > 0.0 && top.is_finite() {
            self.set_pdf_destination_fraction(
                self.page_index,
                Some((1.0 - top / page_height).clamp(0.0, 1.0)),
            );
        }
        self.status = "Other application's annotation (read-only).".to_owned();
        ctx.request_repaint();
    }

    fn jump_to_annotation(&mut self, index: usize, ctx: &Context) {
        let Some(annotation) = self.annotations.get(index).cloned() else {
            return;
        };
        let Some(page_height) = self
            .document
            .as_ref()
            .and_then(|document| document.pages.get(annotation.page_index))
            .map(|page| page.height)
        else {
            return;
        };
        let top = match &annotation.kind {
            AnnotationKind::Comment { anchor, .. } => anchor.1,
            _ => annotation.rect.top,
        };
        self.record_jump_origin();
        self.set_view_mode(DocumentViewMode::Pdf, ctx);
        self.go_to_page(annotation.page_index);
        if page_height.is_finite() && page_height > 0.0 && top.is_finite() {
            self.set_pdf_destination_fraction(
                self.page_index,
                Some((1.0 - top / page_height).clamp(0.0, 1.0)),
            );
        }
        match annotation.kind {
            AnnotationKind::TextBox { .. } => self.select_text_box(index),
            AnnotationKind::Comment { .. } => self.start_comment_edit(index),
            _ => {}
        }
        ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::PageInfo;

    fn marker(page: usize, top: f32, style: MarkerStyle) -> EditorAnnotation {
        EditorAnnotation {
            page_index: page,
            rect: PdfRect::new(50.0, top - 12.0, 150.0, top),
            kind: AnnotationKind::Marker {
                color_rgb: [1.0, 0.9, 0.2],
                opacity: 0.4,
                style,
            },
        }
    }

    #[test]
    fn annotations_are_grouped_by_page_and_sorted_in_reading_order() {
        let annotations = vec![
            marker(2, 100.0, MarkerStyle::Underline),
            marker(0, 300.0, MarkerStyle::Highlight),
            marker(0, 600.0, MarkerStyle::Underline),
        ];
        let rows = annotation_rows(&annotations, None);
        assert_eq!(
            rows.iter().map(|row| row.index).collect::<Vec<_>>(),
            vec![2, 1, 0]
        );
        assert_eq!(
            rows.iter()
                .map(|row| row.label.as_str())
                .collect::<Vec<_>>(),
            vec!["Underline", "Highlight", "Underline"]
        );
    }

    #[test]
    fn highlight_preview_uses_only_characters_inside_the_marker() {
        let (mut app, _ctx, _worker) = annotation_saving::tests::fixture();
        let chars = "Read me outside"
            .chars()
            .enumerate()
            .map(|(index, ch)| PageTextChar {
                ch,
                rect: Some(PdfRect::new(
                    50.0 + index as f32 * 6.0,
                    688.0,
                    55.0 + index as f32 * 6.0,
                    700.0,
                )),
                font_size: Some(12.0),
                bold: false,
                italic: false,
            })
            .collect();
        app.document.as_mut().unwrap().text_chars[0] = Some(chars);
        let mut highlight = marker(0, 700.0, MarkerStyle::Highlight);
        highlight.rect.right = 90.0;
        assert_eq!(marker_excerpt(&highlight, app.document.as_ref()), "Read me");
    }

    #[test]
    fn list_cache_refreshes_after_annotation_changes() {
        let (mut app, _ctx, _worker) = annotation_saving::tests::fixture();
        app.annotations
            .push(marker(0, 700.0, MarkerStyle::Highlight));
        let first = app.annotation_list_rows();
        assert!(Arc::ptr_eq(&first, &app.annotation_list_rows()));
        app.annotations[0].kind = AnnotationKind::Marker {
            color_rgb: [1.0, 0.9, 0.2],
            opacity: 0.4,
            style: MarkerStyle::Underline,
        };
        app.annotation_save_generation += 1;
        let updated = app.annotation_list_rows();
        assert!(!Arc::ptr_eq(&first, &updated));
        assert_eq!(updated.rows[0].label, "Underline");
    }

    #[test]
    fn foreign_marks_are_listed_and_navigate_without_becoming_editable() {
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        let document = app.document.as_mut().unwrap();
        document.page_count = 2;
        document.pages = vec![PageInfo::new(612.0, 792.0); 2];
        document.text_chars = vec![None; 2];
        app.set_foreign_annotations_for_test(vec![ForeignAnnotation {
            page_index: 1,
            rects: vec![PdfRect::new(50.0, 580.0, 100.0, 600.0)],
            precise_text_geometry: false,
            kind: ForeignAnnotationKind::Comment,
            text: "External note".into(),
            author: "Reader".into(),
            color_rgb: [1.0, 1.0, 0.0],
            key: "foreign".into(),
        }]);
        let rows = app.annotation_list_rows();
        assert_eq!(rows.rows[0].label, "Comment · other app · External note");
        assert!(rows.rows[0].foreign);
        let _ = ctx.run(Default::default(), |ctx| {
            app.jump_to_foreign_annotation(0, ctx)
        });
        assert_eq!(app.page_index, 1);
        assert!(app.annotations.is_empty());
        assert!(app.editing_comment.is_none());
        assert!(app.status.contains("read-only"));
    }

    #[test]
    fn list_preview_refreshes_when_page_text_arrives() {
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        let path = app.document.as_ref().unwrap().path.clone();
        app.annotations
            .push(marker(0, 700.0, MarkerStyle::Highlight));
        assert_eq!(app.annotation_list_rows().rows[0].label, "Highlight");
        let chars = "Word"
            .chars()
            .enumerate()
            .map(|(index, ch)| PageTextChar {
                ch,
                rect: Some(PdfRect::new(
                    50.0 + index as f32 * 6.0,
                    688.0,
                    55.0 + index as f32 * 6.0,
                    700.0,
                )),
                font_size: Some(12.0),
                bold: false,
                italic: false,
            })
            .collect();
        let (sender, receiver) = crossbeam_channel::unbounded();
        app.render_rx = receiver;
        sender
            .send(RenderEvent::TextChars {
                document_epoch: 7,
                path,
                page_index: 0,
                result: Ok(chars),
            })
            .unwrap();
        let _ = ctx.run(Default::default(), |ctx| app.poll_render_results(ctx));
        assert_eq!(app.annotation_list_rows().rows[0].label, "Highlight · Word");
    }

    #[test]
    fn clicking_an_annotation_jumps_to_its_location_and_records_history() {
        fn text_center(shape: &egui::Shape, text: &str) -> Option<Pos2> {
            match shape {
                egui::Shape::Text(shape) if shape.galley.text() == text => {
                    Some(shape.pos + shape.galley.size() * 0.5)
                }
                egui::Shape::Vec(shapes) => {
                    shapes.iter().find_map(|shape| text_center(shape, text))
                }
                _ => None,
            }
        }
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        let document = app.document.as_mut().unwrap();
        document.page_count = 3;
        document.pages = vec![PageInfo::new(612.0, 792.0); 3];
        document.text_chars = vec![None; 3];
        app.annotations = vec![marker(2, 600.0, MarkerStyle::Underline)];
        app.scroll_target_page = None;
        let draw = |app: &mut PdfEditorApp, time, events| {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 500.0))),
                time: Some(time),
                events,
                ..Default::default()
            };
            ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| app.draw_annotation_list(ui));
            })
        };
        let output = draw(&mut app, 0.0, vec![]);
        let point = output
            .shapes
            .iter()
            .find_map(|shape| text_center(&shape.shape, "Underline"))
            .expect("annotation control is visible");
        draw(&mut app, 0.1, vec![egui::Event::PointerMoved(point)]);
        for (time, pressed) in [(0.2, true), (0.3, false)] {
            draw(
                &mut app,
                time,
                vec![egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
        }
        assert_eq!(app.page_index, 2);
        let (page, fraction) = app.pending_resume_fraction.unwrap();
        assert_eq!(page, 2);
        assert!((fraction - (1.0 - 600.0 / 792.0)).abs() < 0.001);
        let _ = ctx.run(Default::default(), |ctx| {
            app.navigate_jump_history(ctx, false)
        });
        assert_eq!(app.page_index, 0);
    }

    #[test]
    fn annotation_jump_positions_the_active_side_by_side_pane() {
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        let document = app.document.as_mut().unwrap();
        document.page_count = 4;
        document.pages = vec![PageInfo::new(612.0, 792.0); 4];
        document.text_chars = vec![None; 4];
        document.native_text = vec![String::new(); 4];
        document.native_text_loaded = vec![true; 4];
        document.links = vec![vec![]; 4];
        let document = document.clone();
        app.tabs[0] = app.active_tab_snapshot(document.clone());
        let mut other = app.active_tab_snapshot(document);
        other.document_epoch = 8;
        app.tabs.push(other);
        app.sbs_mode = Some(SbsModeState {
            left_epoch: 7,
            right_epoch: 8,
        });
        app.annotations = vec![marker(2, 600.0, MarkerStyle::Underline)];
        let draw = |app: &mut PdfEditorApp, time| {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 700.0))),
                time: Some(time),
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| app.draw_sbs_document(ui, ctx));
            });
        };
        draw(&mut app, 0.0);
        let _ = ctx.run(Default::default(), |ctx| app.jump_to_annotation(0, ctx));
        let expected = 1.0 - 600.0 / 792.0;
        assert_eq!(
            app.sbs_pane_states[&7].destination_fraction,
            Some((2, expected))
        );
        assert!(app.pending_resume_fraction.is_none());
        for step in 1..20 {
            draw(&mut app, step as f64 * 0.1);
        }
        let range = app
            .visible_page_ranges
            .iter()
            .find(|range| range.page_index == 2)
            .unwrap();
        assert!(
            (range.top_fraction - expected).abs() < 0.03,
            "landed at {}",
            range.top_fraction
        );
        let _ = ctx.run(Default::default(), |ctx| {
            app.navigate_jump_history(ctx, false)
        });
        assert_eq!(app.page_index, 0);
    }

    #[test]
    fn accessibility_tree_can_activate_an_offscreen_annotation() {
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        let document = app.document.as_mut().unwrap();
        document.page_count = 100;
        document.pages = vec![PageInfo::new(612.0, 792.0); 100];
        document.text_chars = vec![None; 100];
        app.annotations = (0..100)
            .map(|page| EditorAnnotation {
                page_index: page,
                rect: PdfRect::new(50.0, 500.0, 150.0, 520.0),
                kind: AnnotationKind::TextBox {
                    text: format!("Unique item {page}"),
                    font_size: 12.0,
                    color_rgb: [0.0, 0.0, 0.0],
                },
            })
            .collect();
        ctx.enable_accesskit();
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 500.0))),
            ..Default::default()
        };
        let output = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| app.draw_annotation_list(ui));
        });
        let tree = output.platform_output.accesskit_update.unwrap();
        let target = tree
            .nodes
            .iter()
            .find_map(|(id, node)| {
                (node.label() == Some("Text box · Unique item 99")).then_some(*id)
            })
            .expect("distant annotation control");
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 500.0))),
            events: vec![egui::Event::AccessKitActionRequest(
                egui::accesskit::ActionRequest {
                    action: egui::accesskit::Action::Click,
                    target,
                    data: None,
                },
            )],
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| app.draw_annotation_list(ui));
        });
        assert_eq!(app.page_index, 99);
        assert_eq!(app.selected_text_box, Some(99));
    }

    #[test]
    fn jumping_to_a_comment_opens_it_without_changing_its_text() {
        let (mut app, ctx, _worker) = annotation_saving::tests::fixture();
        let comment = EditorAnnotation {
            page_index: 0,
            rect: PdfRect::new(510.0, 500.0, 560.0, 540.0),
            kind: AnnotationKind::Comment {
                id: "comment-1".into(),
                text: "A note to keep".into(),
                author: "Reader".into(),
                author_id: "reader-1".into(),
                color_rgb: [1.0, 0.9, 0.2],
                created_at: String::new(),
                updated_at: String::new(),
                anchor: (100.0, 620.0),
            },
        };
        app.annotations.push(comment.clone());
        let _ = ctx.run(Default::default(), |ctx| app.jump_to_annotation(0, ctx));
        assert_eq!(app.editing_comment, Some(0));
        assert_eq!(app.annotations[0], comment);
        let (_, fraction) = app.pending_resume_fraction.unwrap();
        assert!((fraction - (1.0 - 620.0 / 792.0)).abs() < 0.001);
    }
}
