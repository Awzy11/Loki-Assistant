import { useEffect, useState } from "react";
import { login } from "./transport";

type Probe = { ok: boolean; message?: string; models?: string[]; has_embedding_model?: boolean } | null;
type SetupState = { packs: { id: string; label: string; routines: number }[]; defaults: { ollama_host: string; ollama_port: number; obsidian_host: string; obsidian_port: number } };

const STEPS = ["Welcome", "Models", "Notes", "Layout", "About you", "Login", "Finish"] as const;

async function post(path: string, body: unknown): Promise<{ ok: boolean; status: number; data: any }> {
  const res = await fetch(path, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) });
  let data: any = null;
  try { data = await res.json(); } catch { /* not JSON */ }
  return { ok: res.ok, status: res.status, data };
}

function Field({ label, hint, children }: { label: string; hint?: string; children: React.ReactNode }) {
  return (
    <label className="ob-field">
      <span className="ob-label">{label}</span>
      {children}
      {hint && <span className="ob-hint">{hint}</span>}
    </label>
  );
}

function ProbeNote({ p, extra }: { p: Probe; extra?: React.ReactNode }) {
  if (!p) return null;
  return <div className={p.ok ? "ob-ok" : "ob-warn"} role="status">{p.ok ? "✓ Connected." : p.message ?? "That didn't work."}{p.ok && extra}</div>;
}

export default function Onboarding({ state }: { state: SetupState }) {
  const [step, setStep] = useState(0);
  const d = state.defaults;

  // models
  const [useLocal, setUseLocal] = useState(true);
  const [lHost, setLHost] = useState(d.ollama_host);
  const [lPort, setLPort] = useState(String(d.ollama_port));
  const [lModel, setLModel] = useState("");
  const [lProbe, setLProbe] = useState<Probe>(null);
  const [provider, setProvider] = useState<"none" | "anthropic" | "openai">("none");
  const [cBase, setCBase] = useState("https://api.openai.com/v1");
  const [cKey, setCKey] = useState("");
  const [cModel, setCModel] = useState("");
  const [cProbe, setCProbe] = useState<Probe>(null);
  const [routing, setRouting] = useState<"local" | "cloud">("local");
  const [acceptCloudOnly, setAcceptCloudOnly] = useState(false);

  // notes
  const [oHost, setOHost] = useState(d.obsidian_host);
  const [oPort, setOPort] = useState(String(d.obsidian_port));
  const [oKey, setOKey] = useState("");
  const [oProbe, setOProbe] = useState<Probe>(null);

  // layout, person, account
  const [pack, setPack] = useState<string>("none");
  const [name, setName] = useState("");
  const [about, setAbout] = useState("");
  const [place, setPlace] = useState("");
  const [lat, setLat] = useState("");
  const [lon, setLon] = useState("");
  const [user, setUser] = useState("");
  const [pw, setPw] = useState("");
  const [pw2, setPw2] = useState("");

  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [finishing, setFinishing] = useState(false);

  const hasCloud = provider !== "none" && cKey.trim() !== "" && cModel.trim() !== "";
  const providerName = provider === "anthropic" ? "Anthropic" : "your cloud provider";
  const cloudOnly = !useLocal && hasCloud;

  // pick a sensible cloud model name when the provider changes
  useEffect(() => {
    if (provider === "anthropic" && !cModel) setCModel("claude-sonnet-4-5");
    if (provider === "openai" && !cModel) setCModel("gpt-4o");
  }, [provider]); // eslint-disable-line react-hooks/exhaustive-deps

  async function testLocal() {
    setBusy("local"); setLProbe(null);
    const r = await post("/api/setup/probe", { kind: "ollama", host: lHost.trim(), port: Number(lPort) });
    setLProbe(r.data);
    const models: string[] = r.data?.models ?? [];
    if (r.data?.ok && !lModel) {
      const chat = models.find((m) => !m.toLowerCase().includes("embed"));
      if (chat) setLModel(chat);
    }
    setBusy(null);
  }
  async function testCloud() {
    setBusy("cloud"); setCProbe(null);
    const r = await post("/api/setup/probe", { kind: "cloud", provider, base_url: cBase.trim(), api_key: cKey.trim() });
    setCProbe(r.data);
    setBusy(null);
  }
  async function testNotes() {
    setBusy("notes"); setOProbe(null);
    const r = await post("/api/setup/probe", { kind: "obsidian", host: oHost.trim(), port: Number(oPort), api_key: oKey.trim() });
    setOProbe(r.data);
    setBusy(null);
  }

  const modelsValid = (useLocal ? lModel.trim() !== "" && Number(lPort) > 0 : true) && (provider === "none" || hasCloud) && (useLocal || hasCloud) && (!cloudOnly || acceptCloudOnly);
  const notesValid = oKey.trim() !== "" && Number(oPort) > 0 && oHost.trim() !== "";
  const loc = place.trim() !== "" && lat.trim() !== "" && lon.trim() !== "";
  const youValid = (lat.trim() === "" && lon.trim() === "") || (place.trim() !== "" && !Number.isNaN(Number(lat)) && !Number.isNaN(Number(lon)) && lat.trim() !== "" && lon.trim() !== "");
  const accountValid = /^[A-Za-z0-9._-]{1,32}$/.test(user) && pw.length >= 8 && pw === pw2;
  const valid = [true, modelsValid, notesValid, true, youValid, accountValid, true][step];

  function body() {
    return {
      local: useLocal ? { host: lHost.trim(), port: Number(lPort), model: lModel.trim() } : null,
      cloud: hasCloud ? { provider, base_url: cBase.trim(), api_key: cKey.trim(), model: cModel.trim() } : null,
      routing: useLocal && hasCloud ? routing : "",
      accept_cloud_only: acceptCloudOnly,
      obsidian: { host: oHost.trim(), port: Number(oPort), api_key: oKey.trim() },
      pack,
      person: { name: name.trim(), about: about.trim() },
      location: { name: place.trim(), latitude: loc ? Number(lat) : null, longitude: loc ? Number(lon) : null },
      account: { username: user.trim(), password: pw },
    };
  }

  async function finish() {
    setError(null); setBusy("finish");
    const r = await post("/api/setup/commit", body());
    if (!r.ok) { setError(r.data?.message ?? `Setup failed (HTTP ${r.status}).`); setBusy(null); return; }
    setFinishing(true);
    // the server restarts into normal mode; wait for that, then sign in with the login just chosen
    const started = Date.now();
    for (;;) {
      await new Promise((res) => setTimeout(res, 1200));
      try {
        const s = await fetch("/api/setup/state");
        if (s.status === 404) break; // normal mode: the setup route is gone
      } catch { /* restarting */ }
      if (Date.now() - started > 90_000) { setError("The server is taking a long time to restart. Reload this page in a moment."); setBusy(null); return; }
    }
    try { await login(user.trim(), pw); } catch { /* the login screen will appear instead */ }
    window.location.href = "/";
  }

  if (finishing) {
    return (
      <div className="ob-wrap"><div className="ob-card ob-center">
        <h1>Setting things up…</h1>
        <p className="ob-sub">Saving your settings and restarting Loki. This takes a few seconds.</p>
        {error && <div className="ob-warn" role="alert">{error}</div>}
      </div></div>
    );
  }

  return (
    <div className="ob-wrap">
      <div className="ob-card">
        <ol className="ob-steps" aria-label="Setup progress">
          {STEPS.map((s, i) => <li key={s} className={i === step ? "now" : i < step ? "done" : ""}>{s}</li>)}
        </ol>

        {step === 0 && (
          <section>
            <h1>Welcome to Loki</h1>
            <p className="ob-sub">A personal AI agent that runs on your own machines and works with your Obsidian notes.</p>
            <ul className="ob-bullets">
              <li><b>Private by design.</b> No accounts, no telemetry, nothing hosted for you. Loki only talks to the services you choose here.</li>
              <li><b>Your models.</b> A model on your own computer, a cloud provider, or both. With both, anything sensitive can stay local.</li>
              <li><b>About five minutes.</b> You'll need Ollama and/or a cloud API key, and Obsidian with its "Local REST API" plugin turned on.</li>
            </ul>
          </section>
        )}

        {step === 1 && (
          <section>
            <h1>Which models should Loki use?</h1>
            <p className="ob-sub">Use a local model, a cloud provider, or both. You need at least one.</p>

            <div className="ob-group">
              <label className="ob-check"><input type="checkbox" checked={useLocal} onChange={(e) => setUseLocal(e.target.checked)} /> <b>A local model</b> (through <a href="https://ollama.com" target="_blank" rel="noreferrer noopener">Ollama</a>). Free, private, nothing leaves your machine.</label>
              {useLocal && (
                <div className="ob-grid ob-grid2">
                  <Field label="Ollama address"><input value={lHost} onChange={(e) => { setLHost(e.target.value); setLProbe(null); }} /></Field>
                  <Field label="Port"><input value={lPort} onChange={(e) => { setLPort(e.target.value); setLProbe(null); }} inputMode="numeric" /></Field>
                  <Field label="Chat model" hint={lProbe?.ok ? undefined : "Press Test to list what's installed, or type a name."}>
                    {lProbe?.ok && (lProbe.models?.length ?? 0) > 0
                      ? <select value={lModel} onChange={(e) => setLModel(e.target.value)}>{!lModel && <option value="">Choose…</option>}{lProbe.models!.filter((m) => !m.toLowerCase().includes("embed")).map((m) => <option key={m}>{m}</option>)}</select>
                      : <input value={lModel} onChange={(e) => setLModel(e.target.value)} placeholder="llama3.1:8b" />}
                  </Field>
                  <div className="ob-row"><button type="button" className="ob-btn2" onClick={testLocal} disabled={busy === "local"}>{busy === "local" ? "Testing…" : "Test"}</button></div>
                  <ProbeNote p={lProbe} extra={lProbe?.has_embedding_model === false ? <div className="ob-hint">For meaning-based search of your notes, also run <code>ollama pull nomic-embed-text</code>.</div> : null} />
                </div>
              )}
            </div>

            <div className="ob-group">
              <b>A cloud provider</b> <span className="ob-hint">(optional if you chose a local model)</span>
              <div className="ob-radios">
                {([["none", "None"], ["anthropic", "Anthropic (Claude)"], ["openai", "OpenAI or compatible"]] as const).map(([v, l]) => (
                  <label key={v} className="ob-check"><input type="radio" name="prov" checked={provider === v} onChange={() => { setProvider(v); setCProbe(null); setCModel(""); }} /> {l}</label>
                ))}
              </div>
              {provider !== "none" && (
                <div className="ob-grid">
                  {provider === "openai" && <Field label="Base URL"><input value={cBase} onChange={(e) => { setCBase(e.target.value); setCProbe(null); }} /></Field>}
                  <Field label="API key" hint="Stored only on this machine, in a file only you can read."><input type="password" value={cKey} onChange={(e) => { setCKey(e.target.value); setCProbe(null); }} autoComplete="off" /></Field>
                  <Field label="Model"><input value={cModel} onChange={(e) => setCModel(e.target.value)} /></Field>
                  <div className="ob-row"><button type="button" className="ob-btn2" onClick={testCloud} disabled={busy === "cloud" || !cKey.trim()}>{busy === "cloud" ? "Testing…" : "Test key"}</button></div>
                  <ProbeNote p={cProbe} />
                </div>
              )}
            </div>

            {useLocal && hasCloud && (
              <div className="ob-group">
                <b>You gave both. Which goes first?</b>
                <label className="ob-check"><input type="radio" name="route" checked={routing === "local"} onChange={() => setRouting("local")} /> <span><b>Local first.</b> Your local model answers; the cloud provider is the backup if it fails. Most private.</span></label>
                <label className="ob-check"><input type="radio" name="route" checked={routing === "cloud"} onChange={() => setRouting("cloud")} /> <span><b>Cloud first.</b> The cloud provider answers normal requests; anything sensitive (mail, money, passwords) stays on the local model.</span></label>
              </div>
            )}

            {cloudOnly && (
              <div className="ob-notice" role="alert">
                <b>Before you continue:</b> With no local model, everything you ask Loki, including anything it reads from your notes, mail or finances to answer you, will be sent to {providerName}. Loki's privacy routing (keeping sensitive requests on your machine) can't work without a local model.
                <label className="ob-check"><input type="checkbox" checked={acceptCloudOnly} onChange={(e) => setAcceptCloudOnly(e.target.checked)} /> I'm OK with sending everything to {providerName}.</label>
              </div>
            )}
          </section>
        )}

        {step === 2 && (
          <section>
            <h1>Where are your notes?</h1>
            <p className="ob-sub">Loki reads and writes your Obsidian vault through the <b>Local REST API</b> community plugin. In Obsidian: Settings → Community plugins → search "Local REST API" → install and enable. Then copy the API key from its settings page.</p>
            <div className="ob-grid ob-grid2">
              <Field label="Address"><input value={oHost} onChange={(e) => { setOHost(e.target.value); setOProbe(null); }} /></Field>
              <Field label="Port" hint="The plugin's plain-HTTP port (27123 by default)."><input value={oPort} onChange={(e) => { setOPort(e.target.value); setOProbe(null); }} inputMode="numeric" /></Field>
              <Field label="API key"><input type="password" value={oKey} onChange={(e) => { setOKey(e.target.value); setOProbe(null); }} autoComplete="off" /></Field>
              <div className="ob-row"><button type="button" className="ob-btn2" onClick={testNotes} disabled={busy === "notes" || !oKey.trim()}>{busy === "notes" ? "Testing…" : "Test"}</button></div>
              <ProbeNote p={oProbe} />
            </div>
          </section>
        )}

        {step === 3 && (
          <section>
            <h1>How is your vault organised?</h1>
            <p className="ob-sub">You can change this later by editing <code>profile.toml</code>.</p>
            <label className={`ob-choice ${pack === "none" ? "sel" : ""}`}><input type="radio" name="pack" checked={pack === "none"} onChange={() => setPack("none")} />
              <span><b>Keep it blank</b> <em>(recommended if you already have a system)</em><br />Loki uses plain folder names (Inbox, Daily, Notes…) and adds no routines.</span></label>
            {state.packs.map((p) => (
              <label key={p.id} className={`ob-choice ${pack === p.id ? "sel" : ""}`}><input type="radio" name="pack" checked={pack === p.id} onChange={() => setPack(p.id)} />
                <span><b>{p.label}</b><br />Numbered folders (inbox → backlog → weekly sprint → daily note, with permanent notes and an archive) and {p.routines} routines you can edit: start and close the day, plan and close the week, monthly and quarterly reviews, journaling, triage.</span></label>
            ))}
          </section>
        )}

        {step === 4 && (
          <section>
            <h1>A little about you</h1>
            <p className="ob-sub">Optional. It helps Loki sort mail, find dates in text, draft replies in your voice and show your weather. It stays in <code>profile.toml</code> on this machine.</p>
            <div className="ob-grid">
              <Field label="Your first name"><input value={name} onChange={(e) => setName(e.target.value)} placeholder="Sam" /></Field>
              <Field label="One line about you" hint="For example: a nurse with two kids and a marathon in spring."><input value={about} onChange={(e) => setAbout(e.target.value)} /></Field>
              <Field label="Where you are"><input value={place} onChange={(e) => setPlace(e.target.value)} placeholder="Springfield" /></Field>
              <Field label="Latitude" hint="Only for the weather card. Look up your town's coordinates; leave blank to skip."><input value={lat} onChange={(e) => setLat(e.target.value)} inputMode="decimal" placeholder="39.80" /></Field>
              <Field label="Longitude"><input value={lon} onChange={(e) => setLon(e.target.value)} inputMode="decimal" placeholder="-89.64" /></Field>
            </div>
            {!youValid && <div className="ob-warn">A location needs a name and both coordinates (or leave all three blank).</div>}
          </section>
        )}

        {step === 5 && (
          <section>
            <h1>Choose your login</h1>
            <p className="ob-sub">You'll use this to sign in on each device (this computer, your phone). Pick something only you know.</p>
            <div className="ob-grid">
              <Field label="Username" hint="Letters, numbers, dots, dashes and underscores."><input value={user} onChange={(e) => setUser(e.target.value)} autoComplete="username" /></Field>
              <Field label="Password" hint="At least 8 characters."><input type="password" value={pw} onChange={(e) => setPw(e.target.value)} autoComplete="new-password" /></Field>
              <Field label="Password again"><input type="password" value={pw2} onChange={(e) => setPw2(e.target.value)} autoComplete="new-password" /></Field>
            </div>
            {pw2 && pw !== pw2 && <div className="ob-warn">The two passwords don't match.</div>}
          </section>
        )}

        {step === 6 && (
          <section>
            <h1>Ready to finish</h1>
            <ul className="ob-summary">
              <li><b>Models:</b> {useLocal && hasCloud ? `local (${lModel}) and ${providerName} (${cModel}), ${routing === "local" ? "local first" : "cloud first, sensitive requests stay local"}` : useLocal ? `local only (${lModel}); nothing leaves this machine` : `${providerName} (${cModel}) for everything`}</li>
              <li><b>Notes:</b> Obsidian at {oHost}:{oPort}</li>
              <li><b>Layout:</b> {pack === "none" ? "blank vault, plain folder names" : state.packs.find((p) => p.id === pack)?.label}</li>
              <li><b>You:</b> {name || "(not set)"}{place ? `, ${place}` : ""}</li>
              <li><b>Login:</b> {user}</li>
            </ul>
            {cloudOnly && <div className="ob-notice">Everything you ask will go to {providerName}.</div>}
            {error && <div className="ob-warn" role="alert">{error}</div>}
          </section>
        )}

        <div className="ob-nav">
          {step > 0 ? <button type="button" className="ob-btn2" onClick={() => { setError(null); setStep(step - 1); }} disabled={busy === "finish"}>Back</button> : <span />}
          {step < STEPS.length - 1
            ? <button type="button" className="ob-btn" onClick={() => setStep(step + 1)} disabled={!valid}>{step === 0 ? "Get started" : "Next"}</button>
            : <button type="button" className="ob-btn" onClick={finish} disabled={busy === "finish"}>{busy === "finish" ? "Saving…" : "Finish setup"}</button>}
        </div>
      </div>
    </div>
  );
}
