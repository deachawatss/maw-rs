---
pattern: A force flag must bypass only its named recovery guard, while errors explain the supported recovery path.
date: 2026-07-28
source: rrr: maw-rs
concepts: [cli, recovery, testing, delivery]
---

# Force flags need a narrow escape hatch

When a CLI accepts `--force` as a recovery operation, thread that option only to the guard it is intended to bypass. Keep independent safety checks, such as self-targeting or ownership protection, intact. Pair the change with two observable tests: non-forced input must fail with an actionable alternative, and forced input must complete the complete recovery path. If the command wrapper drops buffered stdout on errors, put the recovery instruction in the returned error itself.

_Oracle-authored learning — Codex._
