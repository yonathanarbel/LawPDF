use super::*;

pub(super) struct UpdateUi {
    pub(super) tx: Sender<UpdateEvent>,
    pub(super) rx: Receiver<UpdateEvent>,
    pub(super) state: UpdateUiState,
    pub(super) check_in_flight: bool,
    pub(super) notice: Option<UpdateNotice>,
    pub(super) next_check: Option<Instant>,
    pub(super) download_version: Option<String>,
    pub(super) download_progress: Option<(u64, Option<u64>)>,
    pub(super) manual_check: bool,
    pub(super) last_check_error: Option<String>,
    pub(super) pending: Option<updater::PendingUpdate>,
    pub(super) manual_download_version: Option<String>,
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
            check_in_flight: !updater::managed_by_store(),
            notice,
            next_check: None,
            download_version: None,
            download_progress: None,
            manual_check: false,
            last_check_error: None,
            pending: None,
            manual_download_version: None,
        }
    }
}
