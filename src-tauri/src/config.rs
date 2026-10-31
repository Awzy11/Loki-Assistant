use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A swappable chat/embedding backend, tagged by `vendor` in TOML (`"ollama"` |
/// `"openai_compatible"` | `"anthropic"`). The same shape backs both the `[llm]` (chat) and
/// `[embedding]` sections — they don't have to be the same vendor (e.g. a cloud chat model
/// paired with local embeddings).
///
/// Not on this list, deliberately: the user's personal Claude Code CLI subscription
/// (`crate::claude::ClaudeClient`). That's a separate, non-BYOK escalation feature — see its
/// own doc comment and `providers::LlmProvider`'s.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "vendor", rename_all = "snake_case")]
pub enum LlmBackend {
    Ollama { host: String, port: u16, model: String },
    OpenaiCompatible {
        base_url: String,
        /// Plaintext here is fine for the hand-edited bootstrap path; omit it and save
        /// through the Settings UI/API instead to store it encrypted (see `crate::secrets`
        /// and `crate::config_api`).
        #[serde(default)]
        api_key: Option<String>,
        model: String,
    },
    Anthropic {
        #[serde(default)]
        api_key: Option<String>,
        model: String,
    },
    Gemini {
        #[serde(default)]
        api_key: Option<String>,
        model: String,
    },
}

impl LlmBackend {
    pub fn vendor(&self) -> &'static str {
        match self {
            LlmBackend::Ollama { .. } => "ollama",
            LlmBackend::OpenaiCompatible { .. } => "openai_compatible",
            LlmBackend::Anthropic { .. } => "anthropic",
            LlmBackend::Gemini { .. } => "gemini",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct OllamaConfig {
    pub host: String,
    pub port: u16,
    pub model: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ObsidianConfig {
    pub host: String,
    pub port: u16,
    pub api_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeConfig {
    /// Use a Claude Code subscription as an engine. A config.toml with no `[claude]` section leaves it off; writing the
    /// section turns it on unless you set `enabled = false`.
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_claude_binary")]
    pub binary_path: String,
    #[serde(default = "default_claude_timeout_secs")]
    pub timeout_secs: u64,
    /// Working directory for Routine Mode's headless `claude` sessions specifically — the
    /// vault-aware CLAUDE.md project directory (e.g. `/home/you/Desktop`), so command
    /// files, CLAUDE.md, and the vault's MCP config resolve exactly as they do in an
    /// interactive session. No default on purpose: `escalate_to_claude` deliberately stays
    /// cwd-agnostic (it's a vault-context-free "answer one question" bridge), but Routine
    /// Mode needs this set correctly or it's silently running with the wrong context — see
    /// `ClaudeClient::run_routine_turn`, which errors clearly if this is `None`.
    #[serde(default)]
    pub routine_working_dir: Option<String>,
    /// Path to the built `loki-bridge-mcp` binary. When set, every Claude turn is spawned with
    /// it as an MCP server so Tier-2 tool calls prompt the user in the Loki UI (instead of
    /// being silently blocked) and Claude gets `device_*` tools for the desktop. Unset keeps
    /// the old behavior: anything outside the allowlist is denied.
    #[serde(default)]
    pub bridge_binary: Option<String>,
    /// Where that helper reaches `loki-server` (the helper runs on the same machine).
    #[serde(default = "default_bridge_url")]
    pub bridge_url: String,
    /// Extra tool-allowlist entries (e.g. `"mcp__zotero__*"`) appended to the built-in Tier 0/1
    /// list in `claude.rs`, so widening what Loki's Claude may do without asking is a config
    /// edit on this server rather than a rebuild. Everything not allowed still reaches the
    /// user as an approval prompt when the bridge is configured.
    #[serde(default)]
    pub extra_allowed_tools: Vec<String>,
    /// Let Claude use its built-in Bash/Read/Write/Edit/Glob/Grep **on this server** (not the
    /// desktop; that's `device_*`). They are never auto-allowed: each call asks for approval
    /// in the UI. Requires the bridge. Off by default.
    #[serde(default)]
    pub server_tools_with_approval: bool,
    /// How `Auto` uses Claude: `inference` (default) makes Claude a pure model inside Loki's
    /// loop (`providers::claude_cli`) — Loki's tools, MCP, commands and approvals, Claude only
    /// thinks; `delegated` runs Claude Code's own agent loop (`claude -p --resume`, the original
    /// behavior). Either can also be chosen per conversation from the engine picker.
    #[serde(default)]
    pub mode: ClaudeMode,
    /// Claude model aliases offered in the engine picker (`claude:<alias>`).
    #[serde(default = "default_inference_models")]
    pub inference_models: Vec<String>,
    /// Model `Auto` uses in `inference` mode; unset = the CLI's default.
    #[serde(default)]
    pub inference_model: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClaudeMode {
    Delegated,
    #[default]
    Inference,
}

fn default_inference_models() -> Vec<String> {
    ["sonnet", "opus", "haiku"].iter().map(|s| s.to_string()).collect()
}

impl Default for ClaudeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            binary_path: default_claude_binary(),
            timeout_secs: default_claude_timeout_secs(),
            routine_working_dir: None,
            bridge_binary: None,
            bridge_url: default_bridge_url(),
            extra_allowed_tools: Vec::new(),
            server_tools_with_approval: false,
            mode: ClaudeMode::Inference,
            inference_models: default_inference_models(),
            inference_model: None,
        }
    }
}

fn default_bridge_url() -> String {
    "http://127.0.0.1:8420".to_string()
}

fn default_claude_binary() -> String {
    "claude".to_string()
}

fn default_claude_timeout_secs() -> u64 {
    120
}

/// How much of the standing instructions rides in every system prompt (`[context]` in
/// config.toml). See `local_context`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextConfig {
    /// Carry only `always_sections` plus an index; the rest is loaded on demand. `false` = the
    /// whole CLAUDE.md in every prompt (the old behavior).
    #[serde(default = "default_true")]
    pub condense: bool,
    #[serde(default = "default_always_sections")]
    pub always_sections: Vec<String>,
    /// Sections preloaded into the first message of a slash-command routine.
    #[serde(default = "default_routine_sections")]
    pub routine_sections: Vec<String>,
    /// The local model's context window in tokens (what Ollama loads it with — `ollama ps`).
    #[serde(default = "default_num_ctx")]
    pub num_ctx: usize,
    /// Share of `num_ctx` spent on verbatim conversation history; the rest is for the system
    /// prompt, tool definitions, tool results and the answer. Older turns are summarized and filed
    /// for recall.
    #[serde(default = "default_history_fraction")]
    pub history_fraction: f32,
}

fn default_num_ctx() -> usize {
    24576
}

fn default_history_fraction() -> f32 {
    0.5
}

fn default_always_sections() -> Vec<String> {
    ["Context", "Vault Structure", "AI Permission Model"].iter().map(|s| s.to_string()).collect()
}

fn default_routine_sections() -> Vec<String> {
    ["Conventions", "The Two Workflows"].iter().map(|s| s.to_string()).collect()
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self { condense: true, always_sections: default_always_sections(), routine_sections: default_routine_sections(), num_ctx: default_num_ctx(), history_fraction: default_history_fraction() }
    }
}

/// Where skills live (`[skills]` in config.toml). Default: Claude Code's own `~/.claude/skills`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillsConfig {
    #[serde(default = "default_skill_dirs")]
    pub dirs: Vec<String>,
}

fn default_skill_dirs() -> Vec<String> {
    vec!["~/.claude/skills".to_string()]
}

impl Default for SkillsConfig {
    fn default() -> Self {
        Self { dirs: default_skill_dirs() }
    }
}

/// Agy (Google Antigravity CLI) as a manual plus-up engine (`[agy]` in config.toml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgyConfig {
    /// Defaults to `~/.local/bin/agy`.
    #[serde(default)]
    pub binary_path: Option<String>,
    /// Models offered in the engine picker as `agy:<model>` (`agy models` lists what's available).
    #[serde(default = "default_agy_models")]
    pub models: Vec<String>,
    #[serde(default = "default_agy_timeout")]
    pub timeout_secs: u64,
}

/// No Agy models unless you list them under `[agy]`: the engine is an optional extra, not part of a fresh setup.
fn default_agy_models() -> Vec<String> {
    Vec::new()
}

fn default_agy_timeout() -> u64 {
    120
}

impl Default for AgyConfig {
    fn default() -> Self {
        Self { binary_path: None, models: default_agy_models(), timeout_secs: default_agy_timeout() }
    }
}

/// Machines Loki may run commands on over SSH (`[remote]` in config.toml). Each name must be an
/// SSH alias configured on the server (`~/.ssh/config`), ideally with its own key. Empty = the
/// `remote_run_command` tool isn't offered at all.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RemoteConfig {
    #[serde(default)]
    pub hosts: Vec<String>,
    /// Shell snippet run before every command on a host (`[remote.preludes] laptop = '''…'''`).
    /// An SSH session doesn't inherit a desktop session's environment, so for a laptop this
    /// discovers the running Hyprland/Wayland/D-Bus session and puts `omarchy-*` on the PATH.
    /// Set by the user in config, never by the model, and not shown in approval prompts.
    #[serde(default)]
    pub preludes: std::collections::HashMap<String, String>,
}

/// One MCP server Loki connects to as a client. Either `command` (stdio) or `url`
/// (Streamable HTTP; an mcp-proxy `…/sse` URL is mapped to its `…/mcp` sibling).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub name: String,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub url: Option<String>,
}

/// Loki's own MCP client settings (`[mcp]` in config.toml). Loki, not any model vendor's CLI,
/// owns which servers exist and which of their tools are in play.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpConfig {
    /// Also load the servers registered with `claude mcp add -s user` (`~/.claude.json`), so
    /// an existing setup works with no re-entry. Explicit `servers` entries win on a name clash.
    #[serde(default = "default_true")]
    pub import_claude_servers: bool,
    /// Server names to skip (retired services, `loki-mcp` itself, …).
    #[serde(default)]
    pub disabled: Vec<String>,
    #[serde(default)]
    pub servers: Vec<McpServerConfig>,
    /// MCP tools offered to the model directly on every turn (full `mcp__server__tool` names).
    /// Everything else is found with `search_tools` and run with `call_tool`. Empty = the
    /// built-in vault set.
    #[serde(default)]
    pub pinned_tools: Vec<String>,
}

impl Default for McpConfig {
    fn default() -> Self {
        Self { import_claude_servers: true, disabled: Vec::new(), servers: Vec::new(), pinned_tools: Vec::new() }
    }
}

/// Limits on how long one turn may keep calling tools.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct AgentConfig {
    /// Tool-call rounds for an ordinary turn.
    #[serde(default = "default_max_tool_rounds")]
    pub max_tool_rounds: usize,
    /// Rounds for a named routine (`/eod`, `/bod`, … — many reads, then the writes).
    #[serde(default = "default_routine_tool_rounds")]
    pub routine_tool_rounds: usize,
    /// Let the local model reason (think) before answering. Off by default — see
    /// `providers::ollama::set_reasoning`.
    #[serde(default)]
    pub local_reasoning: bool,
    /// How long an approval prompt waits for an answer before being denied.
    #[serde(default = "default_approval_timeout")]
    pub approval_timeout_secs: u64,
}

fn default_approval_timeout() -> u64 {
    3600
}

fn default_max_tool_rounds() -> usize {
    8
}

fn default_routine_tool_rounds() -> usize {
    20
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self { max_tool_rounds: default_max_tool_rounds(), routine_tool_rounds: default_routine_tool_rounds(), local_reasoning: false, approval_timeout_secs: default_approval_timeout() }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub llm: LlmBackend,
    /// Defaults to `llm` if omitted — most setups don't need a different backend for
    /// embeddings than for chat. Ollama users typically DO want this set explicitly
    /// (a real embedding model like `nomic-embed-text`, not the chat model) — see
    /// `config.example.toml`.
    #[serde(default)]
    pub embedding: Option<LlmBackend>,
    /// Ordered backup chat models, tried after `llm` fails (quota, 429, unreachable). Written
    /// as repeated `[[fallbacks]]` tables in config.toml. Hand-edited bootstrap path only: an
    /// `api_key` here is plaintext like the other bootstrap keys (the Settings UI's encrypted
    /// secrets file covers `[llm]`/`[embedding]` only). Turns flagged local-only skip any
    /// cloud entry — see `Core::send_message_in_session_with_images`.
    #[serde(default)]
    pub fallbacks: Vec<LlmBackend>,
    /// With a local model as `llm` and a cloud provider in `fallbacks`: send everything that is not sensitive to the cloud first and keep
    /// sensitive turns on the local model (the default is local first, cloud only when the local model fails).
    #[serde(default)]
    pub cloud_first: bool,
    pub obsidian: ObsidianConfig,
    /// Optional — an existing config.toml with no [claude] section still loads fine and
    /// falls back to `claude` on PATH with a 120s timeout.
    #[serde(default)]
    pub claude: ClaudeConfig,
    #[serde(default)]
    pub mcp: McpConfig,
    #[serde(default)]
    pub context: ContextConfig,
    #[serde(default)]
    pub remote: RemoteConfig,
    #[serde(default)]
    pub skills: SkillsConfig,
    #[serde(default)]
    pub agy: AgyConfig,
    #[serde(default)]
    pub agent: AgentConfig,
    #[serde(default)]
    pub speech: crate::speech::SpeechConfig,
}

impl Config {
    pub fn embedding_backend(&self) -> &LlmBackend {
        self.embedding.as_ref().unwrap_or(&self.llm)
    }
}

/// The desktop app's *entire* configuration (2026-08-30) — it's a thin native shell around a
/// real `loki-server` instance (see `lib.rs::run`'s doc comment), not a second place running
/// its own `Core`/knowledge store/Ollama+Obsidian credentials. Deliberately not a field on
/// `Config` above: that struct requires `llm`/`obsidian` unconditionally, which a thin client
/// has no business needing on every machine it's installed on — `load_desktop` below parses
/// only the `[desktop]` table, independent of whether the rest of `config.toml`'s schema is
/// present, valid, or even exists at all.
#[derive(Debug, Clone, Deserialize)]
pub struct DesktopConfig {
    /// Where the real `Core` lives — e.g. `http://100.64.0.10:8420` (a `loki-server`
    /// instance's Tailscale IP:port; matches the pattern already proven to work for this
    /// deployment — see the README's Routine Mode / testing notes on why the Tailscale IP
    /// directly, not `https://loki.example.org/`, sidesteps the self-signed-cert trust problem a
    /// webview is stricter about than a full browser). Defaults to `127.0.0.1:8420` for the
    /// common case of running the desktop app on the same machine as `loki-server` itself.
    #[serde(default = "default_desktop_server_url")]
    pub server_url: String,
    /// Allow the device tools to create/overwrite/edit files on this machine (each still needs
    /// the user's approval in the UI). On by default.
    #[serde(default = "default_true")]
    pub device_write: bool,
    /// Allow the device tools to run shell commands on this machine. OFF by default — turn it
    /// on deliberately, here, in a file the server cannot change.
    #[serde(default)]
    pub device_shell: bool,
}

impl Default for DesktopConfig {
    fn default() -> Self {
        Self { server_url: default_desktop_server_url(), device_write: true, device_shell: false }
    }
}

fn default_true() -> bool {
    true
}

fn default_desktop_server_url() -> String {
    "http://127.0.0.1:8420".to_string()
}

/// Best-effort: a missing file, missing `[desktop]` table, or any other config.toml problem
/// all just fall back to the default — the desktop app has nothing else it strictly needs to
/// launch, unlike `load()` above (used by `loki-server`/`loki-mcp`, which have real
/// llm/obsidian dependencies and fail loudly if those are wrong).
pub fn load_desktop() -> DesktopConfig {
    #[derive(Deserialize, Default)]
    struct Wrapper {
        #[serde(default)]
        desktop: Option<DesktopConfig>,
    }
    let loaded = config_path()
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|raw| toml::from_str::<Wrapper>(&raw).ok())
        .and_then(|w| w.desktop);
    loaded.unwrap_or_default()
}

impl OllamaConfig {
    pub fn base_url(&self) -> String {
        format!("http://{}:{}", self.host, self.port)
    }
}

impl ObsidianConfig {
    pub fn base_url(&self) -> String {
        format!("http://{}:{}", self.host, self.port)
    }
}

pub(crate) fn config_dir() -> anyhow::Result<PathBuf> {
    let dir = dirs::config_dir().ok_or_else(|| anyhow::anyhow!("no config dir found for this OS"))?.join("loki");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn config_path() -> anyhow::Result<PathBuf> {
    Ok(config_dir()?.join("config.toml"))
}

pub(crate) fn secrets_path() -> anyhow::Result<PathBuf> {
    Ok(config_dir()?.join("llm.secrets.toml"))
}

/// The owned knowledge store's SQLite file — data, not config, so it lives under the OS data
/// dir rather than alongside `config.toml`/`llm.secrets.toml`.
pub fn knowledge_db_path() -> anyhow::Result<PathBuf> {
    let dir = dirs::data_dir().ok_or_else(|| anyhow::anyhow!("no data dir found for this OS"))?.join("loki");
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join("knowledge.db"))
}

#[derive(Debug, Default, Deserialize)]
struct LlmSecrets {
    #[serde(default)]
    llm: Option<SecretApiKey>,
    #[serde(default)]
    embedding: Option<SecretApiKey>,
}

#[derive(Debug, Deserialize)]
struct SecretApiKey {
    api_key: String, // ciphertext
}

pub fn load() -> anyhow::Result<Config> {
    let path = config_path()?;
    let raw = std::fs::read_to_string(&path).map_err(|e| {
        anyhow::anyhow!(
            "couldn't read config at {} ({e}). Copy config.example.toml to that path and fill in real values.",
            path.display()
        )
    })?;
    let mut config: Config = toml::from_str(&raw)?;

    // If a backend needs an api_key but config.toml didn't have one inline (the Settings
    // UI/API save path, not the hand-edited bootstrap path), fill it in from the encrypted
    // secrets file written by `config_api::save_backend`.
    if let Ok(secrets_raw) = std::fs::read_to_string(secrets_path()?) {
        let secrets: LlmSecrets = toml::from_str(&secrets_raw).unwrap_or_default();
        if let Some(s) = secrets.llm {
            fill_in_api_key(&mut config.llm, &s.api_key)?;
        }
        if let (Some(embedding), Some(s)) = (config.embedding.as_mut(), secrets.embedding) {
            fill_in_api_key(embedding, &s.api_key)?;
        }
    }

    Ok(config)
}

fn fill_in_api_key(backend: &mut LlmBackend, ciphertext: &str) -> anyhow::Result<()> {
    let slot = match backend {
        LlmBackend::OpenaiCompatible { api_key, .. } => api_key,
        LlmBackend::Anthropic { api_key, .. } => api_key,
        LlmBackend::Gemini { api_key, .. } => api_key,
        LlmBackend::Ollama { .. } => return Ok(()), // no api_key concept for this vendor
    };
    if slot.is_none() {
        *slot = Some(crate::secrets::decrypt(ciphertext)?);
    }
    Ok(())
}

#[cfg(test)]
mod claude_default_tests {
    use super::*;

    #[test]
    fn claude_is_off_without_a_section_and_on_with_one() {
        let base = "[llm]\nvendor = \"ollama\"\nhost = \"h\"\nport = 1\nmodel = \"m\"\n[obsidian]\nhost = \"h\"\nport = 2\napi_key = \"k\"\n";
        let off: Config = toml::from_str(base).unwrap();
        assert!(!off.claude.enabled && !off.cloud_first);
        let on: Config = toml::from_str(&format!("{base}[claude]\ntimeout_secs = 5\n")).unwrap();
        assert!(on.claude.enabled);
        let explicit: Config = toml::from_str(&format!("cloud_first = true\n{base}[claude]\nenabled = false\n")).unwrap();
        assert!(!explicit.claude.enabled && explicit.cloud_first);
    }
}
