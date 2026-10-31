import { useContext, useState } from "react";
import { RefreshContext } from "./refresh";
import { markTask } from "./transport";

/** A task's checkbox that really changes the note in Obsidian: click it to mark done (or reopen), with an optional comment, or to add a comment without changing it. */
export default function TaskCheck({ path, text, section, done }: { path: string; text: string; section: string; done: boolean }) {
  const refresh = useContext(RefreshContext);
  const [open, setOpen] = useState(false);
  const [comment, setComment] = useState("");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState("");

  async function go(state: boolean | null) {
    setBusy(true);
    setErr("");
    try {
      await markTask(path, text, section, state, comment);
      setOpen(false);
      setComment("");
      refresh();
    } catch (e) {
      setErr(String(e instanceof Error ? e.message : e));
    }
    setBusy(false);
  }
  return (
    <span className="tk-wrap">
      <button type="button" className={`db-check ${done ? "on" : ""}`} onClick={() => setOpen((o) => !o)} aria-label={done ? "Done. Click to reopen or comment" : "Open. Click to mark done or comment"} aria-expanded={open}>{done ? "✓" : ""}</button>
      {open && (
        <div className="tk-pop" role="dialog" aria-label="Update this task">
          <textarea rows={2} value={comment} placeholder="Comment (optional): added to the task in your note" onChange={(e) => setComment(e.target.value)} disabled={busy} autoFocus />
          <div className="tk-acts">
            {done ? <button type="button" disabled={busy} onClick={() => void go(false)}>Reopen</button> : <button type="button" className="tk-go" disabled={busy} onClick={() => void go(true)}>Mark done</button>}
            <button type="button" disabled={busy || !comment.trim()} onClick={() => void go(null)}>Comment only</button>
            <button type="button" disabled={busy} onClick={() => setOpen(false)}>Cancel</button>
          </div>
          {err && <p className="tk-err">{err}</p>}
        </div>
      )}
    </span>
  );
}
