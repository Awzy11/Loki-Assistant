import { useCallback, useEffect, useRef, useState } from "react";
import { getNeedsYou, type NeedsYou } from "./transport";

/** The Needs-you + briefings data, shared by every page: loaded once, refreshed in the background. */
export function useNeeds(enabled: boolean) {
  const [data, setData] = useState<NeedsYou | null>(null);
  const [error, setError] = useState<string | null>(null);
  const polls = useRef(0);

  const refresh = useCallback(() => {
    getNeedsYou()
      .then((d) => {
        setData(d);
        setError(null);
        // the local model writes the "why now" lines in the background: look again shortly
        if (d.blurbs_pending && polls.current > 0) {
          polls.current -= 1;
          window.setTimeout(refresh, 7000);
        }
      })
      .catch((e) => setError(String(e instanceof Error ? e.message : e)));
  }, []);

  useEffect(() => {
    if (!enabled) return;
    polls.current = 4;
    refresh();
    const t = window.setInterval(() => {
      polls.current = 2;
      refresh();
    }, 5 * 60 * 1000);
    return () => window.clearInterval(t);
  }, [enabled, refresh]);

  const refreshSoon = useCallback(() => {
    polls.current = 2;
    refresh();
  }, [refresh]);

  return { data, error, refresh: refreshSoon };
}
