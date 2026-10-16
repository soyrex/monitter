//! Keyboard precedence regression for native workspace bindings around text input.
use gpui::{
    div, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Focusable as _, Render, Styled as _, TestAppContext, VisualTestContext, Window,
};
use gpui_kit::{
    component::input::{Input, InputState},
    test::TestWindowExt, EntityInputHandler,
    KeyBinding,
};
use std::{cell::Cell, rc::Rc};

struct InputShortcutHarness {
    input: Entity<InputState>,
    workspace_dispatches: Rc<Cell<usize>>,
}

impl Render for InputShortcutHarness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let tab_dispatches = self.workspace_dispatches.clone();
        let pane_dispatches = self.workspace_dispatches.clone();
        let switch_dispatches = self.workspace_dispatches.clone();
        let select_dispatches = self.workspace_dispatches.clone();
        let select_input = self.input.clone();
        let switch_input = self.input.clone();
        let tab_input = self.input.clone();
        let pane_input = self.input.clone();
        div()
            .size_full()
            .key_context("MonitterDesktop shortcutMode=standard")
            .on_action(move |_: &crate::shortcut_desktop::MoveFocusedTab, window, cx| {
                let focused = tab_input
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window);
                let marked = focused
                    && tab_input.update(cx, |state, cx| {
                        EntityInputHandler::marked_text_range(state, window, cx).is_some()
                    });
                if !crate::shortcut_desktop::focused_input_is_composing(focused, marked) {
                    tab_dispatches.set(tab_dispatches.get() + 1);
                }
            })
            .on_action(
                move |_: &crate::shortcut_desktop::FocusAdjacentPane, window, cx| {
                    let focused = pane_input
                        .read(cx)
                        .focus_handle(cx)
                        .is_focused(window);
                    let marked = focused
                        && pane_input.update(cx, |state, cx| {
                            EntityInputHandler::marked_text_range(state, window, cx).is_some()
                        });
                    if !crate::shortcut_desktop::focused_input_is_composing(focused, marked) {
                        pane_dispatches.set(pane_dispatches.get() + 1);
                    }
                },
            )
            .on_action(move |_: &crate::shortcut_desktop::SwitchNamedWorkspace, window, cx| {
                let focused = switch_input.read(cx).focus_handle(cx).is_focused(window);
                let marked = focused && switch_input.update(cx, |state, cx| {
                    EntityInputHandler::marked_text_range(state, window, cx).is_some()
                });
                if !crate::shortcut_desktop::focused_input_is_composing(focused, marked) {
                    switch_dispatches.set(switch_dispatches.get() + 1);
                }
            })
            .on_action(move |action: &crate::shortcut_desktop::SelectNamedWorkspace, window, cx| {
                let focused = select_input.read(cx).focus_handle(cx).is_focused(window);
                let marked = focused && select_input.update(cx, |state, cx| {
                    EntityInputHandler::marked_text_range(state, window, cx).is_some()
                });
                if !crate::shortcut_desktop::focused_input_is_composing(focused, marked) {
                    select_dispatches.set(action.index);
                }
            })
            .child(Input::new(&self.input))
    }
}

fn input_harness(
    cx: &mut TestAppContext,
) -> (&mut VisualTestContext, Entity<InputState>, Rc<Cell<usize>>) {
    cx.update(gpui_kit::init);
    // Reproduce the production keymap's unscoped bindings. Since these are
    // installed after gpui-kit, they win ties with `Input` actions at depth 2.
    cx.update(|app| {
        app.bind_keys(crate::shortcut_desktop::pane_and_workspace_bindings());
        app.bind_keys([
            KeyBinding::new(
                "cmd-alt-up",
                crate::shortcut_desktop::FocusAdjacentPane {
                    direction: "up".into(),
                },
                None,
            ),
        ]);
    });

    let state_slot = Rc::new(std::cell::RefCell::new(None));
    let workspace_dispatches = Rc::new(Cell::new(0));
    let (_, visual) = cx.add_window_view({
        let state_slot = state_slot.clone();
        let workspace_dispatches = workspace_dispatches.clone();
        move |window, cx| {
            let input = cx.new(|cx| InputState::new(window, cx).default_value("sample text"));
            *state_slot.borrow_mut() = Some(input.clone());
            let harness = cx.new(|_| InputShortcutHarness {
                input,
                workspace_dispatches,
            });
            gpui_kit::component::Root::new(harness, window, cx)
        }
    });
    visual.update(|window, cx| window.draw(cx).clear(cx));
    let input = state_slot.borrow().as_ref().expect("input state").clone();
    (visual, input, workspace_dispatches)
}

#[gpui::test]
fn cmd_ctrl_left_moves_workspace_tab_even_with_input_focused(cx: &mut TestAppContext) {
    let (cx, input, workspace_dispatches) = input_harness(cx);
    cx.update(|window, app| {
        input.update(app, |state, cx| {
            state.set_selected_range(11..11, cx);
            state.focus(window, cx);
        });
        window.press("cmd-ctrl-left", app);
    });

    input.read_with(cx, |state, _| {
        assert_eq!(state.selected_range(), 11..11);
    });
    assert_eq!(workspace_dispatches.get(), 1);
}

#[gpui::test]
fn option_command_navigation_still_reaches_workspace_when_input_is_focused(
    cx: &mut TestAppContext,
) {
    let (cx, input, workspace_dispatches) = input_harness(cx);
    cx.update(|window, app| {
        input.update(app, |state, cx| state.focus(window, cx));
        window.press("cmd-alt-up", app);
    });
    assert_eq!(workspace_dispatches.get(), 1);
    input.read_with(cx, |state, _| {
        assert_eq!(state.value().as_ref(), "sample text")
    });
}

#[gpui::test]
fn ordinary_shift_left_keeps_native_input_selection(cx: &mut TestAppContext) {
    let (cx, input, workspace_dispatches) = input_harness(cx);
    cx.update(|window, app| {
        input.update(app, |state, cx| {
            state.set_selected_range(11..11, cx);
            state.focus(window, cx);
        });
        window.press("shift-left", app);
    });
    input.read_with(cx, |state, _| assert_eq!(state.selected_range(), 10..11));
    assert_eq!(workspace_dispatches.get(), 0);
}

#[gpui::test]
fn workspace_shortcuts_wait_for_marked_text_composition_to_finish(cx: &mut TestAppContext) {
    let (cx, input, workspace_dispatches) = input_harness(cx);
    cx.update(|window, app| {
        input.update(app, |state, cx| {
            state.focus(window, cx);
            EntityInputHandler::replace_and_mark_text_in_range(
                state,
                None,
                "に",
                None,
                window,
                cx,
            );
            assert!(EntityInputHandler::marked_text_range(state, window, cx).is_some());
        });
        for key in ["cmd-ctrl-left", "cmd-shift-left", "cmd-shift-right", "cmd-shift-up", "cmd-shift-down", "cmd-[", "cmd-]", "cmd-1", "cmd-9"] {
            window.press(key, app);
        }
    });
    assert_eq!(workspace_dispatches.get(), 0);
}

#[gpui::test]
fn named_workspace_shortcuts_override_input_selection(cx: &mut TestAppContext) {
    let (cx, input, dispatches) = input_harness(cx);
    for (index, key) in ["cmd-shift-left", "cmd-shift-right", "cmd-shift-up", "cmd-shift-down", "cmd-[", "cmd-]"].into_iter().enumerate() {
        cx.update(|window, app| {
            input.update(app, |state, cx| {
                state.set_selected_range(11..11, cx);
                state.focus(window, cx);
            });
            window.press(key, app);
        });
        assert_eq!(dispatches.get(), index + 1, "{key} reaches workspace action");
        input.read_with(cx, |state, _| {
            assert_eq!(state.selected_range(), 11..11, "{key} does not select text");
            assert_eq!(state.value().as_ref(), "sample text");
        });
    }
}

#[gpui::test]
fn option_shift_left_keeps_word_selection(cx: &mut TestAppContext) {
    let (cx, input, dispatches) = input_harness(cx);
    cx.update(|window, app| {
        input.update(app, |state, cx| {
            state.set_selected_range(11..11, cx);
            state.focus(window, cx);
        });
        window.press("alt-shift-left", app);
    });
    input.read_with(cx, |state, _| assert_eq!(state.selected_range(), 7..11));
    assert_eq!(dispatches.get(), 0);
}

#[gpui::test]
fn numbered_workspace_shortcuts_select_matching_position_with_input_focused(cx: &mut TestAppContext) {
    let (cx, input, dispatches) = input_harness(cx);
    for index in 1..=9 {
        cx.update(|window, app| {
            input.update(app, |state, cx| {
                state.set_selected_range(11..11, cx);
                state.focus(window, cx);
            });
            window.press(&format!("cmd-{index}"), app);
        });
        assert_eq!(dispatches.get(), index);
        input.read_with(cx, |state, _| {
            assert_eq!(state.value().as_ref(), "sample text");
            assert_eq!(state.selected_range(), 11..11);
        });
    }
}
