#![forbid(unsafe_code)]

use std::{cell::RefCell, collections::VecDeque, rc::Rc, time::Duration};

use maw_tmux::{
    SendTextReport, SendThrottle, SubmitConfig, TmuxClient, TmuxError, TmuxRunner,
    CODEX_SUBMIT_CONFIRM_MS, SEND_SETTLE_MS, SUBMIT_CONFIRM_MS,
};

#[derive(Clone, Default)]
struct SharedRunner {
    state: Rc<RefCell<RunnerState>>,
}

#[derive(Default)]
struct RunnerState {
    calls: Vec<(String, Vec<String>)>,
    stdin_calls: Vec<(String, Vec<String>, String)>,
    responses: VecDeque<Result<String, TmuxError>>,
}

impl SharedRunner {
    fn with_responses(responses: Vec<Result<&str, TmuxError>>) -> Self {
        Self {
            state: Rc::new(RefCell::new(RunnerState {
                calls: Vec::new(),
                stdin_calls: Vec::new(),
                responses: responses
                    .into_iter()
                    .map(|response| response.map(str::to_owned))
                    .collect(),
            })),
        }
    }
}

impl TmuxRunner for SharedRunner {
    fn run(&mut self, subcommand: &str, args: &[String]) -> Result<String, TmuxError> {
        let mut state = self.state.borrow_mut();
        state.calls.push((subcommand.to_owned(), args.to_vec()));
        state
            .responses
            .pop_front()
            .unwrap_or_else(|| Err(TmuxError::new(format!("no response for {subcommand}"))))
    }

    fn run_with_stdin(
        &mut self,
        subcommand: &str,
        args: &[String],
        stdin: &[u8],
    ) -> Result<String, TmuxError> {
        let mut state = self.state.borrow_mut();
        state.stdin_calls.push((
            subcommand.to_owned(),
            args.to_vec(),
            String::from_utf8_lossy(stdin).into_owned(),
        ));
        state
            .responses
            .pop_front()
            .unwrap_or_else(|| Err(TmuxError::new(format!("no response for {subcommand}"))))
    }
}

#[test]
fn readiness_gate_polls_until_prompt_visible() {
    let runner = SharedRunner::with_responses(vec![Ok("processing...\n"), Ok("$ \r")]);
    let state = Rc::clone(&runner.state);
    let mut client = TmuxClient::new(runner);
    let mut sleeps = Vec::new();

    let result = client
        .readiness_gate(
            "%1",
            SubmitConfig {
                readiness_timeout_ms: 1_000,
                readiness_poll_ms: 25,
                confirm_interval_ms: SUBMIT_CONFIRM_MS,
            },
            |duration| sleeps.push(duration),
        )
        .expect("readiness gate succeeds");

    assert_eq!(result, maw_tmux::ReadinessResult::Ready);
    assert_eq!(sleeps, vec![Duration::from_millis(25)]);
    let calls = &state.borrow().calls;
    assert_eq!(calls.len(), 2);
    assert!(calls
        .iter()
        .all(|(subcommand, _args)| subcommand == "capture-pane"));
}

#[test]
fn busy_guard_blocks_send_during_active_output() {
    let runner = SharedRunner::with_responses(vec![Ok("thinking...\nwriting answer\n")]);
    let state = Rc::clone(&runner.state);
    let mut client = TmuxClient::new(runner);

    let throttle = client.busy_guard("%2").expect("busy guard captures pane");

    assert_eq!(throttle, SendThrottle::Busy);
    let calls = &state.borrow().calls;
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "capture-pane");

    let runner = SharedRunner::with_responses(vec![Ok("thinking...\nwriting answer\n")]);
    let state = Rc::clone(&runner.state);
    let mut client = TmuxClient::new(runner);
    let error = client
        .send_text_with_config_and_sleeper(
            "%2",
            "hello",
            SubmitConfig {
                readiness_timeout_ms: 0,
                readiness_poll_ms: 1,
                confirm_interval_ms: SUBMIT_CONFIRM_MS,
            },
            |_| {},
        )
        .expect_err("busy pane blocks text submission");

    assert!(error.message.contains("busy"));
    let calls = &state.borrow().calls;
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "capture-pane");
    assert!(!calls
        .iter()
        .any(|(subcommand, _args)| subcommand == "send-keys"));
}

#[test]
fn verify_submit_retries_with_engine_specific_intervals() {
    let claude_sleeps = submit_sleeps_for(SubmitConfig::claude());
    assert_eq!(
        claude_sleeps,
        vec![
            Duration::from_millis(SEND_SETTLE_MS),
            Duration::from_millis(SUBMIT_CONFIRM_MS),
            Duration::from_millis(SUBMIT_CONFIRM_MS),
        ]
    );

    let codex_sleeps = submit_sleeps_for(SubmitConfig::codex());
    assert_eq!(
        codex_sleeps,
        vec![
            Duration::from_millis(SEND_SETTLE_MS),
            Duration::from_millis(CODEX_SUBMIT_CONFIRM_MS),
            Duration::from_millis(CODEX_SUBMIT_CONFIRM_MS),
        ]
    );
}

#[test]
fn fork_divergence_hook_keeps_wind_delivery_at_submit_site() {
    let source = include_str!("../src/core_impl/client_pane_send_parts/pane_text_send_methods.rs");

    assert!(source.contains("wind_delivery::submit_config_for_target(self, target)"));
    assert!(source.contains("wind_delivery::send_text_with_config_and_sleeper("));
    assert!(source.contains("wind_delivery::send_text_ungated_with_sleeper("));
    assert!(source.contains("wind_delivery::busy_probe(self, target)"));
}

#[test]
fn ungated_send_delivers_even_when_pane_is_busy() {
    let runner = SharedRunner::with_responses(vec![
        Ok("node"), // display-message: submit_config_for_target (pane_current_command)
        Ok("0"),    // display-message: exit_mode_if_needed (pane_in_mode)
        Ok(""),     // send-keys: literal text
        Ok(""),     // send-keys: Enter (submit attempt 1)
        Ok("$ \r"), // capture-pane: pane_input_pending (submit-confirm — cleared)
    ]);
    let state = Rc::clone(&runner.state);
    let mut client = TmuxClient::new(runner);

    let report = client
        .send_text_ungated("%busy", "hello from maw hey")
        .expect("ungated send succeeds on busy pane");

    assert!(!report.used_buffer);
    assert_eq!(report.enter_attempts, 1);
    let calls = &state.borrow().calls;
    let first_capture = calls.iter().position(|(cmd, _)| cmd == "capture-pane");
    let first_send = calls
        .iter()
        .position(|(cmd, args)| cmd == "send-keys" && args.iter().any(|a| a.contains("hello")));
    assert!(
        first_send.is_some_and(|s| first_capture.is_none_or(|c| s < c)),
        "ungated: text must be sent BEFORE any capture-pane (no readiness polling)"
    );
}

#[test]
fn gated_send_blocks_on_busy_but_ungated_succeeds() {
    let gated_runner = SharedRunner::with_responses(vec![
        Ok("thinking...\nwriting answer\n"), // capture-pane: readiness gate sees active output
    ]);
    let mut gated_client = TmuxClient::new(gated_runner);
    let gated_result = gated_client.send_text_with_config_and_sleeper(
        "%busy",
        "hello",
        SubmitConfig {
            readiness_timeout_ms: 0,
            readiness_poll_ms: 1,
            confirm_interval_ms: SUBMIT_CONFIRM_MS,
        },
        |_| {},
    );
    assert!(gated_result.is_err(), "gated send must fail on busy pane");

    let ungated_runner = SharedRunner::with_responses(vec![
        Ok("node"), // display-message: pane_current_command
        Ok("0"),    // display-message: pane_in_mode
        Ok(""),     // send-keys: literal text
        Ok(""),     // send-keys: Enter
        Ok("$ \r"), // capture-pane: submit-confirm
    ]);
    let mut ungated_client = TmuxClient::new(ungated_runner);
    let ungated_result = ungated_client.send_text_ungated("%busy", "hello");
    assert!(
        ungated_result.is_ok(),
        "ungated send must succeed regardless of pane state"
    );
}

fn submit_sleeps_for(config: SubmitConfig) -> Vec<Duration> {
    let runner = SharedRunner::with_responses(vec![
        Ok("$ \r"),
        Ok("0"),
        Ok(""),
        Ok(""),
        Ok("$ deploy"),
        Ok(""),
        Ok("$ \r"),
    ]);
    let mut client = TmuxClient::new(runner);
    let mut sleeps = Vec::new();

    let report = client
        .send_text_with_config_and_sleeper("sess:oracle.0", "deploy", config, |duration| {
            sleeps.push(duration);
        })
        .expect("send text succeeds");

    assert_eq!(
        report,
        SendTextReport {
            used_buffer: false,
            enter_attempts: 2,
            warned_pending: false,
        }
    );
    sleeps
}
