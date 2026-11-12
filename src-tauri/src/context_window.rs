//! Token-budgeted conversation windowing. Instead of a fixed "last 12 messages", the local
//! model's request gets as many of the newest messages as fit a token budget; whatever falls
//! outside is covered by the rolling summary plus tagged, per-exchange recall from the knowledge
//! store (`Core::offload_exchange`, the `recall_conversation` tool).

use crate::providers::Message;
use crate::router;

#[derive(Debug, Clone, Copy)]
pub struct ContextPolicy {
    /// Tokens of conversation history to send verbatim (a share of the model's context window;
    /// the rest is for the system prompt, tool definitions, tool results and the answer).
    pub budget_tokens: usize,
    /// Conversation turns (see `router::is_turn`) before this index are covered by the summary.
    pub summarized_upto: usize,
}

/// Rough token estimate (~3.2 characters per token, deliberately a bit pessimistic).
pub fn est_tokens(s: &str) -> usize {
    (s.chars().count() * 10).div_ceil(32)
}

pub fn msg_tokens(m: &Message) -> usize {
    let calls = m.tool_calls.as_ref().map(|c| serde_json::to_string(c).map(|s| est_tokens(&s)).unwrap_or(20)).unwrap_or(0);
    est_tokens(&m.content) + calls + 4
}

/// Index into `history` (whose element 0 is the system prompt) where the verbatim window starts:
/// the newest messages that fit `policy.budget_tokens` (never fewer than `min_msgs`), not reaching
/// into turns the summary already covers, and starting on a user message so a tool call is never
/// separated from its result.
pub fn window_start(history: &[Message], policy: &ContextPolicy, min_msgs: usize) -> usize {
    if history.len() <= 1 {
        return history.len();
    }
    // First raw index that is not covered by the summary.
    let mut floor = 1;
    if policy.summarized_upto > 0 {
        let mut turn = 0;
        floor = history.len();
        for (i, m) in history.iter().enumerate().skip(1) {
            if router::is_turn(m) {
                if turn == policy.summarized_upto {
                    floor = i;
                    break;
                }
                turn += 1;
            }
        }
    }
    let mut used = 0;
    let mut start = history.len();
    for i in (floor..history.len()).rev() {
        let t = msg_tokens(&history[i]);
        if used + t > policy.budget_tokens && history.len() - i > min_msgs {
            break;
        }
        used += t;
        start = i;
    }
    // Begin on a user message (look forward; if none, keep the budget-based start).
    if let Some(off) = history[start..].iter().position(|m| m.role == "user") {
        start += off;
    }
    start.max(floor).min(history.len())
}

/// How many leading conversation turns should be folded into the summary now, or `None` if the
/// unsummarized history still fits comfortably. `turn_tokens[i]` is the token estimate of turn i.
/// Triggers at 90% of the budget (or `max_turns` unsummarized turns) and keeps the newest turns
/// worth ≤35% of the budget (at least `min_keep`) verbatim.
/// Tokens each conversation turn really costs when sent verbatim: the turn's own message plus the tool calls and results that follow
/// it before the next turn. `compaction_target` over these weights keeps a tool-heavy transcript in check, which counting only the
/// text of the turns would not.
pub fn turn_weights(history: &[Message]) -> Vec<usize> {
    let mut weights: Vec<usize> = Vec::new();
    for m in history.iter().skip(1) {
        let t = msg_tokens(m);
        if router::is_turn(m) {
            weights.push(t);
        } else if let Some(last) = weights.last_mut() {
            *last += t;
        }
    }
    weights
}

pub fn compaction_target(turn_tokens: &[usize], summarized_upto: usize, budget: usize, max_turns: usize, min_keep: usize) -> Option<usize> {
    let total = turn_tokens.len();
    if total <= summarized_upto {
        return None;
    }
    let unsummarized: usize = turn_tokens[summarized_upto..].iter().sum();
    if unsummarized * 10 <= budget * 9 && total - summarized_upto <= max_turns {
        return None;
    }
    let (mut keep, mut used) = (0, 0);
    for t in turn_tokens.iter().rev() {
        if keep >= min_keep && used + t > budget * 35 / 100 {
            break;
        }
        used += t;
        keep += 1;
    }
    let target = total.saturating_sub(keep);
    (target > summarized_upto).then_some(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_results_count_toward_their_turn() {
        let mut tool = Message::assistant("");
        tool.role = "tool".into();
        tool.content = "r".repeat(3200);
        let h = vec![Message::system("sys"), Message::user("hi"), Message::assistant("calling"), tool, Message::assistant("done"), Message::user("next")];
        let w = turn_weights(&h);
        assert_eq!(w.len(), 4);
        assert!(w[1] > 900, "the assistant turn carries the tool result that followed it: {:?}", w);
        assert!(w[0] < 10 && w[3] < 10);
    }

    fn conv(n: usize, len: usize) -> Vec<Message> {
        let mut h = vec![Message::system("sys")];
        for i in 0..n {
            h.push(Message::user(format!("{i} {}", "q".repeat(len))));
            h.push(Message::assistant(format!("{i} {}", "a".repeat(len))));
        }
        h
    }

    #[test]
    fn everything_fits_starts_at_one() {
        let h = conv(3, 20);
        assert_eq!(window_start(&h, &ContextPolicy { budget_tokens: 10_000, summarized_upto: 0 }, 4), 1);
    }

    #[test]
    fn window_takes_newest_that_fit_and_starts_on_a_user_message() {
        let h = conv(10, 400); // ~130 tokens per message
        let p = ContextPolicy { budget_tokens: 600, summarized_upto: 0 };
        let s = window_start(&h, &p, 2);
        assert!(s > 1);
        assert_eq!(h[s].role, "user");
        assert!(h.len() - s >= 2);
    }

    #[test]
    fn never_reaches_back_into_summarized_turns() {
        let h = conv(10, 20);
        let p = ContextPolicy { budget_tokens: 100_000, summarized_upto: 8 };
        let s = window_start(&h, &p, 2);
        assert_eq!(s, 9); // turn index 8 is raw index 9
    }

    #[test]
    fn min_msgs_wins_over_a_tiny_budget() {
        let h = conv(4, 800);
        let s = window_start(&h, &ContextPolicy { budget_tokens: 1, summarized_upto: 0 }, 4);
        assert!(h.len() - s >= 4);
    }

    #[test]
    fn compaction_waits_until_the_budget_is_nearly_used() {
        assert_eq!(compaction_target(&[100; 10], 0, 5000, 24, 4), None);
        // 40 turns x 300 tokens blows a 6000 budget: keep ≈35% (7 turns), fold the rest.
        let t = compaction_target(&[300; 40], 0, 6000, 24, 4).unwrap();
        assert!(t > 25 && t < 40);
    }

    #[test]
    fn compaction_keeps_at_least_min_keep() {
        let t = compaction_target(&[5000; 30], 0, 6000, 24, 4).unwrap();
        assert_eq!(t, 26);
    }
}
