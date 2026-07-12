# Liquid Mode Revision Plan

This is the current audit plan after the Grok modularization pass.

## What Is Stable Now

- `model.rs`, `config.rs`, `util.rs`, `cleaning.rs`, `classification.rs`, `normalization.rs`, `cache.rs`, `paragraphs.rs`, and `llm/` are active modules.
- `llm/layout.rs` owns the optional LLM layout application path.
- `mod.rs` still owns orchestration, title selection, shared reference/title predicates, and most tests.
- The public API used by `app.rs` remains `LiquidBlock`, `LiquidBlockRole`, `LiquidDocument`, `LiquidEvent`, `LiquidRequest`, and `spawn_liquid_job`.
- `cargo check` and `cargo test` pass. On Box-synced worktrees, use a temp `CARGO_TARGET_DIR` to avoid external target-file mutation during compilation.
- `--smoke-liquid` gives a repeatable real-PDF audit path.

## Issues Fixed In This Pass

- Removed invalid half-extracted modules that shadowed real functions and broke compilation.
- Restored missing normalization behavior for table-of-contents hiding, adjacent figure source captions, and key-term folding.
- Fixed a multibyte slicing panic in sentence splitting.
- Stripped embedded control characters from PDFium text.
- Repaired common PDF extraction mojibake for symbols and typographic punctuation.
- Bumped the Liquid cache schema after cleaning changes.
- Kept footnote notes distinct from hidden headers, footers, and contents.
- Added a CLI smoke harness for real-PDF Liquid checks.
- Extracted classification and label derivation into `classification.rs` as a verified move.
- Extracted the ordered local normalization pipeline into `normalization.rs` behind `run_local_normalization`.
- Extracted Liquid cache signatures and load/save helpers into `cache.rs`.
- Extracted paragraph and sentence splitting into `paragraphs.rs`, including glued-heading splitting and abbreviation-aware sentence scans.
- Extracted LLM layout application into `llm/layout.rs` and split it into request construction, logging, response parsing, and block reconstruction.
- Added an explicit blocking HTTP timeout for LLM requests.
- Replaced corpus-specific first-name and publication-name title exceptions with structural heuristics.
- Added local document profile classification with cached `DocumentProfile` output and smoke-report visibility.
- Added `--profile-dataset` to sample local PDFs into a labeling manifest, with optional bounded local prediction for profile review.
- Tightened title/front-matter selection based on the 10-PDF smoke pass: split titles, weak PDF metadata titles, citation-embedded titles, repeated book-title fragments, memo sender lines, and running headers now have regression tests.
- Audited a second random 10-PDF sample from the local Gmailer folder and tightened weak metadata title fallback, identifier-like filename handling, address/date/salutation/bullet title rejection, marginalia-vs-title separation, and table-of-contents hiding after abstracts.
- Audited a cross-folder 10-PDF sample from separate top-level folders. The final run had 9 usable outputs and 1 external failure for a password-protected PDF.
- Added title-only Liquid output for image-only PDFs with no selectable native text, so scanned documents no longer fail before the OCR workflow.
- Tightened cross-folder findings: journal-header fallback is now bounded so dated captions, contract fields, and ORCID URLs do not become titles; generic `EXHIBIT A` labels no longer override substantive agreement titles.
- Bumped the Liquid cache schema after title-selection changes.
- Re-ran the 10-PDF smoke pass against fresh temp copies so existing Liquid cache entries could not mask the extracted pipeline.

## Larger Follow-Up Plan

1. Extract orchestration last.
   Move `prepare_liquid_document` only after title selection and shared predicates have stable homes.

2. Add a golden-file fixture set.
   Promote representative outputs from the 10-PDF smoke pass into small text fixtures, not full private PDFs.

3. Treat image-only PDFs as an OCR workflow.
   The smoke pass found scanned PDFs with zero native text. Liquid should offer a direct OCR-first path rather than only reporting no usable text.

4. Consider geometry-aware Liquid extraction.
   PDFium character boxes already exist elsewhere in the app. Using them for headers, footers, columns, and footnotes is the next major quality jump.

5. Train and calibrate the profile model.
   `tools/profile_training.py` now trains small CPU models, exports model JSON, and creates active-learning queues under `to-evaluate/`. The next quality step is replacing silver labels with human-reviewed labels from those queues.
