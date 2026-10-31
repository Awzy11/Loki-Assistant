import { Fragment, useCallback, useEffect, useMemo, useState } from "react";
import { BarList, Kpis, Pills, type Opt } from "./dash";
import ExtractPanel from "./ExtractPanel";
import NewsletterSources from "./NewsletterSources";
import { navigate } from "./router";
import { mailInbox, mailInboxAct, mailInboxDraft, mailInboxMessage, mailInboxScan, newsletterSenders, setNewsletterSender, type InboxItem, type InboxMessage, type InboxPage } from "./transport";

type Dim = "account" | "category" | "urgency" | "class" | "status" | "age" | "quadrant";
type Filters = Record<Dim, Set<string>>;
const EMPTY = (): Filters => ({ account: new Set(), category: new Set(), urgency: new Set(), class: new Set(), status: new Set(), age: new Set(), quadrant: new Set() });

const CATEGORIES = ["finance", "school", "home", "work", "investments", "tech", "cyber", "other"];
const URGENCIES = ["urgent", "priority", "routine", "discard"];
const CLASSES = ["action", "review", "fyi", "receipt", "junk", "stale"];
const STATUSES = ["unread", "read", "slip", "reply"];
const STATUS_LABEL: Record<string, string> = { unread: "Unread", read: "Read", slip: "Read but needs action", reply: "Waiting on my reply" };
const AGES = ["today", "week", "month", "older"];
const AGE_LABEL: Record<string, string> = { today: "Today", week: "This week", month: "This month", older: "Older" };
const QUADRANTS = ["q1", "q2", "q3", "q4"];
const QUAD_LABEL: Record<string, string> = { q1: "Q1 Do", q2: "Q2 Schedule", q3: "Q3 Delegate", q4: "Q4 Delete" };
const QUAD_HINT: Record<string, string> = { q1: "urgent + important", q2: "important, not urgent", q3: "urgent, not important", q4: "neither" };
const GAIN_LABEL: Record<string, string> = { g: "Give to AI", a: "Accelerate", i: "Integrate", n: "No AI" };
const URG_ICON: Record<string, string> = { urgent: "▲", priority: "◆", routine: "●", discard: "✕" };
const cap = (s: string) => (s ? s[0].toUpperCase() + s.slice(1) : s);

const isSlip = (i: InboxItem) => !i.unread && (i.class === "action" || i.urgency === "urgent" || i.urgency === "priority");
const ageOf = (i: InboxItem) => {
  const d = (Date.now() - i.at_ms) / 86_400_000;
  return d < 1 ? "today" : d < 7 ? "week" : d < 30 ? "month" : "older";
};
const sender = (from: string) => {
  const m = from.match(/^\s*"?([^"<]*?)"?\s*<([^>]+)>/);
  return (m ? m[1].trim() || m[2] : from).trim();
};
const addressOf = (from: string) => (from.match(/<([^>]+)>/)?.[1] ?? from).trim().replace(/^"|"$/g, "").toLowerCase();
const NL_LABEL: Record<string, string> = { "": "📰 Mark sender as newsletter", news: "📰 Newsletter: News digest", market: "📰 Newsletter: Market watch", ignore: "📰 Newsletter: ignored" };

// One sender, one choice: feeds the morning News / Market Watch digests (read on the local model only; the inbox is not changed).
function NewsletterPick({ value, onPick, busy, compact }: { value: string; onPick: (bucket: string) => void; busy: boolean; compact?: boolean }) {
  return (
    <select className={compact ? "mx-nl mx-nl-compact" : "mx-nl"} value={value} disabled={busy} aria-label="Mark sender as newsletter" title="Use this sender's newsletters in the morning digests. They are summarized on your local model only; nothing in your inbox changes."
      onChange={(e) => onPick(e.target.value)}>
      <option value="">{NL_LABEL[""]}</option>
      <option value="news">{value === "news" ? "✓ " : ""}→ News digest</option>
      <option value="market">{value === "market" ? "✓ " : ""}→ Market watch</option>
      <option value="ignore">{value === "ignore" ? "✓ " : ""}→ Never a source</option>
      {value && <option value="__clear">✕ Remove from newsletters</option>}
    </select>
  );
}
const when = (ms: number) => {
  const d = (Date.now() - ms) / 86_400_000;
  if (d < 1) return new Date(ms).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
  if (d < 7) return `${Math.floor(d)} d ago`;
  return new Date(ms).toLocaleDateString([], { month: "short", day: "numeric" });
};
const key = (i: InboxItem) => `${i.account}|${i.id}`;
const gmailLink = (i: InboxItem) => `https://mail.google.com/mail/?authuser=${encodeURIComponent(i.account)}#all/${i.id}`;

/** Plain text with links made clickable. Never rendered as HTML. */
function Linked({ text }: { text: string }) {
  const parts = text.split(/(https?:\/\/[^\s<>"')]+)/g);
  return <>{parts.map((p, n) => (n % 2 ? <a key={n} href={p} target="_blank" rel="noreferrer noopener">{p}</a> : <Fragment key={n}>{p}</Fragment>))}</>;
}

function valueOf(i: InboxItem, d: Dim): string[] {
  switch (d) {
    case "account": return [i.account];
    case "category": return [i.category || "other"];
    case "urgency": return [i.urgency || "routine"];
    case "class": return [i.class || "fyi"];
    case "age": return [ageOf(i)];
    case "quadrant": return [i.quadrant];
    case "status": return [i.unread ? "unread" : "read", ...(isSlip(i) ? ["slip"] : []), ...(i.reply ? ["reply"] : [])];
  }
}
const passes = (i: InboxItem, f: Filters, skip?: Dim) =>
  (Object.keys(f) as Dim[]).every((d) => d === skip || f[d].size === 0 || valueOf(i, d).some((v) => f[d].has(v)));

export default function MailDashboard() {
  const [page, setPage] = useState<InboxPage | null>(null);
  const [error, setError] = useState("");
  const [filters, setFilters] = useState<Filters>(EMPTY);
  const [view, setView] = useState<"columns" | "list">("columns");
  const [showNl, setShowNl] = useState(false);
  const [scope, setScope] = useState<"inbox" | "archived" | "all">("inbox");
  const [sel, setSel] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState<string | null>(null);
  const [notice, setNotice] = useState("");
  const [viewing, setViewing] = useState<InboxItem | null>(null);
  const [nl, setNl] = useState<Record<string, string>>({});
  useEffect(() => { newsletterSenders().then((rows) => setNl(Object.fromEntries(rows.filter((r) => r.bucket).map((r) => [r.address, r.bucket])))).catch(() => {}); }, []);
  async function tagSenders(items: InboxItem[], pick: string) {
    const bucket = pick === "__clear" ? "" : pick;
    const seen = new Map<string, string>();
    items.forEach((i) => seen.set(addressOf(i.from), sender(i.from)));
    try {
      for (const [address, name] of seen) await setNewsletterSender(address, name, bucket);
      setNl((m) => { const n = { ...m }; seen.forEach((_, a) => { if (bucket) n[a] = bucket; else delete n[a]; }); return n; });
      const names = [...seen.values()].slice(0, 3).join(", ") + (seen.size > 3 ? ` and ${seen.size - 3} more` : "");
      setNotice(bucket ? `${names}: ${bucket === "news" ? "News digest" : bucket === "market" ? "Market watch" : "never a newsletter source"}.` : `${names}: removed from newsletters.`);
    } catch (e) { setNotice(`Couldn't tag the sender: ${String((e as Error).message || e)}`); }
  }
  const [msg, setMsg] = useState<InboxMessage | "loading" | null>(null);
  const [drafts, setDrafts] = useState<Record<string, { text: string; saved: string } | "busy">>({});

  const load = useCallback(async () => {
    try {
      setPage(await mailInbox());
    } catch (e) {
      setError(String(e));
    }
  }, []);
  useEffect(() => void load(), [load]);
  const running = page?.status === "running";
  useEffect(() => {
    if (!running) return;
    const t = setInterval(() => void load(), 4000);
    return () => clearInterval(t);
  }, [running, load]);

  const everything = page?.items ?? [];
  // Archived mail is read too, but only shows when the user picks Archived or All.
  const nArchived = everything.filter((i) => i.archived).length;
  const items = everything.filter((i) => scope === "all" || (scope === "archived") === !!i.archived);
  const shown = useMemo(() => items.filter((i) => passes(i, filters)), [items, filters]);
  const accounts = useMemo(() => Array.from(new Set(items.map((i) => i.account))).sort(), [items]);
  const triaged = items.some((i) => i.quadrant);
  const count = (d: Dim, v: string) => items.filter((i) => passes(i, filters, d) && valueOf(i, d).includes(v)).length;

  const toggle = (d: Dim, v: string) =>
    setFilters((f) => {
      const next = { ...f, [d]: new Set(f[d]) };
      next[d].has(v) ? next[d].delete(v) : next[d].add(v);
      return next;
    });
  const anyFilter = (Object.values(filters) as Set<string>[]).some((s) => s.size > 0);

  async function act(list: InboxItem[], action: "read" | "unread" | "trash" | "archive" | "unarchive") {
    if (!list.length) return;
    if (action === "trash" && !window.confirm(`Move ${list.length} message${list.length > 1 ? "s" : ""} to Trash? Gmail keeps them for 30 days.`)) return;
    setBusy(action);
    setNotice("");
    try {
      for (const acct of Array.from(new Set(list.map((i) => i.account)))) {
        const ids = list.filter((i) => i.account === acct).map((i) => i.id);
        setNotice(await mailInboxAct(acct, ids, action));
      }
      setSel(new Set());
      await load();
    } catch (e) {
      setError(String(e));
    }
    setBusy(null);
  }
  async function draft(i: InboxItem) {
    setDrafts((d) => ({ ...d, [key(i)]: "busy" }));
    try {
      const r = await mailInboxDraft(i.account, i.id);
      setDrafts((d) => ({ ...d, [key(i)]: { text: r.draft, saved: r.saved } }));
    } catch (e) {
      setDrafts((d) => {
        const n = { ...d };
        delete n[key(i)];
        return n;
      });
      setError(String(e));
    }
  }
  async function openMessage(i: InboxItem) {
    setViewing(i);
    setMsg("loading");
    try {
      setMsg(await mailInboxMessage(i.account, i.id));
    } catch (e) {
      setMsg(null);
      setViewing(null);
      setError(String(e));
    }
  }
  const closeViewer = () => {
    setViewing(null);
    setMsg(null);
  };
  useEffect(() => {
    if (!viewing) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && closeViewer();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [viewing]);

  async function scan(matrix: boolean) {
    setError("");
    try {
      await mailInboxScan(matrix);
      await load();
    } catch (e) {
      setError(String(e));
    }
  }

  const slips = items.filter(isSlip);
  // Options for the shared bar charts and slicers: counts respect every other filter (cross-filtering).
  const opts = (d: Dim, values: string[], label: (v: string) => string, color?: (v: string) => string): Opt[] => values.map((v) => ({ key: v, label: label(v), n: count(d, v), color: color?.(v) }));
  const flip = (d: Dim) => (v: string) => toggle(d, v);
  const catColor = (v: string) => `var(--cat-${v})`;
  const urgLabel = (v: string) => `${URG_ICON[v]} ${cap(v)}`;
  const acctLabel = (a: string) => a.replace(/@gmail\.com$/, "");

  const row = (i: InboxItem) => {
    const k = key(i);
    const dr = drafts[k];
    return (
      <div key={k} className={`mx-row ${i.unread ? "unread" : ""} ${i.urgency === "discard" ? "discard" : ""}`} style={{ ["--cat" as string]: `var(--cat-${i.category || "other"})` }}>
        <input type="checkbox" checked={sel.has(k)} onChange={() => setSel((s) => { const n = new Set(s); n.has(k) ? n.delete(k) : n.add(k); return n; })} aria-label="Select message" />
        <div className="mx-main">
          <button type="button" className="mx-subject mx-open" onClick={() => void openMessage(i)} title="Read it here">{i.subject || "(no subject)"}</button>
          <div className="mx-meta">
            {sender(i.from)} · {when(i.at_ms)}
            {view === "list" && <> · {i.account}</>}
          </div>
          {(i.next || i.why) && <div className="mx-why">{i.next ? `Next: ${i.next}` : i.why}</div>}
          <div className="mx-tags">
            <span className="mx-chip cat"><i />{cap(i.category || "other")}</span>
            <span className={`mx-chip urg urg-${i.urgency || "routine"}`}>{URG_ICON[i.urgency || "routine"]} {cap(i.urgency || "routine")}</span>
            {i.archived && <span className="mx-chip">Archived</span>}
            {i.class && <span className="mx-chip">{cap(i.class)}</span>}
            {i.quadrant && <span className="mx-chip q">{QUAD_LABEL[i.quadrant]}{i.gain ? ` · ${GAIN_LABEL[i.gain]}` : ""}</span>}
            {isSlip(i) && <span className="mx-chip slip">Read, still needs action</span>}
            {i.reply && <span className="mx-chip">Needs a reply</span>}
          </div>
          {dr && dr !== "busy" && (
            <div className="mx-draft">
              <div className="mx-draft-note">{dr.saved}</div>
              <pre>{dr.text}</pre>
              <button type="button" className="loki-chipbtn" onClick={() => void navigator.clipboard?.writeText(dr.text)}>Copy</button>
            </div>
          )}
        </div>
        <div className="mx-acts">
          <button type="button" title="Read it here" onClick={() => void openMessage(i)}>👁 View</button>
          <a className="mx-link" href={gmailLink(i)} target="_blank" rel="noreferrer noopener" title="Open this message in Gmail">Gmail ↗</a>
          <button type="button" title={i.unread ? "Mark read" : "Mark unread"} disabled={!!busy} onClick={() => void act([i], i.unread ? "read" : "unread")}>{i.unread ? "✓ Mark read" : "Mark unread"}</button>
          <button type="button" title="Draft a reply with the local model (saved to Gmail drafts, never sent)" disabled={dr === "busy"} onClick={() => void draft(i)}>{dr === "busy" ? "Drafting…" : "✎ Draft"}</button>
          {i.archived
            ? <button type="button" title="Put it back in the inbox" disabled={!!busy} onClick={() => void act([i], "unarchive")}>↩ To inbox</button>
            : <button type="button" title="Archive: out of the inbox, still in All Mail" disabled={!!busy} onClick={() => void act([i], "archive")}>🗄 Archive</button>}
          <button type="button" title="Move to Trash" disabled={!!busy} onClick={() => void act([i], "trash")}>🗑 Trash</button>
          <NewsletterPick compact value={nl[addressOf(i.from)] ?? ""} busy={!!busy} onPick={(b) => void tagSenders([i], b)} />
        </div>
      </div>
    );
  };

  const order = (list: InboxItem[]) => [...list].sort((a, b) => URGENCIES.indexOf(a.urgency || "routine") - URGENCIES.indexOf(b.urgency || "routine") || b.at_ms - a.at_ms);
  const selected = items.filter((i) => sel.has(key(i)));

  if (!page) return <div className="loki-page"><p className="loki-home-note">{error || "Loading…"}</p></div>;
  return (
    <div className="loki-page mx">
      <div className="loki-page-head">
        <h2>Mail</h2>
        <div className="mx-head-actions">
          <button type="button" className="loki-chipbtn" disabled={running} onClick={() => void scan(false)}>{running ? "Reading…" : "Rescan inbox"}</button>
          <button type="button" className="loki-chipbtn" disabled={running} onClick={() => void scan(true)} title="Eisenhower quadrant, then GAIN for Q1 and Q2">Run two-matrix triage</button>
          <button type="button" className="loki-chipbtn" aria-pressed={showNl} onClick={() => setShowNl((v) => !v)}>Newsletter sources</button>
          <button type="button" className="loki-chipbtn" onClick={() => navigate("/mail/review")}>Inbox review →</button>
        </div>
      </div>
      {showNl && <NewsletterSources />}
      <p className="loki-home-note">
        {running ? `${page.phase} · ${page.done}/${page.total}` : page.updated ? `Sorted ${new Date(page.updated * 1000).toLocaleString([], { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" })} by the local model. Read and unread are both included.` : "Not scanned yet: press Rescan inbox."}
        {" "}Nothing here leaves your servers.
      </p>
      {(error || page.error) && <div className="loki-error">{error || page.error}</div>}
      {notice && <div className="mx-notice">{notice}</div>}

      <div className="mx-scope" role="group" aria-label="Which mail to show">
        <button type="button" className={scope === "inbox" ? "on" : ""} onClick={() => { setScope("inbox"); setSel(new Set()); }}>Inbox <em>{everything.length - nArchived}</em></button>
        <button type="button" className={scope === "archived" ? "on" : ""} onClick={() => { setScope("archived"); setSel(new Set()); }}>Archived <em>{nArchived}</em></button>
        <button type="button" className={scope === "all" ? "on" : ""} onClick={() => { setScope("all"); setSel(new Set()); }}>All <em>{everything.length}</em></button>
        <span className="mx-meta">Archived mail from the last 90 days is read too, but it stays out of the counts and filters unless you pick Archived or All.</span>
      </div>

      <Kpis items={[
        { n: items.length, label: scope === "inbox" ? "In inboxes" : scope === "archived" ? "Archived" : "Inbox and archived", onClick: () => setFilters(EMPTY()) },
        { n: items.filter((i) => i.unread).length, label: "Unread", onClick: () => setFilters({ ...EMPTY(), status: new Set(["unread"]) }) },
        { n: items.filter((i) => i.class === "action").length, label: "Need action", tone: "warn", onClick: () => setFilters({ ...EMPTY(), class: new Set(["action"]) }) },
        { n: slips.length, label: "Read, still need action", tone: slips.length ? "bad" : "", onClick: () => setFilters({ ...EMPTY(), status: new Set(["slip"]) }) },
        { n: items.filter((i) => i.reply).length, label: "Waiting on my reply", onClick: () => setFilters({ ...EMPTY(), status: new Set(["reply"]) }) },
      ]} />

      {items.length > 0 && (
        <div className="mx-charts">
          <section className="mx-card"><h3>By function</h3><BarList opts={opts("category", CATEGORIES, cap, catColor)} active={filters.category} onToggle={flip("category")} /></section>
          <section className="mx-card"><h3>By urgency</h3>
            <div className="mx-stack" role="img" aria-label="Urgency split">
              {URGENCIES.map((u) => {
                const n = count("urgency", u);
                return n ? <button type="button" key={u} className={`urg-fill urg-${u} ${filters.urgency.has(u) ? "on" : ""}`} style={{ flexGrow: n }} onClick={() => toggle("urgency", u)} title={`${cap(u)}: ${n}`}>{n}</button> : null;
              })}
            </div>
            <BarList opts={opts("urgency", URGENCIES, urgLabel, (v) => `var(--urg-${v})`)} active={filters.urgency} onToggle={flip("urgency")} />
          </section>
          <section className="mx-card"><h3>By account</h3>
            <BarList opts={opts("account", accounts, acctLabel, (a) => `var(--acct-${accounts.indexOf(a) % 2})`)} active={filters.account} onToggle={flip("account")} />
          </section>
          {triaged && (
            <section className="mx-card"><h3>Two-matrix triage</h3>
              <div className="mx-quad">
                {QUADRANTS.map((q) => (
                  <button type="button" key={q} className={`mx-q mx-${q} ${filters.quadrant.has(q) ? "on" : ""}`} onClick={() => toggle("quadrant", q)}>
                    <b>{count("quadrant", q)}</b>
                    <span>{QUAD_LABEL[q]}</span>
                    <small>{QUAD_HINT[q]}</small>
                  </button>
                ))}
              </div>
              <div className="mx-gain">
                {Object.entries(GAIN_LABEL).map(([g, l]) => <span key={g} className="mx-chip">{l} {items.filter((i) => i.gain === g && passes(i, filters, "quadrant")).length}</span>)}
              </div>
            </section>
          )}
        </div>
      )}

      {items.length > 0 && (
        <div className="mx-slicers">
          <div className="mx-slicers-head">
            <h3>Filter</h3>
            {anyFilter && <button type="button" className="loki-chipbtn" onClick={() => setFilters(EMPTY())}>Clear filters</button>}
          </div>
          <Pills title="Account" opts={opts("account", accounts, acctLabel)} active={filters.account} onToggle={flip("account")} />
          <Pills title="Function" opts={opts("category", CATEGORIES, cap, catColor)} active={filters.category} onToggle={flip("category")} />
          <Pills title="Urgency" opts={opts("urgency", URGENCIES, urgLabel)} active={filters.urgency} onToggle={flip("urgency")} />
          <Pills title="Class" opts={opts("class", CLASSES, cap)} active={filters.class} onToggle={flip("class")} />
          <Pills title="Status" opts={opts("status", STATUSES, (v) => STATUS_LABEL[v])} active={filters.status} onToggle={flip("status")} />
          <Pills title="Age" opts={opts("age", AGES, (v) => AGE_LABEL[v])} active={filters.age} onToggle={flip("age")} />
          {triaged && <Pills title="Quadrant" opts={opts("quadrant", QUADRANTS, (v) => QUAD_LABEL[v])} active={filters.quadrant} onToggle={flip("quadrant")} />}
        </div>
      )}

      {items.length > 0 && (
        <div className="mx-toolbar">
          <span>{shown.length} of {items.length} messages</span>
          <div className="mx-seg">
            <button type="button" className={view === "columns" ? "on" : ""} onClick={() => setView("columns")}>Side by side</button>
            <button type="button" className={view === "list" ? "on" : ""} onClick={() => setView("list")}>One list</button>
          </div>
          <button type="button" className="loki-chipbtn" onClick={() => setSel(new Set(shown.map(key)))}>Select shown</button>
          {selected.length > 0 && (
            <div className="mx-bulk">
              <b>{selected.length} selected</b>
              <button type="button" disabled={!!busy} onClick={() => void act(selected, "read")}>✓ Mark read</button>
              {selected.some((i) => !i.archived) && <button type="button" disabled={!!busy} onClick={() => void act(selected.filter((i) => !i.archived), "archive")}>🗄 Archive</button>}
              {selected.some((i) => i.archived) && <button type="button" disabled={!!busy} onClick={() => void act(selected.filter((i) => i.archived), "unarchive")}>↩ To inbox</button>}
              <button type="button" disabled={!!busy} onClick={() => void act(selected, "trash")}>🗑 Trash</button>
              <NewsletterPick compact value="" busy={!!busy} onPick={(b) => void tagSenders(selected, b)} />
              <button type="button" onClick={() => setSel(new Set())}>Clear</button>
            </div>
          )}
        </div>
      )}

      {items.length === 0 && !running && <p className="loki-home-note">The inboxes are empty, or haven't been read yet.</p>}
      {view === "columns" ? (
        <div className="mx-cols" style={{ gridTemplateColumns: `repeat(${Math.max(1, accounts.length)}, minmax(0, 1fr))` }}>
          {accounts.map((a) => {
            const list = order(shown.filter((i) => i.account === a));
            return (
              <section key={a} className="mx-col">
                <h3>{a} <small>{list.length}</small></h3>
                {list.length === 0 && <p className="loki-home-note">Nothing matches the filters.</p>}
                {list.map(row)}
              </section>
            );
          })}
        </div>
      ) : (
        <div className="mx-col">{order(shown).map(row)}</div>
      )}
      {viewing && (
        <div className="mx-overlay" onClick={closeViewer}>
          <aside className="mx-viewer" onClick={(e) => e.stopPropagation()} role="dialog" aria-label="Message">
            <div className="mx-viewer-top">
              <div className="mx-tags">
                <span className="mx-chip cat" style={{ ["--cat" as string]: `var(--cat-${viewing.category || "other"})` }}><i />{cap(viewing.category || "other")}</span>
                <span className={`mx-chip urg urg-${viewing.urgency || "routine"}`}>{URG_ICON[viewing.urgency || "routine"]} {cap(viewing.urgency || "routine")}</span>
                {viewing.quadrant && <span className="mx-chip q">{QUAD_LABEL[viewing.quadrant]}{viewing.gain ? ` · ${GAIN_LABEL[viewing.gain]}` : ""}</span>}
              </div>
              <button type="button" className="mx-x" onClick={closeViewer} aria-label="Close">✕</button>
            </div>
            {msg === "loading" || !msg ? (
              <p className="loki-home-note">Loading the message…</p>
            ) : (
              <>
                <h3 className="mx-viewer-subject">{msg.subject || "(no subject)"}</h3>
                <dl className="mx-viewer-meta">
                  <dt>From</dt><dd>{msg.from}</dd>
                  <dt>To</dt><dd>{msg.to}</dd>
                  {msg.cc && <><dt>Cc</dt><dd>{msg.cc}</dd></>}
                  <dt>Date</dt><dd>{msg.date}</dd>
                  <dt>Account</dt><dd>{msg.account}</dd>
                </dl>
                {(viewing.next || viewing.why) && <div className="mx-why">{viewing.next ? `Next: ${viewing.next}` : viewing.why}</div>}
                {msg.attachments.length > 0 && <div className="mx-attach">Attachments: {msg.attachments.join(", ")} (open in Gmail to download)</div>}
                <pre className="mx-viewer-body"><Linked text={msg.body || "(no text in this message)"} /></pre>
                <div className="mx-viewer-actions">
                  <a className="loki-chipbtn" href={msg.link} target="_blank" rel="noreferrer noopener">Open in Gmail ↗</a>
                  <button type="button" className="loki-chipbtn" disabled={!!busy} onClick={() => void act([viewing], viewing.unread ? "read" : "unread").then(() => setViewing(null))}>{viewing.unread ? "Mark read & close" : "Mark unread & close"}</button>
                  <button type="button" className="loki-chipbtn" disabled={drafts[key(viewing)] === "busy"} onClick={() => void draft(viewing)}>{drafts[key(viewing)] === "busy" ? "Drafting…" : "✎ Draft reply"}</button>
                  {viewing.archived
                    ? <button type="button" className="loki-chipbtn" disabled={!!busy} onClick={() => void act([viewing], "unarchive").then(() => closeViewer())}>↩ Move to inbox</button>
                    : <button type="button" className="loki-chipbtn" disabled={!!busy} onClick={() => void act([viewing], "archive").then(() => closeViewer())}>🗄 Archive</button>}
                  <button type="button" className="loki-chipbtn" disabled={!!busy} onClick={() => void act([viewing], "trash").then(() => closeViewer())}>🗑 Trash</button>
                  <NewsletterPick value={nl[addressOf(viewing.from)] ?? ""} busy={!!busy} onPick={(b) => void tagSenders([viewing], b)} />
                </div>
                <ExtractPanel key={key(viewing)} source={{ source: "mail", account: viewing.account, id: viewing.id }} />
                {(() => {
                  const dr = drafts[key(viewing)];
                  return dr && dr !== "busy" ? (
                    <div className="mx-draft">
                      <div className="mx-draft-note">{dr.saved}</div>
                      <pre>{dr.text}</pre>
                      <button type="button" className="loki-chipbtn" onClick={() => void navigator.clipboard?.writeText(dr.text)}>Copy</button>
                    </div>
                  ) : null;
                })()}
              </>
            )}
          </aside>
        </div>
      )}
    </div>
  );
}
