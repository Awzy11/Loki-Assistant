//! Wraps `Core`'s chat/query/ingest capability as MCP tools — the highest-leverage
//! integration path given how much of the user's stack is already MCP-based (Claude Code,
//! n8n via mcp-proxy, the whole `~/.docker/mcp/config.yaml` roster). Reuses the exact same
//! `Core` methods the `/api/v1/*` REST handlers call (`server.rs`), rather than
//! reimplementing chat/query/ingest a third time — one implementation, three front doors
//! now (Tauri, web, MCP).
use crate::core::Core;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::{tool, tool_handler, tool_router, ServerHandler};
use schemars::JsonSchema;
use serde::Deserialize;
use std::sync::Arc;

#[derive(Deserialize, JsonSchema)]
pub struct ChatParams {
    /// Continues this conversation if given; starts a new one otherwise. The response
    /// always reports which session id was actually used.
    #[serde(default)]
    pub session_id: Option<String>,
    /// The message to send Loki.
    pub message: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct QueryParams {
    /// What to search Loki's knowledge store for.
    pub query: String,
    /// Max results to return (default 5).
    #[serde(default = "default_n")]
    pub n: u32,
    /// Restrict to one conversation's history rather than searching everything.
    #[serde(default)]
    pub session_id: Option<String>,
}

fn default_n() -> u32 {
    5
}

#[derive(Deserialize, JsonSchema)]
pub struct IngestParams {
    /// Optional short title for this piece of content.
    #[serde(default)]
    pub title: Option<String>,
    /// The content to remember.
    pub content: String,
    /// Optional tags — if omitted, Loki auto-tags it.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Associate this with a specific conversation, if relevant.
    #[serde(default)]
    pub session_id: Option<String>,
}

#[derive(Clone)]
pub struct LokiMcpServer {
    core: Arc<Core>,
    tool_router: ToolRouter<Self>,
}

impl LokiMcpServer {
    pub fn new(core: Arc<Core>) -> Self {
        Self { core, tool_router: Self::tool_router() }
    }
}

#[tool_router]
impl LokiMcpServer {
    #[tool(
        name = "loki_chat",
        description = "Send a message to Loki (the user's personal agent) and get a reply. Continues a conversation if session_id is given, starts a new one otherwise."
    )]
    async fn loki_chat(&self, Parameters(req): Parameters<ChatParams>) -> String {
        let session_id = req.session_id.unwrap_or_else(|| self.core.new_session());
        // allow_routines: false — kept conservative, matching the legacy HTTP path, rather
        // than assumed. This MCP surface arguably has its own reasonable trust model (a local
        // process someone deliberately configured as an MCP server), but Routine Mode access
        // for it wasn't part of the scoped auth decision this was built against — extend
        // deliberately if wanted, not by default.
        match self.core.send_message_in_session(&session_id, req.message, false).await {
            Ok(turn) => format!("[session_id: {session_id}]\n{}", turn.reply),
            Err(e) => format!("error: {e}"),
        }
    }

    #[tool(
        name = "loki_query",
        description = "Query Loki's knowledge store directly for relevant context, without going through the chat loop — cheaper and faster than loki_chat when you just need context to build your own prompt with."
    )]
    async fn loki_query(&self, Parameters(req): Parameters<QueryParams>) -> String {
        match self.core.query_knowledge(&req.query, req.n, req.session_id).await {
            Ok(results) => serde_json::to_string_pretty(&results).unwrap_or_else(|e| format!("error serializing results: {e}")),
            Err(e) => format!("error: {e}"),
        }
    }

    #[tool(
        name = "loki_ingest",
        description = "Push a piece of content directly into Loki's knowledge store, without a conversational round-trip — e.g. hand Loki a fact to remember from an automated workflow."
    )]
    async fn loki_ingest(&self, Parameters(req): Parameters<IngestParams>) -> String {
        match self.core.ingest_external(req.title, req.content, req.tags, req.session_id).await {
            Ok(chunk_ids) => format!("ok — ingested as {} chunk(s)", chunk_ids.len()),
            Err(e) => format!("error: {e}"),
        }
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for LokiMcpServer {}
