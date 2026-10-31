use super::types::{Message, ToolCall, ToolCallFunction, ToolDef, ToolFunctionDef};
use super::LlmProvider;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One adapter covers OpenAI itself *and* any self-hosted OpenAI-wire-compatible endpoint
/// (vLLM, LM Studio, etc.) — the wire format is identical, only `base_url`/`api_key` differ.
pub struct OpenAiCompatibleClient {
    http: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
    model: String,
}

impl OpenAiCompatibleClient {
    pub fn new(base_url: String, api_key: Option<String>, model: String) -> Self {
        Self { http: reqwest::Client::new(), base_url, api_key, model }
    }
}

// --- Wire format (/v1/chat/completions) ---

#[derive(Debug, Serialize)]
struct OaiTool<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    function: &'a ToolFunctionDef,
}

#[derive(Debug, Serialize)]
struct OaiMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OaiToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct OaiToolCall {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    function: OaiToolCallFunction,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct OaiToolCallFunction {
    name: String,
    /// JSON-encoded string, unlike our internal `arguments: Value` — OpenAI's wire format
    /// wants tool-call arguments as text, not a nested JSON object.
    arguments: String,
}

#[derive(Debug, Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<OaiMessage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<OaiTool<'a>>,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: OaiResponseMessage,
}

#[derive(Debug, Deserialize)]
struct OaiResponseMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<OaiToolCall>>,
}

/// Converts internal history into OpenAI wire messages, synthesizing `tool_call_id`s
/// positionally: each assistant message's `tool_calls[i]` gets an id, and the tool-result
/// messages immediately following it in our history are assumed to correspond in the same
/// order — true by construction, since `agent::run_turn` executes and appends tool results
/// in the same order the model requested them. This lets our internal `Message` type stay
/// id-free (Ollama's wire format doesn't use ids, only `tool_name`) while still round-tripping
/// correctly through a provider that requires them.
fn to_oai_messages(messages: &[Message]) -> Vec<OaiMessage> {
    let mut out = Vec::with_capacity(messages.len());
    let mut pending_ids: Vec<String> = Vec::new();
    let mut next_id = 0usize;

    for m in messages {
        match m.role.as_str() {
            "tool" => {
                let id = if !pending_ids.is_empty() {
                    pending_ids.remove(0)
                } else {
                    let id = format!("call_{next_id}");
                    next_id += 1;
                    id
                };
                out.push(OaiMessage {
                    role: "tool".to_string(),
                    content: Some(m.content.clone()),
                    tool_calls: None,
                    tool_call_id: Some(id),
                });
            }
            "assistant" => {
                let tool_calls = m.tool_calls.as_ref().map(|calls| {
                    calls
                        .iter()
                        .map(|c| {
                            let id = format!("call_{next_id}");
                            next_id += 1;
                            pending_ids.push(id.clone());
                            OaiToolCall {
                                id,
                                kind: "function".to_string(),
                                function: OaiToolCallFunction {
                                    name: c.function.name.clone(),
                                    arguments: c.function.arguments.to_string(),
                                },
                            }
                        })
                        .collect::<Vec<_>>()
                });
                out.push(OaiMessage {
                    role: "assistant".to_string(),
                    content: if m.content.is_empty() { None } else { Some(m.content.clone()) },
                    tool_calls,
                    tool_call_id: None,
                });
            }
            role => {
                out.push(OaiMessage {
                    role: role.to_string(),
                    content: Some(m.content.clone()),
                    tool_calls: None,
                    tool_call_id: None,
                });
            }
        }
    }
    out
}

fn from_oai_response(msg: OaiResponseMessage) -> Message {
    let tool_calls = msg.tool_calls.map(|calls| {
        calls
            .into_iter()
            .map(|c| ToolCall {
                function: ToolCallFunction {
                    name: c.function.name,
                    arguments: serde_json::from_str::<Value>(&c.function.arguments)
                        .unwrap_or(Value::String(c.function.arguments)),
                },
            })
            .collect()
    });
    Message { role: "assistant".to_string(), content: msg.content.unwrap_or_default(), tool_calls, tool_name: None, images: Vec::new(), display: None, thinking: String::new(), at: None }
}

// --- Embeddings (/v1/embeddings) ---

#[derive(Debug, Serialize)]
struct EmbedRequest<'a> {
    model: &'a str,
    input: &'a str,
}

#[derive(Debug, Deserialize)]
struct EmbedResponse {
    data: Vec<EmbedData>,
}

#[derive(Debug, Deserialize)]
struct EmbedData {
    embedding: Vec<f32>,
}

#[async_trait]
impl LlmProvider for OpenAiCompatibleClient {
    async fn chat(&self, messages: &[Message], tools: &[ToolDef]) -> anyhow::Result<Message> {
        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let oai_tools = tools.iter().map(|t| OaiTool { kind: "function", function: &t.function }).collect();
        let req = ChatRequest { model: &self.model, messages: to_oai_messages(messages), tools: oai_tools };

        let mut builder = self.http.post(&url).json(&req);
        if let Some(key) = &self.api_key {
            builder = builder.bearer_auth(key);
        }
        let resp = builder.send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("OpenAI-compatible endpoint returned {status}: {body}");
        }
        let parsed: ChatResponse = resp.json().await?;
        let choice = parsed.choices.into_iter().next().ok_or_else(|| anyhow::anyhow!("no choices in response"))?;
        Ok(from_oai_response(choice.message))
    }

    /// Conservative default: an OpenAI-compatible endpoint might be a real cloud vendor
    /// (OpenAI itself) or a self-hosted local one (vLLM/LM Studio on the LAN) — this adapter
    /// can't tell which from the config shape alone. Treating it as cloud is the fail-closed
    /// direction: a local endpoint gets over-cautiously blocked from receiving local-only
    /// content it would've been fine to see, which is a safe way for this to be wrong, not
    /// a dangerous one.
    fn is_cloud(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        "openai_compatible"
    }

    async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        let url = format!("{}/embeddings", self.base_url.trim_end_matches('/'));
        let mut builder = self.http.post(&url).json(&EmbedRequest { model: &self.model, input: text });
        if let Some(key) = &self.api_key {
            builder = builder.bearer_auth(key);
        }
        let resp = builder.send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("embeddings request failed: {}", resp.status());
        }
        let parsed: EmbedResponse = resp.json().await?;
        parsed.data.into_iter().next().map(|d| d.embedding).ok_or_else(|| anyhow::anyhow!("no embedding returned"))
    }
}
