# SPEC-33: Pulse remote cleanup

Issue: #33
Date: 2026-07-11
Mode: standard

## Objective

When `maw pulse cleanup` removes stale agent worktrees, prune each touched
repository's origin refs and remove only remote branches whose GitHub pull
request has demonstrably merged.

## Acceptance Criteria

- [ ] A live cleanup runs `git -C <repo> fetch --prune origin` once for every
  repository with a stale worktree.
- [ ] Each stale `agents/*` branch is remote-deleted only after `gh pr view`
  reports `state: MERGED`; non-merged or unverifiable branches remain remote.
- [ ] Dry-run reports a merged remote branch it would delete and runs neither
  fetch nor remote deletion.
- [ ] Existing safe local `git branch -d` behavior remains unchanged.

## Seams and Testing

- CLI seam: `maw pulse cleanup [--dry-run]` with temporary worktrees and fake
  `git`, `gh`, and `tmux` executables.
- Prior art: `crates/maw-cli/tests/native_pulse_plugin.rs`.
- Expected values: the issue's literal `MERGED` GitHub PR state and exact
  subprocess argument logs.

## Decisions

### Merge proof is the only deletion authority

- Chose: query `gh pr view <branch> --repo <owner>/<repo> --json state` and
  issue `git push origin --delete <branch>` only for `MERGED`.
- Why: a direct GitHub PR state is explicit merge proof and works regardless
  of local branch topology.
- Rejected: deleting all stale remote branches or inferring merge status from
  `git branch -d`, because neither establishes a remote branch was merged.

### Dry-run is read-only

- Chose: inspect PR state to render planned remote deletion, but skip fetch,
  worktree removal, local deletion, and remote deletion.
- Why: `fetch --prune` mutates local remote-tracking refs, contradicting the
  dry-run promise that cleanup deletes nothing.

## Boundaries

- Always: preserve the local merged-only `git branch -d` deletion.
- Never: force-delete a local branch or delete a remote branch without a
  `MERGED` PR state.
