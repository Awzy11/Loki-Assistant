import { navigate } from "./router";
import type { SessionSummary } from "./transport";

const SUGGESTIONS = ["/bod", "/eod", "What needs me today?", "Catch me up on my inbox", "Summarize this week so far"];

function ago(unix: number): string {
  const s = Math.max(0, Math.floor(Date.now() / 1000) - unix);
  if (s < 90) return "just now";
  if (s < 3600) return `${Math.round(s / 60)}m ago`;
  if (s < 86400) return `${Math.round(s / 3600)}h ago`;
  return `${Math.round(s / 86400)}d ago`;
}

/** `/chat` — a fresh conversation: what to ask, and where to pick up an earlier one. */
export default function ChatLanding({ sessions, onSuggest }: { sessions: SessionSummary[]; onSuggest: (text: string) => void }) {
  const recent = sessions.filter((s) => s.turns > 0).slice(0, 6);
  return (
    <div className="loki-landing">
      <h2>What can I do for you?</h2>
      <p>Ask about your vault, your backlog or today's plan — or start from one of these.</p>
      <div className="loki-landing-chips">
        {SUGGESTIONS.map((s) => (
          <button type="button" key={s} onClick={() => onSuggest(s.startsWith("/") ? `${s} ` : s)}>{s}</button>
        ))}
      </div>
      {recent.length > 0 && (
        <>
          <h3>Pick up where you left off</h3>
          <div className="loki-landing-recent">
            {recent.map((s) => (
              <button type="button" key={s.session_id} onClick={() => navigate(`/chat/${encodeURIComponent(s.session_id)}`)} title={s.title}>
                <span>{s.title}</span>
                <small>{ago(s.updated)}</small>
              </button>
            ))}
          </div>
        </>
      )}
    </div>
  );
}
