# maw-rs Domain Glossary

**fork patch**: A behavioral fix maintained in our fork of maw-js that is not upstream. Must be ported to maw-rs as hardening on existing command surfaces.
**readiness gate**: Pre-send polling that confirms a tmux pane's agent is ready to receive input (shows a prompt, not mid-response). _Avoid_: readiness check, ready check.
**busy guard**: Pre-send check that blocks text delivery when the target pane's agent is actively producing output. _Avoid_: activity check.
**ψ-rescue**: Before worktree removal, copy uncommitted files from the worktree's `ψ/` directory to the main repo's `ψ/` — never overwriting existing files. _Avoid_: psi backup, memory save.
**caller-pane anchor**: Reading `$TMUX_PANE` to target split-window at the caller's pane, preventing workers from spraying into the wrong tmux window. _Avoid_: pane targeting.
**orphan-pane sweep**: Detecting team-spawned tmux panes whose process has died, marking them as zombies for cleanup. _Avoid_: dead pane detection.
**engine resolution**: The fallback chain that determines which AI engine command runs in a new workon window: per-agent config → default config → "claude". _Avoid_: engine detection.
**falsification test**: A regression test designed so that reverting the patched behavior makes the test RED. Proves the test actually exercises the patch, not just passes trivially. _Avoid_: regression test (too generic).
