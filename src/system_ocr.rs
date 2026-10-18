//! On-device OCR through the text recognizer that ships with the operating
//! system: `Windows.Media.Ocr` on Windows and Vision on macOS. Neither needs a
//! separate installation, an account, or a network connection. Other platforms
//! report the engine as unavailable so callers can fall back to Tesseract.

use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SystemOcrError {
    /// This platform has no built-in recognizer, or it cannot be created (for
    /// example, Windows has no OCR language installed).
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    Unavailable(String),
    /// The recognizer exists but could not read this image.
    Failed(String),
}

impl SystemOcrError {
    pub fn message(&self) -> &str {
        match self {
            SystemOcrError::Unavailable(message) | SystemOcrError::Failed(message) => message,
        }
    }
}

/// The user-facing name of the built-in recognizer on this platform.
pub fn engine_name() -> &'static str {
    platform::ENGINE_NAME
}

/// Confirms that the built-in recognizer can be created, and returns the
/// recognition language it will use.
pub fn availability() -> Result<String, SystemOcrError> {
    platform::availability()
}

/// Recognizes the text in one rendered page image (PNG).
pub fn recognize_image(image_path: &Path) -> Result<String, SystemOcrError> {
    platform::recognize_image(image_path)
}

/// One recognized line in image pixel coordinates, with the origin at the
/// top-left corner of the page image.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RecognizedLine {
    pub text: String,
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

impl RecognizedLine {
    fn height(&self) -> f64 {
        (self.bottom - self.top).max(0.0)
    }
}

/// Joins recognized lines into page text in the shape the rest of LawPDF
/// expects from OCR: one line per printed line, with a blank line between
/// paragraphs. Built-in recognizers report lines but not paragraphs, so a
/// paragraph break is inferred from vertical gaps, first-line indents, short
/// closing lines, size changes, and jumps back up the page (a new column).
pub(crate) fn assemble_page_text(lines: &[RecognizedLine]) -> String {
    let lines = merge_same_row_pieces(lines);
    let lines = lines.iter().collect::<Vec<_>>();
    if lines.is_empty() {
        return String::new();
    }

    let line_height = median(lines.iter().map(|line| line.height())).max(1.0);
    let right_margin = lines.iter().map(|line| line.right).fold(f64::MIN, f64::max);

    let mut output = String::new();
    let mut paragraph_left = lines[0].left;
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            let previous = lines[index - 1];
            let gap = line.top - previous.bottom;
            let moved_up = line.top + line_height * 0.5 < previous.top;
            let indented =
                line.left - previous.left > line_height && line.left - paragraph_left > line_height;
            let previous_closes = previous.right < right_margin - line_height * 3.0
                && ends_sentence(&previous.text)
                && line.left <= previous.left + line_height * 3.0;
            let size_changed = {
                let (small, large) = if line.height() < previous.height() {
                    (line.height(), previous.height())
                } else {
                    (previous.height(), line.height())
                };
                small > 0.0 && large / small > 1.3
            };
            let new_paragraph =
                gap > line_height * 0.6 || moved_up || indented || previous_closes || size_changed;
            if new_paragraph {
                output.push_str("\n\n");
                paragraph_left = line.left;
            } else {
                output.push('\n');
                paragraph_left = paragraph_left.min(line.left);
            }
        }
        output.push_str(line.text.trim());
    }
    output
}

/// Recognizers split one printed line at wide gaps, as in a running head
/// ("1949]   NOMINAL CONSIDERATION   217"). Consecutive pieces that share a
/// row are rejoined with a space so the line reads as printed.
fn merge_same_row_pieces(lines: &[RecognizedLine]) -> Vec<RecognizedLine> {
    let mut merged: Vec<RecognizedLine> = Vec::new();
    for line in lines.iter().filter(|line| !line.text.trim().is_empty()) {
        if let Some(previous) = merged.last_mut() {
            let overlap = previous.bottom.min(line.bottom) - previous.top.max(line.top);
            let shorter = previous.height().min(line.height());
            if shorter > 0.0
                && overlap > shorter * 0.5
                && line.left >= previous.right - shorter * 0.5
            {
                previous.text = format!("{} {}", previous.text.trim_end(), line.text.trim());
                previous.right = previous.right.max(line.right);
                previous.top = previous.top.min(line.top);
                previous.bottom = previous.bottom.max(line.bottom);
                continue;
            }
        }
        merged.push(RecognizedLine {
            text: line.text.trim().to_owned(),
            ..line.clone()
        });
    }
    merged
}

fn ends_sentence(text: &str) -> bool {
    text.trim_end()
        .trim_end_matches(['"', '\'', '\u{201d}', '\u{2019}', ')', ']'])
        .ends_with(['.', '?', '!', ':'])
}

fn median(values: impl Iterator<Item = f64>) -> f64 {
    let mut values = values.filter(|value| value.is_finite()).collect::<Vec<_>>();
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

#[cfg(target_os = "windows")]
mod platform {
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;

    use image::imageops::FilterType;
    use windows::Globalization::Language;
    use windows::Graphics::Imaging::{BitmapAlphaMode, BitmapPixelFormat, SoftwareBitmap};
    use windows::Media::Ocr::OcrEngine;
    use windows::Storage::Streams::DataWriter;
    use windows::Win32::System::Com::{
        COINIT_MULTITHREADED, CoIncrementMTAUsage, CoInitializeEx, CoUninitialize,
    };
    use windows::core::HSTRING;

    use super::{RecognizedLine, SystemOcrError, assemble_page_text};

    pub const ENGINE_NAME: &str = "Windows OCR";

    const NO_LANGUAGE: &str = "Windows OCR has no recognition language installed. \
        Open Settings > Time & language > Language & region, choose your language's \
        options, and add Optical character recognition. Then run OCR again.";

    pub fn availability() -> Result<String, SystemOcrError> {
        on_multithreaded_thread(|| {
            let engine = create_engine()?;
            Ok(engine
                .RecognizerLanguage()
                .and_then(|language| language.LanguageTag())
                .map(|tag| tag.to_string())
                .unwrap_or_else(|_| "unknown".to_owned()))
        })
    }

    pub fn recognize_image(image_path: &Path) -> Result<String, SystemOcrError> {
        let path = std::path::absolute(image_path).unwrap_or_else(|_| image_path.to_path_buf());
        on_multithreaded_thread(move || recognize(&path))
    }

    /// WinRT blocking waits must not run on a single-threaded apartment such
    /// as the UI thread, so each request runs on its own MTA thread.
    fn on_multithreaded_thread<T: Send + 'static>(
        work: impl FnOnce() -> Result<T, SystemOcrError> + Send + 'static,
    ) -> Result<T, SystemOcrError> {
        // windows-rs caches agile WinRT factories for the process lifetime.
        // If the last short-lived OCR thread tears down the MTA, those cached
        // factories can point to freed objects on the next page/request.
        // Retain one process-lifetime MTA reference to match their lifetime;
        // each worker still balances its own CoInitializeEx below.
        static MTA_COOKIE: OnceLock<Result<usize, SystemOcrError>> = OnceLock::new();
        std::thread::Builder::new()
            .name("lawpdf-windows-ocr".to_owned())
            .spawn(move || {
                MTA_COOKIE
                    .get_or_init(|| {
                        // SAFETY: Keep COM's MTA alive until process exit, matching
                        // the cached factories. Initialize on this worker so an
                        // uninitialized caller/UI thread keeps its apartment choice.
                        unsafe { CoIncrementMTAUsage() }
                            .map(|cookie| cookie.0 as usize)
                            .map_err(|error| {
                                SystemOcrError::Unavailable(format!(
                                    "Could not initialize Windows OCR: {error}"
                                ))
                            })
                    })
                    .as_ref()
                    .map_err(Clone::clone)?;
                let initialized = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
                let result = work();
                if initialized {
                    unsafe { CoUninitialize() };
                }
                result
            })
            .map_err(|error| {
                SystemOcrError::Failed(format!("Could not start Windows OCR: {error}"))
            })?
            .join()
            .map_err(|_| SystemOcrError::Failed("Windows OCR stopped unexpectedly.".to_owned()))?
    }

    fn create_engine() -> Result<OcrEngine, SystemOcrError> {
        let available = OcrEngine::AvailableRecognizerLanguages().map_err(|error| {
            SystemOcrError::Unavailable(format!("Windows OCR is not available: {error}"))
        })?;
        if available.Size().unwrap_or(0) == 0 {
            return Err(SystemOcrError::Unavailable(NO_LANGUAGE.to_owned()));
        }
        if let Ok(engine) = OcrEngine::TryCreateFromUserProfileLanguages() {
            return Ok(engine);
        }
        for tag in ["en-US", "en-GB", "en"] {
            let Ok(language) = Language::CreateLanguage(&HSTRING::from(tag)) else {
                continue;
            };
            if OcrEngine::IsLanguageSupported(&language).unwrap_or(false) {
                if let Ok(engine) = OcrEngine::TryCreateFromLanguage(&language) {
                    return Ok(engine);
                }
            }
        }
        for index in 0..available.Size().unwrap_or(0) {
            if let Ok(language) = available.GetAt(index) {
                if let Ok(engine) = OcrEngine::TryCreateFromLanguage(&language) {
                    return Ok(engine);
                }
            }
        }
        Err(SystemOcrError::Unavailable(NO_LANGUAGE.to_owned()))
    }

    fn recognize(path: &PathBuf) -> Result<String, SystemOcrError> {
        let failed = |stage: &str, error: windows::core::Error| {
            SystemOcrError::Failed(format!("Windows OCR could not {stage}: {error}"))
        };
        let engine = create_engine()?;

        // Decode in memory rather than handing Windows a file path, so the
        // packaged (Store) app never depends on file-broker access to a temp file.
        let image = image::open(path).map_err(|error| {
            SystemOcrError::Failed(format!("Could not read the OCR page image: {error}"))
        })?;
        let max_dimension = OcrEngine::MaxImageDimension().unwrap_or(10_000).max(1);
        let image = if image.width() > max_dimension || image.height() > max_dimension {
            let scale = f64::from(max_dimension) / f64::from(image.width().max(image.height()));
            image.resize(
                ((f64::from(image.width()) * scale) as u32).max(1),
                ((f64::from(image.height()) * scale) as u32).max(1),
                FilterType::Triangle,
            )
        } else {
            image
        };
        let rgba = image.to_rgba8();
        let (width, height) = rgba.dimensions();
        let mut bgra = rgba.into_raw();
        for pixel in bgra.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        let writer = DataWriter::new().map_err(|error| failed("prepare the page image", error))?;
        writer
            .WriteBytes(&bgra)
            .map_err(|error| failed("prepare the page image", error))?;
        let buffer = writer
            .DetachBuffer()
            .map_err(|error| failed("prepare the page image", error))?;
        let bitmap = SoftwareBitmap::CreateCopyWithAlphaFromBuffer(
            &buffer,
            BitmapPixelFormat::Bgra8,
            width as i32,
            height as i32,
            BitmapAlphaMode::Premultiplied,
        )
        .map_err(|error| failed("prepare the page image", error))?;

        let result = engine
            .RecognizeAsync(&bitmap)
            .and_then(|operation| operation.get())
            .map_err(|error| failed("recognize the page", error))?;
        let ocr_lines = result
            .Lines()
            .map_err(|error| failed("read the recognized lines", error))?;

        let mut lines = Vec::new();
        for index in 0..ocr_lines.Size().unwrap_or(0) {
            let Ok(line) = ocr_lines.GetAt(index) else {
                continue;
            };
            let text = line.Text().map(|text| text.to_string()).unwrap_or_default();
            if text.trim().is_empty() {
                continue;
            }
            let mut bounds: Option<(f64, f64, f64, f64)> = None;
            if let Ok(words) = line.Words() {
                for word_index in 0..words.Size().unwrap_or(0) {
                    let Ok(rect) = words.GetAt(word_index).and_then(|word| word.BoundingRect())
                    else {
                        continue;
                    };
                    let (left, top) = (f64::from(rect.X), f64::from(rect.Y));
                    let (right, bottom) =
                        (left + f64::from(rect.Width), top + f64::from(rect.Height));
                    bounds = Some(match bounds {
                        Some((l, t, r, b)) => {
                            (l.min(left), t.min(top), r.max(right), b.max(bottom))
                        }
                        None => (left, top, right, bottom),
                    });
                }
            }
            let (left, top, right, bottom) = bounds.unwrap_or_else(|| {
                // Without word boxes, keep the reported order and spacing neutral.
                let y = index as f64 * 20.0;
                (0.0, y, 0.0, y + 16.0)
            });
            lines.push(RecognizedLine {
                text,
                left,
                top,
                right,
                bottom,
            });
        }
        Ok(assemble_page_text(&lines))
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use std::path::Path;

    use objc2::AllocAnyThread;
    use objc2::rc::{Retained, autoreleasepool};
    use objc2::runtime::AnyObject;
    use objc2_foundation::{NSArray, NSData, NSDictionary};
    use objc2_vision::{
        VNImageOption, VNImageRequestHandler, VNRecognizeTextRequest, VNRequest,
        VNRequestTextRecognitionLevel,
    };

    use super::{RecognizedLine, SystemOcrError, assemble_page_text};

    pub const ENGINE_NAME: &str = "macOS Vision";

    pub fn availability() -> Result<String, SystemOcrError> {
        // Vision ships with every supported macOS release (13 and later).
        Ok("en-US".to_owned())
    }

    pub fn recognize_image(image_path: &Path) -> Result<String, SystemOcrError> {
        let bytes = std::fs::read(image_path).map_err(|error| {
            SystemOcrError::Failed(format!("Could not read the OCR page image: {error}"))
        })?;
        let (width, height) = image::image_dimensions(image_path).map_err(|error| {
            SystemOcrError::Failed(format!("Could not measure the OCR page image: {error}"))
        })?;
        let (width, height) = (f64::from(width), f64::from(height));

        autoreleasepool(|_| {
            let data = NSData::with_bytes(&bytes);
            let options: Retained<NSDictionary<VNImageOption, AnyObject>> = NSDictionary::new();
            let handler = VNImageRequestHandler::initWithData_options(
                VNImageRequestHandler::alloc(),
                &data,
                &options,
            );
            let request = VNRecognizeTextRequest::new();
            request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
            request.setUsesLanguageCorrection(true);
            let base: &VNRequest = &request;
            let requests = NSArray::from_slice(&[base]);
            handler.performRequests_error(&requests).map_err(|error| {
                SystemOcrError::Failed(format!(
                    "macOS Vision could not recognize the page: {}",
                    error.localizedDescription()
                ))
            })?;

            let Some(results) = request.results() else {
                return Ok(String::new());
            };
            let mut lines = Vec::new();
            for index in 0..results.count() {
                let observation = results.objectAtIndex(index);
                let candidates = observation.topCandidates(1);
                let Some(best) = candidates.firstObject() else {
                    continue;
                };
                let text = best.string().to_string();
                if text.trim().is_empty() {
                    continue;
                }
                // Vision boxes are normalized with the origin at the lower left.
                let rect = unsafe { observation.boundingBox() };
                let left = rect.origin.x * width;
                let right = (rect.origin.x + rect.size.width) * width;
                let top = (1.0 - (rect.origin.y + rect.size.height)) * height;
                let bottom = (1.0 - rect.origin.y) * height;
                lines.push(RecognizedLine {
                    text,
                    left,
                    top,
                    right,
                    bottom,
                });
            }
            Ok(assemble_page_text(&lines))
        })
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
mod platform {
    use std::path::Path;

    use super::SystemOcrError;

    pub const ENGINE_NAME: &str = "none";

    pub fn availability() -> Result<String, SystemOcrError> {
        Err(SystemOcrError::Unavailable(
            "This platform has no built-in OCR engine.".to_owned(),
        ))
    }

    pub fn recognize_image(_image_path: &Path) -> Result<String, SystemOcrError> {
        Err(SystemOcrError::Unavailable(
            "This platform has no built-in OCR engine.".to_owned(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str, left: f64, top: f64, right: f64) -> RecognizedLine {
        RecognizedLine {
            text: text.to_owned(),
            left,
            top,
            right,
            bottom: top + 20.0,
        }
    }

    #[test]
    fn continuous_lines_stay_in_one_paragraph() {
        let text = assemble_page_text(&[
            line(
                "The doctrine of consideration asks whether",
                100.0,
                100.0,
                900.0,
            ),
            line(
                "each side bargained for the other's promise",
                100.0,
                124.0,
                902.0,
            ),
            line("or performance.", 100.0, 148.0, 300.0),
        ]);
        assert_eq!(
            text,
            "The doctrine of consideration asks whether\neach side bargained for the other's promise\nor performance."
        );
    }

    #[test]
    fn indents_gaps_and_closing_lines_start_paragraphs() {
        let text = assemble_page_text(&[
            line("INTRODUCTION", 400.0, 60.0, 600.0),
            line(
                "Courts have long struggled with the question",
                100.0,
                120.0,
                900.0,
            ),
            line("of nominal consideration.", 100.0, 144.0, 420.0),
            line(
                "Consider Batsakis v. Demotsis, decided in",
                140.0,
                168.0,
                900.0,
            ),
            line(
                "1949 by the Texas Court of Civil Appeals.",
                100.0,
                192.0,
                899.0,
            ),
        ]);
        assert_eq!(
            text,
            "INTRODUCTION\n\nCourts have long struggled with the question\nof nominal consideration.\n\nConsider Batsakis v. Demotsis, decided in\n1949 by the Texas Court of Civil Appeals."
        );
    }

    #[test]
    fn smaller_footnote_text_and_new_columns_start_paragraphs() {
        let mut footnote = line(
            "1 See Restatement (Second) of Contracts § 71.",
            100.0,
            900.0,
            700.0,
        );
        footnote.bottom = footnote.top + 12.0;
        let text = assemble_page_text(&[
            line(
                "Body text that runs to the margin and keeps",
                100.0,
                860.0,
                900.0,
            ),
            footnote,
            line("Second column text", 1000.0, 100.0, 1800.0),
        ]);
        assert_eq!(
            text,
            "Body text that runs to the margin and keeps\n\n1 See Restatement (Second) of Contracts § 71.\n\nSecond column text"
        );
    }

    #[test]
    fn running_head_pieces_on_one_row_are_rejoined() {
        let text = assemble_page_text(&[
            line("1949]", 100.0, 60.0, 160.0),
            line("NOMINAL CONSIDERATION AND THE BARGAIN", 380.0, 61.0, 1000.0),
            line("217", 1080.0, 60.0, 1120.0),
            line("I. THE PUZZLE OF THE PEPPERCORN", 400.0, 140.0, 980.0),
        ]);
        assert_eq!(
            text,
            "1949] NOMINAL CONSIDERATION AND THE BARGAIN 217\n\nI. THE PUZZLE OF THE PEPPERCORN"
        );
    }

    #[test]
    fn blank_lines_are_ignored() {
        assert_eq!(assemble_page_text(&[]), "");
        assert_eq!(
            assemble_page_text(&[line("   ", 0.0, 0.0, 10.0), line("Only", 0.0, 30.0, 10.0)]),
            "Only"
        );
    }
}
