//! Native layout regression fixture for the selected-child collaboration visor.
use super::{clipped_visor_body, visor_slide};
use gpui::{
    AppContext as _, Context, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, TestAppContext, VisualTestContext, Window, div, px, size,
};
use std::{cell::RefCell, time::Duration};

const WINDOW_WIDTH: f32 = 800.0;
const WINDOW_HEIGHT: f32 = 600.0;
const TAB_HEIGHT: f32 = 36.0;
const EXPANDED_HEIGHT: f32 = 240.0;
const TOLERANCE: f32 = 1.0;

fn expanded_physical_height() -> f32 {
    crate::design::ui_scaled_px(EXPANDED_HEIGHT).as_f32()
}

struct VisorFixture {
    open: bool,
    motion_allowed: bool,
    samples: RefCell<Vec<f32>>,
}

impl Render for VisorFixture {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let height = visor_slide(
            SharedString::from("visor-fixture-slide"),
            self.open,
            EXPANDED_HEIGHT,
            self.motion_allowed,
            window,
            cx,
        );
        self.samples.borrow_mut().push(height);

        // Match ChatPane's root flex column: the decorative layer belongs to
        // the upper workspace, so it shortens with content as the visor grows.
        let upper = div()
            .w_full()
            .flex_1()
            .min_h_0()
            .relative()
            .overflow_hidden()
            .debug_selector(|| "visor-upper-workspace".into())
            .child(
                div()
                    .absolute()
                    .size_full()
                    .debug_selector(|| "visor-sparkles".into()),
            );
        let tabs = div()
            .w_full()
            .h(px(TAB_HEIGHT))
            .flex_none()
            .debug_selector(|| "visor-tab-bar".into());
        let mut visor = div().w_full().flex_none().flex().flex_col().child(tabs);
        if height > 0.1 {
            let fixed_body = div()
                .absolute()
                .bottom_0()
                .left_0()
                .w_full()
                .h(crate::design::ui_scaled_px(EXPANDED_HEIGHT))
                .debug_selector(|| "visor-fixed-body".into());
            visor = visor.child(
                div()
                    .w_full()
                    .flex_none()
                    .debug_selector(|| "visor-clip-frame".into())
                    .child(clipped_visor_body(height, fixed_body)),
            );
        }

        div()
            .w(px(WINDOW_WIDTH))
            .h(px(WINDOW_HEIGHT))
            .flex()
            .flex_col()
            .child(upper)
            .child(visor)
    }
}

fn draw(visual: &mut VisualTestContext) {
    visual.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear(cx);
    });
}

fn frame(visual: &mut VisualTestContext, duration: Duration) {
    visual.executor().advance_clock(duration);
    visual.update(|window, cx| {
        window.simulate_next_frame(cx);
        window.draw(cx).clear(cx);
    });
}

fn height(visual: &mut VisualTestContext, selector: &'static str) -> f32 {
    visual
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("missing bounds for {selector}"))
        .size
        .height
        .as_f32()
}

fn sample(visual: &mut VisualTestContext, fixture: &gpui::Entity<VisorFixture>) -> f32 {
    visual.update(|_, cx| *fixture.read(cx).samples.borrow().last().unwrap())
}

fn set_open(visual: &mut VisualTestContext, fixture: &gpui::Entity<VisorFixture>, open: bool) {
    visual.update(|_, cx| {
        fixture.update(cx, |fixture, cx| {
            fixture.open = open;
            cx.notify();
        });
    });
    draw(visual);
}

fn set_state(
    visual: &mut VisualTestContext,
    fixture: &gpui::Entity<VisorFixture>,
    open: bool,
    motion_allowed: bool,
) {
    visual.update(|_, cx| {
        fixture.update(cx, |fixture, cx| {
            fixture.open = open;
            fixture.motion_allowed = motion_allowed;
            cx.notify();
        });
    });
    draw(visual);
}

#[gpui_kit::test]
fn visor_expands_closes_and_reverses_without_jumping(cx: &mut TestAppContext) {
    cx.update(|cx| gpui_kit::init(cx));
    let (fixture, mut visual) = cx.add_window_view(|_, _| VisorFixture {
        open: false,
        motion_allowed: true,
        samples: RefCell::new(Vec::new()),
    });
    visual.simulate_resize(size(px(WINDOW_WIDTH), px(WINDOW_HEIGHT)));
    draw(visual);

    assert!(visual.debug_bounds("visor-fixed-body").is_none());
    let upper_closed_bounds = visual.debug_bounds("visor-upper-workspace").unwrap();
    let closed_upper = upper_closed_bounds.size.height.as_f32();
    let closed_sparkles = height(visual, "visor-sparkles");
    let tab_closed = visual.debug_bounds("visor-tab-bar").unwrap();
    assert!((tab_closed.size.width.as_f32() - WINDOW_WIDTH).abs() <= TOLERANCE);
    assert!((tab_closed.size.height.as_f32() - TAB_HEIGHT).abs() <= TOLERANCE);
    assert!((tab_closed.top().as_f32() - upper_closed_bounds.bottom().as_f32()).abs() <= TOLERANCE);
    assert!((closed_upper - (WINDOW_HEIGHT - TAB_HEIGHT)).abs() <= TOLERANCE);
    assert!((closed_sparkles - closed_upper).abs() <= TOLERANCE);

    set_open(visual, &fixture, true);
    let initial_open = sample(visual, &fixture);
    let mut opening = vec![initial_open];
    for _ in 0..5 {
        frame(visual, Duration::from_millis(50));
        opening.push(sample(visual, &fixture));
    }
    assert!(
        opening
            .windows(2)
            .all(|values| values[1] + TOLERANCE >= values[0])
    );
    assert!((opening.last().copied().unwrap() - EXPANDED_HEIGHT).abs() <= TOLERANCE);

    let upper_open_bounds = visual.debug_bounds("visor-upper-workspace").unwrap();
    let open_upper = upper_open_bounds.size.height.as_f32();
    let open_sparkles = height(visual, "visor-sparkles");
    let tab_open = visual.debug_bounds("visor-tab-bar").unwrap();
    let clip_open = visual.debug_bounds("visor-clip-frame").unwrap();
    let body_open = visual.debug_bounds("visor-fixed-body").unwrap();
    assert!((tab_open.size.width.as_f32() - WINDOW_WIDTH).abs() <= TOLERANCE);
    assert!((tab_open.size.height.as_f32() - TAB_HEIGHT).abs() <= TOLERANCE);
    assert!((tab_open.top().as_f32() - upper_open_bounds.bottom().as_f32()).abs() <= TOLERANCE);
    assert!((clip_open.top().as_f32() - tab_open.bottom().as_f32()).abs() <= TOLERANCE);
    assert!((clip_open.size.width.as_f32() - WINDOW_WIDTH).abs() <= TOLERANCE);
    assert!((clip_open.size.height.as_f32() - expanded_physical_height()).abs() <= TOLERANCE);
    assert!((open_upper - (closed_upper - expanded_physical_height())).abs() <= TOLERANCE);
    assert!((open_sparkles - open_upper).abs() <= TOLERANCE);
    assert!(open_upper < closed_upper);
    assert!((body_open.size.width.as_f32() - WINDOW_WIDTH).abs() <= TOLERANCE);
    assert!((body_open.bottom().as_f32() - clip_open.bottom().as_f32()).abs() <= TOLERANCE);

    set_open(visual, &fixture, false);
    let close_start = sample(visual, &fixture);
    assert!((close_start - EXPANDED_HEIGHT).abs() <= TOLERANCE);
    assert!(visual.debug_bounds("visor-fixed-body").is_some());
    frame(visual, Duration::from_millis(75));
    let closing = sample(visual, &fixture);
    assert!(closing > 0.1 && closing < close_start);
    let clip_closing = visual.debug_bounds("visor-clip-frame").unwrap();
    let body_closing = visual.debug_bounds("visor-fixed-body").unwrap();
    assert!((body_closing.bottom().as_f32() - clip_closing.bottom().as_f32()).abs() <= TOLERANCE);
    assert!(height(visual, "visor-upper-workspace") > open_upper);

    set_open(visual, &fixture, true);
    let reverse_start = sample(visual, &fixture);
    assert!((reverse_start - closing).abs() <= TOLERANCE);
    frame(visual, Duration::from_millis(50));
    let reversing = sample(visual, &fixture);
    assert!(reversing > reverse_start);
    frame(visual, Duration::from_millis(300));
    let reopened = sample(visual, &fixture);
    assert!((reopened - EXPANDED_HEIGHT).abs() <= TOLERANCE);
    set_open(visual, &fixture, false);
    frame(visual, Duration::from_millis(300));
    assert!(visual.debug_bounds("visor-fixed-body").is_none());
    assert!((height(visual, "visor-upper-workspace") - closed_upper).abs() <= TOLERANCE);
}

#[gpui_kit::test]
fn visor_reduced_motion_changes_height_immediately(cx: &mut TestAppContext) {
    cx.update(|cx| gpui_kit::init(cx));
    let (fixture, mut visual) = cx.add_window_view(|_, _| VisorFixture {
        open: true,
        motion_allowed: true,
        samples: RefCell::new(Vec::new()),
    });
    visual.simulate_resize(size(px(WINDOW_WIDTH), px(WINDOW_HEIGHT)));
    draw(visual);
    let initial_auto_open = sample(visual, &fixture);
    assert!(initial_auto_open.abs() <= TOLERANCE);
    assert!(visual.debug_bounds("visor-fixed-body").is_none());
    frame(visual, Duration::from_millis(100));
    let auto_open_partial = sample(visual, &fixture);
    assert!(auto_open_partial > 0.1 && auto_open_partial < EXPANDED_HEIGHT);
    frame(visual, Duration::from_millis(200));
    let auto_open_full = sample(visual, &fixture);
    assert!((auto_open_full - EXPANDED_HEIGHT).abs() <= TOLERANCE);

    set_state(visual, &fixture, false, false);
    let closed = sample(visual, &fixture);
    assert!(closed.abs() <= TOLERANCE);
    assert!(visual.debug_bounds("visor-fixed-body").is_none());
    set_state(visual, &fixture, true, false);
    let opened = sample(visual, &fixture);
    assert!((opened - EXPANDED_HEIGHT).abs() <= TOLERANCE);
    assert!(visual.debug_bounds("visor-fixed-body").is_some());
}
