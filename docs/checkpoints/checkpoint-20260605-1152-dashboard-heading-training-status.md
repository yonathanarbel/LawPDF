# Checkpoint: dashboard restored and heading training running

Time: 2026-06-05 11:52 America/Chicago

## Live services

- Local dashboard: `http://127.0.0.1:8765/`
- Public dashboard: `https://moderate-feel-concluded-breeding.trycloudflare.com`
- Password: `Jcucmhe123`
- Dashboard PID: `10304` at verification time; later restarted as needed via `tools/progress_dashboard.py`.
- Cloudflared PID: `8312`.

## Dashboard fixes

- Replaced slow dashboard process scanning with a lightweight `Get-Process` snapshot.
- Bounded label-history aggregation in `tools/progress_dashboard.py`:
  - recent all-Chandra label files capped at 80
  - files per run capped at 20
  - label run dirs capped at 12
- `build_status(...)` timed at about 1.5s after patching.
- `/api/status` returned HTTP 200 after restart.
- Dashboard now explicitly prefers canonical `target/heading_specialist_confusion_diagnostics.json` so rejected experiments do not become the displayed current metric.

## Current accepted heading diagnostic

Canonical file restored to:

- `target/heading_specialist_confusion_diagnostics.json`
- `target/heading_specialist_clash_audit_top40.md`

Accepted 10k strict sample after fragment guard:

- runtime heading: F1 0.3603, P 0.5074, R 0.2794
- final cascade: F1 0.3560, P 0.5037, R 0.2753

Rejected experiment:

- `target/heading_specialist_confusion_diagnostics-runtime-cycle048-centeredprose-sample10k.json`
- Hard centered-prose gate raised precision but dropped F1 to 0.2943 final, so it was not kept as a runtime gate.
- The centered-prose signal remains as a trainable feature only.

## Active heading trainer

Running:

- PID `2072`
- command: `tools/layout_role_training.py --binary-role heading ...`
- output model: `profile-models/layout-heading-pagecaps-earlymeta-allaccum-20260605-0920-candidate/layout-role-model.json`
- report: `profile-models/layout-heading-pagecaps-earlymeta-allaccum-20260605-0920-candidate/layout-role-report.json`
- log: `C:\tmp\lawpdf-heading-pagecaps-earlymeta-20260605-0920\heading-train.log`

Started at:

- 2026-06-05 11:44:36

No report existed yet at 11:52.

## vLLM status

- Windows `http://127.0.0.1:8000/v1/models`: connection refused.
- WSL `http://127.0.0.1:8000/v1/models`: connection refused.
- Do not assume Chandra/vLLM is available until rechecked or restarted.

## Next decision

Wait for the heading trainer report, then compare strict gold heading F1/P/R against:

- current selected heading model: `layout-heading-chandra-structure-disputes-overnight-20260604-2329-cycle048-seed-candidate`
- previous cycle002 candidate: F1 0.3798, P 0.2436, R 0.8607
- accepted runtime diagnostic: final F1 0.3560 on 10k strict sample

If the new model improves strict gold or useful runtime behavior, use it as the next heading seed. If not, use its errors to build the next Chandra dispute cycle once vLLM is reachable again.

