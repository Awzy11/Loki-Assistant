import { useContext, useEffect, useState } from "react";
import { Markdown } from "./Blocks";
import { RefreshContext } from "./refresh";
import { backlogEdit, getBacklogItem } from "./transport";

/** One backlog note opened in place: read it, edit it, comment, set a status, or close it (done / superseded / dropped). Every change is written to the note in Obsidian. */
export default function BacklogItem({ path, name, onClose }: { path: string; name: string; onClose: () => void }) {
  const refresh = useContext(RefreshContext);
  const [text, setText] = useState<string | null>(null);
  const [draft, setDraft] = useState<string | null>(null);
  const [comment, setComment] = useState("");
  const [status, setStatus] = useState("");
  const [closing, setClosing] = useState<"" | "done" | "superseded" | "dropped">("");
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState("");
  const [err, setErr] = useState("");

  const load = () => getBacklogItem(path).then((t) => { setText(t); setDraft(null); }).catch((e) => setErr(String(e instanceof Error ? e.message : e)));
  useEffect(() => { void load(); }, [path]); // eslint-disable-line react-hooks/exhaustive-deps

  async function run(req: Omit<Parameters<typeof backlogEdit>[0], "path">, after?: () => void) {
    setBusy(true);
    setErr("");
    setMsg("");
    try {
      const m = await backlogEdit({ path, ...req });
      setMsg(m);
      refresh();
      if (after) after();
      else await load();
    } catch (e) {
      setErr(String(e instanceof Error ? e.message : e));
    }
    setBusy(false);
  }
  // the body without the frontmatter block, which is shown as a few fields instead
  const body = (text ?? "").replace(/^---\n[\s\S]*?\n---\n?/, "");
  return (
    <div className="bi-back" role="dialog" aria-label={`Backlog item: ${name}`} onClick={(e) => { if (e.target === e.currentTarget) onClose(); }}>
      <div className="bi-panel">
        <header>
          <h3>{name}</h3>
          <button type="button" className="loki-chipbtn" onClick={onClose}>Close ✕</button>
        </header>
        {text === null && !err && <p className="loki-home-note">Loading…</p>}
        {text !== null && draft === null && (
          <>
            <div className="bi-body"><Markdown text={body} /></div>
            <div className="bi-acts"><button type="button" className="loki-chipbtn" onClick={() => setDraft(text)}>Edit the note</button></div>
          </>
        )}
        {text !== null && draft !== null && (
          <>
            <textarea className="bi-edit" value={draft} onChange={(e) => setDraft(e.target.value)} spellCheck={false} disabled={busy} />
            <div className="bi-acts">
              <button type="button" className="loki-chipbtn tk-go" disabled={busy || draft === text} onClick={() => void run({ action: "save", content: draft, base: text })}>Save to Obsidian</button>
              <button type="button" className="loki-chipbtn" disabled={busy} onClick={() => setDraft(null)}>Discard changes</button>
            </div>
          </>
        )}
        {text !== null && draft === null && (
          <div className="bi-tools">
            <label>Comment
              <textarea rows={2} value={comment} placeholder="Added to the note under Comments, dated" onChange={(e) => setComment(e.target.value)} disabled={busy} />
            </label>
            <div className="bi-acts"><button type="button" className="loki-chipbtn" disabled={busy || !comment.trim()} onClick={() => void run({ action: "comment", comment }, () => { setComment(""); void load(); })}>Add comment</button></div>
            <label>Status label (stays in the backlog)
              <span className="bi-row">
                <input type="text" value={status} maxLength={40} placeholder="blocked, waiting, someday…" onChange={(e) => setStatus(e.target.value)} disabled={busy} />
                <button type="button" className="loki-chipbtn" disabled={busy} onClick={() => void run({ action: "status", status })}>{status.trim() ? "Set status" : "Clear status"}</button>
              </span>
            </label>
            <div className="bi-close">
              <span>Close it (files it in 99 - Archive):</span>
              <button type="button" className={`loki-chipbtn ${closing === "done" ? "tk-go" : ""}`} onClick={() => setClosing("done")} disabled={busy}>Done</button>
              <button type="button" className={`loki-chipbtn ${closing === "superseded" ? "tk-go" : ""}`} onClick={() => setClosing("superseded")} disabled={busy}>Superseded</button>
              <button type="button" className={`loki-chipbtn ${closing === "dropped" ? "tk-go" : ""}`} onClick={() => setClosing("dropped")} disabled={busy}>Dropped</button>
            </div>
            {closing && (
              <div className="bi-confirm">
                <input type="text" value={comment} placeholder={closing === "superseded" ? "Replaced by… (e.g. [[Other Item]])" : "Note (optional)"} onChange={(e) => setComment(e.target.value)} disabled={busy} />
                <button type="button" className="loki-chipbtn tk-go" disabled={busy} onClick={() => void run({ action: "close", status: closing, comment }, onClose)}>Confirm: mark {closing} and archive</button>
                <button type="button" className="loki-chipbtn" disabled={busy} onClick={() => setClosing("")}>Cancel</button>
              </div>
            )}
          </div>
        )}
        {msg && <p className="loki-home-note">{msg}</p>}
        {err && <p className="tk-err">{err}</p>}
      </div>
    </div>
  );
}
