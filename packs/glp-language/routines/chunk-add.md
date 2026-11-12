---
description: Add phrases you remember or hear to your gestalt-learning library, with standard spelling, gloss and the pattern they follow.
engine: claude
---
Add new chunks to the user's language library. A chunk is a whole phrase kept with its rhythm: a line of a poem, a film quote, a sentence from a song or a show. Keep chunks whole. Do not split them into words, and do not explain grammar, unless the user asks (that is `/chunk-lookup`).

What the user gave: $ARGUMENTS

If that is empty, ask for the phrase or phrases and where they come from (a poem, a film scene, a show, a conversation). Today's date is in the context.

## Steps

### 1. Find the library
List `{{vault.projects}}/` and look for a folder whose name ends in "GLP" that contains `Library.md`. Read `Library.md`: its frontmatter says the language, the script, and whether phonetic Latin input is accepted. If there are several libraries, ask which one. If there is none, say so, propose the folder and files you would create, and wait for a yes.

### 2. Normalize what was typed
People type the way they say it, often in Latin letters ("fperedi byla tolka smyert"). Convert each phrase to the standard spelling of the language, then show the user's version and the standard form side by side. Mark differences plainly, without scolding: a wrong ending, a dropped sound, a swapped word, a word that was said but spelled by ear.
- Phonetic spelling follows sound, not spelling; read it that way ("fperedi" is впереди).
- For Russian: zh ж, kh х, ts ц, ch ч, sh ш, shch or sch щ, ya я, ye е, yo ё, yu ю, y or yi ы (or the ending ый/ий), a soft sign is usually not written; ё is often typed yo (or just e).
- Put an acute accent on the stressed vowel of every word with more than one syllable when you are sure. If you are not sure of a stress, leave that word unmarked and say so. Never guess silently.
- If a word is ambiguous (two standard readings), ask one short question rather than choosing.

### 3. Gloss
For each chunk give a natural translation and, only if it helps, a very short word-for-word line. Note the syllable count when the chunk has a clear rhythm. If you are unsure of a meaning, say "unsure", not a confident guess.

### 4. Find the frame
If the chunk has a reusable shape (for example "Behind ___ was ___, ahead only ___"), name the frame and its slots. Read `Frames.md` and extend an existing frame when it fits; do not invent a frame for a phrase that has no pattern.

### 5. Show, then write
Show a compact table: standard form with stress, the user's version if different, gloss, frame, status. Then write, one note at a time:
- `Chunks/<Source>.md` in the library folder (create it if the source is new): one table row per chunk with text, gloss, frame, status, date added. Statuses: `heard` (copied from a source), `shaky` (said from memory with a slip), `owned` (said from memory exactly). A chunk the user typed from memory is `owned` or `shaky`; a chunk they pasted from a source is `heard`.
- `Frames.md`: new or extended frames with example fillers.
- `Library.md`: the source index (one line per source: link, what it is, how many chunks, how many owned).
Existing notes: read the whole note, change the text, write the whole note back, read it back to verify. New notes: create them whole.

### 6. Report
One line per chunk added, then one suggestion for what to do next (say them aloud, `/chunk-swap` on a frame, or `/chunk-recite` on the source).

## Rules
- Keep chunks whole and keep the user's own version visible; the slip is information, not an error to hide.
- Never grade the user, count days or mention streaks.
- Reading and glossing a long list is bulk work: hand it to `delegate_local` if it is long, then check the result yourself.
- Do not invent a translation, a stress or a source. Say what you are unsure of.
