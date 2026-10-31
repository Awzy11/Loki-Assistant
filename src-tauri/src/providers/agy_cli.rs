//! Agy (Google Antigravity's CLI) as a plus-up inference engine, on the user's own Agy login.
//!
//! Loki's loop (tools, MCP, approvals, skills, remote commands) stays in Loki; Agy only thinks.
//! Agy is a full coding agent with its own system prompt and tools and offers no "no tools" or
//! "custom system prompt" switch, so the adapter is deliberately conservative:
//!
//! * **Safety.** Headless mode auto-denies any tool that needs permission (verified: a
//!   `write_file` attempt came back in `denied_actions` and nothing was written), and it runs
//!   from an empty scratch directory, so Agy cannot change anything outside Loki's gate.
//! * **Prompt.** Loki's instructions, tool protocol and the transcript go in one `-p=` argument
//!   (attached form, so the text can never be parsed as an option), prefixed with an instruction
//!   not to use Agy's own tools. Tool calls use the same `<<TOOL_CALLS>>` protocol as Claude
//!   (`providers::claude_cli`), including one repair retry.
//! * **Stateless.** No session reuse: each call re-sends the (windowed) conversation. Agy adds a
//!   fixed ~12k-token prompt of its own, so this is a plus-up, not a default.

use super::claude_cli::{parse_reply, render_system, render_transcript};
use super::types::{Message, ToolDef};
use super::LlmProvider;
use async_trait::async_trait;
use serde_json::Value;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

/// Linux caps a single argument at 128 KiB; stay well under it.
const MAX_PROMPT_BYTES: usize = 110_000;

pub struct AgyCliProvider {
    binary: String,
    model: Option<String>,
    cwd: PathBuf,
    timeout: Duration,
}

/// What we need from Agy's `--output-format json` result.
#[derive(Debug, Default, PartialEq)]
pub struct AgyOutput {
    pub response: String,
    pub status: String,
    pub denied: Vec<String>,
    pub usage: String,
}

/// Finds and reads the JSON result in Agy's stdout (it may print notices before it).
pub fn parse_agy_output(stdout: &str) -> Result<AgyOutput, String> {
    let start = stdout.find('{').ok_or_else(|| format!("no JSON in agy output: {}", stdout.chars().take(200).collect::<String>()))?;
    let v: Value = serde_json::from_str(stdout[start..].trim()).map_err(|e| format!("couldn't read agy's JSON output ({e})"))?;
    let u = &v["usage"];
    Ok(AgyOutput {
        response: v["response"].as_str().unwrap_or_default().to_string(),
        status: v["status"].as_str().unwrap_or_default().to_string(),
        denied: v["denied_actions"].as_array().map(|a| a.iter().filter_map(|d| d["action"].as_str().map(String::from)).collect()).unwrap_or_default(),
        usage: format!(
            "in={} out={} thinking={}",
            u["input_tokens"].as_u64().unwrap_or(0),
            u["output_tokens"].as_u64().unwrap_or(0),
            u["thinking_tokens"].as_u64().unwrap_or(0)
        ),
    })
}

const PREFIX: &str = "You are being used purely as the language model behind another application (Loki). \
You have NO tools, files or workspace here: do not try to use any of your own tools, and never write files or run \
commands yourself. Follow the instructions below exactly and reply only with what they ask for.\n\n";

/// The single prompt: prefix, Loki's system text + tool protocol, then the transcript. If it
/// would exceed the argument limit, the OLDEST transcript lines are dropped first.
pub fn build_prompt(messages: &[Message], tools: &[ToolDef]) -> String {
    let system = render_system(messages, tools);
    let head = format!("{PREFIX}=== Instructions ===\n{system}\n=== Conversation ===\n");
    let mut transcript = render_transcript(messages);
    let budget = MAX_PROMPT_BYTES.saturating_sub(head.len());
    if transcript.len() > budget {
        let cut = transcript.len() - budget;
        // Cut at a line boundary on a char boundary, and say so.
        let mut at = cut;
        while at < transcript.len() && !transcript.is_char_boundary(at) {
            at += 1;
        }
        let rest = &transcript[at..];
        let rest = rest.split_once('\n').map(|(_, r)| r).unwrap_or(rest);
        transcript = format!("[earlier conversation omitted for length]\n{rest}");
    }
    format!("{head}{transcript}")
}

impl AgyCliProvider {
    pub fn new(binary: String, model: Option<String>, timeout_secs: u64) -> Self {
        let cwd = dirs::data_dir().unwrap_or_else(std::env::temp_dir).join("loki").join("agy-scratch-cwd");
        let _ = std::fs::create_dir_all(&cwd);
        Self { binary, model, cwd, timeout: Duration::from_secs(timeout_secs.max(30)) }
    }

    async fn run_once(&self, prompt: &str) -> anyhow::Result<AgyOutput> {
        if prompt.len() > MAX_PROMPT_BYTES + 4_000 {
            anyhow::bail!("prompt too large for agy ({} bytes)", prompt.len());
        }
        let mut cmd = Command::new(&self.binary);
        cmd.arg(format!("-p={prompt}")).args(["--output-format", "json"]);
        if let Some(m) = &self.model {
            cmd.args(["--model", m]);
        }
        cmd.current_dir(&self.cwd).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
        let out = tokio::time::timeout(self.timeout, cmd.output())
            .await
            .map_err(|_| anyhow::anyhow!("agy timed out after {}s", self.timeout.as_secs()))?
            .map_err(|e| anyhow::anyhow!("couldn't run '{}': {e}", self.binary))?;
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        match parse_agy_output(&stdout) {
            Ok(parsed) => {
                eprintln!("[loki-debug] agy usage: {} status={} denied={:?}", parsed.usage, parsed.status, parsed.denied);
                Ok(parsed)
            }
            Err(e) => anyhow::bail!("agy failed: {e} {}", stderr.chars().take(300).collect::<String>()),
        }
    }

    async fn complete(&self, messages: &[Message], tools: &[ToolDef]) -> anyhow::Result<Message> {
        let mut prompt = build_prompt(messages, tools);
        let mut out = self.run_once(&prompt).await?;
        // Agy tried one of its own tools (denied in headless mode) and produced nothing: say so, once.
        if out.response.trim().is_empty() && !out.denied.is_empty() {
            eprintln!("[loki-debug] agy: empty reply after denied tools {:?}; retrying with a reminder", out.denied);
            prompt.push_str("\n\n[Reminder] You have no tools of your own here. Reply with plain text (or the tool-call format above) only.\n[Loki]\n");
            out = self.run_once(&prompt).await?;
        }
        if out.response.trim().is_empty() {
            anyhow::bail!("agy returned an empty reply (status {})", out.status);
        }
        match parse_reply(&out.response, tools) {
            Ok(m) => Ok(m),
            Err(problem) => {
                eprintln!("[loki-debug] agy: invalid tool call ({problem}); retrying once");
                prompt.push_str(&format!(
                    "\n{}\n\n[System note]\nYour previous reply was invalid: {problem}\nReply again, using only the listed tool names with the exact format, or plain text.\n[Loki]\n",
                    out.response.trim()
                ));
                let again = self.run_once(&prompt).await?;
                parse_reply(&again.response, tools).map_err(|p| anyhow::anyhow!("agy produced an invalid tool call twice: {p}"))
            }
        }
    }
}

#[async_trait]
impl LlmProvider for AgyCliProvider {
    async fn chat(&self, messages: &[Message], tools: &[ToolDef]) -> anyhow::Result<Message> {
        self.complete(messages, tools).await
    }

    fn is_cloud(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        "agy"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::types::{ToolCall, ToolCallFunction};
    use serde_json::json;

    #[test]
    fn parses_result_with_notices_before_the_json() {
        let out = "jetski: no output produced — a tool required write_file\n{\"conversation_id\":\"x\",\"status\":\"SUCCESS\",\"response\":\"\",\"usage\":{\"input_tokens\":11928,\"output_tokens\":331,\"thinking_tokens\":237},\"denied_actions\":[{\"action\":\"write_file\"}]}";
        let p = parse_agy_output(out).unwrap();
        assert_eq!(p.response, "");
        assert_eq!(p.denied, vec!["write_file"]);
        assert_eq!(p.usage, "in=11928 out=331 thinking=237");
        let ok = parse_agy_output("{\"status\":\"SUCCESS\",\"response\":\"pong\\n\",\"usage\":{}}").unwrap();
        assert_eq!(ok.response, "pong\n");
        assert!(parse_agy_output("not json at all").is_err());
    }

    #[test]
    fn prompt_carries_the_no_own_tools_instruction_the_protocol_and_the_transcript() {
        let msgs = vec![Message::system("You are Loki."), Message::user("hello there")];
        let tools = [ToolDef::function("search_tools", "find tools", json!({"type":"object"}))];
        let p = build_prompt(&msgs, &tools);
        assert!(p.starts_with("You are being used purely as the language model"));
        assert!(p.contains("You are Loki.") && p.contains("<<TOOL_CALLS>>") && p.contains("search_tools"));
        assert!(p.ends_with("[User]\nhello there\n\n[Loki]\n"));
    }

    #[test]
    fn oversized_conversations_drop_the_oldest_lines_first() {
        let mut msgs = vec![Message::system("sys")];
        for i in 0..200 {
            msgs.push(if i % 2 == 0 { Message::user(format!("question {i} {}", "x".repeat(1_500))) } else { Message::assistant(format!("answer {i} {}", "y".repeat(1_500))) });
        }
        let p = build_prompt(&msgs, &[]);
        assert!(p.len() <= MAX_PROMPT_BYTES + 64, "{}", p.len());
        assert!(p.contains("[earlier conversation omitted for length]"));
        assert!(p.contains("answer 199"), "newest turn kept");
        assert!(!p.contains("question 0 "), "oldest turn dropped");
    }

    #[test]
    fn tool_calls_from_agy_use_the_shared_parser() {
        let tools = [ToolDef::function("search_tools", "d", json!({}))];
        let m = parse_reply("<<TOOL_CALLS>>[{\"name\":\"search_tools\",\"arguments\":{\"query\":\"x\"}}]", &tools).unwrap();
        let _: &ToolCall = &m.tool_calls.as_ref().unwrap()[0];
        let _: &ToolCallFunction = &m.tool_calls.as_ref().unwrap()[0].function;
        assert_eq!(m.tool_calls.unwrap()[0].function.name, "search_tools");
    }
}
