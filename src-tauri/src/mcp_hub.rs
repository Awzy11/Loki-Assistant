//! Loki's own MCP client. Connects to every configured MCP server (stdio or Streamable HTTP),
//! caches their tool lists, and runs tool calls on behalf of Loki's agent loop — so tools,
//! permissions and settings live in Loki and the model on the other end is only cognition.
//!
//! Tool names follow Claude Code's convention, `mcp__<server>__<tool>`, so allow-lists,
//! always-allow rules and the UI mean the same thing whichever backend made a call.

use crate::config::{McpConfig, McpServerConfig};
use rmcp::model::CallToolRequestParams;
use rmcp::service::RunningService;
use rmcp::transport::{StreamableHttpClientTransport, TokioChildProcess};
use rmcp::{RoleClient, ServiceExt};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(90);
const CALL_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_RESULT_CHARS: usize = 80_000;

#[derive(Debug, Clone)]
pub struct McpTool {
    pub server: String,
    pub name: String,
    pub full_name: String,
    pub description: String,
    pub schema: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct ServerStatus {
    pub name: String,
    pub connected: bool,
    pub tools: usize,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Transport {
    Stdio { command: String, args: Vec<String>, env: HashMap<String, String> },
    Http { url: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ServerSpec {
    pub name: String,
    pub transport: Transport,
}

/// Servers to connect to: `[[mcp.servers]]` plus (if enabled) the user-scope servers from
/// `claude_json` (the parsed `~/.claude.json`), minus `disabled`. Pure so it can be tested.
pub fn server_specs(cfg: &McpConfig, claude_json: Option<&Value>) -> Vec<ServerSpec> {
    let mut by_name: BTreeMap<String, ServerSpec> = BTreeMap::new();
    if cfg.import_claude_servers {
        for (name, v) in claude_json.and_then(|j| j["mcpServers"].as_object()).into_iter().flatten() {
            if let Some(t) = transport_from_claude(v) {
                by_name.insert(name.clone(), ServerSpec { name: name.clone(), transport: t });
            }
        }
    }
    for s in &cfg.servers {
        if let Some(t) = transport_from_config(s) {
            by_name.insert(s.name.clone(), ServerSpec { name: s.name.clone(), transport: t });
        }
    }
    // Never connect Loki to itself: `loki-mcp` would let the model recurse into Loki's own chat.
    let mut skip: Vec<&str> = cfg.disabled.iter().map(String::as_str).collect();
    skip.extend(["loki-mcp", "loki-bridge"]);
    by_name.into_values().filter(|s| !skip.contains(&s.name.as_str())).collect()
}

fn http_url(url: &str) -> String {
    // mcp-proxy serves the legacy SSE transport at /sse and Streamable HTTP at /mcp.
    match url.strip_suffix("/sse") {
        Some(base) => format!("{base}/mcp"),
        None => url.to_string(),
    }
}

fn transport_from_claude(v: &Value) -> Option<Transport> {
    match v["type"].as_str().unwrap_or("stdio") {
        "stdio" => Some(Transport::Stdio {
            command: v["command"].as_str()?.to_string(),
            args: v["args"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default(),
            env: v["env"]
                .as_object()
                .map(|m| m.iter().filter_map(|(k, x)| x.as_str().map(|s| (k.clone(), s.to_string()))).collect())
                .unwrap_or_default(),
        }),
        "sse" | "http" => Some(Transport::Http { url: http_url(v["url"].as_str()?) }),
        _ => None,
    }
}

fn transport_from_config(s: &McpServerConfig) -> Option<Transport> {
    match (&s.command, &s.url) {
        (Some(command), _) => Some(Transport::Stdio { command: command.clone(), args: s.args.clone(), env: s.env.clone() }),
        (None, Some(url)) => Some(Transport::Http { url: http_url(url) }),
        _ => None,
    }
}

/// Reads `~/.claude.json`, if present.
pub fn read_claude_json() -> Option<Value> {
    let path = dirs::home_dir()?.join(".claude.json");
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

struct Conn {
    service: RunningService<RoleClient, ()>,
}

pub struct McpHub {
    specs: RwLock<Vec<ServerSpec>>,
    conns: tokio::sync::Mutex<HashMap<String, Arc<Conn>>>,
    tools: RwLock<Vec<McpTool>>,
    status: Mutex<BTreeMap<String, ServerStatus>>,
}

impl McpHub {
    pub fn new(specs: Vec<ServerSpec>) -> Arc<Self> {
        let status = specs
            .iter()
            .map(|s| (s.name.clone(), ServerStatus { name: s.name.clone(), connected: false, tools: 0, error: Some("connecting".into()) }))
            .collect();
        Arc::new(Self { specs: RwLock::new(specs), conns: Default::default(), tools: RwLock::new(Vec::new()), status: Mutex::new(status) })
    }

    pub fn from_config(cfg: &McpConfig) -> Arc<Self> {
        let json = if cfg.import_claude_servers { read_claude_json() } else { None };
        Self::new(server_specs(cfg, json.as_ref()))
    }

    /// Connects every server in the background, each with its own timeout; one slow or dead
    /// server (a docker pull, a retired service) never blocks the others.
    pub fn spawn_connect_all(self: &Arc<Self>) {
        let specs = self.specs.read().unwrap().clone();
        for spec in specs {
            let hub = self.clone();
            tokio::spawn(async move {
                match tokio::time::timeout(CONNECT_TIMEOUT, hub.connect(&spec)).await {
                    Ok(Ok(n)) => eprintln!("[loki-debug] mcp: '{}' connected, {n} tool(s)", spec.name),
                    Ok(Err(e)) => hub.mark_failed(&spec.name, &e.to_string()),
                    Err(_) => hub.mark_failed(&spec.name, "timed out connecting"),
                }
            });
        }
    }

    /// Brings the live connections in line with `new`: servers that vanished or changed are dropped, new or changed ones
    /// are connected in the background, the rest are left alone. This is how servers are added, removed, enabled and
    /// disabled without restarting Loki.
    pub async fn reconcile(self: &Arc<Self>, new: Vec<ServerSpec>) {
        let old = self.specs.read().unwrap().clone();
        for o in &old {
            if !new.iter().any(|n| n == o) {
                self.disconnect(&o.name).await;
            }
        }
        *self.specs.write().unwrap() = new.clone();
        for spec in new {
            if old.iter().any(|o| *o == spec) {
                continue;
            }
            self.status.lock().unwrap().insert(spec.name.clone(), ServerStatus { name: spec.name.clone(), connected: false, tools: 0, error: Some("connecting".into()) });
            self.spawn_connect(spec);
        }
    }

    /// Drops one server's connection, tools and status.
    pub async fn disconnect(&self, name: &str) {
        self.conns.lock().await.remove(name); // dropping the connection ends the session (and a stdio child)
        self.tools.write().unwrap().retain(|t| t.server != name);
        self.status.lock().unwrap().remove(name);
    }

    /// Drops and re-opens one server's connection.
    pub async fn reconnect(self: &Arc<Self>, name: &str) -> anyhow::Result<()> {
        let spec = self.specs.read().unwrap().iter().find(|s| s.name == name).cloned().ok_or_else(|| anyhow::anyhow!("'{name}' isn't an enabled server"))?;
        self.disconnect(name).await;
        self.status.lock().unwrap().insert(name.to_string(), ServerStatus { name: name.to_string(), connected: false, tools: 0, error: Some("connecting".into()) });
        self.spawn_connect(spec);
        Ok(())
    }

    fn spawn_connect(self: &Arc<Self>, spec: ServerSpec) {
        let hub = self.clone();
        tokio::spawn(async move {
            match tokio::time::timeout(CONNECT_TIMEOUT, hub.connect(&spec)).await {
                Ok(Ok(n)) => eprintln!("[loki-debug] mcp: '{}' connected, {n} tool(s)", spec.name),
                Ok(Err(e)) => hub.mark_failed(&spec.name, &e.to_string()),
                Err(_) => hub.mark_failed(&spec.name, "timed out connecting"),
            }
        });
    }

    /// One connected server's tools: (name, description).
    pub fn tools_of(&self, server: &str) -> Vec<(String, String)> {
        self.tools.read().unwrap().iter().filter(|t| t.server == server).map(|t| (t.name.clone(), t.description.chars().take(200).collect())).collect()
    }

    fn mark_failed(&self, name: &str, error: &str) {
        eprintln!("[loki-debug] mcp: '{name}' unavailable: {error}");
        self.tools.write().unwrap().retain(|t| t.server != name);
        if let Some(s) = self.status.lock().unwrap().get_mut(name) {
            *s = ServerStatus { name: name.to_string(), connected: false, tools: 0, error: Some(error.chars().take(200).collect()) };
        }
    }

    async fn connect(&self, spec: &ServerSpec) -> anyhow::Result<usize> {
        let service: RunningService<RoleClient, ()> = match &spec.transport {
            Transport::Stdio { command, args, env } => {
                let mut cmd = tokio::process::Command::new(command);
                cmd.args(args).envs(env);
                let (transport, _stderr) = TokioChildProcess::builder(cmd).stderr(std::process::Stdio::null()).spawn()?;
                ().serve(transport).await?
            }
            Transport::Http { url } => ().serve(StreamableHttpClientTransport::from_uri(url.clone())).await?,
        };
        let listed = service.list_all_tools().await?;
        let tools: Vec<McpTool> = listed
            .into_iter()
            .map(|t| {
                let v = serde_json::to_value(&t).unwrap_or(Value::Null);
                McpTool {
                    server: spec.name.clone(),
                    full_name: format!("mcp__{}__{}", spec.name, t.name),
                    name: t.name.to_string(),
                    description: v["description"].as_str().unwrap_or_default().to_string(),
                    schema: v["inputSchema"].clone(),
                }
            })
            .collect();
        let n = tools.len();
        {
            let mut all = self.tools.write().unwrap();
            all.retain(|t| t.server != spec.name);
            all.extend(tools);
        }
        self.conns.lock().await.insert(spec.name.clone(), Arc::new(Conn { service }));
        if let Some(s) = self.status.lock().unwrap().get_mut(&spec.name) {
            *s = ServerStatus { name: spec.name.clone(), connected: true, tools: n, error: None };
        }
        Ok(n)
    }

    /// The same server set in the shape `claude --mcp-config` takes, so the delegated Claude
    /// path uses exactly the servers (and the shared HTTP endpoints) Loki's own hub does,
    /// instead of re-reading `~/.claude.json` and starting a second copy of each.
    pub fn claude_servers(&self) -> serde_json::Map<String, Value> {
        let specs = self.specs.read().unwrap().clone();
        specs
            .iter()
            .map(|s| {
                let v = match &s.transport {
                    Transport::Stdio { command, args, env } => serde_json::json!({ "command": command, "args": args, "env": env }),
                    Transport::Http { url } => serde_json::json!({ "type": "http", "url": url }),
                };
                (s.name.clone(), v)
            })
            .collect()
    }

    pub fn tools(&self) -> Vec<McpTool> {
        self.tools.read().unwrap().clone()
    }

    pub fn tool(&self, full_name: &str) -> Option<McpTool> {
        self.tools.read().unwrap().iter().find(|t| t.full_name == full_name).cloned()
    }

    pub fn status(&self) -> Vec<ServerStatus> {
        self.status.lock().unwrap().values().cloned().collect()
    }

    /// Keyword search over every connected tool's name and description, best first.
    pub fn search(&self, query: &str, limit: usize) -> Vec<McpTool> {
        let words: Vec<String> = query.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| w.len() > 1).map(String::from).collect();
        let mut scored: Vec<(usize, McpTool)> = self
            .tools
            .read()
            .unwrap()
            .iter()
            .filter_map(|t| {
                let name = t.full_name.to_lowercase();
                let desc = t.description.to_lowercase();
                let score: usize = words.iter().map(|w| (if name.contains(w) { 3 } else { 0 }) + (if desc.contains(w) { 1 } else { 0 })).sum();
                (score > 0).then(|| (score, t.clone()))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.full_name.cmp(&b.1.full_name)));
        scored.into_iter().take(limit).map(|(_, t)| t).collect()
    }

    /// Runs `full_name` with `args`, reconnecting once if the server's transport died.
    pub async fn call(&self, full_name: &str, args: &Value) -> anyhow::Result<String> {
        let tool = self.tool(full_name).ok_or_else(|| anyhow::anyhow!("unknown MCP tool '{full_name}' (try search_tools)"))?;
        let arguments = match args {
            Value::Object(m) => Some(m.clone()),
            Value::Null => None,
            _ => anyhow::bail!("tool arguments must be a JSON object"),
        };
        for attempt in 0..2 {
            let conn = { self.conns.lock().await.get(&tool.server).cloned() };
            let Some(conn) = conn else {
                anyhow::bail!("MCP server '{}' is not connected", tool.server);
            };
            let mut params = CallToolRequestParams::new(tool.name.clone());
            params.arguments = arguments.clone();
            match tokio::time::timeout(CALL_TIMEOUT, conn.service.call_tool(params)).await {
                Ok(Ok(result)) => return Ok(result_text(&serde_json::to_value(&result).unwrap_or(Value::Null))),
                Ok(Err(e)) if attempt == 0 => {
                    eprintln!("[loki-debug] mcp: call to '{full_name}' failed ({e}); reconnecting '{}'", tool.server);
                    let spec = self.specs.read().unwrap().iter().find(|s| s.name == tool.server).cloned();
                    if let Some(spec) = spec {
                        self.conns.lock().await.remove(&tool.server);
                        self.connect(&spec).await?;
                    }
                }
                Ok(Err(e)) => anyhow::bail!("{e}"),
                Err(_) => anyhow::bail!("MCP call timed out after {}s", CALL_TIMEOUT.as_secs()),
            }
        }
        unreachable!()
    }
}

/// Over this size, a result made of many articles (`=== ARTICLE [id] ===` blocks, as the Miniflux server returns) is
/// shortened article by article instead of being cut off at the end.
const COMPACT_ABOVE: usize = 24_000;
/// What is kept of one article's text when shortening: enough for a good summary, small enough that all 15 fit.
const ARTICLE_TEXT_MAX: usize = 1_400;
const ARTICLE_MARK: &str = "=== ARTICLE [";

/// Shortens every article's `Content:` to `ARTICLE_TEXT_MAX` characters, keeping every title, feed, date and URL.
/// Text that isn't a run of article blocks comes back unchanged.
pub fn shorten_articles(text: &str) -> String {
    if text.matches(ARTICLE_MARK).count() < 3 {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len() / 3);
    for (i, part) in text.split(ARTICLE_MARK).enumerate() {
        if i == 0 {
            out.push_str(part);
            continue;
        }
        out.push_str(ARTICLE_MARK);
        match part.find("\nContent: ") {
            Some(at) => {
                let (head, body) = part.split_at(at + "\nContent: ".len());
                let trimmed = body.trim_end();
                if trimmed.chars().count() > ARTICLE_TEXT_MAX {
                    out.push_str(head);
                    out.extend(trimmed.chars().take(ARTICLE_TEXT_MAX));
                    out.push_str(" … [article text shortened]");
                    out.push_str(&body[trimmed.len()..]);
                } else {
                    out.push_str(part);
                }
            }
            None => out.push_str(part),
        }
    }
    out
}

/// Clips a tool result to what a model should read; the full text stays available to `{{result:…}}`
/// pipes (see `agent::pipe`). Long article lists are shortened per article first, so the model still sees every
/// headline and link; anything still over the hard cap is cut at the end.
pub fn clip_result(text: String) -> String {
    let text = if text.chars().count() > COMPACT_ABOVE { shorten_articles(&text) } else { text };
    if text.chars().count() > MAX_RESULT_CHARS {
        text.chars().take(MAX_RESULT_CHARS).collect::<String>() + "\n… [result truncated — pipe the full text on with {{result:tool_name}}]"
    } else {
        text
    }
}

/// Flattens a `CallToolResult` (as JSON) into text for a model: text blocks joined, other block
/// types noted, structured content as a fallback, errors prefixed.
pub fn result_text(result: &Value) -> String {
    let mut parts: Vec<String> = Vec::new();
    for block in result["content"].as_array().into_iter().flatten() {
        match block["type"].as_str() {
            Some("text") => parts.push(block["text"].as_str().unwrap_or_default().to_string()),
            Some(other) => parts.push(format!("[{other} content omitted]")),
            None => {}
        }
    }
    let mut text = parts.join("\n");
    if text.trim().is_empty() {
        if let Some(sc) = result.get("structuredContent").filter(|v| !v.is_null()) {
            text = sc.to_string();
        }
    }
    if result["isError"].as_bool().unwrap_or(false) {
        format!("error: {text}")
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn specs_import_claude_servers_and_map_sse() {
        let j = json!({"mcpServers": {
            "obsidian": {"type":"stdio","command":"uvx","args":["mcp-obsidian"],"env":{"K":"v"}},
            "n8n": {"type":"sse","url":"http://127.0.0.1:8102/sse"},
            "loki-mcp": {"type":"stdio","command":"/x/loki-mcp"},
            "anki": {"type":"sse","url":"http://localhost:8107/sse"},
        }});
        let cfg = McpConfig { disabled: vec!["anki".into()], ..Default::default() };
        let specs = server_specs(&cfg, Some(&j));
        let names: Vec<&str> = specs.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["n8n", "obsidian"]); // loki-mcp and disabled anki dropped
        assert_eq!(specs[0].transport, Transport::Http { url: "http://127.0.0.1:8102/mcp".into() });
        assert!(matches!(&specs[1].transport, Transport::Stdio { command, env, .. } if command == "uvx" && env["K"] == "v"));
    }

    #[test]
    fn config_servers_override_imports_and_import_can_be_off() {
        let j = json!({"mcpServers": {"fetch": {"type":"stdio","command":"old"}}});
        let cfg = McpConfig {
            servers: vec![McpServerConfig { name: "fetch".into(), command: Some("new".into()), ..Default::default() }],
            ..Default::default()
        };
        assert!(matches!(&server_specs(&cfg, Some(&j))[0].transport, Transport::Stdio { command, .. } if command == "new"));
        let off = McpConfig { import_claude_servers: false, ..Default::default() };
        assert!(server_specs(&off, Some(&j)).is_empty());
    }

    #[test]
    fn claude_servers_mirror_the_hub_specs() {
        let hub = McpHub::new(vec![
            ServerSpec { name: "github".into(), transport: Transport::Http { url: "http://127.0.0.1:8103/mcp".into() } },
            ServerSpec {
                name: "obsidian".into(),
                transport: Transport::Stdio { command: "uvx".into(), args: vec!["mcp-obsidian".into()], env: [("K".to_string(), "v".to_string())].into() },
            },
        ]);
        let m = hub.claude_servers();
        assert_eq!(m["github"], json!({"type": "http", "url": "http://127.0.0.1:8103/mcp"}));
        assert_eq!(m["obsidian"]["command"], "uvx");
        assert_eq!(m["obsidian"]["env"]["K"], "v");
    }

    #[test]
    fn long_article_lists_are_shortened_per_article_not_cut_off() {
        let article = |n: usize, len: usize| format!("=== ARTICLE [{n}] ===\nTitle: Story {n}\nFeed: Wire  |  Date: 2026-10-27\nURL: https://example.com/{n}\nContent: {}\n\n", "word ".repeat(len / 5));
        let big: String = (1..=15).map(|n| article(n, 9_000)).collect();
        assert!(big.len() > 100_000);
        let out = clip_result(big);
        assert!(out.chars().count() < 40_000, "{}", out.chars().count());
        for n in 1..=15 {
            assert!(out.contains(&format!("https://example.com/{n}\n")), "article {n} lost its link");
        }
        assert!(out.contains("[article text shortened]") && !out.contains("[result truncated"));
        // short results, and text that isn't article blocks, are left alone
        let small = article(1, 400) + &article(2, 400) + &article(3, 400);
        assert_eq!(clip_result(small.clone()), small);
        assert_eq!(shorten_articles("just one long paragraph"), "just one long paragraph");
        // a single article under the cap is untouched even inside a long list
        assert!(shorten_articles(&(article(1, 300) + &article(2, 300) + &article(3, 300))).contains(&"word ".repeat(60)));
    }

    #[test]
    fn result_text_flattens_and_flags_errors() {
        let ok = json!({"content":[{"type":"text","text":"a"},{"type":"image","data":"x"},{"type":"text","text":"b"}]});
        assert_eq!(result_text(&ok), "a\n[image content omitted]\nb");
        assert_eq!(result_text(&json!({"content":[{"type":"text","text":"boom"}],"isError":true})), "error: boom");
        assert_eq!(result_text(&json!({"content":[],"structuredContent":{"n":1}})), "{\"n\":1}");
        let big = json!({"content":[{"type":"text","text":"x".repeat(MAX_RESULT_CHARS + 10_000)}]});
        assert!(result_text(&big).chars().count() > MAX_RESULT_CHARS);
        assert!(clip_result(result_text(&big)).contains("[result truncated"));
    }

    #[test]
    fn search_ranks_name_hits_over_description_hits() {
        let hub = McpHub::new(vec![]);
        let mk = |server: &str, name: &str, desc: &str| McpTool {
            server: server.into(),
            name: name.into(),
            full_name: format!("mcp__{server}__{name}"),
            description: desc.into(),
            schema: Value::Null,
        };
        *hub.tools.write().unwrap() = vec![
            mk("mealie", "get_recipes", "List recipes"),
            mk("osint", "submarine_cables_in_area", "Find undersea cables near a place"),
            mk("odin", "odin_search", "Search country studies for cables"),
        ];
        let hits = hub.search("submarine cables", 5);
        assert_eq!(hits[0].full_name, "mcp__osint__submarine_cables_in_area");
        assert!(hits.iter().any(|t| t.server == "odin")); // description-only match still listed
        assert!(hub.search("zzzz", 5).is_empty());
    }
}
