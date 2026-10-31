---
description: Quarterly review: a deliberate reset of your priorities and long-term plan, area by area.
---
Run the quarterly review. Today's date comes from the conversation context (the `[Current date/time: …]` line). Quarters here are calendar quarters (Jan–Mar, Apr–Jun, Jul–Sep, Oct–Dec).

**A deliberate, protected session, about 45 to 60 minutes.** Do not run it distracted or squeezed into a spare ten minutes. If the user does not have real time now, offer to reschedule rather than rush.

## Steps

### 1. Read the context
Read `{{priorities}}`, `{{long_term_vision}}` (if it exists) and every note in `{{vault.months}}/` since the last quarterly review.

### 2. Area-by-area reset
For each area named in `{{priorities}}`, walk through it with the user:
- What actually shipped or changed this quarter
- Does the priorities note still reflect reality?
- Status flag: ✅ on track / ⚠️ at risk / ❌ dropped / 🔄 evolving

Pull from the monthly notes you read instead of starting from scratch; they are the input data for this review.

### 3. Knowledge and action alignment
Look at the permanent notes in `{{vault.notes}}/` modified since the last review (and `{{values}}` if it exists) against this quarter's backlog and sprint activity. Produce four short lists: **aligned** work, **orphaned actions** (work with no idea behind it, with a proposed next step each), **orphaned knowledge** (ideas with no action, with a candidate action each) and **one sharpest misalignment question**. Present them, get confirmation, and only then write any agreed `informed_by:` links in backlog frontmatter or `## Implications` sections in notes. Carry anything unresolved into step 2 rather than treating it as a separate track.

### 4. Check against the long-term plan
For each area, take its 5-year and 10-year statement from `{{long_term_vision}}` and ask: still true, or needs revising? If a statement is missing or clearly stale (the first run, usually), build it with the user now; do not leave placeholders in a document called a vision.

### 5. Update the priorities note
Rewrite `{{priorities}}` in full so it reflects current reality: no stale project names, no closed-out roles listed as active. It is a snapshot of now, not a log.

### 6. Update the long-term plan if it changed
Rewrite `{{long_term_vision}}` in full if any statement changed.

### 7. Write the quarterly note
Create `{{vault.quarters}}/YYYY-QN.md` from `{{vault.templates}}/Quarterly Review.md` if that template exists; otherwise a table of area, status and what changed. This is the historical record of the review; the priorities note stays a snapshot.

### 8. Report back
The area status table, what changed in the priorities note and the long-term plan, next quarter's north star per area, and the alignment results (counts, links written).

## Rules
- A deliberate session, not a quick pass: reschedule instead of rushing.
- The priorities note and the long-term plan are full rewrites.
- Links and `## Implications` from step 3 are written only after the user confirms.
- Calendar quarters only; do not mix in any fiscal-year convention.
- Describe what is, plainly. A dropped or at-risk area is information, not a failure.
