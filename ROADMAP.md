# Roadmap

Loki is self-hosted only: no accounts, no telemetry, nothing phones home. This list is what is planned, roughly in order. It is a direction, not a promise.

## Next releases

- **Home-server cards (planned for a later release).** Finance (a self-hosted ledger such as Firefly III), Home network (containers, disk, backups, firewall, Tailscale devices), Recently saved and the news globe's data are built and in daily use on the author's own server, but they read it over SSH with hand-written scripts, so they are not in the first public build. The plan is to make each card a small plugin that declares a host, a read-only command and how to show the result, all from `profile.toml`, so anyone can point it at their own machine.
- **Setup from Settings.** First-run setup happens in the browser today; adding or changing a model, a cloud provider or the vault layout later still means editing `config.toml` or `profile.toml`. Next: the same forms inside Settings.
- **No Obsidian required.** A plain-folder vault backend, so Loki works on any folder of Markdown files without the Local REST API plugin.
- **More starter packs** for people who do not use the scrum + Zettelkasten layout.
- **Generic prompts for the remaining routines** (monthly and quarterly reviews, energy check, journal, low-capacity mode, inbox triage).
- **Optional hosted mode** is not planned. If real demand shows up it would be a separate decision.
