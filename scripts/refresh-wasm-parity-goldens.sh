#!/usr/bin/env bash
set -euo pipefail

# Regenerate committed WASM parity golden outputs from the real maw-js checkout.
# Usage:
#   MAW_JS_REF_DIR=/path/to/Soul-Brews-Studio/maw-js scripts/refresh-wasm-parity-goldens.sh
# If MAW_JS_REF_DIR is unset, the maintainer workstation default is used.
: "${MAW_JS_REF_DIR:=/home/agent/github.com/Soul-Brews-Studio/maw-js}"
export MAW_JS_REF_DIR

if [[ ! -d "$MAW_JS_REF_DIR/.git" ]]; then
  echo "MAW_JS_REF_DIR must point at a maw-js git checkout: $MAW_JS_REF_DIR" >&2
  exit 1
fi

cargo test -p maw-plugin-manifest --test wasm_parity_harness \
  generate_wasm_parity_goldens_from_real_maw_js -- --ignored --exact --nocapture
