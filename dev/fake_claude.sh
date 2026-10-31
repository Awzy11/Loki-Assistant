#!/usr/bin/env bash
# Stand-in for the `claude` CLI in stream-json mode. Two shapes:
#  - delegated (`-p "<prompt>" --resume/--session-id ...`): scripted replies, as before.
#  - lean inference (`-p` + prompt on stdin + --system-prompt-file): Loki's own loop drives; this
#    answers with `<<TOOL_CALLS>>` lines or text based on the transcript it was shown.
# Logs to $FAKE_CLAUDE_LOG. While $FAKE_CLAUDE_STATE/limit exists it reports a usage limit.
prompt=""; flags=""; sysfile=""; model=""; sess_new=""; sess_resume=""
while [ $# -gt 0 ]; do
  case "$1" in
    -p) if [ -n "${2:-}" ] && [ "${2#--}" = "$2" ]; then prompt="$2"; shift; fi;;
    --resume) flags="$flags $1 $2"; sess_resume="$2"; shift;;
    --session-id) flags="$flags $1 $2"; sess_new="$2"; shift;;
    --system-prompt-file) sysfile="$2"; shift;;
    --model) model="$2"; shift;;
  esac
  shift
done
emit_result() { echo '{"type":"result","is_error":false,"result":'"$(python3 -c 'import json,sys;print(json.dumps(sys.argv[1]))' "$1")"'}'; }
emit_text() { echo '{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":'"$(python3 -c 'import json,sys;print(json.dumps(sys.argv[1]))' "$1")"'}}}'; }
usage() { echo '{"type":"rate_limit_event","rate_limit_info":{"status":"allowed","unifiedWindows":{"five_hour":{"utilization":0.5,"resetsAt":1791508200}}}}'; }

if [ -e "${FAKE_CLAUDE_STATE:-/nonexistent}/limit" ]; then
  echo '{"type":"result","is_error":true,"result":"Claude AI usage limit reached|1760000000"}'; exit 1
fi

if [ -n "$sysfile" ]; then
  # ---- lean inference mode ----
  delta=$(cat)
  # Simulated CLI session: --session-id starts a history file, --resume appends to it.
  sdir="${FAKE_CLAUDE_STATE:-/tmp}/sessions"; mkdir -p "$sdir"
  if [ -n "$sess_resume" ]; then
    [ -f "$sdir/$sess_resume" ] || { echo '{"type":"result","is_error":true,"result":"No conversation found with session ID"}'; exit 1; }
    printf '%s' "$delta" >> "$sdir/$sess_resume"; transcript=$(cat "$sdir/$sess_resume"); mode=resume
  elif [ -n "$sess_new" ]; then
    printf '%s' "$delta" > "$sdir/$sess_new"; transcript="$delta"; mode=new
  else transcript="$delta"; mode=stateless; fi
  { printf 'LEAN mode=%s model=%s sys_has_tools=%s delta_chars=%s\n' "$mode" "${model:-default}" "$(grep -c '^# Tools' "$sysfile")" "${#delta}"; printf '%s\n---END-DELTA---\n' "$delta"; } >> "${FAKE_CLAUDE_LOG:-/dev/null}"
  usage
  if printf '%s' "$transcript" | grep -q "inference tool test"; then
    if ! printf '%s' "$transcript" | grep -q '^\[Tool result: search_tools\]'; then
      reply='<<TOOL_CALLS>>[{"name":"search_tools","arguments":{"query":"echo"}}]'
    elif ! printf '%s' "$transcript" | grep -q '^\[Tool result: call_tool\]'; then
      reply='<<TOOL_CALLS>>[{"name":"call_tool","arguments":{"name":"mcp__fake__echo","arguments":{"text":"hi"}}}]'
    else
      reply="inference done: $(printf '%s' "$transcript" | grep -A1 '^\[Tool result: call_tool\]' | tail -1)"
    fi
  else
    reply="inference answered"
  fi
  case "$reply" in '<<TOOL_CALLS>>'*) ;; *) emit_text "$reply";; esac
  sid="${sess_resume:-$sess_new}"; [ -n "$sid" ] && printf '[Loki]\n%s\n\n' "$reply" >> "$sdir/$sid"
  emit_result "$reply"
  exit 0
fi

# ---- delegated mode ----
printf 'FLAGS:%s\n' "$flags" >> "${FAKE_CLAUDE_LOG:-/dev/null}"
printf '%s\n---END-PROMPT---\n' "$prompt" >> "${FAKE_CLAUDE_LOG:-/dev/null}"
prompt=$(printf '%s' "$prompt" | tail -n1)
case "$prompt" in *"use a tool"*)
  echo '{"type":"assistant","message":{"content":[{"type":"tool_use","id":"tu1","name":"Read","input":{"file_path":"/x"}}]}}'
  echo '{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"tu1","content":"file body"}]}}';;
esac
case "$prompt" in *"be slow"*)
  emit_text "thinking slowly "
  sleep 60;;
esac
usage
emit_text "claude "; emit_text "answered"
emit_result "claude answered"
