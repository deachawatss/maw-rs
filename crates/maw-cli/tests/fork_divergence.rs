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
  *"status --porcelain -- ψ/"*) if [ "$DONE_STATUS_PSI" = "test" ]; then printf '?? ψ/test.md
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
        assert_eq!(
            std::fs::read_to_string(root.join("capture-count")).expect("capture count"),
            "4"
        );
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
