use maw_bring::{
    decide_split_bring, is_self_bring, parse_bring_args, same_session_target,
    translate_bring_to_flag, BringAliasOptions, BringToTarget, SplitBringDecision,
    SplitBringPolicy,
};

#[test]
fn parse_bring_args_records_to_target_with_window() {
    let parsed = parse_bring_args(&[
        "homekeeper".to_owned(),
        "--to".to_owned(),
        "workspace:agent".to_owned(),
    ])
    .expect("oracle arg parses");

    assert_eq!(parsed.oracle, "homekeeper");
    assert_eq!(parsed.opts.session.as_deref(), Some("workspace"));
    assert_eq!(parsed.opts.split_target.as_deref(), Some("workspace:agent"));
}

#[test]
fn parse_bring_args_ignores_unknown_flags_and_trailing_engine() {
    let parsed = parse_bring_args(&[
        "--unknown".to_owned(),
        "homekeeper".to_owned(),
        "--engine".to_owned(),
    ])
    .expect("unknown flags and missing engine values are tolerated like maw-js");

    assert_eq!(parsed.oracle, "homekeeper");
    assert_eq!(parsed.opts.engine, None);
}

#[test]
fn bring_remaining_edges_match_maw_js_tolerant_alias_contract() {
    let parsed = parse_bring_args(&[
        "--tab".to_owned(),
        "--split".to_owned(),
        "-e".to_owned(),
        "codex".to_owned(),
        "--to".to_owned(),
        "workspace:".to_owned(),
        "homekeeper".to_owned(),
    ])
    .expect("flags parse");

    assert_eq!(parsed.oracle, "homekeeper");
    assert_eq!(
        parsed.opts,
        BringAliasOptions {
            split: true,
            engine: Some("codex".to_owned()),
            pick: false,
            session: Some("workspace".to_owned()),
            split_target: None,
        }
    );
    assert_eq!(
        translate_bring_to_flag(&[
            "--to".to_owned(),
            "workspace".to_owned(),
            "--pick".to_owned()
        ]),
        vec!["--session", "workspace", "--pick"]
    );
    assert_eq!(
        maw_bring::parse_bring_to_target("workspace:"),
        BringToTarget {
            session: "workspace".to_owned(),
            window: None,
        }
    );
}

#[test]
fn split_guard_remaining_same_session_edges_are_stable() {
    assert!(!is_self_bring("workspace:other.3", None));
    assert!(!is_self_bring("", Some("workspace:agent")));
    assert!(!is_self_bring("workspace:other.3", Some("workspace:agent")));
    assert!(same_session_target(
        "workspace:other",
        Some("workspace:agent")
    ));
    assert!(!same_session_target(
        "remote:agent",
        Some("workspace:agent")
    ));
    assert_eq!(
        decide_split_bring(&SplitBringPolicy {
            split: true,
            target: "workspace:other",
            caller_session_window: Some("workspace:agent"),
            split_target: Some("remote:anchor"),
            attached_to_tmux: false,
            allow_self_bring: false,
        }),
        SplitBringDecision::RefuseSameSession
    );
    assert_eq!(
        decide_split_bring(&SplitBringPolicy {
            split: true,
            target: "workspace",
            caller_session_window: Some("workspace:agent"),
            split_target: None,
            attached_to_tmux: true,
            allow_self_bring: true,
        }),
        SplitBringDecision::Split
    );
}
