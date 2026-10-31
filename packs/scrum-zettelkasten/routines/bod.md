---
description: Start the day: read today's plan, check in, and lock a realistic schedule.
---
Start the day. Run it each morning before starting work: it reads today's plan, asks a few quick questions, and ends with a schedule you can actually follow.

Today's date comes from the conversation context (the `[Current date/time: …]` line). This routine expects today's daily note to exist already, because the evening routine prepares it. If it doesn't, create it from `{{daily_template}}` and carry on.

## Steps

### 1. Read the context
Read these together:
- Today's daily note: `{{vault.daily}}/YYYY-MM-DD.md`
- Yesterday's daily note, for the carry-forward check
- The current sprint note: `{{vault.weeks}}/YYYY-Www.md` (its goal, the `{{week.backlog}}` section and the progress line)
- `{{events}}`, for anything dated today

If yesterday's `{{section.end_of_day}}` section is empty, the evening routine was not run. Say so in the report and carry on; never block on it.

### 2. Carry-forward check
Compare yesterday's "moved to backlog" list with today's tasks. Anything that was carried forward but does not appear in today's note slipped through: add it to today's task list under the right heading before you go on.

### 3. Morning check-in
Ask these in ONE message. If the user skips an answer, leave that field as it is and move on; never block on a missing value.

{{properties:bod}}

Also ask which items in the `{{section.savers}}` checklist are already done, and tick them.

Write the answers into today's note (set the properties with a frontmatter edit, tick the checklist). Do not infer a value the user did not give.

{{energy_note}}

### 4. Work-in-progress check
Read the sprint's active items (the list under **{{label.active_list}}:** in the sprint note). For any item that has been active across three or more daily notes without being checked off, ask once: is it blocked, underestimated, or should it move to the `{{week.queue}}` list? Offer to keep it active, move it, or add a blocker note to its backlog file.

If the energy property says it is a low or lighter day, skip the day counts entirely. Mention a stuck item plainly ("still open") with no tally, and only start a conversation about it if the user raises it.

### 5. Confirm today's priorities
`{{section.mits}}` were set during the evening routine. Ask: **do these still hold, or has anything changed overnight?** If they change, note why; it is useful for the weekly retro.

### 6. Build the day
Make a map of the day from today's note:
- **Hard commitments**: anything with a fixed time. Not movable.
- **Open windows**: time nobody has claimed yet.

Fill in the `{{section.focus_windows}}` section. For each active sprint item ask **when** it happens today, not whether, and propose a window for any that is still unplaced. An active item with no time is an unscheduled item.

When you add an item that comes from a backlog file, read the file's `next_action:` and `unlock:` fields. If `next_action:` is set, add `  - *Start: <value>*` under the task; if `unlock:` is set, add `  - *Unlocks: <value>*`.

Say now which tasks will not realistically fit today and offer to move them to the backlog, rather than finding out tonight. Flag clashes (back-to-back commitments with no gap) at the same time.

If a location is set ({{location.name}}) and you have a weather tool, fill in the `{{section.weather}}` section with today's forecast. If the location reads "(not set)", skip the weather.

### 7. Update today's note
Rewrite today's note with everything confirmed: the priorities, the check-in answers, the checklist ticks, the focus windows, the carried-forward tasks, and a one-line "today" summary in the stand-up if the note has one. Change nothing else.

### 8. Report out
- Priorities locked (or changed, and why)
- Carry-forward items added, if any
- Stuck-item flags, if any (and only on a normal day)
- Today's schedule: hard commitments plus the focus windows
- Tasks moved off today
- Clashes, if any
- One line summing up the day, for example "Packed morning, open 1400–1600: the deep-work block fits."

## Rules
- Prefer one full rewrite of a note when several sections change; use a frontmatter-only edit for a single property. Never delete or overwrite anything you have not read.
- The check-in (step 3) and the priorities check (step 5) need a real answer from the user; do not assume them.
- Focus windows must be filled in before you report: a day without placed windows is a day planned to fail.
- Offer any lighter-day plan as a default with an easy way out. It is never a rule.
- Do not frame missed or moved tasks as urgent, at risk, or a failure. Describe what is, plainly.
