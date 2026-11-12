---
description: Say a source from memory and see exactly where you slipped and why; updates what you own.
engine: claude
---
Check a recitation against the library. The user types or pastes what they remember of a source, in any spelling; you compare it with the stored text and show the slips.

What the user asked for: $ARGUMENTS

## Steps

### 1. Find the source
List `{{vault.projects}}/` for the folder ending in "GLP" with `Library.md` (ask if there are several). Read `Library.md` and the chunk note for the source named in the request (ask which if it is unclear or if several match). If the source is not in the library, say so and suggest `/chunk-add`.

### 2. Get the recitation
Ask the user to write out what they remember, line by line, without looking. They may use phonetic Latin letters. Do not show the stored text first. If they paste it all at once, use that.

### 3. Compare
Normalize their version to the standard spelling (the same rules as `/chunk-add`: phonetic spelling follows sound). Compare line by line with the stored text and sort every difference by type:
- **ending or form** (case, gender, number, tense),
- **sound or diminutive** (a dropped or changed sound, a diminutive turned into the plain word),
- **small word** (and/but, a particle, a preposition),
- **different or missing word**,
- **missing line**.
Show a table: line, their version, standard, type of slip. Say what was right first. For each slip, give one clause on why it matters, and where it helps, the rhyme or rhythm cue that would have caught it. Do not reprint the whole source unless the user asks; show only the lines with slips.

### 4. Update the library
Read `Chunks/<Source>.md`, update the status column, and write it back: exact recall becomes `owned`, recall with a slip becomes `shaky` (name the slip in a notes column), a missing line stays or becomes `heard`. Update the owned count for the source in `Library.md`. Append one line to `Practice Log.md`: date, source, how many lines exact, which slip types. Read each note back to verify.

### 5. Report
A short summary: how many lines exact, the slip types, and one suggestion (a `/chunk-swap` on a frame from this source, or `/chunk-lookup` on a slipped phrase).

## Rules
- A slip is information, not a failure. No scores, no streaks, no comparison with earlier days.
- Do not give the answer before they have tried; do give the correct line right after the comparison.
