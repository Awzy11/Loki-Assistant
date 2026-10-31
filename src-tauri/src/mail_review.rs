//! Local-only inbox review: sample each mailbox's unread mail by age, let the local model (the local model) sort
//! it into action / stale / junk / keep, scale the sample up to estimates, and draft a cleanup plan.
//!
//! Everything here stays on this server (Tier 3): the results (which hold senders and subjects) live in
//! `~/.local/share/loki/mail_review/review.json` (mode 600) and are shown only on the user's own screens. A
//! cloud model is never used, and nothing is written to the vault. The status endpoint exposes counts only.

use crate::mail::{self, MsgMeta};
use crate::providers::{LlmProvider, Message};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// (label, Gmail search fragment, how many messages to sample) — oldest buckets are sampled less.
pub const BUCKETS: &[(&str, &str, usize)] = &[
    ("last 7 days", "newer_than:7d", 120),
    ("7-30 days", "older_than:7d newer_than:30d", 120),
    ("1-3 months", "older_than:30d newer_than:90d", 100),
    ("3-12 months", "older_than:90d newer_than:1y", 80),
    ("over a year", "older_than:1y", 80),
];

pub const CLASSES: &[&str] = &["action", "review", "fyi", "receipt", "junk", "stale", "unsure"];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reviewed {
    pub account: String,
    pub id: String,
    pub from: String,
    pub subject: String,
    pub age_days: i64,
    pub bucket: usize,
    pub bulk: bool,
    pub category: String,
    pub snippet: String,
    pub class: String,
    pub why: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BucketStat {
    pub label: String,
    pub total: u64,
    pub sampled: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountReview {
    pub email: String,
    pub unread_total: u64,
    pub buckets: Vec<BucketStat>,
    pub items: Vec<Reviewed>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Review {
    /// "idle" | "running" | "done" | "error"
    pub status: String,
    pub phase: String,
    pub done: usize,
    pub total: usize,
    pub started: i64,
    pub finished: Option<i64>,
    pub error: Option<String>,
    pub accounts: Vec<AccountReview>,
}

pub type Shared = Arc<Mutex<Review>>;

fn store_path() -> Option<std::path::PathBuf> {
    dirs::data_dir().map(|d| d.join("loki").join("mail_review").join("review.json"))
}

pub fn load() -> Review {
    store_path().and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Review { status: "idle".into(), ..Default::default() })
}

fn save(r: &Review) {
    let Some(p) = store_path() else { return };
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
        }
    }
    if std::fs::write(&p, serde_json::to_vec(r).unwrap_or_default()).is_ok() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600));
        }
    }
}

fn category(labels: &[String]) -> String {
    labels.iter().find_map(|l| l.strip_prefix("CATEGORY_")).map(|c| c.to_lowercase()).unwrap_or_default()
}

/// Gmail's own signals are enough to call the obvious mass mail junk without asking the model.
pub fn heuristic(m: &Reviewed) -> Option<(&'static str, &'static str)> {
    if m.bulk && matches!(m.category.as_str(), "promotions" | "social") {
        return Some(("junk", "Gmail marks it promotional/social and it is mass mail"));
    }
    None
}

pub fn build_prompt(batch: &[&Reviewed], today: &str) -> (String, String) {
    let system = "You are sorting one person's unread email for a cleanup. For EACH numbered message pick exactly one class:\n\
        - action: someone is waiting on the user, or there is a task, decision, appointment, deadline, or an account/security/billing problem that needs their attention.\n\
        - review: something they should look over or approve but that needs no reply (a document, draft, statement, form, invitation to accept).\n\
        - fyi: real information from a real person or service they would want to know, nothing to do.\n\
        - receipt: order, payment, shipping, booking confirmation or statement (keep, no action).\n\
        - junk: marketing, promotions, newsletters, social notifications, automated digests, mass mail.\n\
        - stale: it WAS time-sensitive or needed action, but its date or deadline has clearly passed given how old it is.\n\
        Use the age: old time-sensitive mail is stale, not action. Judge only from what is shown; if you cannot tell, pick fyi.\n\
        Reply with ONLY a JSON array, one object per message, like [{\"n\":1,\"class\":\"junk\",\"why\":\"weekly store promotion\"}]. No other text."
        .to_string();
    let mut user = format!("Today is {today}. Messages (age in days; bulk = sent as mass mail):\n");
    for (i, m) in batch.iter().enumerate() {
        let snippet: String = m.snippet.chars().take(110).collect();
        user.push_str(&format!("{}. [{} d old{}{}] from {} | subject: {} | preview: {}\n", i + 1, m.age_days, if m.bulk { ", bulk" } else { "" }, if m.category.is_empty() { String::new() } else { format!(", {}", m.category) }, mail::sender_name(&m.from), m.subject.chars().take(110).collect::<String>(), snippet));
    }
    (system, user)
}

/// (message number, class, short reason) from the model's reply; tolerant of text around the JSON array.
pub fn parse_classes(raw: &str) -> Vec<(usize, String, String)> {
    let (Some(a), Some(b)) = (raw.find('['), raw.rfind(']')) else { return Vec::new() };
    let Ok(v) = serde_json::from_str::<Vec<serde_json::Value>>(&raw[a..=b]) else { return Vec::new() };
    v.iter()
        .filter_map(|o| {
            let n = o["n"].as_u64().or_else(|| o["n"].as_str().and_then(|s| s.parse().ok()))? as usize;
            let c = o["class"].as_str()?.trim().to_lowercase();
            CLASSES.contains(&c.as_str()).then(|| (n, c, o["why"].as_str().unwrap_or("").chars().take(90).collect()))
        })
        .collect()
}

/// Estimated unread messages per class: each bucket's share of the sample, scaled to the bucket's real size.
pub fn estimates(a: &AccountReview) -> HashMap<String, u64> {
    let mut out: HashMap<String, f64> = HashMap::new();
    for (bi, b) in a.buckets.iter().enumerate() {
        let in_bucket: Vec<&Reviewed> = a.items.iter().filter(|i| i.bucket == bi).collect();
        if in_bucket.is_empty() {
            continue;
        }
        for i in &in_bucket {
            *out.entry(i.class.clone()).or_default() += b.total as f64 / in_bucket.len() as f64;
        }
    }
    out.into_iter().map(|(k, v)| (k, v.round() as u64)).collect()
}

const FREEMAIL: &[&str] = &["gmail.com", "yahoo.com", "outlook.com", "hotmail.com", "icloud.com", "aol.com", "proton.me", "protonmail.com", "live.com", "msn.com"];

pub fn domain_of(from: &str) -> Option<String> {
    let addr = from.rsplit('<').next().unwrap_or(from).trim_end_matches('>').trim();
    addr.rsplit_once('@').map(|(_, d)| d.trim().to_lowercase()).filter(|d| d.contains('.'))
}

/// Sender domains that show up repeatedly and are almost always junk: (domain, sampled count, estimated total).
pub fn junk_domains(a: &AccountReview) -> Vec<(String, usize, u64)> {
    let scale = a.unread_total as f64 / a.items.len().max(1) as f64;
    let mut by: HashMap<String, (usize, usize)> = HashMap::new();
    for i in &a.items {
        if let Some(d) = domain_of(&i.from).filter(|d| !FREEMAIL.contains(&d.as_str())) {
            let e = by.entry(d).or_default();
            e.0 += 1;
            if i.class == "junk" {
                e.1 += 1;
            }
        }
    }
    let mut v: Vec<(String, usize, u64)> = by.into_iter().filter(|(_, (n, j))| *n >= 3 && *j * 100 >= *n * 80).map(|(d, (n, _))| (d, n, (n as f64 * scale).round() as u64)).collect();
    v.sort_by(|x, y| y.1.cmp(&x.1).then(x.0.cmp(&y.0)));
    v.truncate(25);
    v
}

/// The cleanup plan for one mailbox, written for a local model to follow step by step. Contains rules drawn from
/// the review (sender domains), so it stays on this server; the generic rules live in docs/MAIL_CLEANUP_PLAYBOOK.md.
pub fn plan_text(a: &AccountReview, date: &str) -> String {
    let est = estimates(a);
    let g = |k: &str| est.get(k).copied().unwrap_or(0);
    let domains = junk_domains(a);
    let mut t = format!("# Inbox cleanup plan for {} ({date})\n\n", a.email);
    t.push_str(&format!(
        "Based on a sample of {} of {} unread messages. Estimated: {} need action, {} to review, {} are stale, {} are junk, {} are worth keeping (information or receipts).\n\n",
        a.items.len(), a.unread_total, g("action"), g("review"), g("stale"), g("junk"), g("fyi") + g("receipt") + g("unsure")
    ));
    t.push_str("You are Loki's mail-cleanup agent, running on a local model. First read the \"Mail cleanup playbook\" (docs/MAIL_CLEANUP_PLAYBOOK.md) and obey its safety rules. Then do ONLY the steps below, in order. If anything does not match what this plan expects, stop and ask the user.\n\n");
    t.push_str("## Step 0 — before touching anything\n- Run each search below as a COUNT first and write the numbers down. Do not change any message until the user has approved the plan.\n- Create the labels `Loki/Junk`, `Loki/Stale`, `Loki/Action`, `Loki/Keep` if they do not exist.\n\n");
    t.push_str("## Step 1 — mass-mail junk (reversible: label + mark read + archive, never delete)\n");
    if domains.is_empty() {
        t.push_str("No single sender stood out. Skip to Step 2.\n\n");
    } else {
        t.push_str("These sender domains were sampled at least 3 times and were at least 80% junk. For each, search `in:inbox is:unread from:<domain>`, check the count is roughly the estimate, then label `Loki/Junk`, mark read, and remove from the inbox:\n");
        for (d, n, e) in &domains {
            t.push_str(&format!("- {d}: {n} in the sample, about {e} unread in total\n"));
        }
        t.push('\n');
    }
    t.push_str("## Step 2 — old promotions and social mail\nSearch `in:inbox is:unread category:promotions older_than:30d`, then the same for `category:social`. Same action as Step 1 (label `Loki/Junk`, mark read, archive).\n\n");
    t.push_str(&format!("## Step 3 — stale mail (about {} messages)\nMessages classified \"stale\" were time-sensitive but are past their date. Do not delete them. List them for the user in groups of 25; for the ones they approve, label `Loki/Stale`, mark read, archive.\n\n", g("stale")));
    t.push_str(&format!("## Step 4 — things that need action (about {} messages)\nLeave these UNREAD and in the inbox. Add the label `Loki/Action`. Then give the user a short list: sender, subject, age, and what is being asked. Never reply, never send. If they ask for a reply, write a DRAFT only.\n\n", g("action")));
    t.push_str(&format!("## Step 4b — things to review (about {} messages)\nThese need the user's eyes but no reply (documents, drafts, statements, invitations). Leave them UNREAD in the inbox, add the label `Loki/Review`, and list them for them with one line on what to look at.\n\n", g("review")));
    t.push_str("## Step 5 — keep (information and receipts)\nLeave in place. Label `Loki/Keep` only if the user asks. Receipts older than a year can be archived after they approve.\n\n");
    t.push_str("## Step 6 — verify and report\nAfter each batch re-run the count search and compare. If a batch changed more than 20% more messages than expected, stop and tell the user. At the end, report per step: expected count, actual count, anything skipped. Keep the log of message ids you touched so every change can be undone.\n");
    t
}

fn gmail_link(email: &str, id: &str) -> String {
    format!("https://mail.google.com/mail/?authuser={email}#all/{id}")
}

pub fn status_json(r: &Review) -> serde_json::Value {
    let accounts: Vec<serde_json::Value> = r
        .accounts
        .iter()
        .map(|a| {
            let est = estimates(a);
            serde_json::json!({ "email": a.email, "unread": a.unread_total, "sampled": a.items.len(), "estimated": est })
        })
        .collect();
    serde_json::json!({ "status": r.status, "phase": r.phase, "done": r.done, "total": r.total, "started": r.started, "finished": r.finished, "error": r.error, "accounts": accounts })
}

/// Runs the whole review (sampling, then classification) and stores the result. `provider` must be local.
pub async fn run(provider: Arc<dyn LlmProvider>, shared: Shared) {
    let set = |f: &dyn Fn(&mut Review)| f(&mut shared.lock().unwrap());
    let Some(cfg) = mail::load_config() else {
        set(&|r| {
            r.status = "error".into();
            r.error = Some("mail isn't set up".into());
        });
        return;
    };
    let now_ms = crate::needs_you::now_unix() * 1000;
    let today = crate::conversation_log::date_format("%Y-%m-%d").unwrap_or_default();
    set(&|r| {
        *r = Review { status: "running".into(), phase: "Counting and sampling".into(), started: now_ms / 1000, ..Default::default() };
    });
    let mut accounts: Vec<AccountReview> = Vec::new();
    for acct in &cfg.accounts {
        set(&|r| r.phase = format!("Sampling {}", acct.email));
        let unread_total = mail::count_matching(&cfg, acct, "in:inbox is:unread").await.unwrap_or(0);
        let mut buckets = Vec::new();
        let mut items = Vec::new();
        for (bi, (label, frag, n)) in BUCKETS.iter().enumerate() {
            let q = format!("in:inbox is:unread {frag}");
            let total = mail::count_matching(&cfg, acct, &q).await.unwrap_or(0);
            let ms: Vec<MsgMeta> = if total > 0 { mail::sample(&cfg, acct, &q, *n).await } else { Vec::new() };
            buckets.push(BucketStat { label: label.to_string(), total, sampled: ms.len() });
            for m in ms {
                let mut it = Reviewed { account: m.account.clone(), id: m.id.clone(), from: m.from.clone(), subject: m.subject.clone(), age_days: ((now_ms - m.at_ms) / 86_400_000).max(0), bucket: bi, bulk: m.bulk, category: category(&m.labels), snippet: m.snippet.clone(), class: String::new(), why: String::new() };
                if let Some((c, w)) = heuristic(&it) {
                    it.class = c.into();
                    it.why = w.into();
                }
                items.push(it);
            }
        }
        accounts.push(AccountReview { email: acct.email.clone(), unread_total, buckets, items });
    }
    let total: usize = accounts.iter().flat_map(|a| a.items.iter()).filter(|i| i.class.is_empty()).count();
    set(&|r| {
        r.phase = "Reading with the local model".into();
        r.total = total;
        r.accounts = accounts.clone();
    });
    for ai in 0..accounts.len() {
        let pending: Vec<usize> = accounts[ai].items.iter().enumerate().filter(|(_, i)| i.class.is_empty()).map(|(k, _)| k).collect();
        for chunk in pending.chunks(18) {
            let batch: Vec<&Reviewed> = chunk.iter().map(|k| &accounts[ai].items[*k]).collect();
            let (system, user) = build_prompt(&batch, &today);
            let msgs = vec![Message::system(system), Message::user(user)];
            let got = match tokio::time::timeout(std::time::Duration::from_secs(150), provider.chat_low_effort(&msgs)).await {
                Ok(Ok(r)) => parse_classes(&r.content),
                _ => Vec::new(),
            };
            for (j, k) in chunk.iter().enumerate() {
                let it = &mut accounts[ai].items[*k];
                match got.iter().find(|(n, ..)| *n == j + 1) {
                    Some((_, c, w)) => {
                        it.class = c.clone();
                        it.why = w.clone();
                    }
                    None => {
                        it.class = "unsure".into();
                        it.why = "the model gave no answer for this one".into();
                    }
                }
            }
            let done = chunk.len();
            let snap = accounts.clone();
            set(&|r| {
                r.done += done;
                r.accounts = snap.clone();
            });
        }
    }
    let end = crate::needs_you::now_unix();
    set(&|r| {
        r.status = "done".into();
        r.phase = "Finished".into();
        r.finished = Some(end);
        r.accounts = accounts.clone();
    });
    save(&shared.lock().unwrap());
}

/// One line on the review page; `group` is the mailbox column it belongs to.
pub struct ReviewRow {
    pub group: String,
    pub head: String,
    pub text: String,
    pub link: Option<String>,
    pub status: &'static str,
    pub heading: bool,
}

/// Rows for the review page, one column per mailbox: summary, then what needs action, what to review, what is
/// stale, and the biggest junk senders.
pub fn rows(r: &Review, now_ms: i64) -> Vec<ReviewRow> {
    let mut out = Vec::new();
    let age = |d: i64| if d < 1 { "today".to_string() } else { format!("{d} d old") };
    for a in &r.accounts {
        let est = estimates(a);
        let g = |k: &str| est.get(k).copied().unwrap_or(0);
        let mut push = |head: String, text: String, link: Option<String>, status: &'static str, heading: bool| out.push(ReviewRow { group: a.email.clone(), head, text, link, status, heading });
        push("Summary".into(), format!("{} unread · about {} need action · {} to review · {} stale · {} junk · {} to keep (from a sample of {})", a.unread_total, g("action"), g("review"), g("stale"), g("junk"), g("fyi") + g("receipt") + g("unsure"), a.items.len()), None, "", false);
        let mut section = |title: &str, class: &str, status: &'static str, oldest_first: bool, take: usize| {
            let mut list: Vec<&Reviewed> = a.items.iter().filter(|i| i.class == class).collect();
            if list.is_empty() {
                return;
            }
            if oldest_first {
                list.sort_by_key(|i| std::cmp::Reverse(i.age_days));
            } else {
                list.sort_by_key(|i| i.age_days);
            }
            push(format!("{title} ({} in the sample)", list.len()), String::new(), None, "", true);
            for i in list.iter().take(take) {
                push(mail::sender_name(&i.from), format!("{} · {} · {}", if i.subject.is_empty() { "(no subject)" } else { &i.subject }, age(i.age_days), i.why), Some(gmail_link(&a.email, &i.id)), status, false);
            }
        };
        section("Needs action", "action", "warn", false, 12);
        section("To review", "review", "ok", false, 8);
        section("Stale", "stale", "off", true, 8);
        let domains = junk_domains(a);
        if !domains.is_empty() {
            push("Biggest junk senders".into(), String::new(), None, "", true);
            for (d, n, e) in domains.iter().take(10) {
                push(d.clone(), format!("{n} in the sample, about {e} unread in total · proposed: label, mark read, archive"), None, "off", false);
            }
        }
    }
    let _ = now_ms;
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn it(from: &str, class: &str, bucket: usize) -> Reviewed {
        Reviewed { account: "a@x".into(), id: "1".into(), from: from.into(), subject: "s".into(), age_days: 3, bucket, bulk: false, category: String::new(), snippet: String::new(), class: class.into(), why: String::new() }
    }

    #[test]
    fn obvious_mass_mail_is_junk_without_asking_the_model() {
        let mut m = it("Promo <p@shop.com>", "", 0);
        m.bulk = true;
        m.category = "promotions".into();
        assert_eq!(heuristic(&m).map(|h| h.0), Some("junk"));
        m.category = "updates".into();
        assert_eq!(heuristic(&m), None);
        m.category = "promotions".into();
        m.bulk = false;
        assert_eq!(heuristic(&m), None, "a person's mail in the promotions tab is not auto-junk");
    }

    #[test]
    fn parses_the_models_json_even_with_chatter() {
        let raw = "Sure! Here you go:\n[{\"n\":1,\"class\":\"Junk\",\"why\":\"promo\"},{\"n\":\"2\",\"class\":\"action\",\"why\":\"asks for a reply\"},{\"n\":3,\"class\":\"banana\"}]\nDone.";
        let c = parse_classes(raw);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0], (1, "junk".to_string(), "promo".to_string()));
        assert_eq!(c[1].0, 2);
        assert!(parse_classes("no json here").is_empty());
    }

    #[test]
    fn prompt_numbers_the_messages_and_carries_age() {
        let mut a = it("Bo <bo@x.com>", "", 0);
        a.subject = "Lunch?".into();
        a.age_days = 40;
        let (sys, user) = build_prompt(&[&a], "2026-10-27");
        assert!(sys.contains("ONLY a JSON array") && sys.contains("stale"));
        assert!(user.starts_with("Today is 2026-10-27") && user.contains("1. [40 d old] from Bo | subject: Lunch?"), "{user}");
    }

    #[test]
    fn estimates_scale_each_bucket_by_its_real_size() {
        let a = AccountReview {
            email: "a@x".into(),
            unread_total: 1100,
            buckets: vec![BucketStat { label: "new".into(), total: 100, sampled: 2 }, BucketStat { label: "old".into(), total: 1000, sampled: 4 }],
            items: vec![it("a", "action", 0), it("b", "junk", 0), it("c", "junk", 1), it("d", "junk", 1), it("e", "junk", 1), it("f", "stale", 1)],
        };
        let e = estimates(&a);
        assert_eq!((e["action"], e["stale"], e["junk"]), (50, 250, 800));
    }

    #[test]
    fn junk_rules_need_repeats_and_skip_personal_domains() {
        let mut items = Vec::new();
        for _ in 0..5 {
            items.push(it("Shop <deals@shop.com>", "junk", 0));
        }
        for _ in 0..4 {
            items.push(it("Pal <pal@yahoo.com>", "junk", 0));
        }
        items.push(it("Bank <a@bank.com>", "junk", 0));
        items.push(it("Bank <b@bank.com>", "fyi", 0));
        items.push(it("Bank <c@bank.com>", "fyi", 0));
        let a = AccountReview { email: "a@x".into(), unread_total: 11, buckets: vec![BucketStat { label: "x".into(), total: 11, sampled: 11 }], items };
        let d = junk_domains(&a);
        assert_eq!(d.len(), 1, "a freemail domain is a person, bank.com is mostly kept: {d:?}");
        assert_eq!((d[0].0.as_str(), d[0].1, d[0].2), ("shop.com", 5, 5));
        let plan = plan_text(&a, "2026-10-27");
        assert!(plan.contains("shop.com: 5 in the sample") && plan.contains("never delete") && plan.contains("Step 6"), "{plan}");
        assert!(plan.contains("only the steps below") || plan.contains("ONLY the steps below"));
    }
}
