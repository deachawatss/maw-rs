use std::time::Duration;

use super::{
    strip_tmux_ansi, SendTextReport, SendThrottle, TmuxClient, TmuxError, TmuxRunner,
    MAX_SUBMIT_ATTEMPTS, SEND_SETTLE_MS, SUBMIT_CONFIRM_MS,
};

pub const CODEX_SUBMIT_CONFIRM_MS: u64 = 200;
pub const CLAUDE_READINESS_TIMEOUT_MS: u64 = 45_000;
pub const CLAUDE_READINESS_POLL_MS: u64 = 1_000;
pub const CODEX_READINESS_TIMEOUT_MS: u64 = 8_000;
pub const CODEX_READINESS_POLL_MS: u64 = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmitEngine {
    Claude,
    Codex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubmitConfig {
    pub readiness_timeout_ms: u64,
    pub readiness_poll_ms: u64,
    pub confirm_interval_ms: u64,
}

impl SubmitConfig {
    #[must_use]
    pub const fn claude() -> Self {
        Self {
            readiness_timeout_ms: CLAUDE_READINESS_TIMEOUT_MS,
            readiness_poll_ms: CLAUDE_READINESS_POLL_MS,
            confirm_interval_ms: SUBMIT_CONFIRM_MS,
        }
    }

    #[must_use]
    pub const fn codex() -> Self {
        Self {
            readiness_timeout_ms: CODEX_READINESS_TIMEOUT_MS,
            readiness_poll_ms: CODEX_READINESS_POLL_MS,
            confirm_interval_ms: CODEX_SUBMIT_CONFIRM_MS,
        }
    }

    #[must_use]
    pub const fn for_engine(engine: SubmitEngine) -> Self {
        match engine {
            SubmitEngine::Claude => Self::claude(),
            SubmitEngine::Codex => Self::codex(),
        }
    }

    #[must_use]
    pub fn for_engine_name(name: &str) -> Self {
        if name.to_ascii_lowercase().contains("codex") {
            Self::codex()
        } else {
            Self::claude()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadinessResult {
    Ready,
    Timeout,
    Busy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadinessGate {
    config: SubmitConfig,
}

impl ReadinessGate {
    #[must_use]
    pub const fn new(config: SubmitConfig) -> Self {
        Self { config }
    }

    #[must_use]
    pub const fn config(self) -> SubmitConfig {
        self.config
    }
}

/// Smart text sending with Wind readiness and submit-confirm hardening.
///
/// # Errors
///
/// Returns the first tmux error from readiness polling, mode exit, text placement, paste, or
/// Enter send.
pub fn send_text_with_config_and_sleeper<R, F>(
    client: &mut TmuxClient<R>,
    target: &str,
    text: &str,
    config: SubmitConfig,
    mut sleep: F,
) -> Result<SendTextReport, TmuxError>
where
    R: TmuxRunner,
    F: FnMut(Duration),
{
    match readiness_gate(client, target, config, &mut sleep)? {
        ReadinessResult::Ready => send_text_body_with_sleeper(client, target, text, config, sleep),
        ReadinessResult::Timeout => Err(TmuxError::new(format!(
            "pane '{target}' did not show a prompt before readiness timeout"
        ))),
        ReadinessResult::Busy => Err(TmuxError::new(format!(
            "pane '{target}' is busy; prompt is not visible"
        ))),
    }
}

/// Send text without polling for prompt readiness first.
///
/// Use for oracle-to-oracle `maw hey` delivery where the target (Claude Code / Codex)
/// queues user input even while the model is generating.  Skips the readiness gate
/// but retains copy-mode exit, buffer/paste mechanics, and submit-confirm.
///
/// # Errors
///
/// Returns the first tmux error from mode exit, text placement, paste, or Enter send.
pub fn send_text_ungated_with_sleeper<R, F>(
    client: &mut TmuxClient<R>,
    target: &str,
    text: &str,
    config: SubmitConfig,
    sleep: F,
) -> Result<SendTextReport, TmuxError>
where
    R: TmuxRunner,
    F: FnMut(Duration),
{
    send_text_body_with_sleeper(client, target, text, config, sleep)
}

/// Poll a pane until its last non-empty captured line shows a prompt.
///
/// # Errors
///
/// Returns the runner error when tmux cannot capture the target pane.
pub fn readiness_gate<R, F>(
    client: &mut TmuxClient<R>,
    target: &str,
    config: SubmitConfig,
    mut sleep: F,
) -> Result<ReadinessResult, TmuxError>
where
    R: TmuxRunner,
    F: FnMut(Duration),
{
    let mut waited_ms = 0;
    let gate = ReadinessGate::new(config);
    let config = gate.config();
    let poll_ms = config.readiness_poll_ms.max(1);
    loop {
        let content = client.capture(target, Some(5))?;
        if pane_prompt_ready_from_capture(&content) {
            return Ok(ReadinessResult::Ready);
        }
        if waited_ms >= config.readiness_timeout_ms {
            return Ok(if pane_capture_has_active_output(&content) {
                ReadinessResult::Busy
            } else {
                ReadinessResult::Timeout
            });
        }
        let remaining_ms = config.readiness_timeout_ms - waited_ms;
        let sleep_ms = poll_ms.min(remaining_ms);
        sleep(Duration::from_millis(sleep_ms));
        waited_ms += sleep_ms;
    }
}

/// Probe the current pane capture once and return a send throttle when no prompt is visible.
///
/// # Errors
///
/// Returns the runner error when tmux cannot capture the target pane.
pub fn busy_probe<R>(client: &mut TmuxClient<R>, target: &str) -> Result<SendThrottle, TmuxError>
where
    R: TmuxRunner,
{
    let content = client.capture(target, Some(5))?;
    if pane_capture_has_active_output(&content) {
        Ok(SendThrottle::Busy)
    } else {
        Ok(SendThrottle::Allowed)
    }
}

/// Infer submit timing from the target pane command.
pub fn submit_config_for_target<R>(client: &mut TmuxClient<R>, target: &str) -> SubmitConfig
where
    R: TmuxRunner,
{
    client.display_pane_current_command(target).map_or_else(
        |_| SubmitConfig::claude(),
        |command| SubmitConfig::for_engine_name(&command),
    )
}

pub(super) fn send_text_body_with_sleeper<R, F>(
    client: &mut TmuxClient<R>,
    target: &str,
    text: &str,
    config: SubmitConfig,
    mut sleep: F,
) -> Result<SendTextReport, TmuxError>
where
    R: TmuxRunner,
    F: FnMut(Duration),
{
    client.exit_mode_if_needed(target)?;
    let used_buffer = text.contains('\n') || text.len() > 500;
    if used_buffer {
        client.load_buffer(text)?;
        client.paste_buffer(target)?;
    } else {
        client.send_keys_literal(target, text)?;
    }
    sleep(Duration::from_millis(SEND_SETTLE_MS));
    let (enter_attempts, warned_pending) =
        submit_with_confirm_config(client, target, &mut sleep, config)?;
    Ok(SendTextReport {
        used_buffer,
        enter_attempts,
        warned_pending,
    })
}

fn submit_with_confirm_config<R, F>(
    client: &mut TmuxClient<R>,
    target: &str,
    sleep: &mut F,
    config: SubmitConfig,
) -> Result<(u32, bool), TmuxError>
where
    R: TmuxRunner,
    F: FnMut(Duration),
{
    for attempt in 1..=MAX_SUBMIT_ATTEMPTS {
        client.send_enter(target)?;
        sleep(Duration::from_millis(config.confirm_interval_ms));
        if !client.pane_input_pending(target) {
            return Ok((attempt, false));
        }
    }
    Ok((MAX_SUBMIT_ATTEMPTS, true))
}

fn pane_prompt_ready_from_capture(content: &str) -> bool {
    last_clean_non_empty_capture_line(content).is_some_and(|line| {
        let trimmed = line.trim_end();
        trimmed.ends_with('$') || trimmed.ends_with('❯') || trimmed.ends_with('>')
    })
}

fn pane_capture_has_active_output(content: &str) -> bool {
    last_clean_non_empty_capture_line(content)
        .is_some_and(|line| !pane_prompt_ready_from_capture(line.as_str()))
}

fn last_clean_non_empty_capture_line(content: &str) -> Option<String> {
    content
        .lines()
        .rfind(|line| !line.trim().is_empty())
        .map(|line| strip_tmux_ansi(line).replace('\r', ""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Default)]
    struct UngatedRunner {
        calls: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    }

    impl TmuxRunner for UngatedRunner {
        fn run(&mut self, subcommand: &str, _args: &[String]) -> Result<String, TmuxError> {
            self.calls
                .lock()
                .expect("calls lock")
                .push(subcommand.to_owned());
            if subcommand == "capture-pane" {
                Ok("$".to_owned())
            } else {
                Ok(String::new())
            }
        }
    }

    #[test]
    fn hey_delivery_is_ungated_before_text_is_sent() {
        let runner = UngatedRunner::default();
        let calls = std::sync::Arc::clone(&runner.calls);
        let mut client = TmuxClient::new(runner);

        send_text_ungated_with_sleeper(
            &mut client,
            "%7",
            "[gale] hello",
            SubmitConfig::claude(),
            |_| {},
        )
        .expect("ungated delivery");

        let calls = calls.lock().expect("calls lock");
        let literal_send = calls
            .iter()
            .position(|call| call == "send-keys")
            .expect("literal text send");
        let confirmation_capture = calls
            .iter()
            .position(|call| call == "capture-pane")
            .expect("post-submit confirmation capture");
        assert!(
            literal_send < confirmation_capture,
            "ungated delivery must not readiness-poll before sending: {calls:?}"
        );
    }
}
