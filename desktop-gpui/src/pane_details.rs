//! Task-scoped Details and lazy diagnostic Timeline for a retained chat pane.
//! All task state comes from the authoritative native snapshot or the bounded
//! `get_task_events` page command; this view never starts a provider turn.
use crate::chat::run_activity::RunActivityRow;
use crate::design::Palette;
use crate::design::ui_scaled_px as px;
use futures::channel::oneshot;
use gpui_kit::{
    assets::IconName,
    component::{
        Disableable, Icon, Sizable,
        button::{Button, ButtonVariants},
    },
    prelude::*,
    *,
};
use monitter_lib::experimental_desktop::DesktopService;
use serde_json::{Value, json};
use std::collections::HashMap;

#[derive(Clone)]
pub enum PaneDetailsEvent {
    OpenTask(String),
    DelegateTask(String),
}

const EVENT_PAGE_LIMIT: u64 = 100;
const EVENT_WORKER_STACK_SIZE: usize = 8 * 1024 * 1024;
const RUN_SUMMARY_WORKER_STACK_SIZE: usize = 8 * 1024 * 1024;
const MAX_TRACKED_TERMINALS: usize = 100;
const MAX_TERMINAL_TITLE_CHARS: usize = 256;
const INITIAL_WORK_PLAN_ITEMS: usize = 8;
const INITIAL_WORK_PLAN_HISTORY: usize = 5;

#[derive(Clone, Debug, PartialEq, Eq)]
struct TrackedTerminal {
    title: String,
}

fn tracked_terminal_page(
    value: &Value,
    host_id: &str,
    cwd: &str,
) -> Result<Vec<TrackedTerminal>, String> {
    let sessions = value.as_array().ok_or_else(|| {
        "Terminal list returned an invalid response; expected an array".to_owned()
    })?;
    if host_id.is_empty() || cwd.trim().is_empty() {
        return Ok(Vec::new());
    }
    Ok(sessions
        .iter()
        .filter(|session| {
            text(session, "status") == "running"
                && text(session, "hostId") == host_id
                && text(session, "cwd") == cwd
        })
        .take(MAX_TRACKED_TERMINALS)
        .map(|session| {
            let title: String = text(session, "title")
                .chars()
                .take(MAX_TERMINAL_TITLE_CHARS)
                .collect();
            TrackedTerminal {
                title: if title.is_empty() {
                    "Untitled terminal".into()
                } else {
                    title
                },
            }
        })
        .collect())
}

fn tracked_task_labels(tasks: &[Value], task_id: &str, host_id: &str, cwd: &str) -> Vec<String> {
    if host_id.is_empty() || cwd.trim().is_empty() {
        return Vec::new();
    }
    tasks
        .iter()
        .filter(|task| {
            text(task, "status") == "running"
                && text(task, "hostId") == host_id
                && text(task, "cwd") == cwd
        })
        .take(MAX_TRACKED_TERMINALS)
        .map(|task| {
            let provider = text(task, "provider");
            if text(task, "id") == task_id {
                format!(
                    "{} task",
                    if provider.is_empty() {
                        "Agent"
                    } else {
                        &provider
                    }
                )
            } else {
                let title = short_text(&text(task, "title"), 200);
                format!(
                    "{} task · {}",
                    if provider.is_empty() {
                        "Agent"
                    } else {
                        &provider
                    },
                    if title.is_empty() { "Untitled" } else { &title }
                )
            }
        })
        .collect()
}

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn rows(value: &Value, key: &str) -> Vec<Value> {
    value
        .get(key)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn snapshot_has_task(snapshot: &Value, task_id: &str) -> bool {
    !task_id.is_empty()
        && snapshot["tasks"]
            .as_array()
            .is_some_and(|tasks| tasks.iter().any(|task| text(task, "id") == task_id))
}

fn short_text(value: &str, limit: usize) -> String {
    let mut result: String = value.chars().take(limit).collect();
    if value.chars().count() > limit {
        result.push('…');
    }
    result
}

fn visible_work_plan_count(total: usize, limit: usize, expanded: bool) -> usize {
    if expanded { total } else { total.min(limit) }
}

fn work_plan_segment_color(status: &str, palette: Palette) -> u32 {
    match status {
        "completed" => 0x3eaa7a,
        "in_progress" => palette.accent,
        "blocked" => 0xc96058,
        "skipped" => mix_rgb(palette.panel, palette.muted, 0.55),
        _ => mix_rgb(palette.panel, palette.muted, 0.19),
    }
}

fn mix_rgb(base: u32, color: u32, amount: f32) -> u32 {
    let channel = |shift: u32| {
        let base = ((base >> shift) & 0xff) as f32;
        let color = ((color >> shift) & 0xff) as f32;
        (base * (1.0 - amount) + color * amount).round() as u32
    };
    (channel(16) << 16) | (channel(8) << 8) | channel(0)
}

fn local_time(timestamp: i64) -> String {
    crate::timestamps::local_datetime(timestamp)
        .map(|date| date.format("%-I:%M %P").to_string())
        .unwrap_or_default()
}

fn font_px(snapshot: &Value, design_size: f32) -> gpui_kit::Pixels {
    let interface_size = snapshot["settings"]["interfaceFontSize"]
        .as_f64()
        .filter(|size| size.is_finite() && *size > 0.)
        .unwrap_or(14.)
        .clamp(8., 32.) as f32;
    px(design_size * interface_size / 14.)
}

/// Merge newest-first API pages, preserving already loaded older history.
/// Incoming records are validated against the current task before retention.
fn merge_event_page(
    existing: &[Value],
    incoming: &[Value],
    task_id: &str,
    before: Option<i64>,
) -> Vec<Value> {
    let mut merged = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let ordered = if before.is_none() {
        incoming.iter().chain(existing).collect::<Vec<_>>()
    } else {
        existing.iter().chain(incoming).collect::<Vec<_>>()
    };
    for event in ordered {
        let id = text(event, "id");
        if text(event, "taskId") == task_id && !id.is_empty() && seen.insert(id) {
            merged.push(event.clone());
        }
    }
    merged.sort_by(|a, b| {
        b["createdAt"]
            .as_i64()
            .unwrap_or_default()
            .cmp(&a["createdAt"].as_i64().unwrap_or_default())
            .then_with(|| text(b, "id").cmp(&text(a, "id")))
    });
    merged
}

fn event_page(value: &Value) -> (Vec<Value>, Option<i64>) {
    let events = value["events"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .take(EVENT_PAGE_LIMIT as usize)
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    let next_before = value["nextBefore"].as_i64();
    (events, next_before)
}

fn event_page_args(task_id: &str, before: Option<i64>) -> Value {
    let mut args = json!({"taskId": task_id, "limit": EVENT_PAGE_LIMIT});
    if let Some(before) = before {
        args["before"] = json!(before);
    }
    args
}

#[cfg(test)]
mod tests {
    use super::{
        INITIAL_WORK_PLAN_HISTORY, INITIAL_WORK_PLAN_ITEMS, event_page_args, merge_event_page,
        snapshot_has_task, text, tracked_task_labels, tracked_terminal_page,
        visible_work_plan_count,
    };
    use serde_json::json;

    #[::core::prelude::v1::test]
    fn timeline_pages_deduplicate_overlaps_filter_tasks_and_sort_newest_first() {
        let existing = vec![
            json!({"id":"e2","taskId":"t","createdAt":20}),
            json!({"id":"e1","taskId":"t","createdAt":10}),
        ];
        let newest = vec![
            json!({"id":"e3","taskId":"t","createdAt":30}),
            json!({"id":"e2","taskId":"t","createdAt":20}),
            json!({"id":"foreign","taskId":"other","createdAt":40}),
        ];
        let merged = merge_event_page(&existing, &newest, "t", None);
        assert_eq!(
            merged
                .iter()
                .map(|event| text(event, "id"))
                .collect::<Vec<_>>(),
            ["e3", "e2", "e1"]
        );

        let older = vec![
            json!({"id":"e1","taskId":"t","createdAt":10}),
            json!({"id":"e0","taskId":"t","createdAt":0}),
        ];
        let merged = merge_event_page(&merged, &older, "t", Some(0));
        assert_eq!(
            merged
                .iter()
                .map(|event| text(event, "id"))
                .collect::<Vec<_>>(),
            ["e3", "e2", "e1", "e0"]
        );
    }

    #[::core::prelude::v1::test]
    fn first_timeline_page_omits_optional_cursor_and_older_page_includes_it() {
        let first = event_page_args("task-1", None);
        assert_eq!(first["taskId"], "task-1");
        assert_eq!(first["limit"], 100);
        assert!(first.get("before").is_none());

        let older = event_page_args("task-1", Some(42));
        assert_eq!(older["before"], 42);
    }

    #[::core::prelude::v1::test]
    fn tracked_process_projection_matches_host_folder_and_running_status() {
        let tasks = vec![
            json!({"id":"current","status":"running","hostId":"h","cwd":"/work","provider":"codex"}),
            json!({"id":"other","status":"running","hostId":"h","cwd":"/work","provider":"claude","title":"Review"}),
            json!({"id":"idle","status":"idle","hostId":"h","cwd":"/work","provider":"codex"}),
            json!({"id":"remote","status":"running","hostId":"x","cwd":"/work","provider":"codex"}),
            json!({"id":"folder","status":"running","hostId":"h","cwd":"/else","provider":"codex"}),
        ];
        assert_eq!(
            tracked_task_labels(&tasks, "current", "h", "/work"),
            ["codex task", "claude task · Review"]
        );
        let terminals = json!([
            {"status":"running","hostId":"h","cwd":"/work","title":"Shell"},
            {"status":"exited","hostId":"h","cwd":"/work","title":"Old"},
            {"status":"running","hostId":"x","cwd":"/work","title":"Remote"},
        ]);
        let projected = tracked_terminal_page(&terminals, "h", "/work").unwrap();
        assert_eq!(projected[0].title, "Shell");
        assert_eq!(projected.len(), 1);
        assert!(tracked_terminal_page(&json!({"sessions": []}), "h", "/work").is_err());
    }

    #[::core::prelude::v1::test]
    fn work_plan_lists_keep_bounded_initial_view_and_expand_to_all_records() {
        assert_eq!(
            visible_work_plan_count(82, INITIAL_WORK_PLAN_ITEMS, false),
            8
        );
        assert_eq!(
            visible_work_plan_count(82, INITIAL_WORK_PLAN_ITEMS, true),
            82
        );
        assert_eq!(
            visible_work_plan_count(4, INITIAL_WORK_PLAN_ITEMS, false),
            4
        );
        assert_eq!(
            visible_work_plan_count(9, INITIAL_WORK_PLAN_HISTORY, false),
            5
        );
        assert_eq!(
            visible_work_plan_count(9, INITIAL_WORK_PLAN_HISTORY, true),
            9
        );
    }

    #[::core::prelude::v1::test]
    fn task_navigation_requires_a_real_task_in_the_current_snapshot() {
        let snapshot = json!({"tasks":[{"id":"present"}]});
        assert!(snapshot_has_task(&snapshot, "present"));
        assert!(!snapshot_has_task(&snapshot, "missing"));
        assert!(!snapshot_has_task(&snapshot, ""));
    }
}

// Detail pages never render the conversation body. Do not retain its JSON
// payload in every closed or background inspector.
fn details_snapshot(snapshot: &Value) -> Value {
    let mut value = serde_json::Map::new();
    for key in ["settings", "tasks", "workPlans", "agents", "hosts", "projects", "collaborations"] {
        if let Some(row) = snapshot.get(key) {
            value.insert(key.to_owned(), row.clone());
        }
    }
    Value::Object(value)
}

pub struct PaneDetails {
    service: DesktopService,
    snapshot: Value,
    task_id: String,
    revision: Option<String>,
    generation: u64,
    request: u64,
    timeline_loaded: bool,
    timeline_loading: bool,
    timeline_error: Option<String>,
    pending_before: Option<i64>,
    next_before: Option<i64>,
    loaded_older: bool,
    events: Vec<Value>,
    event_rows: HashMap<String, Entity<RunActivityRow>>,
    terminal_generation: u64,
    terminal_request: u64,
    terminal_loaded: bool,
    terminal_loading: bool,
    terminal_error: Option<String>,
    terminal_scope: Option<(String, String)>,
    tracked_terminals: Vec<TrackedTerminal>,
    expanded_work_items: Option<String>,
    expanded_work_history: bool,
    expanded_identity: bool,
    expanded_work_notes: std::collections::HashSet<String>,
}

impl PaneDetails {
    pub fn new(
        service: DesktopService,
        snapshot: &Value,
        revision: Option<&str>,
        task_id: &str,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Self {
        let terminal_scope = snapshot["tasks"]
            .as_array()
            .and_then(|tasks| tasks.iter().find(|task| text(task, "id") == task_id))
            .map(|task| (text(task, "hostId"), text(task, "cwd")));
        Self {
            service,
            snapshot: details_snapshot(snapshot),
            task_id: task_id.to_owned(),
            revision: revision.map(str::to_owned),
            generation: 0,
            request: 0,
            timeline_loaded: false,
            timeline_loading: false,
            timeline_error: None,
            pending_before: None,
            next_before: None,
            loaded_older: false,
            events: Vec::new(),
            event_rows: HashMap::new(),
            terminal_generation: 0,
            terminal_request: 0,
            terminal_loaded: false,
            terminal_loading: false,
            terminal_error: None,
            terminal_scope,
            tracked_terminals: Vec::new(),
            expanded_work_items: None,
            expanded_work_history: false,
            expanded_identity: false,
            expanded_work_notes: Default::default(),
        }
    }

    /// Rebind the retained entity to the selected task. A task change clears
    /// task-owned history; a revision change fences in-flight reads. Loaded
    /// history remains visible, and refresh is user-triggered to avoid page
    /// requests on every routine snapshot revision.
    pub fn set_context(
        &mut self,
        snapshot: &Value,
        revision: Option<&str>,
        task_id: &str,
        cx: &mut Context<Self>,
    ) {
        let task_changed = self.task_id != task_id;
        let revision = revision.map(str::to_owned);
        let revision_changed = self.revision != revision;
        let next_scope = snapshot["tasks"]
            .as_array()
            .and_then(|tasks| tasks.iter().find(|task| text(task, "id") == task_id))
            .map(|task| (text(task, "hostId"), text(task, "cwd")));
        let terminal_scope_changed = self.terminal_scope != next_scope;
        self.snapshot = details_snapshot(snapshot);
        if task_changed {
            self.task_id = task_id.to_owned();
            self.generation = self.generation.wrapping_add(1);
            self.request = self.request.wrapping_add(1);
            self.timeline_loaded = false;
            self.timeline_loading = false;
            self.timeline_error = None;
            self.pending_before = None;
            self.next_before = None;
            self.loaded_older = false;
            self.events.clear();
            self.event_rows.clear();
            self.expanded_work_items = None;
            self.expanded_work_history = false;
            self.expanded_identity = false;
            self.expanded_work_notes.clear();
        } else if revision_changed {
            self.generation = self.generation.wrapping_add(1);
            self.request = self.request.wrapping_add(1);
            if self.timeline_loading {
                self.timeline_loading = false;
                self.timeline_error = Some(
                    "Timeline page became stale while the snapshot changed. Retry to refresh it."
                        .into(),
                );
            }
        }
        if task_changed || terminal_scope_changed {
            self.terminal_generation = self.terminal_generation.wrapping_add(1);
            self.terminal_request = self.terminal_request.wrapping_add(1);
            self.terminal_loaded = false;
            self.terminal_loading = false;
            self.terminal_error = None;
            self.tracked_terminals.clear();
        }
        self.terminal_scope = next_scope;
        self.revision = revision;
        cx.notify();
    }

    fn start_terminal_list(&mut self, cx: &mut Context<Self>) {
        if self.terminal_loading || self.task_id.is_empty() {
            return;
        }
        let Some((host_id, cwd)) = self.terminal_scope.clone() else {
            self.terminal_loaded = true;
            self.terminal_error = None;
            self.tracked_terminals.clear();
            cx.notify();
            return;
        };
        if host_id.is_empty() || cwd.trim().is_empty() {
            self.terminal_loaded = true;
            self.terminal_error = None;
            self.tracked_terminals.clear();
            cx.notify();
            return;
        }
        self.terminal_loading = true;
        self.terminal_error = None;
        self.terminal_request = self.terminal_request.wrapping_add(1);
        let request = self.terminal_request;
        let generation = self.terminal_generation;
        let task_id = self.task_id.clone();
        let service = self.service.clone();
        let (sender, receiver) = oneshot::channel();
        let worker_host = host_id.clone();
        let worker_cwd = cwd.clone();
        let worker = std::thread::Builder::new()
            .name("monitter-pane-run-summary".into())
            .stack_size(RUN_SUMMARY_WORKER_STACK_SIZE)
            .spawn(move || {
                let result = service
                    .invoke("list_terminals", json!({}))
                    .and_then(|sessions| {
                        tracked_terminal_page(&sessions, &worker_host, &worker_cwd)
                    });
                let _ = sender.send(result);
            });
        if let Err(error) = worker {
            self.terminal_loading = false;
            self.terminal_error = Some(format!("Could not start tracked process read: {error}"));
            cx.notify();
            return;
        }
        cx.spawn(async move |pane, cx| {
            let result = receiver
                .await
                .unwrap_or_else(|_| Err("Terminal list worker ended without a result".into()));
            let _ = pane.update(cx, |pane, cx| {
                if pane.terminal_generation != generation
                    || pane.terminal_request != request
                    || pane.task_id != task_id
                    || pane.terminal_scope.as_ref() != Some(&(host_id, cwd))
                {
                    return;
                }
                pane.terminal_loading = false;
                match result {
                    Ok(terminals) => {
                        pane.tracked_terminals = terminals;
                        pane.terminal_loaded = true;
                        pane.terminal_error = None;
                    }
                    Err(error) => {
                        pane.terminal_error = Some(error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn start_page(&mut self, before: Option<i64>, cx: &mut Context<Self>) {
        if self.timeline_loading || self.task_id.is_empty() {
            return;
        }
        self.timeline_loading = true;
        self.timeline_error = None;
        self.pending_before = before;
        self.request = self.request.wrapping_add(1);
        let request = self.request;
        let generation = self.generation;
        let task_id = self.task_id.clone();
        let revision = self.revision.clone();
        let service = self.service.clone();
        let (sender, receiver) = oneshot::channel();
        let worker_task = task_id.clone();
        let worker = std::thread::Builder::new()
            .name("monitter-pane-timeline".into())
            .stack_size(EVENT_WORKER_STACK_SIZE)
            .spawn(move || {
                let result =
                    service.invoke("get_task_events", event_page_args(&worker_task, before));
                let _ = sender.send(result);
            });
        if let Err(error) = worker {
            self.timeline_loading = false;
            self.timeline_error = Some(format!("Could not start Timeline worker: {error}"));
            cx.notify();
            return;
        }
        cx.spawn(async move |pane, cx| {
            let result = receiver
                .await
                .unwrap_or_else(|_| Err("Timeline worker ended without a result".into()));
            let _ = pane.update(cx, |pane, cx| {
                if pane.generation != generation
                    || pane.request != request
                    || pane.task_id != task_id
                    || pane.revision != revision
                {
                    return;
                }
                pane.timeline_loading = false;
                match result {
                    Ok(page) => {
                        let (incoming, next_before) = event_page(&page);
                        pane.events = merge_event_page(&pane.events, &incoming, &task_id, before);
                        pane.timeline_loaded = true;
                        pane.timeline_error = None;
                        if before.is_some() {
                            pane.loaded_older = true;
                        }
                        pane.next_before = if before.is_none() && pane.loaded_older {
                            pane.next_before
                        } else {
                            next_before
                        };
                    }
                    Err(error) => pane.timeline_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn render_tracked_processes(&mut self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let scope = self.terminal_scope.clone();
        let tasks = scope
            .as_ref()
            .map(|(host, cwd)| {
                tracked_task_labels(&rows(&self.snapshot, "tasks"), &self.task_id, host, cwd)
            })
            .unwrap_or_default();
        let has_tasks = !tasks.is_empty();
        let valid_scope = scope
            .as_ref()
            .is_some_and(|(host, cwd)| !host.is_empty() && !cwd.trim().is_empty());
        if !self.terminal_loaded && !self.terminal_loading && self.terminal_error.is_none() {
            self.start_terminal_list(cx);
        }
        let mut section = div()
            .w_full()
            .mt(px(14.))
            .pt(px(11.))
            .border_t_1()
            .border_color(rgb(palette.line))
            .flex()
            .flex_col()
            .gap(px(0.))
            .child(
                div()
                    .mt(px(2.))
                    .mb(px(7.))
                    .text_size(font_px(&self.snapshot, 10.))
                    .font_family(crate::design::FONT_MONO)
                    .text_color(rgb(palette.muted))
                    .child("TRACKED PROCESSES"),
            );
        for label in tasks {
            section = section.child(
                div()
                    .my(px(3.))
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .text_size(font_px(&self.snapshot, 11.))
                    .text_color(rgb(palette.ink))
                    .child(
                        Icon::new(IconName::Play)
                            .size(px(11.))
                            .text_color(rgb(palette.ink)),
                    )
                    .child(label),
            );
        }
        for terminal in &self.tracked_terminals {
            section = section.child(
                div()
                    .my(px(3.))
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .text_size(font_px(&self.snapshot, 11.))
                    .text_color(rgb(palette.ink))
                    .child(
                        Icon::new(IconName::SquareTerminal)
                            .size(px(11.))
                            .text_color(rgb(palette.ink)),
                    )
                    .child(format!("Terminal · {}", terminal.title)),
            );
        }
        if let Some(error) = self.terminal_error.clone() {
            let pane = cx.entity().clone();
            section = section
                .child(
                    div()
                        .text_size(font_px(&self.snapshot, 9.))
                        .text_color(rgb(0xb54a55))
                        .child(format!("Could not refresh terminals: {error}")),
                )
                .child(
                    Button::new("tracked-processes-retry")
                        .text()
                        .compact()
                        .xsmall()
                        .text_size(font_px(&self.snapshot, 9.))
                        .label("Retry")
                        .on_click(move |_, _, cx| {
                            let _ = pane.update(cx, |pane, cx| pane.start_terminal_list(cx));
                        }),
                );
        } else if self.terminal_loading {
            section = section.child(
                div()
                    .text_size(font_px(&self.snapshot, 9.))
                    .text_color(rgb(palette.muted))
                    .child("Checking tracked terminals…"),
            );
        } else if !valid_scope {
            section = section.child(
                div()
                    .text_size(font_px(&self.snapshot, 9.))
                    .text_color(rgb(palette.muted))
                    .child("Host and folder are required to match tracked processes."),
            );
        } else if !has_tasks && self.tracked_terminals.is_empty() {
            section = section.child(
                div()
                    .my(px(3.))
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .text_size(font_px(&self.snapshot, 11.))
                    .text_color(rgb(palette.muted))
                    .child(
                        Icon::new(IconName::SquareTerminal)
                            .size(px(11.))
                            .text_color(rgb(palette.muted)),
                    )
                    .child("No Monitter-tracked process in this folder."),
            );
        }
        if self.terminal_loaded {
            let pane = cx.entity().clone();
            section = section.child(
                Button::new("tracked-processes-refresh")
                    .text()
                    .compact()
                    .xsmall()
                    .text_size(font_px(&self.snapshot, 9.))
                    .label(if self.terminal_loading {
                        "Refreshing…"
                    } else {
                        "Refresh processes"
                    })
                    .disabled(self.terminal_loading)
                    .on_click(move |_, _, cx| {
                        let _ = pane.update(cx, |pane, cx| pane.start_terminal_list(cx));
                    }),
            );
        }
        section.into_any_element()
    }

    fn render_work_plans(&mut self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let mut plans: Vec<Value> = rows(&self.snapshot, "workPlans")
            .into_iter()
            .filter(|plan| text(plan, "taskId") == self.task_id)
            .collect();
        plans.sort_by(|a, b| {
            let active = text(&b, "status") == "active";
            let current = text(&a, "status") == "active";
            active.cmp(&current).then_with(|| {
                b["updatedAt"]
                    .as_i64()
                    .unwrap_or_default()
                    .cmp(&a["updatedAt"].as_i64().unwrap_or_default())
            })
        });
        let Some(plan) = plans.first() else {
            return div()
                .text_size(font_px(&self.snapshot, 10.))
                .line_height(relative(1.4))
                .text_color(rgb(palette.muted))
                .child("No work plan published for this chat yet.")
                .into_any_element();
        };
        let items = rows(plan, "items");
        let completed = items
            .iter()
            .filter(|item| text(item, "status") == "completed")
            .count();
        let in_progress = items
            .iter()
            .filter(|item| text(item, "status") == "in_progress")
            .count();
        let blocked = items
            .iter()
            .filter(|item| text(item, "status") == "blocked")
            .count();
        let skipped = items
            .iter()
            .filter(|item| text(item, "status") == "skipped")
            .count();
        let plan_id = text(plan, "id");
        let items_expanded = self.expanded_work_items.as_deref() == Some(plan_id.as_str());
        let visible_items =
            visible_work_plan_count(items.len(), INITIAL_WORK_PLAN_ITEMS, items_expanded);
        let mut card = div()
            .w_full()
            .p_3()
            .border_1()
            .border_color(rgb(palette.line))
            .rounded(px(7.))
            .bg(crate::design::surface_color(palette.panel))
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .text_size(font_px(&self.snapshot, 10.))
                            .font_family(crate::design::FONT_MONO)
                            .text_color(rgb(palette.muted))
                            .child("WORK PLAN"),
                    )
                    .child(
                        div()
                            .text_size(font_px(&self.snapshot, 10.))
                            .text_color(rgb(palette.accent))
                            .child(if text(plan, "status") == "active" {
                                "Active"
                            } else {
                                "Closed"
                            }),
                    ),
            )
            .child(
                div()
                    .text_size(font_px(&self.snapshot, 13.))
                    .text_color(rgb(palette.ink))
                    .child(text(plan, "title")),
            )
            .child(
                div()
                    .text_size(font_px(&self.snapshot, 10.))
                    .text_color(rgb(palette.muted))
                    .child(format!(
                        "{completed} of {} done{}{}{}",
                        items.len(),
                        if in_progress > 0 {
                            format!(" · {in_progress} in progress")
                        } else {
                            String::new()
                        },
                        if blocked > 0 {
                            format!(" · {blocked} blocked")
                        } else {
                            String::new()
                        },
                        if skipped > 0 {
                            format!(" · {skipped} skipped")
                        } else {
                            String::new()
                        },
                    )),
            );
        let mut progress = div()
            .id("work-plan-progress")
            .w_full()
            .h(px(5.))
            .mt(px(4.))
            .mb(px(4.))
            .overflow_hidden()
            .flex()
            .gap(px(2.))
            .rounded_full()
            .bg(crate::design::surface_color(mix_rgb(palette.panel, palette.muted, 0.15)))
            .role(gpui_kit::accesskit::Role::ProgressIndicator)
            .aria_label("Completed work-plan items")
            .aria_min_numeric_value(0.0)
            .aria_max_numeric_value(items.len() as f64)
            .aria_numeric_value(completed as f64);
        for (index, item) in items.iter().enumerate() {
            let status = text(item, "status");
            progress = progress.child(
                div()
                    .id(format!("work-plan-progress-item-{index}"))
                    .flex_1()
                    .min_w(px(0.))
                    .h(px(5.))
                    .rounded_full()
                    .bg(rgb(work_plan_segment_color(&status, palette)))
                    .aria_label(format!("{}: {}", text(item, "title"), status)),
            );
        }
        card = card.child(progress);
        for (index, item) in items.iter().take(visible_items).enumerate() {
            let status = text(item, "status");
            let note = text(item, "note");
            let item_key = format!("{plan_id}-{}-{index}", text(item, "id"));
            let expanded = self.expanded_work_notes.contains(&item_key);
            let color = work_plan_segment_color(&status, palette);
            let status_label = match status.as_str() {
                "completed" => "Done", "blocked" => "Blocked", "in_progress" => "In progress",
                "skipped" => "Skipped", _ => "Pending",
            };
            let icon = match status.as_str() {
                "completed" => IconName::Check, "blocked" => IconName::CircleAlert,
                "in_progress" => IconName::CircleDot, "skipped" => IconName::Minus, _ => IconName::Circle,
            };
            let mut body = div().w_full().flex().flex_col().gap(px(4.))
                .border_t_1().border_color(rgb(palette.line)).py(px(6.))
                .child(div().flex().items_start().gap(px(7.))
                    .child(Icon::new(icon).with_size(px(13.)).text_color(rgb(color)))
                    .child(div().flex_1().min_w_0().whitespace_normal()
                        .text_size(font_px(&self.snapshot, 11.)).text_color(rgb(palette.ink)).child(text(item, "title")))
                    .child(div().flex_shrink_0().text_size(font_px(&self.snapshot, 9.))
                        .text_color(rgb(color)).child(status_label)));
            if expanded && !note.is_empty() {
                body = body.child(div().ml(px(20.)).whitespace_normal()
                    .text_size(font_px(&self.snapshot, 10.)).text_color(rgb(palette.muted)).child(note.clone()));
            }
            if note.is_empty() {
                card = card.child(body);
            } else {
                let toggle_key = item_key.clone();
                card = card.child(Button::new(SharedString::from(format!("work-plan-note-{item_key}")))
                    .text().compact().w_full().h_auto().justify_start().p_0()
                    .accessibility_label(format!("{} notes for {}", if expanded { "Hide" } else { "Show" }, text(item, "title")))
                    .child(body).on_click(cx.listener(move |pane, _, _, cx| {
                        if !pane.expanded_work_notes.remove(&toggle_key) { pane.expanded_work_notes.insert(toggle_key.clone()); }
                        cx.notify();
                    })));
            }
        }
        if items.len() > INITIAL_WORK_PLAN_ITEMS {
            let pane = cx.entity().clone();
            let toggle_plan_id = plan_id.clone();
            let remaining = items.len().saturating_sub(visible_items);
            card = card.child(
                Button::new("work-plan-items-toggle")
                    .text()
                    .compact()
                    .xsmall()
                    .text_size(font_px(&self.snapshot, 9.))
                    .label(if items_expanded {
                        "Show fewer work-plan items".into()
                    } else {
                        format!("Show {remaining} more work-plan items")
                    })
                    .on_click(move |_, _, cx| {
                        let _ = pane.update(cx, |pane, cx| {
                            pane.expanded_work_items =
                                if pane.expanded_work_items.as_deref() == Some(&toggle_plan_id) {
                                    None
                                } else {
                                    Some(toggle_plan_id.clone())
                                };
                            cx.notify();
                        });
                    }),
            );
        }
        let summary = text(plan, "summary");
        if !summary.is_empty() {
            card = card.child(
                div()
                    .p_2()
                    .rounded(px(5.))
                    .bg(crate::design::surface_color(palette.soft))
                    .text_size(font_px(&self.snapshot, 10.))
                    .text_color(rgb(palette.ink))
                    .child(summary),
            );
        }
        if plans.len() > 1 {
            let earlier_count = plans.len() - 1;
            let history_visible = visible_work_plan_count(
                earlier_count,
                INITIAL_WORK_PLAN_HISTORY,
                self.expanded_work_history,
            );
            card = card.child(
                Button::new("work-plan-history-toggle").text().compact().w_full().justify_start()
                    .border_t_1().border_color(rgb(palette.line)).pt_2()
                    .text_size(font_px(&self.snapshot, 10.))
                    .icon(Icon::new(if self.expanded_work_history { IconName::ChevronDown } else { IconName::ChevronRight }).with_size(px(12.)))
                    .label(format!("Earlier plans · {earlier_count}"))
                    .accessibility_label(if self.expanded_work_history { "Collapse earlier plans" } else { "Expand earlier plans" })
                    .on_click(cx.listener(|pane, _, _, cx| {
                        pane.expanded_work_history = !pane.expanded_work_history;
                        cx.notify();
                    })),
            );
            for earlier in plans.iter().skip(1).take(if self.expanded_work_history { history_visible } else { 0 }) {
                let title = text(earlier, "title");
                let status = text(earlier, "status");
                let summary = short_text(&text(earlier, "summary"), 160);
                let mut row = div().w_full().flex().flex_col().gap_1().child(
                    div()
                        .flex()
                        .justify_between()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_size(font_px(&self.snapshot, 10.))
                                .text_color(rgb(palette.ink))
                                .child(title),
                        )
                        .child(
                            div()
                                .text_size(font_px(&self.snapshot, 9.))
                                .text_color(rgb(palette.muted))
                                .child(status),
                        ),
                );
                if !summary.is_empty() {
                    row = row.child(
                        div()
                            .text_size(font_px(&self.snapshot, 9.))
                            .text_color(rgb(palette.muted))
                            .child(summary),
                    );
                }
                card = card.child(row);
            }

        }
        card.into_any_element()
    }

    fn event_row_entities(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(_) = self.snapshot["tasks"]
            .as_array()
            .and_then(|tasks| tasks.iter().find(|task| text(task, "id") == self.task_id))
        else {
            self.event_rows.clear();
            return;
        };
        self.event_rows
            .retain(|id, _| self.events.iter().any(|event| text(event, "id") == *id));
        for event in &self.events {
            let id = text(event, "id");
            if id.is_empty() {
                continue;
            }
            if let Some(row) = self.event_rows.get(&id) {
                row.update(cx, |row, cx| row.set_event(&self.task_id, event, cx));
            } else {
                let service = self.service.clone();
                let task_id = self.task_id.clone();
                let event = event.clone();
                let row = cx.new(|cx| RunActivityRow::new(service, task_id, &event, window, cx));
                self.event_rows.insert(id, row);
            }
        }
    }

    pub fn render_details(
        &mut self,
        palette: Palette,
        git_summary: Option<AnyElement>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tasks = rows(&self.snapshot, "tasks");
        let task = tasks.iter().find(|task| text(task, "id") == self.task_id);
        let Some(task) = task else {
            return div()
                .text_size(font_px(&self.snapshot, 11.))
                .line_height(relative(1.4))
                .text_color(rgb(palette.muted))
                .child("Task details are unavailable.")
                .into_any_element();
        };
        let agent = rows(&self.snapshot, "agents")
            .into_iter()
            .find(|agent| text(&agent, "id") == text(task, "agentId"));
        let agent_name = agent
            .as_ref()
            .map(|agent| text(agent, "name"))
            .filter(|name| !name.is_empty())
            .or_else(|| {
                let archived = text(task, "archivedAgentName");
                (!archived.is_empty()).then_some(archived)
            })
            .unwrap_or_else(|| "Agent unavailable".into());
        let provider = text(task, "provider");
        let model = text(task, "model");
        let host_id = text(task, "hostId");
        let host_name = rows(&self.snapshot, "hosts")
            .into_iter()
            .find(|host| text(host, "id") == host_id)
            .map(|host| text(&host, "name"))
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "Unknown host".into());
        let project_id = text(task, "projectId");
        let project_name = rows(&self.snapshot, "projects")
            .into_iter()
            .find(|project| text(project, "id") == project_id)
            .map(|project| text(&project, "name"))
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "No project".into());
        let folder = text(task, "cwd");
        let folder = if folder.is_empty() {
            "No folder set".into()
        } else {
            folder
        };
        let permissions = match text(task, "sandbox").as_str() {
            "yolo" => "YOLO — skip permissions".to_owned(),
            "harness-configured" => "Harness permissions".to_owned(),
            other => other.to_owned(),
        };
        let codex_account = (provider == "codex").then(|| {
            let home = text(task, "codexHome");
            if home.is_empty() {
                "Inherited default account".into()
            } else {
                home
            }
        });
        let native_session = text(task, "nativeSessionId");
        let identity_image = agent
            .as_ref()
            .and_then(crate::chat::agent_avatar)
            .or_else(|| crate::branding::provider_for_ink(&provider, &model, palette.ink));
        let provider_icon = crate::branding::provider_for_ink(&provider, &model, palette.ink);
        let provider_summary = format!(
            "{provider}{}",
            if model.is_empty() {
                String::new()
            } else {
                format!(" · {model}")
            }
        );
        let avatar = if let Some(image) = identity_image {
            div()
                .w(px(32.))
                .h(px(32.))
                .flex_shrink_0()
                .rounded(px(7.))
                .overflow_hidden()
                .bg(crate::design::surface_color(palette.soft))
                .child(img(image).w(px(32.)).h(px(32.)))
        } else {
            div()
                .w(px(32.))
                .h(px(32.))
                .flex_shrink_0()
                .rounded(px(7.))
                .flex()
                .items_center()
                .justify_center()
                .bg(crate::design::surface_color(palette.soft))
                .text_color(rgb(palette.ink))
                .child(
                    Icon::new(IconName::Bot)
                        .size(px(16.))
                        .text_color(rgb(palette.ink)),
                )
        };
        let header = div()
            .w_full()
            .flex()
            .items_center()
            .gap(px(9.))
            .child(avatar)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .text_size(font_px(&self.snapshot, 12.))
                            .text_color(rgb(palette.ink))
                            .child(agent_name),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .text_size(font_px(&self.snapshot, 10.))
                            .font_family(crate::design::FONT_MONO)
                            .text_color(rgb(palette.muted))
                            .when_some(provider_icon, |this, image| {
                                this.child(img(image).w(px(12.)).h(px(12.)))
                            })
                            .child(provider_summary),
                    ),
            )
            .child(Button::new("pane-identity-info").ghost().compact()
                .icon(Icon::new(IconName::Info).with_size(px(14.)))
                .accessibility_label(if self.expanded_identity { "Hide task information" } else { "Show task information" })
                .on_click(cx.listener(|pane, _, _, cx| { pane.expanded_identity = !pane.expanded_identity; cx.notify(); })));
        let description = agent.as_ref().map(|agent| text(agent, "description"));
        let mut identity = div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(6.))
            .pb(px(8.))
            .mb(px(10.))
            .border_b_1()
            .border_color(rgb(palette.line))
            .child(header);
        if let Some(description) = description.filter(|description| self.expanded_identity && !description.trim().is_empty())
        {
            identity = identity.child(
                div()
                    .ml(px(41.))
                    .text_size(font_px(&self.snapshot, 11.))
                    .line_height(relative(1.4))
                    .text_color(rgb(palette.muted))
                    .child(description),
            );
        }
        let detail_row = |label: &str, value: String, multiline: bool| {
            let mut value_element = div()
                .flex_1()
                .min_w_0()
                .text_size(font_px(&self.snapshot, 11.))
                .text_color(rgb(palette.ink));
            if multiline {
                value_element = value_element.whitespace_normal().child(value);
            } else {
                value_element = value_element
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(value);
            }
            div()
                .w_full()
                .flex()
                .items_start()
                .gap(px(8.))
                .py(px(3.))
                .border_b_1()
                .border_color(rgb(palette.line))
                .child(
                    div()
                        .w(px(84.))
                        .flex_shrink_0()
                        .text_size(font_px(&self.snapshot, 10.))
                        .font_family(crate::design::FONT_MONO)
                        .text_color(rgb(palette.muted))
                        .child(label.to_uppercase()),
                )
                .child(value_element)
        };
        let mut definition_list = div().w_full().flex().flex_col().gap(px(3.));
        definition_list = definition_list
            .child(detail_row("Harness", provider, false))
            .child(detail_row(
                "Model",
                if model.is_empty() {
                    "Harness default".into()
                } else {
                    model
                },
                false,
            ))
            .child(detail_row("Host", host_name, false));
        if let Some(account) = codex_account {
            definition_list = definition_list.child(detail_row("Codex account", account, false));
        }
        definition_list = definition_list
            .child(detail_row("Folder", folder, true))
            .child(detail_row("Project", project_name, false))
            .child(detail_row("Permissions", permissions, false));
        if !native_session.is_empty() {
            definition_list =
                definition_list.child(detail_row("Native session", native_session, false));
        }
        let mut root = div()
            .flex()
            .flex_col()
            .text_size(font_px(&self.snapshot, 11.))
            .line_height(relative(1.4))
            .gap(px(8.))
            .child(identity)
            .when(self.expanded_identity, |root| root.child(definition_list));
        if let Some(git_summary) = git_summary {
            root = root.child(git_summary);
        }
        root = root
            .child(self.render_work_plans(palette, cx))
            .child(self.render_tracked_processes(palette, cx));

        let collaborations: Vec<Value> = rows(&self.snapshot, "collaborations")
            .into_iter()
            .filter(|item| {
                text(item, "fromTaskId") == self.task_id || text(item, "toTaskId") == self.task_id
            })
            .collect();
        let related_tasks: Vec<Value> = tasks
            .into_iter()
            .filter(|item| text(item, "parentTaskId") == self.task_id)
            .collect();
        let agents = rows(&self.snapshot, "agents");
        let mut collaboration_section = div()
            .w_full()
            .mt(px(21.))
            .pt(px(15.))
            .border_t_1()
            .border_color(rgb(palette.line))
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .flex()
                    .gap(px(5.))
                    .mb(px(7.))
                    .text_size(font_px(&self.snapshot, 10.))
                    .font_family(crate::design::FONT_MONO)
                    .text_color(rgb(palette.muted))
                    .child("COLLABORATION")
                    .child(
                        div()
                            .px(px(4.))
                            .rounded(px(3.))
                            .bg(crate::design::surface_color(palette.soft))
                            .child(collaborations.len().to_string()),
                    ),
            );
        if collaborations.is_empty() {
            collaboration_section = collaboration_section.child(
                div()
                    .text_size(font_px(&self.snapshot, 10.))
                    .font_family(crate::design::FONT_MONO)
                    .text_color(rgb(palette.muted))
                    .child("No routed agent messages or delegations yet."),
            );
        } else {
            for item in collaborations.iter().take(40) {
                let outbound = text(item, "fromTaskId") == self.task_id;
                let agent_id = text(item, if outbound { "toAgentId" } else { "fromAgentId" });
                let agent_name = agents
                    .iter()
                    .find(|agent| text(agent, "id") == agent_id)
                    .map(|agent| text(agent, "name"))
                    .filter(|name| !name.is_empty())
                    .unwrap_or_else(|| "Agent".into());
                let kind = if text(item, "kind") == "delegation" {
                    "Delegation"
                } else {
                    "Agent message"
                };
                let mut row = div()
                    .w_full()
                    .border_b_1()
                    .border_color(rgb(palette.line))
                    .py(px(7.))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(font_px(&self.snapshot, 11.))
                                    .text_color(rgb(palette.ink))
                                    .child(format!("{kind} · {agent_name}")),
                            )
                            .child(
                                div()
                                    .text_size(font_px(&self.snapshot, 9.))
                                    .text_color(rgb(palette.muted))
                                    .child(text(item, "status")),
                            ),
                    );
                let summary = if !text(item, "result").is_empty() {
                    text(item, "result")
                } else if !text(item, "error").is_empty() {
                    text(item, "error")
                } else {
                    text(item, "text")
                };
                if !summary.is_empty() {
                    row = row.child(
                        div()
                            .text_size(font_px(&self.snapshot, 10.))
                            .text_color(rgb(palette.muted))
                            .child(short_text(&summary, 240)),
                    );
                }
                let linked_task_id = if outbound {
                    text(item, "toTaskId")
                } else {
                    text(item, "fromTaskId")
                };
                if snapshot_has_task(&self.snapshot, &linked_task_id) {
                    let open_task_id = linked_task_id.clone();
                    collaboration_section = collaboration_section.child(
                        Button::new(SharedString::from(format!(
                            "pane-collaboration-{}",
                            text(item, "id")
                        )))
                        .text()
                        .compact()
                        .w_full()
                        .h_auto()
                        .justify_start()
                        .items_start()
                        .p_0()
                        .accessibility_label(format!("Open related task with {agent_name}"))
                        .child(row)
                        .on_click(cx.listener(move |pane, _, _, cx| {
                            cx.emit(PaneDetailsEvent::OpenTask(open_task_id.clone()));
                            cx.notify();
                        })),
                    );
                } else {
                    let row = row.child(
                        div()
                            .text_size(font_px(&self.snapshot, 9.))
                            .text_color(rgb(palette.muted))
                            .child("Linked task unavailable"),
                    );
                    collaboration_section = collaboration_section.child(
                        Button::new(SharedString::from(format!(
                            "pane-collaboration-unavailable-{}",
                            text(item, "id")
                        )))
                        .text()
                        .compact()
                        .w_full()
                        .h_auto()
                        .justify_start()
                        .items_start()
                        .p_0()
                        .disabled(true)
                        .accessibility_label("Linked task unavailable")
                        .child(row),
                    );
                }
            }
        }
        root = root.child(collaboration_section);

        let mut delegated_section = div()
            .w_full()
            .mt(px(21.))
            .pt(px(15.))
            .border_t_1()
            .border_color(rgb(palette.line))
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .flex()
                    .gap(px(5.))
                    .mb(px(7.))
                    .text_size(font_px(&self.snapshot, 10.))
                    .font_family(crate::design::FONT_MONO)
                    .text_color(rgb(palette.muted))
                    .child("DELEGATED TASKS")
                    .child(
                        div()
                            .px(px(4.))
                            .rounded(px(3.))
                            .bg(crate::design::surface_color(palette.soft))
                            .child(related_tasks.len().to_string()),
                    ),
            );
        if related_tasks.is_empty() {
            delegated_section = delegated_section.child(
                div()
                    .text_size(font_px(&self.snapshot, 10.))
                    .text_color(rgb(palette.muted))
                    .child("No delegated tasks."),
            );
        } else {
            for related in related_tasks.iter().take(40) {
                let agent_name = agents
                    .iter()
                    .find(|agent| text(agent, "id") == text(related, "agentId"))
                    .map(|agent| text(agent, "name"))
                    .filter(|name| !name.is_empty())
                    .unwrap_or_else(|| "Agent unavailable".into());
                let related_id = text(related, "id");
                let row = div()
                    .w_full()
                    .border_b_1()
                    .border_color(rgb(palette.line))
                    .py(px(7.))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_size(font_px(&self.snapshot, 11.))
                            .text_color(rgb(palette.ink))
                            .child(text(related, "title")),
                    )
                    .child(
                        div()
                            .text_size(font_px(&self.snapshot, 9.))
                            .text_color(rgb(palette.muted))
                            .child(format!("{agent_name} · {}", text(related, "status"))),
                    );
                if snapshot_has_task(&self.snapshot, &related_id) {
                    let open_task_id = related_id.clone();
                    delegated_section = delegated_section.child(
                        Button::new(SharedString::from(format!(
                            "pane-delegated-task-{related_id}"
                        )))
                        .text()
                        .compact()
                        .w_full()
                        .h_auto()
                        .justify_start()
                        .items_start()
                        .p_0()
                        .accessibility_label(format!(
                            "Open delegated task {}",
                            text(related, "title")
                        ))
                        .child(row)
                        .on_click(cx.listener(move |pane, _, _, cx| {
                            cx.emit(PaneDetailsEvent::OpenTask(open_task_id.clone()));
                            cx.notify();
                        })),
                    );
                } else {
                    delegated_section = delegated_section.child(
                        Button::new(SharedString::from(format!(
                            "pane-delegated-unavailable-{related_id}"
                        )))
                        .text()
                        .compact()
                        .w_full()
                        .h_auto()
                        .justify_start()
                        .items_start()
                        .p_0()
                        .disabled(true)
                        .accessibility_label("Delegated task unavailable")
                        .child(
                            row.child(
                                div()
                                    .text_size(font_px(&self.snapshot, 9.))
                                    .text_color(rgb(palette.muted))
                                    .child("Task unavailable"),
                            ),
                        ),
                    );
                }
            }
        }
        let delegate_task_id = self.task_id.clone();
        delegated_section = delegated_section.child(
            Button::new("pane-delegate-task")
                .text()
                .compact()
                .icon(Icon::new(IconName::Plus))
                .label("Delegate a task")
                .on_click(cx.listener(move |pane, _, _, cx| {
                    cx.emit(PaneDetailsEvent::DelegateTask(delegate_task_id.clone()));
                    cx.notify();
                })),
        );
        root.child(delegated_section).into_any_element()
    }

    pub fn render_timeline(
        &mut self,
        palette: Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if !self.timeline_loaded && !self.timeline_loading && self.timeline_error.is_none() {
            self.start_page(None, cx);
        }
        self.event_row_entities(window, cx);
        let loading = self.timeline_loading;
        let has_more = self.next_before.is_some();
        let events = self.events.clone();
        let rows_by_id = self.event_rows.clone();
        let error = self.timeline_error.clone();
        let retry_before = self.pending_before;
        let mut root = div()
            .id("pane-timeline-content")
            .flex()
            .flex_col()
            .gap_1()
            .text_size(font_px(&self.snapshot, 11.))
            .line_height(relative(1.4));
        if let Some(error) = error {
            let pane = cx.entity().clone();
            root = root.child(
                div()
                    .w_full()
                    .text_color(rgb(0xb54a55))
                    .child(format!("Could not load full activity: {error}"))
                    .child(
                        Button::new("timeline-retry")
                            .text()
                            .compact()
                            .xsmall()
                            .text_size(font_px(&self.snapshot, 9.5))
                            .label("Retry")
                            .disabled(loading)
                            .on_click(move |_, _, cx| {
                                let _ =
                                    pane.update(cx, |pane, cx| pane.start_page(retry_before, cx));
                            }),
                    ),
            );
        }
        if loading && events.is_empty() {
            root = root.child(
                div()
                    .text_size(font_px(&self.snapshot, 11.))
                    .text_color(rgb(palette.muted))
                    .child("Loading full activity…"),
            );
        }
        if events.is_empty() && !loading && self.timeline_error.is_none() {
            root = root.child(
                div()
                    .text_size(font_px(&self.snapshot, 11.))
                    .text_color(rgb(palette.muted))
                    .child("Run events will appear here as the CLI reports them."),
            );
        }
        for event in events {
            let id = text(&event, "id");
            let stamp = local_time(event["createdAt"].as_i64().unwrap_or_default());
            let border = palette.line;
            let mut body = div().flex_1().min_w_0().flex().flex_col().gap_1();
            if let Some(row) = rows_by_id.get(&id) {
                body = body.child(row.clone());
            }
            let item = div()
                .w_full()
                .flex()
                .gap_2()
                .py_2()
                .border_b_1()
                .border_color(rgb(border))
                .child(
                    div()
                        .w(px(43.))
                        .flex_shrink_0()
                        .text_size(font_px(&self.snapshot, 9.))
                        .font_family(crate::design::FONT_MONO)
                        .text_color(rgb(palette.muted))
                        .child(stamp),
                )
                .child(body);
            root = root.child(item);
        }
        if self.timeline_loaded {
            let pane = cx.entity().clone();
            root = root.child(
                Button::new("timeline-refresh")
                    .w_full()
                    .label(if loading {
                        "Refreshing…"
                    } else {
                        "Refresh activity"
                    })
                    .disabled(loading)
                    .on_click(move |_, _, cx| {
                        let _ = pane.update(cx, |pane, cx| pane.start_page(None, cx));
                    }),
            );
        }
        if has_more {
            let pane = cx.entity().clone();
            let before = self.next_before;
            root = root.child(
                Button::new("timeline-load-older")
                    .w_full()
                    .label(if loading {
                        "Loading…"
                    } else {
                        "Load older activity"
                    })
                    .disabled(loading)
                    .on_click(move |_, _, cx| {
                        let _ = pane.update(cx, |pane, cx| pane.start_page(before, cx));
                    }),
            );
        }
        root.into_any_element()
    }
}

impl EventEmitter<PaneDetailsEvent> for PaneDetails {}
