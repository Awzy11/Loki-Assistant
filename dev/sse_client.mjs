#!/usr/bin/env node
// Test stand-in for the desktop app: streams one chat turn, prints every event, and answers
// bridge requests. Usage: sse_client.mjs <base-url> <token> "<message>" [--allow|--deny|--always]
//   device_read_file / device_list_dir are served from the real filesystem.
import fs from "node:fs";
import os from "node:os";
const [base, token, message, mode = "--allow"] = process.argv.slice(2);
const H = { "Content-Type": "application/json", Authorization: `Bearer ${token}` };
const respond = (id, value) => fetch(`${base}/api/v1/bridge/respond`, { method: "POST", headers: H, body: JSON.stringify({ id, value }) });
const res = await fetch(`${base}/api/v1/chat/stream`, { method: "POST", headers: H, body: JSON.stringify({ session_id: process.env.SESSION || `sse-${Date.now()}`, message, client: { id: "sse-test-client", kind: "cli", capabilities: ["device_fs"] } }) });
const reader = res.body.pipeThrough(new TextDecoderStream()).getReader();
let buf = "";
for (;;) {
  const { value, done } = await reader.read();
  if (done) break;
  buf += value;
  let i;
  while ((i = buf.indexOf("\n\n")) >= 0) {
    const frame = buf.slice(0, i); buf = buf.slice(i + 2);
    const data = frame.split("\n").filter(l => l.startsWith("data:")).map(l => l.slice(5).trim()).join("");
    if (!data) continue;
    const ev = JSON.parse(data);
    if (ev.type !== "text_delta" && ev.type !== "usage") console.log(JSON.stringify(ev).slice(0, 300));
    if (ev.type === "text_delta") process.stdout.write(ev.text);
    if (ev.type === "permission_request") {
      if (mode === "--always") await fetch(`${base}/api/v1/permissions/rules`, { method: "POST", headers: H, body: JSON.stringify({ tool: ev.tool, input: ev.input }) });
      await respond(ev.id, { allow: mode !== "--deny" });
    }
    if (ev.type === "device_request") {
      let content, is_error = false;
      try {
        const p = ev.input.path?.startsWith("/") ? ev.input.path : `${os.homedir()}/${ev.input.path}`;
        if (ev.name === "device_list_dir") content = fs.readdirSync(p, { withFileTypes: true }).map(d => d.name + (d.isDirectory() ? "/" : "")).join("\n");
        else if (ev.name === "device_read_file") content = fs.readFileSync(p, "utf8").split("\n").map((l, n) => `${n + 1}\t${l}`).join("\n");
        else { content = `test client does not implement ${ev.name}`; is_error = true; }
      } catch (e) { content = String(e); is_error = true; }
      await respond(ev.id, { content, is_error });
    }
    if (ev.type === "done" || ev.type === "error") { console.log(); process.exit(0); }
  }
}
