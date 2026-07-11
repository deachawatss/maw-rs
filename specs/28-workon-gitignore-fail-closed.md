# SPEC-28: Fail closed when the managed `.gitignore` block cannot be installed

Issue: #28
Date: 2026-07-11
Mode: standard

## Objective

Prevent a newly created `workon` worktree from launching an L2 session unless
the managed `.gitignore` block that excludes ephemeral `.maw/` state has been
verified as installed.

## Acceptance Criteria

- [ ] A malformed managed block or failed `.gitignore` write makes `workon`
  exit non-zero before it creates a tmux window or sends the engine command.
- [ ] The returned error preserves the underlying cause and tells the operator:
  `Fix .gitignore manually or remove the malformed managed block, then retry`.
- [ ] Integration coverage exercises malformed and read-only `.gitignore`
  inputs through the `maw-rs workon` binary.

## Seams and Testing

- Public seam: `maw-rs workon demo feat --layout nested`, run against the
  existing hermetic fake git/tmux harness.
- Prior art: `crates/maw-cli/tests/native_workon_plugin.rs`.
- Expected values: the issue's literal remediation text, a non-zero process
  status, and no `new-window` command in the fake tmux log.

## Decisions

### Treat managed ignore installation as a launch gate

- Chose: propagate the error from `ensure_gitignore_ephemeral_block` with the
  operator remediation, instead of emitting a warning and continuing.
- Why: `.maw/` state is safety-critical ephemeral state; continuing permits it
  to be committed by otherwise explicit staging.
- Rejected: warning-only continuation or deferring the check until after tmux
  launch, because both permit an unprotected L2 session to start.

## Boundaries

- Always: preserve the detailed failure returned by the managed-block helper.
- Never: launch tmux/engine work after managed-block installation fails.
- Out of scope: rolling back the already-created git worktree or changing the
  managed block's contents.
