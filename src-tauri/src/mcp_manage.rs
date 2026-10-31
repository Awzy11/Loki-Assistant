//! Managing MCP servers while Loki runs: list them, enable, disable, reconnect, add and remove, from Settings, the
//! `/mcp` command or a confirmed chat tool. `config.toml` stays the hand-edited base; what the UI changes is kept in
//! `~/.config/loki/mcp_overrides.json`, so a click never rewrites the TOML.
//!
//! A command (stdio) server runs a program on the Loki host with its permissions, so adding one is validated hard
//! here and always confirmed by a person: Settings asks first, and the chat tool is `ConfirmRequired`. Secrets
//! (environment values, URL query strings) are accepted from Settings but never shown back or typed through chat.

use crate::config::{McpConfig, McpServerConfig};
use crate::mcp_hub::{McpHub, ServerSpec, Transport};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Overrides {
    /// Servers added from the UI.
    #[serde(default)]
    pub servers: Vec<McpServerConfig>,
    /// Hidden or switched off from the UI.
    #[serde(default)]
    pub disabled: Vec<String>,
    /// Switched back on although `config.toml` lists them as disabled.
    #[serde(default)]
    pub enabled: Vec<String>,
}

fn path() -> Option<std::path::PathBuf> {
    Some(dirs::config_dir()?.join("loki").join("mcp_overrides.json"))
}
pub fn load_overrides() -> Overrides {
    path().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}
fn save_overrides(o: &Overrides) -> Result<(), String> {
    let p = path().ok_or("no config directory")?;
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let body = serde_json::to_string_pretty(o).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&p).map_err(|e| e.to_string())?;
        f.write_all(body.as_bytes()).map_err(|e| e.to_string())
    }
    #[cfg(not(unix))]
    std::fs::write(&p, body).map_err(|e| e.to_string())
}

/// Every server Loki knows about, with where it comes from ("claude", "config" or "added"). Not filtered by enabled.
fn candidates(cfg: &McpConfig, claude_json: Option<&Value>, ov: &Overrides) -> BTreeMap<String, (ServerSpec, &'static str)> {
    let mut by: BTreeMap<String, (ServerSpec, &'static str)> = BTreeMap::new();
    // reuse the hub's own parsing: ask it for the specs with nothing disabled
    let open = McpConfig { disabled: Vec::new(), servers: Vec::new(), ..cfg.clone() };
    for s in crate::mcp_hub::server_specs(&open, claude_json) {
        by.insert(s.name.clone(), (s, "claude"));
    }
    let only_servers = McpConfig { import_claude_servers: false, disabled: Vec::new(), servers: cfg.servers.clone(), pinned_tools: Vec::new() };
    for s in crate::mcp_hub::server_specs(&only_servers, None) {
        by.insert(s.name.clone(), (s, "config"));
    }
    let added = McpConfig { import_claude_servers: false, disabled: Vec::new(), servers: ov.servers.clone(), pinned_tools: Vec::new() };
    for s in crate::mcp_hub::server_specs(&added, None) {
        by.insert(s.name.clone(), (s, "added"));
    }
    by
}

fn is_enabled(name: &str, cfg: &McpConfig, ov: &Overrides) -> bool {
    !ov.disabled.iter().any(|d| d == name) && (!cfg.disabled.iter().any(|d| d == name) || ov.enabled.iter().any(|d| d == name))
}

/// The servers that should be connected, with the overrides applied. Used at startup and after every change.
pub fn effective_specs(cfg: &McpConfig) -> Vec<ServerSpec> {
    let json = if cfg.import_claude_servers { crate::mcp_hub::read_claude_json() } else { None };
    let ov = load_overrides();
    candidates(cfg, json.as_ref(), &ov).into_iter().filter(|(n, _)| is_enabled(n, cfg, &ov)).map(|(_, (s, _))| s).collect()
}

/// What a person may see of a server's target: no secrets (URL query strings and credentials, env values).
pub fn redact_url(u: &str) -> String {
    let no_query = u.split(['?', '#']).next().unwrap_or("");
    Regex::new(r"^(https?://)[^/@]*@").unwrap().replace(no_query, "$1").to_string()
}

pub fn validate(s: &McpServerConfig) -> Result<(), String> {
    if !Regex::new(r"^[a-z0-9][a-z0-9-]{0,30}$").unwrap().is_match(&s.name) {
        return Err("The name must be lowercase letters, digits and hyphens (no spaces or underscores), up to 31 characters.".into());
    }
    if ["loki-mcp", "loki-bridge"].contains(&s.name.as_str()) {
        return Err("That name is reserved: Loki never connects to itself.".into());
    }
    match (&s.command, &s.url) {
        (Some(_), Some(_)) | (None, None) => return Err("Give either a command or a URL, not both.".into()),
        (Some(c), None) => {
            if c.is_empty() || c.len() > 200 || c.chars().any(|ch| ch.is_whitespace() || ";|&$<>`'\"()\\*?{}".contains(ch)) {
                return Err("The command must be a single program name or path with no spaces or shell characters; put the options in the arguments.".into());
            }
            if s.args.len() > 20 || s.args.iter().any(|a| a.len() > 400 || a.contains('\n')) {
                return Err("At most 20 arguments, each on one line and under 400 characters.".into());
            }
            if s.env.len() > 20 || s.env.keys().any(|k| !Regex::new(r"^[A-Z_][A-Z0-9_]{0,63}$").unwrap().is_match(k)) {
                return Err("Environment variable names must look like MY_KEY (capitals, digits, underscores).".into());
            }
        }
        (None, Some(u)) => {
            if !(u.starts_with("http://") || u.starts_with("https://")) || u.len() > 500 || u.contains(char::is_whitespace) {
                return Err("The URL must start with http:// or https://.".into());
            }
            if !s.args.is_empty() || !s.env.is_empty() {
                return Err("Arguments and environment variables only apply to command servers.".into());
            }
        }
    }
    Ok(())
}

struct Manager {
    hub: Arc<McpHub>,
    cfg: McpConfig,
}
static MGR: OnceLock<Manager> = OnceLock::new();
pub fn init(hub: Arc<McpHub>, cfg: McpConfig) {
    let _ = MGR.set(Manager { hub, cfg });
}
fn mgr() -> Result<&'static Manager, String> {
    MGR.get().ok_or_else(|| "MCP management isn't ready yet".to_string())
}

/// Everything the Settings page and `/mcp` show: one entry per known server, enabled or not.
pub fn list() -> Result<Vec<Value>, String> {
    let m = mgr()?;
    let json = if m.cfg.import_claude_servers { crate::mcp_hub::read_claude_json() } else { None };
    let ov = load_overrides();
    let status: BTreeMap<String, crate::mcp_hub::ServerStatus> = m.hub.status().into_iter().map(|s| (s.name.clone(), s)).collect();
    Ok(candidates(&m.cfg, json.as_ref(), &ov)
        .into_iter()
        .map(|(name, (spec, source))| {
            let enabled = is_enabled(&name, &m.cfg, &ov);
            let st = status.get(&name);
            let (transport, target, env_keys) = match &spec.transport {
                Transport::Stdio { command, args, env } => ("command", format!("{command} {}", args.join(" ")).trim().chars().take(200).collect::<String>(), env.keys().cloned().collect::<Vec<_>>()),
                Transport::Http { url } => ("url", redact_url(url), Vec::new()),
            };
            json!({
                "name": name, "source": source, "enabled": enabled, "transport": transport, "target": target, "env_keys": env_keys,
                "connected": st.is_some_and(|s| s.connected), "tools": st.map_or(0, |s| s.tools),
                "error": st.and_then(|s| s.error.clone()).filter(|_| enabled),
                "removable": source == "added",
            })
        })
        .collect())
}

pub fn tools_of(name: &str) -> Result<Vec<Value>, String> {
    Ok(mgr()?.hub.tools_of(name).into_iter().map(|(n, d)| json!({ "name": n, "description": d })).collect())
}

/// A readable summary, for the `/mcp` command and the chat tool.
pub fn status_text() -> String {
    let Ok(list) = list() else { return "MCP management isn't ready yet.".into() };
    let mut out = String::from("MCP servers (Loki's own connections):\n");
    for s in &list {
        let state = if s["enabled"] != true { "off".to_string() } else if s["connected"] == true { format!("connected, {} tools", s["tools"]) } else { format!("not connected ({})", s["error"].as_str().unwrap_or("unknown")) };
        out.push_str(&format!("- {} [{}] {}\n", s["name"].as_str().unwrap_or("?"), s["source"].as_str().unwrap_or("?"), state));
    }
    out.push_str("\n`/mcp enable|disable|reconnect <name>` changes one. Adding or removing is in Settings, or ask me in chat and approve the prompt.");
    out
}

#[derive(Debug, Deserialize, Default)]
pub struct Action {
    pub action: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
}

/// Applies one change and brings the live connections in line. Returns a sentence for the person.
pub async fn act(a: Action) -> Result<String, String> {
    let m = mgr()?;
    let name = a.name.trim().to_string();
    let mut ov = load_overrides();
    let json = if m.cfg.import_claude_servers { crate::mcp_hub::read_claude_json() } else { None };
    let known = candidates(&m.cfg, json.as_ref(), &ov);
    let src = known.get(&name).map(|(_, s)| *s);
    let msg = match a.action.as_str() {
        "reconnect" => {
            if src.is_none() {
                return Err(format!("There is no server called '{name}'."));
            }
            m.hub.reconnect(&name).await.map_err(|e| e.to_string())?;
            return Ok(format!("Reconnecting '{name}'. Give it a few seconds, then check the list."));
        }
        "disable" => {
            src.ok_or(format!("There is no server called '{name}'."))?;
            if !ov.disabled.contains(&name) {
                ov.disabled.push(name.clone());
            }
            ov.enabled.retain(|n| *n != name);
            format!("'{name}' is off. Its tools are no longer available.")
        }
        "enable" => {
            src.ok_or(format!("There is no server called '{name}'."))?;
            ov.disabled.retain(|n| *n != name);
            if m.cfg.disabled.contains(&name) && !ov.enabled.contains(&name) {
                ov.enabled.push(name.clone());
            }
            format!("'{name}' is on. It is connecting now.")
        }
        "remove" => {
            if src != Some("added") {
                return Err(format!("'{name}' comes from config.toml or Claude's own list, so Loki can't delete it. Use disable to switch it off."));
            }
            ov.servers.retain(|s| s.name != name);
            ov.disabled.retain(|n| *n != name);
            format!("Removed '{name}'.")
        }
        "add" => {
            let spec = McpServerConfig { name: name.clone(), command: a.command.filter(|c| !c.is_empty()), args: a.args, env: a.env, url: a.url.filter(|u| !u.is_empty()) };
            validate(&spec)?;
            if matches!(src, Some("config" | "claude")) {
                return Err(format!("A server called '{name}' already exists from config.toml or Claude's list. Pick another name."));
            }
            ov.servers.retain(|s| s.name != name);
            ov.servers.push(spec);
            ov.disabled.retain(|n| *n != name);
            format!("Added '{name}'. It is connecting now.")
        }
        other => return Err(format!("Unknown action '{other}'. Use enable, disable, reconnect, add or remove.")),
    };
    save_overrides(&ov)?;
    m.hub.reconcile(effective_specs(&m.cfg)).await;
    Ok(msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(name: &str, command: Option<&str>, url: Option<&str>) -> McpServerConfig {
        McpServerConfig { name: name.into(), command: command.map(Into::into), url: url.map(Into::into), ..Default::default() }
    }

    #[test]
    fn validation_refuses_anything_that_could_smuggle_a_shell() {
        assert!(validate(&s("good-one", Some("npx"), None)).is_ok());
        assert!(validate(&s("good-two", None, Some("http://100.64.0.10:8096/mcp"))).is_ok());
        for bad in [s("Bad Name", Some("npx"), None), s("a_b", Some("npx"), None), s("loki-mcp", Some("npx"), None), s("x", Some("npx -y thing"), None), s("x", Some("sh;rm"), None), s("x", Some("$(id)"), None), s("x", None, Some("file:///etc/passwd")), s("x", None, None), s("x", Some("npx"), Some("http://a"))] {
            assert!(validate(&bad).is_err(), "{:?}", bad.name);
        }
        let mut with_env = s("x", None, Some("http://a/mcp"));
        with_env.env.insert("KEY".into(), "v".into());
        assert!(validate(&with_env).is_err(), "env is only for command servers");
        let mut lower_env = s("x", Some("npx"), None);
        lower_env.env.insert("lower".into(), "v".into());
        assert!(validate(&lower_env).is_err());
    }

    #[test]
    fn urls_are_shown_without_credentials_or_query() {
        assert_eq!(redact_url("https://user:pw@host.example/mcp?token=abc#x"), "https://host.example/mcp");
        assert_eq!(redact_url("http://127.0.0.1:8096/mcp"), "http://127.0.0.1:8096/mcp");
    }

    #[test]
    fn enabled_follows_config_and_overrides() {
        let cfg = McpConfig { disabled: vec!["old".into()], ..Default::default() };
        let mut ov = Overrides::default();
        assert!(is_enabled("fetch", &cfg, &ov) && !is_enabled("old", &cfg, &ov));
        ov.enabled.push("old".into());
        assert!(is_enabled("old", &cfg, &ov));
        ov.disabled.push("fetch".into());
        assert!(!is_enabled("fetch", &cfg, &ov));
    }
}
