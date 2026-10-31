//! Self-improvement loop, step 1: read the durable event logs, count what keeps going wrong
//! (failed tool calls, step-limit hits), and let the local model draft a few *pending* lessons
//! from that. Nothing is applied: lessons wait on the Lessons tab for approval.
//! Mail and finance tools are never part of the evidence.

use serde_json::Value;
use std::collections::HashMap;

/// Tools whose inputs/results are Tier 3 and must not feed any summary.
fn private_tool(name: &str) -> bool {
    name.starts_with("mail_") || name.contains("finance") || name.contains("firefly") || name.contains("ledger")
}

#[derive(Debug, Default)]
pub struct Digest {
    pub sessions: usize,
    pub calls: usize,
    /// (tool, error head, count), most frequent first.
    pub failures: Vec<(String, String, usize)>,
    pub step_limit_hits: usize,
    /// Failures per tool over the whole window (not truncated).
    pub by_tool: HashMap<String, usize>,
}

/// First line of an error, with numbers/ids squashed so near-identical errors group together.
fn error_head(s: &str) -> String {
    let line = s.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
    let mut out = String::new();
    let mut last_digit = false;
    for c in line.chars().take(140) {
        if c.is_ascii_digit() {
            if !last_digit {
                out.push('#');
            }
            last_digit = true;
        } else {
            out.push(c);
            last_digit = false;
        }
    }
    out
}

/// Fold one session's events into the running tallies.
pub fn tally(events: &[Value], since: u64, d: &mut Digest, counts: &mut HashMap<(String, String), usize>) {
    let mut names: HashMap<String, String> = HashMap::new();
    for e in events {
        if e["t"].as_u64().unwrap_or(0) < since {
            continue;
        }
        let ev = &e["ev"];
        match ev["type"].as_str().unwrap_or("") {
            "tool_start" => {
                d.calls += 1;
                if let (Some(id), Some(name)) = (ev["id"].as_str(), ev["name"].as_str()) {
                    names.insert(id.to_string(), name.to_string());
                }
            }
            "tool_result" if ev["is_error"].as_bool() == Some(true) => {
                let name = ev["id"].as_str().and_then(|i| names.get(i)).cloned().unwrap_or_else(|| "?".into());
                if !private_tool(&name) {
                    *counts.entry((name, error_head(ev["content"].as_str().unwrap_or("")))).or_default() += 1;
                }
            }
            "status" if ev["message"].as_str().is_some_and(|m| m.contains("step limit")) => d.step_limit_hits += 1,
            _ => {}
        }
    }
}

pub fn digest(days: u64) -> Digest {
    let since = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0).saturating_sub(days * 86_400);
    let mut d = Digest::default();
    let mut counts = HashMap::new();
    let dir = dirs::data_dir().map(|p| p.join("loki").join("events"));
    for entry in dir.and_then(|p| std::fs::read_dir(p).ok()).into_iter().flatten().flatten() {
        let Some(id) = entry.path().file_stem().map(|s| s.to_string_lossy().to_string()) else { continue };
        let events = crate::event_log::read(&id);
        if events.iter().any(|e| e["t"].as_u64().unwrap_or(0) >= since) {
            d.sessions += 1;
            tally(&events, since, &mut d, &mut counts);
        }
    }
    for ((t, _), n) in &counts {
        *d.by_tool.entry(t.clone()).or_default() += n;
    }
    let mut f: Vec<(String, String, usize)> = counts.into_iter().map(|((t, e), n)| (t, e, n)).collect();
    f.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(&b.0)));
    f.truncate(12);
    d.failures = f;
    d
}

pub fn render(d: &Digest, days: u64) -> String {
    let mut t = format!("Last {days} days: {} sessions, {} tool calls, {} step-limit hits.\n", d.sessions, d.calls, d.step_limit_hits);
    if d.failures.is_empty() {
        t.push_str("No failed tool calls.\n");
    }
    for (tool, err, n) in &d.failures {
        t.push_str(&format!("- {n}× {tool}: {err}\n"));
    }
    t
}

pub fn prompt(d: &Digest, days: u64) -> (String, String) {
    (
        "You help improve a small local assistant. From the failure counts below, write at most 3 short lessons (one sentence each, under 250 characters) that the assistant should follow next time. Each must name the tool and give a concrete fix or habit. Skip failures that look like one-off network errors. Output a JSON array of strings and nothing else.".into(),
        render(d, days),
    )
}

pub fn parse_lessons(reply: &str) -> Vec<String> {
    let (Some(a), Some(b)) = (reply.find('['), reply.rfind(']')) else { return Vec::new() };
    serde_json::from_str::<Vec<String>>(&reply[a..=b]).unwrap_or_default().into_iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).take(3).collect()
}

/// "Rule for next time" lines from findings files, as (workflow, rule). Only the latest 5 rules per file.
pub fn findings_rules(files: &[(String, String)]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (wf, text) in files {
        let rules: Vec<String> = text
            .lines()
            .filter_map(|l| l.split_once("**Rule for next time:**").map(|(_, r)| r.trim().to_string()))
            .filter(|r| r.len() > 10 && r.len() < 360)
            .collect();
        let skip = rules.len().saturating_sub(5);
        out.extend(rules.into_iter().skip(skip).map(|r| (wf.clone(), r)));
    }
    out
}

/// True when the routine file already carries (most of) this rule.
pub fn already_present(file: &str, rule: &str) -> bool {
    let key = rule.chars().take(50).collect::<String>().to_lowercase();
    let key = key.trim_end_matches(|c: char| !c.is_alphanumeric());
    file.to_lowercase().contains(key)
}

/// A drafted lesson becomes a tool hint when it names one of the tools that failed.
pub fn as_proposal(text: String, d: &Digest) -> crate::lessons::Proposal {
    let lower = text.to_lowercase();
    let tool = d.by_tool.keys().filter(|t| t.as_str() != "?").filter(|t| lower.contains(&t.to_lowercase()) || lower.contains(&t.rsplit("__").next().unwrap_or("").to_lowercase())).max_by_key(|t| d.by_tool[*t]).cloned();
    match tool {
        Some(t) => crate::lessons::Proposal { baseline: d.by_tool.get(&t).copied(), tool: Some(t), kind: "tool_hint".into(), text, ..Default::default() },
        None => crate::lessons::Proposal { kind: "lesson".into(), text, ..Default::default() },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tallies_failures_and_skips_private_tools() {
        let ev = |t, v: Value| json!({ "t": t, "ev": v });
        let events = vec![
            ev(10, json!({"type":"tool_start","id":"a","name":"patch_content","input":{}})),
            ev(11, json!({"type":"tool_result","id":"a","content":"Error 40080: bad target 12","is_error":true})),
            ev(12, json!({"type":"tool_start","id":"b","name":"patch_content","input":{}})),
            ev(13, json!({"type":"tool_result","id":"b","content":"Error 40080: bad target 99","is_error":true})),
            ev(14, json!({"type":"tool_start","id":"c","name":"mail_read","input":{}})),
            ev(15, json!({"type":"tool_result","id":"c","content":"secret subject","is_error":true})),
            ev(16, json!({"type":"status","message":"The model tried to call a tool after the step limit; that call was dropped."})),
            ev(1, json!({"type":"tool_start","id":"z","name":"old","input":{}})),
        ];
        let (mut d, mut c) = (Digest::default(), HashMap::new());
        tally(&events, 5, &mut d, &mut c);
        assert_eq!(d.calls, 3);
        assert_eq!(d.step_limit_hits, 1);
        assert_eq!(c.len(), 1);
        assert_eq!(c[&("patch_content".to_string(), "Error #: bad target #".to_string())], 2);
    }

    #[test]
    fn extracts_rules_and_matches_tools() {
        let f = vec![("bod".to_string(), "- **Rule for next time:** Use put_content for whole-note edits.\n- **Worked:** x".to_string())];
        assert_eq!(findings_rules(&f), vec![("bod".to_string(), "Use put_content for whole-note edits.".to_string())]);
        assert!(already_present("Use put_content for whole-note edits, always", "Use put_content for whole-note edits."));
        let mut d = Digest::default();
        d.by_tool.insert("mcp__obsidian__obsidian_patch_content".into(), 19);
        let p = as_proposal("Before obsidian_patch_content, include the H1 title in the heading path.".into(), &d);
        assert_eq!((p.kind.as_str(), p.baseline), ("tool_hint", Some(19)));
        assert_eq!(as_proposal("Be brief.".into(), &d).kind, "lesson");
    }

    #[test]
    fn parses_lessons() {
        assert_eq!(parse_lessons("ok [\"a\", \" b \"] done"), vec!["a", "b"]);
        assert!(parse_lessons("no json").is_empty());
    }
}
