//! Lazy comment import and derived presentation. No PDF or geometry work on open.
use super::*;
use crate::comments::{ReviewMarginMode, author_colors};
use crate::document_store::FileRevision;
use crate::model::DocumentComment;
use crate::review_reading::{CommentThread, anchor_comments_to_blocks, group_comment_threads};

type PendingComment = (u64, usize, (f32, f32), f32, f32);
type CommentGeneration = (u64, usize, u64);

/// A comment must never be journaled with an identity that exists only in RAM.
/// Commit the profile first; failed writes leave both settings and PDFs alone.
fn persist_comment_profile(
    settings: &mut AppSettings,
    name: String,
    persist: impl FnOnce(&AppSettings) -> Result<(), String>,
) -> Result<(), String> {
    let mut candidate = settings.clone();
    candidate.comment_author_name = name;
    if candidate.comment_author_id.is_empty() {
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random)
            .map_err(|_| "Could not create a comment identity. Please try again.".to_owned())?;
        candidate.comment_author_id = random.iter().map(|byte| format!("{byte:02x}")).collect();
    }
    persist(&candidate)?;
    *settings = candidate;
    Ok(())
}

#[derive(Default)]
pub(super) struct CommentsUi {
    documents: HashMap<u64, CommentCache>,
    pub(super) pending_comment: Option<PendingComment>,
    profile_open: bool,
    name: String,
    error: Option<String>,
    derived_tx: Option<Sender<DerivedResult>>,
    derived_rx: Option<Receiver<DerivedResult>>,
}

#[derive(Default)]
struct CommentCache {
    revision: Option<FileRevision>,
    loading: bool,
    error: Option<String>,
    embedded: Option<Vec<DocumentComment>>,
    derived: Arc<ReviewComments>,
    generation: Option<CommentGeneration>,
    desired_generation: Option<CommentGeneration>,
    pending_generation: Option<(FileRevision, CommentGeneration)>,
    geometry: Option<Arc<CommentGeometry>>,
    geometry_serial: u64,
    geometry_ready_seen: HashSet<usize>,
}

struct CommentGeometry {
    /// Retain the Arc, not only its address: allocator address reuse cannot
    /// make a different Review document look like this geometry cache.
    document: Arc<LiquidDocument>,
    pages: HashSet<usize>,
    /// Pages attempted before background native character loading completed.
    unavailable_pages: HashSet<usize>,
    lines: Vec<crate::liquid::DeepLiquidSourceLine>,
    page_heights: Vec<f32>,
}

#[derive(Default)]
pub(super) struct ReviewComments {
    pub(super) threads: Vec<Arc<CommentThread>>,
    pub(super) anchors: HashMap<usize, Vec<usize>>,
    pub(super) colors: HashMap<String, [f32; 3]>,
    pub(super) unanchored: Vec<usize>,
}

struct DerivedResult {
    epoch: u64,
    revision: FileRevision,
    generation: CommentGeneration,
    presentation: ReviewComments,
    geometry: Arc<CommentGeometry>,
}

impl CommentCache {
    fn refresh_geometry_readiness(
        &mut self,
        document: &Arc<LiquidDocument>,
        chars: &[Option<Vec<PageTextChar>>],
    ) {
        let document_key = Arc::as_ptr(document) as usize;
        if self
            .desired_generation
            .is_some_and(|generation| generation.1 != document_key)
        {
            self.geometry_ready_seen.clear();
        }
        if let Some(geometry) = self
            .geometry
            .as_ref()
            .filter(|geometry| Arc::ptr_eq(&geometry.document, document))
        {
            let mut became_ready = false;
            for &page in &geometry.unavailable_pages {
                if chars.get(page).is_some_and(Option::is_some)
                    && self.geometry_ready_seen.insert(page)
                {
                    became_ready = true;
                }
            }
            if became_ready {
                self.geometry_serial = self.geometry_serial.wrapping_add(1);
            }
        }
    }

    fn accept_derived(&mut self, result: DerivedResult) {
        let same_pending =
            self.pending_generation
                .as_ref()
                .is_some_and(|(revision, generation)| {
                    *revision == result.revision && *generation == result.generation
                });
        if same_pending {
            self.pending_generation = None;
        }
        // Even an obsolete annotation edit can contribute reusable geometry
        // for the same Review document. Never downgrade page coverage.
        if self.desired_generation.is_some_and(|generation| {
            generation.1 == Arc::as_ptr(&result.geometry.document) as usize
        }) && self
            .geometry
            .as_ref()
            .is_none_or(|old| old.pages.is_subset(&result.geometry.pages))
        {
            self.geometry = Some(Arc::clone(&result.geometry));
        }
        if self.revision.as_ref() == Some(&result.revision)
            && self.desired_generation == Some(result.generation)
        {
            self.derived = Arc::new(result.presentation);
            self.generation = Some(result.generation);
        }
    }
}

impl PdfEditorApp {
    pub(super) fn comment_author_label_color(&self, color: Color32) -> Color32 {
        let background = match self.liquid_theme {
            LiquidTheme::Paper => [1.0, 0.992, 0.973],
            LiquidTheme::Sepia => [0.98, 0.949, 0.878],
            LiquidTheme::Dark => [0.133, 0.157, 0.204],
        };
        let color = crate::comments::author_text_color_on(
            [
                color.r() as f32 / 255.0,
                color.g() as f32 / 255.0,
                color.b() as f32 / 255.0,
            ],
            background,
        );
        color_from_rgb(color, 255)
    }

    pub(super) fn draw_review_feedback_switch(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.add_space(26.0);
            let before = self.settings.review_margin_mode;
            ui.selectable_value(
                &mut self.settings.review_margin_mode,
                ReviewMarginMode::Footnotes,
                "Show footnotes",
            );
            ui.selectable_value(
                &mut self.settings.review_margin_mode,
                ReviewMarginMode::Comments,
                "Show comments",
            );
            if before != self.settings.review_margin_mode {
                self.liquid_hide_footnotes = false;
                self.chrome.expanded_note = None;
                self.queue_settings_save();
            }
        });
        ui.add_space(5.0);
    }
    pub(super) fn review_shows_comments(&self) -> bool {
        self.settings.review_margin_mode == ReviewMarginMode::Comments
    }

    pub(super) fn open_comment_profile(&mut self) {
        self.comments_ui.name = self.settings.comment_author_name.clone();
        self.comments_ui.error = None;
        self.comments_ui.profile_open = true;
    }

    pub(super) fn ensure_comment_author_id(&mut self) -> Result<(), String> {
        if self.settings.comment_author_id.is_empty() {
            let name = self.settings.comment_author_name.clone();
            persist_comment_profile(&mut self.settings, name, save_settings)?;
        }
        Ok(())
    }

    pub(super) fn comment_display_author(&self, author: &str, author_id: &str) -> String {
        if !author_id.is_empty() && author_id == self.settings.comment_author_id {
            self.settings.comment_author_name.clone()
        } else if author.trim().is_empty() {
            "Unknown author".to_owned()
        } else {
            author.to_owned()
        }
    }

    pub(super) fn draw_comment_profile(&mut self, ctx: &Context) {
        if !self.comments_ui.profile_open {
            return;
        }
        let mut open = true;
        let mut save = false;
        let mut cancel = false;
        egui::Window::new("Comment name")
            .id(egui::Id::new("comment-profile"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.set_width(320.0);
                ui.label("The name shown on your comments in every document.");
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.comments_ui.name)
                        .hint_text("Your name")
                        .char_limit(120),
                );
                if let Some(error) = &self.comments_ui.error {
                    ui.colored_label(Color32::DARK_RED, error);
                }
                ui.horizontal(|ui| {
                    save = ui.button("Save name").clicked()
                        || (response.lost_focus()
                            && ui.input(|input| input.key_pressed(egui::Key::Enter)));
                    cancel = ui.button("Cancel").clicked();
                });
            });
        if save {
            let name = self.comments_ui.name.trim().to_owned();
            if name.is_empty() || name.chars().any(char::is_control) {
                self.comments_ui.error =
                    Some("Enter a name without control characters.".to_owned());
                return;
            }
            if let Err(error) =
                persist_comment_profile(&mut self.settings, name.clone(), save_settings)
            {
                self.comments_ui.error = Some(error);
                return;
            }
            // Display names in unopened documents use this same stable identity.
            // Only the active document needs saving now; other authors are untouched.
            let mut changed = false;
            for annotation in &mut self.annotations {
                if let AnnotationKind::Comment {
                    author, author_id, ..
                } = &mut annotation.kind
                    && !author_id.is_empty()
                    && *author_id == self.settings.comment_author_id
                    && *author != name
                {
                    *author = name.clone();
                    changed = true;
                }
            }
            if changed {
                self.schedule_annotation_autosave(ctx);
            }
            self.comments_ui.profile_open = false;
            if let Some((epoch, page, anchor, width, height)) =
                self.comments_ui.pending_comment.take()
                && epoch == self.document_epoch
            {
                self.add_comment_annotation(ctx, page, anchor, width, height);
            }
        } else if !open || cancel {
            self.comments_ui.profile_open = false;
            self.comments_ui.pending_comment = None;
        }
    }

    pub(super) fn accept_document_comments(
        &mut self,
        epoch: u64,
        path: PathBuf,
        revision: FileRevision,
        result: Result<Vec<DocumentComment>, String>,
    ) {
        // Revision and epoch both matter: an old parse must not replace a rotated
        // page, a newer save, a reused path, or another tab's comments.
        let Some(session) = self.annotation_sessions.get(&epoch) else {
            return;
        };
        let Some(cache) = self.comments_ui.documents.get_mut(&epoch) else {
            return;
        };
        if cache.revision.as_ref() != Some(&revision) {
            return;
        }
        cache.loading = false;
        if session.revision != revision {
            cache.revision = None;
            return;
        }
        let correct_path = self
            .document
            .as_ref()
            .is_some_and(|doc| self.document_epoch == epoch && doc.path == path)
            || self
                .tabs
                .iter()
                .any(|tab| tab.document_epoch == epoch && tab.document.path == path);
        if !correct_path {
            return;
        }
        match result {
            Ok(comments) => {
                cache.embedded = Some(comments);
                cache.error = None;
                cache.generation = None;
            }
            Err(error) => {
                cache.error = Some(error);
            }
        }
    }

    pub(super) fn poll_comment_cache(&mut self, _ctx: &Context) {
        if let Some(receiver) = &self.comments_ui.derived_rx {
            while let Ok(result) = receiver.try_recv() {
                if let Some(cache) = self.comments_ui.documents.get_mut(&result.epoch) {
                    cache.accept_derived(result);
                }
            }
        }
        self.comments_ui.documents.retain(|epoch, _| {
            *epoch == self.document_epoch
                || self.tabs.iter().any(|tab| tab.document_epoch == *epoch)
        });
    }

    pub(super) fn review_comments(
        &mut self,
        document: &Arc<LiquidDocument>,
        ctx: &Context,
    ) -> Arc<ReviewComments> {
        if !self.review_shows_comments() {
            return Arc::default();
        }
        let epoch = self.document_epoch;
        let Some(loaded) = &self.document else {
            return Arc::default();
        };
        let Some(session) = self.annotation_sessions.get(&epoch) else {
            return Arc::default();
        };
        let revision = session.revision.clone();
        let cache = self.comments_ui.documents.entry(epoch).or_default();
        if cache.revision.as_ref() != Some(&revision) {
            cache.revision = Some(revision.clone());
            cache.loading = true;
            cache.error = None;
            cache.embedded = None;
            cache.generation = None;
            cache.pending_generation = None;
            cache.derived = Arc::default();
            if let Err(error) = self.render_tx.send(RenderRequest::DocumentComments {
                document_epoch: epoch,
                path: loaded.path.clone(),
                expected_revision: revision.clone(),
            }) {
                cache.loading = false;
                cache.error = Some(error.to_string());
            }
        }
        let document_key = Arc::as_ptr(document) as usize;
        cache.refresh_geometry_readiness(document, &loaded.text_chars);
        let generation = (
            self.annotation_save_generation,
            document_key,
            cache.geometry_serial,
        );
        if cache.desired_generation != Some(generation) {
            cache.desired_generation = Some(generation);
            cache.derived = Arc::default();
            cache.generation = None;
        }
        if cache.embedded.is_some()
            && cache.generation != Some(generation)
            && cache.pending_generation.is_none()
        {
            if self.comments_ui.derived_tx.is_none() {
                let (tx, rx) = unbounded();
                self.comments_ui.derived_tx = Some(tx);
                self.comments_ui.derived_rx = Some(rx);
            }
            let tx = self.comments_ui.derived_tx.as_ref().unwrap().clone();
            let mut comments = cache.embedded.clone().unwrap_or_default();
            comments.extend(
                self.annotations
                    .iter()
                    .filter_map(DocumentComment::from_editor_annotation),
            );
            let needed_pages = comments
                .iter()
                .map(|comment| comment.page_index)
                .collect::<HashSet<_>>();
            let previous_geometry = cache
                .geometry
                .as_ref()
                .filter(|geometry| Arc::ptr_eq(&geometry.document, document))
                .cloned();
            let missing_pages = needed_pages
                .iter()
                .copied()
                .filter(|page| {
                    previous_geometry.as_ref().is_none_or(|geometry| {
                        !geometry.pages.contains(page)
                            && (!geometry.unavailable_pages.contains(page)
                                || loaded.text_chars.get(*page).is_some_and(Option::is_some))
                    })
                })
                .collect::<HashSet<_>>();
            // Copy character boxes only once for pages that actually contain a
            // comment. Subsequent text/name edits reuse their geometry.
            let geometry_input = if missing_pages.is_empty() {
                None
            } else {
                Some((
                    loaded.pages.clone(),
                    loaded
                        .text_chars
                        .iter()
                        .enumerate()
                        .map(|(page, chars)| {
                            if missing_pages.contains(&page) {
                                chars.clone()
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>(),
                ))
            };
            let document = Arc::clone(document);
            let ctx = ctx.clone();
            cache.pending_generation = Some((revision.clone(), generation));
            std::thread::spawn(move || {
                let colors = author_colors(&comments);
                let threads = group_comment_threads(&comments)
                    .into_iter()
                    .map(Arc::new)
                    .collect::<Vec<_>>();
                let roots = threads
                    .iter()
                    .map(|thread| thread.root.clone())
                    .collect::<Vec<_>>();
                let geometry = if let Some((pages, chars)) = geometry_input {
                    let page_heights = pages.iter().map(|page| page.height).collect();
                    let mut lines = previous_geometry
                        .as_ref()
                        .map(|geometry| geometry.lines.clone())
                        .unwrap_or_default();
                    lines.extend(layout_roles::deep_source_lines_for_pages(&pages, &chars));
                    let mut pages = previous_geometry
                        .as_ref()
                        .map(|geometry| geometry.pages.clone())
                        .unwrap_or_default();
                    let mut unavailable_pages = previous_geometry
                        .as_ref()
                        .map(|geometry| geometry.unavailable_pages.clone())
                        .unwrap_or_default();
                    for page in missing_pages {
                        if chars.get(page).is_some_and(Option::is_some) {
                            pages.insert(page);
                            unavailable_pages.remove(&page);
                        } else {
                            unavailable_pages.insert(page);
                        }
                    }
                    Arc::new(CommentGeometry {
                        document: Arc::clone(&document),
                        pages,
                        unavailable_pages,
                        lines,
                        page_heights,
                    })
                } else {
                    previous_geometry.unwrap_or_else(|| {
                        Arc::new(CommentGeometry {
                            document: Arc::clone(&document),
                            pages: HashSet::new(),
                            unavailable_pages: HashSet::new(),
                            lines: Vec::new(),
                            page_heights: Vec::new(),
                        })
                    })
                };
                let mut anchors = anchor_comments_to_blocks(
                    &roots,
                    &document.block_source_lines,
                    &geometry.lines,
                    &geometry.page_heights,
                );
                // Attach hidden-furniture/note comments to the nearest visible body
                // so switching feedback mode never silently loses an annotation.
                let visible = document
                    .blocks
                    .iter()
                    .enumerate()
                    .filter(|(_, block)| {
                        !is_review_note_display_block(block)
                            && !matches!(block.role, LiquidBlockRole::Metadata)
                            && !review_skips_block_as_furniture(block, false)
                    })
                    .map(|(index, _)| index)
                    .collect::<Vec<_>>();
                let old = std::mem::take(&mut anchors);
                for (block, indices) in old {
                    if let Some(&target) =
                        visible.iter().min_by_key(|&&index| index.abs_diff(block))
                    {
                        anchors.entry(target).or_default().extend(indices);
                    }
                }
                let anchored = anchors.values().flatten().copied().collect::<HashSet<_>>();
                let unanchored = (0..threads.len())
                    .filter(|index| !anchored.contains(index))
                    .collect();
                let _ = tx.send(DerivedResult {
                    epoch,
                    revision,
                    generation,
                    presentation: ReviewComments {
                        threads,
                        anchors,
                        colors,
                        unanchored,
                    },
                    geometry,
                });
                ctx.request_repaint();
            });
        }
        Arc::clone(&cache.derived)
    }

    pub(super) fn draw_comment_load_status(&mut self, ui: &mut egui::Ui) {
        let Some(cache) = self.comments_ui.documents.get(&self.document_epoch) else {
            return;
        };
        if cache.loading || cache.pending_generation.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Loading comments…");
            });
        } else if let Some(error) = cache.error.clone() {
            ui.label(error);
            if ui.button("Retry comments").clicked() {
                self.comments_ui.documents.remove(&self.document_epoch);
            }
        } else if cache.derived.threads.is_empty() {
            ui.label(
                RichText::new("No comments in this document").color(self.liquid_muted_color()),
            );
        }
    }

    pub(super) fn draw_narrow_comments(&mut self, ui: &mut egui::Ui, comments: &ReviewComments) {
        if comments.threads.is_empty() {
            return;
        }
        egui::CollapsingHeader::new(format!("Comments ({})", comments.threads.len()))
            .default_open(true)
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("narrow-review-comments")
                    .max_height(260.0)
                    .show(ui, |ui| {
                        self.draw_comment_list(ui, comments, 0..comments.threads.len())
                    });
            });
    }

    pub(super) fn draw_unanchored_comments(
        &mut self,
        ui: &mut egui::Ui,
        comments: &ReviewComments,
    ) {
        if comments.unanchored.is_empty() {
            return;
        }
        egui::CollapsingHeader::new(format!(
            "Comments outside this Review excerpt ({})",
            comments.unanchored.len()
        ))
        .default_open(true)
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("unanchored-review-comments")
                .max_height(260.0)
                .show(ui, |ui| {
                    self.draw_comment_list(ui, comments, comments.unanchored.iter().copied())
                });
        });
    }

    fn draw_comment_list(
        &mut self,
        ui: &mut egui::Ui,
        comments: &ReviewComments,
        indices: impl IntoIterator<Item = usize>,
    ) {
        for index in indices {
            let Some(thread) = comments.threads.get(index) else {
                continue;
            };
            for comment in std::iter::once(&thread.root).chain(&thread.replies) {
                let name = self.comment_display_author(
                    comment.author.as_deref().unwrap_or(""),
                    comment.author_id.as_deref().unwrap_or(""),
                );
                let color = comments
                    .colors
                    .get(&crate::comments::comment_author_key(comment))
                    .copied()
                    .unwrap_or(comment.color_rgb);
                let color = self.comment_author_label_color(color_from_rgb(color, 255));
                ui.horizontal(|ui| {
                    if comment.is_owned_by(&self.settings.comment_author_id) {
                        if ui.link(RichText::new(name).color(color)).clicked() {
                            self.open_comment_profile();
                        }
                    } else {
                        ui.label(RichText::new(name).color(color).strong());
                    }
                });
                ui.label(&comment.text);
            }
            let anchor = comments
                .anchors
                .iter()
                .find(|(_, indices)| indices.contains(&index))
                .map(|(&block, _)| block);
            if let Some(block) = anchor {
                if ui.small_button("Go to passage").clicked() {
                    self.liquid_scroll_to_block = Some(block);
                }
            } else if ui
                .small_button(format!("Open PDF page {}", thread.root.page_index + 1))
                .clicked()
            {
                self.page_index = thread.root.page_index;
                self.scroll_target_page = Some(thread.root.page_index);
                self.set_view_mode(DocumentViewMode::Pdf, ui.ctx());
            }
            ui.separator();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_is_persisted_before_identity_becomes_available_to_comments() {
        let mut settings = AppSettings::default();
        assert!(
            persist_comment_profile(&mut settings, "Reader".to_owned(), |_| Err(
                "disk full".to_owned()
            ))
            .is_err()
        );
        assert!(settings.comment_author_id.is_empty());
        assert!(settings.comment_author_name.is_empty());
        let mut persisted_id = String::new();
        persist_comment_profile(&mut settings, "Reader".to_owned(), |candidate| {
            assert_eq!(candidate.comment_author_name, "Reader");
            assert!(!candidate.comment_author_id.is_empty());
            persisted_id = candidate.comment_author_id.clone();
            Ok(())
        })
        .unwrap();
        assert_eq!(settings.comment_author_id, persisted_id);
        assert!(
            persist_comment_profile(&mut settings, "New name".to_owned(), |_| Err(
                "disk full".to_owned()
            ))
            .is_err()
        );
        assert_eq!(settings.comment_author_name, "Reader");
        assert_eq!(settings.comment_author_id, persisted_id);
    }

    fn review_document() -> Arc<LiquidDocument> {
        Arc::new(
            serde_json::from_value(serde_json::json!({
                "title":"Article", "blocks":[], "noise_lines_removed":0, "llm_used":false,
                "warnings":[], "source_signature":"source"
            }))
            .unwrap(),
        )
    }

    fn result(
        document: &Arc<LiquidDocument>,
        revision: FileRevision,
        generation: u64,
    ) -> DerivedResult {
        DerivedResult {
            epoch: 1,
            revision,
            generation: (generation, Arc::as_ptr(document) as usize, 0),
            presentation: ReviewComments {
                anchors: HashMap::from([(12, vec![0])]),
                ..Default::default()
            },
            geometry: Arc::new(CommentGeometry {
                document: Arc::clone(document),
                pages: HashSet::from([0]),
                unavailable_pages: HashSet::new(),
                lines: Vec::new(),
                page_heights: vec![792.0],
            }),
        }
    }

    #[test]
    fn old_revision_result_cannot_clear_new_pending_or_restore_comments() {
        let document = review_document();
        let old = FileRevision::from_bytes(b"old PDF");
        let current = FileRevision::from_bytes(b"new PDF");
        let generation = (3, Arc::as_ptr(&document) as usize, 0);
        let mut cache = CommentCache {
            revision: Some(current.clone()),
            desired_generation: Some(generation),
            pending_generation: Some((current.clone(), generation)),
            ..Default::default()
        };
        cache.accept_derived(result(&document, old, 3));
        assert!(cache.derived.anchors.is_empty());
        assert_eq!(
            cache.pending_generation,
            Some((current.clone(), generation))
        );
        cache.accept_derived(result(&document, current, 3));
        assert_eq!(cache.derived.anchors[&12], vec![0]);
        assert!(cache.pending_generation.is_none());
    }

    #[test]
    fn obsolete_annotation_results_reuse_geometry_without_restoring_deleted_comments() {
        let document = review_document();
        let revision = FileRevision::from_bytes(b"PDF");
        let old_generation = (2, Arc::as_ptr(&document) as usize, 0);
        let desired = (3, Arc::as_ptr(&document) as usize, 0);
        let mut cache = CommentCache {
            revision: Some(revision.clone()),
            desired_generation: Some(desired),
            pending_generation: Some((revision.clone(), old_generation)),
            ..Default::default()
        };
        cache.accept_derived(result(&document, revision.clone(), 2));
        assert!(cache.derived.anchors.is_empty());
        assert!(cache.pending_generation.is_none());
        assert!(Arc::ptr_eq(
            &cache.geometry.as_ref().unwrap().document,
            &document
        ));
        let rotated = review_document();
        cache.desired_generation = Some((3, Arc::as_ptr(&rotated) as usize, 0));
        cache.geometry = None;
        cache.accept_derived(result(&document, revision, 3));
        assert!(cache.derived.anchors.is_empty());
        assert!(cache.geometry.is_none());
    }

    #[test]
    fn unavailable_comment_pages_retry_only_once_when_native_text_becomes_ready() {
        let document = review_document();
        let mut cache = CommentCache {
            desired_generation: Some((0, Arc::as_ptr(&document) as usize, 0)),
            geometry: Some(Arc::new(CommentGeometry {
                document: Arc::clone(&document),
                pages: HashSet::new(),
                unavailable_pages: HashSet::from([0]),
                lines: Vec::new(),
                page_heights: vec![792.0],
            })),
            ..Default::default()
        };
        cache.refresh_geometry_readiness(&document, &[None]);
        cache.refresh_geometry_readiness(&document, &[None]);
        assert_eq!(cache.geometry_serial, 0);
        cache.refresh_geometry_readiness(&document, &[Some(Vec::new())]);
        assert_eq!(cache.geometry_serial, 1);
        cache.refresh_geometry_readiness(&document, &[Some(Vec::new())]);
        assert_eq!(cache.geometry_serial, 1);
    }
}
