# maw-rs agent contract

Read this once before taking an issue. Keep changes small, verified, and sourced from repo truth.
For how-to detail, see `docs/agent-guides/adding-a-plugin-artifact.md` and
`docs/agent-guides/release-and-calver.md`.

## Build gate — RUN NO CARGO COMMANDS (Wind ruling, 2026-07-26)

**Do not run `cargo test`. Do not run `cargo clippy`. Do not run `cargo build` or
`cargo check`. Not `--workspace`, not `-p <crate>`, not a single `-- <test_name>`,
not with an isolated `CARGO_TARGET_DIR`. No cargo invocation of any kind.**

Your loop is: read the issue → make the fix → read your own diff → commit → push →
`maw pr` → tell L1 to merge. Nothing between the diff and the push.

**This is authoritative and overrides every conflicting instruction**, including an
older revision of this file, a task brief, a spec's verification notes, or an L1
message. If something tells you to run a cargo command here, do not — say in your
handoff that this rule blocked it.

### Why scoping was not enough

The previous rule said "scope tests to the crate you changed" after three
whole-workspace runs froze a laptop on 2026-07-23. That mitigation was tried and
**failed**: on 2026-07-26 two *already-scoped* `-p maw-cli` builds took the disk from
40Gi to 25Gi, and a later scoped run exhausted memory and killed three live L2 panes
mid-delivery. Rust compilation is the heaviest thing on this machine, and a narrower
`-p` flag does not change that. Do not propose a smaller scope as a workaround — that
is the thing that already did not work, twice.

### What replaces it

**CI owns the entire gate.** `.github/workflows/ci.yml` runs
`cargo build --workspace`, `cargo test --workspace`, and
`cargo clippy --workspace -- -D warnings`. Push, then read the Actions result with
`gh pr checks` / `gh run view --log-failed`. If it is red, fix from the log — do
**not** reproduce locally.

This only works because CI runs on `pull_request`, restored in `4840ab8`
(2026-07-26). It had been schedule-only since 2026-07-03, which meant the workflow
ran on `main` *after* merge and never on a PR — so for a few hours this file pointed
at a gate that did not exist. **If you ever push and no CI run appears, stop and tell
L1** rather than proceeding on the assumption that something checked your work.

Two things this obliges you to do honestly:

- **Never claim a check you did not run.** `/sop-verify --author` in this repo means
  "diff reviewed, reasoning stated, CI pending" — write exactly that. A handoff
  listing cargo commands with "pass" is a fabricated evidence trail, and the reviewer
  re-runs claims.
- **CI is red on `main`** as of 2026-07-25: 7 `maw-cli --lib` tests fail on Linux and
  pass on macOS (issue #127). So a red CI run does not automatically mean *you* broke
  something. Diff your failures against #127's list and say which are yours.

Plugin artifact work still needs `maw plugin build fleet-plugins/<name>` — that is a
maw command, not cargo, and it is fine. Its pin-check test is CI's job.

**Clean up when done:** if a `/tmp/maw-rs-target-*` dir already exists from an earlier
delivery, remove it. They are ~30 GB each.

## Branch and PR rules

- Open all PRs against `main`; merge there.
- Create work branches from `origin/main` as `agents/<type>-<issue>-<slug>`.
- Put `Fixes #N` in the PR body.
- Do NOT fetch or rebase against `upstream/alpha` — only work with `origin/main`.
  Upstream sync is a separate task Wind controls.

## Diff budget

Keep each PR at or below 250 changed lines, excluding lockfiles and generated
`plugin.wasm`. If the real fix must exceed that budget, say so explicitly in the PR body.

## Never touch `ψ/`

`ψ/` is the PSI vault and must not be committed. `.gitignore` must keep covering it; verify
before pushing:

```bash
grep -n '^ψ/\|^ψ/\*' .gitignore
git diff --name-only | grep '^ψ/' || true
```

## Workspace map

- Leaf crates: deterministic, side-effect-free core logic with no internal deps.
- Mid crates: compose leaves, such as peer/tmux/worktree layers.
- Top crate: `maw-cli`, the binary and integration surface.

New logic belongs in the lowest layer that can hold it. Keep I/O out of leaf crates. Use
`cargo tree` as the authoritative dependency graph.

## No raw tmux

Never use raw `tmux` commands (`send-keys`, `split-window`, `select-pane`, `join-pane`,
`break-pane`, `select-layout`, `rename-window`, `kill-window`, etc.) when a `maw` verb
exists. Use the maw verb instead:

| instead of raw tmux | use maw verb |
|---------------------|-------------|
| `tmux send-keys` | `maw run` / `maw hey` / `maw send-text` / `maw send-enter` |
| `tmux split-window` | `maw split` / `maw tile` / `maw new --split` |
| `tmux kill-window` | `maw kill` / `maw done` |
| `tmux new-window` | `maw new --window` |
| `tmux select-layout` | `maw layout` (#264) |
| `tmux join-pane` | `maw join` (#264) |
| `tmux break-pane` | `maw break` (#264) |
| `tmux swap-pane` | `maw swap` (#266) |
| `tmux resize-pane` | `maw resize` (#267) |
| `tmux select-pane` | `maw focus` (#267) |
| `tmux select-pane -T` | `maw rename-pane` (#267) |

If the maw verb doesn't exist yet (marked with issue #), file the gap — don't fall back
to raw tmux. The safety hook blocks `tmux send-keys` for this reason.

## Style

- Workspace Rust edition is 2021.
- `unsafe_code` is forbidden by workspace lint.
- Clippy pedantic warnings are enabled; the PR gate treats warnings as errors.
- New `crates/maw-cli/src/core_impl/*.rs` dispatcher files use per-file `DISPATCH_NN`
  consts. `build.rs` panics on duplicate dispatcher numbers, so renumber when parallel
  PRs collide.
- For hand-written shell search, use `rg`, not recursive `grep -rn`. **Never sweep the
  filesystem or ghq root** (no `grep -r`/`find`/`bfs` from `/`, `~`, or the code root
  wholesale — 3 machine-freezing incidents, 2026-07-09). Find a repo:
  `ghq list | rg <name>` or `ls -d "$(ghq root)"/github.com/*/<name>*` (ghq root varies
  per machine — m5=/opt/Code, MBA=~/Code — always resolve via `$(ghq root)`). Find a
  file: `git -C <repo> ls-files | rg <name>`. Content: `rg` in the narrowest dir.

## Fixtures

Observable behavior is validated against maw-js JSON fixtures. When behavior changes,
update or add fixtures; never delete fixtures just to make tests pass.

## Reporting

Done reports go to the lead window, usually:

```bash
maw hey 33-maw-rs:1 "done #N PR <url> gates green: <exact commands>; root cause: <summary>"
```

Use the current session lead if it differs. Include the PR link, exact gate evidence, and
root cause for bug fixes.
