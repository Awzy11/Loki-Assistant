//! Content-level backstop for the tool-tier permission model (`permissions.rs`). Tool tier
//! alone can't catch sensitive content riding through an otherwise-unrestricted tool call —
//! `append_content` is `Tier::FreeWrite`, but its `content` argument could still carry a
//! dictated SSN or account number the user asked Loki to save. This module scans tool-call
//! argument text for a fixed set of patterns and returns the matched category so
//! `agent::run_turn` can treat the call as if a `LocalOnly` tool had fired for the rest of
//! the turn (same `local_only_touched` mechanism, no separate enforcement path to drift).
//!
//! Deliberately pattern-matching, not an LLM-based classifier — the 2026-09-01 design
//! decision ("tool tier + a lightweight content classifier") chose this specifically to avoid
//! adding a model call's latency/cost to every tool invocation. Patterns favor false positives
//! over false negatives: this is a safety net, and blocking cloud continuation unnecessarily
//! costs a "switch to local" message, while missing a real leak doesn't. Tune the patterns as
//! real false positives show up in use, rather than trying to get them perfect up front.

use crate::providers::{LlmProvider, Message};
use regex::Regex;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Financial,
    Biometric,
    Credential,
    GovernmentId,
}

impl Category {
    pub fn label(&self) -> &'static str {
        match self {
            Category::Financial => "financial account/card number",
            Category::Biometric => "clinical health reading",
            Category::Credential => "credential or API key",
            Category::GovernmentId => "government ID number (SSN-shaped)",
        }
    }
}

macro_rules! lazy_regex {
    ($name:ident, $pat:expr) => {
        fn $name() -> &'static Regex {
            static RE: OnceLock<Regex> = OnceLock::new();
            RE.get_or_init(|| Regex::new($pat).expect("valid regex"))
        }
    };
}

lazy_regex!(ssn_re, r"\b\d{3}-\d{2}-\d{4}\b");
// 13-16 digit runs, optionally space/dash-grouped — deliberately broad (catches most card
// number shapes without a Luhn check). Loose enough to also catch other long digit runs
// (tracking numbers, some ID formats) — accepted per the false-positive-over-false-negative
// tradeoff above.
lazy_regex!(routing_re, r"(?i)\b(routing|account)\s*(number|#|no\.?)\s*[:#]?\s*\d{4,}");
lazy_regex!(
    credential_re,
    r"(?i)(api[_-]?key|secret[_-]?key|password\s*[:=]|-----BEGIN|sk-ant-[a-zA-Z0-9-]{10,}|sk-[a-zA-Z0-9]{10,})"
);
// Clinical content only: a blood-pressure or glucose *reading*, a stated diagnosis, or a dosage with
// a schedule. Ordinary fitness vocabulary (heart rate, bpm, HR zones, sleep or step summaries) is NOT
// matched — a workout log or a daily note full of those must be able to go to a cloud model — and
// neither is the word "diagnosis" on its own (troubleshooting reports use it constantly).
lazy_regex!(
    biometric_re,
    r"(?i)(\b(?:blood pressure|bp)\b[^\n]{0,25}\b\d{2,3}\s*/\s*\d{2,3}\b|\b(?:blood glucose|blood sugar|glucose|a1c)\b[^\n]{0,25}\b\d{2,3}(?:\.\d)?\s?(?:mg/dl|mmol|%)|\bdiagnosed with\b|\b(?:my|his|her|their|your) diagnosis\b|\b\d+(?:\.\d+)?\s?(?:mg|mcg|µg)\b[^.\n]{0,25}\b(?:daily|once|twice|per day|a day|bid|tid|at bedtime)\b)"
);
lazy_regex!(card_candidate_re, r"\b(?:\d[ -]?){13,19}\b");
lazy_regex!(date_re, r"\b(?:19|20)\d{2}[-/ ]\d{2}[-/ ]\d{2}\b");

fn luhn_ok(digits: &str) -> bool {
    let mut sum = 0;
    for (i, c) in digits.chars().rev().enumerate() {
        let Some(mut d) = c.to_digit(10) else { return false };
        if i % 2 == 1 {
            d *= 2;
            if d > 9 {
                d -= 9;
            }
        }
        sum += d;
    }
    sum % 10 == 0
}

/// A payment-card-shaped number: 13–19 digits (spaces/dashes allowed) that pass the Luhn check and
/// aren't a run of dates. Plain digit runs — tracking numbers, repeated set weights, consecutive
/// dates — no longer count.
fn looks_like_card(text: &str) -> bool {
    card_candidate_re().find_iter(text).any(|m| {
        let raw = m.as_str();
        let digits: String = raw.chars().filter(|c| c.is_ascii_digit()).collect();
        (13..=19).contains(&digits.len()) && !date_re().is_match(raw) && luhn_ok(&digits)
    })
}

/// Scans `text` for any recognized sensitive pattern, returning the first category matched.
/// Checked in a fixed priority order — credentials and government IDs first, since those
/// patterns are the highest-confidence and least prone to false positives; financial and
/// biometric (broader, more false-positive-prone) checked after.
pub fn classify(text: &str) -> Option<Category> {
    if credential_re().is_match(text) {
        return Some(Category::Credential);
    }
    if ssn_re().is_match(text) {
        return Some(Category::GovernmentId);
    }
    if routing_re().is_match(text) || looks_like_card(text) {
        return Some(Category::Financial);
    }
    if biometric_re().is_match(text) {
        return Some(Category::Biometric);
    }
    None
}

const PRE_TURN_SYSTEM_PROMPT: &str = r#"You are a strict privacy gate for a personal assistant. You will be shown one message a
user is about to send to a cloud-based AI. Reply with exactly one word — LOCAL or CLOUD — and
nothing else.

Reply LOCAL only if the message contains, or asks the assistant to record or handle, one of:
financial account or card numbers, government ID numbers (SSN or similar), API keys, passwords or
other credentials, or clinical health information (medical diagnoses, prescriptions and dosages,
lab results, blood-pressure or glucose readings).

Reply CLOUD for everything else. That includes workout and training logs (exercises, sets, reps,
weights, times, distances), fitness or sleep summaries and averages (including average heart rate),
step and calorie counts, daily notes, schedules, tasks, project and engineering discussion
(including words like "diagnosis" or "troubleshooting"), and general questions.

Reply with only the single word LOCAL or CLOUD."#;

/// Reads the privacy gate's reply: the first word, ignoring case, punctuation and any leading
/// reasoning block. `Some(true)` = LOCAL, `Some(false)` = CLOUD, `None` = unclear.
fn parse_verdict(reply: &str) -> Option<bool> {
    let text = match reply.rfind("</think>") {
        Some(i) => &reply[i + "</think>".len()..],
        None => reply,
    };
    let word: String = text.trim().chars().skip_while(|c| !c.is_alphabetic()).take_while(|c| c.is_alphabetic()).collect();
    match word.to_ascii_uppercase().as_str() {
        "LOCAL" => Some(true),
        "CLOUD" => Some(false),
        _ => None,
    }
}

/// Credentials and ID numbers: never sent to a cloud model, not even when the user overrides the
/// privacy gate for a message with `/cloud`.
pub fn hard_category(text: &str) -> Option<Category> {
    match classify(text) {
        Some(c @ (Category::Credential | Category::GovernmentId)) => Some(c),
        _ => None,
    }
}

/// Pre-turn privacy gate: should this message stay on this machine, and if so, why? Returns the
/// reason (shown to the user — a flagged message must never be silent), or `None` if it may go to a
/// cloud model. Layered, cheapest-first:
/// 1. The regex `classify()` — near-zero cost, catches clear matches instantly.
/// 2. If that's clean, one `chat_low_effort` call to the *local* model (deliberately the
///    caller-supplied `local_llm`, so the check actually runs on-machine) with a strict LOCAL/CLOUD
///    question, for sensitive content described in prose with no matching shape.
///
/// An unclear answer or a failed call still keeps the message local (a leak costs more than an
/// extra local answer), but now says so and can be overridden with `/cloud`.
pub async fn assess(local_llm: &dyn LlmProvider, text: &str) -> Option<String> {
    if let Some(category) = classify(text) {
        return Some(format!("it contains a {}", category.label()));
    }
    if text.trim().is_empty() {
        return None;
    }
    let messages = [Message::system(PRE_TURN_SYSTEM_PROMPT), Message::user(text)];
    match local_llm.chat_low_effort(&messages).await {
        Ok(reply) => match parse_verdict(&reply.content) {
            Some(false) => None,
            Some(true) => {
                eprintln!("[loki-debug] privacy gate: model verdict LOCAL");
                Some("the privacy check judged it to contain financial, ID, credential or clinical health content".to_string())
            }
            None => {
                eprintln!("[loki-debug] privacy gate: unclear verdict {:?}", reply.content.chars().take(80).collect::<String>());
                Some("the privacy check gave an unclear answer".to_string())
            }
        },
        Err(e) => {
            eprintln!("[loki-debug] pre-turn local classifier call failed, staying local: {e}");
            Some("the privacy check couldn't run".to_string())
        }
    }
}

pub async fn should_stay_local(local_llm: &dyn LlmProvider, text: &str) -> bool {
    assess(local_llm, text).await.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_ssn() {
        assert_eq!(classify("his SSN is 123-45-6789"), Some(Category::GovernmentId));
    }

    #[test]
    fn flags_credit_card() {
        assert_eq!(classify("card 4111 1111 1111 1111 expires soon"), Some(Category::Financial));
    }

    #[test]
    fn flags_routing_number() {
        assert_eq!(classify("routing number: 021000021"), Some(Category::Financial));
    }

    #[test]
    fn flags_api_key() {
        assert_eq!(classify("here is my api_key: abc123xyzsecretvalue"), Some(Category::Credential));
    }

    #[test]
    fn flags_biometric() {
        assert_eq!(classify("blood pressure was 140/90 today"), Some(Category::Biometric));
    }

    #[test]
    fn workout_logs_are_not_biometric() {
        let log = "Pull day. Warm-up 10 min. Rows 5x5 @ 185, avg heart rate 142 bpm, max HR 171. Zone 2 run 45:00 at 138 bpm, 4.1 mi. Sleep 7h12.";
        assert_eq!(classify(log), None);
    }

    #[test]
    fn consecutive_dates_and_set_weights_are_not_card_numbers() {
        assert_eq!(classify("Notes for 2026-10-09 2026-10-10 2026-10-11 and the W41 review"), None);
        assert_eq!(classify("Squat 225 225 225 225 225 225 225 225"), None);
        assert_eq!(classify("tracking 1234567890123456 shipped"), None); // fails Luhn
    }

    #[test]
    fn the_word_diagnosis_alone_is_fine_but_clinical_statements_are_not() {
        assert_eq!(classify("Troubleshooting report: diagnosis and minimal fixes"), None);
        assert_eq!(classify("he was diagnosed with hypertension"), Some(Category::Biometric));
        assert_eq!(classify("take 20 mg once daily"), Some(Category::Biometric));
        assert_eq!(classify("fasting glucose 105 mg/dL"), Some(Category::Biometric));
    }

    #[test]
    fn verdict_parsing_tolerates_punctuation_case_and_extra_words() {
        assert_eq!(parse_verdict("CLOUD"), Some(false));
        assert_eq!(parse_verdict("cloud."), Some(false));
        assert_eq!(parse_verdict("  Cloud\n\nNo sensitive data."), Some(false));
        assert_eq!(parse_verdict("<think>hmm</think>LOCAL"), Some(true));
        assert_eq!(parse_verdict("**LOCAL**"), Some(true));
        assert_eq!(parse_verdict("I think it's fine"), None);
        assert_eq!(parse_verdict(""), None);
    }

    #[test]
    fn hard_categories_are_credentials_and_ids_only() {
        assert!(hard_category("api_key: abc123xyzsecretvalue").is_some());
        assert!(hard_category("SSN 123-45-6789").is_some());
        assert!(hard_category("blood pressure was 140/90").is_none());
        assert!(hard_category("card 4111 1111 1111 1111").is_none());
    }

    #[test]
    fn ignores_ordinary_content() {
        assert_eq!(classify("today's three tasks are laundry, gym, and reading"), None);
    }

    #[test]
    fn credential_takes_priority_over_financial() {
        // A string that could plausibly match both — credential should win per the fixed
        // priority order (higher confidence, checked first).
        assert_eq!(classify("api_key=4111111111111111"), Some(Category::Credential));
    }

    use crate::providers::ToolDef;
    use async_trait::async_trait;

    /// A stub local model whose `chat_low_effort` always answers with a fixed word — enough
    /// to exercise `should_stay_local`'s branching without a real Ollama instance.
    struct StubLocalModel(&'static str);

    #[async_trait]
    impl LlmProvider for StubLocalModel {
        async fn chat(&self, _messages: &[Message], _tools: &[ToolDef]) -> anyhow::Result<Message> {
            Ok(Message::assistant(self.0))
        }
        async fn chat_low_effort(&self, _messages: &[Message]) -> anyhow::Result<Message> {
            Ok(Message::assistant(self.0))
        }
        fn is_cloud(&self) -> bool {
            false
        }
        fn name(&self) -> &str {
            "stub"
        }
    }

    struct FailingLocalModel;

    #[async_trait]
    impl LlmProvider for FailingLocalModel {
        async fn chat(&self, _messages: &[Message], _tools: &[ToolDef]) -> anyhow::Result<Message> {
            anyhow::bail!("unreachable in this test")
        }
        async fn chat_low_effort(&self, _messages: &[Message]) -> anyhow::Result<Message> {
            anyhow::bail!("simulated local model failure")
        }
        fn is_cloud(&self) -> bool {
            false
        }
        fn name(&self) -> &str {
            "failing-stub"
        }
    }

    #[tokio::test]
    async fn regex_match_skips_the_local_model_call_entirely() {
        // A model that would answer CLOUD, but the regex should short-circuit before it's
        // ever consulted — proven by the fact the answer is still `true` (stay local).
        let model = StubLocalModel("CLOUD");
        assert!(should_stay_local(&model, "his SSN is 123-45-6789").await);
    }

    #[tokio::test]
    async fn local_model_cloud_verdict_is_trusted_when_regex_is_clean() {
        let model = StubLocalModel("CLOUD");
        assert!(!should_stay_local(&model, "what's on my calendar tomorrow?").await);
    }

    #[tokio::test]
    async fn local_model_local_verdict_is_honored() {
        let model = StubLocalModel("LOCAL");
        assert!(should_stay_local(&model, "he was diagnosed with something serious last week").await);
    }

    #[tokio::test]
    async fn a_failed_local_model_call_fails_closed_to_local() {
        assert!(should_stay_local(&FailingLocalModel, "what's the weather like").await);
    }
}
