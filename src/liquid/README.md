# Liquid Module

Liquid Mode is LawPDF's clean, semantic reading view for dense legal and academic PDFs.

## Current Status

Active extracted modules:

- `model.rs`
- `config.rs`
- `cleaning.rs`
- `classification.rs`
- `normalization.rs`
- `cache.rs`
- `paragraphs.rs`
- `profile.rs`
- `util.rs`
- `llm/`

`llm/layout.rs` owns the optional LLM layout application path. `mod.rs` still owns orchestration, title selection, shared reference/title predicates, and most tests. The next extractions should be verified moves, not stub files.

## Public API

Only these items are intended for use from outside the module:

- `LiquidBlock`
- `LiquidBlockRole`
- `LiquidDocument`
- `LiquidEvent`
- `LiquidRequest`
- `spawn_liquid_job`

Everything else is implementation detail.

## Key Invariants

- Do not paraphrase, summarize, or invent source text.
- Text cleaning may repair PDF extraction artifacts such as control characters and common mojibake.
- Title selection should prefer front-page titles over filenames, weak PDF metadata, running headers, and citation boilerplate.
- LLM layout is optional and must preserve source text.
- The local heuristic pipeline must produce a usable result when native text exists.
- Document profile classification is local, cached with the Liquid document, and used as an advisory signal for future profile-specific heuristics.

## Useful Checks

```powershell
$env:CARGO_TARGET_DIR='C:\tmp\lawpdf-target'
cargo test
cargo run -- --smoke-liquid --liquid-smoke-output C:\tmp\lawpdf-liquid-smoke.json <pdfs...>
cargo run -- --profile-dataset --profile-dataset-output C:\tmp\lawpdf-profile-sample.json --profile-dataset-predict
python tools\profile_training.py --manifest C:\tmp\lawpdf-profile-sample.json --examples C:\tmp\profile-examples.json --output C:\tmp\profile-report.json --model-output profile-models\profile-linear.json --nb-model-output profile-models\profile-nb.json --active-learning-dir to-evaluate\profile-active
python tools\liquid_quality_training.py --manifest C:\tmp\lawpdf-profile-sample.json --examples C:\tmp\liquid-quality-examples.json --output C:\tmp\liquid-quality-report.json --tag-model-output profile-models\liquid-quality-tags.json --quality-model-output profile-models\liquid-quality-score.json --risk-queue-dir to-evaluate\liquid-quality-risk --grok-prompt-output to-evaluate\liquid-quality-grok-prompts.jsonl
```

`tools/profile_training.py` can export active-learning queues. It ranks documents
where the current Liquid profile, independent rule profile, and trained local
models disagree, then copies the PDFs plus a review manifest under
`to-evaluate/`.

`tools/liquid_quality_training.py` generates synthetic Liquid review labels from
PDF extraction stats plus current Liquid block roles, warnings, and samples. It
trains small CPU-friendly models for likely failure tags and 1-5 Liquid quality
score, then can export a ranked risk queue. The optional Grok prompt JSONL is
for background model review of the same items; it is not required for local
training.

## Deep Liquid Sidecar

Deep Liquid is an opt-in law-review-only path. It sends canonical source lines
with stable IDs to a local Python sidecar and accepts only source-span plans; the
Rust app assembles display text from original source lines and falls back to the
current Liquid stack on validation failure.

The End-to-End NN role policy is intentionally narrower than the older
specialist taxonomy. The core labels are `title`, `heading`, `body`, `footnote`,
and `noise`. List items are treated as `body`. Tables, metadata,
table-of-contents or index rows, navigation rows, running headers/footers, and
captions are treated as `noise`; they are not separate targets for the unified
network. This keeps capacity focused on the reading-flow decisions that matter
most for law-review Liquid Mode.

Enable the current deterministic sidecar contract:

```powershell
$env:LAWPDF_DEEP_LIQUID='1'
$env:LAWPDF_DEEP_LIQUID_SCRIPT='tools\liquid_deep_infer.py'
$env:LAWPDF_DEEP_LIQUID_MODEL_ID='lawreview-liquidnet-span-baseline-v0'
cargo run
```

Train the default GPU document-sequence checkpoint scaffold. This encodes each
line, runs a second transformer across document windows, and predicts both roles
and block starts:

```powershell
python tools\liquid_deep_train.py `
  --examples-input training-data\layout-role-core\lawpdf-layout-role-examples-v5-unseen.json `
  --gold-labels training-data\layout-role-core\lawpdf-latest-labels-v4-expanded-pp-train.json `
  --gold-labels training-data\layout-role-core\lawpdf-latest-labels-v3-holdout.json `
  --output-dir profile-models\end-to-end-liquidnet-sequence-v1 `
  --boundary-loss-weight 0.35 `
  --dashboard-report `
  --dashboard-stage "document-sequence GPU training" `
  --progress-every-batches 10
```

Fast trainer smoke:

```powershell
python tools\liquid_deep_train.py `
  --examples-input training-data\layout-role-core\lawpdf-layout-role-examples-v5-unseen.json `
  --gold-labels training-data\layout-role-core\lawpdf-latest-labels-v3-holdout.json `
  --output-dir C:\tmp\lawreview-liquidnet-smoke `
  --epochs 1 `
  --architecture document_sequence `
  --hidden 64 `
  --layers 2 `
  --heads 4 `
  --max-lines 32 `
  --line-window-stride 32 `
  --batch-size 2 `
  --sample-limit 512
```

Evaluate a checkpoint on held-out layout labels:

```powershell
python tools\liquid_deep_eval.py `
  --checkpoint profile-models\end-to-end-liquidnet-sequence-v1\lawreview-liquidnet.pt `
  --examples-input training-data\layout-role-core\lawpdf-layout-role-examples-v5-unseen.json `
  --gold-labels training-data\layout-role-core\lawpdf-latest-labels-v3-holdout.json `
  --output profile-models\end-to-end-liquidnet-sequence-v1\evaluation-report.json
```

Run a law-review-scoped qualitative sidecar audit. This samples complete
document prefixes, uses the same source-span grouping path as the Python
sidecar, and records document-level flags such as missing titles, over-noise,
and low-confidence blocks:

```powershell
python tools\liquid_deep_qual_audit.py `
  --checkpoint profile-models\end-to-end-liquidnet-sequence-v1\lawreview-liquidnet.pt `
  --examples-input training-data\layout-role-core\lawpdf-layout-role-examples-v5-unseen.json `
  --output profile-models\end-to-end-liquidnet-sequence-v1\qualitative-audit-lawreview.json `
  --filename-regex "(law[-_ ]?review|law_review_article|l\.-rev|sclr|mercer_vol\d+_iss\d+_article|loyola_chicago_vol\d+_iss\d+_article|creighton_law_review_vol\d+)" `
  --exclude-regex "(announcement|catalog|newsletter|old_oregon|uocat|course|evaluation|ethos|computing_news|chemistry_department|story_indexes|water_resources|notice_of_adopted)"
```

Publish End-to-End experiment metrics to the local progress dashboard:

```powershell
python tools\liquid_deep_status_report.py `
  --model-dir profile-models\end-to-end-liquidnet-sequence-v1 `
  --training-report profile-models\end-to-end-liquidnet-sequence-v1\training-report.json `
  --evaluation-report profile-models\end-to-end-liquidnet-sequence-v1\evaluation-report.json `
  --qualitative-audit profile-models\end-to-end-liquidnet-sequence-v1\qualitative-audit-lawreview.json `
  --examples-input training-data\layout-role-core\lawpdf-layout-role-examples-v5-unseen.json `
  --stage "document-sequence training"
```

See `MODULARIZATION_PLAN.md` for the audit findings and larger follow-up plan.
