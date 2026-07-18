use super::*;

pub(super) struct TtsController {
    /// #33 TTS: handle to the running native speech process (macOS `say`), if any.
    pub(super) child: Option<std::process::Child>,
    /// #33 TTS: whether to read footnotes as a separate pass after the body.
    pub(super) include_notes: bool,
    pub(super) provider: PaidTtsProvider,
    pub(super) tx: Sender<PaidTtsEvent>,
    pub(super) rx: Receiver<PaidTtsEvent>,
    pub(super) progress: Option<(usize, usize)>,
}

impl TtsController {
    pub(super) fn new() -> Self {
        let (tx, rx) = unbounded();
        Self {
            child: None,
            include_notes: false,
            provider: PaidTtsProvider::OpenRouter,
            tx,
            rx,
            progress: None,
        }
    }
}
