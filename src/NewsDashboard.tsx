import { useCallback, useEffect, useMemo, useState } from "react";
import { BarList, Kpis, Pills, type Opt } from "./dash";
import ExtractPanel from "./ExtractPanel";
import RoutineBar from "./routines";
import Globe, { type GlobeFocus, type GlobePoint } from "./Globe";
import { newsGeoDay, newsGeoIndex, type GeoDay, type GeoEvent, type NeedsYou } from "./transport";

// Same region colors as the news page's globe, so the two read alike.
const COLORS: Record<string, string> = {
  Technology: "#8a63d2", "Defense Tech": "#d9534f", "Economics/Markets": "#3b82f6", Europe: "#22a3c9", "Western Hemisphere": "#2fb36d",
  "Indo-Pacific": "#e0a030", Africa: "#e07b39", "Global/Cross-Regional": "#c0392b", "Sovereignty & Indie Software": "#8c8cb0", "Ukraine War": "#f2d024",
};
const colour = (c: string) => COLORS[c] ?? "#8a96a0";
type Ev = GeoEvent & { id: string; date: string };
type Sets = { cat: Set<string>; src: Set<string>; prec: Set<string> };
const safeUrl = (u: string) => (/^https?:\/\//i.test(u) ? u : "#");

export default function NewsDashboard({ data, onAsk }: { data: NeedsYou | null; onAsk: (prompt: string) => void }) {
  const [dates, setDates] = useState<{ date: string; events: number }[] | null>(null);
  const [days, setDays] = useState<Record<string, GeoDay>>({});
  const [sel, setSel] = useState<Set<string>>(new Set());
  const [f, setF] = useState<Sets>({ cat: new Set(), src: new Set(), prec: new Set() });
  const [layers, setLayers] = useState({ fires: true, conflict: true, hazards: true });
  const [q, setQ] = useState("");
  const [picked, setPicked] = useState<string | null>(null);
  const [focus, setFocus] = useState<GlobeFocus | null>(null);
  const [spin, setSpin] = useState(false);
  const [error, setError] = useState("");

  const loadDay = useCallback(async (d: string) => {
    try {
      const day = await newsGeoDay(d);
      setDays((m) => ({ ...m, [d]: day }));
    } catch (e) {
      setError(String(e));
    }
  }, []);
  useEffect(() => {
    newsGeoIndex()
      .then((i) => {
        const list = [...i.dates].sort((a, b) => b.date.localeCompare(a.date));
        setDates(list);
        if (list[0]) {
          setSel(new Set([list[0].date]));
          void loadDay(list[0].date);
        }
      })
      .catch((e) => setError(String(e)));
  }, [loadDay]);

  const toggleDate = (d: string) => {
    setSel((s) => {
      const n = new Set(s);
      if (n.has(d)) n.delete(d);
      else {
        n.add(d);
        if (!days[d]) void loadDay(d);
      }
      return n;
    });
  };

  const latest = dates?.[0]?.date ?? "";
  const all: Ev[] = useMemo(() => Object.values(days).filter((d) => sel.has(d.date)).flatMap((d) => d.events.map((e, i) => ({ ...e, id: `${d.date}:${i}`, date: d.date }))), [days, sel]);
  const hay = (e: Ev) => `${e.title} ${e.bluf} ${e.place} ${e.source}`.toLowerCase();
  const passes = (e: Ev, skip?: keyof Sets) =>
    (skip === "cat" || f.cat.size === 0 || f.cat.has(e.category)) &&
    (skip === "src" || f.src.size === 0 || f.src.has(e.source)) &&
    (skip === "prec" || f.prec.size === 0 || f.prec.has(e.precision ?? "city")) &&
    (!q || hay(e).includes(q.toLowerCase()));
  const shown = useMemo(() => all.filter((e) => passes(e)), [all, f, q]); // eslint-disable-line react-hooks/exhaustive-deps
  const toggle = (k: keyof Sets, v: string) =>
    setF((s) => {
      const n = { ...s, [k]: new Set(s[k]) };
      n[k].has(v) ? n[k].delete(v) : n[k].add(v);
      return n;
    });
  const opts = (k: keyof Sets, val: (e: Ev) => string): Opt[] => {
    const m = new Map<string, number>();
    all.forEach((e) => m.set(val(e), m.get(val(e)) ?? 0));
    all.filter((e) => passes(e, k)).forEach((e) => m.set(val(e), (m.get(val(e)) ?? 0) + 1));
    return Array.from(m, ([key, n]) => ({ key, label: key, n, color: k === "cat" ? colour(key) : undefined })).sort((a, b) => b.n - a.n || a.label.localeCompare(b.label));
  };
  const catOpts = opts("cat", (e) => e.category);
  const srcOpts = opts("src", (e) => e.source || "?").slice(0, 10);
  const precOpts = opts("prec", (e) => e.precision ?? "city");

  const chosen = sel.size ? Array.from(sel).map((d) => days[d]).filter(Boolean) : [];
  const points: GlobePoint[] = useMemo(() => {
    const out: GlobePoint[] = shown.map((e) => ({ id: e.id, lon: e.lon, lat: e.lat, color: colour(e.category), kind: "event", faded: e.date !== latest }));
    chosen.forEach((d) => {
      if (layers.fires) (d.layers?.fires ?? []).forEach((x, i) => out.push({ id: `${d.date}:f${i}`, lon: x.lon, lat: x.lat, color: "#ff6a00", kind: "fire", radius: 8, faded: d.date !== latest }));
      if (layers.conflict) (d.layers?.conflict ?? []).forEach((x, i) => out.push({ id: `${d.date}:c${i}`, lon: x.lon, lat: x.lat, color: "#d62828", kind: "conflict", radius: 8 + Math.sqrt(x.deaths) * 1.4, faded: d.date !== latest }));
      if (layers.hazards) (d.layers?.hazards ?? []).forEach((x, i) => out.push({ id: `${d.date}:h${i}`, lon: x.lon, lat: x.lat, color: "#ffcc00", kind: "hazard", faded: d.date !== latest }));
    });
    return out;
  }, [shown, chosen, layers, latest]); // eslint-disable-line react-hooks/exhaustive-deps

  const layerCount = (k: "fires" | "conflict" | "hazards") => chosen.reduce((n, d) => n + (d.layers?.[k]?.length ?? 0), 0);
  const pickedEv = picked ? all.find((e) => e.id === picked) ?? null : null;
  const pickedLayer = (() => {
    if (!picked || pickedEv) return null;
    const m = picked.match(/^(.+):([fch])(\d+)$/);
    const day = m && days[m[1]];
    if (!m || !day) return null;
    const i = Number(m[3]);
    if (m[2] === "f") { const x = day.layers?.fires?.[i]; return x && { t: "Thermal anomaly (NASA FIRMS)", b: x.note, m: `${x.dates} · ${x.frp_mw} MW`, u: "" }; }
    if (m[2] === "c") { const x = day.layers?.conflict?.[i]; return x && { t: `UCDP conflict event: ${x.deaths} deaths (best estimate)`, b: `${x.where}. ${x.note}`, m: x.date, u: "" }; }
    const x = day.layers?.hazards?.[i];
    return x && { t: x.name, b: `GDACS ${x.alert} alert`, m: "", u: x.url ?? "" };
  })();
  const select = (e: Ev) => {
    setPicked(e.id);
    setFocus({ lon: e.lon, lat: e.lat, nonce: Date.now() });
  };
  const digest = data?.briefings?.find((b) => b.kind === "news");
  const ask = (e: Ev) => onAsk(`Give me the background on this news event and what it could mean for me. Use only this text; do not search anything.\n\n${e.title}\n${e.bluf}\n(${e.place}; ${e.source})`);

  if (!dates && !error) return <div className="loki-page"><p className="loki-home-note">Loading the news globe…</p></div>;
  return (
    <div className="loki-page mx">
      <div className="loki-page-head"><h2>News</h2></div>
      <p className="loki-home-note">Geocoded stories from your feeds, on a globe. Drag to rotate, scroll to zoom, click a dot for the story. Same data as the news page.</p>
      {error && <div className="loki-error">{error}</div>}
      <RoutineBar tab="news" />

      <Kpis items={[
        { n: shown.length, label: all.length === shown.length ? "Stories on the globe" : `Stories shown of ${all.length}`, onClick: () => { setF({ cat: new Set(), src: new Set(), prec: new Set() }); setQ(""); } },
        { n: new Set(shown.map((e) => e.category)).size, label: "Regions and topics" },
        { n: new Set(shown.map((e) => e.source)).size, label: "Sources" },
        { n: layerCount("hazards"), label: "Hazard alerts", tone: layerCount("hazards") ? "warn" : "" },
        { n: layerCount("conflict"), label: "Conflict events", tone: layerCount("conflict") ? "bad" : "" },
        { n: layerCount("fires"), label: "Thermal anomalies" },
      ]} />

      <div className="nw-main">
        <section className="nw-globe">
          <Globe points={points} selectedId={picked} focus={focus} spin={spin} onPick={setPicked} />
          {(pickedEv || pickedLayer) && (
            <div className="nw-card">
              <button type="button" className="mx-x" onClick={() => setPicked(null)} aria-label="Close">✕</button>
              {pickedEv ? (
                <>
                  <span className="mx-chip cat" style={{ ["--cat" as string]: colour(pickedEv.category) }}><i />{pickedEv.category}</span>
                  <h3>{pickedEv.title}</h3>
                  <p>{pickedEv.bluf}</p>
                  <p className="mx-meta">📍 {pickedEv.place}{pickedEv.precision === "approx" ? " (approximate)" : ""} · {pickedEv.source} · {pickedEv.date}</p>
                  <div className="mx-viewer-actions">
                    <a className="loki-chipbtn" href={safeUrl(pickedEv.url)} target="_blank" rel="noreferrer noopener">Source ↗</a>
                    <button type="button" className="loki-chipbtn" onClick={() => ask(pickedEv)}>Ask Loki about this</button>
                  </div>
                  <ExtractPanel key={pickedEv.id} source={{ source: "text", label: `a news story (${pickedEv.source})`, text: `${pickedEv.title}\n${pickedEv.bluf}\n${pickedEv.place}\n${pickedEv.date}` }} />
                </>
              ) : pickedLayer ? (
                <>
                  <h3>{pickedLayer.t}</h3>
                  <p>{pickedLayer.b}</p>
                  {pickedLayer.m && <p className="mx-meta">{pickedLayer.m}</p>}
                  {pickedLayer.u && <a className="loki-chipbtn" href={safeUrl(pickedLayer.u)} target="_blank" rel="noreferrer noopener">Report ↗</a>}
                </>
              ) : null}
            </div>
          )}
        </section>
        <section className="nw-side">
          <input className="nw-search" type="search" placeholder="Search stories, places, sources…" value={q} onChange={(e) => setQ(e.target.value)} />
          <div className="nw-list">
            {shown.length === 0 && <p className="loki-home-note">Nothing matches the filters.</p>}
            {shown.map((e) => (
              <button type="button" key={e.id} className={`nw-item ${picked === e.id ? "on" : ""}`} style={{ ["--cat" as string]: colour(e.category) }} onClick={() => select(e)}>
                <b>{e.title}</b>
                <small>{e.place} · {e.source}</small>
                <span className="mx-chip cat"><i />{e.category}</span>
              </button>
            ))}
          </div>
        </section>
      </div>

      <div className="mx-charts" style={{ marginTop: 14 }}>
        <section className="mx-card"><h3>By region and topic</h3><BarList opts={catOpts} active={f.cat} onToggle={(k) => toggle("cat", k)} /></section>
        <section className="mx-card"><h3>By source</h3><BarList opts={srcOpts} active={f.src} onToggle={(k) => toggle("src", k)} /></section>
      </div>

      <div className="mx-slicers">
        <div className="mx-slicers-head"><h3>Filter</h3></div>
        <div className="mx-slicer">
          <span className="mx-slicer-title">Day</span>
          {(dates ?? []).map((d) => (
            <button type="button" key={d.date} className={`mx-pill ${sel.has(d.date) ? "on" : ""}`} onClick={() => toggleDate(d.date)}>{d.date} <em>{d.events}</em></button>
          ))}
        </div>
        <Pills title="Region" opts={catOpts} active={f.cat} onToggle={(k) => toggle("cat", k)} />
        <Pills title="Precision" opts={precOpts} active={f.prec} onToggle={(k) => toggle("prec", k)} />
        <div className="mx-slicer">
          <span className="mx-slicer-title">Layers</span>
          {(["hazards", "conflict", "fires"] as const).map((k) => (
            <button type="button" key={k} className={`mx-pill ${layers[k] ? "on" : ""}`} onClick={() => setLayers((l) => ({ ...l, [k]: !l[k] }))}>{k === "fires" ? "Thermal anomalies" : k === "conflict" ? "Conflict events" : "Hazard alerts"} <em>{layerCount(k)}</em></button>
          ))}
          <button type="button" className={`mx-pill ${spin ? "on" : ""}`} onClick={() => setSpin((s) => !s)}>Slow spin</button>
        </div>
      </div>

      {digest && digest.rows.length > 0 && (
        <section className="mx-card" style={{ marginTop: 14 }}>
          <h3>Today's digest headlines</h3>
          <div className="loki-brief-rows">
            {digest.rows.map((r, i) => (
              <div className="loki-home-row" key={i}>
                <b>{r.link ? <a href={r.link} target="_blank" rel="noreferrer noopener">{r.head} ↗</a> : r.head}</b>
                {r.text && <span>{r.text}</span>}
              </div>
            ))}
          </div>
        </section>
      )}
    </div>
  );
}
