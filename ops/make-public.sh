#!/usr/bin/env bash
# Build the scrubbed tree for the public repo: tracked files minus the private layer (ops/private-paths.txt), then a leak scan.
#   ops/make-public.sh <out-dir>     # out-dir must not exist; fresh tree, no git history
# Exit 1 if the scan finds anything from ops/leak-patterns.txt in the exported tree.
set -euo pipefail
OUT="${1:?usage: make-public.sh <out-dir>}"
[ -e "$OUT" ] && { echo "$OUT already exists" >&2; exit 2; }
cd "$(dirname "$0")/.."
mkdir -p "$OUT"
git ls-files -z | grep -zv -x -f <(sed -e 's/[][\.*^$]/\\&/g' -e 's/^/^/' -e 's/\/$/\//' ops/private-paths.txt | sed 's/$/.*/' | tr '\n' '\n') | xargs -0 -I{} cp --parents {} "$OUT"
cp -r ops/public-overrides/. "$OUT"/   # public stand-ins for files that hold private hosts
cd "$OUT"
if grep -rIn -E -f "$OLDPWD/ops/leak-patterns.txt" . --exclude-dir=node_modules --exclude=package-lock.json --exclude=Cargo.lock; then
  echo "LEAK SCAN: the lines above must be removed or moved to the private layer." >&2
  exit 1
fi
echo "clean: $(find . -type f | wc -l) files in $OUT"
