# glp-language

Learn a language the way many people actually acquire one: whole phrases first (a line of a poem, a film quote, a song), kept with their rhythm, then mixed and matched, and only then taken apart. Four routines, and a library of your own phrases that lives in your vault as plain Markdown.

- `/chunk-add`: put phrases you remember or hear into the library. Type them any way you can (a language's own letters, or just how they sound in Latin letters); it writes the standard spelling next to what you typed, with a gloss and the pattern the phrase follows.
- `/chunk-swap`: take one pattern you already own and change one word at a time, keeping the rhythm, and say the new sentences aloud.
- `/chunk-recite`: say a source from memory; it shows exactly where you slipped and why, and updates what you own.
- `/chunk-lookup`: when a phrase puzzles you, why it works, in as little grammar as it takes.

No streaks, no daily targets: it keeps a plain practice log and never grades you.

## Set up
1. `ops/install-pack.sh glp-language` copies the routines into `~/.config/loki/commands/`.
2. In your vault, make a folder under your projects folder whose name ends in `GLP` (for example `Spanish GLP`) with a `Library.md` in it. The frontmatter says what you are learning:
   ```yaml
   ---
   language: Spanish
   script: Latin
   accept_phonetic_input: false   # true if you type Cyrillic, Greek, etc. as it sounds in Latin letters
   ---
   ```
   `/chunk-add` offers to create the rest (`Chunks/`, `Frames.md`, `Practice Log.md`) the first time you use it.
