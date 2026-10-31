---
description: Plan the week: confirm the sprint, set capacity, split active from queued work, and place it on the calendar.
---
Plan the week. Run it at the start of the week, after the end-of-week routine. Today's date comes from the conversation context (the `[Current date/time: …]` line).

This routine expects the current sprint note (`{{vault.weeks}}/YYYY-Www.md`) to exist with a goal, top priorities and a pre-built `{{week.backlog}}` section from last week's review.

## Steps

### 1. Read the context
Read these together:
- The current sprint note
- The daily notes for the week (`{{vault.daily}}/`), skipping missing dates silently
- Last week's retrospective, from the archive (`{{vault.archive}}/Weekly/YYYY-Www/`), especially its **one thing to change**
- `{{events}}`, for anything that falls this week

If the sprint note has no completed review section, the end-of-week routine was not run: say so and offer to run it first.

{{energy_note}}

If today's energy property reads low or lighter, say so plainly and offer a choice: defer the full planning, or do a light version (goal and top priorities only, no capacity math). It is a suggestion, not a block: do whatever the user actually wants.

### 2. Retro commitment check
State last sprint's "one thing to change" and ask: **is this sprint set up differently because of it?** This needs a real answer. If the same failure mode is already visible in the current setup, name it.

### 3. Weekend inbox triage
Scan `{{vault.inbox}}/` for items created since the end of last week. For each one:
- **Action item**: check `{{vault.archive}}/` first; if it is not already finished, create a backlog file at `{{backlog.item}}`.{{areas_note}} It gets `tags: [action]`, empty `estimate:` and `actual:`, and today's note in `Linked:`.
- **Knowledge**: leave it in the inbox and mention it in the report.
- **Time-sensitive**: pull it straight into the sprint.

### 4. Confirm the sprint backlog
Show the sprint goal, the top priorities and the full `{{week.backlog}}` list. Ask one question: **anything to add, cut or reorder before the week locks in?** Apply the changes, and remove the "open Monday with the planning routine" reminder line from the sprint note.

### 5. Capacity
Ask: **how many effective focused hours do you have this week?** (Not total hours: take out meetings, fixed commitments and the friction that disappears every day.)

Add up the `estimate:` of every sprint item. For any item without one, ask for a rough figure now and write it to its backlog file. Then calculate `Buffer % = (Available − Committed) ÷ Available × 100` and fill in the capacity table in the sprint note, using its existing row labels:

| {{label.available}} | Xh |
| {{label.committed}} | Yh |
| {{label.buffer}} | Zh (N%) |

If the buffer is under 20%, say so and ask which items to move to the `{{week.queue}}` list or cut. Do not carry on silently with an overloaded sprint.

### 6. Active and queued work
Ask which items are **active** this week (starting now) and which are **queued** (committed, waiting for a slot). Keep **five** items active at most across all areas; if the user names more, ask which to move to the queue.

Reorganize the sprint note so the active items sit under the **{{label.active_list}}:** marker and the rest under `{{week.queue}}`, and update the progress line to `{{label.wip_active}}: X/5 … {{label.wip_done}}: 0`.

### 7. Place the week
From the daily notes, map the week: fixed commitments (not movable) and open windows. For each active item ask **when** it happens this week, with a specific day and window. Propose slots for anything unplaced, and flag clashes early. Be clear in the note about which placements are fixed commitments and which are soft (protected, but movable if something real comes up). Include the events from `{{events}}`.

Only place an item at a fixed time when something external really fixes it (office hours, a callback window, a deadline). Otherwise keep it as a soft placement; forcing a rigid slot rarely helps.

### 8. Update the daily notes
For each confirmed time block, update that day's note: add it under the right task heading with its time range, fill in the `{{section.focus_windows}}` table, and update the stand-up summary. If a location is set ({{location.name}}) and you have a weather tool, pre-fill `{{section.weather}}` for days inside the forecast window; skip the weather if the location reads "(not set)". Make sure the first day of the week has a fully planned morning before you report. Rewrite only the notes that need changes.

### 9. Report out
- The retro commitment: acknowledged, or flagged
- Weekend inbox items processed
- The sprint goal
- Capacity: available, committed, buffer %, and a flag if under 20%
- Active items (X/5) and queued items
- The final sprint backlog
- The week map: fixed commitments plus placed focus windows by day
- Clashes, and any open questions

## Rules
- Prefer a full rewrite of a note when several sections change; use a frontmatter-only edit for a single property such as `estimate:`.
- The retro check (step 2) needs a real answer; do not treat it as rhetorical.
- Capacity (step 5) comes before the active/queued split: you cannot split without knowing whether the sprint is overloaded.
- Keep the active limit at five and explain why if the user pushes back; do not silently accept more.
- Every active item needs a specific day and window: "sometime this week" is not a plan.
- If the end-of-week routine was not run, flag it and offer to run it first rather than proceeding with a skeleton note.
- Never describe an overloaded or lighter week as a failure.
