# LawPDF End-to-End Liquid Network Design Review Packet

This file is intended for LLMs or engineers reviewing the LawPDF end-to-end
Liquid network. It includes enough context to reason about architecture, feature
design, data quality, training strategy, evaluation, and likely next
experiments.

The current practical objective is not to win a generic document-layout
benchmark. The objective is to improve LawPDF's Liquid reading mode for
law-review PDFs while preserving source-span fidelity: every visible Liquid block
should map cleanly back to original source lines.

## 1. Executive Summary

LawPDF has two competing/related layout systems:

- A production-oriented stack of specialist/layout classifiers plus runtime
  rules and guards.
- An end-to-end neural path, `LawReviewLiquidNet`, that predicts line roles and
  block boundaries directly from source-line text and geometry.

The best completed end-to-end run so far is:

`profile-models/end-to-end-liquidnet-fivecore-hightrust-qwen70b-capped-v5-excludeholdoutdocs-rareweights-gpu-20260605`

Its checkpoint is:

`lawreview-liquidnet.pt`

Status:

- `promoted_with_remaining_qualitative_flags`
- strict holdout macro F1: `0.5373`
- validation macro F1: `0.6609`
- strong categories: `body`, `footnote`
- weak categories: `heading`, `title`

Later v15/v16 target-agreement teacher-label experiments did not beat v5 on the
strict law-review holdout, even though their broad validation metrics remained
similar. The main open question is whether the bottleneck is model architecture,
label quality, evaluation design, class imbalance, or the objective.

## 2. Task Definition

For each source line from a PDF, the model predicts:

- `title`
- `heading`
- `body`
- `footnote`
- `noise`
- block-start / boundary probability

The downstream Liquid renderer turns these line-level predictions into reading
blocks:

- titles and headings remain visible structure
- body lines become main reading flow
- footnotes become marginalia
- noise is discarded or suppressed
- boundary predictions influence grouping and paragraph/block continuity

The model must handle:

- law-review article front matter
- titles and subtitles split over multiple lines
- author/byline blocks
- table of contents rows and dot leaders
- running headers and footers
- repository metadata
- footnote-heavy pages
- footnote divider cues
- line fragments created by PDF extraction/OCR
- short all-caps section labels
- roman numeral and outline-style headings
- captions/tables and non-reading artifacts

## 3. Key Source Files

Deep Liquid training/evaluation:

- `tools/liquid_deep_train.py`
- `tools/liquid_deep_infer.py`
- `tools/liquid_deep_eval.py`
- `tools/liquid_deep_qual_audit.py`
- `tools/liquid_deep_status_report.py`

Teacher and hardcase tooling:

- `tools/liquid_deep_active_mine.py`
- `tools/liquid_deep_hardcase_mine.py`
- `tools/liquid_deep_hardcase_pack.py`
- `tools/liquid_deep_hardcase_acquire.py`
- `tools/liquid_deep_teacher_agreement.py`
- `tools/liquid_deep_consensus_labels.py`
- `tools/liquid_deep_filter_teacher_labels.py`
- `tools/liquid_deep_label_source_audit.py`
- `tools/liquid_deep_proxy_dev_split.py`
- `tools/liquid_deep_teacher_benchmark.py`

Specialist/layout training:

- `tools/layout_role_training.py`
- `tools/layout_runtime_ensemble_eval.py`
- `tools/heading_specialist_confusion_diagnostics.py`

Core runtime code:

- `src/layout_roles.rs`
- `src/liquid/deep.rs`
- `src/liquid/classification.rs`
- `src/liquid/paragraphs.rs`

Important data/report paths:

- `training-data/layout-role-core/lawpdf-layout-role-examples-chandra-expanded-fast-20260604.json`
- `training-data/layout-role-core/lawpdf-latest-labels-v3-holdout.json`
- `profile-models/end-to-end-liquidnet-fivecore-hightrust-qwen70b-capped-v5-excludeholdoutdocs-rareweights-gpu-20260605/layout-role-report.json`
- `profile-models/end-to-end-liquidnet-fivecore-hightrust-qwen70b-capped-v5-excludeholdoutdocs-rareweights-gpu-20260605/training-report.json`
- `profile-models/end-to-end-liquidnet-fivecore-hightrust-qwen70b-capped-v5-excludeholdoutdocs-rareweights-gpu-20260605/evaluation-report.json`

## 4. Feature Design

The model consumes one source line at a time, but the main architecture
contextualizes a window of neighboring source lines.

### 4.1 Text Features

Tokenization in `tools/liquid_deep_train.py`:

- Lowercase text.
- Split on non-word separators while allowing alphanumeric, apostrophe, and
  hyphen characters inside tokens.
- Hash tokens with a stable FNV-style hash.
- Vocabulary size defaults to `65,536`.
- Token IDs use `0` for padding; real tokens map into `[2, vocab_size)`.
- Maximum tokens per line defaults to `48`.

Consequences:

- No pretrained embeddings are used.
- Vocabulary is fixed and deterministic.
- OOV is not a concept; all tokens hash into the same fixed bucket space.
- Collisions are possible.
- The network must learn legal/layout vocabulary from scratch.

Potential review question:

- Would a compact pretrained language model, character/byte model, or mixed
  hashed n-gram feature improve heading/title discrimination without overfitting?

### 4.2 Geometry Features

`geometry_features(row)` returns 15 floats:

1. `x0 / page_width`
2. `y0 / page_height`
3. `x1 / page_width`
4. `y1 / page_height`
5. normalized line width
6. normalized line height
7. `font_size` or `font_height`
8. `font_ratio_page`
9. `font_ratio_doc`
10. `bold` flag
11. `italic` flag
12. `centered` flag
13. `below_footnote_divider` flag
14. `page_index`
15. `line_index`

The model gets both normalized geometry and raw-ish page/line indices. In
`hierarchical_document`, page and source-line indices are also embedded as
positions.

Potential review questions:

- Should page and line indices be normalized/clipped more carefully?
- Should repeated running-header/footer fingerprints be explicit features?
- Should we add page-region bins: top margin, body area, bottom margin,
  footnote-zone, gutter, side marginalia?
- Should we add extraction confidence, source type, or OCR/PDF backend quality?

### 4.3 Role Canonicalization

The Deep Liquid model currently collapses richer layout roles into five classes:

- `caption`, `contents`, `header_footer`, `header`, `footer`, `table`,
  `metadata` -> `noise`
- `list_item` -> `body`
- kept as-is: `title`, `heading`, `body`, `footnote`, `noise`

This is pragmatic for Liquid output, but it may make learning harder:

- `contents` and `header_footer` are both `noise` but have different visual/text
  cues.
- `list_item` and prose body are both `body` but can look heading-like.
- `metadata` can look like title/front matter.
- `caption/table` noise has different geometry than running headers.

Potential review question:

- Should the network predict richer roles and collapse only at decode time?

## 5. Network Architectures

All architectures are in `tools/liquid_deep_train.py`.

### 5.1 `line`: `LawReviewLiquidNet`

This is the simplest architecture.

Flow:

1. Embed hashed tokens.
2. Token Transformer over one line.
3. Mean-pool non-padding token states.
4. Add projected geometry vector.
5. LayerNorm.
6. Linear role head.
7. Linear boundary head.

Limitation:

- No line-to-line context, so headings, footnote continuations, headers/footers,
  and paragraph boundaries must be inferred locally.

### 5.2 `document_sequence`: `LawReviewLiquidSequenceNet`

This is the promoted v5 architecture.

Flow:

1. Flatten batch windows into individual lines.
2. Embed tokens.
3. Token-level Transformer encodes each line independently.
4. Mean-pool visible token states into one vector per line.
5. Add projected geometry vector.
6. Run a second Transformer over a window of source-line vectors.
7. LayerNorm.
8. Linear role head.
9. Linear boundary head.

Default v5 config:

```json
{
  "architecture": "document_sequence",
  "roles": ["title", "heading", "body", "footnote", "noise"],
  "vocab_size": 65536,
  "max_tokens": 48,
  "max_lines": 192,
  "line_window_stride": 160,
  "hidden": 384,
  "layers": 6,
  "heads": 8,
  "dropout": 0.1,
  "page_position_vocab": 512,
  "source_line_position_vocab": 4096,
  "batch_size": 8,
  "grad_accum_steps": 1,
  "epochs": 8,
  "lr": 0.0002,
  "seed": 17,
  "amp": false,
  "grad_clip_norm": 1.0,
  "boundary_loss_weight": 0.35,
  "role_loss_weights": {
    "title": 4.0,
    "heading": 8.0,
    "noise": 2.0
  },
  "window_role_weights": {}
}
```

Layer split:

- Token encoder uses `max(1, layers // 2)` layers.
- Line encoder uses `max(1, layers - token_layers)` layers.
- For `layers = 6`, this means 3 token layers and 3 line-context layers.

### 5.3 `hierarchical_document`: `LawReviewLiquidHierarchicalNet`

This is an experimental larger architecture.

Additional mechanisms:

- token position embeddings
- line position embeddings
- page position embeddings
- source-line position embeddings
- deeper geometry projection
- attention pooling over token states instead of mean pooling
- local Conv1D line mixer before the line Transformer
- deeper MLP role head
- deeper MLP boundary head
- `norm_first=True` Transformer layers

Potential interpretation:

- It is better aligned with document layout structure than
  `document_sequence`, but available experiment results confound architecture
  changes with changing/noisy label recipes. A clean v5-data architecture
  ablation is still needed.

## 6. Windowing and Sampling

For sequence architectures:

- Max window length: `192` source lines.
- Training windows can overlap.
- Default stride: `160`.
- Holdout split is document-level, not random line-level.

There is optional weighted sampling:

- `--window-role-weight ROLE=FLOAT`
- It oversamples windows containing rare roles.
- v5 did not use window role weights (`{}`).

Review questions:

- Should windows containing `title`, `heading`, footnote starts, frontmatter,
  and repository metadata be oversampled?
- Should windows be page-aware or section-aware rather than fixed-line windows?
- Should very long documents use hierarchical page/document batching?

## 7. Objective and Losses

The training loss combines:

- weighted per-line role cross entropy
- weighted block-boundary binary cross entropy

Formula in code:

`loss = ((role_loss * role_weights + boundary_loss_weight * boundary_loss) * row_weight).mean()`

v5 used:

- `boundary_loss_weight = 0.35`
- title role loss weight `4.0`
- heading role loss weight `8.0`
- noise role loss weight `2.0`
- row weights from labels/examples, clamped to minimum `0.05`
- AdamW optimizer
- gradient clipping at `1.0`

Boundary labels are derived using `is_block_start(previous, row)`:

- new document -> block start
- new page -> block start unless current role is `body` or `footnote`
- role change -> block start
- `body` and `footnote` can continue across adjacent lines
- other roles generally start new blocks

Review questions:

- Is the derived boundary target too heuristic?
- Should block continuity be trained with pairwise or span-level losses?
- Should role and boundary be jointly decoded with constraints?
- Should downstream Liquid layout quality be a training or validation objective?

## 8. Training Data

Main examples for v5/v15/v16:

- `training-data/layout-role-core/lawpdf-layout-role-examples-chandra-expanded-fast-20260604.json`

Dataset summary from reports:

- PDF count: `848`
- total line count: `335,190`
- training rows: `264,950`
- validation rows: `37,072`
- excluded holdout docs: `85`
- excluded label keys: `0`

Strict holdout label file:

- `training-data/layout-role-core/lawpdf-latest-labels-v3-holdout.json`

Holdout exclusion:

- v5/v15/v16 used `--exclude-label-docs`, removing all rows from documents
  containing strict holdout labels before train/validation split.

Label sources include:

- existing layout role examples
- gold/silver labels
- PP-DocLayout-derived labels
- Grok/Qwen/Llama/Nemotron/Poolside teacher labels
- hardcase packs
- agreement-only teacher labels
- Chandra/vLLM teacher labels for hard structural cases

Important caution:

- Agreement-only teacher labels are not the same as hand-reviewed gold.
- Later target-agreement packs caused strict holdout regressions.

## 9. Performance Summary

### 9.1 Promoted v5

Path:

`profile-models/end-to-end-liquidnet-fivecore-hightrust-qwen70b-capped-v5-excludeholdoutdocs-rareweights-gpu-20260605`

Status:

- `promoted_with_remaining_qualitative_flags`
- message: strict holdout macro F1 `0.5373` beat prior v2 `0.5269`

Training:

- elapsed seconds: `552.411`
- epochs: `8`
- train rows: `264,950`
- validation rows: `37,072`

Validation split:

| Metric | Value |
| --- | ---: |
| accuracy | 0.8325 |
| boundary accuracy | 0.8638 |
| macro F1 | 0.6609 |

Validation per-role:

| Role | Support | Precision | Recall | F1 |
| --- | ---: | ---: | ---: | ---: |
| title | 254 | 0.5147 | 0.5512 | 0.5323 |
| heading | 999 | 0.3273 | 0.4685 | 0.3853 |
| body | 24,512 | 0.8814 | 0.8980 | 0.8896 |
| footnote | 6,937 | 0.9066 | 0.7412 | 0.8156 |
| noise | 4,370 | 0.6563 | 0.7092 | 0.6817 |

Strict law-review holdout:

| Metric | Value |
| --- | ---: |
| rows | 487 |
| accuracy | 0.6817 |
| boundary accuracy | 0.6838 |
| macro F1 | 0.5373 |

Strict holdout per-role:

| Role | Support | Precision | Recall | F1 |
| --- | ---: | ---: | ---: | ---: |
| title | 5 | 1.0000 | 0.2000 | 0.3333 |
| heading | 8 | 0.2500 | 0.3750 | 0.3000 |
| body | 146 | 0.5361 | 0.9658 | 0.6895 |
| footnote | 234 | 0.9568 | 0.5684 | 0.7131 |
| noise | 94 | 0.7500 | 0.5745 | 0.6506 |

Qualitative audit:

- docs: `40`
- lines: `8,569`
- blocks: `2,214`
- mean confidence: `0.8637`
- visible line fraction: `0.9142`
- noise line fraction: `0.0858`

Flags:

| Flag | Count |
| --- | ---: |
| missing marginalia | 3 |
| missing title | 1 |
| missing heading | 1 |
| many low-confidence blocks | 15 |
| substantive low-confidence blocks | 5 |

Low-confidence categories:

| Category | Count |
| --- | ---: |
| short_marker | 19 |
| substantive | 54 |
| short_text | 57 |
| repository_metadata | 4 |
| short_caps | 2 |
| toc_or_dotleader | 2 |

### 9.2 v15: Target-Agreement Labels, Not Promoted

Path:

`profile-models/end-to-end-liquidnet-fivecore-targetagreement-v15-v5data-excludeholdoutdocs-rareweights-gpu-20260605`

Status:

- `not_promoted`
- Added 40 cross-teacher target-only agreement labels to proven v5 data.
- Report says the agreement set was not reliable enough for training despite
  role coverage.

Validation:

- accuracy: `0.8328`
- boundary accuracy: `0.8627`
- macro F1: `0.6659`

Strict holdout:

- accuracy: `0.6694`
- boundary accuracy: `0.6694`
- macro F1: `0.4919`

Strict per-role F1:

- title: `0.3333`
- heading: `0.1250`
- body: `0.6779`
- footnote: `0.7147`
- noise: `0.6087`

Interpretation:

- Broad validation macro F1 looked slightly higher than v5, but strict holdout
  worsened materially.
- This suggests either label contamination/noise, holdout brittleness, or a
  mismatch between broad validation and target Liquid quality.

### 9.3 v16: Low-Weight Target-Agreement Labels, Not Promoted

Path:

`profile-models/end-to-end-liquidnet-fivecore-targetagreement-lowweight-v16-v5data-excludeholdoutdocs-rareweights-gpu-20260605`

Status:

- `not_promoted`
- Added 70 low-weight cross-teacher target-agreement labels.
- Strict holdout macro F1 `0.4424`, below v5 `0.5373`.

Validation:

- accuracy: `0.8333`
- boundary accuracy: `0.8634`
- macro F1: `0.6590`

Strict holdout:

- accuracy: `0.6982`
- boundary accuracy: `0.6735`
- macro F1: `0.4424`

Strict per-role F1:

- title: `0.0000`
- heading: `0.1250`
- body: `0.6952`
- footnote: `0.7588`
- noise: `0.6329`

Interpretation:

- Body and footnote stayed competitive.
- Title/heading collapsed on strict holdout.
- The lower weighted target-agreement labels still hurt promotion criteria.

### 9.4 v17: Incomplete/Synced Starting State

Path:

`profile-models/end-to-end-liquidnet-fivecore-targetagreement-qwen-nemotron-poolside-v17-v5data-excludeholdoutdocs-rareweights-gpu-20260606`

Status in this synced workspace:

- `starting`
- no completed checkpoint or training report observed
- progress log only showed early epoch 1 activity when inspected

Interpretation:

- Do not treat v17 as completed evidence unless a full checkpoint and reports are
  present on the original training machine/server.

## 10. Failure Modes Seen So Far

Most important model weaknesses:

- heading detection remains weak
- title recall is weak on strict holdout
- short all-caps headings are confused with running headers/noise
- contents rows and headings can be confused
- repository metadata can be misread as visible content
- marginalia/footnotes can be missing in qualitative audit
- later teacher-label additions can regress strict holdout despite decent broad
  validation numbers

Most important evaluation weaknesses:

- strict holdout is small: `487` rows
- title support is only `5` rows
- heading support is only `8` rows
- a few labels can swing macro F1 substantially
- strict labels may contain some noise, as seen in heading-specialist work
- broad validation split is much larger but may not reflect the exact Liquid
  user experience

## 11. Relationship To Specialist Models

The end-to-end network is not the only layout system. Specialist models and
runtime gates have made real progress, especially for headings and footnotes.

Examples:

- footnote specialist v68 was promoted from Chandra-filtered teacher data
- heading specialist cycle058 was promoted with runtime gates
- later heading cycles 059/060 exist but were not necessarily promoted
- runtime heading guards handle common-section labels and `ABSTRACT` edge cases

The end-to-end network should be judged against this practical stack, not only
against isolated line-role metrics.

Review question:

- Should the end-to-end network replace specialists, provide features to them, or
  operate as a calibrated ensemble member?

## 12. Important Experimental Caveats

1. Architecture and data changes have often been mixed in the same run.
2. v15/v16 are not clean architecture ablations; they changed label recipes.
3. Teacher-agreement labels are not guaranteed true labels.
4. The validation split can look stable while strict holdout regresses.
5. Strict holdout is too small for some rare roles.
6. Some strict labels may themselves be noisy.
7. The network objective optimizes line role/boundary metrics, not final Liquid
   layout quality.

## 13. Suggested Review Questions

Ask reviewers to answer these directly:

1. Is a five-role target space too collapsed for learning?
2. Should the network predict richer roles and collapse them only at decode time?
3. Should headings/titles have separate auxiliary heads or a specialist branch?
4. Should footnotes/body boundaries be trained with span/pair losses rather than
   independent line losses?
5. Is a hashed word vocabulary enough, or should we use character/subword
   features or a small pretrained encoder?
6. Should we include explicit page-region and repeated-header/footer features?
7. Should new teacher labels be soft/confidence-weighted distillation targets
   instead of hard labels?
8. Should validation be stratified by failure mode and document family?
9. Should model selection use qualitative Liquid audits as a gate?
10. Should `hierarchical_document` be retrained on exactly the v5 data recipe
    before judging it?

## 14. Suggested Next Experiments

Priority 1: controlled ablations with frozen v5 data.

- v5 `document_sequence` rerun as baseline
- `hierarchical_document` with identical data
- richer-role target space with decode-time collapse
- text-only ablation
- geometry-only ablation
- no boundary loss ablation
- title/heading auxiliary head ablation
- window oversampling for title/heading/frontmatter

Priority 2: improve evaluation.

- Expand strict holdout with stratified labels:
  - titles/frontmatter
  - section headings
  - short all-caps labels
  - contents rows
  - running headers/footers
  - repository metadata
  - footnote-heavy pages
- Track both line metrics and final Liquid block metrics.
- Keep a small manually reviewed promotion set separate from teacher-label data.

Priority 3: teacher-label discipline.

- Do not add agreement-only teacher labels directly to the main recipe.
- First evaluate agreement packs against a small manually checked sample.
- Use soft labels or lower weights for teacher-only labels.
- Separate teacher source/domain in metadata.
- Run label-source ablations.

Priority 4: decoding and ensemble.

- Use the neural network as one calibrated signal in the specialist runtime
  stack.
- Add role-specific confidence thresholds.
- Apply structural constraints during decoding:
  - titles usually early
  - headings usually sparse
  - footnotes cluster below divider or in note zones
  - running headers repeat
  - metadata/repository blocks have predictable phrases

## 15. Minimal Reproduction Checklist

To reproduce a v5-like run, reviewers need:

- `tools/liquid_deep_train.py`
- the examples JSON:
  - `training-data/layout-role-core/lawpdf-layout-role-examples-chandra-expanded-fast-20260604.json`
- strict holdout labels:
  - `training-data/layout-role-core/lawpdf-latest-labels-v3-holdout.json`
- the gold/teacher label files used in the v5 recipe
- config:
  - architecture `document_sequence`
  - hidden `384`
  - layers `6`
  - heads `8`
  - max lines `192`
  - stride `160`
  - max tokens `48`
  - batch size `8`
  - epochs `8`
  - lr `2e-4`
  - role weights `title=4`, `heading=8`, `noise=2`
  - boundary loss weight `0.35`
  - exclude strict holdout docs

The exact v5 launch command should be reconstructed from checkpoints/logs if
possible. The reports preserve config and metrics but not every CLI label path.

## 16. Current Practical Recommendation

Treat v5 as the current end-to-end baseline. Do not promote v15/v16 based on the
available reports. Do not treat v17 as completed unless a finished checkpoint and
reports are found.

The most useful next outside advice would be:

- how to redesign the target labels and losses for headings/titles
- how to use teacher labels without regressing strict holdout
- whether the hierarchical architecture is worth a clean retry
- how to evaluate final Liquid output directly
- how to combine the end-to-end network with the specialist stack

