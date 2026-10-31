//! Home-server cards (Finance, Home network, Recently saved, news-globe data). They read the owner's own Pi over SSH and are not part of the
//! public build yet (see ROADMAP.md); these entry points keep the rest of Loki unchanged and return nothing.
use crate::needs_you::Briefing;

pub async fn briefings() -> Vec<Briefing> {
    Vec::new()
}

pub async fn news_geo(_name: &str) -> Option<String> {
    None
}

pub fn finance_sync_spawn_loop() {}

pub async fn finance_sync_now() -> Result<String, String> {
    Err("the Finance sync is not part of this build".into())
}
