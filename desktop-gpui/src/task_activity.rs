//! Native goal and observed computer-use HUD. No synthetic lifecycle state.
#[path = "working_bubble.rs"]
pub(crate) mod working_bubble;
use crate::design::{self, ui_scaled_px as px};
use gpui_kit::{
    assets::__private as icon_assets,
    component::{Disableable, Icon, Sizable, button::*},
    prelude::*,
    *,
};
use monitter_lib::experimental_desktop::DesktopService;
use serde_json::{Value, json};
use std::{collections::BTreeMap, time::Duration};

pub enum ActivityEvent {
    Refresh,
}
pub struct TaskActivityPane {
    service: DesktopService,
    key: String,
    task_id: String,
    eligible: bool,
    running: bool,
    // One retained maintenance timer. An absent goal schedules no timer;
    // context changes cancel an obsolete session's pending retry.
    goal_poll: Option<Task<()>>,
    generation: u64,
    read_failures: u32,
    goal: Option<Value>,
    goal_note: String,
    note_expanded: bool,
    palette: design::Palette,
    font_ratio: f32,
    margin_inline: f32,
    tools: Vec<String>,
    expanded: bool,
    loading: bool,
    mutating: bool,
    error: Option<String>,
}
impl EventEmitter<ActivityEvent> for TaskActivityPane {}
fn text(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or_default().to_owned()
}

/// Only current-turn, observed started/completed records imply active computer use.
fn active_tools<'a>(
    task: &Value,
    messages: impl IntoIterator<Item = &'a Value>,
    events: impl IntoIterator<Item = &'a Value>,
) -> Vec<String> {
    if task["status"] != "running" {
        return vec![];
    }
    let Some(start) = messages
        .into_iter()
        .filter(|m| m["role"] == "user")
        .filter_map(|m| m["createdAt"].as_i64())
        .max()
    else {
        return vec![];
    };
    let mut active = BTreeMap::new();
    for event in events
        .into_iter()
        .filter(|e| e["createdAt"].as_i64().is_some_and(|at| at >= start))
    {
        if event["kind"] == "status"
            && matches!(
                event["title"].as_str(),
                Some("turn.completed" | "turn.failed")
            )
        {
            active.clear();
        }
        if event["kind"] != "computer" {
            continue;
        }
        let Ok(data) = serde_json::from_str::<Value>(event["detail"].as_str().unwrap_or_default()) else {
            continue;
        };
        let Some(id) = data["id"].as_str() else {
            continue;
        };
        match data["phase"].as_str() {
            Some("completed") => {
                active.remove(id);
            }
            Some("started") => {
                let summary = text(&data, "summary");
                active.insert(
                    id.to_owned(),
                    if summary.is_empty() {
                        data["tool"].as_str().unwrap_or("Computer use").to_owned()
                    } else {
                        summary
                    },
                );
            }
            _ => {}
        }
    }
    active.into_values().collect()
}

fn snapshot_active_tools(task: &Value, task_id: &str, snapshot: &Value) -> Vec<String> {
    let messages = snapshot["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|message| message["taskId"] == task_id);
    let events = snapshot["events"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|event| event["taskId"] == task_id);
    active_tools(task, messages, events)
}
/// Absent goals need no maintenance. Read failures back off quietly; only
/// known non-complete goals in a running task refresh once per minute.
fn next_goal_read(goal: Option<&Value>, running: bool, failures: u32) -> Option<Duration> {
    // A visible goal already has a known identity. Keep its live accounting
    // current after a transient read failure rather than hiding stale values
    // behind the long discovery retry used for an unknown/absent goal.
    if failures > 0 && goal.is_some_and(|goal| goal["status"] != "complete") {
        return Some(Duration::from_secs(60));
    }
    if failures > 0 {
        return Some(Duration::from_secs(
            (60_u64 * 2_u64.pow((failures - 1).min(4))).min(900),
        ));
    }
    (running && goal.is_some_and(|goal| goal["status"] != "complete"))
        .then_some(Duration::from_secs(60))
}

impl TaskActivityPane {
    #[cfg(test)]
    pub(crate) fn tools(&self) -> &[String] {
        &self.tools
    }
    pub fn new(service: DesktopService, _cx: &mut Context<Self>) -> Self {
        Self {
            service,
            key: String::new(),
            task_id: String::new(),
            eligible: false,
            running: false,
            goal_poll: None,
            generation: 0,
            read_failures: 0,
            goal: None,
            goal_note: String::new(),
            note_expanded: false,
            palette: design::palette(&Value::Null, WindowAppearance::Light),
            font_ratio: 1.,
            margin_inline: 12.,
            tools: vec![],
            expanded: false,
            loading: false,
            mutating: false,
            error: None,
        }
    }
    pub fn set_appearance(
        &mut self,
        palette: design::Palette,
        font_ratio: f32,
        margin_inline: f32,
        cx: &mut Context<Self>,
    ) {
        if self.palette != palette
            || self.font_ratio != font_ratio
            || self.margin_inline != margin_inline
        {
            self.palette = palette;
            self.font_ratio = font_ratio;
            self.margin_inline = margin_inline;
            cx.notify();
        }
    }
    pub fn set_context(&mut self, snapshot: &Value, task_id: &str, cx: &mut Context<Self>) {
        let Some(task) = snapshot["tasks"]
            .as_array()
            .and_then(|tasks| tasks.iter().find(|task| task["id"] == task_id))
        else {
            if self.key.is_empty() {
                return;
            }
            self.goal_poll.take();
            self.generation = self.generation.wrapping_add(1);
            self.key.clear();
            self.task_id.clear();
            self.running = false;
            self.eligible = false;
            self.goal = None;
            self.goal_note.clear();
            self.tools.clear();
            self.error = None;
            self.loading = false;
            self.mutating = false;
            self.read_failures = 0;
            cx.notify();
            return;
        };
        let tools = snapshot_active_tools(task, task_id, snapshot);
        let goal_note = if task["provider"] == "hermes" {
            snapshot["events"]
                .as_array()
                .into_iter()
                .flatten()
                .rev()
                .find(|event| event["taskId"] == task_id && event["kind"] == "goal")
                .map(|event| text(event, "detail"))
                .unwrap_or_default()
        } else {
            String::new()
        };
        let projection_changed = self.tools != tools || self.goal_note != goal_note;
        self.tools = tools;
        self.goal_note = goal_note;
        let key = format!(
            "{}|{}|{}",
            task_id,
            text(task, "nativeSessionId"),
            text(task, "provider")
        );
        let running = task["status"] == "running";
        let running_changed = self.running != running;
        self.running = running;
        if self.key == key {
            if running_changed {
                if self.goal.is_some() {
                    // Read the final counters/status as a turn stops, and
                    // refresh immediately when its next continuation starts.
                    self.refresh(cx);
                } else {
                    self.schedule_goal_read(cx);
                }
            }
            if projection_changed {
                cx.notify();
            }
            return;
        }
        self.goal_poll.take();
        self.key = key;
        self.task_id = task_id.to_owned();
        self.running = task["status"] == "running";
        self.eligible = task["provider"] == "codex" && !text(task, "nativeSessionId").is_empty();
        self.generation = self.generation.wrapping_add(1);
        self.read_failures = 0;
        self.goal = None;
        self.error = None;
        self.loading = false;
        self.mutating = false;
        self.expanded = false;
        if self.eligible {
            self.refresh(cx);
        }
    }
    fn schedule_goal_read(&mut self, cx: &mut Context<Self>) {
        self.goal_poll.take();
        if !self.eligible {
            return;
        }
        let Some(delay) = next_goal_read(self.goal.as_ref(), self.running, self.read_failures)
        else {
            return;
        };
        let generation = self.generation;
        self.goal_poll = Some(cx.spawn(async move |view, cx| {
            cx.background_executor().timer(delay).await;
            let _ = view.update(cx, |view, cx| {
                if view.generation == generation {
                    view.refresh(cx);
                }
            });
        }));
    }
    pub(super) fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.loading || self.mutating || !self.eligible {
            return;
        }
        self.goal_poll.take();
        self.loading = true;
        let service = self.service.clone();
        let id = self.task_id.clone();
        let generation = self.generation;
        let read = cx
            .background_executor()
            .spawn(async move { service.invoke("get_task_goal", json!({"taskId":id})) });
        cx.spawn(async move |view, cx| {
            let result = read.await;
            let _ = view.update(cx, |view, cx| {
                if view.generation != generation {
                    return;
                }
                view.loading = false;
                match result {
                    Ok(goal) => {
                        view.read_failures = 0;
                        let goal = (!goal.is_null()).then_some(goal);
                        if view.goal != goal {
                            view.goal = goal;
                            cx.notify();
                        }
                    }
                    Err(_) => {
                        // A lookup is maintenance, not a user action. Preserve
                        // the last known card and leave the UI error-free.
                        view.read_failures = view.read_failures.saturating_add(1);
                    }
                }
                view.schedule_goal_read(cx);
            });
        })
        .detach();
    }
    fn mutate(&mut self, command: &'static str, cx: &mut Context<Self>) {
        if self.mutating {
            return;
        }
        self.goal_poll.take();
        self.mutating = true;
        self.error = None;
        // Invalidate outstanding reads so a pre-clear response cannot resurrect a goal.
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        let service = self.service.clone();
        let id = self.task_id.clone();
        let task = cx
            .background_executor()
            .spawn(async move { service.invoke(command, json!({"taskId":id})) });
        cx.spawn(async move |view, cx| {
            let result = task.await;
            let _ = view.update(cx, |view, cx| {
                if view.generation != generation {
                    return;
                }
                view.mutating = false;
                view.loading = false;
                match result {
                    Ok(_) => {
                        if command == "clear_task_goal" {
                            view.goal = None;
                            view.read_failures = 0;
                        } else {
                            view.schedule_goal_read(cx);
                        }
                        cx.emit(ActivityEvent::Refresh);
                    }
                    Err(error) => {
                        view.error = Some(error);
                        view.schedule_goal_read(cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}
// Group integer counts like the English desktop number formatter; never round usage.
fn token_count(value: u64) -> String {
    let digits = value.to_string();
    let mut grouped = String::new();
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(character);
    }
    grouped
}
fn mix_rgb(base: u32, color: u32, amount: f32) -> u32 {
    let channel = |shift: u32| {
        let base = ((base >> shift) & 0xff) as f32;
        let color = ((color >> shift) & 0xff) as f32;
        (base * (1. - amount) + color * amount).round() as u32
    };
    (channel(16) << 16) | (channel(8) << 8) | channel(0)
}
impl TaskActivityPane {
    fn render_computer(&mut self, cx: &mut Context<Self>) -> Div {
        let palette = self.palette;
        if !self.tools.is_empty() {
            let mut body = div().flex_1().min_w(px(0.)).child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .text_color(rgb(palette.accent))
                    .child(
                        Icon::default()
                            .data(icon_assets::Monitor.1)
                            .with_size(px(14.)),
                    )
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Using the computer"),
                    ),
            );
            for tool in &self.tools {
                body = body.child(
                    div()
                        .mt(px(5.))
                        .truncate()
                        .text_color(rgb(palette.muted))
                        .child(tool.clone()),
                );
            }
            return div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .px(px(12.))
                    .py(px(10.))
                    .border_1()
                    .border_color(rgb(palette.line))
                    .border_blend_mode(crate::surface_blend::separator_mode())
                    .rounded(px(8.))
                    .bg(crate::design::surface_color(palette.card))
                    .child(body)
                    .child(
                        Button::new("computer-stop")
                            .compact()
                            .label("Stop")
                            .accessibility_label("Stop computer use")
                            .disabled(self.mutating)
                            .on_click(cx.listener(|view, _, _, cx| view.mutate("cancel_task", cx))),
                    );
        }
        div()
    }
}
pub struct ComputerUseBubble { activity: Entity<TaskActivityPane> }
impl ComputerUseBubble {
    pub fn new(activity: Entity<TaskActivityPane>, cx: &mut Context<Self>) -> Self {
        cx.observe(&activity, |_, _, cx| cx.notify()).detach();
        Self { activity }
    }
}
impl Render for ComputerUseBubble {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let visible = !self.activity.read(cx).tools.is_empty();
        if !visible { return div(); }
        let bubble = self.activity.update(cx, |activity, cx| activity.render_computer(cx));
        div().absolute().top(px(8.)).left(px(12.)).right(px(12.))
            .flex().justify_center()
            .child(div().w_full().max_w(px(480.)).text_size(px(12.)).child(bubble))
    }
}
impl Render for TaskActivityPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self.palette;
        let type_size = |size| px(size * self.font_ratio);
        let mut cards = div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(8.))
            .text_size(type_size(12.))
            .text_color(rgb(palette.ink));
        let visible_goal = self
            .goal
            .as_ref()
            .filter(|goal| goal["status"] != "complete");
        if let Some(goal) = visible_goal {
            let active = goal["status"] == "active";
            let status = match goal["status"].as_str().unwrap_or_default() {
                "active" => "Active",
                "paused" => "Paused",
                "blocked" => "Blocked",
                "usageLimited" => "Usage limit",
                "budgetLimited" => "Budget reached",
                other => other,
            };
            let objective = text(goal, "objective");
            let mut card = div()
                .relative()
                .w_full()
                .flex()
                .flex_col()
                .min_h(px(64.))
                .pt(px(10.))
                .pb(px(10.))
                .pl(px(12.))
                .pr(px(48.))
                .border_1()
                .border_b(px(0.))
                .rounded_tl(px(8.))
                .rounded_tr(px(8.))
                .border_color(rgb(if active {
                    mix_rgb(palette.line, palette.accent, 0.45)
                } else {
                    palette.line
                }))
                .bg(crate::design::surface_color(if active {
                    mix_rgb(palette.card, palette.accent, 0.06)
                } else {
                    palette.card
                }))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(7.))
                        .text_color(rgb(palette.accent))
                        .child(Icon::default().data(icon_assets::Goal.1).with_size(px(14.)))
                        .child(div().font_weight(FontWeight::SEMIBOLD).child("Goal"))
                        .child(
                            div()
                                .ml_auto()
                                .font_family(design::FONT_MONO)
                                .text_size(type_size(10.))
                                .text_color(rgb(palette.muted))
                                .child(status.to_owned()),
                        ),
                );
            card = card.child(
                Button::new("goal-expand")
                    .ghost()
                    .w_full()
                    .h_auto()
                    .min_h(px(0.))
                    .p_0()
                    .mt(px(7.))
                    .justify_start()
                    .text_size(type_size(12.))
                    .accessibility_label(if self.expanded {
                        "Collapse goal objective"
                    } else {
                        "Expand goal objective"
                    })
                    .tooltip(objective.clone())
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.expanded = !view.expanded;
                        cx.notify();
                    }))
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .items_center()
                            .gap(px(5.))
                            .overflow_hidden()
                            .child(
                                Icon::default()
                                    .data(if self.expanded {
                                        icon_assets::ChevronDown.1
                                    } else {
                                        icon_assets::ChevronRight.1
                                    })
                                    .with_size(px(12.)),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .truncate()
                                    .child(objective.clone()),
                            ),
                    ),
            );
            if self.expanded {
                let max_height = (window.viewport_size().height / 2.).min(px(360.));
                card = card.child(
                    div()
                        .id("goal-objective-scroll")
                        .mt(px(7.))
                        .w_full()
                        .max_h(max_height)
                        .overflow_y_scroll()
                        .pr(px(4.))
                        .line_height(relative(1.5))
                        .child(objective),
                );
            }
            if let Some(used) = goal["tokensUsed"].as_u64() {
                let budget = goal["tokenBudget"].as_u64().filter(|n| *n > 0);
                let label = budget.map_or_else(
                    || format!("{} tokens", token_count(used)),
                    |budget| format!("{} / {} tokens", token_count(used), token_count(budget)),
                );
                let mut usage = div()
                    .mt(px(7.))
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(12.))
                    .font_family(design::FONT_MONO)
                    .text_size(type_size(10.))
                    .text_color(rgb(palette.muted))
                    .child(label);
                if let Some(budget) = budget {
                    usage = usage.child(
                        div()
                            .w(px(90.))
                            .h(px(5.))
                            .flex_shrink_0()
                            .rounded(px(2.5))
                            .overflow_hidden()
                            .bg(crate::design::surface_color(palette.soft))
                            .child(
                                div()
                                    .h_full()
                                    .w(px(90. * (used as f64 / budget as f64).clamp(0., 1.) as f32))
                                    .bg(rgb(palette.accent)),
                            ),
                    );
                }
                card = card.child(usage);
            }
            card = card.child(
                Button::new("goal-clear")
                    .ghost()
                    .compact()
                    .absolute()
                    .right(px(8.))
                    .bottom(px(8.))
                    .p(px(6.))
                    .icon(
                        Icon::default()
                            .data(icon_assets::Trash.1)
                            .with_size(px(14.)),
                    )
                    .accessibility_label("Clear goal")
                    .tooltip(if self.mutating {
                        "Clearing goal…"
                    } else {
                        "Clear goal"
                    })
                    .disabled(self.mutating)
                    .on_click(cx.listener(|view, _, _, cx| view.mutate("clear_task_goal", cx))),
            );
            cards = cards.child(card);
        }
        if !self.goal_note.is_empty() {
            let note = self.goal_note.clone();
            let mut card = div()
                .w_full()
                .pt(px(10.))
                .pb(px(10.))
                .px(px(12.))
                .border_1()
                .border_b(px(0.))
                .border_color(rgb(palette.line))
                .border_blend_mode(crate::surface_blend::separator_mode())
                .rounded_tl(px(8.))
                .rounded_tr(px(8.))
                .bg(crate::design::surface_color(palette.card))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(7.))
                        .text_color(rgb(palette.accent))
                        .child(Icon::default().data(icon_assets::Goal.1).with_size(px(14.)))
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("Last goal update"),
                        ),
                )
                .child(
                    Button::new("goal-note-expand")
                        .ghost()
                        .w_full()
                        .h_auto()
                        .min_h(px(0.))
                        .p_0()
                        .mt(px(7.))
                        .justify_start()
                        .text_size(type_size(12.))
                        .accessibility_label(if self.note_expanded {
                            "Collapse last goal update"
                        } else {
                            "Expand last goal update"
                        })
                        .tooltip(note.clone())
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.note_expanded = !view.note_expanded;
                            cx.notify();
                        }))
                        .child(div().w_full().truncate().child(note.clone())),
                );
            if self.note_expanded {
                card = card.child(
                    div()
                        .id("goal-note-scroll")
                        .mt(px(7.))
                        .max_h(px(360.))
                        .overflow_y_scroll()
                        .line_height(relative(1.5))
                        .child(note),
                );
            }
            cards = cards.child(card);
        }
        // Goal reads are background maintenance; retain the current card and
        // leave an absent goal quiet while its initial read is pending.
        if let Some(error) = &self.error {
            cards = cards.child(div().text_color(rgb(0xd65b5b)).child(error.clone()));
        }
        div()
            .w_full()
            .flex_shrink_0()
            .px(px(self.margin_inline + 8.))
            .child(cards.max_w(px(design::CHAT_MAX_WIDTH - 16.)).mx_auto())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[::core::prelude::v1::test]
    fn absent_and_complete_goals_do_not_poll_but_known_running_goals_do() {
        assert_eq!(next_goal_read(None, true, 0), None);
        assert_eq!(
            next_goal_read(Some(&json!({"status":"complete"})), true, 0),
            None
        );
        assert_eq!(
            next_goal_read(Some(&json!({"status":"active"})), false, 0),
            None
        );
        assert_eq!(
            next_goal_read(Some(&json!({"status":"active"})), true, 0),
            Some(Duration::from_secs(60))
        );
    }
    #[::core::prelude::v1::test]
    fn lookup_failures_retry_silently_after_long_bounded_backoff() {
        assert_eq!(
            next_goal_read(None, false, 1),
            Some(Duration::from_secs(60))
        );
        assert_eq!(
            next_goal_read(None, true, 2),
            Some(Duration::from_secs(120))
        );
        assert_eq!(
            next_goal_read(None, true, u32::MAX),
            Some(Duration::from_secs(900))
        );
        assert_eq!(next_goal_read(None, true, 0), None);
        assert_eq!(next_goal_read(Some(&json!({"status":"active"})), true, u32::MAX),
            Some(Duration::from_secs(60)));
    }
    #[::core::prelude::v1::test]
    fn computer_activity_requires_current_running_turn_and_matching_lifecycle() {
        let task = json!({"status":"running"});
        let messages = vec![json!({"role":"user","createdAt":20})];
        let event = |id: &str, phase: &str, at: i64| json!({"kind":"computer","createdAt":at,"detail":json!({"id":id,"phase":phase,"summary":id}).to_string()});
        let events = vec![
            event("old", "started", 10),
            event("one", "started", 21),
            event("two", "started", 22),
            event("one", "completed", 23),
        ];
        assert_eq!(active_tools(&task, &messages, &events), ["two"]);
        assert!(active_tools(&json!({"status":"idle"}), &messages, &events).is_empty());
        assert!(active_tools(&task, &[], &events).is_empty());
    }
    #[::core::prelude::v1::test]
    fn turn_completion_clears_observed_activity_and_malformed_events_do_not_activate() {
        let task = json!({"status":"running"});
        let messages = vec![json!({"role":"user","createdAt":1})];
        let events = vec![
            json!({"kind":"computer","createdAt":2,"detail":"bad"}),
            json!({"kind":"computer","createdAt":3,"detail":"{\"id\":\"x\",\"phase\":\"started\"}"}),
            json!({"kind":"status","createdAt":4,"title":"turn.completed"}),
        ];
        assert!(active_tools(&task, &messages, &events).is_empty());
    }

    #[::core::prelude::v1::test]
    fn flat_snapshot_activity_is_scoped_to_its_task() {
        let task = json!({"id":"own","status":"running"});
        let snapshot = json!({
            "tasks":[task],
            "messages":[
                {"id":"own-user","taskId":"own","role":"user","createdAt":20},
                {"id":"other-user","taskId":"other","role":"user","createdAt":22}
            ],
            "events":[
                {"taskId":"own","kind":"computer","createdAt":21,"detail":"{\"id\":\"computer-1\",\"phase\":\"started\",\"summary\":\"Browser\"}"},
                {"taskId":"other","kind":"status","createdAt":23,"title":"turn.completed"}
            ]
        });

        assert_eq!(snapshot_active_tools(&task, "own", &snapshot), ["Browser"]);

        let completed = json!({
            "taskId":"own","kind":"computer","createdAt":24,
            "detail":"{\"id\":\"computer-1\",\"phase\":\"completed\"}"
        });
        let mut completed_snapshot = snapshot.clone();
        completed_snapshot["events"]
            .as_array_mut()
            .unwrap()
            .push(completed);
        assert!(snapshot_active_tools(&task, "own", &completed_snapshot).is_empty());
    }
    #[::core::prelude::v1::test]
    fn borrowed_activity_uses_latest_user_cutoff_and_preserves_event_order() {
        let task = json!({"id":"own","status":"running","provider":"codex","nativeSessionId":"session"});
        let event = |id: &str, phase: &str, at: i64| json!({"taskId":"own","kind":"computer","createdAt":at,
            "detail":json!({"id":id,"phase":phase,"summary":id}).to_string()});
        let mut snapshot = json!({
            "messages":[
                {"taskId":"own","role":"user","createdAt":30},
                {"taskId":"own","role":"assistant","createdAt":200,"text":"large payload".repeat(100_000)},
                {"taskId":"other","role":"user","createdAt":300},
                {"taskId":"own","role":"user","createdAt":10},
                {"taskId":"own","role":"user","createdAt":"invalid"}
            ],
            "events":[event("previous-turn","started",29), event("at-cutoff","started",30),
                {"taskId":"other","kind":"status","createdAt":31,"title":"turn.failed"},
                {"taskId":"own","kind":"computer","createdAt":32,"detail":"malformed"},
                event("at-cutoff","completed",33), event("current","started",34),
                event("unknown-phase","queued",35)]
        });
        assert_eq!(snapshot_active_tools(&task, "own", &snapshot), ["current"]);
        snapshot["events"].as_array_mut().unwrap().push(json!({
            "taskId":"own","kind":"status","createdAt":36,"title":"turn.failed"
        }));
        assert!(snapshot_active_tools(&task, "own", &snapshot).is_empty());
        // Preserve chronological input processing: a later observed start is
        // still visible after the turn-failed marker, just as before this repair.
        snapshot["events"].as_array_mut().unwrap().push(event("restarted", "started", 37));
        assert_eq!(snapshot_active_tools(&task, "own", &snapshot), ["restarted"]);
    }
    #[::core::prelude::v1::test]
    fn inactive_or_userless_activity_does_not_traverse_unused_borrowed_inputs() {
        let never_read = std::iter::once_with(|| -> &Value { panic!("unused history was traversed") });
        let never_read_events = std::iter::once_with(|| -> &Value { panic!("unused events were traversed") });
        assert!(active_tools(&json!({"status":"idle"}), never_read, never_read_events).is_empty());
        let assistant_only = [json!({"role":"assistant","createdAt":100})];
        let never_read_events = std::iter::once_with(|| -> &Value { panic!("no user cutoff must skip events") });
        assert!(active_tools(&json!({"status":"running"}), &assistant_only, never_read_events).is_empty());
    }
}
