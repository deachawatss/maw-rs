use maw_bring::{is_self_bring, parse_bring_to_target, translate_bring_to_flag, BringToTarget};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct FlagFixture {
    name: String,
    input: Vec<String>,
    expected: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TargetFixture {
    name: String,
    input: String,
    expected_session: String,
    expected_window: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SelfGuardFixture {
    name: String,
    target: String,
    caller_session_window: Option<String>,
    expected: bool,
}

#[test]
fn bring_to_flag_fixtures_match_maw_js_portable_spec() {
    let fixtures: Vec<FlagFixture> =
        serde_json::from_str(include_str!("fixtures/bring-to-flag.fixtures.json"))
            .expect("valid bring --to flag fixture json");

    for fixture in fixtures {
        assert_eq!(
            translate_bring_to_flag(&fixture.input),
            fixture.expected,
            "{}",
            fixture.name
        );
    }
}

#[test]
fn bring_to_target_fixtures_match_maw_js_portable_spec() {
    let fixtures: Vec<TargetFixture> =
        serde_json::from_str(include_str!("fixtures/bring-to-target.fixtures.json"))
            .expect("valid bring --to target fixture json");

    for fixture in fixtures {
        assert_eq!(
            parse_bring_to_target(&fixture.input),
            BringToTarget {
                session: fixture.expected_session,
                window: fixture.expected_window,
            },
            "{}",
            fixture.name
        );
    }
}

#[test]
fn bring_self_guard_fixtures_match_maw_js_portable_spec() {
    let fixtures: Vec<SelfGuardFixture> =
        serde_json::from_str(include_str!("fixtures/bring-self-guard.fixtures.json"))
            .expect("valid bring self guard fixture json");

    for fixture in fixtures {
        assert_eq!(
            is_self_bring(&fixture.target, fixture.caller_session_window.as_deref()),
            fixture.expected,
            "{}",
            fixture.name
        );
    }
}
