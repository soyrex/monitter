//! Native application menu; text commands dispatch to the focused GPUI input.
use gpui_kit::{component::input, *};

gpui_kit::actions!(
    monitter_native_menu,
    [
        OpenSettings,
        LockSession,
        HideApp,
        HideOthers,
        ShowAll,
        Minimize,
        Zoom,
        Fullscreen
    ]
);

pub fn install(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-,", OpenSettings, None),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-shift-l", LockSession, None),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-shift-l", LockSession, None),
        KeyBinding::new("cmd-h", HideApp, None),
        KeyBinding::new("alt-cmd-h", HideOthers, None),
        KeyBinding::new("cmd-m", Minimize, None),
        KeyBinding::new("ctrl-cmd-f", Fullscreen, None),
        KeyBinding::new("cmd-enter", Fullscreen, None),
    ]);
    cx.on_action(|_: &HideApp, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.on_action(|_: &Minimize, cx| {
        if let Some(window) = cx.active_window() {
            let _ = window.update(cx, |_, window, _| window.minimize_window());
        }
    });
    cx.on_action(|_: &Zoom, cx| {
        if let Some(window) = cx.active_window() {
            let _ = window.update(cx, |_, window, _| window.zoom_window());
        }
    });
    cx.on_action(|_: &Fullscreen, cx| {
        if let Some(window) = cx.active_window() {
            // Keyboard actions run while their window is on GPUI's update
            // stack. Re-entering it here fails; defer until dispatch finishes.
            cx.defer(move |cx| {
                if let Err(error) = window.update(cx, |_, window, _| window.toggle_fullscreen()) {
                    eprintln!("Cannot toggle fullscreen: {error}");
                }
            });
        }
    });
    cx.set_menus([
        Menu::new("Monitter").items([
            MenuItem::action("Settings…", OpenSettings),
            MenuItem::action("Lock Monitter", LockSession),
            MenuItem::separator(),
            MenuItem::os_submenu("Services", SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action("Hide Monitter", HideApp),
            MenuItem::action("Hide Others", HideOthers),
            MenuItem::action("Show All", ShowAll),
            MenuItem::separator(),
            MenuItem::action("Quit Monitter", super::Quit),
        ]),
        Menu::new("Edit").items([
            MenuItem::action("Undo", input::Undo),
            MenuItem::action("Redo", input::Redo),
            MenuItem::separator(),
            MenuItem::action("Cut", input::Cut),
            MenuItem::action("Copy", input::Copy),
            MenuItem::action("Paste", input::Paste),
            MenuItem::action("Select All", input::SelectAll),
        ]),
        Menu::new("View").items([
            MenuItem::action("New Vertical Split", crate::native_navigation::OpenVerticalSplit),
            MenuItem::action("New Horizontal Split", crate::native_navigation::OpenHorizontalSplit),
            MenuItem::separator(),
            MenuItem::action("Toggle Full Screen", Fullscreen),
        ]),
        Menu::new("Window").items([
            MenuItem::action("Minimize", Minimize),
            MenuItem::action("Zoom", Zoom),
        ]),
    ]);
}
