//! Web Push to the installed PWA: subscriptions are stored on the server, and a finished turn that
//! nobody is watching raises a phone notification. Encryption and VAPID signing are done by
//! `ops/loki-push.py` (pywebpush) in a venv created by `ops/loki-push-setup.sh`; this module only
//! stores subscriptions and runs that helper. Without the setup it reports itself unconfigured.
//!
//! Payloads are end-to-end encrypted to the device (the push service only relays ciphertext), and
//! turns that stayed local for privacy carry a generic body instead of reply text.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;

static LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subscription {
    pub endpoint: String,
    pub keys: Keys,
    #[serde(default)]
    pub client_id: String,
    #[serde(default)]
    pub created: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Keys {
    pub p256dh: String,
    pub auth: String,
}

fn data_dir() -> Option<PathBuf> {
    let d = crate::paths::data_dir()?.join("loki");
    std::fs::create_dir_all(&d).ok()?;
    Some(d)
}

fn conf_dir() -> Option<PathBuf> {
    crate::paths::config_dir().map(|d| d.join("loki"))
}

fn subs_path() -> Option<PathBuf> {
    Some(data_dir()?.join("push_subs.json"))
}

fn load() -> Vec<Subscription> {
    subs_path().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn save(subs: &[Subscription]) {
    if let Some(p) = subs_path() {
        let tmp = p.with_extension("json.tmp");
        if std::fs::write(&tmp, serde_json::to_vec_pretty(subs).unwrap_or_default()).is_ok() {
            let _ = std::fs::rename(tmp, p);
        }
    }
}

pub fn public_key() -> Option<String> {
    std::fs::read_to_string(conf_dir()?.join("vapid_public.txt")).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn python() -> Option<PathBuf> {
    let p = data_dir()?.join("push-venv").join("bin").join("python");
    p.exists().then_some(p)
}

fn helper() -> Option<PathBuf> {
    let p = std::env::current_dir().ok()?.join("ops").join("loki-push.py");
    p.exists().then_some(p)
}

pub fn configured() -> bool {
    public_key().is_some() && python().is_some() && helper().is_some() && conf_dir().map(|d| d.join("vapid_private.pem").exists()).unwrap_or(false)
}

pub fn subscribe(sub: Subscription) {
    let _g = LOCK.lock().unwrap();
    let mut subs = load();
    subs.retain(|s| s.endpoint != sub.endpoint);
    subs.push(Subscription { created: now(), ..sub });
    save(&subs);
}

pub fn unsubscribe(endpoint: &str) {
    let _g = LOCK.lock().unwrap();
    let mut subs = load();
    subs.retain(|s| s.endpoint != endpoint);
    save(&subs);
}

pub fn count() -> usize {
    load().len()
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Sends `payload` (`{title, body, url, tag}`) to every subscribed device, in the background.
/// Subscriptions the push service reports gone are pruned. Returns immediately.
pub fn send_in_background(payload: serde_json::Value) {
    if !configured() {
        return;
    }
    let subs = {
        let _g = LOCK.lock().unwrap();
        load()
    };
    if subs.is_empty() {
        return;
    }
    eprintln!("[loki-debug] push: sending '{}' to {} device(s)", payload["title"].as_str().unwrap_or(""), subs.len());
    tokio::task::spawn_blocking(move || {
        let (Some(py), Some(helper), Some(conf)) = (python(), helper(), conf_dir()) else { return };
        let req = serde_json::json!({
            "subs": subs,
            "payload": payload,
            "vapid_private": conf.join("vapid_private.pem"),
            "contact": crate::profile::get().push_contact(),
        });
        use std::io::Write;
        let child = std::process::Command::new(py).arg(helper).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn();
        let Ok(mut child) = child else { return };
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(req.to_string().as_bytes());
        }
        let Ok(out) = child.wait_with_output() else { return };
        match serde_json::from_slice::<serde_json::Value>(&out.stdout) {
            Ok(v) => {
                if let Some(gone) = v["gone"].as_array() {
                    for e in gone.iter().filter_map(|e| e.as_str()) {
                        unsubscribe(e);
                    }
                }
                if v["errors"].as_array().map(|a| !a.is_empty()).unwrap_or(false) {
                    eprintln!("[loki-debug] push errors: {}", v["errors"]);
                }
            }
            Err(_) => eprintln!("[loki-debug] push helper failed: {}", String::from_utf8_lossy(&out.stderr).chars().take(200).collect::<String>()),
        }
    });
}

/// Body text for a notification: the reply's first line of prose, markdown stripped, ~140 chars.
pub fn preview(reply: &str) -> String {
    let line = reply
        .lines()
        .map(|l| l.trim().trim_start_matches(['#', '>', '-', '*', ' ']).replace(['*', '`', '_'], ""))
        .find(|l| !l.is_empty())
        .unwrap_or_default();
    let mut out: String = line.chars().take(140).collect();
    if line.chars().count() > 140 {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_strips_markdown_and_truncates() {
        assert_eq!(preview("## Done\nbody"), "Done");
        assert_eq!(preview("\n\n**All set** — wrote the note."), "All set — wrote the note.");
        let long = "x".repeat(300);
        assert_eq!(preview(&long).chars().count(), 141);
    }
}
