# Config Parity Report

## Status
- Done: implementation committed and required validation passed.

## Findings
- maw-js `origin/alpha` discovers `maw.config.<N>.json` and `maw.config.<N>.local.json`, sorts by weight, scope rank, local flag, then path, and deep-merges layers.
- Deep merge semantics: objects recurse, scalars and arrays replace, and `null` deletes an inherited key.
- maw-js also inherits singleton XDG config when `MAW_HOME` is set and `MAW_CONFIG_DIR` is not set, so per-instance config can override the base config.
- maw-rs direct readers currently include the wake/workon engine command paths called out by the mission, plus other config readers under `crates/maw-cli/src/core_impl`.

## Changes
- Added shared layered config discovery and deep merge to `maw-xdg`.
- Added config layer fixtures covering maw-js sort order, recursive object merge, array/scalar replacement, `null` deletion, weighted-only config, and `MAW_HOME` instance override behavior.
- Rewired runtime config readers in `maw-cli` to use the merged config object instead of reading only `maw.config.json`.
- Updated `wake` and `workon` command alias tests to cover weighted-only `maw.config.50.json` command maps.
- Reused the shared discovery/merge implementation for `maw config` diagnostics and changed `maw on` read-modify-write target selection to the existing weighted target when present.

## Validation
- `cargo fmt --check` passed.
- `cargo test -p maw-xdg` passed.
- `cargo test -p maw-cli wake_short_e_flag_and_config_commands_engine_resolution` passed.
- `cargo test -p maw-cli workon_build_command_resolves_weighted_only_commands_config` passed.
- `cargo clippy --all-targets` passed.
- `cargo test -p maw-cli -p maw-xdg` passed.

## Open Questions
- None currently.
