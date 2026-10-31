import { createContext, useContext, useEffect, useState } from "react";
import { getRoutines, runWorkflow, type RoutineDef } from "./transport";

/** Starts a slash command (`/news`, …) in a fresh chat and sends it. Provided by App. */
export const RunContext = createContext<(command: string) => void>(() => {});

// The routines come from the profile (profile.toml → [[routines]]); fetched once and shared by every tab.
let cache: Promise<RoutineDef[]> | null = null;
export function useRoutines(): RoutineDef[] {
  const [list, setList] = useState<RoutineDef[]>([]);
  useEffect(() => {
    cache ??= getRoutines().catch(() => { cache = null; return []; });
    let alive = true;
    void cache.then((l) => alive && setList(l));
    return () => { alive = false; };
  }, []);
  return list;
}

/** Buttons that run the routines that belong on this tab: a slash command in a new chat, or an n8n workflow started in the background. */
export default function RoutineBar({ tab }: { tab: string }) {
  const run = useContext(RunContext);
  const [open, setOpen] = useState<string | null>(null);
  const [url, setUrl] = useState("");
  const [note, setNote] = useState<Record<string, string>>({});
  const all = useRoutines();
  const list = all.filter((r) => r.tabs.includes(tab));
  if (!list.length) return null;

  const start = (r: RoutineDef, arg = "") => {
    if (r.run === "workflow") {
      setNote((n) => ({ ...n, [r.id]: "Starting…" }));
      runWorkflow(r.id, false, arg).then(() => setNote((n) => ({ ...n, [r.id]: "Started. It runs in the background; check the note when it finishes." }))).catch((e) => setNote((n) => ({ ...n, [r.id]: String(e.message || e) })));
    } else {
      run(`/${r.id}${arg ? ` ${arg}` : ""}`);
    }
  };
  const tag = (r: RoutineDef) => (r.run === "workflow" ? "workflow" : `/${r.id}`);

  return (
    <div className="rt-bar" role="group" aria-label="Run in Loki">
      <span className="rt-title">Run in Loki</span>
      {list.map((r) =>
        r.input === "url" ? (
          open === r.id ? (
            <span key={r.id} className="rt-url">
              <input value={url} autoFocus placeholder={r.ask || "Paste a link"} onChange={(e) => setUrl(e.target.value)} onKeyDown={(e) => e.key === "Enter" && /^https?:\/\//i.test(url.trim()) && (start(r, url.trim()), setOpen(null), setUrl(""))} />
              <button type="button" className="loki-chipbtn" disabled={!/^https?:\/\//i.test(url.trim())} onClick={() => { start(r, url.trim()); setOpen(null); setUrl(""); }}>Run</button>
              <button type="button" className="loki-chipbtn" onClick={() => setOpen(null)}>Cancel</button>
            </span>
          ) : (
            <span key={r.id} className="rt-one">
              <button type="button" className="loki-chipbtn rt-btn" title={r.hint} onClick={() => setOpen(r.id)}>▶ {r.label} <em>{tag(r)}</em></button>
              {note[r.id] && <small className="db-dim"> {note[r.id]}</small>}
            </span>
          )
        ) : (
          <span key={r.id} className="rt-one">
            <button type="button" className="loki-chipbtn rt-btn" title={r.hint} disabled={note[r.id] === "Starting…"} onClick={() => start(r)}>▶ {r.label} <em>{tag(r)}</em></button>
            {note[r.id] && <small className="db-dim"> {note[r.id]}</small>}
          </span>
        ),
      )}
    </div>
  );
}
