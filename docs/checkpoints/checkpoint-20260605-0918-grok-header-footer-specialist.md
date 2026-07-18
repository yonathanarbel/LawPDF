# Grok Header/Footer Specialist Experiment - 2026-06-05 09:18

Goal: test whether a dedicated `header_footer` specialist can remove repeated edge text, page numbers, running headers, and contents-like clutter before the heading detector has to adjudicate those lines.

Run id: `grok-header-footer-specialist-20260605-0918`

Task source:
- Examples: `training-data\layout-role-core\lawpdf-layout-role-examples-v5-unseen.json`
- Task queue: `training-data\grok-teacher\grok-header-footer-specialist-20260605-0918\tasks.jsonl`
- Queue shape: 240 tasks, `focus=noise`, strict law-review filter, up to 42 lines per task.

Detached runner:
- Parent PID at launch: `33128`
- Script: `tools\run_grok_header_footer_specialist_20260605.ps1`
- Grok loop log: `training-data\grok-teacher\grok-header-footer-specialist-20260605-0918\overnight-loop.log`
- Worker logs: `worker-00.log`, `worker-01.log`
- Raw worker labels: `worker-00.jsonl`, `worker-01.jsonl`
- Merged labels: `training-data\grok-teacher\grok-header-footer-specialist-20260605-0918\gold-labels.json`

Chained model output:
- Candidate model: `profile-models\layout-header-footer-grok-noise-20260605-0918-candidate\layout-role-model.json`
- Candidate report: `profile-models\layout-header-footer-grok-noise-20260605-0918-candidate\layout-role-report.json`
- Training log: `training-data\grok-teacher\grok-header-footer-specialist-20260605-0918\header-footer-train.log`

Implementation details:
- Uses the existing Grok execution-mode command in `tools\grok_layout_label_batch.py`: `grok -p ... --always-approve --cwd ... --no-plan --disable-web-search --max-turns ... --output-format plain`.
- Label loop settings: 2 workers, 8 tasks per worker per round, 4 rounds, confidence >= 4.
- Training settings: binary role `header_footer`, `--refresh-derived-roles`, `--strict-law-review-eval`, Grok source scale `0.35`.

Notes:
- Preparing from the 641 MB Chandra-expanded examples hit a Python `MemoryError`; this run uses the smaller 382 MB v5 unseen examples file.
- The experiment should be treated as a candidate only until its strict law-review gold report lands.
