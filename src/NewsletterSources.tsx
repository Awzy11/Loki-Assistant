import { useEffect, useMemo, useState } from "react";
import { newsletterSenders, setNewsletterSender, type NewsletterSender } from "./transport";

const LABEL: Record<string, string> = { "": "Not a source", news: "News digest", market: "Market watch", ignore: "Ignore" };

// Tag the senders whose newsletters should feed the morning digests. Names and counts only; no message text is shown or sent.
export default function NewsletterSources() {
  const [rows, setRows] = useState<NewsletterSender[] | null>(null);
  const [q, setQ] = useState("");
  const [error, setError] = useState("");
  const [tagged, setTagged] = useState(true);

  useEffect(() => { newsletterSenders().then(setRows).catch((e) => setError(String(e.message || e))); }, []);

  const shown = useMemo(() => {
    const t = q.trim().toLowerCase();
    return (rows ?? []).filter((r) => (!t || r.address.includes(t) || r.name.toLowerCase().includes(t)) && (!tagged || t || r.bucket !== "" || (rows ?? []).length <= 40 || r.count >= 3)).slice(0, 80);
  }, [rows, q, tagged]);

  async function tag(r: NewsletterSender, bucket: string) {
    setError("");
    try {
      await setNewsletterSender(r.address, r.name, bucket);
      setRows((all) => (all ?? []).map((x) => (x.address === r.address ? { ...x, bucket: bucket as NewsletterSender["bucket"] } : x)));
    } catch (e) { setError(String((e as Error).message || e)); }
  }

  const count = (b: string) => (rows ?? []).filter((r) => r.bucket === b).length;
  return (
    <section className="mx-card nl">
      <h3>Newsletter sources</h3>
      <p className="mx-meta">
        Pick the senders whose newsletters should feed the morning News and Market Watch digests ({count("news")} news, {count("market")} market). Only tagged senders, and only mass-mail messages from them, are ever read.
        They are summarized on your local model and never sent to a cloud model. Your inbox is left exactly as it is: nothing is marked read.
      </p>
      <div className="nt-actions">
        <input className="nt-input" value={q} onChange={(e) => setQ(e.target.value)} placeholder="Search senders" aria-label="Search senders" />
        <label className="mx-meta"><input type="checkbox" checked={tagged} onChange={(e) => setTagged(e.target.checked)} /> Hide rare senders</label>
      </div>
      {error && <p className="mx-meta" role="alert">{error}</p>}
      {rows === null && !error && <p className="mx-meta">Loading senders…</p>}
      <div className="db-items">
        {shown.map((r) => (
          <div key={r.address} className="db-item">
            <span><strong>{r.name || r.address}</strong> <em className="db-dim">{r.address} · {r.count} message{r.count === 1 ? "" : "s"}{r.last_ms ? ` · last ${new Date(r.last_ms).toLocaleDateString([], { month: "short", day: "numeric" })}` : ""}</em></span>
            <select aria-label={`Bucket for ${r.name || r.address}`} value={r.bucket} onChange={(e) => void tag(r, e.target.value)}>
              {Object.entries(LABEL).map(([v, l]) => <option key={v} value={v}>{l}</option>)}
            </select>
          </div>
        ))}
        {rows !== null && shown.length === 0 && <p className="mx-meta">No senders match. Run “Rescan inbox” first if the list is empty.</p>}
      </div>
    </section>
  );
}
