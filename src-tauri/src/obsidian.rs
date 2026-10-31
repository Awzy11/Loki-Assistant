use crate::config::ObsidianConfig;
use serde::Deserialize;

pub struct ObsidianClient {
    http: reqwest::Client,
    config: ObsidianConfig,
}

#[derive(Debug, Deserialize)]
struct VaultListing {
    files: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct SearchResult {
    filename: String,
    #[serde(default)]
    score: f64,
}

/// Percent-encodes each path segment (splitting on `/`, which stays a literal separator —
/// never itself encoded). Found necessary the hard way: importing a real vault copy, 3 of
/// 615 notes failed with filenames containing `;` (e.g. "Ideology Demands Full Attainment;
/// Religion Expects Striving.md") — a semicolon in an unencoded URL path broke request
/// construction *before the request was even sent* (not a 404 from the server, a client-side
/// parse failure), confirmed by reproducing it with a raw `curl` call. This isn't a mock-server
/// quirk — it would hit the real Obsidian REST API identically, since it's how the URL gets
/// built client-side, before any server is involved.
fn encode_path(path: &str) -> String {
    path.split('/').map(urlencoding::encode).collect::<Vec<_>>().join("/")
}

impl ObsidianClient {
    pub fn new(config: ObsidianConfig) -> Self {
        Self { http: reqwest::Client::new(), config }
    }

    fn auth(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        builder.bearer_auth(&self.config.api_key)
    }

    /// Lists the vault root, or a specific directory if `path` is Some (no leading/trailing slash needed).
    pub async fn list_files(&self, path: Option<&str>) -> anyhow::Result<Vec<String>> {
        let url = match path {
            Some(p) => format!("{}/vault/{}/", self.config.base_url(), encode_path(p.trim_matches('/'))),
            None => format!("{}/vault/", self.config.base_url()),
        };
        let resp = self.auth(self.http.get(&url)).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("list_files({path:?}) failed: {}", resp.status());
        }
        let listing: VaultListing = resp.json().await?;
        Ok(listing.files)
    }

    /// Reads a single file's raw content.
    pub async fn read_file(&self, path: &str) -> anyhow::Result<String> {
        let url = format!("{}/vault/{}", self.config.base_url(), encode_path(path.trim_start_matches('/')));
        let resp = self.auth(self.http.get(&url)).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("read_file({path}) failed: {}", resp.status());
        }
        Ok(resp.text().await?)
    }

    /// Appends content to a file, creating it if it doesn't exist.
    pub async fn append_content(&self, path: &str, content: &str) -> anyhow::Result<()> {
        let url = format!("{}/vault/{}", self.config.base_url(), encode_path(path.trim_start_matches('/')));
        let resp = self
            .auth(self.http.post(&url))
            .header("Content-Type", "text/markdown")
            .body(content.to_string())
            .send()
            .await?;
        if !resp.status().is_success() {
            anyhow::bail!("append_content({path}) failed: {}", resp.status());
        }
        Ok(())
    }

    /// Replaces a file's whole content (creating it if needed).
    pub async fn put_content(&self, path: &str, content: &str) -> anyhow::Result<()> {
        let url = format!("{}/vault/{}", self.config.base_url(), encode_path(path.trim_start_matches('/')));
        let resp = self.auth(self.http.put(&url)).header("Content-Type", "text/markdown").body(content.to_string()).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("put_content({path}) failed: {}", resp.status());
        }
        Ok(())
    }

    /// Deletes a file outright. No confirmation here — the agent layer must gate this
    /// behind explicit user confirmation before ever calling it (Tier 2, per the vault's
    /// own AI Permission Model — mirror that policy in Loki, don't relax it).
    pub async fn delete_file(&self, path: &str) -> anyhow::Result<()> {
        let url = format!("{}/vault/{}", self.config.base_url(), encode_path(path.trim_start_matches('/')));
        let resp = self.auth(self.http.delete(&url)).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("delete_file({path}) failed: {}", resp.status());
        }
        Ok(())
    }

    /// Simple full-text search across the vault.
    pub async fn simple_search(&self, query: &str, context_length: u32) -> anyhow::Result<Vec<(String, f64)>> {
        let url = format!(
            "{}/search/simple/?query={}&contextLength={}",
            self.config.base_url(),
            urlencoding::encode(query),
            context_length
        );
        let resp = self.auth(self.http.post(&url)).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("simple_search({query}) failed: {}", resp.status());
        }
        let results: Vec<SearchResult> = resp.json().await?;
        Ok(results.into_iter().map(|r| (r.filename, r.score)).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_path_preserves_slashes_but_encodes_special_chars() {
        assert_eq!(
            encode_path("50 - Notes/Ideology Demands Full Attainment; Religion Expects Striving.md"),
            "50%20-%20Notes/Ideology%20Demands%20Full%20Attainment%3B%20Religion%20Expects%20Striving.md"
        );
    }

    #[test]
    fn encode_path_handles_plain_paths_unchanged_in_meaning() {
        assert_eq!(encode_path("10 - Daily/2026-08-24.md"), "10%20-%20Daily/2026-08-24.md");
    }
}
