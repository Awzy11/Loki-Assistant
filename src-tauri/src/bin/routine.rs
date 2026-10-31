//! `loki-routine`: shows a routine prompt exactly as Loki will send it, with your profile's folder names, headings and properties filled in.
//!
//!     loki-routine bod                 # a command by name (looked up like Loki does: ~/.config/loki/commands, then ~/.claude/commands)
//!     loki-routine path/to/eod.md      # a prompt file
//!     loki-routine bod --blank         # render it against the blank-vault defaults instead of your profile
//!
//! Anything the renderer does not know is left as written and listed at the end, so typos in a placeholder are easy to spot.

use loki_lib::profile::{self, Profile};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let blank = args.iter().any(|a| a == "--blank");
    let Some(target) = args.iter().find(|a| !a.starts_with("--")) else {
        eprintln!("usage: loki-routine <command-name | file.md> [--blank]");
        std::process::exit(2);
    };
    let text = if std::path::Path::new(target).is_file() {
        std::fs::read_to_string(target).unwrap_or_else(|e| fail(&format!("could not read {target}: {e}")))
    } else {
        let dirs = [loki_lib::paths::config_dir().map(|c| c.join("loki").join("commands")), dirs::home_dir().map(|h| h.join(".claude").join("commands"))];
        dirs.iter().flatten().find_map(|d| std::fs::read_to_string(d.join(format!("{target}.md"))).ok()).unwrap_or_else(|| fail(&format!("no command called {target} (and no such file)")))
    };
    let profile = if blank { Profile::default() } else { profile::init().cloned().unwrap_or_else(|e| fail(&e)) };
    let out = profile.render(&text);
    println!("{out}");
    let re = regex::Regex::new(r"\{\{\s*([A-Za-z0-9_.:\-]+)\s*\}\}").unwrap();
    let left: std::collections::BTreeSet<String> = re.captures_iter(&out).map(|c| c[0].to_string()).collect();
    if !left.is_empty() {
        eprintln!("\n--- left as written (not a profile placeholder): {}", left.into_iter().collect::<Vec<_>>().join("  "));
    }
}

fn fail(msg: &str) -> ! {
    eprintln!("loki-routine: {msg}");
    std::process::exit(1);
}
