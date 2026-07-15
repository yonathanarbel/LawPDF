# Three Highest-Profit Feature Improvements for Law-Review Liquid Mode

Claude could not be reached because the CLI reported a low credit balance. This file is written from the same feature/performance brief prepared for Claude in `CLAUDE_IDEAS_PROMPT.md`.

## 1. Add A Real Footnote Run Decoder

### Feature / Model Change

Replace the current mostly per-line decision with a lightweight page-level run decoder for footnotes. Keep the current line classifier, but add a second pass that segments each page into contiguous zones:

- body zone
- footnote zone
- repository/front-matter zone
- contents/noise zone

This should not use gold labels at inference. It should use line-level model scores, geometry, divider presence, vertical gaps, font continuity, indentation, note markers, short-form citation cues, and neighboring-line evidence.

The decoder can be simple and fast:

- Compute candidate state costs for each line: `body`, `footnote`, `noise`.
- Add transition penalties:
  - high penalty for body -> footnote before mid-page unless strong evidence exists
  - low penalty for footnote -> footnote with same small font, close vertical gap, same column
  - high penalty for footnote -> body after a numbered note sequence has begun near bottom
  - high penalty for contents/noise -> footnote on contents-like pages
- Use Viterbi/DP per page, or a simpler deterministic start/continue/end state machine.

### Why This Targets Current Failures

The measured failures are mostly not isolated single-line misunderstandings. They are run-boundary errors:

- real footnote continuation lines are kept as body
- body lines near the lower page get pulled into marginalia after an early false start
- old no-divider law reviews need sequence recognition, not just marker recognition
- author-note runs need block-level continuation logic

The current `sequence_footnote_zone` helps, but it is still a hand-built boolean. A decoder can use model scores and context together instead of a single start heuristic.

### Implementation

Rust:

- Add a `LineActionScores` struct after model scoring:
  - `keep_score`
  - `marginalia_score`
  - `hide_noise_score`
  - feature flags already computed on `LayoutLine`
- Add `decode_page_liquid_actions(page, lines, raw_actions)` before emitting final hints.
- Implement transition costs from adjacent line geometry:
  - `same_small_font`
  - `vertical_gap_bucket`
  - `same_left_indent`
  - `same_column`
  - `below_divider`
  - `sequence_footnote_zone`
  - `contents_or_index_entry`
- Output repaired actions, not repaired roles.

Python:

- Mirror the decoder in `tools/layout_role_training.py`.
- Add a `--runtime-decoder-action-eval` mode to compare:
  - current runtime ensemble
  - runtime ensemble + decoder
- Emit confusion and residual examples for decoder-only changes.

### Tests / Evaluation

Unit tests:

- no-divider numbered footnote plus continuation stays marginalia
- lower-page body paragraph without note marker stays keep
- contents dot-leader rows never start a footnote run
- repository citation block on page 1 stays noise/metadata, not marginalia
- `Id. at ...` after a note line stays marginalia

Metrics:

- Primary: `strict_law_review_runtime_ensemble_action_gold_eval`
- Watch:
  - marginalia recall should rise
  - keep precision should rise or remain stable
  - marginalia precision must not fall below current ~92.7%

### Risks

- A decoder can over-smooth and convert a lower-page body passage into marginalia.
- Old law reviews with unusual typography may start footnote zones too early.
- If model scores are poorly calibrated, transition costs may dominate incorrectly.

### Expected Effect

Highest likely gain. Expected to improve `marginalia` recall and `keep` precision. Rough target: +0.5 to +1.5 action accuracy points if tuned on hard residuals.

## 2. Add Column And Block Geometry Features

### Feature / Model Change

Add explicit column/block geometry instead of treating each line as independent page text. Current features know x/y/width, but they do not know enough about columns, body text blocks, footnote columns, or indentation within a block.

New features:

- detected column id
- column count on page
- line belongs to dominant body column
- line belongs to bottom note column
- x0 relative to dominant body left edge
- x0 relative to previous line
- line width relative to dominant body width
- vertical gap from previous line normalized by median line gap
- block start / block continuation flag
- hanging indent pattern for footnotes
- marker protrudes left of continuation text
- footnote run has narrower measure than body run

### Why This Targets Current Failures

Footnotes are often visually distinct not only by font and y-position, but by block geometry:

- numbered marker starts slightly left of note text
- continuation lines align under note text, not marker
- footnotes often have narrower or denser line measure
- two-column law reviews and old scans confuse y-only logic
- body block quotes can be small font but remain in the body column

These features would separate small-font body quotes from actual note runs better than text cues alone.

### Implementation

Rust:

- During `enrich_line_features`, group sorted page lines into rough columns using x-center/x0 clustering.
- Compute per-page dominant body x0/x1/width from medium-font non-noise lines.
- Add fields to `LayoutLine`:
  - `column_index`
  - `column_count`
  - `dominant_body_column`
  - `x0_delta_from_body_left`
  - `width_ratio_to_body`
  - `prev_gap_to_median_gap`
  - `hanging_indent_candidate`
- Emit tokens:
  - `column_count=1/2/many`
  - `column=body/side/unknown`
  - `x_body_delta=...`
  - `width_body_ratio=...`
  - `gap_prev=...`
  - `hanging_indent_note_shape`

Python:

- Mirror clustering and token emission exactly.
- Add feature parity tests like the existing runtime/trainer geometry parity test.
- Include these features in retraining both liquid-core and footnote-specialist candidates.

### Tests / Evaluation

Unit tests:

- body block quote in dominant body column is not marginalia solely due to small font
- hanging-indent numbered footnote emits note-shape token
- continuation line aligned under note text emits continuation-shape token
- two-column page does not use left-column body as previous line for right-column note

Metrics:

- Compare false `keep -> marginalia` body errors before/after.
- Compare `marginalia` precision and `keep` recall.

### Risks

- Column clustering can be brittle on OCR drift or pages with figures/tables.
- Repository covers and TOCs may have their own columns and accidentally look structured.
- Needs careful fallback for sparse pages.

### Expected Effect

Medium-high gain. Best for reducing false marginalia while keeping footnote recall. Likely improves `keep` recall/precision balance and protects marginalia precision.

## 3. Add Page-Level Footnote Prior Features And Calibrated Thresholds

### Feature / Model Change

Add page-level priors describing whether a page likely has footnotes, where the footnote band probably starts, and how dense the note evidence is. Use these priors as features and as runtime thresholds.

New page-level features:

- count of note markers below each y-band
- count of legal citation cues by y-band
- small-font line density by y-band
- divider presence and divider y-position
- estimated footnote band start y
- ratio of small-font lines below estimated band
- page has body-to-note font step change
- page has repeated note-like indentation
- page is repository cover / contents / index / front-matter page

Then use calibrated thresholds:

- On pages with high footnote prior, allow continuation lines with weaker text cues.
- On pages with low footnote prior, require stronger evidence before marginalia.
- On repository/contents pages, strongly suppress marginalia.

### Why This Targets Current Failures

The same line can mean different things on different pages. A small lower-page line in a page with multiple numbered notes is probably a footnote continuation. A small lower-page line on a repository cover is probably boilerplate. The current system has some page-context flags, but not a unified page-level prior.

This targets:

- missed no-divider footnotes
- false footnotes on repository covers
- false footnotes on contents/index pages
- lower-page body text wrongly pulled into marginalia

### Implementation

Rust:

- Add `PageLayoutContext` computed once per page:
  - `footnote_prior`
  - `estimated_note_band_y`
  - `note_marker_count`
  - `legal_cue_count`
  - `small_font_lower_count`
  - `contents_prior`
  - `repository_prior`
- Attach derived buckets to each `LayoutLine`:
  - `page_footnote_prior=low/med/high`
  - `estimated_note_band=none/mid/low`
  - `line_below_estimated_note_band`
  - `page_repository_or_contents_prior`
- Use these in:
  - feature tokens
  - `has_plausible_footnote_geometry`
  - `starts_sequence_footnote_zone`
  - marginalia guards

Python:

- Mirror context computation.
- Add report slices by prior:
  - high-prior pages
  - low-prior pages
  - repository/contents pages
  - no-divider pages

### Tests / Evaluation

Unit tests:

- no-divider page with multiple small numbered notes gets high footnote prior
- repository cover with lower-page citation gets repository prior and low/suppressed footnote action
- contents page with dot leaders gets contents prior and no marginalia
- low-prior page with one small lower body quote does not become marginalia

Metrics:

- Evaluate action metrics by page slice.
- Specifically track:
  - `hide_noise` recall on repository/contents pages
  - `marginalia` recall on high footnote-prior pages
  - `keep` recall on low footnote-prior pages

### Risks

- Bad page prior could suppress true author notes on page 1.
- Some articles have only one footnote on a page; count-based prior may be too conservative.
- Needs special handling for first-page author notes and very old law reviews.

### Expected Effect

Medium gain and good safety. Likely improves `hide_noise` recall and reduces false marginalia. It may improve marginalia recall on high-prior pages if used to relax continuation requirements.
