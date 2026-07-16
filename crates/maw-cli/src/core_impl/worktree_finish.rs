const DISPATCH_57: &[DispatcherEntry] = &[
    DispatcherEntry { command: "done", handler: Handler::Sync(run_done_command) },
    DispatcherEntry { command: "finish", handler: Handler::Sync(run_done_command) },
];

const DONE_USAGE: &str = "usage: maw done <window-name> [--force] [--dry-run] [--clean-branch] [--worktree <path>] or maw done --all [<oracle>] [--force] [--dry-run] [--clean-branch]  (see: maw sleep/kill for non-worktree shutdown)";
const DONE_ALL_USAGE: &str = "usage: maw done --all [<oracle>] [--force] [--dry-run] [--clean-branch]";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
struct DoneOptions { all: bool, force: bool, dry_run: bool, clean_branch: bool, target: Option<String>, worktree: Option<std::path::PathBuf> }

#[derive(Debug, Clone, PartialEq, Eq)]
struct DoneWindow { session: String, index: i32, name: String, cwd: Option<String> }

#[derive(Debug, Clone, PartialEq, Eq)]
struct DoneWorktree { main_path: std::path::PathBuf, full_path: std::path::PathBuf, label: String }

#[derive(Debug, Clone, PartialEq, Eq)]
struct DonePaneInfo { command: String, cwd: String }

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

trait DoneRuntime {
    fn done_list_windows(&mut self) -> Vec<DoneWindow>;
    fn done_current_identity(&mut self) -> Option<(String, i32)>;
    fn done_pane_info(&mut self, target: &str) -> Option<(String, String)>;
    fn done_reap_target(&mut self, target: &str) -> Result<(), String>;
    fn done_tmux(&mut self, command: &str, args: &[String]) -> Result<String, String>;
    fn done_send_text(&mut self, target: &str, text: &str) -> Result<(), String>;
    fn done_git(&mut self, args: &[String]) -> Result<String, String>;
}

fn run_done_command(argv: &[String]) -> CliOutput {
    match done_run(argv, &mut DoneLocal::default()) {
        Ok(stdout) => CliOutput { code: 0, stdout, stderr: String::new() },
        Err(message) => CliOutput { code: 1, stdout: String::new(), stderr: format!("{message}\n") },
    }
}

fn done_run(argv: &[String], local: &mut impl DoneRuntime) -> Result<String, String> {
    let context = DoneContext::from_env();
    done_run_with_context(argv, local, &context)
}

fn done_run_with_cwd(cwd: &std::path::Path, argv: &[String], local: &mut impl DoneRuntime) -> Result<String, String> {
    let context = DoneContext::with_cwd(cwd);
    done_run_with_context(argv, local, &context)
}

fn done_run_with_context(argv: &[String], local: &mut impl DoneRuntime, context: &DoneContext) -> Result<String, String> {
    let options = done_parse_args(argv)?;
    if options.all && options.worktree.is_some() { return Err("done: --worktree cannot be used with --all".to_owned()); }
    if options.all { return Ok(done_run_all(&options, local, context)); }
    let target = options.target.clone().ok_or_else(|| DONE_USAGE.to_owned())?;
    done_run_one_with_context(&target, &options, None, local, context)
}

fn done_parse_args(argv: &[String]) -> Result<DoneOptions, String> {
    let mut options = DoneOptions::default();
    let mut positionals = Vec::<String>::new();
    let mut index = 0_usize;
    while index < argv.len() {
        let arg = &argv[index];
        match arg.as_str() {
            "--all" => options.all = true,
            "--force" => options.force = true,
            "--dry-run" => options.dry_run = true,
            "--clean-branch" => options.clean_branch = true,
            "--worktree" => {
                let value = argv.get(index + 1).ok_or_else(|| "done: missing --worktree value".to_owned())?;
                done_set_worktree_option(&mut options, value)?;
                index += 1;
            }
            "--help" | "-h" => return Err(DONE_USAGE.to_owned()),
            value if value.starts_with("--worktree=") => {
                let value = value.strip_prefix("--worktree=").unwrap_or_default();
                done_set_worktree_option(&mut options, value)?;
            }
            value if value.starts_with('-') => return Err(format!("done: unknown argument {value}")),
            value => positionals.push(value.to_owned()),
        }
        index += 1;
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

fn done_set_worktree_option(options: &mut DoneOptions, value: &str) -> Result<(), String> {
    if options.worktree.is_some() { return Err("done: --worktree specified more than once".to_owned()); }
    done_validate_worktree_arg(value)?;
    options.worktree = Some(std::path::PathBuf::from(value));
    Ok(())
}

fn done_run_one(target: &str, options: &DoneOptions, session_filter: Option<&str>, local: &mut impl DoneRuntime) -> Result<String, String> {
    let context = DoneContext::from_env();
    done_run_one_with_context(target, options, session_filter, local, &context)
}

fn done_run_one_with_context(target: &str, options: &DoneOptions, session_filter: Option<&str>, local: &mut impl DoneRuntime, context: &DoneContext) -> Result<String, String> {
    let mut stdout = String::new();
    let sessions = local.done_list_windows();
    let target_lower = target.to_lowercase();
    let matched = done_find_window(&sessions, &target_lower, session_filter);
    if let Some(window) = &matched { done_assert_may_target_lead(window, &sessions, local, &mut stdout)?; }
    let pane_info = matched.as_ref().and_then(|window| done_live_pane_info(window, local));
    let solo_worktree = matched.as_ref().and_then(|window| solo_worktree_for_holder(&done_tmux_target(window)));
    let selected_worktree = if let Some(path) = solo_worktree {
        done_resolve_registered_worktree(local, &path, context)?
    } else {
        done_select_worktree(target, &target_lower, options, pane_info.as_ref(), local, context, &mut stdout)?
    };
    if !options.dry_run {
        if let Some(worktree) = &selected_worktree {
            done_rescue_psi_notes(worktree, &mut stdout);
        }
    }
    if let Some(window) = &matched {
        if !options.force {
            done_auto_save(window, options, local, pane_info.as_ref(), selected_worktree.as_ref(), &mut stdout);
        }
    } else if options.dry_run {
        let _ = writeln!(stdout, "  \x1b[36m⬡\x1b[0m [dry-run] window '{target}' not running — nothing to auto-save");
    }
    if let Some(window) = &matched {
        done_kill_window(window, options, local, &mut stdout);
    } else { let _ = writeln!(stdout, "  \x1b[90m○\x1b[0m window '{target}' not running"); }
    let removed_worktree = if let Some(worktree) = &selected_worktree {
        done_remove_selected_worktree(worktree, options, local, &mut stdout)?;
        true
    } else {
        false
    };
    if !options.dry_run {
        if let Some(window) = &matched { solo_release_holder(&done_tmux_target(window)); }
    }
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

fn done_run_all(options: &DoneOptions, local: &mut impl DoneRuntime, context: &DoneContext) -> String {
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

impl DoneRuntime for DoneLocal {
    fn done_list_windows(&mut self) -> Vec<DoneWindow> {
        let args = ["-a".to_owned(), "-F".to_owned(), "#{session_name}|||#{window_index}|||#{window_name}|||#{window_active}|||#{pane_current_path}".to_owned()];
        let Ok(raw) = maw_tmux::TmuxRunner::run(&mut self.runner, "list-windows", &args) else { return Vec::new(); };
        raw.lines().filter_map(done_parse_window_line).collect()
    }

    fn done_current_identity(&mut self) -> Option<(String, i32)> {
        done_invoking_pane_identity(&mut self.runner)
    }

    fn done_pane_info(&mut self, target: &str) -> Option<(String, String)> {
        done_validate_tmux_target(target).ok()?;
        let args = ["-t".to_owned(), target.to_owned(), "-p".to_owned(), "#{pane_current_command}\t#{pane_current_path}".to_owned()];
        let raw = maw_tmux::TmuxRunner::run(&mut self.runner, "display-message", &args).ok()?;
        let (command, cwd) = raw.trim_end().split_once('\t').unwrap_or((raw.trim(), ""));
        Some((command.trim().to_owned(), cwd.trim().to_owned()))
    }

    fn done_reap_target(&mut self, target: &str) -> Result<(), String> {
        done_validate_tmux_target(target)?;
        reap_tmux_target(&mut self.runner, target)
    }

    fn done_tmux(&mut self, command: &str, args: &[String]) -> Result<String, String> {
        maw_tmux::TmuxRunner::run(&mut self.runner, command, args).map_err(|error| error.message)
    }

    fn done_send_text(&mut self, target: &str, text: &str) -> Result<(), String> {
        done_validate_tmux_target(target)?;
        let mut client = TmuxClient::local();
        if std::env::var("MAW_TEST_MODE").ok().as_deref() == Some("1") {
            client
                .send_text_ungated_with_sleeper(target, text, |_| {})
                .map(|_| ())
                .map_err(|error| error.message)
        } else {
            client
                .send_text_ungated(target, text)
                .map(|_| ())
                .map_err(|error| error.message)
        }
    }

    fn done_git(&mut self, args: &[String]) -> Result<String, String> { done_git(args) }
}

/// Resolve the session and window of the pane that invoked `maw done`.
///
/// Tmux otherwise resolves `display-message` against client focus, which may
/// be a different window after a focus-switching command such as `maw workon`.
fn done_invoking_pane_identity(runner: &mut impl maw_tmux::TmuxRunner) -> Option<(String, i32)> {
    let pane = crate::wind::team::caller_pane()?;
    let args = [
        "-t".to_owned(),
        pane,
        "-p".to_owned(),
        "#{session_name}\t#{window_index}".to_owned(),
    ];
    let raw = runner.run("display-message", &args).ok()?;
    let (session, index) = raw.trim().split_once('\t')?;
    Some((session.to_owned(), index.parse::<i32>().ok()?))
}

fn done_parse_window_line(line: &str) -> Option<DoneWindow> {
    let mut parts = line.split("|||");
    let session = parts.next()?.to_owned();
    let index = parts.next()?.parse::<i32>().ok()?;
    let name = parts.next()?.to_owned();
    let _ = parts.next();
    let cwd = parts.next().map(str::trim).filter(|value| !value.is_empty()).map(ToOwned::to_owned);
    if session.is_empty() || name.is_empty() { return None; }
    Some(DoneWindow { session, index, name, cwd })
}

fn done_find_window(windows: &[DoneWindow], target_lower: &str, session_filter: Option<&str>) -> Option<DoneWindow> {
    windows.iter().find(|window| session_filter.is_none_or(|session| session == window.session) && window.name.eq_ignore_ascii_case(target_lower)).cloned()
}

fn done_assert_may_target_lead(window: &DoneWindow, windows: &[DoneWindow], local: &mut impl DoneRuntime, stdout: &mut String) -> Result<(), String> {
    let current = local.done_current_identity();
    if let Some(message) = crate::wind::done::self_invocation_message(current.as_ref(), &window.session, window.index, &window.name) {
        let _ = writeln!(stdout, "  \x1b[31m✗\x1b[0m {message}");
        stdout.push_str("  \x1b[90m  run maw done from the lead/parent pane after the DONE ping\x1b[0m\n");
        return Err(message);
    }
    let Some(lead) = done_lead_window(windows, &window.session) else {
        let message = format!("refusing to done window '{}' because the lead window for session '{}' could not be identified", window.name, window.session);
        let _ = writeln!(stdout, "  \x1b[31m✗\x1b[0m {message}");
        return Err(message);
    };
    if lead.index != window.index { return Ok(()); }
    let message = format!("refusing to done lead window '{}' in session '{}' from a non-lead context", window.name, window.session);
    let _ = writeln!(stdout, "  \x1b[31m✗\x1b[0m {message}");
    stdout.push_str("  \x1b[90m  run from the lead window, or target a non-lead agent window\x1b[0m\n");
    Err(message)
}

fn done_lead_window(windows: &[DoneWindow], session: &str) -> Option<DoneWindow> {
    let session_stem = done_session_stem(session);
    windows
        .iter()
        .find(|window| window.session == session && done_session_stem(&window.name) == session_stem)
        .cloned()
}

fn done_non_lead_windows(windows: &[DoneWindow], session: &str) -> Vec<DoneWindow> {
    let Some(lead) = done_lead_window(windows, session) else { return Vec::new(); };
    let mut out = windows.iter().filter(|window| window.session == session && window.index != lead.index).cloned().collect::<Vec<_>>();
    out.sort_by_key(|window| window.index);
    out
}

fn done_current_session_name(windows: &[DoneWindow], oracle: Option<&str>, local: &mut impl DoneRuntime) -> Option<String> {
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

fn done_tmux_target(window: &DoneWindow) -> String { format!("{}:{}", window.session, window.name) }

fn done_live_pane_info(window: &DoneWindow, local: &mut impl DoneRuntime) -> Option<DonePaneInfo> {
    let listed_cwd = window.cwd.as_deref().unwrap_or_default();
    match local.done_pane_info(&done_tmux_target(window)) {
        Some((command, cwd)) => {
            let cwd = if cwd.is_empty() { listed_cwd.to_owned() } else { cwd };
            Some(DonePaneInfo { command, cwd })
        }
        None if !listed_cwd.is_empty() => Some(DonePaneInfo { command: String::new(), cwd: listed_cwd.to_owned() }),
        None => None,
    }
}

fn done_auto_save(window: &DoneWindow, options: &DoneOptions, local: &mut impl DoneRuntime, pane_info: Option<&DonePaneInfo>, worktree: Option<&DoneWorktree>, stdout: &mut String) {
    let target = done_tmux_target(window);
    let command = pane_info.map_or("", |info| info.command.as_str());
    let retro = done_retrospective_command(command);
    if options.dry_run {
        if let Some(retro) = retro { let _ = writeln!(stdout, "  \x1b[36m⬡\x1b[0m [dry-run] would send {retro} to {target} and wait 10s"); } else { stdout.push_str("  \x1b[36m⬡\x1b[0m [dry-run] would skip retro (no retrospective command for this engine)\n"); }
        if let Some(worktree) = worktree { let _ = writeln!(stdout, "  \x1b[36m⬡\x1b[0m [dry-run] would git add + commit + push in {}", worktree.full_path.display()); }
        return;
    }
    if let Some(retro) = retro {
        match local.done_send_text(&target, retro) {
            Ok(()) => crate::wind::done::wait_for_retrospective_prompt(
                || local.done_tmux("capture-pane", &["-t".to_owned(), target.clone(), "-p".to_owned(), "-S".to_owned(), "-40".to_owned()]),
                std::thread::sleep,
                stdout,
            ),
            Err(error) => {
                let _ = writeln!(stdout, "  \x1b[33m⚠\x1b[0m could not send {retro} to {target}: {error}");
            }
        }
    }
    if let Some(worktree) = worktree {
        let cwd = worktree.full_path.display().to_string();
        let _ = local.done_git(&["-C".to_owned(), cwd.clone(), "add".to_owned(), "--".to_owned(), ".".to_owned()]);
        let _ = local.done_git(&["-C".to_owned(), cwd.clone(), "commit".to_owned(), "-m".to_owned(), "chore: auto-save before done".to_owned()]);
        if done_should_push_on_done(std::path::Path::new(&cwd)) {
            let _ = local.done_git(&["-C".to_owned(), cwd, "push".to_owned()]);
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

fn done_kill_window(window: &DoneWindow, options: &DoneOptions, local: &mut impl DoneRuntime, stdout: &mut String) {
    let target = done_tmux_target(window);
    if options.dry_run { let _ = writeln!(stdout, "  \x1b[36m⬡\x1b[0m [dry-run] would kill window {target}"); return; }
    match local.done_reap_target(&target).and_then(|()| local.done_tmux("kill-window", &["-t".to_owned(), target.clone()])) { Ok(_) => { let _ = writeln!(stdout, "  \x1b[32m✓\x1b[0m killed window {target}"); }, Err(_) => stdout.push_str("  \x1b[33m⚠\x1b[0m could not kill window (may already be closed)\n") }
}

fn done_retrospective_command(_command: &str) -> Option<&'static str> {
    // L2 agents run /rrr themselves after maw pr (doctrine: "Completion Boundary").
    // maw done no longer sends a duplicate retro prompt.
    None
}

fn done_select_worktree(target: &str, window_lower: &str, options: &DoneOptions, pane_info: Option<&DonePaneInfo>, local: &mut impl DoneRuntime, context: &DoneContext, stdout: &mut String) -> Result<Option<DoneWorktree>, String> {
    if let Some(path) = &options.worktree {
        let Some(worktree) = done_resolve_registered_worktree(local, path, context)? else {
            return Err(format!("done: --worktree path is not a registered git worktree: {}", path.display()));
        };
        let _ = writeln!(stdout, "  worktree: using explicit --worktree {}", worktree.full_path.display());
        return Ok(Some(worktree));
    }

    if let Some(info) = pane_info {
        if info.cwd.is_empty() { return Ok(None); }
        if let Some(live) = done_resolve_registered_worktree(local, std::path::Path::new(&info.cwd), context)? {
            if let Some(registry) = done_worktree_from_config(window_lower, context) {
                if !done_same_path(&registry.full_path, &live.full_path) {
                    let _ = writeln!(stdout, "  worktree: using live pane cwd {} (registry said {}, stale)", live.full_path.display(), registry.full_path.display());
                }
            }
            return Ok(Some(live));
        }
        let _ = writeln!(stdout, "  \x1b[33m⚠\x1b[0m live pane cwd {} is not a registered git worktree; refusing stale registry fallback", info.cwd);
        return Ok(None);
    }

    if let Some(worktree) = done_worktree_from_config(window_lower, context) { return Ok(Some(worktree)); }
    Ok(done_worktree_by_scan(target, &context.repos_root, stdout))
}

fn done_worktree_from_config(window_lower: &str, context: &DoneContext) -> Option<DoneWorktree> {
    for file in done_fleet_config_files(context) {
        let Ok(raw) = std::fs::read_to_string(&file) else { continue; };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&raw) else { continue; };
        let Some(windows) = json.get("windows").and_then(serde_json::Value::as_array) else { continue; };
        let Some(repo) = windows.iter().find(|item| item.get("name").and_then(serde_json::Value::as_str).is_some_and(|name| name.eq_ignore_ascii_case(window_lower))).and_then(|item| item.get("repo")).and_then(serde_json::Value::as_str) else { continue; };
        let Some(worktree) = done_parse_worktree_path(&done_config_repo_path(repo, context), &context.repos_root) else { break; };
        return Some(worktree);
    }
    None
}

fn done_config_repo_path(repo: &str, context: &DoneContext) -> std::path::PathBuf {
    let path = std::path::Path::new(repo);
    if path.is_absolute() { return path.to_path_buf(); }
    if repo.starts_with("github.com/") {
        if let Some(parent) = context.repos_root.parent() { return parent.join(repo); }
    }
    context.repos_root.join(repo)
}

fn done_resolve_registered_worktree(local: &mut impl DoneRuntime, path: &std::path::Path, context: &DoneContext) -> Result<Option<DoneWorktree>, String> {
    done_validate_exec_path(path)?;
    let top_level = match local.done_git(&["-C".to_owned(), path.display().to_string(), "rev-parse".to_owned(), "--show-toplevel".to_owned()]) {
        Ok(output) => std::path::PathBuf::from(output.trim()),
        Err(_) => return Ok(None),
    };
    if top_level.as_os_str().is_empty() {
        return Ok(None);
    }
    done_validate_exec_path(&top_level)?;
    let Ok(raw) = local.done_git(&[
        "-C".to_owned(),
        top_level.display().to_string(),
        "worktree".to_owned(),
        "list".to_owned(),
        "--porcelain".to_owned(),
    ]) else {
        return Ok(None);
    };
    Ok(done_worktree_from_git_list(&raw, &top_level, context))
}

fn done_worktree_from_git_list(raw: &str, full_path: &std::path::Path, context: &DoneContext) -> Option<DoneWorktree> {
    let paths = raw
        .lines()
        .filter_map(|line| line.strip_prefix("worktree "))
        .map(std::path::PathBuf::from)
        .collect::<Vec<_>>();
    if !paths.iter().any(|path| done_same_path(path, full_path)) {
        return None;
    }
    let parsed = done_parse_worktree_path(full_path, &context.repos_root).or_else(|| {
        done_repos_root_from_cwd(full_path)
            .and_then(|repos_root| done_parse_worktree_path(full_path, &repos_root))
    });
    let listed_main = paths.first()?;
    let main_path = if done_same_path(listed_main, full_path) {
        parsed.as_ref()?.main_path.clone()
    } else {
        listed_main.clone()
    };
    done_validate_exec_path(&main_path).ok()?;
    let label = parsed.map_or_else(|| full_path.display().to_string(), |worktree| worktree.label);
    Some(DoneWorktree {
        main_path,
        full_path: full_path.to_path_buf(),
        label,
    })
}

fn done_same_path(left: &std::path::Path, right: &std::path::Path) -> bool {
    if left == right { return true; }
    let Ok(left) = std::fs::canonicalize(left) else { return false; };
    let Ok(right) = std::fs::canonicalize(right) else { return false; };
    left == right
}

#[cfg(test)]
fn done_run_process(command: &str, args: &[&str], cwd: Option<&std::path::Path>) -> String {
    let mut process = if command == "git" { std::process::Command::new(done_git_executable()) } else { std::process::Command::new(command) };
    process.args(args);
    if let Some(cwd) = cwd { process.current_dir(cwd); }
    let output = process.output().unwrap_or_else(|error| panic!("failed to run {process:?}: {error}"));
    assert!(
        output.status.success(),
        "{process:?} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).to_string()
}

#[cfg(test)]
fn done_git_executable() -> std::path::PathBuf {
    ["/opt/homebrew/bin/git", "/usr/local/bin/git", "/usr/bin/git", "/bin/git"]
        .into_iter()
        .map(std::path::PathBuf::from)
        .find(|path| path.is_file())
        .unwrap_or_else(|| std::path::PathBuf::from("git"))
}

fn done_worktree_by_scan(target: &str, repos_root: &std::path::Path, stdout: &mut String) -> Option<DoneWorktree> {
    let matches = done_find_worktree_paths(target, repos_root);
    if matches.len() > 1 { let _ = writeln!(stdout, "  \x1b[31m✗\x1b[0m refusing to remove worktree '{}' — matches {} repos", target, matches.len()); return None; }
    matches.first().cloned()
}

fn done_rescue_psi_notes(worktree: &DoneWorktree, stdout: &mut String) {
    // Copy uncommitted ψ/ brain notes out of the worktree into the owning main
    // checkout BEFORE auto-save sweeps them into a branch that --clean-branch may
    // force-delete (git branch -D) before the PR merges — losing the notes to GC.
    // Never overwrites existing files; best-effort (rescue failure must not block
    // the rest of `done`).
    match crate::wind::done::rescue_psi(&worktree.full_path, &worktree.main_path) {
        Ok(rescued) if !rescued.is_empty() => {
            let _ = writeln!(stdout, "  \x1b[32m✓\x1b[0m rescued {} uncommitted ψ note(s) to main before removal", rescued.len());
        }
        Ok(_) => {}
        Err(error) => {
            let _ = writeln!(stdout, "  \x1b[33m⚠\x1b[0m ψ rescue skipped: {error}");
        }
    }
}

fn done_remove_selected_worktree(worktree: &DoneWorktree, options: &DoneOptions, local: &mut impl DoneRuntime, stdout: &mut String) -> Result<(), String> {
    if options.dry_run { let _ = writeln!(stdout, "  \x1b[36m⬡\x1b[0m [dry-run] would remove worktree {}", worktree.label); return Ok(()); }
    done_remove_worktree(worktree, options, local, stdout)
}

fn done_remove_worktree(worktree: &DoneWorktree, options: &DoneOptions, local: &mut impl DoneRuntime, stdout: &mut String) -> Result<(), String> {
    done_validate_exec_path(&worktree.main_path)?;
    done_validate_exec_path(&worktree.full_path)?;
    let cargo_target_dir = done_managed_cargo_target_dir(&worktree.full_path);
    let branch = local.done_git(&["-C".to_owned(), worktree.full_path.display().to_string(), "rev-parse".to_owned(), "--abbrev-ref".to_owned(), "HEAD".to_owned()]).unwrap_or_default().trim().to_owned();
    let mut remove_args = vec!["-C".to_owned(), worktree.main_path.display().to_string(), "worktree".to_owned(), "remove".to_owned()];
    if options.force {
        remove_args.push("--force".to_owned());
    }
    remove_args.extend(["--".to_owned(), worktree.full_path.display().to_string()]);
    local.done_git(&remove_args)?;
    if let Some(target_dir) = cargo_target_dir {
        done_reclaim_cargo_target_dir(&target_dir, stdout);
    }
    local.done_git(&["-C".to_owned(), worktree.main_path.display().to_string(), "worktree".to_owned(), "prune".to_owned()])?;
    let _ = writeln!(stdout, "  \x1b[32m✓\x1b[0m removed worktree {}", worktree.label);
    done_cleanup_branch(&worktree.main_path, &branch, options, local, stdout);
    Ok(())
}

fn done_managed_cargo_target_dir(worktree_path: &std::path::Path) -> Option<std::path::PathBuf> {
    let target_dir = done_cargo_target_dir(worktree_path)?;
    (Some(&target_dir) == done_expected_cargo_target_dir(worktree_path).as_ref()).then_some(target_dir)
}

fn done_cargo_target_dir(worktree_path: &std::path::Path) -> Option<std::path::PathBuf> {
    let config = std::fs::read_to_string(worktree_path.join(".cargo/config.toml")).ok()?;
    let mut in_build = false;
    for raw_line in config.lines() {
        let line = raw_line.split_once('#').map_or(raw_line, |(before, _)| before).trim();
        if line.starts_with('[') {
            in_build = line == "[build]";
            continue;
        }
        if !in_build {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != "target-dir" {
            continue;
        }
        let value = value.trim().strip_prefix('"')?.strip_suffix('"')?;
        return Some(std::path::PathBuf::from(value));
    }
    None
}

fn done_expected_cargo_target_dir(worktree_path: &std::path::Path) -> Option<std::path::PathBuf> {
    let slug = worktree_path.file_name()?.to_str()?;
    (!slug.is_empty()).then(|| done_cargo_target_root().join(format!("maw-rs-target-{slug}")))
}

fn done_cargo_target_root() -> std::path::PathBuf {
    if cfg!(unix) { std::path::PathBuf::from("/tmp") } else { std::env::temp_dir() }
}

fn done_reclaim_cargo_target_dir(target_dir: &std::path::Path, stdout: &mut String) {
    if std::fs::symlink_metadata(target_dir).is_err() {
        return;
    }
    let freed = done_path_size_bytes(target_dir).map(done_format_reclaimed_bytes);
    match std::fs::remove_dir_all(target_dir) {
        Ok(()) => {
            let freed = freed.unwrap_or_else(|| "size unavailable".to_owned());
            let _ = writeln!(stdout, "  \x1b[32m✓\x1b[0m reclaimed CARGO_TARGET_DIR {} ({freed} freed)", target_dir.display());
        }
        Err(error) => {
            let _ = writeln!(stdout, "  \x1b[33m⚠\x1b[0m could not reclaim CARGO_TARGET_DIR {}: {error}", target_dir.display());
        }
    }
}

fn done_path_size_bytes(path: &std::path::Path) -> Option<u64> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if metadata.file_type().is_symlink() || metadata.is_file() {
        return Some(metadata.len());
    }
    if !metadata.is_dir() {
        return Some(0);
    }
    std::fs::read_dir(path)
        .ok()?
        .flatten()
        .try_fold(0_u64, |total, entry| done_path_size_bytes(&entry.path()).map(|size| total.saturating_add(size)))
}

fn done_format_reclaimed_bytes(bytes: u64) -> String {
    const MIB: u64 = 1024 * 1024;
    if bytes >= MIB {
        format!("{} MiB", bytes / MIB)
    } else {
        format!("{bytes} bytes")
    }
}

fn done_cleanup_branch(main_path: &std::path::Path, branch: &str, options: &DoneOptions, local: &mut impl DoneRuntime, stdout: &mut String) {
    if branch.is_empty() || branch == "main" || branch == "HEAD" { return; }
    if options.clean_branch { let _ = local.done_git(&["-C".to_owned(), main_path.display().to_string(), "branch".to_owned(), "-D".to_owned(), "--".to_owned(), branch.to_owned()]); let _ = writeln!(stdout, "  \x1b[32m✓\x1b[0m deleted branch {branch}"); } else { let _ = writeln!(stdout, "  \x1b[90m○\x1b[0m branch {branch} retained (use --clean-branch to delete)"); }
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
        .filter(fleet_entry_is_session)
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

fn done_validate_worktree_arg(value: &str) -> Result<(), String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.starts_with('-') || trimmed != value { return Err(format!("done: invalid --worktree path '{value}'")); }
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

    #[derive(Default)]
    struct DoneFakeRuntime {
        windows: Vec<DoneWindow>,
        current: Option<(String, i32)>,
        pane_info: std::collections::BTreeMap<String, (String, String)>,
        top_levels: std::collections::BTreeMap<std::path::PathBuf, std::path::PathBuf>,
        registered: std::collections::BTreeMap<std::path::PathBuf, Vec<std::path::PathBuf>>,
        branches: std::collections::BTreeMap<std::path::PathBuf, String>,
        dirty_removals: std::collections::BTreeSet<std::path::PathBuf>,
        git_calls: Vec<Vec<String>>,
        tmux_calls: Vec<(String, Vec<String>)>,
        sent_text: Vec<(String, String)>,
    }

    impl DoneFakeRuntime {
        fn register_worktree(&mut self, main: &std::path::Path, worktree: &std::path::Path) {
            self.top_levels.insert(worktree.to_path_buf(), worktree.to_path_buf());
            self.registered.entry(main.to_path_buf()).or_default().push(worktree.to_path_buf());
            self.branches.insert(worktree.to_path_buf(), "agent/task".to_owned());
        }

        fn git_cwd(args: &[String]) -> Option<std::path::PathBuf> {
            args.windows(2).find_map(|pair| (pair[0] == "-C").then(|| std::path::PathBuf::from(&pair[1])))
        }

        fn arg_after_separator(args: &[String]) -> Option<std::path::PathBuf> {
            args.iter().position(|arg| arg == "--").and_then(|index| args.get(index + 1)).map(std::path::PathBuf::from)
        }
    }

    impl DoneRuntime for DoneFakeRuntime {
        fn done_list_windows(&mut self) -> Vec<DoneWindow> { self.windows.clone() }

        fn done_current_identity(&mut self) -> Option<(String, i32)> { self.current.clone() }

        fn done_pane_info(&mut self, target: &str) -> Option<(String, String)> { self.pane_info.get(target).cloned() }

        fn done_reap_target(&mut self, _target: &str) -> Result<(), String> { Ok(()) }

        fn done_tmux(&mut self, command: &str, args: &[String]) -> Result<String, String> {
            self.tmux_calls.push((command.to_owned(), args.to_vec()));
            Ok(String::new())
        }

        fn done_send_text(&mut self, target: &str, text: &str) -> Result<(), String> {
            self.sent_text.push((target.to_owned(), text.to_owned()));
            Ok(())
        }

        fn done_git(&mut self, args: &[String]) -> Result<String, String> {
            self.git_calls.push(args.to_vec());
            let cwd = Self::git_cwd(args).ok_or_else(|| "missing -C".to_owned())?;
            if args.ends_with(&["rev-parse".to_owned(), "--show-toplevel".to_owned()]) {
                return self
                    .top_levels
                    .get(&cwd)
                    .map(|path| format!("{}\n", path.display()))
                    .ok_or_else(|| "not a git repository".to_owned());
            }
            if args.ends_with(&["worktree".to_owned(), "list".to_owned(), "--porcelain".to_owned()]) {
                let registered = self.registered.iter().find(|(main, worktrees)| {
                    *main == &cwd || worktrees.iter().any(|worktree| worktree == &cwd)
                });
                let out = if let Some((main, worktrees)) = registered {
                    let mut out = format!("worktree {}\n\n", main.display());
                    for worktree in worktrees {
                        if worktree != main {
                            let _ = write!(out, "worktree {}\n\n", worktree.display());
                        }
                    }
                    out
                } else {
                    format!("worktree {}\n\n", cwd.display())
                };
                return Ok(out);
            }
            if args.ends_with(&["rev-parse".to_owned(), "--abbrev-ref".to_owned(), "HEAD".to_owned()]) {
                return Ok(format!("{}\n", self.branches.get(&cwd).map_or("agent/task", String::as_str)));
            }
            if args.iter().any(|arg| arg == "remove") {
                let worktree = Self::arg_after_separator(args).ok_or_else(|| "missing worktree path".to_owned())?;
                if self.dirty_removals.contains(&worktree) && !args.iter().any(|arg| arg == "--force") {
                    return Err(format!("fatal: '{}' contains modified or untracked files", worktree.display()));
                }
                return Ok(String::new());
            }
            Ok(String::new())
        }
    }

    struct DoneRealGitRuntime { git: std::path::PathBuf }

    impl Default for DoneRealGitRuntime {
        fn default() -> Self { Self { git: done_git_executable() } }
    }

    impl DoneRuntime for DoneRealGitRuntime {
        fn done_list_windows(&mut self) -> Vec<DoneWindow> { Vec::new() }

        fn done_current_identity(&mut self) -> Option<(String, i32)> { None }

        fn done_pane_info(&mut self, _target: &str) -> Option<(String, String)> { None }

        fn done_reap_target(&mut self, _target: &str) -> Result<(), String> { Err("tmux unavailable in real-git test runtime".to_owned()) }

        fn done_tmux(&mut self, _command: &str, _args: &[String]) -> Result<String, String> { Err("tmux unavailable in real-git test runtime".to_owned()) }

        fn done_send_text(&mut self, _target: &str, _text: &str) -> Result<(), String> { Err("tmux unavailable in real-git test runtime".to_owned()) }

        fn done_git(&mut self, args: &[String]) -> Result<String, String> {
            let output = std::process::Command::new(&self.git).args(args).output().map_err(|error| format!("git failed: {error}"))?;
            if output.status.success() { Ok(String::from_utf8_lossy(&output.stdout).to_string()) } else { Err(String::from_utf8_lossy(&output.stderr).trim().to_owned()) }
        }
    }

    struct DoneTempRoot { path: std::path::PathBuf }

    impl DoneTempRoot {
        fn new(name: &str) -> Self {
            static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!("maw-rs-done-{name}-{}-{seq}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("temp root");
            Self { path }
        }

        fn repos_root(&self) -> std::path::PathBuf { self.path.join("github.com") }

        fn fleet_dir(&self) -> std::path::PathBuf { self.path.join("fleet") }

        fn context(&self) -> DoneContext {
            DoneContext { repos_root: self.repos_root(), fleet_dirs: vec![self.fleet_dir()] }
        }
    }

    impl Drop for DoneTempRoot {
        fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.path); }
    }

    fn done_test_window(name: &str) -> DoneWindow {
        let name = if name == "lead" { "s" } else { name };
        DoneWindow { session: "s".to_owned(), index: if name == "s" { 1 } else { 2 }, name: name.to_owned(), cwd: None }
    }

    fn done_test_window_with_cwd(name: &str, cwd: &std::path::Path) -> DoneWindow {
        let name = if name == "lead" { "s" } else { name };
        DoneWindow { session: "s".to_owned(), index: if name == "s" { 1 } else { 2 }, name: name.to_owned(), cwd: Some(cwd.display().to_string()) }
    }

    fn done_write_fleet(root: &DoneTempRoot, window: &str, repo: &str) {
        let fleet_dir = root.fleet_dir();
        std::fs::create_dir_all(&fleet_dir).expect("fleet dir");
        std::fs::write(fleet_dir.join("s.json"), format!(r#"{{"name":"s","windows":[{{"name":"{window}","repo":"{repo}"}}]}}"#)).expect("fleet");
    }

    fn done_args(args: &[&str]) -> Vec<String> { args.iter().map(|arg| (*arg).to_owned()).collect() }

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
    fn done_all_uses_session_identity_not_lowest_window_index() {
        let root = DoneTempRoot::new("inverted-window-order");
        let mut runtime = DoneFakeRuntime {
            windows: vec![
                DoneWindow { session: "01-gale".to_owned(), index: 1, name: "finished-worker".to_owned(), cwd: None },
                DoneWindow { session: "01-gale".to_owned(), index: 2, name: "gale".to_owned(), cwd: None },
                DoneWindow { session: "01-gale".to_owned(), index: 3, name: "active-worker".to_owned(), cwd: None },
            ],
            current: Some(("01-gale".to_owned(), 2)),
            ..DoneFakeRuntime::default()
        };

        let out = done_run_with_context(&done_args(&["--all", "--dry-run"]), &mut runtime, &root.context()).expect("done --all");

        assert!(out.contains("done 01-gale:finished-worker"), "{out}");
        assert!(out.contains("done 01-gale:active-worker"), "{out}");
        assert!(!out.contains("done 01-gale:gale"), "{out}");
        assert!(out.contains("done --all would process 2 window(s)"), "{out}");
    }

    #[test]
    fn done_allows_lowest_index_worker_when_session_lead_is_higher() {
        let root = DoneTempRoot::new("lowest-worker");
        let mut runtime = DoneFakeRuntime {
            windows: vec![
                DoneWindow { session: "01-gale".to_owned(), index: 1, name: "finished-worker".to_owned(), cwd: None },
                DoneWindow { session: "01-gale".to_owned(), index: 2, name: "gale".to_owned(), cwd: None },
            ],
            current: Some(("01-gale".to_owned(), 2)),
            ..DoneFakeRuntime::default()
        };

        let out = done_run_with_context(&done_args(&["finished-worker", "--dry-run"]), &mut runtime, &root.context()).expect("done worker");

        assert!(out.contains("would kill window 01-gale:finished-worker"), "{out}");
    }

    #[test]
    fn done_still_refuses_self_invocation_with_inverted_window_order() {
        let root = DoneTempRoot::new("self-invocation");
        let mut runtime = DoneFakeRuntime {
            windows: vec![
                DoneWindow { session: "01-gale".to_owned(), index: 1, name: "finished-worker".to_owned(), cwd: None },
                DoneWindow { session: "01-gale".to_owned(), index: 2, name: "gale".to_owned(), cwd: None },
            ],
            current: Some(("01-gale".to_owned(), 2)),
            ..DoneFakeRuntime::default()
        };

        let error = done_run_with_context(&done_args(&["gale", "--dry-run"]), &mut runtime, &root.context()).expect_err("self invocation rejected");

        assert_eq!(error, "refusing to done current window 'gale' in session '01-gale'");
    }

    #[test]
    fn done_removes_session_window_without_mutating_squad_roster() {
        let root = DoneTempRoot::new("squad-boundary");
        done_write_fleet(&root, "worker", "acme/app");
        let roster = root.fleet_dir().join("squads/01-core/squad.json");
        std::fs::create_dir_all(roster.parent().expect("roster parent")).expect("roster dir");
        let roster_body = r#"{"name":"01-core","windows":[{"name":"worker","repo":"acme/roster"}],"members":[]}"#;
        std::fs::write(&roster, roster_body).expect("roster");

        assert!(done_remove_from_fleet_config("worker", &root.context(), &mut String::new()));
        assert_eq!(std::fs::read_to_string(roster).expect("roster remains"), roster_body);
        let session = std::fs::read_to_string(root.fleet_dir().join("s.json")).expect("session");
        assert_eq!(serde_json::from_str::<serde_json::Value>(&session).expect("json")["windows"], serde_json::json!([]));
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
    fn done_live_cwd_differs_from_registry_live_wins_with_warning() {
        let root = DoneTempRoot::new("live-wins");
        let context = root.context();
        let main = context.repos_root.join("acme/app");
        let live = main.join("agents/live-task");
        let stale = main.join("agents/stale-task");
        done_write_fleet(&root, "worker", "acme/app/agents/stale-task");

        let mut runtime = DoneFakeRuntime { windows: vec![done_test_window("lead"), done_test_window("worker")], ..DoneFakeRuntime::default() };
        runtime.pane_info.insert("s:worker".to_owned(), ("codex".to_owned(), live.display().to_string()));
        runtime.register_worktree(&main, &live);
        runtime.register_worktree(&main, &stale);

        let out = done_run_with_context(&done_args(&["worker", "--dry-run"]), &mut runtime, &context).expect("done");
        assert!(out.contains(&format!("worktree: using live pane cwd {} (registry said {}, stale)", live.display(), stale.display())), "{out}");
        assert!(out.contains("would remove worktree acme/app/agents/live-task"), "{out}");
        assert!(!out.contains("would remove worktree acme/app/agents/stale-task"), "{out}");
    }

    #[test]
    fn done_cd_redispatched_window_resolves_listed_live_cwd_not_stale_registry() {
        let root = DoneTempRoot::new("listed-live-wins");
        let context = root.context();
        let main = context.repos_root.join("acme/app");
        let live = main.join("agents/new-task");
        let stale = main.join("agents/old-task");
        done_write_fleet(&root, "worker", "acme/app/agents/old-task");

        let mut runtime = DoneFakeRuntime {
            windows: vec![done_test_window("lead"), done_test_window_with_cwd("worker", &live)],
            ..DoneFakeRuntime::default()
        };
        runtime.register_worktree(&main, &live);
        runtime.register_worktree(&main, &stale);

        let out = done_run_with_context(&done_args(&["worker", "--dry-run"]), &mut runtime, &context).expect("done");
        assert!(out.contains(&format!("worktree: using live pane cwd {} (registry said {}, stale)", live.display(), stale.display())), "{out}");
        assert!(out.contains("would remove worktree acme/app/agents/new-task"), "{out}");
        assert!(!out.contains("would remove worktree acme/app/agents/old-task"), "{out}");
    }

    #[test]
    fn done_real_git_worktree_resolves_when_context_repos_root_differs() {
        let root = DoneTempRoot::new("real-git-live-root");
        let main = root.repos_root().join("acme/app");
        let live = main.join("agents/live-task");
        std::fs::create_dir_all(&main).expect("main repo dir");
        std::fs::create_dir_all(main.join("agents")).expect("agents dir");

        done_run_process("git", &["init"], Some(&main));
        done_run_process("git", &["-c", "user.name=maw-test", "-c", "user.email=maw-test@example.invalid", "-c", "commit.gpgsign=false", "commit", "--allow-empty", "-m", "init"], Some(&main));
        let live_path = live.display().to_string();
        done_run_process("git", &["worktree", "add", "-b", "agents/live-task", &live_path], Some(&main));

        let wrong_context = DoneContext { repos_root: root.path.join("wrong-ghq/github.com"), fleet_dirs: Vec::new() };
        let mut runtime = DoneRealGitRuntime::default();
        let resolved = done_resolve_registered_worktree(&mut runtime, &live, &wrong_context).expect("resolve").expect("registered worktree");

        assert!(done_same_path(&resolved.main_path, &main), "{} != {}", resolved.main_path.display(), main.display());
        assert!(done_same_path(&resolved.full_path, &live), "{} != {}", resolved.full_path.display(), live.display());
        assert_eq!(resolved.label, "acme/app/agents/live-task");
    }

    #[test]
    fn done_live_cwd_outside_known_layout_uses_git_worktree_list() {
        let root = DoneTempRoot::new("live-git-list");
        let context = root.context();
        let main = root.path.join("arbitrary/main-checkout");
        let live = root.path.join("arbitrary/worker-checkout");
        let mut runtime = DoneFakeRuntime {
            windows: vec![done_test_window("lead"), done_test_window("worker")],
            ..DoneFakeRuntime::default()
        };
        runtime
            .pane_info
            .insert("s:worker".to_owned(), ("codex".to_owned(), live.display().to_string()));
        runtime.register_worktree(&main, &live);

        let out = done_run_with_context(
            &done_args(&["worker", "--dry-run"]),
            &mut runtime,
            &context,
        )
        .expect("done");

        assert!(
            out.contains(&format!("would remove worktree {}", live.display())),
            "{out}"
        );
        assert!(
            runtime.git_calls.iter().any(|args| {
                args == &vec![
                    "-C".to_owned(),
                    live.display().to_string(),
                    "worktree".to_owned(),
                    "list".to_owned(),
                    "--porcelain".to_owned(),
                ]
            }),
            "{:#?}",
            runtime.git_calls
        );
    }

    #[test]
    fn done_removes_worktree_before_cleaning_its_local_branch() {
        let root = DoneTempRoot::new("branch-cleanup-order");
        let context = root.context();
        let main = context.repos_root.join("acme/app");
        let live = main.join("agents/merged-task");
        let mut runtime = DoneFakeRuntime {
            windows: vec![done_test_window("lead"), done_test_window("worker")],
            ..DoneFakeRuntime::default()
        };
        runtime
            .pane_info
            .insert("s:worker".to_owned(), ("codex".to_owned(), live.display().to_string()));
        runtime.register_worktree(&main, &live);

        done_run_with_context(
            &done_args(&["worker", "--force", "--clean-branch"]),
            &mut runtime,
            &context,
        )
        .expect("done");

        let remove = runtime
            .git_calls
            .iter()
            .position(|args| args.get(2).is_some_and(|arg| arg == "worktree") && args.get(3).is_some_and(|arg| arg == "remove"))
            .expect("worktree remove");
        let branch_delete = runtime
            .git_calls
            .iter()
            .position(|args| args.get(2).is_some_and(|arg| arg == "branch") && args.get(3).is_some_and(|arg| arg == "-D"))
            .expect("branch delete");
        assert!(remove < branch_delete, "{:#?}", runtime.git_calls);
        assert_eq!(runtime.git_calls[remove][1], main.display().to_string());
        assert_eq!(runtime.git_calls[branch_delete][1], main.display().to_string());
    }

    #[test]
    fn done_reclaims_the_worktree_isolated_cargo_target() {
        let root = DoneTempRoot::new("reclaim-target");
        let context = root.context();
        let main = context.repos_root.join("acme/app");
        let slug = root.path.file_name().expect("root name").to_string_lossy();
        let live = main.join("agents").join(slug.as_ref());
        let target = std::path::PathBuf::from("/tmp").join(format!("maw-rs-target-{slug}"));
        std::fs::create_dir_all(live.join(".cargo")).expect("worktree cargo config dir");
        std::fs::create_dir_all(&target).expect("isolated target dir");
        std::fs::write(target.join("artifact"), "test artifact").expect("target artifact");
        std::fs::write(
            live.join(".cargo/config.toml"),
            format!("[build]\ntarget-dir = \"{}\"\n", target.display()),
        )
        .expect("target config");

        let mut runtime = DoneFakeRuntime {
            windows: vec![done_test_window("lead"), done_test_window("worker")],
            ..DoneFakeRuntime::default()
        };
        runtime
            .pane_info
            .insert("s:worker".to_owned(), ("codex".to_owned(), live.display().to_string()));
        runtime.register_worktree(&main, &live);

        let out = done_run_with_context(&done_args(&["worker", "--force"]), &mut runtime, &context)
            .expect("done");

        assert!(!target.exists(), "target should be reclaimed: {}", target.display());
        assert!(out.contains("reclaimed CARGO_TARGET_DIR"), "{out}");
    }

    #[test]
    fn done_dead_pane_falls_back_to_registry() {
        let root = DoneTempRoot::new("dead-registry");
        let context = root.context();
        let stale = context.repos_root.join("acme/app/agents/stale-task");
        done_write_fleet(&root, "worker", "acme/app/agents/stale-task");
        let mut runtime = DoneFakeRuntime::default();

        let out = done_run_with_context(&done_args(&["worker", "--dry-run"]), &mut runtime, &context).expect("done");
        assert!(out.contains("would remove worktree acme/app/agents/stale-task"), "{out}");
        assert!(!out.contains("using live pane cwd"), "{out}");
        assert!(out.contains("window 'worker' not running"), "{out}");
        assert!(out.contains(&stale.display().to_string()) || out.contains("acme/app/agents/stale-task"), "{out}");
    }

    #[test]
    fn done_worktree_override_wins_over_live_and_registry() {
        let root = DoneTempRoot::new("override");
        let context = root.context();
        let main = context.repos_root.join("acme/app");
        let live = main.join("agents/live-task");
        let stale = main.join("agents/stale-task");
        let override_path = main.join("agents/override-task");
        done_write_fleet(&root, "worker", "acme/app/agents/stale-task");

        let mut runtime = DoneFakeRuntime { windows: vec![done_test_window("lead"), done_test_window("worker")], ..DoneFakeRuntime::default() };
        runtime.pane_info.insert("s:worker".to_owned(), ("codex".to_owned(), live.display().to_string()));
        runtime.register_worktree(&main, &live);
        runtime.register_worktree(&main, &stale);
        runtime.register_worktree(&main, &override_path);

        let out = done_run_with_context(&done_args(&["worker", "--dry-run", "--worktree", &override_path.display().to_string()]), &mut runtime, &context).expect("done");
        assert!(out.contains(&format!("worktree: using explicit --worktree {}", override_path.display())), "{out}");
        assert!(out.contains(&format!("would git add + commit + push in {}", override_path.display())), "{out}");
        assert!(out.contains("would remove worktree acme/app/agents/override-task"), "{out}");
        assert!(!out.contains("would remove worktree acme/app/agents/live-task"), "{out}");
        assert!(!out.contains("would remove worktree acme/app/agents/stale-task"), "{out}");
    }

    #[test]
    fn done_worktree_override_rejects_non_worktree_path() {
        let root = DoneTempRoot::new("override-reject");
        let context = root.context();
        let main = context.repos_root.join("acme/app");
        let mut runtime = DoneFakeRuntime::default();
        runtime.top_levels.insert(main.clone(), main.clone());

        let err = done_run_with_context(&done_args(&["worker", "--dry-run", "--worktree", &main.display().to_string()]), &mut runtime, &context).expect_err("reject");
        assert!(err.contains("--worktree path is not a registered git worktree"), "{err}");
    }

    #[test]
    fn done_dirty_worktree_removal_is_refused_without_force() {
        let root = DoneTempRoot::new("dirty");
        let context = root.context();
        let main = context.repos_root.join("acme/app");
        let dirty = main.join("agents/dirty-task");
        done_write_fleet(&root, "worker", "acme/app/agents/dirty-task");

        let mut runtime = DoneFakeRuntime::default();
        runtime.dirty_removals.insert(dirty.clone());

        let err = done_run_with_context(&done_args(&["worker"]), &mut runtime, &context).expect_err("dirty");
        assert!(err.contains("contains modified or untracked files"), "{err}");
        assert!(runtime.git_calls.iter().all(|args| !args.iter().any(|arg| arg == "--force")), "{:?}", runtime.git_calls);
    }

    #[test]
    fn done_force_removes_dirty_worktree_with_git_force() {
        let root = DoneTempRoot::new("dirty-force");
        let context = root.context();
        let main = context.repos_root.join("acme/app");
        let dirty = main.join("agents/dirty-task");
        done_write_fleet(&root, "worker", "acme/app/agents/dirty-task");

        let mut runtime = DoneFakeRuntime::default();
        runtime.dirty_removals.insert(dirty.clone());

        done_run_with_context(&done_args(&["worker", "--force"]), &mut runtime, &context)
            .expect("forced removal of dirty worktree");

        assert!(
            runtime.git_calls.iter().any(|args| {
                args == &vec![
                    "-C".to_owned(),
                    main.display().to_string(),
                    "worktree".to_owned(),
                    "remove".to_owned(),
                    "--force".to_owned(),
                    "--".to_owned(),
                    dirty.display().to_string(),
                ]
            }),
            "{:#?}",
            runtime.git_calls
        );
    }

    #[test]
    fn done_removes_a_solo_worktree_and_releases_its_lease_without_fleet_config() {
        let _lock = env_test_lock().lock().expect("env lock");
        let _state = EnvVarRestore::capture("MAW_STATE_DIR");
        let root = DoneTempRoot::new("solo-lease");
        std::env::set_var("MAW_STATE_DIR", root.path.join("state"));
        let context = root.context();
        let main = context.repos_root.join("acme/app");
        let worktree = main.join("agents/solo-task");
        let lease = solo_lease_path("app");
        solo_acquire_lease(&lease, "s:worker", |_| true).expect("lease");
        solo_set_lease_worktree(&lease, "s:worker", &worktree).expect("worktree record");

        let mut runtime = DoneFakeRuntime { windows: vec![done_test_window("lead"), done_test_window("worker")], ..DoneFakeRuntime::default() };
        runtime.register_worktree(&main, &worktree);

        done_run_with_context(&done_args(&["worker", "--force"]), &mut runtime, &context).expect("done solo");

        assert!(!lease.exists(), "done must release the solo lease");
        assert!(runtime.git_calls.iter().any(|args| args.iter().any(|arg| arg == "remove") && args.iter().any(|arg| arg == &worktree.display().to_string())), "{:#?}", runtime.git_calls);
    }
}
