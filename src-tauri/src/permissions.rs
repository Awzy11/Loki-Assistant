/// Mirrors the four-tier AI Permission Model defined in the vault's CLAUDE.md. Loki only
/// touches the Obsidian vault today, so only three tiers are actually assigned to a tool
/// below — this module exists now, ahead of need, so that when Firefly III, Gmail, raw
/// biometrics, or any other Tier-3-shaped tool gets wired into Loki, tagging it `LocalOnly`
/// is the only change required to keep its output out of `escalate_to_claude` (or, from
/// Phase 1 on, out of any cloud-backed `LlmProvider`).
///
/// As of the Phase 0 tool-registry refactor, the tool→tier mapping itself lives on each
/// `Tool` impl (`agent::tools::Tool::tier`), not in a standalone match statement here — that
/// was a real risk of drift between two copies of the same table. `ToolRegistry::tier_of`
/// is the single source of truth at runtime; it preserves the same fail-closed default this
/// module previously enforced directly (an unrecognized/unregistered tool name is treated as
/// `LocalOnly`, never `Ambient`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// Tier 0 — pure reads, no side effects, non-sensitive.
    Ambient,
    /// Tier 1 — low-consequence or easily reversible writes.
    FreeWrite,
    /// Tier 2 — external effects or hard-to-reverse actions. `agent::run_turn` asks the user
    /// (via `bridge::Bridge::request_permission`) before any tool at this tier executes;
    /// no client, a timeout or a "no" means it never runs.
    ConfirmRequired,
    /// Tier 3 — sensitive data that must never reach a cloud model. Enforced in
    /// `agent::run_turn`: `escalate_to_claude` refuses outright if any tool tagged
    /// `LocalOnly` was called earlier in the same turn.
    LocalOnly,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::tools::ToolRegistry;

    /// The load-bearing property this whole module exists to guarantee: a tool name the
    /// registry doesn't recognize must default to `LocalOnly`, never `Ambient` — a forgotten
    /// or misspelled tool registration fails closed (blocks escalation) instead of silently
    /// leaking whatever it returns to a cloud model.
    #[test]
    fn unregistered_tool_defaults_to_local_only() {
        let registry = ToolRegistry::default_registry();
        assert_eq!(registry.tier_of("some_tool_nobody_registered"), Tier::LocalOnly);
    }

    #[test]
    fn known_tools_keep_their_expected_tiers() {
        let registry = ToolRegistry::default_registry();
        assert_eq!(registry.tier_of("list_files_in_vault"), Tier::Ambient);
        assert_eq!(registry.tier_of("list_files_in_dir"), Tier::Ambient);
        assert_eq!(registry.tier_of("read_file"), Tier::Ambient);
        assert_eq!(registry.tier_of("simple_search"), Tier::Ambient);
        assert_eq!(registry.tier_of("semantic_search_vault"), Tier::Ambient);
        assert_eq!(registry.tier_of("append_content"), Tier::FreeWrite);
        assert_eq!(registry.tier_of("delete_file"), Tier::ConfirmRequired);
        assert_eq!(registry.tier_of("device_read_file"), Tier::Ambient);
        assert_eq!(registry.tier_of("device_write_file"), Tier::ConfirmRequired);
        assert_eq!(registry.tier_of("device_run_command"), Tier::ConfirmRequired);
    }
}
