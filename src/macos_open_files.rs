use std::path::PathBuf;

use crossbeam_channel::Sender;
use objc2::rc::Retained;
use objc2::runtime::Sel;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_foundation::{NSAppleEventDescriptor, NSAppleEventManager, NSObject, NSObjectProtocol};

const CORE_EVENT_CLASS: u32 = u32::from_be_bytes(*b"aevt");
const OPEN_DOCUMENTS_EVENT: u32 = u32::from_be_bytes(*b"odoc");
const DIRECT_OBJECT_KEYWORD: u32 = u32::from_be_bytes(*b"----");

struct OpenDocumentsHandlerIvars {
    sender: Sender<Vec<PathBuf>>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and this main-thread-only
    // object is retained for the duration of the application.
    #[unsafe(super = NSObject)]
    #[name = "LawPDFOpenDocumentsHandler"]
    #[thread_kind = MainThreadOnly]
    #[ivars = OpenDocumentsHandlerIvars]
    struct OpenDocumentsHandler;

    unsafe impl NSObjectProtocol for OpenDocumentsHandler {}

    impl OpenDocumentsHandler {
        #[unsafe(method(handleOpenDocuments:withReplyEvent:))]
        fn handle_open_documents(
            &self,
            event: &NSAppleEventDescriptor,
            _reply: &NSAppleEventDescriptor,
        ) {
            let Some(direct_object) = event.paramDescriptorForKeyword(DIRECT_OBJECT_KEYWORD) else {
                return;
            };
            let paths = (1..=direct_object.numberOfItems())
                .filter_map(|index| direct_object.descriptorAtIndex(index))
                .filter_map(|descriptor| descriptor.fileURLValue())
                .filter_map(|url| url.path())
                .map(|path| PathBuf::from(path.to_string()))
                .collect::<Vec<_>>();

            if !paths.is_empty() && self.ivars().sender.send(paths).is_ok() {
                crate::single_instance::request_repaint();
            }
        }
    }
);

pub struct OpenDocumentsRegistration {
    _handler: Retained<OpenDocumentsHandler>,
}

impl OpenDocumentsRegistration {
    pub fn register(&self) {
        register_handler(&self._handler);
    }
}

pub fn install(sender: Sender<Vec<PathBuf>>) -> OpenDocumentsRegistration {
    let mtm = MainThreadMarker::new().expect("LawPDF must start on the macOS main thread");
    let handler: Retained<OpenDocumentsHandler> = {
        let allocated =
            OpenDocumentsHandler::alloc(mtm).set_ivars(OpenDocumentsHandlerIvars { sender });
        // SAFETY: NSObject's `init` signature is correct for this subclass.
        unsafe { msg_send![super(allocated), init] }
    };
    register_handler(&handler);
    OpenDocumentsRegistration { _handler: handler }
}

fn register_handler(handler: &OpenDocumentsHandler) {
    let manager = NSAppleEventManager::sharedAppleEventManager();
    let selector: Sel = sel!(handleOpenDocuments:withReplyEvent:);
    // SAFETY: `OpenDocumentsHandler` implements the selector with the exact
    // two-NSAppleEventDescriptor signature required by NSAppleEventManager.
    unsafe {
        manager.setEventHandler_andSelector_forEventClass_andEventID(
            handler,
            selector,
            CORE_EVENT_CLASS,
            OPEN_DOCUMENTS_EVENT,
        );
    }
}
