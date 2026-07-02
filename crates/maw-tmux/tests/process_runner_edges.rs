use maw_tmux::{CommandTmuxRunner, TmuxRunner};

#[test]
fn command_runner_default_socket_display_and_stdin_failure_edges() {
    let default_runner = CommandTmuxRunner::new();
    assert_eq!(
        default_runner
            .argv("display-message", &["hello".to_owned()])
            .into_iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>(),
        vec!["tmux", "display-message", "hello"]
    );

    let socket_runner = CommandTmuxRunner::with_program("sh").with_socket("sock");
    assert_eq!(
        socket_runner
            .argv("-c", &["printf ok".to_owned()])
            .into_iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>(),
        vec!["sh", "-S", "sock", "-c", "printf ok"]
    );

    let display = maw_tmux::TmuxError::new("adapter failed").to_string();
    assert_eq!(display, "adapter failed");

    let mut missing = CommandTmuxRunner::with_program("/definitely/missing/tmux-for-maw-rs-test");
    let err = missing
        .run("display-message", &[])
        .expect_err("missing tmux program is reported");
    assert!(err.message.contains("failed to execute"), "{}", err.message);

    let mut runner = CommandTmuxRunner::with_program("sh");
    let stderr = runner
        .run("-c", &["printf out; printf err >&2; exit 9".to_owned()])
        .expect_err("stderr is preferred over stdout for failures");
    assert_eq!(stderr.message, "tmux exited with status 9: err");
}

#[test]
fn command_runner_refuses_leading_dash_tmux_target_before_spawn() {
    use maw_tmux::{CommandTmuxRunner, TmuxRunner};

    let mut runner = CommandTmuxRunner::with_program("/bin/sh");
    let payload =
        std::env::temp_dir().join(format!("maw-tmux-option-injection-{}", std::process::id()));
    let injected_target = format!("-oProxyCommand=touch+{}", payload.display());
    let result = runner.run(
        "display-message",
        &["-t".to_owned(), injected_target, "#{pane_id}".to_owned()],
    );

    let error = result.expect_err("leading-dash target must be rejected");
    assert!(
        error.message.contains("target/session"),
        "unexpected error: {error}"
    );
    assert!(
        !payload.exists(),
        "target option-injection payload must not run before rejection"
    );
}

#[test]
fn command_runner_refuses_leading_dash_tmux_session_before_spawn() {
    use maw_tmux::{CommandTmuxRunner, TmuxRunner};

    let mut runner = CommandTmuxRunner::with_program("/bin/sh");
    let result = runner.run(
        "new-session",
        &["-d".to_owned(), "-s".to_owned(), "-X".to_owned()],
    );

    let error = result.expect_err("leading-dash session must be rejected");
    assert!(
        error.message.contains("target/session"),
        "unexpected error: {error}"
    );
}
