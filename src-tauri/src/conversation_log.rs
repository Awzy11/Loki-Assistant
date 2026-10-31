use crate::knowledge::KnowledgeStore;
use crate::obsidian::ObsidianClient;
use crate::providers::Message;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Where Loki's own conversations live in the vault — replaces the old direct-to-SQLite
/// `chat_message` logging (see git history on `core.rs`'s `log_chat_turn`). The vault is now
/// the source of truth for conversation history, same as everything else Loki touches; the
/// SQLite knowledge store remains the *derived search index* over it, populated the same
/// incremental-reingest way `agent::tools`'s `ReadFile`/`AppendContent` already populate it
/// for ordinary vault notes — not a second, hidden record of what was said.
/// The folder conversation logs go in (`[vault] conversations` in the profile).
pub fn conversation_log_dir() -> String {
    crate::profile::get().vault.conversations.clone()
}

/// Local-time formatting, pulled from the machine's own configured timezone via the system
/// `date` command — DST-aware for free (confirmed: this machine is `America/New_York`, so a
/// conversation logged in EDT or EST gets the correct label either way, no hardcoded offset).
/// Deliberately shells out rather than adding a date/time crate (chrono/time) — this codebase
/// has none, `date` already knows the machine's real timezone via `/etc/localtime`, and a
/// conversation-log timestamp is logged once per turn, not a hot path where a subprocess call
/// would matter. `Command` (blocking), not `tokio::process::Command`, even inside the async
/// task below — a few-ms blocking call this infrequent is the same tradeoff this codebase
/// already makes with `std::sync::Mutex` for "brief and rare" synchronous work.
///
/// Falls back to a UTC Unix-time render (`civil_from_days` below) if the `date` subprocess
/// is ever unavailable or fails — logging a conversation turn should never hard-fail over a
/// timestamp format.
pub fn now_local() -> String {
    local_datetime_now()
}

/// Local time with the weekday (`Fri 2026-10-09 19:47 EDT`), for telling the model what day it is.
pub fn now_local_weekday() -> String {
    run_date(&["+%a %Y-%m-%d %H:%M %Z"]).unwrap_or_else(|| format!("{} (UTC fallback)", format_utc_datetime(now_unix_secs())))
}

fn local_datetime_now() -> String {
    run_date(&["+%Y-%m-%d %H:%M:%S %Z"]).unwrap_or_else(|| format!("{} (UTC fallback)", format_utc_datetime(now_unix_secs())))
}

fn local_date_today() -> String {
    run_date(&["+%Y-%m-%d"]).unwrap_or_else(|| format_utc_date(now_unix_secs()))
}

/// `date -d <ISO date> +<fmt>`, e.g. the ISO week of a day (`"%G-W%V"`).
pub fn date_format_for(date: &str, fmt: &str) -> Option<String> {
    run_date(&["-d", date, &format!("+{fmt}")])
}

/// `date -d @<unix> +<fmt>` in the server's local time zone.
pub fn date_format_at(unix: i64, fmt: &str) -> String {
    run_date(&["-d", &format!("@{unix}"), &format!("+{fmt}")]).unwrap_or_default()
}

/// `date +<fmt>` for the server's local time zone (e.g. `"%Y-%m-%d"`); `None` if `date` fails.
pub fn date_format(fmt: &str) -> Option<String> {
    run_date(&[&format!("+{fmt}")])
}

fn run_date(args: &[&str]) -> Option<String> {
    let output = std::process::Command::new("date").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn now_unix_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Howard Hinnant's `civil_from_days` (days since 1970-01-01 -> proleptic Gregorian y/m/d) —
/// small and well-known enough to inline rather than pull in a date/time crate for it. Used
/// only as the UTC fallback if the local `date` shell-out above ever fails.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn format_utc_date(unix_secs: u64) -> String {
    let (y, mo, d) = civil_from_days((unix_secs / 86_400) as i64);
    format!("{y:04}-{mo:02}-{d:02}")
}

fn format_utc_datetime(unix_secs: u64) -> String {
    let (y, mo, d) = civil_from_days((unix_secs / 86_400) as i64);
    let secs_of_day = unix_secs % 86_400;
    let (h, mi, s) = (secs_of_day / 3600, (secs_of_day % 3600) / 60, secs_of_day % 60);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02} UTC")
}

/// Vault path for a session's conversation log — computed once, the first time a turn is
/// logged for that session, then kept on `SessionState` for the rest of the session's life.
///
/// Uses the **full** session_id, not a truncated suffix — the `/api/v1/chat` caller supplies
/// its own arbitrary `session_id` string (it isn't required to be Loki's own generated
/// `session-<unix>-<hex>` shape), so anything less than the full id risks two different
/// caller-chosen ids colliding on the same filename (confirmed the hard way in testing: a
/// custom id like `"my-test-1"` and another ending in `-1` would both truncate to `1.md`).
/// The Claude Code session UUID (Routine Mode's `--resume` handle) is a different id for a
/// different thing and never appears here.
pub fn path_for_session(session_id: &str) -> String {
    format!("{}/{} - {session_id}.md", conversation_log_dir(), local_date_today())
}

fn frontmatter(session_id: &str, routine: Option<&str>) -> String {
    let mut fm = format!("---\ntags: [loki-conversation]\nsession_id: {session_id}\nstarted: {}\n", local_datetime_now());
    if let Some(r) = routine {
        fm.push_str(&format!("routine: {r}\n"));
    }
    fm.push_str("---\n\n");
    fm
}

fn turn_block(role: &str, content: &str) -> String {
    format!("## {role} — {}\n\n{}\n\n", local_datetime_now(), content.trim())
}

/// Strips a front-door-injected context preamble off the start of a user message before it
/// goes into the vault transcript — e.g. the sidebar plugin's `sidebarPreamble`
/// (`LokiSidebar.qml`), which is prepended only to a session's first API call, never shown
/// in the sidebar's own display, and exists purely to steer the model (identity/tool hints),
/// not to be part of the human-readable record of what the user actually typed.
///
/// Matched structurally (`[Context: ...]` followed by a blank line), not against one
/// hardcoded string — any current or future front door that follows the same
/// bracket-plus-blank-line convention gets stripped here too, without this file needing to
/// know each front door's exact wording. Only strips a prefix at the very start of the
/// message, and only the well-formed `]\n\n` shape; a `[Context:` that doesn't close out that
/// way (hand-typed by the user, say) is left alone rather than guessed at.
pub fn strip_preamble(message: &str) -> &str {
    const MARKER: &str = "[Context:";
    if !message.starts_with(MARKER) {
        return message;
    }
    match message.find("]\n\n") {
        Some(idx) => message[idx + "]\n\n".len()..].trim_start(),
        None => message,
    }
}

/// Appends one completed exchange (the user's message + Loki's reply) to a session's vault
/// conversation log — creating the file with frontmatter on the very first call for that
/// session, otherwise just appending — then re-ingests the note's full current content into
/// the knowledge store so `semantic_search_vault` can find it, via the same
/// `ingest_note_content` path `agent::tools::spawn_reingest` uses for ordinary vault
/// reads/writes (just with `session_id` set, which that path leaves `None` — see
/// `knowledge::import::vault::ingest_note_content`'s doc comment).
///
/// Fire-and-forget by design, matching the write-through logging this replaced: logged on
/// failure, never blocks or fails the actual reply reaching the caller. Both turns are
/// written in **one** append (one HTTP call), not two independent fire-and-forget writes —
/// unlike the old SQLite logging, two racing writes here could land out of order in a
/// human-readable transcript file, which matters more than it did for a search index.
#[allow(clippy::too_many_arguments)]
pub fn log_conversation_turn(
    obsidian: Arc<ObsidianClient>,
    knowledge: Arc<dyn KnowledgeStore>,
    session_id: String,
    path: String,
    is_first_write: bool,
    routine: Option<String>,
    user_message: String,
    reply: String,
) {
    if user_message.is_empty() && reply.is_empty() {
        return;
    }
    tokio::spawn(async move {
        let mut block = String::new();
        if is_first_write {
            block.push_str(&frontmatter(&session_id, routine.as_deref()));
        }
        if !user_message.is_empty() {
            block.push_str(&turn_block("User", &user_message));
        }
        if !reply.is_empty() {
            block.push_str(&turn_block("Loki", &reply));
        }

        if let Err(e) = obsidian.append_content(&path, &block).await {
            eprintln!("[loki-debug] conversation-log append to '{path}' failed (non-fatal): {e}");
            return;
        }

        match obsidian.read_file(&path).await {
            Ok(full_content) => {
                if let Err(e) =
                    crate::knowledge::import::vault::ingest_note_content(knowledge.as_ref(), &path, &full_content, Some(&session_id))
                        .await
                {
                    eprintln!("[loki-debug] conversation-log reingest of '{path}' failed (non-fatal): {e}");
                }
            }
            Err(e) => {
                eprintln!("[loki-debug] conversation-log re-read of '{path}' failed, skipping reingest (non-fatal): {e}");
            }
        }
    });
}

/// Finds the vault path for a session that already has a conversation log, without knowing
/// what date it started on (a session can span days, so `path_for_session`'s `local_date_today`
/// can't be reused for lookup — only for the first-ever write). Lists `98 - Loki
/// Conversations/` and matches the file whose name ends with `- {session_id}.md`.
pub async fn find_log_path_for_session(obsidian: &ObsidianClient, session_id: &str) -> Option<String> {
    let entries = obsidian.list_files(Some(&conversation_log_dir())).await.ok()?;
    let suffix = format!("- {session_id}.md");
    entries.into_iter().find(|e| e.ends_with(&suffix)).map(|e| format!("{}/{e}", conversation_log_dir()))
}

/// Reads a session's conversation log back into the `Vec<Message>` shape `SessionState.history`
/// expects — restores **display history only**. Deliberately doesn't attempt to restore live
/// Routine Mode state (`SessionState.routine_claude_session`): that's a separate Claude CLI
/// session UUID used for `--resume`, never persisted anywhere (including here), and recovering
/// it is a bigger, separate problem — a process restart already silently drops live routine
/// state today regardless of this function existing. See `Core::ensure_session_seeded`.
pub async fn load_history_from_log(obsidian: &ObsidianClient, path: &str) -> anyhow::Result<Vec<Message>> {
    let content = obsidian.read_file(path).await?;
    Ok(parse_log(&content))
}

fn strip_frontmatter(content: &str) -> &str {
    match content.strip_prefix("---\n") {
        Some(rest) => match rest.find("\n---\n\n") {
            Some(idx) => &rest[idx + "\n---\n\n".len()..],
            None => content,
        },
        None => content,
    }
}

/// Inverse of `turn_block`: splits the log body into one raw block per `## {role} —
/// {datetime}` heading, line-by-line rather than via a fixed-offset split, so it's tolerant of
/// blank lines inside a turn's content.
fn split_turn_blocks(body: &str) -> Vec<String> {
    let mut blocks: Vec<String> = Vec::new();
    for line in body.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            blocks.push(heading.to_string());
        } else if let Some(last) = blocks.last_mut() {
            last.push('\n');
            last.push_str(line);
        }
    }
    blocks
}

/// Exact inverse of `frontmatter` + `turn_block` — parses a conversation log note's full
/// content back into ordered `Message`s. Unknown/malformed headings are skipped rather than
/// erroring, so a log file edited by hand (or a future format tweak) degrades gracefully
/// instead of losing the whole session's display history.
fn parse_log(content: &str) -> Vec<Message> {
    let body = strip_frontmatter(content);
    let mut messages = Vec::new();
    for block in split_turn_blocks(body) {
        let mut parts = block.splitn(2, '\n');
        let heading = parts.next().unwrap_or("");
        let turn_content = parts.next().unwrap_or("").trim();
        let at = heading.split_once(" — ").map(|(_, t)| t.trim().to_string()).filter(|t| !t.is_empty());
        let role = match heading.split(" — ").next().unwrap_or("").trim() {
            "User" => "user",
            "Loki" => "assistant",
            _ => continue,
        };
        if !turn_content.is_empty() {
            messages.push(Message {
                role: role.to_string(),
                content: turn_content.to_string(),
                tool_calls: None,
                tool_name: None,
                images: Vec::new(),
                display: None,
                thinking: String::new(),
                at,
            });
        }
    }
    messages
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_from_days_matches_known_dates() {
        // 1970-01-01 is day 0 by definition.
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        // 2026-08-29 — sanity-checked against `date -d @1788040800 -u`.
        assert_eq!(civil_from_days(1788040800 / 86_400), (2026, 8, 29));
    }

    #[test]
    fn local_datetime_uses_the_real_date_command_not_the_utc_fallback() {
        // Smoke test, not a wall-clock assertion — just confirms the `date` shell-out
        // actually succeeded on this machine rather than silently falling back to UTC.
        let dt = local_datetime_now();
        assert!(!dt.is_empty());
        assert!(!dt.contains("UTC fallback"), "got: {dt}");
    }

    #[test]
    fn parse_log_round_trips_a_written_conversation() {
        // Exercises the exact same writer functions `log_conversation_turn` uses
        // (`frontmatter` + `turn_block`), then parses the result back — catches format drift
        // between the write and read sides going forward.
        let mut written = frontmatter("session-abc", Some("bod"));
        written.push_str(&turn_block("User", "What are today's MITs?"));
        written.push_str(&turn_block("Loki", "Your Today's 3 are:\n1. Finish the report\n2. Gym"));
        written.push_str(&turn_block("User", "Sounds good."));

        let messages = parse_log(&written);
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[0].content, "What are today's MITs?");
        assert_eq!(messages[1].role, "assistant");
        assert_eq!(messages[1].content, "Your Today's 3 are:\n1. Finish the report\n2. Gym");
        assert_eq!(messages[2].role, "user");
        assert_eq!(messages[2].content, "Sounds good.");
    }

    #[test]
    fn parse_log_skips_frontmatter_and_handles_no_routine() {
        let mut written = frontmatter("session-xyz", None);
        written.push_str(&turn_block("User", "hello"));
        let messages = parse_log(&written);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "hello");
    }

    #[test]
    fn strip_preamble_removes_a_leading_context_block() {
        let msg = "[Context: this message is coming from a persistent desktop sidebar panel.]\n\nHello, who are you?";
        assert_eq!(strip_preamble(msg), "Hello, who are you?");
    }

    #[test]
    fn strip_preamble_leaves_ordinary_messages_untouched() {
        assert_eq!(strip_preamble("What are today's MITs?"), "What are today's MITs?");
    }

    #[test]
    fn strip_preamble_leaves_a_malformed_context_block_untouched() {
        // No "]\n\n" close — e.g. the user typed something starting with "[Context:".
        let msg = "[Context: I want to give you some context on this project.";
        assert_eq!(strip_preamble(msg), msg);
    }

    #[test]
    fn path_uses_the_full_session_id_not_a_truncated_suffix() {
        // Regression test: an earlier version truncated to "everything after the last
        // hyphen," which collided for caller-supplied session_ids like these two.
        let a = path_for_session("my-test-1");
        let b = path_for_session("other-1");
        assert_ne!(a, b, "different session_ids must never produce the same path");
        assert!(a.ends_with("my-test-1.md"), "path was: {a}");
        assert!(a.starts_with(&conversation_log_dir()), "path was: {a}");
    }
}
