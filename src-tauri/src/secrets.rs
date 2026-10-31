use base64::Engine;
use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305, Nonce,
};
use rand::RngCore;
use std::fs;
use std::path::PathBuf;

/// Encrypts/decrypts secrets at rest — BYOK API keys today (Phase 1), per-connection PKMS
/// credentials from Phase 5 — using a machine-local key generated on first use and stored
/// with `0600` permissions at `~/.config/loki/secret.key`, never committed, never round-tripped
/// to any frontend. This is separate from `config.toml`, which stays plaintext (as it already
/// was for the Obsidian API key) for the hand-edit bootstrap path — encryption only applies to
/// values saved through the Settings UI/API, stored in `~/.config/loki/llm.secrets.toml`.
fn key_path() -> anyhow::Result<PathBuf> {
    let dir = crate::paths::config_dir().ok_or_else(|| anyhow::anyhow!("no config dir found for this OS"))?.join("loki");
    fs::create_dir_all(&dir)?;
    Ok(dir.join("secret.key"))
}

fn load_or_create_key() -> anyhow::Result<ChaCha20Poly1305> {
    let path = key_path()?;
    let key_bytes = if path.exists() {
        let raw = fs::read(&path)?;
        if raw.len() != 32 {
            anyhow::bail!("secret.key at {} is the wrong length (expected 32 bytes, got {})", path.display(), raw.len());
        }
        raw
    } else {
        let mut key = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut key);
        crate::paths::write_private(&path, key)?;
        key.to_vec()
    };
    ChaCha20Poly1305::new_from_slice(&key_bytes).map_err(|e| anyhow::anyhow!("bad key material: {e}"))
}

/// Encrypts `plaintext`, returning a base64 string safe to write to disk or a TOML file:
/// `base64(nonce || ciphertext)`, a fresh random nonce per call.
pub fn encrypt(plaintext: &str) -> anyhow::Result<String> {
    let cipher = load_or_create_key()?;
    let mut nonce_bytes = [0u8; 12];
    rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext =
        cipher.encrypt(nonce, plaintext.as_bytes()).map_err(|e| anyhow::anyhow!("encryption failed: {e}"))?;
    let mut combined = nonce_bytes.to_vec();
    combined.extend_from_slice(&ciphertext);
    Ok(base64::engine::general_purpose::STANDARD.encode(combined))
}

/// Reverses `encrypt`.
pub fn decrypt(encoded: &str) -> anyhow::Result<String> {
    let cipher = load_or_create_key()?;
    let combined = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|e| anyhow::anyhow!("invalid ciphertext encoding: {e}"))?;
    if combined.len() < 12 {
        anyhow::bail!("ciphertext too short to contain a nonce");
    }
    let (nonce_bytes, ciphertext) = combined.split_at(12);
    let nonce = Nonce::from_slice(nonce_bytes);
    let plaintext = cipher.decrypt(nonce, ciphertext).map_err(|e| anyhow::anyhow!("decryption failed: {e}"))?;
    Ok(String::from_utf8(plaintext)?)
}
