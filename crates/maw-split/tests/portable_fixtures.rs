use maw_split::{decide_split_policy, ClaudePanePolicy, SplitPolicyInput, SplitPolicyReason};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Fixture {
    name: String,
    input: FixtureInput,
    expected: Option<ExpectedDecision>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FixtureInput {
    pane_current_command: Option<String>,
    no_attach: Option<bool>,
    requested_policy: Option<String>,
    force_split: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct ExpectedDecision {
    action: String,
    reason: String,
}

fn action_from_str(value: &str) -> ClaudePanePolicy {
    match value {
        "split" => ClaudePanePolicy::Split,
        "background-tab" => ClaudePanePolicy::BackgroundTab,
        "link-window" => ClaudePanePolicy::LinkWindow,
        "refuse" => ClaudePanePolicy::Refuse,
        other => panic!("unknown action {other}"),
    }
}

fn reason_from_str(value: &str) -> SplitPolicyReason {
    match value {
        "not-attaching" => SplitPolicyReason::NotAttaching,
        "force-split" => SplitPolicyReason::ForceSplit,
        "not-claude" => SplitPolicyReason::NotClaude,
        "claude-policy" => SplitPolicyReason::ClaudePolicy,
        other => panic!("unknown reason {other}"),
    }
}

#[test]
fn split_policy_fixtures_match_maw_js_portable_spec() {
    let fixtures: Vec<Fixture> =
        serde_json::from_str(include_str!("fixtures/split-policy.fixtures.json"))
            .expect("valid split policy fixture json");

    for fixture in fixtures {
        let actual = decide_split_policy(&SplitPolicyInput {
            pane_current_command: fixture.input.pane_current_command,
            no_attach: fixture.input.no_attach.unwrap_or(false),
            requested_policy: fixture.input.requested_policy,
            force_split: fixture.input.force_split.unwrap_or(false),
        });

        if let Some(error) = fixture.error {
            let err = actual.expect_err("fixture expects split policy error");
            assert!(err.contains(&error), "{}: {err:?}", fixture.name);
            continue;
        }

        let expected = fixture.expected.expect("fixture has expected decision");
        let actual = actual.expect("fixture expects split policy decision");
        assert_eq!(
            actual.action,
            action_from_str(&expected.action),
            "{}",
            fixture.name
        );
        assert_eq!(
            actual.reason,
            reason_from_str(&expected.reason),
            "{}",
            fixture.name
        );
    }
}

#[test]
fn split_policy_string_and_empty_edges_are_covered() {
    assert_eq!(ClaudePanePolicy::Split.as_str(), "split");
    assert_eq!(ClaudePanePolicy::BackgroundTab.as_str(), "background-tab");
    assert_eq!(ClaudePanePolicy::LinkWindow.as_str(), "link-window");
    assert_eq!(ClaudePanePolicy::Refuse.as_str(), "refuse");
    assert_eq!(SplitPolicyReason::NotAttaching.as_str(), "not-attaching");
    assert_eq!(SplitPolicyReason::ForceSplit.as_str(), "force-split");
    assert_eq!(SplitPolicyReason::NotClaude.as_str(), "not-claude");
    assert_eq!(SplitPolicyReason::ClaudePolicy.as_str(), "claude-policy");

    assert_eq!(
        decide_split_policy(&SplitPolicyInput {
            pane_current_command: None,
            requested_policy: Some(String::new()),
            ..SplitPolicyInput::default()
        })
        .unwrap()
        .reason,
        SplitPolicyReason::NotClaude
    );

    assert!(!maw_split::is_claude_like_pane(Some("1.2.3.4")));
}
