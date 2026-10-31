//! Per-conversation event bus: every client attached to a conversation (the one that started
//! a turn, plus anything watching it: desktop, phone, sidebar, web) receives that conversation's
//! events. Approval prompts go to every attached client (first answer wins); device requests go
//! to a client that declared the capability. Nothing here knows about HTTP — `server.rs` turns
//! subscriptions into SSE streams.

use crate::events::ChatEvent;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

/// What a client says about itself when it attaches.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ClientInfo {
    #[serde(default)]
    pub id: String,
    /// "web", "desktop", "sidebar", "android", "cli", …  (informational)
    #[serde(default)]
    pub kind: String,
    /// What this client can do for the server. Currently: `device_fs` — can run the `device_*`
    /// tools (files, shell) on the machine it is running on.
    #[serde(default, alias = "caps")]
    pub capabilities: Vec<String>,
}

struct Sub {
    id: u64,
    client: ClientInfo,
    tx: UnboundedSender<ChatEvent>,
}

#[derive(Default)]
pub struct Bus {
    next: AtomicU64,
    subs: Mutex<HashMap<String, Vec<Sub>>>,
}

/// Detaches a client from a conversation when dropped (stream closed, tab closed, turn aborted).
pub struct SubGuard {
    bus: Arc<Bus>,
    session: String,
    id: u64,
}

impl Drop for SubGuard {
    fn drop(&mut self) {
        let mut subs = self.bus.subs.lock().unwrap();
        if let Some(list) = subs.get_mut(&self.session) {
            list.retain(|s| s.id != self.id);
            if list.is_empty() {
                subs.remove(&self.session);
            }
        }
    }
}

impl Bus {
    pub fn subscribe(self: &Arc<Self>, session: &str, client: ClientInfo) -> (UnboundedReceiver<ChatEvent>, SubGuard) {
        let (tx, rx) = unbounded_channel();
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        self.subs.lock().unwrap().entry(session.to_string()).or_default().push(Sub { id, client, tx });
        (rx, SubGuard { bus: self.clone(), session: session.to_string(), id })
    }

    /// How many clients are attached to `session` right now (0 = nobody is watching).
    pub fn subscriber_count(&self, session: &str) -> usize {
        self.subs.lock().unwrap().get(session).map(|l| l.len()).unwrap_or(0)
    }

    /// Delivers `ev` to every client attached to `session`; returns how many received it.
    pub fn publish(&self, session: &str, ev: ChatEvent) -> usize {
        crate::event_log::record(session, &ev);
        let subs = self.subs.lock().unwrap();
        subs.get(session).map(|list| list.iter().filter(|s| s.tx.send(ev.clone()).is_ok()).count()).unwrap_or(0)
    }

    /// Delivers `ev` to the first attached client that declared `capability`. `false` if none.
    pub fn publish_to_capable(&self, session: &str, capability: &str, ev: ChatEvent) -> bool {
        let subs = self.subs.lock().unwrap();
        subs.get(session)
            .and_then(|list| list.iter().find(|s| s.client.capabilities.iter().any(|c| c == capability) && s.tx.send(ev.clone()).is_ok()))
            .is_some()
    }

    pub fn attached(&self, session: &str) -> usize {
        self.subs.lock().unwrap().get(session).map(|l| l.len()).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client(id: &str, caps: &[&str]) -> ClientInfo {
        ClientInfo { id: id.into(), kind: "t".into(), capabilities: caps.iter().map(|c| c.to_string()).collect() }
    }
    fn status(m: &str) -> ChatEvent {
        ChatEvent::Status { message: m.into() }
    }

    #[tokio::test]
    async fn publish_reaches_every_attached_client_and_only_that_session() {
        let bus = Arc::new(Bus::default());
        let (mut a, _ga) = bus.subscribe("s1", client("a", &[]));
        let (mut b, _gb) = bus.subscribe("s1", client("b", &[]));
        let (mut other, _go) = bus.subscribe("s2", client("c", &[]));
        assert_eq!(bus.publish("s1", status("hi")), 2);
        assert!(matches!(a.recv().await.unwrap(), ChatEvent::Status { .. }));
        assert!(matches!(b.recv().await.unwrap(), ChatEvent::Status { .. }));
        assert!(other.try_recv().is_err());
        assert_eq!(bus.publish("nobody", status("x")), 0);
    }

    #[tokio::test]
    async fn capable_routing_picks_a_client_with_the_capability() {
        let bus = Arc::new(Bus::default());
        let (mut phone, _gp) = bus.subscribe("s", client("phone", &[]));
        let (mut desktop, _gd) = bus.subscribe("s", client("desktop", &["device_fs"]));
        assert!(bus.publish_to_capable("s", "device_fs", status("dev")));
        assert!(matches!(desktop.recv().await.unwrap(), ChatEvent::Status { .. }));
        assert!(phone.try_recv().is_err());
        assert!(!bus.publish_to_capable("s", "screenshot", status("x")));
    }

    #[tokio::test]
    async fn dropping_the_guard_detaches() {
        let bus = Arc::new(Bus::default());
        let (_rx, guard) = bus.subscribe("s", client("a", &["device_fs"]));
        assert_eq!(bus.attached("s"), 1);
        drop(guard);
        assert_eq!(bus.attached("s"), 0);
        assert!(!bus.publish_to_capable("s", "device_fs", status("x")));
    }

    #[test]
    fn client_info_accepts_caps_alias_and_defaults() {
        let c: ClientInfo = serde_json::from_str(r#"{"id":"x","caps":["device_fs"]}"#).unwrap();
        assert_eq!(c.capabilities, vec!["device_fs"]);
        let d: ClientInfo = serde_json::from_str("{}").unwrap();
        assert!(d.capabilities.is_empty());
    }
}
