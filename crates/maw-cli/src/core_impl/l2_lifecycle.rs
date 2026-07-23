const DISPATCH_382: &[DispatcherEntry] = &[DispatcherEntry {
    command: "__l2-watch",
    handler: Handler::Sync(l2_watch_command),
}];
use maw_tmux::TmuxRunner as _;

const L2_WATCH_INTERVAL_SECS: u64 = 25;
const L2_IDLE_TIMEOUT_SECS: u64 = 180;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
enum L2TerminalState {
    Exited,
    Error,
    Pr,
    Findings,
    Ready,
    Blocked,
    Idle,
}

impl L2TerminalState {
    const fn handoff_kind(self) -> &'static str {
        match self {
            Self::Findings => "FINDINGS",
            Self::Pr | Self::Ready => "READY",
            Self::Blocked => "BLOCKED",
            Self::Error => "ERROR",
            Self::Idle => "IDLE",
            Self::Exited => "EXITED",
        }
    }

    const fn body(self) -> &'static str {
        match self {
            Self::Findings => "L2 emitted a FINDINGS handoff",
            Self::Pr => "L2 opened a pull request",
            Self::Ready => "L2 emitted a READY handoff",
            Self::Blocked => "L2 is blocked at an interactive prompt",
            Self::Error => "L2 pane reported a hard error",
            Self::Idle => "L2 is idle at a prompt",
            Self::Exited => "L2 pane exited",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct L2ParentMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    parent_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    l1_oracle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    l1_session: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    l2_pane: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    repo: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct L2Event {
    version: u8,
    l2_pane: String,
    l2_session: String,
    l1_oracle: String,
    l1_session: String,
    repo: String,
    issue: u64,
    state: L2TerminalState,
    transition_seq: u64,
    message: String,
    notified: bool,
    notified_at: Option<String>,
    created_at: String,
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct L2Transition {
    last_state: Option<L2TerminalState>,
    transition_seq: u64,
    #[serde(default)]
    pending_event: Option<L2Event>,
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct L2ObserverOwner {
    active: bool,
    pid: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct L2Snapshot<'a> {
    alive: bool,
    command: &'a str,
    text: &'a str,
    unchanged_secs: u64,
}

#[allow(clippy::too_many_arguments)]
fn format_l2_handoff(
    kind: &str,
    l1_oracle: &str,
    repo: &str,
    issue: u64,
    mode: &str,
    risk: &str,
    body: &str,
    engine: &str,
) -> String {
    format!(
        "[{l1_oracle}:{repo}] {kind} issue #{issue} ({mode}/{risk}): {body} — Oracle-authored ({engine} L2)"
    )
}

fn l2_classify_snapshot(snapshot: &L2Snapshot<'_>, idle_timeout_secs: u64) -> Option<L2TerminalState> {
    if !snapshot.alive || workon_is_shell_command(snapshot.command) {
        return Some(L2TerminalState::Exited);
    }
    let recent = snapshot.text.lines().rev().take(20).collect::<Vec<_>>().join("\n");
    let lower = recent.to_ascii_lowercase();
    if lower.contains("panicked at")
        || lower.contains("fatal error")
        || lower.contains("engine exited with status")
    {
        return Some(L2TerminalState::Error);
    }
    if lower.contains("github.com/") && lower.contains("/pull/") {
        return Some(L2TerminalState::Pr);
    }
    if recent.lines().any(|line| line.trim_start().starts_with("FINDINGS")) {
        return Some(L2TerminalState::Findings);
    }
    if recent.lines().any(|line| line.trim_start().starts_with("READY:")) {
        return Some(L2TerminalState::Ready);
    }
    if l2_is_approval_prompt(&lower) {
        return Some(L2TerminalState::Blocked);
    }
    if snapshot.unchanged_secs >= idle_timeout_secs && is_stuck_activity_snapshot(snapshot.text) {
        return Some(L2TerminalState::Idle);
    }
    None
}

fn l2_is_approval_prompt(lower: &str) -> bool {
    lower.contains("[y/n]")
        || lower.contains("yes/no")
        || lower.contains("approve this")
        || lower.contains("allow this")
        || lower.contains("trust this")
        || lower.contains("permission required")
}

fn l2_record_parent_metadata(cwd: &Path, metadata: &L2ParentMetadata) -> Result<(), String> {
    let dir = cwd.join(".maw");
    std::fs::create_dir_all(&dir).map_err(|error| format!("l2 metadata: create {}: {error}", dir.display()))?;
    let body = serde_json::to_string_pretty(metadata).map_err(|error| format!("l2 metadata: render: {error}"))? + "\n";
    let tmp = dir.join(format!(".l2-meta.{}.tmp", std::process::id()));
    std::fs::write(&tmp, body).map_err(|error| format!("l2 metadata: write {}: {error}", tmp.display()))?;
    std::fs::rename(&tmp, dir.join("l2-meta.json")).map_err(|error| format!("l2 metadata: replace: {error}"))
}

fn l2_record_pane_metadata(cwd: &Path, pane: &str, metadata: &L2ParentMetadata) -> Result<(), String> {
    let dir = cwd.join(".maw");
    std::fs::create_dir_all(&dir).map_err(|error| format!("l2 metadata: create {}: {error}", dir.display()))?;
    let body = serde_json::to_string_pretty(metadata).map_err(|error| format!("l2 metadata: render: {error}"))? + "\n";
    let tmp = dir.join(format!(".l2-meta-{}.{}.tmp", l2_pane_key(pane), std::process::id()));
    std::fs::write(&tmp, body).map_err(|error| format!("l2 metadata: write {}: {error}", tmp.display()))?;
    let path = l2_pane_metadata_path(cwd, pane);
    std::fs::rename(&tmp, &path).map_err(|error| format!("l2 metadata: replace {}: {error}", path.display()))
}

fn l2_prepare_observer(
    cwd: &Path,
    pane: &str,
    l1_oracle: &str,
    l1_session: Option<&str>,
    l2_session: Option<&str>,
) -> Result<(), String> {
    let metadata = L2ParentMetadata {
        parent_session_id: l1_session.map(str::to_owned),
        session_id: l2_session.map(str::to_owned).or_else(|| Some(pane.to_owned())),
        l1_oracle: Some(l1_oracle.to_owned()),
        l1_session: l1_session.map(str::to_owned).or_else(|| Some(l1_oracle.to_owned())),
        l2_pane: Some(pane.to_owned()),
        repo: cwd.file_name().and_then(std::ffi::OsStr::to_str).map(str::to_owned),
    };
    l2_record_pane_metadata(cwd, pane, &metadata)?;
    l2_record_parent_metadata(cwd, &metadata)?;
    l2_arm_observer(cwd, pane)
}

fn l2_pane_key(pane: &str) -> String {
    pane.chars().filter(char::is_ascii_alphanumeric).collect()
}

fn l2_observer_owner_path(cwd: &Path, pane: &str) -> std::path::PathBuf {
    cwd.join(".maw").join(format!("l2-observer-{}.json", l2_pane_key(pane)))
}

fn l2_write_observer_owner(cwd: &Path, pane: &str, owner: &L2ObserverOwner) -> Result<(), String> {
    let path = l2_observer_owner_path(cwd, pane);
    let body = serde_json::to_string(owner).map_err(|error| format!("l2 observer: render ownership: {error}"))?;
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&tmp, body).map_err(|error| format!("l2 observer: write ownership: {error}"))?;
    std::fs::rename(&tmp, path).map_err(|error| format!("l2 observer: replace ownership: {error}"))
}

fn l2_claim_observer_owner(cwd: &Path, pane: &str) -> Result<bool, String> {
    let dir = cwd.join(".maw");
    std::fs::create_dir_all(&dir).map_err(|error| format!("l2 observer: create metadata directory: {error}"))?;
    let _lock = PrQueueLock::acquire(&dir)?;
    let path = l2_observer_owner_path(cwd, pane);
    let owner = std::fs::read_to_string(&path)
        .ok()
        .and_then(|body| serde_json::from_str::<L2ObserverOwner>(&body).ok())
        .unwrap_or_default();
    if owner.active {
        return Ok(false);
    }
    l2_write_observer_owner(cwd, pane, &L2ObserverOwner { active: true, pid: None })?;
    Ok(true)
}

fn l2_release_observer_owner(cwd: &Path, pane: &str) -> Result<(), String> {
    let dir = cwd.join(".maw");
    std::fs::create_dir_all(&dir).map_err(|error| format!("l2 observer: create metadata directory: {error}"))?;
    let _lock = PrQueueLock::acquire(&dir)?;
    l2_write_observer_owner(cwd, pane, &L2ObserverOwner { active: false, pid: None })
}

#[cfg(test)]
thread_local! {
    static L2_TEST_ARM_FAILURE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static L2_TEST_ARM_COUNT: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn l2_test_fail_next_arm() {
    L2_TEST_ARM_FAILURE.with(|failure| failure.set(true));
}

#[cfg(test)]
fn l2_test_arm_count() -> u32 {
    L2_TEST_ARM_COUNT.with(std::cell::Cell::get)
}

#[cfg(test)]
fn l2_arm_observer(cwd: &Path, pane: &str) -> Result<(), String> {
    if !l2_claim_observer_owner(cwd, pane)? {
        return Ok(());
    }
    if L2_TEST_ARM_FAILURE.with(|failure| failure.replace(false)) {
        l2_release_observer_owner(cwd, pane)?;
        return Err("l2 observer: injected arm failure".to_owned());
    }
    L2_TEST_ARM_COUNT.with(|count| count.set(count.get().saturating_add(1)));
    Ok(())
}

#[cfg(not(test))]
fn l2_arm_observer(cwd: &Path, pane: &str) -> Result<(), String> {
    if std::env::var("MAW_TEST_MODE").as_deref() == Ok("1") { return Ok(()); }
    if !l2_claim_observer_owner(cwd, pane)? {
        return Ok(());
    }
    let executable = std::env::current_exe().map_err(|error| format!("l2 observer: current executable: {error}"))?;
    let child = std::process::Command::new(executable)
        .args(["__l2-watch", pane])
        .current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|error| format!("l2 observer: spawn: {error}"));
    match child {
        Ok(child) => l2_write_observer_owner(cwd, pane, &L2ObserverOwner { active: true, pid: Some(child.id()) }),
        Err(error) => {
            l2_release_observer_owner(cwd, pane)?;
            Err(error)
        }
    }
}

fn l2_watch_command(argv: &[String]) -> CliOutput {
    let result = argv.first().ok_or_else(|| "usage: maw __l2-watch <pane>".to_owned()).and_then(|pane| l2_watch(pane));
    match result {
        Ok(()) => CliOutput { code: 0, stdout: String::new(), stderr: String::new() },
        Err(message) => CliOutput { code: 1, stdout: String::new(), stderr: format!("{message}\n") },
    }
}

fn l2_watch(pane: &str) -> Result<(), String> {
    validate_activity_tmux_target(pane)?;
    let cwd = std::env::current_dir().map_err(|error| format!("l2 observer: cwd: {error}"))?;
    let result = l2_watch_loop(&cwd, pane);
    if let Err(error) = l2_release_observer_owner(&cwd, pane) {
        eprintln!("l2 observer: release ownership: {error}");
    }
    result
}

fn l2_watch_loop(cwd: &Path, pane: &str) -> Result<(), String> {
    let config = merged_config_value_in_dir(cwd);
    let interval = config.get("l2_watch_interval_secs").and_then(serde_json::Value::as_u64).unwrap_or(L2_WATCH_INTERVAL_SECS);
    let idle_timeout = config.get("l2_idle_timeout_secs").and_then(serde_json::Value::as_u64).unwrap_or(L2_IDLE_TIMEOUT_SECS);
    let mut runner = maw_tmux::CommandTmuxRunner::new();
    let mut previous = String::new();
    let mut unchanged_since = std::time::Instant::now();
    let mut last_state = None;
    let mut launched = false;
    loop {
        let capture = runner.run("capture-pane", &["-t".to_owned(), pane.to_owned(), "-p".to_owned(), "-S".to_owned(), "-80".to_owned()]);
        let command = runner.run("display-message", &["-t".to_owned(), pane.to_owned(), "-p".to_owned(), "#{pane_current_command}".to_owned()]);
        let alive = capture.is_ok() && command.is_ok();
        let text = capture.as_deref().unwrap_or_default();
        if text != previous {
            previous.clear();
            previous.push_str(text);
            unchanged_since = std::time::Instant::now();
        }
        let snapshot = L2Snapshot {
            alive,
            command: command.as_deref().unwrap_or_default(),
            text,
            unchanged_secs: unchanged_since.elapsed().as_secs(),
        };
        launched |= snapshot.alive && !workon_is_shell_command(snapshot.command);
        let state = launched.then(|| l2_classify_snapshot(&snapshot, idle_timeout)).flatten();
        if let Some(state) = state.filter(|state| Some(*state) != last_state) {
            match l2_emit_state(cwd, pane, state, None) {
                Ok(_) => {
                    last_state = Some(state);
                    if matches!(state, L2TerminalState::Exited | L2TerminalState::Error | L2TerminalState::Pr | L2TerminalState::Findings | L2TerminalState::Ready) {
                        return Ok(());
                    }
                }
                Err(error) => eprintln!("l2 observer: retrying {state:?} transition: {error}"),
            }
        } else if state.is_none() {
            last_state = None;
            if let Err(error) = l2_observe_active(cwd, pane) {
                eprintln!("l2 observer: retrying pending transition: {error}");
            }
        }
        std::thread::sleep(std::time::Duration::from_secs(interval.max(1)));
    }
}

fn l2_delivery_context(cwd: &Path) -> (u64, String, String, String) {
    let value = std::fs::read_to_string(cwd.join(".maw/delivery.json"))
        .ok()
        .and_then(|body| serde_json::from_str::<serde_json::Value>(&body).ok())
        .unwrap_or_default();
    let issue = value.get("issue").and_then(serde_json::Value::as_u64).unwrap_or_default();
    let mode = value.get("mode").and_then(serde_json::Value::as_str).unwrap_or("standard").to_owned();
    let risk = value.get("riskTags").and_then(serde_json::Value::as_array)
        .map(|tags| tags.iter().filter_map(serde_json::Value::as_str).collect::<Vec<_>>().join(","))
        .filter(|risk| !risk.is_empty()).unwrap_or_else(|| "none".to_owned());
    let engine = value.get("engine").and_then(serde_json::Value::as_str).unwrap_or("codex").to_owned();
    (issue, mode, risk, engine)
}

fn l2_emit_state(cwd: &Path, pane: &str, state: L2TerminalState, body: Option<&str>) -> Result<bool, String> {
    let metadata_path = l2_pane_metadata_path(cwd, pane);
    let metadata = std::fs::read_to_string(if metadata_path.exists() { metadata_path } else { cwd.join(".maw/l2-meta.json") })
        .map_err(|error| format!("l2 event: read metadata: {error}"))
        .and_then(|raw| serde_json::from_str::<L2ParentMetadata>(&raw).map_err(|error| format!("l2 event: parse metadata: {error}")))?;
    let (issue, mode, risk, engine) = l2_delivery_context(cwd);
    let repo = metadata.repo.clone().or_else(|| cwd.file_name().and_then(std::ffi::OsStr::to_str).map(str::to_owned)).unwrap_or_else(|| "unknown".to_owned());
    let l1_oracle = metadata.l1_oracle.clone().or_else(|| std::fs::read_to_string(cwd.join(".maw/l1-oracle")).ok().map(|value| value.trim().to_owned())).unwrap_or_else(|| "unknown".to_owned());
    let mut delivered = l2_flush_pending_transition(cwd, pane)?;
    let message = format_l2_handoff(state.handoff_kind(), &l1_oracle, &repo, issue, &mode, &risk, body.unwrap_or_else(|| state.body()), &engine);
    let event = L2Event {
        version: 1,
        l2_pane: pane.to_owned(),
        l2_session: metadata.session_id.clone().unwrap_or_else(|| pane.to_owned()),
        l1_oracle: l1_oracle.clone(),
        l1_session: metadata.l1_session.or(metadata.parent_session_id).unwrap_or(l1_oracle),
        repo,
        issue,
        state,
        transition_seq: 0,
        message,
        notified: false,
        notified_at: None,
        created_at: cli_dispatch_now_iso(),
    };
    if !l2_stage_transition(cwd, pane, event)? {
        return Ok(delivered);
    }
    delivered |= l2_flush_pending_transition(cwd, pane)?;
    Ok(delivered)
}

fn l2_pane_metadata_path(cwd: &Path, pane: &str) -> std::path::PathBuf {
    cwd.join(".maw").join(format!("l2-meta-{}.json", l2_pane_key(pane)))
}

fn l2_emit_pr_event(cwd: &Path, pr_number: u64, url: &str) -> Result<bool, String> {
    let pane = std::env::var("MAW_L2_PANE_ID")
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| std::fs::read_to_string(cwd.join(".maw/pane-id")).ok().map(|value| value.trim().to_owned()).filter(|value| !value.is_empty()))
        .unwrap_or_else(|| "unknown".to_owned());
    let metadata_path = l2_pane_metadata_path(cwd, &pane);
    if !metadata_path.exists() {
        let l1_oracle = std::fs::read_to_string(cwd.join(".maw/l1-oracle")).ok().map(|value| value.trim().to_owned()).filter(|value| !value.is_empty());
        l2_record_pane_metadata(cwd, &pane, &L2ParentMetadata {
            session_id: Some(pane.clone()),
            l1_session: l1_oracle.clone(),
            l1_oracle,
            l2_pane: Some(pane.clone()),
            repo: cwd.file_name().and_then(std::ffi::OsStr::to_str).map(str::to_owned),
            ..L2ParentMetadata::default()
        })?;
    }
    let metadata = std::fs::read_to_string(metadata_path).map_err(|error| format!("l2 event: read PR metadata: {error}"))?;
    let metadata = serde_json::from_str::<L2ParentMetadata>(&metadata).map_err(|error| format!("l2 event: parse PR metadata: {error}"))?;
    let metadata_pane = metadata.l2_pane.as_deref().unwrap_or(&pane);
    l2_emit_state(cwd, metadata_pane, L2TerminalState::Pr, Some(&format!("PR #{pr_number} ready. {url}")))
}

fn l2_transition_path(cwd: &Path, pane: &str) -> std::path::PathBuf {
    cwd.join(".maw").join(format!("l2-transition-{}.json", l2_pane_key(pane)))
}

fn l2_read_transition(path: &Path) -> L2Transition {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|body| serde_json::from_str::<L2Transition>(&body).ok())
        .unwrap_or_default()
}

fn l2_write_transition(path: &Path, transition: &L2Transition) -> Result<(), String> {
    let body = serde_json::to_string(transition).map_err(|error| format!("l2 event: render transition: {error}"))?;
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&tmp, body).map_err(|error| format!("l2 event: write transition: {error}"))?;
    std::fs::rename(&tmp, path).map_err(|error| format!("l2 event: replace transition: {error}"))
}

fn l2_stage_transition(cwd: &Path, pane: &str, mut event: L2Event) -> Result<bool, String> {
    let dir = cwd.join(".maw");
    std::fs::create_dir_all(&dir).map_err(|error| format!("l2 event: create transition directory: {error}"))?;
    let _lock = PrQueueLock::acquire(&dir)?;
    let path = l2_transition_path(cwd, pane);
    let mut transition = l2_read_transition(&path);
    if transition.pending_event.is_some() || transition.last_state == Some(event.state) {
        return Ok(false);
    }
    transition.transition_seq = transition.transition_seq.saturating_add(1);
    event.transition_seq = transition.transition_seq;
    transition.pending_event = Some(event);
    l2_write_transition(&path, &transition)?;
    Ok(true)
}

fn l2_event_key(event: &L2Event) -> (&str, L2TerminalState, u64) {
    (&event.l2_session, event.state, event.transition_seq)
}

fn l2_flush_pending_transition(cwd: &Path, pane: &str) -> Result<bool, String> {
    let dir = cwd.join(".maw");
    std::fs::create_dir_all(&dir).map_err(|error| format!("l2 event: create transition directory: {error}"))?;
    let pending = {
        let _lock = PrQueueLock::acquire(&dir)?;
        l2_read_transition(&l2_transition_path(cwd, pane)).pending_event
    };
    let Some(event) = pending else { return Ok(false) };
    let _ = l2_enqueue_event(&event)?;
    let _lock = PrQueueLock::acquire(&dir)?;
    let path = l2_transition_path(cwd, pane);
    let mut transition = l2_read_transition(&path);
    if transition.pending_event.as_ref().is_some_and(|pending| l2_event_key(pending) == l2_event_key(&event)) {
        transition.last_state = Some(event.state);
        transition.pending_event = None;
        l2_write_transition(&path, &transition)?;
    }
    Ok(true)
}

fn l2_observe_active(cwd: &Path, pane: &str) -> Result<(), String> {
    l2_flush_pending_transition(cwd, pane)?;
    let dir = cwd.join(".maw");
    std::fs::create_dir_all(&dir).map_err(|error| format!("l2 event: create transition directory: {error}"))?;
    let _lock = PrQueueLock::acquire(&dir)?;
    let path = l2_transition_path(cwd, pane);
    let mut transition = l2_read_transition(&path);
    if transition.pending_event.is_none() && transition.last_state.take().is_some() {
        l2_write_transition(&path, &transition)?;
    }
    Ok(())
}

fn l2_enqueue_event(event: &L2Event) -> Result<bool, String> {
    let root = pr_review_queue_root()?;
    let _lock = PrQueueLock::acquire(&root)?;
    let mut lines = pr_read_queue_lines(&root, "l2-events.jsonl")?;
    let duplicate = lines.iter().filter_map(|line| serde_json::from_str::<L2Event>(line).ok()).any(|row| {
        row.l2_session == event.l2_session && row.state == event.state && row.transition_seq == event.transition_seq
    });
    if duplicate { return Ok(false); }
    lines.push(serde_json::to_string(&event).map_err(|error| format!("l2 event: render: {error}"))?);
    pr_write_queue_lines(&root, "l2-events.jsonl", &lines)?;
    Ok(true)
}

fn l2_drain_events() -> Result<Vec<L2Event>, String> {
    let root = pr_review_queue_root()?;
    let _lock = PrQueueLock::acquire(&root)?;
    let current_oracles = [std::env::var("MAW_ORACLE").ok(), l2_current_tmux_session()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    let current_session = ["MAW_SESSION_ID", "CLAUDE_SESSION_ID"].iter().find_map(|key| std::env::var(key).ok());
    let mut events = Vec::new();
    for line in pr_read_queue_lines(&root, "l2-events.jsonl")? {
        let Ok(event) = serde_json::from_str::<L2Event>(&line) else { continue };
        let targets_l1 = current_oracles.iter().any(|value| value == &event.l1_oracle)
            || current_session.as_ref().is_some_and(|value| value == &event.l1_session);
        if !event.notified && targets_l1 {
            events.push(event);
        }
    }
    Ok(events)
}

fn l2_acknowledge_events(events: &[L2Event]) -> Result<(), String> {
    if events.is_empty() {
        return Ok(());
    }
    let root = pr_review_queue_root()?;
    let _lock = PrQueueLock::acquire(&root)?;
    let keys = events.iter().map(l2_event_key).collect::<std::collections::HashSet<_>>();
    let mut retained = Vec::new();
    let mut archived = pr_read_queue_lines(&root, "l2-events.jsonl.archived")?;
    for line in pr_read_queue_lines(&root, "l2-events.jsonl")? {
        let Ok(mut event) = serde_json::from_str::<L2Event>(&line) else { retained.push(line); continue };
        if event.notified || !keys.contains(&l2_event_key(&event)) {
            retained.push(line);
            continue;
        }
        event.notified = true;
        event.notified_at = Some(cli_dispatch_now_iso());
        archived.push(serde_json::to_string(&event).map_err(|error| format!("l2 event: archive render: {error}"))?);
    }
    pr_write_queue_lines(&root, "l2-events.jsonl", &retained)?;
    pr_write_queue_lines(&root, "l2-events.jsonl.archived", &archived)
}

fn l2_current_tmux_session() -> Option<String> {
    std::env::var_os("TMUX")?;
    let mut runner = maw_tmux::CommandTmuxRunner::new();
    runner.run("display-message", &["-p".to_owned(), "#{session_name}".to_owned()]).ok().map(|value| value.trim().to_owned()).filter(|value| !value.is_empty())
}

#[cfg(test)]
mod l2_lifecycle_tests {
    use super::*;

    #[test]
    fn canonical_handoff_golden_literals() {
        for (kind, body) in [("FINDINGS", "found cause"), ("READY", "PR #7"), ("BLOCKED", "needs input"), ("ERROR", "panic"), ("IDLE", "at prompt"), ("EXITED", "pane died")] {
            assert_eq!(
                format_l2_handoff(kind, "gale", "maw-rs", 99, "standard", "api", body, "codex"),
                format!("[gale:maw-rs] {kind} issue #99 (standard/api): {body} — Oracle-authored (codex L2)")
            );
        }
    }

    #[test]
    fn observer_terminal_precedence_and_idle_timeout() {
        let snapshot = |alive, command, text, unchanged_secs| L2Snapshot { alive, command, text, unchanged_secs };
        assert_eq!(l2_classify_snapshot(&snapshot(false, "codex", "READY: done", 500), 180), Some(L2TerminalState::Exited));
        assert_eq!(l2_classify_snapshot(&snapshot(true, "codex", "fatal error\nhttps://github.com/a/b/pull/7", 0), 180), Some(L2TerminalState::Error));
        assert_eq!(l2_classify_snapshot(&snapshot(true, "codex", "https://github.com/a/b/pull/7", 0), 180), Some(L2TerminalState::Pr));
        assert_eq!(l2_classify_snapshot(&snapshot(true, "codex", "FINDINGS issue #99", 0), 180), Some(L2TerminalState::Findings));
        assert_eq!(l2_classify_snapshot(&snapshot(true, "codex", "Allow this command? [y/n]", 0), 180), Some(L2TerminalState::Blocked));
        assert_eq!(l2_classify_snapshot(&snapshot(true, "codex", ">", 179), 180), None);
        assert_eq!(l2_classify_snapshot(&snapshot(true, "codex", ">", 180), 180), Some(L2TerminalState::Idle));
    }

    #[test]
    fn durable_queue_deduplicates_and_drains_exactly_once() {
        let _guard = env_test_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let _state = EnvVarRestore::capture("MAW_STATE_DIR");
        let _oracle = EnvVarRestore::capture("MAW_ORACLE");
        let root = std::env::temp_dir().join(format!("maw-rs-l2-events-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("state root");
        std::env::set_var("MAW_STATE_DIR", &root);
        std::env::set_var("MAW_ORACLE", "gale");
        let event = L2Event {
            version: 1,
            l2_pane: "%9".to_owned(),
            l2_session: "child-1".to_owned(),
            l1_oracle: "gale".to_owned(),
            l1_session: "parent-1".to_owned(),
            repo: "maw-rs".to_owned(),
            issue: 99,
            state: L2TerminalState::Findings,
            transition_seq: 1,
            message: "one durable handoff".to_owned(),
            notified: false,
            notified_at: None,
            created_at: "2026-07-23T00:00:00.000Z".to_owned(),
        };
        assert!(l2_enqueue_event(&event).expect("enqueue"));
        assert!(!l2_enqueue_event(&event).expect("dedupe"));
        let first = l2_drain_events().expect("first drain");
        assert_eq!(first.iter().map(|event| event.message.as_str()).collect::<Vec<_>>(), ["one durable handoff"]);
        l2_acknowledge_events(&first).expect("acknowledge banner delivery");
        assert!(l2_drain_events().expect("second drain").is_empty());
        assert!(std::fs::read_to_string(root.join("l2-events.jsonl")).expect("active queue").is_empty());
        let archived = std::fs::read_to_string(root.join("l2-events.jsonl.archived")).expect("archive");
        assert_eq!(archived.lines().count(), 1);
        assert!(archived.contains("\"notified\":true"));
        std::fs::remove_dir_all(root).expect("cleanup state");
    }

    #[test]
    fn enqueue_failure_keeps_transition_pending_until_the_event_is_durable() {
        let _guard = env_test_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let _state = EnvVarRestore::capture("MAW_STATE_DIR");
        let _oracle = EnvVarRestore::capture("MAW_ORACLE");
        let root = std::env::temp_dir().join(format!("maw-rs-l2-pending-{}", std::process::id()));
        let repo = root.join("maw-rs");
        let blocked_state = root.join("blocked-state");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(repo.join(".maw")).expect("repo metadata");
        std::fs::write(&blocked_state, "not a directory").expect("blocked state path");
        std::env::set_var("MAW_ORACLE", "test-l1");
        std::fs::write(repo.join(".maw/delivery.json"), r#"{"issue":99,"mode":"standard","riskTags":["api"],"engine":"codex"}"#).expect("delivery");
        l2_record_pane_metadata(&repo, "%9", &L2ParentMetadata {
            session_id: Some("child-1".to_owned()),
            l1_oracle: Some("test-l1".to_owned()),
            l1_session: Some("parent-1".to_owned()),
            l2_pane: Some("%9".to_owned()),
            repo: Some("maw-rs".to_owned()),
            ..L2ParentMetadata::default()
        })
        .expect("pane metadata");

        std::env::set_var("MAW_STATE_DIR", &blocked_state);
        assert!(l2_emit_state(&repo, "%9", L2TerminalState::Findings, None).is_err());
        let pending = std::fs::read_to_string(l2_transition_path(&repo, "%9")).expect("pending transition");
        let transition = serde_json::from_str::<L2Transition>(&pending).expect("pending transition json");
        assert!(transition.pending_event.is_some(), "{pending}");
        assert_eq!(transition.last_state, None, "{pending}");

        std::env::set_var("MAW_STATE_DIR", root.join("state"));
        assert!(l2_emit_state(&repo, "%9", L2TerminalState::Findings, None).expect("retry pending event"));
        let events = l2_drain_events().expect("durable event is visible");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].transition_seq, 1);
        std::fs::remove_dir_all(root).expect("cleanup root");
    }

    #[test]
    fn active_periods_allow_idle_and_blocked_to_reenter() {
        let _guard = env_test_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let _state = EnvVarRestore::capture("MAW_STATE_DIR");
        let _oracle = EnvVarRestore::capture("MAW_ORACLE");
        let root = std::env::temp_dir().join(format!("maw-rs-l2-reenter-{}", std::process::id()));
        let repo = root.join("maw-rs");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(repo.join(".maw")).expect("repo metadata");
        std::env::set_var("MAW_STATE_DIR", root.join("state"));
        std::env::set_var("MAW_ORACLE", "gale");
        std::fs::write(repo.join(".maw/delivery.json"), r#"{"issue":99,"mode":"standard","riskTags":["api"],"engine":"codex"}"#).expect("delivery");
        l2_record_pane_metadata(&repo, "%9", &L2ParentMetadata {
            session_id: Some("child-1".to_owned()), l1_oracle: Some("gale".to_owned()), l1_session: Some("parent-1".to_owned()), l2_pane: Some("%9".to_owned()), repo: Some("maw-rs".to_owned()), ..L2ParentMetadata::default()
        })
        .expect("pane metadata");

        assert!(l2_emit_state(&repo, "%9", L2TerminalState::Idle, None).expect("first idle"));
        l2_observe_active(&repo, "%9").expect("idle became active");
        assert!(l2_emit_state(&repo, "%9", L2TerminalState::Idle, None).expect("second idle"));
        l2_observe_active(&repo, "%9").expect("idle became active again");
        assert!(l2_emit_state(&repo, "%9", L2TerminalState::Blocked, None).expect("first blocked"));
        l2_observe_active(&repo, "%9").expect("blocked became active");
        assert!(l2_emit_state(&repo, "%9", L2TerminalState::Blocked, None).expect("second blocked"));

        let events = l2_drain_events().expect("all reentered states queued");
        assert_eq!(events.iter().map(|event| event.state).collect::<Vec<_>>(), [L2TerminalState::Idle, L2TerminalState::Idle, L2TerminalState::Blocked, L2TerminalState::Blocked]);
        assert_eq!(events.iter().map(|event| event.transition_seq).collect::<Vec<_>>(), [1, 2, 3, 4]);
        std::fs::remove_dir_all(root).expect("cleanup root");
    }

    #[test]
    fn swarm_pr_events_keep_each_pane_scoped_identity() {
        let _guard = env_test_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let _state = EnvVarRestore::capture("MAW_STATE_DIR");
        let _pane = EnvVarRestore::capture("MAW_L2_PANE_ID");
        let _oracle = EnvVarRestore::capture("MAW_ORACLE");
        let root = std::env::temp_dir().join(format!("maw-rs-l2-swarm-pr-{}", std::process::id()));
        let repo = root.join("maw-rs");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(repo.join(".maw")).expect("repo metadata");
        std::env::set_var("MAW_STATE_DIR", root.join("state"));
        std::env::set_var("MAW_ORACLE", "gale");
        std::fs::write(repo.join(".maw/delivery.json"), r#"{"issue":99,"mode":"standard","riskTags":["api"],"engine":"codex"}"#).expect("delivery");
        for (pane, session) in [("%11", "member-one"), ("%12", "member-two")] {
            l2_record_pane_metadata(&repo, pane, &L2ParentMetadata {
                session_id: Some(session.to_owned()), l1_oracle: Some("gale".to_owned()), l1_session: Some("parent-1".to_owned()), l2_pane: Some(pane.to_owned()), repo: Some("maw-rs".to_owned()), ..L2ParentMetadata::default()
            })
            .expect("pane metadata");
        }

        std::env::set_var("MAW_L2_PANE_ID", "%12");
        assert!(l2_emit_pr_event(&repo, 102, "https://github.com/acme/maw-rs/pull/102").expect("member two PR"));
        std::env::set_var("MAW_L2_PANE_ID", "%11");
        assert!(l2_emit_pr_event(&repo, 101, "https://github.com/acme/maw-rs/pull/101").expect("member one PR"));

        let events = l2_drain_events().expect("two PR handoffs");
        assert_eq!(events.iter().map(|event| event.l2_session.as_str()).collect::<std::collections::BTreeSet<_>>(), std::collections::BTreeSet::from(["member-one", "member-two"]));
        assert_eq!(events.iter().map(|event| event.l2_pane.as_str()).collect::<std::collections::BTreeSet<_>>(), std::collections::BTreeSet::from(["%11", "%12"]));
        std::fs::remove_dir_all(root).expect("cleanup root");
    }

    #[test]
    fn observer_ownership_is_idempotent_and_arm_failure_releases_it() {
        let root = std::env::temp_dir().join(format!("maw-rs-l2-observer-owner-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".maw")).expect("metadata directory");
        let before = l2_test_arm_count();
        l2_prepare_observer(&root, "%9", "gale", Some("parent-1"), None).expect("first arm");
        l2_prepare_observer(&root, "%9", "gale", Some("parent-1"), None).expect("idempotent rearm");
        assert_eq!(l2_test_arm_count(), before + 1);

        l2_test_fail_next_arm();
        assert!(l2_prepare_observer(&root, "%10", "gale", Some("parent-1"), None).is_err());
        let owner = std::fs::read_to_string(l2_observer_owner_path(&root, "%10")).expect("released ownership record");
        assert!(owner.contains("\"active\":false"), "{owner}");
        std::fs::remove_dir_all(root).expect("cleanup root");
    }

    #[test]
    fn events_replay_when_the_hook_fails_after_selecting_them() {
        let _guard = env_test_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let _state = EnvVarRestore::capture("MAW_STATE_DIR");
        let _oracle = EnvVarRestore::capture("MAW_ORACLE");
        let root = std::env::temp_dir().join(format!("maw-rs-l2-replay-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("state root");
        std::env::set_var("MAW_STATE_DIR", &root);
        std::env::set_var("MAW_ORACLE", "gale");
        let event = L2Event {
            version: 1, l2_pane: "%9".to_owned(), l2_session: "child-1".to_owned(), l1_oracle: "gale".to_owned(), l1_session: "parent-1".to_owned(), repo: "maw-rs".to_owned(), issue: 99, state: L2TerminalState::Findings, transition_seq: 1, message: "replay after failed hook".to_owned(), notified: false, notified_at: None, created_at: "2026-07-23T00:00:00.000Z".to_owned(),
        };
        assert!(l2_enqueue_event(&event).expect("enqueue"));
        let interrupted = l2_drain_events().expect("hook selects event before later failure");
        assert_eq!(interrupted.len(), 1);
        let replay = l2_drain_events().expect("next hook replays event");
        assert_eq!(replay.iter().map(|event| event.message.as_str()).collect::<Vec<_>>(), ["replay after failed hook"]);
        l2_acknowledge_events(&replay).expect("successful hook acknowledges banner");
        assert!(l2_drain_events().expect("ack removes replay").is_empty());
        std::fs::remove_dir_all(root).expect("cleanup root");
    }

    #[test]
    fn findings_exit_and_idle_surface_once_through_hook_banner() {
        let _guard = env_test_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let _state = EnvVarRestore::capture("MAW_STATE_DIR");
        let _oracle = EnvVarRestore::capture("MAW_ORACLE");
        let root = std::env::temp_dir().join(format!("maw-rs-l2-hook-{}", std::process::id()));
        let repo = root.join("maw-rs");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(repo.join(".maw")).expect("repo metadata");
        std::env::set_var("MAW_STATE_DIR", root.join("state"));
        std::env::set_var("MAW_ORACLE", "gale");
        std::fs::write(repo.join(".maw/delivery.json"), r#"{"issue":99,"mode":"standard","riskTags":["api"],"engine":"codex"}"#).expect("delivery");
        l2_record_parent_metadata(&repo, &L2ParentMetadata {
            session_id: Some("child-1".to_owned()),
            l1_oracle: Some("gale".to_owned()),
            l1_session: Some("parent-1".to_owned()),
            l2_pane: Some("%9".to_owned()),
            repo: Some("maw-rs".to_owned()),
            ..L2ParentMetadata::default()
        }).expect("metadata");
        let cases = [
            L2Snapshot { alive: true, command: "codex", text: "FINDINGS issue #99", unchanged_secs: 0 },
            L2Snapshot { alive: false, command: "codex", text: "", unchanged_secs: 0 },
            L2Snapshot { alive: true, command: "codex", text: ">", unchanged_secs: 180 },
        ];
        for snapshot in cases {
            let state = l2_classify_snapshot(&snapshot, 180).expect("terminal state");
            assert!(l2_emit_state(&repo, "%9", state, None).expect("emit state"));
            assert!(!l2_emit_state(&repo, "%9", state, None).expect("dedupe same transition"));
        }
        let (_, banner) = wind_pr_queue_run(&[]).expect("hook drain");
        for kind in ["FINDINGS", "EXITED", "IDLE"] {
            assert_eq!(banner.matches(&format!("] {kind} issue")).count(), 1, "{banner}");
        }
        let (_, second) = wind_pr_queue_run(&[]).expect("second hook drain");
        assert!(!second.contains("L2 Events"), "{second}");
        std::fs::remove_dir_all(root).expect("cleanup root");
    }
}
