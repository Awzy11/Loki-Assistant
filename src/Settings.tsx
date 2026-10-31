import { useEffect, useState } from "react";
import McpSettings from "./McpSettings";
import {
  checkForUpdate,
  getVersion,
  type UpdateCheck,
  getClaudeConfig,
  getLlmConfig,
  getStoredApiToken,
  listPermissionRules,
  removePermissionRule,
  listHostGrants,
  listWorkspaces,
  trustFolder,
  untrustFolder,
  type TrustedFolder,
  grantHost,
  revokeHost,
  type HostGrants,
  saveClaudeConfig,
  saveLlmConfig,
  setStoredApiToken,
  type ClaudeConfig,
  type PublicBackend,
} from "./transport";

type Vendor = "ollama" | "openai_compatible" | "anthropic" | "gemini";

const VENDOR_FIELDS: Record<Vendor, string[]> = {
  ollama: ["host", "port", "model"],
  openai_compatible: ["base_url", "model"],
  anthropic: ["model"],
  gemini: ["model"],
};

const VENDOR_LABELS: Record<Vendor, string> = {
  ollama: "Ollama (local)",
  openai_compatible: "OpenAI-compatible",
  anthropic: "Anthropic",
  gemini: "Gemini",
};

function BackendForm({ section, label }: { section: "llm" | "embedding"; label: string }) {
  const [vendor, setVendor] = useState<Vendor>("ollama");
  const [fields, setFields] = useState<Record<string, string>>({});
  const [apiKey, setApiKey] = useState("");
  const [hasApiKey, setHasApiKey] = useState(false);
  const [status, setStatus] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    getLlmConfig(section)
      .then((cfg: PublicBackend | null) => {
        if (cfg) {
          const v = (cfg.vendor as Vendor) || "ollama";
          setVendor(v);
          setHasApiKey(cfg.has_api_key);
          const f: Record<string, string> = {};
          for (const key of VENDOR_FIELDS[v] ?? []) {
            if (key in cfg) f[key] = String(cfg[key]);
          }
          setFields(f);
        }
      })
      .catch(() => {})
      .finally(() => setLoading(false));
  }, [section]);

  function updateField(key: string, value: string) {
    setFields((f) => ({ ...f, [key]: value }));
  }

  async function save() {
    setStatus(null);
    const payload: Record<string, unknown> = { vendor };
    for (const key of VENDOR_FIELDS[vendor]) {
      const raw = fields[key] ?? "";
      payload[key] = key === "port" ? Number(raw) : raw;
    }
    if (apiKey.trim()) payload.api_key = apiKey.trim();

    try {
      await saveLlmConfig(section, payload);
      setApiKey("");
      if (payload.api_key) setHasApiKey(true);
      setStatus("Saved — restart Loki for this to take effect.");
    } catch (e) {
      setStatus(`Error: ${String(e)}`);
    }
  }

  if (loading) return <div className="loki-settings-section">Loading {label}…</div>;

  return (
    <div className="loki-settings-section">
      <h2>{label}</h2>
      <label className="loki-settings-field">
        Vendor
        <select value={vendor} onChange={(e) => setVendor(e.currentTarget.value as Vendor)}>
          {(Object.keys(VENDOR_LABELS) as Vendor[]).map((v) => (
            <option key={v} value={v}>
              {VENDOR_LABELS[v]}
            </option>
          ))}
        </select>
      </label>
      {VENDOR_FIELDS[vendor].map((key) => (
        <label className="loki-settings-field" key={key}>
          {key}
          <input value={fields[key] ?? ""} onChange={(e) => updateField(key, e.currentTarget.value)} placeholder={key} />
        </label>
      ))}
      {vendor !== "ollama" && (
        <label className="loki-settings-field">
          API key {hasApiKey && <span className="loki-settings-hint">(already set — leave blank to keep it)</span>}
          <input
            type="password"
            value={apiKey}
            onChange={(e) => setApiKey(e.currentTarget.value)}
            placeholder={hasApiKey ? "••••••••" : "sk-..."}
          />
        </label>
      )}
      <button type="button" onClick={save}>
        Save {label}
      </button>
      {status && <div className="loki-settings-status">{status}</div>}
    </div>
  );
}

/**
 * `loki-server` guards `/api/config/*` and `/api/v1/*` behind a bearer token (see `auth.rs`),
 * and neither a browser nor the desktop app's webview (a thin shell pointed at `loki-server`
 * as of 2026-08-30 — see `transport.ts`'s module doc) has a way to read that token off disk
 * itself. Paste it once from `~/.config/loki/api_token` on the machine running `loki-server`;
 * stored in `localStorage`, never sent anywhere except as the Authorization header on Loki's
 * own API calls. Every client needs this now — there's no more IPC-bypasses-auth path.
 */
function ApiTokenSettings() {
  const [token, setToken] = useState(() => getStoredApiToken());
  const [status, setStatus] = useState<string | null>(null);

  function save() {
    setStoredApiToken(token);
    setStatus(token.trim() ? "Saved." : "Cleared.");
  }

  return (
    <div className="loki-settings-section">
      <h2>Loki API token</h2>
      <p className="loki-settings-note">
        Required for chatting with Loki and for saving model config from this web deployment. Find it at{" "}
        <code>~/.config/loki/api_token</code> on the machine running <code>loki-server</code>.
      </p>
      <label className="loki-settings-field">
        Token
        <input
          type="password"
          value={token}
          onChange={(e) => setToken(e.currentTarget.value)}
          placeholder="paste token here"
        />
      </label>
      <button type="button" onClick={save}>
        Save token
      </button>
      {status && <div className="loki-settings-status">{status}</div>}
    </div>
  );
}

/**
 * `[claude]` — the CLI subprocess Routine Mode relays to. Not vendor/api_key-shaped like
 * `[llm]`/`[embedding]` (see `BackendForm` above), so it's a separate small form rather than
 * another `BackendForm` instance. Not selectable as a chat/embedding backend — this is
 * Routine Mode's own subprocess config, unrelated to which model answers ordinary chat.
 */
function ClaudeConfigForm() {
  const [config, setConfig] = useState<ClaudeConfig>({ binary_path: "claude", timeout_secs: 120, routine_working_dir: "" });
  const [status, setStatus] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    getClaudeConfig()
      .then((cfg) => setConfig({ ...cfg, routine_working_dir: cfg.routine_working_dir ?? "" }))
      .catch(() => {})
      .finally(() => setLoading(false));
  }, []);

  async function save() {
    setStatus(null);
    try {
      await saveClaudeConfig({
        ...config,
        timeout_secs: Number(config.timeout_secs) || 120,
        routine_working_dir: config.routine_working_dir?.trim() ? config.routine_working_dir.trim() : null,
      });
      setStatus("Saved — restart Loki for this to take effect.");
    } catch (e) {
      setStatus(`Error: ${String(e)}`);
    }
  }

  if (loading) return <div className="loki-settings-section">Loading Claude subprocess…</div>;

  return (
    <div className="loki-settings-section">
      <h2>Claude subprocess (Routine Mode)</h2>
      <label className="loki-settings-field">
        Binary path
        <input
          value={config.binary_path}
          onChange={(e) => setConfig((c) => ({ ...c, binary_path: e.currentTarget.value }))}
          placeholder="claude"
        />
      </label>
      <label className="loki-settings-field">
        Routine working directory
        <input
          value={config.routine_working_dir ?? ""}
          onChange={(e) => setConfig((c) => ({ ...c, routine_working_dir: e.currentTarget.value }))}
          placeholder="/home/you/notes"
        />
      </label>
      <label className="loki-settings-field">
        Timeout (seconds)
        <input
          type="number"
          value={config.timeout_secs}
          onChange={(e) => setConfig((c) => ({ ...c, timeout_secs: Number(e.currentTarget.value) }))}
        />
      </label>
      <label className="loki-settings-field">
        Approval bridge binary <span className="loki-settings-hint">(loki-bridge-mcp path; enables approval prompts + device tools for Claude turns)</span>
        <input
          value={config.bridge_binary ?? ""}
          onChange={(e) => setConfig((c) => ({ ...c, bridge_binary: e.currentTarget.value || null }))}
          placeholder="/path/to/Loki/src-tauri/target/release/loki-bridge-mcp"
        />
      </label>
      <label className="loki-settings-field">
        Extra tools Claude may use without asking <span className="loki-settings-hint">(one per line, e.g. mcp__zotero__*)</span>
        <textarea
          rows={3}
          value={(config.extra_allowed_tools ?? []).join("\n")}
          onChange={(e) =>
            setConfig((c) => ({ ...c, extra_allowed_tools: e.currentTarget.value.split("\n").map((l) => l.trim()).filter(Boolean) }))
          }
        />
      </label>
      <label className="loki-settings-field">
        <span>
          <input
            type="checkbox"
            checked={config.server_tools_with_approval ?? false}
            onChange={(e) => setConfig((c) => ({ ...c, server_tools_with_approval: e.currentTarget.checked }))}
          />{" "}
          Let Claude use Bash / file tools on this server <span className="loki-settings-hint">(every call asks for approval)</span>
        </span>
      </label>
      <button type="button" onClick={save}>
        Save Claude subprocess config
      </button>
      {status && <div className="loki-settings-status">{status}</div>}
    </div>
  );
}

function PermissionRulesSettings() {
  const [rules, setRules] = useState<string[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const refresh = () => listPermissionRules().then(setRules).catch((e) => setError(String(e)));
  useEffect(() => {
    refresh();
  }, []);
  return (
    <details className="loki-settings-section loki-collapsible">
      <summary>
        <h2>Always-allowed actions{rules ? ` (${rules.length})` : ""}</h2>
      </summary>
      <p className="loki-settings-note">
        Saved on the server, so they apply on every device. Each is as narrow as the action you approved
        (the exact command, the file, the website). Deletes and other outward-facing actions always ask.
      </p>
      {error && <div className="loki-settings-status">{error}</div>}
      {rules && rules.length === 0 && <div className="loki-settings-status">None saved.</div>}
      {rules?.map((rule) => (
        <div key={rule} className="loki-rule-row">
          <code>{rule}</code>
          <button type="button" onClick={() => removePermissionRule(rule).then(refresh).catch((e) => setError(String(e)))}>
            Revoke
          </button>
        </div>
      ))}
    </details>
  );
}

function TrustedHostsSettings() {
  const [data, setData] = useState<HostGrants | null>(null);
  const [error, setError] = useState<string | null>(null);
  const refresh = () => listHostGrants().then(setData).catch((e) => setError(String(e)));
  useEffect(() => {
    refresh();
  }, []);
  return (
    <div className="loki-settings-section">
      <h2>Trusted remote hosts</h2>
      <p className="loki-settings-note">
        Trust a host for a while and Loki stops asking before non-destructive commands there. Read-only commands
        already run without asking. Deletes, sudo, redirects, command substitution and anything touching keys or
        tokens still ask. Held in server memory only: it expires on its own and is cleared when Loki restarts.
      </p>
      {error && <div className="loki-settings-status">{error}</div>}
      {data?.configured.length === 0 && <div className="loki-settings-status">No remote hosts configured.</div>}
      {data?.configured.map((host) => {
        const grant = data.hosts.find((g) => g.host === host);
        return (
          <div key={host} className="loki-rule-row">
            <code>{host}</code>
            {grant ? (
              <>
                <span>trusted, {Math.ceil(grant.seconds_left / 60)} min left</span>
                <button type="button" onClick={() => revokeHost(host).then(refresh).catch((e) => setError(String(e)))}>
                  Revoke
                </button>
              </>
            ) : (
              <button type="button" onClick={() => grantHost(host, 120).then(refresh).catch((e) => setError(String(e)))}>
                Trust for 2 hours
              </button>
            )}
          </div>
        );
      })}
    </div>
  );
}

import NotificationSettings, { InAppAlertSettings } from "./Notifications";

function TrustedFoldersSettings() {
  const [data, setData] = useState<{ workspaces: TrustedFolder[]; hosts: string[] } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [host, setHost] = useState("device");
  const [path, setPath] = useState("");
  const [how, setHow] = useState("120");
  const refresh = () => listWorkspaces().then(setData).catch((e) => setError(String(e)));
  useEffect(() => {
    refresh();
  }, []);
  function add() {
    setError(null);
    trustFolder(host, path, how === "always" ? { permanent: true } : { minutes: Number(how) })
      .then(() => {
        setPath("");
        return refresh();
      })
      .catch((e) => setError(String(e)));
  }
  return (
    <div className="loki-settings-section">
      <h2>Trusted folders</h2>
      <p className="loki-settings-note">
        Trust a project folder and Loki stops asking before file edits and non-destructive commands inside it —
        on this computer ("device", the desktop app) or on any configured remote host. Deletes, redirects,
        anything touching keys or tokens, and anything outside the folder still ask. Whole home folders and system
        folders can't be trusted. Also available in chat: <code>/trust</code>, <code>/untrust</code>, <code>/trusted</code>.
      </p>
      {error && <div className="loki-settings-status">{error}</div>}
      {data?.workspaces.length === 0 && <div className="loki-settings-status">No trusted folders.</div>}
      {data?.workspaces.map((w) => (
        <div key={`${w.host}:${w.path}`} className="loki-rule-row">
          <code>
            {w.host}: {w.path}
          </code>
          <span>{w.seconds_left === null ? "always" : `${Math.ceil(w.seconds_left / 60)} min left`}</span>
          <button type="button" onClick={() => untrustFolder(w.host, w.path).then(refresh).catch((e) => setError(String(e)))}>
            Remove
          </button>
        </div>
      ))}
      <div className="loki-rule-row">
        <select value={host} onChange={(e) => setHost(e.target.value)}>
          {(data?.hosts ?? ["device"]).map((h) => (
            <option key={h} value={h}>
              {h}
            </option>
          ))}
        </select>
        <input value={path} onChange={(e) => setPath(e.target.value)} placeholder="/home/you/projects/thing or ~/projects/thing" style={{ flex: 1, minWidth: "12rem" }} />
        <select value={how} onChange={(e) => setHow(e.target.value)}>
          <option value="60">1 hour</option>
          <option value="120">2 hours</option>
          <option value="480">8 hours</option>
          <option value="always">Always</option>
        </select>
        <button type="button" disabled={!path.trim()} onClick={add}>
          Trust folder
        </button>
      </div>
    </div>
  );
}

/** Shows the running version; the update check contacts the public repository only when pressed. */
function VersionSettings() {
  const [v, setV] = useState<{ version: string; install: string } | null>(null);
  const [res, setRes] = useState<UpdateCheck | null>(null);
  const [err, setErr] = useState("");
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    getVersion().then(setV).catch(() => {});
  }, []);
  async function check() {
    setBusy(true);
    setErr("");
    try {
      setRes(await checkForUpdate());
    } catch (e) {
      setErr(String(e instanceof Error ? e.message : e));
    }
    setBusy(false);
  }
  return (
    <div className="loki-settings-section">
      <h2>Version</h2>
      <p className="loki-settings-note">
        Loki {v ? v.version : "…"}{v ? ` (${v.install})` : ""}. Checking for updates contacts GitHub once, when you press the button, and sends nothing about you.
      </p>
      <button type="button" onClick={() => void check()} disabled={busy}>Check for updates</button>
      {res && (
        <div className="loki-settings-status">
          {res.newer ? `${res.latest} is available. ` : ""}{res.message}{res.newer && res.url ? <> <a href={res.url} target="_blank" rel="noreferrer">Release notes</a></> : null}
        </div>
      )}
      {err && <div className="loki-settings-status">{err}</div>}
    </div>
  );
}

export default function Settings() {
  return (
    <div className="loki-settings">
      <p className="loki-settings-note">
        Most changes take effect on Loki's next restart (the tools section below applies right away). "Claude subprocess" below
        is optional: it lets Loki use your own Claude Code subscription, and it stays off unless you add a [claude] section.
        Your chat and embedding models are set in the sections below.
      </p>
      <VersionSettings />
      <ApiTokenSettings />
      <NotificationSettings />
      <InAppAlertSettings />
      <BackendForm section="llm" label="Chat model" />
      <BackendForm section="embedding" label="Embedding model" />
      <McpSettings />
      <ClaudeConfigForm />
      <PermissionRulesSettings />
      <TrustedHostsSettings />
      <TrustedFoldersSettings />
    </div>
  );
}
