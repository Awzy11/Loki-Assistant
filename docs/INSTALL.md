# Installing Loki

Loki is one small server plus a web app. You need two things running on a machine you control: [Ollama](https://ollama.com) with a chat model (or a cloud provider's API key), and [Obsidian](https://obsidian.md) with the *Local REST API* community plugin enabled. Then install Loki one of these ways and finish setup in the browser.

## Docker (recommended on every platform)

1. Install Docker (Docker Desktop on macOS and Windows; Docker Engine on Linux).
2. Save [compose.yaml](../compose.yaml) in a folder and run `docker compose up -d` there.
3. Open <http://localhost:8420> and follow the setup. Use the address `host.docker.internal` for Ollama and Obsidian; the setup pre-fills it. That name means "the machine Docker runs on".
4. Make Ollama and Obsidian reachable from the container:
   - **Ollama** listens on `127.0.0.1` only by default, which a container cannot reach. Start it with `OLLAMA_HOST=0.0.0.0` (on Linux with systemd: `sudo systemctl edit ollama`, add `Environment="OLLAMA_HOST=0.0.0.0"` under `[Service]`, restart). Docker Desktop on macOS and Windows reaches it without this change only if Ollama's app setting "Expose Ollama to the network" is on.
   - **Obsidian's Local REST API** plugin has a "Bind address" setting; use `0.0.0.0` (or the Docker bridge address) and keep the API key set. Prefer the plugin's non-encrypted port 27123 for a first test.
   - A host firewall such as `ufw` may block the container. Allow the Docker bridge to those two ports.
5. Your login and settings live in the `loki-data` volume. To update: `docker compose pull && docker compose up -d`.

The compose file publishes the port on `127.0.0.1` only. Leave it that way until setup is done.

## Linux and Raspberry Pi (download)

1. Download the archive for your machine from [Releases](https://github.com/Awzy11/Loki-Assistant/releases): `linux-x86_64` for ordinary PCs, `linux-aarch64` for a Raspberry Pi 4/5/500 with a 64-bit OS. Check it against `SHA256SUMS`.
2. `tar xzf loki-*.tar.gz && cd loki-*` and run `./loki.sh`.
3. Open <http://127.0.0.1:8420>.

To keep it running, use a systemd user service that runs `loki.sh`.

## macOS (download)

Builds are for Apple silicon (`macos-arm64`) and Intel (`macos-x86_64`). They are not signed with an Apple developer certificate, so macOS will refuse to open them the first time. Either right-click `loki-server` in Finder and choose *Open*, or run `xattr -dr com.apple.quarantine .` inside the unpacked folder. Then `./loki.sh` and open <http://127.0.0.1:8420>. Config lives in `~/.config/loki`.

## Windows

Use Docker Desktop with the steps above, or WSL 2:

1. In PowerShell: `wsl --install -d Ubuntu`, then open Ubuntu.
2. Follow *Linux* above inside Ubuntu. Open <http://localhost:8420> in your Windows browser (WSL forwards it).
3. Ollama and Obsidian run on Windows, not inside WSL. From WSL, reach them at the Windows host's address (`ip route show default` prints it), and enable "Expose Ollama to the network" and the Obsidian plugin's bind address as above. With WSL's *mirrored networking* mode (`networkingMode=mirrored` in `.wslconfig`) plain `127.0.0.1` works.

## From source

See the README. You need Rust and Node 20 or newer.

## Updating

Settings has a *Check for updates* button. It contacts GitHub once, when pressed. Then: Docker, `docker compose pull && docker compose up -d`; download, unpack the new archive over the old folder; source, `git pull` and rebuild. Your settings are outside these folders and are kept.
