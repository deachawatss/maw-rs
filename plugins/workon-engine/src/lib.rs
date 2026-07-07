//! Reference gate plugin — demonstrates the `hooks.gate` CLI-interception
//! contract wired in `crates/maw-cli/src/core_impl/dispatcher.rs::run_plugin_gate`.
//!
//! A gate plugin declares `"hooks": { "gate": ["cmd:<verb>"] }` in plugin.json.
//! Before the native handler for `<verb>` runs, the dispatcher invokes this
//! plugin with the full argv and inspects the returned `InvokeOutput`:
//!
//!   * `ok: false`                 → block the command (exit 1, stderr = error)
//!   * `ok: true`  + `output`      → replace the command (exit 0, stdout = output)
//!   * `ok: true`  + no `output`   → pass through to the native handler
//!
//! NOTE: engine resolution for `maw workon` (`-e/--engine/--codex/--claude`,
//! trust warnings, `.maw/strategy.json` recording) is owned NATIVELY in
//! `crates/maw-cli/src/{wind,core_impl}/workon.rs`. This plugin therefore PASSES
//! THROUGH real workon invocations and exists only as the canonical, testable
//! example of the gate contract. It reacts to explicit demo flags so all three
//! outcomes can be exercised without affecting normal `workon` usage.

use extism_pdk::*;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct InvokeInput {
    #[allow(dead_code)]
    source: String,
    args: Vec<String>,
}

#[derive(Serialize)]
struct InvokeOutput {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl InvokeOutput {
    fn passthrough() -> Self {
        Self {
            ok: true,
            output: None,
            error: None,
        }
    }
    fn replace(output: String) -> Self {
        Self {
            ok: true,
            output: Some(output),
            error: None,
        }
    }
    fn block(error: String) -> Self {
        Self {
            ok: false,
            output: None,
            error: Some(error),
        }
    }
}

#[plugin_fn]
pub fn handle(input: String) -> FnResult<String> {
    let ctx: InvokeInput =
        serde_json::from_str(&input).map_err(|e| extism_pdk::Error::msg(e.to_string()))?;

    let out = if ctx.args.iter().any(|a| a == "--gate-demo-block") {
        InvokeOutput::block("workon-engine: blocked by gate demo\n".to_owned())
    } else if ctx.args.iter().any(|a| a == "--gate-demo-replace") {
        InvokeOutput::replace("workon-engine: replaced by gate demo\n".to_owned())
    } else {
        // Real workon usage: defer to the native handler.
        InvokeOutput::passthrough()
    };

    Ok(serde_json::to_string(&out)?)
}
