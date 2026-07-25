# Law review reconstruction benchmark

The primary gate for Review Mode's Markdown output. Replaces the five-article
`agentic-review-corpus`, which was too small and too modern to predict
behaviour on the archive the product actually faces.

## What it is

32 law review articles, stratified across nine journals and roughly a century
of typesetting, selected by `tools/bench_select.py` with seed 20260725 from
17,519 PDFs in `lrscraper_backup`. `lawreview-32-manifest.json` records each
document's source path, journal, page count and note-head count, so the set can
be rebuilt exactly. The PDFs themselves are not in the repository.

Two selection rules, both learned from being burned:

- **No document that appears in the LM2 training corpus.** Two of the five
  articles in the old corpus are in it (047 at 102 of 120 sampled lines, 057 at
  104). That contamination made a retrained model look like it improved
  in-sample while held-out quality halved.
- **Must have a footnote apparatus** — at least 20 small-font lines opening
  like a numbered note. Journal archives are not all articles: the Oregon
  folder holds the *Old Oregon* alumni magazine and a program-evaluation
  report, and South Carolina holds book-review round-ups. Including them drags
  mean recall down 18 points while measuring something reconstruction was never
  aimed at.

## Baseline, v0.2.11

`lawreview-50-baseline-20260725.json` holds the full run, including the 18
non-article documents, for reference.

| Set | Docs | Mean source recall | Worst | Critical defects | Zero footnotes |
|---|---:|---:|---:|---:|---:|
| **Articles (the gate)** | 32 | **88.7%** | 47% | 118 (3.7/doc) | 11 |
| Non-articles (excluded) | 18 | 70.7% | 3% | 58 (3.2/doc) | 14 |

Recall splits sharply by era rather than by journal:

| Journal | Articles | Mean recall |
|---|---:|---:|
| georgetown_law_journal (1912–1943 volumes) | 2 | 0.47 |
| south_carolina_law_review | 3 | 0.86 |
| st_johns_law_review | 2 | 0.87 |
| chicago_law_review | 5 | 0.91 |
| santa_clara_law_review | 4 | 0.91 |
| marquette_law_review | 1 | 0.92 |
| creighton_law_review | 5 | 0.93 |
| loyola_university_chicago_law_journal | 5 | 0.93 |
| mercer_law_review | 5 | 0.94 |

**11 of 32 articles emit no inline footnotes at all**, falling back to an
unlinked notes section because too few body markers are detected.

Top defects: 48 suspected fused paragraphs, 46 source-loss flags, 27 footnote
definitions rendered as body text, 24 footnote sequence gaps.

## Why the earlier numbers were optimistic

| Benchmark | Docs | Mean recall |
|---|---:|---:|
| Original five (modern, born-digital) | 5 | 94.0% |
| Chicago twelve (mixed era) | 12 | 85.7% |
| **This set (stratified, articles only)** | 32 | **88.7%** |

The five-article set reported a 31% defect reduction from two fixes in
v0.2.11 that produced **exactly zero** change on the Chicago twelve. A small
benchmark can move a lot and mean nothing.

## Running it

```
python tools/bench_select.py --index journal-index.json --train train.jsonl \
    --out bench-pdfs --manifest benchmarks/lawreview-32-manifest.json

LAWPDF_MODEL_DIR=profile-models lawpdf --lm2-copy-md --output out/ bench-pdfs/*.pdf
python tools/md_verify.py out/ --source bench-pdfs/ --json report.json
```

Clear `%APPDATA%/LawPDF/liquid2-cache` first, and never run two benchmarks at
once — they share that directory and will corrupt each other's results.
