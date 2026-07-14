const WIND_CLEANUP_USAGE: &str =
    "usage: maw cleanup [--zombie-agents] [--teams] [--fix|--yes]  (no scope flag runs both; --fix kills zombie panes, team prune is always safe)";

fn wind_cleanup_command(argv: &[String]) -> CliOutput {
    let (mut zombie, mut teams, mut apply) = (false, false, false);
    for arg in argv {
        match arg.as_str() {
            "--zombie-agents" | "--zombies" => zombie = true,
            "--teams" => teams = true,
            "--fix" | "--yes" | "-y" => apply = true,
            "--help" | "-h" => {
                return CliOutput {
                    code: 0,
                    stdout: format!("{WIND_CLEANUP_USAGE}\n"),
                    stderr: String::new(),
                };
            }
            other => {
                return CliOutput {
                    code: 2,
                    stdout: String::new(),
                    stderr: format!("cleanup: unexpected argument {other}\n{WIND_CLEANUP_USAGE}\n"),
                };
            }
        }
    }
    if !zombie && !teams {
        zombie = true;
        teams = true;
    }

    let mut output = CliOutput {
        code: 0,
        stdout: String::new(),
        stderr: String::new(),
    };
    if zombie {
        let mut view_args = vec!["--zombie-agents".to_owned()];
        if apply {
            view_args.push("--yes".to_owned());
        }
        wind_cleanup_append(&mut output, &view_run_command(&view_args));
    }
    if teams {
        wind_cleanup_append(&mut output, &team_run_command(&["prune".to_owned()]));
    }
    output
}

fn wind_cleanup_append(output: &mut CliOutput, result: &CliOutput) {
    output.stdout.push_str(&result.stdout);
    output.stderr.push_str(&result.stderr);
    if result.code != 0 {
        output.code = result.code;
    }
}

#[cfg(test)]
mod wind_cleanup_tests {
    use super::*;

    #[test]
    fn cleanup_hook_keeps_native_usage_and_rejects_unknown_flags() {
        let help = run_cli(&["cleanup".to_owned(), "--help".to_owned()]);
        assert_eq!(help.code, 0);
        assert_eq!(help.stdout, format!("{WIND_CLEANUP_USAGE}\n"));

        let invalid = run_cli(&["cleanup".to_owned(), "--bogus".to_owned()]);
        assert_eq!(invalid.code, 2);
        assert!(invalid.stderr.contains("unexpected argument --bogus"));
        assert!(!invalid.stderr.contains("port pending"));
    }
}
