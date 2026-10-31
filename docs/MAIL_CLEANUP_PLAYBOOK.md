# Mail cleanup playbook (for a local model)

Generic instructions. Contains no mailbox data. The personalised plan for each mailbox comes from the inbox review (`/mail/review`) and stays on the Loki server.

## Tools
- `mail_count(query, account)`: exact count. Always run first.
- `mail_label(account, query, expected_count, label)`: add a `Loki/…` label.
- `mail_mark_read(account, query, expected_count)`
- `mail_trash(account, query, expected_count)`: moves to Trash (recoverable 30 days). Starred, sent and draft mail are skipped automatically.
- `mail_draft_reply(account, id, body)`: saves a Gmail draft. It cannot send.
Each write call is confirmed by the user. One account per call; at most 1000 messages per call (default 500); the tool tells you how many still match.

## Rules that never change
1. Never send mail. Never permanently delete. Trash is the strongest action.
2. Count first, then pass that number as `expected_count`. If the tool says the count drifted more than 20%, stop and report; do not retry with a looser query.
3. Label before you act (`Loki/Junk`, `Loki/Stale`, `Loki/Action`, `Loki/Review`, `Loki/Keep`) so every change can be found and undone.
4. Queries must be narrow and name what they select: e.g. `in:inbox category:updates older_than:30d`, `from:newsletter@example.com`. Never trash by a bare word.
5. Summaries: describe, don't quote. No addresses, phone numbers, account numbers or amounts.
6. Work in order and report counts after each step. Plan your steps; ask the user to raise `/steps` if you need more.

## Classes
action (needs a reply or task) · review (look over, no reply) · fyi · receipt (keep) · junk (promotions, newsletters) · stale (old and time-sensitive, now expired) · unsure (leave alone).

## Steps
0. Read the mailbox's plan and confirm the account.
1. Junk senders from the plan: label `Loki/Junk`, count, mark read, then trash.
2. Stale messages: label `Loki/Stale`, count, mark read, then trash.
3. Action items: label `Loki/Action`, leave unread in the inbox.
4. Review items: label `Loki/Review`, leave unread.
5. Leave fyi, receipt and unsure alone.
6. Report counts per step and what was left.
