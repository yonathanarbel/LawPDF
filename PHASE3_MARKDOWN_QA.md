# Copy MD Phase 3 quality audit

Audit date: 2026-07-23

## Outcome

The Copy MD workflow, final generator, footnote-definition rendering, and
navigation affordances work. A strict real-article audit did not produce a
pristine gold output, however: all nine tested documents retain at least one
upstream Review Mode classification or assembly artifact. The closest result is
article 037.

This is intentionally reported as an incomplete Phase 3 quality gate rather
than hidden behind a passing aggregate. The generator defects discovered during
the audit were fixed and tested. Classification defects were not changed, in
accordance with `md_plan.md` Phase 0.

## Generator fixes made from real-article evidence

1. Private marker sentinels are stripped from every final output path, including
   fenced table blocks. This fixes the U+E000/U+E001 leakage observed in article
   007.
2. Lowercase alphabetic subsection labels such as `a.` and `b.` render as H4.
   Article 037 now preserves the expected hierarchy under Part II.B.
3. Blocks beginning with a long PDF footnote separator are omitted from the
   article body, with an explicit export warning. Across corpus articles 011,
   042, 047, and 057, this removed 135 duplicated standalone fragments while
   preserving the linked footnote definitions.

## Real-article matrix

| Article | Result | Footnotes | Principal observation |
|---|---|---:|---|
| 007 | Pipeline failure | no usable links | Front-matter/title and heading roles are wrong; private sentinels exposed the table-path generator defect that is now fixed. |
| 008 | Pipeline failure | no usable links | Title and heading outline contain source furniture; several expected sections are absent. |
| 011 | Known hard case | 167 definitions, 0 unresolved referenced IDs | Outline is navigable and 19 standalone separator fragments are now omitted, but two separators and note fragments remain fused inside body paragraphs. |
| 016 | Pipeline failure | no usable links | Front matter and an agency-reporting/table fragment are classified as headings. |
| 037 | Best available with known artifacts | 312 definitions, 0 unresolved referenced IDs | Lowercase subsection hierarchy is fixed and Part II.B is navigable; unattached note numbers such as 126, 130, and 161 remain at paragraph starts. |
| 040 | Pipeline failure | no usable links | Table-of-contents and citation fragments are classified as headings. |
| 042 | Known hard case | 132 definitions, 0 unresolved referenced IDs | `Massachusetts v. Feeney,` is incorrectly classified as an H2. |
| 047 | Known hard case | 127 definitions, 0 unresolved referenced IDs | Twenty standalone separator fragments are now omitted, but note numbers such as 16, 30, and 131 remain at paragraph starts. |
| 057 | Known hard case | 458 definitions, 0 unresolved referenced IDs | Seventy-six separator fragments are now omitted, but warranty text and citation prose remain false headings. |

“0 unresolved referenced IDs” means every Markdown `[^id]` reference that was
emitted has a definition. It does not mean every superscript in the PDF was
successfully converted to Markdown; the unattached numeric markers above are
the counterexamples.

## Checklist results on the three most navigable outputs

| Check | 011 | 037 | 047 |
|---|---|---|---|
| No standalone page numbers or private marker sentinels | Pass | Pass | Pass |
| No `word-\ncontinuation` dehyphenation artifact | Pass | Pass | Pass |
| Every emitted Markdown footnote reference resolves | Pass | Pass | Pass |
| No footnote artifact interrupts body flow | Fail | Partial | Partial |
| Heading outline is usable | Pass | Partial: Part II.B.1 is body text | Pass |
| Paragraph boundaries are sane | Fail: one 754-word fused paragraph | Pass with long legal paragraphs | Pass |
| Targeted section and footnote can be retrieved | Pass | Pass | Pass |

Navigation spot checks:

- 011 subsection B explains the open/closed model spectrum and argues that open
  systems are valued for innovation, access, diversity, and dispersed power.
  Footnote 9 resolves to Angela Luna's Bipartisan Policy Center article.
- 037 Part II.B predicts that large AI-driven legal-productivity gains could
  increase trials and access to justice while creating uncertain effects for
  lawyer wages, employment, and the human elements of representation. Footnote
  241 resolves to the cited discussion of India's high judicial caseload and
  trial backlog.
- 047 Part II.B analyzes when anti-Israel or anti-Zionist conduct might be
  objectively offensive enough to support a hostile-environment theory under
  Title VI. Footnote 70 resolves to “See supra p. 7.”

## Preview and interaction verification

- The five corpus Markdown files were parsed as Markdown and checked for
  structural headings and resolvable footnote IDs.
- A browser screenshot preview was not available because the local browser
  runtime exposed no installed browser engine. This is an environment
  limitation, not counted as visual QA.
- The toolbar's primary Copy MD control has the required hover text, including
  its shortcut. The attached options control now has its own `Markdown options`
  hover text.

## Evidence pack

`agentic-review-corpus/` contains:

- five original PDFs;
- five final post-Review Mode Markdown exports;
- five one-row-per-source-line CSVs;
- actual local runtime actions, raw emission scores, and action confidence;
- clearly labeled shadow CatBoost probabilities and top-three per-line SHAP
  contributions;
- global standardized PCA coordinates and loading metadata;
- hashes, a manifest, known-artifact labels, and validation instructions.

PCA is used for global variance and clustering context. It is not described as
prediction attribution; the SHAP columns provide the load-bearing features for
the shadow prediction.
