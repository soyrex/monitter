use super::workspace_shortcuts_blocked;

#[test]
fn foreground_overlays_and_shutdown_block_workspace_shortcuts() {
    assert!(workspace_shortcuts_blocked(
        true, false, false, false, false, false, false
    ));
    assert!(workspace_shortcuts_blocked(
        false, true, false, false, false, false, false
    ));
    assert!(workspace_shortcuts_blocked(
        false, false, true, false, false, false, false
    ));
    assert!(workspace_shortcuts_blocked(
        false, false, false, true, false, false, false
    ));
    assert!(workspace_shortcuts_blocked(
        false, false, false, false, true, false, false
    ));
    assert!(workspace_shortcuts_blocked(
        false, false, false, false, false, true, false
    ));
    assert!(workspace_shortcuts_blocked(
        false, false, false, false, false, false, true
    ));
    assert!(!workspace_shortcuts_blocked(
        false, false, false, false, false, false, false
    ));
}
