use crate::providers::{LlmProvider, Message};
use std::sync::Arc;
use std::time::Duration;

/// Best-effort auto-tagging via a cheap chat call — 3-5 lowercase topic tags, mirroring
/// Muninn's pattern. Reuses Muninn's own postmortem fix rather than repeating its mistake:
/// a too-short timeout there silently produced zero tags for every document until raised to
/// 120s. Never blocks ingest on failure — any error (timeout, bad output, whatever) just
/// yields no auto-tags, logged, not propagated.
const TAG_TIMEOUT_SECS: u64 = 120;
const MAX_CONTENT_CHARS: usize = 3000;

pub async fn auto_tag(llm: &Arc<dyn LlmProvider>, content: &str) -> Vec<String> {
    let excerpt: String = content.chars().take(MAX_CONTENT_CHARS).collect();
    let prompt = format!(
        "Suggest 3-5 short, lowercase topic tags for the following text. Respond with ONLY a \
         comma-separated list of tags, nothing else — no explanation, no numbering.\n\n{excerpt}"
    );
    let messages = vec![Message::user(prompt)];

    match tokio::time::timeout(Duration::from_secs(TAG_TIMEOUT_SECS), llm.chat_low_effort(&messages)).await {
        Ok(Ok(reply)) => parse_tags(&reply.content),
        Ok(Err(e)) => {
            eprintln!("[loki-debug] auto_tag: chat call failed, skipping tags: {e}");
            Vec::new()
        }
        Err(_) => {
            eprintln!("[loki-debug] auto_tag: timed out after {TAG_TIMEOUT_SECS}s, skipping tags");
            Vec::new()
        }
    }
}

fn parse_tags(raw: &str) -> Vec<String> {
    // trim_matches only strips leading/trailing characters, never interior ones — so this
    // doesn't need a hyphen/underscore exception to preserve something like "up-and-coming";
    // only the edges (stray bullets, leading "- ", trailing periods) need stripping.
    raw.split(',')
        .map(|t| t.trim().trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase())
        .filter(|t| !t.is_empty())
        .take(5)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_comma_separated_tags() {
        assert_eq!(parse_tags("finance, budgeting, taxes"), vec!["finance", "budgeting", "taxes"]);
    }

    #[test]
    fn strips_stray_punctuation_and_caps_at_five() {
        assert_eq!(
            parse_tags("- Finance. , Budgeting! , A, B, C, D"),
            vec!["finance", "budgeting", "a", "b", "c"]
        );
    }

    #[test]
    fn empty_input_yields_no_tags() {
        assert!(parse_tags("").is_empty());
    }
}
