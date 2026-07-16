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
    let path = std::env::temp_dir().join(format!("maw-rs-workon-{name}-{stamp}"));
    fs::create_dir_all(&path).expect("temp dir");
    path
}

fn isolated_cargo_target_dir(worktree: &Path) -> PathBuf {
    let slug = worktree
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .expect("worktree directory name");
    let root = if cfg!(unix) {
        PathBuf::from("/tmp")
    } else {
        std::env::temp_dir()
    };
    root.join(format!("maw-rs-target-{slug}"))
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

fn seed_hermetic_root(root: &Path, existing_windows: &str) -> PathBuf {
    let bin_dir = root.join("bin");
    fs::create_dir_all(&bin_dir).expect("bin dir");
    write_exe(
        &bin_dir.join("tmux"),
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$MAW_FAKE_TMUX_LOG"
case "$1" in
  display-message)
    case "$*" in
      *pane_current_command*) printf 'node\n' ;;
      *) printf '50-mawjs\n' ;;
    esac
    ;;
  list-windows) printf '%s' "$MAW_FAKE_TMUX_WINDOWS" ;;
  has-session) exit 1 ;;
  capture-pane) printf '$\n' ;;
  new-session|new-window|send-keys|select-window) exit 0 ;;
  *) printf 'unexpected tmux %s\n' "$1" >&2; exit 9 ;;
esac
"#,
    );
    write_exe(
        &bin_dir.join("git"),
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$MAW_FAKE_GIT_LOG"
if [ "$3" = "branch" ]; then exit 1; fi
if [ "$3" = "for-each-ref" ]; then
  if [ -f "$2/.maw-test-existing-agent-branch" ]; then printf 'agents/feat\n'; fi
  exit 0
fi
if [ "$3" = "ls-files" ]; then exit 0; fi
if [ "$3" = "fetch" ] && [ "$4" = "origin" ]; then
  if [ "${MAW_FAKE_GIT_FETCH_FAIL:-0}" = "1" ]; then
    printf 'fatal: Authentication failed for origin\n' >&2
    exit 128
  fi
  exit 0
fi
if [ "$3" = "symbolic-ref" ] && [ "$6" = "refs/remotes/origin/HEAD" ]; then
  printf 'origin/main\n'
  exit 0
fi
if [ "$3" = "worktree" ] && [ "$4" = "list" ]; then
  printf 'worktree %s\nHEAD 0000000000000000000000000000000000000000\nbranch refs/heads/main\n\n' "$2"
  exit 0
fi
if [ "$3" = "rev-parse" ] && [ "$4" = "--show-toplevel" ]; then
  cd "$2" 2>/dev/null || exit 128
  pwd
  exit 0
fi
if [ "$3" = "worktree" ] && [ "$4" = "add" ]; then
  mkdir -p "$5"
  printf 'gitdir: fake\n' > "$5/.git"
  mkdir -p "$5/ψ/memory"
  printf 'local worktree memory\n' > "$5/ψ/memory/local.md"
  exit 0
fi
if [ "$3" = "clean" ] && [ "$4" = "-fd" ]; then
  if [ "${MAW_FAKE_GIT_CLEAN:-0}" = "1" ]; then
    rm -rf "$2/.cargo" "$2/ψ"
  fi
  exit 0
fi
printf 'unexpected git args: %s\n' "$*" >&2
exit 9
"#,
    );

    let xdg_config = root.join("xdg-config");
    let ghq = root.join("ghq");
    let repo = ghq.join("github.com/acme/demo");
    fs::create_dir_all(&repo).expect("repo");
    fs::write(repo.join(".git"), "gitdir: main\n").expect("git marker");
    let config_dir = xdg_config.join("maw");
    fs::create_dir_all(&config_dir).expect("config dir");
    fs::write(
        config_dir.join("maw.config.json"),
        serde_json::json!({"commands":{"default":"echo launch"}}).to_string(),
    )
    .expect("config");
    fs::write(root.join("windows.txt"), existing_windows).expect("windows");
    bin_dir
}

fn run(root: &Path, bin_dir: &Path, args: &[&str]) -> std::process::Output {
    run_with_tmux_env(root, bin_dir, args, Some("/tmp/tmux-1000/default,123,0"))
}

fn run_with_tmux_env(
    root: &Path,
    bin_dir: &Path,
    args: &[&str],
    tmux_env: Option<&str>,
) -> std::process::Output {
    run_from(root, root, bin_dir, args, tmux_env)
}

fn run_from(
    cwd: &Path,
    root: &Path,
    bin_dir: &Path,
    args: &[&str],
    tmux_env: Option<&str>,
) -> std::process::Output {
    run_from_with_git_clean(cwd, root, bin_dir, args, tmux_env, false)
}

fn run_from_with_git_clean(
    cwd: &Path,
    root: &Path,
    bin_dir: &Path,
    args: &[&str],
    tmux_env: Option<&str>,
    git_clean_removes_untracked_state: bool,
) -> std::process::Output {
    let mut command = Command::new(bin());
    command
        .args(args)
        .current_dir(cwd)
        .env_clear()
        .env("PATH", bin_dir)
        .env("HOME", root.join("home"))
        .env("XDG_CONFIG_HOME", root.join("xdg-config"))
        .env("XDG_STATE_HOME", root.join("xdg-state"))
        .env("XDG_DATA_HOME", root.join("xdg-data"))
        .env("XDG_CACHE_HOME", root.join("xdg-cache"))
        .env("MAW_TEST_MODE", "1")
        .env("MAW_JS_REF_DIR", "/nonexistent")
        .env("GHQ_ROOT", root.join("ghq"))
        .env("MAW_FAKE_TMUX_LOG", root.join("tmux.log"))
        .env(
            "MAW_FAKE_TMUX_WINDOWS",
            fs::read_to_string(root.join("windows.txt")).expect("windows"),
        )
        .env("MAW_FAKE_GIT_LOG", root.join("git.log"));
    if git_clean_removes_untracked_state {
        command.env("MAW_FAKE_GIT_CLEAN", "1");
    }
    if let Some(value) = tmux_env {
        command.env("TMUX", value);
    }
    command.output().expect("run maw-rs")
}

fn normalize_root(text: &str, root: &Path) -> String {
    text.replace(&root.display().to_string(), "<ROOT>")
}

fn run_git(cwd: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {:?} failed:\nstdout={}\nstderr={}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("git utf8")
}

fn run_with_system_git(root: &Path, bin_dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(bin())
        .args(args)
        .current_dir(root)
        .env_clear()
        .env("PATH", format!("{}:/usr/bin:/bin", bin_dir.display()))
        .env("HOME", root.join("home"))
        .env("XDG_CONFIG_HOME", root.join("xdg-config"))
        .env("XDG_STATE_HOME", root.join("xdg-state"))
        .env("XDG_DATA_HOME", root.join("xdg-data"))
        .env("XDG_CACHE_HOME", root.join("xdg-cache"))
        .env("MAW_TEST_MODE", "1")
        .env("MAW_JS_REF_DIR", "/nonexistent")
        .env("MAW_FAKE_TMUX_LOG", root.join("tmux.log"))
        .env(
            "MAW_FAKE_TMUX_WINDOWS",
            fs::read_to_string(root.join("windows.txt")).expect("windows"),
        )
        .env("TMUX", "/tmp/tmux-1000/default,123,0")
        .output()
        .expect("run maw-rs")
}

fn run_with_fetch_failure(root: &Path, bin_dir: &Path, args: &[&str]) -> std::process::Output {
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
        .env("MAW_JS_REF_DIR", "/nonexistent")
        .env("GHQ_ROOT", root.join("ghq"))
        .env("MAW_FAKE_TMUX_LOG", root.join("tmux.log"))
        .env(
            "MAW_FAKE_TMUX_WINDOWS",
            fs::read_to_string(root.join("windows.txt")).expect("windows"),
        )
        .env("MAW_FAKE_GIT_LOG", root.join("git.log"))
        .env("MAW_FAKE_GIT_FETCH_FAIL", "1")
        .env("TMUX", "/tmp/tmux-1000/default,123,0");
    command.output().expect("run maw-rs")
}

#[test]
fn native_workon_fresh_branch_uses_fetched_origin_tip_not_stale_local_main() {
    let root = temp_dir("origin-tip");
    let bin_dir = seed_hermetic_root(&root, "shell\n");
    fs::remove_file(bin_dir.join("git")).expect("remove fake git");

    let remote = root.join("remote.git");
    run_git(
        &root,
        &["init", "--bare", remote.to_str().expect("remote path")],
    );
    let seed = root.join("seed");
    fs::create_dir_all(&seed).expect("seed dir");
    run_git(&seed, &["init"]);
    run_git(&seed, &["config", "user.email", "test@example.com"]);
    run_git(&seed, &["config", "user.name", "Test User"]);
    fs::write(seed.join("README.md"), "initial\n").expect("initial file");
    run_git(&seed, &["add", "README.md"]);
    run_git(&seed, &["commit", "-m", "initial"]);
    run_git(&seed, &["branch", "-M", "main"]);
    run_git(
        &seed,
        &[
            "remote",
            "add",
            "origin",
            remote.to_str().expect("remote path"),
        ],
    );
    run_git(&seed, &["push", "-u", "origin", "main"]);
    run_git(
        &root,
        &[
            "--git-dir",
            remote.to_str().expect("remote path"),
            "symbolic-ref",
            "HEAD",
            "refs/heads/main",
        ],
    );

    let local = root.join("local");
    run_git(
        &root,
        &[
            "clone",
            remote.to_str().expect("remote path"),
            local.to_str().expect("local path"),
        ],
    );
    let upstream = root.join("upstream");
    run_git(
        &root,
        &[
            "clone",
            remote.to_str().expect("remote path"),
            upstream.to_str().expect("upstream path"),
        ],
    );
    run_git(&upstream, &["config", "user.email", "test@example.com"]);
    run_git(&upstream, &["config", "user.name", "Test User"]);
    fs::write(upstream.join("README.md"), "origin tip\n").expect("origin tip file");
    run_git(&upstream, &["add", "README.md"]);
    run_git(&upstream, &["commit", "-m", "origin tip"]);
    run_git(&upstream, &["push"]);

    let local_main_before_fetch = run_git(&local, &["rev-parse", "main"]);
    let output = run_with_system_git(
        &root,
        &bin_dir,
        &["workon", local.to_str().expect("local path"), "fresh"],
    );

    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let branch_tip = run_git(&local, &["rev-parse", "agents/fresh"]);
    let origin_tip = run_git(&local, &["rev-parse", "origin/main"]);
    assert_eq!(branch_tip.trim(), origin_tip.trim());
    assert_ne!(branch_tip.trim(), local_main_before_fetch.trim());
}

#[test]
fn native_workon_base_overrides_the_default_origin_start_point() {
    let root = temp_dir("base");
    let bin_dir = seed_hermetic_root(&root, "shell\n");

    let output = run(
        &root,
        &bin_dir,
        &[
            "workon",
            "demo",
            "feat",
            "--base",
            "origin/release",
            "--layout",
            "nested",
        ],
    );

    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let git_log = fs::read_to_string(root.join("git.log")).expect("git log");
    assert!(git_log.contains("fetch origin"), "{git_log}");
    assert!(
        git_log.contains("worktree add") && git_log.contains("origin/release"),
        "{git_log}"
    );
}

#[test]
fn native_workon_fetch_failure_is_actionable_and_does_not_create_a_worktree() {
    let root = temp_dir("fetch-failure");
    let bin_dir = seed_hermetic_root(&root, "shell\n");

    let output = run_with_fetch_failure(
        &root,
        &bin_dir,
        &["workon", "demo", "feat", "--layout", "nested"],
    );

    assert!(
        !output.status.success(),
        "workon should stop when fetch fails"
    );
    let stderr = String::from_utf8(output.stderr).expect("stderr");
    assert!(stderr.contains("failed to fetch origin"), "{stderr}");
    assert!(
        stderr.contains("network and origin authentication"),
        "{stderr}"
    );
    let git_log = fs::read_to_string(root.join("git.log")).expect("git log");
    assert!(git_log.contains("fetch origin"), "{git_log}");
    assert!(!git_log.contains("worktree add"), "{git_log}");
}

#[test]
fn native_workon_existing_branch_fetches_before_reusing_that_branch() {
    let root = temp_dir("existing-branch");
    let bin_dir = seed_hermetic_root(&root, "shell\n");
    let repo = root.join("ghq/github.com/acme/demo");
    fs::write(
        repo.join(".maw-test-existing-agent-branch"),
        "agents/feat\n",
    )
    .expect("branch marker");

    let output = run(
        &root,
        &bin_dir,
        &[
            "workon", "demo", "--wt", "feat", "--name", "feat", "--layout", "nested",
        ],
    );

    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let git_log = fs::read_to_string(root.join("git.log")).expect("git log");
    let fetch = git_log.find("fetch origin").expect("fetch origin");
    let add = git_log.find("worktree add").expect("worktree add");
    assert!(fetch < add, "{git_log}");
    assert!(
        git_log.contains("worktree add") && git_log.contains("agents/feat"),
        "{git_log}"
    );
}

#[test]
fn native_workon_create_nested_matches_committed_golden_without_ref_checkout() {
    let root = temp_dir("create");
    let bin_dir = seed_hermetic_root(&root, "shell\n");

    let output = run(
        &root,
        &bin_dir,
        &["workon", "demo", "feat", "--layout", "nested"],
    );

    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        normalize_root(&String::from_utf8(output.stdout).expect("stdout"), &root),
        include_str!("fixtures/native-workon/create-nested.stdout")
    );
    assert_eq!(String::from_utf8(output.stderr).expect("stderr"), "");
    let tmux_log = fs::read_to_string(root.join("tmux.log")).expect("tmux log");
    assert!(
        tmux_log.contains("new-window -P -F #{window_id} -t 50-mawjs: -n demo-feat -c"),
        "{tmux_log}"
    );
    assert!(
        tmux_log.contains("send-keys -t 50-mawjs:demo-feat -l echo launch"),
        "{tmux_log}"
    );
    assert!(
        tmux_log.contains("send-keys -t 50-mawjs:demo-feat Enter"),
        "{tmux_log}"
    );
    let git_log = fs::read_to_string(root.join("git.log")).expect("git log");
    assert!(git_log.contains("worktree add"), "{git_log}");
}

#[test]
fn native_workon_rejects_malformed_managed_gitignore_before_launching() {
    let root = temp_dir("malformed-gitignore");
    let bin_dir = seed_hermetic_root(&root, "shell\n");
    let repo = root.join("ghq/github.com/acme/demo");
    fs::write(
        repo.join(".gitignore"),
        "# >>> maw ephemeral markers (managed by maw-rs) >>>\n.maw/delivery.json\n",
    )
    .expect("malformed gitignore");

    let output = run(
        &root,
        &bin_dir,
        &["workon", "demo", "feat", "--layout", "nested"],
    );

    assert!(!output.status.success(), "workon should fail closed");
    let stderr = String::from_utf8(output.stderr).expect("stderr");
    assert!(
        stderr.contains("malformed managed .gitignore block (missing end marker)"),
        "{stderr}"
    );
    assert!(
        stderr
            .contains("Fix .gitignore manually or remove the malformed managed block, then retry"),
        "{stderr}"
    );
    assert!(
        fs::read_to_string(root.join("tmux.log")).map_or(true, |log| !log.contains("new-window")),
        "L2 session must not launch"
    );
}

#[cfg(unix)]
#[test]
fn native_workon_rejects_read_only_gitignore_before_launching() {
    use std::os::unix::fs::PermissionsExt;

    let root = temp_dir("readonly-gitignore");
    let bin_dir = seed_hermetic_root(&root, "shell\n");
    let gitignore = root.join("ghq/github.com/acme/demo/.gitignore");
    fs::write(&gitignore, "target/\n").expect("gitignore");
    fs::set_permissions(&gitignore, fs::Permissions::from_mode(0o444)).expect("make read-only");

    let output = run(
        &root,
        &bin_dir,
        &["workon", "demo", "feat", "--layout", "nested"],
    );

    assert!(!output.status.success(), "workon should fail closed");
    let stderr = String::from_utf8(output.stderr).expect("stderr");
    assert!(stderr.contains("workon: write .gitignore:"), "{stderr}");
    assert!(
        stderr
            .contains("Fix .gitignore manually or remove the malformed managed block, then retry"),
        "{stderr}"
    );
    assert!(
        fs::read_to_string(root.join("tmux.log")).map_or(true, |log| !log.contains("new-window")),
        "L2 session must not launch"
    );
}

#[cfg(unix)]
#[test]
fn native_workon_omx_create_restores_worktree_state_after_git_clean() {
    let root = temp_dir("worktree-state");
    let bin_dir = seed_hermetic_root(&root, "shell\n");
    let repo = root.join("ghq/github.com/acme/demo");
    fs::create_dir_all(repo.join("ψ/memory/learnings")).expect("main psi");
    fs::write(repo.join("ψ/memory/learnings/main.md"), "main memory\n").expect("main learning");
    fs::write(repo.join("Cargo.toml"), "[workspace]\nmembers = []\n").expect("cargo toml");
    fs::write(
        root.join("xdg-config/maw/maw.config.json"),
        serde_json::json!({"commands":{"default":"echo launch","omx":"omx-launch --direct"}})
            .to_string(),
    )
    .expect("omx config");

    let output = run_from_with_git_clean(
        &root,
        &root,
        &bin_dir,
        &[
            "workon", "demo", "feat", "--layout", "nested", "--engine", "omx",
        ],
        Some("/tmp/tmux-1000/default,123,0"),
        true,
    );

    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let wt = repo.join("agents/feat");
    let psi = wt.join("ψ");
    assert!(fs::symlink_metadata(&psi)
        .expect("psi metadata")
        .file_type()
        .is_symlink());
    assert_eq!(fs::read_link(&psi).expect("psi link"), repo.join("ψ"));
    let expected_target = isolated_cargo_target_dir(&wt);
    assert_eq!(
        fs::read_to_string(wt.join(".cargo/config.toml")).expect("cargo config"),
        format!("[build]\ntarget-dir = \"{}\"\n", expected_target.display())
    );
    let tmux_log = fs::read_to_string(root.join("tmux.log")).expect("tmux log");
    assert!(
        tmux_log.contains("send-keys -t 50-mawjs:demo-feat -l omx-launch --direct"),
        "{tmux_log}"
    );
}

#[test]
fn native_workon_reuse_repairs_missing_isolated_cargo_target_config() {
    let root = temp_dir("reuse-isolated-target");
    let bin_dir = seed_hermetic_root(&root, "shell\n");
    let repo = root.join("ghq/github.com/acme/demo");
    let worktree = repo.join("agents/reused");
    fs::create_dir_all(&worktree).expect("worktree");
    fs::write(worktree.join(".git"), "gitdir: fake\n").expect("worktree git marker");
    fs::write(repo.join("Cargo.toml"), "[workspace]\nmembers = []\n").expect("cargo toml");

    let output = run(&root, &bin_dir, &["workon", "demo", "reused"]);

    assert!(
        output.status.success(),
        "stdout={}\\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let expected_target = isolated_cargo_target_dir(&worktree);
    assert_eq!(
        fs::read_to_string(worktree.join(".cargo/config.toml")).expect("cargo config"),
        format!("[build]\ntarget-dir = \"{}\"\n", expected_target.display())
    );
}

#[test]
fn native_workon_reuse_window_is_hermetic_and_does_not_spawn() {
    let root = temp_dir("reuse");
    let bin_dir = seed_hermetic_root(&root, "demo\n");

    let output = run(&root, &bin_dir, &["workon", "demo"]);

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout"),
        include_str!("fixtures/native-workon/reuse-window.stdout")
    );
    let tmux_log = fs::read_to_string(root.join("tmux.log")).expect("tmux log");
    assert!(
        tmux_log.contains("select-window -t 50-mawjs:demo"),
        "{tmux_log}"
    );
    assert!(!tmux_log.contains("new-window"), "{tmux_log}");
}

#[test]
fn native_workon_records_parent_oracle_for_l1_handoff() {
    let root = temp_dir("l1-oracle");
    let bin_dir = seed_hermetic_root(&root, "demo\n");

    let output = run(&root, &bin_dir, &["workon", "demo"]);

    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata = root.join("ghq/github.com/acme/demo/.maw");
    assert_eq!(
        fs::read_to_string(metadata.join("l1-oracle")).expect("oracle metadata"),
        "50-mawjs\n"
    );
    assert!(
        !metadata.join("l1-pane").exists(),
        "new workon runs must not persist the legacy pane target"
    );
}

#[test]
fn native_workon_outside_tmux_creates_session_and_prints_attach_plan() {
    let root = temp_dir("outside");
    let bin_dir = seed_hermetic_root(&root, "");

    let output = run_with_tmux_env(&root, &bin_dir, &["workon", "demo"], None);

    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout");
    assert!(
        stdout.contains("workon 'demo' in new session demo"),
        "{stdout}"
    );
    assert!(stdout.contains("run: tmux attach -t demo"), "{stdout}");
    let tmux_log = fs::read_to_string(root.join("tmux.log")).expect("tmux log");
    assert!(tmux_log.contains("has-session -t =demo"), "{tmux_log}");
    assert!(
        tmux_log.contains("new-session -d -s demo -c") && tmux_log.contains("-n demo"),
        "{tmux_log}"
    );
    assert!(
        tmux_log.contains("send-keys -t demo:demo -l echo launch"),
        "{tmux_log}"
    );
    assert!(
        tmux_log.contains("display-message -t demo:demo -p #{pane_in_mode}"),
        "{tmux_log}"
    );
    assert!(
        tmux_log.contains("capture-pane -t demo:demo -e -p -S -10"),
        "{tmux_log}"
    );
}

#[test]
fn native_workon_headless_oracle_flag_uses_the_requested_existing_session() {
    let root = temp_dir("headless-oracle");
    let bin_dir = seed_hermetic_root(&root, "");
    write_exe(
        &bin_dir.join("tmux"),
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$MAW_FAKE_TMUX_LOG"
case "$1" in
  has-session) [ "$2" = "-t" ] && [ "$3" = "=01-gale" ] ;;
  list-windows) printf '%s' "$MAW_FAKE_TMUX_WINDOWS" ;;
  display-message)
    case "$*" in
      *pane_current_command*) printf 'node\n' ;;
      *) printf '0\n' ;;
    esac
    ;;
  capture-pane) printf '$\n' ;;
  new-window|send-keys|select-window) exit 0 ;;
  *) printf 'unexpected tmux %s\n' "$1" >&2; exit 9 ;;
esac
"#,
    );

    let output = run_with_tmux_env(
        &root,
        &bin_dir,
        &["workon", "demo", "feat", "--oracle", "01-gale"],
        None,
    );

    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout");
    assert!(stdout.contains("workon 'demo-feat' in 01-gale"), "{stdout}");
    assert!(!stdout.contains("run: tmux attach"), "{stdout}");

    let tmux_log = fs::read_to_string(root.join("tmux.log")).expect("tmux log");
    assert!(tmux_log.contains("has-session -t =01-gale"), "{tmux_log}");
    assert!(
        tmux_log.contains("list-windows -t 01-gale -F #{window_name}"),
        "{tmux_log}"
    );
    assert!(
        tmux_log.contains("new-window -P -F #{window_id} -t 01-gale: -n demo-feat -c"),
        "{tmux_log}"
    );
    assert!(!tmux_log.contains("new-session"), "{tmux_log}");
}

#[test]
fn native_workon_dot_resolves_current_repo() {
    let root = temp_dir("dot");
    let bin_dir = seed_hermetic_root(&root, "shell\n");
    let repo = root.join("ghq/github.com/acme/demo");

    let output = run_from(
        &repo,
        &root,
        &bin_dir,
        &["workon", "."],
        Some("/tmp/tmux-1000/default,123,0"),
    );

    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout");
    assert!(stdout.contains("workon 'demo' in 50-mawjs"), "{stdout}");
    let tmux_log = fs::read_to_string(root.join("tmux.log")).expect("tmux log");
    assert!(
        tmux_log.contains("new-window -P -F #{window_id} -t 50-mawjs: -n demo -c"),
        "{tmux_log}"
    );
    let git_log = fs::read_to_string(root.join("git.log")).expect("git log");
    assert!(git_log.contains("rev-parse --show-toplevel"), "{git_log}");
}

#[test]
fn native_workon_registers_dispatcher_and_guards_layout() {
    assert_eq!(dispatcher_status("workon"), DispatchKind::Native);
    let root = temp_dir("layout");
    let bin_dir = seed_hermetic_root(&root, "");

    let output = run(&root, &bin_dir, &["workon", "demo", "--layout", "bad"]);

    assert!(!output.status.success());
    assert_eq!(String::from_utf8(output.stdout).expect("stdout"), "");
    assert!(String::from_utf8(output.stderr)
        .expect("stderr")
        .contains("workon: --layout must be nested or legacy"));
}
