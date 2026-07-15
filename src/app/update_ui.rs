use super::*;

pub(super) struct UpdateUi {
    pub(super) tx: Sender<UpdateEvent>,
    pub(super) rx: Receiver<UpdateEvent>,
    pub(super) state: UpdateUiState,
    pub(super) check_in_flight: bool,
    pub(super) notice: Option<UpdateNotice>,
    pub(super) next_check: Option<Instant>,
}

impl UpdateUi {
    pub(super) fn new(
        tx: Sender<UpdateEvent>,
        rx: Receiver<UpdateEvent>,
        notice: Option<UpdateNotice>,
    ) -> Self {
        Self {
            tx,
            rx,
            state: UpdateUiState::Idle,
            check_in_flight: true,
            notice,
            next_check: None,
        }
    }
}
