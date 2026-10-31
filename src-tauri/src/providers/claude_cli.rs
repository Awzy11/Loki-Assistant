//! Claude as a pure inference engine, on the user's own Claude subscription.
//!
//! Loki's agent loop (tools, MCP, permissions, approvals) stays in Loki; this provider only
//! turns a conversation plus tool definitions into the model's next message. It shells out to
//! `claude -p` with everything agentic switched off — no built-in tools, no MCP, no settings
//! files, no CLAUDE.md, no session persistence — so each call is one lean, stateless request.
//!
//! **Cost, measured.** The prompt cache is server-side, so separate `claude -p` processes share
//! it — but only for an *identical* prompt: when a stateless prompt grows by appending, earlier
//! message content is NOT reused (checked with single-block and multi-block input), so every
//! tool round and turn would re-pay for the whole transcript (~2x the cost of a resumed
//! session in a live comparison). So conversations run in a **resumed CLI session used purely
//! as a cache vehicle**: Loki stays the source of truth and sends only the messages the session
//! hasn't seen, after checking a fingerprint of what it holds. If anything diverges (another
//! engine answered in between, the history was rewritten, the tool set changed) it starts a
//! fresh session and replays — correctness never depends on the session's memory.
//!
//! **A CLI quirk this depends on.** By default the CLI sends first-party requests in an
//! experimental wire format whose only cache breakpoint is a trailing system-role message that
//! is re-rendered differently on the next request, so a resumed session's history was
//! re-written to the cache on every call (found by capturing the real requests with a
//! TLS-intercepting proxy). `CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS=1` restores incremental
//! caching: a follow-up turn writes tens of tokens instead of the whole transcript.
//!
//! **Tool calls.** The CLI can't take Loki's tool schemas natively, so tools are described in
//! the system prompt and the model replies either with plain prose (streamed live) or with a
//! `<<TOOL_CALLS>>` line followed by a JSON array. Invalid output gets one repair retry.
//!
//! Selecting this provider is an explicit, separate choice (`[claude] mode = "inference"` or a
//! `claude:<model>` engine) — it is not one of the BYOK API-key vendors in `LlmBackend`.

use super::types::{Message, ToolCall, ToolCallFunction, ToolDef};
use super::LlmProvider;
use crate::claude::PrivateTemp;
use async_trait::async_trait;
use serde_json::Value;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Mutex;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

pub const TOOL_SENTINEL: &str = "<<TOOL_CALLS>>";
const MAX_TOOL_RESULT_CHARS: usize = 20_000;
/// Transcript size (chars) above which Loki falls back to windowing + summary for this provider.
pub const FULL_HISTORY_BUDGET_CHARS: usize = 200_000;

/// What a resumed CLI session already contains, as far as Loki knows.
#[derive(Clone)]
struct SessionInfo {
    uuid: String,
    /// Leading non-system messages of Loki's history represented in the session, including
    /// Claude's own replies.
    consumed: usize,
    /// Fingerprint of those messages — detects rewritten or reordered history.
    fingerprint: u64,
    /// The system prompt the session was created with (tools included).
    system: String,
}

/// Shared across turns: CLI session state per (Loki conversation, model).
#[derive(Default)]
pub struct ClaudeSessions {
    map: Mutex<std::collections::HashMap<String, SessionInfo>>,
}

pub struct ClaudeCliProvider {
    binary: String,
    model: Option<String>,
    cwd: PathBuf,
    idle_timeout: Duration,
    last_usage: Mutex<Option<Value>>,
    /// `Some((state, key))` = run in a resumed session as a cache vehicle; `None` = stateless.
    sessions: Option<(std::sync::Arc<ClaudeSessions>, String)>,
}

impl ClaudeCliProvider {
    /// `model`: a CLI alias (`sonnet`, `opus`, `haiku`) or full id; `None` = the CLI's default.
    pub fn new(binary: String, model: Option<String>, idle_timeout_secs: u64) -> Self {
        Self::with_sessions(binary, model, idle_timeout_secs, None)
    }

    /// `sessions`: shared state plus this conversation's key (`"<loki session>|<model>"`).
    pub fn with_sessions(
        binary: String,
        model: Option<String>,
        idle_timeout_secs: u64,
        sessions: Option<(std::sync::Arc<ClaudeSessions>, String)>,
    ) -> Self {
        // A neutral working directory: no CLAUDE.md, memory or project settings to discover.
        let cwd = dirs::data_dir().unwrap_or_else(std::env::temp_dir).join("loki").join("claude-inference-cwd");
        let _ = std::fs::create_dir_all(&cwd);
        Self { binary, model, cwd, idle_timeout: Duration::from_secs(idle_timeout_secs.max(30)), last_usage: Mutex::new(None), sessions }
    }

    async fn run_once(
        &self,
        system: &str,
        prompt: &str,
        session: SessionFlag<'_>,
        on_delta: &mut (dyn for<'s> FnMut(&'s str) + Send),
    ) -> anyhow::Result<String> {
        let sys_file = PrivateTemp::write("loki-sys", "txt", system.as_bytes())?;
        let mut cmd = Command::new(&self.binary);
        cmd.arg("-p")
            .arg("--system-prompt-file")
            .arg(&sys_file.path)
            .args(["--tools", ""])
            .arg("--strict-mcp-config")
            .args(["--setting-sources", ""])
            .arg("--disable-slash-commands")
            .args(["--output-format", "stream-json", "--verbose", "--include-partial-messages"]);
        match session {
            SessionFlag::None => {
                cmd.arg("--no-session-persistence");
            }
            SessionFlag::New(id) => {
                cmd.args(["--session-id", id]);
            }
            SessionFlag::Resume(id) => {
                cmd.args(["--resume", id]);
            }
        }
        if let Some(m) = &self.model {
            cmd.args(["--model", m]);
        }
        // The CLI's newer experimental wire format re-renders earlier messages between
        // requests (a trailing system-role message carries the only cache breakpoint and is
        // re-rendered as a plain string on the next call), so a resumed session's history never
        // hits the prompt cache. Without the experimental betas the conversation caches
        // incrementally — verified by capturing the real requests (see the module docs).
        cmd.env("CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS", "1");
        cmd.current_dir(&self.cwd).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
        let mut child = cmd.spawn().map_err(|e| anyhow::anyhow!("couldn't run '{}': {e}", self.binary))?;

        // The transcript goes over stdin: it can exceed the per-argument size limit.
        let mut stdin = child.stdin.take().expect("piped");
        let prompt_owned = prompt.to_string();
        let writer = tokio::spawn(async move {
            let _ = stdin.write_all(prompt_owned.as_bytes()).await;
            let _ = stdin.shutdown().await;
        });
        let mut stderr = child.stderr.take().expect("piped");
        let stderr_task = tokio::spawn(async move {
            let mut s = String::new();
            let _ = stderr.read_to_string(&mut s).await;
            s
        });

        let mut lines = BufReader::new(child.stdout.take().expect("piped")).lines();
        let mut gate = StreamGate::default();
        let mut parsed = CliOutput::default();
        loop {
            match tokio::time::timeout(self.idle_timeout, lines.next_line()).await {
                Err(_) => anyhow::bail!("claude CLI timed out after {}s of silence", self.idle_timeout.as_secs()),
                Ok(Err(e)) => anyhow::bail!("reading claude output failed: {e}"),
                Ok(Ok(None)) => break,
                Ok(Ok(Some(line))) => {
                    if let Some(delta) = parsed.handle_line(&line) {
                        if let Some(release) = gate.push(&delta) {
                            on_delta(&release);
                        }
                    }
                }
            }
        }
        let status = child.wait().await?;
        let _ = writer.await;
        let stderr_text = stderr_task.await.unwrap_or_default();
        if let Some(rest) = gate.finish() {
            on_delta(&rest);
        }
        if let Some(u) = parsed.usage.take() {
            *self.last_usage.lock().unwrap() = Some(u);
        }
        parsed.finish(status.success(), &stderr_text)
    }

    async fn complete(
        &self,
        messages: &[Message],
        tools: &[ToolDef],
        on_delta: &mut (dyn for<'s> FnMut(&'s str) + Send),
    ) -> anyhow::Result<Message> {
        let convo: Vec<&Message> = messages.iter().skip_while(|m| m.role == "system").collect();
        let fresh_system = render_system(messages, tools);

        // Can the existing CLI session carry this call? Only if it holds exactly a prefix of
        // this conversation (fingerprint), under the same system prompt. A call with no tools
        // (last round) reuses the session's prompt: the "out of tool budget" note arrives as a
        // message, and replies are parsed without tools.
        let mut why_not = String::from("no session yet");
        let reusable = self.sessions.as_ref().and_then(|(state, key)| {
            let info = state.map.lock().unwrap().get(key).cloned()?;
            let prefix_ok = info.consumed <= convo.len() && fingerprint(&convo[..info.consumed.min(convo.len())]) == info.fingerprint;
            let system_ok = tools.is_empty() || info.system == fresh_system;
            let has_new = info.consumed < convo.len();
            if !(prefix_ok && system_ok && has_new) {
                why_not = format!(
                    "prefix_ok={prefix_ok} system_ok={system_ok} has_new={has_new} (consumed={}, history={})",
                    info.consumed,
                    convo.len()
                );
                return None;
            }
            Some(info)
        });
        if self.sessions.is_some() {
            match &reusable {
                Some(i) => eprintln!("[loki-debug] claude inference session: resume {} (consumed {} of {})", &i.uuid[..8], i.consumed, convo.len()),
                None => eprintln!("[loki-debug] claude inference session: new ({why_not})"),
            }
        }

        let (system, prompt, flag_owned): (String, String, Option<(bool, String)>) = match (&reusable, &self.sessions) {
            // Resume: only what the session hasn't seen.
            (Some(info), _) => (info.system.clone(), render_messages(&convo[info.consumed..]), Some((false, info.uuid.clone()))),
            // New session: full transcript, with the cue for the first reply.
            (None, Some(_)) => (fresh_system.clone(), render_transcript(messages), Some((true, new_uuid()))),
            (None, None) => (fresh_system.clone(), render_transcript(messages), None),
        };
        let flag = match &flag_owned {
            None => SessionFlag::None,
            Some((true, id)) => SessionFlag::New(id),
            Some((false, id)) => SessionFlag::Resume(id),
        };

        let text = match self.run_once(&system, &prompt, flag, on_delta).await {
            Ok(t) => t,
            Err(e) if reusable.is_some() => {
                // The CLI may have lost the session (pruned, different machine). Forget it and
                // replay the whole conversation into a fresh one, once.
                eprintln!("[loki-debug] claude inference: resume failed ({e}); starting a fresh session");
                self.forget_session();
                return self.complete_fresh(messages, tools, on_delta).await;
            }
            Err(e) => return Err(e),
        };

        match parse_reply(&text, tools) {
            Ok(reply) => {
                self.remember_session(flag_owned.map(|(_, id)| id), &system, &convo, &reply);
                Ok(reply)
            }
            Err(problem) => {
                // One repair attempt in a throwaway stateless call, then drop the session: its
                // history now contains a malformed reply we don't want it to learn from.
                eprintln!("[loki-debug] claude inference: invalid tool call ({problem}); retrying once");
                self.forget_session();
                let mut repair = render_transcript(messages);
                repair.push_str(&format!(
                    "{}\n\n[System note]\nYour previous reply was invalid: {problem}\nReply again. To call tools use exactly `{TOOL_SENTINEL}` followed by a JSON array of {{\"name\":..., \"arguments\":{{...}}}} using only the listed tool names; otherwise answer in plain text.\n\n[Loki]\n",
                    text.trim()
                ));
                let text2 = self.run_once(&fresh_system, &repair, SessionFlag::None, &mut |_| {}).await?;
                parse_reply(&text2, tools).map_err(|p| anyhow::anyhow!("claude produced an invalid tool call twice: {p}"))
            }
        }
    }

    /// Replays everything into a brand-new session (used after a failed resume).
    async fn complete_fresh(
        &self,
        messages: &[Message],
        tools: &[ToolDef],
        on_delta: &mut (dyn for<'s> FnMut(&'s str) + Send),
    ) -> anyhow::Result<Message> {
        let convo: Vec<&Message> = messages.iter().skip_while(|m| m.role == "system").collect();
        let system = render_system(messages, tools);
        let id = new_uuid();
        let flag = if self.sessions.is_some() { SessionFlag::New(&id) } else { SessionFlag::None };
        let text = self.run_once(&system, &render_transcript(messages), flag, on_delta).await?;
        let reply = parse_reply(&text, tools).map_err(|p| anyhow::anyhow!("claude produced an invalid tool call: {p}"))?;
        self.remember_session(self.sessions.as_ref().map(|_| id), &system, &convo, &reply);
        Ok(reply)
    }

    fn forget_session(&self) {
        if let Some((state, key)) = &self.sessions {
            state.map.lock().unwrap().remove(key);
        }
    }

    /// Records that the session now holds `convo` plus Claude's `reply`.
    fn remember_session(&self, uuid: Option<String>, system: &str, convo: &[&Message], reply: &Message) {
        let (Some((state, key)), Some(uuid)) = (&self.sessions, uuid) else { return };
        let mut all: Vec<&Message> = convo.to_vec();
        all.push(reply);
        state.map.lock().unwrap().insert(
            key.clone(),
            SessionInfo { uuid, consumed: all.len(), fingerprint: fingerprint(&all), system: system.to_string() },
        );
    }
}

enum SessionFlag<'a> {
    None,
    New(&'a str),
    Resume(&'a str),
}

fn new_uuid() -> String {
    let mut b = [0u8; 16];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut b);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: Vec<String> = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{}-{}-{}-{}-{}", h[0..4].concat(), h[4..6].concat(), h[6..8].concat(), h[8..10].concat(), h[10..16].concat())
}

/// Order-sensitive hash of what each message says (role, text, tool calls, tool name). Equal
/// fingerprints mean "the same conversation prefix".
fn fingerprint(msgs: &[&Message]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    let mut eat = |bytes: &[u8]| {
        for b in bytes {
            h = (h ^ *b as u64).wrapping_mul(0x100000001b3);
        }
        h = (h ^ 0xff).wrapping_mul(0x100000001b3);
    };
    for m in msgs {
        eat(m.role.as_bytes());
        eat(m.content.as_bytes());
        if let Some(n) = &m.tool_name {
            eat(n.as_bytes());
        }
        for c in m.tool_calls.iter().flatten() {
            eat(c.function.name.as_bytes());
            eat(serde_json::to_string(&c.function.arguments).unwrap_or_default().as_bytes());
        }
    }
    h
}

#[async_trait]
impl LlmProvider for ClaudeCliProvider {
    async fn chat(&self, messages: &[Message], tools: &[ToolDef]) -> anyhow::Result<Message> {
        self.complete(messages, tools, &mut |_| {}).await
    }

    async fn chat_stream(
        &self,
        messages: &[Message],
        tools: &[ToolDef],
        on_delta: &mut (dyn for<'s> FnMut(&'s str) + Send),
    ) -> anyhow::Result<Message> {
        self.complete(messages, tools, on_delta).await
    }

    fn is_cloud(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        "claude-inference"
    }

    fn full_history_budget(&self) -> Option<usize> {
        Some(FULL_HISTORY_BUDGET_CHARS)
    }

    fn take_usage(&self) -> Option<Value> {
        self.last_usage.lock().unwrap().take()
    }
}

// ---------- prompt rendering (append-only; keep deterministic) ----------

/// System prompt = Loki's own system message(s) at the front, then the tool protocol.
pub fn render_system(messages: &[Message], tools: &[ToolDef]) -> String {
    let mut out = String::new();
    for m in messages.iter().take_while(|m| m.role == "system") {
        out.push_str(m.content.trim());
        out.push_str("\n\n");
    }
    if tools.is_empty() {
        out.push_str("Reply in plain text. No tools are available right now.\n");
        return out;
    }
    let mut sorted: Vec<&ToolDef> = tools.iter().collect();
    sorted.sort_by(|a, b| a.function.name.cmp(&b.function.name));
    out.push_str("# Tools\nYou can call these tools. Each takes a JSON object of arguments:\n");
    for t in sorted {
        out.push_str(&format!(
            "- {}: {}\n  arguments schema: {}\n",
            t.function.name,
            t.function.description.lines().next().unwrap_or(""),
            compact_schema(&t.function.parameters)
        ));
    }
    out.push_str(&format!(
        "\n# How to respond\nTo call tools, reply with ONLY this, starting at the very first character, and nothing else:\n{TOOL_SENTINEL}[{{\"name\":\"tool_name\",\"arguments\":{{...}}}}]\nYou may put several calls in the array; their results come back as [Tool result] messages. When you have what you need, reply with your final answer in plain text (no {TOOL_SENTINEL}). Use only the tool names listed above.\n"
    ));
    out
}

fn compact_schema(v: &Value) -> String {
    serde_json::to_string(v).unwrap_or_default()
}

/// The conversation after the leading system messages, in order. Later system messages
/// (summaries, budget notes) are rendered as notes.
pub fn render_transcript(messages: &[Message]) -> String {
    let convo: Vec<&Message> = messages.iter().skip_while(|m| m.role == "system").collect();
    let mut out = render_messages(&convo);
    out.push_str("[Loki]\n");
    out
}

/// Renders messages in order, without the trailing cue. Used for a whole transcript and, in a
/// resumed session, for just the messages the session hasn't seen.
pub fn render_messages(messages: &[&Message]) -> String {
    let mut out = String::new();
    for m in messages.iter().copied() {
        match m.role.as_str() {
            "user" => out.push_str(&format!("[User]\n{}\n\n", m.content.trim())),
            "assistant" => {
                if !m.content.trim().is_empty() {
                    out.push_str(&format!("[Loki]\n{}\n\n", m.content.trim()));
                }
                for call in m.tool_calls.iter().flatten() {
                    out.push_str(&format!(
                        "[Loki called a tool]\n{} {}\n\n",
                        call.function.name,
                        serde_json::to_string(&call.function.arguments).unwrap_or_default()
                    ));
                }
            }
            "tool" => {
                let mut body = m.content.clone();
                if body.chars().count() > MAX_TOOL_RESULT_CHARS {
                    body = body.chars().take(MAX_TOOL_RESULT_CHARS).collect::<String>() + "\n… [truncated]";
                }
                out.push_str(&format!("[Tool result: {}]\n{}\n\n", m.tool_name.as_deref().unwrap_or("tool"), body));
            }
            _ => out.push_str(&format!("[System note]\n{}\n\n", m.content.trim())),
        }
    }
    out
}

// ---------- reply parsing ----------

/// `Ok` with either plain text, or text plus validated tool calls. `Err(reason)` when the model
/// tried to call tools but got the form or a tool name wrong.
pub fn parse_reply(text: &str, tools: &[ToolDef]) -> Result<Message, String> {
    let text = text.trim();
    let Some(idx) = text.find(TOOL_SENTINEL) else {
        return Ok(Message::assistant(text.to_string()));
    };
    if tools.is_empty() {
        return Ok(Message::assistant(text[..idx].trim().to_string()));
    }
    let before = text[..idx].trim().to_string();
    // If the model re-issued the block ("Wait — re-issuing cleanly"), the last one wins.
    let last = text.rfind(TOOL_SENTINEL).unwrap_or(idx);
    let mut json_part = text[last + TOOL_SENTINEL.len()..].trim();
    // Tolerate a code fence around the JSON.
    json_part = json_part.trim_start_matches("```json").trim_start_matches("```").trim_end_matches("```").trim();
    let value: Value = serde_json::from_str(json_part).map_err(|e| format!("the JSON after {TOOL_SENTINEL} didn't parse ({e})"))?;
    let items = match value {
        Value::Array(a) => a,
        obj @ Value::Object(_) => vec![obj],
        _ => return Err("expected a JSON array of calls".to_string()),
    };
    if items.is_empty() {
        return Err("the call array was empty".to_string());
    }
    let mut calls = Vec::new();
    for item in items {
        let name = item["name"].as_str().ok_or("a call is missing \"name\"")?.to_string();
        if !tools.iter().any(|t| t.function.name == name) {
            return Err(format!("'{name}' is not an available tool"));
        }
        let arguments = match &item["arguments"] {
            Value::Null => serde_json::json!({}),
            v @ Value::Object(_) => v.clone(),
            _ => return Err(format!("arguments for '{name}' must be an object")),
        };
        calls.push(ToolCall { function: ToolCallFunction { name, arguments } });
    }
    let mut m = Message::assistant(before);
    m.tool_calls = Some(calls);
    Ok(m)
}

// ---------- streaming gate + CLI output ----------

/// Holds back the first few characters of a reply until we know whether it is a tool call
/// (starts with the sentinel — stream nothing) or prose (stream it all).
#[derive(Default)]
pub struct StreamGate {
    buf: String,
    state: GateState,
}

#[derive(Default, PartialEq)]
enum GateState {
    #[default]
    Undecided,
    Prose,
    ToolCall,
}

impl StreamGate {
    /// Returns text to emit now, if any.
    pub fn push(&mut self, delta: &str) -> Option<String> {
        if self.state == GateState::ToolCall {
            return None;
        }
        self.buf.push_str(delta);
        if self.state == GateState::Undecided {
            let t = self.buf.trim_start();
            if t.starts_with(TOOL_SENTINEL) {
                self.state = GateState::ToolCall;
                self.buf.clear();
                return None;
            } else if t.len() >= TOOL_SENTINEL.len() || !TOOL_SENTINEL.starts_with(t) {
                self.state = GateState::Prose;
            } else {
                return None;
            }
        }
        // Prose: emit up to the first sentinel (then swallow the rest), holding back a trailing
        // partial prefix of the sentinel until the next chunk resolves it.
        if let Some(i) = self.buf.find(TOOL_SENTINEL) {
            let out = self.buf[..i].to_string();
            self.state = GateState::ToolCall;
            self.buf.clear();
            return if out.is_empty() { None } else { Some(out) };
        }
        let mut keep = 0;
        for n in (1..TOOL_SENTINEL.len().min(self.buf.len() + 1)).rev() {
            if self.buf.ends_with(&TOOL_SENTINEL[..n]) {
                keep = n;
                break;
            }
        }
        let cut = self.buf.len() - keep;
        if cut == 0 {
            return None;
        }
        let out = self.buf[..cut].to_string();
        self.buf.drain(..cut);
        Some(out)
    }

    /// Anything still held back when the stream ends (a very short reply).
    pub fn finish(&mut self) -> Option<String> {
        if self.state != GateState::ToolCall && !self.buf.is_empty() {
            self.state = GateState::Prose;
            Some(std::mem::take(&mut self.buf))
        } else {
            None
        }
    }
}

/// One-line token/cost summary of a CLI `result` event, for comparing modes in the journal.
pub fn usage_summary(result: &Value) -> String {
    let u = &result["usage"];
    format!(
        "in={} cache_create={} cache_read={} out={} cost_usd={:.4}",
        u["input_tokens"].as_u64().unwrap_or(0),
        u["cache_creation_input_tokens"].as_u64().unwrap_or(0),
        u["cache_read_input_tokens"].as_u64().unwrap_or(0),
        u["output_tokens"].as_u64().unwrap_or(0),
        result["total_cost_usd"].as_f64().unwrap_or(0.0)
    )
}

#[derive(Default)]
struct CliOutput {
    final_text: Option<String>,
    error_text: Option<String>,
    streamed: String,
    usage: Option<Value>,
}

impl CliOutput {
    /// Returns a text delta if the line carried one.
    fn handle_line(&mut self, line: &str) -> Option<String> {
        let v: Value = serde_json::from_str(line).ok()?;
        match v["type"].as_str()? {
            "stream_event" => {
                let d = &v["event"]["delta"];
                if v["event"]["type"] == "content_block_delta" && d["type"] == "text_delta" {
                    let t = d["text"].as_str()?.to_string();
                    self.streamed.push_str(&t);
                    return Some(t);
                }
            }
            "rate_limit_event" => {
                let info = v["rate_limit_info"].clone();
                if info["status"] == "rejected" {
                    self.error_text = Some(format!("Claude usage limit reached (rate limit rejected): {info}"));
                }
                self.usage = Some(info);
            }
            "result" => {
                let text = v["result"].as_str().unwrap_or_default().to_string();
                eprintln!("[loki-debug] claude usage (inference): {}", usage_summary(&v));
                if v["is_error"].as_bool().unwrap_or(false) {
                    self.error_text = Some(if text.is_empty() { v.to_string() } else { text });
                } else {
                    self.final_text = Some(text);
                }
            }
            _ => {}
        }
        None
    }

    fn finish(self, exited_ok: bool, stderr: &str) -> anyhow::Result<String> {
        if let Some(e) = self.error_text {
            anyhow::bail!("claude CLI reported an error: {e}");
        }
        let text = self.final_text.unwrap_or(self.streamed);
        if !exited_ok && text.trim().is_empty() {
            anyhow::bail!("claude CLI failed: {}", stderr.trim());
        }
        if text.trim().is_empty() {
            anyhow::bail!("claude CLI returned no output");
        }
        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tool(name: &str) -> ToolDef {
        ToolDef::function(name, "does a thing\nsecond line", json!({"type":"object","properties":{"q":{"type":"string"}}}))
    }

    #[test]
    fn system_lists_tools_sorted_and_is_deterministic() {
        let msgs = vec![Message::system("You are Loki."), Message::user("hi")];
        let a = render_system(&msgs, &[tool("zeta"), tool("alpha")]);
        let b = render_system(&msgs, &[tool("alpha"), tool("zeta")]);
        assert_eq!(a, b, "tool order must not change the prefix");
        assert!(a.starts_with("You are Loki."));
        assert!(a.find("- alpha:").unwrap() < a.find("- zeta:").unwrap());
        assert!(!a.contains("second line"));
        assert!(render_system(&msgs, &[]).contains("No tools are available"));
    }

    #[test]
    fn transcript_is_append_only() {
        let mut msgs = vec![Message::system("sys"), Message::user("one"), Message::assistant("two")];
        let before = render_transcript(&msgs);
        msgs.push(Message::user("three"));
        let after = render_transcript(&msgs);
        // Everything before the trailing "[Loki]\n" cue is an unchanged prefix of the new render.
        let stem = before.trim_end_matches("[Loki]\n");
        assert!(after.starts_with(stem));
        assert!(after.ends_with("[User]\nthree\n\n[Loki]\n"));
    }

    #[test]
    fn transcript_renders_tool_traffic() {
        let mut call = Message::assistant("");
        call.tool_calls = Some(vec![ToolCall { function: ToolCallFunction { name: "search_tools".into(), arguments: json!({"query":"x"}) } }]);
        let t = render_transcript(&[Message::user("q"), call, Message::tool_result("search_tools", "found it")]);
        assert!(t.contains("[Loki called a tool]\nsearch_tools {\"query\":\"x\"}"));
        assert!(t.contains("[Tool result: search_tools]\nfound it"));
    }

    #[test]
    fn parse_plain_text_and_tool_calls() {
        let tools = [tool("search_tools")];
        let plain = parse_reply("All done.", &tools).unwrap();
        assert_eq!(plain.content, "All done.");
        assert!(plain.tool_calls.is_none());

        let call = parse_reply(r#"<<TOOL_CALLS>>[{"name":"search_tools","arguments":{"q":"cables"}}]"#, &tools).unwrap();
        assert_eq!(call.tool_calls.as_ref().unwrap()[0].function.arguments["q"], "cables");

        let fenced = parse_reply("Looking.\n<<TOOL_CALLS>>\n```json\n{\"name\":\"search_tools\"}\n```", &tools).unwrap();
        assert_eq!(fenced.content, "Looking.");
        assert_eq!(fenced.tool_calls.unwrap()[0].function.arguments, json!({}));
    }

    #[test]
    fn parse_rejects_bad_calls_with_a_reason() {
        let tools = [tool("search_tools")];
        assert!(parse_reply("<<TOOL_CALLS>>[{\"name\":\"rm_rf\"}]", &tools).unwrap_err().contains("not an available tool"));
        assert!(parse_reply("<<TOOL_CALLS>>not json", &tools).unwrap_err().contains("didn't parse"));
        assert!(parse_reply("<<TOOL_CALLS>>[]", &tools).unwrap_err().contains("empty"));
        assert!(parse_reply("<<TOOL_CALLS>>[{\"name\":\"search_tools\",\"arguments\":5}]", &tools).is_err());
        // With no tools offered the sentinel is just text.
        assert!(parse_reply("<<TOOL_CALLS>>[]", &[]).unwrap().tool_calls.is_none());
    }

    #[test]
    fn gate_streams_prose_but_swallows_tool_calls() {
        let mut g = StreamGate::default();
        let mut out = String::new();
        for d in ["Hel", "lo ", "there"] {
            if let Some(t) = g.push(d) {
                out.push_str(&t);
            }
        }
        assert_eq!(out, "Hello there");

        let mut g = StreamGate::default();
        for d in ["<<TOOL_", "CALLS>>[{\"name\"", ":\"x\"}]"] {
            assert!(g.push(d).is_none());
        }
        assert!(g.finish().is_none());

        // Prose that can't be the sentinel streams immediately, even when very short.
        let mut g = StreamGate::default();
        assert_eq!(g.push("ok").as_deref(), Some("ok"));
        // A reply that is only a prefix of the sentinel is held, then released at the end.
        let mut g = StreamGate::default();
        assert!(g.push("<<").is_none());
        assert_eq!(g.finish().as_deref(), Some("<<"));
        // Prose that merely starts like the sentinel is released once it diverges.
        let mut g = StreamGate::default();
        assert_eq!(g.push("<<not a call").as_deref(), Some("<<not a call"));
    }

    #[test]
    fn gate_stops_at_sentinel_after_prose() {
        let mut g = StreamGate::default();
        let mut out = String::new();
        for d in ["Let me check. <<TOOL", "_CALLS>>[{\"name\"", ":\"x\"}]", " more"] {
            if let Some(t) = g.push(d) {
                out.push_str(&t);
            }
        }
        assert_eq!(out, "Let me check. ");
        assert!(g.finish().is_none());

        // A partial sentinel prefix that turns out to be prose is released.
        let mut g = StreamGate::default();
        let mut out = String::new();
        for d in ["a <<TO", "day is fine"] {
            if let Some(t) = g.push(d) {
                out.push_str(&t);
            }
        }
        assert_eq!(out, "a <<TOday is fine");
    }

    #[test]
    fn parse_reply_uses_last_block() {
        let tools = [tool("a")];
        let m = parse_reply("Hi.\n<<TOOL_CALLS>>[{\"name\":\"a\"}]\nWait — redo:\n<<TOOL_CALLS>>[{\"name\":\"a\",\"arguments\":{\"k\":1}}]", &tools).unwrap();
        assert_eq!(m.content, "Hi.");
        assert_eq!(m.tool_calls.unwrap()[0].function.arguments["k"], 1);
    }

    #[test]
    fn fingerprint_is_order_and_content_sensitive() {
        let a = Message::user("one");
        let b = Message::assistant("two");
        assert_eq!(fingerprint(&[&a, &b]), fingerprint(&[&a, &b]));
        assert_ne!(fingerprint(&[&a, &b]), fingerprint(&[&b, &a]));
        assert_ne!(fingerprint(&[&a]), fingerprint(&[&Message::user("one!")]));
        let mut with_call = Message::assistant("two");
        with_call.tool_calls = Some(vec![ToolCall { function: ToolCallFunction { name: "x".into(), arguments: json!({}) } }]);
        assert_ne!(fingerprint(&[&b]), fingerprint(&[&with_call]));
    }

    #[test]
    fn delta_render_has_no_cue_and_matches_transcript_body() {
        let u = Message::user("q");
        let all = render_transcript(&[Message::system("s"), u.clone()]);
        let delta = render_messages(&[&u]);
        assert!(!delta.ends_with("[Loki]\n"));
        assert!(all.starts_with(&delta));
    }

    #[test]
    fn uuids_are_v4_shaped() {
        let id = new_uuid();
        assert_eq!(id.len(), 36);
        assert_eq!(id.as_bytes()[14], b'4');
        assert_ne!(id, new_uuid());
    }

    #[test]
    fn cli_output_collects_text_usage_and_errors() {
        let mut o = CliOutput::default();
        assert_eq!(
            o.handle_line(r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"hi"}}}"#),
            Some("hi".to_string())
        );
        o.handle_line(r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed","unifiedWindows":{}}}"#);
        o.handle_line(r#"{"type":"result","is_error":false,"result":"hi there"}"#);
        assert!(o.usage.is_some());
        assert_eq!(o.finish(true, "").unwrap(), "hi there");

        let mut e = CliOutput::default();
        e.handle_line(r#"{"type":"result","is_error":true,"result":"Failed to authenticate: OAuth session expired"}"#);
        assert!(crate::router::is_limit_error(&e.finish(false, "").unwrap_err().to_string()));
    }
}
