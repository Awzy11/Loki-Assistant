pub mod agent;
pub mod agent_limits;
pub mod auth;
pub mod bridge;
pub mod bus;
pub mod claude;
pub mod commands;
pub mod config;
pub mod config_api;
pub mod content_classifier;
pub mod context_stats;
pub mod context_window;
pub mod conversation_log;
pub mod device;
pub mod core;
pub mod engines;
pub mod audit;
pub mod event_log;
pub mod events;
pub mod homelab;
pub mod setup;
pub mod extract;
pub mod improve;
pub mod lessons;
pub mod troubleshoot;
pub mod knowledge;
pub mod local_context;
pub mod mail;
pub mod mail_inbox;
pub mod mail_review;
pub mod mcp_hub;
pub mod backlog_edit;
pub mod note_tasks;
pub mod newsletters;
pub mod notes_ai;
pub mod profile;
pub mod mcp_manage;
pub mod needs_you;
pub mod mcp_server;
pub mod obsidian;
pub mod activity_log;
pub mod permission_rules;
pub mod permissions;
pub mod providers;
pub mod push;
pub mod router;
pub mod secrets;
pub mod server;
pub mod skills;
pub mod speech;
pub mod tool_policy;
pub mod session_store;

/// #13 (device-context snapshot) — the one deliberate, narrow exception to the thin-client
/// refactor's "no local IPC" rule (see `run`'s doc comment below). Shells out to `grim`
/// (Wayland/wlroots screenshot tool — already present on this Hyprland/Omarchy machine) to
/// capture the whole screen, returns it as a base64 PNG string for the frontend to attach to
/// a chat message via `Message::images`. Reachable only from the origin explicitly allowlisted
/// in `capabilities/remote-capture.json` — every other Tauri IPC surface stays closed.
///
/// Full-screen only, not the active window specifically — `grim` can target a geometry (via
/// `slurp` for interactive selection, or `hyprctl activewindow` for the focused window's
/// exact geometry programmatically) but that's a `-g` flag away if narrower scope is ever
/// wanted; whole-screen is the simpler default for "look at what I'm doing" context.
#[tauri::command]
fn capture_screenshot() -> Result<String, String> {
    use base64::Engine;
    let output =
        std::process::Command::new("grim").arg("-t").arg("png").arg("-").output().map_err(|e| format!("couldn't run grim: {e}"))?;
    if !output.status.success() {
        return Err(format!("grim exited with {}: {}", output.status, String::from_utf8_lossy(&output.stderr)));
    }
    Ok(base64::engine::general_purpose::STANDARD.encode(&output.stdout))
}

/// Runs one `device_*` tool (file read/list/write/edit, shell) on this machine on behalf of the
/// page, which relays requests from `loki-server`. The single device entry point — see
/// `device.rs` for the policy it enforces (protected paths, `[desktop]` switches). Async so a
/// slow shell command doesn't block the webview's IPC thread.
#[tauri::command]
async fn device_call(name: String, input: serde_json::Value) -> Result<String, String> {
    let cfg = config::load_desktop();
    tauri::async_runtime::spawn_blocking(move || device::call(&name, &input, &cfg))
        .await
        .map_err(|e| e.to_string())?
}

/// Loki's desktop app is a thin native shell (2026-08-30) — no embedded `Core`, no local
/// knowledge store, no local Ollama/Obsidian credentials on this machine at all. It just opens
/// a window pointed at a real `loki-server` instance's URL (`config::DesktopConfig`). The
/// bundled React frontend already knows how to be a plain web client — bearer-token `fetch`
/// calls via `transport.ts`'s `isTauri() === false` path — and Tauri only injects its IPC
/// bridge into locally-loaded content by default, not a remote origin the window navigates to
/// (no `dangerousRemoteDomainIpcAccess`/`remote` capability configured for most origins), so
/// `isTauri()` correctly evaluates false in this window with zero frontend changes needed. One
/// `Core` ever running anywhere — on whichever machine runs `loki-server` — not a separate
/// instance per device. One deliberate, narrow exception (#13, 2026-08-31): the specific
/// `loki-server` origin this window loads *is* granted IPC access, via
/// `capabilities/remote-capture.json` — but scoped to that one origin and carrying zero
/// `core:default`/plugin permissions, so `capture_screenshot` (below) is the only thing it can
/// actually reach, not a reopening of the old embedded-IPC surface.
///
/// Replaces an earlier version of this function that ran its own embedded `Core` behind Tauri
/// IPC commands (`send_message_v1`, `get_history_v1`, etc.) — removed entirely, not
/// deauthorized or left dormant, once "one unified information pool, not separate instances"
/// became the explicit goal: a desktop app on a different machine than `loki-server` had a
/// fully separate session/knowledge pool under that model, which is exactly what this fixes.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let desktop_cfg = config::load_desktop();
    let url: tauri::Url = desktop_cfg
        .server_url
        .parse()
        .unwrap_or_else(|e| panic!("desktop.server_url '{}' isn't a valid URL: {e}", desktop_cfg.server_url));

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![capture_screenshot, device_call])
        .setup(move |app| {
            use tauri_plugin_opener::OpenerExt;
            // This window is a view on the server's web app. Anything that leaves that origin (a Gmail, news or
            // obsidian:// link) goes to the system's handler instead of loading inside the window, where the
            // webview can't show it ("This URL can't be shown").
            let origin = (url.scheme().to_string(), url.host_str().map(str::to_string), url.port_or_known_default());
            let same = move |u: &tauri::Url| matches!(u.scheme(), "about" | "data" | "blob") || (u.scheme() == origin.0 && u.host_str().map(str::to_string) == origin.1 && u.port_or_known_default() == origin.2);
            let (h1, same1) = (app.handle().clone(), same.clone());
            let h2 = app.handle().clone();
            tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::External(url))
                .title("Loki")
                .inner_size(1000.0, 720.0)
                .on_navigation(move |u| {
                    if same1(u) {
                        return true;
                    }
                    let _ = h1.opener().open_url(u.as_str(), None::<&str>);
                    false
                })
                .on_new_window(move |u, _features| {
                    if !same(&u) {
                        let _ = h2.opener().open_url(u.as_str(), None::<&str>);
                    }
                    tauri::webview::NewWindowResponse::Deny
                })
                .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
