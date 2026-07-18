# Heading Specialist Cycle056 Progress - 2026-06-05 21:08 CDT

## Current state

- Active run: `layout-heading-cycle056-faststack-cycle055hardneg-20260605-candidate`
- Active PID: `18816`
- Training source: `training-data/layout-role-core/lawpdf-layout-role-examples-chandra-expanded-fast-20260604.json`
- Gold-label list: `C:\tmp\lawpdf-heading-cycle051-targeted-20260605-1705\gold-labels-with-cycle051-targeted.txt`
- Active label-file count: `480`
- Output dir: `profile-models/layout-heading-cycle056-faststack-cycle055hardneg-20260605-candidate`
- Logs:
  - `C:\tmp\lawpdf-heading-cycle056-faststack-20260605\heading-train.out.log`
  - `C:\tmp\lawpdf-heading-cycle056-faststack-20260605\heading-train.err.log`

## Latest log evidence

- Loaded `335,190` lines from `848` PDFs.
- Applied gold labels:
  - available `329,063`
  - matched `195,643`
  - changed `27,265`
- Stacked-token phase is progressing:
  - `50,000 / 335,190` at elapsed `143.3s`
  - `100,000 / 335,190` at elapsed `305.1s`
  - `150,000 / 335,190` at elapsed `463.2s`

## vLLM / Chandra

- Windows check failed:
  - `http://127.0.0.1:8000/v1/models` returned unable to connect.
- WSL check failed:
  - `curl` to `127.0.0.1:8000/v1/models` could not connect.
- Do not depend on Chandra until the endpoint is reverified.

## Diagnostic/data update while cycle056 runs

- Created hard-positive audit material:
  - `training-data/chandra-teacher/heading-cycle055-faststack-runtime-fn-hardpos-20260605-labels.json`
  - `80` rows from cycle055 false negatives/model misses.
- This pack was removed from the active gold-label list after inspection because it includes apparent noisy gold positives such as citation/body lines marked as headings.
- Active list is back to `480` label files.
- Created suspicious-gold audit:
  - `target/heading_cycle055_suspicious_gold_heading_false_negatives_20260605.json`
  - `8` obvious suspicious rows found in the top model-missed audit sample.
- Patched `tools/heading_specialist_confusion_diagnostics.py` to flag `gold_heading_likely_noise` in future reports.
- Added `runtime_gated_heading_confusion_gold_noise_adjusted` to the diagnostic output so promotion decisions can compare strict gold against a noise-aware Liquid-oriented metric.
- Patched stacked scoring in `tools/layout_role_training.py` and `tools/heading_specialist_confusion_diagnostics.py` to reuse hashed feature indices per feature dimension across stacked models. This should speed the next cycle and diagnostics without changing predictions.
- Validation:
  - `python -m py_compile tools/layout_role_training.py tools/heading_specialist_confusion_diagnostics.py`

## Current judgment

- Footnote and body specialists are no longer the limiting problem.
- Heading progress is real but shallow; the next high-value work is separating true section headings from body/title/list/header material and improving the quality of heading gold labels.
- The strict heading metric is partially contaminated by noisy gold, so runtime diagnostics and qualitative audits should drive promotion decisions.

## Cycle056 outcome - 21:28 CDT

- Cycle056 completed and wrote:
  - `profile-models/layout-heading-cycle056-faststack-cycle055hardneg-20260605-candidate/layout-role-model.json`
  - `profile-models/layout-heading-cycle056-faststack-cycle055hardneg-20260605-candidate/layout-role-report.json`
- Broad strict-gold comparison against cycle055:
  - cycle055 heading F1 `0.3852`
  - cycle056 heading F1 `0.3844`
  - recommendation: do not promote on raw strict-gold metrics.
- Runtime 10k diagnostic:
  - `target/heading_specialist_confusion_diagnostics-cycle056-faststack-sample10k.json`
  - TP `54`, FP `38`, FN `193`, TN `9715`
  - precision `0.5870`, recall `0.2186`, F1 `0.3186`
  - This ties cycle055 parity exactly, so cycle056 is not promoted.
- Noise-aware runtime metric:
  - TP `54`, FP `38`, FN `182`, TN `9726`
  - F1 `0.3293`
  - `11` strict-gold heading false negatives were flagged as likely noisy gold.

## Rejected follow-up tests

- Narrow top-of-page running-title veto:
  - baseline runtime F1 `0.3186`
  - guarded F1 `0.3006`
  - killed `8` false positives but also `5` true headings, so rejected.
- Existing edge-running-header/footer rule as a heading-specialist veto:
  - baseline runtime F1 `0.3186`
  - veto F1 `0.2741`
  - a variant preserving clear section headings still only reached F1 `0.2848`
  - rejected because it killed true top-page headings such as `MAPS AND CHARTS`, `CIVIL PROCEDURE`, `APPELLATE PRACTICE & PROCEDURE`, and `RIGHT TO BEAR ARMS`.
- Fresh header/footer specialist:
  - `profile-models/layout-header-footer-cycle002-expanded-hardlabels-20260605-candidate`
  - High recall but too many false positives on broad gold.
  - On the cycle056 top 38 heading false positives, it caught only `2`, both true body rows, and missed the one gold `header_footer` row.
  - Not promoted.

## Cycle057 queued

- Added hard-negative label pack from cycle056 runtime false positives, then filtered it before cycle057 launched:
  - `training-data/chandra-teacher/heading-cycle056-faststack-runtime-fp-hardneg-20260605-labels.json`
  - `38` labels
  - intermediate replacement: `training-data/chandra-teacher/heading-cycle056-faststack-runtime-fp-hardneg-nonlist-20260605-labels.json`
  - `30` labels, excluding `8` ambiguous `list_item` outline-heading rows
  - active replacement: `training-data/chandra-teacher/heading-cycle056-faststack-runtime-fp-hardneg-safe-20260605-labels.json`
  - `9` labels, keeping only high-confidence non-heading rows: prose continuations, title fragments, and one header/footer row
  - excludes short all-caps/topic rows like `CRIMINAL LAW`, `BACKGROUND`, and `MEDICAL EXPERT WITNESSES` because those may be legitimate Liquid headings despite current strict labels.
- Added noisy-gold correction pack:
  - `training-data/chandra-teacher/heading-suspicious-gold-heading-corrections-20260605-labels.json`
  - `8` labels correcting obvious strict-gold heading noise:
    - `4` body continuations
    - `4` footnote/citation-note lines
- Added to active gold-label list:
  - `C:\tmp\lawpdf-heading-cycle051-targeted-20260605-1705\gold-labels-with-cycle051-targeted.txt`
  - active label-file count is now `482`
- Initial direct cycle057 launch failed because the ad hoc PowerShell command split `NAME=PATH` stacked-model arguments. It produced only an argument parser error and no model/report.
- Created corrected idle watcher:
  - `tools/start_heading_cycle057_when_idle.ps1`
- Started watcher detached:
  - PID `33164`
  - watcher log: `C:\tmp\lawpdf-heading-cycle057-faststack-20260605\watcher.log`
  - current state at 21:46 CDT: waiting for active NN process `37172`
- When the NN clears, the watcher will launch cycle057:
  - model dir: `profile-models/layout-heading-cycle057-faststack-cycle056hardneg-20260605-candidate`
  - stdout: `C:\tmp\lawpdf-heading-cycle057-faststack-20260605\heading-train.out.log`
  - stderr: `C:\tmp\lawpdf-heading-cycle057-faststack-20260605\heading-train.err.log`
- Cycle057 uses the optimized stacked-index reuse code added after cycle056 started.

## Queue update - 22:03 CDT

- Cycle057 has not launched yet because the external NN loop keeps starting new GPU runs:
  - v8 finished and was not promoted.
  - v9 finished and was not promoted.
  - v10 is active:
    - PID `512`
    - output dir: `profile-models/end-to-end-liquidnet-fivecore-titleheading-v10-v5data-excludeholdoutdocs-rareweights-gpu-20260605`
- The watcher is behaving correctly:
  - it waits for active `liquid_deep_train.py`/eval/audit processes
  - it requires a 120-second idle window before launching specialist cycle057
- vLLM/Chandra is still not reachable from Windows or WSL on `127.0.0.1:8000`.

## Cycle057 and release update - 22:49 CDT

- Cycle057 eventually launched after an idle window and completed:
  - model: `profile-models/layout-heading-cycle057-faststack-cycle056hardneg-20260605-candidate/layout-role-model.json`
  - report: `profile-models/layout-heading-cycle057-faststack-cycle056hardneg-20260605-candidate/layout-role-report.json`
- Cycle057 training benefited from the stacked-index reuse optimization:
  - stacked token generation finished in `455.7s`
  - full train finished in `736.9s`
- Cycle057 did not improve the runtime heading diagnostic before gate changes:
  - TP `54`, FP `38`, FN `193`, TN `9715`
  - precision `0.5870`, recall `0.2186`, F1 `0.3186`
  - same as cycle055/cycle056 on the 10k strict-law-review runtime sample
  - not promoted.
- Added a narrow runtime gate exception for `page_contents_like` lines that are clear section headings:
  - requires clear roman/letter section-heading text
  - requires heading shape: `heading_geometry_like`, or centered/bold with `font_ratio_body >= 1.02`
  - rejects dot-leader contents fragments/lines and period-ending prose
- Validation after this gate:
  - cycle055 + gate: TP `55`, FP `38`, FN `192`, TN `9715`, F1 `0.3235`
  - cycle057 + gate: TP `55`, FP `38`, FN `192`, TN `9715`, F1 `0.3235`
  - because cycle057 ties cycle055, kept the shipped cycle055 model and shipped only the gate.
- Verification:
  - `python -m py_compile tools/layout_role_training.py tools/heading_specialist_confusion_diagnostics.py`
  - `cargo fmt --check`
  - `cargo test heading_specialist --quiet`
  - `cargo build --release`

## Cycle059 expanded2 noisy-gold attempt - 00:10 CDT, 2026-06-06

- Expanded the diagnostic likely-noisy-heading detector again for obvious non-heading strict labels:
  - TOC/index and dot-leader contents rows
  - early article title/byline fragments
  - law-review masthead/header rows
  - kept ambiguous labels such as `ABSTRACT` and topical one-word labels out of the automatic-noise bucket.
- Noise-expanded2 diagnostic on cycle058:
  - raw/runtime metric unchanged: TP `60`, FP `38`, FN `187`, F1 `0.3478`
  - likely noisy strict heading labels: `86`
  - noise-adjusted runtime metric: TP `60`, FP `38`, FN `101`, precision `0.6122`, recall `0.3727`, F1 `0.4633`
- Created correction pack:
  - `training-data/chandra-teacher/heading-suspicious-gold-heading-corrections-expanded2-20260605-labels.json`
  - `86` labels total: `57` body, `13` contents, `7` title, `7` footnote, `2` header_footer
  - appended to active heading gold-label list:
    - `C:\tmp\lawpdf-heading-cycle051-targeted-20260605-1705\gold-labels-with-cycle051-targeted.txt`
    - active label-file count: `484`
- Trained cycle059:
  - model: `profile-models/layout-heading-cycle059-faststack-expanded2-goldnoise-20260605-candidate/layout-role-model.json`
  - report: `profile-models/layout-heading-cycle059-faststack-expanded2-goldnoise-20260605-candidate/layout-role-report.json`
  - gold stats: available `329,092`, changed `27,331`, matched `195,657`
  - stacked tokens done in `481.5s`
  - training done in `744.0s`
- Comparison against promoted cycle058:
  - macro F1 `0.6776` vs `0.6783` (`-0.0007`)
  - heading F1 `0.3875` vs `0.3887` (`-0.0012`)
  - recommendation: `do_not_promote`
- Runtime 10k diagnostic:
  - cycle059 ties cycle058: TP `60`, FP `38`, FN `187`, F1 `0.3478`
- Decision:
  - do not promote cycle059
  - keep release on cycle058
  - copied cycle058/noiseexpanded2 diagnostic back to canonical dashboard files:
    - `target/heading_specialist_confusion_diagnostics.json`
    - `target/heading_specialist_clash_audit_top40.md`
- Current release remains:
  - `target/release/lawpdf-latest.exe`
  - `target/release/lawpdf-latest-heading-cycle058-commonsection-20260605-234300.exe`

## Abstract page-contents heading gate - 00:19 CDT, 2026-06-06

- Deep audit of remaining non-noisy heading false negatives showed that broad weak-topic heading relaxation was not worth shipping:
  - weak short-topic/font rule: strict F1 `0.3504`, but clean/noise-adjusted F1 fell from `0.4633` to `0.4561`
  - it admitted many running headers and short body/header fragments such as `NOTES AND COMMENT`, `FEDERAL TAXATION`, and `BANKRUPTCY` header rows
  - rejected.
- A narrow exact `ABSTRACT` exception was clean on the same sample:
  - added `1` true heading
  - added `0` false positives
  - strict runtime heading metric improved from TP `60`, FP `38`, FN `187`, F1 `0.3478`
  - to TP `61`, FP `38`, FN `186`, precision `0.6162`, recall `0.2470`, F1 `0.3526`
  - noise-adjusted runtime metric improved to F1 `0.4692`
- Implemented:
  - added `abstract` to exact common section labels in `tools/heading_specialist_confusion_diagnostics.py`
  - added `abstract` to `looks_like_common_section_heading_label` in `src/layout_roles.rs`
  - added a Rust heading-specialist test for page-contents-like `ABSTRACT` with weak/no shape
  - updated release marker with `pagecontents-abstract-heading-v1`
- Verification:
  - `python -m py_compile tools/layout_role_training.py tools/heading_specialist_confusion_diagnostics.py`
  - `cargo fmt --check`
  - `cargo test heading_specialist --quiet`
  - `python tools/heading_specialist_confusion_diagnostics.py ... --output-json target/heading_specialist_confusion_diagnostics-cycle058-abstract-gate-sample10k.json`
  - `cargo build --release`
- Fresh release copied:
  - `target/release/lawpdf-latest.exe`
  - `target/release/lawpdf-latest-heading-cycle058-abstract-20260606-001939.exe`
- Dashboard canonical files updated:
  - `target/heading_specialist_confusion_diagnostics.json`
  - `target/heading_specialist_clash_audit_top40.md`
- Fresh release copied:
  - `target/release/lawpdf-latest.exe`
  - `target/release/lawpdf-latest-heading-pagecontents-20260605-224927.exe`

## Common section label gate - 23:05 CDT

- Audit showed a recurring strict heading false-negative cluster where real `INTRODUCTION` headings were tagged `page_contents_like` and blocked by the runtime heading gate.
- Tested broader `page_contents_like` clear-heading relaxation first:
  - F1 `0.3483`
  - added `7` TP but also `9` FP, including TOC rows/title/metadata/list rows, so rejected.
- Implemented the narrower exact common-section-label exception:
  - labels: `introduction`, `background`, `analysis`, `discussion`, `conclusion`, `appendix`
  - still rejects period-ending lines and dot-leader contents lines
  - only broadens the candidate gate; the heading model must still vote `heading`
- Runtime diagnostic on cycle055:
  - previous gate: TP `55`, FP `38`, FN `192`, F1 `0.3235`
  - common-section gate: TP `60`, FP `38`, FN `187`, precision `0.6122`, recall `0.2429`, F1 `0.3478`
- Runtime diagnostic on cycle057 tied cycle055 exactly after the gate:
  - TP `60`, FP `38`, FN `187`, F1 `0.3478`
  - kept cycle055 as shipped model.
- Verification:
  - `python -m py_compile tools/layout_role_training.py tools/heading_specialist_confusion_diagnostics.py`
  - `cargo fmt --check`
  - `cargo test heading_specialist --quiet`
  - `cargo build --release`
- Fresh release copied:
  - `target/release/lawpdf-latest.exe`
  - `target/release/lawpdf-latest-heading-commonsection-20260605-230509.exe`

## Cycle058 noisy-gold correction model - 23:43 CDT

- Expanded the diagnostic likely-noisy-heading detector to include sentence-case body continuation lines that strict gold marks as headings.
- Noise-expanded diagnostic on cycle055 + common-section gate:
  - raw/runtime metric unchanged: TP `60`, FP `38`, FN `187`, F1 `0.3478`
  - likely noisy strict heading labels: `66`
  - noise-adjusted runtime metric: TP `60`, FP `38`, FN `121`, precision `0.6122`, recall `0.3315`, F1 `0.4301`
- Created correction pack:
  - `training-data/chandra-teacher/heading-suspicious-gold-heading-corrections-expanded-20260605-labels.json`
  - `66` labels total: `59` body, `7` footnote
  - appended to active heading gold-label list:
    - `C:\tmp\lawpdf-heading-cycle051-targeted-20260605-1705\gold-labels-with-cycle051-targeted.txt`
    - active label-file count: `483`
- Trained cycle058:
  - model: `profile-models/layout-heading-cycle058-faststack-expanded-goldnoise-20260605-candidate/layout-role-model.json`
  - report: `profile-models/layout-heading-cycle058-faststack-expanded-goldnoise-20260605-candidate/layout-role-report.json`
  - loaded `335,190` lines / `848` PDFs
  - gold stats: available `329,090`, changed `27,314`, matched `195,656`
  - stacked tokens done in `430.6s`
  - training done in `693.3s`
- Broad strict-gold comparison against shipped cycle055:
  - heading F1 `0.3887` vs `0.3852` (`+0.0035`)
  - heading recall `0.8766` vs `0.8692` (`+0.0074`)
  - macro F1 `0.6783` vs `0.6762` (`+0.0021`)
  - comparison script recommendation: `promote`
- Runtime 10k diagnostic with current gates:
  - cycle058 ties cycle055: TP `60`, FP `38`, FN `187`, F1 `0.3478`
  - no runtime regression on the tracked sample.
- Promotion:
  - updated `src/layout_roles.rs` heading model include to cycle058
  - copied cycle058 diagnostic to canonical dashboard files:
    - `target/heading_specialist_confusion_diagnostics.json`
    - `target/heading_specialist_clash_audit_top40.md`
  - rebuilt release:
    - `target/release/lawpdf-latest.exe`
    - `target/release/lawpdf-latest-heading-cycle058-commonsection-20260605-234300.exe`
- Verification:
  - `python -m py_compile tools/layout_role_training.py tools/heading_specialist_confusion_diagnostics.py`
  - `cargo fmt --check`
  - `cargo test heading_specialist --quiet`
  - `cargo build --release`
