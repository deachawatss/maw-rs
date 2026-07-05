# maw-js fork patch migration

This branch keeps `maw-rs` on top of upstream core and migrates Wind/deachawat fork-patch behavior by hardening existing maw-rs commands/modules instead of creating duplicate plugin names.

## Naming convention

- Patch existing upstream surfaces when they already exist: `done`, `team`, `workon`, `overview`, `cleanup`, `pr`, `tmux`/`send-text`.
- Add internal helpers only when there is no existing owner: submit/pane-delivery helpers live under `maw-tmux`, not as a new user-facing `maw-*` command.
- Keep Wind-only workflow policy out of generic names unless it is explicitly config-gated.

## Migrated in this fork branch

### submit / pane delivery hardening

Owner: existing `maw-tmux` send/capture helpers.

Migrated from maw-js fork patches:

- pending-input detection for prompts with typed content
- Codex `[Pasted Content ... chars]` false-positive submit detection
- OSC escape stripping while preserving ordinary captured text
- existing retry-Enter path remains in `send_text_with_sleeper`
- readiness polling before pane writes, busy-output guard, and engine-specific submit confirm intervals live in `core_impl::wind_delivery`

Fork-patch map:

| maw-js patch | Wind module | Hook site | Proof test |
|---|---|---|---|
| `comm-send.ts` #14/#15 readiness + busy guard | `crates/maw-tmux/src/core_impl/wind_delivery.rs` | `TmuxClient::send_text_with_config_and_sleeper` / `TmuxClient::busy_guard` in `part02_2.rs` | `readiness_gate_polls_until_prompt_visible`, `busy_guard_blocks_send_during_active_output` |
| `comm-send.ts` #21 verify-submit retry intervals | `crates/maw-tmux/src/core_impl/wind_delivery.rs` | `TmuxClient::send_text` config selection in `part02_2.rs` | `verify_submit_uses_engine_specific_intervals` |
| fork/upstream divergence guard | `crates/maw-tmux/src/core_impl/wind_delivery.rs` | Wind hook calls retained in `part02_2.rs` | `fork_divergence_hook_keeps_wind_delivery_at_submit_site` |

Proof tests:

- `cargo test -p maw-tmux pending_input_detection_matches_maw_js_prompt_heuristic`
- `cargo test -p maw-tmux readiness_gate_polls_until_prompt_visible`
- `cargo test -p maw-tmux busy_guard_blocks_send_during_active_output`
- `cargo test -p maw-tmux verify_submit_uses_engine_specific_intervals`
- `cargo test -p maw-tmux fork_divergence_hook_keeps_wind_delivery_at_submit_site`

### done hardening

Owner: existing `maw done` / `finish` implementation.

Migrated from maw-js fork patches:

- auto-save push is now guarded; it skips push when branch is `main`, `HEAD`, empty, has no live remote branch, or its PR is `MERGED`/`CLOSED`
- branch/PR-state guard has pure unit coverage so upstream sync cannot silently remove the behavior
- existing lead-window self-guard, retrospective command selection, worktree removal, and fleet config cleanup remain on the existing command surface

Proof tests:

- `cargo test -p maw-cli done_push_guard_blocks_main_head_and_closed_pr_states`

### repo hygiene

- Resolved committed conflict markers in `REPORT.md` by preserving both report sections under a single heading.

## Already covered by upstream maw-rs source

The fork-patch audit found upstream maw-rs already has native command surfaces for:

- `done` / `finish`
- `team`
- `workon`
- `overview`-adjacent tmux/fleet helpers
- `send-text`, `send-enter`, `run`, and tmux helpers
- `shellenv`
- cleanup/fleet/team delete paths

So this migration intentionally patches existing implementations instead of adding duplicate plugins like `maw-done` or `maw-team`.

## Remaining follow-up candidates

These should be implemented as small hardening PRs on existing surfaces, not new plugin names:

1. `team`: anchored spawn/kickoff and orphan pane sweep proofs.
2. `workon`: fresh-worktree command sanitization proofs and engine-resolution policy tests.
3. `overview`: roster reconciliation and pane-base-index proofs.
4. `cleanup`: leaked internal session/team cleanup proofs.
5. `pr`/fleet queue: dedupe and closed-PR reconciliation proofs.
6. `spawn`: replace any remaining hardcoded shell use with the user's shell/default bash through shared tmux spawn helpers.

## Verification gates for this branch

Run before using or pushing the fork branch:

```bash
cargo fmt --all -- --check
cargo test -p maw-tmux pending_input_detection_matches_maw_js_prompt_heuristic
cargo test -p maw-tmux readiness_gate_polls_until_prompt_visible
cargo test -p maw-tmux busy_guard_blocks_send_during_active_output
cargo test -p maw-tmux verify_submit_uses_engine_specific_intervals
cargo test -p maw-tmux fork_divergence_hook_keeps_wind_delivery_at_submit_site
cargo test -p maw-cli done_push_guard_blocks_main_head_and_closed_pr_states
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
target/debug/maw-rs --version
target/debug/maw-rs ls --json
```

No upstream PR is created from this branch unless explicitly requested later.
