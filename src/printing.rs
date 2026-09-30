//! Print an immutable copy through PDFKit, keeping unsaved edits and the source safe.
use std::cell::Cell;
use std::ffi::c_void;
use std::path::{Path, PathBuf};

use objc2::rc::Retained;
use objc2::{
    AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel,
};
use objc2_app_kit::{NSApplication, NSPrintInfo, NSPrintOperation};
use objc2_foundation::{NSObject, NSObjectProtocol, NSString, NSURL};
use objc2_pdf_kit::{PDFDocument, PDFPrintScalingMode};

use crate::document_store::{DocumentStore, FileRevision};
use crate::model::EditorAnnotation;

pub(crate) struct PrintCopy {
    // Keep private temporary PDF bytes alive until the modal print operation
    // has finished spooling or has been cancelled, then remove them.
    _directory: tempfile::TempDir,
    path: PathBuf,
    title: String,
}

pub(crate) fn prepare_copy(
    store: &DocumentStore,
    source: &Path,
    revision: &FileRevision,
    annotations: &[EditorAnnotation],
) -> Result<PrintCopy, String> {
    let original = store.print_source(revision)?;
    let original_path = original
        .to_str()
        .ok_or_else(|| "The PDF path is not valid Unicode.".to_owned())?;
    let url = NSURL::fileURLWithPath(&NSString::from_str(original_path));
    // SAFETY: This PDFDocument is used only on this preparation thread. Check
    // the immutable original before export can remove an owner-password wrapper.
    unsafe {
        let original = PDFDocument::initWithURL(PDFDocument::alloc(), &url)
            .ok_or_else(|| "macOS could not read this PDF for printing.".to_owned())?;
        if original.isLocked() || !original.allowsPrinting() {
            return Err("This PDF does not allow printing.".to_owned());
        }
    }
    let directory = tempfile::Builder::new()
        .prefix("lawpdf-print-")
        .tempdir()
        .map_err(|error| format!("Could not prepare the print preview: {error}"))?;
    let path = directory.path().join("document.pdf");
    store.export_annotations(source, revision, annotations, &path)?;
    let title = source
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    Ok(PrintCopy {
        _directory: directory,
        path,
        title,
    })
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements. AppKit calls this
    // delegate on the main thread because separate-thread printing is disabled.
    #[unsafe(super = NSObject)]
    #[name = "LawPDFPrintCompletion"]
    #[thread_kind = MainThreadOnly]
    #[ivars = Cell<Option<bool>>]
    struct PrintCompletion;

    unsafe impl NSObjectProtocol for PrintCompletion {}

    impl PrintCompletion {
        #[unsafe(method(printOperationDidRun:success:contextInfo:))]
        fn print_operation_did_run(
            &self,
            _operation: &NSPrintOperation,
            success: bool,
            _context: *mut c_void,
        ) {
            self.ivars().set(Some(success));
            crate::single_instance::request_repaint();
        }
    }
);

pub(crate) struct PrintJob {
    _operation: Retained<NSPrintOperation>,
    _document: Retained<PDFDocument>,
    completion: Retained<PrintCompletion>,
    _copy: PrintCopy,
}

impl PrintJob {
    pub(crate) fn start(copy: PrintCopy) -> Result<Self, String> {
        let mtm = MainThreadMarker::new()
            .ok_or_else(|| "The print dialog must open on the main thread.".to_owned())?;
        let app = NSApplication::sharedApplication(mtm);
        let window = app
            .mainWindow()
            .or_else(|| app.keyWindow())
            .ok_or_else(|| "Bring the LawPDF window forward before printing.".to_owned())?;
        let path = copy
            .path
            .to_str()
            .ok_or_else(|| "The print path is not valid Unicode.".to_owned())?;
        let url = NSURL::fileURLWithPath(&NSString::from_str(path));
        // SAFETY: Native objects, callback target, and temporary PDF are retained
        // until didRun fires. A sheet returns immediately instead of nesting a
        // modal run loop inside winit's borrowed event handler (which can abort
        // when the PDF menu opens its Save panel).
        unsafe {
            let document = PDFDocument::initWithURL(PDFDocument::alloc(), &url)
                .ok_or_else(|| "macOS could not open this PDF for printing.".to_owned())?;
            if document.isLocked() || !document.allowsPrinting() {
                return Err("This PDF does not allow printing.".to_owned());
            }
            let info = NSPrintInfo::sharedPrintInfo();
            let operation = document
                .printOperationForPrintInfo_scalingMode_autoRotate(
                    Some(&info),
                    PDFPrintScalingMode::PageScaleDownToFit,
                    true,
                    mtm,
                )
                .ok_or_else(|| "macOS could not create a print preview for this PDF.".to_owned())?;
            operation.setJobTitle(Some(&NSString::from_str(&copy.title)));
            operation.setShowsPrintPanel(true);
            operation.setShowsProgressPanel(true);
            operation.setCanSpawnSeparateThread(false);
            let allocated = PrintCompletion::alloc(mtm).set_ivars(Cell::new(None));
            let completion: Retained<PrintCompletion> = msg_send![super(allocated), init];
            operation.runOperationModalForWindow_delegate_didRunSelector_contextInfo(
                &window,
                Some(&completion),
                Some(sel!(printOperationDidRun:success:contextInfo:)),
                std::ptr::null_mut(),
            );
            Ok(Self {
                _operation: operation,
                _document: document,
                completion,
                _copy: copy,
            })
        }
    }

    pub(crate) fn take_result(&self) -> Option<bool> {
        self.completion.ivars().take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AnnotationKind, PdfRect};
    use lopdf::{Document, Object, dictionary};

    fn fixture() -> (tempfile::TempDir, DocumentStore, PathBuf, FileRevision) {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("Unicode β – résumé.pdf");
        let mut pdf = Document::with_version("1.7");
        let pages = pdf.new_object_id();
        let page = pdf.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Resources" => dictionary! {},
        });
        pdf.objects.insert(
            pages,
            Object::Dictionary(dictionary! {
                "Type" => "Pages", "Kids" => vec![Object::Reference(page)], "Count" => 1,
            }),
        );
        let catalog = pdf.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages });
        pdf.trailer.set("Root", catalog);
        pdf.save(&source).unwrap();
        let store = DocumentStore::for_print_test(directory.path().join("recovery"));
        let revision = store.capture(&source).unwrap();
        (directory, store, source, revision)
    }

    #[test]
    fn print_copy_includes_unsaved_edits_preserves_source_and_cleans_up() {
        let (_directory, store, source, revision) = fixture();
        let original = std::fs::read(&source).unwrap();
        let annotations = vec![EditorAnnotation {
            page_index: 0,
            rect: PdfRect::new(20.0, 20.0, 180.0, 60.0),
            kind: AnnotationKind::TextBox {
                text: "Unsaved print note".to_owned(),
                font_size: 12.0,
                color_rgb: [0.0; 3],
            },
        }];
        store.journal(&source, &revision, 7, &annotations).unwrap();
        let copy = prepare_copy(&store, &source, &revision, &annotations).unwrap();
        assert_eq!(
            crate::pdf_backend::load_lawpdf_annotations(&copy.path).unwrap(),
            annotations
        );
        assert_eq!(std::fs::read(&source).unwrap(), original);
        assert_eq!(
            store
                .pending_for(&source)
                .unwrap()
                .unwrap()
                .record
                .generation,
            7
        );
        assert_eq!(copy.title, "Unicode β – résumé.pdf");
        let path = copy.path.clone();
        drop(copy);
        assert!(!path.exists());
        assert!(!path.parent().unwrap().exists());
    }

    #[test]
    fn print_uses_the_selected_revision_and_rejects_a_damaged_snapshot() {
        let (_directory, store, source, revision) = fixture();
        std::fs::write(&source, b"changed by another application").unwrap();
        let copy = prepare_copy(&store, &source, &revision, &[]).unwrap();
        assert_eq!(Document::load(&copy.path).unwrap().get_pages().len(), 1);
        assert_eq!(
            std::fs::read(&source).unwrap(),
            b"changed by another application"
        );
        let snapshot = store.print_source(&revision).unwrap();
        std::fs::write(snapshot, b"damaged source snapshot").unwrap();
        assert!(prepare_copy(&store, &source, &revision, &[]).is_err());
    }
}
