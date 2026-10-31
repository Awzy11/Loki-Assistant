//! `loki-mcp` — Loki exposed as an MCP server over stdio, for Claude Code, n8n (via
//! mcp-proxy, matching every other MCP server in the user's stack), or any other MCP client.
//! Same `Core` as the desktop app and `loki-server` — this is a third front door, not a
//! separate implementation of chat/query/ingest.
use loki_lib::core::Core;
use loki_lib::mcp_server::LokiMcpServer;
use rmcp::{transport::stdio, ServiceExt};
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg = loki_lib::config::load()?;
    let core = Arc::new(Core::new(cfg));
    let server = LokiMcpServer::new(core);

    eprintln!("[loki-mcp] starting, stdio transport");
    let running = server.serve(stdio()).await?;
    running.waiting().await?;
    Ok(())
}
