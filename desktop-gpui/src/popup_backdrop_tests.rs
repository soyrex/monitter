//! Exercise popup backdrop ownership through the component Root and Popover.
use futures::channel::oneshot;
use gpui_kit::{
    AppContext as _, Context, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    Styled as _, TestAppContext, VisualTestContext, Window, div, point, px, rgb, size,
};
use gpui_kit::{
    RenderImage,
    component::{
        WindowExt, button::Button, popover::Popover, surface_backdrop::PopupBackdropFactory,
    },
};
use image::{Frame, ImageBuffer, Rgba};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
};

const TEST_WIDTH: u32 = 800;
const TEST_HEIGHT: u32 = 600;

type PendingCapture = Option<oneshot::Sender<Option<Arc<RenderImage>>>>;

fn test_image(color: [u8; 4]) -> Arc<RenderImage> {
    Arc::new(RenderImage::new(vec![Frame::new(ImageBuffer::from_pixel(
        TEST_WIDTH,
        TEST_HEIGHT,
        Rgba(color),
    ))]))
}

struct PopupHarness {
    captures: Rc<Cell<usize>>,
    content_saw_capture: Rc<Cell<bool>>,
}

impl Render for PopupHarness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let captures = self.captures.clone();
        let content_saw_capture = self.content_saw_capture.clone();
        div().size_full().bg(rgb(0x181818)).child(
            Popover::new("outer-popup")
                .trigger(
                    Button::new("outer-popup-trigger").child(
                        div()
                            .debug_selector(|| "outer-popup-trigger".into())
                            .child("Open"),
                    ),
                )
                .content(move |_, _, _| {
                    content_saw_capture.set(captures.get() > 0);
                    div()
                        .id("outer-popup-content")
                        .debug_selector(|| "outer-popup-content".into())
                        .w(px(180.))
                        .h(px(120.))
                        .child(
                            Popover::new("nested-popup")
                                .trigger(
                                    Button::new("nested-popup-trigger").child(
                                        div()
                                            .debug_selector(|| "nested-popup-trigger".into())
                                            .child("Nested"),
                                    ),
                                )
                                .content(|_, _, _| {
                                    div()
                                        .id("nested-popup-content")
                                        .debug_selector(|| "nested-popup-content".into())
                                        .size(px(60.))
                                }),
                        )
                }),
        )
    }
}

#[gpui_kit::test]
fn popover_backdrop_captures_before_mount_shares_leases_and_fences_stale_completion(
    cx: &mut TestAppContext,
) {
    cx.update(|cx| gpui_kit::init(cx));
    let captures = Rc::new(Cell::new(0usize));
    let pending = Rc::new(RefCell::new(Vec::<PendingCapture>::new()));
    let captures_for_factory = captures.clone();
    let pending_for_factory = pending.clone();
    let factory: PopupBackdropFactory = Rc::new(move |_, cx| {
        captures_for_factory.set(captures_for_factory.get() + 1);
        let (sender, receiver) = oneshot::channel();
        pending_for_factory.borrow_mut().push(Some(sender));
        Some(
            cx.background_executor()
                .spawn(async move { receiver.await.ok().flatten() }),
        )
    });

    let content_saw_capture = Rc::new(Cell::new(false));
    let (_, visual) = cx.add_window_view({
        let captures = captures.clone();
        let content_saw_capture = content_saw_capture.clone();
        move |window, cx| {
            let content = cx.new(|_| PopupHarness {
                captures,
                content_saw_capture,
            });
            gpui_kit::component::Root::new(content, window, cx)
        }
    });
    visual.simulate_resize(size(px(TEST_WIDTH as f32), px(TEST_HEIGHT as f32)));
    visual.run_until_parked();
    visual.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.configure_popup_backdrop(cx, Some(factory));
    });

    let trigger = visual
        .debug_bounds("outer-popup-trigger")
        .expect("outer trigger rendered");
    visual.simulate_click(trigger.center(), Default::default());
    visual.update(|window, cx| window.draw(cx).clear(cx));
    assert_eq!(captures.get(), 1, "first open acquires one scene capture");
    assert!(
        content_saw_capture.get(),
        "capture starts before the content closure mounts"
    );
    assert!(visual.debug_bounds("outer-popup-content").is_some());

    let nested = visual
        .debug_bounds("nested-popup-trigger")
        .expect("nested trigger rendered");
    visual.simulate_click(nested.center(), Default::default());
    visual.update(|window, cx| window.draw(cx).clear(cx));
    assert!(visual.debug_bounds("nested-popup-content").is_some());
    assert_eq!(
        captures.get(),
        1,
        "nested popovers share the active capture"
    );

    let outer = visual.debug_bounds("outer-popup-content").unwrap();
    visual.simulate_click(
        point(outer.right() - px(12.), outer.bottom() - px(12.)),
        Default::default(),
    );
    visual.update(|window, cx| window.draw(cx).clear(cx));
    assert!(visual.debug_bounds("outer-popup-content").is_some());
    assert!(visual.debug_bounds("nested-popup-content").is_none());
    assert_eq!(
        captures.get(),
        1,
        "closing one owner retains the shared capture for the remaining popup"
    );

    visual.simulate_click(point(px(500.), px(500.)), Default::default());
    visual.update(|window, cx| window.draw(cx).clear(cx));
    assert!(visual.debug_bounds("outer-popup-content").is_none());
    assert!(visual.debug_bounds("nested-popup-content").is_none());

    // Reopen while the first capture is still pending. A completion from the
    // old generation must not displace the newly opened popup's image.
    let trigger = visual
        .debug_bounds("outer-popup-trigger")
        .expect("trigger remains rendered");
    visual.simulate_click(trigger.center(), Default::default());
    visual.update(|window, cx| window.draw(cx).clear(cx));
    assert_eq!(
        captures.get(),
        2,
        "closing the last popup releases its capture lease"
    );

    let current_image = test_image([0, 255, 0, 255]);
    pending.borrow_mut()[1]
        .take()
        .expect("reopened capture sender")
        .send(Some(current_image))
        .expect("reopened capture receiver remains live");
    visual.run_until_parked();
    let retained_current =
        visual.update(|window, cx| gpui_kit::component::popup_backdrop_for_test(window, cx));
    assert!(
        retained_current.is_some(),
        "the reopened popup retains its completed image"
    );

    let stale_image = test_image([255, 0, 255, 255]);
    pending.borrow_mut()[0]
        .take()
        .expect("original capture sender")
        .send(Some(stale_image))
        .expect("stale task still has its receiver");
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
    assert!(visual.debug_bounds("outer-popup-content").is_some());
    let retained_after_stale =
        visual.update(|window, cx| gpui_kit::component::popup_backdrop_for_test(window, cx));
    assert!(
        retained_current
            .as_ref()
            .zip(retained_after_stale.as_ref())
            .is_some_and(|(current, after)| Arc::ptr_eq(current, after)),
        "stale completion must not replace the reopened popup image"
    );
}

struct ControlledPopupHarness { open: Rc<Cell<bool>> }
impl Render for ControlledPopupHarness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(Popover::new("controlled-popup")
            .open(self.open.get())
            .trigger(Button::new("controlled-trigger").label("Model"))
            .content(|_, _, _| div().size(px(100.))))
    }
}
#[gpui_kit::test]
fn controlled_popup_close_releases_backdrop_and_reopen_acquires_fresh_capture(cx: &mut TestAppContext) {
    cx.update(|cx| gpui_kit::init(cx));
    let open = Rc::new(Cell::new(false));
    let captures = Rc::new(Cell::new(0));
    let factory: PopupBackdropFactory = { let captures = captures.clone(); Rc::new(move |_, cx| {
        captures.set(captures.get()+1);
        Some(cx.background_executor().spawn(async { Some(test_image([0,255,0,255])) }))
    }) };
    let (_, visual) = cx.add_window_view({ let open = open.clone(); move |window, cx| {
        let content = cx.new(|_| ControlledPopupHarness { open });
        gpui_kit::component::Root::new(content, window, cx)
    }});
    visual.update(|window,cx| { window.draw(cx).clear(cx); window.configure_popup_backdrop(cx,Some(factory)); });
    open.set(true);
    visual.update(|window,cx| { window.refresh(); window.draw(cx).clear(cx); });
    visual.run_until_parked();
    assert_eq!(captures.get(),1);
    assert!(visual.update(|window,cx| gpui_kit::component::popup_backdrop_for_test(window,cx)).is_some());
    open.set(false);
    visual.update(|window,cx| { window.refresh(); window.draw(cx).clear(cx); });
    assert!(visual.update(|window,cx| gpui_kit::component::popup_backdrop_for_test(window,cx)).is_none());
    open.set(true);
    visual.update(|window,cx| { window.refresh(); window.draw(cx).clear(cx); });
    assert_eq!(captures.get(),2);
}

#[gpui_kit::test]
fn popup_geometry_publishes_actual_positioned_bounds_survives_cache_and_clears_first_close(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let open = Rc::new(Cell::new(false));
    let (_, visual) = cx.add_window_view({
        let open = open.clone();
        move |window, cx| {
            let content = cx.new(|_| ControlledPopupHarness { open });
            gpui_kit::component::Root::new(content, window, cx)
        }
    });
    visual.update(|window, cx| window.draw(cx).clear(cx));
    open.set(true);
    visual.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear(cx);
        window.draw(cx).clear(cx);
        let bounds = gpui_kit::component::surface_backdrop::popup_bounds(window, cx);
        assert_eq!(bounds.len(), 1);
        assert!(bounds[0].size.width >= px(100.));
        assert!(bounds[0].size.height >= px(100.));
        assert!(bounds[0].origin.y > px(0.), "popup is positioned relative to its trigger");
        window.draw(cx).clear(cx);
        assert_eq!(gpui_kit::component::surface_backdrop::popup_bounds(window, cx), bounds);
    });
    open.set(false);
    visual.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear(cx);
        assert!(gpui_kit::component::surface_backdrop::popup_bounds(window, cx).is_empty());
    });
}
