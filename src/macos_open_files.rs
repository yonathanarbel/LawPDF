use std::cell::{Cell, RefCell};
use std::path::PathBuf;

use crossbeam_channel::Sender;
use objc2::rc::Retained;
use objc2::runtime::Sel;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSApplicationWillFinishLaunchingNotification, NSMenu, NSMenuItem,
};
use objc2_foundation::{
    NSAppleEventDescriptor, NSAppleEventManager, NSNotification, NSNotificationCenter, NSObject,
    NSObjectProtocol, NSUserDefaults, ns_string,
};

const CORE_EVENT_CLASS: u32 = u32::from_be_bytes(*b"aevt");
const OPEN_DOCUMENTS_EVENT: u32 = u32::from_be_bytes(*b"odoc");
const QUIT_APPLICATION_EVENT: u32 = u32::from_be_bytes(*b"quit");
const DIRECT_OBJECT_KEYWORD: u32 = u32::from_be_bytes(*b"----");

/// Keep AppKit from creating its automatic Touch Bar responder observer.
///
/// LawPDF does not provide Touch Bar controls. On affected macOS releases,
/// AppKit's automatic observer can nevertheless remove the same KVO
/// registration twice while the window responder chain changes, terminating
/// the app in `_NSTouchBarFinderObservation::invalidate`. Install this default
/// before `eframe` asks AppKit to create `NSApplication` or any windows.
pub fn install_appkit_crash_workarounds() {
    NSUserDefaults::standardUserDefaults()
        .setBool_forKey(false, ns_string!("NSFunctionBarAPIEnabled"));
}

struct OpenDocumentsHandlerIvars {
    sender: Sender<Vec<PathBuf>>,
    quit_requested: Cell<bool>,
    quit_items: RefCell<Vec<Retained<NSMenuItem>>>,
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
        #[unsafe(method(requestQuit:))]
        fn request_quit(&self, _sender: Option<&NSObject>) {
            self.ivars().quit_requested.set(true);
            crate::single_instance::request_repaint();
        }

        #[unsafe(method(handleQuitApplication:withReplyEvent:))]
        fn handle_quit_application(
            &self,
            _event: &NSAppleEventDescriptor,
            _reply: &NSAppleEventDescriptor,
        ) {
            self.ivars().quit_requested.set(true);
            crate::single_instance::request_repaint();
        }

        #[unsafe(method(applicationWillFinishLaunching:))]
        fn application_will_finish_launching(&self, _notification: &NSNotification) {
            // NSApplication installs its default Apple-event handlers during
            // startup, overwriting our pre-run registration. Finder's initial
            // open-document event arrives BEFORE eframe creates the app, so
            // registering only in its AppCreator loses the cold-launch file.
            // Reinstall after AppKit's setup but before the initial event.
            register_handler(self);
        }

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
        let app = NSApplication::sharedApplication(MainThreadMarker::from(&*self._handler));
        if let Some(menu) = app.mainMenu() {
            route_quit_menu(&menu, &self._handler);
        }
    }

    pub fn take_quit_requested(&self) -> bool {
        self._handler.ivars().quit_requested.replace(false)
    }
}

impl Drop for OpenDocumentsRegistration {
    fn drop(&mut self) {
        // Unregister while the main-thread handler is still alive. AppKit must
        // not dispatch an open-document event into an application being torn down.
        NSAppleEventManager::sharedAppleEventManager()
            .removeEventHandlerForEventClass_andEventID(CORE_EVENT_CLASS, OPEN_DOCUMENTS_EVENT);
        NSAppleEventManager::sharedAppleEventManager()
            .removeEventHandlerForEventClass_andEventID(CORE_EVENT_CLASS, QUIT_APPLICATION_EVENT);
        for item in self._handler.ivars().quit_items.borrow_mut().drain(..) {
            // SAFETY: Clear the menu item's weak target before releasing it.
            unsafe {
                item.setTarget(None);
                item.setAction(Some(sel!(terminate:)));
            }
        }
        // SAFETY: The observer is retained until after it is unregistered.
        unsafe { NSNotificationCenter::defaultCenter().removeObserver(&*self._handler) };
    }
}

pub fn install(sender: Sender<Vec<PathBuf>>) -> OpenDocumentsRegistration {
    let mtm = MainThreadMarker::new().expect("LawPDF must start on the macOS main thread");
    let handler: Retained<OpenDocumentsHandler> = {
        let allocated = OpenDocumentsHandler::alloc(mtm).set_ivars(OpenDocumentsHandlerIvars {
            sender,
            quit_requested: Cell::new(false),
            quit_items: RefCell::new(Vec::new()),
        });
        // SAFETY: NSObject's `init` signature is correct for this subclass.
        unsafe { msg_send![super(allocated), init] }
    };
    // Observe launch without replacing winit's NSApplication delegate.
    // SAFETY: The selector accepts the notification argument and the observer
    // remains alive until OpenDocumentsRegistration unregisters it.
    unsafe {
        NSNotificationCenter::defaultCenter().addObserver_selector_name_object(
            &*handler,
            sel!(applicationWillFinishLaunching:),
            Some(NSApplicationWillFinishLaunchingNotification),
            None,
        );
    }
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
        manager.setEventHandler_andSelector_forEventClass_andEventID(
            handler,
            sel!(handleQuitApplication:withReplyEvent:),
            CORE_EVENT_CLASS,
            QUIT_APPLICATION_EVENT,
        );
    }
}

fn route_quit_menu(menu: &NSMenu, handler: &OpenDocumentsHandler) {
    for item in menu.itemArray() {
        if item.action() == Some(sel!(terminate:)) {
            // winit's native Quit bypasses egui's cancellable close/save flow.
            // SAFETY: Our selector accepts the sender, and Drop clears this
            // weak target before the retained handler is released.
            unsafe {
                item.setTarget(Some(handler));
                item.setAction(Some(sel!(requestQuit:)));
            }
            handler.ivars().quit_items.borrow_mut().push(item);
        } else if let Some(submenu) = item.submenu() {
            route_quit_menu(&submenu, handler);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disables_automatic_touch_bar_integration() {
        install_appkit_crash_workarounds();

        assert!(
            !NSUserDefaults::standardUserDefaults()
                .boolForKey(ns_string!("NSFunctionBarAPIEnabled"))
        );
    }
}
