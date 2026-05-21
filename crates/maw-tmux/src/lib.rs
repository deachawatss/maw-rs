//! Testable tmux command and parser adapter for maw-rs.
//!
//! This crate ports the deterministic parts of maw-js `src/core/transport/tmux-class.ts`:
//! shell-safe command construction plus parsing of `list-windows` / `list-panes` output.
//! Real process execution is intentionally injected through [`TmuxRunner`].

use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    ffi::OsString,
    fmt,
    io::Write,
    process::{Command, Stdio},
};

use maw_matcher::{resolve_by_name, Named, ResolveOptions, ResolveResult};

const DEFAULT_CAPTURE_LINES: u32 = 80;
const DEFAULT_PTY_COLS_LIMIT: u32 = 500;
const DEFAULT_PTY_ROWS_LIMIT: u32 = 200;
const MAX_SUBMIT_ATTEMPTS: u32 = 4;
const COOLDOWN_MS: u64 = 500;
const QUOTA_PER_MINUTE: u32 = 100;
const QUOTA_WINDOW_MS: u64 = 60_000;

const VALID_LAYOUTS: [&str; 5] = [
    "even-horizontal",
    "even-vertical",
    "main-horizontal",
    "main-vertical",
    "tiled",
];

/// Tmux format used by maw-js pane target fallback resolution.
pub const PANE_TARGET_FORMAT: &str =
    "#{pane_id}|||#{session_name}:#{window_index}.#{pane_index}|||#{pane_title}|||#{@maw_tile_role}|||#{pane_current_path}";

/// Tmux window metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TmuxWindow {
    pub index: u32,
    pub name: String,
    pub active: bool,
    pub cwd: Option<String>,
}

/// Tmux session metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TmuxSession {
    pub name: String,
    pub windows: Vec<TmuxWindow>,
}

/// Tmux pane metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TmuxPane {
    pub id: String,
    pub command: String,
    pub target: String,
    pub title: String,
    pub pid: Option<u32>,
    pub cwd: Option<String>,
    pub last_activity: Option<u64>,
}

/// Options for creating a tmux session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSessionOptions {
    pub window: Option<String>,
    pub cwd: Option<String>,
    pub detached: bool,
    pub command: Option<String>,
    pub print_format: Option<String>,
}

impl Default for NewSessionOptions {
    fn default() -> Self {
        Self {
            window: None,
            cwd: None,
            detached: true,
            command: None,
            print_format: None,
        }
    }
}

/// Options for creating a grouped tmux session.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GroupedSessionOptions {
    pub cols: Option<u32>,
    pub rows: Option<u32>,
    pub window: Option<String>,
    pub window_size: Option<String>,
}

/// Options for creating a tmux pane split.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SplitWindowOptions {
    pub cwd: Option<String>,
    pub command: Option<String>,
    pub print_format: Option<String>,
}

/// Options for selecting a tmux pane.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SelectPaneOptions {
    pub title: Option<String>,
}

/// Outcome from maw-js-style smart text submission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendTextReport {
    pub used_buffer: bool,
    pub enter_attempts: u32,
    pub warned_pending: bool,
}

/// Options for lock-protected `split-window` construction.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SplitWindowLockedOptions {
    pub vertical: Option<bool>,
    pub pct: Option<u32>,
    pub shell_command: Option<String>,
}

/// Pane tags: title plus tmux `@custom` options.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PaneTags {
    pub title: String,
    pub meta: BTreeMap<String, String>,
}

/// Minimal pane shape used by `maw tmux ls` annotation logic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TmuxLsPaneRef {
    pub id: String,
    pub target: String,
    pub command: Option<String>,
}

/// Result of tmux send destructive-command safety scanning.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DestructiveCheck {
    pub destructive: bool,
    pub reasons: Vec<String>,
}

/// Options for Rust's maw-js-compatible `maw tmux send` action wrapper.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TmuxSendCommandOptions {
    pub literal: bool,
    pub allow_destructive: bool,
    pub force: bool,
}

/// Options for Rust's maw-js-compatible `maw tmux split` action wrapper.
#[derive(Debug, Clone, PartialEq)]
pub struct TmuxSplitActionOptions {
    pub vertical: bool,
    pub pct: f64,
    pub command: Option<String>,
}

impl Default for TmuxSplitActionOptions {
    fn default() -> Self {
        Self {
            vertical: false,
            pct: 50.0,
            command: None,
        }
    }
}

/// Per-pane heartbeat throttle state for `maw tmux send`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SendTrackerEntry {
    pub last_ts: u64,
    pub count: u32,
    pub window_start: u64,
}

/// Send throttle outcome before tmux mutation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendThrottle {
    Allowed,
    Cooldown { cooldown_ms: u64 },
    Quota { quota_per_minute: u32 },
}

/// In-memory cooldown + quota tracker ported from maw-js `_sendTracker`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TmuxSendTracker {
    entries: BTreeMap<String, SendTrackerEntry>,
}

impl TmuxSendTracker {
    /// Return the current entry for tests/diagnostics.
    #[must_use]
    pub fn get(&self, resolved: &str) -> Option<SendTrackerEntry> {
        self.entries.get(resolved).copied()
    }

    /// Insert or replace a tracker entry for tests/recovery.
    pub fn set(&mut self, resolved: impl Into<String>, entry: SendTrackerEntry) {
        self.entries.insert(resolved.into(), entry);
    }

    /// Clear all tracker entries.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Apply maw-js heartbeat cooldown and quota gates.
    ///
    /// `force` bypasses the tracker and does not mutate it, matching the JavaScript action.
    pub fn check(&mut self, resolved: &str, now_ms: u64, force: bool) -> SendThrottle {
        if force {
            return SendThrottle::Allowed;
        }
        let Some(prev) = self.entries.get_mut(resolved) else {
            self.entries.insert(
                resolved.to_owned(),
                SendTrackerEntry {
                    last_ts: now_ms,
                    count: 1,
                    window_start: now_ms,
                },
            );
            return SendThrottle::Allowed;
        };
        if now_ms.saturating_sub(prev.last_ts) < COOLDOWN_MS {
            return SendThrottle::Cooldown {
                cooldown_ms: COOLDOWN_MS,
            };
        }
        if now_ms.saturating_sub(prev.window_start) > QUOTA_WINDOW_MS {
            prev.count = 0;
            prev.window_start = now_ms;
        }
        if prev.count >= QUOTA_PER_MINUTE {
            return SendThrottle::Quota {
                quota_per_minute: QUOTA_PER_MINUTE,
            };
        }
        prev.last_ts = now_ms;
        prev.count += 1;
        SendThrottle::Allowed
    }
}

/// Outcome from a high-level `maw tmux send` action attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TmuxSendCommandOutcome {
    Sent,
    Throttled(SendThrottle),
}

/// Execution action selected by maw-js `maw tmux attach`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TmuxAttachAction {
    Print { session: String },
    SwitchClient { session: String },
    Attach { session: String },
    Recover { session: String },
}

/// Spawn command selected by `cmdTmuxAttach` or its recovery path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnCommand {
    pub program: String,
    pub args: Vec<String>,
}

/// Candidate shown by maw-js attach recovery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachRecoveryCandidate {
    pub oracle: String,
    pub label: String,
}

/// Fleet entry fragment used to seed attach recovery candidates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachRecoveryFleetEntry {
    pub session: String,
    pub first_window_name: Option<String>,
    pub repo: Option<String>,
}

/// Pure attach recovery decision after candidate construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttachRecoveryDecision {
    NoCandidates,
    AutoWake {
        command: SpawnCommand,
        label: String,
    },
    PrintCandidates {
        candidates: Vec<AttachRecoveryCandidate>,
    },
    Prompt {
        candidates: Vec<AttachRecoveryCandidate>,
    },
    WakeChoice {
        command: SpawnCommand,
    },
    InvalidChoice,
}

/// Options for Rust's maw-js-compatible `maw tmux kill` action wrapper.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TmuxKillCommandOptions {
    pub force: bool,
    pub session: bool,
}

/// Target plus source metadata after kill fallback resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TmuxKillTarget {
    pub resolved: String,
    pub source: String,
}

/// Successful tmux kill operation kind and concrete target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TmuxKillOutcome {
    Pane { target: String },
    Session { session: String },
}

/// Candidate name that can resolve to a live tmux pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneTargetCandidate {
    pub name: String,
    pub resolved: String,
    pub source: String,
    pub target: String,
}

impl Named for PaneTargetCandidate {
    fn name(&self) -> &str {
        &self.name
    }
}

/// Resolution result for orphan pane kill fallback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaneTargetResolution {
    None,
    Match {
        candidate: PaneTargetCandidate,
    },
    Ambiguous {
        candidates: Vec<PaneTargetCandidate>,
    },
}

/// Error returned by an injected tmux runner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TmuxError {
    pub message: String,
}

impl TmuxError {
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for TmuxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for TmuxError {}

/// Injectable tmux execution seam.
pub trait TmuxRunner {
    /// Run `tmux <subcommand> <args...>` and return stdout.
    ///
    /// # Errors
    ///
    /// Returns [`TmuxError`] when tmux exits non-zero or the host command cannot be executed.
    fn run(&mut self, subcommand: &str, args: &[String]) -> Result<String, TmuxError>;

    /// Run `tmux <subcommand> <args...>` with stdin.
    ///
    /// # Errors
    ///
    /// Returns [`TmuxError`] when the runner does not support stdin or tmux execution fails.
    fn run_with_stdin(
        &mut self,
        subcommand: &str,
        args: &[String],
        _stdin: &[u8],
    ) -> Result<String, TmuxError> {
        self.run(subcommand, args)
    }
}

/// Concrete tmux runner backed by `std::process::Command`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandTmuxRunner {
    program: OsString,
    socket: Option<OsString>,
}

impl Default for CommandTmuxRunner {
    fn default() -> Self {
        Self {
            program: OsString::from("tmux"),
            socket: None,
        }
    }
}

impl CommandTmuxRunner {
    /// Create a runner that invokes the default `tmux` binary.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a runner that invokes a custom tmux-compatible program.
    #[must_use]
    pub fn with_program(program: impl Into<OsString>) -> Self {
        Self {
            program: program.into(),
            socket: None,
        }
    }

    /// Set the tmux socket passed as `-S <socket>`.
    #[must_use]
    pub fn with_socket(mut self, socket: impl Into<OsString>) -> Self {
        self.socket = Some(socket.into());
        self
    }

    /// Return the exact argv vector this runner will execute.
    ///
    /// This keeps runtime command construction testable without requiring a live tmux server.
    #[must_use]
    pub fn argv(&self, subcommand: &str, tmux_args: &[String]) -> Vec<OsString> {
        let mut command_line = vec![self.program.clone()];
        if let Some(socket) = &self.socket {
            command_line.push(OsString::from("-S"));
            command_line.push(socket.clone());
        }
        command_line.push(OsString::from(subcommand));
        command_line.extend(tmux_args.iter().map(OsString::from));
        command_line
    }
}

impl TmuxRunner for CommandTmuxRunner {
    fn run(&mut self, subcommand: &str, args: &[String]) -> Result<String, TmuxError> {
        self.run_command(subcommand, args, None)
    }

    fn run_with_stdin(
        &mut self,
        subcommand: &str,
        args: &[String],
        stdin: &[u8],
    ) -> Result<String, TmuxError> {
        self.run_command(subcommand, args, Some(stdin))
    }
}

impl CommandTmuxRunner {
    fn run_command(
        &self,
        subcommand: &str,
        args: &[String],
        stdin: Option<&[u8]>,
    ) -> Result<String, TmuxError> {
        let command_line = self.argv(subcommand, args);
        let Some((program, rest)) = command_line.split_first() else {
            return Err(TmuxError::new("missing tmux program"));
        };
        let mut command = Command::new(program);
        command.args(rest);
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        if stdin.is_some() {
            command.stdin(Stdio::piped());
        }
        let mut child = command.spawn().map_err(|error| {
            TmuxError::new(format!(
                "failed to execute {}: {error}",
                program.to_string_lossy()
            ))
        })?;
        if let Some(stdin) = stdin {
            let mut child_stdin = child
                .stdin
                .take()
                .ok_or_else(|| TmuxError::new("failed to open tmux stdin"))?;
            child_stdin.write_all(stdin).map_err(|error| {
                TmuxError::new(format!(
                    "failed to write stdin for {}: {error}",
                    program.to_string_lossy()
                ))
            })?;
        }
        let output = child.wait_with_output().map_err(|error| {
            TmuxError::new(format!(
                "failed to collect {} output: {error}",
                program.to_string_lossy()
            ))
        })?;
        if output.status.success() {
            return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
        }
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        let detail = if stderr.is_empty() { stdout } else { stderr };
        let code = output
            .status
            .code()
            .map_or_else(|| "signal".to_owned(), |code| code.to_string());
        if detail.is_empty() {
            Err(TmuxError::new(format!("tmux exited with status {code}")))
        } else {
            Err(TmuxError::new(format!(
                "tmux exited with status {code}: {detail}"
            )))
        }
    }
}

/// Testable tmux client that delegates all execution to [`TmuxRunner`].
pub struct TmuxClient<R> {
    runner: R,
}

impl TmuxClient<CommandTmuxRunner> {
    /// Create a client backed by the local `tmux` binary.
    #[must_use]
    pub fn local() -> Self {
        Self::new(CommandTmuxRunner::new())
    }

    /// Create a client backed by the local `tmux` binary on a specific socket.
    #[must_use]
    pub fn local_with_socket(socket: impl Into<OsString>) -> Self {
        Self::new(CommandTmuxRunner::new().with_socket(socket))
    }
}

impl<R> TmuxClient<R>
where
    R: TmuxRunner,
{
    #[must_use]
    pub const fn new(runner: R) -> Self {
        Self { runner }
    }

    /// List session names; tmux-unavailable errors are fail-soft and return an empty list.
    pub fn list_session_names(&mut self) -> Vec<String> {
        self.runner
            .run(
                "list-sessions",
                &["-F".to_owned(), "#{session_name}".to_owned()],
            )
            .map(|raw| parse_session_names(&raw))
            .unwrap_or_default()
    }

    /// List all sessions/windows in a single tmux call; tmux-unavailable errors return empty.
    pub fn list_all(&mut self) -> Vec<TmuxSession> {
        self.runner
            .run(
                "list-windows",
                &[
                    "-a".to_owned(),
                    "-F".to_owned(),
                    "#{session_name}|||#{window_index}|||#{window_name}|||#{window_active}|||#{pane_current_path}".to_owned(),
                ],
            )
            .map(|raw| parse_list_all_windows(&raw))
            .unwrap_or_default()
    }

    /// List one session's windows.
    ///
    /// # Errors
    ///
    /// Returns the injected runner error when tmux rejects the session target.
    pub fn list_windows(&mut self, session: &str) -> Result<Vec<TmuxWindow>, TmuxError> {
        let raw = self.runner.run(
            "list-windows",
            &[
                "-t".to_owned(),
                session.to_owned(),
                "-F".to_owned(),
                "#{window_index}:#{window_name}:#{window_active}".to_owned(),
            ],
        )?;
        Ok(parse_list_windows(&raw))
    }

    /// Get all pane IDs; tmux-unavailable errors return empty.
    pub fn list_pane_ids(&mut self) -> BTreeSet<String> {
        self.runner
            .run(
                "list-panes",
                &["-a".to_owned(), "-F".to_owned(), "#{pane_id}".to_owned()],
            )
            .map(|raw| parse_pane_ids(&raw))
            .unwrap_or_default()
    }

    /// Get structured pane information; tmux-unavailable errors return empty.
    pub fn list_panes(&mut self) -> Vec<TmuxPane> {
        self.runner
            .run(
                "list-panes",
                &[
                    "-a".to_owned(),
                    "-F".to_owned(),
                    "#{pane_id}|||#{pane_current_command}|||#{session_name}:#{window_name}.#{pane_index}|||#{pane_title}|||#{pane_pid}|||#{pane_current_path}|||#{window_activity}".to_owned(),
                ],
            )
            .map(|raw| parse_list_panes(&raw))
            .unwrap_or_default()
    }

    /// Check whether a tmux session exists.
    pub fn has_session(&mut self, name: &str) -> bool {
        self.runner
            .run("has-session", &["-t".to_owned(), name.to_owned()])
            .is_ok()
    }

    /// Create a tmux session, then enable window renumbering like maw-js.
    ///
    /// # Errors
    ///
    /// Returns the runner error when `new-session` fails. `set-option` remains best-effort.
    pub fn new_session(
        &mut self,
        name: &str,
        options: &NewSessionOptions,
    ) -> Result<String, TmuxError> {
        let mut args = Vec::new();
        if options.detached {
            args.push("-d".to_owned());
        }
        if let Some(print_format) = &options.print_format {
            args.extend(["-P".to_owned(), "-F".to_owned(), print_format.clone()]);
        }
        args.extend(["-s".to_owned(), name.to_owned()]);
        if let Some(window) = &options.window {
            args.extend(["-n".to_owned(), window.clone()]);
        }
        if let Some(cwd) = &options.cwd {
            args.extend(["-c".to_owned(), cwd.clone()]);
        }
        if let Some(command) = &options.command {
            args.push(command.clone());
        }
        let out = self.runner.run("new-session", &args)?;
        self.set_option(name, "renumber-windows", "on");
        Ok(out)
    }

    /// Return the first pane ID for a target; errors return `None`.
    pub fn first_pane_id(&mut self, target: &str) -> Option<String> {
        self.runner
            .run(
                "list-panes",
                &[
                    "-t".to_owned(),
                    target.to_owned(),
                    "-F".to_owned(),
                    "#{pane_id}".to_owned(),
                ],
            )
            .ok()
            .and_then(|raw| {
                raw.lines()
                    .map(str::trim)
                    .find(|line| !line.is_empty())
                    .map(str::to_owned)
            })
    }

    /// Create a grouped session sharing windows with `parent`.
    ///
    /// # Errors
    ///
    /// Returns the runner error when the `new-session -t` call fails.
    pub fn new_grouped_session(
        &mut self,
        parent: &str,
        name: &str,
        options: &GroupedSessionOptions,
    ) -> Result<(), TmuxError> {
        let mut args = vec![
            "-d".to_owned(),
            "-t".to_owned(),
            parent.to_owned(),
            "-s".to_owned(),
            name.to_owned(),
        ];
        if let Some(cols) = options.cols {
            args.extend(["-x".to_owned(), cols.to_string()]);
        }
        if let Some(rows) = options.rows {
            args.extend(["-y".to_owned(), rows.to_string()]);
        }
        self.runner.run("new-session", &args)?;
        if let Some(window_size) = &options.window_size {
            self.set_option(name, "window-size", window_size);
        }
        if let Some(window) = &options.window {
            self.select_window(&format!("{name}:{window}"));
        }
        Ok(())
    }

    /// Kill a tmux session best-effort.
    pub fn kill_session(&mut self, name: &str) {
        self.try_run("kill-session", &["-t".to_owned(), name.to_owned()]);
    }

    /// Create a tmux window.
    ///
    /// # Errors
    ///
    /// Returns the runner error when tmux rejects the request.
    pub fn new_window(
        &mut self,
        session: &str,
        name: &str,
        cwd: Option<&str>,
    ) -> Result<(), TmuxError> {
        let mut args = vec![
            "-t".to_owned(),
            format!("{session}:"),
            "-n".to_owned(),
            name.to_owned(),
        ];
        if let Some(cwd) = cwd {
            args.extend(["-c".to_owned(), cwd.to_owned()]);
        }
        self.runner.run("new-window", &args).map(|_| ())
    }

    /// Select a tmux window best-effort.
    pub fn select_window(&mut self, target: &str) {
        self.try_run("select-window", &["-t".to_owned(), target.to_owned()]);
    }

    /// Switch the current tmux client best-effort.
    pub fn switch_client(&mut self, session: &str) {
        self.try_run("switch-client", &["-t".to_owned(), session.to_owned()]);
    }

    /// Kill a tmux window best-effort.
    pub fn kill_window(&mut self, target: &str) {
        self.try_run("kill-window", &["-t".to_owned(), target.to_owned()]);
    }

    /// Kill a tmux pane best-effort.
    pub fn kill_pane(&mut self, target: &str) {
        self.try_run("kill-pane", &["-t".to_owned(), target.to_owned()]);
    }

    /// Run maw-js `cmdTmuxKill` against an already-resolved/fallback-adjusted target.
    ///
    /// # Errors
    ///
    /// Returns safety refusal or runner errors.
    pub fn kill_target_action(
        &mut self,
        target: &TmuxKillTarget,
        fleet_sessions: &BTreeSet<String>,
        options: &TmuxKillCommandOptions,
    ) -> Result<TmuxKillOutcome, TmuxError> {
        let session = tmux_session_from_target(&target.resolved);
        if is_fleet_or_view_session(&session, fleet_sessions) && !options.force {
            return Err(TmuxError::new(format!(
                "refusing to kill: session '{session}' is fleet or view.\n  killing would terminate a live oracle (or its mirror).\n  pass --force to override (you really want to kill a fleet session)"
            )));
        }

        if options.session {
            self.runner
                .run("kill-session", &["-t".to_owned(), session.clone()])
                .map_err(|error| {
                    TmuxError::new(format!(
                        "kill failed for '{}' (from {}): {}",
                        target.resolved, target.source, error.message
                    ))
                })?;
            Ok(TmuxKillOutcome::Session { session })
        } else {
            self.runner
                .run("kill-pane", &["-t".to_owned(), target.resolved.clone()])
                .map_err(|error| {
                    TmuxError::new(format!(
                        "kill failed for '{}' (from {}): {}",
                        target.resolved, target.source, error.message
                    ))
                })?;
            Ok(TmuxKillOutcome::Pane {
                target: target.resolved.clone(),
            })
        }
    }

    /// Return the command running in a pane.
    ///
    /// # Errors
    ///
    /// Returns the runner error when tmux cannot inspect the target.
    pub fn get_pane_command(&mut self, target: &str) -> Result<String, TmuxError> {
        let raw = self.runner.run(
            "list-panes",
            &[
                "-t".to_owned(),
                target.to_owned(),
                "-F".to_owned(),
                "#{pane_current_command}".to_owned(),
            ],
        )?;
        Ok(raw.lines().next().unwrap_or_default().to_owned())
    }

    /// Return the current command for a pane through tmux `display-message`.
    ///
    /// This matches the safety lookup used by maw-js `cmdTmuxSend`.
    ///
    /// # Errors
    ///
    /// Returns the runner error when tmux cannot inspect the target.
    pub fn display_pane_current_command(&mut self, target: &str) -> Result<String, TmuxError> {
        self.runner
            .run(
                "display-message",
                &[
                    "-p".to_owned(),
                    "-t".to_owned(),
                    target.to_owned(),
                    "#{pane_current_command}".to_owned(),
                ],
            )
            .map(|raw| raw.trim().to_owned())
    }

    /// Return command and cwd for a pane.
    ///
    /// # Errors
    ///
    /// Returns the runner error when tmux cannot inspect the target.
    pub fn get_pane_info(&mut self, target: &str) -> Result<(String, String), TmuxError> {
        let raw = self.runner.run(
            "list-panes",
            &[
                "-t".to_owned(),
                target.to_owned(),
                "-F".to_owned(),
                "#{pane_current_command}\t#{pane_current_path}".to_owned(),
            ],
        )?;
        let first = raw.lines().next().unwrap_or_default();
        let (command, cwd) = first.split_once('\t').unwrap_or((first, ""));
        Ok((command.to_owned(), cwd.to_owned()))
    }

    /// Create a tmux pane split.
    ///
    /// # Errors
    ///
    /// Returns the runner error when tmux rejects the request.
    pub fn split_window(
        &mut self,
        target: Option<&str>,
        options: &SplitWindowOptions,
    ) -> Result<String, TmuxError> {
        let mut args = Vec::new();
        if let Some(print_format) = &options.print_format {
            args.extend(["-P".to_owned(), "-F".to_owned(), print_format.clone()]);
        }
        if let Some(target) = target {
            args.extend(["-t".to_owned(), target.to_owned()]);
        }
        if let Some(cwd) = &options.cwd {
            args.extend(["-c".to_owned(), cwd.clone()]);
        }
        if let Some(command) = &options.command {
            args.push(command.clone());
        }
        self.runner.run("split-window", &args)
    }

    /// Build and run the tmux args used by maw-js `splitWindowLocked`.
    ///
    /// This method does not sleep; callers that need cross-call settling own scheduling/locking.
    ///
    /// # Errors
    ///
    /// Returns the runner error when tmux rejects the split.
    pub fn split_window_locked(
        &mut self,
        target: &str,
        options: &SplitWindowLockedOptions,
    ) -> Result<(), TmuxError> {
        let mut args = vec!["-t".to_owned(), target.to_owned()];
        match options.vertical {
            Some(true) => args.push("-v".to_owned()),
            Some(false) => args.push("-h".to_owned()),
            None => {}
        }
        if let Some(pct) = options.pct {
            args.extend(["-l".to_owned(), format!("{pct}%")]);
        }
        if let Some(shell_command) = &options.shell_command {
            args.push(shell_command.clone());
        }
        self.runner.run("split-window", &args).map(|_| ())
    }

    /// Run the high-level maw-js `maw tmux split` mutation against an already-resolved pane.
    ///
    /// # Errors
    ///
    /// Returns validation or runner errors.
    pub fn split_pane_action(
        &mut self,
        resolved: &str,
        options: &TmuxSplitActionOptions,
    ) -> Result<(), TmuxError> {
        self.runner
            .run("split-window", &tmux_split_action_args(resolved, options)?)
            .map(|_| ())
    }

    /// Run maw-js `cmdTmuxSplit` against a resolved target with command-style error wrapping.
    ///
    /// # Errors
    ///
    /// Returns pct validation or wrapped runner errors.
    pub fn split_target_action(
        &mut self,
        target: &TmuxKillTarget,
        options: &TmuxSplitActionOptions,
    ) -> Result<(), TmuxError> {
        self.runner
            .run(
                "split-window",
                &tmux_split_action_args(&target.resolved, options)?,
            )
            .map(|_| ())
            .map_err(|error| {
                TmuxError::new(format!(
                    "split-window failed for '{}' (from {}): {}",
                    target.resolved, target.source, error.message
                ))
            })
    }

    /// Select a pane, optionally setting its title.
    ///
    /// # Errors
    ///
    /// Returns the runner error when tmux rejects the request.
    pub fn select_pane(
        &mut self,
        target: &str,
        options: &SelectPaneOptions,
    ) -> Result<(), TmuxError> {
        let mut args = vec!["-t".to_owned(), target.to_owned()];
        if let Some(title) = &options.title {
            args.extend(["-T".to_owned(), title.clone()]);
        }
        self.runner.run("select-pane", &args).map(|_| ())
    }

    /// Set pane title and/or tmux `@custom` metadata.
    ///
    /// # Errors
    ///
    /// Returns the first runner error from title or metadata writes.
    pub fn tag_pane(
        &mut self,
        target: &str,
        title: Option<&str>,
        meta: &[(String, String)],
    ) -> Result<(), TmuxError> {
        if let Some(title) = title {
            self.runner.run(
                "select-pane",
                &[
                    "-t".to_owned(),
                    target.to_owned(),
                    "-T".to_owned(),
                    title.to_owned(),
                ],
            )?;
        }
        for (raw_key, value) in meta {
            let key = normalize_pane_tag_key(raw_key);
            self.runner.run(
                "set-option",
                &[
                    "-p".to_owned(),
                    "-t".to_owned(),
                    target.to_owned(),
                    key,
                    value.clone(),
                ],
            )?;
        }
        Ok(())
    }

    /// Read pane title and tmux `@custom` metadata.
    ///
    /// # Errors
    ///
    /// Returns the runner error when the title probe fails. Metadata probe is best-effort.
    pub fn read_pane_tags(&mut self, target: &str) -> Result<PaneTags, TmuxError> {
        let title = self
            .runner
            .run(
                "display-message",
                &[
                    "-p".to_owned(),
                    "-t".to_owned(),
                    target.to_owned(),
                    "#{pane_title}".to_owned(),
                ],
            )?
            .trim()
            .to_owned();
        let raw = self.try_run(
            "show-options",
            &["-p".to_owned(), "-t".to_owned(), target.to_owned()],
        );
        Ok(PaneTags {
            title,
            meta: parse_pane_tag_options(&raw),
        })
    }

    /// Select a tmux layout.
    ///
    /// # Errors
    ///
    /// Returns the runner error when tmux rejects the request.
    pub fn select_layout(&mut self, target: &str, layout: &str) -> Result<(), TmuxError> {
        self.runner
            .run(
                "select-layout",
                &["-t".to_owned(), target.to_owned(), layout.to_owned()],
            )
            .map(|_| ())
    }

    /// Apply a maw-js `maw tmux layout` preset after validating the allowed set.
    ///
    /// # Errors
    ///
    /// Returns validation or runner errors.
    pub fn select_valid_layout(&mut self, resolved: &str, preset: &str) -> Result<(), TmuxError> {
        validate_layout_preset(preset)?;
        let window = tmux_window_target(resolved);
        self.select_layout(&window, preset)
    }

    /// Run maw-js `cmdTmuxLayout` against a resolved target with command-style error wrapping.
    ///
    /// # Errors
    ///
    /// Returns preset validation or wrapped runner errors.
    pub fn select_layout_action(
        &mut self,
        target: &TmuxKillTarget,
        preset: &str,
    ) -> Result<(), TmuxError> {
        validate_layout_preset(preset)?;
        let window = tmux_window_target(&target.resolved);
        self.runner
            .run(
                "select-layout",
                &["-t".to_owned(), window.clone(), preset.to_owned()],
            )
            .map(|_| ())
            .map_err(|error| {
                TmuxError::new(format!(
                    "select-layout failed for '{}' (from {}): {}",
                    window, target.source, error.message
                ))
            })
    }

    /// Send tmux keys to a target.
    ///
    /// # Errors
    ///
    /// Returns the runner error when tmux rejects the request.
    pub fn send_keys(&mut self, target: &str, keys: &[String]) -> Result<(), TmuxError> {
        let mut args = vec!["-t".to_owned(), target.to_owned()];
        args.extend(keys.iter().cloned());
        self.runner.run("send-keys", &args).map(|_| ())
    }

    /// Send literal text through `tmux send-keys -l`.
    ///
    /// # Errors
    ///
    /// Returns the runner error when tmux rejects the request.
    pub fn send_keys_literal(&mut self, target: &str, text: &str) -> Result<(), TmuxError> {
        self.runner
            .run(
                "send-keys",
                &[
                    "-t".to_owned(),
                    target.to_owned(),
                    "-l".to_owned(),
                    text.to_owned(),
                ],
            )
            .map(|_| ())
    }

    /// Run the high-level maw-js `maw tmux send` mutation against an already-resolved pane.
    ///
    /// The caller owns target resolution and user-facing output; this method ports the action
    /// gates and exact `send-keys` argument shape.
    ///
    /// # Errors
    ///
    /// Returns validation, safety, lookup, or runner errors.
    pub fn send_command_to_pane(
        &mut self,
        tracker: &mut TmuxSendTracker,
        resolved: &str,
        command: &str,
        options: &TmuxSendCommandOptions,
        now_ms: u64,
    ) -> Result<TmuxSendCommandOutcome, TmuxError> {
        if command.is_empty() {
            return Err(TmuxError::new(
                "usage: maw tmux send <target> <command> [--literal] [--allow-destructive] [--force]",
            ));
        }
        match tracker.check(resolved, now_ms, options.force) {
            SendThrottle::Allowed => {}
            throttle => return Ok(TmuxSendCommandOutcome::Throttled(throttle)),
        }

        let destructive = check_destructive(command);
        if destructive.destructive && !options.allow_destructive {
            return Err(TmuxError::new(format!(
                "refusing to send: command matches destructive patterns:\n{}\n  pass --allow-destructive to bypass (review carefully first)",
                destructive
                    .reasons
                    .iter()
                    .map(|reason| format!("  - {reason}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            )));
        }

        let pane_current_command = self.display_pane_current_command(resolved)?;
        if is_claude_like_pane(Some(&pane_current_command)) && !options.force {
            return Err(TmuxError::new(format!(
                "refusing to send: pane '{resolved}' is running '{pane_current_command}' (claude-like).\n  injecting keys would collide with the AI's turn.\n  pass --force to override (you really want to type into a live claude pane)"
            )));
        }

        self.runner
            .run(
                "send-keys",
                &tmux_send_command_args(resolved, command, options.literal),
            )
            .map(|_| TmuxSendCommandOutcome::Sent)
    }

    /// Paste tmux buffer into a target.
    ///
    /// # Errors
    ///
    /// Returns the runner error when tmux rejects the request.
    pub fn paste_buffer(&mut self, target: &str) -> Result<(), TmuxError> {
        self.runner
            .run("paste-buffer", &["-t".to_owned(), target.to_owned()])
            .map(|_| ())
    }

    /// Load text into tmux buffer via stdin.
    ///
    /// # Errors
    ///
    /// Returns the runner error when tmux rejects the buffer load.
    pub fn load_buffer(&mut self, text: &str) -> Result<(), TmuxError> {
        self.runner
            .run_with_stdin("load-buffer", &["-".to_owned()], text.as_bytes())
            .map(|_| ())
    }

    /// Smart text sending: buffer for multiline/long payloads, literal send otherwise, then submit-confirm.
    ///
    /// This is the synchronous maw-rs port of maw-js `sendText`; callers own any real-time settle delay.
    ///
    /// # Errors
    ///
    /// Returns the first tmux error from mode exit, text placement, paste, or Enter send.
    pub fn send_text(&mut self, target: &str, text: &str) -> Result<SendTextReport, TmuxError> {
        self.exit_mode_if_needed(target)?;
        let used_buffer = text.contains('\n') || text.len() > 500;
        if used_buffer {
            self.load_buffer(text)?;
            self.paste_buffer(target)?;
        } else {
            self.send_keys_literal(target, text)?;
        }
        let (enter_attempts, warned_pending) = self.submit_with_confirm(target)?;
        Ok(SendTextReport {
            used_buffer,
            enter_attempts,
            warned_pending,
        })
    }

    fn submit_with_confirm(&mut self, target: &str) -> Result<(u32, bool), TmuxError> {
        for attempt in 1..=MAX_SUBMIT_ATTEMPTS {
            self.send_keys(target, &["Enter".to_owned()])?;
            if !self.pane_input_pending(target) {
                return Ok((attempt, false));
            }
        }
        Ok((MAX_SUBMIT_ATTEMPTS, true))
    }

    /// Capture recent pane contents using `tmux capture-pane`.
    ///
    /// # Errors
    ///
    /// Returns the runner error when tmux cannot capture the target.
    pub fn capture(&mut self, target: &str, lines: Option<u32>) -> Result<String, TmuxError> {
        let lines = lines.unwrap_or(DEFAULT_CAPTURE_LINES);
        self.runner.run(
            "capture-pane",
            &[
                "-t".to_owned(),
                target.to_owned(),
                "-e".to_owned(),
                "-p".to_owned(),
                "-S".to_owned(),
                format!("-{lines}"),
            ],
        )
    }

    /// Resize a pane best-effort, clamping to maw-js default pty limits.
    pub fn resize_pane(&mut self, target: &str, cols: u32, rows: u32) {
        let cols = clamp_pty(cols, DEFAULT_PTY_COLS_LIMIT);
        let rows = clamp_pty(rows, DEFAULT_PTY_ROWS_LIMIT);
        self.try_run(
            "resize-pane",
            &[
                "-t".to_owned(),
                target.to_owned(),
                "-x".to_owned(),
                cols.to_string(),
                "-y".to_owned(),
                rows.to_string(),
            ],
        );
    }

    /// Resize a window best-effort, clamping to maw-js default pty limits.
    pub fn resize_window(&mut self, target: &str, cols: u32, rows: u32) {
        let cols = clamp_pty(cols, DEFAULT_PTY_COLS_LIMIT);
        let rows = clamp_pty(rows, DEFAULT_PTY_ROWS_LIMIT);
        self.try_run(
            "resize-window",
            &[
                "-t".to_owned(),
                target.to_owned(),
                "-x".to_owned(),
                cols.to_string(),
                "-y".to_owned(),
                rows.to_string(),
            ],
        );
    }

    /// Leave tmux copy-mode when the target reports `#{pane_in_mode} == 1`.
    ///
    /// # Errors
    ///
    /// Returns non-`not in a mode` cancellation errors from tmux. Probe failures return `Ok(false)`.
    pub fn exit_mode_if_needed(&mut self, target: &str) -> Result<bool, TmuxError> {
        let probe = self.runner.run(
            "display-message",
            &[
                "-t".to_owned(),
                target.to_owned(),
                "-p".to_owned(),
                "#{pane_in_mode}".to_owned(),
            ],
        );
        if probe.is_ok_and(|raw| raw.trim() == "1") {
            return match self.runner.run(
                "send-keys",
                &[
                    "-t".to_owned(),
                    target.to_owned(),
                    "-X".to_owned(),
                    "cancel".to_owned(),
                ],
            ) {
                Ok(_) => Ok(true),
                Err(error) if error.message.contains("not in a mode") => Ok(false),
                Err(error) => Err(error),
            };
        }
        Ok(false)
    }

    /// Check whether captured pane text still appears to contain unsubmitted input.
    pub fn pane_input_pending(&mut self, target: &str) -> bool {
        self.capture(target, Some(5))
            .is_ok_and(|content| pane_input_pending_from_capture(&content))
    }

    /// Set a tmux environment variable.
    ///
    /// # Errors
    ///
    /// Returns the runner error when tmux rejects the request.
    pub fn set_environment(
        &mut self,
        session: &str,
        key: &str,
        value: &str,
    ) -> Result<(), TmuxError> {
        self.runner
            .run(
                "set-environment",
                &[
                    "-t".to_owned(),
                    session.to_owned(),
                    key.to_owned(),
                    value.to_owned(),
                ],
            )
            .map(|_| ())
    }

    /// Set a tmux option best-effort.
    pub fn set_option(&mut self, target: &str, option: &str, value: &str) {
        self.try_run(
            "set-option",
            &[
                "-t".to_owned(),
                target.to_owned(),
                option.to_owned(),
                value.to_owned(),
            ],
        );
    }

    /// Set a tmux value best-effort.
    pub fn set(&mut self, target: &str, option: &str, value: &str) {
        self.try_run(
            "set",
            &[
                "-t".to_owned(),
                target.to_owned(),
                option.to_owned(),
                value.to_owned(),
            ],
        );
    }

    fn try_run(&mut self, subcommand: &str, args: &[String]) -> String {
        self.runner.run(subcommand, args).unwrap_or_default()
    }
}

fn clamp_pty(value: u32, max: u32) -> u32 {
    value.clamp(1, max)
}

/// Strip common ANSI CSI sequences that tmux captures from pane output.
#[must_use]
pub fn strip_tmux_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == 0x1b && bytes.get(index + 1) == Some(&b'[') {
            index += 2;
            while index < bytes.len() && (bytes[index].is_ascii_digit() || bytes[index] == b';') {
                index += 1;
            }
            if index < bytes.len()
                && matches!(
                    bytes[index],
                    b'm' | b'G' | b'K' | b'H' | b'F' | b'J' | b'A'..=b'Z'
                )
            {
                index += 1;
                continue;
            }
            out.push('\u{1b}');
            out.push('[');
            continue;
        }
        let Some(ch) = input[index..].chars().next() else {
            break;
        };
        out.push(ch);
        index += ch.len_utf8();
    }
    out
}

/// Return true when captured pane output appears to have pending prompt input.
#[must_use]
pub fn pane_input_pending_from_capture(content: &str) -> bool {
    let Some(last) = content.lines().rfind(|line| !line.trim().is_empty()) else {
        return false;
    };
    let clean = strip_tmux_ansi(last).replace('\r', "");
    prompt_has_input(&clean)
}

fn prompt_has_input(line: &str) -> bool {
    let chars = line.chars().collect::<Vec<_>>();
    for (index, ch) in chars.iter().enumerate() {
        if !matches!(ch, '#' | '$' | '%' | '>' | '❯' | '»') {
            continue;
        }
        let mut next = index + 1;
        let mut saw_space = false;
        while next < chars.len() && chars[next].is_whitespace() {
            saw_space = true;
            next += 1;
        }
        if saw_space && next < chars.len() && !chars[next].is_whitespace() {
            return true;
        }
    }
    false
}

/// Scan a command for maw-js `maw tmux send` destructive deny-list patterns.
#[must_use]
pub fn check_destructive(command: &str) -> DestructiveCheck {
    let mut reasons = Vec::new();
    if contains_word(command, "rm") {
        reasons.push("rm — removes files".to_owned());
    }
    if contains_word(command, "sudo") {
        reasons.push("sudo — elevated privileges".to_owned());
    }
    if has_redirect(command, false) {
        reasons.push("> redirect — overwrites".to_owned());
    }
    if has_redirect(command, true) {
        reasons.push(">> redirect — appends (possibly to wrong place)".to_owned());
    }
    if has_operator_with_rhs(command, ';') {
        reasons.push("; command chain — multiple commands".to_owned());
    }
    if has_sequence_with_rhs(command, "&&") {
        reasons.push("&& chain — conditional execution".to_owned());
    }
    if has_operator_with_rhs(command, '|') {
        reasons.push("| pipe — composition (review carefully)".to_owned());
    }
    let lower = command.to_lowercase();
    if lower.contains("git reset --hard") {
        reasons.push("git reset --hard — discards changes".to_owned());
    }
    if contains_word(&lower, "git") && lower.contains("push") && lower.contains("--force") {
        reasons.push("git push --force — rewrites history".to_owned());
    }
    if contains_word(&lower, "git") && lower.contains("clean -f") {
        reasons.push("git clean -f — removes untracked files".to_owned());
    }
    if contains_word(&lower, "gh") && contains_word(&lower, "delete") {
        reasons.push("gh delete — removes GitHub resource".to_owned());
    }
    if lower.contains("kill -9") {
        reasons.push("kill -9 — force-terminate process".to_owned());
    }
    if lower.contains("drop table") {
        reasons.push("DROP TABLE — removes database table".to_owned());
    }
    DestructiveCheck {
        destructive: !reasons.is_empty(),
        reasons,
    }
}

fn contains_word(haystack: &str, needle: &str) -> bool {
    let bytes = haystack.as_bytes();
    let needle = needle.as_bytes();
    if needle.is_empty() || bytes.len() < needle.len() {
        return false;
    }
    for index in 0..=bytes.len() - needle.len() {
        if !bytes[index..].starts_with(needle) {
            continue;
        }
        let before = index.checked_sub(1).and_then(|i| bytes.get(i));
        let after = bytes.get(index + needle.len());
        if before.is_none_or(|byte| !is_word_byte(*byte))
            && after.is_none_or(|byte| !is_word_byte(*byte))
        {
            return true;
        }
    }
    false
}

fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn has_redirect(command: &str, append: bool) -> bool {
    let bytes = command.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if append {
            if bytes[index..].starts_with(b">>") && has_non_space_after(&bytes[index + 2..]) {
                return true;
            }
            index += 1;
        } else {
            if bytes[index] == b'>'
                && bytes.get(index + 1) != Some(&b'>')
                && has_non_space_after(&bytes[index + 1..])
            {
                return true;
            }
            index += 1;
        }
    }
    false
}

fn has_operator_with_rhs(command: &str, operator: char) -> bool {
    command
        .split_once(operator)
        .is_some_and(|(_, rhs)| !rhs.trim().is_empty())
}

fn has_sequence_with_rhs(command: &str, sequence: &str) -> bool {
    command
        .split_once(sequence)
        .is_some_and(|(_, rhs)| !rhs.trim().is_empty())
}

fn has_non_space_after(bytes: &[u8]) -> bool {
    bytes.iter().any(|byte| !byte.is_ascii_whitespace())
}

/// Detect Claude Code or version-shaped Claude wrapper pane commands.
#[must_use]
pub fn is_claude_like_pane(pane_current_command: Option<&str>) -> bool {
    let Some(command) = pane_current_command else {
        return false;
    };
    let command = command.to_lowercase();
    if command.contains("claude") {
        return true;
    }
    is_three_part_numeric_version(command.trim())
}

fn is_three_part_numeric_version(value: &str) -> bool {
    let mut parts = value.split('.');
    let Some(first) = parts.next() else {
        return false;
    };
    let Some(second) = parts.next() else {
        return false;
    };
    let Some(third) = parts.next() else {
        return false;
    };
    if parts.next().is_some() {
        return false;
    }
    [first, second, third]
        .iter()
        .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

/// Protect fleet and view sessions from accidental kill operations.
#[must_use]
pub fn is_fleet_or_view_session(session_name: &str, fleet_sessions: &BTreeSet<String>) -> bool {
    fleet_sessions.contains(session_name)
        || session_name == "maw-view"
        || session_name.ends_with("-view")
}

/// Validate maw-js `maw tmux layout` presets.
///
/// # Errors
///
/// Returns a message listing every valid preset when `preset` is invalid.
pub fn validate_layout_preset(preset: &str) -> Result<(), TmuxError> {
    if VALID_LAYOUTS.contains(&preset) {
        Ok(())
    } else {
        Err(TmuxError::new(format!(
            "invalid layout '{preset}'. Valid: {}",
            VALID_LAYOUTS.join(", ")
        )))
    }
}

/// Strip a pane suffix from a tmux target so layout applies to the window.
#[must_use]
pub fn tmux_window_target(resolved: &str) -> String {
    let Some(dot) = resolved.rfind('.') else {
        return resolved.to_owned();
    };
    let Some(colon) = resolved.rfind(':') else {
        return resolved.to_owned();
    };
    if dot > colon + 1
        && resolved[dot + 1..]
            .bytes()
            .all(|byte| byte.is_ascii_digit())
    {
        resolved[..dot].to_owned()
    } else {
        resolved.to_owned()
    }
}

/// Validate and render maw-js `maw tmux split --pct`.
///
/// # Errors
///
/// Returns the maw-js-compatible bounds message for NaN, infinities, and values outside `1..=99`.
pub fn split_pct_arg(pct: f64) -> Result<String, TmuxError> {
    if !pct.is_finite() || !(1.0..=99.0).contains(&pct) {
        return Err(TmuxError::new(format!("--pct must be 1-99 (got {pct})")));
    }
    Ok(format_js_number(pct))
}

fn format_js_number(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        value.to_string()
    }
}

/// Build tmux args for maw-js `cmdTmuxSplit`.
///
/// # Errors
///
/// Returns pct validation errors.
pub fn tmux_split_action_args(
    resolved: &str,
    options: &TmuxSplitActionOptions,
) -> Result<Vec<String>, TmuxError> {
    let mut args = vec![
        if options.vertical { "-v" } else { "-h" }.to_owned(),
        "-l".to_owned(),
        format!("{}%", split_pct_arg(options.pct)?),
        "-t".to_owned(),
        resolved.to_owned(),
    ];
    if let Some(command) = &options.command {
        args.push(command.clone());
    }
    Ok(args)
}

/// Build tmux args for maw-js `cmdTmuxSend`.
#[must_use]
pub fn tmux_send_command_args(resolved: &str, command: &str, literal: bool) -> Vec<String> {
    let mut args = vec!["-t".to_owned(), resolved.to_owned(), command.to_owned()];
    if !literal {
        args.push("Enter".to_owned());
    }
    args
}

/// Pure branch selector for maw-js `cmdTmuxAttach`.
#[must_use]
pub fn decide_tmux_attach_action(
    resolved: &str,
    alive_sessions: &BTreeSet<String>,
    print: bool,
    is_tty: bool,
    in_tmux: bool,
) -> TmuxAttachAction {
    let session = resolved.split(':').next().unwrap_or_default().to_owned();
    if !alive_sessions.contains(&session) {
        return TmuxAttachAction::Recover { session };
    }
    if print || !is_tty {
        return TmuxAttachAction::Print { session };
    }
    if in_tmux {
        TmuxAttachAction::SwitchClient { session }
    } else {
        TmuxAttachAction::Attach { session }
    }
}

/// Build the `tmux` process command selected for a live attach action.
#[must_use]
pub fn tmux_attach_spawn_command(action: &TmuxAttachAction) -> Option<SpawnCommand> {
    match action {
        TmuxAttachAction::SwitchClient { session } => Some(SpawnCommand {
            program: "tmux".to_owned(),
            args: vec!["switch-client".to_owned(), "-t".to_owned(), session.clone()],
        }),
        TmuxAttachAction::Attach { session } => Some(SpawnCommand {
            program: "tmux".to_owned(),
            args: vec!["attach".to_owned(), "-t".to_owned(), session.clone()],
        }),
        TmuxAttachAction::Print { .. } | TmuxAttachAction::Recover { .. } => None,
    }
}

/// Strip `-oracle` from bare repo names while preserving org/repo slugs.
#[must_use]
pub fn wake_arg_for_similar_oracle(candidate: &str) -> String {
    if candidate.contains('/') {
        candidate.to_owned()
    } else {
        candidate
            .strip_suffix("-oracle")
            .unwrap_or(candidate)
            .to_owned()
    }
}

fn maw_wake_attach_command(oracle: &str) -> SpawnCommand {
    SpawnCommand {
        program: "maw".to_owned(),
        args: vec!["wake".to_owned(), oracle.to_owned(), "-a".to_owned()],
    }
}

/// Build attach recovery candidates from a stale fleet session and similar oracle repos.
#[must_use]
pub fn attach_recovery_candidates(
    target: &str,
    session: &str,
    source: &str,
    fleet_entries: &[AttachRecoveryFleetEntry],
    cloned_repos: &[String],
) -> Vec<AttachRecoveryCandidate> {
    let mut candidates = Vec::new();
    if source.starts_with("fleet-stem")
        || source.starts_with("fleet-window")
        || source.starts_with("live-session")
    {
        if let Some(entry) = fleet_entries.iter().find(|entry| entry.session == session) {
            if let Some(window) = &entry.first_window_name {
                let oracle = window.strip_suffix("-oracle").unwrap_or(window).to_owned();
                let cloned = entry
                    .repo
                    .as_deref()
                    .and_then(|repo| {
                        cloned_repos
                            .iter()
                            .find(|path| path.ends_with(&format!("/{repo}")))
                    })
                    .is_some();
                candidates.push(AttachRecoveryCandidate {
                    oracle,
                    label: format!(
                        "{window} ({})",
                        if cloned { "cloned" } else { "not cloned" }
                    ),
                });
            }
        }
    }

    for similar in similar_oracle_candidates_from_repos(target, cloned_repos) {
        let oracle = wake_arg_for_similar_oracle(&similar);
        if !candidates
            .iter()
            .any(|candidate| candidate.oracle == oracle)
        {
            candidates.push(AttachRecoveryCandidate {
                oracle,
                label: similar,
            });
        }
    }
    candidates
}

/// Decide attach recovery behavior after candidates are known.
#[must_use]
pub fn decide_attach_recovery(
    candidates: &[AttachRecoveryCandidate],
    is_tty: bool,
    choice: Option<usize>,
) -> AttachRecoveryDecision {
    match candidates.len() {
        0 => AttachRecoveryDecision::NoCandidates,
        1 => AttachRecoveryDecision::AutoWake {
            command: maw_wake_attach_command(&candidates[0].oracle),
            label: candidates[0].label.clone(),
        },
        _ if !is_tty => AttachRecoveryDecision::PrintCandidates {
            candidates: candidates.to_vec(),
        },
        _ => match choice {
            Some(choice) if (1..=candidates.len()).contains(&choice) => {
                AttachRecoveryDecision::WakeChoice {
                    command: maw_wake_attach_command(&candidates[choice - 1].oracle),
                }
            }
            Some(_) => AttachRecoveryDecision::InvalidChoice,
            None => AttachRecoveryDecision::Prompt {
                candidates: candidates.to_vec(),
            },
        },
    }
}

/// Return the session component from a tmux target.
#[must_use]
pub fn tmux_session_from_target(resolved: &str) -> String {
    resolved.split(':').next().unwrap_or_default().to_owned()
}

/// Apply maw-js orphan-pane fallback for `cmdTmuxKill`.
///
/// Only unresolved bare session-name fallbacks (`source == "session-name"` and `resolved == target`)
/// consult pane titles, tile roles, and worktree aliases. Exact pane IDs and qualified targets are
/// preserved.
///
/// # Errors
///
/// Returns an ambiguity error with concrete candidates when a natural name matches multiple panes.
pub fn resolve_kill_target_with_pane_fallback(
    target: &str,
    resolved: &str,
    source: &str,
    session_kill: bool,
    list_panes_output: &str,
) -> Result<TmuxKillTarget, TmuxError> {
    if !session_kill && source == "session-name" && resolved == target {
        match resolve_pane_target_from_list_panes_output(target, list_panes_output) {
            PaneTargetResolution::Match { candidate } => {
                return Ok(TmuxKillTarget {
                    resolved: candidate.resolved,
                    source: format!("{} ({})", candidate.source, candidate.name),
                });
            }
            PaneTargetResolution::Ambiguous { candidates } => {
                return Err(TmuxError::new(format_pane_ambiguity_error(
                    target,
                    &candidates,
                )));
            }
            PaneTargetResolution::None => {}
        }
    }
    Ok(TmuxKillTarget {
        resolved: resolved.to_owned(),
        source: source.to_owned(),
    })
}

fn format_pane_ambiguity_error(target: &str, candidates: &[PaneTargetCandidate]) -> String {
    let lines = candidates
        .iter()
        .map(|candidate| {
            let target_note = if candidate.target.is_empty() {
                String::new()
            } else {
                format!(" ({})", candidate.target)
            };
            format!(
                "    • {} → {}{} [{}]",
                candidate.name, candidate.resolved, target_note, candidate.source
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "'{target}' is ambiguous — matches {} panes:\n{lines}\n  use the pane id or full session:window.pane target",
        candidates.len()
    )
}

fn basename(path: &str) -> &str {
    path.split('/')
        .rfind(|part| !part.is_empty())
        .unwrap_or(path)
}

fn worktree_names_from_cwd(cwd: &str) -> Vec<(String, String)> {
    let base = basename(cwd);
    if base.is_empty() {
        return Vec::new();
    }
    let mut out = vec![(base.to_owned(), "worktree-dir".to_owned())];
    let Some((repo, rest)) = base.split_once(".wt-") else {
        return out;
    };
    let role = rest
        .split_once('-')
        .map(|(_, role)| role)
        .unwrap_or_default()
        .trim();
    if !role.is_empty() {
        out.push((role.to_owned(), "worktree-role".to_owned()));
        if let Some(repo_stem) = repo.strip_suffix("-oracle") {
            if !repo_stem.is_empty() {
                out.push((format!("{repo_stem}-{role}"), "worktree-alias".to_owned()));
            }
        }
    }
    out
}

/// Parse `PANE_TARGET_FORMAT` rows into pane target resolution candidates.
#[must_use]
pub fn pane_target_candidates_from_list_panes_output(raw: &str) -> Vec<PaneTargetCandidate> {
    let mut candidates = Vec::new();
    for line in raw.lines().filter(|line| !line.trim().is_empty()) {
        let fields = line.split("|||").collect::<Vec<_>>();
        let id = fields.first().copied().unwrap_or_default().trim();
        let target = fields.get(1).copied().unwrap_or_default().trim();
        let title = fields.get(2).copied().unwrap_or_default();
        let tile_role = fields.get(3).copied().unwrap_or_default();
        let cwd = fields.get(4).copied().unwrap_or_default();
        let resolved = if id.is_empty() { target } else { id };
        if resolved.is_empty() {
            continue;
        }
        add_pane_target_candidate(&mut candidates, title, resolved, "pane-title", target);
        add_pane_target_candidate(&mut candidates, tile_role, resolved, "tile-role", target);
        for (name, source) in worktree_names_from_cwd(cwd) {
            add_pane_target_candidate(&mut candidates, &name, resolved, &source, target);
        }
    }
    candidates
}

fn add_pane_target_candidate(
    candidates: &mut Vec<PaneTargetCandidate>,
    name: &str,
    resolved: &str,
    source: &str,
    target: &str,
) {
    let name = name.trim();
    if name.is_empty() {
        return;
    }
    candidates.push(PaneTargetCandidate {
        name: name.to_owned(),
        resolved: resolved.to_owned(),
        source: source.to_owned(),
        target: target.to_owned(),
    });
}

fn unique_by_resolved(candidates: Vec<PaneTargetCandidate>) -> Vec<PaneTargetCandidate> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for candidate in candidates {
        if seen.insert(candidate.resolved.clone()) {
            out.push(candidate);
        }
    }
    out
}

/// Resolve a natural pane title, tile role, worktree dir, or worktree alias to a pane id.
#[must_use]
pub fn resolve_pane_target_from_candidates(
    target: &str,
    candidates: &[PaneTargetCandidate],
) -> PaneTargetResolution {
    let trimmed = target.trim().to_lowercase();
    let exact = unique_by_resolved(
        candidates
            .iter()
            .filter(|candidate| candidate.name.to_lowercase() == trimmed)
            .cloned()
            .collect(),
    );
    match exact.len() {
        1 => {
            return PaneTargetResolution::Match {
                candidate: exact[0].clone(),
            }
        }
        2.. => return PaneTargetResolution::Ambiguous { candidates: exact },
        0 => {}
    }

    match resolve_by_name(target, candidates, ResolveOptions::default()) {
        ResolveResult::Exact { matched } | ResolveResult::Fuzzy { matched } => {
            PaneTargetResolution::Match { candidate: matched }
        }
        ResolveResult::Ambiguous { candidates } => PaneTargetResolution::Ambiguous {
            candidates: unique_by_resolved(candidates),
        },
        ResolveResult::None { .. } => PaneTargetResolution::None,
    }
}

/// Resolve a pane target directly from `PANE_TARGET_FORMAT` list-panes output.
#[must_use]
pub fn resolve_pane_target_from_list_panes_output(target: &str, raw: &str) -> PaneTargetResolution {
    resolve_pane_target_from_candidates(target, &pane_target_candidates_from_list_panes_output(raw))
}

/// Parse `tmux list-sessions -F '#{session_name}\t#{session_created}'` style epoch rows.
#[must_use]
pub fn parse_session_epoch_list(raw: &str) -> BTreeMap<String, u64> {
    let mut out = BTreeMap::new();
    for line in raw.lines().filter(|line| !line.trim().is_empty()) {
        let Some((name, epoch_raw)) = line.split_once('\t') else {
            continue;
        };
        let Ok(epoch) = epoch_raw.parse::<u64>() else {
            continue;
        };
        if !name.is_empty() && epoch > 0 {
            out.insert(name.to_owned(), epoch);
        }
    }
    out
}

/// Parse tmux session creation rows.
#[must_use]
pub fn parse_session_created_list(raw: &str) -> BTreeMap<String, u64> {
    parse_session_epoch_list(raw)
}

/// Parse tmux session activity rows.
#[must_use]
pub fn parse_session_activity_list(raw: &str) -> BTreeMap<String, u64> {
    parse_session_epoch_list(raw)
}

/// Parse `maw ls --active` duration values. Bare numbers are minutes.
#[must_use]
pub fn parse_active_duration_seconds(raw: Option<&str>) -> Option<u64> {
    let trimmed = raw?.trim().to_lowercase();
    if trimmed.is_empty() {
        return None;
    }
    let last = trimmed.chars().last()?;
    let (digits, unit) = match last {
        's' | 'm' | 'h' | 'd' => (&trimmed[..trimmed.len() - 1], last),
        _ => (trimmed.as_str(), 'm'),
    };
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let value = digits.parse::<u64>().ok()?;
    if value == 0 {
        return None;
    }
    let multiplier = match unit {
        's' => 1,
        'm' => 60,
        'h' => 60 * 60,
        'd' => 24 * 60 * 60,
        _ => return None,
    };
    value.checked_mul(multiplier)
}

/// Return the valid duration argument supplied to a flag such as `--active`.
#[must_use]
pub fn active_duration_arg(argv: &[String], flag: &str) -> Option<String> {
    for (index, arg) in argv.iter().enumerate() {
        if arg == flag {
            let next = argv.get(index + 1)?;
            return (!next.starts_with('-') && parse_active_duration_seconds(Some(next)).is_some())
                .then(|| next.clone());
        }
        if let Some(value) = arg.strip_prefix(&format!("{flag}=")) {
            if parse_active_duration_seconds(Some(value)).is_some() {
                return Some(value.to_owned());
            }
        }
    }
    None
}

/// Format an epoch second as a deterministic UTC timestamp.
#[must_use]
pub fn format_session_created(epoch_seconds: Option<u64>) -> String {
    let Some(epoch_seconds) = epoch_seconds.filter(|epoch| *epoch > 0) else {
        return "—".to_owned();
    };
    let days = epoch_seconds / 86_400;
    let Ok(days) = i64::try_from(days) else {
        return "—".to_owned();
    };
    let seconds_of_day = epoch_seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}")
}

fn civil_from_days(days_since_unix_epoch: i64) -> (i64, i64, i64) {
    let z = days_since_unix_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = y + i64::from(month <= 2);
    (year, month, day)
}

/// Return unique matching oracle repo slugs, preserving input order.
#[must_use]
pub fn similar_oracle_candidates_from_repos(target: &str, repos: &[String]) -> Vec<String> {
    let query = target.to_lowercase();
    let mut out = Vec::new();
    for repo in repos {
        let name = repo_name_from_path(repo);
        if !name.ends_with("-oracle") || !name.to_lowercase().contains(&query) {
            continue;
        }
        let slug = repo_slug_from_path(repo);
        if !out.iter().any(|existing| existing == &slug) {
            out.push(slug);
        }
    }
    out
}

fn repo_name_from_path(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn repo_slug_from_path(path: &str) -> String {
    let parts = path
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.len() >= 2 {
        parts[parts.len() - 2..].join("/")
    } else {
        repo_name_from_path(path).to_owned()
    }
}

/// Annotate a pane for `maw tmux ls`: team > fleet > view > orphan > empty.
#[must_use]
pub fn annotate_pane(
    pane: &TmuxLsPaneRef,
    fleet_sessions: &BTreeSet<String>,
    team_by_pane: &BTreeMap<String, String>,
) -> String {
    let session = pane
        .target
        .split_once(':')
        .map_or(pane.target.as_str(), |(session, _)| session);
    if let Some(team) = team_by_pane.get(&pane.id) {
        return format!("team: {team}");
    }
    if fleet_sessions.contains(session) {
        return format!("fleet: {}", strip_numeric_prefix(session));
    }
    if session == "maw-view" || session.ends_with("-view") {
        return format!("view: {session}");
    }
    if is_claude_like_pane(pane.command.as_deref()) {
        return "orphan".to_owned();
    }
    String::new()
}

/// Normalize pane metadata keys to tmux `@custom` option names.
#[must_use]
pub fn normalize_pane_tag_key(raw_key: &str) -> String {
    if raw_key.starts_with('@') {
        raw_key.to_owned()
    } else {
        format!("@{raw_key}")
    }
}

/// Parse `show-options -p -t <pane>` output for tmux `@custom` metadata.
#[must_use]
pub fn parse_pane_tag_options(raw: &str) -> BTreeMap<String, String> {
    let mut meta = BTreeMap::new();
    for line in raw.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let Some((key, rest)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        if !key.starts_with('@') {
            continue;
        }
        let value = parse_tmux_option_value(rest.trim());
        meta.insert(key.to_owned(), value);
    }
    meta
}

fn parse_tmux_option_value(value: &str) -> String {
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        return unescape_tmux_quoted_value(&value[1..value.len() - 1]);
    }
    value.to_owned()
}

fn unescape_tmux_quoted_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut escaped = false;
    for ch in value.chars() {
        if escaped {
            out.push(ch);
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else {
            out.push(ch);
        }
    }
    if escaped {
        out.push('\\');
    }
    out
}

/// Shell-quote one tmux command argument using the same safe-character policy as maw-js.
#[must_use]
pub fn shell_quote(value: impl fmt::Display) -> String {
    let value = value.to_string();
    if !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b':' | b'-' | b'/')
        })
    {
        value
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

/// Build the shell command used by maw-js-style `tmux [-S socket] subcommand args...` execution.
#[must_use]
pub fn tmux_shell_command(socket: Option<&str>, subcommand: &str, args: &[String]) -> String {
    let socket_flag =
        socket.map_or_else(String::new, |socket| format!("-S {} ", shell_quote(socket)));
    let joined_args = args.iter().map(shell_quote).collect::<Vec<_>>().join(" ");
    if joined_args.is_empty() {
        format!("tmux {socket_flag}{subcommand}")
    } else {
        format!("tmux {socket_flag}{subcommand} {joined_args}")
    }
}

/// Parse `tmux list-sessions -F '#{session_name}'` output.
#[must_use]
pub fn parse_session_names(raw: &str) -> Vec<String> {
    raw.lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Parse maw-js `list-windows -a` format.
#[must_use]
pub fn parse_list_all_windows(raw: &str) -> Vec<TmuxSession> {
    let mut sessions: Vec<TmuxSession> = Vec::new();
    for line in raw.lines().filter(|line| !line.is_empty()) {
        let fields = line.split("|||").collect::<Vec<_>>();
        if fields.len() < 4 {
            continue;
        }
        let session_name = fields[0];
        let window = TmuxWindow {
            index: fields[1].parse().unwrap_or(0),
            name: fields[2].to_owned(),
            active: fields[3] == "1",
            cwd: fields
                .get(4)
                .and_then(|cwd| (!cwd.is_empty()).then(|| (*cwd).to_owned())),
        };
        if let Some(session) = sessions
            .iter_mut()
            .find(|session| session.name == session_name)
        {
            session.windows.push(window);
        } else {
            sessions.push(TmuxSession {
                name: session_name.to_owned(),
                windows: vec![window],
            });
        }
    }
    sessions
}

/// Parse maw-js `list-windows -t <session> -F '#{window_index}:#{window_name}:#{window_active}'` output.
#[must_use]
pub fn parse_list_windows(raw: &str) -> Vec<TmuxWindow> {
    raw.lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            let mut parts = line.splitn(3, ':');
            let index = parts
                .next()
                .and_then(|value| value.parse().ok())
                .unwrap_or(0);
            let name = parts.next().unwrap_or_default().to_owned();
            let active = parts.next() == Some("1");
            TmuxWindow {
                index,
                name,
                active,
                cwd: None,
            }
        })
        .collect()
}

/// Parse `tmux list-panes -a -F '#{pane_id}'` output.
#[must_use]
pub fn parse_pane_ids(raw: &str) -> BTreeSet<String> {
    raw.lines()
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Parse maw-js structured `list-panes -a` format.
#[must_use]
pub fn parse_list_panes(raw: &str) -> Vec<TmuxPane> {
    raw.lines()
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            let fields = line.split("|||").collect::<Vec<_>>();
            (fields.len() >= 4).then(|| TmuxPane {
                id: fields[0].to_owned(),
                command: fields[1].to_owned(),
                target: fields[2].to_owned(),
                title: fields[3].to_owned(),
                pid: fields.get(4).and_then(|pid| pid.parse().ok()),
                cwd: fields
                    .get(5)
                    .and_then(|cwd| (!cwd.is_empty()).then(|| (*cwd).to_owned())),
                last_activity: fields.get(6).and_then(|activity| activity.parse().ok()),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct FakeRunner {
        calls: Vec<(String, Vec<String>)>,
        stdin_calls: Vec<(String, Vec<String>, String)>,
        responses: Vec<Result<String, TmuxError>>,
    }

    impl FakeRunner {
        fn with_responses(responses: Vec<Result<&str, TmuxError>>) -> Self {
            Self {
                calls: Vec::new(),
                stdin_calls: Vec::new(),
                responses: responses
                    .into_iter()
                    .map(|response| response.map(str::to_owned))
                    .collect(),
            }
        }
    }

    impl FakeRunner {
        fn next_response(&mut self) -> Result<String, TmuxError> {
            if self.responses.is_empty() {
                return Err(TmuxError::new("no response"));
            }
            self.responses.remove(0)
        }
    }

    impl TmuxRunner for FakeRunner {
        fn run(&mut self, subcommand: &str, args: &[String]) -> Result<String, TmuxError> {
            self.calls.push((subcommand.to_owned(), args.to_vec()));
            self.next_response()
        }

        fn run_with_stdin(
            &mut self,
            subcommand: &str,
            args: &[String],
            stdin: &[u8],
        ) -> Result<String, TmuxError> {
            self.stdin_calls.push((
                subcommand.to_owned(),
                args.to_vec(),
                String::from_utf8_lossy(stdin).into_owned(),
            ));
            self.next_response()
        }
    }

    #[test]
    fn shell_quote_matches_maw_js_safe_chars_and_single_quote_escape() {
        assert_eq!(
            shell_quote("alpha_1:/tmp/repo.wt-main"),
            "alpha_1:/tmp/repo.wt-main"
        );
        assert_eq!(shell_quote("two words"), "'two words'");
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
        assert_eq!(shell_quote(""), "''");
    }

    #[test]
    fn command_runner_argv_matches_tmux_socket_order_without_executing() {
        let runner = CommandTmuxRunner::with_program("/usr/bin/tmux").with_socket("/tmp/maw sock");
        let argv = runner.argv(
            "list-panes",
            &["-a".to_owned(), "-F".to_owned(), "#{pane_id}".to_owned()],
        );
        assert_eq!(
            argv,
            vec![
                OsString::from("/usr/bin/tmux"),
                OsString::from("-S"),
                OsString::from("/tmp/maw sock"),
                OsString::from("list-panes"),
                OsString::from("-a"),
                OsString::from("-F"),
                OsString::from("#{pane_id}"),
            ]
        );
    }

    #[test]
    fn tmux_shell_command_includes_optional_socket() {
        assert_eq!(
            tmux_shell_command(
                Some("/tmp/maw sock"),
                "list-windows",
                &[
                    "-a".to_owned(),
                    "-F".to_owned(),
                    "#{window_name}".to_owned()
                ],
            ),
            "tmux -S '/tmp/maw sock' list-windows -a -F '#{window_name}'",
        );
    }

    #[test]
    fn parse_list_all_groups_windows_by_session_in_order() {
        let sessions = parse_list_all_windows(
            "s1|||1|||alpha|||1|||/tmp/a\ns1|||2|||beta|||0|||\ns2|||1|||gamma|||0|||/tmp/g\n",
        );
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].name, "s1");
        assert_eq!(sessions[0].windows[0].cwd.as_deref(), Some("/tmp/a"));
        assert_eq!(sessions[0].windows[1].cwd, None);
        assert!(sessions[0].windows[0].active);
        assert_eq!(sessions[1].windows[0].name, "gamma");
    }

    #[test]
    fn parse_list_windows_matches_maw_js_colon_format() {
        assert_eq!(
            parse_list_windows("1:oracle:1\n2:notes:0\n"),
            vec![
                TmuxWindow {
                    index: 1,
                    name: "oracle".to_owned(),
                    active: true,
                    cwd: None
                },
                TmuxWindow {
                    index: 2,
                    name: "notes".to_owned(),
                    active: false,
                    cwd: None
                },
            ],
        );
    }

    #[test]
    fn parse_list_panes_handles_optional_numeric_fields() {
        let panes = parse_list_panes(
            "%1|||claude|||s:oracle.0|||title|||123|||/repo|||456\n%2|||zsh|||s:logs.0|||||||||\n",
        );
        assert_eq!(panes.len(), 2);
        assert_eq!(panes[0].pid, Some(123));
        assert_eq!(panes[0].cwd.as_deref(), Some("/repo"));
        assert_eq!(panes[0].last_activity, Some(456));
        assert_eq!(panes[1].pid, None);
    }

    #[test]
    fn client_session_mutators_match_maw_js_arg_order() {
        let runner = FakeRunner::with_responses(vec![
            Ok("%1\n"),
            Err(TmuxError::new("set-option ignored")),
            Ok(""),
            Ok(""),
        ]);
        let mut client = TmuxClient::new(runner);
        let out = client
            .new_session(
                "maw",
                &NewSessionOptions {
                    window: Some("agent".to_owned()),
                    cwd: Some("/repo".to_owned()),
                    command: Some("exec zsh -li".to_owned()),
                    print_format: Some("#{pane_id}".to_owned()),
                    ..NewSessionOptions::default()
                },
            )
            .expect("new session ok");
        assert_eq!(out, "%1\n");
        client
            .new_window("maw", "logs", Some("/tmp"))
            .expect("new window ok");
        client.kill_session("old");

        assert_eq!(client.runner.calls[0].0, "new-session");
        assert_eq!(
            client.runner.calls[0].1,
            vec![
                "-d",
                "-P",
                "-F",
                "#{pane_id}",
                "-s",
                "maw",
                "-n",
                "agent",
                "-c",
                "/repo",
                "exec zsh -li",
            ]
        );
        assert_eq!(client.runner.calls[1].0, "set-option");
        assert_eq!(
            client.runner.calls[2],
            (
                "new-window".to_owned(),
                vec!["-t", "maw:", "-n", "logs", "-c", "/tmp"]
                    .into_iter()
                    .map(str::to_owned)
                    .collect()
            )
        );
        assert_eq!(client.runner.calls[3].0, "kill-session");
    }

    #[test]
    fn client_pane_commands_match_maw_js_arg_order() {
        let runner = FakeRunner::with_responses(vec![
            Ok("%9\n"),
            Ok("claude\n"),
            Ok("zsh\t/repo\n"),
            Ok("%10\n"),
            Ok(""),
            Ok(""),
            Ok(""),
        ]);
        let mut client = TmuxClient::new(runner);
        assert_eq!(client.first_pane_id("maw:agent"), Some("%9".to_owned()));
        assert_eq!(
            client.get_pane_command("%9").expect("pane command"),
            "claude"
        );
        assert_eq!(
            client.get_pane_info("%9").expect("pane info"),
            ("zsh".to_owned(), "/repo".to_owned())
        );
        let split = client
            .split_window(
                Some("maw:agent"),
                &SplitWindowOptions {
                    cwd: Some("/repo".to_owned()),
                    command: Some("exec zsh -li".to_owned()),
                    print_format: Some("#{pane_id}".to_owned()),
                },
            )
            .expect("split ok");
        assert_eq!(split, "%10\n");
        client
            .select_pane(
                "%10",
                &SelectPaneOptions {
                    title: Some("oracle".to_owned()),
                },
            )
            .expect("select pane ok");
        client
            .send_keys_literal("%10", "hello | world")
            .expect("literal send ok");
        client
            .send_keys("%10", &["Enter".to_owned()])
            .expect("send keys ok");

        assert_eq!(client.runner.calls[0].0, "list-panes");
        assert_eq!(client.runner.calls[3].0, "split-window");
        assert_eq!(
            client.runner.calls[3].1,
            vec![
                "-P",
                "-F",
                "#{pane_id}",
                "-t",
                "maw:agent",
                "-c",
                "/repo",
                "exec zsh -li",
            ]
        );
        assert_eq!(client.runner.calls[5].0, "send-keys");
        assert_eq!(
            client.runner.calls[5].1,
            vec!["-t", "%10", "-l", "hello | world"]
        );
    }

    #[test]
    fn tmux_safety_destructive_patterns_match_maw_js_cases() {
        let cases = [
            ("ls -la", false),
            ("echo hello", false),
            ("date", false),
            ("pwd && cd /", true),
            ("rm file.txt", true),
            ("rm -rf /tmp/junk", true),
            ("sudo apt update", true),
            ("echo > /etc/passwd", true),
            ("echo >> ~/.bashrc", true),
            ("cat file ; echo done", true),
            ("test && rm -f", true),
            ("cat file | grep x", true),
            ("git reset --hard HEAD", true),
            ("git push --force origin main", true),
            ("git clean -fd", true),
            ("gh repo delete foo/bar", true),
            ("kill -9 12345", true),
            ("DROP TABLE users", true),
            ("drop table users", true),
            ("echo 'rm trick'", true),
            ("", false),
        ];
        for (command, destructive) in cases {
            let check = check_destructive(command);
            assert_eq!(check.destructive, destructive, "{command}");
            assert_eq!(check.reasons.is_empty(), !destructive, "{command}");
        }
        let multi = check_destructive("sudo rm -rf /");
        assert!(multi.destructive);
        assert!(multi.reasons.len() >= 2);
    }

    #[test]
    fn tmux_safety_claude_like_pane_matches_maw_js_cases() {
        assert!(is_claude_like_pane(Some("claude")));
        assert!(is_claude_like_pane(Some("CLAUDE")));
        assert!(is_claude_like_pane(Some("bun run claude")));
        assert!(is_claude_like_pane(Some("2.1.111")));
        assert!(!is_claude_like_pane(Some("2.0.0-alpha.105")));
        assert!(!is_claude_like_pane(Some("bash")));
        assert!(!is_claude_like_pane(Some("vim")));
        assert!(!is_claude_like_pane(None));
        assert!(!is_claude_like_pane(Some("")));
    }

    #[test]
    fn tmux_safety_fleet_or_view_session_matches_maw_js_cases() {
        let fleet = BTreeSet::from([
            "101-mawjs".to_owned(),
            "112-fusion".to_owned(),
            "114-mawjs-no2".to_owned(),
        ]);
        assert!(is_fleet_or_view_session("101-mawjs", &fleet));
        assert!(is_fleet_or_view_session("maw-view", &fleet));
        assert!(is_fleet_or_view_session("mawjs-view", &fleet));
        assert!(is_fleet_or_view_session("fusion-view", &fleet));
        assert!(!is_fleet_or_view_session("random-session", &fleet));
        assert!(!is_fleet_or_view_session("view-something", &fleet));
        assert!(is_fleet_or_view_session("maw-view", &BTreeSet::new()));
        assert!(is_fleet_or_view_session("anything-view", &BTreeSet::new()));
    }

    #[test]
    fn tmux_action_layout_and_split_validation_match_maw_js_cases() {
        let error = validate_layout_preset("bogus").expect_err("invalid layout");
        assert!(error.message.contains("invalid layout 'bogus'"));
        assert!(error.message.contains("even-horizontal"));
        assert!(error.message.contains("main-horizontal"));
        assert!(error.message.contains("tiled"));
        assert!(validate_layout_preset("tiled").is_ok());

        for pct in [0.0, 100.0, -5.0, f64::NAN] {
            let error = split_pct_arg(pct).expect_err("invalid pct");
            assert!(error.message.contains("--pct must be 1-99"));
        }
        assert_eq!(split_pct_arg(50.0).expect("valid pct"), "50");
        assert_eq!(split_pct_arg(12.5).expect("valid fractional pct"), "12.5");
        assert_eq!(
            tmux_split_action_args(
                "alpha:0.1",
                &TmuxSplitActionOptions {
                    vertical: false,
                    pct: 40.0,
                    command: Some("bash -lc 'echo hi'".to_owned()),
                },
            )
            .expect("valid split args"),
            vec!["-h", "-l", "40%", "-t", "alpha:0.1", "bash -lc 'echo hi'"]
        );
        assert_eq!(tmux_window_target("some-session:0.1"), "some-session:0");
        assert_eq!(tmux_window_target("some-session"), "some-session");
    }

    #[test]
    fn tmux_split_and_layout_actions_wrap_host_failures_like_maw_js() {
        let target = TmuxKillTarget {
            resolved: "%1".to_owned(),
            source: "pane-id".to_owned(),
        };
        let runner = FakeRunner::with_responses(vec![Err(TmuxError::new("split bad"))]);
        let mut client = TmuxClient::new(runner);
        let error = client
            .split_target_action(&target, &TmuxSplitActionOptions::default())
            .expect_err("split error wrapped");
        assert_eq!(
            error.message,
            "split-window failed for '%1' (from pane-id): split bad"
        );

        let target = TmuxKillTarget {
            resolved: "demo:1.2".to_owned(),
            source: "session:w.p".to_owned(),
        };
        let runner = FakeRunner::with_responses(vec![Err(TmuxError::new("layout denied"))]);
        let mut client = TmuxClient::new(runner);
        let error = client
            .select_layout_action(&target, "tiled")
            .expect_err("layout error wrapped");
        assert_eq!(
            error.message,
            "select-layout failed for 'demo:1' (from session:w.p): layout denied"
        );
        assert_eq!(
            client.runner.calls,
            vec![(
                "select-layout".to_owned(),
                vec!["-t".to_owned(), "demo:1".to_owned(), "tiled".to_owned()]
            )]
        );

        let error = client
            .select_layout_action(&target, "spiral")
            .expect_err("invalid layout");
        assert!(error.message.contains("invalid layout 'spiral'"));
    }

    #[test]
    fn tmux_attach_action_branches_match_maw_js_cases() {
        let alive = BTreeSet::from(["some-session".to_owned()]);
        assert_eq!(
            decide_tmux_attach_action(
                "%999",
                &BTreeSet::from(["%999".to_owned()]),
                true,
                true,
                false
            ),
            TmuxAttachAction::Print {
                session: "%999".to_owned()
            }
        );
        assert_eq!(
            decide_tmux_attach_action("some-session:0.1", &alive, true, true, false),
            TmuxAttachAction::Print {
                session: "some-session".to_owned()
            }
        );
        assert_eq!(
            decide_tmux_attach_action("some-session:0.1", &alive, false, false, false),
            TmuxAttachAction::Print {
                session: "some-session".to_owned()
            }
        );
        assert_eq!(
            decide_tmux_attach_action("some-session:0.1", &alive, false, true, true),
            TmuxAttachAction::SwitchClient {
                session: "some-session".to_owned()
            }
        );
        assert_eq!(
            decide_tmux_attach_action("some-session:0.1", &alive, false, true, false),
            TmuxAttachAction::Attach {
                session: "some-session".to_owned()
            }
        );
        assert_eq!(
            decide_tmux_attach_action("ghost-session", &alive, false, true, false),
            TmuxAttachAction::Recover {
                session: "ghost-session".to_owned()
            }
        );

        assert_eq!(
            tmux_attach_spawn_command(&TmuxAttachAction::SwitchClient {
                session: "some-session".to_owned()
            }),
            Some(SpawnCommand {
                program: "tmux".to_owned(),
                args: vec![
                    "switch-client".to_owned(),
                    "-t".to_owned(),
                    "some-session".to_owned()
                ],
            })
        );
        assert_eq!(
            tmux_attach_spawn_command(&TmuxAttachAction::Attach {
                session: "some-session".to_owned()
            }),
            Some(SpawnCommand {
                program: "tmux".to_owned(),
                args: vec![
                    "attach".to_owned(),
                    "-t".to_owned(),
                    "some-session".to_owned()
                ],
            })
        );
        assert_eq!(
            tmux_attach_spawn_command(&TmuxAttachAction::Print {
                session: "some-session".to_owned()
            }),
            None
        );
    }

    #[test]
    fn tmux_attach_recovery_candidates_and_choices_match_maw_js() {
        let cloned_repos = vec![
            "/opt/Code/github.com/Soul-Brews-Studio/pulse-oracle".to_owned(),
            "/opt/Code/github.com/Soul-Brews-Studio/pulse-helper-oracle".to_owned(),
            "/opt/Code/github.com/Org/sleeping-oracle".to_owned(),
        ];
        assert_eq!(
            wake_arg_for_similar_oracle("pulse-oracle"),
            "pulse".to_owned()
        );
        assert_eq!(
            wake_arg_for_similar_oracle("Soul-Brews-Studio/pulse-oracle"),
            "Soul-Brews-Studio/pulse-oracle".to_owned()
        );

        let candidates = attach_recovery_candidates(
            "pulse",
            "ghost",
            "session-name",
            &[],
            &["/opt/Code/github.com/Soul-Brews-Studio/pulse-oracle".to_owned()],
        );
        assert_eq!(
            candidates,
            vec![AttachRecoveryCandidate {
                oracle: "Soul-Brews-Studio/pulse-oracle".to_owned(),
                label: "Soul-Brews-Studio/pulse-oracle".to_owned(),
            }]
        );
        assert_eq!(
            decide_attach_recovery(&candidates, false, None),
            AttachRecoveryDecision::AutoWake {
                command: SpawnCommand {
                    program: "maw".to_owned(),
                    args: vec![
                        "wake".to_owned(),
                        "Soul-Brews-Studio/pulse-oracle".to_owned(),
                        "-a".to_owned()
                    ],
                },
                label: "Soul-Brews-Studio/pulse-oracle".to_owned(),
            }
        );

        let candidates = attach_recovery_candidates(
            "44-sleeping",
            "44-sleeping",
            "fleet-stem (44-sleeping)",
            &[AttachRecoveryFleetEntry {
                session: "44-sleeping".to_owned(),
                first_window_name: Some("sleeping-oracle".to_owned()),
                repo: Some("Org/sleeping-oracle".to_owned()),
            }],
            &cloned_repos,
        );
        assert_eq!(
            candidates[0],
            AttachRecoveryCandidate {
                oracle: "sleeping".to_owned(),
                label: "sleeping-oracle (cloned)".to_owned(),
            }
        );

        let candidates =
            attach_recovery_candidates("pulse", "pulse", "session-name", &[], &cloned_repos);
        assert_eq!(candidates.len(), 2);
        assert_eq!(
            decide_attach_recovery(&candidates, false, None),
            AttachRecoveryDecision::PrintCandidates {
                candidates: candidates.clone()
            }
        );
        assert_eq!(
            decide_attach_recovery(&candidates, true, None),
            AttachRecoveryDecision::Prompt {
                candidates: candidates.clone()
            }
        );
        assert_eq!(
            decide_attach_recovery(&candidates, true, Some(2)),
            AttachRecoveryDecision::WakeChoice {
                command: SpawnCommand {
                    program: "maw".to_owned(),
                    args: vec![
                        "wake".to_owned(),
                        "Soul-Brews-Studio/pulse-helper-oracle".to_owned(),
                        "-a".to_owned()
                    ],
                }
            }
        );
        assert_eq!(
            decide_attach_recovery(&candidates, true, Some(3)),
            AttachRecoveryDecision::InvalidChoice
        );
        assert_eq!(
            decide_attach_recovery(&[], true, None),
            AttachRecoveryDecision::NoCandidates
        );
    }

    #[test]
    fn tmux_send_tracker_matches_maw_js_cooldown_and_quota_gate() {
        let mut tracker = TmuxSendTracker::default();
        assert_eq!(tracker.check("%1", 1_000, false), SendThrottle::Allowed);
        assert_eq!(
            tracker.check("%1", 1_100, false),
            SendThrottle::Cooldown { cooldown_ms: 500 }
        );
        assert_eq!(tracker.check("%1", 1_600, false), SendThrottle::Allowed);
        assert_eq!(
            tracker.get("%1"),
            Some(SendTrackerEntry {
                last_ts: 1_600,
                count: 2,
                window_start: 1_000,
            })
        );

        tracker.set(
            "%2",
            SendTrackerEntry {
                last_ts: 10_000,
                count: 100,
                window_start: 0,
            },
        );
        assert_eq!(
            tracker.check("%2", 11_000, false),
            SendThrottle::Quota {
                quota_per_minute: 100
            }
        );
        assert_eq!(tracker.check("%2", 61_001, false), SendThrottle::Allowed);
        assert_eq!(
            tracker.get("%2"),
            Some(SendTrackerEntry {
                last_ts: 61_001,
                count: 1,
                window_start: 61_001,
            })
        );

        tracker.set(
            "%3",
            SendTrackerEntry {
                last_ts: 20_000,
                count: 100,
                window_start: 0,
            },
        );
        assert_eq!(tracker.check("%3", 20_001, true), SendThrottle::Allowed);
        assert_eq!(
            tracker.get("%3"),
            Some(SendTrackerEntry {
                last_ts: 20_000,
                count: 100,
                window_start: 0,
            })
        );
    }

    #[test]
    fn tmux_send_action_gates_and_args_match_maw_js_cases() {
        assert_eq!(
            tmux_send_command_args("%1", "echo hello", false),
            vec!["-t", "%1", "echo hello", "Enter"]
        );
        assert_eq!(
            tmux_send_command_args("%1", "C-c", true),
            vec!["-t", "%1", "C-c"]
        );

        let runner = FakeRunner::with_responses(vec![Ok("bash\n"), Ok("")]);
        let mut client = TmuxClient::new(runner);
        let mut tracker = TmuxSendTracker::default();
        let outcome = client
            .send_command_to_pane(
                &mut tracker,
                "%1",
                "echo hello",
                &TmuxSendCommandOptions::default(),
                1_000,
            )
            .expect("send succeeds");
        assert_eq!(outcome, TmuxSendCommandOutcome::Sent);
        assert_eq!(client.runner.calls[0].0, "display-message");
        assert_eq!(
            client.runner.calls[0].1,
            vec!["-p", "-t", "%1", "#{pane_current_command}"]
        );
        assert_eq!(
            client.runner.calls[1],
            (
                "send-keys".to_owned(),
                vec!["-t", "%1", "echo hello", "Enter"]
                    .into_iter()
                    .map(str::to_owned)
                    .collect()
            )
        );

        let runner = FakeRunner::with_responses(vec![Ok("bash\n")]);
        let mut client = TmuxClient::new(runner);
        let mut tracker = TmuxSendTracker::default();
        let error = client
            .send_command_to_pane(
                &mut tracker,
                "%2",
                "rm -rf /tmp/junk",
                &TmuxSendCommandOptions::default(),
                2_000,
            )
            .expect_err("destructive command blocked");
        assert!(error.message.contains("refusing to send"));
        assert!(error.message.contains("--allow-destructive"));
        assert!(client.runner.calls.is_empty());

        let runner = FakeRunner::with_responses(vec![Ok("claude\n")]);
        let mut client = TmuxClient::new(runner);
        let mut tracker = TmuxSendTracker::default();
        let error = client
            .send_command_to_pane(
                &mut tracker,
                "%3",
                "echo hello",
                &TmuxSendCommandOptions::default(),
                3_000,
            )
            .expect_err("claude-like pane blocked");
        assert!(error.message.contains("claude-like"));
        assert_eq!(client.runner.calls.len(), 1);

        let runner = FakeRunner::with_responses(vec![Ok("claude\n"), Ok("")]);
        let mut client = TmuxClient::new(runner);
        let mut tracker = TmuxSendTracker::default();
        let outcome = client
            .send_command_to_pane(
                &mut tracker,
                "%4",
                "C-c",
                &TmuxSendCommandOptions {
                    literal: true,
                    allow_destructive: false,
                    force: true,
                },
                4_000,
            )
            .expect("force bypasses claude-like pane");
        assert_eq!(outcome, TmuxSendCommandOutcome::Sent);
        assert_eq!(
            client.runner.calls[1].1,
            vec!["-t", "%4", "C-c"]
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn pane_target_resolver_indexes_titles_roles_and_worktree_aliases() {
        let raw = [
            "%101|||47-mawjs:1.0|||codex-headless-demo-layout|||tile-1|||/opt/Code/github.com/Soul-Brews-Studio/mawjs-oracle.wt-7-codex-headless",
            "%202|||47-mawjs:1.1|||notes|||researcher|||/opt/Code/github.com/Soul-Brews-Studio/notes-oracle.wt-2-researcher",
        ]
        .join("\n");

        let names = pane_target_candidates_from_list_panes_output(&raw)
            .into_iter()
            .map(|candidate| {
                format!(
                    "{}:{}:{}",
                    candidate.name, candidate.source, candidate.resolved
                )
            })
            .collect::<Vec<_>>();

        assert!(names.contains(&"codex-headless-demo-layout:pane-title:%101".to_owned()));
        assert!(names.contains(&"tile-1:tile-role:%101".to_owned()));
        assert!(names.contains(&"codex-headless:worktree-role:%101".to_owned()));
        assert!(names.contains(&"mawjs-codex-headless:worktree-alias:%101".to_owned()));

        let hit = resolve_pane_target_from_list_panes_output("mawjs-codex-headless", &raw);
        assert_eq!(
            hit,
            PaneTargetResolution::Match {
                candidate: PaneTargetCandidate {
                    name: "mawjs-codex-headless".to_owned(),
                    resolved: "%101".to_owned(),
                    source: "worktree-alias".to_owned(),
                    target: "47-mawjs:1.0".to_owned(),
                }
            }
        );

        let hit = resolve_pane_target_from_list_panes_output("codex-headless-demo-layout", &raw);
        assert_eq!(
            hit,
            PaneTargetResolution::Match {
                candidate: PaneTargetCandidate {
                    name: "codex-headless-demo-layout".to_owned(),
                    resolved: "%101".to_owned(),
                    source: "pane-title".to_owned(),
                    target: "47-mawjs:1.0".to_owned(),
                }
            }
        );
    }

    #[test]
    fn pane_target_resolver_keeps_ambiguous_matches_safe() {
        let raw = [
            "%1|||47-mawjs:1.0|||codex-a|||worker|||/tmp/mawjs-oracle.wt-1-codex",
            "%2|||47-mawjs:1.1|||codex-b|||worker|||/tmp/mawjs-oracle.wt-2-codex",
        ]
        .join("\n");
        let hit = resolve_pane_target_from_list_panes_output("worker", &raw);
        match hit {
            PaneTargetResolution::Ambiguous { candidates } => {
                assert_eq!(
                    candidates
                        .iter()
                        .map(|candidate| candidate.resolved.clone())
                        .collect::<Vec<_>>(),
                    vec!["%1", "%2"]
                );
            }
            other => panic!("expected ambiguous, got {other:?}"),
        }

        let candidates = vec![
            PaneTargetCandidate {
                name: "fleet-alpha".to_owned(),
                resolved: "%1".to_owned(),
                source: "pane-title".to_owned(),
                target: "s:1.1".to_owned(),
            },
            PaneTargetCandidate {
                name: "one-view".to_owned(),
                resolved: "%2".to_owned(),
                source: "pane-title".to_owned(),
                target: "s:1.2".to_owned(),
            },
            PaneTargetCandidate {
                name: "two-view".to_owned(),
                resolved: "%3".to_owned(),
                source: "pane-title".to_owned(),
                target: "s:1.3".to_owned(),
            },
        ];
        assert_eq!(
            resolve_pane_target_from_candidates("alpha", &candidates),
            PaneTargetResolution::Match {
                candidate: candidates[0].clone()
            }
        );
        assert_eq!(
            resolve_pane_target_from_candidates("view", &candidates),
            PaneTargetResolution::Ambiguous {
                candidates: vec![candidates[1].clone(), candidates[2].clone()]
            }
        );
    }

    #[test]
    fn tmux_kill_action_refuses_fleet_and_force_kills_session() {
        let runner = FakeRunner::with_responses(vec![Ok("")]);
        let mut client = TmuxClient::new(runner);
        let fleet = BTreeSet::from(["101-mawjs".to_owned()]);
        let target = TmuxKillTarget {
            resolved: "101-mawjs:0.1".to_owned(),
            source: "session:w.p".to_owned(),
        };

        let error = client
            .kill_target_action(&target, &fleet, &TmuxKillCommandOptions::default())
            .expect_err("fleet session protected");
        assert!(error
            .message
            .contains("refusing to kill: session '101-mawjs' is fleet or view"));
        assert!(client.runner.calls.is_empty());

        let outcome = client
            .kill_target_action(
                &target,
                &fleet,
                &TmuxKillCommandOptions {
                    force: true,
                    session: true,
                },
            )
            .expect("forced session kill succeeds");
        assert_eq!(
            outcome,
            TmuxKillOutcome::Session {
                session: "101-mawjs".to_owned()
            }
        );
        assert_eq!(
            client.runner.calls,
            vec![(
                "kill-session".to_owned(),
                vec!["-t".to_owned(), "101-mawjs".to_owned()]
            )]
        );
    }

    #[test]
    fn tmux_kill_action_uses_orphan_pane_fallback_and_wraps_errors() {
        let raw = "%101|||scratch:0.0|||worker|||tile-a|||/tmp/repo.wt-1-scout\n";
        let target =
            resolve_kill_target_with_pane_fallback("scout", "scout", "session-name", false, raw)
                .expect("fallback target");
        assert_eq!(
            target,
            TmuxKillTarget {
                resolved: "%101".to_owned(),
                source: "worktree-role (scout)".to_owned(),
            }
        );

        let runner = FakeRunner::with_responses(vec![Ok("")]);
        let mut client = TmuxClient::new(runner);
        let outcome = client
            .kill_target_action(
                &target,
                &BTreeSet::new(),
                &TmuxKillCommandOptions::default(),
            )
            .expect("pane kill succeeds");
        assert_eq!(
            outcome,
            TmuxKillOutcome::Pane {
                target: "%101".to_owned()
            }
        );
        assert_eq!(
            client.runner.calls,
            vec![(
                "kill-pane".to_owned(),
                vec!["-t".to_owned(), "%101".to_owned()]
            )]
        );

        let runner = FakeRunner::with_responses(vec![Err(TmuxError::new("kill denied"))]);
        let mut client = TmuxClient::new(runner);
        let error = client
            .kill_target_action(
                &target,
                &BTreeSet::new(),
                &TmuxKillCommandOptions::default(),
            )
            .expect_err("kill failure wrapped");
        assert_eq!(
            error.message,
            "kill failed for '%101' (from worktree-role (scout)): kill denied"
        );
    }

    #[test]
    fn tmux_kill_fallback_reports_ambiguous_pane_aliases() {
        let raw = [
            "%71|||demo:2.0|||codex||||||/repos/a",
            "%72|||demo:3.0|||codex||||||/repos/b",
        ]
        .join("\n");
        let error =
            resolve_kill_target_with_pane_fallback("codex", "codex", "session-name", false, &raw)
                .expect_err("ambiguous alias refused");
        assert!(error
            .message
            .contains("'codex' is ambiguous — matches 2 panes:"));
        assert!(error
            .message
            .contains("• codex → %71 (demo:2.0) [pane-title]"));
        assert!(error
            .message
            .contains("• codex → %72 (demo:3.0) [pane-title]"));

        let preserved =
            resolve_kill_target_with_pane_fallback("codex", "codex", "session-name", true, &raw)
                .expect("session kill does not fallback");
        assert_eq!(
            preserved,
            TmuxKillTarget {
                resolved: "codex".to_owned(),
                source: "session-name".to_owned(),
            }
        );
    }

    #[test]
    fn tmux_ls_recent_pure_helpers_match_maw_js_tests() {
        let raw =
            "old-session\t100\nnew-session\t300\nmid-session\t200\nzero\t0\nbad\tnope\nmissing\n";
        assert_eq!(
            parse_session_created_list(raw),
            BTreeMap::from([
                ("mid-session".to_owned(), 200),
                ("new-session".to_owned(), 300),
                ("old-session".to_owned(), 100),
            ])
        );
        assert_eq!(format_session_created(None), "—");
        assert_eq!(format_session_created(Some(0)), "—");
        assert_eq!(format_session_created(Some(300)), "1970-01-01 00:05:00");
        assert_eq!(parse_active_duration_seconds(Some("30m")), Some(1800));
        assert_eq!(parse_active_duration_seconds(Some("1h")), Some(3600));
        assert_eq!(parse_active_duration_seconds(Some("2d")), Some(172_800));
        assert_eq!(parse_active_duration_seconds(Some("45")), Some(2700));
        assert_eq!(parse_active_duration_seconds(Some("0m")), None);
        assert_eq!(
            active_duration_arg(&["--active".to_owned(), "1h".to_owned()], "--active"),
            Some("1h".to_owned())
        );
        assert_eq!(
            active_duration_arg(&["--active=2d".to_owned()], "--active"),
            Some("2d".to_owned())
        );
        assert_eq!(
            active_duration_arg(
                &["--active".to_owned(), "session-filter".to_owned()],
                "--active"
            ),
            None
        );
    }

    #[test]
    fn annotate_pane_matches_maw_js_precedence() {
        let fleet = BTreeSet::from([
            "101-mawjs".to_owned(),
            "112-fusion".to_owned(),
            "114-mawjs-no2".to_owned(),
        ]);
        let teams = BTreeMap::from([("%300".to_owned(), "scout @ iter-triage".to_owned())]);
        assert_eq!(
            annotate_pane(
                &TmuxLsPaneRef {
                    id: "%100".to_owned(),
                    target: "101-mawjs:0.0".to_owned(),
                    command: Some("claude".to_owned())
                },
                &fleet,
                &BTreeMap::new(),
            ),
            "fleet: mawjs"
        );
        assert_eq!(
            annotate_pane(
                &TmuxLsPaneRef {
                    id: "%101".to_owned(),
                    target: "114-mawjs-no2:0.0".to_owned(),
                    command: Some("claude".to_owned())
                },
                &fleet,
                &BTreeMap::new(),
            ),
            "fleet: mawjs-no2"
        );
        assert_eq!(
            annotate_pane(
                &TmuxLsPaneRef {
                    id: "%200".to_owned(),
                    target: "maw-view:0.0".to_owned(),
                    command: Some("claude".to_owned())
                },
                &fleet,
                &BTreeMap::new(),
            ),
            "view: maw-view"
        );
        assert_eq!(
            annotate_pane(
                &TmuxLsPaneRef {
                    id: "%201".to_owned(),
                    target: "mawjs-view:0.0".to_owned(),
                    command: Some("claude".to_owned())
                },
                &fleet,
                &BTreeMap::new(),
            ),
            "view: mawjs-view"
        );
        assert_eq!(
            annotate_pane(
                &TmuxLsPaneRef {
                    id: "%300".to_owned(),
                    target: "101-mawjs:0.1".to_owned(),
                    command: Some("bun".to_owned())
                },
                &fleet,
                &teams,
            ),
            "team: scout @ iter-triage"
        );
        assert_eq!(
            annotate_pane(
                &TmuxLsPaneRef {
                    id: "%600".to_owned(),
                    target: "view-foo:0.0".to_owned(),
                    command: Some("claude".to_owned())
                },
                &fleet,
                &BTreeMap::new(),
            ),
            "orphan"
        );
        assert_eq!(
            annotate_pane(
                &TmuxLsPaneRef {
                    id: "%700".to_owned(),
                    target: "any:0.0".to_owned(),
                    command: Some("bash".to_owned())
                },
                &BTreeSet::new(),
                &BTreeMap::new(),
            ),
            ""
        );
    }

    #[test]
    fn similar_oracle_candidates_preserve_org_slug_ambiguity() {
        let repos = vec![
            "/opt/Code/github.com/laris-co/pulse-oracle".to_owned(),
            "/opt/Code/github.com/Soul-Brews-Studio/pulse-oracle".to_owned(),
            "/opt/Code/github.com/Soul-Brews-Studio/pulse-oracle".to_owned(),
            "/opt/Code/github.com/Soul-Brews-Studio/other".to_owned(),
        ];
        assert_eq!(
            similar_oracle_candidates_from_repos("pulse", &repos),
            vec![
                "laris-co/pulse-oracle".to_owned(),
                "Soul-Brews-Studio/pulse-oracle".to_owned(),
            ]
        );
        assert!(similar_oracle_candidates_from_repos("x", &[]).is_empty());
    }

    #[test]
    fn split_window_locked_builds_maw_js_args() {
        let runner = FakeRunner::with_responses(vec![Ok(""), Ok(""), Ok("")]);
        let mut client = TmuxClient::new(runner);
        client
            .split_window_locked("main:0", &SplitWindowLockedOptions::default())
            .expect("default split ok");
        client
            .split_window_locked(
                "main:1",
                &SplitWindowLockedOptions {
                    vertical: Some(true),
                    pct: Some(33),
                    shell_command: Some("zsh".to_owned()),
                },
            )
            .expect("vertical split ok");
        client
            .split_window_locked(
                "main:2",
                &SplitWindowLockedOptions {
                    vertical: Some(false),
                    pct: Some(20),
                    shell_command: None,
                },
            )
            .expect("horizontal split ok");

        assert_eq!(
            client.runner.calls,
            vec![
                (
                    "split-window".to_owned(),
                    vec!["-t", "main:0"]
                        .into_iter()
                        .map(str::to_owned)
                        .collect()
                ),
                (
                    "split-window".to_owned(),
                    vec!["-t", "main:1", "-v", "-l", "33%", "zsh"]
                        .into_iter()
                        .map(str::to_owned)
                        .collect()
                ),
                (
                    "split-window".to_owned(),
                    vec!["-t", "main:2", "-h", "-l", "20%"]
                        .into_iter()
                        .map(str::to_owned)
                        .collect()
                ),
            ]
        );
    }

    #[test]
    fn tag_pane_sets_title_and_meta_with_auto_at_prefix() {
        let runner = FakeRunner::with_responses(vec![Ok(""), Ok(""), Ok("")]);
        let mut client = TmuxClient::new(runner);
        let meta = vec![
            ("agent-name".to_owned(), "scout".to_owned()),
            ("@role".to_owned(), "teammate".to_owned()),
        ];
        client
            .tag_pane("s:0.1", Some("oracle main"), &meta)
            .expect("tag pane ok");

        assert_eq!(
            client.runner.calls,
            vec![
                (
                    "select-pane".to_owned(),
                    vec!["-t", "s:0.1", "-T", "oracle main"]
                        .into_iter()
                        .map(str::to_owned)
                        .collect()
                ),
                (
                    "set-option".to_owned(),
                    vec!["-p", "-t", "s:0.1", "@agent-name", "scout"]
                        .into_iter()
                        .map(str::to_owned)
                        .collect()
                ),
                (
                    "set-option".to_owned(),
                    vec!["-p", "-t", "s:0.1", "@role", "teammate"]
                        .into_iter()
                        .map(str::to_owned)
                        .collect()
                ),
            ]
        );
    }

    #[test]
    fn read_pane_tags_parses_quoted_meta_options() {
        let runner = FakeRunner::with_responses(vec![
            Ok("oracle\n"),
            Ok("@agent-name \"scout\"\n@role teammate\n@quote \"say \\\"hi\\\"\"\nwindow-style default\n"),
        ]);
        let mut client = TmuxClient::new(runner);
        let tags = client.read_pane_tags("s:0.1").expect("read tags ok");
        assert_eq!(tags.title, "oracle");
        assert_eq!(
            tags.meta,
            BTreeMap::from([
                ("@agent-name".to_owned(), "scout".to_owned()),
                ("@quote".to_owned(), "say \"hi\"".to_owned()),
                ("@role".to_owned(), "teammate".to_owned()),
            ])
        );
        assert_eq!(client.runner.calls[0].0, "display-message");
        assert_eq!(client.runner.calls[1].0, "show-options");
    }

    #[test]
    fn send_text_uses_literal_path_and_retries_until_capture_clears() {
        let runner = FakeRunner::with_responses(vec![
            Ok("0"),
            Ok(""),
            Ok(""),
            Ok("\u{1b}[32m❯\u{1b}[0m deploy now\r"),
            Ok(""),
            Ok("\u{1b}[32m❯\u{1b}[0m \r"),
        ]);
        let mut client = TmuxClient::new(runner);
        let report = client
            .send_text("sess:oracle.0", "deploy now")
            .expect("send text ok");
        assert_eq!(
            report,
            SendTextReport {
                used_buffer: false,
                enter_attempts: 2,
                warned_pending: false,
            }
        );
        assert_eq!(client.runner.calls[0].0, "display-message");
        assert_eq!(
            client.runner.calls[1].1,
            vec!["-t", "sess:oracle.0", "-l", "deploy now"]
        );
        assert_eq!(
            client.runner.calls[2].1,
            vec!["-t", "sess:oracle.0", "Enter"]
        );
        assert_eq!(client.runner.calls[3].0, "capture-pane");
        assert_eq!(
            client.runner.calls[4].1,
            vec!["-t", "sess:oracle.0", "Enter"]
        );
        assert_eq!(client.runner.stdin_calls.len(), 0);
    }

    #[test]
    fn send_text_uses_buffer_path_for_multiline_or_long_payloads() {
        let long_text = "x".repeat(501);
        let runner = FakeRunner::with_responses(vec![Ok("0"), Ok(""), Ok(""), Ok(""), Ok("$ \r")]);
        let mut client = TmuxClient::new(runner);
        let report = client
            .send_text("sess:oracle.0", &long_text)
            .expect("send text ok");
        assert!(report.used_buffer);
        assert_eq!(report.enter_attempts, 1);
        assert_eq!(
            client.runner.stdin_calls,
            vec![("load-buffer".to_owned(), vec!["-".to_owned()], long_text,)]
        );
        assert_eq!(client.runner.calls[1].0, "paste-buffer");
    }

    #[test]
    fn send_text_reports_warning_after_max_pending_retries() {
        let runner = FakeRunner::with_responses(vec![
            Ok("0"),
            Ok(""),
            Ok(""),
            Ok("$ deploy"),
            Ok(""),
            Ok("$ deploy"),
            Ok(""),
            Ok("$ deploy"),
            Ok(""),
            Ok("$ deploy"),
        ]);
        let mut client = TmuxClient::new(runner);
        let report = client
            .send_text("sess:oracle.0", "deploy")
            .expect("send text ok");
        assert_eq!(report.enter_attempts, 4);
        assert!(report.warned_pending);
        assert_eq!(
            client
                .runner
                .calls
                .iter()
                .filter(|(subcommand, args)| subcommand == "send-keys"
                    && args
                        == &vec![
                            "-t".to_owned(),
                            "sess:oracle.0".to_owned(),
                            "Enter".to_owned()
                        ])
                .count(),
            4
        );
    }

    #[test]
    fn capture_resize_and_exit_mode_match_maw_js_runtime_helpers() {
        let runner = FakeRunner::with_responses(vec![
            Ok("captured"),
            Err(TmuxError::new("ignored")),
            Ok("1"),
            Ok(""),
        ]);
        let mut client = TmuxClient::new(runner);
        assert_eq!(client.capture("%1", Some(5)).expect("capture"), "captured");
        client.resize_pane("%1", 0, 999);
        assert!(client.exit_mode_if_needed("%1").expect("exit mode"));

        assert_eq!(client.runner.calls[0].0, "capture-pane");
        assert_eq!(
            client.runner.calls[0].1,
            vec!["-t", "%1", "-e", "-p", "-S", "-5"]
        );
        assert_eq!(client.runner.calls[1].0, "resize-pane");
        assert_eq!(
            client.runner.calls[1].1,
            vec!["-t", "%1", "-x", "1", "-y", "200"]
        );
        assert_eq!(client.runner.calls[2].0, "display-message");
        assert_eq!(client.runner.calls[3].1, vec!["-t", "%1", "-X", "cancel"]);
    }

    #[test]
    fn pending_input_detection_matches_maw_js_prompt_heuristic() {
        assert!(pane_input_pending_from_capture("old\n$ maw hey oracle"));
        assert!(pane_input_pending_from_capture(
            "\u{1b}[32m❯\u{1b}[0m cargo test"
        ));
        assert!(!pane_input_pending_from_capture("old\n$ "));
        assert!(!pane_input_pending_from_capture("command output only"));
        assert_eq!(strip_tmux_ansi("a\u{1b}[31mred\u{1b}[0m"), "ared");
    }

    #[test]
    fn client_fail_soft_lists_and_records_runner_args() {
        let runner =
            FakeRunner::with_responses(vec![Ok("s1\ns2\n"), Err(TmuxError::new("no server"))]);
        let mut client = TmuxClient::new(runner);
        assert_eq!(client.list_session_names(), vec!["s1", "s2"]);
        assert!(client.list_all().is_empty());
        assert_eq!(client.runner.calls[0].0, "list-sessions");
        assert_eq!(client.runner.calls[1].0, "list-windows");
    }

    #[test]
    fn client_listing_helpers_parse_outputs_and_fail_soft_where_expected() {
        let runner = FakeRunner::with_responses(vec![
            Ok("0:agent:1\n1:logs:0\n"),
            Ok("%1\n\n%2\n"),
            Err(TmuxError::new("no panes")),
            Ok("%1|||zsh|||s:agent.0|||main|||42|||/repo|||900\n"),
            Ok(""),
            Err(TmuxError::new("missing")),
        ]);
        let mut client = TmuxClient::new(runner);

        assert_eq!(
            client.list_windows("s").expect("windows parse"),
            vec![
                TmuxWindow {
                    index: 0,
                    name: "agent".to_owned(),
                    active: true,
                    cwd: None,
                },
                TmuxWindow {
                    index: 1,
                    name: "logs".to_owned(),
                    active: false,
                    cwd: None,
                },
            ]
        );
        assert_eq!(
            client.list_pane_ids(),
            BTreeSet::from(["%1".to_owned(), "%2".to_owned()])
        );
        assert!(client.list_pane_ids().is_empty());
        assert_eq!(
            client.list_panes(),
            vec![TmuxPane {
                id: "%1".to_owned(),
                command: "zsh".to_owned(),
                target: "s:agent.0".to_owned(),
                title: "main".to_owned(),
                pid: Some(42),
                cwd: Some("/repo".to_owned()),
                last_activity: Some(900),
            }]
        );
        assert!(client.has_session("s"));
        assert!(!client.has_session("ghost"));

        assert_eq!(client.runner.calls[0].0, "list-windows");
        assert_eq!(client.runner.calls[1].0, "list-panes");
        assert_eq!(client.runner.calls[2].0, "list-panes");
        assert_eq!(client.runner.calls[3].0, "list-panes");
        assert_eq!(client.runner.calls[4].0, "has-session");
        assert_eq!(client.runner.calls[5].0, "has-session");
    }

    #[test]
    fn client_grouped_session_and_best_effort_mutators_match_arg_order() {
        let runner = FakeRunner::with_responses(vec![
            Ok(""),
            Ok(""),
            Err(TmuxError::new("select ignored")),
            Ok(""),
            Ok(""),
            Ok(""),
            Ok(""),
            Ok(""),
        ]);
        let mut client = TmuxClient::new(runner);

        client
            .new_grouped_session(
                "parent",
                "child",
                &GroupedSessionOptions {
                    cols: Some(120),
                    rows: Some(40),
                    window: Some("agent".to_owned()),
                    window_size: Some("manual".to_owned()),
                },
            )
            .expect("grouped session ok");
        client.select_window("child:agent");
        client.switch_client("child");
        client.kill_window("child:logs");
        client.kill_pane("%2");
        client.set("child", "@maw", "on");

        assert_eq!(
            client.runner.calls,
            vec![
                (
                    "new-session".to_owned(),
                    vec!["-d", "-t", "parent", "-s", "child", "-x", "120", "-y", "40"]
                        .into_iter()
                        .map(str::to_owned)
                        .collect()
                ),
                (
                    "set-option".to_owned(),
                    vec!["-t", "child", "window-size", "manual"]
                        .into_iter()
                        .map(str::to_owned)
                        .collect()
                ),
                (
                    "select-window".to_owned(),
                    vec!["-t", "child:agent"]
                        .into_iter()
                        .map(str::to_owned)
                        .collect()
                ),
                (
                    "select-window".to_owned(),
                    vec!["-t", "child:agent"]
                        .into_iter()
                        .map(str::to_owned)
                        .collect()
                ),
                (
                    "switch-client".to_owned(),
                    vec!["-t", "child"].into_iter().map(str::to_owned).collect()
                ),
                (
                    "kill-window".to_owned(),
                    vec!["-t", "child:logs"]
                        .into_iter()
                        .map(str::to_owned)
                        .collect()
                ),
                (
                    "kill-pane".to_owned(),
                    vec!["-t", "%2"].into_iter().map(str::to_owned).collect()
                ),
                (
                    "set".to_owned(),
                    vec!["-t", "child", "@maw", "on"]
                        .into_iter()
                        .map(str::to_owned)
                        .collect()
                ),
            ]
        );
    }

    #[test]
    fn client_split_layout_resize_and_environment_helpers_match_arg_order() {
        let runner = FakeRunner::with_responses(vec![Ok(""), Ok(""), Ok(""), Ok(""), Ok("")]);
        let mut client = TmuxClient::new(runner);

        client
            .split_pane_action(
                "s:0.1",
                &TmuxSplitActionOptions {
                    vertical: true,
                    pct: 25.0,
                    command: None,
                },
            )
            .expect("split pane action ok");
        client
            .select_layout("s:0", "tiled")
            .expect("select layout ok");
        client
            .select_valid_layout("s:0.1", "even-horizontal")
            .expect("valid layout ok");
        client.resize_window("s:0", 999, 0);
        client
            .set_environment("s", "MAW_MODE", "test")
            .expect("set env ok");

        assert_eq!(
            client.runner.calls,
            vec![
                (
                    "split-window".to_owned(),
                    vec!["-v", "-l", "25%", "-t", "s:0.1"]
                        .into_iter()
                        .map(str::to_owned)
                        .collect()
                ),
                (
                    "select-layout".to_owned(),
                    vec!["-t", "s:0", "tiled"]
                        .into_iter()
                        .map(str::to_owned)
                        .collect()
                ),
                (
                    "select-layout".to_owned(),
                    vec!["-t", "s:0", "even-horizontal"]
                        .into_iter()
                        .map(str::to_owned)
                        .collect()
                ),
                (
                    "resize-window".to_owned(),
                    vec!["-t", "s:0", "-x", "500", "-y", "1"]
                        .into_iter()
                        .map(str::to_owned)
                        .collect()
                ),
                (
                    "set-environment".to_owned(),
                    vec!["-t", "s", "MAW_MODE", "test"]
                        .into_iter()
                        .map(str::to_owned)
                        .collect()
                ),
            ]
        );
    }

    #[test]
    fn runner_default_stdin_and_constructor_paths_are_testable_without_tmux_io() {
        struct RunOnlyRunner {
            calls: Vec<(String, Vec<String>)>,
        }

        impl TmuxRunner for RunOnlyRunner {
            fn run(&mut self, subcommand: &str, args: &[String]) -> Result<String, TmuxError> {
                self.calls.push((subcommand.to_owned(), args.to_vec()));
                Ok("fallback".to_owned())
            }
        }

        let mut runner = RunOnlyRunner { calls: Vec::new() };
        assert_eq!(
            runner
                .run_with_stdin("load-buffer", &["-".to_owned()], b"ignored")
                .expect("default stdin delegates"),
            "fallback"
        );
        assert_eq!(
            runner.calls,
            vec![("load-buffer".to_owned(), vec!["-".to_owned()])]
        );

        assert_eq!(
            CommandTmuxRunner::new().argv("display-message", &[]),
            vec![OsString::from("tmux"), OsString::from("display-message")]
        );
        assert_eq!(
            TmuxClient::local().runner.argv(
                "list-sessions",
                &["-F".to_owned(), "#{session_name}".to_owned()]
            ),
            vec![
                OsString::from("tmux"),
                OsString::from("list-sessions"),
                OsString::from("-F"),
                OsString::from("#{session_name}"),
            ]
        );
        assert_eq!(
            TmuxClient::local_with_socket("/tmp/maw.sock")
                .runner
                .argv("list-panes", &[]),
            vec![
                OsString::from("tmux"),
                OsString::from("-S"),
                OsString::from("/tmp/maw.sock"),
                OsString::from("list-panes"),
            ]
        );
    }

    #[test]
    fn command_runner_process_adapter_handles_success_stdin_and_errors_without_tmux() {
        let mut printf_runner = CommandTmuxRunner::with_program("/usr/bin/printf");
        assert_eq!(
            printf_runner
                .run("hello %s", &["world".to_owned()])
                .expect("printf succeeds"),
            "hello world"
        );

        let mut cat_runner = CommandTmuxRunner::with_program("/bin/cat");
        assert_eq!(
            cat_runner
                .run_with_stdin("-", &[], b"buffer text")
                .expect("cat echoes stdin"),
            "buffer text"
        );

        let mut shell_runner = CommandTmuxRunner::with_program("/bin/sh");
        let error = shell_runner
            .run("-c", &["printf denied >&2; exit 7".to_owned()])
            .expect_err("shell exits non-zero");
        assert_eq!(error.message, "tmux exited with status 7: denied");

        let mut missing_runner = CommandTmuxRunner::with_program("/definitely/not/a/tmux");
        let error = missing_runner
            .run("list-sessions", &[])
            .expect_err("missing program");
        assert!(error
            .message
            .contains("failed to execute /definitely/not/a/tmux"));

        let mut quiet_failure_runner = CommandTmuxRunner::with_program("/bin/sh");
        let error = quiet_failure_runner
            .run("-c", &["exit 9".to_owned()])
            .expect_err("empty stderr/stdout reports status only");
        assert_eq!(error.message, "tmux exited with status 9");
    }

    #[test]
    fn error_display_and_tracker_clear_cover_diagnostic_paths() {
        let error = TmuxError::new("tmux failed");
        assert_eq!(error.to_string(), "tmux failed");

        let mut tracker = TmuxSendTracker::default();
        assert_eq!(tracker.check("%1", 1_000, false), SendThrottle::Allowed);
        assert!(tracker.get("%1").is_some());
        tracker.clear();
        assert_eq!(tracker.get("%1"), None);
    }

    #[test]
    fn send_action_empty_throttled_and_tmux_lookup_error_paths_are_safe() {
        let mut client = TmuxClient::new(FakeRunner::default());
        let mut tracker = TmuxSendTracker::default();
        let error = client
            .send_command_to_pane(
                &mut tracker,
                "%1",
                "",
                &TmuxSendCommandOptions::default(),
                1_000,
            )
            .expect_err("empty command rejected before tmux lookup");
        assert!(error.message.contains("usage: maw tmux send"));
        assert!(client.runner.calls.is_empty());

        let mut client = TmuxClient::new(FakeRunner::default());
        let mut tracker = TmuxSendTracker::default();
        tracker.set(
            "%1",
            SendTrackerEntry {
                last_ts: 1_000,
                count: 1,
                window_start: 1_000,
            },
        );
        let outcome = client
            .send_command_to_pane(
                &mut tracker,
                "%1",
                "echo two",
                &TmuxSendCommandOptions::default(),
                1_100,
            )
            .expect("cooldown reported without tmux lookup");
        assert_eq!(
            outcome,
            TmuxSendCommandOutcome::Throttled(SendThrottle::Cooldown { cooldown_ms: 500 })
        );

        let runner = FakeRunner::with_responses(vec![Err(TmuxError::new("pane gone"))]);
        let mut client = TmuxClient::new(runner);
        let mut tracker = TmuxSendTracker::default();
        let error = client
            .send_command_to_pane(
                &mut tracker,
                "%9",
                "echo safe",
                &TmuxSendCommandOptions::default(),
                2_000,
            )
            .expect_err("display-message error propagates");
        assert_eq!(error.message, "pane gone");
        assert_eq!(client.runner.calls[0].0, "display-message");
    }

    #[test]
    fn client_error_branches_preserve_context_and_do_not_require_tmux() {
        let target = TmuxKillTarget {
            resolved: "demo:1.2".to_owned(),
            source: "session:w.p".to_owned(),
        };
        let runner = FakeRunner::with_responses(vec![Err(TmuxError::new("session denied"))]);
        let mut client = TmuxClient::new(runner);
        let error = client
            .kill_target_action(
                &target,
                &BTreeSet::new(),
                &TmuxKillCommandOptions {
                    force: false,
                    session: true,
                },
            )
            .expect_err("session kill wraps runner error");
        assert_eq!(
            error.message,
            "kill failed for 'demo:1.2' (from session:w.p): session denied"
        );

        let runner =
            FakeRunner::with_responses(vec![Ok("1"), Err(TmuxError::new("not in a mode"))]);
        let mut client = TmuxClient::new(runner);
        assert!(!client
            .exit_mode_if_needed("%1")
            .expect("stale copy-mode cancellation is benign"));

        let runner = FakeRunner::with_responses(vec![Ok("1"), Err(TmuxError::new("server lost"))]);
        let mut client = TmuxClient::new(runner);
        let error = client
            .exit_mode_if_needed("%1")
            .expect_err("non-benign cancellation error propagates");
        assert_eq!(error.message, "server lost");
    }

    #[test]
    fn pure_edge_cases_cover_malformed_ansi_targets_and_duration_inputs() {
        assert_eq!(
            strip_tmux_ansi("left\u{1b}[2Kright\u{1b}[1G!"),
            "leftright!"
        );
        assert_eq!(strip_tmux_ansi("left\u{1b}[?right"), "left\u{1b}[?right");
        assert_eq!(strip_tmux_ansi("wide λ"), "wide λ");
        assert!(!pane_input_pending_from_capture("\n \n\t"));
        assert!(contains_word("please rm now", "rm"));
        assert!(!contains_word("farmhouse", "rm"));
        assert!(!check_destructive("program").destructive);
        assert!(!has_redirect("echo hi >", false));
        assert!(!has_redirect("echo hi >>", true));
        assert!(!is_claude_like_pane(Some(".")));
        assert!(!is_claude_like_pane(Some("1.")));
        assert!(!is_claude_like_pane(None));
        assert_eq!(tmux_window_target("session.window.1"), "session.window.1");
        assert_eq!(tmux_window_target("session:win.x"), "session:win.x");
        assert_eq!(
            parse_session_activity_list("s\t123\nbad\tnope\n"),
            BTreeMap::from([("s".to_owned(), 123)])
        );
        assert_eq!(parse_active_duration_seconds(Some("10s")), Some(10));
        assert_eq!(parse_active_duration_seconds(Some("15x")), None);
        assert_eq!(parse_active_duration_seconds(Some("")), None);
        assert_eq!(
            active_duration_arg(&["--active".to_owned()], "--active"),
            None
        );
        assert_eq!(
            active_duration_arg(&["--active=15m".to_owned()], "--active"),
            Some("15m".to_owned())
        );
        assert_eq!(
            active_duration_arg(&["--active=0m".to_owned()], "--active"),
            None
        );
        assert_eq!(
            active_duration_arg(&["--active".to_owned(), "-v".to_owned()], "--active"),
            None
        );
        assert_eq!(format_session_created(Some(1)), "1970-01-01 00:00:01");
        assert_eq!(
            similar_oracle_candidates_from_repos("plain", &["plain-oracle".to_owned()]),
            vec!["plain-oracle"]
        );
        assert_eq!(
            tmux_shell_command(Some(""), "list-panes", &[]),
            "tmux -S '' list-panes"
        );
        assert_eq!(
            parse_pane_tag_options("@broken\nnot-meta value\n"),
            BTreeMap::new()
        );
        assert_eq!(
            parse_pane_tag_options("@quoted \"value\\\\tail\\\\\""),
            BTreeMap::from([("@quoted".to_owned(), "value\\tail\\".to_owned())])
        );
        assert_eq!(parse_list_all_windows("too|||short\n"), Vec::new());
        assert!(pane_target_candidates_from_list_panes_output("||||||||||||").is_empty());
        assert_eq!(basename("///"), "///");
        assert!(worktree_names_from_cwd("").is_empty());
        assert_eq!(
            worktree_names_from_cwd("/tmp/project-oracle.wt-7-codex")
                .into_iter()
                .map(|(name, source)| format!("{source}:{name}"))
                .collect::<Vec<_>>(),
            vec![
                "worktree-dir:project-oracle.wt-7-codex",
                "worktree-role:codex",
                "worktree-alias:project-codex",
            ]
        );
        assert_eq!(parse_tmux_pane_target(":win.1"), None);
        assert_eq!(parse_tmux_pane_target("session:.1"), None);
        assert_eq!(parse_tmux_pane_target("session:win."), None);
    }

    #[test]
    fn tmux_client_remaining_simple_queries_use_runner_outputs() {
        let runner = FakeRunner::with_responses(vec![
            Ok("1:main:1\n2:logs:0\n"),
            Ok("bash\nzsh\n"),
            Ok("vim\t/tmp/repo\n"),
            Ok("pane title\n"),
            Ok("@role worker\n@quoted \"hello\\\\ world\"\nwindow-option ignored\nmalformed\n"),
        ]);
        let mut client = TmuxClient::new(runner);

        assert_eq!(
            client.list_windows("demo").expect("windows parse"),
            vec![
                TmuxWindow {
                    index: 1,
                    name: "main".to_owned(),
                    active: true,
                    cwd: None,
                },
                TmuxWindow {
                    index: 2,
                    name: "logs".to_owned(),
                    active: false,
                    cwd: None,
                },
            ]
        );
        assert_eq!(client.get_pane_command("%1").expect("command"), "bash");
        assert_eq!(
            client.get_pane_info("%1").expect("pane info"),
            ("vim".to_owned(), "/tmp/repo".to_owned())
        );
        assert_eq!(
            client.read_pane_tags("%1").expect("tags"),
            PaneTags {
                title: "pane title".to_owned(),
                meta: BTreeMap::from([
                    ("@quoted".to_owned(), "hello\\ world".to_owned()),
                    ("@role".to_owned(), "worker".to_owned()),
                ]),
            }
        );
    }

    #[test]
    fn tmux_client_tag_pane_writes_title_and_normalized_metadata() {
        let runner = FakeRunner::with_responses(vec![Ok(""), Ok(""), Ok("")]);
        let mut client = TmuxClient::new(runner);

        client
            .tag_pane(
                "%2",
                Some("worker"),
                &[
                    ("role".to_owned(), "executor".to_owned()),
                    ("@node".to_owned(), "alpha".to_owned()),
                ],
            )
            .expect("tag writes");

        assert_eq!(
            client.runner.calls,
            vec![
                (
                    "select-pane".to_owned(),
                    vec![
                        "-t".to_owned(),
                        "%2".to_owned(),
                        "-T".to_owned(),
                        "worker".to_owned(),
                    ],
                ),
                (
                    "set-option".to_owned(),
                    vec![
                        "-p".to_owned(),
                        "-t".to_owned(),
                        "%2".to_owned(),
                        "@role".to_owned(),
                        "executor".to_owned(),
                    ],
                ),
                (
                    "set-option".to_owned(),
                    vec![
                        "-p".to_owned(),
                        "-t".to_owned(),
                        "%2".to_owned(),
                        "@node".to_owned(),
                        "alpha".to_owned(),
                    ],
                ),
            ]
        );
    }

    #[test]
    fn tmux_client_simple_query_and_tag_errors_propagate_runner_context() {
        let mut client = TmuxClient::new(FakeRunner::with_responses(vec![Err(TmuxError::new(
            "no windows",
        ))]));
        assert_eq!(
            client.list_windows("demo").expect_err("list error").message,
            "no windows"
        );

        let mut client = TmuxClient::new(FakeRunner::with_responses(vec![Err(TmuxError::new(
            "no command",
        ))]));
        assert_eq!(
            client
                .get_pane_command("%1")
                .expect_err("command error")
                .message,
            "no command"
        );

        let mut client = TmuxClient::new(FakeRunner::with_responses(vec![Err(TmuxError::new(
            "no info",
        ))]));
        assert_eq!(
            client.get_pane_info("%1").expect_err("info error").message,
            "no info"
        );

        let mut client = TmuxClient::new(FakeRunner::with_responses(vec![Err(TmuxError::new(
            "title denied",
        ))]));
        assert_eq!(
            client
                .tag_pane("%1", Some("title"), &[])
                .expect_err("title error")
                .message,
            "title denied"
        );

        let mut client = TmuxClient::new(FakeRunner::with_responses(vec![
            Ok(""),
            Err(TmuxError::new("meta denied")),
        ]));
        assert_eq!(
            client
                .tag_pane(
                    "%1",
                    Some("title"),
                    &[("role".to_owned(), "worker".to_owned())]
                )
                .expect_err("meta error")
                .message,
            "meta denied"
        );

        let mut client = TmuxClient::new(FakeRunner::with_responses(vec![Err(TmuxError::new(
            "no title",
        ))]));
        assert_eq!(
            client
                .read_pane_tags("%1")
                .expect_err("title read error")
                .message,
            "no title"
        );
    }

    #[test]
    fn fake_runner_no_response_and_resolution_none_paths_are_explicit() {
        let mut client = TmuxClient::new(FakeRunner::default());
        let error = client
            .list_windows("missing")
            .expect_err("empty fake runner reports no response");
        assert_eq!(error.message, "no response");

        let target = resolve_kill_target_with_pane_fallback(
            "ghost",
            "ghost",
            "session-name",
            false,
            "%1|||demo:1.1|||worker|||role|||/tmp/repo.wt-1-codex\n",
        )
        .expect("no pane fallback preserves session target");
        assert_eq!(
            target,
            TmuxKillTarget {
                resolved: "ghost".to_owned(),
                source: "session-name".to_owned(),
            }
        );

        assert_eq!(
            resolve_pane_target_from_candidates("ghost", &[]),
            PaneTargetResolution::None
        );
        assert_eq!(
            format_pane_ambiguity_error(
                "worker",
                &[
                    PaneTargetCandidate {
                        name: "worker".to_owned(),
                        resolved: "%1".to_owned(),
                        source: "pane-title".to_owned(),
                        target: String::new(),
                    },
                    PaneTargetCandidate {
                        name: "worker".to_owned(),
                        resolved: "%2".to_owned(),
                        source: "tile-role".to_owned(),
                        target: "demo:1.2".to_owned(),
                    },
                ],
            ),
            "'worker' is ambiguous — matches 2 panes:\n    • worker → %1 [pane-title]\n    • worker → %2 (demo:1.2) [tile-role]\n  use the pane id or full session:window.pane target"
        );
        assert_eq!(unescape_tmux_quoted_value("tail\\"), "tail\\");
    }

    #[test]
    fn attach_recovery_includes_fleet_window_clone_label_and_dedupes_repo_candidate() {
        let fleet_entries = vec![AttachRecoveryFleetEntry {
            session: "101-mawjs".to_owned(),
            first_window_name: Some("pulse-oracle".to_owned()),
            repo: Some("pulse-oracle".to_owned()),
        }];
        let cloned_repos = vec![
            "/opt/Code/github.com/Soul-Brews-Studio/pulse-oracle".to_owned(),
            "/opt/Code/github.com/Soul-Brews-Studio/pulse-helper-oracle".to_owned(),
        ];

        assert_eq!(
            attach_recovery_candidates(
                "pulse",
                "101-mawjs",
                "fleet-window (pulse)",
                &fleet_entries,
                &cloned_repos,
            ),
            vec![
                AttachRecoveryCandidate {
                    oracle: "pulse".to_owned(),
                    label: "pulse-oracle (cloned)".to_owned(),
                },
                AttachRecoveryCandidate {
                    oracle: "Soul-Brews-Studio/pulse-oracle".to_owned(),
                    label: "Soul-Brews-Studio/pulse-oracle".to_owned(),
                },
                AttachRecoveryCandidate {
                    oracle: "Soul-Brews-Studio/pulse-helper-oracle".to_owned(),
                    label: "Soul-Brews-Studio/pulse-helper-oracle".to_owned(),
                },
            ]
        );
    }
}

/// Parsed `session:window.pane` tmux target parts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TmuxPaneTargetParts {
    pub session: String,
    pub window: String,
    pub pane: String,
}

/// Live tmux pane projection used by discovery inventory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoverLivePane {
    pub source: String,
    pub id: String,
    pub target: String,
    pub session: String,
    pub window: String,
    pub pane: String,
    pub command: Option<String>,
    pub title: Option<String>,
    pub pid: Option<u32>,
    pub cwd: Option<String>,
    pub last_activity: Option<u64>,
    pub awake: bool,
    pub matches: Vec<String>,
}

/// Result of pure live-state projection from already-listed tmux panes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TmuxLiveStateResult {
    pub source: String,
    pub live: Vec<DiscoverLivePane>,
    pub warnings: Vec<String>,
}

/// Peer target decorated with tmux liveness metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerTargetWithLive {
    pub name: Option<String>,
    pub url: String,
    pub source: maw_peer::PeerSourceKind,
    pub node: Option<String>,
    pub oracle: Option<String>,
    pub awake: bool,
    pub live_targets: Vec<String>,
    pub live_sessions: Vec<String>,
}

/// Parse a tmux pane target shaped like `session:window.pane`.
#[must_use]
pub fn parse_tmux_pane_target(target: &str) -> Option<TmuxPaneTargetParts> {
    let colon = target.find(':')?;
    let dot = target.rfind('.')?;
    if colon == 0 || dot <= colon + 1 || dot == target.len() - 1 {
        return None;
    }
    Some(TmuxPaneTargetParts {
        session: target[..colon].to_owned(),
        window: target[colon + 1..dot].to_owned(),
        pane: target[dot + 1..].to_owned(),
    })
}

/// Resolve live tmux state from already-collected panes and peer targets.
#[must_use]
pub fn resolve_tmux_live_state(
    peers: &[maw_peer::PeerTarget],
    panes: &[TmuxPane],
) -> TmuxLiveStateResult {
    let mut live = panes
        .iter()
        .map(|pane| tmux_pane_to_live_pane(pane, peers))
        .collect::<Vec<_>>();
    live.sort_by(|left, right| left.target.cmp(&right.target));
    TmuxLiveStateResult {
        source: "tmux".to_owned(),
        live,
        warnings: Vec::new(),
    }
}

/// Mark peer targets awake when their configured signals match live tmux panes.
#[must_use]
pub fn mark_peer_targets_live(
    peers: &[maw_peer::PeerTarget],
    live: &[DiscoverLivePane],
) -> Vec<PeerTargetWithLive> {
    peers
        .iter()
        .map(|peer| {
            let peer_signals = normalized_peer_signals(peer);
            let matching = live
                .iter()
                .filter(|pane| {
                    pane_signals(pane)
                        .iter()
                        .any(|signal| peer_signals.iter().any(|peer_signal| peer_signal == signal))
                })
                .collect::<Vec<_>>();
            PeerTargetWithLive {
                name: peer.name.clone(),
                url: peer.url.clone(),
                source: peer.source,
                node: peer.node.clone(),
                oracle: peer.oracle.clone(),
                awake: !matching.is_empty(),
                live_targets: matching.iter().map(|pane| pane.target.clone()).collect(),
                live_sessions: unique_preserve_order(
                    matching.iter().map(|pane| pane.session.clone()).collect(),
                ),
            }
        })
        .collect()
}

fn tmux_pane_to_live_pane(pane: &TmuxPane, peers: &[maw_peer::PeerTarget]) -> DiscoverLivePane {
    let parsed =
        parse_tmux_pane_target(&pane.target).unwrap_or_else(|| fallback_target_parts(&pane.target));
    let mut live = DiscoverLivePane {
        source: "tmux".to_owned(),
        id: pane.id.clone(),
        target: pane.target.clone(),
        session: parsed.session,
        window: parsed.window,
        pane: parsed.pane,
        command: empty_to_none(&pane.command),
        title: empty_to_none(&pane.title),
        pid: pane.pid,
        cwd: pane.cwd.as_deref().and_then(empty_to_none),
        last_activity: pane.last_activity,
        awake: true,
        matches: Vec::new(),
    };
    let live_signals = pane_signals(&live);
    live.matches = peers
        .iter()
        .filter(|peer| {
            let peer_signals = normalized_peer_signals(peer);
            live_signals
                .iter()
                .any(|signal| peer_signals.iter().any(|peer_signal| peer_signal == signal))
        })
        .map(|peer| {
            peer.name
                .clone()
                .or_else(|| peer.node.clone())
                .or_else(|| peer.oracle.clone())
                .unwrap_or_else(|| peer.url.clone())
        })
        .collect();
    live
}

fn fallback_target_parts(target: &str) -> TmuxPaneTargetParts {
    let session = target
        .split_once(':')
        .map_or(target, |(session, _)| session);
    TmuxPaneTargetParts {
        session: session.to_owned(),
        window: String::new(),
        pane: String::new(),
    }
}

fn pane_signals(pane: &DiscoverLivePane) -> Vec<String> {
    let mut signals = Vec::new();
    signals.extend(normalized_aliases(Some(&pane.session)));
    signals.extend(normalized_aliases(Some(&pane.window)));
    signals.extend(normalized_aliases(pane.title.as_deref()));
    if let Some(cwd) = pane.cwd.as_deref().and_then(path_basename) {
        signals.extend(normalized_aliases(Some(cwd)));
    }
    signals
}

fn normalized_peer_signals(peer: &maw_peer::PeerTarget) -> Vec<String> {
    let mut signals = Vec::new();
    signals.extend(normalized_aliases(peer.name.as_deref()));
    signals.extend(normalized_aliases(peer.node.as_deref()));
    signals.extend(normalized_aliases(peer.oracle.as_deref()));
    signals
}

fn normalized_aliases(value: Option<&str>) -> Vec<String> {
    let Some(normalized) = normalize_signal(value) else {
        return Vec::new();
    };
    let without_numeric = strip_numeric_prefix(&normalized).to_owned();
    let without_oracle = strip_oracle_suffix(&normalized).to_owned();
    let without_both = strip_oracle_suffix(strip_numeric_prefix(&normalized)).to_owned();
    unique_preserve_order(vec![
        normalized,
        without_numeric,
        without_oracle,
        without_both,
    ])
    .into_iter()
    .filter(|value| !value.is_empty())
    .collect()
}

fn normalize_signal(value: Option<&str>) -> Option<String> {
    let trimmed = value?.trim().to_lowercase();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn strip_numeric_prefix(value: &str) -> &str {
    let Some((prefix, rest)) = value.split_once('-') else {
        return value;
    };
    if !prefix.is_empty() && prefix.bytes().all(|byte| byte.is_ascii_digit()) {
        rest
    } else {
        value
    }
}

fn strip_oracle_suffix(value: &str) -> &str {
    value.strip_suffix("-oracle").unwrap_or(value)
}

fn path_basename(path: &str) -> Option<&str> {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|part| !part.is_empty())
}

fn empty_to_none(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn unique_preserve_order(values: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    for value in values {
        if !out.iter().any(|existing| existing == &value) {
            out.push(value);
        }
    }
    out
}
