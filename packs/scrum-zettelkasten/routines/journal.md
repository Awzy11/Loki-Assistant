---
description: Journal: save a freeform entry exactly as written, then surface ideas worth keeping and things you found you want.
---
Open a journal entry, receive freeform input, save it, and then offer to develop anything worth keeping.

## Steps

### 1. Gather the entry context
Ask three things in one message:
> What's the **date** for this entry? (default: today)
> What **time** is it? (default: now)
> What's the **setting**? (for example "morning coffee", "after the run")

Wait for the answer.

### 2. Confirm you are ready
Reply with exactly one line, then wait without prompting, summarizing or responding until the user says they are finished ("done", "that's it", "end"):
> Ready. Go ahead; write as much or as little as you want. Tell me when you're done.

### 3. Save the entry
- Filename: `YYYY-MM-DD HHMM - setting.md` (24-hour time, no separator), in `{{vault.journal}}/`.
- Frontmatter: `date`, `time`, `setting`, `tags: [journal]`.
- Body: the entry **exactly as written**: no edits, no paraphrase, typos and fragments included.

### 4. Offer permanent-note candidates
Look for atomic ideas the user articulated: realizations, principles, decisions, beliefs, connections. If you find any, list them as a numbered list with a short title and one sentence each, and ask whether to create stubs in `{{vault.notes}}/`. If there are none, say "No permanent note candidates found in this entry." Wait before creating anything.

### 5. Offer "found wants"
A found want is something the user discovered they actually want, as opposed to a goal set on purpose: things described as freeing or deeply satisfying, absences that hurt, recurring images that point at a value, statements about what is non-negotiable. List candidates the same way and ask whether to add any to `{{values}}`. If there are none, say so.

### 6. Write what was approved
- For each approved found want, append a section to `{{values}}` (a heading, a `Surfaced: [[…entry…]]` line and two or three sentences in the user's own voice) and refresh its `updated:` date.
- For each approved idea, create one stub in `{{vault.notes}}/` (one idea per stub), with `tags: [permanent-note, stub]`, a `source:` link to the entry, the date, a one-sentence seed and the line `*Stub: develop further.*`.
- Finish with one line saying how many of each were written.

## Rules
- Never edit the entry text.
- Never reflect or summarize before the user says they are finished.
- Notes and found wants are suggestions; always ask first.
- The journal folder holds raw entries only; processing happens in the notes folder.
