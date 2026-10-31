import { artUrl } from "./art";
import { briefingPrompt } from "./askPrompts";
import { navigate, kindOf } from "./router";
import { dashboardFor } from "./dashboards";
import type { Briefing, NeedsYou } from "./transport";

/** Web links open in a new tab; `obsidian://` and `zotero://` links hand off to the app. */
const linkProps = (url: string) => ({ href: url, target: url.startsWith("http") ? "_blank" : undefined, rel: "noreferrer" });

function Sparkline({ points }: { points: (number | null)[] }) {
  const vals = points.filter((p): p is number => p !== null);
  if (vals.length < 2) return <svg className="loki-spark" viewBox="0 0 220 48" aria-hidden="true" />;
  const min = Math.min(...vals), max = Math.max(...vals);
  const span = max - min || 1;
  const x = (i: number) => (i / (points.length - 1)) * 216 + 2;
  const y = (v: number) => 44 - ((v - min) / span) * 38;
  let d = "";
  let pen = false;
  points.forEach((p, i) => {
    if (p === null) {
      pen = false;
      return;
    }
    d += `${pen ? "L" : "M"}${x(i).toFixed(1)} ${y(p).toFixed(1)} `;
    pen = true;
  });
  const lastIdx = points.map((p) => p !== null).lastIndexOf(true);
  return (
    <svg className="loki-spark" viewBox="0 0 220 48" aria-hidden="true">
      <path d={d} fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
      {lastIdx >= 0 && <circle cx={x(lastIdx)} cy={y(points[lastIdx] as number)} r="3.5" fill="currentColor" />}
    </svg>
  );
}

function RowView({ r }: { r: Briefing["rows"][number] }) {
  if (r.heading) return <div className="loki-brief-subhead">{r.head}</div>;
  return (
    <div className="loki-home-row">
      <b>{r.status && <span className={`loki-dot loki-dot-${r.status}`} aria-hidden="true" />}{r.link ? <a {...linkProps(r.link)}>{r.head} ↗</a> : r.head}</b>
      {r.text && <span>{r.text}</span>}
    </div>
  );
}

/** Rows in one list, or side by side (one column per group) when rows carry a group, e.g. one per mailbox. */
function RowList({ rows }: { rows: Briefing["rows"] }) {
  const groups = Array.from(new Set(rows.map((r) => r.group).filter((g): g is string => !!g)));
  if (groups.length === 0) return <div className="loki-brief-rows">{rows.map((r, i) => <RowView r={r} key={i} />)}</div>;
  return (
    <div className="loki-brief-cols">
      {groups.map((g) => (
        <section className="loki-brief-col" key={g}>
          <h3>{g}</h3>
          <div className="loki-brief-rows">{rows.filter((r) => r.group === g).map((r, i) => <RowView r={r} key={i} />)}</div>
        </section>
      ))}
    </div>
  );
}

function Body({ b }: { b: Briefing }) {
  return (
    <>
      {b.series.length > 0 && (
        <div className="loki-spark-grid">
          {b.series.map((s) => {
            const vals = s.points.filter((p): p is number => p !== null);
            const last = vals.length ? vals[vals.length - 1] : null;
            return (
              <div className="loki-spark-tile" key={s.name}>
                <small>{s.name}</small>
                <b>{last !== null ? `${Number.isInteger(last) ? last : last.toFixed(1)}${s.unit ? ` ${s.unit}` : ""}` : "—"}</b>
                <Sparkline points={s.points} />
                {vals.length > 1 && <small>{Math.min(...vals)} – {Math.max(...vals)}</small>}
              </div>
            );
          })}
        </div>
      )}
      <RowList rows={b.rows} />
    </>
  );
}

/** `/b/<kind>` — one briefing as its own page. */
export default function BriefingPage({ slug, data, onAsk }: { slug: string; data: NeedsYou | null; onAsk: (prompt: string) => void }) {
  const kind = kindOf(slug);
  const b = data?.briefings?.find((x) => x.kind === kind);
  if (!data) return <div className="loki-page"><p className="loki-home-note">Loading…</p></div>;
  if (!b) {
    return (
      <div className="loki-page">
        <div className="loki-page-head"><h2>Nothing here today</h2></div>
        <p className="loki-home-note">This card has no data right now (or it's hidden on a lighter day).</p>
        <button type="button" className="loki-chipbtn" onClick={() => navigate("/")}>Back to Home</button>
      </div>
    );
  }
  const dash = dashboardFor(b, onAsk);
  return dash ?? <BriefingView b={b} onAsk={onAsk} />;
}

/** The hero + rows layout shared by the briefing pages and the day pages. */
export function BriefingView({ b, onAsk, nav }: { b: Briefing; onAsk: (prompt: string) => void; nav?: React.ReactNode }) {
  const ask = briefingPrompt(b);
  return (
    <div className="loki-page">
      {nav}
      <div className="loki-brief-hero" style={{ backgroundImage: artUrl(b.id, b.kind, `${b.title} ${b.blurb}`) }}>
        <div className="loki-home-shade" />
        <span className="loki-home-pill">{b.pill}</span>
        <div className="loki-home-copy">
          <small>{b.blurb}</small>
          <h2>{b.title}</h2>
        </div>
      </div>
      <div className="loki-brief-body">
        <Body b={b} />
        <div className="loki-brief-actions">
          {b.link && <a className="loki-chipbtn" {...linkProps(b.link)}>Open {b.link.replace(/^https?:\/\//, "").replace(/\/$/, "")} ↗</a>}
          {b.kind === "mail" && <button type="button" className="loki-chipbtn" onClick={() => navigate("/mail/review")}>Inbox review →</button>}
          {b.note && <a className="loki-chipbtn" {...linkProps(b.note)}>Open the note ↗</a>}
          {b.kind !== "finance" && b.kind !== "mail" && b.kind !== "mailreview" && <button type="button" className="loki-chipbtn" onClick={() => onAsk(ask)}>Ask Loki about this</button>}
          <span className="loki-ny-source">{b.source_path}</span>
        </div>
      </div>
    </div>
  );
}
