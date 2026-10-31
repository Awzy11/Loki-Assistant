#!/usr/bin/env python3
"""Authorize one Gmail account for Loki's local models (read-only) and install its token on the Loki server.

Run on the LAPTOP (it has a browser):
    ops/gmail-auth.py ~/Downloads/client_secret*.json you@example.com
    ops/gmail-auth.py ~/Downloads/client_secret*.json other@example.com
    ops/gmail-auth.py reuse you@example.com --modify   # wider scope; "reuse" = use the client already on the server: mark read, labels, Trash, drafts

What it does: opens Google's consent page for the read-only scope, waits for the approval on a
loopback port, checks the approved account is the one you named, then copies the refresh token to
the server (~/.config/loki/gmail_tokens/<email>.json, mode 600) together with the OAuth client it was approved with
(each account can use a different Google project's client). Tokens are
never printed or kept on this machine. Standard library only. See docs/MAIL.md.
"""
import http.server, json, os, secrets, shutil, subprocess, sys, tempfile, threading, urllib.parse, urllib.request, webbrowser

MODIFY = "--modify" in sys.argv
if MODIFY:
    sys.argv.remove("--modify")
SCOPE = "https://www.googleapis.com/auth/gmail.modify" if MODIFY else "https://www.googleapis.com/auth/gmail.readonly"
HOST = os.environ.get("LOKI_HOST", "loki-server")


def main():
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    client_path, email = os.path.expanduser(sys.argv[1]), sys.argv[2].strip().lower()
    c = None
    if client_path == "reuse" or (not os.path.exists(client_path) and not __import__("glob").glob(os.path.expanduser("~/Downloads/client_secret*.json"))):
        # Reuse the OAuth client already stored in this account's token file on the server (read in memory, never printed).
        r = subprocess.run(["ssh", HOST, f"cat ~/.config/loki/gmail_tokens/{email}.json"], capture_output=True, text=True)
        try:
            old = json.loads(r.stdout)
            c = {"client_id": old["client_id"], "client_secret": old["client_secret"]}
            print(f"Reusing the OAuth client already installed for {email} on {HOST}.")
        except Exception:
            sys.exit(f"No client file given and none found for {email} on {HOST}. Download one (docs/MAIL.md, step 4) and pass its path.")
    if c is None:
        if not os.path.exists(client_path):
            # "client_secret_XXXX.json" in the docs is a placeholder: use the newest real download if there is one.
            import glob
            found = sorted(glob.glob(os.path.expanduser("~/Downloads/client_secret*.json")), key=os.path.getmtime)
            client_path = found[-1]
            print("Using", client_path)
        raw = json.load(open(client_path))
        c = raw.get("installed") or raw.get("web")
        if not c:
            sys.exit("That doesn't look like an OAuth client file (no 'installed' section). Create a Desktop-app client.")
    state = secrets.token_urlsafe(16)
    got = {}

    class H(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            q = urllib.parse.parse_qs(urllib.parse.urlparse(self.path).query)
            if q.get("state", [""])[0] == state and "code" in q:
                got["code"] = q["code"][0]
                msg = "Loki is authorized for this account. You can close this tab."
            else:
                got["error"] = q.get("error", ["unexpected request"])[0]
                msg = "Authorization failed: " + got["error"]
            self.send_response(200)
            self.send_header("Content-Type", "text/plain; charset=utf-8")
            self.end_headers()
            self.wfile.write(msg.encode())
            threading.Thread(target=self.server.shutdown, daemon=True).start()

        def log_message(self, *a):
            pass

    srv = http.server.HTTPServer(("127.0.0.1", 0), H)
    redirect = f"http://127.0.0.1:{srv.server_port}"
    url = "https://accounts.google.com/o/oauth2/v2/auth?" + urllib.parse.urlencode({
        "client_id": c["client_id"], "redirect_uri": redirect, "response_type": "code", "scope": SCOPE,
        "access_type": "offline", "prompt": "consent", "login_hint": email, "state": state,
    })
    print(f"Opening Google's consent page for {email} (" + ("read and modify" if MODIFY else "read-only") + " mail access)…")
    print("If a browser doesn't open, visit:\n" + url)
    webbrowser.open(url)
    srv.serve_forever()
    if "code" not in got:
        sys.exit("Not authorized: " + got.get("error", "no code received"))

    body = urllib.parse.urlencode({"code": got["code"], "client_id": c["client_id"], "client_secret": c["client_secret"], "redirect_uri": redirect, "grant_type": "authorization_code"}).encode()
    tok = json.load(urllib.request.urlopen(urllib.request.Request("https://oauth2.googleapis.com/token", data=body)))
    if "refresh_token" not in tok:
        sys.exit("Google returned no refresh token. Remove Loki at myaccount.google.com/permissions and run this again.")
    if tok.get("scope", "").strip() != SCOPE:
        sys.exit("Unexpected scope granted: " + tok.get("scope", ""))
    req = urllib.request.Request("https://gmail.googleapis.com/gmail/v1/users/me/profile", headers={"Authorization": "Bearer " + tok["access_token"]})
    who = json.load(urllib.request.urlopen(req)).get("emailAddress", "").lower()
    if who != email:
        sys.exit(f"You approved {who}, not {email}. Nothing was saved.")

    tmp = tempfile.mkdtemp()
    try:
        path = os.path.join(tmp, f"{email}.json")
        fd = os.open(path, os.O_WRONLY | os.O_CREAT, 0o600)
        with os.fdopen(fd, "w") as f:
            # each account keeps the client it was authorized with (they can come from different Google projects)
            json.dump({"email": email, "refresh_token": tok["refresh_token"], "scope": SCOPE, "client_id": c["client_id"], "client_secret": c["client_secret"]}, f)
        subprocess.run(["ssh", HOST, "mkdir -p ~/.config/loki/gmail_tokens && chmod 700 ~/.config/loki ~/.config/loki/gmail_tokens"], check=True)
        subprocess.run(["scp", "-q", path, f"{HOST}:.config/loki/gmail_tokens/{email}.json"], check=True)
        subprocess.run(["ssh", HOST, f"chmod 600 ~/.config/loki/gmail_client.json ~/.config/loki/gmail_tokens/{email}.json"], check=True)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    level = "read and modify" if MODIFY else "read-only"
    print(f"Done: {email} is authorized ({level}) and installed on {HOST}. Restart loki-server to pick it up.")


if __name__ == "__main__":
    main()
