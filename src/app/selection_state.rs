use super::*;

#[derive(Default)]
pub(super) struct SelectionState {
    pub(super) text: Option<TextSelection>,
    pub(super) liquid_all: bool,
    pub(super) anchor: Option<(usize, usize)>,
    pub(super) toolbar_rect: Option<Rect>,
    pub(super) pending_select_all: bool,
}
