# Email for Loki's local models

Read-only Gmail for the self-hosted models, shown on the Home dashboard (Mail card) and available to the
local model as the `mail_inbox` and `mail_read` tools. **Email is Tier 3: it never goes to a cloud model.**

## How the privacy line is held
- The tools are native to Loki (not an MCP server), so the delegated Claude path can't reach them.
- They are `Tier::LocalOnly`, and `agent::run_turn` refuses to run them at all when the model answering is a cloud one.
- A turn that uses them can't be handed to a cloud model afterwards (the existing local-only rule).
- The dashboard card face shows counts only; sender/subject lines are on the Mail page. "Ask Loki" is hidden on it.
- Triage (reply / fyi / noise) is done by the local model, in memory only.
- Debug logs redact mail tool arguments and results.

## One-time Google setup (about 10 minutes)
1. https://console.cloud.google.com → create a project (e.g. "Loki mail").
2. APIs & Services → Library → enable **Gmail API**.
3. APIs & Services → OAuth consent screen: User type **External**, app name "Loki", your address as support/developer contact.
   Add both accounts under **Test users**. Then click **Publish app** (status "In production") — in "Testing" status Google expires
   refresh tokens after 7 days. An unverified app only shows a warning ("Google hasn't verified this app" → Advanced → continue), and
   only for accounts you use.
4. APIs & Services → Credentials → Create credentials → **OAuth client ID** → type **Desktop app** → create → **Download JSON**. It lands in ~/Downloads as `client_secret_<numbers>.apps.googleusercontent.com.json` (the `*` in the commands below matches that name).
5. On the laptop, once per account:
   ```
   ~/Desktop/Loki/ops/gmail-auth.py ~/Downloads/client_secret*.json you@example.com
   ~/Desktop/Loki/ops/gmail-auth.py ~/Downloads/client_secret*.json other@example.com
   ```
   Each opens Google's consent page for the **read-only** scope, checks you approved the right account, and installs the token on the Loki server
   (`~/.config/loki/gmail_tokens/<email>.json`, mode 600; each token file also carries the OAuth client it was approved with, so accounts from different Google projects work). Tokens are never printed.
6. Restart loki-server. Check: `loki-cli mail-status` (counts only).

Revoke any time at https://myaccount.google.com/permissions, or delete the token file.

## Later: mark as read, trash, draft replies
These need the wider `gmail.modify` scope (re-run the script with that scope). Plan: user-initiated dashboard buttons (Mark read, Trash)
and approval-gated agent tools (Tier 2); drafts are created as Gmail drafts only. Loki will have no send function. Permanent delete is not
planned — Trash only, which Gmail keeps for 30 days.
