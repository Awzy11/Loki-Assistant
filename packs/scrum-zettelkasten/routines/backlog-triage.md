---
description: Triage the backlog: sort every item with Eisenhower then GAIN, flag what is stale, and propose what to archive.
---
Apply the Eisenhower → GAIN filter to the backlog. Today's date comes from the conversation context (the `[Current date/time: …]` line).

**Usage:** pass an area name to look at one area, or nothing for all areas. The areas are the subfolders of `{{vault.backlog}}/`.

## Steps

### 1. Scope
List the area folders in `{{vault.backlog}}/` and read every file in scope (all areas in parallel).

### 2. Eisenhower for each item
- **Q1 Do:** urgent and important (a deadline within two weeks, blocks other work, real consequences)
- **Q2 Schedule:** important, not urgent
- **Q3 Delegate:** urgent, not important; name the person or mechanism it should go to
- **Q4 Drop:** neither: stale, superseded, no longer relevant

An item with no update for six weeks and no date-anchored dependency is presumed Q4 unless its content says otherwise; flag it explicitly rather than silently assigning Q2. Read `{{priorities}}` for what currently matters.

### 3. Q4 dispositions
- Past event with no follow-up value → archive
- Superseded or no longer relevant → archive with a note saying why
- Never-was or a bad capture → propose deletion

Present the full list as a table and wait for confirmation before touching anything. The user may rescue items.

### 4. Q3 flags
List them separately, each with who or what it should be delegated to. Leave them in the backlog; the user decides.

### 5. GAIN for Q1 and Q2
**G** Give to AI (repetitive, rules-based, worth doing at scale: pass all three and it is a G) · **A** Accelerate with AI (research, analysis, drafting; the user directs) · **I** Integrate (needs judgment, relationships or presence) · **N** No AI.

### 6. Patterns
Name: G items that fit one 30 to 60 minute batch block; Q1 items not in the current sprint (they need a slot or a conscious deferral); items that keep reappearing and never get done (candidates for permanent deletion rather than another sprint slot).

### 7. Present it
One table per area: item, Eisenhower, GAIN, proposed action (keep, sprint slot, delegate to X, archive, delete). Then the proposed archives and deletions with reasons, the G batch list and the sprint additions. Ask: **confirm archives and deletions, rescue anything from Q4, and add anything I missed?**

### 8. Execute what is confirmed
- **Archive:** write the file to `{{archive.item}}` with `tags: [action, archived]`, `Completed:` and `Archived:` dates and the current week's note on the `Linked:` line; confirm the write; only then remove the original from the backlog.
- **Delete:** only items the user explicitly approved.
- **Sprint additions:** recommend them; do not edit the sprint note unless asked.

### 9. Log it
Append a `## Run - YYYY-MM-DD` section (areas covered, what was archived or deleted, one to three findings, the G batch list) to `{{vault.reference}}/System/Two-Matrix Task Filter.md`, creating it if needed.

## Rules
- Never archive or delete without explicit confirmation of the full list.
- Stale does not mean deleted: flag it, do not auto-remove it.
- The G batch list is the most useful output; always surface it.
- Prefer a full rewrite of a note when it changes a lot; use targeted edits for single sections.
- If an area is empty, say so and move on.
