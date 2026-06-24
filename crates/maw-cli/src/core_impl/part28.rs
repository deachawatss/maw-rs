fn run_attach_plan(argv: &[String]) -> CliOutput {
    let mut print = false;
    let mut readonly = false;
    let mut plan_json = false;
    let mut yes = false;
    let mut ssh_alias: Option<String> = None;
    let mut alive = BTreeSet::new();
    let mut target: Option<String> = None;
    let mut index = 0;
    while index < argv.len() {
        match argv[index].as_str() {
            "--help" | "-h" => return attach_usage_ok(),
            "--print" => print = true,
            "--readonly" | "--read-only" | "-r" => readonly = true,
            "--plan-json" | "--dry-run" => plan_json = true,
            "--yes" | "-y" => yes = true,
            "--ssh-alias" => {
                let Some(value) = argv.get(index + 1) else {
                    return attach_usage_error("attach: missing --ssh-alias value");
                };
                ssh_alias = Some(value.to_owned());
                index += 1;
            }
            "--alive" => {
                let Some(value) = argv.get(index + 1) else {
                    return attach_usage_error("attach: missing --alive value");
                };
                alive.insert(value.to_owned());
                index += 1;
            }
            arg if arg.starts_with("--alive=") => {
                alive.insert(arg["--alive=".len()..].to_owned());
            }
            arg if arg.starts_with("--ssh-alias=") => {
                ssh_alias = Some(arg["--ssh-alias=".len()..].to_owned());
            }
            arg if arg.starts_with('-') => {
                return attach_usage_error(&format!("attach: unknown argument {arg}"));
            }
            value => {
                if target.is_some() {
                    return attach_usage_error("attach: target already provided");
                }
                target = Some(value.to_owned());
            }
        }
        index += 1;
    }

    let Some(target) = target else {
        return attach_usage_error("attach: target required");
    };
    if let Some((node, session_name)) = parse_explicit_remote_attach_target(&target) {
        let alias = ssh_alias.unwrap_or_else(|| node.clone());
        let stdout = if plan_json {
            render_attach_remote_plan_json(&target, &node, &session_name, &alias, yes)
        } else {
            render_attach_remote_plan_text(&target, &node, &session_name, &alias, yes)
        };
        return CliOutput { code: 0, stdout, stderr: String::new() };
    }
    if alive.is_empty() {
        let mut client = TmuxClient::local();
        alive = client.list_session_names().into_iter().collect();
    }
    let resolved_target = match resolve_tmux_attach_session(&target, &alive) {
        TmuxAttachSessionResolution::Match { session }
        | TmuxAttachSessionResolution::Missing { session } => session,
        TmuxAttachSessionResolution::Ambiguous { candidates, .. } => {
            return attach_ambiguous_error(&target, &candidates);
        }
    };
    let in_tmux = std::env::var_os("TMUX").is_some();
    let action = decide_tmux_attach_action(&resolved_target, &alive, print || plan_json, false, in_tmux);
    let session = attach_action_session(&action);
    let stdout = if plan_json {
        render_attach_plan_json(&target, session, &action, readonly)
    } else {
        render_attach_plan_text(&target, session, &action, readonly)
    };
    let code = i32::from(matches!(action, TmuxAttachAction::Recover { .. }));
    CliOutput {
        code,
        stdout,
        stderr: String::new(),
    }
}

fn attach_ambiguous_error(target: &str, candidates: &[String]) -> CliOutput {
    CliOutput {
        code: 2,
        stdout: String::new(),
        stderr: format!(
            "attach: '{target}' matches multiple sessions: {}\n  use the full name: maw-rs attach <exact-session>\n",
            candidates.join(", ")
        ),
    }
}

fn attach_usage_ok() -> CliOutput {
    CliOutput {
        code: 0,
        stdout: attach_usage_text(),
        stderr: String::new(),
    }
}

fn attach_usage_error(message: &str) -> CliOutput {
    CliOutput {
        code: 2,
        stdout: String::new(),
        stderr: format!("{message}\n{}", attach_usage_text()),
    }
}

fn attach_usage_text() -> String {
    "usage: maw-rs attach <target> [--print] [--readonly|-r]\n       maw-rs a <target> [--print] [--readonly|-r]\n".to_owned()
}


fn parse_explicit_remote_attach_target(target: &str) -> Option<(String, String)> {
    let (node, session_name) = target.split_once(':')?;
    let node = node.trim();
    let session_name = session_name.trim();
    if node.is_empty() || session_name.is_empty() {
        return None;
    }
    if session_name
        .split_once('.')
        .map_or_else(|| session_name.chars().all(|c| c.is_ascii_digit()), |(window, pane)| {
            window.chars().all(|c| c.is_ascii_digit()) && pane.chars().all(|c| c.is_ascii_digit())
        })
    {
        return None;
    }
    Some((node.to_owned(), session_name.to_owned()))
}

fn render_attach_remote_plan_text(
    target: &str,
    node: &str,
    session_name: &str,
    ssh_alias: &str,
    yes: bool,
) -> String {
    let yes_suffix = if yes { " -y" } else { "" };
    format!(
        "  \x1b[36m·\x1b[0m [dry-run] Tier 3 (remote) — would attach to {node}:{session_name} via ssh {ssh_alias}
  command: maw-rs attach-ssh --node {node} --session {session_name} --ssh-alias {ssh_alias}{yes_suffix}
  resolved: {target} → {node}:{session_name}
"
    )
}

fn render_attach_remote_plan_json(
    target: &str,
    node: &str,
    session_name: &str,
    ssh_alias: &str,
    yes: bool,
) -> String {
    let attach_ssh_args = vec![
        "--node".to_owned(),
        node.to_owned(),
        "--session".to_owned(),
        session_name.to_owned(),
        "--ssh-alias".to_owned(),
        ssh_alias.to_owned(),
    ];
    format!(
        "{{\"command\":\"attach\",\"alias\":\"a\",\"target\":{},\"action\":\"remote-attach\",\"tier\":3,\"node\":{},\"sessionName\":{},\"sshAlias\":{},\"yes\":{},\"attachSshArgs\":{}}}\n",
        json_string(target),
        json_string(node),
        json_string(session_name),
        json_string(ssh_alias),
        yes,
        json_string_array(&attach_ssh_args)
    )
}

fn render_attach_plan_text(
    target: &str,
    session: &str,
    action: &TmuxAttachAction,
    readonly: bool,
) -> String {
    match action {
        TmuxAttachAction::Recover { .. } => format!(
            "attach: '{target}' resolved to missing session {session}\n  → maw wake {target} --attach\n"
        ),
        TmuxAttachAction::Print { .. }
        | TmuxAttachAction::SwitchClient { .. }
        | TmuxAttachAction::Attach { .. } => {
            let args = attach_command_args(action, readonly);
            format!(
                "Run: tmux {}\n  resolved: {target} → {session}\n  detach with: Ctrl-b d\n",
                args.join(" ")
            )
        }
    }
}

fn render_attach_plan_json(
    target: &str,
    session: &str,
    action: &TmuxAttachAction,
    readonly: bool,
) -> String {
    let kind = match action {
        TmuxAttachAction::Print { .. } => "print",
        TmuxAttachAction::SwitchClient { .. } => "switch-client",
        TmuxAttachAction::Attach { .. } => "attach",
        TmuxAttachAction::Recover { .. } => "recover",
    };
    let args = attach_command_args(action, readonly);
    format!(
        "{{\"command\":\"attach\",\"alias\":\"a\",\"target\":{},\"session\":{},\"action\":{},\"tmuxArgs\":{}}}\n",
        json_string(target),
        json_string(session),
        json_string(kind),
        json_string_array(&args)
    )
}

fn attach_command_args(action: &TmuxAttachAction, readonly: bool) -> Vec<String> {
    if readonly {
        return vec![
            "attach".to_owned(),
            "-r".to_owned(),
            "-t".to_owned(),
            attach_action_session(action).to_owned(),
        ];
    }
    tmux_attach_spawn_command(action).map_or_else(
        || vec!["attach".to_owned(), "-t".to_owned(), attach_action_session(action).to_owned()],
        |command| command.args,
    )
}

fn attach_action_session(action: &TmuxAttachAction) -> &str {
    match action {
        TmuxAttachAction::Print { session }
        | TmuxAttachAction::SwitchClient { session }
        | TmuxAttachAction::Attach { session }
        | TmuxAttachAction::Recover { session } => session,
    }
}


const STREAM_USAGE: &str = "usage: maw-rs stream <session>:<win> [--into <session>] [--name <alias>] [--dry-run|--plan-json] | maw-rs stream --unlink <session>:<alias> [--dry-run|--plan-json]";
const STREAM_PLACEHOLDER_WINDOW: &str = "maw-stream-placeholder";

#[derive(Debug, Clone, PartialEq, Eq)]
struct StreamOptions {
    into: Option<String>,
    name: Option<String>,
    unlink: bool,
    dry_run: bool,
    plan_json: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StreamSource {
    session: String,
    index: u32,
    name: String,
    target: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StreamPlan {
    source: Option<String>,
    into: String,
    name: String,
    target: String,
    created_destination: bool,
    renamed_shared_window: bool,
    unlinked: bool,
    tmux_commands: Vec<Vec<String>>,
}

fn run_stream_command(argv: &[String]) -> CliOutput {
    let (target, opts) = match parse_stream_args(argv) {
        Ok(parsed) => parsed,
        Err(message) => return stream_usage_error(&message),
    };
    if opts.unlink {
        let plan = match build_stream_unlink_plan(&target) {
            Ok(plan) => plan,
            Err(message) => return stream_usage_error(&message),
        };
        return execute_or_render_stream_plan(&plan, opts.dry_run, opts.plan_json);
    }

    match build_stream_link_plan(&target, &opts) {
        Ok(plan) => execute_or_render_stream_plan(&plan, opts.dry_run, opts.plan_json),
        Err(StreamBuildError::Usage(message)) => stream_usage_error(&message),
        Err(StreamBuildError::Command(message)) => command_target_error("stream", &message),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum StreamBuildError {
    Usage(String),
    Command(String),
}

fn build_stream_unlink_plan(target: &str) -> Result<StreamPlan, String> {
    let parsed = parse_stream_window_target(target)?;
    Ok(StreamPlan {
        source: None,
        into: parsed.0.clone(),
        name: parsed.1.clone(),
        target: format!("{}:{}", parsed.0, parsed.1),
        created_destination: false,
        renamed_shared_window: false,
        unlinked: true,
        tmux_commands: vec![vec![
            "unlink-window".to_owned(),
            "-t".to_owned(),
            format!("{}:{}", parsed.0, parsed.1),
        ]],
    })
}

fn build_stream_link_plan(target: &str, opts: &StreamOptions) -> Result<StreamPlan, StreamBuildError> {
    let mut client = TmuxClient::local();
    let source = resolve_stream_source(&mut client, target).map_err(StreamBuildError::Command)?;
    let destination = stream_destination(opts).map_err(StreamBuildError::Command)?;
    if destination == source.session {
        return Err(StreamBuildError::Command(
            "destination session must differ from source session".to_owned(),
        ));
    }
    let alias = assert_stream_name("window alias", opts.name.as_deref().unwrap_or(&source.name))
        .map_err(StreamBuildError::Usage)?;
    let destination_exists = client.has_session(&destination);
    if !destination_exists && opts.into.is_some() {
        return Err(StreamBuildError::Command(format!(
            "destination session '{destination}' not found"
        )));
    }
    let before = if destination_exists {
        client
            .list_windows(&destination)
            .map_err(|error| StreamBuildError::Command(error.message))?
    } else {
        Vec::new()
    };
    if before.iter().any(|window| window.name == alias) {
        let hint = if opts.name.is_some() {
            "choose a different --name"
        } else {
            "use --name <alias>"
        };
        return Err(StreamBuildError::Command(format!(
            "destination window '{destination}:{alias}' already exists; {hint}"
        )));
    }
    Ok(stream_link_plan_from_parts(
        source,
        &destination,
        &alias,
        destination_exists,
        &before,
    ))
}

fn stream_destination(opts: &StreamOptions) -> Result<String, String> {
    if let Some(into) = opts.into.as_deref() {
        return assert_stream_name("destination session", into);
    }
    current_tmux_session_name().map(|current| {
        if current.ends_with("-view") {
            current
        } else {
            format!("{current}-view")
        }
    })
}

fn stream_link_plan_from_parts(
    source: StreamSource,
    destination: &str,
    alias: &str,
    destination_exists: bool,
    before: &[maw_tmux::TmuxWindow],
) -> StreamPlan {
    let index = next_stream_window_index(before, destination_base_index(destination));
    let destination_target = format!("{destination}:{index}");
    let mut commands = Vec::new();
    if !destination_exists {
        commands.push(vec![
            "new-session".to_owned(),
            "-d".to_owned(),
            "-s".to_owned(),
            destination.to_owned(),
            "-n".to_owned(),
            STREAM_PLACEHOLDER_WINDOW.to_owned(),
        ]);
    }
    commands.push(vec![
        "link-window".to_owned(),
        "-d".to_owned(),
        "-s".to_owned(),
        source.target.clone(),
        "-t".to_owned(),
        destination_target.clone(),
    ]);
    if alias != source.name {
        commands.push(vec![
            "rename-window".to_owned(),
            "-t".to_owned(),
            destination_target.clone(),
            alias.to_owned(),
        ]);
    }
    commands.push(vec![
        "set-window-option".to_owned(),
        "-t".to_owned(),
        destination_target,
        "@maw-linked-from".to_owned(),
        source.target.clone(),
    ]);
    if !destination_exists {
        commands.push(vec![
            "kill-window".to_owned(),
            "-t".to_owned(),
            format!("{destination}:{STREAM_PLACEHOLDER_WINDOW}"),
        ]);
    }
    StreamPlan {
        source: Some(source.target),
        into: destination.to_owned(),
        name: alias.to_owned(),
        target: format!("{destination}:{alias}"),
        created_destination: !destination_exists,
        renamed_shared_window: alias != source.name,
        unlinked: false,
        tmux_commands: commands,
    }
}

fn parse_stream_args(argv: &[String]) -> Result<(String, StreamOptions), String> {
    let mut opts = StreamOptions { into: None, name: None, unlink: false, dry_run: false, plan_json: false };
    let mut target = None;
    let mut index = 0;
    while index < argv.len() {
        match argv[index].as_str() {
            "--help" | "-h" => return Err(STREAM_USAGE.to_owned()),
            "--unlink" => opts.unlink = true,
            "--dry-run" => opts.dry_run = true,
            "--plan-json" => opts.plan_json = true,
            "--into" => {
                let Some(value) = argv.get(index + 1) else { return Err("stream: missing --into value".to_owned()); };
                opts.into = Some(value.clone());
                index += 1;
            }
            "--name" => {
                let Some(value) = argv.get(index + 1) else { return Err("stream: missing --name value".to_owned()); };
                opts.name = Some(value.clone());
                index += 1;
            }
            arg if arg.starts_with("--into=") => opts.into = Some(arg["--into=".len()..].to_owned()),
            arg if arg.starts_with("--name=") => opts.name = Some(arg["--name=".len()..].to_owned()),
            arg if arg.starts_with('-') => return Err(format!("stream: unknown argument {arg}")),
            value => {
                if target.is_some() { return Err("stream: target already provided".to_owned()); }
                target = Some(value.to_owned());
            }
        }
        index += 1;
    }
    target.map(|target| (target, opts)).ok_or_else(|| STREAM_USAGE.to_owned())
}

fn stream_usage_error(message: &str) -> CliOutput {
    let stderr = if message == STREAM_USAGE { format!("{STREAM_USAGE}\n") } else { format!("{message}\n{STREAM_USAGE}\n") };
    CliOutput { code: 2, stdout: String::new(), stderr }
}

fn parse_stream_window_target(target: &str) -> Result<(String, String), String> {
    let raw = target.trim();
    if raw.is_empty() || raw.starts_with('-') { return Err(STREAM_USAGE.to_owned()); }
    let Some((session, window)) = raw.split_once(':') else {
        return Err("stream: target must be <session>:<window>".to_owned());
    };
    let session = session.trim();
    let window = window.trim();
    if session.is_empty() || window.is_empty() {
        return Err("stream: target must be <session>:<window>".to_owned());
    }
    if window.rsplit_once('.').is_some_and(|(_, pane)| pane.chars().all(|c| c.is_ascii_digit())) {
        return Err("stream: target must be a tmux window, not a pane".to_owned());
    }
    Ok((session.to_owned(), window.to_owned()))
}

fn assert_stream_name(kind: &str, value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.starts_with('-') || trimmed.contains(':') {
        return Err(format!("stream: invalid {kind}: {}", if value.is_empty() { "(empty)" } else { value }));
    }
    Ok(trimmed.to_owned())
}

fn resolve_stream_source<R: maw_tmux::TmuxRunner>(
    client: &mut TmuxClient<R>,
    target: &str,
) -> Result<StreamSource, String> {
    let (session, window) = parse_stream_window_target(target)?;
    let windows = client
        .list_windows(&session)
        .map_err(|_| format!("source session '{session}' not found"))?;
    if window.chars().all(|c| c.is_ascii_digit()) {
        let index = window.parse::<u32>().map_err(|_| format!("source window '{session}:{window}' not found"))?;
        let Some(found) = windows.iter().find(|candidate| candidate.index == index) else {
            return Err(format!("source window '{session}:{window}' not found"));
        };
        return Ok(StreamSource { session, index: found.index, name: found.name.clone(), target: format!("{}:{}", target_session(target), found.index) });
    }
    let exact: Vec<_> = windows.iter().filter(|candidate| candidate.name == window).collect();
    match exact.as_slice() {
        [found] => Ok(StreamSource { session: session.clone(), index: found.index, name: found.name.clone(), target: format!("{session}:{}", found.index) }),
        [] => {
            let available = if windows.is_empty() { "(none)".to_owned() } else { windows.iter().map(|w| format!("{}:{}", w.index, w.name)).collect::<Vec<_>>().join(", ") };
            Err(format!("source window '{session}:{window}' not found; windows: {available}"))
        }
        _ => Err(format!(
            "source window '{session}:{window}' is ambiguous; use one of: {}",
            exact.iter().map(|w| format!("{session}:{}", w.index)).collect::<Vec<_>>().join(", ")
        )),
    }
}

fn target_session(target: &str) -> String {
    target.split_once(':').map_or(target, |(session, _)| session).to_owned()
}

fn current_tmux_session_name() -> Result<String, String> {
    let output = std::process::Command::new("tmux")
        .args(["display-message", "-p", "#{session_name}"])
        .output()
        .map_err(|error| format!("--into is required outside tmux ({error})"))?;
    if !output.status.success() {
        return Err("--into is required outside tmux".to_owned());
    }
    let session = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if session.is_empty() { Err("--into is required outside tmux".to_owned()) } else { Ok(session) }
}

fn destination_base_index(session: &str) -> u32 {
    let output = std::process::Command::new("tmux")
        .args(["show-options", "-t", session, "-gv", "base-index"])
        .output();
    output.ok().and_then(|out| {
        if out.status.success() { String::from_utf8_lossy(&out.stdout).trim().parse::<u32>().ok() } else { None }
    }).unwrap_or(0)
}

fn next_stream_window_index(windows: &[maw_tmux::TmuxWindow], base_index: u32) -> u32 {
    let used = windows.iter().map(|window| window.index).collect::<BTreeSet<_>>();
    (base_index..10_000).find(|index| !used.contains(index)).unwrap_or(base_index)
}

fn execute_or_render_stream_plan(plan: &StreamPlan, dry_run: bool, plan_json: bool) -> CliOutput {
    if plan_json {
        return CliOutput { code: 0, stdout: render_stream_plan_json(plan), stderr: String::new() };
    }
    if dry_run {
        return CliOutput { code: 0, stdout: render_stream_dry_run(plan), stderr: String::new() };
    }
    for command in &plan.tmux_commands {
        let Some((subcommand, args)) = command.split_first() else { continue; };
        let status = std::process::Command::new("tmux").args(args).status();
        match status {
            Ok(status) if status.success() => {}
            Ok(status) => return command_target_error("stream", &format!("tmux {subcommand} exited {}", status.code().unwrap_or(1))),
            Err(error) => return command_target_error("stream", &format!("failed to execute tmux {subcommand}: {error}")),
        }
    }
    CliOutput { code: 0, stdout: format_stream_result(plan), stderr: String::new() }
}

fn render_stream_dry_run(plan: &StreamPlan) -> String {
    let mut stdout = String::new();
    for command in &plan.tmux_commands {
        let _ = writeln!(stdout, "tmux {}", command.join(" "));
    }
    stdout
}

fn render_stream_plan_json(plan: &StreamPlan) -> String {
    format!(
        "{{\"command\":\"stream\",\"source\":{},\"into\":{},\"name\":{},\"target\":{},\"createdDestination\":{},\"renamedSharedWindow\":{},\"unlinked\":{},\"tmuxCommands\":{}}}\n",
        plan.source.as_ref().map_or("null".to_owned(), |source| json_string(source)),
        json_string(&plan.into),
        json_string(&plan.name),
        json_string(&plan.target),
        plan.created_destination,
        plan.renamed_shared_window,
        plan.unlinked,
        json_string_matrix(&plan.tmux_commands)
    )
}

fn json_string_matrix(rows: &[Vec<String>]) -> String {
    format!("[{}]", rows.iter().map(|row| json_string_array(row)).collect::<Vec<_>>().join(","))
}

fn format_stream_result(plan: &StreamPlan) -> String {
    if plan.unlinked { return format!("stream: unlinked {}\n", plan.target); }
    let created = if plan.created_destination { " (created destination)" } else { "" };
    let renamed = if plan.renamed_shared_window { " (renamed shared window)" } else { "" };
    format!("stream: linked {} -> {}{created}{renamed}\n", plan.source.as_deref().unwrap_or(""), plan.target)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AttachSshTarget {
    node: String,
    session_name: String,
    ssh_alias: String,
}

fn run_attach_ssh_command(argv: &[String]) -> CliOutput {
    let (target, dry_run, plan_json) = match parse_attach_ssh_args(argv) {
        Ok(parsed) => parsed,
        Err(message) => return attach_ssh_usage_error(&message),
    };
    if let Err(message) = validate_attach_ssh_target(&target) {
        return command_target_error("attach-ssh", &message);
    }
    let probe_args = attach_ssh_probe_args(&target.ssh_alias);
    let attach_args = attach_ssh_exec_args(&target.ssh_alias, &target.session_name);
    if plan_json {
        return CliOutput {
            code: 0,
            stdout: format!(
                "{{\"command\":\"attach-ssh\",\"node\":{},\"sessionName\":{},\"sshAlias\":{},\"probeArgs\":{},\"sshArgs\":{}}}\n",
                json_string(&target.node),
                json_string(&target.session_name),
                json_string(&target.ssh_alias),
                json_string_array(&probe_args),
                json_string_array(&attach_args)
            ),
            stderr: String::new(),
        };
    }
    if dry_run {
        return CliOutput {
            code: 0,
            stdout: format!(
                "ssh {}\nssh {}\n",
                probe_args.join(" "),
                attach_args.join(" ")
            ),
            stderr: String::new(),
        };
    }
    match run_ssh_preflight(&target.ssh_alias) {
        Ok(()) => {}
        Err(reason) => {
            let msg = format!(
                "✗ ssh {} unreachable in 3s ({reason})\n  • check ~/.ssh/config for 'Host {}'\n  • check 'maw peers list' for the routing alias\n  • try the WG hostname directly: ssh {}.wg\n",
                target.ssh_alias, target.ssh_alias, target.ssh_alias
            );
            return CliOutput { code: 1, stdout: String::new(), stderr: msg };
        }
    }
    let status = std::process::Command::new("ssh")
        .args(attach_args)
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status();
    match status {
        Ok(status) if status.success() => CliOutput { code: 0, stdout: String::new(), stderr: String::new() },
        Ok(status) => command_target_error("attach-ssh", &format!("ssh attach to {} ({}) failed with exit {}", target.node, target.ssh_alias, status.code().unwrap_or(1))),
        Err(error) => command_target_error("attach-ssh", &format!("failed to execute ssh: {error}")),
    }
}

fn parse_attach_ssh_args(argv: &[String]) -> Result<(AttachSshTarget, bool, bool), String> {
    let mut node = None;
    let mut session_name = None;
    let mut ssh_alias = None;
    let mut dry_run = false;
    let mut plan_json = false;
    let mut positional = Vec::new();
    let mut index = 0;
    while index < argv.len() {
        match argv[index].as_str() {
            "--help" | "-h" => return Err(attach_ssh_usage_text()),
            "--dry-run" => dry_run = true,
            "--plan-json" => plan_json = true,
            "--node" => {
                let Some(value) = argv.get(index + 1) else { return Err("attach-ssh: missing --node value".to_owned()); };
                node = Some(value.clone());
                index += 1;
            }
            "--session" | "--session-name" => {
                let Some(value) = argv.get(index + 1) else { return Err("attach-ssh: missing --session value".to_owned()); };
                session_name = Some(value.clone());
                index += 1;
            }
            "--ssh-alias" => {
                let Some(value) = argv.get(index + 1) else { return Err("attach-ssh: missing --ssh-alias value".to_owned()); };
                ssh_alias = Some(value.clone());
                index += 1;
            }
            arg if arg.starts_with("--node=") => node = Some(arg["--node=".len()..].to_owned()),
            arg if arg.starts_with("--session=") => session_name = Some(arg["--session=".len()..].to_owned()),
            arg if arg.starts_with("--session-name=") => session_name = Some(arg["--session-name=".len()..].to_owned()),
            arg if arg.starts_with("--ssh-alias=") => ssh_alias = Some(arg["--ssh-alias=".len()..].to_owned()),
            arg if arg.starts_with('-') => return Err(format!("attach-ssh: unknown argument {arg}")),
            value => positional.push(value.to_owned()),
        }
        index += 1;
    }
    if node.is_none() && positional.len() >= 2 {
        node = Some(positional[0].clone());
        session_name = Some(positional[1].clone());
    }
    if ssh_alias.is_none() {
        ssh_alias.clone_from(&node);
    }
    Ok((AttachSshTarget {
        node: node.ok_or_else(|| "attach-ssh: --node required".to_owned())?,
        session_name: session_name.ok_or_else(|| "attach-ssh: --session required".to_owned())?,
        ssh_alias: ssh_alias.ok_or_else(|| "attach-ssh: --ssh-alias required".to_owned())?,
    }, dry_run, plan_json))
}

fn attach_ssh_usage_error(message: &str) -> CliOutput {
    let usage = attach_ssh_usage_text();
    let stderr = if message == usage { format!("{usage}\n") } else { format!("{message}\n{usage}\n") };
    CliOutput { code: 2, stdout: String::new(), stderr }
}

fn attach_ssh_usage_text() -> String {
    "usage: maw-rs attach-ssh --node <node> --session <session> --ssh-alias <alias> [--dry-run|--plan-json]".to_owned()
}

fn validate_attach_ssh_target(target: &AttachSshTarget) -> Result<(), String> {
    validate_attach_ssh_node(&target.node)?;
    validate_attach_ssh_alias(&target.node, &target.ssh_alias)?;
    if !is_safe_attach_ssh_token(&target.session_name) {
        return Err(format!(
            "cannot attach to {}: unsafe tmux session '{}'",
            target.node, target.session_name
        ));
    }
    Ok(())
}

fn validate_attach_ssh_node(node: &str) -> Result<(), String> {
    if !is_safe_attach_ssh_token(node) {
        return Err(format!("cannot attach: unsafe node '{node}'"));
    }
    Ok(())
}

fn validate_attach_ssh_alias(node: &str, alias: &str) -> Result<(), String> {
    if alias.trim().is_empty() {
        return Err(format!("cannot attach to {node}: missing SSH target"));
    }
    if !is_safe_attach_ssh_token(alias) {
        return Err(format!("cannot attach to {node}: unsafe ssh alias '{alias}'"));
    }
    Ok(())
}

fn is_safe_attach_ssh_token(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty()
        && trimmed == value
        && !trimmed.starts_with('-')
        && trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':' | '-'))
}

fn attach_ssh_probe_args(alias: &str) -> Vec<String> {
    vec!["-o".to_owned(), "ConnectTimeout=3".to_owned(), "-o".to_owned(), "BatchMode=yes".to_owned(), alias.to_owned(), "true".to_owned()]
}

fn attach_ssh_exec_args(alias: &str, session_name: &str) -> Vec<String> {
    vec!["-tt".to_owned(), alias.to_owned(), format!("tmux attach-session -t {session_name}")]
}

fn run_ssh_preflight(alias: &str) -> Result<(), String> {
    let status = std::process::Command::new("ssh")
        .args(attach_ssh_probe_args(alias))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|error| error.to_string())?;
    if status.success() { Ok(()) } else { Err(format!("ssh exited {}", status.code().map_or_else(|| "?".to_owned(), |code| code.to_string()))) }
}

fn run_run_command(argv: &[String]) -> CliOutput {
    let (target, text) = match parse_run_command_args(argv) {
        Ok(parsed) => parsed,
        Err(message) => return run_usage_error(&message),
    };
    let mut client = TmuxClient::local();
    let resolved = match resolve_local_tmux_command_target(&mut client, &target) {
        Ok(target) => target,
        Err(message) => return command_target_error("run", &message),
    };
    if !text.is_empty() {
        if let Err(error) = client.send_keys_literal(&resolved, &text) {
            return command_target_error("run", &format!("tmux send-keys failed: {error}"));
        }
    }
    if let Err(error) = client.send_enter(&resolved) {
        return command_target_error("run", &format!("tmux send-keys failed: {error}"));
    }
    CliOutput {
        code: 0,
        stdout: format!("\x1b[32mran\x1b[0m → {resolved}: {}\n", truncate_cli_text(&text, 200)),
        stderr: String::new(),
    }
}

fn run_send_enter_command(argv: &[String]) -> CliOutput {
    let (target, count) = match parse_send_enter_command_args(argv) {
        Ok(parsed) => parsed,
        Err(message) => return send_enter_usage_error(&message),
    };
    let mut client = TmuxClient::local();
    let resolved = match resolve_local_tmux_command_target(&mut client, &target) {
        Ok(target) => target,
        Err(message) => return command_target_error("send-enter", &message),
    };
    for _ in 0..count {
        if let Err(error) = client.send_enter(&resolved) {
            return command_target_error(
                "send-enter",
                &format!("tmux send-keys failed: {error}"),
            );
        }
    }
    let plural = if count == 1 {
        "Enter".to_owned()
    } else {
        format!("{count} Enters")
    };
    CliOutput {
        code: 0,
        stdout: format!("\x1b[32mdelivered\x1b[0m → {resolved}: {plural}\n"),
        stderr: String::new(),
    }
}

fn parse_run_command_args(argv: &[String]) -> Result<(String, String), String> {
    let Some(target_index) = argv.iter().position(|arg| !arg.starts_with('-')) else {
        return Err("usage: maw-rs run <target> \"<cmd>\"".to_owned());
    };
    let target = argv[target_index].clone();
    let text = argv[target_index + 1..].join(" ");
    Ok((target, text))
}

fn parse_send_enter_command_args(argv: &[String]) -> Result<(String, usize), String> {
    let mut target = None;
    let mut count = 1usize;
    let mut index = 0;
    while index < argv.len() {
        let arg = &argv[index];
        if matches!(arg.as_str(), "--N" | "-N" | "--n") {
            let Some(next) = argv.get(index + 1) else {
                return Err("--N requires a positive integer (got: nothing)".to_owned());
            };
            count = parse_send_enter_count(next, next)?;
            index += 2;
            continue;
        }
        if let Some(value) = arg
            .strip_prefix("--N=")
            .or_else(|| arg.strip_prefix("--n="))
        {
            count = parse_send_enter_count(value, arg)?;
            index += 1;
            continue;
        }
        if target.is_none() && !arg.starts_with('-') {
            target = Some(arg.clone());
        }
        index += 1;
    }
    let Some(target) = target else {
        return Err("usage: maw-rs send-enter <target> [--N <count>]".to_owned());
    };
    Ok((target, count))
}

fn parse_send_enter_count(raw: &str, label: &str) -> Result<usize, String> {
    match raw.parse::<usize>() {
        Ok(count) if count > 0 => Ok(count),
        _ => Err(format!("--N requires a positive integer (got: {label})")),
    }
}

fn resolve_local_tmux_command_target(
    client: &mut TmuxClient<maw_tmux::CommandTmuxRunner>,
    query: &str,
) -> Result<String, String> {
    if query.starts_with('%') {
        return Ok(query.to_owned());
    }
    let sessions = client
        .list_all()
        .into_iter()
        .map(|session| RouteSession {
            name: session.name,
            windows: session
                .windows
                .into_iter()
                .map(|window| RouteWindow {
                    index: window.index,
                    name: window.name,
                    active: window.active,
                })
                .collect(),
            source: None,
        })
        .collect::<Vec<_>>();
    match resolve_route_target(query, &RouteConfig::default(), &sessions) {
        RouteResult::Local { target } | RouteResult::SelfNode { target } => Ok(target),
        RouteResult::Peer { node, target, .. } => Err(format!(
            "cross-node target '{query}' (node '{node}', target '{target}') is not supported"
        )),
        RouteResult::Error { detail, hint, .. } => {
            if let Some(hint) = hint {
                Err(format!("{detail} — {hint}"))
            } else {
                Err(detail)
            }
        }
    }
}

fn command_target_error(command: &str, message: &str) -> CliOutput {
    CliOutput {
        code: 1,
        stdout: String::new(),
        stderr: format!("{command}: {message}\n"),
    }
}

fn run_usage_error(message: &str) -> CliOutput {
    CliOutput {
        code: 2,
        stdout: String::new(),
        stderr: format!("{message}\nusage: maw-rs run <target> \"<cmd>\"\n"),
    }
}

fn send_enter_usage_error(message: &str) -> CliOutput {
    CliOutput {
        code: 2,
        stdout: String::new(),
        stderr: format!("{message}\nusage: maw-rs send-enter <target> [--N <count>]\n"),
    }
}

fn truncate_cli_text(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let truncated = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{truncated}…")
    } else {
        truncated
    }
}
