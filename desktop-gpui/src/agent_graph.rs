//! Retained, read-only graph of recorded task delegation and communication.
use crate::design::ui_scaled_px as px;
use crate::design::{self, Palette};
use gpui_kit::{
    assets::IconName,
    component::{Icon, Sizable as _},
    prelude::*,
    *,
};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
    sync::Arc,
};

pub enum AgentGraphEvent {
    BackToChat,
}
#[derive(Clone, Debug)]
struct Node {
    name: String,
    title: String,
    status: String,
    model: String,
    parent: Option<usize>,
    depth: usize,
    x: f32,
    y: f32,
}
#[derive(Clone, Default, Debug)]
struct GraphModel {
    nodes: Vec<Node>,
    messages: Vec<(usize, usize)>,
    width: f32,
    height: f32,
}
fn field(v: &Value, key: &str) -> String {
    v[key].as_str().unwrap_or_default().to_owned()
}
fn rows<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v[key].as_array().map(Vec::as_slice).unwrap_or(&[])
}
const CARD_WIDTH: f32 = 238.;
const CARD_HEIGHT: f32 = 112.;
const COLUMN_STEP: f32 = 270.;
const LEVEL_STEP: f32 = 176.;
const GRAPH_PADDING: f32 = 24.;
fn clamped_zoom(zoom: f32) -> f32 {
    if zoom.is_finite() {
        zoom.clamp(0.4, 2.)
    } else {
        1.
    }
}
fn active(status: &str) -> bool {
    matches!(status, "running" | "queued")
}
fn finished(status: &str) -> bool {
    !status.is_empty() && !active(status)
}
fn layout_nodes(model: &mut GraphModel, expanded: bool) {
    let mut counts = HashMap::<usize, usize>::new();
    for (index, node) in model.nodes.iter().enumerate() {
        if index == 0 || active(&node.status) {
            *counts.entry(node.depth).or_default() += 1;
        }
    }
    let widest = counts.values().copied().max().unwrap_or(1);
    let mut columns = HashMap::<usize, usize>::new();
    for (index, node) in model.nodes.iter_mut().enumerate() {
        if index == 0 || active(&node.status) {
            let column = columns.entry(node.depth).or_default();
            node.x = GRAPH_PADDING
                + (widest - counts[&node.depth]) as f32 * COLUMN_STEP / 2.
                + *column as f32 * COLUMN_STEP;
            node.y = GRAPH_PADDING + node.depth as f32 * LEVEL_STEP;
            *column += 1;
        }
    }
    let done = model
        .nodes
        .iter()
        .enumerate()
        .filter(|(i, node)| *i > 0 && finished(&node.status))
        .map(|(i, _)| i)
        .collect::<Vec<_>>();
    let base_y = model
        .nodes
        .iter()
        .enumerate()
        .filter(|(i, node)| *i == 0 || active(&node.status))
        .map(|(_, node)| node.y + CARD_HEIGHT)
        .fold(GRAPH_PADDING, f32::max)
        + 28.;
    let columns = if expanded { 3 } else { 1 };
    for (slot, index) in done.iter().enumerate() {
        model.nodes[*index].x = GRAPH_PADDING + (slot % columns) as f32 * COLUMN_STEP;
        model.nodes[*index].y = base_y + (slot / columns) as f32 * (CARD_HEIGHT + 24.);
    }
    let first_done = done.first().copied();
    model.width = model
        .nodes
        .iter()
        .enumerate()
        .filter(|(i, node)| *i == 0 || active(&node.status) || expanded || Some(*i) == first_done)
        .map(|(_, node)| node.x + CARD_WIDTH)
        .fold(0., f32::max)
        + GRAPH_PADDING * 2.
        + model.messages.len() as f32 * 8.;
    model.height = model
        .nodes
        .iter()
        .enumerate()
        .filter(|(i, node)| *i == 0 || active(&node.status) || expanded || Some(*i) == first_done)
        .map(|(_, node)| node.y + CARD_HEIGHT)
        .fold(0., f32::max)
        + GRAPH_PADDING;
}

/// Project only graph facts: streaming message bodies and timestamps do not
/// invalidate the retained geometry, and plan updates stay independent.
fn graph_input(snapshot: &Value, task_id: &str, items: &[Value]) -> Value {
    let project = |value: &Value, keys: &[&str]| {
        Value::Object(
            keys.iter()
                .map(|key| ((*key).to_owned(), value[*key].clone()))
                .collect(),
        )
    };
    let tasks = rows(snapshot, "tasks")
        .iter()
        .filter(|v| field(v, "id") == task_id)
        .map(|v| project(v, &["id", "agentId", "title", "status", "model"]))
        .collect::<Vec<_>>();
    let agent_id = tasks
        .first()
        .map(|v| field(v, "agentId"))
        .unwrap_or_default();
    let agents = rows(snapshot, "agents")
        .iter()
        .filter(|v| field(v, "id") == agent_id)
        .map(|v| project(v, &["id", "name"]))
        .collect::<Vec<_>>();
    let graph_items = items
        .iter()
        .map(|v| {
            project(
                v,
                &[
                    "taskId",
                    "parentTaskId",
                    "agentThreadId",
                    "parentThreadId",
                    "agentPath",
                    "agentName",
                    "title",
                    "status",
                    "model",
                ],
            )
        })
        .collect::<Vec<_>>();
    let scope = std::iter::once(task_id.to_owned())
        .chain(
            items
                .iter()
                .map(|v| field(v, "taskId"))
                .filter(|id| !id.is_empty()),
        )
        .collect::<HashSet<_>>();
    let collaborations = rows(snapshot, "collaborations")
        .iter()
        .filter(|v| {
            field(v, "kind") == "message"
                && scope.contains(&field(v, "fromTaskId"))
                && scope.contains(&field(v, "toTaskId"))
        })
        .map(|v| project(v, &["kind", "fromTaskId", "toTaskId"]))
        .collect::<Vec<_>>();
    let events = rows(snapshot, "events")
        .iter()
        .filter(|v| field(v, "kind") == "subagent" && scope.contains(&field(v, "taskId")))
        .map(|v| project(v, &["kind", "taskId", "detail"]))
        .collect::<Vec<_>>();
    serde_json::json!({"tasks":tasks,"agents":agents,"graphItems":graph_items,"collaborations":collaborations,"events":events})
}

impl GraphModel {
    fn build(snapshot: &Value, task_id: &str, items: &[Value]) -> Self {
        let task = rows(snapshot, "tasks")
            .iter()
            .find(|v| field(v, "id") == task_id)
            .unwrap_or(&Value::Null);
        let agent = rows(snapshot, "agents")
            .iter()
            .find(|v| field(v, "id") == field(task, "agentId"))
            .unwrap_or(&Value::Null);
        let mut model = Self::default();
        model.nodes.push(Node {
            name: if field(agent, "name").is_empty() {
                "Main agent".into()
            } else {
                field(agent, "name")
            },
            title: field(task, "title"),
            status: field(task, "status"),
            model: field(task, "model"),
            parent: None,
            depth: 0,
            x: 24.,
            y: 24.,
        });
        let mut tasks = HashMap::from([(task_id.to_owned(), 0)]);
        let mut threads = HashMap::new();
        let mut paths = HashMap::new();
        let mut included = Vec::new();
        // Accept only descendants of this task, even if a caller passes broader records.
        let mut pending: Vec<&Value> = items.iter().collect();
        loop {
            let before = pending.len();
            pending.retain(|item| {
                let parent = field(item, "parentTaskId");
                if !tasks.contains_key(&parent) {
                    return true;
                }
                let index = model.nodes.len();
                model.nodes.push(Node {
                    name: field(item, "agentName"),
                    title: field(item, "title"),
                    status: field(item, "status"),
                    model: field(item, "model"),
                    parent: tasks.get(&parent).copied(),
                    depth: 1,
                    x: 0.,
                    y: 0.,
                });
                let id = field(item, "taskId");
                if !id.is_empty() && id != task_id {
                    tasks.entry(id).or_insert(index);
                }
                let thread = field(item, "agentThreadId");
                if !thread.is_empty() {
                    threads.insert(thread, index);
                }
                let path = field(item, "agentPath");
                if !path.is_empty() {
                    paths.insert(path, index);
                }
                included.push(*item);
                false
            });
            if pending.is_empty() || pending.len() == before {
                break;
            }
        }
        for (offset, item) in included.iter().enumerate() {
            let index = offset + 1;
            let thread = field(item, "parentThreadId");
            let path = field(item, "agentPath");
            let path_parent = path
                .rsplit_once('/')
                .and_then(|(parent, _)| paths.get(parent))
                .copied();
            let parent = threads
                .get(&thread)
                .copied()
                .or(path_parent)
                .or(model.nodes[index].parent);
            if let Some(parent) = parent.filter(|p| *p != index) {
                model.nodes[index].parent = Some(parent);
            }
        }
        for index in 1..model.nodes.len() {
            let mut cursor = model.nodes[index].parent;
            let mut seen = HashSet::from([index]);
            let mut depth = 0;
            while let Some(parent) = cursor {
                if !seen.insert(parent) {
                    break;
                }
                depth += 1;
                cursor = model.nodes[parent].parent;
            }
            model.nodes[index].depth = depth.max(1);
        }
        // Each depth is a horizontal rank: parent-to-child flow runs downward.
        // Center ranks against the widest one so the main agent anchors the map.
        let mut counts = HashMap::<usize, usize>::new();
        for node in &model.nodes {
            *counts.entry(node.depth).or_default() += 1;
        }
        let widest = counts.values().copied().max().unwrap_or(1);
        let mut columns = HashMap::<usize, usize>::new();
        for node in &mut model.nodes {
            let column = columns.entry(node.depth).or_default();
            node.x = GRAPH_PADDING
                + (widest - counts[&node.depth]) as f32 * COLUMN_STEP / 2.
                + *column as f32 * COLUMN_STEP;
            node.y = GRAPH_PADDING + node.depth as f32 * LEVEL_STEP;
            *column += 1;
        }
        let mut edges = HashSet::new();
        let mut add_edge = |from: Option<usize>, to: Option<usize>| {
            if let (Some(a), Some(b)) = (from, to) {
                if a != b && active(&model.nodes[a].status) && active(&model.nodes[b].status) {
                    edges.insert((a, b));
                }
            }
        };
        for record in rows(snapshot, "collaborations")
            .iter()
            .filter(|v| field(v, "kind") == "message")
        {
            add_edge(
                tasks.get(&field(record, "fromTaskId")).copied(),
                tasks.get(&field(record, "toTaskId")).copied(),
            );
        }
        // Only explicit Codex sendInput records have proven sender/receiver identity.
        for event in rows(snapshot, "events")
            .iter()
            .filter(|v| field(v, "kind") == "subagent" && tasks.contains_key(&field(v, "taskId")))
        {
            let detail: Value =
                serde_json::from_str(&field(event, "detail")).unwrap_or(Value::Null);
            let activity = detail.get("activity").unwrap_or(&detail);
            let item = activity.get("item").unwrap_or(activity);
            if field(item, "type") == "collabAgentToolCall" && field(item, "tool") == "sendInput" {
                let from = threads.get(&field(item, "senderThreadId")).copied();
                for receiver in rows(item, "receiverThreadIds") {
                    add_edge(
                        from,
                        receiver.as_str().and_then(|id| threads.get(id)).copied(),
                    );
                }
            }
        }
        model.messages = edges.into_iter().collect();
        model.messages.sort_unstable();
        model.width = model
            .nodes
            .iter()
            .map(|n| n.x + CARD_WIDTH)
            .fold(0., f32::max)
            + GRAPH_PADDING * 2.
            + model.messages.len() as f32 * 8.;
        model.height = model
            .nodes
            .iter()
            .map(|n| n.y + CARD_HEIGHT)
            .fold(0., f32::max)
            + GRAPH_PADDING;
        model
    }
}

pub struct AgentGraph {
    snapshot: Rc<Value>,
    task_id: String,
    items: Arc<Vec<Value>>,
    palette: Palette,
    model: GraphModel,
    graph_input: Value,
    completed_expanded: bool,
    zoom: f32,
    zoom_slider: Entity<gpui_kit::component::slider::SliderState>,
    _zoom_subscription: Subscription,
    graph_scroll: ScrollHandle,
}
impl EventEmitter<AgentGraphEvent> for AgentGraph {}
impl AgentGraph {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let zoom_slider = cx.new(|_| {
            gpui_kit::component::slider::SliderState::new()
                .min(40.)
                .max(200.)
                .step(1.)
                .default_value(100.)
        });
        let zoom_subscription = cx.subscribe(
            &zoom_slider,
            |this, _, event: &gpui_kit::component::slider::SliderEvent, cx| {
                if let gpui_kit::component::slider::SliderEvent::Change(
                    gpui_kit::component::slider::SliderValue::Single(value),
                ) = event
                {
                    this.zoom = clamped_zoom(*value / 100.);
                    cx.notify();
                }
            },
        );
        Self {
            snapshot: Rc::new(Value::Null),
            task_id: String::new(),
            items: Arc::new(Vec::new()),
            palette: design::palette(&Value::Null, WindowAppearance::Light),
            model: GraphModel::default(),
            graph_input: Value::Null,
            completed_expanded: false,
            zoom: 1.,
            zoom_slider,
            _zoom_subscription: zoom_subscription,
            graph_scroll: ScrollHandle::new(),
        }
    }
    pub fn set_context(
        &mut self,
        snapshot: Rc<Value>,
        task_id: &str,
        items: Arc<Vec<Value>>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) {
        let changed = !Rc::ptr_eq(&self.snapshot, &snapshot)
            || self.task_id != task_id
            || !Arc::ptr_eq(&self.items, &items);
        if !changed && self.palette == palette {
            return;
        }
        let graph_input = graph_input(&snapshot, task_id, &items);
        let graph_changed = self.graph_input != graph_input || self.task_id != task_id;
        if graph_changed {
            if self.task_id != task_id {
                self.completed_expanded = false;
            }
            self.model = GraphModel::build(&graph_input, task_id, rows(&graph_input, "graphItems"));
            self.graph_input = graph_input;
        }
        let palette_changed = self.palette != palette;
        self.snapshot = snapshot;
        self.task_id = task_id.into();
        self.items = items;
        self.palette = palette;
        if graph_changed || palette_changed {
            cx.notify();
        }
    }
}
impl Render for AgentGraph {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette;
        let zoom = self.zoom;
        let scaled = move |value: f32| px(value * zoom);
        let mut model = self.model.clone();
        layout_nodes(&mut model, self.completed_expanded);
        let lines = model.clone();
        let graph_scroll = self.graph_scroll.clone();
        let completed_expanded = self.completed_expanded;
        let zoom_slider = self.zoom_slider.clone();
        let completed_count = model
            .nodes
            .iter()
            .skip(1)
            .filter(|node| finished(&node.status))
            .count();
        let first_completed = model
            .nodes
            .iter()
            .enumerate()
            .skip(1)
            .find(|(_, node)| finished(&node.status))
            .map(|(index, _)| index);
        let diagram = div()
            .relative()
            .w(scaled(model.width))
            .h(scaled(model.height))
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| {
                        let draw = |a: &Node, b: &Node, lane: Option<f32>, window: &mut Window| {
                            let message = lane.is_some();
                            let start = point(
                                bounds.origin.x
                                    + scaled(
                                        a.x + if message { CARD_WIDTH } else { CARD_WIDTH / 2. },
                                    ),
                                bounds.origin.y
                                    + scaled(
                                        a.y + if message {
                                            CARD_HEIGHT / 2.
                                        } else {
                                            CARD_HEIGHT
                                        },
                                    ),
                            );
                            let end = point(
                                bounds.origin.x
                                    + scaled(
                                        b.x + if message { CARD_WIDTH } else { CARD_WIDTH / 2. },
                                    ),
                                bounds.origin.y
                                    + scaled(b.y + if message { CARD_HEIGHT / 2. } else { 0. }),
                            );
                            let mut path =
                                PathBuilder::stroke(scaled(if message { 2. } else { 1. }));
                            path.move_to(start);
                            if let Some(lane) = lane {
                                let x = bounds.origin.x + scaled(lane);
                                path.line_to(point(x, start.y));
                                path.line_to(point(x, end.y));
                                path.line_to(end);
                                path.move_to(point(end.x + scaled(7.), end.y - scaled(4.)));
                                path.line_to(end);
                                path.line_to(point(end.x + scaled(7.), end.y + scaled(4.)));
                            } else {
                                let middle = (start.y + end.y) / 2.;
                                path.line_to(point(start.x, middle));
                                path.line_to(point(end.x, middle));
                                path.line_to(end);
                                path.move_to(point(end.x - scaled(4.), end.y - scaled(6.)));
                                path.line_to(end);
                                path.line_to(point(end.x + scaled(4.), end.y - scaled(6.)));
                            }
                            if let Ok(path) = path.build() {
                                window
                                    .paint_path(path, rgb(if message { p.accent } else { p.line }));
                            }
                        };
                        for (index, node) in lines.nodes.iter().enumerate() {
                            if let Some(parent) = node.parent.filter(|_| completed_expanded || index == 0 || active(&node.status)) {
                                draw(&lines.nodes[parent], node, None, window);
                            }
                        }
                        let right = lines
                            .nodes
                            .iter()
                            .map(|n| n.x + CARD_WIDTH)
                            .fold(0., f32::max);
                        for (index, (a, b)) in lines.messages.iter().enumerate() {
                            draw(
                                &lines.nodes[*a],
                                &lines.nodes[*b],
                                Some(right + 24. + index as f32 * 8.),
                                window,
                            );
                        }
                    },
                )
                .absolute()
                .inset_0(),
            )
            .children(model.nodes.iter().enumerate().filter(move |(index, node)| {
                *index == 0 || active(&node.status) || completed_expanded || Some(*index) == first_completed
            }).map(|(index, node)| {
                div()
                    .id(("graph-node", index))
                    .debug_selector(move || format!("graph-node-{index}").into())
                    .absolute()
                    .left(scaled(node.x))
                    .top(scaled(node.y))
                    .w(scaled(CARD_WIDTH))
                    .h(scaled(CARD_HEIGHT))
                    .p(scaled(12.))
                    .rounded(scaled(4.))
                    .border_1()
                    .border_color(rgb(if index == 0 || active(&node.status) {
                        p.accent
                    } else {
                        p.line
                    }))
                    .bg(design::surface_color(p.panel))
                    .flex()
                    .flex_col()
                    .gap(scaled(5.))
                    .overflow_hidden()
                    .when(index > 0 && finished(&node.status), |el| el.on_click(cx.listener(|this, _, _, cx| { this.completed_expanded = !this.completed_expanded; cx.notify(); })))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(scaled(6.))
                            .child(
                                Icon::new(IconName::Bot)
                                    .size(scaled(15.))
                                    .text_color(rgb(p.accent)),
                            )
                            .text_size(scaled(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(p.ink))
                            .child(if index > 0 && finished(&node.status) && !completed_expanded { format!("{completed_count} finished agents · click to expand") } else if node.name.is_empty() {
                                "Subagent".to_string()
                            } else {
                                node.name.clone()
                            }),
                    )
                    .when(index == 0 || active(&node.status) || completed_expanded, |el| el.child(
                        div()
                            .text_size(scaled(10.))
                            .text_color(rgb(p.muted))
                            .child(format!(
                                "{} · {}",
                                if index == 0 { "Main" } else { "Subagent" },
                                if node.status.is_empty() {
                                    "Status unavailable"
                                } else {
                                    &node.status
                                }
                            )),
                    ))
                    .when(index == 0 || active(&node.status) || completed_expanded, |el| el.child(
                        div()
                            .text_size(scaled(11.))
                            .text_color(rgb(p.ink))
                            .child(node.title.clone()),
                    ))
                    .when(index == 0 || active(&node.status) || completed_expanded, |el| el.child(div().text_size(scaled(10.)).text_color(rgb(p.muted)).child(
                        if node.model.is_empty() {
                            "Model not reported".into()
                        } else {
                            node.model.clone()
                        },
                    )))
            }));
        div()
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(design::surface_color(p.paper))
            .text_color(rgb(p.ink))
            .on_pinch(cx.listener(|this, event: &PinchEvent, window, cx| {
                this.zoom = clamped_zoom(this.zoom * (1. + event.delta));
                this.zoom_slider.update(cx, |slider, cx| {
                    slider.set_value(this.zoom * 100., window, cx)
                });
                cx.notify();
            }))
            .child(
                div()
                    .px(px(14.))
                    .py(px(8.))
                    .text_size(px(10.))
                    .text_color(rgb(p.muted))
                    .child(if model.nodes.len() == 1 {
                        "No subagents recorded for this chat · No recorded agent messages"
                    } else if model.messages.is_empty() {
                        "Lines: delegation · No recorded messages between currently active agents"
                    } else {
                        "Lines: delegation · Accent arrows: recorded messages between active agents"
                    }),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .child(ScrollEdgeFade::new(
                        ScrollEdgeFade::new(
                            div()
                                .id("agent-graph-scroll")
                                .debug_selector(|| "agent-graph-scroll".into())
                                .size_full()
                                .overflow_scroll()
                                .track_scroll(&graph_scroll)
                                .child(diagram),
                            graph_scroll.clone(),
                            Axis::Vertical,
                            px(18.),
                        ),
                        graph_scroll,
                        Axis::Horizontal,
                        px(18.),
                    ))
                    .child(
                        div()
                            .absolute()
                            .left(px(12.))
                            .bottom(px(12.))
                            .w(px(220.))
                            .rounded(px(6.))
                            .border_1()
                            .border_color(rgb(p.line))
                            .bg(design::surface_color(p.panel).opacity(0.82))
                            .p(px(8.))
                            .opacity(0.04)
                            .hover(|style| style.opacity(1.))
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                div()
                                    .w(px(42.))
                                    .text_center()
                                    .text_size(px(10.))
                                    .child(format!("{:.0}%", zoom * 100.)),
                            )
                            .child(
                                div()
                                    .id("graph-zoom-slider")
                                    .debug_selector(|| "graph-zoom-slider".into())
                                    .w(px(120.))
                                    .child(gpui_kit::component::slider::Slider::new(&zoom_slider)),
                            )
                            .child(
                                div()
                                    .id("graph-zoom-reset")
                                    .cursor_pointer()
                                    .text_size(px(10.))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.zoom = 1.;
                                        this.zoom_slider.update(cx, |slider, cx| {
                                            slider.set_value(100., window, cx)
                                        });
                                        cx.notify();
                                    }))
                                    .child("Reset"),
                            ),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[::core::prelude::v1::test]
    fn hierarchy_flows_downward_with_nonoverlapping_centered_ranks() {
        let items = vec![
            serde_json::json!({"taskId":"a","parentTaskId":"root"}),
            serde_json::json!({"taskId":"b","parentTaskId":"root"}),
            serde_json::json!({"taskId":"c","parentTaskId":"a"}),
        ];
        let graph = GraphModel::build(&Value::Null, "root", &items);
        let root = &graph.nodes[0];
        let a = &graph.nodes[1];
        let b = &graph.nodes[2];
        let c = &graph.nodes[3];
        assert_eq!(a.y, b.y);
        assert!(root.y + CARD_HEIGHT < a.y);
        assert!(a.y + CARD_HEIGHT < c.y);
        assert!(a.x + CARD_WIDTH < b.x);
        assert_eq!(root.x, (a.x + b.x) / 2.);
        assert!(
            graph
                .nodes
                .iter()
                .all(|node| node.x + CARD_WIDTH <= graph.width
                    && node.y + CARD_HEIGHT <= graph.height)
        );
    }
    #[::core::prelude::v1::test]
    fn zoom_bounds_keep_graph_legible_and_finite() {
        assert_eq!(clamped_zoom(0.), 0.4);
        assert_eq!(clamped_zoom(3.), 2.);
        assert_eq!(clamped_zoom(f32::NAN), 1.);
        assert_eq!(clamped_zoom(1.15), 1.15);
    }
    #[::core::prelude::v1::test]
    fn completed_nodes_stack_then_expand_to_nonoverlapping_grid() {
        let items = vec![
            json!({"taskId":"a","parentTaskId":"root","status":"completed"}),
            json!({"taskId":"b","parentTaskId":"root","status":"failed"}),
            json!({"taskId":"c","parentTaskId":"root","status":"running"}),
        ];
        let base = GraphModel::build(&Value::Null, "root", &items);
        let mut stacked = base.clone();
        layout_nodes(&mut stacked, false);
        assert_eq!(
            (stacked.nodes[1].x, stacked.nodes[1].y),
            (stacked.nodes[2].x, stacked.nodes[2].y)
        );
        let mut expanded = base;
        layout_nodes(&mut expanded, true);
        assert_ne!(
            (expanded.nodes[1].x, expanded.nodes[1].y),
            (expanded.nodes[2].x, expanded.nodes[2].y)
        );
        assert_eq!(expanded.nodes[1].y, expanded.nodes[2].y);
    }
    #[::core::prelude::v1::test]
    fn scoped_nested_graph_only_uses_real_active_messages() {
        let snapshot = json!({"tasks":[{"id":"root","status":"running"}],"collaborations":[{"kind":"message","fromTaskId":"a","toTaskId":"b"},{"kind":"delegation","fromTaskId":"b","toTaskId":"a"},{"kind":"message","fromTaskId":"b","toTaskId":"done"}]});
        let items = vec![
            json!({"id":"a","taskId":"a","parentTaskId":"root","status":"running"}),
            json!({"id":"b","taskId":"b","parentTaskId":"a","status":"queued"}),
            json!({"id":"done","taskId":"done","parentTaskId":"root","status":"completed"}),
            json!({"id":"other","parentTaskId":"unrelated","status":"running"}),
        ];
        let graph = GraphModel::build(&snapshot, "root", &items);
        assert_eq!(graph.nodes.len(), 4);
        assert_eq!(graph.nodes[2].parent, Some(1));
        assert_eq!(graph.nodes[2].depth, 2);
        assert_eq!(graph.messages, vec![(1, 2)]);
    }
    #[::core::prelude::v1::test]
    fn empty_snapshot_has_main_node_and_no_invented_activity() {
        let graph = GraphModel::build(&Value::Null, "root", &[]);
        assert_eq!(graph.nodes.len(), 1);
        assert!(graph.messages.is_empty());
        assert!(graph.nodes[0].model.is_empty());
        assert!(rows(&Value::Null, "workPlans").is_empty());
    }
    #[::core::prelude::v1::test]
    fn native_thread_identity_preserves_nesting() {
        let items = vec![
            json!({"id":"a","parentTaskId":"root","agentThreadId":"a","status":"running"}),
            json!({"id":"b","parentTaskId":"root","parentThreadId":"a","agentThreadId":"b","status":"running"}),
        ];
        let graph = GraphModel::build(&Value::Null, "root", &items);
        assert_eq!(graph.nodes[2].parent, Some(1));
        assert_eq!(graph.nodes[2].depth, 2);
    }
}

#[cfg(test)]
#[path = "agent_graph_tests.rs"]
mod render_tests;
