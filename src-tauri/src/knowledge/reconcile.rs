//! Cleans up `knowledge_items` rows left behind when a vault note is deleted.
//!
//! Deletion never happens through Loki's own process: `ObsidianClient::delete_file` has no
//! callers anywhere in this codebase, no delete tool is registered in Loki's own
//! `ToolRegistry`, and Routine Mode's `TIER01_ALLOWED_TOOLS` explicitly excludes
//! `obsidian_delete_file` (Tier 2). Notes only get deleted via Claude calling the raw Obsidian
//! MCP tool directly in an interactive session — entirely outside Loki's `Core`. There's no
//! event to hook, so this is a periodic reconciliation sweep instead: list every vault path
//! Loki's index thinks exists, list every markdown file that actually exists, and remove the
//! difference. See `Core::spawn_knowledge_reconciliation` for how this gets scheduled.

use super::KnowledgeStore;
use crate::knowledge::import::vault::list_all_markdown_files;
use crate::obsidian::ObsidianClient;
use std::collections::HashSet;

/// Returns the number of orphaned rows removed.
pub async fn reconcile_deleted_notes(knowledge: &dyn KnowledgeStore, obsidian: &ObsidianClient) -> anyhow::Result<usize> {
    let indexed = knowledge.vault_note_paths().await?;
    if indexed.is_empty() {
        return Ok(0);
    }
    let live: HashSet<String> = list_all_markdown_files(obsidian).await?.into_iter().collect();

    let mut removed = 0;
    for path in indexed {
        if !live.contains(&path) {
            knowledge.delete_by_source("vault_note", &path).await?;
            removed += 1;
        }
    }
    Ok(removed)
}
