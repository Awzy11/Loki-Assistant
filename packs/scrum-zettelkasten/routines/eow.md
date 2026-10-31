---
description: Close the week: review the sprint, update the estimation log, run the retro, and archive the week.
---
Close the week. Run it at the end of the working week, after the end-of-day routine. If the user passes a date as an argument, use it to identify the week; otherwise today's date comes from the conversation context (the `[Current date/time: …]` line).

## Steps

### 1. Read the week
Read the current sprint note, every daily note for the week, and `{{estimation_log}}` (step 4 needs it).

### 2. Scan for slip-throughs
In each daily note's `{{section.captures}}` and `{{section.end_of_day}}` sections, look for anything that was captured but never reached a backlog file, or that an end-of-day list moved to the backlog but has no file in `{{vault.backlog}}/`. Flag these before you go on.

### 3. Fill in the sprint review
Rewrite the sprint note in full, keeping the `{{week.backlog}}` section word for word (it is the historical record).
- **Completed in the sprint**: every checked item, with `est: Xh / actual: Yh` inline where it is known.
- **Completed outside the sprint**: emergent work confirmed done in the week's daily notes.
- **Returned to the backlog**: every unchecked sprint item that carries forward, as a `[[wiki link]]` with a short note on why and when it is next scheduled.

Update the progress line to `{{label.wip_active}}: 0/5 … {{label.wip_done}}: N`.

### 4. Update the estimation log
For each completed item whose backlog or archive file has both `estimate:` and `actual:`, add a row to `{{estimation_log}}` (a full rewrite): item, week, estimate, actual, ratio. Then recalculate **both** coefficients in the log's existing convention: total actual ÷ total estimate (for planning the sprint load) and the average of the individual ratios (for calibrating single tasks), and update the data-point count and the last-updated week.

If a completed item has no `actual:`, ask for it now, in one message. The log is only useful if it is complete.

### 5. Write the retrospective
Be honest, not diplomatic:
- **What worked**: behaviors of the system that helped execution
- **What didn't**: patterns that made items slip, stall or never start
- **One thing to change next sprint**: a single specific behavior. A commitment, not a wish. If it is a personal pattern rather than a pure process or data fix, make the desired behavior the path of least resistance with an easy opt-out instead of a rule ("always do X when Y"), which tends to backfire.
- **Estimation accuracy**: the average ratio from step 4. If it is consistently above 1.5×, call it a system problem, not a data point.

Also look at the capacity table (`{{label.available}}`, `{{label.committed}}`, `{{label.buffer}}`): was the buffer kept above 20%? If the sprint ran over, say how many hours were committed against how many were available.

### 6. Priorities check (interactive)
Read `{{priorities}}`. For each area in it, summarize what the sprint delivered and ask the user: did this sprint make meaningful progress here, and is it on track, at risk or stalled? Wait for the answers before moving on. A stalled area should show up in next week's goal.

### 7. Archive finished items
For each item confirmed done this sprint: confirm `actual:` is filled in, write the file to `{{archive.item}}` (same area and title as in the backlog) with `tags: [action, archived]`, the completion date, `Archived: YYYY-MM-DD` and this week's note in `Linked:`, **confirm the write succeeded**, and only then remove the original from the backlog.

For items returned to the backlog with a new date, add that date to their `Linked:` line. For events in `{{events}}` that happened this week and are confirmed done, remove them.

### 8. Ask for next week's goal and priorities
**Always ask; never assume.** Ask: (1) the sprint goal in one sentence: what does a successful next week look like? (2) the top three priorities: the outcomes that matter most, not a task list. If step 6 showed an area at risk, use that to frame the question.

### 9. Prepare the next sprint note
- Make sure next week's sprint note (`{{vault.weeks}}/YYYY-Www.md`) exists; if not, create it from `{{week_template}}`.
- Fill in `{{week.goal}}` with what the user gave you.
- Show the pre-built backlog and ask: anything to add, cut or reorder before Monday?
- Pull in backlog items whose `Linked:` line has a date in next week, and events from `{{events}}` that fall in it, under the right headings.
- Add the empty capacity table (`{{label.available}}`, `{{label.committed}}`, `{{label.buffer}}`) with the values left blank for the planning routine to fill.
- Add a one-line reminder to open Monday with the week-planning routine.
- If a location is set ({{location.name}}) and you have a weather tool, pre-fill the forecast in any daily notes you create (best effort, about five days ahead); skip it if the location reads "(not set)".

If next Monday falls in a new calendar month or quarter, add a monthly or quarterly review item to `{{vault.backlog}}/` and to next week's sprint so it goes through the normal planning. Skip this silently when neither boundary is crossed.

### 10. Archive the week
After the review is written and the next sprint is prepared, move the closed week into `{{vault.archive}}/Weekly/YYYY-Www/`. For the sprint note and for each daily note that exists for the week: read it, write it to its new path, confirm each write, and only then delete the original. Do the reads together, then the writes together, then the deletes. Wiki links to the moved notes keep working, so no link cleanup is needed. This is the last thing you do before reporting.

### 11. Report back
- The completed list, with estimate against actual
- Items returned to the backlog, and items archived
- The estimation log update: new data points and the new coefficients
- The week archived, and where
- Slip-throughs found
- The capacity assessment
- The priorities summary
- Whether a monthly or quarterly review was added
- The state of next week's sprint note, and any open questions

## Rules
- Prefer a full rewrite of a note when several sections change; use a frontmatter-only edit for single properties like `actual:`.
- **Archive moves**: write the new copy first, confirm it exists, and only then delete the original. Never delete before the copy is confirmed.
- Batch independent reads and writes together.
- Always ask for next week's goal and priorities, and for any missing actual before you touch the estimation log.
- This routine runs after the end-of-day routine and does not replace it.
- Completed items go to the archive, matching their backlog area; never leave them in the backlog.
- The estimation log is updated every week: it has no value if it is only updated sometimes.
- Archive the week last, just before you report.
