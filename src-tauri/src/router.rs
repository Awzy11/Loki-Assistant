//! Backend routing helpers: availability failover and cross-backend context handoff.
//!
//! Loki's own transcript (`SessionState::history`) is the source of truth for a conversation.
//! Claude Code keeps a *second*, private copy inside its CLI session (`--resume`), which is
//! kept for the normal case because it preserves Claude's prompt cache. That copy goes stale
//! whenever a turn is answered by anyone else (a local/Gemini model after a usage limit, a
//! privacy-flagged turn, a restart that lost the session UUID). This module provides:
//!
//! - `is_limit_error` — recognises "this backend is out of quota / overloaded" failures that
//!   should fail over to the next backend rather than surface as an error.
//! - `build_catchup` — renders the turns Claude's session hasn't seen as a handoff block to
//!   prepend to the next message sent to it, so no manual context transfer is needed.
//! - `first_success` — tries an ordered provider chain until one succeeds.
//! - `Cooldown` — remembers that Claude hit its limit so we don't pay a doomed CLI spawn on
//!   every turn while it is out.

use crate::providers::{LlmProvider, Message};
use std::future::Future;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// True for failures meaning "this backend can't serve the turn right now" — usage/rate limits,
/// quota exhaustion, overload, upstream 5xx, being unreachable, or (Claude CLI) not being
/// logged in / an expired OAuth session, which should fail over with an explanation rather
/// than surface as a dead end. Deliberately a text match:
/// the Claude CLI reports its subscription limit as prose ("5-hour limit reached ∙ resets
/// 3pm"), and the HTTP providers surface status codes inside their error strings. Timeouts are
/// NOT included: a timed-out Claude turn may have been mid-way through vault writes, and
/// re-running it elsewhere could duplicate them.
pub fn is_limit_error(msg: &str) -> bool {
    let m = msg.to_lowercase();
    const NEEDLES: &[&str] = &[
        "usage limit",
        "limit reached",
        "rate limit",
        "rate_limit",
        "too many requests",
        "429",
        "quota",
        "resource_exhausted",
        "overloaded",
        "529",
        "503",
        "502",
        "service unavailable",
        "credit balance",
        "connection refused",
        "error sending request",
        "couldn't run",
        // Claude CLI login problems (fix: `claude /login` on the server).
        "failed to authenticate",
        "oauth session expired",
        "not logged in",
        "please run /login",
    ];
    NEEDLES.iter().any(|n| m.contains(n))
}

/// Claude CLI can exit 0 and print a limit notice as its "reply". Real answers are not this
/// short and do not mention a limit, so require both to avoid misfiring on a conversation
/// *about* rate limits.
pub fn reply_is_limit_notice(reply: &str) -> bool {
    reply.len() < 400 && is_limit_error(reply) && reply.to_lowercase().contains("limit")
}

const MAX_TURN_CHARS: usize = 2_000;
const MAX_CATCHUP_CHARS: usize = 20_000;

/// A transcript turn: a user/assistant message with text. Tool traffic, empty assistant
/// tool-call messages and system messages are not turns. This is the unit
/// `SessionState::claude_synced`, `summarized_upto` and `build_catchup` count in: it is stable
/// across a restart (history reloaded from the vault log has only these) and across backends
/// (`agent::run_turn` inserts system/tool messages that shift raw indices).
pub fn is_turn(m: &Message) -> bool {
    (m.role == "user" || m.role == "assistant") && !m.content.trim().is_empty()
}

pub fn convo_len(history: &[Message]) -> usize {
    history.iter().filter(|m| is_turn(m)).count()
}

/// Renders turns `[from, to)` (see `is_turn`) as a handoff block. If `summary` is
/// `Some((text, upto))` and `from < upto`, turns before `upto` are replaced by that summary and
/// only the rest are listed verbatim. Keeps the *most recent* turns when over budget and says
/// how many were omitted. Turns whose index is in `private` are replaced by a placeholder.
/// `None` when there is nothing to hand off.
pub fn build_catchup(
    history: &[Message],
    from: usize,
    to: usize,
    summary: Option<(&str, usize)>,
    private: &[usize],
) -> Option<String> {
    let convo: Vec<&Message> = history.iter().filter(|m| is_turn(m)).collect();
    let to = to.min(convo.len());
    let (summary_text, from) = match summary {
        Some((text, upto)) if !text.trim().is_empty() && from < upto => (Some(text.trim()), upto.min(to)),
        _ => (None, from),
    };
    if from >= to && summary_text.is_none() {
        return None;
    }
    let first = from.min(to);
    let lines: Vec<String> = convo[first..to]
        .iter()
        .enumerate()
        .map(|(i, m)| {
            // Turns that stayed local for privacy are never forwarded to Claude.
            if private.contains(&(first + i)) {
                return "(a private exchange that stayed on this machine is not shown)".to_string();
            }
            let who = if m.role == "user" { "User" } else { "Assistant" };
            let mut text = m.content.trim().to_string();
            if text.chars().count() > MAX_TURN_CHARS {
                text = text.chars().take(MAX_TURN_CHARS).collect::<String>() + " …[truncated]";
            }
            format!("{who}: {text}")
        })
        .collect();

    let mut kept: Vec<&String> = Vec::new();
    let mut used = 0usize;
    for line in lines.iter().rev() {
        if used + line.len() > MAX_CATCHUP_CHARS && !kept.is_empty() {
            break;
        }
        used += line.len();
        kept.push(line);
    }
    kept.reverse();
    let omitted = lines.len() - kept.len();

    let mut out = String::from(
        "[Loki context handoff — this is context from this same conversation that this Claude \
         session has not seen (another assistant answered, or the session was restarted). Treat it \
         as prior conversation. Do not summarize or comment on this block; just answer the message \
         after it.]\n",
    );
    if let Some(text) = summary_text {
        out.push_str(&format!("Summary of the earlier conversation:\n{text}\n\nRecent turns:\n"));
    }
    if omitted > 0 {
        out.push_str(&format!("({omitted} earlier turn(s) omitted for length)\n"));
    }
    for line in kept {
        out.push_str(line);
        out.push('\n');
    }
    out.push_str("[End of handoff]\n\n");
    Some(out)
}

/// What `first_success` learned: the value, which provider produced it, and a human-readable
/// note per provider that was skipped before it (for the UI's "answered by X because Y").
pub struct Failover<T> {
    pub value: T,
    pub provider: String,
    pub skipped: Vec<String>,
}

/// Tries `chain` in order, returning the first `Ok`. Any provider error moves on to the next
/// provider (providers are stateless per call, so retrying elsewhere is safe); if every
/// provider fails, the error lists each one.
pub async fn first_success<T, F, Fut>(chain: &[Arc<dyn LlmProvider>], mut f: F) -> anyhow::Result<Failover<T>>
where
    F: FnMut(Arc<dyn LlmProvider>) -> Fut,
    Fut: Future<Output = anyhow::Result<T>>,
{
    let mut skipped = Vec::new();
    for provider in chain {
        let mut result = f(provider.clone()).await;
        // llama-server sometimes rejects a malformed tool call ("peg-native" parse error) — it's
        // sampling noise, so one retry on the same provider usually succeeds.
        if matches!(&result, Err(e) if e.to_string().contains("peg-native")) {
            eprintln!("[loki-debug] provider '{}' returned a peg-native parse error, retrying once", provider.name());
            result = f(provider.clone()).await;
        }
        match result {
            Ok(value) => return Ok(Failover { value, provider: provider.name().to_string(), skipped }),
            Err(e) => {
                eprintln!("[loki-debug] provider '{}' failed, trying next: {e}", provider.name());
                let first_line = e.to_string().lines().next().unwrap_or("").chars().take(160).collect::<String>();
                skipped.push(format!("{}: {first_line}", provider.name()));
            }
        }
    }
    anyhow::bail!("every configured model failed — {}", skipped.join(" | "))
}

/// Process-wide "Claude is out until" marker. The limit is per-account, not per-session, so
/// one instance lives on `Core`. Cheap to check; expires on its own so Claude is retried
/// automatically.
#[derive(Default)]
pub struct Cooldown {
    until: Mutex<Option<Instant>>,
}

impl Cooldown {
    pub fn trip(&self, for_how_long: Duration) {
        *self.until.lock().unwrap() = Some(Instant::now() + for_how_long);
    }

    /// Time left, or `None` if not cooling down (also clears an expired marker).
    pub fn remaining(&self) -> Option<Duration> {
        let mut guard = self.until.lock().unwrap();
        match *guard {
            Some(t) if t > Instant::now() => Some(t - Instant::now()),
            Some(_) => {
                *guard = None;
                None
            }
            None => None,
        }
    }

    pub fn clear(&self) {
        *self.until.lock().unwrap() = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::ToolDef;
    use async_trait::async_trait;

    #[test]
    fn limit_detection() {
        assert!(is_limit_error("Claude AI usage limit reached|1760000000"));
        assert!(is_limit_error("5-hour limit reached ∙ resets 3pm"));
        assert!(is_limit_error("gemini returned 429 Too Many Requests"));
        assert!(is_limit_error("RESOURCE_EXHAUSTED: quota exceeded"));
        assert!(is_limit_error("anthropic: overloaded_error"));
        assert!(!is_limit_error("claude CLI timed out after 120s"));
        assert!(is_limit_error("claude CLI reported an error: Failed to authenticate: OAuth session expired and could not be refreshed"));
        assert!(is_limit_error("Not logged in · Please run /login"));
        assert!(!is_limit_error("invalid api key"));
    }

    #[test]
    fn limit_notice_only_when_short() {
        assert!(reply_is_limit_notice("5-hour limit reached ∙ resets 3pm"));
        let long = format!("Here is how rate limit headers work. {}", "x".repeat(500));
        assert!(!reply_is_limit_notice(&long));
        assert!(!reply_is_limit_notice("Sure, done."));
    }

    #[test]
    fn catchup_skips_tool_traffic_and_new_message() {
        let h = vec![
            Message::system("sys"),
            Message::user("q1"),
            Message::assistant("a1"),
            Message::tool_result("simple_search", "junk"),
            Message::user("q2"),
        ];
        // Turns: q1 a1 q2 (tool traffic and system aren't turns). Claude saw q1 (cursor 1);
        // the new message q2 is excluded via `to`.
        assert_eq!(convo_len(&h), 3);
        let c = build_catchup(&h, 1, 2, None, &[]).unwrap();
        assert!(c.contains("Assistant: a1"));
        assert!(!c.contains("junk"));
        assert!(!c.contains("q1"));
        assert!(!c.contains("q2"));
        assert!(build_catchup(&h, 2, 2, None, &[]).is_none());

        // An empty assistant tool-call message must not count as a turn (restart-stability).
        let mut with_empty = h.clone();
        with_empty.insert(3, Message::assistant(""));
        assert_eq!(convo_len(&with_empty), 3);
    }

    #[test]
    fn catchup_keeps_recent_turns_when_over_budget() {
        let mut h = vec![Message::system("sys")];
        for i in 0..40 {
            h.push(Message::user(format!("turn-{i} {}", "y".repeat(1500))));
        }
        let c = build_catchup(&h, 0, h.len(), None, &[]).unwrap();
        assert!(c.contains("turn-39"));
        assert!(!c.contains("turn-0 "));
        assert!(c.contains("earlier turn(s) omitted"));
        assert!(c.len() < MAX_CATCHUP_CHARS + 2_000);
    }

    #[test]
    fn catchup_uses_summary_for_summarized_turns() {
        let mut h = vec![Message::system("sys")];
        for i in 0..6 {
            h.push(if i % 2 == 0 { Message::user(format!("u{i}")) } else { Message::assistant(format!("a{i}")) });
        }
        // Summary covers turns 0..4; Claude has seen none of them.
        let c = build_catchup(&h, 0, 6, Some(("they planned a trip", 4)), &[]).unwrap();
        assert!(c.contains("they planned a trip"));
        assert!(!c.contains("u0") && !c.contains("a3"));
        assert!(c.contains("u4") && c.contains("a5"));
        // Claude already saw past the summarized range -> summary not repeated.
        let c = build_catchup(&h, 4, 6, Some(("they planned a trip", 4)), &[]).unwrap();
        assert!(!c.contains("they planned a trip"));
        assert!(c.contains("u4"));
    }

    #[test]
    fn catchup_never_forwards_private_turns() {
        let h = vec![
            Message::user("my ssn is 123-45-6789"),
            Message::assistant("noted, staying local"),
            Message::user("what's the weather"),
            Message::assistant("sunny"),
            Message::user("new question"),
        ];
        let c = build_catchup(&h, 0, 4, None, &[0, 1]).unwrap();
        assert!(!c.contains("123-45-6789") && !c.contains("staying local"));
        assert!(c.contains("private exchange") && c.contains("sunny"));
    }

    struct Mock {
        name: &'static str,
        fail: bool,
    }

    #[async_trait]
    impl LlmProvider for Mock {
        async fn chat(&self, _m: &[Message], _t: &[ToolDef]) -> anyhow::Result<Message> {
            if self.fail {
                anyhow::bail!("{} 429 rate limit\nsecond line", self.name)
            }
            Ok(Message::assistant(format!("from {}", self.name)))
        }
        fn is_cloud(&self) -> bool {
            false
        }
        fn name(&self) -> &str {
            self.name
        }
    }

    fn chain(spec: &[(&'static str, bool)]) -> Vec<Arc<dyn LlmProvider>> {
        spec.iter().map(|(n, f)| Arc::new(Mock { name: n, fail: *f }) as Arc<dyn LlmProvider>).collect()
    }

    #[tokio::test]
    async fn failover_skips_failed_providers_in_order() {
        let c = chain(&[("a", true), ("b", false), ("c", false)]);
        let r = first_success(&c, |p| async move { p.chat(&[], &[]).await.map(|m| m.content) }).await.unwrap();
        assert_eq!(r.value, "from b");
        assert_eq!(r.provider, "b");
        assert_eq!(r.skipped.len(), 1);
        assert!(r.skipped[0].starts_with("a: a 429"));
        assert!(!r.skipped[0].contains("second line"));
    }

    #[tokio::test]
    async fn failover_errors_when_all_fail() {
        let c = chain(&[("a", true), ("b", true)]);
        let e = first_success(&c, |p| async move { p.chat(&[], &[]).await.map(|m| m.content) }).await.err().unwrap();
        let s = e.to_string();
        assert!(s.contains("a:") && s.contains("b:"));
    }

    #[test]
    fn cooldown_expires() {
        let c = Cooldown::default();
        assert!(c.remaining().is_none());
        c.trip(Duration::from_millis(30));
        assert!(c.remaining().is_some());
        std::thread::sleep(Duration::from_millis(50));
        assert!(c.remaining().is_none());
    }
}
