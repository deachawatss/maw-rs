use maw_cli::{dispatcher_status, run_cli, DispatchKind};

fn run(args: &[&str]) -> maw_cli::CliOutput {
    run_cli(&args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>())
}

#[test]
fn activity_is_registered_native_and_usage_is_offline() {
    assert_eq!(dispatcher_status("activity"), DispatchKind::Native);

    let missing_target = run(&["activity"]);
    assert_eq!(missing_target.code, 2);
    assert!(missing_target.stderr.contains("usage: maw activity"));

    let bad_target = run(&["activity", "--all", "s:main"]);
    assert_eq!(bad_target.code, 2);
    assert!(bad_target.stderr.contains("usage: maw activity"));
}
