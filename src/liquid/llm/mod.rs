//! LLM refinement layer (optional).
//!
//! Sub-structure planned:
//! - prompt.rs   - prompt construction and block selection/compacting
//! - client.rs   - HTTP, providers (Groq/OpenRouter), response parsing
//! - layout.rs   - apply_llm_layout orchestration and response application
//! - logging.rs  - detailed request/response/error logging
//! - types.rs    - internal Llm* types
//!
//! System + user prompts now live in prompt.rs.
//! Only advertises fields the pipeline actually supports: role, label, action=remove,
//! visual_break_before. No legacy _legacy_* prompt strings remain.

pub mod client;
mod layout;
pub mod prompt;

pub(crate) use layout::apply_llm_layout;
