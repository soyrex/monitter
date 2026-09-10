use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem as Native, Submenu},
    AppHandle, Wry,
};

pub const STANDARD_SHORTCUT_MODE: &str = "standard";
pub const VIM_SHORTCUT_MODE: &str = "vim";

/// The Vim pane-prefix chord owns Cmd/Ctrl-W, so only standard mode assigns it
/// as a native menu accelerator. The item remains clickable in both modes.
pub fn close_tab_accelerator(shortcut_mode: &str) -> Option<&'static str> {
    (shortcut_mode == STANDARD_SHORTCUT_MODE).then_some("CmdOrCtrl+W")
}

pub fn build_for_shortcut_mode(
    app: &AppHandle<Wry>,
    shortcut_mode: &str,
) -> tauri::Result<Menu<Wry>> {
    let close_tab = MenuItem::with_id(
        app,
        "close-tab",
        "Close Tab",
        true,
        close_tab_accelerator(shortcut_mode),
    )?;
    let file = Submenu::with_items(app, "File", true, &[&close_tab])?;
    let edit = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &Native::undo(app, None)?,
            &Native::redo(app, None)?,
            &Native::separator(app)?,
            &Native::cut(app, None)?,
            &Native::copy(app, None)?,
            &Native::paste(app, None)?,
            &Native::select_all(app, None)?,
        ],
    )?;
    let window = Submenu::with_items(
        app,
        "Window",
        true,
        &[&Native::minimize(app, None)?, &Native::maximize(app, None)?],
    )?;
    #[cfg(target_os = "macos")]
    {
        let application = Submenu::with_items(
            app,
            "Monitter",
            true,
            &[
                &Native::about(app, Some("About Monitter"), None)?,
                &Native::separator(app)?,
                &Native::services(app, None)?,
                &Native::separator(app)?,
                &Native::hide(app, None)?,
                &Native::hide_others(app, None)?,
                &Native::show_all(app, None)?,
                &Native::separator(app)?,
                &Native::quit(app, None)?,
            ],
        )?;
        let view = Submenu::with_items(app, "View", true, &[&Native::fullscreen(app, None)?])?;
        Menu::with_items(app, &[&application, &file, &edit, &view, &window])
    }
    #[cfg(not(target_os = "macos"))]
    {
        file.append(&Native::separator(app)?)?;
        file.append(&Native::quit(app, None)?)?;
        Menu::with_items(app, &[&file, &edit, &window])
    }
}

/// Settings load after Tauri has constructed its initial menu. Start with the
/// persisted-field default so the first native menu is safe for older data.
pub fn build(app: &AppHandle<Wry>) -> tauri::Result<Menu<Wry>> {
    build_for_shortcut_mode(app, STANDARD_SHORTCUT_MODE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_mode_assigns_close_tab_accelerator() {
        assert_eq!(
            close_tab_accelerator(STANDARD_SHORTCUT_MODE),
            Some("CmdOrCtrl+W")
        );
    }

    #[test]
    fn vim_mode_reserves_close_tab_prefix() {
        assert_eq!(close_tab_accelerator(VIM_SHORTCUT_MODE), None);
    }
}
