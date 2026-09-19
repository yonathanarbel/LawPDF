//! Supervised PDFium boundary. Native code never receives a write command for
//! the user's PDF. A failed or stalled child is killed; the next request starts
//! a fresh one. Protocol limits apply before allocating peer-supplied lengths.
// Unit tests exercise the in-process backend; the built binary verifies supervision.
#![cfg_attr(test, allow(dead_code))]
use crate::model::{LoadedDocument, PageLink, PageTextChar, RenderedPage};
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
static CLIENT: OnceLock<Mutex<Option<ProcessClient>>> = OnceLock::new();
static CHILD: OnceLock<Mutex<Option<Weak<Mutex<Child>>>>> = OnceLock::new();
static SHUTTING_DOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[derive(Serialize, Deserialize)]
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
}

#[derive(Serialize, Deserialize)]
enum Response {
    Ready,
    Document(LoadedDocument),
    Links(Vec<Vec<PageLink>>),
    Text(String),
    Characters(Vec<PageTextChar>),
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
    fn start() -> io::Result<Self> {
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
        *CHILD
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
            .name("pdf-process-transport".to_owned())
            .spawn(move || {
                while let Ok(job) = receiver.recv() {
                    let result = (|| {
                        write_json(&mut input, &job.request, MAX_REQUEST)?;
                        input.flush()?;
                        let mut response: Result<Response, String> =
                            read_json(&mut output, MAX_METADATA)?;
                        let blob = read_frame(&mut output, MAX_PIXELS * 4)?;
                        match &mut response {
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
                            _ if !blob.is_empty() => {
                                return Err(io::Error::other("Unexpected PDF worker payload"));
                            }
                            _ => {}
                        }
                        Ok(response)
                    })();
                    let failed = result.is_err();
                    let _ = job.reply.send(result);
                    if failed {
                        break;
                    }
                }
            })?;
        Ok(client)
    }

    fn call(&self, request: Request) -> io::Result<Result<Response, String>> {
        let (reply, result) = bounded(1);
        self.requests
            .try_send(Job { request, reply })
            .map_err(|_| io::Error::other("PDF worker is unavailable"))?;
        result.recv_timeout(REQUEST_DEADLINE).map_err(|_| {
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
    SHUTTING_DOWN.store(true, std::sync::atomic::Ordering::Release);
    terminate_current_worker();
}

fn terminate_current_worker() {
    if let Some(registry) = CHILD.get()
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
    anyhow::ensure!(!SHUTTING_DOWN.load(std::sync::atomic::Ordering::Acquire), "LawPDF is closing");
    let mut client = CLIENT
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if client.is_none() {
        *client = Some(ProcessClient::start().map_err(|_| {
            anyhow!("Could not start the isolated PDF worker. Reinstall LawPDF if this continues.")
        })?);
    }
    match client.as_ref().expect("started worker").call(request) {
        Ok(result) => result.map_err(anyhow::Error::msg),
        Err(_) => {
            client.take();
            Err(anyhow!(
                "The PDF worker stopped or exceeded its two-minute limit. LawPDF kept your recovery edits. Try opening the PDF again; the worker will restart automatically."
            ))
        }
    }
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
            Response::Characters(chars) => Ok(chars),
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
        let _ = call(Request::Close {
            path: path.to_path_buf(),
        });
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
    engine.render_page(path, 0, 0.5)?;
    terminate_current_worker(); // Simulate a crash while the reader stays alive.
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
    original.require_current(path).map_err(anyhow::Error::msg)?;
    shutdown();
    anyhow::ensure!(PdfEngine::new().is_err(), "Shutdown must reject new worker requests");
    println!("PDF worker crash recovery passed; source unchanged");
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
        let response = execute(&engine, request).map_err(|error| format!("{error:#}"));
        write_json(&mut output, &response, MAX_METADATA)?;
        let blob = match &response {
            Ok(Response::Page(page)) => page.rgba.as_slice(),
            Ok(Response::Vision(page)) => page.rgb.as_slice(),
            _ => &[],
        };
        write_frame(&mut output, blob, MAX_PIXELS * 4)?;
        output.flush()?;
    }
}

fn execute(engine: &NativePdfEngine, request: Request) -> Result<Response> {
    match &request {
        Request::Ready | Request::Close { .. } => {}
        Request::Open { path, .. }
        | Request::Links { path, .. }
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
        Request::Links { page_count, .. } => anyhow::ensure!(
            *page_count <= 20_000,
            "This PDF exceeds the 20,000-page limit."
        ),
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
        Request::Text { path, page } => engine.load_page_text(&path, page).map(Response::Text),
        Request::Characters { path, page } => engine
            .load_page_text_chars(&path, page)
            .map(Response::Characters),
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
    }
}

fn pixel_bytes(width: usize, height: usize, channels: usize) -> io::Result<usize> {
    width
        .checked_mul(height)
        .filter(|pixels| *pixels > 0 && *pixels <= MAX_PIXELS)
        .and_then(|pixels| pixels.checked_mul(channels))
        .ok_or_else(|| io::Error::other("PDF raster exceeds the pixel limit"))
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
