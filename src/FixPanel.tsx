import { useEffect, useRef, useState } from "react";
import { Markdown } from "./Blocks";
import { parseFixOutcome, startFix, troubleshootStatus, getReport, type FixOutcome, type FixState, type TroubleshootJob } from "./transport";

type Phase = "idle" | "working" | "review" | "applying" | "applied" | "discarded" | "error";

/** "Fix with Claude" for one troubleshoot report: stage the changes, review them, then apply or
 * discard. `initial` restores where a previous visit left off (so a staged fix can be reviewed
 * later, from any device). */
export default function FixPanel({ reportId, initial }: { reportId: string; initial?: { state: FixState; outcome?: FixOutcome; last?: FixOutcome } | null }) {
  const [note, setNote] = useState("");
  const [phase, setPhase] = useState<Phase>("idle");
  const [fix, setFix] = useState<FixOutcome | null>(null);
  const alive = useRef(true);
  useEffect(() => () => void (alive.current = false), []);

  useEffect(() => {
    if (!initial) return;
    if (initial.state === "staged" && initial.outcome) {
      setFix(initial.outcome);
      setPhase("review");
    } else if (initial.state === "applying") {
      setFix(initial.outcome ?? null);
      setPhase("applying");
      void followApply();
    } else if (initial.state === "applied") {
      setFix(initial.last ?? initial.outcome ?? null);
      setPhase("applied");
    } else if (initial.state === "discarded") {
      setPhase("discarded");
    } else if (initial.state === "error") {
      setFix(initial.last ?? initial.outcome ?? { status: "error", result: "The last attempt failed." });
      setPhase("error");
    }
  }, [initial]);

  /** Watches a deploy that is already running (or was started elsewhere) until the report says how it ended. */
  async function followApply() {
    for (let i = 0; i < 400 && alive.current; i++) {
      await new Promise((r) => setTimeout(r, 3000));
      try {
        const f = (await getReport(reportId)).fix;
        if (f && f.state !== "applying") {
          setFix(f.last ?? f.outcome ?? null);
          setPhase(f.state === "applied" ? "applied" : f.state === "error" ? "error" : f.state === "staged" ? "review" : "discarded");
          return;
        }
      } catch {
        // the server restarts during a deploy; keep trying
      }
    }
  }

  async function waitJob(id: string, maxMinutes: number): Promise<TroubleshootJob | null> {
    for (let i = 0; i < maxMinutes * 20 && alive.current; i++) {
      await new Promise((r) => setTimeout(r, 3000));
      try {
        const job = await troubleshootStatus(id);
        if (job.status !== "running") return job;
      } catch {
        // a dropped connection (phone sleeping) just retries
      }
    }
    return null;
  }

  async function run(action: "fix" | "apply" | "discard") {
    setPhase(action === "apply" ? "applying" : "working");
    try {
      const jobId = await startFix(reportId, action, note);
      if (action === "apply") {
        void jobId;
        await followApply();
        return;
      }
      const job = await waitJob(jobId, 40);
      if (!job) return;
      const out = parseFixOutcome(job);
      setFix(out);
      setPhase(out.status !== "done" ? "error" : action === "fix" ? "review" : "discarded");
    } catch (e) {
      setFix({ status: "error", result: String(e instanceof Error ? e.message : e) });
      setPhase("error");
    }
  }

  return (
    <div className="loki-fix">
      <h3>Fix these issues</h3>
      {phase === "idle" && (
        <>
          <p className="loki-lessons-hint">Claude edits an isolated copy of Loki's code on your laptop and checks that it builds. Nothing goes live until you press Apply.</p>
          <textarea rows={2} value={note} onChange={(e) => setNote(e.currentTarget.value)} placeholder="Optional: which findings to fix, or anything to leave alone" />
          <button type="button" className="loki-primary" onClick={() => run("fix")}>
            Fix with Claude
          </button>
        </>
      )}
      {phase === "working" && (
        <div className="loki-thinking">
          <span className="loki-spinner" /> Claude is working on your laptop… (a few minutes)
        </div>
      )}
      {phase === "applying" && (
        <div className="loki-thinking">
          <span className="loki-spinner" /> Applying and deploying… (a server change takes about 3 minutes and restarts Loki)
        </div>
      )}
      {phase === "review" && fix && (
        <>
          <Markdown text={fix.result} />
          {fix.nothing_changed ? (
            <p className="loki-lessons-hint">Claude didn't change any files.</p>
          ) : (
            <>
              {fix.diffstat && <pre className="loki-diffstat">{fix.diffstat}</pre>}
              <ul className="loki-fix-impact">
                {fix.backend && <li>Server change — applying rebuilds Loki on the server and restarts it (~3 min).</li>}
                {fix.frontend && <li>Web app change — takes effect after you reload.</li>}
                {fix.sidebar && fix.sidebar.length > 0 && <li>Sidebar change — your laptop's sidebar reloads.</li>}
              </ul>
            </>
          )}
          <div className="loki-fix-actions">
            {!fix.nothing_changed && (
              <button type="button" className="loki-primary" onClick={() => run("apply")}>
                Apply &amp; deploy
              </button>
            )}
            <button type="button" onClick={() => run("discard")}>
              Discard
            </button>
          </div>
        </>
      )}
      {phase === "applied" && <div className="loki-fix-done">✅ {fix?.result ?? "Applied."}</div>}
      {phase === "discarded" && <div className="loki-fix-done">Discarded — nothing was changed.</div>}
      {phase === "error" && fix && (
        <>
          <div className="loki-error">{fix.result}</div>
          <button type="button" onClick={() => setPhase("idle")}>
            Try again
          </button>
        </>
      )}
    </div>
  );
}
