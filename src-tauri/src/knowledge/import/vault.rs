use crate::knowledge::{IngestDoc, KnowledgeStore};
use crate::obsidian::ObsidianClient;

#[derive(Debug, Default)]
pub struct ImportSummary {
    pub notes_seen: usize,
    pub notes_ingested: usize,
    pub errors: Vec<String>,
}

/// Walks the whole vault via the *existing* `ObsidianClient` — reusing vault reading rather
/// than building a second path to it — and ingests every markdown note into the knowledge
/// store as `source_type: "vault_note"`, idempotent by `source_id` = vault path (re-running
/// this safely replaces rather than duplicates, same as any other ingest).
///
/// Includes `99 - Archive/` — an earlier version of this skipped it, matching the
/// vault-search-mcp reference implementation's own convention, but the user's own archive is
/// still real knowledge worth being able to search (closed-out projects, completed backlog
/// items), not something to silently drop just because the vault's own workflow moved it out
/// of active status. If a future need arises to exclude it again, that should be a real
/// `QueryFilter`/`source_type` distinction made at query time, not an ingest-time omission —
/// the difference matters because the former is reversible and inspectable, the latter just
/// makes the data disappear.
///
/// Deliberately **not called from anywhere in the app yet** — this is a library function for
/// the user to invoke deliberately (a CLI flag, a future admin action, or directly), not
/// something that runs automatically on startup. Importing his entire real vault is a real
/// action on his actual life-management system; per the migration plan's own verification
/// step, it should be run against a copy first, not fired off unattended.
pub async fn import_vault(obsidian: &ObsidianClient, knowledge: &dyn KnowledgeStore) -> anyhow::Result<ImportSummary> {
    let paths = list_all_markdown_files(obsidian).await?;
    let mut summary = ImportSummary { notes_seen: paths.len(), ..Default::default() };

    for path in paths {
        match import_one_note(obsidian, knowledge, &path).await {
            Ok(()) => summary.notes_ingested += 1,
            Err(e) => summary.errors.push(format!("{path}: {e}")),
        }
    }
    Ok(summary)
}

/// Re-ingests a single note — the same operation `import_vault` does per-file, exposed
/// separately so the incremental re-sync path (re-ingest on Loki's own `read_file`/
/// `append_content` calls — see `agent::tools`) doesn't need a full vault walk.
pub async fn import_one_note(obsidian: &ObsidianClient, knowledge: &dyn KnowledgeStore, path: &str) -> anyhow::Result<()> {
    let content = obsidian.read_file(path).await?;
    ingest_note_content(knowledge, path, &content, None).await
}

/// Split out from `import_one_note` so callers that already have the note's content in hand
/// (e.g. `AppendContent`'s tool, which just wrote it and knows what it wrote) don't need a
/// redundant `read_file` round trip just to re-ingest.
///
/// `session_id` is `None` for ordinary vault notes (the vast majority of callers) and
/// `Some(...)` only for `conversation_log`'s re-ingest of a `98 - Loki Conversations/*.md`
/// file — letting `agent::build_context_window`'s existing session-scoped query keep working
/// unchanged even though conversation history is now vault-note-backed, not a separate
/// `chat_message` source type.
pub async fn ingest_note_content(
    knowledge: &dyn KnowledgeStore,
    path: &str,
    content: &str,
    session_id: Option<&str>,
) -> anyhow::Result<()> {
    let (tags, title, body) = parse_frontmatter(content, path);
    knowledge
        .ingest(IngestDoc {
            source_type: "vault_note".to_string(),
            source_id: path.to_string(),
            title: Some(title),
            content: body,
            tags,
            session_id: session_id.map(|s| s.to_string()),
        })
        .await?;
    Ok(())
}

/// `pub` (not just crate-private) so `knowledge::reconcile` can reuse it for the "does this
/// path still exist" sweep, rather than duplicating vault directory-walking a second time.
pub async fn list_all_markdown_files(obsidian: &ObsidianClient) -> anyhow::Result<Vec<String>> {
    let mut out = Vec::new();
    // Each stack entry is a directory path already ending in "/" ("" for the vault root).
    let mut stack: Vec<String> = vec![String::new()];
    while let Some(prefix) = stack.pop() {
        let query_dir = if prefix.is_empty() { None } else { Some(prefix.as_str()) };
        let entries = obsidian.list_files(query_dir).await?;
        for entry in entries {
            let full_path = format!("{prefix}{entry}");
            if entry.ends_with('/') {
                stack.push(full_path);
            } else if entry.ends_with(".md") {
                out.push(full_path);
            }
        }
    }
    Ok(out)
}

/// Extracts YAML frontmatter tags + a title, and returns the body with frontmatter stripped
/// (so embeddings are computed over actual content, not metadata). Assumes LF line endings
/// (`---\n...\n---\n`) — true for every note in this vault, generated by Obsidian on Linux;
/// not attempting CRLF handling since it doesn't occur here and getting the byte-offset math
/// wrong for a case that can't happen isn't worth it.
///
/// Handles both tag conventions the vault's own CLAUDE.md documents: inline
/// (`tags: [a, b, c]`) and block-list (`tags:\n  - a\n  - b`). This is the fix for
/// vault-search-mcp's documented gap, where frontmatter (and therefore tags) was stripped
/// and discarded before chunking, never captured anywhere.
fn parse_frontmatter(content: &str, path: &str) -> (Vec<String>, String, String) {
    let (tags, body) = match content.strip_prefix("---\n").and_then(|rest| rest.find("\n---\n").map(|end| (rest, end))) {
        Some((rest, end)) => (extract_tags(&rest[..end]), rest[end + "\n---\n".len()..].to_string()),
        None => (Vec::new(), content.to_string()),
    };

    let title = extract_title(&body)
        .unwrap_or_else(|| path.rsplit('/').next().unwrap_or(path).trim_end_matches(".md").to_string());

    (tags, title, body)
}

fn extract_tags(frontmatter: &str) -> Vec<String> {
    let lines: Vec<&str> = frontmatter.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        let Some(rest) = line.trim().strip_prefix("tags:") else { continue };
        let rest = rest.trim();

        if let Some(inline) = rest.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            return inline
                .split(',')
                .map(|t| t.trim().trim_matches('"').trim_matches('\'').to_string())
                .filter(|t| !t.is_empty())
                .collect();
        }

        if rest.is_empty() {
            let mut out = Vec::new();
            for next in &lines[i + 1..] {
                let nt = next.trim();
                match nt.strip_prefix("- ") {
                    Some(item) => out.push(item.trim().to_string()),
                    None if nt.is_empty() => continue,
                    None => break, // next top-level frontmatter key — stop
                }
            }
            return out;
        }
    }
    Vec::new()
}

fn extract_title(body: &str) -> Option<String> {
    body.lines().find_map(|l| l.trim().strip_prefix("# ").map(|t| t.trim().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_inline_tags() {
        let content = "---\ntags: [action, knowledge]\nestimate: 1h\n---\n# My Note\n\nBody text.";
        let (tags, title, body) = parse_frontmatter(content, "30 - Backlog/Tech/My Note.md");
        assert_eq!(tags, vec!["action", "knowledge"]);
        assert_eq!(title, "My Note");
        assert_eq!(body, "# My Note\n\nBody text.");
    }

    #[test]
    fn parses_block_list_tags() {
        let content = "---\ntags:\n  - training\n  - semperfit\n  - stim\nestimate:\n---\nSession notes here.";
        let (tags, _title, _body) = parse_frontmatter(content, "15 - Training/2026-08-24.md");
        assert_eq!(tags, vec!["training", "semperfit", "stim"]);
    }

    #[test]
    fn falls_back_to_filename_when_no_heading() {
        let content = "---\ntags: [x]\n---\nJust a body, no heading.";
        let (_tags, title, _body) = parse_frontmatter(content, "10 - Daily/2026-08-24.md");
        assert_eq!(title, "2026-08-24");
    }

    #[test]
    fn note_with_no_frontmatter_still_parses() {
        let content = "# Plain Note\n\nNo frontmatter at all.";
        let (tags, title, body) = parse_frontmatter(content, "50 - Notes/Plain Note.md");
        assert!(tags.is_empty());
        assert_eq!(title, "Plain Note");
        assert_eq!(body, content);
    }
}
