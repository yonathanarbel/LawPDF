# Checkpoint: vLLM label generation and heading diagnostics

Date: 2026-06-05 07:50 America/Chicago

## Dashboard

- Public URL: https://reading-testing-charging-subsidiaries.trycloudflare.com/
- Local URL: http://127.0.0.1:8765/
- Password: `Jcucmhe123`
- Dashboard PID: 1128
- Cloudflare tunnel PID: 13752
- Dashboard current run: `chandra-structure-disputes-heading-body-vllm-20260605-gen2-v5`
- Current run labels shown by API: 388 labels from 21 pages

## Training still running

- Parent Chandra structure loop PID: 33568
- Active child trainer PID: 34656
- Training lock: `training-data\chandra-teacher\_layout-train.lock`
- Lock owner:
  - pid 33568
  - run_id `chandra-structure-disputes-heading-body-vllm-20260605`
  - cycle 2
  - started 2026-06-05T07:24:39
- Current trainer is the heading seed model:
  - `profile-models\layout-heading-chandra-structure-disputes-heading-body-vllm-20260605-cycle002-seed-candidate\layout-role-model.json`
- No report/model file had been emitted yet at the checkpoint.

## Focused training run data

Run: `chandra-structure-disputes-heading-body-vllm-20260605`

- Batch 1 accepted 154 labels from 6 pages.
- Batch 2 accepted 124 labels from 6 pages.
- Total: 278 labels from 12 pages.
- Role counts:
  - body 109
  - footnote 128
  - title 4
  - metadata 14
  - header_footer 13
  - contents 5
  - list_item 4
  - table 1

## Generation-only run

Run: `chandra-structure-disputes-heading-body-vllm-20260605-gen2-v5`

Purpose: keep vLLM useful while the expanded-data trainer holds the CPU/training lock.

Command used the smaller examples file:

- `--examples-input training-data\layout-role-core\lawpdf-layout-role-examples-v5-unseen.json`
- `--batch-size 6`
- `--heading-pages-per-batch 4`
- `--candidate-pool 600`
- `--max-batches 4`
- `--train-every-batches 0`
- `--skip-final-train`
- `--ocr-jobs 5`

Result:

- Loaded 184,945 lines.
- Strict law-review paths: 259.
- Candidate queue: 130 pages.
- Batch 1: 60 labels.
- Batch 2: 106 labels.
- Batch 3: 105 labels.
- Batch 4: 117 labels.
- Total: 388 labels from 21 pages.
- Role counts:
  - body 219
  - footnote 119
  - contents 34
  - header_footer 12
  - title 2
  - metadata 2

Combined focused body/heading runs added 666 labels so far.

## Tooling changes

- `tools\chandra_structure_disagreement_loop.py`
  - Added `--skip-final-train`.
  - This allows Chandra label generation without starting another body/heading training pass.
- `tools\heading_specialist_confusion_diagnostics.py`
  - Added `model_missed_audit40` and markdown section `Top 40 Model-Missed True Headings`.
- `tools\progress_dashboard.py`
  - Added dashboard table `Top 40 Model-Missed True Headings`.
  - Dashboard needs a refreshed diagnostic JSON before this new table has rows.

## Next actions

1. Wait for PID 34656 to finish.
2. Compare the produced cycle002 reports against the current selected body and heading models.
3. Run a new diagnostic refresh so `model_missed_audit40` appears on the dashboard.
4. Train a follow-up candidate that includes both focused runs if the first candidate does not already improve.
