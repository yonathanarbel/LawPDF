//! Supervised PDFium boundary. Native code never receives a write command for
//! the user's PDF. A failed or stalled child is killed; the next request starts
//! a fresh one. Protocol limits apply before allocating peer-supplied lengths.
// Unit tests exercise the in-process backend; the built binary verifies supervision.
#![cfg_attr(test, allow(dead_code))]
use crate::model::{LoadedDocument, PageLink, PageTextChar, PdfNavigation, RenderedPage};
use crate::pdf_backend::{NativePdfEngine, RenderQuality, VisionPage};
use anyhow::{Result, anyhow};
use crossbeam_channel::{Sender, bounded};
use serde::{Deserialize, Serialize};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

const MAX_REQUEST: usize = 128 * 1024;
const MAX_METADATA: usize = 64 * 1024 * 1024;
const MAX_PIXELS: usize = 16_000_000;
const REQUEST_DEADLINE: Duration = Duration::from_secs(120);
const INTERACTIVE_DEADLINE: Duration = Duration::from_secs(10);
const OPENING_PREVIEW_DEADLINE: Duration = Duration::from_secs(2);
static BATCH_CLIENT: OnceLock<Mutex<Option<ProcessClient>>> = OnceLock::new();
static INTERACTIVE_CLIENT: OnceLock<Mutex<Option<ProcessClient>>> = OnceLock::new();
static BATCH_CHILD: OnceLock<Mutex<Option<Weak<Mutex<Child>>>>> = OnceLock::new();
static INTERACTIVE_CHILD: OnceLock<Mutex<Option<Weak<Mutex<Child>>>>> = OnceLock::new();
static SHUTTING_DOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[derive(Clone, Copy)]
enum WorkerLane {
    Batch,
    Interactive,
}

impl WorkerLane {
    fn client(self) -> &'static OnceLock<Mutex<Option<ProcessClient>>> {
        match self {
            Self::Batch => &BATCH_CLIENT,
            Self::Interactive => &INTERACTIVE_CLIENT,
        }
    }
    fn child(self) -> &'static OnceLock<Mutex<Option<Weak<Mutex<Child>>>>> {
        match self {
            Self::Batch => &BATCH_CHILD,
            Self::Interactive => &INTERACTIVE_CHILD,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
enum Request {
    Ready,
    Open {
        path: PathBuf,
        optimized: bool,
    },
    Links {
        path: PathBuf,
        page_count: usize,
    },
    Navigation {
        path: PathBuf,
        page_count: usize,
    },
    Text {
        path: PathBuf,
        page: usize,
    },
    Characters {
        path: PathBuf,
        page: usize,
    },
    Render {
        path: PathBuf,
        page: usize,
        zoom: f32,
        quality: RenderQuality,
        editor: bool,
    },
    Vision {
        path: PathBuf,
        page: usize,
        size: u32,
    },
    Close {
        path: PathBuf,
    },
    #[cfg(feature = "devtools")]
    TestDelay {
        millis: u64,
    },
}

#[derive(Serialize, Deserialize)]
enum Response {
    Ready,
    Document(LoadedDocument),
    Links(Vec<Vec<PageLink>>),
    Navigation(PdfNavigation),
    Text(String),
    Characters {
        #[serde(skip)]
        chars: Vec<PageTextChar>,
    },
    Page(RenderedPage),
    Vision(VisionPage),
}

struct Job {
    request: Request,
    reply: Sender<io::Result<Result<Response, String>>>,
}
struct ProcessClient {
    requests: Sender<Job>,
    child: Arc<Mutex<Child>>,
}

impl ProcessClient {
    fn start(lane: WorkerLane) -> io::Result<Self> {
        let mut command = Command::new(std::env::current_exe()?);
        command
            .arg("--pdf-worker")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        // These are not needed to parse documents. Do not expose provider
        // credentials to the untrusted parser process through its environment.
        for name in [
            "OPENAI_API_KEY",
            "OPENROUTER_API_KEY",
            "GROQ_API_KEY",
            "LAWPDF_OPENAI_API_KEY",
            "LAWPDF_OPENROUTER_API_KEY",
            "LAWPDF_GROQ_API_KEY",
        ] {
            command.env_remove(name);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
        let mut child = command.spawn()?;
        let mut input = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("No PDF worker input"))?;
        let mut output = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("No PDF worker output"))?;
        let child = Arc::new(Mutex::new(child));
        *lane
            .child()
            .get_or_init(|| Mutex::new(None))
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(Arc::downgrade(&child));
        let (requests, receiver) = bounded::<Job>(1);
        // Construct the owner before spawning the transport thread. A failed
        // thread spawn then drops the owner and reaps the child.
        let client = Self { requests, child };
        if SHUTTING_DOWN.load(std::sync::atomic::Ordering::Acquire) {
            return Err(io::Error::other("LawPDF is closing"));
        }
        std::thread::Builder::new()
            .name(
                match lane {
                    WorkerLane::Batch => "pdf-batch-transport",
                    WorkerLane::Interactive => "pdf-interactive-transport",
                }
                .to_owned(),
            )
            .spawn(move || {
                while let Ok(job) = receiver.recv() {
                    let result: io::Result<Result<Response, String>> = (|| {
                        write_json(&mut input, &job.request, MAX_REQUEST)?;
                        input.flush()?;
                        let mut response: Result<Response, String> =
                            read_json(&mut output, MAX_METADATA)?;
                        let blob = read_frame(&mut output, MAX_PIXELS * 4)?;
                        assign_payload(&mut response, blob)?;
                        Ok(response)
                    })();
                    let failed = result.is_err();
                    if let Err(error) = &result {
                        crate::diagnostics::error("pdf_worker_transport_error", &error.to_string());
                    }
                    let _ = job.reply.send(result);
                    if failed {
                        break;
                    }
                }
            })?;
        Ok(client)
    }

    fn call(&self, request: Request, deadline: Duration) -> io::Result<Result<Response, String>> {
        let (reply, result) = bounded(1);
        self.requests
            .try_send(Job { request, reply })
            .map_err(|_| io::Error::other("PDF worker is unavailable"))?;
        result.recv_timeout(deadline).map_err(|_| {
            io::Error::new(io::ErrorKind::TimedOut, "The PDF worker stopped responding")
        })?
    }
}

impl Drop for ProcessClient {
    fn drop(&mut self) {
        let mut child = self.child.lock().unwrap_or_else(|error| error.into_inner());
        let _ = child.kill();
        let _ = child.wait();
    }
}

pub fn shutdown() {
    crate::diagnostics::event("pdf_worker_shutdown_requested");
    SHUTTING_DOWN.store(true, std::sync::atomic::Ordering::Release);
    terminate_worker(WorkerLane::Batch);
    terminate_worker(WorkerLane::Interactive);
    crate::diagnostics::event("pdf_worker_shutdown_complete");
}

fn terminate_worker(lane: WorkerLane) {
    if let Some(registry) = lane.child().get()
        && let Some(child) = registry
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()
            .and_then(Weak::upgrade)
    {
        let mut child = child.lock().unwrap_or_else(|error| error.into_inner());
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn call(request: Request) -> Result<Response> {
    call_with_policy(request, WorkerLane::Batch, REQUEST_DEADLINE, 1)
}

fn call_interactive(request: Request, retry: bool) -> Result<Response> {
    call_with_policy(
        request,
        WorkerLane::Interactive,
        INTERACTIVE_DEADLINE,
        if retry { 2 } else { 1 },
    )
}

fn call_with_policy(
    request: Request,
    lane: WorkerLane,
    deadline: Duration,
    attempts: usize,
) -> Result<Response> {
    anyhow::ensure!(
        !SHUTTING_DOWN.load(std::sync::atomic::Ordering::Acquire),
        "LawPDF is closing"
    );
    let mut client = lane
        .client()
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    for attempt in 0..attempts.max(1) {
        anyhow::ensure!(
            !SHUTTING_DOWN.load(std::sync::atomic::Ordering::Acquire),
            "LawPDF is closing"
        );
        if client.is_none() {
            *client = Some(ProcessClient::start(lane).map_err(|error| {
                crate::diagnostics::error("pdf_worker_start_error", &error.to_string());
                anyhow!(
                    "Could not start the isolated PDF worker. Reinstall LawPDF if this continues."
                )
            })?);
        }
        match client
            .as_ref()
            .expect("started worker")
            .call(request.clone(), deadline)
        {
            Ok(result) => return result.map_err(anyhow::Error::msg),
            Err(error) => {
                crate::diagnostics::error("pdf_worker_call_error", &error.to_string());
                client.take();
                if attempt + 1 >= attempts.max(1) {
                    return Err(anyhow!(
                        "The PDF worker stopped or timed out. LawPDF kept your recovery edits. Try again; the worker will restart automatically."
                    ));
                }
            }
        }
    }
    unreachable!()
}

pub struct PdfEngine;
impl PdfEngine {
    pub fn new() -> Result<Self> {
        match call(Request::Ready)? {
            Response::Ready => Ok(Self),
            _ => Err(protocol_error()),
        }
    }
    pub fn load_document_adaptive(&self, path: &Path, optimized: bool) -> Result<LoadedDocument> {
        match call(Request::Open {
            path: path.to_path_buf(),
            optimized,
        })? {
            Response::Document(document) => Ok(document),
            _ => Err(protocol_error()),
        }
    }
    pub fn load_document(&self, path: &Path) -> Result<LoadedDocument> {
        self.load_document_adaptive(path, false)
    }
    pub fn load_document_metadata_only(&self, path: &Path) -> Result<LoadedDocument> {
        self.load_document_adaptive(path, true)
    }
    pub fn load_document_links(
        &self,
        path: &Path,
        page_count: usize,
    ) -> Result<Vec<Vec<PageLink>>> {
        match call(Request::Links {
            path: path.to_path_buf(),
            page_count,
        })? {
            Response::Links(links) => Ok(links),
            _ => Err(protocol_error()),
        }
    }
    pub fn load_document_navigation(
        &self,
        path: &Path,
        page_count: usize,
    ) -> Result<PdfNavigation> {
        match call(Request::Navigation {
            path: path.to_path_buf(),
            page_count,
        })? {
            Response::Navigation(navigation) => Ok(navigation),
            _ => Err(protocol_error()),
        }
    }
    pub fn load_page_text(&self, path: &Path, page: usize) -> Result<String> {
        match call(Request::Text {
            path: path.to_path_buf(),
            page,
        })? {
            Response::Text(text) => Ok(text),
            _ => Err(protocol_error()),
        }
    }
    pub fn load_page_text_chars(&self, path: &Path, page: usize) -> Result<Vec<PageTextChar>> {
        match call(Request::Characters {
            path: path.to_path_buf(),
            page,
        })? {
            Response::Characters { chars } => Ok(chars),
            _ => Err(protocol_error()),
        }
    }
    pub fn render_page(&self, path: &Path, page: usize, zoom: f32) -> Result<RenderedPage> {
        self.render_page_internal(path, page, zoom, RenderQuality::Crisp, false)
    }
    pub fn render_page_with_quality(
        &self,
        path: &Path,
        page: usize,
        zoom: f32,
        quality: RenderQuality,
    ) -> Result<RenderedPage> {
        self.render_page_internal(path, page, zoom, quality, true)
    }
    pub(crate) fn render_page_internal(
        &self,
        path: &Path,
        page: usize,
        zoom: f32,
        quality: RenderQuality,
        editor: bool,
    ) -> Result<RenderedPage> {
        match call(Request::Render {
            path: path.to_path_buf(),
            page,
            zoom,
            quality,
            editor,
        })? {
            Response::Page(page) => Ok(page),
            _ => Err(protocol_error()),
        }
    }
    pub(crate) fn render_page_interactive(
        &self,
        path: &Path,
        page: usize,
        zoom: f32,
        quality: RenderQuality,
        editor: bool,
        retry: bool,
    ) -> Result<RenderedPage> {
        match call_interactive(
            Request::Render {
                path: path.to_path_buf(),
                page,
                zoom,
                quality,
                editor,
            },
            retry,
        )? {
            Response::Page(page) => Ok(page),
            _ => Err(protocol_error()),
        }
    }
    pub(crate) fn render_opening_preview(&self, path: &Path, page: usize) -> Result<RenderedPage> {
        match call_with_policy(
            Request::Render {
                path: path.to_path_buf(),
                page,
                zoom: 1.0,
                quality: RenderQuality::Fast,
                editor: false,
            },
            WorkerLane::Interactive,
            OPENING_PREVIEW_DEADLINE,
            1,
        )? {
            Response::Page(page) => Ok(page),
            _ => Err(protocol_error()),
        }
    }
    pub fn render_page_for_vision(
        &self,
        path: &Path,
        page: usize,
        size: u32,
    ) -> Result<VisionPage> {
        match call(Request::Vision {
            path: path.to_path_buf(),
            page,
            size,
        })? {
            Response::Vision(page) => Ok(page),
            _ => Err(protocol_error()),
        }
    }
    pub fn export_page_png(
        &self,
        path: &Path,
        page: usize,
        destination: &Path,
        scale: f32,
    ) -> Result<()> {
        let rendered = self.render_page(path, page, scale)?;
        crate::pdf_backend::save_rgba_png(
            destination,
            rendered.width as u32,
            rendered.height as u32,
            rendered.rgba,
        )
    }
    pub fn close_document(&self, path: &Path) {
        let request = Request::Close {
            path: path.to_path_buf(),
        };
        let _ = call(request.clone());
        let interactive_started = INTERACTIVE_CLIENT.get().is_some_and(|slot| {
            slot.lock()
                .unwrap_or_else(|error| error.into_inner())
                .is_some()
        });
        if interactive_started {
            let _ = call_with_policy(request, WorkerLane::Interactive, INTERACTIVE_DEADLINE, 1);
        }
    }
}

fn protocol_error() -> anyhow::Error {
    anyhow!("The PDF worker returned an unexpected response.")
}

#[cfg(feature = "devtools")]
pub fn verify_supervision(path: &Path) -> Result<()> {
    let original = crate::document_store::FileRevision::read(path).map_err(anyhow::Error::msg)?;
    let engine = PdfEngine::new()?;
    let document = engine.load_document(path)?;
    anyhow::ensure!(document.page_count > 0, "The QA PDF has no pages");
    let direct_chars = NativePdfEngine::new()?.load_page_text_chars(path, 0)?;
    let worker_chars = engine.load_page_text_chars(path, 0)?;
    anyhow::ensure!(
        direct_chars.len() == worker_chars.len()
            && direct_chars.iter().zip(&worker_chars).all(|(direct, worker)| {
                direct.ch == worker.ch
                    && direct.rect == worker.rect
                    && direct.font_size == worker.font_size
                    && direct.bold == worker.bold
                    && direct.italic == worker.italic
            }),
        "PDF worker returned different character geometry or formatting"
    );
    let batch_page = engine.render_page(path, 0, 0.5)?;
    let interactive_page =
        engine.render_page_interactive(path, 0, 0.5, RenderQuality::Crisp, false, true)?;
    anyhow::ensure!(
        batch_page.width == interactive_page.width
            && batch_page.height == interactive_page.height
            && batch_page.rgba == interactive_page.rgba,
        "Separate workers returned different page pixels"
    );
    let child_id = |lane: WorkerLane| -> Option<u32> {
        let child = lane.child().get()?.lock().ok()?.as_ref()?.upgrade()?;
        let id = child.lock().ok()?.id();
        Some(id)
    };
    anyhow::ensure!(
        child_id(WorkerLane::Batch).is_some()
            && child_id(WorkerLane::Interactive).is_some()
            && child_id(WorkerLane::Batch) != child_id(WorkerLane::Interactive),
        "PDF lanes must use separate processes"
    );

    // Hold the batch lane in the child while a visible page is rendered.
    let batch = std::thread::spawn(|| {
        call_with_policy(
            Request::TestDelay { millis: 3000 },
            WorkerLane::Batch,
            Duration::from_secs(10),
            1,
        )
    });
    let until = std::time::Instant::now() + Duration::from_secs(2);
    while BATCH_CLIENT.get().unwrap().try_lock().is_ok() && std::time::Instant::now() < until {
        std::thread::sleep(Duration::from_millis(5));
    }
    anyhow::ensure!(
        BATCH_CLIENT.get().unwrap().try_lock().is_err(),
        "Batch lane did not start"
    );
    engine.render_page_interactive(path, 0, 0.5, RenderQuality::Crisp, false, true)?;
    anyhow::ensure!(
        !batch.is_finished(),
        "Interactive rendering waited for the batch lane"
    );
    batch
        .join()
        .map_err(|_| anyhow!("Batch QA thread panicked"))??;

    // An interactive crash is retried once on a new child.
    terminate_worker(WorkerLane::Interactive);
    engine.render_page_interactive(path, 0, 0.5, RenderQuality::Crisp, false, true)?;
    // A timed-out transport is killed and a later request starts cleanly.
    anyhow::ensure!(
        call_with_policy(
            Request::TestDelay { millis: 1000 },
            WorkerLane::Interactive,
            Duration::from_millis(50),
            1
        )
        .is_err(),
        "Interactive timeout was not enforced"
    );
    engine.render_page_interactive(path, 0, 0.5, RenderQuality::Crisp, false, true)?;

    terminate_worker(WorkerLane::Batch); // Simulate a batch crash while the reader stays alive.
    anyhow::ensure!(
        engine.load_document(path).is_err(),
        "The killed worker should report failure"
    );
    let recovered = engine.load_document(path)?;
    anyhow::ensure!(
        recovered.page_count == document.page_count,
        "The restarted worker changed metadata"
    );
    engine.render_page(path, 0, 0.5)?;
    engine.close_document(path);
    // Saving replaces a file, so both workers must release their cached handles.
    // Use a disposable copy and verify that annotations survive the rewrite.
    let scratch = tempfile::NamedTempFile::new()?.into_temp_path();
    std::fs::copy(path, &scratch)?;
    engine.render_page(&scratch, 0, 0.5)?;
    engine.render_page_interactive(&scratch, 0, 0.5, RenderQuality::Crisp, false, true)?;
    engine.close_document(&scratch);
    let mark = crate::model::EditorAnnotation {
        page_index: 0,
        rect: crate::model::PdfRect::new(20.0, 90.0, 180.0, 110.0),
        kind: crate::model::AnnotationKind::Marker {
            color_rgb: [1.0, 0.93, 0.45],
            opacity: 0.42,
            style: crate::model::MarkerStyle::Highlight,
        },
    };
    crate::pdf_backend::save_with_annotations(&scratch, &scratch, &[mark])?;
    anyhow::ensure!(
        crate::pdf_backend::load_lawpdf_annotations(&scratch)?.len() == 1,
        "The saved annotation did not survive reopening"
    );
    engine.render_page(&scratch, 0, 0.5)?;
    engine.render_page_interactive(&scratch, 0, 0.5, RenderQuality::Crisp, false, true)?;
    engine.close_document(&scratch);
    original.require_current(path).map_err(anyhow::Error::msg)?;
    shutdown();
    anyhow::ensure!(
        PdfEngine::new().is_err(),
        "Shutdown must reject new worker requests"
    );
    println!(
        "Separate PDF workers, interactive timeout and crash recovery passed; source unchanged"
    );
    Ok(())
}

pub fn run_worker() -> io::Result<()> {
    let engine =
        NativePdfEngine::new().map_err(|_| io::Error::other("Could not initialize PDFium"))?;
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    loop {
        let request = match read_json::<Request>(&mut input, MAX_REQUEST) {
            Ok(request) => request,
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(error) => return Err(error),
        };
        let mut response = execute(&engine, request).map_err(|error| format!("{error:#}"));
        let characters = match &response {
            Ok(Response::Characters { chars }) => match crate::page_text_codec::encode(chars) {
                Ok(bytes) => Some(bytes),
                Err(error) => {
                    response = Err(format!("Could not encode PDF page text: {error}"));
                    None
                }
            },
            _ => None,
        };
        write_json(&mut output, &response, MAX_METADATA)?;
        let blob = match &response {
            Ok(Response::Page(page)) => page.rgba.as_slice(),
            Ok(Response::Vision(page)) => page.rgb.as_slice(),
            Ok(Response::Characters { .. }) => characters.as_deref().unwrap_or_default(),
            _ => &[],
        };
        write_frame(&mut output, blob, MAX_PIXELS * 4)?;
        output.flush()?;
    }
}

fn execute(engine: &NativePdfEngine, request: Request) -> Result<Response> {
    match &request {
        Request::Ready | Request::Close { .. } => {}
        #[cfg(feature = "devtools")]
        Request::TestDelay { .. } => {}
        Request::Open { path, .. }
        | Request::Links { path, .. }
        | Request::Navigation { path, .. }
        | Request::Text { path, .. }
        | Request::Characters { path, .. }
        | Request::Render { path, .. }
        | Request::Vision { path, .. } => {
            let metadata = std::fs::metadata(path)?;
            anyhow::ensure!(
                metadata.is_file() && metadata.len() <= crate::document_store::MAX_DOCUMENT_BYTES,
                "This PDF exceeds the 512 MB document limit."
            );
        }
    }
    match &request {
        Request::Text { page, .. }
        | Request::Characters { page, .. }
        | Request::Render { page, .. }
        | Request::Vision { page, .. } => {
            anyhow::ensure!(*page < 20_000, "This page exceeds the 20,000-page limit.")
        }
        Request::Links { page_count, .. } | Request::Navigation { page_count, .. } => {
            anyhow::ensure!(
                *page_count <= 20_000,
                "This PDF exceeds the 20,000-page limit."
            )
        }
        _ => {}
    }
    match request {
        Request::Ready => Ok(Response::Ready),
        Request::Open { path, optimized } => engine
            .load_document_adaptive(&path, optimized)
            .map(Response::Document),
        Request::Links { path, page_count } => engine
            .load_document_links(&path, page_count)
            .map(Response::Links),
        Request::Navigation { path, page_count } => engine
            .load_document_navigation(&path, page_count)
            .map(Response::Navigation),
        Request::Text { path, page } => engine.load_page_text(&path, page).map(Response::Text),
        Request::Characters { path, page } => engine
            .load_page_text_chars(&path, page)
            .map(|chars| Response::Characters { chars }),
        Request::Render {
            path,
            page,
            zoom,
            quality,
            editor,
        } => engine
            .render_page_internal(&path, page, zoom, quality, editor)
            .map(Response::Page),
        Request::Vision { path, page, size } => engine
            .render_page_for_vision(&path, page, size)
            .map(Response::Vision),
        Request::Close { path } => {
            engine.close_document(&path);
            Ok(Response::Ready)
        }
        #[cfg(feature = "devtools")]
        Request::TestDelay { millis } => {
            anyhow::ensure!(millis <= 5000, "QA delay exceeds the limit");
            std::thread::sleep(Duration::from_millis(millis));
            Ok(Response::Ready)
        }
    }
}

fn pixel_bytes(width: usize, height: usize, channels: usize) -> io::Result<usize> {
    width
        .checked_mul(height)
        .filter(|pixels| *pixels > 0 && *pixels <= MAX_PIXELS)
        .and_then(|pixels| pixels.checked_mul(channels))
        .ok_or_else(|| io::Error::other("PDF raster exceeds the pixel limit"))
}

fn assign_payload(response: &mut Result<Response, String>, blob: Vec<u8>) -> io::Result<()> {
    match response {
        Ok(Response::Page(page)) => {
            if pixel_bytes(page.width, page.height, 4)? != blob.len() {
                return Err(io::Error::other("Invalid PDF raster length"));
            }
            page.rgba = blob;
        }
        Ok(Response::Vision(page)) => {
            if pixel_bytes(page.width, page.height, 3)? != blob.len() {
                return Err(io::Error::other("Invalid analysis raster length"));
            }
            page.rgb = blob;
        }
        Ok(Response::Characters { chars }) => {
            *chars = crate::page_text_codec::decode(&blob)?;
        }
        _ if !blob.is_empty() => return Err(io::Error::other("Unexpected PDF worker payload")),
        _ => {}
    }
    Ok(())
}
fn write_json(output: &mut impl Write, value: &impl Serialize, limit: usize) -> io::Result<()> {
    let bytes = serde_json::to_vec(value).map_err(io::Error::other)?;
    write_frame(output, &bytes, limit)
}
fn read_json<T: serde::de::DeserializeOwned>(input: &mut impl Read, limit: usize) -> io::Result<T> {
    serde_json::from_slice(&read_frame(input, limit)?).map_err(io::Error::other)
}
fn write_frame(output: &mut impl Write, bytes: &[u8], limit: usize) -> io::Result<()> {
    if bytes.len() > limit {
        return Err(io::Error::other(
            "PDF worker response exceeds the supported size",
        ));
    }
    output.write_all(&(bytes.len() as u32).to_le_bytes())?;
    output.write_all(bytes)
}
fn read_frame(input: &mut impl Read, limit: usize) -> io::Result<Vec<u8>> {
    let mut length = [0u8; 4];
    input.read_exact(&mut length)?;
    let length = u32::from_le_bytes(length) as usize;
    if length > limit {
        return Err(io::Error::other(
            "PDF worker frame exceeds the supported size",
        ));
    }
    let mut bytes = vec![0; length];
    input.read_exact(&mut bytes)?;
    Ok(bytes)
}

#[cfg(test)]
mod character_transport_tests {
    use super::*;

    #[test]
    fn response_metadata_never_contains_character_data() {
        let response: Result<Response, String> = Ok(Response::Characters {
            chars: vec![PageTextChar {
                ch: '§', rect: None, font_size: Some(12.0), bold: false, italic: true,
            }],
        });
        let metadata = serde_json::to_vec(&response).unwrap();
        assert!(metadata.len() < 64);
        assert!(!String::from_utf8_lossy(&metadata).contains('§'));
        let decoded: Result<Response, String> = serde_json::from_slice(&metadata).unwrap();
        assert!(matches!(decoded, Ok(Response::Characters { chars }) if chars.is_empty()));
    }

    #[test]
    fn character_payload_populates_response_and_rejects_corruption() {
        let original = vec![PageTextChar {
            ch: '𝄞', rect: Some(crate::model::PdfRect::new(1.0, 2.0, 3.0, 4.0)),
            font_size: Some(9.5), bold: true, italic: true,
        }];
        let encoded = crate::page_text_codec::encode(&original).unwrap();
        let mut response = Ok(Response::Characters { chars: Vec::new() });
        assign_payload(&mut response, encoded.clone()).unwrap();
        let Ok(Response::Characters { chars }) = response else { panic!("wrong response") };
        assert_eq!((chars[0].ch, chars[0].rect, chars[0].font_size, chars[0].bold, chars[0].italic),
                   (original[0].ch, original[0].rect, original[0].font_size, original[0].bold, original[0].italic));
        let mut response = Ok(Response::Characters { chars: Vec::new() });
        assert!(assign_payload(&mut response, encoded[..encoded.len() - 1].to_vec()).is_err());
        assert!(assign_payload(&mut Ok(Response::Ready), encoded).is_err());
    }
}
