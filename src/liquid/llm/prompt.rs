//! Prompt construction for the LLM layout pass.
//!
//! High-value target for isolated testing (prompt text, block selection, compacting).
//!
//! Contains compact system and user prompt builders that only request fields the
//! pipeline actually honors (role/type, label, action=remove, visual_break_before).
//! Removed all references to unsupported inline markup tags
//! (<bold>, <italics>, <underline>) and box/bkground_color/text_color (renderer +
//! pdfium extraction provide none; only visual_break_before and role changes are applied).

use std::collections::HashSet;

use crate::liquid::model::{LiquidBlock, LiquidBlockRole};
use crate::liquid::util::compact_for_prompt;

pub(crate) struct LlmPromptInput {
    pub text: String,
    pub count: usize,
}

pub(crate) fn build_llm_prompt_input(blocks: &[LiquidBlock]) -> LlmPromptInput {
    let mut text = String::new();
    let mut count = 0usize;

    for index in select_llm_prompt_indices(blocks) {
        let block = &blocks[index];
        let snippet = compact_for_prompt(&block.text, prompt_snippet_limit(block.role));
        let entry = format!(
            "{index}|{}|{}w|{}\n",
            block.role.prompt_name(),
            crate::liquid::util::word_count(&block.text),
            snippet
        );
        if text.len() + entry.len() > crate::liquid::config::MAX_LLM_PROMPT_CHARS {
            continue;
        }
        text.push_str(&entry);
        count += 1;
    }

    LlmPromptInput { text, count }
}

fn select_llm_prompt_indices(blocks: &[LiquidBlock]) -> Vec<usize> {
    let budget =
        crate::liquid::config::TARGET_LLM_PROMPT_BLOCKS.min(crate::liquid::config::MAX_LLM_BLOCKS);
    if budget == 0 {
        return Vec::new();
    }

    let visible = blocks
        .iter()
        .enumerate()
        .skip(1)
        .filter_map(|(index, block)| (!should_skip_llm_prompt_block(block.role)).then_some(index))
        .collect::<Vec<_>>();
    if visible.len() <= budget {
        return visible;
    }

    let mut selected = Vec::with_capacity(budget);
    let mut seen = HashSet::new();
    for index in visible
        .iter()
        .take(crate::liquid::config::OPENING_LLM_CONTEXT_BLOCKS)
        .copied()
    {
        push_unique_prompt_index(index, &mut selected, &mut seen);
    }

    // Prioritize structural/important roles for the LLM prompt budget
    for index in visible.iter().copied() {
        if is_structural_prompt_block(blocks[index].role) {
            push_unique_prompt_index(index, &mut selected, &mut seen);
        }
    }

    if selected.len() > budget {
        selected.sort_unstable();
        selected.dedup();
        return evenly_sample_indices(&selected, budget);
    }

    let remaining = visible
        .into_iter()
        .filter(|index| !seen.contains(index))
        .collect::<Vec<_>>();
    let remaining_budget = budget.saturating_sub(selected.len());
    for index in evenly_sample_indices(&remaining, remaining_budget) {
        push_unique_prompt_index(index, &mut selected, &mut seen);
    }

    selected.sort_unstable();
    selected
}

pub(crate) fn prompt_snippet_limit(role: LiquidBlockRole) -> usize {
    match role {
        LiquidBlockRole::Title | LiquidBlockRole::Heading => 180,
        LiquidBlockRole::Abstract | LiquidBlockRole::Lead => 220,
        LiquidBlockRole::Explainer
        | LiquidBlockRole::Takeaway
        | LiquidBlockRole::Holding
        | LiquidBlockRole::Issue => 200,
        LiquidBlockRole::Definition | LiquidBlockRole::KeyClause => 160,
        LiquidBlockRole::Caption | LiquidBlockRole::Marginalia | LiquidBlockRole::Table => 180,
        LiquidBlockRole::Syllabus => 220,
        _ => crate::liquid::config::MAX_LLM_BLOCK_CHARS,
    }
}

pub(crate) fn should_skip_llm_prompt_block(role: LiquidBlockRole) -> bool {
    matches!(
        role,
        LiquidBlockRole::Header
            | LiquidBlockRole::Footer
            | LiquidBlockRole::Footnote
            | LiquidBlockRole::Caption
            | LiquidBlockRole::Table
            | LiquidBlockRole::Contents
            | LiquidBlockRole::Noise
            | LiquidBlockRole::SectionBreak
    )
}

pub(crate) fn push_unique_prompt_index(
    index: usize,
    selected: &mut Vec<usize>,
    seen: &mut HashSet<usize>,
) {
    if seen.insert(index) {
        selected.push(index);
    }
}

pub(crate) fn evenly_sample_indices(indices: &[usize], limit: usize) -> Vec<usize> {
    if limit == 0 || indices.is_empty() {
        return Vec::new();
    }
    if indices.len() <= limit {
        return indices.to_vec();
    }
    if limit == 1 {
        return vec![indices[0]];
    }

    let last = indices.len() - 1;
    let mut sampled = Vec::with_capacity(limit);
    for slot in 0..limit {
        let position = slot * last / (limit - 1);
        sampled.push(indices[position]);
    }
    sampled.dedup();
    if sampled.len() < limit {
        for index in indices.iter().copied() {
            if sampled.len() >= limit {
                break;
            }
            if !sampled.contains(&index) {
                sampled.push(index);
            }
        }
        sampled.sort_unstable();
    }
    sampled
}

pub(crate) fn is_structural_prompt_block(role: LiquidBlockRole) -> bool {
    matches!(
        role,
        LiquidBlockRole::Heading
            | LiquidBlockRole::Subheading
            | LiquidBlockRole::Lead
            | LiquidBlockRole::Abstract
            | LiquidBlockRole::Syllabus
            | LiquidBlockRole::AuthorInfo
            | LiquidBlockRole::Explainer
            | LiquidBlockRole::Takeaway
            | LiquidBlockRole::Holding
            | LiquidBlockRole::Issue
            | LiquidBlockRole::Definition
            | LiquidBlockRole::Marginalia
            | LiquidBlockRole::KeyClause
            | LiquidBlockRole::Metadata
            | LiquidBlockRole::Quote
            | LiquidBlockRole::Caption
            | LiquidBlockRole::Table
    )
}

/// Returns the compact system prompt for the LLM layout pass.
/// Only advertises supported features: role reclassification, labels, visual_break_before,
/// and action=remove. No mentions of inline <bold>/<italics> tags (pdfium extraction
/// produces none) or box/bkground_color/text_color (renderer ignores them).
pub(crate) fn build_system_prompt() -> &'static str {
    "\
You are the layout planner for LawPDF Review Mode, a calm reading view for PDFs of news, articles, law review pieces, and legal documents. \
The user will provide compact descriptors of numbered source blocks, not the full source text. \
Your job is to return JSON styling metadata for the shown source_index values so the app can keep the original text but improve the reading layout. \
The input may include paragraph text, headings, bylines, abstracts, footnotes, page-boundary artifacts, and OCR junk.\n\n\
Non-negotiable fidelity rules:\n\
- Never rewrite, paraphrase, summarize, translate, correct, or omit source text.\n\
- Do not invent text. Do not merge unrelated paragraphs. Do not split a sentence unless it is already a stand-alone heading, header, footer, or quote.\n\
- Use source_index exactly so the app can map your style decision back to the source block.\n\
- The optional block field is only an identifier; it is not used as display text.\n\n\
Style rules:\n\
- type may be heading1 through heading9, paragraph, lead, lede, abstract, syllabus, author_info, explainer, takeaway, holding, issue, definition, key_clause, caption, table, contents, metadata, header, footer, footnote, quote_para, or noise.\n\
- paragraph is the default; omit type when the block is an ordinary paragraph.\n\
- Use lead/lede for the first substantive article paragraph or standfirst only.\n\
- Mark extracted table-of-contents headings, dot-leader page entries, isolated page numbers, repeated running headers/footers, repository cover boilerplate, and OCR/layout junk as type noise when they add no reading value. Mark figure/table/photo captions and source or credit lines as type caption. Mark court syllabus blocks (Syllabus header, Question Presented, early Held:) as type syllabus. Mark columnar/digit-dense tables as type table. Mark exam/export metadata lines such as character limits, percentages, generated source filenames, and \"Contracts Exam - Part\" lines as type metadata. Mark all footnote text as type footnote.\n\
- Never mark footnotes, titles, authors, abstracts, section headings, or main prose as noise. Use header/footer/contents only when preserving that subtype is useful; otherwise use noise for discardable clutter.\n\
- visual_break_before defaults to false; include it only at major transitions where a reader benefits from a visual pause.\n\
- action defaults to keep; use action remove only for true duplicate artifacts, isolated page numbers, or OCR junk that should not appear anywhere in the Liquid document.\n\
- Return valid JSON only. No explanation. No markdown."
}

/// Builds the compact user prompt for the LLM layout pass.
/// Only solicits the fields the rest of the pipeline actually consumes
/// (type/role, label, visual_break_before, action remove). Example updated accordingly.
pub(crate) fn build_user_prompt(title: &str, n: usize, indexed_blocks: &str) -> String {
    format!(
        "Document: {title}\n\
Review these {n} compact block descriptors. Format per line is source_index|local_role|word_count|snippet. \
Return entries only for source_index values shown below. For ordinary paragraphs, include only source_index. \
Add type, label, visual_break_before, or action only when they differ from defaults.\n\n\
Use lead/lede for the first substantive article paragraph. Use explainer/takeaway/holding/issue/key_clause/definition for source blocks that already help the reader understand the document. \
Classify captions, metadata, and footnotes explicitly; do not leave them as ordinary paragraphs. Use noise for disposable table-of-contents entries, page numbers, running headers/footers, repeated repository text, and OCR/layout junk.\n\n\
{indexed_blocks}\n\n\
Return JSON in this shape:\n\
{{\"blocks\":[\n\
  {{\"source_index\":2,\"type\":\"lead\"}},\n\
  {{\"source_index\":3,\"type\":\"heading2\",\"visual_break_before\":true}},\n\
  {{\"source_index\":5,\"type\":\"explainer\",\"label\":\"Why it matters\"}},\n\
  {{\"source_index\":8,\"type\":\"noise\",\"action\":\"remove\"}}\n\
]}}"
    )
}
