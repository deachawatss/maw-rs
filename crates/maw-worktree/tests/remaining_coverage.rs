use maw_worktree::{resolve_worktree_window, WorktreeWindowResolution};

#[test]
fn empty_worktree_name_stays_unmatched() {
    assert_eq!(
        resolve_worktree_window("mawjs-oracle", "", &[]),
        WorktreeWindowResolution::None
    );
}

use maw_worktree::{Session, Window};

#[test]
fn suffixed_parent_session_can_scope_worktree_resolution() {
    let sessions = vec![
        Session {
            name: "other".to_owned(),
            windows: vec![Window {
                index: 0,
                name: "agent".to_owned(),
                active: false,
            }],
        },
        Session {
            name: "47-mawjs".to_owned(),
            windows: vec![Window {
                index: 1,
                name: "codex".to_owned(),
                active: true,
            }],
        },
    ];

    assert_eq!(
        resolve_worktree_window("mawjs-oracle", "1-codex", &sessions),
        WorktreeWindowResolution::Bound {
            window: "codex".to_owned(),
        }
    );
}
