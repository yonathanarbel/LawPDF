use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crossbeam_channel::Sender;

use crate::model::OcrPageState;
use crate::render_worker::RenderRequest;

#[derive(Debug, Clone)]
pub struct OcrEvent {
    pub document_epoch: u64,
    pub path: PathBuf,
    pub page_index: usize,
    pub state: OcrPageState,
}

pub fn spawn_ocr_job(
    document_epoch: u64,
    pdf_path: PathBuf,
    page_count: usize,
    tx: Sender<OcrEvent>,
    render_tx: Sender<RenderRequest>,
) {
    thread::spawn(move || {
        for page_index in 0..page_count {
            let _ = tx.send(OcrEvent {
                document_epoch,
                path: pdf_path.clone(),
                page_index,
                state: OcrPageState::Running,
            });

            let state = match ocr_page(&render_tx, &pdf_path, page_index) {
                Ok(text) => OcrPageState::Done(text),
                Err(error) => OcrPageState::Failed(error),
            };

            let _ = tx.send(OcrEvent {
                document_epoch,
                path: pdf_path.clone(),
                page_index,
                state,
            });
        }
    });
}

fn ocr_page(
    render_tx: &Sender<RenderRequest>,
    pdf_path: &PathBuf,
    page_index: usize,
) -> Result<String, String> {
    let image_path = temp_image_path(page_index);
    export_ocr_image(render_tx, pdf_path.clone(), page_index, image_path.clone())?;

    let output = Command::new("tesseract")
        .arg(&image_path)
        .arg("stdout")
        .arg("-l")
        .arg("eng")
        .arg("--psm")
        .arg("6")
        .output();

    let _ = std::fs::remove_file(&image_path);

    let output = output.map_err(|error| {
        format!(
            "failed to run tesseract; install Tesseract OCR and make sure it is on PATH: {error}"
        )
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(if stderr.is_empty() {
            format!("tesseract exited with {}", output.status)
        } else {
            stderr
        });
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn export_ocr_image(
    render_tx: &Sender<RenderRequest>,
    pdf_path: PathBuf,
    page_index: usize,
    image_path: PathBuf,
) -> Result<(), String> {
    let (reply_tx, reply_rx) = crossbeam_channel::unbounded();
    render_tx
        .send(RenderRequest::ExportPagePng {
            path: pdf_path,
            page_index,
            destination: image_path,
            scale: 2.2,
            reply: reply_tx,
        })
        .map_err(|error| format!("PDF worker is not available: {error}"))?;
    reply_rx
        .recv_timeout(Duration::from_secs(120))
        .map_err(|error| format!("timed out rendering OCR page image: {error}"))?
}

fn temp_image_path(page_index: usize) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();

    std::env::temp_dir().join(format!(
        "rust_pdf_editor_ocr_{stamp}_p{}.png",
        page_index + 1
    ))
}
