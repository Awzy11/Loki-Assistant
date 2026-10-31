import { useState } from "react";
import { extractApply, extractRun, type ExtractKind, type ExtractProposal, type ExtractSource } from "./transport";

type Row = ExtractProposal & { on: boolean };
const AREAS = ["", "Home", "School", "Self", "Tech"];
const TOPICS = ["", "Technology", "Self", "Philosophy", "Work", "Other"];
const KINDS: { id: ExtractKind; label: string; hint: string }[] = [
  { id: "event", label: "Event", hint: "Goes on the calendar (Upcoming Events) on its date." },
  { id: "todo", label: "To-do", hint: "With a due date it goes on the calendar; without one it goes to the Inbox for triage." },
  { id: "knowledge", label: "Knowledge", hint: "An idea worth keeping. Goes to the Inbox tagged for your knowledge pipeline." },
  { id: "reference", label: "Reference", hint: "A fact you may need later (a number, address, rule). Goes to the Inbox tagged as reference." },
];

/** Obsidian's own link to the calendar note, so you can open it straight after adding. */
const EVENTS_URL = "obsidian://open?vault=Obsidian%20Vault&file=" + encodeURIComponent("60 - MOCs/Upcoming Events");

const blank = (kind: ExtractKind): Row => ({ on: true, kind, title: "", date: "", start: "", end: "", place: "", people: "", purpose: "", next_action: "", area: "", estimate: "", topic: "", note: "" });

/** Pull events, to-dos, knowledge and reference facts out of a message or any text: the model proposes, you change
 *  anything (type included), add your own, and only what you approve is written to the vault. */
export default function ExtractPanel({ source }: { source: ExtractSource }) {
  const [phase, setPhase] = useState<"idle" | "loading" | "review" | "applying" | "done">("idle");
  const [rows, setRows] = useState<Row[]>([]);
  const [meta, setMeta] = useState({ key: "", label: "", link: "", seen: false });
  const [results, setResults] = useState<string[]>([]);
  const [error, setError] = useState("");

  async function run() {
    setPhase("loading");
    setError("");
    try {
      const r = await extractRun(source);
      setMeta({ key: r.key, label: r.label, link: r.link, seen: r.seen });
      setRows(r.proposals.map((p) => ({ ...blank(p.kind), ...p, on: true })));
      setPhase("review");
    } catch (e) {
      setError(String(e));
      setPhase("idle");
    }
  }
  function byHand() {
    setMeta({ key: source.source === "mail" ? `mail:${source.account}:${source.id}` : `hand:${Date.now()}`, label: source.source === "mail" ? "an email" : source.label, link: "", seen: false });
    setRows([blank("todo")]);
    setPhase("review");
  }
  const problem = (r: Row) => (!r.title.trim() ? "needs a title" : r.kind === "event" && !r.date ? "an event needs a date" : "");
  const chosen = rows.filter((r) => r.on);
  const blocked = chosen.find((r) => problem(r));
  async function apply() {
    if (!chosen.length || blocked) return;
    setPhase("applying");
    try {
      setResults(await extractApply(meta.key, meta.label, meta.link, chosen.map(({ on: _on, ...p }) => p)));
      setPhase("done");
    } catch (e) {
      setError(String(e));
      setPhase("review");
    }
  }
  const edit = (i: number, patch: Partial<Row>) => setRows((rs) => rs.map((r, k) => (k === i ? { ...r, ...patch } : r)));
  const remove = (i: number) => setRows((rs) => rs.filter((_, k) => k !== i));

  const field = (label: string, el: React.ReactNode, wide = false) => <label className={wide ? "ex-wide-row" : ""}>{label}{el}</label>;

  return (
    <div className="ex-panel">
      {phase === "idle" && (
        <div className="mx-viewer-actions">
          <button type="button" className="loki-chipbtn" onClick={() => void run()}>⤓ Find items with the local model</button>
          <button type="button" className="loki-chipbtn" onClick={byHand}>+ Add by hand</button>
        </div>
      )}
      {phase === "loading" && <p className="mx-meta">Reading with the local model…</p>}
      {error && <p className="loki-error">{error}</p>}
      {(phase === "review" || phase === "applying") && (
        <>
          <h4>From {meta.label}</h4>
          {meta.seen && <p className="mx-meta">You already added items from this once. Anything already in Upcoming Events is skipped.</p>}
          {rows.length === 0 && <p className="mx-meta">Nothing was suggested. Add your own below.</p>}
          {rows.map((r, i) => (
            <div key={i} className={`ex-item ${r.on ? "" : "off"}`}>
              <input type="checkbox" checked={r.on} onChange={(e) => edit(i, { on: e.target.checked })} aria-label="Include" />
              <div className="ex-fields">
                <div className="ex-kinds" role="group" aria-label="Type">
                  {KINDS.map((k) => <button type="button" key={k.id} className={r.kind === k.id ? "on" : ""} onClick={() => edit(i, { kind: k.id })}>{k.label}</button>)}
                  <button type="button" className="ex-x" onClick={() => remove(i)} aria-label="Remove this item" title="Remove">✕</button>
                </div>
                <input className="ex-title" value={r.title} placeholder={r.kind === "knowledge" ? "State the idea as one sentence" : "Title"} onChange={(e) => edit(i, { title: e.target.value })} aria-label="Title" />
                {r.kind === "event" && (
                  <>
                    {field("Date", <input type="date" value={r.date} onChange={(e) => edit(i, { date: e.target.value })} />)}
                    {field("From", <input type="time" value={r.start} onChange={(e) => edit(i, { start: e.target.value })} />)}
                    {field("To", <input type="time" value={r.end} onChange={(e) => edit(i, { end: e.target.value })} />)}
                    {field("Place", <input value={r.place} placeholder="Where" onChange={(e) => edit(i, { place: e.target.value })} />)}
                    {field("People", <input value={r.people} placeholder="Who is there" onChange={(e) => edit(i, { people: e.target.value })} />)}
                    {field("Purpose", <input className="ex-wide" value={r.purpose} placeholder="Why it matters" onChange={(e) => edit(i, { purpose: e.target.value })} />, true)}
                    {field("Notes", <textarea rows={2} value={r.note} onChange={(e) => edit(i, { note: e.target.value })} />, true)}
                  </>
                )}
                {r.kind === "todo" && (
                  <>
                    {field("Due", <input type="date" value={r.date} onChange={(e) => edit(i, { date: e.target.value })} />)}
                    {field("Estimate", <input className="ex-short" value={r.estimate} placeholder="30m" onChange={(e) => edit(i, { estimate: e.target.value })} />)}
                    {field("Area", <select value={r.area} onChange={(e) => edit(i, { area: e.target.value })}>{AREAS.map((a) => <option key={a} value={a}>{a || "unsure"}</option>)}</select>)}
                    {field("People", <input value={r.people} placeholder="Who is involved" onChange={(e) => edit(i, { people: e.target.value })} />)}
                    {field("First step", <input className="ex-wide" value={r.next_action} placeholder="Open X, do Y" onChange={(e) => edit(i, { next_action: e.target.value })} />, true)}
                    {field("Notes", <textarea rows={2} value={r.note} onChange={(e) => edit(i, { note: e.target.value })} />, true)}
                  </>
                )}
                {r.kind === "knowledge" && (
                  <>
                    {field("Topic", <select value={r.topic} onChange={(e) => edit(i, { topic: e.target.value })}>{TOPICS.map((a) => <option key={a} value={a}>{a || "unsure"}</option>)}</select>)}
                    {field("Body", <textarea rows={4} value={r.note} placeholder="Explain the idea in a few sentences" onChange={(e) => edit(i, { note: e.target.value })} />, true)}
                  </>
                )}
                {r.kind === "reference" && field("Fact to keep", <textarea rows={3} value={r.note} placeholder="The number, address, rule or instruction" onChange={(e) => edit(i, { note: e.target.value })} />, true)}
                <small className={problem(r) && r.on ? "ex-problem" : "mx-meta"}>{problem(r) && r.on ? `This ${KINDS.find((k) => k.id === r.kind)?.label.toLowerCase()} ${problem(r)}.` : r.kind === "todo" && r.date ? KINDS[0].hint : KINDS.find((k) => k.id === r.kind)?.hint}</small>
              </div>
            </div>
          ))}
          <div className="mx-viewer-actions">
            <button type="button" className="loki-chipbtn" onClick={() => setRows((rs) => [...rs, blank("event")])}>+ Event</button>
            <button type="button" className="loki-chipbtn" onClick={() => setRows((rs) => [...rs, blank("todo")])}>+ To-do</button>
            <button type="button" className="loki-chipbtn" onClick={() => setRows((rs) => [...rs, blank("knowledge")])}>+ Knowledge</button>
            <button type="button" className="loki-chipbtn" onClick={() => setRows((rs) => [...rs, blank("reference")])}>+ Reference</button>
          </div>
          <div className="mx-viewer-actions">
            <button type="button" className="loki-chipbtn ex-go" disabled={phase === "applying" || chosen.length === 0 || !!blocked} onClick={() => void apply()}>{phase === "applying" ? "Adding…" : `Add ${chosen.length} to Loki`}</button>
            {blocked && <span className="ex-problem">Fix the item that {problem(blocked)}, or untick it.</span>}
          </div>
        </>
      )}
      {phase === "done" && (
        <>
          <h4>Done</h4>
          {results.map((r, i) => <p key={i} className="mx-meta">✓ {r}</p>)}
          <div className="mx-viewer-actions">
            {results.some((r) => r.includes("Upcoming Events")) && <a className="loki-chipbtn" href={EVENTS_URL}>Open Upcoming Events ↗</a>}
            <button type="button" className="loki-chipbtn" onClick={() => setPhase("review")}>Back to the list</button>
          </div>
        </>
      )}
    </div>
  );
}
