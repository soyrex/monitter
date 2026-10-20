//! The real chat presents pending permissions above its complete pane surface.
use super::ChatPane;
use gpui_kit::{AppContext as _, TestAppContext, VisualTestContext, size};
use serde_json::json;
use std::rc::Rc;
fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
}
#[gpui_kit::test]
fn blocking_approval_centers_in_pane_and_can_be_reviewed_later(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::surface_blend::initialize_for_ui_fixture();
        crate::native_motion::initialize_for_ui_fixture(cx);
    });
    let snapshot = Rc::new(json!({
        "settings":{"theme":"dark","interfaceScale":100,"interfaceFontSize":14},
        "tasks":[{"id":"approval-fixture","agentId":"fixture-agent","title":"Approval fixture","provider":"codex","status":"running","queue":[]}],
        "agents":[{"id":"fixture-agent","name":"Fixture agent","provider":"codex"}],
        "messages":[],"events":[],"sharing":{},"hosts":[],"channels":[],
        "approvalRequests":[{"id":"pending-fixture","taskId":"approval-fixture","status":"pending","provider":"codex","tool":"Command execution","summary":"/bin/zsh -c 'echo fixture'","detail":"{\"command\":\"echo fixture\\necho second line\"}","risk":"high","rememberable":true}]
    }));
    let (pane, visual) = cx.add_window_view(move |window, cx| {
        ChatPane::performance_fixture("approval-fixture".into(), snapshot, window, cx)
    });
    for (width, height) in [(640., 800.), (400., 600.)] {
        visual.simulate_resize(size(gpui_kit::px(width), gpui_kit::px(height)));
        draw(visual);
        draw(visual);
        let overlay = visual
            .debug_bounds("pane-approval-overlay")
            .expect("automatic pane overlay");
        let modal = visual.debug_bounds("pane-approval-modal").expect("modal");
        assert!((overlay.size.width - gpui_kit::px(width)).abs() < gpui_kit::px(1.));
        assert!((overlay.size.height - gpui_kit::px(height)).abs() < gpui_kit::px(1.));
        assert!((modal.center().x - overlay.center().x).abs() < gpui_kit::px(1.));
        assert!((modal.center().y - overlay.center().y).abs() < gpui_kit::px(1.));
        assert!(modal.left() > overlay.left() && modal.right() < overlay.right());
        let actions = visual.debug_bounds("approval-dock-actions").unwrap();
        assert!(
            actions.bottom() <= modal.bottom(),
            "window {width}x{height}, overlay {overlay:?}, modal {modal:?}, actions {actions:?}, measured height {}",
            visual.read(|cx| pane.read(cx).pane_height_design)
        );
        visual.read(|cx| assert!(pane.read(cx).approval_popup_open));
    }
    visual.update(|_, cx| {
        pane.update(cx, |pane, cx| {
            pane.approval_popup_open = false;
            cx.notify();
        })
    });
    draw(visual);
    visual.read(|cx| {
        assert!(
            !pane.read(cx).approval_popup_open,
            "dismissal persists for the same pending request"
        );
        assert!(
            pane.read(cx).pending_tool_approval().is_some(),
            "dismissal never grants permission"
        );
    });
    visual.update(|window, cx| {
        pane.update(cx, |pane, cx| {
            pane.show_approval_prompt("pending-fixture".into(), window, cx)
        })
    });
    draw(visual);
    assert!(visual.debug_bounds("pane-approval-modal").is_some());
}
