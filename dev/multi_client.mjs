#!/usr/bin/env node
// Two clients on one conversation. "phone" sends a message and has no file access; "desktop"
// only watches (and has file access). Verifies routing, mirroring and approval dismissal.
// Usage: multi_client.mjs <base-url> <token>
import fs from "node:fs";
const [base, token] = process.argv.slice(2);
const H = { "Content-Type": "application/json", Authorization: `Bearer ${token}` };
const SID = `multi-${Date.now()}`;
const respond = (id, value) => fetch(`${base}/api/v1/bridge/respond`, { method: "POST", headers: H, body: JSON.stringify({ id, value }) });

async function* sse(res) {
  const reader = res.body.pipeThrough(new TextDecoderStream()).getReader();
  let buf = "";
  for (;;) {
    const { value, done } = await reader.read();
    if (done) return;
    buf += value;
    let i;
    while ((i = buf.indexOf("\n\n")) >= 0) {
      const frame = buf.slice(0, i); buf = buf.slice(i + 2);
      const data = frame.split("\n").filter(l => l.startsWith("data:")).map(l => l.slice(5).trim()).join("");
      if (data) yield JSON.parse(data);
    }
  }
}

const seen = { desktop: [], phone: [] };
const ctl = new AbortController();

// Desktop: attach as a watcher first, so it sees the whole turn.
const wres = await fetch(`${base}/api/v1/sessions/events?session_id=${SID}&client_id=desktop&kind=desktop&caps=device_fs`, { headers: H, signal: ctl.signal });
const desktop = (async () => {
  try {
    for await (const ev of sse(wres)) {
      seen.desktop.push(ev.type);
      if (ev.type === "device_request") {
        let content, is_error = false;
        try { content = fs.readFileSync(ev.input.path, "utf8").trim(); } catch (e) { content = String(e); is_error = true; }
        await respond(ev.id, { content, is_error });
      }
      if (ev.type === "permission_request") await respond(ev.id, { allow: true }); // the desktop user approves
    }
  } catch {}
})();
await new Promise(r => setTimeout(r, 300)); // let the attach register

// Phone: sends the message, declares NO capabilities, never answers anything itself.
const pres = await fetch(`${base}/api/v1/chat/stream`, {
  method: "POST", headers: H,
  body: JSON.stringify({ session_id: SID, message: "multi test", client: { id: "phone", kind: "android", capabilities: [] } }),
});
let toolResult = null, mirrored = null;
for await (const ev of sse(pres)) {
  seen.phone.push(ev.type);
  if (ev.type === "tool_result" && ev.content && !toolResult) toolResult = ev.content;
  if (ev.type === "done" || ev.type === "error") { seen.phone.push(`final:${(ev.reply || ev.message || "").slice(0, 60)}`); break; }
}
await new Promise(r => setTimeout(r, 300));
ctl.abort(); await desktop;

const has = (arr, t) => arr.includes(t);
const mirroredMsg = true;
const checks = [
  ["phone (no file access) still got the device result, served by the desktop", toolResult && toolResult.length > 0 && !toolResult.startsWith("error")],
  ["desktop saw the phone's message (user_message)", has(seen.desktop, "user_message")],
  ["desktop was asked for device access", has(seen.desktop, "device_request")],
  ["phone was NEVER sent the device request", !has(seen.phone, "device_request")],
  ["both clients were shown the approval", has(seen.desktop, "permission_request") && has(seen.phone, "permission_request")],
  ["phone was told the approval was resolved elsewhere", has(seen.phone, "resolved")],
  ["desktop saw the turn finish", has(seen.desktop, "done")],
];
for (const [name, ok] of checks) console.log(`${ok ? "PASS" : "FAIL"}: ${name}`);
console.log("phone events:", seen.phone.filter(t => t !== "text_delta" && t !== "usage").join(" > "));
console.log("desktop events:", seen.desktop.filter(t => t !== "text_delta" && t !== "usage").join(" > "));
process.exit(checks.every(c => c[1]) ? 0 : 1);
