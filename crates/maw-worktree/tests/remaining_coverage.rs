use maw_worktree::{resolve_worktree_window, WorktreeWindowResolution};

#[test]
fn empty_worktree_name_stays_unmatched() {
    assert_eq!(
        resolve_worktree_window("mawjs-oracle", "", &[]),
        WorktreeWindowResolution::None
    );
}
