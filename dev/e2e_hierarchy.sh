#!/usr/bin/env bash
# End-to-end check of the cloud/local hierarchy with scripted models, a mock vault and no real quota:
#   - a cloud model is told about delegate_local,
#   - a long simple_search result reaches it condensed by the local model,
#   - a /bod routine reaches it with a briefing packet from the local model,
#   - the usage ledger records the offloads.
# Usage: dev/e2e_hierarchy.sh   (from the repo root; needs `cargo build --bin loki-server` and a built dist/)
set -u
ROOT=$(cd "$(dirname "$0")/.." && pwd)
T=$(mktemp -d); trap 'kill $(jobs -p) 2>/dev/null; rm -rf "$T"' EXIT
export HOME=$T/home XDG_CONFIG_HOME=$T/cfg XDG_DATA_HOME=$T/data
mkdir -p $HOME $XDG_CONFIG_HOME/loki/commands $XDG_DATA_HOME $T/vault/"50 - Notes" $T/vault/"10 - Daily"
printf "Run the beginning-of-day routine. Read yesterday's daily note and today's plan.\n" > $XDG_CONFIG_HOME/loki/commands/bod.md
for i in $(seq 1 400); do printf "A zebra story number $i: the zebra crossed the long dusty road at dawn, and every zebra in the herd followed the lead zebra down to the river to drink.\n" > "$T/vault/50 - Notes/zebra$i.md"; done
printf "Yesterday: three tasks, two done.\n" > "$T/vault/10 - Daily/yesterday.md"
python3 $ROOT/dev/fake_hierarchy_models.py 11500 9103 $T/models.log & python3 $ROOT/dev/mock_obsidian_server.py --root $T/vault --port 27127 --token t >/dev/null 2>&1 &
sleep 1
! ss -ltn | grep -q ":8493 " || { echo "port 8493 is already in use"; exit 2; }
LOKI_PORT=8493 LOKI_STATIC_DIR=$ROOT/dist $ROOT/src-tauri/target/debug/loki-server >$T/server.log 2>&1 &
sleep 3
curl -s -X POST localhost:8493/api/setup/commit -H 'content-type: application/json' -d '{"local":{"host":"127.0.0.1","port":11500,"model":"local:1b"},"cloud":{"provider":"openai","base_url":"http://127.0.0.1:9103/v1","api_key":"x","model":"cloudmodel"},"routing":"cloud","accept_cloud_only":false,"obsidian":{"host":"127.0.0.1","port":27127,"api_key":"t"},"pack":"none","account":{"username":"sam","password":"correct horse battery"}}' >/dev/null
sleep 8
TOKEN=$(curl -s -X POST localhost:8493/api/login -H 'content-type: application/json' -d '{"username":"sam","password":"correct horse battery"}' | python3 -c 'import sys,json;print(json.load(sys.stdin)["token"])')
chat() { SESSION=$1 timeout 120 node $ROOT/dev/sse_client.mjs http://127.0.0.1:8493 "$TOKEN" "$2" > $T/chat-$1.out 2>&1; }
pass=0; fail=0
check() { if [ "$2" = "yes" ]; then echo "  PASS: $1"; pass=$((pass+1)); else echo "  FAIL: $1"; fail=$((fail+1)); fi; }

echo "1) long search result is condensed before the cloud model sees it"
chat hA "zebra search please" >/dev/null; A=$(grep "^ANSWER A done" $T/models.log | tail -1); echo "   cloud said: $A"
case "$A" in *"condensed=True"*) check "result was condensed" yes;; *) check "result was condensed" no;; esac
case "$A" in *"hint=True"*) check "cloud model got the delegation hint" yes;; *) check "cloud model got the delegation hint" no;; esac
grep -q '"delegate_offered": true' $T/models.log && check "delegate_local was offered" yes || check "delegate_local was offered" no

echo "2) a routine starts from a briefing packet"
chat hB "/bod" >/dev/null; B=$(grep "^ANSWER B done" $T/models.log | tail -1); echo "   cloud said: $B"
case "$B" in *"packet=True"*"bodyok=True"*) check "packet reached the cloud model" yes;; *) check "packet reached the cloud model" no;; esac

echo "3) the ledger noted the offloads"
LED=$(find $XDG_DATA_HOME -name usage.jsonl | head -1); echo "   $(cat "$LED" 2>/dev/null | tr '\n' ' ' | cut -c1-300)"
grep -q '"what":"condense"' "$LED" && check "condense offload recorded" yes || check "condense offload recorded" no
grep -q '"what":"prep"' "$LED" && check "prep offload recorded" yes || check "prep offload recorded" no
SUM=$(curl -s "localhost:8493/api/v1/usage?days=1" -H "Authorization: Bearer $TOKEN"); echo "   /api/v1/usage: $SUM" | cut -c1-200
case "$SUM" in *chars_kept_from_cloud*) check "usage endpoint answers" yes;; *) check "usage endpoint answers" no;; esac
[ $fail -eq 0 ] || { echo "--- debug"; tail -4 $T/chat-hB.out | cut -c1-250; tail -12 $T/server.log | cut -c1-230; grep -E "^(CLOUD|LOCAL)" $T/models.log | cut -c1-230 | tail -12; }
echo "passed $pass, failed $fail"
[ $fail -eq 0 ]
