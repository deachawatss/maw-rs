## Design contract (authored by L1 — commit this section verbatim as `specs/55-l1-notify-ack.md`)

**`notified=true` must mean "L1 acknowledged," established by exactly ONE authority (maw-rs), never by a fire-and-forget send.** "Acknowledged" is defined by reconciling against GitHub PR state — the same model maw-js used:

- On `maw pr`: enqueue a durable `pending` entry in `~/.maw/pr-queue.jsonl` (maw-rs remains the single locked writer). Attempt the L1 notify, but do NOT write `.maw/delivery-notified` / `notified=true` purely on send-keys exit 0.
- A **reconciler** (ported from maw-js) reconciles pending entries against GitHub via `gh pr view <n>`:
  - PR **merged/closed** → entry is done; drop it (archive to `pr-queue.jsonl.archived`).
  - PR **still open** → re-surface / re-notify L1. A busy pane therefore **delays** the ping, never **drops** it.
- The reconciler is invoked on a defined trigger (a `maw pr --reconcile` / `maw pr reconcile` verb is acceptable, and/or on `maw pr` start). Propose the exact trigger in the PR handoff.
