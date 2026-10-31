#!/usr/bin/env python3
"""Minimal stdio MCP server for tests (no SDK). Tools: echo(text), get_time()."""
import json, sys

TOOLS = [
    {"name": "echo", "description": "Echo some text back (test tool that needs approval).",
     "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}}, "required": ["text"]}},
    {"name": "get_time", "description": "Get the fake time (test tool pre-approved).",
     "inputSchema": {"type": "object", "properties": {}}},
]

for line in sys.stdin:
    try:
        msg = json.loads(line)
    except ValueError:
        continue
    mid, method = msg.get("id"), msg.get("method")
    if mid is None:
        continue  # notification
    if method == "initialize":
        result = {"protocolVersion": msg["params"].get("protocolVersion", "2025-03-26"),
                  "capabilities": {"tools": {}}, "serverInfo": {"name": "fake", "version": "1"}}
    elif method == "tools/list":
        result = {"tools": TOOLS}
    elif method == "tools/call":
        name, args = msg["params"]["name"], msg["params"].get("arguments") or {}
        text = f"echo: {args.get('text')}" if name == "echo" else "12:00 fake o'clock"
        result = {"content": [{"type": "text", "text": text}]}
    else:
        result = {}
    sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": mid, "result": result}) + "\n")
    sys.stdout.flush()
