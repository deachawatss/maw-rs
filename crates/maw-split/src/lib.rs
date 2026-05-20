//! Pure split policy helpers ported from maw-js.
//!
//! This crate mirrors maw-js `decideSplitPolicy` and `isClaudeLikePane`
//! without tmux/runtime IO.

/// Policy for Claude-like source panes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaudePanePolicy {
    Split,
    BackgroundTab,
    LinkWindow,
    Refuse,
}

impl ClaudePanePolicy {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Split => "split",
            Self::BackgroundTab => "background-tab",
            Self::LinkWindow => "link-window",
            Self::Refuse => "refuse",
        }
    }
}

/// Reason explaining a split policy decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitPolicyReason {
    NotAttaching,
    ForceSplit,
    NotClaude,
    ClaudePolicy,
}

impl SplitPolicyReason {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotAttaching => "not-attaching",
            Self::ForceSplit => "force-split",
            Self::NotClaude => "not-claude",
            Self::ClaudePolicy => "claude-policy",
        }
    }
}

/// Input to [`decide_split_policy`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SplitPolicyInput {
    pub pane_current_command: Option<String>,
    pub no_attach: bool,
    pub requested_policy: Option<String>,
    pub force_split: bool,
}

/// Decision returned by [`decide_split_policy`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SplitPolicyDecision {
    pub action: ClaudePanePolicy,
    pub reason: SplitPolicyReason,
}

/// Decide how a split request should behave from a potentially Claude-like pane.
///
/// # Errors
///
/// Returns an error when `requested_policy` is not one of the maw-js accepted
/// policy strings.
pub fn decide_split_policy(input: &SplitPolicyInput) -> Result<SplitPolicyDecision, String> {
    let requested_policy = validate_claude_pane_policy(input.requested_policy.as_deref())?;
    if input.no_attach {
        return Ok(SplitPolicyDecision {
            action: ClaudePanePolicy::Split,
            reason: SplitPolicyReason::NotAttaching,
        });
    }
    if input.force_split {
        return Ok(SplitPolicyDecision {
            action: ClaudePanePolicy::Split,
            reason: SplitPolicyReason::ForceSplit,
        });
    }
    if !is_claude_like_pane(input.pane_current_command.as_deref()) {
        return Ok(SplitPolicyDecision {
            action: ClaudePanePolicy::Split,
            reason: SplitPolicyReason::NotClaude,
        });
    }
    Ok(SplitPolicyDecision {
        action: requested_policy.unwrap_or(ClaudePanePolicy::BackgroundTab),
        reason: SplitPolicyReason::ClaudePolicy,
    })
}

/// Detect Claude Code or version-shaped Claude wrapper pane commands.
#[must_use]
pub fn is_claude_like_pane(pane_current_command: Option<&str>) -> bool {
    let Some(command) = pane_current_command else {
        return false;
    };
    let command = command.to_lowercase();
    if command.contains("claude") {
        return true;
    }
    is_three_part_numeric_version(command.trim())
}

fn validate_claude_pane_policy(value: Option<&str>) -> Result<Option<ClaudePanePolicy>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_empty() {
        return Ok(None);
    }
    match value {
        "split" => Ok(Some(ClaudePanePolicy::Split)),
        "background-tab" => Ok(Some(ClaudePanePolicy::BackgroundTab)),
        "link-window" => Ok(Some(ClaudePanePolicy::LinkWindow)),
        "refuse" => Ok(Some(ClaudePanePolicy::Refuse)),
        _ => Err(
            "--claude-pane-policy must be one of: split, background-tab, link-window, refuse"
                .to_owned(),
        ),
    }
}

fn is_three_part_numeric_version(value: &str) -> bool {
    let mut parts = value.split('.');
    let Some(first) = parts.next() else {
        return false;
    };
    let Some(second) = parts.next() else {
        return false;
    };
    let Some(third) = parts.next() else {
        return false;
    };
    if parts.next().is_some() {
        return false;
    }
    [first, second, third]
        .iter()
        .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_detection_requires_three_numeric_parts() {
        assert!(is_claude_like_pane(Some("2.1.139")));
        assert!(!is_claude_like_pane(Some("2.1")));
        assert!(!is_claude_like_pane(Some("2.1.x")));
    }
}
