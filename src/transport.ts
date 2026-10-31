/** `via` is only ever set on a turn added during this page session (fresh from `sendMessage`)
 * — history restored via `getHistory` doesn't carry it, since it isn't persisted anywhere
 * (the vault conversation log has no per-turn backend field). `undefined` means "unknown /
 * not tracked for this turn," not "was local" — don't treat it as a local indicator. */
export type Turn = {
  role: "user" | "assistant";
  content: string;
  via?: string;
  /** Why a fallback model answered, e.g. "Claude Code hit its usage limit — answered by gemini." */
  note?: string;
  /** Streamed text + tool calls for a turn produced this page session (not restored history). */
  blocks?: Block[];
  /** The user pressed Stop before the reply finished. */
  stopped?: boolean;
  /** Local time sent/received, `YYYY-MM-DD HH:MM:SS TZ` (see `formatStamp`). */
  at?: string;
  /** Thumbs rating on an assistant message. */
  feedback?: "up" | "down";
  /** Latest Claude audit of an assistant message. */
  audit?: Audit;
};

export type AuditFinding = { kind: string; detail: string; severity: string };
export type Audit = {
  verdict: "good" | "minor" | "bad";
  summary: string;
  findings: AuditFinding[];
  better_answer?: string | null;
  suggested_lessons: string[];
  auto?: boolean;
  at: number;
};
export type Lesson = {
  id: string;
  text: string;
  status: "pending" | "approved" | "rejected";
  created: number;
  source?: string | null;
  kind?: "lesson" | "tool_hint" | "routine_edit";
  tool?: string | null;
  target?: string | null;
  append?: string | null;
  baseline?: number | null;
  effect?: string | null;
};

export type ToolBlock = {
  kind: "tool";
  id: string;
  name: string;
  input: unknown;
  result?: string;
  isError?: boolean;
};
export type TextBlock = { kind: "text"; text: string };
/** An approval the turn is waiting on (or that was answered). */
export type PermissionBlock = {
  kind: "permission";
  id: string;
  tool: string;
  input: unknown;
  /** `resolved`: answered on another device. */
  status: "pending" | "allowed" | "denied" | "resolved";
  /** Answered automatically from an "always allow" rule. */
  auto?: boolean;
};
export type Block = TextBlock | ToolBlock | PermissionBlock;

/** Mirrors `ChatEvent` in src-tauri/src/events.rs (serde tag = "type"). */
export type StreamEvent =
  | { type: "hello"; session_id: string; attached: number }
  | { type: "user_message"; text: string; client_id: string }
  | { type: "resolved"; id: string }
  | { type: "started"; session_id: string }
  | { type: "text_delta"; text: string }
  | { type: "reset" }
  | { type: "tool_start"; id: string; name: string; input: unknown }
  | { type: "tool_result"; id: string; content: string; is_error: boolean }
  | { type: "status"; message: string }
  | { type: "permission_request"; id: string; tool: string; input: unknown }
  | { type: "permission_auto"; tool: string; input: unknown }
  | { type: "device_request"; id: string; name: string; input: unknown }
  | { type: "usage"; info: UsageInfo }
  | {
      type: "done";
      session_id: string;
      reply: string;
      via: string;
      note: string | null;
      in_routine_mode: boolean;
      force_local: boolean;
    }
  | { type: "error"; message: string };

/** Subset of the Claude CLI's `rate_limit_event.rate_limit_info` that the UI uses. */
export type UsageInfo = {
  status?: string;
  unifiedWindows?: Record<string, { utilization?: number; resetsAt?: number }>;
};
export type ChatResult = { reply: string; inRoutineMode: boolean; forceLocal: boolean; via: string };
export type HistoryResult = { turns: Turn[]; inRoutineMode: boolean; forceLocal: boolean; busy: boolean };

/**
 * One transport now, not two: the desktop app became a thin native shell (2026-08-30) — a
 * window pointed at a real `loki-server` instance's URL, no embedded `Core`, no Tauri IPC
 * bridge injected into that remote origin. So "the desktop app" and "a browser tab" are the
 * same client from here on — bearer-token `fetch` calls against `/api/v1/*`, full stop. The
 * earlier Tauri-IPC-vs-web branching (`invoke("send_message_v1", ...)` etc.) is gone
 * entirely, not just unreachable — those Tauri commands no longer exist on the Rust side
 * either (see `lib.rs::run`'s doc comment), so keeping the branch would have meant dead code
 * that errors confusingly if anything ever did reach it.
 *
 * Session id persistence: kept in `localStorage` (`loki_session_id`) so a reload doesn't
 * silently orphan a conversation mid-routine. Chat *history* also survives a `loki-server`
 * restart now (`Core::ensure_session_seeded` reads it back from the vault conversation log).
 */

const CLIENT_ID_KEY = "loki_client_id";

/** A stable id for this browser/app install, so a mirror can skip the echo of its own messages. */
export function getClientId(): string {
  try {
    let id = localStorage.getItem(CLIENT_ID_KEY);
    if (!id) {
      id = `c-${Math.random().toString(16).slice(2, 10)}${Date.now().toString(16)}`;
      localStorage.setItem(CLIENT_ID_KEY, id);
    }
    return id;
  } catch {
    return "c-ephemeral";
  }
}

let clientCaps: string[] | null = null;

/** What this client says about itself on every request (see the server's `bus::ClientInfo`).
 * `device_fs` is declared only inside the desktop app, the one client that can run the
 * `device_*` tools; a browser tab or phone declares nothing, so the server never routes file
 * or shell requests to it. */
export async function clientInfo(): Promise<{ id: string; kind: string; capabilities: string[] }> {
  if (clientCaps === null) clientCaps = (await canCaptureScreen()) ? ["device_fs"] : [];
  const kind = clientCaps.includes("device_fs") ? "desktop" : /Android/i.test(navigator.userAgent) ? "android" : "web";
  return { id: getClientId(), kind, capabilities: clientCaps };
}

const SESSION_STORAGE_KEY = "loki_session_id";
const TOKEN_STORAGE_KEY = "loki_api_token";

let sessionId: string | null = null;
try {
  sessionId = localStorage.getItem(SESSION_STORAGE_KEY);
} catch {
  // localStorage can throw in some contexts (private browsing, disabled storage) —
  // fall back to in-memory-only session tracking rather than breaking the app.
}

function rememberSessionId(id: string) {
  sessionId = id;
  try {
    localStorage.setItem(SESSION_STORAGE_KEY, id);
  } catch {
    // Same fallback as above — losing persistence is fine, losing the app isn't.
  }
}

export function getCurrentSessionId(): string | null {
  return sessionId;
}

/** Local-only reset — mirrors the sidebar plugin's `/clear` (`LokiSidebar.qml`'s
 * `clearConversation`) and Claude Code's own `/clear`: starts a genuinely new session (fresh
 * session_id) rather than just hiding old messages, and never touches the server-side
 * conversation log for the old session — that log stays in the vault exactly as it was, a new
 * one starts fresh on the next message. No API call; the caller (App.tsx) is responsible for
 * resetting its own displayed history/state to match. */
export function newSession(): string {
  const id = `web-${Date.now()}-${Math.random().toString(16).slice(2, 10)}`;
  rememberSessionId(id);
  return id;
}

/** Points the client at an existing conversation (the session picker). The caller reloads
 * history afterwards. */
export function switchSession(id: string) {
  rememberSessionId(id);
}

export type SessionSummary = { session_id: string; title: string; updated: number; turns: number };

export async function listSessions(): Promise<SessionSummary[]> {
  const token = getStoredApiToken();
  if (!token) return [];
  const res = await fetch("/api/v1/sessions", { headers: { Authorization: `Bearer ${token}` } });
  if (!res.ok) throw new Error(await res.text());
  return (await res.json()).sessions;
}

export async function renameSession(sessionId: string, title: string): Promise<string> {
  const token = getStoredApiToken();
  const res = await fetch("/api/v1/sessions/rename", {
    method: "POST",
    headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
    body: JSON.stringify({ session_id: sessionId, title }),
  });
  if (!res.ok) throw new Error(await res.text());
  return (await res.json()).title;
}

const SCREEN_CLEAR_MARKER_PREFIX = "loki_screen_cleared_count_";

/**
 * "Clear screen" is a display-only reset — it hides turns from the UI, it does not delete
 * anything server-side (the vault conversation log and knowledge store are untouched). Keyed
 * per session id and stored in `localStorage` (turn *count*, not content or a timestamp) so
 * the cleared view survives a reload without needing any server-side "hidden" concept: on
 * next load, `getHistory()` still returns everything, and the UI just slices off the first
 * `N` turns using this marker.
 */
export function getScreenClearMarker(sid: string | null): number {
  if (!sid) return 0;
  try {
    return Number(localStorage.getItem(SCREEN_CLEAR_MARKER_PREFIX + sid)) || 0;
  } catch {
    return 0;
  }
}

export function setScreenClearMarker(sid: string | null, count: number) {
  if (!sid) return;
  try {
    localStorage.setItem(SCREEN_CLEAR_MARKER_PREFIX + sid, String(count));
  } catch {
    // Best-effort, same as the other localStorage writes in this file — worst case the
    // clear doesn't survive a reload, it still works for the current page session.
  }
}

export function getStoredApiToken(): string {
  try {
    return localStorage.getItem(TOKEN_STORAGE_KEY) ?? "";
  } catch {
    return "";
  }
}

export function setStoredApiToken(token: string) {
  try {
    localStorage.setItem(TOKEN_STORAGE_KEY, token.trim());
  } catch {
    // Best-effort — see the localStorage try/catches above.
  }
}

/** Exchanges a username/password for the same shared API token every other integration uses
 * (server-side: `auth::login`), and stores it exactly as `setStoredApiToken` would — this is
 * a friendlier front door onto the same token, not a separate session concept. Throws with a
 * plain "invalid username or password" on failure (the server doesn't distinguish which). */
export async function login(username: string, password: string): Promise<void> {
  const res = await fetch("/api/login", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ username, password }),
  });
  if (!res.ok) throw new Error(res.status === 401 ? "Invalid username or password." : await res.text());
  const body = await res.json();
  setStoredApiToken(body.token);
}

/** `/api/config/*` and `/api/v1/*` share this — returns `{}` rather than throwing when no
 * token is set yet, so Settings/the chat UI can still render their "no token" state instead
 * of crashing on load. */
function authHeaderIfPresent(): Record<string, string> {
  const token = getStoredApiToken();
  return token ? { Authorization: `Bearer ${token}` } : {};
}

/** `images`: base64 PNG strings (no data: URI prefix) — e.g. a device-context screenshot
 * from `captureScreenshot` below. Optional — omitted entirely from the request body when
 * empty, matching the server's `#[serde(default)]` field. */
export async function sendMessage(message: string, images: string[] = []): Promise<ChatResult> {
  const token = getStoredApiToken();
  if (!token) {
    throw new Error(
      "No Loki API token set — open Settings and paste the token from ~/.config/loki/api_token on the machine running loki-server.",
    );
  }
  const res = await fetch("/api/v1/chat", {
    method: "POST",
    headers: { "Content-Type": "application/json", Authorization: `Bearer ${token}` },
    body: JSON.stringify({ session_id: sessionId ?? undefined, message, ...(images.length ? { images } : {}) }),
  });
  if (!res.ok) throw new Error(await res.text());
  const body = await res.json();
  rememberSessionId(body.session_id);
  return { reply: body.reply, inRoutineMode: body.in_routine_mode, forceLocal: body.force_local, via: body.via };
}

/**
 * Streaming variant of `sendMessage`: POSTs to `/api/v1/chat/stream` and calls `onEvent` for
 * each server-sent event. Resolves when the stream ends; rejects on HTTP/network failure.
 * Aborting `signal` closes the connection, which makes the server cancel the turn (it kills
 * the Claude process) — that is how the Stop button works.
 *
 * Uses `fetch` + a stream reader rather than `EventSource` because the endpoint needs the
 * bearer-token `Authorization` header, which `EventSource` can't send.
 */
export async function streamMessage(
  message: string,
  images: string[],
  onEvent: (ev: StreamEvent) => void,
  signal: AbortSignal,
): Promise<void> {
  const token = getStoredApiToken();
  if (!token) {
    throw new Error("No Loki API token set — log in or open Settings and paste the token.");
  }
  const res = await fetch("/api/v1/chat/stream", {
    method: "POST",
    headers: { "Content-Type": "application/json", Authorization: `Bearer ${token}` },
    body: JSON.stringify({
      session_id: sessionId ?? undefined,
      message,
      client: await clientInfo(),
      detach: true, // survive the connection dropping (phone backgrounded); Stop calls cancelTurn
      ...(images.length ? { images } : {}),
    }),
    signal,
  });
  if (!res.ok || !res.body) throw new Error(await res.text());
  const reader = res.body.pipeThrough(new TextDecoderStream()).getReader();
  let buf = "";
  for (;;) {
    const { value, done } = await reader.read();
    if (done) break;
    buf += value;
    let sep: number;
    while ((sep = buf.indexOf("\n\n")) >= 0) {
      const frame = buf.slice(0, sep);
      buf = buf.slice(sep + 2);
      const data = frame
        .split("\n")
        .filter((l) => l.startsWith("data:"))
        .map((l) => l.slice(5).trimStart())
        .join("\n");
      if (!data) continue; // keep-alive comment frames have no data
      const ev = JSON.parse(data) as StreamEvent;
      if (ev.type === "started") rememberSessionId(ev.session_id);
      onEvent(ev);
    }
  }
}

/** Stops the running turn of the current conversation (Stop button). */
export async function cancelTurn(): Promise<void> {
  if (!sessionId) return;
  await fetch("/api/v1/chat/cancel", {
    method: "POST",
    headers: { "Content-Type": "application/json", ...authHeaderIfPresent() },
    body: JSON.stringify({ session_id: sessionId }),
  }).catch(() => {});
}

/**
 * Attaches to the current conversation without sending anything and reports every event of every
 * turn, including ones started by other clients (the phone, the sidebar, another window), plus
 * approval and device requests. Reconnects with a short delay until `signal` aborts.
 */
export async function watchSession(
  sid: string,
  onEvent: (ev: StreamEvent) => void,
  signal: AbortSignal,
): Promise<void> {
  const token = getStoredApiToken();
  if (!token) return;
  while (!signal.aborted) {
    try {
      const me = await clientInfo();
      const q = new URLSearchParams({ session_id: sid, client_id: me.id, kind: me.kind, caps: me.capabilities.join(",") });
      const res = await fetch(`/api/v1/sessions/events?${q}`, { headers: { Authorization: `Bearer ${token}` }, signal });
      if (!res.ok || !res.body) throw new Error(`watch failed: HTTP ${res.status}`);
      const reader = res.body.pipeThrough(new TextDecoderStream()).getReader();
      let buf = "";
      for (;;) {
        const { value, done } = await reader.read();
        if (done) break;
        buf += value;
        let sep: number;
        while ((sep = buf.indexOf("\n\n")) >= 0) {
          const frame = buf.slice(0, sep);
          buf = buf.slice(sep + 2);
          const data = frame
            .split("\n")
            .filter((l) => l.startsWith("data:"))
            .map((l) => l.slice(5).trimStart())
            .join("\n");
          if (data) onEvent(JSON.parse(data) as StreamEvent);
        }
      }
    } catch {
      // network drop or server restart: fall through to the retry delay
    }
    if (signal.aborted) return;
    await new Promise((r) => setTimeout(r, 2000));
  }
}

/** Captures the screen via the Tauri `capture_screenshot` command (#13) — only reachable when
 * running as the desktop app against an origin `capabilities/remote-capture.json` allowlists
 * (see that file and `lib.rs::run`'s doc comment). Returns `null` in a plain browser tab, or
 * on any capture failure, rather than throwing — callers treat "no screenshot available" as a
 * normal, expected case, not an error state. */
export async function captureScreenshot(): Promise<string | null> {
  try {
    if (!(await canCaptureScreen())) return null;
    const { invoke } = await import("@tauri-apps/api/core");
    return await invoke<string>("capture_screenshot");
  } catch {
    return null;
  }
}

/** Whether `captureScreenshot` is worth offering at all — true only in the desktop app,
 * against an origin `capabilities/remote-capture.json` allowlists. Split out from
 * `captureScreenshot` itself so the UI can decide up front whether to show the button, without
 * actually triggering a capture just to find out. */
export async function canCaptureScreen(): Promise<boolean> {
  try {
    const { isTauri } = await import("@tauri-apps/api/core");
    return isTauri();
  } catch {
    return false;
  }
}

export async function getHistory(): Promise<HistoryResult> {
  if (!sessionId) return { turns: [], inRoutineMode: false, forceLocal: false, busy: false };
  const token = getStoredApiToken();
  if (!token) return { turns: [], inRoutineMode: false, forceLocal: false, busy: false };
  const res = await fetch(`/api/v1/history?session_id=${encodeURIComponent(sessionId)}`, {
    headers: { Authorization: `Bearer ${token}` },
  });
  if (!res.ok) throw new Error(await res.text());
  const body = await res.json();
  return { turns: body.turns, inRoutineMode: body.in_routine_mode, forceLocal: body.force_local, busy: !!body.busy };
}

/** A `[llm]`/`[embedding]` backend as read back from config.toml + the encrypted secrets
 * file — `has_api_key` tells you whether one is set without ever exposing it. */
export type PublicBackend = {
  vendor: string;
  has_api_key: boolean;
  [field: string]: string | number | boolean;
};

export async function getLlmConfig(section: string): Promise<PublicBackend | null> {
  const res = await fetch(`/api/config/${section}`, { headers: authHeaderIfPresent() });
  if (!res.ok) throw new Error(await res.text());
  return res.json();
}

export async function saveLlmConfig(section: string, input: Record<string, unknown>): Promise<void> {
  const res = await fetch(`/api/config/${section}`, {
    method: "POST",
    headers: { "Content-Type": "application/json", ...authHeaderIfPresent() },
    body: JSON.stringify(input),
  });
  if (!res.ok) throw new Error(await res.text());
}

/** Mirrors Rust's `ClaudeConfig` (`src-tauri/src/config.rs`). */
export type ClaudeConfig = {
  binary_path: string;
  timeout_secs: number;
  routine_working_dir: string | null;
  bridge_binary?: string | null;
  bridge_url?: string;
  extra_allowed_tools?: string[];
  server_tools_with_approval?: boolean;
};

export async function getClaudeConfig(): Promise<ClaudeConfig> {
  const res = await fetch("/api/config/claude", { headers: authHeaderIfPresent() });
  if (!res.ok) throw new Error(await res.text());
  return res.json();
}

export async function saveClaudeConfig(input: ClaudeConfig): Promise<void> {
  const res = await fetch("/api/config/claude", {
    method: "POST",
    headers: { "Content-Type": "application/json", ...authHeaderIfPresent() },
    body: JSON.stringify(input),
  });
  if (!res.ok) throw new Error(await res.text());
}


/** Answers a `permission_request` / `device_request` the server is waiting on. */
export async function respondBridge(id: string, value: unknown): Promise<void> {
  const res = await fetch("/api/v1/bridge/respond", {
    method: "POST",
    headers: { "Content-Type": "application/json", ...authHeaderIfPresent() },
    body: JSON.stringify({ id, value }),
  });
  // 404 just means the turn already moved on (timed out or was stopped) — nothing to do.
  if (!res.ok && res.status !== 404) throw new Error(await res.text());
}

/** Runs a `device_*` tool on this machine through the desktop app's Tauri command. In a plain
 * browser tab there is no device access, which is reported back to the model as an error. */
export async function runDeviceTool(name: string, input: unknown): Promise<{ content: string; is_error: boolean }> {
  if (!(await canCaptureScreen())) {
    return { content: "this client is a browser tab and cannot access files or run commands — use the Loki desktop app", is_error: true };
  }
  try {
    const { invoke } = await import("@tauri-apps/api/core");
    return { content: await invoke<string>("device_call", { name, input }), is_error: false };
  } catch (e) {
    return { content: String(e), is_error: true };
  }
}

// ---- "Always allow" rules: stored on the server, shared by every client/device ----

/** Mirrors `permission_rules::can_always` — hides the button for actions that always ask. The
 * server is authoritative and rejects these anyway. */
export function canAlwaysAllow(tool: string): boolean {
  return !/delete|merge|push|publish|send|trash|spam|remove|drop/i.test(tool);
}

export async function addPermissionRule(tool: string, input: unknown): Promise<void> {
  const res = await fetch("/api/v1/permissions/rules", {
    method: "POST",
    headers: { "Content-Type": "application/json", ...authHeaderIfPresent() },
    body: JSON.stringify({ tool, input }),
  });
  if (!res.ok) throw new Error(await res.text());
}

export async function listPermissionRules(): Promise<string[]> {
  const res = await fetch("/api/v1/permissions/rules", { headers: authHeaderIfPresent() });
  if (!res.ok) throw new Error(await res.text());
  return (await res.json()).rules;
}

export async function removePermissionRule(rule: string): Promise<void> {
  const res = await fetch("/api/v1/permissions/rules/remove", {
    method: "POST",
    headers: { "Content-Type": "application/json", ...authHeaderIfPresent() },
    body: JSON.stringify({ rule }),
  });
  if (!res.ok && res.status !== 404) throw new Error(await res.text());
}


// ---- Session host trust: non-destructive remote commands on a host stop asking for a while ----

export type HostGrants = { hosts: { host: string; seconds_left: number }[]; configured: string[] };

export async function listHostGrants(): Promise<HostGrants> {
  const res = await fetch("/api/v1/permissions/hosts", { headers: authHeaderIfPresent() });
  if (!res.ok) throw new Error(await res.text());
  return res.json();
}

export async function grantHost(host: string, minutes = 120): Promise<void> {
  const res = await fetch("/api/v1/permissions/hosts/grant", {
    method: "POST",
    headers: { "Content-Type": "application/json", ...authHeaderIfPresent() },
    body: JSON.stringify({ host, minutes }),
  });
  if (!res.ok) throw new Error(await res.text());
}

export async function revokeHost(host: string): Promise<void> {
  const res = await fetch("/api/v1/permissions/hosts/revoke", {
    method: "POST",
    headers: { "Content-Type": "application/json", ...authHeaderIfPresent() },
    body: JSON.stringify({ host }),
  });
  if (!res.ok && res.status !== 404) throw new Error(await res.text());
}

// ---- engine picker ----

export type Engine = { id: string; label: string; kind: string; cloud: boolean };
export type EngineList = { engines: Engine[]; default: string; session: string | null; claude_mode: string; summary?: string };

export async function listEngines(): Promise<EngineList> {
  const q = sessionId ? `?session_id=${encodeURIComponent(sessionId)}` : "";
  const res = await fetch(`/api/v1/engines${q}`, { headers: authHeaderIfPresent() });
  if (!res.ok) throw new Error(await res.text());
  return res.json();
}

/** Sets this conversation's engine (`scope: "session"`, creating the session id if the
 * conversation hasn't started) or the server-wide default (`scope: "default"`). `auto` clears. */
export async function setEngine(engine: string, scope: "session" | "default"): Promise<void> {
  if (scope === "session" && !sessionId) rememberSessionId(newSession());
  const res = await fetch("/api/v1/engines", {
    method: "POST",
    headers: { "Content-Type": "application/json", ...authHeaderIfPresent() },
    body: JSON.stringify({ engine, session_id: scope === "session" ? sessionId : undefined }),
  });
  if (!res.ok) throw new Error(await res.text());
}


// ---- feedback, audits, lessons, troubleshooting ----

async function postJson(path: string, body: unknown): Promise<Response> {
  const res = await fetch(path, {
    method: "POST",
    headers: { "Content-Type": "application/json", ...authHeaderIfPresent() },
    body: JSON.stringify(body),
  });
  if (!res.ok) throw new Error(await res.text());
  return res;
}

export async function rateTurn(index: number, rating: "up" | "down" | "none"): Promise<void> {
  if (!sessionId) return;
  await postJson("/api/v1/feedback", { session_id: sessionId, index, rating });
}

/** Asks Claude to audit one assistant message (can take a minute). */
export async function auditTurn(index: number): Promise<Audit> {
  if (!sessionId) throw new Error("No conversation yet.");
  return (await postJson("/api/v1/audit", { session_id: sessionId, index })).json();
}

export async function getLessons(): Promise<{ lessons: Lesson[]; settings: { auto: "off" | "downvotes" | "all_local" } }> {
  const res = await fetch("/api/v1/lessons", { headers: authHeaderIfPresent() });
  if (!res.ok) throw new Error(await res.text());
  return res.json();
}

export async function updateLesson(id: string, action: "approve" | "reject" | "delete" | "edit", text?: string): Promise<void> {
  await postJson("/api/v1/lessons/update", { id, action, text });
}

export async function setAuditAuto(auto: "off" | "downvotes" | "all_local"): Promise<void> {
  await postJson("/api/v1/audit/settings", { auto });
}

export type TroubleshootJob = { id: string; status: "running" | "done" | "error"; result: string };

export async function startTroubleshoot(note: string, ui: unknown): Promise<string> {
  if (!sessionId) throw new Error("Start a conversation first.");
  return (await (await postJson("/api/v1/troubleshoot", { session_id: sessionId, note, ui })).json()).id;
}

export async function troubleshootStatus(id: string): Promise<TroubleshootJob> {
  const res = await fetch(`/api/v1/troubleshoot/${id}`, { headers: authHeaderIfPresent() });
  if (!res.ok) throw new Error(await res.text());
  return res.json();
}

/** "2026-10-09 15:43:50 EDT" → "3:43 PM" (today) or "Oct 9, 3:43 PM". Unparseable → as given. */
export function formatStamp(at?: string): string {
  if (!at) return "";
  const m = at.match(/^(\d{4})-(\d{2})-(\d{2}) (\d{2}):(\d{2})/);
  if (!m) return at;
  const d = new Date(+m[1], +m[2] - 1, +m[3], +m[4], +m[5]);
  const time = d.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
  const now = new Date();
  if (d.toDateString() === now.toDateString()) return time;
  const date = d.toLocaleDateString([], { month: "short", day: "numeric", ...(d.getFullYear() !== now.getFullYear() ? { year: "numeric" } : {}) });
  return `${date}, ${time}`;
}

/** Local "now" in the server's log format, for turns created in this page session. */
export function nowStamp(): string {
  const d = new Date();
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
}

export type SlashCommand = { name: string; description: string };

export async function listCommands(): Promise<SlashCommand[]> {
  const res = await fetch("/api/v1/commands", { headers: authHeaderIfPresent() });
  if (!res.ok) return [];
  return (await res.json()).commands;
}

/** "Fix with Claude": stage (`fix`), `apply`, or `discard` the changes for a troubleshoot report.
 * Runs on the laptop (where the source lives); returns a job id to poll with `troubleshootStatus`. */
export async function startFix(reportId: string, action: "fix" | "apply" | "discard", note = ""): Promise<string> {
  return (await (await postJson("/api/v1/fix", { id: reportId, action, note })).json()).id;
}

export type FixOutcome = {
  status: "done" | "error";
  result: string;
  diffstat?: string;
  backend?: boolean;
  frontend?: boolean;
  sidebar?: string[];
  nothing_changed?: boolean;
};

/** The CLI prints one JSON object as its last line; anything else is an error message. */
export function parseFixOutcome(job: TroubleshootJob): FixOutcome {
  const last = job.result.trim().split("\n").pop() ?? "";
  try {
    return JSON.parse(last) as FixOutcome;
  } catch {
    return { status: "error", result: job.result || "No answer from the laptop." };
  }
}


// ---- speech (STT/TTS through the model server's worker) ----

export async function getSpeechStatus(): Promise<{ configured: boolean; voice: string }> {
  try {
    const res = await fetch("/api/v1/speech/status", { headers: authHeaderIfPresent() });
    return res.ok ? res.json() : { configured: false, voice: "" };
  } catch {
    return { configured: false, voice: "" };
  }
}

/** Sends a recording to the speech-to-text model on the model server; resolves to the transcript. */
export async function transcribeAudio(blob: Blob): Promise<string> {
  const res = await fetch("/api/v1/stt", {
    method: "POST",
    headers: { "Content-Type": blob.type || "audio/webm", ...authHeaderIfPresent() },
    body: blob,
  });
  if (!res.ok) throw new Error(await res.text());
  return (await res.json()).text as string;
}

/** Asks the text-to-speech model on the model server to read `text`; resolves to the audio. */
export async function synthesizeSpeech(text: string): Promise<Blob> {
  const res = await fetch("/api/v1/tts", {
    method: "POST",
    headers: { "Content-Type": "application/json", ...authHeaderIfPresent() },
    body: JSON.stringify({ text }),
  });
  if (!res.ok) throw new Error(await res.text());
  return res.blob();
}

// ---- past troubleshoot reports ----

export type FixState = "staged" | "applying" | "applied" | "discarded" | "error";
export type ReportSummary = {
  id: string;
  created: number;
  status: "done" | "error" | "running";
  client: string;
  note: string;
  summary: string;
  fix: { state: FixState; at: number } | null;
};
export type ReportDetail = Omit<ReportSummary, "fix"> & {
  result: string;
  fix: { state: FixState; at: number; outcome?: FixOutcome; last?: FixOutcome } | null;
};

export async function listReports(): Promise<ReportSummary[]> {
  const res = await fetch("/api/v1/reports", { headers: authHeaderIfPresent() });
  if (!res.ok) throw new Error(await res.text());
  return (await res.json()).reports;
}

export async function getReport(id: string): Promise<ReportDetail> {
  const res = await fetch(`/api/v1/reports/${encodeURIComponent(id)}`, { headers: authHeaderIfPresent() });
  if (!res.ok) throw new Error(await res.text());
  return res.json();
}

// ---- push notifications ----

export async function getPushStatus(): Promise<{ configured: boolean; key: string | null; devices: number }> {
  const res = await fetch("/api/v1/push/status", { headers: authHeaderIfPresent() });
  if (!res.ok) throw new Error(await res.text());
  return res.json();
}

export async function sendPushSubscription(sub: PushSubscriptionJSON): Promise<void> {
  await postJson("/api/v1/push/subscribe", { endpoint: sub.endpoint, keys: sub.keys, client_id: getClientId() });
}

export async function removePushSubscription(endpoint: string): Promise<void> {
  await postJson("/api/v1/push/unsubscribe", { endpoint });
}

export async function sendTestPush(): Promise<void> {
  await postJson("/api/v1/push/test", {});
}

// ---- context controls ----

export type ContextReport = {
  window: number;
  used: number;
  breakdown: { system: number; tools: number; summary: number; history: number };
  history_budget: number;
  turns: { total: number; summarized: number; verbatim: number };
  autocompact: { enabled: boolean; at_tokens: number; at_turns: number; unsummarized_tokens: number; unsummarized_turns: number; percent: number };
  last_measured: { promptTokens: number; completionTokens: number } | null;
  settings: { autocompact: boolean; history_fraction: number; num_ctx: number; max_turns: number };
};

export async function getContext(sessionId: string): Promise<ContextReport> {
  const res = await fetch(`/api/v1/context?session_id=${encodeURIComponent(sessionId)}`, { headers: authHeaderIfPresent() });
  if (!res.ok) throw new Error(await res.text());
  return res.json();
}

export async function setContextSettings(patch: Partial<ContextReport["settings"]>): Promise<void> {
  const res = await fetch("/api/v1/context/settings", {
    method: "POST",
    headers: { "Content-Type": "application/json", ...authHeaderIfPresent() },
    body: JSON.stringify(patch),
  });
  if (!res.ok) throw new Error(await res.text());
}

// ---- workspace trust ----

export type TrustedFolder = { host: string; path: string; seconds_left: number | null };

export async function listWorkspaces(): Promise<{ workspaces: TrustedFolder[]; hosts: string[] }> {
  const res = await fetch("/api/v1/permissions/workspaces", { headers: authHeaderIfPresent() });
  if (!res.ok) throw new Error(await res.text());
  return res.json();
}

export async function trustFolder(host: string, path: string, opts: { minutes?: number; permanent?: boolean }): Promise<void> {
  const res = await fetch("/api/v1/permissions/workspaces/grant", {
    method: "POST",
    headers: { "Content-Type": "application/json", ...authHeaderIfPresent() },
    body: JSON.stringify({ host, path, ...opts }),
  });
  if (!res.ok) throw new Error(await res.text());
}

export async function untrustFolder(host: string, path: string): Promise<void> {
  const res = await fetch("/api/v1/permissions/workspaces/revoke", {
    method: "POST",
    headers: { "Content-Type": "application/json", ...authHeaderIfPresent() },
    body: JSON.stringify({ host, path }),
  });
  if (!res.ok && res.status !== 404) throw new Error(await res.text());
}

// ---- needs you ----

export type NeedsItem = {
  id: string;
  title: string;
  why: string;
  source: string;
  source_path: string;
  kind: string;
  score: number;
  when: string | null;
  next_action: string | null;
  link?: string | null;
  blurb?: string | null;
};
export type BriefingRow = { head: string; text: string; link: string | null; status?: "ok" | "fail" | "off" | "warn" | null; group?: string | null; heading?: boolean };
export type BriefingSeries = { name: string; unit: string; points: (number | null)[] };
export type Briefing = {
  id: string;
  kind: "shape" | "week" | "weather" | "countdowns" | "backlog" | "rhythm" | "finance" | "news" | "signals" | "leads" | "mail" | "mailreview" | "workflows" | "zotero" | "saved" | "network" | "loki" | "day" | "start";
  series: BriefingSeries[];
  title: string;
  blurb: string;
  pill: string;
  link: string | null;
  source_path: string;
  note?: string | null;
  rows: BriefingRow[];
  /** Structured data for the tab's dashboard, when the server provides it. */
  data?: unknown;
};
export type NeedsYou = {
  date: string;
  weekday: string;
  state: { starting_state: string; mode: string; note: string };
  items: NeedsItem[];
  briefings?: Briefing[];
  hidden_by_capacity: number;
  snoozed: number;
  blurbs_pending?: boolean;
  focus: { current: { start: string; end: string; type: string; what: string } | null; next: { start: string; end: string; type: string; what: string } | null };
};

export async function getNeedsYou(): Promise<NeedsYou> {
  const res = await fetch("/api/v1/needs-you", { headers: authHeaderIfPresent() });
  if (!res.ok) throw new Error(await res.text());
  return res.json();
}

export async function decideNeedsYou(item: Pick<NeedsItem, "id" | "title">, action: "done" | "defer" | "not_now" | "rate", opts: { days?: number; reason?: string; rating?: "up" | "down" } = {}): Promise<void> {
  const res = await fetch("/api/v1/needs-you/decision", {
    method: "POST",
    headers: { "Content-Type": "application/json", ...authHeaderIfPresent() },
    body: JSON.stringify({ id: item.id, title: item.title, action, days: opts.days, reason: opts.reason ?? "", rating: opts.rating ?? "" }),
  });
  if (!res.ok) throw new Error(await res.text());
}

export async function getDay(date: string): Promise<Briefing> {
  const res = await fetch(`/api/v1/day?date=${encodeURIComponent(date)}`, { headers: authHeaderIfPresent() });
  if (!res.ok) throw new Error(await res.text());
  return res.json();
}

export type MailReviewStatus = { status: string; phase: string; done: number; total: number; error: string; accounts: { email: string; unread_total: number; sampled: number }[] };

async function authed(path: string, init: RequestInit = {}): Promise<Response> {
  const token = getStoredApiToken();
  const res = await fetch(path, { ...init, headers: { ...(init.headers ?? {}), Authorization: `Bearer ${token}` } });
  if (!res.ok) throw new Error(await res.text());
  return res;
}
export const mailReviewStatus = async (): Promise<MailReviewStatus> => (await authed("/api/v1/mail/review/status")).json();
export const mailReviewStart = async (): Promise<void> => void (await authed("/api/v1/mail/review/start", { method: "POST" }));
export const mailReviewPage = async (): Promise<Briefing> => (await authed("/api/v1/mail/review")).json();
export const mailPlanText = async (): Promise<string> => (await authed("/api/v1/mail/plan")).text();
export async function runImprove(): Promise<string> {
  return (await authed("/api/v1/improve/run", { method: "POST" })).text();
}

export type InboxItem = {
  account: string;
  id: string;
  from: string;
  subject: string;
  at_ms: number;
  unread: boolean;
  snippet: string;
  category: string;
  urgency: string;
  class: string;
  reply: boolean;
  why: string;
  quadrant: string;
  gain: string;
  next: string;
  archived?: boolean;
};
export type InboxPage = { status: string; phase: string; done: number; total: number; error: string; updated: number; items: InboxItem[] };
export const mailInbox = async (): Promise<InboxPage> => (await authed("/api/v1/mail/inbox")).json();
export const mailInboxScan = async (matrix: boolean): Promise<void> =>
  void (await authed("/api/v1/mail/inbox/scan", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ matrix }) }));
export const mailInboxAct = async (account: string, ids: string[], action: "read" | "unread" | "trash" | "archive" | "unarchive"): Promise<string> =>
  (await authed("/api/v1/mail/inbox/act", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ account, ids, action }) })).text();
export const mailInboxDraft = async (account: string, id: string): Promise<{ draft: string; saved: string }> =>
  (await authed("/api/v1/mail/inbox/draft", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ account, id }) })).json();
export type InboxMessage = { account: string; id: string; from: string; to: string; cc: string; subject: string; date: string; body: string; attachments: string[]; unread: boolean; link: string };
export const mailInboxMessage = async (account: string, id: string): Promise<InboxMessage> =>
  (await authed(`/api/v1/mail/inbox/message?account=${encodeURIComponent(account)}&id=${encodeURIComponent(id)}`)).json();

export type GeoEvent = { title: string; category: string; place: string; lat: number; lon: number; bluf: string; source: string; url: string; precision?: string };
export type GeoDay = {
  date: string;
  events: GeoEvent[];
  layers?: {
    fires?: { lat: number; lon: number; note: string; dates: string; frp_mw: number }[];
    conflict?: { lat: number; lon: number; deaths: number; where: string; note: string; date: string }[];
    hazards?: { lat: number; lon: number; name: string; alert: string; url?: string }[];
  };
};
export const newsGeoIndex = async (): Promise<{ dates: { date: string; events: number }[] }> => (await authed("/api/v1/news/geo?date=index")).json();
export const newsGeoDay = async (date: string): Promise<GeoDay> => (await authed(`/api/v1/news/geo?date=${encodeURIComponent(date)}`)).json();

export type ExtractKind = "event" | "todo" | "knowledge" | "reference";
export type ExtractProposal = {
  kind: ExtractKind;
  title: string;
  date: string;
  start: string;
  end: string;
  place: string;
  people: string;
  purpose: string;
  next_action: string;
  area: string;
  estimate: string;
  topic: string;
  note: string;
};
export type ExtractSource = { source: "mail"; account: string; id: string } | { source: "text"; text: string; label: string };
export const extractRun = async (src: ExtractSource): Promise<{ key: string; seen: boolean; label: string; link: string; proposals: ExtractProposal[] }> =>
  (await authed("/api/v1/extract", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(src) })).json();
export const extractApply = async (key: string, label: string, link: string, proposals: ExtractProposal[]): Promise<string[]> =>
  (await (await authed("/api/v1/extract/apply", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ key, label, link, proposals }) })).json()).results;

export type Lead = {
  id: number; source: string; title: string; url: string; author: string; published_at: string; kind: string; score: number;
  effort: string; budget: string; summary: string; scope: string; tools: string[]; lang: string; forme: number; forme_why: string;
  status: string; cluster: number | null; llm: boolean; process: string; pain: string; family_size?: number;
};
export type LeadStep = { t: string; done: boolean };
export type LeadDetail = Lead & {
  text: string; notes: string; draft: string; steps: LeadStep[];
  related: { id: number; title: string; source: string; kind: string; status: string; sim: number }[];
  family?: { id: number; label: string; reuse: string };
};
export type LeadFacets = { total: number; kind: Record<string, number>; source: Record<string, number>; tool: Record<string, number>; status: Record<string, number>; process: Record<string, number>; effort: Record<string, number>; flags: Record<string, number> };
export type LeadFamily = { id: number; label: string; reuse: string; size: number; steps: LeadStep[]; members: Lead[] };

/** The leads app's API (the leads app), reached through Loki's server. */
export async function leadsApi<T = unknown>(path: string, init: RequestInit = {}): Promise<T> {
  const res = await authed(`/api/v1/leads/${path}`, { ...init, headers: { ...(init.headers ?? {}), ...(init.body ? { "Content-Type": "application/json" } : {}) } });
  return res.json();
}
export async function leadsDownload(path: string, fallbackName: string): Promise<void> {
  const res = await authed(`/api/v1/leads/${path}`);
  const name = /filename="([^"]+)"/.exec(res.headers.get("content-disposition") ?? "")?.[1] ?? fallbackName;
  const url = URL.createObjectURL(await res.blob());
  const a = document.createElement("a");
  a.href = url;
  a.download = name;
  a.click();
  setTimeout(() => URL.revokeObjectURL(url), 2000);
}

export type McpServerInfo = { name: string; source: "claude" | "config" | "added"; enabled: boolean; transport: "command" | "url"; target: string; env_keys: string[]; connected: boolean; tools: number; error?: string | null; removable: boolean };
export const mcpServers = async (): Promise<McpServerInfo[]> => (await (await authed("/api/v1/mcp/servers")).json()).servers;
export const mcpTools = async (name: string): Promise<{ name: string; description: string }[]> => (await (await authed(`/api/v1/mcp/servers/${encodeURIComponent(name)}/tools`)).json()).tools;
export const mcpAction = async (body: { action: string; name: string; command?: string; args?: string[]; url?: string; env?: Record<string, string> }): Promise<string> =>
  (await (await authed("/api/v1/mcp/action", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) })).json()).message;

/** Ticks (true), reopens (false) or only comments on (null) one checkbox task in a daily or sprint note. */
export async function markTask(path: string, text: string, section: string, done: boolean | null, comment: string): Promise<string> {
  return (await (await authed("/api/v1/tasks/mark", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ path, text, section, done, comment }) })).json()).message;
}

/** The whole text of one backlog note. */
export async function getBacklogItem(path: string): Promise<string> {
  return (await (await authed(`/api/v1/backlog/item?path=${encodeURIComponent(path)}`)).json()).content;
}

/** Edits one backlog note: save (whole text), comment, status (a label), or close (done | superseded | dropped, filed in the archive). */
export async function backlogEdit(req: { path: string; action: "save" | "comment" | "status" | "close"; content?: string; base?: string; comment?: string; status?: string }): Promise<string> {
  return (await (await authed("/api/v1/backlog/item", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(req) })).json()).message;
}

/** Asks the Ledger (the ledger) to sync now; resolves with what Loki did. */
export async function financeSyncNow(): Promise<string> {
  return (await (await authed("/api/v1/finance/sync", { method: "POST" })).json()).message;
}

/** Adds the /bod and /eod (or /bow and /eow) to-dos to a daily or sprint note if they're missing. */
export async function ensureRoutines(path: string): Promise<boolean> {
  return (await (await authed("/api/v1/tasks/ensure-routines", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ path }) })).json()).added;
}

export type NoteProposal = { op: "create" | "append"; path: string; content: string; heading: string; why: string };
export const notesFind = async (q: string): Promise<string[]> => (await (await authed(`/api/v1/notes/find?q=${encodeURIComponent(q)}`)).json()).paths;
export const notesPlan = async (req: { title: string; text: string; mode: "new" | "update" | "auto"; targets: string[] }): Promise<{ proposals: NoteProposal[]; looked_at: string[] }> =>
  (await authed("/api/v1/notes/plan", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(req) })).json();
export const notesApply = async (proposals: NoteProposal[]): Promise<string[]> =>
  (await (await authed("/api/v1/notes/apply", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ proposals }) })).json()).results;

/** Starts a runnable n8n workflow (News digest, Leads scan, Market watch). `force` re-runs even if today's note exists. */
export const runWorkflow = async (slug: string, force = false, input = ""): Promise<void> => {
  const res = await authed("/api/v1/workflows/run", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ slug, force, input }) });
  if (!res.ok) throw new Error((await res.text()) || `HTTP ${res.status}`);
};

export type NewsletterSender = { address: string; name: string; count: number; last_ms: number; bucket: "" | "news" | "market" | "ignore" };
export const newsletterSenders = async (): Promise<NewsletterSender[]> => (await (await authed("/api/v1/mail/newsletters/senders")).json()).senders;
export const setNewsletterSender = async (address: string, name: string, bucket: string): Promise<void> => {
  const res = await authed("/api/v1/mail/newsletters/senders", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ address, name, bucket }) });
  if (!res.ok) throw new Error((await res.text()) || `HTTP ${res.status}`);
};

/** One entry of the routine registry in the profile (what a tab's buttons run). */
export type RoutineDef = { id: string; label: string; hint: string; period: string; tabs: string[]; run: "" | "chat" | "workflow"; input: "" | "url"; ask: string; todo: string };
export const getRoutines = async (): Promise<RoutineDef[]> => (await (await authed("/api/v1/routines")).json()).routines;

export type UpdateCheck = { current: string; latest: string | null; newer: boolean; url?: string; install?: string; message: string };

export async function getVersion(): Promise<{ version: string; install: string }> {
  const res = await fetch("/api/v1/version", { headers: authHeaderIfPresent() });
  if (!res.ok) throw new Error(await res.text());
  return res.json();
}

/** Contacts the public repository's releases. Only called from the "Check for updates" button. */
export async function checkForUpdate(): Promise<UpdateCheck> {
  const res = await fetch("/api/v1/version/check", { method: "POST", headers: authHeaderIfPresent() });
  if (!res.ok) throw new Error(await res.text());
  return res.json();
}
