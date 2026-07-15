# Checkpoint: runtime heading diagnostics and dashboard

Date: 2026-06-05 07:15 America/Chicago

## Running services

- Dashboard local URL: http://127.0.0.1:8765/
- Dashboard public URL from prior tunnel: https://reading-testing-charging-subsidiaries.trycloudflare.com/
- Dashboard password: `Jcucmhe123`
- Dashboard process: PID 34956
- Cloudflare tunnel process: PID 13752
- vLLM/Chandra was reported running by the user at http://127.0.0.1:8000/v1, PID 1941, served model `chandra`, max model length 18000, `--max-num-seqs 10`.

## Current release

- Release executable: `target\release\lawpdf.exe`
- Latest copy: `target\release\lawpdf-latest.exe`
- Release timestamp: 2026-06-05 06:56:34
- SHA256: `E4AE06E8DC1C8F0418546BAA3B54556402A05ADA0CB80EC14E941BC7F1C13257`
- Release includes the runtime heading stack tokens and the first-page display/title fragment guard.

## Model status

- Heading model remains `layout-heading-chandra-structure-disputes-overnight-20260604-2329-cycle048-seed-candidate`.
- Body model remains `layout-body-chandra-structure-disputes-20260604-interim-104950-cycle064-candidate`.
- Footnote model remains `layout-footnote-v68-chandra-filtered3-w035-20260603-candidate`.
- Later heading candidates cycle049, cycle050, and cycle051 were rejected; they did not beat cycle048 in controlled eval or had unacceptable recall tradeoffs.

## Latest runtime-aligned heading diagnostic

File: `target\heading_specialist_confusion_diagnostics-runtime-cycle048-sample.json`

Scope: deterministic 20,000-line sample from strict law-review gold labels.

- Raw heading specialist: F1 0.2662, precision 0.1661, recall 0.6700, TP 335, FP 1682, FN 165, TN 17818.
- Runtime-gated heading: F1 0.3386, precision 0.3852, recall 0.3020, TP 151, FP 241, FN 349, TN 19259.
- Residual after high-priority specialists: F1 0.3405, precision 0.3886, recall 0.3030, TP 150, FP 236, FN 345, TN 12204.
- Final cascade: F1 0.3386, precision 0.3886, recall 0.3000, TP 150, FP 236, FN 350, TN 19264.

Interpretation: runtime gating greatly improves precision but cuts recall hard. The next useful work is a false-negative audit of true headings blocked by the runtime gate or missed by the specialist, not more blind hard-negative training.

## New changes after release build

- `tools/heading_specialist_confusion_diagnostics.py` now emits:
  - `runtime_gated_false_negative_causes`
  - `false_negative_audit40`
  - a markdown section titled `Top 40 Runtime-Gated False Negatives`
- `tools/progress_dashboard.py` now displays:
  - runtime-gated false-negative cause counts
  - a `Top 40 Missed True Headings` table

These are diagnostic/dashboard changes only. They do not affect the released executable.
