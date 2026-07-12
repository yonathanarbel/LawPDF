//! LLM layout application: request construction, logging, response parsing,
//! and conversion back into Liquid blocks.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::UNIX_EPOCH;

use schemars::schema_for;
use serde_json::{Value, json};

use crate::settings::app_data_dir;

use super::client::{get_llm_client, send_llm_request_with_retries};
use super::prompt::{build_llm_prompt_input, build_system_prompt, build_user_prompt};
use crate::liquid::classification::{
    label_for_block, looks_like_exam_metadata, style_type_to_role,
};
use crate::liquid::config::{ENABLE_DETAILED_LLM_LOGGING, LLM_LOG_PREVIEW_CHARS};
use crate::liquid::model::{
    LiquidBlock, LiquidBlockRole, LiquidLlmLog, LlmAction, LlmBlock, LlmLayout, LlmProvider,
};
use crate::liquid::push_section_break_if_needed;
use crate::liquid::util::{extract_json_object, now_unix_secs, preview};

pub(crate) fn apply_llm_layout(
    blocks: Vec<LiquidBlock>,
    title: &str,
    source_signature: &str,
    provider: LlmProvider,
    api_key: &str,
) -> Result<Vec<LiquidBlock>, String> {
    let prompt_input = build_llm_prompt_input(&blocks);
    let indexed_blocks = prompt_input.text;

    if indexed_blocks.trim().is_empty() {
        return Ok(blocks);
    }

    let request = build_layout_request(&provider, title, prompt_input.count, &indexed_blocks);
    let log_context = LlmLogContext {
        title,
        source_signature,
        provider: &provider,
        block_count: blocks.len(),
        request: &request,
    };

    let _ = write_llm_call_log(
        &log_context,
        LlmLogOutcome {
            success: false,
            error: Some(format!(
                "{} request queued; awaiting response.",
                provider.name
            )),
            ..LlmLogOutcome::default()
        },
    );

    let http_client = get_llm_client();
    let response = send_llm_request_with_retries(http_client, &provider, api_key, &request.body)
        .map_err(|error| {
            let message = error.clone();
            let log_path = write_llm_call_log(
                &log_context,
                LlmLogOutcome {
                    success: false,
                    error: Some(message.clone()),
                    ..LlmLogOutcome::default()
                },
            );
            with_log_path(message, log_path)
        })?;

    let parsed = parse_layout_response(response, &provider, &log_context)?;
    let _ = write_llm_call_log(
        &log_context,
        LlmLogOutcome {
            success: true,
            http_status: Some(parsed.http_status),
            generation_id: parsed.generation_id.clone(),
            response_text: Some(parsed.response_text.clone()),
            assistant_content: Some(parsed.assistant_content.clone()),
            parsed_layout_blocks: Some(parsed.parsed_layout_blocks),
            ..LlmLogOutcome::default()
        },
    );

    Ok(apply_layout_to_blocks(blocks, parsed.layout))
}

struct LayoutRequest {
    prompt_block_count: usize,
    system_prompt: &'static str,
    user_prompt: String,
    body: Value,
}

fn build_layout_request(
    provider: &LlmProvider,
    title: &str,
    prompt_block_count: usize,
    indexed_blocks: &str,
) -> LayoutRequest {
    let system_prompt = build_system_prompt();
    let user_prompt = build_user_prompt(title, prompt_block_count, indexed_blocks);

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

    let layout_schema = schema_for!(LlmLayout);
    if let Ok(schema_val) = serde_json::to_value(&layout_schema) {
        body["response_format"] = json!({
            "type": "json_schema",
            "json_schema": {
                "name": "LlmLayout",
                "strict": false,
                "schema": schema_val
            }
        });
    }

    LayoutRequest {
        prompt_block_count,
        system_prompt,
        user_prompt,
        body,
    }
}

struct LlmLogContext<'a> {
    title: &'a str,
    source_signature: &'a str,
    provider: &'a LlmProvider,
    block_count: usize,
    request: &'a LayoutRequest,
}

#[derive(Default)]
struct LlmLogOutcome {
    success: bool,
    error: Option<String>,
    http_status: Option<u16>,
    generation_id: Option<String>,
    response_text: Option<String>,
    assistant_content: Option<String>,
    parsed_layout_blocks: Option<usize>,
}

fn write_llm_call_log(context: &LlmLogContext<'_>, outcome: LlmLogOutcome) -> Option<PathBuf> {
    let response_preview = outcome
        .response_text
        .as_ref()
        .map(|text| preview(text, LLM_LOG_PREVIEW_CHARS));
    let assistant_content_preview = outcome
        .assistant_content
        .as_ref()
        .map(|text| preview(text, LLM_LOG_PREVIEW_CHARS));

    write_liquid_llm_log(&LiquidLlmLog {
        timestamp_unix_secs: now_unix_secs(),
        title: context.title.to_owned(),
        source_signature: context.source_signature.to_owned(),
        provider: context.provider.name.to_owned(),
        model: context.provider.model.to_owned(),
        block_count: context.block_count,
        prompt_block_count: context.request.prompt_block_count,
        system_prompt: Some(context.request.system_prompt.to_owned()),
        user_prompt: Some(context.request.user_prompt.clone()),
        request_body: Some(context.request.body.clone()),
        http_status: outcome.http_status,
        success: outcome.success,
        error: outcome.error,
        generation_id: outcome.generation_id,
        response_preview,
        assistant_content_preview,
        response_text: outcome.response_text,
        assistant_content: outcome.assistant_content,
        parsed_layout_blocks: outcome.parsed_layout_blocks,
    })
}

struct ParsedLayoutResponse {
    layout: LlmLayout,
    http_status: u16,
    generation_id: Option<String>,
    response_text: String,
    assistant_content: String,
    parsed_layout_blocks: usize,
}

fn parse_layout_response(
    response: reqwest::blocking::Response,
    provider: &LlmProvider,
    log_context: &LlmLogContext<'_>,
) -> Result<ParsedLayoutResponse, String> {
    let status = response.status();
    let generation_id = response
        .headers()
        .get("X-Generation-Id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);

    let response_bytes = response.bytes().map_err(|error| {
        let message = format!("Could not read {} response body: {error}", provider.name);
        let log_path = write_llm_call_log(
            log_context,
            LlmLogOutcome {
                success: false,
                error: Some(message.clone()),
                http_status: Some(status.as_u16()),
                generation_id: generation_id.clone(),
                ..LlmLogOutcome::default()
            },
        );
        with_log_path(message, log_path)
    })?;
    let response_text = String::from_utf8_lossy(&response_bytes).to_string();

    if !status.is_success() {
        let message = format!("{} returned HTTP {status}", provider.name);
        let log_path = write_llm_call_log(
            log_context,
            LlmLogOutcome {
                success: false,
                error: Some(message.clone()),
                http_status: Some(status.as_u16()),
                generation_id,
                response_text: Some(response_text.clone()),
                ..LlmLogOutcome::default()
            },
        );
        return Err(with_log_path(message, log_path));
    }

    let response_json = serde_json::from_str::<Value>(&response_text).map_err(|error| {
        let message = format!("{} response was not valid JSON: {error}", provider.name);
        let log_path = write_llm_call_log(
            log_context,
            LlmLogOutcome {
                success: false,
                error: Some(message.clone()),
                http_status: Some(status.as_u16()),
                generation_id: generation_id.clone(),
                response_text: Some(response_text.clone()),
                ..LlmLogOutcome::default()
            },
        );
        with_log_path(message, log_path)
    })?;

    let content = response_json
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            let message = format!(
                "{} response did not include message content.",
                provider.name
            );
            let log_path = write_llm_call_log(
                log_context,
                LlmLogOutcome {
                    success: false,
                    error: Some(message.clone()),
                    http_status: Some(status.as_u16()),
                    generation_id: generation_id.clone(),
                    response_text: Some(response_text.clone()),
                    ..LlmLogOutcome::default()
                },
            );
            with_log_path(message, log_path)
        })?;
    let assistant_content = extract_json_object(content);
    let layout = serde_json::from_str::<LlmLayout>(&assistant_content).map_err(|error| {
        let message = format!("{} layout JSON could not be parsed: {error}", provider.name);
        let log_path = write_llm_call_log(
            log_context,
            LlmLogOutcome {
                success: false,
                error: Some(message.clone()),
                http_status: Some(status.as_u16()),
                generation_id: generation_id.clone(),
                response_text: Some(response_text.clone()),
                assistant_content: Some(assistant_content.clone()),
                ..LlmLogOutcome::default()
            },
        );
        with_log_path(message, log_path)
    })?;
    let parsed_layout_blocks = layout.blocks.len();

    Ok(ParsedLayoutResponse {
        layout,
        http_status: status.as_u16(),
        generation_id,
        response_text,
        assistant_content,
        parsed_layout_blocks,
    })
}

fn apply_layout_to_blocks(blocks: Vec<LiquidBlock>, layout: LlmLayout) -> Vec<LiquidBlock> {
    let llm_map: HashMap<usize, LlmBlock> = layout
        .blocks
        .into_iter()
        .map(|block| (block.source_index, block))
        .collect();

    let mut result = Vec::with_capacity(blocks.len());
    for (idx, mut block) in blocks.into_iter().enumerate() {
        if idx == 0 {
            result.push(block);
            continue;
        }
        if looks_like_exam_metadata(&block.text) {
            if idx > 1 {
                push_section_break_if_needed(&mut result);
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
                push_section_break_if_needed(&mut result);
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
                .filter(|label| !label.trim().is_empty())
                .map(str::to_owned)
                .or_else(|| label_for_block(block.role, &block.text));
        }
        result.push(block);
    }

    result
}

fn write_liquid_llm_log(log: &LiquidLlmLog) -> Option<PathBuf> {
    if !ENABLE_DETAILED_LLM_LOGGING {
        return None;
    }
    let dir = app_data_dir()?.join("liquid-logs");
    std::fs::create_dir_all(&dir).ok()?;

    if let Ok(read_dir) = std::fs::read_dir(&dir) {
        let mut entries: Vec<_> = read_dir
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
            .collect();
        if entries.len() > 50 {
            entries.sort_by_key(|entry| {
                entry
                    .metadata()
                    .ok()
                    .and_then(|metadata| metadata.modified().ok())
                    .unwrap_or(UNIX_EPOCH)
            });
            let to_delete = entries.len() - 50;
            for old in entries.into_iter().take(to_delete) {
                let _ = std::fs::remove_file(old.path());
            }
        }
    }

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
