#![forbid(unsafe_code)]

use std::fmt::Write as _;
use std::time::Duration;

const RRR_WAIT_MAX_POLLS: usize = 16;
const RRR_WAIT_INTERVAL: Duration = Duration::from_secs(2);

/// Build the self-invocation error for `maw done <own-window-name>`.
#[must_use]
pub fn self_invocation_message(
    current: Option<&(String, i32)>,
    target_session: &str,
    target_index: i32,
    target_name: &str,
) -> Option<String> {
    let (current_session, current_index) = current?;
    (current_session == target_session && *current_index == target_index).then(|| {
        format!("refusing to done current window '{target_name}' in session '{target_session}'")
    })
}

/// Wait until the target pane capture shows an agent/shell prompt again.
pub fn wait_for_retrospective_prompt<C, S>(capture: C, sleep: S, stdout: &mut String)
where
    C: FnMut() -> Result<String, String>,
    S: FnMut(Duration),
{
    match wait_for_prompt_with(capture, sleep, RRR_WAIT_MAX_POLLS, wait_interval()) {
        Ok(polls) => {
            let _ = writeln!(
                stdout,
                "  \x1b[32m✓\x1b[0m retrospective prompt returned after {polls} poll(s)"
            );
        }
        Err(error) => {
            let _ = writeln!(
                stdout,
                "  \x1b[33m⚠\x1b[0m retrospective completion unconfirmed: {error}"
            );
        }
    }
}

pub use super::done_rescue::rescue_psi;

fn wait_interval() -> Duration {
    std::env::var("MAW_DONE_RRR_WAIT_INTERVAL_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map_or(RRR_WAIT_INTERVAL, Duration::from_millis)
}

fn wait_for_prompt_with<C, S>(
    mut capture: C,
    mut sleep: S,
    max_polls: usize,
    interval: Duration,
) -> Result<usize, String>
where
    C: FnMut() -> Result<String, String>,
    S: FnMut(Duration),
{
    if max_polls == 0 {
        return Err("no prompt polls configured".to_owned());
    }
    for poll in 1..=max_polls {
        let content = capture()?;
        if capture_has_prompt(&content) {
            return Ok(poll);
        }
        if poll < max_polls {
            sleep(interval);
        }
    }
    let polls = u64::try_from(max_polls).unwrap_or(u64::MAX);
    Err(format!(
        "prompt did not return within {}s",
        polls.saturating_mul(interval.as_secs())
    ))
}

fn capture_has_prompt(content: &str) -> bool {
    content
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .is_some_and(line_looks_like_prompt)
}

fn line_looks_like_prompt(line: &str) -> bool {
    let clean = maw_tmux::strip_tmux_ansi(line).replace('\r', "");
    let trimmed = clean.trim_end();
    if trimmed.is_empty() || trimmed.len() > 120 {
        return false;
    }
    matches!(trimmed, "$" | "#" | "%" | ">" | "❯" | "»")
        || trimmed.ends_with(['$', '#', '%', '>', '❯', '»'])
}
