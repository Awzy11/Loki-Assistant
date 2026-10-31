//! Backs the Settings UI's read/write of `[llm]`/`[embedding]` config — a Tauri command pair
//! in `lib.rs` and an axum route pair in `server.rs` both call straight into this module.
//! Deliberately independent of the running `Core`/provider instances: saving here only
//! rewrites files on disk (`config.toml` + the encrypted `llm.secrets.toml`) and takes effect
//! on Loki's next restart — no hot-swap in Phase 1, per the plan ("don't over-build this now").
//!
//! Known gap, worth closing before this ever runs somewhere less trusted than a personal
//! LAN: these endpoints are unauthenticated today, same as `/api/message`/`/api/history` —
//! but unlike those, a write here can redirect Loki's model traffic (and BYOK API key) to
//! wherever the caller wants. Phase 2.5 adds bearer-token auth to every route; until then this
//! carries more risk than the existing routes did, not less.

use crate::config::{config_dir, secrets_path, ClaudeConfig};
use crate::secrets;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use std::collections::BTreeMap;
use std::fs;
use toml::Value as Toml;

fn config_path() -> anyhow::Result<std::path::PathBuf> {
    Ok(config_dir()?.join("config.toml"))
}

/// What the Settings UI sends when saving a backend. `api_key`, if present, is the plaintext
/// value the user just typed — encrypted before anything touches disk (see `save_backend`)
/// and never sent back on a subsequent read.
#[derive(Debug, Deserialize)]
pub struct BackendInput {
    pub vendor: String,
    #[serde(flatten)]
    pub fields: BTreeMap<String, Json>,
    #[serde(default)]
    pub api_key: Option<String>,
}

/// What a GET returns — same shape minus any secret material.
#[derive(Debug, Serialize)]
pub struct PublicBackend {
    pub vendor: String,
    #[serde(flatten)]
    pub fields: BTreeMap<String, Json>,
    pub has_api_key: bool,
}

fn read_toml() -> anyhow::Result<Toml> {
    let path = config_path()?;
    let raw = fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("couldn't read {} ({e})", path.display()))?;
    Ok(toml::from_str(&raw)?)
}

fn write_toml(doc: &Toml) -> anyhow::Result<()> {
    fs::write(config_path()?, toml::to_string_pretty(doc)?)?;
    Ok(())
}

fn read_secrets_toml() -> anyhow::Result<Toml> {
    match fs::read_to_string(secrets_path()?) {
        Ok(raw) => Ok(toml::from_str(&raw)?),
        Err(_) => Ok(Toml::Table(Default::default())),
    }
}

fn write_secrets_toml(doc: &Toml) -> anyhow::Result<()> {
    let path = secrets_path()?;
    fs::write(&path, toml::to_string_pretty(doc)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// `section` is `"llm"` or `"embedding"`.
pub fn get_backend(section: &str) -> anyhow::Result<Option<PublicBackend>> {
    let doc = read_toml()?;
    let Some(table) = doc.get(section).and_then(|v| v.as_table()) else {
        return Ok(None);
    };

    let secrets_doc = read_secrets_toml()?;
    let has_api_key = table.contains_key("api_key")
        || secrets_doc.get(section).and_then(|v| v.get("api_key")).is_some();

    let vendor = table.get("vendor").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let mut fields = BTreeMap::new();
    for (k, v) in table {
        if k == "api_key" || k == "vendor" {
            continue;
        }
        fields.insert(k.clone(), toml_to_json(v));
    }
    Ok(Some(PublicBackend { vendor, fields, has_api_key }))
}

pub fn save_backend(section: &str, input: BackendInput) -> anyhow::Result<()> {
    let mut doc = read_toml()?;
    let table = doc.as_table_mut().ok_or_else(|| anyhow::anyhow!("config.toml root isn't a table"))?;

    let mut new_section = toml::map::Map::new();
    new_section.insert("vendor".to_string(), Toml::String(input.vendor.clone()));
    for (k, v) in &input.fields {
        new_section.insert(k.clone(), json_to_toml(v));
    }
    table.insert(section.to_string(), Toml::Table(new_section));
    write_toml(&doc)?;

    if let Some(plaintext) = input.api_key {
        let mut secrets_doc = read_secrets_toml()?;
        let secrets_table = secrets_doc
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("llm.secrets.toml root isn't a table"))?;
        let mut entry = toml::map::Map::new();
        entry.insert("api_key".to_string(), Toml::String(secrets::encrypt(&plaintext)?));
        secrets_table.insert(section.to_string(), Toml::Table(entry));
        write_secrets_toml(&secrets_doc)?;
    }
    Ok(())
}

/// `[claude]` (`binary_path`/`timeout_secs`/`routine_working_dir`) doesn't fit the
/// vendor+api_key shape `get_backend`/`save_backend` assume — `save_backend` unconditionally
/// writes a `vendor` key, which has no meaning here. Reads/writes the `[claude]` table
/// directly instead, via the same `ClaudeConfig` struct `config::load` already deserializes
/// into, so there's exactly one definition of what fields exist.
pub fn get_claude_config() -> anyhow::Result<ClaudeConfig> {
    let doc = read_toml()?;
    match doc.get("claude") {
        Some(table) => Ok(table.clone().try_into()?),
        None => Ok(ClaudeConfig::default()),
    }
}

pub fn save_claude_config(input: ClaudeConfig) -> anyhow::Result<()> {
    let mut doc = read_toml()?;
    let table = doc.as_table_mut().ok_or_else(|| anyhow::anyhow!("config.toml root isn't a table"))?;
    table.insert("claude".to_string(), Toml::try_from(&input)?);
    write_toml(&doc)
}

fn toml_to_json(v: &Toml) -> Json {
    match v {
        Toml::String(s) => Json::String(s.clone()),
        Toml::Integer(i) => Json::Number((*i).into()),
        Toml::Float(f) => serde_json::Number::from_f64(*f).map(Json::Number).unwrap_or(Json::Null),
        Toml::Boolean(b) => Json::Bool(*b),
        Toml::Array(arr) => Json::Array(arr.iter().map(toml_to_json).collect()),
        Toml::Table(t) => Json::Object(t.iter().map(|(k, v)| (k.clone(), toml_to_json(v))).collect()),
        Toml::Datetime(dt) => Json::String(dt.to_string()),
    }
}

fn json_to_toml(v: &Json) -> Toml {
    match v {
        Json::Null => Toml::String(String::new()),
        Json::Bool(b) => Toml::Boolean(*b),
        Json::Number(n) => {
            if let Some(i) = n.as_i64() {
                Toml::Integer(i)
            } else if let Some(f) = n.as_f64() {
                Toml::Float(f)
            } else {
                Toml::String(n.to_string())
            }
        }
        Json::String(s) => Toml::String(s.clone()),
        Json::Array(arr) => Toml::Array(arr.iter().map(json_to_toml).collect()),
        Json::Object(map) => {
            let mut t = toml::map::Map::new();
            for (k, v) in map {
                t.insert(k.clone(), json_to_toml(v));
            }
            Toml::Table(t)
        }
    }
}
