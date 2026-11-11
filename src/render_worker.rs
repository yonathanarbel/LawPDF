use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, TrySendError, bounded, unbounded};
use eframe::egui::Context;

use crate::document_store::{DocumentStore, FileRevision};
use crate::model::{
    DocumentComment, EditorAnnotation, ForeignAnnotation, LoadedDocument, PageTextChar,
    RenderedPage,
};
use crate::pdf_backend::{PdfEngine, RenderQuality, SaveReport, rotate_pdf_page_checked};

const OPENING_PREVIEW_MIN_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Debug)]
pub struct OpenedDocument {
    pub document: LoadedDocument,
    pub revision: FileRevision,
    pub annotations: Vec<EditorAnnotation>,
    pub recovery_pending: bool,
    pub timings: OpenTimings,
}

#[derive(Debug, Clone, Copy)]
pub struct OpenTimings {
    pub snapshot: Duration,
    pub metadata: Duration,
    pub annotations: Duration,
    pub revision_and_recovery: Duration,
}

/// UI producers never wait for capacity. Saves and explicit document commands
/// have their own queue so speculative page/text work cannot starve them.
#[derive(Clone)]
pub struct RenderSender {
    urgent: Sender<RenderRequest>,
    background: Sender<RenderRequest>,
    live_documents: Arc<Mutex<Option<HashSet<u64>>>>,
    viewports: Arc<Mutex<HashMap<u64, RenderViewport>>>,
}

#[derive(Debug, Clone, Copy)]
struct RenderViewport {
    first: usize,
    last: usize,
    focus: usize,
}

impl RenderSender {
    /// Cancel speculative work when its tab closes. Writes are deliberately
    /// excluded: a queued save must still report its actual outcome.
    pub fn set_live_documents(&self, epochs: impl IntoIterator<Item = u64>) {
        let epochs: HashSet<_> = epochs.into_iter().collect();
        *self
            .live_documents
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(epochs.clone());
        self.viewports
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .retain(|epoch, _| epochs.contains(epoch));
    }

    pub fn set_viewport(&self, epoch: u64, first: usize, last: usize, focus: usize) {
        self.viewports
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(epoch, RenderViewport { first, last, focus });
    }

    pub fn send(&self, request: RenderRequest) -> Result<(), TrySendError<RenderRequest>> {
        if request_priority(&request) == 0 {
            self.urgent.try_send(request)
        } else {
            self.background.try_send(request)
        }
    }
}

#[cfg(test)]
impl From<Sender<RenderRequest>> for RenderSender {
    fn from(sender: Sender<RenderRequest>) -> Self {
        Self {
            urgent: sender.clone(),
            background: sender,
            live_documents: Arc::new(Mutex::new(None)),
            viewports: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PageRenderKey {
    pub editor_annotations: bool,
    pub document_epoch: u64,
    pub page_index: usize,
    pub render_scale_key: u32,
}

impl PageRenderKey {
    pub fn new(document_epoch: u64, page_index: usize, render_scale: f32) -> Self {
        Self {
            editor_annotations: true,
            document_epoch,
            page_index,
            render_scale_key: float_key(render_scale),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ThumbnailRenderKey {
    pub document_epoch: u64,
    pub page_index: usize,
    pub render_scale_key: u32,
}

impl ThumbnailRenderKey {
    pub fn new(document_epoch: u64, page_index: usize, render_scale: f32) -> Self {
        Self {
            document_epoch,
            page_index,
            render_scale_key: float_key(render_scale),
        }
    }
}

#[derive(Debug)]
pub enum RenderRequest {
    PrepareSources {
        paths: Vec<PathBuf>,
        defer_background: bool,
    },
    OpenDocument {
        path: PathBuf,
        optimize_large_documents: bool,
        preview_page: Option<usize>,
    },
    LoadDocument {
        path: PathBuf,
        optimize_large_documents: bool,
        reply: Sender<Result<LoadedDocument, String>>,
    },
    EnrichDocument {
        document_epoch: u64,
        path: PathBuf,
    },
    DocumentComments {
        document_epoch: u64,
        path: PathBuf,
        expected_revision: FileRevision,
    },
    ForeignAnnotations {
        document_epoch: u64,
        path: PathBuf,
        expected_revision: FileRevision,
    },
    TextCharsAsync {
        document_epoch: u64,
        path: PathBuf,
        page_index: usize,
    },
    TextPageAsync {
        document_epoch: u64,
        path: PathBuf,
        page_index: usize,
    },
    Page {
        key: PageRenderKey,
        path: PathBuf,
        zoom: f32,
        render_scale: f32,
        fast: bool,
    },
    PageImmediate {
        path: PathBuf,
        page_index: usize,
        render_scale: f32,
        fast: bool,
        reply: Sender<Result<RenderedPage, String>>,
    },
    Thumbnail {
        key: ThumbnailRenderKey,
        path: PathBuf,
        render_scale: f32,
    },
    ExportPagePng {
        path: PathBuf,
        page_index: usize,
        destination: PathBuf,
        scale: f32,
        reply: Sender<Result<(), String>>,
    },
    SaveAnnotations {
        source: PathBuf,
        destination: PathBuf,
        annotations: Vec<EditorAnnotation>,
        reply: Sender<Result<SaveReport, String>>,
    },
    AutosaveAnnotations {
        document_epoch: u64,
        path: PathBuf,
        generation: u64,
        annotations: Vec<EditorAnnotation>,
        expected_revision: FileRevision,
    },
    RotatePage {
        document_epoch: u64,
        path: PathBuf,
        page_index: usize,
        clockwise: bool,
        expected_revision: FileRevision,
    },
}

/// A validated raster converted into egui's pixel format off the UI thread.
#[derive(Debug)]
pub struct PreparedPage {
    pub page_index: usize,
    pub width: usize,
    pub height: usize,
    pub image: egui::ColorImage,
}

impl TryFrom<RenderedPage> for PreparedPage {
    type Error = String;
    fn try_from(page: RenderedPage) -> Result<Self, Self::Error> {
        if page.width == 0
            || page.height == 0
            || page
                .width
                .checked_mul(page.height)
                .and_then(|pixels| pixels.checked_mul(4))
                != Some(page.rgba.len())
        {
            return Err("The PDF worker returned an invalid page image.".to_owned());
        }
        let image = egui::ColorImage::from_rgba_unmultiplied([page.width, page.height], &page.rgba);
        Ok(Self {
            page_index: page.page_index,
            width: page.width,
            height: page.height,
            image,
        })
    }
}

#[derive(Debug)]
pub enum RenderEvent {
    WorkerStopped,
    PageSkipped {
        key: PageRenderKey,
    },
    SourcesPrepared {
        paths: Vec<PathBuf>,
        converted: usize,
        errors: Vec<String>,
        defer_background: bool,
    },
    DocumentOpened {
        path: PathBuf,
        result: Result<OpenedDocument, String>,
    },
    OpeningPreview {
        path: PathBuf,
        page: PreparedPage,
    },
    DocumentEnriched {
        document_epoch: u64,
        path: PathBuf,
        result: Result<LoadedDocument, String>,
    },
    DocumentComments {
        document_epoch: u64,
        path: PathBuf,
        revision: FileRevision,
        result: Result<Vec<DocumentComment>, String>,
    },
    ForeignAnnotations {
        document_epoch: u64,
        path: PathBuf,
        revision: FileRevision,
        result: Result<Vec<ForeignAnnotation>, String>,
    },
    Page {
        key: PageRenderKey,
        path: PathBuf,
        _zoom: f32,
        render_scale: f32,
        result: Result<PreparedPage, String>,
    },
    Thumbnail {
        key: ThumbnailRenderKey,
        path: PathBuf,
        result: Result<PreparedPage, String>,
    },
    TextChars {
        document_epoch: u64,
        path: PathBuf,
        page_index: usize,
        result: Result<Vec<PageTextChar>, String>,
    },
    TextPage {
        document_epoch: u64,
        path: PathBuf,
        page_index: usize,
        result: Result<String, String>,
    },
    AnnotationsSaved {
        document_epoch: u64,
        path: PathBuf,
        generation: u64,
        result: Result<SaveReport, String>,
    },
    PageRotated {
        document_epoch: u64,
        path: PathBuf,
        page_index: usize,
        result: Result<(LoadedDocument, i64, FileRevision, Vec<EditorAnnotation>), String>,
    },
}

pub fn save_annotations(
    worker: &RenderSender,
    source: &std::path::Path,
    destination: &std::path::Path,
    annotations: &[EditorAnnotation],
) -> Result<SaveReport, String> {
    let (reply, result) = unbounded();
    worker
        .send(RenderRequest::SaveAnnotations {
            source: source.to_path_buf(),
            destination: destination.to_path_buf(),
            annotations: annotations.to_vec(),
            reply,
        })
        .map_err(|error| format!("PDF worker is not available: {error}"))?;
    // A timeout could report failure while a queued save still modifies the file.
    result
        .recv()
        .map_err(|error| format!("PDF worker stopped before confirming the save: {error}"))?
}

fn supervise_render_worker(
    work: impl FnOnce(),
    event_tx: Sender<RenderEvent>,
    repaint: Option<Context>,
) {
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)).is_err() {
        let _ = event_tx.send(RenderEvent::WorkerStopped);
        if let Some(ctx) = repaint {
            ctx.request_repaint();
        }
    }
}

pub fn spawn_render_worker(
    repaint_context: Option<Context>,
) -> (RenderSender, Receiver<RenderEvent>) {
    let (urgent_tx, urgent_rx) = bounded(64);
    let (request_tx, request_rx) = bounded(128);
    let (event_tx, event_rx) = bounded(8);
    let live_documents = Arc::new(Mutex::new(None::<HashSet<u64>>));
    let worker_documents = live_documents.clone();
    let viewports = Arc::new(Mutex::new(HashMap::new()));
    let worker_viewports = viewports.clone();
    let panic_sender = event_tx.clone();
    let panic_context = repaint_context.clone();

    thread::spawn(move || {
        supervise_render_worker(
            || {
                let mut engine = None;
                let mut backlog = VecDeque::new();
                loop {
                    let Some(request) = next_worker_request(
                        &urgent_rx,
                        &request_rx,
                        &mut backlog,
                        &worker_viewports,
                    ) else {
                        break;
                    };
                    let epoch = cancellable_epoch(&request);
                    let cancelled = || {
                        epoch.is_some_and(|epoch| {
                            worker_documents
                                .lock()
                                .unwrap_or_else(|error| error.into_inner())
                                .as_ref()
                                .is_some_and(|live| !live.contains(&epoch))
                        })
                    };
                    if cancelled() {
                        continue;
                    }
                    if let RenderRequest::Page { key, .. } = &request
                        && worker_viewports
                            .lock()
                            .unwrap_or_else(|error| error.into_inner())
                            .get(&key.document_epoch)
                            .is_some_and(|viewport| {
                                key.page_index < viewport.first || key.page_index > viewport.last
                            })
                    {
                        if event_tx
                            .send(RenderEvent::PageSkipped { key: *key })
                            .is_ok()
                            && let Some(ctx) = &repaint_context
                        {
                            ctx.request_repaint();
                        }
                        continue;
                    }
                    if engine.is_none() {
                        match PdfEngine::new() {
                            Ok(ready) => engine = Some(ready),
                            Err(error) => {
                                let _ = event_tx.send(error_event(request, error.to_string()));
                                if let Some(ctx) = &repaint_context {
                                    ctx.request_repaint();
                                }
                                continue;
                            }
                        }
                    }
                    let engine = engine.as_ref().expect("initialized PDF worker");
                    let event = match request {
                        RenderRequest::PrepareSources {
                            paths,
                            defer_background,
                        } => {
                            let (paths, converted, errors) = crate::app::prepare_open_paths(paths);
                            RenderEvent::SourcesPrepared {
                                paths,
                                converted,
                                errors,
                                defer_background,
                            }
                        }
                        RenderRequest::OpenDocument {
                            path,
                            optimize_large_documents,
                            preview_page,
                        } => {
                            engine.close_document(&path);
                            if let Some(preview_page) = preview_page
                                && optimize_large_documents
                                && std::fs::metadata(&path).is_ok_and(|metadata| {
                                    metadata.is_file()
                                        && metadata.len() >= OPENING_PREVIEW_MIN_BYTES
                                })
                            {
                                let preview_path = path.clone();
                                let sender = event_tx.clone();
                                let context = repaint_context.clone();
                                // This preview is never editable: the normal open waits for
                                // snapshot and revision verification before creating a tab.
                                let _ = thread::Builder::new()
                                    .name("lawpdf-opening-preview".to_owned())
                                    .spawn(move || {
                                        let result = (|| {
                                            let engine = PdfEngine::new()?;
                                            engine
                                                .render_opening_preview(&preview_path, preview_page)
                                                .or_else(|error| {
                                                    if preview_page == 0 {
                                                        Err(error)
                                                    } else {
                                                        engine.render_opening_preview(
                                                            &preview_path,
                                                            0,
                                                        )
                                                    }
                                                })
                                        })();
                                        if let Ok(page) = result
                                            && let Ok(page) = PreparedPage::try_from(page)
                                            && sender
                                                .send(RenderEvent::OpeningPreview {
                                                    path: preview_path,
                                                    page,
                                                })
                                                .is_ok()
                                            && let Some(ctx) = context
                                        {
                                            ctx.request_repaint();
                                        }
                                    });
                            }
                            let result = (|| {
                                let started = Instant::now();
                                let revision = DocumentStore::new()?.capture(&path)?;
                                let snapshot = started.elapsed();
                                let started = Instant::now();
                                let document = engine
                                    .load_document_adaptive(&path, optimize_large_documents)
                                    .map_err(|error| format!("{error:#}"))?;
                                let metadata = started.elapsed();
                                let started = Instant::now();
                                let annotations = crate::pdf_backend::load_lawpdf_annotations(
                                    &path,
                                )
                                .map_err(|error| {
                                    format!("Could not read PDF annotations: {error:#}")
                                })?;
                                let annotation_time = started.elapsed();
                                let started = Instant::now();
                                revision.require_current(&path)?;
                                let recovery_pending =
                                    DocumentStore::new()?.pending_for(&path)?.is_some();
                                let revision_and_recovery = started.elapsed();
                                crate::diagnostics::document_open_phases(
                                    snapshot,
                                    metadata,
                                    annotation_time,
                                    revision_and_recovery,
                                );
                                Ok(OpenedDocument {
                                    document,
                                    revision,
                                    annotations,
                                    recovery_pending,
                                    timings: OpenTimings {
                                        snapshot,
                                        metadata,
                                        annotations: annotation_time,
                                        revision_and_recovery,
                                    },
                                })
                            })();
                            RenderEvent::DocumentOpened { path, result }
                        }
                        RenderRequest::LoadDocument {
                            path,
                            optimize_large_documents,
                            reply,
                        } => {
                            let _ = reply.send(
                                engine
                                    .load_document_adaptive(&path, optimize_large_documents)
                                    .map_err(|error| error.to_string()),
                            );
                            continue;
                        }
                        RenderRequest::EnrichDocument {
                            document_epoch,
                            path,
                        } => RenderEvent::DocumentEnriched {
                            document_epoch,
                            path: path.clone(),
                            result: engine
                                .load_document(&path)
                                .map_err(|error| error.to_string()),
                        },
                        RenderRequest::DocumentComments {
                            document_epoch,
                            path,
                            expected_revision,
                        } => {
                            // Comment extraction is opt-in and does not occupy the
                            // native rendering thread while a long PDF is parsed.
                            let sender = event_tx.clone();
                            let context = repaint_context.clone();
                            let source = path.clone();
                            let revision = expected_revision.clone();
                            let spawned = thread::Builder::new()
                                .name("lawpdf-comments".to_owned())
                                .spawn(move || {
                                    let result =
                                        crate::pdf_backend::load_document_comments_checked(
                                            &source, &revision,
                                        )
                                        .map_err(|error| format!("{error:#}"));
                                    let _ = sender.send(RenderEvent::DocumentComments {
                                        document_epoch,
                                        path: source,
                                        revision,
                                        result,
                                    });
                                    if let Some(context) = context {
                                        context.request_repaint();
                                    }
                                });
                            match spawned {
                                Ok(_) => continue,
                                Err(error) => RenderEvent::DocumentComments {
                                    document_epoch,
                                    path,
                                    revision: expected_revision,
                                    result: Err(format!(
                                        "Could not start PDF comment loading: {error}"
                                    )),
                                },
                            }
                        }
                        RenderRequest::ForeignAnnotations {
                            document_epoch,
                            path,
                            expected_revision,
                        } => {
                            let sender = event_tx.clone();
                            let context = repaint_context.clone();
                            let source = path.clone();
                            let revision = expected_revision.clone();
                            let spawned = thread::Builder::new()
                                .name("lawpdf-foreign-annotations".to_owned())
                                .spawn(move || {
                                    let result = std::panic::catch_unwind(|| {
                                        crate::pdf_backend::load_foreign_annotations_checked(
                                            &source, &revision,
                                        )
                                    })
                                    .map_err(|_| {
                                        "The annotation reader stopped unexpectedly.".to_owned()
                                    })
                                    .and_then(|result| {
                                        result.map_err(|error| format!("{error:#}"))
                                    });
                                    let _ = sender.send(RenderEvent::ForeignAnnotations {
                                        document_epoch,
                                        path: source,
                                        revision,
                                        result,
                                    });
                                    if let Some(context) = context {
                                        context.request_repaint();
                                    }
                                });
                            match spawned {
                                Ok(_) => continue,
                                Err(error) => RenderEvent::ForeignAnnotations {
                                    document_epoch,
                                    path,
                                    revision: expected_revision,
                                    result: Err(format!(
                                        "Could not start annotation loading: {error}"
                                    )),
                                },
                            }
                        }
                        RenderRequest::TextCharsAsync {
                            document_epoch,
                            path,
                            page_index,
                        } => RenderEvent::TextChars {
                            document_epoch,
                            path: path.clone(),
                            page_index,
                            result: engine
                                .load_page_text_chars(&path, page_index)
                                .map_err(|error| error.to_string()),
                        },
                        RenderRequest::TextPageAsync {
                            document_epoch,
                            path,
                            page_index,
                        } => RenderEvent::TextPage {
                            document_epoch,
                            path: path.clone(),
                            page_index,
                            result: engine
                                .load_page_text(&path, page_index)
                                .map_err(|error| error.to_string()),
                        },
                        RenderRequest::Page {
                            key,
                            path,
                            zoom,
                            render_scale,
                            fast,
                        } => RenderEvent::Page {
                            key,
                            path: path.clone(),
                            _zoom: zoom,
                            render_scale,
                            result: engine
                                .render_page_interactive(
                                    &path,
                                    key.page_index,
                                    render_scale,
                                    if fast {
                                        RenderQuality::Fast
                                    } else {
                                        RenderQuality::Crisp
                                    },
                                    key.editor_annotations,
                                    {
                                        let viewports = worker_viewports
                                            .lock()
                                            .unwrap_or_else(|error| error.into_inner());
                                        viewports
                                            .get(&key.document_epoch)
                                            .is_none_or(|viewport| viewport.focus == key.page_index)
                                    },
                                )
                                .map_err(|error| error.to_string())
                                .and_then(PreparedPage::try_from),
                        },
                        RenderRequest::PageImmediate {
                            path,
                            page_index,
                            render_scale,
                            fast,
                            reply,
                        } => {
                            let _ = reply.send(
                                engine
                                    .render_page_interactive(
                                        &path,
                                        page_index,
                                        render_scale,
                                        if fast {
                                            RenderQuality::Fast
                                        } else {
                                            RenderQuality::Crisp
                                        },
                                        true,
                                        true,
                                    )
                                    .map_err(|error| error.to_string()),
                            );
                            continue;
                        }
                        RenderRequest::Thumbnail {
                            key,
                            path,
                            render_scale,
                        } => RenderEvent::Thumbnail {
                            key,
                            path: path.clone(),
                            result: engine
                                .render_page_internal(
                                    &path,
                                    key.page_index,
                                    render_scale,
                                    RenderQuality::Fast,
                                    false,
                                )
                                .map_err(|error| error.to_string())
                                .and_then(PreparedPage::try_from),
                        },
                        RenderRequest::ExportPagePng {
                            path,
                            page_index,
                            destination,
                            scale,
                            reply,
                        } => {
                            let _ = reply.send(
                                engine
                                    .export_page_png(&path, page_index, &destination, scale)
                                    .map_err(|error| error.to_string()),
                            );
                            continue;
                        }
                        RenderRequest::SaveAnnotations {
                            source,
                            destination,
                            annotations,
                            reply,
                        } => {
                            engine.close_document(&source);
                            engine.close_document(&destination);
                            let result = crate::pdf_backend::save_with_annotations(
                                &source,
                                &destination,
                                &annotations,
                            )
                            .map_err(|error| format!("{error:#}"));
                            let _ = reply.send(result);
                            continue;
                        }
                        RenderRequest::AutosaveAnnotations {
                            document_epoch,
                            path,
                            generation,
                            annotations,
                            expected_revision,
                        } => {
                            engine.close_document(&path);
                            let result = crate::pdf_backend::save_with_annotations_checked(
                                &path,
                                &path,
                                &annotations,
                                &expected_revision,
                            )
                            .map_err(|error| format!("{error:#}"))
                            .map(|mut report| {
                                if let Some(revision) = &report.revision {
                                    report.recovery_warning = DocumentStore::new()
                                        .and_then(|store| {
                                            store
                                                .acknowledge_saved_file(&path, generation, revision)
                                        })
                                        .err();
                                }
                                report
                            });
                            RenderEvent::AnnotationsSaved {
                                document_epoch,
                                path: path.clone(),
                                generation,
                                result,
                            }
                        }
                        RenderRequest::RotatePage {
                            document_epoch,
                            path,
                            page_index,
                            clockwise,
                            expected_revision,
                        } => {
                            engine.close_document(&path);
                            let result = rotate_pdf_page_checked(
                                &path,
                                page_index,
                                clockwise,
                                &expected_revision,
                            )
                            .and_then(|(rotation, revision)| {
                                let annotations =
                                    crate::pdf_backend::load_lawpdf_annotations(&path)?;
                                engine
                                    .load_document_adaptive(&path, true)
                                    .map(|document| (document, rotation, revision, annotations))
                            })
                            .map_err(|error| error.to_string());
                            RenderEvent::PageRotated {
                                document_epoch,
                                path: path.clone(),
                                page_index,
                                result,
                            }
                        }
                    };

                    if cancelled() {
                        continue;
                    }
                    if event_tx.send(event).is_ok() {
                        if let Some(ctx) = &repaint_context {
                            ctx.request_repaint();
                        }
                    }
                }
            },
            panic_sender,
            panic_context,
        )
    });

    (
        RenderSender {
            urgent: urgent_tx,
            background: request_tx,
            live_documents,
            viewports,
        },
        event_rx,
    )
}

fn cancellable_epoch(request: &RenderRequest) -> Option<u64> {
    match request {
        RenderRequest::Page { key, .. } => Some(key.document_epoch),
        RenderRequest::Thumbnail { key, .. } => Some(key.document_epoch),
        RenderRequest::EnrichDocument { document_epoch, .. }
        | RenderRequest::DocumentComments { document_epoch, .. }
        | RenderRequest::ForeignAnnotations { document_epoch, .. }
        | RenderRequest::TextCharsAsync { document_epoch, .. }
        | RenderRequest::TextPageAsync { document_epoch, .. } => Some(*document_epoch),
        _ => None,
    }
}

fn next_worker_request(
    urgent: &Receiver<RenderRequest>,
    background: &Receiver<RenderRequest>,
    backlog: &mut VecDeque<RenderRequest>,
    viewports: &Mutex<HashMap<u64, RenderViewport>>,
) -> Option<RenderRequest> {
    if let Ok(request) = urgent.try_recv() {
        return Some(request);
    }
    if backlog.is_empty() && background.is_empty() {
        crossbeam_channel::select_biased! {
            recv(urgent) -> request => return request.ok(),
            recv(background) -> request => backlog.push_back(request.ok()?),
        }
    }
    let viewports = viewports.lock().unwrap_or_else(|error| error.into_inner());
    next_prioritized_request(background, backlog, &viewports)
}

fn next_prioritized_request(
    request_rx: &Receiver<RenderRequest>,
    backlog: &mut VecDeque<RenderRequest>,
    viewports: &HashMap<u64, RenderViewport>,
) -> Option<RenderRequest> {
    if backlog.is_empty() {
        backlog.push_back(request_rx.recv().ok()?);
    }
    while backlog.len() < 128
        && let Ok(request) = request_rx.try_recv()
    {
        push_coalesced(backlog, request);
    }
    let best = backlog
        .iter()
        .enumerate()
        .min_by_key(|(index, request)| {
            let distance = match request {
                RenderRequest::Page { key, .. } => viewports
                    .get(&key.document_epoch)
                    .map_or(0, |viewport| key.page_index.abs_diff(viewport.focus)),
                _ => 0,
            };
            (request_priority(request), distance, *index)
        })
        .map(|(index, _)| index)?;
    backlog.remove(best)
}

fn push_coalesced(backlog: &mut VecDeque<RenderRequest>, request: RenderRequest) {
    if let Some(target) = coalescing_target(&request)
        && let Some(position) = backlog
            .iter()
            .position(|pending| coalescing_target(pending).as_ref() == Some(&target))
    {
        backlog.remove(position);
    }
    backlog.push_back(request);
}

fn request_priority(request: &RenderRequest) -> u8 {
    match request {
        RenderRequest::PrepareSources { .. }
        | RenderRequest::OpenDocument { .. }
        | RenderRequest::LoadDocument { .. }
        | RenderRequest::PageImmediate { .. }
        | RenderRequest::ExportPagePng { .. }
        | RenderRequest::SaveAnnotations { .. }
        | RenderRequest::AutosaveAnnotations { .. }
        | RenderRequest::RotatePage { .. } => 0,
        RenderRequest::Page { .. } => 1,
        RenderRequest::EnrichDocument { .. } => 2,
        RenderRequest::DocumentComments { .. } => 2,
        RenderRequest::ForeignAnnotations { .. } => 2,
        RenderRequest::TextCharsAsync { .. } => 3,
        RenderRequest::Thumbnail { .. } => 4,
        RenderRequest::TextPageAsync { .. } => 5,
    }
}

#[cfg(test)]
fn coalesce_render_request(
    mut request: RenderRequest,
    request_rx: &Receiver<RenderRequest>,
    backlog: &mut VecDeque<RenderRequest>,
) -> RenderRequest {
    let Some((document_epoch, page_index, path, thumbnail)) = coalescing_target(&request) else {
        return request;
    };

    while let Ok(next) = request_rx.try_recv() {
        let replaces_current = coalescing_target(&next).is_some_and(
            |(next_epoch, next_page, next_path, next_thumbnail)| {
                next_epoch == document_epoch
                    && next_page == page_index
                    && next_path == path
                    && next_thumbnail == thumbnail
            },
        );

        if replaces_current {
            request = next;
        } else {
            backlog.push_back(next);
        }
    }

    request
}

fn coalescing_target(request: &RenderRequest) -> Option<(u64, usize, PathBuf, bool)> {
    match request {
        RenderRequest::Page { key, path, .. } => {
            Some((key.document_epoch, key.page_index, path.clone(), false))
        }
        RenderRequest::Thumbnail { key, path, .. } => {
            Some((key.document_epoch, key.page_index, path.clone(), true))
        }
        _ => None,
    }
}

fn error_event(request: RenderRequest, message: String) -> RenderEvent {
    match request {
        RenderRequest::PrepareSources {
            defer_background, ..
        } => RenderEvent::SourcesPrepared {
            paths: Vec::new(),
            converted: 0,
            errors: vec![message],
            defer_background,
        },
        RenderRequest::OpenDocument { path, .. } => RenderEvent::DocumentOpened {
            path,
            result: Err(message),
        },
        RenderRequest::LoadDocument { reply, .. } => {
            let _ = reply.send(Err(message));
            RenderEvent::Thumbnail {
                key: ThumbnailRenderKey::new(0, 0, 0.0),
                path: PathBuf::new(),
                result: Err("PDF worker failed before loading document".to_owned()),
            }
        }
        RenderRequest::EnrichDocument {
            document_epoch,
            path,
        } => RenderEvent::DocumentEnriched {
            document_epoch,
            path,
            result: Err(message),
        },
        RenderRequest::DocumentComments {
            document_epoch,
            path,
            expected_revision,
        } => RenderEvent::DocumentComments {
            document_epoch,
            path,
            revision: expected_revision,
            result: Err(message),
        },
        RenderRequest::ForeignAnnotations {
            document_epoch,
            path,
            expected_revision,
        } => RenderEvent::ForeignAnnotations {
            document_epoch,
            path,
            revision: expected_revision,
            result: Err(message),
        },
        RenderRequest::TextCharsAsync {
            document_epoch,
            path,
            page_index,
        } => RenderEvent::TextChars {
            document_epoch,
            path,
            page_index,
            result: Err(message),
        },
        RenderRequest::TextPageAsync {
            document_epoch,
            path,
            page_index,
        } => RenderEvent::TextPage {
            document_epoch,
            path,
            page_index,
            result: Err(message),
        },
        RenderRequest::Page {
            key,
            path,
            zoom,
            render_scale,
            ..
        } => RenderEvent::Page {
            key,
            path,
            _zoom: zoom,
            render_scale,
            result: Err(message),
        },
        RenderRequest::PageImmediate { reply, .. } => {
            let _ = reply.send(Err(message));
            RenderEvent::Thumbnail {
                key: ThumbnailRenderKey::new(0, 0, 0.0),
                path: PathBuf::new(),
                result: Err("PDF worker failed before immediate page render".to_owned()),
            }
        }
        RenderRequest::Thumbnail { key, path, .. } => RenderEvent::Thumbnail {
            key,
            path,
            result: Err(message),
        },
        RenderRequest::SaveAnnotations { reply, .. } => {
            let _ = reply.send(Err(message));
            RenderEvent::Thumbnail {
                key: ThumbnailRenderKey::new(0, 0, 0.0),
                path: PathBuf::new(),
                result: Err("PDF worker failed before saving annotations".to_owned()),
            }
        }
        RenderRequest::ExportPagePng { reply, .. } => {
            let _ = reply.send(Err(message));
            RenderEvent::Thumbnail {
                key: ThumbnailRenderKey::new(0, 0, 0.0),
                path: PathBuf::new(),
                result: Err("PDF worker failed before exporting PNG".to_owned()),
            }
        }
        RenderRequest::AutosaveAnnotations {
            document_epoch,
            path,
            generation,
            ..
        } => RenderEvent::AnnotationsSaved {
            document_epoch,
            path,
            generation,
            result: Err(message),
        },
        RenderRequest::RotatePage {
            document_epoch,
            path,
            page_index,
            ..
        } => RenderEvent::PageRotated {
            document_epoch,
            path,
            page_index,
            result: Err(message),
        },
    }
}

fn float_key(value: f32) -> u32 {
    if value.is_finite() {
        (value.max(0.0) * 1000.0).round() as u32
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use lopdf::{Document, Object, dictionary};

    #[test]
    fn worker_panic_notifies_and_wakes_the_ui() {
        let ctx = Context::default();
        let wakes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = Arc::clone(&wakes);
        ctx.set_request_repaint_callback(move |_| {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        });
        let (sender, receiver) = unbounded();
        supervise_render_worker(|| panic!("simulated worker failure"), sender, Some(ctx));
        assert!(matches!(
            receiver.try_recv(),
            Ok(RenderEvent::WorkerStopped)
        ));
        assert!(wakes.load(std::sync::atomic::Ordering::SeqCst) > 0);
    }

    #[test]
    fn prepared_page_preserves_alpha_and_rejects_invalid_dimensions() {
        let rgba = vec![255, 100, 20, 255, 128, 64, 32, 128];
        let expected = egui::ColorImage::from_rgba_unmultiplied([2, 1], &rgba);
        let page = PreparedPage::try_from(RenderedPage {
            page_index: 4,
            width: 2,
            height: 1,
            rgba,
        })
        .unwrap();
        assert_eq!(page.page_index, 4);
        assert_eq!(page.image.size, expected.size);
        assert_eq!(page.image.pixels, expected.pixels);
        for (width, height, rgba) in [(0, 1, vec![]), (2, 2, vec![0; 3]), (usize::MAX, 2, vec![])] {
            assert!(
                PreparedPage::try_from(RenderedPage {
                    page_index: 0,
                    width,
                    height,
                    rgba
                })
                .is_err()
            );
        }
    }

    #[test]
    fn render_worker_returns_prepared_pixels_for_a_real_pdf_without_changing_it() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("sample.pdf");
        write_blank_pdf(&path);
        let before = fs::read(&path).unwrap();
        let (sender, events) = spawn_render_worker(None);
        let key = PageRenderKey::new(7, 0, 1.0);
        sender
            .send(RenderRequest::Page {
                key,
                path: path.clone(),
                zoom: 1.0,
                render_scale: 1.0,
                fast: false,
            })
            .unwrap();
        match events.recv_timeout(Duration::from_secs(15)).unwrap() {
            RenderEvent::Page {
                key: actual,
                result: Ok(page),
                ..
            } => {
                assert_eq!(actual, key);
                assert!(page.width > 0 && page.height > 0);
                assert_eq!(page.image.size, [page.width, page.height]);
                assert_eq!(page.image.pixels.len(), page.width * page.height);
            }
            other => panic!("unexpected render event: {other:?}"),
        }
        assert_eq!(fs::read(path).unwrap(), before);
    }

    fn write_blank_pdf(path: &Path) {
        let mut document = Document::with_version("1.5");
        let catalog_id = document.new_object_id();
        let pages_id = document.new_object_id();
        let page_id = document.new_object_id();

        document.objects.insert(
            catalog_id,
            Object::Dictionary(dictionary! {
                "Type" => Object::Name(b"Catalog".to_vec()),
                "Pages" => Object::Reference(pages_id),
            }),
        );
        document.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => Object::Name(b"Pages".to_vec()),
                "Kids" => Object::Array(vec![Object::Reference(page_id)]),
                "Count" => Object::Integer(1),
            }),
        );
        document.objects.insert(
            page_id,
            Object::Dictionary(dictionary! {
                "Type" => Object::Name(b"Page".to_vec()),
                "Parent" => Object::Reference(pages_id),
                "MediaBox" => Object::Array(vec![0.into(), 0.into(), 612.into(), 792.into()]),
                "Resources" => Object::Dictionary(dictionary! {}),
            }),
        );
        document.trailer.set("Root", Object::Reference(catalog_id));
        document.save(path).unwrap();
    }

    fn page_request(page_index: usize, render_scale: f32) -> RenderRequest {
        RenderRequest::Page {
            key: PageRenderKey::new(7, page_index, render_scale),
            path: PathBuf::from("document.pdf"),
            zoom: 1.0,
            render_scale,
            fast: false,
        }
    }

    #[test]
    fn coalescing_supersedes_same_page_and_keeps_distinct_pages() {
        let (tx, rx) = unbounded();
        tx.send(page_request(0, 2.0)).unwrap();
        tx.send(page_request(1, 1.5)).unwrap();
        tx.send(page_request(0, 3.0)).unwrap();
        let mut backlog = VecDeque::new();

        let current = coalesce_render_request(page_request(0, 1.0), &rx, &mut backlog);

        match current {
            RenderRequest::Page {
                key, render_scale, ..
            } => {
                assert_eq!(key.page_index, 0);
                assert_eq!(render_scale, 3.0);
            }
            other => panic!("expected page render, got {other:?}"),
        }
        assert_eq!(backlog.len(), 1);
        assert!(matches!(
            backlog.pop_front(),
            Some(RenderRequest::Page {
                key: PageRenderKey { page_index: 1, .. },
                render_scale: 1.5,
                ..
            })
        ));
    }

    #[test]
    fn coalescing_keeps_thumbnail_and_page_requests_distinct() {
        let (tx, rx) = unbounded();
        tx.send(RenderRequest::Thumbnail {
            key: ThumbnailRenderKey::new(7, 0, 0.25),
            path: PathBuf::from("document.pdf"),
            render_scale: 0.25,
        })
        .unwrap();
        let mut backlog = VecDeque::new();

        let current = coalesce_render_request(page_request(0, 1.0), &rx, &mut backlog);

        assert!(matches!(current, RenderRequest::Page { .. }));
        assert!(matches!(
            backlog.pop_front(),
            Some(RenderRequest::Thumbnail { .. })
        ));
    }

    #[test]
    fn visible_page_render_overtakes_full_document_enrichment() {
        let (tx, rx) = unbounded();
        tx.send(RenderRequest::EnrichDocument {
            document_epoch: 7,
            path: PathBuf::from("document.pdf"),
        })
        .unwrap();
        tx.send(page_request(2, 1.0)).unwrap();
        let mut backlog = VecDeque::new();

        let next = next_prioritized_request(&rx, &mut backlog, &HashMap::new()).unwrap();

        assert!(matches!(
            next,
            RenderRequest::Page {
                key: PageRenderKey { page_index: 2, .. },
                ..
            }
        ));
        assert!(matches!(
            backlog.pop_front(),
            Some(RenderRequest::EnrichDocument {
                document_epoch: 7,
                ..
            })
        ));
    }

    #[test]
    fn visible_page_render_overtakes_queued_search_extraction() {
        let (tx, rx) = unbounded();
        tx.send(RenderRequest::TextPageAsync {
            document_epoch: 7,
            path: PathBuf::from("document.pdf"),
            page_index: 100,
        })
        .unwrap();
        tx.send(page_request(3, 1.0)).unwrap();
        let mut backlog = VecDeque::new();

        let next = next_prioritized_request(&rx, &mut backlog, &HashMap::new()).unwrap();

        assert!(matches!(
            next,
            RenderRequest::Page {
                key: PageRenderKey { page_index: 3, .. },
                ..
            }
        ));
        assert!(matches!(
            backlog.pop_front(),
            Some(RenderRequest::TextPageAsync {
                page_index: 100,
                ..
            })
        ));
    }

    #[test]
    fn page_requests_are_ordered_by_the_current_viewport() {
        let (tx, rx) = unbounded();
        tx.send(page_request(10, 1.0)).unwrap();
        tx.send(page_request(99, 1.0)).unwrap();
        tx.send(page_request(100, 1.0)).unwrap();
        let viewports = HashMap::from([(
            7,
            RenderViewport {
                first: 97,
                last: 103,
                focus: 100,
            },
        )]);
        let mut backlog = VecDeque::new();
        let next = next_prioritized_request(&rx, &mut backlog, &viewports).unwrap();
        assert!(matches!(
            next,
            RenderRequest::Page {
                key: PageRenderKey {
                    page_index: 100,
                    ..
                },
                ..
            }
        ));
    }

    #[test]
    fn obsolete_page_requests_are_reported_without_rendering() {
        let (sender, events) = spawn_render_worker(None);
        sender.set_viewport(7, 97, 103, 100);
        sender.send(page_request(10, 1.0)).unwrap();
        assert!(matches!(
            events
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap(),
            RenderEvent::PageSkipped {
                key: PageRenderKey { page_index: 10, .. }
            }
        ));
    }

    #[test]
    fn annotation_saves_replace_pdfs_after_pdfium_has_cached_both_paths() {
        use crate::model::{AnnotationKind, MarkerStyle, PdfRect};
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("lawpdf-worker-save-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let source = dir.join("source.pdf");
        let destination = dir.join("destination.pdf");
        write_blank_pdf(&source);
        write_blank_pdf(&destination);
        let (worker, _events) = spawn_render_worker(None);
        let render = |path: &Path| {
            let (reply, result) = unbounded();
            worker
                .send(RenderRequest::PageImmediate {
                    path: path.to_path_buf(),
                    page_index: 0,
                    render_scale: 0.5,
                    fast: false,
                    reply,
                })
                .unwrap();
            result
                .recv_timeout(Duration::from_secs(10))
                .unwrap()
                .unwrap()
        };
        render(&source);
        render(&destination);
        let annotations = [EditorAnnotation {
            page_index: 0,
            rect: PdfRect::new(72.0, 650.0, 220.0, 668.0),
            kind: AnnotationKind::Marker {
                color_rgb: [1.0, 0.93, 0.45],
                opacity: 0.42,
                style: MarkerStyle::Highlight,
            },
        }];
        save_annotations(&worker, &source, &source, &annotations).unwrap();
        assert_eq!(
            crate::pdf_backend::load_lawpdf_annotations(&source)
                .unwrap()
                .len(),
            1
        );
        render(&source);
        save_annotations(&worker, &source, &destination, &annotations).unwrap();
        assert_eq!(
            crate::pdf_backend::load_lawpdf_annotations(&destination)
                .unwrap()
                .len(),
            1
        );
        render(&destination);
        // Save again to release the reopened PDFium handle before cleanup.
        save_annotations(&worker, &destination, &destination, &annotations).unwrap();
        drop(worker);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn enrichment_loads_full_layout_after_fast_open() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "lawpdf-worker-enrichment-{}-{nonce}.pdf",
            std::process::id()
        ));
        write_blank_pdf(&path);

        let (request_tx, event_rx) = spawn_render_worker(None);
        let (reply_tx, reply_rx) = unbounded();
        request_tx
            .send(RenderRequest::LoadDocument {
                path: path.clone(),
                optimize_large_documents: true,
                reply: reply_tx,
            })
            .unwrap();
        let opened = reply_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("worker did not reply to fast open")
            .expect("fast open failed");
        assert!(opened.optimized);

        request_tx
            .send(RenderRequest::EnrichDocument {
                document_epoch: 42,
                path: path.clone(),
            })
            .unwrap();
        let event = event_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("worker did not finish enrichment");
        match event {
            RenderEvent::DocumentEnriched {
                document_epoch,
                path: event_path,
                result,
            } => {
                assert_eq!(document_epoch, 42);
                assert_eq!(event_path, path);
                let enriched = result.expect("full enrichment failed");
                assert!(!enriched.optimized);
                assert_eq!(enriched.page_count, 1);
                assert_eq!(enriched.pages.len(), 1);
            }
            other => panic!("expected document enrichment, got {other:?}"),
        }

        drop(request_tx);
        let _ = fs::remove_file(path);
    }
}
