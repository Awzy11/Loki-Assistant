import { useEffect, useState } from "react";
import { askDesktopPermission, getAlertPrefs, playChime, setAlertPrefs } from "./alerts";
import { getPushStatus, removePushSubscription, sendPushSubscription, sendTestPush } from "./transport";

function keyToBytes(b64url: string): Uint8Array {
  const pad = "=".repeat((4 - (b64url.length % 4)) % 4);
  const raw = atob((b64url + pad).replace(/-/g, "+").replace(/_/g, "/"));
  return Uint8Array.from(raw, (c) => c.charCodeAt(0));
}

/** Phone notifications for this device: Loki sends one when a reply finishes while nobody is
 * watching the conversation (you left the app). Needs the installed PWA or a browser with push. */
export default function NotificationSettings() {
  const supported = typeof window !== "undefined" && "serviceWorker" in navigator && "PushManager" in window && "Notification" in window;
  const [subscribed, setSubscribed] = useState(false);
  const [serverReady, setServerReady] = useState<boolean | null>(null);
  const [devices, setDevices] = useState(0);
  const [msg, setMsg] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function refresh() {
    try {
      const st = await getPushStatus();
      setServerReady(st.configured);
      setDevices(st.devices);
    } catch {
      setServerReady(null);
    }
    if (supported) {
      const reg = await navigator.serviceWorker.getRegistration();
      setSubscribed(!!(await reg?.pushManager.getSubscription()));
    }
  }
  useEffect(() => void refresh(), []); // eslint-disable-line react-hooks/exhaustive-deps

  async function enable() {
    setBusy(true);
    setMsg(null);
    try {
      const st = await getPushStatus();
      if (!st.configured || !st.key) throw new Error("Push isn't set up on the server yet.");
      const perm = await Notification.requestPermission();
      if (perm !== "granted") throw new Error("Notifications are blocked — allow them for Loki in your phone's settings, then try again.");
      const reg = await navigator.serviceWorker.ready;
      const sub = (await reg.pushManager.getSubscription()) ?? (await reg.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: keyToBytes(st.key) as BufferSource }));
      await sendPushSubscription(sub.toJSON());
      setMsg("Notifications are on for this device.");
    } catch (e) {
      setMsg(String(e instanceof Error ? e.message : e));
    } finally {
      setBusy(false);
      refresh();
    }
  }

  async function disable() {
    setBusy(true);
    try {
      const reg = await navigator.serviceWorker.getRegistration();
      const sub = await reg?.pushManager.getSubscription();
      if (sub) {
        await removePushSubscription(sub.endpoint);
        await sub.unsubscribe();
      }
      setMsg("Notifications are off for this device.");
    } finally {
      setBusy(false);
      refresh();
    }
  }

  return (
    <section className="loki-settings-section">
      <h2>Notifications</h2>
      {!supported && <p className="loki-lessons-hint">This browser can't receive push notifications. On Android, install Loki as an app (Chrome → Install app) and open it from there.</p>}
      {supported && serverReady === false && <p className="loki-lessons-hint">Push isn't set up on the server yet — run <code>ops/loki-push-setup.sh</code> on the server.</p>}
      {supported && serverReady !== false && (
        <>
          <p className="loki-lessons-hint">
            Get a notification on this device when a reply finishes while you're away from the app. Replies that stayed local for privacy never put their text in the notification. {devices} device{devices === 1 ? "" : "s"} subscribed.
          </p>
          <div className="loki-fix-actions">
            {!subscribed ? (
              <button type="button" className="loki-primary" onClick={enable} disabled={busy}>
                Turn on notifications
              </button>
            ) : (
              <>
                <button type="button" onClick={() => sendTestPush().then(() => setMsg("Test sent — it should arrive in a few seconds.")).catch((e) => setMsg(String(e)))}>
                  Send a test
                </button>
                <button type="button" onClick={disable} disabled={busy}>
                  Turn off
                </button>
              </>
            )}
          </div>
        </>
      )}
      {msg && <p className="loki-lessons-hint">{msg}</p>}
    </section>
  );
}

/** What happens when a reply finishes while the app is open but you're somewhere else in it (or in another window). */
export function InAppAlertSettings() {
  const [prefs, setPrefs] = useState(getAlertPrefs());
  const [note, setNote] = useState<string | null>(null);
  function change(patch: Partial<typeof prefs>) {
    const next = { ...prefs, ...patch };
    setPrefs(next);
    setAlertPrefs(next);
  }
  return (
    <div className="loki-settings-section">
      <h2>Alerts while the app is open</h2>
      <p className="loki-settings-note">
        When a reply finishes and you are on another page, tab or window, Loki can tell you. (When the app is closed, phone notifications above take over.)
      </p>
      <div className="loki-rule-row">
        <label><input type="checkbox" checked={prefs.toast} onChange={(e) => change({ toast: e.target.checked })} /> Show a "Loki finished" card with a View button</label>
      </div>
      <div className="loki-rule-row">
        <label><input type="checkbox" checked={prefs.sound} onChange={(e) => change({ sound: e.target.checked })} /> Play a short chime</label>
        <button type="button" onClick={playChime}>Test</button>
      </div>
      <div className="loki-rule-row">
        <label>
          <input
            type="checkbox"
            checked={prefs.desktop}
            onChange={async (e) => {
              if (e.target.checked && !(await askDesktopPermission())) {
                setNote("The browser didn't allow system notifications; the card and chime still work.");
              } else {
                setNote(null);
              }
              change({ desktop: e.target.checked });
            }}
          />{" "}
          Also show a system notification when this window isn't in front
        </label>
      </div>
      {note && <div className="loki-settings-status">{note}</div>}
    </div>
  );
}
