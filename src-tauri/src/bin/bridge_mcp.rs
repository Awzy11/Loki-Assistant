//! `loki-bridge-mcp` — a tiny stdio MCP server that `loki-server` hands to each Claude Code
//! process it spawns (`--mcp-config`). It has no logic of its own: it forwards to
//! `loki-server`'s `/api/v1/bridge/*` endpoints so that, for this turn's session,
//!
//! - `approve` backs `--permission-prompt-tool` (Claude asks, the Loki UI shows Yes/No), and
//! - `device_*` tools let Claude act on the machine the Loki desktop app runs on.
//!
//! Configured entirely through env vars set by the spawner: `LOKI_URL`, `LOKI_TOKEN`,
//! `LOKI_SESSION_ID`.
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::transport::stdio;
use rmcp::{tool, tool_handler, tool_router, ServerHandler, ServiceExt};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Deserialize, JsonSchema)]
struct ApproveParams {
    tool_name: String,
    input: Value,
    #[serde(default)]
    #[allow(dead_code)]
    tool_use_id: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct ReadParams {
    /// Absolute path, or relative to the home directory.
    path: String,
    /// 1-based first line to return.
    #[serde(default)]
    offset: Option<u64>,
    /// Max lines to return (default 2000).
    #[serde(default)]
    limit: Option<u64>,
}

#[derive(Deserialize, JsonSchema)]
struct PathParams {
    path: String,
}

#[derive(Deserialize, JsonSchema)]
struct WriteParams {
    path: String,
    content: String,
}

#[derive(Deserialize, JsonSchema)]
struct EditParams {
    path: String,
    old_string: String,
    new_string: String,
    #[serde(default)]
    replace_all: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
struct RunParams {
    command: String,
    #[serde(default)]
    cwd: Option<String>,
    /// Default 60, max 300.
    #[serde(default)]
    timeout_secs: Option<u64>,
}

#[derive(Clone)]
struct Bridge {
    http: reqwest::Client,
    url: String,
    token: String,
    session_id: String,
    tool_router: ToolRouter<Self>,
}

impl Bridge {
    fn new() -> anyhow::Result<Self> {
        let var = |k: &str| std::env::var(k).map_err(|_| anyhow::anyhow!("{k} is not set"));
        Ok(Self {
            http: reqwest::Client::new(),
            url: var("LOKI_URL")?.trim_end_matches('/').to_string(),
            token: var("LOKI_TOKEN")?,
            session_id: var("LOKI_SESSION_ID")?,
            tool_router: Self::tool_router(),
        })
    }

    async fn post(&self, path: &str, body: Value) -> Result<Value, String> {
        let resp = self
            .http
            .post(format!("{}{path}", self.url))
            .bearer_auth(&self.token)
            // Approvals wait on a human, possibly for a long time (the server's own limit applies,
            // default 1 hour); this must outlast it or the helper would deny first.
            .timeout(std::time::Duration::from_secs(3700))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("couldn't reach loki-server: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("loki-server returned {}", resp.status()));
        }
        resp.json().await.map_err(|e| format!("bad response from loki-server: {e}"))
    }

    async fn device(&self, name: &str, input: Value) -> String {
        match self.post("/api/v1/bridge/device", json!({ "session_id": self.session_id, "name": name, "input": input })).await {
            Ok(v) => {
                let content = v["content"].as_str().unwrap_or_default();
                if v["is_error"].as_bool().unwrap_or(false) {
                    format!("error: {content}")
                } else {
                    content.to_string()
                }
            }
            Err(e) => format!("error: {e}"),
        }
    }
}

#[tool_router]
impl Bridge {
    #[tool(name = "approve", description = "Permission prompt: asks the Loki user to approve or deny a tool call.")]
    async fn approve(&self, Parameters(p): Parameters<ApproveParams>) -> String {
        match self
            .post("/api/v1/bridge/permission", json!({ "session_id": self.session_id, "tool_name": p.tool_name, "input": p.input }))
            .await
        {
            Ok(v) => v.to_string(),
            // Fail closed: if the bridge is unreachable the answer is no.
            Err(e) => json!({ "behavior": "deny", "message": e }).to_string(),
        }
    }

    #[tool(name = "device_read_file", description = "Read a text file on the user's computer (the device running the Loki desktop app). Returns numbered lines.")]
    async fn device_read_file(&self, Parameters(p): Parameters<ReadParams>) -> String {
        self.device("device_read_file", json!({ "path": p.path, "offset": p.offset, "limit": p.limit })).await
    }

    #[tool(name = "device_list_dir", description = "List a directory on the user's computer. Directories end with '/'.")]
    async fn device_list_dir(&self, Parameters(p): Parameters<PathParams>) -> String {
        self.device("device_list_dir", json!({ "path": p.path })).await
    }

    #[tool(name = "device_write_file", description = "Create or overwrite a file on the user's computer. The user is asked to approve each write.")]
    async fn device_write_file(&self, Parameters(p): Parameters<WriteParams>) -> String {
        self.device("device_write_file", json!({ "path": p.path, "content": p.content })).await
    }

    #[tool(
        name = "device_edit_file",
        description = "Replace text in an existing file on the user's computer. old_string must match exactly once unless replace_all is true. The user is asked to approve each edit."
    )]
    async fn device_edit_file(&self, Parameters(p): Parameters<EditParams>) -> String {
        self.device(
            "device_edit_file",
            json!({ "path": p.path, "old_string": p.old_string, "new_string": p.new_string, "replace_all": p.replace_all }),
        )
        .await
    }

    #[tool(name = "device_run_command", description = "Run a shell command on the user's computer and return its output. The user is asked to approve each command.")]
    async fn device_run_command(&self, Parameters(p): Parameters<RunParams>) -> String {
        self.device("device_run_command", json!({ "command": p.command, "cwd": p.cwd, "timeout_secs": p.timeout_secs })).await
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Bridge {}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let service = Bridge::new()?.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
