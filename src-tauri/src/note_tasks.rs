//! Ticking a task in a daily or sprint note from Loki. Exactly one checkbox line changes (and optionally gets a short
//! dated comment on the end); every other line is left as it was, and the edit is refused if that isn't true.

use crate::needs_you::clean_md;

/// Notes Loki may tick tasks in: today's and past daily notes, and sprint notes.
pub fn allowed_path(p: &str) -> bool {
    crate::profile::get().is_task_note(p)
}

/// Comparable form of a task's text: markdown cleaned, lowercase, and no whitespace at all (cards show "College : 2303" for
/// "College *(MIT 1)*: 2303").
fn norm(s: &str) -> String {
    clean_md(s).trim_end_matches('…').chars().filter(|c| !c.is_whitespace()).collect::<String>().to_lowercase()
}

/// `(done, text after the checkbox)` for a task line, or `None`.
fn parse(line: &str) -> Option<(bool, &str)> {
    let t = line.trim_start();
    if let Some(r) = t.strip_prefix("- [ ] ") {
        Some((false, r))
    } else {
        t.strip_prefix("- [x] ").or_else(|| t.strip_prefix("- [X] ")).map(|r| (true, r))
    }
}

/// A comment as it is written on the line: one line, no markdown that would break the task, short.
pub fn clean_comment(c: &str) -> String {
    c.replace(['\n', '\r', '*', '`'], " ").split_whitespace().collect::<Vec<_>>().join(" ").chars().take(300).collect()
}

/// Sets, clears or leaves a task's checkbox (`done`: `Some(true)` tick, `Some(false)` reopen, `None` comment only) and adds
/// `comment` (dated `stamp`, e.g. "10/27") to the end of the line. `text` is the task as shown (markdown cleaned).
/// `section` narrows the match when the same text appears under more than one heading.
pub fn mark(note: &str, text: &str, section: &str, done: Option<bool>, comment: &str, stamp: &str) -> Result<(String, String), String> {
    let want = norm(text);
    if want.is_empty() {
        return Err("no task text".into());
    }
    let mut sec = String::new();
    let mut in_tasks = false;
    let mut hits: Vec<usize> = Vec::new();
    let lines: Vec<&str> = note.lines().collect();
    for (i, l) in lines.iter().enumerate() {
        if let Some(h) = l.strip_prefix("## ") {
            // only the task areas: a daily note's tasks and habits, a sprint note's backlog (headings come from the profile)
            in_tasks = {
                let p = crate::profile::get();
                [p.daily.sections.tasks.as_str(), p.daily.sections.savers.as_str(), p.week.sections.backlog.as_str()].iter().any(|a| h.trim().starts_with(*a))
            };
            sec.clear();
        }
        if let Some(h) = l.strip_prefix("### ") {
            sec = h.trim().to_string();
        }
        if !in_tasks {
            continue;
        }
        if let Some((_, rest)) = parse(l) {
            let have = norm(rest);
            // the text shown on a card may be shortened, so a prefix of the line also matches
            if have == want || (want.chars().count() >= 10 && have.starts_with(&want)) {
                if section.is_empty() || sec.eq_ignore_ascii_case(section) {
                    hits.push(i);
                }
            }
        }
    }
    let at = match hits.as_slice() {
        [one] => *one,
        [] => return Err("couldn't find that task in the note (it may have been edited or moved)".into()),
        _ => return Err("that task appears more than once in the note, so I won't guess which to change".into()),
    };
    let line = lines[at];
    let (was_done, _) = parse(line).unwrap();
    let mut new = line.to_string();
    match done {
        Some(true) if !was_done => new = new.replacen("- [ ] ", "- [x] ", 1),
        Some(false) if was_done => new = new.replacen("- [x] ", "- [ ] ", 1).replacen("- [X] ", "- [ ] ", 1),
        _ => {}
    }
    let c = clean_comment(comment);
    if !c.is_empty() {
        new = format!("{} *({stamp}: {c})*", new.trim_end());
    }
    if new == line {
        return Ok((note.to_string(), line.to_string()));
    }
    let mut out: Vec<String> = lines.iter().map(|s| s.to_string()).collect();
    out[at] = new.clone();
    let mut text_out = out.join("\n");
    if note.ends_with('\n') {
        text_out.push('\n');
    }
    // safety: only that one line may differ
    let changed = note.lines().zip(text_out.lines()).filter(|(a, b)| a != b).count();
    if changed != 1 || note.lines().count() != text_out.lines().count() {
        return Err("stopped: the edit would have changed more than one line. Nothing was written.".into());
    }
    Ok((text_out, new))
}

/// The routine to-dos every day and every sprint should carry (`/bod` and `/eod`, `/bow` and `/eow`), added to the
/// note if they're missing: under a routines heading at the top of the tasks section (daily) or at the top of
/// the sprint backlog's sections (sprint); the heading names come from the profile. Returns the new text, or `None` if nothing was missing. Every original
/// line is kept in order.
pub fn ensure_routines(note: &str, kind: &str) -> Option<String> {
    let p = crate::profile::get();
    let (area, routines_h) = if kind == "week" { (format!("## {}", p.week.sections.backlog), format!("### {}", p.week.sections.routines)) } else { (format!("## {}", p.daily.sections.tasks), format!("### {}", p.daily.sections.routines)) };
    // the to-dos come from the routine registry in the profile (those with a `todo` and a daily or weekly period)
    let cmds = p.routine_todos(kind);
    if cmds.is_empty() {
        return None;
    }
    let lines: Vec<&str> = note.lines().collect();
    let area_i = lines.iter().position(|l| l.trim_end() == area.as_str())?;
    let area_end = (area_i + 1..lines.len()).find(|&i| lines[i].starts_with("## ")).unwrap_or(lines.len());
    let has = |cmd: &str| lines[area_i..area_end].iter().any(|l| parse(l).is_some_and(|(_, r)| r.trim_start().starts_with(cmd)));
    let missing: Vec<String> = cmds.iter().filter(|(c, _)| !has(c.as_str())).map(|(c, w)| format!("- [ ] {c} — {w}")).collect();
    if missing.is_empty() {
        return None;
    }
    let mut out: Vec<String> = lines.iter().map(|s| s.to_string()).collect();
    if let Some(h) = (area_i + 1..area_end).find(|&i| lines[i].trim() == routines_h) {
        let e = (h + 1..area_end).find(|&i| lines[i].starts_with('#')).unwrap_or(area_end);
        let at = (h + 1..e).rev().find(|&i| !lines[i].trim().is_empty()).map_or(h + 1, |i| i + 1);
        for (k, m) in missing.iter().enumerate() {
            out.insert(at + k, m.clone());
        }
    } else {
        let at = if kind == "week" { (area_i + 1..area_end).find(|&i| lines[i].starts_with("### ")).unwrap_or(area_end) } else { area_i + 1 + usize::from(lines.get(area_i + 1).is_some_and(|l| l.trim().is_empty())) };
        let mut block = vec![routines_h.clone()];
        block.extend(missing);
        block.push(String::new());
        for (k, b) in block.into_iter().enumerate() {
            out.insert(at + k, b);
        }
    }
    let mut text = out.join("\n");
    if note.ends_with('\n') {
        text.push('\n');
    }
    crate::extract::preserves(note, &text).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTE: &str = "# 2026-10-27\n\n## Tasks\n\n### School\n- [ ] College *(MIT 1)*: 2303 · 2304\n- [x] Read the brief\n\n### Home\n- [ ] Evening reset + Endel\n- [ ] Read the brief\n\n## Captures\n- [ ] not a task\n";

    #[test]
    fn ticks_exactly_one_line_and_keeps_the_rest() {
        let (n, line) = mark(NOTE, "College : 2303 · 2304", "", Some(true), "", "10/27").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(line, "- [x] College *(MIT 1)*: 2303 · 2304");
        assert_eq!(NOTE.lines().zip(n.lines()).filter(|(a, b)| a != b).count(), 1);
        assert!(n.ends_with('\n'));
    }

    #[test]
    fn adds_a_dated_comment_and_can_reopen() {
        let (n, line) = mark(NOTE, "Evening reset + Endel", "", Some(true), "did it *early* after dinner\nwith Alex", "10/27").unwrap();
        assert_eq!(line, "- [x] Evening reset + Endel *(10/27: did it early after dinner with Alex)*");
        let (back, l2) = mark(&n, "Evening reset + Endel (10/27: did it early after dinner with Alex)", "", Some(false), "", "10/27").unwrap();
        assert!(l2.starts_with("- [ ] Evening reset + Endel"));
        assert!(back.contains("- [ ] Evening reset + Endel"));
        let (c, l3) = mark(NOTE, "Evening reset + Endel", "", None, "pushed to tomorrow", "10/27").unwrap();
        assert!(l3.starts_with("- [ ] Evening reset + Endel *(10/27: pushed to tomorrow)*") && c.contains("pushed to tomorrow"));
    }

    #[test]
    fn refuses_to_guess() {
        assert!(mark(NOTE, "Read the brief", "", Some(true), "", "x").unwrap_err().contains("more than once"));
        let (_, l) = mark(NOTE, "Read the brief", "Home", Some(true), "", "x").unwrap();
        assert_eq!(l, "- [x] Read the brief");
        assert!(mark(NOTE, "something that isn't there", "", Some(true), "", "x").is_err());
        assert!(mark(NOTE, "not a task", "", Some(true), "", "x").is_err(), "checkboxes outside the Tasks area are never touched");
        assert!(mark(NOTE, "", "", Some(true), "", "x").is_err());
    }

    #[test]
    fn adds_the_routine_todos_once_and_keeps_everything() {
        let day = "---\nx: 1\n---\n# D\n\n## SAVERS\n- [ ] Silence\n\n## Tasks\n\n### School\n- [ ] A\n\n## Captures\n";
        let a = ensure_routines(day, "day").unwrap();
        assert!(a.contains("## Tasks\n\n### Routines\n- [ ] /bod — start the day\n- [ ] /eod — close the day\n\n### School"));
        assert!(crate::extract::preserves(day, &a));
        assert!(ensure_routines(&a, "day").is_none(), "second run adds nothing");
        // one present (ticked), one missing: only the missing one is added, inside the existing section
        let half = day.replace("## Tasks\n", "## Tasks\n### Routines\n- [x] /bod — start the day\n");
        let b = ensure_routines(&half, "day").unwrap();
        assert!(b.contains("- [x] /bod — start the day\n- [ ] /eod — close the day"));
        assert_eq!(b.matches("/bod").count(), 1);
        let week = "# W\n\n## Sprint Backlog\n**Active this sprint:**\n1. x\n\n### Standing Commitments\n- [ ] y\n\n## Sprint Review\n";
        let w = ensure_routines(week, "week").unwrap();
        assert!(w.contains("1. x\n\n### Routines\n- [ ] /bow — plan the week\n- [ ] /eow — close the week\n\n### Standing Commitments"));
        assert!(ensure_routines("# no task area\n", "day").is_none());
    }

    #[test]
    fn savers_checkboxes_can_be_ticked() {
        let n = "## SAVERS\n- [ ] Silence\n- [ ] Reading\n\n## Captures\n- [ ] Silence\n";
        let (out, line) = mark(n, "Silence", "", Some(true), "", "x").unwrap();
        assert_eq!(line, "- [x] Silence");
        assert_eq!(out.matches("- [x] Silence").count(), 1);
    }

    #[test]
    fn only_daily_and_sprint_notes_are_allowed() {
        assert!(allowed_path("10 - Daily/2026-10-27.md") && allowed_path("20 - Weeks/2026-W44.md"));
        for bad in ["30 - Backlog/Home/x.md", "10 - Daily/../AGENTS.md", "10 - Daily/sub/x.md", "10 - Daily/x.txt", "AI Infrastructure/bod.md"] {
            assert!(!allowed_path(bad), "{bad}");
        }
    }
}
