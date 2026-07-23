# SPEC-99: maw-owned L2 lifecycle observer + canonical handoff emit

Issue: #99
Date: 2026-07-23
Mode: standard
Author: Gale (Oracle L1) — synthesized from Codex L2 discovery FINDINGS (2026-07-23)

## Objective

The L2→L1 handoff has failed the same way four times because the handoff message is
**emitted, delivered, and observed by the agent, not by maw-rs**. Two coupled defects:

- **Emit drift:** each L2 hand-constructs the `[oracle:repo]` bracket + signed body, so the
  format drifts and is hand-repaired every delivery (NWFTH: 3 friction rows / 4 sessions).
- **Observe gap:** notification depends on the L2 *pushing* via `maw hey` (tmux send-keys),
  which races with L1 input and cannot fire for the stuck/blocked/error/idle cases.

Success: maw-rs **owns** both surfaces. One formatter produces every handoff literal; one
per-L2 observer detects every terminal state and delivers through a durable queue the L1
already drains reliably — with **zero dependency on the L2 running `maw hey` or formatting a
message by hand.** The error class becomes structurally impossible, not patched again.

L2 discovery verdict (accepted): *"maw-rs has partial ingredients but no owned lifecycle state
machine. workon records parent/pane metadata; activity classifies unchanged/prompt output;
notify persists inbox messages; the PR path persists queue rows. None connects spawn → observe
→ deduplicate → durable L1 consumption, and the current PR re-surface still invokes maw hey."*

## Acceptance Criteria

- [ ] Dispatch an L2 that (a) hands off FINDINGS, (b) errors/exits, (c) idles at a prompt →
      in **all three** the parent L1 receives exactly **one** reliable notification, with **no**
      `maw hey` call and no send-keys by the L2.
- [ ] Notification survives the L1 being mid-turn or the human typing concurrently (no send-keys
      race) — it surfaces on the next SessionStart/UserPromptSubmit drain.
- [ ] The canonical handoff literal is produced by a **single** maw-rs formatter; no caller
      hand-builds the bracket or the `— Oracle-authored (…)` signature. A golden test pins each
      literal (regression guard against future silent drift).
- [ ] `maw wake <agent> --parent-session-id <id>` **succeeds** and records parent metadata
      (currently exits "unknown argument"). Proven by a red→green test on that exact command.
- [ ] Works for both `maw workon` (single L2) and `maw team`/swarm (member spawn).
- [ ] Scoped crate tests green locally (`cargo test -p maw-cli` plus any other crate the diff
      touches); the full `cargo test --workspace` cross-crate matrix is proven in **CI**
      (`ci.yml`), never on the dev machine — per maw-rs `AGENTS.md` (workspace build/test is
      CI/post-merge only; trimmed per-crate gates miss cross-crate integration tests, so CI owns
      that coverage — see maw-rs #61→#64, #69→#70).

## Seams and Testing

- **Formatter** `format_l2_handoff(kind, l1_oracle, repo, issue, mode, risk, body, engine)` —
  the one producer of the literal. Unit + golden tests.
- **Observer classifier** — pure fn: (pane snapshot sequence, config) → `Option<TerminalState>`.
  Table-test the precedence + idle timeout with scripted snapshots (no live tmux).
- **Durable queue** `~/.maw/l2-events.jsonl` (+`.archived`) — mirror the PR-queue seam in
  `crates/maw-cli/src/core_impl/github_pull_request.rs`: `PrQueueLock`, `pr_read_queue_lines`/
  `pr_write_queue_lines`, `notified`/`notified_at`, archive-on-drain.
- **Drain** — SessionStart + UserPromptSubmit banner (the same reliable path that surfaces the
  PR queue; recap showed it firing at this session's start).
- **Parent seam** — one parse+resolve+persist module; the four current parsers converge on it.
- Prior art: PR-queue reconcile (`github_pull_request.rs`); issue #24 red-test pattern (notified
  terminal row + variant key); activity classifier (`activity_core.rs`); `.maw/delivery.json` +
  `.maw/delivery-notified` in `crates/maw-cli/src/wind/workon.rs`.
- Expected values: golden literal per kind; repro `maw wake coder --parent-session-id parent-1`
  red→green; scripted idle/exit/PR pane emits exactly one row per transition with correct state.

## Decisions

### D1 — Watcher host: per-L2 detached observer, armed at spawn
- Chose: `maw workon` and `maw team`/swarm spawn a **per-L2 detached observer** (a hidden
  `maw __watch <pane>`-style sidecar) keyed to the L2's tmux pane, torn down by the **same
  reaper** that reaps the pane (extend the 7adabab one-pane teardown scope).
- Why: matches the existing per-pane lifecycle maw already owns (pane create + `.maw/delivery.json`
  + reaper); dies with its L2; no new supervised long-lived daemon; zero dependency on L1
  attentiveness (the interim mitigation) or L2 self-report (the recurring bug).
- Rejected: a single long-lived maw daemon watching all panes (new supervised process, must
  discover panes, single point of failure — more infra than the fix needs); L1-session-armed
  watcher (that is the interim workaround, explicitly non-structural).

### D2 — Idle timeout & terminal-state precedence
- Chose: observer polls its pane every ~25s (config `l2_watch_interval_secs`, default 25) and
  classifies via the existing activity classifier. Terminal states, highest precedence first —
  emit exactly one event on each *new* transition to a higher/different state:
  1. `exited` — pane process died / shell returned
  2. `error` — hard error/panic signature
  3. `pr` — a PR opened for this pane (`maw pr` / PR URL)
  4. `findings` / `ready` — explicit handoff signature
  5. `blocked` — at an interactive approval/trust prompt
  6. `idle` — activity-classified "unchanged + at-prompt" for ≥ `l2_idle_timeout_secs`
     (default **180s**) — the last-resort net for the silent-stuck case signatures miss.
- Why: a dead/errored pane is the most urgent and most damaging if missed; `idle` is the catch-all
  that closes the exact gap #99 names. One-event-per-transition (observer tracks last-emitted
  state) prevents spam.
- Rejected: idle-only (misses fast PR/error); signature-only (misses silent stall — the actual
  historical failure).

### D3 — Durable queue, drain, and dedupe
- Chose: `~/.maw/l2-events.jsonl` modeled on `pr-queue.jsonl` — same `PrQueueLock` file-lock,
  same read/write helpers, `notified`/`notified_at`, archive-on-consume. Row:
  `{version, l2Pane, l2Session, l1Oracle, l1Session, repo, issue, state, transitionSeq,
  message, notified, notifiedAt, createdAt}`.
- **Dedupe key = `(l2Session, state, transitionSeq)`** — a stable identity, **never the rendered
  message** (issue #24: a presentation string is not a stable queue key). `transitionSeq` is a
  monotonic per-pane counter so a re-entered state is a new event but a re-polled same transition
  is idempotent.
- Drain: at SessionStart + UserPromptSubmit the L1 loads **every** unnotified row targeting its
  `l1Session`/`l1Oracle` (including pre-existing `open` rows, not only newly-appended — 2026-07-11
  durable-queue reconciliation learning), surfaces them, sets `notified=true`, archives.
- Why: reuses the one delivery channel proven to reach the L1 (the PR-queue banner), and the two
  durable-queue learnings that already cost real bugs.
- Rejected: send-keys delivery (the racy path being removed); DB (`pr-queue.jsonl` file model is
  the established, tested pattern).

### D4 — Canonical signed literal (the emit fix)
- Chose: one formatter, single producer, canonical form (minimal churn from current usage):
  `[<l1_oracle>:<repo>] <KIND> issue #<N> (<mode>/<risk>): <body> — Oracle-authored (<engine> L2)`
  where `<KIND>` ∈ {FINDINGS, READY, BLOCKED, ERROR, IDLE, EXITED}. Every emit path (workon
  handoff, team spawn, PR handoff, observer events) calls it; no hand-construction anywhere.
  A **golden test** pins each kind's literal.
- Why: one producer ⇒ the format cannot drift; the golden test makes any future change a
  deliberate, reviewed edit.
- Note: handoff bodies are delivered via the durable file queue, not tmux send-keys, so they may
  contain em-dashes/Unicode freely. The ASCII-only constraint applies **only** to
  `maw workon --prompt` (send-keys), never to handoff message bodies.

### D5 — One shared workon/team parent seam (fixes the repro)
- Chose: a single parent-metadata module (parse + resolve + persist). It accepts the full flag
  set (`--parent-session-id`, `--parent`, `--session-id`), resolves parent identity
  (session, pane, l1_oracle, repo) via one function (generalize
  `new_resolve_parent_session_id`), and writes `.maw/l2-meta.json` for the observer to read.
  `maw workon`, `maw wake`/`awaken`, `maw team`/swarm, and `maw bud` all route through it.
- Why: the L2 found `--parent-session-id` handled by four divergent parsers (`awaken.rs`,
  `workspace_scaffold_commands.rs`, `buddy_workspace.rs`, `swarm.rs`); that fragmentation IS the
  "wake rejects the flag team-spawn emits" bug and leaves the observer without a reliable "who is
  my L1." One seam makes the reject impossible and gives the observer one source of truth.
- Rejected: patching only the `wake` parser to accept the flag (fixes the symptom command but
  leaves four parsers to drift again — a 5th recurrence in waiting).

## Boundaries

- **Always:** maw-rs owns emit (one formatter) and observe (one observer + one durable queue +
  one drain + one parent seam). Reuse `PrQueueLock` discipline and stable identity keys. Scoped
  crate tests are the local proof; the full `cargo test --workspace` matrix is CI's job
  (`ci.yml`), never run on the dev machine (maw-rs `AGENTS.md`).
- **Never (fix-permanently guard — this is the 5th-recurrence guard):**
  - no `codex.md` / `AGENTS.md` / doctrine edit telling the L2 to ping harder or format more
    carefully — that is the symptom patch that recurred 4×;
  - no new `maw hey` variant and no "if the ping didn't land, also do X" fallback;
  - no send-keys delivery of handoff **content**;
  - no presentation-string dedupe key;
  - no correctness that depends on the L2 *remembering* to run a command.
- Out of scope: changing the tmux layout/reaper beyond wiring the observer teardram; upstream/alpha
  sync (origin/main only).

## Verification notes (for the implementing L2)

- maw-rs builds are 25–30GB and mostly compile. Verify with **scoped** crate tests only
  (`cargo test -p maw-cli`, plus any other crate the diff touches) — **never**
  `cargo test --workspace` on the dev machine (it OOMs; maw-rs `AGENTS.md` makes CI the owner of
  the full matrix). A quiet scoped compile is not a hang: give it a 900s+ ceiling and **never kill
  a slow compile** (killing corrupts incremental state → a self-inflicted re-timeout loop).
- Prove each terminal state (findings / exited / idle) end-to-end: observer → queue row → L1 drain
  banner, exactly once per transition.
