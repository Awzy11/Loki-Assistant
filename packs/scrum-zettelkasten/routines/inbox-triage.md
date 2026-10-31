---
description: Clear the inbox: sort every captured file into the backlog, your notes, a project or the bin, using two quick filters.
---
Clear `{{vault.inbox}}/` by routing every file to its permanent home with two filters: Eisenhower (how urgent and important), then GAIN (who should do it, you or AI). Today's date comes from the conversation context (the `[Current date/time: …]` line).

## Steps

### 1. Read the inbox
List `{{vault.inbox}}/`. If it is empty, say so and stop. Batch-read every file and note its frontmatter tags (`[action]`, `[knowledge]`, both, or none). If a `_README.md` is there, read it for local rules but never route, edit or delete it.

### 2. Sort by tag
- `[action]` → the backlog (`{{vault.backlog}}/`)
- `[knowledge]` → permanent notes (`{{vault.notes}}/`), literature notes (`{{vault.sources}}/`), a project (`{{vault.projects}}/`) or reference (`{{vault.reference}}/`)
- both → a backlog file **and** a notes or reference file
- no tags → decide from the content
- empty → delete (confirm only if you have any doubt)

### 3. Eisenhower on action items
Give each `[action]` file a quadrant:
- **Q1 Do:** urgent and important (hard deadline within two weeks, blocks other work, real consequences)
- **Q2 Schedule:** important, not urgent (belongs in a sprint)
- **Q3 Delegate:** urgent, not important (time-sensitive but not the user's to own; can be assigned or automated)
- **Q4 Drop:** neither (stale, superseded, not worth a slot)

Items with no urgency and no update for six weeks or more lean Q4; say so explicitly before proposing to delete them. Read `{{priorities}}` for what the user currently cares about.

### 4. GAIN on Q1 and Q2
- **G Give to AI:** repetitive, rules-based, worth doing at scale; AI does it, the user reviews. Name the specific action.
- **A Accelerate:** hard research, analysis or drafting; the user gives direction, AI does the heavy lifting.
- **I Integrate:** needs the user's judgment, relationships or presence; AI assists, the user leads.
- **N No AI:** physical presence, personal relationships, lived experience.

### 5. Route knowledge items
Pick the home that fits: a permanent note (stated assertively, one idea, for example "Walking Reduces Restlessness"), a literature note, a project folder, or reference for data and logs. A multi-idea planning document can be promoted as-is with a descriptive title and a note that it can be split later.

### 6. Pick the backlog area
{{areas_note}}

### 7. Show the plan before doing anything
A table of file, tags, Eisenhower, GAIN and destination. Call out: Q1 items (confirm they will be in the next sprint), G items (candidates for one batch block), every deletion, and anything ambiguous. Ask: **anything to reroute, rescue or clarify before I execute?**

### 8. Execute what is confirmed
- **Action items:** create `{{backlog.item}}` with `tags: [action]`, the content restructured under a clean `## Action Items` checklist, a `Linked:` line (keep existing dates, add today for a new capture) and a `## Resources` section if there were links. Then remove the inbox original.
- **Knowledge items:** create the destination file with sensible frontmatter (tags, source, created) and a `## Related` section linking only to notes that exist. Then remove the original.
- **Both:** create both files first, then remove the original.
- **Deletions:** only what the user approved; empty files may go silently.

Write everything first and delete afterwards. Never delete an original before its copy is confirmed.

### 9. Log it
Append an "Inbox Triage" section (date, files processed, the routing table, two to four observed patterns) to `{{vault.mocs}}/Two-Matrix Triage - YYYY-MM-DD.md`, creating the note if needed.

### 10. Report back
Files processed, notes created (with destinations), backlog items created (with areas), files deleted, what remains in the inbox and why, Q1 flags, the G batch list, and open questions.

## Rules
- Never delete a file that has content without the user's confirmation.
- The inbox is a holding zone, not storage; the goal is zero files left. Flag anything older than 48 hours, and anything older than a week more firmly.
- Preserve what the user captured: restructure lightly, do not summarize it away.
- Always surface the G batch list, even if it cannot be acted on today.
- Batch independent reads together, then writes, then deletes.
