//! Platform-aware keybindings. Monitter's shortcuts are written for macOS with
//! `cmd`. On other platforms `cmd` is the Super key, which desktop environments
//! reserve, so plain `cmd-…` shortcuts use `ctrl` there instead.
//!
//! `cmd` combined with `ctrl` stays on Super+Ctrl: that is what Settings documents
//! for pane navigation, and swapping it would collide with the `cmd-alt-…` shortcuts.

use gpui_kit::{Action, KeyBinding};

/// Drop-in replacement for `KeyBinding::new` that translates the keystroke for the platform.
pub fn new<A: Action>(keystrokes: &str, action: A, context: Option<&str>) -> KeyBinding {
    KeyBinding::new(&translate(keystrokes), action, context)
}

/// macOS keeps `cmd`. Tests keep it on every platform so they exercise the same bindings.
fn translate(keystrokes: &str) -> String {
    if cfg!(any(target_os = "macos", test)) {
        keystrokes.to_owned()
    } else {
        keystrokes.split(' ').map(to_ctrl).collect::<Vec<_>>().join(" ")
    }
}

/// Translate one keystroke such as `cmd-shift-t` to `ctrl-shift-t`. Only the modifier
/// prefix is inspected, so keys like `[`, `,` and `-` pass through untouched.
fn to_ctrl(keystroke: &str) -> String {
    const MODIFIERS: [&str; 6] = ["secondary", "ctrl", "alt", "shift", "cmd", "fn"];
    let mut rest = keystroke;
    let mut modifiers: Vec<&str> = Vec::new();
    while let Some((head, tail)) = rest.split_once('-') {
        if !MODIFIERS.contains(&head) || tail.is_empty() {
            break;
        }
        modifiers.push(head);
        rest = tail;
    }
    if modifiers.contains(&"cmd") && !modifiers.contains(&"ctrl") {
        for modifier in modifiers.iter_mut().filter(|modifier| **modifier == "cmd") {
            *modifier = "ctrl";
        }
    }
    modifiers.push(rest);
    modifiers.join("-")
}

#[cfg(test)]
mod tests {
    use super::to_ctrl;

    #[test]
    fn plain_cmd_becomes_ctrl() {
        assert_eq!(to_ctrl("cmd-t"), "ctrl-t");
        assert_eq!(to_ctrl("cmd-shift-c"), "ctrl-shift-c");
        assert_eq!(to_ctrl("cmd-alt-left"), "ctrl-alt-left");
        assert_eq!(to_ctrl("alt-cmd-h"), "alt-ctrl-h");
        assert_eq!(to_ctrl("cmd-enter"), "ctrl-enter");
    }

    #[test]
    fn punctuation_keys_pass_through() {
        assert_eq!(to_ctrl("cmd-["), "ctrl-[");
        assert_eq!(to_ctrl("cmd-]"), "ctrl-]");
        assert_eq!(to_ctrl("cmd-,"), "ctrl-,");
        assert_eq!(to_ctrl("cmd--"), "ctrl--");
        assert_eq!(to_ctrl("cmd-1"), "ctrl-1");
    }

    #[test]
    fn cmd_with_ctrl_stays_on_super() {
        assert_eq!(to_ctrl("cmd-ctrl-left"), "cmd-ctrl-left");
        assert_eq!(to_ctrl("ctrl-cmd-f"), "ctrl-cmd-f");
    }

    #[test]
    fn bindings_without_cmd_are_unchanged() {
        assert_eq!(to_ctrl("ctrl-shift-k"), "ctrl-shift-k");
        assert_eq!(to_ctrl("alt-left"), "alt-left");
        assert_eq!(to_ctrl("ctrl-tab"), "ctrl-tab");
        assert_eq!(to_ctrl("escape"), "escape");
    }
}
