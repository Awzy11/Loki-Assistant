//! Live events a turn emits while it runs, for streaming front doors (SSE). Non-streaming
//! callers pass no sink and these are simply dropped.

use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChatEvent {
    /// A user message was submitted to the conversation (so other attached clients can show it).
    /// `client_id` identifies the sender, so it can skip its own echo.
    UserMessage { text: String, client_id: String },
    /// An approval or device request was answered (by some client); others should dismiss it.
    Resolved { id: String },
    /// First event of a stream: the session id the turn runs in.
    Started { session_id: String },
    /// Incremental assistant text.
    TextDelta { text: String },
    /// Discard text streamed so far this turn: the backend that produced it failed and the
    /// turn is being retried on the next one.
    Reset,
    /// A tool call began. `input` is the final arguments.
    ToolStart { id: String, name: String, input: Value },
    ToolResult { id: String, content: String, is_error: bool },
    /// The model's reasoning trace for one round (recorded for audits; clients may ignore it).
    Thinking { text: String },
    /// Human-readable progress/status line (e.g. "Claude is out of usage, trying gemini…").
    Status { message: String },
    /// Claude subscription usage as reported by the CLI's `rate_limit_event` (raw window info).
    Usage { info: Value },
    /// The turn wants the user's approval for `tool(input)`. The client answers via
    /// `POST /api/v1/bridge/respond` with `{id, value: {allow: bool}}`.
    PermissionRequest { id: String, tool: String, input: Value },
    /// A permission request was answered automatically by a saved always-allow rule.
    PermissionAuto { tool: String, input: Value },
    /// The turn needs the client's machine to run a `device_*` tool. The client answers via
    /// `POST /api/v1/bridge/respond` with `{id, value: {content, is_error}}`.
    DeviceRequest { id: String, name: String, input: Value },
    /// Terminal event: the turn finished. Carries the same fields as `/api/v1/chat`'s body.
    Done {
        session_id: String,
        reply: String,
        via: String,
        note: Option<String>,
        in_routine_mode: bool,
        force_local: bool,
    },
    /// Terminal event: the turn failed.
    Error { message: String },
}

/// Where a running turn publishes its events: the conversation's bus, which fans out to every
/// attached client. Cheap to clone.
#[derive(Clone)]
pub struct EventSink {
    pub bus: std::sync::Arc<crate::bus::Bus>,
    pub session_id: String,
    /// The client that started this turn (so mirrors can skip its own echo).
    pub client_id: String,
}

impl EventSink {
    pub fn send(&self, event: ChatEvent) {
        self.bus.publish(&self.session_id, event);
    }
}

pub fn emit(sink: Option<&EventSink>, event: ChatEvent) {
    if let Some(s) = sink {
        s.send(event);
    }
}
