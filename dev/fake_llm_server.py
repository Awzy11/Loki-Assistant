#!/usr/bin/env python3
"""OpenAI-compatible stand-in for failover tests. Stdlib only.

- Classifier requests (system prompt mentions "privacy gate") -> "CLOUD".
- model == "bad" and the request carries `tools` -> HTTP 429 (simulates quota exhaustion).
- Everything else -> "<model> answered".
- /v1/embeddings -> a fixed 8-dim vector.
Usage: fake_llm_server.py [port]
"""
import json, sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

class H(BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def _send(self, code, obj):
        b = json.dumps(obj).encode()
        self.send_response(code); self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(b))); self.end_headers(); self.wfile.write(b)
    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers.get("Content-Length", 0))) or b"{}")
        if self.path.endswith("/embeddings"):
            return self._send(200, {"data": [{"embedding": [0.1] * 8, "index": 0}]})
        msgs = body.get("messages", [])
        if any("privacy gate" in (m.get("content") or "") for m in msgs if m.get("role") == "system"):
            text = "CLOUD"
        elif body.get("model") == "bad" and body.get("tools"):
            return self._send(429, {"error": {"message": "quota exceeded"}})
        elif body.get("model") == "good" and body.get("tools") and any(
            m.get("role") == "user" and "delete note" in (m.get("content") or "") for m in msgs
        ) and not any(m.get("role") == "tool" for m in msgs):
            # Ask Loki to call the Tier-2 delete_file tool, to exercise the approval gate.
            return self._send(200, {"choices": [{"index": 0, "finish_reason": "tool_calls", "message": {
                "role": "assistant", "content": "I'll delete that note.", "tool_calls": [{
                    "id": "call_1", "type": "function",
                    "function": {"name": "delete_file", "arguments": json.dumps({"path": "50 - Notes/victim.md"})}}]}}]})
        elif body.get("model") == "good" and body.get("tools") and any(
            m.get("role") == "user" and "mcp test" in (m.get("content") or "") for m in msgs
        ):
            # Scripted MCP flow: search_tools -> call_tool(echo) -> call_tool(get_time) -> answer.
            n = sum(1 for m in msgs if m.get("role") == "tool")
            def call(name, args):
                return self._send(200, {"choices": [{"index": 0, "finish_reason": "tool_calls", "message": {
                    "role": "assistant", "content": "", "tool_calls": [{"id": f"c{n}", "type": "function",
                    "function": {"name": name, "arguments": json.dumps(args)}}]}}]})
            if n == 0: return call("search_tools", {"query": "echo"})
            if n == 1: return call("call_tool", {"name": "mcp__fake__echo", "arguments": {"text": "hi"}})
            if n == 2: return call("call_tool", {"name": "mcp__fake__get_time", "arguments": {}})
            text = "mcp done: " + " | ".join(m.get("content", "") for m in msgs if m.get("role") == "tool")
        elif body.get("model") == "good" and body.get("tools") and any(
            m.get("role") == "user" and "multi test" in (m.get("content") or "") for m in msgs
        ):
            # device_read_file (served by a capable client) -> echo (needs approval) -> answer.
            n = sum(1 for m in msgs if m.get("role") == "tool")
            def call2(name, args):
                return self._send(200, {"choices": [{"index": 0, "finish_reason": "tool_calls", "message": {
                    "role": "assistant", "content": "", "tool_calls": [{"id": f"m{n}", "type": "function",
                    "function": {"name": name, "arguments": json.dumps(args)}}]}}]})
            if n == 0: return call2("device_read_file", {"path": "/etc/hostname"})
            if n == 1: return call2("call_tool", {"name": "mcp__fake__echo", "arguments": {"text": "multi"}})
            text = "multi done: " + " | ".join(m.get("content", "")[:40] for m in msgs if m.get("role") == "tool")
        elif body.get("model") == "good" and body.get("tools") and any(
            m.get("role") == "user" and "remote test" in (m.get("content") or "") for m in msgs
        ):
            n = sum(1 for m in msgs if m.get("role") == "tool")
            if n == 0:
                return self._send(200, {"choices": [{"index": 0, "finish_reason": "tool_calls", "message": {
                    "role": "assistant", "content": "", "tool_calls": [{"id": "r0", "type": "function", "function": {
                        "name": "remote_run_command", "arguments": json.dumps({"host": "austinlaptop", "command": "hyprctl version"})}}]}}]})
            if n == 1:
                return self._send(200, {"choices": [{"index": 0, "finish_reason": "tool_calls", "message": {
                    "role": "assistant", "content": "", "tool_calls": [{"id": "r1", "type": "function", "function": {
                        "name": "remote_run_command", "arguments": json.dumps({"host": "evilhost", "command": "id"})}}]}}]})
            text = "remote done: " + " | ".join(m.get("content", "")[:70] for m in msgs if m.get("role") == "tool")
        elif any(m.get("role") == "tool" for m in msgs):
            text = "tool said: " + [m for m in msgs if m.get("role") == "tool"][-1].get("content", "")
        else:
            text = f"{body.get('model')} answered"
        self._send(200, {"choices": [{"index": 0, "finish_reason": "stop", "message": {"role": "assistant", "content": text}}]})

ThreadingHTTPServer(("127.0.0.1", int(sys.argv[1]) if len(sys.argv) > 1 else 9101), H).serve_forever()
