//! File and shell access on the machine the Loki desktop app runs on. The server never touches
//! this machine directly: a `device_request` event arrives in the webview, the page calls the
//! one Tauri command (`lib.rs::device_call`), and the answer goes back over the bridge.
//!
//! Defense in depth, because the page that can invoke this is served by `loki-server` over the
//! network: approval prompts live in the UI, but this module also enforces what the local
//! `config.toml`'s `[desktop]` table allows (writes on by default, shell OFF by default), and
//! refuses a fixed list of credential locations whatever the UI says.

use crate::config::DesktopConfig;
use serde_json::Value;
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const MAX_READ_BYTES: u64 = 5 * 1024 * 1024;
const DEFAULT_READ_LINES: usize = 2000;
const MAX_OUTPUT_CHARS: usize = 30_000;

/// Directories (relative to `$HOME`) never readable or writable through the bridge.
const PROTECTED: &[&str] = &[
    ".ssh",
    ".gnupg",
    ".aws",
    ".config/loki",
    ".local/share/loki",
    ".password-store",
    ".mozilla",
    ".config/chromium",
    ".config/google-chrome",
    ".config/BraveSoftware",
    ".config/Bitwarden",
    ".claude/.credentials.json",
    ".git-credentials",
    ".netrc",
];

fn home() -> Result<PathBuf, String> {
    dirs::home_dir().ok_or_else(|| "couldn't determine the home directory".to_string())
}

/// `~`, relative-to-home and absolute paths all accepted.
fn resolve(raw: &str, home: &Path) -> PathBuf {
    let raw = raw.trim();
    if raw == "~" {
        return home.to_path_buf();
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        return home.join(rest);
    }
    let p = Path::new(raw);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        home.join(p)
    }
}

/// Resolves `.` and `..` lexically. The result (not the caller's raw string) is what gets
/// checked *and* what gets accessed, so the two can never disagree.
fn normalize(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Canonicalizes `path` (or, if it doesn't exist yet, its nearest existing ancestor plus the
/// remainder) so symlinks and `..` can't walk around the protected list.
fn canonical_lossy(path: &Path) -> PathBuf {
    let mut missing: Vec<std::ffi::OsString> = Vec::new();
    let mut cur = path.to_path_buf();
    loop {
        if let Ok(c) = cur.canonicalize() {
            return missing.iter().rev().fold(c, |acc, part| acc.join(part));
        }
        match (cur.file_name().map(|n| n.to_os_string()), cur.parent().map(|p| p.to_path_buf())) {
            (Some(name), Some(parent)) => {
                missing.push(name);
                cur = parent;
            }
            _ => return path.to_path_buf(),
        }
    }
}

fn check_path(raw: &str, home: &Path) -> Result<PathBuf, String> {
    let path = canonical_lossy(&normalize(&resolve(raw, home)));
    let home_c = canonical_lossy(home);
    for protected in PROTECTED {
        if path.starts_with(home_c.join(protected)) {
            return Err(format!("access to ~/{protected} is blocked by Loki's device policy"));
        }
    }
    Ok(path)
}

fn str_arg<'a>(input: &'a Value, key: &str) -> Result<&'a str, String> {
    input[key].as_str().ok_or_else(|| format!("missing required string argument '{key}'"))
}

fn truncate(mut s: String) -> String {
    if s.chars().count() > MAX_OUTPUT_CHARS {
        s = s.chars().take(MAX_OUTPUT_CHARS).collect::<String>() + "\n… [output truncated]";
    }
    s
}

pub fn read_file(path: &Path, offset: Option<u64>, limit: Option<u64>) -> Result<String, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !meta.is_file() {
        return Err(format!("{} is not a file", path.display()));
    }
    if meta.len() > MAX_READ_BYTES {
        return Err(format!("{} is {} bytes; too large to read (limit {MAX_READ_BYTES})", path.display(), meta.len()));
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let text = String::from_utf8(bytes).map_err(|_| format!("{} is not UTF-8 text", path.display()))?;
    let start = offset.unwrap_or(1).max(1) as usize - 1;
    let take = limit.map(|l| l as usize).unwrap_or(DEFAULT_READ_LINES);
    let lines: Vec<String> =
        text.lines().enumerate().skip(start).take(take).map(|(i, l)| format!("{}\t{l}", i + 1)).collect();
    Ok(truncate(lines.join("\n")))
}

pub fn list_dir(path: &Path) -> Result<String, String> {
    let mut names: Vec<String> = std::fs::read_dir(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .filter_map(|e| e.ok())
        .map(|e| {
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            format!("{}{}", e.file_name().to_string_lossy(), if is_dir { "/" } else { "" })
        })
        .collect();
    names.sort();
    Ok(truncate(names.join("\n")))
}

pub fn write_file(path: &Path, content: &str) -> Result<String, String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    std::fs::write(path, content).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(format!("wrote {} bytes to {}", content.len(), path.display()))
}

pub fn edit_file(path: &Path, old: &str, new: &str, replace_all: bool) -> Result<String, String> {
    if old.is_empty() {
        return Err("old_string must not be empty".to_string());
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let count = text.matches(old).count();
    match (count, replace_all) {
        (0, _) => Err("old_string was not found in the file".to_string()),
        (n, false) if n > 1 => Err(format!("old_string matches {n} places; add more context or set replace_all")),
        _ => {
            let updated = if replace_all { text.replace(old, new) } else { text.replacen(old, new, 1) };
            std::fs::write(path, updated).map_err(|e| e.to_string())?;
            Ok(format!("edited {} ({count} replacement{})", path.display(), if count == 1 { "" } else { "s" }))
        }
    }
}

/// How long output pipes get to close after the shell itself has exited.
const PIPE_GRACE: Duration = Duration::from_secs(2);

/// Reads a pipe to its end on a thread, appending into the returned buffer; the receiver gets one message when the pipe closes.
fn drain(mut pipe: impl Read + Send + 'static) -> (Arc<Mutex<Vec<u8>>>, std::sync::mpsc::Receiver<()>) {
    let buf = Arc::new(Mutex::new(Vec::new()));
    let (tx, rx) = std::sync::mpsc::channel();
    let shared = buf.clone();
    std::thread::spawn(move || {
        let mut chunk = [0u8; 8192];
        while let Ok(n) = pipe.read(&mut chunk) {
            if n == 0 {
                break;
            }
            shared.lock().unwrap().extend_from_slice(&chunk[..n]);
        }
        let _ = tx.send(());
    });
    (buf, rx)
}

pub fn run_command(command: &str, cwd: &Path, timeout_secs: u64) -> Result<String, String> {
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0) // own group, so a timeout can kill the command's children too
        .spawn()
        .map_err(|e| format!("couldn't start shell: {e}"))?;
    // Drain pipes on threads so a chatty command can't block on a full pipe. Each thread appends into a shared buffer and signals when its
    // pipe closes, so the output read so far is still available if we have to stop waiting.
    let (out_buf, out_done) = drain(child.stdout.take().unwrap());
    let (err_buf, err_done) = drain(child.stderr.take().unwrap());
    let group = format!("-{}", child.id());
    let kill_group = || {
        // Some shells (dash) don't exec a lone command, and a command can leave background children behind; both keep the pipes open.
        let _ = Command::new("kill").args(["-KILL", "--", &group]).status();
    };
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    let status = loop {
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(st) => break Some(st),
            None if Instant::now() >= deadline => {
                kill_group();
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            None => std::thread::sleep(Duration::from_millis(25)),
        }
    };
    // The shell is gone, but a background child (`npm run dev &`, `sleep 600 &`) can still hold the pipes open. Give them a short grace
    // period to close, then stop those children and return what was read, rather than waiting on them indefinitely.
    let grace = if status.is_some() { PIPE_GRACE.min(deadline.saturating_duration_since(Instant::now())) } else { PIPE_GRACE };
    let wait_until = Instant::now() + grace;
    let closed = |rx: &std::sync::mpsc::Receiver<()>| rx.recv_timeout(wait_until.saturating_duration_since(Instant::now())).is_ok();
    let both_closed = closed(&out_done) & closed(&err_done);
    let mut note = String::new();
    if !both_closed {
        kill_group();
        let _ = out_done.recv_timeout(Duration::from_secs(1));
        let _ = err_done.recv_timeout(Duration::from_secs(1));
        if status.is_some() {
            note = "\n[a background process was still running when the command ended; it was stopped]".to_string();
        }
    }
    let text_of = |buf: &Arc<Mutex<Vec<u8>>>| String::from_utf8_lossy(&buf.lock().unwrap()).into_owned();
    let mut text = text_of(&out_buf);
    let stderr = text_of(&err_buf);
    if !stderr.is_empty() {
        text.push_str(&format!("\n[stderr]\n{stderr}"));
    }
    text.push_str(&note);
    match status {
        Some(st) if st.success() => Ok(truncate(text)),
        Some(st) => Err(truncate(format!("exit status {}\n{text}", st.code().map_or("signal".into(), |c| c.to_string())))),
        None => Err(truncate(format!("timed out after {timeout_secs}s and was killed\n{text}"))),
    }
}

/// Entry point for the Tauri command: dispatches one `device_*` tool call, enforcing `cfg`.
pub fn call(name: &str, input: &Value, cfg: &DesktopConfig) -> Result<String, String> {
    let home = home()?;
    match name {
        "device_read_file" => {
            let p = check_path(str_arg(input, "path")?, &home)?;
            read_file(&p, input["offset"].as_u64(), input["limit"].as_u64())
        }
        "device_list_dir" => list_dir(&check_path(str_arg(input, "path")?, &home)?),
        "device_write_file" | "device_edit_file" if !cfg.device_write => {
            Err("file writes are disabled: set device_write = true in the [desktop] table of ~/.config/loki/config.toml".into())
        }
        "device_write_file" => write_file(&check_path(str_arg(input, "path")?, &home)?, str_arg(input, "content")?),
        "device_edit_file" => edit_file(
            &check_path(str_arg(input, "path")?, &home)?,
            str_arg(input, "old_string")?,
            str_arg(input, "new_string")?,
            input["replace_all"].as_bool().unwrap_or(false),
        ),
        "device_run_command" if !cfg.device_shell => Err(
            "shell commands are disabled on this device: set device_shell = true in the [desktop] table of ~/.config/loki/config.toml to allow them"
                .into(),
        ),
        "device_run_command" => {
            let cwd = match input["cwd"].as_str() {
                Some(c) => check_path(c, &home)?,
                None => home.clone(),
            };
            let timeout = input["timeout_secs"].as_u64().unwrap_or(60).clamp(1, 300);
            run_command(str_arg(input, "command")?, &cwd, timeout)
        }
        other => Err(format!("unknown device tool '{other}'")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("loki-device-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn cfg(write: bool, shell: bool) -> DesktopConfig {
        DesktopConfig { server_url: String::new(), device_write: write, device_shell: shell }
    }

    #[test]
    fn read_numbers_lines_and_honors_offset_limit() {
        let d = tmp("read");
        let f = d.join("a.txt");
        std::fs::write(&f, "one\ntwo\nthree\n").unwrap();
        assert_eq!(read_file(&f, None, None).unwrap(), "1\tone\n2\ttwo\n3\tthree");
        assert_eq!(read_file(&f, Some(2), Some(1)).unwrap(), "2\ttwo");
    }

    #[test]
    fn read_rejects_binary() {
        let d = tmp("bin");
        let f = d.join("b.bin");
        std::fs::write(&f, [0xff, 0xfe, 0x00, 0x80]).unwrap();
        assert!(read_file(&f, None, None).unwrap_err().contains("not UTF-8"));
    }

    #[test]
    fn edit_requires_unique_match_unless_replace_all() {
        let d = tmp("edit");
        let f = d.join("e.txt");
        std::fs::write(&f, "a b a").unwrap();
        assert!(edit_file(&f, "a", "x", false).unwrap_err().contains("2 places"));
        assert!(edit_file(&f, "zzz", "x", false).unwrap_err().contains("not found"));
        edit_file(&f, "b", "B", false).unwrap();
        edit_file(&f, "a", "x", true).unwrap();
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "x B x");
    }

    #[test]
    fn write_creates_parents_and_list_marks_dirs() {
        let d = tmp("write");
        write_file(&d.join("sub/dir/f.txt"), "hi").unwrap();
        assert_eq!(list_dir(&d).unwrap(), "sub/");
        assert_eq!(list_dir(&d.join("sub/dir")).unwrap(), "f.txt");
    }

    #[test]
    fn protected_paths_are_blocked_including_via_dotdot() {
        let home = tmp("home");
        std::fs::create_dir_all(home.join(".ssh")).unwrap();
        std::fs::write(home.join(".ssh/id_ed25519"), "KEY").unwrap();
        assert!(check_path("~/.ssh/id_ed25519", &home).is_err());
        assert!(check_path(".ssh", &home).is_err());
        assert!(check_path("docs/../.ssh/id_ed25519", &home).is_err());
        assert!(check_path(".config/loki/api_token", &home).is_err());
        assert!(check_path("docs/notes.txt", &home).is_ok());
    }

    #[test]
    fn protected_paths_are_blocked_via_symlink() {
        let home = tmp("symhome");
        std::fs::create_dir_all(home.join(".ssh")).unwrap();
        std::os::unix::fs::symlink(home.join(".ssh"), home.join("innocent")).unwrap();
        assert!(check_path("innocent/id_rsa", &home).is_err());
    }

    #[test]
    fn run_command_captures_output_status_and_timeout() {
        let d = tmp("run");
        assert_eq!(run_command("echo hi", &d, 5).unwrap().trim(), "hi");
        assert!(run_command("echo oops >&2; exit 3", &d, 5).unwrap_err().contains("exit status 3"));
        let started = Instant::now();
        assert!(run_command("sleep 30", &d, 1).unwrap_err().contains("timed out"));
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn a_background_child_cannot_hold_the_call_open() {
        let d = tmp("bg");
        let started = Instant::now();
        let out = run_command("sleep 30 & echo started", &d, 20).unwrap();
        assert!(started.elapsed() < Duration::from_secs(8), "waited on the background child: {:?}", started.elapsed());
        assert!(out.contains("started") && out.contains("background process"));
        // A command whose output simply ends is unaffected.
        let quick = Instant::now();
        assert_eq!(run_command("echo done", &d, 20).unwrap().trim(), "done");
        assert!(quick.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn policy_switches_are_enforced() {
        let input = json!({"path": "x.txt", "content": "c", "command": "true"});
        assert!(call("device_write_file", &input, &cfg(false, false)).unwrap_err().contains("disabled"));
        assert!(call("device_edit_file", &input, &cfg(false, false)).unwrap_err().contains("disabled"));
        assert!(call("device_run_command", &input, &cfg(true, false)).unwrap_err().contains("disabled"));
        assert!(call("nope", &input, &cfg(true, true)).unwrap_err().contains("unknown"));
    }
}
