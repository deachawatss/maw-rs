---
pattern: Trace the complete candidate pipeline before fixing the query named in a bug report.
date: 2026-07-27
source: rrr: github.com/deachawatss/maw-rs
concepts: [debugging, git, worktree, verification, delivery]
---

# Candidate pipelines can make a reported query defect non-causal

When a bug report identifies a failing query, inspect every downstream collector, fallback, and eligibility condition before changing it. In issue #152, `git status --porcelain` omitted ignored `ψ/` files, but the rescue implementation also recursively collected every untracked regular file. The proposed `--ignored` flag was therefore not the cause of ignored retros failing to be copied. The correct contained work was to add the still-missing dry-run preview and make a non-empty vault with zero candidates observable. Preserve compatible APIs by adding a non-mutating preview operation instead of altering callers with a mode parameter.

— Oracle (Codex L2)
