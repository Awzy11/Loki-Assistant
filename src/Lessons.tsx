import { useEffect, useState } from "react";
import { getLessons, runImprove, setAuditAuto, updateLesson, type Lesson } from "./transport";

export default function Lessons() {
  const [lessons, setLessons] = useState<Lesson[]>([]);
  const [auto, setAuto] = useState<"off" | "downvotes" | "all_local">("downvotes");
  const [error, setError] = useState<string | null>(null);
  const [finding, setFinding] = useState(false);
  const [evidence, setEvidence] = useState("");

  function refresh() {
    getLessons()
      .then((d) => {
        setLessons(d.lessons);
        setAuto(d.settings.auto);
      })
      .catch((e) => setError(String(e)));
  }
  useEffect(refresh, []);

  async function act(id: string, action: "approve" | "reject" | "delete" | "edit", text?: string) {
    try {
      await updateLesson(id, action, text);
      refresh();
    } catch (e) {
      setError(String(e));
    }
  }

  async function findLessons() {
    setFinding(true);
    setEvidence("");
    try {
      setEvidence(await runImprove());
      refresh();
    } catch (e) {
      setError(String(e));
    }
    setFinding(false);
  }

  function edit(l: Lesson) {
    const t = window.prompt("Edit lesson", l.text);
    if (t && t.trim() && t.trim() !== l.text) act(l.id, "edit", t);
  }

  const group = (status: Lesson["status"], title: string, hint: string) => {
    const items = lessons.filter((l) => l.status === status);
    return (
      <section className="loki-lessons-group">
        <h3>
          {title} <span className="loki-lessons-count">{items.length}</span>
        </h3>
        <p className="loki-lessons-hint">{hint}</p>
        {items.map((l) => (
          <div key={l.id} className="loki-lesson">
            <div className="loki-lesson-text">
              {l.kind && l.kind !== "lesson" && <span className="loki-lesson-kind">{l.kind === "tool_hint" ? `Tool hint · ${l.tool ?? ""}` : "Routine edit"}</span>}
              {l.text}
            </div>
            {l.kind === "routine_edit" && l.target && (
              <pre className="loki-plan loki-diff"><b>{l.target}</b>{"\n"}{(l.append ?? "").split("\n").filter((x) => x.trim()).map((x) => `+ ${x}`).join("\n")}</pre>
            )}
            {l.effect && <div className="loki-lessons-hint">Effect: {l.effect}</div>}
            <div className="loki-lesson-actions">
              {status !== "approved" && <button onClick={() => act(l.id, "approve")}>Approve</button>}
              {status === "pending" && <button onClick={() => act(l.id, "reject")}>Reject</button>}
              {status === "approved" && <button onClick={() => act(l.id, "reject")}>Disable</button>}
              <button onClick={() => edit(l)}>Edit</button>
              <button onClick={() => act(l.id, "delete")}>Delete</button>
            </div>
          </div>
        ))}
      </section>
    );
  };

  return (
    <div className="loki-lessons">
      <h2>Proposals</h2>
      <p className="loki-lessons-hint">
        When Claude audits a local-model answer it can suggest a short rule. Approved lessons are added to the local model's
        instructions on every turn; nothing is used until you approve it.
      </p>
      <label className="loki-lessons-auto">
        Automatic audits{" "}
        <select
          value={auto}
          onChange={(e) => {
            const v = e.currentTarget.value as typeof auto;
            setAuto(v);
            setAuditAuto(v).catch((err) => setError(String(err)));
          }}
        >
          <option value="off">Off — only when I press Audit</option>
          <option value="downvotes">When I give a thumbs-down</option>
          <option value="all_local">Every local-model reply (uses more Claude)</option>
        </select>
      </label>
      <div className="loki-lessons-find">
        <button disabled={finding} onClick={() => void findLessons()}>{finding ? "Reading the last 7 days…" : "Find proposals from failures and findings"}</button>
        <span className="loki-lessons-hint"> Counts failed tool calls and step-limit hits, reads the findings files’ rules, and drafts proposals that wait here. Approving a routine edit appends the line to that routine file in the vault.</span>
        {evidence && <pre className="loki-plan">{evidence}</pre>}
      </div>
      {error && <div className="loki-error">{error}</div>}
      {group("pending", "Waiting for approval", "Suggested by audits.")}
      {group("approved", "In force", "Injected into the local model's prompt.")}
      {group("rejected", "Rejected / disabled", "Kept so they aren't suggested again.")}
    </div>
  );
}
