# Can the emission model be pushed further?

Measured 2026-07-24/25. Short answer: **no, not on its own** — and the attempt
produced a more useful result than success would have.

Retraining the LM2 CatBoost emission model raises line-level macro F1 from
**0.687 to 0.905**, a 32% relative gain, and makes held-out document quality
**twice as bad**. Every stage above the emissions is tuned around the shipped
model's deliberate miscalibration, so a better-calibrated model breaks them.

## Before and after

Document quality, `tools/md_verify.py`, five held-out law review articles that
appear in no training corpus:

| | Held-out critical defects | Mean source recall |
|---|---:|---:|
| Start of session | 26 | 94.0% |
| **End of session** | **18** | **94.0%** |

The 31% reduction comes from two reconstruction fixes, not from the model. The
in-sample five moved 58 → 52, but two of those articles are in the training
corpus and are excluded from any model comparison (see Leakage).

## What was tried on the model

Line-level macro F1, split **by document**, 335,190 rows from 848 documents:

| Model | Line macro F1 | Held-out critical |
|---|---:|---:|
| **Shipped `epoch51lv-relabels-tc`** | 0.6872 | **26** |
| Retrained, tuned, full data | **0.9054** | 52 |
| Retrained, class-weighted toward shipped bias | 0.9007 | 40 |
| Retrained, context two-pass disabled | 0.9054 | 33 |
| Retrained, silver labels down-weighted ×0.3 | 0.9049 | 54 |
| Deliberately weak probe (60 iters, 15k rows) | ~0.81 | 49 |

**The relationship is inverse.** The best line-level model is the worst
end-to-end. Four independent retraining strategies all lose to the shipped
model, by margins from 27% to 108%.

The benchmark is not insensitive: the weak probe degrades held-out quality from
26 to 49, so emission quality does propagate. It propagates in the wrong
direction for a better-calibrated model.

### Why

The runtime is a stack: CatBoost emissions → a context two-pass model → a
sequential decoder → ~30 named overlays → a −6.0 footnote bias. The shipped
model's raw output is deliberately skewed — hide_noise precision 0.40,
marginalia precision 0.68, both at high recall — and everything above it exists
to pull that back. Replacing the emissions with well-calibrated ones removes
the skew those stages correct for.

Two results support this and no other reading. Disabling the context two-pass
model, which is trained on the old emissions, recovers 52 → 33. Class-weighting
the new model back toward the old bias recovers 52 → 40. Both move as predicted
without closing the gap.

**Implication: the emission model is not the lever.** More or better training
data — which is what the OCR corpus would supply — would raise line-level F1
further, and there is no evidence it would improve the product. Squeezing the
model requires re-tuning the decoder, overlays and bias jointly with it, which
is a different and much larger piece of work than swapping a `.cbm`.

### Hyperparameter search

Optuna TPE over depth, learning rate, L2, border count, random strength,
bootstrap type and its parameter. Stopped after ~1 h when it plateaued: best
validation macro F1 moved from 0.9202 (shipped hyperparameters) to 0.9216
(`depth 7, lr 0.038, l2 1.49, border 128, random_strength 3.72, Bayesian,
bagging_temperature 0.465`). **Nearly all of the line-level gain comes from
retraining on the current corpus, not from hyperparameters.**

### LiquidVision contributes nothing measurable

Turning it off leaves held-out defects unchanged at 18 and recall at 94.0%. It
carries 4.2% of the shipped model's feature-importance mass. This is worth
knowing independently: it is a per-page render plus inference on every
document, for no measured quality.

## The two fixes that did help

Both are reconstruction, both measured on the held-out five, recall unchanged.

**Citation fragments were becoming headings** (26 → 20). Law reviews italicise
case names, and a stray italic fragment upstream was read as a heading, so
outlines contained `## Raich,`, `## Sebelius,`, `## Alito in Reed v. Goertz,`.
A heading never trails off in a comma, and a short line containing a party
separator is a citation. These now render as prose.

**Numbered lists inside footnotes were starting new notes** (20 → 18). In one
article, "1. If X is clearly illegal" and "2. Elseif X is maybe-illegal" —
pseudocode inside footnote 194 — were parsed as notes 1 and 2, splitting the
note and breaking the sequence. Footnote numbers run upward through an article,
so a large backward step is not a note head.

A third guard, requiring an indented line to open a sentence before splitting a
paragraph, was implemented, measured at **exactly zero effect**, and reverted.
Carrying guards that are validated only by the reasoning behind them is what
produced the separator bug documented in `MD_RECONSTRUCTION_BASELINE.md`.

## A second, harder held-out set — and what it says about the first

The five held-out articles are modern, born-digital law reviews. To test
whether the conclusions generalise, I drew 12 more from
`lrscraper_backup/chicago_law_review`, excluding the 6 Chicago articles that
appear in the training corpus, seeded and unseen before measurement.

| Set | Docs | Critical defects | Per doc | Mean recall | Worst recall |
|---|---:|---:|---:|---:|---:|
| Original held-out (modern) | 5 | 18 | 3.6 | 94.0% | 91.1% |
| **Chicago (mixed era)** | 12 | 43 | 3.6 | **85.7%** | **44.5%** |

Defects per document match, but **content recall is 8 points worse and one
article loses 56% of its text**. The first held-out set was easier than the
corpus the product actually faces.

**Today's two fixes have exactly zero effect here** — 43 critical defects and
85.7% recall both before and after. They address failure modes present in the
modern set (italic case names, pseudocode in footnotes) and absent from this
one. A 31% improvement measured on five documents did not generalise at all to
twelve others. That is worth remembering before trusting any single small
benchmark, including this one.

### The dominant failure on older volumes: body markers are not found

Seven of the twelve articles fall below the footnote-linking threshold and emit
**no inline footnotes at all**, falling back to an unlinked notes section:

| Article | Note heads found | Markers detected | Landing |
|---|---:|---:|---:|
| c09 | 63 | 16 | 0.562 |
| c12 | 54 | 12 | 0.583 |
| c10 | 43 | 25 | 0.800 |
| c08 | 59 | 36 | 0.750 |
| c02 | 116 | 182 | 0.500 |
| c06 (healthy, for contrast) | 276 | 264 | 1.000 |

The footnote *text* is recovered — c09's note 22 appears in full — but with no
markers in the body there is nothing to link it to. This matters directly for
the 330k-document corpus, which is largely this kind of scanned or
OCR-rebuilt volume.

The obvious hypothesis, that font-size-only superscript detection misses
markers which are raised but not smaller, is **not** the explanation. Measured
in the PDF, c09 has 70 raised digits of which 15 exceed the 0.80×-body size
threshold, so I carried the per-glyph baseline into `CharMeta` and accepted
either signal. Result: landing moved 0.555→0.600 and 0.800→0.815 on two
articles, 0.500→0.467 on another, and c09 did not move at all; defects and
recall were unchanged at 43 and 85.7%. The change was reverted.

So I instrumented the extractor instead of inferring from a second library.
`lawpdf --dump-char-metrics --page N file.pdf` prints what pdfium actually
reports per glyph. On page 3 of each article:

| | Distinct font sizes | Superscript digits report |
|---|---:|---|
| c06 (links 1.000) | 5 | **6.00** against body 11.04 — clearly separable |
| c09 (links 0.562) | 9 | **10.00**, the same as body, sharing the line's baseline |

**In c09 pdfium reports no size and no baseline difference for superscripts at
all.** The signal the detector needs is absent from the text layer as pdfium
presents it, which is why neither the size threshold nor the baseline test can
work, and why adding the baseline test changed nothing.

PyMuPDF, reading the same file, *does* report size-7 raised glyphs. So the
information exists in the PDF and is lost in this extraction path. That makes
the next step concrete and narrow: find out why pdfium collapses superscript
metrics on these volumes — character-level API choice, font-matrix handling, or
`loose_bounds` versus `tight_bounds`. Swapping to `tight_bounds` alone does not
do it: positions shift by ~2pt and the digits still report body size.

This matters out of proportion to its size. Older and OCR-rebuilt volumes are
most of a 330k-document law review corpus, and on them the product currently
recovers footnote text but cannot link any of it.

## Leakage

Model comparisons use only the held-out five. Matching sampled source lines
against the training corpus:

| Article | Sampled lines found in training data |
|---|---:|
| 047-antisemitism-title-vi (in-sample) | 102 / 120 |
| 057-contract-wrapped-property (in-sample) | 104 / 120 |
| 011, 037, 042 (in-sample) | 0–3 / 120 |
| h01, h02, h04, h05 (held-out) | 0 / 120 |
| h03 (held-out) | 11 / 120 |

Two of the five in-sample articles are in the retrained model's training set,
which is why in-sample defects appeared to *improve* (57 → 33) for the
retrained model while held-out doubled.

## Reproducing

```
lawpdf --dump-lm2-training --output train.jsonl        # 335k rows, 16 s
python tools/lm2_train.py --data train.jsonl \
    --baseline profile-models/lm2-native-catboost-runtime/*.cbm --trials 40
```

To evaluate a candidate end-to-end, set `LAWPDF_LM2_NATIVE_CATBOOST_MODEL`,
clear `%APPDATA%/LawPDF/liquid2-cache`, run `--lm2-copy-md`, then
`tools/md_verify.py`. **Never run two benchmarks concurrently**: they share
that cache directory and will silently corrupt each other's results.

## Caveats

The model conclusion rests on five held-out documents. The direction is large
and consistent across six configurations, so I am confident in it, but the
exact defect counts carry real uncertainty and the model comparisons were not
repeated on the 12-article Chicago set.

The 330k-document OCR corpus was not reachable from this machine, so
"augmenting training data" was tested only by proxy, through label-provenance
weighting on the existing 335k-line corpus. Given that better line-level
accuracy made the product worse in every configuration tried, more data is
unlikely to be the constraint until the stack above the emissions is retuned.

One methodological note, learned the hard way: two benchmark runs launched
concurrently share `%APPDATA%/LawPDF/liquid2-cache` and each clears it, which
silently produced a plausible-looking but wrong result (79/53 instead of
52/18). Benchmarks must run one at a time.

## What I would do next

1. **Instrument marker detection inside the extraction path.** Seven of twelve
   older articles lose all footnote linking; the cause is not what the obvious
   hypothesis predicted, and it is the single largest quality gap measured.
2. **Leave the emission model alone** unless the decoder, overlays and footnote
   bias are retuned jointly with it.
3. **Treat the 12-article Chicago set as the primary benchmark**, not the
   modern five — it is closer to the corpus the product faces, and it caught a
   generalisation failure the smaller set could not see.
