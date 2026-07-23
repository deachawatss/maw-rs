const DISPATCH_382: &[DispatcherEntry] = &[DispatcherEntry {
    command: "__l2-watch",
    handler: Handler::Sync(l2_watch_command),
}];
use maw_tmux::TmuxRunner as _;

const L2_WATCH_INTERVAL_SECS: u64 = 25;
const L2_IDLE_TIMEOUT_SECS: u64 = 180;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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

fn l2_prepare_observer(cwd: &Path, pane: &str, l1_oracle: &str, l1_session: Option<&str>) -> Result<(), String> {
    let metadata = L2ParentMetadata {
        parent_session_id: l1_session.map(str::to_owned),
        l1_oracle: Some(l1_oracle.to_owned()),
        l1_session: l1_session.map(str::to_owned).or_else(|| Some(l1_oracle.to_owned())),
        l2_pane: Some(pane.to_owned()),
        repo: cwd.file_name().and_then(std::ffi::OsStr::to_str).map(str::to_owned),
        ..L2ParentMetadata::default()
    };
    l2_record_parent_metadata(cwd, &metadata)?;
    l2_arm_observer(cwd, pane)
}

fn l2_arm_observer(cwd: &Path, pane: &str) -> Result<(), String> {
    if std::env::var("MAW_TEST_MODE").as_deref() == Ok("1") { return Ok(()); }
    let metadata = std::fs::read_to_string(cwd.join(".maw/l2-meta.json")).map_err(|error| format!("l2 observer: read metadata: {error}"))?;
    std::fs::write(l2_pane_metadata_path(cwd, pane), metadata).map_err(|error| format!("l2 observer: snapshot metadata: {error}"))?;
    let executable = std::env::current_exe().map_err(|error| format!("l2 observer: current executable: {error}"))?;
    std::process::Command::new(executable)
        .args(["__l2-watch", pane])
        .current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("l2 observer: spawn: {error}"))
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
    let config = merged_config_value_in_dir(&cwd);
    let interval = config.get("l2_watch_interval_secs").and_then(serde_json::Value::as_u64).unwrap_or(L2_WATCH_INTERVAL_SECS);
    let idle_timeout = config.get("l2_idle_timeout_secs").and_then(serde_json::Value::as_u64).unwrap_or(L2_IDLE_TIMEOUT_SECS);
    let mut runner = maw_tmux::CommandTmuxRunner::new();
    let mut previous = String::new();
    let mut unchanged_since = std::time::Instant::now();
    let mut last_state = None;
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
        if let Some(state) = l2_classify_snapshot(&snapshot, idle_timeout).filter(|state| Some(*state) != last_state) {
            l2_emit_state(&cwd, pane, state, None)?;
            last_state = Some(state);
            if matches!(state, L2TerminalState::Exited | L2TerminalState::Error | L2TerminalState::Pr | L2TerminalState::Findings | L2TerminalState::Ready) {
                return Ok(());
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
    let seq = l2_next_transition_seq(cwd)?;
    let message = format_l2_handoff(state.handoff_kind(), &l1_oracle, &repo, issue, &mode, &risk, body.unwrap_or_else(|| state.body()), &engine);
    l2_enqueue_event(&L2Event {
        version: 1,
        l2_pane: pane.to_owned(),
        l2_session: metadata.session_id.clone().unwrap_or_else(|| pane.to_owned()),
        l1_oracle: l1_oracle.clone(),
        l1_session: metadata.l1_session.or(metadata.parent_session_id).unwrap_or(l1_oracle),
        repo,
        issue,
        state,
        transition_seq: seq,
        message,
        notified: false,
        notified_at: None,
        created_at: cli_dispatch_now_iso(),
    })
}

fn l2_pane_metadata_path(cwd: &Path, pane: &str) -> std::path::PathBuf {
    let key = pane.chars().filter(char::is_ascii_alphanumeric).collect::<String>();
    cwd.join(".maw").join(format!("l2-meta-{key}.json"))
}

fn l2_emit_pr_event(cwd: &Path, pr_number: u64, url: &str) -> Result<bool, String> {
    let metadata_path = cwd.join(".maw/l2-meta.json");
    if !metadata_path.exists() {
        let l1_oracle = std::fs::read_to_string(cwd.join(".maw/l1-oracle")).ok().map(|value| value.trim().to_owned()).filter(|value| !value.is_empty());
        let pane = std::fs::read_to_string(cwd.join(".maw/pane-id")).ok().map(|value| value.trim().to_owned()).filter(|value| !value.is_empty()).unwrap_or_else(|| "unknown".to_owned());
        l2_record_parent_metadata(cwd, &L2ParentMetadata {
            l1_session: l1_oracle.clone(),
            l1_oracle,
            l2_pane: Some(pane),
            repo: cwd.file_name().and_then(std::ffi::OsStr::to_str).map(str::to_owned),
            ..L2ParentMetadata::default()
        })?;
    }
    let metadata = std::fs::read_to_string(metadata_path).map_err(|error| format!("l2 event: read PR metadata: {error}"))?;
    let metadata = serde_json::from_str::<L2ParentMetadata>(&metadata).map_err(|error| format!("l2 event: parse PR metadata: {error}"))?;
    let pane = metadata.l2_pane.as_deref().unwrap_or("unknown");
    l2_emit_state(cwd, pane, L2TerminalState::Pr, Some(&format!("PR #{pr_number} ready. {url}")))
}

fn l2_next_transition_seq(cwd: &Path) -> Result<u64, String> {
    let path = cwd.join(".maw/l2-transition-seq");
    let next = std::fs::read_to_string(&path).ok().and_then(|value| value.trim().parse::<u64>().ok()).unwrap_or(0).saturating_add(1);
    std::fs::write(&path, format!("{next}\n")).map_err(|error| format!("l2 event: write transition sequence: {error}"))?;
    Ok(next)
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

fn l2_drain_events() -> Result<Vec<String>, String> {
    let root = pr_review_queue_root()?;
    let _lock = PrQueueLock::acquire(&root)?;
    let current_oracle = std::env::var("MAW_ORACLE").ok().or_else(l2_current_tmux_session);
    let current_session = ["MAW_SESSION_ID", "CLAUDE_SESSION_ID"].iter().find_map(|key| std::env::var(key).ok());
    let mut retained = Vec::new();
    let mut archived = pr_read_queue_lines(&root, "l2-events.jsonl.archived")?;
    let mut messages = Vec::new();
    for line in pr_read_queue_lines(&root, "l2-events.jsonl")? {
        let Ok(mut event) = serde_json::from_str::<L2Event>(&line) else { retained.push(line); continue };
        let targets_l1 = current_oracle.as_ref().is_some_and(|value| value == &event.l1_oracle)
            || current_session.as_ref().is_some_and(|value| value == &event.l1_session);
        if event.notified || !targets_l1 { retained.push(line); continue; }
        event.notified = true;
        event.notified_at = Some(cli_dispatch_now_iso());
        messages.push(event.message.clone());
        archived.push(serde_json::to_string(&event).map_err(|error| format!("l2 event: archive render: {error}"))?);
    }
    pr_write_queue_lines(&root, "l2-events.jsonl", &retained)?;
    pr_write_queue_lines(&root, "l2-events.jsonl.archived", &archived)?;
    Ok(messages)
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
        assert_eq!(l2_drain_events().expect("first drain"), ["one durable handoff"]);
        assert!(l2_drain_events().expect("second drain").is_empty());
        assert!(std::fs::read_to_string(root.join("l2-events.jsonl")).expect("active queue").is_empty());
        let archived = std::fs::read_to_string(root.join("l2-events.jsonl.archived")).expect("archive");
        assert_eq!(archived.lines().count(), 1);
        assert!(archived.contains("\"notified\":true"));
        std::fs::remove_dir_all(root).expect("cleanup state");
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
