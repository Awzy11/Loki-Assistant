//! The engine picker: which model(s) Loki uses for a conversation. `auto` is Loki's normal
//! routing (Claude first, privacy classifier, failover); the others pin a specific engine.
//! The default and each conversation's choice live on the server, so every client (web,
//! desktop, sidebar, phone) sees and changes the same setting.

use crate::config::{ClaudeMode, LlmBackend};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const AUTO: &str = "auto";
pub const CLAUDE_CODE: &str = "claude-code";
pub const CONFIGURED: &str = "configured";

#[derive(Debug, Clone, Serialize)]
pub struct Engine {
    pub id: String,
    pub label: String,
    /// "auto" | "claude_code" | "claude" | "configured" | "ollama"
    pub kind: &'static str,
    /// Sends the conversation off this machine.
    pub cloud: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct EngineList {
    pub engines: Vec<Engine>,
    /// The server-wide default (what a conversation without its own choice uses).
    pub default: String,
    /// This conversation's own choice, if it has one.
    pub session: Option<String>,
    pub claude_mode: ClaudeMode,
    /// What `Auto` does here, in one sentence (shown in the chat header).
    pub summary: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct EngineSettings {
    #[serde(default)]
    pub default_engine: Option<String>,
}

fn settings_path() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("loki").join("settings.json"))
}

impl EngineSettings {
    pub fn load() -> Self {
        settings_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(path) = settings_path() else { return };
        let result = (|| -> anyhow::Result<()> {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, serde_json::to_vec_pretty(self)?)?;
            Ok(())
        })();
        if let Err(e) = result {
            eprintln!("[loki-debug] couldn't save engine settings (non-fatal): {e}");
        }
    }
}

/// "ollama/llama3.1:8b", "gemini/gemini-2.5-pro", …
pub fn backend_label(b: &LlmBackend) -> String {
    let model = match b {
        LlmBackend::Ollama { model, .. }
        | LlmBackend::OpenaiCompatible { model, .. }
        | LlmBackend::Anthropic { model, .. }
        | LlmBackend::Gemini { model, .. } => model,
    };
    format!("{}/{model}", b.vendor())
}

pub fn backend_is_cloud(b: &LlmBackend) -> bool {
    !matches!(b, LlmBackend::Ollama { .. })
}

/// Model names from an Ollama server's `/api/tags`, embedding-only models excluded.
pub async fn ollama_models(host: &str, port: u16) -> Vec<String> {
    #[derive(Deserialize)]
    struct Tags {
        models: Vec<Tag>,
    }
    #[derive(Deserialize)]
    struct Tag {
        name: String,
    }
    let url = format!("http://{host}:{port}/api/tags");
    let client = match reqwest::Client::builder().timeout(std::time::Duration::from_secs(3)).build() {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let Ok(resp) = client.get(url).send().await else { return Vec::new() };
    let Ok(tags) = resp.json::<Tags>().await else { return Vec::new() };
    let mut names: Vec<String> = tags.models.into_iter().map(|t| t.name).filter(|n| !n.to_lowercase().contains("embed")).collect();
    names.sort();
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_and_cloudness() {
        let o = LlmBackend::Ollama { host: "h".into(), port: 1, model: "m".into() };
        assert_eq!(backend_label(&o), "ollama/m");
        assert!(!backend_is_cloud(&o));
        let g = LlmBackend::Gemini { api_key: None, model: "g".into() };
        assert_eq!(backend_label(&g), "gemini/g");
        assert!(backend_is_cloud(&g));
    }
}
