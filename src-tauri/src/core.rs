use crate::agent;
use crate::agent::tools::{ToolCtx, ToolRegistry};
use crate::bridge::Bridge;
use crate::claude::ClaudeClient;
use crate::config::{self, Config};
use crate::content_classifier;
use crate::engines::{self, Engine, EngineList, EngineSettings};
use crate::events::{emit, ChatEvent, EventSink};
use crate::conversation_log;
use crate::knowledge::{IngestDoc, KnowledgeStore, QueryFilter, SearchResult, SqliteKnowledgeStore};
use crate::obsidian::ObsidianClient;
use crate::providers::{LlmProvider, Message};
use crate::router::{self, Cooldown};
use crate::session_store::{self, Persisted};
use rand::Rng;
use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};

/// One conversation's state. `busy` guards against overlapping `send_message` calls on the
/// *same* session racing on the same history (a client retry landing while the first request
/// is still in flight — happened for real 2026-08-07). Different sessions never contend with
/// each other: the outer `Core::sessions` lock is only ever held for the brief synchronous
/// bookkeeping around a call, never across the actual (slow) `agent::run_turn`.
#[derive(Default)]
struct SessionState {
    history: Vec<Message>,
    busy: bool,
    /// The active Claude Code session UUID this conversation relays clean (non-local-only)
    /// turns to via `ClaudeClient::run_routine_turn` (`--resume`) — `None` until the first
    /// turn that clears the pre-turn classifier starts one. See
    /// `send_message_in_session`'s routing logic. Despite the name (kept from when this was
    /// exclusively Routine Mode's field — see `ClaudeClient::run_routine_turn`'s doc comment
    /// on that broadened scope), this is now also the default chat backend, not just named
    /// `/command` routines.
    routine_claude_session: Option<String>,
    /// Manual override, toggled by typing `/exit`: while `true`, every turn in this session
    /// is answered locally regardless of what the pre-turn classifier says, without touching
    /// `routine_claude_session` — so a later `/exit` back to `false` can resume the same
    /// Claude session where it left off. Independent of the classifier's own per-turn
    /// local/cloud decision; this is the user's own override, not a substitute for it.
    force_local: bool,
    /// This session's vault conversation-log path (`98 - Loki Conversations/*.md`), set the
    /// first time a turn is logged — `None` beforehand means "not created yet," which is how
    /// `log_conversation_turn` decides whether to write frontmatter. See `conversation_log.rs`.
    conversation_log_path: Option<String>,
    /// The routine command name this session started with (e.g. `"bod"`), if any — captured
    /// once when Routine Mode starts (see `finish_routine_turn`) and used only to decide the
    /// `routine:` frontmatter field at the moment the conversation log file is first created.
    /// A session that starts as ordinary chat and only later runs a routine won't get this
    /// tag retroactively — the frontmatter reflects how the session started, once, not
    /// everything that happened in it (patching it in after the fact isn't worth the
    /// complexity for a label).
    routine_name: Option<String>,
    /// How many entries of `history` Claude's own CLI session has already seen (everything
    /// before this index). Anything at or past it — turns answered by other backends, or the
    /// whole transcript after a restart that lost the CLI session — is sent to Claude as a
    /// handoff block (`router::build_catchup`) on its next turn, so switching backends never
    /// needs a manual context transfer.
    claude_synced: usize,
    /// Rolling summary of transcript turns `[0, summarized_upto)` — see `session_store`.
    summary: String,
    summarized_upto: usize,
    summarizing: bool,
    title: Option<String>,
    /// Typed by the user — the AI never renames it.
    title_manual: bool,
    /// Transcript size when the AI last named it (0 = never).
    title_ai_at: usize,
    /// Transcript turn indices that stayed local for privacy; never forwarded to Claude or
    /// summarized. See `router::build_catchup`.
    private_turns: Vec<usize>,
    /// This conversation's engine choice (`engines`); `None` = the server default.
    engine: Option<String>,
    /// Thumbs up/down per assistant message index.
    feedback: std::collections::HashMap<usize, String>,
}

/// Snapshots a session's durable state to disk (and fills in its title on first use).
fn persist_state(sessions: &StdMutex<HashMap<String, SessionState>>, session_id: &str) {
    let snapshot = {
        let mut guard = sessions.lock().unwrap();
        let Some(state) = guard.get_mut(session_id) else { return };
        for m in state.history.iter_mut().filter(|m| m.at.is_none() && router::is_turn(m)) {
            m.at = Some(conversation_log::now_local());
        }
        if state.title.is_none() {
            state.title = state
                .history
                .iter()
                .find(|m| m.role == "user" && router::is_turn(m))
                .map(|m| session_store::title_from(conversation_log::strip_preamble(m.display.as_deref().unwrap_or(&m.content))));
        }
        Persisted {
            claude_session: state.routine_claude_session.clone(),
            claude_synced: state.claude_synced,
            force_local: state.force_local,
            summary: state.summary.clone(),
            summarized_upto: state.summarized_upto,
            title: state.title.clone(),
            title_manual: state.title_manual,
            title_ai_at: state.title_ai_at,
            updated: session_store::now_unix(),
            turns: router::convo_len(&state.history),
            private_turns: state.private_turns.clone(),
            engine: state.engine.clone(),
            feedback: state.feedback.clone(),
        }
    };
    session_store::save(session_id, &snapshot);
}

/// Shared application logic, transport-agnostic. Tauri's IPC commands, the web server, and
/// (Phase 2.5) the MCP server all wrap this in an `Arc` and call the same methods — one
/// implementation, multiple front doors, so none of them can drift apart from the others.
///
/// `llm`/`knowledge` are trait objects — `Core` no longer knows or cares whether the model
/// backend is Ollama or a cloud provider (Phase 1), nor whether the knowledge store is the
/// old query-only Chroma wrapper or the owned SQLite store (Phase 2).
///
/// `sessions` (Phase 2.5) replaced a single global `Mutex<Vec<Message>>` + one `busy` flag —
/// real per-session state, so an external caller (REST API, MCP) can hold its own
/// conversation without blocking or corrupting the user's own chat window, or vice versa.
pub struct Core {
    llm: Arc<dyn LlmProvider>,
    knowledge: Arc<dyn KnowledgeStore>,
    /// `Arc`, not a plain owned client — `conversation_log::log_conversation_turn` runs in a
    /// detached `tokio::spawn`'d task (fire-and-forget, same as the old chat logging it
    /// replaced), which needs an owned/`'static` handle rather than a borrow tied to the
    /// calling scope.
    obsidian: Arc<ObsidianClient>,
    needs_blurbs: Arc<std::sync::Mutex<std::collections::HashMap<String, String>>>,
    needs_blurbing: Arc<std::sync::atomic::AtomicBool>,
    titling: Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
    mail_triage: Arc<std::sync::Mutex<std::collections::HashMap<String, String>>>,
    mail_triaging: Arc<std::sync::atomic::AtomicBool>,
    mail_review: crate::mail_review::Shared,
    claude: ClaudeClient,
    /// `[remote] hosts` from config, for the session host-grant route.
    remote_hosts: Vec<String>,
    tools: ToolRegistry,
    /// Loki's own MCP client; connected in the background by `spawn_mcp_connect`.
    mcp: Arc<crate::mcp_hub::McpHub>,
    sessions: Arc<StdMutex<HashMap<String, SessionState>>>,
    /// Identity/memory block appended to the local-model system prompt — see
    /// `local_context::build`'s doc comment for what this is and why it exists. Computed once
    /// at startup, same "restart to pick up a change" convention as the rest of `Config`.
    local_context: crate::local_context::LocalContext,
    skills: crate::skills::Skills,
    /// Ordered backup chat models after `llm` — see `Config::fallbacks`.
    fallbacks: Vec<Arc<dyn LlmProvider>>,
    /// See `Config::cloud_first`.
    cloud_first: bool,
    /// Set when Claude Code reports a usage limit/overload; skips Claude until it expires.
    claude_cooldown: Cooldown,
    /// Claude CLI settings for building inference providers on demand (see `providers::claude_cli`).
    claude_cfg: crate::config::ClaudeConfig,
    agy_cfg: crate::config::AgyConfig,
    agent_cfg: crate::config::AgentConfig,
    speech_cfg: crate::speech::SpeechConfig,
    /// Tokens of conversation history sent verbatim to the local model (see `context_window`).
    /// CLI sessions used as cache vehicles by the inference provider (see `providers::claude_cli`).
    claude_sessions: Arc<crate::providers::claude_cli::ClaudeSessions>,
    /// Display labels for `llm` and each fallback, in order (engine picker).
    configured_label: String,
    fallback_labels: Vec<String>,
    /// Where to list local models from (the first Ollama endpoint in the config).
    ollama_endpoint: Option<(String, u16)>,
    engine_settings: StdMutex<EngineSettings>,
    /// Request/response channel to whoever is watching a streaming turn — see `bridge.rs`.
    bridge: Arc<Bridge>,
}

/// How long to stop trying Claude after it reports a limit. The CLI's own reset time is
/// prose we don't parse; retrying every 15 minutes costs one cheap failed spawn at worst.
/// `LOKI_CLAUDE_COOLDOWN_SECS` overrides it (used by `dev/e2e_failover.sh`).
/// Login problems are fixed by logging in, so retry soon; limits wait out the window.
fn claude_cooldown_for(error: &str) -> std::time::Duration {
    let e = error.to_lowercase();
    if ["authenticate", "oauth", "not logged in", "/login"].iter().any(|n| e.contains(n)) {
        return std::time::Duration::from_secs(60);
    }
    claude_cooldown_duration()
}

fn claude_cooldown_duration() -> std::time::Duration {
    let secs = std::env::var("LOKI_CLAUDE_COOLDOWN_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(15 * 60);
    std::time::Duration::from_secs(secs)
}

/// Clears a session's busy flag on drop — including on early return via `?` or task
/// cancellation, not just the happy path. `std::sync::Mutex`, not `tokio::sync::Mutex`, is
/// deliberate here: it's only ever locked for brief synchronous HashMap bookkeeping, never
/// held across an `.await`, which is what makes a plain synchronous `Drop` impl safe to use
/// for cleanup — an async mutex can't be locked from `drop()`.
struct SessionBusyGuard<'a> {
    sessions: &'a StdMutex<HashMap<String, SessionState>>,
    session_id: String,
}

impl Drop for SessionBusyGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut sessions) = self.sessions.lock() {
            if let Some(state) = sessions.get_mut(&self.session_id) {
                state.busy = false;
            }
        }
    }
}

#[derive(serde::Serialize)]
pub struct ChatTurn {
    pub role: String,
    pub content: String,
    /// Position among the conversation's turns — the handle for feedback and audits.
    pub index: usize,
    /// Local time sent/received (`YYYY-MM-DD HH:MM:SS TZ`), if known.
    pub at: Option<String>,
    /// "up" | "down" on assistant messages.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feedback: Option<String>,
    /// Latest Claude audit of this message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audit: Option<crate::audit::AuditRecord>,
}

/// A completed turn's reply, plus which backend actually produced it — `"claude"` for a
/// turn relayed via `ClaudeClient::run_routine_turn`, `self.llm.name()` (e.g. `"ollama"`,
/// `"gemini"`) for one answered by `agent::run_turn`, or `"system"` for a synthesized reply
/// that never touched a model at all (the `/exit` toggle confirmation). Lets a caller show a
/// per-turn "this one went to Claude / stayed local" indicator instead of only a
/// session-level one — see `is_in_routine_mode`'s doc comment on why that alone isn't
/// specific enough once a session can legitimately mix backends turn by turn.
pub struct SentTurn {
    pub reply: String,
    pub via: String,
    /// Set when the turn was answered by a fallback, e.g. "Claude Code hit its usage limit —
    /// answered by gemini. Retrying Claude in 14 min." For the UI to show beside the badge.
    pub note: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum SendMessageError {
    #[error(
        "Loki is still working on a previous message in this conversation. Wait for it to \
         finish before sending another — overlapping requests to the *same* session would \
         corrupt each other's history rather than queue politely. Other sessions aren't \
         affected by this."
    )]
    Busy,
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

fn new_session_id() -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let suffix: u32 = rand::thread_rng().gen();
    format!("session-{now}-{suffix:x}")
}

/// A real RFC 4122 v4 UUID, required by `claude --session-id` ("must be a valid UUID") — not
/// the same shape as `new_session_id()`, which is Loki's own conversation-key format. Built
/// from `rand`/`OsRng` (already a dependency, same pattern `auth.rs` uses) rather than adding
/// a `uuid` crate dependency for one string.
fn new_claude_session_uuid() -> String {
    let mut bytes = [0u8; 16];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut bytes);
    bytes[6] = (bytes[6] & 0x0f) | 0x40; // version 4
    bytes[8] = (bytes[8] & 0x3f) | 0x80; // variant 10 (RFC 4122)
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15]
    )
}

/// Toggles `SessionState::force_local` — since Claude Code is the default per-turn backend
/// (2026-09-01), this now means "force local-only for this conversation" the first time it's
/// typed, and "go back to Claude Code by default" the next.
const ROUTINE_EXIT_COMMAND: &str = "/exit";

impl Core {
    pub fn new(config: Config) -> Self {
        // Build the provider handles first — they only need shared borrows of
        // config.llm/config.embedding — before partially moving the rest of `config` by value.
        let llm = crate::providers::factory::build(&config.llm);
        let embedder = crate::providers::factory::build(config.embedding_backend());
        let db_path = config::knowledge_db_path().expect("couldn't determine knowledge store path");
        let knowledge: Arc<dyn KnowledgeStore> = Arc::new(
            SqliteKnowledgeStore::open(&db_path, embedder, llm.clone())
                .expect("couldn't open the knowledge store — see the error above"),
        );
        let obsidian = Arc::new(ObsidianClient::new(config.obsidian));
        let fallbacks: Vec<Arc<dyn LlmProvider>> = config.fallbacks.iter().map(crate::providers::factory::build).collect();
        if let Some(local) = std::iter::once(&llm).chain(fallbacks.iter()).find(|p| !p.is_cloud()) {
            crate::agent::tools::set_local_provider(local.clone());
        }
        let cloud_first = config.cloud_first;
        let claude_cfg = config.claude.clone();
        let agy_cfg = config.agy.clone();
        let agent_cfg = config.agent.clone();
        crate::agent_limits::init(agent_cfg.max_tool_rounds.max(25), agent_cfg.routine_tool_rounds.max(60));
        let speech_cfg = config.speech.clone();
        crate::context_stats::init(&config.context);
        crate::providers::ollama::set_reasoning(config.agent.local_reasoning);
        crate::bridge::set_approval_timeout(config.agent.approval_timeout_secs);
        if let crate::config::LlmBackend::Ollama { host, port, model } = &config.llm {
            crate::providers::ollama::spawn_preload(format!("http://{host}:{port}"), model.clone(), config.context.num_ctx);
        }
        let configured_label = engines::backend_label(&config.llm);
        let fallback_labels = config.fallbacks.iter().map(engines::backend_label).collect();
        let ollama_endpoint = std::iter::once(&config.llm).chain(config.fallbacks.iter()).find_map(|b| match b {
            crate::config::LlmBackend::Ollama { host, port, .. } => Some((host.clone(), *port)),
            _ => None,
        });
        let mcp = crate::mcp_hub::McpHub::new(crate::mcp_manage::effective_specs(&config.mcp));
        crate::mcp_manage::init(mcp.clone(), config.mcp.clone());
        let remote_hosts = config.remote.hosts.clone();
        let tools = ToolRegistry::with_mcp(
            mcp.clone(),
            crate::tool_policy::ToolPolicy::new(&config.claude.extra_allowed_tools),
            config.mcp.pinned_tools.clone(),
        )
        .with_remote(config.remote.hosts.clone(), config.remote.preludes.clone());
        // Email (read-only, local models only) — registered only once an account has been authorized.
        let mut tools = tools;
        if crate::mail::load_config().is_some() {
            tools.register(Box::new(crate::agent::tools::MailInbox));
            tools.register(Box::new(crate::agent::tools::MailRead));
            tools.register(Box::new(crate::agent::tools::MailCount));
            tools.register(Box::new(crate::agent::tools::MailAct("read")));
            tools.register(Box::new(crate::agent::tools::MailAct("label")));
            tools.register(Box::new(crate::agent::tools::MailAct("trash")));
            tools.register(Box::new(crate::agent::tools::MailAct("archive")));
            tools.register(Box::new(crate::agent::tools::MailDraft));
        }
        tools.register(Box::new(crate::agent::tools::DelegateLocal));
        tools.register(Box::new(crate::agent::tools::McpServers));
        tools.register(Box::new(crate::agent::tools::McpManage));
        let routine_working_dir = config.claude.routine_working_dir.clone();
        let claude = ClaudeClient::new(config.claude).with_mcp_servers(mcp.claude_servers()).with_activity_log(obsidian.clone());
        let mut local_context = crate::local_context::build(routine_working_dir.as_deref(), &config.context);
        let skills = crate::skills::Skills::load(&config.skills.dirs);
        local_context.prompt_block.push_str(&skills.prompt_block());
        Self {
            llm,
            knowledge,
            obsidian,
            needs_blurbs: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
            needs_blurbing: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            titling: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
            mail_triage: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
            mail_triaging: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            mail_review: Arc::new(std::sync::Mutex::new(crate::mail_review::load())),
            claude,
            remote_hosts,
            tools,
            mcp,
            sessions: Arc::new(StdMutex::new(HashMap::new())),
            local_context,
            skills,
            fallbacks,
            cloud_first,
            claude_cfg,
            agy_cfg,
            agent_cfg,
            speech_cfg,
            claude_sessions: Arc::new(Default::default()),
            configured_label,
            fallback_labels,
            ollama_endpoint,
            engine_settings: StdMutex::new(EngineSettings::load()),
            claude_cooldown: Cooldown::default(),
            bridge: Arc::new(Bridge::new(
                Arc::new(match dirs::data_dir() {
                    Some(d) => crate::permission_rules::PermissionRules::load(d.join("loki").join("always_allow.json")),
                    None => crate::permission_rules::PermissionRules::in_memory(),
                }),
                Arc::new(crate::bus::Bus::default()),
            )),
        }
    }

    /// Names a conversation with the local model after its first exchange, and again when it has
    /// roughly tripled — unless the user has named it (`rename_session`), which is final.
    fn maybe_auto_title(&self, session_id: &str) {
        let Some(provider) = std::iter::once(&self.llm).chain(self.fallbacks.iter()).find(|p| !p.is_cloud()).cloned() else { return };
        let (first_user, first_asst, last_user, len) = {
            let sessions = self.sessions.lock().unwrap();
            let Some(state) = sessions.get(session_id) else { return };
            if state.title_manual {
                return;
            }
            let len = router::convo_len(&state.history);
            let due = (state.title_ai_at == 0 && len >= 2) || (state.title_ai_at > 0 && len >= state.title_ai_at * 3 && len >= 8);
            if !due {
                return;
            }
            let text = |m: &Message| conversation_log::strip_preamble(m.display.as_deref().unwrap_or(&m.content)).trim().to_string();
            let convo: Vec<&Message> = state.history.iter().filter(|m| router::is_turn(m)).collect();
            let fu = convo.iter().find(|m| m.role == "user").map(|m| text(m)).unwrap_or_default();
            let fa = convo.iter().find(|m| m.role == "assistant").map(|m| m.content.clone()).unwrap_or_default();
            let lu = convo.iter().rev().find(|m| m.role == "user").map(|m| text(m));
            (fu, fa, lu, len)
        };
        if first_user.is_empty() || !self.titling.lock().unwrap().insert(session_id.to_string()) {
            return;
        }
        let sessions = self.sessions.clone();
        let titling = self.titling.clone();
        let session_id = session_id.to_string();
        tokio::spawn(async move {
            let title = session_store::generate_title(&provider, &first_user, &first_asst, last_user.as_deref()).await;
            {
                let mut guard = sessions.lock().unwrap();
                if let Some(state) = guard.get_mut(&session_id) {
                    if !state.title_manual {
                        if let Some(t) = &title {
                            state.title = Some(t.clone());
                        }
                        state.title_ai_at = len;
                    }
                }
            }
            persist_state(&sessions, &session_id);
            titling.lock().unwrap().remove(&session_id);
        });
    }

    /// One-time (per conversation) pass over saved conversations that were never AI-named: a title that
    /// still equals the first message's opening is replaced by an AI title; anything else is treated as
    /// typed by the user and left alone (and marked manual). Newest first, one at a time, in the background.
    pub fn spawn_title_backfill(self: &Arc<Self>) {
        let core = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(25)).await;
            let Some(provider) = std::iter::once(&core.llm).chain(core.fallbacks.iter()).find(|p| !p.is_cloud()).cloned() else { return };
            let mut done = 0;
            for s in session_store::list().into_iter().filter(|s| s.turns >= 2).take(120) {
                if done >= 60 {
                    break;
                }
                let Some(mut p) = session_store::load(&s.session_id) else { continue };
                if p.title_manual || p.title_ai_at > 0 {
                    continue;
                }
                let Some(path) = conversation_log::find_log_path_for_session(&core.obsidian, &s.session_id).await else { continue };
                let Ok(history) = conversation_log::load_history_from_log(&core.obsidian, &path).await else { continue };
                let text = |m: &Message| conversation_log::strip_preamble(m.display.as_deref().unwrap_or(&m.content)).trim().to_string();
                let convo: Vec<&Message> = history.iter().filter(|m| router::is_turn(m)).collect();
                let Some(first_user) = convo.iter().find(|m| m.role == "user").map(|m| text(m)) else { continue };
                let fallback = session_store::title_from(&first_user);
                let cur = p.title.clone().unwrap_or_default();
                if !cur.is_empty() && cur != fallback && cur != "Untitled conversation" {
                    p.title_manual = true; // someone typed it
                    session_store::save(&s.session_id, &p);
                    continue;
                }
                let first_asst = convo.iter().find(|m| m.role == "assistant").map(|m| m.content.clone()).unwrap_or_default();
                let last_user = convo.iter().rev().find(|m| m.role == "user").map(|m| text(m));
                if let Some(t) = session_store::generate_title(&provider, &first_user, &first_asst, last_user.as_deref()).await {
                    p.title = Some(t.clone());
                    p.title_ai_at = convo.len().max(2);
                    session_store::save(&s.session_id, &p);
                    if let Some(state) = core.sessions.lock().unwrap().get_mut(&s.session_id) {
                        if !state.title_manual {
                            state.title = Some(t);
                            state.title_ai_at = p.title_ai_at;
                        }
                    }
                    done += 1;
                }
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
            if done > 0 {
                eprintln!("[loki-debug] titled {done} older conversation(s) with the local model");
            }
        });
    }

    /// Runs `knowledge::reconcile::reconcile_deleted_notes` once immediately, then on a 6-hour
    /// interval for the life of the process — deletion happens entirely outside Loki (see that
    /// module's doc comment), so this periodic sweep is the only thing that ever catches up
    /// with it. Called once from each binary's startup (`bin/server.rs`, `lib.rs::run`) right
    /// after `Core` is wrapped in an `Arc`.
    pub fn spawn_knowledge_reconciliation(self: &Arc<Self>) {
        let core = self.clone();
        tokio::spawn(async move {
            loop {
                match crate::knowledge::reconcile::reconcile_deleted_notes(core.knowledge.as_ref(), core.obsidian.as_ref()).await {
                    Ok(0) => {}
                    Ok(n) => eprintln!("[loki-debug] knowledge reconciliation: removed {n} orphaned note row(s)"),
                    Err(e) => eprintln!("[loki-debug] knowledge reconciliation failed (non-fatal): {e}"),
                }
                tokio::time::sleep(std::time::Duration::from_secs(6 * 3600)).await;
            }
        });
    }

    /// If `session_id` isn't in memory yet (a fresh process, or the first lookup of a
    /// `session_id` a client persisted from a previous run), attempts to seed it from that
    /// session's vault conversation log before it's touched — restores **display history**
    /// only (see `conversation_log::load_history_from_log`'s doc comment for why this
    /// deliberately doesn't also restore live Routine Mode state). A no-op, cheaply, for a
    /// session already in memory or one with no matching log file (brand new conversation).
    async fn ensure_session_seeded(&self, session_id: &str) {
        if self.sessions.lock().unwrap().contains_key(session_id) {
            return;
        }
        let persisted = session_store::load(session_id);
        // If we have saved state for this session its log must exist, so a miss is most likely
        // a transient vault/REST hiccup (e.g. right after a restart): retry briefly rather than
        // continuing with an empty history.
        let attempts = if persisted.is_some() { 3 } else { 1 };
        let mut found = None;
        for attempt in 0..attempts {
            if attempt > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(400)).await;
            }
            found = match conversation_log::find_log_path_for_session(&self.obsidian, session_id).await {
                Some(path) => conversation_log::load_history_from_log(&self.obsidian, &path).await.ok().map(|h| (h, Some(path))),
                None => None,
            };
            if found.is_some() {
                break;
            }
        }
        // Nothing on disk or in the vault: a brand-new conversation.
        if found.is_none() && persisted.is_none() {
            return;
        }
        let (history, path) = found.unwrap_or_default();
        // The vault log holds what was typed (`/bod`); put the instructions back so a routine
        // in progress keeps its context across a restart.
        let history: Vec<Message> = history
            .into_iter()
            .map(|mut m| {
                if m.role == "user" {
                    if let Some(e) = crate::commands::expand(&m.content) {
                        m.display = Some(std::mem::replace(&mut m.content, e.text));
                    }
                }
                m
            })
            .collect();
        let p = persisted.unwrap_or_default();
        let mut sessions = self.sessions.lock().unwrap();
        sessions.entry(session_id.to_string()).or_insert_with(|| SessionState {
            history,
            conversation_log_path: path,
            // Restores the Claude CLI session across restarts (`--resume`); if the CLI no
            // longer has it, the turn falls back to a fresh session plus a full handoff.
            routine_claude_session: p.claude_session,
            claude_synced: p.claude_synced,
            force_local: p.force_local,
            summary: p.summary,
            summarized_upto: p.summarized_upto,
            title: p.title,
            title_manual: p.title_manual,
            title_ai_at: p.title_ai_at,
            private_turns: p.private_turns,
            engine: p.engine,
            feedback: p.feedback,
            ..Default::default()
        });
    }

    /// Generates a fresh session id — used by every caller (Tauri, web, REST/MCP) that wants
    /// a new, independent conversation. There's no shared default session anymore — see the
    /// removed `Core::send_message`/`Core::get_history` and their matching legacy HTTP routes
    /// (`/api/message`/`/api/history`) and Tauri commands, deleted once the frontend finished
    /// migrating to the session-scoped, bearer-token-authenticated v1 path and nothing called
    /// the unauthenticated legacy path anymore — it had stopped being a real "no token
    /// plumbing exists yet" tradeoff and become unnecessary open attack surface (full chat +
    /// real vault reads/writes, unauthenticated, reachable by anything on the LAN/Tailscale).
    // ---- engine picker ----

    /// The engine this conversation uses: its own choice, else the server default, else `auto`.
    pub fn effective_engine(&self, session_id: &str) -> String {
        let own = self.sessions.lock().unwrap().get(session_id).and_then(|s| s.engine.clone());
        own.or_else(|| self.engine_settings.lock().unwrap().default_engine.clone()).unwrap_or_else(|| engines::AUTO.to_string())
    }

    /// One plain sentence on what `Auto` does with the current setup, for the chat header.
    pub fn auto_summary(&self) -> String {
        let local = !self.llm.is_cloud() || self.fallbacks.iter().any(|p| !p.is_cloud());
        let cloud = self.llm.is_cloud() || self.fallbacks.iter().any(|p| p.is_cloud());
        if self.claude_cfg.enabled {
            return "Auto: Claude when it helps, a local model for anything sensitive".into();
        }
        match (local, cloud) {
            (true, true) if self.cloud_first => "Auto: your cloud provider, and a local model for anything sensitive".into(),
            (true, true) => "Auto: your local model, with your cloud provider as backup".into(),
            (false, true) => "Using your cloud provider for everything (no local model set up)".into(),
            _ => "Using your local model; nothing leaves this machine".into(),
        }
    }

    pub async fn list_engines(&self, session_id: Option<&str>) -> EngineList {
        let mut list = vec![Engine { id: engines::AUTO.into(), label: "Auto (recommended)".into(), kind: "auto", cloud: false }];
        if self.claude_cfg.enabled {
            list.push(Engine { id: engines::CLAUDE_CODE.into(), label: "Claude Code (its own agent loop)".into(), kind: "claude_code", cloud: true });
            for m in &self.claude_cfg.inference_models {
                list.push(Engine { id: format!("claude:{m}"), label: format!("Claude {m} (Loki's loop)"), kind: "claude", cloud: true });
            }
        }
        for m in &self.agy_cfg.models {
            list.push(Engine { id: format!("agy:{m}"), label: format!("Agy {m} (Loki's loop)"), kind: "agy", cloud: true });
        }
        list.push(Engine {
            id: engines::CONFIGURED.into(),
            label: format!("Configured model: {}", self.configured_label),
            kind: "configured",
            cloud: self.llm.is_cloud(),
        });
        for (i, label) in self.fallback_labels.iter().enumerate() {
            list.push(Engine { id: format!("fallback:{i}"), label: format!("Fallback: {label}"), kind: "configured", cloud: self.fallbacks[i].is_cloud() });
        }
        if let Some((host, port)) = &self.ollama_endpoint {
            for m in engines::ollama_models(host, *port).await {
                list.push(Engine { id: format!("ollama:{m}"), label: format!("Ollama: {m}"), kind: "ollama", cloud: false });
            }
        }
        let session = session_id.and_then(|id| self.sessions.lock().unwrap().get(id).and_then(|s| s.engine.clone()));
        EngineList {
            engines: list,
            default: self.engine_settings.lock().unwrap().default_engine.clone().unwrap_or_else(|| engines::AUTO.to_string()),
            session,
            claude_mode: self.claude_cfg.mode,
            summary: self.auto_summary(),
        }
    }

    /// Sets this conversation's engine (`session_id` given; `"auto"` clears it back to the
    /// default) or the server-wide default (`session_id` is `None`).
    pub async fn set_engine(&self, session_id: Option<&str>, engine: &str) -> Result<(), String> {
        let valid = match engine {
            engines::AUTO | engines::CLAUDE_CODE | engines::CONFIGURED => true,
            e if e.starts_with("fallback:") => e[9..].parse::<usize>().map(|i| i < self.fallbacks.len()).unwrap_or(false),
            e if e.starts_with("claude:") => self.claude_cfg.inference_models.iter().any(|m| m == &e[7..]),
            e if e.starts_with("agy:") => self.agy_cfg.models.iter().any(|m| m == &e[4..]),
            e if e.starts_with("ollama:") => e.len() > 7 && self.ollama_endpoint.is_some(),
            _ => false,
        };
        if !valid {
            return Err(format!("unknown engine '{engine}'"));
        }
        match session_id {
            Some(id) => {
                self.ensure_session_seeded(id).await;
                {
                    let mut sessions = self.sessions.lock().unwrap();
                    let state = sessions.entry(id.to_string()).or_default();
                    state.engine = if engine == engines::AUTO { None } else { Some(engine.to_string()) };
                }
                persist_state(&self.sessions, id);
            }
            None => {
                let mut s = self.engine_settings.lock().unwrap();
                s.default_engine = if engine == engines::AUTO { None } else { Some(engine.to_string()) };
                s.save();
            }
        }
        Ok(())
    }

    /// Claude as an inference engine for this conversation: runs in a resumed CLI session kept
    /// per (conversation, model) so the prompt cache is reused incrementally.
    fn claude_inference(&self, model: Option<String>, session_id: &str) -> Arc<dyn LlmProvider> {
        let key = format!("{session_id}|{}", model.as_deref().unwrap_or("default"));
        Arc::new(crate::providers::ClaudeCliProvider::with_sessions(
            self.claude_cfg.binary_path.clone(),
            model,
            self.claude_cfg.timeout_secs,
            Some((self.claude_sessions.clone(), key)),
        ))
    }

    /// A specific (non-`auto`, non-`claude-code`) engine as a provider.
    fn provider_for_engine(&self, id: &str, session_id: &str) -> Option<Arc<dyn LlmProvider>> {
        if id == engines::CONFIGURED {
            return Some(self.llm.clone());
        }
        if let Some(i) = id.strip_prefix("fallback:").and_then(|n| n.parse::<usize>().ok()) {
            return self.fallbacks.get(i).cloned();
        }
        if let Some(model) = id.strip_prefix("agy:") {
            let bin = self.agy_cfg.binary_path.clone().unwrap_or_else(|| {
                dirs::home_dir().map(|h| h.join(".local/bin/agy").display().to_string()).unwrap_or_else(|| "agy".to_string())
            });
            return Some(Arc::new(crate::providers::AgyCliProvider::new(bin, Some(model.to_string()), self.agy_cfg.timeout_secs)));
        }
        if let Some(model) = id.strip_prefix("claude:") {
            return Some(self.claude_inference(Some(model.to_string()), session_id));
        }
        if let (Some(model), Some((host, port))) = (id.strip_prefix("ollama:"), &self.ollama_endpoint) {
            return Some(crate::providers::factory::build(&crate::config::LlmBackend::Ollama {
                host: host.clone(),
                port: *port,
                model: model.to_string(),
            }));
        }
        None
    }

    /// Connects every configured MCP server in the background (see `McpHub::spawn_connect_all`).
    /// Called once from `loki-server`'s startup; other binaries that embed a `Core` skip it.
    pub fn spawn_mcp_connect(&self) {
        self.mcp.spawn_connect_all();
    }

    /// SSH hosts Loki may run commands on (`[remote] hosts`).
    pub fn remote_hosts(&self) -> &[String] {
        &self.remote_hosts
    }

    /// A read-only MCP tool for an n8n workflow (see `/api/v1/tools/call`). Only the tools on this list can be called this way.
    pub async fn call_workflow_tool(&self, name: &str, args: &serde_json::Value) -> Result<String, String> {
        const ALLOWED: &[&str] = &["mcp__youtube-transcript__get_transcript", "mcp__youtube-transcript__get_video_info", "mcp__youtube-transcript__get_timed_transcript"];
        if !ALLOWED.contains(&name) {
            return Err(format!("'{name}' is not available to workflows"));
        }
        let call = |a: serde_json::Value| async move {
            tokio::time::timeout(std::time::Duration::from_secs(90), self.mcp.call(name, &a))
                .await
                .map_err(|_| "the tool took longer than 90 seconds".to_string())?
                .map(|t| crate::needs_you::unwrap_result(&t))
                .map_err(|e| e.to_string())
        };
        if name != "mcp__youtube-transcript__get_transcript" {
            return call(args.clone()).await;
        }
        // A long video's transcript comes in pages; collect them (up to 8) into one answer.
        let (mut title, mut text, mut cursor, mut truncated) = (String::new(), String::new(), None::<String>, false);
        for page in 0..8 {
            let mut a = args.clone();
            if let (Some(c), Some(o)) = (&cursor, a.as_object_mut()) {
                o.insert("next_cursor".into(), serde_json::Value::String(c.clone()));
            }
            let v: serde_json::Value = serde_json::from_str(&call(a).await?).map_err(|e| format!("unexpected transcript reply: {e}"))?;
            if title.is_empty() {
                title = v["title"].as_str().unwrap_or("").to_string();
            }
            text.push_str(v["transcript"].as_str().unwrap_or(""));
            text.push('\n');
            cursor = v["next_cursor"].as_str().filter(|c| !c.is_empty()).map(String::from);
            if cursor.is_none() {
                break;
            }
            truncated = page == 7;
        }
        Ok(serde_json::json!({ "title": title, "transcript": text.trim(), "truncated": truncated }).to_string())
    }

    pub fn mcp_status(&self) -> Vec<crate::mcp_hub::ServerStatus> {
        self.mcp.status()
    }

    pub fn bridge(&self) -> &Arc<Bridge> {
        &self.bridge
    }

    /// The per-conversation event bus every client attaches to.
    pub fn bus(&self) -> &Arc<crate::bus::Bus> {
        self.bridge.bus()
    }

    pub fn new_session(&self) -> String {
        new_session_id()
    }

    /// `allow_routines` gates whether this turn can be routed to Claude Code at all, not just
    /// a default — it's the actual enforcement point (not convention, not session-id
    /// pattern-matching) for keeping that surface behind `/api/v1/chat`'s bearer-token auth.
    /// Callers: `server::v1_chat` (always `true`), `mcp_server`'s `loki_chat` (always `false`
    /// — see its own comment for why) and the Tauri `send_message_v1` command (always `true`
    /// — IPC is its own trust boundary, no token needed there).
    pub async fn send_message_in_session(
        &self,
        session_id: &str,
        message: String,
        allow_routines: bool,
    ) -> Result<SentTurn, SendMessageError> {
        self.send_message_in_session_with_images(session_id, message, allow_routines, Vec::new()).await
    }

    /// Same as `send_message_in_session`, plus base64-encoded images (e.g. a device-context
    /// screenshot, #13) attached to this turn. Images only reach the local-model path
    /// (`agent::run_turn`, via `Message::images` — see its doc comment on why only the Ollama
    /// provider actually forwards them today) — if a routine is active, this turn's text still
    /// relays to Claude as usual, but any attached image is silently dropped for that turn,
    /// since the Claude CLI relay has no image channel here.
    pub async fn send_message_in_session_with_images(
        &self,
        session_id: &str,
        message: String,
        allow_routines: bool,
        images: Vec<String>,
    ) -> Result<SentTurn, SendMessageError> {
        self.send_message_streaming(session_id, message, allow_routines, images, None).await
    }

    /// The full implementation behind the two wrappers above. With a `sink`, the turn emits
    /// live `ChatEvent`s (text deltas, tool calls/results, status, usage) while it runs.
    /// Dropping this future cancels the turn: the Claude child process is killed
    /// (`kill_on_drop`) and the session's busy flag is released.
    pub async fn send_message_streaming(
        &self,
        session_id: &str,
        message: String,
        allow_routines: bool,
        images: Vec<String>,
        sink: Option<EventSink>,
    ) -> Result<SentTurn, SendMessageError> {
        let sink = sink.as_ref();
        self.ensure_session_seeded(session_id).await;
        // `/cloud <message>`: the user explicitly sends this one message past the privacy gate (credentials
        // and ID numbers still never leave the machine).
        let (message, cloud_override) = match message.trim_start().strip_prefix("/cloud") {
            Some(rest) if rest.is_empty() || rest.starts_with(char::is_whitespace) => {
                if rest.trim().is_empty() {
                    return Err(SendMessageError::Other(anyhow::anyhow!("type your message after /cloud, e.g. `/cloud summarize my workout`")));
                }
                (rest.trim().to_string(), true)
            }
            _ => (message, false),
        };
        if allow_routines {
            let info_reply = if message.trim().eq_ignore_ascii_case("/needsyou") {
                Some(crate::needs_you::render(&crate::needs_you::build(&self.obsidian).await))
            } else {
                self.info_command(session_id, &message)
            };
            if let Some(reply) = info_reply {
                let (history, force_local) = {
                    let sessions = self.sessions.lock().unwrap();
                    sessions.get(session_id).map(|s| (s.history.clone(), s.force_local)).unwrap_or_default()
                };
                let mut history = history;
                let mut user = Message::user(message.clone());
                user.at = Some(conversation_log::now_local());
                history.push(user);
                return self.finish_toggle_turn(session_id, history, reply, force_local, &message).await.map(|reply| SentTurn { reply, via: "system".to_string(), note: None });
            }
        }
        if allow_routines && message.trim().eq_ignore_ascii_case("/compact") {
            let reply = match self.compact_session(session_id).await {
                Ok(text) => text,
                Err(e) => e,
            };
            let (history, force_local) = {
                let sessions = self.sessions.lock().unwrap();
                sessions.get(session_id).map(|s| (s.history.clone(), s.force_local)).unwrap_or_default()
            };
            let mut history = history;
            let mut user = Message::user(message.clone());
            user.at = Some(conversation_log::now_local());
            history.push(user);
            return self.finish_toggle_turn(session_id, history, reply, force_local, &message).await.map(|reply| SentTurn { reply, via: "system".to_string(), note: None });
        }
        // Slash commands: Loki expands `~/.claude/commands/<name>.md` itself so they work on any
        // engine. The delegated Claude Code path leaves the text alone — the CLI expands it.
        let expansion = {
            let engine_now = self.effective_engine(session_id);
            let delegated_now = engine_now == engines::CLAUDE_CODE
                || (engine_now == engines::AUTO && self.claude_cfg.enabled && self.claude_cfg.mode == crate::config::ClaudeMode::Delegated);
            if allow_routines && !delegated_now { crate::commands::expand(&message) } else { None }
        };
        let (history, active_routine_session, force_local, claude_synced, summary, summarized_upto, private_turns) = {
            let mut sessions = self.sessions.lock().unwrap();
            let state = sessions.entry(session_id.to_string()).or_default();
            if state.busy {
                return Err(SendMessageError::Busy);
            }
            state.busy = true;
            let mut user_msg = if images.is_empty() {
                Message::user(message.clone())
            } else {
                Message::user_with_images(message.clone(), images)
            };
            if let Some(e) = &expansion {
                // The model gets the full instructions; people (UI, vault log, title) see `/bod`.
                user_msg.content = format!("{}{}", self.local_context.routine_block().unwrap_or_default(), e.text);
                user_msg.display = Some(message.clone());
                state.routine_name.get_or_insert_with(|| e.name.clone());
            }
            user_msg.at = Some(conversation_log::now_local());
            state.history.push(user_msg);
            (
                state.history.clone(),
                state.routine_claude_session.clone(),
                state.force_local,
                state.claude_synced,
                state.summary.clone(),
                state.summarized_upto,
                state.private_turns.clone(),
            )
        };
        let _release = SessionBusyGuard { sessions: &self.sessions, session_id: session_id.to_string() };
        // Let every other attached client (phone, desktop, sidebar, web) show this message.
        if let Some(s) = sink {
            emit(Some(s), ChatEvent::UserMessage { text: message.clone(), client_id: s.client_id.clone() });
        }

        // Set when this turn must not reach a cloud model: the `/exit` override, or the
        // pre-turn classifier flagging the message. Restricts the fallback chain below.
        let mut local_only = force_local;
        // Why this turn stays local (shown to the user whenever the privacy gate holds a message back).
        let mut local_reason: Option<String> = None;
        // Why Claude was skipped, if it was — becomes `SentTurn::note`.
        let mut claude_skipped: Option<String> = None;
        // Which engine this conversation uses (the picker); `auto` = normal routing.
        let engine = self.effective_engine(session_id);
        // The delegated Claude Code loop runs for `auto` (when `[claude] mode = delegated`) and
        // when explicitly chosen; any other engine runs in Loki's own loop below.
        let delegated = match engine.as_str() {
            engines::CLAUDE_CODE => true,
            engines::AUTO => self.claude_cfg.enabled && self.claude_cfg.mode == crate::config::ClaudeMode::Delegated,
            _ => false,
        };

        if allow_routines {
            if message.trim().eq_ignore_ascii_case(ROUTINE_EXIT_COMMAND) {
                let now_local = !force_local;
                let reply = if now_local {
                    "Switched to local-only for this conversation — nothing will be sent to \
                     Claude Code until you type /exit again."
                        .to_string()
                } else {
                    "Switched back to Claude Code as the default for this conversation.".to_string()
                };
                return self.finish_toggle_turn(session_id, history, reply, now_local, &message).await.map(|reply| SentTurn { reply, via: "system".to_string(), note: None });
            }

            // Default-to-Claude routing (2026-09-01 design decision): every turn is eligible
            // for Claude Code unless `force_local` (the `/exit` override above) is set, or
            // the pre-turn classifier flags this specific message as local-only-shaped. The
            // classifier call itself always runs on `self.llm` (the configured local model),
            // never whatever this session might otherwise be talking to — see
            // `content_classifier::should_stay_local`'s doc comment for why that has to be
            // the case for the check to mean anything.
            if !force_local {
                local_reason = if cloud_override {
                    content_classifier::hard_category(&message)
                        .map(|c| format!("it contains a {} (never sent to a cloud model, not even with /cloud)", c.label()))
                } else {
                    content_classifier::assess(self.llm.as_ref(), &message).await
                };
                local_only = local_reason.is_some();
                if let Some(why) = &local_reason {
                    eprintln!("[loki-debug] privacy gate held a message local: {why}");
                    let hint = if cloud_override { "" } else { " To send this to a cloud model anyway, resend it as /cloud <your message>." };
                    emit(sink, ChatEvent::Status { message: format!("Kept local: {why}.{hint}") });
                }
            }
            if local_only && engine == engines::CLAUDE_CODE {
                return Err(SendMessageError::Other(anyhow::anyhow!(
                    "this message was kept local because {}, so it can't go to Claude Code. {}Pick a local model (Ollama) from the engine menu, or switch back to Auto.",
                    local_reason.as_deref().unwrap_or("it looks sensitive"),
                    if cloud_override { "" } else { "Resend it as `/cloud <message>` to override, or " }
                )));
            }
            if !local_only && delegated {
                if let Some(left) = self.claude_cooldown.remaining() {
                    claude_skipped = Some(format!("Claude Code is out of usage right now (retrying in {} min)", left.as_secs() / 60 + 1));
                } else {
                    let (mut claude_session_id, mut is_new_session) = match active_routine_session {
                        Some(id) => (id, false),
                        None => (new_claude_session_uuid(), true),
                    };
                    // Only a streaming turn has a client to prompt or run device tools.
                    let token = if sink.is_some() { crate::auth::load_or_create_token().ok() } else { None };
                    let bridge_session = token.as_deref().map(|t| crate::claude::BridgeSession { session_id, token: t });
                    let mut retried = false;
                    let result = loop {
                        // Handoff: send Claude whatever it hasn't seen — turns other backends
                        // answered, or the whole transcript if its CLI session was lost.
                        // `history` ends with the new user message, which is sent as-is, not in
                        // the digest. Private (stayed-local) turns are replaced by a placeholder.
                        let seen = if is_new_session { 0 } else { claude_synced };
                        let catchup = router::build_catchup(
                            &history,
                            seen,
                            router::convo_len(&history).saturating_sub(1),
                            Some((&summary, summarized_upto)),
                            &private_turns,
                        );
                        let outgoing = match &catchup {
                            Some(block) => format!("{block}{message}"),
                            None => message.clone(),
                        };
                        let r = self
                            .claude
                            .run_routine_turn(&claude_session_id, &outgoing, is_new_session, sink, bridge_session.as_ref())
                            .await;
                        // A persisted session id the CLI no longer knows: start fresh, once.
                        if let Err(e) = &r {
                            if !is_new_session && !retried && e.to_string().contains("No conversation found") {
                                retried = true;
                                claude_session_id = new_claude_session_uuid();
                                is_new_session = true;
                                emit(sink, ChatEvent::Reset);
                                continue;
                            }
                        }
                        break r;
                    };
                    match result {
                        Ok(reply) if !router::reply_is_limit_notice(&reply) => {
                            // Purely a label for the vault conversation log's `routine:`
                            // frontmatter (`SessionState::routine_name`) — only checked on the
                            // turn that actually starts a fresh Claude session, and only
                            // affects that metadata, not routing.
                            let routine_name_if_starting = if is_new_session {
                                let trigger = message.trim().trim_start_matches('/').to_lowercase();
                                self.claude
                                    .list_available_commands()
                                    .ok()
                                    .filter(|commands| commands.contains(&trigger))
                                    .map(|_| trigger)
                            } else {
                                None
                            };
                            return self
                                .finish_routine_turn(session_id, history, reply, Some(claude_session_id), routine_name_if_starting, &message)
                                .await
                                .map(|reply| SentTurn { reply, via: "claude".to_string(), note: None });
                        }
                        Ok(limit_notice) => {
                            emit(sink, ChatEvent::Reset);
                            self.claude_cooldown.trip(claude_cooldown_duration());
                            if engine == engines::CLAUDE_CODE {
                                return Err(SendMessageError::Other(anyhow::anyhow!("Claude Code hit its usage limit ({}). Pick another engine, or switch to Auto to fail over.", limit_notice.trim())));
                            }
                            claude_skipped = Some(format!("Claude Code hit its usage limit ({})", limit_notice.trim()));
                        }
                        Err(e) if router::is_limit_error(&e.to_string()) => {
                            emit(sink, ChatEvent::Reset);
                            self.claude_cooldown.trip(claude_cooldown_for(&e.to_string()));
                            let first_line: String = e.to_string().lines().next().unwrap_or("").chars().take(140).collect();
                            if engine == engines::CLAUDE_CODE {
                                return Err(SendMessageError::Other(anyhow::anyhow!("Claude Code is unavailable ({first_line}). Pick another engine, or switch to Auto to fail over.")));
                            }
                            claude_skipped = Some(format!("Claude Code is unavailable ({first_line})"));
                            eprintln!("[loki-debug] claude failed over: {e}");
                        }
                        Err(e) => return Err(e.into()),
                    }
                    // A new Claude session that failed before answering is simply discarded; the
                    // next attempt starts another with a fresh handoff.
                }
            }
            // Flagged local-only, forced local, or Claude unavailable — fall through to the
            // provider chain below. Leaves `routine_claude_session` untouched: a later clean
            // turn resumes it and gets the missed turns as a handoff block.
        }

        // Provider chain: the configured `llm`, then `[[fallbacks]]` in order. Cloud fallbacks
        // are excluded for local-only turns (the primary keeps its pre-existing behavior; its
        // tool-level guard is in `agent::run_turn`).
        if let Some(why) = &claude_skipped {
            emit(sink, ChatEvent::Status { message: format!("{why} — using {} instead", self.llm.name()) });
        }
        let mut chain: Vec<Arc<dyn LlmProvider>> = Vec::new();
        match engine.as_str() {
            engines::AUTO | engines::CLAUDE_CODE => {
                // `auto` in inference mode: Claude (as a pure model) first, unless it's out of
                // usage or this message must stay local; then the usual chain.
                if engine == engines::AUTO
                    && self.claude_cfg.enabled
                    && self.claude_cfg.mode == crate::config::ClaudeMode::Inference
                    && !local_only
                    && allow_routines
                {
                    match self.claude_cooldown.remaining() {
                        Some(left) => {
                            claude_skipped = Some(format!("Claude is out of usage right now (retrying in {} min)", left.as_secs() / 60 + 1));
                        }
                        None => chain.push(self.claude_inference(self.claude_cfg.inference_model.clone(), session_id)),
                    }
                }
                if self.cloud_first && !local_only && engine == engines::AUTO {
                    // Everything that is not sensitive goes to the cloud provider first; the local model is the backup.
                    chain.extend(self.fallbacks.iter().filter(|p| p.is_cloud()).cloned());
                    chain.push(self.llm.clone());
                    chain.extend(self.fallbacks.iter().filter(|p| !p.is_cloud()).cloned());
                } else {
                    chain.push(self.llm.clone());
                    chain.extend(self.fallbacks.iter().filter(|p| !(local_only && p.is_cloud())).cloned());
                }
            }
            explicit => {
                let provider = self
                    .provider_for_engine(explicit, session_id)
                    .ok_or_else(|| SendMessageError::Other(anyhow::anyhow!("engine '{explicit}' isn't available")))?;
                if local_only && provider.is_cloud() {
                    return Err(SendMessageError::Other(anyhow::anyhow!(
                        "this message was kept local because {}, so it can't go to {}. {}Pick a local model (Ollama) from the engine menu.",
                        local_reason.as_deref().unwrap_or("it looks sensitive"),
                        explicit,
                        if cloud_override { "" } else { "Resend it as `/cloud <message>` to override, or " }
                    )));
                }
                // A cloud engine you picked still falls back to the local model if it stalls or
                // fails, so the turn is answered (and the note says why) instead of dying.
                let fall_back = provider.is_cloud();
                chain.push(provider);
                if fall_back {
                    chain.push(self.llm.clone());
                }
            }
        }

        let ctx = ToolCtx {
            obsidian: &self.obsidian,
            knowledge: self.knowledge.clone(),
            claude: &self.claude,
            bridge: &self.bridge,
            session_id,
            context: &self.local_context,
            skills: &self.skills,
        };
        let summary_ref = (!summary.is_empty()).then_some(summary.as_str());
        // A named routine (/eod, /bod, …) reads a lot before it writes: give it more rounds.
        let is_routine = self.sessions.lock().unwrap().get(session_id).map(|s| s.routine_name.is_some()).unwrap_or(false);
        let max_rounds = if is_routine { crate::agent_limits::routine() } else { crate::agent_limits::ordinary() };
        let user_turn_index = router::convo_len(&history).saturating_sub(1);
        let outcome = router::first_success(&chain, |provider| {
            let history = history.clone();
            let ctx = &ctx;
            let tools = &self.tools;
            let local_context = self.local_context.prompt_block.as_str();
            async move {
                let r = agent::run_turn(provider.as_ref(), tools, ctx, session_id, history, local_context, summary_ref, sink, max_rounds, crate::context_window::ContextPolicy { budget_tokens: crate::context_stats::get().budget_tokens(), summarized_upto }).await;
                if let Err(e) = &r {
                    emit(sink, ChatEvent::Reset);
                    if provider.name() == "claude-inference" && router::is_limit_error(&e.to_string()) {
                        self.claude_cooldown.trip(claude_cooldown_for(&e.to_string()));
                    }
                }
                r
            }
        })
        .await?;
        let (updated_history, reply) = outcome.value;

        {
            let mut sessions = self.sessions.lock().unwrap();
            if let Some(state) = sessions.get_mut(session_id) {
                state.history = updated_history;
                if local_only {
                    // This exchange stayed local for privacy: remember so it is never handed
                    // to Claude or folded into a summary later.
                    state.private_turns.push(user_turn_index);
                    state.private_turns.push(router::convo_len(&state.history).saturating_sub(1));
                }
            }
        }

        self.log_conversation_turn(session_id, &message, &reply);

        let mut notes: Vec<String> = claude_skipped.into_iter().collect();
        if !force_local {
            if let Some(why) = &local_reason {
                notes.push(if cloud_override { format!("kept local because {why}") } else { format!("kept local because {why} (resend as /cloud <message> to send it to a cloud model)") });
            }
        }
        notes.extend(outcome.skipped);
        let note = (!notes.is_empty()).then(|| format!("{} — answered by {}.", notes.join("; "), outcome.provider));
        Ok(SentTurn { reply, via: outcome.provider, note })
    }

    /// Shared tail for a turn relayed to Claude Code, whether starting a fresh session or
    /// continuing one via `--resume`: appends the reply to `history` as an assistant turn,
    /// writes the session's `routine_claude_session` to `new_routine_session` (always
    /// `Some(uuid)` here — `/exit` no longer routes through this function, see
    /// `finish_toggle_turn` for that toggle instead), persists history, and logs the turn —
    /// the same bookkeeping the local-model path does at the end of `send_message_in_session`,
    /// just without going through `agent::run_turn`. `routine_name_if_starting` is
    /// `Some(trigger)` only when the turn both started a fresh Claude session and its message
    /// happens to match a known `~/.claude/commands/` name (see the call site) — a label for
    /// the conversation log's frontmatter, not a routing decision; see
    /// `SessionState::routine_name`'s doc comment.
    async fn finish_routine_turn(
        &self,
        session_id: &str,
        mut history: Vec<Message>,
        reply: String,
        new_routine_session: Option<String>,
        routine_name_if_starting: Option<String>,
        user_message: &str,
    ) -> Result<String, SendMessageError> {
        history.push(Message::assistant(reply.clone()));
        {
            let mut sessions = self.sessions.lock().unwrap();
            if let Some(state) = sessions.get_mut(session_id) {
                state.history = history;
                state.routine_claude_session = new_routine_session;
                // Claude has now seen everything in `history`, including this reply.
                state.claude_synced = router::convo_len(&state.history);
                if let Some(name) = routine_name_if_starting {
                    state.routine_name = Some(name);
                }
            }
        }
        self.log_conversation_turn(session_id, user_message, &reply);
        Ok(reply)
    }

    /// Handles the `/exit` toggle: flips `force_local` for the session and appends a reply
    /// confirming the new state. Deliberately does not touch `routine_claude_session` either
    /// way — toggling back to cloud later resumes the same Claude session right where it left
    /// off (via `--resume`) rather than starting a new one, same as a classifier-flagged turn
    /// leaves it alone (see `send_message_in_session_with_images`).
    async fn finish_toggle_turn(
        &self,
        session_id: &str,
        mut history: Vec<Message>,
        reply: String,
        now_local: bool,
        user_message: &str,
    ) -> Result<String, SendMessageError> {
        history.push(Message::assistant(reply.clone()));
        {
            let mut sessions = self.sessions.lock().unwrap();
            if let Some(state) = sessions.get_mut(session_id) {
                state.history = history;
                state.force_local = now_local;
            }
        }
        self.log_conversation_turn(session_id, user_message, &reply);
        Ok(reply)
    }

    /// Write-through conversation logging — appends the completed exchange to the session's
    /// vault note (`98 - Loki Conversations/*.md`, creating it with frontmatter on the first
    /// call for a session) and re-ingests it into the knowledge store. Fire-and-forget: see
    /// `conversation_log::log_conversation_turn`'s doc comment for why, and why both turns go
    /// in one call rather than two.
    fn log_conversation_turn(&self, session_id: &str, user_message: &str, reply: &str) {
        let (path, is_first_write, routine) = {
            let mut sessions = self.sessions.lock().unwrap();
            let Some(state) = sessions.get_mut(session_id) else { return };
            match &state.conversation_log_path {
                Some(p) => (p.clone(), false, None),
                None => {
                    let p = conversation_log::path_for_session(session_id);
                    state.conversation_log_path = Some(p.clone());
                    (p, true, state.routine_name.clone())
                }
            }
        };
        persist_state(&self.sessions, session_id);
        self.offload_exchange(session_id);
        self.maybe_summarize(session_id);
        self.maybe_auto_title(session_id);
        conversation_log::log_conversation_turn(
            self.obsidian.clone(),
            self.knowledge.clone(),
            session_id.to_string(),
            path,
            is_first_write,
            routine,
            conversation_log::strip_preamble(user_message).to_string(),
            reply.to_string(),
        );
    }

    /// Conversations for the session picker: those with persisted state (title, last activity),
    /// newest first, then older vault-only logs from before session state was persisted
    /// (listed by date; opening one loads its transcript and starts persisting it).
    /// Gives a conversation a custom title. Keeps its last-activity time (renaming shouldn't
    /// reorder the list). Older vault-only logs have no state file and can't be renamed.
    pub fn rename_session(&self, session_id: &str, title: &str) -> Result<String, String> {
        let title: String = title.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(120).collect();
        if title.is_empty() {
            return Err("title can't be empty".into());
        }
        let in_memory = {
            let mut guard = self.sessions.lock().unwrap();
            match guard.get_mut(session_id) {
                Some(state) => {
                    state.title = Some(title.clone());
                    state.title_manual = true;
                    true
                }
                None => false,
            }
        };
        match session_store::load(session_id) {
            Some(mut p) => {
                p.title = Some(title.clone());
                p.title_manual = true;
                session_store::save(session_id, &p);
            }
            None if in_memory => persist_state(&self.sessions, session_id),
            None => return Err("no saved conversation with that id".into()),
        }
        Ok(title)
    }

    /// Thumbs up/down on an assistant message. `rating`: "up" | "down" | "none".
    pub fn rate_turn(&self, session_id: &str, index: usize, rating: &str) -> Result<(), String> {
        if !matches!(rating, "up" | "down" | "none") {
            return Err("rating must be up, down or none".into());
        }
        {
            let mut guard = self.sessions.lock().unwrap();
            let state = guard.get_mut(session_id).ok_or("unknown conversation")?;
            let is_assistant = state.history.iter().filter(|m| router::is_turn(m)).nth(index).map(|m| m.role == "assistant").unwrap_or(false);
            if !is_assistant {
                return Err("only assistant messages can be rated".into());
            }
            if rating == "none" {
                state.feedback.remove(&index);
            } else {
                state.feedback.insert(index, rating.to_string());
            }
        }
        persist_state(&self.sessions, session_id);
        Ok(())
    }

    /// Whether a downvote should kick off an audit on its own (setting `downvotes`/`all_local`).
    pub fn auto_audit_on_downvote(&self) -> bool {
        crate::lessons::settings().auto != "off"
    }

    pub fn auto_audit_all_local(&self) -> bool {
        crate::lessons::settings().auto == "all_local"
    }

    /// Claude reviews one assistant message. Refuses privacy-flagged turns.
    pub async fn audit_turn(&self, session_id: &str, index: usize, auto: bool) -> Result<crate::audit::AuditRecord, String> {
        self.ensure_session_seeded(session_id).await;
        let (question, answer, ordinal, engine_note) = {
            let guard = self.sessions.lock().unwrap();
            let state = guard.get(session_id).ok_or("unknown conversation")?;
            let convo: Vec<&Message> = state.history.iter().filter(|m| router::is_turn(m)).collect();
            let answer = convo.get(index).filter(|m| m.role == "assistant").ok_or("that isn't an assistant message")?;
            let uidx = (0..index).rev().find(|i| convo[*i].role == "user").ok_or("no user message before that reply")?;
            if state.private_turns.contains(&index) || state.private_turns.contains(&uidx) {
                return Err("that turn was kept local for privacy and can't be sent to Claude".into());
            }
            let ordinal = convo[..=uidx].iter().filter(|m| m.role == "user").count();
            let q = convo[uidx].display.clone().unwrap_or_else(|| convo[uidx].content.clone());
            let note = match state.engine.as_deref() {
                Some(e) => format!("(Conversation engine setting: {e}.)\n"),
                None => String::new(),
            };
            (q, answer.content.clone(), ordinal, note)
        };
        let events = crate::event_log::turn_events(session_id, ordinal);
        let process = crate::event_log::render(&events);
        let process = if process.chars().count() > 12000 { process.chars().take(12000).collect::<String>() + "\n… [truncated]" } else { process };
        let lessons: String = crate::lessons::list().iter().filter(|l| l.status == "approved" && l.kind != "routine_edit").map(|l| format!("- {}", l.text)).collect::<Vec<_>>().join("\n");
        let prompt = crate::audit::build_prompt(
            conversation_log::strip_preamble(&question),
            &answer,
            if process.is_empty() { "(no process record for this turn)" } else { &process },
            &engine_note,
            if lessons.is_empty() { "(none)" } else { &lessons },
        );
        let args: Vec<String> = crate::audit::CLAUDE_ARGS.iter().map(|s| s.to_string()).collect();
        let text = self.claude.ask_with(&prompt, &args, None).await.map_err(|e| format!("Claude audit failed: {e}"))?;
        let rec = crate::audit::parse(session_id, index, auto, &text)?;
        crate::audit::save(&rec);
        crate::lessons::add_pending(&rec.suggested_lessons, &format!("{session_id}#{index}"));
        Ok(rec)
    }

    /// Builds the troubleshoot evidence bundle and returns its directory.
    pub async fn build_troubleshoot_bundle(&self, session_id: &str, ui: &crate::troubleshoot::UiSnapshot) -> anyhow::Result<std::path::PathBuf> {
        let dir = crate::troubleshoot::bundle_dir()?;
        let turns = self.get_history_in_session(session_id).await;
        let private: Vec<usize> = self.sessions.lock().unwrap().get(session_id).map(|s| s.private_turns.clone()).unwrap_or_default();
        let mut md = format!("# Conversation {session_id}\n\n");
        for t in &turns {
            let who = if t.role == "user" { "User" } else { "Loki" };
            let body = if private.contains(&t.index) { "[withheld: kept local for privacy]".to_string() } else { t.content.clone() };
            md.push_str(&format!("## {who} — {}\n", t.at.as_deref().unwrap_or("time unknown")));
            if let Some(f) = &t.feedback {
                md.push_str(&format!("(rated: thumbs {f})\n"));
            }
            if let Some(a) = &t.audit {
                md.push_str(&format!("(Claude audit: {} — {})\n", a.verdict, a.summary));
            }
            md.push_str(&format!("\n{body}\n\n"));
        }
        std::fs::write(dir.join("transcript.md"), md)?;
        let events = crate::event_log::read(session_id);
        let skip: Vec<usize> = {
            // user-turn ordinals of private turns, so their process records are withheld too
            let guard = self.sessions.lock().unwrap();
            match guard.get(session_id) {
                Some(st) => {
                    let convo: Vec<&Message> = st.history.iter().filter(|m| router::is_turn(m)).collect();
                    private.iter().filter_map(|i| (0..=*i.min(&convo.len().saturating_sub(1))).rev().find(|j| convo.get(*j).map(|m| m.role == "user").unwrap_or(false))).map(|u| convo[..=u].iter().filter(|m| m.role == "user").count()).collect()
                }
                None => Vec::new(),
            }
        };
        let mut n = 0;
        let kept: Vec<serde_json::Value> = events
            .into_iter()
            .filter(|e| {
                if e["ev"]["type"] == "user_message" {
                    n += 1;
                }
                !skip.contains(&n)
            })
            .collect();
        let tail: Vec<serde_json::Value> = kept.iter().rev().take(300).rev().cloned().collect();
        std::fs::write(dir.join("events.txt"), crate::event_log::render(&tail))?;
        if private.is_empty() {
            if let Ok(out) = std::process::Command::new("journalctl").args(["-u", "loki-server", "-n", "250", "--no-pager", "-o", "short-iso"]).output() {
                let log: String = String::from_utf8_lossy(&out.stdout).lines().map(|l| l.chars().take(600).collect::<String>()).collect::<Vec<_>>().join("\n");
                std::fs::write(dir.join("server_log.txt"), log)?;
            }
        }
        crate::troubleshoot::write_ui(&dir, ui);
        Ok(dir)
    }

    pub async fn run_troubleshoot(&self, dir: &std::path::PathBuf, note: &str) -> Result<String, String> {
        let repo = std::env::current_dir().map(|p| p.display().to_string()).unwrap_or_else(|_| ".".into());
        let prompt = crate::troubleshoot::prompt(note, dir, &repo, dir.join("ui.png").exists());
        let args = crate::troubleshoot::claude_args(dir, &repo);
        self.claude.ask_with(&prompt, &args, dir.to_str()).await.map_err(|e| format!("Claude troubleshooting failed: {e}"))
    }

    pub async fn list_sessions(&self) -> Vec<session_store::SessionSummary> {
        let mut out = session_store::list();
        let known: std::collections::HashSet<String> = out.iter().map(|s| s.session_id.clone()).collect();
        if let Ok(mut files) = self.obsidian.list_files(Some(&conversation_log::conversation_log_dir())).await {
            files.sort();
            files.reverse();
            for f in files {
                // File names are `YYYY-MM-DD - <session_id>.md`.
                let Some(stem) = f.strip_suffix(".md") else { continue };
                let Some((date, id)) = stem.split_once(" - ") else { continue };
                if known.contains(id) {
                    continue;
                }
                out.push(session_store::SessionSummary {
                    session_id: id.to_string(),
                    title: format!("{date} · earlier conversation"),
                    updated: 0,
                    turns: 0,
                });
            }
        }
        out.truncate(200);
        out
    }

    /// Folds old turns into the session's rolling summary in the background once enough have
    /// piled up past the last summary. Uses the first *local* model available — a summary can
    /// contain anything said in the conversation — and skips silently if every configured
    /// model is cloud. Private turns are left out of the input.
    fn maybe_summarize(&self, session_id: &str) {
        let Some(provider) = std::iter::once(&self.llm).chain(self.fallbacks.iter()).find(|p| !p.is_cloud()).cloned() else {
            return;
        };
        let (existing, turns, target, private) = {
            let mut sessions = self.sessions.lock().unwrap();
            let Some(state) = sessions.get_mut(session_id) else { return };
            let total = router::convo_len(&state.history);
            if state.summarizing {
                return;
            }
            let settings = crate::context_stats::get();
            if !settings.autocompact {
                return;
            }
            let turn_tokens: Vec<usize> = state.history.iter().filter(|m| router::is_turn(m)).map(crate::context_window::msg_tokens).collect();
            let Some(target) = crate::context_window::compaction_target(&turn_tokens, state.summarized_upto, settings.budget_tokens(), settings.max_turns, 4) else {
                return;
            };
            let _ = total;
            let turns: Vec<(usize, Message)> = state
                .history
                .iter()
                .filter(|m| router::is_turn(m))
                .cloned()
                .enumerate()
                .skip(state.summarized_upto)
                .take(target - state.summarized_upto)
                .collect();
            state.summarizing = true;
            (state.summary.clone(), turns, target, state.private_turns.clone())
        };
        let sessions = self.sessions.clone();
        let session_id = session_id.to_string();
        tokio::spawn(async move {
            let result = session_store::summarize(&provider, &existing, &turns, &private).await;
            {
                let mut guard = sessions.lock().unwrap();
                if let Some(state) = guard.get_mut(&session_id) {
                    state.summarizing = false;
                    match &result {
                        Ok(text) => {
                            state.summary = text.clone();
                            state.summarized_upto = target;
                        }
                        Err(e) => eprintln!("[loki-debug] summarizing '{session_id}' failed (will retry later): {e}"),
                    }
                }
            }
            if result.is_ok() {
                persist_state(&sessions, &session_id);
            }
        });
    }

    /// Files the conversation's newest exchange (question + answer) in the knowledge store as its
    /// own item — numbered, timestamped and auto-tagged — so it can be recalled precisely after it
    /// falls out of the context window. Idempotent (`session#turn`); privacy-flagged turns are
    /// never filed. Runs in the background.
    fn offload_exchange(&self, session_id: &str) {
        let doc = {
            let sessions = self.sessions.lock().unwrap();
            let Some(state) = sessions.get(session_id) else { return };
            let convo: Vec<&Message> = state.history.iter().filter(|m| router::is_turn(m)).collect();
            Self::exchange_doc(session_id, &convo, convo.len().saturating_sub(2), &state.private_turns)
        };
        if let Some(doc) = doc {
            let knowledge = self.knowledge.clone();
            tokio::spawn(async move {
                if let Err(e) = knowledge.ingest(doc).await {
                    eprintln!("[loki-debug] filing exchange for recall failed (non-fatal): {e}");
                }
            });
        }
    }

    /// Builds the knowledge item for the exchange whose user turn is at convo index `user_idx`.
    fn exchange_doc(session_id: &str, convo: &[&Message], user_idx: usize, private: &[usize]) -> Option<IngestDoc> {
        let user = convo.get(user_idx).filter(|m| m.role == "user")?;
        let answer = convo.get(user_idx + 1).filter(|m| m.role == "assistant")?;
        if private.contains(&user_idx) || private.contains(&(user_idx + 1)) {
            return None;
        }
        let k = convo[..=user_idx].iter().filter(|m| m.role == "user").count();
        let asked = conversation_log::strip_preamble(user.display.as_deref().unwrap_or(&user.content)).trim().to_string();
        let first_line: String = asked.lines().next().unwrap_or("").chars().take(70).collect();
        Some(IngestDoc {
            source_type: "chat_exchange".to_string(),
            source_id: format!("{session_id}#{k}"),
            title: Some(format!("Turn {k} · {} · {first_line}", user.at.as_deref().unwrap_or("time unknown"))),
            content: format!("User: {}\n\nLoki: {}", asked.chars().take(2000).collect::<String>(), answer.content.chars().take(6000).collect::<String>()),
            tags: Vec::new(), // auto-tagged by the store
            session_id: Some(session_id.to_string()),
        })
    }

    /// The "Needs you" list (see `needs_you.rs`).
    pub async fn needs_you(&self) -> serde_json::Value {
        let mut v = crate::needs_you::build(&self.obsidian).await;
        self.attach_blurbs(&mut v);
        // Cards that need this server's own state or tools; hidden on a Depleted day like the other briefings.
        if v["state"]["mode"] != "low" {
            let (workflows, zotero, weather) = tokio::join!(self.n8n_briefing(), self.zotero_briefing(), crate::needs_you::fetch_weather());
            let weather = weather.and_then(|w| crate::needs_you::weather_briefing(&w, &crate::profile::get().location.name));
            let status = self.loki_status_briefing().await;
            let mail = self.mail_briefing().await;
            if let Some(list) = v["briefings"].as_array_mut() {
                if let Some(m) = mail.and_then(|b| serde_json::to_value(b).ok()) {
                    let at = list.iter().position(|b| b["kind"] == "leads").map(|i| i + 1).unwrap_or(list.len());
                    list.insert(at, m);
                }
                // order: weather after the week, workflows after leads, zotero after finance, Loki last
                let place = |list: &Vec<serde_json::Value>, kinds: &[&str]| list.iter().position(|b| kinds.contains(&b["kind"].as_str().unwrap_or(""))).map(|i| i + 1);
                if let Some(w) = weather.and_then(|b| serde_json::to_value(b).ok()) {
                    let at = place(list, &["week"]).unwrap_or(list.len());
                    list.insert(at, w);
                }
                if let Some(x) = workflows.and_then(|b| serde_json::to_value(b).ok()) {
                    let at = place(list, &["leads"]).unwrap_or(list.len());
                    list.insert(at, x);
                }
                if let Some(z) = zotero.and_then(|b| serde_json::to_value(b).ok()) {
                    let at = place(list, &["finance"]).or_else(|| place(list, &["workflows"])).unwrap_or(list.len());
                    list.insert(at, z);
                }
                list.push(status);
            }
        }
        if let Some(start) = crate::needs_you::start_here_briefing(&crate::profile::get()).and_then(|b| serde_json::to_value(b).ok()) {
            if let Some(list) = v["briefings"].as_array_mut() {
                list.insert(0, start);
            }
        }
        v
    }

    /// n8n through Loki's own MCP connection: workflows, their schedules, and the recent runs.
    async fn n8n_briefing(&self) -> Option<crate::needs_you::Briefing> {
        use crate::needs_you as ny;
        let call = |name: &str, args: serde_json::Value| {
            let name = name.to_string();
            async move { tokio::time::timeout(std::time::Duration::from_secs(8), self.mcp.call(&name, &args)).await.ok()?.ok().map(|t| ny::unwrap_result(&t)) }
        };
        let workflows = ny::parse_n8n_workflows(&call("mcp__n8n__list_workflows", serde_json::json!({})).await?);
        let execs = ny::parse_n8n_executions(&call("mcp__n8n__list_executions", serde_json::json!({ "limit": 100 })).await.unwrap_or_default());
        let mut schedules = std::collections::HashMap::new();
        for w in workflows.iter().filter(|w| w.active) {
            if let Some(sc) = call("mcp__n8n__get_workflow", serde_json::json!({ "workflow_id": w.id })).await.and_then(|t| ny::parse_schedule(&t)) {
                schedules.insert(w.id.clone(), sc);
            }
        }
        ny::n8n_briefing(&workflows, &execs, &schedules, ny::now_unix())
    }

    /// Recently added Zotero items, through Loki's own MCP connection.
    async fn zotero_briefing(&self) -> Option<crate::needs_you::Briefing> {
        use crate::needs_you as ny;
        let text = tokio::time::timeout(std::time::Duration::from_secs(8), self.mcp.call("mcp__zotero__zotero_get_recent", &serde_json::json!({ "limit": 12 }))).await.ok()?.ok()?;
        ny::zotero_briefing(&ny::parse_zotero_recent(&ny::unwrap_result(&text)), ny::now_unix())
    }

    /// Starts the local-only inbox review in the background (the local model reads a sample of each mailbox's unread mail).
    pub fn start_mail_review(&self) -> Result<(), String> {
        if crate::mail::load_config().is_none() {
            return Err("mail isn't set up".into());
        }
        if self.mail_review.lock().unwrap().status == "running" {
            return Err("a review is already running".into());
        }
        let provider = std::iter::once(&self.llm).chain(self.fallbacks.iter()).find(|p| !p.is_cloud()).cloned().ok_or("the review needs a local model, and none is configured")?;
        let shared = self.mail_review.clone();
        {
            let mut r = shared.lock().unwrap();
            r.status = "running".into();
            r.phase = "Starting".into();
            r.done = 0;
            r.total = 0;
            r.error = None;
        }
        tokio::spawn(crate::mail_review::run(provider, shared));
        Ok(())
    }

    /// Progress and per-class estimates only — no senders or subjects.
    pub fn mail_review_status(&self) -> serde_json::Value {
        crate::mail_review::status_json(&self.mail_review.lock().unwrap())
    }

    /// The full review as a page (senders and subjects: for the user's own screens only).
    pub fn mail_review_page(&self) -> crate::needs_you::Briefing {
        crate::needs_you::mail_review_briefing(&self.mail_review.lock().unwrap(), crate::needs_you::now_unix() * 1000)
    }

    /// Self-improvement loop. Gathers evidence (failed tool calls, step-limit hits, the findings files'
    /// "Rule for next time" lines), queues *pending* proposals for the user's approval, and checks what
    /// approved tool hints did a week on. Nothing is applied here.
    pub async fn improve_run(&self) -> String {
        const DAYS: u64 = 7;
        let d = crate::improve::digest(DAYS);
        let mut out = crate::improve::render(&d, DAYS);

        // Effect tracking: approved tool hints a week old get a before/after failure count.
        let now = crate::needs_you::now_unix().max(0) as u64;
        for l in crate::lessons::list() {
            if l.status == "approved" && l.effect.is_none() && l.approved_at.is_some_and(|t| now.saturating_sub(t) >= DAYS * 86_400) {
                if let (Some(tool), Some(before)) = (&l.tool, l.baseline) {
                    let after = d.by_tool.get(tool).copied().unwrap_or(0);
                    let verdict = if after < before { "improved" } else { "no improvement" };
                    crate::lessons::set_effect(&l.id, &format!("{verdict}: {before} → {after} failures of {tool} in 7 days"));
                }
            }
        }

        let mut queued = 0;
        // 1. Findings rules → routine edits (deterministic; no model needed).
        let mut files = Vec::new();
        let agent = crate::profile::get().vault.agent.clone();
        if let Ok(names) = self.obsidian.list_files(Some(&format!("{agent}/findings"))).await {
            for n in names.iter().filter(|n| n.ends_with("-findings.md")) {
                let path = format!("{agent}/findings/{}", n.trim_start_matches('/'));
                if let Ok(text) = self.obsidian.read_file(&path).await {
                    files.push((n.trim_end_matches("-findings.md").to_string(), text));
                }
            }
        }
        let date = crate::conversation_log::date_format("%Y-%m-%d").unwrap_or_default();
        let mut props = Vec::new();
        for (wf, rule) in crate::improve::findings_rules(&files) {
            let target = format!("{agent}/{wf}.md");
            let Ok(current) = self.obsidian.read_file(&target).await else { continue };
            if crate::improve::already_present(&current, &rule) {
                continue;
            }
            props.push(crate::lessons::Proposal {
                text: format!("Add to {wf}.md: {rule}"),
                kind: "routine_edit".into(),
                target: Some(target),
                append: Some(format!("\n- Learned rule ({date}): {rule}\n")),
                ..Default::default()
            });
        }
        queued += crate::lessons::add_proposals(&props, &format!("findings:{date}"));

        // 2. Failure digest → lessons / tool hints, drafted by the local model.
        if !d.failures.is_empty() || d.step_limit_hits > 0 {
            match std::iter::once(&self.llm).chain(self.fallbacks.iter()).find(|p| !p.is_cloud()).cloned() {
                None => out.push_str("\nNo local model is available to draft lessons."),
                Some(provider) => {
                    let (system, user) = crate::improve::prompt(&d, DAYS);
                    let msgs = vec![Message::system(system), Message::user(user)];
                    match tokio::time::timeout(std::time::Duration::from_secs(150), provider.chat_low_effort(&msgs)).await {
                        Ok(Ok(r)) => {
                            let props: Vec<_> = crate::improve::parse_lessons(&r.content).into_iter().map(|t| crate::improve::as_proposal(t, &d)).collect();
                            queued += crate::lessons::add_proposals(&props, &format!("improve:{date}"));
                        }
                        _ => out.push_str("\nThe local model didn't answer in time; findings rules were still checked."),
                    }
                }
            }
        } else {
            out.push_str("\nNo failures to learn from.");
        }
        format!("{out}\nQueued {queued} pending proposal(s) on the Improve tab for your approval.")
    }

    /// Approving a routine edit writes it to the vault. Returns an error (leaving it pending) if it can't.
    pub async fn apply_routine_edit(&self, id: &str) -> Result<(), String> {
        let l = crate::lessons::get(id).ok_or("no such proposal")?;
        if l.kind != "routine_edit" {
            return Ok(());
        }
        let (Some(target), Some(append)) = (l.target, l.append) else { return Err("proposal has no target".into()) };
        let agent = &crate::profile::get().vault.agent;
        if !target.starts_with(&format!("{agent}/")) || !target.ends_with(".md") {
            return Err(format!("routine edits may only touch {agent}/*.md"));
        }
        let current = self.obsidian.read_file(&target).await.map_err(|e| e.to_string())?;
        if current.contains(append.trim()) {
            return Ok(());
        }
        self.obsidian.append_content(&target, &append).await.map_err(|e| e.to_string())
    }

    /// Extract: the local model proposes action items and events from a message or any text. Nothing is written here.
    pub async fn extract_run(&self, source: &str, account: &str, id: &str, text: &str, label: &str) -> Result<serde_json::Value, String> {
        let provider = self.local_llm().ok_or("no local model is available")?;
        let (body, key, label, link) = if source == "mail" {
            let body = crate::mail::tool_read(&serde_json::json!({ "account": account, "id": id })).await?;
            (body, format!("mail:{account}:{id}"), "an email".to_string(), format!("https://mail.google.com/mail/?authuser={}#all/{id}", urlencoding::encode(account)))
        } else {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            text.hash(&mut h);
            let label = if label.trim().is_empty() { "Loki".to_string() } else { label.chars().take(60).collect() };
            (text.to_string(), format!("text:{:x}", h.finish()), label, String::new())
        };
        if body.trim().is_empty() {
            return Err("there is no text to read".into());
        }
        let today = conversation_log::date_format("%Y-%m-%d").unwrap_or_default();
        let weekday = conversation_log::date_format("%A").unwrap_or_default();
        let (system, user) = crate::extract::prompt(&body, &today, &weekday);
        let msgs = vec![Message::system(system), Message::user(user)];
        let reply = tokio::time::timeout(std::time::Duration::from_secs(150), provider.chat_low_effort(&msgs)).await.map_err(|_| "the local model took too long".to_string())?.map_err(|e| e.to_string())?.content;
        Ok(serde_json::json!({ "key": key, "seen": crate::extract::seen(&key), "label": label, "link": link, "proposals": crate::extract::parse(&reply) }))
    }

    /// Writes what the user approved: dated items into Upcoming Events (inserted in date order, every existing line kept),
    /// undated actions as Inbox notes. Only the approved fields are written.
    pub async fn extract_apply(&self, key: &str, label: &str, link: &str, proposals: Vec<crate::extract::Proposal>) -> Result<Vec<String>, String> {
        use crate::extract as ex;
        let today = conversation_log::date_format("%Y-%m-%d").unwrap_or_default();
        let label: String = if label.trim().is_empty() { "Loki".into() } else { label.chars().take(60).collect() };
        let props: Vec<ex::Proposal> = proposals.into_iter().filter_map(ex::sanitize).take(12).collect();
        if props.is_empty() {
            return Err("nothing valid to add".into());
        }
        let mut results = Vec::new();
        let dated: Vec<&ex::Proposal> = props.iter().filter(|p| ex::is_dated(p)).collect();
        if !dated.is_empty() {
            let old = self.obsidian.read_file(&ex::events_path()).await.map_err(|e| e.to_string())?;
            let mut text = old.clone();
            for p in dated {
                let marker = if p.kind == "todo" { format!("**Due: {}**", p.title) } else { format!("**{}**", p.title) };
                if text.contains(&marker) {
                    results.push(format!("Already in Upcoming Events: {}", p.title));
                    continue;
                }
                let day = conversation_log::date_format_for(&p.date, "%A").unwrap_or_default();
                text = ex::insert_event(&text, &p.date, &day, &ex::event_lines(p, &label, &today));
                results.push(format!("Added to Upcoming Events on {}: {}", p.date, p.title));
            }
            if text != old {
                if !ex::preserves(&old, &text) {
                    return Err("stopped: the edit would have changed existing lines in Upcoming Events. Nothing was written.".into());
                }
                self.obsidian.put_content(&ex::events_path(), &text).await.map_err(|e| e.to_string())?;
            }
        }
        for p in props.iter().filter(|p| !ex::is_dated(p)) {
            let base = ex::safe_filename(&p.title);
            if base.is_empty() {
                continue;
            }
            let mut name = base.clone();
            let mut n = 2;
            let inbox = &crate::profile::get().vault.inbox;
            while self.obsidian.read_file(&format!("{inbox}/{name}.md")).await.is_ok() && n < 20 {
                name = format!("{base} {n}");
                n += 1;
            }
            self.obsidian.append_content(&format!("{inbox}/{name}.md"), &ex::inbox_note(p, &label, link, &today)).await.map_err(|e| e.to_string())?;
            results.push(if p.kind == "todo" { format!("To-do added to the Inbox: {name}") } else { format!("{} added to the Inbox: {name}", if p.kind == "reference" { "Reference note" } else { "Knowledge note" }) });
        }
        if results.iter().any(|r| r.starts_with("Added") || r.contains("added to the Inbox")) {
            ex::mark_seen(key);
        }
        Ok(results)
    }

    /// Ticks, reopens or comments on one task in a daily or sprint note. Only that line changes.
    pub async fn mark_task(&self, path: &str, text: &str, section: &str, done: Option<bool>, comment: &str) -> Result<String, String> {
        if !crate::note_tasks::allowed_path(path) {
            return Err("Loki only ticks tasks in daily and sprint notes".into());
        }
        let note = self.obsidian.read_file(path).await.map_err(|e| e.to_string())?;
        let stamp = conversation_log::date_format("%m/%d").unwrap_or_default();
        let (new, line) = crate::note_tasks::mark(&note, text, section, done, comment, &stamp)?;
        if new == note {
            return Ok("Nothing to change.".into());
        }
        self.obsidian.put_content(path, &new).await.map_err(|e| e.to_string())?;
        Ok(format!("Updated the note: {}", line.trim().chars().take(120).collect::<String>()))
    }

    /// The full text of one backlog item (`30 - Backlog/<Area>/<Title>.md`).
    pub async fn backlog_item(&self, path: &str) -> Result<String, String> {
        if !crate::backlog_edit::allowed_path(path) {
            return Err("Loki only opens backlog items here".into());
        }
        self.obsidian.read_file(path).await.map_err(|e| e.to_string())
    }

    /// Edits one backlog item. `action`: `save` (whole note; refused if it changed since `base`), `comment`,
    /// `status` (a free label that stays in the backlog), or `close` (`status` done|superseded|dropped: filed in 99 - Archive).
    pub async fn backlog_edit(&self, path: &str, action: &str, content: &str, base: &str, comment: &str, status: &str) -> Result<String, String> {
        use crate::backlog_edit as be;
        if !be::allowed_path(path) {
            return Err("Loki only edits backlog items here".into());
        }
        let cur = self.obsidian.read_file(path).await.map_err(|e| e.to_string())?;
        let c = crate::needs_you::clock();
        let stamp = c.date.get(5..).unwrap_or("").replace('-', "/");
        let msg = match action {
            "save" => {
                if cur != base {
                    return Err("This note changed in Obsidian since you opened it. Close and reopen it to see the latest, then edit again.".into());
                }
                be::rewrite_ok(&cur, content)?;
                self.obsidian.put_content(path, content).await.map_err(|e| e.to_string())?;
                "Saved to the note."
            }
            "comment" => {
                let new = be::add_comment(&cur, &stamp, comment)?;
                self.obsidian.put_content(path, &new).await.map_err(|e| e.to_string())?;
                "Comment added to the note."
            }
            "status" => {
                let label = status.trim();
                if label.chars().count() > 40 || label.contains('\n') {
                    return Err("Keep the status to a short label.".into());
                }
                let new = be::set_field(&cur, "status", label);
                self.obsidian.put_content(path, &new).await.map_err(|e| e.to_string())?;
                "Status set."
            }
            "close" => {
                let target = crate::profile::get().archive_target(path).ok_or("bad path")?;
                if self.obsidian.read_file(&target).await.is_ok() {
                    return Err(format!("{target} already exists. Rename one of them in Obsidian first."));
                }
                let new = be::archived(&cur, status, &c.date, &c.iso_week, comment)?;
                self.obsidian.put_content(&target, &new).await.map_err(|e| e.to_string())?;
                // the original goes only once the archive copy reads back
                if self.obsidian.read_file(&target).await.map_err(|e| e.to_string())?.trim().is_empty() {
                    return Err("The archive copy didn't save, so nothing was removed.".into());
                }
                self.obsidian.delete_file(path).await.map_err(|e| e.to_string())?;
                "Closed and filed in the archive."
            }
            _ => return Err("unknown action".into()),
        };
        crate::needs_you::forget_backlog();
        Ok(msg.into())
    }

    /// Vault notes whose names or text match `q` (for choosing where a note should be added).
    pub async fn notes_find(&self, q: &str) -> Result<Vec<String>, String> {
        if q.trim().len() < 2 {
            return Ok(Vec::new());
        }
        let hits = self.obsidian.simple_search(q.trim(), 40).await.map_err(|e| e.to_string())?;
        let mut out: Vec<String> = Vec::new();
        for (p, _) in hits {
            if crate::notes_ai::allowed_path(&p) && !out.contains(&p) {
                out.push(p);
            }
            if out.len() >= 12 {
                break;
            }
        }
        Ok(out)
    }

    /// The local model proposes how to file a note: as a new note, additions to existing notes, or its choice. Writes nothing.
    pub async fn notes_plan(&self, title: &str, text: &str, mode: &str, targets: &[String]) -> Result<serde_json::Value, String> {
        let provider = self.local_llm().ok_or("no local model is available")?;
        if text.trim().is_empty() {
            return Err("there is no text to file".into());
        }
        let mode = if matches!(mode, "new" | "update") { mode } else { "auto" };
        let mut cands: Vec<(String, String)> = Vec::new();
        if mode != "new" {
            let mut paths: Vec<String> = targets.iter().filter(|p| crate::notes_ai::allowed_path(p)).cloned().collect();
            if paths.is_empty() {
                let q = if title.trim().is_empty() { text.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim_start_matches('#').trim().to_string() } else { title.to_string() };
                paths = self.notes_find(&q).await.unwrap_or_default().into_iter().take(4).collect();
            }
            for p in paths.into_iter().take(5) {
                if let Ok(t) = self.obsidian.read_file(&p).await {
                    cands.push((p, t.chars().take(1800).collect()));
                }
            }
            if mode == "update" && cands.is_empty() {
                return Err("no existing notes to add to. Pick one, or choose New note.".into());
            }
        }
        let today = conversation_log::date_format("%Y-%m-%d").unwrap_or_default();
        let (system, user) = crate::notes_ai::prompt(title, text, mode, &cands, &today);
        let msgs = vec![Message::system(system), Message::user(user)];
        let reply = tokio::time::timeout(std::time::Duration::from_secs(240), provider.chat_low_effort(&msgs)).await.map_err(|_| "the local model took too long".to_string())?.map_err(|e| e.to_string())?.content;
        let mut props = crate::notes_ai::parse(&reply);
        // appends only go to notes that exist; creates only to paths that don't; "new"/"update" modes keep to their kind
        let mut out = Vec::new();
        for p in props.drain(..) {
            let exists = self.obsidian.read_file(&p.path).await.is_ok();
            let keep = match (mode, p.op.as_str()) {
                ("new", "create") | ("update", "append") | ("auto", _) => true,
                _ => false,
            };
            if keep && ((p.op == "append") == exists) {
                out.push(serde_json::json!({ "op": p.op, "path": p.path, "content": p.content, "heading": p.heading, "why": p.why }));
            }
        }
        if out.is_empty() {
            return Err("the model didn't return a usable plan. You can try again, or save the note as it is.".into());
        }
        Ok(serde_json::json!({ "proposals": out, "looked_at": cands.iter().map(|c| c.0.clone()).collect::<Vec<_>>() }))
    }

    /// Writes what the user approved. New notes never overwrite; additions only add to the end of a note.
    pub async fn notes_apply(&self, proposals: Vec<crate::notes_ai::NoteProposal>) -> Result<Vec<String>, String> {
        use crate::notes_ai as na;
        let props: Vec<na::NoteProposal> = proposals.into_iter().filter_map(na::sanitize).take(6).collect();
        if props.is_empty() {
            return Err("nothing valid to write".into());
        }
        let mut results = Vec::new();
        for p in props {
            if p.op == "create" {
                if self.obsidian.read_file(&p.path).await.is_ok() {
                    results.push(format!("Skipped, already exists: {}", p.path));
                    continue;
                }
                self.obsidian.put_content(&p.path, &na::with_frontmatter(&p.content)).await.map_err(|e| e.to_string())?;
                results.push(format!("Created {}", p.path));
            } else {
                let old = self.obsidian.read_file(&p.path).await.map_err(|_| format!("{} doesn't exist any more", p.path))?;
                let new = na::appended(&old, &p.content, &p.heading);
                if !crate::extract::preserves(&old, &new) {
                    return Err(format!("stopped: the edit would have changed existing lines in {}. Nothing was written there.", p.path));
                }
                self.obsidian.put_content(&p.path, &new).await.map_err(|e| e.to_string())?;
                results.push(format!("Added to {}", p.path));
            }
        }
        Ok(results)
    }

    /// Makes sure a daily or sprint note carries its routine to-dos (/bod and /eod, or /bow and /eow). Returns whether it added any.
    pub async fn ensure_routines(&self, path: &str) -> Result<bool, String> {
        if !crate::note_tasks::allowed_path(path) {
            return Err("only daily and sprint notes".into());
        }
        let note = self.obsidian.read_file(path).await.map_err(|e| e.to_string())?;
        let kind = if crate::profile::Profile::inside(path, &crate::profile::get().vault.weeks) { "week" } else { "day" };
        match crate::note_tasks::ensure_routines(&note, kind) {
            Some(new) => {
                self.obsidian.put_content(path, &new).await.map_err(|e| e.to_string())?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// First local (non-cloud) model, for work that must never reach a cloud one.
    pub fn local_llm(&self) -> Option<Arc<dyn LlmProvider>> {
        std::iter::once(&self.llm).chain(self.fallbacks.iter()).find(|p| !p.is_cloud()).cloned()
    }

    /// The drafted cleanup plan for every reviewed mailbox, as text.
    pub fn mail_plan(&self) -> String {
        let r = self.mail_review.lock().unwrap();
        let date = crate::conversation_log::date_format("%Y-%m-%d").unwrap_or_default();
        if r.accounts.is_empty() || r.status != "done" {
            return "No finished review yet. Run the inbox review first.".to_string();
        }
        r.accounts.iter().map(|a| crate::mail_review::plan_text(a, &date)).collect::<Vec<_>>().join("\n\n---\n\n")
    }

    /// The Mail card: unread counts and the newest unread primary messages from every authorized account, each
    /// tagged by the LOCAL model (reply / fyi / noise) in the background. Email text never leaves this machine.
    async fn mail_briefing(&self) -> Option<crate::needs_you::Briefing> {
        let data = crate::mail::dashboard_data().await?;
        // sort anything not yet sorted, in the background, with a local model only (batches of 18, JSON out)
        let todo: Vec<crate::mail::MsgMeta> = {
            let cache = self.mail_triage.lock().unwrap();
            data.iter().flat_map(|(_, _, ms)| ms.iter()).filter(|m| !cache.contains_key(&format!("{}:{}", m.account, m.id))).cloned().collect()
        };
        if !todo.is_empty() && !self.mail_triaging.swap(true, std::sync::atomic::Ordering::SeqCst) {
            match std::iter::once(&self.llm).chain(self.fallbacks.iter()).find(|p| !p.is_cloud()).cloned() {
                Some(provider) => {
                    let cache = self.mail_triage.clone();
                    let busy = self.mail_triaging.clone();
                    tokio::spawn(async move {
                        let now_ms = crate::needs_you::now_unix() * 1000;
                        let today = conversation_log::date_format("%Y-%m-%d").unwrap_or_default();
                        let reviewed: Vec<crate::mail_review::Reviewed> = todo
                            .iter()
                            .map(|m| crate::mail_review::Reviewed { account: m.account.clone(), id: m.id.clone(), from: m.from.clone(), subject: m.subject.clone(), age_days: ((now_ms - m.at_ms) / 86_400_000).max(0), bucket: 0, bulk: m.bulk, category: m.labels.iter().find_map(|l| l.strip_prefix("CATEGORY_")).map(|c| c.to_lowercase()).unwrap_or_default(), snippet: m.snippet.clone(), class: String::new(), why: String::new() })
                            .collect();
                        let to_ask: Vec<&crate::mail_review::Reviewed> = reviewed.iter().filter(|r| crate::mail_review::heuristic(r).is_none()).collect();
                        for chunk in to_ask.chunks(18) {
                            let (system, user) = crate::mail_review::build_prompt(chunk, &today);
                            let msgs = vec![Message::system(system), Message::user(user)];
                            let Ok(Ok(reply)) = tokio::time::timeout(std::time::Duration::from_secs(150), provider.chat_low_effort(&msgs)).await else { break };
                            let got = crate::mail_review::parse_classes(&reply.content);
                            let mut c = cache.lock().unwrap();
                            if c.len() > 800 {
                                c.clear();
                            }
                            for (j, r) in chunk.iter().enumerate() {
                                let class = got.iter().find(|(n, ..)| *n == j + 1).map(|(_, c, _)| c.clone()).unwrap_or_else(|| "unsure".into());
                                c.insert(format!("{}:{}", r.account, r.id), class);
                            }
                        }
                        busy.store(false, std::sync::atomic::Ordering::SeqCst);
                    });
                }
                None => self.mail_triaging.store(false, std::sync::atomic::Ordering::SeqCst),
            }
        }
        let tri = self.mail_triage.lock().unwrap().clone();
        crate::needs_you::mail_briefing(&data, &tri, crate::needs_you::now_unix() * 1000)
    }

    /// "Loki" briefing: engine, the local model on the model server, tools, context, and what waits on you.
    async fn loki_status_briefing(&self) -> serde_json::Value {
        let row = |head: &str, text: String| serde_json::json!({ "head": head, "text": text, "link": null });
        let mut rows = Vec::new();
        let mut notes: Vec<String> = Vec::new();
        let mut model = serde_json::Value::Null;
        rows.push(row("Engine", format!("{} (default)", self.configured_label)));
        if let Some((host, port)) = &self.ollama_endpoint {
            let url = format!("http://{host}:{port}/api/ps");
            let ps = tokio::time::timeout(std::time::Duration::from_secs(3), async { reqwest::get(&url).await.ok()?.json::<serde_json::Value>().await.ok() }).await.ok().flatten();
            match ps.as_ref().and_then(|p| p["models"].as_array()).and_then(|m| m.iter().find(|x| x["name"].as_str().map(|n| !n.contains("embed")).unwrap_or(false))) {
                Some(m) => {
                    let vram = m["size_vram"].as_f64().unwrap_or(0.0) / 1e9;
                    let ctx = m["context_length"].as_u64().unwrap_or(0) / 1024;
                    rows.push(row("Local model", format!("{} loaded, {:.0} GB on the GPU, {}k context", m["name"].as_str().unwrap_or("?"), vram, ctx)));
                    model = serde_json::json!({ "loaded": true, "name": m["name"], "vram_gb": vram.round(), "context_k": ctx });
                    notes.push("model loaded".into());
                }
                None => {
                    rows.push(row("Local model", "not loaded right now (it loads on the next message)".into()));
                    model = serde_json::json!({ "loaded": false });
                    notes.push("model idle".into());
                }
            }
        }
        let mcp = self.mcp_status();
        let up = mcp.iter().filter(|s| s.connected).count();
        let tools: usize = mcp.iter().filter(|s| s.connected).map(|s| s.tools).sum();
        rows.push(row("Tools (MCP)", format!("{up} of {} servers connected, {tools} tools", mcp.len())));
        let down: Vec<String> = mcp.iter().filter(|s| !s.connected).map(|s| s.name.clone()).collect();
        if !down.is_empty() {
            rows.push(row("Not connected", down.join(", ")));
            notes.push(format!("{} MCP down", down.len()));
        }
        let ctx = crate::context_stats::get();
        rows.push(row("Context", format!("{}k window, autocompact {}", ctx.num_ctx / 1024, if ctx.autocompact { "on" } else { "off" })));
        let reports = crate::troubleshoot::list_reports(30);
        let staged = reports.iter().filter(|r| r["fix"]["state"] == "staged").count();
        if staged > 0 {
            rows.push(row("Waiting on you", format!("{staged} fix{} ready to review (Reports)", if staged == 1 { "" } else { "es" })));
            notes.push(format!("{staged} fix to review"));
        }
        let blurb = if notes.is_empty() { "All systems normal".to_string() } else { notes.join(" · ") };
        let data = serde_json::json!({
            "engine": self.configured_label, "model": model,
            "mcp": { "up": up, "total": mcp.len(), "tools": tools, "down": down },
            "context": { "window_k": ctx.num_ctx / 1024, "autocompact": ctx.autocompact },
            "staged_fixes": staged,
            "step_limits": { "ordinary": crate::agent_limits::ordinary(), "routine": crate::agent_limits::routine() },
        });
        serde_json::json!({ "id": "loki-status", "kind": "loki", "title": "Loki", "blurb": blurb, "pill": "status", "link": null, "source_path": "loki-server", "rows": rows, "series": [], "data": data })
    }

    /// Adds the cached "why now" sentence to each item; any item without one is written in the
    /// background by the local model (never a cloud one) and shows up on the next refresh.
    fn attach_blurbs(&self, v: &mut serde_json::Value) {
        let date = v["date"].as_str().unwrap_or("").to_string();
        let mut missing: Vec<(String, String)> = Vec::new();
        {
            let cache = self.needs_blurbs.lock().unwrap();
            if let Some(items) = v["items"].as_array_mut() {
                for item in items.iter_mut() {
                    let key = format!("{}|{date}", item["id"].as_str().unwrap_or(""));
                    match cache.get(&key) {
                        Some(b) => item["blurb"] = serde_json::json!(b),
                        None => {
                            let ask = format!(
                                "Item: {}\nWhere it came from: {}\nWhen: {}\nKind: {}",
                                item["title"].as_str().unwrap_or(""),
                                item["why"].as_str().unwrap_or(""),
                                item["when"].as_str().unwrap_or("not timed"),
                                item["kind"].as_str().unwrap_or("")
                            );
                            missing.push((key, ask));
                        }
                    }
                }
            }
        }
        v["blurbs_pending"] = serde_json::json!(!missing.is_empty());
        if missing.is_empty() || self.needs_blurbing.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let Some(provider) = std::iter::once(&self.llm).chain(self.fallbacks.iter()).find(|p| !p.is_cloud()).cloned() else {
            self.needs_blurbing.store(false, std::sync::atomic::Ordering::SeqCst);
            return;
        };
        let cache = self.needs_blurbs.clone();
        let busy = self.needs_blurbing.clone();
        tokio::spawn(async move {
            for (key, ask) in missing {
                let msgs = vec![
                    Message::system(
                        "You write ONE calm, plain sentence (at most 22 words) telling the user why an item is worth their attention today. \
                         Use only the facts given; don't invent names, times or details. No exclamation marks and no urgency words such as \
                         urgent, critical, overdue or deadline. Reply with the sentence only.",
                    ),
                    Message::user(ask),
                ];
                match provider.chat_low_effort(&msgs).await {
                    Ok(reply) => {
                        let text: String = reply.content.trim().trim_matches('"').lines().next().unwrap_or("").chars().take(170).collect();
                        if !text.is_empty() {
                            let mut c = cache.lock().unwrap();
                            if c.len() > 400 {
                                c.clear();
                            }
                            c.insert(key, text);
                        }
                    }
                    Err(e) => {
                        eprintln!("[loki-debug] needs-you blurb failed (non-fatal): {e}");
                        break;
                    }
                }
            }
            busy.store(false, std::sync::atomic::Ordering::SeqCst);
        });
    }

    /// One calendar day as a page (`GET /api/v1/day?date=`).
    pub async fn day_page(&self, date: &str) -> crate::needs_you::Briefing {
        crate::needs_you::build_day(&self.obsidian, date).await
    }

    /// Records a decision on a "Needs you" item and appends it to the vault's findings file.
    pub async fn needs_you_decide(&self, id: &str, title: &str, action: &str, days: Option<i64>, reason: &str, rating: &str) -> Result<(), String> {
        let today = crate::needs_you::clock().date;
        let line = crate::needs_you::record(id, title, action, days, reason, rating, &today)?;
        // Plain decisions stay in the local store; only ones with a reason or rating are worth a vault line.
        if reason.trim().is_empty() && rating.is_empty() && action != "defer" {
            return Ok(());
        }
        let findings = format!("{}/findings/needs-you-findings.md", crate::profile::get().vault.agent);
        let path = findings.as_str();
        let text = if self.obsidian.read_file(path).await.is_ok() { format!("{line}\n") } else { format!("{}{line}\n", crate::needs_you::FINDINGS_HEADER) };
        self.obsidian.append_content(path, &text).await.map_err(|e| e.to_string())
    }

    /// Slash commands Loki answers itself without a model: `/context`, `/trusted`,
    /// `/trust <device|host> <folder> [always|<minutes>]`, `/untrust <device|host> <folder>`.
    fn info_command(&self, session_id: &str, message: &str) -> Option<String> {
        let m = message.trim();
        if m.eq_ignore_ascii_case("/context") {
            return Some(crate::context_stats::render(&self.context_report(session_id)));
        }
        if let Some(r) = crate::agent_limits::command(m, 25, 60) {
            return Some(r);
        }
        if m.eq_ignore_ascii_case("/mcp") {
            return Some(crate::mcp_manage::status_text());
        }
        if let Some(rest) = m.strip_prefix("/mcp ").or_else(|| m.strip_prefix("/MCP ")) {
            let mut w = rest.split_whitespace();
            let (Some(verb), Some(name), None) = (w.next(), w.next(), w.next()) else { return Some("Usage: `/mcp` to list, or `/mcp enable|disable|reconnect <name>`.".into()) };
            if !matches!(verb, "enable" | "disable" | "reconnect") {
                return Some("Typing `/mcp` supports enable, disable and reconnect. To add or remove a server, use Settings, or ask me and approve the prompt.".into());
            }
            let a = crate::mcp_manage::Action { action: verb.to_string(), name: name.to_string(), ..Default::default() };
            tokio::spawn(async move {
                if let Err(e) = crate::mcp_manage::act(a).await {
                    eprintln!("[loki-debug] /mcp failed: {e}");
                }
            });
            return Some(format!("Asked to {verb} '{name}'. Run `/mcp` in a few seconds to see the result."));
        }
        let rules = self.bridge.rules();
        if m.eq_ignore_ascii_case("/trusted") {
            let list = rules.trusted_workspaces();
            if list.is_empty() {
                return Some("No trusted folders. `/trust device ~/projects/x` (this computer's desktop app) or `/trust <host> ~/projects/x` trusts one for 2 hours; add `always` to keep it.".into());
            }
            let lines: Vec<String> = list
                .into_iter()
                .map(|(h, p, left)| format!("- {h}: {p} — {}", left.map(|s| format!("{} min left", s / 60 + 1)).unwrap_or_else(|| "always".into())))
                .collect();
            return Some(format!("Trusted folders (edits and non-destructive commands inside them stop asking):\n{}\n\n`/untrust <host> <folder>` removes one.", lines.join("\n")));
        }
        let (verb, rest) = m.split_once(char::is_whitespace).map(|(v, r)| (v, r.trim())).unwrap_or((m, ""));
        let trust = verb.eq_ignore_ascii_case("/trust");
        if !trust && !verb.eq_ignore_ascii_case("/untrust") {
            return None;
        }
        let usage = "Usage: `/trust <device|host> <folder> [always|minutes]` or `/untrust <device|host> <folder>`. Hosts: device (the desktop app's computer), ".to_string() + &self.remote_hosts.join(", ");
        let mut words = rest.split_whitespace();
        let (Some(host), Some(path)) = (words.next(), words.next()) else { return Some(usage) };
        let host = if matches!(host, "here" | "desktop" | "local") { crate::permission_rules::DEVICE_HOST } else { host };
        if host != crate::permission_rules::DEVICE_HOST && !self.remote_hosts.iter().any(|h| h == host) {
            return Some(format!("'{host}' isn't one I can reach. {usage}"));
        }
        if !trust {
            return Some(if rules.untrust_workspace(host, path) { format!("No longer trusting {path} on {host}.") } else { format!("{path} wasn't trusted on {host}.") });
        }
        let ttl = match words.next() {
            Some("always") | Some("permanent") => None,
            Some(n) => match n.parse::<u64>() {
                Ok(min) => Some(std::time::Duration::from_secs(min.clamp(1, 1440) * 60)),
                Err(_) => return Some(usage),
            },
            None => Some(std::time::Duration::from_secs(120 * 60)),
        };
        Some(match rules.trust_workspace(host, path, ttl) {
            Ok(p) => format!(
                "Trusting {p} on {host} {}. File edits and non-destructive commands inside it won't ask; deletes, redirects, secrets and anything outside it still do.",
                if ttl.is_none() { "until you remove it".to_string() } else { format!("for {} min", ttl.unwrap().as_secs() / 60) }
            ),
            Err(e) => e,
        })
    }

    /// What fills this conversation's context window, how close autocompact is, and the live
    /// settings — for the context bar/panel in every UI and for `/context`. Token figures are
    /// estimates (`context_window::est_tokens`); `last_measured` is what the model itself counted.
    pub fn context_report(&self, session_id: &str) -> serde_json::Value {
        use crate::context_window::{est_tokens, msg_tokens, window_start, ContextPolicy};
        let settings = crate::context_stats::get();
        let budget = settings.budget_tokens();
        let (history, summary, summarized_upto) = {
            let sessions = self.sessions.lock().unwrap();
            match sessions.get(session_id) {
                Some(s) => (s.history.clone(), s.summary.clone(), s.summarized_upto),
                None => (Vec::new(), String::new(), 0),
            }
        };
        let system = history.first().filter(|m| m.role == "system").map(|m| est_tokens(&m.content)).unwrap_or_else(|| est_tokens(&self.local_context.prompt_block) + 1_500);
        let tools = est_tokens(&serde_json::to_string(&self.tools.definitions()).unwrap_or_default());
        let summary_tokens = if summary.is_empty() { 0 } else { est_tokens(&summary) + 20 };
        let policy = ContextPolicy { budget_tokens: budget, summarized_upto };
        let start = if history.len() > 1 { window_start(&history, &policy, agent::MIN_WINDOW_MESSAGES) } else { history.len() };
        let verbatim: Vec<&Message> = history.iter().skip(start.max(1)).collect();
        let history_tokens: usize = verbatim.iter().map(|m| msg_tokens(m)).sum();
        let verbatim_turns = verbatim.iter().filter(|m| router::is_turn(m)).count();
        let turn_tokens: Vec<usize> = history.iter().filter(|m| router::is_turn(m)).map(msg_tokens).collect();
        let total_turns = turn_tokens.len();
        let unsum_tokens: usize = turn_tokens.iter().skip(summarized_upto).sum();
        let unsum_turns = total_turns.saturating_sub(summarized_upto);
        let at_tokens = budget * 9 / 10;
        let percent = (unsum_tokens * 100 / at_tokens.max(1)).max(unsum_turns * 100 / settings.max_turns.max(1)).min(100);
        let used = system + tools + summary_tokens + history_tokens;
        serde_json::json!({
            "session_id": session_id,
            "window": settings.num_ctx,
            "used": used,
            "breakdown": { "system": system, "tools": tools, "summary": summary_tokens, "history": history_tokens },
            "history_budget": budget,
            "turns": { "total": total_turns, "summarized": summarized_upto, "verbatim": verbatim_turns },
            "autocompact": {
                "enabled": settings.autocompact,
                "at_tokens": at_tokens,
                "at_turns": settings.max_turns,
                "unsummarized_tokens": unsum_tokens,
                "unsummarized_turns": unsum_turns,
                "percent": percent,
            },
            "last_measured": crate::context_stats::last_measured(session_id),
            "settings": settings,
        })
    }

    /// `/compact`: fold everything but the newest few turns into the rolling summary now, and file
    /// the folded exchanges (tagged) for recall. Returns the message shown to the user.
    pub async fn compact_session(&self, session_id: &str) -> Result<String, String> {
        const KEEP: usize = 4;
        let provider = std::iter::once(&self.llm).chain(self.fallbacks.iter()).find(|p| !p.is_cloud()).cloned().ok_or("Compacting needs a local model to write the summary, and none is configured.")?;
        let (existing, turns, target, private, total, before_tokens, docs) = {
            let mut sessions = self.sessions.lock().unwrap();
            let Some(state) = sessions.get_mut(session_id) else { return Ok("Nothing to compact yet — this conversation is empty.".into()) };
            if state.summarizing {
                return Ok("A summary is already being written in the background; try again in a moment.".into());
            }
            let convo: Vec<&Message> = state.history.iter().filter(|m| router::is_turn(m)).collect();
            let total = convo.len();
            if total <= KEEP || total - KEEP <= state.summarized_upto {
                return Ok(format!("Nothing to compact — only {} turn(s) are outside the newest {KEEP}, and they're already summarized.", total.saturating_sub(KEEP)));
            }
            let target = total - KEEP;
            let before_tokens: usize = convo[state.summarized_upto..].iter().map(|m| crate::context_window::msg_tokens(m)).sum();
            // File the exchanges being folded (bounded: newest 40 of them).
            let first_user = (0..target).rev().filter(|i| convo[*i].role == "user").nth(39).unwrap_or(0);
            let docs: Vec<IngestDoc> = (first_user..target).filter(|i| convo[*i].role == "user").filter_map(|i| Self::exchange_doc(session_id, &convo, i, &state.private_turns)).collect();
            let turns: Vec<(usize, Message)> = convo.iter().map(|m| (*m).clone()).enumerate().skip(state.summarized_upto).take(target - state.summarized_upto).collect();
            state.summarizing = true;
            (state.summary.clone(), turns, target, state.private_turns.clone(), total, before_tokens, docs)
        };
        let result = session_store::summarize(&provider, &existing, &turns, &private).await;
        let filed = docs.len();
        {
            let mut sessions = self.sessions.lock().unwrap();
            if let Some(state) = sessions.get_mut(session_id) {
                state.summarizing = false;
                if let Ok(text) = &result {
                    state.summary = text.clone();
                    state.summarized_upto = target;
                }
            }
        }
        let summary = result.map_err(|e| format!("Couldn't write the summary: {e}"))?;
        persist_state(&self.sessions, session_id);
        let knowledge = self.knowledge.clone();
        tokio::spawn(async move {
            for doc in docs {
                if let Err(e) = knowledge.ingest(doc).await {
                    eprintln!("[loki-debug] filing exchange during /compact failed (non-fatal): {e}");
                }
            }
        });
        let after_tokens = crate::context_window::est_tokens(&summary);
        Ok(format!(
            "Compacted. {} older turn(s) are now a summary (about {after_tokens} tokens instead of ~{before_tokens}); the newest {KEEP} of {total} turns stay verbatim. \
             {filed} exchange(s) are being filed, tagged, for recall — I search them automatically when they matter, or you can ask me to recall something specific.",
            target
        ))
    }

    /// Whether `session_id` has an active Claude Code session — true from its first clean
    /// (non-local-only) turn onward, since Claude Code is the default backend now, not an
    /// opt-in named-routine mode. Used by the `/api/v1/chat` handler and the Tauri desktop
    /// command to surface a UX signal without the caller needing to know anything about
    /// `SessionState`'s internals.
    pub fn is_in_routine_mode(&self, session_id: &str) -> bool {
        self.sessions.lock().unwrap().get(session_id).map(|s| s.routine_claude_session.is_some()).unwrap_or(false)
    }

    /// Whether `session_id` currently has the `/exit` local-only override active — see
    /// `ROUTINE_EXIT_COMMAND`'s doc comment. Surfaced alongside `is_in_routine_mode` so the
    /// frontend can distinguish "never talked to Claude yet" from "deliberately forced local."
    /// Whether the conversation's newest exchange stayed local for privacy (its text must never be
    /// put in a push notification, which travels through Google's push service).
    pub fn last_turn_private(&self, session_id: &str) -> bool {
        let guard = self.sessions.lock().unwrap();
        guard.get(session_id).map(|s| {
            let n = router::convo_len(&s.history);
            n > 0 && (s.private_turns.contains(&(n - 1)) || s.force_local)
        }).unwrap_or(false)
    }

    pub fn speech_config(&self) -> &crate::speech::SpeechConfig {
        &self.speech_cfg
    }

    pub fn is_busy(&self, session_id: &str) -> bool {
        self.sessions.lock().unwrap().get(session_id).map(|s| s.busy).unwrap_or(false)
    }

    pub fn is_force_local(&self, session_id: &str) -> bool {
        self.sessions.lock().unwrap().get(session_id).map(|s| s.force_local).unwrap_or(false)
    }

    pub async fn get_history_in_session(&self, session_id: &str) -> Vec<ChatTurn> {
        self.ensure_session_seeded(session_id).await;
        let sessions = self.sessions.lock().unwrap();
        sessions
            .get(session_id)
            .map(|s| {
                let audits = crate::audit::latest_by_index(session_id);
                s.history
                    .iter()
                    .filter(|m| router::is_turn(m))
                    .enumerate()
                    .map(|(i, m)| ChatTurn {
                        role: m.role.clone(),
                        content: m.display.clone().unwrap_or_else(|| m.content.clone()),
                        index: i,
                        at: m.at.clone(),
                        feedback: s.feedback.get(&i).cloned(),
                        audit: audits.get(&i).cloned(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Direct knowledge-store query, bypassing the chat loop entirely — the "plug context
    /// into other things" capability (Phase 2.5): another caller (REST or MCP) asks "what
    /// does Loki know about X" and gets ranked context back to use in its own prompt, without
    /// a conversational round-trip or the latency/cost of a full agent turn. Shared by both
    /// the `/api/v1/query` handler and the `loki_query` MCP tool — one implementation.
    pub async fn query_knowledge(
        &self,
        query: &str,
        n: u32,
        session_id: Option<String>,
    ) -> anyhow::Result<Vec<SearchResult>> {
        self.knowledge.query(query, n, QueryFilter { source_type: None, session_id }).await
    }

    /// Lets an external caller push content directly into the knowledge store — the inverse
    /// of a query, and the inverse direction of Phase 4/5's pull-based PKMS import/connect.
    /// Shared by `/api/v1/ingest` and the `loki_ingest` MCP tool.
    pub async fn ingest_external(
        &self,
        title: Option<String>,
        content: String,
        tags: Vec<String>,
        session_id: Option<String>,
    ) -> anyhow::Result<Vec<String>> {
        let source_id = format!("external:{}", now_unix_nanos());
        self.knowledge
            .ingest(IngestDoc { source_type: "external_ingest".to_string(), source_id, title, content, tags, session_id })
            .await
    }
}

fn now_unix_nanos() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)
}
