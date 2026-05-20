use maw_plugin_manifest::{parse_api, parse_cli, parse_hooks, ApiMethod, CliFlagKind, HookPolicy};
use serde_json::json;

#[test]
fn parse_cli_rejects_malformed_cli_shapes_and_preserves_optional_fields() {
    assert_eq!(parse_cli(&json!({})).expect("missing cli is valid"), None);

    let parsed = parse_cli(&json!({
        "cli": {
            "command": "demo",
            "aliases": ["d"],
            "help": "hi",
            "flags": { "verbose": "boolean" }
        }
    }))
    .expect("valid cli")
    .expect("cli present");
    assert_eq!(parsed.command, "demo");
    assert_eq!(parsed.aliases, Some(vec!["d".to_owned()]));
    assert_eq!(parsed.help, Some("hi".to_owned()));
    assert_eq!(
        parsed.flags.expect("flags present").get("verbose"),
        Some(&CliFlagKind::Boolean)
    );

    expect_error(&json!({ "cli": [] }), "plugin.json: cli must be an object");
    expect_error(
        &json!({ "cli": { "command": "" } }),
        "plugin.json: cli.command must be a non-empty string",
    );
    expect_error(
        &json!({ "cli": { "command": "x", "aliases": [1] } }),
        "plugin.json: cli.aliases must be an array of strings",
    );
    expect_error(
        &json!({ "cli": { "command": "x", "flags": [] } }),
        "plugin.json: cli.flags must be an object",
    );
    expect_error(
        &json!({ "cli": { "command": "x", "flags": { "bad": "object" } } }),
        "plugin.json: cli.flags[\"bad\"] must be \"boolean\", \"string\", or \"number\"",
    );
}

#[test]
fn parse_api_rejects_malformed_api_objects() {
    assert_eq!(parse_api(&json!({})).expect("missing api is valid"), None);

    let parsed = parse_api(&json!({
        "api": { "path": "/api/demo", "methods": ["GET", "POST"] }
    }))
    .expect("valid api")
    .expect("api present");
    assert_eq!(parsed.path, "/api/demo");
    assert_eq!(parsed.methods, vec![ApiMethod::Get, ApiMethod::Post]);

    assert_eq!(ApiMethod::Get.as_str(), "GET");
    assert_eq!(CliFlagKind::Number.as_str(), "number");

    expect_api_error(&json!({ "api": [] }), "plugin.json: api must be an object");
    expect_api_error(
        &json!({ "api": { "path": "", "methods": ["GET"] } }),
        "plugin.json: api.path must be a non-empty string",
    );
    expect_api_error(
        &json!({ "api": { "path": "/api/demo", "methods": ["PUT"] } }),
        "plugin.json: api.methods must be an array",
    );
}

#[test]
fn parse_hooks_validates_lifecycle_hook_branches() {
    assert_eq!(
        parse_hooks(&json!({})).expect("missing hooks is valid"),
        None
    );

    let parsed = parse_hooks(&json!({
        "hooks": {
            "wake": { "script": "wake.ts", "handler": "onWake", "ensures": ["db"], "policy": "best-effort" },
            "sleep": {},
            "serve": {}
        }
    }))
    .expect("valid hooks")
    .expect("hooks present");
    let wake = parsed.wake.expect("wake present");
    assert_eq!(wake.script, Some("wake.ts".to_owned()));
    assert_eq!(wake.handler, Some("onWake".to_owned()));
    assert_eq!(wake.ensures, Some(vec!["db".to_owned()]));
    assert_eq!(wake.policy, Some(HookPolicy::BestEffort));
    assert_eq!(HookPolicy::FailFast.as_str(), "fail-fast");
    assert!(parsed.sleep.is_some());
    assert!(parsed.serve.is_some());

    expect_hooks_error(
        &json!({ "hooks": { "wake": [] } }),
        "plugin.json: hooks.wake must be an object",
    );
    expect_hooks_error(
        &json!({ "hooks": { "wake": { "script": "" } } }),
        "plugin.json: hooks.wake.script must be a non-empty string",
    );
    expect_hooks_error(
        &json!({ "hooks": { "sleep": { "handler": "" } } }),
        "plugin.json: hooks.sleep.handler must be a non-empty string",
    );
    expect_hooks_error(
        &json!({ "hooks": { "serve": { "ensures": [""] } } }),
        "plugin.json: hooks.serve.ensures must be an array of non-empty strings",
    );
    expect_hooks_error(
        &json!({ "hooks": { "wake": { "policy": "hard" } } }),
        "plugin.json: hooks.wake.policy must be",
    );
    expect_hooks_error(
        &json!({ "hooks": [] }),
        "plugin.json: hooks must be an object",
    );
    expect_hooks_error(
        &json!({ "hooks": { "on": [1] } }),
        "plugin.json: hooks.on must be an array of strings",
    );
    expect_hooks_error(
        &json!({ "hooks": { "gate": [1] } }),
        "plugin.json: hooks.gate must be an array of strings",
    );
    expect_hooks_error(
        &json!({ "hooks": { "filter": "not-array" } }),
        "plugin.json: hooks.filter must be an array of strings",
    );
}

fn expect_error(input: &serde_json::Value, expected: &str) {
    let error = parse_cli(input).expect_err("expected parse_cli error");
    assert!(
        error.contains(expected),
        "{error:?} did not contain {expected:?}"
    );
}

fn expect_api_error(input: &serde_json::Value, expected: &str) {
    let error = parse_api(input).expect_err("expected parse_api error");
    assert!(
        error.contains(expected),
        "{error:?} did not contain {expected:?}"
    );
}

fn expect_hooks_error(input: &serde_json::Value, expected: &str) {
    let error = parse_hooks(input).expect_err("expected parse_hooks error");
    assert!(
        error.contains(expected),
        "{error:?} did not contain {expected:?}"
    );
}
