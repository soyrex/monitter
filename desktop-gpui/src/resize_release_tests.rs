use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    AppContext as _, Context, InteractiveElement as _, IntoElement, Modifiers, MouseButton,
    ParentElement as _, Render, Styled as _, TestAppContext, VisualTestContext, Window, div, point,
    px,
};
use gpui_kit::component::resizable::{ResizableState, h_resizable, resizable_panel};

struct ResizableReleaseHarness {
    state: gpui::Entity<ResizableState>,
    resize_commits: Rc<Cell<usize>>,
    stop_mouse_up: bool,
    stop_mouse_up_out: bool,
}

impl Render for ResizableReleaseHarness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let content = div()
            .size_full()
            .debug_selector(|| "release-content".into());
        let content = if self.stop_mouse_up {
            content.on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        } else {
            content
        };
        let content = if self.stop_mouse_up_out {
            content.on_mouse_up_out(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        } else {
            content
        };

        div().w(px(400.)).h(px(100.)).child(
            h_resizable("resize-release-regression")
                .with_state(&self.state)
                .on_resize({
                    let resize_commits = self.resize_commits.clone();
                    move |_, _, _| resize_commits.set(resize_commits.get() + 1)
                })
                .child(
                    resizable_panel().size(px(150.)).child(
                        div()
                            .size_full()
                            .debug_selector(|| "first-release-panel".into()),
                    ),
                )
                .child(
                    resizable_panel()
                        .size(px(250.))
                        .child(content.debug_selector(|| "second-release-panel".into())),
                ),
        )
    }
}

fn release_harness(
    cx: &mut TestAppContext,
    stop_mouse_up: bool,
    stop_mouse_up_out: bool,
) -> (
    &mut VisualTestContext,
    gpui::Entity<ResizableState>,
    Rc<Cell<usize>>,
) {
    cx.update(|cx| gpui_kit::init(cx));
    let state = cx.update(|cx| cx.new(|_| ResizableState::default()));
    let resize_commits = Rc::new(Cell::new(0));
    let (_, cx) = cx.add_window_view({
        let state = state.clone();
        let resize_commits = resize_commits.clone();
        move |_, _| ResizableReleaseHarness {
            state,
            resize_commits,
            stop_mouse_up,
            stop_mouse_up_out,
        }
    });
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.update(|window, cx| window.draw(cx).clear(cx));
    (cx, state, resize_commits)
}

fn drag_to(cx: &mut VisualTestContext, to_x: f32) {
    // Read the current boundary for every drag. The second drag runs after the
    // first resize, so a fixed initial x would no longer hit the handle.
    let boundary = cx
        .debug_bounds("second-release-panel")
        .expect("second panel should be laid out")
        .left()
        .as_f32();
    cx.simulate_mouse_down(
        point(px(boundary - 2.), px(50.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.simulate_mouse_move(
        point(px(boundary + 10.), px(50.)),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    cx.simulate_mouse_move(
        point(px(to_x), px(50.)),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
}

#[gpui::test]
fn release_ends_drag_when_panel_content_stops_mouse_up_and_mouse_up_out(cx: &mut TestAppContext) {
    let (cx, state, resize_commits) = release_harness(cx, true, true);

    drag_to(cx, 220.);
    cx.simulate_mouse_up(
        point(px(220.), px(50.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.update(|window, cx| window.draw(cx).clear(cx));

    state.read_with(cx, |state, _| {
        assert_eq!(state.sizes(), &vec![px(220.), px(180.)]);
    });
    assert_eq!(resize_commits.get(), 1);

    // An unpressed move after release must not continue the previous drag.
    cx.simulate_mouse_move(point(px(300.), px(50.)), None, Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    state.read_with(cx, |state, _| {
        assert_eq!(state.sizes(), &vec![px(220.), px(180.)]);
    });
    assert_eq!(resize_commits.get(), 1);
}

#[gpui::test]
fn unpressed_move_recovers_missed_release_and_next_drag_is_independent(cx: &mut TestAppContext) {
    let (cx, state, resize_commits) = release_harness(cx, false, false);

    // Deliberately omit mouse-up. A move without a pressed button must finish
    // the gesture exactly once instead of leaving the handle in its drag state.
    drag_to(cx, 220.);
    cx.simulate_mouse_move(point(px(220.), px(50.)), None, Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    state.read_with(cx, |state, _| {
        assert_eq!(state.sizes(), &vec![px(220.), px(180.)]);
    });
    assert_eq!(resize_commits.get(), 1);

    // A later left-button gesture remains usable and produces its own commit.
    drag_to(cx, 260.);
    cx.simulate_mouse_up(
        point(px(260.), px(50.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.update(|window, cx| window.draw(cx).clear(cx));
    state.read_with(cx, |state, _| {
        assert_eq!(state.sizes(), &vec![px(260.), px(140.)]);
    });
    assert_eq!(resize_commits.get(), 2);
}

#[gpui::test]
fn deactivation_commits_an_in_progress_resize(cx: &mut TestAppContext) {
    let (cx, state, resize_commits) = release_harness(cx, false, false);

    drag_to(cx, 220.);
    state.read_with(cx, |state, _| {
        assert_eq!(state.sizes(), &vec![px(220.), px(180.)]);
    });

    cx.deactivate_window();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    state.read_with(cx, |state, _| {
        assert_eq!(state.sizes(), &vec![px(220.), px(180.)]);
    });
    assert_eq!(resize_commits.get(), 1);

    // Blur ends the held gesture; a later unpressed move cannot alter it.
    cx.simulate_mouse_move(point(px(300.), px(50.)), None, Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    state.read_with(cx, |state, _| {
        assert_eq!(state.sizes(), &vec![px(220.), px(180.)]);
    });
    assert_eq!(resize_commits.get(), 1);
}
