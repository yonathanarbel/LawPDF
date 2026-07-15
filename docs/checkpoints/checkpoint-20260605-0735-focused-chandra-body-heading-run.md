# Checkpoint: focused Chandra body/heading run

Date: 2026-06-05 07:35 America/Chicago

## Dashboard

- Public URL: https://reading-testing-charging-subsidiaries.trycloudflare.com/
- Local URL: http://127.0.0.1:8765/
- Password: `Jcucmhe123`
- Dashboard PID: 30172
- Cloudflare tunnel PID from prior status: 13752
- Dashboard current run: `chandra-structure-disputes-heading-body-vllm-20260605`
- Dashboard goal window: 2026-06-05 07:19 to 2026-06-05 09:00

## Chandra/vLLM

- vLLM endpoint was verified live at http://127.0.0.1:8000/v1/models.
- Served model: `chandra`.
- User-reported vLLM PID: 1941.
- User-reported vLLM config: max model length 18000, `--max-num-seqs 10`, about 23.7 GB / 24.6 GB GPU memory after load.

## Focused run

- Run ID: `chandra-structure-disputes-heading-body-vllm-20260605`
- Output root: `training-data\chandra-teacher\chandra-structure-disputes-heading-body-vllm-20260605`
- Temp root: `C:\tmp\lawpdf-chandra-structure-disputes-heading-body-vllm-20260605`
- Main loop PID: 33568
- Current child trainer PID at checkpoint: 34656
- Log: `training-data\chandra-teacher\chandra-structure-disputes-heading-body-vllm-20260605\structure-disputes.log`
- Dashboard stdout/stderr:
  - `training-data\chandra-teacher\chandra-structure-disputes-heading-body-vllm-20260605\dashboard.stdout.log`
  - `training-data\chandra-teacher\chandra-structure-disputes-heading-body-vllm-20260605\dashboard.stderr.log`

Command shape:

```powershell
python tools\chandra_structure_disagreement_loop.py `
  --run-id chandra-structure-disputes-heading-body-vllm-20260605 `
  --examples-input training-data\layout-role-core\lawpdf-layout-role-examples-chandra-expanded-fast-20260604.json `
  --tmp-root C:\tmp\lawpdf-chandra-structure-disputes-heading-body-vllm-20260605 `
  --batch-size 10 `
  --heading-pages-per-batch 7 `
  --candidate-pool 800 `
  --max-batches 2 `
  --train-every-batches 2 `
  --ocr-jobs 8 `
  --min-similarity 0.80 `
  --model body_chandra=profile-models/layout-body-chandra-structure-disputes-20260604-interim-104950-cycle064-candidate/layout-role-model.json `
  --model heading_chandra=profile-models/layout-heading-chandra-structure-disputes-overnight-20260604-2329-cycle048-seed-candidate/layout-role-model.json
```

Note: an earlier aborted start passed all default models explicitly, which duplicated default stack names in the log. That process was stopped before OCR. The active process uses default stack plus only `body_chandra` and `heading_chandra`.

## Progress

- Loaded 335,190 Chandra-expanded example lines.
- Strict law-review paths: 432.
- Broader law-review-like paths: 514.
- Candidate queue: 256 pages.
- Batch 1:
  - selected 10 pages, top bucket `heading_hard_negative`
  - Chandra OCR: 10/10 ok, average 36.46 seconds/page, average native similarity 0.5898
  - accepted labels: 154 from 6 pages
  - role counts: body 66, footnote 54, metadata 13, header_footer 8, contents 5, list_item 4, title 3, table 1
- Batch 2:
  - selected 10 pages, top bucket `heading_hard_negative`
  - Chandra OCR: 10/10 ok, average 33.847 seconds/page, average native similarity 0.48
  - accepted labels: 124 from 6 pages
  - role counts: body 43, footnote 74, header_footer 5, metadata 1, title 1
- Current run total visible on dashboard: 278 labels from 12 accepted pages.

## Current activity

The loop is training after batch 2. At the checkpoint it is running the heading seed candidate:

- Model output: `profile-models\layout-heading-chandra-structure-disputes-heading-body-vllm-20260605-cycle002-seed-candidate\layout-role-model.json`
- Report output: `profile-models\layout-heading-chandra-structure-disputes-heading-body-vllm-20260605-cycle002-seed-candidate\layout-role-report.json`
- Prediction output: `C:\tmp\lawpdf-chandra-structure-disputes-heading-body-vllm-20260605\layout-heading-chandra-structure-disputes-heading-body-vllm-20260605-cycle002-seed-candidate-predictions.json`

Expected next steps after the seed heading trainer exits:

1. Train the body candidate for cycle 002.
2. Train the final heading candidate for cycle 002 using the body candidate in the stack.
3. Stop at `--max-batches 2`.
4. Compare new reports against current selected models before promoting anything.

## Diagnostics added this turn

- `tools\heading_specialist_confusion_diagnostics.py` now emits runtime-gated false-negative causes and `false_negative_audit40`.
- `tools\progress_dashboard.py` now displays runtime-gated false-negative causes and a `Top 40 Missed True Headings` table.
- Latest runtime sample remains:
  - raw heading F1 0.2662 / precision 0.1661 / recall 0.6700
  - runtime-gated F1 0.3386 / precision 0.3852 / recall 0.3020
  - false-negative causes: model_missed 165, runtime_gate_shape 105, runtime_gate_period 40, runtime_gate_other 36, runtime_gate_first_page_title_guard 3
