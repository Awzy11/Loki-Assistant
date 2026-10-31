//! Automatic "## Loki Activity" logging. After a vault write tool call runs (success or failure),
//! one line is appended under that heading in today's daily note, in code, so it no longer
//! depends on the model remembering. Best effort: any problem is printed and swallowed, and it
//! never fails the original write. Writes that are themselves log entries are not logged again.

use crate::obsidian::ObsidianClient;
use serde_json::Value;

const HEADING: &str = "## Loki Activity";

/// `(short tool name, vault path)` when `name` is a vault write, else `None`. Covers Loki's
/// native tools and the MCP obsidian tools reached through `call_tool`.
pub fn vault_write(name: &str, args: &Value) -> Option<(String, String)> {
    let short = match name {
        "append_content" | "mcp__obsidian__obsidian_append_content" => "append_content",
        "mcp__obsidian__obsidian_patch_content" => "patch_content",
        "mcp__obsidian__obsidian_put_content" => "put_content",
        "delete_file" | "mcp__obsidian__obsidian_delete_file" => "delete_file",
        _ => return None,
    };
    let path = ["filepath", "path"].iter().find_map(|k| args[*k].as_str())?;
    Some((short.to_string(), path.to_string()))
}

fn looks_like_log_line(line: &str) -> bool {
    let t = line.trim_start();
    let b = t.as_bytes();
    t.starts_with("- ") && b.len() >= 7 && b[2].is_ascii_digit() && b[3].is_ascii_digit() && b[4] == b':' && b[5].is_ascii_digit() && b[6].is_ascii_digit()
}

/// True when the write is itself an activity-log entry, so it must not be logged again.
pub fn is_log_write(args: &Value) -> bool {
    let content = args["content"].as_str().unwrap_or_default();
    content.contains(HEADING) || content.lines().any(looks_like_log_line)
}

pub fn format_line(time: &str, tool: &str, path: &str, ok: bool) -> String {
    let path = path.split_whitespace().collect::<Vec<_>>().join(" ");
    let status = if ok { "auto-logged" } else { "FAILED, auto-logged" };
    format!("- {time} — {tool} — {path} — {status}")
}

fn now_local() -> Option<(String, String)> {
    let out = std::process::Command::new("date").arg("+%F %H:%M").output().ok()?;
    let text = String::from_utf8(out.stdout).ok()?;
    let (d, t) = text.trim().split_once(' ')?;
    Some((d.to_string(), t.to_string()))
}

/// Appends one entry for a finished vault write to today's daily note. Skips quietly when the
/// daily note doesn't exist yet (it never creates one).
pub async fn record(obsidian: &ObsidianClient, tool: &str, path: &str, ok: bool, args: &Value) {
    if is_log_write(args) {
        return;
    }
    let Some((date, time)) = now_local() else { return };
    let daily = crate::profile::get().daily_path(&date);
    let existing = match obsidian.read_file(&daily).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[loki-debug] activity log skipped, can't read '{daily}' (non-fatal): {e}");
            return;
        }
    };
    let mut chunk = String::new();
    if !existing.ends_with('\n') {
        chunk.push('\n');
    }
    if !existing.contains(HEADING) {
        chunk.push_str(&format!("\n{HEADING}\n"));
    }
    chunk.push_str(&format_line(&time, tool, path, ok));
    chunk.push('\n');
    if let Err(e) = obsidian.append_content(&daily, &chunk).await {
        eprintln!("[loki-debug] activity log append to '{daily}' failed (non-fatal): {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn recognises_vault_writes() {
        assert_eq!(vault_write("append_content", &json!({"path": "a.md", "content": "x"})), Some(("append_content".to_string(), "a.md".to_string())));
        assert_eq!(vault_write("mcp__obsidian__obsidian_put_content", &json!({"filepath": "b.md"})), Some(("put_content".to_string(), "b.md".to_string())));
        assert!(vault_write("read_file", &json!({"path": "a.md"})).is_none());
        assert!(vault_write("remote_run_command", &json!({"command": "ls"})).is_none());
    }

    #[test]
    fn skips_its_own_log_lines() {
        assert!(is_log_write(&json!({"content": "\n- 22:44 — put_content — x — y\n"})));
        assert!(is_log_write(&json!({"content": "## Loki Activity\n"})));
        assert!(!is_log_write(&json!({"content": "just a note"})));
    }

    #[test]
    fn formats_one_line() {
        assert_eq!(format_line("22:44", "put_content", "a/b.md", true), "- 22:44 — put_content — a/b.md — auto-logged");
        let failed = format_line("22:44", "delete_file", "a\nb.md", false);
        assert!(failed.contains("FAILED"));
        assert!(!failed.contains('\n'));
    }
}
