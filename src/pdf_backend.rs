use std::cell::RefCell;
use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{Context, Result, anyhow};
use image::RgbaImage;
use lopdf::{
    Dictionary, Document, Object, ObjectId, Stream, StringFormat,
    content::{Content, Operation},
    dictionary,
};
use pdfium_render::prelude::*;

use crate::model::{
    AnnotationKind, EditorAnnotation, LoadedDocument, MarkerStyle, PageInfo, PageLink,
    PageTextChar, PdfRect, RenderedPage,
};

pub struct PdfEngine {
    pdfium: &'static Pdfium,
    open_documents: RefCell<VecDeque<OpenPdfDocument>>,
}

struct OpenPdfDocument {
    path: PathBuf,
    document: PdfDocument<'static>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderQuality {
    Crisp,
    Fast,
}

/// RGB page raster for LiquidVision (LmV tier), plus page size in points.
pub struct VisionPage {
    pub rgb: Vec<u8>,
    pub width: usize,
    pub height: usize,
    pub page_width_pts: f64,
    pub page_height_pts: f64,
}

const OPEN_DOCUMENT_CACHE_CAP: usize = 3;
static PDFIUM: OnceLock<Result<&'static Pdfium, String>> = OnceLock::new();
pub const LAWPDF_COMMENT_ID_PREFIX: &str = "LawPDF-comment-";
const LAWPDF_CROPBOX_LOCAL_COORDS_ENV: &str = "LAWPDF_CROPBOX_LOCAL_COORDS";
const LAWPDF_WIDE_FOOTNOTE_DIVIDERS_ENV: &str = "LAWPDF_WIDE_FOOTNOTE_DIVIDERS";

impl PdfEngine {
    pub fn new() -> Result<Self> {
        let pdfium = PDFIUM
            .get_or_init(|| {
                bind_pdfium()
                    .map(Pdfium::new)
                    .map(|pdfium| Box::leak(Box::new(pdfium)) as &'static Pdfium)
                    .map_err(|error| format!("{error:#}"))
            })
            .as_ref()
            .copied()
            .map_err(|error| anyhow!(error.clone()))?;

        Ok(Self {
            pdfium,
            open_documents: RefCell::new(VecDeque::new()),
        })
    }

    pub fn load_document(&self, path: &Path) -> Result<LoadedDocument> {
        self.with_open_document(path, |document| {
            let page_count = document.pages().len() as usize;
            let mut pages = Vec::with_capacity(page_count);
            let mut native_text = Vec::with_capacity(page_count);
            let mut native_text_loaded = Vec::with_capacity(page_count);
            let mut text_chars = Vec::with_capacity(page_count);
            let links = load_pdf_web_links(path, page_count)
                .unwrap_or_else(|_| vec![Vec::new(); page_count]);
            let vector_rule_pages = load_pdf_vector_rule_pages(path, page_count)
                .unwrap_or_else(|_| vec![PageVectorRuleGeometry::default(); page_count]);

            for page_index in 0..page_count {
                let page = document
                    .pages()
                    .get(page_index as u16)
                    .with_context(|| format!("failed to read page {}", page_index + 1))?;

                let crop_box = cropbox_local_coords_enabled()
                    .then(|| page_crop_box_if_distinct_from_media(&page))
                    .flatten();
                let width = crop_box
                    .map(|box_| visible_page_extent(box_.width(), page.width().value))
                    .unwrap_or(page.width().value);
                let height = crop_box
                    .map(|box_| visible_page_extent(box_.height(), page.height().value))
                    .unwrap_or(page.height().value);
                let mut page_info = PageInfo::with_footnote_divider_y_from_top(
                    width,
                    height,
                    detect_footnote_divider_y_from_top(&page, page_index, crop_box, width, height),
                );
                let (
                    path_object_rects,
                    image_object_rects,
                    thin_horizontal_object_rects,
                    thin_vertical_object_rects,
                ) = page_object_rects(&page, crop_box);
                let vector_rules = vector_rule_pages
                    .get(page_index)
                    .cloned()
                    .unwrap_or_default()
                    .with_crop_box(crop_box);
                page_info = page_info.with_page_object_rects(
                    path_object_rects,
                    image_object_rects,
                    thin_horizontal_object_rects,
                    thin_vertical_object_rects,
                );
                page_info = page_info.with_vector_rule_geometry(
                    vector_rules.horizontal_rules,
                    vector_rules.vertical_rules,
                    vector_rules.ruled_cells,
                );
                if let Some(box_) = crop_box {
                    page_info = page_info.with_coordinate_offset(box_.left, box_.bottom);
                }
                pages.push(page_info);
                native_text.push(String::new());
                native_text_loaded.push(false);
                text_chars.push(None);
            }

            let title = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("Untitled PDF")
                .to_owned();

            Ok(LoadedDocument {
                path: path.to_path_buf(),
                title,
                page_count,
                pages,
                native_text,
                native_text_loaded,
                text_chars,
                links,
            })
        })
    }

    pub fn load_page_text(&self, path: &Path, page_index: usize) -> Result<String> {
        self.with_open_document(path, |document| {
            let page = document
                .pages()
                .get(page_index as u16)
                .with_context(|| format!("failed to read page {}", page_index + 1))?;

            let text_page = page
                .text()
                .with_context(|| format!("failed to read text on page {}", page_index + 1))?;

            Ok(text_page.all())
        })
    }

    pub fn load_page_text_chars(
        &self,
        path: &Path,
        page_index: usize,
    ) -> Result<Vec<PageTextChar>> {
        self.with_open_document(path, |document| {
            let page = document
                .pages()
                .get(page_index as u16)
                .with_context(|| format!("failed to read page {}", page_index + 1))?;

            let text_page = page
                .text()
                .with_context(|| format!("failed to read text on page {}", page_index + 1))?;

            Ok(extract_text_chars(&text_page))
        })
    }

    pub fn render_page(&self, path: &Path, page_index: usize, zoom: f32) -> Result<RenderedPage> {
        self.render_page_with_quality(path, page_index, zoom, RenderQuality::Crisp)
    }

    pub fn render_page_with_quality(
        &self,
        path: &Path,
        page_index: usize,
        zoom: f32,
        quality: RenderQuality,
    ) -> Result<RenderedPage> {
        self.with_open_document(path, |document| {
            let page = document
                .pages()
                .get(page_index as u16)
                .with_context(|| format!("failed to read page {}", page_index + 1))?;

            let target_width = (page.width().value * zoom).round().clamp(64.0, 8192.0) as i32;
            let config = match quality {
                RenderQuality::Crisp => PdfRenderConfig::new()
                    .set_target_width(target_width)
                    .render_form_data(true)
                    .use_lcd_text_rendering(true)
                    .set_text_smoothing(true)
                    .set_path_smoothing(true)
                    .set_image_smoothing(true),
                RenderQuality::Fast => PdfRenderConfig::new()
                    .set_target_width(target_width)
                    .render_form_data(true)
                    .use_lcd_text_rendering(false)
                    .set_text_smoothing(false)
                    .set_path_smoothing(false)
                    .set_image_smoothing(false),
            };

            let bitmap = page
                .render_with_config(&config)
                .with_context(|| format!("failed to render page {}", page_index + 1))?;

            let image = bitmap.as_image().to_rgba8();
            let (width, height) = image.dimensions();

            Ok(RenderedPage {
                page_index,
                width: width as usize,
                height: height as usize,
                rgba: image.into_raw(),
            })
        })
    }

    /// Render a page for LiquidVision (LmV tier): letterbox-ready RGB raster
    /// scaled so the long edge is `imgsz` px, plus the page size in points.
    /// Config matches the plain render validated against the Python (fitz) sidecar.
    pub fn render_page_for_vision(
        &self,
        path: &Path,
        page_index: usize,
        imgsz: u32,
    ) -> Result<VisionPage> {
        self.with_open_document(path, |document| {
            let page = document
                .pages()
                .get(page_index as u16)
                .with_context(|| format!("failed to read page {}", page_index + 1))?;
            let w_pts = page.width().value as f64;
            let h_pts = page.height().value as f64;
            let scale = (imgsz as f64 / w_pts).min(imgsz as f64 / h_pts);
            let target_w = (w_pts * scale).round().clamp(1.0, imgsz as f64) as i32;
            let target_h = (h_pts * scale).round().clamp(1.0, imgsz as f64) as i32;
            let config = PdfRenderConfig::new()
                .set_target_width(target_w)
                .set_target_height(target_h)
                .render_form_data(true);
            let bitmap = page
                .render_with_config(&config)
                .with_context(|| format!("failed to render page {}", page_index + 1))?;
            let image = bitmap.as_image().to_rgb8();
            let (width, height) = image.dimensions();
            Ok(VisionPage {
                rgb: image.into_raw(),
                width: width as usize,
                height: height as usize,
                page_width_pts: w_pts,
                page_height_pts: h_pts,
            })
        })
    }

    pub fn export_page_png(
        &self,
        pdf_path: &Path,
        page_index: usize,
        destination: &Path,
        scale: f32,
    ) -> Result<()> {
        let rendered = self.render_page(pdf_path, page_index, scale)?;
        save_rgba_png(
            destination,
            rendered.width as u32,
            rendered.height as u32,
            rendered.rgba,
        )
    }

    pub fn close_document(&self, path: &Path) {
        self.open_documents
            .borrow_mut()
            .retain(|open| open.path != path);
    }

    fn with_open_document<T>(
        &self,
        path: &Path,
        operation: impl FnOnce(&PdfDocument<'static>) -> Result<T>,
    ) -> Result<T> {
        let mut open_documents = self.open_documents.borrow_mut();
        if let Some(position) = open_documents.iter().position(|open| open.path == path) {
            if position != 0 {
                if let Some(open_document) = open_documents.remove(position) {
                    open_documents.push_front(open_document);
                }
            }
        } else {
            let document = self
                .pdfium
                .load_pdf_from_file(path, None)
                .with_context(|| format!("failed to open {}", path.display()))?;
            open_documents.push_front(OpenPdfDocument {
                path: path.to_path_buf(),
                document,
            });
            while open_documents.len() > OPEN_DOCUMENT_CACHE_CAP {
                open_documents.pop_back();
            }
        }

        let open_document = open_documents
            .front()
            .ok_or_else(|| anyhow!("internal PDF cache is empty"))?;
        operation(&open_document.document)
    }
}

fn page_crop_box(page: &PdfPage<'_>) -> PdfRect {
    page.boundaries()
        .crop()
        .map(|box_| {
            PdfRect::new(
                box_.bounds.left().value,
                box_.bounds.bottom().value,
                box_.bounds.right().value,
                box_.bounds.top().value,
            )
        })
        .unwrap_or_else(|_| PdfRect::new(0.0, 0.0, page.width().value, page.height().value))
}

fn page_media_box(page: &PdfPage<'_>) -> PdfRect {
    page.boundaries()
        .media()
        .map(|box_| {
            PdfRect::new(
                box_.bounds.left().value,
                box_.bounds.bottom().value,
                box_.bounds.right().value,
                box_.bounds.top().value,
            )
        })
        .unwrap_or_else(|_| PdfRect::new(0.0, 0.0, page.width().value, page.height().value))
}

fn page_crop_box_if_distinct_from_media(page: &PdfPage<'_>) -> Option<PdfRect> {
    let crop_box = page_crop_box(page);
    (!pdf_rect_close(crop_box, page_media_box(page))).then_some(crop_box)
}

fn pdf_rect_close(left: PdfRect, right: PdfRect) -> bool {
    const EPSILON: f32 = 0.01;
    (left.left - right.left).abs() <= EPSILON
        && (left.bottom - right.bottom).abs() <= EPSILON
        && (left.right - right.right).abs() <= EPSILON
        && (left.top - right.top).abs() <= EPSILON
}

fn visible_page_extent(crop_extent: f32, fallback_extent: f32) -> f32 {
    if crop_extent.is_finite() && crop_extent > 0.0 {
        crop_extent
    } else {
        fallback_extent
    }
}

fn cropbox_local_coords_enabled() -> bool {
    std::env::var(LAWPDF_CROPBOX_LOCAL_COORDS_ENV)
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn crop_local_visible_rect(rect: PdfRect, crop_box: PdfRect) -> Option<PdfRect> {
    let left = (rect.left - crop_box.left).max(0.0);
    let bottom = (rect.bottom - crop_box.bottom).max(0.0);
    let right = (rect.right - crop_box.left).min(crop_box.width());
    let top = (rect.top - crop_box.bottom).min(crop_box.height());
    (right > left && top > bottom).then(|| PdfRect::new(left, bottom, right, top))
}

fn extract_text_chars(text_page: &PdfPageText<'_>) -> Vec<PageTextChar> {
    text_page
        .chars()
        .iter()
        .filter_map(|char| {
            let ch = char.unicode_char()?;
            if ch == '\0' {
                return None;
            }
            let rect = char
                .loose_bounds()
                .or_else(|_| char.tight_bounds())
                .ok()
                .map(|rect| {
                    PdfRect::new(
                        rect.left().value,
                        rect.bottom().value,
                        rect.right().value,
                        rect.top().value,
                    )
                })
                .filter(|rect| rect.width() > 0.0 && rect.height() > 0.0);

            let font_size = {
                let size = char.scaled_font_size().value;
                size.is_finite().then_some(size).filter(|size| *size > 0.0)
            };
            let font_name = char.font_name().to_ascii_lowercase();
            let bold = font_weight_is_bold(char.font_weight())
                || char.font_is_bold_reenforced()
                || font_name.contains("bold")
                || font_name.contains("black")
                || font_name.contains("semibold");
            let italic = char.font_is_italic()
                || font_name.contains("italic")
                || font_name.contains("oblique");

            Some(PageTextChar {
                ch,
                rect,
                font_size,
                bold,
                italic,
            })
        })
        .collect()
}

fn font_weight_is_bold(weight: Option<PdfFontWeight>) -> bool {
    matches!(
        weight,
        Some(
            PdfFontWeight::Weight600
                | PdfFontWeight::Weight700Bold
                | PdfFontWeight::Weight800
                | PdfFontWeight::Weight900
        )
    ) || matches!(weight, Some(PdfFontWeight::Custom(value)) if value >= 600)
}

fn detect_footnote_divider_y_from_top(
    page: &PdfPage<'_>,
    page_index: usize,
    crop_box: Option<PdfRect>,
    width: f32,
    height: f32,
) -> Option<f32> {
    let detector_mode = if page_index > 0 && wide_footnote_dividers_enabled() {
        FootnoteDividerDetectorMode::Wide
    } else {
        FootnoteDividerDetectorMode::Current
    };
    let mut candidates = Vec::new();
    for object in page.objects().iter() {
        let Some(_path) = object.as_path_object() else {
            continue;
        };
        let Ok(bounds) = object.bounds() else {
            continue;
        };
        let bounds = PdfRect::new(
            bounds.left().value,
            bounds.bottom().value,
            bounds.right().value,
            bounds.top().value,
        );
        let Some(bounds) = crop_box
            .map(|box_| crop_local_visible_rect(bounds, box_))
            .unwrap_or(Some(bounds))
        else {
            continue;
        };
        let left = bounds.left;
        let right = bounds.right;
        let bottom = bounds.bottom;
        let top = bounds.top;
        let line_width = (right - left).abs();
        let line_height = (top - bottom).abs();
        let y_from_top = height - ((top + bottom) * 0.5);
        if is_footnote_divider_candidate(
            left,
            line_width,
            line_height,
            y_from_top,
            width,
            height,
            detector_mode,
        ) {
            candidates.push((y_from_top, left));
        }
    }

    candidates.sort_by(|(y_a, x_a), (y_b, x_b)| {
        (x_a / width.max(1.0) - 0.12)
            .abs()
            .total_cmp(&(x_b / width.max(1.0) - 0.12).abs())
            .then_with(|| y_b.total_cmp(y_a))
    });
    candidates.first().map(|(y, _)| *y)
}

#[derive(Debug, Clone, Copy)]
enum FootnoteDividerDetectorMode {
    Current,
    Wide,
}

fn is_footnote_divider_candidate(
    left: f32,
    line_width: f32,
    line_height: f32,
    y_from_top: f32,
    page_width: f32,
    page_height: f32,
    mode: FootnoteDividerDetectorMode,
) -> bool {
    let page_width = page_width.max(1.0);
    let page_height = page_height.max(1.0);
    let y_ratio = y_from_top / page_height;
    let width_ratio = line_width / page_width;
    let left_ratio = left / page_width;
    match mode {
        FootnoteDividerDetectorMode::Current => {
            line_height <= 2.5
                && (0.45..=0.90).contains(&y_ratio)
                && (0.10..=0.55).contains(&width_ratio)
                && left_ratio <= 0.28
        }
        FootnoteDividerDetectorMode::Wide => {
            line_height <= 4.0
                && (0.42..=0.94).contains(&y_ratio)
                && (0.06..=0.92).contains(&width_ratio)
                && left_ratio <= 0.45
        }
    }
}

fn page_object_rects(
    page: &PdfPage<'_>,
    crop_box: Option<PdfRect>,
) -> (Vec<PdfRect>, Vec<PdfRect>, Vec<PdfRect>, Vec<PdfRect>) {
    let mut path_rects = Vec::new();
    let mut image_rects = Vec::new();
    let mut thin_horizontal = Vec::new();
    let mut thin_vertical = Vec::new();
    for object in page.objects().iter() {
        let is_path = object.as_path_object().is_some();
        let is_image = object.as_image_object().is_some();
        if !is_path && !is_image {
            continue;
        }
        let Ok(bounds) = object.bounds() else {
            continue;
        };
        let bounds = PdfRect::new(
            bounds.left().value,
            bounds.bottom().value,
            bounds.right().value,
            bounds.top().value,
        );
        let Some(bounds) = crop_box
            .map(|box_| crop_local_visible_rect(bounds, box_))
            .unwrap_or(Some(bounds))
        else {
            continue;
        };
        if is_path {
            let width = bounds.width();
            let height = bounds.height();
            if height <= 3.0 && width >= 8.0 {
                thin_horizontal.push(bounds);
            }
            if width <= 3.0 && height >= 8.0 {
                thin_vertical.push(bounds);
            }
            path_rects.push(bounds);
        } else if is_image {
            image_rects.push(bounds);
        }
    }
    (path_rects, image_rects, thin_horizontal, thin_vertical)
}

#[derive(Debug, Clone, Default)]
struct PageVectorRuleGeometry {
    horizontal_rules: Vec<PdfRect>,
    vertical_rules: Vec<PdfRect>,
    ruled_cells: Vec<PdfRect>,
}

impl PageVectorRuleGeometry {
    fn with_crop_box(self, crop_box: Option<PdfRect>) -> Self {
        let Some(crop_box) = crop_box else {
            return self;
        };
        Self {
            horizontal_rules: self
                .horizontal_rules
                .into_iter()
                .filter_map(|rect| crop_local_visible_rect(rect, crop_box))
                .collect(),
            vertical_rules: self
                .vertical_rules
                .into_iter()
                .filter_map(|rect| crop_local_visible_rect(rect, crop_box))
                .collect(),
            ruled_cells: self
                .ruled_cells
                .into_iter()
                .filter_map(|rect| crop_local_visible_rect(rect, crop_box))
                .filter(|rect| rect.width() >= 8.0 && rect.height() >= 6.0)
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct ContentMatrix {
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    e: f32,
    f: f32,
}

impl Default for ContentMatrix {
    fn default() -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 0.0,
            f: 0.0,
        }
    }
}

impl ContentMatrix {
    fn multiply(self, other: Self) -> Self {
        Self {
            a: self.a * other.a + self.c * other.b,
            b: self.b * other.a + self.d * other.b,
            c: self.a * other.c + self.c * other.d,
            d: self.b * other.c + self.d * other.d,
            e: self.a * other.e + self.c * other.f + self.e,
            f: self.b * other.e + self.d * other.f + self.f,
        }
    }

    fn transform_point(self, x: f32, y: f32) -> (f32, f32) {
        (
            self.a * x + self.c * y + self.e,
            self.b * x + self.d * y + self.f,
        )
    }
}

#[derive(Debug, Clone, Copy)]
struct VectorGraphicsState {
    ctm: ContentMatrix,
    line_width: f32,
}

impl Default for VectorGraphicsState {
    fn default() -> Self {
        Self {
            ctm: ContentMatrix::default(),
            line_width: 1.0,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct PathSegment {
    start: (f32, f32),
    end: (f32, f32),
    line_width: f32,
}

#[derive(Debug, Clone, Copy)]
struct PendingRect {
    rect: PdfRect,
    line_width: f32,
}

fn load_pdf_vector_rule_pages(
    source: &Path,
    page_count: usize,
) -> Result<Vec<PageVectorRuleGeometry>> {
    let document = Document::load(source)
        .with_context(|| format!("failed to load source PDF {}", source.display()))?;
    let pages = document.get_pages();
    let mut output = vec![PageVectorRuleGeometry::default(); page_count];
    for (page_number, page_id) in pages {
        let Some(slot) = output.get_mut(page_number.saturating_sub(1) as usize) else {
            continue;
        };
        let Ok(content) = document.get_and_decode_page_content(page_id) else {
            continue;
        };
        *slot = vector_rules_from_operations(&content.operations);
    }
    Ok(output)
}

fn vector_rules_from_operations(operations: &[Operation]) -> PageVectorRuleGeometry {
    let mut state = VectorGraphicsState::default();
    let mut stack = Vec::new();
    let mut current_point: Option<(f32, f32)> = None;
    let mut subpath_start: Option<(f32, f32)> = None;
    let mut segments = Vec::new();
    let mut rects = Vec::new();
    let mut geometry = PageVectorRuleGeometry::default();

    for operation in operations {
        match operation.operator.as_str() {
            "q" => stack.push(state),
            "Q" => {
                if let Some(previous) = stack.pop() {
                    state = previous;
                }
            }
            "cm" => {
                if operation.operands.len() >= 6
                    && let Some(matrix) = matrix_from_operands(&operation.operands)
                {
                    state.ctm = state.ctm.multiply(matrix);
                }
            }
            "w" => {
                if let Some(width) = operand_number(&operation.operands, 0) {
                    state.line_width = width.max(0.1);
                }
            }
            "m" => {
                if let Some(point) = transformed_point(&operation.operands, state.ctm) {
                    current_point = Some(point);
                    subpath_start = Some(point);
                }
            }
            "l" => {
                if let (Some(start), Some(end)) = (
                    current_point,
                    transformed_point(&operation.operands, state.ctm),
                ) {
                    segments.push(PathSegment {
                        start,
                        end,
                        line_width: state.line_width,
                    });
                    current_point = Some(end);
                }
            }
            "h" => {
                if let (Some(start), Some(end)) = (current_point, subpath_start) {
                    segments.push(PathSegment {
                        start,
                        end,
                        line_width: state.line_width,
                    });
                    current_point = Some(end);
                }
            }
            "re" => {
                if let Some(rect) = transformed_rect_from_operands(&operation.operands, state.ctm) {
                    let line_width = state.line_width.max(0.1);
                    rects.push(PendingRect { rect, line_width });
                    append_rect_segments(rect, line_width, &mut segments);
                }
            }
            "S" | "s" | "B" | "B*" | "b" | "b*" => {
                append_stroked_path_geometry(&mut geometry, &segments, &rects, true);
                segments.clear();
                rects.clear();
                current_point = None;
                subpath_start = None;
            }
            "f" | "f*" | "F" => {
                append_stroked_path_geometry(&mut geometry, &[], &rects, false);
                segments.clear();
                rects.clear();
                current_point = None;
                subpath_start = None;
            }
            "n" => {
                segments.clear();
                rects.clear();
                current_point = None;
                subpath_start = None;
            }
            _ => {}
        }
    }

    dedupe_rects(&mut geometry.horizontal_rules);
    dedupe_rects(&mut geometry.vertical_rules);
    dedupe_rects(&mut geometry.ruled_cells);
    geometry
}

fn append_stroked_path_geometry(
    geometry: &mut PageVectorRuleGeometry,
    segments: &[PathSegment],
    rects: &[PendingRect],
    stroked: bool,
) {
    if stroked {
        for segment in segments {
            append_axis_aligned_segment(geometry, *segment);
        }
    }
    for pending in rects {
        let rect = pending.rect;
        if stroked && rect.width() >= 8.0 && rect.height() >= 6.0 {
            geometry.ruled_cells.push(rect);
        }
        if stroked {
            append_rect_rule_edges(geometry, rect, pending.line_width);
        } else {
            append_thin_filled_rect_rule(geometry, rect);
        }
    }
}

fn append_axis_aligned_segment(geometry: &mut PageVectorRuleGeometry, segment: PathSegment) {
    let dx = (segment.end.0 - segment.start.0).abs();
    let dy = (segment.end.1 - segment.start.1).abs();
    let pad = (segment.line_width.max(0.5) * 0.5).max(0.25);
    if dx >= 8.0 && dy <= 0.75 {
        let rect = PdfRect::new(
            segment.start.0.min(segment.end.0),
            segment.start.1.min(segment.end.1) - pad,
            segment.start.0.max(segment.end.0),
            segment.start.1.max(segment.end.1) + pad,
        );
        geometry.horizontal_rules.push(rect);
    } else if dy >= 8.0 && dx <= 0.75 {
        let rect = PdfRect::new(
            segment.start.0.min(segment.end.0) - pad,
            segment.start.1.min(segment.end.1),
            segment.start.0.max(segment.end.0) + pad,
            segment.start.1.max(segment.end.1),
        );
        geometry.vertical_rules.push(rect);
    }
}

fn append_rect_rule_edges(geometry: &mut PageVectorRuleGeometry, rect: PdfRect, line_width: f32) {
    let pad = (line_width.max(0.5) * 0.5).max(0.25);
    if rect.width() >= 8.0 {
        geometry.horizontal_rules.push(PdfRect::new(
            rect.left,
            rect.bottom - pad,
            rect.right,
            rect.bottom + pad,
        ));
        geometry.horizontal_rules.push(PdfRect::new(
            rect.left,
            rect.top - pad,
            rect.right,
            rect.top + pad,
        ));
    }
    if rect.height() >= 8.0 {
        geometry.vertical_rules.push(PdfRect::new(
            rect.left - pad,
            rect.bottom,
            rect.left + pad,
            rect.top,
        ));
        geometry.vertical_rules.push(PdfRect::new(
            rect.right - pad,
            rect.bottom,
            rect.right + pad,
            rect.top,
        ));
    }
}

fn append_thin_filled_rect_rule(geometry: &mut PageVectorRuleGeometry, rect: PdfRect) {
    if rect.height() <= 3.0 && rect.width() >= 8.0 {
        geometry.horizontal_rules.push(rect);
    } else if rect.width() <= 3.0 && rect.height() >= 8.0 {
        geometry.vertical_rules.push(rect);
    }
}

fn append_rect_segments(rect: PdfRect, line_width: f32, segments: &mut Vec<PathSegment>) {
    let bottom_left = (rect.left, rect.bottom);
    let bottom_right = (rect.right, rect.bottom);
    let top_right = (rect.right, rect.top);
    let top_left = (rect.left, rect.top);
    segments.push(PathSegment {
        start: bottom_left,
        end: bottom_right,
        line_width,
    });
    segments.push(PathSegment {
        start: bottom_right,
        end: top_right,
        line_width,
    });
    segments.push(PathSegment {
        start: top_right,
        end: top_left,
        line_width,
    });
    segments.push(PathSegment {
        start: top_left,
        end: bottom_left,
        line_width,
    });
}

fn transformed_point(operands: &[Object], ctm: ContentMatrix) -> Option<(f32, f32)> {
    let x = operand_number(operands, 0)?;
    let y = operand_number(operands, 1)?;
    Some(ctm.transform_point(x, y))
}

fn transformed_rect_from_operands(operands: &[Object], ctm: ContentMatrix) -> Option<PdfRect> {
    let x = operand_number(operands, 0)?;
    let y = operand_number(operands, 1)?;
    let width = operand_number(operands, 2)?;
    let height = operand_number(operands, 3)?;
    let points = [
        ctm.transform_point(x, y),
        ctm.transform_point(x + width, y),
        ctm.transform_point(x + width, y + height),
        ctm.transform_point(x, y + height),
    ];
    Some(rect_from_points(&points))
}

fn rect_from_points(points: &[(f32, f32); 4]) -> PdfRect {
    let mut left = f32::INFINITY;
    let mut bottom = f32::INFINITY;
    let mut right = f32::NEG_INFINITY;
    let mut top = f32::NEG_INFINITY;
    for (x, y) in points {
        left = left.min(*x);
        bottom = bottom.min(*y);
        right = right.max(*x);
        top = top.max(*y);
    }
    PdfRect::new(left, bottom, right, top)
}

fn matrix_from_operands(operands: &[Object]) -> Option<ContentMatrix> {
    Some(ContentMatrix {
        a: operand_number(operands, 0)?,
        b: operand_number(operands, 1)?,
        c: operand_number(operands, 2)?,
        d: operand_number(operands, 3)?,
        e: operand_number(operands, 4)?,
        f: operand_number(operands, 5)?,
    })
}

fn operand_number(operands: &[Object], index: usize) -> Option<f32> {
    operands
        .get(index)
        .and_then(|object| object.as_float().ok())
}

fn dedupe_rects(rects: &mut Vec<PdfRect>) {
    rects.sort_by(|left, right| {
        quantize_rect(*left)
            .partial_cmp(&quantize_rect(*right))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    rects.dedup_by(|left, right| rects_close(*left, *right, 0.25));
}

fn quantize_rect(rect: PdfRect) -> (i32, i32, i32, i32) {
    (
        (rect.left * 4.0).round() as i32,
        (rect.bottom * 4.0).round() as i32,
        (rect.right * 4.0).round() as i32,
        (rect.top * 4.0).round() as i32,
    )
}

fn rects_close(left: PdfRect, right: PdfRect, tolerance: f32) -> bool {
    (left.left - right.left).abs() <= tolerance
        && (left.bottom - right.bottom).abs() <= tolerance
        && (left.right - right.right).abs() <= tolerance
        && (left.top - right.top).abs() <= tolerance
}

fn wide_footnote_dividers_enabled() -> bool {
    let explicit = std::env::var(LAWPDF_WIDE_FOOTNOTE_DIVIDERS_ENV)
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false);
    explicit
        || std::env::var("LAWPDF_LM2_RUNTIME_PRESET")
            .ok()
            .is_some_and(|value| {
                value.eq_ignore_ascii_case(
                    "v25-d1-sandwiched-note-start-wide-sandwich-postcue-citation-next1-near8cue-wide-divider-guard",
                )
            })
}

fn bind_pdfium() -> Result<Box<dyn PdfiumLibraryBindings>> {
    for candidate in pdfium_candidates() {
        if candidate.exists() {
            return Pdfium::bind_to_library(candidate.to_string_lossy().to_string())
                .with_context(|| format!("failed to bind PDFium from {}", candidate.display()));
        }
    }

    Pdfium::bind_to_system_library().context(
        "failed to bind PDFium; put the PDFium dynamic library beside the executable, in vendor/, on the system library path, or set PDFIUM_DYNAMIC_LIB_PATH",
    )
}

fn pdfium_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let library_names = pdfium_library_names();

    if let Ok(path) = std::env::var("PDFIUM_DYNAMIC_LIB_PATH") {
        if !path.trim().is_empty() {
            candidates.push(PathBuf::from(path));
        }
    }

    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            push_pdfium_names(&mut candidates, exe_dir, &library_names);
            if cfg!(target_os = "macos")
                && exe_dir
                    .file_name()
                    .is_some_and(|name| name == std::ffi::OsStr::new("MacOS"))
            {
                if let Some(contents_dir) = exe_dir.parent() {
                    push_pdfium_names(
                        &mut candidates,
                        &contents_dir.join("Frameworks"),
                        &library_names,
                    );
                    push_pdfium_names(
                        &mut candidates,
                        &contents_dir.join("Resources"),
                        &library_names,
                    );
                }
            }
        }
    }

    if let Ok(current_dir) = std::env::current_dir() {
        push_pdfium_names(&mut candidates, &current_dir, &library_names);
        push_pdfium_names(&mut candidates, &current_dir.join("vendor"), &library_names);
    }

    push_pdfium_names(
        &mut candidates,
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("vendor"),
        &library_names,
    );
    candidates
}

fn push_pdfium_names(candidates: &mut Vec<PathBuf>, dir: &Path, names: &[&str]) {
    for name in names {
        candidates.push(dir.join(name));
    }
}

fn pdfium_library_names() -> Vec<&'static str> {
    if cfg!(target_os = "windows") {
        vec!["pdfium.dll"]
    } else if cfg!(target_os = "macos") {
        vec!["libpdfium.dylib", "pdfium.dylib"]
    } else {
        vec!["libpdfium.so", "pdfium.so"]
    }
}

pub fn save_rgba_png(path: &Path, width: u32, height: u32, rgba: Vec<u8>) -> Result<()> {
    let image = RgbaImage::from_raw(width, height, rgba)
        .ok_or_else(|| anyhow!("rendered page buffer has invalid dimensions"))?;
    image
        .save(path)
        .with_context(|| format!("failed to save PNG {}", path.display()))
}

pub fn export_text(path: &Path, document: &LoadedDocument, ocr_text: &[String]) -> Result<()> {
    let mut output = String::new();

    for page_index in 0..document.page_count {
        output.push_str(&format!("--- Page {} ---\n", page_index + 1));

        let native = document
            .native_text
            .get(page_index)
            .map(String::as_str)
            .unwrap_or_default()
            .trim();
        if !native.is_empty() {
            output.push_str(native);
            output.push('\n');
        }

        let ocr = ocr_text
            .get(page_index)
            .map(String::as_str)
            .unwrap_or_default()
            .trim();
        if !ocr.is_empty() && ocr != native {
            output.push_str("\n[OCR]\n");
            output.push_str(ocr);
            output.push('\n');
        }

        output.push('\n');
    }

    fs::write(path, output).with_context(|| format!("failed to write {}", path.display()))
}

pub fn save_with_annotations(
    source: &Path,
    destination: &Path,
    annotations: &[EditorAnnotation],
) -> Result<()> {
    let mut document = Document::load(source)
        .with_context(|| format!("failed to load source PDF {}", source.display()))?;
    remove_lawpdf_owned_annotations(&mut document)?;
    let pages = document.get_pages();

    for annotation in annotations {
        let page_number = annotation.page_index as u32 + 1;
        let page_id = *pages
            .get(&page_number)
            .ok_or_else(|| anyhow!("PDF has no page {}", page_number))?;
        let annotation_id = document.new_object_id();
        let object = annotation_to_pdf_object(annotation);
        document.objects.insert(annotation_id, object);
        append_annotation(&mut document, page_id, annotation_id)?;
    }

    document.prune_objects();
    document.compress();
    if source == destination {
        save_document_in_place(&mut document, destination)?;
    } else {
        document
            .save(destination)
            .with_context(|| format!("failed to save {}", destination.display()))?;
    }

    Ok(())
}

pub fn load_pdf_web_links(source: &Path, page_count: usize) -> Result<Vec<Vec<PageLink>>> {
    let document = Document::load(source)
        .with_context(|| format!("failed to load source PDF {}", source.display()))?;
    let pages = document.get_pages();
    let mut links = vec![Vec::new(); page_count];

    for (page_number, page_id) in pages {
        let page_index = page_number.saturating_sub(1) as usize;
        if page_index >= links.len() {
            continue;
        }
        let annots = {
            let page = document.get_object(page_id)?.as_dict()?;
            page.get(b"Annots").ok().cloned()
        };
        let Some(annots) = annots else {
            continue;
        };

        append_pdf_web_links_from_annots(&document, &annots, &mut links[page_index]);
    }

    Ok(links)
}

pub fn load_lawpdf_annotations(source: &Path) -> Result<Vec<EditorAnnotation>> {
    let document = Document::load(source)
        .with_context(|| format!("failed to load source PDF {}", source.display()))?;
    let pages = document.get_pages();
    let mut annotations = Vec::new();

    for (page_number, page_id) in pages {
        let page_index = page_number.saturating_sub(1) as usize;
        let annots = {
            let page = document.get_object(page_id)?.as_dict()?;
            page.get(b"Annots").ok().cloned()
        };
        let Some(annots) = annots else {
            continue;
        };

        match annots {
            Object::Array(annots) => {
                for annot in &annots {
                    if let Some(annotation) =
                        lawpdf_owned_annotation_from_pdf(&document, annot, page_index)
                    {
                        annotations.push(annotation);
                    }
                }
            }
            Object::Reference(annots_id) => {
                if let Ok(annots) = document.get_object(annots_id).and_then(Object::as_array) {
                    for annot in annots {
                        if let Some(annotation) =
                            lawpdf_owned_annotation_from_pdf(&document, annot, page_index)
                        {
                            annotations.push(annotation);
                        }
                    }
                }
            }
            annot => {
                if let Some(annotation) =
                    lawpdf_owned_annotation_from_pdf(&document, &annot, page_index)
                {
                    annotations.push(annotation);
                }
            }
        }
    }

    Ok(annotations)
}

#[cfg(test)]
fn load_lawpdf_comments(source: &Path) -> Result<Vec<EditorAnnotation>> {
    Ok(load_lawpdf_annotations(source)?
        .into_iter()
        .filter(|annotation| matches!(annotation.kind, AnnotationKind::Comment { .. }))
        .collect())
}

pub fn sync_lawpdf_comments(source: &Path, comments: &[EditorAnnotation]) -> Result<usize> {
    let mut document = Document::load(source)
        .with_context(|| format!("failed to load source PDF {}", source.display()))?;
    remove_lawpdf_comment_annotations(&mut document)?;
    let pages = document.get_pages();
    let mut saved = 0usize;

    for annotation in comments {
        if !matches!(annotation.kind, AnnotationKind::Comment { .. }) {
            continue;
        }

        let page_number = annotation.page_index as u32 + 1;
        let Some(page_id) = pages.get(&page_number).copied() else {
            continue;
        };
        let annotation_id = document.new_object_id();
        let object = annotation_to_pdf_object(annotation);
        document.objects.insert(annotation_id, object);
        append_annotation(&mut document, page_id, annotation_id)?;
        saved += 1;
    }

    document.prune_objects();
    document.compress();
    save_document_in_place(&mut document, source)?;

    Ok(saved)
}

pub fn save_with_ocr_text(
    source: &Path,
    destination: &Path,
    page_sizes: &[(f32, f32)],
    ocr_text: &[String],
) -> Result<()> {
    let mut document = Document::load(source)
        .with_context(|| format!("failed to load source PDF {}", source.display()))?;
    append_ocr_text_layers(&mut document, page_sizes, ocr_text)?;
    document.prune_objects();
    document.compress();
    document
        .save(destination)
        .with_context(|| format!("failed to save {}", destination.display()))?;

    Ok(())
}

fn append_ocr_text_layers(
    document: &mut Document,
    page_sizes: &[(f32, f32)],
    ocr_text: &[String],
) -> Result<()> {
    let pages = document.get_pages();
    let font_id = document.new_object_id();
    document.objects.insert(
        font_id,
        Object::Dictionary(dictionary! {
            "Type" => Object::Name(b"Font".to_vec()),
            "Subtype" => Object::Name(b"Type1".to_vec()),
            "BaseFont" => Object::Name(b"Helvetica".to_vec()),
            "Encoding" => Object::Name(b"WinAnsiEncoding".to_vec()),
        }),
    );

    for (page_index, text) in ocr_text.iter().enumerate() {
        if text.trim().is_empty() {
            continue;
        }
        let page_number = page_index as u32 + 1;
        let Some(page_id) = pages.get(&page_number).copied() else {
            continue;
        };
        let page_size = page_sizes
            .get(page_index)
            .copied()
            .unwrap_or((612.0, 792.0));
        ensure_ocr_font_resource(document, page_id, font_id)?;
        let content = ocr_text_content(text, page_size)?;
        let stream_id = document.add_object(Stream::new(dictionary! {}, content));
        append_page_content_stream(document, page_id, stream_id)?;
    }

    Ok(())
}

fn ensure_ocr_font_resource(
    document: &mut Document,
    page_id: ObjectId,
    font_id: ObjectId,
) -> Result<()> {
    let resources = {
        let page = document.get_object_mut(page_id)?.as_dict_mut()?;
        page.get(b"Resources").ok().cloned()
    };

    match resources {
        Some(Object::Reference(resources_id)) => {
            let font_object = document
                .get_object(resources_id)?
                .as_dict()?
                .get(b"Font")
                .ok()
                .cloned();
            match font_object {
                Some(Object::Reference(fonts_id)) => {
                    document
                        .get_object_mut(fonts_id)?
                        .as_dict_mut()?
                        .set("LawPDFOCR", Object::Reference(font_id));
                }
                Some(Object::Dictionary(mut fonts)) => {
                    fonts.set("LawPDFOCR", Object::Reference(font_id));
                    document
                        .get_object_mut(resources_id)?
                        .as_dict_mut()?
                        .set("Font", Object::Dictionary(fonts));
                }
                _ => {
                    document
                        .get_object_mut(resources_id)?
                        .as_dict_mut()?
                        .set("Font", ocr_font_dictionary(font_id));
                }
            }
        }
        Some(Object::Dictionary(mut resources)) => {
            ensure_ocr_font_in_resources(document, &mut resources, font_id)?;
            document
                .get_object_mut(page_id)?
                .as_dict_mut()?
                .set("Resources", Object::Dictionary(resources));
        }
        _ => {
            document.get_object_mut(page_id)?.as_dict_mut()?.set(
                "Resources",
                Object::Dictionary(dictionary! {
                    "Font" => ocr_font_dictionary(font_id)
                }),
            );
        }
    }

    Ok(())
}

fn ensure_ocr_font_in_resources(
    document: &mut Document,
    resources: &mut Dictionary,
    font_id: ObjectId,
) -> Result<()> {
    match resources.get(b"Font").ok().cloned() {
        Some(Object::Reference(fonts_id)) => {
            document
                .get_object_mut(fonts_id)?
                .as_dict_mut()?
                .set("LawPDFOCR", Object::Reference(font_id));
        }
        Some(Object::Dictionary(mut fonts)) => {
            fonts.set("LawPDFOCR", Object::Reference(font_id));
            resources.set("Font", Object::Dictionary(fonts));
        }
        _ => {
            resources.set("Font", ocr_font_dictionary(font_id));
        }
    }
    Ok(())
}

fn ocr_font_dictionary(font_id: ObjectId) -> Object {
    Object::Dictionary(dictionary! {
        "LawPDFOCR" => Object::Reference(font_id)
    })
}

fn ocr_text_content(text: &str, page_size: (f32, f32)) -> Result<Vec<u8>> {
    let (_, page_height) = page_size;
    let mut operations = vec![
        Operation::new("BT", vec![]),
        Operation::new(
            "Tf",
            vec![Object::Name(b"LawPDFOCR".to_vec()), Object::Real(8.0)],
        ),
        Operation::new("TL", vec![Object::Real(9.6)]),
        Operation::new("Tr", vec![Object::Integer(3)]),
        Operation::new(
            "Tm",
            vec![
                Object::Real(1.0),
                Object::Real(0.0),
                Object::Real(0.0),
                Object::Real(1.0),
                Object::Real(36.0),
                Object::Real((page_height - 36.0).max(36.0)),
            ],
        ),
    ];

    for line in ocr_pdf_lines(text) {
        operations.push(Operation::new(
            "Tj",
            vec![literal(sanitize_pdf_text(&line))],
        ));
        operations.push(Operation::new("T*", vec![]));
    }
    operations.push(Operation::new("ET", vec![]));

    Content { operations }
        .encode()
        .context("failed to encode OCR text layer")
}

fn ocr_pdf_lines(text: &str) -> Vec<String> {
    text.lines()
        .flat_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return vec![String::new()];
            }
            line.chars()
                .collect::<Vec<_>>()
                .chunks(96)
                .map(|chunk| chunk.iter().collect::<String>())
                .collect::<Vec<_>>()
        })
        .collect()
}

fn sanitize_pdf_text(text: &str) -> String {
    text.chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect::<String>()
}

fn append_page_content_stream(
    document: &mut Document,
    page_id: ObjectId,
    stream_id: ObjectId,
) -> Result<()> {
    let existing = {
        let page = document.get_object(page_id)?.as_dict()?;
        page.get(b"Contents").ok().cloned()
    };

    let contents = match existing {
        Some(Object::Array(mut array)) => {
            array.push(Object::Reference(stream_id));
            Object::Array(array)
        }
        Some(Object::Reference(existing_id)) => Object::Array(vec![
            Object::Reference(existing_id),
            Object::Reference(stream_id),
        ]),
        Some(other) => Object::Array(vec![other, Object::Reference(stream_id)]),
        None => Object::Reference(stream_id),
    };

    document
        .get_object_mut(page_id)?
        .as_dict_mut()?
        .set("Contents", contents);
    Ok(())
}

fn annotation_to_pdf_object(annotation: &EditorAnnotation) -> Object {
    match &annotation.kind {
        AnnotationKind::Marker {
            color_rgb,
            opacity,
            style,
        } => {
            let rect = rect_array(annotation.rect);
            let quad_points = quad_points(annotation.rect);
            let subtype = match style {
                MarkerStyle::Highlight => b"Highlight".to_vec(),
                MarkerStyle::Underline => b"Underline".to_vec(),
            };
            let contents = match style {
                MarkerStyle::Highlight => "Highlighted text",
                MarkerStyle::Underline => "Underlined text",
            };
            Object::Dictionary(dictionary! {
                "Type" => Object::Name(b"Annot".to_vec()),
                "Subtype" => Object::Name(subtype),
                "Rect" => rect,
                "QuadPoints" => quad_points,
                "C" => color_array(*color_rgb),
                "CA" => Object::Real(*opacity),
                "Contents" => literal(contents),
                "LawPDF" => Object::Boolean(true),
                "F" => Object::Integer(4),
            })
        }
        AnnotationKind::TextBox {
            text,
            font_size,
            color_rgb,
        } => Object::Dictionary(dictionary! {
            "Type" => Object::Name(b"Annot".to_vec()),
            "Subtype" => Object::Name(b"FreeText".to_vec()),
            "Rect" => rect_array(annotation.rect),
            "Contents" => literal(text),
            "DA" => literal(format!("/Helv {font_size} Tf 0 0 0 rg")),
            "C" => color_array(*color_rgb),
            "F" => Object::Integer(4),
        }),
        AnnotationKind::Comment {
            id,
            text,
            color_rgb,
            updated_at,
            anchor,
            ..
        } => Object::Dictionary(dictionary! {
            "Type" => Object::Name(b"Annot".to_vec()),
            "Subtype" => Object::Name(b"Text".to_vec()),
            "Rect" => rect_array(annotation.rect),
            "Contents" => literal(text),
            "T" => literal("LawPDF"),
            "NM" => literal(id),
            "M" => literal(updated_at),
            "Name" => Object::Name(b"Comment".to_vec()),
            "Open" => Object::Boolean(false),
            "C" => color_array(*color_rgb),
            // Private key: the on-text anchor point for the dotted leader.
            "LawA" => Object::Array(vec![Object::Real(anchor.0), Object::Real(anchor.1)]),
            "LawPDF" => Object::Boolean(true),
            "F" => Object::Integer(4),
        }),
        AnnotationKind::Signature {
            signer,
            signed_at,
            strokes,
        } => {
            let mut contents = format!("Signed by {signer}");
            if !signed_at.trim().is_empty() {
                contents.push_str(&format!(" at {signed_at}"));
            }

            Object::Dictionary(dictionary! {
                "Type" => Object::Name(b"Annot".to_vec()),
                "Subtype" => Object::Name(b"Ink".to_vec()),
                "Rect" => rect_array(annotation.rect),
                "InkList" => ink_list(strokes),
                "C" => color_array([0.0, 0.0, 0.0]),
                "Border" => Object::Array(vec![0.into(), 0.into(), 1.into()]),
                "Contents" => literal(contents),
                "F" => Object::Integer(4),
            })
        }
    }
}

fn lawpdf_comment_from_annotation(
    document: &Document,
    annotation: &Object,
    page_index: usize,
) -> Option<EditorAnnotation> {
    let dict = annotation_dict(document, annotation)?;
    if !is_lawpdf_comment_dict(dict) {
        return None;
    }

    let id = dict.get(b"NM").ok().and_then(pdf_object_text)?;
    let text = dict
        .get(b"Contents")
        .ok()
        .and_then(pdf_object_text)
        .unwrap_or_default();
    let rect = dict.get(b"Rect").ok().and_then(pdf_rect_from_object)?;
    let color_rgb = dict
        .get(b"C")
        .ok()
        .and_then(pdf_color_from_object)
        .unwrap_or([1.0, 0.86, 0.32]);
    let updated_at = dict
        .get(b"M")
        .ok()
        .and_then(pdf_object_text)
        .unwrap_or_default();
    let created_at = updated_at.clone();
    // Older files (and non-LawPDF readers) won't have the anchor; fall back to
    // the card's center so the leader still points somewhere sensible.
    let anchor = dict
        .get(b"LawA")
        .ok()
        .and_then(pdf_point_from_object)
        .unwrap_or((
            (rect.left + rect.right) * 0.5,
            (rect.top + rect.bottom) * 0.5,
        ));

    Some(EditorAnnotation {
        page_index,
        rect,
        kind: AnnotationKind::Comment {
            id,
            text,
            color_rgb,
            created_at,
            updated_at,
            anchor,
        },
    })
}

fn lawpdf_owned_annotation_from_pdf(
    document: &Document,
    annotation: &Object,
    page_index: usize,
) -> Option<EditorAnnotation> {
    let dict = annotation_dict(document, annotation)?;
    if is_lawpdf_comment_dict(dict) {
        return lawpdf_comment_from_annotation(document, annotation, page_index);
    }
    if !is_lawpdf_owned_dict(dict) {
        return None;
    }
    let subtype = dict.get(b"Subtype").ok().and_then(pdf_object_text)?;
    let style = if subtype.eq_ignore_ascii_case("Highlight") {
        MarkerStyle::Highlight
    } else if subtype.eq_ignore_ascii_case("Underline") {
        MarkerStyle::Underline
    } else {
        return None;
    };
    let rect = dict.get(b"Rect").ok().and_then(pdf_rect_from_object)?;
    let color_rgb = dict
        .get(b"C")
        .ok()
        .and_then(pdf_color_from_object)
        .unwrap_or([1.0, 0.93, 0.45]);
    let opacity = dict
        .get(b"CA")
        .ok()
        .and_then(pdf_number)
        .unwrap_or(0.42)
        .clamp(0.0, 1.0);
    Some(EditorAnnotation {
        page_index,
        rect,
        kind: AnnotationKind::Marker {
            color_rgb,
            opacity,
            style,
        },
    })
}

fn append_pdf_web_links_from_annots(
    document: &Document,
    annots: &Object,
    page_links: &mut Vec<PageLink>,
) {
    match annots {
        Object::Array(annots) => {
            for annot in annots {
                if let Some(link) = pdf_web_link_from_annotation(document, annot) {
                    page_links.push(link);
                }
            }
        }
        Object::Reference(annots_id) => {
            if let Ok(annots) = document.get_object(*annots_id).and_then(Object::as_array) {
                for annot in annots {
                    if let Some(link) = pdf_web_link_from_annotation(document, annot) {
                        page_links.push(link);
                    }
                }
            }
        }
        annot => {
            if let Some(link) = pdf_web_link_from_annotation(document, annot) {
                page_links.push(link);
            }
        }
    }
}

fn pdf_web_link_from_annotation(document: &Document, annotation: &Object) -> Option<PageLink> {
    let dict = annotation_dict(document, annotation)?;
    let subtype = dict.get(b"Subtype").ok().and_then(pdf_object_text)?;
    if !subtype.eq_ignore_ascii_case("Link") {
        return None;
    }

    let rect = dict.get(b"Rect").ok().and_then(pdf_rect_from_object)?;
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return None;
    }

    let uri = dict
        .get(b"A")
        .ok()
        .and_then(|action| pdf_uri_from_action(document, action))?;
    let url = normalize_web_url(&uri)?;

    Some(PageLink { rect, url })
}

fn pdf_uri_from_action(document: &Document, action: &Object) -> Option<String> {
    let dict = annotation_dict(document, action)?;
    let action_type = dict.get(b"S").ok().and_then(pdf_object_text)?;
    if !action_type.eq_ignore_ascii_case("URI") {
        return None;
    }

    dict.get(b"URI").ok().and_then(pdf_object_text)
}

fn normalize_web_url(uri: &str) -> Option<String> {
    let trimmed = uri.trim().trim_matches(char::from(0));
    if trimmed.is_empty()
        || trimmed
            .chars()
            .any(|ch| ch.is_control() || ch.is_whitespace())
    {
        return None;
    }

    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        Some(trimmed.to_owned())
    } else if lower.starts_with("www.") {
        Some(format!("https://{trimmed}"))
    } else {
        None
    }
}

fn remove_lawpdf_comment_annotations(document: &mut Document) -> Result<usize> {
    remove_lawpdf_annotations_matching(document, false)
}

fn remove_lawpdf_owned_annotations(document: &mut Document) -> Result<usize> {
    remove_lawpdf_annotations_matching(document, true)
}

fn remove_lawpdf_annotations_matching(
    document: &mut Document,
    include_owned: bool,
) -> Result<usize> {
    let pages = document.get_pages();
    let mut removed = 0usize;
    let mut removed_object_ids = Vec::new();

    for page_id in pages.values().copied() {
        let annots = {
            let page = document.get_object(page_id)?.as_dict()?;
            page.get(b"Annots").ok().cloned()
        };
        let Some(annots) = annots else {
            continue;
        };

        match annots {
            Object::Array(annots) => {
                let filtered = filter_lawpdf_comment_annots(
                    document,
                    annots,
                    &mut removed_object_ids,
                    &mut removed,
                    include_owned,
                );
                document
                    .get_object_mut(page_id)?
                    .as_dict_mut()?
                    .set("Annots", Object::Array(filtered));
            }
            Object::Reference(annots_id) => {
                let annots = document
                    .get_object(annots_id)?
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                let filtered = filter_lawpdf_comment_annots(
                    document,
                    annots,
                    &mut removed_object_ids,
                    &mut removed,
                    include_owned,
                );
                let annots_array = document.get_object_mut(annots_id)?.as_array_mut()?;
                annots_array.clear();
                annots_array.extend(filtered);
            }
            annot => {
                if is_matching_lawpdf_annotation(document, &annot, include_owned) {
                    document
                        .get_object_mut(page_id)?
                        .as_dict_mut()?
                        .set("Annots", Object::Array(Vec::new()));
                    removed += 1;
                }
            }
        }
    }

    for id in removed_object_ids {
        document.objects.remove(&id);
    }

    Ok(removed)
}

fn filter_lawpdf_comment_annots(
    document: &Document,
    annots: Vec<Object>,
    removed_object_ids: &mut Vec<ObjectId>,
    removed: &mut usize,
    include_owned: bool,
) -> Vec<Object> {
    let mut kept = Vec::with_capacity(annots.len());
    for annot in annots {
        if is_matching_lawpdf_annotation(document, &annot, include_owned) {
            if let Object::Reference(id) = annot {
                removed_object_ids.push(id);
            }
            *removed += 1;
        } else {
            kept.push(annot);
        }
    }
    kept
}

fn is_matching_lawpdf_annotation(
    document: &Document,
    annotation: &Object,
    include_owned: bool,
) -> bool {
    annotation_dict(document, annotation).is_some_and(|dict| {
        is_lawpdf_comment_dict(dict) || (include_owned && is_lawpdf_owned_dict(dict))
    })
}

fn is_lawpdf_comment_dict(dict: &Dictionary) -> bool {
    dict.get(b"NM")
        .ok()
        .and_then(pdf_object_text)
        .is_some_and(|id| id.starts_with(LAWPDF_COMMENT_ID_PREFIX))
}

fn is_lawpdf_owned_dict(dict: &Dictionary) -> bool {
    matches!(dict.get(b"LawPDF"), Ok(Object::Boolean(true))) || is_lawpdf_comment_dict(dict)
}

fn annotation_dict<'a>(document: &'a Document, annotation: &'a Object) -> Option<&'a Dictionary> {
    match annotation {
        Object::Dictionary(dict) => Some(dict),
        Object::Reference(id) => document.get_object(*id).ok()?.as_dict().ok(),
        _ => None,
    }
}

fn append_annotation(
    document: &mut Document,
    page_id: ObjectId,
    annotation_id: ObjectId,
) -> Result<()> {
    let existing_annots = {
        let page = document.get_object(page_id)?.as_dict()?;
        page.get(b"Annots").ok().cloned()
    };

    match existing_annots {
        Some(Object::Array(mut annots)) => {
            annots.push(Object::Reference(annotation_id));
            document
                .get_object_mut(page_id)?
                .as_dict_mut()?
                .set("Annots", Object::Array(annots));
        }
        Some(Object::Reference(annots_id)) => {
            let annots = document.get_object_mut(annots_id)?.as_array_mut()?;
            annots.push(Object::Reference(annotation_id));
        }
        _ => {
            document.get_object_mut(page_id)?.as_dict_mut()?.set(
                "Annots",
                Object::Array(vec![Object::Reference(annotation_id)]),
            );
        }
    }

    Ok(())
}

fn rect_array(rect: PdfRect) -> Object {
    Object::Array(vec![
        Object::Real(rect.left),
        Object::Real(rect.bottom),
        Object::Real(rect.right),
        Object::Real(rect.top),
    ])
}

fn quad_points(rect: PdfRect) -> Object {
    Object::Array(vec![
        Object::Real(rect.left),
        Object::Real(rect.top),
        Object::Real(rect.right),
        Object::Real(rect.top),
        Object::Real(rect.left),
        Object::Real(rect.bottom),
        Object::Real(rect.right),
        Object::Real(rect.bottom),
    ])
}

fn color_array(rgb: [f32; 3]) -> Object {
    Object::Array(vec![
        Object::Real(rgb[0]),
        Object::Real(rgb[1]),
        Object::Real(rgb[2]),
    ])
}

fn pdf_object_text(object: &Object) -> Option<String> {
    match object {
        Object::String(bytes, _) => Some(String::from_utf8_lossy(bytes).to_string()),
        Object::Name(bytes) => Some(String::from_utf8_lossy(bytes).to_string()),
        _ => None,
    }
}

fn pdf_rect_from_object(object: &Object) -> Option<PdfRect> {
    let values = object.as_array().ok()?;
    if values.len() < 4 {
        return None;
    }
    Some(PdfRect::new(
        pdf_number(&values[0])?,
        pdf_number(&values[1])?,
        pdf_number(&values[2])?,
        pdf_number(&values[3])?,
    ))
}

fn pdf_point_from_object(object: &Object) -> Option<(f32, f32)> {
    let values = object.as_array().ok()?;
    if values.len() < 2 {
        return None;
    }
    Some((pdf_number(&values[0])?, pdf_number(&values[1])?))
}

fn pdf_color_from_object(object: &Object) -> Option<[f32; 3]> {
    let values = object.as_array().ok()?;
    if values.len() < 3 {
        return None;
    }
    Some([
        pdf_number(&values[0])?.clamp(0.0, 1.0),
        pdf_number(&values[1])?.clamp(0.0, 1.0),
        pdf_number(&values[2])?.clamp(0.0, 1.0),
    ])
}

fn pdf_number(object: &Object) -> Option<f32> {
    match object {
        Object::Integer(value) => Some(*value as f32),
        Object::Real(value) => Some(*value),
        _ => None,
    }
}

fn ink_list(strokes: &[Vec<(f32, f32)>]) -> Object {
    Object::Array(
        strokes
            .iter()
            .map(|stroke| {
                Object::Array(
                    stroke
                        .iter()
                        .flat_map(|(x, y)| [Object::Real(*x), Object::Real(*y)])
                        .collect(),
                )
            })
            .collect(),
    )
}

fn literal(value: impl AsRef<str>) -> Object {
    Object::String(value.as_ref().as_bytes().to_vec(), StringFormat::Literal)
}

fn save_document_in_place(document: &mut Document, destination: &Path) -> Result<()> {
    let temp = temp_pdf_path(destination);
    if temp.exists() {
        let _ = fs::remove_file(&temp);
    }

    document
        .save(&temp)
        .with_context(|| format!("failed to write temporary PDF {}", temp.display()))?;
    fs::copy(&temp, destination)
        .with_context(|| format!("failed to replace {}", destination.display()))?;
    let _ = fs::remove_file(&temp);
    Ok(())
}

fn temp_pdf_path(destination: &Path) -> PathBuf {
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    let file_name = destination
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("document.pdf");
    parent.join(format!(
        ".{file_name}.{}.lawpdf-comments.tmp",
        std::process::id()
    ))
}

pub fn sidecar_path_for_export(source: &Path, suffix: &str, extension: &str) -> PathBuf {
    let stem = source
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("document");
    source.with_file_name(format!("{stem}-{suffix}.{extension}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vector_rules_from_operations_extracts_stroked_rect_cells() {
        let operations = vec![
            Operation::new(
                "re",
                vec![
                    Object::from(100),
                    Object::from(200),
                    Object::from(160),
                    Object::from(40),
                ],
            ),
            Operation::new("S", vec![]),
        ];

        let geometry = vector_rules_from_operations(&operations);

        assert_eq!(
            geometry.ruled_cells,
            vec![PdfRect::new(100.0, 200.0, 260.0, 240.0)]
        );
        assert_eq!(geometry.horizontal_rules.len(), 2);
        assert_eq!(geometry.vertical_rules.len(), 2);
    }

    #[test]
    fn vector_rules_from_operations_applies_ctm_to_line_segments() {
        let operations = vec![
            Operation::new(
                "cm",
                vec![
                    Object::from(1),
                    Object::from(0),
                    Object::from(0),
                    Object::from(1),
                    Object::from(10),
                    Object::from(20),
                ],
            ),
            Operation::new("m", vec![Object::from(0), Object::from(0)]),
            Operation::new("l", vec![Object::from(50), Object::from(0)]),
            Operation::new("S", vec![]),
        ];

        let geometry = vector_rules_from_operations(&operations);

        assert_eq!(geometry.horizontal_rules.len(), 1);
        let rule = geometry.horizontal_rules[0];
        assert_eq!(rule.left, 10.0);
        assert_eq!(rule.right, 60.0);
        assert!((rule.bottom - 19.5).abs() < 0.001);
        assert!((rule.top - 20.5).abs() < 0.001);
    }

    #[test]
    fn in_place_save_round_trips_highlights_and_comments_without_duplicates() {
        let path = std::env::temp_dir().join(format!(
            "lawpdf-annotations-roundtrip-{}-unit.pdf",
            std::process::id()
        ));
        write_blank_pdf(&path);
        let annotations = vec![
            EditorAnnotation {
                page_index: 0,
                rect: PdfRect::new(72.0, 650.0, 220.0, 668.0),
                kind: AnnotationKind::Marker {
                    color_rgb: [1.0, 0.93, 0.45],
                    opacity: 0.42,
                    style: MarkerStyle::Highlight,
                },
            },
            EditorAnnotation {
                page_index: 0,
                rect: PdfRect::new(500.0, 650.0, 530.0, 680.0),
                kind: AnnotationKind::Comment {
                    id: format!("{LAWPDF_COMMENT_ID_PREFIX}roundtrip"),
                    text: "Remember this point".to_owned(),
                    color_rgb: [1.0, 0.78, 0.28],
                    created_at: "2026-07-12T00:00:00Z".to_owned(),
                    updated_at: "2026-07-12T00:00:00Z".to_owned(),
                    anchor: (210.0, 659.0),
                },
            },
        ];

        save_with_annotations(&path, &path, &annotations).unwrap();
        assert_eq!(load_lawpdf_annotations(&path).unwrap().len(), 2);
        save_with_annotations(&path, &path, &annotations).unwrap();
        let loaded = load_lawpdf_annotations(&path).unwrap();
        assert_eq!(loaded.len(), 2);
        assert!(loaded.iter().any(|annotation| matches!(
            annotation.kind,
            AnnotationKind::Marker {
                style: MarkerStyle::Highlight,
                ..
            }
        )));
        assert!(
            loaded
                .iter()
                .any(|annotation| matches!(annotation.kind, AnnotationKind::Comment { .. }))
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn sync_lawpdf_comments_updates_without_duplicates_and_deletes() {
        let path = std::env::temp_dir().join(format!(
            "lawpdf-comments-sync-{}-{}.pdf",
            std::process::id(),
            "unit"
        ));
        write_blank_pdf(&path);

        let mut comment = EditorAnnotation {
            page_index: 0,
            rect: PdfRect::new(72.0, 700.0, 100.0, 728.0),
            kind: AnnotationKind::Comment {
                id: format!("{LAWPDF_COMMENT_ID_PREFIX}unit"),
                text: "First note".to_owned(),
                color_rgb: [1.0, 0.78, 0.28],
                created_at: "2026-05-29T00:00:00Z".to_owned(),
                updated_at: "2026-05-29T00:00:00Z".to_owned(),
                anchor: (86.0, 714.0),
            },
        };

        assert_eq!(sync_lawpdf_comments(&path, &[comment.clone()]).unwrap(), 1);
        let loaded = load_lawpdf_comments(&path).unwrap();
        assert_eq!(loaded.len(), 1);

        if let AnnotationKind::Comment {
            text, color_rgb, ..
        } = &mut comment.kind
        {
            *text = "Updated note".to_owned();
            *color_rgb = [0.46, 0.70, 1.0];
        }

        assert_eq!(sync_lawpdf_comments(&path, &[comment]).unwrap(), 1);
        let loaded = load_lawpdf_comments(&path).unwrap();
        assert_eq!(loaded.len(), 1);
        match &loaded[0].kind {
            AnnotationKind::Comment {
                text, color_rgb, ..
            } => {
                assert_eq!(text, "Updated note");
                assert_eq!(*color_rgb, [0.46, 0.70, 1.0]);
            }
            _ => panic!("expected comment"),
        }

        assert_eq!(sync_lawpdf_comments(&path, &[]).unwrap(), 0);
        assert!(load_lawpdf_comments(&path).unwrap().is_empty());
        let _ = fs::remove_file(path);
    }

    #[test]
    fn load_pdf_web_links_reads_uri_link_annotations() {
        let path = std::env::temp_dir().join(format!(
            "lawpdf-web-link-{}-{}.pdf",
            std::process::id(),
            "unit"
        ));
        write_link_pdf(&path, "https://example.com/path");

        let links = load_pdf_web_links(&path, 1).unwrap();

        assert_eq!(links.len(), 1);
        assert_eq!(links[0].len(), 1);
        assert_eq!(links[0][0].url, "https://example.com/path");
        assert_eq!(links[0][0].rect, PdfRect::new(72.0, 700.0, 180.0, 720.0));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn load_pdf_web_links_normalizes_www_and_ignores_non_web_actions() {
        let path = std::env::temp_dir().join(format!(
            "lawpdf-web-link-normalize-{}-{}.pdf",
            std::process::id(),
            "unit"
        ));
        write_link_pdf(&path, "www.example.com");

        let links = load_pdf_web_links(&path, 1).unwrap();

        assert_eq!(links[0][0].url, "https://www.example.com");
        assert_eq!(normalize_web_url("javascript:alert(1)"), None);
        let _ = fs::remove_file(path);
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

    fn write_link_pdf(path: &Path, uri: &str) {
        let mut document = Document::with_version("1.5");
        let catalog_id = document.new_object_id();
        let pages_id = document.new_object_id();
        let page_id = document.new_object_id();
        let link_id = document.new_object_id();

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
                "Annots" => Object::Array(vec![Object::Reference(link_id)]),
            }),
        );
        document.objects.insert(
            link_id,
            Object::Dictionary(dictionary! {
                "Type" => Object::Name(b"Annot".to_vec()),
                "Subtype" => Object::Name(b"Link".to_vec()),
                "Rect" => Object::Array(vec![72.into(), 700.into(), 180.into(), 720.into()]),
                "Border" => Object::Array(vec![0.into(), 0.into(), 0.into()]),
                "A" => Object::Dictionary(dictionary! {
                    "S" => Object::Name(b"URI".to_vec()),
                    "URI" => literal(uri),
                }),
            }),
        );
        document.trailer.set("Root", Object::Reference(catalog_id));
        document.save(path).unwrap();
    }
}
