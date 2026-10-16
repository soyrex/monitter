//! Exercise the public dialog/backdrop lifecycle against a real GPUI window.
use futures::channel::oneshot;
use gpui_kit::RenderImage;
use gpui_kit::component::{DialogBackdropFactory, WindowExt};
use gpui_kit::{
    AppContext as _, Context, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    Styled as _, TestAppContext, VisualTestContext, Window, div, px, rgb, size,
};
use image::{Frame, ImageBuffer, Rgba};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
};

const TEST_WIDTH: u32 = 800;
const TEST_HEIGHT: u32 = 600;

struct BackdropContent;

impl Render for BackdropContent {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("dialog-backdrop-test-background")
            .size_full()
            .bg(rgb(0x181818))
    }
}

type PendingCapture = Option<oneshot::Sender<Option<Arc<RenderImage>>>>;

fn test_image(color: [u8; 4]) -> Arc<RenderImage> {
    // Bound the fake scene to the test window's 800x600 viewport (< 2 MiB).
    Arc::new(RenderImage::new(vec![Frame::new(ImageBuffer::from_pixel(
        TEST_WIDTH,
        TEST_HEIGHT,
        Rgba(color),
    ))]))
}

fn open_dialog(window: &mut Window, cx: &mut gpui_kit::App, title: &'static str) {
    window.open_dialog(cx, move |dialog, _, _| dialog.title(title));
}

#[gpui_kit::test]
fn dialog_backdrop_is_shared_and_stale_capture_cannot_replace_reopened_modal(
    cx: &mut TestAppContext,
) {
    cx.update(|cx| gpui_kit::init(cx));
    let captures = Rc::new(Cell::new(0usize));
    let pending = Rc::new(RefCell::new(Vec::<PendingCapture>::new()));
    let captures_for_factory = captures.clone();
    let pending_for_factory = pending.clone();
    let factory: DialogBackdropFactory = Rc::new(move |_, cx| {
        captures_for_factory.set(captures_for_factory.get() + 1);
        let (sender, receiver) = oneshot::channel();
        pending_for_factory.borrow_mut().push(Some(sender));
        Some(
            cx.background_executor()
                .spawn(async move { receiver.await.ok().flatten() }),
        )
    });

    let (_, visual) = cx.add_window_view(|window, cx| {
        let content = cx.new(|_| BackdropContent);
        gpui_kit::component::Root::new(content, window, cx)
    });
    visual.simulate_resize(size(px(TEST_WIDTH as f32), px(TEST_HEIGHT as f32)));
    visual.run_until_parked();
    visual.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.configure_dialog_backdrop(cx, Some(factory), Some(0.25));
    });

    // The first modal starts one bounded capture; completion installs its
    // image on the backdrop before a nested dialog is added.
    visual.update(|window, cx| open_dialog(window, cx, "parent"));
    visual.run_until_parked();
    assert_eq!(captures.get(), 1);
    pending.borrow_mut()[0]
        .take()
        .expect("first capture sender")
        .send(Some(test_image([0, 255, 0, 255])))
        .expect("first capture receiver remains live");
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));

    visual.update(|window, cx| open_dialog(window, cx, "nested"));
    visual.run_until_parked();
    assert_eq!(captures.get(), 1, "nested dialogs share the first capture");
    assert!(visual.debug_bounds("dialog-layer").is_some());

    visual.update(|window, cx| window.close_dialog(cx));
    visual.run_until_parked();
    assert!(visual.update(|window, cx| window.has_active_dialog(cx)));
    assert_eq!(
        captures.get(),
        1,
        "closing one dialog retains the shared backdrop"
    );
    assert!(visual.debug_bounds("dialog-layer").is_some());

    visual.update(|window, cx| window.close_all_dialogs(cx));
    visual.run_until_parked();
    assert!(!visual.update(|window, cx| window.has_active_dialog(cx)));
    assert!(visual.debug_bounds("dialog-layer").is_none());

    // Leave capture 1 pending through a full close/reopen. Capture 2 completes
    // first with green; late capture 1 is magenta and must not replace it.
    visual.update(|window, cx| open_dialog(window, cx, "stale source"));
    visual.run_until_parked();
    assert_eq!(captures.get(), 2);
    visual.update(|window, cx| window.close_all_dialogs(cx));
    visual.update(|window, cx| open_dialog(window, cx, "reopened current"));
    visual.run_until_parked();
    assert_eq!(captures.get(), 3);

    let current_image = test_image([0, 255, 0, 255]);
    pending.borrow_mut()[2]
        .take()
        .expect("current capture sender")
        .send(Some(current_image.clone()))
        .expect("current capture receiver remains live");
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
    assert!(visual.update(|window, cx| window.has_active_dialog(cx)));
    let retained_current =
        visual.update(|window, cx| gpui_kit::component::modal_backdrop_for_test(window, cx));
    assert!(
        retained_current
            .as_ref()
            .is_some_and(|image| Arc::ptr_eq(image, &current_image)),
        "current generation should retain its completed image"
    );

    let stale_image = test_image([255, 0, 255, 255]);
    pending.borrow_mut()[1]
        .take()
        .expect("stale capture sender")
        .send(Some(stale_image))
        .expect("stale task still has its receiver");
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
    assert!(
        visual.update(|window, cx| window.has_active_dialog(cx)),
        "late capture completion must not disturb the reopened dialog"
    );
    let retained_after_stale =
        visual.update(|window, cx| gpui_kit::component::modal_backdrop_for_test(window, cx));
    assert!(
        retained_current
            .as_ref()
            .zip(retained_after_stale.as_ref())
            .is_some_and(|(current, after)| Arc::ptr_eq(current, after)),
        "late stale generation must not replace the current backdrop image"
    );
    assert!(visual.debug_bounds("dialog-layer").is_some());
}
