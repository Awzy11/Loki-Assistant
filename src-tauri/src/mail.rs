//! Read-only Gmail for Loki's LOCAL models (Tier 3: mail never goes to a cloud model).
//!
//! * Credentials live only on this server: the OAuth client in `~/.config/loki/gmail_client.json`
//!   (the file Google's console downloads) and one refresh token per account in
//!   `~/.config/loki/gmail_tokens/<email>.json` (both mode 600, never in config.toml, the repo,
//!   the vault or a chat). `ops/gmail-auth.py` creates the token files.
//! * Scope is `gmail.modify` (re-authorized 2026-10-27): mark read, labels, Trash and drafts. That scope can also send
//!   mail, so "no send, no permanent delete" is enforced here in code (see `tests::never_sends_or_deletes_for_good`).
//!   Write tools are `ConfirmRequired`, work in batches with an exact count first, and Trash skips starred mail.
//! * The agent tools built on this (`mail_inbox`, `mail_read`) are `Tier::LocalOnly` and are refused
//!   outright when the model answering is a cloud one.

use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct Account {
    pub email: String,
    refresh_token: String,
    /// The OAuth client this account was approved with (accounts can come from different Google projects).
    client_id: String,
    client_secret: String,
}

#[derive(Debug, Clone)]
pub struct MailConfig {
    pub accounts: Vec<Account>,
}

fn config_dir() -> Option<PathBuf> {
    crate::paths::config_dir().map(|d| d.join("loki"))
}

#[derive(Deserialize)]
struct ClientFile {
    installed: Option<ClientInner>,
    web: Option<ClientInner>,
}
#[derive(Deserialize)]
struct ClientInner {
    client_id: String,
    client_secret: String,
}

/// Every authorized account (each token file carries its own OAuth client; an older shared
/// `gmail_client.json` is the fallback), or `None` while mail isn't set up.
pub fn load_config() -> Option<MailConfig> {
    let dir = config_dir()?;
    let shared: Option<ClientInner> = std::fs::read(dir.join("gmail_client.json")).ok().and_then(|b| serde_json::from_slice::<ClientFile>(&b).ok()).and_then(|c| c.installed.or(c.web));
    let mut accounts = Vec::new();
    for e in std::fs::read_dir(dir.join("gmail_tokens")).ok()?.flatten() {
        if e.path().extension().and_then(|x| x.to_str()) != Some("json") {
            continue;
        }
        let Ok(bytes) = std::fs::read(e.path()) else { continue };
        let Ok(v) = serde_json::from_slice::<Value>(&bytes) else { continue };
        let (Some(email), Some(rt)) = (v["email"].as_str(), v["refresh_token"].as_str()) else { continue };
        let own = v["client_id"].as_str().zip(v["client_secret"].as_str());
        let (cid, secret) = match (own, &shared) {
            (Some((i, s)), _) => (i.to_string(), s.to_string()),
            (None, Some(c)) => (c.client_id.clone(), c.client_secret.clone()),
            _ => continue,
        };
        accounts.push(Account { email: email.to_string(), refresh_token: rt.to_string(), client_id: cid, client_secret: secret });
    }
    accounts.sort_by(|a, b| a.email.cmp(&b.email));
    (!accounts.is_empty()).then_some(MailConfig { accounts })
}

fn token_cache() -> &'static Mutex<HashMap<String, (Instant, String)>> {
    static C: OnceLock<Mutex<HashMap<String, (Instant, String)>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

fn http() -> reqwest::Client {
    reqwest::Client::builder().timeout(Duration::from_secs(15)).build().unwrap_or_default()
}

/// A short-lived access token for the account (refreshed from the stored refresh token, cached ~50 min).
async fn access_token(_cfg: &MailConfig, acct: &Account) -> Result<String, String> {
    if let Some((at, tok)) = token_cache().lock().unwrap().get(&acct.email) {
        if at.elapsed() < Duration::from_secs(3000) {
            return Ok(tok.clone());
        }
    }
    let res = http()
        .post("https://oauth2.googleapis.com/token")
        .form(&[("client_id", acct.client_id.as_str()), ("client_secret", acct.client_secret.as_str()), ("refresh_token", acct.refresh_token.as_str()), ("grant_type", "refresh_token")])
        .send()
        .await
        .map_err(|e| format!("couldn't reach Google: {e}"))?;
    let status = res.status();
    let v: Value = res.json().await.map_err(|e| e.to_string())?;
    match v["access_token"].as_str() {
        Some(t) if status.is_success() => {
            token_cache().lock().unwrap().insert(acct.email.clone(), (Instant::now(), t.to_string()));
            Ok(t.to_string())
        }
        _ => Err(format!("Google refused the token for {} ({}): re-run ops/gmail-auth.py", acct.email, v["error"].as_str().unwrap_or("unknown"))),
    }
}

async fn api_get(token: &str, path: &str, query: &[(&str, String)]) -> Result<Value, String> {
    let res = http().get(format!("https://gmail.googleapis.com/gmail/v1/users/me/{path}")).bearer_auth(token).query(query).send().await.map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        return Err(format!("Gmail returned {}", res.status()));
    }
    res.json().await.map_err(|e| e.to_string())
}

#[derive(Debug, Clone, PartialEq)]
pub struct MsgMeta {
    pub account: String,
    pub id: String,
    pub from: String,
    pub subject: String,
    /// Milliseconds since the epoch (Gmail's `internalDate`).
    pub at_ms: i64,
    pub snippet: String,
    pub unread: bool,
    /// Gmail labels (INBOX, UNREAD, CATEGORY_PROMOTIONS, …).
    pub labels: Vec<String>,
    /// Mass-mail signals in the headers: List-Unsubscribe, List-Id or a bulk Precedence.
    pub bulk: bool,
}

/// One message's metadata from a `format=metadata` response.
pub fn parse_meta(account: &str, m: &Value) -> Option<MsgMeta> {
    let header = |name: &str| m["payload"]["headers"].as_array().and_then(|h| h.iter().find(|x| x["name"].as_str().map(|n| n.eq_ignore_ascii_case(name)).unwrap_or(false))).and_then(|x| x["value"].as_str()).unwrap_or("").to_string();
    Some(MsgMeta {
        account: account.to_string(),
        id: m["id"].as_str()?.to_string(),
        from: header("From"),
        subject: header("Subject"),
        at_ms: m["internalDate"].as_str().and_then(|s| s.parse().ok()).unwrap_or(0),
        snippet: html_unescape(m["snippet"].as_str().unwrap_or("")),
        unread: m["labelIds"].as_array().map(|l| l.iter().any(|x| x == "UNREAD")).unwrap_or(false),
        labels: m["labelIds"].as_array().map(|l| l.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default(),
        bulk: !header("List-Unsubscribe").is_empty() || !header("List-Id").is_empty() || matches!(header("Precedence").to_lowercase().as_str(), "bulk" | "list" | "junk"),
    })
}

fn html_unescape(s: &str) -> String {
    s.replace("&#39;", "'").replace("&quot;", "\"").replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">")
}

/// `Name <a@b.c>` → `Name` (or the address when there is no name).
pub fn sender_name(from: &str) -> String {
    let f = from.trim();
    match f.split_once('<') {
        Some((name, rest)) => {
            let n = name.trim().trim_matches('"').trim();
            if n.is_empty() { rest.trim_end_matches('>').trim().to_string() } else { n.to_string() }
        }
        None => f.to_string(),
    }
}

fn b64url(s: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buf = 0u32;
    let mut bits = 0;
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'-' | b'+' => 62,
            b'_' | b'/' => 63,
            _ => continue,
        } as u32;
        buf = (buf << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }
    out
}

fn part_text(part: &Value, want: &str) -> Option<String> {
    if part["mimeType"].as_str() == Some(want) {
        if let Some(d) = part["body"]["data"].as_str() {
            return Some(String::from_utf8_lossy(&b64url(d)).to_string());
        }
    }
    part["parts"].as_array()?.iter().find_map(|p| part_text(p, want))
}

/// HTML to readable text: `<script>`, `<style>`, `<head>` and comments are dropped whole (that is where JSON-LD and CSS
/// live), block tags become line breaks, and entities are decoded.
pub(crate) fn strip_html(h: &str) -> String {
    let lower = h.to_lowercase();
    let mut src = String::with_capacity(h.len());
    let mut i = 0;
    'outer: while i < h.len() {
        if h.is_char_boundary(i) {
            for (open, close) in [("<script", "</script>"), ("<style", "</style>"), ("<head", "</head>"), ("<!--", "-->")] {
                if lower[i..].starts_with(open) {
                    match lower[i..].find(close) {
                        Some(end) => i += end + close.len(),
                        None => i = h.len(),
                    }
                    continue 'outer;
                }
            }
            let c = h[i..].chars().next().unwrap();
            src.push(c);
            i += c.len_utf8();
        } else {
            i += 1;
        }
    }
    let mut out = String::new();
    let mut tag = String::new();
    let mut in_tag = false;
    for c in src.chars() {
        match c {
            '<' => {
                in_tag = true;
                tag.clear();
            }
            '>' if in_tag => {
                in_tag = false;
                let name = tag.trim_start_matches('/').split(|c: char| !c.is_ascii_alphanumeric()).next().unwrap_or("").to_lowercase();
                if matches!(name.as_str(), "br" | "p" | "div" | "tr" | "li" | "h1" | "h2" | "h3" | "h4" | "table" | "blockquote" | "hr") {
                    out.push('\n');
                } else {
                    out.push(' ');
                }
            }
            _ if in_tag => tag.push(c),
            _ => out.push(c),
        }
    }
    let text = html_unescape(&out).replace("&nbsp;", " ").replace('\u{a0}', " ");
    let mut lines: Vec<String> = text.lines().map(|l| l.split_whitespace().collect::<Vec<_>>().join(" ")).collect();
    lines.dedup_by(|a, b| a.is_empty() && b.is_empty());
    lines.join("\n").trim().to_string()
}

/// A plain-text part that is really data (JSON), not prose.
fn looks_like_json(s: &str) -> bool {
    let t = s.trim();
    (t.starts_with('{') || t.starts_with('[')) && serde_json::from_str::<Value>(t).is_ok()
}

/// The readable body of a `format=full` message: its plain-text part, else its HTML with tags removed.
pub fn parse_body(m: &Value) -> String {
    let p = &m["payload"];
    part_text(p, "text/plain").filter(|t| !t.trim().is_empty() && !looks_like_json(t)).or_else(|| part_text(p, "text/html").map(|h| strip_html(&h))).unwrap_or_default()
}

fn pick<'a>(cfg: &'a MailConfig, account: Option<&str>) -> Vec<&'a Account> {
    match account.filter(|a| !a.is_empty() && *a != "all") {
        Some(a) => cfg.accounts.iter().filter(|x| x.email.eq_ignore_ascii_case(a)).collect(),
        None => cfg.accounts.iter().collect(),
    }
}

/// Messages matching a Gmail search (`in:inbox is:unread`, `from:…`) in one account, newest first.
pub async fn list(cfg: &MailConfig, acct: &Account, query: &str, limit: usize) -> Result<Vec<MsgMeta>, String> {
    let token = access_token(cfg, acct).await?;
    let ids = api_get(&token, "messages", &[("q", query.to_string()), ("maxResults", limit.min(25).to_string())]).await?;
    let mut out = Vec::new();
    for m in ids["messages"].as_array().cloned().unwrap_or_default() {
        let Some(id) = m["id"].as_str() else { continue };
        let q = [("format", "metadata".to_string()), ("metadataHeaders", "From".to_string()), ("metadataHeaders", "Subject".to_string()), ("metadataHeaders", "List-Unsubscribe".to_string()), ("metadataHeaders", "List-Id".to_string()), ("metadataHeaders", "Precedence".to_string())];
        if let Ok(full) = api_get(&token, &format!("messages/{id}"), &q).await {
            out.extend(parse_meta(&acct.email, &full));
        }
    }
    Ok(out)
}

/// Gmail's estimate of how many messages match a search (one cheap call).
pub async fn count_matching(cfg: &MailConfig, acct: &Account, query: &str) -> Option<u64> {
    // resultSizeEstimate is capped (~200) and unreliable, so count exactly by paging ids (500 per call).
    let token = access_token(cfg, acct).await.ok()?;
    let mut total: u64 = 0;
    let mut page: Option<String> = None;
    for _ in 0..400 {
        let mut q = vec![("q", query.to_string()), ("maxResults", "500".to_string()), ("fields", "nextPageToken,messages/id".to_string())];
        if let Some(p) = &page {
            q.push(("pageToken", p.clone()));
        }
        let v = api_get(&token, "messages", &q).await.ok()?;
        total += v["messages"].as_array().map_or(0, |a| a.len() as u64);
        page = v["nextPageToken"].as_str().map(String::from);
        if page.is_none() {
            break;
        }
    }
    Some(total)
}

/// Up to `n` messages matching a search (newest first), metadata only, fetched a few at a time.
pub async fn sample(cfg: &MailConfig, acct: &Account, query: &str, n: usize) -> Vec<MsgMeta> {
    use futures_util::StreamExt;
    let Ok(token) = access_token(cfg, acct).await else { return Vec::new() };
    let mut ids: Vec<String> = Vec::new();
    let mut page: Option<String> = None;
    while ids.len() < n {
        let mut q = vec![("q", query.to_string()), ("maxResults", (n - ids.len()).min(500).to_string())];
        if let Some(p) = &page {
            q.push(("pageToken", p.clone()));
        }
        let Ok(v) = api_get(&token, "messages", &q).await else { break };
        ids.extend(v["messages"].as_array().into_iter().flatten().filter_map(|m| m["id"].as_str().map(String::from)));
        page = v["nextPageToken"].as_str().map(String::from);
        if page.is_none() {
            break;
        }
    }
    ids.truncate(n);
    let hdrs = [("format", "metadata".to_string()), ("metadataHeaders", "From".to_string()), ("metadataHeaders", "Subject".to_string()), ("metadataHeaders", "List-Unsubscribe".to_string()), ("metadataHeaders", "List-Id".to_string()), ("metadataHeaders", "Precedence".to_string())];
    let email = acct.email.clone();
    futures_util::stream::iter(ids)
        .map(|id| {
            let token = token.clone();
            let hdrs = hdrs.clone();
            let email = email.clone();
            async move { api_get(&token, &format!("messages/{id}"), &hdrs).await.ok().and_then(|m| parse_meta(&email, &m)) }
        })
        .buffer_unordered(6)
        .filter_map(|x| async move { x })
        .collect()
        .await
}

/// Unread messages in the inbox (Gmail's own count).
pub async fn unread_count(cfg: &MailConfig, acct: &Account) -> Option<u64> {
    let token = access_token(cfg, acct).await.ok()?;
    api_get(&token, "labels/INBOX", &[]).await.ok()?["messagesUnread"].as_u64()
}

/// Gmail's own per-label counts (messages and conversations, unread and total) for the inbox, its tabs, spam and
/// trash. Counts only. Gmail's sidebar badges count conversations, so they read lower than message counts.
pub async fn label_stats() -> Vec<(String, serde_json::Value)> {
    let Some(cfg) = load_config() else { return Vec::new() };
    let mut out = Vec::new();
    for acct in &cfg.accounts {
        let Ok(token) = access_token(&cfg, acct).await else { continue };
        let mut m = serde_json::Map::new();
        for label in ["INBOX", "CATEGORY_PERSONAL", "CATEGORY_PROMOTIONS", "CATEGORY_SOCIAL", "CATEGORY_UPDATES", "CATEGORY_FORUMS", "SPAM", "TRASH"] {
            if let Ok(v) = api_get(&token, &format!("labels/{label}"), &[]).await {
                m.insert(label.to_lowercase(), serde_json::json!({ "messages_unread": v["messagesUnread"], "threads_unread": v["threadsUnread"], "messages_total": v["messagesTotal"], "threads_total": v["threadsTotal"] }));
            }
        }
        out.push((acct.email.clone(), serde_json::Value::Object(m)));
    }
    out
}

fn ago_ms(now_ms: i64, at_ms: i64) -> String {
    let s = ((now_ms - at_ms) / 1000).max(0);
    if s < 5400 { format!("{} min", (s / 60).max(1)) } else if s < 172_800 { format!("{} h", (s + 1800) / 3600) } else { format!("{} d", (s + 43_200) / 86_400) }
}

async fn api_post(token: &str, path: &str, body: &Value) -> Result<Value, String> {
    let res = http().post(format!("https://gmail.googleapis.com/gmail/v1/users/me/{path}")).bearer_auth(token).json(body).send().await.map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        let code = res.status();
        let text = res.text().await.unwrap_or_default();
        let why = if code.as_u16() == 403 && text.contains("insufficientPermissions") { " (this account is still on the read-only scope; re-run ops/gmail-auth.py with --modify)" } else { "" };
        return Err(format!("Gmail returned {code}{why}"));
    }
    Ok(res.json().await.unwrap_or(Value::Null))
}

/// Message ids matching a search, newest first, up to `cap`.
pub async fn all_ids(cfg: &MailConfig, acct: &Account, query: &str, cap: usize) -> Result<Vec<String>, String> {
    let token = access_token(cfg, acct).await?;
    let mut ids: Vec<String> = Vec::new();
    let mut page: Option<String> = None;
    while ids.len() < cap {
        let mut q = vec![("q", query.to_string()), ("maxResults", (cap - ids.len()).min(500).to_string()), ("fields", "nextPageToken,messages/id".to_string())];
        if let Some(p) = &page {
            q.push(("pageToken", p.clone()));
        }
        let v = api_get(&token, "messages", &q).await?;
        ids.extend(v["messages"].as_array().into_iter().flatten().filter_map(|m| m["id"].as_str().map(String::from)));
        page = v["nextPageToken"].as_str().map(String::from);
        if page.is_none() {
            break;
        }
    }
    Ok(ids)
}

/// A search must name what it selects (a label, tab, sender, age, …); a bare word could match anything.
pub fn check_query(q: &str) -> Result<String, String> {
    let q = q.trim();
    const KEYS: [&str; 14] = ["in:", "label:", "category:", "from:", "to:", "subject:", "older_than:", "newer_than:", "before:", "after:", "is:", "has:", "list:", "larger:"];
    if q.is_empty() || !KEYS.iter().any(|k| q.contains(k)) {
        return Err("The query must use at least one Gmail operator such as in:inbox, category:updates, from:…, older_than:30d or label:…".into());
    }
    Ok(q.to_string())
}

/// The id of a label by name, created if it doesn't exist yet.
async fn label_id(token: &str, name: &str) -> Result<String, String> {
    let list = api_get(token, "labels", &[]).await?;
    if let Some(id) = list["labels"].as_array().into_iter().flatten().find(|l| l["name"].as_str() == Some(name)).and_then(|l| l["id"].as_str()) {
        return Ok(id.to_string());
    }
    let made = api_post(token, "labels", &json!({ "name": name, "labelListVisibility": "labelShow", "messageListVisibility": "show" })).await?;
    made["id"].as_str().map(String::from).ok_or_else(|| "Gmail didn't return the new label's id".to_string())
}

/// Tool: exact count for a search in one account (counts only).
pub async fn tool_count(args: &Value) -> Result<String, String> {
    let cfg = load_config().ok_or("Mail isn't set up on this server")?;
    let query = check_query(args["query"].as_str().unwrap_or(""))?;
    let mut lines = Vec::new();
    for acct in pick(&cfg, args["account"].as_str()) {
        lines.push(format!("{}: {} messages match", acct.email, count_matching(&cfg, acct, &query).await.ok_or("couldn't count")?));
    }
    Ok(lines.join("\n"))
}

/// Tool: act on messages matching a search, one account at a time, in a capped batch. `action` is `read`, `label` or
/// `trash`. The caller must pass the count it saw (`expected_count`); a search that now matches more than 20% above
/// that is refused. Trash never takes starred, sent or draft mail and is never permanent.
pub async fn tool_act(action: &str, args: &Value) -> Result<String, String> {
    let cfg = load_config().ok_or("Mail isn't set up on this server")?;
    let account = args["account"].as_str().filter(|a| !a.is_empty() && *a != "all").ok_or("name one account")?;
    let acct = pick(&cfg, Some(account)).into_iter().next().ok_or("unknown account")?.clone();
    let mut query = check_query(args["query"].as_str().unwrap_or(""))?;
    let expected = args["expected_count"].as_u64().ok_or("expected_count is required: run mail_count first and pass its number")? as usize;
    if action == "trash" {
        query.push_str(" -is:starred -in:sent -in:drafts");
    }
    let limit = (args["limit"].as_u64().unwrap_or(500) as usize).clamp(1, 1000);
    let actual = count_matching(&cfg, &acct, &query).await.ok_or("couldn't count")? as usize;
    if actual == 0 {
        return Ok("Nothing matches; nothing changed.".into());
    }
    if actual as f64 > expected as f64 * 1.2 + 5.0 {
        return Err(format!("Stopped: the search matches {actual} messages, more than 20% above the {expected} you expected. Nothing changed. Re-check the query."));
    }
    let token = access_token(&cfg, &acct).await?;
    let (add, remove, done): (Vec<String>, Vec<String>, &str) = match action {
        "read" => (vec![], vec!["UNREAD".into()], "marked read"),
        "trash" => (vec!["TRASH".into()], vec!["INBOX".into()], "moved to Trash"),
        "archive" => (vec![], vec!["INBOX".into()], "archived"),
        "label" => {
            let name = args["label"].as_str().filter(|l| l.starts_with("Loki/")).ok_or("label must start with Loki/ (e.g. Loki/Junk)")?;
            (vec![label_id(&token, name).await?], vec![], "labelled")
        }
        _ => return Err("unknown action".into()),
    };
    let ids = all_ids(&cfg, &acct, &query, limit).await?;
    for chunk in ids.chunks(1000) {
        api_post(&token, "messages/batchModify", &json!({ "ids": chunk, "addLabelIds": add, "removeLabelIds": remove })).await?;
    }
    *data_cache().lock().unwrap() = None;
    Ok(format!("{}: {} messages {done}. {} more still match.", acct.email, ids.len(), actual.saturating_sub(ids.len())))
}

/// A click on the dashboard (the user's own action, no model involved): mark read/unread or move to Trash for specific
/// message ids in one account. Trash is never permanent.
pub async fn act_ids(account: &str, ids: &[String], action: &str) -> Result<usize, String> {
    let cfg = load_config().ok_or("Mail isn't set up on this server")?;
    let acct = pick(&cfg, Some(account)).into_iter().next().ok_or("unknown account")?.clone();
    let (add, remove): (Vec<&str>, Vec<&str>) = match action {
        "read" => (vec![], vec!["UNREAD"]),
        "unread" => (vec!["UNREAD"], vec![]),
        "trash" => (vec!["TRASH"], vec!["INBOX"]),
        "archive" => (vec![], vec!["INBOX"]),
        "unarchive" => (vec!["INBOX"], vec![]),
        _ => return Err("unknown action".into()),
    };
    if ids.iter().any(|i| i.is_empty() || !i.chars().all(|c| c.is_ascii_alphanumeric())) {
        return Err("bad message id".into());
    }
    let token = access_token(&cfg, &acct).await?;
    api_post(&token, "messages/batchModify", &json!({ "ids": ids, "addLabelIds": add, "removeLabelIds": remove })).await?;
    *data_cache().lock().unwrap() = None;
    Ok(ids.len())
}

/// Adds a `Loki/…` label to specific messages in one account (nothing is removed, nothing is trashed). Used to mark digested
/// newsletters for deletion so the user can review and clear them in Gmail.
pub async fn label_ids(account: &str, ids: &[String], label: &str) -> Result<usize, String> {
    if !label.starts_with("Loki/") {
        return Err("label must start with Loki/".into());
    }
    if ids.iter().any(|i| i.is_empty() || !i.chars().all(|c| c.is_ascii_alphanumeric())) {
        return Err("bad message id".into());
    }
    let cfg = load_config().ok_or("Mail isn't set up on this server")?;
    let acct = pick(&cfg, Some(account)).into_iter().next().ok_or("unknown account")?.clone();
    let token = access_token(&cfg, &acct).await?;
    let lid = label_id(&token, label).await?;
    for chunk in ids.chunks(1000) {
        api_post(&token, "messages/batchModify", &json!({ "ids": chunk, "addLabelIds": [lid] })).await?;
    }
    *data_cache().lock().unwrap() = None;
    Ok(ids.len())
}

/// Tool: save a reply to one message as a Gmail DRAFT (never sent).
pub async fn tool_draft(args: &Value) -> Result<String, String> {
    use base64::Engine;
    let cfg = load_config().ok_or("Mail isn't set up on this server")?;
    let account = args["account"].as_str().filter(|a| !a.is_empty()).ok_or("name the account")?;
    let acct = pick(&cfg, Some(account)).into_iter().next().ok_or("unknown account")?.clone();
    let id = args["id"].as_str().ok_or("id is required")?;
    let body = args["body"].as_str().filter(|b| !b.trim().is_empty()).ok_or("body is required")?;
    let token = access_token(&cfg, &acct).await?;
    let msg = api_get(&token, &format!("messages/{id}"), &[("format", "metadata".into()), ("metadataHeaders", "From".into()), ("metadataHeaders", "Reply-To".into()), ("metadataHeaders", "Subject".into()), ("metadataHeaders", "Message-ID".into()), ("metadataHeaders", "References".into())]).await?;
    let h = |n: &str| msg["payload"]["headers"].as_array().and_then(|a| a.iter().find(|x| x["name"].as_str().is_some_and(|k| k.eq_ignore_ascii_case(n)))).and_then(|x| x["value"].as_str()).unwrap_or("").replace(['\r', '\n'], " ");
    let to = if h("Reply-To").is_empty() { h("From") } else { h("Reply-To") };
    if to.is_empty() {
        return Err("that message has no sender to reply to".into());
    }
    let subject = h("Subject");
    let subject = if subject.to_lowercase().starts_with("re:") { subject } else { format!("Re: {subject}") };
    let refs = if h("References").is_empty() { h("Message-ID") } else { format!("{} {}", h("References"), h("Message-ID")) };
    let raw = format!("To: {to}\r\nSubject: {subject}\r\nIn-Reply-To: {}\r\nReferences: {refs}\r\nContent-Type: text/plain; charset=UTF-8\r\n\r\n{body}", h("Message-ID"));
    let mut payload = json!({ "message": { "raw": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw) } });
    if let Some(t) = msg["threadId"].as_str() {
        payload["message"]["threadId"] = json!(t);
    }
    let d = api_post(&token, "drafts", &payload).await?;
    *data_cache().lock().unwrap() = None;
    Ok(format!("Saved as a draft in {} (draft id {}). Nothing was sent; the user reviews and sends it themselves.", acct.email, d["id"].as_str().unwrap_or("?")))
}

/// What the local model gets to read about one message, as text lines (for `mail_inbox`).
pub fn describe(m: &MsgMeta, now_ms: i64) -> String {
    format!("[{}] {} | from {} | {} | {} ago{} | {}", m.id, m.account, sender_name(&m.from), if m.subject.is_empty() { "(no subject)" } else { &m.subject }, ago_ms(now_ms, m.at_ms), if m.unread { " | unread" } else { "" }, m.snippet.chars().take(160).collect::<String>())
}

/// Tool: list/search messages (read-only).
pub async fn tool_inbox(args: &Value) -> Result<String, String> {
    let cfg = load_config().ok_or("Mail isn't set up on this server (see docs/MAIL.md)")?;
    let query = args["query"].as_str().filter(|q| !q.trim().is_empty()).unwrap_or("in:inbox is:unread");
    let limit = (args["limit"].as_u64().unwrap_or(30) as usize).clamp(1, 100);
    let now_ms = crate::needs_you::now_unix() * 1000;
    let mut lines = Vec::new();
    for acct in pick(&cfg, args["account"].as_str()) {
        let ms = sample(&cfg, acct, query, limit).await;
        lines.extend(ms.iter().map(|m| describe(m, now_ms)));
        if ms.len() >= limit {
            // Say so when the list stops at the limit: otherwise a partial list reads as the whole inbox.
            lines.push(format!("[{}: showing the newest {} matches; more exist. Raise `limit` (max 100) or narrow the query, e.g. add newer_than:7d or from:…]", acct.email, ms.len()));
        }
    }
    Ok(if lines.is_empty() { "No messages match.".to_string() } else { lines.join("\n") })
}

/// Tool: read one message's text (read-only), truncated.
pub async fn tool_read(args: &Value) -> Result<String, String> {
    let cfg = load_config().ok_or("Mail isn't set up on this server (see docs/MAIL.md)")?;
    let id = args["id"].as_str().ok_or("give the message id from mail_inbox")?;
    let candidates = pick(&cfg, args["account"].as_str());
    for acct in candidates {
        let token = access_token(&cfg, acct).await?;
        if let Ok(m) = api_get(&token, &format!("messages/{id}"), &[("format", "full".to_string())]).await {
            let meta = parse_meta(&acct.email, &m);
            let head = meta.map(|m| format!("From: {}\nSubject: {}\nAccount: {}\n\n", m.from, m.subject, m.account)).unwrap_or_default();
            let body = parse_body(&m);
            let cut: String = body.chars().take(12000).collect();
            return Ok(format!("{head}{cut}{}", if body.chars().count() > 12000 { "\n… [truncated]" } else { "" }));
        }
    }
    Err("no message with that id in the accounts searched".into())
}

/// One message for the in-app viewer (the user's own click): headers, readable text (HTML tags removed, never rendered as
/// HTML), attachment names and a Gmail link. Reading does not mark it read.
pub async fn read_message(account: &str, id: &str) -> Result<Value, String> {
    let cfg = load_config().ok_or("Mail isn't set up on this server")?;
    let acct = pick(&cfg, Some(account)).into_iter().next().ok_or("unknown account")?.clone();
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err("bad message id".into());
    }
    let token = access_token(&cfg, &acct).await?;
    let m = api_get(&token, &format!("messages/{id}"), &[("format", "full".to_string())]).await?;
    let h = |n: &str| m["payload"]["headers"].as_array().and_then(|a| a.iter().find(|x| x["name"].as_str().is_some_and(|k| k.eq_ignore_ascii_case(n)))).and_then(|x| x["value"].as_str()).unwrap_or("").to_string();
    fn names(p: &Value, out: &mut Vec<String>) {
        if let Some(f) = p["filename"].as_str().filter(|f| !f.is_empty()) {
            out.push(f.to_string());
        }
        for c in p["parts"].as_array().into_iter().flatten() {
            names(c, out);
        }
    }
    let mut attachments = Vec::new();
    names(&m["payload"], &mut attachments);
    let body = parse_body(&m);
    let body: String = body.chars().take(40_000).collect();
    Ok(json!({
        "account": acct.email, "id": id, "from": h("From"), "to": h("To"), "cc": h("Cc"), "subject": h("Subject"), "date": h("Date"),
        "body": body, "attachments": attachments,
        "unread": m["labelIds"].as_array().is_some_and(|l| l.iter().any(|x| x == "UNREAD")),
        "link": format!("https://mail.google.com/mail/?authuser={}#all/{}", urlencoding::encode(&acct.email), id),
    }))
}

fn data_cache() -> &'static Mutex<Option<(Instant, Vec<(String, Option<u64>, Vec<MsgMeta>)>)>> {
    static C: OnceLock<Mutex<Option<(Instant, Vec<(String, Option<u64>, Vec<MsgMeta>)>)>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(None))
}

/// Per account: unread count and the newest unread primary-inbox messages. Cached 3 minutes.
pub async fn dashboard_data() -> Option<Vec<(String, Option<u64>, Vec<MsgMeta>)>> {
    if let Some((at, d)) = data_cache().lock().unwrap().as_ref() {
        if at.elapsed() < Duration::from_secs(180) {
            return Some(d.clone());
        }
    }
    let cfg = load_config()?;
    let mut out = Vec::new();
    for acct in &cfg.accounts {
        let count = unread_count(&cfg, acct).await;
        let mut msgs = sample(&cfg, acct, "in:inbox is:unread", 40).await;
        msgs.sort_by_key(|m| std::cmp::Reverse(m.at_ms));
        out.push((acct.email.clone(), count, msgs));
    }
    *data_cache().lock().unwrap() = Some((Instant::now(), out.clone()));
    Some(out)
}

#[cfg(test)]
mod tests {
    #[test]
    fn never_sends_or_deletes_for_good() {
        // The wider scope could do both, so keep the code honest. The needles are built so this test never matches itself.
        let src = [include_str!("mail.rs"), include_str!("agent/tools.rs"), include_str!("mail_inbox.rs")].concat();
        for needle in [["messages", "/send"].concat(), ["drafts", "/send"].concat(), ["batch", "Delete"].concat(), ["messages", "/delete"].concat(), ["Method::", "DELETE"].concat()] {
            assert!(!src.contains(&needle), "found {needle}");
        }
    }

    #[test]
    fn html_to_text_drops_script_and_style_and_keeps_breaks() {
        let h = "<html><head><title>x</title><style>p{color:red}</style></head><body><script type=\"application/ld+json\">{\"@type\":\"Event\"}</script><p>Hello&nbsp;there</p><div>Line two<br>Line three</div><!-- hidden --></body></html>";
        let t = super::strip_html(h);
        assert_eq!(t, "Hello there\n\nLine two\nLine three");
        assert!(!t.contains("@type") && !t.contains("color"));
        assert!(super::looks_like_json("{\"a\":1}") && !super::looks_like_json("Hi {there}"));
    }

    #[test]
    fn queries_must_name_what_they_select() {
        assert!(super::check_query("").is_err());
        assert!(super::check_query("newsletter").is_err());
        assert!(super::check_query("in:inbox category:updates older_than:30d").is_ok());
    }

    use super::*;

    #[test]
    fn parses_message_metadata_and_bodies() {
        let m = json!({"id": "18c", "internalDate": "1792000000000", "snippet": "Can you send the Q3 numbers&#39; summary?", "labelIds": ["INBOX", "UNREAD"],
            "payload": {"headers": [{"name": "From", "value": "\"Col Rivera\" <rivera@example.mil>"}, {"name": "subject", "value": "Q3 numbers"}]}});
        let meta = parse_meta("me@example.com", &m).unwrap();
        assert_eq!((meta.id.as_str(), meta.subject.as_str(), meta.unread), ("18c", "Q3 numbers", true));
        assert_eq!(sender_name(&meta.from), "Col Rivera");
        assert!(meta.snippet.contains("numbers' summary"));
        assert_eq!(sender_name("plain@example.com"), "plain@example.com");
        assert_eq!(sender_name("<a@b.c>"), "a@b.c");
        // "Hello <b>there</b>" base64url
        let html = json!({"payload": {"mimeType": "multipart/alternative", "parts": [{"mimeType": "text/html", "body": {"data": "SGVsbG8gPGI-dGhlcmU8L2I-"}}]}});
        assert_eq!(parse_body(&html), "Hello there");
        let plain = json!({"payload": {"mimeType": "multipart/alternative", "parts": [{"mimeType": "text/plain", "body": {"data": "SGVsbG8gdGhlcmU"}}, {"mimeType": "text/html", "body": {"data": "PGI-eDwvYj4"}}]}});
        assert_eq!(parse_body(&plain), "Hello there");
    }

    #[test]
    fn describes_a_message_for_the_model() {
        let m = MsgMeta { account: "a@x".into(), id: "1".into(), from: "Bo <bo@x>".into(), subject: "Lunch?".into(), at_ms: 1_000_000, snippet: "Are you free".into(), unread: true, labels: vec![], bulk: false };
        assert_eq!(describe(&m, 1_000_000 + 2 * 3600 * 1000), "[1] a@x | from Bo | Lunch? | 2 h ago | unread | Are you free");
    }
}
