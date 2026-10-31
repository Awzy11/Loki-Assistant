//! Speech in and out through the GPU workstation Worker on the model server (the same service Odin's Echo and Skald
//! use): speech-to-text via `/jobs/transcribe-diarize` (whisper.cpp) and text-to-speech via
//! `/jobs/tts` (Kokoro). Both are async jobs — submit a file, poll the job, read the result.
//!
//! The worker is protected by a bearer token (Odin's `WORKER_SHARED_SECRET`). Loki reads it from
//! `$LOKI_SPEECH_TOKEN` or `~/.config/loki/speech_token` — never from config.toml or the repo.
//! Without it, speech reports itself as not configured instead of failing mysteriously.

use serde::Deserialize;
use serde_json::Value;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Deserialize)]
pub struct SpeechConfig {
    #[serde(default = "default_worker_url")]
    pub worker_url: String,
    /// Kokoro voice id (`af_heart`, `am_michael`, `bf_emma`, …); empty = the worker's default.
    #[serde(default)]
    pub tts_voice: String,
}

fn default_worker_url() -> String {
    std::env::var("LOKI_SPEECH_URL").unwrap_or_else(|_| "http://127.0.0.1:8100".to_string())
}

impl Default for SpeechConfig {
    fn default() -> Self {
        Self { worker_url: default_worker_url(), tts_voice: String::new() }
    }
}

pub fn token() -> Option<String> {
    if let Ok(t) = std::env::var("LOKI_SPEECH_TOKEN") {
        if !t.trim().is_empty() {
            return Some(t.trim().to_string());
        }
    }
    let p = dirs::config_dir()?.join("loki").join("speech_token");
    std::fs::read_to_string(p).ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
}

pub fn configured() -> bool {
    token().is_some()
}

fn client() -> reqwest::Client {
    reqwest::Client::builder().timeout(Duration::from_secs(60)).build().unwrap_or_default()
}

/// Submits `bytes` as a multipart `file` to `/jobs/<endpoint>` and waits for the job to finish.
async fn run_job(cfg: &SpeechConfig, endpoint: &str, filename: &str, content_type: &str, bytes: Vec<u8>, fields: &[(&str, String)], max_wait: Duration) -> Result<(String, Value), String> {
    let token = token().ok_or("speech isn't set up: the worker token is missing (see ~/.config/loki/speech_token)")?;
    let http = client();
    let part = reqwest::multipart::Part::bytes(bytes).file_name(filename.to_string()).mime_str(content_type).map_err(|e| e.to_string())?;
    let mut form = reqwest::multipart::Form::new().part("file", part);
    for (k, v) in fields {
        form = form.text(k.to_string(), v.clone());
    }
    let resp = http
        .post(format!("{}/jobs/{endpoint}", cfg.worker_url))
        .bearer_auth(&token)
        .multipart(form)
        .send()
        .await
        .map_err(|e| format!("couldn't reach the speech worker: {e}"))?;
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED || resp.status() == reqwest::StatusCode::FORBIDDEN {
        return Err("the speech worker rejected Loki's token".to_string());
    }
    if !resp.status().is_success() {
        return Err(format!("speech worker error {}: {}", resp.status(), resp.text().await.unwrap_or_default().chars().take(200).collect::<String>()));
    }
    let job_id = resp.json::<Value>().await.map_err(|e| e.to_string())?["job_id"].as_str().ok_or("the worker didn't return a job id")?.to_string();
    let deadline = Instant::now() + max_wait;
    while Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(600)).await;
        let Ok(r) = http.get(format!("{}/jobs/{job_id}", cfg.worker_url)).bearer_auth(&token).send().await else { continue };
        let Ok(job) = r.json::<Value>().await else { continue };
        match job["status"].as_str() {
            Some("done") => return Ok((job_id, job["result"].clone())),
            Some("failed") => return Err(job["error"].as_str().unwrap_or("the speech job failed").to_string()),
            _ => {}
        }
    }
    Err("the speech worker took too long".to_string())
}

/// Speech → text. Dictation, so speaker labels are dropped and segments are joined.
pub async fn transcribe(cfg: &SpeechConfig, audio: Vec<u8>, content_type: &str) -> Result<String, String> {
    let ext = if content_type.contains("mp4") || content_type.contains("m4a") {
        "m4a"
    } else if content_type.contains("ogg") {
        "ogg"
    } else if content_type.contains("wav") {
        "wav"
    } else {
        "webm"
    };
    let (_, result) = run_job(cfg, "transcribe-diarize", &format!("voice.{ext}"), "application/octet-stream", audio, &[("title", "Loki voice input".to_string())], Duration::from_secs(180)).await?;
    let text = result["segments"]
        .as_array()
        .map(|segs| segs.iter().filter_map(|s| s["text"].as_str()).map(str::trim).filter(|t| !t.is_empty()).collect::<Vec<_>>().join(" "))
        .or_else(|| result["text"].as_str().map(str::to_string))
        .unwrap_or_default();
    Ok(text.trim().to_string())
}

/// Text → speech. Returns the audio bytes and their content type.
pub async fn synthesize(cfg: &SpeechConfig, text: &str, voice: Option<&str>) -> Result<(Vec<u8>, String), String> {
    let speak = speakable(text);
    if speak.is_empty() {
        return Err("there's nothing to read aloud".to_string());
    }
    let voice = voice.filter(|v| !v.is_empty()).unwrap_or(&cfg.tts_voice).to_string();
    let (job_id, _) = run_job(cfg, "tts", "speech.txt", "text/plain", speak.into_bytes(), &[("title", "Loki reply".to_string()), ("voice", voice)], Duration::from_secs(240)).await?;
    let token = token().ok_or("speech isn't set up")?;
    let resp = client().get(format!("{}/jobs/{job_id}/audio", cfg.worker_url)).bearer_auth(&token).send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("couldn't fetch the audio ({})", resp.status()));
    }
    let ct = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("audio/mpeg").to_string();
    Ok((resp.bytes().await.map_err(|e| e.to_string())?.to_vec(), ct))
}

/// Strips markdown and clutter so the voice reads prose, not syntax; caps very long replies at a
/// sentence boundary.
pub fn speakable(md: &str) -> String {
    let mut out = String::new();
    let mut in_code = false;
    for line in md.lines() {
        let t = line.trim();
        if t.starts_with("```") {
            in_code = !in_code;
            if in_code {
                out.push_str("(code omitted.)\n");
            }
            continue;
        }
        if in_code || t.is_empty() || t.chars().all(|c| matches!(c, '-' | '|' | ':' | ' ' | '=')) {
            continue;
        }
        let mut l = t.trim_start_matches(['#', '>', '-', '*', ' ']).to_string();
        // [text](url) → text, and drop emphasis/inline-code marks
        while let (Some(a), Some(b)) = (l.find("]("), l.find('[')) {
            if b < a {
                if let Some(end) = l[a..].find(')') {
                    let text = l[b + 1..a].to_string();
                    l.replace_range(b..a + end + 1, &text);
                    continue;
                }
            }
            break;
        }
        let l: String = l.chars().filter(|c| !matches!(c, '*' | '_' | '`' | '|' | '~')).collect();
        out.push_str(l.trim());
        out.push('\n');
    }
    let out = out.trim().to_string();
    if out.chars().count() <= 3500 {
        return out;
    }
    let cut: String = out.chars().take(3500).collect();
    match cut.rfind(['.', '!', '?']) {
        Some(i) => format!("{} (The rest of the reply is on screen.)", &cut[..=i]),
        None => cut,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speakable_strips_markdown_and_code() {
        let md = "## Plan\n\n- **Read** the [note](https://x.test/a) first\n- then `append` it\n\n```rust\nlet x = 1;\n```\n\n| a | b |\n|---|---|\n\nDone.";
        let s = speakable(md);
        assert!(s.contains("Plan"));
        assert!(s.contains("Read the note first"));
        assert!(s.contains("then append it"));
        assert!(s.contains("(code omitted.)"));
        assert!(!s.contains("let x"));
        assert!(!s.contains("https://"));
        assert!(!s.contains("**"));
        assert!(s.ends_with("Done."));
    }

    #[test]
    fn long_replies_are_cut_at_a_sentence() {
        let long = "This is a sentence. ".repeat(400);
        let s = speakable(&long);
        assert!(s.chars().count() < 3600);
        assert!(s.contains("rest of the reply is on screen"));
    }
}
