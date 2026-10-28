use super::*;
use crate::tts::{SpeechCancellation, SpeechEvent};

pub(super) struct TtsController {
    pub(super) active: Option<SpeechCancellation>,
    pub(super) job_id: u64,
    pub(super) exporting: bool,
    pub(super) tx: Sender<SpeechEvent>,
    pub(super) rx: Receiver<SpeechEvent>,
    pub(super) progress: Option<(usize, usize)>,
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
        }
    }

    pub(super) fn is_active(&self) -> bool {
        self.active.is_some()
    }
}
