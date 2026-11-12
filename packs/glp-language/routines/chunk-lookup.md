---
description: Why a phrase works, in as little grammar as it takes. Tied to a phrase you already own.
engine: claude
---
Explain one chunk the user already knows: why it is built the way it is. This is stage four, grammar as a lookup, never as a drill. Keep it short.

What the user asked about: $ARGUMENTS

## Steps

### 1. Find the chunk
List `{{vault.projects}}/` for the folder ending in "GLP" with `Library.md` (ask if there are several). Search the chunk notes in `Chunks/` for the phrase or word the user named. If it is not in the library, offer to add it with `/chunk-add` first, then explain it.

### 2. Explain with the least grammar that does the job
Say what the user asked about and nothing more. Use at most four short points: what the forms are (case, gender, number, tense or aspect as relevant), why this form here, and what would change if one slot changed. Use the user's other chunks as examples wherever a pattern recurs: "the same ending is in ___ and ___". Prefer showing a pattern over naming a rule. Mark anything archaic, poetic or irregular as that.
If a piece is uncertain, say so; do not state a rule you are not sure of.

### 3. Close the loop
Write `analyzed` into the grammar column of that chunk's row (read the note, change it, write it back, read it back). Do not change its ownership status.

### 4. Report
The explanation, followed by one line: the chunk is now marked as looked up, and what to say aloud next.

## Rules
- Short beats complete. If the user wants more, they will ask.
- Never turn this into a lesson, a quiz or a list of rules to memorize.
