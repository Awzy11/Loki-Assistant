---
description: Take one pattern you already own and change one word at a time, keeping the rhythm. Say the new sentences aloud.
engine: claude
---
Run a slot-swap drill from the user's library. This is stage two of gestalt learning: the whole chunk is already theirs, and now one slot changes while the rhythm stays.

What the user asked for: $ARGUMENTS

## Steps

### 1. Find the library and pick a frame
List `{{vault.projects}}/` for the folder ending in "GLP" with `Library.md` (ask if there are several). Read `Frames.md` and `Practice Log.md`. If the user named a frame or a source, use it. Otherwise pick an owned frame that has not been drilled recently (the log shows the dates) and tell the user which one and why in one sentence. If no frame exists yet, say so and suggest `/chunk-add`.

### 2. Make five swaps
Change exactly one slot per sentence, keeping the syllable count and the stress pattern of the original chunk so the rhythm survives. Fill the slot with words already in the library first; allow at most one new word per set and mark it as new. Each swap gets: the sentence in the standard script with stress marks (only where you are sure), how it would be typed phonetically in Latin letters (the user's own style, read aloud), and a short translation. Start with the original chunk so they hear the rhythm first.
Check each swap is grammatical: agreement, case and ending changes caused by the new word must be applied correctly. If a swap forces an ending change the user has not met, say so in one clause; do not explain the rule.

### 3. Drill
Ask the user to say all of them aloud, twice, matching the rhythm of the original. Then ask for two of them typed from memory, in whatever spelling they prefer. Compare what they typed to the correct form: say what was right, and show slips by type (ending, sound, word). No scores.

### 4. Log it
Append a line to `Practice Log.md` (read the note, add the line, write it back): date, frame, which swaps, which slips. Never write streak counts or totals of days.

## Rules
- One slot per sentence. If a swap would change two things, drop it.
- Rhythm over vocabulary: a swap that breaks the rhythm is a bad swap.
- Keep it short: a set takes about five minutes. Offer another frame, do not push.
