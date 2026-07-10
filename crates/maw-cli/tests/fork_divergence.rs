#![forbid(unsafe_code)]

mod done_hardening {
    use maw_cli::wind::done::rescue_psi;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::{Mutex, OnceLock};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn done_bin() -> PathBuf {
        PathBuf::from(env!("CARGO_BIN_EXE_maw-rs"))
    }

    fn unique_root(name: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "maw-rs-done-hardening-{name}-{}-{stamp}",
            std::process::id()
        ))
    }

    fn write_file(path: &Path, text: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("parent dir");
        }
        std::fs::write(path, text).expect("write file");
    }

    fn chmod_exec(path: &Path) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(path).expect("metadata").permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(path, permissions).expect("chmod");
        }
    }

    fn write_fake_git(bin: &Path) {
        write_file(
            &bin.join("git"),
            r#"#!/bin/sh
printf '%s
' "$*" >> "$DONE_GIT_LOG"
case "$*" in
  *"rev-parse --abbrev-ref HEAD"*) printf 'main
' ;;
  *"rev-parse --git-common-dir"*) printf '%s/.git
' "$DONE_MAIN" ;;
  *"rev-parse --show-toplevel"*) if [ "$DONE_FAKE_WORKTREE" = "1" ]; then printf '%s
' "$DONE_WORKTREE"; fi ;;
  *"worktree list --porcelain"*) if [ "$DONE_FAKE_WORKTREE" = "1" ]; then printf 'worktree %s
' "$DONE_WORKTREE"; fi ;;
  *"status --porcelain -- ψ/"*)
    if [ "$DONE_STATUS_PSI" = "test" ]; then printf '?? ψ/test.md
'; fi
    if [ "$DONE_STATUS_PSI" = "symlink" ]; then printf '?? ψ/note.md
?? ψ/leak.md
'; fi ;;
  *) exit 0 ;;
esac
"#,
        );
        chmod_exec(&bin.join("git"));
    }

    fn write_fake_tmux(bin: &Path) {
        write_file(
            &bin.join("tmux"),
            r#"#!/bin/sh
printf '%s
' "$*" >> "$DONE_TMUX_LOG"
case "$1" in
  list-windows) printf '13-nova|||0|||nova-oracle|||1|||%s
13-nova|||1|||task-done|||0|||%s
' "$DONE_MAIN" "$DONE_WORKTREE" ;;
  display-message)
    case "$*" in
      *pane_current_command*) printf '%s	%s
' "${DONE_PANE_COMMAND:-codex}" "$DONE_WORKTREE" ;;
      *) printf '13-nova	%s
' "${DONE_CURRENT_INDEX:-0}" ;;
    esac ;;
  capture-pane)
    count=0
    if [ -f "$DONE_CAPTURE_COUNT" ]; then IFS= read -r count < "$DONE_CAPTURE_COUNT"; fi
    count=$((count + 1)); printf '%s' "$count" > "$DONE_CAPTURE_COUNT"
    if [ "$count" -lt 4 ]; then printf 'retrospective still running
'; else printf 'ctx%% 
'; fi ;;
  send-keys|kill-window) exit 0 ;;
  *) exit 0 ;;
esac
"#,
        );
        chmod_exec(&bin.join("tmux"));
    }

    fn seed_done_fixture(name: &str) -> (PathBuf, PathBuf, PathBuf, PathBuf) {
        let root = unique_root(name);
        let bin = root.join("bin");
        let config = root.join("config");
        let main = root.join("ghq/github.com/org/repo");
        let worktree = main.join("agents/task-done");
        std::fs::create_dir_all(&bin).expect("bin dir");
        std::fs::create_dir_all(&worktree).expect("worktree dir");
        write_file(
            &config.join("fleet/13-nova.json"),
            r#"{"name":"13-nova","windows":[{"name":"task-done","repo":"org/repo/agents/task-done"}]}"#,
        );
        write_fake_git(&bin);
        write_fake_tmux(&bin);
        (root, bin, main, worktree)
    }

    fn done_command(root: &Path, bin: &Path, main: &Path, worktree: &Path) -> Command {
        let mut command = Command::new(done_bin());
        command
            .current_dir(root)
            .env_clear()
            .env("HOME", root.join("home"))
            .env("MAW_CONFIG_DIR", root.join("config"))
            .env("XDG_CONFIG_HOME", root.join("xdg-config"))
            .env("XDG_STATE_HOME", root.join("state"))
            .env("XDG_DATA_HOME", root.join("data"))
            .env("XDG_CACHE_HOME", root.join("cache"))
            .env("GHQ_ROOT", root.join("ghq"))
            .env("MAW_JS_REF_DIR", "/nonexistent")
            .env("PATH", bin)
            .env("DONE_MAIN", main)
            .env("DONE_WORKTREE", worktree)
            .env("DONE_GIT_LOG", root.join("git.log"))
            .env("DONE_TMUX_LOG", root.join("tmux.log"))
            .env("DONE_CAPTURE_COUNT", root.join("capture-count"))
            .env("MAW_TEST_MODE", "1")
            .env("MAW_DONE_RRR_WAIT_INTERVAL_MS", "0");
        command
    }

    #[test]
    fn psi_rescue_copies_uncommitted_files_without_overwrite() {
        let _guard = env_lock().lock().expect("env lock");
        let root = unique_root("psi");
        let bin = root.join("bin");
        let main = root.join("main");
        let worktree = root.join("worktree");
        std::fs::create_dir_all(&bin).expect("bin dir");
        write_fake_git(&bin);
        write_file(&main.join("ψ/test.md"), "main copy");
        write_file(&worktree.join("ψ/test.md"), "worktree copy");
        std::env::set_var("PATH", &bin);
        std::env::set_var("DONE_MAIN", &main);
        std::env::set_var("DONE_STATUS_PSI", "test");
        std::env::set_var("DONE_GIT_LOG", root.join("git.log"));

        let rescued = rescue_psi(&worktree, &main).expect("rescue ok");
        assert_eq!(
            std::fs::read_to_string(main.join("ψ/test.md")).expect("main file"),
            "main copy"
        );
        assert_eq!(rescued.len(), 1, "rescued paths: {rescued:?}");
        let rescued_name = rescued[0]
            .file_name()
            .and_then(|name| name.to_str())
            .expect("rescued name");
        assert!(
            rescued_name.starts_with("test-")
                && Path::new(rescued_name)
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("md")),
            "{rescued_name}"
        );
        assert_eq!(
            std::fs::read_to_string(&rescued[0]).expect("rescued file"),
            "worktree copy"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn psi_rescue_skips_symlinks_and_does_not_exfiltrate() {
        // Security: a worktree ψ/ symlink pointing outside the worktree (e.g. a
        // planted `ψ/leak.md -> ~/.ssh/id_rsa`) MUST NOT be dereferenced and its
        // target's contents copied into the main repo ψ/. Real files are still
        // rescued; the symlink is skipped.
        let _guard = env_lock().lock().expect("env lock");
        let root = unique_root("psi-symlink");
        let bin = root.join("bin");
        let main = root.join("main");
        let worktree = root.join("worktree");
        std::fs::create_dir_all(&bin).expect("bin dir");
        write_fake_git(&bin);
        write_file(&root.join("secret/id_rsa"), "TOP SECRET KEY MATERIAL");
        write_file(&worktree.join("ψ/note.md"), "real memory worth keeping");
        std::os::unix::fs::symlink(root.join("secret/id_rsa"), worktree.join("ψ/leak.md"))
            .expect("plant symlink");
        std::env::set_var("PATH", &bin);
        std::env::set_var("DONE_MAIN", &main);
        std::env::set_var("DONE_STATUS_PSI", "symlink");
        std::env::set_var("DONE_GIT_LOG", root.join("git.log"));

        let rescued = rescue_psi(&worktree, &main).expect("rescue ok");
        let names: Vec<String> = rescued
            .iter()
            .filter_map(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .map(str::to_owned)
            })
            .collect();
        assert!(
            names.iter().any(|name| name.starts_with("note")),
            "real file must be rescued: {names:?}"
        );
        assert!(
            !names.iter().any(|name| name.starts_with("leak")),
            "symlink must NOT be rescued: {names:?}"
        );
        assert!(
            !main.join("ψ/leak.md").exists(),
            "symlink target must not be materialized in main ψ/"
        );
        for path in &rescued {
            assert!(
                !std::fs::read_to_string(path)
                    .unwrap_or_default()
                    .contains("TOP SECRET"),
                "secret content exfiltrated into {}",
                path.display()
            );
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn done_rescues_uncommitted_psi_through_compiled_binary() {
        // Wiring guard (not the isolated rescue_psi unit test above): the compiled
        // `maw done` MUST rescue uncommitted ψ/ notes to main BEFORE it removes the
        // worktree / force-deletes the branch. Drives the real binary end-to-end.
        let (root, bin, main, worktree) = seed_done_fixture("psi-wire");
        // Fake git reports ψ/test.md as uncommitted (DONE_STATUS_PSI=test); the
        // physical file must exist in the worktree for rescue_psi to copy it.
        write_file(&worktree.join("ψ/test.md"), "worktree note worth keeping");
        let output = done_command(&root, &bin, &main, &worktree)
            .env("DONE_STATUS_PSI", "test")
            .env("DONE_FAKE_WORKTREE", "1")
            .args([
                "done",
                "task-done",
                "--worktree",
                worktree.to_str().expect("worktree utf8"),
            ])
            .output()
            .expect("run done");
        assert!(
            output.status.success(),
            "stderr={}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("rescued"),
            "expected rescue line; stdout={stdout}"
        );
        // main had no ψ/test.md, so it is copied there verbatim before removal.
        assert_eq!(
            std::fs::read_to_string(main.join("ψ/test.md")).expect("rescued main file"),
            "worktree note worth keeping"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn rrr_wait_polls_until_prompt_returns() {
        let (root, bin, main, worktree) = seed_done_fixture("rrr");
        let output = done_command(&root, &bin, &main, &worktree)
            .env("DONE_PANE_COMMAND", "claude")
            .args(["done", "task-done"])
            .output()
            .expect("run done");
        assert!(
            output.status.success(),
            "stderr={}",
            String::from_utf8_lossy(&output.stderr)
        );
        let count: u32 = std::fs::read_to_string(root.join("capture-count"))
            .expect("capture count")
            .trim()
            .parse()
            .expect("count is numeric");
        assert!(count >= 4, "expected ≥4 capture-pane polls, got {count}");
        let log = std::fs::read_to_string(root.join("tmux.log")).expect("tmux log");
        assert!(
            log.contains("send-keys -t 13-nova:task-done -l /rrr"),
            "{log}"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn self_invocation_guard_blocks_own_window() {
        let (root, bin, main, worktree) = seed_done_fixture("self");
        let output = done_command(&root, &bin, &main, &worktree)
            .env("DONE_CURRENT_INDEX", "1")
            .args(["done", "task-done", "--dry-run"])
            .output()
            .expect("run done");
        assert!(!output.status.success());
        let stderr = String::from_utf8(output.stderr).expect("stderr utf8");
        assert!(
            stderr.contains("refusing to done current window 'task-done' in session '13-nova'"),
            "{stderr}"
        );
        let _ = std::fs::remove_dir_all(root);
    }
}

mod team_hardening {
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::Mutex,
        time::{SystemTime, UNIX_EPOCH},
    };

    use maw_cli::{run_cli, wind::team};
    use maw_tmux::{TmuxError, TmuxRunner};

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[derive(Default)]
    struct MockTmuxRunner {
        calls: Vec<(String, Vec<String>)>,
    }

    impl TmuxRunner for MockTmuxRunner {
        fn run(&mut self, subcommand: &str, args: &[String]) -> Result<String, TmuxError> {
            self.calls.push((subcommand.to_owned(), args.to_owned()));
            Ok(String::new())
        }
    }

    fn temp_dir(label: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "maw-rs-team-harden-{label}-{}-{stamp}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("temp dir");
        path
    }

    fn write_file(path: &Path, text: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("parent dir");
        }
        fs::write(path, text).expect("write file");
    }

    #[test]
    fn caller_pane_anchor() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let old = std::env::var_os("TMUX_PANE");
        std::env::set_var("TMUX_PANE", "%42");

        assert_eq!(team::caller_pane().as_deref(), Some("%42"));
        assert_eq!(
            team::spawn_pane_target(team::caller_pane().as_deref()),
            "%42"
        );

        match old {
            Some(value) => std::env::set_var("TMUX_PANE", value),
            None => std::env::remove_var("TMUX_PANE"),
        }
    }

    #[test]
    fn omx_auto_kickoff() {
        let root = temp_dir("kickoff");
        let prompt_path = root.join("spawn-prompt.md");
        let prompt = "Issue #1: begin now\nReport DONE to L2.";
        write_file(&prompt_path, prompt);
        let mut runner = MockTmuxRunner::default();

        team::omx_auto_kickoff_with(&mut runner, "%9", &prompt_path).expect("kickoff");

        assert!(runner.calls.iter().any(|(command, args)| {
            command == "send-keys" && args.iter().any(|arg| arg == prompt)
        }));
        assert!(runner.calls.iter().any(|(command, args)| {
            command == "send-keys" && args.last().is_some_and(|arg| arg == "Enter")
        }));
    }

    #[test]
    fn orphan_pane_sweep() {
        let member_panes = vec!["%1".to_owned(), "%2".to_owned()];
        let pane_pids = vec![("%1".to_owned(), 101), ("%2".to_owned(), 202)];

        let zombies = team::orphan_sweep_from_pids(&member_panes, &pane_pids, |pid| pid == 101);

        assert_eq!(zombies, vec!["%2".to_owned()]);
    }

    #[test]
    fn zombie_count() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let root = temp_dir("status-zombie");
        let home = root.join("home");
        let config = home.join(".claude/teams/alpha/config.json");
        write_file(
            &config,
            r#"{"name":"alpha","members":[{"name":"live","tmuxPaneId":"%1"},{"name":"dead","tmuxPaneId":"%2"}],"createdAt":0,"callerPane":"%9"}"#,
        );
        let old_home = std::env::var_os("HOME");
        let old_pids = std::env::var_os("MAW_RS_TEAM_PANE_PIDS");
        std::env::set_var("HOME", &home);
        std::env::set_var(
            "MAW_RS_TEAM_PANE_PIDS",
            format!("%1|{}\n%2|4294967295", std::process::id()),
        );

        let output = run_cli(&["team".to_owned(), "status".to_owned(), "alpha".to_owned()]);

        assert_eq!(output.code, 0, "{}", output.stderr);
        assert!(
            output.stdout.contains("2 agents") && output.stdout.contains("zombie"),
            "expected zombie status in output: {}",
            output.stdout
        );

        match old_home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
        match old_pids {
            Some(value) => std::env::set_var("MAW_RS_TEAM_PANE_PIDS", value),
            None => std::env::remove_var("MAW_RS_TEAM_PANE_PIDS"),
        }
    }
}

mod workon_hardening {
    use std::{
        fs,
        path::{Path, PathBuf},
        process::{Command, Output},
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
        let path = std::env::temp_dir().join(format!("maw-rs-fork-divergence-{name}-{stamp}"));
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

    fn seed_root(root: &Path, config: Option<&str>) -> PathBuf {
        let bin_dir = root.join("bin");
        fs::create_dir_all(&bin_dir).expect("bin dir");
        write_exe(
            &bin_dir.join("tmux"),
            r#"#!/bin/sh
printf '%s\n' "$*" >> "$MAW_FAKE_TMUX_LOG"
case "$1" in
  display-message) printf '50-mawjs\n' ;;
  list-windows) printf '%s' "$MAW_FAKE_TMUX_WINDOWS" ;;
  new-window|send-keys|select-window) exit 0 ;;
  capture-pane) printf '$ \r\n' ;;
  *) printf 'unexpected tmux %s\n' "$1" >&2; exit 9 ;;
esac
"#,
        );
        write_exe(
            &bin_dir.join("git"),
            r#"#!/bin/sh
printf '%s\n' "$*" >> "$MAW_FAKE_GIT_LOG"
if [ "$3" = "branch" ]; then exit 1; fi
if [ "$3" = "worktree" ] && [ "$4" = "list" ] && [ "$5" = "--porcelain" ]; then
  printf 'worktree %s\nHEAD 0000000000000000000000000000000000000000\nbranch refs/heads/main\n\n' "$2"
  exit 0
fi
if [ "$3" = "worktree" ] && [ "$4" = "add" ]; then
  /bin/mkdir -p "$5/.maw" "$5/.git"
  /bin/printf '{}\n' > "$5/.maw/phase.json"
  /bin/printf '{}\n' > "$5/.maw/delivery.json"
  /bin/printf '\n' > "$5/.maw/l1-review-request.json"
  /bin/printf '\n' > "$5/.maw/delivery-notified"
  /bin/printf '\n' > "$5/.maw/done-pinged"
  /bin/printf '\n' > "$5/.git/index.lock"
  /bin/printf 'stale\n' > "$5/stale.tmp"
  exit 0
fi
if [ "$3" = "clean" ] && [ "$4" = "-fd" ]; then exit 0; fi
if [ "$3" = "for-each-ref" ]; then exit 0; fi
printf 'unexpected git args: %s\n' "$*" >&2
exit 9
"#,
        );

        let repo = root.join("ghq/github.com/acme/demo");
        fs::create_dir_all(repo.join(".maw")).expect("repo dirs");
        fs::write(repo.join(".git"), "gitdir: main\n").expect("git marker");
        fs::write(repo.join("CLAUDE.md"), "main claude\n").expect("claude");
        fs::write(repo.join(".maw/delivery.json"), "{}\n").expect("delivery");
        if let Some(config) = config {
            let config_dir = root.join("xdg-config/maw");
            fs::create_dir_all(&config_dir).expect("config dir");
            fs::write(config_dir.join("maw.config.json"), config).expect("config");
        }
        bin_dir
    }

    fn run(root: &Path, bin_dir: &Path, args: &[&str]) -> Output {
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
            .env("GHQ_ROOT", root.join("ghq"))
            .env("TMUX", "/tmp/tmux-1000/default,123,0")
            .env("MAW_FAKE_TMUX_LOG", root.join("tmux.log"))
            .env("MAW_FAKE_TMUX_WINDOWS", "shell\n")
            .env("MAW_FAKE_GIT_LOG", root.join("git.log"));
        command.output().expect("run maw-rs")
    }

    fn assert_success(output: &Output) {
        assert!(
            output.status.success(),
            "stdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn sent_command(root: &Path) -> String {
        fs::read_to_string(root.join("tmux.log")).expect("tmux log")
    }

    #[test]
    fn fresh_worktree_creates_agents_dir() {
        let root = temp_dir("fresh-clean");
        let bin_dir = seed_root(&root, Some(r#"{"commands":{"default":"claude"}}"#));

        let output = run(&root, &bin_dir, &["workon", "demo", "feat"]);

        assert_success(&output);
        let git_log = fs::read_to_string(root.join("git.log")).expect("git log");
        assert!(git_log.contains("worktree add"), "{git_log}");
    }

    #[test]
    fn engine_resolves_configured_command() {
        let root = temp_dir("untrusted-engine");
        let bin_dir = seed_root(
            &root,
            Some(r#"{"commands":{"default":"codex exec"},"trustedRepos":["acme/other"]}"#),
        );

        let output = run(&root, &bin_dir, &["workon", "demo"]);

        assert_success(&output);
        assert!(sent_command(&root).contains("send-keys -t 50-mawjs:demo -l codex exec"));
    }

    #[test]
    fn explicit_omx_engine_drives_both_launch_and_trust_audit() {
        let root = temp_dir("explicit-omx-engine");
        let bin_dir = seed_root(
            &root,
            Some(
                r#"{"commands":{"default":"claude","omx":"omx --xhigh"},"engineTrustedRepos":{"omx":["acme/demo"]}}"#,
            ),
        );

        let output = run(
            &root,
            &bin_dir,
            &["workon", "demo", "issue-42", "--engine", "omx"],
        );

        assert_success(&output);
        assert!(sent_command(&root).contains("send-keys -t 50-mawjs:demo-issue-42 -l omx --xhigh"));
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(!stdout.contains("non-Claude engine 'claude'"), "{stdout}");
        assert!(!stdout.contains("not trusted"), "{stdout}");
    }

    #[test]
    fn fresh_worktree_sanitizes_stale_state() {
        let root = temp_dir("fresh-sanitize");
        let bin_dir = seed_root(&root, Some(r#"{"commands":{"default":"claude"}}"#));
        let output = run(&root, &bin_dir, &["workon", "demo", "feat"]);
        assert_success(&output);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("sanitized worktree"),
            "expected sanitize line; stdout={stdout}"
        );
        assert!(
            stdout.contains(".maw/phase.json"),
            "should scrub stale phase.json; stdout={stdout}"
        );
        assert!(
            stdout.contains(".git/index.lock"),
            "should scrub stale index.lock; stdout={stdout}"
        );
        assert!(
            stdout.contains(".gitignore: added maw ephemeral markers block"),
            "should inject gitignore block; stdout={stdout}"
        );
    }

    #[test]
    fn untrusted_engine_warns_and_trusted_does_not() {
        // Wiring guard: prepare_engine's trust warning must reach stdout for a
        // non-Claude engine on an untrusted repo, and stay silent when trusted.
        let untrusted_root = temp_dir("engine-untrusted");
        let bin_dir = seed_root(
            &untrusted_root,
            Some(r#"{"commands":{"default":"codex exec"},"trustedRepos":["acme/other"]}"#),
        );
        let output = run(&untrusted_root, &bin_dir, &["workon", "demo"]);
        assert_success(&output);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("not trusted"),
            "expected trust warning; stdout={stdout}"
        );
        assert!(
            stdout.contains("codex"),
            "warning names the engine; stdout={stdout}"
        );

        let trusted_root = temp_dir("engine-trusted");
        let bin_dir = seed_root(
            &trusted_root,
            Some(r#"{"commands":{"default":"codex exec"},"trustedRepos":["acme/demo"]}"#),
        );
        let output = run(&trusted_root, &bin_dir, &["workon", "demo"]);
        assert_success(&output);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            !stdout.contains("not trusted"),
            "trusted repo must not warn; stdout={stdout}"
        );
    }

    #[test]
    fn engine_resolution_fallback_chain() {
        let root = temp_dir("specific");
        let bin_dir = seed_root(
            &root,
            Some(
                r#"{"commands":{"demo-feat":"CODEX_HOME=$PWD/.codex codex exec","default":"claude --continue"},"trustedRepos":["acme/demo"]}"#,
            ),
        );
        assert_success(&run(&root, &bin_dir, &["workon", "demo", "feat"]));
        assert!(sent_command(&root).contains("CODEX_HOME=$PWD/.codex codex exec"));

        let root = temp_dir("default");
        let bin_dir = seed_root(
            &root,
            Some(r#"{"commands":{"default":"omx --direct"},"trustedRepos":["acme/demo"]}"#),
        );
        assert_success(&run(&root, &bin_dir, &["workon", "demo", "task"]));
        assert!(sent_command(&root).contains("send-keys -t 50-mawjs:demo-task -l omx --direct"));

        let root = temp_dir("fallback");
        let bin_dir = seed_root(&root, None);
        assert_success(&run(&root, &bin_dir, &["workon", "demo", "plain"]));
        assert!(sent_command(&root).contains("send-keys -t 50-mawjs:demo-plain -l claude"));
    }
}
