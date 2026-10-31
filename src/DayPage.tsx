import { useEffect, useState } from "react";
import { BriefingView } from "./BriefingPage";
import { weekOf } from "./NavStrip";
import { dashboardFor } from "./dashboards";
import { navigate } from "./router";
import { getDay, type Briefing } from "./transport";

function shift(date: string, n: number): string {
  const [y, m, d] = date.split("-").map(Number);
  const x = new Date(y, m - 1, d + n);
  return `${x.getFullYear()}-${String(x.getMonth() + 1).padStart(2, "0")}-${String(x.getDate()).padStart(2, "0")}`;
}

/** `/day/<date>` — one calendar day from the vault (today's shape for today). */
export default function DayPage({ date, today, onAsk }: { date: string; today: string; onAsk: (prompt: string) => void }) {
  const iso = date || today;
  const [b, setB] = useState<Briefing | null>(null);
  const [err, setErr] = useState<string | null>(null);
  useEffect(() => {
    if (!iso) return;
    setB(null);
    setErr(null);
    getDay(iso).then(setB).catch((e) => setErr(String(e instanceof Error ? e.message : e)));
  }, [iso]);

  const nav = (
    <div className="loki-page-head">
      <div className="loki-daynav">
        <button type="button" className="loki-chipbtn" onClick={() => navigate(`/day/${shift(iso, -1)}`)} aria-label="Previous day">‹</button>
        <button type="button" className="loki-chipbtn" onClick={() => navigate(`/day/${today}`)} disabled={iso === today}>Today</button>
        <button type="button" className="loki-chipbtn" onClick={() => navigate(`/day/${shift(iso, 1)}`)} aria-label="Next day">›</button>
        <small>{weekOf(iso)[0].iso} → {weekOf(iso)[6].iso}</small>
      </div>
    </div>
  );
  if (err) return <div className="loki-page">{nav}<p className="loki-home-err">{err}</p></div>;
  if (!b) return <div className="loki-page">{nav}<p className="loki-home-note">Loading…</p></div>;
  const dash = dashboardFor(b, onAsk);
  return dash ? <>{nav}{dash}</> : <BriefingView b={b} onAsk={onAsk} nav={nav} />;
}
