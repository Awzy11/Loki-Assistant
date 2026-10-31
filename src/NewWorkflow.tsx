import { useContext, useState } from "react";
import { RunContext } from "./routines";

// Hands the description to Loki's chat, which drafts the workflow with the n8n tools (creating one asks you first).
const prompt = (what: string) => `Build me a new n8n workflow. What I want: ${what.trim()}

How to build it:
- First list my existing n8n workflows and reuse their credentials and patterns (Obsidian API Key credential, Miniflux, the Loki API credential). Do not ask me for secrets.
- Plain n8n nodes for everything that does not need inference (fetching, filtering, scoring, formatting, writing to the vault, marking read). Use AI only where judgment is really needed: Fabric on Gemini 2.5 Flash (http://localhost:8080/chat), falling back to the local model only if Gemini reports a usage limit.
- Schedules are America/New_York. Build it inactive, tell me the node plan and what each AI step does, then create it only after I say yes.`;

export default function NewWorkflow() {
  const run = useContext(RunContext);
  const [text, setText] = useState("");
  const [open, setOpen] = useState(false);
  if (!open) return <button type="button" className="loki-chipbtn" onClick={() => setOpen(true)}>＋ New workflow with Claude</button>;
  return (
    <section className="mx-card nw">
      <h3>New workflow</h3>
      <p className="mx-meta">Describe what it should do and when. Claude drafts the n8n workflow in a chat and asks before creating anything.</p>
      <textarea className="nt-area" rows={4} value={text} onChange={(e) => setText(e.target.value)} placeholder="e.g. Every Sunday at 1800, pull my unread Zotero items tagged to-read and add them to the week note as a reading list." />
      <div className="nt-actions">
        <button type="button" className="loki-chipbtn" disabled={text.trim().length < 10} onClick={() => run(prompt(text))}>Draft with Claude</button>
        <button type="button" className="loki-chipbtn" onClick={() => setOpen(false)}>Cancel</button>
      </div>
    </section>
  );
}
