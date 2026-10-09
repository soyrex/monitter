//! Exercise the retained sidebar hover transition through real pointer enter
//! and exit events, including the full fade back to the idle state.
use gpui::{
    AppContext as _, Context, InteractiveElement as _, StatefulInteractiveElement as _, IntoElement, ParentElement as _,
    Render, Styled as _, TestAppContext, VisualTestContext, Window, div, point, px, rgb,
};
use std::{cell::Cell, rc::Rc, time::Duration};

struct ControlHoverHarness {
    samples: Rc<Cell<f32>>,
}

impl Render for ControlHoverHarness {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let hover = gpui_kit::component::hover_motion::HoverMotion::new("hover-fixture-row", true, window, cx);
        let color = hover.color("background", rgb(0x456789).opacity(0.).into(), rgb(0x456789).into(), window, cx);
        self.samples.set(color.a);

        div()
            .w(px(360.))
            .h(px(240.))
            .flex()
            .flex_col()
            .child(
                div()
                    .id("hover-fixture-row").debug_selector(|| "hover-fixture-row".into())
                    .w(px(280.))
                    .h(px(56.))
                    .bg(color)
                    .on_hover(cx.listener(move |_, hovered, _, cx| {
                        hover.on_hover(*hovered, cx);
                    })),
            )
            // A managed tooltip and the shared fade must coexist on one button.
            .child(gpui_kit::component::button::Button::new("hover-tooltip-fixture")
                .label("Tooltip control").tooltip("Tooltip remains available"))
            .child(div().id("hover-fixture-outside").debug_selector(|| "hover-fixture-outside".into()).w(px(280.)).h(px(120.)))
    }
}

fn fixture(cx: &mut TestAppContext) -> (Rc<Cell<f32>>, &mut VisualTestContext) {
    cx.update(|cx| gpui_kit::init(cx));
    let samples = Rc::new(Cell::new(0.));
    let (_, visual) = cx.add_window_view({
        let samples = samples.clone();
        move |_, _| ControlHoverHarness { samples }
    });
    visual.update(|window, _| window.activate_window());
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
    (samples, visual)
}

fn center(visual: &mut VisualTestContext, id: &'static str) -> gpui::Point<gpui::Pixels> {
    let bounds = visual.debug_bounds(id).expect("fixture item has layout bounds");
    point(
        bounds.left() + bounds.size.width / 2.,
        bounds.top() + bounds.size.height / 2.,
    )
}

fn frame(visual: &mut VisualTestContext, elapsed: Duration) {
    visual.executor().advance_clock(elapsed);
    visual.update(|window, cx| {
        window.simulate_next_frame(cx);
        window.draw(cx).clear(cx);
    });
}

#[gpui_kit::test]
fn shared_control_hover_fades_in_and_out(cx: &mut TestAppContext) {
    let (samples, visual) = fixture(cx);
    assert_eq!(samples.get(), 0.);

    let row_center = center(visual, "hover-fixture-row");
    visual.simulate_mouse_move(row_center, None, Default::default());
    visual.update(|window, cx| window.draw(cx).clear(cx));
    assert_eq!(samples.get(), 0., "enter starts from the idle opacity");
    frame(visual, Duration::from_millis(90));
    assert!(samples.get() > 0.8, "enter animation advances while hovered");
    frame(visual, Duration::from_millis(90));
    assert!((samples.get() - 1.).abs() < 0.01);

    let outside_center = center(visual, "hover-fixture-outside");
    visual.simulate_mouse_move(outside_center, None, Default::default());
    visual.update(|window, cx| window.draw(cx).clear(cx));
    assert!((samples.get() - 1.).abs() < 0.01, "exit begins from the visible state");
    frame(visual, Duration::from_millis(90));
    assert!(samples.get() > 0.05 && samples.get() < 0.2, "exit fades rather than sticking");
    frame(visual, Duration::from_millis(90));
    assert_eq!(samples.get(), 0., "after 180 ms the unhovered row is idle");
}

#[gpui_kit::test]
fn shared_control_hover_reverses_without_jump(cx: &mut TestAppContext) {
    let (samples, visual) = fixture(cx);
    let row = center(visual, "hover-fixture-row");
    visual.simulate_mouse_move(row, None, Default::default());
    visual.update(|window, cx| window.draw(cx).clear(cx));
    frame(visual, Duration::from_millis(45));
    let before = samples.get();
    assert!(before > 0. && before < 1.);
    let outside = center(visual, "hover-fixture-outside");
    visual.simulate_mouse_move(outside, None, Default::default());
    visual.update(|window, cx| window.draw(cx).clear(cx));
    assert!((samples.get() - before).abs() < 0.01);
    frame(visual, Duration::from_millis(180));
    assert_eq!(samples.get(), 0.);
}
#[gpui_kit::test]
fn shared_control_hover_respects_reduced_motion(cx: &mut TestAppContext) {
    let (samples, visual) = fixture(cx);
    visual.update(|_, cx| cx.set_reduce_motion(true));
    let row = center(visual, "hover-fixture-row");
    visual.simulate_mouse_move(row, None, Default::default());
    visual.update(|window, cx| window.draw(cx).clear(cx));
    assert_eq!(samples.get(), 1.);
    let outside = center(visual, "hover-fixture-outside");
    visual.simulate_mouse_move(outside, None, Default::default());
    visual.update(|window, cx| window.draw(cx).clear(cx));
    assert_eq!(samples.get(), 0.);
}
