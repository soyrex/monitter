//! Mounted activity headers keep their leading icon at the shared left edge.
use super::*;
use gpui_kit::{AppContext as _, TestAppContext, VisualTestContext, size};
use ::core::prelude::v1::test;

fn draw(visual: &mut VisualTestContext) {
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
}

#[gpui_kit::test]
fn activity_disclosure_caret_trails_summary_and_leading_icon_is_flush(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let mut mounted = None;
    let (_, visual) = cx.add_window_view(|window, cx| {
        let row = cx.new(|_| RunActivityRow::for_tool(
            DesktopService::disconnected_for_ui_fixture(), "fixture-task",
            vec![json!({"id":"alignment-tool","kind":"tool","title":"commandExecution","createdAt":1,"detail":"{\"type\":\"commandExecution\",\"command\":\"pwd\",\"status\":\"completed\"}"})], false, false));
        mounted = Some(row.clone());
        gpui_kit::component::Root::new(row, window, cx)
    });
    let row = mounted.unwrap();
    for width in [280., 600.] {
        visual.simulate_resize(size(px(width), px(400.)));
        draw(visual);
        for expanded in [false, true] {
            let bounds = visual.debug_bounds("run-group-alignment-tool").unwrap();
            let leading = visual.debug_bounds("run-leading-alignment-tool").unwrap();
            let caret = visual.debug_bounds("run-caret-alignment-tool").unwrap();
            assert!((leading.left() - bounds.left()).abs() <= px(0.5), "leading icon aligns with transcript edge");
            assert!(caret.left() > leading.right());
            assert!((caret.right() - bounds.right()).abs() <= px(0.5), "caret aligns with right edge");
            row.read_with(visual, |row, _| assert_eq!(row.expanded, expanded));
            visual.simulate_click(caret.center(), Default::default());
            draw(visual);
        }
    }
}

fn mcp_event(status: &str) -> Value {
    json!({"id":"retained-tool","taskId":"fixture-task","kind":"tool","title":"mcpToolCall","createdAt":1,
        "detail":json!({"id":"call-provider-1","type":"mcpToolCall","server":"monitter","tool":"build_request_update","status":status,"arguments":{"status":"ready"}}).to_string()})
}

#[test]
fn mcp_label_names_server_and_tool_and_uses_transport_status() {
    let running = mcp_event("inProgress");
    assert_eq!(tool_intent(&running), "Monitter · Build request update");
    assert_eq!(tool_state(&running), ToolState::Running);
    assert_eq!(tool_state(&mcp_event("completed")), ToolState::Completed);
    assert_eq!(tool_state(&mcp_event("failed")), ToolState::Failed);
    assert_eq!(tool_state(&mcp_event("cancelled")), ToolState::Failed);
    let mut errored = mcp_event("completed");
    let mut detail: Value = serde_json::from_str(errored["detail"].as_str().unwrap()).unwrap();
    detail["result"] = json!({"isError":true}); errored["detail"] = json!(detail.to_string());
    assert_eq!(tool_state(&errored), ToolState::Failed);
    assert_eq!(tool_state(&mcp_event("")), ToolState::Unknown, "argument status must never supply execution status");
}

#[gpui_kit::test]
fn tool_call_updates_spinner_to_tick_in_same_native_row(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let mut mounted = None;
    let (_, visual) = cx.add_window_view(|window, cx| {
        let row = cx.new(|cx| RunActivityRow::new(DesktopService::disconnected_for_ui_fixture(), "fixture-task", &mcp_event("inProgress"), window, cx));
        mounted = Some(row.clone());
        gpui_kit::component::Root::new(row, window, cx)
    });
    let row = mounted.unwrap();
    visual.simulate_resize(size(px(280.), px(400.)));
    draw(visual);
    assert!(visual.debug_bounds("run-state-retained-tool-running").is_some());
    assert!(visual.debug_bounds("run-state-retained-tool-completed").is_none());
    row.update(visual, |row,cx| row.set_event("fixture-task", &mcp_event("completed"),cx));
    draw(visual);
    assert!(visual.debug_bounds("run-state-retained-tool-running").is_none());
    assert!(visual.debug_bounds("run-state-retained-tool-completed").is_some());
    let bounds = visual.debug_bounds("run-single-retained-tool").unwrap();
    let caret = visual.debug_bounds("run-caret-retained-tool").unwrap();
    assert!((caret.right() - bounds.right()).abs() <= px(0.5));
}

#[gpui_kit::test]
fn activity_disclosure_flattens_only_a_duplicate_tool_branch(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    cx.update(|cx| {
        let events = (0..7).map(|i| json!({"id":format!("flat-{i}"),"kind":"tool","title":"commandExecution","detail":"{}"})).collect::<Vec<_>>();
        let mut row = RunActivityRow::for_tool(DesktopService::disconnected_for_ui_fixture(), "flat-task", events.clone(), false, false);
        row.process_branches = vec![ProcessBranch { mode:GroupMode::Tool, events:events.clone() }];
        assert!(row.repeats_only_tool_branch(), "seven identical command events need no repeated branch");
        row.process_approvals = vec![json!({"id":"approval"})];
        assert!(!row.repeats_only_tool_branch(), "approval history keeps semantic grouping");
        row.process_approvals.clear();
        row.process_branches[0].events.pop();
        assert!(!row.repeats_only_tool_branch(), "partial branch must not hide other events");
        row.process_branches = vec![ProcessBranch {mode:GroupMode::Reasoning,events:events.clone()}];
        assert!(!row.repeats_only_tool_branch(), "thought process remains distinct");
        row.process_branches.push(ProcessBranch {mode:GroupMode::Tool,events});
        assert!(!row.repeats_only_tool_branch(), "mixed trees retain real branches");
        let _ = cx;
    });
}
