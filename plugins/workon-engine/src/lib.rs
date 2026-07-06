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

fn has_engine_flag(args: &[String]) -> Option<String> {
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--engine" | "-e" => return args.get(i + 1).cloned(),
            "--codex" => return Some("codex".to_owned()),
            "--claude" => return Some("claude".to_owned()),
            _ => {}
        }
        i += 1;
    }
    None
}

#[plugin_fn]
pub fn handle(input: String) -> FnResult<String> {
    let ctx: InvokeInput = serde_json::from_str(&input)
        .map_err(|e| extism_pdk::Error::msg(e.to_string()))?;

    let sub_args = if !ctx.args.is_empty() && ctx.args[0] == "workon" {
        &ctx.args[1..]
    } else {
        &ctx.args[..]
    };

    let engine = match has_engine_flag(sub_args) {
        Some(e) => e,
        None => {
            return Ok(serde_json::to_string(&InvokeOutput {
                ok: true,
                output: None,
                error: None,
            })?);
        }
    };

    let output = format!(
        "\x1b[36mworkon-engine\x1b[0m: intercepted with engine='{}'\n\
         TODO: implement full workon with engine override via host functions\n",
        engine
    );

    Ok(serde_json::to_string(&InvokeOutput {
        ok: true,
        output: Some(output),
        error: None,
    })?)
}
