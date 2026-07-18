# Checkpoint: non-Grok heading feature and queue

Time: 2026-06-05 10:12 America/Chicago

## What changed

- Stopped using Grok for the header/footer specialist. The small canary timed out and wrote 0 labels, consistent with the user concern that Grok may be out of credits.
- Kept the generated Grok task folders as failed/suspended artifacts only:
  - `training-data/grok-teacher/grok-header-footer-specialist-20260605-0918`
  - `training-data/grok-teacher/grok-header-footer-specialist-small-20260605-0945`
- Added a new non-Grok heading feature in both runtime tokenization and Python training:
  - `centered_prose_continuation`
  - `centered_body_clause_not_heading`
- The new feature targets centered, mixed-case prose continuations with punctuation, citation digits, lowercase starts, or line-break hyphens.
- I tested using it as a hard runtime gate and rejected that approach because it hurt F1.

## Diagnostic result

Baseline after fragment guards on 10k strict sample:

- runtime heading: F1 0.3603, P 0.5074, R 0.2794
- final cascade: F1 0.3560, P 0.5037, R 0.2753

Hard centered-prose runtime gate on the same 10k sample:

- runtime heading: F1 0.2994, P 0.5747, R 0.2024
- final cascade: F1 0.2943, P 0.5698, R 0.1984

Decision: do not use centered-prose as a runtime gate. Keep it as a trainable feature only.

## Validation

- `python -m py_compile tools\layout_role_training.py tools\heading_specialist_confusion_diagnostics.py` passed.
- `rustfmt --edition 2024 src\layout_roles.rs --check` passed after formatting.
- `cargo test runtime_layout_features_match_trainer_geometry_tokens` timed out after 5 minutes; I stopped the stale Cargo process.

## Background training queue

Script:

- `tools/run_heading_pagecaps_after_resources_free_20260605.ps1`

Current behavior:

- No Grok dependency.
- No header/footer Grok stacked model.
- Waits for no active trainer and at least 2.5 GB available RAM.
- Uses `System.Diagnostics.PerformanceCounter("Memory", "Available KBytes")` for RAM rather than WMI.

Latest log:

- `C:\tmp\lawpdf-heading-pagecaps-earlymeta-20260605-0920\heading-train.log`
- At 10:09:25 it was alive, with `active_trainers=0 free_kb=1937768`, so it was still waiting for RAM.

Queued model output:

- `profile-models/layout-heading-pagecaps-earlymeta-allaccum-20260605-0920-candidate/layout-role-model.json`
- No report existed yet as of this checkpoint.

