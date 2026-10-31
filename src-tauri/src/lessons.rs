//! Lessons learned from Claude's audits of the local model. Approved lessons are injected into
//! the local model's system prompt (`agent::system_prompt`) — the "tailoring" half of the
//! audit loop. Pending lessons wait for the user's approval; nothing is injected unapproved.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;

static LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lesson {
    pub id: String,
    pub text: String,
    /// "pending" | "approved" | "rejected"
    pub status: String,
    pub created: u64,
    /// Audit that suggested it (session id + turn index), if any.
    #[serde(default)]
    pub source: Option<String>,
    /// "lesson" (default) | "tool_hint" | "routine_edit". Only the first two reach the model's prompt.
    #[serde(default = "default_kind")]
    pub kind: String,
    /// tool_hint: the tool it is about.
    #[serde(default)]
    pub tool: Option<String>,
    /// routine_edit: vault file to edit, and the text appended to it on approval (shown as a diff).
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub append: Option<String>,
    /// tool_hint: failures of that tool in the 7 days before it was proposed.
    #[serde(default)]
    pub baseline: Option<usize>,
    #[serde(default)]
    pub approved_at: Option<u64>,
    /// Filled in a week after approval: what changed.
    #[serde(default)]
    pub effect: Option<String>,
}

fn default_kind() -> String {
    "lesson".into()
}

/// A proposal not yet stored.
#[derive(Debug, Clone, Default)]
pub struct Proposal {
    pub text: String,
    pub kind: String,
    pub tool: Option<String>,
    pub target: Option<String>,
    pub append: Option<String>,
    pub baseline: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditSettings {
    /// "off" | "downvotes" | "all_local"
    pub auto: String,
}

impl Default for AuditSettings {
    fn default() -> Self {
        Self { auto: "downvotes".into() }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Store {
    #[serde(default)]
    lessons: Vec<Lesson>,
    #[serde(default)]
    settings: AuditSettings,
}

fn path() -> Option<PathBuf> {
    let d = dirs::data_dir()?.join("loki");
    std::fs::create_dir_all(&d).ok()?;
    Some(d.join("lessons.json"))
}

fn load() -> Store {
    path().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn save(s: &Store) {
    if let Some(p) = path() {
        let tmp = p.with_extension("json.tmp");
        if let Ok(b) = serde_json::to_vec_pretty(s) {
            if std::fs::write(&tmp, b).is_ok() {
                let _ = std::fs::rename(tmp, p);
            }
        }
    }
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub fn list() -> Vec<Lesson> {
    let _g = LOCK.lock().unwrap();
    let mut l = load().lessons;
    l.sort_by(|a, b| b.created.cmp(&a.created));
    l
}

pub fn settings() -> AuditSettings {
    let _g = LOCK.lock().unwrap();
    load().settings
}

pub fn set_auto(mode: &str) -> Result<(), String> {
    if !matches!(mode, "off" | "downvotes" | "all_local") {
        return Err("auto must be off, downvotes or all_local".into());
    }
    let _g = LOCK.lock().unwrap();
    let mut s = load();
    s.settings.auto = mode.to_string();
    save(&s);
    Ok(())
}

/// Adds suggested lessons as pending, skipping near-duplicates of existing ones.
pub fn add_pending(texts: &[String], source: &str) -> usize {
    let props: Vec<Proposal> = texts.iter().map(|t| Proposal { text: t.clone(), kind: "lesson".into(), ..Default::default() }).collect();
    add_proposals(&props, source)
}

/// Adds proposals (any kind) as pending, skipping duplicates.
pub fn add_proposals(props: &[Proposal], source: &str) -> usize {
    let _g = LOCK.lock().unwrap();
    let mut s = load();
    let mut added = 0;
    for p in props {
        let t = p.text.trim();
        if t.is_empty() || t.len() > 400 {
            continue;
        }
        let key = t.to_lowercase();
        if s.lessons.iter().any(|l| l.text.to_lowercase() == key) {
            continue;
        }
        s.lessons.push(Lesson {
            id: format!("l{:x}{:04x}", now(), rand::random::<u16>()),
            text: t.to_string(),
            status: "pending".into(),
            created: now(),
            source: Some(source.to_string()),
            kind: if p.kind.is_empty() { "lesson".into() } else { p.kind.clone() },
            tool: p.tool.clone(),
            target: p.target.clone(),
            append: p.append.clone(),
            baseline: p.baseline,
            approved_at: None,
            effect: None,
        });
        added += 1;
    }
    if added > 0 {
        save(&s);
    }
    added
}

pub fn get(id: &str) -> Option<Lesson> {
    list().into_iter().find(|l| l.id == id)
}

pub fn set_effect(id: &str, effect: &str) {
    let _g = LOCK.lock().unwrap();
    let mut s = load();
    if let Some(l) = s.lessons.iter_mut().find(|l| l.id == id) {
        l.effect = Some(effect.to_string());
        save(&s);
    }
}

/// `action`: approve | reject | delete | edit (with `text`).
pub fn update(id: &str, action: &str, text: Option<&str>) -> Result<(), String> {
    let _g = LOCK.lock().unwrap();
    let mut s = load();
    let Some(pos) = s.lessons.iter().position(|l| l.id == id) else { return Err("no such lesson".into()) };
    match action {
        "approve" => {
            s.lessons[pos].status = "approved".into();
            s.lessons[pos].approved_at = Some(now());
        }
        "reject" => s.lessons[pos].status = "rejected".into(),
        "delete" => {
            s.lessons.remove(pos);
        }
        "edit" => {
            let t = text.map(str::trim).filter(|t| !t.is_empty() && t.len() <= 400).ok_or("edit needs 1–400 characters of text")?;
            s.lessons[pos].text = t.to_string();
        }
        _ => return Err("action must be approve, reject, delete or edit".into()),
    }
    save(&s);
    Ok(())
}

/// The block appended to the local model's system prompt: approved lessons, newest first,
/// capped so it can't crowd out the context window.
pub fn prompt_block() -> String {
    let approved: Vec<Lesson> = list().into_iter().filter(|l| l.status == "approved" && l.kind != "routine_edit").collect();
    if approved.is_empty() {
        return String::new();
    }
    let mut out = String::from("\n\nLessons learned from past reviews of your answers (follow them):\n");
    let mut used = 0;
    for l in approved.iter().take(15) {
        used += l.text.len();
        if used > 2400 {
            break;
        }
        out.push_str(&format!("- {}\n", l.text));
    }
    out
}

/// Short restatement placed just before the latest user message.
pub fn reminder_block() -> String {
    let approved: Vec<Lesson> = list().into_iter().filter(|l| l.status == "approved" && l.kind != "routine_edit").collect();
    if approved.is_empty() {
        return String::new();
    }
    let mut out = String::from("Before you answer, follow these rules learned from reviews of your earlier answers:\n");
    for l in approved.iter().take(8) {
        out.push_str(&format!("- {}\n", l.text));
    }
    out
}
