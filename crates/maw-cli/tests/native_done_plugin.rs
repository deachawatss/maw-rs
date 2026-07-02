use maw_cli::{dispatcher_status, DispatchKind};
use std::path::{Path, PathBuf};
use std::process::Command;

fn done_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_maw-rs"))
}

fn done_write(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("parent dir");
    }
    std::fs::write(path, text).expect("write file");
}

fn done_chmod(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(path).expect("metadata").permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions).expect("chmod");
    }
}

fn done_seed(name: &str) -> (PathBuf, PathBuf, PathBuf) {
    let root =
        std::env::temp_dir().join(format!("maw-rs-native-done-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let home = root.join("home");
    let config = root.join("config");
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).expect("bin dir");
    done_write(
        &config.join("fleet/13-nova.json"),
        r#"{"name":"13-nova","windows":[{"name":"task-done","repo":"org/repo/agents/task-done"}]}"#,
    );
    std::fs::create_dir_all(root.join("ghq/github.com/org/repo/agents/task-done"))
        .expect("worktree dir");
    done_write(
        &bin.join("tmux"),
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$DONE_TMUX_LOG"
case "$1" in
  list-windows)
    if [ "$DONE_TMUX_MODE" = "empty" ]; then exit 0; fi
    printf '13-nova|||0|||nova-oracle|||1|||/work/repo\n13-nova|||1|||task-done|||0|||/work/repo/agents/task-done\n'
    ;;
  display-message)
    if [ "$2" = "-p" ]; then printf '13-nova\t0\n'; exit 0; fi
    case "$3" in
      13-nova:task-done) printf 'codex\t/work/repo/agents/task-done\n' ;;
      *) exit 7 ;;
    esac
    ;;
  send-keys|kill-window)
    exit 0
    ;;
  *) exit 64 ;;
esac
"#,
    );
    done_chmod(&bin.join("tmux"));
    (root, home, config)
}

fn done_command(root: &Path, home: &Path, config: &Path) -> Command {
    let mut command = Command::new(done_bin());
    command
        .current_dir(root)
        .env_clear()
        .env("HOME", home)
        .env("MAW_CONFIG_DIR", config)
        .env("XDG_CONFIG_HOME", root.join("xdg-config"))
        .env("XDG_STATE_HOME", root.join("state"))
        .env("XDG_DATA_HOME", root.join("data"))
        .env("XDG_CACHE_HOME", root.join("cache"))
        .env("GHQ_ROOT", root.join("ghq"))
        .env("MAW_JS_REF_DIR", "/nonexistent")
        .env("DONE_TMUX_LOG", root.join("tmux.log"))
        .env("PATH", root.join("bin"));
    command
}

#[test]
fn done_native_matched_dry_run_is_hermetic_without_js_ref() {
    let (root, home, config) = done_seed("matched");
    let output = done_command(&root, &home, &config)
        .args(["done", "task-done", "--dry-run"])
        .output()
        .expect("run done");
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout"),
        format!(
            "{}\n",
            include_str!("fixtures/native-done/matched-dry-run.stdout")
        )
    );
    assert_eq!(String::from_utf8(output.stderr).expect("stderr"), "");
    let log = std::fs::read_to_string(root.join("tmux.log")).expect("tmux log");
    assert!(log.contains("list-windows"), "{log}");
    assert!(!log.contains("kill-window"), "dry run killed: {log}");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn done_native_config_dry_run_and_guard_do_not_touch_real_fleet() {
    let (root, home, config) = done_seed("config");
    let output = done_command(&root, &home, &config)
        .env("DONE_TMUX_MODE", "empty")
        .args(["finish", "task-done", "--dry-run"])
        .output()
        .expect("run finish");
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout"),
        format!(
            "{}\n",
            include_str!("fixtures/native-done/config-dry-run.stdout")
        )
    );
    assert_eq!(dispatcher_status("done"), DispatchKind::Native);
    assert_eq!(dispatcher_status("finish"), DispatchKind::Native);

    let guarded = done_command(&root, &home, &config)
        .args(["done", "-Sbad", "--dry-run"])
        .output()
        .expect("run guard");
    assert!(!guarded.status.success());
    assert_eq!(
        String::from_utf8(guarded.stderr).expect("stderr"),
        "done: unknown argument -Sbad\n"
    );
    let log = std::fs::read_to_string(root.join("tmux.log")).expect("tmux log");
    assert!(!log.contains("-Sbad"), "guarded target reached tmux: {log}");
    let _ = std::fs::remove_dir_all(root);
}
