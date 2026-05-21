//! Portable worktree-to-window matching policy.
//!
//! This crate mirrors maw-js `src/core/fleet/worktree-window-match.ts` and
//! uses the same JSON fixture contract from `test/spec/worktree-window-match.fixtures.json`.

use maw_matcher::{resolve_worktree_target, Named, ResolveResult};

/// Window metadata used by the worktree matcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    pub index: u32,
    pub name: String,
    pub active: bool,
}

impl Named for Window {
    fn name(&self) -> &str {
        &self.name
    }
}

/// Session metadata used by the worktree matcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub name: String,
    pub windows: Vec<Window>,
}

/// Worktree window resolution result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorktreeWindowResolution {
    Bound {
        window: String,
    },
    Ambiguous {
        query: String,
        candidates: Vec<String>,
    },
    None,
}

/// Resolve the tmux window name associated with a linked worktree.
#[must_use]
pub fn resolve_worktree_window(
    main_repo_name: &str,
    wt_name: &str,
    sessions: &[Session],
) -> WorktreeWindowResolution {
    let task_part = strip_numeric_prefix(wt_name);
    let parent_sessions = parent_sessions_for(main_repo_name, sessions);

    if !parent_sessions.is_empty() {
        let scoped_windows = dedupe_windows_by_name(&parent_sessions);
        if let Some(window) = bound_window(wt_name, &scoped_windows) {
            return WorktreeWindowResolution::Bound { window };
        }
        if let Some(window) = bound_window(task_part, &scoped_windows) {
            return WorktreeWindowResolution::Bound { window };
        }
    }

    let all_windows = dedupe_windows_by_name(sessions);
    if let Some(window) = bound_window(wt_name, &all_windows) {
        return WorktreeWindowResolution::Bound { window };
    }

    match resolve_worktree_target(task_part, &all_windows) {
        ResolveResult::Exact { matched } | ResolveResult::Fuzzy { matched } => {
            WorktreeWindowResolution::Bound {
                window: matched.name,
            }
        }
        ResolveResult::Ambiguous { candidates } => WorktreeWindowResolution::Ambiguous {
            query: task_part.to_owned(),
            candidates: candidates
                .into_iter()
                .map(|candidate| candidate.name)
                .collect(),
        },
        ResolveResult::None { .. } => WorktreeWindowResolution::None,
    }
}

fn dedupe_windows_by_name(sessions: &[Session]) -> Vec<Window> {
    let mut windows: Vec<Window> = Vec::new();
    for window in sessions.iter().flat_map(|session| &session.windows) {
        if let Some(existing) = windows
            .iter_mut()
            .find(|existing| existing.name == window.name)
        {
            *existing = window.clone();
        } else {
            windows.push(window.clone());
        }
    }
    windows
}

fn parent_oracle_name_from_main_repo(main_repo_name: &str) -> &str {
    main_repo_name
        .strip_suffix("-oracle")
        .unwrap_or(main_repo_name)
}

fn parent_sessions_for(main_repo_name: &str, sessions: &[Session]) -> Vec<Session> {
    let parent = parent_oracle_name_from_main_repo(main_repo_name);
    sessions
        .iter()
        .filter(|session| session.name == parent || session.name.ends_with(&format!("-{parent}")))
        .cloned()
        .collect()
}

fn bound_window(target: &str, windows: &[Window]) -> Option<String> {
    match resolve_worktree_target(target, windows) {
        ResolveResult::Exact { matched } | ResolveResult::Fuzzy { matched } => Some(matched.name),
        ResolveResult::None { .. } | ResolveResult::Ambiguous { .. } => None,
    }
}

fn strip_numeric_prefix(name: &str) -> &str {
    let Some((prefix, rest)) = name.split_once('-') else {
        return name;
    };
    if !prefix.is_empty() && prefix.bytes().all(|byte| byte.is_ascii_digit()) {
        rest
    } else {
        name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_non_numeric_prefixes_intact() {
        assert_eq!(strip_numeric_prefix("abc-feature"), "abc-feature");
        assert_eq!(strip_numeric_prefix("1-feature"), "feature");
    }

    #[test]
    fn later_duplicate_window_name_replaces_earlier_entry() {
        let windows = dedupe_windows_by_name(&[Session {
            name: "mawjs".to_owned(),
            windows: vec![
                Window {
                    index: 0,
                    name: "agent".to_owned(),
                    active: false,
                },
                Window {
                    index: 3,
                    name: "agent".to_owned(),
                    active: true,
                },
            ],
        }]);

        assert_eq!(
            windows,
            vec![Window {
                index: 3,
                name: "agent".to_owned(),
                active: true,
            }]
        );
    }
}
