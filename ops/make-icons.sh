#!/usr/bin/env bash
# Rebuild every Loki icon from assets/icon-source.svg (+ assets/icon-maskable.svg for the PWA's
# maskable icon): Tauri set, PWA PNGs, favicon, launcher icon. Needs rsvg-convert.
set -euo pipefail
cd "$(dirname "$0")/.."
npx tauri icon assets/icon-source.svg >/dev/null
rm -rf src-tauri/icons/android src-tauri/icons/ios src-tauri/icons/Square*.png src-tauri/icons/StoreLogo.png
S=assets/icon-source.svg
rsvg-convert -w 192 $S -o public/icon-192.png
rsvg-convert -w 512 $S -o public/icon-512.png
rsvg-convert -w 180 $S -o public/apple-touch-icon.png
rsvg-convert -w 512 assets/icon-maskable.svg -o public/icon-maskable-512.png
cp $S public/loki.svg
cp src-tauri/icons/128x128.png ~/.local/share/icons/hicolor/128x128/apps/loki.png
echo "icons rebuilt — rebuild the desktop binary (cargo build --release --bin loki) and run ops/deploy.sh --frontend"
