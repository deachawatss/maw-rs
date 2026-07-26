# maw-rs

Rust port of maw-js — distributed terminal multiplexing & fleet management.
A Cargo workspace of small, focused crates. BUSL-1.1 licensed.
For repo-wide agent execution conventions, read `AGENTS.md` first; this file remains the Claude-specific memory and release detail.

## Build Gate — DO NOT BUILD OR TEST LOCALLY (Wind ruling, 2026-07-26)

**No `cargo test`. No `cargo clippy`. No `cargo build`. Not scoped, not
`-p <crate>`, not a single `-- <test_name>`. None.**

Make the fix, read the diff, commit, push, open the PR, hand to L1. That is
the whole L2 loop for this repo.

Rust compilation is the single heaviest thing that runs on this machine. Two
concurrent scoped builds took the disk from 40Gi to 25Gi in minutes on
2026-07-26, and a later run exhausted **memory** and killed three live L2
panes outright. The work survived only because it was in worktrees. This is
not a tuning problem to be solved with a narrower `-p` flag — the earlier
"scope tests to what you changed" rule was already scoped, and it still
crashed the box.

**CI is the gate.** Push, then read the GitHub Actions result. If CI is red,
fix from the log — do not reproduce locally to "see it fail".

Consequences to be honest about, so nobody is surprised:

- A PR now reaches L1 with **no local verification evidence**. `/sop-verify
  --author` for this repo means "diff reviewed, reasoning stated", not
  "commands run". Say so plainly in the handoff — do not claim a check you
  did not run.
- L1 reviews the diff and the CI result, not a claimed test run.
- **maw-rs CI has been red on `main` since 2026-07-25** (7 `maw-cli --lib`
  tests fail on Linux, pass on macOS — issue #127). While that is true, CI
  cannot distinguish a new break from the standing one. Compare against the
  known-failing set in #127 before concluding a PR broke something.

This rule is maw-rs only. Other repos keep their normal proportional
verification.

## Development Lane — Lightweight, but L2 worktrees ARE used

maw-rs is infra, so the **merge lane** is lightweight. That governs how a
change lands, not where it is written.

**L2 deliveries here run in worktrees, like everywhere else.** An earlier
revision of this file said "do not use `maw workon` or L2 worktrees" and
justified it with "worktrees duplicate the entire Cargo dependency tree
(~100GB)". Both halves are now wrong:

- `.cargo/config.toml` points every build at a shared `/tmp/maw-rs-target`,
  so worktrees do **not** each carry a dependency tree.
- maw-rs #116 (`d713b94`, 2026-07-26) made worktree creation unconditional
  on every lane. There is no supported way to dispatch without one short of
  `--no-wt`.

Worktrees are also what saved three in-flight deliveries when the machine
ran out of memory on 2026-07-26 — the panes died, the work did not.

- Dispatch: `maw workon maw-rs --wt issue-N-<slug> -e codex`.
- Build artifacts go to `/tmp/maw-rs-target` (`.cargo/config.toml`), not
  inside the repo — but see the Build Gate above: **you are not building.**
- L1 merges. Wind's machine is not the build farm, and now it is not the
  test farm either.

## Branches

- `main` — the only development branch. All work here, direct push.
- `alpha` — upstream sync only (`upstream/alpha` from Soul-Brews-Studio).
  Used for fetching upstream changes, not for development.

## Releases (CalVer)

Version scheme (day-based CalVer, decided 2026-07-05; matches `maw-calver`'s
`compute_version()`):

```
stable:  v<YY>.<M>.<DD>                 one per day
alpha:   v<YY>.<M>.<DD>-alpha.<HMM>     HMM = H×100+M, TZ=Bangkok
beta:    v<YY>.<M>.<DD>-beta.<HMM>      independent channel
```

`HMM` is wall-clock time as a decimal integer with no leading zero (18:30 →
`1830`, 09:05 → `905`). Every minute is a unique slot — no merge-order
collisions. If `HMM` ≤ the highest existing suffix for the same base+channel,
the crate advances to the next calendar day (`next_calendar_base`).

Transition note: before 2026-07-05 the last number was a per-month release
*sequence* (SEQ-era `v26.7.2`–`v26.7.7`). Those tags were retired on
2026-07-05 (notes archived in the vault, commits untouched) and the current
line restarted day-based at `v26.7.5` (= 2026-07-05, same commit as SEQ-era
v26.7.7). The exact commit and build time are embedded in the binary
(`maw --version`) regardless of scheme.

Cut flow **on this fork**: `main` is the only branch `origin` has, and every PR
targets it (merged PRs #104/#106/#108/#111/#112 are all `base=main`). A release
tags `v<YY>.<M>.<DD>` (stable) or `v<YY>.<M>.<DD>-alpha.<HMM>` (pre-release) off
`main` and publishes a GitHub release. `Fixes #N` auto-closes normally, since
`main` *is* the default branch here.

`alpha` is **not** a branch on this fork — it exists only as `upstream/alpha` on
the read-only `Soul-Brews-Studio` fork-sync remote, where it genuinely is the
trunk (upstream squash-merges into `alpha` and promotes `alpha` → `main`). This
paragraph described that upstream flow, inherited verbatim at fork time. Pass
`gh --repo deachawatss/maw-rs` explicitly: with an `upstream` remote and no
default set, a bare `gh pr list` reports upstream's `base=alpha` PRs and reads
like confirmation.

macOS install note: copying a new binary over an installed one can SIGKILL on
next run (stale code-sign cache on the reused inode) — `rm` first, then `cp`.

## Architecture

Layered Cargo workspace:

- **Leaf crates** — self-contained, deterministic, side-effect-free core
  logic (matching, routing, identity, transport, plugin manifest, …) with no
  internal dependencies.
- **Mid crates** — compose the leaf crates (e.g. `maw-peer`, `maw-tmux`,
  `maw-worktree`).
- **Top crate** — `maw-cli`, the binary, depends on the rest of the workspace.

Run `cargo tree` for the current, authoritative dependency graph.

## Conventions

- `forbid(unsafe_code)`, clippy pedantic clean.
- Rust edition 2021.
- Behavior is validated against maw-js JSON test fixtures.
- Core crates stay deterministic and side-effect-free.
- Recursive search in Bash: always `rg` (ripgrep), never bare `grep -rn` —
  it's parallel and skips `.gitignore`/`target/` automatically. Filter with
  `rg -g '*.rs' PATTERN`; add `-u` for gitignored files. Never sweep
  `/opt/Code` with `grep -rn`. (Claude Code's Grep tool already uses ripgrep;
  this rule is for hand-written Bash.)

## Fleet Intelligence Principles

Oracle intelligence = engine × written memory × asking the right peer.

1. **SEARCH-FIRST** — before guessing, search the vault / oracle MCP, or
   `maw hey` the oracle that has actually hit the problem.
2. **WRITE-BACK** — solved something hard? Write the manual/skill immediately.
   Unwritten knowledge dies at compact; your manual is the next oracle's way out.
3. **VERIFY-DONE** — never mark done without running it; dogfood your own tools.
4. **DONE-CRITERIA TEACHING** — dispatch work with explicit gates (tests green,
   files ≤250). Clear criteria teach the receiver to own the loop.
5. **HUMILITY-COMPOUND** — model tiers change monthly; the vault compounds
   forever. The smartest oracle is the one whose peers never relearn a lesson.
6. **TEACH-DONT-EDIT** — when helping another oracle, teach and hand over the
   commands; never edit a peer's repo yourself.

## Further Docs

See `docs/` for deeper references — including the parity matrix, wire
protocol, "adding a command" guide, agent/coder team spawn conventions, and
the WASM migration design. Shipped fleet plugin artifacts (WASM ship tier,
sha256 pin lifecycle) live in `fleet-plugins/` — see its README.
