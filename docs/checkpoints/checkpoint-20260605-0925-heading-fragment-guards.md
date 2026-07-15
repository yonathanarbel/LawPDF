# Heading Fragment Guards - 2026-06-05 09:25

Context:
- The early article/title/byline guard improved runtime-gated heading diagnostics on the 20k strict-gold sample:
  - before: F1 0.3386, precision 0.3852, recall 0.3020
  - after: F1 0.3630, precision 0.4849, recall 0.2900
- The remaining top false positives are mostly body/table/header-footer lines, especially:
  - numeric table/page cells like `20081`
  - lowercase body fragments like `and`, `do`, `the plaintiff`
  - running journal headers like `MERCER LAW REVIEW`

Implemented:
- Runtime heading guard for:
  - numeric table-cell fragments: `looks_like_numeric_table_cell_fragment`
  - lowercase short body fragments: `text_is_all_lowercase_alpha_fragment` via `heading_specialist_fragment_should_not_be_heading`
- Matching training tokens in `feature_tokens`:
  - `numeric_table_cell_fragment`
  - `numeric_fragment_not_heading`
  - `lowercase_body_fragment`
  - `lowercase_fragment_heading_shape_conflict`
- Mirrored the runtime guard in `tools\heading_specialist_confusion_diagnostics.py`.
- Added Rust assertions for the concrete audited shapes `20081` and `do`.

Verification:
- `python -m py_compile tools\layout_role_training.py tools\heading_specialist_confusion_diagnostics.py`
- `rustfmt --edition 2024 --check src\layout_roles.rs`

Not run yet:
- Full `cargo check`, because the prior compile stayed active for about 10 minutes under memory pressure.
- Refreshed 20k diagnostic for the fragment guard, because free RAM was under 1 GB while `layout_role_training.py` was still active.

Queued:
- `tools\run_heading_pagecaps_after_resources_free_20260605.ps1`
- Parent PID at launch: `22716`
- It waits until no `layout_role_training.py`, no Grok/header-footer experiment, and free RAM >= 5 GB, then trains:
  - `profile-models\layout-heading-pagecaps-earlymeta-allaccum-20260605-0920-candidate`
  - using the Chandra-expanded examples plus accumulated Chandra label list.
