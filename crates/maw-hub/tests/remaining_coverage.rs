use maw_hub::{load_workspace_configs, workspaces_dir};
use std::{
    fs,
    time::{SystemTime, UNIX_EPOCH},
};

fn temp_dir(name: &str) -> std::path::PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("maw-hub-{name}-{nonce}"))
}

#[test]
fn valid_shape_with_non_string_agent_reports_deserialize_warning() {
    let dir = temp_dir("bad-agent");
    let workspaces = workspaces_dir(&dir);
    fs::create_dir_all(&workspaces).expect("create workspaces dir");
    fs::write(
        workspaces.join("bad.json"),
        r#"{"id":"one","hubUrl":"wss://hub.example","token":"secret","sharedAgents":[1]}"#,
    )
    .expect("write config");

    let report = load_workspace_configs(&dir).expect("load configs");

    assert!(report.configs.is_empty());
    assert_eq!(report.warnings.len(), 1);
    assert!(report.warnings[0].contains("failed to parse workspace config: bad.json"));
    let _ = fs::remove_dir_all(dir);
}
