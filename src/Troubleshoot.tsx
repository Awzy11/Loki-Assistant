import { useEffect, useRef, useState } from "react";
import { Markdown } from "./Blocks";
import FixPanel from "./FixPanel";
import { startTroubleshoot, troubleshootStatus } from "./transport";

// Remember recent console errors so Claude can see them.
const recentErrors: string[] = [];
if (typeof window !== "undefined") {
  const push = (s: string) => {
    recentErrors.push(`${new Date().toISOString()} ${s}`.slice(0, 500));
    if (recentErrors.length > 30) recentErrors.shift();
  };
  window.addEventListener("error", (e) => push(`error: ${e.message}`));
  window.addEventListener("unhandledrejection", (e) => push(`unhandled rejection: ${String(e.reason)}`));
  const orig = console.error.bind(console);
  console.error = (...a: unknown[]) => {
    push(`console.error: ${a.map(String).join(" ")}`);
    orig(...a);
  };
}

async function collectUi() {
  const info = {
    viewport: { w: window.innerWidth, h: window.innerHeight, dpr: window.devicePixelRatio },
    userAgent: navigator.userAgent,
    installedPwa: window.matchMedia("(display-mode: standalone)").matches,
    colorScheme: window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light",
    touch: navigator.maxTouchPoints > 0,
    url: location.href,
    consoleErrors: recentErrors.slice(-15),
  };
  const rendered = (document.querySelector(".loki-messages")?.innerHTML ?? "").slice(0, 100000);
  let screenshot_b64: string | null = null;
  try {
    const { default: html2canvas } = await import("html2canvas");
    const canvas = await html2canvas(document.body, { scale: Math.min(1, 1100 / window.innerWidth), logging: false, backgroundColor: null });
    screenshot_b64 = canvas.toDataURL("image/png").split(",")[1] ?? null;
  } catch (e) {
    (info as Record<string, unknown>).screenshotError = String(e);
  }
  return { kind: "web", info, rendered, screenshot_b64 };
}

/** "Troubleshoot with Claude": sends this conversation, its recorded processes, server log and a
 * snapshot of this screen to Claude for a read-only diagnosis. */
export default function Troubleshoot({ onClose }: { onClose: () => void }) {
  const [note, setNote] = useState("");
  const [phase, setPhase] = useState<"idle" | "capturing" | "waiting" | "done" | "error">("idle");
  const [result, setResult] = useState("");
  const [reportId, setReportId] = useState("");
  const alive = useRef(true);
  useEffect(() => () => void (alive.current = false), []);

  async function go() {
    setPhase("capturing");
    try {
      const ui = await collectUi();
      const id = await startTroubleshoot(note, ui);
      setReportId(id);
      setPhase("waiting");
      for (let i = 0; i < 120 && alive.current; i++) {
        await new Promise((r) => setTimeout(r, 3000));
        let job;
        try {
          job = await troubleshootStatus(id);
        } catch {
          continue; // a dropped connection (phone sleeping) just retries
        }
        if (job.status !== "running") {
          setResult(job.result);
          setPhase(job.status === "done" ? "done" : "error");
          return;
        }
      }
    } catch (e) {
      setResult(String(e instanceof Error ? e.message : e));
      setPhase("error");
    }
  }

  return (
    <div className="loki-modal-backdrop" onClick={onClose}>
      <div className="loki-modal" onClick={(e) => e.stopPropagation()}>
        <div className="loki-modal-head">
          <h2>Troubleshoot with Claude</h2>
          <button type="button" onClick={onClose} aria-label="Close">
            ✕
          </button>
        </div>
        {phase === "idle" && (
          <>
            <p className="loki-lessons-hint">
              Claude will read this conversation, what Loki did during each turn, the recent server log, and a screenshot of this screen
              (read-only). Private/local-only turns are withheld.
            </p>
            <textarea rows={3} value={note} onChange={(e) => setNote(e.currentTarget.value)} placeholder="What looks wrong? (optional)" autoFocus />
            <button type="button" className="loki-primary" onClick={go}>
              Send to Claude
            </button>
          </>
        )}
        {(phase === "capturing" || phase === "waiting") && (
          <div className="loki-thinking">
            <span className="loki-spinner" /> {phase === "capturing" ? "Capturing this screen…" : "Claude is reading the evidence… (about a minute)"}
          </div>
        )}
        {(phase === "done" || phase === "error") && (
          <>
            {phase === "error" && <div className="loki-error">Couldn't finish:</div>}
            <Markdown text={result} />
            <button type="button" onClick={() => navigator.clipboard?.writeText(result)}>
              Copy
            </button>
            {phase === "done" && reportId && <FixPanel reportId={reportId} />}
          </>
        )}
      </div>
    </div>
  );
}
