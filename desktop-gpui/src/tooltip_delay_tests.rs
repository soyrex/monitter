use gpui::{AppContext as _, Bounds, Context, Entity, IntoElement, Render,
    TestAppContext, VisualTestContext, Window, div, point, px, size};
use gpui_kit::{base::{TooltipOverlay, TooltipRequest}, component::Root};
use std::{cell::{Cell, RefCell}, rc::Rc, time::Duration};

struct TooltipContent;
impl Render for TooltipContent {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}
fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
}
#[gpui_kit::test]
fn tooltip_delay_waits_full_second_for_each_trigger(cx: &mut TestAppContext) {
    cx.update(|cx| gpui_kit::init(cx));
    let slot: Rc<RefCell<Option<Entity<TooltipOverlay>>>> = Rc::new(RefCell::new(None));
    let retained = slot.clone();
    let (_, mut visual) = cx.add_window_view(move |window, cx| {
        let overlay = cx.new(|_| TooltipOverlay::new());
        *retained.borrow_mut() = Some(overlay.clone());
        Root::new(overlay, window, cx)
    });
    let overlay = slot.borrow_mut().take().unwrap();
    let builds = Rc::new(Cell::new(0));
    draw(&mut visual);
    for _ in 0..2 {
        let before = builds.get();
        let counter = builds.clone();
        visual.update(|window, cx| overlay.update(cx, |overlay, cx| {
            overlay.request_show(TooltipRequest::new(
                Bounds::new(point(px(20.), px(20.)), size(px(30.), px(30.))),
                move |_, cx| {
                    counter.set(counter.get() + 1);
                    cx.new(|_| TooltipContent).into()
                }), window, cx);
        }));
        draw(&mut visual);
        visual.executor().advance_clock(Duration::from_millis(999));
        draw(&mut visual);
        assert_eq!(builds.get(), before, "tooltip appeared before a full second");
        visual.executor().advance_clock(Duration::from_millis(1));
        draw(&mut visual);
        assert!(builds.get() > before);
    }
}
