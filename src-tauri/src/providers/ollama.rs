use super::types::{Message, ToolDef};
use super::LlmProvider;
use crate::config::OllamaConfig;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Context window (tokens) sent with every chat request, set once at startup from `[context]
/// num_ctx`. Every Loki call uses the same value, so Ollama never reloads the model to change it.
/// Whether the main chat path lets the model reason before answering (`[agent] local_reasoning`).
/// Off by default: with a 35B reasoning model, agent turns spent minutes per tool round thinking
/// (re-deriving the date, second-guessing file names). Hard problems go through the sequential-
/// thinking MCP tool instead, which keeps reasoning visible and bounded.
static REASONING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_reasoning(on: bool) {
    REASONING.store(on, std::sync::atomic::Ordering::Relaxed);
}

fn chat_think() -> Option<bool> {
    if REASONING.load(std::sync::atomic::Ordering::Relaxed) { None } else { Some(false) }
}

static NUM_CTX: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

pub fn set_num_ctx(n: usize) {
    NUM_CTX.store(n, std::sync::atomic::Ordering::Relaxed);
}

/// Loads `model` on the Ollama host at startup so the first real request isn't a cold start, and
/// pins it (`keep_alive: -1`). One-shot: staying loaded afterwards is the server's job
/// (`OLLAMA_KEEP_ALIVE=-1`) plus every client sending `keep_alive: -1`. If the model is already
/// loaded this adopts its current window size, because asking for a different `num_ctx` would make
/// Ollama reload it.
pub fn spawn_preload(base_url: String, model: String, preferred_ctx: usize) {
    if tokio::runtime::Handle::try_current().is_err() {
        return;
    }
    tokio::spawn(async move {
        let http = reqwest::Client::builder().timeout(std::time::Duration::from_secs(300)).build().unwrap_or_default();
        match keep_warm_once(&http, &base_url, &model, preferred_ctx).await {
            Some(note) => eprintln!("[loki-debug] preload: {note}"),
            None => eprintln!("[loki-debug] preload: Ollama at {base_url} didn't answer (first request will load the model)"),
        }
    });
}

async fn keep_warm_once(http: &reqwest::Client, base: &str, model: &str, preferred_ctx: usize) -> Option<String> {
    let ps: serde_json::Value = http.get(format!("{base}/api/ps")).send().await.ok()?.json().await.ok()?;
    let loaded_ctx = ps["models"]
        .as_array()
        .and_then(|a| a.iter().find(|m| m["name"] == model || m["model"] == model))
        .and_then(|m| m["context_length"].as_u64());
    let ctx = loaded_ctx.map(|c| c as usize).unwrap_or(preferred_ctx);
    let mut body = serde_json::json!({ "model": model, "prompt": "", "stream": false, "keep_alive": -1 });
    if ctx > 0 {
        body["options"] = serde_json::json!({ "num_ctx": ctx });
    }
    let ok = http.post(format!("{base}/api/generate")).json(&body).send().await.ok()?.status().is_success();
    ok.then(|| if loaded_ctx.is_some() { format!("{model} stays loaded (window {ctx})") } else { format!("loaded {model} (window {ctx})") })
}

#[derive(Debug, Serialize)]
struct RequestOptions {
    num_ctx: usize,
}

fn request_options() -> Option<RequestOptions> {
    match NUM_CTX.load(std::sync::atomic::Ordering::Relaxed) {
        0 => None,
        n => Some(RequestOptions { num_ctx: n }),
    }
}

#[derive(Debug, Serialize)]
struct ChatRequest<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<RequestOptions>,
    model: &'a str,
    messages: &'a [Message],
    tools: &'a [ToolDef],
    stream: bool,
    /// Keeps the model resident in memory between calls. Ollama's default unload-after-5min
    /// means every message after a short pause pays a ~45s cold-load penalty (measured against
    /// qwen3.6:35b-a3b on the GPU workstation, 2026-08-06) — 30m keeps it warm for a normal work session.
    keep_alive: i64,
    /// `None` lets the model use its own default (extended "thinking" reasoning enabled, for
    /// the main conversational path — that reasoning is the point there). `Some(false)` is
    /// used only by `chat_low_effort` (see below): measured against real vault content, a
    /// thinking-enabled call for a trivial classification task (auto-tagging) took 67s;
    /// `think: false` on the identical prompt/content took 0.9s with a comparably good
    /// answer — a real fix for a real bottleneck at bulk-import scale, not a quality
    /// compromise on the path where reasoning quality actually matters.
    #[serde(skip_serializing_if = "Option::is_none")]
    think: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    message: Message,
    /// Final chunk only: tokens the model read for this request / wrote in its answer.
    #[serde(default)]
    prompt_eval_count: Option<u64>,
    #[serde(default)]
    eval_count: Option<u64>,
}

#[derive(Debug, serde::Serialize)]
struct EmbedRequest<'a> {
    model: &'a str,
    input: &'a str,
}

#[derive(Debug, Deserialize)]
struct EmbedResponse {
    embeddings: Vec<Vec<f32>>,
}

pub struct OllamaClient {
    http: reqwest::Client,
    config: OllamaConfig,
    /// Token counts from the last chat call, handed to the agent loop by `take_usage`.
    last_usage: std::sync::Mutex<Option<serde_json::Value>>,
}

impl OllamaClient {
    pub fn new(config: OllamaConfig) -> Self {
        Self { http: reqwest::Client::new(), config, last_usage: std::sync::Mutex::new(None) }
    }

    /// Streaming variant of `chat_impl`: Ollama's `stream: true` mode returns NDJSON, one
    /// partial `message` per line. Text accumulates; `tool_calls` arrive whole in whichever
    /// chunk carries them.
    async fn chat_stream_impl(
        &self,
        messages: &[Message],
        tools: &[ToolDef],
        on_delta: &mut (dyn for<'s> FnMut(&'s str) + Send),
    ) -> anyhow::Result<Message> {
        use futures_util::StreamExt;
        let url = format!("{}/api/chat", self.config.base_url());
        let req = ChatRequest { model: &self.config.model, messages, tools, stream: true, keep_alive: -1, think: chat_think(), options: request_options() };
        let resp = self.http.post(&url).json(&req).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Ollama returned {status}: {body}");
        }
        let mut out = Message::assistant(String::new());
        let mut buf: Vec<u8> = Vec::new();
        let mut bytes = resp.bytes_stream();
        let handle_line = |line: &[u8], out: &mut Message, on_delta: &mut (dyn for<'s> FnMut(&'s str) + Send)| -> anyhow::Result<()> {
            if line.iter().all(|b| b.is_ascii_whitespace()) {
                return Ok(());
            }
            let raw: serde_json::Value = serde_json::from_slice(line)?;
            if let Some(err) = raw.get("error") {
                let msg = err.as_str().map(str::to_string).unwrap_or_else(|| err.to_string());
                anyhow::bail!("Ollama: {msg}");
            }
            let chunk: ChatResponse = serde_json::from_value(raw)?;
            if let Some(prompt) = chunk.prompt_eval_count {
                *self.last_usage.lock().unwrap() = Some(serde_json::json!({ "kind": "context", "promptTokens": prompt, "completionTokens": chunk.eval_count.unwrap_or(0) }));
            }
            if !chunk.message.content.is_empty() {
                on_delta(&chunk.message.content);
                out.content.push_str(&chunk.message.content);
            }
            if !chunk.message.thinking.is_empty() {
                out.thinking.push_str(&chunk.message.thinking);
            }
            if let Some(calls) = chunk.message.tool_calls {
                out.tool_calls.get_or_insert_with(Vec::new).extend(calls);
            }
            Ok(())
        };
        while let Some(piece) = bytes.next().await {
            buf.extend_from_slice(&piece?);
            while let Some(pos) = buf.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = buf.drain(..=pos).collect();
                handle_line(&line, &mut out, on_delta)?;
            }
        }
        handle_line(&buf, &mut out, on_delta)?;
        Ok(out)
    }

    async fn chat_impl(&self, messages: &[Message], tools: &[ToolDef], think: Option<bool>) -> anyhow::Result<Message> {
        let url = format!("{}/api/chat", self.config.base_url());
        let req = ChatRequest { model: &self.config.model, messages, tools, stream: false, keep_alive: -1, think, options: request_options() };

        let resp = self.http.post(&url).json(&req).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Ollama returned {status}: {body}");
        }
        let parsed: ChatResponse = resp.json().await?;
        Ok(parsed.message)
    }
}

#[async_trait]
impl LlmProvider for OllamaClient {
    /// Sends the conversation + available tools to the model, returns its reply
    /// (which may itself contain tool_calls the agent loop needs to execute).
    async fn chat(&self, messages: &[Message], tools: &[ToolDef]) -> anyhow::Result<Message> {
        self.chat_impl(messages, tools, None).await
    }

    async fn chat_stream(
        &self,
        messages: &[Message],
        tools: &[ToolDef],
        on_delta: &mut (dyn for<'s> FnMut(&'s str) + Send),
    ) -> anyhow::Result<Message> {
        self.chat_stream_impl(messages, tools, on_delta).await
    }

    /// See the `think` field's doc comment on `ChatRequest` — this is the 67s-vs-0.9s fix,
    /// used by low-stakes classification-shaped calls (today: `knowledge::tags::auto_tag`)
    /// rather than the main conversational path.
    async fn chat_low_effort(&self, messages: &[Message]) -> anyhow::Result<Message> {
        self.chat_impl(messages, &[], Some(false)).await
    }

    /// Ollama runs locally — never sends data off this machine.
    fn is_cloud(&self) -> bool {
        false
    }

    fn take_usage(&self) -> Option<serde_json::Value> {
        self.last_usage.lock().unwrap().take()
    }

    fn name(&self) -> &str {
        "ollama"
    }

    /// Uses whatever model this `OllamaClient` was configured with — for the `[embedding]`
    /// section that's a real embedding model (e.g. `nomic-embed-text`), a separate config
    /// instance from whatever chat model `[llm]` points at, even though both are "ollama".
    async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        let url = format!("{}/api/embed", self.config.base_url());
        let resp = self
            .http
            .post(&url)
            .json(&EmbedRequest { model: &self.config.model, input: text })
            .send()
            .await?;
        if !resp.status().is_success() {
            anyhow::bail!("embed failed: {}", resp.status());
        }
        let parsed: EmbedResponse = resp.json().await?;
        parsed.embeddings.into_iter().next().ok_or_else(|| anyhow::anyhow!("embed returned no vectors"))
    }
}
