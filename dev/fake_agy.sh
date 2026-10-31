#!/bin/sh
# Test stand-in for `agy -p=<prompt> --output-format json [--model m]`.
prompt=""; model=""
while [ $# -gt 0 ]; do
  case "$1" in -p=*) prompt="${1#-p=}";; --model) model="$2"; shift;; esac
  shift
done
printf 'AGY model=%s cwd=%s chars=%s\n' "${model:-default}" "$(basename "$PWD")" "${#prompt}" >> "${FAKE_AGY_LOG:-/dev/null}"
printf '%s\n---END---\n' "$prompt" >> "${FAKE_AGY_LOG:-/dev/null}"
out() { printf '{"conversation_id":"c","status":"SUCCESS","response":%s,"num_turns":1,"usage":{"input_tokens":100,"output_tokens":5,"thinking_tokens":0}%s}\n' "$(python3 -c 'import json,sys;print(json.dumps(sys.argv[1]))' "$1")" "$2"; }
case "$prompt" in
  *"agy denied test"*)
    case "$prompt" in
      *"[Reminder]"*) out "recovered after reminder" "";;
      *) echo 'jetski: no output produced — a tool required the "write_file" permission' >&2; out "" ',"denied_actions":[{"action":"write_file","display_name":"WriteToFile"}]';;
    esac;;
  *"agy tool test"*)
    case "$prompt" in
      *"[Tool result: search_tools]"*) out "agy done: found tools" "";;
      *) out '<<TOOL_CALLS>>[{"name":"search_tools","arguments":{"query":"echo"}}]' "";;
    esac;;
  *) out "agy answered" "";;
esac
