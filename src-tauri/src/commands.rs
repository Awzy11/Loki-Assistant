//! Slash commands (`/bod`, `/eod`, `/news`, …): markdown files in `~/.config/loki/commands` (Loki's own folder, searched first) or
//! `~/.claude/commands` (the Claude Code folder).
//! Loki expands them itself, so they work on any engine — Claude, Gemini, or a local model —
//! instead of only inside Claude Code's own CLI. The model sees the full instructions; the UI
//! and the vault log keep showing what the user typed (`/bod`).
//!
//! Format (same as Claude Code): plain markdown; `$ARGUMENTS` is replaced by whatever followed
//! the command name, and if the file has no placeholder the arguments are appended.
//! Profile placeholders such as `{{vault.daily}}`, `{{section.tasks}}` and `{{properties:bod}}` are filled in from the user's profile
//! (see `profile.rs`), so one prompt works with any vault layout; anything that isn't ours (`{{result:...}}`) is left alone.

use std::path::PathBuf;

pub struct Expanded {
    pub name: String,
    pub text: String,
}

/// Where commands live, in the order they are searched: a name in Loki's own folder wins over the same name in Claude Code's.
fn commands_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(c) = crate::paths::config_dir() {
        v.push(c.join("loki").join("commands"));
    }
    if let Some(h) = dirs::home_dir() {
        v.push(h.join(".claude").join("commands"));
    }
    v
}

/// Names reserved for Loki's own handling.
const RESERVED: &[&str] = &["exit", "clear", "compact", "context", "needsyou", "trust", "untrust", "trusted", "cloud", "mcp", "steps"];

pub fn expand(message: &str) -> Option<Expanded> {
    commands_dirs().iter().find_map(|d| expand_in(message, d))
}

pub fn expand_in(message: &str, dir: &std::path::Path) -> Option<Expanded> {
    let trimmed = message.trim();
    let rest = trimmed.strip_prefix('/')?;
    let (name, args) = match rest.split_once(char::is_whitespace) {
        Some((n, a)) => (n, a.trim()),
        None => (rest, ""),
    };
    // Looks like a command name (so "/usr/bin/foo" or "/ 3" are not), and isn't reserved.
    let valid = !name.is_empty()
        && name.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
    if !valid || RESERVED.contains(&name) {
        return None;
    }
    let body = crate::profile::get().render(&std::fs::read_to_string(dir.join(format!("{name}.md"))).ok()?);
    let text = if body.contains("$ARGUMENTS") {
        body.replace("$ARGUMENTS", args)
    } else if args.is_empty() {
        body
    } else {
        format!("{body}\n\nARGUMENTS: {args}")
    };
    Some(Expanded { name: name.to_string(), text })
}

#[derive(serde::Serialize, Clone)]
pub struct CommandInfo {
    pub name: String,
    pub description: String,
}

/// Every command Loki can expand, plus its own built-ins, for the `/` suggestion popup.
pub fn list() -> Vec<CommandInfo> {
    let mut out = vec![
        CommandInfo { name: "clear".into(), description: "Start a new conversation".into() },
        CommandInfo { name: "context".into(), description: "Show what fills the context window and how close autocompact is".into() },
        CommandInfo { name: "needsyou".into(), description: "What needs you now: a short ranked list from today's note, the week and Upcoming Events".into() },
        CommandInfo { name: "trust".into(), description: "Trust a folder on this computer or another host: /trust <device|host> <folder> [always|minutes]".into() },
        CommandInfo { name: "untrust".into(), description: "Stop trusting a folder: /untrust <device|host> <folder>".into() },
        CommandInfo { name: "trusted".into(), description: "List the trusted folders".into() },
        CommandInfo { name: "compact".into(), description: "Fold older turns into a summary and file them for recall (frees context)".into() },
        CommandInfo { name: "cloud".into(), description: "Send this one message to a cloud model even if the privacy check held it back: /cloud <message>".into() },
        CommandInfo { name: "exit".into(), description: "Keep this conversation local-only (nothing goes to Claude)".into() },
    ];
    let mut seen = std::collections::HashSet::new();
    for dir in commands_dirs() {
        out.extend(list_in(&dir).into_iter().filter(|c| seen.insert(c.name.clone())));
    }
    out
}

pub fn list_in(dir: &std::path::Path) -> Vec<CommandInfo> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut v: Vec<CommandInfo> = rd
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let path = e.path();
            if path.extension().and_then(|x| x.to_str()) != Some("md") {
                return None;
            }
            let name = path.file_stem()?.to_str()?.to_string();
            let ok = name.chars().next().is_some_and(|c| c.is_ascii_lowercase())
                && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
            if !ok || RESERVED.contains(&name.as_str()) {
                return None;
            }
            Some(CommandInfo { description: describe(&std::fs::read_to_string(&path).ok()?), name })
        })
        .collect();
    v.sort_by(|a, b| a.name.cmp(&b.name));
    v
}

/// Front-matter `description:` if present, else the first non-heading line.
fn describe(body: &str) -> String {
    let mut lines = body.lines();
    if body.starts_with("---") {
        lines.next();
        for l in lines.by_ref() {
            if l.trim() == "---" {
                break;
            }
            if let Some(d) = l.strip_prefix("description:") {
                return d.trim().trim_matches('"').chars().take(140).collect();
            }
        }
    }
    let first = body
        .lines()
        .map(|l| l.trim().trim_start_matches('#').trim())
        .find(|l| !l.is_empty() && !l.starts_with("---"))
        .unwrap_or("");
    // First sentence, capped.
    let sentence = first.split_once(". ").map(|(a, _)| a).unwrap_or(first);
    sentence.trim_end_matches('.').chars().take(110).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir_with(files: &[(&str, &str)]) -> PathBuf {
        let d = std::env::temp_dir().join(format!("loki-cmd-test-{}-{}", std::process::id(), files.len()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        for (n, c) in files {
            std::fs::write(d.join(format!("{n}.md")), c).unwrap();
        }
        d
    }

    #[test]
    fn lists_commands_with_descriptions() {
        let d = dir_with(&[("bod", "Beginning of day standup. Run each morning.\n\n## Steps"), ("fm", "---\ndescription: \"Front matter wins\"\n---\nbody"), ("Bad Name", "x"), ("exit", "reserved")]);
        let l = list_in(&d);
        let names: Vec<_> = l.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["bod", "fm"]);
        assert_eq!(l[0].description, "Beginning of day standup");
        assert_eq!(l[1].description, "Front matter wins");
    }

    #[test]
    fn expands_with_and_without_arguments() {
        let d = dir_with(&[("eod", "Close the day. Date: $ARGUMENTS."), ("bod", "Start the day."), ("lc", "Lesson card walkthrough.")]);
        assert_eq!(expand_in("/eod 2026-10-08", &d).unwrap().text, "Close the day. Date: 2026-10-08.");
        assert_eq!(expand_in("/eod", &d).unwrap().text, "Close the day. Date: .");
        assert_eq!(expand_in("  /bod  ", &d).unwrap().text, "Start the day.");
        // No placeholder: arguments are appended.
        assert_eq!(expand_in("/lc 1101", &d).unwrap().text, "Lesson card walkthrough.\n\nARGUMENTS: 1101");
        assert_eq!(expand_in("/bod", &d).unwrap().name, "bod");
    }

    #[test]
    fn fills_profile_placeholders_and_leaves_other_braces_alone() {
        let d = dir_with(&[("daily-check", "Read {{vault.daily}}/ under {{section.tasks}}. Then use {{result:get_transcript.transcript}} as is.")]);
        assert_eq!(expand_in("/daily-check", &d).unwrap().text, "Read 10 - Daily/ under Tasks. Then use {{result:get_transcript.transcript}} as is.");
    }

    #[test]
    fn shipped_routine_prompts_have_no_unfilled_placeholders() {
        // every prompt in every starter pack must render completely under the pack's own profile and under the blank-vault defaults
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("packs");
        let ph = regex::Regex::new(r"\{\{\s*([A-Za-z0-9_.:\-]+)\s*\}\}").unwrap();
        let mut seen = 0;
        for pack in std::fs::read_dir(&root).unwrap().flatten() {
            let Ok(rd) = std::fs::read_dir(pack.path().join("routines")) else { continue };
            for f in rd.flatten().filter(|f| f.path().extension().and_then(|x| x.to_str()) == Some("md")) {
                let text = std::fs::read_to_string(f.path()).unwrap();
                for (label, p) in [("starter profile", crate::profile::Profile::starter()), ("blank defaults", crate::profile::Profile::default())] {
                    let out = p.render(&text);
                    let left: Vec<String> = ph.captures_iter(&out).map(|c| c[0].to_string()).collect();
                    assert!(left.is_empty(), "{} under {label} still has {left:?}", f.path().display());
                    assert!(![concat!("Quan", "tico"), "MEMORY.md", concat!("mar", "ine")].iter().any(|w| out.to_lowercase().contains(&w.to_lowercase())), "{} contains personal text", f.path().display());
                }
                seen += 1;
            }
        }
        assert!(seen >= 4, "expected the starter pack's BOD, EOD, BOW and EOW prompts, found {seen}");
    }

    #[test]
    fn ignores_non_commands_and_reserved_names() {
        let d = dir_with(&[("exit", "nope"), ("bod", "x")]);
        assert!(expand_in("hello /bod", &d).is_none());
        assert!(expand_in("/usr/bin/env", &d).is_none());
        assert!(expand_in("/exit", &d).is_none()); // Loki's own toggle
        assert!(expand_in("/missing", &d).is_none());
        assert!(expand_in("/../etc/passwd", &d).is_none());
        assert!(expand_in("/", &d).is_none());
    }
}
