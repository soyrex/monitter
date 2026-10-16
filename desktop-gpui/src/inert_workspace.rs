//! Live workspace paint without input, focus, tab stops, or accessibility.
use gpui_kit::*;
pub struct InertWorkspace {
    child: AnyElement,
}
impl InertWorkspace {
    pub fn new(child: AnyElement) -> Self {
        Self { child }
    }
}
impl IntoElement for InertWorkspace {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
impl Element for InertWorkspace {
    type RequestLayoutState = ();
    type PrepaintState = ();
    // The interactive workspace never uses this namespace. Cached state and
    // dispatch ranges cannot leak between its normal and locked appearances.
    fn id(&self) -> Option<ElementId> {
        Some("inert-workspace".into())
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (
            window.with_inert_content(true, |window| self.child.request_layout(window, cx)),
            (),
        )
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        window.with_inert_content(true, |window| self.child.prepaint(window, cx));
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        window.with_inert_content(true, |window| self.child.paint(window, cx));
    }
}

#[cfg(all(test, feature = "visual-export"))]
mod tests {
    use super::InertWorkspace;
    use gpui::StatefulInteractiveElement as _;
    use gpui::{
        App, AppContext as _, Context, Entity, FocusHandle, InteractiveElement as _, IntoElement,
        KeyDownEvent, Modifiers, ParentElement as _, Render, Role, StyleRefinement, Styled as _,
        TestAppContext, Window, div, point, px,
    };
    use std::{cell::Cell, rc::Rc};
    struct Child {
        focus: FocusHandle,
        clicks: Rc<Cell<usize>>,
        keys: Rc<Cell<usize>>,
        accessible: Rc<Cell<bool>>,
    }
    impl Render for Child {
        fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            self.accessible.set(window.is_a11y_enabled());
            let clicks = self.clicks.clone();
            let keys = self.keys.clone();
            div()
                .id("private-workspace-control")
                .role(Role::Button)
                .track_focus(&self.focus)
                .size_full()
                .on_click(move |_, _, _| clicks.set(clicks.get() + 1))
                .on_key_down(move |_: &KeyDownEvent, _, _| keys.set(keys.get() + 1))
                .child("Private transcript text")
        }
    }
    struct Harness {
        inert: bool,
        child: Entity<Child>,
    }
    impl Render for Harness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let child = self
                .child
                .clone()
                .cached(StyleRefinement::default().size_full())
                .into_any_element();
            div().size_full().child(if self.inert {
                InertWorkspace::new(child).into_any_element()
            } else {
                child
            })
        }
    }
    #[gpui::test]
    fn session_lock_inert_workspace_cached_child_has_no_mouse_keys_or_accessibility_then_restores(
        cx: &mut TestAppContext,
    ) {
        let clicks = Rc::new(Cell::new(0));
        let keys = Rc::new(Cell::new(0));
        let accessible = Rc::new(Cell::new(false));
        let (a, b, c) = (clicks.clone(), keys.clone(), accessible.clone());
        let (host, visual) = cx.add_window_view(move |window, cx| {
            let child = cx.new(|cx| {
                let focus = cx.focus_handle();
                focus.focus(window, cx);
                Child {
                    focus,
                    clicks: a,
                    keys: b,
                    accessible: c,
                }
            });
            Harness {
                inert: false,
                child,
            }
        });
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        assert!(accessible.get());
        visual.simulate_click(point(px(10.), px(10.)), Modifiers::default());
        visual.simulate_keystrokes("a");
        assert!(clicks.get() > 0);
        assert!(keys.get() > 0);
        host.update(visual, |host, cx| {
            host.inert = true;
            cx.notify();
        });
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        assert!(!accessible.get());
        let before = (clicks.get(), keys.get());
        for _ in 0..2 {
            visual.simulate_click(point(px(10.), px(10.)), Modifiers::default());
            visual.simulate_keystrokes("a tab");
            visual.update(|window, cx| window.refresh());
            visual.run_until_parked();
            visual.update(|window, cx| window.draw(cx).clear(cx));
        }
        assert_eq!((clicks.get(), keys.get()), before);
        host.update(visual, |host, cx| {
            host.inert = false;
            cx.notify();
        });
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        visual.update(|window, _| assert!(window.is_a11y_enabled()));
        visual.simulate_click(point(px(10.), px(10.)), Modifiers::default());
        assert!(clicks.get() > before.0);
    }
}
