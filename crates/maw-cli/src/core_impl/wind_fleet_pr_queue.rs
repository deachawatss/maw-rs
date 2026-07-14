const WIND_PR_QUEUE_USAGE: &str = "usage: maw fleet pr-queue [--no-reconcile]";
const WIND_PR_QUEUE_GHOST_GRACE_MS: u64 = 10 * 60 * 1_000;

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct WindPrQueueEntry {
    ts: u64,
    from: String,
    repo: String,
    pr_numbers: Vec<u64>,
    branch: String,
    status: String,
    #[serde(rename = "merged_at", skip_serializing_if = "Option::is_none")]
    merged_at: Option<u64>,
    #[serde(rename = "closed_at", skip_serializing_if = "Option::is_none")]
    closed_at: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WindPrQueueStatus {
    Merged,
    Closed,
}

#[derive(Debug, Default)]
struct WindPrQueueObservation {
    state: Option<String>,
    gone: bool,
}

#[derive(Debug)]
struct WindPrQueueUpdate {
    repo: String,
    pr_number: u64,
    status: WindPrQueueStatus,
    stamp_ms: u64,
}

#[derive(Debug, Default)]
struct WindPrQueueReconciliation {
    cleared: usize,
    ghosts: usize,
    unresolved: usize,
}

fn wind_fleet_pr_queue(argv: &[String]) -> Option<Result<(i32, String), String>> {
    (argv.first().is_some_and(|arg| arg == "pr-queue"))
        .then(|| wind_pr_queue_run(&argv[1..]))
}

fn wind_pr_queue_run(argv: &[String]) -> Result<(i32, String), String> {
    let mut reconcile = true;
    for arg in argv {
        match arg.as_str() {
            "--no-reconcile" => reconcile = false,
            "--help" | "-h" => return Ok((0, format!("{WIND_PR_QUEUE_USAGE}\n"))),
            _ => return Err(format!("fleet pr-queue: unknown argument {arg}\n{WIND_PR_QUEUE_USAGE}")),
        }
    }

    let root = wind_pr_queue_root()?;
    let mut pending = wind_pr_queue_pending(&root)?;
    let mut output = String::new();
    if reconcile && !pending.is_empty() {
        let reconciliation = wind_pr_queue_reconcile(&root, &pending)?;
        if reconciliation.cleared > 0 {
            let _ = writeln!(
                output,
                "  \x1b[32m✓\x1b[0m reconciled {} PR(s) against GitHub (merged/closed cleared)",
                reconciliation.cleared
            );
        }
        if reconciliation.ghosts > 0 {
            let _ = writeln!(
                output,
                "  \x1b[33m✓\x1b[0m dequeued {} ghost entr(ies) — PR unresolvable on GitHub (deleted / bad number)",
                reconciliation.ghosts
            );
        }
        if reconciliation.unresolved > 0 {
            let _ = writeln!(
                output,
                "  \x1b[33m⚠\x1b[0m {} entr(ies) skipped — repo not found under ghq, left pending",
                reconciliation.unresolved
            );
        }
        pending = wind_pr_queue_pending(&root)?;
    }

    if pending.is_empty() {
        output.push_str("  \x1b[32m✓\x1b[0m No pending PRs in queue.\n");
        return Ok((0, output));
    }

    let _ = write!(
        output,
        "\n  \x1b[34m\x1b[1mPR Queue\x1b[0m  {} pending\n\n",
        pending.len()
    );
    let now = wind_pr_queue_now_ms();
    for entry in pending {
        let age = now.saturating_sub(entry.ts).saturating_add(30_000) / 60_000;
        let numbers = entry
            .pr_numbers
            .iter()
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(
            output,
            "  \x1b[33m●\x1b[0m {} PR {numbers} — from {} ({age}m ago, branch: {})",
            wind_pr_queue_display(&entry.repo),
            wind_pr_queue_display(&entry.from),
            wind_pr_queue_display(&entry.branch)
        );
    }
    output.push('\n');
    Ok((0, output))
}

fn wind_pr_queue_root() -> Result<std::path::PathBuf, String> {
    std::env::var_os("MAW_STATE_DIR")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".maw")))
        .ok_or_else(|| "fleet pr-queue: HOME/MAW_STATE_DIR unavailable".to_owned())
}

fn wind_pr_queue_entries(root: &std::path::Path) -> Result<Vec<WindPrQueueEntry>, String> {
    let path = root.join("pr-queue.jsonl");
    let body = match std::fs::read_to_string(&path) {
        Ok(body) => body,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("fleet pr-queue: read {}: {error}", path.display())),
    };
    let entries = body
        .lines()
        .map(serde_json::from_str::<WindPrQueueEntry>)
        .collect::<Result<Vec<_>, _>>();
    Ok(entries.unwrap_or_default())
}

fn wind_pr_queue_pending(root: &std::path::Path) -> Result<Vec<WindPrQueueEntry>, String> {
    let mut seen = std::collections::BTreeSet::new();
    Ok(wind_pr_queue_entries(root)?
        .into_iter()
        .filter(|entry| entry.status == "pending" && wind_pr_queue_valid_repo(&entry.repo) && !entry.pr_numbers.is_empty())
        .filter(|entry| seen.insert(wind_pr_queue_key(entry)))
        .collect())
}

fn wind_pr_queue_key(entry: &WindPrQueueEntry) -> String {
    let mut numbers = entry.pr_numbers.clone();
    numbers.sort_unstable();
    format!("{}#{}", entry.repo, numbers.iter().map(u64::to_string).collect::<Vec<_>>().join(","))
}

fn wind_pr_queue_reconcile(
    root: &std::path::Path,
    pending: &[WindPrQueueEntry],
) -> Result<WindPrQueueReconciliation, String> {
    let now = wind_pr_queue_now_ms();
    let ghq_root = wind_pr_queue_ghq_root();
    let mut reconciliation = WindPrQueueReconciliation::default();
    let mut updates = Vec::new();

    for entry in pending {
        let Some((owner, repo)) = wind_pr_queue_repo_owner(ghq_root.as_deref(), &entry.repo) else {
            reconciliation.unresolved += 1;
            continue;
        };
        for pr_number in &entry.pr_numbers {
            let observed = wind_pr_queue_gh_state(&owner, &repo, *pr_number);
            let Some(status) = wind_pr_queue_classify(&observed, entry.ts, now) else {
                continue;
            };
            if status == WindPrQueueStatus::Closed && observed.gone && observed.state.as_deref() != Some("CLOSED") {
                reconciliation.ghosts += 1;
            }
            updates.push(WindPrQueueUpdate {
                repo: entry.repo.clone(),
                pr_number: *pr_number,
                status,
                stamp_ms: now,
            });
        }
    }
    reconciliation.cleared = wind_pr_queue_apply_updates(root, &updates)?;
    Ok(reconciliation)
}

fn wind_pr_queue_ghq_root() -> Option<std::path::PathBuf> {
    let root = std::env::var_os("GHQ_ROOT").map(std::path::PathBuf::from).or_else(|| {
        std::process::Command::new("ghq")
            .arg("root")
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| std::path::PathBuf::from(String::from_utf8_lossy(&output.stdout).trim()))
    })?;
    Some(if root.file_name().is_some_and(|name| name == "github.com") {
        root
    } else {
        root.join("github.com")
    })
}

fn wind_pr_queue_repo_owner(ghq_root: Option<&std::path::Path>, repo: &str) -> Option<(String, String)> {
    let repo = repo.strip_prefix("github.com/").unwrap_or(repo);
    if let Some((owner, name)) = repo.split_once('/') {
        return (wind_pr_queue_safe_segment(owner) && wind_pr_queue_safe_segment(name))
            .then(|| (owner.to_owned(), name.to_owned()));
    }
    if !wind_pr_queue_safe_segment(repo) {
        return None;
    }
    let root = ghq_root?;
    std::fs::read_dir(root).ok()?.flatten().find_map(|entry| {
        let owner = entry.file_name().to_string_lossy().into_owned();
        (wind_pr_queue_safe_segment(&owner) && root.join(&owner).join(repo).join(".git").exists())
            .then(|| (owner, repo.to_owned()))
    })
}

fn wind_pr_queue_safe_segment(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn wind_pr_queue_valid_repo(repo: &str) -> bool {
    let repo = repo.strip_prefix("github.com/").unwrap_or(repo);
    repo.split_once('/').map_or_else(
        || wind_pr_queue_safe_segment(repo),
        |(owner, name)| wind_pr_queue_safe_segment(owner) && wind_pr_queue_safe_segment(name) && !name.contains('/'),
    )
}

fn wind_pr_queue_gh_state(owner: &str, repo: &str, pr_number: u64) -> WindPrQueueObservation {
    let target = format!("{owner}/{repo}");
    let output = std::process::Command::new("gh")
        .args(["pr", "view", &pr_number.to_string(), "--repo", &target, "--json", "state,mergedAt"])
        .output();
    let Ok(output) = output else {
        return WindPrQueueObservation::default();
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
        return WindPrQueueObservation {
            state: None,
            gone: stderr.contains("could not resolve to a pull request") || stderr.contains("could not resolve to a pullrequest") || stderr.contains("no pull requests found"),
        };
    }
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&output.stdout) else {
        return WindPrQueueObservation::default();
    };
    WindPrQueueObservation {
        state: value.get("state").and_then(serde_json::Value::as_str).map(ToOwned::to_owned),
        gone: false,
    }
}

fn wind_pr_queue_classify(
    observed: &WindPrQueueObservation,
    entry_ts: u64,
    now: u64,
) -> Option<WindPrQueueStatus> {
    match observed.state.as_deref() {
        Some("MERGED") => Some(WindPrQueueStatus::Merged),
        Some("CLOSED") => Some(WindPrQueueStatus::Closed),
        _ if observed.gone && now.saturating_sub(entry_ts) > WIND_PR_QUEUE_GHOST_GRACE_MS => Some(WindPrQueueStatus::Closed),
        _ => None,
    }
}

fn wind_pr_queue_apply_updates(root: &std::path::Path, updates: &[WindPrQueueUpdate]) -> Result<usize, String> {
    if updates.is_empty() {
        return Ok(0);
    }
    let mut entries = wind_pr_queue_entries(root)?;
    let mut changed = 0;
    for entry in &mut entries {
        if entry.status != "pending" {
            continue;
        }
        let Some(update) = updates.iter().find(|update| update.repo == entry.repo && entry.pr_numbers.contains(&update.pr_number)) else {
            continue;
        };
        let status = match update.status {
            WindPrQueueStatus::Merged => "merged",
            WindPrQueueStatus::Closed => "closed",
        };
        entry.status.clear();
        entry.status.push_str(status);
        match update.status {
            WindPrQueueStatus::Merged => entry.merged_at = Some(update.stamp_ms),
            WindPrQueueStatus::Closed => entry.closed_at = Some(update.stamp_ms),
        }
        changed += 1;
    }
    if changed > 0 {
        wind_pr_queue_write(root, &entries)?;
    }
    Ok(changed)
}

fn wind_pr_queue_write(root: &std::path::Path, entries: &[WindPrQueueEntry]) -> Result<(), String> {
    std::fs::create_dir_all(root).map_err(|error| format!("fleet pr-queue: create {}: {error}", root.display()))?;
    let body = entries
        .iter()
        .map(serde_json::to_string)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("fleet pr-queue: render queue: {error}"))?
        .join("\n")
        + "\n";
    let path = root.join("pr-queue.jsonl");
    std::fs::write(&path, body).map_err(|error| format!("fleet pr-queue: write {}: {error}", path.display()))
}

fn wind_pr_queue_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
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
    fn fleet_pr_queue_dedupes_post_tool_double_writes_by_sorted_pr_numbers() {
        let _guard = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _state = EnvVarRestore::capture("MAW_STATE_DIR");
        let root = temp_root();
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("state root");
        std::fs::write(
            root.join("pr-queue.jsonl"),
            concat!(
                "{\"ts\":1700000000000,\"from\":\"gale\",\"repo\":\"maw-js\",\"prNumbers\":[77,88],\"branch\":\"agents/issue-2-first\",\"status\":\"pending\"}\n",
                "{\"ts\":1700000001000,\"from\":\"post-tool\",\"repo\":\"maw-js\",\"prNumbers\":[88,77],\"branch\":\"agents/issue-2-second\",\"status\":\"pending\"}\n"
            ),
        )
        .expect("queue fixture");
        std::env::set_var("MAW_STATE_DIR", &root);

        let output = run_cli(&args(&["fleet", "pr-queue", "--no-reconcile"]));

        assert_eq!(output.code, 0, "{}", output.stderr);
        assert!(output.stdout.contains("1 pending"), "{}", output.stdout);
        assert!(output.stdout.contains("from gale"), "{}", output.stdout);
        assert!(!output.stdout.contains("post-tool"), "{}", output.stdout);
        std::fs::remove_dir_all(root).expect("cleanup temp state");
    }

    #[test]
    fn old_ghosts_dequeue_but_new_or_transient_entries_stay_pending() {
        let now = 1_700_000_000_000;
        let old = now - 20 * 60 * 1_000;
        let fresh = now - 60 * 1_000;

        assert_eq!(
            wind_pr_queue_classify(&WindPrQueueObservation { state: None, gone: true }, old, now),
            Some(WindPrQueueStatus::Closed)
        );
        assert_eq!(
            wind_pr_queue_classify(&WindPrQueueObservation { state: None, gone: true }, fresh, now),
            None
        );
        assert_eq!(
            wind_pr_queue_classify(&WindPrQueueObservation::default(), old, now),
            None
        );
        assert_eq!(
            wind_pr_queue_classify(
                &WindPrQueueObservation {
                    state: Some("MERGED".to_owned()),
                    gone: false,
                },
                fresh,
                now,
            ),
            Some(WindPrQueueStatus::Merged)
        );
    }

    #[test]
    fn reconciliation_marks_every_duplicate_entry_closed() {
        let root = temp_root();
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("state root");
        std::fs::write(
            root.join("pr-queue.jsonl"),
            concat!(
                "{\"ts\":1700000000000,\"from\":\"gale\",\"repo\":\"maw-js\",\"prNumbers\":[77],\"branch\":\"agents/issue-2-first\",\"status\":\"pending\"}\n",
                "{\"ts\":1700000001000,\"from\":\"post-tool\",\"repo\":\"maw-js\",\"prNumbers\":[77],\"branch\":\"agents/issue-2-second\",\"status\":\"pending\"}\n"
            ),
        )
        .expect("queue fixture");

        let changed = wind_pr_queue_apply_updates(
            &root,
            &[WindPrQueueUpdate {
                repo: "maw-js".to_owned(),
                pr_number: 77,
                status: WindPrQueueStatus::Closed,
                stamp_ms: 1_700_000_000_000,
            }],
        )
        .expect("reconcile queue");

        assert_eq!(changed, 2);
        assert!(wind_pr_queue_pending(&root).expect("pending queue").is_empty());
        let rows = std::fs::read_to_string(root.join("pr-queue.jsonl")).expect("rewritten queue");
        assert_eq!(rows.matches("\"status\":\"closed\"").count(), 2);
        assert_eq!(rows.matches("\"closed_at\":1700000000000").count(), 2);
        std::fs::remove_dir_all(root).expect("cleanup temp state");
    }
}
