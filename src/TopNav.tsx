import NavStrip from "./NavStrip";
import { navigate, type Route } from "./router";
import type { NeedsYou } from "./transport";

export { BRIEF_NAMES } from "./NavStrip";

/** Along the top of every page except Home: the logo (→ Home) and app pages, then the week + pages strip. */
export default function TopNav({ route, data, onTroubleshoot }: { route: Route; data: NeedsYou | null; onTroubleshoot: () => void }) {
  const cls = (on: boolean) => (on ? "on" : "");
  return (
    <header className="loki-topnav">
      <div className="loki-topnav-row">
        <button type="button" className="loki-topnav-brand" onClick={() => navigate("/")} title="Home" aria-label="Home">
          <img src="/loki.svg?v=2" alt="" width={28} height={28} />
          <span>Loki</span>
        </button>
        <nav className="loki-topnav-links" aria-label="App">
          <button type="button" className={cls(route.name === "chat")} onClick={() => navigate("/chat")}>Chat</button>
          <button type="button" className={cls(route.name === "improve" || route.name === "reports" || route.name === "lessons")} onClick={() => navigate("/improve")}>Improve</button>
          <button type="button" className={cls(route.name === "settings")} onClick={() => navigate("/settings")}>Settings</button>
          <button type="button" onClick={onTroubleshoot} title="Have Claude look at this chat, its logs and this screen">Troubleshoot</button>
        </nav>
      </div>
      <NavStrip data={data} route={route} />
    </header>
  );
}
