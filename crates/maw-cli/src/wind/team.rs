#![forbid(unsafe_code)]

use std::{collections::BTreeMap, path::Path};

/// Return the current tmux caller pane from `TMUX_PANE` when it is safe.
#[must_use]
pub fn caller_pane() -> Option<String> {
    std::env::var("TMUX_PANE")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| is_valid_pane_id(value))
}

/// Resolve the tmux `split-window -t` target for team worker pane creation.
#[must_use]
pub fn spawn_pane_target(caller_pane: Option<&str>) -> String {
    caller_pane
        .filter(|value| is_valid_pane_id(value))
        .map_or_else(|| ".".to_owned(), str::to_owned)
}

/// Send a persisted OMX spawn prompt into a tmux pane with the local runner.
///
/// # Errors
///
/// Returns an error when the pane id is unsafe, the prompt cannot be read, the
/// prompt is invalid, or tmux rejects the send.
pub fn omx_auto_kickoff(pane_id: &str, prompt_path: impl AsRef<Path>) -> Result<(), String> {
    let mut runner = maw_tmux::CommandTmuxRunner::default();
    omx_auto_kickoff_with(&mut runner, pane_id, prompt_path)
}

/// Send a persisted OMX spawn prompt through an injected tmux runner.
///
/// # Errors
///
/// Returns an error when the pane id is unsafe, the prompt cannot be read, the
/// prompt is invalid, or the injected runner rejects the send.
pub fn omx_auto_kickoff_with<R>(
    runner: &mut R,
    pane_id: &str,
    prompt_path: impl AsRef<Path>,
) -> Result<(), String>
where
    R: maw_tmux::TmuxRunner,
{
    validate_public_pane_id(pane_id)?;
    let path = prompt_path.as_ref();
    let prompt = std::fs::read_to_string(path)
        .map_err(|error| format!("team omx kickoff: read {} failed: {error}", path.display()))?;
    validate_kickoff_prompt(&prompt)?;
    runner
        .run(
            "send-keys",
            &maw_tmux::tmux_send_keys_literal_args(pane_id, &prompt),
        )
        .map_err(|error| format!("team omx kickoff: send prompt failed: {error}"))?;
    runner
        .run("send-keys", &maw_tmux::tmux_send_enter_args(pane_id))
        .map_err(|error| format!("team omx kickoff: send enter failed: {error}"))?;
    Ok(())
}

/// Return member pane ids whose tmux PID is missing or no longer alive.
#[must_use]
pub fn orphan_sweep_from_pids(
    member_pane_ids: &[String],
    pane_pids: &[(String, u32)],
    mut is_pid_alive: impl FnMut(u32) -> bool,
) -> Vec<String> {
    let pane_to_pid = pane_pids.iter().cloned().collect::<BTreeMap<_, _>>();
    member_pane_ids
        .iter()
        .filter(|pane_id| {
            pane_to_pid
                .get(*pane_id)
                .is_none_or(|pid| !is_pid_alive(*pid))
        })
        .cloned()
        .collect()
}

/// Return zombie pane ids for a team config file using live tmux pane PID data.
///
/// # Errors
///
/// Returns an error when the team config cannot be read, JSON cannot be parsed,
/// or tmux pane PID discovery fails.
pub fn orphan_sweep(config_path: &Path) -> Result<Vec<String>, String> {
    let member_panes = member_pane_ids(config_path)?;
    let pane_pids = pane_pids()?;
    Ok(orphan_sweep_from_pids(&member_panes, &pane_pids, pid_alive))
}

fn member_pane_ids(config_path: &Path) -> Result<Vec<String>, String> {
    let text = std::fs::read_to_string(config_path).map_err(|error| {
        format!(
            "team orphan sweep: read {} failed: {error}",
            config_path.display()
        )
    })?;
    let value: serde_json::Value = serde_json::from_str(&text).map_err(|error| {
        format!(
            "team orphan sweep: parse {} failed: {error}",
            config_path.display()
        )
    })?;
    let Some(members) = value.get("members").and_then(serde_json::Value::as_array) else {
        return Ok(Vec::new());
    };
    Ok(members
        .iter()
        .filter(|member| {
            member.get("agentType").and_then(serde_json::Value::as_str) != Some("team-lead")
        })
        .filter_map(|member| member.get("tmuxPaneId").and_then(serde_json::Value::as_str))
        .filter(|pane_id| is_valid_pane_id(pane_id))
        .map(str::to_owned)
        .collect())
}

fn pane_pids() -> Result<Vec<(String, u32)>, String> {
    if let Ok(raw) = std::env::var("MAW_RS_TEAM_PANE_PIDS") {
        return Ok(raw.lines().filter_map(parse_pane_pid).collect());
    }
    let mut runner = maw_tmux::CommandTmuxRunner::default();
    let raw = maw_tmux::TmuxRunner::run(
        &mut runner,
        "list-panes",
        &[
            "-a".to_owned(),
            "-F".to_owned(),
            "#{pane_id}|#{pane_pid}".to_owned(),
        ],
    )
    .map_err(|error| format!("team orphan sweep: list-panes failed: {error}"))?;
    Ok(raw.lines().filter_map(parse_pane_pid).collect())
}

fn parse_pane_pid(line: &str) -> Option<(String, u32)> {
    let (pane_id, pid) = line.split_once('|')?;
    if !is_valid_pane_id(pane_id) {
        return None;
    }
    Some((pane_id.to_owned(), pid.parse().ok()?))
}

fn pid_alive(pid: u32) -> bool {
    // Linux fast path: /proc/<pid> exists iff the process is alive.
    let proc_root = std::path::Path::new("/proc");
    if proc_root.is_dir() {
        return proc_root.join(pid.to_string()).exists();
    }
    // Portable fallback (macOS/BSD have no /proc): `kill -0 <pid>` performs
    // existence/permission checking without sending a signal — exit 0 iff the
    // process is alive. Without this, orphan-sweep flags every live pane as a
    // zombie on macOS (the primary fleet host).
    std::process::Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn validate_kickoff_prompt(prompt: &str) -> Result<(), String> {
    if prompt.is_empty() {
        return Err("team omx kickoff: prompt is empty".to_owned());
    }
    if prompt.chars().any(|ch| ch == '\0') {
        return Err("team omx kickoff: prompt contains NUL".to_owned());
    }
    Ok(())
}

fn validate_public_pane_id(pane_id: &str) -> Result<(), String> {
    if is_valid_pane_id(pane_id) {
        Ok(())
    } else {
        Err(format!(
            "invalid pane id {pane_id:?}: expected tmux %pane id"
        ))
    }
}

fn is_valid_pane_id(value: &str) -> bool {
    !value.is_empty()
        && value.starts_with('%')
        && !value.starts_with("%-")
        && !value
            .chars()
            .any(|ch| ch.is_whitespace() || ch.is_control() || ch == '\0')
}
