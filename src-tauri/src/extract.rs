//! Extract: the local model reads one message (or any text in Loki) and PROPOSES things worth keeping: events,
//! to-dos, knowledge and reference facts. the user can change any item's type, edit every field, add items by hand,
//! and nothing is written until he approves each one. Approved events (and to-dos with a date) go into
//! `60 - MOCs/Upcoming Events.md`, which is the calendar, in its existing format; undated to-dos, knowledge and
//! reference items become `00 - Inbox/` notes for `/inbox-triage` (knowledge ones tagged `[knowledge]`). Only the
//! approved fields are written, never the message text. Already-extracted sources are remembered.

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// The dated-events file (from the profile).
pub fn events_path() -> String {
    crate::profile::get().events_path()
}
pub const TOPICS: [&str; 5] = ["Technology", "Self", "Philosophy", "Work", "Other"];

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Proposal {
    /// "event" | "todo" | "knowledge" | "reference"
    pub kind: String,
    pub title: String,
    #[serde(default)]
    pub date: String,
    #[serde(default)]
    pub start: String,
    #[serde(default)]
    pub end: String,
    #[serde(default)]
    pub place: String,
    /// Who is involved, as written ("Maj Smith, Alex").
    #[serde(default)]
    pub people: String,
    /// Why it matters, one line.
    #[serde(default)]
    pub purpose: String,
    #[serde(default)]
    pub next_action: String,
    #[serde(default)]
    pub area: String,
    /// e.g. "30m", "1h".
    #[serde(default)]
    pub estimate: String,
    /// Knowledge notes: which part of the vault it belongs with.
    #[serde(default)]
    pub topic: String,
    /// Free text: notes for events and to-dos, the body for knowledge and reference items.
    #[serde(default)]
    pub note: String,
}

pub fn prompt(text: &str, today: &str, weekday: &str) -> (String, String) {
    let prof = crate::profile::get();
    let who = prof.person_intro();
    // The backlog areas come from the profile; a flat backlog has none, so the field is left out.
    let area_field = if prof.backlog.areas.is_empty() { String::new() } else { format!("\"area\":\"{}\",", prof.backlog.areas.join("|")) };
    let system = format!(
        "You find what is worth keeping in a piece of text, for one person: {who}. Today is {weekday} {today}. Resolve relative dates (Friday, next week) to YYYY-MM-DD from today.\n\
Output a JSON array and nothing else; [] if there is nothing. One object per item. Leave a field \"\" when the text does not give it.\n\
event: {{\"kind\":\"event\",\"title\":\"what it is\",\"date\":\"YYYY-MM-DD\",\"start\":\"HH:MM 24h\",\"end\":\"HH:MM\",\"place\":\"\",\"people\":\"names\",\"purpose\":\"why it matters, one line\",\"note\":\"\"}}\n\
todo: {{\"kind\":\"todo\",\"title\":\"short imperative\",\"date\":\"due date YYYY-MM-DD\",\"next_action\":\"first step, under 12 words\",{area_field}\"estimate\":\"30m or 1h\",\"people\":\"\",\"note\":\"\"}}\n\
knowledge: {{\"kind\":\"knowledge\",\"title\":\"one assertive sentence stating the idea\",\"topic\":\"Technology|Self|Philosophy|Work|Other\",\"note\":\"1 to 4 sentences explaining it in your own words\"}}\n\
reference: {{\"kind\":\"reference\",\"title\":\"what it is\",\"note\":\"the fact to keep: a number, address, rule or instruction\"}}\n\
Rules: an event has a date he should be at or aware of. A todo is something he must do or reply to. Knowledge is a durable idea, insight or claim worth keeping, not a task. Reference is a concrete fact he may need later. Do not invent dates, times, places, names or facts that are not in the text. Skip marketing and anything with nothing worth keeping. At most 10 items. Titles under 100 characters; do not copy long passages."
    );
    (system, text.chars().take(9000).collect())
}

fn valid_date(d: &str) -> bool {
    Regex::new(r"^\d{4}-\d{2}-\d{2}$").unwrap().is_match(d) && d[5..7].parse::<u32>().is_ok_and(|m| (1..=12).contains(&m)) && d[8..10].parse::<u32>().is_ok_and(|x| (1..=31).contains(&x))
}
fn valid_time(t: &str) -> bool {
    Regex::new(r"^([01]\d|2[0-3]):[0-5]\d$").unwrap().is_match(t)
}

fn kind_of(s: &str) -> Option<&'static str> {
    match s.trim().to_lowercase().as_str() {
        "event" => Some("event"),
        "todo" | "to-do" | "action" | "task" => Some("todo"),
        "knowledge" | "idea" | "insight" => Some("knowledge"),
        "reference" | "fact" => Some("reference"),
        _ => None,
    }
}

/// Cleans one proposal, whether it came from the model or was edited or typed by the user: same rules for all.
/// Bad dates, times and labels are dropped rather than trusted; items missing what their type needs are rejected.
pub fn sanitize(p: Proposal) -> Option<Proposal> {
    let one = |s: &str, n: usize| s.replace(['\n', '\r'], " ").split_whitespace().collect::<Vec<_>>().join(" ").chars().take(n).collect::<String>();
    let kind = kind_of(&p.kind)?;
    let title = one(&p.title, 160);
    if title.is_empty() {
        return None;
    }
    let date = if valid_date(p.date.trim()) { p.date.trim().to_string() } else { String::new() };
    if kind == "event" && date.is_empty() {
        return None; // an event without a date can't go on the calendar
    }
    let t = |x: &str| if valid_time(x.trim()) { x.trim().to_string() } else { String::new() };
    let area = ["Home", "School", "Self", "Tech"].iter().find(|a| a.eq_ignore_ascii_case(p.area.trim())).map(|a| a.to_string()).unwrap_or_default();
    let topic = TOPICS.iter().find(|a| a.eq_ignore_ascii_case(p.topic.trim())).map(|a| a.to_string()).unwrap_or_default();
    let body = matches!(kind, "knowledge" | "reference");
    // the note keeps its line breaks for knowledge and reference items
    let note = if body { p.note.replace('\r', "").trim().chars().take(1500).collect::<String>() } else { one(&p.note, 300) };
    Some(Proposal {
        kind: kind.to_string(),
        title,
        date: if body { String::new() } else { date },
        start: if kind == "event" { t(&p.start) } else { String::new() },
        end: if kind == "event" { t(&p.end) } else { String::new() },
        place: if kind == "event" { one(&p.place, 100) } else { String::new() },
        people: if body { String::new() } else { one(&p.people, 120) },
        purpose: if kind == "event" { one(&p.purpose, 200) } else { String::new() },
        next_action: if kind == "todo" { one(&p.next_action, 120) } else { String::new() },
        area: if kind == "todo" { area } else { String::new() },
        estimate: if kind == "todo" { one(&p.estimate, 10) } else { String::new() },
        topic: if kind == "knowledge" { topic } else { String::new() },
        note,
    })
}

/// Parses the model's answer into clean proposals.
pub fn parse(reply: &str) -> Vec<Proposal> {
    let (Some(a), Some(b)) = (reply.find('['), reply.rfind(']')) else { return Vec::new() };
    let raw: Vec<Value> = serde_json::from_str(&reply[a..=b]).unwrap_or_default();
    raw.iter().take(10).filter_map(|v| serde_json::from_value::<Proposal>(v.clone()).ok()).filter_map(sanitize).collect()
}

/// Does this item go on the calendar (the Upcoming Events note)?
pub fn is_dated(p: &Proposal) -> bool {
    !p.date.is_empty() && matches!(p.kind.as_str(), "event" | "todo")
}

/// The lines an approved dated item adds under its date in the events file: a bullet, then sub-bullets for the detail.
pub fn event_lines(p: &Proposal, label: &str, today: &str) -> Vec<String> {
    let title = if p.kind == "todo" { format!("Due: {}", p.title) } else { p.title.clone() };
    let mut bits: Vec<String> = Vec::new();
    if !p.start.is_empty() {
        bits.push(if p.end.is_empty() { p.start.clone() } else { format!("{}–{}", p.start, p.end) });
    }
    if !p.place.is_empty() {
        bits.push(p.place.clone());
    }
    let detail = if bits.is_empty() { String::new() } else { format!(" — {}", bits.join(", ")) };
    let mut out = vec![format!("- **{title}**{detail} *(extracted from {label}, {today})*")];
    for (name, v) in [("With", &p.people), ("Purpose", &p.purpose), ("First step", &p.next_action), ("Notes", &p.note)] {
        if !v.is_empty() {
            out.push(format!("  - {name}: {v}"));
        }
    }
    out
}

fn is_month(h: &str) -> bool {
    Regex::new(r"^\d{4}-\d{2}$").unwrap().is_match(h.trim())
}

/// Inserts `lines` under `### <date>` inside `## <YYYY-MM>`, creating either heading in date order. Returns the new text.
pub fn insert_event(text: &str, date: &str, day_name: &str, lines: &[String]) -> String {
    let month = &date[..7];
    let mut ls: Vec<String> = text.lines().map(String::from).collect();
    let heading = format!("### {date} — {day_name}");
    let last_content = |ls: &[String], from: usize, to: usize| -> Option<usize> { (from..to).rev().find(|&i| !ls[i].trim().is_empty()) };
    match ls.iter().position(|l| l.trim() == format!("## {month}")) {
        None => {
            let pos = ls.iter().position(|l| l.starts_with("## ") && is_month(&l[3..]) && l[3..].trim() > month).unwrap_or(ls.len());
            let mut block = vec![format!("## {month}"), String::new(), heading];
            block.extend(lines.iter().cloned());
            block.push(String::new());
            if pos == ls.len() && ls.last().is_some_and(|l| !l.trim().is_empty()) {
                block.insert(0, String::new());
            }
            for (k, b) in block.into_iter().enumerate() {
                ls.insert(pos + k, b);
            }
        }
        Some(m) => {
            let end = (m + 1..ls.len()).find(|&i| ls[i].starts_with("## ")).unwrap_or(ls.len());
            match (m + 1..end).find(|&i| ls[i].starts_with(&format!("### {date}"))) {
                Some(d) => {
                    let e = (d + 1..end).find(|&i| ls[i].starts_with('#')).unwrap_or(end);
                    let at = last_content(&ls, d + 1, e).map_or(d + 1, |i| i + 1);
                    for (k, l) in lines.iter().enumerate() {
                        ls.insert(at + k, l.clone());
                    }
                }
                None => {
                    let later = (m + 1..end).find(|&i| ls[i].starts_with("### ") && ls[i].len() >= 14 && &ls[i][4..14] > date);
                    let mut block = vec![heading.clone()];
                    block.extend(lines.iter().cloned());
                    block.push(String::new());
                    match later {
                        Some(i) => {
                            for (k, b) in block.into_iter().enumerate() {
                                ls.insert(i + k, b);
                            }
                        }
                        None => {
                            // end of the month: after its last bullet; the blank line that followed it stays after the new block
                            let at = last_content(&ls, m + 1, end).map_or(m + 1, |i| i + 1);
                            let mut block = vec![String::new(), heading];
                            block.extend(lines.iter().cloned());
                            for (k, b) in block.into_iter().enumerate() {
                                ls.insert(at + k, b);
                            }
                        }
                    }
                }
            }
        }
    }
    let mut out = ls.join("\n");
    if text.ends_with('\n') || !text.contains('\n') {
        out.push('\n');
    }
    out
}

/// Every original line still present, in order (blank lines aside): the safety check before overwriting the file.
pub fn preserves(old: &str, new: &str) -> bool {
    let mut it = new.lines();
    old.lines().filter(|l| !l.trim().is_empty()).all(|l| it.any(|n| n == l))
}

pub fn safe_filename(title: &str) -> String {
    let s: String = title.chars().map(|c| if "/\\:*?\"<>|#^[]".contains(c) { ' ' } else { c }).collect();
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    s.chars().take(80).collect::<String>().trim().to_string()
}

/// An Inbox note for anything that isn't dated: an undated to-do (tagged `[action]`), or a knowledge or reference
/// item (tagged `[knowledge]`). Captures only the approved fields and a link back; never the message text.
pub fn inbox_note(p: &Proposal, label: &str, link: &str, today: &str) -> String {
    let mut s = String::from("---\n");
    match p.kind.as_str() {
        "todo" => {
            s.push_str("tags: [action]\n");
            s.push_str(&format!("estimate: {}\n", p.estimate));
            s.push_str(&format!("next_action: {}\n", p.next_action.replace(':', " -")));
        }
        "reference" => s.push_str("tags: [knowledge, reference]\n"),
        _ => {
            s.push_str("tags: [knowledge]\n");
            if !p.topic.is_empty() {
                s.push_str(&format!("topic: {}\n", p.topic));
            }
        }
    }
    if !link.is_empty() {
        s.push_str(&format!("source: {link}\n"));
    }
    s.push_str(&format!("---\n# {}\n", p.title));
    if p.kind == "todo" {
        if !p.next_action.is_empty() {
            s.push_str(&format!("\n- [ ] {}\n", p.next_action));
        }
        if !p.people.is_empty() {
            s.push_str(&format!("\nWith: {}\n", p.people));
        }
        if !p.area.is_empty() {
            s.push_str(&format!("\nSuggested area: {}.\n", p.area));
        }
    }
    if !p.note.is_empty() {
        s.push_str(&format!("\n{}\n", p.note));
    }
    s.push_str(&format!("\nCaptured from {label} on {today} with Loki Extract.\n"));
    s
}

fn seen_path() -> Option<std::path::PathBuf> {
    let d = dirs::data_dir()?.join("loki");
    std::fs::create_dir_all(&d).ok()?;
    Some(d.join("extract_seen.json"))
}
pub fn seen(key: &str) -> bool {
    seen_path().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str::<HashMap<String, u64>>(&t).ok()).is_some_and(|m| m.contains_key(key))
}
pub fn mark_seen(key: &str) {
    let Some(p) = seen_path() else { return };
    let mut m: HashMap<String, u64> = std::fs::read_to_string(&p).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    m.insert(key.to_string(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0));
    if m.len() > 2000 {
        m.clear();
    }
    let _ = std::fs::write(p, serde_json::to_string(&m).unwrap_or_default());
}

#[cfg(test)]
mod tests {
    use super::*;

    const EV: &str = "# Upcoming Events\n\n---\n\n## 2026-10\n\n### 2026-10-27 — Tuesday\n- **ATLAS ask**\n  - detail\n\n### 2026-10-29 — Thursday\n- **Call**\n\n## 2026-12\n\n### 2026-12-13 — Sunday\n- **City MILE**\n";

    fn lines(t: &str) -> Vec<String> { vec![format!("- **{t}**")] }

    #[test]
    fn parses_all_four_kinds_and_cleans() {
        let p = parse("sure: [{\"kind\":\"event\",\"title\":\"Inspection\",\"date\":\"2026-11-03\",\"start\":\"10:00\",\"end\":\"25:99\",\"people\":\"Maj Smith\",\"purpose\":\"annual check\"},{\"kind\":\"event\",\"title\":\"No date\"},{\"kind\":\"action\",\"title\":\"Reply\",\"date\":\"someday\",\"area\":\"school\",\"estimate\":\"30m\"},{\"kind\":\"knowledge\",\"title\":\"Defaults beat rules\",\"topic\":\"self\",\"note\":\"Line one.\\nLine two.\"},{\"kind\":\"reference\",\"title\":\"Gate code\",\"note\":\"4821\"},{\"kind\":\"junk\",\"title\":\"x\"}]");
        assert_eq!(p.len(), 5 - 1 + 0); // event w/o date and the junk kind are dropped
        assert_eq!((p[0].start.as_str(), p[0].end.as_str(), p[0].people.as_str()), ("10:00", "", "Maj Smith"));
        assert_eq!((p[1].kind.as_str(), p[1].date.as_str(), p[1].area.as_str(), p[1].estimate.as_str()), ("todo", "", "School", "30m"));
        assert_eq!((p[2].kind.as_str(), p[2].topic.as_str()), ("knowledge", "Self"));
        assert!(p[2].note.contains('\n'));
        assert_eq!(p[3].kind, "reference");
    }

    #[test]
    fn user_edits_are_held_to_the_same_rules() {
        let mut p = Proposal { kind: "knowledge".into(), title: "An idea".into(), date: "2026-11-03".into(), place: "x".into(), people: "y".into(), ..Default::default() };
        let c = sanitize(p.clone()).unwrap();
        assert!(c.date.is_empty() && c.place.is_empty() && c.people.is_empty(), "knowledge keeps no event fields");
        p.kind = "event".into();
        p.date.clear();
        assert!(sanitize(p).is_none(), "an event needs a date");
        assert!(sanitize(Proposal { kind: "todo".into(), title: "  ".into(), ..Default::default() }).is_none());
    }

    #[test]
    fn inserts_in_date_order_and_keeps_everything() {
        let a = insert_event(EV, "2026-10-27", "Tuesday", &lines("New"));
        assert!(a.contains("  - detail\n- **New**\n\n### 2026-10-29"));
        let b = insert_event(EV, "2026-10-28", "Wednesday", &lines("Mid"));
        let (i27, i28, i29) = (b.find("### 2026-10-27").unwrap(), b.find("### 2026-10-28").unwrap(), b.find("### 2026-10-29").unwrap());
        assert!(i27 < i28 && i28 < i29);
        assert!(preserves(EV, &b));
        let c = insert_event(EV, "2026-10-31", "Saturday", &lines("Late"));
        assert!(c.find("### 2026-10-31").unwrap() < c.find("## 2026-12").unwrap());
        assert!(preserves(EV, &c) && c.contains("- **Late**\n\n## 2026-12"));
        let d = insert_event(EV, "2026-11-02", "Monday", &lines("Nov"));
        assert!(d.find("## 2026-10").unwrap() < d.find("## 2026-11").unwrap() && d.find("## 2026-11").unwrap() < d.find("## 2026-12").unwrap());
        let e = insert_event(EV, "2027-01-05", "Tuesday", &lines("Jan"));
        assert!(e.trim_end().ends_with("- **Jan**") && preserves(EV, &e));
    }

    #[test]
    fn events_carry_people_purpose_and_notes_as_sub_bullets() {
        let p = Proposal { kind: "event".into(), title: "Call with Maj Smith".into(), date: "2026-10-29".into(), start: "14:00".into(), end: "15:00".into(), place: "Teams".into(), people: "Maj Smith".into(), purpose: "Acme Software Factory".into(), note: "time may change".into(), ..Default::default() };
        let l = event_lines(&p, "an email", "2026-10-27");
        assert_eq!(l[0], "- **Call with Maj Smith** — 14:00–15:00, Teams *(extracted from an email, 2026-10-27)*");
        assert_eq!(&l[1..], ["  - With: Maj Smith", "  - Purpose: Acme Software Factory", "  - Notes: time may change"]);
        let t = Proposal { kind: "todo".into(), title: "Renew policy".into(), date: "2026-11-05".into(), next_action: "compare quotes".into(), ..Default::default() };
        assert_eq!(event_lines(&t, "an email", "2026-10-27")[1], "  - First step: compare quotes");
        assert!(is_dated(&t) && !is_dated(&Proposal { kind: "knowledge".into(), date: "2026-11-05".into(), ..Default::default() }));
    }

    #[test]
    fn prompt_offers_the_profile_areas_and_names_nobody() {
        let (sys, _) = prompt("x", "2026-10-30", "Friday");
        assert!(sys.contains("\"area\":\"Tech|Home|School|Self\"") || sys.contains("\"area\":\"Home|School|Self|Tech\""), "{sys}");
        assert!(sys.contains("for one person: the user") || sys.contains("for one person:"));
    }

    #[test]
    fn inbox_notes_for_todos_knowledge_and_reference() {
        let t = Proposal { kind: "todo".into(), title: "Send phone number".into(), next_action: "reply".into(), estimate: "5m".into(), area: "Home".into(), people: "PM".into(), ..Default::default() };
        let n = inbox_note(&t, "an email", "https://mail.google.com/x", "2026-10-27");
        assert!(n.starts_with("---\ntags: [action]\nestimate: 5m") && n.contains("Suggested area: Home.") && n.contains("With: PM"));
        let k = Proposal { kind: "knowledge".into(), title: "Defaults beat rules".into(), topic: "Self".into(), note: "Explained.".into(), ..Default::default() };
        let kn = inbox_note(&k, "a chat reply", "", "2026-10-27");
        assert!(kn.starts_with("---\ntags: [knowledge]\ntopic: Self\n---\n# Defaults beat rules") && kn.contains("Explained."));
        assert!(inbox_note(&Proposal { kind: "reference".into(), title: "Gate".into(), ..Default::default() }, "x", "", "d").contains("tags: [knowledge, reference]"));
        assert_eq!(safe_filename("Re: a/b? [x] #1"), "Re a b x 1");
    }
}
