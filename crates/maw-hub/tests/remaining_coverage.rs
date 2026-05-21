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

#[test]
fn loader_surfaces_filesystem_errors_for_unusable_workspace_paths() {
    let config_file = temp_dir("config-file");
    fs::write(&config_file, "not a directory").expect("write config path as file");
    let create_err = load_workspace_configs(&config_file).expect_err("file parent blocks mkdir");
    assert!(matches!(
        create_err.kind(),
        std::io::ErrorKind::NotADirectory | std::io::ErrorKind::AlreadyExists
    ));
    let _ = fs::remove_file(&config_file);

    let dir = temp_dir("workspaces-file");
    fs::create_dir_all(&dir).expect("create config dir");
    fs::write(workspaces_dir(&dir), "not a directory").expect("write workspaces as file");
    let read_err = load_workspace_configs(&dir).expect_err("workspaces file blocks read_dir");
    assert!(matches!(
        read_err.kind(),
        std::io::ErrorKind::NotADirectory | std::io::ErrorKind::InvalidInput
    ));
    let _ = fs::remove_dir_all(dir);
}
