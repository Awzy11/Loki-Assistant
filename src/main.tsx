import React from "react";
import ReactDOM from "react-dom/client";
import { useEffect, useState } from "react";
import App from "./App";
import Onboarding from "./Onboarding";

/** A server with no config yet answers /api/setup/state and gets the first-run setup; a normal server answers 404 and gets the app. */
function Root() {
  const [setup, setSetup] = useState<"checking" | "setup" | "app">("checking");
  const [state, setState] = useState<any>(null);
  useEffect(() => {
    fetch("/api/setup/state")
      .then(async (r) => { if (r.ok) { setState(await r.json()); setSetup("setup"); } else setSetup("app"); })
      .catch(() => setSetup("app"));
  }, []);
  if (setup === "checking") return null;
  return setup === "setup" && state ? <Onboarding state={state} /> : <App />;
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <Root />
  </React.StrictMode>,
);

// Installable app: register the (app-shell-only) service worker where the browser allows it
// (HTTPS or localhost). Failure is harmless — the app works the same without it.
if ("serviceWorker" in navigator) {
  window.addEventListener("load", () => {
    navigator.serviceWorker.register("/sw.js").catch(() => {});
  });
}
