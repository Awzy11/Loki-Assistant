---
description: Mid-day energy and performance trend check against your recent baseline.
---
A trend check, not a snapshot: it combines your recent notes with a quick self-report into an honest, actionable read on where you are and what to do next. Run it any time. Today's date comes from the conversation context (the `[Current date/time: …]` line).

## Steps

### 1. Read the context
Read together: today's daily note, yesterday's, the day before's, and the same weekday last week (`{{vault.daily}}/YYYY-MM-DD.md`).

Older daily notes may have been archived with their week, so for any note not found in `{{vault.daily}}/`, look in `{{vault.archive}}/Weekly/YYYY-Www/YYYY-MM-DD.md` (ISO week number) before calling it missing. If notes are missing from both places, say so and carry on with what you have.

### 2. Self-report
Ask four things in a single message and wait for the answers:
1. **Current time** (you have no clock)
2. **Energy**, 1 to 5 (1 = running on fumes, 5 = locked in)
3. **Focus**: Sharp / Foggy / Scattered
4. **Body**: Fine / Tired / Wired / Sore

### 3. Read the signals
From each note, take what is there: the daily checklist items done (today), the top priorities (`{{section.mits}}`) done out of total, the count of checked tasks, how full `{{section.captures}}` is (empty / light 1 to 3 / active 4+), how substantive the end-of-day section is, and how packed `{{section.focus_windows}}` was. Keep this as input to your synthesis; do not print a table.

### 4. Trend
- **Linear (today against the prior two days):** improving, declining, stable or volatile, from priorities done, task volume and captures.
- **Same weekday last week:** above, below or consistent. This separates a structural pattern ("Thursdays are always heavy") from a real deviation.

Combine them into one label: normal for this weekday, below baseline, above baseline, declining, recovery or volatile.

### 5. Report (short enough to read in 30 seconds)
- **Energy read:** one sentence. The self-report is the anchor; the notes either confirm it or you name the mismatch (for example 4/5 energy with no priorities touched).
- **Trend:** the label plus one sentence.
- **What kind of work next:** sharp and high energy → deep work or hard decisions; foggy and medium → admin and routine; scattered → a break first, then low-stakes work; low across the board → triage what must happen today and what can move.
- **One honest flag, only if something stands out** (priorities untouched past mid-afternoon, three days below baseline with no cause, self-report that does not match the notes, no open windows left). Skip it otherwise.

### 6. Write it back
Append one line to today's `{{section.captures}}` section: `- **HAID HHMM**: Energy X/5 · Focus · Body · Trend: label`. This is the longitudinal record.

## Rules
- The self-report is the anchor: never override it with inference from the notes; name a mismatch instead.
- A trend needs at least two of the four notes. With only today's, say so and give a snapshot.
- Do not manufacture urgency. If things look fine, say so.
- The write-back always happens.
- Step 2 needs a real answer; do not assume one.
