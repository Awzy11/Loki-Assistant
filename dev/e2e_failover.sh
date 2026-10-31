#!/usr/bin/env bash
# End-to-end check of Claude -> fallback failover and the context handoff, with no real
# models, no real vault and no quota: fake LLM server + fake `claude` + mock Obsidian.
# Usage: dev/e2e_failover.sh   (from the repo root; needs `cargo build --bin loki-server`)
set -u
ROOT=$(cd "$(dirname "$0")/.." && pwd)
T=$(mktemp -d); trap 'kill $(jobs -p) 2>/dev/null; rm -rf "$T"' EXIT
export XDG_CONFIG_HOME=$T/cfg XDG_DATA_HOME=$T/data FAKE_CLAUDE_LOG=$T/claude.log FAKE_CLAUDE_STATE=$T/state
if [ -n "${REAL_CLAUDE:-}" ]; then CLAUDE_BIN=claude; else CLAUDE_BIN=$ROOT/dev/fake_claude.sh; fi
mkdir -p $XDG_CONFIG_HOME/loki $XDG_DATA_HOME $FAKE_CLAUDE_STATE $T/vault $T/home/.claude/commands
printf "Greet \$ARGUMENTS warmly. inference tool test\n" > $T/home/.claude/commands/hello-cmd.md
cp -r $ROOT/dev/dummy-vault/. $T/vault/
echo victim > "$T/vault/50 - Notes/victim.md"
[ -n "${START_LIMITED:-}" ] && touch $FAKE_CLAUDE_STATE/limit
cat > $XDG_CONFIG_HOME/loki/config.toml <<TOML
[llm]
vendor = "openai_compatible"
base_url = "http://127.0.0.1:9101/v1"
api_key = "x"
model = "${PRIMARY_MODEL:-bad}"
[[fallbacks]]
vendor = "openai_compatible"
base_url = "http://127.0.0.1:9101/v1"
api_key = "x"
model = "good"
[embedding]
vendor = "openai_compatible"
base_url = "http://127.0.0.1:9101/v1"
api_key = "x"
model = "emb"
[obsidian]
host = "127.0.0.1"
port = 27124
api_key = "t"
[claude]
binary_path = "$CLAUDE_BIN"
routine_working_dir = "$T"
bridge_binary = "$ROOT/src-tauri/target/debug/loki-bridge-mcp"
bridge_url = "http://127.0.0.1:8499"
mode = "${CLAUDE_MODE:-delegated}"
extra_allowed_tools = ["mcp__fake__get_time"]
[agy]
binary_path = "$ROOT/dev/fake_agy.sh"
[remote]
hosts = ["austinlaptop"]
[skills]
dirs = ["$T/home/.claude/skills"]
[mcp]
import_claude_servers = false
[[mcp.servers]]
name = "fake"
command = "python3"
args = ["$ROOT/dev/fake_mcp.py"]
TOML
python3 $ROOT/dev/fake_llm_server.py 9101 & python3 $ROOT/dev/mock_obsidian_server.py --root $T/vault --port 27124 --token t >/dev/null 2>&1 &
start_server() { FAKE_AGY_LOG=$T/agy.log LOKI_SSH_BIN=$ROOT/dev/fakebin/ssh HOME=$T/home LOKI_STATIC_DIR=$ROOT/dist LOKI_CLAUDE_COOLDOWN_SECS=4 LOKI_PORT=8499 LOKI_BIND=127.0.0.1 $ROOT/src-tauri/target/debug/loki-server >$T/server.log 2>&1 & SERVER_PID=$!; }
start_server
sleep 3; grep -q listening $T/server.log || { echo "server failed to start:"; cat $T/server.log; exit 1; }
PW=$(grep password: $T/server.log | awk '{print $2}')
TOKEN=$(curl -s -X POST localhost:8499/api/login -H 'Content-Type: application/json' -d "{\"username\":\"loki\",\"password\":\"$PW\"}" | python3 -c 'import sys,json;print(json.load(sys.stdin)["token"])')
if [ "${1:-}" = "--client" ]; then shift; node $ROOT/dev/sse_client.mjs http://127.0.0.1:8499 "$TOKEN" "$@"; rc=$?; grep -E "claude failed over|loki-debug\] (blocked|permission)" $T/server.log | cut -c1-600; exit $rc; fi
if [ "${1:-}" = "--serve" ]; then echo "UI: http://127.0.0.1:8499  user=loki password=$PW"; sleep ${2:-1800}; exit 0; fi
SID=${SID:-e2e}
chat() { curl -s -X POST localhost:8499/api/v1/chat -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' -d "{\"session_id\":\"$SID\",\"message\":\"$1\"}"; echo; }
if [ "${1:-}" = "--inference" ]; then
  H="Authorization: Bearer $TOKEN"
  ev() { node $ROOT/dev/sse_client.mjs http://127.0.0.1:8499 "$TOKEN" "$@" 2>&1 | grep -E '"type":"(tool_start|permission_request|done|error)"' | sed -E 's/"(input|reply)":"?[^,}]{0,40}[^,}]*/"\1":…/' | cut -c1-170; }
  echo "1) auto + mode=inference: Claude is a pure model in Loki's loop (search_tools -> call_tool -> approval)"; ev "inference tool test" --allow
  echo "2) the CLI session carries the conversation; later calls send only what it hasn't seen:"
  grep -E "^LEAN" $FAKE_CLAUDE_LOG | sed 's/^/   /'
  python3 - "$FAKE_CLAUDE_LOG" <<'PY'
import sys,re
log=open(sys.argv[1]).read().split("---END-DELTA---\n")
calls=[]
for chunk in log:
    m=re.match(r"LEAN mode=(\w+) .*?delta_chars=(\d+)\n(.*)", chunk, re.S)
    if m: calls.append((m[1], int(m[2]), m[3]))
modes=[c[0] for c in calls[:3]]
ok = modes==["new","resume","resume"]
first_full = calls[0][2]
ok &= all("inference tool test" not in c[2] for c in calls[1:3])   # the user message is NOT re-sent
print("   modes:",modes,"| earlier content re-sent on resume:", any("inference tool test" in c[2] for c in calls[1:3]), "->", "PASS" if ok else "FAIL")
PY
  echo "2b) another engine answers in between; Claude's next delta must carry the missed turns:"
  pick() { curl -s -X POST localhost:8499/api/v1/engines -H "$H" -H 'Content-Type: application/json' -d "{\"engine\":\"$1\",\"session_id\":\"div1\"}" -o /dev/null; }
  say() { SESSION=div1 node $ROOT/dev/sse_client.mjs http://127.0.0.1:8499 "$TOKEN" "$1" 2>&1 | grep -E '"type":"(done|error)"' | sed -E 's/.*"reply":"([^"]*)".*"via":"([^"]*)".*/   \2: \1/' | cut -c1-120; }
  pick claude:sonnet; say "hello one"
  pick configured;    say "hello two"
  pick claude:sonnet; say "hello three"
  python3 - "$FAKE_CLAUDE_LOG" <<'PY'
import sys,re
chunks=[c for c in open(sys.argv[1]).read().split("---END-DELTA---\n") if c.startswith("LEAN")]
last=chunks[-1]; first_line=last.split("\n",1)[0]; body=last.split("\n",1)[1]
ok=("mode=resume" in first_line) and "hello two" in body and "good answered" in body and "hello three" in body and "hello one" not in body
print("   resume delta carries the missed turns, not the old ones:", "PASS" if ok else "FAIL\n"+last[:300])
PY
  echo "2c) CLI session lost (pruned): Loki must recover with a full replay in a new session:"
  rm -rf $FAKE_CLAUDE_STATE/sessions
  say "hello four"
  python3 - "$FAKE_CLAUDE_LOG" <<'PY'
import sys
chunks=[c for c in open(sys.argv[1]).read().split("---END-DELTA---\n") if c.startswith("LEAN")]
last=chunks[-1]; head=last.split("\n",1)[0]; body=last.split("\n",1)[1]
ok="mode=new" in head and all(w in body for w in ["hello one","hello two","hello three","hello four"])
print("   fresh session with full replay:", "PASS" if ok else "FAIL\n"+last[:300])
PY
  echo "2d) slash command: model sees the file, history/UI still shows what was typed:"
  SESSION=cmd1 node $ROOT/dev/sse_client.mjs http://127.0.0.1:8499 "$TOKEN" "/hello-cmd world" --allow 2>&1 | grep -E '"type":"(done|error)"' | sed -E 's/.*"reply":"([^"]*)".*"via":"([^"]*)".*/   \2: \1/' | cut -c1-110
  grep -q "Greet world warmly" $FAKE_CLAUDE_LOG && echo "   PASS: expanded instructions (with \$ARGUMENTS) reached the model" || echo "   FAIL: not expanded"
  curl -s "localhost:8499/api/v1/history?session_id=cmd1" -H "$H" | python3 -c 'import sys,json;t=json.load(sys.stdin)["turns"];print("   history first turn:",repr(t[0]["content"]),"->","PASS" if t[0]["content"]=="/hello-cmd world" else "FAIL")'
  echo "3) engine picker:"; curl -s "localhost:8499/api/v1/engines?session_id=pick1" -H "$H" | python3 -c 'import sys,json;d=json.load(sys.stdin);print("   ",[e["id"] for e in d["engines"]],"default=",d["default"],"mode=",d["claude_mode"])'
  curl -s -X POST localhost:8499/api/v1/engines -H "$H" -H 'Content-Type: application/json' -d '{"engine":"claude:haiku","session_id":"pick1"}' -o /dev/null -w "   set session engine claude:haiku -> %{http_code}\n"
  curl -s -X POST localhost:8499/api/v1/engines -H "$H" -H 'Content-Type: application/json' -d '{"engine":"nonsense"}' -o /dev/null -w "   set bogus engine -> %{http_code} (expect 400)\n"
  SESSION=pick1 node $ROOT/dev/sse_client.mjs http://127.0.0.1:8499 "$TOKEN" "hello" 2>&1 | grep -E '"type":"(done|error)"' | cut -c1-140
  echo "   model flag seen by the CLI:"; grep -E "^LEAN" $FAKE_CLAUDE_LOG | tail -1
  echo "4) explicit cloud engine + sensitive message must refuse:"
  SESSION=pick1 node $ROOT/dev/sse_client.mjs http://127.0.0.1:8499 "$TOKEN" "my ssn is 123-45-6789" 2>&1 | grep -E '"type":"(done|error)"' | cut -c1-220
  grep -q "123-45-6789" $FAKE_CLAUDE_LOG && echo "   FAIL: sensitive text reached claude" || echo "   PASS: sensitive text never reached claude"
  echo "5) explicit configured engine (no Claude involved):"
  curl -s -X POST localhost:8499/api/v1/engines -H "$H" -H 'Content-Type: application/json' -d '{"engine":"configured","session_id":"pick2"}' -o /dev/null
  SESSION=pick2 node $ROOT/dev/sse_client.mjs http://127.0.0.1:8499 "$TOKEN" "hello" 2>&1 | grep -E '"type":"(done|error)"' | cut -c1-140
  echo "6) explicit claude-code with the limit hit must error, not fail over:"
  curl -s -X POST localhost:8499/api/v1/engines -H "$H" -H 'Content-Type: application/json' -d '{"engine":"claude-code","session_id":"pick3"}' -o /dev/null
  touch $FAKE_CLAUDE_STATE/limit
  SESSION=pick3 node $ROOT/dev/sse_client.mjs http://127.0.0.1:8499 "$TOKEN" "hello" 2>&1 | grep -E '"type":"(done|error)"' | cut -c1-200
  exit 0
fi
if [ "${1:-}" = "--multi" ]; then
  touch $FAKE_CLAUDE_STATE/limit  # force the local (Loki-harness) path
  node $ROOT/dev/multi_client.mjs http://127.0.0.1:8499 "$TOKEN"; exit $?
fi
if [ "${1:-}" = "--agy" ]; then
  H="Authorization: Bearer $TOKEN"
  echo "engines include Agy: $(curl -s localhost:8499/api/v1/engines -H "$H" | python3 -c 'import sys,json;print([e["id"] for e in json.load(sys.stdin)["engines"] if e["id"].startswith("agy:")])')"
  curl -s -X POST localhost:8499/api/v1/engines -H "$H" -H 'Content-Type: application/json' -d '{"engine":"agy:gemini-3.1-pro-high","session_id":"agy1"}' -o /dev/null -w "pick agy:gemini-3.1-pro-high -> %{http_code}\n"
  curl -s -X POST localhost:8499/api/v1/engines -H "$H" -H 'Content-Type: application/json' -d '{"engine":"agy:not-a-model","session_id":"agy1"}' -o /dev/null -w "pick unknown agy model -> %{http_code} (expect 400)\n"
  say() { SESSION=agy1 node $ROOT/dev/sse_client.mjs http://127.0.0.1:8499 "$TOKEN" "$1" --allow 2>&1 | grep -E '"type":"(tool_start|done|error)"' | sed -E 's/"input":.*//; s/.*"reply":"([^"]*)".*"via":"([^"]*)".*/DONE via=\2: \1/' | cut -c1-150; }
  echo "1) plain turn:";                 say "hello"
  echo "2) tool loop (search_tools):";   say "agy tool test"
  echo "3) denied-tool empty reply recovers with a reminder:"; say "agy denied test"
  echo "4) sensitive message must be refused for a cloud engine:"; say "my ssn is 123-45-6789"
  grep -q "123-45-6789" $T/agy.log && echo "   FAIL: sensitive text reached agy" || echo "   PASS: sensitive text never reached agy"
  echo "agy invocations (model, scratch cwd, prompt size):"; grep '^AGY' $T/agy.log | sort | uniq -c | sed 's/^/   /'
  exit 0
fi
if [ "${1:-}" = "--remote" ]; then
  touch $FAKE_CLAUDE_STATE/limit  # force the local (Loki-harness) path
  echo "remote_run_command: allow-listed host (approve) then a host NOT on the list:"
  node $ROOT/dev/sse_client.mjs http://127.0.0.1:8499 "$TOKEN" "remote test please" --allow 2>&1 | grep -E '"type":"(permission_request|tool_result|done)"' | sed -E 's/"input":(\{[^}]*\}).*/"input":\1/; s/.*"reply":"(.{0,200}).*"via".*/DONE: \1/' | cut -c1-230
  exit 0
fi
if [ "${1:-}" = "--cli" ]; then
  touch $FAKE_CLAUDE_STATE/limit  # force the local (Loki-harness) path
  CLI=$ROOT/src-tauri/target/debug/loki-cli; export LOKI_URL=http://127.0.0.1:8499 LOKI_TOKEN=$TOKEN
  echo "engines via CLI:   $($CLI engines | python3 -c 'import sys,json;print([e["id"] for e in json.load(sys.stdin)["engines"]])')"
  OUT=$T/cli_chat.out; SESSION_ID=cli-$$
  $CLI chat $SESSION_ID "multi test" > $OUT 2>&1 & CPID=$!
  # Approve the prompt from a *separate* process, as the sidebar does with its button.
  for i in $(seq 1 100); do
    RID=$(python3 - $OUT <<'PY'
import sys,json
for l in open(sys.argv[1]):
    try: o=json.loads(l)
    except: continue
    if o.get("type")=="permission_request": print(o["id"]); break
PY
)
    [ -n "$RID" ] && break; sleep 0.2
  done
  echo "approval requested: ${RID:-NONE} -> answering allow from a separate loki-cli process"
  [ -n "$RID" ] && $CLI respond "$RID" '{"allow":true}'
  wait $CPID; echo "chat exit code: $? (0 = done)"; [ -n "${KEEP_CLI_OUT:-}" ] && cp $OUT "$KEEP_CLI_OUT"
  python3 - $OUT <<'PY'
import sys,json
ev=[json.loads(l) for l in open(sys.argv[1]) if l.startswith("{")]
types=[e["type"] for e in ev]
dev=[e for e in ev if e["type"]=="tool_result" and e.get("content") and not e["content"].startswith("error")]
print("PASS" if "device_request" in types and dev else "FAIL", ": CLI served the device request itself (file read result:", (dev[0]["content"][:30] if dev else None), ")")
print("PASS" if "permission_request" in types and "done" in types else "FAIL", ": approval then done")
print("events:", " > ".join(t for t in types if t not in ("text_delta","usage")))
PY
  echo "history via CLI: $($CLI history $SESSION_ID | python3 -c 'import sys,json;t=json.load(sys.stdin)["turns"];print(len(t),"turns; first:",repr(t[0]["content"]))')"
  echo "sessions via CLI: $($CLI sessions | python3 -c 'import sys,json;print([s["title"] for s in json.load(sys.stdin)["sessions"]])')"
  exit 0
fi
if [ "${1:-}" = "--mcp" ]; then
  touch $FAKE_CLAUDE_STATE/limit  # force the local (Loki-harness) path
  echo "mcp status:"; curl -s localhost:8499/api/v1/mcp -H "Authorization: Bearer $TOKEN"; echo
  echo "scripted turn (approve the echo prompt):"
  node $ROOT/dev/sse_client.mjs http://127.0.0.1:8499 "$TOKEN" "mcp test please" --allow 2>&1 | grep -E '"type":"(tool_start|tool_result|permission_request|done|error)"' | cut -c1-210
  exit 0
fi
if [ "${1:-}" = "--rules" ]; then
  C() { node $ROOT/dev/sse_client.mjs http://127.0.0.1:8499 "$TOKEN" "Use the WebFetch tool on https://example.com and give the page title in 3 words." $1 2>&1 | grep -E '"type":"(permission_request|permission_auto|error)"' | cut -c1-140; }
  echo "run 1 (approve with 'always'):"; C --always
  echo "rules on server:"; curl -s localhost:8499/api/v1/permissions/rules -H "Authorization: Bearer $TOKEN"; echo
  echo "run 2 (expect permission_auto, no request):"; C --deny
  echo "revoke + run 3 (expect a request again):"
  curl -s -X POST localhost:8499/api/v1/permissions/rules/remove -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' -d '{"rule":"WebFetch:example.com"}' -o /dev/null -w "   remove -> %{http_code}\n"
  C --deny
  echo "delete can't be always:"; curl -s -X POST localhost:8499/api/v1/permissions/rules -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' -d '{"tool":"delete_file","input":{}}' -w "   -> %{http_code}\n"
  exit 0
fi
if [ "${1:-}" = "--sessions" ]; then
  SID=sess-e2e; short() { python3 -c 'import sys,json;d=json.loads(sys.stdin.read());print("   via=%s reply=%s"%(d["via"],d["reply"]))'; }
  echo "1) normal turn -> claude";                 chat "hello one" | short
  echo "2) sensitive turn -> must stay local";     chat "my ssn is 123-45-6789" | short
  echo "3) clean turn -> claude";                  chat "hello three" | short
  if grep -q "123-45-6789" $FAKE_CLAUDE_LOG; then echo "   FAIL: sensitive text reached claude"; else echo "   PASS: sensitive text never reached claude"; fi
  grep -q "private exchange" $FAKE_CLAUDE_LOG && echo "   PASS: handoff carried a placeholder for the private exchange"
  S1=$(grep -o -- '--session-id [0-9a-f-]*' $FAKE_CLAUDE_LOG | head -1 | awk '{print $2}')
  echo "4) restart server; same session -> expect --resume $S1"
  kill $SERVER_PID; sleep 1; start_server; sleep 3
  chat "hello four" | short
  tail -n 30 $FAKE_CLAUDE_LOG | grep FLAGS | tail -1
  if tail -n 30 $FAKE_CLAUDE_LOG | grep FLAGS | tail -1 | grep -q -- "--resume $S1"; then echo "   PASS: claude session restored across restart"; else echo "   FAIL: claude session not restored"; fi
  echo "   history after restart:"; curl -s "localhost:8499/api/v1/history?session_id=sess-e2e" -H "Authorization: Bearer $TOKEN" | python3 -c 'import sys,json;d=json.load(sys.stdin);print("   ",len(d["turns"]),"turns")'
  echo "   vault log dir:"; ls "$T/vault/98 - Loki Conversations/" 2>&1 | sed 's/^/   /'
  echo "5) session list:"; curl -s localhost:8499/api/v1/sessions -H "Authorization: Bearer $TOKEN"; echo
  exit 0
fi
echo "1) Claude healthy:";           chat "first question"
touch $FAKE_CLAUDE_STATE/limit
echo "2) Claude at limit -> expect fallback 'good' with note:"; chat "second question"
echo "3) Cooldown active (no new claude spawn):"; n=$(grep -c END-PROMPT $T/claude.log); chat "third question"; echo "claude spawns during turn 3: $(( $(grep -c END-PROMPT $T/claude.log) - n ))"
rm $FAKE_CLAUDE_STATE/limit; sleep 5
echo "4) Claude recovered -> expect via=claude, and its prompt carries the handoff:"; chat "fourth question"
echo "--- last prompt sent to claude:"; awk '/---END-PROMPT---/{n++} END{}1' $T/claude.log | tail -12

echo "5) SSE stream with a tool call:"
curl -sN -X POST localhost:8499/api/v1/chat/stream -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' -d '{"session_id":"e2e","message":"please use a tool"}'
