const DISPATCH_381: &[DispatcherEntry] = &[DispatcherEntry {
    command: "solo",
    handler: Handler::Sync(solo_run_command),
}];

const SOLO_USAGE: &str = "usage: maw solo <repo> <issue-slug> [--profile <name>]";

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

    let result = solo_create_window(runner, &repo, &slug, profile.as_deref(), &session, &window_name);
    if result.is_err() { let _ = std::fs::remove_file(&lease); }
    result
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
    let window = workon_new_window(runner, session, window_name, &repo.repo_path)?;
    workon_wait_for_shell_prompt(runner, &window)?;
    workon_send_window_command_to_target(runner, &window, "claude", &repo.repo_path, Some("claude"), Some("Issue delivery lead: inspect the issue and coordinate the Codex worktree pane."))?;
    let codex_pane = workon_tmux_run(runner, "split-window", &["-h", "-P", "-F", "#{pane_id}", "-t", &window, "-c", workon_path_str(&target_path)?])?;
    workon_wait_for_shell_prompt(runner, &codex_pane)?;
    solo_send_command(runner, &codex_pane, &solo_append_profile("codex", profile))?;
    Ok(format!("solo '{window_name}' in {session} (L1: {}, L2: {})\n", repo.repo_path.display(), target_path.display()))
}

fn solo_send_command<R: maw_tmux::TmuxRunner>(runner: &mut R, pane: &str, command: &str) -> Result<(), String> {
    runner.run("send-keys", &maw_tmux::tmux_send_keys_literal_args(pane, command)).map_err(|error| error.message)?;
    runner.run("send-keys", &maw_tmux::tmux_send_enter_args(pane)).map_err(|error| error.message)?;
    Ok(())
}

fn solo_append_profile(command: &str, profile: Option<&str>) -> String {
    if solo_is_codex_family(command) { profile.map_or_else(|| command.to_owned(), |profile| format!("{command} -p {profile}")) } else { command.to_owned() }
}

fn solo_is_codex_family(engine: &str) -> bool { engine.trim_start().starts_with("codex") }

fn solo_lease_path(repo_name: &str) -> std::path::PathBuf { maw_state_path(&current_xdg_env(), &["lease", &format!("{repo_name}.json")]) }

fn solo_read_lease(path: &std::path::Path) -> Result<String, String> {
    std::fs::read_to_string(path).map(|value| value.trim().to_owned()).map_err(|error| format!("solo: read {}: {error}", path.display()))
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

fn solo_require_unleased<R: maw_tmux::TmuxRunner>(repo_name: &str, runner: &mut R) -> Result<(), String> {
    let path = solo_lease_path(repo_name);
    if !path.exists() { return Ok(()); }
    let holder = solo_read_lease(&path)?;
    if solo_holder_live(runner, &holder) { return Err(format!("workon: repo lease is held by {holder}")); }
    std::fs::remove_file(&path).map_err(|error| format!("workon: release stale repo lease: {error}"))
}

#[cfg(test)]
mod solo_tests {
    use super::*;

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
}
