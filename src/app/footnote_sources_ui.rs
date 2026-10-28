//! Source lookup is an explicit reading action. Painting a note performs no I/O.
use super::*;
use crate::footnote_sources::{self, LookupEvent, LookupRequest, SourceSearch};
use sha2::{Digest, Sha256};

const MAX_BATCH: usize = 32;
const MAX_PENDING: usize = 64;

#[derive(Default)]
pub(super) struct FootnoteSourcesUi {
    endpoint: Option<Result<String, String>>,
    views: HashMap<u64, HashMap<String, NoteSources>>,
    results: HashMap<String, LookupState>,
    chosen: HashMap<String, String>,
    pending: VecDeque<PendingLookup>,
    active: Option<String>,
    worker: Option<Sender<LookupRequest>>,
    receiver: Option<Receiver<LookupEvent>>,
    last_tabs: (u64, usize),
}

struct NoteSources {
    text: String,
    key: String,
    query: String,
    selection_key: String,
    links: Arc<Vec<String>>,
}

impl NoteSources {
    fn matches(&self, text: &str) -> bool {
        self.text == text
    }
}

enum LookupState {
    Queued,
    Ready(Arc<SourceSearch>),
    Failed(String),
}

struct PendingLookup {
    epoch: u64,
    request: LookupRequest,
}

impl PdfEditorApp {
    fn ensure_note_sources(&mut self, note_key: &str, text: &str) -> String {
        let epoch = self.document_epoch;
        if let Some(view) = self
            .footnote_sources_ui
            .views
            .get(&epoch)
            .and_then(|views| views.get(note_key))
            && view.matches(text)
        {
            return view.key.clone();
        }
        let endpoint = self
            .footnote_sources_ui
            .endpoint
            .get_or_insert_with(footnote_sources::configured_endpoint);
        let query = footnote_sources::normalize_query(text);
        let key = footnote_sources::query_key(endpoint.as_deref().unwrap_or("unavailable"), &query);
        let links = Arc::new(crate::citation_links::explicit_links(text));
        self.footnote_sources_ui
            .views
            .entry(epoch)
            .or_default()
            .insert(
                note_key.to_owned(),
                NoteSources {
                    text: text.to_owned(),
                    key: key.clone(),
                    query,
                    selection_key: format!("{:x}", Sha256::digest(text.as_bytes())),
                    links,
                },
            );
        key
    }

    pub(super) fn footnote_sources_button_label(&self, note_key: &str, text: &str) -> &'static str {
        let Some(view) = self
            .footnote_sources_ui
            .views
            .get(&self.document_epoch)
            .and_then(|views| views.get(note_key))
            .filter(|view| view.matches(text))
        else {
            return "Find source";
        };
        if self
            .footnote_sources_ui
            .chosen
            .contains_key(&view.selection_key)
        {
            return "Open source";
        }
        match self.footnote_sources_ui.results.get(&view.key) {
            Some(LookupState::Queued) => "Searching…",
            Some(LookupState::Ready(results)) if !results.candidates.is_empty() => "Sources",
            _ if !view.links.is_empty() => "Sources",
            _ => "Find source",
        }
    }

    pub(super) fn request_footnote_sources(&mut self, note_key: &str, text: &str, ctx: &Context) {
        self.ensure_note_sources(note_key, text);
        let selection_key = self.footnote_sources_ui.views[&self.document_epoch][note_key]
            .selection_key
            .clone();
        if let Some(url) = self.footnote_sources_ui.chosen.get(&selection_key) {
            ctx.open_url(egui::OpenUrl::new_tab(url));
            return;
        }
        let view = &self.footnote_sources_ui.views[&self.document_epoch][note_key];
        if view.links.len() == 1 {
            let url = view.links[0].clone();
            ctx.open_url(egui::OpenUrl::new_tab(&url));
            self.footnote_sources_ui.chosen.insert(selection_key, url);
            return;
        }
        if !view.links.is_empty() {
            return;
        }
        self.queue_footnote_lookup(note_key, text, false, ctx);
    }

    fn queue_footnote_lookup(
        &mut self,
        note_key: &str,
        text: &str,
        retry: bool,
        ctx: &Context,
    ) -> bool {
        let key = self.ensure_note_sources(note_key, text);
        if matches!(
            self.footnote_sources_ui.results.get(&key),
            Some(LookupState::Queued)
        ) || (!retry
            && matches!(
                self.footnote_sources_ui.results.get(&key),
                Some(LookupState::Ready(_))
            ))
        {
            return false;
        }
        if self.footnote_sources_ui.pending.len() >= MAX_PENDING {
            self.status =
                "Source lookup queue is full. Let the current notes finish first.".to_owned();
            return false;
        }
        if let Some(Err(error)) = &self.footnote_sources_ui.endpoint {
            self.footnote_sources_ui
                .results
                .insert(key, LookupState::Failed(error.clone()));
            return false;
        }
        let query = self.footnote_sources_ui.views[&self.document_epoch][note_key]
            .query
            .clone();
        if query.is_empty() {
            return false;
        }
        self.footnote_sources_ui
            .results
            .insert(key.clone(), LookupState::Queued);
        self.footnote_sources_ui.pending.push_back(PendingLookup {
            epoch: self.document_epoch,
            request: LookupRequest {
                key,
                query,
                refresh: retry,
            },
        });
        self.pump_footnote_lookup(ctx);
        true
    }

    fn pump_footnote_lookup(&mut self, ctx: &Context) {
        if self.footnote_sources_ui.active.is_some() || self.footnote_sources_ui.pending.is_empty()
        {
            return;
        }
        if self.footnote_sources_ui.worker.is_none() {
            let Some(Ok(endpoint)) = &self.footnote_sources_ui.endpoint else {
                return;
            };
            let (tx, rx) = unbounded();
            self.footnote_sources_ui.worker = Some(footnote_sources::spawn_lookup_worker(
                endpoint.clone(),
                tx,
                ctx.clone(),
            ));
            self.footnote_sources_ui.receiver = Some(rx);
        }
        let pending = self.footnote_sources_ui.pending.pop_front().unwrap();
        let key = pending.request.key.clone();
        match self
            .footnote_sources_ui
            .worker
            .as_ref()
            .unwrap()
            .try_send(pending.request)
        {
            Ok(()) => self.footnote_sources_ui.active = Some(key),
            Err(error) => {
                self.footnote_sources_ui
                    .results
                    .insert(key, LookupState::Failed(error.to_string()));
            }
        }
        ctx.request_repaint();
    }

    pub(super) fn poll_footnote_sources(&mut self, ctx: &Context) {
        if self.footnote_sources_ui.views.is_empty() {
            return;
        }
        if let Some(receiver) = &self.footnote_sources_ui.receiver {
            while let Ok(event) = receiver.try_recv() {
                if self.footnote_sources_ui.active.as_deref() == Some(event.key.as_str()) {
                    self.footnote_sources_ui.active = None;
                }
                let state = match event.result {
                    Ok(results) => {
                        if results.candidates.is_empty() && !results.partial_errors.is_empty() {
                            for pending in self.footnote_sources_ui.pending.drain(..) {
                                self.footnote_sources_ui
                                    .results
                                    .remove(&pending.request.key);
                            }
                            self.status = "Source search paused because the search engine did not finish. Retry one footnote to resume.".to_owned();
                        }
                        LookupState::Ready(Arc::new(results))
                    }
                    Err(error) => {
                        for pending in self.footnote_sources_ui.pending.drain(..) {
                            self.footnote_sources_ui
                                .results
                                .remove(&pending.request.key);
                        }
                        self.status = "Source search paused after a failed request. Retry one footnote to resume.".to_owned();
                        LookupState::Failed(error)
                    }
                };
                self.footnote_sources_ui.results.insert(event.key, state);
            }
        }
        let tabs = (self.document_epoch, self.tabs.len());
        if self.footnote_sources_ui.last_tabs != tabs {
            self.footnote_sources_ui.last_tabs = tabs;
            let epochs = self
                .tabs
                .iter()
                .map(|tab| tab.document_epoch)
                .chain(std::iter::once(self.document_epoch))
                .collect::<HashSet<_>>();
            self.footnote_sources_ui
                .views
                .retain(|epoch, _| epochs.contains(epoch));
            let mut removed = Vec::new();
            self.footnote_sources_ui.pending.retain(|pending| {
                if epochs.contains(&pending.epoch) {
                    true
                } else {
                    removed.push(pending.request.key.clone());
                    false
                }
            });
            for key in removed {
                self.footnote_sources_ui.results.remove(&key);
            }
            let mut keys = self
                .footnote_sources_ui
                .views
                .values()
                .flat_map(|views| views.values().map(|view| view.key.clone()))
                .collect::<HashSet<_>>();
            keys.extend(self.footnote_sources_ui.active.iter().cloned());
            keys.extend(
                self.footnote_sources_ui
                    .pending
                    .iter()
                    .map(|pending| pending.request.key.clone()),
            );
            self.footnote_sources_ui
                .results
                .retain(|key, _| keys.contains(key));
            let selections = self
                .footnote_sources_ui
                .views
                .values()
                .flat_map(|views| views.values().map(|view| view.selection_key.clone()))
                .collect::<HashSet<_>>();
            self.footnote_sources_ui
                .chosen
                .retain(|key, _| selections.contains(key));
        }
        self.pump_footnote_lookup(ctx);
    }

    pub(super) fn draw_footnote_sources(&mut self, ui: &mut egui::Ui, note_key: &str, text: &str) {
        let key = self.ensure_note_sources(note_key, text);
        let links =
            Arc::clone(&self.footnote_sources_ui.views[&self.document_epoch][note_key].links);
        let selection_key = self.footnote_sources_ui.views[&self.document_epoch][note_key]
            .selection_key
            .clone();
        for url in links.iter() {
            if ui.hyperlink_to(source_link_label(url), url).clicked() {
                self.footnote_sources_ui
                    .chosen
                    .insert(selection_key.clone(), url.clone());
            }
        }
        let mut search = false;
        let mut refresh = false;
        let result = match self.footnote_sources_ui.results.get(&key) {
            Some(LookupState::Ready(result)) => Some(Arc::clone(result)),
            Some(LookupState::Queued) => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Finding article sources…");
                });
                None
            }
            Some(LookupState::Failed(error)) => {
                ui.label(error);
                search = ui.small_button("Retry source search").clicked();
                refresh = true;
                None
            }
            None => {
                search = ui.small_button(if links.is_empty() { "Find article sources" } else { "Search for other articles" })
                    .on_hover_text("Searches BBGPT, or Crossref if BBGPT is unavailable, using this footnote text. The PDF stays on this computer.").clicked();
                None
            }
        };
        if let Some(result) = result {
            if result.via_crossref {
                ui.label(RichText::new("Crossref results · BBGPT unavailable").small());
            }
            if result.candidates.is_empty() {
                ui.label(if result.partial_errors.is_empty() {
                    "No article sources found."
                } else {
                    "The search was incomplete; no sources were returned."
                });
            } else {
                ui.label(RichText::new("Possible article sources").strong());
                for candidate in &result.candidates {
                    if ui.hyperlink_to(&candidate.title, &candidate.url).clicked() {
                        self.footnote_sources_ui
                            .chosen
                            .insert(selection_key.clone(), candidate.url.clone());
                    }
                    let authors = candidate.authors.join(", ");
                    let detail = match candidate.year {
                        Some(year) if !authors.is_empty() => format!("{authors} · {year}"),
                        Some(year) => year.to_string(),
                        None => authors,
                    };
                    if !detail.is_empty() {
                        ui.label(RichText::new(detail).small());
                    }
                    ui.add_space(3.0);
                }
            }
            if !result.partial_errors.is_empty() {
                ui.label("Some search engines did not finish.");
            }
            search |= ui.small_button("Search again").clicked();
            refresh = true;
        }
        if search {
            self.queue_footnote_lookup(note_key, text, refresh, ui.ctx());
        }
    }

    pub(super) fn draw_footnote_source_batch_controls(&mut self, ui: &mut egui::Ui) {
        let document = match &self.liquid_mode2_state {
            LiquidState::Ready(document) => Some(Arc::clone(document)),
            _ => None,
        };
        if ui.add_enabled(document.is_some(), egui::Button::new("Find article links"))
            .on_hover_text("Search up to 32 unresolved footnotes in the prepared Review using BBGPT, or Crossref if BBGPT is unavailable. Only footnote text is sent; existing URLs and DOIs are reused locally.")
            .clicked() && let Some(document) = document {
            let mut queued = 0;
            'blocks: for (block_index, block) in document.blocks.iter().enumerate() {
                if !is_review_note_display_block(block) { continue; }
                for (ordinal, (marker, body)) in split_fused_review_notes(&block.text).into_iter().enumerate() {
                    let note_key = if marker.parse::<u16>().is_ok() { marker } else { format!("b{block_index}.{ordinal}") };
                    let body = callout_body_text("Footnote", &body).trim();
                    self.ensure_note_sources(&note_key, body);
                    if !self.footnote_sources_ui.views[&self.document_epoch][&note_key].links.is_empty()
                        || self.footnote_sources_ui.chosen.contains_key(&self.footnote_sources_ui.views[&self.document_epoch][&note_key].selection_key) { continue; }
                    if self.queue_footnote_lookup(&note_key, body, false, ui.ctx()) { queued += 1; }
                    if queued >= MAX_BATCH { break 'blocks; }
                }
            }
            self.status = if queued == 0 { "These footnotes already have links or cached source results.".to_owned() }
                else { format!("Finding article links for {queued} footnotes in the background.") };
        }
        let queued = self.footnote_sources_ui.pending.len();
        if queued > 0 {
            ui.label(format!("{queued} queued"));
            if ui.small_button("Stop queue").clicked() {
                for pending in self.footnote_sources_ui.pending.drain(..) {
                    self.footnote_sources_ui
                        .results
                        .remove(&pending.request.key);
                }
            }
        }
    }
}

fn source_link_label(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .map(|host| format!("Open source · {host}"))
        .unwrap_or_else(|| "Open source".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reused_note_number_cannot_display_another_notes_results() {
        let view = NoteSources {
            text: "Alice, First Article (2024)".to_owned(),
            key: "first".to_owned(),
            query: String::new(),
            selection_key: "full-body".to_owned(),
            links: Arc::default(),
        };
        assert!(view.matches("Alice, First Article (2024)"));
        assert!(!view.matches("Bob, Different Article (2025)"));
    }
}
