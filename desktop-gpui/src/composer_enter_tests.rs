//! End-to-end Enter dispatch through GPUI's real multiline Textarea.
//!
//! This mirrors ChatPane's ancestor `capture_action(Enter, ...)` gate while
//! keeping the send effect local to the test; it never invokes a provider.
use gpui::{
    AppContext as _, Context, Entity, FocusHandle, Focusable as _, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, Styled as _, TestAppContext, VisualTestContext,
    Window, div,
};
use gpui_kit::{
    EntityInputHandler,
    component::input::{Enter, Textarea, TextareaState},
    test::TestWindowExt,
};
use std::{cell::Cell, rc::Rc};

struct ComposerEnterHarness {
    input: Entity<TextareaState>,
    focus: FocusHandle,
    send_with_enter: bool,
    sends: Rc<Cell<usize>>,
}

impl Render for ComposerEnterHarness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let input = self.input.clone();
        let sends = self.sends.clone();
        let send_with_enter = self.send_with_enter;
        div()
            .size_full()
            .key_context("MonitterChat")
            .track_focus(&self.focus)
            .capture_action(move |action: &Enter, window, cx| {
                let keyboard_available = input.read(cx).focus_handle(cx).is_focused(window)
                    && input.update(cx, |state, cx| {
                        EntityInputHandler::marked_text_range(state, window, cx).is_none()
                    });
                if keyboard_available && !action.shift && (send_with_enter || action.secondary) {
                    sends.set(sends.get() + 1);
                    cx.stop_propagation();
                }
            })
            .child(Textarea::new(&self.input).h(gpui_kit::px(100.)))
    }
}

fn composer(
    cx: &mut TestAppContext,
    send_with_enter: bool,
) -> (
    &mut VisualTestContext,
    Entity<TextareaState>,
    Rc<Cell<usize>>,
) {
    cx.update(|app| {
        gpui_kit::init(app);
        app.bind_keys(crate::shortcut_desktop::pane_and_workspace_bindings());
        app.on_action(|_: &crate::native_menu::Fullscreen, _| {});
    });
    let input_slot = Rc::new(std::cell::RefCell::new(None));
    let sends = Rc::new(Cell::new(0));
    let (_, visual) = cx.add_window_view({
        let input_slot = input_slot.clone();
        let sends = sends.clone();
        move |window, cx| {
            let input = cx.new(|cx| TextareaState::new(window, cx));
            *input_slot.borrow_mut() = Some(input.clone());
            let view = cx.new(|cx| ComposerEnterHarness {
                input,
                focus: cx.focus_handle(),
                send_with_enter,
                sends,
            });
            gpui_kit::component::Root::new(view, window, cx)
        }
    });
    visual.update(|window, cx| window.draw(cx).clear(cx));
    let input = input_slot
        .borrow()
        .as_ref()
        .expect("textarea state")
        .clone();
    (visual, input, sends)
}

fn enter(cx: &mut VisualTestContext, key: &str) {
    cx.update(|window, app| window.press(key, app));
}

#[gpui::test]
fn plain_enter_sends_only_when_preference_is_enabled(cx: &mut TestAppContext) {
    let (visual, input, sends) = composer(cx, true);
    visual.update(|window, app| input.update(app, |state, cx| state.focus(window, cx)));
    enter(visual, "enter");
    assert_eq!(sends.get(), 1);
    input.read_with(cx, |state, _| assert_eq!(state.value().as_ref(), ""));

    let (visual, input, sends) = composer(cx, false);
    visual.update(|window, app| input.update(app, |state, cx| state.focus(window, cx)));
    enter(visual, "enter");
    assert_eq!(sends.get(), 0);
    input.read_with(cx, |state, _| assert_eq!(state.value().as_ref(), "\n"));
}

#[gpui::test]
fn shift_enter_inserts_newline_and_cmd_enter_does_not_send(cx: &mut TestAppContext) {
    let (visual, input, sends) = composer(cx, true);
    visual.update(|window, app| input.update(app, |state, cx| state.focus(window, cx)));
    enter(visual, "shift-enter");
    assert_eq!(sends.get(), 0);
    input.read_with(cx, |state, _| assert_eq!(state.value().as_ref(), "\n"));

    let (visual, input, sends) = composer(cx, false);
    visual.update(|window, app| input.update(app, |state, cx| state.focus(window, cx)));
    enter(visual, "cmd-enter");
    assert_eq!(sends.get(), 0);
    input.read_with(cx, |state, _| assert_eq!(state.value().as_ref(), ""));
}

#[gpui::test]
fn marked_ime_composition_enter_never_sends(cx: &mut TestAppContext) {
    let (visual, input, sends) = composer(cx, true);
    visual.update(|window, app| {
        input.update(app, |state, cx| {
            state.focus(window, cx);
            EntityInputHandler::replace_and_mark_text_in_range(state, None, "に", None, window, cx);
            assert!(EntityInputHandler::marked_text_range(state, window, cx).is_some());
        });
    });
    enter(visual, "enter");
    assert_eq!(sends.get(), 0);
}

#[gpui::test]
fn fullscreen_aliases_dispatch_without_sending_or_changing_composer(cx: &mut TestAppContext) {
    let toggles = Rc::new(Cell::new(0));
    let (visual, input, sends) = composer(cx, true);
    let observed = toggles.clone();
    visual.update(move |_, app| { app.on_action(move |_: &crate::native_menu::Fullscreen, _| {
        observed.set(observed.get() + 1);
    }); });
    visual.update(|window, app| input.update(app, |state, cx| {
        state.set_value("draft", window, cx);
        state.focus(window, cx);
    }));
    enter(visual, "cmd-ctrl-f");
    enter(visual, "cmd-enter");
    assert_eq!(toggles.get(), 2);
    assert_eq!(sends.get(), 0);
    input.read_with(cx, |state, _| assert_eq!(state.value().as_ref(), "draft"));
}

#[gpui::test]
fn fullscreen_aliases_toggle_real_window_from_focused_composer(cx: &mut TestAppContext) {
    let (visual, input, sends) = composer(cx, true);
    visual.update(|window, app| {
        crate::native_menu::install(app);
        window.activate_window();
        input.update(app, |state, cx| {
            state.set_value("preserved draft", window, cx);
            state.focus(window, cx);
        });
        assert!(!window.is_fullscreen());
    });
    enter(visual, "cmd-ctrl-f");
    visual.run_until_parked();
    visual.update(|window, _| assert!(window.is_fullscreen(), "Ctrl+Cmd+F enters fullscreen"));
    enter(visual, "cmd-enter");
    visual.run_until_parked();
    visual.update(|window, _| assert!(!window.is_fullscreen(), "Cmd+Enter exits fullscreen"));
    assert_eq!(sends.get(), 0);
    input.read_with(cx, |state, _| assert_eq!(state.value().as_ref(), "preserved draft"));
}
