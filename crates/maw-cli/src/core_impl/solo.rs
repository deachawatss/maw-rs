const DISPATCH_381: &[DispatcherEntry] = &[DispatcherEntry {
    command: "solo",
    handler: Handler::Sync(solo_run_command),
}];

const SOLO_USAGE: &str = "usage: maw solo <repo> <issue-slug> [--profile <name>]";

struct SoloLeaseTarget<'a> {
    path: &'a std::path::Path,
    holder: &'a str,
}

fn solo_run_command(argv: &[String]) -> CliOutput {
    match solo_run(argv, &mut maw_tmux::CommandTmuxRunner::new()) {
        Ok(stdout) => CliOutput { code: 0, stdout, stderr: String::new() },
        Err(message) => CliOutput { code: 1, stdout: String::new(), stderr: format!("{message}\n") },
    }
}

fn solo_run<R: maw_tmux::TmuxRunner>(argv: &[String], runner: &mut R) -> Result<String, String> {
    let (repo_arg, slug, profile) = solo_parse_args(argv)?;
    let repo = workon_resolve_repo(&repo_arg)?;
    let pane = crate::wind::team::caller_pane().ok_or_else(|| "solo: run from a tmux pane so maw can use the current session".to_owned())?;
    let session = workon_tmux_run(runner, "display-message", &["-t", &pane, "-p", "#{session_name}"])?;
    workon_validate_tmux_target(&session)?;
    let window_name = format!("{}-{}", repo.repo_name, workon_sanitize_task_slug(&slug));
    let holder = format!("{session}:{window_name}");
    let lease = solo_lease_path(&repo.repo_name);
    solo_acquire_lease(&lease, &holder, |existing| solo_holder_live(runner, existing))?;

    let result = solo_create_window(
        runner,
        &repo,
        &slug,
        profile.as_deref(),
        &session,
        &window_name,
        &SoloLeaseTarget { path: &lease, holder: &holder },
    );
    solo_finish(&lease, result)
}

fn solo_finish(lease: &std::path::Path, result: Result<String, String>) -> Result<String, String> {
    match result {
        Ok(output) => Ok(output),
        Err(error) => match std::fs::remove_file(lease) {
            Ok(()) => Err(error),
            Err(release_error) if release_error.kind() == std::io::ErrorKind::NotFound => Err(error),
            Err(release_error) => Err(format!("{error}\nsolo: release lease {}: {release_error}", lease.display())),
        },
    }
}

fn solo_parse_args(argv: &[String]) -> Result<(String, String, Option<String>), String> {
    let mut positional = Vec::new();
    let mut profile = None;
    let mut index = 0;
    while index < argv.len() {
        match argv[index].as_str() {
            "--profile" => {
                let Some(value) = argv.get(index + 1) else { return Err("solo: --profile requires a value".to_owned()); };
                workon_validate_query(value, "profile")?;
                profile = Some(value.clone());
                index += 2;
            }
            value if value.starts_with("--profile=") => {
                let value = value.trim_start_matches("--profile=");
                workon_validate_query(value, "profile")?;
                profile = Some(value.to_owned());
                index += 1;
            }
            value if value.starts_with('-') => return Err(SOLO_USAGE.to_owned()),
            value => { positional.push(value.to_owned()); index += 1; }
        }
    }
    let [repo, slug] = positional.as_slice() else { return Err(SOLO_USAGE.to_owned()) };
    workon_validate_query(repo, "repo")?;
    workon_validate_slug_input(slug, "issue-slug")?;
    Ok((repo.clone(), slug.clone(), profile))
}

fn solo_create_window<R: maw_tmux::TmuxRunner>(
    runner: &mut R,
    repo: &WorkonRepo,
    slug: &str,
    profile: Option<&str>,
    session: &str,
    window_name: &str,
    lease: &SoloLeaseTarget<'_>,
) -> Result<String, String> {
    let options = WorkonOptions { repo: repo.repo_name.clone(), task: Some(slug.to_owned()), wt: None, fresh: false, name: None, base: None, engine: Some("codex".to_owned()), profile: None, layout: WorkonLayout::Nested, prompt: None, oracle: None };
    let request = workon_resolve_worktree_name(&options)?.ok_or_else(|| "solo: missing worktree slug".to_owned())?;
    let worktrees = workon_find_worktrees(&repo.parent_dir, &repo.repo_name);
    let branches = workon_agent_branches(&repo.repo_path)?;
    let target_path = match workon_plan_worktree(repo, &request, false, WorkonLayout::Nested, &worktrees, &branches)? {
        WorkonWorktreePlan::Reuse { path } => path,
        WorkonWorktreePlan::Create { wt_path, branch, branch_exists, .. } => {
            workon_create_worktree(repo, &wt_path, &branch, branch_exists, None, WorkonLayout::Nested)?;
            workon_finish_created_worktree(repo, &wt_path, &mut String::new())?;
            wt_path
        }
    };
    solo_set_lease_worktree(lease.path, lease.holder, &target_path)?;
    let codex_pane = workon_new_window(runner, session, window_name, &target_path)?;
    workon_record_pane_id(&target_path, &codex_pane)?;
    workon_wait_for_shell_prompt(runner, &codex_pane)?;
    solo_launch_l2(runner, &codex_pane, &target_path, profile)?;
    Ok(format!("solo '{window_name}' in {session} (L1: current pane, L2: {})\n", target_path.display()))
}

fn solo_launch_l2<R: maw_tmux::TmuxRunner>(
    runner: &mut R,
    pane: &str,
    target_path: &std::path::Path,
    profile: Option<&str>,
) -> Result<(), String> {
    let engine = solo_append_profile("codex", profile);
    if let Err(error) = workon_send_window_command_to_target(runner, pane, "codex", target_path, Some(&engine), None) {
        return Err(solo_launch_failure(runner, pane, &error));
    }
    workon_wait_for_launch(runner, pane).map_err(|error| solo_launch_failure(runner, pane, &error))
}

fn solo_launch_failure<R: maw_tmux::TmuxRunner>(runner: &mut R, pane: &str, error: &str) -> String {
    let capture = workon_tmux_run(runner, "capture-pane", &["-t", pane, "-p", "-S", "-20"]).unwrap_or_default();
    let message = format!("solo: L2 engine failed to start in {pane}: {error}");
    if capture.is_empty() { message } else { format!("{message}\npane output:\n{capture}") }
}

fn solo_append_profile(command: &str, profile: Option<&str>) -> String {
    if solo_is_codex_family(command) { profile.map_or_else(|| command.to_owned(), |profile| format!("{command} -p {profile}")) } else { command.to_owned() }
}

fn solo_is_codex_family(engine: &str) -> bool { engine.trim_start().starts_with("codex") }

fn solo_lease_path(repo_name: &str) -> std::path::PathBuf { maw_state_path(&current_xdg_env(), &["lease", &format!("{repo_name}.json")]) }

#[derive(Debug, Clone, PartialEq, Eq)]
struct SoloLease {
    holder: String,
    worktree: Option<std::path::PathBuf>,
}

fn solo_read_lease(path: &std::path::Path) -> Result<String, String> { Ok(solo_read_lease_record(path)?.holder) }

fn solo_read_lease_record(path: &std::path::Path) -> Result<SoloLease, String> {
    let value = std::fs::read_to_string(path).map_err(|error| format!("solo: read {}: {error}", path.display()))?;
    if let Ok(record) = serde_json::from_str::<serde_json::Value>(&value) {
        let holder = record.get("holder").and_then(serde_json::Value::as_str).filter(|holder| !holder.is_empty())
            .ok_or_else(|| format!("solo: invalid lease {}", path.display()))?;
        let worktree = record.get("worktree").and_then(serde_json::Value::as_str).filter(|path| !path.is_empty()).map(std::path::PathBuf::from);
        return Ok(SoloLease { holder: holder.to_owned(), worktree });
    }
    let holder = value.trim();
    if holder.is_empty() { return Err(format!("solo: invalid lease {}", path.display())); }
    Ok(SoloLease { holder: holder.to_owned(), worktree: None })
}

fn solo_set_lease_worktree(path: &std::path::Path, holder: &str, worktree: &std::path::Path) -> Result<(), String> {
    let record = solo_read_lease_record(path)?;
    if record.holder != holder { return Err("solo: lease holder changed before worktree registration".to_owned()); }
    let body = serde_json::json!({"holder": holder, "worktree": worktree}).to_string();
    std::fs::write(path, body).map_err(|error| format!("solo: record worktree lease: {error}"))
}

fn solo_acquire_lease(path: &std::path::Path, holder: &str, mut is_live: impl FnMut(&str) -> bool) -> Result<(), String> {
    if let Some(parent) = path.parent() { std::fs::create_dir_all(parent).map_err(|error| format!("solo: create lease directory: {error}"))?; }
    for _ in 0..2 {
        match std::fs::OpenOptions::new().write(true).create_new(true).open(path) {
            Ok(mut file) => return std::io::Write::write_all(&mut file, holder.as_bytes()).map_err(|error| format!("solo: write lease: {error}")),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let existing = solo_read_lease(path)?;
                if existing == holder { return Ok(()); }
                if is_live(&existing) { return Err(format!("solo: repo lease is held by {existing}")); }
                std::fs::remove_file(path).map_err(|error| format!("solo: release stale lease: {error}"))?;
            }
            Err(error) => return Err(format!("solo: acquire lease: {error}")),
        }
    }
    Err("solo: lease changed while acquiring; retry".to_owned())
}

fn solo_holder_live<R: maw_tmux::TmuxRunner>(runner: &mut R, holder: &str) -> bool {
    workon_tmux_run(runner, "list-windows", &["-a", "-F", "#{session_name}:#{window_name}"])
        .is_ok_and(|windows| windows.lines().any(|window| window == holder))
}

fn solo_release_holder(holder: &str) {
    let dir = maw_state_path(&current_xdg_env(), &["lease"]);
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if solo_read_lease(&path).ok().as_deref() == Some(holder) { let _ = std::fs::remove_file(path); }
    }
}

fn solo_worktree_for_holder(holder: &str) -> Option<std::path::PathBuf> {
    let entries = std::fs::read_dir(maw_state_path(&current_xdg_env(), &["lease"])).ok()?;
    entries.flatten().find_map(|entry| {
        solo_read_lease_record(&entry.path()).ok().and_then(|record| (record.holder == holder).then_some(record.worktree).flatten())
    })
}

fn solo_require_workon_session<R: maw_tmux::TmuxRunner>(
    repo_name: &str,
    session: Option<&str>,
    runner: &mut R,
) -> Result<(), String> {
    let path = solo_lease_path(repo_name);
    if !path.exists() { return Ok(()); }
    let holder = solo_read_lease(&path)?;
    if !solo_holder_live(runner, &holder) {
        return std::fs::remove_file(&path).map_err(|error| format!("workon: release stale repo lease: {error}"));
    }
    if solo_lease_allows_session(&holder, session) { return Ok(()); }
    Err(format!("workon: repo lease is held by {holder}"))
}

fn solo_lease_allows_session(holder: &str, session: Option<&str>) -> bool {
    holder.split_once(':').is_some_and(|(holder_session, _)| Some(holder_session) == session)
}

#[cfg(test)]
mod solo_tests {
    use super::*;

    struct SoloLaunchMockTmux {
        capture_responses: std::collections::VecDeque<String>,
    }

    impl maw_tmux::TmuxRunner for SoloLaunchMockTmux {
        fn run(&mut self, subcommand: &str, args: &[String]) -> Result<String, maw_tmux::TmuxError> {
            match subcommand {
                "display-message" if args.iter().any(|arg| arg == "#{pane_current_command}") => Ok("zsh\n".to_owned()),
                "display-message" => Ok("0\n".to_owned()),
                "capture-pane" => Ok(self.capture_responses.pop_front().unwrap_or_else(|| "codex: profile fast is invalid\n$".to_owned())),
                "send-keys" => Ok(String::new()),
                other => Err(maw_tmux::TmuxError::new(format!("unexpected {other}"))),
            }
        }
    }

    #[test]
    fn solo_profiles_are_forwarded_only_to_codex_family_engines() {
        assert_eq!(solo_append_profile("codex", Some("xhigh")), "codex -p xhigh");
        assert_eq!(solo_append_profile("codex-launch", Some("fast")), "codex-launch -p fast");
        assert_eq!(solo_append_profile("claude", Some("standard")), "claude");
        let parsed = workon_parse_args(&["repo".to_owned(), "--codex".to_owned(), "--profile".to_owned(), "xhigh".to_owned()]).expect("profile parse");
        assert_eq!(parsed.engine.as_deref(), Some("codex -p xhigh"));
    }

    #[test]
    fn solo_lease_conflict_and_stale_holder_reclamation() {
        let root = std::env::temp_dir().join(format!("maw-solo-lease-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let path = root.join("maw-rs.json");

        solo_acquire_lease(&path, "wind:maw-rs-73", |_| true).expect("first holder");
        let error = solo_acquire_lease(&path, "wind:maw-rs-74", |_| true).expect_err("active holder conflicts");
        assert!(error.contains("wind:maw-rs-73"), "{error}");
        solo_acquire_lease(&path, "wind:maw-rs-74", |_| false).expect("stale holder is reclaimed");
        assert_eq!(solo_read_lease(&path).expect("lease"), "wind:maw-rs-74");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn solo_stale_lease_is_reclaimed_only_when_its_holder_is_gone() {
        let root = std::env::temp_dir().join(format!("maw-solo-stale-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let path = root.join("maw-rs.json");
        solo_acquire_lease(&path, "gale:active", |_| true).expect("active lease");
        assert!(solo_acquire_lease(&path, "gale:new", |_| true).is_err());
        assert_eq!(solo_read_lease(&path).expect("still active"), "gale:active");
        solo_acquire_lease(&path, "gale:new", |_| false).expect("stale lease reclaimed");
        assert_eq!(solo_read_lease(&path).expect("new holder"), "gale:new");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn workon_from_a_different_session_is_refused_by_a_live_solo_lease() {
        assert!(solo_lease_allows_session("01-gale:maw-rs-revsolo", Some("01-gale")));
        assert!(!solo_lease_allows_session("01-gale:maw-rs-revsolo", Some("02-other")));
        assert!(!solo_lease_allows_session("01-gale:maw-rs-revsolo", None));
    }

    #[test]
    fn solo_failed_l2_launch_surfaces_pane_error_and_releases_lease() {
        let root = std::env::temp_dir().join(format!("maw-solo-launch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let lease = root.join("maw-rs.json");
        solo_acquire_lease(&lease, "01-gale:maw-rs-73", |_| true).expect("lease acquired");
        let mut runner = SoloLaunchMockTmux {
            capture_responses: std::collections::VecDeque::from(["$\n".to_owned(), "$\n".to_owned(), "codex: profile fast is invalid\n$".to_owned()]),
        };

        let error = solo_launch_l2(&mut runner, "%2", &root, Some("fast")).expect_err("failed engine launch");
        assert!(error.contains("profile fast is invalid"), "{error}");
        assert!(solo_finish(&lease, Err(error)).is_err());
        assert!(!lease.exists(), "failed launch releases its lease");
        let _ = std::fs::remove_dir_all(root);
    }
}
