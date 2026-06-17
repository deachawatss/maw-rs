# maw-rs

Rust port of maw-js — distributed terminal multiplexing & fleet management.
21 crates in a Cargo workspace, 99.85% test coverage, BUSL-1.1 licensed.

## Build Gate

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Both must pass before any PR. No file > 250 lines.

## Branches

- `main` — stable, protected. Never push/merge directly.
- `alpha` — integration branch. All PRs target alpha.
- `agents/*` — codex coder worktree branches.

## Team Profiles

Charter files in `ψ/teams/`:

| Profile | Coders | Engine | Usage |
|---------|--------|--------|-------|
| `team-codex3` | 3 | codex (headless) | `maw team up team-codex3` |
| `team-codex5` | 5 | codex | `maw team up team-codex5` |
| `team-omx5` | 5 | omx | `maw team up team-omx5` |

### Headless Mode (codex exec)

Fire-and-forget dispatch — no TUI, coders `maw hey` back on start/done/blocked:

```bash
CODEX_HOME=~/.codex-team/N codex exec \
  --dangerously-bypass-approvals-and-sandbox \
  -c model="gpt-5.5" \
  "TASK: <description>
   First: maw hey 182-maw-rs:maw-rs 'codex-N starting — <task>'
   Done: maw hey 182-maw-rs:maw-rs 'codex-N done — <task> PR#N'
   Blocked: maw hey 182-maw-rs:maw-rs 'codex-N blocked — <reason>'" \
  --json &
```

### CODEX_HOME Map

| Slot | Path | Used by |
|------|------|---------|
| 1 | `~/.codex-team/1` | codex-1 |
| 2 | `~/.codex-team/2` | (spare) |
| 3 | `~/.codex-team/3` | codex-2 |
| 4 | `~/.codex-team/4` | (spare) |
| 5 | `~/.codex-team/5` | codex-3 |

All slots pre-trusted for this repo.

### Available Codex Models

| Model | Flag | Reasoning | Use for |
|-------|------|-----------|---------|
| `gpt-5.5` | `-m gpt-5.5` | No | Default — fast, good for crate porting |
| `o3` | `-m o3` | Yes | Complex architecture, cross-crate refactor |
| `o4-mini` | `-m o4-mini` | Yes | Cheaper reasoning, simple logic tasks |
| `codex-mini` | `-m codex-mini` | No | Fastest, repetitive tasks |

Reasoning effort: `-c model_reasoning_effort="low|medium|high"`

### Headless Dispatch Template

```bash
# Fire-and-forget with maw hey callback
CODEX_HOME=~/.codex-team/N codex exec \
  --dangerously-bypass-approvals-and-sandbox \
  -m gpt-5.5 \
  -c model_reasoning_effort="low" \
  "TASK: <description>
   REPO: /opt/Code/github.com/Soul-Brews-Studio/maw-rs
   PROTOCOL:
     1. maw hey 182-maw-rs:maw-rs 'codex-N starting — <task>'
     2. implement → cargo build → cargo test → cargo clippy → fix → repeat
     3. git add + commit + gh pr create --base alpha
     4. maw hey 182-maw-rs:maw-rs 'codex-N done — <task> PR#N'
     5. If blocked: maw hey 182-maw-rs:maw-rs 'codex-N blocked — <reason>'" \
  --json &
```

Override model per task:
- Leaf crate port: `-m gpt-5.5` (fast, repetitive)
- Complex crate: `-m o3 -c model_reasoning_effort="high"` (reasoning)
- Quick fix: `-m o4-mini` (cheap)

### Skill: `/oracle-team`

```
/oracle-team up [profile]     # spawn (default: first yaml in ψ/teams/)
/oracle-team down [1,2,3]     # safe teardown (partial or all)
/oracle-team lead             # peek → merge → dispatch → nudge
/oracle-team status           # read-only peek
```

Loop mode: `/loop 5m /oracle-team lead`

## Crate Dependency Graph

```
Wave 1 (leaf, 17 crates — no internal deps):
  maw-auth, maw-auto-wake, maw-bind, maw-bring, maw-calver,
  maw-feed, maw-fuzzy, maw-hub, maw-identity, maw-matcher,
  maw-plugin-manifest, maw-plugin-scaffold, maw-policy,
  maw-routing, maw-split, maw-transport, maw-xdg

Wave 2 (mid, 3 crates):
  maw-peer (→ maw-xdg)
  maw-tmux (→ maw-matcher, maw-peer)
  maw-worktree (→ maw-matcher)

Wave 3 (top, 1 crate):
  maw-cli (→ all 20 crates)
```

## Conventions

- `forbid(unsafe_code)`, clippy pedantic
- Edition 2021
- JSON fixture validation from maw-js test specs
- Deterministic, side-effect-free core crates (Phase 1)
