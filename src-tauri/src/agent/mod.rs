pub mod tools;

use crate::content_classifier;
use crate::context_window::{window_start, ContextPolicy};
use crate::router;
use crate::events::{emit, ChatEvent, EventSink};
use crate::knowledge::QueryFilter;
use crate::permissions::Tier;
use crate::providers::{LlmProvider, Message};
use tools::{ToolCtx, ToolRegistry};

mod pipe;

const SYSTEM_PROMPT_BASE: &str = r#"You are Loki, the user's personal agent. You manage the user's Obsidian vault (notes, tasks and plans), and you also answer everyday questions (a calculation, a fact, anything unrelated to the vault) directly and naturally. You are reached through several front doors: the web/desktop app, a sidebar on the user's desktop, and a voice-note webhook.

Tools: vault tools (list, read, search, append), `semantic_search_vault` (meaning-based) and `simple_search` (exact keywords), `delete_file` (the user approves every delete; if they decline, don't retry), `device_*` tools that act on the user's own computer through the desktop app (reads run at once; writes, edits and commands ask the user; they error in a browser tab), `read_guidance` and `read_memory` (load parts of the user's standing instructions and memory that are only indexed below), `read_skill` (load an expert how-to skill from the Skills index below before doing that kind of task), `recall_conversation` (search the older parts of THIS conversation that no longer fit your context — use it before saying you don't remember something), `remote_run_command` (run a command on one of the user's other machines, like a Linux laptop, over SSH — the user approves every command), and `search_tools` + `call_tool` for dozens more tools on MCP servers (GitHub, Mealie, n8n, Miniflux, Zotero, OSINT, Odin, weather, web fetch...). To pass one tool's long output (a transcript, a page) into another tool, never retype it: give the string \"{{result:tool_name.field}}\" as the argument and Loki substitutes the full text. When a request needs something you don't have directly, `search_tools` first, then `call_tool` with the full name. Tools outside the pre-approved set ask the user; if he declines, say so and stop.

Work efficiently: after 2-3 tool calls you usually have enough — synthesize and answer rather than searching indefinitely, and say plainly what's partial. Be direct and concise; briefly say what you're doing when you use a tool. If a task needs several coordinated vault edits, go step by step.

Named routines (EOD, BOD, BOW, EOW, …): if the user's message already contains the routine's instructions, follow them in order without exploring beyond them; if they only name the routine, call `read_guidance` for that routine's section first. Batch your reads — request several files in one round. `read_file` takes vault paths only; memory file names go to `read_memory`, never `read_file`. Plan your tool rounds: finish the writes before you run out. You answer without a hidden reasoning phase, so for anything that needs careful multi-step reasoning, call the `sequentialthinking` tool (find it with `search_tools`) and think there step by step.

Use the current date and time given at the end of the user's latest message. Daily notes can exist for future dates, so never infer today's date from the file list.

For planning or decision requests (picking priorities, defining criteria), ask the user one or two questions or lay out options first, and don't write to the vault until they confirm. Never edit a note in the same turn you propose its content.

Some tools return local-only content that must never leave this machine. If one was used earlier in the same turn, escalation to a cloud model is refused in code, and a cloud model's turn is stopped rather than continued; answer from what you have."#;

/// Who the user is, from the profile (empty when `[person]` is not filled in).
fn person_block() -> String {
    let p = crate::profile::get();
    if p.person.name.trim().is_empty() && p.person.about.trim().is_empty() {
        return String::new();
    }
    format!("\n\nThe user is {}.", p.person_intro())
}

const KNOWLEDGE_STORE_NOTE: &str = "\n\nOlder parts of a long conversation may appear as a short retrieved summary rather than the full text; if you need the exact original wording, ask the user.";

/// Minimal parameterization (Phase 2) — was a single static const through Phase 0/1. The
/// only thing that varies today is whether to mention the knowledge-store recall behavior,
/// which is always true now that Phase 2 exists, so this is currently a fixed concat rather
/// than a real template — deliberately not over-built ahead of Phase 3/4/5 actually adding
/// more describable sources (imported/connected PKMS content).
/// Removes model-internal artefacts from a final answer: reasoning that leaked ahead of a
/// `</think>`, and raw `<tool_call>` markup the model wrote as text (it cannot run once the
/// round limit has withdrawn the tools). An answer that was only tool markup becomes a plain
/// explanation instead of garbage.
#[cfg(test)]
pub(crate) fn clean_reply(raw: &str) -> String {
    clean_reply_flagged(raw).0
}

/// As `clean_reply`, plus whether raw tool-call markup was removed. When prose survives, a note
/// says a tool call was dropped — otherwise the reply reads as if it simply stopped.
pub(crate) fn clean_reply_flagged(raw: &str) -> (String, bool) {
    let mut t = raw.to_string();
    if let Some(i) = t.rfind("</think>") {
        t = t[i + "</think>".len()..].to_string();
    }
    let had_tool_markup = t.contains("<tool_call>");
    while let Some(a) = t.find("<tool_call>") {
        match t[a..].find("</tool_call>") {
            Some(b) => t.replace_range(a..a + b + "</tool_call>".len(), ""),
            None => t.truncate(a),
        }
    }
    let t = t.trim().to_string();
    if had_tool_markup {
        if t.is_empty() {
            return ("I ran out of steps before I could finish that — say \"continue\" and I'll pick up from here.".to_string(), true);
        }
        return (
            format!("{t}\n\n_(I tried to make a tool call after running out of steps, so nothing further was written. Say \"continue\" to pick up, or use Redo with a stronger model.)_"),
            true,
        );
    }
    (if t.is_empty() { raw.trim().to_string() } else { t }, false)
}

const SYSTEM_PROMPT_BASE_HEAD: &str = "You are Loki, the user's personal agent.";

fn system_prompt(local_context: &str) -> String {
    format!("{SYSTEM_PROMPT_BASE}{}{KNOWLEDGE_STORE_NOTE}{local_context}{}", person_block(), crate::lessons::prompt_block())
}


/// Builds what actually gets sent to `llm.chat()` for this turn — distinct from the full
/// `history` `run_turn` returns to `Core` for persistence/display. Keeps the system prompt, the
/// rolling summary, the newest messages that fit the token budget (`context_window`), and — when
/// older messages were left out — the filed, tagged exchanges of this conversation that best match
/// what was just said.
async fn build_context_window(
    llm: &dyn LlmProvider,
    ctx: &ToolCtx<'_>,
    session_id: &str,
    history: &[Message],
    summary: Option<&str>,
    policy: &ContextPolicy,
) -> Vec<Message> {
    // Providers with a big context + prefix cache take the whole transcript while it fits, so
    // the prompt stays an append-only prefix (see `providers::claude_cli`).
    if let Some(budget) = llm.full_history_budget() {
        if history.iter().map(|m| m.content.len()).sum::<usize>() <= budget {
            return history.to_vec();
        }
    }
    let start = window_start(history, policy, MIN_WINDOW_MESSAGES);
    let dropped = start > 1;
    if !dropped && summary.is_none() {
        return history.to_vec();
    }

    let mut context = vec![history[0].clone()];
    if let Some(summary) = summary {
        context.push(Message::system(format!("Summary of this conversation so far (older turns):\n{summary}")));
    }
    if dropped {
        // Older exchanges are filed (tagged, numbered) in the knowledge store: pull the ones that
        // matter for what was just said. Exchanges still inside the window are skipped.
        let latest_user = history[start..].iter().rev().find(|m| m.role == "user").map(|m| m.content.clone()).unwrap_or_default();
        let prev_assistant = history[start..].iter().rev().find(|m| m.role == "assistant").map(|m| m.content.chars().take(300).collect::<String>()).unwrap_or_default();
        let query_text = format!("{latest_user}\n{prev_assistant}");
        let first_in_window = history[1..start].iter().filter(|m| m.role == "user" && router::is_turn(m)).count() + 1;
        let filter = QueryFilter { source_type: Some("chat_exchange".to_string()), session_id: Some(session_id.to_string()) };
        match ctx.knowledge.query(query_text.trim(), 8, filter).await {
            Ok(results) => {
                let mut block = String::from(
                    "Earlier in this conversation (filed exchanges retrieved for this message, not the full transcript — \
                     use recall_conversation for more, or ask the user if you need the exact wording of something that isn't here):\n",
                );
                let mut any = false;
                for r in results {
                    let turn: usize = r.id.rsplit('#').next().and_then(|n| n.parse().ok()).unwrap_or(usize::MAX);
                    if turn >= first_in_window {
                        continue;
                    }
                    any = true;
                    let tags = if r.tags.is_empty() { String::new() } else { format!(" #{}", r.tags.join(" #")) };
                    block.push_str(&format!("[Turn {turn}{tags}] {}\n", r.snippet));
                }
                if any {
                    context.push(Message::system(block));
                }
            }
            Err(e) => eprintln!("[loki-debug] context retrieval failed, continuing on the recent window only: {e}"),
        }
    }
    context.extend(history[start..].iter().cloned());
    context
}

/// Fewest recent messages ever sent verbatim, however small the token budget.
pub(crate) const MIN_WINDOW_MESSAGES: usize = 4;

/// A turn that failed after it had already changed something (a vault write, a command, an approved action). Retrying it on another
/// model would start from the top and could repeat those changes, so the router surfaces this instead of failing over.
#[derive(Debug)]
pub struct FailedAfterWrites {
    pub cause: String,
    pub tools: Vec<String>,
}

impl std::fmt::Display for FailedAfterWrites {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} This turn had already run {} before it failed, so it was not retried on another model (that could repeat those changes). Check what was written, then ask again.", self.cause.trim_end(), self.tools.join(", "))
    }
}

impl std::error::Error for FailedAfterWrites {}

/// Tools that only read can be re-run safely. Anything else (writes, commands, MCP tools that aren't allow-listed as reads) counts as
/// a change once it has been executed, whatever it returned: a failed write may still have landed.
fn changes_things(tier: Tier) -> bool {
    !matches!(tier, Tier::Ambient | Tier::LocalOnly)
}

/// Wraps a mid-turn failure as `FailedAfterWrites` when something already changed; otherwise passes it through for normal failover.
fn after_writes(err: anyhow::Error, changed: &[String]) -> anyhow::Error {
    if changed.is_empty() {
        err
    } else {
        let mut tools = changed.to_vec();
        tools.dedup();
        anyhow::Error::new(FailedAfterWrites { cause: err.to_string(), tools })
    }
}

/// Runs one full turn: takes the conversation so far (already including the new user
/// message), loops through tool calls until the model produces a plain reply, and
/// returns the updated *full* conversation (for persistence/display — never trimmed) plus
/// the final assistant text. What's actually sent to `llm.chat()` each round is a separate,
/// bounded working copy built by `build_context_window` — see its doc comment.
pub async fn run_turn(
    llm: &dyn LlmProvider,
    registry: &ToolRegistry,
    ctx: &ToolCtx<'_>,
    session_id: &str,
    mut history: Vec<Message>,
    local_context: &str,
    summary: Option<&str>,
    sink: Option<&EventSink>,
    max_rounds: usize,
    policy: ContextPolicy,
) -> anyhow::Result<(Vec<Message>, String)> {
    let tools = registry.definitions();

    // Ensure the system prompt is present exactly once, at the front.
    if history.first().map(|m| m.role.as_str()) != Some("system") {
        history.insert(0, Message::system(system_prompt(local_context)));
    } else if history[0].content.starts_with(SYSTEM_PROMPT_BASE_HEAD) {
        // A long-lived conversation picks up newly approved lessons on its next turn.
        const MARK: &str = "\n\nLessons learned from past reviews";
        let base = history[0].content.split(MARK).next().unwrap_or("").to_string();
        history[0].content = format!("{base}{}", crate::lessons::prompt_block());
    }

    let mut context_for_model = build_context_window(llm, ctx, session_id, &history, summary, &policy).await;
    // Small local models weight the end of the context most, so approved lessons are also
    // restated right before the user's latest message (not stored in the transcript).
    let reminder = crate::lessons::reminder_block();
    // The system prompt is built once per conversation, so the date goes on every request.
    let mut suffix = format!("[Current date/time: {}]", crate::conversation_log::now_local_weekday());
    if !reminder.is_empty() {
        suffix = format!("{suffix}\n\n[{}]", reminder.trim());
    }
    if let Some(pos) = context_for_model.iter().rposition(|m| m.role == "user") {
        // Appended to the user turn (this request's copy only): chat templates often ignore
        // system messages after the first, and small models weight the final lines most.
        let m = &mut context_for_model[pos];
        m.content = format!("{}\n\n{suffix}", m.content);
    }

    // Names of every Tier::LocalOnly tool called so far this turn (across all rounds, not
    // just the current one) — see `permissions.rs` / `ToolRegistry::tier_of`. Once non-empty,
    // `escalate_to_claude` is refused for the rest of the turn, regardless of what the model
    // puts in its prompt.
    let mut local_only_touched: Vec<String> = Vec::new();
    // Full (unclipped) results of this turn's tools, newest last, for `{{result:…}}` pipes.
    let mut piped: Vec<(String, String)> = Vec::new();
    // Tools that changed something this turn (see `FailedAfterWrites`).
    let mut changed: Vec<String> = Vec::new();

    for round in 0..max_rounds {
        let is_last_round = round == max_rounds - 1;
        // Two rounds before the cutoff, say so while tools still work — a small model that is told
        // only on the last (tool-less) round tends to write its final tool call as plain text.
        if max_rounds >= 5 && round == max_rounds - 3 {
            let warn = Message::system(
                "Tool-call budget: 2 rounds left. Stop exploring. If this task needs vault writes, make them in \
                 your next round (batch the calls), then give your final answer.",
            );
            history.push(warn.clone());
            context_for_model.push(warn);
        }

        // On the last round, don't offer tools at all — force a synthesized answer from
        // whatever's already been gathered instead of letting the model attempt yet another
        // call and hitting a hard error. Local models are meaningfully more prone to
        // open-ended search sprees than Claude is; degrade gracefully rather than fail.
        let tools_for_round: &[_] = if is_last_round { &[] } else { &tools };
        if is_last_round {
            let budget_msg = Message::system(
                "You're out of tool-call budget for this turn. Do not attempt another tool call — \
                 answer now using only what you've already found, and say plainly if it's incomplete.",
            );
            history.push(budget_msg.clone());
            context_for_model.push(budget_msg);
        }

        // Generalized cloud-escalation guard (Phase 1): Phase 0's guard only covered the
        // `escalate_to_claude` *tool call* — but once BYOK lets the primary chat model itself
        // be cloud-backed, local-only content sitting in history from an earlier round would
        // otherwise get silently forwarded to it on the very next call. Unlike Phase 0's case,
        // there's no separate local model to fall back to here — `llm` IS the configured
        // model — so the correct move is to stop the turn and say why, not to retry locally.
        if llm.is_cloud() {
            if let Some(blocker) = local_only_touched.first() {
                let msg = format!(
                    "I can't continue this turn — local-only content was gathered via '{blocker}' \
                     (Tier: local-only, per the vault's AI Permission Model), and the configured \
                     model ('{}') is a cloud provider. That content must not leave this machine. \
                     Switch to a local model to continue this conversation, or ask something that \
                     doesn't depend on what was just gathered.",
                    llm.name()
                );
                eprintln!("[loki-debug] blocked cloud chat call: local_only_touched={local_only_touched:?}");
                history.push(Message::system(msg.clone()));
                return Ok((history, msg));
            }
        }

        eprintln!(
            "[loki-debug] round {round}: sending {} messages to {} (tools offered: {})",
            context_for_model.len(),
            llm.name(),
            tools_for_round.len()
        );
        let reply = llm
            .chat_stream(&context_for_model, tools_for_round, &mut |t: &str| {
                emit(sink, ChatEvent::TextDelta { text: t.to_string() })
            })
            .await
            .map_err(|e| after_writes(e, &changed))?;
        if let Some(info) = llm.take_usage() {
            if info["kind"] == "context" {
                crate::context_stats::record_measured(session_id, &info);
            } else {
                emit(sink, ChatEvent::Usage { info });
            }
        }
        if !reply.thinking.trim().is_empty() {
            emit(sink, ChatEvent::Thinking { text: reply.thinking.chars().take(6000).collect() });
        }
        eprintln!(
            "[loki-debug] round {round}: got reply, content={:?}, tool_calls={:?}",
            reply.content, reply.tool_calls
        );

        let tool_calls = reply.tool_calls.clone().unwrap_or_default();
        if tool_calls.is_empty() || is_last_round {
            let mut reply = reply;
            let (cleaned, dropped_call) = clean_reply_flagged(&reply.content);
            if dropped_call {
                emit(sink, ChatEvent::Status { message: "The model tried to call a tool after the step limit; that call was dropped.".into() });
            }
            if cleaned != reply.content {
                // Drop what streamed (reasoning scratch, raw tool-call markup) and show the clean text.
                emit(sink, ChatEvent::Reset);
                emit(sink, ChatEvent::TextDelta { text: cleaned.clone() });
                reply.content = cleaned;
            }
            let final_text = reply.content.clone();
            history.push(reply.clone());
            context_for_model.push(reply);
            return Ok((history, final_text));
        }

        history.push(reply.clone());
        context_for_model.push(reply);

        for (call_index, call) in tool_calls.into_iter().enumerate() {
            // The model's own name for the call (kept for the tool-result message it expects)
            // vs. the tool actually being invoked (differs for `call_tool`). Gating, the content
            // classifier, approvals and the UI all use the effective tool and arguments.
            let model_tool_name = call.function.name.clone();
            let (name, shown_args) = registry.resolve(&call.function.name, &call.function.arguments);
            // `{{result:tool}}` placeholders become the earlier tool's full output (server-side, so
            // the model never has to retype long text). The UI keeps the short placeholder form.
            let effective_args = match pipe::substitute(&shown_args, &piped) {
                Ok(v) => v,
                Err(why) => {
                    let msg = format!("error: {why}");
                    emit(sink, ChatEvent::ToolStart { id: format!("local-{round}-{call_index}"), name: name.clone(), input: shown_args.clone() });
                    emit(sink, ChatEvent::ToolResult { id: format!("local-{round}-{call_index}"), content: msg.clone(), is_error: true });
                    let result_msg = Message::tool_result(model_tool_name.clone(), msg);
                    history.push(result_msg.clone());
                    context_for_model.push(result_msg);
                    continue;
                }
            };
            let call_id = format!("local-{round}-{call_index}");
            emit(sink, ChatEvent::ToolStart { id: call_id.clone(), name: name.clone(), input: shown_args.clone() });

            // Hard-block escalation if any local-only tool was touched earlier this turn.
            // Checked before registry.execute() runs, so claude.ask() is never even called —
            // this can't be argued around by what the model puts in the prompt text.
            if name == "escalate_to_claude" {
                if let Some(blocker) = local_only_touched.first() {
                    let msg = format!(
                        "error: escalation refused — this turn already touched local-only content \
                         via '{blocker}' (Tier: local-only, per the vault's AI Permission Model). \
                         That content must not reach a cloud model. Answer using local reasoning \
                         and what you've already gathered instead."
                    );
                    eprintln!("[loki-debug] blocked escalate_to_claude: local_only_touched={local_only_touched:?}");
                    emit(sink, ChatEvent::ToolResult { id: call_id, content: msg.clone(), is_error: true });
                    let result_msg = Message::tool_result(model_tool_name.clone(), msg);
                    history.push(result_msg.clone());
                    context_for_model.push(result_msg);
                    continue;
                }
            } else if registry.tier_of(&name) == Tier::LocalOnly {
                local_only_touched.push(name.clone());
            } else if registry.tier_of(&name) == Tier::FreeWrite {
                // Content-level backstop for the FreeWrite-tier gap (see `content_classifier`
                // doc comment): tool tier alone doesn't catch sensitive content riding through
                // an otherwise-unrestricted call, e.g. `append_content`'s `content` argument.
                // Scans the raw call arguments — cheaper and catches more than scanning only
                // the `content` field by name, and doesn't need per-tool field knowledge here.
                if let Some(category) = content_classifier::classify(&effective_args.to_string()) {
                    let marker = format!("{name} (content classifier: {})", category.label());
                    eprintln!(
                        "[loki-debug] content classifier flagged '{name}' as {category:?} — \
                         treating as local-only for the rest of this turn"
                    );
                    local_only_touched.push(marker);
                }
            }

            // Arguments that can never succeed are rejected before anyone is asked to approve.
            if let Err(why) = registry.validate(&name, &effective_args) {
                let msg = format!("error: {why}");
                emit(sink, ChatEvent::ToolResult { id: call_id, content: msg.clone(), is_error: true });
                let result_msg = Message::tool_result(model_tool_name.clone(), msg);
                history.push(result_msg.clone());
                context_for_model.push(result_msg);
                continue;
            }

            // Tier 2 gate: ask the user before anything that needs confirmation runs. Enforced
            // here, in code — a denied (or unanswerable) request means the tool never executes.
            if registry.tier_of(&name) == Tier::ConfirmRequired
                && !ctx.bridge.request_permission(ctx.session_id, &name, &effective_args).await
            {
                let msg = format!("error: '{name}' was not approved by the user, so it was not run.");
                emit(sink, ChatEvent::ToolResult { id: call_id, content: msg.clone(), is_error: true });
                let result_msg = Message::tool_result(model_tool_name.clone(), msg);
                history.push(result_msg.clone());
                context_for_model.push(result_msg);
                continue;
            }

            // Email is Tier 3: a cloud model can't even trigger a read of it.
            let result = if name.starts_with("mail_") && llm.is_cloud() {
                "error: email is local-only. Switch this conversation to a local model (Ollama) to use it.".to_string()
            } else {
                eprintln!("[loki-debug] executing tool {name} with args {}", if name.starts_with("mail_") { "<redacted>".to_string() } else { format!("{effective_args:?}") });
                if changes_things(registry.tier_of(&name)) {
                    changed.push(name.clone());
                }
                registry.execute(&name, &effective_args, ctx).await
            };
            // Automatic activity log: one line per vault write, in code (best effort, never fails the call).
            if let Some((tool, path)) = crate::activity_log::vault_write(&name, &effective_args) {
                crate::activity_log::record(ctx.obsidian, &tool, &path, !result.starts_with("error:"), &effective_args).await;
            }
            if !name.starts_with("mail_") {
                eprintln!("[loki-debug] tool result: {}", &result[..result.floor_char_boundary(result.len().min(300))]);
            }
            if !result.starts_with("error:") {
                piped.push((name.clone(), result.clone()));
            }
            let result = if name.starts_with("mcp__") { crate::mcp_hub::clip_result(result) } else { result };
            emit(sink, ChatEvent::ToolResult { id: call_id, is_error: result.starts_with("error:"), content: result.clone() });
            let result_msg = Message::tool_result(model_tool_name.clone(), result);
            history.push(result_msg.clone());
            context_for_model.push(result_msg);
        }
    }

    unreachable!("loop always returns on the last round")
}

#[cfg(test)]
mod failover_guard_tests {
    use super::*;

    #[test]
    fn only_reads_are_safe_to_repeat() {
        assert!(!changes_things(Tier::Ambient));
        assert!(!changes_things(Tier::LocalOnly));
        assert!(changes_things(Tier::FreeWrite));
        assert!(changes_things(Tier::ConfirmRequired));
    }

    #[test]
    fn a_failure_before_any_change_still_fails_over() {
        let e = after_writes(anyhow::anyhow!("529 overloaded"), &[]);
        assert!(e.downcast_ref::<FailedAfterWrites>().is_none());
    }

    #[test]
    fn a_failure_after_a_change_names_what_ran() {
        let e = after_writes(anyhow::anyhow!("connection reset"), &["append_content".into(), "append_content".into()]);
        let f = e.downcast_ref::<FailedAfterWrites>().expect("wrapped");
        assert_eq!(f.tools, vec!["append_content".to_string()]);
        assert!(e.to_string().contains("connection reset") && e.to_string().contains("append_content") && e.to_string().contains("not retried"));
    }
}

#[cfg(test)]
mod clean_reply_tests {
    use super::clean_reply;

    #[test]
    fn strips_leaked_reasoning_before_think_close() {
        assert_eq!(clean_reply("Let me count... four?\n</think>\n\nSo long!"), "So long!");
    }

    #[test]
    fn strips_tool_call_markup() {
        let out = clean_reply("Checking.\n<tool_call><function=search_tools></function></tool_call>");
        assert!(out.starts_with("Checking."));
        assert!(!out.contains("<tool_call>"));
    }

    #[test]
    fn prose_plus_dropped_call_gets_a_note() {
        let out = clean_reply("Closing out EOD:\n<tool_call><function=put_content></function></tool_call>");
        assert!(out.starts_with("Closing out EOD:"));
        assert!(out.contains("nothing further was written"));
    }

    #[test]
    fn markup_only_becomes_an_explanation() {
        assert!(clean_reply("<tool_call><function=x>").contains("ran out of steps"));
    }

    #[test]
    fn plain_text_untouched() {
        assert_eq!(clean_reply("Paris."), "Paris.");
    }
}
