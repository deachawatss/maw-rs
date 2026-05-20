# maw-rs

Rust port of the maw-js portable core.

`maw-rs` is intentionally starting with deterministic, side-effect-free crates.
Each crate copies the same JSON fixture contract from `maw-js/test/spec/` and
must pass those fixtures in Rust before runtime IO, transports, or CLI commands
move over.

## Phase 1 status

Cargo workspace scaffolded and pushed to `main`.

| Crate | maw-js source | Portable fixture |
| --- | --- | --- |
| `maw-matcher` | `src/core/matcher/resolve-target.ts`, `normalize-target.ts` | `matcher-resolve-target.fixtures.json`, `normalize-target.fixtures.json` |
| `maw-calver` | `scripts/calver.ts` | `calver.fixtures.json` |
| `maw-policy` | `src/plugin/default-active.ts`, `src/plugin/tier.ts`, `src/plugin/manifest-constants.ts` | `plugin-policy.fixtures.json` |
| `maw-worktree` | `src/core/fleet/worktree-window-match.ts` | `worktree-window-match.fixtures.json` |
| `maw-transport` | `src/core/transport/transport.ts` | `transport-router.fixtures.json` |
| `maw-routing` | `src/core/routing.ts` | `routing.fixtures.json` |
| `maw-identity` | `src/core/fleet/session-name.ts`, `src/core/fleet/node-identity.ts` | `canonical-session-name.fixtures.json`, `canonical-node-identity.fixtures.json` |
| `maw-bring` | `src/commands/shared/bring-flags.ts` | `bring-to-flag.fixtures.json`, `bring-to-target.fixtures.json`, `bring-self-guard.fixtures.json` |
| `maw-split` | `src/vendor/mpr-plugins/split/impl.ts`, `src/commands/plugins/tmux/safety.ts` | `split-policy.fixtures.json` |
| `maw-peer` | `src/commands/shared/peer-sources.ts` | `peer-source-resolver.fixtures.json` |
| `maw-tmux` | `src/core/transport/tmux-class.ts`, `src/commands/shared/discover-live-state.ts` | tmux parser unit tests, `discover-tmux-live-state.fixtures.json` |
| `maw-hub` | `src/transports/hub-config.ts` | `test/isolated/hub-config.test.ts`, hub config loader coverage |
| `maw-feed` | `src/lib/feed.ts` | `test/isolated/feed-lib-coverage.test.ts` |
| `maw-auth` | `src/lib/federation-auth.ts` | federation auth pure helper and O6 decision tests |

Current local gates:

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

## Phase 2 plan

1. Add side-effecting transport implementations behind the `maw-transport` policy:
   - tmux via the `tmux` CLI
   - HTTP federation via injectable IO first; runtime HTTP client wiring later
   - Zenoh via the Rust stylos/themion ecosystem
2. Add runtime adapters for fleet/worktree/session discovery around the pure crates.
3. Keep maw-js and maw-rs running side-by-side until command parity is proven.

## Phase 3 plan

1. Add a `maw-rs` CLI with `clap`.
2. Port high-value fast-path commands first: `ls`, `hey`, `peek`, and target resolution helpers.
3. Validate each command against maw-js fixtures or captured golden outputs before replacing any default `maw` entrypoint.
