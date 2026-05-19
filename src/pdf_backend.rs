use std::cell::RefCell;
use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use image::RgbaImage;
use lopdf::{
    Dictionary, Document, Object, ObjectId, Stream, StringFormat,
    content::{Content, Operation},
    dictionary,
};
use pdfium_render::prelude::*;

use crate::model::{
    AnnotationKind, EditorAnnotation, LoadedDocument, MarkerStyle, PageInfo, PageTextChar, PdfRect,
    RenderedPage,
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

const OPEN_DOCUMENT_CACHE_CAP: usize = 3;

impl PdfEngine {
    pub fn new() -> Result<Self> {
        let bindings = bind_pdfium()?;
        let pdfium = Box::leak(Box::new(Pdfium::new(bindings)));

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

            for page_index in 0..page_count {
                let page = document
                    .pages()
                    .get(page_index as u16)
                    .with_context(|| format!("failed to read page {}", page_index + 1))?;

                pages.push(PageInfo::new(page.width().value, page.height().value));
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

            Some(PageTextChar { ch, rect })
        })
        .collect()
}

fn bind_pdfium() -> Result<Box<dyn PdfiumLibraryBindings>> {
    for candidate in pdfium_candidates() {
        if candidate.exists() {
            return Pdfium::bind_to_library(candidate.to_string_lossy().to_string())
                .with_context(|| format!("failed to bind PDFium from {}", candidate.display()));
        }
    }

    Pdfium::bind_to_system_library().context(
        "failed to bind PDFium; put pdfium.dll beside the executable, in vendor\\pdfium.dll, on PATH, or set PDFIUM_DYNAMIC_LIB_PATH",
    )
}

fn pdfium_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Ok(path) = std::env::var("PDFIUM_DYNAMIC_LIB_PATH") {
        if !path.trim().is_empty() {
            candidates.push(PathBuf::from(path));
        }
    }

    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            candidates.push(exe_dir.join("pdfium.dll"));
        }
    }

    if let Ok(current_dir) = std::env::current_dir() {
        candidates.push(current_dir.join("pdfium.dll"));
        candidates.push(current_dir.join("vendor").join("pdfium.dll"));
    }

    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("vendor")
            .join("pdfium.dll"),
    );
    candidates
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
    document
        .save(destination)
        .with_context(|| format!("failed to save {}", destination.display()))?;

    Ok(())
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

pub fn sidecar_path_for_export(source: &Path, suffix: &str, extension: &str) -> PathBuf {
    let stem = source
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("document");
    source.with_file_name(format!("{stem}-{suffix}.{extension}"))
}
