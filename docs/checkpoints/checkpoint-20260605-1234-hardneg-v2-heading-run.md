# Checkpoint: heading hard-negative v2 run

Time: 2026-06-05 12:34 America/Chicago

## Latest completed heading candidate

Model:

- `profile-models/layout-heading-pagecaps-earlymeta-allaccum-20260605-0920-candidate`

Strict law-review heading result:

- F1 0.3596
- Precision 0.2256
- Recall 0.8859
- total 149,694

Decision:

- Not promotable. It raised recall but precision fell too much compared with current selected heading model:
  - `layout-heading-chandra-structure-disputes-overnight-20260604-2329-cycle048-seed-candidate`
  - F1 0.3842, P 0.2472, R 0.8609

## vLLM status

- `http://127.0.0.1:8000/v1/models` still refused connections from Windows.
- Chandra/vLLM is not currently usable from this process.

## New local training data

Created:

- `training-data/chandra-teacher/heading-hard-negative-audit-v2-20260605-labels.json`

Contents:

- 35 labels total
- 29 `body`
- 4 `header_footer`
- 2 `title`

Source:

- `heading_hard_negative_audit_v2`

Notes:

- Built from current top 40 heading false-positive clashes after fragment guards.
- Skipped `list_item` rows because many are section-heading-like for Liquid Mode even if strict taxonomy says not heading.

## Active background run

Model:

- `profile-models/layout-heading-hardneg-v2-bias23-20260605-1225-candidate`

Process:

- PID `8384`

Command intent:

- binary heading specialist
- examples: `training-data/layout-role-core/lawpdf-layout-role-examples-chandra-expanded-fast-20260604.json`
- gold labels: existing accumulated list plus `heading-hard-negative-audit-v2-20260605-labels.json`
- role bias: `heading=-2.3`
- stacked models: main, liquid, doclaynet main/liquid, body, body_chandra, heading_chandra

Logs:

- `C:\tmp\lawpdf-heading-hardneg-v2-bias23-20260605-1225\heading-train.out.log`
- `C:\tmp\lawpdf-heading-hardneg-v2-bias23-20260605-1225\heading-train.err.log`

Expected dashboard report:

- `profile-models/layout-heading-hardneg-v2-bias23-20260605-1225-candidate/layout-role-report.json`

