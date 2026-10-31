// Small building blocks shared by the dashboard tabs (Mail, News, …). Styles: `.mx-*` in App.css.
export type Kpi = { n: number | string; label: string; tone?: "warn" | "bad" | ""; onClick?: () => void };

export function Kpis({ items }: { items: Kpi[] }) {
  return (
    <div className="mx-kpis">
      {items.map((k) => (
        <button type="button" key={k.label} className={`mx-kpi ${k.tone ?? ""}`} onClick={k.onClick} disabled={!k.onClick}>
          <b>{k.n}</b>
          <span>{k.label}</span>
        </button>
      ))}
    </div>
  );
}

export type Opt = { key: string; label: string; n: number; color?: string };

/** Clickable horizontal bars: one per option, filtering when clicked. */
export function BarList({ opts, active, onToggle }: { opts: Opt[]; active: Set<string>; onToggle: (key: string) => void }) {
  const max = Math.max(1, ...opts.map((o) => o.n));
  return (
    <div className="mx-bars">
      {opts.map((o) => (
        <button type="button" key={o.key} className={`mx-bar ${active.has(o.key) ? "on" : ""} ${o.n === 0 ? "zero" : ""}`} onClick={() => onToggle(o.key)} title={`Filter to ${o.label}`}>
          <span className="mx-bar-label"><i style={{ background: o.color ?? "var(--accent)" }} />{o.label}</span>
          <span className="mx-bar-track"><span style={{ width: `${(o.n / max) * 100}%`, background: o.color ?? "var(--accent)" }} /></span>
          <span className="mx-bar-n">{o.n}</span>
        </button>
      ))}
    </div>
  );
}

/** One slicer row: a title and toggle pills with live counts. */
export function Pills({ title, opts, active, onToggle }: { title: string; opts: Opt[]; active: Set<string>; onToggle: (key: string) => void }) {
  return (
    <div className="mx-slicer">
      <span className="mx-slicer-title">{title}</span>
      {opts.map((o) => (
        <button type="button" key={o.key} className={`mx-pill ${active.has(o.key) ? "on" : ""}`} onClick={() => onToggle(o.key)} disabled={o.n === 0 && !active.has(o.key)}>
          {o.color && <i style={{ background: o.color }} />}
          {o.label} <em>{o.n}</em>
        </button>
      ))}
    </div>
  );
}
