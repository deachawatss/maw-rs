use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

use maw_cli::{dispatcher_status, DispatchKind};

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_maw-rs"))
}

fn temp_dir(name: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("maw-rs-config-{name}-{stamp}"));
    fs::create_dir_all(&path).expect("temp dir");
    path
}

fn seed_config(root: &Path) -> PathBuf {
    let config = root.join("config");
    fs::create_dir_all(&config).expect("config dir");
    fs::write(
        config.join("maw.config.json"),
        "{\n  \"node\": \"old-node\",\n  \"port\": 3456,\n  \"env\": {\n    \"SECRET\": \"raw-secret-never-print\"\n  }\n}\n",
    )
    .expect("config");
    config
}

fn fake_maw_path(root: &Path) -> PathBuf {
    let bin_dir = root.join("fake-bin");
    fs::create_dir_all(&bin_dir).expect("fake bin");
    let maw = bin_dir.join("maw");
    fs::write(&maw, "#!/bin/sh\necho DELEGATED-MAW \"$@\"\nexit 99\n").expect("fake maw");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&maw, fs::Permissions::from_mode(0o755)).expect("chmod fake maw");
    }
    bin_dir
}

fn run_config(root: &Path, args: &[&str]) -> Output {
    let config = root.join("config");
    let fake_bin = fake_maw_path(root);
    Command::new(bin())
        .args(args)
        .env_clear()
        .env("PATH", fake_bin)
        .env("HOME", root.join("home"))
        .env("MAW_CONFIG_DIR", config)
        .env("MAW_JS_REF_DIR", "/nonexistent")
        .output()
        .expect("run maw-rs")
}

fn assert_no_delegation(output: &Output) {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stdout.contains("DELEGATED-MAW"),
        "stdout delegated: {stdout}"
    );
    assert!(
        !stderr.contains("DELEGATED-MAW"),
        "stderr delegated: {stderr}"
    );
}

fn assert_success_golden(root: &Path, args: &[&str], expected: &str) {
    let output = run_config(root, args);
    assert_no_delegation(&output);
    assert!(
        output.status.success(),
        "stderr={} stdout={}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(String::from_utf8(output.stdout).expect("stdout"), expected);
    assert_eq!(String::from_utf8(output.stderr).expect("stderr"), "");
}

#[test]
fn config_dispatch_is_native() {
    assert_eq!(dispatcher_status("config"), DispatchKind::Native);
}

#[test]
fn config_set_node_and_port_goldens_without_maw_delegation() {
    let root = temp_dir("golden");
    let config = seed_config(&root);
    assert_success_golden(
        &root,
        &["config", "set", "node", "new-node"],
        include_str!("fixtures/native-config/set-node.stdout"),
    );
    assert_success_golden(
        &root,
        &["config", "set", "port", "4567", "--json"],
        include_str!("fixtures/native-config/set-port-json.stdout"),
    );
    let body = fs::read_to_string(config.join("maw.config.json")).expect("config body");
    assert!(body.contains("\"node\": \"new-node\""));
    assert!(body.contains("\"port\": 4567"));
    assert!(body.contains("raw-secret-never-print"));
    assert!(!config.join("maw.config.json.tmp").exists());
}

#[test]
fn config_rejects_bad_node_and_port_before_any_delegation() {
    let root = temp_dir("reject");
    let config = seed_config(&root);
    let output = run_config(&root, &["config", "set", "node", "--bad"]);
    assert_no_delegation(&output);
    assert!(!output.status.success());
    assert!(String::from_utf8(output.stderr)
        .expect("stderr")
        .contains("invalid node"));
    let output = run_config(&root, &["config", "set", "port", "70000"]);
    assert_no_delegation(&output);
    assert!(!output.status.success());
    assert!(String::from_utf8(output.stderr)
        .expect("stderr")
        .contains("invalid port"));
    let body = fs::read_to_string(config.join("maw.config.json")).expect("config body");
    assert!(body.contains("\"node\": \"old-node\""));
    assert!(body.contains("\"port\": 3456"));
}
