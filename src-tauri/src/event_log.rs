//! Durable per-conversation record of what a turn *did* (tool calls, results, status lines,
//! reasoning traces, approvals, errors) — the "processes" that the chat transcript alone doesn't
//! show. Feeds audits and "Troubleshoot with Claude". Text deltas and usage are not recorded.

use crate::events::ChatEvent;
use serde_json::Value;
use std::io::Write;
use std::path::PathBuf;

const MAX_FILE_BYTES: u64 = 6 * 1024 * 1024;
const MAX_FIELD_CHARS: usize = 8000;

fn dir() -> Option<PathBuf> {
    let d = crate::paths::data_dir()?.join("loki").join("events");
    std::fs::create_dir_all(&d).ok()?;
    Some(d)
}

fn file(session_id: &str) -> Option<PathBuf> {
    let safe: String = session_id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).take(80).collect();
    Some(dir()?.join(format!("{safe}.jsonl")))
}

fn clip(v: &mut Value) {
    match v {
        Value::String(s) if s.chars().count() > MAX_FIELD_CHARS => {
            let cut: String = s.chars().take(MAX_FIELD_CHARS).collect();
            *s = format!("{cut}… [truncated]");
        }
        Value::Object(m) => m.values_mut().for_each(clip),
        Value::Array(a) => a.iter_mut().for_each(clip),
        _ => {}
    }
}

pub fn record(session_id: &str, ev: &ChatEvent) {
    if matches!(ev, ChatEvent::TextDelta { .. } | ChatEvent::Usage { .. } | ChatEvent::Resolved { .. } | ChatEvent::Started { .. }) {
        return;
    }
    let Some(path) = file(session_id) else { return };
    if std::fs::metadata(&path).map(|m| m.len() > MAX_FILE_BYTES).unwrap_or(false) {
        return;
    }
    let Ok(mut v) = serde_json::to_value(ev) else { return };
    clip(&mut v);
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let line = serde_json::json!({ "t": t, "ev": v });
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{line}");
    }
}

pub fn read(session_id: &str) -> Vec<Value> {
    let Some(path) = file(session_id) else { return Vec::new() };
    std::fs::read_to_string(path).unwrap_or_default().lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

/// Events of the `ordinal`-th user turn (1-based): everything from its `user_message` up to the
/// next one. Empty for turns that predate the log.
pub fn turn_events(session_id: &str, ordinal: usize) -> Vec<Value> {
    let mut n = 0;
    read(session_id)
        .into_iter()
        .filter(|e| {
            if e["ev"]["type"] == "user_message" {
                n += 1;
            }
            n == ordinal
        })
        .collect()
}

/// Compact one-line-per-event rendering for prompts and reports.
pub fn render(events: &[Value]) -> String {
    events
        .iter()
        .map(|e| {
            let t = e["t"].as_u64().unwrap_or(0);
            let ev = &e["ev"];
            let kind = ev["type"].as_str().unwrap_or("?");
            let mut body = ev.clone();
            if let Some(o) = body.as_object_mut() {
                o.remove("type");
            }
            format!("[{t}] {kind}: {body}")
        })
        .collect::<Vec<_>>()
        .join("\n")
}
