use std::collections::VecDeque;
use std::path::PathBuf;
use std::thread;

use crossbeam_channel::{Receiver, Sender, unbounded};

use crate::model::{LoadedDocument, PageTextChar, RenderedPage};
use crate::pdf_backend::{PdfEngine, RenderQuality};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PageRenderKey {
    pub document_epoch: u64,
    pub page_index: usize,
    pub zoom_key: u32,
    pub render_scale_key: u32,
}

impl PageRenderKey {
    pub fn new(document_epoch: u64, page_index: usize, zoom: f32, render_scale: f32) -> Self {
        let _ = zoom;
        Self {
            document_epoch,
            page_index,
            zoom_key: 0,
            render_scale_key: float_key(render_scale),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ThumbnailRenderKey {
    pub document_epoch: u64,
    pub page_index: usize,
}

#[derive(Debug)]
pub enum RenderRequest {
    LoadDocument {
        path: PathBuf,
        reply: Sender<Result<LoadedDocument, String>>,
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
    },
    PageImmediate {
        path: PathBuf,
        page_index: usize,
        render_scale: f32,
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
}

#[derive(Debug)]
pub enum RenderEvent {
    Page {
        key: PageRenderKey,
        path: PathBuf,
        _zoom: f32,
        render_scale: f32,
        result: Result<RenderedPage, String>,
    },
    Thumbnail {
        key: ThumbnailRenderKey,
        path: PathBuf,
        result: Result<RenderedPage, String>,
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
}

pub fn spawn_render_worker() -> (Sender<RenderRequest>, Receiver<RenderEvent>) {
    let (request_tx, request_rx) = unbounded();
    let (event_tx, event_rx) = unbounded();

    thread::spawn(move || {
        let engine = match PdfEngine::new() {
            Ok(engine) => engine,
            Err(error) => {
                let message = error.to_string();
                while let Ok(request) = request_rx.recv() {
                    let _ = event_tx.send(error_event(request, message.clone()));
                }
                return;
            }
        };

        let mut backlog = VecDeque::new();
        loop {
            let Some(request) = backlog.pop_front().or_else(|| request_rx.recv().ok()) else {
                break;
            };
            let request = coalesce_render_request(request, &request_rx, &mut backlog);
            let event = match request {
                RenderRequest::LoadDocument { path, reply } => {
                    let _ = reply.send(
                        engine
                            .load_document(&path)
                            .map_err(|error| error.to_string()),
                    );
                    continue;
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
                } => RenderEvent::Page {
                    key,
                    path: path.clone(),
                    _zoom: zoom,
                    render_scale,
                    result: engine
                        .render_page(&path, key.page_index, render_scale)
                        .map_err(|error| error.to_string()),
                },
                RenderRequest::PageImmediate {
                    path,
                    page_index,
                    render_scale,
                    reply,
                } => {
                    let _ = reply.send(
                        engine
                            .render_page(&path, page_index, render_scale)
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
                        .render_page_with_quality(
                            &path,
                            key.page_index,
                            render_scale,
                            RenderQuality::Fast,
                        )
                        .map_err(|error| error.to_string()),
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
            };

            let _ = event_tx.send(event);
        }
    });

    (request_tx, event_rx)
}

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
        RenderRequest::LoadDocument { reply, .. } => {
            let _ = reply.send(Err(message));
            RenderEvent::Thumbnail {
                key: ThumbnailRenderKey {
                    document_epoch: 0,
                    page_index: 0,
                },
                path: PathBuf::new(),
                result: Err("PDF worker failed before loading document".to_owned()),
            }
        }
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
                key: ThumbnailRenderKey {
                    document_epoch: 0,
                    page_index: 0,
                },
                path: PathBuf::new(),
                result: Err("PDF worker failed before immediate page render".to_owned()),
            }
        }
        RenderRequest::Thumbnail { key, path, .. } => RenderEvent::Thumbnail {
            key,
            path,
            result: Err(message),
        },
        RenderRequest::ExportPagePng { reply, .. } => {
            let _ = reply.send(Err(message));
            RenderEvent::Thumbnail {
                key: ThumbnailRenderKey {
                    document_epoch: 0,
                    page_index: 0,
                },
                path: PathBuf::new(),
                result: Err("PDF worker failed before exporting PNG".to_owned()),
            }
        }
    }
}

fn float_key(value: f32) -> u32 {
    if value.is_finite() {
        (value.max(0.0) * 1000.0).round() as u32
    } else {
        0
    }
}
