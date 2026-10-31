//! "Troubleshoot with Claude": gathers an evidence bundle for one conversation — the transcript,
//! what the turns did (event log), recent server log, and what the client's UI actually looked
//! like (screenshot + layout facts) — and lets Claude read it, read-only, to diagnose. Private
//! (local-only) turns are withheld. Jobs run in the background; clients poll for the result.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};

/// What the client reports about its own rendering.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct UiSnapshot {
    #[serde(default)]
    pub kind: String,
    /// Free-form facts: viewport, user agent, theme, widget width, console errors…
    #[serde(default)]
    pub info: serde_json::Value,
    /// Text/markup as the client rendered it (DOM snippet, or the sidebar's rich-text strings).
    #[serde(default)]
    pub rendered: String,
    /// Base64 PNG of the client's UI, when it can take one.
    #[serde(default)]
    pub screenshot_b64: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Job {
    pub id: String,
    /// "running" | "done" | "error"
    pub status: String,
    pub result: String,
    pub dir: String,
    pub created: u64,
}

static JOBS: LazyLock<Mutex<HashMap<String, Job>>> = LazyLock::new(Default::default);

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub fn new_job(dir: &PathBuf) -> String {
    let id = format!("t{:x}{:04x}", now(), rand::random::<u16>());
    JOBS.lock().unwrap().insert(id.clone(), Job { id: id.clone(), status: "running".into(), result: String::new(), dir: dir.display().to_string(), created: now() });
    id
}

pub fn finish(id: &str, ok: bool, result: String) {
    let snapshot = {
        let mut jobs = JOBS.lock().unwrap();
        let Some(j) = jobs.get_mut(id) else { return };
        j.status = if ok { "done" } else { "error" }.into();
        j.result = result;
        j.clone()
    };
    // Persist next to the evidence so a report survives a server restart (Fix can use it later).
    if !snapshot.dir.is_empty() {
        let _ = std::fs::write(std::path::Path::new(&snapshot.dir).join("job.json"), serde_json::to_vec_pretty(&snapshot).unwrap_or_default());
    }
}

pub fn job(id: &str) -> Option<Job> {
    if let Some(j) = JOBS.lock().unwrap().get(id) {
        return Some(j.clone());
    }
    // Not in memory (server restarted): look for a persisted report.
    let root = crate::paths::data_dir()?.join("loki").join("troubleshoot");
    for e in std::fs::read_dir(root).ok()?.flatten() {
        if let Ok(raw) = std::fs::read_to_string(e.path().join("job.json")) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
                if v["id"] == id {
                    return Some(Job {
                        id: id.to_string(),
                        status: v["status"].as_str().unwrap_or("error").to_string(),
                        result: v["result"].as_str().unwrap_or("").to_string(),
                        dir: e.path().display().to_string(),
                        created: v["created"].as_u64().unwrap_or(0),
                    });
                }
            }
        }
    }
    None
}

pub fn bundle_dir() -> anyhow::Result<PathBuf> {
    let d = crate::paths::data_dir().ok_or_else(|| anyhow::anyhow!("no data dir"))?.join("loki").join("troubleshoot").join(format!("{}-{:04x}", now(), rand::random::<u16>()));
    std::fs::create_dir_all(&d)?;
    // Keep the 20 newest bundles.
    if let Some(parent) = d.parent() {
        if let Ok(rd) = std::fs::read_dir(parent) {
            let mut all: Vec<_> = rd.filter_map(|e| e.ok()).map(|e| e.path()).collect();
            all.sort();
            let excess = all.len().saturating_sub(20);
            for old in all.into_iter().take(excess) {
                let _ = std::fs::remove_dir_all(old);
            }
        }
    }
    Ok(d)
}

pub fn write_ui(dir: &PathBuf, ui: &UiSnapshot) {
    use base64::Engine as _;
    let meta = serde_json::json!({ "client_kind": ui.kind, "info": ui.info });
    let _ = std::fs::write(dir.join("ui.json"), serde_json::to_vec_pretty(&meta).unwrap_or_default());
    if !ui.rendered.is_empty() {
        let clipped: String = ui.rendered.chars().take(120_000).collect();
        let _ = std::fs::write(dir.join("ui-rendered.txt"), clipped);
    }
    if let Some(b64) = &ui.screenshot_b64 {
        if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64.trim()) {
            let _ = std::fs::write(dir.join("ui.png"), bytes);
        }
    }
}

pub fn prompt(note: &str, dir: &PathBuf, repo: &str, has_png: bool) -> String {
    format!(
        "You are helping the user troubleshoot Loki, their personal AI agent (Rust server + React web/PWA app + a Quickshell/QML \
desktop sidebar). The user pressed \"Troubleshoot with Claude\" in one of the Loki UIs. Evidence is in {dir}:\n\
- transcript.md — the conversation with timestamps, thumbs ratings and audit verdicts\n\
- events.txt — what the turns did: tool calls, results, status lines, reasoning traces, errors (one per line, unix time)\n\
- server_log.txt — recent loki-server log (may be absent)\n\
- ui.json — facts about the client that sent this (kind, viewport, user agent, etc.)\n\
- ui-rendered.txt — the text/markup the client rendered, if provided\n\
{png}\
Loki's source is at {repo} (src/ = React app, src-tauri/src = server). The sidebar's QML is in the sidebar plugin folder ($LOKI_SIDEBAR_DIR, default ~/.config/omarchy/plugins/loki.sidebar/) on their laptop and may not be readable from here.\n\n\
the user's note: {note}\n\n\
Do this: read the evidence; say what is actually wrong, separating (1) data/server problems, (2) rendering problems in the \
specific client, (3) model-quality problems. If you can see the screenshot, describe what it shows and where it differs from what \
the transcript says should be shown. Compare against how the same content renders in other clients if the evidence shows it. \
Then give concrete, minimal fixes naming files and functions. Do not modify anything; you have read-only tools. Be concise \
and use short markdown sections.",
        dir = dir.display(),
        repo = repo,
        note = if note.trim().is_empty() { "(none given)" } else { note.trim() },
        png = if has_png { "- ui.png — a screenshot of the client's UI at the moment they pressed the button (open it with the Read tool and look at it)\n" } else { "" },
    )
}

pub fn claude_args(dir: &PathBuf, repo: &str) -> Vec<String> {
    let mut a: Vec<String> = ["--tools", "Read,Grep,Glob", "--allowedTools", "Read", "Grep", "Glob", "--setting-sources", "", "--no-session-persistence", "--add-dir"].iter().map(|s| s.to_string()).collect();
    a.push(dir.display().to_string());
    a.push(repo.to_string());
    a
}


// ---- past reports ----

fn reports_root() -> Option<PathBuf> {
    Some(crate::paths::data_dir()?.join("loki").join("troubleshoot"))
}

/// Finds a report's bundle directory by its job id.
pub fn find_dir(id: &str) -> Option<PathBuf> {
    for e in std::fs::read_dir(reports_root()?).ok()?.flatten() {
        if let Ok(raw) = std::fs::read_to_string(e.path().join("job.json")) {
            if serde_json::from_str::<serde_json::Value>(&raw).map(|v| v["id"] == id).unwrap_or(false) {
                return Some(e.path());
            }
        }
    }
    None
}

fn read_json(p: &std::path::Path) -> Option<serde_json::Value> {
    serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()
}

fn first_line(md: &str) -> String {
    md.lines()
        .map(|l| l.trim().trim_start_matches('#').trim())
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .chars()
        .take(160)
        .collect()
}

fn summarize_dir(dir: &std::path::Path, with_result: bool) -> Option<serde_json::Value> {
    let job = read_json(&dir.join("job.json"))?;
    let ui = read_json(&dir.join("ui.json")).unwrap_or_default();
    let note = std::fs::read_to_string(dir.join("note.txt")).unwrap_or_default();
    let fix = read_json(&dir.join("fix-state.json"));
    let result = job["result"].as_str().unwrap_or("");
    let mut v = serde_json::json!({
        "id": job["id"],
        "created": job["created"],
        "status": job["status"],
        "client": ui["client_kind"],
        "note": note.trim(),
        "summary": first_line(result),
        "fix": fix.as_ref().map(|f| serde_json::json!({ "state": f["state"], "at": f["at"] })),
    });
    if with_result {
        v["result"] = serde_json::Value::String(result.to_string());
        v["fix"] = fix.unwrap_or(serde_json::Value::Null);
    }
    Some(v)
}

/// Newest-first list of past troubleshoot reports with their fix state.
pub fn list_reports(limit: usize) -> Vec<serde_json::Value> {
    let Some(root) = reports_root() else { return Vec::new() };
    let mut out: Vec<serde_json::Value> = std::fs::read_dir(root).ok().into_iter().flatten().flatten().filter_map(|e| summarize_dir(&e.path(), false)).collect();
    out.sort_by(|a, b| b["created"].as_u64().cmp(&a["created"].as_u64()));
    out.truncate(limit);
    out
}

pub fn report_detail(id: &str) -> Option<serde_json::Value> {
    summarize_dir(&find_dir(id)?, true)
}

/// Remembers what a Fix did for a report: "staged" (awaiting review), "applied", "discarded" or
/// "error". The staged outcome (summary, diffstat, what it touches) is kept so the review can be
/// reopened later from any device.
pub fn record_fix(id: &str, action: &str, outcome: serde_json::Value) -> Result<(), String> {
    let dir = find_dir(id).ok_or("unknown report")?;
    let ok = outcome["status"] == "done";
    let state = match (action, ok) {
        ("applying", _) => "applying",
        (_, false) => "error",
        ("fix", true) => "staged",
        ("apply", true) => "applied",
        ("discard", true) => "discarded",
        _ => return Err("action must be fix, applying, apply or discard".into()),
    };
    let path = dir.join("fix-state.json");
    let mut cur = read_json(&path).unwrap_or_else(|| serde_json::json!({}));
    cur["state"] = state.into();
    cur["at"] = now().into();
    if action == "fix" {
        cur["outcome"] = outcome;
    } else if action == "applying" {
        // keep the staged outcome so the review is still there if the apply never reports back
    } else {
        cur["last"] = outcome;
    }
    std::fs::write(path, serde_json::to_vec_pretty(&cur).unwrap_or_default()).map_err(|e| e.to_string())
}
