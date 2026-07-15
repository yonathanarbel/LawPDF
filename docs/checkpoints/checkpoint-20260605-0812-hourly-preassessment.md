# Checkpoint: 08:12 focused body/heading status

Date: 2026-06-05 08:12 America/Chicago

## Current processes

- Dashboard PID: 1128
- Cloudflare tunnel PID: 13752
- Main focused training loop PID: 33568
- Active body trainer PID: 35636
- Training lock is still held by `chandra-structure-disputes-heading-body-vllm-20260605`, cycle 2.

## Candidate training status

The heading seed candidate finished:

- Model: `profile-models\layout-heading-chandra-structure-disputes-heading-body-vllm-20260605-cycle002-seed-candidate\layout-role-model.json`
- Strict law-review gold evaluation:
  - total 149,694
  - heading F1 0.3803
  - precision 0.2440
  - recall 0.8610

Comparison to current selected heading model:

- Current selected `layout-heading-chandra-structure-disputes-overnight-20260604-2329-cycle048-seed-candidate`
- Strict law-review gold evaluation:
  - total 149,491
  - heading F1 0.3842
  - precision 0.2472
  - recall 0.8609

Decision: do not promote the cycle002 seed heading model. It is slightly worse than current cycle048 on the broad strict-gold evaluation despite looking good on the tiny held-out slice.

The body candidate is still training:

- Expected model: `profile-models\layout-body-chandra-structure-disputes-heading-body-vllm-20260605-cycle002-candidate\layout-role-model.json`
- Expected report: `profile-models\layout-body-chandra-structure-disputes-heading-body-vllm-20260605-cycle002-candidate\layout-role-report.json`

## Generated labels since 07:19

Focused training run:

- `chandra-structure-disputes-heading-body-vllm-20260605`
- 278 labels from 12 pages

Generation-only run 2:

- `chandra-structure-disputes-heading-body-vllm-20260605-gen2-v5`
- 388 labels from 21 pages

Generation-only run 3:

- `chandra-structure-disputes-heading-body-vllm-20260605-gen3-v5`
- 479 labels from 25 pages

Combined focused labels added so far:

- 1,145 labels from 58 pages
- body 533
- footnote 517
- header_footer 29
- contents 39
- title 6
- metadata 16
- list_item 4
- table 1

Interpretation: these runs are mostly heading hard-negative pages. They are useful for precision and body/heading arbitration, but they are not producing many positive heading labels.

## Failed generation attempt

- `chandra-structure-disputes-heading-body-vllm-20260605-gen4-v5` failed with `MemoryError` while loading `lawpdf-layout-role-examples-v5-unseen.json`.
- Cause: body trainer is using around 3 GB RAM and the extra selector could not load the 382 MB JSON simultaneously.
- Decision: do not retry another generator until the body trainer exits.

## Current improvement assessment

- The dominant broad heading problem remains precision: too many title/front-matter/body/display lines are called headings.
- Runtime gating improves precision but lowers recall heavily.
- The new Chandra labels are mainly hard negatives, which is aligned with improving precision.
- The first new heading seed model did not beat cycle048, so the next best path is:
  1. wait for body and final heading candidates from the in-flight pair,
  2. refresh diagnostics with the newly added `model_missed_audit40`,
  3. train an all-accumulated follow-up that includes gen2 and gen3 labels,
  4. only promote if strict-gold and runtime-gated diagnostics improve together.
