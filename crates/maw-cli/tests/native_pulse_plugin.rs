use maw_cli::{dispatcher_status, DispatchKind};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_maw-rs"))
}

fn temp_dir(name: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("maw-rs-native-pulse-{name}-{stamp}"));
    fs::create_dir_all(&path).expect("temp dir");
    path
}

fn chmod_exec(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path).expect("metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).expect("chmod");
    }
}

fn write_fake_gh(bin_dir: &Path) {
    let gh = bin_dir.join("gh");
    fs::write(
        &gh,
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$MAW_PULSE_GH_LOG"
if [ "$1 $2" = 'issue list' ]; then
  printf '%s\n' '[{"number":20,"title":"📅 2026-06-25 Daily Thread","labels":[{"name":"daily-thread"}]},{"number":21,"title":"P001 launch board","labels":[{"name":"oracle:nova"}]},{"number":19,"title":"registry cleanup","labels":[]},{"number":22,"title":"ship pulse native","labels":[{"name":"oracle:pulse"}]}]'
  exit 0
fi
printf 'unexpected gh: %s\n' "$*" >&2
exit 42
"#,
    )
    .expect("write fake gh");
    chmod_exec(&gh);
}

fn write_fake_cleanup_gh(bin_dir: &Path) {
    let gh = bin_dir.join("gh");
    fs::write(
        &gh,
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$MAW_PULSE_GH_LOG"
case "$*" in
  *'pr view agents/1-old --repo acme/widgets --json state')
    printf '%s\n' '{"state":"MERGED"}'
    exit 0
    ;;
  *'pr view agents/merged --repo acme/widgets --json state')
    printf '%s\n' '{"state":"MERGED"}'
    exit 0
    ;;
  *'pr view agents/open --repo acme/widgets --json state')
    printf '%s\n' '{"state":"OPEN"}'
    exit 0
    ;;
esac
printf 'unexpected gh: %s\n' "$*" >&2
exit 42
"#,
    )
    .expect("write fake cleanup gh");
    chmod_exec(&gh);
}

fn write_fake_git(bin_dir: &Path) {
    let git = bin_dir.join("git");
    fs::write(
        &git,
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$MAW_PULSE_GIT_LOG"
case "$*" in
  *'rev-parse --abbrev-ref HEAD') printf 'agents/1-old\n'; exit 0 ;;
  *'worktree list --porcelain') exit 0 ;;
esac
printf 'unexpected git: %s\n' "$*" >&2
exit 42
"#,
    )
    .expect("write fake git");
    chmod_exec(&git);
}

fn write_fake_cleanup_git(bin_dir: &Path) {
    let git = bin_dir.join("git");
    fs::write(
        &git,
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$MAW_PULSE_GIT_LOG"
case "$*" in
  *'/agents/merged rev-parse --abbrev-ref HEAD') printf 'agents/merged\n'; exit 0 ;;
  *'/agents/open rev-parse --abbrev-ref HEAD') printf 'agents/open\n'; exit 0 ;;
  *'worktree list --porcelain') exit 0 ;;
esac
exit 0
"#,
    )
    .expect("write fake cleanup git");
    chmod_exec(&git);
}

fn write_fake_tmux(bin_dir: &Path) {
    let tmux = bin_dir.join("tmux");
    fs::write(
        &tmux,
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$MAW_PULSE_TMUX_LOG"
if [ "$*" = 'list-windows -a -F #W' ]; then
  printf '1-active\n'
  exit 0
fi
printf 'unexpected tmux: %s\n' "$*" >&2
exit 42
"#,
    )
    .expect("write fake tmux");
    chmod_exec(&tmux);
}

fn run(root: &Path, args: &[&str]) -> std::process::Output {
    let bin_dir = root.join("bin");
    let home = root.join("home");
    let xdg_config = root.join("xdg-config");
    let xdg_data = root.join("xdg-data");
    let xdg_state = root.join("xdg-state");
    let ghq = root.join("ghq");
    fs::create_dir_all(&home).expect("home");
    fs::create_dir_all(xdg_config.join("maw")).expect("xdg config");
    fs::write(
        xdg_config.join("maw/maw.config.json"),
        r#"{"node":"ci","oracle":"pulse-test"}"#,
    )
    .expect("seed config");
    fs::create_dir_all(&xdg_data).expect("xdg data");
    fs::create_dir_all(&xdg_state).expect("xdg state");
    fs::create_dir_all(&ghq).expect("ghq");

    Command::new(bin())
        .args(args)
        .current_dir(root)
        .env_clear()
        .env("PATH", &bin_dir)
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", &xdg_config)
        .env("XDG_DATA_HOME", &xdg_data)
        .env("XDG_STATE_HOME", &xdg_state)
        .env("GHQ_ROOT", &ghq)
        .env("TMUX", root.join("tmux-socket"))
        .env("MAW_JS_REF_DIR", "/nonexistent")
        .env("MAW_PULSE_GH_LOG", root.join("gh.log"))
        .env("MAW_PULSE_GIT_LOG", root.join("git.log"))
        .env("MAW_PULSE_TMUX_LOG", root.join("tmux.log"))
        .output()
        .expect("run maw-rs")
}

#[test]
fn native_pulse_list_matches_committed_golden_without_ref_checkout() {
    let root = temp_dir("list");
    let bin_dir = root.join("bin");
    fs::create_dir_all(&bin_dir).expect("bin dir");
    write_fake_gh(&bin_dir);

    let output = run(&root, &["pulse", "list"]);

    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout"),
        format!("{}\n", include_str!("fixtures/native-pulse/list.stdout"))
    );
    assert_eq!(String::from_utf8(output.stderr).expect("stderr"), "");
    assert_eq!(
        fs::read_to_string(root.join("gh.log")).expect("gh log"),
        "issue list --repo laris-co/pulse-oracle --state open --json number,title,labels --limit 50\n"
    );
}

#[test]
fn native_pulse_cleanup_dry_run_is_hermetic_and_matches_golden() {
    let root = temp_dir("cleanup");
    let bin_dir = root.join("bin");
    fs::create_dir_all(&bin_dir).expect("bin dir");
    write_fake_git(&bin_dir);
    write_fake_cleanup_gh(&bin_dir);
    write_fake_tmux(&bin_dir);
    let worktree = root.join("ghq/github.com/acme/widgets/agents/1-old");
    fs::create_dir_all(&worktree).expect("worktree");
    fs::write(
        worktree.join(".git"),
        "gitdir: ../../../.git/worktrees/1-old\n",
    )
    .expect("git marker");

    let output = run(&root, &["pulse", "cleanup", "--dry-run"]);

    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout"),
        format!(
            "{}\n",
            include_str!("fixtures/native-pulse/cleanup-dry-run.stdout")
        )
    );
    assert_eq!(String::from_utf8(output.stderr).expect("stderr"), "");
    assert_eq!(
        fs::read_to_string(root.join("tmux.log")).expect("tmux log"),
        "list-sessions -F #{session_name}\nlist-windows -a -F #W\n"
    );
    assert!(fs::read_to_string(root.join("git.log"))
        .expect("git log")
        .contains("rev-parse --abbrev-ref HEAD"));
    assert_eq!(
        fs::read_to_string(root.join("gh.log")).expect("gh log"),
        "pr view agents/1-old --repo acme/widgets --json state\n"
    );
    let git_log = fs::read_to_string(root.join("git.log")).expect("git log");
    assert!(!git_log.contains("fetch --prune origin"));
    assert!(!git_log.contains("push origin --delete agents/1-old"));
}

#[test]
fn native_pulse_cleanup_prunes_once_and_deletes_only_merged_remote_branches() {
    let root = temp_dir("cleanup-live");
    let bin_dir = root.join("bin");
    fs::create_dir_all(&bin_dir).expect("bin dir");
    write_fake_cleanup_git(&bin_dir);
    write_fake_cleanup_gh(&bin_dir);
    write_fake_tmux(&bin_dir);
    for name in ["merged", "open"] {
        let worktree = root.join(format!("ghq/github.com/acme/widgets/agents/{name}"));
        fs::create_dir_all(&worktree).expect("worktree");
        fs::write(
            worktree.join(".git"),
            "gitdir: ../../../.git/worktrees/test\n",
        )
        .expect("git marker");
    }

    let output = run(&root, &["pulse", "cleanup"]);

    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout");
    assert!(stdout.contains("deleted remote branch agents/merged"));
    assert!(!stdout.contains("deleted remote branch agents/open"));
    let git_log = fs::read_to_string(root.join("git.log")).expect("git log");
    assert_eq!(git_log.matches("fetch --prune origin").count(), 1);
    assert!(git_log.contains("push origin --delete agents/merged"));
    assert!(!git_log.contains("push origin --delete agents/open"));
    assert!(git_log.contains("branch -d agents/merged"));
    assert!(git_log.contains("branch -d agents/open"));
    assert_eq!(
        fs::read_to_string(root.join("gh.log")).expect("gh log"),
        concat!(
            "pr view agents/merged --repo acme/widgets --json state\n",
            "pr view agents/open --repo acme/widgets --json state\n"
        )
    );
}

#[test]
fn native_dispatcher_registers_pulse_plugin() {
    assert_eq!(dispatcher_status("pulse"), DispatchKind::Native);
}
