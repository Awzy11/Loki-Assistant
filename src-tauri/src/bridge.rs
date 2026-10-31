//! Request/response bridge between a running turn and the person at the client.
//!
//! A turn that needs a human decision (approve a Tier-2 action) or something only the client
//! can do (read a file on the desktop the app runs on) emits a request event on the session's
//! live SSE stream and waits here for the client's answer, posted back to
//! `/api/v1/bridge/respond`. Both the local-model path (`agent::run_turn`) and the Claude path
//! (via the `loki-bridge-mcp` helper Claude is spawned with) go through this one broker, so
//! the gate is enforced once, on the server, not in UI code.
//!
//! Fail-closed: no connected client, a timeout, or a dropped turn all mean "denied"/"error",
//! never "allowed".

use crate::bus::Bus;
use crate::events::ChatEvent;
use crate::permission_rules::PermissionRules;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::oneshot;

/// Names of the tools that act on the device running the client. Shared by the local tool
/// registry, the HTTP bridge endpoint, and `loki-bridge-mcp`.
pub const DEVICE_TOOLS: &[&str] = &[
    "device_read_file",
    "device_list_dir",
    "device_write_file",
    "device_edit_file",
    "device_run_command",
];

/// Device tools that change state and therefore need the user's approval each time (the
/// client may auto-answer from "always allow" rules). Reads are allowed without a prompt.
pub fn device_tool_needs_confirmation(name: &str) -> bool {
    matches!(name, "device_write_file" | "device_edit_file" | "device_run_command")
}

/// How long an approval waits for an answer before being denied (`[agent] approval_timeout_secs`).
/// Long on purpose: a request raised while you're away from the screen waits for you (and sends a
/// push notification) instead of being denied or vanishing.
static APPROVAL_TIMEOUT_SECS: AtomicU64 = AtomicU64::new(3600);

pub fn set_approval_timeout(secs: u64) {
    APPROVAL_TIMEOUT_SECS.store(secs.clamp(30, 6 * 3600), Ordering::Relaxed);
}

fn approval_timeout() -> Duration {
    Duration::from_secs(APPROVAL_TIMEOUT_SECS.load(Ordering::Relaxed))
}

/// If a screen is attached but nobody answers within this long, nudge the phone as well.
const PUSH_DELAY_WHEN_ATTACHED: Duration = Duration::from_secs(20);
const DEVICE_TIMEOUT: Duration = Duration::from_secs(150);

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum BridgeError {
    #[error("no client is connected to this turn (it is not interactive)")]
    NoClient,
    #[error("timed out waiting for the client")]
    Timeout,
    #[error("the turn was cancelled")]
    Cancelled,
}

/// One request waiting for an answer.
struct Pending {
    session: String,
    tx: oneshot::Sender<Value>,
    /// The approval event, kept so a client that attaches later (the phone coming back) is shown
    /// the prompt again instead of it having silently vanished.
    replay: Option<ChatEvent>,
}

pub struct Bridge {
    rules: Arc<PermissionRules>,
    bus: Arc<Bus>,
    next_id: AtomicU64,
    pending: Arc<Mutex<HashMap<String, Pending>>>,
}

impl Default for Bridge {
    fn default() -> Self {
        Self::new(Arc::new(PermissionRules::in_memory()), Arc::new(Bus::default()))
    }
}

/// Who a request is delivered to.
enum Target<'a> {
    /// Every client attached to the conversation (approvals: any of them may answer).
    All,
    /// One client that declared this capability (device tools).
    Capable(&'a str),
}

/// When the waiting future ends (answered, timed out, or the turn was cancelled), forget the
/// request and tell every client to dismiss the prompt.
struct PendingGuard<'a> {
    bridge: &'a Bridge,
    id: String,
    session_id: String,
}

impl Drop for PendingGuard<'_> {
    fn drop(&mut self) {
        self.bridge.pending.lock().unwrap().remove(&self.id);
        self.bridge.bus.publish(&self.session_id, ChatEvent::Resolved { id: self.id.clone() });
    }
}

impl Bridge {
    pub fn new(rules: Arc<PermissionRules>, bus: Arc<Bus>) -> Self {
        Self { rules, bus, next_id: AtomicU64::new(0), pending: Arc::new(Mutex::new(HashMap::new())) }
    }

    pub fn rules(&self) -> &Arc<PermissionRules> {
        &self.rules
    }

    pub fn bus(&self) -> &Arc<Bus> {
        &self.bus
    }

    /// Publishes the event built by `make_event` (given a fresh request id) to `target` on
    /// `session_id` and waits for the answer. Approvals (`Target::All`) wait even when no client is
    /// attached — the prompt is replayed to whoever attaches next, and `push` (if given) is sent to
    /// the phone; a device request (`Target::Capable`) can only be served by an attached client, so
    /// without one it fails at once.
    async fn ask(
        &self,
        session_id: &str,
        target: Target<'_>,
        make_event: impl FnOnce(String) -> ChatEvent,
        timeout: Duration,
        push: Option<serde_json::Value>,
    ) -> Result<Value, BridgeError> {
        let id = format!("req-{}", self.next_id.fetch_add(1, Ordering::Relaxed));
        let event = make_event(id.clone());
        let is_approval = matches!(target, Target::All);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id.clone(), Pending { session: session_id.to_string(), tx, replay: is_approval.then(|| event.clone()) });
        let _guard = PendingGuard { bridge: self, id: id.clone(), session_id: session_id.to_string() };
        let delivered = match target {
            Target::All => self.bus.publish(session_id, event) > 0,
            Target::Capable(cap) => self.bus.publish_to_capable(session_id, cap, event),
        };
        if !delivered && !is_approval {
            return Err(BridgeError::NoClient);
        }
        if let Some(payload) = push {
            // Nobody attached: tell the phone now. Someone attached: give them a moment to answer
            // before pinging the phone too.
            let delay = if delivered { PUSH_DELAY_WHEN_ATTACHED } else { Duration::ZERO };
            let pending = self.pending.clone();
            let watch_id = id.clone();
            tokio::spawn(async move {
                tokio::time::sleep(delay).await;
                if pending.lock().unwrap().contains_key(&watch_id) {
                    crate::push::send_in_background(payload);
                }
            });
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(_)) => Err(BridgeError::Cancelled),
            Err(_) => Err(BridgeError::Timeout),
        }
    }

    /// Delivers a client's answer. `false` if the request is unknown (already answered by
    /// another client, timed out, or the turn is gone).
    pub fn respond(&self, id: &str, value: Value) -> bool {
        match self.pending.lock().unwrap().remove(id) {
            Some(p) => p.tx.send(value).is_ok(),
            None => false,
        }
    }

    /// Approval prompts still waiting in `session_id`, for a client that has just attached.
    pub fn pending_events(&self, session_id: &str) -> Vec<ChatEvent> {
        self.pending.lock().unwrap().values().filter(|p| p.session == session_id).filter_map(|p| p.replay.clone()).collect()
    }

    /// Asks the user to approve `tool` with `input` (any attached client may answer). Any
    /// failure is a denial.
    pub async fn request_permission(&self, session_id: &str, tool: &str, input: &Value) -> bool {
        if self.rules.is_allowed(tool, input) {
            // Saved "always allow" rule: no prompt, but show the user it happened.
            self.bus.publish(session_id, ChatEvent::PermissionAuto { tool: tool.to_string(), input: input.clone() });
            return true;
        }
        let tool_owned = tool.to_string();
        let input_owned = input.clone();
        let push = serde_json::json!({
            "title": "Loki needs your approval",
            "body": approval_body(tool, input),
            "url": format!("/?session={session_id}"),
            "tag": format!("approval-{session_id}"),
            "sticky": true,
        });
        let answer = self
            .ask(
                session_id,
                Target::All,
                move |id| ChatEvent::PermissionRequest { id, tool: tool_owned, input: input_owned },
                approval_timeout(),
                Some(push),
            )
            .await;
        match answer {
            Ok(v) => v["allow"].as_bool().unwrap_or(false),
            Err(e) => {
                eprintln!("[loki-debug] permission request for '{tool}' denied: {e}");
                if e == BridgeError::Timeout {
                    crate::push::send_in_background(serde_json::json!({
                        "title": "Loki's approval request timed out",
                        "body": format!("Nobody answered in time, so {} was denied.", pretty_tool(tool)),
                        "url": format!("/?session={session_id}"),
                        "tag": format!("approval-{session_id}"),
                    }));
                }
                false
            }
        }
    }

    /// Runs a `device_*` tool on an attached client that has the `device_fs` capability.
    /// `confirm`: ask the user first (the Claude path, where nothing else gates it; the local
    /// path gates in `agent::run_turn`). Returns the tool's text output, or an error string
    /// suitable for handing to a model.
    pub async fn device_call(&self, session_id: &str, name: &str, input: &Value, confirm: bool) -> Result<String, String> {
        if !DEVICE_TOOLS.contains(&name) {
            return Err(format!("unknown device tool '{name}'"));
        }
        if confirm && device_tool_needs_confirmation(name) && !self.request_permission(session_id, name, input).await {
            return Err("the user denied this action".to_string());
        }
        let name_owned = name.to_string();
        let input_owned = input.clone();
        let answer = self
            .ask(
                session_id,
                Target::Capable("device_fs"),
                move |id| ChatEvent::DeviceRequest { id, name: name_owned, input: input_owned },
                DEVICE_TIMEOUT,
                None,
            )
            .await
            .map_err(|e| match e {
                BridgeError::NoClient => {
                    "no device is connected to this conversation — device tools need the Loki desktop app (or another client with file access) attached".to_string()
                }
                other => other.to_string(),
            })?;
        let content = answer["content"].as_str().unwrap_or_default().to_string();
        if answer["is_error"].as_bool().unwrap_or(false) {
            Err(content)
        } else {
            Ok(content)
        }
    }
}

/// JSON a Claude Code `--permission-prompt-tool` must return.
/// "mcp__github__push_files" → "github · push_files".
fn pretty_tool(tool: &str) -> String {
    match tool.strip_prefix("mcp__").and_then(|r| r.split_once("__")) {
        Some((server, name)) => format!("{server} · {name}"),
        None => tool.to_string(),
    }
}

/// Notification text for an approval: which tool and, if it names one, the file or page it targets.
/// Never the content being written or the command being run.
fn approval_body(tool: &str, input: &Value) -> String {
    let target = ["file_path", "path", "filepath", "title", "url"].iter().find_map(|k| input.get(*k).and_then(|v| v.as_str())).map(|s| s.chars().take(80).collect::<String>());
    match target {
        Some(t) => format!("{} → {t}. Tap to approve or deny.", pretty_tool(tool)),
        None => format!("{}. Tap to approve or deny.", pretty_tool(tool)),
    }
}

pub fn permission_decision(allow: bool, input: &Value) -> Value {
    if allow {
        json!({ "behavior": "allow", "updatedInput": input })
    } else {
        json!({ "behavior": "deny", "message": "The user declined this action in Loki." })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::{ClientInfo, SubGuard};
    use tokio::sync::mpsc::UnboundedReceiver;

    fn client(id: &str, caps: &[&str]) -> ClientInfo {
        ClientInfo { id: id.into(), kind: "test".into(), capabilities: caps.iter().map(|c| c.to_string()).collect() }
    }

    /// A bridge plus one attached client that has file access.
    fn setup() -> (Arc<Bridge>, SubGuard, UnboundedReceiver<ChatEvent>) {
        let bridge = Arc::new(Bridge::default());
        let (rx, guard) = bridge.bus().subscribe("s", client("desktop", &["device_fs"]));
        (bridge, guard, rx)
    }

    async fn next_request(rx: &mut UnboundedReceiver<ChatEvent>) -> ChatEvent {
        loop {
            match rx.recv().await.unwrap() {
                ChatEvent::Resolved { .. } => continue,
                other => return other,
            }
        }
    }

    #[tokio::test]
    async fn permission_allow_and_deny() {
        let (bridge, _g, mut rx) = setup();
        let b = bridge.clone();
        let t = tokio::spawn(async move { b.request_permission("s", "obsidian_delete_file", &json!({"path":"a.md"})).await });
        let ChatEvent::PermissionRequest { id, tool, .. } = next_request(&mut rx).await else { panic!("wrong event") };
        assert_eq!(tool, "obsidian_delete_file");
        assert!(bridge.respond(&id, json!({"allow": true})));
        assert!(t.await.unwrap());

        let b = bridge.clone();
        let t = tokio::spawn(async move { b.request_permission("s", "x", &json!({})).await });
        let ChatEvent::PermissionRequest { id, .. } = next_request(&mut rx).await else { panic!() };
        bridge.respond(&id, json!({"allow": false}));
        assert!(!t.await.unwrap());
    }

    #[tokio::test]
    async fn saved_rule_auto_allows_without_prompting() {
        let (bridge, _g, mut rx) = setup();
        bridge.rules().add("Bash", &json!({"command": "ls"})).unwrap();
        assert!(bridge.request_permission("s", "Bash", &json!({"command": "ls"})).await);
        assert!(matches!(rx.recv().await.unwrap(), ChatEvent::PermissionAuto { .. }));
        let b = bridge.clone();
        let t = tokio::spawn(async move { b.request_permission("s", "Bash", &json!({"command": "rm -rf /"})).await });
        assert!(matches!(next_request(&mut rx).await, ChatEvent::PermissionRequest { .. }));
        t.abort();
    }

    #[tokio::test]
    async fn device_calls_without_a_client_fail_at_once() {
        let bridge = Bridge::default();
        assert!(bridge.device_call("nobody", "device_read_file", &json!({}), false).await.unwrap_err().contains("no device"));
    }

    #[tokio::test]
    async fn an_approval_waits_for_a_client_that_attaches_later() {
        let bridge = Arc::new(Bridge::default());
        let b = bridge.clone();
        let t = tokio::spawn(async move { b.request_permission("away", "obsidian_delete_file", &json!({"path": "x.md"})).await });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!t.is_finished(), "must wait, not deny, when nobody is attached");
        // The phone comes back: the prompt is replayed to it.
        let replay = bridge.pending_events("away");
        let [ChatEvent::PermissionRequest { id, tool, .. }] = replay.as_slice() else { panic!("expected one pending approval, got {replay:?}") };
        assert_eq!(tool, "obsidian_delete_file");
        assert!(bridge.pending_events("another-session").is_empty());
        assert!(bridge.respond(id, json!({"allow": true})));
        assert!(t.await.unwrap());
        assert!(bridge.pending_events("away").is_empty());
    }

    #[tokio::test]
    async fn an_unanswered_approval_is_denied_after_the_timeout() {
        let bridge = Bridge::default();
        let r = bridge.ask("s", Target::All, |id| ChatEvent::PermissionRequest { id, tool: "x".into(), input: json!({}) }, Duration::from_millis(40), None).await;
        assert_eq!(r.unwrap_err(), BridgeError::Timeout);
        assert!(bridge.pending_events("s").is_empty());
    }

    #[test]
    fn approval_notifications_name_the_tool_and_target_but_never_content() {
        let b = approval_body("mcp__obsidian__obsidian_append_content", &json!({"filepath": "10 - Daily/2026-10-12.md", "content": "secret text"}));
        assert!(b.contains("obsidian · obsidian_append_content"));
        assert!(b.contains("10 - Daily/2026-10-12.md"));
        assert!(!b.contains("secret"));
        assert!(approval_body("Bash", &json!({"command": "rm -rf /"})).starts_with("Bash."));
    }

    #[tokio::test]
    async fn device_call_round_trip_and_error() {
        let (bridge, _g, mut rx) = setup();
        let b = bridge.clone();
        let t = tokio::spawn(async move { b.device_call("s", "device_read_file", &json!({"path":"/a"}), true).await });
        let ChatEvent::DeviceRequest { id, name, .. } = next_request(&mut rx).await else { panic!() };
        assert_eq!(name, "device_read_file"); // reads never prompt, even with confirm=true
        bridge.respond(&id, json!({"content":"hello","is_error":false}));
        assert_eq!(t.await.unwrap().unwrap(), "hello");

        let b = bridge.clone();
        let t = tokio::spawn(async move { b.device_call("s", "device_write_file", &json!({}), true).await });
        let ChatEvent::PermissionRequest { id, .. } = next_request(&mut rx).await else { panic!("write must prompt first") };
        bridge.respond(&id, json!({"allow": false}));
        assert!(t.await.unwrap().unwrap_err().contains("denied"));
    }

    #[tokio::test]
    async fn dropped_waiter_cleans_up_and_late_answer_is_rejected() {
        let (bridge, _g, mut rx) = setup();
        let b = bridge.clone();
        let t = tokio::spawn(async move { b.request_permission("s", "x", &json!({})).await });
        let ChatEvent::PermissionRequest { id, .. } = next_request(&mut rx).await else { panic!() };
        t.abort();
        let _ = t.await;
        assert!(!bridge.respond(&id, json!({"allow": true})));
    }

    #[tokio::test]
    async fn approvals_reach_every_client_and_others_are_told_it_was_resolved() {
        let bridge = Arc::new(Bridge::default());
        let (mut phone, _gp) = bridge.bus().subscribe("s", client("phone", &[]));
        let (mut desktop, _gd) = bridge.bus().subscribe("s", client("desktop", &["device_fs"]));
        let b = bridge.clone();
        let t = tokio::spawn(async move { b.request_permission("s", "mcp__x__y", &json!({})).await });
        let ChatEvent::PermissionRequest { id, .. } = phone.recv().await.unwrap() else { panic!("phone should be asked") };
        assert!(matches!(desktop.recv().await.unwrap(), ChatEvent::PermissionRequest { .. }));
        // The phone answers; the desktop must be told to dismiss its prompt.
        assert!(bridge.respond(&id, json!({"allow": true})));
        assert!(t.await.unwrap());
        assert!(matches!(desktop.recv().await.unwrap(), ChatEvent::Resolved { id: rid } if rid == id));
        // A second (late) answer from the other client is rejected.
        assert!(!bridge.respond(&id, json!({"allow": false})));
    }

    #[tokio::test]
    async fn device_requests_only_go_to_a_capable_client() {
        let bridge = Arc::new(Bridge::default());
        // Only a phone (no file access) is attached: the request cannot be served.
        let (_phone, _gp) = bridge.bus().subscribe("s", client("phone", &[]));
        let err = bridge.device_call("s", "device_read_file", &json!({"path": "/a"}), false).await.unwrap_err();
        assert!(err.contains("no device is connected"));
        // Attach a desktop: now it is routed there, not to the phone.
        let (mut desktop, _gd) = bridge.bus().subscribe("s", client("desktop", &["device_fs"]));
        let b = bridge.clone();
        let t = tokio::spawn(async move { b.device_call("s", "device_read_file", &json!({"path": "/a"}), false).await });
        let ChatEvent::DeviceRequest { id, .. } = next_request(&mut desktop).await else { panic!() };
        bridge.respond(&id, json!({"content": "x", "is_error": false}));
        assert_eq!(t.await.unwrap().unwrap(), "x");
    }

    #[test]
    fn unknown_device_tool_rejected() {
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let bridge = Bridge::default();
        assert!(rt.block_on(bridge.device_call("s", "rm_rf", &json!({}), false)).is_err());
    }
}
