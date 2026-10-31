---
description: Monthly check-in: a quick status pass across the life areas in your priorities note.
---
Run the monthly check-in. Run it at the start of a new month. Today's date comes from the conversation context (the `[Current date/time: …]` line).

**Light touch, about 10 minutes.** This is a status pass, not a reset; the deep reset is the quarterly review. If it runs long or turns up an area that needs real rework, stop and flag that area for the next quarterly review instead of solving it here.

## Steps

### 1. Read the context
Read `{{priorities}}` and the previous month's note in `{{vault.months}}/` (if there is one).

### 2. Status-flag pass
The areas are the ones named in `{{priorities}}`. For each, give your own read first, based on roughly the last four weeks of daily and weekly notes, then ask the user to confirm or correct it. Propose, then verify; do not open each area with a blank "how is it going?".

Status key: **✅ on track · ⚠️ at risk (name the blocker) · ❌ dropped (say why explicitly) · 🔄 evolving (the goal itself is changing).**

### 3. What changed
Ask: **is there anything the priorities note does not reflect yet?** If so, record it in this month's note. Do not edit the priorities note at this cadence unless something in it is actively wrong and misleading; in that case fix that one thing and say so.

### 4. Write the note
Create `{{vault.months}}/YYYY-MM.md` from `{{vault.templates}}/Monthly Check-In.md` if that template exists; otherwise use a simple table of area, status flag and one line of why.

### 5. Report back
The status table, what changed, and anything flagged for the next quarterly review.

## Rules
- Keep it to about ten minutes. Running long is itself a signal: flag the area for the quarterly review.
- Every status must come from a real answer; never default one to ✅.
- Prefer a full rewrite of a note when several parts change.
- Describe what is, plainly. A ⚠️ or ❌ is information, not a failure.
