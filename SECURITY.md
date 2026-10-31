# Security

Loki runs on your own machine and your own server. It has no accounts, no telemetry and no hosted component, so there is nothing of yours on anyone else's computer.

## Reporting a problem

Please report a vulnerability privately through GitHub's "Report a vulnerability" button on the Security tab of this repository, not in a public issue. Include what you saw, how to reproduce it and the version. I will answer within a week.

## What to keep in mind when you run it

- The server serves your notes and can run tools. Keep it on a private network (a tailnet or LAN) and behind its login; do not expose it to the internet.
- Mail and money are handled by local models only. If you add a cloud engine, check `docs/` for which tools it can reach before you connect your accounts.
- API keys and tokens live in `~/.config/loki/`, never in the vault or the repository. Keep that folder private (mode 700).
