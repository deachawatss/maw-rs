use maw_cli::{dispatcher_status, DispatchKind};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_maw-rs"))
}

fn temp_dir(name: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("maw-rs-native-demo-{name}-{stamp}"));
    fs::create_dir_all(&path).expect("temp dir");
    path
}

fn run(args: &[&str], root: &Path) -> std::process::Output {
    Command::new(bin())
        .args(args)
        .current_dir(root)
        .env("MAW_HOME", root.join("home"))
        .env("HOME", root.join("user-home"))
        .env("GHQ_ROOT", root.join("ghq"))
        .env("MAW_JS_REF_DIR", "/nonexistent")
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .output()
        .expect("run maw-rs")
}

#[test]
fn native_demo_no_tmux_matches_committed_maw_js_golden_without_ref_checkout() {
    let root = temp_dir("no-tmux");
    let output = run(&["demo", "--fast"], &root);

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout"),
        include_str!("fixtures/native-demo/no-tmux.stdout")
    );
    assert_eq!(String::from_utf8(output.stderr).expect("stderr"), "");
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn native_demo_is_registered_and_help_is_offline_temp_home() {
    assert_eq!(dispatcher_status("demo"), DispatchKind::Native);
    let root = temp_dir("help");
    let output = run(&["demo", "--help"], &root);

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout");
    assert!(
        stdout.starts_with("maw demo — simulated multi-agent session\n"),
        "{stdout}"
    );
    assert!(stdout.contains("Usage: maw demo [--fast]"), "{stdout}");
    assert_eq!(String::from_utf8(output.stderr).expect("stderr"), "");
    fs::remove_dir_all(root).expect("cleanup");
}
