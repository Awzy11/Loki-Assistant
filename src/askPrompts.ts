import type { Briefing, NeedsItem } from "./transport";

const NO_SEARCH = "Everything you need is above, so don't search the vault or run commands for it; if something is missing, ask me.";

/** The card's own facts as text, so the model works from what is on screen instead of hunting for it. */
function cardFacts(b: Briefing): string {
  const rows = b.rows.slice(0, 14).map((r) => `- ${r.head}${r.text ? `: ${r.text}` : ""}`);
  const series = b.series.map((s) => `- ${s.name}${s.unit ? ` (${s.unit})` : ""}, oldest to newest: ${s.points.map((p) => (p === null ? "–" : Number.isInteger(p) ? p : p.toFixed(1))).join(", ")}`);
  return [...rows, ...series].join("\n").slice(0, 2400);
}

/** "Ask Loki" on a briefing card: the card's contents plus a clear, bounded question. */
export function briefingPrompt(b: Briefing): string {
  return `This is my dashboard card "${b.title}" (${b.pill}): ${b.blurb}.\n\n${cardFacts(b)}\n\nUsing only this, tell me what stands out and what, if anything, I should do about it today. ${NO_SEARCH}`;
}

/** "Ask Loki" on a Needs-you item: where it came from and exactly which note to read. */
export function itemPrompts(item: NeedsItem): { label: string; prompt: string }[] {
  const ctx = `"${item.title}"${item.when ? ` (${item.when})` : ""}. Why it's on my list: ${item.why}.${item.next_action ? ` Next action noted: ${item.next_action}.` : ""}`;
  const src = item.source_path && /\.md$/.test(item.source_path) ? ` It comes from the note "${item.source_path}": read just that one note if you need more detail.` : "";
  const bound = `${src} Don't search the whole vault or run shell commands; ask me if something is missing.`;
  return [
    { label: "Work on this", prompt: `Help me make progress on this item from my dashboard: ${ctx}${bound} Suggest the smallest next step, and do it only if it's safe.` },
    { label: "Draft an email", prompt: `Draft a short email for this item from my dashboard: ${ctx} Ask me who it's to and the key points if you need them. Show the draft here; don't send anything.` },
    { label: "Research", prompt: `Research this for me: ${ctx} Use your vault search and web tools sparingly (a few targeted lookups), then give a short brief with sources and what I should do next.` },
  ];
}
