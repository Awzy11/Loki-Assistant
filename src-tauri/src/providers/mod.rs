pub mod agy_cli;
pub mod anthropic;
pub mod claude_cli;
pub mod factory;
pub mod gemini;
pub mod ollama;
pub mod openai_compatible;
pub mod types;

pub use agy_cli::AgyCliProvider;
pub use anthropic::AnthropicClient;
pub use claude_cli::ClaudeCliProvider;
pub use gemini::GeminiClient;
pub use ollama::OllamaClient;
pub use openai_compatible::OpenAiCompatibleClient;
pub use types::{Message, ToolCall, ToolCallFunction, ToolDef, ToolFunctionDef};

use async_trait::async_trait;

/// A swappable chat backend — local (Ollama) or cloud (OpenAI-compatible, Anthropic).
///
/// `ClaudeClient` (see `crate::claude`, the *delegated* Claude Code loop) deliberately does NOT
/// implement this trait, and the BYOK `LlmBackend` config enum has no Claude-subscription
/// variant: the subscription can't be selected through the API-key vendor list by accident.
/// Claude as an inference engine is `ClaudeCliProvider` (`providers::claude_cli`), constructed
/// only from the explicit `[claude]` settings / `claude:<model>` engines.
#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn chat(&self, messages: &[Message], tools: &[ToolDef]) -> anyhow::Result<Message>;

    /// Does this provider send data off this machine? Every cloud-backed provider must
    /// return `true` — this is what the local-only escalation guard in `agent::run_turn`
    /// checks, both for `escalate_to_claude`-style tool escalation (Phase 0) and, generalized
    /// in Phase 1, for the primary chat call itself when a cloud provider is the daily driver.
    fn is_cloud(&self) -> bool;

    /// Short identifier for logging/debugging (e.g. "ollama", "anthropic").
    fn name(&self) -> &str;

    /// Like `chat`, but calls `on_delta` with text as it is generated. The default has no
    /// token streaming: it calls `chat` and delivers the finished text in one piece (only for
    /// a plain reply — narration that accompanies tool calls is delivered too, it is part of
    /// the turn). Providers with a streaming wire format override this.
    async fn chat_stream(
        &self,
        messages: &[Message],
        tools: &[ToolDef],
        on_delta: &mut (dyn for<'s> FnMut(&'s str) + Send),
    ) -> anyhow::Result<Message> {
        let reply = self.chat(messages, tools).await?;
        if !reply.content.is_empty() {
            on_delta(&reply.content);
        }
        Ok(reply)
    }

    /// Providers with a large context and a prefix cache (the Claude CLI) want the whole
    /// transcript every turn — an append-only prefix keeps the cache warm, whereas a sliding
    /// window would break it. `Some(chars)` = send everything up to that many characters.
    fn full_history_budget(&self) -> Option<usize> {
        None
    }

    /// Usage info (e.g. Claude's rate-limit windows) from the last call, if the provider has
    /// any. Taken (cleared) by the agent loop and forwarded to the UI.
    fn take_usage(&self) -> Option<serde_json::Value> {
        None
    }

    /// A cheaper/faster variant of `chat` for low-stakes, classification-shaped calls (today:
    /// auto-tagging in `knowledge::tags`) where deep reasoning isn't worth the latency.
    /// Default just delegates to `chat` — providers with no equivalent lever (OpenAI-compatible,
    /// Anthropic) get identical behavior to `chat`, unchanged. `OllamaClient` overrides this to
    /// disable qwen3's extended "thinking" step: measured at 67s vs. 0.9s for the same
    /// real-content prompt, with a comparably good answer either way — see its doc comment.
    async fn chat_low_effort(&self, messages: &[Message]) -> anyhow::Result<Message> {
        self.chat(messages, &[]).await
    }

    /// Embeds a piece of text into a vector, for the knowledge store. Not every provider
    /// supports this (Anthropic has no embeddings API) — the default returns a clear error
    /// rather than requiring every impl to write the same boilerplate.
    async fn embed(&self, _text: &str) -> anyhow::Result<Vec<f32>> {
        anyhow::bail!("{} does not support embeddings", self.name())
    }
}
