import { useEffect, useState } from "react";
import NotesPage, { hasDraft } from "./NotesPage";

const OPEN_KEY = "loki-notes-open";

/** The notes pad, docked bottom-left on every tab. Collapse it to a small button; the draft (and any proposal you're
 *  reviewing) stays until you save it to the vault or discard it. Ctrl+Alt+N toggles it. */
export default function NotesDock() {
  const [open, setOpen] = useState(() => { try { return localStorage.getItem(OPEN_KEY) === "1"; } catch { return false; } });
  const [draft, setDraft] = useState(hasDraft);
  useEffect(() => { try { localStorage.setItem(OPEN_KEY, open ? "1" : "0"); } catch { /* private window */ } }, [open]);
  useEffect(() => {
    const k = (e: KeyboardEvent) => { if (e.ctrlKey && e.altKey && e.key.toLowerCase() === "n") { e.preventDefault(); setOpen((o) => !o); } };
    window.addEventListener("keydown", k);
    return () => window.removeEventListener("keydown", k);
  }, []);
  return (
    <div className={`nd ${open ? "open" : ""}`}>
      {/* stays mounted when collapsed so the draft and any open proposal survive tab changes */}
      <div className="nd-panel" hidden={!open}>
        <header>
          <b>Notes</b>
          <button type="button" className="loki-chipbtn" onClick={() => setOpen(false)} aria-label="Collapse notes">Collapse ▾</button>
        </header>
        <div className="nd-body"><NotesPage onDraft={setDraft} /></div>
      </div>
      {!open && (
        <button type="button" className="nd-tab" onClick={() => setOpen(true)} aria-label="Open notes">
          ✎ Notes{draft && <i className="nd-dot" title="Unsaved draft" />}
        </button>
      )}
    </div>
  );
}
