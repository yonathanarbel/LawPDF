use super::*;

pub(super) struct SearchState {
    pub(super) query: String,
    pub(super) focus_request: bool,
    pub(super) hits: Vec<SearchHit>,
    pub(super) selected_hit: Option<usize>,
    pub(super) show_highlights: bool,
    pub(super) pending_annotation: bool,
}

impl Default for SearchState {
    fn default() -> Self {
        Self {
            query: String::new(),
            focus_request: false,
            hits: Vec::new(),
            selected_hit: None,
            show_highlights: true,
            pending_annotation: false,
        }
    }
}

impl SearchState {
    pub(super) fn replace_page_hits(&mut self, page: usize, hits: Vec<SearchHit>) {
        let selected = self
            .selected_hit
            .and_then(|index| self.hits.get(index))
            .cloned();
        self.hits
            .retain(|hit| hit.source == SearchSource::ReviewText || hit.page_index != page);
        self.hits.extend(hits);
        self.restore_selection(selected);
    }

    fn restore_selection(&mut self, selected: Option<SearchHit>) {
        sort_search_hits(&mut self.hits);
        self.selected_hit = selected
            .as_ref()
            .and_then(|selected| self.hits.iter().position(|hit| hit == selected))
            .or_else(|| (!self.hits.is_empty()).then_some(0));
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
            block_index: Some(hit.block_index),
        })
        .collect()
}

impl PdfEditorApp {
    pub(super) fn refresh_review_search(&mut self) {
        if self.search_state.query.trim().is_empty() {
            return;
        }
        let selected = self
            .search_state
            .selected_hit
            .and_then(|index| self.search_state.hits.get(index))
            .cloned();
        self.search_state
            .hits
            .retain(|hit| hit.source != SearchSource::ReviewText);
        if let LiquidState::Ready(review) = &self.liquid_mode2_state {
            self.search_state
                .hits
                .extend(review_search_hits(review, self.search_state.query.trim()));
        }
        self.search_state.restore_selection(selected);
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
