Write a file called ideas.md in this repository.

You are reviewing LawPDF's Liquid Mode layout-role system. The product goal is a very smooth law-review article reading experience. Law review PDFs usually contain title/front matter, abstract, body text, headings, footnotes, page headers/footers, table-of-contents pages, repository cover boilerplate, and citation metadata. In Liquid Mode, the most important behavior is:

- Main body prose should remain in the main reading flow.
- Footnotes/endnotes/author notes/citation notes should become marginalia/notes, not be merged into body paragraphs.
- Titles, authors, abstracts, and section headings should be preserved as useful structure.
- Table of contents rows, running headers/footers, page numbers, repository cover clutter, SSRN download lines, dot-leader rows, and OCR/layout junk should be hidden as noise.
- We do not need perfect fine-grained classification of junk; for Liquid Mode, junk can all become hidden noise.

The current model system is CPU-friendly and mostly line-level. It uses PDF geometry and typography, not just text. The main implementation is in:

- src/layout_roles.rs: runtime layout extraction, feature generation, deterministic guards, and model hint application.
- tools/layout_role_training.py: offline training/evaluation harness with mirrored feature logic.
- src/liquid/config.rs: Liquid schema/model version.

Current runtime architecture:

1. Extract physical text lines from PDF text chars.
2. Compute line geometry:
   - page index
   - line index
   - x0/x1/y0/y1 ratios
   - width ratio
   - font size
   - font ratio relative to page median
   - font ratio relative to page 75th percentile/reference font
   - font ratio relative to document median
   - bold/italic
   - centered
3. Detect footnote divider geometry:
   - page_has_footnote_divider
   - below_footnote_divider
   - distance_below_divider
4. Detect repeated edge text/header-footer candidates.
5. Detect contents/index pages and entries:
   - dot leader rows
   - spaced dot leader fragments
   - page_contents_like
   - contents_or_index_entry
6. Detect repository and SSRN cover boilerplate:
   - recommended citation
   - repository citation
   - volume/issue/article identifiers
   - scholarship URLs, SSRN copies, DOI-like metadata, contact boilerplate
7. Detect footnote/citation cues:
   - numeric note markers
   - bare numeric note marker
   - compact legal note marker like "12See ..."
   - legal note marker
   - symbol author-note marker
   - general citation note start
   - citation continuation text
   - publication citation continuation text
   - bibliographic lead text
   - short-form citation cue, recently added: leading/internal Id./Ibid. variants
8. Detect no-divider footnote sequences:
   - sequence_footnote_zone
   - split numeric marker sequences
   - small-font legal note runs
   - contextual citation footnote zones
   - bibliographic lead before marker
   - URL citation continuation bridge
9. Recently added label-free previous-line context features:
   - prev_line_present
   - prev_sequence_footnote_zone
   - prev_below_footnote_divider
   - prev_small_font
   - prev_note_marker
   - prev_legal_note_cue
   - prev_y_gap bucket
   - prev_left_delta bucket
   - prev_font_delta bucket
   - prev_context_footnote_continuation
   These are intentionally based only on inference-available geometry/derived cues, not gold labels.
10. Models:
   - main layout-role model: title/front matter/body/heading/noise/etc.
   - liquid-core model: collapsed Liquid roles, especially keep/noise/footnote-related roles.
   - footnote specialist model: binary footnote vs not-footnote.
   - deterministic runtime gates decide final Liquid action: hide_noise, marginalia, or keep.

Feature examples currently emitted to hashed Naive Bayes model:

- Text tokens: first word, last word, top content tokens.
- Page and zone buckets: page=0/1/2/4/9, y0 buckets, x0 buckets, width buckets, line_index buckets.
- Font buckets: fs_page, fs_page_ref, fs_doc.
- Word count bucket, uppercase-ratio bucket.
- Style: is_bold, is_italic, is_centered.
- Divider: page_has_footnote_divider, below_footnote_divider, dist_divider bucket.
- Sequence/context: sequence_footnote_zone, previous-line context tokens.
- Legal/citation text: note_marker, bare_numeric_note_marker, compact_legal_note_marker, legal_note_marker, general_citation_note_start, citation_continuation_text, publication_citation_text, bibliographic_lead_text, short_form_citation_cue.
- Contents/noise: page_contents_like, contents_or_index_entry, page_contents_entry, dot_leader_contents, plain_page_number_line, edge_plain_page_number, running_law_review_cite_text, edge_running_law_review_cite.
- Geometry interaction tokens:
  - geom_below_divider_note_font
  - geom_no_divider_note_start
  - geom_no_divider_compact_note_start
  - geom_no_divider_legal_marker_note_start
  - geom_no_divider_general_citation_note_start
  - geom_no_divider_general_citation_note_start_relaxed
  - geom_no_divider_general_cite_midpage
  - geom_no_divider_compact_see_midpage
  - geom_no_divider_legal_note
  - geom_first_page_symbol_author_note
  - geom_no_divider_bibliographic_lead
  - geom_no_divider_sequence_note
  - geom_no_divider_small_mid_body

Important recent engineering changes:

- Removed an overly broad fallback that treated small lower-page text as marginalia without enough note evidence. This improved marginalia precision but reduced recall.
- Added deterministic noise handling for table-of-contents rows, spaced dot-leader contents pages, and repository cover identifiers.
- Added short-form citation cue handling for leading Id./Ibid. so lines like "Id. at 418-19..." are treated as legal note cues.
- Added previous-line context features, but only as inference-safe derived context, not previous gold/predicted label leakage.
- Schema was bumped to 241 for "short-form-citation-prev-context-v1".

Performance snapshot:

The key metric is strict_law_review_runtime_ensemble_action_gold_eval, because it measures actual Liquid action behavior on audited hard-case Grok/gold labels for law-review documents.

Before schema 241:

- Total audited lines: 8,687
- Accuracy: 87.07%
- Macro F1: 87.24%
- hide_noise: precision 100.0%, recall 83.3%, F1 90.9%
- marginalia: precision 92.7%, recall 87.0%, F1 89.7%
- keep: precision 73.9%, recall 90.0%, F1 81.1%

Schema 241 with existing active models:

- Accuracy: 87.38%
- Macro F1: 87.49%
- hide_noise: precision 100.0%, recall 83.2%, F1 90.8%
- marginalia: precision 92.7%, recall 87.7%, F1 90.1%
- keep: precision 74.5%, recall 90.0%, F1 81.5%

Schema 241 with a retrained liquid-core candidate v32:

- Accuracy: 87.42%
- Macro F1: 87.53%
- hide_noise: precision 100.0%, recall 83.4%, F1 90.9%
- marginalia: precision 92.7%, recall 87.7%, F1 90.1%
- keep: precision 74.6%, recall 90.0%, F1 81.5%

A footnote-specialist candidate v33 was trained with the new previous-context and Id./Ibid. features. Its standalone binary footnote metrics looked strong:

- Overall binary eval: accuracy 98.81%, macro F1 98.24%
- footnote precision 95.4%, recall 99.2%, F1 97.2%
- strict law-review binary eval: accuracy 98.56%, macro F1 98.33%
- strict footnote precision 96.3%, recall 99.2%, F1 97.7%

But v33 has not yet been scored as the runtime footnote-specialist replacement in the final ensemble, so do not assume it improves Liquid actions until runtime-scored.

Known failure modes:

1. Missed real footnotes kept in main text.
   - Especially old law reviews with no visible divider.
   - Footnote continuations that do not start with a number.
   - Short-form citations like "Id. at ..." were a known miss; recently patched.
   - Bibliographic continuation lines can be hard if they are mid-page or look like prose.
   - Author-note acknowledgements on page 1 often lack numeric markers and look like metadata/front matter.

2. Main body falsely converted to marginalia.
   - Body lines with inline citations, small font, or lower-page location can look like footnotes.
   - Some body quotations or block quote fragments are smaller font and lower on page.
   - Some pages have no divider and a footnote sequence detector can start too early.

3. Noise vs marginalia conflicts.
   - Repository cover citations near the bottom of page 1 can look like footnotes.
   - Page numbers and running law-review citations near footnote zones can be confused unless explicitly guarded.
   - Contents/index rows near the bottom of a page can look footnote-like if only position/font are used.

4. Heading/front matter is weaker than body/footnote.
   - The broad role metrics show heading/front_matter are still noisy, but for Liquid Mode this matters less than keep/marginalia/hide actions.

5. Model/feature limitations.
   - Current model is hashed Naive Bayes; adding tokens can cause hash collisions and requires retraining/evaluation.
   - The system has many deterministic gates and specialized heuristics; model probabilities are not deeply calibrated.
   - The previous-line feature is only one-line context. It does not yet model full multi-line blocks/runs with a CRF/HMM/sequence decoder.
   - Current features are mostly per-line plus previous-line. They do not fully use next-line context, column segmentation, paragraph grouping, or page-level distribution statistics beyond contents/repeated-header logic.
   - There is no explicit confidence-margin feature that says "model was uncertain, defer to conservative keep."

Current best guess about where gains are:

- More broad random data is lower yield than targeted residual labels.
- Most valuable labels are hard residuals:
  - gold footnote predicted keep
  - gold body predicted marginalia
  - repository/contents/page-number false marginalia
- We care more about action metrics than fine-grained role accuracy.
- For user experience, marginalia recall matters, but false body-to-marginalia is visually bad, so precision cannot collapse.

Your task:

Write ideas.md with the 3 highest-profit feature improvements we can make next to further improve law-review Liquid Mode. Focus on features or modeling changes, not generic "collect more data" unless the data collection is tied to a specific feature. For each idea include:

1. What feature/modeling change to implement.
2. Why it targets the current measured failures.
3. How to implement it in Rust runtime and Python trainer/evaluator.
4. What tests/evaluation should prove it works.
5. Risks and likely false positives.
6. Expected performance effect, with a rough guess of whether it helps hide_noise, marginalia, or keep.

Be concrete and technical. Do not write a vague brainstorming list. Assume we can edit Rust and Python, train local CPU models, and call Grok/Claude for labels, but final runtime must be local and fast.
