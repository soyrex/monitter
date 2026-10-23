//! Headless lock interactions use synthetic inputs, never the live profile.
use super::{LockState, SessionLock};
use gpui::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement as _, Render, Styled as _, TestAppContext, Window, div,
};
use gpui_kit::component::input::InputState;
use std::{cell::Cell, rc::Rc};
struct Harness {
    lock: Entity<SessionLock>,
    leaked_keys: Rc<Cell<usize>>,
}
impl Render for Harness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let leaked = self.leaked_keys.clone();
        div()
            .size_full()
            .on_key_down(move |event: &KeyDownEvent, _, _| {
                if event.keystroke.key == "escape" {
                    leaked.set(leaked.get() + 1);
                }
            })
            .child(self.lock.clone())
    }
}
#[gpui::test]
fn session_lock_escape_stays_inside_dialog_and_pin_is_masked(cx: &mut TestAppContext) {
    cx.update(|cx| gpui_kit::init(cx));
    let leaked = Rc::new(Cell::new(0));
    let count = leaked.clone();
    let (host, visual) = cx.add_window_view(move |window, cx| {
        let lock = cx.new(|cx| {
            cx.set_global(LockState(true));
            let mut make =
                |cx: &mut gpui::App| cx.new(|cx| InputState::new(window, cx).masked(true));
            let pin = make(cx);
            let current = make(cx);
            let confirmation = make(cx);
            SessionLock {
                disconnected_fixture: true,
                focus: cx.focus_handle(),
                locked: true,
                configured: Some(true),
                busy: false,
                locking: false,
                pin,
                current,
                confirmation,
                error: None,
                notice: None,
                attempts: 0,
                retry_after: None,
                retry_timer: None,
                palette: crate::design::palette(
                    &serde_json::Value::Null,
                    gpui::WindowAppearance::Dark,
                ),
                lock_generation: 0,
            }
        });
        Harness {
            lock,
            leaked_keys: count,
        }
    });
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
    visual.simulate_keystrokes("0");
    visual.run_until_parked();
    visual.simulate_input("427");
    visual.simulate_keystrokes("backspace");
    visual.simulate_input("7");
    visual.simulate_keystrokes("escape");
    host.read_with(visual, |host, cx| {
        let lock = host.lock.read(cx);
        assert!(lock.is_locked());
        assert!(super::locked(cx));
        assert_eq!(lock.pin.read(cx).value().as_ref(), "0427");
        assert!(lock.pin.read(cx).presentation().is_masked());
    });
    assert_eq!(leaked.get(), 0, "escape must not reach the workspace");
    visual.update(|window, cx| window.draw(cx).clear(cx));
    visual.simulate_click(gpui::point(gpui::px(10.), gpui::px(10.)), Default::default());
    host.read_with(visual, |host, cx| {
        assert_eq!(host.lock.read(cx).pin.read(cx).value().as_ref(), "0427", "click preserves entered digits without a dialog");
        assert!(host.lock.read(cx).is_locked(), "click cannot unlock the workspace");
    });
}
