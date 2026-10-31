pub mod chunking;
pub mod import;
pub mod reconcile;
pub mod sqlite_store;
pub mod tags;

pub use sqlite_store::SqliteKnowledgeStore;

use async_trait::async_trait;
use serde::Serialize;

/// Scoping for a query — restrict to a source type (e.g. only vault notes, not chat history)
/// or a chat session (only this conversation's prior turns).
#[derive(Debug, Clone, Default)]
pub struct QueryFilter {
    pub source_type: Option<String>,
    pub session_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SearchResult {
    pub id: String,
    pub title: Option<String>,
    pub snippet: String,
    pub distance: f64,
    pub tags: Vec<String>,
}

/// A document to ingest — one note, one chat message, one imported/connected item.
/// `source_type` + `source_id` together are the idempotency key (mirrors Muninn's
/// idempotent-by-source-id ingest pattern): re-ingesting the same pair replaces rather than
/// duplicates. `session_id` is set for `source_type: "chat_message"` rows so a query can be
/// scoped to "this conversation" — `None` for everything else (vault notes, imports).
pub struct IngestDoc {
    pub source_type: String,
    pub source_id: String,
    pub title: Option<String>,
    pub content: String,
    pub tags: Vec<String>,
    pub session_id: Option<String>,
}

/// A swappable knowledge/context store.
#[async_trait]
pub trait KnowledgeStore: Send + Sync {
    async fn query(&self, query: &str, n: u32, filter: QueryFilter) -> anyhow::Result<Vec<SearchResult>>;

    /// Returns the chunk IDs written, for later deletion — same bookkeeping pattern
    /// Muninn uses (store chunk IDs on the owning row so a delete/update knows what
    /// to remove from the vector index without a full re-scan).
    async fn ingest(&self, doc: IngestDoc) -> anyhow::Result<Vec<String>>;

    async fn delete(&self, chunk_ids: &[String]) -> anyhow::Result<()>;

    /// All vault paths currently indexed as `source_type: "vault_note"` — used by
    /// `reconcile::reconcile_deleted_notes` to find rows whose backing note no longer exists
    /// (deletion happens entirely outside Loki's process, via the raw Obsidian MCP tool from
    /// an interactive Claude Code session — see that module's doc comment).
    async fn vault_note_paths(&self) -> anyhow::Result<Vec<String>>;

    /// Removes a single item (its `knowledge_items` row *and* its chunks/vectors) by
    /// `source_type`/`source_id` — unlike `delete`, which only takes already-known chunk ids
    /// (the "replace on re-ingest" case), this is the "the source is just gone" case and needs
    /// to look up and remove the item row itself too, or it'd stay orphaned same as today.
    async fn delete_by_source(&self, source_type: &str, source_id: &str) -> anyhow::Result<()>;
}
