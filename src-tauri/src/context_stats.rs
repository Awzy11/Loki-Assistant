//! Context controls for Loki's own agent loop: the live, user-changeable context settings
//! (window size, share spent on verbatim history, autocompact on/off and its turn trigger), what
//! the model last measured for each conversation, and the plain-text `/context` report.
//!
//! Settings start from `[context]` in config.toml and are then overridden by
//! `~/.local/share/loki/context_settings.json`, which the UI's controls write. They apply from the
//! next turn without a restart.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock, RwLock};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextSettings {
    /// Fold old turns into the rolling summary on their own once the trigger is reached.
    pub autocompact: bool,
    /// Share of the window spent on verbatim conversation history (0.10–0.90).
    pub history_fraction: f32,
    /// The local model's context window in tokens (what Ollama loads it with).
    pub num_ctx: usize,
    /// Unsummarized turns that trigger an autocompact regardless of size.
    pub max_turns: usize,
}

impl ContextSettings {
    fn clamped(mut self) -> Self {
        self.history_fraction = self.history_fraction.clamp(0.10, 0.90);
        self.num_ctx = self.num_ctx.clamp(8_192, 262_144);
        self.max_turns = self.max_turns.clamp(6, 200);
        self
    }
    pub fn budget_tokens(&self) -> usize {
        (self.num_ctx as f32 * self.history_fraction) as usize
    }
}

static SETTINGS: RwLock<ContextSettings> = RwLock::new(ContextSettings { autocompact: true, history_fraction: 0.125, num_ctx: 262_144, max_turns: crate::session_store::SUMMARY_TRIGGER });

fn settings_path() -> Option<std::path::PathBuf> {
    dirs::data_dir().map(|d| d.join("loki").join("context_settings.json"))
}

/// Loads the settings: config.toml first, then the saved override file if there is one.
pub fn init(cfg: &crate::config::ContextConfig) {
    let mut s = ContextSettings { autocompact: true, history_fraction: cfg.history_fraction, num_ctx: cfg.num_ctx, max_turns: crate::session_store::SUMMARY_TRIGGER };
    if let Some(saved) = settings_path().and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice::<ContextSettings>(&b).ok()) {
        s = saved;
    }
    let s = s.clamped();
    crate::providers::ollama::set_num_ctx(s.num_ctx);
    *SETTINGS.write().unwrap() = s;
}

pub fn get() -> ContextSettings {
    SETTINGS.read().unwrap().clone()
}

/// Changes any of the settings (absent fields stay as they are), applies them and saves them.
pub fn update(patch: &Value) -> Result<ContextSettings, String> {
    let mut s = get();
    if let Some(v) = patch["autocompact"].as_bool() {
        s.autocompact = v;
    }
    if let Some(v) = patch["history_fraction"].as_f64() {
        s.history_fraction = v as f32;
    }
    if let Some(v) = patch["num_ctx"].as_u64() {
        s.num_ctx = v as usize;
    }
    if let Some(v) = patch["max_turns"].as_u64() {
        s.max_turns = v as usize;
    }
    let s = s.clamped();
    crate::providers::ollama::set_num_ctx(s.num_ctx);
    *SETTINGS.write().unwrap() = s.clone();
    if let Some(p) = settings_path() {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        std::fs::write(p, serde_json::to_vec_pretty(&s).unwrap_or_default()).map_err(|e| e.to_string())?;
    }
    Ok(s)
}

fn measured() -> &'static Mutex<HashMap<String, Value>> {
    static M: OnceLock<Mutex<HashMap<String, Value>>> = OnceLock::new();
    M.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Remembers what the model said the last request cost (`prompt_eval_count`), per conversation.
pub fn record_measured(session_id: &str, info: &Value) {
    let mut v = info.clone();
    v["at"] = json!(crate::conversation_log::now_local());
    measured().lock().unwrap().insert(session_id.to_string(), v);
}

pub fn last_measured(session_id: &str) -> Option<Value> {
    measured().lock().unwrap().get(session_id).cloned()
}

/// Short plain-text form of a report, for `/context` in any UI.
pub fn render(r: &Value) -> String {
    let n = |p: &str| r.pointer(p).and_then(|v| v.as_u64()).unwrap_or(0);
    let pct = |a: u64, b: u64| if b == 0 { 0 } else { (a * 100 / b).min(999) };
    let window = n("/window");
    let used = n("/used");
    let mut out = format!(
        "**Context** — about {} of {} tokens in use ({}%), {} free.\n\n",
        k(used),
        k(window),
        pct(used, window),
        k(window.saturating_sub(used))
    );
    out.push_str(&format!(
        "- System prompt and standing instructions: {}\n- Tool definitions: {}\n- Conversation summary (older turns): {}\n- Recent conversation, word for word: {} ({} turns)\n",
        k(n("/breakdown/system")),
        k(n("/breakdown/tools")),
        k(n("/breakdown/summary")),
        k(n("/breakdown/history")),
        n("/turns/verbatim")
    ));
    if let Some(m) = r.get("last_measured").filter(|v| !v.is_null()) {
        out.push_str(&format!("- Last request, as the model counted it: {} prompt tokens\n", k(m["promptTokens"].as_u64().unwrap_or(0))));
    }
    let on = r.pointer("/autocompact/enabled").and_then(|v| v.as_bool()).unwrap_or(true);
    out.push_str(&format!(
        "\n**Autocompact** is {}. It folds older turns into the summary when the unsummarized part reaches {} tokens or {} turns; right now it is at {} tokens / {} turns ({}% of the way).\n",
        if on { "on" } else { "off" },
        k(n("/autocompact/at_tokens")),
        n("/autocompact/at_turns"),
        k(n("/autocompact/unsummarized_tokens")),
        n("/autocompact/unsummarized_turns"),
        n("/autocompact/percent")
    ));
    out.push_str("\nType `/compact` to compact now. Window size, history share and autocompact are in Settings → Context.");
    out
}

fn k(n: u64) -> String {
    if n >= 10_000 {
        format!("{}k", n / 1000)
    } else if n >= 1000 {
        format!("{:.1}k", n as f64 / 1000.0)
    } else {
        n.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_clamps_and_reports() {
        let s = ContextSettings { autocompact: true, history_fraction: 5.0, num_ctx: 10, max_turns: 1 }.clamped();
        assert_eq!((s.history_fraction, s.num_ctx, s.max_turns), (0.90, 8_192, 6));
        assert_eq!(s.budget_tokens(), (8_192.0 * 0.9) as usize);
    }

    #[test]
    fn render_mentions_the_trigger_and_compact() {
        let r = json!({"window": 262144, "used": 30000, "breakdown": {"system": 4000, "tools": 3000, "summary": 800, "history": 22000}, "turns": {"verbatim": 9},
            "autocompact": {"enabled": true, "at_tokens": 29491, "at_turns": 24, "unsummarized_tokens": 22000, "unsummarized_turns": 9, "percent": 74}});
        let t = render(&r);
        assert!(t.contains("11%") && t.contains("74% of the way") && t.contains("/compact"), "{t}");
    }
}
