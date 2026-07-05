#![forbid(unsafe_code)]

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
if [ "$3" = "worktree" ] && [ "$4" = "add" ]; then
  /bin/mkdir -p "$5/.maw" "$5/.git"
  /bin/printf '{}\n' > "$5/.maw/phase.json"
  /bin/printf '{}\n' > "$5/.maw/strategy.json"
  /bin/printf '\n' > "$5/.maw/solo-justified"
  /bin/printf '\n' > "$5/.maw/aggregate-verified"
  /bin/printf '\n' > "$5/.maw/done-pinged"
  /bin/printf '\n' > "$5/.git/index.lock"
  /bin/printf 'stale\n' > "$5/stale.tmp"
  exit 0
fi
if [ "$3" = "clean" ] && [ "$4" = "-fd" ]; then exit 0; fi
printf 'unexpected git args: %s\n' "$*" >&2
exit 9
"#,
        );

        let repo = root.join("ghq/github.com/acme/demo");
        fs::create_dir_all(repo.join(".maw")).expect("repo dirs");
        fs::write(repo.join(".git"), "gitdir: main\n").expect("git marker");
        fs::write(repo.join("CLAUDE.md"), "main claude\n").expect("claude");
        fs::write(repo.join(".maw/strategy.json"), "{}\n").expect("strategy");
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
    fn fresh_worktree_cleans_stale_state() {
        let root = temp_dir("fresh-clean");
        let bin_dir = seed_root(&root, Some(r#"{"commands":{"default":"claude"}}"#));

        let output = run(&root, &bin_dir, &["workon", "demo", "feat"]);

        assert_success(&output);
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
            "main claude\n"
        );
        let git_log = fs::read_to_string(root.join("git.log")).expect("git log");
        assert!(git_log.contains("worktree add"), "{git_log}");
        assert!(git_log.contains("clean -fd"), "{git_log}");
    }

    #[test]
    fn engine_warn_untrusted_repo() {
        let root = temp_dir("untrusted-engine");
        let bin_dir = seed_root(
            &root,
            Some(r#"{"commands":{"default":"codex exec"},"trustedRepos":["acme/other"]}"#),
        );

        let output = run(&root, &bin_dir, &["workon", "demo"]);

        assert_success(&output);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("non-Claude engine 'codex' is not trusted for acme/demo"),
            "{stdout}"
        );
        assert!(sent_command(&root).contains("send-keys -t 50-mawjs:demo -l codex exec"));
        let strategy = fs::read_to_string(root.join("ghq/github.com/acme/demo/.maw/strategy.json"))
            .expect("strategy");
        let strategy: serde_json::Value = serde_json::from_str(&strategy).expect("strategy json");
        assert_eq!(strategy["engine"], "codex");
        assert_eq!(strategy["engineCommand"], "codex exec");
        assert_eq!(strategy["engineWarned"], true);
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
