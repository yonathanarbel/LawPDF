use super::*;

#[derive(Default)]
pub(super) struct PlatformUi {
    pending: Option<Receiver<Result<(), String>>>,
}

impl PdfEditorApp {
    pub(super) fn request_linux_default_reader(&mut self, ctx: &Context) {
        if self.platform_ui.pending.is_some() {
            return;
        }
        let (sender, receiver) = unbounded();
        self.platform_ui.pending = Some(receiver);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = crate::linux_desktop::set_default_pdf_reader();
            let _ = sender.send(result);
            ctx.request_repaint();
        });
        self.status = "Registering LawPDF as the default PDF reader…".to_owned();
    }

    pub(super) fn poll_platform_actions(&mut self) {
        let Some(receiver) = &self.platform_ui.pending else {
            return;
        };
        match receiver.try_recv() {
            Ok(result) => {
                self.platform_ui.pending = None;
                match result {
                    Ok(()) => self.status = "LawPDF is now the default PDF reader.".to_owned(),
                    Err(error) => self.push_error_notice(error),
                }
            }
            Err(crossbeam_channel::TryRecvError::Disconnected) => {
                self.platform_ui.pending = None;
                self.push_error_notice("Desktop registration did not complete.");
            }
            Err(crossbeam_channel::TryRecvError::Empty) => {}
        }
    }
}
