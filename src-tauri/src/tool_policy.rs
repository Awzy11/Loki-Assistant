//! Which tools run without asking. One policy for everything that executes tools — Loki's own
//! agent loop and the delegated Claude Code path — so the allow-list lives in Loki, not in any
//! model vendor's settings.
//!
//! `TIER01_ALLOWED_TOOLS` (below, mirrored from the vault CLAUDE.md AI Permission Model) plus
//! `[claude] extra_allowed_tools` from config.toml are auto-allowed. Anything else — including
//! every MCP tool Loki has never been told about — is `ConfirmRequired`: the user is asked, and
//! "always" rules (`permission_rules`) can then make a specific call permanent.

use crate::permissions::Tier;

/// The exact set of tools a headless Routine Mode session is allowed to call, scoped to
/// Tier 0 (ambient read) and Tier 1 (free write) per the vault's CLAUDE.md AI Permission
/// Model — never Tier 2/3 tools like `obsidian_delete_file`. Enumerated explicitly rather
/// than wildcarded (`mcp__obsidian__*` would also grant delete) so a Tier change in
/// CLAUDE.md requires a deliberate, visible edit here too, not silent drift. `ToolSearch`
/// is included because the weather tool is deferred and BOD/EOD-style routines load it via
/// `ToolSearch` before first use (see `~/.claude/commands/bod.md`).
///
/// This list may need extending as more routines get exercised through Routine Mode — if a
/// routine needs a tool not on this list, it fails closed (blocked) rather than silently
/// reaching for something outside Tier 0/1. That's the intended behavior, not a bug to
/// work around by widening the list reflexively.
pub const TIER01_ALLOWED_TOOLS: &[&str] = &[
    "ToolSearch",
    // --- Obsidian — Tier 0 reads + Tier 1 writes (never delete_file, Tier 2) ---
    "mcp__obsidian__obsidian_list_files_in_vault",
    "mcp__obsidian__obsidian_list_files_in_dir",
    "mcp__obsidian__obsidian_get_file_contents",
    "mcp__obsidian__obsidian_batch_get_file_contents",
    "mcp__obsidian__obsidian_simple_search",
    "mcp__obsidian__obsidian_complex_search",
    "mcp__obsidian__obsidian_get_periodic_note",
    "mcp__obsidian__obsidian_get_recent_periodic_notes",
    "mcp__obsidian__obsidian_get_recent_changes",
    // Added since the original CLAUDE.md Tier 0 table was written — both pure reads.
    "mcp__obsidian__obsidian_get_frontmatter",
    "mcp__obsidian__obsidian_search_by_tag",
    "mcp__obsidian__obsidian_append_content",
    "mcp__obsidian__obsidian_patch_content",
    // Tier 1 full-note-rewrite tool — only reachable at all once the `uvx mcp-obsidian`
    // server (not the old Docker `mcp/obsidian` image) is registered; see CLAUDE.md's
    // Obsidian Vault Access section.
    "mcp__obsidian__obsidian_put_content",
    "mcp__openweather__weather",
    // Loki's own owned knowledge store (sqlite-vec embeddings), exposed to this shelled-out
    // Claude Code process via the `loki-mcp` stdio server (registered `-s user` on this
    // machine's Claude CLI, 2026-09-12) — semantic RAG over the vault *and* past conversation
    // logs (98 - Loki Conversations/), which obsidian_simple_search/complex_search can't do
    // (keyword/JsonLogic only, no embeddings). Read-only (`loki_query` bypasses the chat
    // loop, no side effects) — deliberately not `loki_chat` (would let this session recurse
    // into Loki's own chat loop) or `loki_ingest` (a write capability not needed here).
    "mcp__loki-mcp__loki_query",
    // --- Widened 2026-09-14: the rest of CLAUDE.md's Tier 0/1 table, previously left off
    // this allowlist entirely (Routine Mode had only ever gotten Obsidian + weather +
    // loki_query). Every tool name below was confirmed live and reachable on the Loki server's
    // registered MCP servers before being added — names sometimes differ from what
    // CLAUDE.md's own tables say (e.g. Miniflux/GitHub/Mealie's tables predate tool
    // renames/additions on the actual running servers); the *names here* are ground truth,
    // not the prose tables. Tier 2 tools (create/delete/publish/merge/etc.) are still never
    // included — see each domain's Tier 2 row in CLAUDE.md for what's deliberately excluded.
    //
    // Miniflux — Tier 0 reads + Tier 1 (mark_entries_read, trivial even though irreversible).
    "mcp__miniflux__get_unread_entries",
    "mcp__miniflux__get_feeds",
    "mcp__miniflux__get_categories",
    "mcp__miniflux__get_entries_with_content",
    "mcp__miniflux__mark_entries_read",
    // Mealie — Tier 0 reads + Tier 1 writes (low-stakes, easily undone).
    "mcp__mealie__get_recipes",
    "mcp__mealie__get_recipe_concise",
    "mcp__mealie__get_recipe_detailed",
    "mcp__mealie__get_shopping_lists",
    "mcp__mealie__get_shopping_list",
    "mcp__mealie__get_shopping_list_items",
    "mcp__mealie__get_all_mealplans",
    "mcp__mealie__get_todays_mealplan",
    "mcp__mealie__get_tags",
    "mcp__mealie__get_categories",
    "mcp__mealie__create_shopping_list_item",
    "mcp__mealie__update_shopping_list_item",
    "mcp__mealie__create_mealplan",
    "mcp__mealie__update_recipe",
    // n8n — Tier 0 reads + Tier 1 (activate/deactivate *existing* flows only — never
    // create/update/delete_workflow, Tier 2).
    "mcp__n8n__list_workflows",
    "mcp__n8n__get_workflow",
    "mcp__n8n__list_executions",
    "mcp__n8n__get_execution",
    "mcp__n8n__activate_workflow",
    "mcp__n8n__deactivate_workflow",
    // Gumroad — Tier 0 reads + Tier 1 (update_product; CLAUDE.md scopes this to draft-state
    // edits only, which this allowlist can't itself enforce — never create_product, Tier 2).
    "mcp__gumroad__list_products",
    "mcp__gumroad__get_product",
    "mcp__gumroad__list_sales",
    "mcp__gumroad__update_product",
    // GitHub — Tier 0 reads only. Every write tool (create/update/delete/merge/push/comment/
    // review/assign) is Tier 2 and deliberately excluded.
    "mcp__github__get_file_contents",
    "mcp__github__list_issues",
    "mcp__github__list_pull_requests",
    "mcp__github__list_branches",
    "mcp__github__list_commits",
    "mcp__github__search_code",
    "mcp__github__search_issues",
    "mcp__github__search_pull_requests",
    "mcp__github__search_repositories",
    "mcp__github__search_users",
    "mcp__github__get_commit",
    "mcp__github__get_me",
    "mcp__github__list_releases",
    "mcp__github__get_release_by_tag",
    "mcp__github__get_label",
    "mcp__github__get_latest_release",
    "mcp__github__get_tag",
    "mcp__github__get_team_members",
    "mcp__github__get_teams",
    "mcp__github__issue_read",
    "mcp__github__list_issue_types",
    "mcp__github__list_tags",
    "mcp__github__pull_request_read",
    // Spotify — Tier 0 reads + Tier 1 (ambient playback control, no lasting effect). Playlist
    // mutation (addTracksToPlaylist, createPlaylist, saveOrRemoveAlbumForUser) is untiered in
    // CLAUDE.md, so deliberately left off rather than assumed Tier 1.
    "mcp__spotify__getNowPlaying",
    "mcp__spotify__searchSpotify",
    "mcp__spotify__getMyPlaylists",
    "mcp__spotify__getQueue",
    "mcp__spotify__getRecentlyPlayed",
    "mcp__spotify__getUsersSavedTracks",
    "mcp__spotify__getAvailableDevices",
    "mcp__spotify__checkUsersSavedAlbums",
    "mcp__spotify__getAlbums",
    "mcp__spotify__getAlbumTracks",
    "mcp__spotify__getPlaylistTracks",
    "mcp__spotify__playMusic",
    "mcp__spotify__pausePlayback",
    "mcp__spotify__resumePlayback",
    "mcp__spotify__skipToNext",
    "mcp__spotify__skipToPrevious",
    "mcp__spotify__setVolume",
    "mcp__spotify__adjustVolume",
    "mcp__spotify__addToQueue",
    // Samsung Health — Tier 0, all tools (data quality unreliable per CLAUDE.md — read
    // freely, trust cautiously).
    "mcp__samsung-health__get_health_summary",
    "mcp__samsung-health__get_recovery_trend",
    "mcp__samsung-health__get_today_health",
    "mcp__samsung-health__get_weekly_average",
    // Gutenberg, Airbnb, YouTube Transcript, Fetch — Tier 0, "all tools".
    "mcp__gutenberg__search_books",
    "mcp__gutenberg__search_books_by_subject",
    "mcp__gutenberg__get_book_info",
    "mcp__gutenberg__get_book_text",
    "mcp__airbnb__airbnb_search",
    "mcp__airbnb__airbnb_listing_details",
    "mcp__youtube-transcript__get_transcript",
    "mcp__youtube-transcript__get_timed_transcript",
    "mcp__youtube-transcript__get_video_info",
    "mcp__fetch__fetch",
    // Odin (Phase A / College country studies) and OSINT (subsea cables, IXPs, ASNs, satellites,
    // outages) — keyless, read-only lookups.
    "mcp__odin__*",
    "mcp__osint__*",
];

/// Named like Claude Code's MCP tools (`mcp__<server>__<tool>`) so the same names work in
/// allow-lists, always-allow rules and the UI whichever backend made the call.
#[derive(Debug, Clone, Default)]
pub struct ToolPolicy {
    allowed: Vec<String>,
}

impl ToolPolicy {
    pub fn new(extra_allowed: &[String]) -> Self {
        let mut allowed: Vec<String> = TIER01_ALLOWED_TOOLS.iter().map(|t| t.to_string()).collect();
        allowed.extend(extra_allowed.iter().cloned());
        Self { allowed }
    }

    /// Exact match, or a trailing `*` wildcard (`mcp__osint__*`).
    pub fn is_allowed(&self, name: &str) -> bool {
        self.allowed.iter().any(|pat| match pat.strip_suffix('*') {
            Some(prefix) => name.starts_with(prefix),
            None => pat == name,
        })
    }

    /// Tier for an MCP tool: allow-listed reads are `Ambient`, allow-listed writes `FreeWrite`
    /// (their arguments still go through the content classifier), everything else asks.
    pub fn tier(&self, name: &str) -> Tier {
        if !self.is_allowed(name) {
            return Tier::ConfirmRequired;
        }
        if looks_read_only(name) { Tier::Ambient } else { Tier::FreeWrite }
    }
}

/// MCP servers whose tools only look things up, whatever the tool is called.
const READ_ONLY_SERVERS: &[&str] = &[
    "mcp__osint__",
    "mcp__odin__",
    "mcp__fetch__",
    "mcp__openweather__",
    "mcp__gutenberg__",
    "mcp__youtube-transcript__",
    "mcp__airbnb__",
    "mcp__samsung-health__",
];

fn looks_read_only(full_name: &str) -> bool {
    if READ_ONLY_SERVERS.iter().any(|p| full_name.starts_with(p)) {
        return true;
    }
    let short = full_name.rsplit("__").next().unwrap_or(full_name).to_lowercase();
    ["get", "list", "search", "read", "find", "query", "fetch", "check", "weather", "transcript", "info", "summary"]
        .iter()
        .any(|w| short.contains(w))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlisted_reads_and_writes_get_their_tiers() {
        let p = ToolPolicy::new(&[]);
        assert_eq!(p.tier("mcp__obsidian__obsidian_get_file_contents"), Tier::Ambient);
        assert_eq!(p.tier("mcp__obsidian__obsidian_append_content"), Tier::FreeWrite);
        assert_eq!(p.tier("mcp__osint__colo_facilities"), Tier::Ambient); // wildcard + read-looking
    }

    #[test]
    fn everything_else_asks() {
        let p = ToolPolicy::new(&[]);
        assert_eq!(p.tier("mcp__obsidian__obsidian_delete_file"), Tier::ConfirmRequired);
        assert_eq!(p.tier("mcp__github__create_pull_request"), Tier::ConfirmRequired);
        assert_eq!(p.tier("mcp__never_heard_of__thing"), Tier::ConfirmRequired);
    }

    #[test]
    fn config_extras_extend_the_allowlist() {
        let p = ToolPolicy::new(&["mcp__zotero__*".to_string()]);
        assert!(p.is_allowed("mcp__zotero__zotero_search_items"));
        assert!(!ToolPolicy::new(&[]).is_allowed("mcp__zotero__zotero_search_items"));
    }
}
