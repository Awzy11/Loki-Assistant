//! Durable per-session state that the vault conversation log doesn't carry: which Claude CLI
//! session this conversation is attached to, how far that session has seen, the rolling
//! summary, and a title. One small JSON file per session under `~/.local/share/loki/sessions/`.
//! The vault log stays the human-readable transcript; this is operational state, and losing it
//! only costs a re-handoff, never data.

use crate::providers::{LlmProvider, Message};
use crate::router::is_turn;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Persisted {
    #[serde(default)]
    pub claude_session: Option<String>,
    /// Transcript turns (see `router::is_turn`) Claude's CLI session has seen.
    #[serde(default)]
    pub claude_synced: usize,
    #[serde(default)]
    pub force_local: bool,
    /// Rolling summary of turns `[0, summarized_upto)`.
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub summarized_upto: usize,
    #[serde(default)]
    pub title: Option<String>,
    /// The title was typed by the user: the AI never changes it again.
    #[serde(default)]
    pub title_manual: bool,
    /// Transcript size (messages) when the AI last named this conversation; 0 = never.
    #[serde(default)]
    pub title_ai_at: usize,
    /// Unix seconds of the last completed turn.
    #[serde(default)]
    pub updated: u64,
    #[serde(default)]
    pub turns: usize,
    /// Transcript turn indices that stayed local for privacy — never handed to Claude or
    /// folded into the summary.
    #[serde(default)]
    pub private_turns: Vec<usize>,
    /// This conversation's engine choice (`engines`), if it has one.
    #[serde(default)]
    pub engine: Option<String>,
    /// Thumbs up/down per assistant message index (`"up"` / `"down"`).
    #[serde(default)]
    pub feedback: std::collections::HashMap<usize, String>,
}

fn dir() -> anyhow::Result<PathBuf> {
    let d = dirs::data_dir().ok_or_else(|| anyhow::anyhow!("no data dir"))?.join("loki").join("sessions");
    std::fs::create_dir_all(&d)?;
    Ok(d)
}

/// Session ids come from clients; keep file names safe and collision-free.
fn file_name(session_id: &str) -> String {
    let safe: String = session_id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    if safe == session_id && !safe.is_empty() && safe.len() <= 120 {
        format!("{safe}.json")
    } else {
        // Hash-suffixed so two different odd ids can't collide after sanitizing.
        let h = session_id.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
        format!("{}-{h:016x}.json", safe.chars().take(60).collect::<String>())
    }
}

pub fn load(session_id: &str) -> Option<Persisted> {
    let raw = std::fs::read_to_string(dir().ok()?.join(file_name(session_id))).ok()?;
    serde_json::from_str(&raw).ok()
}

pub fn save(session_id: &str, p: &Persisted) {
    let result = (|| -> anyhow::Result<()> {
        let path = dir()?.join(file_name(session_id));
        let tmp = path.with_extension("json.tmp");
        // Carry the session id inside the file so `list` can report it despite name sanitizing.
        let mut v = serde_json::to_value(p)?;
        v["session_id"] = session_id.into();
        std::fs::write(&tmp, serde_json::to_vec_pretty(&v)?)?;
        std::fs::rename(tmp, path)?;
        Ok(())
    })();
    if let Err(e) = result {
        eprintln!("[loki-debug] couldn't persist session state for '{session_id}' (non-fatal): {e}");
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionSummary {
    pub session_id: String,
    pub title: String,
    pub updated: u64,
    pub turns: usize,
}

/// Sessions with persisted state, newest first.
pub fn list() -> Vec<SessionSummary> {
    let Ok(d) = dir() else { return Vec::new() };
    let Ok(entries) = std::fs::read_dir(d) else { return Vec::new() };
    let mut out: Vec<SessionSummary> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("json"))
        .filter_map(|e| {
            let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(e.path()).ok()?).ok()?;
            let id = v["session_id"].as_str()?.to_string();
            let p: Persisted = serde_json::from_value(v).ok()?;
            Some(SessionSummary {
                title: p.title.unwrap_or_else(|| "Untitled conversation".to_string()),
                session_id: id,
                updated: p.updated,
                turns: p.turns,
            })
        })
        .collect();
    out.sort_by(|a, b| b.updated.cmp(&a.updated));
    out
}

/// A model's reply cleaned into a short title: first line, no quotes/markdown/"Title:", no final
/// full stop, at most 8 words and 60 characters. `None` if nothing usable is left.
pub fn clean_title(raw: &str) -> Option<String> {
    let line = raw.trim().lines().map(str::trim).find(|l| !l.is_empty())?;
    let line = line.trim_start_matches('#').trim();
    let lower = line.to_lowercase();
    let line = if lower.starts_with("title:") { line[6..].trim() } else { line };
    let line = line.trim_matches(|c: char| matches!(c, '"' | '\'' | '`' | '*' | '“' | '”' | '‘' | '’')).trim_end_matches(['.', '!', ':']).trim();
    let words: Vec<&str> = line.split_whitespace().collect();
    if words.is_empty() || words.len() > 8 {
        return None;
    }
    let t: String = words.join(" ").chars().take(60).collect();
    Some(t)
}

/// Asks a local model for a 3–6 word title for a conversation (never a cloud model: the text is private).
pub async fn generate_title(provider: &Arc<dyn LlmProvider>, first_user: &str, first_assistant: &str, last_user: Option<&str>) -> Option<String> {
    let cap = |s: &str, n: usize| s.chars().take(n).collect::<String>();
    let mut ask = format!("First message: {}\n\nFirst reply: {}", cap(first_user, 600), cap(first_assistant, 400));
    if let Some(l) = last_user.filter(|l| *l != first_user) {
        ask.push_str(&format!("\n\nMost recent message: {}", cap(l, 300)));
    }
    let msgs = vec![
        Message::system(
            "You name conversations. Reply with ONLY a title of 3 to 6 words that says what the conversation is about, in plain \
             sentence case. No quotes, no trailing punctuation, no prefix like \"Title:\". If a command like /bod or /eod started it, \
             name the subject, not the command.",
        ),
        Message::user(ask),
    ];
    let reply = tokio::time::timeout(std::time::Duration::from_secs(40), provider.chat_low_effort(&msgs)).await.ok()?.ok()?;
    clean_title(&reply.content)
}

pub fn title_from(message: &str) -> String {
    let line = message.trim().lines().next().unwrap_or("").trim();
    let t: String = line.chars().take(60).collect();
    if line.chars().count() > 60 { format!("{t}…") } else { t }
}

pub fn now_unix() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Summarize once this many turns sit beyond the last summary, keeping the newest
/// `SUMMARY_KEEP_RECENT` verbatim.
pub const SUMMARY_TRIGGER: usize = 24;
pub const SUMMARY_KEEP_RECENT: usize = 10;

const SUMMARY_PROMPT: &str = "You maintain a running summary of a conversation between the user and their assistant. \
Merge the existing summary (if any) with the new turns into one updated summary. Keep decisions, facts about \
the user's situation, open questions, tasks in progress, and names/paths/numbers that matter. Drop pleasantries and \
tool noise. Write plain prose or short bullets, under 350 words. Output only the summary.";

/// Produces the merged summary of `turns` into `existing`. `provider` should be a local model:
/// the summary of a conversation can contain anything that was said in it.
pub async fn summarize(provider: &Arc<dyn LlmProvider>, existing: &str, turns: &[(usize, Message)], private: &[usize]) -> anyhow::Result<String> {
    let mut body = String::new();
    if !existing.trim().is_empty() {
        body.push_str(&format!("Existing summary:\n{}\n\n", existing.trim()));
    }
    body.push_str("New turns:\n");
    for (_, m) in turns.iter().filter(|(i, m)| is_turn(m) && !private.contains(i)) {
        let who = if m.role == "user" { "User" } else { "Assistant" };
        let text: String = m.content.trim().chars().take(1500).collect();
        body.push_str(&format!("{who}: {text}\n"));
    }
    let reply = provider.chat_low_effort(&[Message::system(SUMMARY_PROMPT), Message::user(body)]).await?;
    let text = reply.content.trim().to_string();
    if text.is_empty() {
        anyhow::bail!("summarizer returned nothing");
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_are_safe_and_collision_resistant() {
        assert_eq!(file_name("web-123-abc"), "web-123-abc.json");
        let a = file_name("a/b");
        let b = file_name("a_b");
        assert_ne!(a, b);
        assert!(!a.contains('/'));
        assert!(!file_name("../../etc/passwd").contains('/'));
    }

    #[test]
    fn model_titles_are_cleaned() {
        assert_eq!(clean_title("\"Loki hook deploy plan.\"\n"), Some("Loki hook deploy plan".into()));
        assert_eq!(clean_title("Title: ATLAS one-page ask"), Some("ATLAS one-page ask".into()));
        assert_eq!(clean_title("## **Weekly sprint review**"), Some("Weekly sprint review".into()));
        assert_eq!(clean_title("  \n"), None);
        assert_eq!(clean_title("This is a very long answer that is really a sentence not a title at all"), None);
    }

    #[test]
    fn title_truncates_on_char_boundary() {
        assert_eq!(title_from("  hello\nworld"), "hello");
        let long = "é".repeat(80);
        let t = title_from(&long);
        assert_eq!(t.chars().count(), 61);
        assert!(t.ends_with('…'));
    }

    struct Echo(std::sync::Mutex<String>);
    #[async_trait::async_trait]
    impl LlmProvider for Echo {
        async fn chat(&self, messages: &[Message], _t: &[crate::providers::ToolDef]) -> anyhow::Result<Message> {
            *self.0.lock().unwrap() = messages.iter().map(|m| m.content.clone()).collect::<Vec<_>>().join("\n");
            Ok(Message::assistant("merged summary"))
        }
        fn is_cloud(&self) -> bool {
            false
        }
        fn name(&self) -> &str {
            "echo"
        }
    }

    #[tokio::test]
    async fn summarize_merges_existing_and_skips_private_turns() {
        let echo = Arc::new(Echo(Default::default()));
        let provider: Arc<dyn LlmProvider> = echo.clone();
        let turns = vec![
            (0, Message::user("plan the trip")),
            (1, Message::assistant("my ssn is 123-45-6789")),
            (2, Message::user("book the hotel")),
        ];
        let out = summarize(&provider, "earlier: chose Lisbon", &turns, &[1]).await.unwrap();
        assert_eq!(out, "merged summary");
        let seen = echo.0.lock().unwrap().clone();
        assert!(seen.contains("earlier: chose Lisbon") && seen.contains("plan the trip") && seen.contains("book the hotel"));
        assert!(!seen.contains("123-45-6789"), "private turn must not be summarized");
    }

    #[test]
    fn persisted_round_trips_and_tolerates_missing_fields() {
        let p: Persisted = serde_json::from_str(r#"{"claude_session":"x"}"#).unwrap();
        assert_eq!(p.claude_session.as_deref(), Some("x"));
        assert_eq!(p.claude_synced, 0);
        let again: Persisted = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(p, again);
    }
}
