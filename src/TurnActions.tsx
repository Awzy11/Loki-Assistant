import { useRef, useState } from "react";
import { Markdown } from "./Blocks";
import ExtractPanel from "./ExtractPanel";
import { auditTurn, rateTurn, synthesizeSpeech, type EngineList, type Turn } from "./transport";

/** Copy / thumbs / redo / audit row under an assistant message, plus the audit result panel. */
export default function TurnActions({
  turn,
  index,
  isLast,
  busy,
  engineList,
  onChange,
  onRedo,
}: {
  turn: Turn;
  index: number;
  isLast: boolean;
  busy: boolean;
  engineList: EngineList | null;
  onChange: (patch: Partial<Turn>) => void;
  onRedo: (engineId: string | null) => void;
}) {
  const [copied, setCopied] = useState(false);
  const [auditing, setAuditing] = useState(false);
  const [auditError, setAuditError] = useState<string | null>(null);
  const [open, setOpen] = useState(false);
  const [speech, setSpeech] = useState<"idle" | "loading" | "playing">("idle");
  const [extracting, setExtracting] = useState(false);
  const audio = useRef<HTMLAudioElement | null>(null);
  const speechRun = useRef(0);

  /** Read the reply aloud (Kokoro on the model server). Press again to stop. */
  async function toggleSpeech() {
    const run = ++speechRun.current;
    if (speech !== "idle") {
      audio.current?.pause();
      setSpeech("idle");
      return;
    }
    setSpeech("loading");
    setAuditError(null);
    try {
      const blob = await synthesizeSpeech(turn.content);
      if (run !== speechRun.current) return; // stopped while it was being generated
      const url = URL.createObjectURL(blob);
      const a = new Audio(url);
      audio.current = a;
      a.onended = () => {
        setSpeech("idle");
        URL.revokeObjectURL(url);
      };
      a.onerror = () => setSpeech("idle");
      await a.play();
      setSpeech("playing");
    } catch (e) {
      setSpeech("idle");
      setAuditError(String(e instanceof Error ? e.message : e));
    }
  }

  async function copy() {
    try {
      await navigator.clipboard.writeText(turn.content);
    } catch {
      const ta = document.createElement("textarea");
      ta.value = turn.content;
      document.body.appendChild(ta);
      ta.select();
      document.execCommand("copy");
      ta.remove();
    }
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1500);
  }

  function rate(r: "up" | "down") {
    const next = turn.feedback === r ? "none" : r;
    onChange({ feedback: next === "none" ? undefined : next });
    rateTurn(index, next).catch((e) => setAuditError(String(e)));
    // A downvote may start an automatic audit on the server; look for it shortly.
    if (next === "down") window.setTimeout(() => setAuditError(null), 0);
  }

  async function audit() {
    setAuditing(true);
    setAuditError(null);
    try {
      const a = await auditTurn(index);
      onChange({ audit: a });
      setOpen(true);
    } catch (e) {
      setAuditError(String(e instanceof Error ? e.message : e));
    } finally {
      setAuditing(false);
    }
  }

  const a = turn.audit;
  return (
    <>
      <div className="loki-actions">
        <button type="button" onClick={copy} title="Copy response" aria-label="Copy response">
          {copied ? "✓" : "⧉"}
        </button>
        <button type="button" className={speech !== "idle" ? "loki-act-on" : ""} onClick={toggleSpeech} title={speech === "idle" ? "Read this aloud" : "Stop reading"} aria-label="Read aloud">
          {speech === "loading" ? "…" : speech === "playing" ? "■" : "🔊"}
        </button>
        <button type="button" className={extracting ? "loki-act-on" : ""} onClick={() => setExtracting((x) => !x)} title="Pull action items and events out of this reply" aria-label="Extract action items and events">
          ⤓
        </button>
        <button type="button" className={turn.feedback === "up" ? "loki-act-on" : ""} onClick={() => rate("up")} title="Good response" aria-label="Good response">
          👍
        </button>
        <button type="button" className={turn.feedback === "down" ? "loki-act-on loki-act-down" : ""} onClick={() => rate("down")} title="Bad response" aria-label="Bad response">
          👎
        </button>
        {isLast && (
          <>
            <button type="button" onClick={() => onRedo(null)} disabled={busy} title="Redo this response" aria-label="Redo response">
              ↻
            </button>
            {engineList && (
              <select
                className="loki-redo-with"
                value=""
                disabled={busy}
                onChange={(e) => e.currentTarget.value && onRedo(e.currentTarget.value)}
                title="Redo with a different model (switches this conversation to it)"
              >
                <option value="">Redo with…</option>
                {engineList.engines.map((en) => (
                  <option key={en.id} value={en.id}>
                    {en.cloud ? "☁ " : "🔒 "}
                    {en.label}
                  </option>
                ))}
              </select>
            )}
          </>
        )}
        <button
          type="button"
          className={a ? `loki-audit-chip loki-audit-${a.verdict}` : "loki-audit-btn"}
          onClick={() => (a ? setOpen((o) => !o) : audit())}
          disabled={auditing}
          title={a ? "Claude's audit of this response" : "Have Claude audit this response"}
        >
          {auditing ? "Auditing…" : a ? (a.verdict === "good" ? "✅ Audited" : a.verdict === "minor" ? "⚠️ Audited" : "❌ Audited") : "🔎 Audit"}
        </button>
      </div>
      {auditError && <div className="loki-audit-error">{auditError}</div>}
      {a && open && (
        <div className="loki-audit-panel">
          <div className="loki-audit-summary">{a.summary}</div>
          {a.findings.length > 0 && (
            <ul>
              {a.findings.map((f, i) => (
                <li key={i}>
                  <b>{f.kind}</b> <span className={`loki-sev loki-sev-${f.severity}`}>{f.severity}</span> — {f.detail}
                </li>
              ))}
            </ul>
          )}
          {a.better_answer && (
            <details>
              <summary>Claude's suggested answer</summary>
              <Markdown text={a.better_answer} />
            </details>
          )}
          {a.suggested_lessons.length > 0 && (
            <div className="loki-audit-lessons">
              Suggested lesson{a.suggested_lessons.length > 1 ? "s" : ""} (waiting for your approval in the Lessons tab): {a.suggested_lessons.join(" · ")}
            </div>
          )}
          <button type="button" onClick={audit} disabled={auditing}>
            Re-audit
          </button>
        </div>
      )}
      {extracting && <ExtractPanel source={{ source: "text", label: "a Loki reply", text: turn.content }} />}
    </>
  );
}
