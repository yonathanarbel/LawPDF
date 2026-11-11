use super::*;
use crate::tts::{SpeechCancellation, SpeechEvent};

pub(super) struct TtsController {
    pub(super) active: Option<SpeechCancellation>,
    pub(super) job_id: u64,
    pub(super) exporting: bool,
    pub(super) tx: Sender<SpeechEvent>,
    pub(super) rx: Receiver<SpeechEvent>,
    pub(super) progress: Option<(usize, usize)>,
    pub(super) follow_epoch: Option<u64>,
    pub(super) follow_view: Option<DocumentViewMode>,
    pub(super) follow_document: Option<std::sync::Weak<LiquidDocument>>,
    pub(super) speaking_block: Option<usize>,
    pub(super) follow_target: Option<usize>,
}

impl TtsController {
    pub(super) fn new() -> Self {
        let (tx, rx) = unbounded();
        Self {
            active: None,
            job_id: 0,
            exporting: false,
            tx,
            rx,
            progress: None,
            follow_epoch: None,
            follow_view: None,
            follow_document: None,
            speaking_block: None,
            follow_target: None,
        }
    }

    pub(super) fn is_active(&self) -> bool {
        self.active.is_some()
    }
}
