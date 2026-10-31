use crate::config::ClaudeConfig;
use crate::tool_policy::TIER01_ALLOWED_TOOLS;
use crate::events::{emit, ChatEvent, EventSink};
use serde_json::Value;
use std::collections::HashSet;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;
use tokio::time::timeout;


/// MCP server name `loki-bridge-mcp` is registered under for a turn.
const BRIDGE_SERVER: &str = "loki-bridge";

/// Claude Code's built-in file/shell tools. Headless on `loki-server`'s host they would touch
/// the server's disk; with the bridge on, Claude uses the `device_*` tools instead.
const DISALLOWED_BUILTINS: &[&str] = &["Bash", "Read", "Write", "Edit", "MultiEdit", "Glob", "Grep", "NotebookEdit"];

const BRIDGE_SYSTEM_PROMPT: &str = "You are running inside Loki, the user's personal agent app. You do not have Claude \
Code's built-in file or shell tools. To act on the user's computer use the device_* tools: device_read_file, \
device_list_dir, device_write_file, device_edit_file, device_run_command. Reads run immediately; writes, edits and \
commands each ask the user to approve in the Loki UI, so say briefly what you are about to do. The vault is reached \
through the Obsidian tools as usual.";

const BRIDGE_SYSTEM_PROMPT_SERVER_TOOLS: &str = "You are running inside Loki, the user's personal agent app, on the Loki server (a Raspberry \
Pi). Your built-in Bash, Read, Write, Edit, Glob and Grep act on THAT server, and every call asks the user to approve in \
the Loki UI, so say briefly what you are about to do and keep calls few. To act on the user's own computer use the \
device_* tools instead: device_read_file, device_list_dir, device_write_file, device_edit_file, device_run_command \
(reads run immediately; writes, edits and commands ask the user). The vault is reached through the Obsidian tools as usual.";

/// A temp file readable only by this user, deleted on drop. Used for per-turn MCP configs and
/// system prompts so secrets and long prompts stay out of argv (`ps`, argument-length limits).
pub(crate) struct PrivateTemp {
    pub(crate) path: std::path::PathBuf,
}

impl PrivateTemp {
    pub(crate) fn write(prefix: &str, ext: &str, bytes: &[u8]) -> anyhow::Result<Self> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let dir = std::env::var_os("XDG_RUNTIME_DIR").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir);
        let nonce: u64 = rand::random();
        let path = dir.join(format!("{prefix}-{}-{nonce:x}.{ext}", std::process::id()));
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path)?;
        f.write_all(bytes)?;
        Ok(Self { path })
    }
}

impl Drop for PrivateTemp {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Identifies the Loki session a Claude turn belongs to, for the bridge helper.
pub struct BridgeSession<'a> {
    pub session_id: &'a str,
    pub token: &'a str,
}

/// Bridges Loki to the user's own Claude Code subscription via the CLI, non-interactively.
/// Authenticates with whatever login session already exists on this machine (the user's own
/// Claude Pro/Max account, via `claude /login`) — no API key involved, billed under the
/// subscription exactly like any other Claude Code session. This is the only practical way
/// to reach Claude from Loki's Rust core: the Claude Agent SDK that actually carries
/// subscription billing only ships as a Python/TypeScript library, so shelling out to the
/// CLI is the bridge rather than a second language runtime embedded in the app.
///
/// Two distinct call shapes:
/// - `ask()` — one-shot, no vault access, no session. "Answer this one hard question."
/// - `run_routine_turn()` — Routine Mode: a resumable, multi-turn session with real vault/MCP
///   tool access (scoped to `TIER01_ALLOWED_TOOLS`), running an actual `~/.claude/commands/`
///   file end to end. See `core.rs`'s `send_message_in_session` for the relay logic that
///   decides which one a given message uses.
pub struct ClaudeClient {
    config: ClaudeConfig,
    /// MCP servers handed to every delegated turn — Loki's own server set (see `McpHub::claude_servers`).
    mcp_servers: serde_json::Map<String, Value>,
    /// When set, vault writes made inside a delegated turn are auto-logged to the daily note.
    activity: Option<std::sync::Arc<crate::obsidian::ObsidianClient>>,
}

impl ClaudeClient {
    pub fn new(config: ClaudeConfig) -> Self {
        Self { config, mcp_servers: Default::default(), activity: None }
    }

    pub fn with_mcp_servers(mut self, servers: serde_json::Map<String, Value>) -> Self {
        self.mcp_servers = servers;
        self
    }

    /// Auto-log vault writes made by delegated turns (see `activity_log`).
    pub fn with_activity_log(mut self, obsidian: std::sync::Arc<crate::obsidian::ObsidianClient>) -> Self {
        self.activity = Some(obsidian);
        self
    }

    /// Spawns best-effort log appends for vault writes that finished since the last call.
    fn log_finished_writes(&self, state: &mut StreamState) {
        let done = std::mem::take(&mut state.done_writes);
        let Some(obsidian) = &self.activity else { return };
        for (tool, path, ok, args) in done {
            let o = obsidian.clone();
            tokio::spawn(async move { crate::activity_log::record(&o, &tool, &path, ok, &args).await });
        }
    }

    /// Spawns `claude` with `args`, optionally in `cwd`, and returns trimmed stdout. Shared
    /// by `ask()` and `run_routine_turn()` — the only things that differ between them are the
    /// arguments and whether a working directory is set, not the process/timeout/error
    /// handling around the call.
    async fn spawn_and_capture(&self, args: &[String], cwd: Option<&str>) -> anyhow::Result<String> {
        let mut cmd = Command::new(&self.config.binary_path);
        cmd.args(args);
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }

        let output = timeout(Duration::from_secs(self.config.timeout_secs), cmd.output())
            .await
            .map_err(|_| {
                anyhow::anyhow!("claude CLI timed out after {}s", self.config.timeout_secs)
            })?
            .map_err(|e| {
                anyhow::anyhow!(
                    "couldn't run '{}': {e}. Is Claude Code installed and on PATH \
                     (or is claude.binary_path in config.toml pointing at the right binary)?",
                    self.config.binary_path
                )
            })?;

        if !output.status.success() {
            // The CLI reports subscription-limit notices on stdout, not stderr — include both so
            // `router::is_limit_error` can recognise them and fail over.
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            anyhow::bail!("claude CLI exited with {}: {} {}", output.status, stderr.trim(), stdout.trim());
        }

        let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if text.is_empty() {
            anyhow::bail!("claude CLI returned no output");
        }
        Ok(text)
    }

    /// Runs one headless Claude Code turn on `prompt` and returns its final text answer.
    /// The subprocess has no vault access of its own — the caller must pass along whatever
    /// context Loki's local model has already gathered, so Claude reasons over it directly
    /// rather than re-discovering it. Keeps this a pure "smarter reasoning for one question"
    /// bridge instead of a second, harder-to-audit vault-editing surface.
    pub async fn ask(&self, prompt: &str) -> anyhow::Result<String> {
        if prompt.trim().is_empty() {
            anyhow::bail!("escalate_to_claude called with an empty prompt");
        }
        let args = vec!["-p".to_string(), prompt.to_string(), "--output-format".to_string(), "text".to_string()];
        self.spawn_and_capture(&args, None).await
    }

    /// Like `ask`, with extra CLI flags and a working directory — for the review jobs (audits
    /// run tool-less; troubleshooting gets read-only file access to its evidence bundle).
    pub async fn ask_with(&self, prompt: &str, extra_args: &[String], cwd: Option<&str>) -> anyhow::Result<String> {
        let mut args = vec!["-p".to_string(), prompt.to_string(), "--output-format".to_string(), "text".to_string()];
        args.extend_from_slice(extra_args);
        self.spawn_and_capture(&args, cwd).await
    }

    /// The set of routine names Loki will recognize as a Routine Mode trigger — the file
    /// stems under `~/.claude/commands/` (e.g. `bod`, `eod`, `bow`). Deliberately reads the
    /// real directory rather than hardcoding a list, so a new command the user adds there works
    /// immediately with no Loki code change.
    pub fn list_available_commands(&self) -> anyhow::Result<HashSet<String>> {
        let dir = dirs::home_dir()
            .ok_or_else(|| anyhow::anyhow!("couldn't determine home directory"))?
            .join(".claude")
            .join("commands");
        let mut names = HashSet::new();
        for entry in std::fs::read_dir(&dir)
            .map_err(|e| anyhow::anyhow!("couldn't read {}: {e}", dir.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("md") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    names.insert(stem.to_lowercase());
                }
            }
        }
        Ok(names)
    }

    /// Runs one turn of a Routine Mode session — a real, resumable Claude Code conversation
    /// running an actual `~/.claude/commands/<name>.md` file, with scoped vault/MCP tool
    /// access (`TIER01_ALLOWED_TOOLS`). `is_new_session` picks `--session-id` (start) vs.
    /// `--resume` (continue) for `claude_session_id`; every reply on the same Loki
    /// conversation while Routine Mode is active should reuse the same id and pass `false`.
    ///
    /// Requires `config.routine_working_dir` — errors clearly if unset rather than running
    /// with an unpredictable cwd, since CLAUDE.md/command-file/MCP-config resolution depends
    /// on it (see the field's doc comment in `config.rs`).
    pub async fn run_routine_turn(
        &self,
        claude_session_id: &str,
        message: &str,
        is_new_session: bool,
        sink: Option<&EventSink>,
        bridge: Option<&BridgeSession<'_>>,
    ) -> anyhow::Result<String> {
        let working_dir = self.config.routine_working_dir.as_deref().ok_or_else(|| {
            anyhow::anyhow!(
                "Routine Mode requires claude.routine_working_dir to be set in config.toml — \
                 point it at the vault-aware CLAUDE.md project directory (e.g. the directory \
                 containing ~/.claude/commands' matching project CLAUDE.md), so command files \
                 and MCP config resolve the same way they do in an interactive session."
            )
        })?;

        // stream-json + --verbose + --include-partial-messages: one JSON object per line —
        // text deltas, tool_use / tool_result blocks, rate-limit info, and a final `result`.
        let mut args = vec![
            "-p".to_string(),
            message.to_string(),
            "--output-format".to_string(),
            "stream-json".to_string(),
            "--verbose".to_string(),
            "--include-partial-messages".to_string(),
        ];
        if is_new_session {
            args.push("--session-id".to_string());
        } else {
            args.push("--resume".to_string());
        }
        args.push(claude_session_id.to_string());
        args.push("--permission-mode".to_string());
        args.push("acceptEdits".to_string());
        let mut allowed: Vec<String> = TIER01_ALLOWED_TOOLS.iter().map(|t| t.to_string()).collect();
        allowed.extend(self.config.extra_allowed_tools.iter().cloned());
        // Loki owns this process's permissions: load NO settings files. Any of user, project
        // or local settings can carry allow rules (the vault working directory's
        // `.claude/settings.local.json` auto-allows delete and several Bash patterns) that
        // would silently bypass the approval gate — verified: `project,local` still honors
        // them, an empty list does not. MCP servers (`~/.claude.json`), CLAUDE.md and
        // `~/.claude/commands` still load.
        args.push("--setting-sources".to_string());
        args.push(String::new());

        // MCP servers: with no settings sources loaded the CLI no longer picks up the user-scope
        // servers in `~/.claude.json`, so Loki passes them explicitly — the server list is part
        // of what Loki owns. The bridge helper (approvals + device tools) is added when
        // configured and this turn has a client to ask (a streaming turn). Written to a 0600
        // temp file rather than argv so tokens in server env never show up in `ps`.
        let mut mcp_servers = self.mcp_servers.clone();
        let bridge_on = matches!((self.config.bridge_binary.as_deref(), bridge), (Some(_), Some(_)));
        if let (Some(binary), Some(b)) = (self.config.bridge_binary.as_deref(), bridge) {
            mcp_servers.insert(
                BRIDGE_SERVER.to_string(),
                serde_json::json!({
                    "command": binary,
                    "env": { "LOKI_URL": self.config.bridge_url, "LOKI_TOKEN": b.token, "LOKI_SESSION_ID": b.session_id }
                }),
            );
        }
        let _mcp_file = if mcp_servers.is_empty() {
            None
        } else {
            let file = PrivateTemp::write("loki-mcp", "json", serde_json::json!({ "mcpServers": mcp_servers }).to_string().as_bytes())?;
            args.push("--mcp-config".to_string());
            args.push(file.path.display().to_string());
            Some(file)
        };
        if bridge_on {
            args.push("--permission-prompt-tool".to_string());
            args.push(format!("mcp__{BRIDGE_SERVER}__approve"));
            // `device_*` handle their own approval prompts server-side, so they can be
            // pre-allowed here without bypassing the gate. The CLI's own file/shell tools would
            // act on the server's disk, not the user's computer, so they are removed outright.
            allowed.extend(crate::bridge::DEVICE_TOOLS.iter().map(|t| format!("mcp__{BRIDGE_SERVER}__{t}")));
            let server_tools = self.config.server_tools_with_approval;
            if !server_tools {
                args.push("--disallowedTools".to_string());
                args.push(DISALLOWED_BUILTINS.join(" "));
            }
            args.push("--append-system-prompt".to_string());
            args.push(if server_tools { BRIDGE_SYSTEM_PROMPT_SERVER_TOOLS } else { BRIDGE_SYSTEM_PROMPT }.to_string());
        }
        args.push("--allowedTools".to_string());
        args.push(allowed.join(" "));

        let mut cmd = Command::new(&self.config.binary_path);
        cmd.args(&args)
            .current_dir(working_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // Dropping this future (client disconnected / Stop pressed) must not leave a
            // Claude process running tool calls in the background.
            .kill_on_drop(true);
        let mut child = cmd.spawn().map_err(|e| {
            anyhow::anyhow!(
                "couldn't run '{}': {e}. Is Claude Code installed and on PATH \
                 (or is claude.binary_path in config.toml pointing at the right binary)?",
                self.config.binary_path
            )
        })?;
        let stdout = child.stdout.take().expect("piped");
        let mut stderr = child.stderr.take().expect("piped");
        let stderr_task = tokio::spawn(async move {
            let mut buf = String::new();
            let _ = stderr.read_to_string(&mut buf).await;
            buf
        });

        // `timeout_secs` is an *idle* timeout here: a long tool-using turn that keeps
        // producing events is fine, a silent hang is not.
        let idle = Duration::from_secs(self.config.timeout_secs);
        let mut lines = BufReader::new(stdout).lines();
        let mut state = StreamState::default();
        loop {
            match timeout(idle, lines.next_line()).await {
                Err(_) => anyhow::bail!("claude CLI timed out after {}s of silence", self.config.timeout_secs),
                Ok(Err(e)) => anyhow::bail!("reading claude output failed: {e}"),
                Ok(Ok(None)) => break,
                Ok(Ok(Some(line))) => {
                    state.handle_line(&line, sink);
                    self.log_finished_writes(&mut state);
                }
            }
        }
        let status = child.wait().await?;
        let stderr_text = stderr_task.await.unwrap_or_default();
        state.finish(status.success(), &stderr_text)
    }
}

/// Accumulates what a `stream-json` run tells us while forwarding live events.
#[derive(Default)]
struct StreamState {
    /// `result.result` of the final `result` event.
    final_text: Option<String>,
    /// Set when the run ended in an error `result`, or a rate-limit rejection was reported.
    error_text: Option<String>,
    /// Text seen via deltas, used only if the CLI never sent a `result`.
    streamed_text: String,
    /// Vault writes seen starting, by tool-use id: (short tool, path, arguments).
    pending_writes: std::collections::HashMap<String, (String, String, Value)>,
    /// Vault writes that have finished: (short tool, path, succeeded, arguments).
    done_writes: Vec<(String, String, bool, Value)>,
}

impl StreamState {
    fn handle_line(&mut self, line: &str, sink: Option<&EventSink>) {
        let Ok(v) = serde_json::from_str::<Value>(line) else { return };
        match v.get("type").and_then(Value::as_str) {
            Some("stream_event") => {
                let delta = &v["event"]["delta"];
                if v["event"]["type"] == "content_block_delta" && delta["type"] == "text_delta" {
                    if let Some(t) = delta["text"].as_str() {
                        self.streamed_text.push_str(t);
                        emit(sink, ChatEvent::TextDelta { text: t.to_string() });
                    }
                }
            }
            Some("assistant") => {
                for block in v["message"]["content"].as_array().into_iter().flatten() {
                    if block["type"] == "tool_use" {
                        if let Some((tool, path)) = crate::activity_log::vault_write(block["name"].as_str().unwrap_or_default(), &block["input"]) {
                            self.pending_writes.insert(block["id"].as_str().unwrap_or_default().to_string(), (tool, path, block["input"].clone()));
                        }
                        emit(
                            sink,
                            ChatEvent::ToolStart {
                                id: block["id"].as_str().unwrap_or_default().to_string(),
                                name: block["name"].as_str().unwrap_or_default().to_string(),
                                input: block["input"].clone(),
                            },
                        );
                    }
                }
            }
            Some("user") => {
                for block in v["message"]["content"].as_array().into_iter().flatten() {
                    if block["type"] == "tool_result" {
                        if let Some((tool, path, args)) = self.pending_writes.remove(block["tool_use_id"].as_str().unwrap_or_default()) {
                            let text = tool_result_text(&block["content"]);
                            let ok = !(block["is_error"].as_bool().unwrap_or(false) || text.starts_with("error:"));
                            self.done_writes.push((tool, path, ok, args));
                        }
                        emit(
                            sink,
                            ChatEvent::ToolResult {
                                id: block["tool_use_id"].as_str().unwrap_or_default().to_string(),
                                content: tool_result_text(&block["content"]),
                                is_error: block["is_error"].as_bool().unwrap_or(false) || tool_result_text(&block["content"]).starts_with("error:"),
                            },
                        );
                    }
                }
            }
            Some("rate_limit_event") => {
                let info = v["rate_limit_info"].clone();
                if info["status"] == "rejected" {
                    self.error_text = Some(format!("Claude usage limit reached (rate limit rejected): {info}"));
                }
                emit(sink, ChatEvent::Usage { info });
            }
            Some("result") => {
                eprintln!("[loki-debug] claude usage (delegated): {}", crate::providers::claude_cli::usage_summary(&v));
                let text = v["result"].as_str().unwrap_or_default().to_string();
                if v["is_error"].as_bool().unwrap_or(false) {
                    self.error_text = Some(if text.is_empty() { v.to_string() } else { text });
                } else {
                    self.final_text = Some(text);
                }
            }
            _ => {}
        }
    }

    fn finish(self, exited_ok: bool, stderr: &str) -> anyhow::Result<String> {
        if let Some(e) = self.error_text {
            anyhow::bail!("claude CLI reported an error: {e}");
        }
        let text = self.final_text.unwrap_or(self.streamed_text);
        if !exited_ok && text.trim().is_empty() {
            anyhow::bail!("claude CLI failed: {}", stderr.trim());
        }
        let text = text.trim().to_string();
        if text.is_empty() {
            anyhow::bail!("claude CLI returned no output");
        }
        Ok(text)
    }
}

/// A tool_result's `content` is either a plain string or an array of `{type:"text", text}`.
fn tool_result_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(items) => items.iter().filter_map(|i| i["text"].as_str()).collect::<Vec<_>>().join("\n"),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(lines: &[&str]) -> (anyhow::Result<String>, Vec<ChatEvent>) {
        let bus = std::sync::Arc::new(crate::bus::Bus::default());
        let (mut rx, _guard) = bus.subscribe("t", crate::bus::ClientInfo::default());
        let sink = EventSink { bus: bus.clone(), session_id: "t".into(), client_id: "test".into() };
        let mut st = StreamState::default();
        for l in lines {
            st.handle_line(l, Some(&sink));
        }
        let res = st.finish(true, "");
        let mut evs = Vec::new();
        while let Ok(e) = rx.try_recv() {
            evs.push(e);
        }
        (res, evs)
    }

    #[test]
    fn parses_tool_use_text_and_result() {
        let (res, evs) = run(&[
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"/a"}}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"1\thello"}]}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"hel"}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"lo"}}}"#,
            r#"{"type":"result","is_error":false,"result":"hello"}"#,
        ]);
        assert_eq!(res.unwrap(), "hello");
        assert!(matches!(&evs[0], ChatEvent::ToolStart { name, .. } if name == "Read"));
        assert!(matches!(&evs[1], ChatEvent::ToolResult { content, .. } if content == "1\thello"));
        assert!(matches!(&evs[3], ChatEvent::TextDelta { text } if text == "lo"));
    }

    #[test]
    fn error_result_becomes_limit_detectable_error() {
        let (res, _) = run(&[r#"{"type":"result","is_error":true,"result":"Claude AI usage limit reached|1760000000"}"#]);
        let msg = res.unwrap_err().to_string();
        assert!(crate::router::is_limit_error(&msg));
    }

    #[test]
    fn rejected_rate_limit_is_an_error() {
        let (res, _) = run(&[r#"{"type":"rate_limit_event","rate_limit_info":{"status":"rejected","rateLimitType":"five_hour"}}"#]);
        assert!(crate::router::is_limit_error(&res.unwrap_err().to_string()));
    }

    #[test]
    fn tool_result_array_content() {
        let v: Value = serde_json::from_str(r#"[{"type":"text","text":"a"},{"type":"text","text":"b"}]"#).unwrap();
        assert_eq!(tool_result_text(&v), "a\nb");
    }
}

#[cfg(test)]
mod activity_tests {
    use super::*;

    #[test]
    fn pairs_vault_writes_with_their_results() {
        let mut st = StreamState::default();
        st.handle_line(r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"mcp__obsidian__obsidian_put_content","input":{"filepath":"a/b.md","content":"x"}},{"type":"tool_use","id":"t2","name":"Read","input":{"file_path":"/x"}}]}}"#, None);
        st.handle_line(r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"ok"},{"type":"tool_result","tool_use_id":"t2","content":"hi"}]}}"#, None);
        assert_eq!(st.done_writes.len(), 1);
        assert_eq!(st.done_writes[0].0, "put_content");
        assert_eq!(st.done_writes[0].1, "a/b.md");
        assert!(st.done_writes[0].2);
        assert!(st.pending_writes.is_empty());
    }

    #[test]
    fn failed_write_is_marked_failed() {
        let mut st = StreamState::default();
        st.handle_line(r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"mcp__obsidian__obsidian_delete_file","input":{"filepath":"a.md"}}]}}"#, None);
        st.handle_line(r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","is_error":true,"content":"nope"}]}}"#, None);
        assert!(!st.done_writes[0].2);
    }
}
