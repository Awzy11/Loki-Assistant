import { useEffect, useState } from "react";
import { Markdown } from "./Blocks";
import FixPanel from "./FixPanel";
import { getReport, listReports, type ReportDetail, type ReportSummary } from "./transport";

function when(unix: number): string {
  if (!unix) return "";
  const d = new Date(unix * 1000);
  const now = new Date();
  const time = d.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
  if (d.toDateString() === now.toDateString()) return `Today, ${time}`;
  return `${d.toLocaleDateString([], { month: "short", day: "numeric" })}, ${time}`;
}

const FIX_LABEL: Record<string, string> = {
  staged: "Fix ready to review",
  applying: "Applying and deploying…",
  applied: "Fix applied",
  discarded: "Fix discarded",
  error: "Fix failed",
};

/** Past "Troubleshoot with Claude" reports and what their fixes did — the same list on every device. */
export default function Reports() {
  const [reports, setReports] = useState<ReportSummary[] | null>(null);
  const [open, setOpen] = useState<ReportDetail | null>(null);
  const [error, setError] = useState<string | null>(null);

  function refresh() {
    listReports().then(setReports).catch((e) => setError(String(e)));
  }
  useEffect(refresh, []);

  async function show(id: string) {
    try {
      setOpen(await getReport(id));
    } catch (e) {
      setError(String(e));
    }
  }

  if (open) {
    return (
      <div className="loki-reports">
        <button type="button" onClick={() => { setOpen(null); refresh(); }}>
          ← All reports
        </button>
        <h2>Report from {when(open.created)}</h2>
        <p className="loki-lessons-hint">
          Sent from {open.client === "sidebar" ? "the sidebar" : "the web app"}
          {open.note ? ` — “${open.note}”` : ""}
        </p>
        <Markdown text={open.result} />
        {open.status === "done" && <FixPanel reportId={open.id} initial={open.fix} />}
      </div>
    );
  }

  return (
    <div className="loki-reports">
      <h2>Reports</h2>
      <p className="loki-lessons-hint">Every “Troubleshoot with Claude” report and what its Fix did. Tap one to read it or review its fix.</p>
      {error && <div className="loki-error">{error}</div>}
      {reports === null && !error && <div className="loki-thinking"><span className="loki-spinner" /> Loading…</div>}
      {reports?.length === 0 && <div className="loki-empty">No reports yet.</div>}
      {reports?.map((r) => (
        <button type="button" key={r.id} className="loki-report-card" onClick={() => show(r.id)}>
          <div className="loki-report-top">
            <span>{when(r.created)}</span>
            <span className="loki-report-client">{r.client === "sidebar" ? "sidebar" : "web"}</span>
            {r.fix && <span className={`loki-report-fix loki-report-fix-${r.fix.state}`}>{FIX_LABEL[r.fix.state] ?? r.fix.state}</span>}
          </div>
          <div className="loki-report-summary">{r.summary || "(no summary)"}</div>
          {r.note && <div className="loki-report-note">“{r.note}”</div>}
        </button>
      ))}
    </div>
  );
}
