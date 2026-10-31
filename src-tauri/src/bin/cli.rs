//! `loki-cli` — the Loki client protocol (docs/PROTOCOL.md) as a command-line tool. Exists so
//! clients that can't comfortably embed an HTTP/SSE stack (the QML sidebar) still speak the full
//! protocol: they spawn this and read one JSON event per line. It also runs `device_*` tools
//! requested by the server (same policy as the desktop app) and answers them itself.
//!
//!   loki-cli chat <session> <message> [--no-device]   stream a turn (NDJSON events; exit 0 on done)
//!   loki-cli watch <session>                          attach and stream every event (reconnects)
//!   loki-cli respond <request-id> '<json>'            answer an approval / device request
//!   loki-cli always <tool> '<input json>'             save an "always allow" rule
//!   loki-cli rate <session> <index> up|down|none | audit <session> <index> | cancel <session>
//!   loki-cli lessons | lesson <id> approve|reject|delete|edit [text] | troubleshoot <session> <note> [info-json]
//!   loki-cli sessions | history <session> | engines [<session>] | engine <id> [<session>]
//!
//! Config: server URL from `[desktop] server_url` in ~/.config/loki/config.toml (or $LOKI_URL),
//! token from ~/.config/loki/api_token (or $LOKI_TOKEN). Killing `chat` closes the stream, which
//! cancels the turn on the server (that is "Stop").
use futures_util::StreamExt;
use loki_lib::{config, device};
use serde_json::{json, Value};

struct Ctx {
    base: String,
    token: String,
    http: reqwest::Client,
    client: Value,
}

impl Ctx {
    fn new(device_enabled: bool) -> anyhow::Result<Self> {
        let base = std::env::var("LOKI_URL").unwrap_or_else(|_| config::load_desktop().server_url).trim_end_matches('/').to_string();
        let token = match std::env::var("LOKI_TOKEN") {
            Ok(t) => t,
            Err(_) => {
                let p = loki_lib::paths::config_dir().ok_or_else(|| anyhow::anyhow!("no config dir"))?.join("loki").join("api_token");
                std::fs::read_to_string(&p).map_err(|e| anyhow::anyhow!("couldn't read {} ({e}); set LOKI_TOKEN", p.display()))?.trim().to_string()
            }
        };
        let id = std::env::var("LOKI_CLIENT_ID").unwrap_or_else(|_| "sidebar-main".to_string());
        let kind = std::env::var("LOKI_CLIENT_KIND").unwrap_or_else(|_| "sidebar".to_string());
        let caps: Vec<&str> = if device_enabled { vec!["device_fs"] } else { vec![] };
        Ok(Self { base, token, http: reqwest::Client::new(), client: json!({ "id": id, "kind": kind, "capabilities": caps }) })
    }

    fn get(&self, path: &str) -> reqwest::RequestBuilder {
        self.http.get(format!("{}{path}", self.base)).bearer_auth(&self.token)
    }

    fn post(&self, path: &str) -> reqwest::RequestBuilder {
        self.http.post(format!("{}{path}", self.base)).bearer_auth(&self.token)
    }

    async fn respond(&self, id: &str, value: Value) {
        let _ = self.post("/api/v1/bridge/respond").json(&json!({ "id": id, "value": value })).send().await;
    }
}

/// Prints the event as one JSON line; if it is a device request, runs it locally and answers.
async fn emit(ctx: &Ctx, ev: &Value, device_enabled: bool) {
    println!("{ev}");
    if device_enabled && ev["type"] == "device_request" {
        let (id, name, input) = (ev["id"].as_str().unwrap_or_default().to_string(), ev["name"].as_str().unwrap_or_default().to_string(), ev["input"].clone());
        let cfg = config::load_desktop();
        let result = tokio::task::spawn_blocking(move || device::call(&name, &input, &cfg)).await.unwrap_or_else(|e| Err(e.to_string()));
        let value = match result {
            Ok(content) => json!({ "content": content, "is_error": false }),
            Err(content) => json!({ "content": content, "is_error": true }),
        };
        ctx.respond(&id, value).await;
    }
}

async fn chat(session: &str, message: &str, device_enabled: bool) -> anyhow::Result<i32> {
    let ctx = Ctx::new(device_enabled)?;
    let resp = ctx
        .post("/api/v1/chat/stream")
        .json(&json!({ "session_id": session, "message": message, "client": ctx.client }))
        .send()
        .await?;
    if !resp.status().is_success() {
        println!("{}", json!({ "type": "error", "message": format!("HTTP {}: {}", resp.status(), resp.text().await.unwrap_or_default()) }));
        return Ok(1);
    }
    let mut stream = resp.bytes_stream();
    let mut buf = String::new();
    let mut code = 1;
    'outer: while let Some(chunk) = stream.next().await {
        buf.push_str(&String::from_utf8_lossy(&chunk?));
        while let Some(i) = buf.find("\n\n") {
            let frame: String = buf.drain(..i + 2).collect();
            let data: String = frame.lines().filter_map(|l| l.strip_prefix("data:")).map(|l| l.trim_start()).collect::<Vec<_>>().join("\n");
            let Ok(ev) = serde_json::from_str::<Value>(&data) else { continue };
            emit(&ctx, &ev, device_enabled).await;
            match ev["type"].as_str() {
                Some("done") => {
                    code = 0;
                    break 'outer;
                }
                Some("error") => break 'outer,
                _ => {}
            }
        }
    }
    Ok(code)
}

async fn watch(session: &str) -> anyhow::Result<()> {
    let ctx = Ctx::new(true)?;
    loop {
        let q = [
            ("session_id", session.to_string()),
            ("client_id", ctx.client["id"].as_str().unwrap_or("sidebar").to_string()),
            ("kind", ctx.client["kind"].as_str().unwrap_or("sidebar").to_string()),
            ("caps", "device_fs".to_string()),
        ];
        if let Ok(resp) = ctx.get("/api/v1/sessions/events").query(&q).send().await {
            if resp.status().is_success() {
                let mut stream = resp.bytes_stream();
                let mut buf = String::new();
                while let Some(Ok(chunk)) = stream.next().await {
                    buf.push_str(&String::from_utf8_lossy(&chunk));
                    while let Some(i) = buf.find("\n\n") {
                        let frame: String = buf.drain(..i + 2).collect();
                        let data: String = frame.lines().filter_map(|l| l.strip_prefix("data:")).map(|l| l.trim_start()).collect::<Vec<_>>().join("\n");
                        if let Ok(ev) = serde_json::from_str::<Value>(&data) {
                            emit(&ctx, &ev, true).await;
                        }
                    }
                }
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}

async fn print_json(req: reqwest::RequestBuilder) -> anyhow::Result<i32> {
    let resp = req.send().await?;
    let ok = resp.status().is_success();
    println!("{}", resp.text().await?);
    Ok(if ok { 0 } else { 1 })
}


// ---------- "Fix with Claude": make the changes a troubleshoot report identified ----------
//
// Runs on the machine that has the source (the laptop). Claude edits an isolated git worktree
// (branch `fix/<id>`) and a staged copy of the sidebar plugin — nothing live changes until
// `fix-apply`. Claude gets file edit tools plus build/test and local-git commands only: no
// push, no network, no deploy.

fn home() -> std::path::PathBuf {
    dirs::home_dir().unwrap_or_default()
}

fn repo_dir() -> std::path::PathBuf {
    std::env::var("LOKI_REPO").map(Into::into).unwrap_or_else(|_| home().join("Desktop/Loki"))
}

fn fixes_dir() -> std::path::PathBuf {
    home().join(".local/share/loki-fixes")
}

fn plugin_dir() -> std::path::PathBuf {
    if let Ok(d) = std::env::var("LOKI_SIDEBAR_DIR") {
        return std::path::PathBuf::from(d);
    }
    // Plugin folders are named `<author>.<id>`; take the one that ends in loki-sidebar (or `loki.sidebar`).
    let plugins = home().join(".config/omarchy/plugins");
    std::fs::read_dir(&plugins)
        .ok()
        .and_then(|rd| rd.flatten().map(|e| e.path()).find(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.ends_with("loki-sidebar") || n == "loki.sidebar")))
        .unwrap_or_else(|| plugins.join("loki.sidebar"))
}

fn tool_path() -> String {
    let h = home();
    format!(
        "{}:{}:{}:{}",
        h.join(".cargo/bin").display(),
        h.join(".local/share/mise/shims").display(),
        h.join(".local/bin").display(),
        std::env::var("PATH").unwrap_or_default()
    )
}

fn valid_job_id(id: &str) -> bool {
    id.len() > 1 && id.len() < 40 && id.chars().all(|c| c.is_ascii_alphanumeric())
}

fn git(dir: &std::path::Path, args: &[&str]) -> anyhow::Result<String> {
    let out = std::process::Command::new("git").arg("-C").arg(dir).args(args).env("PATH", tool_path()).output()?;
    if !out.status.success() {
        anyhow::bail!("git {} failed: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

static LAST_OUTCOME: std::sync::Mutex<Option<Value>> = std::sync::Mutex::new(None);

fn say(v: Value) -> anyhow::Result<i32> {
    let ok = v["status"] == "done";
    *LAST_OUTCOME.lock().unwrap() = Some(v.clone());
    println!("{v}");
    Ok(if ok { 0 } else { 1 })
}

fn fail(msg: impl Into<String>) -> anyhow::Result<i32> {
    say(json!({ "status": "error", "result": msg.into() }))
}

/// Lists files that differ between the live sidebar plugin and its staged copy.
fn sidebar_changes(staged: &std::path::Path) -> Vec<String> {
    let live = plugin_dir();
    let mut changed = Vec::new();
    if let Ok(rd) = std::fs::read_dir(staged) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name == "LokiSidebar.qml" || !e.path().is_file() {
                continue; // LokiSidebar.qml is generated from LokiSidebar.v2.qml
            }
            let a = std::fs::read(e.path()).unwrap_or_default();
            let b = std::fs::read(live.join(&name)).unwrap_or_default();
            if a != b {
                changed.push(name);
            }
        }
    }
    changed.sort();
    changed
}

async fn fix(job_id: &str, note: &str) -> anyhow::Result<i32> {
    if !valid_job_id(job_id) {
        return fail("bad job id");
    }
    let ctx = Ctx::new(false)?;
    let job: Value = ctx.get(&format!("/api/v1/troubleshoot/{job_id}")).send().await?.json().await?;
    if job["status"] != "done" {
        return fail("That troubleshoot report isn't finished (or failed) — there is nothing to fix from.");
    }
    let report = job["result"].as_str().unwrap_or_default().to_string();
    let repo = repo_dir();
    let root = fixes_dir().join(job_id);
    let wt = root.join("repo");
    let staged = root.join("sidebar");
    if root.exists() {
        return fail("A fix for this report already exists — apply or discard it first.");
    }
    std::fs::create_dir_all(&root)?;
    let base_branch = git(&repo, &["branch", "--show-current"])?;
    let base_sha = git(&repo, &["rev-parse", "HEAD"])?;
    let branch = format!("fix/{job_id}");
    git(&repo, &["worktree", "add", "-q", "-b", &branch, wt.to_str().unwrap_or_default(), "HEAD"])?;
    let _ = std::os::unix::fs::symlink(repo.join("node_modules"), wt.join("node_modules"));
    let _ = std::process::Command::new("cp").arg("-a").arg(plugin_dir()).arg(&staged).status();

    let prompt = format!(
        "You are fixing problems in Loki, the user's personal AI agent (Rust server in src-tauri/, React web app in src/, \
and a Quickshell/QML desktop sidebar). A troubleshooting review found issues; fix what it identified.\n\n\
## Troubleshooting report\n{report}\n\n## the user's note\n{note}\n\n\
## Rules\n\
- Your working directory is an isolated git worktree of the Loki repo (branch {branch}). Edit files here.\n\
- The sidebar plugin is NOT in the repo: a staged copy is at {staged}. Edit LokiSidebar.v2.qml there (LokiSidebar.qml is generated from it by the apply step — do not edit it). Shared JS is LokiEvents.js.\n\
- Make the smallest change that fixes each identified problem. Do not refactor, rename, or add features beyond that. If a finding needs a decision from the user or can't be fixed from the code, say so instead of guessing.\n\
- Verify before finishing: `cd src-tauri && cargo check` (and `cargo test` if you changed Rust logic), `npx tsc --noEmit` and `npm run build` for web changes. For QML, re-read your edit carefully (no runtime available).\n\
- Commit your repo changes on this branch (`git add -A` then `git commit -m \"fix: ...\"`). Do not push, do not deploy, do not touch anything outside the worktree and the staged sidebar copy.\n\
- End with a short plain summary: what you changed (file + why), what you verified, and anything you did NOT fix.",
        staged = staged.display()
    );
    let allowed = [
        "Read", "Grep", "Glob", "Edit", "Write", "Bash(cargo check:*)", "Bash(cargo test:*)", "Bash(cargo build:*)", "Bash(npx tsc:*)",
        "Bash(npm run build:*)", "Bash(git status:*)", "Bash(git diff:*)", "Bash(git add:*)", "Bash(git commit:*)", "Bash(git log:*)",
    ];
    let mut cmd = tokio::process::Command::new("claude");
    cmd.current_dir(&wt)
        .env("PATH", tool_path())
        .env("CARGO_TARGET_DIR", fixes_dir().join("target"))
        .args(["-p", &prompt, "--output-format", "text", "--setting-sources", "", "--no-session-persistence", "--permission-mode", "acceptEdits", "--tools", "Read,Grep,Glob,Edit,Write,Bash", "--allowedTools"])
        .args(allowed)
        .args(["--disallowedTools", "Bash(git push:*)", "WebFetch", "WebSearch", "--add-dir"])
        .arg(&staged)
        .kill_on_drop(true);
    let out = match tokio::time::timeout(std::time::Duration::from_secs(25 * 60), cmd.output()).await {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => return fail(format!("couldn't run claude: {e}")),
        Err(_) => return fail("Claude took longer than 25 minutes; the work so far is in the fix worktree (discard it to clean up)."),
    };
    let summary = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() && summary.is_empty() {
        return fail(format!("claude failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    // If Claude left changes uncommitted, commit them so the diff is complete.
    let _ = git(&wt, &["add", "-A"]);
    let _ = git(&wt, &["commit", "-q", "-m", &format!("fix: changes from troubleshoot {job_id}")]);
    let diffstat = git(&repo, &["diff", "--stat", &format!("{base_sha}..{branch}")]).unwrap_or_default();
    let changed_files = git(&repo, &["diff", "--name-only", &format!("{base_sha}..{branch}")]).unwrap_or_default();
    let backend = changed_files.lines().any(|f| f.starts_with("src-tauri/"));
    let frontend = changed_files.lines().any(|f| f.starts_with("src/") || f == "index.html" || f.starts_with("public/"));
    let sidebar = sidebar_changes(&staged);
    let meta = json!({ "base_branch": base_branch, "base_sha": base_sha, "branch": branch, "backend": backend, "frontend": frontend, "sidebar": sidebar });
    std::fs::write(root.join("meta.json"), meta.to_string())?;
    say(json!({ "status": "done", "result": summary, "diffstat": diffstat, "backend": backend, "frontend": frontend, "sidebar": sidebar, "nothing_changed": changed_files.trim().is_empty() && sidebar.is_empty() }))
}

async fn fix_apply(job_id: &str) -> anyhow::Result<i32> {
    if !valid_job_id(job_id) {
        return fail("bad job id");
    }
    let repo = repo_dir();
    let root = fixes_dir().join(job_id);
    let meta: Value = serde_json::from_str(&std::fs::read_to_string(root.join("meta.json")).map_err(|_| anyhow::anyhow!("no pending fix with that id"))?)?;
    let branch = meta["branch"].as_str().unwrap_or_default().to_string();
    if !git(&repo, &["status", "--porcelain", "--untracked-files=no"])?.is_empty() {
        return fail("The Loki repo has uncommitted changes — commit or stash them, then apply again.");
    }
    if git(&repo, &["branch", "--show-current"])? != meta["base_branch"].as_str().unwrap_or_default() {
        return fail("The repo is on a different branch than when the fix started — switch back, then apply.");
    }
    let mut log = vec![];
    if git(&repo, &["rev-parse", &branch]).is_ok() && git(&repo, &["rev-list", "--count", &format!("{}..{branch}", meta["base_sha"].as_str().unwrap_or("HEAD"))]).unwrap_or_default() != "0" {
        if let Err(e) = git(&repo, &["merge", "--no-edit", &branch]) {
            let _ = git(&repo, &["merge", "--abort"]);
            return fail(format!("Merge conflict — nothing was applied. {e}"));
        }
        log.push(format!("merged {branch}"));
    }
    let deploy_arg = if meta["backend"] == true { Some(None) } else if meta["frontend"] == true { Some(Some("--frontend")) } else { None };
    if let Some(arg) = deploy_arg {
        let mut c = std::process::Command::new(repo.join("ops/deploy.sh"));
        if let Some(a) = arg {
            c.arg(a);
        }
        let out = c.current_dir(&repo).env("PATH", tool_path()).output()?;
        if !out.status.success() {
            return fail(format!("Merged, but deploy failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
        }
        log.push(if arg.is_some() { "web app deployed" } else { "server + web app deployed and restarted" }.to_string());
    }
    let _ = git(&repo, &["worktree", "remove", "--force", root.join("repo").to_str().unwrap_or_default()]);
    let _ = git(&repo, &["branch", "-D", &branch]);

    // The sidebar swap goes last, in a detached helper: replacing the plugin makes Quickshell reload
    // the sidebar, which would otherwise kill this process halfway through its own update.
    let files: Vec<String> = meta["sidebar"].as_array().map(|a| a.iter().filter_map(|f| f.as_str().map(String::from)).collect()).unwrap_or_default();
    if files.is_empty() {
        let _ = std::fs::remove_dir_all(&root);
    } else {
        let bak = fixes_dir().join(format!("sidebar-backup-{job_id}"));
        let _ = std::process::Command::new("cp").arg("-a").arg(plugin_dir()).arg(&bak).status();
        let staged = root.join("sidebar");
        let live = plugin_dir();
        let mut script = String::from("sleep 1; ");
        for f in &files {
            script.push_str(&format!("cp '{}' '{}'; ", staged.join(f).display(), live.join(f).display()));
        }
        if files.iter().any(|f| f == "LokiSidebar.v2.qml") {
            script.push_str(&format!("cp '{}' '{}'; ", live.join("LokiSidebar.v2.qml").display(), live.join("LokiSidebar.qml").display()));
        }
        script.push_str(&format!("rm -rf '{}'", root.display()));
        let _ = std::process::Command::new("setsid").args(["sh", "-c", &script]).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn();
        log.push(format!("sidebar updated ({} file(s); backup at {}) — it reloads in a moment", files.len(), bak.display()));
    }
    say(json!({ "status": "done", "result": log.join("; ") }))
}

async fn fix_discard(job_id: &str) -> anyhow::Result<i32> {
    if !valid_job_id(job_id) {
        return fail("bad job id");
    }
    let repo = repo_dir();
    let root = fixes_dir().join(job_id);
    let _ = git(&repo, &["worktree", "remove", "--force", root.join("repo").to_str().unwrap_or_default()]);
    let _ = git(&repo, &["branch", "-D", &format!("fix/{job_id}")]);
    let _ = std::fs::remove_dir_all(&root);
    say(json!({ "status": "done", "result": "Discarded — nothing was changed." }))
}

/// Sends the conversation plus this machine's screen to Claude for diagnosis and prints the
/// outcome as one JSON line. `info` is optional JSON the caller knows about its own rendering.
async fn troubleshoot(session: &str, note: &str, info: Option<&str>) -> anyhow::Result<i32> {
    use base64::Engine as _;
    let ctx = Ctx::new(false)?;
    // A real screenshot of what the sidebar looks like (scaled down to keep the upload small).
    let shot = std::process::Command::new("grim").args(["-s", "0.6", "-t", "png", "-"]).output().ok().filter(|o| o.status.success() && !o.stdout.is_empty());
    let screenshot_b64 = shot.map(|o| base64::engine::general_purpose::STANDARD.encode(o.stdout));
    let mut info: Value = info.and_then(|i| serde_json::from_str(i).ok()).unwrap_or_else(|| json!({}));
    info["screenshot_taken"] = json!(screenshot_b64.is_some());
    let ui = json!({ "kind": "sidebar", "info": info, "rendered": std::env::var("LOKI_RENDERED").unwrap_or_default(), "screenshot_b64": screenshot_b64 });
    let started: Value = ctx.post("/api/v1/troubleshoot").json(&json!({ "session_id": session, "note": note, "ui": ui })).send().await?.json().await?;
    let Some(id) = started["id"].as_str() else {
        println!("{}", json!({ "status": "error", "result": started.to_string() }));
        return Ok(1);
    };
    for _ in 0..120 {
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        let job: Value = ctx.get(&format!("/api/v1/troubleshoot/{id}")).send().await?.json().await?;
        if job["status"] != "running" {
            println!("{job}");
            return Ok(if job["status"] == "done" { 0 } else { 1 });
        }
    }
    println!("{}", json!({ "status": "error", "result": "Timed out waiting for Claude." }));
    Ok(1)
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let a = |i: usize| args.get(i).cloned().unwrap_or_default();
    let result: anyhow::Result<i32> = async {
        match a(0).as_str() {
            "chat" if args.len() >= 3 => chat(&a(1), &a(2), !args.iter().any(|x| x == "--no-device")).await,
            "watch" if args.len() >= 2 => watch(&a(1)).await.map(|_| 0),
            "respond" if args.len() >= 3 => {
                let ctx = Ctx::new(false)?;
                ctx.respond(&a(1), serde_json::from_str(&a(2))?).await;
                Ok(0)
            }
            "always" if args.len() >= 3 => {
                let ctx = Ctx::new(false)?;
                print_json(ctx.post("/api/v1/permissions/rules").json(&json!({ "tool": a(1), "input": serde_json::from_str::<Value>(&a(2))? }))).await
            }
            "sessions" => print_json(Ctx::new(false)?.get("/api/v1/sessions")).await,
            "rename" if args.len() >= 3 => {
                let ctx = Ctx::new(false)?;
                print_json(ctx.post("/api/v1/sessions/rename").json(&json!({ "session_id": a(1), "title": a(2) }))).await
            }
            "rate" if args.len() >= 4 => {
                let ctx = Ctx::new(false)?;
                print_json(ctx.post("/api/v1/feedback").json(&json!({ "session_id": a(1), "index": a(2).parse::<u64>().unwrap_or(0), "rating": a(3) }))).await
            }
            "audit" if args.len() >= 3 => {
                let ctx = Ctx::new(false)?;
                print_json(ctx.post("/api/v1/audit").json(&json!({ "session_id": a(1), "index": a(2).parse::<u64>().unwrap_or(0) })).timeout(std::time::Duration::from_secs(300))).await
            }
            "cancel" if args.len() >= 2 => {
                let ctx = Ctx::new(false)?;
                print_json(ctx.post("/api/v1/chat/cancel").json(&json!({ "session_id": a(1) }))).await
            }
            "fix" | "fix-apply" | "fix-discard" if args.len() >= 2 => {
                let action = match a(0).as_str() {
                    "fix" => "fix",
                    "fix-apply" => "apply",
                    _ => "discard",
                };
                if action == "apply" {
                    // Applying restarts the server, so mark it up front: devices show "applying".
                    if let Ok(ctx) = Ctx::new(false) {
                        let _ = ctx.post(&format!("/api/v1/reports/{}/fix-record", a(1))).json(&json!({ "action": "applying", "outcome": { "status": "done" } })).send().await;
                    }
                }
                let code = match action {
                    "fix" => fix(&a(1), &a(2)).await,
                    "apply" => fix_apply(&a(1)).await,
                    _ => fix_discard(&a(1)).await,
                }?;
                // Tell the server what happened so the report's history is visible on every device.
                // The deploy restarts the server, so retry until it is back (up to ~3 min).
                if let (Some(outcome), Ok(ctx)) = (LAST_OUTCOME.lock().unwrap().clone(), Ctx::new(false)) {
                    for _ in 0..36 {
                        let sent = ctx.post(&format!("/api/v1/reports/{}/fix-record", a(1))).json(&json!({ "action": action, "outcome": outcome })).send().await;
                        if sent.map(|r| r.status().is_success()).unwrap_or(false) {
                            break;
                        }
                        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    }
                }
                Ok(code)
            }
            "reports" => {
                let ctx = Ctx::new(false)?;
                match args.get(1) {
                    Some(id) => print_json(ctx.get(&format!("/api/v1/reports/{id}"))).await,
                    None => print_json(ctx.get("/api/v1/reports")).await,
                }
            }
            "commands" => print_json(Ctx::new(false)?.get("/api/v1/commands")).await,
            "mail-review-start" => print_json(Ctx::new(false)?.post("/api/v1/mail/review/start")).await,
            "mail-review-status" => print_json(Ctx::new(false)?.get("/api/v1/mail/review/status")).await,
            "mail-status" => print_json(Ctx::new(false)?.get("/api/v1/mail/status")).await,
            "needs" => print_json(Ctx::new(false)?.get("/api/v1/needs-you")).await,
            "needs-decide" if args.len() >= 4 => {
                let ctx = Ctx::new(false)?;
                let days: Option<i64> = args.get(5).and_then(|d| d.parse().ok());
                print_json(ctx.post("/api/v1/needs-you/decision").json(&json!({ "id": a(1), "title": a(2), "action": a(3), "days": days, "reason": args.get(4).cloned().unwrap_or_default() }))).await
            }
            "context" => print_json(Ctx::new(false)?.get(&format!("/api/v1/context?session_id={}", a(1)))).await,
            "lessons" => print_json(Ctx::new(false)?.get("/api/v1/lessons")).await,
            "lesson" if args.len() >= 3 => {
                let ctx = Ctx::new(false)?;
                print_json(ctx.post("/api/v1/lessons/update").json(&json!({ "id": a(1), "action": a(2), "text": args.get(3) }))).await
            }
            "troubleshoot" if args.len() >= 2 => troubleshoot(&a(1), &a(2), args.get(3).map(String::as_str)).await,
            "history" if args.len() >= 2 => print_json(Ctx::new(false)?.get("/api/v1/history").query(&[("session_id", a(1))])).await,
            "engines" => {
                let ctx = Ctx::new(false)?;
                let mut req = ctx.get("/api/v1/engines");
                if args.len() >= 2 {
                    req = req.query(&[("session_id", a(1))]);
                }
                print_json(req).await
            }
            "engine" if args.len() >= 2 => {
                let ctx = Ctx::new(false)?;
                let mut body = json!({ "engine": a(1) });
                if args.len() >= 3 {
                    body["session_id"] = json!(a(2));
                }
                print_json(ctx.post("/api/v1/engines").json(&body)).await
            }
            _ => {
                eprintln!("usage: loki-cli chat|watch|respond|always|sessions|history|engines|engine … (see src/bin/cli.rs)");
                Ok(2)
            }
        }
    }
    .await;
    match result {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            println!("{}", json!({ "type": "error", "message": e.to_string() }));
            std::process::exit(1);
        }
    }
}
