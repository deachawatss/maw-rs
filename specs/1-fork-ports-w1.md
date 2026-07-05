# SPEC-1: Wave-1 Fork Patch Ports — maw-rs Hardening

Issue: #1
Date: 2026-07-05
Strategy: TEAM (4 workers, explicit L1 split)

## Objective

Port the remaining maw-js fork-patch behaviors into existing maw-rs command surfaces. These are hardening patches that prevent data loss (ψ-rescue), avoid misrouted panes (caller-pane anchor), ensure agent readiness before submission (readiness gate), and sanitize worktree state (engine resolution).

The migration doc (`docs/maw-js-fork-patch-migration.md`) already landed submit/done push-guard patches. This issue covers the **remaining follow-up candidates** listed in that doc: team, workon, plus deeper done/submit hardening.

## Seams (Test Surfaces)

Each cluster has one primary test seam — all pure-logic, no live tmux/git:

- [ ] Seam 1 (W1): `crates/maw-tmux` — `cargo test -p maw-tmux` regression tests for readiness gate, busy guard, and verify-submit retry with configurable timeouts
- [ ] Seam 2 (W2): `crates/maw-cli` done module — `cargo test -p maw-cli` tests for ψ-rescue (plant uncommitted ψ file in scratch dir, assert survival after simulated worktree removal), /rrr wait, self-invocation guard
- [ ] Seam 3 (W3): `crates/maw-cli` team_core module — `cargo test -p maw-cli` tests for caller-pane anchor ($TMUX_PANE), OMX auto-kickoff, orphan-pane sweep
- [ ] Seam 4 (W4): `crates/maw-cli` workon module — `cargo test -p maw-cli` tests for task-slug sanitization, engine default/warn/record, fresh-worktree command sanitization

## Design Decisions

### Decision 1: Harden existing surfaces, no new crates or commands
- **Chose:** Patch `maw-tmux::TmuxClient` and `maw-cli::core_impl::{done,team_core,workon}` directly
- **Why:** Migration doc mandates patching existing upstream surfaces, not creating duplicate plugin names
- **Rejected:** New `maw-done`, `maw-team-harden` crates — would duplicate command surfaces and diverge from upstream

### Decision 2: Pure-logic tests with fixture injection, no live tmux
- **Chose:** Mock `TmuxRunner` for submit pipeline; temp-dir fixtures for ψ-rescue; struct-based engine config for workon
- **Why:** 99.85% coverage target requires deterministic tests; live tmux makes CI flaky
- **Rejected:** Integration tests against real tmux server — flaky in CI, slow

### Decision 3: Falsification-checked regression tests
- **Chose:** Every ported behavior gets a focused test that fails RED when the patch line is reverted
- **Why:** Oracle learning from maw-js #84 epic — "the test IS the checklist; presence-grep checklists rot silently"
- **Rejected:** Grep-based presence checks — rot silently (proven in maw-js)

## Worker Assignments (disjoint files)

### W1: hey/submit pipeline
**Files:** `crates/maw-tmux/src/core_impl/part01.rs` + new test file
**Behavior:**
1. **Readiness gate** — before sending text, poll pane capture for ctx% prompt readiness. Claude timeout 45s, Codex timeout 8s. Return early if pane shows active command output.
2. **Busy guard** — skip send if last capture shows the agent is mid-response (no prompt line visible).
3. **Verify-submit retry** — enhance `submit_with_confirm` with configurable per-engine poll intervals.
**maw-js refs:** comm-send.ts #14/#15/#21 (readiness polling, Codex false-positive detection, submit retry)
**Test contract:**
- `readiness_gate_polls_until_prompt_visible` — mock runner returns "processing..." then "$ ", assert gate passes on second poll
- `busy_guard_blocks_send_during_active_output` — mock runner capture shows no prompt, assert SendThrottle::Busy
- `verify_submit_retries_with_engine_specific_intervals` — assert Claude uses 700ms confirm, Codex uses faster

### W2: done hardening
**Files:** `crates/maw-cli/src/core_impl/done.rs`
**Behavior:**
1. **ψ-rescue NEVER-overwrite** — before `git worktree remove`, scan worktree for uncommitted `ψ/` files. Copy them to the main repo's `ψ/` dir, NEVER overwriting existing files (append timestamp suffix on collision). Resolution: `dirname(git rev-parse --git-common-dir)` for layout-agnostic main-repo detection.
2. **/rrr wait** — after sending retrospective command, wait for it to complete (poll pane for prompt return) before killing window. Current code sends retro but doesn't wait.
3. **Self-invocation guard** — harden `done_assert_may_target_lead` to also block `maw done <own-window-name>` when the caller IS the target window (prevents ENOENT zombie).
**maw-js refs:** done/psi-rescue.ts, done-autosave.ts, done-self-guard.ts
**Test contract:**
- `psi_rescue_copies_uncommitted_files_without_overwrite` — plant ψ/test.md in temp worktree, plant ψ/test.md in main repo, run rescue, assert both survive (main untouched, worktree copy gets timestamp suffix)
- `rrr_wait_polls_until_prompt_returns` — mock pane capture returns retro output then prompt, assert wait completes
- `self_invocation_guard_blocks_own_window` — simulate done targeting own window, assert error

### W3: team hardening
**Files:** `crates/maw-cli/src/core_impl/team_core.rs`
**Behavior:**
1. **Caller-pane anchor** — `team spawn --exec` reads `$TMUX_PANE` env var and anchors split-window to the caller's pane, not the tmux active window. Prevents workers spraying into L1's window.
2. **OMX auto-kickoff** — when engine=omx and --exec, after pane creation, auto-send the spawn prompt to start the OMX worker immediately.
3. **Orphan-pane sweep** — `maw team status` detects panes whose PID is dead (not running) and marks them as zombies in the status display. `maw team prune` kills them.
**maw-js refs:** team-lifecycle.ts #83 (caller-pane anchor fix)
**Test contract:**
- `caller_pane_anchor_uses_tmux_pane_env` — set $TMUX_PANE, assert spawn command targets that pane
- `omx_auto_kickoff_sends_prompt_after_spawn` — mock runner, assert send-keys called after split-window
- `orphan_pane_sweep_detects_dead_pids` — supply pane list with dead PIDs, assert zombie count > 0

### W4: workon hardening
**Files:** `crates/maw-cli/src/core_impl/workon.rs`
**Behavior:**
1. **Fresh-worktree sanitization** — after `git worktree add`, run `git clean -fd` in the new worktree to remove stale build artifacts. Clear any inherited `.maw/` state files. Ensure CLAUDE.md is inherited from main.
2. **Engine default/warn/record** — detect which engine will run in the new window (from merged config). Warn if engine is not trusted for the target repo. Record engine choice in `.maw/strategy.json` if it exists.
3. **Engine resolution tests** — `workon_build_command_in_dir` already works; add tests that verify the maw-js-compatible fallback chain: per-agent → default → "claude".
**maw-js refs:** workon/impl.ts #16/#22 (worktree sanitization, engine warn/record)
**Test contract:**
- `fresh_worktree_cleans_stale_state` — create temp dir with `.maw/phase.json`, run sanitization, assert removed
- `engine_warn_untrusted_repo` — config says engine=codex, repo not in trust list, assert warning emitted
- `engine_resolution_fallback_chain` — test per-agent → default → "claude" priority

## Consolidated Fork Surface (BINDING)

Wind directive — supersedes any per-worker file layout that predates this section.

### Module tree

ALL fork logic lives in ONE module tree: `crates/maw-cli/src/wind/` (a single `mod wind;` hook in `lib.rs`; per-concern files inside: `done.rs`, `team.rs`, `workon.rs`; each ≤250 lines). maw-tmux fork logic stays in `crates/maw-tmux/src/core_impl/wind_delivery.rs` since it belongs to a different crate.

### Test file

Exactly ONE test file: `crates/maw-cli/tests/fork_divergence.rs`. Every worker's assertions merge into it at aggregation, exercising maw-tmux-level behavior through maw-cli's dependency on maw-tmux. Use `mod <concern>_hardening { ... }` scoping inside the file. Per-worker test files (`hey_submit_hardening.rs`, `native_done_hardening.rs`, `team_harden_spawn.rs`, `native_workon_hardening.rs`) are TEMPORARY scaffolding — they MUST be deleted after their asserts move into `fork_divergence.rs`.

### Upstream hooks

Upstream-owned files (`core_impl/done.rs`, `core_impl/team_core.rs`, `core_impl/workon.rs`) keep maximum 1-3-line hooks that call into `wind::*`. No fork logic inline.

### Inventory

ONE inventory doc: `docs/maw-js-fork-patch-migration.md`.

### Merge-drop red line (BINDING)

`crates/maw-cli/tests/fork_divergence.rs` is the ONE canonical merge-drop checklist. Every merge-exposed fork behavior — including W1's submit/readiness gate in maw-tmux — MUST have its divergence assert in this file, exercised through maw-cli's dependency on maw-tmux. If an upstream merge drops any hook, `fork_divergence.rs` alone must go red. `hey_submit_hardening.rs` may keep crate-local unit tests of `wind_delivery` internals, but those are NOT the merge-drop signal.

### Aggregation gate

Before `.maw/aggregate-verified`, verify:
- `ls crates/maw-cli/src/wind/` is non-empty (fork module tree exists)
- `test -f crates/maw-cli/tests/fork_divergence.rs` (consolidated test exists)
- `fork_divergence.rs` contains divergence asserts for ALL 4 clusters (W1-W4)
- ZERO leftover per-worker test files in `crates/maw-cli/tests/` beyond `fork_divergence.rs`

## Success Criteria

- [ ] `cargo test --workspace` green with all new regression tests
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` clean
- [ ] `cargo fmt --check` clean
- [ ] No file exceeds 250 lines
- [ ] Each ported behavior has a focused test that goes RED when the patch line is reverted (falsification)
- [ ] Worker files are strictly disjoint — no cross-contamination

## Boundaries

- **Always:** Pure-logic tests, mock TmuxRunner, temp-dir fixtures
- **Always:** `#![forbid(unsafe_code)]`, clippy pedantic
- **Never:** Live tmux in tests, real git operations in unit tests, new user-facing commands
- **Never:** Touch files outside your assigned cluster

## Out of Scope

- overview: roster reconciliation (follow-up #3 in migration doc)
- cleanup: leaked session cleanup (follow-up #4)
- pr/fleet queue: dedupe/reconciliation (follow-up #5)
- spawn: hardcoded shell replacement (follow-up #6)
