use super::{chunking, tags, IngestDoc, KnowledgeStore, QueryFilter, SearchResult};
use crate::providers::LlmProvider;
use async_trait::async_trait;
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// The owned knowledge store: one SQLite file holding both relational metadata
/// (`knowledge_items`, `knowledge_chunk_meta`) and vector search (`knowledge_chunks`, a
/// `sqlite-vec` `vec0` virtual table) — no external Chroma server, no second embedded engine.
/// Handles both vault notes and chat messages through the same schema (`source_type`
/// distinguishes them), which is the whole point relative to the two single-content-type
/// reference implementations this was designed against (Muninn: notes-shaped only;
/// vault-search-mcp: even thinner, no tags/timestamps at all).
///
/// A `std::sync::Mutex<Connection>` guards the (blocking) rusqlite handle. Every method here
/// is careful to finish its embedding/tagging `.await`s *before* taking the lock, so the lock
/// is only ever held across synchronous SQLite calls — never across an `.await` point.
pub struct SqliteKnowledgeStore {
    conn: Mutex<Connection>,
    /// Produces vectors — whatever `[embedding]` is configured to (Phase 1).
    embedder: Arc<dyn LlmProvider>,
    /// Produces auto-tags via a small chat call — deliberately the *main chat* provider
    /// (`[llm]`), not `embedder`, since an embedding model (e.g. `nomic-embed-text`) isn't
    /// chat-capable. Both are `Arc<dyn LlmProvider>` so `Core` and this store can share the
    /// same instances instead of constructing the chat provider twice.
    tagger: Arc<dyn LlmProvider>,
}

impl SqliteKnowledgeStore {
    pub fn open(path: &Path, embedder: Arc<dyn LlmProvider>, tagger: Arc<dyn LlmProvider>) -> anyhow::Result<Self> {
        // Process-global registration (safe to call once per process) — every Connection
        // opened after this point gets the vec0 module for free, no per-connection loading.
        unsafe {
            rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(
                sqlite_vec::sqlite3_vec_init as *const (),
            )));
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        init_schema(&conn)?;
        Ok(Self { conn: Mutex::new(conn), embedder, tagger })
    }

    #[cfg(test)]
    fn open_in_memory(embedder: Arc<dyn LlmProvider>, tagger: Arc<dyn LlmProvider>) -> anyhow::Result<Self> {
        unsafe {
            rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(
                sqlite_vec::sqlite3_vec_init as *const (),
            )));
        }
        let conn = Connection::open_in_memory()?;
        init_schema(&conn)?;
        Ok(Self { conn: Mutex::new(conn), embedder, tagger })
    }
}

fn init_schema(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS knowledge_items (
            id TEXT PRIMARY KEY,
            source_type TEXT NOT NULL,
            source_id TEXT NOT NULL,
            title TEXT,
            content TEXT NOT NULL,
            tags TEXT NOT NULL DEFAULT '[]',
            owner TEXT NOT NULL DEFAULT 'user',
            session_id TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            chunk_ids TEXT NOT NULL DEFAULT '[]'
        );
        CREATE INDEX IF NOT EXISTS idx_knowledge_items_source ON knowledge_items(source_type, source_id);
        CREATE INDEX IF NOT EXISTS idx_knowledge_items_session ON knowledge_items(session_id);

        CREATE TABLE IF NOT EXISTS knowledge_chunk_meta (
            chunk_rowid INTEGER PRIMARY KEY,
            item_id TEXT NOT NULL,
            chunk_index INTEGER NOT NULL,
            snippet TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS knowledge_meta (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        ",
    )?;
    Ok(())
}

/// `knowledge_chunks` (the vec0 table) is created lazily, on first ingest, once we know the
/// embedding dimension in use — `vec0(embedding float[N])` needs a fixed N at creation time.
/// Recorded in `knowledge_meta` so a later dimension mismatch (e.g. switching embedding
/// models) is a clear error instead of a confusing SQLite failure deep in an INSERT.
fn ensure_vec_table(conn: &Connection, dim: usize) -> anyhow::Result<()> {
    let existing: Option<String> = conn
        .query_row("SELECT value FROM knowledge_meta WHERE key = 'embedding_dim'", [], |r| r.get(0))
        .optional()?;
    match existing {
        Some(v) => {
            let existing_dim: usize = v.parse().unwrap_or(0);
            if existing_dim != dim {
                anyhow::bail!(
                    "embedding dimension changed (was {existing_dim}, now {dim}) — the knowledge \
                     store's vector table is fixed-dimension. Delete the knowledge.db file and \
                     re-ingest if you've switched embedding models."
                );
            }
        }
        None => {
            conn.execute_batch(&format!(
                "CREATE VIRTUAL TABLE IF NOT EXISTS knowledge_chunks USING vec0(embedding float[{dim}]);"
            ))?;
            conn.execute("INSERT INTO knowledge_meta(key, value) VALUES ('embedding_dim', ?1)", params![
                dim.to_string()
            ])?;
        }
    }
    Ok(())
}

fn now_unix() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

#[async_trait]
impl KnowledgeStore for SqliteKnowledgeStore {
    async fn query(&self, query: &str, n: u32, filter: QueryFilter) -> anyhow::Result<Vec<SearchResult>> {
        let vector = self.embedder.embed(query).await?;
        let vector_json = serde_json::to_string(&vector)?;

        let conn = self.conn.lock().unwrap();

        let table_exists: bool = conn
            .query_row("SELECT 1 FROM sqlite_master WHERE type='table' AND name='knowledge_chunks'", [], |_| Ok(true))
            .optional()?
            .unwrap_or(false);
        if !table_exists {
            // Nothing has ever been ingested — an empty result, not an error.
            return Ok(Vec::new());
        }

        // Over-fetch chunks, then filter/dedupe-by-item in Rust — simpler and more portable
        // than folding arbitrary metadata filters into the vec0 MATCH query itself, and
        // matches Muninn's own "over-fetch then diversify" pattern (it hit and fixed a real
        // bug where one long document dominated every result slot).
        let fetch_n = (n as i64 * 6).max(30);
        let mut stmt = conn.prepare(
            "SELECT rowid, distance FROM knowledge_chunks WHERE embedding MATCH ?1 AND k = ?2 ORDER BY distance",
        )?;
        let rows: Vec<(i64, f64)> =
            stmt.query_map(params![vector_json, fetch_n], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
        drop(stmt);

        let mut results = Vec::new();
        let mut seen_items: HashSet<String> = HashSet::new();
        for (chunk_rowid, distance) in rows {
            let meta: Option<(String, String)> = conn
                .query_row(
                    "SELECT item_id, snippet FROM knowledge_chunk_meta WHERE chunk_rowid = ?1",
                    params![chunk_rowid],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            let Some((item_id, snippet)) = meta else { continue };

            let item: Option<(Option<String>, String, Option<String>, String)> = conn
                .query_row(
                    "SELECT title, source_type, session_id, tags FROM knowledge_items WHERE id = ?1",
                    params![item_id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .optional()?;
            let Some((title, source_type, session_id, tags_json)) = item else { continue };
            let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();

            if let Some(want) = &filter.source_type {
                if &source_type != want {
                    continue;
                }
            }
            if let Some(want) = &filter.session_id {
                if session_id.as_deref() != Some(want.as_str()) {
                    continue;
                }
            }
            if !seen_items.insert(item_id.clone()) {
                continue; // one result per source item, not per chunk
            }

            results.push(SearchResult { id: item_id, title, snippet, distance, tags });
            if results.len() >= n as usize {
                break;
            }
        }

        Ok(results)
    }

    async fn ingest(&self, doc: IngestDoc) -> anyhow::Result<Vec<String>> {
        let chunks = chunking::chunk_text(&doc.content);
        if chunks.is_empty() {
            return Ok(Vec::new());
        }

        // All async work (embedding, tagging) happens before the lock is taken — see the
        // struct's doc comment on why.
        let mut embeddings = Vec::with_capacity(chunks.len());
        for chunk in &chunks {
            embeddings.push(self.embedder.embed(chunk).await?);
        }
        let tags = if doc.tags.is_empty() { tags::auto_tag(&self.tagger, &doc.content).await } else { doc.tags.clone() };

        let dim = embeddings[0].len();
        let item_id = format!("{}:{}", doc.source_type, doc.source_id);
        let now = now_unix();
        let tags_json = serde_json::to_string(&tags)?;

        let conn = self.conn.lock().unwrap();
        ensure_vec_table(&conn, dim)?;

        // Idempotent-by-source-id (mirrors Muninn): drop any chunks from a previous ingest of
        // this same item before writing new ones, so re-ingesting replaces, not duplicates.
        let existing_chunk_ids: Option<String> =
            conn.query_row("SELECT chunk_ids FROM knowledge_items WHERE id = ?1", params![item_id], |r| r.get(0)).optional()?;
        if let Some(json) = existing_chunk_ids {
            if let Ok(ids) = serde_json::from_str::<Vec<String>>(&json) {
                for id in ids {
                    conn.execute("DELETE FROM knowledge_chunks WHERE rowid = ?1", params![id])?;
                    conn.execute("DELETE FROM knowledge_chunk_meta WHERE chunk_rowid = ?1", params![id])?;
                }
            }
        }

        let mut chunk_ids = Vec::with_capacity(chunks.len());
        for (i, (chunk, vector)) in chunks.iter().zip(embeddings.iter()).enumerate() {
            let vector_json = serde_json::to_string(vector)?;
            conn.execute("INSERT INTO knowledge_chunks(embedding) VALUES (?1)", params![vector_json])?;
            let chunk_rowid = conn.last_insert_rowid();
            let snippet: String = chunk.chars().take(400).collect();
            conn.execute(
                "INSERT INTO knowledge_chunk_meta(chunk_rowid, item_id, chunk_index, snippet) VALUES (?1, ?2, ?3, ?4)",
                params![chunk_rowid, item_id, i as i64, snippet],
            )?;
            chunk_ids.push(chunk_rowid.to_string());
        }
        let chunk_ids_json = serde_json::to_string(&chunk_ids)?;

        conn.execute(
            "INSERT INTO knowledge_items (id, source_type, source_id, title, content, tags, owner, session_id, created_at, updated_at, chunk_ids)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'user', ?7, ?8, ?8, ?9)
             ON CONFLICT(id) DO UPDATE SET
                title = excluded.title, content = excluded.content, tags = excluded.tags,
                session_id = excluded.session_id, updated_at = excluded.updated_at, chunk_ids = excluded.chunk_ids",
            params![item_id, doc.source_type, doc.source_id, doc.title, doc.content, tags_json, doc.session_id, now, chunk_ids_json],
        )?;

        Ok(chunk_ids)
    }

    async fn delete(&self, chunk_ids: &[String]) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        for id in chunk_ids {
            conn.execute("DELETE FROM knowledge_chunks WHERE rowid = ?1", params![id])?;
            conn.execute("DELETE FROM knowledge_chunk_meta WHERE chunk_rowid = ?1", params![id])?;
        }
        Ok(())
    }

    async fn vault_note_paths(&self) -> anyhow::Result<Vec<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT source_id FROM knowledge_items WHERE source_type = 'vault_note'")?;
        let paths: Vec<String> = stmt.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
        Ok(paths)
    }

    async fn delete_by_source(&self, source_type: &str, source_id: &str) -> anyhow::Result<()> {
        let item_id = format!("{source_type}:{source_id}");
        let conn = self.conn.lock().unwrap();
        let chunk_ids_json: Option<String> =
            conn.query_row("SELECT chunk_ids FROM knowledge_items WHERE id = ?1", params![item_id], |r| r.get(0)).optional()?;
        if let Some(json) = chunk_ids_json {
            if let Ok(ids) = serde_json::from_str::<Vec<String>>(&json) {
                for id in ids {
                    conn.execute("DELETE FROM knowledge_chunks WHERE rowid = ?1", params![id])?;
                    conn.execute("DELETE FROM knowledge_chunk_meta WHERE chunk_rowid = ?1", params![id])?;
                }
            }
        }
        conn.execute("DELETE FROM knowledge_items WHERE id = ?1", params![item_id])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::{Message, ToolDef};

    /// A deterministic fake provider — no network, no real model — so ingest/query can be
    /// exercised end-to-end against the *real* sqlite-vec extension without depending on
    /// the user's actual Ollama/Anthropic/OpenAI infra being reachable from this test run.
    struct FakeProvider;

    #[async_trait]
    impl LlmProvider for FakeProvider {
        async fn chat(&self, _messages: &[Message], _tools: &[ToolDef]) -> anyhow::Result<Message> {
            Ok(Message {
                role: "assistant".into(),
                content: "test, fake, tags".into(),
                tool_calls: None,
                tool_name: None,
                images: Vec::new(),
                display: None,
            thinking: String::new(),
            at: None,
            })
        }
        fn is_cloud(&self) -> bool {
            false
        }
        fn name(&self) -> &str {
            "fake"
        }
        async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
            // A cheap, deterministic "embedding": character frequency bucketed into 8 dims.
            // Different text -> different vector; same text -> same vector. Good enough to
            // prove ingest/query wiring without a real embedding model.
            let mut v = [0f32; 8];
            for (i, b) in text.bytes().enumerate() {
                v[i % 8] += b as f32;
            }
            Ok(v.to_vec())
        }
    }

    fn store() -> SqliteKnowledgeStore {
        let provider: Arc<dyn LlmProvider> = Arc::new(FakeProvider);
        SqliteKnowledgeStore::open_in_memory(provider.clone(), provider).unwrap()
    }

    #[tokio::test]
    async fn ingest_then_query_round_trips() {
        let store = store();
        store
            .ingest(IngestDoc {
                source_type: "vault_note".into(),
                source_id: "10 - Daily/2026-08-24.md".into(),
                title: Some("2026-08-24".into()),
                content: "Today's focus is finishing the Loki knowledge store.".into(),
                tags: vec![],
                session_id: None,
            })
            .await
            .unwrap();

        let results = store.query("Loki knowledge store", 5, QueryFilter::default()).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "vault_note:10 - Daily/2026-08-24.md");
    }

    #[tokio::test]
    async fn query_before_any_ingest_returns_empty_not_error() {
        let store = store();
        let results = store.query("anything", 5, QueryFilter::default()).await.unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn reingesting_same_source_id_replaces_not_duplicates() {
        let store = store();
        let doc = || IngestDoc {
            source_type: "chat_message".into(),
            source_id: "msg-1".into(),
            title: None,
            content: "first version".into(),
            tags: vec![],
            session_id: Some("session-a".into()),
        };
        store.ingest(doc()).await.unwrap();
        let mut second = doc();
        second.content = "second version, edited".into();
        store.ingest(second).await.unwrap();

        let conn = store.conn.lock().unwrap();
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM knowledge_items WHERE id = 'chat_message:msg-1'", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 1, "re-ingesting the same source_id should replace, not duplicate the item row");
        let chunk_count: i64 = conn.query_row("SELECT COUNT(*) FROM knowledge_chunk_meta", [], |r| r.get(0)).unwrap();
        assert_eq!(chunk_count, 1, "old chunks should be deleted, not left orphaned, on re-ingest");
    }

    #[tokio::test]
    async fn delete_by_source_removes_item_row_and_chunks() {
        let store = store();
        store
            .ingest(IngestDoc {
                source_type: "vault_note".into(),
                source_id: "30 - Backlog/Tech/Throwaway.md".into(),
                title: Some("Throwaway".into()),
                content: "a note that will be deleted".into(),
                tags: vec![],
                session_id: None,
            })
            .await
            .unwrap();

        store.delete_by_source("vault_note", "30 - Backlog/Tech/Throwaway.md").await.unwrap();

        let conn = store.conn.lock().unwrap();
        let item_count: i64 =
            conn.query_row("SELECT COUNT(*) FROM knowledge_items WHERE id = ?1", params!["vault_note:30 - Backlog/Tech/Throwaway.md"], |r| r.get(0)).unwrap();
        assert_eq!(item_count, 0, "the knowledge_items row itself should be gone, not just its chunks");
        let chunk_count: i64 = conn.query_row("SELECT COUNT(*) FROM knowledge_chunk_meta", [], |r| r.get(0)).unwrap();
        assert_eq!(chunk_count, 0);
    }

    #[tokio::test]
    async fn vault_note_paths_only_returns_vault_notes() {
        let store = store();
        store
            .ingest(IngestDoc {
                source_type: "vault_note".into(),
                source_id: "50 - Notes/Some Idea.md".into(),
                title: None,
                content: "an idea".into(),
                tags: vec![],
                session_id: None,
            })
            .await
            .unwrap();
        store
            .ingest(IngestDoc {
                source_type: "external_ingest".into(),
                source_id: "external:1".into(),
                title: None,
                content: "not a vault note".into(),
                tags: vec![],
                session_id: None,
            })
            .await
            .unwrap();

        let paths = store.vault_note_paths().await.unwrap();
        assert_eq!(paths, vec!["50 - Notes/Some Idea.md".to_string()]);
    }

    #[tokio::test]
    async fn query_filters_by_session_id() {
        let store = store();
        store
            .ingest(IngestDoc {
                source_type: "chat_message".into(),
                source_id: "msg-a".into(),
                title: None,
                content: "budget planning discussion".into(),
                tags: vec![],
                session_id: Some("session-1".into()),
            })
            .await
            .unwrap();
        store
            .ingest(IngestDoc {
                source_type: "chat_message".into(),
                source_id: "msg-b".into(),
                title: None,
                content: "budget planning discussion".into(),
                tags: vec![],
                session_id: Some("session-2".into()),
            })
            .await
            .unwrap();

        let filtered = store
            .query("budget planning discussion", 10, QueryFilter { source_type: None, session_id: Some("session-1".into()) })
            .await
            .unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, "chat_message:msg-a");
    }
}
