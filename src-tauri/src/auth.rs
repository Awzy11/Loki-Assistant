//! Bearer-token auth for the routes that need it (Phase 2.5) — `/api/v1/*` and
//! `/api/config/*`. A single shared token, not per-caller: fine for "the user's own
//! integrations reaching Loki over the LAN/Tailscale," not meant to scale into a
//! multi-tenant auth story. Legacy `/api/message`/`/api/history` stay unauthenticated —
//! the existing desktop/web chat UI has no token plumbing and this phase doesn't add any
//! to it, deliberately keeping that surface's risk where it already was rather than
//! expanding scope into a frontend auth flow.
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use axum::{
    extract::Request,
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::Response,
};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

fn token_path() -> anyhow::Result<PathBuf> {
    Ok(crate::config::config_dir()?.join("api_token"))
}

/// Separate from `token_path()` deliberately — the webhook (voice-note capture from the
/// Pebble Index 01, via a phone app) is a token handed to a *device*, not just pasted into
/// Settings once by the user. A lost/compromised phone should only cost revoking this
/// one file, not the token every other integration (Omarchy, Settings, curl testing) relies
/// on. See `require_webhook_token`.
fn webhook_token_path() -> anyhow::Result<PathBuf> {
    Ok(crate::config::config_dir()?.join("webhook_token"))
}

fn load_or_create_token_at(path: PathBuf) -> anyhow::Result<String> {
    if let Ok(existing) = fs::read_to_string(&path) {
        let trimmed = existing.trim().to_string();
        if !trimmed.is_empty() {
            return Ok(trimmed);
        }
    }
    let mut bytes = [0u8; 32];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut bytes);
    let token = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    crate::paths::write_private(&path, &token)?;
    Ok(token)
}

/// Reads the token, generating and persisting a fresh one on first run.
pub fn load_or_create_token() -> anyhow::Result<String> {
    load_or_create_token_at(token_path()?)
}

/// Same mechanics as `load_or_create_token`, separate file — see `webhook_token_path`.
pub fn load_or_create_webhook_token() -> anyhow::Result<String> {
    load_or_create_token_at(webhook_token_path()?)
}

fn check_bearer(headers: &HeaderMap, expected: &str) -> bool {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|got| constant_time_eq(got.as_bytes(), expected.as_bytes()))
        .unwrap_or(false)
}

/// Compares two secrets without stopping at the first difference, so response timing doesn't reveal how much of a guess was right.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let mut diff = a.len() ^ b.len();
    for i in 0..a.len().max(b.len()) {
        diff |= (a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0)) as usize;
    }
    std::hint::black_box(diff) == 0
}

/// axum middleware — reject with 401 unless `Authorization: Bearer <token>` matches the
/// token from `load_or_create_token()`. Reads the token file on every request rather than
/// caching it in memory, so rotating the file (delete + restart, or a future admin action)
/// takes effect without needing a process restart to notice — a deliberate simplicity
/// tradeoff for a single-shared-token model, not something to keep if this grows into
/// per-caller tokens later.
pub async fn require_bearer_token(headers: HeaderMap, request: Request, next: Next) -> Result<Response, StatusCode> {
    let expected = load_or_create_token().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if check_bearer(&headers, &expected) {
        Ok(next.run(request).await)
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

/// Same as `require_bearer_token`, checked against the separate webhook token instead — see
/// `webhook_token_path`'s doc comment for why this is a distinct file/middleware rather than
/// reusing the main one.
pub async fn require_webhook_token(headers: HeaderMap, request: Request, next: Next) -> Result<Response, StatusCode> {
    let expected = load_or_create_webhook_token().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if check_bearer(&headers, &expected) {
        Ok(next.run(request).await)
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

// --- Login (2026-09-01) — a friendlier front door onto the same bearer token ---
//
// This does NOT replace `require_bearer_token`/the shared `api_token` file above — every
// authenticated request still carries that same bearer token, unchanged. What this adds is a
// nicer way to *obtain* it: instead of SSHing into whichever machine runs `loki-server` and
// catting `~/.config/loki/api_token` by hand, a device logs in once with a username/password
// and gets the same token handed back, to store in `localStorage` exactly as before. There's
// still only one real credential that matters for API access (the token) — login is a
// UX improvement on first-time setup, not a new session/authorization model.

#[derive(Debug, Serialize, Deserialize)]
struct Credentials {
    username: String,
    /// Argon2 PHC string (includes the salt and parameters) — never the plaintext password.
    password_hash: String,
}

fn credentials_path() -> anyhow::Result<PathBuf> {
    Ok(crate::config::config_dir()?.join("credentials.toml"))
}

fn hash_password(password: &str) -> anyhow::Result<String> {
    let salt = SaltString::generate(&mut rand::rngs::OsRng);
    Argon2::default().hash_password(password.as_bytes(), &salt).map(|h| h.to_string()).map_err(|e| anyhow::anyhow!("{e}"))
}

/// First run only: generates a random username="loki" / password pair, hashes the password,
/// and persists it to `credentials.toml` (0600, same permission pattern as `api_token`).
/// Returns the plaintext password — the ONLY time it's ever available in plaintext anywhere,
/// including in this process's own memory beyond this call. Printed once to `loki-server`'s
/// own startup log (see `bin/server.rs`) so it can be read off the machine and handed to
/// the user, then never persisted or logged again.
fn create_credentials() -> anyhow::Result<(Credentials, String)> {
    let mut bytes = [0u8; 9];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut bytes);
    // Base32-ish (Crockford, no padding) rather than hex — meant to actually be typed into a
    // login form once per device, not just pasted, so it skips ambiguous characters (0/O, 1/I/L).
    const ALPHABET: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    let password: String = bytes.iter().map(|b| ALPHABET[(*b as usize) % ALPHABET.len()] as char).collect();
    let password_hash = hash_password(&password)?;
    let creds = Credentials { username: "loki".to_string(), password_hash };
    let path = credentials_path()?;
    crate::paths::write_private(&path, toml::to_string(&creds)?)?;
    Ok((creds, password))
}

/// Setup mode: stores the login the person chose (username and an Argon2 hash of the password) in `credentials.toml` (0600).
pub fn set_credentials(username: &str, password: &str) -> anyhow::Result<()> {
    let creds = Credentials { username: username.to_string(), password_hash: hash_password(password)? };
    let path = credentials_path()?;
    crate::paths::write_private(&path, toml::to_string(&creds)?)?;
    Ok(())
}

/// Called once at `loki-server` startup (see `bin/server.rs`). If `credentials.toml` doesn't
/// exist yet, generates it and returns the plaintext password to print — `None` on every
/// subsequent startup, once credentials already exist.
pub fn ensure_credentials_exist() -> anyhow::Result<Option<String>> {
    let path = credentials_path()?;
    if path.exists() {
        return Ok(None);
    }
    let (_, password) = create_credentials()?;
    Ok(Some(password))
}

fn load_credentials() -> anyhow::Result<Credentials> {
    let raw = fs::read_to_string(credentials_path()?)?;
    Ok(toml::from_str(&raw)?)
}

/// Verifies a username/password pair against the stored credentials and, on success, returns
/// the same shared API token every other authenticated caller uses (`load_or_create_token`).
/// Deliberately doesn't distinguish "bad username" from "bad password" in its `Err` — both
/// collapse to the login endpoint's generic 401, so a failed attempt can't be used to probe
/// which usernames exist (moot with exactly one user today, but costs nothing to do right).
pub fn login(username: &str, password: &str) -> anyhow::Result<String> {
    let creds = load_credentials()?;
    let hash = PasswordHash::new(&creds.password_hash).map_err(|e| anyhow::anyhow!("{e}"))?;
    let ok = creds.username == username && Argon2::default().verify_password(password.as_bytes(), &hash).is_ok();
    if !ok {
        anyhow::bail!("invalid username or password");
    }
    load_or_create_token()
}

#[cfg(test)]
mod tests {
    #[test]
    fn constant_time_eq_matches_equality() {
        assert!(super::constant_time_eq(b"abc123", b"abc123"));
        assert!(!super::constant_time_eq(b"abc123", b"abc124"));
        assert!(!super::constant_time_eq(b"abc", b"abc123"));
        assert!(!super::constant_time_eq(b"abc123", b"abc"));
        assert!(!super::constant_time_eq(b"", b"x"));
        assert!(super::constant_time_eq(b"", b""));
    }
}
