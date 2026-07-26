# Failure modes and remediation

Measured at v0.2.14 on the 100-article benchmark: mean source recall 90.9%,
median 0.934, 671 critical defects, 1 clean document. Half the defects live in
18 documents. Five documents sit below 0.80 recall.

## What is failing

| Defect | Count | Share |
|---|---:|---:|
| `footnote.definition_in_body` | 246 | 36.7% |
| `paragraph.suspected_fusion` | 141 | 21.0% |
| `footnote.sequence_gap` | 82 | 12.2% |
| `recall.source_loss` | 69 | 10.3% |
| `footnote.sequence_disorder` | 31 | 4.6% |
| `heading.fused` | 29 | 4.3% |
| `heading.unterminated` | 23 | 3.4% |
| `furniture.page_number` | 22 | 3.3% |
| everything else | 28 | 4.2% |

Reading the instances rather than the labels changes what they mean.
`heading.unterminated` is mostly a *truncated* heading — "C. The Rule of Lenity
Should Trump Use of Legislative History and" — where a two-line heading lost its
second line, not body text mistaken for a heading. `definition_in_body` is
mostly footnote text that could not be attached to the note above it. And some
of it is neither: old bound volumes carry advertisements ("If You Haven't Eaten
at Beren's You've Missed Something") that are not article content at all.

## Five failure modes

**FM1 — Classification deletes text irreversibly.** A line the model calls
`HideNoise` is removed and cannot be recovered by anything downstream.
Georgetown 1926 loses 1,088 lines this way, 21% of its text, against perhaps
220 lines of genuine running heads in a volume that long. After the v0.2.14
marginalia fix, generation retains 87–100% of what reaches it, so this is now
the dominant *recall* loss on real articles.

**FM2 — Nothing distinguishes a footnote's start from its continuation.**
373 defects, 56% of the total. The model can say "this line is marginalia"; it
cannot say "this line continues the note above". So continuation lines cannot be
attached, and land as stray body text.

**FM3 — Nothing marks a paragraph boundary.** 141 defects. Paragraph starts are
recovered by a first-line-indent heuristic in `paragraph_boundary`, which is why
fusion persists wherever scanned geometry is noisy.

**FM4 — Nothing marks a heading's extent.** 66 defects. A heading spanning two
lines has no way to say so.

**FM5 — Journal archives are not all articles.** Four of the five worst
documents are an Oregon course catalog, a municipal notice, and the alumni
magazine. Partly a benchmark composition issue; also a real case, since users
open such PDFs.

## The common root

FM2, FM3 and FM4 are one problem: **the label space assigns a role per line and
says nothing about structure.** `Keep | Marginalia | HideNoise` cannot express
"starts a unit" versus "continues one", nor how far a unit extends. Together
they are **580 of 671 defects, 86%**.

Every guard shipped in this repository exists to reconstruct that missing
information downstream — the separator rule, the indent threshold, note
monotonicity, the heading demotion, the citation-continuation test. Each fixed
2–8 defects. At that rate the remaining backlog is a hundred more rules, and
each one is a new failure mode: two of the three worst bugs found in this
project were guards that were correct in intent and destructive in effect.

FM1 is a different structural error: **a classifier is allowed to delete.**
Both of the largest bugs found here — the separator rule discarding whole
blocks, and marginalia never being emitted — had this shape. Deletion should
not be a side effect of a role decision.

## Options

### A. Extend the tag set to role × boundary, and decode under constraints

Give each line a role *and* a boundary tag (`B` for begins a unit, `I` for
inside one), then decode the document as a sequence under the invariants law
review articles satisfy: note numbers ascend, every marker has one definition,
headings nest.

Addresses FM2, FM3, FM4 at the root — 86% of defects — and lets guards be
deleted rather than accumulated.

*The labels already exist implicitly.* Boundary tags are derivable from what is
already measured: first-line indent gives paragraph starts, the printed note
number gives footnote starts, font and centring give heading extent. The
labelling can be done offline against the existing 335k-line corpus and checked
against the invariants before any training happens.

**Cost: high, and higher than it looks.** Retraining the emission model in
isolation was measured in this project and made held-out quality *worse* — line
macro F1 rose 0.687 → 0.905 while defects doubled — because the decoder,
overlays and footnote bias above it are co-adapted to the current model's
deliberate miscalibration. So this cannot be a drop-in model swap; the assembly
layer has to be reworked to consume boundaries instead of inferring them. Weeks,
not days.

**Benefit: the only option that lowers the ceiling rather than the floor.**

### B. Constrained decoding on the current emissions

Keep the existing model. Replace per-line argmax plus overlays with a document
decode under the same invariants.

Directly targets the 113 sequence gap and disorder defects and improves footnote
placement. Cheaper than A because it needs no retraining and no relabelling, and
it is a prerequisite for A anyway — A's boundaries are only useful if something
consumes them coherently.

**Cost: moderate. Benefit: good, and it is the half of A that carries most of
the value.**

### C. Make classification non-destructive

Stop `HideNoise` from deleting. Route those lines to a suppressed channel that
the generator can still reach, and make furniture removal an explicit, auditable
pass with its own measurement.

Addresses FM1, the dominant remaining recall loss. Structural in the sense that
matters: it removes a whole category of silent-deletion bug rather than fixing
instances of it.

**Cost: low to moderate. Benefit: high, and it makes the next mistake visible
instead of invisible.**

### D. Keep adding guards

What has been done so far. Cheap per fix, fast to ship, and each one measured.

**Cost per defect is rising and the failure rate of the approach is now known:**
of five guards written in this project, one shipped a large regression, one was
inert, and two were reverted after measuring no effect. Explicitly not
recommended as the primary path.

## Recommendation

**Do C first, then B, then A.**

C is the best ratio in the plan: it attacks the largest remaining recall loss,
it is small, and it removes the bug *shape* that produced the two worst defects
found here. Do it with the existing benchmark as the gate.

B next, because it delivers most of A's benefit without retraining and is
required by A regardless.

A last, and only with the assembly rework budgeted alongside it. The measured
lesson from this project is that improving the model without improving what
consumes it makes the product worse.

Two things not on the list. More training data does not help: four retraining
strategies were measured and all lost to the shipped model end to end. Re-OCRing
does not help either: extraction already recovers 98–100% of the text layer, so
there is nothing for OCR to add.

## What "done" looks like

Recall will not reach 100%; running heads and page numbers are removed
deliberately and count against it. On the evidence here the achievable ceiling
is roughly 95%, and the defect budget worth targeting is the 86% attributable to
missing structure. Track both with `tools/bench_compare.py`, which judges
content-recovering changes on defect *rate* and fails on any per-document recall
regression.
