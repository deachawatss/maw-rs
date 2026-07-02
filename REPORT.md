# rs-plugins PR-4 report

## Survey

- Worktree: `agents/plugin-wasm` off `origin/alpha`.
- Reference checkout: `/opt/Code/github.com/Soul-Brews-Studio/maw-js`, fetched `origin/alpha` before survey.
- Current reference count: 95 plugin manifests under `src/commands/plugins/` and `src/vendor/mpr-plugins/`.
- Existing WASM parity fixtures in this worktree: 18 (`check`, `cleanup`, `config`, `consent`, `cross-team-queue`, `federation`, `learn`, `park`, `peek`, `ping`, `profile`, `project`, `send`, `serve-peer-startup-warnings`, `shellenv`, `triggers`, `trivial`, `workspace`).
- Manifest-level unconverted candidates in the fetched reference: 77. This is lower than the mission's approximate 117 total / 99 remaining, so the fetched `maw-js` alpha state is the source of truth for this pass.

House pattern studied:
- AssemblyScript conversions live in `examples/wasm-parity/<plugin>/src/plugin.ts`.
- Built WASM and hash metadata are committed into `crates/maw-plugin-manifest/tests/fixtures/wasm-parity/<plugin>/`.
- Fixture manifests use `entry.kind = "wasm"`, `path = "plugin.wasm"`, `export = "handle"`, and narrow capability strings.
- `host-state.json` seeds exact fake host responses and the harness asserts golden output plus optional host-call transcript.
- Scaffold tooling currently creates generic Rust/AssemblyScript plugins, but fixture conversions use the AssemblyScript SDK shim in `packages/wasm-sdk`.

Ranking principles:
- Daily-use/read/reporting verbs first.
- Bounded host functions first: `fs:read:*`, `sdk:config:read`, `sdk:localserver`, and narrow tmux read/tag APIs.
- Skip or defer serve/daemon/long-running/session-spawning flows for this wave.
- Prefer conversions with small, deterministic parity fixtures.

## Ranked unconverted list

1. `contacts` - daily coordination primitive; bounded `psi/contacts.json` read/write. Converted in this pass.
2. `signals` - daily situational awareness; bounded signal-directory read. Converted in this pass.
3. `costs` - daily resource check; bounded localserver read-only API. Converted in this pass.
4. `tag` - useful routing metadata; bounded tmux tags/title read-write, but mutates live tmux state.
5. `whoami` - tiny session identity primitive; good low-risk follow-up.
6. `session` - alias-level identity primitive; should likely share `whoami` conversion shape.
7. `ls` - daily live-session listing; likely tmux read plus formatting.
8. `panes` - daily pane metadata read; likely tmux read/tags read.
9. `find` - diagnostic search across sessions/fleet metadata; bounded read if scoped carefully.
10. `locate` - diagnostic oracle lookup; bounded read of fleet/config/tmux.
11. `activity` - read-only pane activity classification; bounded tmux capture with fixture snapshots. Converted in this follow-up batch.
12. `capture` - direct tmux capture; read-only but output-size behavior needs careful parity.
13. `discover` - federation inventory/status read; likely config/tmux read. Converted in this follow-up batch.
14. `transport` - transport diagnostics; likely read-only.
15. `peers` - peer alias read/manage; read subcommands first, mutating subcommands later.
16. `trust` - trust list read/manage; consent/trust mutation needs human-at-terminal boundaries.
17. `scope` - routing namespace primitive; likely config/state file read/write.
18. `inbox` - high value, but broader queue/approval surface.
19. `messages` - high value, but SQLite-backed ledger and API surface make it larger.
20. `pr` - useful, but likely needs bounded `gh` exec/API decisions.
21. `assign` - useful GitHub action; external side effects mean later.
22. `about` - useful read-only oracle information. Converted in this follow-up batch.
23. `pulse` - task pulse add/list/cleanup; split read/write modes.
24. `rename` - bounded tmux mutation; similar risk class to `tag`.
25. `zoom` - bounded tmux mutation.
26. `send-enter` - bounded tmux send, but mutation/AI-pane safety must match host policy.
27. `send-text` - bounded tmux send plus Enter; same safety class as `send`.
28. `run` - tmux send with Enter; destructive text policy matters.
29. `tab` - list/peek/message tabs; split read vs send modes.
30. `pane` - pane swap mutation; bounded tmux operation but live-state mutating.
31. `tile` - tmux layout/spawn; mutating and potentially pane-spawning.
32. `tmux` - broad tmux control surface; convert narrow subcommands only.
33. `fleet` - valuable, but persistent registry management is broader.
34. `oracle` - broad management surface; split read-only list/about before scan/register/prune.
35. `health` - useful diagnostics; needs several exec/fs/network checks.
36. `doctor` - useful diagnostics plus auto-heal; split read-only checks first.
37. `completions` - deterministic output, low operational value.
38. `setup` - host setup helpers; side-effect heavy.
39. `plugin` - plugin lifecycle; already has native Rust work and broad build/install behavior.
40. `artifact-manager` - file lifecycle; larger state model.
41. `token` - credential-sensitive; defer.
42. `oracle-skills` - external pass-through; defer.
43. `zenoh-scout` - opt-in discovery provider; external/network dependency.
44. `ui` - UI management; likely opens/serves UI.
45. `view` - attach/view behavior; interactive.
46. `attach` - interactive attach/wake fallback.
47. `attach-ssh` - remote live attach.
48. `follow` - live PTY/websocket stream; long-running.
49. `stream` - long-running tmux mirroring.
50. `bg` - detached long command; long-running process.
51. `overview` - valuable dashboard, but creates/kills a long-lived tmux overview session; skipped this wave.
52. `wake` - session-spawning; skipped this wave.
53. `workon` - worktree/session-spawning; skipped this wave.
54. `split` - starts/attaches sessions/panes; skipped this wave.
55. `swarm` - multi-agent spawning; skipped this wave.
56. `team` - multi-agent lifecycle; skipped this wave.
57. `oracle-workon` - composes wake/split/swarm; skipped this wave.
58. `mega` - team manager; skipped this wave.
59. `avengers` - team manager; skipped this wave.
60. `broadcast` - multi-target send; mutation plus fan-out.
61. `talk-to` - signed federation send; network/consent sensitive.
62. `pair` - human pairing/trust handshake; defer.
63. `reunion` - federation sync side effects.
64. `soul-sync` - cross-node sync side effects.
65. `restart` - process management; defer.
66. `kill` - destructive tmux mutation; defer.
67. `sleep` - session shutdown; defer.
68. `stop` - all-fleet shutdown; defer.
69. `done` - retrospective plus kill plus worktree removal; defer.
70. `archive` - archive session/data; defer.
71. `absorb` - archive and switch ownership; defer.
72. `take` - move tmux window; defer.
73. `resume` - parked agent wake/attach; defer.
74. `bud` - creates new oracle; defer.
75. `awaken` - bud plus ritual; defer.
76. `incubate` - bud/wake around repo; defer.
77. `init` - first-run wizard/config write; defer until interactive prompts are modeled.

Per-oracle plugin note:
- I did not find a `squad` plugin manifest in the fetched `maw-js` plugin paths. The `squad` term appears as a team/workspace target in tests.
- A per-oracle plugin pattern should convert as one manifest per oracle/plugin slug with the same `entry` export and a narrow capability set derived from that oracle's actual operations. Shared helper logic can stay in the AssemblyScript SDK/example source, but each per-oracle fixture should keep its own manifest, host-state transcript, and golden cases so capability drift is visible.
- I did not touch athena's files.

## Conversions

- `contacts` converted as an AssemblyScript WASM parity fixture:
  - Source: `examples/wasm-parity/contacts/src/plugin.ts`
  - Fixture: `crates/maw-plugin-manifest/tests/fixtures/wasm-parity/contacts/`
  - Covered CLI cases: no args, `ls`, and `rm` without a name.
  - Capabilities: `fs:read:data`, `fs:write:data`.
- `signals` converted as an AssemblyScript WASM parity fixture:
  - Source: `examples/wasm-parity/signals/src/plugin.ts`
  - Fixture: `crates/maw-plugin-manifest/tests/fixtures/wasm-parity/signals/`
  - Covered CLI cases: no args and `--days 3 --json`.
  - Capabilities: `fs:read:data`.
- `costs` converted as an AssemblyScript WASM parity fixture:
  - Source: `examples/wasm-parity/costs/src/plugin.ts`
  - Fixture: `crates/maw-plugin-manifest/tests/fixtures/wasm-parity/costs/`
  - Covered CLI cases: no args and `--daily --json`.
  - Capabilities: `sdk:localserver`.
- `about` converted as an AssemblyScript WASM parity fixture:
  - Source: `examples/wasm-parity/about/src/plugin.ts`
  - Fixture: `crates/maw-plugin-manifest/tests/fixtures/wasm-parity/about/`
  - Covered CLI cases: `athena` and no args.
  - Capabilities: `fs:read:data`, `fs:read:config`, `tmux:read`.
- `activity` converted as an AssemblyScript WASM parity fixture:
  - Source: `examples/wasm-parity/activity/src/plugin.ts`
  - Fixture: `crates/maw-plugin-manifest/tests/fixtures/wasm-parity/activity/`
  - Covered CLI cases: `athena:0`, `athena:0 --json`, and no args.
  - Capabilities: `tmux:read`.
- `discover` converted as an AssemblyScript WASM parity fixture:
  - Source: `examples/wasm-parity/discover/src/plugin.ts`
  - Fixture: `crates/maw-plugin-manifest/tests/fixtures/wasm-parity/discover/`
  - Covered CLI cases: `--peers config`, `--peers config --json`, `--awake`, and invalid `--peers bogus`.
  - Capabilities: `sdk:config:read`, `fs:read:config`, `tmux:read`.

## Test evidence

- `npm run build:contacts` from `packages/wasm-sdk` passed; rebuilt wasm matched the committed fixture bytes with `cmp`.
- `cargo test -p maw-plugin-manifest contacts` passed: 2 contacts parity/capability tests.
- `npm run build:signals` from `packages/wasm-sdk` passed.
- `cargo test -p maw-plugin-manifest signals` passed: 2 signals parity/capability tests.
- `npm run build:costs` from `packages/wasm-sdk` passed.
- `cargo test -p maw-plugin-manifest costs` passed: 2 costs parity/capability tests.
- Before `a6b6b56e` (`contacts`): `cargo fmt --check`, `cargo test -p maw-plugin-manifest`, and `cargo clippy --all-targets` passed.
- Before `93fd199b` (`signals`): `cargo fmt --check`, `cargo test -p maw-plugin-manifest`, and `cargo clippy --all-targets` passed.
- Before `df9ddc3a` (`costs`): `cargo fmt --check`, `cargo test -p maw-plugin-manifest`, and `cargo clippy --all-targets` passed.
- `npm run build:about` from `packages/wasm-sdk` passed.
- `cargo test -p maw-plugin-manifest about` passed: 2 about parity/capability tests.
- Before the `about` commit: `cargo fmt --check`, `cargo test -p maw-plugin-manifest`, and `cargo clippy --all-targets` passed.
- `npm run build:activity` from `packages/wasm-sdk` passed.
- `cargo test -p maw-plugin-manifest activity` passed: 2 activity parity/capability tests.
- Before the `activity` commit: `cargo fmt --check`, `cargo test -p maw-plugin-manifest`, and `cargo clippy --all-targets` passed.
- `npm run build:discover` from `packages/wasm-sdk` passed.
- `cargo test -p maw-plugin-manifest discover` passed: discover-filtered package tests, including 2 discover parity/capability tests.
- Before the `discover` commit: `cargo fmt --check`, `cargo test -p maw-plugin-manifest`, and `cargo clippy --all-targets` passed.

## Blockers / risks

- No hard blocker yet.
- `cargo clippy --all-targets` may be expensive for the full workspace; it remains required before each plugin commit.
- `discover` is intentionally scoped to read-only config/fleet/tmux paths for this batch; scout/both network discovery remains outside this fixture.
