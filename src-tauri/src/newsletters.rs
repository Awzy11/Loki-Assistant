//! Newsletter sources for the morning digests. the user tags a sender `news` or `market` in the mail dashboard; the n8n
//! workflows then ask for that sender's recent messages. Deterministic on purpose: a message is handed out only if its
//! sender is tagged AND it carries mass-mail headers (List-Unsubscribe, List-Id, bulk Precedence). No model decides
//! eligibility, so finance, school, cyber or work mail from anyone else can never reach a digest. Gmail is only read,
//! never changed (nothing is marked read); "already digested" lives in this server's own state. The bodies leave here
//! only for a local model.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};
use std::sync::Mutex;

pub const BUCKETS: &[&str] = &["news", "market", "ignore"];
const MAX_SENDERS_PER_QUERY: usize = 25;
const TEXT_CHARS: usize = 1800;

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct Sender {
    bucket: String,
    #[serde(default)]
    name: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct NState {
    senders: BTreeMap<String, Sender>,
    digested: Vec<String>,
}

static STATE: Mutex<Option<NState>> = Mutex::new(None);

fn path() -> Option<std::path::PathBuf> {
    let d = dirs::data_dir()?.join("loki").join("mail_inbox");
    std::fs::create_dir_all(&d).ok()?;
    Some(d.join("newsletters.json"))
}

fn with<R>(f: impl FnOnce(&mut NState) -> R) -> R {
    let mut g = STATE.lock().unwrap();
    if g.is_none() {
        *g = Some(path().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default());
    }
    f(g.as_mut().unwrap())
}

fn save() {
    let snapshot = with(|s| serde_json::to_vec(s).ok());
    let (Some(p), Some(b)) = (path(), snapshot) else { return };
    let tmp = p.with_extension("json.tmp");
    #[cfg(unix)]
    let ok = {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp).and_then(|mut f| f.write_all(&b)).is_ok()
    };
    #[cfg(not(unix))]
    let ok = std::fs::write(&tmp, &b).is_ok();
    if ok {
        let _ = std::fs::rename(tmp, p);
    }
}

/// `Name <a@b.c>` → `a@b.c`, lowercased.
pub fn address(from: &str) -> String {
    let f = from.trim();
    let inner = match (f.rfind('<'), f.rfind('>')) {
        (Some(a), Some(b)) if a < b => &f[a + 1..b],
        _ => f,
    };
    inner.trim().trim_matches('"').to_lowercase()
}

/// An address safe to put in a Gmail search (no quotes, parentheses or spaces).
pub fn valid_address(a: &str) -> bool {
    a.contains('@') && a.len() <= 120 && a.chars().all(|c| c.is_ascii_alphanumeric() || "@._%+-".contains(c))
}

/// The people who write to this inbox, most frequent first: names and counts only, never message text.
pub fn senders_json() -> Value {
    let mut seen: BTreeMap<String, (String, usize, i64)> = BTreeMap::new();
    for (from, at) in crate::mail_inbox::sender_samples() {
        let a = address(&from);
        if !valid_address(&a) {
            continue;
        }
        let e = seen.entry(a).or_insert((crate::mail::sender_name(&from), 0, 0));
        e.1 += 1;
        e.2 = e.2.max(at);
    }
    let tagged: BTreeMap<String, Sender> = with(|s| s.senders.clone());
    for (a, s) in &tagged {
        seen.entry(a.clone()).or_insert((s.name.clone(), 0, 0));
    }
    let mut rows: Vec<Value> = seen
        .into_iter()
        .map(|(a, (name, n, last))| json!({ "address": a, "name": name, "count": n, "last_ms": last, "bucket": tagged.get(&a).map(|s| s.bucket.clone()).unwrap_or_default() }))
        .collect();
    // tagged senders first, then by how often they write
    rows.sort_by(|x, y| (y["bucket"].as_str().unwrap_or("").is_empty() as u8).cmp(&(x["bucket"].as_str().unwrap_or("").is_empty() as u8)).reverse().then(y["count"].as_u64().cmp(&x["count"].as_u64())));
    rows.truncate(150);
    json!({ "senders": rows, "buckets": BUCKETS })
}

/// Tags a sender (`news`, `market`, `ignore`) or clears the tag (empty bucket).
pub fn set_bucket(addr: &str, name: &str, bucket: &str) -> Result<(), String> {
    let a = addr.trim().to_lowercase();
    if !valid_address(&a) {
        return Err("that doesn't look like an email address".into());
    }
    if !bucket.is_empty() && !BUCKETS.contains(&bucket) {
        return Err(format!("bucket must be one of {}", BUCKETS.join(", ")));
    }
    with(|s| {
        if bucket.is_empty() {
            s.senders.remove(&a);
        } else {
            s.senders.insert(a, Sender { bucket: bucket.to_string(), name: name.chars().take(80).collect() });
        }
    });
    save();
    Ok(())
}

/// The Gmail search for the tagged senders of one bucket, or None if there are none.
pub fn query_for(addrs: &[String]) -> Option<String> {
    if addrs.is_empty() {
        return None;
    }
    let list: Vec<String> = addrs.iter().take(MAX_SENDERS_PER_QUERY).map(|a| format!("from:{a}")).collect();
    Some(format!("({}) newer_than:3d", list.join(" OR ")))
}

fn flatten(body: &str) -> String {
    let no_urls = regex::Regex::new(r"https?://\S+").unwrap().replace_all(body, " ");
    let t = no_urls.split_whitespace().collect::<Vec<_>>().join(" ");
    t.chars().take(TEXT_CHARS).collect()
}

/// Recent messages from this bucket's tagged senders that carry mass-mail headers and haven't been digested yet.
pub async fn pending(bucket: &str, limit: usize) -> Result<Value, String> {
    if !BUCKETS.contains(&bucket) || bucket == "ignore" {
        return Err("bucket must be news or market".into());
    }
    let addrs: Vec<String> = with(|s| s.senders.iter().filter(|(_, v)| v.bucket == bucket).map(|(a, _)| a.clone()).collect());
    let Some(q) = query_for(&addrs) else { return Ok(json!({ "items": [] })) };
    let cfg = crate::mail::load_config().ok_or("Mail isn't set up on this server")?;
    let done: HashSet<String> = with(|s| s.digested.iter().cloned().collect());
    let mut metas = Vec::new();
    for acct in &cfg.accounts {
        if let Ok(list) = crate::mail::list(&cfg, acct, &q, 25).await {
            metas.extend(list.into_iter().filter(|m| m.bulk && !done.contains(&m.id) && addrs.contains(&address(&m.from))));
        }
    }
    metas.sort_by(|a, b| b.at_ms.cmp(&a.at_ms));
    let mut items = Vec::new();
    for m in metas.into_iter().take(limit.clamp(1, 20)) {
        let Ok(full) = crate::mail::read_message(&m.account, &m.id).await else { continue };
        items.push(json!({
            "id": m.id, "account": m.account, "sender": crate::mail::sender_name(&m.from), "address": address(&m.from), "subject": m.subject, "date_ms": m.at_ms,
            "text": flatten(full["body"].as_str().unwrap_or("")), "link": full["link"],
        }));
    }
    Ok(json!({ "items": items }))
}

/// Remembers messages as digested so the next morning skips them. Gmail itself is untouched.
pub fn mark_digested(ids: &[String]) {
    with(|s| {
        for id in ids.iter().filter(|i| !i.is_empty() && i.chars().all(|c| c.is_ascii_alphanumeric())) {
            if !s.digested.contains(id) {
                s.digested.push(id.clone());
            }
        }
        let n = s.digested.len();
        if n > 3000 {
            s.digested.drain(..n - 3000);
        }
    });
    save();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses() {
        assert_eq!(address("The Hustle <News@TheHustle.co>"), "news@thehustle.co");
        assert_eq!(address("\"A B\" <a@b.c>"), "a@b.c");
        assert_eq!(address("plain@x.org"), "plain@x.org");
        assert!(valid_address("a.b+c@x-y.org"));
        assert!(!valid_address("a@b.c OR from:boss@x.org"));
        assert!(!valid_address("no-at-sign"));
    }

    #[test]
    fn queries_and_buckets() {
        assert_eq!(query_for(&[]), None);
        assert_eq!(query_for(&["a@b.c".into(), "d@e.f".into()]).unwrap(), "(from:a@b.c OR from:d@e.f) newer_than:3d");
        assert!(set_bucket("not an address", "", "news").is_err());
        assert!(set_bucket("a@b.c", "", "weird").is_err());
    }

    #[test]
    fn text_is_flattened_and_trimmed() {
        let t = flatten("Hello\n\n  world  https://x.com/track?id=1 bye");
        assert_eq!(t, "Hello world bye");
        assert!(flatten(&"word ".repeat(2000)).chars().count() <= TEXT_CHARS);
    }
}
