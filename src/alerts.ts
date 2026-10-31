/** "A turn finished while I'm not looking at the chat" alerts, for while the app is open. */
export type AlertPrefs = { toast: boolean; sound: boolean; desktop: boolean };

const KEY = "loki_alerts";
const DEFAULTS: AlertPrefs = { toast: true, sound: false, desktop: true };

export function getAlertPrefs(): AlertPrefs {
  try {
    return { ...DEFAULTS, ...(JSON.parse(localStorage.getItem(KEY) ?? "{}") as Partial<AlertPrefs>) };
  } catch {
    return DEFAULTS;
  }
}

export function setAlertPrefs(p: AlertPrefs) {
  try {
    localStorage.setItem(KEY, JSON.stringify(p));
  } catch {
    // best effort
  }
}

/** A short two-note chime from the Web Audio API (no sound file needed). */
export function playChime() {
  try {
    const Ctx = window.AudioContext ?? (window as unknown as { webkitAudioContext: typeof AudioContext }).webkitAudioContext;
    const ctx = new Ctx();
    [660, 880].forEach((f, i) => {
      const o = ctx.createOscillator();
      const g = ctx.createGain();
      o.frequency.value = f;
      g.gain.setValueAtTime(0.0001, ctx.currentTime + i * 0.16);
      g.gain.exponentialRampToValueAtTime(0.12, ctx.currentTime + i * 0.16 + 0.02);
      g.gain.exponentialRampToValueAtTime(0.0001, ctx.currentTime + i * 0.16 + 0.3);
      o.connect(g).connect(ctx.destination);
      o.start(ctx.currentTime + i * 0.16);
      o.stop(ctx.currentTime + i * 0.16 + 0.32);
    });
  } catch {
    // audio can be blocked until the user has interacted; harmless
  }
}

/** A system notification, only if the page isn't visible and permission was granted. */
export function desktopNotify(title: string, body: string, onClick: () => void) {
  try {
    if (typeof Notification === "undefined" || Notification.permission !== "granted") return;
    const n = new Notification(title, { body, icon: "/icon-192.png?v=2", tag: "loki-turn" });
    n.onclick = () => {
      window.focus();
      onClick();
      n.close();
    };
  } catch {
    // not supported in this webview
  }
}

export async function askDesktopPermission(): Promise<boolean> {
  try {
    if (typeof Notification === "undefined") return false;
    if (Notification.permission === "granted") return true;
    return (await Notification.requestPermission()) === "granted";
  } catch {
    return false;
  }
}
