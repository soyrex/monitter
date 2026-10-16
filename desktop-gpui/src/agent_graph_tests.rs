use super::*;
use ::core::prelude::v1::test;
use serde_json::json;

struct NarrowGraph {
    graph: Entity<AgentGraph>,
}
impl Render for NarrowGraph {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        // This fixture constrains the real viewport in logical screen pixels.
        // Design tokens use a 125% default scale, so ui_scaled_px(300.) would
        // create a 375-pixel pane while the assertions below require 300.
        div()
            .w(gpui_kit::px(300.))
            .h(gpui_kit::px(340.))
            .child(self.graph.clone())
    }
}

#[gpui_kit::test]
fn narrow_graph_keeps_slider_inside_pane_without_map_heading_or_workplan(cx: &mut TestAppContext) {
    cx.update(|cx| gpui_kit::init(cx));
    let mut graph = None;
    let (_, visual) = cx.add_window_view(|_, cx| {
        let entity = cx.new(AgentGraph::new);
        entity.update(cx, |view, cx| view.set_context(
            Rc::new(json!({"tasks":[{"id":"root","title":"Root","status":"running"}]})),
            "root", Arc::new(vec![json!({"id":"child","parentTaskId":"root","status":"running","agentName":"Worker"})]),
            design::palette(&Value::Null, WindowAppearance::Light), cx));
        graph = Some(entity.clone());
        NarrowGraph { graph: entity }
    });
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
    let slider = visual
        .debug_bounds("graph-zoom-slider")
        .expect("zoom slider is laid out");
    let scroll = visual
        .debug_bounds("agent-graph-scroll")
        .expect("diagram has scroll viewport");
    assert!(slider.right().as_f32() <= 300., "slider bounds: {slider:?}");
    assert!(
        scroll.size.width.as_f32() <= 300.,
        "scroll bounds: {scroll:?}"
    );
    assert!(visual.debug_bounds("graph-plan-scroll").is_none());
    assert!(visual.debug_bounds("graph-back").is_none());
}

#[::core::prelude::v1::test]
fn streaming_text_and_plan_updates_do_not_change_graph_input() {
    let mut snapshot = json!({"tasks":[{"id":"root","agentId":"agent","title":"Root","status":"running"}],"messages":[{"taskId":"root","text":"partial"}],"workPlans":[]});
    let before = graph_input(&snapshot, "root", &[]);
    snapshot["messages"][0]["text"] = json!("partial continued");
    snapshot["workPlans"] = json!([{"taskId":"root","title":"Plan"}]);
    assert_eq!(before, graph_input(&snapshot, "root", &[]));
    snapshot["tasks"][0]["status"] = json!("completed");
    assert_ne!(before, graph_input(&snapshot, "root", &[]));
}

#[::core::prelude::v1::test]
fn session_id_cannot_claim_an_unrelated_task_as_a_descendant() {
    let items = vec![
        json!({"id":"unrelated-task","parentTaskId":"root"}),
        json!({"id":"foreign","parentTaskId":"unrelated-task"}),
    ];
    let model = GraphModel::build(&Value::Null, "root", &items);
    assert_eq!(model.nodes.len(), 2);
}

#[gpui_kit::test]
fn zoom_scales_cards_and_keeps_controls_fixed(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let (graph, visual) = cx.add_window_view(|window, cx| {
        let mut graph = AgentGraph::new(cx);
        graph.set_context(
            Rc::new(json!({"tasks":[{"id":"root","title":"Root","status":"running"}]})),
            "root",
            Arc::new(vec![]),
            design::palette(&Value::Null, window.appearance()),
            cx,
        );
        graph
    });
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
    let before = visual.debug_bounds("graph-node-0").expect("main card");
    let controls_before = visual
        .debug_bounds("graph-zoom-slider")
        .expect("zoom slider");
    assert!(controls_before.bottom().as_f32() > 0.);
    assert!(visual.debug_bounds("graph-zoom-in").is_none());
    assert!(visual.debug_bounds("graph-zoom-out").is_none());
    let after = visual.debug_bounds("graph-node-0").expect("card");
    assert_eq!(after, before);
    assert_eq!(
        controls_before,
        visual.debug_bounds("graph-zoom-slider").unwrap()
    );
    let reset = visual.debug_bounds("graph-zoom-reset").expect("reset zoom");
    visual.simulate_click(reset.center(), Default::default());
    visual.run_until_parked();
    visual.update(|_, cx| assert_eq!(graph.read(cx).zoom, 1.));
}

#[gpui_kit::test]
fn completed_subagents_expand_from_a_shared_stack(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let (_, visual) = cx.add_window_view(|window, cx| {
        let mut graph = AgentGraph::new(cx);
        graph.set_context(Rc::new(json!({"tasks":[{"id":"root","title":"Root","status":"running"}]})), "root", Arc::new(vec![json!({"id":"done-a","taskId":"done-a","parentTaskId":"root","status":"completed","agentName":"Done A"}), json!({"id":"done-b","taskId":"done-b","parentTaskId":"root","status":"failed","agentName":"Done B"}), json!({"id":"live","taskId":"live","parentTaskId":"root","status":"running","agentName":"Live"})]), design::palette(&Value::Null, window.appearance()), cx);
        graph
    });
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
    assert!(visual.debug_bounds("graph-node-1").is_some());
    assert!(visual.debug_bounds("graph-node-2").is_none());
    let stacked = visual.debug_bounds("graph-node-1").unwrap();
    let live = visual.debug_bounds("graph-node-3").unwrap();
    assert!(live.origin.y < stacked.origin.y);
    visual.simulate_click(stacked.center(), Default::default());
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
    let first = visual.debug_bounds("graph-node-1").unwrap();
    let second = visual.debug_bounds("graph-node-2").unwrap();
    assert_ne!(first.origin, second.origin);
}
