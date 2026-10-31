/**
 * Procedural "album art" for dashboard cards: layered paper-cut waves, a glowing orb, flowing
 * ribbons and drifting petals, drawn as an SVG from a seeded random generator so the same card
 * always gets the same picture. Original artwork (no images are stored or fetched).
 */

type Palette = { bg: [string, string]; deep: string; mid: string; light: string; accent: string; warm: string; cream: string };

const PALETTES: Record<string, Palette> = {
  task: { bg: ["#0c2225", "#143b3a"], deep: "#12373a", mid: "#2a6f69", light: "#5fa596", accent: "#e8875a", warm: "#f0b27a", cream: "#f6d9b8" },
  event: { bg: ["#170f22", "#2c1740"], deep: "#2a1740", mid: "#5b3084", light: "#b0517a", accent: "#f08a5d", warm: "#f6b87a", cream: "#fbe0b8" },
  sprint: { bg: ["#0a1a2c", "#12304c"], deep: "#12304c", mid: "#2a6290", light: "#5fa3b8", accent: "#e7b56d", warm: "#efcf8e", cream: "#f7e8c4" },
  shape: { bg: ["#0b2229", "#103a44"], deep: "#0f3b47", mid: "#2b7d86", light: "#6fb8b4", accent: "#f2a65a", warm: "#f6c987", cream: "#fbe7c4" },
  week: { bg: ["#121a33", "#1f2d57"], deep: "#1d2c5a", mid: "#4256a8", light: "#7f8fd6", accent: "#f08fa0", warm: "#f5b9a8", cream: "#fbe1d6" },
  news: { bg: ["#15181d", "#26303b"], deep: "#252f3b", mid: "#4d6478", light: "#8aa3b5", accent: "#e2a24a", warm: "#efc784", cream: "#f6e4c0" },
  signals: { bg: ["#220f14", "#3b1a22"], deep: "#3a1a21", mid: "#8a3a3f", light: "#c9695a", accent: "#f0a35c", warm: "#f5c58a", cream: "#fbe3c4" },
  leads: { bg: ["#1c1309", "#34240f"], deep: "#33240f", mid: "#8a5a22", light: "#c9944a", accent: "#4fb39a", warm: "#f0cd7a", cream: "#f8ecc4" },
  rhythm: { bg: ["#0e1d2e", "#17324a"], deep: "#16324a", mid: "#2f6f94", light: "#74b0c4", accent: "#d6a0e0", warm: "#f0c6ec", cream: "#f8e6f2" },
  countdowns: { bg: ["#1d1030", "#3a1b4a"], deep: "#38194a", mid: "#7a3a7a", light: "#c46a8a", accent: "#f2a15c", warm: "#f7c58a", cream: "#fce2c6" },
  network: { bg: ["#0d1f1f", "#16393a"], deep: "#153838", mid: "#2f7a72", light: "#6ec0a8", accent: "#7ad0ff", warm: "#b6e6d8", cream: "#e2f6ee" },
  saved: { bg: ["#112116", "#1c3a26"], deep: "#1b3a26", mid: "#3c7a50", light: "#86c08a", accent: "#e8c26a", warm: "#f0d78e", cream: "#f8efc8" },
  desert: { bg: ["#2a150d", "#4a2414"], deep: "#4a2414", mid: "#a8552e", light: "#d98a55", accent: "#f4c26a", warm: "#f7d9a0", cream: "#fdeccc" },
  dusk: { bg: ["#141430", "#2a2152"], deep: "#2a2152", mid: "#5b4a9c", light: "#9a86d6", accent: "#f2886a", warm: "#f7b894", cream: "#fde0cc" },
  backlog: { bg: ["#1f170d", "#3a2a14"], deep: "#3a2a14", mid: "#8a6428", light: "#c9a05a", accent: "#7ec8a8", warm: "#f0d28e", cream: "#f8ecc8" },
  finance: { bg: ["#0b2018", "#13392b"], deep: "#13392b", mid: "#2f7a56", light: "#6fc29a", accent: "#e8c95a", warm: "#f2dd8e", cream: "#faf0c4" },
  day: { bg: ["#0c2225", "#143b3a"], deep: "#12373a", mid: "#2a6f69", light: "#5fa596", accent: "#e8875a", warm: "#f0b27a", cream: "#f6d9b8" },
  workflows: { bg: ["#101c26", "#1b3447"], deep: "#1b3447", mid: "#3a6f93", light: "#79aecb", accent: "#f2a65a", warm: "#f6c987", cream: "#fbe7c4" },
  zotero: { bg: ["#24101a", "#401b2c"], deep: "#401b2c", mid: "#8a3a58", light: "#c9707e", accent: "#e8c26a", warm: "#f2d58e", cream: "#faecc8" },
  weather: { bg: ["#0e2236", "#1c4468"], deep: "#1b4468", mid: "#3d7fae", light: "#86bfdc", accent: "#f4a56a", warm: "#f8cf94", cream: "#fdeccf" },
  mail: { bg: ["#101a30", "#1d2f57"], deep: "#1c2f5a", mid: "#3f5fa8", light: "#7f9bd6", accent: "#f0a58a", warm: "#f6c9b0", cream: "#fce6d8" },
  loki: { bg: ["#0b1f16", "#13402b"], deep: "#12402b", mid: "#2b7a4b", light: "#5fb27a", accent: "#e3b94f", warm: "#efcf6a", cream: "#f7ecb8" },
};

function hash(str: string): number {
  let h = 1779033703 ^ str.length;
  for (let i = 0; i < str.length; i++) {
    h = Math.imul(h ^ str.charCodeAt(i), 3432918353);
    h = (h << 13) | (h >>> 19);
  }
  return (h ^ (h >>> 16)) >>> 0;
}

function rng(seed: string) {
  let a = hash(seed);
  return () => {
    a |= 0;
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const W = 1200;
const H = 640;

function wave(r: () => number, base: number, amp: number, fill: string, opacity: number): string {
  const steps = 5;
  let d = `M -60 ${H + 20} L -60 ${base}`;
  let x = -60;
  const dx = (W + 120) / steps;
  for (let i = 0; i < steps; i++) {
    const y1 = base - amp * (0.4 + r());
    const y2 = base + amp * (r() - 0.6);
    const yEnd = base + amp * (r() - 0.5);
    d += ` C ${(x + dx * 0.35).toFixed(0)} ${y1.toFixed(0)}, ${(x + dx * 0.65).toFixed(0)} ${y2.toFixed(0)}, ${(x + dx).toFixed(0)} ${yEnd.toFixed(0)}`;
    x += dx;
  }
  d += ` L ${W + 60} ${H + 20} Z`;
  return `<path d="${d}" fill="${fill}" fill-opacity="${opacity}"/>`;
}

function ribbon(r: () => number, p: Palette, widthScale: number): string {
  const x0 = -40 + r() * 200;
  const y0 = H * (0.7 + r() * 0.3);
  const c1x = W * (0.2 + r() * 0.3), c1y = H * (r() * 0.5);
  const c2x = W * (0.5 + r() * 0.3), c2y = H * (0.5 + r() * 0.6);
  const x1 = W * (0.75 + r() * 0.3), y1 = H * (r() * 0.6);
  const d = `M ${x0.toFixed(0)} ${y0.toFixed(0)} C ${c1x.toFixed(0)} ${c1y.toFixed(0)}, ${c2x.toFixed(0)} ${c2y.toFixed(0)}, ${x1.toFixed(0)} ${y1.toFixed(0)}`;
  const colors = [p.cream, p.warm, p.accent, p.light];
  const w = (16 + r() * 28) * widthScale;
  let out = "";
  for (let i = 0; i < 4; i++) {
    out += `<path d="${d}" fill="none" stroke="${colors[(i + Math.floor(r() * 4)) % 4]}" stroke-opacity="${(0.55 + r() * 0.35).toFixed(2)}" stroke-width="${(w * (1 - i * 0.22)).toFixed(1)}" stroke-linecap="round" transform="translate(${(i * 7).toFixed(0)} ${(i * 9).toFixed(0)})"/>`;
  }
  return out;
}

function petal(r: () => number, p: Palette, cx: number, cy: number, size: number): string {
  const colors = [p.cream, p.warm, p.accent, p.light, p.mid];
  const fill = colors[Math.floor(r() * colors.length)];
  const rot = (r() * 360).toFixed(0);
  // teardrop: pointed at the top, round at the bottom
  const s = size;
  return `<path d="M 0 ${-s} C ${s * 0.9} ${-s * 0.2}, ${s * 0.7} ${s * 0.9}, 0 ${s * 0.9} C ${-s * 0.7} ${s * 0.9}, ${-s * 0.9} ${-s * 0.2}, 0 ${-s} Z" fill="${fill}" fill-opacity="${(0.75 + r() * 0.25).toFixed(2)}" transform="translate(${cx.toFixed(0)} ${cy.toFixed(0)}) rotate(${rot})"/>`;
}


type Ctx = { r: () => number; p: Palette; seed: string };

const f = (n: number) => n.toFixed(0);

function stars(c: Ctx, n: number): string {
  let o = "";
  for (let i = 0; i < n; i++) o += `<circle cx="${f(c.r() * W)}" cy="${f(c.r() * H * 0.7)}" r="${(0.8 + c.r() * 2.2).toFixed(1)}" fill="${c.p.cream}" fill-opacity="${(0.4 + c.r() * 0.6).toFixed(2)}"/>`;
  return o;
}

function orb(_c: Ctx, x: number, y: number, r: number): string {
  return `<circle cx="${f(x)}" cy="${f(y)}" r="${f(r * 1.9)}" fill="url(#glow)"/><circle cx="${f(x)}" cy="${f(y)}" r="${f(r)}" fill="url(#orb)"/>`;
}

function hills(c: Ctx): string {
  const { r, p } = c;
  let body = wave(r, H * 0.52, 90, p.light, 0.55);
  body += orb(c, W * (0.55 + r() * 0.3), H * (0.2 + r() * 0.2), 70 + r() * 70);
  body += wave(r, H * 0.62, 80, p.mid, 0.85) + ribbon(r, p, 1) + wave(r, H * 0.74, 70, p.deep, 0.95) + ribbon(r, p, 0.7) + wave(r, H * 0.86, 50, p.bg[0], 1);
  const n = 16 + Math.floor(r() * 8);
  for (let i = 0; i < n; i++) {
    const t = i / (n - 1);
    body += petal(r, p, W * (0.2 + t * 0.75) + (r() - 0.5) * 160, H * (0.85 - t * 0.65 + Math.sin(t * 6) * 0.08) + (r() - 0.5) * 120, 8 + r() * 26);
  }
  return body;
}

function ocean(c: Ctx): string {
  const { r, p } = c;
  let b = stars(c, 40) + orb(c, W * (0.3 + r() * 0.4), H * 0.26, 60 + r() * 40);
  for (let i = 0; i < 6; i++) {
    const y = H * (0.5 + i * 0.09);
    let d = `M -20 ${f(y)}`;
    for (let x = -20; x < W + 40; x += 80) d += ` q 40 ${f(-18 - r() * 22)} 80 0`;
    d += ` L ${W + 40} ${H + 10} L -20 ${H + 10} Z`;
    b += `<path d="${d}" fill="${[p.light, p.mid, p.mid, p.deep, p.deep, p.bg[0]][i]}" fill-opacity="${(0.5 + i * 0.1).toFixed(2)}"/>`;
  }
  const mx = W * (0.3 + r() * 0.4);
  for (let i = 0; i < 9; i++) b += `<rect x="${f(mx - 40 + r() * 80)}" y="${f(H * 0.52 + i * 22)}" width="${f(20 + r() * 50)}" height="4" rx="2" fill="${p.cream}" fill-opacity="${(0.55 - i * 0.05).toFixed(2)}"/>`;
  return b;
}

function city(c: Ctx): string {
  const { r, p } = c;
  let b = stars(c, 25) + orb(c, W * (0.65 + r() * 0.2), H * 0.3, 60 + r() * 40);
  for (let layer = 0; layer < 3; layer++) {
    let x = -20;
    const base = H * (0.78 + layer * 0.05);
    const col = [p.mid, p.deep, p.bg[0]][layer];
    while (x < W + 20) {
      const w = 40 + r() * 90, h = (70 + r() * 200) * (1 - layer * 0.22);
      b += `<rect x="${f(x)}" y="${f(base - h)}" width="${f(w)}" height="${f(h + 80)}" fill="${col}" fill-opacity="${(0.7 + layer * 0.15).toFixed(2)}"/>`;
      if (layer === 2) for (let k = 0; k < w / 14; k++) if (r() > 0.55) b += `<rect x="${f(x + 6 + k * 14)}" y="${f(base - h + 10 + r() * (h - 20))}" width="6" height="8" fill="${p.warm}" fill-opacity="0.85"/>`;
      x += w + 4 + r() * 10;
    }
  }
  return b;
}

function constellation(c: Ctx): string {
  const { r, p } = c;
  let b = stars(c, 70) + orb(c, W * (0.72 + r() * 0.15), H * (0.3 + r() * 0.15), 50 + r() * 40);
  for (let i = 0; i < 3; i++) b += `<ellipse cx="${f(W * 0.7)}" cy="${f(H * 0.35)}" rx="${f(220 + i * 110)}" ry="${f(70 + i * 38)}" fill="none" stroke="${p.warm}" stroke-opacity="${(0.28 - i * 0.07).toFixed(2)}" stroke-width="2" transform="rotate(${f(-18 + i * 4)} ${f(W * 0.7)} ${f(H * 0.35)})"/>`;
  const pts: [number, number][] = [];
  for (let i = 0; i < 9; i++) pts.push([W * (0.08 + i * 0.105) + (r() - 0.5) * 60, H * (0.2 + r() * 0.55)]);
  b += `<polyline points="${pts.map((q) => `${f(q[0])},${f(q[1])}`).join(" ")}" fill="none" stroke="${p.light}" stroke-opacity="0.55" stroke-width="2"/>`;
  for (const [x, y] of pts) b += `<circle cx="${f(x)}" cy="${f(y)}" r="${(5 + r() * 6).toFixed(1)}" fill="${p.cream}"/><circle cx="${f(x)}" cy="${f(y)}" r="16" fill="${p.warm}" fill-opacity="0.14"/>`;
  return b + wave(r, H * 0.9, 40, p.bg[0], 1);
}

function contour(c: Ctx): string {
  const { r, p } = c;
  const cx = W * (0.55 + r() * 0.25), cy = H * (0.45 + r() * 0.15);
  const phase = [r() * 6, r() * 6, r() * 6];
  let b = "";
  for (let k = 12; k >= 1; k--) {
    let d = "";
    for (let a = 0; a <= 64; a++) {
      const t = (a / 64) * Math.PI * 2;
      const rad = k * 32 * (1 + 0.18 * Math.sin(3 * t + phase[0]) + 0.1 * Math.sin(5 * t + phase[1]) + 0.07 * Math.sin(2 * t + phase[2]));
      d += `${a ? "L" : "M"}${f(cx + Math.cos(t) * rad * 1.6)} ${f(cy + Math.sin(t) * rad)} `;
    }
    b += `<path d="${d}Z" fill="${k % 3 === 0 ? p.mid : "none"}" fill-opacity="0.18" stroke="${k % 4 === 0 ? p.accent : p.light}" stroke-opacity="${(0.25 + (12 - k) * 0.05).toFixed(2)}" stroke-width="${k % 4 === 0 ? 3 : 1.5}"/>`;
  }
  for (let i = 0; i < 5; i++) b += `<circle cx="${f(cx + (r() - 0.5) * 520)}" cy="${f(cy + (r() - 0.5) * 300)}" r="7" fill="${p.warm}"/><circle cx="${f(cx + (r() - 0.5) * 520)}" cy="${f(cy + (r() - 0.5) * 300)}" r="18" fill="none" stroke="${p.warm}" stroke-opacity="0.5" stroke-width="2"/>`;
  return b;
}

function dunes(c: Ctx): string {
  const { r, p } = c;
  let b = orb(c, W * (0.25 + r() * 0.5), H * (0.3 + r() * 0.1), 80 + r() * 40);
  [0.55, 0.66, 0.78, 0.9].forEach((y, i) => {
    b += wave(r, H * y, 55 - i * 8, [p.light, p.mid, p.deep, p.bg[0]][i], 0.9);
  });
  for (let i = 0; i < 5; i++) {
    const x = W * (0.1 + r() * 0.8), y = H * (0.12 + r() * 0.25);
    b += `<path d="M ${f(x - 16)} ${f(y)} q 8 -10 16 0 q 8 -10 16 0" fill="none" stroke="${p.cream}" stroke-width="3" stroke-linecap="round" stroke-opacity="0.8"/>`;
  }
  return b;
}

function vortex(c: Ctx): string {
  const { r, p } = c;
  const cx = W * (0.62 + r() * 0.15), cy = H * 0.45;
  let b = stars(c, 20);
  for (let k = 0; k < 7; k++) {
    let d = "";
    for (let a = 0; a < 90; a++) {
      const t = a / 90, ang = t * Math.PI * 5 + k * 0.9, rad = 20 + t * (210 + k * 16);
      d += `${a ? "L" : "M"}${f(cx + Math.cos(ang) * rad * 1.4)} ${f(cy + Math.sin(ang) * rad * 0.62)} `;
    }
    b += `<path d="${d}" fill="none" stroke="${[p.cream, p.warm, p.accent, p.light][k % 4]}" stroke-opacity="${(0.75 - k * 0.07).toFixed(2)}" stroke-width="${(14 - k).toFixed(0)}" stroke-linecap="round"/>`;
  }
  return b + `<circle cx="${f(cx)}" cy="${f(cy)}" r="30" fill="${p.bg[0]}"/>` + wave(r, H * 0.9, 40, p.bg[0], 1);
}

function trail(c: Ctx): string {
  const { r, p } = c;
  let b = orb(c, W * (0.7 + r() * 0.15), H * (0.22 + r() * 0.1), 60 + r() * 40);
  b += wave(r, H * 0.5, 100, p.light, 0.6) + wave(r, H * 0.62, 90, p.mid, 0.9);
  const sx = W * (0.15 + r() * 0.2);
  const d = `M ${f(sx)} ${H + 20} C ${f(sx + 200)} ${f(H * 0.85)}, ${f(W * 0.2)} ${f(H * 0.7)}, ${f(W * 0.45)} ${f(H * 0.68)} S ${f(W * 0.8)} ${f(H * 0.58)}, ${f(W * 0.88)} ${f(H * 0.5)}`;
  b += `<path d="${d}" fill="none" stroke="${p.bg[0]}" stroke-width="64" stroke-linecap="round" stroke-opacity="0.9"/><path d="${d}" fill="none" stroke="${p.warm}" stroke-width="30" stroke-linecap="round"/><path d="${d}" fill="none" stroke="${p.cream}" stroke-width="3" stroke-dasharray="14 18" stroke-linecap="round"/>`;
  b += wave(r, H * 0.88, 50, p.deep, 0.95);
  for (let i = 0; i < 10; i++) b += petal(r, p, W * (0.1 + r() * 0.8), H * (0.55 + r() * 0.4), 6 + r() * 14);
  return b;
}

function aurora(c: Ctx): string {
  const { r, p } = c;
  let b = stars(c, 60);
  for (let k = 0; k < 5; k++) {
    const y0 = H * (0.18 + k * 0.07);
    let d = `M -40 ${f(y0)}`;
    for (let x = -40; x <= W + 40; x += 160) d += ` C ${f(x + 50)} ${f(y0 - 110 * r())}, ${f(x + 110)} ${f(y0 + 110 * r())}, ${f(x + 160)} ${f(y0)}`;
    b += `<path d="${d} L ${W + 40} ${f(y0 + 200)} L -40 ${f(y0 + 200)} Z" fill="${[p.light, p.accent, p.warm, p.mid, p.cream][k]}" fill-opacity="${(0.16 - k * 0.015).toFixed(3)}"/>`;
    b += `<path d="${d}" fill="none" stroke="${[p.cream, p.warm, p.light, p.accent, p.mid][k]}" stroke-opacity="0.5" stroke-width="${6 - k}" stroke-linecap="round"/>`;
  }
  b += wave(r, H * 0.78, 90, p.deep, 0.95) + wave(r, H * 0.9, 50, p.bg[0], 1);
  for (let i = 0; i < 14; i++) {
    const x = W * r(), y = H * (0.8 + r() * 0.15), h = 30 + r() * 60;
    b += `<path d="M ${f(x)} ${f(y - h)} l 14 ${f(h)} h -28 Z" fill="${p.bg[0]}"/>`;
  }
  return b;
}

function chart(c: Ctx): string {
  const { r, p } = c;
  let b = stars(c, 14);
  for (let g = 1; g < 6; g++) b += `<line x1="0" x2="${W}" y1="${f(g * H * 0.15)}" y2="${f(g * H * 0.15)}" stroke="${p.cream}" stroke-opacity="0.07"/>`;
  [p.mid, p.accent].forEach((col, k) => {
    let y = H * (0.6 - k * 0.1), d = `M -10 ${f(y)}`;
    for (let x = 0; x <= W + 20; x += 60) {
      y = Math.max(H * 0.15, Math.min(H * 0.85, y + (r() - 0.5) * 120 - 6));
      d += ` L ${f(x)} ${f(y)}`;
    }
    b += `<path d="${d} L ${W + 20} ${H} L -10 ${H} Z" fill="${col}" fill-opacity="${k ? 0.1 : 0.28}"/><path d="${d}" fill="none" stroke="${k ? p.warm : p.light}" stroke-width="4" stroke-linejoin="round" stroke-linecap="round"/>`;
  });
  return b + orb(c, W * 0.85, H * 0.22, 48) + wave(r, H * 0.93, 30, p.bg[0], 1);
}

function forest(c: Ctx): string {
  const { r, p } = c;
  let b = orb(c, W * (0.2 + r() * 0.6), H * 0.25, 60 + r() * 40);
  b += wave(r, H * 0.56, 80, p.light, 0.5);
  for (let layer = 0; layer < 3; layer++) {
    const base = H * (0.7 + layer * 0.08);
    const col = [p.mid, p.deep, p.bg[0]][layer];
    for (let x = -20; x < W + 30; x += 22 + r() * 26) {
      const h = (90 + r() * 150) * (1 - layer * 0.18);
      b += `<path d="M ${f(x)} ${f(base - h)} l ${f(h * 0.2)} ${f(h)} h ${f(-h * 0.4)} Z" fill="${col}"/><path d="M ${f(x)} ${f(base - h * 0.7)} l ${f(h * 0.26)} ${f(h * 0.6)} h ${f(-h * 0.52)} Z" fill="${col}"/>`;
    }
  }
  return b;
}

type Motif = (c: Ctx) => string;
const MOTIFS: Record<string, Motif> = { hills, ocean, city, constellation, contour, dunes, vortex, trail, aurora, chart, forest };
const MOTIF_PALETTE: Record<string, string> = { ocean: "rhythm", city: "news", constellation: "signals", contour: "leads", dunes: "desert", vortex: "signals", trail: "desert", aurora: "countdowns", chart: "sprint", forest: "saved", hills: "task" };
const KIND_MOTIF: Record<string, string> = { mail: "aurora", workflows: "constellation", zotero: "forest", weather: "hills", day: "trail", backlog: "forest", finance: "chart", shape: "trail", week: "aurora", countdowns: "aurora", rhythm: "ocean", news: "city", signals: "vortex", leads: "contour", saved: "forest", network: "constellation", loki: "constellation", start: "trail" };

/** Picks the scene from what the card is about; the seed decides among the neutral ones. */
export function motifFor(kind: string, text: string, seed: string): string {
  const t = text.toLowerCase();
  const rules: [RegExp, string][] = [
    [/\b(run|workout|deka|gym|lift|training|fitness|hike|ruck|mile)\b/, "trail"],
    [/(cyclone|storm|hurricane|hotspot|wildfire|earthquake|flood|outage|alert)/, "vortex"],
    [/(oil|market|yield|bond|econom|inflation|rates?\b|stock|price|wti|fed\b)/, "chart"],
    [/(news|ukraine|iran|war|election|politic|defen[cs]e|europe|africa|china|russia|brief)/, "city"],
    [/(loki|deploy|server|hook|code|build|fix|dashboard|mcp|model|agent|satellite|orbit)/, "constellation"],
    [/(lead|client|hire|quote|n8n|automation|invoice|gig)/, "contour"],
    [/(sleep|rest|cold|recover|weight|waist|meditat|endel|reset)/, "ocean"],
    [/(house|home|bowie|bruno|sami|family|kids|laundry|camp|garden|plates)/, "dunes"],
    [/(read|note|source|saved|learn|lesson|study|research|inbox|transcript)/, "forest"],
    [/(call|email|memo|jrap|csc|class|lecture|sprint|week|deadline|due)/, "aurora"],
  ];
  const hit = rules.find(([re]) => re.test(t));
  if (hit) return hit[1];
  if (KIND_MOTIF[kind]) return KIND_MOTIF[kind];
  const neutral = ["hills", "ocean", "dunes", "aurora", "forest", "contour", "trail"];
  return neutral[hash(seed) % neutral.length];
}

export function artSvg(seed: string, kind: string, text = "", rounded = 0): string {
  const motif = motifFor(kind, text, seed);
  const p = PALETTES[kind] && !["task", "event", "sprint"].includes(kind) ? PALETTES[kind] : PALETTES[MOTIF_PALETTE[motif]] ?? PALETTES[kind] ?? PALETTES.task;
  const r = rng(`${kind}:${seed}`);
  const body = (MOTIFS[motif] ?? hills)({ r, p, seed });
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${W} ${H}" preserveAspectRatio="xMidYMid slice">
<defs>
<linearGradient id="bg" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="${p.bg[0]}"/><stop offset="1" stop-color="${p.bg[1]}"/></linearGradient>
<radialGradient id="orb"><stop offset="0" stop-color="${p.cream}"/><stop offset="0.7" stop-color="${p.warm}"/><stop offset="1" stop-color="${p.accent}"/></radialGradient>
<radialGradient id="glow"><stop offset="0" stop-color="${p.warm}" stop-opacity="0.55"/><stop offset="1" stop-color="${p.warm}" stop-opacity="0"/></radialGradient>
<filter id="grain"><feTurbulence type="fractalNoise" baseFrequency="0.85" numOctaves="2" seed="${hash(seed) % 100}"/><feColorMatrix values="0 0 0 0 1  0 0 0 0 1  0 0 0 0 1  0 0 0 0.07 0"/></filter>
</defs>
${rounded ? `<clipPath id="rc"><rect width="${W}" height="${H}" rx="${rounded}"/></clipPath><g clip-path="url(#rc)">` : ""}
<rect width="${W}" height="${H}" fill="url(#bg)"/>
${body}
<rect width="${W}" height="${H}" filter="url(#grain)"/>
${rounded ? "</g>" : ""}
</svg>`;
}

/** A CSS `url(...)` for use as a background image. */
export function artUrl(seed: string, kind: string, text = ""): string {
  return `url("data:image/svg+xml;utf8,${encodeURIComponent(artSvg(seed, kind, text))}")`;
}
