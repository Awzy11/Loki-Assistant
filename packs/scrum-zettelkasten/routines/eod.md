---
description: Close the day: fill in the end-of-day section, file captures, record actuals, and prepare tomorrow.
---
Close the day. If the user passes a date as an argument, use that day instead of today; otherwise today's date comes from the conversation context (the `[Current date/time: …]` line).

## Steps

### 1. Read the notes
Read today's and yesterday's daily notes together (`{{vault.daily}}/YYYY-MM-DD.md`).

### 2. Check yesterday for slip-throughs
Scan yesterday's `{{section.captures}}` and unchecked items. Flag anything that never made it into a backlog file (`{{vault.backlog}}/`) or the inbox (`{{vault.inbox}}/`).

### 3. Record actuals on completed backlog items
Before you write the Done list, gather every actual in one pass. For each completed item that has a backlog file with an `estimate:` and an empty `actual:`, ask for all of them **in one message** ("How long did each of these actually take? A, B, C"), not one question per item. Write each answer into that file's `actual:` field (a frontmatter edit). If the user does not know, write an approximate value with a tilde (`~2h`) rather than leaving it blank. This has to happen before the item is archived, because the actual is part of its record.

### 4. Fill in today's `{{section.end_of_day}}` section
Rewrite today's note in full, keeping the `{{section.captures}}` section word for word (it is the record).

- **Done**: every checked item, plus time-blocked events that clearly happened. For items with a backlog file that has an estimate, add `est: Xh / actual: Yh` inline.
- **Moved to backlog**: every unchecked item that carries forward, as a `[[wiki link]]`.
- **Notes**: context, carry-forwards, follow-ups.
- **`{{section.reflection}}`** (a `###` sub-section), in two passes:
  1. **Derive these yourself first; do not ask.** *Planned schedule* (the `{{section.focus_windows}}` table in order), *actual schedule* (reconstructed from checked and unchecked tasks and the notes), *variance* (what shifted, was added or was cut, and why), and *estimation accuracy* (from the actuals gathered in step 3). Write them straight in. Only ask if something is truly ambiguous.
  2. **Then ask the rest in ONE message.** Propose an answer for the adjustment so the user can confirm or override it rather than answering cold:
     - **Tomorrow**: one adjustment, proposed from today's variance
     - **Invisible work**: anything done that was not on the list
     - The properties below

{{properties:eod}}

After the reflection is written, set those properties in today's note with a frontmatter edit (the exact word for a choice, the plain number for a number). Properties the morning routine already set stay as they are.

### 5. Process every capture
For each item in the `{{section.captures}}` section:
- **New action item**: first search `{{vault.archive}}/` for it; if it was already completed, note that and create nothing. Otherwise create `{{backlog.item}}`.{{areas_note}} It gets `tags: [action]` and empty `estimate:` and `actual:`. Fill `next_action:` with one concrete first step if you know it, and `unlock:` if finishing it opens up something the user wants to do.
- **New knowledge**: create `{{vault.inbox}}/<Title>.md` with `tags: [knowledge]`.
- **Updates an existing backlog item**: rewrite that file with the new information merged in.
- **Date-specific event**: add it to `{{events}}` under the right month and date heading (and create or update that day's note stub). Do not make a backlog file for something that is tied to a date.

Every new or updated backlog or inbox file gets today's note in its `Linked:` line, for example `Linked: [[YYYY-MM-DD]]` (append if the line exists).

### 6. Update completed and changed backlog items
- Items updated but not finished: rewrite them with the new status and today's date in `Linked:`.
- Items confirmed finished: make sure `actual:` is filled in (step 3), write the file to `{{archive.item}}` (same area and title as in the backlog) with `tags: [action, archived]`, `Archived: YYYY-MM-DD` and this week's note in `Linked:`, **confirm that write succeeded**, and only then remove the original from the backlog.
- Events in `{{events}}` whose date has passed and are confirmed done: remove them.

### 7. Check sprint alignment (silent)
Read the sprint note if it is not already in context. Compare today's Done list with the sprint goal and active items: did today move any active item, is one going untouched mid-week, did the active count creep past the limit? Do not ask the user.

{{energy_note}}

Apply the same idea to how you report this check: on a normal day, put anything genuinely at risk in tomorrow's note with a brief flag; on a lower-energy day, state an untouched item plainly ("still open") with no tally or urgency, and do not flag a deliberate rest day or a deprioritized item at all.

### 8. Prepare tomorrow's note
- If tomorrow's note (`{{vault.daily}}/YYYY-MM-DD.md`) does not exist, create it from `{{daily_template}}`.
- Pull in backlog items whose `Linked:` line has tomorrow's date, anything in `{{events}}` dated tomorrow, and the active sprint items that need work.
- **Ask for tomorrow's `{{section.mits}}`** every time; they change daily.
- Suggest `{{section.focus_windows}}` for tomorrow from the active items and the fixed commitments already in the note, marked *suggested* (the morning routine confirms them).
- For tasks pulled from a backlog file, add the `next_action:` and `unlock:` lines as in the morning routine (`  - *Start: …*`, `  - *Unlocks: …*`).
- If a location is set ({{location.name}}) and you have a weather tool, fill tomorrow's `{{section.weather}}` section; skip it if the location reads "(not set)".

### 9. Report back
- Done list, with estimate against actual where there is one
- Items moved to the backlog, and items archived
- Actuals recorded
- New files created
- Slip-throughs found
- Sprint flags, if any
- Suggested focus windows for tomorrow
- Anything ambiguous you want the user to look at

## Rules
- Prefer a full rewrite of a note when several sections change; use a frontmatter-only edit for single properties like `actual:`.
- **Archive moves**: write the new copy first, confirm it exists, and only then delete the original. Never delete before the copy is confirmed.
- Batch independent reads and writes together.
- Always ask for tomorrow's priorities, and always ask for missing actuals (in one message). Never skip either.
- Completed items go to the archive, never back into the backlog.
- Capture dates belong in `Linked:` alongside scheduled dates.
- Never describe a quiet or lighter day as a failure.
