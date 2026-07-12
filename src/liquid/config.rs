//! Configuration constants, limits, model names, and provider definitions.
//!
//! Extracted in Phase 1 of the modularization plan.

pub const LIQUID_SCHEMA_VERSION: u32 = 243;
pub const LIQUID_LAYOUT_MODEL_VERSION: &str = "layout-role-v6-frontmatter-active-20260601+footnote-v18-symbol-publication-sequence-prior-neg4+sequence-heading-repo-lowercase-numeric-body-guard-v1+local-numeric-lowercase-footnote-guard-v1+paragraph-symbol-author-note-repair-v1+late-author-note-run-repair-v1+short-numeric-id-note-repair-v1+lawreview-vol-header-demote-v1+page-less-toc-strip-v1+toc-title-skip-v1+toc-display-mask-v1+short-page-less-toc-strip-v1+lawreview-marginalia-run-repair-v2+citation-note-run-repair-v1+inline-body-fragment-repair-v1+repository-frontmatter-strip-v1+toc-table-title-strip-v1+split-column-toc-strip-v1+lawreview-visible-role-noise-repair-v6+lawreview-heading-fragment-repair-v1+lawreview-table-note-fragment-repair-v2+lawreview-split-marker-repair-v1+profile-after-hints-v1+initial-layout-hint-footnote-protect-v2+citation-continuation-header-guard-v1+biblio-lead-footnote-sequence-v1+publication-citation-bridge-v1+symbol-author-note-sequence-v1+ssrn-repository-noise-guard-v1+layout-noise-role-v1+plain-page-number-marginalia-guard-v1+url-citation-continuation-bridge-v1+header-footer-v1-neg20+running-lawreview-cite-noise-guard-v1+plain-page-number-table-guard-v1+liquid-core-v34-block-geometry-v1+bare-numeric-marker-noise-v1+old-law-small-font-note-run-v1+repository-taxonomy-commons-noise-v1+footnote-specialist-extra-prior-neg6-v1+repository-contact-email-noise-v1+page-context-contents-index-guard-v1+split-toc-page-noise-guard-v1+split-marker-footnote-dotleader-guard-v1+contextual-sequence-repo-bridge-guard-v1+llm-noise-role-prompt-v1+runtime-contents-index-feature-parity-v1+explicit-contents-noise-priority-v1+repository-cover-id-noise-hint-v1+spaced-dotleader-contents-noise-v1+evidence-required-small-font-marginalia-v1+short-form-citation-prev-context-v1+decoded-footnote-run-v1+midpage-indented-note-run-v1+quoted-citation-sequence-guard-v1+quoted-citation-decoder-guard-v1+dense-citation-prelude-row-v1+sequence-citation-row-v1+midpage-numbered-note-run-v1+administrative-status-marginalia-guard-v1+split-legacy-index-page-context-v1+inline-body-citation-guard-v1+inline-note-body-fragment-guard-v1+year-parenthetical-continuation-repair-v1+page-parenthetical-citation-continuation-repair-v1+small-font-sequence-numeric-note-fragment-repair-v1+numeric-note-fragment-geometry-repair-v1+small-font-sequence-citation-material-repair-v1+edge-case-name-running-header-guard-v1+bare-numeric-sequence-marker-context-repair-v1+numeric-enum-note-run-context-repair-v1+small-font-numeric-body-fragment-run-repair-v1+numeric-citation-fragment-run-repair-v1+small-font-note-run-continuation-repair-v1+below-divider-small-font-continuation-repair-v1+nonlegal-study-prompt-noise-v1+orphan-contents-page-fragment-noise-v1+feedback-source-lines-v1+administrative-notice-marginalia-guard-v1+uncited-allcaps-topic-heading-guard-v1+body-specialist-cycle064-heading-hardneg-cycle048-seed-v1+lrev-feature-v1+liquid-margin-notes-v1+runtime-heading-stack-v1+heading-cycle055-faststack-v1+heading-cycle058-expanded-goldnoise-v1+lawreview-masthead-heading-guard-v1+sentence-fragment-heading-guard-v1+allcaps-topic-heading-shape-v1+pagecontents-clear-heading-gate-v1+pagecontents-common-section-heading-v1+pagecontents-abstract-heading-v1";

pub const OPENROUTER_URL: &str = "https://openrouter.ai/api/v1/chat/completions";
pub const OPENROUTER_MODEL: &str = "openai/gpt-oss-120b:free";

pub const GROQ_URL: &str = "https://api.groq.com/openai/v1/chat/completions";
pub const GROQ_MODEL: &str = "openai/gpt-oss-20b";

pub const MAX_LLM_BLOCKS: usize = 260;
pub const MAX_LLM_BLOCK_CHARS: usize = 320;
pub const MAX_LLM_PROMPT_CHARS: usize = 52_000;
pub const TARGET_LLM_PROMPT_BLOCKS: usize = 180;
pub const OPENING_LLM_CONTEXT_BLOCKS: usize = 28;

pub const MAX_READER_AID_SECTION_BLOCKS: usize = 8;
pub const MAX_KEY_TERM_SECTION_BLOCKS: usize = 24;

pub const BETA_REQUIRE_LLM_WHEN_KEY_PRESENT: bool = false;
pub const LLM_LOG_PREVIEW_CHARS: usize = 16_000;

/// When true, write full detailed per-LLM-call JSON logs (including full prompts and
/// response bodies) under AppData/LawPDF/liquid-logs/. Default false to prevent
/// unbounded disk usage from normal Liquid Mode use. Set true only for debugging
/// LLM layout behavior.
pub const ENABLE_DETAILED_LLM_LOGGING: bool = false;
