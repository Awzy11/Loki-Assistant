import { navigate, slugOf, type Route } from "./router";
import type { NeedsYou } from "./transport";

/** Order of the page buttons, and what each is called in the strip. */
export const NAV_ORDER = ["start", "shape", "week", "weather", "mail", "news", "leads", "workflows", "backlog", "finance", "zotero", "signals", "countdowns", "rhythm", "saved", "network", "loki"];
export const BRIEF_NAMES: Record<string, string> = {
  start: "Start",
  shape: "Day",
  week: "Sprint",
  news: "News",
  leads: "Leads",
  mail: "Mail",
  workflows: "Workflows",
  weather: "Weather",
  zotero: "Zotero",
  backlog: "Backlog",
  finance: "Finance",
  signals: "Signals",
  countdowns: "Countdowns",
  rhythm: "Rhythm",
  saved: "Saved",
  network: "Network",
  loki: "Loki",
};

const pad = (n: number) => String(n).padStart(2, "0");

/** The seven days of the week containing `date` (YYYY-MM-DD), Monday first. */
export function weekOf(date: string): { iso: string; label: string; day: number }[] {
  const [y, m, d] = date.split("-").map(Number);
  const base = new Date(y, m - 1, d);
  const monday = new Date(base);
  monday.setDate(base.getDate() - ((base.getDay() + 6) % 7));
  return Array.from({ length: 7 }, (_, i) => {
    const x = new Date(monday);
    x.setDate(monday.getDate() + i);
    return { iso: `${x.getFullYear()}-${pad(x.getMonth() + 1)}-${pad(x.getDate())}`, label: x.toLocaleDateString([], { weekday: "short" }), day: x.getDate() };
  });
}

/** The week's days (today highlighted), then Needs you and every page, all as links. */
export default function NavStrip({ data, route }: { data: NeedsYou | null; route: Route }) {
  if (!data) return <div className="loki-navstrip" />;
  const today = data.date;
  const onDay = (iso: string) => route.name === "day" && (route.date === iso || (route.date === "" && iso === today));
  const kinds = new Set((data.briefings ?? []).map((b) => b.kind));
  return (
    <nav className="loki-navstrip" aria-label="Week and pages">
      {weekOf(today).map((d) => (
        <button type="button" key={d.iso} className={`loki-navday ${d.iso === today ? "today" : ""} ${onDay(d.iso) ? "on" : ""}`} onClick={() => navigate(`/day/${d.iso}`)} title={d.iso}>
          <small>{d.label}</small> {d.day}
        </button>
      ))}
      <span className="loki-topnav-sep" aria-hidden="true" />
      <button type="button" className={`loki-navpage needs ${route.name === "needs" || route.name === "needslist" ? "on" : ""}`} onClick={() => navigate("/needs")}>
        Needs you ({data.items.length})
      </button>
      {NAV_ORDER.filter((k) => kinds.has(k as never)).map((k) => {
        const href = k === "shape" ? `/day/${today}` : `/b/${slugOf(k)}`;
        const on = k === "shape" ? onDay(today) : route.name === "brief" && route.slug === slugOf(k);
        return (
          <button type="button" key={k} className={`loki-navpage ${on ? "on" : ""}`} onClick={() => navigate(href)}>
            {BRIEF_NAMES[k]}
          </button>
        );
      })}
    </nav>
  );
}
