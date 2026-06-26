use maw_cli::{dispatcher_status, DispatchKind};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn update_temp_dir(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "maw-rs-native-update-{label}-{}-{nonce}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("bin")).expect("bin");
    fs::create_dir_all(root.join("config/maw")).expect("config");
    fs::create_dir_all(root.join("state")).expect("state");
    root
}

fn update_chmod_exec(path: &Path) {
    let mut perms = fs::metadata(path).expect("metadata").permissions();
    perms.set_mode(0o700);
    fs::set_permissions(path, perms).expect("chmod");
}

fn update_write_fake_marker(bin_dir: &Path, name: &str, marker: &str) {
    let path = bin_dir.join(name);
    fs::write(&path, format!("#!/bin/sh\necho '{marker} $*'\nexit 0\n")).expect("marker");
    update_chmod_exec(&path);
}

fn update_run(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_maw-rs"))
        .args(args)
        .env_clear()
        .env("PATH", root.join("bin"))
        .env("HOME", root)
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_STATE_HOME", root.join("state"))
        .env("MAW_CONFIG_DIR", root.join("config/maw"))
        .env("MAW_JS_REF_DIR", "/nonexistent")
        .env("CARGO_TERM_COLOR", "never")
        .output()
        .expect("run update")
}

fn update_assert_no_delegation(output: &std::process::Output) {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stdout.contains("DELEGATED-MAW"), "stdout={stdout}");
    assert!(!stderr.contains("DELEGATED-MAW"), "stderr={stderr}");
    assert!(!stdout.contains("DELEGATED-BUN"), "stdout={stdout}");
    assert!(!stderr.contains("DELEGATED-BUN"), "stderr={stderr}");
}

#[test]
fn update_upgrade_runtime_fake_maw_no_delegate_proof() {
    assert_eq!(dispatcher_status("update"), DispatchKind::Native);
    assert_eq!(dispatcher_status("upgrade"), DispatchKind::Native);
    let root = update_temp_dir("runtime-proof");
    let bin_dir = root.join("bin");
    update_write_fake_marker(&bin_dir, "maw", "DELEGATED-MAW");
    update_write_fake_marker(&bin_dir, "bun", "DELEGATED-BUN");

    let update = update_run(&root, &["update", "--yes"]);
    assert_eq!(
        update.status.code(),
        Some(1),
        "stderr={}",
        String::from_utf8_lossy(&update.stderr)
    );
    update_assert_no_delegation(&update);
    assert!(
        update.stdout.is_empty(),
        "stdout={}",
        String::from_utf8_lossy(&update.stdout)
    );
    assert_eq!(
        String::from_utf8_lossy(&update.stderr),
        include_str!("fixtures/native-update-upgrade/update-main.stderr")
    );

    let upgrade = update_run(&root, &["upgrade", "alpha", "--yes"]);
    assert_eq!(
        upgrade.status.code(),
        Some(1),
        "stderr={}",
        String::from_utf8_lossy(&upgrade.stderr)
    );
    update_assert_no_delegation(&upgrade);
    assert!(
        upgrade.stdout.is_empty(),
        "stdout={}",
        String::from_utf8_lossy(&upgrade.stdout)
    );
    assert_eq!(
        String::from_utf8_lossy(&upgrade.stderr),
        include_str!("fixtures/native-update-upgrade/upgrade-alpha.stderr")
    );
    let _ = fs::remove_dir_all(root);
}
