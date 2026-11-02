//! Private print snapshots and the layout shared by preview and printer output.
use crate::document_store::{DocumentStore, FileRevision};
use crate::model::{EditorAnnotation, LoadedDocument};
use image::{Rgba, RgbaImage, imageops};
use lopdf::{Document, Object};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

const MAX_PAGES: usize = 65_535;
const MAX_SHEET_PIXELS: u64 = 32_000_000;
const POINTS_PER_MM: f32 = 72.0 / 25.4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageParity {
    All,
    Odd,
    Even,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaperSize {
    Letter,
    Legal,
    A4,
    A3,
}
impl PaperSize {
    pub fn points(self) -> (f32, f32) {
        match self {
            Self::Letter => (612.0, 792.0),
            Self::Legal => (612.0, 1008.0),
            Self::A4 => (210.0 * POINTS_PER_MM, 297.0 * POINTS_PER_MM),
            Self::A3 => (297.0 * POINTS_PER_MM, 420.0 * POINTS_PER_MM),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScaleMode {
    Fit,
    Shrink,
    Actual,
}
#[derive(Clone, Debug, PartialEq)]
pub struct PrintOptions {
    /// Empty means all pages. User-facing page numbers are one-based.
    pub range: String,
    pub parity: PageParity,
    pub reverse: bool,
    pub copies: u16,
    pub collate: bool,
    pub paper: PaperSize,
    pub landscape: bool,
    pub scale: ScaleMode,
    pub pages_per_sheet: usize,
    pub grayscale: bool,
    pub include_annotations: bool,
    pub margin_mm: f32,
}
impl Default for PrintOptions {
    fn default() -> Self {
        Self {
            range: String::new(),
            parity: PageParity::All,
            reverse: false,
            copies: 1,
            collate: true,
            paper: PaperSize::Letter,
            landscape: false,
            scale: ScaleMode::Fit,
            pages_per_sheet: 1,
            grayscale: false,
            include_annotations: true,
            margin_mm: 6.35,
        }
    }
}
impl PrintOptions {
    pub fn paper_points(&self) -> (f32, f32) {
        let (width, height) = self.paper.points();
        if self.landscape {
            (height, width)
        } else {
            (width, height)
        }
    }
    pub fn selected_pages(&self, page_count: usize) -> Result<Vec<usize>, String> {
        if page_count == 0 || page_count > MAX_PAGES {
            return Err("This document has no printable pages or exceeds the page limit.".into());
        }
        if self.range.len() > 8192 {
            return Err("The page range is too long.".into());
        }
        let mut pages = Vec::new();
        let mut seen = HashSet::new();
        let mut append = |number: usize| {
            if seen.insert(number) {
                pages.push(number - 1);
            }
        };
        if self.range.trim().is_empty() {
            for page in 1..=page_count {
                append(page);
            }
        } else {
            let parse_number = |value: &str| -> Result<usize, String> {
                let value = value.trim();
                if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                    return Err("Use page numbers and ranges, for example: 1–3, 5, 8.".into());
                }
                let number = value
                    .parse::<usize>()
                    .map_err(|_| "A page number is too large.".to_owned())?;
                if number == 0 || number > page_count {
                    return Err(format!(
                        "Page {number} is outside this document (1–{page_count})."
                    ));
                }
                Ok(number)
            };
            let normalized = self.range.replace(['–', '—'], "-");
            let mut expanded = 0usize;
            for part in normalized.split(',') {
                if let Some((start, end)) = part.split_once('-') {
                    let start = parse_number(start)?;
                    let end = parse_number(end)?;
                    if start > end {
                        return Err("Write ranges in ascending order, such as 2–5; use Reverse order to print backwards.".into());
                    }
                    expanded += end - start + 1;
                    if expanded > MAX_PAGES * 4 {
                        return Err(
                            "This range repeats too many pages. Simplify the page selection."
                                .into(),
                        );
                    }
                    for page in start..=end {
                        append(page);
                    }
                } else {
                    append(parse_number(part)?);
                }
            }
        }
        pages.retain(|index| match self.parity {
            PageParity::All => true,
            PageParity::Odd => index % 2 == 0,
            PageParity::Even => index % 2 == 1,
        });
        if self.reverse {
            pages.reverse();
        }
        if pages.is_empty() {
            return Err("No pages match this range and odd/even selection.".into());
        }
        Ok(pages)
    }
    /// Sheets for one copy; the spooler applies copies and collation.
    pub fn sheet_count(&self, page_count: usize) -> Result<usize, String> {
        self.validate_layout()?;
        Ok(self
            .selected_pages(page_count)?
            .len()
            .div_ceil(self.pages_per_sheet))
    }
    fn validate_layout(&self) -> Result<(), String> {
        if !matches!(self.pages_per_sheet, 1 | 2 | 4) {
            return Err("Choose 1, 2, or 4 pages per sheet.".into());
        }
        if !(1..=999).contains(&self.copies) {
            return Err("Choose between 1 and 999 copies.".into());
        }
        if !self.margin_mm.is_finite() || !(0.0..=50.0).contains(&self.margin_mm) {
            return Err("Margins must be between 0 and 50 mm.".into());
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct PrintSource {
    pub document: LoadedDocument,
    pub annotations: Vec<EditorAnnotation>,
    /// An editing session's immutable recovery revision, when available.
    pub revision: Option<FileRevision>,
}
pub struct PreparedPrint {
    pub path: PathBuf,
    pub document: LoadedDocument,
    include_annotations: bool,
    opened: AtomicBool,
    directory: Option<tempfile::TempDir>,
}
impl PreparedPrint {
    pub fn page_count(&self) -> usize {
        self.document.page_count
    }
}
impl Drop for PreparedPrint {
    fn drop(&mut self) {
        // IPC can wait for another native operation. Keep cleanup off the UI
        // thread, and retain the directory until the worker releases its file.
        if self.opened.load(Ordering::Acquire)
            && let Some(directory) = self.directory.take()
        {
            let path = self.path.clone();
            std::thread::spawn(move || {
                if let Ok(engine) = crate::pdf_backend::PdfEngine::new() {
                    engine.close_document(&path);
                }
                drop(directory);
            });
        }
    }
}
pub fn prepare(source: &PrintSource, include_annotations: bool) -> Result<PreparedPrint, String> {
    prepare_with_store(&DocumentStore::new()?, source, include_annotations)
}

fn prepare_with_store(
    store: &DocumentStore,
    source: &PrintSource,
    include_annotations: bool,
) -> Result<PreparedPrint, String> {
    if source.document.page_count == 0
        || source.document.page_count > MAX_PAGES
        || source.document.pages.len() != source.document.page_count
    {
        return Err(
            "The document's page information is incomplete. Reopen it before printing.".into(),
        );
    }
    for page in &source.document.pages {
        if !page.width.is_finite()
            || !page.height.is_finite()
            || !(0.01..=1_000_000.0).contains(&page.width)
            || !(0.01..=1_000_000.0).contains(&page.height)
        {
            return Err("A PDF page has invalid dimensions.".into());
        }
    }
    // The performance cache excludes this temporary directory prefix.
    let directory = tempfile::Builder::new()
        .prefix("lawpdf-print-")
        .tempdir()
        .map_err(|error| format!("Could not create a private print copy: {error}"))?;
    let path = directory.path().join("document.pdf");
    let annotations = if include_annotations {
        source.annotations.as_slice()
    } else {
        &[]
    };
    match &source.revision {
        Some(revision) => {
            // Read permissions from the protected original, before export can
            // strip an owner-password wrapper from the private print copy.
            require_print_permission(&store.print_source(revision)?)?;
            store.export_annotations(&source.document.path, revision, annotations, &path)?;
        }
        None => {
            let revision = FileRevision::read(&source.document.path)?;
            require_print_permission(&source.document.path)?;
            crate::pdf_backend::save_with_annotations_checked(
                &source.document.path,
                &path,
                annotations,
                &revision,
            )
            .map_err(|error| format!("Could not prepare the PDF for printing: {error:#}"))?;
        }
    }
    let mut snapshot = Document::load(&path)
        .map_err(|error| format!("Could not verify the print copy: {error}"))?;
    if snapshot.get_pages().len() != source.document.page_count {
        return Err("The PDF's pages changed. Reopen it before printing.".into());
    }
    if !include_annotations {
        remove_markup(&mut snapshot)?;
        snapshot.prune_objects();
        snapshot
            .save(&path)
            .map_err(|error| format!("Could not prepare the unmarked print copy: {error}"))?;
    }
    let mut document = source.document.clone();
    document.path = path.clone();
    Ok(PreparedPrint {
        path,
        document,
        include_annotations,
        opened: AtomicBool::new(false),
        directory: Some(directory),
    })
}

fn require_print_permission(path: &std::path::Path) -> Result<(), String> {
    match crate::pdf_backend::allows_printing_at(path) {
        Ok(true) => Ok(()),
        Ok(false) => Err("This PDF does not allow printing.".into()),
        Err(error) => Err(format!("Could not read this PDF for printing: {error:#}")),
    }
}
/// Remove saved markup, including third-party markup. Retain widgets so that
/// disabling annotations does not discard a filled form's answers.
fn remove_markup(document: &mut Document) -> Result<(), String> {
    for page_id in document.get_pages().values() {
        let page = document
            .get_dictionary(*page_id)
            .map_err(|error| error.to_string())?;
        let annotations = match page.get(b"Annots") {
            Ok(value) => document
                .dereference(value)
                .and_then(|(_, value)| value.as_array())
                .map_err(|error| format!("Could not read page annotations: {error}"))?
                .clone(),
            Err(_) => continue,
        };
        let mut widgets = Vec::new();
        for annotation in annotations {
            let (_, object) = document
                .dereference(&annotation)
                .map_err(|error| format!("Could not read a PDF annotation: {error}"))?;
            let is_widget = object
                .as_dict()
                .ok()
                .and_then(|dictionary| dictionary.get(b"Subtype").ok())
                .and_then(|subtype| subtype.as_name().ok())
                .is_some_and(|subtype| subtype == b"Widget");
            if is_widget {
                widgets.push(annotation);
            }
        }
        let page = document
            .get_object_mut(*page_id)
            .and_then(Object::as_dict_mut)
            .map_err(|error| error.to_string())?;
        if widgets.is_empty() {
            page.remove(b"Annots");
        } else {
            page.set("Annots", Object::Array(widgets));
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
struct Rect {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}
fn sheet_cells(options: &PrintOptions) -> Result<Vec<Rect>, String> {
    options.validate_layout()?;
    let (width, height) = options.paper_points();
    let (columns, rows) = match options.pages_per_sheet {
        1 => (1, 1),
        2 if options.landscape => (2, 1),
        2 => (1, 2),
        _ => (2, 2),
    };
    let margin = options.margin_mm * POINTS_PER_MM;
    let gap = if options.pages_per_sheet > 1 {
        12.0
    } else {
        0.0
    };
    let cell_width = (width - 2.0 * margin - gap * (columns - 1) as f32) / columns as f32;
    let cell_height = (height - 2.0 * margin - gap * (rows - 1) as f32) / rows as f32;
    if cell_width < 36.0 || cell_height < 36.0 {
        return Err("These margins leave too little space to print.".into());
    }
    Ok((0..rows)
        .flat_map(|row| {
            (0..columns).map(move |column| Rect {
                x: margin + column as f32 * (cell_width + gap),
                y: margin + row as f32 * (cell_height + gap),
                width: cell_width,
                height: cell_height,
            })
        })
        .collect())
}
fn page_placement(cell: Rect, width: f32, height: f32, mode: ScaleMode) -> (Rect, f32) {
    let fit = (cell.width / width).min(cell.height / height);
    let scale = match mode {
        ScaleMode::Fit => fit,
        ScaleMode::Shrink => fit.min(1.0),
        ScaleMode::Actual => 1.0,
    };
    let width = width * scale;
    let height = height * scale;
    (
        Rect {
            x: cell.x + (cell.width - width) / 2.0,
            y: cell.y + (cell.height - height) / 2.0,
            width,
            height,
        },
        scale,
    )
}
fn sheet_pixels(options: &PrintOptions, dpi: f32) -> Result<(u32, u32), String> {
    if !dpi.is_finite() || !(36.0..=600.0).contains(&dpi) {
        return Err("Print resolution must be between 36 and 600 dpi.".into());
    }
    let (width, height) = options.paper_points();
    let width = (width * dpi / 72.0).round() as u32;
    let height = (height * dpi / 72.0).round() as u32;
    if u64::from(width) * u64::from(height) > MAX_SHEET_PIXELS || width > 16_384 || height > 16_384
    {
        return Err(
            "This paper and resolution combination is too large. Choose a lower print resolution."
                .into(),
        );
    }
    Ok((width, height))
}
pub fn render_sheet(
    prepared: &PreparedPrint,
    options: &PrintOptions,
    sheet_index: usize,
    dpi: f32,
) -> Result<RgbaImage, String> {
    if prepared.include_annotations != options.include_annotations {
        return Err("The print preview is updating. Wait for it to finish.".into());
    }
    let cells = sheet_cells(options)?;
    let pages = options.selected_pages(prepared.page_count())?;
    let start = sheet_index
        .checked_mul(options.pages_per_sheet)
        .filter(|start| *start < pages.len())
        .ok_or_else(|| "This sheet is outside the selected page range.".to_owned())?;
    let (width, height) = sheet_pixels(options, dpi)?;
    let mut sheet = RgbaImage::from_pixel(width, height, Rgba([255; 4]));
    let engine = crate::pdf_backend::PdfEngine::new()
        .map_err(|error| format!("Could not start the PDF renderer: {error:#}"))?;
    prepared.opened.store(true, Ordering::Release);
    for (page_index, cell) in pages[start..].iter().take(cells.len()).zip(cells) {
        let page = &prepared.document.pages[*page_index];
        let (placement, scale) = page_placement(cell, page.width, page.height, options.scale);
        let rendered = engine
            .render_page_internal(&prepared.path, *page_index, dpi / 72.0 * scale,
                crate::pdf_backend::RenderQuality::Print, false)
            .map_err(|error| format!("Could not render page {}: {error:#}", page_index + 1))?;
        let page_image =
            RgbaImage::from_raw(rendered.width as u32, rendered.height as u32, rendered.rgba)
                .ok_or_else(|| "The PDF renderer returned an invalid image.".to_owned())?;
        compose_page(&mut sheet, &page_image, cell, placement, dpi);
    }
    if options.grayscale {
        to_grayscale(&mut sheet);
    }
    Ok(sheet)
}
/// Clip before resizing, so actual-size output of huge pages never allocates a
/// bitmap larger than the visible paper cell.
fn compose_page(sheet: &mut RgbaImage, page: &RgbaImage, cell: Rect, placement: Rect, dpi: f32) {
    let pixels = dpi / 72.0;
    let left = placement.x.max(cell.x);
    let top = placement.y.max(cell.y);
    let right = (placement.x + placement.width).min(cell.x + cell.width);
    let bottom = (placement.y + placement.height).min(cell.y + cell.height);
    let x = (left * pixels).round().clamp(0.0, sheet.width() as f32) as u32;
    let y = (top * pixels).round().clamp(0.0, sheet.height() as f32) as u32;
    let end_x = (right * pixels)
        .round()
        .clamp(x as f32, sheet.width() as f32) as u32;
    let end_y = (bottom * pixels)
        .round()
        .clamp(y as f32, sheet.height() as f32) as u32;
    if x == end_x || y == end_y || page.width() == 0 || page.height() == 0 {
        return;
    }
    let source_x = ((left - placement.x) / placement.width * page.width() as f32)
        .floor()
        .clamp(0.0, (page.width() - 1) as f32) as u32;
    let source_y = ((top - placement.y) / placement.height * page.height() as f32)
        .floor()
        .clamp(0.0, (page.height() - 1) as f32) as u32;
    let source_end_x = ((right - placement.x) / placement.width * page.width() as f32)
        .ceil()
        .clamp((source_x + 1) as f32, page.width() as f32) as u32;
    let source_end_y = ((bottom - placement.y) / placement.height * page.height() as f32)
        .ceil()
        .clamp((source_y + 1) as f32, page.height() as f32) as u32;
    let crop = imageops::crop_imm(
        page,
        source_x,
        source_y,
        source_end_x - source_x,
        source_end_y - source_y,
    )
    .to_image();
    let resized = imageops::resize(&crop, end_x - x, end_y - y, imageops::FilterType::Lanczos3);
    imageops::overlay(sheet, &resized, i64::from(x), i64::from(y));
}
fn to_grayscale(image: &mut RgbaImage) {
    for pixel in image.pixels_mut() {
        let gray = ((u32::from(pixel[0]) * 299
            + u32::from(pixel[1]) * 587
            + u32::from(pixel[2]) * 114
            + 500)
            / 1000) as u8;
        pixel[0] = gray;
        pixel[1] = gray;
        pixel[2] = gray;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AnnotationKind, PageInfo, PdfRect};
    use lopdf::dictionary;
    #[test]
    fn ranges_preserve_order_deduplicate_then_apply_parity_and_reverse() {
        let mut options = PrintOptions {
            range: "5, 1–3, 2, 8-9".into(),
            ..Default::default()
        };
        assert_eq!(options.selected_pages(10).unwrap(), vec![4, 0, 1, 2, 7, 8]);
        options.parity = PageParity::Odd;
        options.reverse = true;
        assert_eq!(options.selected_pages(10).unwrap(), vec![8, 2, 0, 4]);
        options.pages_per_sheet = 4;
        assert_eq!(options.sheet_count(10).unwrap(), 1);
    }
    #[test]
    fn malformed_or_unbounded_ranges_are_rejected() {
        for range in [
            "0",
            "8",
            "2-1",
            "1-",
            "-2",
            "1,,2",
            "1,",
            "1-2-3",
            "+2",
            "9999999999999999999999999",
        ] {
            assert!(
                PrintOptions {
                    range: range.into(),
                    ..Default::default()
                }
                .selected_pages(7)
                .is_err(),
                "{range}"
            );
        }
        assert!(PrintOptions::default().selected_pages(0).is_err());
        assert!(PrintOptions::default().selected_pages(usize::MAX).is_err());
        assert!(
            PrintOptions {
                range: "2".into(),
                parity: PageParity::Odd,
                ..Default::default()
            }
            .selected_pages(2)
            .is_err()
        );
    }
    #[test]
    fn n_up_geometry_stays_inside_margins_in_both_orientations() {
        for landscape in [false, true] {
            for pages_per_sheet in [1, 2, 4] {
                let options = PrintOptions {
                    landscape,
                    pages_per_sheet,
                    ..Default::default()
                };
                let cells = sheet_cells(&options).unwrap();
                let (width, height) = options.paper_points();
                assert_eq!(cells.len(), pages_per_sheet);
                for cell in &cells {
                    assert!(cell.x >= 17.99 && cell.y >= 17.99);
                    assert!(cell.x + cell.width <= width - 17.99);
                    assert!(cell.y + cell.height <= height - 17.99);
                }
                for (index, first) in cells.iter().enumerate() {
                    for second in &cells[index + 1..] {
                        assert!(
                            first.x + first.width <= second.x
                                || second.x + second.width <= first.x
                                || first.y + first.height <= second.y
                                || second.y + second.height <= first.y
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn scaling_preserves_aspect_and_actual_size_clips_without_touching_margins() {
        let cell = Rect {
            x: 10.0,
            y: 10.0,
            width: 80.0,
            height: 80.0,
        };
        let (fit, scale) = page_placement(cell, 160.0, 80.0, ScaleMode::Fit);
        assert_eq!(scale, 0.5);
        assert_eq!((fit.width, fit.height, fit.y), (80.0, 40.0, 30.0));
        assert_eq!(page_placement(cell, 40.0, 40.0, ScaleMode::Shrink).1, 1.0);
        let (actual, scale) = page_placement(cell, 160.0, 80.0, ScaleMode::Actual);
        assert_eq!(scale, 1.0);
        let mut sheet = RgbaImage::from_pixel(100, 100, Rgba([255; 4]));
        let page = RgbaImage::from_pixel(160, 80, Rgba([255, 0, 0, 255]));
        compose_page(&mut sheet, &page, cell, actual, 72.0);
        assert_eq!(sheet.get_pixel(10, 10).0, [255, 0, 0, 255]);
        assert_eq!(sheet.get_pixel(9, 50).0, [255; 4]);
        assert_eq!(sheet.get_pixel(90, 50).0, [255; 4]);
        to_grayscale(&mut sheet);
        assert_eq!(sheet.get_pixel(50, 50).0, [76, 76, 76, 255]);
    }
    #[test]
    fn raster_limits_reject_oversized_or_invalid_allocations() {
        let options = PrintOptions {
            paper: PaperSize::A3,
            ..Default::default()
        };
        assert!(sheet_pixels(&options, 300.0).is_ok());
        assert!(sheet_pixels(&options, 600.0).is_err());
        assert!(sheet_pixels(&options, f32::NAN).is_err());
        assert!(
            sheet_cells(&PrintOptions {
                margin_mm: f32::INFINITY,
                ..Default::default()
            })
            .is_err()
        );
        assert!(
            sheet_cells(&PrintOptions {
                pages_per_sheet: usize::MAX,
                ..Default::default()
            })
            .is_err()
        );
    }
    fn fixture() -> (tempfile::TempDir, PrintSource) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("original.pdf");
        let mut pdf = Document::with_version("1.5");
        let pages = pdf.new_object_id();
        let note = pdf.add_object(dictionary! { "Type" => "Annot", "Subtype" => "Text", "Rect" => vec![10.into(), 10.into(), 30.into(), 30.into()], "Contents" => Object::string_literal("Foreign annotation") });
        let widget = pdf.add_object(dictionary! { "Type" => "Annot", "Subtype" => "Widget", "FT" => "Tx", "T" => Object::string_literal("Name"), "V" => Object::string_literal("Keep answer"), "Rect" => vec![40.into(), 10.into(), 100.into(), 30.into()] });
        let page = pdf.add_object(dictionary! { "Type" => "Page", "Parent" => pages, "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()], "Resources" => dictionary! {}, "Annots" => vec![note.into(), widget.into()] });
        pdf.objects.insert(
            pages,
            dictionary! { "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1 }.into(),
        );
        let catalog = pdf.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages });
        pdf.trailer.set("Root", catalog);
        pdf.save(&path).unwrap();
        let document = LoadedDocument {
            path,
            title: "original.pdf".into(),
            page_count: 1,
            pages: vec![PageInfo::new(612.0, 792.0)],
            native_text: vec![String::new()],
            native_text_loaded: vec![false],
            text_chars: vec![None],
            links: vec![vec![]],
            links_loaded: false,
            optimized: true,
        };
        let annotation = EditorAnnotation {
            page_index: 0,
            rect: PdfRect::new(50.0, 100.0, 150.0, 130.0),
            kind: AnnotationKind::TextBox {
                text: "Unsaved note".into(),
                font_size: 12.0,
                color_rgb: [0.0; 3],
            },
        };
        (
            directory,
            PrintSource {
                document,
                annotations: vec![annotation],
                revision: None,
            },
        )
    }
    #[test]
    fn snapshot_includes_unsaved_notes_without_changing_source_and_is_removed_on_drop() {
        let (_directory, source) = fixture();
        let original = std::fs::read(&source.document.path).unwrap();
        let prepared = prepare(&source, true).unwrap();
        let snapshot = Document::load(&prepared.path).unwrap();
        let page = *snapshot.get_pages().values().next().unwrap();
        assert_eq!(
            snapshot
                .get_dictionary(page)
                .unwrap()
                .get(b"Annots")
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            3
        );
        assert_eq!(std::fs::read(&source.document.path).unwrap(), original);
        let path = prepared.path.clone();
        drop(prepared);
        assert!(!path.exists());
    }
    #[test]
    fn selected_recovery_revision_survives_external_edits_and_rejects_snapshot_damage() {
        let (directory, mut source) = fixture();
        let store = DocumentStore::for_print_test(directory.path().join("recovery"));
        let revision = store.capture(&source.document.path).unwrap();
        source.revision = Some(revision.clone());
        std::fs::write(&source.document.path, b"new external revision").unwrap();
        let prepared = prepare_with_store(&store, &source, true).unwrap();
        assert_eq!(Document::load(&prepared.path).unwrap().get_pages().len(), 1);
        assert_eq!(
            crate::pdf_backend::load_lawpdf_annotations(&prepared.path).unwrap(),
            source.annotations
        );
        assert_eq!(
            std::fs::read(&source.document.path).unwrap(),
            b"new external revision"
        );
        let snapshot = store.print_source(&revision).unwrap();
        std::fs::write(snapshot, b"damaged source snapshot").unwrap();
        assert!(prepare_with_store(&store, &source, true).is_err());
    }

    #[test]
    fn protected_source_print_permissions_are_checked_before_annotation_export() {
        let (directory, mut source) = fixture();
        let mut pdf = Document::load(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/owner-password.pdf"),
        )
        .unwrap();
        let mut permissions = lopdf::Permissions::all();
        permissions.remove(lopdf::Permissions::PRINTABLE);
        let state = lopdf::EncryptionState::try_from(lopdf::EncryptionVersion::V2 {
            document: &pdf,
            owner_password: "lawpdf-print-test",
            user_password: "",
            key_length: 128,
            permissions,
        })
        .unwrap();
        pdf.encrypt(&state).unwrap();
        pdf.save(&source.document.path).unwrap();
        let original = std::fs::read(&source.document.path).unwrap();
        assert!(
            prepare(&source, true)
                .err()
                .unwrap()
                .contains("does not allow printing")
        );
        let store = DocumentStore::for_print_test(directory.path().join("recovery"));
        source.revision = Some(store.capture(&source.document.path).unwrap());
        assert!(
            prepare_with_store(&store, &source, true)
                .err()
                .unwrap()
                .contains("does not allow printing")
        );
        assert_eq!(std::fs::read(&source.document.path).unwrap(), original);
    }

    #[test]
    fn native_print_render_preserves_content_markup_visibility_and_sheet_geometry() {
        let (_directory, mut source) = fixture();
        let mut pdf = Document::load(&source.document.path).unwrap();
        let page = *pdf.get_pages().values().next().unwrap();
        let font = pdf.add_object(
            dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" },
        );
        let content = pdf.add_object(lopdf::Stream::new(dictionary! {},
            b"q 0.1 0.3 0.8 rg 72 500 200 80 re f Q BT /F1 24 Tf 0 0 0 rg 72 650 Td (LawPDF print proof) Tj ET".to_vec()));
        let mut annotations = pdf
            .get_dictionary(page)
            .unwrap()
            .get(b"Annots")
            .unwrap()
            .as_array()
            .unwrap()
            .clone();
        // Green is print-only (Print + NoView); red is screen-only. Blue is
        // hidden even though its Print flag is set. These must match paper.
        for (x, y, flags, color) in [
            (400, 650, 4 | 32, "0 0.8 0"),
            (480, 650, 0, "1 0 0"),
            (400, 580, 4 | 2, "0 0 1"),
        ] {
            let appearance = pdf.add_object(lopdf::Stream::new(
                dictionary! {
                    "Type" => "XObject", "Subtype" => "Form",
                    "BBox" => vec![0.into(), 0.into(), 50.into(), 50.into()],
                    "Resources" => dictionary! {},
                },
                format!("{color} rg 0 0 50 50 re f").into_bytes(),
            ));
            let annotation = pdf.add_object(dictionary! {
                "Type" => "Annot", "Subtype" => "Square", "F" => flags,
                "Rect" => vec![x.into(), y.into(), (x + 50).into(), (y + 50).into()],
                "AP" => dictionary! { "N" => appearance },
            });
            annotations.push(annotation.into());
        }
        let page_dictionary = pdf.get_object_mut(page).unwrap().as_dict_mut().unwrap();
        page_dictionary.set("Contents", content);
        page_dictionary.set(
            "Resources",
            dictionary! { "Font" => dictionary! { "F1" => font } },
        );
        page_dictionary.set("Annots", annotations);
        pdf.save(&source.document.path).unwrap();
        source.annotations.push(EditorAnnotation {
            page_index: 0,
            rect: PdfRect::new(72.0, 600.0, 300.0, 625.0),
            kind: AnnotationKind::Marker {
                color_rgb: [1.0, 1.0, 0.0],
                opacity: 0.7,
                style: crate::model::MarkerStyle::Highlight,
            },
        });
        let original = std::fs::read(&source.document.path).unwrap();
        let prepared = prepare(&source, true).unwrap();
        let mut options = PrintOptions {
            margin_mm: 0.0,
            scale: ScaleMode::Actual,
            ..Default::default()
        };
        let color = render_sheet(&prepared, &options, 0, 72.0).unwrap();
        assert_eq!(color.dimensions(), (612, 792));
        let base_blue = color.get_pixel(100, 252);
        assert!(base_blue[2] > 180 && base_blue[0] < 80);
        let yellow = color.get_pixel(100, 182);
        assert!(
            yellow[0] > 200 && yellow[1] > 200 && yellow[2] < 160,
            "unsaved printable markup must be visible: {yellow:?}"
        );
        let green = color.get_pixel(425, 117);
        assert!(
            green[1] > 150 && green[0] < 50 && green[2] < 50,
            "Print + NoView annotation must appear on paper: {green:?}"
        );
        assert_eq!(
            color.get_pixel(505, 117).0,
            [255; 4],
            "screen-only annotation must not print"
        );
        assert_eq!(
            color.get_pixel(425, 187).0,
            [255; 4],
            "hidden annotation must not print"
        );
        let text_pixels = (110..150)
            .flat_map(|y| (70..300).map(move |x| (x, y)))
            .filter(|&(x, y)| color.get_pixel(x, y)[0] < 100)
            .count();
        assert!(text_pixels > 100, "native text must reach the sheet raster");
        options.grayscale = true;
        let grayscale = render_sheet(&prepared, &options, 0, 72.0).unwrap();
        assert!(
            grayscale
                .pixels()
                .all(|pixel| pixel[0] == pixel[1] && pixel[1] == pixel[2])
        );
        options.grayscale = false;
        let high_resolution = render_sheet(&prepared, &options, 0, 300.0).unwrap();
        assert_eq!(high_resolution.dimensions(), (2550, 3300));
        options.pages_per_sheet = 4;
        options.landscape = true;
        options.margin_mm = 6.35;
        options.scale = ScaleMode::Fit;
        let n_up = render_sheet(&prepared, &options, 0, 72.0).unwrap();
        assert_eq!(n_up.dimensions(), (792, 612));
        assert!(
            n_up.enumerate_pixels()
                .filter(|(x, y, _)| *x > 396 || *y > 306)
                .all(|(_, _, pixel)| pixel.0 == [255; 4]),
            "unused n-up slots must be blank"
        );
        let unmarked = prepare(&source, false).unwrap();
        let clean_options = PrintOptions {
            include_annotations: false,
            margin_mm: 0.0,
            scale: ScaleMode::Actual,
            ..Default::default()
        };
        let clean = render_sheet(&unmarked, &clean_options, 0, 72.0).unwrap();
        assert_eq!(clean.get_pixel(100, 182).0, [255; 4]);
        assert_eq!(clean.get_pixel(425, 117).0, [255; 4]);
        assert_eq!(clean.get_pixel(100, 252), base_blue);
        assert_eq!(std::fs::read(&source.document.path).unwrap(), original);
        // Opt-in evidence contains only this synthetic fixture, never user PDFs.
        if let Some(output) = std::env::var_os("LAWPDF_PRINT_QA_DIR") {
            let output = PathBuf::from(output);
            std::fs::create_dir_all(&output).unwrap();
            color.save(output.join("print-color.png")).unwrap();
            grayscale.save(output.join("print-grayscale.png")).unwrap();
            n_up.save(output.join("print-four-up.png")).unwrap();
            clean.save(output.join("print-without-markup.png")).unwrap();
            high_resolution
                .save(output.join("print-300dpi.png"))
                .unwrap();
            std::fs::write(output.join("synthetic-source.pdf"), original).unwrap();
        }
    }

    #[test]
    fn unmarked_snapshot_removes_foreign_notes_but_keeps_filled_form_widgets() {
        let (_directory, source) = fixture();
        let original = std::fs::read(&source.document.path).unwrap();
        let prepared = prepare(&source, false).unwrap();
        let snapshot = Document::load(&prepared.path).unwrap();
        let page = *snapshot.get_pages().values().next().unwrap();
        let annotations = snapshot
            .get_dictionary(page)
            .unwrap()
            .get(b"Annots")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(annotations.len(), 1);
        let (_, widget) = snapshot.dereference(&annotations[0]).unwrap();
        assert_eq!(
            widget
                .as_dict()
                .unwrap()
                .get(b"Subtype")
                .unwrap()
                .as_name()
                .unwrap(),
            b"Widget"
        );
        assert_eq!(
            widget
                .as_dict()
                .unwrap()
                .get(b"V")
                .unwrap()
                .as_str()
                .unwrap(),
            b"Keep answer"
        );
        assert_eq!(std::fs::read(&source.document.path).unwrap(), original);
    }
}
