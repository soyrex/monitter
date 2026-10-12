//! Real retained history reflows its visible rows without preparing history again.
use super::{ChatPane, transcript_preparation_counts};
use crate::design;
use ::core::prelude::v1::test;
use gpui_kit::{AppContext as _, FollowMode, ListOffset, TestAppContext, VisualTestContext, size};
use serde_json::json;
use std::rc::Rc;

fn draw(visual: &mut VisualTestContext) {
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
}

#[gpui_kit::test]
fn ten_thousand_rows_reflow_without_preparation_and_keep_the_scroll_anchor(
    cx: &mut TestAppContext,
) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::surface_blend::initialize_for_ui_fixture();
        crate::native_motion::initialize_for_ui_fixture(cx);
    });
    let snapshot = Rc::new(json!({
        "settings":{"theme":"dark", "interfaceScale":100, "interfaceFontSize":14},
        "tasks":[{"id":"reflow-task", "agentId":"fixture-agent", "title":"Reflow fixture",
            "provider":"codex", "status":"idle", "queue":[], "hostId":"fixture-host", "cwd":""}],
        "agents":[{"id":"fixture-agent", "name":"Fixture agent", "provider":"codex"}],
        "messages":(0..10_000).map(|index| json!({
            "id":if index == 9_002 { "width-probe".into() } else { format!("row-{index}") },
            "taskId":"reflow-task", "role":"user", "createdAt":index + 1,
            "text":if index == 9_002 {
                "Responsive markdown keeps its logical anchor while the pane changes width. ".repeat(6)
            } else { format!("Short history row {index}") },
        })).collect::<Vec<_>>(),
        "events":[], "approvalRequests":[], "sharing":{}, "hosts":[], "channels":[],
    }));
    let (pane, visual) = cx.add_window_view(move |window, cx| {
        ChatPane::performance_fixture("reflow-task".into(), snapshot, window, cx)
    });
    visual.simulate_resize(size(gpui_kit::px(600.), gpui_kit::px(1_100.)));
    draw(visual);
    draw(visual);
    visual.update(|_, cx| {
        pane.update(cx, |pane, cx| {
            let list = &pane.transcripts["reflow-task"].list;
            list.set_follow_mode(FollowMode::Normal);
            list.scroll_to(ListOffset {
                item_ix: 9_000,
                offset_in_item: gpui_kit::px(0.),
            });
            // A programmatic ListState scroll must invalidate its retained
            // entity, like the real scroll handler; notifying only the pane
            // intentionally leaves the clean cached transcript sibling intact.
            pane.invalidate_transcript_list(cx);
        });
    });
    draw(visual);
    let baseline = transcript_preparation_counts();
    let anchor = visual.read(|cx| {
        pane.read(cx).transcripts["reflow-task"]
            .list
            .logical_scroll_top()
            .item_ix
    });
    let original_height = visual
        .debug_bounds("message-body-width-probe")
        .expect("visible probe")
        .size
        .height;
    let mut prior_width = visual
        .debug_bounds("message-bubble-width-probe")
        .unwrap()
        .size
        .width;
    for width in [540., 470., 450., 400., 360.] {
        visual.simulate_resize(size(gpui_kit::px(width), gpui_kit::px(1_100.)));
        draw(visual);
        draw(visual);
        assert_eq!(
            transcript_preparation_counts(),
            baseline,
            "width-only reflow must not visit the 10,000 retained messages"
        );
        let bubble = visual
            .debug_bounds("message-bubble-width-probe")
            .expect("probe remains visible");
        visual.read(|cx| {
            let pane = pane.read(cx);
            assert!(pane.compact_detail);
            let geometry = pane.transcript_geometry.get();
            let expected = design::ui_scaled_px(geometry.attachment_width());
            assert!(
                (bubble.size.width - expected).as_f32().abs() < 1.5,
                "visible bubble must use current geometry, including the 420px threshold"
            );
            assert_eq!(
                pane.transcripts["reflow-task"]
                    .list
                    .logical_scroll_top()
                    .item_ix,
                anchor
            );
            assert!(!pane.transcripts["reflow-task"].list.is_following_tail());
        });
        assert_ne!(
            bubble.size.width, prior_width,
            "visible geometry must change during reflow"
        );
        prior_width = bubble.size.width;
    }
    let narrow_height = visual
        .debug_bounds("message-body-width-probe")
        .unwrap()
        .size
        .height;
    assert!(
        narrow_height > original_height,
        "Markdown must wrap at the new visible width"
    );
}
