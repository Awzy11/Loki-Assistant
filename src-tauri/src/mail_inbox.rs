//! The mail dashboard's data: every message in each inbox (read or not), sorted by the local model into a
//! function category, an urgency and a class, plus an optional two-matrix pass (Eisenhower, then GAIN).
//! Results live on this server (`~/.local/share/loki/mail_inbox/inbox.json`, mode 0600), never in the vault, and
//! only local models read the mail. The dashboard's buttons (mark read, trash, draft) are the user's own clicks.

use crate::providers::{LlmProvider, Message};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub const CATEGORIES: &[&str] = &["finance", "school", "home", "work", "investments", "tech", "cyber", "other"];
pub const URGENCIES: &[&str] = &["urgent", "priority", "routine", "discard"];
/// Archived = not in the inbox, not sent or a draft; spam and trash are excluded by Gmail itself.
pub const ARCHIVED_QUERY: &str = "-in:inbox -in:sent -in:drafts -in:chats newer_than:90d";
pub const CLASSES: &[&str] = &["action", "review", "fyi", "receipt", "junk", "stale"];

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Item {
    pub account: String,
    pub id: String,
    pub from: String,
    pub subject: String,
    pub at_ms: i64,
    pub unread: bool,
    pub snippet: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub urgency: String,
    #[serde(default)]
    pub class: String,
    /// Looks like it wants a reply.
    #[serde(default)]
    pub reply: bool,
    #[serde(default)]
    pub why: String,
    /// Eisenhower quadrant `q1`..`q4`, GAIN letter and a short next step, once the two-matrix pass has run.
    #[serde(default)]
    pub quadrant: String,
    #[serde(default)]
    pub gain: String,
    #[serde(default)]
    pub next: String,
    /// Archived in Gmail (out of the inbox, still in All Mail). Scanned like the rest, but kept out of the dashboard's
    /// counts and filters unless the user picks the Archived or All view.
    #[serde(default)]
    pub archived: bool,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct State {
    pub status: String,
    pub phase: String,
    pub done: usize,
    pub total: usize,
    pub error: String,
    pub updated: i64,
    pub items: Vec<Item>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);
static RUNNING: AtomicBool = AtomicBool::new(false);

fn path() -> Option<std::path::PathBuf> {
    let d = crate::paths::data_dir()?.join("loki").join("mail_inbox");
    std::fs::create_dir_all(&d).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o700));
    }
    Some(d.join("inbox.json"))
}

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut g = STATE.lock().unwrap();
    if g.is_none() {
        let mut s: State = path().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        if s.status == "running" {
            s.status = "interrupted".into(); // the server restarted mid-scan
        }
        *g = Some(s);
    }
    f(g.as_mut().unwrap())
}

fn save() {
    let snapshot = with(|s| s.clone());
    if let Some(p) = path() {
        let tmp = p.with_extension("json.tmp");
        if let Ok(b) = serde_json::to_vec(&snapshot) {
            #[cfg(unix)]
            let ok = {
                use std::io::Write;
                use std::os::unix::fs::OpenOptionsExt;
                std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp).and_then(|mut f| f.write_all(&b)).is_ok()
            };
            #[cfg(not(unix))]
            let ok = std::fs::write(&tmp, b).is_ok();
            if ok {
                let _ = std::fs::rename(tmp, p);
            }
        }
    }
}

/// Everything the dashboard shows.
pub fn page_json() -> Value {
    with(|s| json!({ "status": s.status, "phase": s.phase, "done": s.done, "total": s.total, "error": s.error, "updated": s.updated, "items": s.items,
                     "categories": CATEGORIES, "urgencies": URGENCIES, "classes": CLASSES }))
}

/// (From header, received ms) of every sorted message: lets the newsletter settings list who writes most. Names and times only.
pub fn sender_samples() -> Vec<(String, i64)> {
    with(|s| s.items.iter().map(|i| (i.from.clone(), i.at_ms)).collect())
}

/// Counts only (safe to read in a Claude session).
pub fn status_json() -> Value {
    with(|s| {
        // inbox figures only; archived mail is counted on its own
        let count = |f: &dyn Fn(&Item) -> bool| s.items.iter().filter(|i| !i.archived && f(i)).count();
        json!({ "status": s.status, "phase": s.phase, "done": s.done, "total": s.total, "error": s.error, "updated": s.updated,
                "items": count(&|_| true), "archived": s.items.iter().filter(|i| i.archived).count(), "unread": count(&|i| i.unread), "unclassified": s.items.iter().filter(|i| i.class.is_empty()).count(),
                "slip_throughs": count(&|i| !i.unread && (i.class == "action" || matches!(i.urgency.as_str(), "urgent" | "priority"))),
                "triaged": count(&|i| !i.quadrant.is_empty()) })
    })
}

fn clean(v: &str, allowed: &[&str], fallback: &str) -> String {
    let v = v.trim().to_lowercase();
    if allowed.contains(&v.as_str()) { v } else { fallback.to_string() }
}

/// Parsed model answer for one message in a batch.
#[derive(Debug, PartialEq)]
pub struct Verdict {
    pub n: usize,
    pub category: String,
    pub urgency: String,
    pub class: String,
    pub reply: bool,
    pub why: String,
}

pub fn classify_prompt(items: &[&Item], today: &str) -> (String, String) {
    let who = crate::profile::get().person_intro();
    let system = format!(
        "You sort one person's email inbox. About the person: {who}. Today is {today}.\\n\\
For each numbered message answer with one JSON object, and output a JSON array of them and nothing else:\\n\\
{{\\\"n\\\":1,\\\"category\\\":\\\"school\\\",\\\"urgency\\\":\\\"routine\\\",\\\"class\\\":\\\"fyi\\\",\\\"reply\\\":false,\\\"why\\\":\\\"six words\\\"}}\\n\\
category (function of the message): finance (bills, banking, taxes, pay, insurance), school (College, courses, assignments, Moodle), home (family, house, housing, kids, errands), work (military duties, unit, official business), investments (brokerage, markets, retirement), tech (software, services, accounts, projects), cyber (security alerts, breaches, passwords, 2FA), other.\\n\\
urgency: urgent (a deadline within about 3 days or real consequences now), priority (matters, needs doing this week or two), routine (normal, no rush), discard (no value: promotions, expired, noise).\\n\\
class: action (he must do something), review (look over or approve, no reply), fyi (useful info), receipt (order or payment record), junk (promotions and noise), stale (time-sensitive and now expired).\\n\\
reply: true only if a person is waiting on a written reply from him. A message already read can still be action: judge the content, not the read flag. why: at most 8 words, no quotes from the message."
    );
    let mut user = String::new();
    for (i, m) in items.iter().enumerate() {
        let age = ((chrono_now_ms() - m.at_ms) / 86_400_000).max(0);
        user.push_str(&format!("{}. from: {} | subject: {} | {} | {}d old | preview: {}\\n", i + 1, crate::mail::sender_name(&m.from), m.subject, if m.unread { "unread" } else { "read" }, age, m.snippet.chars().take(240).collect::<String>()));
    }
    (system, user)
}

fn chrono_now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn json_array(reply: &str) -> Vec<Value> {
    let (Some(a), Some(b)) = (reply.find('['), reply.rfind(']')) else { return Vec::new() };
    serde_json::from_str::<Vec<Value>>(&reply[a..=b]).unwrap_or_default()
}

pub fn parse_verdicts(reply: &str) -> Vec<Verdict> {
    json_array(reply)
        .into_iter()
        .filter_map(|v| {
            Some(Verdict {
                n: v["n"].as_u64()? as usize,
                category: clean(v["category"].as_str().unwrap_or(""), CATEGORIES, "other"),
                urgency: clean(v["urgency"].as_str().unwrap_or(""), URGENCIES, "routine"),
                class: clean(v["class"].as_str().unwrap_or(""), CLASSES, "fyi"),
                reply: v["reply"].as_bool().unwrap_or(false),
                why: v["why"].as_str().unwrap_or("").chars().take(80).collect(),
            })
        })
        .collect()
}

pub fn matrix_prompt(items: &[&Item], today: &str) -> (String, String) {
    let who = crate::profile::get().person_intro();
    let system = format!(
        "You run a two-matrix filter on one person's email: first Eisenhower, then GAIN. About the person: {who}. Most of what reaches this inbox is theirs to do. Today is {today}.\\n\\
Output a JSON array and nothing else, one object per message: {{\\\"n\\\":1,\\\"quadrant\\\":\\\"q2\\\",\\\"gain\\\":\\\"i\\\",\\\"next\\\":\\\"open the form and sign\\\"}}\\n\\
quadrant: q1 = urgent and important (hard deadline within 2 weeks, real consequences, blocks other work); q2 = important, not urgent (moves toward his goals, belongs in a sprint); q3 = urgent, not important (time-sensitive but not his to own, could be delegated or automated); q4 = neither (stale, superseded, not worth a slot; leans q4 if nothing has happened for six weeks).\\n\\
gain, only for q1 and q2 (empty string otherwise): g = Give to AI (repetitive, rules-based; AI does it, he reviews); a = Accelerate (research or synthesis; he directs, AI does the heavy lifting); i = Integrate (needs his judgment, relationships or presence; AI assists); n = No AI (physical presence or personal relationships).\\n\\
next: at most 12 words, the first concrete step. No quotes from the message."
    );
    let mut user = String::new();
    for (i, m) in items.iter().enumerate() {
        let age = ((chrono_now_ms() - m.at_ms) / 86_400_000).max(0);
        user.push_str(&format!("{}. from: {} | subject: {} | {}d old | sorted as {}/{}/{} | preview: {}\\n", i + 1, crate::mail::sender_name(&m.from), m.subject, age, m.category, m.urgency, m.class, m.snippet.chars().take(200).collect::<String>()));
    }
    (system, user)
}

/// (n, quadrant, gain, next)
pub fn parse_matrix(reply: &str) -> Vec<(usize, String, String, String)> {
    json_array(reply)
        .into_iter()
        .filter_map(|v| {
            let q = clean(v["quadrant"].as_str().unwrap_or(""), &["q1", "q2", "q3", "q4"], "");
            if q.is_empty() {
                return None;
            }
            let gain = clean(v["gain"].as_str().unwrap_or(""), &["g", "a", "i", "n"], "");
            let gain = if q == "q1" || q == "q2" { gain } else { String::new() };
            Some((v["n"].as_u64()? as usize, q, gain, v["next"].as_str().unwrap_or("").chars().take(100).collect()))
        })
        .collect()
}

fn set_phase(phase: &str, done: usize, total: usize) {
    with(|s| {
        s.phase = phase.to_string();
        s.done = done;
        s.total = total;
    });
}

/// Reads every inbox message, sorts the ones not yet sorted, and (when asked) runs the two-matrix pass.
/// Only a local model is ever used. One scan at a time.
pub async fn scan(provider: Arc<dyn LlmProvider>, matrix: bool) {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    with(|s| {
        s.status = "running".into();
        s.error.clear();
    });
    let outcome = scan_inner(provider, matrix).await;
    with(|s| {
        match outcome {
            Ok(()) => {
                s.status = "done".into();
                s.phase = "Finished".into();
                s.updated = chrono_now_ms() / 1000;
            }
            Err(e) => {
                s.status = "error".into();
                s.error = e;
            }
        }
    });
    save();
    RUNNING.store(false, Ordering::SeqCst);
}

async fn scan_inner(provider: Arc<dyn LlmProvider>, matrix: bool) -> Result<(), String> {
    let cfg = crate::mail::load_config().ok_or("Mail isn't set up on this server")?;
    let today = crate::conversation_log::date_format("%Y-%m-%d").unwrap_or_default();
    let prev: HashMap<(String, String), Item> = with(|s| s.items.iter().map(|i| ((i.account.clone(), i.id.clone()), i.clone())).collect());
    let mut items: Vec<Item> = Vec::new();
    set_phase("Reading the inboxes", 0, 0);
    for acct in &cfg.accounts {
        let mut metas: Vec<(crate::mail::MsgMeta, bool)> = crate::mail::sample(&cfg, acct, "in:inbox", 500).await.into_iter().map(|m| (m, false)).collect();
        metas.sort_by_key(|(m, _)| std::cmp::Reverse(m.at_ms));
        // recently archived mail is read too, so nothing slips through just because it left the inbox
        let mut arch: Vec<(crate::mail::MsgMeta, bool)> = crate::mail::sample(&cfg, acct, ARCHIVED_QUERY, 300).await.into_iter().map(|m| (m, true)).collect();
        arch.sort_by_key(|(m, _)| std::cmp::Reverse(m.at_ms));
        metas.extend(arch);
        for (m, archived) in metas {
            let mut it = prev.get(&(m.account.clone(), m.id.clone())).cloned().unwrap_or_default();
            it.archived = archived;
            it.account = m.account;
            it.id = m.id;
            it.from = m.from;
            it.subject = m.subject;
            it.at_ms = m.at_ms;
            it.unread = m.unread;
            it.snippet = m.snippet.chars().take(240).collect();
            items.push(it);
        }
    }
    with(|s| s.items = items.clone());
    save();

    let todo: Vec<usize> = (0..items.len()).filter(|&i| items[i].class.is_empty()).collect();
    let mtodo = |items: &Vec<Item>| -> Vec<usize> { (0..items.len()).filter(|&i| matrix && items[i].quadrant.is_empty() && !matches!(items[i].class.as_str(), "junk" | "stale" | "receipt")).collect() };
    let total = todo.len();
    for (b, chunk) in todo.chunks(12).enumerate() {
        set_phase("Sorting messages with the local model", b * 12, total);
        let refs: Vec<&Item> = chunk.iter().map(|&i| &items[i]).collect();
        let (system, user) = classify_prompt(&refs, &today);
        let msgs = vec![Message::system(system), Message::user(user)];
        let reply = match tokio::time::timeout(std::time::Duration::from_secs(150), provider.chat_low_effort(&msgs)).await {
            Ok(Ok(r)) => r.content,
            _ => continue, // left unsorted; the next scan retries them
        };
        for v in parse_verdicts(&reply) {
            if let Some(&i) = v.n.checked_sub(1).and_then(|k| chunk.get(k)) {
                let it = &mut items[i];
                it.category = v.category;
                it.urgency = v.urgency;
                it.class = v.class;
                it.reply = v.reply;
                it.why = v.why;
            }
        }
        with(|s| s.items = items.clone());
        save();
    }

    if matrix {
        let todo = mtodo(&items);
        let total = todo.len();
        for (b, chunk) in todo.chunks(12).enumerate() {
            set_phase("Running the two-matrix triage", b * 12, total);
            let refs: Vec<&Item> = chunk.iter().map(|&i| &items[i]).collect();
            let (system, user) = matrix_prompt(&refs, &today);
            let msgs = vec![Message::system(system), Message::user(user)];
            let Ok(Ok(reply)) = tokio::time::timeout(std::time::Duration::from_secs(150), provider.chat_low_effort(&msgs)).await else { continue };
            for (n, q, g, next) in parse_matrix(&reply.content) {
                if let Some(&i) = n.checked_sub(1).and_then(|k| chunk.get(k)) {
                    items[i].quadrant = q;
                    items[i].gain = g;
                    items[i].next = next;
                }
            }
            with(|s| s.items = items.clone());
            save();
        }
    }
    with(|s| s.items = items);
    Ok(())
}

/// A click on the dashboard: mark read/unread or move to Trash for specific messages in one account.
pub async fn act(account: &str, ids: &[String], action: &str) -> Result<String, String> {
    if ids.is_empty() || ids.len() > 200 {
        return Err("choose between 1 and 200 messages".into());
    }
    let n = crate::mail::act_ids(account, ids, action).await?;
    with(|s| match action {
        "trash" => s.items.retain(|i| !(i.account == account && ids.contains(&i.id))),
        "archive" | "unarchive" => s.items.iter_mut().filter(|i| i.account == account && ids.contains(&i.id)).for_each(|i| i.archived = action == "archive"),
        _ => s.items.iter_mut().filter(|i| i.account == account && ids.contains(&i.id)).for_each(|i| i.unread = action == "unread"),
    });
    save();
    Ok(format!("{n} message(s) {}", match action { "trash" => "moved to Trash", "unread" => "marked unread", "archive" => "archived", "unarchive" => "moved back to the inbox", _ => "marked read" }))
}

/// Has the local model write a reply to one message, saves it as a Gmail draft (never sent), and returns the text.
pub async fn draft(provider: Arc<dyn LlmProvider>, account: &str, id: &str) -> Result<Value, String> {
    let body = crate::mail::tool_read(&json!({ "account": account, "id": id })).await?;
    let prof = crate::profile::get();
    let (name, about) = (prof.person_name(), prof.person.about.trim());
    let context = if about.is_empty() { String::new() } else { format!(" ({about})") };
    let system = format!("You write email replies for {name}{context}. Write a short, courteous, direct reply in plain text, in their voice, ending with just '{name}'. Do not invent facts, dates or commitments: where they must decide something, put it in [square brackets]. Output only the reply text, no subject line, no commentary.");
    let msgs = vec![Message::system(system), Message::user(body.chars().take(8000).collect::<String>())];
    let reply = tokio::time::timeout(std::time::Duration::from_secs(150), provider.chat_low_effort(&msgs)).await.map_err(|_| "the local model took too long".to_string())?.map_err(|e| e.to_string())?.content;
    let text = reply.trim().to_string();
    if text.is_empty() {
        return Err("the local model returned an empty draft".into());
    }
    let saved = crate::mail::tool_draft(&json!({ "account": account, "id": id, "body": text })).await?;
    Ok(json!({ "draft": text, "saved": saved }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_verdicts_and_falls_back_on_bad_labels() {
        let v = parse_verdicts("here: [{\"n\":1,\"category\":\"Finance\",\"urgency\":\"URGENT\",\"class\":\"action\",\"reply\":true,\"why\":\"bill due\"},{\"n\":2,\"category\":\"weird\",\"urgency\":\"x\",\"class\":\"y\"}] ok");
        assert_eq!((v[0].category.as_str(), v[0].urgency.as_str(), v[0].reply), ("finance", "urgent", true));
        assert_eq!((v[1].category.as_str(), v[1].urgency.as_str(), v[1].class.as_str()), ("other", "routine", "fyi"));
        assert!(parse_verdicts("no json").is_empty());
    }

    #[test]
    fn matrix_gain_only_for_q1_q2() {
        let m = parse_matrix("[{\"n\":1,\"quadrant\":\"q1\",\"gain\":\"a\",\"next\":\"x\"},{\"n\":2,\"quadrant\":\"q4\",\"gain\":\"g\"},{\"n\":3,\"quadrant\":\"bad\"}]");
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].2, "a");
        assert_eq!(m[1].2, "");
    }

    #[test]
    fn status_counts_slip_throughs() {
        *STATE.lock().unwrap() = Some(State { items: vec![
            Item { id: "a".into(), unread: false, class: "action".into(), ..Default::default() },
            Item { id: "b".into(), unread: true, class: "action".into(), ..Default::default() },
            Item { id: "c".into(), unread: false, class: "fyi".into(), urgency: "priority".into(), ..Default::default() },
            Item { id: "d".into(), unread: false, class: "fyi".into(), urgency: "routine".into(), ..Default::default() },
            Item { id: "e".into(), unread: false, class: "action".into(), archived: true, ..Default::default() },
        ], ..Default::default() });
        let s = status_json();
        assert_eq!((s["items"].as_u64(), s["slip_throughs"].as_u64(), s["unread"].as_u64(), s["archived"].as_u64()), (Some(4), Some(2), Some(1), Some(1)));
    }
}
