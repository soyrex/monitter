//! Native GUI regression coverage for assistant response metadata disclosure.
use gpui::{
    AppContext as _, Context, InteractiveElement as _, IntoElement, Modifiers, MouseButton,
    ParentElement as _, Render, Styled as _, TestAppContext, VisualTestContext, Window, div, point,
    px,
};
use serde_json::{Value, json};

struct ResponseMetadataHarness {
    expanded: bool,
    metadata: Value,
}

impl Render for ResponseMetadataHarness {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let weak = cx.entity().downgrade();
        let settings = json!({});
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(8.))
            // Model an assistant message bubble followed by its tertiary footer.
            .child(
                div()
                    .id("response-fixture-bubble")
                    .debug_selector(|| "response-fixture-bubble".into())
                    .w(px(360.))
                    .h(px(72.))
                    .p(px(8.))
                    .child("Assistant response body"),
            )
            .child(
                div()
                    .id("response-fixture-footer")
                    .debug_selector(|| "response-fixture-footer".into())
                    .w(px(360.))
                    .child(crate::response_metadata::render(
                        &self.metadata,
                        "fixture-message",
                        &settings,
                        window.appearance(),
                        self.expanded,
                        move |_, cx| {
                            let _ = weak.update(cx, |harness, cx| {
                                harness.expanded = !harness.expanded;
                                cx.notify();
                            });
                        },
                    )),
            )
    }
}

fn fixture(cx: &mut TestAppContext) -> (gpui::Entity<ResponseMetadataHarness>, &mut VisualTestContext) {
    cx.update(|cx| gpui_kit::init(cx));
    let (harness, visual) = cx.add_window_view(|_, _| ResponseMetadataHarness {
        expanded: false,
        metadata: json!({
            "model": "MiniMax-M3",
            "inputTokens": 1200,
            "outputTokens": 340,
            "requestedModel": "MiniMax-M3",
            "requestedEffort": "high",
            "confidence": 0.91,
            "routeApplied": true,
            "jevRationale": "The response metadata details are available.",
        }),
    });
    visual.update(|window, _| window.activate_window());
    redraw(visual);
    (harness, visual)
}

fn center(visual: &mut VisualTestContext, id: &'static str) -> gpui::Point<gpui::Pixels> {
    let bounds = visual.debug_bounds(id).expect("fixture element should be laid out");
    point(
        bounds.left() + bounds.size.width / 2.,
        bounds.top() + bounds.size.height / 2.,
    )
}

fn redraw(visual: &mut VisualTestContext) {
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
}

fn click(visual: &mut VisualTestContext, position: gpui::Point<gpui::Pixels>) {
    visual.simulate_mouse_down(position, MouseButton::Left, Modifiers::default());
    visual.simulate_mouse_up(position, MouseButton::Left, Modifiers::default());
    redraw(visual);
}

#[gpui_kit::test]
fn response_metadata_info_and_chevron_toggle_persist_across_redraws(cx: &mut TestAppContext) {
    let (harness, visual) = fixture(cx);
    let toggle_id = "response-metadata-toggle-fixture-message";
    assert!(visual.debug_bounds("response-details-fixture-message").is_none());

    visual
        .debug_bounds(toggle_id)
        .expect("metadata toggle should be laid out");
    let info_target = center(visual, "response-metadata-info-fixture-message");
    click(visual, info_target);
    assert!(
        visual.debug_bounds("response-details-fixture-message").is_some(),
        "clicking the info/model side should open details and survive redraw",
    );
    assert!(harness.read_with(visual, |state, _| state.expanded));

    visual
        .debug_bounds(toggle_id)
        .expect("metadata toggle should remain laid out while expanded");
    let chevron_target = center(visual, "response-metadata-chevron-fixture-message");
    click(visual, chevron_target);
    assert!(
        visual.debug_bounds("response-details-fixture-message").is_none(),
        "clicking the chevron side should close details",
    );
    assert!(!harness.read_with(visual, |state, _| state.expanded));
}

#[gpui_kit::test]
fn response_metadata_footer_sits_outside_the_assistant_bubble(cx: &mut TestAppContext) {
    let (_, visual) = fixture(cx);
    let bubble = visual
        .debug_bounds("response-fixture-bubble")
        .expect("assistant bubble should be laid out");
    let footer = visual
        .debug_bounds("response-fixture-footer")
        .expect("response metadata footer should be laid out");
    assert!(
        footer.top() >= bubble.bottom(),
        "tertiary metadata footer must follow the response bubble",
    );
}
