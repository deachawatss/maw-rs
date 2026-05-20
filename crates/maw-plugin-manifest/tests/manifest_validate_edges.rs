use maw_plugin_manifest::{
    parse_api, parse_cli, parse_cron, parse_engine, parse_hooks, parse_module, parse_transport,
    ApiMethod, CliFlagKind, HookPolicy,
};
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

#[test]
fn parse_cron_module_and_transport_reject_malformed_sections() {
    assert_eq!(parse_cron(&json!({})).expect("missing cron is valid"), None);
    let cron = parse_cron(&json!({ "cron": { "schedule": "* * * * *", "handler": "tick" } }))
        .expect("valid cron")
        .expect("cron present");
    assert_eq!(cron.schedule, "* * * * *");
    assert_eq!(cron.handler, Some("tick".to_owned()));
    expect_cron_error(
        &json!({ "cron": [] }),
        "plugin.json: cron must be an object",
    );
    expect_cron_error(
        &json!({ "cron": { "schedule": "" } }),
        "plugin.json: cron.schedule must be a non-empty string",
    );
    expect_cron_error(
        &json!({ "cron": { "schedule": "* * * * *", "handler": 1 } }),
        "plugin.json: cron.handler must be a string",
    );

    assert_eq!(
        parse_module(&json!({})).expect("missing module is valid"),
        None
    );
    let module = parse_module(&json!({ "module": { "exports": ["thing"], "path": "./mod.ts" } }))
        .expect("valid module")
        .expect("module present");
    assert_eq!(module.exports, vec!["thing".to_owned()]);
    assert_eq!(module.path, "./mod.ts");
    expect_module_error(
        &json!({ "module": [] }),
        "plugin.json: module must be an object",
    );
    expect_module_error(
        &json!({ "module": { "exports": [], "path": "./mod.ts" } }),
        "plugin.json: module.exports must be a non-empty array of strings",
    );
    expect_module_error(
        &json!({ "module": { "exports": ["thing"], "path": "" } }),
        "plugin.json: module.path must be a non-empty string",
    );

    assert_eq!(
        parse_transport(&json!({})).expect("missing transport is valid"),
        None
    );
    let transport = parse_transport(&json!({ "transport": { "peer": false } }))
        .expect("valid transport")
        .expect("transport present");
    assert_eq!(transport.peer, Some(false));
    expect_transport_error(
        &json!({ "transport": [] }),
        "plugin.json: transport must be an object",
    );
    expect_transport_error(
        &json!({ "transport": { "peer": "yes" } }),
        "plugin.json: transport.peer must be a boolean",
    );
}

#[test]
fn parse_engine_rejects_malformed_serve_process_metadata() {
    assert_eq!(
        parse_engine(&json!({})).expect("missing engine is valid"),
        None
    );
    assert_eq!(
        parse_engine(&json!({ "engine": {} }))
            .expect("valid empty engine")
            .expect("engine present")
            .serve,
        None
    );

    let engine = parse_engine(&json!({
        "engine": {
            "serve": {
                "command": "bun run serve",
                "prefix": "/api/demo",
                "health": "/health",
                "events": ["MessageSend"],
                "eventPath": "/events"
            }
        }
    }))
    .expect("valid engine")
    .expect("engine present");
    let serve = engine.serve.expect("serve present");
    assert_eq!(serve.command, Some("bun run serve".to_owned()));
    assert_eq!(serve.prefix, Some("/api/demo".to_owned()));
    assert_eq!(serve.health, Some("/health".to_owned()));
    assert_eq!(serve.events, Some(vec!["MessageSend".to_owned()]));
    assert_eq!(serve.event_path, Some("/events".to_owned()));

    expect_engine_error(
        &json!({ "engine": [] }),
        "plugin.json: engine must be an object",
    );
    expect_engine_error(
        &json!({ "engine": { "serve": [] } }),
        "plugin.json: engine.serve must be an object",
    );
    expect_engine_error(
        &json!({ "engine": { "serve": { "command": "" } } }),
        "plugin.json: engine.serve.command must be a non-empty string",
    );
    expect_engine_error(
        &json!({ "engine": { "serve": { "prefix": "/demo" } } }),
        "plugin.json: engine.serve.prefix must start with /api/",
    );
    expect_engine_error(
        &json!({ "engine": { "serve": { "health": "health" } } }),
        "plugin.json: engine.serve.health must be an absolute path",
    );
    expect_engine_error(
        &json!({ "engine": { "serve": { "eventPath": "events" } } }),
        "plugin.json: engine.serve.eventPath must be an absolute path",
    );
    expect_engine_error(
        &json!({ "engine": { "serve": { "events": [""] } } }),
        "plugin.json: engine.serve.events must be an array of non-empty strings",
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

fn expect_cron_error(input: &serde_json::Value, expected: &str) {
    let error = parse_cron(input).expect_err("expected parse_cron error");
    assert!(
        error.contains(expected),
        "{error:?} did not contain {expected:?}"
    );
}

fn expect_module_error(input: &serde_json::Value, expected: &str) {
    let error = parse_module(input).expect_err("expected parse_module error");
    assert!(
        error.contains(expected),
        "{error:?} did not contain {expected:?}"
    );
}

fn expect_transport_error(input: &serde_json::Value, expected: &str) {
    let error = parse_transport(input).expect_err("expected parse_transport error");
    assert!(
        error.contains(expected),
        "{error:?} did not contain {expected:?}"
    );
}

fn expect_engine_error(input: &serde_json::Value, expected: &str) {
    let error = parse_engine(input).expect_err("expected parse_engine error");
    assert!(
        error.contains(expected),
        "{error:?} did not contain {expected:?}"
    );
}
