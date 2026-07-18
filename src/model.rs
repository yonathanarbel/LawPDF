use std::path::PathBuf;

use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
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
    Comment {
        id: String,
        text: String,
        color_rgb: [f32; 3],
        created_at: String,
        updated_at: String,
        /// The point on the page text the comment refers to, in PDF
        /// coordinates. The annotation's `rect` is the margin card; a dotted
        /// leader connects this anchor to that card.
        anchor: (f32, f32),
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageInfo {
    pub width: f32,
    pub height: f32,
    pub footnote_divider_y_from_top: Option<f32>,
    pub coord_offset_left: f32,
    pub coord_offset_bottom: f32,
    pub path_object_rects: Vec<PdfRect>,
    pub image_object_rects: Vec<PdfRect>,
    pub thin_horizontal_object_rects: Vec<PdfRect>,
    pub thin_vertical_object_rects: Vec<PdfRect>,
    pub vector_horizontal_rule_rects: Vec<PdfRect>,
    pub vector_vertical_rule_rects: Vec<PdfRect>,
    pub vector_ruled_cell_rects: Vec<PdfRect>,
}

impl PageInfo {
    #[cfg(test)]
    pub fn new(width: f32, height: f32) -> Self {
        Self {
            width,
            height,
            footnote_divider_y_from_top: None,
            coord_offset_left: 0.0,
            coord_offset_bottom: 0.0,
            path_object_rects: Vec::new(),
            image_object_rects: Vec::new(),
            thin_horizontal_object_rects: Vec::new(),
            thin_vertical_object_rects: Vec::new(),
            vector_horizontal_rule_rects: Vec::new(),
            vector_vertical_rule_rects: Vec::new(),
            vector_ruled_cell_rects: Vec::new(),
        }
    }

    pub fn with_footnote_divider_y_from_top(
        width: f32,
        height: f32,
        footnote_divider_y_from_top: Option<f32>,
    ) -> Self {
        Self {
            width,
            height,
            footnote_divider_y_from_top,
            coord_offset_left: 0.0,
            coord_offset_bottom: 0.0,
            path_object_rects: Vec::new(),
            image_object_rects: Vec::new(),
            thin_horizontal_object_rects: Vec::new(),
            thin_vertical_object_rects: Vec::new(),
            vector_horizontal_rule_rects: Vec::new(),
            vector_vertical_rule_rects: Vec::new(),
            vector_ruled_cell_rects: Vec::new(),
        }
    }

    pub fn with_coordinate_offset(mut self, left: f32, bottom: f32) -> Self {
        self.coord_offset_left = left;
        self.coord_offset_bottom = bottom;
        self
    }

    pub fn with_page_object_rects(
        mut self,
        path_object_rects: Vec<PdfRect>,
        image_object_rects: Vec<PdfRect>,
        thin_horizontal_object_rects: Vec<PdfRect>,
        thin_vertical_object_rects: Vec<PdfRect>,
    ) -> Self {
        self.path_object_rects = path_object_rects;
        self.image_object_rects = image_object_rects;
        self.thin_horizontal_object_rects = thin_horizontal_object_rects;
        self.thin_vertical_object_rects = thin_vertical_object_rects;
        self
    }

    pub fn with_vector_rule_geometry(
        mut self,
        horizontal_rules: Vec<PdfRect>,
        vertical_rules: Vec<PdfRect>,
        ruled_cells: Vec<PdfRect>,
    ) -> Self {
        self.vector_horizontal_rule_rects = horizontal_rules;
        self.vector_vertical_rule_rects = vertical_rules;
        self.vector_ruled_cell_rects = ruled_cells;
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageTextChar {
    pub ch: char,
    pub rect: Option<PdfRect>,
    pub font_size: Option<f32>,
    pub bold: bool,
    pub italic: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PageLink {
    pub rect: PdfRect,
    pub url: String,
}

#[derive(Debug, Clone)]
pub struct LoadedDocument {
    pub path: PathBuf,
    pub title: String,
    pub page_count: usize,
    pub pages: Vec<PageInfo>,
    pub native_text: Vec<String>,
    pub native_text_loaded: Vec<bool>,
    pub text_chars: Vec<Option<Vec<PageTextChar>>>,
    pub links: Vec<Vec<PageLink>>,
    pub links_loaded: bool,
    /// Large-document performance mode keeps opening, searching, and rendering
    /// responsive by deferring expensive page-layout analysis.
    pub optimized: bool,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    pub page_index: usize,
    pub source: SearchSource,
    pub match_start: usize,
    pub match_end: usize,
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
