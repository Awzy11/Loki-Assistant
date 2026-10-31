import { useEffect, useState } from "react";
import { Markdown } from "./Blocks";
import { notesApply, notesFind, notesPlan, type NoteProposal } from "./transport";

const KEY = "loki-notes-draft";
type Mode = "new" | "update" | "auto";
type Row = NoteProposal & { on: boolean };
const MODES: { id: Mode; label: string; hint: string }[] = [
  { id: "new", label: "New note", hint: "Loki cleans it up and files it as its own note." },
  { id: "update", label: "Add to existing notes", hint: "Loki adds it to the notes you pick (or the closest matches it finds). Existing text is never changed." },
  { id: "auto", label: "Let Loki decide", hint: "Loki proposes a new note, additions to related notes, or both." },
];

function loadDraft(): { title: string; text: string } {
  try {
    return JSON.parse(localStorage.getItem(KEY) ?? "") as { title: string; text: string };
  } catch {
    return { title: "", text: "" };
  }
}

/** Write a note in markdown, then have the local model file it into the vault: as its own note, as additions to
 *  existing notes, or its choice. You review and edit every proposed write before anything reaches Obsidian. */
/** The draft (title + text), shared with the dock so it can show that one is waiting. */
export const hasDraft = (): boolean => { const d = loadDraft(); return !!(d.title.trim() || d.text.trim()); };

export default function NotesPage({ onDraft }: { onDraft?: (has: boolean) => void }) {
  const [title, setTitle] = useState(() => loadDraft().title);
  const [text, setText] = useState(() => loadDraft().text);
  const [preview, setPreview] = useState(false);
  const [mode, setMode] = useState<Mode>("auto");
  const [targets, setTargets] = useState<string[]>([]);
  const [q, setQ] = useState("");
  const [found, setFound] = useState<string[]>([]);
  const [rows, setRows] = useState<Row[] | null>(null);
  const [busy, setBusy] = useState<"" | "plan" | "apply">("");
  const [results, setResults] = useState<string[]>([]);
  const [err, setErr] = useState("");

  useEffect(() => {
    try { localStorage.setItem(KEY, JSON.stringify({ title, text })); } catch { /* private window */ }
    onDraft?.(!!(title.trim() || text.trim()));
  }, [title, text]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => {
    if (q.trim().length < 2) { setFound([]); return; }
    const t = setTimeout(() => { notesFind(q).then(setFound).catch(() => setFound([])); }, 300);
    return () => clearTimeout(t);
  }, [q]);

  async function plan() {
    setBusy("plan"); setErr(""); setResults([]);
    try {
      const r = await notesPlan({ title, text, mode, targets });
      setRows(r.proposals.map((p) => ({ ...p, on: true })));
    } catch (e) {
      setErr(String(e instanceof Error ? e.message : e));
    }
    setBusy("");
  }
  function asIs() {
    const name = (title.trim() || text.split("\n").find((l) => l.trim())?.replace(/^#+\s*/, "").slice(0, 60) || "Note").replace(/[\\/:*?"<>|#^[\]]/g, " ").trim();
    setRows([{ on: true, op: "create", path: `00 - Inbox/${name}.md`, content: `---\ntags: [knowledge]\n---\n${title.trim() ? `# ${title.trim()}\n\n` : ""}${text.trim()}\n`, heading: "", why: "Saved as written, no AI." }]);
    setErr("");
  }
  async function apply() {
    if (!rows) return;
    setBusy("apply"); setErr("");
    try {
      const r = await notesApply(rows.filter((x) => x.on).map(({ on: _on, ...p }) => p));
      setResults(r);
      setRows(null);
      if (r.some((x) => !x.startsWith("Skipped"))) { setTitle(""); setText(""); setTargets([]); }
    } catch (e) {
      setErr(String(e instanceof Error ? e.message : e));
    }
    setBusy("");
  }
  const edit = (i: number, patch: Partial<Row>) => setRows((rs) => rs && rs.map((r, j) => (j === i ? { ...r, ...patch } : r)));
  const url = (p: string) => "obsidian://open?vault=Obsidian%20Vault&file=" + encodeURIComponent(p.replace(/\.md$/, ""));

  return (
    <div className="nt">
      <p className="loki-home-note">Write in markdown. Loki files it into your vault, as its own note or added to existing ones. Nothing is written until you approve it. Your draft stays here, on every tab, until you save or discard it.</p>
      {rows === null ? (
        <>
          <input className="nt-title" type="text" value={title} placeholder="Title (optional)" onChange={(e) => setTitle(e.target.value)} />
          <div className="nt-tabs">
            <button type="button" className={`loki-chipbtn ${!preview ? "tk-go" : ""}`} onClick={() => setPreview(false)}>Write</button>
            <button type="button" className={`loki-chipbtn ${preview ? "tk-go" : ""}`} onClick={() => setPreview(true)}>Preview</button>
          </div>
          {preview ? <div className="nt-preview"><Markdown text={text || "*Nothing yet.*"} /></div> : <textarea className="nt-text" value={text} placeholder="# Start writing…" onChange={(e) => setText(e.target.value)} spellCheck />}
          <fieldset className="nt-modes">
            <legend>Where should it go?</legend>
            {MODES.map((m) => (
              <label key={m.id} className={mode === m.id ? "on" : ""}>
                <input type="radio" name="nt-mode" checked={mode === m.id} onChange={() => setMode(m.id)} /> <b>{m.label}</b> <small>{m.hint}</small>
              </label>
            ))}
          </fieldset>
          {mode !== "new" && (
            <div className="nt-targets">
              <input type="search" value={q} placeholder="Find a note to add it to (optional)…" onChange={(e) => setQ(e.target.value)} />
              {found.length > 0 && (
                <ul>{found.filter((p) => !targets.includes(p)).map((p) => <li key={p}><button type="button" className="loki-chipbtn" onClick={() => setTargets([...targets, p])}>+ {p}</button></li>)}</ul>
              )}
              {targets.length > 0 && (
                <div className="nt-picked">{targets.map((p) => <button type="button" key={p} className="mx-chip" title="Remove" onClick={() => setTargets(targets.filter((x) => x !== p))}>{p} ✕</button>)}</div>
              )}
              <small className="mx-meta">{targets.length ? "Loki will add to these." : "None picked: Loki looks for related notes itself."}</small>
            </div>
          )}
          <div className="bi-acts">
            <button type="button" className="loki-chipbtn tk-go" disabled={busy !== "" || !text.trim()} onClick={() => void plan()}>{busy === "plan" ? "Loki is reading it…" : "Run it through AI"}</button>
            <button type="button" className="loki-chipbtn" disabled={busy !== "" || !text.trim()} onClick={asIs}>Save as written (no AI)</button>
            <button type="button" className="loki-chipbtn" disabled={busy !== "" || (!text && !title)} onClick={() => { setText(""); setTitle(""); }}>Discard draft</button>
          </div>
        </>
      ) : (
        <>
          <h3>Review what Loki would write</h3>
          {rows.map((r, i) => (
            <section key={i} className={`mx-card nt-prop ${r.on ? "" : "off"}`}>
              <label className="nt-head"><input type="checkbox" checked={r.on} onChange={(e) => edit(i, { on: e.target.checked })} /> <b>{r.op === "create" ? "New note" : "Add to existing note"}</b></label>
              {r.why && <p className="mx-meta">{r.why}</p>}
              <label>Path<input type="text" value={r.path} disabled={r.op === "append"} onChange={(e) => edit(i, { path: e.target.value })} /></label>
              {r.op === "append" && <label>Under heading (optional)<input type="text" value={r.heading} onChange={(e) => edit(i, { heading: e.target.value })} /></label>}
              <label>{r.op === "create" ? "Note" : "Text to add at the end"}<textarea className="nt-prop-text" value={r.content} onChange={(e) => edit(i, { content: e.target.value })} spellCheck={false} /></label>
            </section>
          ))}
          <div className="bi-acts">
            <button type="button" className="loki-chipbtn tk-go" disabled={busy !== "" || !rows.some((r) => r.on)} onClick={() => void apply()}>{busy === "apply" ? "Writing…" : "Write to the vault"}</button>
            <button type="button" className="loki-chipbtn" disabled={busy !== ""} onClick={() => setRows(null)}>Back to my note</button>
          </div>
        </>
      )}
      {err && <p className="tk-err">{err}</p>}
      {results.length > 0 && (
        <section className="mx-card">
          <h3>Done</h3>
          <ul>{results.map((r, i) => { const p = r.replace(/^(Created|Added to|Skipped, already exists:) /, ""); return <li key={i}>{r} {!r.startsWith("Skipped") && <a href={url(p)}>Open ↗</a>}</li>; })}</ul>
        </section>
      )}
    </div>
  );
}
