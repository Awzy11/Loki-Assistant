import { useState } from "react";
import { artUrl } from "./art";
import Carousel, { type CardData } from "./Carousel";
import { ActionPanel, KIND_LABELS, askPrompts } from "./NeedsYou";
import { navigate } from "./router";
import { decideNeedsYou, type NeedsItem, type NeedsYou } from "./transport";

/** `/needs` — the second carousel: what needs you, with the card sheet for decisions and Ask Loki. */
export default function NeedsPage({ data, error, refresh, onAsk }: { data: NeedsYou | null; error: string | null; refresh: () => void; onAsk: (prompt: string) => void }) {
  const [open, setOpen] = useState<NeedsItem | null>(null);
  const items = data?.items ?? [];
  const cards: CardData[] = items.map((it) => ({ id: it.id, kind: it.kind, title: it.title, blurb: it.blurb ?? it.why, pill: `${KIND_LABELS[it.kind] ?? it.kind}${it.when ? ` · ${it.when}` : ""}` }));

  return (
    <div className="loki-page loki-page-carousel">
      <div className="loki-page-head">
        <h2>Needs you{data ? ` — ${data.weekday} ${data.date}` : ""}</h2>
        <button type="button" className="loki-chipbtn" onClick={() => navigate("/needs/list")}>List view</button>
      </div>
      {data?.state.note && <p className="loki-home-note">{data.state.note}</p>}
      {data?.focus.current && <p className="loki-home-note">Now {data.focus.current.start}–{data.focus.current.end}: {data.focus.current.what}</p>}
      {error && <p className="loki-home-err">{error}</p>}
      <Carousel
        cards={cards}
        emptyText={data ? "Nothing needs you right now" : "Loading…"}
        onOpen={(c) => setOpen(items.find((i) => i.id === c.id) ?? null)}
        actions={(c) => {
          const it = items.find((i) => i.id === c.id);
          if (!it) return null;
          const decide = (action: "done" | "defer" | "not_now", days?: number) => decideNeedsYou(it, action, { days }).then(refresh).catch(() => {});
          return (
            <>
              <button type="button" className="primary" onClick={() => decide("done")}>Done</button>
              <button type="button" onClick={() => decide("defer", 1)}>Tomorrow</button>
              <button type="button" onClick={() => decide("not_now")}>Not today</button>
              <button type="button" onClick={() => onAsk(askPrompts(it)[0].prompt)}>Ask Loki</button>
              <button type="button" onClick={() => setOpen(it)}>Details</button>
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
      {open && (
        <div className="loki-home-sheet-wrap" onClick={() => setOpen(null)}>
          <div className="loki-home-sheet" onClick={(e) => e.stopPropagation()} role="dialog" aria-label={open.title}>
            <div className="loki-home-sheet-art" style={{ backgroundImage: artUrl(open.id, open.kind, `${open.title} ${open.why}`) }}>
              <div className="loki-home-shade" />
              <span className="loki-home-pill">{KIND_LABELS[open.kind] ?? open.kind}{open.when ? ` · ${open.when}` : ""}</span>
              <h2>{open.title}</h2>
              <button type="button" className="loki-home-close" onClick={() => setOpen(null)} aria-label="Close">✕</button>
            </div>
            <ActionPanel
              item={open}
              onDone={() => {
                setOpen(null);
                refresh();
              }}
              onAsk={(p) => {
                setOpen(null);
                onAsk(p);
              }}
            />
          </div>
        </div>
      )}
    </div>
  );
}
