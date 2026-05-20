use maw_peer::{format_peer_list, validate_peer_alias, validate_peer_url, PeerListRow};

fn row(
    alias: &str,
    url: &str,
    node: Option<&str>,
    nickname: Option<&str>,
    last_seen: Option<&str>,
) -> PeerListRow {
    PeerListRow {
        alias: alias.to_owned(),
        url: url.to_owned(),
        node: node.map(str::to_owned),
        nickname: nickname.map(str::to_owned),
        last_seen: last_seen.map(str::to_owned),
        stale: false,
        stale_age_ms: None,
    }
}

#[test]
fn peer_impl_validation_matches_maw_js_alias_and_url_contract() {
    assert_eq!(validate_peer_alias("alice"), None);
    assert_eq!(validate_peer_alias("a_1-2"), None);
    assert_eq!(validate_peer_alias("0"), None);

    assert!(validate_peer_alias("Bad_Alias")
        .unwrap()
        .contains("invalid alias"));
    assert!(validate_peer_alias("OkAlias")
        .unwrap()
        .contains("^[a-z0-9][a-z0-9_-]{0,31}$"));
    assert!(validate_peer_alias("_bad").is_some());
    assert!(validate_peer_alias(&"a".repeat(33)).is_some());

    assert_eq!(validate_peer_url("http://127.0.0.1:1"), None);
    assert_eq!(validate_peer_url("https://example.org"), None);
    assert!(validate_peer_url("ftp://127.0.0.1:1")
        .unwrap()
        .contains("must be http:// or https://"));
    assert!(validate_peer_url("not a url")
        .unwrap()
        .contains("invalid URL"));
}

#[test]
fn format_peer_list_matches_maw_js_empty_and_table_contract() {
    assert_eq!(format_peer_list(&[]), "no peers");

    let output = format_peer_list(&[
        row("a", "http://a", None, None, None),
        row("b", "http://b", Some("n2"), Some("Bee"), Some("2026-01-01")),
    ]);

    assert!(output.starts_with("alias"), "{output}");
    assert!(output.contains("nickname"), "{output}");
    assert!(
        output
            .lines()
            .any(|line| line.starts_with('a') && line.contains("http://a") && line.contains('-')),
        "{output}"
    );
    assert!(
        output
            .lines()
            .any(|line| line.starts_with('b') && line.contains("http://b") && line.contains("n2")),
        "{output}"
    );
    assert!(output.contains("Bee"), "{output}");
    assert!(output.contains("2026-01-01"), "{output}");
}

#[test]
fn format_peer_list_renders_stale_suffix_like_maw_js() {
    let day_ms = 24 * 60 * 60 * 1000;
    let mut old = row(
        "old",
        "http://old",
        Some("old-node"),
        None,
        Some("2026-05-01T00:00:00.000Z"),
    );
    old.stale = true;
    old.stale_age_ms = Some(9 * day_ms);

    let mut never = row("never", "http://never", None, None, None);
    never.stale = true;
    never.stale_age_ms = None;

    let output = format_peer_list(&[old, never]);

    assert!(
        output.contains("\u{1b}[2m(stale, last seen 9d ago)\u{1b}[0m"),
        "{output}"
    );
    assert!(
        output.contains("\u{1b}[2m(stale, never seen)\u{1b}[0m"),
        "{output}"
    );
}
