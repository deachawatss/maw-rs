use maw_cli::{dispatcher_status, DispatchKind};
use std::path::{Path, PathBuf};
use std::process::Command;

fn pane_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_maw-rs"))
}

fn pane_write(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("parent");
    }
    std::fs::write(path, text).expect("write");
}

fn pane_temp(name: &str) -> PathBuf {
    let root =
        std::env::temp_dir().join(format!("maw-rs-native-pane-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("bin")).expect("temp");
    root
}

fn pane_install_fake_tmux(root: &Path) {
    pane_write(
        &root.join("bin/tmux"),
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$MAW_FAKE_TMUX_LOG"
case "$1" in
  list-panes)
    if [ "$2" != "-t" ] || [ "$3" != "%7" ] || [ "$4" != "-F" ]; then
      echo "unexpected list-panes argv: $*" >&2
      exit 8
    fi
    printf '%s\n' '0|||%7|||lead|||10' '1|||%8|||tile-1|||20' '2|||%9|||tile-2|||40'
    ;;
  swap-pane)
    if [ "$3" != "%8" ] || [ "$5" != "%9" ]; then
      echo "unexpected swap argv: $*" >&2
      exit 9
    fi
    ;;
  *)
    echo "unexpected tmux command: $*" >&2
    exit 7
    ;;
esac
"#,
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(root.join("bin/tmux"))
            .expect("metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(root.join("bin/tmux"), permissions).expect("chmod");
    }
}

fn pane_command(root: &Path) -> Command {
    let mut command = Command::new(pane_bin());
    command
        .current_dir(root)
        .env_clear()
        .env("HOME", root.join("home"))
        .env("MAW_HOME", root.join("home/.maw"))
        .env("XDG_CONFIG_HOME", root.join("xdg-config"))
        .env("XDG_STATE_HOME", root.join("xdg-state"))
        .env("XDG_DATA_HOME", root.join("xdg-data"))
        .env("XDG_CACHE_HOME", root.join("xdg-cache"))
        .env("TMUX", "/tmp/tmux-104,1,0")
        .env("TMUX_PANE", "%7")
        .env("MAW_FAKE_TMUX_LOG", root.join("tmux.log"))
        .env("PATH", root.join("bin"));
    command
}

#[test]
fn pane_native_swap_uses_fake_tmux_and_safe_targets() {
    let root = pane_temp("swap");
    pane_install_fake_tmux(&root);
    let output = pane_command(&root)
        .args(["pane", "swap", "tile-1", "bottom"])
        .output()
        .expect("run pane swap");
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout"),
        "\u{1b}[32m✓\u{1b}[0m swapped tile-1 ↔ tile-2\n"
    );
    assert_eq!(String::from_utf8(output.stderr).expect("stderr"), "");
    assert_eq!(dispatcher_status("pane"), DispatchKind::Native);
    let log = std::fs::read_to_string(root.join("tmux.log")).expect("log");
    assert!(
        log.contains(
            "list-panes -t %7 -F #{pane_index}|||#{pane_id}|||#{pane_title}|||#{pane_top}"
        ),
        "{log}"
    );
    assert!(log.contains("swap-pane -s %8 -t %9"), "{log}");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn pane_native_guards_reject_tmux_injection_before_fake_tmux() {
    let root = pane_temp("guard");
    pane_install_fake_tmux(&root);
    let output = pane_command(&root)
        .args(["pane", "swap", "-t", "1"])
        .output()
        .expect("run pane guard");
    assert!(!output.status.success());
    assert_eq!(
        String::from_utf8(output.stderr).expect("stderr"),
        "pane: invalid pane target \"-t\"\n"
    );
    let log = std::fs::read_to_string(root.join("tmux.log")).unwrap_or_default();
    assert!(
        !log.contains("list-panes"),
        "guard must not list panes: {log}"
    );
    assert!(
        !log.contains("swap-pane"),
        "guard must not swap panes: {log}"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn pane_native_requires_tmux_without_real_session_touch() {
    let root = pane_temp("notmux");
    let mut command = pane_command(&root);
    command.env_remove("TMUX").env_remove("TMUX_PANE");
    let output = command
        .args(["pane", "swap", "0", "1"])
        .output()
        .expect("run");
    assert!(!output.status.success());
    assert_eq!(
        String::from_utf8(output.stderr).expect("stderr"),
        "\u{1b}[33m⚠\u{1b}[0m pane requires tmux\n"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("tmux.log")).unwrap_or_default(),
        ""
    );
    let _ = std::fs::remove_dir_all(root);
}
