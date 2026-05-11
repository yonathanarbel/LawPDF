use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, UNIX_EPOCH};

use crossbeam_channel::Sender;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::settings::app_data_dir;

const LIQUID_SCHEMA_VERSION: u32 = 1;
const OPENROUTER_URL: &str = "https://openrouter.ai/api/v1/chat/completions";
const OPENROUTER_MODEL: &str = "openrouter/free";
const MAX_LLM_BLOCKS: usize = 180;
const MAX_LLM_BLOCK_CHARS: usize = 260;

#[derive(Debug, Clone)]
pub struct LiquidRequest {
    pub document_epoch: u64,
    pub path: PathBuf,
    pub title: String,
    pub pages: Vec<String>,
    pub openrouter_api_key: Option<String>,
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
    Paragraph,
    Definition,
    Clause,
    ListItem,
    Quote,
    KeyClause,
}

#[derive(Debug, Deserialize)]
struct LlmLayout {
    #[serde(default)]
    blocks: Vec<LlmBlock>,
}

#[derive(Debug, Deserialize)]
struct LlmBlock {
    source_index: usize,
    role: LiquidBlockRole,
    #[serde(default)]
    label: Option<String>,
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
    let openrouter_api_key = request
        .openrouter_api_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    if let Some(cached) = load_cached_document(&source_signature) {
        if cached.llm_used || openrouter_api_key.is_none() {
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

    if let Some(api_key) = openrouter_api_key.as_deref() {
        match apply_openrouter_layout(&mut blocks, api_key) {
            Ok(()) => llm_used = true,
            Err(error) => warnings.push(format!(
                "OpenRouter layout failed; used local layout. {error}"
            )),
        }
    } else {
        warnings.push("OpenRouter key missing; used local layout.".to_owned());
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

fn clean_source_text(pages: &[String]) -> (String, usize) {
    let mut output = String::new();
    let mut removed = 0usize;

    for page in pages {
        let (cleaned, page_removed) = clean_page_text(page);
        removed += page_removed;
        if !cleaned.trim().is_empty() {
            if !output.is_empty() {
                output.push_str("\n\n");
            }
            output.push_str(cleaned.trim());
        }
    }

    (output, removed)
}

fn clean_page_text(page: &str) -> (String, usize) {
    let mut lines = page
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();

    let mut removed = 0usize;
    let footnote_start = lines
        .iter()
        .enumerate()
        .skip(lines.len().saturating_mul(2) / 3)
        .find_map(|(index, line)| looks_like_footnote_line(line).then_some(index));

    if let Some(index) = footnote_start {
        removed += lines.len().saturating_sub(index);
        lines.truncate(index);
    }

    let cleaned = lines
        .into_iter()
        .map(remove_inline_footnote_markers)
        .collect::<Vec<_>>()
        .join("\n");
    (cleaned, removed)
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

fn remove_inline_footnote_markers(line: &str) -> String {
    let chars = line.chars().collect::<Vec<_>>();
    let mut output = String::with_capacity(line.len());
    let mut index = 0usize;

    while index < chars.len() {
        let ch = chars[index];
        if is_superscript_digit(ch) {
            index += 1;
            continue;
        }

        if matches!(ch, '[' | '(') {
            let close = if ch == '[' { ']' } else { ')' };
            let mut end = index + 1;
            while end < chars.len() && chars[end].is_ascii_digit() {
                end += 1;
            }
            let had_digits = end > index + 1;
            if had_digits
                && end < chars.len()
                && chars[end] == close
                && output
                    .chars()
                    .last()
                    .is_some_and(|prev| !prev.is_whitespace())
            {
                index = end + 1;
                continue;
            }
        }

        if ch.is_ascii_digit() {
            let start = index;
            while index < chars.len() && chars[index].is_ascii_digit() {
                index += 1;
            }
            let len = index - start;
            let prev = output.chars().last();
            let next = chars.get(index).copied();
            if len <= 3
                && prev
                    .is_some_and(|value| value.is_alphabetic() || matches!(value, ')' | '"' | '\''))
                && next.is_none_or(|value| {
                    value.is_whitespace() || matches!(value, '.' | ',' | ';' | ':')
                })
            {
                continue;
            }
            for digit in &chars[start..index] {
                output.push(*digit);
            }
            continue;
        }

        output.push(ch);
        index += 1;
    }

    output.split_whitespace().collect::<Vec<_>>().join(" ")
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
}

fn flush_paragraph(current: &mut String, paragraphs: &mut Vec<String>) {
    let value = current.split_whitespace().collect::<Vec<_>>().join(" ");
    if !value.is_empty() {
        paragraphs.push(value);
    }
    current.clear();
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
    if looks_like_definition(text) {
        return LiquidBlockRole::Definition;
    }
    if looks_like_list_item(text) {
        return LiquidBlockRole::ListItem;
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

fn apply_openrouter_layout(blocks: &mut [LiquidBlock], api_key: &str) -> Result<(), String> {
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
        return Ok(());
    }

    let schema = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "blocks": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "source_index": { "type": "integer" },
                        "role": {
                            "type": "string",
                            "enum": ["title", "heading", "subheading", "paragraph", "definition", "clause", "list_item", "quote", "key_clause"]
                        },
                        "label": { "type": ["string", "null"] }
                    },
                    "required": ["source_index", "role", "label"]
                }
            }
        },
        "required": ["blocks"]
    });

    let body = json!({
        "model": OPENROUTER_MODEL,
        "messages": [
            {
                "role": "system",
                "content": "You classify legal-document blocks for a view-only liquid reader. Do not rewrite, summarize, paraphrase, or add new text. Return JSON only."
            },
            {
                "role": "user",
                "content": format!(
                    "Classify these source blocks. Use heading/subheading for structure, definition for defined terms, key_clause for obligations/deadlines/payment/confidentiality/risk, clause for numbered legal clauses, list_item for enumerated items, quote for quoted blocks, paragraph otherwise. Preserve source_index exactly.\n\n{indexed_blocks}"
                )
            }
        ],
        "response_format": {
            "type": "json_schema",
            "json_schema": {
                "name": "lawpdf_liquid_layout",
                "strict": true,
                "schema": schema
            }
        }
    });

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(35))
        .build()
        .map_err(|error| error.to_string())?;
    let response = client
        .post(OPENROUTER_URL)
        .bearer_auth(api_key)
        .header("HTTP-Referer", "https://github.com/yonathanarbel/LawPDF")
        .header("X-Title", "LawPDF")
        .json(&body)
        .send()
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?
        .json::<serde_json::Value>()
        .map_err(|error| error.to_string())?;

    let content = response
        .pointer("/choices/0/message/content")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "OpenRouter response did not include message content.".to_owned())?;
    let content = strip_json_fence(content);
    let layout = serde_json::from_str::<LlmLayout>(&content).map_err(|error| error.to_string())?;

    for block in layout.blocks {
        if let Some(target) = blocks.get_mut(block.source_index) {
            if block.source_index != 0 {
                target.role = block.role;
                target.label = block.label.filter(|label| !label.trim().is_empty());
            }
        }
    }

    Ok(())
}

fn strip_json_fence(content: &str) -> String {
    let trimmed = content.trim();
    if !trimmed.starts_with("```") {
        return trimmed.to_owned();
    }
    let without_start = trimmed
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim();
    without_start.trim_end_matches("```").trim().to_owned()
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
