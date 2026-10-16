//! Native workspace shortcuts.  The global key bindings only name actions;
//! this module routes each action back through the retained Desktop entity so
//! tab, draft, and terminal lifecycle stays owned by the existing surface.
use gpui_kit::component::WindowExt as _;
use gpui_kit::*;

pub(super) fn workspace_shortcuts_blocked(
    dialog_open: bool,
    preferences_open: bool,
    palette_open: bool,
    closing: bool,
    command_bar_open: bool,
    draft_open: bool,
    composing_in_focused_composer: bool,
) -> bool {
    dialog_open
        || preferences_open
        || palette_open
        || closing
        || command_bar_open
        || draft_open
        || composing_in_focused_composer
}

pub(super) fn focused_input_is_composing(focused: bool, has_marked_text: bool) -> bool {
    focused && has_marked_text
}

impl crate::Desktop {
    /// Workspace shortcuts are suspended only while a retained chat composer
    /// currently owns focus and has marked text for an active IME composition.
    pub(super) fn focused_composer_is_composing(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        for pane in self.chat_panes.borrow().values() {
            let composer = pane.read(cx).composer.clone();
            let focused = composer.read(cx).focus_handle(cx).is_focused(window);
            let has_marked_text = focused
                && composer.update(cx, |input, cx| {
                    EntityInputHandler::marked_text_range(input, window, cx).is_some()
                });
            if focused_input_is_composing(focused, has_marked_text) {
                return true;
            }
        }
        false
    }
}

#[derive(gpui_kit::Action, Clone, PartialEq, Default, Debug)]
#[action(namespace = monitter_desktop)]
pub struct OpenTaskDraft;
#[derive(gpui_kit::Action, Clone, PartialEq, Default, Debug)]
#[action(namespace = monitter_desktop)]
pub struct OpenFocusedTerminal;
#[derive(gpui_kit::Action, Clone, PartialEq, Default, Debug)]
#[action(namespace = monitter_desktop)]
pub struct ToggleFocusedDetail;
#[derive(gpui_kit::Action, Clone, PartialEq, Default, Debug)]
#[action(namespace = monitter_desktop)]
pub struct ToggleWorkspaceSidebar;
#[derive(gpui_kit::Action, Clone, PartialEq, Default, Debug)]
#[action(namespace = monitter_desktop)]
pub struct ZoomFocusedPane;
#[derive(gpui_kit::Action, Clone, PartialEq, Default, Debug)]
#[action(namespace = monitter_desktop)]
pub struct RestorePaneLayout;
#[derive(gpui_kit::Action, Clone, PartialEq, Default, Debug)]
#[action(namespace = monitter_desktop)]
pub struct OpenPreferences;
#[derive(gpui_kit::Action, Clone, PartialEq, Default, Debug)]
#[action(namespace = monitter_desktop)]
pub struct OpenSwitcher;
#[derive(gpui_kit::Action, Clone, PartialEq, Default, Debug)]
#[action(namespace = monitter_desktop)]
pub struct OpenCommands;
#[derive(gpui_kit::Action, Clone, PartialEq, Default, Debug)]
#[action(namespace = monitter_desktop)]
pub struct TerminalOpenSwitcher;
#[derive(gpui_kit::Action, Clone, PartialEq, Default, Debug)]
#[action(namespace = monitter_desktop)]
pub struct TerminalOpenCommands;
#[derive(gpui_kit::Action, Clone, PartialEq, Eq, Debug)]
#[action(namespace = monitter_desktop, no_json)]
pub struct FocusAdjacentPane {
    pub direction: String,
}
#[derive(gpui_kit::Action, Clone, PartialEq, Eq, Debug)]
#[action(namespace = monitter_desktop, no_json)]
pub struct MoveFocusedTab {
    pub direction: String,
}
#[derive(gpui_kit::Action, Clone, PartialEq, Eq, Debug)]
#[action(namespace = monitter_desktop, no_json)]
pub struct SwitchNamedWorkspace {
    pub direction: i8,
}

#[derive(gpui_kit::Action, Clone, PartialEq, Eq, Debug)]
#[action(namespace = monitter_desktop, no_json)]
pub struct SelectNamedWorkspace {
    pub index: usize,
}

/// Installed after gpui-kit so these global actions win over input selection.
pub(super) fn pane_and_workspace_bindings() -> Vec<KeyBinding> {
    let mut bindings = Vec::new();
    for direction in ["left", "right", "up", "down"] {
        bindings.push(KeyBinding::new(
            &format!("cmd-ctrl-{direction}"),
            MoveFocusedTab { direction: direction.into() },
            None,
        ));
    }
    for (key, direction) in [
        ("cmd-shift-left", -1), ("cmd-shift-up", -1), ("cmd-[", -1),
        ("cmd-shift-right", 1), ("cmd-shift-down", 1), ("cmd-]", 1),
    ] {
        bindings.push(KeyBinding::new(key, SwitchNamedWorkspace { direction }, None));
    }
    for index in 1..=9 {
        bindings.push(KeyBinding::new(
            &format!("cmd-{index}"), SelectNamedWorkspace { index }, None,
        ));
    }
    bindings.extend([
        KeyBinding::new("cmd-ctrl-f", crate::native_menu::Fullscreen, None),
        KeyBinding::new("cmd-enter", crate::native_menu::Fullscreen, None),
    ]);
    bindings
}

#[derive(gpui_kit::Action, Clone, PartialEq, Eq, Debug)]
#[action(namespace = monitter_desktop, no_json)]
pub struct FocusRelativeTab {
    pub direction: i8,
}
#[derive(gpui_kit::Action, Clone, PartialEq, Default, Debug)]
#[action(namespace = monitter_desktop)]
pub struct EqualizeWorkspace;
#[derive(gpui_kit::Action, Clone, PartialEq, Default, Debug)]
#[action(namespace = monitter_desktop)]
pub struct ScaleUp;
#[derive(gpui_kit::Action, Clone, PartialEq, Default, Debug)]
#[action(namespace = monitter_desktop)]
pub struct ScaleDown;
#[derive(gpui_kit::Action, Clone, PartialEq, Default, Debug)]
#[action(namespace = monitter_desktop)]
pub struct ScaleReset;
#[derive(gpui_kit::Action, Clone, PartialEq, Eq, Debug)]
#[action(namespace = monitter_desktop, no_json)]
pub struct VimFocusDirection {
    pub direction: String,
}
#[derive(gpui_kit::Action, Clone, PartialEq, Eq, Debug)]
#[action(namespace = monitter_desktop, no_json)]
pub struct VimWorkspaceShortcut {
    pub operation: String,
}

#[derive(Clone)]
pub struct DesktopShortcuts {
    weak: WeakEntity<crate::Desktop>,
}

impl DesktopShortcuts {
    pub fn new(weak: WeakEntity<crate::Desktop>) -> Self {
        Self { weak }
    }

    fn dispatch(
        &self,
        window_handle: &AnyWindowHandle,
        cx: &mut App,
        action: impl FnOnce(&mut crate::Desktop, &mut Window, &mut Context<crate::Desktop>) + 'static,
    ) {
        let weak = self.weak.clone();
        let handle = window_handle.clone();
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_root, window, cx| {
                let _ = weak.update(cx, |desktop, cx| {
                    // Global actions can dispatch without an interactive root.
                    // Wait for authoritative workspace reconciliation first.
                    if desktop.startup_pending { return; }
                    // A sheet/preferences dialog owns its keyboard. In particular,
                    // don't let workspace shortcuts mutate the hidden dock below it.
                    if workspace_shortcuts_blocked(
                        window.has_active_dialog(cx),
                        desktop.preferences_open || crate::session_lock::locked(cx),
                        desktop
                            .palette
                            .as_ref()
                            .is_some_and(|palette| palette.read(cx).is_open()),
                        desktop.closing,
                        crate::vim_command::bar(cx).is_some_and(|bar| bar.read(cx).is_open()),
                        desktop.draft_creation_open,
                        desktop.focused_composer_is_composing(window, cx),
                    ) {
                        return;
                    }
                    action(desktop, window, cx);
                    cx.notify();
                });
            });
        });
    }
}

pub fn install(window_handle: AnyWindowHandle, shortcuts: DesktopShortcuts, cx: &mut App) {
    cx.on_action({
        let shortcuts = shortcuts.clone();
        move |_: &OpenTaskDraft, cx| {
            shortcuts.dispatch(&window_handle, cx, |desktop, window, cx| {
                desktop.open_chat_draft(None, window, cx);
            });
        }
    });
    cx.on_action({
        let shortcuts = shortcuts.clone();
        move |_: &OpenFocusedTerminal, cx| {
            shortcuts.dispatch(&window_handle, cx, |desktop, window, cx| {
                // Shift+Cmd+T is specifically the terminal command, even when
                // focus currently belongs to a browser tab. Do not route it
                // through Cmd+T's browser-aware new-tab policy.
                let result = desktop
                    .dock
                    .update(cx, |dock, cx| dock.add_focused_empty_tab(window, cx))
                    .and_then(|id| {
                        desktop.empty_pane_action(
                            &id,
                            crate::empty_pane::EmptyPaneAction::Terminal,
                            window,
                            cx,
                        )
                    });
                if let Err(error) = result {
                    desktop.error = Some(error);
                }
            });
        }
    });
    cx.on_action({
        let shortcuts = shortcuts.clone();
        move |_: &ToggleFocusedDetail, cx| {
            shortcuts.dispatch(&window_handle, cx, |desktop, window, cx| {
                desktop.switch_surface(crate::Sidebar::Subagents, window, cx);
            });
        }
    });
    cx.on_action({
        let shortcuts = shortcuts.clone();
        move |_: &ToggleWorkspaceSidebar, cx| {
            shortcuts.dispatch(&window_handle, cx, |desktop, window, cx| {
                desktop.toggle_sidebar(window, cx);
            });
        }
    });
    cx.on_action({
        let shortcuts = shortcuts.clone();
        move |_: &ZoomFocusedPane, cx| {
            shortcuts.dispatch(&window_handle, cx, |desktop, window, cx| {
                let group = desktop.dock.read(cx).focused_group_node(window, cx);
                if let Some(group) = group {
                    let area = desktop.dock.read(cx).area();
                    area.update(cx, |area, cx| {
                        area.set_zoomed_in(group, window, cx);
                    });
                } else {
                    desktop.error = Some("Focus a workspace pane before maximizing it.".into());
                }
            });
        }
    });
    cx.on_action({
        let shortcuts = shortcuts.clone();
        move |_: &RestorePaneLayout, cx| {
            shortcuts.dispatch(&window_handle, cx, |desktop, window, cx| {
                let area = desktop.dock.read(cx).area();
                area.update(cx, |area, cx| area.set_zoomed_out(window, cx));
            });
        }
    });
    cx.on_action({
        let shortcuts = shortcuts.clone();
        move |_: &OpenPreferences, cx| {
            shortcuts.dispatch(&window_handle, cx, |desktop, window, cx| {
                desktop.show_preferences(window, cx);
            });
        }
    });
    cx.on_action({
        let shortcuts = shortcuts.clone();
        move |_: &OpenSwitcher, cx| {
            shortcuts.dispatch(&window_handle, cx, |desktop, window, _cx| {
                desktop.open_palette(crate::command_palette::PaletteMode::Switch, window, _cx);
            });
        }
    });
    cx.on_action({
        let shortcuts = shortcuts.clone();
        move |_: &OpenCommands, cx| {
            shortcuts.dispatch(&window_handle, cx, |desktop, window, _cx| {
                desktop.open_palette(crate::command_palette::PaletteMode::Commands, window, _cx);
            });
        }
    });
    cx.on_action({
        let shortcuts = shortcuts.clone();
        move |_: &TerminalOpenSwitcher, cx| {
            shortcuts.dispatch(&window_handle, cx, |desktop, window, cx| {
                if desktop
                    .dock
                    .read(cx)
                    .focused_pane_id(window, cx)
                    .is_some_and(|id| id.starts_with("terminal:"))
                {
                    desktop.open_palette(crate::command_palette::PaletteMode::Switch, window, cx);
                }
            });
        }
    });
    cx.on_action({
        let shortcuts = shortcuts.clone();
        move |_: &TerminalOpenCommands, cx| {
            shortcuts.dispatch(&window_handle, cx, |desktop, window, cx| {
                if desktop
                    .dock
                    .read(cx)
                    .focused_pane_id(window, cx)
                    .is_some_and(|id| id.starts_with("terminal:"))
                {
                    desktop.open_palette(crate::command_palette::PaletteMode::Commands, window, cx);
                }
            });
        }
    });
    cx.on_action({
        let shortcuts = shortcuts.clone();
        move |action: &FocusAdjacentPane, cx| {
            let direction = action.direction.clone();
            shortcuts.dispatch(&window_handle, cx, move |desktop, window, cx| {
                desktop.dock.update(cx, |dock, cx| {
                    dock.focus_adjacent_pane(&direction, window, cx)
                });
            });
        }
    });
    cx.on_action({
        let shortcuts = shortcuts.clone();
        move |action: &MoveFocusedTab, cx| {
            let direction = action.direction.clone();
            shortcuts.dispatch(&window_handle, cx, move |desktop, window, cx| {
                desktop.dock.update(cx, |dock, cx| {
                    dock.move_focused_tab(&direction, window, cx);
                });
            });
        }
    });
    cx.on_action({
        let shortcuts = shortcuts.clone();
        move |action: &SelectNamedWorkspace, cx| {
            let index = action.index;
            shortcuts.dispatch(&window_handle, cx, move |desktop, window, cx| {
                desktop.select_named_workspace(index, window, cx);
            });
        }
    });
    cx.on_action({
        let shortcuts = shortcuts.clone();
        move |action: &SwitchNamedWorkspace, cx| {
            let direction = action.direction;
            shortcuts.dispatch(&window_handle, cx, move |desktop, window, cx| {
                desktop.cycle_named_workspace(direction, window, cx);
            });
        }
    });
    cx.on_action({
        let shortcuts = shortcuts.clone();
        move |action: &FocusRelativeTab, cx| {
            let direction = action.direction;
            shortcuts.dispatch(&window_handle, cx, move |desktop, window, cx| {
                desktop.dock.update(cx, |dock, cx| {
                    dock.focus_relative_tab(direction, window, cx);
                });
            });
        }
    });
    cx.on_action({
        let shortcuts = shortcuts.clone();
        move |_: &EqualizeWorkspace, cx| {
            shortcuts.dispatch(&window_handle, cx, |desktop, window, cx| {
                desktop.dock.update(cx, |dock, cx| {
                    dock.equalize_workspace(window, cx);
                });
            });
        }
    });
    cx.on_action({
        let shortcuts = shortcuts.clone();
        move |action: &VimFocusDirection, cx| {
            let direction = action.direction.clone();
            shortcuts.dispatch(&window_handle, cx, move |desktop, window, cx| {
                desktop.dock.update(cx, |dock, cx| {
                    dock.focus_adjacent_pane(&direction, window, cx)
                });
            });
        }
    });
    {
        let shortcuts = shortcuts.clone();
        let scale_shortcuts = shortcuts.clone();
        cx.on_action(move |action: &VimWorkspaceShortcut, cx| {
            let operation = action.operation.clone();
            shortcuts.dispatch(&window_handle, cx, move |desktop, window, cx| {
                let result = match operation.as_str() {
                    "first" | "last" | "next" | "previous" => {
                        desktop.dock.update(cx, |dock, cx| {
                            dock.focus_relative_pane(&operation, window, cx)
                        })
                    }
                    "equalize" => desktop
                        .dock
                        .update(cx, |dock, cx| dock.equalize_workspace(window, cx)),
                    "resize-max-horizontal" => desktop.dock.update(cx, |dock, cx| {
                        dock.resize_focused_group("horizontal", None, true, window, cx)
                    }),
                    "resize-max-vertical" => desktop.dock.update(cx, |dock, cx| {
                        dock.resize_focused_group("vertical", None, true, window, cx)
                    }),
                    "resize-horizontal-plus" => desktop.dock.update(cx, |dock, cx| {
                        dock.resize_focused_group("horizontal", Some(10), false, window, cx)
                    }),
                    "resize-horizontal-minus" => desktop.dock.update(cx, |dock, cx| {
                        dock.resize_focused_group("horizontal", Some(-10), false, window, cx)
                    }),
                    "resize-vertical-plus" => desktop.dock.update(cx, |dock, cx| {
                        dock.resize_focused_group("vertical", Some(10), false, window, cx)
                    }),
                    "resize-vertical-minus" => desktop.dock.update(cx, |dock, cx| {
                        dock.resize_focused_group("vertical", Some(-10), false, window, cx)
                    }),
                    "left" | "right" | "up" | "down" => desktop.dock.update(cx, |dock, cx| {
                        dock.move_focused_group(&operation, window, cx)
                    }),
                    "split-down" | "split-right" | "split-up" | "split-left" => {
                        if desktop.neovim_split_focused(matches!(operation.as_str(), "split-right" | "split-left"), window, cx) { true } else { desktop.dock.update(cx, |dock, cx| {
                            dock.split_focused_group(
                                operation.trim_start_matches("split-"),
                                None,
                                window,
                                cx,
                            )
                            .unwrap_or(false)
                        }) }
                    }
                    "close" => match desktop.close_focused_pane(window, cx) { Ok(()) => true, Err(error) => { desktop.error = Some(error); true } },
                    "only" => desktop
                        .dock
                        .update(cx, |dock, cx| dock.keep_focused_group(window, cx)),
                    "rotate" => desktop
                        .dock
                        .update(cx, |dock, cx| dock.rearrange_groups(None, 1, window, cx)),
                    "rotate-reverse" => desktop
                        .dock
                        .update(cx, |dock, cx| dock.rearrange_groups(None, -1, window, cx)),
                    "exchange" => desktop
                        .dock
                        .update(cx, |dock, cx| dock.rearrange_groups(None, 0, window, cx)),
                    _ => false,
                };
                if !result {
                    desktop.error = Some(format!(
                        "Vim workspace action '{operation}' is not available here."
                    ));
                }
            });
        });
        cx.on_action(move |_: &ScaleUp, cx| {
            scale_shortcuts.dispatch(&window_handle, cx, |desktop, _window, cx| {
                update_scale(desktop, 5, cx);
            });
        });
    }
    {
        let shortcuts = shortcuts.clone();
        cx.on_action(move |_: &ScaleDown, cx| {
            shortcuts.dispatch(&window_handle, cx, |desktop, _window, cx| {
                update_scale(desktop, -5, cx);
            });
        });
    }
    {
        let shortcuts = shortcuts.clone();
        cx.on_action(move |_: &ScaleReset, cx| {
            shortcuts.dispatch(&window_handle, cx, |desktop, _window, cx| {
                update_scale(desktop, 0, cx);
            });
        });
    }

    cx.bind_keys(pane_and_workspace_bindings());
    cx.bind_keys([
        KeyBinding::new("cmd-shift-c", OpenTaskDraft, None),
        KeyBinding::new("cmd-shift-t", OpenFocusedTerminal, None),
        KeyBinding::new("cmd-shift-b", ToggleFocusedDetail, None),
        KeyBinding::new("cmd-b", ToggleWorkspaceSidebar, None),
        KeyBinding::new("cmd-up", ZoomFocusedPane, None),
        KeyBinding::new("cmd-down", RestorePaneLayout, None),
        KeyBinding::new("cmd-,", OpenPreferences, None),
        KeyBinding::new("cmd-k", OpenSwitcher, None),
        KeyBinding::new("cmd-p", OpenCommands, None),
        KeyBinding::new("ctrl-shift-k", TerminalOpenSwitcher, None),
        KeyBinding::new("ctrl-shift-p", TerminalOpenCommands, None),
        KeyBinding::new("cmd-alt-left", FocusRelativeTab { direction: -1 }, None),
        KeyBinding::new("cmd-alt-right", FocusRelativeTab { direction: 1 }, None),
        KeyBinding::new(
            "cmd-alt-up",
            FocusAdjacentPane {
                direction: "up".into(),
            },
            None,
        ),
        KeyBinding::new(
            "cmd-alt-down",
            FocusAdjacentPane {
                direction: "down".into(),
            },
            None,
        ),
        KeyBinding::new("cmd-alt-=", EqualizeWorkspace, None),
        KeyBinding::new(
            "ctrl-w t",
            VimWorkspaceShortcut {
                operation: "first".into(),
            },
            Some(monitter_gpui_desktop::neovim_keys::PANE_ALTERNATES),
        ),
        KeyBinding::new(
            "ctrl-w b",
            VimWorkspaceShortcut {
                operation: "last".into(),
            },
            Some(monitter_gpui_desktop::neovim_keys::PANE_ALTERNATES),
        ),
        KeyBinding::new(
            "ctrl-w w",
            VimWorkspaceShortcut {
                operation: "next".into(),
            },
            Some(monitter_gpui_desktop::neovim_keys::PANE_ALTERNATES),
        ),
        KeyBinding::new(
            "ctrl-w W",
            VimWorkspaceShortcut {
                operation: "previous".into(),
            },
            Some(monitter_gpui_desktop::neovim_keys::PANE_ALTERNATES),
        ),
        KeyBinding::new(
            "ctrl-w H",
            VimWorkspaceShortcut {
                operation: "left".into(),
            },
            Some(monitter_gpui_desktop::neovim_keys::PANE_ALTERNATES),
        ),
        KeyBinding::new(
            "ctrl-w J",
            VimWorkspaceShortcut {
                operation: "down".into(),
            },
            Some(monitter_gpui_desktop::neovim_keys::PANE_ALTERNATES),
        ),
        KeyBinding::new(
            "ctrl-w K",
            VimWorkspaceShortcut {
                operation: "up".into(),
            },
            Some(monitter_gpui_desktop::neovim_keys::PANE_ALTERNATES),
        ),
        KeyBinding::new(
            "ctrl-w L",
            VimWorkspaceShortcut {
                operation: "right".into(),
            },
            Some(monitter_gpui_desktop::neovim_keys::PANE_ALTERNATES),
        ),
        KeyBinding::new(
            "ctrl-w s",
            VimWorkspaceShortcut {
                operation: "split-down".into(),
            },
            Some(monitter_gpui_desktop::neovim_keys::PANE_ALTERNATES),
        ),
        KeyBinding::new(
            "ctrl-w v",
            VimWorkspaceShortcut {
                operation: "split-right".into(),
            },
            Some(monitter_gpui_desktop::neovim_keys::PANE_ALTERNATES),
        ),
        KeyBinding::new(
            "ctrl-w c",
            VimWorkspaceShortcut {
                operation: "close".into(),
            },
            Some(monitter_gpui_desktop::neovim_keys::PANE_ALTERNATES),
        ),
        KeyBinding::new(
            "ctrl-w o",
            VimWorkspaceShortcut {
                operation: "only".into(),
            },
            Some(monitter_gpui_desktop::neovim_keys::PANE_ALTERNATES),
        ),
        KeyBinding::new(
            "ctrl-w r",
            VimWorkspaceShortcut {
                operation: "rotate".into(),
            },
            Some(monitter_gpui_desktop::neovim_keys::PANE_ALTERNATES),
        ),
        KeyBinding::new(
            "ctrl-w R",
            VimWorkspaceShortcut {
                operation: "rotate-reverse".into(),
            },
            Some(monitter_gpui_desktop::neovim_keys::PANE_ALTERNATES),
        ),
        KeyBinding::new(
            "ctrl-w x",
            VimWorkspaceShortcut {
                operation: "exchange".into(),
            },
            Some(monitter_gpui_desktop::neovim_keys::PANE_ALTERNATES),
        ),
        KeyBinding::new(
            "ctrl-w =",
            VimWorkspaceShortcut {
                operation: "equalize".into(),
            },
            Some(monitter_gpui_desktop::neovim_keys::PANE_ALTERNATES),
        ),
        KeyBinding::new(
            "ctrl-w _",
            VimWorkspaceShortcut {
                operation: "resize-max-vertical".into(),
            },
            Some(monitter_gpui_desktop::neovim_keys::PANE_ALTERNATES),
        ),
        KeyBinding::new(
            "ctrl-w |",
            VimWorkspaceShortcut {
                operation: "resize-max-horizontal".into(),
            },
            Some(monitter_gpui_desktop::neovim_keys::PANE_ALTERNATES),
        ),
        KeyBinding::new(
            "ctrl-w +",
            VimWorkspaceShortcut {
                operation: "resize-horizontal-plus".into(),
            },
            Some(monitter_gpui_desktop::neovim_keys::PANE_ALTERNATES),
        ),
        KeyBinding::new(
            "ctrl-w -",
            VimWorkspaceShortcut {
                operation: "resize-horizontal-minus".into(),
            },
            Some(monitter_gpui_desktop::neovim_keys::PANE_ALTERNATES),
        ),
        KeyBinding::new(
            "ctrl-w >",
            VimWorkspaceShortcut {
                operation: "resize-vertical-plus".into(),
            },
            Some(monitter_gpui_desktop::neovim_keys::PANE_ALTERNATES),
        ),
        KeyBinding::new(
            "ctrl-w <",
            VimWorkspaceShortcut {
                operation: "resize-vertical-minus".into(),
            },
            Some(monitter_gpui_desktop::neovim_keys::PANE_ALTERNATES),
        ),
        KeyBinding::new("cmd-0", ScaleReset, None),
        KeyBinding::new("cmd-=", ScaleUp, None),
        KeyBinding::new("cmd-shift-=", ScaleUp, None),
        KeyBinding::new("cmd--", ScaleDown, None),
        KeyBinding::new(
            "ctrl-w",
            NoAction {},
            Some(monitter_gpui_desktop::neovim_keys::PANE_ALTERNATES),
        ),
        KeyBinding::new(
            "ctrl-w h",
            VimFocusDirection {
                direction: "left".into(),
            },
            Some(monitter_gpui_desktop::neovim_keys::PANE_ALTERNATES),
        ),
        KeyBinding::new(
            "ctrl-w j",
            VimFocusDirection {
                direction: "down".into(),
            },
            Some(monitter_gpui_desktop::neovim_keys::PANE_ALTERNATES),
        ),
        KeyBinding::new(
            "ctrl-w k",
            VimFocusDirection {
                direction: "up".into(),
            },
            Some(monitter_gpui_desktop::neovim_keys::PANE_ALTERNATES),
        ),
        KeyBinding::new(
            "ctrl-w l",
            VimFocusDirection {
                direction: "right".into(),
            },
            Some(monitter_gpui_desktop::neovim_keys::PANE_ALTERNATES),
        ),
        KeyBinding::new(
            "ctrl-w left",
            VimFocusDirection {
                direction: "left".into(),
            },
            Some(monitter_gpui_desktop::neovim_keys::PANE_ALTERNATES),
        ),
        KeyBinding::new(
            "ctrl-w down",
            VimFocusDirection {
                direction: "down".into(),
            },
            Some(monitter_gpui_desktop::neovim_keys::PANE_ALTERNATES),
        ),
        KeyBinding::new(
            "ctrl-w up",
            VimFocusDirection {
                direction: "up".into(),
            },
            Some(monitter_gpui_desktop::neovim_keys::PANE_ALTERNATES),
        ),
        KeyBinding::new(
            "ctrl-w right",
            VimFocusDirection {
                direction: "right".into(),
            },
            Some(monitter_gpui_desktop::neovim_keys::PANE_ALTERNATES),
        ),
    ]);
}

impl crate::Desktop {
    pub(super) fn run_vim_command(
        &mut self,
        command: crate::vim_command::VimCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::vim_command::VimCommand as V;
        let result: Result<bool, String> = match command {
            V::NewChat => {
                self.open_chat_draft(None, window, cx);
                Ok(true)
            }
            V::SelectTab(target) => Ok(self.dock.update(cx, |dock, cx| {
                dock.select_focused_tab_target(&target, window, cx)
            })),
            V::CloseTab(target) => {
                let selected = target.map_or(true, |target| {
                    self.dock.update(cx, |dock, cx| {
                        dock.select_focused_tab_target(&target, window, cx)
                    })
                });
                if selected {
                    self.close_focused_pane(window, cx).map(|_| true)
                } else {
                    Ok(false)
                }
            }
            V::OnlyTabs => Ok(self
                .dock
                .update(cx, |dock, cx| dock.keep_focused_tab(window, cx))),
            V::MoveTab(target) => Ok(self.dock.update(cx, |dock, cx| {
                dock.move_focused_tab_order(&target, window, cx)
            })),
            V::Split { direction, size } => self.dock.update(cx, |dock, cx| {
                dock.split_focused_group(&direction, size, window, cx)
            }),
            V::Terminal => self.dock.update(cx, |dock, cx| dock.add_focused_empty_tab(window, cx))
                .and_then(|id| self.empty_pane_action(&id, crate::empty_pane::EmptyPaneAction::Terminal, window, cx).map(|_| true)),
            V::ClosePane(target) => Ok(self
                .dock
                .update(cx, |dock, cx| dock.close_focused_group(target, window, cx))),
            V::OnlyPane => Ok(self
                .dock
                .update(cx, |dock, cx| dock.keep_focused_group(window, cx))),
            V::FocusPane(direction) => Ok(self.dock.update(cx, |dock, cx| {
                if matches!(direction.as_str(), "first" | "last" | "next" | "previous") {
                    dock.focus_relative_pane(&direction, window, cx)
                } else {
                    dock.focus_adjacent_pane(&direction, window, cx)
                }
            })),
            V::Rotate(direction) => Ok(self.dock.update(cx, |dock, cx| {
                dock.rearrange_groups(None, direction, window, cx)
            })),
            V::Exchange(target) => Ok(self
                .dock
                .update(cx, |dock, cx| dock.rearrange_groups(target, 0, window, cx))),
            V::MovePane(direction) => Ok(self.dock.update(cx, |dock, cx| {
                dock.move_focused_group(&direction, window, cx)
            })),
            V::Equalize => Ok(self
                .dock
                .update(cx, |dock, cx| dock.equalize_workspace(window, cx))),
            V::Resize {
                axis,
                amount,
                maximize,
            } => Ok(self.dock.update(cx, |dock, cx| {
                dock.resize_focused_group(&axis, amount, maximize, window, cx)
            })),
            V::Help => Ok(true),
        };
        match result {
            Err(error) => self.error = Some(error),
            Ok(false) => {
                self.error = Some(
                    "That Vim workspace command is unavailable in the current pane layout.".into(),
                )
            }
            Ok(true) => {}
        }
        cx.notify();
    }
}

fn update_scale(desktop: &mut crate::Desktop, delta: i16, cx: &mut Context<crate::Desktop>) {
    let current = crate::design::interface_scale();
    let target = if delta == 0 {
        125
    } else {
        (current as i32 + delta as i32).clamp(80, 200) as u16
    };
    if target == current {
        return;
    }
    let Some(preferences) = desktop.preferences.as_mut() else {
        desktop.error = Some("Workspace preferences are still loading.".into());
        cx.notify();
        return;
    };
    preferences.data.appearance.interface_scale = target;
    crate::design::set_interface_scale(target);
    desktop.queue_preferences_save(cx);
}

#[cfg(test)]
#[path = "shortcut_tests.rs"]
mod tests;
