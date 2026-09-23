//! Placement of Review Mode side notes.
//!
//! A side note belongs beside the line that cites it. Law review notes are often
//! longer than the paragraph they annotate, and several can share one line, so a
//! note cannot always have the room it wants. The reader rarely needs every note
//! in full while reading, so the layout keeps the body column steady and makes the
//! notes give way instead:
//!
//! 1. Each note starts at its callout, or just below the note above it.
//! 2. A note that would run into the next note's callout is cut to the rows that
//!    fit, with a "more" row, but never below a legible minimum.
//! 3. Very long notes are capped even when there is room, so one citation string
//!    cannot fill the margin.
//! 4. When crowding has pushed a note well below its callout, it shrinks to two
//!    rows so the stack catches up with the text instead of drifting away.
//! 5. When the right margin is crowded, a note may cross to the left margin
//!    (`assign_margin_sides`), so a dense page uses both.
//!
//! The full note is always one click away. Nothing here depends on egui, so the
//! rules can be tested directly.

/// One note waiting for a place in the margin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MarginNoteRequest {
    /// Top of the text line that cites the note, in the same space as the result.
    pub anchor_top: f32,
    /// Rows the note needs to show in full at the margin width.
    pub total_rows: usize,
}

/// Shared measurements for a margin column.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MarginNoteMetrics {
    /// Height of one wrapped text row.
    pub row_height: f32,
    /// Fixed height around the text rows (padding).
    pub chrome_height: f32,
    /// Height of the "more" row shown under a shortened note.
    pub more_height: f32,
    /// Vertical space kept between two notes.
    pub gap: f32,
    /// Most rows a note shows before it is shortened, even with room to spare.
    pub max_rows: usize,
    /// Fewest rows a shortened note keeps while it is still near its callout.
    pub min_rows: usize,
    /// How far below its callout a note may be pushed before it collapses to two rows.
    pub drift_limit: f32,
}

impl MarginNoteMetrics {
    pub fn note_height(&self, rows: usize, shortened: bool) -> f32 {
        self.chrome_height
            + rows as f32 * self.row_height
            + if shortened { self.more_height } else { 0.0 }
    }
}

/// Where a note goes and how much of it shows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MarginNotePlacement {
    pub top: f32,
    pub height: f32,
    /// Text rows shown.
    pub rows: usize,
    /// True when the note shows fewer rows than it has.
    pub shortened: bool,
}

impl MarginNotePlacement {
    pub fn bottom(&self) -> f32 {
        self.top + self.height
    }
}

/// Rows a note keeps once crowding has pushed it far from its callout.
const COLLAPSED_ROWS: usize = 2;

/// Place notes in reading order. Requests must be sorted by `anchor_top`; the
/// caller builds them in document order, which already is.
pub fn place_margin_notes(
    requests: &[MarginNoteRequest],
    metrics: &MarginNoteMetrics,
) -> Vec<MarginNotePlacement> {
    let min_rows = metrics.min_rows.max(1);
    let max_rows = metrics.max_rows.max(min_rows);
    let mut placements = Vec::with_capacity(requests.len());
    let mut previous_bottom: Option<f32> = None;

    for (index, request) in requests.iter().enumerate() {
        let total_rows = request.total_rows.max(1);
        let top = match previous_bottom {
            Some(bottom) => request.anchor_top.max(bottom + metrics.gap),
            None => request.anchor_top,
        };
        let drift = top - request.anchor_top;

        let mut rows = total_rows.min(max_rows);
        if drift > metrics.drift_limit {
            rows = rows.min(COLLAPSED_ROWS);
        } else if let Some(next) = requests.get(index + 1) {
            let room = next.anchor_top - metrics.gap - top;
            if metrics.note_height(rows, rows < total_rows) > room {
                let usable = room - metrics.chrome_height - metrics.more_height;
                let fit = if usable > 0.0 && metrics.row_height > 0.0 {
                    (usable / metrics.row_height).floor() as usize
                } else {
                    0
                };
                rows = fit.clamp(min_rows.min(total_rows), rows);
            }
        }
        let rows = rows.min(total_rows).max(1);
        let shortened = rows < total_rows;
        let height = metrics.note_height(rows, shortened);
        previous_bottom = Some(top + height);
        placements.push(MarginNotePlacement {
            top,
            height,
            rows,
            shortened,
        });
    }
    placements
}

/// Which margin a note goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarginSide {
    Right,
    Left,
}

/// Choose a margin for each note. Notes belong on the right; a note moves to
/// the left margin only when the right one is so crowded that the note would
/// land more than `patience` below its callout and the left margin would seat
/// it higher. Heights are estimated at the row cap, before any shortening.
pub fn assign_margin_sides(
    requests: &[MarginNoteRequest],
    metrics: &MarginNoteMetrics,
    patience: f32,
) -> Vec<MarginSide> {
    let max_rows = metrics.max_rows.max(1);
    let mut right_bottom: Option<f32> = None;
    let mut left_bottom: Option<f32> = None;
    let top_after = |bottom: Option<f32>, anchor: f32| match bottom {
        Some(bottom) => anchor.max(bottom + metrics.gap),
        None => anchor,
    };
    requests
        .iter()
        .map(|request| {
            let rows = request.total_rows.clamp(1, max_rows);
            let height = metrics.note_height(rows, rows < request.total_rows);
            let right_top = top_after(right_bottom, request.anchor_top);
            let left_top = top_after(left_bottom, request.anchor_top);
            let side = if right_top - request.anchor_top <= patience || right_top <= left_top {
                MarginSide::Right
            } else {
                MarginSide::Left
            };
            match side {
                MarginSide::Right => right_bottom = Some(right_top + height),
                MarginSide::Left => left_bottom = Some(left_top + height),
            }
            side
        })
        .collect()
}

/// Lowest point any placed note reaches, so the scroll area can make room.
pub fn margin_notes_bottom(placements: &[MarginNotePlacement]) -> Option<f32> {
    placements
        .iter()
        .map(MarginNotePlacement::bottom)
        .reduce(f32::max)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metrics() -> MarginNoteMetrics {
        MarginNoteMetrics {
            row_height: 10.0,
            chrome_height: 4.0,
            more_height: 8.0,
            gap: 6.0,
            max_rows: 8,
            min_rows: 3,
            drift_limit: 60.0,
        }
    }

    fn request(anchor_top: f32, total_rows: usize) -> MarginNoteRequest {
        MarginNoteRequest {
            anchor_top,
            total_rows,
        }
    }

    #[test]
    fn a_lone_short_note_sits_at_its_callout_in_full() {
        let placed = place_margin_notes(&[request(100.0, 2)], &metrics());
        assert_eq!(placed.len(), 1);
        assert_eq!(placed[0].top, 100.0);
        assert_eq!(placed[0].rows, 2);
        assert!(!placed[0].shortened);
        assert_eq!(placed[0].height, 24.0);
    }

    #[test]
    fn a_note_is_pushed_below_the_note_above_it() {
        let placed = place_margin_notes(&[request(0.0, 2), request(10.0, 2)], &metrics());
        assert_eq!(placed[1].top, placed[0].bottom() + 6.0);
        assert!(!placed[1].shortened);
    }

    #[test]
    fn a_long_note_gives_way_to_the_next_callout() {
        // Room before the next callout: 100 - 6 - 0 = 94. Chrome 4 + more 8 leaves 82: 8 rows.
        // The cap is 8 rows, so ask for a smaller room: next callout at 60 -> 54 - 12 = 42 -> 4 rows.
        let placed = place_margin_notes(&[request(0.0, 20), request(60.0, 2)], &metrics());
        assert!(placed[0].shortened);
        assert_eq!(placed[0].rows, 4);
        assert!(placed[0].bottom() + 6.0 <= placed[1].top + 0.001);
        assert_eq!(placed[1].top, 60.0);
    }

    #[test]
    fn a_very_long_note_is_capped_even_with_room() {
        let placed = place_margin_notes(&[request(0.0, 30)], &metrics());
        assert_eq!(placed[0].rows, 8);
        assert!(placed[0].shortened);
        assert_eq!(placed[0].height, 4.0 + 80.0 + 8.0);
    }

    #[test]
    fn a_crowded_note_keeps_a_legible_minimum_and_pushes_the_next() {
        // The next callout is only 10 points below: no room at all.
        let placed = place_margin_notes(&[request(0.0, 12), request(10.0, 1)], &metrics());
        assert_eq!(placed[0].rows, 3);
        assert!(placed[0].shortened);
        assert_eq!(placed[1].top, placed[0].bottom() + 6.0);
    }

    #[test]
    fn a_short_note_is_never_padded_up_to_the_minimum() {
        let placed = place_margin_notes(&[request(0.0, 1), request(2.0, 1)], &metrics());
        assert_eq!(placed[0].rows, 1);
        assert!(!placed[0].shortened);
    }

    #[test]
    fn notes_pushed_far_from_their_callouts_collapse_to_two_rows() {
        let requests = (0..8).map(|i| request(i as f32, 12)).collect::<Vec<_>>();
        let placed = place_margin_notes(&requests, &metrics());
        let collapsed = placed.iter().filter(|p| p.rows == 2).count();
        assert!(collapsed >= 4, "{placed:?}");
        // Collapsing keeps the stack from marching away: it grows by two rows plus the gap.
        let last = placed.last().unwrap();
        let before = placed[placed.len() - 2];
        assert_eq!(last.top, before.bottom() + 6.0);
        assert_eq!(last.height, 4.0 + 20.0 + 8.0);
        // A one-row note is never stretched by the collapse rule.
        let short = place_margin_notes(
            &[request(0.0, 30), request(0.0, 1)],
            &MarginNoteMetrics {
                drift_limit: 0.0,
                ..metrics()
            },
        );
        assert_eq!(short[1].rows, 1);
        assert!(!short[1].shortened);
    }

    #[test]
    fn placements_never_overlap_and_never_rise_above_their_callouts() {
        let mut seed = 7_u64;
        let mut next = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) as u32
        };
        for _ in 0..200 {
            let count = 1 + (next() % 12) as usize;
            let mut anchor = 0.0_f32;
            let requests = (0..count)
                .map(|_| {
                    anchor += (next() % 90) as f32;
                    request(anchor, 1 + (next() % 25) as usize)
                })
                .collect::<Vec<_>>();
            let placed = place_margin_notes(&requests, &metrics());
            for (request, placement) in requests.iter().zip(&placed) {
                assert!(placement.top >= request.anchor_top);
                assert!(placement.rows >= 1 && placement.rows <= request.total_rows);
                assert_eq!(placement.shortened, placement.rows < request.total_rows);
            }
            for pair in placed.windows(2) {
                assert!(pair[1].top >= pair[0].bottom() + 6.0 - 0.001, "{placed:?}");
            }
        }
    }

    #[test]
    fn notes_stay_right_until_the_right_margin_is_crowded() {
        let metrics = metrics();
        // Well spaced: all right.
        let spaced = [request(0.0, 2), request(100.0, 2), request(200.0, 2)];
        assert!(
            assign_margin_sides(&spaced, &metrics, 20.0)
                .iter()
                .all(|side| *side == MarginSide::Right)
        );
        // Three long notes on one line: the second overflows left, the third
        // returns right because the left is now taller.
        let crowded = [request(0.0, 8), request(0.0, 8), request(0.0, 8)];
        assert_eq!(
            assign_margin_sides(&crowded, &metrics, 20.0),
            vec![MarginSide::Right, MarginSide::Left, MarginSide::Right]
        );
    }

    #[test]
    fn a_short_push_is_not_worth_crossing_the_page() {
        let metrics = metrics();
        // The second note waits 18 points on the right; patience is 30.
        let requests = [request(0.0, 1), request(2.0, 1)];
        assert_eq!(
            assign_margin_sides(&requests, &metrics, 30.0),
            vec![MarginSide::Right, MarginSide::Right]
        );
    }

    #[test]
    fn bottom_reports_the_lowest_note() {
        let placed = place_margin_notes(&[request(0.0, 2), request(100.0, 3)], &metrics());
        assert_eq!(margin_notes_bottom(&placed), Some(100.0 + 4.0 + 30.0));
        assert_eq!(margin_notes_bottom(&[]), None);
    }
}
