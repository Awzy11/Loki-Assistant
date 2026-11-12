use crate::bridge::{self, Bridge};
use crate::claude::ClaudeClient;
use crate::knowledge::{import::vault as vault_import, KnowledgeStore, QueryFilter};
use crate::mcp_hub::McpHub;
use crate::tool_policy::ToolPolicy;
use crate::obsidian::ObsidianClient;
use crate::permissions::Tier;
use crate::providers::{Message, ToolDef};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

/// Everything a tool might need at call time. Not every tool uses every field —
/// e.g. `escalate_to_claude` only touches `claude`, `semantic_search_vault` only
/// touches `knowledge`. Bundled together so `Tool::execute`'s signature doesn't grow
/// a new positional argument every time a new backend gets wired in.
///
/// `knowledge` is `Arc`, not a borrowed reference, specifically so `ReadFile`/`AppendContent`
/// can clone it into a `tokio::spawn`'d background re-ingest (Phase 3's incremental re-sync)
/// without fighting `ToolCtx`'s own borrow lifetime.
pub struct ToolCtx<'a> {
    pub obsidian: &'a ObsidianClient,
    pub knowledge: Arc<dyn KnowledgeStore>,
    pub claude: &'a ClaudeClient,
    /// For tools that need the client (device tools) and for the Tier-2 confirmation gate.
    pub bridge: &'a Bridge,
    pub session_id: &'a str,
    /// Standing instructions + memory, for `read_guidance` / `read_memory`.
    pub context: &'a crate::local_context::LocalContext,
    pub skills: &'a crate::skills::Skills,
}

/// Fire-and-forget re-ingest of one vault note into the knowledge store, used after Loki
/// itself reads or writes a note (Phase 3's incremental re-sync — see the module's tools
/// below). Never blocks the tool call it's triggered from, and never fails it: a slow or
/// unreachable embedding backend shouldn't make a read/append call itself fail, only skip
/// this note's re-indexing for now.
fn spawn_reingest(knowledge: Arc<dyn KnowledgeStore>, path: String, content: String) {
    tokio::spawn(async move {
        if let Err(e) = vault_import::ingest_note_content(knowledge.as_ref(), &path, &content, None).await {
            eprintln!("[loki-debug] background re-ingest of '{path}' failed (non-fatal): {e}");
        }
    });
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn def(&self) -> ToolDef;
    /// The tier this tool is registered at. Note this is NOT the sole source of truth
    /// for `escalate_to_claude`'s behavior — see `ToolRegistry::tier_of`'s doc comment
    /// and `agent::run_turn`'s special-case handling of that one tool name.
    fn tier(&self) -> Tier;
    /// Rejects arguments that can never succeed (e.g. a host that isn't allow-listed) BEFORE the
    /// user is asked to approve the call — nobody should be prompted for something that will be
    /// refused anyway. Default: accept.
    fn validate(&self, _args: &Value) -> Result<(), String> {
        Ok(())
    }
    /// True for the `fs_*` tools (#13) — `Core`/`agent::run_turn` never calls `execute` on
    /// these; the device Loki is talking through (the desktop app) executes them locally and
    /// hands the result back via `/api/v1/chat/tool-result`, since the files these touch live
    /// on whichever machine is running that client, not on `loki-server`'s own machine. See
    /// `agent::run_turn`'s doc comment on `TurnOutcome` for the actual handoff mechanics.
    fn is_client_executed(&self) -> bool {
        false
    }
    /// For a client-executed tool, `agent::run_turn` never calls this — each `fs_*` tool's own
    /// `execute` body just returns an error, as a defensive backstop against a bug that routes
    /// one here anyway.
    async fn execute(&self, args: &Value, ctx: &ToolCtx<'_>) -> anyhow::Result<String>;
}

/// MCP tools offered to the model directly every turn when `[mcp] pinned_tools` is empty — the
/// vault essentials. Everything else is reached with `search_tools` + `call_tool`, so a turn
/// carries a handful of tool schemas instead of every MCP server's full catalogue (which would
/// not fit a small local model's context).
const DEFAULT_PINNED: &[&str] = &[
    // Loki's own read_file / append_content / simple_search / list_files_* already cover the
    // basics; only the vault tools it lacks are pinned.
    "mcp__obsidian__obsidian_patch_content",
    "mcp__obsidian__obsidian_get_periodic_note",
];

pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
    mcp: Option<Arc<McpHub>>,
    policy: ToolPolicy,
    pinned: Vec<String>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self { tools: HashMap::new(), mcp: None, policy: ToolPolicy::new(&[]), pinned: Vec::new() }
    }

    /// Registry with Loki's MCP client wired in: discovery/call meta-tools, pinned MCP tools,
    /// and tiers for every `mcp__*` name from `policy`.
    pub fn with_mcp(hub: Arc<McpHub>, policy: ToolPolicy, pinned: Vec<String>) -> Self {
        let mut r = Self::default_registry();
        r.register(Box::new(SearchTools { hub: hub.clone() }));
        r.register(Box::new(CallTool));
        r.pinned = if pinned.is_empty() { DEFAULT_PINNED.iter().map(|s| s.to_string()).collect() } else { pinned };
        r.mcp = Some(hub);
        r.policy = policy;
        r
    }

    /// Adds `remote_run_command` for the allow-listed SSH hosts (no-op when `hosts` is empty).
    pub fn with_remote(mut self, hosts: Vec<String>, preludes: std::collections::HashMap<String, String>) -> Self {
        if !hosts.is_empty() {
            self.register(Box::new(RemoteRun { hosts, preludes }));
        }
        self
    }

    /// Pre-approval argument check; see `Tool::validate`. Unknown tools pass (they fail later).
    pub fn validate(&self, name: &str, args: &Value) -> Result<(), String> {
        self.tools.get(name).map(|t| t.validate(args)).unwrap_or(Ok(()))
    }

    /// Unwraps the `call_tool` meta-tool into the tool it actually invokes, so gating, the
    /// content classifier, approval prompts and rules all see the real tool name and arguments.
    pub fn resolve(&self, name: &str, args: &Value) -> (String, Value) {
        if name == "call_tool" {
            if let Some(target) = args.get("name").and_then(|v| v.as_str()) {
                let inner = args.get("arguments").cloned().unwrap_or_else(|| json!({}));
                return (target.to_string(), inner);
            }
        }
        (name.to_string(), args.clone())
    }

    pub fn register(&mut self, tool: Box<dyn Tool>) {
        let name = tool.def().function.name.clone();
        self.tools.insert(name, tool);
    }

    pub fn definitions(&self) -> Vec<ToolDef> {
        let mut defs: Vec<ToolDef> = self.tools.values().map(|t| t.def()).collect();
        if let Some(hub) = &self.mcp {
            for name in &self.pinned {
                if let Some(t) = hub.tool(name) {
                    let desc: String = t.description.chars().take(400).collect();
                    defs.push(ToolDef::function(&t.full_name, &desc, t.schema.clone()));
                }
            }
        }
        defs
    }

    /// Fail-closed, same property `permissions::tier_of` had before this refactor: a tool
    /// name not present in the registry — i.e. registered wrong, or simply forgotten — is
    /// treated as `LocalOnly` rather than `Ambient`. This is the one property that must
    /// survive every future change to how tools get registered.
    pub fn tier_of(&self, name: &str) -> Tier {
        if let Some(t) = self.tools.get(name) {
            return t.tier();
        }
        // MCP tools are tiered by the shared policy (allow-listed = free, else asks). Anything
        // else that isn't registered stays fail-closed.
        if self.mcp.is_some() && name.starts_with("mcp__") {
            return self.policy.tier(name);
        }
        Tier::LocalOnly
    }

    /// Runs a tool and returns its full, unclipped result. `clip` is for what the model reads.
    pub async fn execute(&self, name: &str, args: &Value, ctx: &ToolCtx<'_>) -> String {
        match self.tools.get(name) {
            Some(tool) => match tool.execute(args, ctx).await {
                Ok(s) => s,
                Err(e) => format!("error: {e}"),
            },
            None => match (&self.mcp, name.starts_with("mcp__")) {
                (Some(hub), true) => match hub.call(name, args).await {
                    Ok(s) => s,
                    Err(e) => format!("error: {e}"),
                },
                _ => format!("error: unknown tool '{name}'"),
            },
        }
    }

    /// Drops everything that writes, escalates or needs the client, leaving read-only (Ambient and
    /// LocalOnly) tools. Used for `delegate_local`'s sub-turn.
    pub fn retain_read_only(&mut self) {
        self.tools.retain(|name, t| matches!(t.tier(), Tier::Ambient | Tier::LocalOnly) && !t.is_client_executed() && name != "escalate_to_claude" && name != "delegate_local");
        self.mcp = None;
    }

    /// The 7 tools Loki v0 already had, ported into the registry unchanged in behavior.
    pub fn default_registry() -> Self {
        let mut r = Self::new();
        r.register(Box::new(ListFilesInVault));
        r.register(Box::new(ListFilesInDir));
        r.register(Box::new(ReadFile));
        r.register(Box::new(AppendContent));
        r.register(Box::new(SimpleSearch));
        r.register(Box::new(SemanticSearchVault));
        r.register(Box::new(RecallConversation));
        r.register(Box::new(EscalateToClaude));
        r.register(Box::new(DeleteFile));
        r.register(Box::new(ReadGuidance));
        r.register(Box::new(ReadMemory));
        r.register(Box::new(ReadSkill));
        for tool in DeviceTool::all() {
            r.register(Box::new(tool));
        }
        r
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

struct ListFilesInVault;
#[async_trait]
impl Tool for ListFilesInVault {
    fn def(&self) -> ToolDef {
        ToolDef::function(
            "list_files_in_vault",
            "List all files and folders at the root of the vault.",
            json!({ "type": "object", "properties": {}, "required": [] }),
        )
    }
    fn tier(&self) -> Tier {
        Tier::Ambient
    }
    async fn execute(&self, _args: &Value, ctx: &ToolCtx<'_>) -> anyhow::Result<String> {
        let files = ctx.obsidian.list_files(None).await?;
        Ok(json!(files).to_string())
    }
}

struct ListFilesInDir;
#[async_trait]
impl Tool for ListFilesInDir {
    fn def(&self) -> ToolDef {
        ToolDef::function(
            "list_files_in_dir",
            "List files and folders in a specific vault directory.",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Directory path relative to vault root, no leading/trailing slash" }
                },
                "required": ["path"]
            }),
        )
    }
    fn tier(&self) -> Tier {
        Tier::Ambient
    }
    async fn execute(&self, args: &Value, ctx: &ToolCtx<'_>) -> anyhow::Result<String> {
        let path = args.get("path").and_then(|v| v.as_str()).unwrap_or_default();
        let files = ctx.obsidian.list_files(Some(path)).await?;
        Ok(json!(files).to_string())
    }
}

struct ReadFile;
#[async_trait]
impl Tool for ReadFile {
    fn def(&self) -> ToolDef {
        ToolDef::function(
            "read_file",
            "Read the full contents of a single file in the vault.",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path relative to the vault root, e.g. 'Folder/Note.md'" }
                },
                "required": ["path"]
            }),
        )
    }
    fn tier(&self) -> Tier {
        Tier::Ambient
    }
    async fn execute(&self, args: &Value, ctx: &ToolCtx<'_>) -> anyhow::Result<String> {
        let path = args.get("path").and_then(|v| v.as_str()).unwrap_or_default();
        let content = ctx.obsidian.read_file(path).await?;
        // Opportunistic incremental re-sync (Phase 3): a note becomes searchable the moment
        // Loki reads it, even before a full vault import has run — doesn't add latency to
        // the read itself.
        spawn_reingest(ctx.knowledge.clone(), path.to_string(), content.clone());
        Ok(content)
    }
}

struct AppendContent;
#[async_trait]
impl Tool for AppendContent {
    fn def(&self) -> ToolDef {
        ToolDef::function(
            "append_content",
            "Create a file (if it doesn't exist) or append content to an existing file.",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path relative to vault root" },
                    "content": { "type": "string", "description": "Content to append (or full content for a new file)" }
                },
                "required": ["path", "content"]
            }),
        )
    }
    fn tier(&self) -> Tier {
        Tier::FreeWrite
    }
    async fn execute(&self, args: &Value, ctx: &ToolCtx<'_>) -> anyhow::Result<String> {
        let path = args.get("path").and_then(|v| v.as_str()).unwrap_or_default();
        let content = args.get("content").and_then(|v| v.as_str()).unwrap_or_default();
        ctx.obsidian.append_content(path, content).await?;
        // Incremental re-sync (Phase 3): re-ingest is needed here (not just on read) because
        // this call *changes* the note's content — a stale index entry is actively wrong,
        // not just missing. Uses `read_file` to re-fetch the note's actual full content
        // rather than assuming `content` (the appended fragment) is the whole note.
        if let Ok(full_content) = ctx.obsidian.read_file(path).await {
            spawn_reingest(ctx.knowledge.clone(), path.to_string(), full_content);
        }
        Ok("ok".to_string())
    }
}

struct SimpleSearch;
#[async_trait]
impl Tool for SimpleSearch {
    fn def(&self) -> ToolDef {
        ToolDef::function(
            "simple_search",
            "Exact keyword/full-text search across the whole vault.",
            json!({
                "type": "object",
                "properties": { "query": { "type": "string" } },
                "required": ["query"]
            }),
        )
    }
    fn tier(&self) -> Tier {
        Tier::Ambient
    }
    async fn execute(&self, args: &Value, ctx: &ToolCtx<'_>) -> anyhow::Result<String> {
        let query = args.get("query").and_then(|v| v.as_str()).unwrap_or_default();
        let results = ctx.obsidian.simple_search(query, 100).await?;
        Ok(json!(results).to_string())
    }
}

/// Searches this conversation's filed exchanges (the ones that fell out of the model's context).
struct RecallConversation;
#[async_trait]
impl Tool for RecallConversation {
    fn def(&self) -> ToolDef {
        ToolDef::function(
            "recall_conversation",
            "Search the EARLIER parts of this conversation (older than what you can currently see) by meaning. Each hit shows its turn number, time, tags and the exchange text. Use it when the user refers to something said earlier that you can't see.",
            json!({
                "type": "object",
                "properties": { "query": { "type": "string", "description": "What to look for, in plain words." } },
                "required": ["query"]
            }),
        )
    }
    fn tier(&self) -> Tier {
        Tier::Ambient
    }
    async fn execute(&self, args: &Value, ctx: &ToolCtx<'_>) -> anyhow::Result<String> {
        let query = args.get("query").and_then(|v| v.as_str()).unwrap_or_default();
        let filter = QueryFilter { source_type: Some("chat_exchange".to_string()), session_id: Some(ctx.session_id.to_string()) };
        let results = ctx.knowledge.query(query, 6, filter).await?;
        if results.is_empty() {
            return Ok("Nothing earlier in this conversation matches that (older turns may not be filed yet).".to_string());
        }
        Ok(results
            .into_iter()
            .map(|r| {
                let tags = if r.tags.is_empty() { String::new() } else { format!(" #{}", r.tags.join(" #")) };
                format!("[{}{tags}]\n{}", r.title.unwrap_or_else(|| r.id.clone()), r.snippet)
            })
            .collect::<Vec<_>>()
            .join("\n\n"))
    }
}

struct SemanticSearchVault;
#[async_trait]
impl Tool for SemanticSearchVault {
    fn def(&self) -> ToolDef {
        ToolDef::function(
            "semantic_search_vault",
            "Meaning-based search across the vault via embeddings — finds conceptually related notes even when the wording doesn't match the query.",
            json!({
                "type": "object",
                "properties": { "query": { "type": "string" } },
                "required": ["query"]
            }),
        )
    }
    fn tier(&self) -> Tier {
        Tier::Ambient
    }
    async fn execute(&self, args: &Value, ctx: &ToolCtx<'_>) -> anyhow::Result<String> {
        let query = args.get("query").and_then(|v| v.as_str()).unwrap_or_default();
        let results = ctx.knowledge.query(query, 5, QueryFilter::default()).await?;
        Ok(json!(results).to_string())
    }
}

/// Note: this tool's registered `tier()` is largely academic — `agent::run_turn` special-cases
/// `escalate_to_claude` by name (checking `local_only_touched` before allowing the call) rather
/// than routing it through the generic "is this tool's tier LocalOnly, so mark it touched"
/// branch. That's intentional and preserved from the pre-refactor behavior: calling
/// `escalate_to_claude` itself does not mark local-only-touched for the rest of the turn.
struct EscalateToClaude;
#[async_trait]
impl Tool for EscalateToClaude {
    fn def(&self) -> ToolDef {
        ToolDef::function(
            "escalate_to_claude",
            "Hand a question to Claude Code (the user's own Pro subscription, running non-interactively on this machine) when it needs better reasoning than you can reliably provide. Pass a complete, self-contained prompt — Claude has no vault access of its own in this call, so include any relevant context you've already gathered.",
            json!({
                "type": "object",
                "properties": {
                    "prompt": { "type": "string", "description": "A complete, self-contained question or task for Claude, including any relevant context or vault content you've already gathered." }
                },
                "required": ["prompt"]
            }),
        )
    }
    fn tier(&self) -> Tier {
        Tier::LocalOnly
    }
    async fn execute(&self, args: &Value, ctx: &ToolCtx<'_>) -> anyhow::Result<String> {
        let prompt = args.get("prompt").and_then(|v| v.as_str()).unwrap_or_default();
        ctx.claude.ask(prompt).await
    }
}

/// Tier 2: deletes a vault note. Registered now that there is a confirmation gate —
/// `agent::run_turn` asks the user before any `ConfirmRequired` tool executes.
struct DeleteFile;
#[async_trait]
impl Tool for DeleteFile {
    fn def(&self) -> ToolDef {
        ToolDef::function(
            "delete_file",
            "Delete a file from the vault. Irreversible; the user is asked to confirm each time.",
            json!({
                "type": "object",
                "properties": { "path": { "type": "string", "description": "File path relative to vault root" } },
                "required": ["path"]
            }),
        )
    }
    fn tier(&self) -> Tier {
        Tier::ConfirmRequired
    }
    async fn execute(&self, args: &Value, ctx: &ToolCtx<'_>) -> anyhow::Result<String> {
        let path = args.get("path").and_then(|v| v.as_str()).unwrap_or_default();
        ctx.obsidian.delete_file(path).await?;
        Ok(format!("deleted {path}"))
    }
}

/// A tool that runs on the machine the client app is on (files and shell on the desktop), not
/// on `loki-server`'s host. Execution is a round trip through the bridge: the server emits a
/// `device_request` event and the desktop app answers. Writes/commands are `ConfirmRequired`;
/// `agent::run_turn` prompts the user before calling `execute`, so `execute` itself asks the
/// bridge with `confirm = false`.
struct DeviceTool {
    name: &'static str,
    description: &'static str,
    schema: Value,
    tier: Tier,
}

impl DeviceTool {
    fn all() -> Vec<DeviceTool> {
        vec![
            DeviceTool {
                name: "device_read_file",
                description: "Read a text file on the user's computer (the device running the Loki desktop app). Returns numbered lines.",
                schema: json!({"type":"object","properties":{
                    "path":{"type":"string","description":"Absolute path, or relative to the home directory"},
                    "offset":{"type":"integer","description":"1-based first line to return"},
                    "limit":{"type":"integer","description":"Max lines to return (default 2000)"}},"required":["path"]}),
                tier: Tier::Ambient,
            },
            DeviceTool {
                name: "device_list_dir",
                description: "List a directory on the user's computer. Directories end with '/'.",
                schema: json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}),
                tier: Tier::Ambient,
            },
            DeviceTool {
                name: "device_write_file",
                description: "Create or overwrite a file on the user's computer. The user is asked to approve each write.",
                schema: json!({"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"]}),
                tier: Tier::ConfirmRequired,
            },
            DeviceTool {
                name: "device_edit_file",
                description: "Replace text in an existing file on the user's computer. old_string must match exactly once unless replace_all is true. The user is asked to approve each edit.",
                schema: json!({"type":"object","properties":{
                    "path":{"type":"string"},"old_string":{"type":"string"},"new_string":{"type":"string"},
                    "replace_all":{"type":"boolean"}},"required":["path","old_string","new_string"]}),
                tier: Tier::ConfirmRequired,
            },
            DeviceTool {
                name: "device_run_command",
                description: "Run a shell command on the user's computer and return its output. The user is asked to approve each command.",
                schema: json!({"type":"object","properties":{
                    "command":{"type":"string"},"cwd":{"type":"string"},
                    "timeout_secs":{"type":"integer","description":"default 60, max 300"}},"required":["command"]}),
                tier: Tier::ConfirmRequired,
            },
        ]
    }
}

#[async_trait]
impl Tool for DeviceTool {
    fn def(&self) -> ToolDef {
        ToolDef::function(self.name, self.description, self.schema.clone())
    }
    fn tier(&self) -> Tier {
        self.tier
    }
    fn is_client_executed(&self) -> bool {
        true
    }
    async fn execute(&self, args: &Value, ctx: &ToolCtx<'_>) -> anyhow::Result<String> {
        debug_assert!(bridge::DEVICE_TOOLS.contains(&self.name));
        ctx.bridge.device_call(ctx.session_id, self.name, args, false).await.map_err(|e| anyhow::anyhow!(e))
    }
}

/// Finds MCP tools by keyword, so the model can discover what exists without every schema being
/// in its prompt. Returns names and one-line descriptions; the model then calls a tool directly
/// (if pinned) or through `call_tool`.
struct SearchTools {
    hub: Arc<McpHub>,
}

#[async_trait]
impl Tool for SearchTools {
    fn def(&self) -> ToolDef {
        ToolDef::function(
            "search_tools",
            "Search the many extra tools available through MCP servers (Obsidian, GitHub, Mealie, n8n, Miniflux, Zotero, OSINT, Odin, weather, web fetch, ...). Returns matching tool names with descriptions and argument names. Use call_tool to run one.",
            json!({"type":"object","properties":{"query":{"type":"string","description":"What you want to do, e.g. 'submarine cables' or 'list recipes'"}},"required":["query"]}),
        )
    }
    fn tier(&self) -> Tier {
        Tier::Ambient
    }
    async fn execute(&self, args: &Value, _ctx: &ToolCtx<'_>) -> anyhow::Result<String> {
        let query = args.get("query").and_then(|v| v.as_str()).unwrap_or_default();
        let hits = self.hub.search(query, 8);
        if hits.is_empty() {
            let servers: Vec<String> = self.hub.status().into_iter().filter(|s| s.connected).map(|s| s.name).collect();
            return Ok(format!("No tools matched '{query}'. Connected servers: {}", servers.join(", ")));
        }
        let lines: Vec<String> = hits
            .iter()
            .map(|t| {
                let args: Vec<String> = t.schema["properties"].as_object().map(|p| p.keys().cloned().collect()).unwrap_or_default();
                let desc: String = t.description.lines().next().unwrap_or("").chars().take(160).collect();
                format!("{}({}) — {desc}", t.full_name, args.join(", "))
            })
            .collect();
        Ok(lines.join("\n"))
    }
}

/// Runs any MCP tool by full name. `ToolRegistry::resolve` rewrites calls to this into the
/// target tool before gating, so this `execute` only runs for a malformed call.
struct CallTool;

#[async_trait]
impl Tool for CallTool {
    fn def(&self) -> ToolDef {
        ToolDef::function(
            "call_tool",
            "Run an MCP tool found with search_tools. Pass its full name and its arguments object. To feed one tool's output into another WITHOUT retyping it, use \"{{result:tool_name}}\" (alone or inside other text; or \"{{result:tool_name.field}}\" for a JSON field, e.g. \"{{result:get_transcript.transcript}}\") as an argument value — the full earlier result is substituted for you. Tools outside the pre-approved set ask the user to approve first.",
            json!({"type":"object","properties":{
                "name":{"type":"string","description":"Full tool name, e.g. mcp__osint__submarine_cables_in_area"},
                "arguments":{"type":"object","description":"Arguments for that tool"}},"required":["name"]}),
        )
    }
    fn tier(&self) -> Tier {
        Tier::Ambient
    }
    async fn execute(&self, _args: &Value, _ctx: &ToolCtx<'_>) -> anyhow::Result<String> {
        anyhow::bail!("call_tool needs a 'name' (use search_tools to find one)")
    }
}

/// Read-only Gmail search/listing for the LOCAL model (see `mail.rs`). Tier 3: the turn that uses it
/// can never be answered by a cloud model, and a cloud model can't call it at all.
pub struct MailInbox;

#[async_trait]
impl Tool for MailInbox {
    fn def(&self) -> ToolDef {
        ToolDef::function(
            "mail_inbox",
            "List or search the user's email (read-only). Defaults to unread inbox messages. Returns one line per message: [id] account | from | subject | age | unread | preview. Use mail_read with the id to read one. Never quote email content into notes unless they ask.",
            json!({"type":"object","properties":{
                "query":{"type":"string","description":"Gmail search, e.g. 'in:inbox is:unread', 'from:sinclair', 'newer_than:2d'"},
                "account":{"type":"string","description":"One account's address, or omit for all"},
                "limit":{"type":"integer","description":"default 30, max 100"}}}),
        )
    }
    fn tier(&self) -> Tier {
        Tier::LocalOnly
    }
    async fn execute(&self, args: &Value, _ctx: &ToolCtx<'_>) -> anyhow::Result<String> {
        crate::mail::tool_inbox(args).await.map_err(|e| anyhow::anyhow!(e))
    }
}

/// Exact count for a Gmail search (counts only), for the LOCAL model.
pub struct MailCount;

#[async_trait]
impl Tool for MailCount {
    fn def(&self) -> ToolDef {
        ToolDef::function(
            "mail_count",
            "Exact number of messages matching a Gmail search, per account. Run this before mail_mark_read, mail_label or mail_trash and pass the number as expected_count.",
            json!({"type":"object","properties":{
                "query":{"type":"string","description":"Gmail search with an operator, e.g. 'in:inbox category:updates older_than:30d'"},
                "account":{"type":"string","description":"One account's address, or omit for all"}},"required":["query"]}),
        )
    }
    fn tier(&self) -> Tier {
        Tier::LocalOnly
    }
    async fn execute(&self, args: &Value, _ctx: &ToolCtx<'_>) -> anyhow::Result<String> {
        crate::mail::tool_count(args).await.map_err(|e| anyhow::anyhow!(e))
    }
}

/// Mark read / label / trash a batch of messages. Each call is confirmed by the user and stops if the count has drifted.
pub struct MailAct(pub &'static str);

#[async_trait]
impl Tool for MailAct {
    fn def(&self) -> ToolDef {
        let (name, what) = match self.0 {
            "read" => ("mail_mark_read", "Mark up to 1000 messages matching a Gmail search as read."),
            "archive" => ("mail_archive", "Archive up to 1000 messages matching a Gmail search: they leave the inbox and stay in All Mail."),
            "label" => ("mail_label", "Add a Loki/… label (e.g. Loki/Junk, Loki/Stale, Loki/Action, Loki/Review, Loki/Keep) to up to 1000 messages matching a Gmail search."),
            _ => ("mail_trash", "Move up to 1000 messages matching a Gmail search to Trash (recoverable for 30 days; starred, sent and draft mail are skipped; never permanent)."),
        };
        let mut props = json!({
            "account":{"type":"string","description":"Exactly one account's address"},
            "query":{"type":"string","description":"Gmail search with an operator, e.g. 'in:inbox category:updates older_than:30d'"},
            "expected_count":{"type":"integer","description":"The number mail_count returned for this same query. The call stops if the real count is more than 20% higher."},
            "limit":{"type":"integer","description":"Messages per call, default 500, max 1000"}});
        if self.0 == "label" {
            props["label"] = json!({"type":"string","description":"Label name, must start with Loki/"});
        }
        let required = if self.0 == "label" { json!(["account", "query", "expected_count", "label"]) } else { json!(["account", "query", "expected_count"]) };
        ToolDef::function(name, what, json!({"type":"object","properties":props,"required":required}))
    }
    fn tier(&self) -> Tier {
        Tier::ConfirmRequired
    }
    async fn execute(&self, args: &Value, _ctx: &ToolCtx<'_>) -> anyhow::Result<String> {
        crate::mail::tool_act(self.0, args).await.map_err(|e| anyhow::anyhow!(e))
    }
}

/// Save a reply as a Gmail draft (never sent).
pub struct MailDraft;

#[async_trait]
impl Tool for MailDraft {
    fn def(&self) -> ToolDef {
        ToolDef::function(
            "mail_draft_reply",
            "Save a reply to one message as a Gmail DRAFT for the user to review and send it themselves. This cannot send anything.",
            json!({"type":"object","properties":{
                "account":{"type":"string","description":"The account address from mail_inbox"},
                "id":{"type":"string","description":"Message id from mail_inbox"},
                "body":{"type":"string","description":"Plain-text reply, in the user's voice"}},"required":["account","id","body"]}),
        )
    }
    fn tier(&self) -> Tier {
        Tier::ConfirmRequired
    }
    async fn execute(&self, args: &Value, _ctx: &ToolCtx<'_>) -> anyhow::Result<String> {
        crate::mail::tool_draft(args).await.map_err(|e| anyhow::anyhow!(e))
    }
}

/// Read one email's text (read-only, truncated), for the LOCAL model.
pub struct MailRead;

#[async_trait]
impl Tool for MailRead {
    fn def(&self) -> ToolDef {
        ToolDef::function(
            "mail_read",
            "Read one email (headers and text, truncated to ~12000 characters) by the id that mail_inbox returned. Read-only.",
            json!({"type":"object","properties":{
                "id":{"type":"string","description":"Message id from mail_inbox"},
                "account":{"type":"string","description":"The account address from mail_inbox"}},"required":["id"]}),
        )
    }
    fn tier(&self) -> Tier {
        Tier::LocalOnly
    }
    async fn execute(&self, args: &Value, _ctx: &ToolCtx<'_>) -> anyhow::Result<String> {
        crate::mail::tool_read(args).await.map_err(|e| anyhow::anyhow!(e))
    }
}

/// Loads a section of the user's standing instructions (CLAUDE.md) that the system prompt only
/// indexes.
struct ReadGuidance;

#[async_trait]
impl Tool for ReadGuidance {
    fn def(&self) -> ToolDef {
        ToolDef::function(
            "read_guidance",
            "Load a section of the user's standing instructions (CLAUDE.md) listed under 'Other sections' in your system prompt — vault conventions, infrastructure and hosts, MCP/tool notes, training and school formats. Call it before working in that area.",
            json!({"type":"object","properties":{"section":{"type":"string","description":"Section name, e.g. 'Conventions' or 'Machine & Infrastructure Access'"}},"required":["section"]}),
        )
    }
    fn tier(&self) -> Tier {
        Tier::Ambient
    }
    async fn execute(&self, args: &Value, ctx: &ToolCtx<'_>) -> anyhow::Result<String> {
        Ok(ctx.context.guidance(args.get("section").and_then(|v| v.as_str()).unwrap_or_default()))
    }
}

/// Loads one persistent-memory file named in the memory index.
struct ReadMemory;

#[async_trait]
impl Tool for ReadMemory {
    fn def(&self) -> ToolDef {
        ToolDef::function(
            "read_memory",
            "Load a persistent-memory file named in the memory index in your system prompt (e.g. 'project_loki.md') when it looks relevant to the task.",
            json!({"type":"object","properties":{"file":{"type":"string","description":"File name from the memory index"}},"required":["file"]}),
        )
    }
    fn tier(&self) -> Tier {
        Tier::Ambient
    }
    async fn execute(&self, args: &Value, ctx: &ToolCtx<'_>) -> anyhow::Result<String> {
        ctx.context.read_memory(args.get("file").and_then(|v| v.as_str()).unwrap_or_default()).map_err(|e| anyhow::anyhow!(e))
    }
}

/// Runs a shell command on another machine the user owns, over SSH (e.g. their laptop). Only hosts
/// in `[remote] hosts` are accepted; every call asks the user first (Tier 2) and shows the host and
/// the exact command. Uses the server's `~/.ssh/config` alias for the host (own key, batch mode).
struct RemoteRun {
    hosts: Vec<String>,
    preludes: std::collections::HashMap<String, String>,
}

const REMOTE_MAX_OUTPUT_CHARS: usize = 80_000;

/// Runs `command` on `host` over SSH. The script (`prelude` + `command`) is sent on stdin to
/// `bash -s`, so there is no shell-quoting between us and the remote machine and the login shell
/// doesn't matter. `ssh_bin` is overridable for tests.
pub async fn run_remote(
    ssh_bin: &str,
    hosts: &[String],
    host: &str,
    command: &str,
    prelude: &str,
    timeout_secs: u64,
) -> Result<String, String> {
    use tokio::io::AsyncWriteExt;
    if !hosts.iter().any(|h| h == host) {
        return Err(format!("'{host}' is not an allowed remote host (allowed: {})", hosts.join(", ")));
    }
    if command.trim().is_empty() {
        return Err("command is empty".to_string());
    }
    let script = if prelude.trim().is_empty() { format!("{command}\n") } else { format!("{}\n{command}\n", prelude.trim()) };
    let mut cmd = tokio::process::Command::new(ssh_bin);
    cmd.args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=10", host, "bash", "-s"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let mut child = cmd.spawn().map_err(|e| format!("couldn't run ssh: {e}"))?;
    let mut stdin = child.stdin.take().expect("piped");
    let writer = tokio::spawn(async move {
        let _ = stdin.write_all(script.as_bytes()).await;
        let _ = stdin.shutdown().await;
    });
    let out = tokio::time::timeout(std::time::Duration::from_secs(timeout_secs.clamp(1, 300)), child.wait_with_output())
        .await
        .map_err(|_| format!("timed out after {timeout_secs}s and was stopped"))?
        .map_err(|e| format!("couldn't run ssh: {e}"))?;
    let _ = writer.await;
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    let err = String::from_utf8_lossy(&out.stderr);
    if !err.trim().is_empty() {
        text.push_str(&format!("\n[stderr]\n{err}"));
    }
    if text.chars().count() > REMOTE_MAX_OUTPUT_CHARS {
        text = text.chars().take(REMOTE_MAX_OUTPUT_CHARS).collect::<String>() + "\n… [output truncated]";
    }
    if out.status.success() {
        Ok(text)
    } else {
        Err(format!("exit status {}\n{text}", out.status.code().map_or("signal".to_string(), |c| c.to_string())))
    }
}

#[async_trait]
impl Tool for RemoteRun {
    fn def(&self) -> ToolDef {
        ToolDef::function(
            "remote_run_command",
            &format!(
                "Run a shell command on another machine the user owns, over SSH. Allowed hosts: {}. The user approves every command (they see the host and the exact command), so say briefly what it does and keep commands small and specific. Use it to inspect or change that machine (e.g. a Linux laptop: read the omarchy skill first with read_skill). On their laptop the running desktop session is set up for you, so hyprctl and omarchy-* commands work. Prefer one targeted command over several exploratory ones.",
                self.hosts.join(", ")
            ),
            json!({"type":"object","properties":{
                "host":{"type":"string","description":"One of the allowed hosts"},
                "command":{"type":"string","description":"Shell command to run there"},
                "timeout_secs":{"type":"integer","description":"default 60, max 300"}},"required":["host","command"]}),
        )
    }
    fn tier(&self) -> Tier {
        Tier::ConfirmRequired
    }
    fn validate(&self, args: &Value) -> Result<(), String> {
        let host = args.get("host").and_then(|v| v.as_str()).unwrap_or_default();
        if !self.hosts.iter().any(|h| h == host) {
            return Err(format!("'{host}' is not an allowed remote host (allowed: {})", self.hosts.join(", ")));
        }
        if args.get("command").and_then(|v| v.as_str()).unwrap_or_default().trim().is_empty() {
            return Err("command is empty".to_string());
        }
        Ok(())
    }
    async fn execute(&self, args: &Value, _ctx: &ToolCtx<'_>) -> anyhow::Result<String> {
        let host = args.get("host").and_then(|v| v.as_str()).unwrap_or_default();
        let command = args.get("command").and_then(|v| v.as_str()).unwrap_or_default();
        let timeout = args.get("timeout_secs").and_then(|v| v.as_u64()).unwrap_or(60);
        let ssh = std::env::var("LOKI_SSH_BIN").unwrap_or_else(|_| "ssh".to_string());
        let prelude = self.preludes.get(host).map(String::as_str).unwrap_or("");
        run_remote(&ssh, &self.hosts, host, command, prelude, timeout).await.map_err(|e| anyhow::anyhow!(e))
    }
}

#[cfg(test)]
mod remote_tests {
    use super::*;

    fn fake_ssh(script: &str) -> String {
        let p = std::env::temp_dir().join(format!("loki-fake-ssh-{}-{}", std::process::id(), script.len()));
        std::fs::write(&p, format!("#!/bin/sh\n{script}\n")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        p.display().to_string()
    }

    #[tokio::test]
    async fn only_allowlisted_hosts_run_and_the_script_reaches_bash_intact() {
        // Reports the host/args it got and the script it was fed on stdin.
        let ssh = fake_ssh(r#"echo "args:$*"; echo "stdin:"; cat"#);
        let hosts = vec!["laptop".to_string()];
        let ok = run_remote(&ssh, &hosts, "laptop", "uname -a; echo 'q'", "export X=1", 5).await.unwrap();
        assert!(ok.contains("laptop bash -s"), "{ok}");
        assert!(ok.contains("export X=1\nuname -a; echo 'q'"), "prelude then command, unquoted: {ok}");
        let no_prelude = run_remote(&ssh, &hosts, "laptop", "id", "", 5).await.unwrap();
        assert!(no_prelude.contains("stdin:\nid\n"), "{no_prelude}");
        let denied = run_remote(&ssh, &hosts, "evilhost", "id", "", 5).await.unwrap_err();
        assert!(denied.contains("not an allowed remote host"));
        // A host that looks like an option can never get through the allow-list.
        assert!(run_remote(&ssh, &hosts, "-oProxyCommand=x", "id", "", 5).await.is_err());
        assert!(run_remote(&ssh, &hosts, "laptop", "  ", "", 5).await.is_err());
    }

    #[tokio::test]
    async fn failures_and_timeouts_are_reported() {
        let hosts = vec!["h".to_string()];
        let fail = fake_ssh("cat >/dev/null; echo boom >&2; exit 3");
        let e = run_remote(&fail, &hosts, "h", "x", "", 5).await.unwrap_err();
        assert!(e.contains("exit status 3") && e.contains("boom"), "{e}");
        let slow = fake_ssh("sleep 30");
        let started = std::time::Instant::now();
        assert!(run_remote(&slow, &hosts, "h", "x", "", 1).await.unwrap_err().contains("timed out"));
        assert!(started.elapsed() < std::time::Duration::from_secs(10));
    }
}

/// Loads a skill (expert how-to notes) listed in the system prompt's Skills index.
struct ReadSkill;

#[async_trait]
impl Tool for ReadSkill {
    fn def(&self) -> ToolDef {
        ToolDef::function(
            "read_skill",
            "Load a skill named in the Skills index in your system prompt (e.g. 'omarchy' for configuring the user's Hyprland/Omarchy desktop). Call it BEFORE doing that kind of task. With only `name` you get the skill's main text and a list of its reference files; pass `file` to load one of them.",
            json!({"type":"object","properties":{
                "name":{"type":"string","description":"Skill name from the index"},
                "file":{"type":"string","description":"Optional reference file listed by the skill, e.g. 'hyprland.md'"}},"required":["name"]}),
        )
    }
    fn tier(&self) -> Tier {
        Tier::Ambient
    }
    async fn execute(&self, args: &Value, ctx: &ToolCtx<'_>) -> anyhow::Result<String> {
        let name = args.get("name").and_then(|v| v.as_str()).unwrap_or_default();
        let file = args.get("file").and_then(|v| v.as_str());
        ctx.skills.read(name, file).map_err(|e| anyhow::anyhow!(e))
    }
}

// ---------------------------------------------------------------------------------------------
// delegate_local — hierarchy: a cloud model hands a job to the local model.

static LOCAL_PROVIDER: std::sync::OnceLock<Arc<dyn crate::providers::LlmProvider>> = std::sync::OnceLock::new();

/// The local model, once one is configured (what delegated jobs, condensing and prep packets run on).
pub fn local_provider() -> Option<Arc<dyn crate::providers::LlmProvider>> {
    LOCAL_PROVIDER.get().cloned()
}

/// Called once at startup with the first non-cloud model.
pub fn set_local_provider(p: Arc<dyn crate::providers::LlmProvider>) {
    let _ = LOCAL_PROVIDER.set(p);
}

/// Strips what must not travel up in a summary of private material: anything with an `@`, and long digit runs.
pub fn scrub_summary(s: &str, cap: usize) -> String {
    let words: Vec<String> = s
        .split(' ')
        .map(|w| {
            if w.contains('@') {
                "[address]".to_string()
            } else {
                let mut out = String::new();
                let mut run = String::new();
                for c in w.chars().chain(std::iter::once(' ')) {
                    if c.is_ascii_digit() {
                        run.push(c);
                        continue;
                    }
                    if run.len() >= 6 {
                        out.push_str("[number]");
                    } else {
                        out.push_str(&run);
                    }
                    run.clear();
                    if c != ' ' {
                        out.push(c);
                    }
                }
                out
            }
        })
        .collect();
    let joined = words.join(" ");
    if joined.chars().count() > cap {
        format!("{}… [shortened]", joined.chars().take(cap).collect::<String>())
    } else {
        joined
    }
}

/// True for an address that is not on the public internet: loopback, private ranges, link-local, carrier-grade NAT (Tailscale lives
/// there), unspecified and multicast.
fn is_internal_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v) => {
            let o = v.octets();
            v.is_loopback() || v.is_private() || v.is_link_local() || v.is_unspecified() || v.is_multicast() || v.is_broadcast() || (o[0] == 100 && (64..128).contains(&o[1])) || o[0] == 0
        }
        std::net::IpAddr::V6(v) => {
            let seg = v.segments();
            v.is_loopback() || v.is_unspecified() || v.is_multicast() || (seg[0] & 0xfe00) == 0xfc00 || (seg[0] & 0xffc0) == 0xfe80 || v.to_ipv4_mapped().is_some_and(|m| is_internal_ip(std::net::IpAddr::V4(m)))
        }
    }
}

/// Only plain public web pages: http(s), no credentials in the address, no internal host names, and no address that resolves inside this
/// network. (A page that redirects somewhere internal is stopped by the redirect policy of the client that fetches it.)
async fn check_public_url(raw: &str) -> Result<reqwest::Url, String> {
    let url = reqwest::Url::parse(raw.trim()).map_err(|_| "that is not a valid web address".to_string())?;
    if !matches!(url.scheme(), "http" | "https") || !url.username().is_empty() || url.password().is_some() {
        return Err("only plain http or https addresses are allowed".into());
    }
    let host = url.host_str().ok_or("the address has no host")?.to_lowercase();
    // Names that only mean something inside a network. Anything else is settled by where the name resolves, below.
    let internal_name = host == "localhost" || [".localhost", ".local", ".internal", ".lan", ".localdomain"].iter().any(|suffix| host.ends_with(suffix));
    if internal_name || !host.contains('.') && host.parse::<std::net::IpAddr>().is_err() && !host.starts_with('[') {
        return Err("that address is on this network, not the public web".into());
    }
    let port = url.port_or_known_default().unwrap_or(80);
    let addrs: Vec<std::net::SocketAddr> = tokio::net::lookup_host((host.trim_matches(['[', ']']), port)).await.map_err(|_| "the host could not be found".to_string())?.collect();
    if addrs.is_empty() || addrs.iter().any(|a| is_internal_ip(a.ip())) {
        return Err("that address is on this network, not the public web".into());
    }
    Ok(url)
}

/// The scout's own web fetch: a public page as plain text. Not offered to the main model (which has the MCP fetch tool); it exists so a
/// delegated job can read a page and report only what was asked for.
pub struct WebFetch;

#[async_trait]
impl Tool for WebFetch {
    fn def(&self) -> ToolDef {
        ToolDef::function(
            "web_fetch",
            "Fetch a public web page (http or https) and return its text with markup removed, up to 30000 characters. Read-only; addresses on this network are refused.",
            json!({"type":"object","properties":{"url":{"type":"string","description":"The page address"}},"required":["url"]}),
        )
    }
    fn tier(&self) -> Tier {
        Tier::Ambient
    }
    async fn execute(&self, args: &Value, _ctx: &ToolCtx<'_>) -> anyhow::Result<String> {
        let raw = args.get("url").and_then(|v| v.as_str()).unwrap_or_default();
        let mut url = check_public_url(raw).await.map_err(|e| anyhow::anyhow!(e))?;
        // Redirects are followed by hand, up to three, and every hop goes through the same public-address check as the first.
        let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(20)).user_agent("Mozilla/5.0 (compatible; Loki)").redirect(reqwest::redirect::Policy::none()).build()?;
        let mut hops = 0;
        let res = loop {
            let res = client.get(url.clone()).send().await.map_err(|e| anyhow::anyhow!("couldn't fetch the page: {e}"))?;
            if res.status().is_redirection() {
                hops += 1;
                let next = res.headers().get("location").and_then(|v| v.to_str().ok()).and_then(|l| url.join(l).ok());
                let Some(next) = next.filter(|_| hops <= 3) else { anyhow::bail!("the page redirected somewhere that can't be followed") };
                url = check_public_url(next.as_str()).await.map_err(|e| anyhow::anyhow!("a redirect was refused: {e}"))?;
                continue;
            }
            break res;
        };
        if !res.status().is_success() {
            anyhow::bail!("the page answered {}", res.status());
        }
        let is_html = res.headers().get("content-type").and_then(|v| v.to_str().ok()).map(|v| v.contains("html")).unwrap_or(true);
        let bytes = res.bytes().await.map_err(|e| anyhow::anyhow!("couldn't read the page: {e}"))?;
        let body = String::from_utf8_lossy(&bytes[..bytes.len().min(2_000_000)]).into_owned();
        let text = if is_html { crate::mail::strip_html(&body) } else { body };
        Ok(text.chars().take(30_000).collect())
    }
}

/// Marks the briefing packet inside a routine's first message.
pub const PACKET_MARK: &str = "[Briefing packet";

const PREP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(240);
const PREP_ROUNDS: usize = 12;

/// Before a daily or weekly routine runs on a cloud model, the local model does the reading the routine starts with and writes it up as one
/// packet, so the cloud model begins from a few hundred words instead of reading a dozen notes itself. Read-only, no email, and discarded
/// if anything private was touched. `None` (and the routine simply runs as before) on any failure or timeout.
pub async fn prep_packet(ctx: &ToolCtx<'_>, routine_text: &str) -> Option<String> {
    let local = local_provider()?;
    let mut reg = ToolRegistry::default_registry();
    reg.retain_read_only();
    reg.register(Box::new(WebFetch));
    let task = format!(
        "You are preparing a briefing packet for another assistant, a paid model that is about to run the routine below for the user. Do only \
         the read-only gathering the routine begins with: find and read the notes, plans, events and backlog items it will need. Do not write \
         anything, do not ask the user anything, and do not do the routine's analysis or reach conclusions. Reply with the packet: one line per \
         source, `path — the facts the routine will use` (dates, task text with its status, numbers), compact and in the order the routine \
         needs them. Under 900 words. The other assistant opens any file itself when it must edit or quote it.\n\nTHE ROUTINE:\n{routine_text}"
    );
    let sid = format!("prep-{}", ctx.session_id);
    let policy = crate::context_window::ContextPolicy { budget_tokens: crate::context_stats::get().budget_tokens(), summarized_upto: 0 };
    let run = crate::agent::run_turn(local.as_ref(), &reg, ctx, &sid, vec![Message::user(task)], &ctx.context.prompt_block, None, None, PREP_ROUNDS, policy);
    let (history, reply) = tokio::time::timeout(PREP_TIMEOUT, Box::pin(run)).await.ok()?.ok()?;
    let private = history.iter().any(|m| m.role == "tool" && m.tool_name.as_deref().is_some_and(|n| reg.tier_of(n) == Tier::LocalOnly));
    let reply = reply.trim();
    if private || reply.is_empty() {
        return None;
    }
    let chars_read: usize = history.iter().filter(|m| m.role == "tool").map(|m| m.content.chars().count()).sum();
    crate::usage_ledger::record_offload("prep", chars_read, reply.chars().count());
    Some(format!(
        "{PACKET_MARK} prepared by Loki's local model from the notes this routine reads. It saves you the reading; it can miss or misread things, so open a file yourself before you edit it or rely on its exact wording.]\n{reply}"
    ))
}

pub struct DelegateLocal;

#[async_trait]
impl Tool for DelegateLocal {
    fn def(&self) -> ToolDef {
        ToolDef::function(
            "delegate_local",
            "Hand a self-contained, read-only job to the user's local model (free to run, unlike you), which can read the vault, fetch public web pages and, when authorized, read the user's email, then returns a short written result. Use it for bulk reading, searching, sorting, counting, fetching and first-pass summaries so you spend fewer tokens, and for anything involving the user's email or finances. Ask for the form you want: key facts plus the file paths or addresses they came from, so you can open exact text yourself when you must quote or edit it. If the job touches email, the result is a short summary with addresses and long numbers removed, so ask for what you need (counts, themes, what needs action), not raw text. The local model cannot write anything.",
            json!({"type":"object","properties":{
                "task":{"type":"string","description":"Complete instructions for the local model, including what form the result should take."}},"required":["task"]}),
        )
    }
    fn tier(&self) -> Tier {
        Tier::Ambient
    }
    async fn execute(&self, args: &Value, ctx: &ToolCtx<'_>) -> anyhow::Result<String> {
        let task = args.get("task").and_then(|v| v.as_str()).unwrap_or_default().trim();
        if task.is_empty() {
            anyhow::bail!("task is empty");
        }
        let Some(local) = LOCAL_PROVIDER.get() else { anyhow::bail!("no local model is configured") };
        let mut reg = ToolRegistry::default_registry();
        if crate::mail::load_config().is_some() {
            reg.register(Box::new(MailInbox));
            reg.register(Box::new(MailRead));
            reg.register(Box::new(MailCount));
        }
        reg.retain_read_only();
        reg.register(Box::new(WebFetch));
        let wrapped = format!(
            "{task}\n\n[You are doing a delegated, read-only job for another assistant. Use the tools you need, then reply with a short written result (under 200 words). Summarize in your own words: do not quote messages, and leave out email addresses, phone numbers, account numbers and amounts of money.]"
        );
        let sid = format!("delegate-{}", ctx.session_id);
        let policy = crate::context_window::ContextPolicy { budget_tokens: crate::context_stats::get().budget_tokens(), summarized_upto: 0 };
        let (history, reply) = crate::agent::run_turn(local.as_ref(), &reg, ctx, &sid, vec![Message::user(wrapped)], &ctx.context.prompt_block, None, None, crate::agent_limits::ordinary(), policy).await?;
        let private = history.iter().any(|m| m.role == "tool" && m.tool_name.as_deref().is_some_and(|n| reg.tier_of(n) == Tier::LocalOnly));
        let rounds = history.iter().filter(|m| m.role == "tool").count();
        let chars_read: usize = history.iter().filter(|m| m.role == "tool").map(|m| m.content.chars().count()).sum();
        crate::usage_ledger::record_offload("scout", chars_read, reply.chars().count());
        if let Some(dir) = crate::paths::data_dir().map(|d| d.join("loki")) {
            use std::io::Write;
            let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("delegations.jsonl")) {
                let _ = writeln!(f, "{}", json!({"t": t, "private": private, "tool_calls": rounds, "reply_chars": reply.chars().count()}));
            }
        }
        Ok(if private { scrub_summary(&reply, 1200) } else { scrub_summary(&reply, 6000) })
    }
}

#[cfg(test)]
mod delegate_tests {
    use super::{check_public_url, is_internal_ip, scrub_summary};

    #[test]
    fn internal_addresses_are_recognised() {
        for a in ["127.0.0.1", "10.1.2.3", "192.168.7.5", "172.16.0.9", "169.254.1.1", "100.100.100.100", "0.0.0.0", "::1", "fd00::1", "fe80::1", "::ffff:192.168.0.1"] {
            assert!(is_internal_ip(a.parse().unwrap()), "{a}");
        }
        for a in ["8.8.8.8", "93.184.216.34", "2606:4700:4700::1111", "100.63.0.1", "100.128.0.1"] {
            assert!(!is_internal_ip(a.parse().unwrap()), "{a}");
        }
    }

    #[tokio::test]
    async fn only_public_web_addresses_pass() {
        for bad in ["ftp://example.com/x", "http://localhost:8420", "http://intranet/x", "http://printer.lan/", "http://192.168.7.5/", "http://user:pw@example.com/", "http://[::1]/", "file:///etc/passwd", "nonsense"] {
            assert!(check_public_url(bad).await.is_err(), "{bad}");
        }
        assert!(check_public_url("http://8.8.8.8/").await.is_ok());
    }

    #[test]
    fn scrubs_addresses_and_long_numbers() {
        let s = scrub_summary("Mail from bob@example.com about account 12345678 and room 214, total 3 items", 500);
        assert!(!s.contains("example.com") && !s.contains("12345678"));
        assert!(s.contains("[address]") && s.contains("[number]") && s.contains("room 214") && s.contains("3 items"));
    }

    #[test]
    fn caps_length() {
        assert!(scrub_summary(&"word ".repeat(500), 100).ends_with("[shortened]"));
    }
}

// ---------------------------------------------------------------------------------------------
// MCP management from chat

/// List Loki's MCP servers and their state.
pub struct McpServers;

#[async_trait]
impl Tool for McpServers {
    fn def(&self) -> ToolDef {
        ToolDef::function("mcp_servers", "List the MCP servers Loki is connected to (on or off, connected or not, how many tools). Use before changing one.", json!({"type":"object","properties":{}}))
    }
    fn tier(&self) -> Tier {
        Tier::Ambient
    }
    async fn execute(&self, _args: &Value, _ctx: &ToolCtx<'_>) -> anyhow::Result<String> {
        Ok(crate::mcp_manage::status_text())
    }
}

/// Enable, disable, reconnect, add or remove an MCP server. Every call is confirmed by the user.
pub struct McpManage;

#[async_trait]
impl Tool for McpManage {
    fn def(&self) -> ToolDef {
        ToolDef::function(
            "mcp_manage",
            "Change one of Loki's MCP servers: enable, disable, reconnect, add or remove. The user is asked to approve each call. Adding a server with a `command` runs that program on the Loki server, so only add what the user asked for, using the exact command and arguments they gave. Never put secrets in arguments; environment variables can only be set in Settings. Servers from config.toml or Claude's list can be disabled but not removed.",
            json!({"type":"object","properties":{
                "action":{"type":"string","enum":["enable","disable","reconnect","add","remove"]},
                "name":{"type":"string","description":"Server name: lowercase letters, digits, hyphens"},
                "command":{"type":"string","description":"add only: one program name or path (no spaces); options go in args"},
                "args":{"type":"array","items":{"type":"string"},"description":"add only: arguments for the command"},
                "url":{"type":"string","description":"add only: an http(s) MCP endpoint, instead of a command"}},"required":["action","name"]}),
        )
    }
    fn tier(&self) -> Tier {
        Tier::ConfirmRequired
    }
    fn validate(&self, args: &Value) -> Result<(), String> {
        if args.get("env").is_some() {
            return Err("environment variables can only be set in Settings, not through chat".into());
        }
        Ok(())
    }
    async fn execute(&self, args: &Value, _ctx: &ToolCtx<'_>) -> anyhow::Result<String> {
        let a = crate::mcp_manage::Action {
            action: args["action"].as_str().unwrap_or_default().to_string(),
            name: args["name"].as_str().unwrap_or_default().to_string(),
            command: args["command"].as_str().map(String::from),
            args: args["args"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default(),
            url: args["url"].as_str().map(String::from),
            env: Default::default(),
        };
        crate::mcp_manage::act(a).await.map_err(|e| anyhow::anyhow!(e))
    }
}
