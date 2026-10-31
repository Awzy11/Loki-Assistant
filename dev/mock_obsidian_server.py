#!/usr/bin/env python3
"""
A minimal stand-in for the Obsidian Local REST API plugin, backed by a plain folder of
markdown files instead of a real Obsidian vault. Exists so Loki's importer (and anything
else that talks to ObsidianClient) can be exercised end-to-end against safe, synthetic data
without a second real Obsidian instance, and without touching the real vault or its REST
API port (27123) at all — this binds to 27124 by default.

Implements just the endpoints ObsidianClient (src-tauri/src/obsidian.rs) actually calls:
  GET  /vault/                  -> {"files": [...]}   (root listing)
  GET  /vault/{dir}/            -> {"files": [...]}   (dir listing, trailing "/" on dirs)
  GET  /vault/{path}            -> raw file content (text/plain body)
  POST /vault/{path}            -> append (or create) raw text body
  GET  /search/simple/?query=&contextLength=   -> naive substring search across all notes

Auth: same shape as the real plugin — `Authorization: Bearer <token>` must match
--token (default: dummy-test-token). Stdlib only, no dependencies to install.

Usage:
    python3 mock_obsidian_server.py [--root dummy-vault] [--port 27124] [--token dummy-test-token]
"""
import argparse
import json
import os
import urllib.parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

ROOT = None
TOKEN = None


def list_dir(rel_path: str):
    """Immediate children of rel_path, dirs suffixed with '/', matching the real plugin's
    convention (confirmed against Loki's own reference notes on that API's behavior)."""
    abs_path = os.path.join(ROOT, rel_path) if rel_path else ROOT
    if not os.path.isdir(abs_path):
        return None
    names = []
    for entry in sorted(os.listdir(abs_path)):
        if entry.startswith("."):
            continue
        full = os.path.join(abs_path, entry)
        names.append(entry + "/" if os.path.isdir(full) else entry)
    return names


class Handler(BaseHTTPRequestHandler):
    def log_message(self, fmt, *args):
        print(f"[mock-obsidian] {self.address_string()} - {fmt % args}")

    def _authorized(self) -> bool:
        auth = self.headers.get("Authorization", "")
        return auth == f"Bearer {TOKEN}"

    def _send_json(self, status: int, payload):
        body = json.dumps(payload).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _send_text(self, status: int, text: str):
        body = text.encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "text/plain; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if not self._authorized():
            return self._send_json(401, {"error": "unauthorized"})

        parsed = urllib.parse.urlparse(self.path)
        path = urllib.parse.unquote(parsed.path)

        if path.startswith("/search/simple/"):
            qs = urllib.parse.parse_qs(parsed.query)
            query = (qs.get("query") or [""])[0]
            results = []
            for dirpath, _dirnames, filenames in os.walk(ROOT):
                for fname in filenames:
                    if not fname.endswith(".md"):
                        continue
                    full = os.path.join(dirpath, fname)
                    rel = os.path.relpath(full, ROOT)
                    with open(full, "r", encoding="utf-8") as f:
                        content = f.read()
                    if query.lower() in content.lower():
                        results.append({"filename": rel, "score": 1.0})
            return self._send_json(200, results)

        if not path.startswith("/vault/"):
            return self._send_json(404, {"error": "not found"})

        rel = path[len("/vault/"):]
        if rel == "" or rel.endswith("/"):
            listing = list_dir(rel.rstrip("/"))
            if listing is None:
                return self._send_json(404, {"error": "directory not found"})
            return self._send_json(200, {"files": listing})

        abs_path = os.path.join(ROOT, rel)
        if not os.path.isfile(abs_path):
            return self._send_json(404, {"error": "file not found"})
        with open(abs_path, "r", encoding="utf-8") as f:
            content = f.read()
        return self._send_text(200, content)

    def do_POST(self):
        if not self._authorized():
            return self._send_json(401, {"error": "unauthorized"})

        parsed = urllib.parse.urlparse(self.path)
        path = urllib.parse.unquote(parsed.path)

        if path.startswith("/search/simple/"):
            # ObsidianClient POSTs to this endpoint (matches the real plugin's API shape).
            return self.do_GET()

        if not path.startswith("/vault/"):
            return self._send_json(404, {"error": "not found"})

        rel = path[len("/vault/"):]
        length = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(length).decode("utf-8") if length else ""

        abs_path = os.path.join(ROOT, rel)
        os.makedirs(os.path.dirname(abs_path), exist_ok=True)
        mode = "a" if os.path.isfile(abs_path) else "w"
        with open(abs_path, mode, encoding="utf-8") as f:
            f.write(body)
        return self._send_text(200, "ok")

    def do_DELETE(self):
        if not self._authorized():
            return self._send_json(401, {"error": "unauthorized"})
        path = urllib.parse.unquote(urllib.parse.urlparse(self.path).path)
        if not path.startswith("/vault/"):
            return self._send_json(404, {"error": "not found"})
        abs_path = os.path.join(ROOT, path[len("/vault/"):])
        if not os.path.isfile(abs_path):
            return self._send_json(404, {"error": "no such file"})
        os.remove(abs_path)
        return self._send_text(200, "ok")


def main():
    global ROOT, TOKEN
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", default=os.path.join(os.path.dirname(__file__), "dummy-vault"))
    parser.add_argument("--port", type=int, default=27124)
    parser.add_argument("--token", default="dummy-test-token")
    args = parser.parse_args()

    ROOT = os.path.abspath(args.root)
    TOKEN = args.token

    print(f"[mock-obsidian] serving '{ROOT}' on http://127.0.0.1:{args.port}")
    print(f"[mock-obsidian] expecting 'Authorization: Bearer {TOKEN}'")
    server = ThreadingHTTPServer(("127.0.0.1", args.port), Handler)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        print("\n[mock-obsidian] stopped")


if __name__ == "__main__":
    main()
