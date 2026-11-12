#!/usr/bin/env python3
"""Two scripted models for dev/e2e_hierarchy.sh, in one process. Stdlib only.

  local  (Ollama wire format, port A): answers the privacy gate with CLOUD, condense requests with a fixed digest, briefing-packet
         requests with a fixed packet, anything else "pong".
  cloud  (OpenAI wire format, port B): "zebra search" -> calls simple_search, then reports whether the result it got was condensed;
         "/bod" -> reports whether its first message carried a briefing packet. Every request body is appended to a log file.
Usage: fake_hierarchy_models.py LOCAL_PORT CLOUD_PORT LOGFILE
"""
import json, sys, threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

LOG = open(sys.argv[3], "a")

def text_of(m):
    c = m.get("content")
    return c if isinstance(c, str) else json.dumps(c)

class Local(BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def do_GET(self):
        b = json.dumps({"models": [{"name": "local:1b"}]}).encode()
        self.send_response(200); self.send_header("Content-Type", "application/json"); self.send_header("Content-Length", str(len(b))); self.end_headers(); self.wfile.write(b)
    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers.get("Content-Length", 0))) or b"{}")
        msgs = body.get("messages", [])
        system = " ".join(text_of(m) for m in msgs if m.get("role") == "system")
        user = " ".join(text_of(m) for m in msgs if m.get("role") == "user")
        LOG.write("LOCAL " + json.dumps({"system": system[:200], "user": user[:200]}) + "\n"); LOG.flush()
        if "privacy gate" in system:
            out = "CLOUD"
        elif "condense tool output" in system:
            out = "CONDENSED-DIGEST: zebras are striped animals."
        elif "preparing a briefing packet" in user:
            out = "PACKET-BODY: 10 - Daily/yesterday.md - three tasks, two done."
        else:
            out = "pong"
        if body.get("stream") is False:
            b = json.dumps({"model": "local:1b", "message": {"role": "assistant", "content": out}, "done": True, "done_reason": "stop"}).encode()
            self.send_response(200); self.send_header("Content-Type", "application/json"); self.send_header("Content-Length", str(len(b))); self.end_headers(); self.wfile.write(b)
            return
        self.send_response(200); self.send_header("Content-Type", "application/x-ndjson"); self.end_headers()
        self.wfile.write((json.dumps({"model": "local:1b", "message": {"role": "assistant", "content": out}, "done": False}) + "\n").encode())
        self.wfile.write((json.dumps({"model": "local:1b", "message": {"role": "assistant", "content": ""}, "done": True, "done_reason": "stop", "prompt_eval_count": 5, "eval_count": 2}) + "\n").encode())

class Cloud(BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def _send(self, obj):
        b = json.dumps(obj).encode()
        self.send_response(200); self.send_header("Content-Type", "application/json"); self.send_header("Content-Length", str(len(b))); self.end_headers(); self.wfile.write(b)
    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers.get("Content-Length", 0))) or b"{}")
        if self.path.endswith("/embeddings"):
            return self._send({"data": [{"embedding": [0.1] * 8, "index": 0}]})
        msgs = body.get("messages", [])
        users = [text_of(m) for m in msgs if m.get("role") == "user"]
        tools = [text_of(m) for m in msgs if m.get("role") == "tool"]
        has_hint = any("Cost note: you run on a paid cloud model" in u for u in users)
        has_delegate = any(t.get("function", {}).get("name") == "delegate_local" for t in body.get("tools") or [])
        LOG.write("CLOUD " + json.dumps({"hint": has_hint, "delegate_offered": has_delegate, "tool_results": [t[:80] for t in tools], "first_user": users[0][:80] if users else ""}) + "\n"); LOG.flush()
        joined = " ".join(users)
        if "zebra search" in joined:
            if not tools:
                return self._send({"choices": [{"index": 0, "finish_reason": "tool_calls", "message": {"role": "assistant", "content": "I'll search the vault for zebras.", "tool_calls": [{"id": "c1", "type": "function", "function": {"name": "simple_search", "arguments": json.dumps({"query": "zebra"})}}]}}]})
            text = f"A done: condensed={tools[-1].startswith('[Condensed by')} chars={len(tools[-1])} hint={has_hint}"
        elif "beginning-of-day" in joined:
            text = f"B done: packet={'[Briefing packet' in joined} bodyok={'PACKET-BODY' in joined}"
        else:
            text = "cloud answered"
        LOG.write("ANSWER " + text + "\n"); LOG.flush()
        self._send({"choices": [{"index": 0, "finish_reason": "stop", "message": {"role": "assistant", "content": text}}]})

for port, h in ((int(sys.argv[1]), Local), (int(sys.argv[2]), Cloud)):
    threading.Thread(target=ThreadingHTTPServer(("127.0.0.1", port), h).serve_forever, daemon=True).start()
threading.Event().wait()
