//! Read other applications' annotations without adding them to editable state.
use super::*;
use crate::document_store::{FileRevision, MAX_DOCUMENT_BYTES, read_limited};
use crate::model::{ForeignAnnotation, ForeignAnnotationKind};

const MAX_FOREIGN_MARKS: usize = 100_000;
const MAX_QUADS_PER_MARK: usize = 10_000;

pub fn load_foreign_annotations_checked(
    source: &Path,
    expected: &FileRevision,
) -> Result<Vec<ForeignAnnotation>> {
    let bytes = read_limited(source, MAX_DOCUMENT_BYTES).map_err(anyhow::Error::msg)?;
    anyhow::ensure!(
        FileRevision::from_bytes(&bytes) == *expected,
        "The PDF changed outside LawPDF. Reopen it to load annotations from the new revision."
    );
    let document = Document::load_mem(&bytes).context("Could not read PDF annotations")?;
    foreign_annotations(&document)
}

fn foreign_annotations(document: &Document) -> Result<Vec<ForeignAnnotation>> {
    let mut result = Vec::new();
    for (number, page_id) in document.get_pages() {
        let page_index = number.saturating_sub(1) as usize;
        let page = document.get_object(page_id)?.as_dict()?;
        let Ok(annots) = page.get(b"Annots") else {
            continue;
        };
        let annots = match annots {
            Object::Reference(id) => document.get_object(*id)?,
            value => value,
        };
        let items = match annots {
            Object::Array(items) => items.as_slice(),
            value => std::slice::from_ref(value),
        };
        let mut geometry = None;
        for (ordinal, object) in items.iter().enumerate() {
            let Some(dict) = annotation_dict(document, object) else {
                continue;
            };
            if is_lawpdf_owned_dict(dict) {
                continue;
            }
            let kind = match dict
                .get(b"Subtype")
                .ok()
                .and_then(pdf_object_text)
                .as_deref()
            {
                Some("Highlight") => ForeignAnnotationKind::Highlight,
                Some("Underline") => ForeignAnnotationKind::Underline,
                Some("Squiggly") => ForeignAnnotationKind::Squiggly,
                Some("StrikeOut") => ForeignAnnotationKind::StrikeOut,
                Some("Text") => ForeignAnnotationKind::Comment,
                Some("FreeText") => ForeignAnnotationKind::TextBox,
                _ => continue,
            };
            let mut rects = Vec::new();
            let mut precise_text_geometry = false;
            if matches!(
                kind,
                ForeignAnnotationKind::Highlight
                    | ForeignAnnotationKind::Underline
                    | ForeignAnnotationKind::Squiggly
                    | ForeignAnnotationKind::StrikeOut
            ) && let Ok(value) = dict.get(b"QuadPoints")
            {
                let value = match value {
                    Object::Reference(id) => document.get_object(*id).ok(),
                    value => Some(value),
                };
                if let Some(points) = value.and_then(|value| value.as_array().ok()) {
                    anyhow::ensure!(
                        points.len() <= MAX_QUADS_PER_MARK * 8,
                        "Too many annotation quadrilaterals to index safely"
                    );
                    if !points.is_empty() && points.len() % 8 == 0 {
                        let quads = points
                            .chunks_exact(8)
                            .map(|quad| {
                                let values =
                                    quad.iter().map(pdf_number).collect::<Option<Vec<_>>>()?;
                                if !values.iter().all(|value| value.is_finite()) {
                                    return None;
                                }
                                let xs = [values[0], values[2], values[4], values[6]];
                                let ys = [values[1], values[3], values[5], values[7]];
                                let rect = PdfRect::new(
                                    xs.into_iter().fold(f32::INFINITY, f32::min),
                                    ys.into_iter().fold(f32::INFINITY, f32::min),
                                    xs.into_iter().fold(f32::NEG_INFINITY, f32::max),
                                    ys.into_iter().fold(f32::NEG_INFINITY, f32::max),
                                );
                                (rect.width() > 0.0 && rect.height() > 0.0).then_some(rect)
                            })
                            .collect::<Option<Vec<_>>>();
                        if let Some(quads) = quads {
                            rects = quads;
                            precise_text_geometry = true;
                        }
                    }
                }
            }
            if rects.is_empty()
                && let Some(rect) = dict.get(b"Rect").ok().and_then(pdf_rect_from_object)
                && [rect.left, rect.bottom, rect.right, rect.top]
                    .into_iter()
                    .all(f32::is_finite)
                && rect.width() > 0.0
                && rect.height() > 0.0
            {
                rects.push(rect);
            }
            if rects.is_empty() {
                continue;
            }
            let geometry = match geometry {
                Some(geometry) => geometry,
                None => {
                    let value = pdf_page_geometry(document, page_id)?;
                    geometry = Some(value);
                    value
                }
            };
            for rect in &mut rects {
                *rect = geometry.rect(*rect, false);
            }
            let text = dict
                .get(b"Contents")
                .ok()
                .and_then(pdf_object_text)
                .unwrap_or_default();
            let author = dict
                .get(b"T")
                .ok()
                .and_then(pdf_object_text)
                .unwrap_or_default();
            let color_rgb = dict
                .get(b"C")
                .ok()
                .and_then(pdf_color_from_object)
                .unwrap_or([0.42, 0.47, 0.58]);
            let key = comment_reference_key(document, object)
                .unwrap_or_else(|| format!("doc-inline-{page_index}-{ordinal}"));
            anyhow::ensure!(
                result.len() < MAX_FOREIGN_MARKS,
                "Too many annotations to index safely"
            );
            result.push(ForeignAnnotation {
                page_index,
                rects,
                precise_text_geometry,
                kind,
                text,
                author,
                color_rgb,
                key,
            });
        }
    }
    result.sort_by(|a, b| {
        a.page_index
            .cmp(&b.page_index)
            .then_with(|| b.rects[0].top.total_cmp(&a.rects[0].top))
            .then_with(|| a.key.cmp(&b.key))
    });
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(path: &Path, rotate: bool) {
        let mut document = Document::with_version("1.5");
        let catalog = document.new_object_id();
        let pages = document.new_object_id();
        let page = document.new_object_id();
        let highlight = document.add_object(dictionary! { "Type" => "Annot", "Subtype" => "Highlight",
            "Rect" => vec![50.into(), 650.into(), 150.into(), 700.into()],
            "QuadPoints" => vec![50.into(), 700.into(), 150.into(), 700.into(), 50.into(), 690.into(), 150.into(), 690.into(),
                50.into(), 670.into(), 120.into(), 670.into(), 50.into(), 660.into(), 120.into(), 660.into()] });
        let comment = document.add_object(dictionary! { "Type" => "Annot", "Subtype" => "Text",
            "Rect" => vec![20.into(), 400.into(), 40.into(), 420.into()], "Contents" => Object::string_literal("External comment") });
        let owned = document.add_object(
            dictionary! { "Type" => "Annot", "Subtype" => "Underline", "LawPDF" => true,
            "Rect" => vec![20.into(), 500.into(), 80.into(), 515.into()] },
        );
        document.objects.insert(
            catalog,
            Object::Dictionary(dictionary! { "Type" => "Catalog", "Pages" => pages }),
        );
        document.objects.insert(pages, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => vec![Object::Reference(page)], "Count" => 1 }));
        let mut page_dict = dictionary! { "Type" => "Page", "Parent" => pages, "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        "CropBox" => vec![20.into(), 40.into(), 600.into(), 760.into()], "Resources" => dictionary! {},
        "Annots" => vec![Object::Reference(highlight), Object::Reference(comment), Object::Reference(owned)] };
        if rotate {
            page_dict.set("Rotate", 90);
        }
        document.objects.insert(page, Object::Dictionary(page_dict));
        document.trailer.set("Root", catalog);
        document.save(path).unwrap();
    }

    #[test]
    fn imports_foreign_quads_and_comments_without_claiming_owned_marks() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("annotated.pdf");
        fixture(&path, false);
        let before = std::fs::read(&path).unwrap();
        let revision = FileRevision::read(&path).unwrap();
        let marks = load_foreign_annotations_checked(&path, &revision).unwrap();
        assert_eq!(marks.len(), 2);
        assert_eq!(marks[0].kind, ForeignAnnotationKind::Highlight);
        assert_eq!(marks[0].rects.len(), 2);
        assert!(marks[0].precise_text_geometry);
        assert_eq!(marks[0].rects[0], PdfRect::new(30.0, 650.0, 130.0, 660.0));
        assert_eq!(marks[1].kind, ForeignAnnotationKind::Comment);
        assert_eq!(marks[1].text, "External comment");
        assert_eq!(std::fs::read(&path).unwrap(), before);
        std::fs::write(&path, b"changed externally").unwrap();
        assert!(load_foreign_annotations_checked(&path, &revision).is_err());
    }

    #[test]
    fn foreign_quads_follow_cropped_rotated_page_geometry() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("rotated.pdf");
        fixture(&path, true);
        let revision = FileRevision::read(&path).unwrap();
        let marks = load_foreign_annotations_checked(&path, &revision).unwrap();
        let highlight = marks
            .iter()
            .find(|mark| mark.kind == ForeignAnnotationKind::Highlight)
            .unwrap();
        let geometry = crate::page_geometry::PageGeometry {
            bounds: PdfRect::new(20.0, 40.0, 600.0, 760.0),
            rotation: 90,
        };
        assert_eq!(
            highlight.rects[0],
            geometry.rect(PdfRect::new(50.0, 690.0, 150.0, 700.0), false)
        );
    }

    #[test]
    fn saving_an_owned_mark_preserves_foreign_marks_even_without_contents() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("annotated.pdf");
        fixture(&path, false);
        let before =
            load_foreign_annotations_checked(&path, &FileRevision::read(&path).unwrap()).unwrap();
        let own = EditorAnnotation {
            page_index: 0,
            rect: PdfRect::new(100.0, 300.0, 150.0, 315.0),
            kind: AnnotationKind::Marker {
                color_rgb: [1.0, 1.0, 0.0],
                opacity: 0.4,
                style: MarkerStyle::Highlight,
            },
        };
        save_with_annotations(&path, &path, &[own]).unwrap();
        let after =
            load_foreign_annotations_checked(&path, &FileRevision::read(&path).unwrap()).unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn missing_or_incomplete_quads_do_not_claim_precise_text_geometry() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("annotated.pdf");
        fixture(&path, false);
        let mut document = Document::load(&path).unwrap();
        let page = document.get_pages()[&1];
        let no_quads = document.add_object(dictionary! { "Type" => "Annot", "Subtype" => "Highlight",
            "Rect" => vec![50.into(), 500.into(), 150.into(), 520.into()], "Contents" => Object::string_literal("No quads") });
        let partial = document.add_object(dictionary! { "Type" => "Annot", "Subtype" => "Underline",
            "Rect" => vec![50.into(), 450.into(), 150.into(), 470.into()], "Contents" => Object::string_literal("Incomplete quads"),
            "QuadPoints" => vec![50.into(), 470.into(), 150.into(), 470.into(), 50.into(), 450.into(), 150.into(), 450.into(), 123.into()] });
        append_annotation(&mut document, page, no_quads).unwrap();
        append_annotation(&mut document, page, partial).unwrap();
        document.save(&path).unwrap();
        let marks =
            load_foreign_annotations_checked(&path, &FileRevision::read(&path).unwrap()).unwrap();
        for text in ["No quads", "Incomplete quads"] {
            let mark = marks.iter().find(|mark| mark.text == text).unwrap();
            assert!(!mark.precise_text_geometry);
            assert_eq!(mark.rects.len(), 1);
        }
    }
}
