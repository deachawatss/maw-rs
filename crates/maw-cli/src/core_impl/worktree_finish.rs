const DISPATCH_57: &[DispatcherEntry] = &[
    DispatcherEntry { command: "done", handler: Handler::Sync(run_done_command) },
    DispatcherEntry { command: "finish", handler: Handler::Sync(run_done_command) },
];

const DONE_USAGE: &str = "usage: maw done <window-name> [--force] [--dry-run] [--keep-branch] [--clean-branch] [--worktree <path>] or maw done --all [<oracle>] [--force] [--dry-run] [--keep-branch] [--clean-branch]  (see: maw sleep/kill for non-worktree shutdown)";
const DONE_ALL_USAGE: &str = "usage: maw done --all [<oracle>] [--force] [--dry-run] [--keep-branch] [--clean-branch]";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
struct DoneOptions { all: bool, force: bool, dry_run: bool, clean_branch: bool, keep_branch: bool, target: Option<String>, worktree: Option<std::path::PathBuf> }

#[derive(Debug, Clone, PartialEq, Eq)]
struct DoneWindow { session: String, index: i32, name: String, cwd: Option<String> }

#[derive(Debug, Clone, PartialEq, Eq)]
struct DonePane {
    session: String,
    window_index: i32,
    window_name: String,
    pane_index: i32,
    pane_id: String,
    active: bool,
    command: String,
    cwd: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DoneWorktree { main_path: std::path::PathBuf, full_path: std::path::PathBuf, label: String }

#[derive(Debug, Clone, PartialEq, Eq)]
struct DonePaneInfo { command: String, cwd: String }

#[derive(Debug, Clone)]
struct DoneContext {
    repos_root: std::path::PathBuf,
    fleet_dirs: Vec<std::path::PathBuf>,
    solo_lease_dir: std::path::PathBuf,
}

impl DoneContext {
    fn from_env() -> Self {
        let env = current_xdg_env();
        Self {
            repos_root: ghq_root().join("github.com"),
            fleet_dirs: fleet_read_dirs_for_env(&env),
            solo_lease_dir: maw_state_path(&env, &["lease"]),
        }
    }

    fn with_cwd(cwd: &std::path::Path) -> Self {
        let env = current_xdg_env();
        Self {
            repos_root: done_repos_root_from_cwd(cwd)
                .unwrap_or_else(|| ghq_root().join("github.com")),
            fleet_dirs: fleet_read_dirs_for_env(&env),
            solo_lease_dir: maw_state_path(&env, &["lease"]),
        }
    }
}

#[derive(Default)]
struct DoneLocal { runner: maw_tmux::CommandTmuxRunner }

trait DoneRuntime {
    fn done_list_windows(&mut self) -> Vec<DoneWindow>;
    fn done_list_panes(&mut self) -> Vec<DonePane>;
    fn done_current_identity(&mut self) -> Option<(String, i32)>;
    fn done_current_pane(&mut self) -> Option<String>;
    fn done_pane_info(&mut self, target: &str) -> Option<(String, String)>;
    fn done_reap_target(&mut self, target: &str) -> Result<(), String>;
    fn done_reap_pane(&mut self, pane_id: &str) -> Result<(), String>;
    fn done_tmux(&mut self, command: &str, args: &[String]) -> Result<String, String>;
    fn done_send_text(&mut self, target: &str, text: &str) -> Result<(), String>;
    fn done_git(&mut self, args: &[String]) -> Result<String, String>;
    fn done_pr_state(&mut self, main_path: &std::path::Path, branch: &str) -> Result<PrGithubState, String>;
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
            "--keep-branch" => options.keep_branch = true,
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
    if options.clean_branch && options.keep_branch {
        return Err("done: --keep-branch cannot be used with --clean-branch".to_owned());
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
    let panes = local.done_list_panes();
    let target_lower = target.to_lowercase();
    let matched = done_find_window(&sessions, &target_lower, session_filter);
    let matched_pane = done_find_pane(&panes, &target_lower, session_filter)?;
    if let Some(window) = &matched { done_assert_may_target_lead(window, &sessions, options.force, local, &mut stdout)?; }
    if let Some(pane) = &matched_pane { done_assert_not_invoking_pane(pane, local, &mut stdout)?; }
    let pane_info = matched
        .as_ref()
        .and_then(|window| done_live_pane_info(window, local))
        .or_else(|| matched_pane.as_ref().map(done_pane_info));
    let solo_worktree = matched
        .as_ref()
        .and_then(|window| solo_worktree_for_holder_in_dir(&done_tmux_target(window), &context.solo_lease_dir));
    let selected_worktree = if let Some(path) = solo_worktree {
        done_resolve_registered_worktree(local, &path, context)?
    } else {
        done_select_worktree(target, &target_lower, options, pane_info.as_ref(), local, context, &mut stdout)?
    };
    if let Some(pane) = &matched_pane { done_assert_may_target_pane(pane, selected_worktree.as_ref(), &mut stdout)?; }
    if let Some(worktree) = &selected_worktree {
        if !options.dry_run || worktree.full_path.is_dir() {
            done_rescue_psi_notes(worktree, options.dry_run, &mut stdout);
        }
    }
    if let Some(window) = &matched {
        if !options.force {
            done_auto_save(window, options, local, pane_info.as_ref(), selected_worktree.as_ref(), &mut stdout);
        }
    } else if options.dry_run {
        let _ = writeln!(stdout, "  \x1b[36m⬡\x1b[0m [dry-run] window '{target}' not running — nothing to auto-save");
    }
    let worktree_pane_id = selected_worktree
        .as_ref()
        .and_then(done_recorded_worktree_pane_id);
    let removed_worktree = if let Some(worktree) = &selected_worktree {
        done_remove_selected_worktree(
            worktree,
            worktree_pane_id.as_deref(),
            options,
            local,
            &mut stdout,
        )?;
        if !options.dry_run {
            done_sweep_repo_after_removal(&worktree.main_path, &panes, options, local, &mut stdout);
        }
        true
    } else {
        false
    };
    if let Some(window) = &matched {
        done_kill_window(window, options, local, &mut stdout);
    } else if let Some(worktree) = &selected_worktree {
        done_kill_worktree_pane(
            worktree,
            worktree_pane_id.as_deref(),
            options,
            local,
            &mut stdout,
        )?;
    } else {
        let _ = writeln!(stdout, "  \x1b[90m○\x1b[0m window '{target}' not running");
    }
    if !options.dry_run {
        if let Some(window) = &matched { solo_release_holder(&done_tmux_target(window)); }
    }
    if !removed_worktree { stdout.push_str("  \x1b[90m○\x1b[0m no worktree to remove (may be a main window)\n"); }
    let config_target = done_config_target(&target_lower, matched.as_ref(), selected_worktree.as_ref());
    if options.dry_run {
        if matched.is_none() && !removed_worktree { done_fail_missing_target(target, &panes, context)?; }
        let _ = writeln!(stdout, "  \x1b[36m⬡\x1b[0m [dry-run] would remove '{config_target}' from fleet config if present\n");
        return Ok(stdout);
    }
    let removed_config = done_remove_from_fleet_config(&config_target, context, &mut stdout);
    if !removed_config { stdout.push_str("  \x1b[90m○\x1b[0m not in any fleet config\n"); }
    if matched.is_none() && !removed_worktree && !removed_config { done_fail_missing_target(target, &panes, context)?; }
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

    fn done_list_panes(&mut self) -> Vec<DonePane> {
        let args = ["-a".to_owned(), "-F".to_owned(), "#{session_name}|||#{window_index}|||#{window_name}|||#{pane_index}|||#{pane_id}|||#{pane_active}|||#{pane_current_command}|||#{pane_current_path}".to_owned()];
        let Ok(raw) = maw_tmux::TmuxRunner::run(&mut self.runner, "list-panes", &args) else { return Vec::new(); };
        raw.lines().filter_map(done_parse_pane_line).collect()
    }

    fn done_current_identity(&mut self) -> Option<(String, i32)> {
        done_invoking_pane_identity(&mut self.runner)
    }

    fn done_current_pane(&mut self) -> Option<String> { crate::wind::team::caller_pane() }

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

    fn done_reap_pane(&mut self, pane_id: &str) -> Result<(), String> {
        done_validate_tmux_target(pane_id)?;
        reap_tmux_pane(&mut self.runner, pane_id)
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

    fn done_pr_state(&mut self, main_path: &std::path::Path, branch: &str) -> Result<PrGithubState, String> {
        done_pr_state_for_branch(main_path, branch)
    }
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

fn done_parse_pane_line(line: &str) -> Option<DonePane> {
    let mut parts = line.split("|||");
    let session = parts.next()?.to_owned();
    let window_index = parts.next()?.parse::<i32>().ok()?;
    let window_name = parts.next()?.to_owned();
    let pane_index = parts.next()?.parse::<i32>().ok()?;
    let pane_id = parts.next()?.to_owned();
    let active = parts.next()? == "1";
    let command = parts.next()?.to_owned();
    let cwd = parts.next().map(str::trim).filter(|value| !value.is_empty()).map(ToOwned::to_owned);
    (!session.is_empty() && !window_name.is_empty() && pane_id.starts_with('%')).then_some(DonePane {
        session,
        window_index,
        window_name,
        pane_index,
        pane_id,
        active,
        command,
        cwd,
    })
}

fn done_find_window(windows: &[DoneWindow], target_lower: &str, session_filter: Option<&str>) -> Option<DoneWindow> {
    windows.iter().find(|window| session_filter.is_none_or(|session| session == window.session) && window.name.eq_ignore_ascii_case(target_lower)).cloned()
}

fn done_find_pane(panes: &[DonePane], target_lower: &str, session_filter: Option<&str>) -> Result<Option<DonePane>, String> {
    let matches = panes
        .iter()
        .filter(|pane| {
            session_filter.is_none_or(|session| session == pane.session)
                && done_pane_targets(pane).iter().any(|candidate| candidate.eq_ignore_ascii_case(target_lower))
        })
        .cloned()
        .collect::<Vec<_>>();
    match matches.len() {
        0 => Ok(None),
        1 => Ok(matches.into_iter().next()),
        _ => Err(format!(
            "done: target '{target_lower}' is ambiguous; matches panes:\n{}",
            matches.iter().map(done_pane_target).collect::<Vec<_>>().join("\n")
        )),
    }
}

fn done_pane_targets(pane: &DonePane) -> [String; 3] {
    [
        pane.pane_id.clone(),
        done_pane_target(pane),
        format!("{}:{}.{}", pane.session, pane.window_name, pane.pane_index),
    ]
}

fn done_pane_target(pane: &DonePane) -> String { format!("{}:{}.{}", pane.session, pane.window_index, pane.pane_index) }

fn done_assert_may_target_lead(window: &DoneWindow, windows: &[DoneWindow], force: bool, local: &mut impl DoneRuntime, stdout: &mut String) -> Result<(), String> {
    let current = local.done_current_identity();
    if let Some(message) = crate::wind::done::self_invocation_message(current.as_ref(), &window.session, window.index, &window.name) {
        let _ = writeln!(stdout, "  \x1b[31m✗\x1b[0m {message}");
        stdout.push_str("  \x1b[90m  run maw done from the lead/parent pane after the DONE ping\x1b[0m\n");
        return Err(message);
    }
    let Some(lead) = done_lead_window(windows, &window.session) else {
        if force { return Ok(()); }
        let message = format!("refusing to done window '{}' because the lead window for session '{}' could not be identified; retry with --force to retire the orphaned delivery", window.name, window.session);
        let _ = writeln!(stdout, "  \x1b[31m✗\x1b[0m {message}");
        return Err(message);
    };
    if lead.index != window.index { return Ok(()); }
    let message = format!("refusing to done lead window '{}' in session '{}' from a non-lead context", window.name, window.session);
    let _ = writeln!(stdout, "  \x1b[31m✗\x1b[0m {message}");
    stdout.push_str("  \x1b[90m  run from the lead window, or target a non-lead agent window\x1b[0m\n");
    Err(message)
}

fn done_assert_not_invoking_pane(pane: &DonePane, local: &mut impl DoneRuntime, stdout: &mut String) -> Result<(), String> {
    if local.done_current_pane().is_some_and(|current| current == pane.pane_id) {
        let message = format!("refusing to done invoking pane '{}'", pane.pane_id);
        let _ = writeln!(stdout, "  \x1b[31m✗\x1b[0m {message}");
        return Err(message);
    }
    Ok(())
}

fn done_assert_may_target_pane(pane: &DonePane, worktree: Option<&DoneWorktree>, stdout: &mut String) -> Result<(), String> {
    if worktree.and_then(done_recorded_worktree_pane_id).as_deref() == Some(pane.pane_id.as_str()) { return Ok(()); }
    let message = format!("refusing to done pane '{}': it is not a recorded worktree L2", pane.pane_id);
    let _ = writeln!(stdout, "  \x1b[31m✗\x1b[0m {message}");
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

fn done_pane_info(pane: &DonePane) -> DonePaneInfo {
    DonePaneInfo { command: pane.command.clone(), cwd: pane.cwd.clone().unwrap_or_default() }
}

fn done_config_target(target: &str, matched_window: Option<&DoneWindow>, worktree: Option<&DoneWorktree>) -> String {
    if matched_window.is_some() { return target.to_owned(); }
    worktree.and_then(done_worktree_slug).unwrap_or(target).to_owned()
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

fn done_kill_worktree_pane(
    worktree: &DoneWorktree,
    pane_id: Option<&str>,
    options: &DoneOptions,
    local: &mut impl DoneRuntime,
    stdout: &mut String,
) -> Result<(), String> {
    let Some(pane_id) = pane_id.map(str::to_owned) else {
        let _ = writeln!(stdout, "  \x1b[90m○\x1b[0m window '{}' not running", worktree.label);
        return Ok(());
    };
    done_validate_tmux_target(&pane_id)?;
    if options.dry_run {
        let _ = writeln!(stdout, "  \x1b[36m⬡\x1b[0m [dry-run] would kill split pane {pane_id}");
        return Ok(());
    }
    let details = local.done_tmux(
        "display-message",
        &[
            "-t".to_owned(),
            pane_id.clone(),
            "-p".to_owned(),
            "#{window_panes}\t#{pane_current_path}\t#{window_id}".to_owned(),
        ],
    );
    let Ok(details) = details else {
        let _ = writeln!(stdout, "  \x1b[33m⚠\x1b[0m split pane {pane_id} already gone; continuing cleanup");
        return Ok(());
    };
    let mut fields = details.trim_end().split('\t');
    let Some(count) = fields.next() else {
        let _ = writeln!(stdout, "  \x1b[33m⚠\x1b[0m split pane {pane_id} already gone; continuing cleanup");
        return Ok(());
    };
    let Some(_cwd) = fields.next() else {
        let _ = writeln!(stdout, "  \x1b[33m⚠\x1b[0m split pane {pane_id} already gone; continuing cleanup");
        return Ok(());
    };
    let Some(window) = fields.next() else {
        let _ = writeln!(stdout, "  \x1b[33m⚠\x1b[0m split pane {pane_id} already gone; continuing cleanup");
        return Ok(());
    };
    let count = count.parse::<u32>().map_err(|_| format!("done: invalid pane count for {pane_id}"))?;
    if count <= 1 {
        return Err(format!("done: refusing to kill sole pane {pane_id}; it no longer has a split-pane parent"));
    }
    // The marker identifies the pane from the resolved worktree before removal.
    // Pane targets were also verified by `done_assert_may_target_pane`; tmux
    // marks the cwd as " (deleted)" after removal, so it cannot prove ownership here.
    local.done_reap_pane(&pane_id)?;
    local.done_tmux("kill-pane", &["-t".to_owned(), pane_id.clone()])?;
    if let Err(error) = done_rebalance_workon_window(local, window) {
        let _ = writeln!(stdout, "  \x1b[33m⚠\x1b[0m could not rebalance workon panes: {error}");
    }
    let _ = writeln!(stdout, "  \x1b[32m✓\x1b[0m killed split pane {pane_id}");
    Ok(())
}

fn done_recorded_worktree_pane_id(worktree: &DoneWorktree) -> Option<String> {
    std::fs::read_to_string(worktree.full_path.join(".maw/pane-id")).ok().map(|pane_id| pane_id.trim().to_owned())
}

fn done_rebalance_workon_window(
    local: &mut impl DoneRuntime,
    window: &str,
) -> Result<(), String> {
    done_validate_tmux_target(window)?;
    let marker = local
        .done_tmux(
            "show-window-options",
            &[
                "-q".to_owned(),
                "-v".to_owned(),
                "-t".to_owned(),
                window.to_owned(),
                WORKON_LAYOUT_MARKER.to_owned(),
            ],
        )
        .unwrap_or_default();
    if marker.trim() != WORKON_LAYOUT_PRESET {
        return Ok(());
    }
    local.done_tmux(
        "select-layout",
        &[
            "-t".to_owned(),
            window.to_owned(),
            WORKON_LAYOUT_PRESET.to_owned(),
        ],
    )?;
    Ok(())
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
    done_worktree_by_scan(target, &context.repos_root, stdout)
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

fn done_worktree_by_scan(target: &str, repos_root: &std::path::Path, stdout: &mut String) -> Result<Option<DoneWorktree>, String> {
    let matches = done_find_worktree_paths(target, repos_root);
    if matches.len() > 1 {
        let candidates = matches.iter().map(|worktree| format!("  {}", worktree.label)).collect::<Vec<_>>().join("\n");
        let message = format!("done: target '{target}' is ambiguous; matches worktrees:\n{candidates}");
        let _ = writeln!(stdout, "  \x1b[31m✗\x1b[0m {message}");
        return Err(message);
    }
    Ok(matches.into_iter().next())
}

fn done_rescue_psi_notes(worktree: &DoneWorktree, dry_run: bool, stdout: &mut String) {
    // Copy uncommitted ψ/ brain notes out of the worktree into the owning main
    // checkout BEFORE auto-save sweeps them into a branch that --clean-branch may
    // force-delete (git branch -D) before the PR merges — losing the notes to GC.
    // Never overwrites existing files; best-effort (rescue failure must not block
    // the rest of `done`).
    let rescue = if dry_run {
        crate::wind::done::preview_rescue_psi(&worktree.full_path, &worktree.main_path)
    } else {
        crate::wind::done::rescue_psi(&worktree.full_path, &worktree.main_path)
    };
    match rescue {
        Ok(rescued) if !rescued.is_empty() => {
            let action = if dry_run { "[dry-run] would rescue" } else { "rescued" };
            let _ = writeln!(stdout, "  \x1b[32m✓\x1b[0m {action} {} uncommitted ψ note(s) to main before removal", rescued.len());
        }
        Ok(_) if done_psi_has_entries(worktree) => {
            let prefix = if dry_run { "[dry-run] " } else { "" };
            let _ = writeln!(stdout, "  \x1b[33m⚠\x1b[0m {prefix}ψ rescue found no uncommitted notes although ψ/ is non-empty");
        }
        Ok(_) => {}
        Err(error) => {
            let _ = writeln!(stdout, "  \x1b[33m⚠\x1b[0m ψ rescue skipped: {error}");
        }
    }
}

fn done_psi_has_entries(worktree: &DoneWorktree) -> bool {
    std::fs::read_dir(worktree.full_path.join("ψ")).is_ok_and(|mut entries| entries.next().is_some())
}

fn done_remove_selected_worktree(
    worktree: &DoneWorktree,
    recorded_pane_id: Option<&str>,
    options: &DoneOptions,
    local: &mut impl DoneRuntime,
    stdout: &mut String,
) -> Result<(), String> {
    if options.dry_run { let _ = writeln!(stdout, "  \x1b[36m⬡\x1b[0m [dry-run] would remove worktree {}", worktree.label); return Ok(()); }
    done_remove_worktree(worktree, recorded_pane_id, options, local, stdout)
}

fn done_remove_worktree(
    worktree: &DoneWorktree,
    recorded_pane_id: Option<&str>,
    options: &DoneOptions,
    local: &mut impl DoneRuntime,
    stdout: &mut String,
) -> Result<(), String> {
    done_validate_exec_path(&worktree.main_path)?;
    done_validate_exec_path(&worktree.full_path)?;
    let cargo_target_dir = done_managed_cargo_target_dir(&worktree.full_path);
    let branch = local.done_git(&["-C".to_owned(), worktree.full_path.display().to_string(), "rev-parse".to_owned(), "--abbrev-ref".to_owned(), "HEAD".to_owned()]).unwrap_or_default().trim().to_owned();
    let cleaned = crate::wind::workon::remove_ephemeral_markers(&worktree.full_path)
        .map_err(|error| format!("done: clean maw markers: {error}"))?;
    let mut remove_args = vec!["-C".to_owned(), worktree.main_path.display().to_string(), "worktree".to_owned(), "remove".to_owned()];
    if options.force {
        remove_args.push("--force".to_owned());
    }
    remove_args.extend(["--".to_owned(), worktree.full_path.display().to_string()]);
    if let Err(error) = local.done_git(&remove_args) {
        if cleaned.iter().any(|marker| marker == ".maw/pane-id") {
            if let Some(pane_id) = recorded_pane_id {
                if let Err(restore_error) = done_restore_recorded_pane_id(worktree, pane_id) {
                    return Err(format!("{error}; additionally failed to restore pane ownership proof: {restore_error}"));
                }
            }
        }
        return Err(error);
    }
    if let Some(target_dir) = cargo_target_dir {
        done_reclaim_cargo_target_dir(&target_dir, stdout);
    }
    local.done_git(&["-C".to_owned(), worktree.main_path.display().to_string(), "worktree".to_owned(), "prune".to_owned()])?;
    let _ = writeln!(stdout, "  \x1b[32m✓\x1b[0m removed worktree {}", worktree.label);
    done_cleanup_branch(&worktree.main_path, &branch, options, local, stdout);
    Ok(())
}

fn done_restore_recorded_pane_id(worktree: &DoneWorktree, pane_id: &str) -> Result<(), String> {
    let marker = worktree.full_path.join(".maw/pane-id");
    let Some(parent) = marker.parent() else {
        return Err(format!("done: invalid pane marker path {}", marker.display()));
    };
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("done: restore pane marker directory {}: {error}", parent.display()))?;
    std::fs::write(&marker, format!("{pane_id}\n"))
        .map_err(|error| format!("done: restore pane marker {}: {error}", marker.display()))
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
    if options.keep_branch {
        let _ = writeln!(stdout, "  \x1b[90m○\x1b[0m branch {branch} retained (--keep-branch)");
        return;
    }
    if options.clean_branch {
        done_delete_branch(main_path, branch, local, stdout, "requested by --clean-branch", false);
        return;
    }
    match local.done_pr_state(main_path, branch) {
        Ok(PrGithubState::Merged) => {
            done_delete_branch(main_path, branch, local, stdout, "merged PR", true);
        }
        Ok(PrGithubState::Open) => {
            let _ = writeln!(stdout, "  \x1b[90m○\x1b[0m branch {branch} retained (PR open)");
        }
        Ok(PrGithubState::Closed) => {
            let _ = writeln!(stdout, "  \x1b[90m○\x1b[0m branch {branch} retained (PR closed without merge)");
        }
        Err(error) => {
            let _ = writeln!(stdout, "  \x1b[33m⚠\x1b[0m branch {branch} retained (PR state unavailable: {error})");
        }
    }
}

fn done_delete_branch(main_path: &std::path::Path, branch: &str, local: &mut impl DoneRuntime, stdout: &mut String, reason: &str, delete_remote: bool) -> bool {
    let args = ["-C".to_owned(), main_path.display().to_string(), "branch".to_owned(), "-D".to_owned(), "--".to_owned(), branch.to_owned()];
    match local.done_git(&args) {
        Ok(_) => {
            if delete_remote && done_branch_name_allows_push(branch) {
                let args = ["-C".to_owned(), main_path.display().to_string(), "push".to_owned(), "origin".to_owned(), "--delete".to_owned(), branch.to_owned()];
                match local.done_git(&args) {
                    Ok(_) => {
                        let _ = writeln!(stdout, "  \x1b[32m✓\x1b[0m deleted branch {branch} local+remote ({reason})");
                    }
                    Err(error) => {
                        let _ = writeln!(stdout, "  \x1b[32m✓\x1b[0m deleted branch {branch} local ({reason}); remote retained: {error}");
                    }
                }
            } else {
                let _ = writeln!(stdout, "  \x1b[32m✓\x1b[0m deleted branch {branch} local ({reason})");
            }
            true
        }
        Err(error) => {
            let _ = writeln!(stdout, "  \x1b[33m⚠\x1b[0m branch {branch} retained (delete failed after {reason}: {error})");
            false
        }
    }
}

fn done_find_worktree_paths(target: &str, repos_root: &std::path::Path) -> Vec<DoneWorktree> {
    let target_lower = target.to_lowercase();
    done_collect_worktree_paths(repos_root)
        .into_iter()
        .filter(|worktree| done_worktree_aliases(worktree).iter().any(|alias| alias.eq_ignore_ascii_case(&target_lower)))
        .collect()
}

fn done_collect_worktree_paths(repos_root: &std::path::Path) -> Vec<DoneWorktree> {
    let mut out = Vec::new();
    let Ok(orgs) = std::fs::read_dir(repos_root) else { return out; };
    for org in orgs.flatten().filter(|entry| entry.path().is_dir()) {
        let Ok(repos) = std::fs::read_dir(org.path()) else { continue; };
        for repo in repos.flatten().filter(|entry| entry.path().is_dir()) { done_collect_repo_worktrees(&repo.path(), repos_root, &mut out); }
    }
    out.sort_by(|a, b| a.full_path.cmp(&b.full_path));
    out
}

fn done_collect_repo_worktrees(repo_path: &std::path::Path, repos_root: &std::path::Path, out: &mut Vec<DoneWorktree>) {
    let Some(name) = repo_path.file_name().and_then(std::ffi::OsStr::to_str) else { return; };
    if name.contains(".wt-") { if let Some(worktree) = done_parse_worktree_path(repo_path, repos_root) { out.push(worktree); } }
    let agents = repo_path.join("agents");
    let Ok(entries) = std::fs::read_dir(agents) else { return; };
    for entry in entries.flatten().filter(|entry| entry.path().is_dir()) {
        if let Some(worktree) = done_parse_worktree_path(&entry.path(), repos_root) { out.push(worktree); }
    }
}

fn done_worktree_aliases(worktree: &DoneWorktree) -> Vec<String> {
    let Some(slug) = done_worktree_slug(worktree) else { return Vec::new(); };
    let mut aliases = vec![slug.to_owned()];
    if let Some(repo) = worktree.main_path.file_name().and_then(std::ffi::OsStr::to_str) {
        aliases.push(format!("{repo}-{slug}"));
    }
    aliases
}

fn done_worktree_slug(worktree: &DoneWorktree) -> Option<&str> {
    let name = worktree.full_path.file_name()?.to_str()?;
    if worktree.full_path.parent()?.file_name()?.to_str() == Some("agents") { return Some(name); }
    name.split_once(".wt-").map(|(_, slug)| slug)
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

fn done_fail_missing_target(target: &str, panes: &[DonePane], context: &DoneContext) -> Result<(), String> {
    let mut candidates = panes.iter().flat_map(done_pane_targets).collect::<Vec<_>>();
    candidates.extend(done_collect_worktree_paths(&context.repos_root).iter().flat_map(done_worktree_aliases));
    candidates.sort();
    candidates.dedup();
    let hint = if target.eq_ignore_ascii_case("all") {
        "\n  did you mean `maw done --all`?".to_owned()
    } else {
        done_nearest_target(target, &candidates).map_or_else(String::new, |candidate| format!("\n  did you mean '{candidate}'?"))
    };
    Err(format!(
        "no done target matched '{target}'{hint}\n  accepted forms: <slug>, <repo>-<slug>, <pane-id> (for example %26), <session>:<window>.<pane>"
    ))
}

fn done_nearest_target(target: &str, candidates: &[String]) -> Option<String> {
    let (distance, candidate) = candidates
        .iter()
        .map(|candidate| (done_edit_distance(target, candidate), candidate))
        .min_by_key(|(distance, candidate)| (*distance, candidate.len()))?;
    (distance <= (target.chars().count() / 5).max(2)).then(|| candidate.clone())
}

fn done_edit_distance(left: &str, right: &str) -> usize {
    let right = right.chars().collect::<Vec<_>>();
    let mut previous = (0..=right.len()).collect::<Vec<_>>();
    for (left_index, left_char) in left.chars().enumerate() {
        let mut current = vec![left_index + 1];
        for (right_index, right_char) in right.iter().enumerate() {
            let replace = previous[right_index] + usize::from(left_char != *right_char);
            current.push((current[right_index] + 1).min(previous[right_index + 1] + 1).min(replace));
        }
        previous = current;
    }
    previous[right.len()]
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
        panes: Vec<DonePane>,
        current: Option<(String, i32)>,
        current_pane: Option<String>,
        pane_info: std::collections::BTreeMap<String, (String, String)>,
        top_levels: std::collections::BTreeMap<std::path::PathBuf, std::path::PathBuf>,
        registered: std::collections::BTreeMap<std::path::PathBuf, Vec<std::path::PathBuf>>,
        branches: std::collections::BTreeMap<std::path::PathBuf, String>,
        local_branches: std::collections::BTreeMap<std::path::PathBuf, std::collections::BTreeSet<String>>,
        pr_states: std::collections::BTreeMap<(std::path::PathBuf, String), Result<PrGithubState, String>>,
        dirty_removals: std::collections::BTreeSet<std::path::PathBuf>,
        git_errors: std::collections::BTreeMap<Vec<String>, String>,
        tmux_responses: std::collections::BTreeMap<String, String>,
        git_calls: Vec<Vec<String>>,
        tmux_calls: Vec<(String, Vec<String>)>,
        reaped_targets: Vec<String>,
        actions: Vec<&'static str>,
        sent_text: Vec<(String, String)>,
    }

    impl DoneFakeRuntime {
        fn register_worktree(&mut self, main: &std::path::Path, worktree: &std::path::Path) {
            self.register_worktree_branch(main, worktree, "agent/task");
        }

        fn register_worktree_branch(&mut self, main: &std::path::Path, worktree: &std::path::Path, branch: &str) {
            self.top_levels.insert(worktree.to_path_buf(), worktree.to_path_buf());
            self.registered.entry(main.to_path_buf()).or_default().push(worktree.to_path_buf());
            self.branches.insert(worktree.to_path_buf(), branch.to_owned());
            self.local_branches.entry(main.to_path_buf()).or_default().insert(branch.to_owned());
        }

        fn set_pr_state(&mut self, main: &std::path::Path, branch: &str, state: PrGithubState) {
            self.pr_states.insert((main.to_path_buf(), branch.to_owned()), Ok(state));
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

        fn done_list_panes(&mut self) -> Vec<DonePane> { self.panes.clone() }

        fn done_current_identity(&mut self) -> Option<(String, i32)> { self.current.clone() }

        fn done_current_pane(&mut self) -> Option<String> { self.current_pane.clone() }

        fn done_pane_info(&mut self, target: &str) -> Option<(String, String)> { self.pane_info.get(target).cloned() }

        fn done_reap_target(&mut self, target: &str) -> Result<(), String> {
            self.reaped_targets.push(target.to_owned());
            Ok(())
        }

        fn done_reap_pane(&mut self, _pane_id: &str) -> Result<(), String> { Ok(()) }

        fn done_tmux(&mut self, command: &str, args: &[String]) -> Result<String, String> {
            self.tmux_calls.push((command.to_owned(), args.to_vec()));
            if command == "kill-pane" {
                self.actions.push("kill-pane");
            }
            Ok(self.tmux_responses.get(command).cloned().unwrap_or_default())
        }

        fn done_send_text(&mut self, target: &str, text: &str) -> Result<(), String> {
            self.sent_text.push((target.to_owned(), text.to_owned()));
            Ok(())
        }

        fn done_git(&mut self, args: &[String]) -> Result<String, String> {
            self.git_calls.push(args.to_vec());
            if let Some(error) = self.git_errors.get(args) {
                return Err(error.clone());
            }
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
                    let mut out = format!("worktree {}\nbranch refs/heads/main\n\n", main.display());
                    for worktree in worktrees {
                        if worktree != main {
                            let branch = self.branches.get(worktree).map_or("agent/task", String::as_str);
                            let _ = write!(out, "worktree {}\nbranch refs/heads/{branch}\n\n", worktree.display());
                        }
                    }
                    out
                } else {
                    format!("worktree {}\nbranch refs/heads/main\n\n", cwd.display())
                };
                return Ok(out);
            }
            if args.iter().any(|arg| arg == "branch") && args.iter().any(|arg| arg == "--format=%(refname:short)") {
                let branches = self.local_branches.get(&cwd).into_iter().flatten().filter(|branch| branch.starts_with("agents/")).cloned().collect::<Vec<_>>();
                return Ok(if branches.is_empty() { String::new() } else { format!("{}\n", branches.join("\n")) });
            }
            if args.ends_with(&["rev-parse".to_owned(), "--abbrev-ref".to_owned(), "HEAD".to_owned()]) {
                return Ok(format!("{}\n", self.branches.get(&cwd).map_or("agent/task", String::as_str)));
            }
            if args.iter().any(|arg| arg == "remove") {
                self.actions.push("remove-worktree");
                let worktree = Self::arg_after_separator(args).ok_or_else(|| "missing worktree path".to_owned())?;
                if self.dirty_removals.contains(&worktree) && !args.iter().any(|arg| arg == "--force") {
                    return Err(format!("fatal: '{}' contains modified or untracked files", worktree.display()));
                }
                for worktrees in self.registered.values_mut() {
                    worktrees.retain(|registered| registered != &worktree);
                }
                self.branches.remove(&worktree);
                self.top_levels.remove(&worktree);
                return Ok(String::new());
            }
            if args.iter().any(|arg| arg == "branch") && args.iter().any(|arg| arg == "-D") {
                if let Some(branch) = Self::arg_after_separator(args) {
                    if let Some(branches) = self.local_branches.get_mut(&cwd) {
                        branches.remove(&branch.display().to_string());
                    }
                }
                return Ok(String::new());
            }
            Ok(String::new())
        }

        fn done_pr_state(&mut self, main_path: &std::path::Path, branch: &str) -> Result<PrGithubState, String> {
            self.pr_states
                .get(&(main_path.to_path_buf(), branch.to_owned()))
                .cloned()
                .unwrap_or_else(|| Err("gh unavailable".to_owned()))
        }
    }

    struct DoneRealGitRuntime {
        git: std::path::PathBuf,
        tmux_responses: std::collections::BTreeMap<String, String>,
        tmux_calls: Vec<(String, Vec<String>)>,
    }

    impl Default for DoneRealGitRuntime {
        fn default() -> Self {
            Self {
                git: done_git_executable(),
                tmux_responses: std::collections::BTreeMap::new(),
                tmux_calls: Vec::new(),
            }
        }
    }

    impl DoneRuntime for DoneRealGitRuntime {
        fn done_list_windows(&mut self) -> Vec<DoneWindow> { Vec::new() }

        fn done_list_panes(&mut self) -> Vec<DonePane> { Vec::new() }

        fn done_current_identity(&mut self) -> Option<(String, i32)> { None }

        fn done_current_pane(&mut self) -> Option<String> { None }

        fn done_pane_info(&mut self, _target: &str) -> Option<(String, String)> { None }

        fn done_reap_target(&mut self, _target: &str) -> Result<(), String> { Ok(()) }

        fn done_reap_pane(&mut self, _pane_id: &str) -> Result<(), String> { Ok(()) }

        fn done_tmux(&mut self, command: &str, args: &[String]) -> Result<String, String> {
            self.tmux_calls.push((command.to_owned(), args.to_vec()));
            Ok(self.tmux_responses.get(command).cloned().unwrap_or_default())
        }

        fn done_send_text(&mut self, _target: &str, _text: &str) -> Result<(), String> { Err("tmux unavailable in real-git test runtime".to_owned()) }

        fn done_git(&mut self, args: &[String]) -> Result<String, String> {
            let output = std::process::Command::new(&self.git).args(args).output().map_err(|error| format!("git failed: {error}"))?;
            if output.status.success() { Ok(String::from_utf8_lossy(&output.stdout).to_string()) } else { Err(String::from_utf8_lossy(&output.stderr).trim().to_owned()) }
        }

        fn done_pr_state(&mut self, main_path: &std::path::Path, branch: &str) -> Result<PrGithubState, String> {
            done_pr_state_for_branch(main_path, branch)
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
            DoneContext {
                repos_root: self.repos_root(),
                fleet_dirs: vec![self.fleet_dir()],
                solo_lease_dir: self.path.join("state/lease"),
            }
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

    fn done_test_pane(id: &str, index: i32, command: &str, cwd: &std::path::Path) -> DonePane {
        DonePane {
            session: "s".to_owned(),
            window_index: 1,
            window_name: "s".to_owned(),
            pane_index: index,
            pane_id: id.to_owned(),
            active: index == 0,
            command: command.to_owned(),
            cwd: Some(cwd.display().to_string()),
        }
    }

    fn done_write_fleet(root: &DoneTempRoot, window: &str, repo: &str) {
        let fleet_dir = root.fleet_dir();
        std::fs::create_dir_all(&fleet_dir).expect("fleet dir");
        std::fs::write(fleet_dir.join("s.json"), format!(r#"{{"name":"s","windows":[{{"name":"{window}","repo":"{repo}"}}]}}"#)).expect("fleet");
    }

    fn done_args(args: &[&str]) -> Vec<String> { args.iter().map(|arg| (*arg).to_owned()).collect() }

    fn done_init_unignored_worktree(root: &DoneTempRoot, label: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let main = root.repos_root().join("acme/app");
        let worktree = main.join("agents").join(label);
        std::fs::create_dir_all(main.join("agents")).expect("worktree parent");
        done_run_process("git", &["init"], Some(&main));
        std::fs::write(main.join("README.md"), "fixture\n").expect("seed fixture");
        done_run_process("git", &["add", "README.md"], Some(&main));
        done_run_process(
            "git",
            &[
                "-c",
                "user.name=maw-test",
                "-c",
                "user.email=maw-test@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-m",
                "seed done marker fixture",
            ],
            Some(&main),
        );
        let branch = format!("agents/{label}");
        let worktree_arg = worktree.display().to_string();
        done_run_process("git", &["worktree", "add", "-b", &branch, &worktree_arg], Some(&main));
        (main, worktree)
    }

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
    fn done_workon_name_and_slug_resolve_the_same_split_pane_worktree() {
        let root = DoneTempRoot::new("workon-name");
        let context = root.context();
        let worktree = context.repos_root.join("acme/app/agents/issue-147");
        std::fs::create_dir_all(worktree.join(".maw")).expect("marker dir");
        std::fs::write(worktree.join(".maw/pane-id"), "%42\n").expect("pane marker");

        for target in ["issue-147", "app-issue-147"] {
            let output = done_run_with_context(&done_args(&[target, "--dry-run"]), &mut DoneFakeRuntime::default(), &context)
                .expect("accepted worktree target");
            assert!(output.contains("would kill split pane %42"), "{output}");
            assert!(output.contains("would remove worktree acme/app/agents/issue-147"), "{output}");
            assert!(output.contains("would remove 'issue-147' from fleet config"), "{output}");
        }
    }

    #[test]
    fn done_pane_discovery_finds_a_split_l2_behind_the_active_oracle() {
        let root = DoneTempRoot::new("pane-discovery");
        let context = root.context();
        let main = context.repos_root.join("acme/app");
        let worktree = main.join("agents/issue-147");
        std::fs::create_dir_all(worktree.join(".maw")).expect("marker dir");
        std::fs::write(worktree.join(".maw/pane-id"), "%42\n").expect("pane marker");
        let raw = format!(
            "s|||1|||s|||0|||%25|||1|||claude|||{}\ns|||1|||s|||1|||%42|||0|||codex|||{}\n",
            main.display(), worktree.display()
        );
        let panes = raw.lines().filter_map(done_parse_pane_line).collect::<Vec<_>>();
        assert!(panes[0].active && panes[1].cwd.as_deref() == Some(worktree.to_str().expect("utf8 path")));

        for target in ["%42", "s:1.1"] {
            let mut runtime = DoneFakeRuntime { panes: panes.clone(), ..DoneFakeRuntime::default() };
            runtime.register_worktree(&main, &worktree);
            let output = done_run_with_context(&done_args(&[target, "--dry-run"]), &mut runtime, &context)
                .expect("accepted pane target");
            assert!(output.contains("would kill split pane %42"), "{output}");
            assert!(output.contains("would remove worktree acme/app/agents/issue-147"), "{output}");
        }
    }

    #[test]
    fn done_allows_a_claude_worktree_pane_and_refuses_a_claude_lead_pane() {
        let root = DoneTempRoot::new("claude-pane-ownership");
        let context = root.context();
        let main = context.repos_root.join("acme/app");
        let worktree = main.join("agents/issue-147");
        std::fs::create_dir_all(worktree.join(".maw")).expect("marker dir");
        std::fs::write(worktree.join(".maw/pane-id"), "%42\n").expect("pane marker");

        let mut runtime = DoneFakeRuntime {
            panes: vec![
                done_test_pane("%25", 0, "claude", &main),
                done_test_pane("%42", 1, "claude", &worktree),
            ],
            ..DoneFakeRuntime::default()
        };
        runtime.register_worktree(&main, &worktree);

        let output = done_run_with_context(&done_args(&["%42", "--dry-run"]), &mut runtime, &context)
            .expect("recorded Claude L2 pane must be accepted");
        assert!(output.contains("would kill split pane %42"), "{output}");

        let error = done_run_with_context(&done_args(&["%25", "--dry-run"]), &mut runtime, &context)
            .expect_err("Claude lead pane without a worktree marker must be refused");
        assert!(error.contains("not a recorded worktree L2"), "{error}");
    }

    #[test]
    fn done_refuses_ambiguous_or_self_pane_targets_and_suggests_near_matches() {
        let root = DoneTempRoot::new("done-target-guards");
        let context = root.context();
        for repo in ["acme/app", "org/app"] {
            std::fs::create_dir_all(context.repos_root.join(repo).join("agents/task")).expect("worktree dir");
        }
        let ambiguous = done_run_with_context(&done_args(&["app-task", "--dry-run"]), &mut DoneFakeRuntime::default(), &context)
            .expect_err("ambiguous aliases must fail");
        assert!(ambiguous.contains("ambiguous") && ambiguous.contains("acme/app/agents/task") && ambiguous.contains("org/app/agents/task"), "{ambiguous}");
        let missing = done_run_with_context(&done_args(&["app-taks", "--dry-run"]), &mut DoneFakeRuntime::default(), &context)
            .expect_err("missing target");
        assert!(missing.contains("accepted forms") && missing.contains("did you mean 'app-task'?"), "{missing}");

        let worktree = context.repos_root.join("acme/app/agents/task");
        let self_pane = done_test_pane("%42", 1, "codex", &worktree);
        let self_error = done_run_with_context(
            &done_args(&["%42", "--dry-run"]),
            &mut DoneFakeRuntime { panes: vec![self_pane], current_pane: Some("%42".to_owned()), ..DoneFakeRuntime::default() },
            &context,
        )
        .expect_err("own pane must fail");
        assert!(self_error.contains("invoking pane"), "{self_error}");
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
    fn done_leadless_session_refuses_without_force_and_names_escape_hatch() {
        let root = DoneTempRoot::new("leadless-refusal");
        let mut runtime = DoneFakeRuntime {
            windows: vec![DoneWindow {
                session: "orphan".to_owned(),
                index: 1,
                name: "worker".to_owned(),
                cwd: None,
            }],
            ..DoneFakeRuntime::default()
        };

        let err = done_run_with_context(&done_args(&["worker", "--dry-run"]), &mut runtime, &root.context())
            .expect_err("leadless session must require --force");

        assert_eq!(
            err,
            "refusing to done window 'worker' because the lead window for session 'orphan' could not be identified; retry with --force to retire the orphaned delivery"
        );
    }

    #[test]
    fn done_force_retires_leadless_session_worktree_window_and_branch() {
        let root = DoneTempRoot::new("leadless-force");
        let context = root.context();
        let main = context.repos_root.join("acme/app");
        let worktree = main.join("agents/worker");
        std::fs::create_dir_all(&worktree).expect("worktree dir");
        done_write_fleet(&root, "worker", "acme/app/agents/worker");
        let mut runtime = DoneFakeRuntime {
            windows: vec![DoneWindow {
                session: "orphan".to_owned(),
                index: 1,
                name: "worker".to_owned(),
                cwd: None,
            }],
            ..DoneFakeRuntime::default()
        };
        runtime.register_worktree(&main, &worktree);

        let output = done_run_with_context(
            &done_args(&["worker", "--force", "--clean-branch"]),
            &mut runtime,
            &context,
        )
        .expect("--force must retire the leadless delivery");

        assert!(output.contains("removed worktree acme/app/agents/worker"), "{output}");
        assert!(output.contains("deleted branch agent/task"), "{output}");
        assert_eq!(runtime.reaped_targets, vec!["orphan:worker"]);
        assert!(runtime.git_calls.iter().any(|args| {
            args == &vec![
                "-C".to_owned(),
                main.display().to_string(),
                "branch".to_owned(),
                "-D".to_owned(),
                "--".to_owned(),
                "agent/task".to_owned(),
            ]
        }), "{:#?}", runtime.git_calls);
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

        let wrong_context = DoneContext {
            repos_root: root.path.join("wrong-ghq/github.com"),
            fleet_dirs: Vec::new(),
            solo_lease_dir: root.path.join("wrong-state/lease"),
        };
        let mut runtime = DoneRealGitRuntime::default();
        let resolved = done_resolve_registered_worktree(&mut runtime, &live, &wrong_context).expect("resolve").expect("registered worktree");

        assert!(done_same_path(&resolved.main_path, &main), "{} != {}", resolved.main_path.display(), main.display());
        assert!(done_same_path(&resolved.full_path, &live), "{} != {}", resolved.full_path.display(), live.display());
        assert_eq!(resolved.label, "acme/app/agents/live-task");
    }

    #[test]
    fn done_dry_run_previews_an_ignored_psi_retro() {
        let root = DoneTempRoot::new("dry-run-psi-rescue");
        let context = root.context();
        let main = context.repos_root.join("acme/app");
        let worktree = main.join("agents/dry-run-psi-rescue");
        let relative = "ψ/memory/retrospectives/2026-07-27/dry-run.md";
        std::fs::create_dir_all(main.join("agents")).expect("main repo dir");
        done_run_process("git", &["init"], Some(&main));
        std::fs::write(main.join(".gitignore"), "ψ/\n").expect("ignore ψ");
        std::fs::write(main.join("README.md"), "dry-run fixture\n").expect("seed readme");
        done_run_process("git", &["add", ".gitignore", "README.md"], Some(&main));
        done_run_process(
            "git",
            &[
                "-c",
                "user.name=maw-test",
                "-c",
                "user.email=maw-test@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-m",
                "seed dry-run rescue fixture",
            ],
            Some(&main),
        );
        let worktree_arg = worktree.display().to_string();
        done_run_process(
            "git",
            &[
                "worktree",
                "add",
                "-b",
                "agents/dry-run-psi-rescue",
                &worktree_arg,
            ],
            Some(&main),
        );
        let note = worktree.join(relative);
        std::fs::create_dir_all(note.parent().expect("note parent")).expect("note dir");
        std::fs::write(&note, "preview this note\n").expect("write note");

        let mut runtime = DoneFakeRuntime {
            windows: vec![done_test_window("lead"), done_test_window_with_cwd("worker", &worktree)],
            ..DoneFakeRuntime::default()
        };
        runtime.register_worktree(&main, &worktree);
        let out = done_run_with_context(&done_args(&["worker", "--dry-run"]), &mut runtime, &context)
            .expect("done dry run");

        assert!(out.contains("[dry-run] would rescue 1 uncommitted ψ note(s) to main before removal"), "{out}");
        assert!(!main.join(relative).exists(), "dry run must not copy the note");
        let rescued = crate::wind::done::rescue_psi(&worktree, &main).expect("rescue ignored retro");
        assert_eq!(rescued, vec![main.join(relative)]);
        assert_eq!(std::fs::read_to_string(main.join(relative)).expect("rescued note"), "preview this note\n");
    }

    #[test]
    fn done_reports_a_nonempty_psi_without_rescue_candidates() {
        let root = DoneTempRoot::new("empty-psi-rescue");
        let main = root.repos_root().join("acme/app");
        let worktree = main.join("agents/empty-psi-rescue");
        std::fs::create_dir_all(main.join("agents")).expect("worktree parent");
        std::fs::create_dir_all(main.join("ψ/teams")).expect("tracked ψ dir");
        done_run_process("git", &["init"], Some(&main));
        std::fs::write(main.join(".gitignore"), "ψ/\n").expect("ignore ψ");
        std::fs::write(main.join("README.md"), "empty rescue fixture\n").expect("seed readme");
        std::fs::write(main.join("ψ/teams/roster.yaml"), "name: gale\n").expect("tracked ψ file");
        done_run_process("git", &["add", ".gitignore", "README.md"], Some(&main));
        done_run_process("git", &["add", "-f", "ψ/teams/roster.yaml"], Some(&main));
        done_run_process(
            "git",
            &[
                "-c",
                "user.name=maw-test",
                "-c",
                "user.email=maw-test@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-m",
                "seed empty rescue fixture",
            ],
            Some(&main),
        );
        let worktree_arg = worktree.display().to_string();
        done_run_process(
            "git",
            &["worktree", "add", "-b", "agents/empty-psi-rescue", &worktree_arg],
            Some(&main),
        );

        let mut stdout = String::new();
        done_rescue_psi_notes(
            &DoneWorktree {
                main_path: main,
                full_path: worktree,
                label: "acme/app/agents/empty-psi-rescue".to_owned(),
            },
            false,
            &mut stdout,
        );

        assert!(stdout.contains("ψ rescue found no uncommitted notes although ψ/ is non-empty"), "{stdout}");
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
    fn done_merged_branch_cleanup_deletes_remote_but_other_pr_states_do_not() {
        let main = std::path::PathBuf::from("/tmp/acme/app");
        let branch = "agents/merged-task";
        let remote_delete = vec![
            "-C".to_owned(),
            main.display().to_string(),
            "push".to_owned(),
            "origin".to_owned(),
            "--delete".to_owned(),
            branch.to_owned(),
        ];
        let mut merged = DoneFakeRuntime::default();
        merged.set_pr_state(&main, branch, PrGithubState::Merged);
        let mut merged_out = String::new();

        done_cleanup_branch(&main, branch, &DoneOptions::default(), &mut merged, &mut merged_out);

        assert!(merged.git_calls.iter().any(|args| args == &remote_delete), "{:#?}", merged.git_calls);
        assert!(merged_out.contains("deleted branch agents/merged-task local+remote (merged PR)"), "{merged_out}");

        for (label, state) in [
            ("open", Ok(PrGithubState::Open)),
            ("closed", Ok(PrGithubState::Closed)),
            ("unavailable", Err("gh unavailable".to_owned())),
        ] {
            let mut runtime = DoneFakeRuntime::default();
            runtime.pr_states.insert((main.clone(), branch.to_owned()), state);
            let mut stdout = String::new();

            done_cleanup_branch(&main, branch, &DoneOptions::default(), &mut runtime, &mut stdout);

            assert!(runtime.git_calls.iter().all(|args| args != &remote_delete), "{label}: {:#?}", runtime.git_calls);
            assert!(runtime.git_calls.iter().all(|args| !args.iter().any(|arg| arg == "-D")), "{label}: {:#?}", runtime.git_calls);
        }
    }

    #[test]
    fn done_remote_branch_cleanup_reports_remote_delete_errors_without_blocking() {
        let main = std::path::PathBuf::from("/tmp/acme/app");
        let branch = "agents/merged-task";
        let remote_delete = vec![
            "-C".to_owned(),
            main.display().to_string(),
            "push".to_owned(),
            "origin".to_owned(),
            "--delete".to_owned(),
            branch.to_owned(),
        ];
        let mut runtime = DoneFakeRuntime::default();
        runtime.set_pr_state(&main, branch, PrGithubState::Merged);
        runtime.git_errors.insert(remote_delete, "network unavailable".to_owned());
        let mut stdout = String::new();

        done_cleanup_branch(&main, branch, &DoneOptions::default(), &mut runtime, &mut stdout);

        assert!(stdout.contains("deleted branch agents/merged-task local (merged PR); remote retained: network unavailable"), "{stdout}");
    }

    #[test]
    fn done_sweep_removes_merged_deliveries_and_retains_an_open_delivery() {
        let root = DoneTempRoot::new("merged-sweep");
        let context = root.context();
        let main = context.repos_root.join("acme/app");
        let target = main.join("agents/done-target");
        let merged = main.join("agents/merged");
        let open = main.join("agents/open");
        let mut runtime = DoneFakeRuntime { windows: vec![done_test_window("lead"), done_test_window("worker")], ..DoneFakeRuntime::default() };
        runtime.pane_info.insert("s:worker".to_owned(), ("codex".to_owned(), target.display().to_string()));
        for (path, branch, state) in [
            (&target, "agents/done-target", PrGithubState::Merged),
            (&merged, "agents/merged", PrGithubState::Merged),
            (&open, "agents/open", PrGithubState::Open),
        ] {
            runtime.register_worktree_branch(&main, path, branch);
            runtime.set_pr_state(&main, branch, state);
        }
        runtime.local_branches.entry(main.clone()).or_default().insert("agents/merged-without-worktree".to_owned());
        runtime.set_pr_state(&main, "agents/merged-without-worktree", PrGithubState::Merged);

        let out = done_run_with_context(&done_args(&["worker", "--force"]), &mut runtime, &context).expect("done");

        assert!(out.contains("sweep removed stale worktree"), "{out}");
        assert!(out.contains("sweep retained") && out.contains("PR open"), "{out}");
        assert_eq!(runtime.local_branches.get(&main).cloned().unwrap_or_default(), ["agents/open".to_owned()].into());
        let remote_deletes = runtime.git_calls.iter()
            .filter(|args| args.get(2).is_some_and(|arg| arg == "push") && args.get(3).is_some_and(|arg| arg == "origin") && args.get(4).is_some_and(|arg| arg == "--delete"))
            .filter_map(|args| args.get(5).cloned())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(remote_deletes, ["agents/done-target".to_owned(), "agents/merged".to_owned(), "agents/merged-without-worktree".to_owned()].into());
    }

    #[test]
    fn done_sweep_never_removes_a_live_merged_delivery() {
        let root = DoneTempRoot::new("live-sweep");
        let context = root.context();
        let main = context.repos_root.join("acme/app");
        let target = main.join("agents/done-target");
        let live = main.join("agents/live");
        let mut runtime = DoneFakeRuntime {
            windows: vec![done_test_window("lead"), done_test_window("worker")],
            panes: vec![done_test_pane("%88", 1, "codex", &live)],
            ..DoneFakeRuntime::default()
        };
        runtime.pane_info.insert("s:worker".to_owned(), ("codex".to_owned(), target.display().to_string()));
        for (path, branch) in [(&target, "agents/done-target"), (&live, "agents/live")] {
            runtime.register_worktree_branch(&main, path, branch);
            runtime.set_pr_state(&main, branch, PrGithubState::Merged);
        }

        let out = done_run_with_context(&done_args(&["worker", "--force"]), &mut runtime, &context).expect("done");

        assert!(out.contains("sweep retained") && out.contains("live pane"), "{out}");
        assert!(runtime.registered[&main].contains(&live));
    }

    #[test]
    fn done_keep_branch_overrides_merged_pr_default() {
        let root = DoneTempRoot::new("keep-merged");
        let context = root.context();
        let main = context.repos_root.join("acme/app");
        let target = main.join("agents/done-target");
        let mut runtime = DoneFakeRuntime { windows: vec![done_test_window("lead"), done_test_window("worker")], ..DoneFakeRuntime::default() };
        runtime.pane_info.insert("s:worker".to_owned(), ("codex".to_owned(), target.display().to_string()));
        runtime.register_worktree_branch(&main, &target, "agents/done-target");
        runtime.set_pr_state(&main, "agents/done-target", PrGithubState::Merged);

        let out = done_run_with_context(&done_args(&["worker", "--force", "--keep-branch"]), &mut runtime, &context).expect("done");

        assert!(out.contains("retained (--keep-branch)"), "{out}");
        assert!(runtime.local_branches[&main].contains("agents/done-target"));
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
    fn done_retires_recorded_split_pane_after_worktree_removal_marks_its_cwd_deleted() {
        let root = DoneTempRoot::new("split-pane");
        let context = root.context();
        let main = context.repos_root.join("acme/app");
        let worktree = main.join("agents/issue-94");
        std::fs::create_dir_all(worktree.join(".maw")).expect("marker dir");
        std::fs::write(worktree.join(".maw/pane-id"), "%42\n").expect("pane marker");
        done_write_fleet(&root, "issue-94", "acme/app/agents/issue-94");

        let mut runtime = DoneFakeRuntime::default();
        runtime.register_worktree(&main, &worktree);
        runtime
            .tmux_responses
            .insert("display-message".to_owned(), format!("2\t{} (deleted)\t@7\n", worktree.display()));

        let output = done_run_with_context(&done_args(&["issue-94", "--force"]), &mut runtime, &context)
            .expect("done split pane");

        assert!(output.contains("killed split pane %42"), "{output}");
        assert!(runtime.tmux_calls.iter().any(|(command, args)| command == "kill-pane" && args == &done_args(&["-t", "%42"])));
        assert!(!runtime.tmux_calls.iter().any(|(command, _)| command == "kill-window"));
        assert!(runtime.git_calls.iter().any(|args| args.iter().any(|arg| arg == "remove")));
        assert_eq!(runtime.actions, vec!["remove-worktree", "kill-pane"]);
    }

    #[test]
    fn done_removes_unignored_ephemeral_markers_and_retires_recorded_pane() {
        let root = DoneTempRoot::new("unignored-marker-cleanup");
        let (_main, worktree) = done_init_unignored_worktree(&root, "marker-task");
        std::fs::create_dir_all(worktree.join(".maw")).expect("marker dir");
        std::fs::write(worktree.join(".maw/pane-id"), "%42\n").expect("pane marker");
        std::fs::write(worktree.join(".maw/delivery.json"), "{}\n").expect("delivery marker");
        std::fs::write(worktree.join(".maw/phase.json"), "{}\n").expect("phase marker");

        let mut runtime = DoneRealGitRuntime::default();
        runtime
            .tmux_responses
            .insert("display-message".to_owned(), format!("2\t{} (deleted)\t@7\n", worktree.display()));
        let output = done_run_with_context(
            &done_args(&["marker-task", "--worktree", &worktree.display().to_string()]),
            &mut runtime,
            &root.context(),
        )
        .expect("unignored maw markers must not block done");

        assert!(output.contains("removed worktree acme/app/agents/marker-task"), "{output}");
        assert!(!worktree.exists(), "worktree should be removed: {}", worktree.display());
        assert!(runtime.tmux_calls.iter().any(|(command, args)| {
            command == "kill-pane" && args == &done_args(&["-t", "%42"])
        }));
    }

    #[test]
    fn done_preserves_pane_proof_when_genuine_dirt_refuses_removal() {
        let root = DoneTempRoot::new("marker-cleanup-user-dirt");
        let (_main, worktree) = done_init_unignored_worktree(&root, "dirty-marker-task");
        std::fs::create_dir_all(worktree.join(".maw")).expect("marker dir");
        std::fs::write(worktree.join(".maw/pane-id"), "%42\n").expect("pane marker");
        std::fs::write(worktree.join(".maw/delivery.json"), "{}\n").expect("delivery marker");
        std::fs::write(worktree.join("README.md"), "user edit\n").expect("genuine user dirt");

        let mut runtime = DoneRealGitRuntime::default();
        let error = done_run_with_context(
            &done_args(&["dirty-marker-task", "--worktree", &worktree.display().to_string()]),
            &mut runtime,
            &root.context(),
        )
        .expect_err("genuine user dirt must still refuse removal");

        assert!(error.contains("contains modified or untracked files"), "{error}");
        assert_eq!(std::fs::read_to_string(worktree.join(".maw/pane-id")).expect("restored pane proof"), "%42\n");
        assert!(!worktree.join(".maw/delivery.json").exists(), "non-proof markers stay cleaned");
        assert!(runtime.tmux_calls.iter().all(|(command, _)| command != "kill-pane"));
    }

    #[test]
    fn done_rebalances_a_marked_workon_window_after_closing_its_pane() {
        let root = DoneTempRoot::new("split-pane-rebalance");
        let context = root.context();
        let main = context.repos_root.join("acme/app");
        let worktree = main.join("agents/issue-94");
        std::fs::create_dir_all(worktree.join(".maw")).expect("marker dir");
        std::fs::write(worktree.join(".maw/pane-id"), "%42\n").expect("pane marker");
        done_write_fleet(&root, "issue-94", "acme/app/agents/issue-94");

        let mut runtime = DoneFakeRuntime::default();
        runtime.register_worktree(&main, &worktree);
        runtime.tmux_responses.insert("display-message".to_owned(), format!("3\t{}\t@7\n", worktree.display()));
        runtime
            .tmux_responses
            .insert("show-window-options".to_owned(), "main-vertical\n".to_owned());

        done_run_with_context(&done_args(&["issue-94", "--force"]), &mut runtime, &context)
            .expect("done split pane");

        assert!(runtime.tmux_calls.iter().any(|(command, args)| {
            command == "select-layout"
                && args == &done_args(&["-t", "@7", "main-vertical"])
        }));
    }

    #[test]
    fn done_continues_cleanup_when_the_recorded_split_pane_is_already_gone() {
        let root = DoneTempRoot::new("dead-split-pane");
        let context = root.context();
        let main = context.repos_root.join("acme/app");
        let worktree = main.join("agents/issue-94");
        std::fs::create_dir_all(worktree.join(".maw")).expect("marker dir");
        std::fs::write(worktree.join(".maw/pane-id"), "%42\n").expect("pane marker");
        done_write_fleet(&root, "issue-94", "acme/app/agents/issue-94");

        let mut runtime = DoneFakeRuntime::default();
        runtime.register_worktree(&main, &worktree);

        let output = done_run_with_context(
            &done_args(&["issue-94", "--force", "--clean-branch"]),
            &mut runtime,
            &context,
        )
        .expect("dead pane must not block worktree cleanup");

        assert!(output.contains("split pane %42 already gone"), "{output}");
        assert!(
            runtime.git_calls.iter().any(|args| args.iter().any(|arg| arg == "remove")),
            "{:#?}",
            runtime.git_calls
        );
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
    fn done_refuses_dirty_worktree_before_killing_its_pane() {
        let root = DoneTempRoot::new("dirty");
        let context = root.context();
        let main = context.repos_root.join("acme/app");
        let dirty = main.join("agents/dirty-task");
        std::fs::create_dir_all(dirty.join(".maw")).expect("marker dir");
        std::fs::write(dirty.join(".maw/pane-id"), "%42\n").expect("pane marker");
        done_write_fleet(&root, "worker", "acme/app/agents/dirty-task");

        let mut runtime = DoneFakeRuntime::default();
        runtime.dirty_removals.insert(dirty.clone());
        runtime.register_worktree(&main, &dirty);
        runtime
            .tmux_responses
            .insert("display-message".to_owned(), format!("2\t{}\t@7\n", dirty.display()));

        let err = done_run_with_context(&done_args(&["worker"]), &mut runtime, &context).expect_err("dirty");
        assert!(err.contains("contains modified or untracked files"), "{err}");
        assert!(runtime.git_calls.iter().all(|args| !args.iter().any(|arg| arg == "--force")), "{:?}", runtime.git_calls);
        assert!(
            runtime
                .tmux_calls
                .iter()
                .all(|(command, _)| command != "kill-pane"),
            "a failed worktree removal must leave its pane intact: {:#?}",
            runtime.tmux_calls
        );
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
        let _lock = env_test_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
