use super::types::{Message, ToolCall, ToolCallFunction, ToolDef};
use super::LlmProvider;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Adapter for Google's native Generative Language API (`generativelanguage.googleapis.com`)
/// — a real BYOK cloud provider, authenticated with the *user's own* Gemini API key. Talks to
/// Gemini's own `generateContent` endpoint and function-calling shape directly (not the
/// OpenAI-compatibility shim Gemini also exposes) — same "genuinely adapt the provider's real
/// wire format" approach `anthropic.rs` takes, for the same reason: the native API is the one
/// that's actually guaranteed to track Gemini's current capabilities.
pub struct GeminiClient {
    http: reqwest::Client,
    api_key: Option<String>,
    model: String,
}

impl GeminiClient {
    pub fn new(api_key: Option<String>, model: String) -> Self {
        Self { http: reqwest::Client::new(), api_key, model }
    }
}

/// A single content part, both directions. Gemini's `contents[].parts` array can mix plain
/// text with a function call or function result within the same turn — modeled as an enum
/// with `#[serde(untagged)]` (Gemini's wire format doesn't tag parts with a `type` field the
/// way Anthropic's blocks do; the field *names present* disambiguate the variant instead).
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(untagged)]
enum Part {
    Text { text: String },
    FunctionCall { #[serde(rename = "functionCall")] function_call: FunctionCall },
    FunctionResponse { #[serde(rename = "functionResponse")] function_response: FunctionResponse },
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct FunctionCall {
    name: String,
    #[serde(default)]
    args: Value,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct FunctionResponse {
    name: String,
    response: Value,
}

#[derive(Debug, Serialize)]
struct Content {
    role: String, // "user" | "model" — no "system" or "tool" role in this API
    parts: Vec<Part>,
}

#[derive(Debug, Serialize)]
struct SystemInstruction {
    parts: Vec<Part>,
}

#[derive(Debug, Serialize)]
struct FunctionDeclaration<'a> {
    name: &'a str,
    description: &'a str,
    parameters: &'a Value,
}

#[derive(Debug, Serialize)]
struct Tool<'a> {
    #[serde(rename = "functionDeclarations")]
    function_declarations: Vec<FunctionDeclaration<'a>>,
}

#[derive(Debug, Serialize)]
struct GenerateContentRequest<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    system_instruction: Option<SystemInstruction>,
    contents: Vec<Content>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<Tool<'a>>,
}

#[derive(Debug, Deserialize)]
struct GenerateContentResponse {
    #[serde(default)]
    candidates: Vec<Candidate>,
}

#[derive(Debug, Deserialize)]
struct Candidate {
    content: CandidateContent,
}

#[derive(Debug, Deserialize)]
struct CandidateContent {
    #[serde(default)]
    parts: Vec<Part>,
}

/// Converts internal history into Gemini's `contents`/`parts` shape:
/// - `system`-role messages get pulled out into the top-level `system_instruction` (there can
///   be more than one — same "concatenate them" approach `anthropic.rs` uses).
/// - `user` and `assistant` messages become `user`/`model` content turns respectively.
/// - Each assistant `tool_calls` entry becomes a `FunctionCall` part.
/// - Each following `tool`-role message becomes a `FunctionResponse` part, matched to its
///   call **by name**, not a synthesized id — Gemini's function-calling protocol correlates
///   call and response by function name, so (unlike the Anthropic/OpenAI-compatible adapters)
///   there's no id bookkeeping needed here at all.
/// - Consecutive tool-result messages collapse into one `user` turn containing multiple
///   `FunctionResponse` parts, matching how Gemini expects parallel tool results.
fn to_gemini_request<'a>(messages: &[Message], tools: &'a [ToolDef]) -> GenerateContentRequest<'a> {
    let mut system_parts = Vec::new();
    let mut out: Vec<Content> = Vec::new();

    for m in messages {
        match m.role.as_str() {
            "system" => system_parts.push(m.content.clone()),
            "user" => out.push(Content { role: "user".into(), parts: vec![Part::Text { text: m.content.clone() }] }),
            "assistant" => {
                let mut parts = Vec::new();
                if !m.content.is_empty() {
                    parts.push(Part::Text { text: m.content.clone() });
                }
                if let Some(calls) = &m.tool_calls {
                    for c in calls {
                        parts.push(Part::FunctionCall {
                            function_call: FunctionCall { name: c.function.name.clone(), args: c.function.arguments.clone() },
                        });
                    }
                }
                out.push(Content { role: "model".into(), parts });
            }
            "tool" => {
                let name = m.tool_name.clone().unwrap_or_default();
                let part = Part::FunctionResponse {
                    function_response: FunctionResponse { name, response: json!({ "result": m.content }) },
                };
                let collapse_into_last = matches!(
                    out.last(),
                    Some(last) if last.role == "user" && matches!(last.parts.first(), Some(Part::FunctionResponse { .. }))
                );
                if collapse_into_last {
                    out.last_mut().unwrap().parts.push(part);
                } else {
                    out.push(Content { role: "user".into(), parts: vec![part] });
                }
            }
            _ => {}
        }
    }

    let gemini_tools = if tools.is_empty() {
        Vec::new()
    } else {
        vec![Tool {
            function_declarations: tools
                .iter()
                .map(|t| FunctionDeclaration {
                    name: &t.function.name,
                    description: &t.function.description,
                    parameters: &t.function.parameters,
                })
                .collect(),
        }]
    };

    GenerateContentRequest {
        system_instruction: if system_parts.is_empty() {
            None
        } else {
            Some(SystemInstruction { parts: vec![Part::Text { text: system_parts.join("\n\n") }] })
        },
        contents: out,
        tools: gemini_tools,
    }
}

fn from_gemini_response(resp: GenerateContentResponse) -> anyhow::Result<Message> {
    let candidate = resp.candidates.into_iter().next().ok_or_else(|| anyhow::anyhow!("Gemini returned no candidates"))?;
    let mut text = String::new();
    let mut tool_calls = Vec::new();
    for part in candidate.content.parts {
        match part {
            Part::Text { text: t } => {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(&t);
            }
            Part::FunctionCall { function_call } => {
                tool_calls.push(ToolCall { function: ToolCallFunction { name: function_call.name, arguments: function_call.args } });
            }
            Part::FunctionResponse { .. } => {} // never appears in a response
        }
    }
    Ok(Message {
        role: "assistant".to_string(),
        content: text,
        tool_calls: if tool_calls.is_empty() { None } else { Some(tool_calls) },
        tool_name: None,
        images: Vec::new(),
                display: None,
    thinking: String::new(),
    at: None,
    })
}

#[async_trait]
impl LlmProvider for GeminiClient {
    async fn chat(&self, messages: &[Message], tools: &[ToolDef]) -> anyhow::Result<Message> {
        let api_key = self.api_key.as_deref().ok_or_else(|| anyhow::anyhow!("Gemini backend configured with no api_key"))?;
        let req = to_gemini_request(messages, tools);
        let url = format!("https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent", self.model);

        let resp = self.http.post(&url).header("x-goog-api-key", api_key).json(&req).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Gemini API returned {status}: {body}");
        }
        let parsed: GenerateContentResponse = resp.json().await?;
        from_gemini_response(parsed)
    }

    /// Always true — this is the real Gemini cloud API, authenticated with the user's own
    /// API key, not a local model.
    fn is_cloud(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        "gemini"
    }

    // No embed() override — Gemini does have an embeddings endpoint (`embedContent`), but
    // it's a separate, unrequested surface; picking Gemini for `[embedding]` falls through to
    // the trait's default "does not support embeddings" error until that's actually needed.
}
