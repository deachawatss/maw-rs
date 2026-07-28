#[derive(Debug, Clone, PartialEq, Eq)]
struct DoneSweepWorktree {
    path: std::path::PathBuf,
    branch: Option<String>,
}

fn done_pr_state_for_branch(main_path: &std::path::Path, branch: &str) -> Result<PrGithubState, String> {
    done_validate_exec_path(main_path)?;
    if !done_branch_name_allows_push(branch) {
        return Err(format!("done: invalid delivery branch {branch:?}"));
    }
    let output = std::process::Command::new("gh")
        .current_dir(main_path)
        .args(["pr", "view", branch, "--json", "number", "--jq", ".number"])
        .output()
        .map_err(|error| format!("done: resolve PR for {branch}: {error}"))?;
    if !output.status.success() {
        return Err(pr_command_failure(&format!("done: resolve PR for {branch}"), &output));
    }
    let number = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u64>()
        .map_err(|error| format!("done: invalid PR number for {branch}: {error}"))?;
    let mut process = PrNativeProcess;
    let remote = process.pr_git_remote_url(main_path, "origin")?;
    let repo = pr_github_repo_from_remote(&remote)?;
    process.pr_gh_review_state(&repo, number)
}

fn done_sweep_repo_after_removal(
    main_path: &std::path::Path,
    panes: &[DonePane],
    options: &DoneOptions,
    local: &mut impl DoneRuntime,
    stdout: &mut String,
) {
    let worktrees = match done_sweep_worktree_list(main_path, local) {
        Ok(worktrees) => worktrees,
        Err(error) => {
            let _ = writeln!(stdout, "  \x1b[33m⚠\x1b[0m stale-worktree sweep skipped: {error}");
            return;
        }
    };
    let mut removed_worktrees = 0_usize;
    for candidate in worktrees.into_iter().filter(|worktree| done_sweep_is_agents_worktree(main_path, &worktree.path)) {
        if done_sweep_has_live_pane(&candidate.path, panes) {
            let _ = writeln!(stdout, "  \x1b[90m○\x1b[0m sweep retained {} (live pane)", candidate.path.display());
            continue;
        }
        let Some(branch) = candidate.branch.as_deref() else {
            let _ = writeln!(stdout, "  \x1b[33m⚠\x1b[0m sweep retained {} (branch unavailable)", candidate.path.display());
            continue;
        };
        match local.done_pr_state(main_path, branch) {
            Ok(PrGithubState::Open) => {
                let _ = writeln!(stdout, "  \x1b[90m○\x1b[0m sweep retained {} (PR open)", candidate.path.display());
                continue;
            }
            Err(error) => {
                let _ = writeln!(stdout, "  \x1b[33m⚠\x1b[0m sweep retained {} (PR state unavailable: {error})", candidate.path.display());
                continue;
            }
            Ok(PrGithubState::Merged | PrGithubState::Closed) => {}
        }
        let worktree = DoneWorktree {
            main_path: main_path.to_path_buf(),
            full_path: candidate.path.clone(),
            label: candidate.path.display().to_string(),
        };
        done_rescue_psi_notes(&worktree, false, stdout);
        let remove_args = [
            "-C".to_owned(),
            main_path.display().to_string(),
            "worktree".to_owned(),
            "remove".to_owned(),
            "--".to_owned(),
            candidate.path.display().to_string(),
        ];
        match local.done_git(&remove_args) {
            Ok(_) => {
                removed_worktrees += 1;
                let _ = writeln!(stdout, "  \x1b[32m✓\x1b[0m sweep removed stale worktree {}", candidate.path.display());
                done_cleanup_branch(main_path, branch, options, local, stdout);
            }
            Err(error) => {
                let _ = writeln!(stdout, "  \x1b[33m⚠\x1b[0m sweep retained {} (worktree remove failed: {error})", candidate.path.display());
            }
        }
    }
    if let Err(error) = done_sweep_prune(main_path, local) {
        let _ = writeln!(stdout, "  \x1b[33m⚠\x1b[0m stale-worktree sweep prune failed: {error}");
    }
    let removed_branches = done_sweep_merged_agent_branches(main_path, options, local, stdout);
    if removed_worktrees == 0 && removed_branches == 0 {
        let _ = writeln!(stdout, "  \x1b[90m○\x1b[0m stale-worktree sweep removed nothing");
    } else {
        let _ = writeln!(stdout, "  \x1b[36m⬡\x1b[0m stale-worktree sweep removed {removed_worktrees} worktree(s) and {removed_branches} merged branch(es)");
    }
}

fn done_sweep_worktree_list(main_path: &std::path::Path, local: &mut impl DoneRuntime) -> Result<Vec<DoneSweepWorktree>, String> {
    let args = ["-C".to_owned(), main_path.display().to_string(), "worktree".to_owned(), "list".to_owned(), "--porcelain".to_owned()];
    local.done_git(&args).map(|raw| done_sweep_parse_worktrees(&raw))
}

fn done_sweep_parse_worktrees(raw: &str) -> Vec<DoneSweepWorktree> {
    raw.split("\n\n")
        .filter_map(|record| {
            let path = record.lines().find_map(|line| line.strip_prefix("worktree ")).map(std::path::PathBuf::from)?;
            let branch = record
                .lines()
                .find_map(|line| line.strip_prefix("branch refs/heads/"))
                .map(ToOwned::to_owned);
            Some(DoneSweepWorktree { path, branch })
        })
        .collect()
}

fn done_sweep_is_agents_worktree(main_path: &std::path::Path, path: &std::path::Path) -> bool {
    let Ok(relative) = path.strip_prefix(main_path) else { return false; };
    let mut components = relative.components();
    matches!(components.next(), Some(std::path::Component::Normal(name)) if name == "agents") && components.next().is_some()
}

fn done_sweep_has_live_pane(worktree: &std::path::Path, panes: &[DonePane]) -> bool {
    panes.iter().filter_map(|pane| pane.cwd.as_deref()).map(|cwd| cwd.trim_end_matches(" (deleted)")).any(|cwd| {
        let cwd = std::path::Path::new(cwd);
        done_same_path(cwd, worktree) || cwd.starts_with(worktree)
    })
}

fn done_sweep_prune(main_path: &std::path::Path, local: &mut impl DoneRuntime) -> Result<String, String> {
    let args = ["-C".to_owned(), main_path.display().to_string(), "worktree".to_owned(), "prune".to_owned()];
    local.done_git(&args)
}

fn done_sweep_merged_agent_branches(
    main_path: &std::path::Path,
    options: &DoneOptions,
    local: &mut impl DoneRuntime,
    stdout: &mut String,
) -> usize {
    if options.keep_branch {
        return 0;
    }
    let args = ["-C".to_owned(), main_path.display().to_string(), "branch".to_owned(), "--format=%(refname:short)".to_owned(), "--list".to_owned(), "agents/*".to_owned()];
    let Ok(raw) = local.done_git(&args) else {
        let _ = writeln!(stdout, "  \x1b[33m⚠\x1b[0m stale-branch sweep skipped: local branch list unavailable");
        return 0;
    };
    let checked_out = done_sweep_worktree_list(main_path, local)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|worktree| worktree.branch)
        .collect::<std::collections::BTreeSet<_>>();
    let mut removed = 0_usize;
    for branch in raw.lines().map(str::trim) {
        if branch.is_empty() || checked_out.contains(branch) {
            continue;
        }
        if matches!(local.done_pr_state(main_path, branch), Ok(PrGithubState::Merged))
            && done_delete_branch(main_path, branch, local, stdout, "merged PR sweep", true)
        {
            removed += 1;
        }
    }
    removed
}
