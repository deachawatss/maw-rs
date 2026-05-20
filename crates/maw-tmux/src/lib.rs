//! Testable tmux command and parser adapter for maw-rs.
//!
//! This crate ports the deterministic parts of maw-js `src/core/transport/tmux-class.ts`:
//! shell-safe command construction plus parsing of `list-windows` / `list-panes` output.
//! Real process execution is intentionally injected through [`TmuxRunner`].

use std::{collections::BTreeSet, error::Error, ffi::OsString, fmt, process::Command};

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
        let command_line = self.argv(subcommand, args);
        let Some((program, rest)) = command_line.split_first() else {
            return Err(TmuxError::new("missing tmux program"));
        };
        let output = Command::new(program).args(rest).output().map_err(|error| {
            TmuxError::new(format!(
                "failed to execute {}: {error}",
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
        responses: Vec<Result<String, TmuxError>>,
    }

    impl FakeRunner {
        fn with_responses(responses: Vec<Result<&str, TmuxError>>) -> Self {
            Self {
                calls: Vec::new(),
                responses: responses
                    .into_iter()
                    .map(|response| response.map(str::to_owned))
                    .collect(),
            }
        }
    }

    impl TmuxRunner for FakeRunner {
        fn run(&mut self, subcommand: &str, args: &[String]) -> Result<String, TmuxError> {
            self.calls.push((subcommand.to_owned(), args.to_vec()));
            if self.responses.is_empty() {
                return Err(TmuxError::new("no response"));
            }
            self.responses.remove(0)
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
    fn client_fail_soft_lists_and_records_runner_args() {
        let runner =
            FakeRunner::with_responses(vec![Ok("s1\ns2\n"), Err(TmuxError::new("no server"))]);
        let mut client = TmuxClient::new(runner);
        assert_eq!(client.list_session_names(), vec!["s1", "s2"]);
        assert!(client.list_all().is_empty());
        assert_eq!(client.runner.calls[0].0, "list-sessions");
        assert_eq!(client.runner.calls[1].0, "list-windows");
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
