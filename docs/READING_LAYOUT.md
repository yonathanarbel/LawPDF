# Reading layout

LawPDF's window has three layers: the masthead, the page, and the markup bar.
This note records how each one works and why, for anyone changing them.
The code lives in `src/app/chrome.rs`, `src/app/review_margin_ui.rs`,
`src/review_margin.rs`, and `src/review_masthead.rs`.

## Masthead

The masthead replaces the old two-row toolbar. Its first row is the tab strip.
The second names the document the way a law review page does: authors on the
left, the title in the centre, the citation on the right, over a double rule.
The third holds navigation: the contents menu, the Original / Review / Side by
side switch, find, file actions, reading settings (Review Mode only), and a
**⋯** menu for rarely used actions (OCR, recovery, default reader, Review
details and feedback, settings).

The identity comes from the PDF's own text, so it appears in the original view
before Review Mode has run. `review_masthead.rs` reads the opening page's byline
and title block, the running heads (`452 NEW YORK UNIVERSITY LAW REVIEW
[Vol. 99:451` and `May 2024] GENERATIVE INTERPRETATION 453`), and the volume
line. The printed opening page wins over Review Mode's title, because Review
Mode sometimes takes the journal's name for the title. Documents without a
journal identity fall back to the file name.

The title face is Frank Ruhl Libre Black, embedded in the binary. Everything
else uses EB Garamond.

## Markup bar

Marking up moved to a floating bar: select, marker, text box, and signature;
the six marker presets; undo and redo; rotate (original view) or read aloud
(Review Mode); zoom, which changes text size in Review Mode; page controls in
the original view; and save status. Status messages appear above the bar for a
few seconds instead of in a status line.

Drag the bar by its grip to move it. The position is saved as an offset from
its home at the bottom centre of the page area and is always kept inside the
window. Double-click the grip to send it home.

## Footnotes in the margin

Review Mode draws the body column first and records where each footnote number
landed. The notes are then placed in the margin beside those lines. The rules,
all in `review_margin.rs` and unit-tested there:

1. A note starts at its callout, or just below the note above it.
2. A note that would run into the next note's callout is shortened to the rows
   that fit, with a "more" line, but never below three rows.
3. Any note longer than nine rows is shortened even when there is room.
4. A note pushed far below its callout by crowding shrinks to two rows so the
   stack catches up with the text.
5. Notes belong in the right margin. A note crosses to the left margin only
   when the right one is crowded and the left would seat it closer to its line.

The body text never moves to make room. Clicking a shortened note, or its
number in the text, opens the whole note over the margin; Esc or a click
elsewhere closes it. Hovering a note lights up its number in the text, and the
reverse.

When the window is too narrow for margins at the chosen column width, the
column narrows (down to 540 points) before the margins are given up. Below
that, notes open as popovers from their numbers, as before. Every note is also
listed at the end of the article.

The side panel (pages, search, chat, notes) stays open in the original view
and closed in Review Mode by default; the button at the left of the masthead
toggles it, and the choice is remembered for each view.
