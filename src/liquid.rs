use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crossbeam_channel::Sender;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::settings::app_data_dir;

const LIQUID_SCHEMA_VERSION: u32 = 9;
const OPENROUTER_URL: &str = "https://openrouter.ai/api/v1/chat/completions";
const OPENROUTER_MODEL: &str = "openai/gpt-oss-120b:free";
const GROQ_URL: &str = "https://api.groq.com/openai/v1/chat/completions";
const GROQ_MODEL: &str = "openai/gpt-oss-20b";
const MAX_LLM_BLOCKS: usize = 400;
const MAX_LLM_BLOCK_CHARS: usize = 1_500;
const BETA_REQUIRE_LLM_WHEN_KEY_PRESENT: bool = false;
const LLM_LOG_PREVIEW_CHARS: usize = 16_000;

#[derive(Debug, Clone)]
pub struct LiquidRequest {
    pub document_epoch: u64,
    pub path: PathBuf,
    pub title: String,
    pub pages: Vec<String>,
    pub groq_api_key: Option<String>,
    pub openrouter_api_key: Option<String>,
}

#[derive(Debug, Clone, Copy)]
struct LlmProvider {
    name: &'static str,
    url: &'static str,
    model: &'static str,
    max_tokens_field: &'static str,
    max_completion_tokens: usize,
    reasoning_effort: Option<&'static str>,
    openrouter_headers: bool,
}

#[derive(Debug, Clone)]
pub struct LiquidEvent {
    pub document_epoch: u64,
    pub path: PathBuf,
    pub result: Result<LiquidDocument, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiquidDocument {
    pub title: String,
    pub blocks: Vec<LiquidBlock>,
    pub footnotes_removed: usize,
    pub llm_used: bool,
    pub warnings: Vec<String>,
    pub source_signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiquidBlock {
    pub role: LiquidBlockRole,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LiquidBlockRole {
    Title,
    Heading,
    Subheading,
    Abstract,
    AuthorInfo,
    Paragraph,
    Definition,
    Clause,
    ListItem,
    Quote,
    KeyClause,
    Header,
    Footer,
    Footnote,
    Metadata,
    SectionBreak,
}

#[derive(Debug, Deserialize)]
struct LlmLayout {
    #[serde(default)]
    blocks: Vec<LlmBlock>,
}

#[derive(Debug, Deserialize)]
struct LlmBlock {
    source_index: usize,
    #[serde(default, rename = "block")]
    _block: Option<String>,
    #[serde(default, rename = "type")]
    style_type: Option<String>,
    #[serde(default)]
    role: Option<LiquidBlockRole>,
    #[serde(default = "default_keep")]
    action: LlmAction,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    visual_break_before: bool,
    #[serde(default, rename = "box")]
    _box_emphasis: Option<bool>,
    #[serde(default, rename = "bkground_color")]
    _background_color: Option<String>,
    #[serde(default, rename = "text_color")]
    _text_color: Option<String>,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum LlmAction {
    Keep,
    Remove,
}

fn default_keep() -> LlmAction {
    LlmAction::Keep
}

#[derive(Debug, Serialize)]
struct LiquidLlmLog {
    timestamp_unix_secs: u64,
    title: String,
    source_signature: String,
    provider: String,
    model: String,
    block_count: usize,
    prompt_block_count: usize,
    system_prompt: Option<String>,
    user_prompt: Option<String>,
    request_body: Option<serde_json::Value>,
    http_status: Option<u16>,
    success: bool,
    error: Option<String>,
    generation_id: Option<String>,
    response_preview: Option<String>,
    assistant_content_preview: Option<String>,
    response_text: Option<String>,
    assistant_content: Option<String>,
    parsed_layout_blocks: Option<usize>,
}

pub fn spawn_liquid_job(request: LiquidRequest, tx: Sender<LiquidEvent>) {
    thread::spawn(move || {
        let document_epoch = request.document_epoch;
        let path = request.path.clone();
        let result = prepare_liquid_document(request);
        let _ = tx.send(LiquidEvent {
            document_epoch,
            path,
            result,
        });
    });
}

pub fn prepare_liquid_document(request: LiquidRequest) -> Result<LiquidDocument, String> {
    let source_signature = source_signature(&request.path);
    let groq_api_key = request
        .groq_api_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let openrouter_api_key = request
        .openrouter_api_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    if let Some(cached) = load_cached_document(&source_signature) {
        if cached.llm_used || (groq_api_key.is_none() && openrouter_api_key.is_none()) {
            return Ok(cached);
        }
    }

    let (source_text, footnotes_removed) = clean_source_text(&request.pages);
    if source_text.trim().is_empty() {
        return Err("Liquid Mode could not find usable text in this document.".to_owned());
    }

    let mut blocks = build_local_blocks(&request.title, &source_text);
    let mut warnings = Vec::new();
    let mut llm_used = false;

    let mut providers = Vec::new();
    if let Some(api_key) = groq_api_key.as_deref() {
        providers.push((
            LlmProvider {
                name: "Groq",
                url: GROQ_URL,
                model: GROQ_MODEL,
                max_tokens_field: "max_completion_tokens",
                max_completion_tokens: 2048,
                reasoning_effort: Some("medium"),
                openrouter_headers: false,
            },
            api_key,
        ));
    }
    if let Some(api_key) = openrouter_api_key.as_deref() {
        providers.push((
            LlmProvider {
                name: "OpenRouter",
                url: OPENROUTER_URL,
                model: OPENROUTER_MODEL,
                max_tokens_field: "max_tokens",
                max_completion_tokens: 8192,
                reasoning_effort: None,
                openrouter_headers: true,
            },
            api_key,
        ));
    }

    if providers.is_empty() {
        warnings.push("Groq/OpenRouter key missing; used local layout.".to_owned());
    } else {
        let mut last_error = None;
        let mut provider_errors = Vec::new();
        for (provider, api_key) in providers {
            match apply_llm_layout(
                blocks.clone(),
                &request.title,
                &source_signature,
                provider,
                api_key,
            ) {
                Ok(refined) => {
                    blocks = refined;
                    llm_used = true;
                    if !provider_errors.is_empty() {
                        warnings.push(format!(
                            "{}; used {} layout.",
                            provider_errors.join("; "),
                            provider.name
                        ));
                    }
                    last_error = None;
                    break;
                }
                Err(error) => {
                    let provider_error = format!("{} layout failed: {error}", provider.name);
                    provider_errors.push(provider_error.clone());
                    last_error = Some(provider_error);
                }
            }
        }
        if let Some(error) = last_error {
            if BETA_REQUIRE_LLM_WHEN_KEY_PRESENT {
                return Err(format!(
                    "LLM Liquid Mode failed in beta mode; not showing local fallback. {error}"
                ));
            }
            blocks = build_local_blocks(&request.title, &source_text);
            warnings.push(format!("{error}; used local layout."));
        }
    }

    let document = LiquidDocument {
        title: request.title,
        blocks,
        footnotes_removed,
        llm_used,
        warnings,
        source_signature,
    };
    let _ = save_cached_document(&document);
    Ok(document)
}

fn detect_running_headers(pages: &[String]) -> HashMap<String, ()> {
    if pages.len() < 2 {
        return HashMap::new();
    }
    let threshold = ((pages.len() as f32 * 0.25).ceil() as usize).max(2);
    let mut freq: HashMap<String, usize> = HashMap::new();
    for page in pages {
        let candidates: Vec<String> = page
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .take(3)
            .map(|l| {
                l.to_ascii_lowercase()
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .filter(|l| l.len() < 120)
            .collect();
        // deduplicate per-page so one page can only vote once per candidate
        let mut seen = std::collections::HashSet::new();
        for candidate in candidates {
            if seen.insert(candidate.clone()) {
                *freq.entry(candidate).or_default() += 1;
            }
        }
    }
    freq.into_iter()
        .filter(|(_, count)| *count >= threshold)
        .map(|(key, _)| (key, ()))
        .collect()
}

fn is_running_header(line: &str, headers: &HashMap<String, ()>) -> bool {
    let normalised = line
        .trim()
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    headers.contains_key(&normalised)
}

fn is_lone_page_number(line: &str) -> bool {
    let t = line.trim();
    if t.chars().all(|c| c.is_ascii_digit()) && !t.is_empty() {
        return true;
    }
    // "Page N of M" or "- N -"
    let lower = t.to_ascii_lowercase();
    if lower.starts_with("page ") && lower.split_whitespace().count() <= 4 {
        return true;
    }
    if t.starts_with("- ") && t.ends_with(" -") && t.len() <= 10 {
        return true;
    }
    false
}

fn clean_source_text(pages: &[String]) -> (String, usize) {
    let headers = detect_running_headers(pages);
    let mut output = String::new();
    let mut removed = 0usize;

    for page in pages {
        let (cleaned, page_removed) = clean_page_text(page, &headers);
        removed += page_removed;
        let trimmed = cleaned.trim();
        if trimmed.is_empty() {
            continue;
        }
        if output.is_empty() {
            output.push_str(trimmed);
        } else {
            // Detect how to join: inspect the last non-empty line of previous content
            let last_line = output
                .lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("")
                .trim_end();
            if last_line.ends_with('-') {
                // Cross-page hyphenation: strip the hyphen and join directly
                let new_len = output.trim_end_matches(|c: char| c.is_whitespace()).len();
                output.truncate(new_len.saturating_sub(1)); // remove the '-'
                output.push_str(trimmed);
            } else if last_line.ends_with(['.', '?', '!', ':', '"', '\u{201d}']) {
                // Clear sentence end → paragraph break
                output.push_str("\n\n");
                output.push_str(trimmed);
            } else {
                // Mid-sentence continuation across page boundary
                output.push('\n');
                output.push_str(trimmed);
            }
        }
    }

    (output, removed)
}

fn clean_page_text(page: &str, headers: &HashMap<String, ()>) -> (String, usize) {
    let lines: Vec<&str> = page
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .filter(|line| !is_running_header(line, headers) && !is_lone_page_number(line))
        .collect();

    (lines.join("\n"), 0)
}

fn looks_like_footnote_line(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.len() < 18 {
        return false;
    }
    let mut chars = trimmed.chars().peekable();
    let Some(first) = chars.peek().copied() else {
        return false;
    };
    if is_superscript_digit(first) {
        return true;
    }
    if !first.is_ascii_digit() {
        return false;
    }
    let mut digits = 0usize;
    while chars.peek().is_some_and(|ch| ch.is_ascii_digit()) {
        digits += 1;
        chars.next();
    }
    if digits == 0 || digits > 3 {
        return false;
    }
    chars
        .peek()
        .is_some_and(|ch| ch.is_whitespace() || matches!(ch, '.' | ')' | ']'))
}

fn build_local_blocks(title: &str, source_text: &str) -> Vec<LiquidBlock> {
    let mut blocks = Vec::new();
    blocks.push(LiquidBlock {
        role: LiquidBlockRole::Title,
        text: title.to_owned(),
        label: None,
    });

    let paragraphs = split_paragraphs(source_text);
    for (index, text) in paragraphs.into_iter().enumerate() {
        if text.trim().is_empty() {
            continue;
        }
        let role = classify_block(&text, index);
        let label = label_for_block(role, &text);
        blocks.push(LiquidBlock { role, text, label });
    }

    blocks
}

fn split_paragraphs(source_text: &str) -> Vec<String> {
    let mut paragraphs = Vec::new();
    let mut current = String::new();

    for raw_line in source_text.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            flush_paragraph(&mut current, &mut paragraphs);
            continue;
        }

        if looks_like_heading(line) || looks_like_list_item(line) {
            flush_paragraph(&mut current, &mut paragraphs);
            paragraphs.push(line.to_owned());
            continue;
        }

        if current.is_empty() {
            current.push_str(line);
        } else if current.ends_with('-') {
            current.pop();
            current.push_str(line);
        } else {
            current.push(' ');
            current.push_str(line);
        }
    }

    flush_paragraph(&mut current, &mut paragraphs);
    paragraphs
        .into_iter()
        .flat_map(expand_dense_paragraph)
        .collect()
}

fn flush_paragraph(current: &mut String, paragraphs: &mut Vec<String>) {
    let value = current.split_whitespace().collect::<Vec<_>>().join(" ");
    if !value.is_empty() {
        paragraphs.push(value);
    }
    current.clear();
}

fn expand_dense_paragraph(text: String) -> Vec<String> {
    let mut output = Vec::new();
    let mut rest = text.trim().to_owned();
    if rest.is_empty() {
        return output;
    }

    if let Some((header, body)) = split_pdf_export_header(&rest) {
        output.push(header);
        rest = body;
    }

    for segment in split_long_paragraph(&rest) {
        let segment = segment.trim();
        if !segment.is_empty() {
            output.push(segment.to_owned());
        }
    }
    output
}

fn split_pdf_export_header(text: &str) -> Option<(String, String)> {
    let marker = "Characters:";
    let marker_pos = text.find(marker)?;
    let after_marker = marker_pos + marker.len();
    let slash = text[after_marker..].find('/')?;
    let mut split_at = after_marker + slash + 1;

    while let Some(ch) = text[split_at..].chars().next() {
        if ch.is_ascii_digit() || ch == ',' || ch.is_whitespace() {
            split_at += ch.len_utf8();
        } else {
            break;
        }
    }

    let header = text[..split_at].trim();
    let body = text[split_at..].trim();
    if header.len() < 40 || body.len() < 40 {
        return None;
    }
    Some((header.to_owned(), body.to_owned()))
}

fn split_long_paragraph(text: &str) -> Vec<String> {
    const TARGET_CHARS: usize = 620;
    const MAX_CHARS: usize = 1_050;

    if text.chars().count() <= MAX_CHARS {
        return vec![text.to_owned()];
    }

    let mut parts = Vec::new();
    let mut current = String::new();
    for sentence in split_sentences(text) {
        let sentence = sentence.trim();
        if sentence.is_empty() {
            continue;
        }
        let current_len = current.chars().count();
        let sentence_len = sentence.chars().count();
        if !current.is_empty()
            && current_len >= TARGET_CHARS
            && current_len + sentence_len > TARGET_CHARS
        {
            parts.push(current.trim().to_owned());
            current.clear();
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(sentence);
    }
    if !current.trim().is_empty() {
        parts.push(current.trim().to_owned());
    }

    if parts.len() <= 1 {
        vec![text.to_owned()]
    } else {
        parts
    }
}

fn split_sentences(text: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut start = 0usize;
    let mut chars = text.char_indices().peekable();
    while let Some((idx, ch)) = chars.next() {
        if !matches!(ch, '.' | '?' | '!') {
            continue;
        }
        let Some((next_idx, next)) = chars.peek().copied() else {
            continue;
        };
        if !next.is_whitespace() {
            continue;
        }
        let end = idx + ch.len_utf8();
        let sentence = text[start..end].trim();
        if !sentence.is_empty() {
            sentences.push(sentence.to_owned());
        }
        start = next_idx;
    }
    let tail = text[start..].trim();
    if !tail.is_empty() {
        sentences.push(tail.to_owned());
    }
    sentences
}

fn classify_block(text: &str, index: usize) -> LiquidBlockRole {
    if index == 0 && text.len() < 160 && uppercase_ratio(text) > 0.55 {
        return LiquidBlockRole::Heading;
    }
    if looks_like_heading(text) {
        return if text.len() < 70 {
            LiquidBlockRole::Heading
        } else {
            LiquidBlockRole::Subheading
        };
    }
    if looks_like_exam_metadata(text) {
        return LiquidBlockRole::Metadata;
    }
    if looks_like_definition(text) {
        return LiquidBlockRole::Definition;
    }
    if looks_like_list_item(text) {
        return LiquidBlockRole::ListItem;
    }
    if looks_like_footnote_line(text) {
        return LiquidBlockRole::Footnote;
    }
    if looks_like_clause(text) {
        return LiquidBlockRole::Clause;
    }
    if text.starts_with('"') || text.starts_with('“') {
        return LiquidBlockRole::Quote;
    }
    if contains_key_clause_language(text) {
        return LiquidBlockRole::KeyClause;
    }
    LiquidBlockRole::Paragraph
}

fn looks_like_heading(text: &str) -> bool {
    let trimmed = text.trim();
    let upper = trimmed.to_ascii_uppercase();
    let starts_with_legal_heading = [
        "ARTICLE ",
        "SECTION ",
        "PART ",
        "EXHIBIT ",
        "SCHEDULE ",
        "APPENDIX ",
        "RECITALS",
        "WHEREAS",
    ]
    .iter()
    .any(|prefix| upper.starts_with(prefix));

    starts_with_legal_heading
        || (trimmed.len() <= 92
            && uppercase_ratio(trimmed) > 0.72
            && trimmed.chars().any(char::is_alphabetic))
}

fn looks_like_exam_metadata(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("contracts exam - part")
        && lower.contains("character limit:")
        && lower.contains("characters:")
}

fn looks_like_definition(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains(" means ")
        || lower.contains(" shall mean ")
        || lower.contains(" is defined as ")
        || text.starts_with('"')
        || text.starts_with('“')
}

fn looks_like_list_item(text: &str) -> bool {
    let trimmed = text.trim_start();
    trimmed.starts_with("- ")
        || trimmed.starts_with("• ")
        || starts_with_enumerator(trimmed, '(')
        || starts_with_numbered_prefix(trimmed)
}

fn looks_like_clause(text: &str) -> bool {
    starts_with_numbered_prefix(text.trim_start())
        || text
            .trim_start()
            .chars()
            .take(4)
            .collect::<String>()
            .contains('.')
}

fn starts_with_numbered_prefix(text: &str) -> bool {
    let mut seen_digit = false;
    for ch in text.chars().take(8) {
        if ch.is_ascii_digit() {
            seen_digit = true;
            continue;
        }
        if seen_digit && matches!(ch, '.' | ')') {
            return true;
        }
        if !matches!(ch, '.' | ' ') {
            return false;
        }
    }
    false
}

fn starts_with_enumerator(text: &str, open: char) -> bool {
    let mut chars = text.chars();
    chars.next() == Some(open)
        && chars.next().is_some_and(|ch| ch.is_ascii_alphanumeric())
        && chars.next() == Some(')')
}

fn contains_key_clause_language(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    [
        "shall",
        "must",
        "may not",
        "deadline",
        "terminate",
        "termination",
        "confidential",
        "indemn",
        "payment",
        "notice",
        "breach",
        "liable",
        "liability",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn label_for_block(role: LiquidBlockRole, text: &str) -> Option<String> {
    match role {
        LiquidBlockRole::Definition => text
            .split_once(" means ")
            .or_else(|| text.split_once(" shall mean "))
            .map(|(term, _)| term.trim_matches(['"', '“', '”', ' ', '.']).to_owned())
            .filter(|term| !term.is_empty() && term.len() <= 80)
            .or_else(|| Some("Definition".to_owned())),
        LiquidBlockRole::KeyClause => key_clause_label(text).map(str::to_owned),
        _ => None,
    }
}

fn key_clause_label(text: &str) -> Option<&'static str> {
    let lower = text.to_ascii_lowercase();
    if lower.contains("termination") || lower.contains("terminate") {
        Some("Termination")
    } else if lower.contains("payment") || lower.contains("fee") || lower.contains("invoice") {
        Some("Payment")
    } else if lower.contains("confidential") {
        Some("Confidentiality")
    } else if lower.contains("notice") {
        Some("Notice")
    } else if lower.contains("indemn") || lower.contains("liability") || lower.contains("liable") {
        Some("Risk")
    } else if lower.contains("shall") || lower.contains("must") {
        Some("Obligation")
    } else {
        None
    }
}

fn apply_llm_layout(
    blocks: Vec<LiquidBlock>,
    title: &str,
    source_signature: &str,
    provider: LlmProvider,
    api_key: &str,
) -> Result<Vec<LiquidBlock>, String> {
    let indexed_blocks = blocks
        .iter()
        .enumerate()
        .skip(1)
        .take(MAX_LLM_BLOCKS)
        .map(|(index, block)| {
            let text = truncate_for_prompt(&block.text, MAX_LLM_BLOCK_CHARS);
            format!("{index}: {text}")
        })
        .collect::<Vec<_>>()
        .join("\n");

    if indexed_blocks.trim().is_empty() {
        return Ok(blocks);
    }

    let n = blocks.len().saturating_sub(1).min(MAX_LLM_BLOCKS);

    let _legacy_system_prompt = "\
You are a document readability engine for \"Liquid Mode\", a clean reading view.\
You receive numbered text blocks extracted from a PDF (law review, legal brief, or similar).\
Your job: output a restructuring plan — classify, filter junk, and mark topic breaks.\n\n\
Rules:\n\
• Never rewrite, paraphrase, summarize, or add text.\n\
• action \"remove\": page headers (journal name, volume/issue/year repeated across pages),\
 isolated page numbers, orphaned single words or fragments, OCR artifacts.\n\
• role \"abstract\": the document's abstract or summary section.\n\
• role \"author_info\": author names, affiliations, footnote-star bios.\n\
• visual_break_before true: set ONLY when a major argumentative or topical shift occurs\
 that the reader would benefit from seeing as a visual pause — not for every new paragraph,\
 only for section-level transitions. Use sparingly (≤ 10 % of blocks).\n\
• label: for definitions → the defined term; for key_clause → topic keyword\
 (Payment, Termination, Confidentiality, Notice, Risk, Obligation); else null.\n\
• Return valid JSON only. No explanation.";

    let _legacy_user_prompt = format!(
        "Document: {title}\n\
Classify {n} blocks below. Preserve source_index exactly.\n\n\
{indexed_blocks}\n\n\
Return: {{\"blocks\":[{{\"source_index\":N,\"role\":\"...\",\"action\":\"keep\",\"label\":null,\"visual_break_before\":false}}]}}"
    );

    let system_prompt = "\
You are a document design engine for LawPDF Liquid Mode. \
The user will provide the full text of a document as numbered source blocks. \
Your job is to reproduce the document structure with total text fidelity by returning JSON metadata for each paragraph or stand-alone part. \
The input mirrors the real PDF extraction pipeline: it may include paragraph breaks, line breaks, page-boundary artifacts, and inline formatting markers.\n\n\
Non-negotiable fidelity rules:\n\
- Never rewrite, paraphrase, summarize, translate, correct, or omit source text.\n\
- Do not invent text. Do not merge unrelated paragraphs. Do not split a sentence unless it is already a stand-alone heading, header, footer, or quote.\n\
- Preserve inline markup exactly, including <bold>...</bold>, <italics>...</italics>, <underline>...</underline>, small-caps markers, and any other XML-like tags already present in the source.\n\
- Do not move text into or out of formatting tags. Do not normalize tag names. Do not escape or delete formatting tags in block identifiers.\n\
- Use source_index exactly so the app can map your style decision back to the source block.\n\
- The block field is only an identifier: if the part has fewer than 10 words, use the full text; otherwise use the first 5 words, an ellipsis, and the last 5 words.\n\n\
Style rules:\n\
- type may be heading1 through heading9, paragraph, metadata, header, footer, footnote, or quote_para.\n\
- paragraph is the default; omit type when the block is an ordinary paragraph.\n\
- Mark exam/export metadata lines such as character limits, percentages, generated source filenames, and \"Contracts Exam - Part\" lines as type metadata. Mark all footnote text as type footnote. Mark all bottom-of-page footer text, page numbers, docket/citation footer lines, and repeated footer artifacts as type footer. Mark repeated top-of-page running text as type header.\n\
- Do not use action remove for footnotes, headers, or footers unless the text is a duplicate artifact with no value. Classify them so Liquid Mode can hide them or move them out of the main reading stream.\n\
- box defaults to false; include box only when a block deserves emphasis as a boxed callout.\n\
- bkground_color defaults to none; include it only for intentional emphasis, using a simple color name or hex value.\n\
- text_color defaults to none; include it only if a non-default text color is useful.\n\
- visual_break_before defaults to false; include it only at major transitions where a reader benefits from a visual pause.\n\
- action defaults to keep; use action remove only for true duplicate artifacts, isolated page numbers, or OCR junk that should not appear anywhere in the Liquid document.\n\
- Return valid JSON only. No explanation. No markdown.";

    let user_prompt = format!(
        "Document: {title}\n\
Review {n} numbered source blocks below. Create one JSON entry for each paragraph or stand-alone part. \
Preserve source_index exactly. For ordinary paragraphs, include only source_index and block. \
Add type, box, bkground_color, text_color, visual_break_before, or action only when they differ from defaults.\n\n\
Important: preserve any inline formatting tags shown in the source, such as <bold> or <italics>, inside the block identifier when those words fall within the identifier. \
Classify metadata, footnotes, headers, and footer text explicitly; do not leave them as ordinary paragraphs.\n\n\
{indexed_blocks}\n\n\
Return JSON in this shape:\n\
{{\"blocks\":[\n\
  {{\"source_index\":1,\"block\":\"Anonymous ID 436 Contracts Exam ... Question A legal issue\"}},\n\
  {{\"source_index\":2,\"block\":\"Question A\",\"type\":\"heading2\",\"visual_break_before\":true}},\n\
  {{\"source_index\":3,\"block\":\"Student Answer\",\"type\":\"heading3\"}},\n\
  {{\"source_index\":4,\"block\":\"<italics>Tongish</italics> explains ... discretion in good faith\"}},\n\
  {{\"source_index\":5,\"block\":\"3 / 4\",\"type\":\"paragraph\",\"box\":true,\"bkground_color\":\"#fff4cc\"}},\n\
  {{\"source_index\":6,\"block\":\"Contracts Exam - Part IV ... 7,895 / 10,000\",\"type\":\"metadata\"}},\n\
  {{\"source_index\":7,\"block\":\"1 See Restatement ... duty of good faith\",\"type\":\"footnote\"}},\n\
  {{\"source_index\":8,\"block\":\"436-Updated_Contracts_SP26_Arbel ... Page 2\",\"type\":\"footer\"}}\n\
]}}"
    );

    let mut body = json!({
        "model": provider.model,
        "messages": [
            { "role": "system", "content": system_prompt },
            { "role": "user",   "content": user_prompt }
        ]
    });
    body[provider.max_tokens_field] = json!(provider.max_completion_tokens);
    if let Some(reasoning_effort) = provider.reasoning_effort {
        body["reasoning_effort"] = json!(reasoning_effort);
    }

    let _ = write_liquid_llm_log(&LiquidLlmLog {
        timestamp_unix_secs: now_unix_secs(),
        title: title.to_owned(),
        source_signature: source_signature.to_owned(),
        provider: provider.name.to_owned(),
        model: provider.model.to_owned(),
        block_count: blocks.len(),
        prompt_block_count: n,
        system_prompt: Some(system_prompt.to_owned()),
        user_prompt: Some(user_prompt.clone()),
        request_body: Some(body.clone()),
        http_status: None,
        success: false,
        error: Some("OpenRouter request queued; awaiting response.".to_owned()),
        generation_id: None,
        response_preview: None,
        assistant_content_preview: None,
        response_text: None,
        assistant_content: None,
        parsed_layout_blocks: None,
    });

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(120))
        .http1_only()
        .no_gzip()
        .no_brotli()
        .no_deflate()
        .no_zstd()
        .build()
        .map_err(|error| {
            let message = format!("Could not create OpenRouter client: {error}");
            let log_path = write_liquid_llm_log(&LiquidLlmLog {
                timestamp_unix_secs: now_unix_secs(),
                title: title.to_owned(),
                source_signature: source_signature.to_owned(),
                provider: provider.name.to_owned(),
                model: provider.model.to_owned(),
                block_count: blocks.len(),
                prompt_block_count: n,
                system_prompt: Some(system_prompt.to_owned()),
                user_prompt: Some(user_prompt.clone()),
                request_body: Some(body.clone()),
                http_status: None,
                success: false,
                error: Some(message.clone()),
                generation_id: None,
                response_preview: None,
                assistant_content_preview: None,
                response_text: None,
                assistant_content: None,
                parsed_layout_blocks: None,
            });
            with_log_path(message, log_path)
        })?;
    let request_builder = client
        .post(provider.url)
        .bearer_auth(api_key)
        .header("Accept", "application/json")
        .header("Accept-Encoding", "identity");
    let request_builder = if provider.openrouter_headers {
        request_builder
            .header("HTTP-Referer", "https://github.com/yonathanarbel/LawPDF")
            .header("X-Title", "LawPDF")
    } else {
        request_builder
    };
    let response = request_builder.json(&body).send().map_err(|error| {
        let message = format!("Could not reach {}: {error}", provider.name);
        let log_path = write_liquid_llm_log(&LiquidLlmLog {
            timestamp_unix_secs: now_unix_secs(),
            title: title.to_owned(),
            source_signature: source_signature.to_owned(),
            provider: provider.name.to_owned(),
            model: provider.model.to_owned(),
            block_count: blocks.len(),
            prompt_block_count: n,
            system_prompt: Some(system_prompt.to_owned()),
            user_prompt: Some(user_prompt.clone()),
            request_body: Some(body.clone()),
            http_status: None,
            success: false,
            error: Some(message.clone()),
            generation_id: None,
            response_preview: None,
            assistant_content_preview: None,
            response_text: None,
            assistant_content: None,
            parsed_layout_blocks: None,
        });
        with_log_path(message, log_path)
    })?;

    let status = response.status();
    let generation_id = response
        .headers()
        .get("X-Generation-Id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let response_bytes = response.bytes().map_err(|error| {
        let message = format!("Could not read {} response body: {error}", provider.name);
        let log_path = write_liquid_llm_log(&LiquidLlmLog {
            timestamp_unix_secs: now_unix_secs(),
            title: title.to_owned(),
            source_signature: source_signature.to_owned(),
            provider: provider.name.to_owned(),
            model: provider.model.to_owned(),
            block_count: blocks.len(),
            prompt_block_count: n,
            system_prompt: Some(system_prompt.to_owned()),
            user_prompt: Some(user_prompt.clone()),
            request_body: Some(body.clone()),
            http_status: Some(status.as_u16()),
            success: false,
            error: Some(message.clone()),
            generation_id: generation_id.clone(),
            response_preview: None,
            assistant_content_preview: None,
            response_text: None,
            assistant_content: None,
            parsed_layout_blocks: None,
        });
        with_log_path(message, log_path)
    })?;
    let response_text = String::from_utf8_lossy(&response_bytes).to_string();

    if !status.is_success() {
        let message = format!("{} returned HTTP {status}", provider.name);
        let log_path = write_liquid_llm_log(&LiquidLlmLog {
            timestamp_unix_secs: now_unix_secs(),
            title: title.to_owned(),
            source_signature: source_signature.to_owned(),
            provider: provider.name.to_owned(),
            model: provider.model.to_owned(),
            block_count: blocks.len(),
            prompt_block_count: n,
            system_prompt: Some(system_prompt.to_owned()),
            user_prompt: Some(user_prompt.clone()),
            request_body: Some(body.clone()),
            http_status: Some(status.as_u16()),
            success: false,
            error: Some(message.clone()),
            generation_id,
            response_preview: Some(preview(&response_text, LLM_LOG_PREVIEW_CHARS)),
            assistant_content_preview: None,
            response_text: Some(response_text.clone()),
            assistant_content: None,
            parsed_layout_blocks: None,
        });
        return Err(with_log_path(message, log_path));
    }

    let response_json =
        serde_json::from_str::<serde_json::Value>(&response_text).map_err(|error| {
            let message = format!("{} response was not valid JSON: {error}", provider.name);
            let log_path = write_liquid_llm_log(&LiquidLlmLog {
                timestamp_unix_secs: now_unix_secs(),
                title: title.to_owned(),
                source_signature: source_signature.to_owned(),
                provider: provider.name.to_owned(),
                model: provider.model.to_owned(),
                block_count: blocks.len(),
                prompt_block_count: n,
                system_prompt: Some(system_prompt.to_owned()),
                user_prompt: Some(user_prompt.clone()),
                request_body: Some(body.clone()),
                http_status: Some(status.as_u16()),
                success: false,
                error: Some(message.clone()),
                generation_id: generation_id.clone(),
                response_preview: Some(preview(&response_text, LLM_LOG_PREVIEW_CHARS)),
                assistant_content_preview: None,
                response_text: Some(response_text.clone()),
                assistant_content: None,
                parsed_layout_blocks: None,
            });
            with_log_path(message, log_path)
        })?;

    let content = response_json
        .pointer("/choices/0/message/content")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            let message = format!(
                "{} response did not include message content.",
                provider.name
            );
            let log_path = write_liquid_llm_log(&LiquidLlmLog {
                timestamp_unix_secs: now_unix_secs(),
                title: title.to_owned(),
                source_signature: source_signature.to_owned(),
                provider: provider.name.to_owned(),
                model: provider.model.to_owned(),
                block_count: blocks.len(),
                prompt_block_count: n,
                system_prompt: Some(system_prompt.to_owned()),
                user_prompt: Some(user_prompt.clone()),
                request_body: Some(body.clone()),
                http_status: Some(status.as_u16()),
                success: false,
                error: Some(message.clone()),
                generation_id: generation_id.clone(),
                response_preview: Some(preview(&response_text, LLM_LOG_PREVIEW_CHARS)),
                assistant_content_preview: None,
                response_text: Some(response_text.clone()),
                assistant_content: None,
                parsed_layout_blocks: None,
            });
            with_log_path(message, log_path)
        })?;
    let content = extract_json_object(content);
    let layout = serde_json::from_str::<LlmLayout>(&content).map_err(|error| {
        let message = format!("OpenRouter layout JSON could not be parsed: {error}");
        let log_path = write_liquid_llm_log(&LiquidLlmLog {
            timestamp_unix_secs: now_unix_secs(),
            title: title.to_owned(),
            source_signature: source_signature.to_owned(),
            provider: provider.name.to_owned(),
            model: provider.model.to_owned(),
            block_count: blocks.len(),
            prompt_block_count: n,
            system_prompt: Some(system_prompt.to_owned()),
            user_prompt: Some(user_prompt.clone()),
            request_body: Some(body.clone()),
            http_status: Some(status.as_u16()),
            success: false,
            error: Some(message.clone()),
            generation_id: generation_id.clone(),
            response_preview: Some(preview(&response_text, LLM_LOG_PREVIEW_CHARS)),
            assistant_content_preview: Some(preview(&content, LLM_LOG_PREVIEW_CHARS)),
            response_text: Some(response_text.clone()),
            assistant_content: Some(content.clone()),
            parsed_layout_blocks: None,
        });
        with_log_path(message, log_path)
    })?;
    let parsed_layout_blocks = layout.blocks.len();

    let _ = write_liquid_llm_log(&LiquidLlmLog {
        timestamp_unix_secs: now_unix_secs(),
        title: title.to_owned(),
        source_signature: source_signature.to_owned(),
        provider: provider.name.to_owned(),
        model: provider.model.to_owned(),
        block_count: blocks.len(),
        prompt_block_count: n,
        system_prompt: Some(system_prompt.to_owned()),
        user_prompt: Some(user_prompt.clone()),
        request_body: Some(body.clone()),
        http_status: Some(status.as_u16()),
        success: true,
        error: None,
        generation_id,
        response_preview: Some(preview(&response_text, LLM_LOG_PREVIEW_CHARS)),
        assistant_content_preview: Some(preview(&content, LLM_LOG_PREVIEW_CHARS)),
        response_text: Some(response_text.clone()),
        assistant_content: Some(content.clone()),
        parsed_layout_blocks: Some(parsed_layout_blocks),
    });

    // Build a lookup by source_index
    let llm_map: HashMap<usize, LlmBlock> = layout
        .blocks
        .into_iter()
        .map(|b| (b.source_index, b))
        .collect();

    // Reconstruct block list: apply roles/labels, inject section breaks, drop removed blocks
    let mut result: Vec<LiquidBlock> = Vec::with_capacity(blocks.len());
    for (idx, mut block) in blocks.into_iter().enumerate() {
        if idx == 0 {
            // Always keep the title block untouched
            result.push(block);
            continue;
        }
        if looks_like_exam_metadata(&block.text) {
            if idx > 1 {
                result.push(LiquidBlock {
                    role: LiquidBlockRole::SectionBreak,
                    text: String::new(),
                    label: None,
                });
            }
            block.role = LiquidBlockRole::Metadata;
            result.push(block);
            continue;
        }
        if let Some(llm) = llm_map.get(&idx) {
            if llm.action == LlmAction::Remove {
                continue;
            }
            if llm.visual_break_before {
                result.push(LiquidBlock {
                    role: LiquidBlockRole::SectionBreak,
                    text: String::new(),
                    label: None,
                });
            }
            if let Some(role) = llm
                .role
                .or_else(|| llm.style_type.as_deref().map(style_type_to_role))
            {
                block.role = role;
            }
            block.label = llm
                .label
                .as_deref()
                .filter(|l| !l.trim().is_empty())
                .map(str::to_owned);
        }
        result.push(block);
    }

    Ok(result)
}

fn style_type_to_role(style_type: &str) -> LiquidBlockRole {
    match style_type.trim().to_ascii_lowercase().as_str() {
        "heading1" | "heading2" | "heading3" => LiquidBlockRole::Heading,
        "heading4" | "heading5" | "heading6" | "heading7" | "heading8" | "heading9" => {
            LiquidBlockRole::Subheading
        }
        "quote_para" => LiquidBlockRole::Quote,
        "header" => LiquidBlockRole::Header,
        "footer" => LiquidBlockRole::Footer,
        "footnote" => LiquidBlockRole::Footnote,
        "metadata" => LiquidBlockRole::Metadata,
        "paragraph" => LiquidBlockRole::Paragraph,
        _ => LiquidBlockRole::Paragraph,
    }
}

fn write_liquid_llm_log(log: &LiquidLlmLog) -> Option<PathBuf> {
    let dir = app_data_dir()?.join("liquid-logs");
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(format!(
        "{}-{}.json",
        log.timestamp_unix_secs, log.source_signature
    ));
    let bytes = serde_json::to_vec_pretty(log).ok()?;
    std::fs::write(&path, bytes).ok()?;
    Some(path)
}

fn with_log_path(message: String, log_path: Option<PathBuf>) -> String {
    match log_path {
        Some(path) => format!("{message} Log: {}", path.display()),
        None => message,
    }
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

fn preview(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_owned();
    }
    let mut truncated = value.chars().take(max_chars).collect::<String>();
    truncated.push_str("...");
    truncated
}

fn extract_json_object(content: &str) -> String {
    let trimmed = content.trim();
    let without_fence = if trimmed.starts_with("```") {
        trimmed
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim()
            .trim_end_matches("```")
            .trim()
    } else {
        trimmed
    };

    let Some(start) = without_fence.find('{') else {
        return without_fence.to_owned();
    };
    let Some(end) = without_fence.rfind('}') else {
        return without_fence.to_owned();
    };
    without_fence[start..=end].to_owned()
}

fn truncate_for_prompt(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    let mut value = text.chars().take(max_chars).collect::<String>();
    value.push_str("...");
    value
}

fn uppercase_ratio(text: &str) -> f32 {
    let letters = text.chars().filter(|ch| ch.is_alphabetic()).count();
    if letters == 0 {
        return 0.0;
    }
    let uppercase = text.chars().filter(|ch| ch.is_uppercase()).count();
    uppercase as f32 / letters as f32
}

fn is_superscript_digit(ch: char) -> bool {
    matches!(
        ch,
        '⁰' | '¹' | '²' | '³' | '⁴' | '⁵' | '⁶' | '⁷' | '⁸' | '⁹'
    )
}

fn load_cached_document(source_signature: &str) -> Option<LiquidDocument> {
    let path = cache_path(source_signature)?;
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice::<LiquidDocument>(&bytes).ok()
}

fn save_cached_document(document: &LiquidDocument) -> Result<(), String> {
    let path = cache_path(&document.source_signature)
        .ok_or_else(|| "Could not find Liquid Mode cache directory.".to_owned())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create Liquid Mode cache: {error}"))?;
    }
    let bytes = serde_json::to_vec(document).map_err(|error| error.to_string())?;
    std::fs::write(path, bytes).map_err(|error| error.to_string())
}

fn cache_path(source_signature: &str) -> Option<PathBuf> {
    app_data_dir().map(|dir| {
        dir.join("liquid-cache")
            .join(format!("{source_signature}.json"))
    })
}

fn source_signature(path: &Path) -> String {
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let metadata = std::fs::metadata(path).ok();
    let modified = metadata
        .as_ref()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    let len = metadata.map(|metadata| metadata.len()).unwrap_or_default();
    stable_hash(&format!(
        "v{LIQUID_SCHEMA_VERSION}|{}|{modified}|{len}",
        canonical.display()
    ))
}

fn stable_hash(value: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}
