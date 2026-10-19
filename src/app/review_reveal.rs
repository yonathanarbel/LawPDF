//! The reveal that plays the first time a document opens in Review Mode.
//!
//! The words of the page the reader was on lift off the printed page and
//! settle where they now live: body text into the reading column, footnote
//! text into the margin cards beside the lines that cite them. It is a
//! one-time explanation of what Review Mode did to the page, so it is kept
//! short, it plays once per document, it never runs when the reader prefers
//! reduced motion, and any click, scroll or key ends it at once.
//!
//! Sources and destinations are measured, never guessed: the printed
//! positions come from the page's text layer, the reflowed positions from the
//! galleys Review Mode actually drew this frame. Words that have no measured
//! destination (page furniture, text that wrapped onto the next page) fade
//! where they are.

use std::collections::HashMap;

use std::sync::Arc;

use egui::{Color32, Galley, Pos2, Rect, TextureId, Vec2};

use super::PagePlacement;
use crate::liquid::{LiquidBlockRole, LiquidDocument};
use crate::model::{PageTextChar, PdfRect};
use crate::review_reading::{is_review_note_display_block, split_fused_review_notes};

/// Flight time of the reveal. Short enough to read as a settle, long enough
/// to follow a word with the eye.
pub(super) const REVEAL_SECONDS: f64 = 1.05;
/// Source lines of each Review block on the page: (page index, line text).
pub(super) type BlockLines = HashMap<usize, Vec<(usize, String)>>;
/// Source lines of each footnote shown as a margin note, by note number.
pub(super) type NoteLines = HashMap<u16, Vec<String>>;

/// Below this many matched words the effect reads as noise; skip it.
const MIN_MATCHED_WORDS: usize = 16;
/// A page with more words than this is a table or a dense index; skip it.
const MAX_PAGE_WORDS: usize = 1400;
/// Frames to wait for Review Mode to scroll to the page before giving up.
const MAX_SETTLE_FRAMES: u8 = 8;
/// Reading-order stagger: the last word leaves this far into the flight.
const STAGGER: f32 = 0.32;
/// Alignment work is bounded so a pathological page cannot stall a frame.
const MAX_ALIGN_TOKENS: usize = 700;

#[derive(Debug, Clone)]
pub(super) struct RevealWord {
    pub text: String,
    pub key: String,
    pub from: Rect,
    pub to: Option<Landing>,
    /// 0..1 position in reading order; drives the stagger.
    pub order: f32,
}

/// Where a word sits in Review Mode: the row box it occupies, its baseline,
/// and the text format it was drawn with, so the same galley lands on it
/// pixel for pixel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Landing {
    pub rect: Rect,
    pub baseline: f32,
    pub style: LandingStyle,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct LandingStyle {
    pub size: f32,
    pub line_height: Option<f32>,
}

/// A word as a galley drew it this frame.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct DrawnWord {
    pub text: String,
    pub rect: Rect,
    pub baseline: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum RevealPhase {
    /// Review Mode is scrolling to the page's first block. The printed page
    /// stays on screen so the switch does not flash.
    Settling {
        frames: u8,
    },
    /// Destinations are measured and the words are in flight.
    Playing {
        started: f64,
    },
    Done,
}

/// The words of a paragraph label as Review Mode drew them this frame.
#[derive(Debug, Clone)]
pub(super) struct CapturedLabel {
    pub block_index: usize,
    pub rect: Rect,
    pub style: LandingStyle,
    pub words: Vec<DrawnWord>,
}

/// Words of a margin note card as drawn this frame, keyed by note number.
#[derive(Debug, Clone)]
pub(super) struct CapturedNote {
    pub number: u16,
    pub style: LandingStyle,
    pub words: Vec<DrawnWord>,
}

#[derive(Debug)]
pub(super) struct ReviewReveal {
    pub page_index: usize,
    /// The page as the original view last showed it, in screen coordinates.
    pub page_rect: Rect,
    pub texture: Option<TextureId>,
    pub words: Vec<RevealWord>,
    pub phase: RevealPhase,
    pub labels: Vec<CapturedLabel>,
    pub notes: Vec<CapturedNote>,
    /// Where last frame's labels and notes sat. The flight starts only once
    /// two frames agree: the first frame after a jump can still be settling
    /// (a scrollbar appearing, a margin finding its width), and words
    /// measured then would land a word or a line off.
    pub last_layout: Vec<(usize, Rect, usize)>,
    /// One galley per matched word in its landing format, laid out once
    /// when the flight starts. Frames scale these meshes instead of
    /// rasterizing every word at a new size.
    pub galleys: Vec<Option<Arc<Galley>>>,
}

impl ReviewReveal {
    /// Measure the printed page. `None` when the page has no usable text
    /// layer, too few words, or far too many.
    pub(super) fn plan(
        page_index: usize,
        placement: &PagePlacement,
        texture: Option<TextureId>,
        chars: &[PageTextChar],
        viewport: Rect,
    ) -> Option<Self> {
        let page_words = page_words(chars);
        if page_words.len() < MIN_MATCHED_WORDS || page_words.len() > MAX_PAGE_WORDS {
            return None;
        }
        let total = page_words.len().max(1) as f32;
        let words = page_words
            .into_iter()
            .enumerate()
            .map(|(index, (text, rect))| RevealWord {
                key: key(&text),
                text,
                from: placement.pdf_rect_to_screen(rect),
                to: None,
                order: index as f32 / total,
            })
            .filter(|word| word.from.intersects(viewport.expand(40.0)))
            .collect::<Vec<_>>();
        if words.len() < MIN_MATCHED_WORDS {
            return None;
        }
        Some(Self {
            page_index,
            page_rect: placement.rect,
            texture,
            words,
            phase: RevealPhase::Settling { frames: 0 },
            labels: Vec::new(),
            notes: Vec::new(),
            last_layout: Vec::new(),
            galleys: Vec::new(),
        })
    }

    pub(super) fn is_capturing(&self) -> bool {
        matches!(self.phase, RevealPhase::Settling { .. })
    }

    pub(super) fn is_active(&self) -> bool {
        !matches!(self.phase, RevealPhase::Done)
    }

    /// Called once per frame after Review Mode has drawn. Starts the flight
    /// when the page's blocks are on screen, or gives up after a few frames.
    pub(super) fn settle(
        &mut self,
        now: f64,
        viewport: Rect,
        source_lines: &BlockLines,
        note_lines: &NoteLines,
    ) {
        let RevealPhase::Settling { frames } = self.phase else {
            return;
        };
        let on_screen = self.labels.iter().any(|label| {
            label.rect.intersects(viewport)
                && source_lines
                    .get(&label.block_index)
                    .is_some_and(|lines| lines.iter().any(|(page, _)| *page == self.page_index))
        });
        let layout = self.layout_fingerprint();
        let stable = on_screen && !layout.is_empty() && layout == self.last_layout;
        self.last_layout = layout;
        if !stable {
            if frames + 1 >= MAX_SETTLE_FRAMES {
                self.phase = RevealPhase::Done;
            } else {
                self.phase = RevealPhase::Settling { frames: frames + 1 };
            }
            self.labels.clear();
            self.notes.clear();
            return;
        }
        let matched = self.resolve(source_lines, note_lines);
        self.labels.clear();
        self.notes.clear();
        self.phase = if matched >= MIN_MATCHED_WORDS {
            RevealPhase::Playing { started: now }
        } else {
            RevealPhase::Done
        };
    }

    /// Enough of this frame's layout to tell whether it moved since the
    /// last one. Rounded, since the same layout can differ by float noise.
    fn layout_fingerprint(&self) -> Vec<(usize, Rect, usize)> {
        fn rounded(rect: Rect) -> Rect {
            Rect::from_min_max(
                Pos2::new(rect.left().round(), rect.top().round()),
                Pos2::new(rect.right().round(), rect.bottom().round()),
            )
        }
        let labels = self
            .labels
            .iter()
            .map(|label| (label.block_index, rounded(label.rect), label.words.len()));
        let notes = self.notes.iter().map(|note| {
            let first = note
                .words
                .first()
                .map(|word| word.rect)
                .unwrap_or(Rect::NOTHING);
            (
                usize::MAX - usize::from(note.number),
                rounded(first),
                note.words.len(),
            )
        });
        labels.chain(notes).collect()
    }

    /// Pair printed words with their Review Mode positions. Returns how many
    /// found a destination.
    fn resolve(&mut self, source_lines: &BlockLines, note_lines: &NoteLines) -> usize {
        let page_keys = self.words.iter().map(|w| w.key.clone()).collect::<Vec<_>>();
        let mut matched = 0usize;
        let labels = std::mem::take(&mut self.labels);
        let notes = std::mem::take(&mut self.notes);

        // Body: every label of a block, matched through the block's source
        // lines on this page.
        let mut block_words: HashMap<usize, Vec<(DrawnWord, LandingStyle)>> = HashMap::new();
        for label in labels {
            let style = label.style;
            block_words
                .entry(label.block_index)
                .or_default()
                .extend(label.words.into_iter().map(|word| (word, style)));
        }
        for (block_index, words) in block_words {
            let Some(lines) = source_lines.get(&block_index) else {
                continue;
            };
            let lines = lines
                .iter()
                .filter(|(page, _)| *page == self.page_index)
                .map(|(_, text)| text.as_str())
                .collect::<Vec<_>>();
            if lines.is_empty() {
                continue;
            }
            matched += self.assign(&page_keys, &lines, &words);
        }
        // Notes: the card's words, matched through the note's source lines.
        for note in notes {
            let Some(lines) = note_lines.get(&note.number) else {
                continue;
            };
            let lines = lines.iter().map(String::as_str).collect::<Vec<_>>();
            let words = note
                .words
                .into_iter()
                .map(|word| (word, note.style))
                .collect::<Vec<_>>();
            matched += self.assign(&page_keys, &lines, &words);
        }
        matched
    }

    /// Place `lines` (source text on this page) inside the page's words, then
    /// align the drawn `words` to those lines and copy their rects over.
    fn assign(
        &mut self,
        page_keys: &[String],
        lines: &[&str],
        words: &[(DrawnWord, LandingStyle)],
    ) -> usize {
        let page_slots = locate_lines(page_keys, lines);
        if page_slots.is_empty() {
            return 0;
        }
        let drawn_keys = words
            .iter()
            .map(|(word, _)| key(&word.text))
            .collect::<Vec<_>>();
        let source_keys = page_slots
            .iter()
            .map(|slot| page_keys[*slot].clone())
            .collect::<Vec<_>>();
        let pairs = align(&drawn_keys, &source_keys);
        let mut matched = 0usize;
        for (drawn, source) in pairs {
            let page_index = page_slots[source];
            let word = &mut self.words[page_index];
            if word.to.is_none() {
                let (drawn_word, style) = &words[drawn];
                word.to = Some(Landing {
                    rect: drawn_word.rect,
                    baseline: drawn_word.baseline,
                    style: *style,
                });
                matched += 1;
            }
        }
        matched
    }

    /// Eased 0..1 progress of the flight, or `None` when not playing.
    pub(super) fn progress(&self, now: f64) -> Option<f32> {
        match self.phase {
            RevealPhase::Playing { started } => {
                let linear = ((now - started) / REVEAL_SECONDS).clamp(0.0, 1.0) as f32;
                Some(ease_in_out(linear))
            }
            RevealPhase::Settling { .. } => Some(0.0),
            RevealPhase::Done => None,
        }
    }

    pub(super) fn finished(&self, now: f64) -> bool {
        match self.phase {
            RevealPhase::Playing { started } => now - started >= REVEAL_SECONDS,
            RevealPhase::Done => true,
            RevealPhase::Settling { .. } => false,
        }
    }
}

/// The first block Review Mode draws from `page_index`: where the reflow
/// opens when the reader arrives from that page.
pub(super) fn first_block_on_page(document: &LiquidDocument, page_index: usize) -> Option<usize> {
    let mut candidates = document
        .block_source_lines
        .iter()
        .filter(|source| {
            source
                .lines
                .iter()
                .any(|line| line.page_index == page_index)
        })
        .map(|source| source.block_index)
        .filter(|&index| {
            document.blocks.get(index).is_some_and(|block| {
                matches!(
                    block.role,
                    LiquidBlockRole::Title
                        | LiquidBlockRole::Heading
                        | LiquidBlockRole::Subheading
                        | LiquidBlockRole::Abstract
                        | LiquidBlockRole::Lead
                        | LiquidBlockRole::Paragraph
                        | LiquidBlockRole::Quote
                        | LiquidBlockRole::ListItem
                        | LiquidBlockRole::Clause
                )
            })
        })
        .collect::<Vec<_>>();
    candidates.sort_unstable();
    candidates.first().copied()
}

/// Source text on `page_index`, by block, and by footnote number for the
/// notes that live there.
pub(super) fn source_maps(document: &LiquidDocument, page_index: usize) -> (BlockLines, NoteLines) {
    let mut by_block: BlockLines = HashMap::new();
    for source in &document.block_source_lines {
        let lines = source
            .lines
            .iter()
            .map(|line| (line.page_index, line.text.clone()))
            .collect::<Vec<_>>();
        by_block
            .entry(source.block_index)
            .or_default()
            .extend(lines);
    }
    let mut by_note: NoteLines = HashMap::new();
    for (block_index, block) in document.blocks.iter().enumerate() {
        if !is_review_note_display_block(block) {
            continue;
        }
        let Some(lines) = by_block.get(&block_index) else {
            continue;
        };
        let on_page = lines
            .iter()
            .filter(|(page, _)| *page == page_index)
            .map(|(_, text)| text.clone())
            .collect::<Vec<_>>();
        if on_page.is_empty() {
            continue;
        }
        for (marker, _) in split_fused_review_notes(&block.text) {
            if let Ok(number) = marker.parse::<u16>() {
                by_note.entry(number).or_insert_with(|| on_page.clone());
            }
        }
    }
    (by_block, by_note)
}

/// A word's place along the flight at overall progress `p`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Flight {
    /// Top-left of the word's galley.
    pub pos: Pos2,
    /// Scale of the landing galley: below 1 while still near print size.
    pub scale: f32,
    pub alpha: f32,
    /// How far this word has left its printed position (0 = not yet).
    pub departed: f32,
    /// Within a pixel of its destination: the settled text shows instead.
    pub settled: bool,
}

/// `end` is where the word's galley origin sits once landed. Words with no
/// destination stay on the printed page and fade with it; they get `None`.
pub(super) fn flight(word: &RevealWord, end: Pos2, p: f32) -> Option<Flight> {
    let to = word.to?;
    let depart = STAGGER * word.order;
    let local = ((p - depart) / (1.0 - depart)).clamp(0.0, 1.0);
    let e = ease_in_out(local);
    let from_size = (word.from.height() * 0.82).clamp(6.0, 40.0);
    let lift = if word.from.center().y > to.rect.center().y {
        18.0
    } else {
        10.0
    };
    let pos = word.from.min + (end - word.from.min) * e
        - Vec2::new(0.0, lift * (std::f32::consts::PI * e).sin());
    let size = from_size + (to.style.size - from_size) * e;
    Some(Flight {
        pos,
        scale: size / to.style.size.max(1.0),
        // Invisible until it lifts, so the printed glyph underneath is the
        // only one the eye sees at rest.
        alpha: smoothstep(0.0, 0.1, local),
        departed: local,
        settled: local >= 0.97,
    })
}

/// How much of the printed page is still on screen at progress `p`. It
/// stays until every word has lifted (the last departs at `STAGGER`).
pub(super) fn page_alpha(p: f32) -> f32 {
    1.0 - smoothstep(STAGGER, 0.62, p)
}

pub(super) fn with_alpha(color: Color32, alpha: f32) -> Color32 {
    color.gamma_multiply(alpha.clamp(0.0, 1.0))
}

fn ease_in_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Matching key: letters and digits, lowercased. Punctuation and ligature
/// forms differ between the text layer and the reflow; letters do not.
pub(super) fn key(text: &str) -> String {
    text.chars()
        .filter(|ch| ch.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Group a page's characters into words with their printed rectangles.
pub(super) fn page_words(chars: &[PageTextChar]) -> Vec<(String, PdfRect)> {
    let mut words = Vec::new();
    let mut text = String::new();
    let mut rect: Option<PdfRect> = None;
    let mut last_right = f32::NAN;
    let mut last_mid = f32::NAN;
    let mut last_height = 0.0f32;
    let flush =
        |text: &mut String, rect: &mut Option<PdfRect>, words: &mut Vec<(String, PdfRect)>| {
            if let Some(r) = rect.take()
                && !text.trim().is_empty()
            {
                words.push((std::mem::take(text), r));
            }
            text.clear();
        };
    for ch in chars {
        let Some(r) = ch.rect else {
            flush(&mut text, &mut rect, &mut words);
            continue;
        };
        if ch.ch.is_whitespace() || ch.ch.is_control() {
            flush(&mut text, &mut rect, &mut words);
            continue;
        }
        let height = (r.top - r.bottom).abs().max(0.1);
        let mid = (r.top + r.bottom) * 0.5;
        if let Some(current) = rect {
            let new_line = (mid - last_mid).abs() > 0.6 * last_height.max(height);
            let gap = r.left - last_right;
            let missing_space = gap > 0.33 * height;
            let backwards = gap < -0.6 * height;
            if new_line || missing_space || backwards {
                flush(&mut text, &mut rect, &mut words);
            } else {
                rect = Some(PdfRect {
                    left: current.left.min(r.left),
                    right: current.right.max(r.right),
                    top: current.top.max(r.top),
                    bottom: current.bottom.min(r.bottom),
                });
            }
        }
        if rect.is_none() {
            rect = Some(r);
        }
        text.push(ch.ch);
        last_right = r.right;
        last_mid = mid;
        last_height = height;
    }
    flush(&mut text, &mut rect, &mut words);
    words
}

/// The words of a drawn galley with their screen boxes and baselines.
/// Glyphs whose font height differs from `font_height` (a note's number
/// beside its body) are skipped when `font_height` is given.
pub(super) fn galley_words(
    galley: &egui::Galley,
    origin: Pos2,
    font_height: Option<f32>,
) -> Vec<DrawnWord> {
    let mut words = Vec::new();
    for row in &galley.rows {
        let top = origin.y + row.pos.y;
        let bottom = top + row.size.y;
        let mut text = String::new();
        let mut left = 0.0f32;
        let mut right = 0.0f32;
        let mut baseline = top;
        let mut flush = |text: &mut String, left: f32, right: f32, baseline: f32| {
            if !text.is_empty() {
                words.push(DrawnWord {
                    text: std::mem::take(text),
                    rect: Rect::from_min_max(Pos2::new(left, top), Pos2::new(right, bottom)),
                    baseline,
                });
            }
        };
        for glyph in &row.glyphs {
            let x0 = origin.x + row.pos.x + glyph.pos.x;
            let x1 = x0 + glyph.advance_width;
            let other_font =
                font_height.is_some_and(|height| (glyph.font_height - height).abs() > 0.5);
            if glyph.chr.is_whitespace() || other_font {
                flush(&mut text, left, right, baseline);
                continue;
            }
            if text.is_empty() {
                left = x0;
                baseline = top + glyph.pos.y;
            }
            right = x1;
            text.push(glyph.chr);
        }
        flush(&mut text, left, right, baseline);
    }
    words
}

/// Where a landed word's galley origin goes so its first glyph sits on the
/// measured baseline at the measured left edge.
pub(super) fn landing_origin(landing: &Landing, galley: &egui::Galley) -> Pos2 {
    let glyph = galley.rows.first().and_then(|row| {
        row.glyphs
            .first()
            .map(|glyph| row.pos + glyph.pos.to_vec2())
    });
    match glyph {
        Some(glyph) => Pos2::new(landing.rect.left() - glyph.x, landing.baseline - glyph.y),
        None => landing.rect.min,
    }
}

/// The layout job for one landed word, in its landing format.
pub(super) fn landing_job(text: &str, style: LandingStyle) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    job.append(
        text,
        0.0,
        egui::text::TextFormat {
            font_id: egui::FontId::proportional(style.size),
            color: Color32::PLACEHOLDER,
            line_height: style.line_height,
            ..Default::default()
        },
    );
    job.wrap.max_width = f32::INFINITY;
    job
}

/// Find each source line, in order, inside the page's word keys. Returns the
/// page word index of every located token. Lines come from the same text
/// layer as the page, so a line is a contiguous run; a run is accepted when
/// its first two tokens match, then walked with one-token tolerance.
pub(super) fn locate_lines(page_keys: &[String], lines: &[&str]) -> Vec<usize> {
    let mut slots = Vec::new();
    let mut cursor = 0usize;
    for line in lines {
        let tokens = line
            .split_whitespace()
            .map(key)
            .filter(|k| !k.is_empty())
            .collect::<Vec<_>>();
        if tokens.is_empty() {
            continue;
        }
        let probe = tokens.len().min(2);
        let start = (cursor..page_keys.len().saturating_sub(probe - 1))
            .chain(0..cursor.min(page_keys.len()))
            .find(|&at| (0..probe).all(|i| page_keys[at + i] == tokens[i]));
        let Some(start) = start else {
            continue;
        };
        let mut at = start;
        let mut i = 0usize;
        while i < tokens.len() && at < page_keys.len() {
            if page_keys[at] == tokens[i] {
                slots.push(at);
                at += 1;
                i += 1;
            } else if at + 1 < page_keys.len() && page_keys[at + 1] == tokens[i] {
                at += 1; // a token on the page the line does not carry
            } else if i + 1 < tokens.len() && page_keys[at] == tokens[i + 1] {
                i += 1; // a token in the line the page does not carry
            } else {
                break;
            }
        }
        cursor = at;
    }
    slots
}

/// A drawn key agrees with a printed one when equal, or when the print
/// carries a footnote number glued to the word ("priors12" for "priors").
fn keys_match(drawn: &str, printed: &str) -> bool {
    if drawn.is_empty() {
        return false;
    }
    if drawn == printed {
        return true;
    }
    printed.len() > drawn.len()
        && printed.starts_with(drawn)
        && drawn.chars().any(char::is_alphabetic)
        && printed[drawn.len()..].chars().all(|c| c.is_ascii_digit())
}

/// Longest common subsequence of drawn keys (`left`) and printed keys
/// (`right`), as (left, right) index pairs. Bounded so a huge block cannot
/// stall a frame.
pub(super) fn align(left: &[String], right: &[String]) -> Vec<(usize, usize)> {
    let n = left.len().min(MAX_ALIGN_TOKENS);
    let m = right.len().min(MAX_ALIGN_TOKENS);
    if n == 0 || m == 0 {
        return Vec::new();
    }
    let mut table = vec![0u16; (n + 1) * (m + 1)];
    let at = |i: usize, j: usize| i * (m + 1) + j;
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            table[at(i, j)] = if keys_match(&left[i], &right[j]) {
                table[at(i + 1, j + 1)] + 1
            } else {
                table[at(i + 1, j)].max(table[at(i, j + 1)])
            };
        }
    }
    let mut pairs = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    while i < n && j < m {
        if keys_match(&left[i], &right[j]) {
            pairs.push((i, j));
            i += 1;
            j += 1;
        } else if table[at(i + 1, j)] >= table[at(i, j + 1)] {
            i += 1;
        } else {
            j += 1;
        }
    }
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(c: char, left: f32, top: f32) -> PageTextChar {
        PageTextChar {
            ch: c,
            rect: Some(PdfRect {
                left,
                right: left + 5.0,
                top,
                bottom: top - 10.0,
            }),
            font_size: Some(10.0),
            bold: false,
            italic: false,
        }
    }

    fn text_chars(line: &str, top: f32) -> Vec<PageTextChar> {
        let mut out = Vec::new();
        let mut x = 0.0;
        for c in line.chars() {
            if c == ' ' {
                out.push(PageTextChar {
                    ch: ' ',
                    rect: None,
                    font_size: None,
                    bold: false,
                    italic: false,
                });
            } else {
                out.push(ch(c, x, top));
            }
            x += 5.0;
        }
        out
    }

    #[test]
    fn page_words_group_characters_and_split_on_missing_spaces() {
        let mut chars = text_chars("the court", 100.0);
        // "held" glued to the next word with no space glyph but a visible gap.
        chars.extend([
            ch('h', 60.0, 100.0),
            ch('e', 65.0, 100.0),
            ch('l', 70.0, 100.0),
            ch('d', 75.0, 100.0),
        ]);
        chars.extend([ch('s', 90.0, 100.0), ch('o', 95.0, 100.0)]);
        let words = page_words(&chars);
        let texts = words.iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>();
        assert_eq!(texts, ["the", "court", "held", "so"]);
        assert_eq!(words[1].1.left, 20.0);
        assert_eq!(words[1].1.right, 45.0);
    }

    #[test]
    fn page_words_break_at_line_changes() {
        let mut chars = text_chars("one", 100.0);
        chars.extend(text_chars("two", 88.0));
        let words = page_words(&chars);
        assert_eq!(words.len(), 2);
    }

    #[test]
    fn locate_lines_walks_contiguous_runs_with_tolerance() {
        let page = [
            "running", "head", "the", "court", "held", "that", "flood", "12", "means", "water",
        ]
        .iter()
        .map(|s| key(s))
        .collect::<Vec<_>>();
        // The reflow dropped the callout "12" that the page carries.
        let slots = locate_lines(&page, &["the court held", "that flood means water"]);
        assert_eq!(slots, vec![2, 3, 4, 5, 6, 8, 9]);
    }

    #[test]
    fn align_pairs_in_order_and_skips_rewrapped_hyphenation() {
        let drawn = ["the", "court", "held", "that", "textualism", "fails"]
            .map(key)
            .to_vec();
        let printed = ["the", "court", "held", "that", "textual", "ism", "fails"]
            .map(key)
            .to_vec();
        let pairs = align(&drawn, &printed);
        assert_eq!(pairs, vec![(0, 0), (1, 1), (2, 2), (3, 3), (5, 6)]);
    }

    #[test]
    fn flight_leaves_late_words_in_place_early_and_lands_everyone() {
        let word = RevealWord {
            text: "held".into(),
            key: "held".into(),
            from: Rect::from_min_size(Pos2::new(100.0, 600.0), Vec2::new(30.0, 12.0)),
            to: Some(Landing {
                rect: Rect::from_min_size(Pos2::new(400.0, 200.0), Vec2::new(40.0, 20.0)),
                baseline: 215.0,
                style: LandingStyle {
                    size: 17.0,
                    line_height: None,
                },
            }),
            order: 0.9,
        };
        let end = Pos2::new(400.0, 201.0);
        let early = flight(&word, end, 0.1).unwrap();
        assert_eq!(early.departed, 0.0);
        assert_eq!(early.pos, word.from.min);
        assert_eq!(early.alpha, 0.0);
        let late = flight(&word, end, 1.0).unwrap();
        assert_eq!(late.pos, end);
        assert!((late.scale - 1.0).abs() < 1e-4);
        assert_eq!(late.alpha, 1.0);
        assert!(late.settled);
        assert!(!flight(&word, end, 0.5).unwrap().settled);
        let stray = RevealWord {
            to: None,
            ..word.clone()
        };
        assert!(flight(&stray, end, 1.0).is_none());
    }

    #[test]
    fn align_accepts_footnote_numbers_glued_to_printed_words() {
        let drawn = ["pro", "business", "priors", "12", "if"].map(key).to_vec();
        let printed = ["pro", "business", "priors12", "if"].map(key).to_vec();
        let pairs = align(&drawn, &printed);
        assert_eq!(pairs, vec![(0, 0), (1, 1), (2, 2), (4, 3)]);
        assert!(!keys_match("12", "1234"));
        assert!(!keys_match("", "x"));
    }

    #[test]
    fn page_stays_until_every_word_has_lifted() {
        assert_eq!(page_alpha(0.0), 1.0);
        assert_eq!(page_alpha(STAGGER), 1.0);
        assert_eq!(page_alpha(0.62), 0.0);
    }
}
