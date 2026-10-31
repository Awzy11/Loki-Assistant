//! "Needs you": a short, ranked list of what deserves the user's attention now, built from the
//! vault (today's note, the week's Active items, Upcoming Events, backlog next actions) and Loki's
//! own queue (fixes waiting for review). Ranking is plain rules, computed on this server, and uses
//! his own Starting state to show fewer items on harder days (Low-Capacity Protocol). Decisions
//! (done / defer / not now, with an optional reason and a thumbs rating) are stored in
//! `~/.local/share/loki/needs_you.json` and appended to the vault's `needs-you-findings.md`.

use crate::obsidian::ObsidianClient;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize)]
pub struct Item {
    pub id: String,
    pub title: String,
    /// Why it is on the list — the source and how fresh it is, in calm words.
    pub why: String,
    pub source: String,
    pub source_path: String,
    pub kind: String,
    pub score: i32,
    pub when: Option<String>,
    pub next_action: Option<String>,
    /// Where this item leads: a URL in its line, a linked note, or its source note.
    pub link: Option<String>,
    /// One calm sentence from the local model on why this is worth attention today (filled in later).
    pub blurb: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Clock {
    pub date: String,
    pub weekday: String,
    pub iso_week: String,
    pub minutes: u32,
}

pub fn clock() -> Clock {
    let f = crate::conversation_log::date_format("%Y-%m-%d|%a|%G-W%V|%H%M").unwrap_or_default();
    let p: Vec<&str> = f.split('|').collect();
    let hm: u32 = p.get(3).and_then(|v| v.parse().ok()).unwrap_or(0);
    Clock {
        date: p.first().unwrap_or(&"1970-01-01").to_string(),
        weekday: p.get(1).unwrap_or(&"").to_string(),
        iso_week: p.get(2).unwrap_or(&"").to_string(),
        minutes: hm / 100 * 60 + hm % 100,
    }
}

// ---------- small date helpers ----------

fn ymd(date: &str) -> Option<(i64, i64, i64)> {
    let mut it = date.split('-').map(|v| v.parse::<i64>().ok());
    Some((it.next()??, it.next()??, it.next()??))
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Whole days from `from` to `to` (both `YYYY-MM-DD`); `None` if either doesn't parse.
pub fn days_between(from: &str, to: &str) -> Option<i64> {
    let (a, b) = (ymd(from)?, ymd(to)?);
    Some(days_from_civil(b.0, b.1, b.2) - days_from_civil(a.0, a.1, a.2))
}

// ---------- parsing the notes ----------

/// `key: value` lines of the leading `---` block, values as raw text (empty = unset).
pub fn frontmatter(text: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let t = text.trim_start_matches('\n');
    let Some(rest) = t.strip_prefix("---\n") else { return out };
    let Some(end) = rest.find("\n---") else { return out };
    for line in rest[..end].lines() {
        if let Some((k, v)) = line.split_once(':') {
            out.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    out
}

/// `## <title>` / `### <title>`: the heading lines the section parsers look for. The titles come from the profile.
fn h2(title: &str) -> String {
    format!("## {title}")
}

fn h3(title: &str) -> String {
    format!("### {title}")
}

fn dsec() -> &'static crate::profile::DailySections {
    &crate::profile::get().daily.sections
}

fn wsec() -> &'static crate::profile::WeekSections {
    &crate::profile::get().week.sections
}

fn wlab() -> &'static crate::profile::WeekLabels {
    &crate::profile::get().week.labels
}

/// The progress line `Active: 3/5 ... Done: 2`, found by its two labels.
fn wip_line(week: &str) -> String {
    let (a, d) = (format!("{}:", wlab().wip_active), format!("{}:", wlab().wip_done));
    week.lines().find(|l| l.contains(&a) && l.contains(&d)).map(clean_md).unwrap_or_default()
}

fn section<'a>(text: &'a str, heading: &str) -> &'a str {
    let Some(start) = text.find(heading) else { return "" };
    let rest = &text[start + heading.len()..];
    let end = rest.find("\n## ").or_else(|| rest.find("\n---")).unwrap_or(rest.len());
    &rest[..end]
}

#[derive(Debug, Clone, PartialEq)]
pub struct Task {
    pub section: String,
    pub text: String,
    pub mit: bool,
    /// The line as written (wikilinks and URLs intact), for links.
    pub raw: String,
}

/// Unchecked top-level tasks under `## Tasks`, with their `### Section` and whether they're tagged MIT.
pub fn open_tasks(text: &str) -> Vec<Task> {
    let body = section(text, &format!("\n## {}", dsec().tasks));
    let mut sec = String::new();
    let mut out = Vec::new();
    for line in body.lines() {
        if let Some(h) = line.strip_prefix("### ") {
            sec = h.trim().to_string();
        } else if let Some(t) = line.strip_prefix("- [ ] ") {
            let clean = clean_md(t);
            if clean.to_lowercase().starts_with("heads-up") || sec == dsec().routines {
                continue;
            }
            out.push(Task { section: sec.clone(), mit: t.contains("MIT"), text: clean, raw: t.to_string() });
        }
    }
    out
}

/// Today's 3 as plain text (empty slots dropped).
pub fn todays_three(text: &str) -> Vec<String> {
    section(text, &h2(&dsec().mits))
        .lines()
        .filter_map(|l| {
            let (n, rest) = l.split_once(". ")?;
            n.trim().chars().all(|c| c.is_ascii_digit()).then(|| clean_md(rest))
        })
        .filter(|s| !s.is_empty())
        .collect()
}

/// Today's 3 as written (links intact).
pub fn todays_three_raw(text: &str) -> Vec<String> {
    section(text, &h2(&dsec().mits))
        .lines()
        .filter_map(|l| {
            let (n, rest) = l.split_once(". ")?;
            n.trim().chars().all(|c| c.is_ascii_digit()).then(|| rest.trim().to_string())
        })
        .filter(|s| !s.is_empty())
        .collect()
}

/// Items of the week note's "Active this sprint" list.
pub fn active_items(text: &str) -> Vec<String> {
    let Some(start) = text.find(&format!("**{}:**", wlab().active_list)) else { return Vec::new() };
    let rest = &text[start..];
    let end = rest.find("\n###").unwrap_or(rest.len());
    rest[..end]
        .lines()
        .skip(1)
        .filter_map(|l| {
            let (n, r) = l.split_once(". ")?;
            n.trim().chars().all(|c| c.is_ascii_digit()).then(|| r.trim().to_string())
        })
        .filter(|s| !s.starts_with("*(left open"))
        .collect()
}

/// `(date, bullet text)` for the `### YYYY-MM-DD — Day` blocks of Upcoming Events.
pub fn upcoming_events(text: &str) -> Vec<(String, String)> {
    upcoming_events_raw(text).into_iter().map(|(d, t)| (d, clean_md(&t))).collect()
}

/// Same, with each bullet as written (links intact).
pub fn upcoming_events_raw(text: &str) -> Vec<(String, String)> {
    let head = Regex::new(r"^### (\d{4}-\d{2}-\d{2})").unwrap();
    let mut date: Option<String> = None;
    let mut out = Vec::new();
    for line in text.lines() {
        if let Some(c) = head.captures(line) {
            date = Some(c[1].to_string());
        } else if line.starts_with('#') {
            date = None;
        } else if let (Some(d), Some(b)) = (&date, line.strip_prefix("- ")) {
            out.push((d.clone(), b.to_string()));
        }
    }
    out
}

/// `(start_min, end_min, type, assigned)` rows of today's Focus Windows table.
pub fn focus_windows(text: &str) -> Vec<(u32, u32, String, String)> {
    focus_windows_raw(text).into_iter().map(|(a, b, t, who)| (a, b, t, clean_md(&who))).collect()
}

/// Same, with the assignment as written (links intact).
pub fn focus_windows_raw(text: &str) -> Vec<(u32, u32, String, String)> {
    let range = Regex::new(r"(\d{4})\s*[–-]\s*(\d{4})").unwrap();
    section(text, &h2(&dsec().focus_windows))
        .lines()
        .filter(|l| l.starts_with('|'))
        .filter_map(|l| {
            let cells: Vec<&str> = l.split('|').map(str::trim).collect();
            let c = range.captures(cells.get(1)?)?;
            let m = |s: &str| s.parse::<u32>().ok().map(|v| v / 100 * 60 + v % 100);
            Some((m(&c[1])?, m(&c[2])?, clean_md(cells.get(2)?), cells.get(3)?.to_string()))
        })
        .collect()
}

/// Wikilinks, bold/italic marks and extra spaces removed; the text a person reads.
pub fn clean_md(s: &str) -> String {
    let link = Regex::new(r"\[\[([^\]|]+)(?:\|([^\]]+))?\]\]").unwrap();
    let s = link.replace_all(s, |c: &regex::Captures| c.get(2).map(|m| m.as_str()).unwrap_or(&c[1]).to_string());
    let s = s.replace(['*', '`'], "");
    let s = Regex::new(r"\(MIT[^)]*\)").unwrap().replace_all(&s, "").to_string();
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn urlenc(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// `obsidian://` link that opens a vault note by path or by name (Obsidian resolves names like a wikilink).
pub fn note_url(path_or_name: &str) -> String {
    let vault = std::env::var("LOKI_OBSIDIAN_VAULT").unwrap_or_else(|_| "Obsidian Vault".to_string());
    format!("obsidian://open?vault={}&file={}", urlenc(&vault), urlenc(path_or_name.trim_end_matches(".md")))
}

/// The first useful link in a line of markdown: a `[text](https://…)` link, a bare URL, else a `[[wikilink]]` as an Obsidian link.
pub fn first_link(raw: &str) -> Option<String> {
    let md = Regex::new(r"\[[^\]]*\]\((https?://[^)\s]+)\)").unwrap();
    if let Some(c) = md.captures(raw) {
        return Some(c[1].to_string());
    }
    let bare = Regex::new(r"https?://[^\s)\]>]+").unwrap();
    if let Some(m) = bare.find(raw) {
        return Some(m.as_str().trim_end_matches(['.', ',', ';']).to_string());
    }
    let wiki = Regex::new(r"\[\[([^\]|#]+)").unwrap();
    wiki.captures(raw).map(|c| note_url(c[1].trim()))
}

fn wikilinks(s: &str) -> Vec<String> {
    Regex::new(r"\[\[([^\]|]+)").unwrap().captures_iter(s).map(|c| c[1].trim().to_string()).collect()
}

fn words(s: &str) -> std::collections::HashSet<String> {
    s.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| w.len() > 3).map(str::to_string).collect()
}

/// Two titles about the same thing (half of the shorter one's words are shared).
pub fn similar(a: &str, b: &str) -> bool {
    let (wa, wb) = (words(a), words(b));
    let n = wa.len().min(wb.len());
    n > 0 && wa.intersection(&wb).count() * 2 >= n
}

fn fnv(s: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

/// Titles stay one readable line: cut at a word boundary near 110 characters.
fn shorten(title: &str) -> String {
    if title.chars().count() <= 110 {
        return title.to_string();
    }
    let cut: String = title.chars().take(110).collect();
    let cut = cut.rsplit_once(' ').map(|(a, _)| a.to_string()).unwrap_or(cut);
    format!("{}…", cut.trim_end_matches([':', ',', ' ', '·', '—', '-']))
}

fn item(kind: &str, title: &str, why: String, source: &str, path: &str, score: i32) -> Item {
    let title = &shorten(title);
    Item {
        id: fnv(&format!("{path}|{title}")),
        title: title.to_string(),
        why,
        source: source.to_string(),
        source_path: path.to_string(),
        kind: kind.to_string(),
        score,
        when: None,
        next_action: None,
        link: if path.ends_with(".md") { Some(note_url(path)) } else { None },
        blurb: None,
    }
}

// ---------- learning from past decisions ----------

/// Nudges scores from what the user did with the same (or a similar) item before: set aside three or
/// more times in three weeks → lower, with a gentle note; a recent 👎 → lower; a recent 👍 → higher.
pub fn adjust(items: &mut [Item], decisions: &[Decision], today: &str) {
    for it in items.iter_mut() {
        let related: Vec<&Decision> = decisions.iter().filter(|d| d.id == it.id || similar(&d.title, &it.title)).collect();
        let mut days_set_aside: Vec<&str> = related
            .iter()
            .filter(|d| matches!(d.action.as_str(), "defer" | "not_now") && days_between(&d.on, today).map(|n| (0..=21).contains(&n)).unwrap_or(false))
            .map(|d| d.on.as_str())
            .collect();
        days_set_aside.sort();
        days_set_aside.dedup();
        if days_set_aside.len() >= 3 {
            it.score -= 15;
            it.why = format!("{} — set aside {} times in the last three weeks, so still worth wanting?", it.why, days_set_aside.len());
        }
        let latest = related.iter().filter(|d| !d.rating.is_empty() && days_between(&d.on, today).map(|n| (0..=14).contains(&n)).unwrap_or(false)).max_by_key(|d| d.on.clone());
        match latest.map(|d| d.rating.as_str()) {
            Some("down") => it.score -= 25,
            Some("up") => it.score += 10,
            _ => {}
        }
    }
}

// ---------- briefings: passive cards (today's shape, the week, news, signals, leads) ----------

#[derive(Debug, Clone, Serialize)]
pub struct Row {
    pub head: String,
    pub text: String,
    pub link: Option<String>,
    /// "ok" | "fail" | "off" | "warn" — drawn as a coloured dot before the heading.
    pub status: Option<String>,
    /// Pages that compare things (the two mailboxes) show one column per group.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// A sub-heading inside a column rather than an item.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub heading: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Briefing {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub blurb: String,
    pub pill: String,
    pub link: Option<String>,
    pub source_path: String,
    /// Opens the source note in Obsidian, when the card comes from one.
    pub note: Option<String>,
    pub rows: Vec<Row>,
    /// Numeric series for the page's sparklines (Rhythm); empty for other cards.
    pub series: Vec<Series>,
    /// Structured data for the tab's dashboard (windows, tasks, capacity, …); the text rows stay for the card.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Series {
    pub name: String,
    pub unit: String,
    /// Oldest first; `None` where the day had no value.
    pub points: Vec<Option<f64>>,
}

pub(crate) fn row(head: &str, text: &str, link: Option<String>) -> Row {
    Row { head: head.to_string(), text: text.to_string(), link, status: None, group: None, heading: false }
}

pub(crate) fn row_l(head: &str, text: &str, link: Option<String>) -> Row {
    Row { head: head.to_string(), text: text.to_string(), link, status: None, group: None, heading: false }
}

pub(crate) fn row_s(head: &str, text: &str, status: &str) -> Row {
    Row { head: head.to_string(), text: text.to_string(), link: None, status: Some(status.to_string()), group: None, heading: false }
}

pub(crate) fn briefing(kind: &str, title: &str, blurb: &str, pill: &str, link: Option<&str>, path: &str, rows: Vec<Row>) -> Briefing {
    Briefing { id: fnv(&format!("brief|{kind}|{path}")), kind: kind.into(), title: title.into(), blurb: blurb.into(), pill: pill.into(), link: link.map(String::from), source_path: path.into(), note: if path.ends_with(".md") { Some(note_url(path)) } else { None }, rows, series: Vec::new(), data: None }
}

/// First sentence (up to ~`max` characters) of a paragraph, markdown removed.
pub fn first_sentence(text: &str, max: usize) -> String {
    let t = clean_md(text);
    let cut = t.find(". ").map(|i| t[..=i].chars().count()).unwrap_or_else(|| t.chars().count());
    let s: String = t.chars().take(cut.min(max)).collect();
    if s.chars().count() < t.chars().count() && cut > max { format!("{}…", s.trim_end()) } else { s }
}

/// Today's Focus Windows as a timeline, with the current block marked.
pub fn shape_briefing(day: &str, now_min: u32, path: &str) -> Option<Briefing> {
    let raw = focus_windows_raw(day);
    let w: Vec<(u32, u32, String, String)> = raw.iter().map(|(a, b, t, who)| (*a, *b, t.clone(), clean_md(who))).collect();
    if w.is_empty() {
        return None;
    }
    let cur = w.iter().position(|(a, b, ..)| now_min >= *a && now_min < *b);
    let nxt = w.iter().position(|(a, ..)| *a > now_min);
    let blurb = match (cur, nxt) {
        (Some(i), _) => format!("Now: {}", w[i].3),
        (None, Some(i)) => format!("Next at {}: {}", fmt_min(w[i].0), w[i].3),
        _ => "Today's blocks are done".to_string(),
    };
    let rows = w
        .iter()
        .enumerate()
        .map(|(i, (a, b, t, who))| row_l(&format!("{}–{}{}", fmt_min(*a), fmt_min(*b), if Some(i) == cur { "  ◀ now" } else { "" }), &format!("{t}: {who}"), first_link(&raw[i].3)))
        .collect();
    Some(briefing("shape", "Today's shape", &blurb, &format!("{} blocks", w.len()), None, path, rows))
}

/// Every task under `## Tasks` (open and done), with its section, MIT number and links.
pub struct TaskFull {
    pub section: String,
    pub text: String,
    pub done: bool,
    pub mit: Option<u8>,
    pub link: Option<String>,
}

pub fn all_tasks(text: &str) -> Vec<TaskFull> {
    let mit = Regex::new(r"MIT\s*(\d)").unwrap();
    let mut sec = String::new();
    let mut out = Vec::new();
    for line in section(text, &format!("\n## {}", dsec().tasks)).lines() {
        if let Some(h) = line.strip_prefix("### ") {
            sec = h.trim().to_string();
            continue;
        }
        let t = line.trim_start();
        let (done, rest) = if let Some(r) = t.strip_prefix("- [ ] ") { (false, r) } else if let Some(r) = t.strip_prefix("- [x] ").or_else(|| t.strip_prefix("- [X] ")) { (true, r) } else { continue };
        let clean = clean_md(rest);
        if clean.is_empty() || clean.to_lowercase().starts_with("heads-up") {
            continue;
        }
        out.push(TaskFull { section: sec.clone(), done, mit: mit.captures(rest).and_then(|c| c[1].parse().ok()), link: first_link(rest), text: clean });
    }
    out
}

/// Today's dashboard: the day's blocks with the current one marked, the three MITs with their state, and the tasks by section.
pub fn shape_data(day: &str, now_min: u32) -> Value {
    let windows: Vec<Value> = focus_windows_raw(day)
        .iter()
        .map(|(a, b, t, who)| json!({ "start": a, "end": b, "type": t, "what": clean_md(who), "link": first_link(who), "current": now_min >= *a && now_min < *b }))
        .collect();
    let all = all_tasks(day);
    // the /bod and /eod to-dos are shown on their own, not counted among the day's tasks
    let routines: Vec<Value> = all.iter().filter(|t| t.section == dsec().routines).map(|t| json!({ "cmd": t.text.split_whitespace().next().unwrap_or(""), "text": t.text, "done": t.done })).collect();
    let tasks: Vec<&TaskFull> = all.iter().filter(|t| t.section != dsec().routines).collect();
    let mits: Vec<Value> = todays_three(day)
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let n = (i + 1) as u8;
            let linked: Vec<&&TaskFull> = tasks.iter().filter(|x| x.mit == Some(n)).collect();
            let state = if linked.is_empty() { "unknown" } else if linked.iter().all(|x| x.done) { "done" } else { "open" };
            json!({ "n": n, "text": t, "state": state })
        })
        .collect();
    let mut sections: Vec<Value> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    for t in &tasks {
        if !names.contains(&t.section) {
            names.push(t.section.clone());
        }
    }
    for name in names {
        let items: Vec<Value> = tasks.iter().filter(|t| t.section == name).map(|t| json!({ "text": t.text, "done": t.done, "mit": t.mit, "link": t.link })).collect();
        let done = items.iter().filter(|i| i["done"] == true).count();
        sections.push(json!({ "name": name, "done": done, "total": items.len(), "items": items }));
    }
    let savers: Vec<Value> = section(day, &h2(&dsec().savers)).lines().filter_map(|l| { let t = l.trim_start(); let (done, r) = if let Some(r) = t.strip_prefix("- [ ] ") { (false, r) } else { (true, t.strip_prefix("- [x] ").or_else(|| t.strip_prefix("- [X] "))?) }; Some(json!({ "text": clean_md(r), "done": done })) }).collect();
    let weather: Vec<Value> = section(day, &h2(&dsec().weather))
        .lines()
        .filter(|l| l.starts_with('|') && !l.contains("---") && !l.contains("| Time "))
        .filter_map(|l| {
            let c: Vec<String> = l.split('|').map(|x| clean_md(x)).collect();
            (c.len() >= 6 && !c[1].is_empty()).then(|| json!({ "slot": c[1].split('(').next().unwrap_or("").trim(), "cond": c[2], "temp": c[3], "humidity": c[4], "rain": c[5] }))
        })
        .collect();
    let fm = frontmatter(day);
    json!({
        "now_min": now_min, "windows": windows, "mits": mits, "mits_title": dsec().mits, "routines_title": dsec().routines, "routine_cmds": crate::profile::get().routine_todos("day").into_iter().map(|(c, _)| c).collect::<Vec<_>>(), "sections": sections, "routines": routines, "savers": savers, "weather": weather,
        "state": energy_value(&fm), "done": tasks.iter().filter(|t| t.done).count(), "total": tasks.len(),
    })
}

/// Markdown cleaned and cut to `max` characters with an ellipsis, never at a sentence boundary.
fn clip(s: &str, max: usize) -> String {
    let t = clean_md(s);
    if t.chars().count() <= max { t } else { format!("{}…", t.chars().take(max).collect::<String>().trim_end()) }
}

fn first_number(s: &str) -> Option<f64> {
    Regex::new(r"(\d+(?:\.\d+)?)").unwrap().captures(s).and_then(|c| c[1].parse().ok())
}

/// The sprint dashboard: goal, priorities, capacity, WIP, domain progress, active items, events and what's done.
pub fn week_data(week: &str, today: &str, iso_week: &str) -> Value {
    let cell = |label: &str| -> String { week.lines().find(|l| l.starts_with('|') && l.contains(label)).and_then(|l| l.split('|').nth(2)).map(clean_md).unwrap_or_default() };
    let (avail, committed, buffer) = (cell(&wlab().available), cell(&wlab().committed), cell(&wlab().buffer));
    let pct = Regex::new(r"\((\d+)%\)").unwrap().captures(&buffer).and_then(|c| c[1].parse::<f64>().ok());
    let wip_line = wip_line(week);
    let wip = Regex::new(&format!(r"{}:\s*(\d+)/(\d+).*{}:\s*(\d+)", regex::escape(&wlab().wip_active), regex::escape(&wlab().wip_done))).unwrap().captures(&wip_line).map(|c| (c[1].parse::<u32>().unwrap_or(0), c[2].parse::<u32>().unwrap_or(5), c[3].parse::<u32>().unwrap_or(0)));
    let goal_sec = section(week, &h2(&wsec().goal));
    let goal = goal_sec.lines().map(clean_md).find(|l| !l.is_empty()).unwrap_or_default();
    let priorities: Vec<String> = goal_sec.lines().filter_map(|l| { let (n, t) = l.trim_start().split_once(". ")?; n.chars().all(|c| c.is_ascii_digit()).then(|| clip(t, 260)) }).collect();
    let days_left = frontmatter(week).get("dates").and_then(|d| d.split('/').nth(1).map(|e| e.trim().to_string())).and_then(|end| days_between(today, &end)).map(|d| d + 1);
    let active: Vec<Value> = active_items(week).iter().map(|a| json!({ "text": clip(&a.replace("~~", ""), 200), "done": a.contains("~~"), "link": first_link(a) })).collect();
    // domains: the ### sections under Sprint Backlog that hold checkboxes (not standing commitments, events, queue or done)
    let backlog = section(week, &h2(&wsec().backlog));
    let mut domains: Vec<Value> = Vec::new();
    let (mut name, mut items): (String, Vec<Value>) = (String::new(), Vec::new());
    let mut flush = |name: &str, items: &mut Vec<Value>, domains: &mut Vec<Value>| {
        let w = wsec();
        let skip = w.skip_groups.iter().chain([&w.events, &w.routines]).any(|s| name.starts_with(s.as_str())) || name.contains(w.queue.as_str()) || name.contains(w.done.as_str());
        if !name.is_empty() && !skip && !items.is_empty() {
            let done = items.iter().filter(|i| i["done"] == true).count();
            domains.push(json!({ "name": name, "done": done, "total": items.len(), "items": items.clone() }));
        }
        items.clear();
    };
    for line in backlog.lines() {
        if let Some(h) = line.strip_prefix("### ") {
            flush(&name, &mut items, &mut domains);
            name = h.trim().to_string();
            continue;
        }
        let t = line.trim_start();
        let (done, rest) = if let Some(r) = t.strip_prefix("- [ ] ") { (false, r) } else if let Some(r) = t.strip_prefix("- [x] ").or_else(|| t.strip_prefix("- [X] ")) { (true, r) } else { continue };
        items.push(json!({ "text": clip(rest, 200), "done": done, "link": first_link(rest) }));
    }
    flush(&name, &mut items, &mut domains);
    let ev_re = Regex::new(r"^- \*\*([^*]+)\*\*\s*[—-]\s*(.*)$").unwrap();
    let events: Vec<Value> = section(&backlog, &h3(&wsec().events)).lines().filter_map(|l| ev_re.captures(l.trim_start()).map(|c| json!({ "day": c[1].trim(), "text": clip(&c[2], 320) }))).collect();
    let queue: usize = section(&backlog, &h3(&wsec().queue)).lines().filter(|l| l.trim_start().starts_with("- [ ] ")).map(|l| l.split(" · ").count()).sum();
    let done_items: Vec<String> = section(&backlog, &h3(&wsec().done)).lines().filter(|l| l.trim_start().starts_with("- ")).map(|l| clip(l.trim_start().trim_start_matches("- "), 200)).collect();
    let routines: Vec<Value> = section(&backlog, &h3(&wsec().routines)).lines().take_while(|l| !l.starts_with('#')).filter_map(|l| { let t = l.trim_start(); let (done, r) = if let Some(r) = t.strip_prefix("- [ ] ") { (false, r) } else { (true, t.strip_prefix("- [x] ").or_else(|| t.strip_prefix("- [X] "))?) }; Some(json!({ "cmd": r.split_whitespace().next().unwrap_or(""), "text": clean_md(r), "done": done })) }).collect();
    json!({
        "routines": routines, "routines_title": wsec().routines, "routine_cmds": crate::profile::get().routine_todos("week").into_iter().map(|(c, _)| c).collect::<Vec<_>>(), "week": iso_week, "days_left": days_left, "goal": first_sentence(&goal, 220), "priorities": priorities,
        "capacity": { "available": first_number(&avail), "committed": first_number(&committed), "buffer": first_number(&buffer), "buffer_pct": pct },
        "wip": wip.map(|(a, m, d)| json!({ "active": a, "max": m, "done": d })),
        "active": active, "domains": domains, "events": events, "queue": queue, "done": done_items,
    })
}

/// Countdowns dashboard: everything dated in the next 60 days, with a rough topic.
pub fn countdowns_data(events: &str, today: &str) -> Value {
    let topic = |t: &str| {
        let l = t.to_lowercase();
        if ["csc", "class", "lecture", "jrap", "6100", "assignment", "brief"].iter().any(|k| l.contains(k)) { "school" }
        else if ["deka", "run", "mcm", "race", "cft", "workout", "spartan", "hyrox"].iter().any(|k| l.contains(k)) { "fitness" }
        else if ["bruno", "house", "bowie", "family", "halloween", "kids", "sami"].iter().any(|k| l.contains(k)) { "home" }
        else { "other" }
    };
    let mut list: Vec<(i64, String, String, Option<String>)> = upcoming_events_raw(events).into_iter().filter_map(|(d, t)| days_between(today, &d).filter(|n| (0..=60).contains(n)).map(|n| (n, d, t.clone(), first_link(&t)))).collect();
    list.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    let items: Vec<Value> = list.iter().map(|(n, d, t, l)| json!({ "date": d, "days": n, "text": clip(t, 260), "topic": topic(t), "link": l })).collect();
    json!({ "today": today, "events": items })
}

/// The week note's goal, top 3, capacity and Active items.
pub fn week_briefing(week: &str, today: &str, iso_week: &str, path: &str) -> Option<Briefing> {
    if week.trim().is_empty() {
        return None;
    }
    let cell = |label: &str| -> String {
        week.lines().find(|l| l.starts_with('|') && l.contains(label)).and_then(|l| l.split('|').nth(2)).map(|c| clean_md(c)).unwrap_or_default()
    };
    let (avail, committed, buffer) = (cell(&wlab().available), cell(&wlab().committed), cell(&wlab().buffer));
    let active_line = wip_line(week);
    let active = Regex::new(&format!(r"{}:\s*(\d+/\d+)", regex::escape(&wlab().wip_active))).unwrap().captures(&active_line).map(|c| c[1].to_string()).unwrap_or_default();
    let left = frontmatter(week).get("dates").and_then(|d| d.split('/').nth(1).map(|e| e.trim().to_string())).and_then(|end| days_between(today, &end));
    let mut bits: Vec<String> = Vec::new();
    if !active.is_empty() {
        bits.push(format!("{active} active"));
    }
    if !buffer.is_empty() {
        bits.push(format!("buffer {buffer}"));
    }
    if let Some(d) = left {
        bits.push(format!("{} day{} left", d + 1, if d == 0 { "" } else { "s" }));
    }
    let mut rows = Vec::new();
    let goal = section(week, &h2(&wsec().goal));
    if let Some(g) = goal.lines().map(clean_md).find(|l| !l.is_empty()) {
        rows.push(row("Goal", &first_sentence(&g, 200), None));
    }
    for l in goal.lines().filter(|l| l.trim_start().chars().next().is_some_and(|c| c.is_ascii_digit()) && l.contains(". ")) {
        if let Some((n, t)) = l.trim_start().split_once(". ") {
            rows.push(row_l(&format!("Priority {n}"), &first_sentence(t, 160), first_link(t)));
        }
    }
    if !avail.is_empty() {
        rows.push(row("Capacity", &format!("{avail} available · {committed} committed · {buffer} buffer"), None));
    }
    for (i, a) in active_items(week).iter().enumerate() {
        rows.push(row_l(&format!("Active {}", i + 1), &first_sentence(a, 140), first_link(a)));
    }
    Some(briefing("week", &format!("Week {}", iso_week.split('-').nth(1).unwrap_or(iso_week)), &bits.join(" · "), "This week", None, path, rows))
}

/// The news digest's per-category summaries (BLUF), one row each.
pub fn news_briefing(text: &str, date: &str, path: &str) -> Option<Briefing> {
    let bluf = section(text, "## BLUF");
    let head = Regex::new(r"^\*\*([^*:]+):\*\*\s*(.+)$").unwrap();
    let rows: Vec<Row> = bluf.split("\n\n").filter_map(|p| head.captures(p.trim()).map(|c| row_l(c[1].trim(), &first_sentence(&c[2], 230), Some(note_url(path))))).collect();
    if rows.is_empty() {
        return None;
    }
    let blurb = rows[0].text.clone();
    let n = rows.len();
    Some(briefing("news", "News digest", &blurb, &format!("{n} categories · {date}"), crate::profile::get().link("news").as_deref(), path, rows))
}

/// The digest's "Signals" (data-backed watch items), one row each with its source link.
pub fn signals_briefing(text: &str, path: &str) -> Option<Briefing> {
    let sig = section(text, "## Signals");
    let src = Regex::new(r"^[^·]+·\s*\d{4}-\d{2}-\d{2}\s*·\s*(https?://\S+)").unwrap();
    let mut rows = Vec::new();
    for block in sig.split("\n### ").skip(1) {
        let mut lines = block.lines();
        let title = clean_md(lines.next().unwrap_or(""));
        let rest: Vec<&str> = lines.collect();
        let link = rest.iter().find_map(|l| src.captures(l).map(|c| c[1].to_string()));
        let body = rest.iter().filter(|l| !src.is_match(l)).copied().collect::<Vec<_>>().join(" ");
        if !title.is_empty() {
            rows.push(row(&title, &first_sentence(&body, 220), link));
        }
    }
    if rows.is_empty() {
        return None;
    }
    let blurb = rows[0].head.clone();
    let n = rows.len();
    let host = Regex::new(r"^https?://(?:www\.)?([^/]+)").unwrap();
    let items: Vec<Value> = rows.iter().map(|r| json!({ "title": r.head, "text": r.text, "link": r.link, "domain": r.link.as_deref().and_then(|l| host.captures(l)).map(|c| c[1].to_string()).unwrap_or_default() })).collect();
    let mut b = briefing("signals", "Signals to watch", &blurb, &format!("{n} signals"), None, path, rows);
    b.data = Some(json!({ "items": items }));
    Some(b)
}

/// Today's lead scan: the ranked hire-me posts, plus the pipeline counts when the dashboard answered.
pub fn leads_briefing(text: &str, path: &str, stats: Option<&Value>) -> Option<Briefing> {
    let counts = Regex::new(r"Hire-me:\s*(\d+)\s*·\s*Fix-it:\s*(\d+)").unwrap().captures(text);
    let (hire, fix) = counts.map(|c| (c[1].to_string(), c[2].to_string())).unwrap_or(("0".into(), "0".into()));
    let url = Regex::new(r"(https?://\S+)").unwrap();
    let mut rows = Vec::new();
    for block in section(text, "## Hire-me (ranked)").split("\n### ").skip(1) {
        let mut lines = block.lines();
        let title = clean_md(lines.next().unwrap_or(""));
        let title = title.split_once(". ").map(|(_, t)| t.to_string()).unwrap_or(title);
        let meta = lines.next().unwrap_or("");
        let link = url.captures(meta).map(|c| c[1].to_string());
        let detail = meta.split(" · ").skip(1).collect::<Vec<_>>().join(" · ");
        rows.push(row(&title, &clean_md(&detail), link));
    }
    if let Some(st) = stats {
        let by = &st["by_status"];
        let g = |k: &str| by[k].as_u64().unwrap_or(0);
        rows.push(row("Pipeline", &format!("{} interested · {} replied · {} new", g("interested"), g("replied"), g("new")), crate::profile::get().link("leads")));
    }
    if rows.is_empty() && hire == "0" {
        return None;
    }
    let blurb = rows.first().filter(|r| r.head != "Pipeline").map(|r| r.head.clone()).unwrap_or_else(|| format!("{fix} fix-it posts today"));
    let items: Vec<Value> = rows.iter().filter(|r| r.head != "Pipeline").map(|r| json!({ "title": r.head, "detail": r.text, "link": r.link })).collect();
    let mut b = briefing("leads", &format!("Leads — {hire} hire-me, {fix} fix-it"), &blurb, "Today's scan", crate::profile::get().link("leads").as_deref(), path, rows);
    b.data = Some(json!({ "hire": hire.parse::<u32>().unwrap_or(0), "fix": fix.parse::<u32>().unwrap_or(0), "items": items, "stats": stats.cloned() }));
    Some(b)
}


// ---------- more briefings: countdowns, rhythm, saved, network, Loki status ----------

/// The next dated events (beyond today, within 45 days) as "in N days" rows.
pub fn countdowns_briefing(events: &str, today: &str, path: &str) -> Option<Briefing> {
    let mut upcoming: Vec<(i64, String, String)> = upcoming_events_raw(events)
        .into_iter()
        .filter_map(|(d, t)| days_between(today, &d).filter(|n| (1..=45).contains(n)).map(|n| (n, d, t)))
        .collect();
    upcoming.sort_by(|a, b| a.0.cmp(&b.0));
    upcoming.truncate(8);
    if upcoming.is_empty() {
        return None;
    }
    let when = |n: i64| if n == 1 { "tomorrow".to_string() } else { format!("in {n} days") };
    let rows: Vec<Row> = upcoming.iter().map(|(n, d, t)| row_l(&format!("{} · {d}", when(*n)), &first_sentence(t, 150), first_link(t))).collect();
    let blurb = format!("{} — {}", first_sentence(&upcoming[0].2, 70), when(upcoming[0].0));
    Some(briefing("countdowns", "Countdowns", &blurb, &format!("{} ahead", upcoming.len()), None, path, rows))
}

/// The tracked properties of one daily note (see `[[daily.properties]]` in the profile): key to the raw value, empty ones left out.
#[derive(Debug, Clone, Default)]
pub struct DayProps {
    pub date: String,
    pub values: std::collections::BTreeMap<String, String>,
}

impl DayProps {
    pub fn num(&self, key: &str) -> Option<f64> {
        self.values.get(key).and_then(|v| v.trim().parse::<f64>().ok())
    }

    pub fn text(&self, key: &str) -> &str {
        self.values.get(key).map(String::as_str).unwrap_or("")
    }

    #[cfg(test)]
    pub fn with(date: &str, pairs: &[(&str, &str)]) -> DayProps {
        DayProps { date: date.into(), values: pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect() }
    }
}

pub fn day_props(date: &str, note: &str) -> DayProps {
    let fm = frontmatter(note);
    let mut values = std::collections::BTreeMap::new();
    for p in &crate::profile::get().daily.properties {
        if let Some(v) = fm.get(&p.key).map(|v| v.trim()).filter(|v| !v.is_empty() && !matches!(v.to_lowercase().as_str(), "null" | "~")) {
            values.insert(p.key.clone(), v.to_string());
        }
    }
    DayProps { date: date.to_string(), values }
}

/// The value of the energy property in a note's frontmatter (empty when you don't track one).
fn energy_value(fm: &HashMap<String, String>) -> String {
    crate::profile::get().energy().and_then(|p| fm.get(&p.key).cloned()).unwrap_or_default()
}

fn avg(v: &[f64]) -> Option<f64> {
    (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64)
}

/// The last two weeks of the tracked daily-note properties as sparklines and a few plain rows, built from the profile's property list.
pub fn rhythm_briefing(days: &[DayProps]) -> Option<Briefing> {
    let props = &crate::profile::get().daily.properties;
    if props.is_empty() || days.iter().all(|d| d.values.is_empty()) {
        return None;
    }
    let mut series = Vec::new();
    let mut rows = Vec::new();
    let mut bits = Vec::new();
    for p in props {
        match p.kind.as_str() {
            "number" => {
                let points: Vec<Option<f64>> = days.iter().map(|d| d.num(&p.key)).collect();
                let vals: Vec<f64> = points.iter().flatten().copied().collect();
                series.push(Series { name: p.label.clone(), unit: p.unit.clone(), points });
                let unit = if p.unit.is_empty() { String::new() } else { format!(" {}", p.unit) };
                match p.summary.as_str() {
                    "average" => {
                        if let Some(a) = avg(&vals) {
                            rows.push(row(&p.label, &format!("{a:.1}{unit} average over {} days", vals.len()), None));
                            bits.push(format!("{} {a:.1}{unit}", p.label.to_lowercase()));
                        }
                    }
                    "trend" => {
                        if vals.len() >= 2 {
                            rows.push(row(&p.label, &format!("{:.1} → {:.1}{unit}", vals[0], vals[vals.len() - 1]), None));
                            bits.push(format!("{} {:.1} → {:.1}", p.label.to_lowercase(), vals[0], vals[vals.len() - 1]));
                        }
                    }
                    "average_nonzero" => {
                        let nz: Vec<f64> = vals.iter().copied().filter(|v| *v > 0.0).collect();
                        if let Some(a) = avg(&nz) {
                            rows.push(row(&p.label, &format!("{a:.1} a day on days that had any"), None));
                        }
                    }
                    _ => {}
                }
            }
            "choice" => {
                let known = days.iter().filter(|d| !d.text(&p.key).is_empty()).count();
                if known == 0 {
                    continue;
                }
                let count = |o: &str| days.iter().filter(|d| d.text(&p.key).eq_ignore_ascii_case(o)).count();
                rows.push(row(&p.label, &p.options.iter().map(|o| format!("{} {o}", count(o))).collect::<Vec<_>>().join(" · "), None));
                if p.role == "energy" {
                    if let Some(o) = p.gentle.first().or(p.low.first()) {
                        bits.push(format!("{} of {known} starts {o}", count(o)));
                    }
                }
            }
            _ => {}
        }
    }
    let mut b = briefing("rhythm", "Rhythm", &bits.join(" · "), &format!("{} days", days.len()), None, &format!("{} (properties)", crate::profile::get().vault.daily), rows);
    b.series = series;
    let day_json: Vec<Value> = days
        .iter()
        .map(|d| {
            let vals: serde_json::Map<String, Value> = props
                .iter()
                .map(|p| (p.key.clone(), if p.kind == "number" { d.num(&p.key).map(Value::from).unwrap_or(Value::Null) } else if d.text(&p.key).is_empty() { Value::Null } else { Value::from(d.text(&p.key)) }))
                .collect();
            json!({ "date": d.date, "values": vals })
        })
        .collect();
    b.data = Some(json!({ "props": props, "days": day_json }));
    Some(b)
}

/// Reads the last `n` days' notes (today's folder, then the weekly archive) and returns their properties.
async fn recent_days(obs: &ObsidianClient, today: &str, n: i64) -> Vec<DayProps> {
    let mut out = Vec::new();
    for back in (0..n).rev() {
        let date = add_days(today, -back);
        let mut note = obs.read_file(&crate::profile::get().daily_path(&date)).await.ok();
        if note.is_none() {
            if let Some(week) = crate::conversation_log::date_format_for(&date, "%G-W%V") {
                note = obs.read_file(&crate::profile::get().archived_daily_path(&week, &date)).await.ok();
            }
        }
        if let Some(text) = note {
            out.push(day_props(&date, &text));
        } else {
            out.push(DayProps { date, values: Default::default() });
        }
    }
    out
}

// ---------- mail review page ----------

/// The inbox-review results as a briefing page (rows come from `mail_review::rows`). Sensitive: only served to the user's own UI.
pub fn mail_review_briefing(r: &crate::mail_review::Review, now_ms: i64) -> Briefing {
    let rows: Vec<Row> = crate::mail_review::rows(r, now_ms)
        .into_iter()
        .map(|x| {
            let mut out = row_l(&x.head, &x.text, x.link);
            if !x.status.is_empty() {
                out.status = Some(x.status.to_string());
            }
            out.group = Some(x.group);
            out.heading = x.heading;
            out
        })
        .collect();
    let blurb = match r.status.as_str() {
        "running" => format!("{} ({}/{})", r.phase, r.done, r.total),
        "done" => format!("{} mailbox{} reviewed", r.accounts.len(), if r.accounts.len() == 1 { "" } else { "es" }),
        "error" => r.error.clone().unwrap_or_else(|| "the review failed".into()),
        _ => "Not run yet".to_string(),
    };
    briefing("mailreview", "Inbox review", &blurb, &r.status, Some("https://mail.google.com/"), "mail review (local)", rows)
}

// ---------- mail ----------

/// The Mail page: one column per mailbox, each with a summary and the newest unread mail grouped as
/// needs action / to review / FYI, as sorted by the local model. The card face has counts only.
pub fn mail_briefing(data: &[(String, Option<u64>, Vec<crate::mail::MsgMeta>)], triage: &HashMap<String, String>, now_ms: i64) -> Option<Briefing> {
    if data.is_empty() {
        return None;
    }
    let class_of = |m: &crate::mail::MsgMeta| -> Option<&str> {
        let h = crate::mail_review::Reviewed { account: m.account.clone(), id: m.id.clone(), from: m.from.clone(), subject: m.subject.clone(), age_days: 0, bucket: 0, bulk: m.bulk, category: m.labels.iter().find_map(|l| l.strip_prefix("CATEGORY_")).map(|c| c.to_lowercase()).unwrap_or_default(), snippet: String::new(), class: String::new(), why: String::new() };
        if crate::mail_review::heuristic(&h).is_some() {
            return Some("junk");
        }
        triage.get(&format!("{}:{}", m.account, m.id)).map(String::as_str)
    };
    let age = |at_ms: i64| {
        let s = ((now_ms - at_ms) / 1000).max(0);
        if s < 5400 { format!("{} min", (s / 60).max(1)) } else if s < 172_800 { format!("{} h", (s + 1800) / 3600) } else { format!("{} d", (s + 43_200) / 86_400) }
    };
    let mut rows = Vec::new();
    let (mut unread_total, mut action_total) = (0u64, 0usize);
    for (email, count, msgs) in data {
        unread_total += count.unwrap_or(0);
        let mut newest: Vec<&crate::mail::MsgMeta> = msgs.iter().collect();
        newest.sort_by_key(|m| std::cmp::Reverse(m.at_ms));
        let by = |c: &str| newest.iter().copied().filter(|m| class_of(m) == Some(c)).collect::<Vec<_>>();
        let (action, review, fyi) = (by("action"), by("review"), by("fyi"));
        let hidden = newest.iter().filter(|m| matches!(class_of(m), Some("junk") | Some("receipt") | Some("stale"))).count();
        let pending: Vec<_> = newest.iter().copied().filter(|m| class_of(m).is_none() || class_of(m) == Some("unsure")).collect();
        action_total += action.len();
        let put = |rows: &mut Vec<Row>, mut r: Row| {
            r.group = Some(email.clone());
            rows.push(r);
        };
        let mut h = row("Summary", &format!("{} unread in the inbox · newest {} sorted: {} need action · {} to review · {} FYI · {} receipts and junk hidden{}", count.map(|c| c.to_string()).unwrap_or_else(|| "?".into()), newest.len(), action.len(), review.len(), fyi.len(), hidden, if pending.is_empty() { String::new() } else { format!(" · {} still being read", pending.len()) }), None);
        h.status = None;
        put(&mut rows, h);
        let section = |rows: &mut Vec<Row>, title: &str, list: &[&crate::mail::MsgMeta], status: &str| {
            if list.is_empty() {
                return;
            }
            let mut head = row(&format!("{title} ({})", list.len()), "", None);
            head.heading = true;
            put(rows, head);
            for m in list.iter().take(12) {
                let subject = if m.subject.is_empty() { "(no subject)" } else { m.subject.as_str() };
                let mut r = row_l(&crate::mail::sender_name(&m.from), &format!("{subject} · {} ago", age(m.at_ms)), Some(format!("https://mail.google.com/mail/?authuser={}#all/{}", m.account, m.id)));
                if !status.is_empty() {
                    r.status = Some(status.to_string());
                }
                put(rows, r);
            }
        };
        section(&mut rows, "Needs action", &action, "warn");
        section(&mut rows, "To review", &review, "ok");
        section(&mut rows, "FYI", &fyi, "");
        section(&mut rows, "Still being read", &pending, "");
    }
    let blurb = if action_total > 0 { format!("{unread_total} unread · {action_total} need action") } else { format!("{unread_total} unread") };
    Some(briefing("mail", "Mail", &blurb, &format!("{} account{}", data.len(), if data.len() == 1 { "" } else { "s" }), Some("https://mail.google.com/"), "gmail", rows))
}

// ---------- n8n workflows, Zotero, weather ----------

/// MCP text results come back as `{"result": "..."}` or as the bare text; give the text.
pub fn unwrap_result(text: &str) -> String {
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Object(m)) => m.get("result").and_then(|v| v.as_str()).map(String::from).unwrap_or_else(|| text.to_string()),
        _ => text.to_string(),
    }
}

/// Seconds since the epoch for `2026-10-26T04:00:25.054Z`.
pub fn parse_iso_unix(t: &str) -> Option<i64> {
    let (d, rest) = t.split_once('T')?;
    let (y, m, day) = ymd(d)?;
    let mut hms = rest.trim_end_matches('Z').split(':');
    let h: i64 = hms.next()?.parse().ok()?;
    let mi: i64 = hms.next()?.parse().ok()?;
    let sec: i64 = hms.next()?.split('.').next()?.parse().ok()?;
    Some(days_from_civil(y, m, day) * 86400 + h * 3600 + mi * 60 + sec)
}

fn ago(secs: i64) -> String {
    let s = secs.max(0);
    if s < 5400 {
        format!("{} min ago", (s / 60).max(1))
    } else if s < 172_800 {
        format!("{} h ago", (s + 1800) / 3600)
    } else {
        format!("{} d ago", (s + 43_200) / 86400)
    }
}

fn until(secs: i64) -> String {
    if secs <= 0 {
        "due now".to_string()
    } else if secs < 5400 {
        format!("in {} min", (secs / 60).max(1))
    } else if secs < 172_800 {
        format!("in {} h", (secs + 1800) / 3600)
    } else {
        format!("in {} d", (secs + 43_200) / 86400)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Workflow {
    pub id: String,
    pub name: String,
    pub active: bool,
}

/// `- [id] name (active)` lines from n8n's list_workflows.
pub fn parse_n8n_workflows(text: &str) -> Vec<Workflow> {
    let re = Regex::new(r"^- \[([^\]]+)\] (.+) \((active|inactive)\)\s*$").unwrap();
    text.lines().filter_map(|l| re.captures(l.trim_end()).map(|c| Workflow { id: c[1].to_string(), name: c[2].to_string(), active: &c[3] == "active" })).collect()
}

#[derive(Debug, Clone, PartialEq)]
pub struct Exec {
    pub workflow: String,
    pub status: String,
    pub at: i64,
}

/// `- [execId] workflowId | status | ISO time` lines from list_executions (newest first).
pub fn parse_n8n_executions(text: &str) -> Vec<Exec> {
    let re = Regex::new(r"^- \[[^\]]+\] (\S+) \| (\S+) \| (\S+)\s*$").unwrap();
    text.lines().filter_map(|l| re.captures(l).and_then(|c| parse_iso_unix(&c[3]).map(|at| Exec { workflow: c[1].to_string(), status: c[2].to_string(), at }))).collect()
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Schedule {
    pub text: String,
    /// Fixed repeat interval, when the trigger has one.
    pub every_secs: Option<i64>,
}

/// The schedule trigger of a workflow's JSON (interval or cron), in words.
pub fn parse_schedule(workflow_json: &str) -> Option<Schedule> {
    let wf: Value = serde_json::from_str(workflow_json).ok()?;
    let node = wf["nodes"].as_array()?.iter().find(|n| n["type"].as_str().map(|t| t.contains("scheduleTrigger") || t.ends_with(".cron")).unwrap_or(false))?;
    let iv = node["parameters"]["rule"]["interval"].as_array()?.first()?;
    let field = iv["field"].as_str().unwrap_or("");
    let n = |k: &str| iv[k].as_i64().unwrap_or(1).max(1);
    Some(match field {
        "hours" => Schedule { text: format!("every {} h", n("hoursInterval")), every_secs: Some(n("hoursInterval") * 3600) },
        "minutes" => Schedule { text: format!("every {} min", n("minutesInterval")), every_secs: Some(n("minutesInterval") * 60) },
        "seconds" => Schedule { text: format!("every {} s", n("secondsInterval")), every_secs: Some(n("secondsInterval")) },
        "days" => Schedule { text: format!("every {} day{}", n("daysInterval"), if n("daysInterval") == 1 { "" } else { "s" }), every_secs: Some(n("daysInterval") * 86400) },
        "weeks" => Schedule { text: format!("every {} week{}", n("weeksInterval"), if n("weeksInterval") == 1 { "" } else { "s" }), every_secs: Some(n("weeksInterval") * 7 * 86400) },
        "cronExpression" => Schedule { text: format!("cron {}", iv["expression"].as_str().unwrap_or("?")), every_secs: None },
        other => Schedule { text: format!("every {other}"), every_secs: None },
    })
}

/// The workflows page: every workflow with its schedule, last run and next run, newest problems first.
pub fn n8n_briefing(workflows: &[Workflow], execs: &[Exec], schedules: &HashMap<String, Schedule>, now: i64) -> Option<Briefing> {
    if workflows.is_empty() {
        return None;
    }
    let last = |id: &str| execs.iter().find(|e| e.workflow == id);
    let ok = |s: &str| s == "success";
    let mut active: Vec<&Workflow> = workflows.iter().filter(|w| w.active).collect();
    active.sort_by_key(|w| (last(&w.id).map(|e| ok(&e.status)).unwrap_or(true), w.name.clone()));
    let inactive: Vec<&Workflow> = workflows.iter().filter(|w| !w.active).collect();
    let mut rows = Vec::new();
    let mut failing = 0;
    for w in &active {
        let sch = schedules.get(&w.id);
        let mut bits: Vec<String> = vec!["active".into()];
        if let Some(sc) = sch {
            bits.push(sc.text.clone());
        }
        let mut status = "warn";
        match last(&w.id) {
            Some(e) => {
                bits.push(format!("last run {} — {}", ago(now - e.at), e.status));
                status = if ok(&e.status) { "ok" } else { "fail" };
                if !ok(&e.status) {
                    failing += 1;
                }
                if let Some(every) = sch.and_then(|s| s.every_secs) {
                    bits.push(format!("next {}", until(e.at + every - now)));
                }
            }
            None => bits.push("no runs recorded yet".into()),
        }
        let runs = execs.iter().filter(|e| e.workflow == w.id).count();
        if runs > 1 {
            bits.push(format!("{runs} recent runs"));
        }
        let mut r = row_s(&w.name, &bits.join(" · "), status);
        r.link = crate::profile::get().workflow_link(&w.id);
        rows.push(r);
    }
    for w in &inactive {
        let mut r = row_s(&w.name, "inactive", "off");
        r.link = crate::profile::get().workflow_link(&w.id);
        rows.push(r);
    }
    // runs per day, last 7 days (from the recent executions n8n returned)
    let today_idx = now.div_euclid(86400);
    let points: Vec<Option<f64>> = (0..7).rev().map(|back| Some(execs.iter().filter(|e| e.at.div_euclid(86400) == today_idx - back).count() as f64)).collect();
    let recent_failed = execs.iter().filter(|e| !ok(&e.status) && now - e.at < 86400 * 3).count();
    let blurb = if failing > 0 {
        format!("{failing} failing · {} active of {}", active.len(), workflows.len())
    } else if let Some(e) = active.iter().filter_map(|w| last(&w.id)).max_by_key(|e| e.at) {
        format!("{} active of {} · last run {} ok", active.len(), workflows.len(), ago(now - e.at))
    } else {
        format!("{} active of {}", active.len(), workflows.len())
    };
    let mut b = briefing("workflows", "Workflows", &blurb, if failing > 0 || recent_failed > 0 { "check" } else { "n8n" }, crate::profile::get().link("workflows").as_deref(), "n8n", rows);
    b.series = vec![Series { name: "Runs a day".into(), unit: "".into(), points }];
    let name_of = |id: &str| workflows.iter().find(|w| w.id == id).map(|w| w.name.clone()).unwrap_or_else(|| id.to_string());
    let wf_json: Vec<Value> = workflows
        .iter()
        .map(|w| {
            let e = last(&w.id);
            let sch = schedules.get(&w.id);
            json!({
                "id": w.id, "name": w.name, "active": w.active, "link": crate::profile::get().workflow_link(&w.id),
                "schedule": sch.map(|s| s.text.clone()),
                "last_status": e.map(|e| e.status.clone()), "last_ago": e.map(|e| ago(now - e.at)), "last_at": e.map(|e| e.at),
                "next": e.and_then(|e| sch.and_then(|s| s.every_secs).map(|ev| until(e.at + ev - now))),
                "runs": execs.iter().filter(|x| x.workflow == w.id).count(),
            })
        })
        .collect();
    let by_day: Vec<Value> = (0..7).rev().map(|back| {
        let day: Vec<&Exec> = execs.iter().filter(|e| e.at.div_euclid(86400) == today_idx - back).collect();
        json!({ "back": back, "ok": day.iter().filter(|e| ok(&e.status)).count(), "failed": day.iter().filter(|e| !ok(&e.status)).count() })
    }).collect();
    let recent: Vec<Value> = execs.iter().take(15).map(|e| json!({ "workflow": name_of(&e.workflow), "status": e.status, "ago": ago(now - e.at) })).collect();
    b.data = Some(json!({ "workflows": wf_json, "by_day": by_day, "recent": recent, "failing": failing }));
    Some(b)
}

#[derive(Debug, Clone, PartialEq)]
pub struct ZItem {
    pub title: String,
    pub kind: String,
    pub authors: String,
    pub added: String,
    pub key: String,
}

/// Items of Zotero's "Most Recently Added" markdown.
pub fn parse_zotero_recent(text: &str) -> Vec<ZItem> {
    let mut out = Vec::new();
    for block in text.split("\n## ").skip(1) {
        let mut lines = block.lines();
        let title = lines.next().unwrap_or("").split_once(". ").map(|(_, t)| t.trim().to_string()).unwrap_or_default();
        let field = |name: &str| block.lines().find_map(|l| l.strip_prefix(&format!("**{name}:** "))).unwrap_or("").trim().to_string();
        if !title.is_empty() {
            out.push(ZItem { title, kind: field("Type"), authors: field("Authors"), added: field("Added"), key: field("Item Key") });
        }
    }
    out
}

pub fn zotero_briefing(items: &[ZItem], now: i64) -> Option<Briefing> {
    // standalone attachments and notes aren't worth a card row
    let shown: Vec<&ZItem> = items.iter().filter(|i| !matches!(i.kind.as_str(), "attachment" | "note")).take(8).collect();
    if shown.is_empty() {
        return None;
    }
    let week = shown.iter().filter(|i| parse_iso_unix(&i.added).map(|t| now - t < 7 * 86400).unwrap_or(false)).count();
    let rows: Vec<Row> = shown
        .iter()
        .map(|i| {
            let when = parse_iso_unix(&i.added).map(|t| ago(now - t)).unwrap_or_default();
            let who = if i.authors.starts_with("No authors") { String::new() } else { format!(" · {}", i.authors) };
            let link = if i.title.starts_with("http") { Some(i.title.clone()) } else if !i.key.is_empty() { Some(format!("zotero://select/library/items/{}", i.key)) } else { None };
            row_l(&first_sentence(&i.title, 140), &format!("{}{} · added {}", i.kind, who, when), link)
        })
        .collect();
    let blurb = format!("{week} added this week · {}", first_sentence(&shown[0].title, 70));
    let zitems: Vec<Value> = shown.iter().map(|i| {
        let at = parse_iso_unix(&i.added);
        let link = if i.title.starts_with("http") { Some(i.title.clone()) } else if !i.key.is_empty() { Some(format!("zotero://select/library/items/{}", i.key)) } else { None };
        json!({ "title": clip(&i.title, 160), "kind": i.kind, "authors": if i.authors.starts_with("No authors") { String::new() } else { i.authors.clone() }, "added": at.map(|t| ago(now - t)), "added_at": at, "link": link })
    }).collect();
    let mut b = briefing("zotero", "Zotero", &blurb, &format!("{} recent", shown.len()), crate::profile::get().link("zotero").as_deref(), "zotero", rows);
    b.data = Some(json!({ "items": zitems, "week": week }));
    Some(b)
}

fn weather_text(code: i64) -> &'static str {
    match code {
        0 => "Clear",
        1 => "Mostly clear",
        2 => "Partly cloudy",
        3 => "Overcast",
        45 | 48 => "Fog",
        51 | 53 | 55 => "Drizzle",
        56 | 57 | 66 | 67 => "Freezing rain",
        61 => "Light rain",
        63 => "Rain",
        65 => "Heavy rain",
        71 | 73 | 75 | 77 => "Snow",
        80 | 81 | 82 => "Rain showers",
        85 | 86 => "Snow showers",
        95 => "Thunderstorm",
        96 | 99 => "Thunderstorm with hail",
        _ => "Mixed",
    }
}

/// Open-Meteo forecast JSON (imperial units) as the Weather card.
pub fn weather_briefing(w: &Value, place: &str) -> Option<Briefing> {
    let cur = &w["current"];
    let temp = cur["temperature_2m"].as_f64()?;
    let d = &w["daily"];
    let day_f = |k: &str, i: usize| d[k][i].as_f64();
    let hi = day_f("temperature_2m_max", 0);
    let lo = day_f("temperature_2m_min", 0);
    let rain = d["precipitation_probability_max"][0].as_i64().unwrap_or(0);
    let cond = weather_text(cur["weather_code"].as_i64().unwrap_or(-1));
    let r0 = |v: f64| format!("{}°", v.round() as i64);
    let mut rows = vec![row("Now", &format!("{} · {cond} · feels like {} · wind {} mph", r0(temp), r0(cur["apparent_temperature"].as_f64().unwrap_or(temp)), cur["wind_speed_10m"].as_f64().unwrap_or(0.0).round()), None)];
    if let (Some(h), Some(l)) = (hi, lo) {
        rows.push(row("Today", &format!("high {} / low {} · {rain}% chance of rain", r0(h), r0(l)), None));
    }
    // 9am / 12pm / 5pm / 8pm today, like the daily note
    let times = w["hourly"]["time"].as_array();
    if let (Some(times), Some(today)) = (times, w["daily"]["time"][0].as_str()) {
        for (hour, label) in [(9, "9 am"), (12, "12 pm"), (17, "5 pm"), (20, "8 pm")] {
            let key = format!("{today}T{hour:02}:00");
            if let Some(i) = times.iter().position(|t| t.as_str() == Some(key.as_str())) {
                let t = w["hourly"]["temperature_2m"][i].as_f64().map(|v| r0(v)).unwrap_or_default();
                let p = w["hourly"]["precipitation_probability"][i].as_i64().unwrap_or(0);
                let c = weather_text(w["hourly"]["weather_code"][i].as_i64().unwrap_or(-1));
                rows.push(row(label, &format!("{t} · {c} · {p}% rain"), None));
            }
        }
    }
    let clock = |k: &str| d[k][0].as_str().and_then(|t| t.split('T').nth(1)).unwrap_or("").to_string();
    if !clock("sunrise").is_empty() {
        rows.push(row("Sun", &format!("rises {} · sets {}", clock("sunrise"), clock("sunset")), None));
    }
    for i in 1..4 {
        if let (Some(day), Some(h), Some(l)) = (d["time"][i].as_str(), day_f("temperature_2m_max", i), day_f("temperature_2m_min", i)) {
            let c = weather_text(d["weather_code"][i].as_i64().unwrap_or(-1));
            let p = d["precipitation_probability_max"][i].as_i64().unwrap_or(0);
            rows.push(row(day, &format!("{c} · {} / {} · {p}% rain", r0(h), r0(l)), None));
        }
    }
    let blurb = format!("{} {cond}{}", r0(temp), match (hi, lo) { (Some(h), Some(l)) => format!(" · high {} low {}", r0(h), r0(l)), _ => String::new() });
    // dashboard data: the next 36 hours from the current hour, and the days
    let now_t = cur["time"].as_str().unwrap_or("");
    let start = w["hourly"]["time"].as_array().and_then(|a| a.iter().position(|t| t.as_str().is_some_and(|s| s >= now_t))).unwrap_or(0);
    let hourly: Vec<Value> = (start..(start + 36).min(w["hourly"]["time"].as_array().map_or(0, |a| a.len())))
        .map(|i| json!({ "time": w["hourly"]["time"][i], "temp": w["hourly"]["temperature_2m"][i], "pop": w["hourly"]["precipitation_probability"][i], "cond": weather_text(w["hourly"]["weather_code"][i].as_i64().unwrap_or(-1)) }))
        .collect();
    let daily: Vec<Value> = (0..d["time"].as_array().map_or(0, |a| a.len()))
        .map(|i| json!({ "date": d["time"][i], "hi": d["temperature_2m_max"][i], "lo": d["temperature_2m_min"][i], "pop": d["precipitation_probability_max"][i], "cond": weather_text(d["weather_code"][i].as_i64().unwrap_or(-1)), "sunrise": d["sunrise"][i], "sunset": d["sunset"][i] }))
        .collect();
    let mut b = briefing("weather", "Weather", &blurb, place, Some("https://open-meteo.com/"), "open-meteo.com", rows);
    b.data = Some(json!({ "now": { "temp": temp, "feels": cur["apparent_temperature"], "wind": cur["wind_speed_10m"], "cond": cond }, "hourly": hourly, "daily": daily, "place": place }));
    Some(b)
}

fn weather_cache() -> &'static std::sync::Mutex<Option<(std::time::Instant, Value)>> {
    static C: std::sync::OnceLock<std::sync::Mutex<Option<(std::time::Instant, Value)>>> = std::sync::OnceLock::new();
    C.get_or_init(|| std::sync::Mutex::new(None))
}

/// A first-run card for a fresh install: what to do next to make Loki yours. Shown until the profile names the user and has routines.
pub fn start_here_briefing(p: &crate::profile::Profile) -> Option<Briefing> {
    let no_routines = p.routines.is_empty();
    let no_name = p.person.name.trim().is_empty();
    if !no_routines && !no_name {
        return None;
    }
    let mut rows: Vec<Row> = Vec::new();
    if no_routines {
        rows.push(row("1 · Pick a starter pack", "Run ops/install-pack.sh scrum-zettelkasten for a ready-made daily and weekly system with 11 routines (/bod, /eod, /bow...), or skip it and keep the blank vault.", None));
    }
    if no_name {
        rows.push(row("2 · Tell Loki who you are", "Add your name, a line about you and your location under [person] and [location] in ~/.config/loki/profile.toml, then restart loki-server. It shapes mail sorting, replies in your voice and the weather card.", None));
    }
    rows.push(row("3 · Say hello", "Open Chat and ask: \"What can you do with my notes?\"", None));
    rows.push(row("4 · Look around", "Settings holds your models, tools and notifications; docs/PROFILE.md explains every profile field.", None));
    Some(briefing("start", "Start here", "A few quick steps to make Loki yours", "New", None, "", rows))
}

/// Open-Meteo (keyless) for the profile's location; cached 15 minutes. Only the place's coordinates leave this machine. None when no coordinates are set.
pub async fn fetch_weather() -> Option<Value> {
    let loc = &crate::profile::get().location;
    let (lat, lon) = (loc.latitude?, loc.longitude?);
    if let Some((at, v)) = weather_cache().lock().unwrap().as_ref() {
        if at.elapsed() < std::time::Duration::from_secs(900) {
            return Some(v.clone());
        }
    }
    let url = format!("https://api.open-meteo.com/v1/forecast?latitude={lat}&longitude={lon}&current=temperature_2m,apparent_temperature,weather_code,wind_speed_10m&hourly=temperature_2m,precipitation_probability,weather_code&daily=weather_code,temperature_2m_max,temperature_2m_min,precipitation_probability_max,sunrise,sunset&temperature_unit=fahrenheit&wind_speed_unit=mph&timezone=auto&forecast_days=4");
    let c = reqwest::Client::builder().timeout(std::time::Duration::from_secs(6)).build().ok()?;
    let v: Value = c.get(url).send().await.ok()?.json().await.ok()?;
    *weather_cache().lock().unwrap() = Some((std::time::Instant::now(), v.clone()));
    Some(v)
}

pub fn now_unix() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

// ---------- day pages, backlog and finance ----------

/// One calendar day as a page: Today's 3, focus windows, open tasks, that day's events and state.
pub fn day_briefing(date: &str, label: &str, note: Option<&str>, events: &str, path: &str) -> Briefing {
    let mut rows: Vec<Row> = Vec::new();
    let mut bits: Vec<String> = Vec::new();
    for (i, t) in note.map(todays_three_raw).unwrap_or_default().iter().enumerate() {
        rows.push(row_l(&format!("Today's 3 · {}", i + 1), &clean_md(t), first_link(t)));
    }
    let wins = note.map(focus_windows_raw).unwrap_or_default();
    for (a, b, t, who) in &wins {
        rows.push(row_l(&format!("{}–{}", fmt_min(*a), fmt_min(*b)), &format!("{t}: {}", clean_md(who)), first_link(who)));
    }
    if !wins.is_empty() {
        bits.push(format!("{} blocks", wins.len()));
    }
    let open = note.map(open_tasks).unwrap_or_default();
    for t in open.iter().take(10) {
        rows.push(row_l(&format!("Open · {}", t.section), &t.text, first_link(&t.raw)));
    }
    if !open.is_empty() {
        bits.push(format!("{} open", open.len()));
    }
    let done = note.map(|n| n.lines().filter(|l| l.starts_with("- [x] ") || l.starts_with("- [X] ")).count()).unwrap_or(0);
    if done > 0 {
        rows.push(row("Done", &format!("{done} task{} checked off", if done == 1 { "" } else { "s" }), None));
        bits.push(format!("{done} done"));
    }
    for (d, text) in upcoming_events_raw(events).into_iter().filter(|(d, _)| d == date) {
        let _ = d;
        rows.push(row_l("Event", &clean_md(&text), first_link(&text)));
        bits.push("event".into());
    }
    if let Some(n) = note {
        let fm = frontmatter(n);
        let mut state: Vec<String> = Vec::new();
        // compact properties: the words (choices, text) first, then the measurements
        let compact: Vec<&crate::profile::Property> = crate::profile::get().daily.properties.iter().filter(|p| p.compact).collect();
        for p in compact.iter().filter(|p| p.kind != "number").chain(compact.iter().filter(|p| p.kind == "number")) {
            if let Some(v) = fm.get(&p.key).map(|v| v.trim()).filter(|v| !v.is_empty()) {
                let name = if p.short.is_empty() { p.label.to_lowercase() } else { p.short.clone() };
                state.push(if p.kind == "number" && !p.unit.is_empty() { format!("{name} {v} {}", p.unit) } else { format!("{name} {v}") });
            }
        }
        if !state.is_empty() {
            rows.push(row("State", &state.join(" · "), None));
        }
    }
    if note.is_none() {
        rows.push(row("No note yet", "There is no daily note for this day.", None));
    }
    bits.dedup();
    let blurb = if bits.is_empty() { "Nothing planned yet".to_string() } else { bits.join(" · ") };
    briefing("day", label, &blurb, date, None, path, rows)
}

/// Reads the day's note (daily folder, then the weekly archive) and builds its page.
pub async fn build_day(obs: &ObsidianClient, date: &str) -> Briefing {
    let mut path = crate::profile::get().daily_path(date);
    let mut note = obs.read_file(&path).await.ok();
    if note.is_none() {
        if let Some(week) = crate::conversation_log::date_format_for(date, "%G-W%V") {
            let alt = crate::profile::get().archived_daily_path(&week, date);
            if let Ok(t) = obs.read_file(&alt).await {
                path = alt;
                note = Some(t);
            }
        }
    }
    let events = obs.read_file(&crate::profile::get().events_path()).await.unwrap_or_default();
    let label = crate::conversation_log::date_format_for(date, "%A, %B %-d").unwrap_or_else(|| date.to_string());
    let mut b = day_briefing(date, &label, note.as_deref(), &events, &path);
    // the Day tab gets the same dashboard as Today; only the current day has a "now" marker
    if let Some(n) = note.as_deref() {
        let c = clock();
        b.kind = "shape".into();
        b.data = Some(shape_data(n, if c.date == date { c.minutes } else { 0 }));
    }
    b
}

/// Backlog overview: item counts per area, plus the sprint's Queue from the week note.
pub fn backlog_briefing(counts: &[(String, usize)], week: &str, path: &str) -> Option<Briefing> {
    let total: usize = counts.iter().map(|(_, n)| n).sum();
    if total == 0 {
        return None;
    }
    let mut rows: Vec<Row> = counts.iter().map(|(a, n)| row(a, &format!("{n} item{}", if *n == 1 { "" } else { "s" }), None)).collect();
    let queue: Vec<(String, Option<String>)> = section(week, &h3(&wsec().queue))
        .lines()
        .filter(|l| l.trim_start().starts_with("- [ ] "))
        .map(|l| l.trim_start().trim_start_matches("- [ ] ").to_string())
        .map(|raw| (first_sentence(&raw, 110), first_link(&raw)))
        .filter(|(t, _)| !t.is_empty())
        .collect();
    for (q, link) in queue.iter().take(8) {
        rows.push(row_l("Queue", q, link.clone()));
    }
    let blurb = format!("{total} items{}", if queue.is_empty() { String::new() } else { format!(" · {} queued for the sprint", queue.len()) });
    Some(briefing("backlog", "Backlog", &blurb, &format!("{total} items"), None, path, rows))
}

/// From a vault search, the first note under the project/backlog/knowledge folders whose file name shares a
/// real word with `text` — so a plain "ATLAS 6100A/B timeline" can link to its project note without guessing wildly.
pub fn pick_note(results: &[(String, f64)], text: &str) -> Option<String> {
    let want = words(text);
    results
        .iter()
        .map(|(p, _)| p)
        .filter(|p| crate::profile::get().linkable_prefixes().iter().any(|d| p.starts_with(d.as_str())) && p.ends_with(".md") && !p.ends_with("AGENTS.md"))
        .find(|p| !words(p.rsplit('/').next().unwrap_or(p)).is_disjoint(&want))
        .cloned()
}

fn link_cache() -> &'static Mutex<HashMap<String, Option<String>>> {
    static C: std::sync::OnceLock<Mutex<HashMap<String, Option<String>>>> = std::sync::OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Gives rows that have no link of their own the note their text is about (cached by text).
async fn link_rows_by_search(obs: &ObsidianClient, rows: &mut [Row], only_heads: &[&str]) {
    for r in rows.iter_mut().filter(|r| r.link.is_none() && only_heads.iter().any(|h| r.head.starts_with(h))) {
        let key = r.text.clone();
        let cached = link_cache().lock().unwrap().get(&key).cloned();
        let found = match cached {
            Some(v) => v,
            None => {
                let query: String = words(&r.text).into_iter().take(5).collect::<Vec<_>>().join(" ");
                let v = if query.is_empty() { None } else { obs.simple_search(&query, 40).await.ok().and_then(|res| pick_note(&res, &r.text)).map(|p| note_url(&p)) };
                link_cache().lock().unwrap().insert(key, v.clone());
                v
            }
        };
        r.link = found;
    }
}

/// Newest `YYYY-MM-DD<suffix>.md` file in a vault folder.
async fn latest_dated(obs: &ObsidianClient, folder: &str, suffix: &str) -> Option<String> {
    let re = Regex::new(&format!(r"^(\d{{4}}-\d{{2}}-\d{{2}}){}\.md$", regex::escape(suffix))).ok()?;
    let mut names: Vec<String> = obs.list_files(Some(folder)).await.ok()?.into_iter().filter(|f| re.is_match(f)).collect();
    names.sort();
    names.pop().map(|n| format!("{folder}/{n}"))
}

pub fn leads_url() -> String {
    std::env::var("LOKI_LEADS_URL").unwrap_or_else(|_| "http://127.0.0.1:8030".to_string())
}

async fn leads_stats() -> Option<Value> {
    let c = reqwest::Client::builder().timeout(std::time::Duration::from_secs(4)).build().ok()?;
    c.get(format!("{}/api/stats", leads_url())).send().await.ok()?.json().await.ok()
}

/// Forget the cached backlog list (after an edit, so the dashboard shows it).
pub fn forget_backlog() {
    *backlog_cache().lock().unwrap() = None;
}

fn backlog_cache() -> &'static Mutex<Option<(std::time::Instant, Value)>> {
    static C: std::sync::OnceLock<Mutex<Option<(std::time::Instant, Value)>>> = std::sync::OnceLock::new();
    C.get_or_init(|| Mutex::new(None))
}

/// The Backlog dashboard: every backlog item with its estimate, next action and unlock, and whether the
/// sprint has it Active or Queued. Reads each item's frontmatter (8 at a time), cached 5 minutes.
pub async fn backlog_data(obs: &ObsidianClient, week: &str) -> Value {
    use futures_util::StreamExt;
    if let Some((at, v)) = backlog_cache().lock().unwrap().as_ref() {
        if at.elapsed() < std::time::Duration::from_secs(300) {
            return v.clone();
        }
    }
    let link = Regex::new(r"\[\[([^\]|#]+)").unwrap();
    let names = |s: &str| -> Vec<String> { link.captures_iter(s).map(|c| c[1].trim().to_lowercase()).collect() };
    let active: Vec<String> = active_items(week).iter().flat_map(|a| names(a)).collect();
    let queued: Vec<String> = names(&section(week, &h3(&wsec().queue)));
    let mut files: Vec<(String, String)> = Vec::new();
    for (area, dir) in crate::profile::get().backlog_dirs() {
        if let Ok(list) = obs.list_files(Some(&dir)).await {
            files.extend(list.into_iter().filter(|f| f.ends_with(".md")).map(|f| (area.clone(), f)));
        }
    }
    let items: Vec<Value> = futures_util::stream::iter(files)
        .map(|(area, f)| async move {
            let dir = crate::profile::get().backlog_dirs().into_iter().find(|(a, _)| *a == area).map(|(_, d)| d).unwrap_or_default();
            let path = format!("{dir}/{}", f.trim_start_matches('/'));
            let text = obs.read_file(&path).await.unwrap_or_default();
            let fm = frontmatter(&text);
            let get = |k: &str| fm.get(k).map(|v| clean_md(v)).filter(|v| !matches!(v.to_lowercase().as_str(), "null" | "~" | "none")).unwrap_or_default();
            let name = f.trim_end_matches(".md").to_string();
            (area, name, path, get("estimate"), get("next_action"), get("unlock"), get("status"))
        })
        .buffer_unordered(8)
        .map(|(area, name, path, estimate, next_action, unlock, status)| {
            let key = name.to_lowercase();
            let sprint = if active.contains(&key) { "active" } else if queued.contains(&key) { "queue" } else { "" };
            json!({ "area": area, "name": name, "estimate": estimate, "next_action": next_action, "unlock": unlock, "status": status, "sprint": sprint, "path": path, "link": note_url(&path) })
        })
        .collect()
        .await;
    let mut items = items;
    items.sort_by(|a, b| {
        let rank = |v: &Value| match v["sprint"].as_str() { Some("active") => 0, Some("queue") => 1, _ => 2 };
        rank(a).cmp(&rank(b)).then(a["area"].as_str().cmp(&b["area"].as_str())).then(a["name"].as_str().cmp(&b["name"].as_str()))
    });
    let v = json!({ "items": items });
    *backlog_cache().lock().unwrap() = Some((std::time::Instant::now(), v.clone()));
    v
}

/// All briefing cards that have data today, in display order.
pub async fn build_briefings(obs: &ObsidianClient, now: &Clock, day: &str, week: &str, day_path: &str, week_path: &str) -> Vec<Briefing> {
    let mut out = Vec::new();
    let mut shape_card = shape_briefing(day, now.minutes, day_path);
    if let Some(b) = shape_card.as_mut() {
        b.data = Some(shape_data(day, now.minutes));
    }
    out.extend(shape_card);
    let mut week_card = week_briefing(week, &now.date, &now.iso_week, week_path);
    if let Some(b) = week_card.as_mut() {
        b.data = Some(week_data(week, &now.date, &now.iso_week));
        link_rows_by_search(obs, &mut b.rows, &["Active", "Priority"]).await;
    }
    out.extend(week_card);
    let events_file = crate::profile::get().events_path();
    let events_text = obs.read_file(&events_file).await.unwrap_or_default();
    let mut count_card = countdowns_briefing(&events_text, &now.date, &events_file);
    if let Some(b) = count_card.as_mut() {
        b.data = Some(countdowns_data(&events_text, &now.date));
        link_rows_by_search(obs, &mut b.rows, &["in ", "tomorrow"]).await;
    }
    out.extend(count_card);
    let mut counts = Vec::new();
    for (area, dir) in crate::profile::get().backlog_dirs() {
        if let Ok(files) = obs.list_files(Some(&dir)).await {
            counts.push((area, files.iter().filter(|f| f.ends_with(".md")).count()));
        }
    }
    let mut backlog_card = backlog_briefing(&counts, week, week_path);
    if let Some(b) = backlog_card.as_mut() {
        b.data = Some(backlog_data(obs, week).await);
        link_rows_by_search(obs, &mut b.rows, &["Queue"]).await;
    }
    out.extend(backlog_card);
    out.extend(rhythm_briefing(&recent_days(obs, &now.date, 14).await));
    if let Some(path) = latest_dated(obs, &crate::profile::get().news_dir(), "").await {
        if let Ok(text) = obs.read_file(&path).await {
            let date = path.rsplit('/').next().unwrap_or("").trim_end_matches(".md").to_string();
            out.extend(news_briefing(&text, &date, &path));
            out.extend(signals_briefing(&text, &path));
        }
    }
    if let Some(path) = latest_dated(obs, &crate::profile::get().leads_dir(), " Leads").await {
        if let Ok(text) = obs.read_file(&path).await {
            out.extend(leads_briefing(&text, &path, leads_stats().await.as_ref()));
        }
    }
    out.extend(crate::homelab::briefings().await);
    out
}

// ---------- ranking ----------

/// How many items to show and the lowest score worth showing, from the energy property (Starting state in the starter pack).
pub fn capacity(starting: &str) -> (&'static str, usize, i32) {
    match crate::profile::get().energy_mode(starting) {
        "low" => ("low", 3, 70),
        "gentle" => ("gentle", 5, 40),
        _ => ("normal", 8, 0),
    }
}

/// Drops near-duplicates (keeps the higher score), sorts, and applies the capacity limit.
pub fn rank(mut items: Vec<Item>, starting: &str) -> (Vec<Item>, usize, &'static str) {
    items.sort_by(|a, b| b.score.cmp(&a.score).then(a.title.cmp(&b.title)));
    let mut kept: Vec<Item> = Vec::new();
    for it in items {
        if !kept.iter().any(|k| similar(&k.title, &it.title)) {
            kept.push(it);
        }
    }
    let (mode, max, floor) = capacity(starting);
    let total = kept.len();
    kept.retain(|i| i.score >= floor);
    kept.truncate(max);
    let hidden = total - kept.len();
    (kept, hidden, mode)
}

// ---------- decisions ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Decision {
    pub id: String,
    pub title: String,
    pub action: String, // done | defer | not_now
    pub until: Option<String>,
    pub reason: String,
    pub rating: String, // up | down | ""
    pub on: String,     // date it was recorded
}

fn store_path() -> Option<std::path::PathBuf> {
    dirs::data_dir().map(|d| d.join("loki").join("needs_you.json"))
}

pub fn load_decisions() -> Vec<Decision> {
    store_path().and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn save_decisions(list: &[Decision]) -> Result<(), String> {
    let p = store_path().ok_or("no data dir")?;
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    std::fs::write(p, serde_json::to_vec_pretty(list).unwrap_or_default()).map_err(|e| e.to_string())
}

/// An item is hidden if it was marked done or not-now today, or deferred to a later day.
pub fn hidden_by(decisions: &[Decision], id: &str, today: &str) -> bool {
    decisions.iter().filter(|d| d.id == id).any(|d| match d.action.as_str() {
        "done" | "not_now" => d.on == today,
        "defer" => d.until.as_deref().map(|u| u > today).unwrap_or(false),
        _ => false,
    })
}

/// Records a decision and returns the line for the findings file.
pub fn record(id: &str, title: &str, action: &str, days: Option<i64>, reason: &str, rating: &str, today: &str) -> Result<String, String> {
    if !matches!(action, "done" | "defer" | "not_now" | "rate") {
        return Err("action must be done, defer, not_now or rate".into());
    }
    let until = (action == "defer").then(|| add_days(today, days.unwrap_or(1).clamp(1, 60)));
    let mut list = load_decisions();
    list.retain(|d| !(d.id == id && d.on == today && d.action == action));
    list.push(Decision { id: id.to_string(), title: title.to_string(), action: action.to_string(), until: until.clone(), reason: reason.to_string(), rating: rating.to_string(), on: today.to_string() });
    let cutoff = add_days(today, -60);
    list.retain(|d| d.on >= cutoff);
    save_decisions(&list)?;
    let mut line = format!("- {today} — {title} — {action}");
    if let Some(u) = until {
        line.push_str(&format!(" until {u}"));
    }
    if !rating.is_empty() {
        line.push_str(&format!(" — rated {rating}"));
    }
    if !reason.trim().is_empty() {
        line.push_str(&format!(" — reason: {}", reason.trim()));
    }
    Ok(line)
}

pub fn add_days(date: &str, n: i64) -> String {
    let Some((y, m, d)) = ymd(date) else { return date.to_string() };
    let z = days_from_civil(y, m, d) + n + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d2 = doy - (153 * mp + 2) / 5 + 1;
    let m2 = if mp < 10 { mp + 3 } else { mp - 9 };
    let y2 = yoe + era * 400 + if m2 <= 2 { 1 } else { 0 };
    format!("{y2:04}-{m2:02}-{d2:02}")
}

// ---------- building the list ----------

fn fmt_min(m: u32) -> String {
    format!("{:02}{:02}", m / 60, m % 60)
}

/// Everything the dashboard shows, assembled from the vault and Loki's own queue.
pub async fn build(obs: &ObsidianClient) -> Value {
    let now = clock();
    let day_path = crate::profile::get().daily_path(&now.date);
    let week_path = crate::profile::get().week_path(&now.iso_week);
    let events_path = crate::profile::get().events_path();
    let events_path = events_path.as_str();
    let day = obs.read_file(&day_path).await.unwrap_or_default();
    let week = obs.read_file(&week_path).await.unwrap_or_default();
    let events = obs.read_file(events_path).await.unwrap_or_default();

    let fm = frontmatter(&day);
    let starting = energy_value(&fm);
    let mut items: Vec<Item> = Vec::new();

    // Today's MITs (unchecked, tagged), then the rest of today's open tasks.
    let tasks = open_tasks(&day);
    let mit_count = tasks.iter().filter(|t| t.mit).count();
    for (i, t) in tasks.iter().filter(|t| t.mit).enumerate() {
        let mut it = item("task", &t.text, format!("One of today's 3, still open (today's note, {})", t.section), "Today's note", &day_path, 90 - i as i32);
        it.link = first_link(&t.raw).or(it.link);
        items.push(it);
    }
    if mit_count == 0 {
        for (i, t) in todays_three(&day).iter().enumerate() {
            items.push(item("task", t, "From today's 3 (the note doesn't mark which are done)".into(), "Today's note", &day_path, 85 - i as i32));
        }
    }
    for t in tasks.iter().filter(|t| !t.mit).take(6) {
        let mut it = item("task", &t.text, format!("Open in today's note ({})", t.section), "Today's note", &day_path, 35);
        it.link = first_link(&t.raw).or(it.link);
        items.push(it);
    }

    // Dated events from Upcoming Events: today (with times), then the next week.
    let range = Regex::new(r"(\d{4})\s*[–-]\s*(\d{4})").unwrap();
    for (date, text_raw) in upcoming_events_raw(&events) {
        let text = clean_md(&text_raw);
        let Some(delta) = days_between(&now.date, &date) else { continue };
        if !(0..=7).contains(&delta) {
            continue;
        }
        let mut score = match delta {
            0 => 75,
            1 => 58,
            2 | 3 => 45,
            _ => 30,
        };
        let mut when = None;
        if let Some(c) = range.captures(&text) {
            let m = |s: &str| s.parse::<u32>().unwrap_or(0) / 100 * 60 + s.parse::<u32>().unwrap_or(0) % 100;
            let (a, b) = (m(&c[1]), m(&c[2]));
            if delta == 0 {
                if now.minutes >= b {
                    continue; // already over
                }
                score += if now.minutes >= a { 25 } else if a - now.minutes <= 90 { 20 } else { 0 };
                when = Some(format!("{}–{}", fmt_min(a), fmt_min(b)));
            }
        }
        let label = match delta {
            0 => "Today".to_string(),
            1 => "Tomorrow".to_string(),
            n => format!("In {n} days ({date})"),
        };
        let mut it = item("event", &text, format!("{label}, from Upcoming Events"), "Upcoming Events", events_path, score);
        it.when = when.or(Some(date.clone()));
        it.link = first_link(&text_raw).or(it.link);
        items.push(it);
    }

    // Active sprint items, with the backlog note's next action when it is linked.
    for a in active_items(&week) {
        let title = clean_md(&a);
        let mut it = item("sprint", &title, format!("Active in this week's sprint ({})", now.iso_week), "Week note", &week_path, 50);
        it.link = first_link(&a).or(it.link);
        for link in wikilinks(&a) {
            for (_, dir) in crate::profile::get().backlog_dirs() {
                if let Ok(note) = obs.read_file(&format!("{dir}/{link}.md")).await {
                    if let Some(n) = frontmatter(&note).get("next_action").filter(|v| !v.is_empty()) {
                        it.next_action = Some(n.clone());
                    }
                    break;
                }
            }
        }
        items.push(it);
    }

    // Loki's own queue: fixes Claude prepared that wait for review.
    for r in crate::troubleshoot::list_reports(30) {
        if r["fix"]["state"] == "staged" {
            let id = r["id"].as_str().unwrap_or("");
            let mut it = item("loki", "Review a fix Claude prepared for Loki", format!("Report {id} has changes staged for your review (Reports → Fix)"), "Loki", id, 48);
            it.when = None;
            items.push(it);
        }
    }

    let decisions = load_decisions();
    let before = items.len();
    items.retain(|i| !hidden_by(&decisions, &i.id, &now.date));
    let snoozed = before - items.len();
    adjust(&mut items, &decisions, &now.date);
    let (ranked, hidden_by_capacity, mode) = rank(items, &starting);

    let windows = focus_windows(&day);
    let current = windows.iter().find(|(a, b, ..)| now.minutes >= *a && now.minutes < *b);
    let next = windows.iter().find(|(a, ..)| *a > now.minutes);
    let block = |w: Option<&(u32, u32, String, String)>| w.map(|(a, b, t, who)| json!({ "start": fmt_min(*a), "end": fmt_min(*b), "type": t, "what": who }));

    // Passive cards are hidden on a Depleted day, to keep only what matters.
    let briefings = if mode == "low" { Vec::new() } else { build_briefings(obs, &now, &day, &week, &day_path, &week_path).await };

    json!({
        "date": now.date,
        "weekday": now.weekday,
        "state": { "starting_state": starting, "mode": mode,
                   "note": match mode { "low" => "Starting state Depleted — only what matters most today.", "gentle" => "Starting state Partial — a shorter list today.", _ => "" } },
        "items": ranked,
        "briefings": briefings,
        "hidden_by_capacity": hidden_by_capacity,
        "snoozed": snoozed,
        "focus": { "current": block(current), "next": block(next) },
    })
}

/// Plain-text form for `/needsyou` in any UI.
pub fn render(r: &Value) -> String {
    let items = r["items"].as_array().cloned().unwrap_or_default();
    if items.is_empty() {
        return "Nothing needs you right now.".to_string();
    }
    let mut out = format!("**Needs you — {}** ({} item{})\n", r["weekday"].as_str().unwrap_or(""), items.len(), if items.len() == 1 { "" } else { "s" });
    if let Some(n) = r["state"]["note"].as_str().filter(|n| !n.is_empty()) {
        out.push_str(&format!("_{n}_\n"));
    }
    for (i, it) in items.iter().enumerate() {
        out.push_str(&format!("\n{}. **{}**", i + 1, it["title"].as_str().unwrap_or("")));
        if let Some(w) = it["when"].as_str() {
            out.push_str(&format!(" ({w})"));
        }
        out.push_str(&format!("\n   {}", it["why"].as_str().unwrap_or("")));
        if let Some(n) = it["next_action"].as_str() {
            out.push_str(&format!("\n   Next: {n}"));
        }
    }
    out.push_str("\n\nOpen the Needs you page to mark items done, defer them or add a reason.");
    out
}

/// Findings-file header for the first write.
pub const FINDINGS_HEADER: &str = "---\ntags: [system, findings]\n---\n\n# Needs-you findings\n\nDecisions and ratings from the Loki \"Needs you\" page, appended by Loki. Read by BOD to learn what the user defers or skips.\n\n";

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: &str = "---\nstarting_state: Partial\n---\n\n# 2026-10-27\n\n## Today's 3\n1. ATLAS ask\n2. \n3. Loki dashboard\n\n---\n\n## Focus Windows\n| Time | Type | Assigned To |\n|---|---|---|\n| ~0900–1015 | School | College lecture |\n| ~1345–1630 | School | ATLAS ask |\n\n---\n\n## Tasks\n\n### School\n- [ ] ATLAS: finish the one-page ask *(MIT 1)*\n- [x] College\n- [ ] Fix the quote\n\n### Projects\n- [ ] Heads-up for tomorrow (not a task today): x\n- [ ] [[Fix Broken Voicemail]] later\n\n## Captures\n- [ ] not a task\n";

    #[test]
    fn parses_the_daily_note() {
        assert_eq!(frontmatter(DAY).get("starting_state").map(String::as_str), Some("Partial"));
        assert_eq!(todays_three(DAY), vec!["ATLAS ask", "Loki dashboard"]);
        let t = open_tasks(DAY);
        assert_eq!(t.len(), 3, "{t:?}");
        assert!(t[0].mit && t[0].section == "School" && t[0].text == "ATLAS: finish the one-page ask");
        assert_eq!(t[2].text, "Fix Broken Voicemail later");
        let w = focus_windows(DAY);
        assert_eq!((w[0].0, w[0].1, w[1].0), (540, 615, 825));
    }

    #[test]
    fn parses_events_and_active_items() {
        let ev = "## 2026-10\n### 2026-10-27 — Tuesday\n- ATLAS ask due ~1345–1630\n  - sub\n### 2026-10-29 — Thursday\n- Call with Maj Smith\n";
        assert_eq!(upcoming_events(ev), vec![("2026-10-27".to_string(), "ATLAS ask due ~1345–1630".to_string()), ("2026-10-29".to_string(), "Call with Maj Smith".to_string())]);
        let wk = "**Active this sprint:** *(set)*\n1. ATLAS timeline, ~4h\n2. [[Fix Broken Voicemail]], 30m\n5. *(left open on purpose)*\n\n### Standing\n1. no\n";
        assert_eq!(active_items(wk), vec!["ATLAS timeline, ~4h", "[[Fix Broken Voicemail]], 30m"]);
    }

    #[test]
    fn date_math() {
        assert_eq!(days_between("2026-10-27", "2026-11-02"), Some(6));
        assert_eq!(add_days("2026-10-30", 3), "2026-11-02");
        assert_eq!(add_days("2026-03-01", -1), "2026-02-28");
    }

    fn it(title: &str, score: i32) -> Item {
        item("task", title, String::new(), "t", "p", score)
    }

    #[test]
    fn rank_dedupes_and_limits_by_capacity() {
        let items = vec![it("ATLAS one-page ask to Col Rivera", 90), it("ATLAS ask for Col Rivera draft", 50), it("Fix voicemail", 30), it("Call Maj Smith", 75), it("Run", 60), it("Laundry", 45), it("Review fix", 48)];
        let (normal, _, mode) = rank(items.clone(), "Fresh");
        assert_eq!(mode, "normal");
        assert_eq!(normal.len(), 6, "duplicate dropped");
        let (gentle, hidden, mode) = rank(items.clone(), "Partial");
        assert_eq!((mode, gentle.len(), hidden), ("gentle", 5, 1));
        let (low, _, mode) = rank(items, "Depleted");
        assert_eq!(mode, "low");
        assert_eq!(low.iter().map(|i| i.score).collect::<Vec<_>>(), vec![90, 75]);
    }

    const WEEK: &str = "---\nweek: 2026-W44\ndates: 2026-10-26 / 2026-11-01\n---\n\n# Week\n\n## Sprint Goal\n**Keep a good rhythm this month, and take care of myself.** A cold is coming on.\n\n**Top 3 priorities (per the user):**\n1. Keep a good rhythm.\n2. Take care of myself.\n\n---\n## Capacity\n\n| | Hours |\n|---|---|\n| Available (effective) | 10h |\n| Committed (Active + Queue) | 5.5h |\n| Buffer | **4.5h (45%)** |\n\n---\n> **Active: 4/5** | **Done: 0**\n\n**Active this sprint:** *(set)*\n1. ATLAS 6100A/B timeline, ~4h\n2. [[Fix Broken Voicemail]], 30m\n\n### Standing\n";

    #[test]
    fn builds_the_week_and_shape_cards() {
        let b = week_briefing(WEEK, "2026-10-27", "2026-W44", "20 - Weeks/2026-W44.md").unwrap();
        assert_eq!(b.title, "Week W44");
        assert_eq!(b.blurb, "4/5 active · buffer 4.5h (45%) · 6 days left");
        assert!(b.rows.iter().any(|r| r.head == "Goal" && r.text.starts_with("Keep a good rhythm")), "{:?}", b.rows);
        assert!(b.rows.iter().any(|r| r.head == "Capacity" && r.text.contains("10h available")));
        assert_eq!(b.rows.iter().filter(|r| r.head.starts_with("Active ")).count(), 2);
        let s = shape_briefing(DAY, 600, "p").unwrap();
        assert_eq!(s.blurb, "Now: College lecture");
        assert!(s.rows[0].head.ends_with("now"));
        assert_eq!(shape_briefing(DAY, 1200, "p").unwrap().blurb, "Today's blocks are done");
    }

    #[test]
    fn builds_news_signals_and_leads_cards() {
        let news = "# News Digest\n\n## BLUF\n\n**Technology:** Tesla stretched its geofence. Elsewhere VW won a permit.\n\n**Defense Tech:** AUSA 2026 set the tone, with industry arguing over scale. More here.\n\n---\n\n## Signals\n\n### Oil and rates moved sharply\nWTI rose 13%. The yield fell.\nFRED · 2026-10-23 · https://fred.stlouisfed.org/series/DCOILWTICO\n\n### Red cyclone\nGDACS lists a Red cyclone.\nGDACS · 2026-10-26 · https://www.gdacs.org/x\n\n## Technology\n";
        let n = news_briefing(news, "2026-10-26", "p").unwrap();
        assert_eq!(n.rows.len(), 2);
        assert_eq!(n.rows[0].text, "Tesla stretched its geofence.");
        assert_eq!(n.pill, "2 categories · 2026-10-26");
        let sg = signals_briefing(news, "p").unwrap();
        assert_eq!(sg.rows.len(), 2);
        assert_eq!(sg.rows[0].link.as_deref(), Some("https://fred.stlouisfed.org/series/DCOILWTICO"));
        assert_eq!(sg.rows[0].text, "WTI rose 13%.");
        let leads = "---\ntags: [leads]\n---\n\n*Scanned 134 entries. Hire-me: 3 · Fix-it: 7 · Skipped: 84.*\n\n## Hire-me (ranked)\n### 1. AI agent for WhatsApp — n8n Community\nhttps://community.n8n.io/t/x/1 · Effort: L · Budget: unstated\nText.\n\n### 2. Developers for chatbots\nhttps://community.n8n.io/t/x/2 · Effort: ongoing · Budget: unstated\n\n## Fix-it\n";
        let stats = json!({"by_status": {"interested": 5, "replied": 1, "new": 316}});
        let l = leads_briefing(leads, "p", Some(&stats)).unwrap();
        assert_eq!(l.title, "Leads — 3 hire-me, 7 fix-it");
        assert_eq!(l.blurb, "AI agent for WhatsApp — n8n Community");
        assert_eq!(l.rows.len(), 3);
        assert_eq!(l.rows[0].link.as_deref(), Some("https://community.n8n.io/t/x/1"));
        assert_eq!(l.rows[2].text, "5 interested · 1 replied · 316 new");
    }

    #[test]
    fn builds_countdowns_rhythm_network_and_saved() {
        let ev = "## 2026-10\n### 2026-10-27 — Tue\n- Today thing\n### 2026-10-31 — Sat\n- City 10K, Riverton, 0800\n### 2026-11-01 — Sun\n- ATLAS 6100A soft-send\n### 2027-03-01 — Mon\n- far away\n";
        let c = countdowns_briefing(ev, "2026-10-27", "p").unwrap();
        assert_eq!(c.rows.len(), 2);
        assert_eq!(c.rows[0].head, "in 4 days · 2026-10-31");
        assert!(c.blurb.starts_with("City 10K"));
        let mk = |d: &str, sleep: Option<f64>, w: Option<f64>, st: &str| {
            let mut p = DayProps::with(d, &[("masking_cost", "Med"), ("mits_done", "2")]);
            if let Some(v) = sleep { p.values.insert("sleep_h".into(), v.to_string()); }
            if let Some(v) = w { p.values.insert("weight_lbs".into(), v.to_string()); }
            if !st.is_empty() { p.values.insert("starting_state".into(), st.into()); }
            p
        };
        let days = vec![mk("2026-10-25", Some(6.0), Some(216.0), "Partial"), mk("2026-10-26", Some(7.0), Some(214.8), "Partial"), mk("2026-10-27", None, None, "")];
        let r = rhythm_briefing(&days).unwrap();
        assert!(r.blurb.contains("sleep 6.5 h") && r.blurb.contains("2 of 2 starts Partial"), "{}", r.blurb);
        assert_eq!(r.series[0].points, vec![Some(6.0), Some(7.0), None]);
    }

    #[test]
    fn builds_day_backlog_and_finance() {
        let d = day_briefing("2026-10-27", "Tuesday, October 27", Some(DAY), "### 2026-10-27 — Tue\n- Call with the mentor\n", "p");
        assert!(d.rows.iter().any(|r| r.head == "Today's 3 · 1" && r.text == "ATLAS ask"));
        assert!(d.rows.iter().any(|r| r.head == "0900–1015"));
        assert!(d.rows.iter().any(|r| r.head == "Event" && r.text.contains("mentor")));
        assert!(d.rows.iter().any(|r| r.head == "State" && r.text.contains("start Partial")));
        assert!(d.blurb.contains("2 blocks") && d.blurb.contains("3 open") && d.blurb.contains("1 done"), "{}", d.blurb);
        let none = day_briefing("2026-10-28", "Wed", None, "", "p");
        assert_eq!(none.rows[0].head, "No note yet");
        let week = "### ⏳ Queue\n- **Paused:** x\n- [ ] [[Harbor House Rental Fallback Research]]: est 1h.\n- [ ] Hard run, 5 mi progression\n\n### ✅ Done\n";
        let b = backlog_briefing(&[("Tech".into(), 49), ("Home".into(), 13)], week, "p").unwrap();
        assert_eq!(b.blurb, "62 items · 2 queued for the sprint");
        assert_eq!(b.rows.iter().filter(|r| r.head == "Queue").count(), 2);
    }

    #[test]
    fn builds_the_n8n_workflows_card() {
        let list = "- [9lrCl62Uj5U4QzoI] Sam School Records - Backfill (inactive)\n- [xzXpdec8xfSMk1op] Canonical Docs Refresh (active)\n- [Ab] Broken one (active)\n";
        let wfs = parse_n8n_workflows(list);
        assert_eq!(wfs.len(), 3);
        assert!(wfs[1].active && !wfs[0].active);
        let ex = "- [288501] xzXpdec8xfSMk1op | success | 2026-10-26T04:00:25.054Z\n- [288500] xzXpdec8xfSMk1op | success | 2026-10-25T04:00:25.045Z\n- [288502] Ab | error | 2026-10-26T10:00:00.000Z\n";
        let execs = parse_n8n_executions(ex);
        assert_eq!(execs.len(), 3);
        assert_eq!(execs[0].at, parse_iso_unix("2026-10-26T04:00:25.054Z").unwrap());
        let wf_json = r#"{"nodes":[{"type":"n8n-nodes-base.scheduleTrigger","parameters":{"rule":{"interval":[{"field":"hours","hoursInterval":24}]}}},{"type":"n8n-nodes-base.code"}]}"#;
        let sc = parse_schedule(wf_json).unwrap();
        assert_eq!((sc.text.as_str(), sc.every_secs), ("every 24 h", Some(86400)));
        assert_eq!(parse_schedule(r#"{"nodes":[{"type":"n8n-nodes-base.scheduleTrigger","parameters":{"rule":{"interval":[{"field":"cronExpression","expression":"0 6 * * *"}]}}}]}"#).unwrap().text, "cron 0 6 * * *");
        let mut sch = HashMap::new();
        sch.insert("xzXpdec8xfSMk1op".to_string(), sc);
        let now = parse_iso_unix("2026-10-26T12:00:00Z").unwrap();
        let b = n8n_briefing(&wfs, &execs, &sch, now).unwrap();
        assert_eq!(b.blurb, "1 failing · 2 active of 3");
        assert_eq!(b.pill, "check");
        // failing workflow first, then the healthy one, then inactive
        assert_eq!(b.rows[0].head, "Broken one");
        assert_eq!(b.rows[0].status.as_deref(), Some("fail"));
        assert_eq!(b.rows[1].status.as_deref(), Some("ok"));
        assert!(b.rows[1].text.contains("every 24 h") && b.rows[1].text.contains("last run 8 h ago") && b.rows[1].text.contains("next in 16 h"), "{}", b.rows[1].text);
        assert_eq!(b.rows[2].status.as_deref(), Some("off"));
        assert_eq!(b.series[0].points.len(), 7);
        assert_eq!(unwrap_result(r#"{"result":"- [a] b (active)"}"#), "- [a] b (active)");
        assert_eq!(unwrap_result("- [a] b (active)"), "- [a] b (active)");
    }

    #[test]
    fn builds_the_zotero_and_weather_cards() {
        let md = "# 3 Most Recently Added Items\n\n## 1. https://www.cisa.gov/x\n**Type:** webpage\n**Item Key:** Z6CAVZ8U\n**Date:** No date\n**Authors:** No authors listed\n**Added:** 2026-10-26T17:48:12Z\n\n## 2. How China Fights in Large-Scale Combat Operations\n**Type:** report\n**Date:** 2025-04\n**Authors:** G-2, U.S. Army TRADOC\n**Added:** 2026-10-24T00:35:35Z\n\n## 3. loose.pdf\n**Type:** attachment\n**Added:** 2026-10-23T00:00:00Z\n";
        let items = parse_zotero_recent(md);
        assert_eq!(items.len(), 3);
        assert_eq!(items[1].authors, "G-2, U.S. Army TRADOC");
        let now = parse_iso_unix("2026-10-27T00:00:00Z").unwrap();
        let z = zotero_briefing(&items, now).unwrap();
        assert_eq!(z.rows.len(), 2, "attachments are left out");
        assert!(z.blurb.starts_with("2 added this week"), "{}", z.blurb);
        assert!(z.rows[1].text.contains("report · G-2, U.S. Army TRADOC · added"));
        let w = json!({
            "current": {"temperature_2m": 61.4, "apparent_temperature": 59.0, "weather_code": 2, "wind_speed_10m": 6.2},
            "daily": {"time": ["2026-10-27", "2026-10-28", "2026-10-29", "2026-10-30"], "temperature_2m_max": [68.0, 70.0, 66.0, 64.0], "temperature_2m_min": [49.0, 51.0, 50.0, 47.0],
                      "precipitation_probability_max": [10, 20, 60, 0], "weather_code": [2, 3, 63, 0], "sunrise": ["2026-10-27T07:21"], "sunset": ["2026-10-27T18:08"]},
            "hourly": {"time": ["2026-10-27T09:00", "2026-10-27T12:00", "2026-10-27T17:00", "2026-10-27T20:00"], "temperature_2m": [55.0, 64.0, 66.0, 58.0], "precipitation_probability": [0, 5, 10, 10], "weather_code": [1, 2, 2, 0]}
        });
        let wb = weather_briefing(&w, "Home").unwrap();
        assert_eq!(wb.blurb, "61° Partly cloudy · high 68° low 49°");
        assert!(wb.rows.iter().any(|r| r.head == "12 pm" && r.text.starts_with("64° · Partly cloudy")));
        assert!(wb.rows.iter().any(|r| r.head == "Sun" && r.text.contains("sets 18:08")));
        assert_eq!(wb.rows.iter().filter(|r| r.head.starts_with("2026-10-")).count(), 3);
    }


    #[test]
    fn today_data_has_routines_savers_and_weather_apart_from_the_tasks() {
        let day = "---\nstarting_state: Fresh\n---\n# D\n\n## SAVERS\n- [x] Silence\n- [ ] Reading\n\n## Weather\n| Time | Conditions | Temp | Humidity | Rain % |\n|---|---|---|---|---|\n| Morning (9am) | Clear sky | ~59°F | 72% | — |\n\n## Focus Windows\n| Time | Type | Assigned To |\n|---|---|---|\n| ~0900–1015 | School | College |\n\n## Tasks\n\n### Routines\n- [ ] /bod — start the day\n- [x] /eod — close the day\n\n### School\n- [ ] College\n\n## Captures\n";
        let d = shape_data(day, 600);
        assert_eq!(d["routines"].as_array().unwrap().len(), 2);
        assert_eq!((d["routines"][0]["cmd"].as_str(), d["routines"][1]["done"].as_bool()), (Some("/bod"), Some(true)));
        assert_eq!((d["total"].as_u64(), d["sections"].as_array().unwrap().len()), (Some(1), 1), "routines aren't counted as tasks");
        assert_eq!((d["savers"][0]["done"].as_bool(), d["savers"].as_array().unwrap().len()), (Some(true), 2));
        assert_eq!((d["weather"][0]["slot"].as_str(), d["weather"][0]["cond"].as_str()), (Some("Morning"), Some("Clear sky")));
        assert!(open_tasks(day).iter().all(|t| t.section != "Routines"), "routines don't show up in Needs-you");
        let week = "---\ndates: 2026-10-26 / 2026-11-01\n---\n## Sprint Backlog\n### Routines\n- [ ] /bow — plan the week\n- [x] /eow — close the week\n### School\n- [ ] X\n";
        let w = week_data(week, "2026-10-27", "2026-W44");
        assert_eq!(w["routines"].as_array().unwrap().len(), 2);
        assert!(w["domains"].as_array().unwrap().iter().all(|x| x["name"] != "Routines"));
    }

    #[test]
    fn first_sentence_counts_characters_not_bytes() {
        assert_eq!(first_sentence("Dash — then more – still. Next one here", 100), "Dash — then more – still.");
    }

    #[test]
    fn dashboard_data_for_batch_three_cards() {
        let s = signals_briefing("## Signals\n### A signal\nSome detail text.\nsource · 2026-10-26 · https://www.example.com/a\n", "p").unwrap();
        assert_eq!(s.data.unwrap()["items"][0]["domain"], "example.com");
        let z = zotero_briefing(&[ZItem { title: "A paper".into(), kind: "journalArticle".into(), authors: "Doe".into(), added: "2026-10-26T10:00:00Z".into(), key: "ABCD".into() }], 1_793_000_000).unwrap();
        assert_eq!(z.data.unwrap()["items"][0]["link"], "zotero://select/library/items/ABCD");
    }

    #[test]
    fn dashboard_data_for_batch_two_cards() {
        let stats = serde_json::json!({ "by_status": { "interested": 2, "new": 5 }, "by_kind": { "hire": 3 }, "per_day": [] });
        let l = leads_briefing("Hire-me: 2 · Fix-it: 7\n\n## Hire-me (ranked)\n### 1. Build a thing\nplatform · detail · https://example.com/x\n", "p", Some(&stats)).unwrap();
        let d = l.data.unwrap();
        assert_eq!((d["hire"].as_u64(), d["items"].as_array().map(|a| a.len())), (Some(2), Some(1)));
        assert_eq!(d["stats"]["by_status"]["new"], 5);
        let r = rhythm_briefing(&[DayProps::with("2026-10-26", &[("sleep_h", "7.5"), ("weight_lbs", "215"), ("starting_state", "Fresh"), ("mits_done", "2")])]).unwrap().data.unwrap();
        assert_eq!(r["days"][0]["values"]["sleep_h"], 7.5);
        assert_eq!(r["days"][0]["values"]["starting_state"], "Fresh");
        assert!(r["days"][0]["values"]["waist_in"].is_null());
        assert_eq!(r["props"].as_array().unwrap().len(), 4);
        assert_eq!(r["props"][1]["role"], "energy");
    }

    #[test]
    fn dashboard_data_for_today_the_sprint_and_countdowns() {
        let d = shape_data(DAY, 600);
        assert!(d["windows"].as_array().is_some_and(|w| !w.is_empty()));
        assert_eq!(d["now_min"], 600);
        let w = week_data(WEEK, "2026-10-27", "2026-W44");
        assert_eq!(w["week"], "2026-W44");
        assert!(w["capacity"]["buffer_pct"].as_f64().is_some() || w["capacity"]["buffer"].as_f64().is_some());
        let c = countdowns_data("## 2026-11\n### 2026-11-02 — Monday\n- City run at the track\n### 2026-10-28 — Wednesday\n- College lecture\n", "2026-10-27");
        let e = c["events"].as_array().unwrap();
        assert_eq!((e[0]["days"].as_i64(), e[0]["topic"].as_str()), (Some(1), Some("school")));
        assert_eq!(e[1]["topic"], "fitness");
    }

    #[test]
    fn builds_the_mail_page_with_a_column_per_mailbox() {
        let mk = |acct: &str, id: &str, from: &str, subj: &str| crate::mail::MsgMeta { account: acct.into(), id: id.into(), from: from.into(), subject: subj.into(), at_ms: 1_000_000, snippet: String::new(), unread: true, labels: vec![], bulk: false };
        let promo = { let mut m = mk("a@x", "2", "Promo <p@x>", "50% off"); m.bulk = true; m.labels = vec!["CATEGORY_PROMOTIONS".into()]; m };
        let data = vec![
            ("a@x".to_string(), Some(7u64), vec![mk("a@x", "1", "Rivera <s@x>", "Q3 numbers"), promo, mk("a@x", "3", "Bank <b@x>", "Statement ready"), mk("a@x", "4", "Doc <d@x>", "Please review the draft")]),
            ("b@x".to_string(), Some(3u64), vec![mk("b@x", "9", "Pat <p@y>", "Dinner Friday?")]),
        ];
        let tri: HashMap<String, String> = [("a@x:1", "action"), ("a@x:3", "fyi"), ("a@x:4", "review")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let b = mail_briefing(&data, &tri, 1_000_000 + 3 * 3600 * 1000).unwrap();
        assert_eq!(b.blurb, "10 unread · 1 need action", "card face has counts only");
        let col = |g: &str| b.rows.iter().filter(|r| r.group.as_deref() == Some(g)).collect::<Vec<_>>();
        assert_eq!(col("a@x").len() + col("b@x").len(), b.rows.len(), "every row belongs to a mailbox column");
        let a = col("a@x");
        assert!(a[0].head == "Summary" && a[0].text.contains("1 need action") && a[0].text.contains("1 to review") && a[0].text.contains("1 FYI") && a[0].text.contains("1 receipts and junk hidden"), "{}", a[0].text);
        assert!(a.iter().any(|r| r.heading && r.head == "Needs action (1)"));
        assert!(a.iter().any(|r| r.head == "Rivera" && r.status.as_deref() == Some("warn")));
        assert!(!a.iter().any(|r| r.head == "Promo"), "junk stays hidden");
        assert!(col("b@x").iter().any(|r| r.head == "Pat"), "unsorted mail is listed under 'still being read'");
    }

    #[test]
    fn items_with_something_to_link_to_get_links() {
        assert_eq!(first_link("see [the post](https://forum.example/t/1) now").as_deref(), Some("https://forum.example/t/1"));
        assert_eq!(first_link("read https://a.example/x, then reply").as_deref(), Some("https://a.example/x"));
        assert_eq!(first_link("[[Fix Broken Voicemail]], 30m").as_deref(), Some("obsidian://open?vault=Obsidian%20Vault&file=Fix%20Broken%20Voicemail"));
        assert_eq!(first_link("[[Some Note#Heading|alias]]").as_deref(), Some("obsidian://open?vault=Obsidian%20Vault&file=Some%20Note"));
        assert_eq!(first_link("nothing to link"), None);
        assert_eq!(note_url("70 - Sources/Technology/A & B.md"), "obsidian://open?vault=Obsidian%20Vault&file=70%20-%20Sources%2FTechnology%2FA%20%26%20B");
        // week: active sprint items link to their note
        let b = week_briefing(WEEK, "2026-10-27", "2026-W44", "20 - Weeks/2026-W44.md").unwrap();
        let voicemail = b.rows.iter().find(|r| r.head == "Active 2").unwrap();
        assert!(voicemail.link.as_deref().unwrap().ends_with("file=Fix%20Broken%20Voicemail"));
        assert!(b.rows.iter().find(|r| r.head == "Active 1").unwrap().link.is_none());
        assert!(b.note.as_deref().unwrap().contains("20%20-%20Weeks%2F2026-W44"));
        // day page: a wikilinked task and a linked event
        let day = "# d\n\n## Tasks\n\n### Home\n- [ ] [[Fix Broken Voicemail]] later\n- [ ] Plain task\n";
        let d = day_briefing("2026-10-27", "Tue", Some(day), "### 2026-10-27 — Tue\n- Call [[Maj Smith]] https://meet.example/x\n", "10 - Daily/2026-10-27.md");
        assert!(d.rows.iter().find(|r| r.text == "Fix Broken Voicemail later").unwrap().link.is_some());
        assert!(d.rows.iter().find(|r| r.text == "Plain task").unwrap().link.is_none());
        assert_eq!(d.rows.iter().find(|r| r.head == "Event").unwrap().link.as_deref(), Some("https://meet.example/x"));
        let z = zotero_briefing(&[ZItem { title: "https://www.cisa.gov/x".into(), kind: "webpage".into(), authors: "No authors listed".into(), added: "2026-10-26T17:48:12Z".into(), key: "Z6CAVZ8U".into() }, ZItem { title: "How China Fights".into(), kind: "report".into(), authors: "G-2".into(), added: "2026-10-24T00:35:35Z".into(), key: "3Q35W7G6".into() }], parse_iso_unix("2026-10-27T00:00:00Z").unwrap()).unwrap();
        assert_eq!(z.rows[0].link.as_deref(), Some("https://www.cisa.gov/x"));
        assert_eq!(z.rows[1].link.as_deref(), Some("zotero://select/library/items/3Q35W7G6"));
    }

    #[test]
    fn plain_text_finds_its_note_only_when_the_names_overlap() {
        let res = vec![("98 - Loki Conversations/2026-10-26 - web.md".to_string(), -9.0), ("10 - Daily/2026-10-26.md".to_string(), -8.0), ("30 - Backlog/School/ATLAS Project Setup.md".to_string(), -7.0), ("40 - Projects/Skald/README.md".to_string(), -6.0)];
        assert_eq!(pick_note(&res, "ATLAS 6100A/B timeline, in the 6100 blocks").as_deref(), Some("30 - Backlog/School/ATLAS Project Setup.md"));
        assert_eq!(pick_note(&res, "City 10K race Saturday"), None, "no overlap, no link");
    }

    #[test]
    fn past_decisions_adjust_scores() {
        let mk = |action: &str, rating: &str, on: &str| Decision { id: "p".into(), title: "Fix the quote".into(), action: action.into(), until: None, reason: String::new(), rating: rating.into(), on: on.into() };
        let mut items = vec![item("task", "Fix the quote", "why".into(), "t", "p", 50)];
        items[0].id = "p".into();
        let set_aside = vec![mk("not_now", "", "2026-10-20"), mk("defer", "", "2026-10-22"), mk("not_now", "", "2026-10-25")];
        adjust(&mut items, &set_aside, "2026-10-27");
        assert_eq!(items[0].score, 35);
        assert!(items[0].why.contains("set aside 3 times"));
        let mut one = vec![item("task", "Fix the quote", "why".into(), "t", "p", 50)];
        adjust(&mut one, &[mk("rate", "down", "2026-10-26")], "2026-10-27");
        assert_eq!(one[0].score, 25);
        let mut up = vec![item("task", "Fix the quote", "why".into(), "t", "p", 50)];
        adjust(&mut up, &[mk("rate", "up", "2026-10-26"), mk("rate", "down", "2026-09-01")], "2026-10-27");
        assert_eq!(up[0].score, 60);
        let mut old = vec![item("task", "Fix the quote", "why".into(), "t", "p", 50)];
        adjust(&mut old, &[mk("rate", "down", "2026-09-01")], "2026-10-27");
        assert_eq!(old[0].score, 50, "old ratings don't count");
    }

    #[test]
    fn start_here_card_shows_until_the_profile_is_filled_in() {
        let blank = crate::profile::Profile::default();
        let b = start_here_briefing(&blank).unwrap();
        assert_eq!((b.kind.as_str(), b.pill.as_str()), ("start", "New"));
        assert!(b.rows.iter().any(|r| r.head.starts_with("1 ")) && b.rows.iter().any(|r| r.head.starts_with("2 ")));
        let mut full = crate::profile::Profile::starter();
        assert!(start_here_briefing(&full).is_some(), "starter pack but no name yet");
        full.person.name = "Sam".into();
        assert!(start_here_briefing(&full).is_none());
    }

    #[test]
    fn decisions_hide_for_the_right_time() {
        let d = |action: &str, until: Option<&str>, on: &str| Decision { id: "x".into(), title: "t".into(), action: action.into(), until: until.map(String::from), reason: String::new(), rating: String::new(), on: on.into() };
        assert!(hidden_by(&[d("done", None, "2026-10-27")], "x", "2026-10-27"));
        assert!(!hidden_by(&[d("done", None, "2026-10-27")], "x", "2026-10-28"));
        assert!(hidden_by(&[d("defer", Some("2026-10-30"), "2026-10-27")], "x", "2026-10-29"));
        assert!(!hidden_by(&[d("defer", Some("2026-10-30"), "2026-10-27")], "x", "2026-10-30"));
        assert!(!hidden_by(&[d("done", None, "2026-10-27")], "y", "2026-10-27"));
    }
}
