#!/usr/bin/env bash
# One-time setup of Web Push for Loki on the server (the Loki server): a venv with pywebpush and a VAPID key
# pair. Safe to re-run (keeps existing keys).
set -euo pipefail
DATA="$HOME/.local/share/loki"; CONF="$HOME/.config/loki"
mkdir -p "$DATA" "$CONF"
[ -x "$DATA/push-venv/bin/python" ] || python3 -m venv "$DATA/push-venv"
"$DATA/push-venv/bin/pip" install --quiet --upgrade pip pywebpush
if [ ! -f "$CONF/vapid_private.pem" ]; then
  "$DATA/push-venv/bin/python" - "$CONF" <<'PY'
import sys, base64
from py_vapid import Vapid01
from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat
conf = sys.argv[1]
v = Vapid01(); v.generate_keys(); v.save_key(f"{conf}/vapid_private.pem")
pub = v.public_key.public_bytes(Encoding.X962, PublicFormat.UncompressedPoint)
open(f"{conf}/vapid_public.txt", "w").write(base64.urlsafe_b64encode(pub).rstrip(b"=").decode())
PY
  chmod 600 "$CONF/vapid_private.pem"
fi
echo "push ready: public key $(cut -c1-12 "$CONF/vapid_public.txt")…"
