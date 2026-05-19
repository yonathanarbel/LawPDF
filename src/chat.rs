use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use crossbeam_channel::Sender;
use serde_json::json;

const OPENROUTER_URL: &str = "https://openrouter.ai/api/v1/chat/completions";

#[derive(Debug, Clone, Copy)]
pub struct ChatModel {
    pub label: &'static str,
    pub id: &'static str,
}

pub const CHAT_MODELS: [ChatModel; 5] = [
    ChatModel {
        label: "DeepSeek V4 Pro",
        id: "deepseek/deepseek-v4-pro",
    },
    ChatModel {
        label: "DeepSeek V4 Flash",
        id: "deepseek/deepseek-v4-flash",
    },
    ChatModel {
        label: "GPT-OSS 120B Free",
        id: "openai/gpt-oss-120b:free",
    },
    ChatModel {
        label: "GPT-OSS 20B Free",
        id: "openai/gpt-oss-20b:free",
    },
    ChatModel {
        label: "Qwen3 Next 80B Free",
        id: "qwen/qwen3-next-80b-a3b-instruct:free",
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatRole {
    User,
    Assistant,
}

impl ChatRole {
    fn api_role(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub document_epoch: u64,
    pub path: PathBuf,
    pub api_key: String,
    pub model: String,
    pub visible_messages: Vec<ChatMessage>,
    pub document_context: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ChatEvent {
    pub document_epoch: u64,
    pub path: PathBuf,
    pub result: Result<String, String>,
}

pub fn spawn_chat_job(request: ChatRequest, tx: Sender<ChatEvent>) {
    thread::spawn(move || {
        let document_epoch = request.document_epoch;
        let path = request.path.clone();
        let result = run_chat_request(request);
        let _ = tx.send(ChatEvent {
            document_epoch,
            path,
            result,
        });
    });
}

fn run_chat_request(request: ChatRequest) -> Result<String, String> {
    let mut messages = Vec::new();
    messages.push(json!({
        "role": "system",
        "content": "You are LawPDF Chat. Answer only from the supplied PDF text and the user's follow-up questions. Cite page numbers when useful. If the PDF text does not contain the answer, say so plainly."
    }));

    let mut context_attached = false;
    for message in request.visible_messages {
        let mut content = message.content;
        if !context_attached && message.role == ChatRole::User {
            if let Some(context) = request.document_context.as_ref() {
                content = format!(
                    "PDF text for this chat follows. Use it as the source of truth.\n\n{context}\n\nUser question:\n{content}"
                );
                context_attached = true;
            }
        }
        messages.push(json!({
            "role": message.role.api_role(),
            "content": content,
        }));
    }

    let body = json!({
        "model": request.model,
        "messages": messages,
        "temperature": 0.2,
    });

    let response = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|error| format!("Could not create chat client: {error}"))?
        .post(OPENROUTER_URL)
        .bearer_auth(request.api_key)
        .header("Accept", "application/json")
        .header("Accept-Encoding", "identity")
        .header("HTTP-Referer", "https://github.com/yonathanarbel/LawPDF")
        .header("X-Title", "LawPDF")
        .json(&body)
        .send()
        .map_err(|error| format!("Chat request failed: {error}"))?;

    let status = response.status();
    let response_text = response
        .text()
        .map_err(|error| format!("Could not read chat response: {error}"))?;
    if !status.is_success() {
        return Err(format!(
            "OpenRouter chat returned HTTP {status}: {}",
            preview(&response_text, 1200)
        ));
    }

    let response_json =
        serde_json::from_str::<serde_json::Value>(&response_text).map_err(|error| {
            format!(
                "OpenRouter chat response was not JSON: {error}; {}",
                preview(&response_text, 1200)
            )
        })?;
    response_json
        .pointer("/choices/0/message/content")
        .and_then(serde_json::Value::as_str)
        .map(|content| content.trim().to_owned())
        .filter(|content| !content.is_empty())
        .ok_or_else(|| "OpenRouter chat response did not include message content.".to_owned())
}

fn preview(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_owned();
    }
    let mut value = value.chars().take(max_chars).collect::<String>();
    value.push_str("...");
    value
}
