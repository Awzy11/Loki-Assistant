import { useEffect, useState } from "react";
import Carousel, { type CardData } from "./Carousel";
import { Icon } from "./Icons";
import NavStrip, { BRIEF_NAMES } from "./NavStrip";
import { briefingPrompt } from "./askPrompts";
import { navigate, parseRoute, slugOf } from "./router";
import type { Briefing, NeedsYou } from "./transport";

function useClock() {
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    const t = window.setInterval(() => setNow(new Date()), 15000);
    return () => window.clearInterval(t);
  }, []);
  return now;
}

const pageOf = (b: Briefing, today: string) => (b.kind === "shape" ? `/day/${today}` : `/b/${slugOf(b.kind)}`);

/** `/` — the landing page: a carousel of briefing cards (art, a one-line title, actions), with the week and every page along the top. */
export default function HomePage({ data, error, onAsk }: { data: NeedsYou | null; error: string | null; onAsk: (prompt: string) => void }) {
  const now = useClock();
  const briefs = data?.briefings ?? [];
  const today = data?.date ?? "";
  const cards: CardData[] = briefs.map((b) => ({ id: b.id, kind: b.kind, title: b.title, blurb: b.blurb, pill: `${BRIEF_NAMES[b.kind] ?? "Briefing"} · ${b.pill}` }));
  const time = now.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
  const dateLabel = now.toLocaleDateString([], { weekday: "long", month: "long", day: "numeric" });

  return (
    <div className="loki-home">
      <header className="loki-home-top">
        <div className="loki-home-brand">
          <img src="/loki.svg?v=2" alt="" width={26} height={26} />
          <span>Loki</span>
          <span className="loki-home-sep">/</span>
          <b>Home</b>
        </div>
        <div className="loki-home-clock">
          <span className="loki-home-time">{time}</span>
          <span className="loki-home-date">{dateLabel}</span>
        </div>
      </header>

      <NavStrip data={data} route={parseRoute("/")} />

      {data?.state.note && <p className="loki-home-note">{data.state.note}</p>}
      {error && <p className="loki-home-err">{error}</p>}

      <Carousel
        cards={cards}
        emptyText={data ? "No briefings today" : "Loading…"}
        onOpen={(c) => {
          const b = briefs.find((x) => x.id === c.id);
          if (b) navigate(pageOf(b, today));
        }}
        actions={(c) => {
          const b = briefs.find((x) => x.id === c.id);
          if (!b) return null;
          return (
            <>
              <button type="button" className="primary" onClick={() => navigate(pageOf(b, today))}>Open</button>
              {b.kind !== "finance" && b.kind !== "mail" && (
                <button type="button" onClick={() => onAsk(briefingPrompt(b))}>Ask Loki</button>
              )}
              {b.link && (
                <a className="loki-card-link" href={b.link} target="_blank" rel="noreferrer">{b.link.replace(/^https?:\/\//, "").replace(/\/$/, "")} ↗</a>
              )}
            </>
          );
        }}
      />

      {data && (data.hidden_by_capacity > 0 || data.snoozed > 0) && (
        <p className="loki-home-foot">
          {data.hidden_by_capacity > 0 && `${data.hidden_by_capacity} more held back to keep today light. `}
          {data.snoozed > 0 && `${data.snoozed} set aside or deferred.`}
        </p>
      )}

      <nav className="loki-home-dock" aria-label="Loki">
        {(
          [
            ["/", "home", "Home"],
            ["/chat", "chat", "Chat"],
            ["/reports", "reports", "Reports"],
            ["/lessons", "lessons", "Lessons"],
            ["/settings", "settings", "Settings"],
          ] as [string, "home" | "chat" | "reports" | "lessons" | "settings", string][]
        ).map(([to, icon, label]) => (
          <button type="button" key={to} className={to === "/" ? "on" : ""} onClick={() => navigate(to)} title={label} aria-label={label}>
            <Icon name={icon} />
          </button>
        ))}
      </nav>
    </div>
  );
}
