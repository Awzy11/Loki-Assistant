import { useCallback, useEffect, useState } from "react";
import { BriefingView } from "./BriefingPage";
import { mailPlanText, mailReviewPage, mailReviewStart, mailReviewStatus, type Briefing, type MailReviewStatus } from "./transport";

/** `/mail/review` — run the local inbox review, watch progress, read the result and the cleanup plan. */
export default function MailReviewPage({ onAsk }: { onAsk: (prompt: string) => void }) {
  const [st, setSt] = useState<MailReviewStatus | null>(null);
  const [page, setPage] = useState<Briefing | null>(null);
  const [plan, setPlan] = useState<string | null>(null);
  const [err, setErr] = useState("");

  const refresh = useCallback(async () => {
    try {
      const s = await mailReviewStatus();
      setSt(s);
      if (s.status === "done") setPage(await mailReviewPage());
    } catch (e) {
      setErr(String(e));
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);
  useEffect(() => {
    if (st?.status !== "running") return;
    const t = setInterval(() => void refresh(), 4000);
    return () => clearInterval(t);
  }, [st?.status, refresh]);

  const run = async () => {
    setErr("");
    setPlan(null);
    try {
      await mailReviewStart();
      await refresh();
    } catch (e) {
      setErr(String(e));
    }
  };
  const showPlan = async () => {
    try {
      setPlan(await mailPlanText());
    } catch (e) {
      setErr(String(e));
    }
  };

  const running = st?.status === "running";
  return (
    <>
      <div className="loki-page">
        <div className="loki-page-head"><h2>Inbox review</h2></div>
        <p className="loki-home-note">The local model samples each mailbox and sorts it. Nothing leaves your servers and nothing is changed in Gmail.</p>
        <div className="loki-brief-actions">
          <button type="button" className="loki-chipbtn" disabled={running} onClick={() => void run()}>{running ? "Reviewing…" : st?.status === "done" ? "Run again" : "Run the review"}</button>
          {st?.status === "done" && <button type="button" className="loki-chipbtn" onClick={() => void showPlan()}>Show the cleanup plan</button>}
          {running && <span className="loki-ny-source">{st?.phase} · {st?.done}/{st?.total}</span>}
          {(err || st?.error) && <span className="loki-ny-source">{err || st?.error}</span>}
        </div>
        {plan && (
          <div className="loki-brief-body">
            <button type="button" className="loki-chipbtn" onClick={() => void navigator.clipboard?.writeText(plan)}>Copy plan</button>
            <pre className="loki-plan">{plan}</pre>
          </div>
        )}
      </div>
      {page && <BriefingView b={page} onAsk={onAsk} />}
    </>
  );
}
