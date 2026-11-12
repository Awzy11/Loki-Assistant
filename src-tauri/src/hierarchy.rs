//! The hierarchy: a paid cloud model decides and writes, the free local model reads. This module holds the two pieces that run without
//! the cloud model asking: condensing long read-only tool results, and the briefing packet a routine starts from.
//!
//! Safety rules the rest of Loki relies on: only tools the settings list are condensed (never file reads or note contents, which a
//! model may need to quote or edit exactly), the full result is kept server-side for `{{result:tool}}` pipes so a write can still use
//! it, and any failure falls back to the untouched result. Email and finance never get here: they are local-only and never reach a
//! cloud model in the first place.

use crate::agent::tools::local_provider;
use crate::config::HierarchyConfig;
use crate::providers::Message;
use serde_json::Value;
use std::sync::OnceLock;
use std::time::Duration;

static SETTINGS: OnceLock<HierarchyConfig> = OnceLock::new();

/// Called once at startup from `[hierarchy]`.
pub fn set(c: HierarchyConfig) {
    let _ = SETTINGS.set(c);
}

pub fn settings() -> HierarchyConfig {
    SETTINGS.get().cloned().unwrap_or_default()
}

fn matches(pattern: &str, name: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => name.starts_with(prefix),
        None => name == pattern,
    }
}

/// Whether a result from `name` of `len` characters should be condensed for a cloud model.
pub fn should_condense(c: &HierarchyConfig, name: &str, len: usize) -> bool {
    c.condense && len >= c.condense_min_chars && c.condense_tools.iter().any(|p| matches(p, name))
}

/// Most of a page the local model is shown; the rest is dropped (and said so).
const MAX_INPUT_CHARS: usize = 60_000;
/// The longest digest handed up.
const MAX_DIGEST_CHARS: usize = 4_000;
const CONDENSE_TIMEOUT: Duration = Duration::from_secs(90);

fn prompt(goal: &str, why: &str, tool: &str, args: &Value, raw: &str, truncated: bool) -> (String, String) {
    let system = "You condense tool output for another assistant so it can work from a short version. Keep every fact, number, date, name, \
        address and link that is relevant to what the assistant is trying to do, plus anything that would change its decision. Drop navigation, \
        advertising, boilerplate and repetition. Quote exact figures and short key phrases verbatim; never invent or guess. If nothing in the \
        output is relevant, say so in one line. Reply with the condensed text only, under about 500 words."
        .to_string();
    let user = format!(
        "What the assistant is doing: {goal}\nWhy it called this tool: {why}\nTool: {tool} {}\n\n--- OUTPUT{} ---\n{raw}",
        args.to_string().chars().take(300).collect::<String>(),
        if truncated { " (cut off for length)" } else { "" }
    );
    (system, user)
}

/// The local model's short version of `raw`, with a note saying so, or `None` if it was not worth it or anything failed.
pub async fn condense(tool: &str, args: &Value, goal: &str, why: &str, raw: &str) -> Option<String> {
    let local = local_provider()?;
    let truncated = raw.chars().count() > MAX_INPUT_CHARS;
    let shown: String = raw.chars().take(MAX_INPUT_CHARS).collect();
    let (system, user) = prompt(&goal.chars().take(800).collect::<String>(), &why.chars().take(400).collect::<String>(), tool, args, &shown, truncated);
    let reply = tokio::time::timeout(CONDENSE_TIMEOUT, local.chat(&[Message::system(system), Message::user(user)], &[])).await.ok()?.ok()?;
    let digest: String = reply.content.trim().chars().take(MAX_DIGEST_CHARS).collect();
    let n = raw.chars().count();
    if digest.is_empty() || digest.chars().count() * 10 >= n * 8 {
        return None;
    }
    crate::usage_ledger::record_offload("condense", n, digest.chars().count());
    Some(format!(
        "[Condensed by Loki's local model from {n} characters of {tool} output, keeping what bears on your task. If something you need is missing, call the tool again with a narrower request.]\n\n{digest}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> HierarchyConfig {
        HierarchyConfig::default()
    }

    #[test]
    fn only_listed_tools_over_the_threshold_are_condensed() {
        let c = cfg();
        assert!(should_condense(&c, "mcp__fetch__fetch", 20_000));
        assert!(should_condense(&c, "mcp__openweather__weather", 6_000));
        assert!(should_condense(&c, "simple_search", 9_000));
        assert!(!should_condense(&c, "mcp__fetch__fetch", 5_999), "short results pass through");
        assert!(!should_condense(&c, "read_file", 200_000), "file reads must stay exact");
        assert!(!should_condense(&c, "mcp__obsidian__obsidian_get_file_contents", 200_000));
        assert!(!should_condense(&c, "mail_read", 200_000));
    }

    #[test]
    fn can_be_switched_off_and_retargeted() {
        let mut c = cfg();
        c.condense = false;
        assert!(!should_condense(&c, "mcp__fetch__fetch", 20_000));
        c.condense = true;
        c.condense_tools = vec!["my_tool".into(), "other__*".into()];
        assert!(should_condense(&c, "my_tool", 7_000) && should_condense(&c, "other__x", 7_000));
        assert!(!should_condense(&c, "mcp__fetch__fetch", 20_000));
    }

    #[test]
    fn the_prompt_carries_the_goal_and_the_output() {
        let (sys, user) = prompt("find the forecast for Friday", "checking weather", "mcp__openweather__weather", &serde_json::json!({"city": "Springfield"}), "RAW TEXT", true);
        assert!(sys.contains("never invent"));
        assert!(user.contains("find the forecast for Friday") && user.contains("Springfield") && user.contains("RAW TEXT") && user.contains("cut off for length"));
    }
}
