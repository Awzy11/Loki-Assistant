#!/usr/bin/env bash
# Installs a starter pack (packs/<name>) for the current user. It never overwrites anything you already have:
#   - profile.toml is copied to ~/.config/loki/profile.toml only if you don't have one;
#   - each routine prompt is copied to ~/.config/loki/commands/ only if no command of that name exists there or in ~/.claude/commands.
# Usage: ops/install-pack.sh <pack-name> [--dry-run]      e.g.  ops/install-pack.sh scrum-zettelkasten
set -euo pipefail

pack="${1:-}"; dry=0; [ "${2:-}" = "--dry-run" ] && dry=1
here="$(cd "$(dirname "$0")/.." && pwd)"
src="$here/packs/$pack"
if [ -z "$pack" ] || [ ! -d "$src" ]; then
  echo "usage: ops/install-pack.sh <pack-name> [--dry-run]"; echo "packs available: $(ls "$here/packs" 2>/dev/null | tr '\n' ' ')"; exit 2
fi

cfg="${XDG_CONFIG_HOME:-$HOME/.config}/loki"
cmds="$cfg/commands"
claude_cmds="$HOME/.claude/commands"
do_it() { if [ "$dry" = 1 ]; then :; else "$@"; fi; }
verb="installed"; [ "$dry" = 1 ] && verb="would install"

echo "Installing pack '$pack'$([ "$dry" = 1 ] && echo ' (dry run: nothing will be written)')"
do_it mkdir -p "$cfg" "$cmds"

if [ -f "$src/profile.toml" ]; then
  if [ -e "$cfg/profile.toml" ]; then
    echo "  kept your existing $cfg/profile.toml (the pack's version is $src/profile.toml if you want to compare)"
  else
    do_it install -m 600 "$src/profile.toml" "$cfg/profile.toml"; echo "  $verb profile -> $cfg/profile.toml"
  fi
fi

added=0; skipped=0
if [ -d "$src/routines" ]; then
  for f in "$src"/routines/*.md; do
    [ -e "$f" ] || continue
    name="$(basename "$f")"
    if [ -e "$cmds/$name" ] || [ -e "$claude_cmds/$name" ]; then
      echo "  skipped /${name%.md}: you already have a command with that name"; skipped=$((skipped+1))
    else
      do_it install -m 644 "$f" "$cmds/$name"; echo "  $verb /${name%.md}"; added=$((added+1))
    fi
  done
fi

echo "Done: $added routine(s) $([ "$dry" = 1 ] && echo would be installed || echo installed), $skipped skipped."
echo "Next: preview one with   loki-routine bod   (shows the prompt with your folder names filled in), then restart loki-server."
