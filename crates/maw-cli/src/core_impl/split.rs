const DISPATCH_113: &[DispatcherEntry] = &[DispatcherEntry { command: "split", handler: Handler::Sync(split_run_command) }];

const SPLIT_USAGE: &str = "usage: maw split <target> [-v|--vertical] [--pct N] [--cmd <cmd>] [--dry-run]";

/// Narrowest pane worth creating, in columns.
///
/// Roughly where Claude Code's TUI stops wrapping pathologically; below it a
/// pane is technically alive and practically unusable. #180 reported four panes
/// at 10 columns each in an 89-column window, created silently.
const SPLIT_MIN_WIDTH: u16 = 40;

#[derive(Debug, Clone, PartialEq)]
struct SplitOptions { target: String, vertical: bool, pct: f64, command: Option<String>, dry_run: bool }

fn split_run_command(argv: &[String]) -> CliOutput {
    match split_run_with_runner(argv, &mut maw_tmux::CommandTmuxRunner::new()) {
        Ok(stdout) => CliOutput { code: 0, stdout, stderr: String::new() },
        Err((code, message)) => CliOutput { code, stdout: String::new(), stderr: format!("{message}\n") },
    }
}

fn split_run_with_runner<R: maw_tmux::TmuxRunner>(
    argv: &[String],
    runner: &mut R,
) -> Result<String, (i32, String)> {
    let opts = split_parse_args(argv)?;
    split_validate_tmux_target(&opts.target).map_err(|message| (1, message))?;
    if let Some(command) = opts.command.as_deref() { split_validate_command_text(command).map_err(|message| (1, message))?; }
    if opts.dry_run { return Ok(split_render_dry_run(&opts)); }
    // A vertical split shares height, not width, so the column guard does not
    // apply to it. Only refuse when the width is actually knowable: if tmux
    // cannot be asked, fail open rather than block a legitimate split on a
    // failed measurement.
    if !opts.vertical {
        if let Some(width) = split_target_width(runner, &opts.target) {
            split_guard_width(width, opts.pct).map_err(|message| (1, message))?;
        }
    }
    let tmux_args = split_tmux_args(&opts).map_err(|message| (1, message))?;
    runner.run("split-window", &tmux_args).map_err(|error| (1, error.message))?;
    Ok(format!("split → {}\n", opts.target))
}

fn split_parse_args(argv: &[String]) -> Result<SplitOptions, (i32, String)> {
    let mut target = None;
    let mut vertical = false;
    let mut pct = 50.0f64;
    let mut command = None;
    let mut dry_run = false;
    let mut index = 0usize;
    while index < argv.len() {
        match argv[index].as_str() {
            "--help" | "-h" => return Err((2, SPLIT_USAGE.to_owned())),
            "-v" | "--vertical" => vertical = true,
            "--dry-run" => dry_run = true,
            "--pct" => {
                let Some(value) = argv.get(index + 1) else { return Err((2, "split: missing --pct value".to_owned())); };
                pct = split_parse_pct(value)?;
                index += 1;
            }
            "--cmd" => {
                let Some(value) = argv.get(index + 1) else { return Err((2, "split: missing --cmd value".to_owned())); };
                command = Some(value.clone());
                index += 1;
            }
            arg if arg.starts_with("--pct=") => pct = split_parse_pct(&arg["--pct=".len()..])?,
            arg if arg.starts_with("--cmd=") => command = Some(arg["--cmd=".len()..].to_owned()),
            arg if arg.starts_with('-') => return Err((2, format!("split: unknown argument {arg}"))),
            value => {
                if target.is_some() { return Err((2, "split: target already provided".to_owned())); }
                target = Some(value.to_owned());
            }
        }
        index += 1;
    }
    Ok(SplitOptions { target: target.ok_or_else(|| (2, SPLIT_USAGE.to_owned()))?, vertical, pct, command, dry_run })
}

fn split_parse_pct(value: &str) -> Result<f64, (i32, String)> {
    value.parse::<f64>().map_err(|_| (2, format!("split: invalid --pct value {value}")))
}

fn split_tmux_args(opts: &SplitOptions) -> Result<Vec<String>, String> {
    let options = maw_tmux::TmuxSplitActionOptions { vertical: opts.vertical, pct: opts.pct, command: opts.command.clone() };
    maw_tmux::tmux_split_action_args(&opts.target, &options).map_err(|error| error.message)
}

fn split_render_dry_run(opts: &SplitOptions) -> String {
    let flag = if opts.vertical { "-v" } else { "-h" };
    let command = opts.command.as_ref().map(|cmd| format!(" -- {cmd}")).unwrap_or_default();
    format!("tmux split-window {flag} -l {}% -t {}{command}\n", opts.pct, opts.target)
}

fn split_validate_tmux_target(value: &str) -> Result<(), String> {
    if value.is_empty() || value.trim() != value || value.starts_with('-') || value.chars().any(char::is_control) {
        return Err("split target must be non-empty, unpadded, not start with '-', and contain no control characters".to_owned());
    }
    Ok(())
}

/// Current width of the split target, or `None` when tmux cannot be asked.
fn split_target_width<R: maw_tmux::TmuxRunner>(runner: &mut R, target: &str) -> Option<u16> {
    let args = ["-p".to_owned(), "-t".to_owned(), target.to_owned(), "#{pane_width}".to_owned()];
    runner.run("display-message", &args).ok()?.trim().parse().ok()
}

/// Refuse a horizontal split that would leave either side unusably narrow.
///
/// #180 chose *refuse* over *auto-rebalance*: rebalancing silently moves panes
/// the operator did not ask about, and in an 89-column window no arrangement of
/// four panes is usable — the honest answer is that the split does not fit.
fn split_guard_width(width: u16, pct: f64) -> Result<(), String> {
    // tmux spends one column on the divider; the rest is shared by the two sides.
    // Kept in f64 throughout: converting back to integers only to compare would
    // add a lossy cast for no gain, and the widths are only ever rendered.
    let usable = f64::from(width.saturating_sub(1));
    let new_pane = (usable * pct.clamp(0.0, 100.0) / 100.0).floor();
    let remaining = usable - new_pane;
    if new_pane.min(remaining) >= f64::from(SPLIT_MIN_WIDTH) {
        return Ok(());
    }
    Err(format!(
        "split refused: {width} columns split {pct:.0}/{:.0} leaves panes of {new_pane:.0} and {remaining:.0} columns, \
         under the {SPLIT_MIN_WIDTH}-column minimum.\n  \
         → widen the window, or put it elsewhere:  maw break <pane>   (own window)\n  \
         → stack instead of side-by-side:          maw split {{target}} --vertical\n  \
         → even out what is already there:         maw resize equal",
        100.0 - pct
    ))
}

fn split_validate_command_text(value: &str) -> Result<(), String> {
    if value.is_empty() || value.chars().any(char::is_control) {
        return Err("split command must be non-empty and contain no control characters".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod split_tests {
    use super::*;

    #[derive(Default)]
    struct SplitFakeRunner { calls: Vec<(String, Vec<String>)> }

    impl maw_tmux::TmuxRunner for SplitFakeRunner {
        fn run(&mut self, subcommand: &str, args: &[String]) -> Result<String, maw_tmux::TmuxError> {
            self.calls.push((subcommand.to_owned(), args.to_vec()));
            Ok(String::new())
        }
    }

    fn split_strings(values: &[&str]) -> Vec<String> { values.iter().map(|value| (*value).to_owned()).collect() }

    #[test]
    fn split_dispatch_fragment_owns_split() {
        assert_eq!(DISPATCH_113[0].command, "split");
    }

    #[test]
    fn split_uses_tmux_runner_and_argv_vec() {
        let mut runner = SplitFakeRunner::default();
        let out = split_run_with_runner(&split_strings(&["%1", "--vertical", "--pct", "25", "--cmd", "echo hi"]), &mut runner).unwrap();
        assert_eq!(out, "split → %1\n");
        assert_eq!(runner.calls[0].0, "split-window");
        assert_eq!(runner.calls[0].1, vec!["-v", "-l", "25%", "-t", "%1", "echo hi"]);
    }

    /// Runner that answers `display-message` with a fixed pane width.
    struct SplitWidthRunner { width: u16, calls: Vec<(String, Vec<String>)> }

    impl maw_tmux::TmuxRunner for SplitWidthRunner {
        fn run(&mut self, subcommand: &str, args: &[String]) -> Result<String, maw_tmux::TmuxError> {
            self.calls.push((subcommand.to_owned(), args.to_vec()));
            if subcommand == "display-message" {
                return Ok(format!("{}\n", self.width));
            }
            Ok(String::new())
        }
    }

    #[test]
    fn split_guard_refuses_the_reported_89_column_case() {
        // #180: an 89-column window split 50/50 gives 44 each — fine. The report
        // was four panes at ~10 columns, i.e. splitting what was already split.
        // Assert the actual widths, not merely that it errored.
        assert!(split_guard_width(89, 50.0).is_ok(), "89 split 50/50 is 44/44 and must be allowed");

        // Exact boundary: one column goes to the divider, so 81 is the narrowest
        // window that yields two 40-column panes. 80 misses by exactly one.
        assert!(split_guard_width(81, 50.0).is_ok(), "81 → 40/40, the narrowest that fits");
        let err = split_guard_width(80, 50.0).unwrap_err();
        assert!(err.contains("39 and 40 columns"), "80 → 39/40, one short: {err}");

        let err = split_guard_width(44, 50.0).unwrap_err();
        assert!(err.contains("21 and 22 columns"), "message must name both widths: {err}");
        assert!(err.contains("40-column minimum"), "message must name the minimum: {err}");
        assert!(err.contains("maw break"), "message must name an alternative: {err}");

        // A lopsided split is refused on the narrow side even when the wide side is fine.
        let err = split_guard_width(100, 90.0).unwrap_err();
        assert!(err.contains("89 and 10 columns"), "{err}");
    }

    #[test]
    fn split_refuses_before_touching_tmux_when_the_result_would_be_unusable() {
        let mut runner = SplitWidthRunner { width: 44, calls: Vec::new() };
        let err = split_run_with_runner(&split_strings(&["%1"]), &mut runner).unwrap_err();

        assert_eq!(err.0, 1);
        assert!(err.1.contains("split refused"), "{}", err.1);
        assert!(
            !runner.calls.iter().any(|(verb, _)| verb == "split-window"),
            "a refused split must not run split-window; calls: {:?}",
            runner.calls
        );
    }

    #[test]
    fn split_allows_a_wide_window_and_skips_the_guard_when_vertical() {
        let mut wide = SplitWidthRunner { width: 200, calls: Vec::new() };
        split_run_with_runner(&split_strings(&["%1"]), &mut wide).expect("wide split allowed");
        assert!(wide.calls.iter().any(|(verb, _)| verb == "split-window"));

        // Vertical shares height, not width — never measured, never refused.
        let mut narrow = SplitWidthRunner { width: 20, calls: Vec::new() };
        split_run_with_runner(&split_strings(&["%1", "--vertical"]), &mut narrow)
            .expect("vertical split is not width-constrained");
        assert!(
            !narrow.calls.iter().any(|(verb, _)| verb == "display-message"),
            "vertical must not measure width; calls: {:?}",
            narrow.calls
        );
    }

    #[test]
    fn split_falls_open_when_the_width_cannot_be_measured() {
        // The default fake answers every command with an empty string, so the
        // width is unknowable. A failed measurement must not block a split.
        let mut runner = SplitFakeRunner::default();
        split_run_with_runner(&split_strings(&["%1"]), &mut runner).expect("unmeasurable width allowed");
        assert!(runner.calls.iter().any(|(verb, _)| verb == "split-window"));
    }

    #[test]
    fn split_rejects_injection_targets_before_runner() {
        let mut runner = SplitFakeRunner::default();
        let err = split_run_with_runner(&split_strings(&["bad\nname"]), &mut runner).unwrap_err();
        assert!(err.1.contains("contain no control"));
        assert!(runner.calls.is_empty());
    }
}
