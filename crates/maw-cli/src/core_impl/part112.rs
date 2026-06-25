const DISPATCH_112: &[DispatcherEntry] = &[DispatcherEntry { command: "view", handler: Handler::Sync(view_run_command) }];

fn view_run_command(argv: &[String]) -> CliOutput {
    match view_run_with_runner(argv, &mut maw_tmux::CommandTmuxRunner::new()) {
        Ok(output) | Err(output) => output,
    }
}

fn view_run_with_runner<R: maw_tmux::TmuxRunner>(
    argv: &[String],
    runner: &mut R,
) -> Result<CliOutput, CliOutput> {
    let mut attach_args = argv.to_vec();
    attach_args.push("--readonly".to_owned());
    attach_args.push("--print".to_owned());
    attach_run_with_runner(&attach_args, runner)
}

#[cfg(test)]
mod view_tests {
    use super::*;

    struct ViewFakeRunner;

    impl maw_tmux::TmuxRunner for ViewFakeRunner {
        fn run(&mut self, subcommand: &str, _args: &[String]) -> Result<String, maw_tmux::TmuxError> {
            if subcommand == "list-sessions" { Ok("50-mawjs\n".to_owned()) } else { Ok(String::new()) }
        }
    }

    fn view_strings(values: &[&str]) -> Vec<String> { values.iter().map(|value| (*value).to_owned()).collect() }

    #[test]
    fn view_dispatch_fragment_owns_view() {
        assert_eq!(DISPATCH_112[0].command, "view");
    }

    #[test]
    fn view_is_readonly_attach_plan() {
        let output = view_run_with_runner(&view_strings(&["mawjs"]), &mut ViewFakeRunner).unwrap();
        assert!(output.stdout.contains("tmux attach -r -t 50-mawjs"));
    }
}
