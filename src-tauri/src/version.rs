//! Which version is running, and an update check that only talks to the public repository when the person presses the button.
//! Nothing here runs on a timer and nothing is sent about the install: it is a plain read of the repository's latest release.

use serde_json::{json, Value};

const REPO: &str = "Awzy11/Loki-Assistant";

pub fn current() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// How this copy was installed, which decides what "update" means: a container image, a release archive, or a build from source.
pub fn install_kind() -> &'static str {
    if std::env::var("LOKI_IN_CONTAINER").map(|v| v == "1").unwrap_or(false) {
        "docker"
    } else if std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("loki.sh").exists())).unwrap_or(false) {
        "archive"
    } else {
        "source"
    }
}

fn how_to_update(kind: &str) -> &'static str {
    match kind {
        "docker" => "Run `docker compose pull && docker compose up -d`. Your settings and login are in the data volume and stay put.",
        "archive" => "Download the new archive from the release page, stop Loki, and unpack it over the old folder. Your settings live in ~/.config/loki and are not touched.",
        _ => "Run `git pull`, then `npm ci && npm run build` and rebuild the server with `cargo build --release` in src-tauri.",
    }
}

pub fn info() -> Value {
    json!({ "version": current(), "install": install_kind() })
}

/// Numeric (major, minor, patch) of `v1.2.3` or `1.2.3-rc1`; anything unparsable is `None`.
fn parse(v: &str) -> Option<(u64, u64, u64)> {
    let core = v.trim().trim_start_matches('v').split(['-', '+']).next()?;
    let mut it = core.split('.').map(|p| p.parse::<u64>());
    Some((it.next()?.ok()?, it.next()?.ok()?, it.next()?.ok()?))
}

pub fn is_newer(latest: &str, current: &str) -> bool {
    matches!((parse(latest), parse(current)), (Some(l), Some(c)) if l > c)
}

pub async fn check() -> Result<Value, String> {
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(10)).user_agent(format!("loki-assistant/{}", current())).build().map_err(|e| e.to_string())?;
    let res = client.get(format!("https://api.github.com/repos/{REPO}/releases/latest")).send().await.map_err(|_| "Couldn't reach GitHub to check for updates.".to_string())?;
    if res.status().as_u16() == 404 {
        return Ok(json!({ "current": current(), "latest": null, "newer": false, "message": "No release has been published yet." }));
    }
    if !res.status().is_success() {
        return Err(format!("GitHub answered {}.", res.status()));
    }
    let v: Value = res.json().await.map_err(|e| e.to_string())?;
    let tag = v["tag_name"].as_str().unwrap_or_default().to_string();
    let newer = is_newer(&tag, current());
    let kind = install_kind();
    Ok(json!({
        "current": current(), "latest": tag, "newer": newer, "url": v["html_url"], "install": kind,
        "message": if newer { how_to_update(kind) } else { "You are on the latest version." },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions() {
        assert!(is_newer("v0.2.0", "0.1.0"));
        assert!(is_newer("0.1.1", "0.1.0"));
        assert!(!is_newer("v0.1.0", "0.1.0"));
        assert!(!is_newer("v0.1.0", "0.2.0"));
        assert!(is_newer("v0.10.0", "0.9.9"));
        assert!(!is_newer("garbage", "0.1.0"));
    }
}
