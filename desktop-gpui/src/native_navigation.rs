//! Global desktop navigation actions, routed to the dock pane that owns focus.
use crate::docking::DockWorkspace;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::dock::{DockPlacement, PaneRef, PanelId};
use gpui_kit::*;

fn tab_at_index<T: Copy>(panels: &[T], one_based_index: usize) -> Option<T> {
    one_based_index
        .checked_sub(1)
        .and_then(|index| panels.get(index))
        .copied()
}

fn active_tab<T: Copy>(panels: &[T], active_ix: usize) -> Option<T> {
    panels.get(active_ix).copied()
}

#[derive(gpui_kit::Action, Clone, PartialEq, Default, Debug)]
#[action(namespace = monitter_desktop)]
pub struct OpenNewTab;

#[derive(gpui_kit::Action, Clone, PartialEq, Default, Debug)]
#[action(namespace = monitter_desktop)]
pub struct OpenVerticalSplit;

#[derive(gpui_kit::Action, Clone, PartialEq, Default, Debug)]
#[action(namespace = monitter_desktop)]
pub struct OpenHorizontalSplit;

#[derive(gpui_kit::Action, Clone, PartialEq, Default, Debug)]
#[action(namespace = monitter_desktop)]
pub struct CloseActiveTab;

#[derive(gpui_kit::Action, Clone, PartialEq, Eq, Debug)]
#[action(namespace = monitter_desktop, no_json)]
pub struct SelectTab {
    pub index: usize,
}

#[derive(Clone)]
pub struct NativeNavigation {
    weak: WeakEntity<crate::Desktop>,
}

impl NativeNavigation {
    pub fn new(weak: WeakEntity<crate::Desktop>) -> Self {
        Self { weak }
    }

    fn dispatch(
        &self,
        window_handle: &AnyWindowHandle,
        cx: &mut App,
        apply: impl FnOnce(
            &mut crate::Desktop,
            &mut Window,
            &mut Context<crate::Desktop>,
        ) -> Result<(), String>
        + 'static,
    ) {
        let weak = self.weak.clone();
        let handle = self
            .weak
            .read_with(cx, |desktop, cx| desktop.active_workspace_window(cx))
            .ok()
            .flatten()
            .unwrap_or_else(|| window_handle.clone());
        // Defer until global action dispatch releases its App borrow.
        cx.defer(move |cx| {
            match handle.update(cx, |_root, window, cx| {
                weak.update(cx, |desktop, cx| {
                    desktop.with_workspace_window(window, cx, |desktop, window, cx| {
                        // Native App actions bypass the inert startup paint tree.
                        // Cached presentation cannot authorize workspace mutations.
                        if desktop.startup_pending {
                            return;
                        }
                        if crate::shortcut_desktop::workspace_shortcuts_blocked(
                            window.has_active_dialog(cx),
                            desktop.preferences_open || crate::session_lock::locked(cx),
                            desktop
                                .palette
                                .as_ref()
                                .is_some_and(|palette| palette.read(cx).is_open()),
                            desktop.closing,
                            crate::vim_command::bar_for(window, cx)
                                .is_some_and(|bar| bar.read(cx).is_open()),
                            desktop.draft_creation_open,
                            desktop.focused_composer_is_composing(window, cx),
                        ) {
                            return;
                        }
                        match apply(desktop, window, cx) {
                            Ok(()) => {}
                            Err(error) => {
                                desktop.error = Some(error);
                                cx.notify();
                            }
                        }
                    });
                })
            }) {
                Ok(Ok(())) => {}
                Ok(Err(error)) => eprintln!("Native navigation target is unavailable: {error}"),
                Err(error) => eprintln!("Native navigation window is unavailable: {error}"),
            }
        });
    }
}

pub fn install(window_handle: AnyWindowHandle, navigation: NativeNavigation, cx: &mut App) {
    cx.on_action({
        let navigation = navigation.clone();
        move |_: &OpenVerticalSplit, cx| {
            navigation.dispatch(&window_handle, cx, |desktop, window, cx| {
                if desktop.neovim_split_focused(true, window, cx) {
                    Ok(())
                } else {
                    desktop.open_focused_empty_split(true, window, cx)
                }
            });
        }
    });
    cx.on_action({
        let navigation = navigation.clone();
        move |_: &OpenHorizontalSplit, cx| {
            navigation.dispatch(&window_handle, cx, |desktop, window, cx| {
                if desktop.neovim_split_focused(false, window, cx) {
                    Ok(())
                } else {
                    desktop.open_focused_empty_split(false, window, cx)
                }
            });
        }
    });
    cx.on_action({
        let navigation = navigation.clone();
        move |_: &OpenNewTab, cx| {
            navigation.dispatch(&window_handle, cx, |desktop, window, cx| {
                desktop.open_focused_empty_tab(window, cx)
            });
        }
    });
    cx.on_action({
        let navigation = navigation.clone();
        move |_: &CloseActiveTab, cx| {
            navigation.dispatch(&window_handle, cx, |desktop, window, cx| {
                // On non-macOS the primary modifier maps to Control; preserve
                // Ctrl+W inside a terminal, where it belongs to the shell.
                if !cfg!(target_os = "macos")
                    && desktop
                        .dock
                        .read(cx)
                        .focused_pane_id(window, cx)
                        .is_some_and(|id| id.starts_with("terminal:"))
                {
                    return Ok(());
                }
                desktop.close_focused_pane(window, cx)
            });
        }
    });
    cx.on_action({
        let navigation = navigation.clone();
        move |action: &SelectTab, cx| {
            let index = action.index.clamp(1, 9);
            navigation.dispatch(&window_handle, cx, move |desktop, window, cx| {
                desktop.select_focused_tab_at(window, index, cx)
            });
        }
    });

    cx.bind_keys([
        KeyBinding::new("secondary-d", OpenVerticalSplit, None),
        KeyBinding::new("secondary-shift-d", OpenHorizontalSplit, None),
        KeyBinding::new("cmd-t", OpenNewTab, None),
        KeyBinding::new("cmd-w", CloseActiveTab, None),
        KeyBinding::new("cmd-1", SelectTab { index: 1 }, None),
        KeyBinding::new("cmd-2", SelectTab { index: 2 }, None),
        KeyBinding::new("cmd-3", SelectTab { index: 3 }, None),
        KeyBinding::new("cmd-4", SelectTab { index: 4 }, None),
        KeyBinding::new("cmd-5", SelectTab { index: 5 }, None),
        KeyBinding::new("cmd-6", SelectTab { index: 6 }, None),
        KeyBinding::new("cmd-7", SelectTab { index: 7 }, None),
        KeyBinding::new("cmd-8", SelectTab { index: 8 }, None),
        KeyBinding::new("cmd-9", SelectTab { index: 9 }, None),
    ]);
}

impl DockWorkspace {
    fn focused_tab_group(&self, window: &Window, cx: &App) -> Option<FocusedTabGroup> {
        let focused_id = self.focused_pane_id(window, cx)?;
        let focused_key = self.panel_key(&focused_id)?;
        let area = self.area().read(cx);
        for placement in [
            DockPlacement::Center,
            DockPlacement::Left,
            DockPlacement::Right,
            DockPlacement::Bottom,
        ] {
            let Some(tree) = area.layout(placement) else {
                continue;
            };
            let mut found = None;
            tree.root().walk(&mut |node| {
                if let PaneRef::Tabs { panels, active_ix } = node.kind()
                    && panels.contains(&focused_key)
                {
                    found = Some(FocusedTabGroup {
                        panels: panels.to_vec(),
                        active_ix,
                    });
                }
            });
            if found.is_some() {
                return found;
            }
        }
        None
    }

    pub fn select_focused_tab_at(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let Some(group) = self.focused_tab_group(window, cx) else {
            return Ok(());
        };
        let Some(panel) = tab_at_index(&group.panels, index) else {
            return Ok(());
        };
        let id = self
            .pane_id_for_key(panel)
            .ok_or("The selected tab is unavailable.")?;
        if !self.focus(&id, window, cx) {
            return Err("The selected tab could not be focused.".into());
        }
        cx.notify();
        Ok(())
    }

    /// Browser shortcuts cycle the owning pane group, including its other
    /// surfaces; they must not jump to browser tabs in unrelated split panes.
    pub fn cycle_focused_tab(
        &mut self,
        forward: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let Some(group) = self.focused_tab_group(window, cx) else {
            return Ok(());
        };
        if group.panels.is_empty() {
            return Ok(());
        }
        let current = group.active_ix.min(group.panels.len() - 1);
        let next = if forward {
            (current + 1) % group.panels.len()
        } else {
            (current + group.panels.len() - 1) % group.panels.len()
        };
        let id = self
            .pane_id_for_key(group.panels[next])
            .ok_or("The next tab is unavailable.")?;
        if !self.focus(&id, window, cx) {
            return Err("The next tab could not be focused.".into());
        }
        cx.notify();
        Ok(())
    }

    pub fn close_focused_tab(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let Some(focused) = self.focused_tab_group(window, cx) else {
            return Ok(());
        };
        let Some(active) = active_tab(&focused.panels, focused.active_ix) else {
            return Err("The focused tab group is empty.".into());
        };
        let id = self
            .pane_id_for_key(active)
            .ok_or_else(|| "The focused tab is no longer available.".to_owned())?;
        if !self.is_closable(&id, cx) {
            return Err("This pane cannot be closed.".into());
        }
        if !self.remove(&id, window, cx) {
            return Err("The focused tab could not be closed.".into());
        }
        Ok(())
    }
}

struct FocusedTabGroup {
    panels: Vec<PanelId>,
    active_ix: usize,
}

#[cfg(test)]
mod tests {
    use super::{active_tab, tab_at_index};

    #[::core::prelude::v1::test]
    fn shortcuts_use_one_based_index_with_no_fallback_to_another_group() {
        let panels = ["first", "second", "third"];
        assert_eq!(tab_at_index(&panels, 1), Some("first"));
        assert_eq!(tab_at_index(&panels, 3), Some("third"));
        assert_eq!(tab_at_index(&panels, 0), None);
        assert_eq!(tab_at_index(&panels, 4), None);
        assert_eq!(active_tab(&panels, 1), Some("second"));
        assert_eq!(active_tab(&panels, 3), None);
    }
}
