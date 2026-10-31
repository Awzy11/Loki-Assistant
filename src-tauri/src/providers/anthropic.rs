use super::types::{Message, ToolCall, ToolCallFunction, ToolDef};
use super::LlmProvider;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Adapter for Anthropic's Messages API (`api.anthropic.com`) — a real BYOK cloud provider,
/// authenticated with the *user's own* Anthropic API key.
///
/// **Not the same thing as `crate::claude::ClaudeClient`.** That's the user's personal `claude`
/// CLI subscription shell-out (no API key, billed under his Pro/Max login, personal-use-only,
/// never selectable here — see `providers::LlmProvider`'s doc comment). This file talks to
/// Anthropic's HTTP API directly with `reqwest`, using whatever key the user configured in
/// `[llm]`/`[embedding]`. Same company, sometimes the same model family — completely
/// different auth model and code path. Don't merge these two files.
pub struct AnthropicClient {
    http: reqwest::Client,
    api_key: Option<String>,
    model: String,
}

impl AnthropicClient {
    pub fn new(api_key: Option<String>, model: String) -> Self {
        Self { http: reqwest::Client::new(), api_key, model }
    }
}

const ANTHROPIC_VERSION: &str = "2023-06-01";
const MAX_TOKENS: u32 = 4096;

/// A single content block, both directions — Anthropic messages are arrays of these, not
/// plain strings, and the "system" prompt is a top-level field rather than a message role.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Block {
    Text { text: String },
    ToolUse { id: String, name: String, input: Value },
    ToolResult { tool_use_id: String, content: String },
}

#[derive(Debug, Serialize)]
struct AnthropicMessage {
    role: String, // "user" | "assistant" — no "system" or "tool" role in this API
    content: Vec<Block>,
}

#[derive(Debug, Serialize)]
struct AnthropicTool<'a> {
    name: &'a str,
    description: &'a str,
    input_schema: &'a Value,
}

#[derive(Debug, Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<AnthropicTool<'a>>,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    content: Vec<Block>,
}

/// Converts internal history into Anthropic's message/block shape:
/// - `system`-role messages get pulled out into the top-level `system` string (concatenated —
///   there can be more than one, e.g. `agent::run_turn`'s last-round "out of budget" notice).
/// - Each assistant `tool_calls` entry becomes a `ToolUse` block with a synthesized id.
/// - Each following `tool`-role message becomes a `ToolResult` block, matched to that id by
///   position (same positional-correlation approach as the OpenAI-compatible adapter, for the
///   same reason: our internal `Message` type doesn't carry provider-specific ids).
/// - Consecutive tool-result messages collapse into one `user` turn containing multiple
///   `ToolResult` blocks, since that's the shape Anthropic expects for parallel tool calls.
fn to_anthropic_request<'a>(model: &'a str, messages: &[Message], tools: &'a [ToolDef]) -> ChatRequest<'a> {
    let mut system_parts = Vec::new();
    let mut out: Vec<AnthropicMessage> = Vec::new();
    let mut next_id = 0usize;
    let mut pending_ids: Vec<String> = Vec::new();

    for m in messages {
        match m.role.as_str() {
            "system" => system_parts.push(m.content.clone()),
            "user" => {
                out.push(AnthropicMessage { role: "user".into(), content: vec![Block::Text { text: m.content.clone() }] })
            }
            "assistant" => {
                let mut blocks = Vec::new();
                if !m.content.is_empty() {
                    blocks.push(Block::Text { text: m.content.clone() });
                }
                if let Some(calls) = &m.tool_calls {
                    for c in calls {
                        let id = format!("toolu_{next_id}");
                        next_id += 1;
                        pending_ids.push(id.clone());
                        blocks.push(Block::ToolUse {
                            id,
                            name: c.function.name.clone(),
                            input: c.function.arguments.clone(),
                        });
                    }
                }
                out.push(AnthropicMessage { role: "assistant".into(), content: blocks });
            }
            "tool" => {
                let id = if !pending_ids.is_empty() {
                    pending_ids.remove(0)
                } else {
                    let id = format!("toolu_{next_id}");
                    next_id += 1;
                    id
                };
                let block = Block::ToolResult { tool_use_id: id, content: m.content.clone() };
                let collapse_into_last = matches!(
                    out.last(),
                    Some(last) if last.role == "user" && matches!(last.content.first(), Some(Block::ToolResult { .. }))
                );
                if collapse_into_last {
                    out.last_mut().unwrap().content.push(block);
                } else {
                    out.push(AnthropicMessage { role: "user".into(), content: vec![block] });
                }
            }
            _ => {}
        }
    }

    let anthropic_tools = tools
        .iter()
        .map(|t| AnthropicTool {
            name: &t.function.name,
            description: &t.function.description,
            input_schema: &t.function.parameters,
        })
        .collect();

    ChatRequest {
        model,
        max_tokens: MAX_TOKENS,
        system: if system_parts.is_empty() { None } else { Some(system_parts.join("\n\n")) },
        messages: out,
        tools: anthropic_tools,
    }
}

fn from_anthropic_response(blocks: Vec<Block>) -> Message {
    let mut text = String::new();
    let mut tool_calls = Vec::new();
    for block in blocks {
        match block {
            Block::Text { text: t } => {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(&t);
            }
            Block::ToolUse { name, input, .. } => {
                tool_calls.push(ToolCall { function: ToolCallFunction { name, arguments: input } });
            }
            Block::ToolResult { .. } => {} // never appears in a response
        }
    }
    Message {
        role: "assistant".to_string(),
        content: text,
        tool_calls: if tool_calls.is_empty() { None } else { Some(tool_calls) },
        tool_name: None,
        images: Vec::new(),
        display: None,
    thinking: String::new(),
    at: None,
    }
}

#[async_trait]
impl LlmProvider for AnthropicClient {
    async fn chat(&self, messages: &[Message], tools: &[ToolDef]) -> anyhow::Result<Message> {
        let api_key = self
            .api_key
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("Anthropic backend configured with no api_key"))?;
        let req = to_anthropic_request(&self.model, messages, tools);

        let resp = self
            .http
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .json(&req)
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Anthropic API returned {status}: {body}");
        }
        let parsed: ChatResponse = resp.json().await?;
        Ok(from_anthropic_response(parsed.content))
    }

    /// Always true — this is the real Anthropic cloud API, not the personal CLI subscription.
    fn is_cloud(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        "anthropic"
    }

    // No embed() override — Anthropic has no embeddings API. The trait's default
    // implementation returns a clear "does not support embeddings" error, which is exactly
    // right here: picking Anthropic for `[embedding]` is a config mistake, not something to
    // silently paper over.
}
