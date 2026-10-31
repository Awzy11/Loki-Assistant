import { useState } from "react";
import { login } from "./transport";

/** Shown instead of the chat/settings shell whenever no API token is stored yet — a friendlier
 * front door than SSHing into whichever machine runs loki-server to copy ~/.config/loki/api_token
 * by hand. See transport.ts's `login` doc comment: this exchanges username/password for that
 * same shared token, once per device, rather than replacing it with a session concept. */
export default function Login({ onSuccess }: { onSuccess: () => void }) {
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    if (!password.trim() || busy) return;
    setBusy(true);
    setError(null);
    try {
      await login(username.trim(), password);
      onSuccess();
    } catch (e) {
      setError(String(e instanceof Error ? e.message : e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="loki-login">
      <form className="loki-login-form" onSubmit={submit}>
        <h2>Log in to Loki</h2>
        <label>
          Username
          <input value={username} onChange={(e) => setUsername(e.currentTarget.value)} autoFocus autoComplete="username" />
        </label>
        <label>
          Password
          <input type="password" value={password} onChange={(e) => setPassword(e.currentTarget.value)} />
        </label>
        {error && <div className="loki-error">{error}</div>}
        <button type="submit" disabled={busy || !password.trim() || !username.trim()}>
          {busy ? "Logging in…" : "Log in"}
        </button>
        <p className="loki-login-hint">First time? The username and password were printed once in the terminal where you started <code>loki-server</code>.</p>
      </form>
    </div>
  );
}
