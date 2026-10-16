//! Workspace alternate chords must yield to Neovim's editing modes.
pub const PANE_ALTERNATES: &str =
    "MonitterDesktop && shortcutMode==vim && !NeovimInsert && !NeovimCmdline && !NeovimTerminal";

pub fn context(mode: &str, cmdline: bool) -> &'static str {
    if cmdline || mode.starts_with("cmdline") {
        "MonitterNativeNeovim NeovimCmdline"
    } else if mode.starts_with("insert") || mode.starts_with("replace") {
        "MonitterNativeNeovim NeovimInsert"
    } else if mode == "terminal" {
        "MonitterNativeNeovim NeovimTerminal"
    } else {
        "MonitterNativeNeovim NeovimNormal"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{KeyBindingContextPredicate, KeyContext};
    fn enabled(mode: &str, child: &str) -> bool {
        let root = KeyContext::parse(&format!("MonitterDesktop shortcutMode={mode}")).unwrap();
        let child = KeyContext::parse(child).unwrap();
        KeyBindingContextPredicate::parse(PANE_ALTERNATES)
            .unwrap()
            .depth_of(&[root, child])
            .is_some()
    }
    #[test]
    fn editor_editing_modes_retain_ctrl_w() {
        for mode in [
            "insert",
            "replace",
            "cmdline_normal",
            "cmdline_insert",
            "terminal",
        ] {
            assert!(!enabled("vim", context(mode, false)), "intercepted {mode}");
        }
        assert!(!enabled("vim", context("normal", true)));
    }
    #[test]
    fn alternates_navigate_all_normal_pane_types() {
        for child in [
            context("normal", false),
            context("visual", false),
            "MonitterNativeTerminal",
            "ChatPane",
            "EmptyPane",
        ] {
            assert!(enabled("vim", child), "missing alternate in {child}");
            assert!(!enabled("standard", child));
        }
    }
}
