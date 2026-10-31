import { useEffect, useState } from "react";
import { getContext, setContextSettings, type ContextReport } from "./transport";

const SIZES = [32768, 65536, 98304, 131072, 196608, 262144];

function k(n: number) {
  return n >= 10000 ? `${Math.round(n / 1000)}k` : n >= 1000 ? `${(n / 1000).toFixed(1)}k` : String(n);
}

/** Header chip: "ctx 12%" (how full the window is) that turns amber/red as autocompact nears. */
export function ContextChip({ report, onClick }: { report: ContextReport | null; onClick: () => void }) {
  if (!report) return null;
  const pct = Math.round((report.used * 100) / Math.max(1, report.window));
  const near = report.autocompact.percent;
  const level = near >= 90 ? "high" : near >= 70 ? "mid" : "low";
  return (
    <button type="button" className={`loki-ctx-chip loki-usage-${level}`} onClick={onClick} title="Context window — click for details and controls">
      ctx {pct}%{near >= 70 ? ` · compact ${near}%` : ""}
    </button>
  );
}

/** What fills the context window, how close autocompact is, and the controls for it. */
export default function ContextPanel({ sessionId, report, onClose, onChanged, onCompact }: {
  sessionId: string;
  report: ContextReport;
  onClose: () => void;
  onChanged: (r: ContextReport) => void;
  onCompact: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const s = report.settings;
  const parts: { label: string; value: number; cls: string }[] = [
    { label: "System prompt", value: report.breakdown.system, cls: "sys" },
    { label: "Tool definitions", value: report.breakdown.tools, cls: "tools" },
    { label: "Summary of older turns", value: report.breakdown.summary, cls: "sum" },
    { label: "Recent conversation", value: report.breakdown.history, cls: "hist" },
  ];

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  async function change(patch: Partial<ContextReport["settings"]>) {
    setBusy(true);
    setErr(null);
    try {
      await setContextSettings(patch);
      onChanged(await getContext(sessionId));
    } catch (e) {
      setErr(String(e instanceof Error ? e.message : e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="loki-ctx-overlay" onClick={onClose}>
      <div className="loki-ctx-panel" onClick={(e) => e.stopPropagation()} role="dialog" aria-label="Context">
        <div className="loki-ctx-head">
          <strong>Context</strong>
          <button type="button" onClick={onClose} aria-label="Close">✕</button>
        </div>
        <p className="loki-ctx-line">
          About <b>{k(report.used)}</b> of <b>{k(report.window)}</b> tokens in use · {k(Math.max(0, report.window - report.used))} free
        </p>
        <div className="loki-ctx-bar" title="Estimated tokens">
          {parts.map((p) => (
            <span key={p.cls} className={`loki-ctx-seg loki-ctx-${p.cls}`} style={{ width: `${(p.value * 100) / Math.max(1, report.window)}%` }} />
          ))}
        </div>
        <ul className="loki-ctx-legend">
          {parts.map((p) => (
            <li key={p.cls}>
              <span className={`loki-ctx-dot loki-ctx-${p.cls}`} /> {p.label} <b>{k(p.value)}</b>
              {p.cls === "hist" && ` (${report.turns.verbatim} turns)`}
            </li>
          ))}
          {report.last_measured && <li className="loki-ctx-muted">Last request, as the model counted it: {k(report.last_measured.promptTokens)} tokens</li>}
        </ul>

        <h4>Autocompact</h4>
        <div className="loki-ctx-bar loki-ctx-trigger" title="Progress toward the next automatic compaction">
          <span className={`loki-ctx-seg loki-usage-${report.autocompact.percent >= 90 ? "high" : report.autocompact.percent >= 70 ? "mid" : "low"}`} style={{ width: `${report.autocompact.percent}%` }} />
        </div>
        <p className="loki-ctx-line">
          {report.autocompact.enabled ? "On" : "Off"} · {report.autocompact.percent}% of the way: {k(report.autocompact.unsummarized_tokens)} of {k(report.autocompact.at_tokens)} tokens, {report.autocompact.unsummarized_turns} of {report.autocompact.at_turns} turns. {report.turns.summarized} of {report.turns.total} turns are already in the summary.
        </p>

        <div className="loki-ctx-controls">
          <label>
            <input type="checkbox" checked={s.autocompact} disabled={busy} onChange={(e) => change({ autocompact: e.target.checked })} /> Autocompact
          </label>
          <label>
            Context size
            <select value={s.num_ctx} disabled={busy} onChange={(e) => change({ num_ctx: Number(e.target.value) })}>
              {[...new Set([...SIZES, s.num_ctx])].sort((a, b) => a - b).map((n) => (
                <option key={n} value={n}>{k(n)}</option>
              ))}
            </select>
          </label>
          <label>
            History share {Math.round(s.history_fraction * 100)}%
            <input type="range" min={10} max={60} step={5} value={Math.round(s.history_fraction * 100)} disabled={busy} onChange={(e) => change({ history_fraction: Number(e.target.value) / 100 })} />
          </label>
          <label>
            Compact after
            <input type="number" min={6} max={200} value={s.max_turns} disabled={busy} onChange={(e) => change({ max_turns: Number(e.target.value) })} /> turns
          </label>
        </div>
        <p className="loki-ctx-muted">A smaller context size or a different history share applies from the next message; changing the size makes the model reload once on the server.</p>
        {err && <p className="loki-ctx-err">{err}</p>}
        <div className="loki-ctx-actions">
          <button type="button" onClick={() => { onCompact(); onClose(); }}>Compact now</button>
        </div>
      </div>
    </div>
  );
}
