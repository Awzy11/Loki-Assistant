import { useContext, useEffect, useMemo, useState, type ReactNode } from "react";
import { BarList, Kpis, Pills, type Opt } from "./dash";
import { navigate } from "./router";
import RoutineBar, { RunContext } from "./routines";
import BacklogItem from "./BacklogItem";
import NewWorkflow from "./NewWorkflow";
import TaskCheck from "./TaskCheck";
import { RefreshContext } from "./refresh";
import { ensureRoutines, financeSyncNow, runWorkflow, type Briefing } from "./transport";

// ---------- shared bits ----------
const TYPE_COLOR: [string, string][] = [["school", "#3987e5"], ["research", "#3987e5"], ["admin", "#8c98a2"], ["home", "#d55181"], ["family", "#d55181"], ["project", "#199e70"], ["fitness", "#2f9e4a"], ["physical", "#2f9e4a"], ["personal", "#c98500"]];
const typeColor = (t: string) => TYPE_COLOR.find(([k]) => t.toLowerCase().includes(k))?.[1] ?? "#9085e9";
const fmt = (m: number) => `${String(Math.floor(m / 60)).padStart(2, "0")}${String(m % 60).padStart(2, "0")}`;
const href = (u: string) => ({ href: u, target: u.startsWith("http") ? "_blank" : undefined, rel: "noreferrer noopener" });
const Link = ({ url, children }: { url?: string | null; children: ReactNode }) => (url ? <a {...href(url)}>{children}</a> : <>{children}</>);

function Shell({ b, children, onAsk }: { b: Briefing; children: ReactNode; onAsk?: (p: string) => void }) {
  return (
    <div className="loki-page mx">
      <div className="loki-page-head">
        <h2>{b.title}</h2>
        <div className="mx-head-actions">
          {b.note && <a className="loki-chipbtn" {...href(b.note)}>Open the note ↗</a>}
          {onAsk && <button type="button" className="loki-chipbtn" onClick={() => onAsk(`Here is the "${b.title}" card: ${b.blurb}. ${b.rows.map((r) => `${r.head}: ${r.text}`).join(" | ")}. Using only this, what stands out and what should I do first? Do not search anything.`)}>Ask Loki about this</button>}
          <button type="button" className="loki-chipbtn" onClick={() => navigate("/")}>Home</button>
        </div>
      </div>
      <p className="loki-home-note">{b.blurb}</p>
      <RoutineBar tab={b.kind} />
      {children}
    </div>
  );
}

const Check = ({ done }: { done: boolean }) => <span className={`db-check ${done ? "on" : ""}`} aria-label={done ? "Done" : "Open"}>{done ? "✓" : ""}</span>;
const Progress = ({ done, total, color }: { done: number; total: number; color?: string }) => (
  <span className="db-prog"><span style={{ width: `${total ? (done / total) * 100 : 0}%`, background: color ?? "var(--good)" }} /></span>
);

// ---------- Today ----------
type TWin = { start: number; end: number; type: string; what: string; link?: string | null; current: boolean };
type TItem = { text: string; done: boolean; mit?: number | null; link?: string | null };
type TRoutine = { cmd: string; text: string; done: boolean };
type TData = {
  now_min: number; windows: TWin[]; mits: { n: number; text: string; state: string }[]; mits_title?: string; routines_title?: string; routine_cmds?: string[];
  sections: { name: string; done: number; total: number; items: TItem[] }[];
  routines?: TRoutine[]; savers?: { text: string; done: boolean }[]; weather?: { slot: string; cond: string; temp: string; humidity: string; rain: string }[];
  state: string; done: number; total: number;
};

/** The day's (or sprint's) routine to-dos: tick them in the note, or click to run the command in Loki. Makes sure the lines exist in the note. */
function RoutineList({ path, routines, want, section }: { path: string; routines: TRoutine[]; want: string[]; section: string }) {
  const run = useContext(RunContext);
  const refresh = useContext(RefreshContext);
  const missing = want.some((c) => !routines.some((r) => r.cmd === c));
  useEffect(() => {
    if (!missing) return;
    ensureRoutines(path).then((added) => added && refresh()).catch(() => {});
  }, [path, missing]); // eslint-disable-line react-hooks/exhaustive-deps
  const ordered = want.map((c) => routines.find((r) => r.cmd === c)).filter((r): r is TRoutine => !!r);
  if (!ordered.length) return <p className="mx-meta">Adding the routine to-dos to the note…</p>;
  return (
    <div className="td-routines">
      {ordered.map((r) => (
        <div key={r.cmd} className={`td-routine ${r.done ? "done" : ""}`}>
          <TaskCheck path={path} text={r.text} section={section} done={r.done} />
          <button type="button" className="td-run" onClick={() => run(r.cmd)} title={`Run ${r.cmd} in Loki`}>▶ <b>{r.cmd}</b> <span>{r.text.replace(r.cmd, "").replace(/^\s*—\s*/, "")}</span></button>
        </div>
      ))}
    </div>
  );
}

function Today({ b, onAsk }: { b: Briefing; onAsk: (p: string) => void }) {
  const d = b.data as TData;
  const HOUR = 46;
  const lo = Math.floor(Math.min(7 * 60, ...d.windows.map((w) => w.start)) / 60) * 60;
  const hi = Math.ceil(Math.max(21 * 60, ...d.windows.map((w) => w.end)) / 60) * 60;
  const y = (m: number) => ((m - lo) / 60) * HOUR;
  const hours = [...Array((hi - lo) / 60 + 1)].map((_, i) => lo + i * 60);
  const cur = d.windows.find((w) => w.current);
  const next = d.windows.find((w) => w.start > d.now_min);
  const mitsDone = d.mits.filter((m) => m.state === "done").length;
  const pct = d.total ? d.done / d.total : 0;
  const C = 2 * Math.PI * 34;
  return (
    <Shell b={b} onAsk={onAsk}>
      <Kpis items={[
        { n: cur ? cur.what.slice(0, 22) : next ? `${fmt(next.start)}` : "Done", label: cur ? "Now" : next ? `Next: ${next.what.slice(0, 26)}` : "Blocks finished" },
        { n: `${d.windows.filter((w) => w.end <= d.now_min).length}/${d.windows.length}`, label: "Blocks done" },
        { n: `${mitsDone}/${d.mits.length}`, label: "MITs done" },
        { n: `${d.done}/${d.total}`, label: "Tasks done" },
        { n: d.state || "—", label: "Starting state" },
      ]} />
      <div className="td-grid">
        <section className="mx-card td-sched">
          <h3>Today's schedule</h3>
          <div className="td-day" style={{ height: y(hi) }} role="img" aria-label="Today's schedule">
            {hours.map((h) => <div key={h} className="td-hour" style={{ top: y(h) }}><span>{fmt(h)}</span></div>)}
            {d.windows.map((w, i) => (
              <div key={i} className={`td-block ${w.current ? "now" : ""} ${w.end <= d.now_min ? "past" : ""}`} style={{ top: y(w.start), height: Math.max(24, y(w.end) - y(w.start) - 2), ["--c" as string]: typeColor(w.type) }} title={`${fmt(w.start)}–${fmt(w.end)} ${w.type}: ${w.what}`}>
                <b>{fmt(w.start)}–{fmt(w.end)} <em>{w.type}</em></b>
                <span><Link url={w.link}>{w.what}</Link></span>
              </div>
            ))}
            {d.now_min >= lo && d.now_min <= hi && <div className="td-nowline" style={{ top: y(d.now_min) }}><i /></div>}
          </div>
        </section>
        <div className="td-side">
          <section className="mx-card td-progress">
            <svg viewBox="0 0 84 84" className="td-ring" role="img" aria-label={`${d.done} of ${d.total} tasks done`}>
              <circle cx="42" cy="42" r="34" fill="none" stroke="var(--field)" strokeWidth="9" />
              <circle cx="42" cy="42" r="34" fill="none" stroke="var(--good)" strokeWidth="9" strokeLinecap="round" strokeDasharray={`${C * pct} ${C}`} transform="rotate(-90 42 42)" />
              <text x="42" y="47" textAnchor="middle" className="td-ring-n">{d.done}/{d.total}</text>
            </svg>
            <div>
              <h3>{d.mits_title || "Priorities"}</h3>
              <div className="db-mits">
                {d.mits.map((m) => (
                  <div key={m.n} className={`db-mit ${m.state}`}><b>{m.n}</b><span>{m.text}</span><em>{m.state === "done" ? "Done" : m.state === "open" ? "In progress" : "Not tracked"}</em></div>
                ))}
              </div>
            </div>
          </section>
          <section className="mx-card">
            <h3>Routines</h3>
            <RoutineList path={b.source_path} routines={d.routines ?? []} want={d.routine_cmds ?? []} section={d.routines_title ?? "Routines"} />
          </section>
          {(d.savers?.length ?? 0) > 0 && (
            <section className="mx-card">
              <h3>SAVERS <small>{d.savers!.filter((s) => s.done).length}/{d.savers!.length}</small></h3>
              <div className="td-savers">
                {d.savers!.map((s) => <span key={s.text} className={`td-saver ${s.done ? "done" : ""}`}><TaskCheck path={b.source_path} text={s.text} section="" done={s.done} />{s.text}</span>)}
              </div>
            </section>
          )}
          {(d.weather?.length ?? 0) > 0 && (
            <section className="mx-card">
              <h3>Weather</h3>
              <div className="td-weather">
                {d.weather!.map((w) => <div key={w.slot}><b>{w.slot}</b><span>{w.temp}</span><small>{w.cond}</small></div>)}
              </div>
            </section>
          )}
        </div>
      </div>
      <div className="mx-charts" style={{ marginTop: 14 }}>
        {d.sections.map((s) => (
          <section className="mx-card" key={s.name}>
            <h3>{s.name} <small>{s.done}/{s.total}</small></h3>
            <Progress done={s.done} total={s.total} />
            <div className="db-items">
              {s.items.map((it, i) => <div key={i} className={`db-item ${it.done ? "done" : ""}`}><TaskCheck path={b.source_path} text={it.text} section={s.name} done={it.done} /><Link url={it.link}>{it.text}</Link>{it.mit ? <span className="mx-chip q">MIT {it.mit}</span> : null}</div>)}
            </div>
          </section>
        ))}
      </div>
    </Shell>
  );
}

// ---------- Sprint ----------
type SData = {
  week: string; days_left: number | null; goal: string; priorities: string[]; routines_title?: string; routine_cmds?: string[];
  capacity: { available: number | null; committed: number | null; buffer: number | null; buffer_pct: number | null };
  wip: { active: number; max: number; done: number } | null;
  active: { text: string; done: boolean; link?: string | null }[];
  domains: { name: string; done: number; total: number; items: TItem[] }[];
  events: { day: string; text: string }[]; queue: number; done: string[];
  routines?: TRoutine[];
};

function Sprint({ b, onAsk }: { b: Briefing; onAsk: (p: string) => void }) {
  const d = b.data as SData;
  const cap = d.capacity;
  const avail = cap.available ?? 0, com = cap.committed ?? 0;
  const pct = cap.buffer_pct ?? (avail ? Math.round(((avail - com) / avail) * 100) : null);
  const total = d.domains.reduce((n, x) => n + x.total, 0), done = d.domains.reduce((n, x) => n + x.done, 0);
  return (
    <Shell b={b} onAsk={onAsk}>
      <Kpis items={[
        { n: d.days_left ?? "—", label: "Days left" },
        { n: d.wip ? `${d.wip.active}/${d.wip.max}` : "—", label: "Active (WIP)" },
        { n: d.wip?.done ?? done, label: "Done" },
        { n: pct !== null ? `${pct}%` : "—", label: "Buffer", tone: pct !== null && pct < 20 ? "bad" : "" },
        { n: d.queue, label: "In the queue" },
      ]} />
      <section className="mx-card" style={{ marginBottom: 14 }}>
        <h3>Routines</h3>
        <RoutineList path={b.source_path} routines={d.routines ?? []} want={d.routine_cmds ?? []} section={d.routines_title ?? "Routines"} />
      </section>
      <div className="mx-charts">
        <section className="mx-card">
          <h3>Goal</h3>
          <p className="db-goal">{d.goal}</p>
          {d.priorities.length > 0 && <ol className="db-prio">{d.priorities.map((p, i) => <li key={i}>{p}</li>)}</ol>}
        </section>
        <section className="mx-card">
          <h3>Capacity</h3>
          {avail > 0 ? (
            <>
              <div className="db-cap" role="img" aria-label="Capacity">
                <span className="used" style={{ flexGrow: com }}>{com}h committed</span>
                <span className="free" style={{ flexGrow: Math.max(0, avail - com) }}>{Math.max(0, +(avail - com).toFixed(1))}h buffer</span>
              </div>
              <p className="mx-meta">{avail}h available. Target: keep at least 20% as buffer.</p>
            </>
          ) : <p className="mx-meta">No capacity numbers in the note yet.</p>}
          {d.wip && (
            <div className="db-slots" aria-label="Work in progress">
              {[...Array(d.wip.max)].map((_, i) => <span key={i} className={i < d.wip!.active ? "on" : ""} />)}
              <em>{d.wip.active} of {d.wip.max} active slots</em>
            </div>
          )}
        </section>
        <section className="mx-card">
          <h3>Progress by area <small>{done}/{total}</small></h3>
          {d.domains.map((x) => (
            <div key={x.name} className="db-dom"><span>{x.name}</span><Progress done={x.done} total={x.total} color={typeColor(x.name)} /><em>{x.done}/{x.total}</em></div>
          ))}
        </section>
      </div>
      {d.active.length > 0 && (
        <section className="mx-card" style={{ marginTop: 14 }}>
          <h3>Active this sprint</h3>
          <div className="db-items">{d.active.map((a, i) => <div key={i} className={`db-item ${a.done ? "done" : ""}`}><Check done={a.done} /><Link url={a.link}>{a.text}</Link></div>)}</div>
        </section>
      )}
      <div className="mx-charts" style={{ marginTop: 14 }}>
        {d.domains.map((x) => (
          <section className="mx-card" key={x.name} style={{ borderTop: `3px solid ${typeColor(x.name)}` }}>
            <h3>{x.name}</h3>
            <div className="db-items">{x.items.map((it, i) => <div key={i} className={`db-item ${it.done ? "done" : ""}`}><TaskCheck path={b.source_path} text={it.text} section={x.name} done={it.done} /><Link url={it.link}>{it.text}</Link></div>)}</div>
          </section>
        ))}
      </div>
      {d.events.length > 0 && (
        <section className="mx-card" style={{ marginTop: 14 }}>
          <h3>Events this week</h3>
          <div className="db-week">{d.events.map((e, i) => <div key={i} className="db-day"><b>{e.day}</b><span>{e.text}</span></div>)}</div>
        </section>
      )}
      {d.done.length > 0 && (
        <section className="mx-card" style={{ marginTop: 14 }}>
          <h3>Done</h3>
          <div className="db-items">{d.done.map((x, i) => <div key={i} className="db-item done"><Check done />{x}</div>)}</div>
        </section>
      )}
    </Shell>
  );
}

// ---------- Backlog ----------
type BItem = { area: string; name: string; estimate: string; next_action: string; unlock: string; status?: string; sprint: string; path: string; link: string };
const AREA_COLOR: Record<string, string> = { Tech: "#199e70", Home: "#d55181", School: "#3987e5", Self: "#c98500" };
const hours = (e: string) => {
  const h = e.match(/(\d+(?:\.\d+)?)\s*h/i), m = e.match(/(\d+)\s*m/i);
  return (h ? parseFloat(h[1]) : 0) + (m ? parseInt(m[1], 10) / 60 : 0);
};

function Backlog({ b, onAsk }: { b: Briefing; onAsk: (p: string) => void }) {
  const items = (b.data as { items: BItem[] }).items;
  const [area, setArea] = useState<Set<string>>(new Set());
  const [sprint, setSprint] = useState<Set<string>>(new Set());
  const [has, setHas] = useState<Set<string>>(new Set());
  const [q, setQ] = useState("");
  const [openItem, setOpenItem] = useState<BItem | null>(null);
  const flip = (set: Set<string>, setter: (s: Set<string>) => void) => (k: string) => { const n = new Set(set); n.has(k) ? n.delete(k) : n.add(k); setter(n); };
  const hasOf = (i: BItem) => [...(i.next_action ? ["next"] : ["no_next"]), ...(i.estimate ? ["est"] : ["no_est"]), ...(i.unlock ? ["unlock"] : [])];
  const pass = (i: BItem, skip?: string) =>
    (skip === "area" || area.size === 0 || area.has(i.area)) &&
    (skip === "sprint" || sprint.size === 0 || sprint.has(i.sprint || "none")) &&
    (skip === "has" || has.size === 0 || [...has].every((h) => hasOf(i).includes(h))) &&
    (!q || `${i.name} ${i.next_action} ${i.unlock}`.toLowerCase().includes(q.toLowerCase()));
  const shown = useMemo(() => items.filter((i) => pass(i)), [items, area, sprint, has, q]); // eslint-disable-line react-hooks/exhaustive-deps
  const areas = Array.from(new Set(items.map((i) => i.area))).sort();
  const opt = (k: string, label: string, test: (i: BItem) => boolean, skip: string, color?: string): Opt => ({ key: k, label, n: items.filter((i) => pass(i, skip) && test(i)).length, color });
  const totalH = shown.reduce((n, i) => n + hours(i.estimate), 0);
  return (
    <Shell b={b} onAsk={onAsk}>
      <Kpis items={[
        { n: items.length, label: "Backlog items", onClick: () => { setArea(new Set()); setSprint(new Set()); setHas(new Set()); setQ(""); } },
        { n: items.filter((i) => i.sprint === "active").length, label: "Active this sprint", onClick: () => setSprint(new Set(["active"])) },
        { n: items.filter((i) => i.sprint === "queue").length, label: "Queued", onClick: () => setSprint(new Set(["queue"])) },
        { n: items.filter((i) => !i.next_action).length, label: "Without a next action", tone: "warn", onClick: () => setHas(new Set(["no_next"])) },
        { n: `${Math.round(totalH * 10) / 10}h`, label: "Estimated (shown)" },
      ]} />
      <div className="mx-charts">
        <section className="mx-card"><h3>By area</h3><BarList opts={areas.map((a) => opt(a, a, (i) => i.area === a, "area", AREA_COLOR[a]))} active={area} onToggle={flip(area, setArea)} /></section>
        <section className="mx-card"><h3>Sprint status</h3><BarList opts={[opt("active", "Active", (i) => i.sprint === "active", "sprint", "#e6a04b"), opt("queue", "Queued", (i) => i.sprint === "queue", "sprint", "#6f8fb0"), opt("none", "Not in the sprint", (i) => !i.sprint, "sprint", "#6a757d")]} active={sprint} onToggle={flip(sprint, setSprint)} /></section>
      </div>
      <div className="mx-slicers">
        <div className="mx-slicers-head"><h3>Filter</h3><input className="nw-search" style={{ maxWidth: 280 }} type="search" placeholder="Search items…" value={q} onChange={(e) => setQ(e.target.value)} /></div>
        <Pills title="Area" opts={areas.map((a) => opt(a, a, (i) => i.area === a, "area", AREA_COLOR[a]))} active={area} onToggle={flip(area, setArea)} />
        <Pills title="Sprint" opts={[opt("active", "Active", (i) => i.sprint === "active", "sprint"), opt("queue", "Queued", (i) => i.sprint === "queue", "sprint"), opt("none", "Not in sprint", (i) => !i.sprint, "sprint")]} active={sprint} onToggle={flip(sprint, setSprint)} />
        <Pills title="Has" opts={[opt("next", "Next action", (i) => !!i.next_action, "has"), opt("no_next", "No next action", (i) => !i.next_action, "has"), opt("est", "Estimate", (i) => !!i.estimate, "has"), opt("unlock", "Unlocks something", (i) => !!i.unlock, "has")]} active={has} onToggle={flip(has, setHas)} />
      </div>
      <p className="mx-toolbar">{shown.length} of {items.length} items</p>
      <div className="db-cards">
        {shown.map((i) => (
          <article key={`${i.area}/${i.name}`} className="db-card" style={{ ["--c" as string]: AREA_COLOR[i.area] ?? "#9085e9" }}>
            <h4><button type="button" className="bi-name" onClick={() => setOpenItem(i)}>{i.name}</button> <a className="bi-ext" {...href(i.link)} title="Open in Obsidian" aria-label="Open in Obsidian">↗</a></h4>
            <div className="mx-tags">
              <span className="mx-chip cat" style={{ ["--cat" as string]: AREA_COLOR[i.area] }}><i />{i.area}</span>
              {i.estimate && <span className="mx-chip">{i.estimate}</span>}
              {i.status && <span className="mx-chip">{i.status}</span>}
              {i.sprint === "active" && <span className="mx-chip urg urg-priority">◆ Active</span>}
              {i.sprint === "queue" && <span className="mx-chip">Queued</span>}
            </div>
            {i.next_action ? <p><b>Start:</b> {i.next_action}</p> : <p className="mx-meta">No next action yet.</p>}
            {i.unlock && <p className="mx-meta">Unlocks: {i.unlock}</p>}
          </article>
        ))}
      </div>
      {openItem && <BacklogItem path={openItem.path} name={openItem.name} onClose={() => setOpenItem(null)} />}
    </Shell>
  );
}

// ---------- Countdowns ----------
type CEv = { date: string; days: number; text: string; topic: string; link?: string | null };
const TOPIC_COLOR: Record<string, string> = { school: "#3987e5", fitness: "#2f9e4a", home: "#d55181", other: "#9085e9" };
const TOPIC_LABEL: Record<string, string> = { school: "School", fitness: "Fitness", home: "Home and family", other: "Other" };

function Countdowns({ b, onAsk }: { b: Briefing; onAsk: (p: string) => void }) {
  const events = (b.data as { events: CEv[] }).events;
  const [topics, setTopics] = useState<Set<string>>(new Set());
  const shown = events.filter((e) => topics.size === 0 || topics.has(e.topic));
  const max = Math.max(30, ...events.map((e) => e.days));
  const opts: Opt[] = Object.keys(TOPIC_LABEL).map((k) => ({ key: k, label: TOPIC_LABEL[k], n: events.filter((e) => e.topic === k).length, color: TOPIC_COLOR[k] }));
  const flip = (k: string) => setTopics((s) => { const n = new Set(s); n.has(k) ? n.delete(k) : n.add(k); return n; });
  const next = events.find((e) => e.days >= 0);
  const months = Array.from(new Set(shown.map((e) => e.date.slice(0, 7))));
  const when = (n: number) => (n === 0 ? "Today" : n === 1 ? "Tomorrow" : `${n} days`);
  return (
    <Shell b={b} onAsk={onAsk}>
      <Kpis items={[
        { n: next ? when(next.days) : "—", label: next ? `Next: ${next.text.slice(0, 34)}` : "Nothing ahead" },
        { n: events.filter((e) => e.days <= 7).length, label: "In the next 7 days", tone: "warn" },
        { n: events.filter((e) => e.days <= 30).length, label: "In the next 30 days" },
        { n: events.length, label: "In the next 60 days" },
      ]} />
      <section className="mx-card">
        <h3>The next {max} days</h3>
        <div className="db-line-track" role="img" aria-label="Events along the next days">
          {[0, 7, 14, 21, 28, 35, 42, 49, 56].filter((d) => d <= max).map((d) => <span key={d} className="db-tick" style={{ left: `${(d / max) * 100}%` }}>{d === 0 ? "today" : `+${d}d`}</span>)}
          {shown.map((e, i) => <a key={i} className="db-dot" style={{ left: `${(e.days / max) * 100}%`, background: TOPIC_COLOR[e.topic], top: `${22 + (i % 3) * 14}px` }} title={`${e.date} · ${e.text}`} {...(e.link ? href(e.link) : {})} />)}
        </div>
        <div className="mx-slicers" style={{ border: 0, padding: "8px 0 0", background: "none" }}>
          <Pills title="Topic" opts={opts} active={topics} onToggle={flip} />
        </div>
      </section>
      {months.map((m) => (
        <section key={m} style={{ marginTop: 14 }}>
          <h3 className="db-month">{new Date(`${m}-01T12:00:00`).toLocaleDateString([], { month: "long", year: "numeric" })}</h3>
          <div className="db-cards">
            {shown.filter((e) => e.date.startsWith(m)).map((e, i) => (
              <article key={i} className={`db-card db-cd ${e.days <= 3 ? "hot" : e.days <= 7 ? "warm" : ""}`} style={{ ["--c" as string]: TOPIC_COLOR[e.topic] }}>
                <div className="db-days"><b className={e.days === 0 ? "today" : ""}>{e.days === 0 ? "Today" : e.days}</b>{e.days > 0 && <span>{e.days === 1 ? "day" : "days"}</span>}</div>
                <div>
                  <p className="mx-meta">{new Date(`${e.date}T12:00:00`).toLocaleDateString([], { weekday: "short", month: "short", day: "numeric" })}</p>
                  <p><Link url={e.link}>{e.text}</Link></p>
                  <span className="mx-chip cat" style={{ ["--cat" as string]: TOPIC_COLOR[e.topic] }}><i />{TOPIC_LABEL[e.topic]}</span>
                </div>
              </article>
            ))}
          </div>
        </section>
      ))}
    </Shell>
  );
}

// ---------- charts ----------
function LineChart({ points, labels, color, unit, avg, wide, height = 130 }: { points: (number | null)[]; labels: string[]; color: string; unit?: string; avg?: boolean; wide?: boolean; height?: number }) {
  const vals = points.filter((p): p is number => p !== null);
  if (vals.length < 2) return <p className="mx-meta">Not enough readings yet.</p>;
  const W = wide ? 660 : 340, L = 34, R = 8, T = 12, B = 22;
  const lo = Math.min(...vals), hi = Math.max(...vals), pad = (hi - lo || 1) * 0.15;
  const y0 = lo - pad, y1 = hi + pad;
  const x = (i: number) => L + (i / (points.length - 1)) * (W - L - R);
  const y = (v: number) => T + (1 - (v - y0) / (y1 - y0)) * (height - T - B);
  const mean = vals.reduce((a, b) => a + b, 0) / vals.length;
  let d = "", pen = false;
  points.forEach((p, i) => { if (p === null) { pen = false; return; } d += `${pen ? "L" : "M"}${x(i).toFixed(1)} ${y(p).toFixed(1)} `; pen = true; });
  const fmtv = (v: number) => (Number.isInteger(v) ? String(v) : v.toFixed(1));
  const last = points.map((p) => p !== null).lastIndexOf(true);
  return (
    <svg className="db-chart" viewBox={`0 0 ${W} ${height}`} role="img" aria-label={`Line chart, ${fmtv(lo)} to ${fmtv(hi)} ${unit ?? ""}`}>
      {[lo, (lo + hi) / 2, hi].map((v, i) => <g key={i}><line x1={L} x2={W - R} y1={y(v)} y2={y(v)} className="db-grid" /><text x={L - 6} y={y(v) + 4} textAnchor="end" className="db-axis">{fmtv(v)}</text></g>)}
      {avg && <><line x1={L} x2={W - R} y1={y(mean)} y2={y(mean)} stroke={color} strokeDasharray="4 4" opacity=".5" /><text x={W - R} y={y(mean) - 4} textAnchor="end" className="db-axis">avg {fmtv(mean)}</text></>}
      <path d={d} fill="none" stroke={color} strokeWidth="2.2" strokeLinejoin="round" strokeLinecap="round" />
      {points.map((p, i) => p !== null && <circle key={i} cx={x(i)} cy={y(p)} r={i === last ? 4 : 2.6} fill={color} stroke="var(--bg1, #0f2630)" strokeWidth="1.5"><title>{`${labels[i]}: ${fmtv(p)} ${unit ?? ""}`}</title></circle>)}
      {[0, Math.floor((points.length - 1) / 2), points.length - 1].map((i) => <text key={i} x={x(i)} y={height - 5} textAnchor={i === 0 ? "start" : i === points.length - 1 ? "end" : "middle"} className="db-axis">{labels[i]}</text>)}
    </svg>
  );
}

const when = (iso: string) => new Date(iso);
const dayLabel = (iso: string) => when(`${iso.slice(0, 10)}T12:00:00`).toLocaleDateString([], { weekday: "short", month: "short", day: "numeric" });

// ---------- Workflows ----------
type WF = { id: string; name: string; active: boolean; link: string; schedule?: string | null; last_status?: string | null; last_ago?: string | null; next?: string | null; runs: number };
type WData = { workflows: WF[]; by_day: { back: number; ok: number; failed: number }[]; recent: { workflow: string; status: string; ago: string }[]; failing: number };
const RUNNABLE: [string, string][] = [["News Digest", "news-digest"], ["Leads Scan", "leads-scan"], ["Market Watch", "market-watch"]];
function RunButton({ name }: { name: string }) {
  const slug = RUNNABLE.find(([n]) => name.startsWith(n))?.[1];
  const [state, setState] = useState<"idle" | "busy" | "started" | string>("idle");
  if (!slug) return null;
  const go = async (force: boolean) => {
    if (force && !window.confirm("Run again even though today's note exists? It will be overwritten.")) return;
    setState("busy");
    try { await runWorkflow(slug, force); setState("started"); } catch (e) { setState(String((e as Error).message || e)); }
  };
  return (
    <p className="mx-meta">
      <button type="button" className="loki-chipbtn" disabled={state === "busy"} onClick={() => go(false)}>{state === "busy" ? "Starting…" : "Run now"}</button>{" "}
      <button type="button" className="loki-chipbtn" disabled={state === "busy"} onClick={() => go(true)} title="Run even if today's note already exists">Re-run</button>
      {state === "started" && <em className="db-dim"> Started. It takes a few minutes; check the note when it finishes.</em>}
      {state !== "idle" && state !== "busy" && state !== "started" && <em className="db-dim"> {state}</em>}
    </p>
  );
}
const wfState = (w: WF) => (!w.last_status ? "warn" : w.last_status === "success" ? "ok" : "fail");

function Workflows({ b, onAsk }: { b: Briefing; onAsk: (p: string) => void }) {
  const d = b.data as WData;
  const active = d.workflows.filter((w) => w.active), inactive = d.workflows.filter((w) => !w.active);
  const total = d.by_day.reduce((n, x) => n + x.ok + x.failed, 0);
  const max = Math.max(1, ...d.by_day.map((x) => x.ok + x.failed));
  return (
    <Shell b={b} onAsk={onAsk}>
      <Kpis items={[
        { n: `${active.length}/${d.workflows.length}`, label: "Workflows active" },
        { n: d.failing, label: "Failing", tone: d.failing ? "bad" : "" },
        { n: total, label: "Runs in 7 days" },
        { n: active.filter((w) => w.last_status).length, label: "Have run recently" },
      ]} />
      <div className="mx-charts">
        <section className="mx-card">
          <h3>Runs per day</h3>
          <div className="db-cols" role="img" aria-label="Runs per day, successes and failures">
            {d.by_day.map((x) => (
              <div key={x.back} className="db-col" title={`${x.ok} ok, ${x.failed} failed`}>
                <div className="db-colbar" style={{ height: `${((x.ok + x.failed) / max) * 100}%` }}>
                  {x.failed > 0 && <span style={{ flexGrow: x.failed, background: "var(--bad)" }} />}
                  {x.ok > 0 && <span style={{ flexGrow: x.ok, background: "var(--good)" }} />}
                </div>
                <small>{x.back === 0 ? "today" : `-${x.back}d`}</small><em>{x.ok + x.failed}</em>
              </div>
            ))}
          </div>
        </section>
        <section className="mx-card">
          <h3>Latest runs</h3>
          <div className="db-items">{d.recent.length === 0 && <p className="mx-meta">No runs recorded.</p>}{d.recent.map((r, i) => <div key={i} className="db-item"><span className={`loki-dot loki-dot-${r.status === "success" ? "ok" : "fail"}`} aria-hidden="true" /><span>{r.workflow}</span><em className="db-dim">{r.status} · {r.ago}</em></div>)}</div>
        </section>
      </div>
      <div style={{ marginTop: 14 }}><NewWorkflow /></div>
      <h3 className="db-month" style={{ marginTop: 14 }}>Active workflows</h3>
      <div className="db-cards">
        {active.map((w) => (
          <article key={w.id} className="db-card" style={{ ["--c" as string]: wfState(w) === "ok" ? "var(--good)" : wfState(w) === "fail" ? "var(--bad)" : "var(--warn)" }}>
            <h4><span className={`loki-dot loki-dot-${wfState(w)}`} aria-hidden="true" /> <a {...href(w.link)}>{w.name}</a></h4>
            <div className="mx-tags">{w.schedule && <span className="mx-chip">{w.schedule}</span>}<span className="mx-chip">{wfState(w) === "ok" ? "Last run OK" : wfState(w) === "fail" ? `Last run ${w.last_status}` : "No runs yet"}</span></div>
            <p className="mx-meta">{w.last_ago ? `Last run ${w.last_ago}` : "Never run in the records n8n returned"}{w.next ? ` · next ${w.next}` : ""}{w.runs > 1 ? ` · ${w.runs} recent runs` : ""}</p>
            <RunButton name={w.name} />
          </article>
        ))}
      </div>
      {inactive.length > 0 && (
        <section className="mx-card" style={{ marginTop: 14 }}>
          <h3>Inactive <small>{inactive.length}</small></h3>
          <div className="mx-tags">{inactive.map((w) => <a key={w.id} className="mx-chip" {...href(w.link)}>{w.name}</a>)}</div>
        </section>
      )}
    </Shell>
  );
}

// ---------- Network & security ----------
type NData = { issues: number; tiles: { label: string; value: string; sub: string; state: string }[]; devices: { name: string; online: boolean; state: string }[] };

function Network({ b, onAsk }: { b: Briefing; onAsk: (p: string) => void }) {
  const d = b.data as NData;
  const online = d.devices.filter((x) => x.online).length;
  return (
    <Shell b={b} onAsk={onAsk}>
      <Kpis items={[
        { n: d.issues === 0 ? "All quiet" : d.issues, label: d.issues === 0 ? "Nothing to look at" : "Things to look at", tone: d.issues ? "bad" : "" },
        { n: `${online}/${d.devices.length}`, label: "Tailnet devices reachable", tone: online < d.devices.length ? "warn" : "" },
      ]} />
      <div className="db-cards" style={{ marginTop: 4 }}>
        {d.tiles.map((t) => (
          <article key={t.label} className="db-card" style={{ ["--c" as string]: t.state === "ok" ? "var(--good)" : t.state === "warn" ? "var(--warn)" : "#6f8fb0" }}>
            <p className="mx-meta">{t.label}</p>
            <div className="db-big">{t.value}</div>
            <div className="mx-tags"><span className={`mx-chip ${t.state === "warn" ? "slip" : ""}`}>{t.state === "ok" ? "✓ OK" : t.state === "warn" ? "▲ Check it" : "● Info"}</span></div>
            {t.sub && <p className="mx-meta">{t.sub}</p>}
          </article>
        ))}
      </div>
      <section className="mx-card" style={{ marginTop: 14 }}>
        <h3>Devices on the tailnet</h3>
        <div className="db-cards" style={{ gridTemplateColumns: "repeat(auto-fill, minmax(190px, 1fr))" }}>
          {d.devices.map((x) => <div key={x.name} className="db-dev"><span className={`loki-dot loki-dot-${x.online ? "ok" : "fail"}`} aria-hidden="true" /><b>{x.name}</b><em>{x.online ? x.state : "offline"}</em></div>)}
        </div>
      </section>
      {b.link && <p style={{ marginTop: 14 }}><a className="loki-chipbtn" {...href(b.link)}>Open monitoring ↗</a></p>}
    </Shell>
  );
}

// ---------- Weather ----------
type WxData = { now: { temp: number; feels: number; wind: number; cond: string }; hourly: { time: string; temp: number; pop: number; cond: string }[]; daily: { date: string; hi: number; lo: number; pop: number; cond: string; sunrise: string; sunset: string }[]; place: string };

function Weather({ b, onAsk }: { b: Briefing; onAsk: (p: string) => void }) {
  const d = b.data as WxData;
  const t0 = d.daily[0];
  const hl = d.hourly.map((h) => when(h.time).toLocaleTimeString([], { hour: "numeric" }));
  const clock = (iso: string) => when(iso).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
  return (
    <Shell b={b} onAsk={onAsk}>
      <div className="mx-charts">
        <section className="mx-card db-hero">
          <p className="mx-meta">{d.place}</p>
          <div className="db-bigtemp">{Math.round(d.now.temp)}°</div>
          <p>{d.now.cond} · feels like {Math.round(d.now.feels)}° · wind {Math.round(d.now.wind)} mph</p>
          {t0 && <p className="mx-meta">High {Math.round(t0.hi)}° · low {Math.round(t0.lo)}° · rain {t0.pop}% · sunrise {clock(t0.sunrise)} · sunset {clock(t0.sunset)}</p>}
        </section>
        <section className="mx-card" style={{ gridColumn: "span 2" }}>
          <h3>Next 36 hours: temperature (°F)</h3>
          <LineChart wide points={d.hourly.map((h) => Math.round(h.temp))} labels={hl} color="var(--accent)" unit="°F" />
          <h3 style={{ marginTop: 10 }}>Chance of rain</h3>
          <div className="db-cols tight" style={{ height: 56 }} role="img" aria-label="Chance of rain by hour">
            {d.hourly.map((h, i) => <div key={i} className="db-col" title={`${hl[i]}: ${h.pop}% ${h.cond}`}><div className="db-colbar" style={{ height: `${Math.max(2, h.pop)}%` }}><span style={{ background: "#3987e5" }} /></div></div>)}
          </div>
        </section>
      </div>
      <div className="db-cards" style={{ marginTop: 14 }}>
        {d.daily.map((x) => (
          <article key={x.date} className="db-card" style={{ ["--c" as string]: "#3987e5" }}>
            <h4>{dayLabel(x.date)}</h4>
            <div className="db-big">{Math.round(x.hi)}° <small>/ {Math.round(x.lo)}°</small></div>
            <p>{x.cond}</p><p className="mx-meta">Rain {x.pop}%</p>
          </article>
        ))}
      </div>
    </Shell>
  );
}

// ---------- Rhythm ----------
// Built from the profile's list of tracked daily-note properties, so it shows whatever you track.
type RProp = { key: string; label: string; type: "number" | "choice" | "text"; unit: string; options: string[]; role: string; summary: string; chart: string };
type RDay = { date: string; values: Record<string, number | string | null> };
const RAMP = ["#2f9e4a", "#e6a04b", "#e66767"]; // best, middle, worst (options are listed best to worst)
const LINE_COLORS = ["#3987e5", "#d55181", "#c98500", "#2f9e4a", "#9085e9", "#199e70"];
const optColor = (options: string[], v: string) => {
  const i = options.findIndex((o) => o.toLowerCase() === v.toLowerCase());
  return i < 0 ? undefined : RAMP[options.length <= 1 ? 0 : Math.round((i * 2) / (options.length - 1))];
};
const avgOf = (v: (number | null)[]) => { const x = v.filter((n): n is number => n !== null); return x.length ? x.reduce((a, b) => a + b, 0) / x.length : null; };
const numAt = (d: RDay, k: string) => (typeof d.values[k] === "number" ? (d.values[k] as number) : null);
const textAt = (d: RDay, k: string) => (typeof d.values[k] === "string" ? (d.values[k] as string) : "");

function Strip({ prop, days }: { prop: RProp; days: RDay[] }) {
  return (
    <div className="db-strip-row">
      <span title={prop.options.join(" → ")}>{prop.label}</span>
      <div className="db-strip">{days.map((d) => { const v = textAt(d, prop.key); return <i key={d.date} style={{ background: optColor(prop.options, v) ?? "var(--field)" }} title={`${dayLabel(d.date)}: ${v || "not recorded"}`}>{v ? v[0].toUpperCase() : ""}</i>; })}</div>
    </div>
  );
}

function Rhythm({ b, onAsk }: { b: Briefing; onAsk: (p: string) => void }) {
  const { props, days } = b.data as { props: RProp[]; days: RDay[] };
  const labels = days.map((d) => when(`${d.date}T12:00:00`).toLocaleDateString([], { month: "short", day: "numeric" }));
  const numbers = props.filter((p) => p.type === "number"), choices = props.filter((p) => p.type === "choice");
  const kpis: { n: string | number; label: string }[] = [];
  for (const p of props) {
    const unit = p.unit ? ` ${p.unit}` : "";
    if (p.type === "number") {
      const vals = days.map((d) => numAt(d, p.key)).filter((x): x is number => x !== null);
      if (p.summary === "average") kpis.push({ n: vals.length ? `${avgOf(vals)!.toFixed(1)}${unit}` : "—", label: `Average ${p.label.toLowerCase()}` });
      else if (p.summary === "trend") kpis.push({ n: vals.length > 1 ? `${(vals[vals.length - 1] - vals[0]).toFixed(1)}${unit}` : "—", label: `${p.label} change` });
      else if (p.summary === "average_nonzero") { const a = avgOf(vals.filter((x) => x > 0)); kpis.push({ n: a !== null ? a.toFixed(1) : "—", label: `${p.label} on days with any` }); }
    } else if (p.role === "energy") {
      kpis.push({ n: p.options.map((o) => days.filter((d) => textAt(d, p.key).toLowerCase() === o.toLowerCase()).length).join("/"), label: p.options.join(" / ") });
    }
  }
  return (
    <Shell b={b} onAsk={onAsk}>
      {kpis.length > 0 && <Kpis items={kpis.slice(0, 4)} />}
      {numbers.length > 0 && (
        <div className="mx-charts">
          {numbers.map((p, i) => {
            const max = Math.max(3, ...days.map((d) => numAt(d, p.key) ?? 0));
            return (
              <section key={p.key} className="mx-card">
                <h3>{p.label}{p.unit ? ` (${p.unit})` : ""}</h3>
                {p.chart === "bars" ? (
                  <div className="db-cols" style={{ height: 90 }} role="img" aria-label={`${p.label} per day`}>
                    {days.map((d, k) => { const v = numAt(d, p.key); return <div key={d.date} className="db-col" title={`${labels[k]}: ${v ?? "not recorded"}`}><div className="db-colbar" style={{ height: `${((v ?? 0) / max) * 100}%` }}><span style={{ background: "var(--good)" }} /></div><em>{v ?? ""}</em></div>; })}
                  </div>
                ) : (
                  <LineChart points={days.map((d) => numAt(d, p.key))} labels={labels} color={LINE_COLORS[i % LINE_COLORS.length]} unit={p.unit} avg={p.summary === "average"} />
                )}
              </section>
            );
          })}
        </div>
      )}
      {choices.length > 0 && (
        <section className="mx-card" style={{ marginTop: 14 }}>
          <h3>How the days felt <small>{labels[0]} to {labels[labels.length - 1]}</small></h3>
          {choices.map((p) => <Strip key={p.key} prop={p} days={days} />)}
          <div className="db-legend"><span><i style={{ background: RAMP[0] }} />Best</span><span><i style={{ background: RAMP[1] }} />Middle</span><span><i style={{ background: RAMP[2] }} />Worst</span><span><i style={{ background: "var(--field)", border: "1px solid var(--line)" }} />Not recorded</span></div>
        </section>
      )}
    </Shell>
  );
}

// ---------- Signals ----------
type SigItem = { title: string; text: string; link?: string | null; domain: string };

function Signals({ b, onAsk }: { b: Briefing; onAsk: (p: string) => void }) {
  const items = (b.data as { items: SigItem[] }).items;
  const [dom, setDom] = useState<Set<string>>(new Set());
  const domains = Array.from(new Set(items.map((i) => i.domain || "no link")));
  const opts: Opt[] = domains.map((d) => ({ key: d, label: d, n: items.filter((i) => (i.domain || "no link") === d).length, color: "#3987e5" }));
  const flip = (k: string) => setDom((s) => { const n = new Set(s); n.has(k) ? n.delete(k) : n.add(k); return n; });
  const shown = items.filter((i) => dom.size === 0 || dom.has(i.domain || "no link"));
  return (
    <Shell b={b} onAsk={onAsk}>
      <Kpis items={[{ n: items.length, label: "Signals today", onClick: () => setDom(new Set()) }, { n: domains.length, label: "Sources" }, { n: items.filter((i) => i.link).length, label: "With a source link" }]} />
      {domains.length > 1 && <section className="mx-card"><h3>By source</h3><BarList opts={opts} active={dom} onToggle={flip} /></section>}
      <div className="db-cards" style={{ marginTop: 14 }}>
        {shown.map((i, k) => (
          <article key={k} className="db-card" style={{ ["--c" as string]: "#3987e5" }}>
            <h4>{i.title}</h4>
            <p>{i.text}</p>
            <div className="mx-tags">{i.domain && <span className="mx-chip">{i.domain}</span>}{i.link && <a className="loki-chipbtn" {...href(i.link)}>Read the source ↗</a>}</div>
          </article>
        ))}
      </div>
    </Shell>
  );
}

// ---------- Saved ----------
type SavedItem = { name: string; folder: string; at?: number | null; link: string };
const FOLDER_COLOR = ["#9085e9", "#199e70", "#d55181", "#c98500", "#3987e5", "#2f9e4a"];

function Saved({ b, onAsk }: { b: Briefing; onAsk: (p: string) => void }) {
  const d = b.data as { items: SavedItem[]; inbox_all: number; inbox_old: number };
  const [folder, setFolder] = useState<Set<string>>(new Set());
  const folders = Array.from(new Set(d.items.map((i) => i.folder || "Sources")));
  const col = (f: string) => FOLDER_COLOR[folders.indexOf(f) % FOLDER_COLOR.length];
  const opts: Opt[] = folders.map((f) => ({ key: f, label: f, n: d.items.filter((i) => (i.folder || "Sources") === f).length, color: col(f) }));
  const flip = (k: string) => setFolder((s) => { const n = new Set(s); n.has(k) ? n.delete(k) : n.add(k); return n; });
  const shown = d.items.filter((i) => folder.size === 0 || folder.has(i.folder || "Sources"));
  const day = (t?: number | null) => (t ? new Date(t * 1000).toLocaleDateString([], { month: "short", day: "numeric" }) : "");
  return (
    <Shell b={b} onAsk={onAsk}>
      <Kpis items={[
        { n: d.items.length, label: "Saved in the last 14 days", onClick: () => setFolder(new Set()) },
        { n: d.inbox_all, label: "Waiting in the Inbox" },
        { n: d.inbox_old, label: "Inbox items over 48 hours", tone: d.inbox_old ? "warn" : "" },
      ]} />
      {d.inbox_old > 0 && <p className="mx-notice" style={{ color: "var(--warn)" }}>{d.inbox_old} inbox item{d.inbox_old === 1 ? " is" : "s are"} past the 48-hour rule. Run /inbox-triage when you have a block for it.</p>}
      {folders.length > 1 && <section className="mx-card"><h3>By section</h3><BarList opts={opts} active={folder} onToggle={flip} /></section>}
      <div className="db-cards" style={{ marginTop: 14 }}>
        {shown.map((i, k) => (
          <article key={k} className="db-card" style={{ ["--c" as string]: col(i.folder || "Sources") }}>
            <h4><a {...href(i.link)}>{i.name}</a></h4>
            <div className="mx-tags"><span className="mx-chip cat" style={{ ["--cat" as string]: col(i.folder || "Sources") }}><i />{i.folder || "Sources"}</span><span className="mx-chip">{day(i.at)}</span></div>
          </article>
        ))}
      </div>
    </Shell>
  );
}

// ---------- Zotero ----------
type ZIt = { title: string; kind: string; authors: string; added?: string | null; added_at?: number | null; link?: string | null };
const KIND_LABEL: Record<string, string> = { journalArticle: "Journal article", webpage: "Web page", book: "Book", report: "Report", conferencePaper: "Conference paper", document: "Document", thesis: "Thesis", preprint: "Preprint" };

function Zotero({ b, onAsk }: { b: Briefing; onAsk: (p: string) => void }) {
  const d = b.data as { items: ZIt[]; week: number };
  const [kind, setKind] = useState<Set<string>>(new Set());
  const kinds = Array.from(new Set(d.items.map((i) => i.kind)));
  const opts: Opt[] = kinds.map((k, n) => ({ key: k, label: KIND_LABEL[k] ?? k, n: d.items.filter((i) => i.kind === k).length, color: FOLDER_COLOR[n % FOLDER_COLOR.length] }));
  const flip = (k: string) => setKind((s) => { const n = new Set(s); n.has(k) ? n.delete(k) : n.add(k); return n; });
  const shown = d.items.filter((i) => kind.size === 0 || kind.has(i.kind));
  return (
    <Shell b={b} onAsk={onAsk}>
      <Kpis items={[{ n: d.items.length, label: "Recently added", onClick: () => setKind(new Set()) }, { n: d.week, label: "Added this week" }, { n: kinds.length, label: "Kinds of item" }]} />
      {kinds.length > 1 && <section className="mx-card"><h3>By type</h3><BarList opts={opts} active={kind} onToggle={flip} /></section>}
      <div className="db-cards" style={{ marginTop: 14 }}>
        {shown.map((i, k) => (
          <article key={k} className="db-card" style={{ ["--c" as string]: FOLDER_COLOR[kinds.indexOf(i.kind) % FOLDER_COLOR.length] }}>
            <h4>{i.link ? <a {...href(i.link)}>{i.title}</a> : i.title}</h4>
            {i.authors && <p className="mx-meta">{i.authors}</p>}
            <div className="mx-tags"><span className="mx-chip">{KIND_LABEL[i.kind] ?? i.kind}</span>{i.added && <span className="mx-chip">added {i.added}</span>}</div>
          </article>
        ))}
      </div>
    </Shell>
  );
}

// ---------- Loki ----------
type LokiData = { engine: string; model: { loaded?: boolean; name?: string; vram_gb?: number; context_k?: number } | null; mcp: { up: number; total: number; tools: number; down: string[] }; context: { window_k: number; autocompact: boolean }; staged_fixes: number; step_limits: { ordinary: number; routine: number } };

function LokiCard({ b, onAsk }: { b: Briefing; onAsk: (p: string) => void }) {
  const d = b.data as LokiData;
  const tiles: { label: string; value: string; sub: string; state: "ok" | "warn" | "info" }[] = [
    { label: "Default engine", value: d.engine, sub: "Chat uses this unless you pick another", state: "info" },
    { label: "Local model", value: d.model?.loaded ? "Loaded" : "Idle", sub: d.model?.loaded ? `${d.model.name} · ${d.model.vram_gb} GB on the GPU · ${d.model.context_k}k context` : "It loads on the next message", state: d.model?.loaded ? "ok" : "info" },
    { label: "Tools (MCP)", value: `${d.mcp.up}/${d.mcp.total}`, sub: d.mcp.down.length ? `Not connected: ${d.mcp.down.join(", ")}` : `${d.mcp.tools} tools available`, state: d.mcp.down.length ? "warn" : "ok" },
    { label: "Context window", value: `${d.context.window_k}k`, sub: `Autocompact ${d.context.autocompact ? "on" : "off"}`, state: "info" },
    { label: "Steps per turn", value: `${d.step_limits.ordinary} / ${d.step_limits.routine}`, sub: "Ordinary / routines. Change it with /steps", state: "info" },
    { label: "Waiting on you", value: String(d.staged_fixes), sub: d.staged_fixes ? "Fixes ready to review on the Improve tab" : "Nothing to review", state: d.staged_fixes ? "warn" : "ok" },
  ];
  return (
    <Shell b={b} onAsk={onAsk}>
      <div className="db-cards">
        {tiles.map((t) => (
          <article key={t.label} className="db-card" style={{ ["--c" as string]: t.state === "ok" ? "var(--good)" : t.state === "warn" ? "var(--warn)" : "#6f8fb0" }}>
            <p className="mx-meta">{t.label}</p>
            <div className="db-big">{t.value}</div>
            <p className="mx-meta">{t.sub}</p>
          </article>
        ))}
      </div>
      <p style={{ marginTop: 14 }}><button type="button" className="loki-chipbtn" onClick={() => navigate("/improve")}>Open the Improve tab →</button></p>
    </Shell>
  );
}

// ---------- Finance (figures shown only on this page; never sent to a model) ----------
type FSync = {
  ledger?: { ts?: number | null; ok?: boolean | null; err?: string; tx?: number; running?: boolean } | null;
  simplefin?: { connected?: boolean | null; last_ok?: number | null; state?: { ts?: number | null; ok?: boolean | null; err?: string; running?: boolean } | null } | null;
  loki?: { fetched_at: number; last_try: number; error: string; now: number };
};
type FData = {
  month: { spent?: number | null; budget?: number | null; income?: number | null };
  networth: { net?: number | null; assets?: number | null; liabilities?: number | null };
  bills: { date: string; kind: string; name: string; amount?: number | null }[];
  uncalibrated: number;
  accounts?: { label: string; kind: string; balance?: number | null; last_tx?: string | null }[];
  manual?: { name: string; kind: string; sub?: string | null; balance?: number | null; change30?: number | null; as_of?: string | null }[];
  history?: { date: string; assets: number; liabilities: number; net: number }[];
  payday?: { paycheck?: { name?: string; amount?: number } | null; next_payday?: string | null; days?: number; count?: number; total?: number; cash_now?: number | null; projected?: number | null; per_day?: number | null; uncalibrated_cash?: number };
  spending?: { rows: { category: string; spent: number; budget?: number | null; avg3: number; prev: number }[]; series: { ym: string; spent: number; income: number }[] };
  sync?: FSync;
};
const money = (v?: number | null) => (v === null || v === undefined ? "—" : `${v < 0 ? "-" : ""}$${Math.abs(Math.round(v)).toLocaleString()}`);
const since = (secs: number) => (secs < 90 ? "just now" : secs < 5400 ? `${Math.round(secs / 60)} min ago` : secs < 172_800 ? `${Math.round(secs / 3600)} h ago` : `${Math.round(secs / 86_400)} d ago`);

/** How fresh the numbers are, link by link: the Ledger's mirror of Firefly, the bank link, and Loki's own copy. */
function SyncBar({ s }: { s?: FSync }) {
  const refresh = useContext(RefreshContext);
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState("");
  if (!s?.loki) return null;
  const now = s.loki.now;
  const parts: { label: string; text: string; state: "ok" | "warn" | "bad" | "info" }[] = [];
  const lok = now - s.loki.fetched_at;
  parts.push(s.loki.error ? { label: "Loki's copy", text: `${since(lok)}; the last refresh failed (${s.loki.error})`, state: lok > 1800 ? "bad" : "warn" } : { label: "Loki's copy", text: `refreshed ${since(lok)}`, state: lok > 1800 ? "warn" : "ok" });
  const L = s.ledger;
  if (L) {
    const age = L.ts ? now - L.ts : null;
    parts.push(L.ok === false ? { label: "Ledger and Firefly", text: `sync failing${L.err ? `: ${L.err}` : ""}`, state: "bad" } : { label: "Ledger and Firefly", text: age === null ? "no sync recorded yet" : `synced ${since(age)}${L.running ? " (running now)" : ""}`, state: age !== null && age > 3600 ? "warn" : "ok" });
  }
  const F = s.simplefin;
  if (F) {
    if (!F.connected) parts.push({ label: "Bank link", text: "not connected (SimpleFIN)", state: "info" });
    else {
      const age = F.last_ok ? now - F.last_ok : null;
      parts.push(F.state?.ok === false ? { label: "Bank link", text: `last attempt failed${F.state.err ? `: ${F.state.err}` : ""}`, state: "bad" } : { label: "Bank link", text: age === null ? "connected, no sync yet" : `synced ${since(age)}`, state: age !== null && age > 26 * 3600 ? "warn" : "ok" });
    }
  }
  async function syncNow() {
    setBusy(true);
    setMsg("");
    try {
      setMsg(await financeSyncNow());
      refresh();
    } catch (e) {
      setMsg(String(e instanceof Error ? e.message : e));
    }
    setBusy(false);
  }
  return (
    <section className="mx-card fn-sync">
      <div className="fn-sync-row">
        {parts.map((p) => <span key={p.label} className={`fn-pill fn-${p.state}`}><i />{p.label}: {p.text}</span>)}
        <button type="button" className="loki-chipbtn" disabled={busy} onClick={() => void syncNow()}>{busy ? "Syncing (about 10 s)…" : "Sync now"}</button>
      </div>
      {msg && <p className="mx-meta">{msg}</p>}
    </section>
  );
}

function Finance({ b }: { b: Briefing }) {
  const d = b.data as FData;
  const { spent = 0, budget = 0, income = 0 } = d.month;
  const used = budget ? Math.round(((spent ?? 0) / budget) * 100) : null;
  const top = Math.max(1, income ?? 0, spent ?? 0, budget ?? 0);
  const assets = d.networth.assets ?? 0, liab = Math.abs(d.networth.liabilities ?? 0);
  const daysTo = (iso: string) => Math.round((new Date(`${iso}T12:00:00`).getTime() - Date.now()) / 86_400_000);
  const hist = d.history ?? [];
  const cats = d.spending?.rows.filter((r) => r.spent > 0 || (r.budget ?? 0) > 0).slice(0, 12) ?? [];
  const catMax = Math.max(1, ...cats.map((r) => Math.max(r.spent, r.budget ?? 0)));
  const pd = d.payday;
  return (
    <Shell b={b}>
      <p className="mx-notice" style={{ color: "var(--text-dim)" }}>These figures are shown only on this page. Loki doesn't send them to any model.</p>
      <SyncBar s={d.sync} />
      <Kpis items={[
        { n: used !== null ? `${used}%` : "—", label: "Of the month's budget used", tone: used !== null && used > 100 ? "bad" : used !== null && used > 85 ? "warn" : "" },
        { n: money(spent), label: "Spent this month" },
        { n: money(income), label: "Income this month" },
        { n: money(d.networth.net), label: "Net worth" },
        { n: d.bills.length, label: "Bills in 7 days" },
      ]} />
      <div className="mx-charts">
        <section className="mx-card">
          <h3>This month</h3>
          {[["Income", income ?? 0, "var(--good)"], ["Spent", spent ?? 0, "var(--urg-priority, #e6a04b)"], ...(budget ? [["Budget", budget ?? 0, "#6f8fb0"]] : [])].map(([label, v, c]) => (
            <div key={label as string} className="db-dom"><span>{label as string}</span><span className="db-prog" style={{ margin: 0 }}><span style={{ width: `${((v as number) / top) * 100}%`, background: c as string }} /></span><em>{money(v as number)}</em></div>
          ))}
          {budget ? <p className="mx-meta">{used}% of the budget is spent{used !== null && used > 100 ? ", over by " + money((spent ?? 0) - budget) : ""}.</p> : <p className="mx-meta">No budget set for this month.</p>}
        </section>
        <section className="mx-card">
          <h3>Net worth</h3>
          <div className="db-big">{money(d.networth.net)}</div>
          <div className="db-cap" role="img" aria-label="Assets and liabilities">
            <span className="free" style={{ flexGrow: assets }}>Assets {money(assets)}</span>
            {liab > 0 && <span className="used" style={{ flexGrow: liab, background: "var(--bad)" }}>Debts {money(liab)}</span>}
          </div>
        </section>
        {hist.length > 1 && (
          <section className="mx-card" style={{ gridColumn: "span 2" }}>
            <h3>Net worth, month by month</h3>
            <LineChart wide points={hist.map((h) => Math.round(h.net))} labels={hist.map((h) => new Date(`${h.date}T12:00:00`).toLocaleDateString([], { month: "short", year: "2-digit" }))} color="#3987e5" unit="$" />
          </section>
        )}
      </div>

      {cats.length > 0 && (
        <section className="mx-card" style={{ marginTop: 14 }}>
          <h3>Spending by category this month</h3>
          <div className="fn-cats">
            {cats.map((r) => {
              const over = r.budget ? r.spent > r.budget : false;
              return (
                <div key={r.category} className="fn-cat">
                  <span>{r.category}</span>
                  <span className="db-prog" style={{ margin: 0 }} title={r.budget ? `Budget ${money(r.budget)}` : "No budget"}>
                    <span style={{ width: `${(r.spent / catMax) * 100}%`, background: over ? "var(--bad)" : "var(--urg-priority, #e6a04b)" }} />
                  </span>
                  <em>{money(r.spent)}{r.budget ? ` of ${money(r.budget)}` : ""}</em>
                  <small>{r.avg3 > 0 ? `3-month average ${money(r.avg3)}` : ""}</small>
                </div>
              );
            })}
          </div>
        </section>
      )}

      {pd && pd.next_payday && (
        <section className="mx-card" style={{ marginTop: 14 }}>
          <h3>Until the next payday</h3>
          <p>{pd.paycheck?.name ? `${pd.paycheck.name}, ` : ""}{pd.days === 0 ? "today" : pd.days === 1 ? "tomorrow" : `in ${pd.days} days`} ({new Date(`${pd.next_payday}T12:00:00`).toLocaleDateString([], { month: "short", day: "numeric" })}){pd.paycheck?.amount ? `, about ${money(pd.paycheck.amount)}` : ""}.</p>
          <p className="mx-meta">{pd.count ?? 0} bill{pd.count === 1 ? "" : "s"} due before then ({money(pd.total)}). Cash now {money(pd.cash_now)}{pd.projected !== null && pd.projected !== undefined ? `, about ${money(pd.projected)} after those bills` : ""}{pd.per_day !== null && pd.per_day !== undefined ? `, roughly ${money(pd.per_day)} a day to spend` : ""}.{pd.uncalibrated_cash ? ` ${pd.uncalibrated_cash} cash account${pd.uncalibrated_cash === 1 ? " has" : "s have"} no starting balance, so this is incomplete.` : ""}</p>
        </section>
      )}

      <div className="mx-charts" style={{ marginTop: 14 }}>
        {(d.accounts?.length ?? 0) > 0 && (
          <section className="mx-card">
            <h3>Accounts</h3>
            <div className="db-items">{d.accounts!.map((a) => <div key={a.label} className="db-item"><span>{a.label}</span><em className="db-dim">{a.kind}{a.last_tx ? ` · last activity ${new Date(`${a.last_tx}T12:00:00`).toLocaleDateString([], { month: "short", day: "numeric" })}` : ""} · <b>{money(a.balance)}</b></em></div>)}</div>
          </section>
        )}
        {(d.manual?.length ?? 0) > 0 && (
          <section className="mx-card">
            <h3>Investments and loans</h3>
            <div className="db-items">{d.manual!.map((m) => <div key={m.name} className="db-item"><span>{m.name}</span><em className="db-dim">{m.sub ?? m.kind}{m.change30 ? ` · ${m.change30 > 0 ? "+" : ""}${money(m.change30)} in 30 days` : ""} · <b>{money(m.balance)}</b></em></div>)}</div>
          </section>
        )}
        <section className="mx-card">
          <h3>Coming up</h3>
          {d.bills.length === 0 && <p className="mx-meta">Nothing due in the next 7 days.</p>}
          <div className="db-items">
            {d.bills.map((x, i) => { const n = daysTo(x.date); return (
              <div key={i} className="db-item"><span className={`mx-chip ${n <= 2 ? "slip" : ""}`}>{n <= 0 ? "Today" : n === 1 ? "Tomorrow" : `${n} days`}</span><span>{x.name}</span><em className="db-dim">{x.kind} · {money(x.amount)}</em></div>
            ); })}
          </div>
        </section>
      </div>
      {d.uncalibrated > 0 && <p className="mx-notice" style={{ color: "var(--warn)" }}>{d.uncalibrated} account{d.uncalibrated === 1 ? " has" : "s have"} no starting balance in the Ledger, so its totals may be off.</p>}
      {b.link && <p style={{ marginTop: 14 }}><a className="loki-chipbtn" {...href(b.link)}>Open the Ledger ↗</a></p>}
    </Shell>
  );
}

/** The dashboard for a briefing, when the server sent structured data for it. */
export function dashboardFor(b: Briefing, onAsk: (p: string) => void): ReactNode | null {
  if (!b.data) return null;
  switch (b.kind) {
    case "shape": return <Today b={b} onAsk={onAsk} />;
    case "week": return <Sprint b={b} onAsk={onAsk} />;
    case "backlog": return <Backlog b={b} onAsk={onAsk} />;
    case "countdowns": return <Countdowns b={b} onAsk={onAsk} />;
    case "workflows": return <Workflows b={b} onAsk={onAsk} />;
    case "network": return <Network b={b} onAsk={onAsk} />;
    case "weather": return <Weather b={b} onAsk={onAsk} />;
    case "rhythm": return <Rhythm b={b} onAsk={onAsk} />;
    case "signals": return <Signals b={b} onAsk={onAsk} />;
    case "saved": return <Saved b={b} onAsk={onAsk} />;
    case "zotero": return <Zotero b={b} onAsk={onAsk} />;
    case "loki": return <LokiCard b={b} onAsk={onAsk} />;
    case "finance": return <Finance b={b} />;
    default: return null;
  }
}
