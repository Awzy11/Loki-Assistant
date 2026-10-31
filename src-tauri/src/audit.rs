//! Claude audits of the local model's answers. Claude reads one finished turn (question, the
//! tool calls and reasoning trace the event log recorded, and the answer) and returns a verdict
//! plus findings, a better answer when the original was weak, and candidate lessons (which wait
//! for the user's approval in `lessons`). Runs tool-less and never sees privacy-flagged turns.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub kind: String,
    pub detail: String,
    pub severity: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditRecord {
    pub session_id: String,
    /// Index of the audited assistant message in the conversation (`ChatTurn::index`).
    pub index: usize,
    pub at: u64,
    /// "good" | "minor" | "bad"
    pub verdict: String,
    pub summary: String,
    #[serde(default)]
    pub findings: Vec<Finding>,
    #[serde(default)]
    pub better_answer: Option<String>,
    #[serde(default)]
    pub suggested_lessons: Vec<String>,
    #[serde(default)]
    pub auto: bool,
}

fn dir() -> Option<PathBuf> {
    let d = dirs::data_dir()?.join("loki").join("audits");
    std::fs::create_dir_all(&d).ok()?;
    Some(d)
}

fn file(session_id: &str) -> Option<PathBuf> {
    let safe: String = session_id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).take(80).collect();
    Some(dir()?.join(format!("{safe}.jsonl")))
}

pub fn load(session_id: &str) -> Vec<AuditRecord> {
    let Some(p) = file(session_id) else { return Vec::new() };
    std::fs::read_to_string(p).unwrap_or_default().lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

/// Latest audit per message index.
pub fn latest_by_index(session_id: &str) -> std::collections::HashMap<usize, AuditRecord> {
    let mut m = std::collections::HashMap::new();
    for r in load(session_id) {
        m.insert(r.index, r);
    }
    m
}

pub fn save(rec: &AuditRecord) {
    use std::io::Write;
    let Some(p) = file(&rec.session_id) else { return };
    if let (Ok(mut f), Ok(line)) = (std::fs::OpenOptions::new().create(true).append(true).open(p), serde_json::to_string(rec)) {
        let _ = writeln!(f, "{line}");
    }
}

pub const CLAUDE_ARGS: [&str; 7] = ["--tools", "", "--setting-sources", "", "--no-session-persistence", "--model", "sonnet"];

pub fn build_prompt(question: &str, answer: &str, process: &str, engine_note: &str, lessons: &str) -> String {
    format!(
        "You are auditing an answer given by a small local language model that acts as the user's personal assistant \
(it manages the user's Obsidian vault and answers everyday questions, using tools). Judge it strictly but fairly.\n\n\
Check: factual/logical correctness, whether it did what was asked, tool misuse (wrong tool, bad arguments, \
claiming to have done something it did not), hallucination, missed steps, unnecessary verbosity, and whether \
its reasoning trace (if shown) went off track. Do not penalise style preferences.\n\n\
{engine_note}\n\
## Lessons already in force (do not suggest duplicates)\n{lessons}\n\n\
## User's message\n{question}\n\n\
## What the model did (tool calls, results, reasoning trace — may be empty)\n{process}\n\n\
## The model's final answer\n{answer}\n\n\
Reply with ONLY one JSON object, no prose, no code fence:\n\
{{\"verdict\":\"good|minor|bad\",\"summary\":\"one or two sentences\",\
\"findings\":[{{\"kind\":\"correctness|tool-use|hallucination|missed-step|verbosity|reasoning|other\",\"detail\":\"...\",\"severity\":\"low|medium|high\"}}],\
\"better_answer\":\"a corrected answer, or null if the original was fine\",\
\"suggested_lessons\":[\"short, general, imperative rule the local model should follow in future (max 2, only if a real pattern; else [])\"]}}"
    )
}

/// Extracts and parses the JSON object in Claude's reply.
pub fn parse(session_id: &str, index: usize, auto: bool, text: &str) -> Result<AuditRecord, String> {
    let (a, b) = (text.find('{'), text.rfind('}'));
    let (Some(a), Some(b)) = (a, b) else { return Err("Claude's audit contained no JSON".into()) };
    let v: Value = serde_json::from_str(&text[a..=b]).map_err(|e| format!("couldn't parse the audit: {e}"))?;
    let verdict = match v["verdict"].as_str() {
        Some(x @ ("good" | "minor" | "bad")) => x.to_string(),
        _ => "minor".to_string(),
    };
    let findings = v["findings"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|f| {
                    Some(Finding {
                        kind: f["kind"].as_str().unwrap_or("other").to_string(),
                        detail: f["detail"].as_str()?.to_string(),
                        severity: f["severity"].as_str().unwrap_or("low").to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(AuditRecord {
        session_id: session_id.to_string(),
        index,
        at: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
        verdict,
        summary: v["summary"].as_str().unwrap_or("").to_string(),
        findings,
        better_answer: v["better_answer"].as_str().filter(|s| !s.trim().is_empty()).map(String::from),
        suggested_lessons: v["suggested_lessons"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default(),
        auto,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_audit_json_inside_noise() {
        let t = "Here you go:\n{\"verdict\":\"bad\",\"summary\":\"Wrong path\",\"findings\":[{\"kind\":\"tool-use\",\"detail\":\"guessed path\",\"severity\":\"high\"}],\"better_answer\":null,\"suggested_lessons\":[\"Verify a path before writing.\"]}";
        let r = parse("s", 3, false, t).unwrap();
        assert_eq!(r.verdict, "bad");
        assert_eq!(r.findings.len(), 1);
        assert!(r.better_answer.is_none());
        assert_eq!(r.suggested_lessons, vec!["Verify a path before writing."]);
    }

    #[test]
    fn rejects_non_json() {
        assert!(parse("s", 0, false, "no json here").is_err());
    }
}
