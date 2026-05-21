use maw_split::{decide_split_policy, ClaudePanePolicy, SplitPolicyInput, SplitPolicyReason};

#[test]
fn claude_like_default_policy_is_background_tab() {
    let decision = decide_split_policy(&SplitPolicyInput {
        pane_current_command: Some("claude".to_owned()),
        ..SplitPolicyInput::default()
    })
    .expect("claude-like pane gets default policy");

    assert_eq!(decision.action, ClaudePanePolicy::BackgroundTab);
    assert_eq!(decision.reason, SplitPolicyReason::ClaudePolicy);
}

#[test]
fn version_like_commands_reject_empty_numeric_segments() {
    assert!(!maw_split::is_claude_like_pane(Some(".1.2")));
    assert!(!maw_split::is_claude_like_pane(Some("1..2")));
}
