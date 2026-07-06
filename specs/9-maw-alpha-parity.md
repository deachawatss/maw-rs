# SPEC-9: maw-rs upstream alpha parity merge

Issue: #9
Date: 2026-07-06
Strategy: TEAM (fallback SOLO if omx unavailable)

## Objective

Merge 132 upstream/alpha commits into our fork branch (agents/1-maw-alpha-parity), preserving all 26 fork-port commits. The fork-ports add done hardening, tmux submit pipeline, workon engine prep, and team lifecycle patches. Upstream has refactored maw-tmux from numbered part files to semantic modules and renamed done.rs to worktree_finish.rs. Goal: a clean merge that gains upstream's new commands while keeping fork behavior intact.

## Seams (Test Surfaces)

- [ ] `cargo test --workspace` — full workspace passes (the merge gate)
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` — no warnings

## Design Decisions

### Decision 1: Merge strategy — plain merge, NOT -X theirs
- **Chose:** `git merge upstream/alpha` with manual conflict resolution
- **Why:** `-X theirs` silently wipes fork patches (Oracle learning 2026-05-08). Manual resolution lets us map our patches to upstream's new file structure.
- **Rejected:** cherry-pick (fails at 100+ divergence — Oracle learning 2026-05-16), `-X theirs` (wipes fork patches)

### Decision 2: Modify/delete resolution — delete our old files, re-apply patches to upstream's new locations
- **Chose:** Accept upstream's file deletions/renames, re-apply fork patches to the new target files
- **Why:** Upstream refactored for good reason (semantic naming, _parts/ subdirs). Fighting the refactoring creates permanent merge debt. Our patches are small and transplantable.
- **Rejected:** Keeping our old files alongside upstream's new ones (duplicate DISPATCH_57 constants, build failure)

### Decision 3: wind_delivery.rs and wind/ modules survive as-is
- **Chose:** Keep fork-specific modules (wind_delivery.rs, src/wind/) — they have no upstream equivalent
- **Why:** These are fork-only additions, not modifications of upstream code. The mod.rs needs updating to reference wind_delivery in upstream's new include structure.

## Conflict Resolution Map

| Conflict | Type | Our patch | Upstream change | Resolution |
|---|---|---|---|---|
| `maw-cli/core_impl/done.rs` | modify/delete | Self-invocation guard, rrr wait, push guard | Renamed to `worktree_finish.rs` | Delete done.rs, apply patches to worktree_finish.rs |
| `maw-cli/core_impl/workon.rs` | content | Engine prep, sanitize task slug, worktree cleanup | Various upstream changes | Manual 3-way merge |
| `maw-tmux/core_impl/mod.rs` | content | Added `pub mod wind_delivery;` | Replaced part includes with semantic modules | Add wind_delivery to new mod.rs |
| `maw-tmux/core_impl/part01.rs` | modify/delete | Added `pub use wind_delivery::*`, `SendThrottle::Busy` | Content moved to `types_runner_parts/` | Delete part01.rs, apply to new locations |
| `maw-tmux/core_impl/part02_2.rs` | modify/delete | send_text uses wind_delivery config | Content moved to `client_pane_send_parts/` | Delete part02_2.rs, apply to new locations |
| `maw-tmux/core_impl/part03.rs` | modify/delete | OSC strip, codex paste detection | Content moved to `action_resolution_parts/` | Delete part03.rs, apply to new locations |
| `maw-tmux/core_impl/tests_impl/part03.rs` | modify/delete | OSC strip + paste tests | Tests moved to `send_text_pending_parts/` | Delete, apply tests to new locations |

## Success Criteria

- [ ] All 26 fork-port commits' changes are present in the merged tree (git log compare)
- [ ] `cargo build --release` succeeds
- [ ] `cargo test --workspace` passes
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` passes
- [ ] No duplicate DISPATCH_NN constants
- [ ] wind_delivery.rs and wind/ module functionality preserved
- [ ] PR body lists newly-portable verbs (candidates for rs-native.list)

## Boundaries

- **Always:** Preserve fork patches; use explicit git add (never -A/.)
- **Ask first:** If any fork-port commit's changes appear dropped after merge
- **Never:** Edit rs-native.list or maw-wrapper.sh; push to upstream; force push; git reset --hard

## Out of Scope

- Expanding rs-native.list (L1 post-review)
- Editing maw-wrapper.sh
- Any new feature work beyond the merge
