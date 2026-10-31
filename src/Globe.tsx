import { useEffect, useRef } from "react";
import { geoDistance, geoGraticule10, geoOrthographic, geoPath } from "d3-geo";
import { feature } from "topojson-client";
import land110 from "world-atlas/land-110m.json";

export type GlobePoint = {
  id: string;
  lon: number;
  lat: number;
  color: string;
  /** Layer marks: translucent disc with this radius in pixels (fires, conflict). */
  radius?: number;
  kind: "event" | "fire" | "conflict" | "hazard";
  faded?: boolean;
};
export type GlobeFocus = { lon: number; lat: number; nonce: number };

// eslint-disable-next-line @typescript-eslint/no-explicit-any
const landShape = feature(land110 as any, (land110 as any).objects.land);
const graticule = geoGraticule10();
const CLUSTER_PX = 30;

/** The rotation that centres `lon`, reached by the shortest way round from `from`. */
const rxFor = (from: number, lon: number) => from + ((((-lon - from) % 360) + 540) % 360) - 180;

type Hit = { x: number; y: number; r: number; ids: string[]; lon: number; lat: number };

/** An orthographic globe drawn on a canvas: continent outlines only, draggable, with the points overlaid. */
export default function Globe({ points, selectedId, focus, spin, onPick }: { points: GlobePoint[]; selectedId: string | null; focus: GlobeFocus | null; spin: boolean; onPick: (id: string) => void }) {
  const wrap = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const view = useRef({ rx: 20, ry: -20, k: 1, w: 600, h: 520, base: 240 });
  const hits = useRef<Hit[]>([]);
  const props = useRef({ points, selectedId, spin, onPick });
  props.current = { points, selectedId, spin, onPick };
  const dirty = useRef(true);
  const anim = useRef<{ rx: number; ry: number; k: number; t: number } | null>(null);
  const idle = useRef(0);

  useEffect(() => {
    dirty.current = true;
  }, [points, selectedId]);

  useEffect(() => {
    if (!focus) return;
    const v = view.current;
    anim.current = { rx: rxFor(v.rx, focus.lon), ry: Math.max(-85, Math.min(85, -focus.lat)), k: Math.max(v.k, 1.8), t: 0 };
    dirty.current = true;
  }, [focus]);

  useEffect(() => {
    const el = canvas.current!;
    const box = wrap.current!;
    const ctx = el.getContext("2d")!;
    const style = getComputedStyle(document.documentElement);
    const accent = style.getPropertyValue("--accent").trim() || "#f0b98a";

    const resize = () => {
      const r = box.getBoundingClientRect();
      const dpr = window.devicePixelRatio || 1;
      view.current.w = Math.max(280, r.width);
      view.current.h = Math.max(320, r.height);
      view.current.base = Math.min(view.current.w, view.current.h) / 2 - 14;
      el.width = view.current.w * dpr;
      el.height = view.current.h * dpr;
      el.style.width = `${view.current.w}px`;
      el.style.height = `${view.current.h}px`;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      dirty.current = true;
    };
    const ro = new ResizeObserver(resize);
    ro.observe(box);
    resize();

    let raf = 0;
    const draw = () => {
      raf = requestAnimationFrame(draw);
      const v = view.current;
      const a = anim.current;
      if (a) {
        a.t = Math.min(1, a.t + 0.04);
        const e = 1 - Math.pow(1 - a.t, 3);
        v.rx += (a.rx - v.rx) * e * 0.35 + 0;
        v.ry += (a.ry - v.ry) * e * 0.35;
        v.k += (a.k - v.k) * e * 0.35;
        if (a.t >= 1 && Math.abs(a.rx - v.rx) < 0.05 && Math.abs(a.ry - v.ry) < 0.05) anim.current = null;
        dirty.current = true;
      } else if (props.current.spin && Date.now() - idle.current > 4000) {
        v.rx += 0.12;
        dirty.current = true;
      }
      if (!dirty.current) return;
      dirty.current = false;

      const { w, h, base } = v;
      const scale = base * v.k;
      const proj = geoOrthographic().translate([w / 2, h / 2]).scale(scale).clipAngle(90).rotate([v.rx, v.ry, 0]);
      const path = geoPath(proj, ctx);
      ctx.clearRect(0, 0, w, h);

      // ocean
      const cx = w / 2, cy = h / 2;
      const g = ctx.createRadialGradient(cx - scale * 0.35, cy - scale * 0.4, scale * 0.1, cx, cy, scale);
      g.addColorStop(0, "#14384a");
      g.addColorStop(1, "#08141f");
      ctx.beginPath();
      path({ type: "Sphere" });
      ctx.fillStyle = g;
      ctx.fill();
      ctx.strokeStyle = "rgba(255,255,255,0.18)";
      ctx.lineWidth = 1;
      ctx.stroke();
      // graticule
      ctx.beginPath();
      path(graticule);
      ctx.strokeStyle = "rgba(255,255,255,0.06)";
      ctx.lineWidth = 0.6;
      ctx.stroke();
      // continents (outline only, a faint fill)
      ctx.beginPath();
      path(landShape);
      ctx.fillStyle = "rgba(94,160,148,0.13)";
      ctx.fill();
      ctx.strokeStyle = "rgba(150,214,198,0.75)";
      ctx.lineWidth = 1.1;
      ctx.stroke();

      // points
      const center: [number, number] = [-v.rx, -v.ry];
      const visible = props.current.points
        .map((p) => ({ p, xy: geoDistance([p.lon, p.lat], center) < Math.PI / 2 - 0.02 ? proj([p.lon, p.lat]) : null }))
        .filter((o): o is { p: GlobePoint; xy: [number, number] } => !!o.xy);
      // layers under the events
      for (const { p, xy } of visible) {
        if (p.kind === "fire" || p.kind === "conflict") {
          const r = Math.max(5, (p.radius ?? 8) * Math.min(2.4, v.k));
          ctx.beginPath();
          ctx.arc(xy[0], xy[1], r, 0, 7);
          ctx.fillStyle = p.color + (p.faded ? "33" : "59");
          ctx.fill();
          ctx.strokeStyle = p.color;
          ctx.lineWidth = 1;
          ctx.stroke();
        } else if (p.kind === "hazard") {
          ctx.font = "16px sans-serif";
          ctx.textAlign = "center";
          ctx.textBaseline = "middle";
          ctx.fillStyle = p.color;
          ctx.fillText("⚠", xy[0], xy[1]);
        }
      }
      // events, clustered by screen cell
      const cells = new Map<string, { xs: number; ys: number; list: { p: GlobePoint; xy: [number, number] }[] }>();
      for (const o of visible) {
        if (o.p.kind !== "event") continue;
        const k = `${Math.floor(o.xy[0] / CLUSTER_PX)}:${Math.floor(o.xy[1] / CLUSTER_PX)}`;
        const c = cells.get(k) ?? { xs: 0, ys: 0, list: [] };
        c.xs += o.xy[0];
        c.ys += o.xy[1];
        c.list.push(o);
        cells.set(k, c);
      }
      const newHits: Hit[] = [];
      for (const c of cells.values()) {
        const n = c.list.length;
        const x = c.xs / n, y = c.ys / n;
        if (n === 1) {
          const { p } = c.list[0];
          const sel = p.id === props.current.selectedId;
          if (sel) {
            ctx.beginPath();
            ctx.arc(x, y, 13, 0, 7);
            ctx.strokeStyle = accent;
            ctx.lineWidth = 2;
            ctx.stroke();
          }
          ctx.beginPath();
          ctx.arc(x, y, 6.5, 0, 7);
          ctx.fillStyle = p.color;
          ctx.globalAlpha = p.faded ? 0.5 : 1;
          ctx.fill();
          ctx.globalAlpha = 1;
          ctx.strokeStyle = "#fff";
          ctx.lineWidth = 1.6;
          ctx.stroke();
          newHits.push({ x, y, r: 11, ids: [p.id], lon: p.lon, lat: p.lat });
        } else {
          ctx.beginPath();
          ctx.arc(x, y, 14, 0, 7);
          ctx.fillStyle = "rgba(16,24,32,0.9)";
          ctx.fill();
          ctx.strokeStyle = "#fff";
          ctx.lineWidth = 1.8;
          ctx.stroke();
          ctx.fillStyle = "#fff";
          ctx.font = "bold 12px sans-serif";
          ctx.textAlign = "center";
          ctx.textBaseline = "middle";
          ctx.fillText(String(n), x, y + 0.5);
          const lon = c.list.reduce((s, o) => s + o.p.lon, 0) / n;
          const lat = c.list.reduce((s, o) => s + o.p.lat, 0) / n;
          newHits.push({ x, y, r: 16, ids: c.list.map((o) => o.p.id), lon, lat });
        }
      }
      hits.current = newHits;
    };
    raf = requestAnimationFrame(draw);

    // drag to rotate, wheel to zoom
    let drag: { x: number; y: number; moved: number } | null = null;
    const down = (e: PointerEvent) => {
      el.setPointerCapture(e.pointerId);
      drag = { x: e.clientX, y: e.clientY, moved: 0 };
      anim.current = null;
      idle.current = Date.now();
    };
    const move = (e: PointerEvent) => {
      if (!drag) return;
      const dx = e.clientX - drag.x, dy = e.clientY - drag.y;
      drag.x = e.clientX;
      drag.y = e.clientY;
      drag.moved += Math.abs(dx) + Math.abs(dy);
      const v = view.current;
      const f = 0.3 / v.k;
      v.rx += dx * f;
      v.ry = Math.max(-85, Math.min(85, v.ry - dy * f));
      idle.current = Date.now();
      dirty.current = true;
    };
    const up = (e: PointerEvent) => {
      const d = drag;
      drag = null;
      if (!d || d.moved > 5) return;
      const r = el.getBoundingClientRect();
      const x = e.clientX - r.left, y = e.clientY - r.top;
      const hit = hits.current.find((h) => Math.hypot(h.x - x, h.y - y) <= h.r);
      if (!hit) return;
      if (hit.ids.length === 1) props.current.onPick(hit.ids[0]);
      else {
        const v = view.current;
        anim.current = { rx: rxFor(v.rx, hit.lon), ry: Math.max(-85, Math.min(85, -hit.lat)), k: Math.min(8, v.k * 2.2), t: 0 };
      }
    };
    const wheel = (e: WheelEvent) => {
      e.preventDefault();
      const v = view.current;
      v.k = Math.max(0.8, Math.min(9, v.k * Math.exp(-e.deltaY * 0.0014)));
      idle.current = Date.now();
      dirty.current = true;
    };
    el.addEventListener("pointerdown", down);
    el.addEventListener("pointermove", move);
    el.addEventListener("pointerup", up);
    el.addEventListener("wheel", wheel, { passive: false });
    return () => {
      cancelAnimationFrame(raf);
      ro.disconnect();
      el.removeEventListener("pointerdown", down);
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", up);
      el.removeEventListener("wheel", wheel);
    };
  }, []);

  const zoom = (f: number) => {
    view.current.k = Math.max(0.8, Math.min(9, view.current.k * f));
    dirty.current = true;
  };
  return (
    <div className="gl-wrap" ref={wrap}>
      <canvas ref={canvas} className="gl-canvas" aria-label="Globe of news events: drag to rotate, scroll to zoom" role="img" />
      <div className="gl-zoom">
        <button type="button" onClick={() => zoom(1.4)} aria-label="Zoom in">+</button>
        <button type="button" onClick={() => zoom(1 / 1.4)} aria-label="Zoom out">−</button>
      </div>
    </div>
  );
}
