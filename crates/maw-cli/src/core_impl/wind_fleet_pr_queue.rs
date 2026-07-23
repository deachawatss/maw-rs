const WIND_PR_QUEUE_USAGE: &str = "usage: maw fleet pr-queue";

fn wind_fleet_pr_queue(argv: &[String]) -> Option<Result<(i32, String), String>> {
    (argv.first().is_some_and(|arg| arg == "pr-queue"))
        .then(|| wind_pr_queue_run(&argv[1..]))
}

fn wind_pr_queue_run(argv: &[String]) -> Result<(i32, String), String> {
    if let Some(arg) = argv.first() {
        match arg.as_str() {
            "--help" | "-h" => return Ok((0, format!("{WIND_PR_QUEUE_USAGE}\n"))),
            _ => return Err(format!("fleet pr-queue: unknown argument {arg}\n{WIND_PR_QUEUE_USAGE}")),
        }
    }

    pr_reconcile_reviews(&mut PrNativeProcess, true)?;
    let pending = pr_load_global_reviews(&pr_review_queue_root()?)?;
    let l2_events = l2_drain_events()?;
    if pending.is_empty() && l2_events.is_empty() {
        return Ok((0, "  \x1b[32m✓\x1b[0m No pending PRs in queue.\n".to_owned()));
    }

    let mut output = String::new();
    if !l2_events.is_empty() {
        let _ = writeln!(output, "\n  \x1b[34m\x1b[1mL2 Events\x1b[0m  {} new\n", l2_events.len());
        for event in &l2_events { let _ = writeln!(output, "  \x1b[33m●\x1b[0m {}", event.message); }
        output.push('\n');
    }
    if !pending.is_empty() {
        let _ = writeln!(output, "\n  \x1b[34m\x1b[1mPR Queue\x1b[0m  {} pending\n", pending.len());
    }
    for request in pending {
        let _ = writeln!(
            output,
            "  \x1b[33m●\x1b[0m {} PR {} — branch: {}",
            wind_pr_queue_display(&request.repo),
            request.pr_number,
            wind_pr_queue_display(&request.branch),
        );
    }
    if !output.ends_with("\n\n") { output.push('\n'); }
    l2_acknowledge_events(&l2_events)?;
    Ok((0, output))
}

fn wind_pr_queue_display(value: &str) -> String {
    value.chars().filter(|character| !character.is_control()).collect()
}

#[cfg(test)]
mod wind_fleet_pr_queue_tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    fn temp_root() -> std::path::PathBuf {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let sequence = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "maw-rs-wind-pr-queue-{}-{sequence}",
            std::process::id()
        ))
    }

    #[test]
    fn fleet_pr_queue_auto_path_archives_open_and_quarantines_fixture_entries() {
        use std::os::unix::fs::PermissionsExt;

        let _guard = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _state = EnvVarRestore::capture("MAW_STATE_DIR");
        let _path = EnvVarRestore::capture("PATH");
        let root = temp_root();
        let _ = std::fs::remove_dir_all(&root);
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).expect("fake gh directory");
        let gh = bin.join("gh");
        std::fs::write(
            &gh,
            concat!(
                "#!/bin/sh\n",
                "case \"$3\" in\n",
                "  71) printf 'MERGED\\n' ;;\n",
                "  72) printf 'OPEN\\n' ;;\n",
                "  73) printf 'repository is inaccessible' >&2; exit 1 ;;\n",
                "esac\n"
            ),
        )
        .expect("fake gh");
        std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).expect("make fake gh executable");
        let original_path = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", format!("{}:{original_path}", bin.display()));
        std::env::set_var("MAW_STATE_DIR", &root);
        std::fs::write(
            root.join("pr-queue.jsonl"),
            concat!(
                "{\"version\":1,\"prUrl\":\"https://github.com/acme/demo/pull/71\",\"prNumber\":71,\"repo\":\"acme/demo\",\"branch\":\"agents/issue-71\",\"status\":\"pending\",\"notified\":false,\"notifiedAt\":null,\"notifier\":null}\n",
                "{\"version\":1,\"prUrl\":\"https://github.com/acme/demo/pull/72\",\"prNumber\":72,\"repo\":\"acme/demo\",\"branch\":\"agents/issue-72\",\"status\":\"pending\",\"notified\":false,\"notifiedAt\":null,\"notifier\":null}\n",
                "{\"version\":1,\"prUrl\":\"https://github.com/acme/demo/pull/73\",\"prNumber\":73,\"repo\":\"acme/demo\",\"branch\":\"agents/issue-73\",\"status\":\"pending\",\"notified\":false,\"notifiedAt\":null,\"notifier\":null,\"reconcileAttempts\":2}\n"
            ),
        )
        .expect("queue fixture");

        let output = run_cli(&args(&["fleet", "pr-queue"]));

        assert_eq!(output.code, 0, "{}", output.stderr);
        assert!(output.stdout.contains("1 pending"), "{}", output.stdout);
        let queue = std::fs::read_to_string(root.join("pr-queue.jsonl")).expect("remaining queue");
        let queue_rows = queue.lines().map(|line| serde_json::from_str::<serde_json::Value>(line).expect("queue row")).collect::<Vec<_>>();
        assert_eq!(queue_rows.len(), 2);
        assert!(queue_rows.iter().any(|row| row["prNumber"] == 72 && row["status"] == "pending"));
        assert!(queue_rows.iter().any(|row| {
            row["prNumber"] == 73
                && row["status"] == "unresolvable"
                && row["reconcileAttempts"] == 3
                && row["lastReconcileError"].as_str().is_some_and(|error| error.contains("exit 1"))
        }));
        let archived = std::fs::read_to_string(root.join("pr-queue.jsonl.archived")).expect("archive");
        let archived_row = serde_json::from_str::<serde_json::Value>(archived.trim()).expect("archive row");
        assert_eq!(archived_row["prNumber"], 71);
        assert_eq!(archived_row["status"], "merged");
        std::fs::remove_dir_all(root).expect("cleanup temp state");
    }
}
