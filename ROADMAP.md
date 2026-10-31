# Roadmap

Loki is self-hosted only: no accounts, no telemetry, nothing phones home. This list is what is planned, roughly in order. It is a direction, not a promise.

## Next releases

- **Home-server cards (planned for a later release).** Finance (a self-hosted ledger such as Firefly III), Home network (containers, disk, backups, firewall, Tailscale devices), Recently saved and the news globe's data are built and in daily use on the author's own server, but they read it over SSH with hand-written scripts, so they are not in the first public build. The plan is to make each card a small plugin that declares a host, a read-only command and how to show the result, all from `profile.toml`, so anyone can point it at their own machine.
- **Setup from Settings.** First-run setup happens in the browser today; adding or changing a model, a cloud provider or the vault layout later still means editing `config.toml` or `profile.toml`. Next: the same forms inside Settings.
- **No Obsidian required.** A plain-folder vault backend, so Loki works on any folder of Markdown files without the Local REST API plugin.
- **More starter packs** for people who do not use the scrum + Zettelkasten layout.
- **Generic prompts for the remaining routines** (monthly and quarterly reviews, energy check, journal, low-capacity mode, inbox triage).
- **An API contract that is tested.** Every endpoint listed in one route table and in [docs/PROTOCOL.md](docs/PROTOCOL.md), with a test that fails when the two drift, plus sample responses that the web app's tests decode, so the server and every screen agree by test.
- **Screen parity check.** A test that fails when a page exists on one surface (web, desktop, phone) without its counterpart or a written reason, so "the same features everywhere" stays true.
- **A recommendations ledger.** Suggestions Loki makes are recorded with their evidence and cost, and you accept or decline each one; later Loki shows whether the ones you accepted worked out.
- **Backup and restore.** One command to back up your settings and Loki's saved data (optionally encrypted), a documented restore, and a check that proves the restore works.
- **Feedback you approve.** A "send feedback" action that drafts a GitHub issue with names, paths and keys scrubbed, shows you exactly what it will send, and sends nothing until you say yes. Still no telemetry.
- **Optional hosted mode** is not planned. If real demand shows up it would be a separate decision.
