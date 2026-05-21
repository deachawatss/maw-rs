use maw_bring::parse_bring_args;

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
