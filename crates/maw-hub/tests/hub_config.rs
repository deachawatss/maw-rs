use maw_hub::{
    load_workspace_configs, validate_workspace_config, workspaces_dir, WorkspaceConfig,
    WorkspaceConfigValidation,
};
use serde_json::json;
use std::{
    fs,
    time::{SystemTime, UNIX_EPOCH},
};

fn invalid(reason: &str) -> WorkspaceConfigValidation {
    WorkspaceConfigValidation::Invalid {
        reason: reason.to_owned(),
    }
}

fn temp_config_dir() -> std::path::PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("maw-rs-hub-config-test-{unique}"))
}

#[test]
fn validates_workspace_config_with_maw_js_reasons() {
    assert_eq!(
        validate_workspace_config(&serde_json::Value::Null),
        invalid("not an object")
    );
    assert_eq!(
        validate_workspace_config(
            &json!({ "hubUrl": "ws://hub", "token": "t", "sharedAgents": [] })
        ),
        invalid("missing/empty id")
    );
    assert_eq!(
        validate_workspace_config(&json!({ "id": "ws", "token": "t", "sharedAgents": [] })),
        invalid("missing/empty hubUrl")
    );
    assert_eq!(
        validate_workspace_config(&json!({ "id": "ws", "hubUrl": "ws://hub", "sharedAgents": [] })),
        invalid("missing/empty token")
    );
    assert_eq!(
        validate_workspace_config(&json!({ "id": "ws", "hubUrl": "ws://hub", "token": "t" })),
        invalid("sharedAgents must be array")
    );
    assert_eq!(
        validate_workspace_config(
            &json!({ "id": "ws", "hubUrl": "http://hub", "token": "t", "sharedAgents": [] })
        ),
        invalid("hubUrl must be ws:|wss: (got http:)")
    );
    assert_eq!(
        validate_workspace_config(
            &json!({ "id": "ws", "hubUrl": "not a url", "token": "t", "sharedAgents": [] })
        ),
        invalid("hubUrl not a valid URL")
    );
}

#[test]
fn accepts_ws_and_wss_configs() {
    assert_eq!(
        validate_workspace_config(
            &json!({ "id": "ws", "hubUrl": "ws://hub", "token": "t", "sharedAgents": [] })
        ),
        WorkspaceConfigValidation::Ok
    );
    assert_eq!(
        validate_workspace_config(
            &json!({ "id": "ws", "hubUrl": "wss://hub", "token": "t", "sharedAgents": ["agent"] })
        ),
        WorkspaceConfigValidation::Ok
    );
}

#[test]
fn loader_creates_dir_keeps_valid_configs_and_reports_bad_files() {
    let config_dir = temp_config_dir();
    let report =
        load_workspace_configs(&config_dir).expect("missing workspaces dir should be created");
    assert!(report.configs.is_empty());
    assert!(report.warnings.is_empty());
    let dir = workspaces_dir(&config_dir);
    assert!(dir.exists());

    fs::write(
        dir.join("valid.json"),
        serde_json::to_string(&json!({
            "id": "alpha",
            "hubUrl": "wss://hub.example.test",
            "token": "secret",
            "sharedAgents": ["mawjs"]
        }))
        .expect("valid json should serialize"),
    )
    .expect("valid fixture should write");
    fs::write(
        dir.join("invalid.json"),
        serde_json::to_string(&json!({
            "id": "bad",
            "hubUrl": "https://not-websocket.example.test",
            "token": "secret",
            "sharedAgents": []
        }))
        .expect("invalid json should serialize"),
    )
    .expect("invalid fixture should write");
    fs::write(dir.join("broken.json"), "{not json").expect("broken fixture should write");
    fs::write(dir.join("notes.txt"), "ignored").expect("non-json fixture should write");

    let report = load_workspace_configs(&config_dir).expect("fixtures should load");
    assert_eq!(
        report.configs,
        vec![WorkspaceConfig {
            id: "alpha".to_owned(),
            hub_url: "wss://hub.example.test".to_owned(),
            token: "secret".to_owned(),
            shared_agents: vec!["mawjs".to_owned()],
        }]
    );
    let warnings = report.warnings.join("\n");
    assert!(warnings.contains("invalid workspace config: invalid.json"));
    assert!(warnings.contains("failed to parse workspace config: broken.json"));
}

#[test]
fn warning_includes_invalid_filename_and_reason() {
    let config_dir = temp_config_dir();
    let dir = workspaces_dir(&config_dir);
    fs::create_dir_all(&dir).expect("workspaces dir should create");
    fs::write(
        dir.join("bad.json"),
        serde_json::to_string(
            &json!({ "id": "ws-bad", "hubUrl": "http://hub", "token": "t", "sharedAgents": [] }),
        )
        .expect("bad json should serialize"),
    )
    .expect("bad fixture should write");

    let report = load_workspace_configs(&config_dir).expect("bad fixture should be skipped");
    assert!(report.configs.is_empty());
    assert!(report.warnings.join("\n").contains(
        "[hub] invalid workspace config: bad.json (hubUrl must be ws:|wss: (got http:))"
    ));
}
