use std::collections::VecDeque;
use std::path::PathBuf;
use std::thread;

use crossbeam_channel::{Receiver, Sender, unbounded};
use eframe::egui::Context;

use crate::model::{EditorAnnotation, LoadedDocument, PageTextChar, RenderedPage};
use crate::pdf_backend::{PdfEngine, RenderQuality, rotate_pdf_page, sync_lawpdf_comments};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PageRenderKey {
    pub document_epoch: u64,
    pub page_index: usize,
    pub render_scale_key: u32,
}

impl PageRenderKey {
    pub fn new(document_epoch: u64, page_index: usize, render_scale: f32) -> Self {
        Self {
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
}

#[derive(Debug)]
pub enum RenderRequest {
    LoadDocument {
        path: PathBuf,
        optimize_large_documents: bool,
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
    SyncComments {
        document_epoch: u64,
        path: PathBuf,
        generation: u64,
        comments: Vec<EditorAnnotation>,
    },
    RotatePage {
        document_epoch: u64,
        path: PathBuf,
        page_index: usize,
        clockwise: bool,
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
    CommentsSaved {
        document_epoch: u64,
        path: PathBuf,
        generation: u64,
        result: Result<usize, String>,
    },
    PageRotated {
        document_epoch: u64,
        path: PathBuf,
        page_index: usize,
        result: Result<(LoadedDocument, i64), String>,
    },
}

pub fn spawn_render_worker(
    repaint_context: Option<Context>,
) -> (Sender<RenderRequest>, Receiver<RenderEvent>) {
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
            let Some(request) = next_prioritized_request(&request_rx, &mut backlog) else {
                break;
            };
            let event = match request {
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
                        .render_page_with_quality(
                            &path,
                            key.page_index,
                            render_scale,
                            if fast {
                                RenderQuality::Fast
                            } else {
                                RenderQuality::Crisp
                            },
                        )
                        .map_err(|error| error.to_string()),
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
                            .render_page_with_quality(
                                &path,
                                page_index,
                                render_scale,
                                if fast {
                                    RenderQuality::Fast
                                } else {
                                    RenderQuality::Crisp
                                },
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
                RenderRequest::SyncComments {
                    document_epoch,
                    path,
                    generation,
                    comments,
                } => {
                    engine.close_document(&path);
                    RenderEvent::CommentsSaved {
                        document_epoch,
                        path: path.clone(),
                        generation,
                        result: sync_lawpdf_comments(&path, &comments)
                            .map_err(|error| error.to_string()),
                    }
                }
                RenderRequest::RotatePage {
                    document_epoch,
                    path,
                    page_index,
                    clockwise,
                } => {
                    engine.close_document(&path);
                    let result = rotate_pdf_page(&path, page_index, clockwise)
                        .and_then(|rotation| {
                            engine
                                .load_document_adaptive(&path, true)
                                .map(|document| (document, rotation))
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

            if event_tx.send(event).is_ok() {
                if let Some(ctx) = &repaint_context {
                    ctx.request_repaint();
                }
            }
        }
    });

    (request_tx, event_rx)
}

fn next_prioritized_request(
    request_rx: &Receiver<RenderRequest>,
    backlog: &mut VecDeque<RenderRequest>,
) -> Option<RenderRequest> {
    if backlog.is_empty() {
        backlog.push_back(request_rx.recv().ok()?);
    }
    while let Ok(request) = request_rx.try_recv() {
        push_coalesced(backlog, request);
    }
    let best = backlog
        .iter()
        .enumerate()
        .min_by_key(|(index, request)| (request_priority(request), *index))
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
        RenderRequest::LoadDocument { .. }
        | RenderRequest::PageImmediate { .. }
        | RenderRequest::ExportPagePng { .. }
        | RenderRequest::SyncComments { .. }
        | RenderRequest::RotatePage { .. } => 0,
        RenderRequest::Page { .. } => 1,
        RenderRequest::TextCharsAsync { .. } => 2,
        RenderRequest::Thumbnail { .. } => 3,
        RenderRequest::TextPageAsync { .. } => 4,
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
        RenderRequest::SyncComments {
            document_epoch,
            path,
            generation,
            ..
        } => RenderEvent::CommentsSaved {
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
            key: ThumbnailRenderKey {
                document_epoch: 7,
                page_index: 0,
            },
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

        let next = next_prioritized_request(&rx, &mut backlog).unwrap();

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

}
