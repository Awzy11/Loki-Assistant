import { useCallback, useEffect, useState } from "react";
import { mcpAction, mcpServers, mcpTools, type McpServerInfo } from "./transport";

const SOURCE: Record<string, string> = { claude: "From Claude's list", config: "From config.toml", added: "Added here" };

/** Settings section for Loki's MCP servers: see them, switch them on and off, reconnect, add and remove, live. */
export default function McpSettings() {
  const [servers, setServers] = useState<McpServerInfo[] | null>(null);
  const [error, setError] = useState("");
  const [note, setNote] = useState("");
  const [open, setOpen] = useState<string | null>(null);
  const [tools, setTools] = useState<{ name: string; description: string }[]>([]);
  const [busy, setBusy] = useState("");
  const [kind, setKind] = useState<"command" | "url">("command");
  const [form, setForm] = useState({ name: "", command: "", args: "", env: "", url: "" });

  const load = useCallback(async () => {
    try {
      setServers(await mcpServers());
      setError("");
    } catch (e) {
      setError(String(e));
    }
  }, []);
  useEffect(() => void load(), [load]);
  // keep refreshing while anything is still connecting
  const connecting = servers?.some((s) => s.enabled && !s.connected && s.error === "connecting");
  useEffect(() => {
    if (!connecting) return;
    const t = setInterval(() => void load(), 2000);
    return () => clearInterval(t);
  }, [connecting, load]);

  async function act(body: Parameters<typeof mcpAction>[0], confirmText?: string) {
    if (confirmText && !window.confirm(confirmText)) return false;
    setBusy(body.name + body.action);
    setError("");
    try {
      setNote(await mcpAction(body));
      await load();
      return true;
    } catch (e) {
      setError(String(e));
      return false;
    } finally {
      setBusy("");
    }
  }
  async function toggleTools(name: string) {
    if (open === name) return setOpen(null);
    setOpen(name);
    setTools([]);
    try {
      setTools(await mcpTools(name));
    } catch (e) {
      setError(String(e));
    }
  }
  async function add() {
    const body = kind === "command"
      ? { action: "add", name: form.name.trim(), command: form.command.trim(), args: form.args.split("\n").map((a) => a.trim()).filter(Boolean), env: Object.fromEntries(form.env.split("\n").map((l) => l.trim()).filter((l) => l.includes("=")).map((l) => [l.slice(0, l.indexOf("=")).trim(), l.slice(l.indexOf("=") + 1)])) }
      : { action: "add", name: form.name.trim(), url: form.url.trim() };
    const warn = kind === "command"
      ? `This will run "${form.command.trim()} ${form.args.split("\n").map((a) => a.trim()).filter(Boolean).join(" ")}" on the Loki server, with that server's permissions, every time Loki starts and whenever it reconnects. Only continue if you trust this program. Add it?`
      : `Loki will connect to ${form.url.trim()} and make its tools available to the assistant. Add it?`;
    if (await act(body, warn)) setForm({ name: "", command: "", args: "", env: "", url: "" });
  }

  return (
    <section className="loki-settings-section">
      <h2>Tools (MCP servers){servers ? ` (${servers.filter((s) => s.enabled).length} on of ${servers.length})` : ""}</h2>
      <p className="loki-settings-hint">
        These are the tool servers Loki's assistants can use. Changes apply right away, no restart. Servers from config.toml or Claude's list can be switched off here but not deleted. Claude's delegated runs pick up changes after Loki restarts.
        In chat, type <code>/mcp</code> to see them, or ask me to change one and approve the prompt.
      </p>
      {error && <div className="loki-error">{error}</div>}
      {note && <p className="mx-notice">{note}</p>}
      {!servers && !error && <p className="loki-settings-hint">Loading…</p>}
      <div className="mcp-list">
        {servers?.map((s) => (
          <div key={s.name} className={`mcp-row ${s.enabled ? "" : "off"}`}>
            <div className="mcp-main">
              <span className={`loki-dot loki-dot-${!s.enabled ? "off" : s.connected ? "ok" : s.error === "connecting" ? "warn" : "fail"}`} aria-hidden="true" />
              <b>{s.name}</b>
              <span className="mx-chip">{SOURCE[s.source]}</span>
              <span className="mx-chip">{s.transport === "command" ? "Command" : "URL"}</span>
              <span className="mcp-state">{!s.enabled ? "Off" : s.connected ? `Connected, ${s.tools} tool${s.tools === 1 ? "" : "s"}` : s.error === "connecting" ? "Connecting…" : `Not connected: ${s.error ?? "unknown"}`}</span>
            </div>
            <code className="mcp-target">{s.target}{s.env_keys.length ? `  [env: ${s.env_keys.join(", ")}]` : ""}</code>
            <div className="mcp-acts">
              {s.enabled && <button type="button" disabled={!!busy} onClick={() => void act({ action: "reconnect", name: s.name })}>Reconnect</button>}
              {s.enabled
                ? <button type="button" disabled={!!busy} onClick={() => void act({ action: "disable", name: s.name })}>Turn off</button>
                : <button type="button" disabled={!!busy} onClick={() => void act({ action: "enable", name: s.name })}>Turn on</button>}
              {s.connected && <button type="button" onClick={() => void toggleTools(s.name)}>{open === s.name ? "Hide tools" : "Show tools"}</button>}
              {s.removable && <button type="button" disabled={!!busy} onClick={() => void act({ action: "remove", name: s.name }, `Remove "${s.name}"? Its settings (including any stored environment values) are deleted.`)}>Remove</button>}
            </div>
            {open === s.name && (
              <div className="mcp-tools">{tools.length === 0 ? "Loading…" : tools.map((t) => <div key={t.name}><b>{t.name}</b><span>{t.description}</span></div>)}</div>
            )}
          </div>
        ))}
      </div>

      <h3 className="mcp-add-h">Add a server</h3>
      <div className="mcp-form">
        <label>Name <input value={form.name} placeholder="my-server" onChange={(e) => setForm({ ...form, name: e.target.value })} /></label>
        <div className="ex-kinds" role="group" aria-label="Type">
          <button type="button" className={kind === "command" ? "on" : ""} onClick={() => setKind("command")}>Runs a command</button>
          <button type="button" className={kind === "url" ? "on" : ""} onClick={() => setKind("url")}>Connects to a URL</button>
        </div>
        {kind === "command" ? (
          <>
            <label>Command <input value={form.command} placeholder="npx" onChange={(e) => setForm({ ...form, command: e.target.value })} /></label>
            <label>Arguments <small>(one per line)</small><textarea rows={3} value={form.args} placeholder={"-y\nsome-mcp-package"} onChange={(e) => setForm({ ...form, args: e.target.value })} /></label>
            <label>Environment <small>(KEY=value per line; stored on the server, never shown again)</small><textarea rows={2} value={form.env} placeholder="API_KEY=…" onChange={(e) => setForm({ ...form, env: e.target.value })} /></label>
            <p className="loki-settings-hint">A command server runs a program on the Loki server. You will be asked to confirm.</p>
          </>
        ) : (
          <label>URL <input value={form.url} placeholder="http://127.0.0.1:8096/mcp" onChange={(e) => setForm({ ...form, url: e.target.value })} /></label>
        )}
        <button type="button" className="loki-chipbtn" disabled={!!busy || !form.name.trim() || (kind === "command" ? !form.command.trim() : !form.url.trim())} onClick={() => void add()}>Add server</button>
      </div>
    </section>
  );
}
