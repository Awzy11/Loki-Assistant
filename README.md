# Loki

A personal AI agent you run yourself. It works with your Obsidian vault, your tools and the models you choose, and everything stays on your own machines: no accounts, no telemetry, nothing hosted for you.

Loki is early software (v0.1). It is in daily use by its author, but expect rough edges, and read [Security](SECURITY.md) before you put it on a network.

## What it does

- **Chat with your notes.** An agent that can list, read, search and append to your Obsidian vault, answer everyday questions, and ask before it does anything it cannot undo (deleting, running commands, writing outside the vault).
- **Local first.** It talks to [Ollama](https://ollama.com) by default. Cloud models and a Claude Code subscription are optional extra engines, and anything marked private (mail, credentials, money) stays on your local model.
- **A daily dashboard.** A "Needs you" page and briefing cards built from your own notes: today's plan, the week, countdowns, backlog, rhythm of your daily properties, weather, and more.
- **Routines as plain prompts.** Morning and evening routines, weekly and monthly reviews, journaling, inbox and backlog triage ship as editable Markdown prompts that read your folder names from a small `profile.toml`.
- **Your tools.** It uses any [MCP](https://modelcontextprotocol.io) server you connect, with a permission model: tools are allowed, ask-first or blocked, and you can approve once or always.
- **One server, many screens.** A web app and an installable phone app (PWA) served by the same small Rust server, plus a desktop app (Tauri) and a command-line client. A conversation started on one screen continues on another.
- **Local mail triage (optional).** Read-only Gmail access, sorted by your local model, with replies saved as drafts you send yourself.

## What you need

- **[Ollama](https://ollama.com)** with a chat model (for example `ollama pull llama3.1:8b`).
- **[Obsidian](https://obsidian.md)** with the *Local REST API* community plugin enabled (Loki reads and writes the vault through it).
- **Rust** (`rustup`) and **Node 20 or newer** to build. On Linux the desktop app also needs Tauri's usual system libraries; the web app does not.

## Install

Pick one. Each ends the same way: open <http://localhost:8420> (the source build below uses 8420 too) and finish setup in the browser. Platform notes are in [docs/INSTALL.md](docs/INSTALL.md).

- **Docker** (Linux, macOS, Windows): `docker compose up -d` using the [compose.yaml](compose.yaml) in this repository. Nothing to compile.
- **Download** (Linux x86_64 and Raspberry Pi, macOS Apple silicon and Intel): unpack the archive from the [Releases](https://github.com/Awzy11/Loki-Assistant/releases) page and run `./loki.sh`.
- **From source**: below.

Something not working? See [docs/TROUBLESHOOTING.md](docs/TROUBLESHOOTING.md).

## Build from source

```bash
git clone https://github.com/Awzy11/Loki-Assistant.git
cd Loki-Assistant
npm install && npm run build
cd src-tauri && cargo build --release --bin loki-server && cd ..
./src-tauri/target/release/loki-server
```

Open <http://127.0.0.1:8420> in a browser on that machine. The first time, Loki walks you through setup right there in the browser:

1. **Models.** A local model (Ollama), a cloud provider (Anthropic or any OpenAI-compatible service), or both. Each can be tested on the spot. If you give only a cloud provider, Loki tells you plainly that everything you ask will be sent to it and asks you to confirm.
2. **Notes.** Your Obsidian vault, through the Local REST API plugin.
3. **Layout.** A blank vault (the default) or the ready-made starter pack with its routines.
4. **About you.** Optional: your name, a line about you and your location.
5. **Login.** The username and password you will use on each device.

When you press *Finish*, Loki saves your settings, switches to normal mode and signs you in. Until setup is finished the server only answers this machine; to set up a server you reach over the network, open an SSH tunnel to it (`ssh -L 8420:127.0.0.1:8420 yourserver`) and use <http://127.0.0.1:8420>.

By default the server listens on `127.0.0.1` only. To reach it from your phone, put it on a private network such as [Tailscale](https://tailscale.com) and set `LOKI_BIND` to that address (or `0.0.0.0` on a trusted network). Do not expose it to the open internet.

## Your vault, your layout

A blank vault is the default: plain folder names (`Inbox`, `Daily`, `Weeks`, `Backlog`, `Notes`...), no routines, nothing assumed about how you work. If you want a ready-made system, pick the starter pack during setup (or later with `ops/install-pack.sh scrum-zettelkasten`).

It adds a `profile.toml` for a numbered-folder vault (inbox → backlog → weekly sprint → daily note, with permanent notes and an archive) and eleven routines you can edit freely. Preview any routine with your folder names filled in:

```bash
./src-tauri/target/release/loki-routine bod
```

Everything in the profile (folders, note headings, daily properties, who you are, where you are, which routines show up where) is documented in [docs/PROFILE.md](docs/PROFILE.md).

## Privacy, by design

- No accounts, telemetry or phone-home. Loki only contacts the services you configure.
- Mail and money are handled by local models only, and Loki's own tools never print their contents into notes.
- Secrets live in `~/.config/loki/` (mode 700), never in the vault or the repository.
- Every action that changes something outside the vault asks first, and you choose how long an approval lasts.

## Documentation

| | |
| --- | --- |
| [docs/INSTALL.md](docs/INSTALL.md) | Install on Linux, Raspberry Pi, macOS and Windows |
| [docs/TROUBLESHOOTING.md](docs/TROUBLESHOOTING.md) | Ollama or Obsidian not reachable, ports, passwords |
| [docs/PROFILE.md](docs/PROFILE.md) | The profile: folders, headings, properties, routines, placeholders |
| [docs/PROTOCOL.md](docs/PROTOCOL.md) | How the web, desktop, phone and CLI screens talk to the server |
| [docs/MAIL.md](docs/MAIL.md) | Optional local Gmail triage |
| [config.example.toml](config.example.toml) | Every setting for the server |
| [ROADMAP.md](ROADMAP.md) | What is planned |

## Contributing and license

See [CONTRIBUTING.md](CONTRIBUTING.md). Loki is released under the [Apache License 2.0](LICENSE); third-party notices are in [NOTICE](NOTICE).
