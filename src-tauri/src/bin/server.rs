use loki_lib::core::Core;
use loki_lib::{auth, config, server};
use std::path::PathBuf;
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Defaults to INFO if RUST_LOG isn't set — enough for TraceLayer's per-request lines
    // (method, path, status, latency) without needing to opt in explicitly every deploy.
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let bind = std::env::var("LOKI_BIND").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port: u16 = std::env::var("LOKI_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(8420);
    let static_dir: PathBuf = std::env::var("LOKI_STATIC_DIR").unwrap_or_else(|_| "dist".to_string()).into();
    let static_dir = if static_dir.exists() {
        Some(static_dir)
    } else {
        eprintln!(
            "warning: static dir '{}' not found — serving API only, no frontend. \
             Run `npm run build` and either set LOKI_STATIC_DIR or run this from the project root.",
            static_dir.display()
        );
        None
    };

    // No config.toml yet: serve the first-run setup in the browser instead of failing. It writes the config and a login, then this
    // process stops serving setup and carries on below in normal mode (no restart, so it behaves the same on every OS and in a container).
    if loki_lib::setup::needs_setup() {
        let addr = format!("{bind}:{port}");
        println!("Loki is not set up yet. Open http://{addr} in a browser on this machine to finish setup.");
        let listener = tokio::net::TcpListener::bind(&addr).await?;
        let serving = axum::serve(listener, loki_lib::setup::router(static_dir.clone()).into_make_service_with_connect_info::<std::net::SocketAddr>())
            .with_graceful_shutdown(loki_lib::setup::wait_finished());
        // A browser holding a connection open must not keep setup alive for long.
        tokio::select! {
            r = serving => r?,
            _ = async { loki_lib::setup::wait_finished().await; tokio::time::sleep(std::time::Duration::from_secs(5)).await } => {}
        }
        println!("Setup finished; starting Loki.");
    }

    // First run only — every startup after this finds credentials.toml already there and
    // returns None. The plaintext password is never written to a log file or persisted
    // anywhere; this stdout line is genuinely the only place it's ever visible.
    if let Some(password) = auth::ensure_credentials_exist()? {
        println!("==============================================================");
        println!(" First run — created login credentials for the Loki API/UI:");
        println!("   username: loki");
        println!("   password: {password}");
        println!(" This is the only time this password is shown. Log in once per");
        println!(" device via the Loki app's login screen — it exchanges this for");
        println!(" the same long-lived API token every other integration uses.");
        println!("==============================================================");
    }

    // The vault layout comes from ~/.config/loki/profile.toml (blank-vault defaults if there is none). A broken profile stops the
    // server here instead of letting it write notes into the wrong folders.
    let profile = loki_lib::profile::init().map_err(|e| anyhow::anyhow!(e))?;
    tracing::info!(inbox = %profile.vault.inbox, backlog = %profile.vault.backlog, areas = profile.backlog.areas.len(), "vault profile loaded");
    let cfg = config::load()?;
    let core = Arc::new(Core::new(cfg));
    core.spawn_knowledge_reconciliation();
    core.spawn_title_backfill();
    core.spawn_mcp_connect();
    loki_lib::homelab::finance_sync_spawn_loop();

    let app = server::router(core, static_dir);
    let addr = format!("{bind}:{port}");
    println!("Loki web server listening on http://{addr}");

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
