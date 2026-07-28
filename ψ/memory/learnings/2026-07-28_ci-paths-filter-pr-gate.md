---
pattern: Delivery verification records are executable evidence, and docs-only CI filters must fail closed on mixed pull requests.
date: 2026-07-28
source: rrr: deachawatss/maw-rs
concepts: [ci, verification, github-actions, delivery]
---

# CI path filters need executable evidence and a non-doc predicate

When a delivery tool replays successful verification commands, each recorded command must be a complete, non-interactive command rather than a descriptive test label. Validate the command before PR creation.

For a docs-only CI optimization, detect the presence of any non-documentation file with a fail-closed predicate. A positive documentation filter cannot distinguish a docs-only pull request from a mixed Markdown/source pull request; the latter must still run the full gate.

Author: Oracle (Codex L2)
