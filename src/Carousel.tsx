import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { artUrl } from "./art";

export type CardData = { id: string; kind: string; title: string; blurb: string; pill: string };

/** Album-style carousel: big cards, the neighbours peeking in; arrows, swipe, ← → and Enter. */
export default function Carousel({ cards, onOpen, emptyText, actions }: { cards: CardData[]; onOpen: (c: CardData, i: number) => void; emptyText?: string; actions?: (c: CardData) => ReactNode }) {
  const [idx, setIdx] = useState(0);
  const drag = useRef<number | null>(null);
  const go = useCallback((d: number) => setIdx((i) => Math.max(0, Math.min(cards.length - 1, i + d))), [cards.length]);

  useEffect(() => setIdx((i) => Math.min(i, Math.max(0, cards.length - 1))), [cards.length]);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement | null;
      if (t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA")) return;
      if (e.key === "ArrowRight") go(1);
      else if (e.key === "ArrowLeft") go(-1);
      else if (e.key === "Enter" && cards[idx]) onOpen(cards[idx], idx);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [go, cards, idx, onOpen]);

  return (
    <>
      <div
        className="loki-home-stage"
        onPointerDown={(e) => (drag.current = e.clientX)}
        onPointerUp={(e) => {
          if (drag.current !== null && Math.abs(e.clientX - drag.current) > 50) go(e.clientX < drag.current ? 1 : -1);
          drag.current = null;
        }}
      >
        {cards.length === 0 && (
          <div className="loki-home-card loki-home-card-center" style={{ backgroundImage: artUrl("empty", "sprint", emptyText ?? "") }}>
            <div className="loki-home-shade" />
            <div className="loki-home-copy">
              <small>All clear</small>
              <h2>{emptyText ?? "Nothing here right now"}</h2>
            </div>
          </div>
        )}
        {cards.map((c, i) => {
          const o = i - idx;
          if (Math.abs(o) > 2) return null;
          return (
            <div
              key={c.id}
              className={`loki-home-card ${o === 0 ? "loki-home-card-center" : ""}`}
              style={{
                backgroundImage: artUrl(c.id, c.kind, `${c.title} ${c.blurb}`),
                transform: `translateX(${o * 64}%) scale(${1 - Math.abs(o) * 0.1})`,
                opacity: o === 0 ? 1 : Math.abs(o) === 1 ? 0.5 : 0.2,
                zIndex: 10 - Math.abs(o),
              }}
              onClick={() => (o === 0 ? onOpen(c, i) : setIdx(i))}
              role="button"
              tabIndex={o === 0 ? 0 : -1}
              aria-label={c.title}
            >
              <div className="loki-home-shade" />
              <span className="loki-home-pill">{c.pill}</span>
              <div className="loki-home-copy">
                <h2 title={c.title}>{c.title}</h2>
                {o === 0 && actions && (
                  <div className="loki-card-actions" onClick={(e) => e.stopPropagation()} onPointerDown={(e) => e.stopPropagation()}>
                    {actions(c)}
                  </div>
                )}
              </div>
            </div>
          );
        })}
        {cards.length > 1 && (
          <>
            <button type="button" className="loki-home-arrow loki-home-prev" onClick={() => go(-1)} disabled={idx === 0} aria-label="Previous">‹</button>
            <button type="button" className="loki-home-arrow loki-home-next" onClick={() => go(1)} disabled={idx === cards.length - 1} aria-label="Next">›</button>
          </>
        )}
      </div>
      {cards.length > 1 && (
        <div className="loki-home-dots">
          {cards.map((c, i) => (
            <button type="button" key={c.id} className={i === idx ? "on" : ""} onClick={() => setIdx(i)} aria-label={`Card ${i + 1}`} />
          ))}
        </div>
      )}
    </>
  );
}
