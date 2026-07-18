# Heading Specialist Checkpoint - 2026-06-05 19:16 CDT

## Runtime/release changes packaged

- Added `sentence_case_body_continuation_should_not_be_heading` to suppress sentence-like body fragments that visually imitate headings.
  - Example target: `Division of the Justice Department reflect this sentiment.42 The`
  - Training/diagnostic feature tokens:
    - `sentence_case_body_continuation`
    - `heading_shape_sentence_fragment_conflict`
- Widened law-review masthead guard for first-page mastheads such as `GEORGETOWN LAW JOURNAL`.
- Added all-caps topic heading as a narrow heading-shape signal for normal-font, non-citation all-caps headings.
  - Example target: `MAPS AND CHARTS`
- Current release copied to:
  - `target/release/lawpdf.exe`
  - `target/release/lawpdf-latest.exe`
  - `target/release/lawpdf-latest-headingguards-20260605-191209.exe`
- Embedded version markers include:
  - `sentence-fragment-heading-guard-v1`
  - `allcaps-topic-heading-shape-v1`

## Verification completed

- `python -m py_compile tools/layout_role_training.py tools/heading_specialist_confusion_diagnostics.py`
- `cargo test heading_specialist --quiet`
- `cargo build --release`

## Active specialist training

- Superseded run: `layout-heading-cycle053-sentencefrag-targeted-20260605-candidate`
- Old PID: `10520`
- Status update:
  - Stopped after a new end-to-end NN process started and the run had produced no output.
  - No model/report was emitted.

## Active specialist training - cycle054

- Run: `layout-heading-cycle054-observable-sentencefrag-allcaps-20260605-candidate`
- PID: `27828`
- Command: `tools/layout_role_training.py` with:
  - `--binary-role heading`
  - `--examples-input training-data/layout-role-core/lawpdf-layout-role-examples-chandra-expanded-fast-20260604.json`
  - `--gold-labels-list C:\tmp\lawpdf-heading-cycle051-targeted-20260605-1705\gold-labels-with-cycle051-targeted.txt`
  - stacked main/liquid/doclaynet/body/body_chandra models
  - `--role-bias heading=-30.0`
  - `--strict-law-review-eval`
- Output dir:
  - `profile-models/layout-heading-cycle054-observable-sentencefrag-allcaps-20260605-candidate`
- Logs:
  - `C:\tmp\lawpdf-heading-cycle054-observable-20260605\heading-train.out.log`
  - `C:\tmp\lawpdf-heading-cycle054-observable-20260605\heading-train.err.log`
- Status at approximately 19:44:
  - CPU-active, memory around 2.8 GB.
  - Loaded `335,190` lines from `848` PDFs.
  - Applied `479` gold label files.
  - Gold label stats: available `329,050`, matched `195,637`, changed `27,273`.
  - Currently in `stacked_tokens_start`; no `stacked_tokens_done` yet.
  - Stacked-token scoring over six models is the current bottleneck.

## Training observability changes

- Added progress prints to `tools/layout_role_training.py` for cached-example training:
  - start
  - loading examples
  - loaded examples
  - applying gold labels
  - stacked-token start/done
  - train start/done
  - evaluation start
- Added future inner progress logging in `stacked_tokens_for_lines` every `50,000` lines.
- Added `tools/compare_heading_specialist_reports.py` to compare baseline/candidate strict heading metrics.

## Cycle055 outcome - 20:30 CDT

- Cycle054 was stopped after spending about 21 CPU-minutes in stacked-token generation with no output.
- `tools/layout_role_training.py` was optimized so stacked model scoring computes base line feature tokens once per line and reuses them across all stacked models.
- Cycle055 run:
  - `profile-models/layout-heading-cycle055-faststack-sentencefrag-allcaps-20260605-candidate`
  - Completed at `2026-06-05 20:17`.
  - Stacked-token phase completed with 50k-line progress markers.
  - Report:
    - `profile-models/layout-heading-cycle055-faststack-sentencefrag-allcaps-20260605-candidate/layout-role-report.json`

### Raw report comparison against cycle051

- Cycle051 strict law-review gold heading:
  - P `0.3011`
  - R `0.7382`
  - F1 `0.4278`
  - macro F1 `0.7024`
- Cycle055 strict law-review gold heading:
  - P `0.2474`
  - R `0.8692`
  - F1 `0.3852`
  - macro F1 `0.6762`
- Raw model report says **do not promote** if using raw line-level model metrics alone.

### Runtime-gated diagnostic comparison

- Cycle051 10k runtime diagnostic:
  - TP `48`, FP `36`, FN `199`, TN `9717`
  - P `0.5714`, R `0.1943`, F1 `0.2900`
- Cycle055 10k runtime diagnostic:
  - `target/heading_specialist_confusion_diagnostics-cycle055-faststack-sample10k.json`
  - TP `51`, FP `27`, FN `196`, TN `9726`
  - P `0.6538`, R `0.2065`, F1 `0.3138`
- Because LawPDF ships the heading specialist behind runtime gates, cycle055 was promoted for the release despite worse raw strict-gold F1.

## Release after cycle055

- Embedded heading specialist path now points to:
  - `profile-models/layout-heading-cycle055-faststack-sentencefrag-allcaps-20260605-candidate/layout-role-model.json`
- Version marker includes:
  - `heading-cycle055-faststack-v1`
- Release artifacts:
  - `target/release/lawpdf.exe`
  - `target/release/lawpdf-latest.exe`
  - `target/release/lawpdf-latest-heading-cycle055-20260605-203046.exe`
- Verification:
  - `python -m py_compile tools/layout_role_training.py tools/heading_specialist_confusion_diagnostics.py tools/compare_heading_specialist_reports.py`
  - `cargo test heading_specialist --quiet`
  - `cargo build --release`
  - marker verified in `target/release/lawpdf.exe` and `target/release/lawpdf-latest.exe`

## Current process/resource note

- A new end-to-end NN job started at `2026-06-05 20:42`.
- Avoid launching more specialist training until it finishes.
- Continue with lightweight diagnostics or source changes only while the NN job is active.

## 20:49 CDT assessment

- Fixed diagnostic parity: `tools/heading_specialist_confusion_diagnostics.py` now treats normal-font all-caps topic headings as a heading-shape signal, matching Rust runtime.
- Reran cycle055 10k runtime diagnostic:
  - `target/heading_specialist_confusion_diagnostics-cycle055-faststack-parity-sample10k.json`
  - TP `54`, FP `38`, FN `193`, TN `9715`
  - P `0.5870`, R `0.2186`, F1 `0.3186`
- Bias sweep over cycle055 found no meaningful improvement:
  - Best additional heading bias was `-2.5`.
  - F1 moved only from `0.3186` to `0.3195`.
  - Saved sweep to `target/heading_cycle055_runtime_bias_sweep_sample10k.json`.
- Prototyped two guards and rejected both:
  - Top-page short running-header veto: hurt F1.
  - Broader sentence/prose continuation veto: hurt F1 by suppressing true headings.

## New targeted training data

- Added hard-negative label pack:
  - `training-data/chandra-teacher/heading-cycle055-faststack-runtime-fp-hardneg-20260605-labels.json`
  - `38` runtime-gated false-positive hard negatives from the cycle055 10k audit.
- Added the label pack to:
  - `C:\tmp\lawpdf-heading-cycle051-targeted-20260605-1705\gold-labels-with-cycle051-targeted.txt`
  - Gold-label list count is now `480`.

## Queued next specialist run

- Added watcher:
  - `tools/start_heading_cycle056_when_idle.ps1`
- Running watcher PID:
  - `32908`
- It waits for active NN training/eval jobs to clear, then waits an idle window of `120` seconds and launches:
  - `layout-heading-cycle056-faststack-cycle055hardneg-20260605-candidate`
- Logs:
  - `C:\tmp\lawpdf-heading-cycle056-faststack-20260605\watcher.log`
  - `C:\tmp\lawpdf-heading-cycle056-faststack-20260605\heading-train.out.log`
  - `C:\tmp\lawpdf-heading-cycle056-faststack-20260605\heading-train.err.log`

## Next comparison

When cycle053 writes `layout-role-report.json`, compare against current packaged heading model:

- Baseline:
  - `profile-models/layout-heading-cycle051-no-selfstack-priorneg30-20260605-candidate/layout-role-report.json`
- Baseline strict law-review gold heading:
  - Precision `0.3011`
  - Recall `0.7382`
  - F1 `0.4278`
  - Macro F1 `0.7024`

Promote cycle053 only if it improves strict law-review gold heading F1 or gives a clear precision/recall tradeoff useful for Liquid Mode.

## Chandra/vLLM

- Windows `http://127.0.0.1:8000/v1/models` is not reachable.
- WSL log showed prior vLLM requests, but direct `wsl --cd ~ curl http://127.0.0.1:8000/v1/models` failed at this checkpoint.
- Do not depend on Chandra until the endpoint is reverified.
