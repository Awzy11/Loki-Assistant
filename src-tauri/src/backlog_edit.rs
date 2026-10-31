//! Edits to backlog items from the Backlog dashboard: comment, rewrite, set a status, or close one
//! (done / superseded / dropped) by moving it to the archive folder the way the vault's archive convention says.
//! Folder names come from the profile (`profile.rs`).
//! Pure text functions here; `Core` does the reads and writes.

/// `<backlog>/<area>/<Title>.md` (or `<backlog>/<Title>.md` for a flat backlog) and nothing else.
pub fn allowed_path(p: &str) -> bool {
    crate::profile::get().is_backlog_item(p)
}

/// Splits the frontmatter block off: `(lines between the --- fences, body after them)`.
fn split(text: &str) -> Option<(Vec<String>, &str)> {
    let rest = text.strip_prefix("---\n")?;
    let end = rest.find("\n---")?;
    let fm: Vec<String> = rest[..end].lines().map(String::from).collect();
    let after = &rest[end + 4..];
    Some((fm, after.strip_prefix('\n').unwrap_or(after)))
}

/// Sets one scalar frontmatter field, replacing the line (and any block list under it) or adding it before the closing fence.
pub fn set_field(text: &str, key: &str, value: &str) -> String {
    let line = if value.is_empty() { format!("{key}:") } else { format!("{key}: {value}") };
    let Some((fm, body)) = split(text) else { return format!("---\n{line}\n---\n{text}") };
    let mut out: Vec<String> = Vec::new();
    let mut done = false;
    let mut skipping = false;
    for l in fm {
        if skipping && (l.starts_with("  ") || l.starts_with("- ") || l.starts_with('\t')) {
            continue;
        }
        skipping = false;
        if l.split_once(':').is_some_and(|(k, _)| k.trim() == key) {
            out.push(line.clone());
            done = true;
            skipping = true;
            continue;
        }
        out.push(l);
    }
    if !done {
        out.push(line);
    }
    format!("---\n{}\n---\n{body}", out.join("\n"))
}

/// Adds a dated bullet under `## Comments`, creating the section at the end of the note if it isn't there.
pub fn add_comment(text: &str, stamp: &str, comment: &str) -> Result<String, String> {
    let c = comment.split_whitespace().collect::<Vec<_>>().join(" ");
    if c.is_empty() {
        return Err("Write a comment first.".into());
    }
    let bullet = format!("- {stamp}: {c}");
    let lines: Vec<&str> = text.lines().collect();
    let Some(h) = lines.iter().position(|l| l.trim() == "## Comments") else {
        return Ok(format!("{}\n\n## Comments\n{bullet}\n", text.trim_end()));
    };
    let mut end = lines.len();
    for (i, l) in lines.iter().enumerate().skip(h + 1) {
        if l.starts_with("## ") || l.starts_with("# ") {
            end = i;
            break;
        }
    }
    // after the last non-blank line of the section
    let mut at = end;
    while at > h + 1 && lines[at - 1].trim().is_empty() {
        at -= 1;
    }
    let mut out: Vec<String> = lines[..at].iter().map(|s| s.to_string()).collect();
    out.push(bullet);
    if at < end || end < lines.len() {
        out.push(String::new());
    }
    out.extend(lines[end..].iter().map(|s| s.to_string()));
    Ok(out.join("\n").trim_end().to_string() + "\n")
}

/// Appends a week link to the `Linked:` line (adds the line if absent).
fn link_week(text: &str, week: &str) -> String {
    let link = format!("[[{week}]]");
    if text.contains(&link) {
        return text.to_string();
    }
    let mut lines: Vec<String> = text.lines().map(String::from).collect();
    if let Some(l) = lines.iter_mut().rev().find(|l| l.starts_with("Linked:")) {
        l.push_str(&format!(" · {link}"));
    } else {
        lines.push(String::new());
        lines.push(format!("Linked: {link}"));
    }
    lines.join("\n") + "\n"
}

/// The note as it is filed in the archive: `tags: [action, archived]`, `status`, the date fields, the week linked.
/// `status` is `done`, `superseded` or `dropped`; `note` (why / by what) goes in as a Comments bullet.
pub fn archived(text: &str, status: &str, today: &str, week: &str, note: &str) -> Result<String, String> {
    if !matches!(status, "done" | "superseded" | "dropped") {
        return Err("Status must be done, superseded or dropped to close an item.".into());
    }
    let mut t = set_field(text, "tags", "[action, archived]");
    t = set_field(&t, "status", status);
    if status == "done" {
        t = set_field(&t, "Completed", today);
    } else {
        t = set_field(&t, "Closed", today);
    }
    t = set_field(&t, "Archived", today);
    if !note.trim().is_empty() {
        let label = if status == "superseded" { "Superseded" } else if status == "dropped" { "Dropped" } else { "Done" };
        t = add_comment(&t, &today[5..].replace('-', "/"), &format!("{label}: {}", note.trim()))?;
    }
    Ok(link_week(&t, week))
}

/// A whole-note rewrite is only accepted if it still has frontmatter when the original did, and isn't empty.
pub fn rewrite_ok(old: &str, new: &str) -> Result<(), String> {
    if new.trim().is_empty() {
        return Err("The note can't be saved empty.".into());
    }
    if old.starts_with("---\n") && !new.starts_with("---\n") {
        return Err("Keep the frontmatter block (the lines between --- at the top).".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTE: &str = "---\ntags: [action]\nestimate: 2h\nnext_action:\n---\n# Thing\n\nBody.\n\nLinked: [[2026-10-20]]\n";

    #[test]
    fn paths() {
        assert!(allowed_path("30 - Backlog/Tech/Thing.md"));
        assert!(!allowed_path("30 - Backlog/Thing.md"));
        assert!(!allowed_path("30 - Backlog/Other/Thing.md"));
        assert!(!allowed_path("30 - Backlog/Tech/../../x.md"));
        assert!(!allowed_path("10 - Daily/2026-10-28.md"));
    }

    #[test]
    fn fields_replace_and_add() {
        let t = set_field(NOTE, "estimate", "3h");
        assert!(t.contains("estimate: 3h") && !t.contains("estimate: 2h"));
        let t = set_field(NOTE, "status", "blocked");
        assert!(t.contains("next_action:\nstatus: blocked\n---\n# Thing"));
        let t = set_field("---\ntags:\n  - action\n  - tech\nx: 1\n---\nbody\n", "tags", "[action, archived]");
        assert_eq!(t, "---\ntags: [action, archived]\nx: 1\n---\nbody\n");
    }

    #[test]
    fn comments_make_and_extend_the_section() {
        let t = add_comment(NOTE, "10/28", "waiting on a reply").unwrap();
        assert!(t.ends_with("Linked: [[2026-10-20]]\n\n## Comments\n- 10/28: waiting on a reply\n"));
        let t2 = add_comment(&t, "10/29", "got it").unwrap();
        assert!(t2.ends_with("## Comments\n- 10/28: waiting on a reply\n- 10/29: got it\n"));
        let mid = "# T\n\n## Comments\n- a\n\n## Resources\n- r\n";
        let t3 = add_comment(mid, "10/28", "b").unwrap();
        assert_eq!(t3, "# T\n\n## Comments\n- a\n- 10/28: b\n\n## Resources\n- r\n");
        assert!(add_comment(NOTE, "x", "  ").is_err());
    }

    #[test]
    fn archive_follows_the_convention() {
        let t = archived(NOTE, "done", "2026-10-28", "2026-W44", "").unwrap();
        assert!(t.contains("tags: [action, archived]") && t.contains("Completed: 2026-10-28") && t.contains("Archived: 2026-10-28") && t.contains("status: done"));
        assert!(t.contains("Linked: [[2026-10-20]] · [[2026-W44]]"));
        let s = archived(NOTE, "superseded", "2026-10-28", "2026-W44", "by [[New Thing]]").unwrap();
        assert!(s.contains("Closed: 2026-10-28") && s.contains("- 10/28: Superseded: by [[New Thing]]") && !s.contains("Completed"));
        assert!(archived(NOTE, "blocked", "2026-10-28", "2026-W44", "").is_err());
    }

    #[test]
    fn rewrites_keep_frontmatter() {
        assert!(rewrite_ok(NOTE, "# no fm").is_err());
        assert!(rewrite_ok(NOTE, "").is_err());
        assert!(rewrite_ok(NOTE, NOTE).is_ok());
    }
}
