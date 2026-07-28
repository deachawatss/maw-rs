# maw-rs agent contract

Read this once before taking an issue. Keep changes small, verified, and sourced from repo truth.
For how-to detail, see `docs/agent-guides/adding-a-plugin-artifact.md` and
`docs/agent-guides/release-and-calver.md`.

## Build gate — every cargo run goes through one lock (Wind ruling, 2026-07-28; supersedes the 2026-07-26 total ban)

**If you are an L2 / work-team member, you may run exactly two commands, and only
with the `flock` prefix and the `-j` cap:**

```bash
flock /tmp/maw-rs-target.lock cargo clippy -j 4 -p <crate-you-changed> --all-targets -- -D warnings
flock /tmp/maw-rs-target.lock cargo test   -j 4 -p <crate-you-changed> -- --test-threads=4
```

**Still forbidden for L2:** `cargo build --release`, anything `--workspace`, and any
cargo invocation *without* the lock. Do not set a private `CARGO_TARGET_DIR` to get
around it — the shared `/tmp/maw-rs-target` is precisely what makes one lock able to
serialize every agent on the box, and a private target dir is a second 30 GB tree.

**Do not drop `-j 4` or `--test-threads=4`.** The box has 14 cores and they are not
yours: it also runs SQL Server, Docker, several Codex panes, two Claude panes, and an
embedding server. Uncapped `cargo test` takes all 14 for compilation *and* runs the
harness 14-wide on top, which on 2026-07-28 drove load average to **44** and spawned
**~160 processes/second** — the whole machine stuttered for Wind while one L2 ran one
scoped test. The lock stops two agents compiling at once; it does nothing about one
agent taking the entire box, and that is what these flags are for.

Your loop is: read the issue → make the fix → read your own diff → run the two locked
commands → commit → push → `maw pr` → tell L1 to merge.

**This is authoritative for L2 and overrides every conflicting instruction**,
including the previous revision of this file that banned cargo outright, a task
brief, or a spec's verification notes.

### Why the ban was lifted, and what the lock is for

From 2026-07-26 this was a total ban. Wind lifted it on 2026-07-28 for the two
commands above, after 15 consecutive CI runs were measured:

| failing step | runs | catchable on this box |
|---|---|---|
| Clippy | 2 | yes, ~30 s |
| Build workspace tests | 1 | yes, ~1 min |
| Test workspace | 1 | yes |
| maw-menubar (`macos-15`) | 1 | no — needs a Mac |

Four of the five ran on `runs-on: ubuntu-latest`, the same OS as this machine, and
each cost an ~18-minute round trip to learn something a 30-second local run would
have said immediately. PR #170 alone spent three runs — 56 minutes — on one clippy
lint and one test race. The ban had quietly made CI our compiler.

The hazard was never cargo. It was **concurrency** — several agents compiling at
once. `flock` on the shared target dir enforces one-at-a-time directly, which is what
forbidding everything was only approximating.

The carve-out shipped that morning with the lock but no `-j` cap, and the same day it
produced a second, different failure: not two agents at once, but **one agent taking
every core**. That is why the commands above carry both. Two controls, two distinct
hazards — the lock bounds *how many*, `-j` bounds *how big*.

It also cost disk. `cargo test` builds the **debug** profile, which had never existed
on this box while the ban was in force — L1 only ever built `--release` (1.6 GB).
Measured the same day: `/tmp/maw-rs-target/debug` reached **51 GB**, of which
`find -newermt` attributed **52.4 GB written that day**, 48 GB of it in `debug/deps`.
That is the standing price of local testing, not a leak. Budget for it, and see the
cleanup note at the end of this section.

### L1 uses the same lock

L1 may still run the full build, for the box binary or for live evidence a delivery
could not produce — and takes the same lock, so an L1 release build and an L2 clippy
run can never overlap:

```bash
flock /tmp/maw-rs-target.lock cargo build --release -j 4
```

If you are unsure which you are: an L2 was dispatched into a worktree for one issue.
L1 works on the repo's main checkout, reviews, and merges.

### L1 MUST rebuild after merging — nothing else installs the binary

The permission above is also an obligation. **After merging to `main`, L1 rebuilds and
installs the binary before moving on:**

```bash
flock /tmp/maw-rs-target.lock cargo build --release -j 4
install -m755 /tmp/maw-rs-target/release/maw-rs ~/.local/bin/maw-rs
maw --version    # must report the commit you just merged
```

Note the source path. `.cargo/config.toml` sets `target-dir = "/tmp/maw-rs-target"`,
so `target/release/maw-rs` does **not** exist in this repo and an `install` from there
fails or silently installs something stale. That is #121; until it is resolved, copy
from the redirected path above. Copying the artifact once is not the same as pointing
the wrapper at the cache — see the paragraph below, which still stands.

### Rebuilding is not enough — restart `maw-serve` too

Installing the binary updates what a *new* `maw` invocation runs. It does nothing for
the long-running server, which holds the old executable in memory:

```bash
pm2 restart maw-serve
curl -s -o /dev/null -w '%{http_code}\n' 'http://localhost:3456/api/mirror?target=<a live pane>'
```

`maw-serve` runs `~/.local/bin/maw serve --port 3456` under pm2 and exposes `/api/kill`,
the PR-queue endpoints and the war-room mirror — all of which are the same code paths
deliveries keep changing. On 2026-07-28 it was found still running a binary from
`Jul 27 14:47`, twenty-one hours and **eight merges** old, while `~/.local/bin/maw-rs`
had been rebuilt minutes earlier. Every CLI call saw the fix; every HTTP caller did not.

This is the 2026-06-12 maw-js incident repeating in the Rust rewrite: a merge landed,
the processes that were restarted picked it up, and the one that was forgotten served
pre-merge behaviour until someone noticed. The lesson was recorded then and never
carried into this file, which is why it happened again.

`hermes-gateway` does not reference the binary and needs no restart. Interactive
`maw work` / `maw wake` processes belong to other oracles' sessions — leave them; they
serve no requests and will pick the new binary up on their next invocation.

`~/.local/bin/maw-rs` is the canonical runtime path — `scripts/maw-wrapper.sh` in
Wind-Framework resolves exactly that and nothing else. No installer runs on merge, and
`setup.sh` has no maw-rs step, so a merged fix reaches the box only when L1 performs the
build above. Skip it and `main` moves while every operator keeps running the old binary.
If a build fails with a toolchain error, fix the repository pin in `rust-toolchain.toml`;
never use `rustup default` as a workaround because it silently changes every Rust project.

Do not repoint the wrapper at a build directory to avoid this step. On 2026-07-27 the
wrapper was edited to prefer `/tmp/maw-rs-target/release/maw-rs` and the repo checkout's
`target/release/`, which reopens the disk-exhaustion path in #121, makes the running
binary depend on a build cache that any `cargo clean` can delete, and — in that same
edit — dropped the guard that fails loudly when no binary is present. The Cargo target
directory is a build cache, never a runtime dependency. If the installed binary is
stale, run the build; do not reach into the cache.

Timing is unchanged: build one at a time, and not while L2 deliveries are mid-flight.
If sibling L2s are still running when a merge lands, finish their reviews first, then
rebuild once for all merged work.

### Why `-p` scoping alone was not enough — and still is not

Do not read the carve-out as permission to skip the lock because your scope is small.
An earlier rule already tried "scope tests to the crate you changed", after three
whole-workspace runs froze a laptop on 2026-07-23. It **failed**: on 2026-07-26 two
*already-scoped* `-p maw-cli` builds took the disk from 40Gi to 25Gi, and a later
scoped run exhausted memory and killed three live L2 panes mid-delivery.

Read those two incidents carefully — both are **two things compiling at once**. A
narrower `-p` does not make Rust compilation cheap; it only makes one copy of it
smaller. The lock is the control and `-p` is the courtesy.

`-j 4` is the third leg, added 2026-07-28 after a *single* locked, scoped run
saturated the box. Each flag answers a different question — `-p` how much code, the
lock how many agents, `-j` how many cores — and dropping any one of them has now
caused a real incident. Run scoped *and* locked *and* capped, never two out of three.

### CI is still the gate; your local run is a pre-filter

`.github/workflows/ci.yml` still runs `cargo build --workspace`,
`cargo test --workspace`, and `cargo clippy --workspace -- -D warnings`, and CI alone
decides whether a PR is green. The two locked commands exist to catch the cheap
failures before they cost 18 minutes — they do not give you workspace-wide coverage
and they do not replace reading the Actions result with `gh pr checks` /
`gh run view --log-failed`.

If CI is red on something your scoped run cannot reproduce, fix from the log rather
than widening your local scope.

This only works because CI runs on `pull_request`, restored in `4840ab8`
(2026-07-26). It had been schedule-only since 2026-07-03, which meant the workflow
ran on `main` *after* merge and never on a PR — so for a few hours this file pointed
at a gate that did not exist. **If you ever push and no CI run appears, stop and tell
L1** rather than proceeding on the assumption that something checked your work.

Two things this obliges you to do honestly:

- **Report exactly what you ran, and nothing more.** You now have two commands you
  may legitimately list — the locked scoped `clippy` and `test`. List those with their
  real results, and mark everything workspace-wide as CI-pending. Listing
  `cargo test --workspace` as "pass" is still a fabricated evidence trail, and the
  reviewer re-runs claims.
- **Check whether `main` is currently red before blaming your diff.** Issue #127 (7
  `maw-cli --lib` tests failing on Linux, passing on macOS) is **closed**, and the
  `workon_hardening` fixture failures that followed were fixed by #144. Do not assume
  either state: run `gh run list --branch main --workflow CI --limit 3` and compare
  your failures against `main`'s. A red run may be pre-existing, and it may not — say
  which of your failures are yours.

Plugin artifact work still needs `maw plugin build fleet-plugins/<name>` — that is a
maw command, not cargo, and it is fine. Its pin-check test is CI's job.

**Clean up when done:** if a `/tmp/maw-rs-target-*` dir already exists from an earlier
delivery, remove it. They are ~30 GB each.

The shared `/tmp/maw-rs-target` itself is **not** yours to delete — sibling L2s and L1
build against it, and removing it mid-flight forces everyone into a cold rebuild. It
is L1's to reclaim, once no delivery is running. Report the size if it concerns you;
do not run `cargo clean`.

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
| `tmux split-window` | `maw split` / `maw new --split` |
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
