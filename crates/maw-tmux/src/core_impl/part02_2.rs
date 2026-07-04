impl<R> TmuxClient<R>
where
    R: TmuxRunner,
{

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
        write_pane_title(&mut self.runner, target, title)?;
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
            .run("send-keys", &tmux_send_keys_literal_args(target, text))
            .map(|_| ())
    }

    /// Send one Enter key through `tmux send-keys`.
    ///
    /// # Errors
    ///
    /// Returns the runner error when tmux rejects the request.
    pub fn send_enter(&mut self, target: &str) -> Result<(), TmuxError> {
        self.runner
            .run("send-keys", &tmux_send_enter_args(target))
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
    /// # Errors
    ///
    /// Returns the first tmux error from readiness polling, mode exit, text placement, paste, or
    /// Enter send.
    pub fn send_text(&mut self, target: &str, text: &str) -> Result<SendTextReport, TmuxError> {
        let config = self.submit_config_for_target(target);
        self.send_text_with_config_and_sleeper(target, text, config, std::thread::sleep)
    }

    /// Smart text sending with explicit engine timing.
    ///
    /// # Errors
    ///
    /// Returns the first tmux error from readiness polling, mode exit, text placement, paste, or
    /// Enter send.
    pub fn send_text_with_config(
        &mut self,
        target: &str,
        text: &str,
        config: SubmitConfig,
    ) -> Result<SendTextReport, TmuxError> {
        self.send_text_with_config_and_sleeper(target, text, config, std::thread::sleep)
    }

    /// Smart text sending with explicit timing and injected sleep.
    ///
    /// # Errors
    ///
    /// Returns the first tmux error from readiness polling, mode exit, text placement, paste, or
    /// Enter send.
    #[doc(hidden)]
    pub fn send_text_with_config_and_sleeper<F>(
        &mut self,
        target: &str,
        text: &str,
        config: SubmitConfig,
        mut sleep: F,
    ) -> Result<SendTextReport, TmuxError>
    where
        F: FnMut(std::time::Duration),
    {
        match self.readiness_gate(target, config, &mut sleep)? {
            ReadinessResult::Ready => self.send_text_body_with_sleeper(target, text, config, sleep),
            ReadinessResult::Timeout => Err(TmuxError::new(format!(
                "pane '{target}' did not show a prompt before readiness timeout"
            ))),
            ReadinessResult::Busy => Err(TmuxError::new(format!(
                "pane '{target}' is busy; prompt is not visible"
            ))),
        }
    }

    /// Poll a pane until its last non-empty captured line shows a prompt.
    ///
    /// # Errors
    ///
    /// Returns the runner error when tmux cannot capture the target pane.
    pub fn readiness_gate<F>(
        &mut self,
        target: &str,
        config: SubmitConfig,
        mut sleep: F,
    ) -> Result<ReadinessResult, TmuxError>
    where
        F: FnMut(std::time::Duration),
    {
        let mut waited_ms = 0;
        let poll_ms = config.readiness_poll_ms.max(1);
        loop {
            let content = self.capture(target, Some(5))?;
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
            sleep(std::time::Duration::from_millis(sleep_ms));
            waited_ms += sleep_ms;
        }
    }

    /// Probe the current pane capture once and return a send throttle when no prompt is visible.
    ///
    /// # Errors
    ///
    /// Returns the runner error when tmux cannot capture the target pane.
    pub fn busy_guard(&mut self, target: &str) -> Result<SendThrottle, TmuxError> {
        let content = self.capture(target, Some(5))?;
        if pane_capture_has_active_output(&content) {
            Ok(SendThrottle::Busy)
        } else {
            Ok(SendThrottle::Allowed)
        }
    }

    #[cfg(test)]
    fn send_text_with_sleeper<F>(
        &mut self,
        target: &str,
        text: &str,
        sleep: F,
    ) -> Result<SendTextReport, TmuxError>
    where
        F: FnMut(std::time::Duration),
    {
        self.send_text_body_with_sleeper(target, text, SubmitConfig::claude(), sleep)
    }

    fn send_text_body_with_sleeper<F>(
        &mut self,
        target: &str,
        text: &str,
        config: SubmitConfig,
        mut sleep: F,
    ) -> Result<SendTextReport, TmuxError>
    where
        F: FnMut(std::time::Duration),
    {
        self.exit_mode_if_needed(target)?;
        let used_buffer = text.contains('\n') || text.len() > 500;
        if used_buffer {
            self.load_buffer(text)?;
            self.paste_buffer(target)?;
        } else {
            self.send_keys_literal(target, text)?;
        }
        sleep(std::time::Duration::from_millis(SEND_SETTLE_MS));
        let (enter_attempts, warned_pending) =
            self.submit_with_confirm_config(target, &mut sleep, config)?;
        Ok(SendTextReport {
            used_buffer,
            enter_attempts,
            warned_pending,
        })
    }

    fn submit_with_confirm_config<F>(
        &mut self,
        target: &str,
        sleep: &mut F,
        config: SubmitConfig,
    ) -> Result<(u32, bool), TmuxError>
    where
        F: FnMut(std::time::Duration),
    {
        for attempt in 1..=MAX_SUBMIT_ATTEMPTS {
            self.send_enter(target)?;
            sleep(std::time::Duration::from_millis(config.confirm_interval_ms));
            if !self.pane_input_pending(target) {
                return Ok((attempt, false));
            }
        }
        Ok((MAX_SUBMIT_ATTEMPTS, true))
    }

    fn submit_config_for_target(&mut self, target: &str) -> SubmitConfig {
        self.display_pane_current_command(target)
            .map_or_else(|_| SubmitConfig::claude(), |command| {
                SubmitConfig::for_engine_name(&command)
            })
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

fn write_pane_title(
    runner: &mut dyn TmuxRunner,
    target: &str,
    title: Option<&str>,
) -> Result<(), TmuxError> {
    let Some(title) = title else {
        return Ok(());
    };
    runner.run(
        "select-pane",
        &[
            "-t".to_owned(),
            target.to_owned(),
            "-T".to_owned(),
            title.to_owned(),
        ],
    )?;
    Ok(())
}
