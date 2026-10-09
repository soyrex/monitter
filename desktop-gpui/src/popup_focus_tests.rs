use gpui::{
    AppContext as _, Context, FocusHandle, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, Styled as _, TestAppContext, Window, div,
};
use gpui_kit::component::{Root, surface_backdrop};
use std::{cell::RefCell, rc::Rc};

struct FocusFixture {
    pane: FocusHandle,
    origin: FocusHandle,
    popup: FocusHandle,
}
impl Render for FocusFixture {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full()
            .child(div().track_focus(&self.pane)
                .child(div().track_focus(&self.origin).child("Composer")))
            .child(div().track_focus(&self.popup).child("Popup"))
    }
}

#[gpui_kit::test]
fn popup_focus_preserves_pane_activity_until_last_popup_closes(cx: &mut TestAppContext) {
    cx.update(|cx| gpui_kit::init(cx));
    let handles = Rc::new(RefCell::new(None));
    let slot = handles.clone();
    let (_, mut visual) = cx.add_window_view(move |window, cx| {
        let fixture = cx.new(|cx| {
            let pane = cx.focus_handle();
            let origin = cx.focus_handle();
            let popup = cx.focus_handle();
            *slot.borrow_mut() = Some((pane.clone(), origin.clone(), popup.clone()));
            FocusFixture { pane, origin, popup }
        });
        Root::new(fixture, window, cx)
    });
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
    let (pane, origin, popup) = handles.borrow_mut().take().unwrap();
    visual.update(|window, cx| {
        window.focus(&origin, cx);
        assert!(crate::docking::pane_visual_active(&pane, window, cx));
        surface_backdrop::set_active("outer-popup".into(), true, window, cx);
        window.focus(&popup, cx);
        assert!(!pane.contains_focused(window, cx));
        assert!(crate::docking::pane_visual_active(&pane, window, cx));
        surface_backdrop::set_active("nested-popup".into(), true, window, cx);
        surface_backdrop::set_active("outer-popup".into(), false, window, cx);
        assert!(crate::docking::pane_visual_active(&pane, window, cx));
        surface_backdrop::set_active("nested-popup".into(), false, window, cx);
        assert!(!crate::docking::pane_visual_active(&pane, window, cx));
        window.focus(&origin, cx);
        assert!(crate::docking::pane_visual_active(&pane, window, cx));
    });
}
