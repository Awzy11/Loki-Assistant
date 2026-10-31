import { useCallback, useEffect, useMemo, useState } from "react";
import { BarList, Kpis, Pills, type Opt } from "./dash";
import RoutineBar from "./routines";
import { leadsApi, leadsDownload, type Lead, type LeadDetail, type LeadFacets, type LeadFamily, type LeadStep } from "./transport";

type Dim = "kind" | "source" | "tool" | "status" | "process" | "effort" | "flags";
type Filters = { days: number; q: string } & Record<Dim, Set<string>>;
const EMPTY = (): Filters => ({ days: 30, q: "", kind: new Set(), source: new Set(), tool: new Set(), status: new Set(), process: new Set(), effort: new Set(), flags: new Set() });

const STATUSES = ["new", "interested", "replied", "won", "passed"];
const STATUS_COLOR: Record<string, string> = { new: "#6f8fb0", interested: "#e6a04b", replied: "#3987e5", won: "#2f9e4a", passed: "#6a757d" };
const KIND_LABEL: Record<string, string> = { hire: "Hire-me", fix: "Fix-it", promo: "Promo", skip: "Skip" };
const FLAG_LABEL: Record<string, string> = { budget: "States a budget", forme: "Also helps me", family: "In a solution family", unanswered: "Unanswered" };
const EFFORT_LABEL: Record<string, string> = { S: "Small", M: "Medium", L: "Large", "?": "Unknown" };
const cap = (s: string) => (s ? s[0].toUpperCase() + s.slice(1) : s);
const safeUrl = (u: string) => (/^https?:\/\//i.test(u) ? u : "#");
const day = (iso: string) => new Date(iso).toLocaleDateString([], { month: "short", day: "numeric" });

function query(f: Filters): string {
  const p = new URLSearchParams({ days: String(f.days) });
  (["kind", "source", "tool", "status", "process", "effort", "flags"] as Dim[]).forEach((d) => f[d].size && p.set(d, Array.from(f[d]).join(",")));
  if (f.q.trim()) p.set("q", f.q.trim());
  return p.toString();
}

export default function LeadsDashboard({ onAsk }: { onAsk: (prompt: string) => void }) {
  const [f, setF] = useState<Filters>(EMPTY);
  const [q, setQ] = useState("");
  const [items, setItems] = useState<Lead[] | null>(null);
  const [facets, setFacets] = useState<LeadFacets | null>(null);
  const [openId, setOpenId] = useState<number | null>(null);
  const [shown, setShown] = useState(40);
  const [error, setError] = useState("");
  const [note, setNote] = useState("");

  useEffect(() => {
    const t = setTimeout(() => setF((x) => (x.q === q ? x : { ...x, q })), 300);
    return () => clearTimeout(t);
  }, [q]);

  const load = useCallback(async () => {
    const qs = query(f);
    try {
      const [it, fa] = await Promise.all([leadsApi<{ items: Lead[] }>(`items?${qs}&limit=400`), leadsApi<LeadFacets>(`facets?${qs}`)]);
      setItems(it.items);
      setFacets(fa);
      setError("");
    } catch (e) {
      setError(String(e));
    }
  }, [f]);
  useEffect(() => void load(), [load]);

  const toggle = (d: Dim) => (k: string) => setF((x) => { const n = new Set(x[d]); n.has(k) ? n.delete(k) : n.add(k); return { ...x, [d]: n }; });
  const anyFilter = (["kind", "source", "tool", "status", "process", "effort", "flags"] as Dim[]).some((d) => f[d].size) || !!f.q;
  const opts = (d: Dim, label: (k: string) => string = cap, color?: (k: string) => string | undefined): Opt[] =>
    Object.entries(facets?.[d] ?? {}).sort((a, b) => b[1] - a[1]).map(([k, n]) => ({ key: k, label: label(k), n, color: color?.(k) }));
  const statusOpts: Opt[] = STATUSES.map((k) => ({ key: k, label: cap(k), n: facets?.status[k] ?? 0, color: STATUS_COLOR[k] }));

  async function refreshFeeds() {
    setNote("Asking the leads app to sync from Miniflux…");
    try {
      await leadsApi("refresh", { method: "POST" });
      setNote("Sync started. New posts appear in a minute or two; press Reload then.");
    } catch (e) {
      setNote(String(e));
    }
  }
  const visible = useMemo(() => (items ?? []).slice(0, shown), [items, shown]);
  const total = facets?.total ?? items?.length ?? 0;

  return (
    <div className="loki-page mx">
      <div className="loki-page-head">
        <h2>Leads</h2>
        <div className="mx-head-actions">
          <button type="button" className="loki-chipbtn" onClick={() => void load()}>Reload</button>
          <button type="button" className="loki-chipbtn" onClick={() => void refreshFeeds()}>Sync from feeds</button>
          <button type="button" className="loki-chipbtn" onClick={() => void leadsDownload(`export?${query(f)}&fmt=csv`, "leads.csv").catch((e) => setError(String(e)))}>Export CSV</button>
          <button type="button" className="loki-chipbtn" onClick={() => void leadsDownload(`export?${query(f)}&fmt=md`, "leads.md").catch((e) => setError(String(e)))}>Export report</button>
          <button type="button" className="loki-chipbtn" onClick={() => void leadsDownload(`export?${query(f)}&fmt=json`, "leads.json").catch((e) => setError(String(e)))}>Export JSON</button>
        </div>
      </div>
      <p className="loki-home-note">Automation requests from forums, sorted and ranked. Nothing is posted anywhere: reply drafts are text you send yourself. Same data and controls as the leads app.</p>
      {error && <div className="loki-error">{error}</div>}
      {note && <div className="mx-notice">{note}</div>}
      <RoutineBar tab="leads" />

      <Kpis items={[
        { n: total, label: `Posts in the last ${f.days} days`, onClick: () => { setF(EMPTY()); setQ(""); } },
        { n: facets?.kind.hire ?? 0, label: "Hire-me", tone: "warn", onClick: () => setF({ ...EMPTY(), kind: new Set(["hire"]) }) },
        { n: facets?.kind.fix ?? 0, label: "Fix-it", onClick: () => setF({ ...EMPTY(), kind: new Set(["fix"]) }) },
        { n: facets?.flags.forme ?? 0, label: "Also help me", onClick: () => setF({ ...EMPTY(), flags: new Set(["forme"]) }) },
        { n: (facets?.status.interested ?? 0) + (facets?.status.replied ?? 0), label: "In conversation", onClick: () => setF({ ...EMPTY(), status: new Set(["interested", "replied"]) }) },
      ]} />

      <div className="mx-charts">
        <section className="mx-card"><h3>Pipeline</h3><BarList opts={statusOpts} active={f.status} onToggle={toggle("status")} /></section>
        <section className="mx-card"><h3>By platform</h3><BarList opts={opts("source", (k) => k, () => "#3987e5").slice(0, 8)} active={f.source} onToggle={toggle("source")} /></section>
        <section className="mx-card"><h3>By tool</h3><BarList opts={opts("tool", (k) => k, () => "#199e70").slice(0, 8)} active={f.tool} onToggle={toggle("tool")} /></section>
      </div>

      <div className="mx-slicers">
        <div className="mx-slicers-head">
          <h3>Filter</h3>
          <input className="nw-search" style={{ maxWidth: 300 }} type="search" placeholder="Search titles and text…" value={q} onChange={(e) => setQ(e.target.value)} />
          {anyFilter && <button type="button" className="loki-chipbtn" onClick={() => { setF(EMPTY()); setQ(""); }}>Clear</button>}
        </div>
        <div className="mx-slicer">
          <span className="mx-slicer-title">Period</span>
          {[7, 14, 30, 60, 90].map((d) => <button type="button" key={d} className={`mx-pill ${f.days === d ? "on" : ""}`} onClick={() => setF((x) => ({ ...x, days: d }))}>{d} days</button>)}
        </div>
        <Pills title="Kind" opts={opts("kind", (k) => KIND_LABEL[k] ?? k)} active={f.kind} onToggle={toggle("kind")} />
        <Pills title="Status" opts={statusOpts} active={f.status} onToggle={toggle("status")} />
        <Pills title="Effort" opts={opts("effort", (k) => EFFORT_LABEL[k] ?? k)} active={f.effort} onToggle={toggle("effort")} />
        <Pills title="Flags" opts={Object.keys(FLAG_LABEL).map((k) => ({ key: k, label: FLAG_LABEL[k], n: facets?.flags[k] ?? 0 }))} active={f.flags} onToggle={toggle("flags")} />
        <Pills title="Process" opts={opts("process", (k) => k)} active={f.process} onToggle={toggle("process")} />
        <Pills title="Tool" opts={opts("tool", (k) => k).slice(0, 14)} active={f.tool} onToggle={toggle("tool")} />
      </div>

      <p className="mx-toolbar">{items ? `${Math.min(shown, items.length)} of ${items.length} posts, best first` : "Loading…"}</p>
      <div className="db-cards">
        {visible.map((l) => (
          <article key={l.id} className="db-card ld-card" style={{ ["--c" as string]: STATUS_COLOR[l.status] ?? "#6f8fb0" }}>
            <h4><button type="button" className="mx-open" onClick={() => setOpenId(l.id)}>{l.title}</button></h4>
            <div className="mx-tags">
              <span className={`mx-chip ${l.kind === "hire" ? "urg urg-priority" : ""}`}>{KIND_LABEL[l.kind] ?? l.kind}</span>
              <span className="mx-chip" style={{ borderColor: STATUS_COLOR[l.status], color: STATUS_COLOR[l.status] }}>{cap(l.status)}</span>
              <span className="mx-chip">{l.source}</span>
              {l.effort && <span className="mx-chip">Effort {l.effort}</span>}
              {l.budget && <span className="mx-chip">Budget {l.budget}</span>}
              {l.forme >= 0.5 && <span className="mx-chip q">Also helps me</span>}
              {l.family_size ? <span className="mx-chip">Family of {l.family_size}</span> : null}
            </div>
            {l.summary && <p>{l.summary}</p>}
            <p className="mx-meta">{day(l.published_at)}{l.author ? ` · ${l.author}` : ""}{l.tools.length ? ` · ${l.tools.slice(0, 3).join(", ")}` : ""}</p>
            <div className="mx-tags">
              <button type="button" className="loki-chipbtn" onClick={() => setOpenId(l.id)}>Open</button>
              <a className="loki-chipbtn" href={safeUrl(l.url)} target="_blank" rel="noreferrer noopener">Post ↗</a>
            </div>
          </article>
        ))}
      </div>
      {items && items.length > shown && <p style={{ marginTop: 12 }}><button type="button" className="loki-chipbtn" onClick={() => setShown((n) => n + 40)}>Show 40 more</button></p>}

      {openId !== null && <LeadPanel id={openId} onClose={() => setOpenId(null)} onOpen={setOpenId} onChanged={() => void load()} onAsk={onAsk} />}
    </div>
  );
}

function LeadPanel({ id, onClose, onOpen, onChanged, onAsk }: { id: number; onClose: () => void; onOpen: (id: number) => void; onChanged: () => void; onAsk: (p: string) => void }) {
  const [d, setD] = useState<LeadDetail | null>(null);
  const [fam, setFam] = useState<LeadFamily | null>(null);
  const [busy, setBusy] = useState("");
  const [err, setErr] = useState("");
  const [notes, setNotes] = useState("");
  const [draft, setDraft] = useState("");
  const [saved, setSaved] = useState("");

  useEffect(() => {
    let live = true;
    setD(null);
    setFam(null);
    setErr("");
    leadsApi<LeadDetail>(`items/${id}`).then((x) => {
      if (!live) return;
      setD(x);
      setNotes(x.notes ?? "");
      setDraft(x.draft ?? "");
      if (x.family) leadsApi<{ families: LeadFamily[] }>(`families?family=${x.family.id}&days=365&lang=any`).then((r) => live && setFam(r.families[0] ?? null)).catch(() => {});
    }).catch((e) => live && setErr(String(e)));
    return () => { live = false; };
  }, [id]);
  useEffect(() => {
    const k = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", k);
    return () => window.removeEventListener("keydown", k);
  }, [onClose]);

  const patch = async (body: Record<string, unknown>, what: string) => {
    setErr("");
    try {
      await leadsApi(`items/${id}`, { method: "PATCH", body: JSON.stringify(body) });
      setSaved(what);
      setTimeout(() => setSaved(""), 2000);
      onChanged();
      return true;
    } catch (e) {
      setErr(String(e));
      return false;
    }
  };
  const setStatus = async (s: string) => { if (d && (await patch({ status: s }, "Status saved"))) setD({ ...d, status: s }); };
  const setKind = async (k: string) => { if (d && (await patch({ kind: k }, "Kind saved"))) setD({ ...d, kind: k }); };
  const saveSteps = async (steps: LeadStep[]) => { if (d) { setD({ ...d, steps }); await patch({ steps }, "Steps saved"); } };
  async function generate(what: "draft" | "steps") {
    setBusy(what);
    setErr("");
    try {
      const r = await leadsApi<{ draft?: string; steps?: LeadStep[] }>(`items/${id}/${what}`, { method: "POST" });
      if (what === "draft" && r.draft !== undefined) setDraft(r.draft);
      if (what === "steps" && r.steps && d) setD({ ...d, steps: r.steps });
      onChanged();
    } catch (e) {
      setErr(String(e));
    }
    setBusy("");
  }
  async function famSteps(steps?: LeadStep[]) {
    if (!d?.family) return;
    try {
      if (steps) {
        await leadsApi(`families/${d.family.id}/steps`, { method: "PATCH", body: JSON.stringify({ steps }) });
        setFam((x) => (x ? { ...x, steps } : x));
      } else {
        setBusy("fam");
        const r = await leadsApi<{ steps: LeadStep[] }>(`families/${d.family.id}/steps`, { method: "POST" });
        setFam((x) => (x ? { ...x, steps: r.steps } : x));
      }
    } catch (e) {
      setErr(String(e));
    }
    setBusy("");
  }

  const StepList = ({ steps, onChange }: { steps: LeadStep[]; onChange: (s: LeadStep[]) => void }) => (
    <div className="db-items">
      {steps.map((s, i) => (
        <div key={i} className={`db-item ${s.done ? "done" : ""}`}>
          <input type="checkbox" checked={s.done} onChange={() => onChange(steps.map((x, k) => (k === i ? { ...x, done: !x.done } : x)))} aria-label="Done" />
          <input className="ld-step" value={s.t} onChange={(e) => onChange(steps.map((x, k) => (k === i ? { ...x, t: e.target.value } : x)))} />
          <button type="button" className="mx-x" style={{ width: 24, height: 24 }} onClick={() => onChange(steps.filter((_, k) => k !== i))} aria-label="Remove step">✕</button>
        </div>
      ))}
      <button type="button" className="loki-chipbtn" onClick={() => onChange([...steps, { t: "", done: false }])}>+ Add a step</button>
    </div>
  );

  return (
    <div className="mx-overlay" onClick={onClose}>
      <aside className="mx-viewer" onClick={(e) => e.stopPropagation()} role="dialog" aria-label="Lead">
        <div className="mx-viewer-top">
          <span className="mx-meta">{saved || (d ? `${KIND_LABEL[d.kind] ?? d.kind} · ${d.source}` : "")}</span>
          <button type="button" className="mx-x" onClick={onClose} aria-label="Close">✕</button>
        </div>
        {err && <p className="loki-error">{err}</p>}
        {!d && !err && <p className="loki-home-note">Loading…</p>}
        {d && (
          <>
            <h3 className="mx-viewer-subject">{d.title}</h3>
            <dl className="mx-viewer-meta">
              <dt>Posted</dt><dd>{day(d.published_at)}{d.author ? ` by ${d.author}` : ""}</dd>
              <dt>Platform</dt><dd>{d.source}{d.tools.length ? ` · ${d.tools.join(", ")}` : ""}</dd>
              {(d.effort || d.budget) && <><dt>Size</dt><dd>Effort {d.effort || "?"}{d.budget ? ` · budget ${d.budget}` : " · no budget stated"}</dd></>}
              {d.process && <><dt>Process</dt><dd>{d.process}</dd></>}
            </dl>
            <div className="mx-viewer-actions">
              <a className="loki-chipbtn" href={safeUrl(d.url)} target="_blank" rel="noreferrer noopener">Open the post ↗</a>
              <button type="button" className="loki-chipbtn" onClick={() => onAsk(`Give me a short, practical take on this forum request and whether it is worth replying to. Use only this text; do not search anything.\n\n${d.title}\n${d.summary}\nScope: ${d.scope}`)}>Ask Loki</button>
            </div>

            <h4 className="ld-h">Status</h4>
            <div className="mx-tags">
              {STATUSES.map((s) => <button type="button" key={s} className={`mx-pill ${d.status === s ? "on" : ""}`} style={d.status === s ? { background: STATUS_COLOR[s], borderColor: STATUS_COLOR[s], color: "#08141f" } : undefined} onClick={() => void setStatus(s)}>{cap(s)}</button>)}
            </div>
            <h4 className="ld-h">Kind</h4>
            <div className="mx-tags">
              {["hire", "fix", "promo", "skip"].map((k) => <button type="button" key={k} className={`mx-pill ${d.kind === k ? "on" : ""}`} onClick={() => void setKind(k)}>{KIND_LABEL[k]}</button>)}
              <span className="mx-meta">Promo and Skip take it out of the lists.</span>
            </div>

            {(d.summary || d.scope || d.pain) && (
              <>
                <h4 className="ld-h">What they want</h4>
                {d.summary && <p>{d.summary}</p>}
                {d.scope && <p className="mx-meta"><b>Smallest version:</b> {d.scope}</p>}
                {d.pain && <p className="mx-meta"><b>The pain:</b> {d.pain}</p>}
              </>
            )}
            {d.forme >= 0.5 && <p className="mx-chip q" style={{ display: "inline-block" }}>Also helps me{d.forme_why ? `: ${d.forme_why}` : ""}</p>}

            <h4 className="ld-h">Next steps</h4>
            <StepList steps={d.steps ?? []} onChange={(s) => void saveSteps(s)} />
            <button type="button" className="loki-chipbtn" disabled={busy === "steps"} onClick={() => void generate("steps")}>{busy === "steps" ? "Thinking…" : d.steps?.length ? "Suggest new steps" : "Suggest next steps"}</button>

            <h4 className="ld-h">Reply draft <small className="mx-meta">(text for you to send by hand; nothing is posted)</small></h4>
            <textarea className="ld-area" rows={5} value={draft} placeholder="No draft yet." onChange={(e) => setDraft(e.target.value)} onBlur={() => draft !== (d.draft ?? "") && void patch({ draft }, "Draft saved")} />
            <div className="mx-viewer-actions">
              <button type="button" className="loki-chipbtn" disabled={busy === "draft"} onClick={() => void generate("draft")}>{busy === "draft" ? "Writing…" : draft ? "Write a new draft" : "Write a reply draft"}</button>
              {draft && <button type="button" className="loki-chipbtn" onClick={() => void navigator.clipboard?.writeText(draft)}>Copy</button>}
            </div>

            <h4 className="ld-h">Notes</h4>
            <textarea className="ld-area" rows={3} value={notes} placeholder="Your notes on this lead." onChange={(e) => setNotes(e.target.value)} onBlur={() => notes !== (d.notes ?? "") && void patch({ notes }, "Notes saved")} />

            {d.family && (
              <>
                <h4 className="ld-h">Solution family: {d.family.label || "similar requests"}</h4>
                {d.family.reuse && <p className="mx-meta">{d.family.reuse}</p>}
                {fam && <p className="mx-meta">{fam.size} similar request{fam.size === 1 ? "" : "s"}.</p>}
                <StepList steps={fam?.steps ?? []} onChange={(s) => void famSteps(s)} />
                <button type="button" className="loki-chipbtn" disabled={busy === "fam"} onClick={() => void famSteps()}>{busy === "fam" ? "Thinking…" : "Suggest family steps"}</button>
                {fam && fam.members.filter((m) => m.id !== id).length > 0 && (
                  <div className="ld-related">{fam.members.filter((m) => m.id !== id).slice(0, 8).map((m) => <button type="button" key={m.id} onClick={() => onOpen(m.id)}>{m.title}</button>)}</div>
                )}
              </>
            )}

            {d.related.length > 0 && (
              <>
                <h4 className="ld-h">Related posts</h4>
                <div className="ld-related">{d.related.map((r) => <button type="button" key={r.id} onClick={() => onOpen(r.id)}>{r.title} <em>{Math.round(r.sim * 100)}% · {r.status}</em></button>)}</div>
              </>
            )}

            <h4 className="ld-h">The post</h4>
            <pre className="mx-viewer-body">{d.text}</pre>
          </>
        )}
      </aside>
    </div>
  );
}
