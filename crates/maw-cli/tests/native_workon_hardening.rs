#![forbid(unsafe_code)]

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
    let path = std::env::temp_dir().join(format!("maw-rs-workon-hardening-{name}-{stamp}"));
    fs::create_dir_all(&path).expect("temp dir");
    path
}

fn write_exe(path: &Path, body: &str) {
    fs::write(path, body).expect("write exe");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path).expect("metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).expect("chmod");
    }
}

fn seed_root(root: &Path) -> PathBuf {
    let bin_dir = root.join("bin");
    fs::create_dir_all(&bin_dir).expect("bin dir");
    write_exe(
        &bin_dir.join("tmux"),
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$MAW_FAKE_TMUX_LOG"
case "$1" in
  display-message) printf '50-mawjs\n' ;;
  list-windows) printf '%s' "$MAW_FAKE_TMUX_WINDOWS" ;;
  new-window|send-keys|select-window|capture-pane) exit 0 ;;
  *) printf 'unexpected tmux %s\n' "$1" >&2; exit 9 ;;
esac
"#,
    );
    write_exe(
        &bin_dir.join("git"),
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$MAW_FAKE_GIT_LOG"
if [ "$3" = "branch" ]; then exit 1; fi
if [ "$3" = "worktree" ] && [ "$4" = "add" ]; then
  /bin/mkdir -p "$5/.maw" "$5/.git"
  printf '{}\n' > "$5/.maw/phase.json"
  printf '{}\n' > "$5/.maw/strategy.json"
  printf '\n' > "$5/.maw/solo-justified"
  printf '\n' > "$5/.maw/aggregate-verified"
  printf '\n' > "$5/.maw/done-pinged"
  printf '\n' > "$5/.git/index.lock"
  printf 'keep claude\n' > "$5/CLAUDE.md"
  printf 'keep context\n' > "$5/CONTEXT.md"
  exit 0
fi
printf 'unexpected git args: %s\n' "$*" >&2
exit 9
"#,
    );

    let repo = root.join("ghq/github.com/acme/demo");
    fs::create_dir_all(&repo).expect("repo");
    fs::write(repo.join(".git"), "gitdir: main\n").expect("git marker");
    let config_dir = root.join("xdg-config/maw");
    fs::create_dir_all(&config_dir).expect("config dir");
    fs::write(
        config_dir.join("maw.config.json"),
        serde_json::json!({"commands":{"default":"claude"}}).to_string(),
    )
    .expect("config");
    bin_dir
}

fn run(root: &Path, bin_dir: &Path, args: &[&str]) -> std::process::Output {
    let mut command = Command::new(bin());
    command
        .args(args)
        .current_dir(root)
        .env_clear()
        .env("PATH", bin_dir)
        .env("HOME", root.join("home"))
        .env("XDG_CONFIG_HOME", root.join("xdg-config"))
        .env("XDG_STATE_HOME", root.join("xdg-state"))
        .env("XDG_DATA_HOME", root.join("xdg-data"))
        .env("XDG_CACHE_HOME", root.join("xdg-cache"))
        .env("MAW_TEST_MODE", "1")
        .env("GHQ_ROOT", root.join("ghq"))
        .env("TMUX", "/tmp/tmux-1000/default,123,0")
        .env("MAW_FAKE_TMUX_LOG", root.join("tmux.log"))
        .env("MAW_FAKE_TMUX_WINDOWS", "shell\n")
        .env("MAW_FAKE_GIT_LOG", root.join("git.log"));
    command.output().expect("run maw-rs")
}

#[test]
fn native_workon_sanitizes_fresh_worktree_state() {
    let root = temp_dir("sanitize");
    let bin_dir = seed_root(&root);

    let output = run(&root, &bin_dir, &["workon", "demo", "feat"]);

    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let wt = root.join("ghq/github.com/acme/demo/agents/1-feat");
    for stale in [
        ".maw/phase.json",
        ".maw/strategy.json",
        ".maw/solo-justified",
        ".maw/aggregate-verified",
        ".maw/done-pinged",
        ".git/index.lock",
    ] {
        assert!(!wt.join(stale).exists(), "{stale} should be removed");
    }
    assert_eq!(
        fs::read_to_string(wt.join("CLAUDE.md")).expect("claude"),
        "keep claude\n"
    );
    assert_eq!(
        fs::read_to_string(wt.join("CONTEXT.md")).expect("context"),
        "keep context\n"
    );
    let tmux_log = fs::read_to_string(root.join("tmux.log")).expect("tmux log");
    assert!(
        tmux_log.contains("send-keys -t 50-mawjs:demo-feat -l claude"),
        "{tmux_log}"
    );
}
