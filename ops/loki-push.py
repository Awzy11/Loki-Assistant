#!/usr/bin/env python3
"""Sends Web Push messages for Loki (run by loki-server inside the push venv).

stdin : {"subs": [{"endpoint": ..., "keys": {"p256dh": ..., "auth": ...}}], "payload": {...},
         "vapid_private": "/path/to/vapid_private.pem", "contact": "mailto:you@example.com"}
stdout: {"sent": N, "gone": [endpoints the push service says no longer exist], "errors": [...]}
"""
import json
import sys

from pywebpush import WebPushException, webpush


def main() -> int:
    req = json.load(sys.stdin)
    sent, gone, errors = 0, [], []
    for sub in req["subs"]:
        try:
            webpush(
                subscription_info=sub,
                data=json.dumps(req["payload"]),
                vapid_private_key=req["vapid_private"],
                vapid_claims={"sub": req.get("contact", "mailto:loki@localhost")},
                ttl=60 * 60 * 12,
                timeout=15,
            )
            sent += 1
        except WebPushException as e:
            status = getattr(getattr(e, "response", None), "status_code", None)
            if status in (404, 410):
                gone.append(sub["endpoint"])
            else:
                errors.append(f"{status}: {str(e)[:160]}")
        except Exception as e:  # network trouble etc.
            errors.append(str(e)[:160])
    print(json.dumps({"sent": sent, "gone": gone, "errors": errors}))
    return 0


if __name__ == "__main__":
    sys.exit(main())
