//! Notes: the user writes markdown in Loki; the local model files it into the vault, either as its own note or as an
//! addition to existing notes (his choice, or the model's suggestion). The model only PROPOSES; nothing is written
//! until he approves each item. New notes never overwrite; additions only ever add to the end of a note.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct NoteProposal {
    /// "create" | "append"
    pub op: String,
    pub path: String,
    /// Create: the whole note. Append: the markdown to add.
    pub content: String,
    /// Append only: a heading to put the addition under.
    #[serde(default)]
    pub heading: String,
    #[serde(default)]
    pub why: String,
}

/// A vault note Loki may create or add to: inside a folder, never in the archive, templates, AI instructions, daily notes or Obsidian's own files.
pub fn allowed_path(p: &str) -> bool {
    let blocked = crate::profile::get().protected_prefixes();
    p.ends_with(".md") && p.len() > 3 && p.len() < 200 && p.contains('/') && !p.contains("..") && !p.starts_with('/') && !p.contains('\\') && !blocked.iter().any(|b| p.starts_with(b.as_str()))
}

pub fn sanitize(mut p: NoteProposal) -> Option<NoteProposal> {
    p.op = p.op.to_lowercase();
    p.path = p.path.trim().to_string();
    p.content = p.content.trim().to_string();
    p.heading = p.heading.trim().trim_start_matches('#').trim().chars().take(80).collect();
    p.why = p.why.chars().take(240).collect();
    if !matches!(p.op.as_str(), "create" | "append") || !allowed_path(&p.path) || p.content.is_empty() || p.content.len() > 40_000 {
        return None;
    }
    Some(p)
}

pub fn parse(reply: &str) -> Vec<NoteProposal> {
    let (Some(a), Some(b)) = (reply.find('['), reply.rfind(']')) else { return Vec::new() };
    let raw: Vec<Value> = serde_json::from_str(&reply[a..=b]).unwrap_or_default();
    raw.iter().take(6).filter_map(|v| serde_json::from_value::<NoteProposal>(v.clone()).ok()).filter_map(sanitize).collect()
}

/// `(system, user)` for the model. `mode`: new | update | auto. `candidates`: (path, start of the note).
pub fn prompt(title: &str, text: &str, mode: &str, candidates: &[(String, String)], today: &str) -> (String, String) {
    let task = match mode {
        "new" => "Turn the note into exactly ONE new vault note (op \"create\").",
        "update" => "Fold the note into the EXISTING notes listed below (op \"append\"), one entry per note that should get an addition. Do not create new notes.",
        _ => "Decide what is best: a new note (op \"create\"), additions to the existing notes listed below (op \"append\"), or both. Prefer adding to an existing note when the content clearly belongs there; otherwise create one.",
    };
    let v = &crate::profile::get().vault;
    let (inbox, notes, projects) = (&v.inbox, &v.notes, &v.projects);
    let mut system = format!(
        "You file the user's personal notes into their Obsidian vault. Today is {today}.\n{task}\n\
Keep his meaning and voice; clean up structure and markdown, never invent facts, and do not add commentary.\n\
Reply with ONLY a JSON array. Each element: {{\"op\":\"create\"|\"append\",\"path\":\"Folder/Name.md\",\"content\":\"markdown\",\"heading\":\"optional heading for an append\",\"why\":\"one short line\"}}.\n\
- create: content is the whole note and starts with frontmatter (--- tags: [knowledge] --- or tags: [action] for a task). New notes go in \"{inbox}/\" unless one is clearly a permanent atomic idea (\"{notes}/\", title states the idea) or belongs to a project (\"{projects}/<Project>/\"). Use a plain descriptive file name.\n\
- append: path must be one of the existing notes below; content is only the new text to add at the end of that note (not the existing text). Link related notes as [[Note Name]] where it helps.\n\
- Use \\n for line breaks inside JSON strings."
    );
    if mode == "new" {
        system.push_str("\nReturn exactly one element.");
    }
    let mut user = format!("Note title: {}\n\nNote:\n{}\n", if title.trim().is_empty() { "(none)" } else { title.trim() }, text);
    if mode != "new" {
        if candidates.is_empty() {
            user.push_str("\nExisting notes: none found.\n");
        } else {
            user.push_str("\nExisting notes:\n");
            for (p, excerpt) in candidates {
                user.push_str(&format!("\n### {p}\n{excerpt}\n"));
            }
        }
    }
    (system, user)
}

/// The new text of an existing note with an addition at the end; the old text is untouched.
pub fn appended(old: &str, add: &str, heading: &str) -> String {
    let mut s = old.trim_end().to_string();
    s.push_str("\n\n");
    if !heading.is_empty() {
        s.push_str(&format!("## {heading}\n"));
    }
    s.push_str(add.trim());
    s.push('\n');
    s
}

/// A new note always opens with frontmatter (the Inbox convention).
pub fn with_frontmatter(content: &str) -> String {
    if content.starts_with("---\n") {
        content.trim_end().to_string() + "\n"
    } else {
        format!("---\ntags: [knowledge]\n---\n{}\n", content.trim())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_limited_to_notes_loki_may_touch() {
        assert!(allowed_path("00 - Inbox/Idea.md"));
        assert!(allowed_path("40 - Projects/Loki/Plan.md"));
        for bad in ["99 - Archive/x.md", "90 - Templates/x.md", "AI Infrastructure/x.md", "10 - Daily/2026-10-28.md", "../x.md", "/etc/x.md", "Idea.md", "00 - Inbox/x.txt"] {
            assert!(!allowed_path(bad), "{bad}");
        }
    }

    #[test]
    fn parses_proposals_and_drops_bad_ones() {
        let r = "Here:\n[{\"op\":\"create\",\"path\":\"00 - Inbox/A.md\",\"content\":\"---\\ntags: [knowledge]\\n---\\nhi\",\"why\":\"x\"},{\"op\":\"append\",\"path\":\"99 - Archive/B.md\",\"content\":\"no\"},{\"op\":\"delete\",\"path\":\"50 - Notes/C.md\",\"content\":\"y\"}]";
        let p = parse(r);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].path, "00 - Inbox/A.md");
        assert!(parse("no json").is_empty());
    }

    #[test]
    fn append_keeps_old_text() {
        let a = appended("# T\n\nold\n", "new bit", "Update");
        assert_eq!(a, "# T\n\nold\n\n## Update\nnew bit\n");
        assert!(crate::extract::preserves("# T\n\nold\n", &a));
        assert_eq!(appended("x", "y", ""), "x\n\ny\n");
    }

    #[test]
    fn new_notes_get_frontmatter() {
        assert_eq!(with_frontmatter("hello"), "---\ntags: [knowledge]\n---\nhello\n");
        assert_eq!(with_frontmatter("---\ntags: [action]\n---\nx"), "---\ntags: [action]\n---\nx\n");
    }
}
