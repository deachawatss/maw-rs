#![forbid(unsafe_code)]

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
            output.stdout.contains("2 agents, 1 zombies"),
            "{}",
            output.stdout
        );
        assert!(output.stdout.contains("Zombies: 1"), "{}", output.stdout);

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
