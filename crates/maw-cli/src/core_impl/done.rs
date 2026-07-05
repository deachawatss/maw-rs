const DISPATCH_57: &[DispatcherEntry] = &[
    DispatcherEntry { command: "done", handler: Handler::Sync(run_done_command) },
    DispatcherEntry { command: "finish", handler: Handler::Sync(run_done_command) },
];

const DONE_USAGE: &str = "usage: maw done <window-name> [--force] [--dry-run] [--clean-branch] or maw done --all [<oracle>] [--force] [--dry-run] [--clean-branch]  (see: maw sleep/kill for non-worktree shutdown)";
const DONE_ALL_USAGE: &str = "usage: maw done --all [<oracle>] [--force] [--dry-run] [--clean-branch]";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
struct DoneOptions { all: bool, force: bool, dry_run: bool, clean_branch: bool, target: Option<String> }

#[derive(Debug, Clone, PartialEq, Eq)]
struct DoneWindow { session: String, index: i32, name: String }

#[derive(Debug, Clone, PartialEq, Eq)]
struct DoneWorktree { main_path: std::path::PathBuf, full_path: std::path::PathBuf, label: String }

#[derive(Debug, Clone)]
struct DoneContext {
    repos_root: std::path::PathBuf,
    fleet_dirs: Vec<std::path::PathBuf>,
}

impl DoneContext {
    fn from_env() -> Self {
        let env = current_xdg_env();
        Self {
            repos_root: ghq_root().join("github.com"),
            fleet_dirs: fleet_read_dirs_for_env(&env),
        }
    }

    fn with_cwd(cwd: &std::path::Path) -> Self {
        let env = current_xdg_env();
        Self {
            repos_root: done_repos_root_from_cwd(cwd)
                .unwrap_or_else(|| ghq_root().join("github.com")),
            fleet_dirs: fleet_read_dirs_for_env(&env),
        }
    }
}

#[derive(Default)]
struct DoneLocal { runner: maw_tmux::CommandTmuxRunner }

fn run_done_command(argv: &[String]) -> CliOutput {
    match done_run(argv, &mut DoneLocal::default()) {
        Ok(stdout) => CliOutput { code: 0, stdout, stderr: String::new() },
        Err(message) => CliOutput { code: 1, stdout: String::new(), stderr: format!("{message}\n") },
    }
}

fn done_run(argv: &[String], local: &mut DoneLocal) -> Result<String, String> {
    let context = DoneContext::from_env();
    done_run_with_context(argv, local, &context)
}

fn done_run_with_cwd(cwd: &std::path::Path, argv: &[String], local: &mut DoneLocal) -> Result<String, String> {
    let context = DoneContext::with_cwd(cwd);
    done_run_with_context(argv, local, &context)
}

fn done_run_with_context(argv: &[String], local: &mut DoneLocal, context: &DoneContext) -> Result<String, String> {
    let options = done_parse_args(argv)?;
    if options.all { return Ok(done_run_all(&options, local, context)); }
    let target = options.target.clone().ok_or_else(|| DONE_USAGE.to_owned())?;
    done_run_one_with_context(&target, &options, None, local, context)
}

fn done_parse_args(argv: &[String]) -> Result<DoneOptions, String> {
    let mut options = DoneOptions::default();
    let mut positionals = Vec::<String>::new();
    for arg in argv {
        match arg.as_str() {
            "--all" => options.all = true,
            "--force" => options.force = true,
            "--dry-run" => options.dry_run = true,
            "--clean-branch" => options.clean_branch = true,
            "--help" | "-h" => return Err(DONE_USAGE.to_owned()),
            value if value.starts_with('-') => return Err(format!("done: unknown argument {value}")),
            value => positionals.push(value.to_owned()),
        }
    }
    if options.all && positionals.len() > 1 {
        return Err(format!("unexpected extra positional arg(s) for maw done --all: {}\n  {DONE_ALL_USAGE}", positionals[1..].join(" ")));
    }
    if !options.all && positionals.len() > 1 {
        let hint = if positionals.first().is_some_and(|value| value.eq_ignore_ascii_case("all")) { "\n  did you mean `maw done --all`?" } else { "" };
        return Err(format!("unexpected extra positional arg(s) for maw done: {}{hint}\n  {DONE_USAGE}", positionals[1..].join(" ")));
    }
    if let Some(target) = positionals.first() { done_validate_target_arg(target, "target")?; options.target = Some(done_normalize_target(target)); }
    if !options.all && options.target.is_none() { return Err(DONE_USAGE.to_owned()); }
    Ok(options)
}

fn done_run_one(target: &str, options: &DoneOptions, session_filter: Option<&str>, local: &mut DoneLocal) -> Result<String, String> {
    let context = DoneContext::from_env();
    done_run_one_with_context(target, options, session_filter, local, &context)
}

fn done_run_one_with_context(target: &str, options: &DoneOptions, session_filter: Option<&str>, local: &mut DoneLocal, context: &DoneContext) -> Result<String, String> {
    let mut stdout = String::new();
    let sessions = local.done_list_windows();
    let target_lower = target.to_lowercase();
    let matched = done_find_window(&sessions, &target_lower, session_filter);
    if let Some(window) = &matched { done_assert_may_target_lead(window, &sessions, local, &mut stdout)?; }
    if let Some(window) = &matched {
        if !options.force {
            done_auto_save(window, options, local, &mut stdout);
            if options.dry_run { return Ok(stdout); }
        }
    } else if options.dry_run {
        let _ = writeln!(stdout, "  \x1b[36m⬡\x1b[0m [dry-run] window '{target}' not running — nothing to auto-save");
    }
    if let Some(window) = &matched { done_kill_window(window, options, local, &mut stdout); } else { let _ = writeln!(stdout, "  \x1b[90m○\x1b[0m window '{target}' not running"); }
    let removed_worktree = done_remove_worktree_via_config(&target_lower, context, options, &mut stdout)? || done_remove_worktree_by_scan(target, &context.repos_root, options, &mut stdout)?;
    if !removed_worktree { stdout.push_str("  \x1b[90m○\x1b[0m no worktree to remove (may be a main window)\n"); }
    if options.dry_run {
        if matched.is_none() && !removed_worktree { done_fail_missing_target(target)?; }
        let _ = writeln!(stdout, "  \x1b[36m⬡\x1b[0m [dry-run] would remove '{target_lower}' from fleet config if present\n");
        return Ok(stdout);
    }
    let removed_config = done_remove_from_fleet_config(&target_lower, context, &mut stdout);
    if !removed_config { stdout.push_str("  \x1b[90m○\x1b[0m not in any fleet config\n"); }
    if matched.is_none() && !removed_worktree && !removed_config { done_fail_missing_target(target)?; }
    stdout.push('\n');
    Ok(stdout)
}

fn done_run_all(options: &DoneOptions, local: &mut DoneLocal, context: &DoneContext) -> String {
    let mut stdout = String::new();
    let sessions = local.done_list_windows();
    let session_name = done_current_session_name(&sessions, options.target.as_deref(), local);
    let Some(session_name) = session_name else {
        let reason = if let Some(oracle) = &options.target { format!("no tmux session found for oracle '{oracle}'") } else if sessions.is_empty() { "no tmux sessions to clean".to_owned() } else { "could not identify current tmux session; run inside tmux".to_owned() };
        let _ = writeln!(stdout, "  \x1b[90m○\x1b[0m {reason}");
        return stdout;
    };
    let targets = done_non_lead_windows(&sessions, &session_name);
    if targets.is_empty() { let _ = writeln!(stdout, "  \x1b[90m○\x1b[0m no non-lead windows in {session_name}"); return stdout; }
    let mode = if options.dry_run { "would process" } else { "processing" };
    let _ = writeln!(stdout, "  \x1b[36m⬡\x1b[0m {mode} {} non-lead window(s) in {session_name}", targets.len());
    let mut processed = 0_usize;
    let mut skipped = 0_usize;
    for window in targets {
        let _ = writeln!(stdout, "\n\x1b[36m→\x1b[0m done {session_name}:{}", window.name);
        match done_run_one_with_context(&window.name, options, Some(&session_name), local, context) { Ok(text) => { stdout.push_str(&text); processed += 1; }, Err(error) => { skipped += 1; let _ = writeln!(stdout, "  \x1b[33m⚠\x1b[0m skipped {}: {error}", window.name); } }
    }
    let verb = if options.dry_run { "would process" } else { "processed" };
    let suffix = if skipped == 0 { String::new() } else { format!(", skipped {skipped}") };
    let _ = writeln!(stdout, "  \x1b[32m✓\x1b[0m done --all {verb} {processed} window(s){suffix}");
    stdout
}

impl DoneLocal {
    fn done_list_windows(&mut self) -> Vec<DoneWindow> {
        let args = ["-a".to_owned(), "-F".to_owned(), "#{session_name}|||#{window_index}|||#{window_name}|||#{window_active}|||#{pane_current_path}".to_owned()];
        let Ok(raw) = maw_tmux::TmuxRunner::run(&mut self.runner, "list-windows", &args) else { return Vec::new(); };
        raw.lines().filter_map(done_parse_window_line).collect()
    }

    fn done_current_identity(&mut self) -> Option<(String, i32)> {
        let args = ["-p".to_owned(), "#{session_name}\t#{window_index}".to_owned()];
        let raw = maw_tmux::TmuxRunner::run(&mut self.runner, "display-message", &args).ok()?;
        let (session, index) = raw.trim().split_once('\t')?;
        Some((session.to_owned(), index.parse::<i32>().ok()?))
    }

    fn done_pane_info(&mut self, target: &str) -> Option<(String, String)> {
        done_validate_tmux_target(target).ok()?;
        let args = ["-t".to_owned(), target.to_owned(), "-p".to_owned(), "#{pane_current_command}\t#{pane_current_path}".to_owned()];
        let raw = maw_tmux::TmuxRunner::run(&mut self.runner, "display-message", &args).ok()?;
        let (command, cwd) = raw.trim_end().split_once('\t').unwrap_or((raw.trim(), ""));
        Some((command.trim().to_owned(), cwd.trim().to_owned()))
    }

    fn done_tmux(&mut self, command: &str, args: &[String]) -> Result<String, String> {
        maw_tmux::TmuxRunner::run(&mut self.runner, command, args).map_err(|error| error.message)
    }

    fn done_send_text(target: &str, text: &str) -> Result<(), String> {
        done_validate_tmux_target(target)?;
        TmuxClient::local().send_text(target, text).map(|_| ()).map_err(|error| error.message)
    }
}

fn done_parse_window_line(line: &str) -> Option<DoneWindow> {
    let mut parts = line.split("|||");
    let session = parts.next()?.to_owned();
    let index = parts.next()?.parse::<i32>().ok()?;
    let name = parts.next()?.to_owned();
    if session.is_empty() || name.is_empty() { return None; }
    Some(DoneWindow { session, index, name })
}

fn done_find_window(windows: &[DoneWindow], target_lower: &str, session_filter: Option<&str>) -> Option<DoneWindow> {
    windows.iter().find(|window| session_filter.is_none_or(|session| session == window.session) && window.name.eq_ignore_ascii_case(target_lower)).cloned()
}

fn done_assert_may_target_lead(window: &DoneWindow, windows: &[DoneWindow], local: &mut DoneLocal, stdout: &mut String) -> Result<(), String> {
    if let Some((current_session, current_index)) = local.done_current_identity() {
        if current_session == window.session && current_index == window.index {
            let message = format!("refusing to done current window '{}' in session '{}'", window.name, window.session);
            let _ = writeln!(stdout, "  \x1b[31m✗\x1b[0m {message}");
            stdout.push_str("  \x1b[90m  run maw done from the lead/parent pane after the DONE ping\x1b[0m\n");
            return Err(message);
        }
    }
    let Some(lead) = done_lead_window(windows, &window.session) else { return Ok(()); };
    if lead.index != window.index { return Ok(()); }
    let message = format!("refusing to done lead window '{}' in session '{}' from a non-lead context", window.name, window.session);
    let _ = writeln!(stdout, "  \x1b[31m✗\x1b[0m {message}");
    stdout.push_str("  \x1b[90m  run from the lead window, or target a non-lead agent window\x1b[0m\n");
    Err(message)
}

fn done_lead_window(windows: &[DoneWindow], session: &str) -> Option<DoneWindow> {
    windows.iter().filter(|window| window.session == session).min_by_key(|window| window.index).cloned()
}

fn done_non_lead_windows(windows: &[DoneWindow], session: &str) -> Vec<DoneWindow> {
    let Some(lead) = done_lead_window(windows, session) else { return Vec::new(); };
    let mut out = windows.iter().filter(|window| window.session == session && window.index != lead.index).cloned().collect::<Vec<_>>();
    out.sort_by_key(|window| window.index);
    out
}

fn done_current_session_name(windows: &[DoneWindow], oracle: Option<&str>, local: &mut DoneLocal) -> Option<String> {
    let sessions = done_session_names(windows);
    if let Some(oracle) = oracle {
        let wanted = done_session_stem(oracle);
        if let Some(name) = sessions.iter().find(|name| done_session_stem(name) == wanted) { return Some(name.clone()); }
        let matches = sessions.iter().filter(|name| done_compact_stem(name) == done_compact_stem(oracle)).cloned().collect::<Vec<_>>();
        if matches.len() == 1 { return matches.first().cloned(); }
        return None;
    }
    if let Some((session, _)) = local.done_current_identity() { if sessions.contains(&session) { return Some(session); } }
    if sessions.len() == 1 { sessions.first().cloned() } else { None }
}

fn done_session_names(windows: &[DoneWindow]) -> Vec<String> {
    let mut names = windows.iter().map(|window| window.session.clone()).collect::<Vec<_>>();
    names.sort();
    names.dedup();
    names
}

fn done_session_stem(value: &str) -> String { value.trim().to_lowercase().trim_start_matches(|c: char| c.is_ascii_digit() || c == '-').trim_end_matches("-oracle").to_owned() }

fn done_compact_stem(value: &str) -> String { done_session_stem(value).chars().filter(char::is_ascii_alphanumeric).collect() }

fn done_repos_root_from_cwd(cwd: &std::path::Path) -> Option<std::path::PathBuf> {
    cwd.ancestors()
        .find(|path| path.file_name().and_then(std::ffi::OsStr::to_str) == Some("github.com"))
        .map(std::path::Path::to_path_buf)
}

fn done_auto_save(window: &DoneWindow, options: &DoneOptions, local: &mut DoneLocal, stdout: &mut String) {
    let target = format!("{}:{}", window.session, window.name);
    let (command, cwd) = local.done_pane_info(&target).unwrap_or_default();
    let retro = done_retrospective_command(&command);
    if options.dry_run {
        if let Some(retro) = retro { let _ = writeln!(stdout, "  \x1b[36m⬡\x1b[0m [dry-run] would send {retro} to {target} and wait 10s"); } else { stdout.push_str("  \x1b[36m⬡\x1b[0m [dry-run] would skip retro (no retrospective command for this engine)\n"); }
        if !cwd.is_empty() { let _ = writeln!(stdout, "  \x1b[36m⬡\x1b[0m [dry-run] would git add + commit + push in {cwd}"); }
        let _ = writeln!(stdout, "  \x1b[36m⬡\x1b[0m [dry-run] would kill window {target}");
        stdout.push_str("  \x1b[36m⬡\x1b[0m [dry-run] would remove worktree + fleet config\n\n");
        return;
    }
    if let Some(retro) = retro {
        match DoneLocal::done_send_text(&target, retro) {
            Ok(()) => done_wait_for_rrr_prompt(&target, local, stdout),
            Err(error) => {
                let _ = writeln!(stdout, "  \x1b[33m⚠\x1b[0m could not send {retro} to {target}: {error}");
            }
        }
    }
    if !cwd.is_empty() {
        let _ = done_git(&["-C".to_owned(), cwd.clone(), "add".to_owned(), "--".to_owned(), ".".to_owned()]);
        let _ = done_git(&["-C".to_owned(), cwd.clone(), "commit".to_owned(), "-m".to_owned(), "chore: auto-save before done".to_owned()]);
        if done_should_push_on_done(std::path::Path::new(&cwd)) {
            let _ = done_git(&["-C".to_owned(), cwd, "push".to_owned()]);
        } else {
            let _ = writeln!(stdout, "  \x1b[90m○\x1b[0m skipped auto-save push (no live remote branch or PR already closed)");
        }
    }
}

fn done_should_push_on_done(cwd: &std::path::Path) -> bool {
    let branch = done_git(&["-C".to_owned(), cwd.display().to_string(), "rev-parse".to_owned(), "--abbrev-ref".to_owned(), "HEAD".to_owned()]).unwrap_or_default().trim().to_owned();
    done_branch_is_pushable(cwd, &branch)
}

fn done_branch_is_pushable(cwd: &std::path::Path, branch: &str) -> bool {
    if !done_branch_name_allows_push(branch) { return false; }
    if done_pr_is_closed_or_merged(cwd, branch) { return false; }
    done_remote_branch_exists(cwd, branch)
}

fn done_branch_name_allows_push(branch: &str) -> bool {
    !branch.is_empty() && branch != "main" && branch != "HEAD"
}

fn done_remote_branch_exists(cwd: &std::path::Path, branch: &str) -> bool {
    std::process::Command::new("git")
        .args(["-C", &cwd.display().to_string(), "ls-remote", "--exit-code", "--heads", "origin", branch])
        .output()
        .is_ok_and(|output| output.status.success())
}

fn done_pr_is_closed_or_merged(cwd: &std::path::Path, branch: &str) -> bool {
    let output = std::process::Command::new("gh")
        .args(["pr", "view", branch, "--repo", ".", "--json", "state", "--jq", ".state"])
        .current_dir(cwd)
        .output();
    let Ok(output) = output else { return false; };
    if !output.status.success() { return false; }
    done_pr_state_is_closed_or_merged(String::from_utf8_lossy(&output.stdout).trim())
}

fn done_pr_state_is_closed_or_merged(state: &str) -> bool {
    matches!(state.trim(), "MERGED" | "CLOSED")
}

fn done_kill_window(window: &DoneWindow, options: &DoneOptions, local: &mut DoneLocal, stdout: &mut String) {
    let target = format!("{}:{}", window.session, window.name);
    if options.dry_run { let _ = writeln!(stdout, "  \x1b[36m⬡\x1b[0m [dry-run] would kill window {target}"); return; }
    match local.done_tmux("kill-window", &["-t".to_owned(), target.clone()]) { Ok(_) => { let _ = writeln!(stdout, "  \x1b[32m✓\x1b[0m killed window {target}"); }, Err(_) => stdout.push_str("  \x1b[33m⚠\x1b[0m could not kill window (may already be closed)\n") }
}

fn done_retrospective_command(command: &str) -> Option<&'static str> {
    let lower = command.to_lowercase();
    if lower.contains("omx") || lower.contains("oh-my-codex") { Some("$rrr") } else if lower.is_empty() || lower.contains("codex") || lower.contains("aider") || lower.contains("opencode") { None } else { Some("/rrr") }
}

const DONE_RRR_WAIT_MAX_POLLS: usize = 16;
const DONE_RRR_WAIT_INTERVAL_SECS: u64 = 2;

fn done_wait_for_rrr_prompt(target: &str, local: &mut DoneLocal, stdout: &mut String) {
    let result = done_wait_for_prompt_with(
        || local.done_tmux("capture-pane", &["-t".to_owned(), target.to_owned(), "-p".to_owned(), "-S".to_owned(), "-40".to_owned()]),
        std::thread::sleep,
        DONE_RRR_WAIT_MAX_POLLS,
        std::time::Duration::from_secs(DONE_RRR_WAIT_INTERVAL_SECS),
    );
    match result {
        Ok(polls) => {
            let _ = writeln!(stdout, "  \x1b[32m✓\x1b[0m retrospective prompt returned after {polls} poll(s)");
        }
        Err(error) => {
            let _ = writeln!(stdout, "  \x1b[33m⚠\x1b[0m retrospective completion unconfirmed: {error}");
        }
    }
}

fn done_wait_for_prompt_with<C, S>(mut capture: C, mut sleep: S, max_polls: usize, interval: std::time::Duration) -> Result<usize, String>
where
    C: FnMut() -> Result<String, String>,
    S: FnMut(std::time::Duration),
{
    if max_polls == 0 { return Err("no prompt polls configured".to_owned()); }
    for poll in 1..=max_polls {
        let content = capture()?;
        if done_capture_has_prompt(&content) { return Ok(poll); }
        if poll < max_polls { sleep(interval); }
    }
    let polls = u64::try_from(max_polls).unwrap_or(u64::MAX);
    Err(format!("prompt did not return within {}s", polls.saturating_mul(interval.as_secs())))
}

fn done_capture_has_prompt(content: &str) -> bool {
    content.lines().rev().find(|line| !line.trim().is_empty()).is_some_and(done_line_looks_like_prompt)
}

fn done_line_looks_like_prompt(line: &str) -> bool {
    let clean = maw_tmux::strip_tmux_ansi(line).replace('\r', "");
    let trimmed = clean.trim_end();
    if trimmed.is_empty() || trimmed.len() > 120 { return false; }
    matches!(trimmed, "$" | "#" | "%" | ">" | "❯" | "»") || trimmed.ends_with(['$', '#', '%', '>', '❯', '»'])
}

fn done_remove_worktree_via_config(window_lower: &str, context: &DoneContext, options: &DoneOptions, stdout: &mut String) -> Result<bool, String> {
    for file in done_fleet_config_files(context) {
        let Ok(raw) = std::fs::read_to_string(&file) else { continue; };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&raw) else { continue; };
        let Some(windows) = json.get("windows").and_then(serde_json::Value::as_array) else { continue; };
        let Some(repo) = windows.iter().find(|item| item.get("name").and_then(serde_json::Value::as_str).is_some_and(|name| name.eq_ignore_ascii_case(window_lower))).and_then(|item| item.get("repo")).and_then(serde_json::Value::as_str) else { continue; };
        let Some(worktree) = done_parse_worktree_path(&context.repos_root.join(repo), &context.repos_root) else { break; };
        if options.dry_run { let _ = writeln!(stdout, "  \x1b[36m⬡\x1b[0m [dry-run] would remove worktree {repo}"); return Ok(true); }
        done_remove_worktree(&worktree, options, stdout)?;
        return Ok(true);
    }
    Ok(false)
}

fn done_remove_worktree_by_scan(target: &str, repos_root: &std::path::Path, options: &DoneOptions, stdout: &mut String) -> Result<bool, String> {
    let matches = done_find_worktree_paths(target, repos_root);
    if matches.len() > 1 { let _ = writeln!(stdout, "  \x1b[31m✗\x1b[0m refusing to remove worktree '{}' — matches {} repos", target, matches.len()); return Ok(false); }
    let Some(worktree) = matches.first() else { return Ok(false); };
    if options.dry_run { let _ = writeln!(stdout, "  \x1b[36m⬡\x1b[0m [dry-run] would remove worktree {}", worktree.label); return Ok(true); }
    done_remove_worktree(worktree, options, stdout)?;
    Ok(true)
}

fn done_remove_worktree(worktree: &DoneWorktree, options: &DoneOptions, stdout: &mut String) -> Result<(), String> {
    done_validate_exec_path(&worktree.main_path)?;
    done_validate_exec_path(&worktree.full_path)?;
    let branch = done_git(&["-C".to_owned(), worktree.full_path.display().to_string(), "rev-parse".to_owned(), "--abbrev-ref".to_owned(), "HEAD".to_owned()]).unwrap_or_default().trim().to_owned();
    let rescued = done_rescue_psi(&worktree.full_path, &worktree.main_path)?;
    if !rescued.is_empty() {
        let _ = writeln!(stdout, "  \x1b[32m✓\x1b[0m rescued {} ψ/ file(s)", rescued.len());
    }
    done_git(&["-C".to_owned(), worktree.main_path.display().to_string(), "worktree".to_owned(), "remove".to_owned(), "--force".to_owned(), "--".to_owned(), worktree.full_path.display().to_string()])?;
    done_git(&["-C".to_owned(), worktree.main_path.display().to_string(), "worktree".to_owned(), "prune".to_owned()])?;
    let _ = writeln!(stdout, "  \x1b[32m✓\x1b[0m removed worktree {}", worktree.label);
    done_cleanup_branch(&worktree.main_path, &branch, options, stdout);
    Ok(())
}

fn done_cleanup_branch(main_path: &std::path::Path, branch: &str, options: &DoneOptions, stdout: &mut String) {
    if branch.is_empty() || branch == "main" || branch == "HEAD" { return; }
    if options.clean_branch { let _ = done_git(&["-C".to_owned(), main_path.display().to_string(), "branch".to_owned(), "-D".to_owned(), "--".to_owned(), branch.to_owned()]); let _ = writeln!(stdout, "  \x1b[32m✓\x1b[0m deleted branch {branch}"); } else { let _ = writeln!(stdout, "  \x1b[90m○\x1b[0m branch {branch} retained (use --clean-branch to delete)"); }
}

fn done_find_worktree_paths(target: &str, repos_root: &std::path::Path) -> Vec<DoneWorktree> {
    let mut out = Vec::new();
    let target_lower = target.to_lowercase();
    let Ok(orgs) = std::fs::read_dir(repos_root) else { return out; };
    for org in orgs.flatten().filter(|entry| entry.path().is_dir()) {
        let Ok(repos) = std::fs::read_dir(org.path()) else { continue; };
        for repo in repos.flatten().filter(|entry| entry.path().is_dir()) { done_scan_repo_worktrees(&repo.path(), repos_root, &target_lower, &mut out); }
    }
    out.sort_by(|a, b| a.full_path.cmp(&b.full_path));
    out
}

fn done_scan_repo_worktrees(repo_path: &std::path::Path, repos_root: &std::path::Path, target_lower: &str, out: &mut Vec<DoneWorktree>) {
    let Some(name) = repo_path.file_name().and_then(std::ffi::OsStr::to_str) else { return; };
    if name.to_lowercase().ends_with(&format!(".wt-{target_lower}")) { if let Some(worktree) = done_parse_worktree_path(repo_path, repos_root) { out.push(worktree); } }
    let agents = repo_path.join("agents");
    let Ok(entries) = std::fs::read_dir(agents) else { return; };
    for entry in entries.flatten().filter(|entry| entry.path().is_dir()) {
        if entry.file_name().to_string_lossy().eq_ignore_ascii_case(target_lower) { if let Some(worktree) = done_parse_worktree_path(&entry.path(), repos_root) { out.push(worktree); } }
    }
}

fn done_parse_worktree_path(full_path: &std::path::Path, repos_root: &std::path::Path) -> Option<DoneWorktree> {
    let rel = full_path.strip_prefix(repos_root).ok()?;
    let parts = rel.components().map(|part| part.as_os_str().to_string_lossy().to_string()).collect::<Vec<_>>();
    if parts.len() >= 4 && parts.get(2).is_some_and(|part| part == "agents") {
        let main_path = repos_root.join(&parts[0]).join(&parts[1]);
        let label = parts.join("/");
        return Some(DoneWorktree { main_path, full_path: full_path.to_path_buf(), label });
    }
    if parts.len() == 2 && parts[1].contains(".wt-") {
        let repo = parts[1].split_once(".wt-")?.0;
        let main_path = repos_root.join(&parts[0]).join(repo);
        return Some(DoneWorktree { main_path, full_path: full_path.to_path_buf(), label: parts[1].clone() });
    }
    None
}

/// Rescue uncommitted `ψ/` files from a worktree into the owning main checkout.
///
/// Existing destination files are never overwritten; collisions receive a timestamp suffix.
///
/// # Errors
///
/// Returns an error when git status fails or when a rescue copy cannot be completed.
#[doc(hidden)]
pub fn done_rescue_psi(worktree_path: &std::path::Path, main_path: &std::path::Path) -> Result<Vec<std::path::PathBuf>, String> {
    let status = done_git(&["-C".to_owned(), worktree_path.display().to_string(), "-c".to_owned(), "core.quotePath=false".to_owned(), "status".to_owned(), "--porcelain".to_owned(), "--".to_owned(), "ψ/".to_owned()])?;
    let sources = done_uncommitted_psi_sources(worktree_path, &status)?;
    if sources.is_empty() { return Ok(Vec::new()); }
    let main_root = done_rescue_main_path(worktree_path, main_path);
    let main_psi = main_root.join("ψ");
    let timestamp = done_unix_timestamp();
    let mut rescued = Vec::new();
    for source in sources {
        let destination = done_rescue_destination(worktree_path, &main_psi, &source, timestamp)?;
        done_copy_without_overwrite(&source, &destination)?;
        rescued.push(destination);
    }
    Ok(rescued)
}

fn done_rescue_main_path(worktree_path: &std::path::Path, fallback: &std::path::Path) -> std::path::PathBuf {
    let common_dir = done_git(&["-C".to_owned(), worktree_path.display().to_string(), "rev-parse".to_owned(), "--git-common-dir".to_owned()]).unwrap_or_default();
    let common_dir = common_dir.trim();
    if common_dir.is_empty() { return fallback.to_path_buf(); }
    let path = std::path::PathBuf::from(common_dir);
    let absolute = if path.is_absolute() { path } else { worktree_path.join(path) };
    absolute.parent().filter(|parent| !parent.as_os_str().is_empty()).map_or_else(|| fallback.to_path_buf(), std::path::Path::to_path_buf)
}

fn done_uncommitted_psi_sources(worktree_path: &std::path::Path, status: &str) -> Result<Vec<std::path::PathBuf>, String> {
    let mut sources = Vec::new();
    for relative in status.lines().filter_map(done_status_psi_path) {
        done_collect_psi_source(&worktree_path.join(relative), &mut sources)?;
    }
    sources.sort();
    sources.dedup();
    Ok(sources)
}

fn done_status_psi_path(line: &str) -> Option<std::path::PathBuf> {
    let path = line.get(3..)?.trim();
    let path = path.rsplit_once(" -> ").map_or(path, |(_, destination)| destination.trim());
    let path = path.trim_matches('"');
    if path == "ψ" || path.starts_with("ψ/") { Some(std::path::PathBuf::from(path)) } else { None }
}

fn done_collect_psi_source(path: &std::path::Path, out: &mut Vec<std::path::PathBuf>) -> Result<(), String> {
    if path.is_file() {
        out.push(path.to_path_buf());
        return Ok(());
    }
    if !path.is_dir() { return Ok(()); }
    let entries = std::fs::read_dir(path).map_err(|error| format!("read ψ rescue dir '{}': {error}", path.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("read ψ rescue entry '{}': {error}", path.display()))?;
        done_collect_psi_source(&entry.path(), out)?;
    }
    Ok(())
}

fn done_rescue_destination(worktree_path: &std::path::Path, main_psi: &std::path::Path, source: &std::path::Path, timestamp: u64) -> Result<std::path::PathBuf, String> {
    let psi_root = worktree_path.join("ψ");
    let relative = source.strip_prefix(&psi_root).map_err(|_| format!("ψ rescue source escaped ψ/: {}", source.display()))?;
    Ok(done_available_destination(&main_psi.join(relative), timestamp))
}

fn done_available_destination(path: &std::path::Path, timestamp: u64) -> std::path::PathBuf {
    if !path.exists() { return path.to_path_buf(); }
    for attempt in 0_u32..1000 {
        let candidate = done_collision_destination(path, timestamp, attempt);
        if !candidate.exists() { return candidate; }
    }
    done_collision_destination(path, timestamp, std::process::id())
}

fn done_collision_destination(path: &std::path::Path, timestamp: u64, attempt: u32) -> std::path::PathBuf {
    let suffix = if attempt == 0 { format!("-{timestamp}") } else { format!("-{timestamp}-{attempt}") };
    let file_stem = path.file_stem().and_then(std::ffi::OsStr::to_str).unwrap_or("psi");
    let file_name = if let Some(extension) = path.extension().and_then(std::ffi::OsStr::to_str) { format!("{file_stem}{suffix}.{extension}") } else { format!("{file_stem}{suffix}") };
    path.with_file_name(file_name)
}

fn done_copy_without_overwrite(source: &std::path::Path, destination: &std::path::Path) -> Result<(), String> {
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent).map_err(|error| format!("create ψ rescue dir '{}': {error}", parent.display()))?;
    }
    let mut input = std::fs::File::open(source).map_err(|error| format!("open ψ rescue source '{}': {error}", source.display()))?;
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|error| format!("create ψ rescue destination '{}': {error}", destination.display()))?;
    std::io::copy(&mut input, &mut output).map_err(|error| format!("copy ψ rescue '{}' -> '{}': {error}", source.display(), destination.display()))?;
    Ok(())
}

fn done_unix_timestamp() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |duration| duration.as_secs())
}

fn done_remove_from_fleet_config(window_lower: &str, context: &DoneContext, stdout: &mut String) -> bool {
    let mut removed = false;
    for file in done_fleet_config_files(context) {
        let Ok(raw) = std::fs::read_to_string(&file) else { continue; };
        let Ok(mut json) = serde_json::from_str::<serde_json::Value>(&raw) else { continue; };
        let before = json.get("windows").and_then(serde_json::Value::as_array).map_or(0, Vec::len);
        if let Some(windows) = json.get_mut("windows").and_then(serde_json::Value::as_array_mut) { windows.retain(|item| !item.get("name").and_then(serde_json::Value::as_str).is_some_and(|name| name.eq_ignore_ascii_case(window_lower))); }
        if json.get("windows").and_then(serde_json::Value::as_array).map_or(0, Vec::len) < before {
            if let Ok(text) = serde_json::to_string_pretty(&json) { let _ = std::fs::write(&file, format!("{text}\n")); }
            let file_name = file.file_name().and_then(std::ffi::OsStr::to_str).unwrap_or("fleet.json");
            let _ = writeln!(stdout, "  \x1b[32m✓\x1b[0m removed from {file_name}");
            removed = true;
        }
    }
    removed
}

fn done_fleet_config_files(context: &DoneContext) -> Vec<std::path::PathBuf> {
    fleet_load_entries_impl(context.fleet_dirs.clone(), false, "fleet")
        .unwrap_or_default()
        .into_iter()
        .map(|entry| entry.path)
        .collect()
}

fn done_git(args: &[String]) -> Result<String, String> {
    let output = std::process::Command::new("git").args(args).output().map_err(|error| format!("git failed: {error}"))?;
    if output.status.success() { Ok(String::from_utf8_lossy(&output.stdout).to_string()) } else { Err(String::from_utf8_lossy(&output.stderr).trim().to_owned()) }
}

fn done_fail_missing_target(window_name: &str) -> Result<(), String> {
    let hint = if window_name.eq_ignore_ascii_case("all") { "\n  did you mean `maw done --all`?" } else { "" };
    Err(format!("no done target matched '{window_name}'{hint}"))
}

fn done_normalize_target(value: &str) -> String { value.trim().to_owned() }

fn done_validate_target_arg(value: &str, label: &str) -> Result<(), String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.starts_with('-') || trimmed != value { return Err(format!("done: invalid {label} '{value}'")); }
    Ok(())
}

fn done_validate_tmux_target(value: &str) -> Result<(), String> { if value.trim().is_empty() || value.starts_with('-') { Err(format!("done: invalid tmux target '{value}'")) } else { Ok(()) } }

fn done_validate_exec_path(path: &std::path::Path) -> Result<(), String> {
    if path.as_os_str().is_empty() || path.components().any(|part| part.as_os_str().to_string_lossy().starts_with('-')) { return Err(format!("done: refusing leading-dash path '{}'", path.display())); }
    Ok(())
}

#[cfg(test)]
mod done_tests {
    use super::*;

    #[test]
    fn done_parse_rejects_leading_dash_positionals() {
        assert_eq!(done_parse_args(&["-Sbad".to_owned()]).unwrap_err(), "done: unknown argument -Sbad");
    }

    #[test]
    fn done_parse_matches_js_extra_positionals() {
        let err = done_parse_args(&["all".to_owned(), "x".to_owned()]).unwrap_err();
        assert!(err.contains("did you mean `maw done --all`?"), "{err}");
    }

    #[test]
    fn done_worktree_path_parses_agents_and_dot_wt() {
        let root = std::path::Path::new("/tmp/ghq/github.com");
        let agents = done_parse_worktree_path(std::path::Path::new("/tmp/ghq/github.com/org/repo/agents/task"), root).unwrap();
        assert_eq!(agents.main_path, std::path::PathBuf::from("/tmp/ghq/github.com/org/repo"));
        let dot = done_parse_worktree_path(std::path::Path::new("/tmp/ghq/github.com/org/repo.wt-task"), root).unwrap();
        assert_eq!(dot.main_path, std::path::PathBuf::from("/tmp/ghq/github.com/org/repo"));
    }

    #[test]
    fn done_push_guard_blocks_main_head_and_closed_pr_states() {
        assert!(!done_branch_name_allows_push(""));
        assert!(!done_branch_name_allows_push("main"));
        assert!(!done_branch_name_allows_push("HEAD"));
        assert!(done_branch_name_allows_push("wind/fork-patch-migration"));
        assert!(done_pr_state_is_closed_or_merged("MERGED"));
        assert!(done_pr_state_is_closed_or_merged("CLOSED\n"));
        assert!(!done_pr_state_is_closed_or_merged("OPEN"));
        assert!(!done_pr_state_is_closed_or_merged(""));
    }
}
