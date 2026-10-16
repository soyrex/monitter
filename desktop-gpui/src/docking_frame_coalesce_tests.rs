//! Multiple input-driven draws before one platform frame must reconcile pane visibility once.
use super::{DockWorkspace, LifecycleHook, PaneFactory, PaneLifecycle, PaneSpec};
use gpui::{AppContext as _, Context, IntoElement, Render, Styled as _, TestAppContext, Window, div};
use gpui_kit::component::dock::DockPlacement;
use std::{cell::RefCell, rc::Rc};

struct Content;
impl Render for Content {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full()
    }
}

#[gpui_kit::test]
fn multiple_draws_before_next_frame_do_not_hide_a_still_painted_pane(cx: &mut TestAppContext) {
    cx.update(|cx| gpui_kit::init(cx));
    let transitions = Rc::new(RefCell::new(Vec::new()));
    let captured = transitions.clone();
    let (_, visual) = cx.add_window_view(move |window, cx| {
        let factory: PaneFactory = Rc::new(|_, _, _| None);
        let workspace = cx.new(|cx| DockWorkspace::new("frame-coalesce-regression", factory, window, cx));
        workspace.update(cx, |workspace, cx| {
            let lifecycle: LifecycleHook = Rc::new(move |event, _, _| {
                if let PaneLifecycle::Visible(visible) = event {
                    captured.borrow_mut().push(visible);
                }
            });
            let spec = PaneSpec {
                id: "browser:fixture".into(),
                title: "Browser fixture".into(),
                content: cx.new(|_| Content).into(),
                focus: cx.focus_handle(),
                lifecycle: Some(lifecycle),
                closable: true,
            };
            workspace.add(spec, DockPlacement::Center, None, window, cx).unwrap();
        });
        gpui_kit::component::Root::new(workspace, window, cx)
    });

    visual.run_until_parked();
    // Ignore callbacks queued while the test harness establishes its initial
    // window; the controlled draw batch below is the regression trigger.
    transitions.borrow_mut().clear();

    // A key dispatch can cause synchronous draws before the platform delivers
    // its next frame. All draws must share one visibility snapshot/callback.
    visual.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.draw(cx).clear(cx);
        window.draw(cx).clear(cx);
        window.simulate_next_frame(cx);
    });
    assert_eq!(
        &*transitions.borrow(),
        &[true],
        "three draws of a pane must not produce a hidden transition"
    );
    transitions.borrow_mut().clear();
    visual.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.simulate_next_frame(cx);
    });
    assert!(
        transitions.borrow().is_empty(),
        "steady subsequent frames must not repeat show/hide transitions"
    );
}
