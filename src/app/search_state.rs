use super::*;

#[derive(Default)]
pub(super) struct SearchState {
    pub(super) query: String,
    pub(super) focus_request: bool,
    pub(super) hits: Vec<SearchHit>,
    pub(super) selected_hit: Option<usize>,
    pub(super) show_highlights: bool,
    pub(super) pending_annotation: bool,
    pub(super) open: bool,
    pub(super) due: Option<Instant>,
    pub(super) session: Option<FindSession>,
    pub(super) pending_step: Option<bool>,
    pub(super) landing: bool,
    pub(super) block_landed: bool,
    pub(super) activated: Option<Instant>,
    pub(super) wrapped: bool,
    pub(super) split_original: bool,
    pub(super) close_sidebar: bool,
    pub(super) geometry: HashMap<usize, HashMap<usize, Vec<PdfRect>>>,
    pub(super) density: std::collections::BTreeMap<usize, usize>,
    pub(super) review_ranges: HashMap<usize, Vec<usize>>,
    pub(super) review_anchor: Option<usize>,
    pub(super) review_anchor_byte: usize,
    pub(super) follow_result: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum FindScope {
    Original,
    Review,
}

enum FindWork {
    Page(usize, String, Option<String>),
    Review(Arc<LiquidDocument>),
}
enum FindResult {
    Page(usize, Vec<SearchHit>),
    Review(Vec<SearchHit>),
}

/// Each query has its own channels. Dropping the receiver on an edit/tab switch
/// prevents even a late completion for the old query from touching the new list.
pub(super) struct FindSession {
    epoch: u64,
    pub(super) scope: FindScope,
    pub(super) query: String,
    pages: VecDeque<usize>,
    queued: HashSet<usize>,
    waiting: HashSet<usize>,
    pub(super) scanned: HashSet<usize>,
    inflight: usize,
    review: Option<Arc<LiquidDocument>>,
    review_complete: bool,
    tx: Sender<FindWork>,
    rx: Receiver<FindResult>,
}

impl FindSession {
    fn new(
        epoch: u64,
        scope: FindScope,
        query: String,
        count: usize,
        anchor: usize,
        ctx: &Context,
    ) -> Self {
        let (tx, input) = crossbeam_channel::bounded::<FindWork>(8);
        let (output, rx) = unbounded();
        let needle = query.clone();
        let repaint = ctx.clone();
        std::thread::spawn(move || {
            for job in input {
                let result = match job {
                    FindWork::Page(page, text, ocr) => {
                        let mut hits = find_hits(&text, &needle, page, SearchSource::NativeText);
                        if let Some(ocr) =
                            ocr.filter(|ocr| !ocr.trim().is_empty() && ocr.trim() != text.trim())
                        {
                            hits.extend(find_hits(&ocr, &needle, page, SearchSource::OcrText));
                        }
                        FindResult::Page(page, hits)
                    }
                    FindWork::Review(review) => {
                        FindResult::Review(review_search_hits(&review, &needle))
                    }
                };
                if output.send(result).is_err() {
                    break;
                }
                repaint.request_repaint();
            }
        });
        let mut pages: Vec<_> = if scope == FindScope::Original {
            (0..count).collect()
        } else {
            vec![]
        };
        pages.sort_by_key(|&p| (p.abs_diff(anchor), p < anchor));
        Self {
            epoch,
            scope,
            query,
            queued: pages.iter().copied().collect(),
            pages: pages.into(),
            waiting: HashSet::new(),
            scanned: HashSet::new(),
            inflight: 0,
            review: None,
            review_complete: false,
            tx,
            rx,
        }
    }
}
impl SearchState {
    pub(super) fn replace_page_hits(&mut self, page: usize, hits: Vec<SearchHit>) {
        let selected = self
            .selected_hit
            .and_then(|index| self.hits.get(index))
            .cloned();
        let start = self.hits.partition_point(|h| h.page_index < page);
        let end = start
            + self.hits[start..]
                .partition_point(|h| h.page_index == page && h.source != SearchSource::ReviewText);
        let end = end.max(start);
        self.hits.splice(start..end, hits);
        self.geometry.remove(&page);
        let count = self.hits[start..].partition_point(|h| h.page_index == page);
        if count == 0 {
            self.density.remove(&page);
        } else {
            self.density.insert(page, count);
        }
        self.selected_hit = selected.and_then(|old| {
            let base = self.hits.partition_point(|h| h.page_index < old.page_index);
            self.hits[base..]
                .iter()
                .take_while(|h| h.page_index == old.page_index)
                .position(|h| {
                    h.source == old.source
                        && h.match_start == old.match_start
                        && h.block_index == old.block_index
                })
                .map(|i| i + base)
        });
    }

    fn restore_selection(&mut self, selected: Option<SearchHit>) {
        self.selected_hit = selected.as_ref().and_then(|selected| {
            self.hits.iter().position(|hit| {
                hit.page_index == selected.page_index
                    && hit.source == selected.source
                    && hit.block_index == selected.block_index
                    && hit.match_start == selected.match_start
            })
        });
    }
}

pub(super) fn review_search_hits(review: &LiquidDocument, query: &str) -> Vec<SearchHit> {
    let source_pages = review
        .block_source_lines
        .iter()
        .filter_map(|source| {
            source
                .lines
                .first()
                .map(|line| (source.block_index, line.page_index))
        })
        .collect::<HashMap<_, _>>();
    find_hits_in_review_blocks(&review.blocks, query)
        .into_iter()
        .map(|hit| SearchHit {
            page_index: source_pages.get(&hit.block_index).copied().unwrap_or(0),
            source: SearchSource::ReviewText,
            match_start: hit.match_start,
            match_end: hit.match_end,
            snippet: hit.snippet,
            snippet_match: hit.snippet_match,
            block_index: Some(hit.block_index),
        })
        .collect()
}

impl PdfEditorApp {
    pub(super) fn find_scope(&self) -> FindScope {
        if self.sbs_mode.is_none()
            && self.view_mode == DocumentViewMode::LiquidMode2
            && !(self.review_pdf_split && self.search_state.split_original)
        {
            FindScope::Review
        } else {
            FindScope::Original
        }
    }

    pub(super) fn update_search_page(&mut self, _ctx: &Context, page: usize) {
        if let Some(s) = &mut self.search_state.session
            && s.scope == FindScope::Original
        {
            s.waiting.remove(&page);
            s.scanned.remove(&page);
            if s.queued.insert(page) {
                s.pages.push_front(page);
            }
        }
    }

    pub(super) fn poll_find(&mut self, ctx: &Context) {
        if !self.search_state.open || self.search_state.query.trim().is_empty() {
            return;
        }
        if let Some(due) = self.search_state.due
            && due > Instant::now()
        {
            ctx.request_repaint_after(due.saturating_duration_since(Instant::now()));
            return;
        }
        self.search_state.due = None;
        let Some(doc) = self.document.as_ref() else {
            return;
        };
        let scope = self.find_scope();
        let review = if scope == FindScope::Review {
            match &self.liquid_mode2_state {
                LiquidState::Ready(review) => Some(Arc::clone(review)),
                _ => None,
            }
        } else {
            None
        };
        let mut session = self.search_state.session.take().filter(|s| {
            s.epoch == self.document_epoch
                && s.scope == scope
                && s.query == self.search_state.query.trim()
                && match (&s.review, &review) {
                    (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                    (None, None) => true,
                    _ => false,
                }
        });
        if session.is_none() {
            let selected = self
                .search_state
                .selected_hit
                .and_then(|i| self.search_state.hits.get(i))
                .cloned();
            self.search_state.hits.clear();
            self.search_state.review_ranges.clear();
            self.search_state.geometry.clear();
            self.search_state.density.clear();
            self.search_state.selected_hit = None;
            let mut s = FindSession::new(
                self.document_epoch,
                scope,
                self.search_state.query.trim().into(),
                doc.page_count,
                self.page_index,
                ctx,
            );
            if let Some(review) = review {
                s.tx.try_send(FindWork::Review(Arc::clone(&review))).ok();
                s.review = Some(review);
                s.inflight = 1;
            }
            if scope == FindScope::Review
                && let Some(hit) = selected.filter(|h| h.source == SearchSource::ReviewText)
            {
                self.search_state.hits.push(hit);
                self.search_state.selected_hit = Some(0);
            }
            session = Some(s);
        }
        let mut s = session.unwrap();
        while let Ok(result) = s.rx.try_recv() {
            s.inflight = s.inflight.saturating_sub(1);
            match result {
                FindResult::Page(page, hits) => {
                    s.scanned.insert(page);
                    self.search_state.replace_page_hits(page, hits);
                }
                FindResult::Review(hits) => {
                    let selected = self
                        .search_state
                        .selected_hit
                        .and_then(|i| self.search_state.hits.get(i))
                        .cloned();
                    self.search_state.hits = hits;
                    self.search_state.restore_selection(selected);
                    self.search_state.review_ranges.clear();
                    self.search_state.density.clear();
                    for (i, h) in self.search_state.hits.iter().enumerate() {
                        *self.search_state.density.entry(h.page_index).or_default() += 1;
                        if let Some(b) = h.block_index {
                            self.search_state
                                .review_ranges
                                .entry(b)
                                .or_default()
                                .push(i);
                        }
                    }
                    s.review_complete = true;
                }
            }
        }
        // Feed short batches. Only query-independent native text extraction may
        // survive a query change; no stale snippets or scroll targets can.
        for _ in 0..8 {
            if s.inflight >= 6 {
                break;
            }
            let Some(page) = s.pages.pop_front() else {
                break;
            };
            s.queued.remove(&page);
            let doc = self.document.as_ref().unwrap();
            if !doc.native_text_loaded.get(page).copied().unwrap_or(false) {
                if s.waiting.len() >= 6 {
                    s.queued.insert(page);
                    s.pages.push_front(page);
                    break;
                }
                let path = doc.path.clone();
                if self.pending_native_text.contains(&page) || self.enqueue_native_text(&path, page)
                {
                    s.waiting.insert(page);
                    continue;
                }
            }
            let doc = self.document.as_ref().unwrap();
            let text = doc.native_text.get(page).cloned().unwrap_or_default();
            let ocr = self
                .ocr_states
                .get(page)
                .and_then(OcrPageState::text)
                .map(String::from);
            if s.tx.try_send(FindWork::Page(page, text, ocr)).is_ok() {
                s.inflight += 1;
            }
        }
        let more = !s.pages.is_empty() && s.waiting.len() < 6 && s.inflight < 6;
        self.search_state.session = Some(s);
        if more {
            ctx.request_repaint();
        }
        if let Some(back) = self.search_state.pending_step {
            self.find_step(ctx, back);
        }
    }

    pub(super) fn find_progress(&self) -> String {
        if self.search_state.query.trim().is_empty() {
            return "Type to find".into();
        }
        let count = self.search_state.hits.len();
        let prefix = if let Some(i) = self.search_state.selected_hit {
            format!("{} of {count}", i + 1)
        } else {
            format!("{count} {}", if count == 1 { "match" } else { "matches" })
        };
        if let Some(s) = &self.search_state.session {
            let pages = self.document.as_ref().map_or(0, |d| d.page_count);
            if s.scope == FindScope::Original && s.scanned.len() < pages {
                return format!("{prefix} so far · {} of {pages} pages", s.scanned.len());
            }
            if s.scope == FindScope::Review {
                if matches!(self.liquid_mode2_state, LiquidState::Failed(_)) {
                    return "Review text unavailable".into();
                }
                if !s.review_complete {
                    return "Searching article…".into();
                }
                if !self.liquid_mode2_complete {
                    return format!("{prefix} so far · preparing article");
                }
            }
        } else if !self.search_state.query.trim().is_empty() {
            return "Searching…".into();
        }
        if self.search_state.wrapped {
            format!("{prefix} · wrapped")
        } else {
            prefix
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn hit(page: usize, source: SearchSource, start: usize) -> SearchHit {
        SearchHit {
            page_index: page,
            source,
            match_start: start,
            match_end: start + 3,
            snippet: "law".to_owned(),
            snippet_match: 0..3,
            block_index: (source == SearchSource::ReviewText).then_some(5),
        }
    }

    #[test]
    fn asynchronous_pdf_search_keeps_review_results_and_current_selection() {
        let review = hit(0, SearchSource::ReviewText, 10);
        let later = hit(2, SearchSource::NativeText, 5);
        let mut state = SearchState {
            hits: vec![review.clone(), later.clone()],
            selected_hit: Some(0),
            ..Default::default()
        };
        let first = hit(0, SearchSource::NativeText, 0);
        state.replace_page_hits(0, vec![first.clone()]);
        assert_eq!(state.hits, [first.clone(), review.clone(), later]);
        assert_eq!(state.hits[state.selected_hit.unwrap()], review);
        state.replace_page_hits(0, vec![first]);
        assert_eq!(
            state.hits.len(),
            3,
            "a repeated page result must replace its earlier hits"
        );
        assert_eq!(state.hits[state.selected_hit.unwrap()], review);
    }
}

pub(super) const WASH: Color32 = Color32::from_rgba_premultiplied(55, 45, 18, 62);
pub(super) fn next_hit(
    len: usize,
    selected: Option<usize>,
    back: bool,
    anchor: impl FnOnce() -> Option<usize>,
) -> (usize, bool) {
    debug_assert!(len > 0);
    if let Some(i) = selected.filter(|i| *i < len) {
        if back {
            if i == 0 {
                (len - 1, true)
            } else {
                (i - 1, false)
            }
        } else if i + 1 == len {
            (0, true)
        } else {
            (i + 1, false)
        }
    } else {
        anchor().map_or((if back { len - 1 } else { 0 }, true), |i| (i, false))
    }
}

#[cfg(test)]
mod interaction_tests;
