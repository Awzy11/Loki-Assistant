import { useEffect, useState } from "react";
import { itemPrompts } from "./askPrompts";
import { decideNeedsYou, getNeedsYou, markTask, type NeedsItem, type NeedsYou } from "./transport";

const KIND_LABEL: Record<string, string> = { task: "Today", event: "Calendar", sprint: "This sprint", loki: "Loki" };

export const askPrompts = itemPrompts;

/** Everything you can do with one item: decide, rate, and hand it to Loki. */
export function ActionPanel({ item, onDone, onAsk }: { item: NeedsItem; onDone: () => void; onAsk?: (prompt: string) => void }) {
  const [reason, setReason] = useState("");
  const [busy, setBusy] = useState(false);
  const [rated, setRated] = useState<"up" | "down" | null>(null);
  const [err, setErr] = useState<string | null>(null);

  // A task that lives in a daily or sprint note can be ticked, and given a comment, in that note.
  const inNote = item.kind === "task" && /^(10 - Daily|20 - Weeks)\/[^/]+\.md$/.test(item.source_path);
  const [skipNote, setSkipNote] = useState(false);

  async function act(action: "done" | "defer" | "not_now", days?: number, lokiOnly = false) {
    setBusy(true);
    setErr(null);
    try {
      if (inNote && !lokiOnly && (action === "done" || reason.trim())) {
        try {
          await markTask(item.source_path, item.title, "", action === "done" ? true : null, reason);
        } catch (e) {
          setErr(`Your note wasn't changed: ${String(e instanceof Error ? e.message : e)}`);
          setSkipNote(true);
          setBusy(false);
          return;
        }
      }
      await decideNeedsYou(item, action, { days, reason });
      onDone();
    } catch (e) {
      setErr(String(e instanceof Error ? e.message : e));
      setBusy(false);
    }
  }
  async function rate(r: "up" | "down") {
    setRated(r);
    try {
      await decideNeedsYou(item, "rate", { rating: r, reason });
    } catch (e) {
      setErr(String(e instanceof Error ? e.message : e));
    }
  }

  return (
    <div className="loki-ny-panel">
      <p className="loki-ny-why">{item.blurb ?? item.why}</p>
      {item.blurb && <p className="loki-ny-source">{item.why}</p>}
      {item.next_action && <p className="loki-ny-next"><b>Next:</b> {item.next_action}</p>}
      {item.link && (
        <p className="loki-ny-next">
          <a href={item.link} target={item.link.startsWith("http") ? "_blank" : undefined} rel="noreferrer">{item.link.startsWith("obsidian://") ? "Open the note ↗" : "Open the link ↗"}</a>
        </p>
      )}
      <input className="loki-ny-reason" value={reason} onChange={(e) => setReason(e.target.value)} placeholder={inNote ? "Comment (optional): kept for future-you and added to the task in your note" : "Comment (optional): kept for future-you"} disabled={busy} />
      <div className="loki-ny-actions">
        <button type="button" disabled={busy} onClick={() => act("done")} title={inNote ? "Ticks it in your note too" : "Hides it here; this item isn't a checkbox in a note"}>{inNote ? "Done (ticks your note)" : "Done"}</button>
        {skipNote && <button type="button" disabled={busy} onClick={() => act("done", undefined, true)}>Mark done in Loki only</button>}
        <button type="button" disabled={busy} onClick={() => act("defer", 1)}>Tomorrow</button>
        <button type="button" disabled={busy} onClick={() => act("defer", 3)}>In 3 days</button>
        <button type="button" disabled={busy} onClick={() => act("defer", 7)}>Next week</button>
        <button type="button" disabled={busy} onClick={() => act("not_now")}>Not today</button>
        <span className="loki-ny-rate">
          <button type="button" className={rated === "up" ? "loki-ny-on" : ""} onClick={() => rate("up")} title="Useful to see">👍</button>
          <button type="button" className={rated === "down" ? "loki-ny-on" : ""} onClick={() => rate("down")} title="Shouldn't be here">👎</button>
        </span>
      </div>
      {onAsk && (
        <div className="loki-ny-actions loki-ny-ask">
          <span>Ask Loki</span>
          {askPrompts(item).map((a) => (
            <button type="button" key={a.label} onClick={() => onAsk(a.prompt)}>{a.label}</button>
          ))}
        </div>
      )}
      {err && <p className="loki-ny-err">{err}</p>}
    </div>
  );
}

function Card({ item, onDone, onAsk }: { item: NeedsItem; onDone: () => void; onAsk?: (prompt: string) => void }) {
  return (
    <article className="loki-ny-card">
      <div className="loki-ny-meta">
        <span className={`loki-ny-kind loki-ny-kind-${item.kind}`}>{KIND_LABEL[item.kind] ?? item.kind}</span>
        {item.when && <span className="loki-ny-when">{item.when}</span>}
      </div>
      <h3>{item.title}</h3>
      <ActionPanel item={item} onDone={onDone} onAsk={onAsk} />
    </article>
  );
}

/** What needs the user now — a short, ranked list from the vault and Loki's own queue. */
export const KIND_LABELS = KIND_LABEL;

export default function NeedsYouPage({ onCount, onAsk }: { onCount?: (n: number) => void; onAsk?: (prompt: string) => void }) {
  const [data, setData] = useState<NeedsYou | null>(null);
  const [error, setError] = useState<string | null>(null);

  function refresh() {
    getNeedsYou()
      .then((d) => {
        setData(d);
        onCount?.(d.items.length);
      })
      .catch((e) => setError(String(e instanceof Error ? e.message : e)));
  }
  useEffect(() => {
    refresh();
    const t = window.setInterval(refresh, 5 * 60 * 1000);
    return () => window.clearInterval(t);
  }, []);

  return (
    <div className="loki-ny">
      <div className="loki-ny-head">
        <h2>Needs you{data ? ` — ${data.weekday} ${data.date}` : ""}</h2>
        <button type="button" onClick={refresh}>Refresh</button>
      </div>
      {error && <div className="loki-settings-status">{error}</div>}
      {data?.state.note && <p className="loki-ny-note">{data.state.note}</p>}
      {data?.focus.current && (
        <p className="loki-ny-focus">Now ({data.focus.current.start}–{data.focus.current.end}): {data.focus.current.what}</p>
      )}
      {data?.focus.next && !data.focus.current && (
        <p className="loki-ny-focus">Next block ({data.focus.next.start}–{data.focus.next.end}): {data.focus.next.what}</p>
      )}
      {data && data.items.length === 0 && <div className="loki-settings-status">Nothing needs you right now.</div>}
      {data?.items.map((it) => <Card key={it.id} item={it} onDone={refresh} onAsk={onAsk} />)}
      {data && (data.hidden_by_capacity > 0 || data.snoozed > 0) && (
        <p className="loki-ny-foot">
          {data.hidden_by_capacity > 0 && `${data.hidden_by_capacity} more held back to keep today light. `}
          {data.snoozed > 0 && `${data.snoozed} set aside or deferred.`}
        </p>
      )}
    </div>
  );
}
