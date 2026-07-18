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
