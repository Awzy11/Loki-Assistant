use super::{AnthropicClient, GeminiClient, LlmProvider, OllamaClient, OpenAiCompatibleClient};
use crate::config::{LlmBackend, OllamaConfig};
use std::sync::Arc;

/// The one place that grows when a new BYOK vendor is added: one enum variant in
/// `config::LlmBackend`, one `providers/<vendor>.rs` file implementing `LlmProvider`, one arm
/// here.
///
/// Returns `Arc`, not `Box`: `Core` holds the main chat `llm` and the knowledge store needs
/// its own handle to a chat-capable provider for auto-tagging (`knowledge::tags`) — reusing
/// the main chat model rather than whatever `[embedding]` points at, since that's very likely
/// not a chat-capable model (e.g. `nomic-embed-text`). `Arc` lets both hold the same instance
/// cheaply instead of constructing/config-loading it twice.
pub fn build(backend: &LlmBackend) -> Arc<dyn LlmProvider> {
    match backend {
        LlmBackend::Ollama { host, port, model } => {
            Arc::new(OllamaClient::new(OllamaConfig { host: host.clone(), port: *port, model: model.clone() }))
        }
        LlmBackend::OpenaiCompatible { base_url, api_key, model } => {
            Arc::new(OpenAiCompatibleClient::new(base_url.clone(), api_key.clone(), model.clone()))
        }
        LlmBackend::Anthropic { api_key, model } => Arc::new(AnthropicClient::new(api_key.clone(), model.clone())),
        LlmBackend::Gemini { api_key, model } => Arc::new(GeminiClient::new(api_key.clone(), model.clone())),
    }
}
