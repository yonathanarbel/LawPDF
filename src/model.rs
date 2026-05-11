use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Select,
    Marker,
    TextBox,
    Signature,
}

impl Tool {
    pub const ALL: [Tool; 4] = [Tool::Select, Tool::Marker, Tool::TextBox, Tool::Signature];

    pub fn label(self) -> &'static str {
        match self {
            Tool::Select => "Select",
            Tool::Marker => "Marker",
            Tool::TextBox => "Text box",
            Tool::Signature => "E-sign",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PdfRect {
    pub left: f32,
    pub bottom: f32,
    pub right: f32,
    pub top: f32,
}

impl PdfRect {
    pub fn new(mut left: f32, mut bottom: f32, mut right: f32, mut top: f32) -> Self {
        if left > right {
            std::mem::swap(&mut left, &mut right);
        }
        if bottom > top {
            std::mem::swap(&mut bottom, &mut top);
        }

        Self {
            left,
            bottom,
            right,
            top,
        }
    }

    pub fn width(self) -> f32 {
        (self.right - self.left).max(0.0)
    }

    pub fn height(self) -> f32 {
        (self.top - self.bottom).max(0.0)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AnnotationKind {
    Marker {
        color_rgb: [f32; 3],
        opacity: f32,
        style: MarkerStyle,
    },
    TextBox {
        text: String,
        font_size: f32,
        color_rgb: [f32; 3],
    },
    Signature {
        signer: String,
        signed_at: String,
        strokes: Vec<Vec<(f32, f32)>>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkerStyle {
    Highlight,
    Underline,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EditorAnnotation {
    pub page_index: usize,
    pub rect: PdfRect,
    pub kind: AnnotationKind,
}

#[derive(Debug, Clone)]
pub struct PageInfo {
    pub width: f32,
    pub height: f32,
}

impl PageInfo {
    pub fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }
}

#[derive(Debug, Clone)]
pub struct PageTextChar {
    pub ch: char,
    pub rect: Option<PdfRect>,
}

#[derive(Debug, Clone)]
pub struct LoadedDocument {
    pub path: PathBuf,
    pub title: String,
    pub page_count: usize,
    pub pages: Vec<PageInfo>,
    pub native_text: Vec<String>,
    pub text_chars: Vec<Option<Vec<PageTextChar>>>,
}

#[derive(Debug, Clone)]
pub struct RenderedPage {
    pub page_index: usize,
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchSource {
    NativeText,
    OcrText,
}

impl SearchSource {
    pub fn label(self) -> &'static str {
        match self {
            SearchSource::NativeText => "PDF",
            SearchSource::OcrText => "OCR",
        }
    }
}

#[derive(Debug, Clone)]
pub struct SearchHit {
    pub page_index: usize,
    pub source: SearchSource,
    pub match_start: usize,
    pub snippet: String,
}

#[derive(Debug, Clone)]
pub enum OcrPageState {
    Idle,
    Pending,
    Running,
    Done(String),
    Failed(String),
}

impl OcrPageState {
    pub fn text(&self) -> Option<&str> {
        match self {
            OcrPageState::Done(text) => Some(text),
            _ => None,
        }
    }

    pub fn label(&self) -> String {
        match self {
            OcrPageState::Idle => "idle".to_owned(),
            OcrPageState::Pending => "pending".to_owned(),
            OcrPageState::Running => "running".to_owned(),
            OcrPageState::Done(_) => "done".to_owned(),
            OcrPageState::Failed(err) => format!("failed: {err}"),
        }
    }
}
