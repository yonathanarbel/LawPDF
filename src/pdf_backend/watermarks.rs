//! Conservative, read-only exclusion of recurring diagonal text watermarks.
//!
//! Work in PDFium character indices, before line splitting. Never remove text
//! by its wording or by bounding-box overlap: normal text underneath a stamp
//! must survive. Rendering and the source PDF are not modified.

use super::*;
use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct Watermarks {
    // Only small candidate summaries are cached, not page handles or glyphs.
    pages: RefCell<BTreeMap<usize, Vec<Candidate>>>,
}

#[derive(Clone)]
struct Candidate {
    text: String,
    indices: Vec<usize>,
    bounds: [f32; 4],
    angle: f32,
    size: f32,
    overlays_body: bool,
}

impl Candidate {
    fn matches(&self, other: &Self) -> bool {
        self.text == other.text
            && (self.angle - other.angle).abs() <= 3.0
            && (self.size / other.size - 1.0).abs() <= 0.12
            && self
                .bounds
                .iter()
                .zip(other.bounds)
                .all(|(a, b)| (a - b).abs() <= 0.025)
    }
}

impl Watermarks {
    pub(super) fn excluded(
        &self,
        document: &PdfDocument<'_>,
        page_index: usize,
        page: &PdfPage<'_>,
        text: &PdfPageText<'_>,
    ) -> Vec<bool> {
        let count = text.chars().len() as usize;
        let mut excluded = vec![false; count];
        if !self.pages.borrow().contains_key(&page_index) {
            self.pages
                .borrow_mut()
                .insert(page_index, candidates(page, text));
        }
        let local = self
            .pages
            .borrow()
            .get(&page_index)
            .cloned()
            .unwrap_or_default();
        let page_count = document.pages().len() as usize;
        if local.is_empty() || page_count < 2 {
            return excluded;
        }

        // Bounded sampling keeps this out of the all-pages opening critical path.
        // Include nearby pages so a stamped chapter need not cover the whole PDF.
        let mut sample = vec![
            page_index,
            page_index.saturating_sub(1),
            (page_index + 1).min(page_count - 1),
            0,
            page_count / 2,
            page_count - 1,
        ];
        sample.sort_unstable();
        sample.dedup();
        for &index in &sample {
            if self.pages.borrow().contains_key(&index) {
                continue;
            }
            let found = document
                .pages()
                .get(index as u16)
                .ok()
                .and_then(|page| page.text().ok().map(|text| candidates(&page, &text)))
                .unwrap_or_default();
            self.pages.borrow_mut().insert(index, found);
        }
        let required = page_count.min(3);
        let pages = self.pages.borrow();
        for candidate in local {
            let repeats = sample
                .iter()
                .filter(|index| {
                    pages
                        .get(index)
                        .is_some_and(|other| other.iter().any(|c| candidate.matches(c)))
                })
                .count();
            if repeats < required {
                continue;
            }
            // Sparse/blank pages can carry the same stamp. Require the overlay
            // evidence on other pages rather than demanding body text locally.
            let over_body = sample
                .iter()
                .filter(|index| {
                    pages.get(index).is_some_and(|other| {
                        other
                            .iter()
                            .any(|c| c.overlays_body && candidate.matches(c))
                    })
                })
                .count();
            if over_body < (required - 1) {
                continue;
            }
            // Include generated whitespace *between* matched glyphs, never the
            // separator after the watermark or any intersecting body characters.
            if let (Some(&start), Some(&end)) =
                (candidate.indices.first(), candidate.indices.last())
            {
                for index in start..=end {
                    excluded[index] = true;
                }
            }
        }
        excluded
    }
}

fn candidates(page: &PdfPage<'_>, text: &PdfPageText<'_>) -> Vec<Candidate> {
    let width = page.width().value.max(1.0);
    let height = page.height().value.max(1.0);
    let mut runs = Vec::<Candidate>::new();
    let mut current: Option<Candidate> = None;
    let mut ordinary_sizes = Vec::new();
    let mut ordinary_boxes = Vec::new();
    for character in text.chars().iter() {
        let Some(ch) = character.unicode_char() else {
            if let Some(run) = current.take() {
                runs.push(run);
            }
            continue;
        };
        if ch.is_whitespace() || ch == '\0' {
            continue;
        }
        let size = character.scaled_font_size().value;
        let bounds = character
            .loose_bounds()
            .or_else(|_| character.tight_bounds())
            .ok();
        // Most text never needs the extra native angle/color calls.
        let style = (size.is_finite() && size >= 24.0)
            .then(|| {
                let angle = character.angle_degrees().ok()?.rem_euclid(180.0);
                // Horizontal/vertical captions, running heads and rotated body text
                // are deliberately outside this conservative detector.
                let diagonal = angle.min(180.0 - angle);
                if !(15.0..=75.0).contains(&diagonal) {
                    return None;
                }
                let color = character.fill_color().ok()?;
                let alpha = color.alpha() as f32 / 255.0;
                let luminance = (0.2126 * color.red() as f32
                    + 0.7152 * color.green() as f32
                    + 0.0722 * color.blue() as f32)
                    / 255.0;
                (alpha > 0.0 && 1.0 - alpha * (1.0 - luminance) >= 0.75).then_some(angle)
            })
            .flatten();
        if let (Some(angle), Some(rect)) = (style, bounds.as_ref()) {
            let b = [
                rect.left().value / width,
                rect.bottom().value / height,
                rect.right().value / width,
                rect.top().value / height,
            ];
            let compatible = current.as_ref().is_some_and(|run| {
                (run.angle - angle).abs() < 3.0
                    && (run.size / size - 1.0).abs() < 0.12
                    && b[0] <= run.bounds[2] + 0.05
                    && b[2] >= run.bounds[0] - 0.05
                    && b[1] <= run.bounds[3] + 0.05
                    && b[3] >= run.bounds[1] - 0.05
            });
            if !compatible {
                if let Some(run) = current.take() {
                    runs.push(run);
                }
                current = Some(Candidate {
                    text: String::new(),
                    indices: Vec::new(),
                    bounds: b,
                    angle,
                    size,
                    overlays_body: false,
                });
            }
            let run = current.as_mut().expect("candidate initialized");
            run.text.push(ch);
            run.indices.push(character.index() as usize);
            run.bounds = [
                run.bounds[0].min(b[0]),
                run.bounds[1].min(b[1]),
                run.bounds[2].max(b[2]),
                run.bounds[3].max(b[3]),
            ];
        } else {
            if let Some(run) = current.take() {
                runs.push(run);
            }
            if ch.is_alphabetic() && size.is_finite() && size > 0.0 {
                ordinary_sizes.push(size);
                if let Some(rect) = bounds {
                    ordinary_boxes.push([
                        rect.left().value / width,
                        rect.bottom().value / height,
                        rect.right().value / width,
                        rect.top().value / height,
                    ]);
                }
            }
        }
    }
    if let Some(run) = current {
        runs.push(run);
    }
    ordinary_sizes.sort_by(f32::total_cmp);
    let body_size = ordinary_sizes
        .get(ordinary_sizes.len() / 2)
        .copied()
        .unwrap_or(12.0);
    runs.retain_mut(|run| {
        let [left, bottom, right, top] = run.bounds;
        let letters = run.text.chars().filter(|ch| ch.is_alphabetic()).count();
        run.overlays_body = ordinary_sizes.len() >= 40
            && run.size >= body_size * 1.8
            && ordinary_boxes
                .iter()
                .filter(|b| b[0] < right && b[2] > left && b[1] < top && b[3] > bottom)
                .take(20)
                .count()
                >= 20;
        (5..=160).contains(&letters)
            && right - left >= 0.20
            && top - bottom >= 0.12
            && (0.25..=0.75).contains(&((left + right) / 2.0))
            && (0.25..=0.75).contains(&((bottom + top) / 2.0))
    });
    runs.truncate(32);
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    // Synthetic end-user QA fixture: no private manuscript or training material.
    fn fixture(path: &Path, stamps: &[(&str, f32, f32)]) {
        fixture_with_blank(path, stamps, None);
    }

    fn fixture_with_blank(path: &Path, stamps: &[(&str, f32, f32)], blank: Option<usize>) {
        let mut doc = Document::with_version("1.7");
        let pages = doc.new_object_id();
        let font = doc.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1",
        "BaseFont" => "Helvetica" });
        let resources = doc.add_object(dictionary! { "Font" => dictionary! { "F1" => font } });
        let mut kids = Vec::new();
        for (page_index, &(stamp, angle, gray)) in stamps.iter().enumerate() {
            let angle = angle.to_radians();
            let (sin, cos) = angle.sin_cos();
            let mut content = format!(
                "q BT /F1 40 Tf {gray} g {cos} {sin} {} {cos} 165 290 Tm ({stamp}) Tj ET Q\n",
                -sin
            );
            for row in 0..if blank == Some(page_index) { 0 } else { 12 } {
                content.push_str(&format!(
                    "BT /F1 12 Tf 0 g 1 0 0 1 72 {} Tm (Ordinary text underneath a stamp must remain selectable.) Tj ET\n",
                    550 - row * 22));
            }
            if blank != Some(page_index) {
                content.push_str("BT /F1 12 Tf 0 g 1 0 0 1 72 230 Tm (The phrase FOR INTERNAL REVIEW is also ordinary prose.) Tj ET\n");
                content.push_str("BT /F1 8 Tf 0 g 1 0 0 1 395 234 Tm (6) Tj ET\n");
            }
            content.push_str(&format!(
                "BT /F1 12 Tf 0 g 1 0 0 1 300 50 Tm ({}) Tj ET\n",
                page_index + 1
            ));
            let stream = doc.add_object(Stream::new(dictionary! {}, content.into_bytes()));
            let page = doc.add_object(dictionary! { "Type" => "Page", "Parent" => pages,
                "Resources" => resources, "Contents" => stream, "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()] });
            kids.push(Object::Reference(page));
        }
        doc.objects.insert(
            pages,
            dictionary! { "Type" => "Pages", "Count" => kids.len() as i64,
            "Kids" => kids }
            .into(),
        );
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages });
        doc.trailer.set("Root", catalog);
        doc.save(path).unwrap();
    }

    #[test]
    fn recurring_watermark_is_excluded_from_both_text_paths_without_changing_pdf() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("recurring.pdf");
        fixture(&path, &[("FOR INTERNAL REVIEW", 45.0, 0.85); 3]);
        let original = fs::read(&path).unwrap();
        let engine = NativePdfEngine::new().unwrap();
        // Query out of order: detection must not depend on previously visited pages.
        for page in [2, 0, 1] {
            let text = engine.load_page_text(&path, page).unwrap();
            let chars = engine.load_page_text_chars(&path, page).unwrap();
            let selected: String = chars.iter().map(|c| c.ch).collect();
            for text in [&text, &selected] {
                assert_eq!(text.matches("FOR INTERNAL REVIEW").count(), 1, "{text}");
                assert_eq!(text.matches("Ordinary text underneath").count(), 12);
                assert!(text.contains("6"));
            }
            // Review receives these exact filtered glyphs before its line classifier.
            let loaded = engine.load_document_optimized(&path).unwrap();
            let lines = crate::layout_roles::deep_source_lines_for_pages(
                &loaded.pages[page..=page],
                &[Some(chars)],
            );
            assert!(
                !lines
                    .iter()
                    .any(|line| line.text.trim() == "FOR INTERNAL REVIEW")
            );
        }
        engine.close_document(&path);
        // Disk caches must also contain the filtered text when the document reopens.
        assert_eq!(
            engine
                .load_page_text(&path, 0)
                .unwrap()
                .matches("FOR INTERNAL REVIEW")
                .count(),
            1
        );
        assert_eq!(fs::read(&path).unwrap(), original);
        // Original rendering still has the watermark: the PDF stream was not edited.
        engine
            .with_open_document(&path, |doc| {
                let page = doc.pages().get(0)?;
                assert_eq!(page.text()?.all().matches("FOR INTERNAL REVIEW").count(), 2);
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn blank_page_uses_overlay_evidence_from_neighboring_pages() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("blank.pdf");
        fixture_with_blank(&path, &[("FOR INTERNAL REVIEW", 45.0, 0.85); 3], Some(1));
        let engine = NativePdfEngine::new().unwrap();
        // Start on the otherwise blank page, before visiting either body page.
        assert_eq!(engine.load_page_text(&path, 1).unwrap().trim(), "2");
        assert_eq!(
            engine
                .load_page_text_chars(&path, 1)
                .unwrap()
                .iter()
                .map(|ch| ch.ch)
                .collect::<String>()
                .trim(),
            "2"
        );
    }

    #[test]
    fn ambiguous_or_non_watermark_text_is_preserved() {
        let cases = [
            vec![
                ("UNIQUE DIAGONAL LABEL", 45.0, 0.85),
                ("OTHER DIAGONAL LABEL", 45.0, 0.85),
                ("THIRD DIAGONAL LABEL", 45.0, 0.85),
            ],
            vec![("DARK DIAGONAL LABEL", 45.0, 0.0); 3],
            vec![("HORIZONTAL HEADING", 0.0, 0.85); 3],
            vec![("SINGLE PAGE NOTICE", 45.0, 0.85)],
        ];
        let dir = tempfile::tempdir().unwrap();
        let engine = NativePdfEngine::new().unwrap();
        for (index, stamps) in cases.iter().enumerate() {
            let path = dir.path().join(format!("preserve-{index}.pdf"));
            fixture(&path, stamps);
            let text = engine.load_page_text(&path, 0).unwrap();
            let chars = engine.load_page_text_chars(&path, 0).unwrap();
            assert!(text.contains(stamps[0].0), "{text}");
            assert!(
                chars
                    .iter()
                    .map(|c| c.ch)
                    .collect::<String>()
                    .contains(stamps[0].0)
            );
        }
    }
}
