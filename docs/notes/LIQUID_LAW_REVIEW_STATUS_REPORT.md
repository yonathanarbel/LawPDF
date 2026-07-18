# Liquid Mode Law Review Status Report

Workspace: `C:\Users\yonat\Box\Gmailer\lawpdf`

Last updated after the schema 214 law-review footnote repair pass, page-less TOC
strip, v14 footnote-specialist promotion, no-divider citation-run sequence
features, numeric-lowercase body guards, full release test pass, smoke pass, and
refreshed portable build.

## Executive Status

The law-review Liquid path is not finished, but the current build is substantially
better than the earlier baseline. It can usually identify law-review articles,
recover repository-cover titles, move dense footnotes into marginalia, and keep
table-of-contents blocks out of the visible reading flow.

Current embedded model stack:

- All-role layout model: `profile-models\layout-role-v6-frontmatter-active-20260601\layout-role-model.json`
- Footnote specialist: `layout-footnote-v14-context-citation-run-bias-neg32-candidate`
- Header/footer specialist: `header-footer-v1-neg20`
- Liquid model version: `layout-role-v6-frontmatter-active-20260601+footnote-v14-context-citation-run-neg32+sequence-heading-repo-lowercase-numeric-body-guard-v1+local-numeric-lowercase-footnote-guard-v1+paragraph-symbol-author-note-repair-v1+late-author-note-run-repair-v1+short-numeric-id-note-repair-v1+lawreview-vol-header-demote-v1+page-less-toc-strip-v1+toc-title-skip-v1+lawreview-marginalia-run-repair-v2+citation-note-run-repair-v1+inline-body-fragment-repair-v1+repository-frontmatter-strip-v1+toc-table-title-strip-v1+split-column-toc-strip-v1+lawreview-visible-role-noise-repair-v6+lawreview-heading-fragment-repair-v1+lawreview-table-note-fragment-repair-v2+lawreview-split-marker-repair-v1+profile-after-hints-v1+initial-layout-hint-footnote-protect-v2+citation-continuation-header-guard-v1+header-footer-v1-neg20`
- Liquid schema version: `214`

Verification:

- `CARGO_TARGET_DIR=C:\tmp\lawpdf-target-toc cargo test --release`: 263 passed, 0 failed.
- `CARGO_TARGET_DIR=C:\tmp\lawpdf-target-toc cargo build --release`: passed.
- Eight-document law-review smoke: 8 documents, 0 failures.
- Schema 214 hard-smoke deltas versus schema 211:
  - Harvard `047...`: marginalia 390 -> 397, paragraphs 86 -> 80.
  - Harvard `057...`: marginalia 1156 -> 1169, paragraphs 294 -> 286, tables 5 -> 1.
  - `ssrn-5380233.pdf`: unchanged role counts.
- Portable package refreshed:
  - `dist\LawPDF-portable\lawpdf.exe`
  - `dist\LawPDF-windows-portable-x64.zip`

The default Box-synced `target\release` build directory hit Windows filesystem
error 1006 during rebuild, so verification and packaging used the non-synced
target directory above.

The installer was not refreshed because Inno Setup (`ISCC.exe`) is not available
on PATH.

## What Changed

### 1. Table of contents is hidden from Liquid reading flow

Liquid Mode no longer shows table-of-contents/navigation blocks as visible content.
The layout model can still use `contents` internally as a structural role, but
final Liquid blocks with `LiquidBlockRole::Contents` are stripped before display.

This addresses the product issue directly: TOCs add clutter and rarely improve the
reading experience in Liquid Mode.

Related implementation:

- Added trust gates so weak model hints cannot force prose into `Contents`,
  `Table`, `ListItem`, `Metadata`, `Heading`, or `Marginalia`.
- Added final contents stripping after local/profile normalization, including
  LLM-style cases where the heading is marked `contents` but dot-leader entries
  survive as headings or paragraphs.
- Schema 197 expands the final strip and visible outline hider to catch
  table-of-contents clutter that arrives as `Table` blocks or extra `Title`
  blocks from cached/LLM output. This is the class most likely to remain visible
  despite a zero final `contents` role count.
- Added collapse coverage for outline-style TOCs such as `Article Outline`
  followed by plain title/page entries without dot leaders.
- Schema 206 adds split-column TOC collapse for extraction shapes like
  `INTRODUCTION` followed by a separate `2257` block, including law-review
  category rows such as `ARTICLES`.
- Schema 212 adds page-less outline TOC stripping and prevents page-less TOC
  entries such as `Theoretical Background` from becoming the document title.
- Updated TOC/navigation tests to assert these blocks are not visible.

### 1a. Schema 214 law-review footnote polish

Schema 214 adds conservative law-review-only repairs for remaining footnote
surface:

- Paragraph author notes beginning with `*` or `**` and containing law-school,
  university, degree, or academic affiliation cues are converted to marginalia.
- Short numeric citation-note fragments such as `88 Id.` and `346 At that` are
  converted from table/heading/list noise to marginalia.
- Author-note continuation runs after a symbol author note are converted to
  marginalia only when the run is followed by real marginalia, avoiding the
  broader false-positive behavior of a generic second marginalia-run repair.
- Journal volume running headers such as `MERCER LAW REVIEW [Vol. 51` are
  demoted out of table flow.

### 2. All-role layout model v6 is integrated

The all-role model was advanced from v5 to v6 with the new Grok front-matter
candidate labels from `C:\tmp\lawpdf-frontmatter-labels-20260601-023425`.
Those labels added title/metadata/body/footnote coverage on law-review first
pages. The v6 artifact is:

`profile-models\layout-role-v6-frontmatter-active-20260601\layout-role-model.json`

Training used the same 490-PDF cached corpus and refreshed derived
geometry/sequence roles with the runtime strong-cue gate:

- Training corpus: 490 PDFs
- Extracted lines: 158,572
- Matched gold/Grok labels: 1,021
- Changed labels during training pass: 338

Silver-label report:

- Accuracy: 0.7944
- Macro F1: 0.6374
- Strong roles: body, footnote, contents
- The silver score dipped versus v5, so adoption was gated on heldout and smoke.

Fast heldout comparison on 185 matched law-review labels:

| Model | Accuracy | Macro F1 |
| --- | ---: | ---: |
| Previous embedded v5 all-role | 0.8811 | 0.7710 |
| Current v6 front-matter active | 0.8865 | 0.7888 |

Footnote F1 stayed stable at 0.9429 on this heldout slice. The production
decision is justified by improved heldout accuracy/macro F1 and stable
law-review smoke results, despite the lower broad silver score.

Schema 170 fixed runtime feature parity for the embedded model. The Python
trainer already used rich geometry/sequence tokens such as `line_index`,
`front_matter_top_band`, `first_page_title_band`, `after_front_matter`,
`sequence_footnote_zone`, repeated edge text, probable author lines, and late
centered prose bands. The Rust runtime now emits those same computable tokens for
the all-role model and specialists, so the model sees the feature space it was
trained on.

Schema 176 also carries true Pdfium font metadata into the runtime line model:
scaled font size, bold, and italic. Runtime line extraction now prefers real PDF
font size over bounding-box height and emits `is_bold` / `is_italic` tokens to
match the trainer. This initially made the model too aggressive on body lines, so
the marginalia gates were tightened: a model-predicted footnote still needs
plausible footnote geometry, such as a divider, lower note band, sequence note
zone, small note-sized font, or real legal-note cues. A bare mid-page inline note
marker like `5 And the` no longer starts a footnote zone.

### 3. Law-review footnotes are much stronger

The v14 footnote specialist is the current footnote model. It keeps the hard
law-review page labels from v13, relaxes the prior to `footnote=-32`, and adds
context citation-run sequence features for no-divider law-review pages. It now
recognizes normal spaced note starts such as `5 See,e.g.`, source-citation starts
such as `106 Arthur C. Graesser et al., ...`, and citation-continuation text that
is confirmed by a nearby real note marker.

The important gate change is that no-divider footnote sequences can now start
from compact legal note markers in small-font note zones, including upper/mid
page note zones where no divider was detected. Weak legal-looking text such as
`U.S.` still does not start a sequence by itself.

Schema 191 adds a second no-divider path for small-font general citation note
starts such as `164 Aaron Smith ... PEW RES.CTR.`. The runtime and trainer now
emit `general_citation_note_start` and
`geom_no_divider_general_citation_note_start` feature tokens. This fixes a
previous visible run in `ssrn-3313837` where footnote 164 appeared as
`ListItem/Table/Subheading/Header` before marginalia resumed.

Schema 195 added two follow-up guards from the same hard page:

- inline body-fragment repair removes `117 For` and restores the following body
  paragraph to `... text. For example, an F-K score ...`
- citation-continuation header guards stop URL-heavy or reporter-heavy note
  continuations from being promoted to `Header`; on `ssrn-3313837`, visible
  headers dropped from 9 to 2 and visible tables dropped from 1 to 0.

Heldout comparison on the 185 matched full-context labels:

| Footnote specialist | Accuracy | Macro F1 | Footnote precision | Footnote recall |
| --- | ---: | ---: | ---: | ---: |
| v4 active | 0.9784 | 0.9631 | 1.0000 | 0.8857 |
| v7 strong-cue gate | 0.9838 | 0.9738 | 0.9444 | 0.9714 |

The two v7 false positives in that heldout set were repository-cover boilerplate
that the runtime already blocks before adding marginalia hints.

Latest v14 training/evaluation report:

- Training corpus: 167,302 extracted lines after merging full hard law-review
  PDFs into the cached examples.
- Hard-page labels: 504 matched labels.
- Broad refreshed-silver test: accuracy 0.9580, macro F1 0.9315, footnote
  precision/recall/F1 0.998 / 0.801 / 0.889.
- Hard-label comparison versus v13: footnote recall 0.6093 -> 0.7993 and
  footnote F1 0.7572 -> 0.8884 with zero hard false positives.
- All-gold comparison versus v13: footnote recall 0.6266 -> 0.8070 and
  footnote F1 0.7615 -> 0.8839.

The broad refreshed-silver score is not directly comparable to v13 because the
sequence pass now labels many more no-divider citation runs as silver footnotes.
Promotion is based on hard-label and smoke behavior. Runtime guards prevent the
main observed v14 false positives: repository citation cover lines, section
headings with `v.` in case names, lowercase citation prose, and numeric lowercase
body fragments such as `13 bankruptcy protection...`.

A v10 candidate was trained with the new general-citation-note feature:

`profile-models\layout-footnote-v10-general-citation-note-bias-neg48-candidate`

It correctly predicted the `164 Aaron...` and `19 2016), http://...` starts on
the target page, but it was not promoted because its gold footnote F1 was lower
than v9 on the then-current gold slice. A combined-label v11 candidate was also
trained later, and v12 became the prior production footnote specialist before the
hard-page v13 promotion.

Useful behavior now:

- Real law-review notes become marginalia.
- Inline note-reference fragments are rejected from marginalia.
- Footnote divider geometry is used.
- Small-font note zones are used.
- Header/footer lines are less likely to contaminate the reading flow.
- Interrupted law-review marginalia runs are repaired when PDF extraction drops
  terminal hyphens or temporarily classifies short continuation lines as body or
  table blocks.
- Small-font general citation-note starts can begin a no-divider sequence before
  the lower-page note band when followed by citation continuations.

### 3a. Inline superscript body-flow repair

Yale Forum exposed a different class of footnote-related failure: the footnotes
were correctly detected, but Pdfium sometimes extracted superscript note
fragments before their body anchor line. Examples included:

- `3 Recycling the` appearing before `makes it worth...`
- `5 And the` appearing before `the first offer...`
- `11 Sure,` being promoted to a heading

The paragraph splitter now delays short inline-note fragments and appends them
after the next body anchor line when the surrounding text shows the current
sentence is unfinished. It also handles very short page-start fragments such as
`11 Sure,` while still excluding citation footnotes.

Regression coverage added:

- normal inline note-marker cleanup remains intact
- Pdfium superscript fragments before anchor lines are reordered
- page-start superscript fragments are reordered
- false marginalia/list-item hints can no longer force those fragments out of
  body flow

### 3b. Prose-heading demotion for forum letters

Yale Forum also exposed ordinary prose promoted to headings:

- `article was getting a board read, and asked for two weeks to make a decision.`
- `article in their next issue.`
- `Sincerely,`

The root cause was not the footnote classifier itself. It was local heading
classification:

- any text beginning with `article ` was uppercased and treated as a legal
  `ARTICLE` heading
- short title-cased closings such as `Sincerely,` satisfied the generic
  `title_like` heading rule

The classifier now distinguishes structural legal headings from ordinary prose.
`Article 5` and `SECTION 2.1 Definitions` still classify as headings, but
lowercase narrative `article ...` continuations do not. Common letter closings
now stay in paragraph flow.

Regression coverage added:

- lowercase `article ...` prose is not a heading
- `Sincerely,` is not a heading
- structural legal headings are preserved
- a Yale-style excerpt keeps the article continuation and closing out of the
  outline

### 4. Repository-cover handling is better

The title/front-matter path now handles common repository and citation covers:

- SSRN/forum letters
- Santa Clara repository covers
- Loyola eCommons / LAW eCommons
- Mercer Law Review short quoted titles
- South Carolina Scholar Commons recent decisions
- Marquette memorial pieces

Repository boilerplate such as `Scholar Commons`, `eCommons`, authorized
administrator language, and repository email lines is generally demoted to
metadata or removed from the main reading flow.

### 5. Grok is now part of the execution loop

Grok labeling/review work is useful when tasks are narrow and isolated. Current
safe pattern:

```powershell
grok -p "clear narrow task..." --always-approve
```

To avoid worktree races, Grok tasks are run in `C:\tmp\lawpdf-grok-parallel-*`
and instructed to write reports or labels there, not to edit production files.

The active parallel task at the time of this update was:

`C:\tmp\lawpdf-grok-parallel-20260531-234618\body-flow-reflow-implementation`

That task produced an implementation-grade plan for the remaining `And the
the...` body-flow artifact. A follow-on Grok label candidate task was started at:

`C:\tmp\lawpdf-grok-parallel-20260601-000310\body-flow-label-candidates`

Another isolated Grok audit was launched at:

`C:\tmp\lawpdf-grok-parallel-20260601-001500\layout-runtime-feature-audit`

The latest isolated Grok audit completed at:

`C:\tmp\lawpdf-grok-parallel-20260601-remaining-lawreview-clutter\remaining-lawreview-clutter-audit.md`

It confirmed the current next targets: repository/front-matter boilerplate that
still reaches visible roles, and mixed body/footnote extraction fragments around
short no-divider note zones.

Grok also produced hard-zone labels at:

`C:\tmp\lawpdf-grok-parallel-20260601-body-note-frontmatter-labels\hard-zone-labels.json`

Those labels covered the `117 For` body-flow fragment, `44 That is...` header
leak, several heading-pollution fragments, and repository cover clutter. A Grok
worker drafted a repository/front-matter cleanup policy at:

`C:\tmp\lawpdf-grok-parallel-20260601-repository-frontmatter-policy`

An isolated Grok audit focused on table-of-contents clutter and is at:

`C:\tmp\lawpdf-grok-parallel-20260601-toc-clutter-audit\toc-clutter-audit.md`

That audit specifically called out table-tagged TOCs and title-tagged TOC
entries; both are now covered by schema 197+ regression tests.

The latest TOC suppression audit is at:

`C:\tmp\lawpdf-grok-parallel-20260601-toc-suppression-audit\toc-suppression-audit.md`

Schema 206 implements its highest-confidence local gap: split-column TOC rows
where the title and page number are extracted as separate blocks.

Another isolated Grok audit focused on remaining footnote false negatives at:

`C:\tmp\lawpdf-grok-parallel-20260601-footnote-v11-fn-audit\footnote-v11-fn-audit.md`

It identified mid-page no-divider general citation starts and compact legal note
markers as the safest next feature target. Schema 198 implements that feature
parity in both Rust runtime features and the Python trainer, then promotes the
v12 footnote specialist.

The next Grok audit focused on Mercer-style visible block clutter at:

`C:\tmp\lawpdf-grok-parallel-20260601-mercer-block-repair-audit\mercer-block-repair-audit.md`

Schema 200 implements the useful part of that audit as a law-review-only
normalization pass: page/volume artifacts are hidden as headers, legal-prose
fragments stop rendering as headings/callouts/tables, and standalone note-like
fragments become marginalia while preserving real enumerated list items.

A follow-up Grok audit focused on `ssrn-3313837` heading/subheading/issue/takeaway
pollution at:

`C:\tmp\lawpdf-grok-parallel-20260601-ssrn-heading-fragment-audit\ssrn-heading-fragment-audit.md`

Schemas 201 through 204 implement the high-confidence part of that audit:
duplicate title headings are hidden, body question fragments are demoted to
paragraphs, citation tails and citation-title continuations become marginalia,
lettered question headings become subheadings instead of `Issue`, table rows stop
polluting the outline, symbol author notes become marginalia, and split numeric
note markers such as `1 33. As...` are repaired.

Another Grok worker produced the next hard-page candidate list at:

`C:\tmp\lawpdf-grok-parallel-20260601-footnote-hard-pages\hard-footnote-pages.md`

## Current Smoke Status

Latest focused smoke:

`C:\tmp\lawreview-regression-schema211-footnote-v14-guard-smoke.json`

Latest `ssrn-3313837` result is included in that schema 211 regression smoke.

Outcome for `C:\Users\yonat\Downloads\ssrn-3313837.pdf`:

- Title: `THE DUTY TO READ THE UNREADABLE`
- Profile: `law_review_article`
- Visible contents blocks: none
- Marginalia blocks: 865, up from 806 in schema 206, 798 in schema 200, and
  689 before the broader
  no-divider/continuation repair.
- Paragraph blocks: 112.
- Heading blocks: 4, down from 14 in schema 200. The remaining samples are the
  main section headings plus `Eligibility:`.
- Issue and takeaway blocks: none, down from 2 `Issue` blocks and 1 `Takeaway`
  block in schema 200.
- Table blocks: 3. These are actual extracted website-statistics table rows
  rather than outline headings.
- Visible headers: 3 (`THE DUTY TO READ THE UNREADABLE`, `INTRODUCTION`,
  `CONCLUSION`), where the duplicate title is now hidden from heading/outline
  display.
- Diagnostic layout hints still found 18 `contents` lines, which is expected;
  they are now stripped from the final Liquid block list.

Outcome for `C:\Users\yonat\Downloads\ssrn-3912101.pdf`:

- Title: `Letter to the Yale Law Journal Forum`
- Profile: `law_review_article`
- Visible contents blocks: none
- Marginalia samples: correct author note and citation notes
- Fixed: `Recycling the makes...`, `And the the first offer...`, `11 Sure,`,
  `article was getting...`, `article in their next issue`, and `Sincerely,`
  as standalone headings are no longer present in fresh schema-200 smoke.
- Current schema-211 Yale smoke has zero heading blocks, 43 marginalia blocks,
  and no visible contents blocks.

Latest law-review regression smoke:

`C:\tmp\lawreview-regression-schema211-footnote-v14-guard-smoke.json`

Summary:

- Documents: 5
- Failures: 0
- Contents blocks: none in all checked outputs

| PDF | Title | Profile | Marginalia | Notes |
| --- | --- | --- | ---: | --- |
| `ssrn-3313837.pdf` | `THE DUTY TO READ THE UNREADABLE` | law_review_article | 865 | v14 recovers more no-divider citation-run footnotes; heading pollution remains down 14->4 since schema 200; issue/takeaway gone. |
| `ssrn-3912101.pdf` | `Letter to the Yale Law Journal Forum` | law_review_article | 43 | v14 recovers additional forum-letter footnotes; TOC hidden; Yale body-flow repair still holds. |
| Loyola article | Full title recovered | law_review_article | 1064 | v14 recovers more note continuations; eCommons cover scaffolding is mostly stripped/demoted; starred author note and split numeric notes repaired. |
| Mercer article | `Bankruptcy` | law_review_article | 408 | v14 recovers more author notes and numbered bankruptcy footnotes; numeric-lowercase body guard keeps `13 bankruptcy protection...` in paragraph flow. |
| Marquette memorial | `In Memoriam: Professor A. C. Umbreit` | law_review_article | 1 | Low-note memorial handled. |

Known excluded regression:

- The Chicago/Judicial Function book-review PDF still needs a separate policy.
  It behaves more like a partial law-review book review than a normal article.

## Current Capabilities

Liquid Mode currently can:

- Classify many law-review articles as `law_review_article`.
- Route dense footnotes into marginalia.
- Hide table-of-contents/navigation blocks from the reading flow.
- Use PDF geometry/font features and embedded layout-role models.
- Use runtime geometry/sequence features that now match the trainer for the
  computable layout tokens.
- Use Pdfium font size, bold, and italic metadata in runtime layout-role
  prediction.
- Emit layout-hint samples in smoke JSON reports for faster model audits.
- Recover titles from repository covers and citation front matter.
- Demote common repository boilerplate.
- Use local layout without requiring an LLM key.
- Use LLM layout mode with structured output, timeout, and retry safeguards when
  provider keys are configured.
- Run smoke tests on arbitrary PDFs:

```powershell
C:\tmp\lawpdf-target\release\lawpdf.exe --smoke-liquid --liquid-smoke-output C:\tmp\out.json <pdfs...>
```

## Remaining Weak Points

### 1. Mixed body/footnote extraction is the highest-yield defect

The model now keeps many notes out of the body, and the older Yale Forum
inline-note artifacts no longer appear in fresh schema-200 smoke. The current
weakness is narrower: some no-divider law-review pages still produce mixed
body/footnote prose around dense note zones.

Recent `ssrn-3313837` fixes:

- `117 For` no longer appears as a `Table` fragment.
- `44 That is...` no longer appears as a `Header`; the marker is stripped and
  the mixed block is demoted to paragraph flow.
- `101 See, e.g., ReadabilityStatistics...` is now marginalia instead of
  `Header`.
- `Google, Facebook...`, `United States Supreme Court3`, `95NY-YQB6].`,
  long citation-title subheadings, and `50 Sign-in-wraps...` no longer appear as
  visible structural roles in schema 211 smoke.
- `Mean Median Standard Deviation` and related website-statistics rows now render
  as `Table` instead of outline headings.

Remaining `ssrn-3313837` examples:

- citation continuations can still remain in paragraph flow when the PDF
  extraction has merged body prose and note prose into a single long line
  (for example the `44 That is...` mixed paragraph).
- `Eligibility:` still appears as a heading-like label. It is much lower impact
  than the earlier citation/body fragments but remains a cleanup candidate.

These look like PDF extraction/order artifacts rather than ordinary
classification errors. The next pass should focus on local run repair around
merged body/note prose and on the hard no-divider pages identified in the Grok
hard-page report.

### 2. Law-review subtypes need policy

Book reviews, recent decisions, memorials, symposium forewords, and short forum
letters should share the law-review reading experience but may need different
title/profile rules.

Useful broad subtypes:

- `law_review_article`
- `law_review_note_or_comment`
- `law_review_book_review`
- `law_review_recent_decision`
- `law_review_forum_or_letter`
- `law_review_memorial_or_tribute`

### 3. `src\liquid\mod.rs` remains too large

The LLM layout path was moved out, but `mod.rs` still owns title extraction,
repository-cover parsing, layout-hint trust policy, inline-note repair, many
predicates, orchestration, and a large test block.

High-value module split:

- `title_extraction.rs`
- `repository_cover.rs`
- `inline_notes.rs`
- `layout_hint_policy.rs`
- smaller law-review test fixture modules

### 4. More labels should be targeted, not random

The next labels should focus on law-review flow failures:

- `body`
- `body_continuation`
- `inline_note_fragment`
- `footnote_start`
- `footnote_continuation`
- `title`
- `author`
- `metadata`
- `contents`
- `heading`
- `header_footer`
- `caption_table`

Broad random-PDF labeling is lower yield until the law-review reading path is
stable.

## Recommended Next Step

Do one focused iteration on law-review clutter quality:

1. Use Grok workers to label hard zones from 40 to 80 law-review PDFs:
   first pages, footnote divider pages, inline-note artifacts, and body lines near
   footnote zones.
2. Add deterministic regression tests for the hard-page examples in
   `hard-footnote-pages.md`, especially no-divider Harvard Law Review pages and
   split marker/body fragments.
3. Train the next specialist on `footnote_start`, `footnote_continuation`,
   `body_near_footnote`, and `inline_note_fragment`, using the existing geometry,
   font-size, divider, and sequence features.
4. Re-run the current law-review smoke set and compare body text, title,
   profile, marginalia, metadata, and hidden contents behavior.
5. Only then broaden document subtype work beyond law-review articles.

Bottom line: the current build is test-clean and packaged. TOC clutter is removed.
The highest-yield next work is not broad classification; it is law-review clutter
polish around mixed extraction fragments and targeted no-divider footnote labels.
