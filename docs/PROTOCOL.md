# Loki client protocol (v1)

Every Loki surface — the web app at `loki.example.org`, the installed Android/desktop PWA, the Tauri
desktop app, the Omarchy sidebar, and any future client — is a thin client of one `loki-server`.
**Models, tools, MCP servers, permissions, settings and sessions all live on the server.** A
client only: sends messages, renders events, answers approval prompts, and (optionally) runs
device tools on the machine it runs on. If you can do those four things you have full parity.

Base URL examples: `https://loki.example.org`, `https://loki-server.your-tailnet.ts.net:8443`,
`http://100.64.0.10:8420`. All `/api/*` calls except `/api/login` need
`Authorization: Bearer <token>`.

## 1. Auth
`POST /api/login {username, password}` → `{token}`. The token is the long-lived API token; store
it per device. (`~/.config/loki/api_token` on the server is the same token.)

## 2. Identify yourself (client info)
Chat and watch requests carry who you are:
```json
{ "id": "stable-per-install", "kind": "web|desktop|sidebar|android|cli", "capabilities": ["device_fs"] }
```
* `id` — stable per install (lets a mirror skip the echo of its own messages).
* `capabilities` — what you can do *for the server*. Today only **`device_fs`**: you can execute the
  `device_*` tools (§6) on your machine. Declare it only if that is true; the server routes file
  and shell requests only to clients that declared it. A phone or browser tab declares nothing.

## 3. Conversations
* A conversation is a `session_id` (client-generated, e.g. `web-<ms>-<rand>`; the server also accepts
  none and returns one in `started`).
* `GET /api/v1/history?session_id=` → `{turns:[{role,content}], in_routine_mode, force_local}`.
  `content` is what the person typed (`/bod`, not the expanded command).
* `GET /api/v1/sessions` → `{sessions:[{session_id,title,updated,turns}]}` — the picker.
* `GET|POST /api/v1/engines` — selectable models and the per-conversation / default choice
  (`{engine, session_id?}`; omit `session_id` to set the server default). `auto` is Loki's routing.
* Slash commands (`/bod`, `/eod`, …) are plain messages; the server expands them.
* `/exit` toggles "local models only" for the conversation; `/clear` is client-side (new session id).

## 4. Sending a turn (streaming)
`POST /api/v1/chat/stream`
```json
{ "session_id": "…", "message": "…", "images": ["<base64 png>"], "client": { …§2… } }
```
Response: Server-Sent Events; each frame is `event: <type>` + `data: <json>` (the JSON repeats `type`).
The stream ends after exactly one `done` or `error` for *your* turn. **Closing the connection
cancels the turn** (the Stop button). Only one turn runs per conversation at a time (a second
sender gets an `error`).

## 5. Watching a conversation (mirror)
`GET /api/v1/sessions/events?session_id=&client_id=&kind=&caps=device_fs` — attach without
sending. You receive the same events for every turn, whoever started it, starting with
`hello {session_id, attached}`. Never ends on its own; reconnect on drop. Use it while idle so a turn
started on another device shows up live and its approvals can be answered from here. Fetch
`/history` after `done` to resync.

### Events
| `type` | fields | meaning |
|---|---|---|
| `hello` | `session_id`, `attached` | (watch only) attached |
| `started` | `session_id` | turn accepted |
| `user_message` | `text`, `client_id` | a client submitted a message (skip if `client_id` is yours) |
| `text_delta` | `text` | assistant text |
| `reset` | — | discard text/tool cards streamed so far (a model failed; retrying another) |
| `tool_start` | `id`, `name`, `input` | tool call began (`name` is the real tool, e.g. `mcp__github__get_me`) |
| `tool_result` | `id`, `content`, `is_error` | its result |
| `status` | `message` | progress / failover note |
| `usage` | `info` | Claude usage windows (`info.unifiedWindows.five_hour.{utilization,resetsAt}`) |
| `permission_request` | `id`, `tool`, `input` | **needs a human**: answer via §6 `respond` with `{allow}` |
| `permission_auto` | `tool`, `input` | allowed by a saved always-rule (show it) |
| `device_request` | `id`, `name`, `input` | run a `device_*` tool and answer via `respond` with `{content,is_error}` — only sent if you declared `device_fs` |
| `resolved` | `id` | that request was answered (by anyone); dismiss it |
| `done` | `session_id`, `reply`, `via`, `note`, `in_routine_mode`, `force_local` | turn finished; `via` = engine that answered, `note` = why a fallback answered |
| `error` | `message` | turn failed |

## 6. Answering requests
`POST /api/v1/bridge/respond {id, value}` (404 = already answered/expired — ignore).
* approval: `value = {allow: true|false}`. Every attached client is asked; **the first answer wins**
  and the rest get `resolved`. No answer in 5 minutes = denied.
* "Always allow": `POST /api/v1/permissions/rules {tool, input}` (rules are server-side, shared by
  all devices; deletes and other outward-facing actions are refused). `GET /api/v1/permissions/rules`,
  `POST /api/v1/permissions/rules/remove {rule}`.

### Device tools (clients with `device_fs`)
`device_read_file {path,offset?,limit?}`, `device_list_dir {path}`, `device_write_file {path,content}`,
`device_edit_file {path,old_string,new_string,replace_all?}`, `device_run_command {command,cwd?,timeout_secs?}`.
Execute locally (the desktop app and sidebar use the same policy module: protected credential paths
are refused, writes/shell follow the local `[desktop]` switches, shell is off by default) and answer
`{content, is_error}`. The server has already asked the user for writes/commands.

## 7. Other endpoints
`GET /api/v1/mcp` (connected MCP servers), `GET|POST /api/config/{llm,embedding,claude}` (settings),
`/api/v1/history`, `/api/v1/query`, `/api/v1/ingest`, `/api/v1/webhook/voice-note` (separate token).

## 8. Parity checklist for a surface
- [ ] login + token storage; [ ] send turn with client info; [ ] render `text_delta`, tool cards
(`tool_start`/`tool_result`), `status`, `usage`; [ ] Stop = close the stream
- [ ] approvals: render `permission_request`, answer allow / allow-always / deny, dismiss on `resolved`
- [ ] mirror: `sessions/events` while idle; show `user_message` from others; resync on `done`
- [ ] sessions picker + resume; engine picker (per conversation + "make default")
- [ ] settings: always-rules list/revoke, Claude/MCP settings
- [ ] device tools only if the surface can really run them (declare `device_fs` only then)

| Surface | Implementation | Device tools |
|---|---|---|
| Web `loki.example.org`, Tailscale HTTPS URL | React app (`src/`), served by `loki-server` | no |
| Android | the same web app, installed as a PWA (manifest + service worker) | no (declares nothing) |
| Desktop app | Tauri window on the same web app | yes (`device_call` command) |
| Sidebar (Omarchy) | QML client of this protocol + `loki-device` helper | yes |
