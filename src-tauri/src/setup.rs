//! First-run onboarding, served by `loki-server` when there is no `config.toml` yet. The browser walks the person through their models
//! (local, cloud or both), their notes, their vault layout, who they are and their login; this module checks each answer
//! (`/api/setup/probe`), writes the files (`/api/setup/commit`) and then carries the server on into its normal mode.
//!
//! Setup mode only answers connections from this machine (or anyone, if `LOKI_SETUP_ALLOW_REMOTE=1`), because nothing is protected by a
//! login until setup finishes.
use axum::extract::{ConnectInfo, Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::path::PathBuf;

/// The routines shipped with the starter pack, written to `~/.config/loki/commands/` when the person picks it.
const PACK_NAME: &str = "scrum-zettelkasten";
const PACK_PROFILE: &str = include_str!("../../packs/scrum-zettelkasten/profile.toml");
const PACK_ROUTINES: &[(&str, &str)] = &[
    ("bod", include_str!("../../packs/scrum-zettelkasten/routines/bod.md")),
    ("eod", include_str!("../../packs/scrum-zettelkasten/routines/eod.md")),
    ("bow", include_str!("../../packs/scrum-zettelkasten/routines/bow.md")),
    ("eow", include_str!("../../packs/scrum-zettelkasten/routines/eow.md")),
    ("bom", include_str!("../../packs/scrum-zettelkasten/routines/bom.md")),
    ("boq", include_str!("../../packs/scrum-zettelkasten/routines/boq.md")),
    ("haid", include_str!("../../packs/scrum-zettelkasten/routines/haid.md")),
    ("journal", include_str!("../../packs/scrum-zettelkasten/routines/journal.md")),
    ("low-cap", include_str!("../../packs/scrum-zettelkasten/routines/low-cap.md")),
    ("inbox-triage", include_str!("../../packs/scrum-zettelkasten/routines/inbox-triage.md")),
    ("backlog-triage", include_str!("../../packs/scrum-zettelkasten/routines/backlog-triage.md")),
];

/// True when this server has never been set up (no config.toml).
pub fn needs_setup() -> bool {
    crate::config::config_dir().map(|d| !d.join("config.toml").exists()).unwrap_or(false)
}

#[derive(Debug, Deserialize)]
pub struct Local {
    pub host: String,
    pub port: u16,
    pub model: String,
}

#[derive(Debug, Deserialize)]
pub struct Cloud {
    /// `anthropic` or `openai` (any OpenAI-compatible service).
    pub provider: String,
    #[serde(default)]
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

#[derive(Debug, Deserialize)]
pub struct ObsidianIn {
    pub host: String,
    pub port: u16,
    pub api_key: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct PersonIn {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub about: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct LocationIn {
    #[serde(default)]
    pub name: String,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct AccountIn {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct SetupRequest {
    pub local: Option<Local>,
    pub cloud: Option<Cloud>,
    /// With both: `local` (local first) or `cloud` (cloud first).
    #[serde(default)]
    pub routing: String,
    /// Required when there is a cloud provider and no local model.
    #[serde(default)]
    pub accept_cloud_only: bool,
    pub obsidian: ObsidianIn,
    /// `none` (blank vault) or the name of a starter pack.
    #[serde(default)]
    pub pack: String,
    #[serde(default)]
    pub person: PersonIn,
    #[serde(default)]
    pub location: LocationIn,
    pub account: AccountIn,
}

fn plain(s: &str, what: &str, max: usize) -> Result<(), String> {
    if s.trim().is_empty() || s.len() > max || s.chars().any(|c| c.is_control()) {
        return Err(format!("{what} is missing or has characters Loki can't store"));
    }
    Ok(())
}

fn host_ok(s: &str) -> bool {
    !s.is_empty() && s.len() <= 253 && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '_' | '[' | ']'))
}

impl SetupRequest {
    pub fn validate(&self) -> Result<(), String> {
        if self.local.is_none() && self.cloud.is_none() {
            return Err("Loki needs at least one model: a local one or a cloud provider.".into());
        }
        if let Some(l) = &self.local {
            if !host_ok(&l.host) || l.port == 0 {
                return Err("The local model's address doesn't look right.".into());
            }
            plain(&l.model, "The local model name", 200)?;
        }
        if let Some(c) = &self.cloud {
            if !matches!(c.provider.as_str(), "anthropic" | "openai") {
                return Err("The cloud provider must be Anthropic or an OpenAI-compatible service.".into());
            }
            plain(&c.api_key, "The cloud API key", 400)?;
            plain(&c.model, "The cloud model name", 200)?;
            if c.provider == "openai" && !(c.base_url.starts_with("https://") || c.base_url.starts_with("http://")) {
                return Err("The OpenAI-compatible base URL must start with https:// (or http://).".into());
            }
        }
        if self.local.is_none() && !self.accept_cloud_only {
            return Err("Without a local model, everything you ask goes to your cloud provider. Please confirm you are OK with that.".into());
        }
        if self.local.is_some() && self.cloud.is_some() && !matches!(self.routing.as_str(), "local" | "cloud") {
            return Err("Choose whether the local model or the cloud provider goes first.".into());
        }
        if !host_ok(&self.obsidian.host) || self.obsidian.port == 0 {
            return Err("The Obsidian address doesn't look right.".into());
        }
        plain(&self.obsidian.api_key, "The Obsidian API key", 400)?;
        if !(self.pack == "none" || self.pack == PACK_NAME || self.pack.is_empty()) {
            return Err("Unknown starter pack.".into());
        }
        if self.person.name.len() > 80 || self.person.about.len() > 400 || self.location.name.len() > 120 {
            return Err("One of the 'about you' answers is too long.".into());
        }
        if self.person.name.chars().chain(self.person.about.chars()).chain(self.location.name.chars()).any(|c| c.is_control()) {
            return Err("The 'about you' answers can't contain line breaks.".into());
        }
        match (self.location.latitude, self.location.longitude) {
            (None, None) => {}
            (Some(a), Some(o)) if (-90.0..=90.0).contains(&a) && (-180.0..=180.0).contains(&o) && !self.location.name.trim().is_empty() => {}
            _ => return Err("A location needs a name and both coordinates (latitude -90 to 90, longitude -180 to 180).".into()),
        }
        let u = &self.account.username;
        if u.is_empty() || u.len() > 32 || !u.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')) {
            return Err("The username can use letters, numbers, dots, dashes and underscores (up to 32).".into());
        }
        if self.account.password.chars().count() < 8 || self.account.password.len() > 200 {
            return Err("The password needs at least 8 characters.".into());
        }
        Ok(())
    }

    /// The text of `config.toml` for these answers.
    pub fn render_config(&self) -> Result<String, String> {
        use toml::Value as V;
        let mut root = toml::Table::new();
        let table = |pairs: Vec<(&str, V)>| -> toml::Table { pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect() };
        let ollama = |l: &Local| table(vec![("vendor", V::String("ollama".into())), ("host", V::String(l.host.clone())), ("port", V::Integer(l.port as i64)), ("model", V::String(l.model.clone()))]);
        let cloud_table = |c: &Cloud| -> toml::Table {
            if c.provider == "anthropic" {
                table(vec![("vendor", V::String("anthropic".into())), ("api_key", V::String(c.api_key.clone())), ("model", V::String(c.model.clone()))])
            } else {
                table(vec![("vendor", V::String("openai_compatible".into())), ("base_url", V::String(c.base_url.clone())), ("api_key", V::String(c.api_key.clone())), ("model", V::String(c.model.clone()))])
            }
        };
        if self.routing == "cloud" && self.local.is_some() && self.cloud.is_some() {
            root.insert("cloud_first".into(), V::Boolean(true));
        }
        match (&self.local, &self.cloud) {
            (Some(l), cloud) => {
                root.insert("llm".into(), V::Table(ollama(l)));
                root.insert(
                    "embedding".into(),
                    V::Table(table(vec![("vendor", V::String("ollama".into())), ("host", V::String(l.host.clone())), ("port", V::Integer(l.port as i64)), ("model", V::String("nomic-embed-text".into()))])),
                );
                if let Some(c) = cloud {
                    root.insert("fallbacks".into(), V::Array(vec![V::Table(cloud_table(c))]));
                }
            }
            (None, Some(c)) => {
                root.insert("llm".into(), V::Table(cloud_table(c)));
                if c.provider == "openai" {
                    root.insert(
                        "embedding".into(),
                        V::Table(table(vec![
                            ("vendor", V::String("openai_compatible".into())),
                            ("base_url", V::String(c.base_url.clone())),
                            ("api_key", V::String(c.api_key.clone())),
                            ("model", V::String("text-embedding-3-small".into())),
                        ])),
                    );
                }
            }
            (None, None) => return Err("no model".into()),
        }
        root.insert(
            "obsidian".into(),
            V::Table(table(vec![("host", V::String(self.obsidian.host.clone())), ("port", V::Integer(self.obsidian.port as i64)), ("api_key", V::String(self.obsidian.api_key.clone()))])),
        );
        let body = toml::to_string(&root).map_err(|e| e.to_string())?;
        Ok(format!("# Written by Loki's first-run setup. See config.example.toml in the repository for every option.\n{body}"))
    }

    /// The text of `profile.toml`, or `None` when there is nothing to write (blank vault and nothing about the person).
    pub fn render_profile(&self) -> Option<String> {
        let starter = self.pack == PACK_NAME;
        let has_person = !self.person.name.trim().is_empty() || !self.person.about.trim().is_empty();
        let has_location = !self.location.name.trim().is_empty();
        if !starter && !has_person && !has_location {
            return None;
        }
        let mut text = if starter { PACK_PROFILE.to_string() } else { "# Written by Loki's first-run setup. docs/PROFILE.md lists every field.\n".to_string() };
        let q = |s: &str| toml::Value::String(s.trim().to_string()).to_string();
        if has_person {
            text = set_in_section(&text, "person", "name", &q(&self.person.name));
            text = set_in_section(&text, "person", "about", &q(&self.person.about));
        }
        if has_location {
            text = set_in_section(&text, "location", "name", &q(&self.location.name));
            if let (Some(a), Some(o)) = (self.location.latitude, self.location.longitude) {
                text = set_in_section(&text, "location", "latitude", &format!("{a}"));
                text = set_in_section(&text, "location", "longitude", &format!("{o}"));
            }
        }
        Some(text)
    }
}

/// Sets `key = value` inside `[section]`, replacing the key's line if it exists (dropping its trailing comment) or adding it under the
/// header, and adding the section at the end if it is missing.
fn set_in_section(text: &str, section: &str, key: &str, value: &str) -> String {
    let header = format!("[{section}]");
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let Some(start) = lines.iter().position(|l| l.trim() == header) else {
        let mut out = text.trim_end().to_string();
        out.push_str(&format!("\n\n{header}\n{key} = {value}\n"));
        return out;
    };
    let end = lines.iter().enumerate().skip(start + 1).find(|(_, l)| l.trim_start().starts_with('[')).map(|(i, _)| i).unwrap_or(lines.len());
    let line = format!("{key} = {value}");
    if let Some(i) = (start + 1..end).find(|&i| {
        let t = lines[i].trim_start();
        t.starts_with(key) && t[key.len()..].trim_start().starts_with('=')
    }) {
        lines[i] = line;
    } else {
        lines.insert(start + 1, line);
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

fn write_private(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    std::fs::write(path, text)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// Writes everything for a validated request. Never overwrites an existing config.toml, profile.toml or command.
pub fn write_files(req: &SetupRequest, dir: &std::path::Path) -> Result<Vec<String>, String> {
    req.validate()?;
    let mut done = Vec::new();
    let conf = dir.join("config.toml");
    if conf.exists() {
        return Err("This server is already set up.".into());
    }
    let config_text = req.render_config()?;
    // The finished config must parse exactly the way the server will read it.
    toml::from_str::<crate::config::Config>(&config_text).map_err(|e| format!("The settings came out invalid: {e}"))?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    if let Some(profile) = req.render_profile() {
        crate::profile::Profile::from_toml(&profile).map_err(|e| format!("The profile came out invalid: {e}"))?;
        let p = dir.join("profile.toml");
        if !p.exists() {
            write_private(&p, &profile).map_err(|e| e.to_string())?;
            done.push("profile.toml".into());
        }
    }
    if req.pack == PACK_NAME {
        let cmds = dir.join("commands");
        std::fs::create_dir_all(&cmds).map_err(|e| e.to_string())?;
        for (name, text) in PACK_ROUTINES {
            let p = cmds.join(format!("{name}.md"));
            if !p.exists() {
                write_private(&p, text).map_err(|e| e.to_string())?;
            }
        }
        done.push(format!("{} routines", PACK_ROUTINES.len()));
    }
    crate::auth::set_credentials(&req.account.username, &req.account.password).map_err(|e| e.to_string())?;
    done.push("login".into());
    // config.toml goes last: its existence is what ends setup mode, so a failure above leaves setup available.
    write_private(&conf, &config_text).map_err(|e| e.to_string())?;
    done.push("config.toml".into());
    Ok(done)
}

// ---------- probes ----------

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Probe {
    Ollama { host: String, port: u16 },
    Obsidian { host: String, port: u16, api_key: String },
    Cloud { provider: String, #[serde(default)] base_url: String, api_key: String },
}

async fn probe(Json(p): Json<Probe>) -> Json<Value> {
    let client = match reqwest::Client::builder().timeout(std::time::Duration::from_secs(6)).build() {
        Ok(c) => c,
        Err(e) => return Json(json!({ "ok": false, "problem": "error", "message": e.to_string() })),
    };
    Json(match p {
        Probe::Ollama { host, port } => {
            if !host_ok(&host) {
                return Json(json!({ "ok": false, "problem": "bad_address", "message": "That address doesn't look right." }));
            }
            match client.get(format!("http://{host}:{port}/api/tags")).send().await {
                Ok(r) if r.status().is_success() => {
                    let v: Value = r.json().await.unwrap_or_default();
                    let models: Vec<String> = v["models"].as_array().map(|a| a.iter().filter_map(|m| m["name"].as_str().map(String::from)).collect()).unwrap_or_default();
                    let embed = models.iter().any(|m| m.starts_with("nomic-embed-text"));
                    json!({ "ok": true, "models": models, "has_embedding_model": embed })
                }
                Ok(r) => json!({ "ok": false, "problem": "status", "message": format!("Something answered but not like Ollama (HTTP {}).", r.status().as_u16()) }),
                Err(_) => json!({ "ok": false, "problem": "unreachable", "message": "Couldn't reach Ollama there. Is it running?" }),
            }
        }
        Probe::Obsidian { host, port, api_key } => {
            if !host_ok(&host) {
                return Json(json!({ "ok": false, "problem": "bad_address", "message": "That address doesn't look right." }));
            }
            match client.get(format!("http://{host}:{port}/vault/")).bearer_auth(api_key).send().await {
                Ok(r) if r.status().is_success() => json!({ "ok": true }),
                Ok(r) if r.status().as_u16() == 401 => json!({ "ok": false, "problem": "bad_key", "message": "The plugin answered but refused that key." }),
                Ok(r) => json!({ "ok": false, "problem": "status", "message": format!("Something answered but not like the Local REST API plugin (HTTP {}).", r.status().as_u16()) }),
                Err(_) => json!({ "ok": false, "problem": "unreachable", "message": "Couldn't reach the Local REST API plugin there. Is Obsidian open with the plugin on?" }),
            }
        }
        Probe::Cloud { provider, base_url, api_key } => {
            let req = if provider == "anthropic" {
                client.get("https://api.anthropic.com/v1/models").header("x-api-key", api_key).header("anthropic-version", "2023-06-01")
            } else if base_url.starts_with("http://") || base_url.starts_with("https://") {
                client.get(format!("{}/models", base_url.trim_end_matches('/'))).bearer_auth(api_key)
            } else {
                return Json(json!({ "ok": false, "problem": "bad_address", "message": "The base URL must start with https://." }));
            };
            match req.send().await {
                Ok(r) if r.status().is_success() => json!({ "ok": true }),
                Ok(r) if matches!(r.status().as_u16(), 401 | 403) => json!({ "ok": false, "problem": "bad_key", "message": "The provider refused that key." }),
                Ok(r) => json!({ "ok": false, "problem": "status", "message": format!("The provider answered with HTTP {}.", r.status().as_u16()) }),
                Err(_) => json!({ "ok": false, "problem": "unreachable", "message": "Couldn't reach the provider. Check the address and your connection." }),
            }
        }
    })
}

// ---------- routes ----------

/// True inside a container image (the Dockerfile sets this), where "this machine" for Ollama and Obsidian is the Docker host.
fn in_container() -> bool {
    std::env::var("LOKI_IN_CONTAINER").map(|v| v == "1").unwrap_or(false)
}

async fn state() -> Json<Value> {
    let (ollama, obsidian) = if in_container() { ("host.docker.internal", "host.docker.internal") } else { ("localhost", "127.0.0.1") };
    Json(json!({
        "mode": "setup",
        "container": in_container(),
        "packs": [{ "id": PACK_NAME, "label": "Zettelkasten + Personal Scrum", "routines": PACK_ROUTINES.len() }],
        "defaults": { "ollama_host": ollama, "ollama_port": 11434, "obsidian_host": obsidian, "obsidian_port": 27123 },
    }))
}

async fn commit(Json(req): Json<SetupRequest>) -> Response {
    let dir: PathBuf = match crate::config::config_dir() {
        Ok(d) => d,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    match write_files(&req, &dir) {
        Ok(done) => {
            // Answer first, then let the server move on into normal mode.
            tokio::spawn(async {
                tokio::time::sleep(std::time::Duration::from_millis(900)).await;
                finish();
            });
            Json(json!({ "ok": true, "wrote": done })).into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "ok": false, "message": e }))).into_response(),
    }
}

static FINISHED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static FINISHED_NOTIFY: tokio::sync::Notify = tokio::sync::Notify::const_new();

/// Resolves once setup has written its files. The server stops listening and carries on into normal mode in the same process, so
/// nothing depends on the OS (no exec, no second process) and services or containers never see an exit.
pub async fn wait_finished() {
    loop {
        let woken = FINISHED_NOTIFY.notified();
        if FINISHED.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        woken.await;
    }
}

fn finish() {
    FINISHED.store(true, std::sync::atomic::Ordering::SeqCst);
    FINISHED_NOTIFY.notify_waiters();
}

/// Refuses connections from other machines unless `LOKI_SETUP_ALLOW_REMOTE=1`.
async fn local_only(State(allow_remote): State<bool>, ConnectInfo(peer): ConnectInfo<SocketAddr>, req: Request, next: Next) -> Response {
    if allow_remote || peer.ip().is_loopback() {
        return next.run(req).await;
    }
    (StatusCode::FORBIDDEN, "Loki is not set up yet. Finish setup from the machine it runs on (http://127.0.0.1), or over an SSH tunnel to it.").into_response()
}

/// The whole server while in setup mode: the app shell plus the `/api/setup/*` routes.
pub fn router(static_dir: Option<PathBuf>) -> Router {
    let allow_remote = std::env::var("LOKI_SETUP_ALLOW_REMOTE").map(|v| v == "1").unwrap_or(false);
    let api = Router::new()
        .route("/api/setup/state", get(state))
        .route("/api/setup/probe", post(probe))
        .route("/api/setup/commit", post(commit))
        .layer(axum::middleware::from_fn_with_state(allow_remote, local_only));
    crate::server::with_app_shell(api, static_dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req() -> SetupRequest {
        SetupRequest {
            local: Some(Local { host: "localhost".into(), port: 11434, model: "llama3.1:8b".into() }),
            cloud: None,
            routing: String::new(),
            accept_cloud_only: false,
            obsidian: ObsidianIn { host: "127.0.0.1".into(), port: 27123, api_key: "abc".into() },
            pack: "none".into(),
            person: PersonIn::default(),
            location: LocationIn::default(),
            account: AccountIn { username: "sam".into(), password: "correct horse".into() },
        }
    }

    fn cloud(provider: &str) -> Cloud {
        Cloud { provider: provider.into(), base_url: "https://api.openai.com/v1".into(), api_key: "sk-1".into(), model: "m".into() }
    }

    #[test]
    fn every_combination_renders_a_config_the_server_can_read() {
        let mut both_local = req();
        both_local.cloud = Some(cloud("anthropic"));
        both_local.routing = "local".into();
        let mut both_cloud = req();
        both_cloud.cloud = Some(cloud("openai"));
        both_cloud.routing = "cloud".into();
        let mut cloud_only = req();
        cloud_only.local = None;
        cloud_only.cloud = Some(cloud("openai"));
        cloud_only.accept_cloud_only = true;
        let mut cloud_only_anthropic = req();
        cloud_only_anthropic.local = None;
        cloud_only_anthropic.cloud = Some(cloud("anthropic"));
        cloud_only_anthropic.accept_cloud_only = true;
        for (name, r) in [("local", req()), ("both, local first", both_local), ("both, cloud first", both_cloud), ("cloud only", cloud_only), ("cloud only, anthropic", cloud_only_anthropic)] {
            r.validate().unwrap_or_else(|e| panic!("{name}: {e}"));
            let text = r.render_config().unwrap();
            let c: crate::config::Config = toml::from_str(&text).unwrap_or_else(|e| panic!("{name}: {e}\n{text}"));
            assert!(!c.claude.enabled, "{name}: Claude Code stays off");
            assert_eq!(c.cloud_first, name == "both, cloud first", "{name}");
            assert_eq!(c.fallbacks.len(), usize::from(name.starts_with("both")), "{name}");
        }
    }

    #[test]
    fn cloud_only_needs_an_explicit_yes_and_one_model_is_required() {
        let mut r = req();
        r.local = None;
        r.cloud = Some(cloud("anthropic"));
        assert!(r.validate().unwrap_err().contains("everything you ask goes to your cloud provider"));
        r.accept_cloud_only = true;
        assert!(r.validate().is_ok());
        r.cloud = None;
        assert!(r.validate().unwrap_err().contains("at least one model"));
        let mut both = req();
        both.cloud = Some(cloud("anthropic"));
        assert!(both.validate().unwrap_err().contains("goes first"));
    }

    #[test]
    fn answers_are_checked_and_cannot_break_the_files() {
        let mut r = req();
        r.account.password = "short".into();
        assert!(r.validate().is_err());
        r.account.password = "long enough".into();
        r.account.username = "bad name".into();
        assert!(r.validate().is_err());
        r.account.username = "sam".into();
        r.obsidian.api_key = "line\nbreak".into();
        assert!(r.validate().is_err());
        r.obsidian.api_key = "k\"ey = 'x'".into();
        assert!(r.validate().is_ok(), "quotes are fine: they are escaped, not pasted");
        let text = r.render_config().unwrap();
        let c: crate::config::Config = toml::from_str(&text).unwrap();
        assert_eq!(c.obsidian.api_key, "k\"ey = 'x'");
        r.location.latitude = Some(95.0);
        r.location.longitude = Some(0.0);
        r.location.name = "X".into();
        assert!(r.validate().is_err());
    }

    #[test]
    fn the_profile_carries_the_person_and_location_into_either_layout() {
        let mut r = req();
        assert!(r.render_profile().is_none(), "blank vault and nobody to describe: no file");
        r.person = PersonIn { name: "Sam \"S\"".into(), about: "a nurse, two kids".into() };
        r.location = LocationIn { name: "Springfield".into(), latitude: Some(39.8), longitude: Some(-89.6) };
        let blank = r.render_profile().unwrap();
        let p = crate::profile::Profile::from_toml(&blank).unwrap();
        assert_eq!((p.person.name.as_str(), p.location.latitude), ("Sam \"S\"", Some(39.8)));
        assert_eq!(p.vault.inbox, "Inbox", "blank vault keeps the plain folder names");
        r.pack = PACK_NAME.into();
        let starter = r.render_profile().unwrap();
        let p = crate::profile::Profile::from_toml(&starter).unwrap();
        assert_eq!((p.person.about.as_str(), p.location.name.as_str(), p.vault.inbox.as_str()), ("a nurse, two kids", "Springfield", "00 - Inbox"));
        assert!(p.routines.len() == 11 && starter.contains("# Where the dashboard cards link to"), "the starter's comments survive");
    }

    #[test]
    fn writing_the_files_is_all_or_nothing_and_never_overwrites() {
        let dir = std::env::temp_dir().join(format!("loki-setup-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut r = req();
        r.pack = PACK_NAME.into();
        r.person.name = "Sam".into();
        // an invalid request writes nothing
        r.account.password = "x".into();
        assert!(write_files(&r, &dir).is_err() && !dir.join("config.toml").exists());
        r.account.password = "long enough".into();
        std::env::set_var("XDG_CONFIG_HOME", &dir);
        let done = write_files(&r, &dir).unwrap();
        assert!(done.contains(&"config.toml".to_string()) && dir.join("commands/bod.md").exists() && dir.join("profile.toml").exists());
        assert!(crate::auth::login("sam", "long enough").is_ok());
        assert!(write_files(&r, &dir).unwrap_err().contains("already set up"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
