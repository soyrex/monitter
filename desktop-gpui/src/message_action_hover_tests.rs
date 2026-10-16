//! Message action visibility is scoped to its own hovered bubble, never focus
//! or a sibling message. This mirrors the production `group` / `group_hover`
//! strip geometry without constructing a task service or invoking actions.
use gpui::{
    AppContext as _, Context, FocusHandle, InteractiveElement as _, IntoElement,
    Modifiers, ParentElement as _, Render, SharedString, Styled as _, TestAppContext,
    VisualTestContext, Window, div, point, px,
};

struct MessageActionHoverHarness {
    focus: FocusHandle,
}

fn message_row(id: &'static str) -> impl IntoElement {
    let group: SharedString = format!("message-actions-{id}").into();
    let strip = div()
        .id(SharedString::from(format!("message-action-strip-{id}")))
        .debug_selector(move || format!("message-action-strip-{id}").into())
        .flex()
        .flex_shrink_0()
        .items_center()
        .gap(px(2.))
        .max_w(px(0.))
        .opacity(0.)
        .overflow_hidden()
        .group_hover(group.clone(), |style| {
            style.max_w(px(76.)).opacity(1.)
        })
        .child(div().w(px(76.)).h(px(23.)).child("actions"));

    div()
        .id(SharedString::from(format!("message-body-{id}")))
        .debug_selector(move || format!("message-body-{id}").into())
        .group(group)
        .w(px(420.))
        .h(px(64.))
        .flex()
        .items_start()
        .child(strip)
        .child(div().ml(px(8.)).child(format!("message {id}")))
}

impl Render for MessageActionHoverHarness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("message-action-pane")
            .size_full()
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(message_row("first"))
            .child(message_row("second"))
            .child(div().id("blank-pane-area").debug_selector(|| "blank-pane-area".into()).w(px(420.)).h(px(240.)))
    }
}

fn fixture(cx: &mut TestAppContext) -> (gpui::Entity<MessageActionHoverHarness>, &mut VisualTestContext) {
    cx.update(|cx| gpui_kit::init(cx));
    let (harness, visual) = cx.add_window_view(|_, cx| MessageActionHoverHarness {
        focus: cx.focus_handle(),
    });
    visual.update(|window, _| window.activate_window());
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
    (harness, visual)
}

fn element_center(cx: &mut VisualTestContext, id: &'static str) -> gpui::Point<gpui::Pixels> {
    let bounds = cx.debug_bounds(id).expect("fixture element should be laid out");
    point(
        bounds.left() + bounds.size.width / 2.,
        bounds.top() + bounds.size.height / 2.,
    )
}

fn strip_width(cx: &mut VisualTestContext, id: &'static str) -> f32 {
    cx.debug_bounds(id)
        .expect("action strip should remain mounted while collapsed")
        .size
        .width
        .as_f32()
}

fn redraw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, app| window.draw(app).clear(app));
}

#[gpui_kit::test]
fn message_actions_expand_only_for_their_own_hover_and_not_focus(cx: &mut TestAppContext) {
    let (harness, visual) = fixture(cx);
    let first_strip = "message-action-strip-first";
    let second_strip = "message-action-strip-second";
    assert_eq!(strip_width(visual, first_strip).round(), 0.);
    assert_eq!(strip_width(visual, second_strip).round(), 0.);

    let first = element_center(visual, "message-body-first");
    visual.simulate_mouse_move(
        first,
        None,
        Modifiers::default(),
    );
    redraw(visual);
    assert_eq!(strip_width(visual, first_strip).round(), 76.);
    assert_eq!(strip_width(visual, second_strip).round(), 0.);

    let blank = element_center(visual, "blank-pane-area");
    visual.simulate_mouse_move(
        blank,
        None,
        Modifiers::default(),
    );
    redraw(visual);
    assert_eq!(strip_width(visual, first_strip).round(), 0.);
    assert_eq!(strip_width(visual, second_strip).round(), 0.);

    let focus = harness.read_with(visual, |harness, _| harness.focus.clone());
    visual.update(|window, app| focus.focus(window, app));
    redraw(visual);
    assert_eq!(strip_width(visual, first_strip).round(), 0.);
    assert_eq!(strip_width(visual, second_strip).round(), 0.);

    let second = element_center(visual, "message-body-second");
    visual.simulate_mouse_move(
        second,
        None,
        Modifiers::default(),
    );
    redraw(visual);
    assert_eq!(strip_width(visual, first_strip).round(), 0.);
    assert_eq!(strip_width(visual, second_strip).round(), 76.);
}
