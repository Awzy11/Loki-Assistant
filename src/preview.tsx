// Dev-only: renders the Home pages with sample data (npm run dev → /preview.html?page=home|needs|brief&slug=rhythm).
import { createRoot } from "react-dom/client";
import "./App.css";
import BriefingPage from "./BriefingPage";
import HomePage from "./HomePage";
import NeedsPage from "./NeedsPage";
import TopNav from "./TopNav";
import ChatLanding from "./ChatLanding";
import Toast from "./Toast";
import { parseRoute } from "./router";
import type { Briefing, NeedsYou } from "./transport";

// Sample data in the shape the server sends: the profile's property list, then one entry per day.
function rhythmData() {
  const props = [
    { key: "sleep_h", label: "Sleep", type: "number", unit: "h", options: [], role: "", summary: "average", chart: "" },
    { key: "weight_lbs", label: "Weight", type: "number", unit: "lbs", options: [], role: "", summary: "trend", chart: "" },
    { key: "starting_state", label: "Starting state", type: "choice", unit: "", options: ["Fresh", "Partial", "Depleted"], role: "energy", summary: "", chart: "" },
    { key: "masking_cost", label: "Masking cost", type: "choice", unit: "", options: ["Low", "Med", "High"], role: "", summary: "", chart: "" },
    { key: "mits_done", label: "MITs done", type: "number", unit: "", options: [], role: "", summary: "average_nonzero", chart: "bars" },
  ];
  const sleep = [6.5, 6, 6.5, 7, null, null, 8, 6, 6.5, 7, 6, 5.5, null, 7];
  const weight = [218.1, 217, 213.8, 214.9, null, null, 215.2, 216, 215.8, 216.2, 215.4, 217.8, null, 214.8];
  const start = ["Fresh", "Partial", "Partial", "Partial", "", "", "Fresh", "Partial", "Partial", "Depleted", "Partial", "Partial", "", "Partial"];
  const mask = ["Low", "Med", "High", "Med", "", "", "Low", "Med", "Med", "High", "Med", "High", "", "Med"];
  const mits = [3, 3, 1, 3, 1, 1, 2, 1, 1, 4, 2, 3, 0, 0];
  const days = sleep.map((_, i) => ({ date: `2026-10-${String(13 + i).padStart(2, "0")}`, values: { sleep_h: sleep[i], weight_lbs: weight[i], starting_state: start[i] || null, masking_cost: mask[i] || null, mits_done: mits[i] } }));
  return { props, days };
}

const b = (kind: Briefing["kind"], title: string, blurb: string, pill: string, rows: [string, string, string?][], extra: Partial<Briefing> = {}): Briefing => ({
  id: `b-${kind}`, kind, title, blurb, pill, link: null, source_path: `vault/${kind}`, series: [], rows: rows.map(([head, text, link]) => ({ head, text, link: link ?? null })), ...extra,
});

const sample: NeedsYou = {
  date: "2026-10-27",
  weekday: "Tue",
  state: { starting_state: "Partial", mode: "gentle", note: "Starting state Partial — a shorter list today." },
  items: [
    { id: "a1", kind: "task", title: "ATLAS: finish the one-page ask to Col Rivera", why: "One of today's 3, still open (today's note, School)", source: "Today's note", source_path: "", score: 90, when: null, next_action: "Open the draft in 6100 Research and close claim 3", blurb: "It is the one thing today that someone else is waiting on." },
    { id: "b2", kind: "event", title: "Call with Maj Smith, Acme Software Factory", why: "In 2 days, from Upcoming Events", source: "Upcoming Events", source_path: "", score: 45, when: "2026-10-29", next_action: null, blurb: null },
    { id: "c3", kind: "sprint", title: "Loki write-logging hook deploy, test it live", why: "Active in this week's sprint", source: "Week note", source_path: "", score: 50, when: null, next_action: null, blurb: null },
    { id: "d4", kind: "task", title: "Run, afternoon, easy and short given the cold", why: "One of today's 3", source: "Today's note", source_path: "", score: 80, when: "1630–1800", next_action: null, blurb: null },
  ],
  briefings: [
    b("shape", "Today's shape", "Now: 6100 Research: ATLAS ask", "3 blocks", [["0900–1015", "School: College lecture"], ["1345–1630  ◀ now", "School: 6100 Research: ATLAS ask"], ["1930–2100", "Projects: Loki buildout"]]),
    b("week", "Week W44", "4/5 active · buffer 4.5h (45%) · 6 days left", "This week", [["Goal", "Keep a good rhythm coming out of Phase A, and take care of myself."], ["Capacity", "10h available · 5.5h committed · 4.5h (45%) buffer"]]),
    b("countdowns", "Countdowns", "City 10K, Riverton — in 4 days", "4 ahead", [["in 4 days · 2026-10-31", "City 10K, Riverton, 0800"], ["in 5 days · 2026-11-01", "ATLAS 6100A soft-send"]]),
    b("rhythm", "Rhythm", "sleep 6.6 h · weight 218.1 → 214.8 · 9 of 12 starts Partial", "14 days", [["Sleep", "6.6 h average over 12 nights"], ["Starting state", "2 Fresh · 9 Partial · 0 Depleted"]], {
      series: [
        { name: "Sleep", unit: "h", points: [6.5, 6, 6.5, 7, null, null, 8, 6, 6.5, 7, 6, 5.5, null, 7] },
        { name: "Weight", unit: "lbs", points: [218.1, 217, 213.8, 214.9, null, null, 215.2, 216, 215.8, 216.2, 215.4, 217.8, null, 214.8] },
        { name: "Waist", unit: "in", points: [36, 36, 35.5, 36, null, null, 36, 35.5, 36, 35.75, 36, 36, null, 36] },
        { name: "MITs done", unit: "", points: [3, 3, 1, 3, 1, 1, 2, 1, 1, 4, 2, 3, 0, 0] },
      ], data: rhythmData() }),
    b("news", "News digest", "Tesla dominated the feed: it stretched its Houston Robotaxi geofence into a rocket shape.", "10 categories · 2026-10-26", [["Technology", "Tesla dominated the feed."], ["Defense Tech", "AUSA 2026 set the tone."]], { link: "https://the news page/" }),
    b("signals", "Signals to watch", "Oil and rates moved sharply — WTI above $107", "5 signals", [["Oil and rates moved sharply", "WTI rose from $95.39 to $107.89.", "https://fred.stlouisfed.org/series/DCOILWTICO"], ["Red tropical cyclone ARNAB-26", "GDACS lists a Red cyclone for India.", "https://www.gdacs.org/"]]),
    b("leads", "Leads — 3 hire-me, 7 fix-it", "AI agent for WhatsApp and Instagram connected to an Altegio CRM", "Today's scan", [["AI agent for WhatsApp and Instagram connected to an Altegio CRM", "Effort: L · Budget: unstated", "https://community.n8n.io/t/x"], ["Pipeline", "5 interested · 1 replied · 316 new"]], { link: "https://the leads app/" }),
    b("backlog", "Backlog", "62 items · 8 queued for the sprint", "62 items", [["Tech", "49 items"], ["Home", "13 items"], ["Queue", "Hard run, 5 mi progression"]]),
    b("finance", "Finance", "60% of budget used · 3 bills in 7 days", "Ledger", [["This month", "$1,200 spent · $4,000 income"], ["Net worth", "$50,000"]], { link: "https://the ledger/" }),
    b("saved", "Recently saved", "Eric Michaud - Build an Obsidian Backend Not a Second Brain!", "2 new", [["Eric Michaud - Build an Obsidian Backend", "Technology · Oct 25"], ["Inbox", "5 items waiting, 3 older than 48 hours"]]),
    b("network", "Home network", "All quiet", "quiet", [["Server containers", "26 running"], ["Backup (restic)", "last run Mon 2026-10-26 03:41:32 — ok"], ["Tailnet devices", "5 of 6 reachable"]], { link: "https://monitor.example.org/" }),
    b("loki", "Loki", "model loaded", "status", [["Engine", "ollama:llama3.1:8b (default)"], ["Local model", "llama3.1:8b loaded, 28 GB on the GPU, 256k context"], ["Tools (MCP)", "13 of 14 servers connected, 240 tools"]]),
  ],
  hidden_by_capacity: 2,
  snoozed: 1,
  focus: { current: { start: "1345", end: "1630", type: "School", what: "6100 Research: ATLAS ask" }, next: null },
};

const q = new URLSearchParams(location.search);
const page = q.get("page") ?? "home";
const slug = q.get("slug") ?? "today";
const route = parseRoute(page === "home" ? "/" : page === "needs" ? "/needs" : `/b/${slug}`);

const chat0 = (<div className="loki-messages" style={{ padding: "1rem 0", minHeight: "40vh" }}><div className="loki-turn loki-turn-user"><div className="loki-turn-role">You</div>Catch me up on BOD.</div></div>);

const chatLanding = (
  <>
    <div className="loki-messages" style={{ padding: "1rem 0" }}>
      <ChatLanding onSuggest={() => {}} sessions={[
        { session_id: "a", title: "Loki hook deploy plan", updated: Math.floor(Date.now() / 1000) - 7200, turns: 12 },
        { session_id: "b", title: "EOD catch-up and BOW planning", updated: Math.floor(Date.now() / 1000) - 32000, turns: 8 },
        { session_id: "c", title: "Obsidian as a backend video", updated: Math.floor(Date.now() / 1000) - 190000, turns: 4 },
      ]} />
    </div>
    <div className="loki-engine-row">
      <label>Model{" "}<select defaultValue="x"><option value="x">☁ Claude sonnet (Loki's loop)</option><option>🔒 Ollama llama3.1:8b</option></select></label>
      <button type="button">Make default</button><button type="button">Back to default</button>
    </div>
  </>
);

const queuePreview = (
  <>
    {chat0}
    <div className="loki-queue">
      <span>Queued next:</span>
      <span className="loki-queue-item"><span>Also check my EOD note from yesterday</span><button type="button">✕</button></span>
      <span className="loki-queue-item"><span>Then draft the ATLAS ask email</span><button type="button">✕</button></span>
      <button type="button">Clear</button>
    </div>
    <form className="loki-input-row"><textarea rows={1} placeholder="Loki is working — type here to queue your next message…" /><button type="submit" className="loki-queue-btn" disabled>Queue</button><button type="button" className="loki-stop">■ Stop</button></form>
    <Toast toast={{ id: 1, kind: "ok", text: "Loki finished", detail: "Loki hook deploy plan", action: { label: "View", run: () => {} } }} onClose={() => {}} />
  </>
);

const chat = (
  <div className="loki-messages" style={{ padding: "1rem 0" }}>
    <div className="loki-turn loki-turn-user"><div className="loki-turn-role">You</div>Catch me up on BOD and what's open today.</div>
    <div className="loki-turn loki-turn-assistant"><div className="loki-turn-role">Loki</div>Here is where things stand for Tuesday: three MITs, one call, and a lighter list because your starting state is Partial.</div>
  </div>
);

createRoot(document.getElementById("root")!).render(
  page === "home" ? (
    <main className="loki-shell loki-shell-home"><HomePage data={sample} error={null} onAsk={() => {}} /></main>
  ) : (
    <main className="loki-shell">
      <TopNav route={route} data={sample} onTroubleshoot={() => {}} />
      {page === "needs" && <NeedsPage data={sample} error={null} refresh={() => {}} onAsk={() => {}} />}
      {page === "brief" && <BriefingPage slug={slug} data={sample} onAsk={() => {}} />}
      {page === "chat" && chat}
      {page === "landing" && chatLanding}
      {page === "queue" && queuePreview}
    </main>
  ),
);
