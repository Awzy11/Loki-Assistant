import { useEffect, useRef, useState } from "react";
import { BlockList, Markdown } from "./Blocks";
import {
  streamMessage,
  getHistory,
  captureScreenshot,
  canCaptureScreen,
  getStoredApiToken,
  getCurrentSessionId,
  getScreenClearMarker,
  setScreenClearMarker,
  newSession,
  listSessions,
  cancelTurn,
  renameSession,
  listEngines,
  setEngine,
  switchSession,
  respondBridge,
  watchSession,
  getClientId,
  runDeviceTool,
  addPermissionRule,
  formatStamp,
  listCommands,
  type SlashCommand,
  nowStamp,
  type Block,
  type EngineList,
  type SessionSummary,
  type StreamEvent,
  type Turn,
  type UsageInfo,
  type ContextReport,
  getContext,
} from "./transport";
import Settings from "./Settings";
import Lessons from "./Lessons";
import Mic from "./Mic";
import ContextPanel, { ContextChip } from "./ContextPanel";
import Reports from "./Reports";
import NeedsYouPage from "./NeedsYou";
import HomePage from "./HomePage";
import NeedsPage from "./NeedsPage";
import BriefingPage from "./BriefingPage";
import MailReviewPage from "./MailReviewPage";
import MailDashboard from "./MailDashboard";
import NotesDock from "./NotesDock";
import NewsDashboard from "./NewsDashboard";
import LeadsDashboard from "./LeadsDashboard";
import { RunContext } from "./routines";
import { RefreshContext } from "./refresh";
import DayPage from "./DayPage";
import ChatLanding from "./ChatLanding";
import Toast, { type ToastData } from "./Toast";
import { desktopNotify, getAlertPrefs, playChime } from "./alerts";
import TopNav from "./TopNav";
import { navigate, parseRoute, usePath } from "./router";
import { useNeeds } from "./useNeeds";
import Troubleshoot from "./Troubleshoot";
import TurnActions from "./TurnActions";
import Login from "./Login";
import "./App.css";

/** Per-provider badge shown on assistant turns that carry `via` (see `Turn`'s doc comment —
 * only turns added this page session have it). `cloud: true` picks the amber cloud styling,
 * `cloud: false` the green local styling, matching the session-level banner's color scheme.
 * An unrecognized `via` value (a future BYOK vendor added to the backend before this map is
 * updated) still renders — generic cloud icon, capitalized name — rather than disappearing. */
const VIA_BADGES: Record<string, { label: string; icon: string; cloud: boolean }> = {
  claude: { label: "Claude Code", icon: "☁️", cloud: true },
  "claude-inference": { label: "Claude (Loki's loop)", icon: "☁️", cloud: true },
  gemini: { label: "Gemini", icon: "☁️", cloud: true },
  anthropic: { label: "Anthropic", icon: "☁️", cloud: true },
  openai_compatible: { label: "OpenAI-compatible", icon: "☁️", cloud: true },
  ollama: { label: "Ollama", icon: "🔒", cloud: false },
  system: { label: "System", icon: "⚙️", cloud: false },
};

function viaBadge(via?: string) {
  if (!via) return null;
  const known = VIA_BADGES[via] ?? { label: via.charAt(0).toUpperCase() + via.slice(1), icon: "☁️", cloud: true };
  if (known.label === "System") return null; // toggle confirmations aren't worth badging
  return (
    <span className={`loki-via-badge ${known.cloud ? "loki-via-cloud" : "loki-via-local"}`} title={`Answered by ${known.label}`}>
      {known.icon} {known.label}
    </span>
  );
}

/** "5h 43%" chip from the Claude CLI's rate-limit info; amber past 70%, red past 90%. */
function usageChip(usage: UsageInfo | null) {
  const w = usage?.unifiedWindows?.five_hour;
  if (!w || typeof w.utilization !== "number") return null;
  const pct = Math.round(w.utilization * 100);
  const level = pct >= 90 ? "high" : pct >= 70 ? "mid" : "low";
  const resets = w.resetsAt ? ` — resets ${new Date(w.resetsAt * 1000).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}` : "";
  return (
    <span className={`loki-usage loki-usage-${level}`} title={`Claude 5-hour usage window${resets}`}>
      {" "}
      · Claude 5h {pct}%
    </span>
  );
}

/** Same breakpoint as the CSS where the conversations sidebar becomes an overlay drawer. */
class DroppedStream extends Error {
  constructor() {
    super("The connection closed before Loki finished replying.");
  }
}

function isTouchDevice(): boolean {
  return typeof window !== "undefined" && window.matchMedia("(pointer: coarse)").matches;
}

function isNarrowScreen(): boolean {
  return typeof window !== "undefined" && window.matchMedia("(max-width: 700px)").matches;
}

function App() {
  const [loggedIn, setLoggedIn] = useState(() => !!getStoredApiToken());
  // Real URLs: / is Home, /needs, /b/<card>, /chat, /reports, /lessons, /settings (see router.ts).
  const route = parseRoute(usePath());
  const needs = useNeeds(loggedIn);
  const sessionsRef = useRef<SessionSummary[]>([]);
  // Messages typed while a reply is still running wait here and go out, in order, when it finishes.
  const [queue, setQueue] = useState<{ id: number; sid: string; text: string }[]>([]);
  const [queuePaused, setQueuePaused] = useState(false);
  const [toast, setToast] = useState<ToastData | null>(null);
  const routeRef = useRef(route);
  routeRef.current = route;
  const queueSid = getCurrentSessionId() ?? "";
  useEffect(() => {
    const names: Record<string, string> = { home: "Home", needs: "Needs you", needslist: "Needs you", chat: "Chat", improve: "Improve", maildash: "Mail", newsdash: "News", leadsdash: "Leads", mailreview: "Inbox review", reports: "Reports", lessons: "Lessons", settings: "Settings", brief: "Briefing", day: "Day" };
    document.title = route.name === "home" ? "Loki" : `Loki — ${names[route.name] ?? ""}`;
  }, [route.name]);
  const [showTroubleshoot, setShowTroubleshoot] = useState(false);
  const [commands, setCommands] = useState<SlashCommand[]>([]);
  const [slashIndex, setSlashIndex] = useState(0);
  const [slashDismissed, setSlashDismissed] = useState(false);
  const [history, setHistory] = useState<Turn[]>([]);
  // Turns before this index are hidden from view, not deleted — see transport.ts's
  // `getScreenClearMarker`/`setScreenClearMarker` doc comment. Purely a "don't overwhelm me
  // with a wall of scrollback" control, distinct from clearing the actual conversation.
  const [clearedCount, setClearedCount] = useState(0);
  const [input, setInput] = useState("");
  const [sending, setSending] = useState(false);
  const [elapsedSec, setElapsedSec] = useState(0);
  // The turn being streamed right now. `null` when idle.
  const [live, setLive] = useState<Block[] | null>(null);
  const [liveStatus, setLiveStatus] = useState<string | null>(null);
  const [usage, setUsage] = useState<UsageInfo | null>(null);
  const [ctxReport, setCtxReport] = useState<ContextReport | null>(null);
  const [showCtx, setShowCtx] = useState(false);
  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  useEffect(() => {
    sessionsRef.current = sessions;
  }, [sessions]);
  const [engineList, setEngineList] = useState<EngineList | null>(null);
  const [currentId, setCurrentId] = useState<string | null>(getCurrentSessionId());
  // Refreshed when the conversation changes and after every turn (sending flips back to false).
  useEffect(() => {
    if (!loggedIn || !currentId || sending) return;
    getContext(currentId).then(setCtxReport).catch(() => {});
  }, [loggedIn, currentId, sending, history.length]);
  const [sidebarOpen, setSidebarOpen] = useState(() => {
    // On a phone the sidebar is an overlay drawer: never start with it covering the chat.
    if (isNarrowScreen()) return false;
    try {
      const saved = localStorage.getItem("loki_sidebar_open");
      if (saved !== null) return saved === "1";
    } catch {
      // fall through to the width-based default
    }
    return window.innerWidth >= 900;
  });
  // A turn another client (phone, sidebar, another window) is running on this conversation.
  const [remoteBusy, setRemoteBusy] = useState(false);
  const abortRef = useRef<AbortController | null>(null);
  const recoverTimer = useRef<number | undefined>(undefined);
  // Mirror of the live blocks for handlers that run outside the render that created them
  // (stream callbacks, approval-button clicks).
  const liveRef = useRef<Block[]>([]);
  function setLiveBlocks(next: Block[]) {
    liveRef.current = next;
    setLive(next);
  }
  const [error, setError] = useState<string | null>(null);
  const [inRoutineMode, setInRoutineMode] = useState(false);
  const [forceLocal, setForceLocal] = useState(false);
  // #13 — a captured screenshot staged for the next outgoing message, or null if none.
  // "Staged, not sent yet" (rather than capturing-and-sending in one step) so you can see
  // what's about to go out and back out before it's actually attached to a turn.
  const [pendingScreenshot, setPendingScreenshot] = useState<string | null>(null);
  const [capturing, setCapturing] = useState(false);
  const [screenCaptureAvailable, setScreenCaptureAvailable] = useState(false);
  const bottomRef = useRef<HTMLDivElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  function reloadHistory() {
    return getHistory()
      .then(({ turns, inRoutineMode: restored, forceLocal: restoredForceLocal, busy }) => {
        // A turn that outlived our connection is still running server-side: show it as busy;
        // the mirror below reattaches and `done` reloads the finished reply.
        if (abortRef.current === null) setRemoteBusy(busy);
        setHistory(turns);
        // Clamp to `turns.length` in case the marker is stale (e.g. left over from a
        // session that got reset) — never hide a turn count larger than what actually exists.
        setClearedCount(Math.min(getScreenClearMarker(getCurrentSessionId()), turns.length));
        setInRoutineMode(restored);
        setForceLocal(restoredForceLocal);
      })
      .catch(() => {});
  }

  function refreshEngines() {
    listEngines().then(setEngineList).catch(() => {});
  }

  function chooseEngine(engine: string, scope: "session" | "default") {
    setEngine(engine, scope)
      .then(() => {
        setCurrentId(getCurrentSessionId());
        refreshEngines();
      })
      .catch((e) => setError(String(e)));
  }

  function promptRename(sess: SessionSummary) {
    const next = window.prompt("Rename conversation", sess.title);
    if (next === null || !next.trim() || next.trim() === sess.title) return;
    renameSession(sess.session_id, next)
      .then(refreshSessions)
      .catch((e) => window.alert(`Couldn't rename: ${e instanceof Error ? e.message : e}`));
  }

  function refreshSessions() {
    listSessions().then(setSessions).catch(() => {});
  }

  useEffect(() => {
    reloadHistory();
    refreshSessions();
    refreshEngines();
    listCommands().then(setCommands).catch(() => {});
    // A tapped notification opens /?session=<id>: jump straight to that conversation.
    try {
      const target = new URLSearchParams(window.location.search).get("session");
      if (target) navigate(`/chat/${encodeURIComponent(target)}`, { replace: true });
    } catch {
      // not a browser context
    }
    canCaptureScreen().then(setScreenCaptureAvailable);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  function toggleSidebar() {
    setSidebarOpen((open) => {
      try {
        localStorage.setItem("loki_sidebar_open", open ? "0" : "1");
      } catch {
        // Best-effort persistence.
      }
      return !open;
    });
  }

  /** On a phone the drawer should get out of the way once you've chosen (not persisted). */
  function closeDrawerIfNarrow() {
    if (isNarrowScreen()) setSidebarOpen(false);
  }

  function openSession(id: string) {
    if (sending) return;
    closeDrawerIfNarrow();
    if (id === currentId && route.name === "chat" && route.id === id) return;
    navigate(`/chat/${encodeURIComponent(id)}`);
  }

  // The URL decides which conversation is on screen: /chat is a fresh landing page, /chat/<id> an existing one.
  const chatId = route.name === "chat" ? route.id ?? "" : null;
  useEffect(() => {
    if (chatId === null || !loggedIn) return;
    if (chatId) {
      if (chatId !== getCurrentSessionId()) {
        switchSession(chatId);
        setCurrentId(chatId);
        setError(null);
        setHistory([]);
        reloadHistory();
        refreshEngines();
      }
    } else if (!sending && (history.length > 0 || clearedCount > 0)) {
      clearConversation();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [chatId, loggedIn]);
  // The first message from the landing page turns it into a real conversation URL.
  useEffect(() => {
    if (sending && chatId === "" && currentId) navigate(`/chat/${encodeURIComponent(currentId)}`, { replace: true });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sending]);
  // The server names a conversation with the local model a few seconds after a turn ends; pick the title up.
  useEffect(() => {
    if (sending) return;
    const t1 = window.setTimeout(refreshSessions, 6000);
    const t2 = window.setTimeout(refreshSessions, 20000);
    return () => {
      window.clearTimeout(t1);
      window.clearTimeout(t2);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sending]);

  function clearScreen() {
    setClearedCount(history.length);
    setScreenClearMarker(getCurrentSessionId(), history.length);
  }

  function showHiddenTurns() {
    setClearedCount(0);
    setScreenClearMarker(getCurrentSessionId(), 0);
  }

  // `/clear` — a real conversation reset (new session_id), not the display-only "Clear
  // screen" button above. Mirrors the sidebar plugin's `/clear`: the old session's vault log
  // is left untouched, a new one starts on the next message sent.
  function clearConversation() {
    setCurrentId(newSession());
    refreshEngines();
    setHistory([]);
    setClearedCount(0);
    setError(null);
    setInRoutineMode(false);
    setForceLocal(false);
  }

  // "Ask Loki" from a card: a new conversation with the prompt filled in (never auto-sent).
  function askLoki(prompt: string) {
    clearConversation();
    setInput(prompt);
    navigate("/chat");
  }

  // A routine button: a fresh chat with the command sent for you (or queued, if a reply is still running).
  const [pendingRun, setPendingRun] = useState<string | null>(null);
  function runLoki(command: string) {
    if (sending || remoteBusy) {
      navigate("/chat");
      void send(command);
      return;
    }
    clearConversation();
    navigate("/chat");
    setPendingRun(command);
  }
  useEffect(() => {
    if (pendingRun && route.name === "chat" && !sending) {
      const c = pendingRun;
      setPendingRun(null);
      void send(c);
    }
  }); // eslint-disable-line react-hooks/exhaustive-deps

  // "/" suggestions: shown while the first word of the input is being typed.
  const slashQuery = !slashDismissed && /^\/[a-z0-9_-]*$/i.test(input) ? input.slice(1).toLowerCase() : null;
  const slashMatches =
    slashQuery === null
      ? []
      : [
          ...commands.filter((c) => c.name.startsWith(slashQuery)),
          ...commands.filter((c) => !c.name.startsWith(slashQuery) && (c.name.includes(slashQuery) || c.description.toLowerCase().includes(slashQuery))),
        ].slice(0, 8);

  function pickSlash(c: SlashCommand) {
    setInput(`/${c.name} `);
    setSlashIndex(0);
    textareaRef.current?.focus();
  }

  const visibleHistory = history.slice(clearedCount);

  function patchTurn(globalIndex: number, patch: Partial<Turn>) {
    setHistory((h) => h.map((t, i) => (i === globalIndex ? { ...t, ...patch } : t)));
  }

  /** Regenerate: ask the same question again (optionally on another model). The earlier answer
   * stays in the conversation — the transcript is append-only. */
  async function redo(globalIndex: number, engineId: string | null) {
    const q = [...history.slice(0, globalIndex)].reverse().find((t) => t.role === "user");
    if (!q || sending || remoteBusy) return;
    if (engineId) {
      try {
        await setEngine(engineId, "session");
        refreshEngines();
      } catch (e) {
        setError(String(e));
        return;
      }
    }
    send(q.content.replace(/\n\n📎 screenshot attached$/, ""));
  }

  async function captureForNextMessage() {
    setCapturing(true);
    setError(null);
    try {
      const png = await captureScreenshot();
      if (!png) {
        setError("Couldn't capture the screen — grim may have failed. Check loki-server's logs.");
        return;
      }
      setPendingScreenshot(png);
    } finally {
      setCapturing(false);
    }
  }

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [history, live]);

  // Auto-grow the textarea to fit its content (up to a CSS-enforced max-height, past which
  // it scrolls internally) — routine-mode replies can genuinely run to several lines (EOD's
  // reflection alone asks 8 questions), and a fixed single-row box makes that miserable to
  // review before sending. Reset to "auto" first so shrinking (e.g. after clearing input)
  // isn't stuck at the tallest height it ever reached.
  useEffect(() => {
    const el = textareaRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${el.scrollHeight}px`;
  }, [input]);

  // A visible, ticking timer while waiting — without this, a genuinely-still-working
  // 60-90s response (this model explores the vault more than Claude would before
  // answering) looks indistinguishable from a hang.
  useEffect(() => {
    if (!sending && !remoteBusy) {
      setElapsedSec(0);
      return;
    }
    const start = Date.now();
    const id = setInterval(() => setElapsedSec(Math.floor((Date.now() - start) / 1000)), 1000);
    return () => clearInterval(id);
  }, [sending, remoteBusy]);

  /** Folds one stream event into the live blocks. Returns the new blocks (immutably). */
  function applyEvent(blocks: Block[], ev: StreamEvent): Block[] {
    switch (ev.type) {
      case "text_delta": {
        const last = blocks[blocks.length - 1];
        if (last && last.kind === "text") return [...blocks.slice(0, -1), { kind: "text", text: last.text + ev.text }];
        return [...blocks, { kind: "text", text: ev.text }];
      }
      case "reset":
        return [];
      case "resolved":
        // Answered on another device: stop asking here.
        return blocks.map((b) =>
          b.kind === "permission" && b.id === ev.id && b.status === "pending" ? { ...b, status: "resolved" as const } : b,
        );
      case "tool_start":
        return [...blocks, { kind: "tool", id: ev.id, name: ev.name, input: ev.input }];
      case "permission_request":
        return [
          ...blocks,
          { kind: "permission", id: ev.id, tool: ev.tool, input: ev.input, status: "pending" as const },
        ];
      case "tool_result":
        return blocks.map((b) =>
          b.kind === "tool" && b.id === ev.id ? { ...b, result: ev.content, isError: ev.is_error } : b,
        );
      default:
        return blocks;
    }
  }

  /** The user's answer to a pending approval card. */
  function answerPermission(id: string, allow: boolean, always: boolean) {
    const block = liveRef.current.find((b) => b.kind === "permission" && b.id === id);
    if (!block || block.kind !== "permission") return;
    if (allow && always) addPermissionRule(block.tool, block.input).catch((e) => setError(String(e)));
    setLiveBlocks(
      liveRef.current.map((b) =>
        b.kind === "permission" && b.id === id ? { ...b, status: allow ? ("allowed" as const) : ("denied" as const) } : b,
      ),
    );
    respondBridge(id, { allow }).catch((e) => setError(String(e)));
  }

  /** Events common to our own turn and a turn another client is running here. */
  function handleLiveEvent(ev: StreamEvent) {
    const blocks = liveRef.current;
    switch (ev.type) {
      case "status":
        setLiveStatus(ev.message);
        return;
      case "usage":
        setUsage(ev.info);
        return;
      case "device_request":
        // Run it on this machine (desktop app) and hand the result back to the server. Only
        // clients that declared `device_fs` are ever sent one.
        runDeviceTool(ev.name, ev.input)
          .then((r) => respondBridge(ev.id, r))
          .catch((e) => setError(String(e)));
        return;
      case "permission_auto":
        // Answered on the server by a saved always-allow rule; show it happened.
        setLiveBlocks([
          ...blocks,
          { kind: "permission", id: `auto-${blocks.length}`, tool: ev.tool, input: ev.input, status: "allowed", auto: true },
        ]);
        return;
      default: {
        const next = applyEvent(blocks, ev);
        if (next !== blocks) setLiveBlocks(next);
      }
    }
  }

  // Mirror: while we aren't sending, stay attached to this conversation so a turn started from
  // another client shows up live here, and its approval prompts can be answered from here.
  useEffect(() => {
    if (!loggedIn || !currentId || sending) return;
    const ctl = new AbortController();
    const me = getClientId();
    watchSession(
      currentId,
      (ev) => {
        if (ev.type === "hello" || ev.type === "started") return;
        if (ev.type === "user_message") {
          if (ev.client_id === me) return;
          setHistory((h) => [...h, { role: "user", content: ev.text }]);
          setLiveBlocks([]);
          setRemoteBusy(true);
        } else if (ev.type === "done" || ev.type === "error") {
          setRemoteBusy(false);
          setLive(null);
          liveRef.current = [];
          setLiveStatus(null);
          if (ev.type === "error") setError(ev.message);
          else {
            reloadHistory();
            refreshSessions();
          }
        } else {
          // An approval replayed on reconnect means a turn is still waiting on us.
          if (ev.type === "permission_request") setRemoteBusy(true);
          if (liveRef.current.length === 0 && live === null) setLiveBlocks([]);
          handleLiveEvent(ev);
        }
      },
      ctl.signal,
    ).catch(() => {});
    return () => ctl.abort();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loggedIn, currentId, sending]);

  // Coming back to the page (or the network returning): resync from the server, since the
  // phone may have suspended us mid-turn.
  useEffect(() => {
    const resync = () => {
      if (document.visibilityState === "hidden" || abortRef.current !== null) return;
      reloadHistory();
      refreshSessions();
    };
    document.addEventListener("visibilitychange", resync);
    window.addEventListener("online", resync);
    window.addEventListener("pageshow", resync);
    return () => {
      document.removeEventListener("visibilitychange", resync);
      window.removeEventListener("online", resync);
      window.removeEventListener("pageshow", resync);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  function stop() {
    if (queue.length > 0) setQueuePaused(true);
    abortRef.current?.abort();
    cancelTurn();
    if (!abortRef.current) {
      // A turn another device started: no stream of ours to close, so reset the view ourselves.
      setRemoteBusy(false);
      setLive(null);
      liveRef.current = [];
      setLiveStatus(null);
      window.setTimeout(reloadHistory, 600);
    }
  }

  // Esc stops a running reply (the textarea is disabled while sending, so listen on window).
  useEffect(() => {
    if (!sending && !remoteBusy) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") stop();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [sending, remoteBusy]);

  // Send the next queued message once the current reply is done (only for the conversation it was typed in).
  useEffect(() => {
    if (sending || remoteBusy || queuePaused) return;
    const sid = getCurrentSessionId() ?? "";
    const next = queue.find((q) => q.sid === sid);
    if (!next) return;
    setQueue((q) => q.filter((x) => x.id !== next.id));
    void send(next.text);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sending, remoteBusy, queue, queuePaused]);

  // The ✓ in the tab title clears when you come back.
  useEffect(() => {
    const clear = () => {
      if (!document.hidden) document.title = document.title.replace(/^✓ /, "");
    };
    document.addEventListener("visibilitychange", clear);
    window.addEventListener("focus", clear);
    return () => {
      document.removeEventListener("visibilitychange", clear);
      window.removeEventListener("focus", clear);
    };
  }, []);

  async function send(override?: string) {
    const message = (override ?? input).trim();
    if (!message) return;
    if (sending || remoteBusy) {
      // A reply is still running: queue this one instead of dropping it.
      setQueue((q) => [...q, { id: Date.now() + Math.random(), sid: getCurrentSessionId() ?? "", text: message }]);
      if (override === undefined) setInput("");
      return;
    }
    if (message.toLowerCase() === "/clear") {
      setInput("");
      clearConversation();
      return;
    }
    const images = pendingScreenshot ? [pendingScreenshot] : [];
    if (override === undefined) setInput("");
    setPendingScreenshot(null);
    setError(null);
    setHistory((h) => [...h, { role: "user", content: images.length ? `${message}\n\n📎 screenshot attached` : message, at: nowStamp() }]);
    setSending(true);
    setLiveBlocks([]);
    setLiveStatus(null);
    const controller = new AbortController();
    abortRef.current = controller;
    let blocks: Block[] = [];
    let finished = false;
    try {
      await streamMessage(
        message,
        images,
        (ev) => {
          blocks = liveRef.current; // approvals answered by click update this ref between events
          if (ev.type === "error") throw new Error(ev.message);
          else if (ev.type === "user_message") return; // our own echo
          else if (ev.type === "done") {
            finished = true;
            const hasText = blocks.some((b) => b.kind === "text" && b.text.trim());
            const final = hasText ? blocks : [...blocks, { kind: "text" as const, text: ev.reply }];
            setHistory((h) => [
              ...h,
              { role: "assistant", content: ev.reply, via: ev.via, note: ev.note ?? undefined, blocks: final, at: nowStamp() },
            ]);
            setInRoutineMode(ev.in_routine_mode);
            setForceLocal(ev.force_local);
            setCurrentId(ev.session_id);
            refreshSessions();
          } else {
            handleLiveEvent(ev);
          }
        },
        controller.signal,
      );
      if (!finished && !controller.signal.aborted) throw new DroppedStream();
    } catch (e) {
      if (controller.signal.aborted) {
        // Stop pressed: keep what streamed so far, marked as interrupted.
        blocks = liveRef.current;
        const text = blocks.filter((b) => b.kind === "text").map((b) => (b as { text: string }).text).join("\n\n");
        setHistory((h) => [...h, { role: "assistant", content: text, blocks, stopped: true, at: nowStamp() }]);
      } else if (e instanceof DroppedStream || e instanceof TypeError) {
        // The connection dropped (phone left the page, network blip) — not a failed turn. It
        // keeps running on the server; pick it back up from there.
        recoverTimer.current = window.setTimeout(reloadHistory, 800);
      } else {
        setError(String(e instanceof Error ? e.message : e));
      }
    } finally {
      setSending(false);
      setLive(null);
      liveRef.current = [];
      setLiveStatus(null);
      const wasStopped = abortRef.current?.signal.aborted ?? false;
      abortRef.current = null;
      if (!wasStopped) announceDone(finished);
    }
  }

  /** "Loki finished" while you're elsewhere in the app, in another tab, or on another window. */
  function announceDone(ok: boolean) {
    const prefs = getAlertPrefs();
    const looking = routeRef.current.name === "chat" && !document.hidden && document.hasFocus();
    if (looking) return;
    const sid = getCurrentSessionId() ?? "";
    const title = sessionsRef.current.find((s) => s.session_id === sid)?.title ?? "your conversation";
    const open = () => navigate(sid ? `/chat/${encodeURIComponent(sid)}` : "/chat");
    if (prefs.toast) setToast({ id: Date.now(), kind: ok ? "ok" : "error", text: ok ? "Loki finished" : "Loki stopped with a problem", detail: title, action: { label: "View", run: open } });
    if (prefs.sound) playChime();
    if (prefs.desktop && (document.hidden || !document.hasFocus())) desktopNotify(ok ? "Loki finished" : "Loki hit a problem", title, open);
    if (document.hidden) document.title = `✓ ${document.title.replace(/^✓ /, "")}`;
  }

  if (!loggedIn) {
    return <Login onSuccess={() => setLoggedIn(true)} />;
  }

  return (
    <div className="loki-app">
      {route.name === "chat" && sidebarOpen && <div className="loki-sidebar-backdrop" onClick={closeDrawerIfNarrow} aria-hidden="true" />}
      {route.name === "chat" && sidebarOpen && (
        <aside className="loki-sidebar">
          <button type="button" className="loki-new-chat" onClick={() => {
              if (sending) return;
              clearConversation();
              closeDrawerIfNarrow();
            }}
            disabled={sending}
          >
            + New chat
          </button>
          <div className="loki-session-list">
            {sessions.length === 0 && <div className="loki-session-empty">No saved conversations yet.</div>}
            {sessions.map((sess) => (
              <div key={sess.session_id} className="loki-session-row">
                <button
                  type="button"
                  className={`loki-session ${sess.session_id === currentId ? "loki-session-active" : ""}`}
                  onClick={() => openSession(sess.session_id)}
                  disabled={sending}
                  title={sess.title}
                >
                  <span className="loki-session-title">{sess.title}</span>
                  <span className="loki-session-meta">{sess.updated ? relativeTime(sess.updated) : ""}</span>
                </button>
                {sess.turns > 0 && (
                  <button type="button" className="loki-session-rename" onClick={() => promptRename(sess)} title="Rename" aria-label={`Rename ${sess.title}`}>
                    ✎
                  </button>
                )}
              </div>
            ))}
          </div>
        </aside>
      )}
    <RunContext.Provider value={runLoki}>
    <RefreshContext.Provider value={needs.refresh}>
    <main className={`loki-shell ${route.name === "home" ? "loki-shell-home" : ""}`}>
      <Toast toast={toast} onClose={() => setToast(null)} />
      {route.name !== "home" && <TopNav route={route} data={needs.data} onTroubleshoot={() => setShowTroubleshoot(true)} />}
      {showTroubleshoot && <Troubleshoot onClose={() => setShowTroubleshoot(false)} />}
      {showCtx && ctxReport && currentId && (
        <ContextPanel sessionId={currentId} report={ctxReport} onClose={() => setShowCtx(false)} onChanged={setCtxReport} onCompact={() => void send("/compact")} />
      )}
      {route.name === "home" && <HomePage data={needs.data} error={needs.error} onAsk={askLoki} />}
      {route.name === "day" && <DayPage date={route.date} today={needs.data?.date ?? ""} onAsk={askLoki} />}
      {route.name === "needs" && <NeedsPage data={needs.data} error={needs.error} refresh={needs.refresh} onAsk={askLoki} />}
      {route.name === "needslist" && <NeedsYouPage onAsk={askLoki} />}
      {route.name === "brief" && <BriefingPage slug={route.slug} data={needs.data} onAsk={askLoki} />}
      {route.name === "mailreview" && <MailReviewPage onAsk={askLoki} />}
      {route.name === "maildash" && <MailDashboard />}
      {route.name === "newsdash" && <NewsDashboard data={needs.data} onAsk={askLoki} />}
      {route.name === "leadsdash" && <LeadsDashboard onAsk={askLoki} />}
      {route.name === "improve" && <div className="loki-improve"><Lessons /><Reports /></div>}
      {route.name === "reports" && <Reports />}
      {route.name === "lessons" && <Lessons />}
      {route.name === "settings" && <Settings />}
      {route.name === "chat" && (
        <>
      <div className="loki-chat-toolbar">
        <button type="button" className="loki-sidebar-toggle" onClick={toggleSidebar} title="Conversations" aria-label="Toggle conversations">
          ☰ Conversations
        </button>
        <span className="loki-subtitle">
          {engineList?.summary ?? "Auto"}
          {usageChip(usage)}
          <ContextChip report={ctxReport} onClick={() => setShowCtx(true)} />
        </span>
      </div>
      {forceLocal ? (
        <div className="loki-routine-banner loki-local-banner">
          Local-only mode — nothing is being sent to Claude Code. Type <code>/exit</code> to go back to Claude.
        </div>
      ) : (
        inRoutineMode && (
          <div className="loki-routine-banner">
            Talking to Claude Code by default. A pre-turn check keeps anything sensitive local automatically —
            type <code>/exit</code> to force local-only for this conversation.
          </div>
        )
      )}
      {(visibleHistory.length > 0 || clearedCount > 0) && (
        <div className="loki-screen-controls">
          {clearedCount > 0 && (
            <button type="button" className="loki-show-hidden" onClick={showHiddenTurns}>
              Show {clearedCount} earlier message{clearedCount === 1 ? "" : "s"}
            </button>
          )}
          {visibleHistory.length > 0 && (
            <button type="button" className="loki-clear-screen" onClick={clearScreen} title="Hides messages from view — doesn't delete the conversation">
              Clear screen
            </button>
          )}
        </div>
      )}
      <div className="loki-messages">
        {visibleHistory.length === 0 && clearedCount === 0 && !live && (
          <ChatLanding sessions={sessions} onSuggest={(t) => setInput(t)} />
        )}
        {visibleHistory.length === 0 && clearedCount > 0 && (
          <div className="loki-empty">Screen cleared — the conversation is still there, just hidden.</div>
        )}
        {visibleHistory.map((turn, i) => (
          <div key={clearedCount + i} className={`loki-turn loki-turn-${turn.role}`}>
            <div className="loki-turn-role">
              {turn.role === "user" ? "You" : "Loki"}
              {turn.role === "assistant" && viaBadge(turn.via)}
              {turn.stopped && <span className="loki-stopped-tag">stopped</span>}
              {turn.at && (
                <time className="loki-turn-time" title={turn.at}>
                  {formatStamp(turn.at)}
                </time>
              )}
            </div>
            {turn.note && <div className="loki-turn-note">{turn.note}</div>}
            {turn.blocks ? <BlockList blocks={turn.blocks} /> : <Markdown text={turn.content} />}
            {turn.role === "assistant" && turn.content && (
              <TurnActions
                turn={turn}
                index={clearedCount + i}
                isLast={clearedCount + i === history.length - 1}
                busy={sending || remoteBusy}
                engineList={engineList}
                onChange={(patch) => patchTurn(clearedCount + i, patch)}
                onRedo={(eng) => redo(clearedCount + i, eng)}
              />
            )}
          </div>
        ))}
        {(sending || remoteBusy) && (
          <div className="loki-turn loki-turn-assistant loki-live">
            <div className="loki-turn-role">Loki</div>
            {liveStatus && <div className="loki-turn-note">{liveStatus}</div>}
            {live && live.length > 0 && <BlockList blocks={live} onAnswer={answerPermission} />}
            <div className="loki-thinking">
              <span className="loki-spinner" /> {sending ? `working… ${elapsedSec}s` : `working on another device… ${elapsedSec}s`}
              {!sending && (
                <button type="button" className="loki-stop loki-stop-inline" onClick={stop} title="Stop this reply">
                  ■ Stop
                </button>
              )}
            </div>
          </div>
        )}
        {error && <div className="loki-error">Error: {error}</div>}
        <div ref={bottomRef} />
      </div>

      {pendingScreenshot && (
        <div className="loki-screenshot-pending">
          📎 Screenshot attached — will send with your next message.{" "}
          <button type="button" onClick={() => setPendingScreenshot(null)}>
            Remove
          </button>
        </div>
      )}

      {engineList && (
        <div className="loki-engine-row">
          <label>
            Model{" "}
            <select
              value={engineList.session ?? engineList.default}
              onChange={(e) => chooseEngine(e.currentTarget.value, "session")}
              disabled={sending}
              title="Which model answers this conversation. Auto lets Loki choose and fail over."
            >
              {engineList.engines.map((en) => (
                <option key={en.id} value={en.id}>
                  {en.cloud ? "☁ " : "🔒 "}
                  {en.label}
                </option>
              ))}
            </select>
          </label>
          {(engineList.session ?? engineList.default) !== engineList.default && (
            <button type="button" onClick={() => chooseEngine(engineList.session as string, "default")} disabled={sending}>
              Make default
            </button>
          )}
          {engineList.session && (
            <button type="button" onClick={() => chooseEngine("auto", "session")} disabled={sending}>
              Back to default
            </button>
          )}
        </div>
      )}

      {slashMatches.length > 0 && (
        <div className="loki-slash" role="listbox" aria-label="Commands">
          {slashMatches.map((c, i) => (
            <button
              type="button"
              key={c.name}
              role="option"
              aria-selected={i === slashIndex}
              className={i === slashIndex ? "loki-slash-active" : ""}
              onMouseDown={(e) => e.preventDefault()}
              onClick={() => pickSlash(c)}
            >
              <span className="loki-slash-name">/{c.name}</span>
              <span className="loki-slash-desc">{c.description}</span>
            </button>
          ))}
        </div>
      )}

      {(queue.some((q) => q.sid === queueSid) || queuePaused) && (
        <div className="loki-queue" aria-label="Queued messages">
          <span className={queuePaused ? "loki-queue-paused" : ""}>{queuePaused ? "Queue paused:" : "Queued next:"}</span>
          {queue.filter((q) => q.sid === queueSid).map((q) => (
            <span className="loki-queue-item" key={q.id} title={q.text}>
              <span>{q.text}</span>
              <button type="button" onClick={() => setQueue((all) => all.filter((x) => x.id !== q.id))} aria-label="Remove from the queue">✕</button>
            </span>
          ))}
          {queuePaused && <button type="button" onClick={() => setQueuePaused(false)}>Resume</button>}
          {queue.some((q) => q.sid === queueSid) && <button type="button" onClick={() => { setQueue((all) => all.filter((x) => x.sid !== queueSid)); setQueuePaused(false); }}>Clear</button>}
        </div>
      )}

      <form
        className="loki-input-row"
        onSubmit={(e) => {
          e.preventDefault();
          send();
        }}
      >
        {screenCaptureAvailable && (
          <button
            type="button"
            title="Attach a screenshot of your screen to the next message"
            disabled={capturing || sending}
            onClick={captureForNextMessage}
          >
            {capturing ? "Capturing…" : "📷"}
          </button>
        )}
        <Mic
          disabled={sending || remoteBusy}
          onError={(m) => setError(m)}
          onText={(t) => {
            setInput((cur) => (cur.trim() ? `${cur.trimEnd()} ${t}` : t));
            textareaRef.current?.focus();
          }}
        />
        <textarea
          ref={textareaRef}
          autoFocus
          rows={1}
          value={input}
          onChange={(e) => {
            setInput(e.currentTarget.value);
            setSlashIndex(0);
            setSlashDismissed(false);
          }}
          onKeyDown={(e) => {
            if (slashMatches.length > 0) {
              if (e.key === "ArrowDown" || e.key === "ArrowUp") {
                e.preventDefault();
                const n = slashMatches.length;
                setSlashIndex((i) => (e.key === "ArrowDown" ? (i + 1) % n : (i - 1 + n) % n));
                return;
              }
              const chosen = slashMatches[Math.min(slashIndex, slashMatches.length - 1)];
              const typedExactly = slashQuery === chosen.name;
              if (e.key === "Tab" || (e.key === "Enter" && !e.shiftKey && !typedExactly)) {
                e.preventDefault();
                pickSlash(chosen);
                return;
              }
              if (e.key === "Escape") {
                e.preventDefault();
                setSlashDismissed(true);
                return;
              }
            }
            // Enter sends; Shift+Enter inserts a newline (the browser does that on its own —
            // only intercept the plain-Enter case, don't preventDefault the shifted one).
            // On a phone (coarse pointer) Enter is a newline and the Send button is the only way to
            // send; there's no Shift key to lean on.
            if (e.key === "Enter" && !e.shiftKey && !isTouchDevice()) {
              e.preventDefault();
              send();
            }
          }}
          placeholder={
            sending || remoteBusy
              ? "Loki is working — type here to queue your next message…"
              : forceLocal
              ? `Local-only — type /exit to go back to Claude, or /clear for a new conversation...${isTouchDevice() ? "" : " (Shift+Enter for a new line)"}`
              : isTouchDevice()
                ? "Message Loki… (type / for commands)"
                : "Message Loki, or type /exit to force local-only, /clear to start over... (Shift+Enter for a new line)"
          }
        />
        {sending && (
          <button type="submit" className="loki-queue-btn" disabled={!input.trim()} title="Send this after the current reply finishes">
            Queue
          </button>
        )}
        {sending ? (
          <button type="button" className="loki-stop" onClick={stop} title="Stop this reply (Esc)">
            ■ Stop
          </button>
        ) : (
          <button type="submit" disabled={!input.trim()}>
            {remoteBusy ? "Queue" : "Send"}
          </button>
        )}
      </form>
        </>
      )}
    </main>
    </RefreshContext.Provider>
    </RunContext.Provider>
    <NotesDock />
    </div>
  );
}

/** "5m ago" / "3h ago" / "2d ago" for the session list. */
function relativeTime(unixSeconds: number): string {
  const secs = Math.max(0, Math.floor(Date.now() / 1000) - unixSeconds);
  if (secs < 90) return "just now";
  if (secs < 3600) return `${Math.round(secs / 60)}m ago`;
  if (secs < 86400) return `${Math.round(secs / 3600)}h ago`;
  return `${Math.round(secs / 86400)}d ago`;
}

export default App;
