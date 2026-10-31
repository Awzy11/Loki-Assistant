import { useEffect, useState } from "react";

/** A tiny History-API router: real URLs, working back/forward, no dependency. */

const EVENT = "loki:navigate";

export function navigate(to: string, opts: { replace?: boolean } = {}) {
  if (to === window.location.pathname + window.location.search) return;
  if (opts.replace) window.history.replaceState({}, "", to);
  else window.history.pushState({}, "", to);
  window.dispatchEvent(new Event(EVENT));
  window.scrollTo?.(0, 0);
}

/** The current path (`/b/news`), re-rendering the caller on navigation. */
export function usePath(): string {
  const [path, setPath] = useState(window.location.pathname);
  useEffect(() => {
    const sync = () => setPath(window.location.pathname);
    window.addEventListener("popstate", sync);
    window.addEventListener(EVENT, sync);
    return () => {
      window.removeEventListener("popstate", sync);
      window.removeEventListener(EVENT, sync);
    };
  }, []);
  return path;
}

export type Route =
  | { name: "home" }
  | { name: "needs" }
  | { name: "needslist" }
  | { name: "brief"; slug: string }
  | { name: "day"; date: string }
  | { name: "chat"; id?: string }
  | { name: "mailreview" }
  | { name: "maildash" }
  | { name: "newsdash" }
  | { name: "leadsdash" }
  | { name: "improve" }
  | { name: "reports" }
  | { name: "lessons" }
  | { name: "settings" };

export function parseRoute(path: string): Route {
  const p = path.replace(/\/+$/, "") || "/";
  if (p === "/") return { name: "home" };
  if (p === "/needs") return { name: "needs" };
  if (p === "/needs/list") return { name: "needslist" };
  if (p === "/mail/review") return { name: "mailreview" };
  if (p === "/mail" || p === "/b/mail") return { name: "maildash" };
  if (p === "/news" || p === "/b/news") return { name: "newsdash" };
  if (p === "/leads" || p === "/b/leads") return { name: "leadsdash" };
  if (p.startsWith("/b/")) return { name: "brief", slug: p.slice(3) };
  if (p === "/day" || p.startsWith("/day/")) return { name: "day", date: p.slice(5) };
  if (p === "/chat") return { name: "chat" };
  if (p.startsWith("/chat/")) return { name: "chat", id: decodeURIComponent(p.slice(6)) };
  if (p === "/improve") return { name: "improve" };
  if (p === "/reports") return { name: "reports" };
  if (p === "/lessons") return { name: "lessons" };
  if (p === "/settings") return { name: "settings" };
  return { name: "home" };
}

/** A briefing card's kind ↔ its URL slug (`shape` reads better as `today`). */
export const slugOf = (kind: string) => (kind === "shape" ? "today" : kind);
export const kindOf = (slug: string) => (slug === "today" ? "shape" : slug);
