# Troubleshooting

**Setup says it "couldn't reach Ollama".** Check that Ollama is running (`curl http://localhost:11434/api/tags` on its machine). If Loki runs in Docker, Ollama must listen beyond loopback (`OLLAMA_HOST=0.0.0.0`) and the address in setup must be `host.docker.internal`. If Ollama is on another machine, use that machine's address and make sure its firewall allows port 11434.

**Setup says it "couldn't reach Obsidian".** Obsidian must be open with the *Local REST API* plugin enabled. Copy the API key from the plugin's settings. Use port 27123 (plain HTTP). From Docker, the plugin's bind address must allow connections from the container, not only `127.0.0.1`.

**A container can't reach the host.** On Linux a firewall such as `ufw` often drops traffic from the Docker bridge to host ports. Allow it, for example `sudo ufw allow from 172.17.0.0/16 to any port 11434 proto tcp`. Compose already maps `host.docker.internal` to the host.

**"Loki is not set up yet. Finish setup from the machine it runs on."** Setup only answers connections from the same machine, because it can set your password. Open it from that machine, or through an SSH tunnel (`ssh -L 8420:127.0.0.1:8420 server` then <http://localhost:8420>). The Docker image allows it because the container sees the browser as a remote address; compose keeps the published port on `127.0.0.1`.

**Port 8420 is in use.** Set `LOKI_PORT` (source and download) or change the left side of `ports` in `compose.yaml`.

**Reaching Loki from my phone.** Put the server on a private network such as Tailscale, and set `LOKI_BIND` to that address. Read [SECURITY.md](../SECURITY.md) before exposing it further.

**macOS says the app can't be opened.** The builds are unsigned. Right-click, *Open*, or `xattr -dr com.apple.quarantine <folder>`.

**I forgot my password.** Delete `~/.config/loki/credentials.toml` (in Docker: `docker compose exec loki rm /data/.config/loki/credentials.toml`) and restart; Loki prints a new one.

**Start over.** Remove `~/.config/loki/config.toml` and restart to run setup again. In Docker, `docker compose down -v` also removes the saved data.
