# Segmenting bound volumes into articles

## The task

A large minority of the PDFs LawPDF is asked to convert are not articles. They
are **bound volumes**: an entire year of a law journal scanned as one file,
containing a dozen or more separate articles, each with its own title, author,
body, and footnote sequence starting at 1.

Every stage of the LM2 pipeline currently treats the file as one document. This
is wrong in a way that is invisible for modern single-article PDFs and
destructive for the volumes, and it blocks at least one structural fix that is
otherwise ready to land.

**Goal: identify article boundaries inside a PDF, and make the document-scoped
parts of the pipeline operate per article rather than per file.**

## Why it matters

The immediate motivation is a measured failure. A constraint was implemented to
replace three hand-written local rules about where a footnote begins, using the
invariant that *footnote numbers ascend*. Measured on the 100-article benchmark:

| | v0.2.15 (shipped) | with the constraint |
|---|---:|---:|
| Mean source recall | 0.945 | 0.947 |
| Documents with linked footnotes | 90 | 92 |
| Defects per 10k words | 5.66 | **7.14** |
| `footnote.definition_in_body` | 293 | **462** |

Rejected and reverted. The damage was concentrated **entirely** in bound
volumes; every modern single-article PDF moved by −1 to 0 defects.

| document | added defects | what it is |
|---|---:|---|
| `b25-georgetow-glj-vol030-1941` | +42 | Georgetown Law Journal, vol. 30 |
| `b24-georgetow-glj-vol030-1941` | +39 | same volume |
| `b22-georgetow-glj-vol030-1941` | +37 | same volume |
| `b30-georgetow-glj-vol014-1925` | +26 | Georgetown Law Journal, vol. 14 |
| `b26-georgetow-glj-vol016-1927` | +26 | Georgetown Law Journal, vol. 16 |
| `b97-st-stjohns-vol13-iss1` | +16 | St. John's Law Review, vol. 13 |
| `b28-georgetow-glj-vol004-1915` | +12 | Georgetown Law Journal, vol. 4 |

The invariant is correct. The **scope** was wrong: footnote numbers ascend *per
article*, not per file. A single longest-ascending-run across a volume selects
one article's numbering chain and demotes every other article's note heads to
continuations. Their text then merges into the preceding note and surfaces as
`footnote.definition_in_body`.

The reverted patch is preserved at:

```
<scratchpad>/global-note-sequence.patch     # 247 lines, applies to src/liquid2.rs
```

It is worth reading before starting. The invariant and the
longest-ascending-subsequence implementation in it are sound and reusable; only
the unit they were applied to was wrong. With correct article boundaries, that
patch is expected to become a net win, and it deletes 117 lines of hand-written
guards when it does.

## Secondary payoff

Article segmentation is not only useful for footnotes. Every document-scoped
computation in the pipeline is currently averaged across a whole volume:

- **Font statistics.** `doc_font_body_size`, `doc_font_body_z`,
  `doc_font_footnote_z` on `DeepLiquidSourceLine` are computed per file. A
  volume spanning 1915–1943 typesetting conventions, or containing both
  articles and book reviews set at different sizes, produces a body-size
  estimate that fits nothing.
- **Heading hierarchy.** Heading nesting is meaningless across article
  boundaries; every article restarts at its own top level.
- **Running heads and furniture.** Detected by repetition across pages. A volume
  has *several* running heads, one per article, each repeating only within its
  own page range — so each is less repetitive than the threshold expects.
- **The document verifier.** `tools/md_verify.py` checks monotone footnote
  numbering and marker↔definition bijection across the whole file, so it reports
  `footnote.sequence_gap` and `sequence_disorder` on volumes that are in fact
  correct per article. 111 of the current defects are in these two categories.

So the same change plausibly improves recall, defect rate, and the trustworthiness
of the measurement instrument at once. It also makes a natural product feature
possible — offering the user a table of contents and letting them extract one
article.

## Where the code lives

The LM2 pipeline, in order:

1. **Extraction** — `src/pdf_backend.rs`. pdfium via `pdfium-render 0.8`.
   `extract_text_chars` (~line 543) builds per-character geometry.
2. **Layout lines** — produces `DeepLiquidSourceLine`
   (`src/liquid/model.rs:194`), one per visual line, carrying page index,
   geometry, margin ratios, and font statistics.
3. **Emission model** — a CatBoost classifier assigning each line an
   `Lm2Action` (`src/liquid2.rs:234`): `Keep`, `Marginalia`, `HideNoise`.
   Models live in `profile-models/`, loaded via the `LAWPDF_MODEL_DIR`
   environment variable.
4. **Context two-pass model, sequential decoder, ~30 named overlays** — all in
   `src/liquid2.rs`. Note the `-6.0` footnote bias; the stack downstream is
   co-adapted to the shipped model's deliberate miscalibration (see below).
5. **Block assembly** — `build_lm2_blocks_with_grouping`
   (`src/liquid2.rs:8292`) groups lines into blocks;
   `apply_action_neutral_blocksplit` (`:8495`) splits them;
   `paragraph_boundary` (`:8957`) decides paragraph starts.
   Entry point for the whole stage: `prepare_liquid_mode2_document_with_timing`
   (`:2083`).
6. **Markdown generation** — `src/liquid/markdown.rs`. Footnote linking is
   gated by `MIN_FOOTNOTE_LANDING_RATE = 0.20` (`:38`) and
   `MAX_FOOTNOTE_AMBIGUOUS_RATE = 0.02` (`:41`).

Relevant helpers for this task:

- `note_head_marker` (`src/liquid2.rs:9120`) — parses a leading note number
  from block or line text.
- `source_line_note_markers` (`:9093`) — note markers found in a line.
- `looks_like_marginalia_note_block_start` (`:9885`).
- `front_matter_zone` on `DeepLiquidSourceLine` — already computed, likely
  useful.

## Signals available for finding boundaries

All of these are already computed and require no new extraction work:

- **Footnote numbering resets.** The strongest signal. A drop from a high note
  number back to 1 or 2 is an article boundary with very high precision. Use
  `note_head_marker` over marginalia lines.
- **Page-level restart.** Articles in bound volumes almost always start on a new
  page. `page_index` is on every line, so candidate boundaries can be
  restricted to page starts, which massively reduces the search space.
- **Title-page typography.** An article's first page has a distinctive
  signature: large centred text, no footnotes or few, often a byline, and
  frequently an unusually large top margin. `margin_centered`, `font_ratio_page`,
  `indent_vs_body` and `line_width_ratio` are all populated.
- **Running-head change.** The repeated header text changes at an article
  boundary. Detecting repeated text per page band is already done for furniture
  removal in `src/liquid/markdown.rs` (`repeated_noise_texts`).
- **Page-number resets or jumps** in the folio band.

Note that the OCR'd scans that make up most of the affected set may have
unreliable geometry; see the `origin_y` caveat below.

## Suggested approaches

Not prescriptive — but a rough ordering by cost.

**1. Boundary detection as a scored decision over page starts.** For each page
start, score the evidence above; take a boundary where the score clears a
threshold. Simple, inspectable, and testable against a hand-labelled boundary
set. The risk is that it becomes another hand-tuned rule, which this project has
a poor record with (see below).

**2. Segmentation as a global optimisation.** Choose the set of boundaries that
best explains the observed note-number sequence — i.e. the partition minimising
the number of ascending runs needed to cover the note numbers, subject to
boundaries falling on page starts. This is attractive because it uses the same
invariant as the reverted patch and needs no threshold, and it directly produces
the per-article scoping the patch requires. Probably the best fit for the
problem as stated.

**3. A learned boundary classifier.** Labels are derivable without annotation
from note-number resets, so a training set can be bootstrapped. Consistent with
the project's general direction but heavier, and the measured lesson below
argues for doing 2 first and seeing whether it suffices.

Whatever the mechanism, the deliverable is the same: a list of article
boundaries (line index or page index), threaded into the document-scoped stages.

## Coordination

**Read `AGENT_BOARD.md` first.** Another agent (agent1) is working on footnote
linking and extraction in this same tree, and the build output, the executable,
and all three caches are global to the machine rather than to a checkout — so a
git worktree does not separate you. A `cargo build` during someone else's sweep
kills that sweep by replacing the running binary, and uncommitted work gets
compiled into the other agent's measurements. Claim the machine on the board
before building or sweeping.

`AGENT_BOARD.md` also records findings that are already measured and should not
be re-derived, including a negative result on superscript detection in scanned
volumes that affects the same documents you are working on.

## How to measure — read this before touching anything

This project has a benchmark and a label-free verifier. **Use them; do not trust
spot checks or unit tests as evidence of end-to-end quality.**

```bash
# One sweep of the 100-article benchmark, ~100 minutes.
bash <scratchpad>/sweep100.sh <tag> C:/tmp/lawpdf-target/release/lawpdf.exe

# Compare two sweeps.
python tools/bench_compare.py --before <scratchpad>/optC2.json \
                              --after  <scratchpad>/<tag>.json
```

**`optC2.json` is no longer a usable baseline.** It does not correspond to any
known tree state — comparisons against it were found to be picking up another
agent's uncommitted changes. Generate a fresh baseline from a committed tree,
with the machine claimed, before trusting any comparison.

`bench_compare.py` reports
coverage, fidelity, content, and like-for-like defects, and **fails on any
per-document recall regression**. It judges content-recovering changes on defect
*rate* rather than count, because recovered text brings its own defects with it
and a change that deletes text must never score better than one that keeps it.

`tools/md_verify.py` is the verifier itself: it checks monotone footnote
numbering, marker↔definition bijection, heading nesting, furniture removal, and
multiset token recall against the source PDF, split into body and footnote zones
by font size. It needs no labels, which is what makes iteration possible.

## Traps that have already cost time

Every one of these produced a wrong conclusion at least once in this project.

1. **Clear the caches.** Block assembly is cached under a schema version, so
   changes to `src/liquid2.rs` are *invisible* until you delete
   `%APPDATA%/LawPDF/liquid2-cache`, `%APPDATA%/LawPDF/liquid2-fast-cache`, and
   `%LOCALAPPDATA%/LawPDF/performance-cache`. `sweep100.sh` does this; ad hoc
   runs do not. This caused two false "my code isn't running" conclusions.
2. **Use the right exporter.** `--lm2-assemble-markdown` is a *simplified*
   renderer with no footnote linking. The real Copy MD path is
   `--lm2-copy-md`. Measuring the wrong one invalidates everything.
3. **Set `LAWPDF_MODEL_DIR`.** Without it the binary probes relative to the
   executable, silently falls back to `lm2-heuristic-fallback`, and produces
   plausible but meaningless output.
4. **Never run two sweeps concurrently, and never build while one runs.** They
   share the cache; concurrent sweeps produced fabricated numbers once, and a
   `cargo build` mid-sweep killed a run by relinking the binary underneath it.
5. **Do not put derived artifacts in the repository.** It lives in a synced Box
   folder. Sweeps, JSON reports, and installers go to the scratchpad.
6. **`footnote.definition_in_body` is gated** on the document having numbered
   definitions, so a document emitting *nothing* scores clean. Defect counts can
   therefore rise when content is correctly recovered. This is why
   `bench_compare.py` uses density.

## The single most important lesson

**Improving one stage in isolation has repeatedly made the product worse.**

Retraining the emission model raised line macro F1 from 0.687 to 0.905 and
*doubled* held-out defects, because the decoder, overlays, and the −6.0 footnote
bias above it are co-adapted to the shipped model's miscalibration. Four
retraining strategies all lost. A hyperparameter search bought +0.0014
validation F1. More training data was ruled out; re-OCR was ruled out for recall
purposes because extraction already recovers 98–100% of the text layer.

Of five hand-written guards shipped in this project, one caused a large
regression, one was inert, and two were reverted after measuring no effect.

So: measure end to end, on the benchmark, with the verifier, before believing
anything.

## Acceptance criteria

1. Boundaries are detected on the affected volumes. A hand-labelled boundary set
   over the ~16 bound volumes in the benchmark is a reasonable first deliverable
   and is cheap to produce.
2. No regression on single-article PDFs. These are the overwhelming majority of
   real usage and must be untouched — a file with one article should segment
   into exactly one article.
3. `bench_compare.py` against `optC2.json` shows no per-document recall
   regression, and defect density does not rise.
4. **Then** re-apply `global-note-sequence.patch` scoped per article and measure
   again. Success is a fall in `footnote.definition_in_body` below the v0.2.15
   baseline of 293, with 117 lines of local guards deleted.

## Related open items

- **`origin_y` probe — answered, negative.** `pdfium-render` exposes
  `tight_bounds()` and `origin_y()` per character; `src/pdf_backend.rs:553`
  reads neither, using `loose_bounds()` — the font em box, which is uniform per
  font size. Three superscript-detection attempts failed because of this.

  Measured by adding both fields and comparing each digit's baseline to the
  median baseline of alphabetic characters on the same visual line (same-line
  determined by em-box vertical overlap, otherwise the running head is compared
  against the body beneath it and every digit looks raised):

  | | b01 (modern, footnotes work) | b23 (1942 scan, fails) |
  |---|---|---|
  | em-box height discriminates | yes, 18/34 shrunk | no, 0/19 |
  | tight glyph height | 6/34 | 0/19 |
  | baseline raised | 16/90, 26/76 | 188/1533 |

  On b01 every raised digit is a genuine footnote marker or note head, at a
  consistent 40% of font size in the body and 32% in the notes. Modern
  producers set superscripts in a smaller font, so the em box happens to carry
  the signal — which is why modern documents have always worked.

  On b23 the raised digits are **not markers**. They are ordinary digits inside
  citations and dates (`83 F. (2d)`, `February 21, 1940`, `275 F. 539`,
  `77th Congress`) and the folio in the running head, with magnitudes scattered
  from 12% to 25% and no clustering. That is OCR baseline jitter.

  **Conclusion: the superscript signal is absent from these files' text layer.**
  No PDF library, fine-tune, or from-scratch content-stream parser can recover
  it, because they would all be reading the same missing data. The remaining
  options for this document class are rasterising the page and measuring ink, or
  re-OCR. The probe is preserved at `<scratchpad>/origin-y-probe.patch`; it was
  reverted from the tree because it costs two extra FFI calls per character in
  the interactive open path and bought nothing for documents that already work.
- **`b07-chicago`** — a modern document with footnote landing rate 0.864 and
  zero definitions emitted. Isolated bug, unrelated to segmentation.
- 16 of 99 benchmark documents emit almost no footnote apparatus despite having
  22–123 note-head-like lines. Nearly all are pre-1950 scans. Segmentation is
  expected to help but may not be sufficient on its own.
