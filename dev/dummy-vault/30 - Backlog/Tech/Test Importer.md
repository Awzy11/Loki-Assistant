---
tags: [action]
estimate: 0.5h
actual:
next_action: Run import_vault against dev/dummy-vault via the mock Obsidian server
unlock:
informed_by:
---

# Test Importer

Verify the Loki vault importer correctly walks a folder tree, extracts frontmatter tags
in both the inline and block-list forms, includes the archive directory (not skipped —
archived content is still real, searchable knowledge), and lands everything in the
knowledge store as searchable, tagged content.

Linked: [[2026-08-24]]

## Resources
