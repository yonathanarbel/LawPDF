use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};

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
use crate::performance_cache::{CachedDocumentMetadata, PerformanceCache};

#[cfg(not(test))]
pub use crate::native_process::PdfEngine;
#[cfg(test)]
pub type PdfEngine = NativePdfEngine;

pub struct NativePdfEngine {
    pdfium: &'static Pdfium,
    open_documents: RefCell<VecDeque<OpenPdfDocument>>,
    performance_cache: PerformanceCache,
    editor_performance_cache: PerformanceCache,
}

struct OpenPdfDocument {
    path: PathBuf,
    document: PdfDocument<'static>,
    editor_annotation_indices: HashMap<usize, Vec<usize>>,
}

struct AnnotationVisibilityGuard<'a>(Vec<(PdfPageAnnotation<'a>, bool)>);

impl AnnotationVisibilityGuard<'_> {
    fn restore(&mut self) -> Result<()> {
        for (annotation, was_hidden) in &mut self.0 {
            annotation.set_is_hidden(*was_hidden)?;
        }
        self.0.clear();
        Ok(())
    }
}

impl Drop for AnnotationVisibilityGuard<'_> {
    fn drop(&mut self) {
        // Restore on error paths too, before the cached document is reused.
        for (annotation, was_hidden) in &mut self.0 {
            let _ = annotation.set_is_hidden(*was_hidden);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RenderQuality {
    Crisp,
    Fast,
}

/// RGB page raster for LiquidVision (LmV tier), plus page size in points.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct VisionPage {
    #[serde(skip)]
    pub rgb: Vec<u8>,
    pub width: usize,
    pub height: usize,
    pub page_width_pts: f64,
    pub page_height_pts: f64,
}

const OPEN_DOCUMENT_CACHE_CAP: usize = 3;
static PDFIUM: OnceLock<Result<&'static Pdfium, String>> = OnceLock::new();
// pdfium-render 0.8's `sync` feature locks library *lifetimes*, not individual
// calls through a shared instance. Our process-wide instance therefore needs
// an operation lock too, including document destruction on worker teardown.
static PDFIUM_ACCESS: Mutex<()> = Mutex::new(());

fn lock_pdfium() -> MutexGuard<'static, ()> {
    PDFIUM_ACCESS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

impl Drop for NativePdfEngine {
    fn drop(&mut self) {
        let _native_access = lock_pdfium();
        self.open_documents.get_mut().clear();
    }
}

pub const LAWPDF_COMMENT_ID_PREFIX: &str = "LawPDF-comment-";
const LAWPDF_WIDE_FOOTNOTE_DIVIDERS_ENV: &str = "LAWPDF_WIDE_FOOTNOTE_DIVIDERS";

impl NativePdfEngine {
    pub fn new() -> Result<Self> {
        let _native_access = lock_pdfium();
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
            performance_cache: PerformanceCache::new(),
            editor_performance_cache: PerformanceCache::new().for_editor(),
        })
    }

    pub fn load_document_adaptive(
        &self,
        path: &Path,
        optimize_large_documents: bool,
    ) -> Result<LoadedDocument> {
        if !optimize_large_documents {
            return self.load_document(path);
        }

        self.load_document_optimized(path)
    }

    fn load_document_optimized(&self, path: &Path) -> Result<LoadedDocument> {
        if let Some(cached) = self.performance_cache.load_document_metadata(path, true) {
            return Ok(self.optimized_document_from_metadata(path, cached));
        }

        let metadata = self.with_open_document(path, |document| {
            let page_count = document.pages().len() as usize;
            let mut pages = Vec::with_capacity(page_count);
            for page_index in 0..page_count {
                let page = document
                    .pages()
                    .get(page_index as u16)
                    .with_context(|| format!("failed to read page {}", page_index + 1))?;
                let geometry = native_page_geometry(&page);
                let width = page.width().value;
                let height = page.height().value;
                let mut page_info = PageInfo::with_footnote_divider_y_from_top(width, height, None);
                map_page_geometry(&mut page_info, geometry);
                pages.push(page_info);
            }
            Ok(CachedDocumentMetadata {
                links: vec![Vec::new(); page_count],
                pages,
                optimized: true,
            })
        })?;
        self.performance_cache
            .save_document_metadata(path, &metadata);
        Ok(self.optimized_document_from_metadata(path, metadata))
    }

    #[cfg(feature = "devtools")]
    pub fn load_document_metadata_only(&self, path: &Path) -> Result<LoadedDocument> {
        self.load_document_optimized(path)
    }

    fn optimized_document_from_metadata(
        &self,
        path: &Path,
        metadata: CachedDocumentMetadata,
    ) -> LoadedDocument {
        let mut document = loaded_document_from_metadata(path, metadata);
        if let Some(links) = self.performance_cache.load_document_links(path) {
            document.links = links;
            document.links_loaded = true;
        }
        document
    }

    pub fn load_document_links(
        &self,
        path: &Path,
        page_count: usize,
    ) -> Result<Vec<Vec<PageLink>>> {
        if let Some(links) = self.performance_cache.load_document_links(path) {
            return Ok(links);
        }
        let links = load_pdf_web_links(path, page_count)?;
        self.performance_cache.save_document_links(path, &links);
        Ok(links)
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

                let geometry = native_page_geometry(&page);
                let width = page.width().value;
                let height = page.height().value;
                let crop_box = None;
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
                map_page_geometry(&mut page_info, geometry);
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
                links_loaded: true,
                optimized: false,
            })
        })
    }

    pub fn load_page_text(&self, path: &Path, page_index: usize) -> Result<String> {
        if let Some(text) = self.performance_cache.load_page_text(path, page_index) {
            return Ok(text);
        }
        let text = self.with_open_document(path, |document| {
            let page = document
                .pages()
                .get(page_index as u16)
                .with_context(|| format!("failed to read page {}", page_index + 1))?;

            let text_page = page
                .text()
                .with_context(|| format!("failed to read text on page {}", page_index + 1))?;

            Ok(text_page.all())
        })?;
        self.performance_cache
            .save_page_text(path, page_index, &text);
        Ok(text)
    }

    pub fn load_page_text_chars(
        &self,
        path: &Path,
        page_index: usize,
    ) -> Result<Vec<PageTextChar>> {
        if let Some(chars) = self
            .performance_cache
            .load_page_text_chars(path, page_index)
        {
            return Ok(chars);
        }
        let chars = self.with_open_document(path, |document| {
            let page = document
                .pages()
                .get(page_index as u16)
                .with_context(|| format!("failed to read page {}", page_index + 1))?;

            let text_page = page
                .text()
                .with_context(|| format!("failed to read text on page {}", page_index + 1))?;

            let geometry = native_page_geometry(&page);
            let mut chars = extract_text_chars(&text_page);
            for character in &mut chars {
                character.rect = character.rect.map(|rect| geometry.rect(rect, false));
            }
            Ok(chars)
        })?;
        self.performance_cache
            .save_page_text_chars(path, page_index, &chars);
        Ok(chars)
    }

    pub fn render_page(&self, path: &Path, page_index: usize, zoom: f32) -> Result<RenderedPage> {
        self.render_page_internal(path, page_index, zoom, RenderQuality::Crisp, false)
    }

    pub fn render_page_with_quality(
        &self,
        path: &Path,
        page_index: usize,
        zoom: f32,
        quality: RenderQuality,
    ) -> Result<RenderedPage> {
        self.render_page_internal(path, page_index, zoom, quality, true)
    }

    pub(crate) fn render_page_internal(
        &self,
        path: &Path,
        page_index: usize,
        zoom: f32,
        quality: RenderQuality,
        editor: bool,
    ) -> Result<RenderedPage> {
        let fast = quality == RenderQuality::Fast;
        let cache = if editor {
            &self.editor_performance_cache
        } else {
            &self.performance_cache
        };
        if let Some(rendered) = cache.load_rendered_page(path, page_index, zoom, fast) {
            return Ok(rendered);
        }
        let rendered = self.with_open_document_data(path, |open| {
            let page = open
                .document
                .pages()
                .get(page_index as u16)
                .with_context(|| format!("failed to read page {}", page_index + 1))?;

            let width = page.width().value;
            let height = page.height().value;
            anyhow::ensure!(
                width.is_finite()
                    && height.is_finite()
                    && width > 0.0
                    && height > 0.0
                    && zoom.is_finite()
                    && zoom > 0.0,
                "This page has invalid dimensions."
            );
            // Limit both axes and the full raster allocation, including unusually
            // tall pages. A width-only cap permits enormous native allocations.
            let limit_by_area = (16_000_000.0_f32 * width / height).sqrt();
            let limit_by_height = 8192.0 * width / height;
            let target_width = (width * zoom)
                .round()
                .min(8192.0)
                .min(limit_by_area)
                .min(limit_by_height)
                .max(1.0) as i32;
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

            // LawPDF draws editable annotations as overlays. Hide only the
            // successfully decoded owned annotations in this in-memory render;
            // third-party annotations and exported rasters remain visible.
            let mut hidden = AnnotationVisibilityGuard(Vec::new());
            if editor {
                for &index in open
                    .editor_annotation_indices
                    .get(&page_index)
                    .into_iter()
                    .flatten()
                {
                    let mut annotation = page.annotations().get(index as _)?;
                    let was_hidden = annotation.is_hidden();
                    annotation.set_is_hidden(true)?;
                    hidden.0.push((annotation, was_hidden));
                }
            }
            let bitmap = page.render_with_config(&config);
            hidden.restore()?;
            let bitmap =
                bitmap.with_context(|| format!("failed to render page {}", page_index + 1))?;

            #[cfg(feature = "bench-image-conversion")]
            let (width, height, rgba) = {
                let image = bitmap.as_image().to_rgba8();
                let (width, height) = image.dimensions();
                (width, height, image.into_raw())
            };

            #[cfg(not(feature = "bench-image-conversion"))]
            let (width, height, rgba) = (
                bitmap.width() as u32,
                bitmap.height() as u32,
                bitmap.as_rgba_bytes(),
            );

            Ok(RenderedPage {
                page_index,
                width: width as usize,
                height: height as usize,
                rgba,
            })
        })?;
        cache.save_rendered_page(path, &rendered, zoom, fast);
        Ok(rendered)
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
        anyhow::ensure!(
            (1..=4096).contains(&imgsz),
            "The requested analysis raster exceeds the size limit."
        );
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
        let _native_access = lock_pdfium();
        self.open_documents
            .borrow_mut()
            .retain(|open| open.path != path);
    }

    fn with_open_document<T>(
        &self,
        path: &Path,
        operation: impl FnOnce(&PdfDocument<'static>) -> Result<T>,
    ) -> Result<T> {
        self.with_open_document_data(path, |open| operation(&open.document))
    }

    fn with_open_document_data<T>(
        &self,
        path: &Path,
        operation: impl FnOnce(&OpenPdfDocument) -> Result<T>,
    ) -> Result<T> {
        let _native_access = lock_pdfium();
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
            anyhow::ensure!(
                document.pages().len() as usize <= 20_000,
                "This PDF exceeds the 20,000-page limit."
            );
            open_documents.push_front(OpenPdfDocument {
                path: path.to_path_buf(),
                document,
                editor_annotation_indices: editor_annotation_indices(path).unwrap_or_default(),
            });
            while open_documents.len() > OPEN_DOCUMENT_CACHE_CAP {
                open_documents.pop_back();
            }
        }

        let open_document = open_documents
            .front()
            .ok_or_else(|| anyhow!("internal PDF cache is empty"))?;
        operation(open_document)
    }
}

fn editor_annotation_indices(path: &Path) -> Result<HashMap<usize, Vec<usize>>> {
    let document = Document::load(path)?;
    let mut indices = HashMap::new();
    for (number, page_id) in document.get_pages() {
        let page = document.get_object(page_id)?.as_dict()?;
        let Ok(annots) = page.get(b"Annots") else {
            continue;
        };
        let annots = match annots {
            Object::Reference(id) => document.get_object(*id)?,
            annots => annots,
        };
        let Ok(annots) = annots.as_array() else {
            continue;
        };
        let page_index = number as usize - 1;
        let owned = annots
            .iter()
            .enumerate()
            .filter_map(|(index, annot)| {
                lawpdf_owned_annotation_from_pdf(&document, annot, page_index).map(|_| index)
            })
            .collect::<Vec<_>>();
        if !owned.is_empty() {
            indices.insert(page_index, owned);
        }
    }
    Ok(indices)
}

fn loaded_document_from_metadata(path: &Path, metadata: CachedDocumentMetadata) -> LoadedDocument {
    let page_count = metadata.pages.len();
    let title = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Untitled PDF")
        .to_owned();
    LoadedDocument {
        path: path.to_path_buf(),
        title,
        page_count,
        pages: metadata.pages,
        native_text: vec![String::new(); page_count],
        native_text_loaded: vec![false; page_count],
        text_chars: vec![None; page_count],
        links: metadata.links,
        links_loaded: false,
        optimized: metadata.optimized,
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

fn native_page_geometry(page: &PdfPage<'_>) -> crate::page_geometry::PageGeometry {
    let crop = page_crop_box(page);
    let media = page_media_box(page);
    crate::page_geometry::PageGeometry {
        bounds: PdfRect::new(
            crop.left.max(media.left),
            crop.bottom.max(media.bottom),
            crop.right.min(media.right),
            crop.top.min(media.top),
        ),
        rotation: page
            .rotation()
            .map(|rotation| rotation.as_degrees() as i64)
            .unwrap_or(0),
    }
}

fn map_page_geometry(page: &mut PageInfo, geometry: crate::page_geometry::PageGeometry) {
    for rectangles in [
        &mut page.path_object_rects,
        &mut page.image_object_rects,
        &mut page.thin_horizontal_object_rects,
        &mut page.thin_vertical_object_rects,
        &mut page.vector_horizontal_rule_rects,
        &mut page.vector_vertical_rule_rects,
        &mut page.vector_ruled_cell_rects,
    ] {
        for rect in rectangles {
            *rect = geometry.rect(*rect, false);
        }
    }
    if geometry.rotation % 180 != 0 {
        std::mem::swap(
            &mut page.thin_horizontal_object_rects,
            &mut page.thin_vertical_object_rects,
        );
        std::mem::swap(
            &mut page.vector_horizontal_rule_rects,
            &mut page.vector_vertical_rule_rects,
        );
    }
    if geometry.rotation != 0 || geometry.bounds.bottom != 0.0 {
        page.footnote_divider_y_from_top = None;
    }
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
    crate::atomic_file::replace_with(path, |file| {
        image
            .write_to(file, image::ImageFormat::Png)
            .map_err(std::io::Error::other)
    })
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

    crate::atomic_file::write(path, output.as_bytes())
        .with_context(|| format!("failed to write {}", path.display()))
}

/// What a successful annotation save did beyond writing `destination`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SaveReport {
    /// A copy of the previous file contents, written before an in-place save.
    pub backup: Option<PathBuf>,
    /// The source carried an owner password (empty user password) and the
    /// copy written to a new destination no longer does.
    pub owner_password_removed: bool,
    /// Hash of the exact bytes written, not a later read of a cloud-synced file.
    pub revision: Option<crate::document_store::FileRevision>,
    pub recovery_warning: Option<String>,
}

impl SaveReport {
    pub fn status_suffix(&self) -> String {
        let mut notes = Vec::new();
        if self.owner_password_removed {
            notes.push("owner password removed from the copy".to_owned());
        }
        if let Some(backup) = &self.backup {
            notes.push(format!("previous version kept at {}", backup.display()));
        }
        if notes.is_empty() {
            String::new()
        } else {
            format!(" ({})", notes.join("; "))
        }
    }
}

/// Why LawPDF refuses to rewrite a PDF in place through lopdf.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RewriteHazard {
    /// The source is protected, including files automatically decrypted with an
    /// empty user password. A full rewrite must not silently remove protection.
    Encrypted,
    /// The file carries a digital signature. A full rewrite changes the bytes
    /// the signature's `/ByteRange` covers, which invalidates it.
    Signed,
}

impl RewriteHazard {
    fn in_place_message(self, path: &Path) -> String {
        match self {
            Self::Encrypted => format!(
                "{} is protected by an owner password. LawPDF cannot automatically preserve \
                 that protection when saving. Use Save As to write an unlocked copy.",
                path.display()
            ),
            Self::Signed => format!(
                "{} is digitally signed. Saving annotations into it would invalidate the \
                 signature. Use Save As to write an annotated copy.",
                path.display()
            ),
        }
    }
}

/// Detect the conditions under which rewriting `document` would damage it.
pub fn rewrite_hazard(document: &Document) -> Option<RewriteHazard> {
    if document.is_encrypted() || document.was_encrypted() {
        return Some(RewriteHazard::Encrypted);
    }
    let is_signature = |dictionary: &Dictionary| {
        let named = |key: &[u8], expected: &[u8]| {
            dictionary
                .get(key)
                .ok()
                .and_then(|object| object.as_name().ok())
                .is_some_and(|name| name == expected)
        };
        named(b"Type", b"Sig") || (named(b"FT", b"Sig") && dictionary.has(b"V"))
    };
    let signed = document.objects.values().any(|object| match object {
        Object::Dictionary(dictionary) => is_signature(dictionary),
        Object::Stream(stream) => is_signature(&stream.dict),
        _ => false,
    });
    signed.then_some(RewriteHazard::Signed)
}

/// Drop the owner-password wrapper from a document lopdf has already decrypted,
/// so that saving it produces a readable, unencrypted file.
fn strip_owner_password(document: &mut Document) -> Result<()> {
    if document.encryption_state.is_none() {
        return Err(anyhow!(
            "this PDF requires a password to open; LawPDF cannot save changes into it"
        ));
    }
    let encrypt_id = document
        .trailer
        .remove(b"Encrypt")
        .and_then(|object| object.as_reference().ok());
    if let Some(id) = encrypt_id {
        document.objects.remove(&id);
    }
    document.encryption_state = None;
    document.max_id = document
        .objects
        .keys()
        .map(|(number, _)| *number)
        .max()
        .unwrap_or(0);
    Ok(())
}

/// Apply the rewrite policy before `document` is written to `destination`.
///
/// In place (`source == destination`): refuse when a hazard exists, and keep a
/// copy of the previous file first. To a new path: allow, stripping an owner
/// password so the copy is readable, and report what happened.
fn prepare_rewrite(
    document: &mut Document,
    source: &Path,
    destination: &Path,
) -> Result<SaveReport> {
    let mut report = SaveReport::default();
    let in_place = same_file(source, destination);
    match rewrite_hazard(document) {
        Some(hazard) if in_place => return Err(anyhow!(hazard.in_place_message(source))),
        Some(RewriteHazard::Encrypted) => {
            strip_owner_password(document)?;
            report.owner_password_removed = true;
        }
        Some(RewriteHazard::Signed) | None => {}
    }
    if in_place {
        report.backup = backup_before_rewrite(source)?;
    }
    Ok(report)
}

fn same_file(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

const REWRITE_BACKUP_DIR: &str = "backups";
const REWRITE_BACKUP_LIMIT: usize = 40;

/// Copy `source` into `<app data>/backups/` before it is rewritten in place.
///
/// One rolling copy is kept per document (the newest pre-save contents), and
/// the folder is bounded so it cannot grow without limit. Returns the backup
/// path, or `None` when no app data directory is available.
fn backup_before_rewrite(source: &Path) -> Result<Option<PathBuf>> {
    let Some(root) = rewrite_backup_dir() else {
        return Ok(None);
    };
    backup_before_rewrite_in(source, &root)
}

fn backup_before_rewrite_in(source: &Path, root: &Path) -> Result<Option<PathBuf>> {
    fs::create_dir_all(root)
        .with_context(|| format!("failed to create backup folder {}", root.display()))?;
    let key = fs::canonicalize(source).unwrap_or_else(|_| source.to_path_buf());
    let digest = crate::hashing::sha256_hex(key.to_string_lossy().as_bytes());
    let file_name = source
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("document.pdf");
    let backup = root.join(format!("{}-{file_name}", &digest[..12]));
    crate::atomic_file::replace_with(&backup, |file| {
        let mut input = fs::File::open(source)?;
        std::io::copy(&mut input, file).map(|_| ())
    })
    .with_context(|| format!("failed to back up {} before saving", source.display()))?;
    prune_rewrite_backups(root, REWRITE_BACKUP_LIMIT);
    Ok(Some(backup))
}

fn rewrite_backup_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("LAWPDF_BACKUP_DIR") {
        return Some(PathBuf::from(dir));
    }
    crate::settings::app_data_dir().map(|dir| dir.join(REWRITE_BACKUP_DIR))
}

fn prune_rewrite_backups(root: &Path, keep: usize) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    let mut files = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            metadata
                .is_file()
                .then(|| (metadata.modified().ok(), entry.path()))
        })
        .collect::<Vec<_>>();
    if files.len() <= keep {
        return;
    }
    files.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    for (_, path) in files.into_iter().skip(keep) {
        let _ = fs::remove_file(path);
    }
}

pub fn save_with_annotations(
    source: &Path,
    destination: &Path,
    annotations: &[EditorAnnotation],
) -> Result<SaveReport> {
    let revision =
        crate::document_store::FileRevision::read(source).map_err(|error| anyhow!(error))?;
    save_with_annotations_checked(source, destination, annotations, &revision)
}

pub fn save_with_annotations_checked(
    source: &Path,
    destination: &Path,
    annotations: &[EditorAnnotation],
    expected: &crate::document_store::FileRevision,
) -> Result<SaveReport> {
    use crate::document_store::{FileRevision, MAX_DOCUMENT_BYTES, read_limited};
    crate::document_store::validate_annotations(annotations).map_err(|error| anyhow!(error))?;
    let source_bytes = read_limited(source, MAX_DOCUMENT_BYTES).map_err(|error| anyhow!(error))?;
    if FileRevision::from_bytes(&source_bytes) != *expected {
        return Err(anyhow!(
            "The PDF changed outside LawPDF. Your changes remain in recovery; save a separate copy."
        ));
    }
    let in_place = same_file(source, destination);
    let destination_revision = if in_place {
        Some(expected.clone())
    } else if destination.exists() {
        Some(FileRevision::read(destination).map_err(|error| anyhow!(error))?)
    } else {
        None
    };
    let mut document = Document::load_mem(&source_bytes)
        .with_context(|| format!("failed to load source PDF {}", source.display()))?;
    let mut report = prepare_rewrite(&mut document, source, destination)?;
    remove_lawpdf_owned_annotations(&mut document)?;
    let pages = document.get_pages();

    for annotation in annotations {
        let page_number = annotation.page_index as u32 + 1;
        let page_id = *pages
            .get(&page_number)
            .ok_or_else(|| anyhow!("PDF has no page {}", page_number))?;
        let annotation_id = document.new_object_id();
        let mut annotation = annotation.clone();
        pdf_page_geometry(&document, page_id)?.annotation(&mut annotation, true);
        let object = annotation_to_pdf_object(&annotation);
        document.objects.insert(annotation_id, object);
        append_annotation(&mut document, page_id, annotation_id)?;
    }

    document.prune_objects();
    document.compress();
    let mut bytes = Vec::new();
    document.save_to(&mut bytes)?;
    if bytes.len() as u64 > MAX_DOCUMENT_BYTES {
        return Err(anyhow!(
            "The annotated PDF exceeds the 512 MB document limit."
        ));
    }
    let revision = if in_place {
        crate::document_store::DocumentStore::new()
            .and_then(|store| store.preserve_bytes(&bytes))
            .map_err(anyhow::Error::msg)?
    } else {
        FileRevision::from_bytes(&bytes)
    };
    crate::atomic_file::replace_with_guard(
        destination,
        |writer| std::io::Write::write_all(writer, &bytes),
        || match &destination_revision {
            Some(previous) => previous
                .require_current(destination)
                .map_err(std::io::Error::other),
            None if destination.exists() => Err(std::io::Error::other(
                "A file appeared at the chosen destination. Choose another filename.",
            )),
            None => Ok(()),
        },
    )?;
    report.revision = Some(revision);

    Ok(report)
}

pub fn rotate_pdf_page(source: &Path, page_index: usize, clockwise: bool) -> Result<i64> {
    let expected = crate::document_store::FileRevision::read(source).map_err(anyhow::Error::msg)?;
    rotate_pdf_page_checked(source, page_index, clockwise, &expected).map(|(rotation, _)| rotation)
}

pub fn rotate_pdf_page_checked(
    source: &Path,
    page_index: usize,
    clockwise: bool,
    expected: &crate::document_store::FileRevision,
) -> Result<(i64, crate::document_store::FileRevision)> {
    let bytes =
        crate::document_store::read_limited(source, crate::document_store::MAX_DOCUMENT_BYTES)
            .map_err(anyhow::Error::msg)?;
    anyhow::ensure!(
        crate::document_store::FileRevision::from_bytes(&bytes) == *expected,
        "This PDF changed outside LawPDF. Reopen it before rotating a page."
    );
    let mut document = Document::load_mem(&bytes)
        .with_context(|| format!("failed to load source PDF {}", source.display()))?;
    prepare_rewrite(&mut document, source, source)?;
    let page_number = page_index as u32 + 1;
    let page_id = document
        .get_pages()
        .get(&page_number)
        .copied()
        .ok_or_else(|| anyhow!("PDF has no page {page_number}"))?;
    let current_rotation = inherited_page_rotation(&document, page_id)?;
    let delta = if clockwise { 90 } else { -90 };
    let rotation = (current_rotation + delta).rem_euclid(360);

    document
        .get_object_mut(page_id)?
        .as_dict_mut()?
        .set("Rotate", Object::Integer(rotation));
    document.prune_objects();
    document.compress();
    let mut output = Vec::new();
    document.save_to(&mut output)?;
    anyhow::ensure!(
        output.len() as u64 <= crate::document_store::MAX_DOCUMENT_BYTES,
        "The rotated PDF exceeds the document size limit."
    );
    let revision = crate::document_store::DocumentStore::new()
        .and_then(|store| store.preserve_bytes(&output))
        .map_err(anyhow::Error::msg)?;
    crate::atomic_file::replace_with_guard(
        source,
        |file| std::io::Write::write_all(file, &output),
        || {
            expected
                .require_current(source)
                .map_err(std::io::Error::other)
        },
    )?;
    Ok((rotation, revision))
}

fn inherited_page_rotation(document: &Document, page_id: ObjectId) -> Result<i64> {
    let mut object_id = page_id;
    for _ in 0..64 {
        let dictionary = document.get_object(object_id)?.as_dict()?;
        if let Ok(value) = dictionary.get(b"Rotate") {
            return pdf_integer(document, value)
                .map(|rotation| rotation.rem_euclid(360))
                .ok_or_else(|| anyhow!("PDF page has an invalid Rotate value"));
        }
        let Some(parent_id) = dictionary
            .get(b"Parent")
            .ok()
            .and_then(|parent| match parent {
                Object::Reference(id) => Some(*id),
                _ => None,
            })
        else {
            return Ok(0);
        };
        object_id = parent_id;
    }
    Err(anyhow!("PDF page tree is too deeply nested"))
}

fn pdf_integer<'a>(document: &'a Document, mut value: &'a Object) -> Option<i64> {
    for _ in 0..64 {
        match value {
            Object::Integer(value) => return Some(*value),
            Object::Reference(id) => value = document.get_object(*id).ok()?,
            _ => return None,
        }
    }
    None
}

fn pdf_page_geometry(
    document: &Document,
    page_id: ObjectId,
) -> Result<crate::page_geometry::PageGeometry> {
    let inherited_rect = |key: &[u8]| -> Option<PdfRect> {
        let mut id = page_id;
        for _ in 0..64 {
            let dict = document.get_object(id).ok()?.as_dict().ok()?;
            if let Ok(value) = dict.get(key) {
                let value = match value {
                    Object::Reference(id) => document.get_object(*id).ok()?,
                    value => value,
                };
                return pdf_rect_from_object(value);
            }
            id = dict.get(b"Parent").ok()?.as_reference().ok()?;
        }
        None
    };
    let media =
        inherited_rect(b"MediaBox").ok_or_else(|| anyhow!("PDF page has no valid MediaBox"))?;
    let crop = inherited_rect(b"CropBox").unwrap_or(media);
    let bounds = PdfRect::new(
        crop.left.max(media.left),
        crop.bottom.max(media.bottom),
        crop.right.min(media.right),
        crop.top.min(media.top),
    );
    anyhow::ensure!(
        bounds.width() > 0.0 && bounds.height() > 0.0,
        "PDF page has an empty visible area"
    );
    let rotation = inherited_page_rotation(document, page_id)?;
    anyhow::ensure!(
        rotation % 90 == 0,
        "PDF page rotation must be a multiple of 90 degrees"
    );
    Ok(crate::page_geometry::PageGeometry { bounds, rotation })
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
        let geometry = pdf_page_geometry(&document, page_id)?;
        for link in &mut links[page_index] {
            link.rect = geometry.rect(link.rect, false);
        }
    }

    Ok(links)
}

pub fn load_lawpdf_annotations(source: &Path) -> Result<Vec<EditorAnnotation>> {
    let document = Document::load(source)
        .with_context(|| format!("failed to load source PDF {}", source.display()))?;
    let pages = document.get_pages();
    let mut annotations = Vec::new();

    for (page_number, page_id) in pages {
        let first_annotation = annotations.len();
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
        if first_annotation < annotations.len() {
            let geometry = pdf_page_geometry(&document, page_id)?;
            for annotation in &mut annotations[first_annotation..] {
                geometry.annotation(annotation, false);
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
    prepare_rewrite(&mut document, source, source)?;
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
        let mut annotation = annotation.clone();
        pdf_page_geometry(&document, page_id)?.annotation(&mut annotation, true);
        let object = annotation_to_pdf_object(&annotation);
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
) -> Result<SaveReport> {
    let mut document = Document::load(source)
        .with_context(|| format!("failed to load source PDF {}", source.display()))?;
    let report = prepare_rewrite(&mut document, source, destination)?;
    append_ocr_text_layers(&mut document, page_sizes, ocr_text)?;
    document.prune_objects();
    document.compress();
    save_document_in_place(&mut document, destination)?;

    Ok(report)
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
            "DA" => literal(format!("/Helv {font_size} Tf {} {} {} rg", color_rgb[0], color_rgb[1], color_rgb[2])),
            "LawPDF" => Object::Boolean(true),
            "LawFontSize" => Object::Real(*font_size),
            "C" => color_array(*color_rgb),
            "F" => Object::Integer(4),
        }),
        AnnotationKind::Comment {
            id,
            text,
            color_rgb,
            updated_at,
            created_at,
            anchor,
        } => Object::Dictionary(dictionary! {
            "Type" => Object::Name(b"Annot".to_vec()),
            "Subtype" => Object::Name(b"Text".to_vec()),
            "Rect" => rect_array(annotation.rect),
            "Contents" => literal(text),
            "T" => literal("LawPDF"),
            "NM" => literal(id),
            "M" => literal(updated_at),
            "CreationDate" => literal(created_at),
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
                "LawPDF" => Object::Boolean(true),
                "LawSigner" => literal(signer),
                "LawSignedAt" => literal(signed_at),
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
    let created_at = dict
        .get(b"CreationDate")
        .ok()
        .and_then(pdf_object_text)
        .unwrap_or_else(|| updated_at.clone());
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
    let rect = dict.get(b"Rect").ok().and_then(pdf_rect_from_object)?;
    let color_rgb = dict
        .get(b"C")
        .ok()
        .and_then(pdf_color_from_object)
        .unwrap_or([1.0, 0.93, 0.45]);
    let kind = match subtype.as_str() {
        "Highlight" | "Underline" => AnnotationKind::Marker {
            color_rgb,
            opacity: dict
                .get(b"CA")
                .ok()
                .and_then(pdf_number)
                .unwrap_or(0.42)
                .clamp(0.0, 1.0),
            style: if subtype == "Highlight" {
                MarkerStyle::Highlight
            } else {
                MarkerStyle::Underline
            },
        },
        "FreeText" => AnnotationKind::TextBox {
            text: dict
                .get(b"Contents")
                .ok()
                .and_then(pdf_object_text)
                .unwrap_or_default(),
            font_size: dict
                .get(b"LawFontSize")
                .ok()
                .and_then(pdf_number)
                .unwrap_or(12.0),
            color_rgb,
        },
        "Ink" => {
            let strokes = dict
                .get(b"InkList")
                .ok()?
                .as_array()
                .ok()?
                .iter()
                .map(|stroke| {
                    let values = stroke.as_array().ok()?;
                    if values.len() % 2 != 0 {
                        return None;
                    }
                    values
                        .chunks_exact(2)
                        .map(|pair| Some((pdf_number(&pair[0])?, pdf_number(&pair[1])?)))
                        .collect::<Option<Vec<_>>>()
                })
                .collect::<Option<Vec<_>>>()?;
            AnnotationKind::Signature {
                signer: dict
                    .get(b"LawSigner")
                    .ok()
                    .and_then(pdf_object_text)
                    .unwrap_or_default(),
                signed_at: dict
                    .get(b"LawSignedAt")
                    .ok()
                    .and_then(pdf_object_text)
                    .unwrap_or_default(),
                strokes,
            }
        }
        _ => return None,
    };
    Some(EditorAnnotation {
        page_index,
        rect,
        kind,
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
        if include_owned {
            lawpdf_owned_annotation_from_pdf(document, annotation, 0).is_some()
        } else {
            is_lawpdf_comment_dict(dict)
        }
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
        Object::String(bytes, _) if bytes.starts_with(&[0xfe, 0xff]) => {
            Some(String::from_utf16_lossy(
                &bytes[2..]
                    .chunks_exact(2)
                    .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                    .collect::<Vec<_>>(),
            ))
        }
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
    let text = value.as_ref();
    let bytes = if text.is_ascii() {
        text.as_bytes().to_vec()
    } else {
        let mut bytes = vec![0xfe, 0xff];
        bytes.extend(text.encode_utf16().flat_map(u16::to_be_bytes));
        bytes
    };
    Object::String(bytes, StringFormat::Literal)
}

fn save_document_in_place(document: &mut Document, destination: &Path) -> Result<()> {
    crate::atomic_file::replace_with(destination, |file| document.save_to(file))
        .with_context(|| format!("failed to save {}", destination.display()))
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
    fn marks_round_trip_on_cropped_rotated_pages_and_after_rotation() {
        let directory = tempfile::tempdir().unwrap();
        for rotation in [0, 90, 180, 270] {
            let path = directory.path().join(format!("rotation-{rotation}.pdf"));
            write_blank_pdf(&path);
            let mut pdf = Document::load(&path).unwrap();
            let page_id = pdf.get_pages()[&1];
            let page = pdf.get_object_mut(page_id).unwrap().as_dict_mut().unwrap();
            page.set("Rotate", Object::Integer(rotation));
            page.set(
                "CropBox",
                Object::Array(vec![20.into(), 40.into(), 600.into(), 760.into()]),
            );
            save_document_in_place(&mut pdf, &path).unwrap();
            let mark = EditorAnnotation {
                page_index: 0,
                rect: PdfRect::new(50.0, 100.0, 180.0, 130.0),
                kind: AnnotationKind::Marker {
                    color_rgb: [1.0, 0.9, 0.2],
                    opacity: 0.4,
                    style: MarkerStyle::Highlight,
                },
            };
            save_with_annotations(&path, &path, &[mark.clone()]).unwrap();
            assert_eq!(load_lawpdf_annotations(&path).unwrap(), vec![mark.clone()]);
            let before = Document::load(&path).unwrap();
            let geometry = pdf_page_geometry(&before, before.get_pages()[&1]).unwrap();
            let raw_rect = geometry.rect(mark.rect, true);
            rotate_pdf_page(&path, 0, true).unwrap();
            let after = Document::load(&path).unwrap();
            let new_geometry = pdf_page_geometry(&after, after.get_pages()[&1]).unwrap();
            let mut expected = mark;
            expected.rect = new_geometry.rect(raw_rect, false);
            let loaded = load_lawpdf_annotations(&path).unwrap();
            assert_eq!(loaded, vec![expected]);
            save_with_annotations(&path, &path, &loaded).unwrap();
            assert_eq!(load_lawpdf_annotations(&path).unwrap(), loaded);
        }
    }

    #[test]
    fn rotate_pdf_page_persists_and_normalizes_quarter_turns() {
        let path = std::env::temp_dir().join(format!(
            "lawpdf-rotate-page-{}-unit.pdf",
            std::process::id()
        ));
        write_blank_pdf(&path);

        assert_eq!(rotate_pdf_page(&path, 0, true).unwrap(), 90);
        assert_eq!(stored_page_rotation(&path), 90);
        assert_eq!(rotate_pdf_page(&path, 0, false).unwrap(), 0);
        assert_eq!(stored_page_rotation(&path), 0);
        assert_eq!(rotate_pdf_page(&path, 0, false).unwrap(), 270);
        assert_eq!(stored_page_rotation(&path), 270);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn rotate_pdf_page_respects_inherited_rotation() {
        let path = std::env::temp_dir().join(format!(
            "lawpdf-rotate-inherited-{}-unit.pdf",
            std::process::id()
        ));
        write_blank_pdf(&path);
        let mut document = Document::load(&path).unwrap();
        let page_id = document.get_pages()[&1];
        let parent_id = match document
            .get_object(page_id)
            .unwrap()
            .as_dict()
            .unwrap()
            .get(b"Parent")
            .unwrap()
        {
            Object::Reference(id) => *id,
            _ => panic!("expected page parent reference"),
        };
        document
            .get_object_mut(parent_id)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("Rotate", Object::Integer(270));
        save_document_in_place(&mut document, &path).unwrap();

        assert_eq!(rotate_pdf_page(&path, 0, true).unwrap(), 0);
        assert_eq!(stored_page_rotation(&path), 0);

        let _ = fs::remove_file(path);
    }

    fn stored_page_rotation(path: &Path) -> i64 {
        let document = Document::load(path).unwrap();
        let page_id = document.get_pages()[&1];
        match document
            .get_object(page_id)
            .unwrap()
            .as_dict()
            .unwrap()
            .get(b"Rotate")
            .unwrap()
        {
            Object::Integer(rotation) => *rotation,
            _ => panic!("expected integer page rotation"),
        }
    }

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

    /// One page, an owner password, an empty user password: the "no copy"
    /// restriction found on court filings and journal downloads.
    const OWNER_PASSWORD_PDF: &[u8] = include_bytes!("../tests/fixtures/owner-password.pdf");

    fn temp_pdf_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "lawpdf-{label}-{}-{}.pdf",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ))
    }

    fn highlight_annotation() -> EditorAnnotation {
        EditorAnnotation {
            page_index: 0,
            rect: PdfRect::new(20.0, 90.0, 180.0, 110.0),
            kind: AnnotationKind::Marker {
                color_rgb: [1.0, 0.93, 0.45],
                opacity: 0.42,
                style: MarkerStyle::Highlight,
            },
        }
    }

    #[test]
    fn every_annotation_kind_survives_save_reopen_edit_and_delete() {
        let source = temp_pdf_path("all-annotations");
        write_blank_pdf(&source);
        let rect = PdfRect::new(72.0, 650.0, 220.0, 668.0);
        let mut annotations = vec![
            highlight_annotation(),
            EditorAnnotation {
                page_index: 0,
                rect,
                kind: AnnotationKind::Marker {
                    color_rgb: [0.2, 0.3, 0.4],
                    opacity: 0.7,
                    style: MarkerStyle::Underline,
                },
            },
            EditorAnnotation {
                page_index: 0,
                rect,
                kind: AnnotationKind::TextBox {
                    text: "Café § 2 — שלום".to_owned(),
                    font_size: 16.0,
                    color_rgb: [0.2, 0.3, 0.4],
                },
            },
            EditorAnnotation {
                page_index: 0,
                rect,
                kind: AnnotationKind::Signature {
                    signer: "Zoë".to_owned(),
                    signed_at: "2026-09-17".to_owned(),
                    strokes: vec![vec![(72.0, 651.0), (100.0, 660.0), (210.0, 653.0)]],
                },
            },
            EditorAnnotation {
                page_index: 0,
                rect,
                kind: AnnotationKind::Comment {
                    id: "LawPDF-comment-roundtrip-all".to_owned(),
                    text: "Check § 2".to_owned(),
                    color_rgb: [1.0, 0.8, 0.3],
                    created_at: "first".to_owned(),
                    updated_at: "later".to_owned(),
                    anchor: (210.0, 659.0),
                },
            },
        ];
        // An unrelated annotation must survive every LawPDF rewrite.
        let mut pdf = Document::load(&source).unwrap();
        let page = *pdf.get_pages().values().next().unwrap();
        let foreign = pdf.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Text", "Rect" => rect_array(rect),
            "Contents" => literal("Other reader's note"),
        });
        append_annotation(&mut pdf, page, foreign).unwrap();
        pdf.save(&source).unwrap();
        for _ in 0..2 {
            save_with_annotations(&source, &source, &annotations).unwrap();
            assert_eq!(load_lawpdf_annotations(&source).unwrap(), annotations);
            let pdf = Document::load(&source).unwrap();
            let page = *pdf.get_pages().values().next().unwrap();
            assert_eq!(
                pdf.get_object(page)
                    .unwrap()
                    .as_dict()
                    .unwrap()
                    .get(b"Annots")
                    .unwrap()
                    .as_array()
                    .unwrap()
                    .len(),
                annotations.len() + 1
            );
        }
        annotations.clear();
        save_with_annotations(&source, &source, &annotations).unwrap();
        assert!(load_lawpdf_annotations(&source).unwrap().is_empty());
        let pdf = Document::load(&source).unwrap();
        let page = *pdf.get_pages().values().next().unwrap();
        assert_eq!(
            pdf.get_object(page)
                .unwrap()
                .as_dict()
                .unwrap()
                .get(b"Annots")
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            1
        );
        fs::remove_file(source).unwrap();
    }

    #[test]
    fn concurrent_engines_render_and_close_annotated_documents_safely() {
        let source = temp_pdf_path("concurrent-engines");
        write_blank_pdf(&source);
        save_with_annotations(&source, &source, &[highlight_annotation()]).unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(6));
        std::thread::scope(|scope| {
            for _ in 0..6 {
                let source = &source;
                let barrier = barrier.clone();
                scope.spawn(move || {
                    let engine = PdfEngine::new().unwrap();
                    barrier.wait();
                    for zoom in [0.25, 0.3, 0.35] {
                        let page = engine.render_page(source, 0, zoom).unwrap();
                        assert!(!page.rgba.is_empty());
                        engine.close_document(source);
                    }
                    // Leave a native document cached so Drop is exercised while
                    // the other engines may still be rendering or closing.
                    engine.load_document(source).unwrap();
                });
            }
        });
        fs::remove_file(source).unwrap();
    }

    #[test]
    fn editor_does_not_double_paint_saved_marks_but_export_keeps_them() {
        let source = temp_pdf_path("annotation-render");
        write_blank_pdf(&source);
        let engine = PdfEngine::new().expect("PDFium is required for release QA");
        let blank = engine.render_page(&source, 0, 0.5).unwrap();
        engine.close_document(&source);
        save_with_annotations(&source, &source, &[highlight_annotation()]).unwrap();
        let exported = engine.render_page(&source, 0, 0.5).unwrap();
        let editor = engine
            .render_page_with_quality(&source, 0, 0.5, RenderQuality::Crisp)
            .unwrap();
        assert_ne!(
            exported.rgba, blank.rgba,
            "export includes the saved highlight"
        );
        assert_eq!(
            editor.rgba, blank.rgba,
            "editor draws the highlight separately"
        );
        // Force an uncached full render after the editor render: hidden flags
        // must not leak into subsequent exports from the same native document.
        let full_again = engine.render_page(&source, 0, 0.51).unwrap();
        assert!(full_again.rgba.chunks_exact(4).any(|p| p[0] != p[2]));
        engine.close_document(&source);
        fs::remove_file(source).unwrap();
    }

    #[test]
    fn owner_password_pdf_is_refused_in_place_and_unlocked_on_save_as() {
        let source = temp_pdf_path("owner-password");
        fs::write(&source, OWNER_PASSWORD_PDF).unwrap();
        let loaded = Document::load(&source).unwrap();
        assert_eq!(rewrite_hazard(&loaded), Some(RewriteHazard::Encrypted));

        let error = save_with_annotations(&source, &source, &[highlight_annotation()])
            .expect_err("in-place save into a protected PDF must be refused");
        assert!(
            error.to_string().contains("owner password"),
            "unexpected error: {error:#}"
        );
        assert_eq!(
            fs::read(&source).unwrap(),
            OWNER_PASSWORD_PDF,
            "a refused save must leave the original untouched"
        );

        let copy = temp_pdf_path("owner-password-copy");
        let report = save_with_annotations(&source, &copy, &[highlight_annotation()]).unwrap();
        assert!(report.owner_password_removed);
        assert!(report.backup.is_none());
        let unlocked = Document::load(&copy).unwrap();
        assert!(unlocked.trailer.get(b"Encrypt").is_err());
        assert!(unlocked.encryption_state.is_none());
        let page_id = *unlocked.get_pages().get(&1).unwrap();
        let content = unlocked.get_page_content(page_id).unwrap();
        assert!(
            content
                .windows(15)
                .any(|window| window == b"HELLO PLAINTEXT"),
            "the copy must carry readable page content"
        );
        assert_eq!(load_lawpdf_annotations(&copy).unwrap().len(), 1);

        let _ = fs::remove_file(source);
        let _ = fs::remove_file(copy);
    }

    #[test]
    fn signed_pdf_is_refused_in_place_but_copies() {
        let source = temp_pdf_path("signed");
        write_blank_pdf(&source);
        let mut document = Document::load(&source).unwrap();
        assert_eq!(rewrite_hazard(&document), None);
        let signature_id = document.new_object_id();
        document.objects.insert(
            signature_id,
            Object::Dictionary(dictionary! {
                "Type" => Object::Name(b"Sig".to_vec()),
                "Filter" => Object::Name(b"Adobe.PPKLite".to_vec()),
                "ByteRange" => Object::Array(vec![Object::Integer(0), Object::Integer(1)]),
            }),
        );
        document
            .trailer
            .set("LawPDFTestSignature", Object::Reference(signature_id));
        document.save(&source).unwrap();
        let signed = Document::load(&source).unwrap();
        assert_eq!(rewrite_hazard(&signed), Some(RewriteHazard::Signed));

        let before = fs::read(&source).unwrap();
        let error = save_with_annotations(&source, &source, &[highlight_annotation()])
            .expect_err("in-place save into a signed PDF must be refused");
        assert!(
            error.to_string().contains("signed"),
            "unexpected error: {error:#}"
        );
        assert_eq!(fs::read(&source).unwrap(), before);

        let copy = temp_pdf_path("signed-copy");
        let report = save_with_annotations(&source, &copy, &[highlight_annotation()]).unwrap();
        assert!(!report.owner_password_removed);
        assert_eq!(load_lawpdf_annotations(&copy).unwrap().len(), 1);

        let _ = fs::remove_file(source);
        let _ = fs::remove_file(copy);
    }

    #[test]
    fn in_place_save_keeps_a_rolling_backup_of_the_previous_bytes() {
        let backup_root = std::env::temp_dir().join(format!(
            "lawpdf-backups-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let source = temp_pdf_path("backup-source");
        write_blank_pdf(&source);
        let original = fs::read(&source).unwrap();

        let backup = backup_before_rewrite_in(&source, &backup_root)
            .unwrap()
            .unwrap();
        assert!(backup.starts_with(&backup_root));
        assert_eq!(fs::read(&backup).unwrap(), original);
        assert!(
            backup.file_name().and_then(|n| n.to_str()).is_some_and(
                |name| name.ends_with(source.file_name().and_then(|n| n.to_str()).unwrap())
            )
        );

        // A second backup of the same document replaces the first: one rolling
        // copy per path, not one per save.
        fs::write(&source, b"%PDF-1.5 changed").unwrap();
        let again = backup_before_rewrite_in(&source, &backup_root)
            .unwrap()
            .unwrap();
        assert_eq!(again, backup);
        assert_eq!(fs::read(&again).unwrap(), b"%PDF-1.5 changed");
        assert_eq!(fs::read_dir(&backup_root).unwrap().count(), 1);

        // The folder is bounded.
        for index in 0..3 {
            let extra = backup_root.join(format!("extra-{index}.pdf"));
            fs::write(&extra, b"x").unwrap();
        }
        prune_rewrite_backups(&backup_root, 2);
        assert_eq!(fs::read_dir(&backup_root).unwrap().count(), 2);

        let _ = fs::remove_dir_all(backup_root);
        let _ = fs::remove_file(source);
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
