//! Server-side "always allow" rules for approval prompts. They live on the `loki-server` host
//! (`~/.local/share/loki/always_allow.json`), so one "Yes, always" applies to every client and
//! device instead of being re-granted per browser.
//!
//! A rule is deliberately narrower than "this tool": shell commands match the **exact**
//! command, file writes/edits match the **path**, `WebFetch` matches the **host**. Actions that
//! are destructive or outward-facing (delete, merge, push, publish, send…) can never be made
//! "always" — they ask every time.

use serde_json::Value;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Mutex;

/// Tool names whose approval can never be made permanent.
pub fn can_always(tool: &str) -> bool {
    let t = tool.to_lowercase();
    !["delete", "merge", "push", "publish", "send", "trash", "spam", "remove", "drop"].iter().any(|w| t.contains(w))
}

fn str_field<'a>(input: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|k| input[*k].as_str())
}

/// `tool` plus whatever makes the rule specific. Shared by matching and creating rules.
pub fn rule_key(tool: &str, input: &Value) -> String {
    let short = tool.rsplit("__").next().unwrap_or(tool);
    let detail = match short {
        "Bash" | "device_run_command" => str_field(input, &["command"]).map(|c| c.trim().to_string()),
        // A rule covers one exact command on one host, never "anything on that machine".
        "remote_run_command" => match (str_field(input, &["host"]), str_field(input, &["command"])) {
            (Some(h), Some(c)) => Some(format!("{h}: {}", c.trim())),
            _ => None,
        },
        "Read" | "Write" | "Edit" | "MultiEdit" | "NotebookEdit" | "device_write_file" | "device_edit_file" => {
            str_field(input, &["file_path", "notebook_path", "path"]).map(str::to_string)
        }
        "Glob" | "Grep" => Some(str_field(input, &["path"]).unwrap_or("").to_string()),
        "WebFetch" => str_field(input, &["url"]).and_then(|u| {
            u.split("://").nth(1).and_then(|rest| rest.split(['/', '?', '#']).next()).map(str::to_string)
        }),
        _ => None,
    };
    match detail {
        Some(d) => format!("{tool}:{d}"),
        None => tool.to_string(),
    }
}

pub struct PermissionRules {
    path: Option<PathBuf>,
    rules: Mutex<BTreeSet<String>>,
    /// host -> expiry of an in-memory session trust (never persisted).
    session_grants: Mutex<std::collections::HashMap<String, std::time::Instant>>,
    /// Where permanent workspace trust is saved (`workspace_trust.json`); `None` = memory only.
    ws_path: Option<PathBuf>,
    /// Trusted folders: (host, path, expiry). `None` expiry = permanent (saved to `ws_path`).
    workspaces: Mutex<Vec<(String, String, Option<std::time::Instant>)>>,
}

impl PermissionRules {
    pub fn in_memory() -> Self {
        Self { path: None, rules: Mutex::new(BTreeSet::new()), session_grants: Mutex::default(), ws_path: None, workspaces: Mutex::default() }
    }

    pub fn load(path: PathBuf) -> Self {
        let rules = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str::<Vec<String>>(&raw).ok())
            .map(|v| v.into_iter().collect())
            .unwrap_or_default();
        let ws_path = path.with_file_name("workspace_trust.json");
        let workspaces: Vec<(String, String, Option<std::time::Instant>)> = std::fs::read_to_string(&ws_path)
            .ok()
            .and_then(|raw| serde_json::from_str::<Vec<(String, String)>>(&raw).ok())
            .unwrap_or_default()
            .into_iter()
            .map(|(h, p)| (h, p, None))
            .collect();
        Self { path: Some(path), rules: Mutex::new(rules), session_grants: Mutex::default(), ws_path: Some(ws_path), workspaces: Mutex::new(workspaces) }
    }

    fn save(&self, rules: &BTreeSet<String>) {
        let Some(path) = &self.path else { return };
        let result = (|| -> anyhow::Result<()> {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let tmp = path.with_extension("json.tmp");
            std::fs::write(&tmp, serde_json::to_vec_pretty(&rules.iter().collect::<Vec<_>>())?)?;
            std::fs::rename(tmp, path)?;
            Ok(())
        })();
        if let Err(e) = result {
            eprintln!("[loki-debug] couldn't save always-allow rules (non-fatal): {e}");
        }
    }

    pub fn is_allowed(&self, tool: &str, input: &Value) -> bool {
        if !can_always(tool) {
            return false;
        }
        if tool.rsplit("__").next() == Some("remote_run_command") {
            if let Some(cmd) = str_field(input, &["command"]) {
                if remote_command_read_only(cmd) {
                    return true;
                }
                if let Some(host) = str_field(input, &["host"]) {
                    if grant_covers(cmd) && self.host_granted(host) {
                        return true;
                    }
                }
            }
        }
        if self.workspace_covers(tool, input) {
            return true;
        }
        self.rules.lock().unwrap().contains(&rule_key(tool, input))
    }

    /// Adds a rule for exactly this call. Errors (with a reason) for never-always tools.
    pub fn add(&self, tool: &str, input: &Value) -> Result<String, String> {
        if !can_always(tool) {
            return Err(format!("'{tool}' can't be made always-allowed; it asks every time"));
        }
        let key = rule_key(tool, input);
        let mut rules = self.rules.lock().unwrap();
        rules.insert(key.clone());
        self.save(&rules);
        Ok(key)
    }

    pub fn remove(&self, key: &str) -> bool {
        let mut rules = self.rules.lock().unwrap();
        let removed = rules.remove(key);
        if removed {
            self.save(&rules);
        }
        removed
    }

    pub fn list(&self) -> Vec<String> {
        self.rules.lock().unwrap().iter().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn keys_are_specific() {
        assert_eq!(rule_key("Bash", &json!({"command": " git status "})), "Bash:git status");
        assert_eq!(rule_key("device_run_command", &json!({"command": "ls"})), "device_run_command:ls");
        assert_eq!(rule_key("device_write_file", &json!({"path": "/a/b.txt", "content": "x"})), "device_write_file:/a/b.txt");
        assert_eq!(rule_key("WebFetch", &json!({"url": "https://example.com/a?b=1"})), "WebFetch:example.com");
        assert_eq!(rule_key("mcp__mealie__create_mealplan", &json!({"x": 1})), "mcp__mealie__create_mealplan");
    }

    #[test]
    fn remote_rules_are_per_host_and_exact_command() {
        let r = PermissionRules::in_memory();
        r.add("remote_run_command", &json!({"host": "laptop", "command": "systemctl restart foo"})).unwrap();
        assert!(r.is_allowed("remote_run_command", &json!({"host": "laptop", "command": "systemctl restart foo"})));
        assert!(!r.is_allowed("remote_run_command", &json!({"host": "otherhost", "command": "systemctl restart foo"})));
        assert!(!r.is_allowed("remote_run_command", &json!({"host": "laptop", "command": "systemctl restart foo; reboot"})));
    }

    #[test]
    fn shell_rules_match_only_the_exact_command() {
        let r = PermissionRules::in_memory();
        r.add("Bash", &json!({"command": "git status"})).unwrap();
        assert!(r.is_allowed("Bash", &json!({"command": "git status"})));
        assert!(!r.is_allowed("Bash", &json!({"command": "git push --force"})));
        assert!(!r.is_allowed("Bash", &json!({"command": "git status; rm -rf ~"})));
    }

    #[test]
    fn destructive_tools_can_never_be_always() {
        let r = PermissionRules::in_memory();
        for t in ["delete_file", "mcp__obsidian__obsidian_delete_file", "mcp__github__merge_pull_request", "mcp__claude_ai_Gmail__send_message"] {
            assert!(r.add(t, &json!({})).is_err(), "{t}");
            assert!(!r.is_allowed(t, &json!({})));
        }
    }

    #[test]
    fn persists_and_removes() {
        let path = std::env::temp_dir().join(format!("loki-rules-test-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let r = PermissionRules::load(path.clone());
        let key = r.add("WebFetch", &json!({"url": "https://example.com/x"})).unwrap();
        let again = PermissionRules::load(path.clone());
        assert_eq!(again.list(), vec![key.clone()]);
        assert!(again.remove(&key));
        assert!(PermissionRules::load(path).list().is_empty());
    }
}

// ---- Remote commands: read-only auto-allow and in-memory session host grants ----

const READ_ONLY_CMDS: &[&str] = &[
    "cd", "pwd", "echo", "printf", "ls", "cat", "head", "tail", "grep", "egrep", "fgrep", "diff", "wc", "md5sum", "sha256sum", "stat", "cut", "tr",
    "uptime", "date", "df", "du", "free", "whoami", "hostname", "uname", "file", "basename", "dirname", "realpath", "readlink", "id", "nproc", "ps",
];

/// Anything mentioning these is never auto-allowed: secrets must not flow into a model's context unasked.
const SENSITIVE: &[&str] = &[
    ".ssh", "keys.env", ".env", "id_rsa", "id_ed25519", "api_token", "secret", "password", "passwd", "shadow", ".gnupg", "credential", "token",
];

/// Words that keep a command out of a session host grant (they still ask).
const DESTRUCTIVE: &[&str] = &[
    "sudo", "rm", "mv", "dd", "tee", "reboot", "shutdown", "poweroff", "halt", "mkfs", "chmod", "chown", "kill", "killall", "pkill", "truncate", "shred", "push", "eval", "base64",
];

fn segment_read_only(seg: &str) -> bool {
    let mut words = seg.split_whitespace();
    let Some(first) = words.next() else { return false };
    let rest: Vec<&str> = words.collect();
    match first {
        "sed" => {
            let mut flags_ok = true;
            let mut script: Option<&str> = None;
            for w in &rest {
                if w.starts_with('-') {
                    if *w != "-n" {
                        flags_ok = false;
                    }
                } else if script.is_none() {
                    script = Some(*w);
                }
            }
            flags_ok
                && script
                    .map(|s| {
                        let s = s.trim_matches('\'');
                        s.strip_suffix('p').map(|b| !b.is_empty() && b.chars().all(|c| c.is_ascii_digit() || c == ',' || c == '$')).unwrap_or(false)
                    })
                    .unwrap_or(false)
        }
        "git" => {
            let sub = rest.iter().find(|w| !w.starts_with('-'));
            matches!(sub.map(|s| *s), Some("status" | "log" | "diff" | "show" | "rev-parse"))
                && !rest.iter().any(|w| w.starts_with("--output") || w.starts_with("--ext-diff") || *w == "-c")
        }
        "systemctl" => {
            let sub = rest.iter().find(|w| !w.starts_with('-'));
            matches!(sub.map(|s| *s), Some("status" | "is-active" | "is-enabled" | "list-units" | "list-timers" | "show"))
        }
        "sort" => !rest.iter().any(|w| w.starts_with("--output") || (w.starts_with('-') && !w.starts_with("--") && w.contains('o'))),
        _ => READ_ONLY_CMDS.contains(&first),
    }
}

/// True when `command` is only read-only commands joined by pipes, `&&`, `||`, `;` or newlines,
/// with no redirects, command substitution, background `&`, or secret-looking paths. Anything
/// unsure is false, so it prompts as before.
pub fn remote_command_read_only(command: &str) -> bool {
    let cleaned = command.replace("2>&1", "").replace("2>/dev/null", "");
    let c = cleaned.trim();
    if c.is_empty() {
        return false;
    }
    let lower = c.to_lowercase();
    if SENSITIVE.iter().any(|s| lower.contains(s)) {
        return false;
    }
    if c.contains(['>', '<', '`']) || c.contains("$(") || c.contains("${") {
        return false;
    }
    let normalized = c.replace("&&", ";").replace("||", ";").replace('|', ";");
    if normalized.contains('&') {
        return false;
    }
    normalized.split([';', '\n']).map(str::trim).filter(|s| !s.is_empty()).all(segment_read_only)
}

/// Whether a session host grant may cover `command` (no destructive words, redirects, secrets or substitution).
fn grant_covers(command: &str) -> bool {
    let lower = command.to_lowercase();
    if SENSITIVE.iter().any(|s| lower.contains(s)) || lower.contains("$(") || lower.contains('`') {
        return false;
    }
    let stripped = lower.replace("2>&1", "").replace("2>/dev/null", "").replace(">/dev/null", "").replace("> /dev/null", "");
    if stripped.contains('>') {
        return false;
    }
    !stripped.split(|c: char| !(c.is_alphanumeric() || c == '-' || c == '_')).any(|w| DESTRUCTIVE.contains(&w))
}

impl PermissionRules {
    /// Trusts `host` for `ttl`: non-destructive commands there stop asking. Memory only, never
    /// saved to `always_allow.json`, gone on restart.
    pub fn grant_host(&self, host: &str, ttl: std::time::Duration) {
        self.session_grants.lock().unwrap().insert(host.to_string(), std::time::Instant::now() + ttl);
    }

    /// (host, seconds left) for every unexpired grant.
    pub fn granted_hosts(&self) -> Vec<(String, u64)> {
        let now = std::time::Instant::now();
        let mut g = self.session_grants.lock().unwrap();
        g.retain(|_, until| *until > now);
        let mut v: Vec<(String, u64)> = g.iter().map(|(h, u)| (h.clone(), u.saturating_duration_since(now).as_secs())).collect();
        v.sort();
        v
    }

    pub fn revoke_host(&self, host: &str) -> bool {
        self.session_grants.lock().unwrap().remove(host).is_some()
    }

    fn host_granted(&self, host: &str) -> bool {
        let mut g = self.session_grants.lock().unwrap();
        g.retain(|_, until| *until > std::time::Instant::now());
        g.contains_key(host)
    }
}

/// Host name used for the desktop app's machine in workspace trust.
pub const DEVICE_HOST: &str = "device";

/// Folders that can never be trusted, and the secret-looking words that disqualify a path.
const NEVER_TRUST_ROOTS: &[&str] = &["etc", "usr", "bin", "sbin", "boot", "dev", "proc", "sys", "var", "lib", "lib64", "root", "run", "opt"];

/// A trusted folder in one comparable form: absolute, no `.`/`..`, no trailing slash, `~/` expanded
/// to this server's home (the machines share the user name, so `~/x` means the same everywhere).
/// `None` if it isn't a path.
pub fn normalize_path(raw: &str) -> Option<String> {
    let raw = raw.trim().trim_matches(|c| c == '"' || c == '\'');
    let expanded = if raw == "~" {
        dirs::home_dir()?.display().to_string()
    } else if let Some(rest) = raw.strip_prefix("~/") {
        format!("{}/{rest}", dirs::home_dir()?.display())
    } else {
        raw.to_string()
    };
    if !expanded.starts_with('/') {
        return None;
    }
    let mut parts: Vec<&str> = Vec::new();
    for c in expanded.split('/') {
        match c {
            "" => {}
            "." | ".." => return None,
            other => parts.push(other),
        }
    }
    if parts.is_empty() {
        return None;
    }
    Some(format!("/{}", parts.join("/")))
}

/// Why `path` is too broad or too sensitive to trust as a workspace (`Ok` carries the normalized form).
pub fn check_trustable(raw: &str) -> Result<String, String> {
    let path = normalize_path(raw).ok_or("give an absolute path (or ~/something) without .. in it")?;
    let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    let home = dirs::home_dir().map(|h| h.display().to_string()).unwrap_or_default();
    if path == home || parts.len() < 2 || NEVER_TRUST_ROOTS.contains(&parts[0]) {
        return Err(format!("{path} is too broad to trust — pick a project folder inside it"));
    }
    let lower = path.to_lowercase();
    if SENSITIVE.iter().any(|w| lower.contains(w)) || parts.iter().any(|p| p.starts_with('.')) {
        return Err(format!("{path} looks like it holds secrets or settings, so it can't be trusted as a workspace"));
    }
    Ok(path)
}

fn inside(folder: &str, path: &str) -> bool {
    path == folder || path.strip_prefix(folder).is_some_and(|rest| rest.starts_with('/'))
}

/// Absolute-path-looking words in a shell command (`~/x`, `/a/b`, quotes stripped), normalized.
fn command_paths(command: &str) -> Option<Vec<String>> {
    let mut out = Vec::new();
    for word in command.split_whitespace() {
        let w = word.trim_matches(|c| c == '"' || c == '\'' || c == ';' || c == '(' || c == ')');
        if w.starts_with('/') || w.starts_with("~/") || w == "~" {
            out.push(normalize_path(w)?); // a path with .. in it is not covered
        }
    }
    Some(out)
}

impl PermissionRules {
    fn save_workspaces(&self, list: &[(String, String, Option<std::time::Instant>)]) {
        let Some(path) = &self.ws_path else { return };
        let perm: Vec<(&String, &String)> = list.iter().filter(|(_, _, u)| u.is_none()).map(|(h, p, _)| (h, p)).collect();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = std::fs::write(path, serde_json::to_vec_pretty(&perm).unwrap_or_default()) {
            eprintln!("[loki-debug] couldn't save workspace trust (non-fatal): {e}");
        }
    }

    /// Trusts a folder on `host` (`device` or a remote host): non-destructive edits and commands
    /// inside it stop asking. `ttl: None` = permanent (saved). Returns the normalized path.
    pub fn trust_workspace(&self, host: &str, raw_path: &str, ttl: Option<std::time::Duration>) -> Result<String, String> {
        let path = check_trustable(raw_path)?;
        let mut list = self.workspaces.lock().unwrap();
        list.retain(|(h, p, _)| !(h == host && *p == path));
        list.push((host.to_string(), path.clone(), ttl.map(|t| std::time::Instant::now() + t)));
        self.save_workspaces(&list);
        Ok(path)
    }

    pub fn untrust_workspace(&self, host: &str, raw_path: &str) -> bool {
        let Some(path) = normalize_path(raw_path) else { return false };
        let mut list = self.workspaces.lock().unwrap();
        let before = list.len();
        list.retain(|(h, p, _)| !(h == host && *p == path));
        let removed = list.len() != before;
        if removed {
            self.save_workspaces(&list);
        }
        removed
    }

    /// (host, path, seconds left or None = permanent) for every live grant.
    pub fn trusted_workspaces(&self) -> Vec<(String, String, Option<u64>)> {
        let now = std::time::Instant::now();
        let mut list = self.workspaces.lock().unwrap();
        list.retain(|(_, _, u)| u.map(|u| u > now).unwrap_or(true));
        let mut v: Vec<_> = list.iter().map(|(h, p, u)| (h.clone(), p.clone(), u.map(|u| u.saturating_duration_since(now).as_secs()))).collect();
        v.sort();
        v
    }

    fn workspace_trusted(&self, host: &str, path: &str) -> bool {
        let now = std::time::Instant::now();
        let mut list = self.workspaces.lock().unwrap();
        list.retain(|(_, _, u)| u.map(|u| u > now).unwrap_or(true));
        list.iter().any(|(h, folder, _)| h == host && inside(folder, path))
    }

    /// Whether a trusted folder covers this call: file writes/edits whose path is inside it, and
    /// non-destructive commands that run in or only touch it. Anything with a redirect,
    /// substitution, destructive word or secret-looking text still asks.
    fn workspace_covers(&self, tool: &str, input: &Value) -> bool {
        match tool.rsplit("__").next().unwrap_or(tool) {
            "device_write_file" | "device_edit_file" => str_field(input, &["path", "file_path"]).and_then(normalize_path).is_some_and(|p| self.workspace_trusted(DEVICE_HOST, &p)),
            "device_run_command" => {
                let (Some(cmd), Some(cwd)) = (str_field(input, &["command"]), str_field(input, &["cwd"]).and_then(normalize_path)) else { return false };
                grant_covers(cmd) && self.workspace_trusted(DEVICE_HOST, &cwd) && command_paths(cmd).is_some_and(|ps| ps.iter().all(|p| self.workspace_trusted(DEVICE_HOST, p)))
            }
            "remote_run_command" => {
                let (Some(host), Some(cmd)) = (str_field(input, &["host"]), str_field(input, &["command"])) else { return false };
                if !grant_covers(cmd) {
                    return false;
                }
                let Some(paths) = command_paths(cmd) else { return false };
                // Runs "in" a folder when it starts with `cd <folder>` ...
                let cd_dir = cmd.trim().strip_prefix("cd ").and_then(|r| r.split(['&', ';']).next()).and_then(|d| normalize_path(d.trim()));
                match cd_dir {
                    Some(dir) => self.workspace_trusted(host, &dir) && paths.iter().all(|p| self.workspace_trusted(host, p)),
                    // ... otherwise every path it names must be inside trusted folders (and it names at least one).
                    None => !paths.is_empty() && paths.iter().all(|p| self.workspace_trusted(host, p)),
                }
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod workspace_tests {
    use super::*;
    use serde_json::json;

    fn r() -> PermissionRules {
        let r = PermissionRules::in_memory();
        r.trust_workspace("laptop", "/home/user/Work/proj", None).unwrap();
        r.trust_workspace(DEVICE_HOST, "/home/user/Desktop/Loki", Some(std::time::Duration::from_secs(60))).unwrap();
        r
    }

    #[test]
    fn broad_or_secret_folders_cannot_be_trusted() {
        let r = PermissionRules::in_memory();
        for bad in ["/", "/etc", "/usr/local/bin", "~", "relative/dir", "/home/user/a/../b", "~/.ssh", "/home/user/.config/loki", "/srv/keys.env"] {
            assert!(r.trust_workspace("device", bad, None).is_err(), "{bad}");
        }
        assert!(r.trust_workspace("device", "/home/user/projects/x/", None).is_ok());
    }

    #[test]
    fn device_edits_inside_the_folder_are_covered() {
        let r = r();
        assert!(r.is_allowed("device_write_file", &json!({"path": "/home/user/Desktop/Loki/src/a.rs", "content": "x"})));
        assert!(!r.is_allowed("device_write_file", &json!({"path": "/home/user/Desktop/Loki2/a.rs"})));
        assert!(!r.is_allowed("device_write_file", &json!({"path": "/home/user/Desktop/Loki/../x"})));
        assert!(r.is_allowed("device_run_command", &json!({"command": "cargo test", "cwd": "/home/user/Desktop/Loki/src-tauri"})));
        assert!(!r.is_allowed("device_run_command", &json!({"command": "rm -rf target", "cwd": "/home/user/Desktop/Loki"})));
        assert!(!r.is_allowed("device_run_command", &json!({"command": "cat /etc/passwd", "cwd": "/home/user/Desktop/Loki"})));
        assert!(!r.is_allowed("device_run_command", &json!({"command": "cargo test"})));
    }

    #[test]
    fn remote_commands_must_run_in_or_only_touch_the_folder() {
        let r = r();
        let run = |c: &str| json!({"host": "laptop", "command": c});
        assert!(r.is_allowed("remote_run_command", &run("cd /home/user/Work/proj && make build")));
        assert!(r.is_allowed("remote_run_command", &run("touch /home/user/Work/proj/a.txt")));
        assert!(!r.is_allowed("remote_run_command", &run("touch /home/user/Work/other/a.txt")));
        assert!(!r.is_allowed("remote_run_command", &run("cd /home/user/Work/proj && cp x /tmp/y")));
        assert!(!r.is_allowed("remote_run_command", &run("make build")), "no folder named, so it asks");
        assert!(!r.is_allowed("remote_run_command", &run("cd /home/user/Work/proj && rm -rf build")));
        assert!(!r.is_allowed("remote_run_command", &json!({"host": "hub", "command": "touch /home/user/Work/proj/a"})), "other host");
    }

    #[test]
    fn grants_expire_revoke_and_permanent_ones_persist() {
        let r = PermissionRules::in_memory();
        r.trust_workspace(DEVICE_HOST, "/home/user/x/y", Some(std::time::Duration::ZERO)).unwrap();
        assert!(r.trusted_workspaces().is_empty());
        r.trust_workspace(DEVICE_HOST, "/home/user/x/y", None).unwrap();
        assert!(r.untrust_workspace(DEVICE_HOST, "/home/user/x/y/"));
        let dir = std::env::temp_dir().join(format!("loki-ws-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let a = PermissionRules::load(dir.join("always_allow.json"));
        a.trust_workspace("hub", "/home/user/p/q", None).unwrap();
        a.trust_workspace("hub", "/home/user/p/s", Some(std::time::Duration::from_secs(60))).unwrap();
        let b = PermissionRules::load(dir.join("always_allow.json"));
        assert_eq!(b.trusted_workspaces(), vec![("hub".to_string(), "/home/user/p/q".to_string(), None)]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod remote_allow_tests {
    use super::*;
    use serde_json::json;

    fn run(host: &str, cmd: &str) -> Value {
        json!({"host": host, "command": cmd})
    }

    #[test]
    fn read_only_commands_are_allowed() {
        for c in [
            "ls -la /tmp",
            "cd ~/x && grep -n foo a.rs | cut -c1-100 | head -5",
            "sed -n '1,60p' f.rs",
            "git status --short",
            "systemctl --user status loki-server --no-pager",
            "md5sum a b",
            "wc -l *.rs | sort -n | tail -5",
            "cat f 2>/dev/null",
        ] {
            assert!(remote_command_read_only(c), "{c}");
        }
    }

    #[test]
    fn anything_else_is_not() {
        for c in [
            "uptime; reboot", "cat a > b", "echo `id`", "echo $(id)", "ls & rm x", "rm -rf x", "sed -i s/a/b/ f", "sed -n '1e ls' f",
            "cat ~/.ssh/id_ed25519", "git diff --output=x", "sort -o x y", "sudo ls", "cat keys.env", "find . -delete", "tee x", "git push",
            "systemctl restart foo", "",
        ] {
            assert!(!remote_command_read_only(c), "{c}");
        }
    }

    #[test]
    fn session_grant_is_host_scoped_and_expires() {
        let r = PermissionRules::in_memory();
        let touch = run("laptop", "touch /tmp/x");
        assert!(!r.is_allowed("remote_run_command", &touch));
        r.grant_host("laptop", std::time::Duration::from_secs(60));
        assert!(r.is_allowed("remote_run_command", &touch));
        assert!(!r.is_allowed("remote_run_command", &run("otherhost", "touch /tmp/x")));
        assert!(!r.is_allowed("remote_run_command", &run("laptop", "rm -rf /tmp/x")));
        assert!(!r.is_allowed("remote_run_command", &run("laptop", "echo hi > /tmp/x")));
        r.grant_host("laptop", std::time::Duration::ZERO);
        assert!(!r.is_allowed("remote_run_command", &touch));
        r.grant_host("laptop", std::time::Duration::from_secs(60));
        assert!(r.revoke_host("laptop"));
        assert!(!r.is_allowed("remote_run_command", &touch));
    }
}
