use maw_feed::parse_line;

#[test]
fn invalid_timestamp_month_is_rejected() {
    assert!(parse_line("2026-13-01 00:00:00 | oracle | host | Stop | project | session").is_none());
}
