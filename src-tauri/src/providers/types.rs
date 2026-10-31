use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Provider-agnostic message/tool-calling vocabulary — an OpenAI/Ollama-style JSON-schema
/// tool-calling shape, kept as the app's internal lingua franca. Per-provider adapters
/// (providers/ollama.rs, and later providers/openai_compatible.rs, providers/anthropic.rs)
/// translate to/from each provider's own wire format around these types, rather than the
/// rest of the app knowing any one provider's shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallFunction {
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub function: ToolCallFunction,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Message {
    pub role: String, // "system" | "user" | "assistant" | "tool"
    #[serde(default)]
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    /// Base64-encoded image data (no data: URI prefix) attached to this message — e.g. a
    /// device-context screenshot (#13). Field name matches Ollama's `/api/chat` wire format
    /// exactly, so `OllamaClient` needs no adapter code to forward these. Other providers
    /// (openai_compatible, anthropic) currently ignore this field — only the Ollama path
    /// (the active daily-driver backend) supports vision today.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
    /// What to show a person instead of `content` — set when `content` is an expanded slash
    /// command (`/bod` → the full instructions). Never sent to a model or provider.
    #[serde(skip)]
    pub display: Option<String>,
    /// The model's reasoning trace for this reply (Ollama `thinking`), kept for audits and
    /// troubleshooting only — never sent back to a model.
    #[serde(default, skip_serializing)]
    pub thinking: String,
    /// Local wall-clock time the message was sent/received (`YYYY-MM-DD HH:MM:SS TZ`).
    #[serde(skip)]
    pub at: Option<String>,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self { role: "system".into(), content: content.into(), tool_calls: None, tool_name: None, images: Vec::new(), display: None, thinking: String::new(), at: None }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self { role: "user".into(), content: content.into(), tool_calls: None, tool_name: None, images: Vec::new(), display: None, thinking: String::new(), at: None }
    }
    /// Same as `user`, plus one or more base64-encoded images (e.g. a device-context
    /// screenshot, #13) attached to the message.
    pub fn user_with_images(content: impl Into<String>, images: Vec<String>) -> Self {
        Self { role: "user".into(), content: content.into(), tool_calls: None, tool_name: None, images, display: None, thinking: String::new(), at: None }
    }
    /// Used by `core.rs`'s Routine Mode relay to append a relayed Claude reply directly to
    /// history — the normal `agent::run_turn` path gets its assistant messages straight from
    /// the provider's own response, which already carries this role, so this constructor is
    /// the routine-mode-specific case, not something `run_turn` itself needs.
    pub fn assistant(content: impl Into<String>) -> Self {
        Self { role: "assistant".into(), content: content.into(), tool_calls: None, tool_name: None, images: Vec::new(), display: None, thinking: String::new(), at: None }
    }
    pub fn tool_result(tool_name: impl Into<String>, content: impl Into<String>) -> Self {
        Self { role: "tool".into(), content: content.into(), tool_calls: None, tool_name: Some(tool_name.into()), images: Vec::new(), display: None, thinking: String::new(), at: None }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolFunctionDef {
    pub name: String,
    pub description: String,
    pub parameters: Value, // JSON schema
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolDef {
    #[serde(rename = "type")]
    pub kind: String, // always "function"
    pub function: ToolFunctionDef,
}

impl ToolDef {
    pub fn function(name: &str, description: &str, parameters: Value) -> Self {
        Self {
            kind: "function".into(),
            function: ToolFunctionDef { name: name.into(), description: description.into(), parameters },
        }
    }
}
