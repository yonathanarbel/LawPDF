# Features, Network Design, and Network Performance

This note is for outside LLM review of the LawPDF end-to-end Liquid network work.
It summarizes what we built, what data it used, how the model is structured, and
where performance is still weak. The goal is to get advice on improving the
end-to-end neural path without losing the practical gains from the existing
specialist models and runtime guards.

## Problem

LawPDF converts law-review PDFs into a "Liquid" reading layout. The hard part is
classifying OCR/PDF source lines into layout roles and block boundaries while
preserving source-span fidelity. The end-to-end Liquid network attempts to
replace or augment a stack of specialist classifiers by predicting, per source
line:

- `title`
- `heading`
- `body`
- `footnote`
- `noise`
- a block-boundary/start signal

The model must work on messy law-review PDFs with running headers, table of
contents rows, repository metadata, footnote-heavy pages, title/byline fragments,
and short all-caps section labels.

## Key Files

- Training script: `tools/liquid_deep_train.py`
- Inference script: `tools/liquid_deep_infer.py`
- Evaluation script: `tools/liquid_deep_eval.py`
- Qualitative audit: `tools/liquid_deep_qual_audit.py`
- Teacher/hardcase tooling:
  - `tools/liquid_deep_active_mine.py`
  - `tools/liquid_deep_hardcase_acquire.py`
  - `tools/liquid_deep_teacher_agreement.py`
  - `tools/liquid_deep_consensus_labels.py`
  - `tools/liquid_deep_filter_teacher_labels.py`
- Core examples: `training-data/layout-role-core/lawpdf-layout-role-examples-chandra-expanded-fast-20260604.json`
- Strict holdout labels: `training-data/layout-role-core/lawpdf-latest-labels-v3-holdout.json`
- Current best end-to-end model:
  - `profile-models/end-to-end-liquidnet-fivecore-hightrust-qwen70b-capped-v5-excludeholdoutdocs-rareweights-gpu-20260605/lawreview-liquidnet.pt`
  - `profile-models/end-to-end-liquidnet-fivecore-hightrust-qwen70b-capped-v5-excludeholdoutdocs-rareweights-gpu-20260605/layout-role-report.json`

## Input Features

Each source line is represented by text tokens plus 15 geometry/layout features.

Text:

- Lowercased word-like tokens.
- Stable hash vocabulary, default `vocab_size = 65536`.
- Max `48` tokens per line.
- Padding id `0`; token ids begin at `2`.

Geometry/layout vector, from `geometry_features()`:

- normalized `x0`, `y0`, `x1`, `y1`
- normalized width and height
- font size or font height
- page-relative font ratio
- document-relative font ratio
- bold flag
- italic flag
- centered flag
- below-footnote-divider flag
- page index
- source line index

Roles are canonicalized before training:

- `caption`, `contents`, `header_footer`, `header`, `footer`, `table`, and `metadata` become `noise`
- `list_item` becomes `body`
- native roles kept: `title`, `heading`, `body`, `footnote`, `noise`

## Network Design

The main architecture used in the promoted end-to-end runs is
`document_sequence`, implemented as `LawReviewLiquidSequenceNet`.

Default promoted v5 config:

- architecture: `document_sequence`
- hidden size: `384`
- layers: `6`
- heads: `8`
- dropout: `0.10`
- max tokens per line: `48`
- max lines per document window: `192`
- window stride: `160`
- batch size: `8`
- epochs: `8`
- learning rate: `2e-4`
- seed: `17`
- mixed precision: `false`
- gradient clipping: `1.0`
- boundary loss weight: `0.35`
- role loss weights:
  - `title = 4.0`
  - `heading = 8.0`
  - `noise = 2.0`

`document_sequence` architecture:

1. Token embedding maps hashed tokens to hidden vectors.
2. A token-level Transformer encodes each line's tokens.
3. Non-padding token embeddings are mean-pooled into one line vector.
4. The 15 geometry features are projected to the same hidden dimension and added
   to the line vector.
5. A second Transformer runs across source-line windows, giving each line local
   document context.
6. Two heads predict:
   - role logits for the five roles
   - block-boundary probability

There are also experimental architectures:

- `line`: encodes each line independently with no cross-line context.
- `hierarchical_document`: adds token positions, line positions, page positions,
  source-line positions, attention pooling over tokens, a local Conv1D line
  mixer, and deeper role/boundary heads.

The promoted model still uses `document_sequence`, not the hierarchical variant.

## Training Data

The v5 promoted run used:

- PDF count: `848`
- total line count: `335,190`
- training rows: `264,950`
- validation rows: `37,072`
- excluded holdout docs: `85`
- excluded label keys: `0`

The strict holdout was excluded at the document level using:

- `training-data/layout-role-core/lawpdf-latest-labels-v3-holdout.json`

Teacher-label strategy:

- The best promoted end-to-end model is v5:
  - `end-to-end-liquidnet-fivecore-hightrust-qwen70b-capped-v5-excludeholdoutdocs-rareweights-gpu-20260605`
- It used high-trust Qwen/Llama70 capped free-teacher labels.
- Later target-agreement labels from Qwen/Nemotron/Poolside were generated, but
  the later trained variants did not beat v5 on the strict holdout.

## Performance

### Promoted End-to-End Model: v5

Path:

`profile-models/end-to-end-liquidnet-fivecore-hightrust-qwen70b-capped-v5-excludeholdoutdocs-rareweights-gpu-20260605`

Status:

- `promoted_with_remaining_qualitative_flags`
- strict holdout macro F1 `0.5373`, beating prior v2 `0.5269`

Validation split, `37,072` rows:

- accuracy: `0.8325`
- boundary accuracy: `0.8638`
- macro F1: `0.6609`

Validation per-role F1:

- title: `0.5323`
- heading: `0.3853`
- body: `0.8896`
- footnote: `0.8156`
- noise: `0.6817`

Strict law-review holdout, `487` rows:

- accuracy: `0.6817`
- boundary accuracy: `0.6838`
- macro F1: `0.5373`

Strict holdout per-role F1:

- title: `0.3333`
- heading: `0.3000`
- body: `0.6895`
- footnote: `0.7131`
- noise: `0.6506`

Qualitative audit:

- 40 docs
- 8,569 lines
- 2,214 blocks
- mean confidence: `0.8637`
- visible line fraction: `0.9142`
- noise line fraction: `0.0858`
- flags:
  - missing marginalia: `3`
  - missing title: `1`
  - missing heading: `1`
  - many low-confidence blocks: `15`
  - substantive low-confidence blocks: `5`

### Later Non-Promoted Runs

v15:

- Added 40 cross-teacher target-only agreement labels to proven v5 data.
- Strict holdout macro F1: `0.4919`
- Not promoted; report says the agreement set was not reliable enough despite
  role coverage.

v16:

- Added 70 low-weight cross-teacher target-agreement labels.
- Validation macro F1: `0.6590`
- Strict holdout macro F1: `0.4424`
- Not promoted; v5 remained current best.

v17:

- Intended Qwen/Nemotron/Poolside target-agreement run.
- Status in synced workspace: `starting`
- No completed checkpoint or metrics were present in the local synced copy when
  reviewed.

## Current Observations

- Body and footnote are the strongest end-to-end categories.
- Heading and title remain weak, especially on strict law-review holdout.
- The model is highly sensitive to small teacher-label additions; later
  agreement-labeled runs regressed strict holdout quality even when validation
  metrics looked similar.
- Strict holdout is small (`487` rows), so individual label noise and document
  selection can strongly affect promotion decisions.
- The neural model currently predicts line roles and boundaries, but it does not
  directly optimize downstream Liquid layout quality such as visible section
  structure, marginalia completeness, or source-span grouping quality.

## Open Questions for Review

1. Should the model remain a five-role classifier, or should we restore richer
   intermediate roles and collapse them only at decode time?
2. Should title/heading be handled by a separate calibrated head or specialist
   auxiliary loss instead of relying on one shared role head?
3. Should we train with pairwise/listwise structure losses for block continuity,
   section transitions, and footnote-body boundaries?
4. Should the strict holdout be expanded and stratified by failure type before
   more teacher labels are allowed to influence promotion?
5. Should teacher labels be treated with soft labels or confidence-weighted
   distillation instead of hard labels, especially for target-agreement packs?
6. Should page/window sampling oversample rare but important structures:
   frontmatter, headings, all-caps short section labels, footnote-heavy pages,
   and repository metadata?
7. Would the hierarchical architecture likely help if trained with the same v5
   clean teacher set, or did later experiments confound architecture changes with
   noisy label additions?
8. Should we add OCR/source-quality features, such as line confidence, text
   density, page region class, or repeated-header fingerprints?
9. Should evaluation optimize final Liquid output directly, not just line-role
   macro F1 and boundary accuracy?
10. What ablations should be run first to decide whether the bottleneck is data
    quality, model capacity, class imbalance, or the objective?

## Recommended Next Experiments

- Freeze the v5 data recipe and run controlled architecture ablations:
  - current `document_sequence`
  - `hierarchical_document`
  - separate heading/title auxiliary heads
  - no geometry, text-only, and geometry-only ablations
- Expand strict holdout with stratified examples before accepting new labels.
- Promote only when both strict holdout and qualitative Liquid audits improve.
- Use soft/confidence-weighted teacher targets for new teacher packs.
- Run a targeted title/heading acquisition loop, but keep new labels out of the
  training set until they pass strict agreement and manual spot checks.

