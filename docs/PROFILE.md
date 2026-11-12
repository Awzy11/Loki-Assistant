# The profile: how Loki learns your vault

Loki does not assume a folder layout. Where your inbox, daily notes, backlog and archive live is described in one small file:

    ~/.config/loki/profile.toml        (or the path in $LOKI_PROFILE)

Every field is optional. Anything you leave out uses a **blank-vault default** (plain folder names: `Inbox`, `Daily`, `Weeks`, `Backlog`,
`Archive`, `Notes`, …). Folder names are relative to the vault root, with no leading or trailing slash.

## Starting points

| You want | Do this |
| --- | --- |
| A fresh, simple vault | No file needed. Loki uses the defaults above. |
| The Zettelkasten + Personal Scrum layout this project grew up with | Copy `packs/scrum-zettelkasten/profile.toml` to `~/.config/loki/profile.toml`. |
| Your own layout | Start from either, change the folder names. |

## What it controls today

```toml
[vault]
inbox = "..."     # raw captures; new notes Loki files for you land here
daily, weeks, months, quarters = "..."
backlog = "..."   # tagged action items waiting for a sprint
projects, notes, mocs, sources, reference = "..."
templates = "..."
archive = "..."   # finished work; Loki files closed backlog items here and never writes into it otherwise
agent = "..."     # your AI routines and run notes; Loki edits here only after you confirm

[backlog]
areas = ["Tech", "Home"]   # one subfolder per area under the backlog and the archive; leave empty for a flat backlog
```

Loki never writes on its own into `archive`, `templates`, `agent`, `daily` or `.obsidian`. Those protections follow your folder names.

## Safety

If `profile.toml` exists but is wrong (a folder that escapes the vault, two roles pointing at the same folder, a typo in a path you named
with `$LOKI_PROFILE`), the server **refuses to start** and says why. It will not guess and quietly write to the wrong place.

## Routines: your own prompts, with your vault filled in

A **routine** is a markdown file of instructions (`bod.md`, `eod.md`, …). You run it as a slash command (`/bod`) or from a button.
Loki looks for `<name>.md` in `~/.config/loki/commands/` first, then in `~/.claude/commands/`.

The files can contain placeholders. When a routine runs, Loki fills them in from your profile, so one prompt works with any vault layout:

| Placeholder | Becomes |
| --- | --- |
| `{{vault.daily}}`, `{{vault.weeks}}`, `{{vault.backlog}}`, `{{vault.inbox}}`, `{{vault.archive}}`, … | the folder names in `[vault]` |
| `{{events}}`, `{{priorities}}`, `{{estimation_log}}`, `{{daily_template}}`, `{{week_template}}` | those file paths |
| `{{section.mits}}`, `{{section.tasks}}`, `{{section.focus_windows}}`, `{{section.captures}}`, … | your daily-note headings |
| `{{week.goal}}`, `{{week.backlog}}`, `{{week.queue}}`, `{{week.done}}`, … | your weekly-note headings |
| `{{label.available}}`, `{{label.committed}}`, `{{label.buffer}}`, `{{label.active_list}}`, … | the weekly note's table and progress labels |
| `{{backlog.item}}`, `{{archive.item}}`, `{{areas_note}}`, `{{areas}}` | where backlog and archive files go (with or without area folders) |
| `{{properties:bod}}` | the list of properties the routine with that id asks about (`asked_by = "bod"`), with how to answer and where to save each |
| `{{energy_note}}` | what to do on a low-energy day, from your `role = "energy"` property (empty if you do not track one) |
| `{{location.name}}` | your location name, or "(not set)" |

Anything else in double braces is left exactly as written (for example `{{result:tool.field}}`, which pipes one tool's result into another).

**Asking for a stronger model:** a routine whose output a small local model gets wrong (writing a language it only half knows, for example) can say so in its front matter: `engine: claude` (or `engine: claude:opus`). When you start it, Loki switches that conversation to Claude and tells you, even if your default engine is a local model. It never overrides an engine you picked for that conversation or the `/exit` local-only switch, and it does nothing if Claude is not set up.

**See what Loki will send:** `loki-routine bod` prints the prompt with your profile applied (add `--blank` to see it against the blank-vault
defaults). Placeholders it does not recognize are listed at the end, which catches typos.

## Installing a starter pack

    ops/install-pack.sh scrum-zettelkasten            # add --dry-run to see what it would do

It copies the pack's `profile.toml` only if you do not have one, and each routine only if no command of that name exists yet, in either
commands folder. It never overwrites your files.

The starter pack's generic BOD, EOD, BOW and EOW are in `packs/scrum-zettelkasten/routines/`. They carry no personal rules: add yours
(or ask your AI to) by editing your copy under `~/.config/loki/commands/`.

## Status

The vault layout, the daily-note properties, the note headings and labels, and the routine registry all come from the profile now.
Still fixed in code: the events file's layout (`## YYYY-MM` / `### YYYY-MM-DD — Day`), the backlog frontmatter fields (`estimate`, `actual`,
`next_action`, `unlock`, `Linked:`), and the weather source. Those become configurable next.

## `[person]` and `[location]`

```toml
[person]
name = "Sam"                      # used when a reply is drafted in your voice
about = "a nurse with two kids"   # one sentence of context for the mail-sorting and date-finding prompts
email = "sam@example.org"         # optional contact for web-push; falls back to the project page

[location]
name = "Springfield"
latitude = 39.80
longitude = -89.64                # weather appears only when both coordinates are set
```
Leave them out and Loki says "the user", shows no weather and sends nothing personal to any prompt.
