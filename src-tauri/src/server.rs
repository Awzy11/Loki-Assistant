use crate::config_api::{self, BackendInput, PublicBackend};
use crate::core::{Core, SendMessageError};
use crate::knowledge::SearchResult;
use axum::{
    body::Bytes,
    extract::{Multipart, Path, Query, RawQuery, State},
    http::{HeaderMap, Method, StatusCode},
    middleware,
    response::{IntoResponse, Json},
    routing::{get, post},
    Router,
};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::{DefaultMakeSpan, DefaultOnResponse, TraceLayer};
use tracing::Level;

/// `section` is `"llm"` or `"embedding"`. Reads/writes `config.toml` + the encrypted
/// secrets file directly, independent of `core` — a save here takes effect on Loki's next
/// restart, not live.
async fn get_llm_config(Path(section): Path<String>) -> Result<Json<Option<PublicBackend>>, (StatusCode, String)> {
    config_api::get_backend(&section).map(Json).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

async fn save_llm_config(
    Path(section): Path<String>,
    Json(input): Json<BackendInput>,
) -> Result<StatusCode, (StatusCode, String)> {
    config_api::save_backend(&section, input)
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

/// Dedicated pair for `[claude]` — see `config_api::get_claude_config`'s doc comment for why
/// this doesn't go through the generic `{section}` handlers above (vendor/api_key-shaped,
/// doesn't fit `binary_path`/`timeout_secs`/`routine_working_dir`).
async fn get_claude_config() -> Result<Json<crate::config::ClaudeConfig>, (StatusCode, String)> {
    config_api::get_claude_config().map(Json).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

async fn save_claude_config(Json(input): Json<crate::config::ClaudeConfig>) -> Result<StatusCode, (StatusCode, String)> {
    config_api::save_claude_config(input)
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

// --- /api/login — a friendlier front door onto the same bearer token (2026-09-01) ---

#[derive(Deserialize)]
struct LoginRequest {
    username: String,
    password: String,
}

#[derive(serde::Serialize)]
struct LoginResponse {
    token: String,
}

/// Unauthenticated by necessity (you don't have the token yet — that's the point) — see
/// `auth::login`'s doc comment for why a bad username and a bad password produce the same
/// generic failure here.
async fn login(Json(req): Json<LoginRequest>) -> Result<Json<LoginResponse>, StatusCode> {
    crate::auth::login(&req.username, &req.password).map(|token| Json(LoginResponse { token })).map_err(|_| StatusCode::UNAUTHORIZED)
}

// --- /api/v1/* — the external integration surface (Phase 2.5) ---

#[derive(Deserialize)]
struct V1ChatRequest {
    #[serde(default)]
    session_id: Option<String>,
    message: String,
    /// Base64-encoded images (no data: URI prefix) attached to this turn — e.g. a
    /// device-context screenshot (#13). See `Message::images`'s doc comment for which
    /// provider actually forwards these.
    #[serde(default)]
    images: Vec<String>,
    /// Who is calling and what it can do (see `bus::ClientInfo`). Optional: a bare client is
    /// attached without capabilities, so device requests are never routed to it.
    #[serde(default)]
    client: Option<crate::bus::ClientInfo>,
    /// Keep the turn running if this connection drops (a phone leaving the page); the caller
    /// cancels explicitly via `POST /api/v1/chat/cancel`. Without it, a dropped stream aborts
    /// the turn (what `loki-cli` relies on for Stop).
    #[serde(default)]
    detach: bool,
}

#[derive(serde::Serialize)]
struct V1ChatResponse {
    session_id: String,
    reply: String,
    /// Lets the frontend show a "talking to Claude Code, type /exit to force local" indicator
    /// without knowing anything about the routing internals. See `Core::is_in_routine_mode`.
    in_routine_mode: bool,
    /// Whether the `/exit` local-only override is active for this session. See
    /// `Core::is_force_local`.
    force_local: bool,
    /// Which backend produced *this specific* reply — `"claude"`, a local provider name
    /// (e.g. `"ollama"`), or `"system"`. See `core::SentTurn`'s doc comment for why this is
    /// per-turn rather than folded into `in_routine_mode`.
    via: String,
    /// Why a fallback answered, if one did — see `core::SentTurn::note`.
    note: Option<String>,
}

/// Shared by `v1_chat` and the voice-note webhook — same underlying turn, different body
/// shapes on the way in (JSON vs. multipart form data; see `webhook_voice_note`'s doc comment
/// for why the webhook doesn't just reuse `v1_chat` directly).
async fn chat_turn(core: &Core, session_id: String, message: String) -> Result<Json<V1ChatResponse>, (StatusCode, String)> {
    chat_turn_with_images(core, session_id, message, Vec::new()).await
}

/// Same as `chat_turn`, plus base64-encoded images attached to this turn (#13).
async fn chat_turn_with_images(
    core: &Core,
    session_id: String,
    message: String,
    images: Vec<String>,
) -> Result<Json<V1ChatResponse>, (StatusCode, String)> {
    // allow_routines: true — every caller of this sits behind bearer-token auth of some kind
    // (see `router()`), which is the deliberate gate for Routine Mode's real Claude Code
    // session access.
    core.send_message_in_session_with_images(&session_id, message, true, images)
        .await
        .map(|turn| {
            let in_routine_mode = core.is_in_routine_mode(&session_id);
            let force_local = core.is_force_local(&session_id);
            Json(V1ChatResponse { session_id, reply: turn.reply, in_routine_mode, force_local, via: turn.via, note: turn.note })
        })
        .map_err(|e| {
            let status = match &e {
                SendMessageError::Busy => StatusCode::CONFLICT,
                SendMessageError::Other(_) => StatusCode::INTERNAL_SERVER_ERROR,
            };
            (status, e.to_string())
        })
}

/// Session-scoped chat — unlike legacy `/api/message` (always `DEFAULT_SESSION`), an
/// external caller gets its own independent conversation. Omit `session_id` to start a new
/// one; the response always echoes back which session was used.
async fn v1_chat(State(core): State<Arc<Core>>, Json(req): Json<V1ChatRequest>) -> Result<Json<V1ChatResponse>, (StatusCode, String)> {
    let session_id = req.session_id.unwrap_or_else(|| core.new_session());
    chat_turn_with_images(&core, session_id, req.message, req.images).await
}

/// `multipart/form-data`, not JSON — the Pebble Index 01's phone companion app posts three
/// fields (`transcription`, `recordedAt` — unix ms, `client` — device name, e.g. `"ring"`).
/// Confirmed live, 2026-08-30, by watching an actual real payload land verbatim in the vault
/// conversation log (a plain-`String`-body version of this handler was tried first, based on
/// an earlier 415 that turned out to mean "wrong Content-Type for Json", not "not JSON at
/// all" — the raw multipart boundary markers ended up in the message text and confused the
/// model, which is what surfaced the real format). Only `transcription` becomes the chat
/// message; `recordedAt`/`client` are parsed but not currently used for anything (no obvious
/// need yet — logged as a debug line in case that changes). Always starts a fresh session (a
/// voice note doesn't carry a `session_id` to resume) — every voice note is its own standalone
/// conversation, logged to `98 - Loki Conversations/` like any other turn.
async fn webhook_voice_note(State(core): State<Arc<Core>>, mut multipart: Multipart) -> Result<Json<V1ChatResponse>, (StatusCode, String)> {
    let mut transcription: Option<String> = None;
    let mut client: Option<String> = None;
    let mut recorded_at: Option<String> = None;

    while let Some(field) =
        multipart.next_field().await.map_err(|e| (StatusCode::BAD_REQUEST, format!("malformed multipart body: {e}")))?
    {
        let name = field.name().unwrap_or("").to_string();
        let text = field.text().await.map_err(|e| (StatusCode::BAD_REQUEST, format!("couldn't read field '{name}': {e}")))?;
        match name.as_str() {
            "transcription" => transcription = Some(text),
            "client" => client = Some(text),
            "recordedAt" => recorded_at = Some(text),
            _ => {} // unrecognized field — ignore rather than reject, in case Pebble adds more later
        }
    }

    let message = transcription
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "multipart body had no non-empty 'transcription' field".to_string()))?;

    eprintln!("[loki-debug] voice-note webhook: client={client:?} recordedAt={recorded_at:?}");

    let session_id = core.new_session();
    chat_turn(&core, session_id, message).await
}

#[derive(Deserialize)]
struct V1QueryRequest {
    query: String,
    #[serde(default = "default_query_n")]
    n: u32,
    #[serde(default)]
    session_id: Option<String>,
}

fn default_query_n() -> u32 {
    5
}

/// Direct knowledge-store query, bypassing the chat loop — the "plug context into other
/// things" capability: another app asks "what does Loki know about X" and gets ranked
/// context back to build its own prompt with, without a full conversational round trip.
async fn v1_query(
    State(core): State<Arc<Core>>,
    Json(req): Json<V1QueryRequest>,
) -> Result<Json<Vec<SearchResult>>, (StatusCode, String)> {
    core.query_knowledge(&req.query, req.n, req.session_id)
        .await
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

#[derive(Deserialize)]
struct V1IngestRequest {
    #[serde(default)]
    title: Option<String>,
    content: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    session_id: Option<String>,
}

#[derive(serde::Serialize)]
struct V1IngestResponse {
    chunk_ids: Vec<String>,
}

/// Lets an external caller push content directly into Loki's knowledge store — the inverse
/// of `v1_query`, and the inverse direction of Phase 4/5's pull-based PKMS import/connect
/// (e.g. an n8n workflow handing Loki a fact to remember without a chat turn).
async fn v1_ingest(
    State(core): State<Arc<Core>>,
    Json(req): Json<V1IngestRequest>,
) -> Result<Json<V1IngestResponse>, (StatusCode, String)> {
    core.ingest_external(req.title, req.content, req.tags, req.session_id)
        .await
        .map(|chunk_ids| Json(V1IngestResponse { chunk_ids }))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

#[derive(serde::Serialize)]
struct V1HistoryResponse {
    turns: Vec<crate::core::ChatTurn>,
    in_routine_mode: bool,
    force_local: bool,
    /// A turn is running in this session right now (lets a client that lost its stream reattach).
    busy: bool,
}

async fn v1_history(State(core): State<Arc<Core>>, Query(params): Query<HashMap<String, String>>) -> impl IntoResponse {
    let session_id = params.get("session_id").cloned().unwrap_or_else(|| "default".to_string());
    let turns = core.get_history_in_session(&session_id).await;
    let in_routine_mode = core.is_in_routine_mode(&session_id);
    let force_local = core.is_force_local(&session_id);
    let busy = core.is_busy(&session_id);
    Json(V1HistoryResponse { turns, in_routine_mode, force_local, busy })
}

/// Builds the HTTP API router. `static_dir`, if given, is the built frontend
/// (`npm run build`'s `dist/` output) — same one Tauri bundles into the desktop
/// app — served at `/`, with unmatched routes falling back to `index.html` so
/// this is a genuine standalone deployment, not just an API.
///
/// Every route sits behind bearer-token auth (`auth.rs`) — `/api/config/*`, `/api/v1/*`.
/// There used to also be an unauthenticated legacy `/api/message`/`/api/history` pair (v0,
/// predating sessions and Routine Mode), justified at the time by "the chat UI has no token
/// plumbing to call anything else with." That stopped being true once the frontend migrated
/// to the session-scoped v1 path — the UI does have token plumbing now, it just doesn't call
/// the legacy routes — but the routes themselves stayed open with no remaining justification:
/// full local-model chat, real vault reads/writes, unauthenticated, reachable by anything on
/// the LAN/Tailscale. Removed entirely (2026-08-30) rather than patched, since nothing legit
/// depends on them anymore — see `core.rs`'s `new_session` doc comment for the fuller story.
pub fn router(core: Arc<Core>, static_dir: Option<PathBuf>) -> Router {
    // No auth middleware — logging in is how you get the token in the first place.
    let login_route = Router::new().route("/api/login", post(login));

    let api = Router::new()
        .route("/api/config/{section}", get(get_llm_config).post(save_llm_config))
        .route("/api/config/claude", get(get_claude_config).post(save_claude_config))
        .route("/api/v1/chat", post(v1_chat))
        .route("/api/v1/chat/stream", post(v1_chat_stream))
        .route("/api/v1/chat/cancel", post(v1_chat_cancel))
        .route("/api/v1/sessions/events", get(v1_watch))
        .route("/api/v1/bridge/respond", post(bridge_respond))
        .route("/api/v1/bridge/device", post(bridge_device))
        .route("/api/v1/bridge/permission", post(bridge_permission))
        .route("/api/v1/query", post(v1_query))
        .route("/api/v1/ingest", post(v1_ingest))
        .route("/api/v1/history", get(v1_history))
        .route("/api/v1/sessions", get(v1_sessions))
        .route("/api/v1/sessions/rename", post(v1_session_rename))
        .route("/api/v1/commands", get(v1_commands))
        .route("/api/v1/feedback", post(v1_feedback))
        .route("/api/v1/audit", post(v1_audit))
        .route("/api/v1/lessons", get(v1_lessons))
        .route("/api/v1/lessons/update", post(v1_lessons_update))
        .route("/api/v1/audit/settings", post(v1_audit_settings))
        .route("/api/v1/permissions/workspaces", get(ws_list))
        .route("/api/v1/permissions/workspaces/grant", post(ws_grant))
        .route("/api/v1/permissions/workspaces/revoke", post(ws_revoke))
        .route("/api/v1/needs-you", get(v1_needs_you))
        .route("/api/v1/needs-you/decision", post(v1_needs_you_decision))
        .route("/api/v1/day", get(v1_day))
        .route("/api/v1/mail/status", get(v1_mail_status))
        .route("/api/v1/mail/review/start", post(v1_mail_review_start))
        .route("/api/v1/mail/review/status", get(v1_mail_review_status))
        .route("/api/v1/mail/review", get(v1_mail_review))
        .route("/api/v1/mail/plan", get(v1_mail_plan))
        .route("/api/v1/improve/run", post(v1_improve_run))
        .route("/api/v1/news/geo", get(v1_news_geo))
        .route("/api/v1/news/render", post(v1_news_render))
        .route("/api/v1/ai/fabric", post(v1_ai_fabric))
        .route("/api/v1/workflows/run", post(v1_workflow_run))
        .route("/api/v1/routines", get(v1_routines))
        .route("/api/v1/leads/{*rest}", axum::routing::any(v1_leads_proxy))
        .route("/api/v1/tasks/mark", post(v1_tasks_mark))
        .route("/api/v1/backlog/item", get(v1_backlog_item).post(v1_backlog_edit))
        .route("/api/v1/notes/find", get(v1_notes_find))
        .route("/api/v1/notes/plan", post(v1_notes_plan))
        .route("/api/v1/notes/apply", post(v1_notes_apply))
        .route("/api/v1/tasks/ensure-routines", post(v1_ensure_routines))
        .route("/api/v1/finance/sync", post(v1_finance_sync))
        .route("/api/v1/extract", post(v1_extract))
        .route("/api/v1/extract/apply", post(v1_extract_apply))
        .route("/api/v1/mail/inbox", get(v1_mail_inbox))
        .route("/api/v1/mail/newsletters", get(v1_newsletters))
        .route("/api/v1/mail/newsletters/digested", post(v1_newsletters_digested))
        .route("/api/v1/mail/newsletters/senders", get(v1_newsletter_senders).post(v1_newsletter_sender_set))
        .route("/api/v1/mail/inbox/status", get(v1_mail_inbox_status))
        .route("/api/v1/mail/inbox/scan", post(v1_mail_inbox_scan))
        .route("/api/v1/mail/inbox/act", post(v1_mail_inbox_act))
        .route("/api/v1/mail/inbox/message", get(v1_mail_inbox_message))
        .route("/api/v1/mail/inbox/draft", post(v1_mail_inbox_draft))
        .route("/api/v1/context", get(v1_context))
        .route("/api/v1/context/settings", post(v1_context_settings))
        .route("/api/v1/troubleshoot", post(v1_troubleshoot).layer(axum::extract::DefaultBodyLimit::max(40 * 1024 * 1024)))
        .route("/api/v1/troubleshoot/{id}", get(v1_troubleshoot_status))
        .route("/api/v1/fix", post(v1_fix))
        .route("/api/v1/push/status", get(v1_push_status))
        .route("/api/v1/push/subscribe", post(v1_push_subscribe))
        .route("/api/v1/push/unsubscribe", post(v1_push_unsubscribe))
        .route("/api/v1/push/test", post(v1_push_test))
        .route("/api/v1/speech/status", get(v1_speech_status))
        .route("/api/v1/stt", post(v1_stt).layer(axum::extract::DefaultBodyLimit::max(25 * 1024 * 1024)))
        .route("/api/v1/tts", post(v1_tts))
        .route("/api/v1/reports", get(v1_reports))
        .route("/api/v1/reports/{id}", get(v1_report))
        .route("/api/v1/reports/{id}/fix-record", post(v1_fix_record))
        .route("/api/v1/mcp", get(v1_mcp_status))
        .route("/api/v1/mcp/servers", get(v1_mcp_servers))
        .route("/api/v1/mcp/servers/{name}/tools", get(v1_mcp_tools))
        .route("/api/v1/mcp/action", post(v1_mcp_action))
        .route("/api/v1/engines", get(engines_list).post(engines_set))
        .route("/api/v1/permissions/rules", get(rules_list).post(rules_add))
        .route("/api/v1/permissions/rules/remove", post(rules_remove))
        .route("/api/v1/permissions/hosts", get(hosts_list))
        .route("/api/v1/permissions/hosts/grant", post(hosts_grant))
        .route("/api/v1/permissions/hosts/revoke", post(hosts_revoke))
        .route_layer(middleware::from_fn(crate::auth::require_bearer_token))
        .with_state(core.clone());

    // A separate token from the rest of `/api/v1/*` — this one gets handed to the Pebble
    // Index 01's phone companion app, a *device*, not typed into Settings once by the user
    // himself. `webhook_voice_note` shares `v1_chat`'s underlying turn logic (`chat_turn`) —
    // a voice note is just a chat message from a different kind of caller — so conversation
    // logging to `98 - Loki Conversations/` is automatic, for free, via the same path every
    // other chat turn already takes. No live push/desktop notification on the reply yet
    // (deliberately deferred — see "Webhook" in the README); check `98 - Loki Conversations/`
    // or the chat UI to see it.
    let webhook = Router::new()
        .route("/api/v1/webhook/voice-note", post(webhook_voice_note))
        .route_layer(middleware::from_fn(crate::auth::require_webhook_token))
        .with_state(core);

    // TraceLayer added last (outermost) so it logs every request that reaches this process at
    // all — including ones the auth middleware rejects — since that's exactly the "did it even
    // arrive, and with what status" visibility this codebase didn't have until a real webhook
    // debugging session (2026-08-30) needed it.
    let combined = api
        .merge(webhook)
        .merge(login_route)
        .layer(CorsLayer::new().allow_origin(Any).allow_methods(Any).allow_headers(Any))
        .layer(
            // Explicit INFO level on both ends — TraceLayer's own defaults are DEBUG, which
            // the default "info" log level (see bin/server.rs) silently drops. Found the hard
            // way: the layer compiled and ran with zero visible output against a real test
            // request until this was made explicit.
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::new().level(Level::INFO))
                .on_response(DefaultOnResponse::new().level(Level::INFO)),
        );

    with_app_shell(combined, static_dir)
}

/// Adds the built web app (and its caching rules) behind `routes`. Unknown `/api/` paths stay a real 404; every other path gets the app shell.
pub fn with_app_shell(combined: Router, static_dir: Option<PathBuf>) -> Router {
    match static_dir {
        Some(dir) => {
            let index = dir.join("index.html");
            let serve_dir = ServeDir::new(&dir).fallback(ServeFile::new(index));
            // Without a Cache-Control header browsers guess how long to keep index.html from its
            // age, so a webview (the desktop app) kept showing the pre-deploy UI for hours. The
            // shell and service worker always revalidate; hashed build files never change.
            let static_service = Router::new().fallback_service(serve_dir).layer(axum::middleware::from_fn(|req: axum::extract::Request, next: axum::middleware::Next| async move {
                let path = req.uri().path().to_string();
                // Pages (/b/news, /chat, …) all get the app shell; an unknown API path is a real 404.
                if path.starts_with("/api/") {
                    return axum::http::StatusCode::NOT_FOUND.into_response();
                }
                let mut res = next.run(req).await;
                let value = if path.starts_with("/assets/") { "public, max-age=31536000, immutable" } else { "no-cache" };
                res.headers_mut().insert(axum::http::header::CACHE_CONTROL, axum::http::HeaderValue::from_static(value));
                res
            }));
            combined.fallback_service(static_service)
        }
        None => combined,
    }
}


/// Aborts the spawned turn when dropped. The SSE response stream owns one, so a client that
/// disconnects (closes the tab, or presses Stop, which aborts its fetch) drops the stream,
/// which aborts the turn: the Claude child is killed and the session's busy flag released.
struct AbortOnDrop(Option<tokio::task::AbortHandle>);
impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        if let Some(h) = self.0.take() {
            h.abort();
        }
    }
}

/// Running streamed turns by session, so `POST /api/v1/chat/cancel` can stop a detached one.
static RUNNING_TURNS: std::sync::LazyLock<std::sync::Mutex<HashMap<String, tokio::task::AbortHandle>>> =
    std::sync::LazyLock::new(Default::default);

fn sse_event(ev: &crate::events::ChatEvent) -> axum::response::sse::Event {
    use axum::response::sse::Event;
    let name = match serde_json::to_value(ev) {
        Ok(v) => v["type"].as_str().unwrap_or("message").to_string(),
        Err(_) => "message".to_string(),
    };
    Event::default().event(name).json_data(ev).unwrap_or_else(|_| Event::default().data("{}"))
}

/// Gives a caller without a client id one, so a mirror can tell its own echo apart.
fn normalized_client(client: Option<crate::bus::ClientInfo>) -> crate::bus::ClientInfo {
    let mut c = client.unwrap_or_default();
    if c.id.is_empty() {
        c.id = format!("anon-{:x}", rand::random::<u64>());
    }
    c
}

/// `POST /api/v1/chat/stream` — same request body as `/api/v1/chat`, but answers with a
/// Server-Sent Events stream of `ChatEvent`s (event name = the event's `type`), ending with
/// exactly one `done` or `error` event. The caller is attached to the conversation's bus for
/// the life of the stream, so other attached clients see the same events.
async fn v1_chat_stream(
    State(core): State<Arc<Core>>,
    Json(req): Json<V1ChatRequest>,
) -> axum::response::sse::Sse<impl futures_util::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>> {
    use axum::response::sse::{KeepAlive, Sse};
    use crate::events::{ChatEvent, EventSink};

    let session_id = req.session_id.clone().unwrap_or_else(|| core.new_session());
    let req_detach = req.detach;
    let client = normalized_client(req.client.clone());
    let (rx, sub) = core.bus().subscribe(&session_id, client.clone());
    let sink = EventSink { bus: core.bus().clone(), session_id: session_id.clone(), client_id: client.id.clone() };
    sink.send(ChatEvent::Started { session_id: session_id.clone() });
    let task = {
        let core = core.clone();
        let session_id = session_id.clone();
        let session_id_done = session_id.clone();
        tokio::spawn(async move {
            let result = core.send_message_streaming(&session_id, req.message, true, req.images, Some(sink.clone())).await;
            if let Ok(turn) = &result {
                if core.auto_audit_all_local() && !matches!(turn.via.as_str(), "claude" | "system") {
                    let (core, sid) = (core.clone(), session_id.clone());
                    tokio::spawn(async move {
                        let n = core.get_history_in_session(&sid).await.len();
                        if n > 0 {
                            let _ = core.audit_turn(&sid, n - 1, true).await;
                        }
                    });
                }
            }
            // Nobody watching (the phone left the page, the app was closed)? Tell the phone.
            if core.bus().subscriber_count(&session_id) == 0 && crate::push::configured() {
                let payload = match &result {
                    Ok(turn) if turn.via != "system" => {
                        let body = if core.last_turn_private(&session_id) { "Loki replied (kept local, so the text isn't shown here).".to_string() } else { crate::push::preview(&turn.reply) };
                        Some(serde_json::json!({ "title": "Loki replied", "body": body, "url": format!("/?session={session_id}"), "tag": session_id }))
                    }
                    Err(e) => Some(serde_json::json!({ "title": "Loki hit a problem", "body": e.to_string().chars().take(120).collect::<String>(), "url": format!("/?session={session_id}"), "tag": session_id })),
                    _ => None,
                };
                if let Some(p) = payload {
                    crate::push::send_in_background(p);
                }
            }
            let terminal = match result {
                Ok(turn) => ChatEvent::Done {
                    in_routine_mode: core.is_in_routine_mode(&session_id),
                    force_local: core.is_force_local(&session_id),
                    session_id,
                    reply: turn.reply,
                    via: turn.via,
                    note: turn.note,
                },
                Err(e) => ChatEvent::Error { message: e.to_string() },
            };
            sink.send(terminal);
            RUNNING_TURNS.lock().unwrap().remove(&session_id_done);
        })
    };
    // A resend must not orphan a still-running older turn (it could never be cancelled).
    if let Some(old) = RUNNING_TURNS.lock().unwrap().insert(session_id.clone(), task.abort_handle()) {
        old.abort();
    }

    // The requester's stream ends with its own turn's terminal event; `sub` keeps it attached.
    let stream = futures_util::stream::unfold((rx, AbortOnDrop(if req_detach { None } else { Some(task.abort_handle()) }), sub, false), |(mut rx, guard, sub, finished)| async move {
        if finished {
            return None;
        }
        let ev = rx.recv().await?;
        let terminal = matches!(ev, ChatEvent::Done { .. } | ChatEvent::Error { .. });
        Some((Ok(sse_event(&ev)), (rx, guard, sub, terminal)))
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

#[derive(Deserialize)]
struct CancelRequest {
    session_id: String,
}

/// `POST /api/v1/chat/cancel` — stop the running streamed turn of a session (the Stop button).
async fn v1_chat_cancel(Json(req): Json<CancelRequest>) -> StatusCode {
    match RUNNING_TURNS.lock().unwrap().remove(&req.session_id) {
        Some(h) => {
            h.abort();
            StatusCode::OK
        }
        None => StatusCode::NOT_FOUND,
    }
}

#[derive(Deserialize)]
struct WatchQuery {
    session_id: String,
    #[serde(default)]
    client_id: String,
    #[serde(default)]
    kind: String,
    /// Comma-separated capabilities, e.g. `device_fs`.
    #[serde(default)]
    caps: String,
}

/// `GET /api/v1/sessions/events?session_id=…[&client_id=…&kind=…&caps=device_fs]` — attach to a
/// conversation without sending anything: you receive every event of every turn (started by
/// any client) plus approval/device requests. Never ends on its own; the first event is
/// `hello`. Fetch `/api/v1/history` first for what already happened.
async fn v1_watch(
    State(core): State<Arc<Core>>,
    axum::extract::Query(q): axum::extract::Query<WatchQuery>,
) -> axum::response::sse::Sse<impl futures_util::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>> {
    use axum::response::sse::{Event, KeepAlive, Sse};

    let client = normalized_client(Some(crate::bus::ClientInfo {
        id: q.client_id,
        kind: q.kind,
        capabilities: q.caps.split(',').map(str::trim).filter(|c| !c.is_empty()).map(String::from).collect(),
    }));
    let (rx, sub) = core.bus().subscribe(&q.session_id, client);
    let hello = Event::default()
        .event("hello")
        .json_data(serde_json::json!({ "type": "hello", "session_id": q.session_id, "attached": core.bus().attached(&q.session_id) }))
        .unwrap_or_else(|_| Event::default().data("{}"));
    let first = futures_util::stream::once(async move { Ok::<_, std::convert::Infallible>(hello) });
    // Approvals still waiting for an answer are shown again to whoever attaches (the phone coming
    // back to the app), so a prompt never silently disappears.
    let replay: Vec<Result<Event, std::convert::Infallible>> = core.bridge().pending_events(&q.session_id).iter().map(|ev| Ok(sse_event(ev))).collect();
    let replay = futures_util::stream::iter(replay);
    let rest = futures_util::stream::unfold((rx, sub), |(mut rx, sub)| async move {
        let ev = rx.recv().await?;
        Some((Ok(sse_event(&ev)), (rx, sub)))
    });
    use futures_util::StreamExt;
    Sse::new(first.chain(replay).chain(rest)).keep_alive(KeepAlive::default())
}

#[derive(Deserialize)]
struct BridgeRespondRequest {
    id: String,
    value: serde_json::Value,
}

/// The client's answer to a `permission_request` / `device_request` event.
async fn bridge_respond(State(core): State<Arc<Core>>, Json(req): Json<BridgeRespondRequest>) -> StatusCode {
    if core.bridge().respond(&req.id, req.value) {
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND // unknown/expired request — the turn already moved on
    }
}

#[derive(Deserialize)]
struct BridgeDeviceRequest {
    session_id: String,
    name: String,
    #[serde(default)]
    input: serde_json::Value,
}

/// Called by `loki-bridge-mcp` (the helper the Claude CLI is spawned with) when Claude uses a
/// `device_*` tool. Asks the session's client to run it, prompting first for writes/commands.
async fn bridge_device(State(core): State<Arc<Core>>, Json(req): Json<BridgeDeviceRequest>) -> Json<serde_json::Value> {
    match core.bridge().device_call(&req.session_id, &req.name, &req.input, true).await {
        Ok(content) => Json(serde_json::json!({ "content": content, "is_error": false })),
        Err(e) => Json(serde_json::json!({ "content": e, "is_error": true })),
    }
}

#[derive(Deserialize)]
struct BridgePermissionRequest {
    session_id: String,
    tool_name: String,
    #[serde(default)]
    input: serde_json::Value,
}

/// Backs Claude Code's `--permission-prompt-tool`: any tool outside the headless allowlist
/// lands here, the user is asked, and the reply is the allow/deny JSON the CLI expects.
async fn bridge_permission(State(core): State<Arc<Core>>, Json(req): Json<BridgePermissionRequest>) -> Json<serde_json::Value> {
    let allow = core.bridge().request_permission(&req.session_id, &req.tool_name, &req.input).await;
    Json(crate::bridge::permission_decision(allow, &req.input))
}

/// `GET /api/v1/sessions` — conversations for the session picker, newest first.
async fn v1_sessions(State(core): State<Arc<Core>>) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "sessions": core.list_sessions().await }))
}

#[derive(Deserialize)]
struct SessionRenameRequest {
    session_id: String,
    title: String,
}

/// `POST /api/v1/sessions/rename` — custom conversation title.
async fn v1_session_rename(State(core): State<Arc<Core>>, Json(req): Json<SessionRenameRequest>) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    core.rename_session(&req.session_id, &req.title).map(|t| Json(serde_json::json!({ "title": t }))).map_err(|e| (StatusCode::BAD_REQUEST, e))
}

#[derive(Deserialize)]
struct FeedbackRequest {
    session_id: String,
    index: usize,
    rating: String,
}

/// `POST /api/v1/feedback` — thumbs up/down on an assistant message. A downvote starts a Claude
/// audit in the background when auto-audit is on (the setting is `downvotes` by default).
async fn v1_feedback(State(core): State<Arc<Core>>, Json(req): Json<FeedbackRequest>) -> Result<StatusCode, (StatusCode, String)> {
    core.rate_turn(&req.session_id, req.index, &req.rating).map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    if req.rating == "down" && core.auto_audit_on_downvote() {
        let (core, sid, idx) = (core.clone(), req.session_id.clone(), req.index);
        tokio::spawn(async move {
            let _ = core.audit_turn(&sid, idx, true).await;
        });
    }
    Ok(StatusCode::OK)
}

#[derive(Deserialize)]
struct AuditRequest {
    session_id: String,
    index: usize,
}

/// `POST /api/v1/audit` — Claude audits one assistant message (waits for the result).
async fn v1_audit(State(core): State<Arc<Core>>, Json(req): Json<AuditRequest>) -> Result<Json<crate::audit::AuditRecord>, (StatusCode, String)> {
    core.audit_turn(&req.session_id, req.index, false).await.map(Json).map_err(|e| (StatusCode::BAD_REQUEST, e))
}

async fn v1_commands() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "commands": crate::commands::list() }))
}

async fn v1_lessons() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "lessons": crate::lessons::list(), "settings": crate::lessons::settings() }))
}

#[derive(Deserialize)]
struct LessonUpdate {
    id: String,
    action: String,
    #[serde(default)]
    text: Option<String>,
}

async fn v1_lessons_update(State(core): State<Arc<Core>>, Json(req): Json<LessonUpdate>) -> Result<StatusCode, (StatusCode, String)> {
    if req.action == "approve" {
        core.apply_routine_edit(&req.id).await.map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    }
    crate::lessons::update(&req.id, &req.action, req.text.as_deref()).map(|_| StatusCode::OK).map_err(|e| (StatusCode::BAD_REQUEST, e))
}

#[derive(Deserialize)]
struct AuditSettingsRequest {
    auto: String,
}

async fn v1_audit_settings(Json(req): Json<AuditSettingsRequest>) -> Result<StatusCode, (StatusCode, String)> {
    crate::lessons::set_auto(&req.auto).map(|_| StatusCode::OK).map_err(|e| (StatusCode::BAD_REQUEST, e))
}

/// `GET /api/v1/needs-you` — the ranked "Needs you" list (see `needs_you.rs`).
async fn v1_needs_you(State(core): State<Arc<Core>>) -> Json<serde_json::Value> {
    Json(core.needs_you().await)
}

/// `POST /api/v1/mail/review/start` — begin the local-only inbox review (the local model reads a sample of unread mail).
async fn v1_mail_review_start(State(core): State<Arc<Core>>) -> Result<StatusCode, (StatusCode, String)> {
    core.start_mail_review().map(|_| StatusCode::ACCEPTED).map_err(|e| (StatusCode::CONFLICT, e))
}

/// `GET /api/v1/mail/review/status` — progress and estimated counts per class. Counts only.
async fn v1_mail_review_status(State(core): State<Arc<Core>>) -> Json<serde_json::Value> {
    Json(core.mail_review_status())
}

/// `GET /api/v1/mail/review` — the review as a page (contains senders and subjects: the user's screens only).
async fn v1_mail_review(State(core): State<Arc<Core>>) -> Json<crate::needs_you::Briefing> {
    Json(core.mail_review_page())
}

/// `GET /api/v1/mail/plan` — the drafted cleanup plan as plain text.
/// `POST /api/v1/finance/sync` — asks the Ledger to sync now, then re-reads it.
async fn v1_finance_sync() -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    crate::homelab::finance_sync_now().await.map(|m| Json(serde_json::json!({ "message": m }))).map_err(|e| (StatusCode::BAD_GATEWAY, e))
}

#[derive(Deserialize)]
struct EnsureRoutinesRequest {
    path: String,
}

/// `POST /api/v1/tasks/ensure-routines` — adds the day's (or sprint's) routine to-dos to its note if they're missing.
async fn v1_ensure_routines(State(core): State<Arc<Core>>, Json(req): Json<EnsureRoutinesRequest>) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    core.ensure_routines(&req.path).await.map(|added| Json(serde_json::json!({ "added": added }))).map_err(|e| (StatusCode::BAD_REQUEST, e))
}

#[derive(Deserialize)]
struct TaskMarkRequest {
    path: String,
    text: String,
    #[serde(default)]
    section: String,
    /// true = tick, false = reopen, absent = comment only
    #[serde(default)]
    done: Option<bool>,
    #[serde(default)]
    comment: String,
}

/// `POST /api/v1/tasks/mark` — tick, reopen or comment on one checkbox task in a daily or sprint note.
async fn v1_tasks_mark(State(core): State<Arc<Core>>, Json(req): Json<TaskMarkRequest>) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    core.mark_task(&req.path, &req.text, &req.section, req.done, &req.comment).await.map(|m| Json(serde_json::json!({ "message": m }))).map_err(|e| (StatusCode::BAD_REQUEST, e))
}

#[derive(Deserialize)]
struct BacklogPathQuery {
    path: String,
}

/// `GET /api/v1/backlog/item?path=` — the whole backlog note.
async fn v1_backlog_item(State(core): State<Arc<Core>>, axum::extract::Query(q): axum::extract::Query<BacklogPathQuery>) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    core.backlog_item(&q.path).await.map(|c| Json(serde_json::json!({ "content": c }))).map_err(|e| (StatusCode::BAD_REQUEST, e))
}

#[derive(Deserialize)]
struct BacklogEditRequest {
    path: String,
    action: String,
    #[serde(default)]
    content: String,
    #[serde(default)]
    base: String,
    #[serde(default)]
    comment: String,
    #[serde(default)]
    status: String,
}

/// `POST /api/v1/backlog/item` — save, comment on, set the status of, or close one backlog item.
async fn v1_backlog_edit(State(core): State<Arc<Core>>, Json(r): Json<BacklogEditRequest>) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    core.backlog_edit(&r.path, &r.action, &r.content, &r.base, &r.comment, &r.status).await.map(|m| Json(serde_json::json!({ "message": m }))).map_err(|e| (StatusCode::BAD_REQUEST, e))
}

#[derive(Deserialize)]
struct NotesFindQuery {
    q: String,
}

/// `GET /api/v1/notes/find?q=` — vault notes matching a search, for choosing where a note is added.
async fn v1_notes_find(State(core): State<Arc<Core>>, axum::extract::Query(q): axum::extract::Query<NotesFindQuery>) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    core.notes_find(&q.q).await.map(|p| Json(serde_json::json!({ "paths": p }))).map_err(|e| (StatusCode::BAD_REQUEST, e))
}

#[derive(Deserialize)]
struct NotesPlanRequest {
    #[serde(default)]
    title: String,
    text: String,
    #[serde(default)]
    mode: String,
    #[serde(default)]
    targets: Vec<String>,
}

/// `POST /api/v1/notes/plan` — the local model proposes how to file a note. Writes nothing.
async fn v1_notes_plan(State(core): State<Arc<Core>>, Json(r): Json<NotesPlanRequest>) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    core.notes_plan(&r.title, &r.text, &r.mode, &r.targets).await.map(Json).map_err(|e| (StatusCode::BAD_REQUEST, e))
}

#[derive(Deserialize)]
struct NotesApplyRequest {
    proposals: Vec<crate::notes_ai::NoteProposal>,
}

/// `POST /api/v1/notes/apply` — writes only what the user approved.
async fn v1_notes_apply(State(core): State<Arc<Core>>, Json(r): Json<NotesApplyRequest>) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    core.notes_apply(r.proposals).await.map(|x| Json(serde_json::json!({ "results": x }))).map_err(|e| (StatusCode::BAD_REQUEST, e))
}

#[derive(Deserialize)]
struct ExtractRequest {
    source: String,
    #[serde(default)]
    account: String,
    #[serde(default)]
    id: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    label: String,
}

/// `POST /api/v1/extract` — the local model proposes action items and events from a message or text. Writes nothing.
async fn v1_extract(State(core): State<Arc<Core>>, Json(req): Json<ExtractRequest>) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    core.extract_run(&req.source, &req.account, &req.id, &req.text, &req.label).await.map(Json).map_err(|e| (StatusCode::BAD_REQUEST, e))
}

#[derive(Deserialize)]
struct ExtractApplyRequest {
    key: String,
    #[serde(default)]
    label: String,
    #[serde(default)]
    link: String,
    proposals: Vec<crate::extract::Proposal>,
}

/// `POST /api/v1/extract/apply` — writes only what the user approved.
async fn v1_extract_apply(State(core): State<Arc<Core>>, Json(req): Json<ExtractApplyRequest>) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    core.extract_apply(&req.key, &req.label, &req.link, req.proposals).await.map(|r| Json(serde_json::json!({ "results": r }))).map_err(|e| (StatusCode::BAD_REQUEST, e))
}

/// Only the leads app's own API paths the dashboard needs; everything else is refused before anything is forwarded.
pub fn leads_path_ok(rest: &str) -> bool {
    let seg: Vec<&str> = rest.split('/').collect();
    let num = |s: &str| !s.is_empty() && s.len() < 10 && s.chars().all(|c| c.is_ascii_digit());
    match seg.as_slice() {
        ["items" | "facets" | "families" | "stats" | "export" | "refresh" | "brief" | "trends"] => true,
        ["items", id] => num(id),
        ["items", id, "draft" | "steps"] => num(id),
        ["families", id, "steps"] => num(id),
        _ => false,
    }
}

/// `/api/v1/leads/*` — forwards to the leads app on the Loki server (the leads app's API) so every Loki client can use it. Post text
/// and replies pass through untouched; the dashboard renders them as plain text.
async fn v1_leads_proxy(method: Method, Path(rest): Path<String>, RawQuery(query): RawQuery, body: Bytes) -> Result<axum::response::Response, (StatusCode, String)> {
    if !leads_path_ok(&rest) || !matches!(method, Method::GET | Method::POST | Method::PATCH) {
        return Err((StatusCode::NOT_FOUND, "not a leads endpoint".into()));
    }
    let url = format!("{}/api/{rest}{}", crate::needs_you::leads_url(), query.map(|q| format!("?{q}")).unwrap_or_default());
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(150)).build().map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let mut req = client.request(method, &url);
    if !body.is_empty() {
        req = req.header("content-type", "application/json").body(body.to_vec());
    }
    let up = req.send().await.map_err(|e| (StatusCode::BAD_GATEWAY, format!("the leads app didn't answer: {e}")))?;
    let status = StatusCode::from_u16(up.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let mut headers = HeaderMap::new();
    for h in ["content-type", "content-disposition"] {
        if let Some(v) = up.headers().get(h) {
            if let Ok(v) = axum::http::HeaderValue::from_bytes(v.as_bytes()) {
                headers.insert(axum::http::HeaderName::from_static(h), v);
            }
        }
    }
    let bytes = up.bytes().await.map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
    Ok((status, headers, bytes).into_response())
}

/// `GET /api/v1/news/geo?date=index|YYYY-MM-DD` — the news globe's geocoded events (public news, read from the home server).
async fn v1_news_geo(Query(params): Query<HashMap<String, String>>) -> Result<axum::response::Response, StatusCode> {
    let name = params.get("date").map(String::as_str).unwrap_or("index");
    let text = crate::homelab::news_geo(name).await.ok_or(StatusCode::NOT_FOUND)?;
    Ok(([(axum::http::header::CONTENT_TYPE, "application/json")], text).into_response())
}

/// `POST /api/v1/news/render` — turn today's digest note into the `the news page` page. The script lives on this host
/// (the Loki server), so the n8n news workflow calls this instead of needing shell access of its own.
async fn v1_news_render() -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let run = tokio::process::Command::new("python3").arg(std::env::var("LOKI_NEWS_RENDER").map_err(|_| (StatusCode::NOT_IMPLEMENTED, "news rendering is not configured (LOKI_NEWS_RENDER)".to_string()))?).kill_on_drop(true).output();
    let out = tokio::time::timeout(std::time::Duration::from_secs(120), run)
        .await
        .map_err(|_| (StatusCode::GATEWAY_TIMEOUT, "news-to-html.py took over 2 minutes".to_string()))?
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let tail = |b: &[u8]| String::from_utf8_lossy(b).lines().rev().take(6).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n");
    if !out.status.success() {
        return Err((StatusCode::BAD_GATEWAY, tail(&out.stderr)));
    }
    Ok(Json(serde_json::json!({ "ok": true, "log": tail(&out.stdout) })))
}

/// `GET /api/v1/routines` — the routines in the profile: what each tab's buttons run.
async fn v1_routines() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "routines": crate::profile::get().routines }))
}

#[derive(Deserialize)]
struct WorkflowRun {
    slug: String,
    #[serde(default)]
    force: bool,
}

/// `POST /api/v1/workflows/run` — the Run button on the Workflows page. Only the listed workflows can be started, through
/// their header-protected n8n webhook; the key stays on this host.
async fn v1_workflow_run(Json(req): Json<WorkflowRun>) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if !crate::profile::get().is_runnable_workflow(&req.slug) {
        return Err((StatusCode::NOT_FOUND, format!("no runnable workflow called {}", req.slug)));
    }
    let key_path = std::env::var("HOME").map(|h| format!("{h}/.config/loki/n8n_run_key")).unwrap_or_default();
    let key = std::fs::read_to_string(&key_path).map_err(|_| (StatusCode::SERVICE_UNAVAILABLE, "the n8n run key is not set up on this host".to_string()))?;
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(20)).build().map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let res = client
        .post(format!("http://127.0.0.1:5678/webhook/run-{}", req.slug))
        .header("X-Run-Key", key.trim())
        .json(&serde_json::json!({ "force": req.force }))
        .send()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("n8n did not answer: {e}")))?;
    if !res.status().is_success() {
        return Err((StatusCode::BAD_GATEWAY, format!("n8n said {} (is the workflow published?)", res.status())));
    }
    Ok(Json(serde_json::json!({ "started": true })))
}

/// One local-model call with a Fabric pattern's prompt as the system message: thinking off, capped output, hard timeout.
async fn local_with_pattern(client: &reqwest::Client, input: &str, pattern: &str, model: &str, format: Option<serde_json::Value>) -> Result<String, String> {
    let mut messages = Vec::new();
    if pattern != "raw_query" {
        if pattern.is_empty() || !pattern.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err("bad pattern name".into());
        }
        let p: serde_json::Value = client.get(format!("http://127.0.0.1:8080/patterns/{pattern}")).send().await.map_err(|e| format!("Fabric pattern lookup failed: {e}"))?.json().await.map_err(|e| format!("Fabric pattern lookup failed: {e}"))?;
        let text = p["Pattern"].as_str().unwrap_or("").trim();
        if text.is_empty() {
            return Err(format!("Fabric has no pattern called {pattern}"));
        }
        if text.contains("{{input}}") {
            // Fabric fills this placeholder with the input and sends the result as the whole prompt.
            messages.push(serde_json::json!({ "role": "user", "content": text.replace("{{input}}", input) }));
            return local_chat(client, model, messages, format).await;
        }
        messages.push(serde_json::json!({ "role": "system", "content": text }));
    }
    messages.push(serde_json::json!({ "role": "user", "content": input }));
    local_chat(client, model, messages, format).await
}

/// The local model the gateway uses: the caller's choice, else `LOKI_GATEWAY_MODEL`, else the `[llm]` model of config.toml.
fn gateway_model(asked: Option<&str>) -> Option<String> {
    if let Some(m) = asked.map(str::trim).filter(|m| !m.is_empty()) {
        return Some(m.to_string());
    }
    if let Ok(m) = std::env::var("LOKI_GATEWAY_MODEL") {
        if !m.trim().is_empty() {
            return Some(m.trim().to_string());
        }
    }
    match crate::config::load().map(|c| c.llm) {
        Ok(crate::config::LlmBackend::Ollama { model, .. }) => Some(model),
        _ => None,
    }
}

/// Where the local model runs: `LOKI_OLLAMA_URL`, else the `[llm]` host and port of config.toml, else this machine.
fn ollama_base() -> &'static str {
    static BASE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    BASE.get_or_init(|| {
        if let Ok(u) = std::env::var("LOKI_OLLAMA_URL") {
            return u.trim_end_matches('/').to_string();
        }
        match crate::config::load().map(|c| c.llm) {
            Ok(crate::config::LlmBackend::Ollama { host, port, .. }) => format!("http://{host}:{port}"),
            _ => "http://127.0.0.1:11434".to_string(),
        }
    })
}

async fn local_chat(client: &reqwest::Client, model: &str, messages: Vec<serde_json::Value>, format: Option<serde_json::Value>) -> Result<String, String> {
    use futures_util::StreamExt;
    // The same context size Loki's own chat uses: Ollama reloads the whole model when a request asks for a different one, and the gateway
    // alternates with chat. Streamed, so that giving up (the timeout below) closes the connection and Ollama stops generating; a
    // non-streamed request that is abandoned keeps running on the GPU and starves the next one.
    let num_ctx = crate::context_stats::get().num_ctx;
    let mut body = serde_json::json!({ "model": model, "stream": true, "think": false, "options": { "temperature": 0.2, "num_ctx": num_ctx, "num_predict": 4096 }, "messages": messages });
    let wants_json = format.is_some();
    if let Some(f) = format {
        body["format"] = f;
    }
    let run = async {
        let res = client.post(format!("{}/api/chat", ollama_base())).json(&body).send().await.map_err(|e| format!("the local model did not answer: {e}"))?;
        if !res.status().is_success() {
            return Err(format!("the local model said {}", res.status()));
        }
        let (mut stream, mut buf, mut text) = (res.bytes_stream(), String::new(), String::new());
        'read: while let Some(chunk) = stream.next().await {
            buf.push_str(&String::from_utf8_lossy(&chunk.map_err(|e| format!("the local model stopped mid-answer: {e}"))?));
            while let Some(i) = buf.find('\n') {
                let line: String = buf.drain(..=i).collect();
                let Ok(v) = serde_json::from_str::<serde_json::Value>(line.trim()) else { continue };
                if let Some(err) = v["error"].as_str() {
                    return Err(format!("the local model reported: {err}"));
                }
                text.push_str(v["message"]["content"].as_str().unwrap_or(""));
                if v["done"].as_bool().unwrap_or(false) {
                    break 'read;
                }
            }
        }
        Ok(text)
    };
    let text = tokio::time::timeout(std::time::Duration::from_secs(280), run).await.map_err(|_| "the local model took longer than 280 seconds, so the call was cancelled".to_string())??;
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err("the local model returned nothing".into());
    }
    if wants_json && serde_json::from_str::<serde_json::Value>(&text).is_err() {
        return Err("the local model did not return valid JSON".into());
    }
    Ok(text)
}

#[derive(Deserialize)]
struct AiFabricRequest {
    input: String,
    #[serde(default)]
    pattern: Option<String>,
    /// Both optional: left out, Fabric uses its own default vendor and model (the one place that is switched).
    #[serde(default)]
    vendor: Option<String>,
    #[serde(default)]
    model: Option<String>,
    /// Mail or other private text: always the local model, whatever vendor, model or Fabric default says.
    #[serde(default)]
    private: bool,
    /// Ask a local model for JSON (any object).
    #[serde(default)]
    json: bool,
    /// Ask a local model for JSON that follows this JSON Schema. Overrides `json`. Unlike Ollama's plain `format: "json"` (which this
    /// model ignores: it still answers in code fences and with unescaped quotes), a schema is enforced while the text is generated.
    #[serde(default)]
    schema: Option<serde_json::Value>,
}

/// The `format` to send Ollama: the caller's schema, or a bare "any object" schema when only `json` was asked for.
fn json_format(json: bool, schema: Option<&serde_json::Value>) -> Option<serde_json::Value> {
    match schema {
        Some(s) if s.is_object() => Some(s.clone()),
        _ if json => Some(serde_json::json!({ "type": "object" })),
        _ => None,
    }
}

/// `POST /api/v1/ai/fabric` — one Fabric call for the n8n workflows (the local model on the local model by default; Gemini only when asked for). Fabric streams its answer and, after a
/// Gemini error (a usage limit, say), leaves the stream open forever; n8n's HTTP node then just reports "stream aborted". This reads
/// the stream itself, stops at the first `complete` or `error` event, and says plainly whether the failure was a usage limit.
async fn v1_ai_fabric(Json(req): Json<AiFabricRequest>) -> Json<serde_json::Value> {
    use futures_util::StreamExt;
    // One call at a time: a burst of parallel requests is what hangs the model runner on the model server, so callers queue here instead.
    static SLOT: std::sync::OnceLock<tokio::sync::Semaphore> = std::sync::OnceLock::new();
    let _slot = SLOT.get_or_init(|| tokio::sync::Semaphore::new(1)).acquire().await;
    let mut engine = String::from("Fabric default");
    let fail = |error: String, limit: bool| Json(serde_json::json!({ "ok": false, "text": "", "limit": limit, "error": error }));
    let client = match reqwest::Client::builder().timeout(std::time::Duration::from_secs(170)).build() {
        Ok(c) => c,
        Err(e) => return fail(e.to_string(), false),
    };
    let pattern_name = req.pattern.clone().unwrap_or_else(|| "raw_query".into());
    // Local (the local model) is the standard; Gemini is used only when a caller asks for it by name.
    if req.private || req.vendor.as_deref() != Some("Gemini") {
        // Fabric stays the pattern library, but the model call is ours: Fabric cannot switch thinking off or cap the output, and it keeps
        // its upstream call running after a caller gives up, which is what left the model busy for half an hour.
        let model = match gateway_model(req.model.as_deref()) {
            Some(m) => m,
            None => return fail("no local model is configured: set [llm] in config.toml or LOKI_GATEWAY_MODEL".into(), false),
        };
        return match local_with_pattern(&client, &req.input, &pattern_name, &model, json_format(req.json, req.schema.as_ref())).await {
            Ok(text) => Json(serde_json::json!({ "ok": true, "text": text, "limit": false, "engine": format!("Ollama {model} (Fabric pattern {pattern_name}, thinking off)"), "error": "" })),
            Err(e) => fail(e, false),
        };
    }
    let mut prompt = serde_json::json!({ "userInput": req.input, "patternName": pattern_name });
    if let Some(v) = req.vendor.filter(|v| !v.is_empty()) {
        prompt["vendor"] = v.into();
    }
    if let Some(m) = req.model.filter(|m| !m.is_empty()) {
        prompt["model"] = m.into();
    }
    if req.private {
        // Tier 3 (mail): never a cloud model. Keep a local model if the caller named one, else the standard local model.
        prompt["vendor"] = "Ollama".into();
        if prompt["model"].as_str().is_none_or(|m| m.is_empty()) {
            prompt["model"] = gateway_model(None).unwrap_or_default().into();
        }
    }
    if let (Some(v), Some(m)) = (prompt["vendor"].as_str(), prompt["model"].as_str()) {
        engine = format!("{v} {m}");
    } else if let Some(v) = prompt["vendor"].as_str() {
        engine = v.to_string();
    }
    let body = serde_json::json!({ "prompts": [prompt], "language": "en" });
    let res = match client.post("http://127.0.0.1:8080/chat").json(&body).send().await {
        Ok(r) => r,
        Err(e) => return fail(format!("Fabric did not answer: {e}"), false),
    };
    if !res.status().is_success() {
        let code = res.status();
        let text = res.text().await.unwrap_or_default();
        return fail(format!("Fabric said {code}: {}", text.chars().take(300).collect::<String>()), false);
    }
    let (mut buf, mut text, mut stream) = (String::new(), String::new(), res.bytes_stream());
    let is_limit = |m: &str| { let l = m.to_lowercase(); ["429", "quota", "rate limit", "rate-limit", "resource_exhausted", "resource exhausted", "too many requests"].iter().any(|k| l.contains(k)) };
    while let Some(chunk) = stream.next().await {
        let Ok(chunk) = chunk else { break };
        buf.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(i) = buf.find('\n') {
            let line: String = buf.drain(..=i).collect();
            let Some(data) = line.trim().strip_prefix("data: ") else { continue };
            let Ok(ev) = serde_json::from_str::<serde_json::Value>(data) else { continue };
            match ev["type"].as_str() {
                Some("content") => text.push_str(ev["content"].as_str().unwrap_or("")),
                Some("error") => {
                    let m = ev["content"].as_str().unwrap_or("unknown error").to_string();
                    return fail(m.chars().take(400).collect(), is_limit(&m));
                }
                Some("complete") => return Json(serde_json::json!({ "ok": !text.trim().is_empty(), "text": text, "limit": false, "engine": engine, "error": if text.trim().is_empty() { "empty reply" } else { "" } })),
                _ => {}
            }
        }
    }
    Json(serde_json::json!({ "ok": !text.trim().is_empty(), "text": text, "limit": false, "engine": engine, "error": if text.trim().is_empty() { "stream ended with no content" } else { "" } }))
}

/// `GET /api/v1/mail/newsletters?bucket=news|market&limit=` — recent messages from the senders tagged for that bucket, for the
/// morning workflows. The text is for a local model only; the n8n workflows summarize it on the local model and nowhere else.
async fn v1_newsletters(Query(params): Query<HashMap<String, String>>) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let bucket = params.get("bucket").map(String::as_str).unwrap_or("news");
    let limit = params.get("limit").and_then(|l| l.parse().ok()).unwrap_or(12);
    crate::newsletters::pending(bucket, limit).await.map(Json).map_err(|e| (StatusCode::BAD_REQUEST, e))
}

#[derive(Deserialize)]
struct DigestedRequest {
    ids: Vec<String>,
}

/// `POST /api/v1/mail/newsletters/digested` — the workflow reports which messages made it into a digest.
async fn v1_newsletters_digested(Json(req): Json<DigestedRequest>) -> StatusCode {
    crate::newsletters::mark_digested(&req.ids);
    StatusCode::NO_CONTENT
}

/// `GET /api/v1/mail/newsletters/senders` — who writes to the inbox (names and counts only) and which bucket each is tagged for.
async fn v1_newsletter_senders() -> Json<serde_json::Value> {
    Json(crate::newsletters::senders_json())
}

#[derive(Deserialize)]
struct SenderSetRequest {
    address: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    bucket: String,
}

/// `POST /api/v1/mail/newsletters/senders` — tag a sender `news`, `market` or `ignore` (empty bucket clears it).
async fn v1_newsletter_sender_set(Json(req): Json<SenderSetRequest>) -> Result<StatusCode, (StatusCode, String)> {
    crate::newsletters::set_bucket(&req.address, &req.name, &req.bucket).map(|_| StatusCode::NO_CONTENT).map_err(|e| (StatusCode::BAD_REQUEST, e))
}

/// `GET /api/v1/mail/inbox` — every inbox message with its sorting, for the dashboard (the signed-in user's own mail).
async fn v1_mail_inbox() -> Json<serde_json::Value> {
    Json(crate::mail_inbox::page_json())
}

/// `GET /api/v1/mail/inbox/status` — counts only.
async fn v1_mail_inbox_status() -> Json<serde_json::Value> {
    Json(crate::mail_inbox::status_json())
}

#[derive(Deserialize)]
struct InboxScanRequest {
    #[serde(default)]
    matrix: bool,
}

async fn v1_mail_inbox_scan(State(core): State<Arc<Core>>, Json(req): Json<InboxScanRequest>) -> Result<StatusCode, (StatusCode, String)> {
    let Some(p) = core.local_llm() else { return Err((StatusCode::SERVICE_UNAVAILABLE, "no local model is available".into())) };
    tokio::spawn(crate::mail_inbox::scan(p, req.matrix));
    Ok(StatusCode::ACCEPTED)
}

/// `GET /api/v1/mail/inbox/message?account=&id=` — one message's text for the in-app viewer.
async fn v1_mail_inbox_message(Query(params): Query<HashMap<String, String>>) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (Some(a), Some(i)) = (params.get("account"), params.get("id")) else { return Err((StatusCode::BAD_REQUEST, "account and id are required".into())) };
    crate::mail::read_message(a, i).await.map(Json).map_err(|e| (StatusCode::BAD_REQUEST, e))
}

#[derive(Deserialize)]
struct InboxActRequest {
    account: String,
    ids: Vec<String>,
    action: String,
}

async fn v1_mail_inbox_act(Json(req): Json<InboxActRequest>) -> Result<String, (StatusCode, String)> {
    crate::mail_inbox::act(&req.account, &req.ids, &req.action).await.map_err(|e| (StatusCode::BAD_REQUEST, e))
}

#[derive(Deserialize)]
struct InboxDraftRequest {
    account: String,
    id: String,
}

async fn v1_mail_inbox_draft(State(core): State<Arc<Core>>, Json(req): Json<InboxDraftRequest>) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let Some(p) = core.local_llm() else { return Err((StatusCode::SERVICE_UNAVAILABLE, "no local model is available".into())) };
    crate::mail_inbox::draft(p, &req.account, &req.id).await.map(Json).map_err(|e| (StatusCode::BAD_REQUEST, e))
}

async fn v1_improve_run(State(core): State<Arc<Core>>) -> String {
    core.improve_run().await
}

async fn v1_mail_plan(State(core): State<Arc<Core>>) -> String {
    core.mail_plan()
}

/// `GET /api/v1/mail/status` — which mail accounts are authorized and their unread counts. Counts only: no sender,
/// subject or text ever comes out of this endpoint.
async fn v1_mail_status() -> Json<serde_json::Value> {
    let Some(data) = crate::mail::dashboard_data().await else { return Json(serde_json::json!({ "configured": false, "accounts": [] })) };
    let accounts: Vec<serde_json::Value> = data.iter().map(|(email, unread, msgs)| serde_json::json!({ "email": email, "unread": unread, "shown": msgs.len() })).collect();
    let labels: serde_json::Map<String, serde_json::Value> = crate::mail::label_stats().await.into_iter().collect();
    Json(serde_json::json!({ "configured": true, "accounts": accounts, "labels": labels }))
}

/// `GET /api/v1/day?date=YYYY-MM-DD` — one calendar day as a briefing page (from the vault).
async fn v1_day(State(core): State<Arc<Core>>, Query(params): Query<HashMap<String, String>>) -> Result<Json<crate::needs_you::Briefing>, (StatusCode, String)> {
    let date = params.get("date").map(String::as_str).unwrap_or("");
    let ok = date.len() == 10 && date.chars().enumerate().all(|(i, c)| if i == 4 || i == 7 { c == '-' } else { c.is_ascii_digit() });
    if !ok {
        return Err((StatusCode::BAD_REQUEST, "date must be YYYY-MM-DD".into()));
    }
    Ok(Json(core.day_page(date).await))
}

#[derive(Deserialize)]
struct NeedsYouDecision {
    id: String,
    #[serde(default)]
    title: String,
    /// done | defer | not_now | rate
    action: String,
    #[serde(default)]
    days: Option<i64>,
    #[serde(default)]
    reason: String,
    /// up | down | ""
    #[serde(default)]
    rating: String,
}

/// `POST /api/v1/needs-you/decision` — mark an item done, defer it N days, or set it aside for today;
/// optionally with a reason and a thumbs rating (those also go to the vault's findings file).
async fn v1_needs_you_decision(State(core): State<Arc<Core>>, Json(req): Json<NeedsYouDecision>) -> Result<StatusCode, (StatusCode, String)> {
    core.needs_you_decide(&req.id, &req.title, &req.action, req.days, &req.reason, &req.rating).await.map(|_| StatusCode::OK).map_err(|e| (StatusCode::BAD_REQUEST, e))
}

/// `GET /api/v1/context?session_id=…` — what fills this conversation's context window, how close
/// autocompact is, and the live context settings (see `Core::context_report`).
async fn v1_context(State(core): State<Arc<Core>>, Query(params): Query<HashMap<String, String>>) -> Json<serde_json::Value> {
    Json(core.context_report(params.get("session_id").map(String::as_str).unwrap_or("")))
}

/// `POST /api/v1/context/settings` — any of `autocompact`, `history_fraction`, `num_ctx`,
/// `max_turns`; applies from the next turn and is saved across restarts.
async fn v1_context_settings(Json(patch): Json<serde_json::Value>) -> Result<Json<crate::context_stats::ContextSettings>, (StatusCode, String)> {
    crate::context_stats::update(&patch).map(Json).map_err(|e| (StatusCode::BAD_REQUEST, e))
}

#[derive(Deserialize)]
struct TroubleshootRequest {
    session_id: String,
    #[serde(default)]
    note: String,
    #[serde(default)]
    ui: crate::troubleshoot::UiSnapshot,
}

/// `POST /api/v1/troubleshoot` — starts a background Claude review of this conversation plus the
/// client's UI snapshot; poll `GET /api/v1/troubleshoot/{id}`.
async fn v1_troubleshoot(State(core): State<Arc<Core>>, Json(req): Json<TroubleshootRequest>) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let dir = core.build_troubleshoot_bundle(&req.session_id, &req.ui).await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let _ = std::fs::write(dir.join("note.txt"), req.note.chars().take(2000).collect::<String>());
    let id = crate::troubleshoot::new_job(&dir);
    let (core, job, note) = (core.clone(), id.clone(), req.note.clone());
    tokio::spawn(async move {
        match core.run_troubleshoot(&dir, &note).await {
            Ok(text) => {
                let _ = std::fs::write(dir.join("claude-report.md"), &text);
                crate::troubleshoot::finish(&job, true, text)
            }
            Err(e) => crate::troubleshoot::finish(&job, false, e),
        }
    });
    Ok(Json(serde_json::json!({ "id": id })))
}

#[derive(Deserialize)]
struct FixRequest {
    /// The troubleshoot report to act on.
    id: String,
    /// "fix" (stage changes for review) | "apply" (merge + deploy the reviewed changes) | "discard".
    action: String,
    #[serde(default)]
    note: String,
}

/// The machine that runs the "Fix with Claude" helper over SSH, and where its `loki-cli` lives; unset = the feature is off.
fn fix_target() -> Option<(String, String)> {
    Some((std::env::var("LOKI_FIX_HOST").ok().filter(|h| !h.is_empty())?, std::env::var("LOKI_FIX_CLI").ok().filter(|c| !c.is_empty())?))
}

/// `POST /api/v1/fix` — "Fix with Claude" from a client that isn't on the laptop (web, PWA,
/// desktop app). The source tree lives on the laptop, so this runs the laptop's
/// `loki-cli fix | fix-apply | fix-discard <report-id>` over SSH — a fixed command with a
/// validated id and the note passed base64-encoded, never an arbitrary shell string. `fix` only
/// stages changes (isolated worktree); `apply` is a separate, explicit request. Poll the returned
/// job at `GET /api/v1/troubleshoot/{job}`.
async fn v1_fix(Json(req): Json<FixRequest>) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    use base64::Engine as _;
    if req.id.is_empty() || req.id.len() > 40 || !req.id.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err((StatusCode::BAD_REQUEST, "bad report id".into()));
    }
    let sub = match req.action.as_str() {
        "fix" => "fix",
        "apply" => "fix-apply",
        "discard" => "fix-discard",
        _ => return Err((StatusCode::BAD_REQUEST, "action must be fix, apply or discard".into())),
    };
    if crate::troubleshoot::job(&req.id).is_none() {
        return Err((StatusCode::NOT_FOUND, "unknown troubleshoot report".into()));
    }
    let Some((fix_host, fix_cli)) = fix_target() else {
        return Err((StatusCode::NOT_IMPLEMENTED, "the fix helper is not configured (LOKI_FIX_HOST and LOKI_FIX_CLI)".into()));
    };
    let note_b64 = base64::engine::general_purpose::STANDARD.encode(req.note.chars().take(2000).collect::<String>().as_bytes());
    let remote = format!("N=$(echo {note_b64} | base64 -d); {fix_cli} {sub} {} \"$N\"", req.id);
    let job = crate::troubleshoot::new_job(&std::path::PathBuf::new());
    let jid = job.clone();
    tokio::spawn(async move {
        let run = tokio::process::Command::new("ssh").args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=10", &fix_host, &remote]).kill_on_drop(true).output();
        match tokio::time::timeout(std::time::Duration::from_secs(40 * 60), run).await {
            Ok(Ok(out)) => {
                let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if text.is_empty() {
                    crate::troubleshoot::finish(&jid, false, format!("The laptop gave no answer (is it awake?): {}", String::from_utf8_lossy(&out.stderr).trim()));
                } else {
                    let ok = serde_json::from_str::<serde_json::Value>(text.lines().last().unwrap_or("")).map(|v| v["status"] == "done").unwrap_or(false);
                    crate::troubleshoot::finish(&jid, ok, text);
                }
            }
            Ok(Err(e)) => crate::troubleshoot::finish(&jid, false, format!("couldn't reach the laptop: {e}")),
            Err(_) => crate::troubleshoot::finish(&jid, false, "The laptop took too long.".into()),
        }
    });
    Ok(Json(serde_json::json!({ "id": job })))
}

async fn v1_push_status() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "configured": crate::push::configured(), "key": crate::push::public_key(), "devices": crate::push::count() }))
}

#[derive(Deserialize)]
struct PushSubscribeRequest {
    endpoint: String,
    keys: crate::push::Keys,
    #[serde(default)]
    client_id: String,
}

async fn v1_push_subscribe(Json(req): Json<PushSubscribeRequest>) -> Result<StatusCode, (StatusCode, String)> {
    if !crate::push::configured() {
        return Err((StatusCode::SERVICE_UNAVAILABLE, "push isn't set up on the server (run ops/loki-push-setup.sh)".into()));
    }
    if !req.endpoint.starts_with("https://") {
        return Err((StatusCode::BAD_REQUEST, "bad push endpoint".into()));
    }
    crate::push::subscribe(crate::push::Subscription { endpoint: req.endpoint, keys: req.keys, client_id: req.client_id, created: 0 });
    Ok(StatusCode::OK)
}

#[derive(Deserialize)]
struct PushUnsubscribeRequest {
    endpoint: String,
}

async fn v1_push_unsubscribe(Json(req): Json<PushUnsubscribeRequest>) -> StatusCode {
    crate::push::unsubscribe(&req.endpoint);
    StatusCode::OK
}

async fn v1_push_test() -> StatusCode {
    crate::push::send_in_background(serde_json::json!({ "title": "Loki", "body": "Notifications are working on this device.", "url": "/", "tag": "loki-test" }));
    StatusCode::OK
}

async fn v1_speech_status(State(core): State<Arc<Core>>) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "configured": crate::speech::configured(), "voice": core.speech_config().tts_voice }))
}

/// `POST /api/v1/stt` — the raw recording (webm/ogg/mp4/wav) in the body; answers `{ "text": … }`
/// from the speech-to-text model on the model server. Dictation: the text goes into the input box, not straight
/// into the conversation.
async fn v1_stt(State(core): State<Arc<Core>>, headers: axum::http::HeaderMap, body: axum::body::Bytes) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if body.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "empty recording".into()));
    }
    let ct = headers.get(axum::http::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("audio/webm").to_string();
    match crate::speech::transcribe(core.speech_config(), body.to_vec(), &ct).await {
        Ok(text) => Ok(Json(serde_json::json!({ "text": text }))),
        Err(e) if e.contains("isn't set up") => Err((StatusCode::SERVICE_UNAVAILABLE, e)),
        Err(e) => Err((StatusCode::BAD_GATEWAY, e)),
    }
}

#[derive(Deserialize)]
struct TtsRequest {
    text: String,
    #[serde(default)]
    voice: Option<String>,
}

/// `POST /api/v1/tts` — `{ text, voice? }` → audio bytes (Kokoro on the model server) to play on the client.
async fn v1_tts(State(core): State<Arc<Core>>, Json(req): Json<TtsRequest>) -> Result<axum::response::Response, (StatusCode, String)> {
    match crate::speech::synthesize(core.speech_config(), &req.text, req.voice.as_deref()).await {
        Ok((bytes, ct)) => Ok(([(axum::http::header::CONTENT_TYPE, ct)], bytes).into_response()),
        Err(e) if e.contains("isn't set up") => Err((StatusCode::SERVICE_UNAVAILABLE, e)),
        Err(e) => Err((StatusCode::BAD_GATEWAY, e)),
    }
}

async fn v1_reports() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "reports": crate::troubleshoot::list_reports(60) }))
}

async fn v1_report(axum::extract::Path(id): axum::extract::Path<String>) -> Result<Json<serde_json::Value>, StatusCode> {
    crate::troubleshoot::report_detail(&id).map(Json).ok_or(StatusCode::NOT_FOUND)
}

#[derive(Deserialize)]
struct FixRecordRequest {
    action: String,
    outcome: serde_json::Value,
}

/// `POST /api/v1/reports/{id}/fix-record` — the laptop's `loki-cli` reports what a Fix did, so the
/// outcome shows up in the past-reports list on every device.
async fn v1_fix_record(axum::extract::Path(id): axum::extract::Path<String>, Json(req): Json<FixRecordRequest>) -> Result<StatusCode, (StatusCode, String)> {
    crate::troubleshoot::record_fix(&id, &req.action, req.outcome).map(|_| StatusCode::OK).map_err(|e| (StatusCode::BAD_REQUEST, e))
}

async fn v1_troubleshoot_status(axum::extract::Path(id): axum::extract::Path<String>) -> Result<Json<crate::troubleshoot::Job>, StatusCode> {
    crate::troubleshoot::job(&id).map(Json).ok_or(StatusCode::NOT_FOUND)
}

/// `GET /api/v1/permissions/rules` — the server-side "always allow" rules shared by all clients.
async fn rules_list(State(core): State<Arc<Core>>) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "rules": core.bridge().rules().list() }))
}

#[derive(Deserialize)]
struct RuleAddRequest {
    tool: String,
    #[serde(default)]
    input: serde_json::Value,
}

/// `POST /api/v1/permissions/rules` — "Yes, always" for exactly this call (see
/// `permission_rules::rule_key` for how specific a rule is). 400 for never-always tools.
async fn rules_add(State(core): State<Arc<Core>>, Json(req): Json<RuleAddRequest>) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    core.bridge().rules().add(&req.tool, &req.input).map(|rule| Json(serde_json::json!({ "rule": rule }))).map_err(|e| (StatusCode::BAD_REQUEST, e))
}

#[derive(Deserialize)]
struct RuleRemoveRequest {
    rule: String,
}

async fn rules_remove(State(core): State<Arc<Core>>, Json(req): Json<RuleRemoveRequest>) -> StatusCode {
    if core.bridge().rules().remove(&req.rule) { StatusCode::OK } else { StatusCode::NOT_FOUND }
}

/// `GET /api/v1/permissions/hosts` — hosts currently trusted for this server session, with seconds left.
async fn hosts_list(State(core): State<Arc<Core>>) -> Json<serde_json::Value> {
    let hosts: Vec<serde_json::Value> = core.bridge().rules().granted_hosts().into_iter().map(|(h, s)| serde_json::json!({ "host": h, "seconds_left": s })).collect();
    Json(serde_json::json!({ "hosts": hosts, "configured": core.remote_hosts() }))
}

#[derive(Deserialize)]
struct HostGrantRequest {
    host: String,
    /// Minutes of trust; clamped to 1..=240, default 120.
    #[serde(default)]
    minutes: Option<u64>,
}

/// `POST /api/v1/permissions/hosts/grant` — trust a configured remote host for a while: non-destructive
/// commands there stop asking. In memory only (never saved), gone on restart. Destructive words, redirects,
/// command substitution and secret-looking paths still ask.
async fn hosts_grant(State(core): State<Arc<Core>>, Json(req): Json<HostGrantRequest>) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if !core.remote_hosts().iter().any(|h| *h == req.host) {
        return Err((StatusCode::BAD_REQUEST, format!("'{}' is not a configured remote host", req.host)));
    }
    let minutes = req.minutes.unwrap_or(120).clamp(1, 240);
    core.bridge().rules().grant_host(&req.host, std::time::Duration::from_secs(minutes * 60));
    Ok(Json(serde_json::json!({ "host": req.host, "minutes": minutes })))
}

#[derive(Deserialize)]
struct HostRevokeRequest {
    host: String,
}

async fn hosts_revoke(State(core): State<Arc<Core>>, Json(req): Json<HostRevokeRequest>) -> StatusCode {
    if core.bridge().rules().revoke_host(&req.host) { StatusCode::OK } else { StatusCode::NOT_FOUND }
}

/// `GET /api/v1/permissions/workspaces` — trusted folders (per host, with minutes left or permanent)
/// and the hosts a folder can be trusted on.
async fn ws_list(State(core): State<Arc<Core>>) -> Json<serde_json::Value> {
    let list: Vec<serde_json::Value> = core
        .bridge()
        .rules()
        .trusted_workspaces()
        .into_iter()
        .map(|(host, path, left)| serde_json::json!({ "host": host, "path": path, "seconds_left": left }))
        .collect();
    let mut hosts = vec![crate::permission_rules::DEVICE_HOST.to_string()];
    hosts.extend(core.remote_hosts().iter().cloned());
    Json(serde_json::json!({ "workspaces": list, "hosts": hosts }))
}

#[derive(Deserialize)]
struct WsGrantRequest {
    host: String,
    path: String,
    /// Minutes of trust (1..=1440, default 120); ignored when `permanent`.
    #[serde(default)]
    minutes: Option<u64>,
    #[serde(default)]
    permanent: bool,
}

/// `POST /api/v1/permissions/workspaces/grant` — trust a folder on `device` or a configured remote host.
async fn ws_grant(State(core): State<Arc<Core>>, Json(req): Json<WsGrantRequest>) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if req.host != crate::permission_rules::DEVICE_HOST && !core.remote_hosts().iter().any(|h| *h == req.host) {
        return Err((StatusCode::BAD_REQUEST, format!("'{}' is neither the desktop device nor a configured remote host", req.host)));
    }
    let ttl = (!req.permanent).then(|| std::time::Duration::from_secs(req.minutes.unwrap_or(120).clamp(1, 1440) * 60));
    core.bridge().rules().trust_workspace(&req.host, &req.path, ttl).map(|path| Json(serde_json::json!({ "host": req.host, "path": path }))).map_err(|e| (StatusCode::BAD_REQUEST, e))
}

#[derive(Deserialize)]
struct WsRevokeRequest {
    host: String,
    path: String,
}

async fn ws_revoke(State(core): State<Arc<Core>>, Json(req): Json<WsRevokeRequest>) -> StatusCode {
    if core.bridge().rules().untrust_workspace(&req.host, &req.path) { StatusCode::OK } else { StatusCode::NOT_FOUND }
}

/// `GET /api/v1/mcp/servers` — every known server (on or off) with where it comes from and its state. No secrets.
async fn v1_mcp_servers() -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    crate::mcp_manage::list().map(|s| Json(serde_json::json!({ "servers": s }))).map_err(|e| (StatusCode::SERVICE_UNAVAILABLE, e))
}

async fn v1_mcp_tools(Path(name): Path<String>) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    crate::mcp_manage::tools_of(&name).map(|t| Json(serde_json::json!({ "tools": t }))).map_err(|e| (StatusCode::SERVICE_UNAVAILABLE, e))
}

/// `POST /api/v1/mcp/action` — enable, disable, reconnect, add or remove one server. The Settings page asks the user to
/// confirm before sending `add` or `remove`.
async fn v1_mcp_action(Json(a): Json<crate::mcp_manage::Action>) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    crate::mcp_manage::act(a).await.map(|m| Json(serde_json::json!({ "message": m }))).map_err(|e| (StatusCode::BAD_REQUEST, e))
}

/// `GET /api/v1/mcp` — which MCP servers Loki is connected to and how many tools each gave it.
async fn v1_mcp_status(State(core): State<Arc<Core>>) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "servers": core.mcp_status() }))
}

#[derive(Deserialize)]
struct EnginesQuery {
    #[serde(default)]
    session_id: Option<String>,
}

/// `GET /api/v1/engines[?session_id=…]` — selectable engines, the server default, and this
/// conversation's own choice.
async fn engines_list(State(core): State<Arc<Core>>, axum::extract::Query(q): axum::extract::Query<EnginesQuery>) -> Json<crate::engines::EngineList> {
    Json(core.list_engines(q.session_id.as_deref()).await)
}

#[derive(Deserialize)]
struct EngineSetRequest {
    engine: String,
    /// A conversation's choice; omit to set the server-wide default.
    #[serde(default)]
    session_id: Option<String>,
}

async fn engines_set(State(core): State<Arc<Core>>, Json(req): Json<EngineSetRequest>) -> Result<StatusCode, (StatusCode, String)> {
    core.set_engine(req.session_id.as_deref(), &req.engine).await.map(|_| StatusCode::OK).map_err(|e| (StatusCode::BAD_REQUEST, e))
}

#[cfg(test)]
mod leads_proxy_tests {
    use super::leads_path_ok;

    #[test]
    fn only_the_dashboards_own_paths_pass() {
        for ok in ["items", "facets", "families", "stats", "export", "refresh", "items/12", "items/12/draft", "items/12/steps", "families/3/steps"] {
            assert!(leads_path_ok(ok), "{ok}");
        }
        for bad in ["", "health", "items/abc", "items/12/delete", "../etc/passwd", "items/12/../x", "families/3", "items//steps", "items/99999999999"] {
            assert!(!leads_path_ok(bad), "{bad}");
        }
    }
}


#[cfg(test)]
mod ai_format_tests {
    use super::json_format;

    #[test]
    fn json_format_prefers_the_schema() {
        let schema = serde_json::json!({ "type": "object", "properties": { "a": { "type": "integer" } } });
        assert_eq!(json_format(false, None), None);
        assert_eq!(json_format(true, None), Some(serde_json::json!({ "type": "object" })));
        assert_eq!(json_format(true, Some(&schema)), Some(schema.clone()));
        assert_eq!(json_format(false, Some(&schema)), Some(schema));
        // a schema that is not an object is ignored rather than sent
        assert_eq!(json_format(false, Some(&serde_json::json!("json"))), None);
    }
}
