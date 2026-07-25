# Markdown reconstruction: measured baseline

## How to measure this correctly

Three traps invalidate a naive measurement. All three silently produce
plausible-looking numbers for the wrong build.

1. **Use the real generator.** `--lm2-assemble-markdown` renders blocks with a
   simplified smoke renderer that performs no footnote linking and skips the
   final assembly rules, so it cannot evaluate shipped Markdown. Use
   `--lm2-copy-md`, which calls `liquid_document_markdown` with default
   options, exactly as Review Mode's Copy MD does. It reproduces the committed
   corpus (article 011: 98 blocks, 167 links).
2. **Load the model.** The binary probes for its CatBoost model relative to the
   executable, so a `cargo build` run from the repo silently falls back to
   `lm2-heuristic-fallback`. Set `LAWPDF_MODEL_DIR` to the repo's
   `profile-models/` and confirm `--lm2-runtime-status` reports
   `native_catboost_context`. The difference is large: footnote landing rates
   on held-out articles moved from 0.19/0.53/0.77 to 0.89/1.00/0.94.
3. **Clear the document cache.** Assembled blocks are cached under
   `%APPDATA%/LawPDF/liquid2-cache/`, keyed by a signature built from the model
   label and `LM2_SCHEMA_VERSION`. Any change to block assembly in `liquid2.rs`
   is invisible until the cache is dropped or the schema version bumped —
   which is what that 40-term version string is for. Changes to
   `liquid/markdown.rs` are unaffected, since the generator runs after the
   cache is read.


Measured 2026-07-24 with `tools/md_verify.py` over the five-article
`agentic-review-corpus`.

Reproduce the baseline row with:

```
python tools/md_verify.py agentic-review-corpus/markdown/ \
    --source agentic-review-corpus/originals/
```

and the current rows by regenerating first (see "How to measure this
correctly" for why each flag matters):

```
LAWPDF_MODEL_DIR=profile-models rm -rf "$APPDATA/LawPDF/liquid2-cache"
lawpdf --lm2-copy-md --output out/ agentic-review-corpus/originals/*.pdf
python tools/md_verify.py out/ --source agentic-review-corpus/originals/
```

Machine-readable reports are written under `training-data/eval-reports/`, which
this repository does not track.

## Result after the fixes below

All rows measured with the same verifier build.

| | Docs clean | Critical defects | Per document | Mean source recall | Worst |
|---|---:|---:|---:|---:|---:|
| Baseline (committed corpus) | 0/5 | 347 | 69.4 | 78.9% | 68.0% |
| Now, in-sample | **1/5** | **58** | **11.6** | **94.3%** | 91.5% |
| Now, held-out (never used to derive a fix) | 0/5 | **26** | **5.2** | **94.0%** | 91.1% |

The held-out five were drawn by seeded random sample from a filtered pool of
law review articles and were never inspected before a fix was written. They
carry *fewer* defects per document than the in-sample five, so nothing is tuned
to the corpus.

Held-out critical defects fell 106 → 100 → 53 → 43 → 26 across the iterations.
Per-document defects fell 69.4 → 5.2, a 92% reduction; content loss fell from
21% of source text to 6%.

Article 011, previously the worst "known hard case" — 61 orphan notes and a
767-word fused paragraph — is clean. Held-out h03 is down to a single defect.

Two classes went to zero on both sets: `footnote.orphan_definition` (281 → 0)
and `paragraph.suspected_fusion` (47 → 0 in-sample), the latter without any
rise in `paragraph.fragment`, so paragraphs split at real boundaries rather
than being chopped. Largest paragraph fell from 2,646 words to 233.

What remains out of sample is thinly spread with no dominant class: 5 footnote
sequence gaps, 5 footnote definitions still rendered as body text, 4
unterminated headings, 3 markers opening a paragraph, and single-digit counts
elsewhere. Each now needs individual investigation rather than a systemic fix.

The baseline row is the committed corpus, produced on the heuristic-fallback
runtime; the current rows use the native model tier. That configuration
difference is part of the improvement, not all of it — see "How to measure this
correctly".

## Headline (original diagnosis)

**0 of 5 documents are clean. Mean source recall is 78.9%.** Between 3% and 32%
of each article's text is absent from the export, and body prose is lost at
roughly three times the rate of footnotes.

| Article | Source recall | Body recall | Footnote recall | Orphan notes |
|---|---:|---:|---:|---:|
| 011 co-governance | 75% | 69% | 85% | 61 / 167 (37%) |
| 037 cost-of-justice | 97% | 85% | 99% | 0 / 312 (0%) |
| 042 beyond-intent | 78% | 53% | 92% | 49 / 132 (37%) |
| 047 antisemitism-title-vi | 68% | 55% | 86% | 56 / 127 (44%) |
| 057 contract-wrapped-property | 77% | 73% | 80% | 115 / 458 (25%) |

The two recall figures measure different failures. *Source recall* is text that
did not survive anywhere — pure loss. *Body recall* is source body tokens that
did not land in an output body paragraph, so it also counts text that survived
but was routed into the footnote zone. Article 037 is the clear case: 97%
source recall with 85% body recall means little was dropped, but roughly a
seventh of its body prose was emitted as footnote content.

## Two findings that change the problem statement

**1. The Phase 3 footnote metric was one-directional.** `PHASE3_MARKDOWN_QA.md`
reports "0 unresolved referenced IDs" for articles 011, 037, 042, 047, and 057.
That check is real but only tests reference → definition, which is trivially
satisfiable by emitting fewer references. The reverse direction was never
measured: 25–44% of footnote definitions in four of the five articles have no
corresponding marker anywhere in the body. Only article 037 achieves a true
bijection.

The unlinked notes arrive in contiguous runs, not scattered singletons. Article
011's body markers run 1–14, 22–23, 28–31, 46–53, 57–60, 64–74, 86–87, 96,
101–112, 118–120. Whole regions are lost at once, which points at page- or
column-level routing rather than per-superscript detection.

**2. Content loss dominates, and no existing metric can see it.** The promotion
gate scores line-level macro F1 over lines that survive into the pipeline. When
a fifth of the source never reaches that stage, the gate is scoring a biased
sample of the easy cases, which is consistent with a reported macro F1 of 1.0
against a product that fails every article it is pointed at.

Sampling the dropped lines shows two distinct causes. Footnote definitions lose
their continuation lines while the first line survives — a paragraph-boundary
failure inside the footnote zone. Body prose is dropped in consecutive runs
mid-paragraph, for example page 3 of article 047 losing "support for Israel,
then, we have to answer a critical threshold question: / How is anti-Jewish
discrimination covered by Title VI at all?"

Running headers are dropped correctly and should stay dropped; the verifier
does not count furniture removal as loss.

## What the invariant checks find

347 critical defects and 62 warnings across the corpus.

| Count | Defect |
|---:|---|
| 281 | `footnote.orphan_definition` — note defined, never referenced in body |
| 40 | `recall.dropped_line` — source line absent from the export |
| 29 | `footnote.marker_opens_paragraph` — `[^n]` opens a paragraph |
| 17 | `footnote.unconverted_marker` — bare note number opens a paragraph |
| 8 | `paragraph.fragment` |
| 5 | `footnote.sequence_gap` |
| 5 | `paragraph.suspected_fusion` |
| 4 | `recall.body_loss` |
| 4 | `recall.source_loss` |
| 3 | `recall.footnote_loss` |
| 11 | heading and furniture defects (see JSON) |

Independent checks corroborate each other, which is the point of using
invariants rather than labels. Article 011's missing-note set `{3, 18, 75-76,
143-144}` is derived from definition numbering, while `footnote.unconverted_marker`
independently flags notes 75 and 76 as bare numbers opening body paragraphs;
neither check knows about the other.

The verifier also reproduces every defect Phase 3 found by hand — article 042's
`Massachusetts v. Feeney,` false heading, article 037's unattached notes 126,
130 and 161, article 011's fused paragraph — and measures each automatically.
Article 011's fusion is worse than the manual audit recorded: at 791 words it
splices a footnote definition into body prose mid-word, joining
`...[hereinafter DUAL` to `governance ensures not only that...`.

## Root cause of the content loss: one `continue`

The loss is not extraction and not classification. Measured against the
per-line `classifications/` CSVs:

| Stage | 047 | 011 | 037 |
|---|---:|---:|---:|
| PDF → extracted lines | 98.6% | 98.2% | 99.9% |
| extracted → Markdown | **67.9%** | **74.6%** | 97.1% |

Lines are extracted correctly and classified correctly — only 27 of 1163 lines
in article 047 are `hidden` — and then the generator drops them. Survival by
final role in 047: `heading` 100%, `marginalia` 88%, `paragraph` **61%**.

The mechanism is `starts_with_footnote_separator` at
`src/liquid/markdown.rs:172`. When a block *begins* with a run of 24 or more
dash-like characters, the whole block is skipped. Blocks that begin with a PDF
footnote separator and then continue into body prose are discarded entirely.

Every fully-dropped paragraph block in the corpus begins with a separator, and
none is dropped for any other reason:

| Article | Blocks dropped (all separator-led) | Tokens lost |
|---|---:|---:|
| 047 | 13 | 2,658 |
| 011 | 12 | 1,778 |
| 042 | 17 | 1,837 |
| 057 | 46 | 5,190 |
| 037 | 0 | 0 |

That is 11,463 of the 20,631 tokens lost corpus-wide. Article 037 scores 97%
for the simple reason that it contains no separator-led blocks — its layout,
not its classification quality, is what distinguishes it.

Article 047 block 23 is representative: a separator line followed by sixteen
lines of body prose, all discarded, including the passage "In keeping with the
analysis above, we will describe the relevant protected characteristic here as
'racial Jewishness' or 'Jewish ancestry.'"

This rule was introduced deliberately as Phase 3 generator fix #3 to remove
duplicated standalone fragments, and it was validated by counting what it
removed (135 fragments) without measuring what else it destroyed. The unit test
at `src/liquid/markdown.rs:1555` covers only separator-plus-short-fragment; no
test covers separator-plus-paragraph. The correct behaviour is to strip the
separator run and keep the remainder, omitting the block only when nothing
substantive follows.

Fixing this alone should raise mean source recall from 78.9% to roughly 93%.
Recovered text still has to be assembled with correct paragraph boundaries, so
the other defect classes will need re-measuring afterwards rather than assumed
resolved.

## Paragraph boundaries: a threshold set for one article

With the model loaded, footnote linking is strong (landing 0.93–1.00) and the
dominant remaining defect is fused paragraphs — article 011 emitted a single
2,646-word "paragraph".

LM2 never calls the paragraph module: `split_long_paragraph` and
`expand_dense_paragraph` in `liquid/paragraphs.rs` belong to the older Liquid
path, and nothing in `liquid2.rs` references them. Boundaries come only from
`paragraph_boundary` (`liquid2.rs:8797`), which on the same page requires a
vertical gap or a left-edge shift greater than 0.055 of page width.

Law reviews mark paragraphs with a first-line indent and no extra leading.
Measured across the ten articles in both sets, that indent is:

| Article | Indent (fraction of page width) |
|---|---:|
| 011, 042, 047, 057 | 0.015–0.026 |
| h05 | 0.022–0.034 |
| h01, h02 | 0.029 |
| h03 | 0.049–0.053 |
| h04 | 0.032–0.036 |
| **037** | **0.059** |

Only article 037 clears 0.055 — which is the whole reason 037 looked like the
best document in the corpus. Its paragraphs split because its indent happens to
be twice as deep as everyone else's, not because it was classified better.

The cross-page branch of the same function already uses the correct `0.025`
test, so the two halves of one predicate disagreed.

There is also a purpose-built splitter for exactly this case,
`apply_action_neutral_blocksplit`, with tests covering "indented paragraph
after sentence end". It is disabled in the shipped tier:

```rust
action_neutral_blocksplit: !native_catboost_active && lm2_action_neutral_blocksplit_enabled(),
```

So the mechanism that splits paragraphs on a first-line indent is switched off
precisely when the native model is loaded. Fixing `paragraph_boundary` restores
splitting for every tier, but the disabled post-pass is worth revisiting on its
own terms — it may still catch cases assembly does not.

Two further details mattered. The same-page sentence-end test looks at the raw
final character, but a paragraph's last line usually ends on a footnote marker,
leaving a callout sentinel or a digit — so the test failed precisely on the
lines that end paragraphs. And blocks are cached, so neither fix appears until
the cache is cleared.

## The footnote gate measured coverage, not correctness

`liquid/markdown.rs` required `landing_rate >= 0.9` before emitting linked
footnotes, falling back to an unlinked appended section otherwise. Landing rate
counts markers that found a note head, so an unmatched marker costs reach, not
accuracy: it simply does not become a link. The risk of attaching a citation to
the wrong sentence comes from *ambiguous* matches, and from placement failure,
which a separate check already catches.

On held-out article h05 the gate discarded 416 correct links because 51 of 468
markers had no matching note — with exactly one ambiguous match in the whole
document. The gate now requires a conservative landing floor plus a tight
ambiguity ceiling.

## Why this caps the model

Every dominant defect is a boundary or routing decision that the shipped action
space cannot express. The production model emits `Keep | Marginalia |
HideNoise` per line; paragraph start versus continuation, footnote start versus
continuation, and marker-to-definition linkage are all resolved downstream by
hand-written heuristics. Retraining the classifier cannot move any of the
counts above, which is consistent with the overlay list having grown to a
40-term schema string without closing the gap.

## Suggested gate

A document passes when it has zero critical defects. Useful sub-targets, in the
order they should be attacked:

1. `source_recall >= 0.98` on every article — stop losing text before tuning
   anything about how retained text is labelled.
2. `body_recall >= 0.95` — stop routing body prose into the footnote zone.
3. Footnote bijection: zero orphan definitions and zero unresolved references.
4. Zero sequence gaps, then the paragraph and heading classes.

Targets 1 and 2 are prerequisites for the others: an article that has lost a
third of its body cannot have a complete note sequence, so the footnote
counts above are partly downstream of the recall failure and should be
re-measured after it is fixed.
