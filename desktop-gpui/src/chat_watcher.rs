//! Pane-local, read-only overview of authoritative work already in progress.
use super::*;
use gpui_kit::base::Selectable as _;
use gpui_kit::component::{ThemeStyled as _, popover::Popover};

pub(super) fn watcher_surface(palette: design::Palette) -> gpui_kit::Rgba {
    design::surface_color(mix_rgb(
        palette.paper,
        palette.panel,
        if palette.dark { 0.28 } else { 0.55 },
    ))
}

fn watcher_mode_button(
    id: &'static str,
    label: &'static str,
    enabled: bool,
    palette: design::Palette,
    ratio: f32,
    window: &mut Window,
    cx: &mut App,
) -> Button {
    let hover = gpui_kit::component::hover_motion::HoverMotion::new(
        SharedString::from(format!("{id}-ink")), enabled, window, cx,
    );
    let primary = cx.theme().primary;
    let foreground = hover.color("foreground", rgb(palette.ink).into(), primary, window, cx);
    let glow = hover.color("glow", primary.opacity(0.), primary.opacity(0.32), window, cx);
    Button::new(id)
        .disabled(!enabled)
        .compact()
        .xsmall()
        .custom(gpui_kit::component::button::ButtonCustomVariant::new(cx).foreground(foreground))
        .h_auto()
        .px(px(12.))
        .py(px(5.))
        .rounded_full()
        .text_size(px(11. * ratio))
        .line_height(px(16. * ratio))
        .icon(Icon::new(IconName::MessageSquare).shadow(vec![
            gpui_kit::BoxShadow::new(px(0.), px(0.), glow).blur_radius(px(5.)),
        ]))
        .label(label)
        .on_hover(move |hovered, _, cx| hover.on_hover(*hovered, cx))
}

// Build state comes from the queue, not the agent's narrative or task status.
// Include combined requests, but never attribute another project's build to this pane.
fn watcher_running_build<'a>(snapshot: &'a Value, task_id: &str) -> Option<&'a Value> {
    let project = values(snapshot, "tasks")
        .find(|task| task["id"] == task_id)?["projectId"].as_str()?;
    values(snapshot, "buildRuns").find(|run| {
        run["status"] == "running" && run["projectId"] == project
            && (run["ownerTaskId"] == task_id || values(snapshot, "buildRequests").any(|request| {
                request["taskId"] == task_id && request["projectId"] == project
                    && run["requestIds"].as_array().is_some_and(|ids| ids.contains(&request["id"]))
            }))
    })
}

fn watcher_pending_build<'a>(snapshot: &'a Value, task_id: &str) -> Option<&'a Value> {
    let project = values(snapshot, "tasks")
        .find(|task| task["id"] == task_id)?["projectId"].as_str()?;
    values(snapshot, "buildRequests").filter(|request| {
        request["taskId"] == task_id && request["projectId"] == project
            && matches!(request["status"].as_str(), Some("ready" | "planned"))
    }).max_by_key(|request| (request["status"] == "ready", request["createdAt"].as_i64().unwrap_or(0)))
}

#[derive(Default)]
pub(super) struct WatcherState {
    pub enabled: bool,
    pub(super) modal_open: bool,
    modal_generation: u64,
    modal_sparkles: Option<Entity<crate::activity_sparkles::SparkleField>>,
    modal_tokens: std::collections::BTreeSet<u64>,
    pub(super) interject_open: bool,
    interjection: Option<InterjectionExchange>,
    scroll: ScrollHandle,
    content_width: Pixels,
    finished_expanded: bool,
    activity_observer: Option<Subscription>,
    finished_plan_expanded: bool,
    pub(super) transitioning: bool,
    pub(super) fade_in: bool,
    pub(super) transition_serial: u64,
    transition_task: Option<Task<()>>,
}

#[derive(Clone, Debug)]
struct InterjectionExchange {
    text: String,
    wire_text: String,
    known_message_ids: Vec<String>,
    assistant_baseline: Vec<(String, String, String)>,
    known_queue_ids: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
struct InterjectionProjection {
    status: String,
    response_id: Option<String>,
    response_text: Option<String>,
}

impl InterjectionExchange {
    fn capture(text: String, wire_text: String, snapshot: &Value, messages: &[Rc<Value>]) -> Self {
        Self {
            text,
            wire_text,
            known_message_ids: messages
                .iter()
                .map(|message| field(message, "id"))
                .collect(),
            assistant_baseline: messages
                .iter()
                .filter(|message| field(message, "role") == "assistant")
                .map(|message| {
                    (
                        field(message, "id"),
                        field(message, "text"),
                        field(message, "streamStatus"),
                    )
                })
                .collect(),
            known_queue_ids: values(snapshot, "queuedMessages")
                .map(|message| field(message, "id"))
                .collect(),
        }
    }

    fn project(
        &self,
        snapshot: &Value,
        task_id: &str,
        messages: &[Rc<Value>],
    ) -> InterjectionProjection {
        let queued = values(snapshot, "queuedMessages")
            .filter(|message| {
                message["taskId"] == task_id
                    && field(message, "text") == self.wire_text
                    && !self.known_queue_ids.contains(&field(message, "id"))
            })
            .last();
        let acknowledged = messages.iter().any(|message| {
            field(message, "role") == "user"
                && field(message, "senderAgentId").is_empty()
                && field(message, "text") == self.wire_text
                && !self.known_message_ids.contains(&field(message, "id"))
        });
        let response = acknowledged.then(|| {
            messages.iter().rev().find(|message| {
                if field(message, "role") != "assistant" {
                    return false;
                }
                let id = field(message, "id");
                let text = field(message, "text");
                let stream_status = field(message, "streamStatus");
                self.assistant_baseline.iter().all(|baseline| {
                    baseline.0 != id || baseline.1 != text || baseline.2 != stream_status
                })
            })
        }).flatten();
        let (response_id, response_text) = response
            .map(|message| (Some(field(message, "id")), Some(field(message, "text"))))
            .unwrap_or_default();
        let status = if let Some(message) = response {
            if field(message, "streamStatus") == "streaming" {
                "Receiving reply…".into()
            } else {
                "Reply received".into()
            }
        } else if acknowledged {
            "Steered · waiting for reply…".into()
        } else if let Some(queued) = queued {
            match field(queued, "status").as_str() {
                "sending" if field(queued, "origin") == "steering" => {
                    "Steering into the current turn…".into()
                }
                "queued" => "Steer unavailable · queued for the next turn".into(),
                "error" => queued["error"]
                    .as_str()
                    .filter(|error| !error.trim().is_empty())
                    .unwrap_or("The steer could not be sent.")
                    .to_owned(),
                _ => "Sending steer…".into(),
            }
        } else {
            "Sending steer…".into()
        };
        InterjectionProjection {
            status,
            response_id,
            response_text,
        }
    }
}

fn values<'a>(snapshot: &'a Value, key: &str) -> impl Iterator<Item = &'a Value> {
    snapshot[key].as_array().into_iter().flatten()
}
fn at(value: &Value, key: &str) -> i64 {
    value[key].as_i64().unwrap_or_default()
}
fn active(status: &str) -> bool {
    matches!(
        status,
        "running" | "queued" | "pending" | "starting" | "in_progress"
    )
}
fn status_label(status: &str) -> &str {
    match status {
        "running" | "in_progress" => "Running",
        "completed" | "succeeded" => "Completed",
        "error" | "failed" => "Failed",
        "interrupted" | "cancelled" => "Stopped",
        "queued" | "pending" | "starting" => "Queued",
        "blocked" => "Blocked",
        "idle" => "Idle",
        _ => "Unknown",
    }
}
fn mix_rgb(base: u32, color: u32, amount: f32) -> u32 {
    let channel = |shift: u32| {
        let base = ((base >> shift) & 0xff) as f32;
        let color = ((color >> shift) & 0xff) as f32;
        (base * (1. - amount) + color * amount).round() as u32
    };
    (channel(16) << 16) | (channel(8) << 8) | channel(0)
}
fn event_time(event: &Value) -> String {
    crate::timestamps::local_datetime(at(event, "createdAt"))
        .map(|time| time.format("%H:%M:%S").to_string())
        .unwrap_or_default()
}

fn watcher_activity_icon(kind: &str) -> IconName {
    match kind.to_ascii_lowercase().as_str() {
        "tool" => IconName::Wrench,
        "computer" | "computer_image" => IconName::Monitor,
        "usage" => IconName::Activity,
        "reasoning" | "jevdecision" => IconName::Brain,
        "subagent" => IconName::Bot,
        "output" => IconName::FileText,
        "agent_progress" | "message" | "collaboration" => IconName::MessageSquare,
        "mail" => IconName::Mail,
        "goal" => IconName::Goal,
        "error" | "warning" => IconName::TriangleAlert,
        "status" => IconName::CircleDot,
        "schedule" => IconName::Clock,
        _ => IconName::Activity,
    }
}

fn compact_activity_text(text: &str) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() > 180 {
        format!("{}…", text.chars().take(179).collect::<String>())
    } else {
        text
    }
}

fn detail_text(value: &Value, keys: &[&str], depth: usize) -> Option<String> {
    if depth > 6 {
        return None;
    }
    match value {
        Value::String(text) => {
            let text = text.trim();
            if text.is_empty() {
                return None;
            }
            if text.len() <= 256 * 1024
                && (text.starts_with('{') || text.starts_with('['))
                && let Ok(nested) = serde_json::from_str::<Value>(text)
            {
                return detail_text(&nested, keys, depth + 1);
            }
            Some(compact_activity_text(text))
        }
        Value::Array(values) => values
            .iter()
            .find_map(|value| detail_text(value, keys, depth + 1)),
        Value::Object(object) => keys
            .iter()
            .filter_map(|key| object.get(*key))
            .find_map(|value| detail_text(value, keys, depth + 1))
            .or_else(|| {
                [
                    "detail",
                    "payload",
                    "activity",
                    "update",
                    "input",
                    "arguments",
                    "search_query",
                    "image_query",
                    "open",
                    "find",
                    "questions",
                    "result",
                    "output",
                    "data",
                    "item",
                ]
                .iter()
                .filter_map(|key| object.get(*key))
                .find_map(|value| detail_text(value, keys, depth + 1))
            }),
        _ => None,
    }
}

fn watcher_event_detail(event: &Value) -> Value {
    let parsed = super::run_activity::parsed_event_detail(event);
    if parsed.is_null() {
        let raw = field(event, "detail");
        if raw.trim().is_empty() { Value::Null } else { Value::String(raw) }
    } else {
        parsed
    }
}

fn number_at(value: &Value, pointers: &[&str]) -> Option<f64> {
    pointers
        .iter()
        .filter_map(|pointer| value.pointer(pointer))
        .find_map(Value::as_f64)
        .filter(|value| value.is_finite() && *value >= 0.)
}

fn grouped_integer(value: f64) -> String {
    let digits = format!("{:.0}", value);
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(character);
    }
    grouped
}

fn watcher_usage_text(event: &Value) -> Option<String> {
    let detail = watcher_event_detail(event);
    let context_used = number_at(&detail, &["/usage/used", "/used"]);
    let context_size = number_at(&detail, &["/usage/size", "/size"]);
    if let Some(used) = context_used {
        if let Some(size) = context_size.filter(|size| *size > 0.) {
            return Some(format!(
                "{} / {} context ({:.0}%)",
                grouped_integer(used),
                grouped_integer(size),
                (used / size * 100.).clamp(0., 999.)
            ));
        }
        return Some(format!("{} context used", grouped_integer(used)));
    }
    let input = number_at(&detail, &[
        "/input_tokens", "/inputTokens", "/input", "/tokens/input",
        "/tokens/input_tokens", "/usage/input_tokens", "/usage/inputTokens",
    ]);
    let output = number_at(&detail, &[
        "/output_tokens", "/outputTokens", "/output", "/tokens/output",
        "/tokens/output_tokens", "/usage/output_tokens", "/usage/outputTokens",
    ]);
    let reasoning = number_at(&detail, &[
        "/reasoning_tokens", "/reasoningTokens", "/tokens/reasoning",
        "/tokens/reasoning_tokens", "/usage/reasoning_tokens",
    ]);
    let total = number_at(&detail, &[
        "/total_tokens", "/totalTokens", "/total", "/tokens/total",
        "/tokens/total_tokens", "/usage/total_tokens", "/usage/totalTokens",
    ]);
    let cost = number_at(&detail, &["/cost", "/cost_usd", "/total_cost_usd", "/usage/cost"]);
    let mut parts = Vec::new();
    if let Some(value) = input {
        parts.push(format!("{} input", grouped_integer(value)));
    }
    if let Some(value) = output {
        parts.push(format!("{} output", grouped_integer(value)));
    }
    if let Some(value) = reasoning.filter(|value| *value > 0.) {
        parts.push(format!("{} reasoning", grouped_integer(value)));
    }
    if parts.is_empty() && let Some(value) = total {
        parts.push(format!("{} tokens", grouped_integer(value)));
    }
    if let Some(value) = cost {
        let precision = if value >= 0.01 { 2 } else { 4 };
        parts.push(format!("${value:.precision$}"));
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

fn watcher_decision_text(event: &Value) -> Option<String> {
    let detail = watcher_event_detail(event);
    let response = detail.get("response")?;
    let candidate = response.get("candidateId").and_then(Value::as_str);
    let outcome = response.get("outcome").and_then(Value::as_str);
    let reason = response.get("reason").and_then(Value::as_str);
    let confidence = response.get("confidence").and_then(Value::as_f64);
    let selection = outcome.map(str::to_owned).or_else(|| {
        candidate.map(|candidate| {
            detail
                .get("labels")
                .and_then(|labels| labels.get(candidate))
                .and_then(Value::as_str)
                .unwrap_or(candidate)
                .to_owned()
        })
    });
    let mut parts = vec!["System One".to_owned()];
    if candidate == Some("abstain") {
        parts.push("abstained".to_owned());
        if let Some(reason) = reason {
            parts.push(reason.to_owned());
        }
    } else if let Some(selection) = selection {
        parts.push(selection);
        if let Some(confidence) = confidence {
            parts.push(format!("{:.0}% confidence", confidence * 100.));
        }
    } else if let Some(reason) = reason {
        parts.push(reason.to_owned());
    } else {
        return None;
    }
    Some(compact_activity_text(&parts.join(" · ")))
}

fn friendly_tool_name(value: &str) -> String {
    let value = value.rsplit("__").next().unwrap_or(value);
    let words = value
        .replace(['_', '-'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let mut characters = words.chars();
    characters
        .next()
        .map(|first| first.to_uppercase().collect::<String>() + characters.as_str())
        .unwrap_or_default()
}

fn watcher_tool_text(event: &Value) -> String {
    let title = compact_activity_text(&field(event, "title"));
    let intent = super::run_activity::tool_intent(event);
    let mut preview = super::run_activity::process_tool_preview(std::slice::from_ref(event))
        .unwrap_or_else(|| intent.clone());
    if preview == intent || preview == "Used a tool" {
        let detail = watcher_event_detail(event);
        if let Some(argument) = detail_text(
            &detail,
            &[
                "cmd", "command", "path", "file_path", "query", "q", "pattern", "url",
                "title", "question", "objective", "message", "prompt",
            ],
            0,
        ) {
            preview = argument;
        }
    }
    if preview != intent && preview != "Used a tool" {
        let text = match intent.as_str() {
            "Ran a command" => format!("Ran {preview}"),
            "Read a file" => format!("Read {preview}"),
            "Edited a file" => format!("Edited {preview}"),
            "Searched the web" | "Searched files" => format!("Searched for {preview}"),
            "Fetched a web page" => format!("Fetched {preview}"),
            "Listed files" => format!("Listed {preview}"),
            "Started an agent" => format!("Started agent: {preview}"),
            "Used a tool" => {
                let name = friendly_tool_name(&title);
                if name.is_empty() || name.eq_ignore_ascii_case("tool activity") {
                    preview
                } else {
                    format!("{name} · {preview}")
                }
            }
            _ => format!("{intent} · {preview}"),
        };
        return compact_activity_text(&text);
    }
    if intent != "Used a tool" {
        intent
    } else if !title.is_empty()
        && !matches!(title.to_ascii_lowercase().as_str(), "tool activity" | "sub-agent activity")
    {
        title
    } else {
        intent
    }
}

fn watcher_activity_text(event: &Value) -> String {
    let kind = field(event, "kind");
    let title = compact_activity_text(&field(event, "title"));
    let detail = watcher_event_detail(event);
    let projected = match kind.to_ascii_lowercase().as_str() {
        "tool" | "subagent" => Some(watcher_tool_text(event)),
        "usage" => watcher_usage_text(event),
        "jevdecision" => watcher_decision_text(event),
        "reasoning" => detail_text(&detail, &["summary", "text", "thinking", "reasoning"], 0),
        "computer" | "computer_image" => detail_text(&detail, &["summary", "context", "action", "url"], 0),
        "output" => detail_text(&detail, &["text", "content", "summary"], 0),
        "error" | "warning" => detail_text(&detail, &["message", "error", "detail", "summary"], 0)
            .map(|detail| if title.is_empty() || title.eq_ignore_ascii_case(&detail) {
                detail
            } else {
                format!("{title} · {detail}")
            }),
        "goal" => detail_text(&detail, &["objective", "goal", "summary", "text"], 0),
        _ => None,
    };
    projected
        .filter(|text| !text.is_empty())
        .or_else(|| (!title.is_empty()).then_some(title))
        .unwrap_or_else(|| "Activity update".to_owned())
}

fn agent_duration(record: &Value) -> String {
    let started = at(record, "createdAt");
    let ended = if active(&field(record, "status")) {
        chrono::Utc::now().timestamp_millis()
    } else {
        at(record, "updatedAt").max(started)
    };
    let seconds = ended.saturating_sub(started).max(0) / 1_000;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}
/// Elapsed run time of the watched task itself, matching the subagent rows.
fn task_duration(task: &Value) -> String {
    let started = at(task, "createdAt");
    let ended = if active(&field(task, "status")) {
        chrono::Utc::now().timestamp_millis()
    } else {
        at(task, "updatedAt").max(started)
    };
    let seconds = ended.saturating_sub(started).max(0) / 1_000;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

// Intrinsic heights are shared only among Watch panes in the same window.
// The measured child never receives the shared minimum, so shrinking content
// can shrink the row again instead of feeding the old height back into itself.
struct WatchCardMeasurement {
    window: gpui_kit::WindowId,
    owner: WeakEntity<ChatPane>,
    height: Pixels,
}
thread_local! {
    static WATCH_CARD_HEIGHTS: std::cell::RefCell<Vec<WatchCardMeasurement>> = const { std::cell::RefCell::new(Vec::new()) };
}
fn watcher_plan_height(window: &Window) -> Pixels {
    let id = window.window_handle().window_id();
    WATCH_CARD_HEIGHTS.with(|entries| entries.borrow().iter()
        .filter(|entry| entry.window == id)
        .map(|entry| entry.height).fold(px(0.), Pixels::max))
}
fn forget_watch_card(owner: &WeakEntity<ChatPane>, cx: &mut App) {
    WATCH_CARD_HEIGHTS.with(|entries| {
        let mut entries = entries.borrow_mut();
        entries.retain(|entry| &entry.owner != owner);
        for entry in entries.iter() {
            if let Some(pane) = entry.owner.upgrade() { cx.notify(pane.entity_id()); }
        }
    });
}
fn measure_watch_card(owner: WeakEntity<ChatPane>, height: Pixels, window: &mut Window, cx: &mut App) {
    let id = window.window_handle().window_id();
    window.defer(cx, move |_, cx| {
        WATCH_CARD_HEIGHTS.with(|entries| {
            let mut entries = entries.borrow_mut();
            let before = entries.iter().filter(|entry| entry.window == id)
                .map(|entry| entry.height).fold(px(0.), Pixels::max);
            entries.retain(|entry| entry.owner.upgrade().is_some_and(|pane| pane.read(cx).watcher.enabled));
            if let Some(entry) = entries.iter_mut().find(|entry| entry.owner == owner) {
                entry.window = id;
                entry.height = height;
            } else if owner.upgrade().is_some() {
                entries.push(WatchCardMeasurement { window: id, owner, height });
            }
            let after = entries.iter().filter(|entry| entry.window == id)
                .map(|entry| entry.height).fold(px(0.), Pixels::max);
            if (after - before).abs() >= px(0.5) {
                for entry in entries.iter().filter(|entry| entry.window == id) {
                    if let Some(pane) = entry.owner.upgrade() { cx.notify(pane.entity_id()); }
                }
            }
        });
    });
}

#[derive(Debug)]
struct WatcherProjection {
    plan: Option<Value>,
    /// True only while this chat has a plan the agent is still working through.
    /// A finished plan never occupies the plan card; it stays a disclosure below
    /// the task summary so the surface keeps describing work in progress.
    plan_active: bool,
    finished_plan: Option<Value>,
    title: String,
    provider: String,
    model: String,
    duration: String,
    request: Option<String>,
    agents: Vec<Value>,
    running_agents: usize,
    status: String,
    progress: Option<String>,
    latest_tool: Option<String>,
    activity: Vec<Value>,
    attention: usize,
}

impl WatcherProjection {
    fn new(
        snapshot: &Value,
        task_id: &str,
        records: &[Value],
        messages: &[Rc<Value>],
    ) -> Self {
        let task = values(snapshot, "tasks")
            .find(|task| task["id"] == task_id)
            .cloned();
        let status = task
            .as_ref()
            .map(|task| field(task, "status"))
            .unwrap_or_default();
        let user_at = if snapshot.get("nativeMessageContext").is_some() {
            snapshot["nativeMessageContext"][task_id]["latestUserAt"].as_i64()
        } else {
            values(snapshot, "messages")
                .filter(|row| row["taskId"] == task_id && row["role"] == "user")
                .map(|row| at(row, "createdAt"))
                .max()
        };
        let plan = values(snapshot, "workPlans")
            .filter(|plan| plan["taskId"] == task_id)
            .max_by_key(|plan| {
                (
                    plan["status"] == "active",
                    at(plan, "updatedAt"),
                    at(plan, "createdAt"),
                )
            })
            .cloned();
        let plan_active = plan
            .as_ref()
            .is_some_and(|plan| plan["status"] == "active");
        let finished_plan = values(snapshot, "workPlans")
            .filter(|plan| plan["taskId"] == task_id && plan["status"] != "active")
            .max_by_key(|plan| (at(plan, "updatedAt"), at(plan, "createdAt")))
            .cloned();
        // The summary describes what this chat is doing now, so it is the latest
        // request the owner sent. Delegated and steered peer traffic is not it.
        let request = messages
            .iter()
            .filter(|message| {
                field(message, "role") == "user"
                    && field(message, "senderAgentId").is_empty()
                    && !field(message, "text").trim().is_empty()
            })
            .max_by_key(|message| at(message, "createdAt"))
            .map(|message| {
                field(message, "text")
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .filter(|text| !text.is_empty());
        // Keep the plan's team visible across turns, including results. Always
        // retain still-active children even if they started before this plan.
        let since = plan
            .as_ref()
            .map(|plan| at(plan, "createdAt"))
            .unwrap_or(user_at.unwrap_or_default());
        let mut agents: Vec<_> = records
            .iter()
            .filter(|record| {
                active(&field(record, "status"))
                    || at(record, "updatedAt").max(at(record, "createdAt")) >= since
            })
            .cloned()
            .collect();
        agents.sort_by_key(|record| {
            (
                !active(&field(record, "status")),
                at(record, "createdAt"),
                field(record, "id"),
            )
        });
        let running_agents = agents
            .iter()
            .filter(|record| active(&field(record, "status")))
            .count();
        let latest = |kinds: &[&str]| {
            values(snapshot, "events")
                .filter(|event| {
                    event["taskId"] == task_id
                        && user_at.is_some_and(|start| at(event, "createdAt") >= start)
                        && kinds.contains(&event["kind"].as_str().unwrap_or_default())
                })
                .max_by_key(|event| at(event, "createdAt"))
                .and_then(|event| event["title"].as_str())
                .filter(|text| !text.trim().is_empty())
                .map(str::to_owned)
        };
        let mut activity: Vec<_> = values(snapshot, "events")
            .filter(|event| {
                event["taskId"] == task_id
                    && user_at.is_some_and(|start| at(event, "createdAt") >= start)
            })
            .cloned()
            .collect();
        activity.sort_by_key(|event| std::cmp::Reverse(at(event, "createdAt")));
        activity.truncate(8);
        let attention = values(snapshot, "approvalRequests")
            .filter(|request| request["taskId"] == task_id && request["status"] == "pending")
            .count();
        Self {
            plan,
            plan_active,
            finished_plan,
            title: task
                .as_ref()
                .map(|task| field(task, "title"))
                .unwrap_or_default(),
            provider: task
                .as_ref()
                .map(|task| field(task, "provider"))
                .unwrap_or_default(),
            model: task
                .as_ref()
                .map(|task| field(task, "model"))
                .unwrap_or_default(),
            duration: task.as_ref().map(task_duration).unwrap_or_default(),
            request,
            agents,
            running_agents,
            status,
            progress: latest(&["agent_progress"]),
            latest_tool: latest(&["tool", "computer"]),
            activity,
            attention,
        }
    }
}

impl ChatPane {
    pub(crate) fn begin_activity_watch_modal(&mut self, cx: &mut Context<Self>) -> u64 {
        self.watcher.modal_generation = self.watcher.modal_generation.wrapping_add(1);
        self.watcher.modal_tokens.insert(self.watcher.modal_generation);
        self.watcher.modal_open = true;
        self.activity.update(cx, |activity, cx| activity.set_watch_mode(true, cx));
        cx.notify();
        self.watcher.modal_generation
    }
    pub(crate) fn release_activity_watch_modal(&mut self, generation: u64, cx: &mut Context<Self>) {
        if !self.watcher.modal_tokens.remove(&generation) { return; }
        self.watcher.modal_open = !self.watcher.modal_tokens.is_empty();
        if !self.watcher.modal_open {
            self.watcher.interject_open = false;
            self.watcher.modal_sparkles = None;
        }
        self.activity.update(cx, |activity, cx| activity.set_watch_mode(self.watcher.modal_open || self.watcher.enabled, cx));
        cx.notify();
    }
    pub(crate) fn render_activity_watch_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let palette = design::palette(&self.snapshot, window.appearance());
        let ratio = self.snapshot["settings"]["interfaceFontSize"].as_f64().unwrap_or(14.).clamp(8., 32.) as f32 / 14.;
        let inset = match field(&self.snapshot["settings"], "interfaceDensity").as_str() {
            "tight" => space::TIGHT, "spacious" => space::SECTION, _ => space::INSET,
        };
        self.activity.update(cx, |activity, cx| {
            activity.set_appearance(palette, ratio, inset, cx);
            activity.set_watch_mode(true, cx);
        });
        let form = self.render_user_questions(window, cx);
        self.render_watcher(palette, Some(form), window, cx)
    }
    /// Overview navigation always opens Watch, including during an outgoing animation.
    pub(crate) fn open_watcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.watcher.enabled && !self.watcher.transitioning {
            return;
        }
        self.watcher.transition_task = None;
        self.watcher.transition_serial = self.watcher.transition_serial.wrapping_add(1);
        self.watcher.transitioning = false;
        self.watcher.fade_in = false;
        self.watcher.enabled = true;
        window.focus(&self.focus, cx);
        self.slash.update(cx, |slash, cx| {
            slash.handle_action(crate::slash_palette::PaletteAction::Dismiss, cx);
        });
        cx.notify();
    }

    /// Save the requested mode during the outgoing half of a mode animation.
    pub(crate) fn persisted_watch_mode(&self) -> bool {
        if self.watcher.transitioning && !self.watcher.fade_in {
            !self.watcher.enabled
        } else {
            self.watcher.enabled
        }
    }

    /// Layout restoration settles both directions without changing keyboard focus.
    pub(crate) fn restore_watch_mode(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.watcher.transition_task = None;
        self.watcher.transition_serial = self.watcher.transition_serial.wrapping_add(1);
        self.watcher.transitioning = false;
        self.watcher.fade_in = false;
        self.watcher.enabled = enabled;
        self.watcher.interject_open = false;
        if !enabled { forget_watch_card(&cx.weak_entity(), cx); }
        self.activity.update(cx, |activity, cx| {
            activity.set_watch_mode(enabled || self.watcher.modal_open, cx);
        });
        cx.notify();
    }

    #[cfg(test)]
    pub(crate) fn watcher_is_open(&self) -> bool { self.watcher.enabled }

    pub(super) fn toggle_watcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.watcher.transitioning {
            return;
        }
        let target = !self.watcher.enabled;
        if !target {
            self.watcher.interject_open = false;
        }
        self.watcher.transitioning = true;
        self.watcher.fade_in = false;
        self.watcher.transition_serial = self.watcher.transition_serial.wrapping_add(1);
        if target {
            window.focus(&self.focus, cx);
            self.slash.update(cx, |slash, cx| {
                slash.handle_action(crate::slash_palette::PaletteAction::Dismiss, cx);
            });
        } else if self.pending_tool_approval().is_some() {
            self.approval_popup_open = true;
            window.focus(&self.approval_focus, cx);
        } else {
            window.focus(&self.composer.read(cx).focus_handle(cx), cx);
        }
        self.watcher.transition_task = Some(cx.spawn(async move |pane, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(300))
                .await;
            if pane
                .update(cx, |pane, cx| {
                    pane.watcher.enabled = target;
                    if !target { forget_watch_card(&cx.weak_entity(), cx); }
                    pane.watcher.fade_in = true;
                    cx.notify();
                })
                .is_err()
            {
                return;
            }
            cx.background_executor()
                .timer(Duration::from_millis(300))
                .await;
            let _ = pane.update(cx, |pane, cx| {
                pane.watcher.transitioning = false;
                pane.watcher.fade_in = false;
                pane.watcher.transition_task = None;
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(super) fn send_watcher_interjection(&mut self, cx: &mut Context<Self>) {
        if self.busy || self.has_pending_user_question() || self.is_performance_fixture() || self.attachments.read(cx).pending() {
            return;
        }
        let text = self.composer.read(cx).value().to_string();
        let attachment_ids = self.attachments.read(cx).ids();
        if text.trim().is_empty() && attachment_ids.is_empty() {
            return;
        }
        let wire_text = chat_slash::literal_prompt(&text);
        let messages = self
            .transcripts
            .get(&self.task_id())
            .map(|state| state.messages.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        self.watcher.interjection = Some(InterjectionExchange::capture(
            text,
            wire_text,
            &self.snapshot,
            &messages,
        ));
        self.slash.update(cx, |slash, cx| {
            slash.handle_action(crate::slash_palette::PaletteAction::Dismiss, cx);
        });
        self.send_with_delivery_mode(Some("steer"), false, cx);
        cx.notify();
    }

    /// Read-only summary of the watched task, shown in the plan slot whenever
    /// this chat has no plan in progress. A finished plan stays reachable below
    /// it as a disclosure so nothing already delivered is hidden away.
    fn render_watcher_task_summary(
        &mut self,
        state: &WatcherProjection,
        palette: design::Palette,
        ratio: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let title = if state.title.trim().is_empty() {
            "Untitled task".to_owned()
        } else {
            state.title.clone()
        };
        let status = status_label(&state.status);
        let mut meta = vec![status.to_owned()];
        if !state.duration.is_empty() {
            meta.push(state.duration.clone());
        }
        if !state.provider.is_empty() {
            meta.push(state.provider.clone());
        }
        if !state.model.is_empty() {
            meta.push(state.model.clone());
        }
        let status_color = if matches!(state.status.as_str(), "error" | "failed" | "blocked") {
            palette.err()
        } else if active(&state.status) {
            palette.accent
        } else {
            palette.muted
        };
        let request = state.request.clone().unwrap_or_else(|| {
            "No request has been sent in this chat yet.".to_owned()
        });
        let mut surface = div()
            .id("watcher-task-summary")
            .debug_selector(|| "watcher-task-summary".into())
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(12.))
            .p(px(16.))
            .text_color(rgb(palette.ink))
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .flex()
                    .items_start()
                    .gap(px(10.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .child(
                                div()
                                    .text_size(px(10. * ratio))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(palette.muted))
                                    .child("CURRENT TASK"),
                            )
                            .child(
                                div()
                                    .text_size(px(14. * ratio))
                                    .font_weight(FontWeight::MEDIUM)
                                    .truncate()
                                    .child(title),
                            ),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .gap(px(5.))
                            .px(px(7.))
                            .py(px(3.))
                            .rounded(px(4.))
                            .bg(design::surface_color(palette.card))
                            .child(div().size(px(5.)).rounded_full().bg(rgb(status_color)))
                            .child(
                                div()
                                    .text_size(px(10. * ratio))
                                    .text_color(rgb(palette.muted))
                                    .child(status.to_owned()),
                            ),
                    ),
            )
            .child(
                div()
                    .font_family(design::FONT_MONO)
                    .text_size(px(10. * ratio))
                    .text_color(rgb(palette.muted))
                    .truncate()
                    .child(meta.join(" · ")),
            )
            .child(
                div()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(5.))
                    .child(
                        div()
                            .text_size(px(11. * ratio))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.muted))
                            .child("CURRENT REQUEST"),
                    )
                    .child(
                        div()
                            .id("watcher-task-request")
                            .debug_selector(|| "watcher-task-request".into())
                            .min_w_0()
                            .whitespace_normal()
                            .line_clamp(5)
                            .text_size(px(12. * ratio))
                            .text_color(rgb(if state.request.is_some() {
                                palette.ink
                            } else {
                                palette.muted
                            }))
                            .child(request),
                    ),
            );
        if let Some(plan) = state.finished_plan.clone() {
            let expanded = self.watcher.finished_plan_expanded;
            let items = values(&plan, "items").count();
            let done = values(&plan, "items")
                .filter(|item| item["status"] == "completed")
                .count();
            let plan_title = field(&plan, "title");
            let heading = if plan_title.trim().is_empty() {
                "Last plan".to_owned()
            } else {
                format!("Last plan · {plan_title}")
            };
            surface = surface.child(
                div()
                    .id("watcher-last-plan")
                    .debug_selector(|| "watcher-last-plan".into())
                    .w_full()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .pt(px(4.))
                    .border_t_1()
                    .border_color(rgb(palette.line))
                    .child(
                        Button::new("watcher-last-plan-toggle")
                            .compact()
                            .xsmall()
                            .ghost()
                            .justify_start()
                            .h(px(22.))
                            .px(px(2.))
                            .icon(
                                Icon::new(if expanded {
                                    IconName::ChevronDown
                                } else {
                                    IconName::ChevronRight
                                })
                                .size(px(10.)),
                            )
                            .label(format!("{heading} · {done}/{items} steps"))
                            .debug_selector(|| "watcher-last-plan-toggle".into())
                            .accessibility_label(if expanded {
                                "Hide the last finished work plan"
                            } else {
                                "Show the last finished work plan"
                            })
                            .on_click(cx.listener(|pane, _, _, cx| {
                                pane.watcher.finished_plan_expanded =
                                    !pane.watcher.finished_plan_expanded;
                                cx.notify();
                            })),
                    )
                    .when(expanded, |section| {
                        section.child(
                            div()
                                .id("watcher-last-plan-card")
                                .debug_selector(|| "watcher-last-plan-card".into())
                                .w_full()
                                .min_w_0()
                                .child(crate::plans::render_plan_card(
                                    &plan, palette, ratio, true,
                                )),
                        )
                    }),
            );
        }
        surface.into_any_element()
    }

    fn render_watcher_interject_menu(
        &mut self,
        palette: design::Palette,
        ratio: f32,
        width: Pixels,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let task_id = self.task_id();
        let messages = self
            .transcripts
            .get(&task_id)
            .map(|state| state.messages.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        let exchange = self.watcher.interjection.clone();
        let projection = exchange
            .as_ref()
            .map(|exchange| exchange.project(&self.snapshot, &task_id, &messages));
        let response = projection.as_ref().and_then(|projection| {
            projection
                .response_id
                .as_ref()
                .zip(projection.response_text.as_ref())
        });
        let response_body = response.map(|(id, text)| {
            self.streaming_bodies
                .get(id)
                .cloned()
                .map(IntoElement::into_any_element)
                .unwrap_or_else(|| {
                    gpui_kit::component::text::TextView::markdown(
                        SharedString::from(format!("watcher-interject-response-{id}")),
                        text.clone(),
                    )
                    .selectable(true)
                    .scrollable(false)
                    .into_any_element()
                })
        });
        let attachment_count = self.attachments.read(cx).ids().len();
        let draft_empty = self.composer.read(cx).value().trim().is_empty();
        let pending_attachment = self.attachments.read(cx).pending();
        let can_send = !self.busy && !pending_attachment && (!draft_empty || attachment_count > 0);
        let error = self.error.clone();
        let font_family = cx.theme().font_family.clone();
        let danger = cx.theme().danger;
        div()
            .id("watcher-interject-menu")
            .debug_selector(|| "watcher-interject-menu".into())
            .w(width)
            .max_h(px(430.))
            .popover_style(cx)
            .overflow_hidden()
            .font_family(font_family.clone())
            .text_color(rgb(palette.ink))
            .when_some(exchange, |surface, exchange| {
                surface.child(
                    div()
                        .id("watcher-interject-exchange")
                        .debug_selector(|| "watcher-interject-exchange".into())
                        .max_h(px(250.))
                        .overflow_y_scroll()
                        .flex()
                        .flex_col()
                        .gap(px(9.))
                        .px(px(12.))
                        .py(px(11.))
                        .child(
                            div()
                                .flex()
                                .justify_end()
                                .child(
                                    div()
                                        .max_w(px(310.))
                                        .px(px(10.))
                                        .py(px(8.))
                                        .rounded(px(10.))
                                        .bg(design::surface_color(mix_rgb(
                                            palette.panel,
                                            palette.accent,
                                            if palette.dark { 0.22 } else { 0.12 },
                                        )))
                                        .text_size(px(12. * ratio))
                                        .child(exchange.text),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .justify_start()
                                .child(
                                    div()
                                        .id("watcher-interject-reply")
                                        .debug_selector(|| "watcher-interject-reply".into())
                                        .max_w(px(326.))
                                        .px(px(10.))
                                        .py(px(8.))
                                        .rounded(px(10.))
                                        .border_1()
                                        .border_color(rgb(palette.line))
                                        .bg(design::surface_color(palette.paper))
                                        .text_size(px(12. * ratio))
                                        .when_some(response_body, |bubble, body| {
                                            bubble.child(body)
                                        })
                                        .when(response.is_none(), |bubble| {
                                            bubble.child(
                                                div()
                                                    .text_color(rgb(palette.muted))
                                                    .child(
                                                        projection
                                                            .as_ref()
                                                            .map(|projection| {
                                                                projection.status.clone()
                                                            })
                                                            .unwrap_or_else(|| {
                                                                "Sending steer…".into()
                                                            }),
                                                    ),
                                            )
                                        }),
                                ),
                        )
                        .when(response.is_some(), |exchange| {
                            exchange.child(
                                div()
                                    .pl(px(2.))
                                    .text_size(px(10. * ratio))
                                    .text_color(rgb(palette.muted))
                                    .child(
                                        projection
                                            .as_ref()
                                            .map(|projection| projection.status.clone())
                                            .unwrap_or_default(),
                                    ),
                            )
                        }),
                )
            })
            .when_some(error, |surface, error| {
                surface.child(
                    div()
                        .px(px(12.))
                        .pt(px(9.))
                        .text_size(px(10. * ratio))
                        .text_color(danger)
                        .child(error),
                )
            })
            .child(
                div()
                    .p(px(12.))
                    .child(
                        div()
                            .id("watcher-interject-composer")
                            .debug_selector(|| "watcher-interject-composer".into())
                            .flex()
                            .items_end()
                            .gap(px(8.))
                            .p(px(design::space::INSET))
                            .rounded(px(design::radius::CONTROL))
                            .border_1()
                            .border_color(design::surface_color(palette.line))
                            .bg(crate::surface_blend::raised_background(
                                crate::surface_blend::Area::Composer,
                                palette.card,
                            ))
                            .child(
                                Textarea::new(&self.composer)
                                    .small()
                                    .appearance(false)
                                    .bordered(false)
                                    .font_family(font_family)
                                    .text_size(px(12. * ratio))
                                    .min_h(px(design::COMPOSER_CONTROL_SIZE))
                                    .max_h(px(108.))
                                    .flex_1()
                                    .disabled(self.busy),
                            )
                            .child(
                                Button::new("watcher-interject-send")
                                    .debug_selector(|| "watcher-interject-send".into())
                                    .outline()
                                    .compact()
                                    .w(px(design::COMPOSER_CONTROL_SIZE))
                                    .h(px(design::COMPOSER_CONTROL_SIZE))
                                    .border_color(rgb(palette.accent))
                                    .text_color(rgb(palette.accent))
                                    .icon(Icon::new(if self.busy {
                                        IconName::LoaderCircle
                                    } else {
                                        IconName::CornerDownLeft
                                    }))
                                    .accessibility_label(if self.busy {
                                        "Sending steer"
                                    } else {
                                        "Send steer"
                                    })
                                    .tooltip("Send steer")
                                    .disabled(!can_send)
                                    .on_click(cx.listener(|pane, _, _, cx| {
                                        pane.send_watcher_interjection(cx)
                                    })),
                            ),
                    )
                    .when(attachment_count > 0, |footer| {
                        footer.child(
                            div()
                                .pt(px(5.))
                                .text_size(px(10. * ratio))
                                .text_color(rgb(palette.muted))
                                .child(format!(
                                    "{attachment_count} attachment{} included",
                                    if attachment_count == 1 { "" } else { "s" }
                                )),
                        )
                    }),
            )
            .into_any_element()
    }

    fn render_watcher_agent_row(
        &self,
        index: usize,
        record: &Value,
        palette: design::Palette,
        ratio: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = field(record, "id");
        let title = field(record, "title");
        let name = field(record, "agentName");
        let status = field(record, "status");
        let activity = field(record, "activity");
        let duration = agent_duration(record);
        let child_id = field(record, "taskId");
        let can_open = !child_id.is_empty()
            && values(&self.snapshot, "tasks").any(|task| task["id"] == child_id);
        let color = if active(&status) {
            palette.accent
        } else if matches!(status.as_str(), "error" | "failed" | "blocked") {
            palette.err()
        } else {
            palette.muted
        };
        let mut row = div()
            .id(SharedString::from(format!("watcher-agent-{id}")))
            .debug_selector(move || format!("watcher-agent-row-{index}").into())
            .w_full()
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(8.))
            .py(px(5.))
            .child(div().size(px(6.)).rounded_full().bg(rgb(color)))
            .child(
                div().min_w_0().max_w(relative(0.55)).child(
                    div()
                        .text_size(px(11. * ratio))
                        .font_weight(FontWeight::MEDIUM)
                        .truncate()
                        .child(if title.is_empty() { name.clone() } else { title.clone() }),
                ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(10. * ratio))
                    .text_color(rgb(palette.muted))
                    .child(if activity.is_empty() {
                        status_label(&status).to_owned()
                    } else {
                        activity.clone()
                    }),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .font_family(design::FONT_MONO)
                    .text_size(px(9. * ratio))
                    .text_color(rgb(palette.muted))
                    .child(duration),
            );
        if can_open {
            let modal_task = child_id.clone();
            row = row.child(
                Button::new(SharedString::from(format!("watcher-open-{id}")))
                    .compact()
                    .xsmall()
                    .ghost()
                    .size(px(18.))
                    .p_0()
                    .icon(Icon::new(IconName::Info).size(px(11.)))
                    .accessibility_label(format!("Show work plan for {title}"))
                    .on_click(cx.listener(move |pane, _, window, cx| {
                        pane.open_peer_workplan(modal_task.clone(), window, cx);
                    })),
            );
        }
        row.into_any_element()
    }

    pub(super) fn render_watcher_toggle(
        &self,
        palette: design::Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let button = Button::new("chat-watcher-toggle")
            .compact()
            .xsmall()
            .ghost()
            .px(px(10.))
            .h(px(24.))
            .icon(
                Icon::default()
                    .data(gpui_kit::assets::__private::Eye.1)
                    .with_size(px(11.)),
            )
            .label("Watch")
            .debug_selector(|| "chat-watcher-toggle".into())
            .accessibility_label("Show workplan and subagents")
            .tooltip("Watch workplan, subagents and activity")
            .on_click(cx.listener(|pane, _, window, cx| {
                pane.toggle_watcher(window, cx);
            }));
        div()
            .id("chat-watch-mode-dock")
            .debug_selector(|| "chat-watch-mode-dock".into())
            .absolute()
            // The dock is positioned relative to the status-bar container,
            // so its fixed-height center stays on that container's top rule.
            .top(px(-12.))
            .left_0()
            .right_0()
            .h(px(24.))
            .px(px(12.))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .id("chat-watch-mode-toggle")
                    .debug_selector(|| "chat-watch-mode-toggle".into())
                    .flex()
                    .items_center()
                    .rounded_full()
                    .border_1()
                    .border_color(rgb(palette.line))
                    .bg(crate::surface_blend::floating_control_background(
                        palette.panel,
                        palette.dark,
                    ))
                    .child(button),
            )
            .into_any_element()
    }

    pub(super) fn render_watcher(
        &mut self,
        palette: design::Palette,
        user_question_form: Option<AnyElement>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.watcher.activity_observer.is_none() {
            self.watcher.activity_observer = Some(cx.observe(&self.activity, |_, _, cx| cx.notify()));
        }
        let records = self.companion.read(cx).records();
        let messages: Vec<Rc<Value>> = self
            .transcripts
            .get(&self.task_id())
            .map(|state| state.messages.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        let state = WatcherProjection::new(&self.snapshot, &self.task_id(), &records, &messages);
        let ratio = self.snapshot["settings"]["interfaceFontSize"]
            .as_f64()
            .unwrap_or(14.)
            .clamp(8., 32.) as f32
            / 14.;
        // Use this pane's measured geometry, not the whole workspace window:
        // adjacent portrait panes must not inherit their window's landscape mode.
        // Keep at least ~300px per column after the outer insets and gutter.
        let width = self.watcher.content_width / design::ui_scaled_px(1.);
        let modal = self.watcher.modal_open;
        let working = modal && state.status == "running";
        let motion = !cx.reduce_motion() && crate::native_motion::motion_allowed();
        // A modal owns its decoration: a hidden docked pane must not obscure it.
        if working {
            let sparkle_id = format!("activity-watch-{}", self.task_id());
            let field = self.watcher.modal_sparkles.get_or_insert_with(|| cx.new(|_| {
                crate::activity_sparkles::SparkleField::new(
                    sparkle_id.into(),
                    crate::activity_sparkles::SparkleLayout::Pane, false, palette, motion,
                )
            }));
            field.update(cx, |field, cx| {
                let changed = field.set_config(crate::activity_sparkles::SparkleLayout::Pane, false, palette, motion);
                let visible = field.set_visible(true).is_some();
                if changed || visible { cx.notify(); }
            });
        } else {
            self.watcher.modal_sparkles = None;
        }
        let modal_sparkles = self.watcher.modal_sparkles.clone();
        let two_columns = if modal { width >= 680. } else { width >= 720. && width > self.pane_height_design };
        let label = |text: String| {
            div()
                .min_w_0()
                .text_size(px(11. * ratio))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(palette.muted))
                .child(text)
        };
        let mut body = div()
            .id("watcher-body")
            .debug_selector(|| "watcher-body".into())
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(if modal { 12. } else { 20. }));
        if let Some(error) = self
            .error
            .as_deref()
            .and_then(connection_recovery::visible_error)
        {
            body = body.child(
                div()
                    .text_size(px(12. * ratio))
                    .text_color(rgb(palette.err()))
                    .child(format!("Connection: {error}")),
            );
        }
        let question_count = self.pending_user_questions().len();
        let permission_count = values(&self.snapshot, "approvalRequests")
            .filter(|request| request["taskId"] == self.task_id()
                && request["status"] == "pending"
                && (request["input"].is_null()
                    || !crate::approval_prompt::mcp_approval_choices(request).is_empty()))
            .count();
        if question_count > 0 {
            body = body.children(user_question_form);
        }
        if permission_count > 0 {
            body = body.child(
                Button::new("watcher-attention")
                    .compact()
                    .small()
                    .w_full()
                    .child(format!(
                        "{} request{} need your attention · Review in chat",
                        permission_count,
                        if permission_count == 1 { "" } else { "s" }
                    ))
                    .debug_selector(|| "watcher-attention".into())
                    .accessibility_label("Review pending requests in chat")
                    .on_click(cx.listener(|pane, _, window, cx| pane.toggle_watcher(window, cx))),
            );
        }
        let build_pill_palette = design::palette(&Value::Null, WindowAppearance::Dark);
        let mut build_pill = None;
        let running_build = watcher_running_build(&self.snapshot, &self.task_id());
        if let Some(build) = running_build.or_else(|| watcher_pending_build(&self.snapshot, &self.task_id())) {
            let running = running_build.is_some();
            let stale = running && build["leaseExpiresAt"].as_i64().is_some_and(|expires| expires < std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as i64);
            let title = if !running { Some(field(build, "title")) } else { values(&self.snapshot, "buildRequests")
                .find(|request| request["taskId"] == self.task_id()
                    && request["projectId"] == build["projectId"]
                    && build["requestIds"].as_array().is_some_and(|ids| ids.contains(&request["id"])))
                .map(|request| field(request, "title")) }.filter(|title| !title.is_empty())
                .unwrap_or_else(|| "Current build".into());
            let status = if stale { "Build needs attention" } else if running { "Building" }
                else if build["status"] == "ready" { "Waiting for build claim" } else { "Preparing build" };
            let mut indicator = div().id("watcher-build")
                .debug_selector(|| "watcher-build".into())
                .max_w_full().min_w_0().h(px(26. * ratio)).px(px(10. * ratio))
                .flex().items_center().gap(px(6.)).rounded_full().border_1()
                .border_color(rgb(build_pill_palette.line))
                .bg(crate::surface_blend::floating_control_background(build_pill_palette.panel, true))
                .text_size(px(11. * ratio)).line_height(px(14. * ratio)).text_color(rgb(palette.accent));
            if running && !stale && !cx.reduce_motion() {
                indicator = indicator.child(gpui_kit::component::spinner::Spinner::new()
                    .xsmall().color(rgb(palette.accent).into()));
            } else {
                indicator = indicator.child(Icon::new(if running { IconName::LoaderCircle } else { IconName::Clock }).size(px(12.)));
            }
            build_pill = Some(indicator.child(div().min_w_0().truncate()
                .child(format!("{status} · {title}"))));
        }
        let mut now = div()
            .id("watcher-now")
            .debug_selector(|| "watcher-now".into())
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(10.))
            .h(px(78.))
            .when(modal, |view| view.h_auto())
            .overflow_hidden();
        let live_line = div()
            .id("watcher-live-line")
            .debug_selector(|| "watcher-live-line".into())
            .w_full()
            .h(px(2.))
            .relative()
            .overflow_hidden()
            .rounded_full()
            .bg(rgb(if state.status == "running" {
                palette.accent
            } else {
                palette.line
            }));
        let live_line = if state.status == "running" && motion {
            live_line
                .with_animation(
                    "watcher-live-line-slide",
                    Animation::new(Duration::from_millis(1800))
                        .repeat_synced()
                        .with_max_fps(30.),
                    move |line, progress| {
                        let clear = rgba(palette.accent << 8);
                        let bright = rgb(palette.accent);
                        line.bg(rgba((palette.accent << 8) | 0x28)).child(
                            div().absolute().top_0().h_full()
                                .left(relative(progress * 1.6 - 0.6))
                                .w(relative(0.6)).flex()
                                .child(div().flex_1().h_full().bg(linear_gradient(
                                    90., linear_color_stop(clear, 0.), linear_color_stop(bright, 1.),
                                )))
                                .child(div().flex_1().h_full().bg(linear_gradient(
                                    90., linear_color_stop(bright, 0.), linear_color_stop(clear, 1.),
                                ))),
                        )
                    },
                )
                .into_any_element()
        } else {
            live_line.into_any_element()
        };
        now = now
            .child(
                div()
                    .id("watcher-latest-update")
                    .debug_selector(|| "watcher-latest-update".into())
                    .min_w_0()
                    .text_size(px(14. * ratio))
                    .font_weight(FontWeight::MEDIUM)
                    .line_height(px(20. * ratio))
                    .line_clamp(3)
                    .child(state.progress.clone().unwrap_or_else(|| {
                        if state.status != "running" {
                            "No active update".into()
                        } else if state.plan_active {
                            "Working through the current plan".into()
                        } else {
                            "Working on the current request".into()
                        }
                    })),
            )
            .child(live_line);
        let goal = self.activity.read(cx).watch_goal().cloned();
        let goal_pill = goal.as_ref().map(|_| {
            self.activity.read(cx).render_watch_goal_pill(palette, ratio)
        });
        let plan_region: AnyElement = if state.plan_active {
            crate::plans::render_plan_popup_filling(&self.snapshot, &self.task_id(), palette, ratio)
        } else {
            self.render_watcher_task_summary(&state, palette, ratio, cx)
        };
        body = body.child(
            div().w_full().min_w_0().flex().flex_col()
                .child(now).when(goal.is_none(), |body| body.child(self.activity.clone())),
        );
        let plan = div()
                .id("watcher-plan")
                .debug_selector(|| "watcher-plan".into())
                .w_full()
                .min_w_0()
                .min_h(if modal { px(0.) } else { watcher_plan_height(window) })
                .flex_shrink_0()
                .rounded(px(12.))
                .border_1()
                .border_color(rgb(if state.plan_active {
                    mix_rgb(palette.panel, palette.accent, 0.45)
                } else { palette.line }))
                .bg(design::surface_color(palette.panel))
                .shadow(vec![
                    gpui_kit::BoxShadow::new(
                        px(0.),
                        px(2.),
                        rgba(if palette.dark { 0x00000024 } else { 0x00000018 }).into(),
                    )
                    .blur_radius(px(8.)),
                ])
                .child({
                    let owner = cx.weak_entity();
                    div().relative().w_full().min_w_0().flex_shrink_0()
                        .when(build_pill.is_some(), |content| content.pt(px(6. * ratio)))
                        .child(plan_region).when(!modal, |content| content.child(
                        canvas(move |bounds, window, cx| {
                            measure_watch_card(owner.clone(), bounds.size.height + px(2.), window, cx);
                        }, |_, _, _, _| ()).absolute().size_full()
                    ))
                });
        let plan: AnyElement = if working {
            let glow = div().id("activity-watch-active-glow")
                .debug_selector(|| "activity-watch-active-glow".into())
                .w_full().min_w_0().rounded(px(12.)).border_1()
                .border_color(rgb(palette.accent).opacity(0.8)).child(plan);
            if motion {
                glow.with_animation("activity-watch-work-glow",
                    Animation::new(Duration::from_millis(2200)).repeat_synced().with_max_fps(30.),
                    move |view, phase| {
                        let pulse = (phase * std::f32::consts::TAU).sin() * 0.5 + 0.5;
                        view.shadow(vec![gpui_kit::BoxShadow::new(px(0.), px(0.),
                            rgb(palette.accent).opacity(0.12 + pulse * 0.14).into())
                            .blur_radius(px(10. + pulse * 8.))])
                    }).into_any_element()
            } else { glow.into_any_element() }
        } else { plan.into_any_element() };
        let mut card_slot = div().id("watcher-card-slot")
            .debug_selector(|| "watcher-card-slot".into())
            .relative().w_full().min_w_0().child(plan);
        if let Some(build_pill) = build_pill {
            // Overlay the border without changing the shared card's measured width.
            card_slot = card_slot.child(div().absolute().top(px(-13. * ratio))
                .left(px(8.)).right(px(8.)).flex().justify_center().child(build_pill));
        }
        if let Some(goal_pill) = goal_pill {
            // Match the build indicator's border-straddling treatment without
            // replacing the Work Plan card with a second goal interface.
            card_slot = card_slot.child(div().absolute().bottom(px(-13. * ratio))
                .left(px(8.)).right(px(8.)).flex().justify_center().child(goal_pill));
        }
        let finished_agents = state
            .agents
            .iter()
            .filter(|record| !active(&field(record, "status")))
            .collect::<Vec<_>>();
        let active_agents = state
            .agents
            .iter()
            .filter(|record| active(&field(record, "status")))
            .collect::<Vec<_>>();
        let mut team = div()
            .id("watcher-agents")
            .debug_selector(|| "watcher-agents".into())
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(5.))
            .child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(label("SUBAGENTS".into()))
                    .when(!state.agents.is_empty(), |heading| heading.child(
                        div()
                            .font_family(design::FONT_MONO)
                            .text_size(px(10. * ratio))
                            .text_color(rgb(palette.muted))
                            .child(format!(
                                "{} running · {} done",
                                state.running_agents, finished_agents.len()
                            )),
                    )),
            );
        let mut agent_cards = div()
            .id("watcher-agent-cards")
            .debug_selector(|| "watcher-agent-cards".into())
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(2.));
        for (index, record) in active_agents.iter().enumerate() {
            agent_cards = agent_cards.child(self.render_watcher_agent_row(
                index, record, palette, ratio, cx,
            ));
        }
        if !finished_agents.is_empty() {
            let expanded = self.watcher.finished_expanded;
            agent_cards = agent_cards.child(
                Button::new("watcher-finished-agents")
                    .debug_selector(|| "watcher-finished-agents".into())
                    .compact()
                    .xsmall()
                    .text()
                    .justify_start()
                    .h(px(22.))
                    .p_0()
                    .opacity(0.5)
                    .label(format!("> {} finished", finished_agents.len()))
                    .accessibility_label(if expanded {
                        "Hide finished subagents"
                    } else {
                        "Show finished subagents"
                    })
                    .on_click(cx.listener(|pane, _, _, cx| {
                        pane.watcher.finished_expanded = !pane.watcher.finished_expanded;
                        cx.notify();
                    })),
            );
            if expanded {
                for (offset, record) in finished_agents.iter().enumerate() {
                    agent_cards = agent_cards.child(self.render_watcher_agent_row(
                        active_agents.len() + offset,
                        record,
                        palette,
                        ratio,
                        cx,
                    ));
                }
            }
        }
        team = team.child(agent_cards);
        let mut activity_feed = div()
            .id("watcher-activity")
            .debug_selector(|| "watcher-activity".into())
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(7.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .child(label("ACTIVITY".into())),
            );
        if state.activity.is_empty() {
            activity_feed = activity_feed.child(
                div()
                    .text_size(px(12. * ratio))
                    .text_color(rgb(palette.muted))
                    .child("No activity recorded for this turn yet."),
            );
        }
        let terminal_font = field(&self.snapshot["settings"], "terminalFont");
        let terminal_font = if terminal_font.trim().is_empty() { "Menlo".to_owned() } else { terminal_font };
        for (index, event) in state.activity.iter().enumerate() {
            let kind = field(event, "kind");
            let description = watcher_activity_text(event);
            let icon_color = if matches!(kind.as_str(), "tool" | "computer" | "computer_image") {
                palette.accent
            } else {
                palette.muted
            };
            activity_feed = activity_feed.child(
                div().id(("watcher-activity-row", index)).w_full().min_w_0()
                    .flex().items_center().gap(px(7.))
                    .child(div().id(("watcher-activity-meta", index))
                        .debug_selector(move || format!("watcher-activity-meta-{index}"))
                        .flex_shrink_0().flex().items_center().gap(px(4.))
                        .font_family(design::FONT_MONO).text_size(px(7.6 * ratio)).line_height(px(11. * ratio))
                        .child(div().debug_selector(move || format!("watcher-activity-type-{index}"))
                            .w(px(10. * ratio)).h(px(10. * ratio)).flex_shrink_0()
                            .text_color(rgb(icon_color))
                            .child(Icon::new(watcher_activity_icon(&kind)).size(px(9. * ratio))))
                        .child(div().debug_selector(move || format!("watcher-activity-time-{index}"))
                            .whitespace_nowrap().text_color(rgb(palette.muted)).child(event_time(event))))
                    .child(div().id(("watcher-activity-description", index))
                        .debug_selector(move || format!("watcher-activity-description-{index}"))
                        .flex_1().min_w_0().font_family(SharedString::from(terminal_font.clone()))
                        .text_size(px(8. * ratio)).line_height(px(12. * ratio))
                        .text_color(rgb(palette.ink)).truncate()
                        .child(description))
            );
        }
        let secondary = div()
            .id("watcher-secondary")
            .debug_selector(|| "watcher-secondary".into())
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(if modal { 12. } else { 20. }))
            .when(!modal || !state.agents.is_empty(), |view| view.child(team))
            .child(activity_feed);
        body = body.child(
            div()
                .id("watcher-columns")
                .debug_selector(|| "watcher-columns".into())
                .w_full()
                .min_w_0()
                .flex()
                .gap(px(if modal { 16. } else { 20. }))
                .when(!two_columns, |layout| layout.flex_col())
                .child(div().flex_1().min_w_0().child(card_slot))
                .child(secondary.flex_1()),
        );
        let interject_width = (window.viewport_size().width - px(24.)).min(px(390.));
        let interject_open = self.watcher.interject_open && question_count == 0;
        let interject_focus = self.composer.read(cx).focus_handle(cx);
        let interject_palette = design::palette(&Value::Null, WindowAppearance::Dark);
        let owner = cx.weak_entity();
        let content_owner = owner.clone();
        let chat_button = watcher_mode_button(
            "chat-mode-chat", if self.watcher.modal_open { "Close" } else { "Chat" }, true, interject_palette, ratio, window, cx,
        )
            .debug_selector(|| "chat-mode-chat".into())
            .accessibility_label(if self.watcher.modal_open { "Close Watch" } else { "Show chat" })
            .tooltip(if self.watcher.modal_open { "Return to activity" } else { "Return to chat" })
            .on_click(cx.listener(|pane, _, window, cx| {
                if pane.watcher.modal_open { window.close_dialog(cx); } else { pane.toggle_watcher(window, cx); }
            }));
        let interject = Popover::new("watcher-interject-popover")
            .appearance(false)
            .anchor(Anchor::BottomCenter)
            .offset(px(7.))
            .open(interject_open)
            .overlay_closable(true)
            .track_focus(&interject_focus)
            .trigger(
                watcher_mode_button(
                    "watcher-back", "Steer", question_count == 0,
                    interject_palette, ratio, window, cx,
                )
                    .debug_selector(|| "watcher-back".into())
                    .accessibility_label("Steer without leaving Watch")
                    .tooltip("Send a steer without leaving Watch"),
            )
            .on_open_change(move |open, window, cx| {
                let _ = owner.update(cx, |pane, cx| {
                    pane.watcher.interject_open = *open;
                    if *open {
                        pane.slash.update(cx, |slash, cx| {
                            slash.handle_action(
                                crate::slash_palette::PaletteAction::Dismiss,
                                cx,
                            );
                        });
                        let focus = pane.composer.read(cx).focus_handle(cx);
                        window.defer(cx, move |window, cx| window.focus(&focus, cx));
                    }
                    cx.notify();
                });
            })
            .content(move |_, _, cx| {
                content_owner
                    .update(cx, |pane, cx| {
                        pane.render_watcher_interject_menu(
                            palette,
                            ratio,
                            interject_width,
                            cx,
                        )
                    })
                    .unwrap_or_else(|_| div().into_any_element())
            });
        let interject_dock = div()
            .id("watcher-interject-dock")
            .debug_selector(|| "watcher-interject-dock".into())
            .absolute()
            .bottom(px(8.))
            .left_0()
            .w_full()
            .min_w_0()
            .px(px(12.))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .id("watcher-interject-bar")
                    .debug_selector(|| "watcher-interject-bar".into())
                    .flex()
                    .items_center()
                    .rounded_full()
                    .border_1()
                    .border_color(rgb(interject_palette.line))
                    .bg(crate::surface_blend::floating_control_background(
                        interject_palette.panel,
                        true,
                    ))
                    .child(chat_button)
                    .child(
                        div()
                            .id("watcher-mode-separator")
                            .debug_selector(|| "watcher-mode-separator".into())
                            .w(px(1.))
                            .h(px(16.))
                            .flex_shrink_0()
                            .bg(rgb(interject_palette.line)),
                    )
                    .child(interject),
            );
        let layout_owner = cx.weak_entity();
        let previous_width = self.watcher.content_width;
        div()
            .id("chat-watcher")
            .debug_selector(|| "chat-watcher".into())
            .relative()
            .size_full()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .items_center()
            .overflow_hidden()
            // The shared parent paints this surface once across header and body.
            .text_color(rgb(palette.ink))
            .font_family(cx.theme().font_family.clone())
            .when(!self.watcher.modal_open, |view| view.on_mouse_down(
                MouseButton::Left,
                cx.listener(|pane, _, _, cx| cx.emit(ChatEvent::Focused(pane.task_id()))),
            ))
            .children(modal_sparkles.map(|field| div().absolute().size_full()
                .debug_selector(|| "activity-watch-sparkles".into()).overflow_hidden().child(field)))
            .child(canvas(move |bounds, window, cx| {
                let width = bounds.size.width;
                if width > px(0.) && (width - previous_width).abs() >= px(1.) {
                    let owner = layout_owner.clone();
                    window.defer(cx, move |_, cx| {
                        let _ = owner.update(cx, |pane, cx| {
                            if (pane.watcher.content_width - width).abs() >= px(1.) {
                                pane.watcher.content_width = width;
                                cx.notify();
                            }
                        });
                    });
                }
            }, |_, _, _, _| ()).absolute().size_full())
            .child(crate::transcript_edge_fade::TranscriptEdgeFade::for_scroll(
                div()
                    .id("watcher-scroll")
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .min_w_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.watcher.scroll)
                    .child(
                        div()
                            .id("watcher-content")
                            .debug_selector(|| "watcher-content".into())
                            .w_full()
                            .flex()
                            .justify_center()
                            .px(px(if modal { 20. } else { 44. }))
                            .pt(px(if modal { 16. } else { 48. }))
                            .pb(px((16. * ratio + 32.).max(48.)))
                            .child(div().w_full().max_w(px(if two_columns { 1200. } else { 760. })).min_w_0().child(body)),
                    ),
                self.watcher.scroll.clone(),
                px(24.),
            ))
            .child(interject_dock)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn watcher_build_indicator_tracks_owner_combined_requests_and_completion() {
        let mut snapshot = json!({"tasks":[{"id":"a","projectId":"p"},{"id":"b","projectId":"p"},{"id":"c","projectId":"q"}],
            "buildRequests":[{"id":"request","taskId":"b","projectId":"p"}],
            "buildRuns":[{"id":"build","ownerTaskId":"a","projectId":"p","status":"running","requestIds":["request"]}]});
        assert!(watcher_running_build(&snapshot, "a").is_some());
        assert!(watcher_running_build(&snapshot, "b").is_some());
        assert!(watcher_running_build(&snapshot, "c").is_none());
        assert!(watcher_running_build(&snapshot, "missing").is_none());
        snapshot["buildRequests"][0]["status"] = json!("planned");
        assert!(watcher_pending_build(&snapshot, "b").is_some());
        assert!(watcher_pending_build(&snapshot, "a").is_none());
        snapshot["buildRequests"][0]["status"] = json!("ready");
        assert_eq!(watcher_pending_build(&snapshot, "b").unwrap()["status"], "ready");
        snapshot["buildRequests"][0]["status"] = json!("cancelled");
        assert!(watcher_pending_build(&snapshot, "b").is_none());
        snapshot["buildRuns"][0]["status"] = json!("succeeded");
        assert!(watcher_running_build(&snapshot, "a").is_none());
        snapshot["buildRuns"][0]["status"] = json!("running");
        snapshot["buildRuns"][0]["projectId"] = json!("q");
        assert!(watcher_running_build(&snapshot, "a").is_none());
        assert!(watcher_running_build(&snapshot, "b").is_none());
    }



    #[::core::prelude::v1::test]
    fn watcher_scopes_plan_team_and_activity_and_keeps_completion() {
        let mut snapshot = json!({"tasks":[{"id":"a","status":"running"}],
            "nativeMessageContext":{"a":{"latestUserAt":100}},
            "workPlans":[{"id":"old","taskId":"a","status":"closed","updatedAt":300,"createdAt":1},
                {"id":"current","taskId":"a","status":"active","updatedAt":150,"createdAt":90},
                {"id":"foreign","taskId":"b","status":"active","updatedAt":400}],
            "events":[{"taskId":"a","kind":"agent_progress","title":"Old update","createdAt":1},
                {"taskId":"a","kind":"agent_progress","title":"Checking tests","createdAt":120},
                {"taskId":"b","kind":"tool","title":"Unrelated tool","createdAt":200},
                {"taskId":"a","kind":"tool","title":"cargo test","createdAt":125}],
            "approvalRequests":[{"taskId":"a","status":"pending"},{"taskId":"b","status":"pending"}]});
        let records = vec![
            json!({"id":"stale","status":"completed","createdAt":1,"updatedAt":80}),
            json!({"id":"live","status":"running","createdAt":1}),
            json!({"id":"done","status":"completed","createdAt":92,"updatedAt":130}),
        ];
        let state = WatcherProjection::new(&snapshot, "a", &records, &[]);
        assert_eq!(state.plan.unwrap()["id"], "current");
        assert!(state.plan_active, "the active plan owns the plan card");
        assert_eq!(state.agents.len(), 2);
        assert_eq!(state.agents[0]["id"], "live");
        assert_eq!(state.running_agents, 1);
        assert_eq!(state.progress.as_deref(), Some("Checking tests"));
        assert_eq!(state.latest_tool.as_deref(), Some("cargo test"));
        assert_eq!(state.attention, 1);
        snapshot["tasks"][0]["status"] = json!("idle");
        snapshot["workPlans"][1]["status"] = json!("closed");
        snapshot["workPlans"][1]["updatedAt"] = json!(400);
        let state = WatcherProjection::new(&snapshot, "a", &records, &[]);
        assert_eq!(state.plan.unwrap()["id"], "current");
        assert_eq!(state.status, "idle");
        assert!(
            !state.plan_active,
            "a finished plan hands the card to the task summary"
        );
    }

    #[::core::prelude::v1::test]
    fn watcher_summary_replaces_the_plan_when_none_is_active() {
        let mut snapshot = json!({"tasks":[{"id":"a","title":"Ship the release","status":"idle",
                "provider":"codex","model":"gpt-5-codex","createdAt":1_000,"updatedAt":4_000}],
            "nativeMessageContext":{"a":{"latestUserAt":100}},
            "workPlans":[{"id":"older","taskId":"a","title":"Older plan","status":"closed",
                "createdAt":1,"updatedAt":300},
                {"id":"latest","taskId":"a","title":"Cutover plan","status":"closed",
                "createdAt":10,"updatedAt":900,"items":[
                    {"id":"one","status":"completed"},{"id":"two","status":"completed"},
                    {"id":"three","status":"pending"}]}],
            "events":[],"approvalRequests":[]});
        let messages = vec![
            Rc::new(json!({"id":"first","taskId":"a","role":"user","text":"Old request",
                "createdAt":10})),
            Rc::new(json!({"id":"second","taskId":"a","role":"user",
                "text":"Finish the cutover and ship the release notes","createdAt":40})),
            Rc::new(json!({"id":"peer","taskId":"a","role":"user","senderAgentId":"builder",
                "text":"Delegated note", "createdAt":60})),
            Rc::new(json!({"id":"answer","taskId":"a","role":"assistant","text":"Done",
                "createdAt":80})),
        ];
        let state = WatcherProjection::new(&snapshot, "a", &[], &messages);
        assert!(
            !state.plan_active,
            "a closed plan never occupies the plan card"
        );
        assert_eq!(
            state.finished_plan.clone().unwrap()["id"],
            "latest",
            "the most recently finished plan stays available"
        );
        assert_eq!(state.title, "Ship the release");
        assert_eq!(state.provider, "codex");
        assert_eq!(state.model, "gpt-5-codex");
        assert_eq!(state.duration, "0:03");
        assert_eq!(
            state.request.as_deref(),
            Some("Finish the cutover and ship the release notes"),
            "the summary reads the owner's latest request, not peer traffic"
        );

        // Republishing the plan as active hands the slot back to the plan card.
        snapshot["workPlans"][1]["status"] = json!("active");
        let active = WatcherProjection::new(&snapshot, "a", &[], &messages);
        assert!(active.plan_active);
        assert_eq!(
            active.finished_plan.unwrap()["id"],
            "older",
            "only non-active plans remain eligible as the last plan"
        );

        // No plan at all still summarises the task, and whitespace collapses.
        snapshot["workPlans"] = json!([]);
        let bare = WatcherProjection::new(&snapshot, "a", &[], &[]);
        assert!(!bare.plan_active);
        assert!(bare.finished_plan.is_none());
        assert!(bare.request.is_none(), "no request means no summary request");
        let wrapped = WatcherProjection::new(
            &snapshot,
            "a",
            &[],
            &[Rc::new(json!({"id":"w","role":"user","text":"line one\n\n  line two  ",
                "createdAt":5}))],
        );
        assert_eq!(wrapped.request.as_deref(), Some("line one line two"));
    }

    #[::core::prelude::v1::test]
    fn watcher_activity_time_includes_seconds() {
        let rendered = event_time(&json!({"createdAt": 1_700_000_000_000_i64}));
        assert_eq!(rendered.matches(':').count(), 2, "{rendered}");
    }

    #[::core::prelude::v1::test]
    fn watcher_activity_summaries_use_common_tool_arguments_and_accounting() {
        let command = json!({
            "kind":"tool", "title":"exec_command",
            "detail":json!({"arguments":json!({"cmd":"cargo test --lib watcher"}).to_string()}).to_string()
        });
        assert_eq!(watcher_activity_text(&command), "Ran cargo test --lib watcher");

        let read = json!({
            "kind":"tool", "title":"read_file",
            "detail":json!({"path":"desktop-gpui/src/chat_watcher.rs"}).to_string()
        });
        assert_eq!(watcher_activity_text(&read), "Read desktop-gpui/src/chat_watcher.rs");

        let search = json!({
            "kind":"tool", "title":"web_search",
            "detail":json!({"arguments":json!({"search_query":[{"q":"GPUI retained view docs"}]}).to_string()}).to_string()
        });
        assert_eq!(watcher_activity_text(&search), "Searched for GPUI retained view docs");

        let plan = json!({
            "kind":"tool", "title":"mcp__monitter__work_plan_start",
            "detail":json!({"arguments":json!({"title":"Improve Watch event rows"}).to_string()}).to_string()
        });
        assert_eq!(watcher_activity_text(&plan), "Worked in monitter · Improve Watch event rows");

        let build_claim = json!({
            "kind":"tool", "title":"build_claim",
            "detail":json!({"type":"mcpToolCall","server":"monitter","tool":"build_claim","arguments":{"request_ids":["request"]}}).to_string()
        });
        assert_eq!(watcher_activity_text(&build_claim), "Monitter · Build claim");

        let usage = json!({
            "kind":"usage", "title":"Usage updated",
            "detail":json!({"tokens":{"input":1234,"output":56,"reasoning":7},"cost":0.0025}).to_string()
        });
        assert_eq!(watcher_activity_text(&usage), "1,234 input · 56 output · 7 reasoning · $0.0025");

        let context = json!({
            "kind":"usage", "title":"Context usage updated",
            "detail":json!({"usage":{"used":159661,"size":258400}}).to_string()
        });
        assert_eq!(watcher_activity_text(&context), "159,661 / 258,400 context (62%)");

        let decision = json!({
            "kind":"jevDecision", "title":"System One decision",
            "detail":json!({
                "labels":{"abstain":"Abstain","blue_square":"Blue square"},
                "response":{"candidateId":"blue_square","confidence":0.48,"status":"ok"}
            }).to_string()
        });
        assert_eq!(watcher_activity_text(&decision), "System One · Blue square · 48% confidence");
    }

    #[::core::prelude::v1::test]
    fn watcher_activity_summaries_keep_reasoning_and_errors_to_one_bounded_line() {
        let reasoning = json!({
            "kind":"reasoning", "title":"Reasoning",
            "detail":json!({"summary":"Inspect the formatter\nthen update its focused tests"}).to_string()
        });
        assert_eq!(watcher_activity_text(&reasoning), "Inspect the formatter then update its focused tests");

        let failure = json!({
            "kind":"error", "title":"Codex error",
            "detail":json!({"message":"Capacity temporarily unavailable"}).to_string()
        });
        assert_eq!(watcher_activity_text(&failure), "Codex error · Capacity temporarily unavailable");

        let long = "word ".repeat(80);
        let output = json!({"kind":"output","title":"Assistant response","detail":long});
        let summary = watcher_activity_text(&output);
        assert!(!summary.contains('\n'));
        assert!(summary.chars().count() <= 180);
        assert!(summary.ends_with('…'));
    }

    #[::core::prelude::v1::test]
    fn watcher_interject_correlates_ack_with_new_or_updated_assistant_reply() {
        let baseline = vec![Rc::new(json!({
            "id":"assistant","taskId":"watch","role":"assistant",
            "text":"Working","streamStatus":"streaming"
        }))];
        let snapshot = json!({"queuedMessages":[]});
        let exchange = InterjectionExchange::capture(
            "Check the edge case".into(),
            "Check the edge case".into(),
            &snapshot,
            &baseline,
        );
        let sending = json!({"queuedMessages":[{
            "id":"steer","taskId":"watch","text":"Check the edge case",
            "status":"sending","origin":"steering"
        }]});
        assert_eq!(
            exchange.project(&sending, "watch", &baseline).status,
            "Steering into the current turn…"
        );

        let peer_only = vec![
            baseline[0].clone(),
            Rc::new(json!({
                "id":"peer","taskId":"watch","role":"user",
                "senderAgentId":"peer-agent","text":"Check the edge case"
            })),
        ];
        assert!(
            exchange
                .project(&json!({"queuedMessages":[]}), "watch", &peer_only)
                .response_id
                .is_none(),
            "a peer message with the same text is not the steer acknowledgement"
        );

        let replied = vec![
            Rc::new(json!({
                "id":"assistant","taskId":"watch","role":"assistant",
                "text":"Working\nI checked it.","streamStatus":"streaming"
            })),
            Rc::new(json!({
                "id":"accepted","taskId":"watch","role":"user",
                "text":"Check the edge case"
            })),
        ];
        let projection = exchange.project(&json!({"queuedMessages":[]}), "watch", &replied);
        assert_eq!(projection.response_id.as_deref(), Some("assistant"));
        assert_eq!(
            projection.response_text.as_deref(),
            Some("Working\nI checked it.")
        );
        assert_eq!(projection.status, "Receiving reply…");

        let fallback = json!({"queuedMessages":[{
            "id":"steer","taskId":"watch","text":"Check the edge case",
            "status":"queued","origin":"steering"
        }]});
        assert_eq!(
            exchange.project(&fallback, "watch", &baseline).status,
            "Steer unavailable · queued for the next turn"
        );
    }
}

#[cfg(all(test, feature = "visual-export", feature = "performance-profiling"))]
mod native_tests {
    use super::*;
    use core::prelude::v1::test;
    use gpui_kit::{TestAppContext, VisualTestContext};
    fn draw(visual: &mut VisualTestContext) {
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
    }
    #[gpui_kit::test]
    fn pane_mode_tracks_animation_target_and_restore_cancels_motion(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let (_, visual) = cx.add_window_view(|window, cx| {
            let pane = cx.new(|cx| ChatPane::performance_fixture(
                "mode-fixture".into(), Rc::new(json!({"settings":{},"tasks":[],"messages":[],"events":[]})), window, cx));
            pane.update(cx, |pane, cx| {
                for enabled in [false, true] {
                    pane.restore_watch_mode(enabled, cx);
                    pane.watcher.modal_open = true;
                    assert_eq!(pane.persisted_watch_mode(), enabled, "modal state is not pane mode");
                    pane.watcher.modal_open = false;
                    pane.toggle_watcher(window, cx);
                    assert_eq!(pane.persisted_watch_mode(), !enabled);
                    pane.restore_watch_mode(enabled, cx);
                    assert_eq!(pane.persisted_watch_mode(), enabled);
                    assert!(!pane.watcher.transitioning);
                    assert!(pane.watcher.transition_task.is_none());
                }
            });
            gpui_kit::component::Root::new(pane, window, cx)
        });
        visual.run_until_parked();
    }

    #[gpui_kit::test]
    fn watcher_shared_card_arrows_and_stacked_activity_metadata(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::surface_blend::initialize_for_ui_fixture();
            crate::native_motion::initialize_for_ui_fixture(cx);
        });
        for theme in ["light", "dark"] {
            for width in [320., 900.] {
                let snapshot = json!({"settings":{"theme":theme,"interfaceFontSize":14,"terminalFont":"Menlo"},
                    "tasks":[{"id":"watch","title":"Current task","agentId":"agent","provider":"codex","status":"running","updatedAt":40,"queue":[]}],
                    "agents":[{"id":"agent","name":"Codex","provider":"codex"}],
                    "nativeMessageContext":{"watch":{"latestUserAt":10}},
                    "messages":[],"subagentSessions":[],"workPlans":[],
                    "events":[{"id":"event","taskId":"watch","kind":"tool","title":"A long terminal activity description stays on one line beside the stacked metadata.\nEven multiline tool commands must truncate rather than expanding this activity row.","createdAt":20}],
                    "approvalRequests":[],"queuedMessages":[],"hosts":[],"channels":[]});
                let mut retained = None;
                let (_, visual) = cx.add_window_view(|window, cx| {
                    let pane = cx.new(|cx| {
                        let mut pane = ChatPane::performance_fixture("watch".into(), Rc::new(snapshot), window, cx);
                        pane.watcher.enabled = true;
                        pane
                    });
                    pane.update(cx, |pane, cx| pane.activity.update(cx, |view, cx| {
                        view.set_watch_goal_fixture(Some(json!({"status":"active","objective":"Goal objective","tokensUsed":124000,"tokenBudget":2000000,"createdAt":1,"updatedAt":30})), cx);
                    }));
                    retained = Some(pane.clone());
                    gpui_kit::component::Root::new(pane, window, cx)
                });
                let pane = retained.unwrap();
                visual.simulate_resize(size(gpui_kit::px(width), gpui_kit::px(1100.)));
                draw(visual); draw(visual);
                assert!(visual.debug_bounds("watcher-goal").is_none(), "the full goal interface is removed from Watch");
                assert!(visual.debug_bounds("watcher-task-summary").is_some(), "the task/work-plan card stays mounted");
                let slot = visual.debug_bounds("watcher-card-slot").unwrap();
                let pill = visual.debug_bounds("watcher-goal-pill").unwrap();
                assert!(pill.top() < slot.bottom() && pill.bottom() > slot.bottom(), "goal pill straddles the card's bottom border");
                assert!(visual.debug_bounds("watcher-goal-usage").is_some());
                assert!(visual.debug_bounds("watcher-goal-status").is_some());
                assert!(visual.debug_bounds("watcher-card-previous").is_none());
                assert!(visual.debug_bounds("watcher-card-next").is_none());
                let kind = visual.debug_bounds("watcher-activity-type-0").unwrap();
                let time = visual.debug_bounds("watcher-activity-time-0").unwrap();
                let description = visual.debug_bounds("watcher-activity-description-0").unwrap();
                assert!((time.center().y - kind.center().y).abs() <= px(1.), "icon and timestamp share one line");
                assert!(time.left() >= kind.right() && description.left() > time.right());
                assert!((description.center().y - time.center().y).abs() <= px(2.), "description shares the activity line");
                assert!(description.right() <= visual.debug_bounds("watcher-activity").unwrap().right() + px(1.));
                assert!(description.size.height <= px(12.1), "long multiline activity stays on one smaller text line: {description:?}");
                assert!(kind.size.height <= px(10.5) && time.size.height <= px(11.5), "compact icon and timestamp stay subordinate");
                pane.update(visual, |pane, cx| pane.activity.update(cx, |view, cx| {
                    view.set_watch_goal_fixture(Some(json!({"status":"paused","objective":"Goal objective","tokensUsed":125000,"tokenBudget":2000000,"createdAt":1,"updatedAt":50})), cx);
                }));
                draw(visual); draw(visual);
                assert!(visual.debug_bounds("watcher-goal-pill").is_some(), "paused goals remain a compact status pill");
                assert!(visual.debug_bounds("watcher-task-summary").is_some(), "goal updates never replace the work card");
                pane.update(visual, |pane, cx| pane.activity.update(cx, |view, cx| view.set_watch_goal_fixture(None, cx)));
                draw(visual); draw(visual);
                assert!(visual.debug_bounds("watcher-goal-pill").is_none());
            }
        }
    }

    #[gpui_kit::test]
    fn watcher_columns_follow_pane_aspect_and_resize_without_losing_content(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::surface_blend::initialize_for_ui_fixture();
            crate::native_motion::initialize_for_ui_fixture(cx);
        });
        for theme in ["light", "dark"] {
            let snapshot = json!({"settings":{"theme":theme,"interfaceFontSize":14},
                "tasks":[{"id":"watch","title":"Responsive Watch","agentId":"agent","status":"running","queue":[]}],
                "agents":[{"id":"agent","name":"Codex","provider":"codex"}],
                "workPlans":[{"id":"plan","taskId":"watch","title":"Current plan","status":"active","createdAt":20,
                    "items":[{"id":"step","title":"Verify responsive layout","status":"in_progress"}]}],
                "subagentSessions":[{"id":"child","parentTaskId":"watch","title":"A long subagent title that still fits its column","status":"running","createdAt":20}],
                "events":[{"id":"event","taskId":"watch","kind":"tool","title":"Checking native geometry","createdAt":21}],
                "messages":[],"approvalRequests":[],"queuedMessages":[],"hosts":[],"channels":[]});
            let (_, visual) = cx.add_window_view(|window, cx| {
                let pane = cx.new(|cx| {
                    let mut pane = ChatPane::performance_fixture("watch".into(), Rc::new(snapshot), window, cx);
                    pane.watcher.enabled = true;
                    pane
                });
                gpui_kit::component::Root::new(pane, window, cx)
            });
            for (width, height, landscape) in [
                (1200., 660., true), (660., 1000., false), (800., 800., false),
                (800., 600., true), (320., 600., false), (660., 400., false), (1200., 660., true),
            ] {
                visual.simulate_resize(size(px(width), px(height)));
                draw(visual); draw(visual); draw(visual);
                let root = visual.debug_bounds("chat-watcher").unwrap();
                let body = visual.debug_bounds("watcher-body").unwrap();
                let now = visual.debug_bounds("watcher-now").unwrap();
                let plan = visual.debug_bounds("watcher-plan").unwrap();
                let secondary = visual.debug_bounds("watcher-secondary").unwrap();
                let agents = visual.debug_bounds("watcher-agents").unwrap();
                let activity = visual.debug_bounds("watcher-activity").unwrap();
                assert!(plan.top() >= now.bottom(), "current update spans above both columns");
                assert!(activity.top() >= agents.bottom(), "activity follows subagents");
                if landscape {
                    assert!((plan.top() - secondary.top()).abs() <= px(1.), "landscape aligns column tops: {plan:?}, {secondary:?}");
                    assert!(secondary.left() >= plan.right() + px(19.), "landscape columns do not overlap");
                    assert!((plan.size.width - secondary.size.width).abs() <= px(2.), "columns share available width");
                    assert!(body.size.width > px(700.), "landscape uses the available width");
                } else {
                    assert!(agents.top() >= plan.bottom() + px(19.), "portrait and narrow layouts stack: {plan:?}, {agents:?}");
                    assert!((plan.size.width - body.size.width).abs() <= px(2.), "stacked plan fills the body");
                }
                for bounds in [now, plan, secondary, agents, activity,
                    visual.debug_bounds("watcher-agent-row-0").unwrap()] {
                    assert!(bounds.left() >= body.left() - px(1.) && bounds.right() <= body.right() + px(1.),
                        "content fits after resize: {bounds:?} in {body:?}");
                }
                let dock = visual.debug_bounds("watcher-interject-bar").unwrap();
                assert!((dock.center().x - root.center().x).abs() <= px(1.), "controls stay centered");
                assert!((root.bottom() - dock.bottom() - px(8.)).abs() <= px(1.), "controls stay at the bottom");
            }
        }
    }

    #[gpui_kit::test]
    fn watcher_build_indicator_mounts_and_clears_after_completion(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::surface_blend::initialize_for_ui_fixture();
            crate::native_motion::initialize_for_ui_fixture(cx);
        });
        for theme in ["light", "dark"] {
        let mut snapshot = json!({"settings":{"theme":theme},
            "tasks":[{"id":"watch","projectId":"p","title":"Build work","agentId":"agent","provider":"codex","status":"running","queue":[]}],
            "agents":[{"id":"agent","name":"Codex","provider":"codex"}],
            "buildRuns":[{"id":"build","projectId":"p","ownerTaskId":"watch","status":"running","requestIds":["r"],"leaseExpiresAt":i64::MAX}],
            "buildRequests":[{"id":"r","projectId":"p","taskId":"watch","title":"Compile desktop with a long descriptive build title that must fit a narrow pane"}],
            "workPlans":[],"subagentSessions":[],"messages":[],"events":[],"approvalRequests":[],"hosts":[],"channels":[]});
        let mut retained = None;
        let (_, visual) = cx.add_window_view(|window, cx| {
            let pane = cx.new(|cx| ChatPane::performance_fixture("watch".into(), Rc::new(snapshot.clone()), window, cx));
            pane.update(cx, |pane, _| pane.watcher.enabled = true);
            retained = Some(pane.clone());
            gpui_kit::component::Root::new(pane, window, cx)
        });
        visual.simulate_resize(size(gpui_kit::px(360.), gpui_kit::px(800.)));
        draw(visual); draw(visual);
        for width in [320., 900.] {
            visual.simulate_resize(size(px(width), px(800.)));
            draw(visual); draw(visual);
            let pill = visual.debug_bounds("watcher-build").expect("running build pill");
            let card = visual.debug_bounds("watcher-plan").unwrap();
            let line = visual.debug_bounds("watcher-live-line").unwrap();
            assert!(pill.top() > line.bottom(), "build status follows animated progress");
            assert!((pill.center().y - card.top()).abs() <= px(1.), "pill straddles top border: {pill:?} {card:?}");
            assert!((pill.center().x - card.center().x).abs() <= px(1.), "pill is centered on card");
            assert!(pill.left() >= card.left() && pill.right() <= card.right(), "long build title fits narrow card");
            visual.update(|window, _| {
                let palette = design::palette(&Value::Null, WindowAppearance::Dark);
                let expected = pill.scale(window.scale_factor());
                let fill = crate::surface_blend::floating_control_background(palette.panel, true);
                let fill = fill.backdrop_blur(fill.backdrop_blur_radius() * window.scale_factor());
                let near = |a: gpui_kit::ScaledPixels, b: gpui_kit::ScaledPixels| (a.as_f32() - b.as_f32()).abs() < 0.6;
                assert!(window.painted_quads().iter().any(|quad| {
                    quad.background == fill
                        && near(quad.bounds.origin.x, expected.origin.x)
                        && near(quad.bounds.origin.y, expected.origin.y)
                        && near(quad.bounds.size.width, expected.size.width)
                        && near(quad.bounds.size.height, expected.size.height)
                }), "pill uses the same dark floating surface as Chat/Steer: expected {expected:?}, nearby {:?}",
                    window.painted_quads().iter().filter(|quad| (quad.bounds.origin.y.as_f32() - expected.origin.y.as_f32()).abs() < 5.)
                        .map(|quad| (quad.bounds, quad.background == fill, quad.background)).collect::<Vec<_>>());
            });
        }
        snapshot["buildRuns"][0]["status"] = json!("succeeded");
        let pane = retained.unwrap();
        pane.update(visual, |pane, cx| pane.sync(Rc::new(snapshot.clone()), Some("active".into()), cx));
        draw(visual); draw(visual);
        assert!(visual.debug_bounds("watcher-build").is_none(), "completed build removes spinner");
        snapshot["buildRequests"][0]["status"] = json!("ready");
        pane.update(visual, |pane, cx| pane.sync(Rc::new(snapshot.clone()), Some("active".into()), cx));
        draw(visual); draw(visual);
        assert!(visual.debug_bounds("watcher-build").is_some(), "waiting request remains visible");
        snapshot["buildRequests"][0]["status"] = json!("cancelled");
        pane.update(visual, |pane, cx| pane.sync(Rc::new(snapshot), Some("active".into()), cx));
        draw(visual); draw(visual);
        assert!(visual.debug_bounds("watcher-build").is_none(), "cancelled request clears");
        }
    }

    struct WatchPair(Vec<Entity<ChatPane>>);
    impl Render for WatchPair {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().flex().children(self.0.iter().map(|pane|
                div().flex_1().min_w_0().h_full().child(pane.clone())))
        }
    }
    #[gpui_kit::test]
    fn watcher_short_card_matches_tallest_current_neighbour(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::surface_blend::initialize_for_ui_fixture();
            crate::native_motion::initialize_for_ui_fixture(cx);
        });
        let mut panes = Vec::new();
        let (_, visual) = cx.add_window_view(|window, cx| {
            for (task, count) in [("tall", 6), ("short", 1)] {
                let snapshot = json!({"settings":{"theme":"dark","interfaceFontSize":14},
                    "tasks":[{"id":task,"title":"Work","agentId":"agent","status":"running","queue":[]}],
                    "agents":[{"id":"agent","name":"Codex","provider":"codex"}],
                    "workPlans":[{"id":task,"taskId":task,"title":"Current plan","status":"active","createdAt":20,
                        "items":(0..count).map(|n| json!({"id":format!("step-{n}"),"title":"Work item","status":"pending"})).collect::<Vec<_>>() }],
                    "subagentSessions":[],"messages":[],"events":[],"approvalRequests":[],"queuedMessages":[],"hosts":[],"channels":[]});
                let pane = cx.new(|cx| {
                    let mut pane = ChatPane::performance_fixture(task.into(), Rc::new(snapshot), window, cx);
                    pane.watcher.enabled = true;
                    pane
                });
                panes.push(pane.clone());
            }
            let pair = cx.new(|_| WatchPair(panes.clone()));
            gpui_kit::component::Root::new(pair, window, cx)
        });
        visual.simulate_resize(size(px(1600.), px(1200.)));
        draw(visual); draw(visual); draw(visual);
        let shortest = visual.debug_bounds("watcher-plan").unwrap();
        let agents = visual.debug_bounds("watcher-agents").unwrap();
        assert!(agents.top() >= shortest.bottom(),
            "portrait split panes stay stacked inside a landscape workspace window");
        visual.update(|window, _| {
            let id = window.window_handle().window_id();
            WATCH_CARD_HEIGHTS.with(|entries| {
                let entries = entries.borrow();
                let heights: Vec<_> = entries.iter().filter(|entry| entry.window == id).map(|entry| entry.height).collect();
                assert_eq!(heights.len(), 2);
                let max = heights.iter().copied().fold(px(0.), Pixels::max);
                let min = heights.iter().copied().fold(max, Pixels::min);
                assert!(max > min + px(60.), "fixture has distinct natural heights");
                assert!((shortest.size.height - max).abs() < px(2.), "short card shares tallest content height: {shortest:?}, {max:?}");
            });
        });
    }
    #[gpui_kit::test]
    fn watcher_history_grows_the_card_and_moves_following_content(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::surface_blend::initialize_for_ui_fixture();
            crate::native_motion::initialize_for_ui_fixture(cx);
        });
        let snapshot = json!({"settings":{"theme":"dark","interfaceFontSize":14},
            "tasks":[{"id":"watch","title":"Work","agentId":"agent","status":"running","queue":[]}],
            "agents":[{"id":"agent","name":"Codex","provider":"codex"}],
            "workPlans":[
                {"id":"new","taskId":"watch","title":"Current","status":"active","createdAt":20,"items":[{"id":"a","title":"Current step","status":"pending"}]},
                {"id":"old","taskId":"watch","title":"Earlier work","status":"completed","createdAt":10,"items":[{"id":"b","title":"Earlier step","status":"completed"}]}],
            "subagentSessions":[],"messages":[],"events":[],"approvalRequests":[],"queuedMessages":[],"hosts":[],"channels":[]});
        let (_, visual) = cx.add_window_view(|window, cx| {
            let pane = cx.new(|cx| {
                let mut pane = ChatPane::performance_fixture("watch".into(), Rc::new(snapshot), window, cx);
                pane.watcher.enabled = true;
                pane
            });
            gpui_kit::component::Root::new(pane, window, cx)
        });
        visual.simulate_resize(size(px(800.), px(1200.)));
        draw(visual); draw(visual);
        let before = visual.debug_bounds("watcher-plan").unwrap();
        let below = visual.debug_bounds("watcher-agents").unwrap();
        let toggle = visual.debug_bounds("work-plan-popup-history").unwrap();
        visual.simulate_click(toggle.center(), Default::default());
        draw(visual); draw(visual);
        let expanded = visual.debug_bounds("watcher-plan").unwrap();
        let moved = visual.debug_bounds("watcher-agents").unwrap();
        assert!(expanded.size.height > before.size.height + px(30.), "history expands card: {before:?} -> {expanded:?}");
        assert!(moved.top() > below.top() + px(30.), "history pushes following content down");
        let toggle = visual.debug_bounds("work-plan-popup-history").unwrap();
        visual.simulate_click(toggle.center(), Default::default());
        draw(visual); draw(visual);
        let collapsed = visual.debug_bounds("watcher-plan").unwrap();
        assert!((collapsed.size.height - before.size.height).abs() < px(2.), "collapse restores intrinsic height");
    }
    #[gpui_kit::test]
    fn watcher_interject_popover_keeps_the_watch_surface_and_retained_draft(
        cx: &mut TestAppContext,
    ) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::surface_blend::initialize_for_ui_fixture();
            crate::native_motion::initialize_for_ui_fixture(cx);
        });
        for theme in ["light", "dark"] {
            let mut snapshot = json!({"settings":{"theme":theme,"interfaceFontSize":14},
                "tasks":[{"id":"watch","title":"Long workplan","agentId":"agent","provider":"codex","status":"running","queue":[]}],
                "agents":[{"id":"agent","name":"Codex","provider":"codex"}],
                "nativeMessageContext":{"watch":{"latestUserAt":10}},
                "workPlans":[{"id":"plan","taskId":"watch","title":"Ship the change","status":"active","createdAt":10,"updatedAt":10,
                    "items":(0..3).map(|i| json!({"id":format!("step-{i}"),"title":format!("Work item {i}"),"status":"pending"})).collect::<Vec<_>>() }],
                "subagentSessions":[
                    {"id":"child-1","parentTaskId":"watch","agentPath":"architecture","title":"Saved Layout Architecture","status":"running","createdAt":10,"updatedAt":10},
                    {"id":"child-2","parentTaskId":"watch","agentPath":"model","title":"Saved Layout Model","status":"running","createdAt":11,"updatedAt":11},
                    {"id":"child-3","parentTaskId":"watch","agentPath":"preview","title":"Saved Layout Preview","status":"running","createdAt":12,"updatedAt":12},
                    {"id":"child-4","parentTaskId":"watch","agentPath":"state","title":"Watcher State Map","status":"running","createdAt":13,"updatedAt":13}],
                "messages":[],"events":[{"id":"update","taskId":"watch","kind":"agent_progress","title":"Checking the implementation","createdAt":11}],
                "approvalRequests":[],"queuedMessages":[],"hosts":[],"channels":[]});
            let mut retained = None;
            let (_, visual) = cx.add_window_view(|window, cx| {
                let pane = cx.new(|cx| {
                    ChatPane::performance_fixture(
                        "watch".into(),
                        Rc::new(snapshot.clone()),
                        window,
                        cx,
                    )
                });
                pane.update(cx, |pane, cx| {
                    assert!(pane.restore_draft("Keep this draft", window, cx));
                });
                retained = Some(pane.clone());
                gpui_kit::component::Root::new(pane, window, cx)
            });
            let pane = retained.unwrap();
            for width in [320., 800.] {
                snapshot["tasks"][0]["status"] = json!("running");
                snapshot["workPlans"][0]["status"] = json!("active");
                snapshot["approvalRequests"] = json!([]);
                for session in snapshot["subagentSessions"].as_array_mut().unwrap() {
                    session["status"] = json!("running");
                }
                pane.update(visual, |pane, cx| {
                    pane.watcher.finished_expanded = false;
                    pane.sync(Rc::new(snapshot.clone()), Some("active".into()), cx)
                });
                visual.simulate_resize(size(gpui_kit::px(width), gpui_kit::px(600.)));
                draw(visual);
                let header = visual
                    .debug_bounds("chat-pane-header")
                    .expect("mode switch header remains mounted");
                let toggle = visual
                    .debug_bounds("chat-watcher-toggle")
                    .expect("single Watch action");
                let mode_dock = visual
                    .debug_bounds("chat-watch-mode-dock")
                    .expect("bottom mode dock");
                let status_bar = visual
                    .debug_bounds("pane-drawer-toolbar")
                    .expect("chat status bar");
                assert!(
                    (toggle.center().x - mode_dock.center().x).abs() <= gpui_kit::px(1.),
                    "Watch action stays centered in the bottom dock: {toggle:?} in {mode_dock:?}"
                );
                assert!(
                    (mode_dock.center().y - status_bar.top()).abs() <= gpui_kit::px(1.),
                    "Watch pill stays vertically centered over the status-bar top border: pill={mode_dock:?}, bar={status_bar:?}"
                );
                assert!(
                    visual.debug_bounds("work-plan-menu-pill").is_none(),
                    "the status bar no longer duplicates the Work Plan surface"
                );
                assert!(
                    visual.debug_bounds("agent-map-pill").is_none(),
                    "the superseded Agent Map action is removed from the status bar"
                );
                assert!(
                    visual.debug_bounds("chat-mode-chat").is_none(),
                    "only the destination action is mounted"
                );
                visual.simulate_click(
                    toggle.center(),
                    Default::default(),
                );
                visual
                    .executor()
                    .advance_clock(std::time::Duration::from_millis(601));
                draw(visual);
                let root = visual
                    .debug_bounds("chat-watcher")
                    .expect("Watcher mounted");
                let chat = visual
                    .debug_bounds("chat-mode-chat")
                    .expect("single Chat action");
                assert!(
                    visual.debug_bounds("chat-watcher-toggle").is_none(),
                    "the Watch action is replaced instead of selected"
                );
                assert!(
                    (root.size.width - gpui_kit::px(width)).abs() <= gpui_kit::px(1.),
                    "Watcher owns the pane body"
                );
                assert!((root.bottom() - gpui_kit::px(600.)).abs() <= gpui_kit::px(1.));
                assert!(root.top() >= header.bottom() - gpui_kit::px(1.));
                visual.update(|window, _| {
                    let palette = design::palette(&snapshot["settings"], window.appearance());
                    let surface = gpui_kit::Bounds::new(
                        header.origin,
                        size(header.size.width, root.bottom() - header.top()),
                    ).scale(window.scale_factor());
                    assert!(window.painted_quads().iter().any(|quad| {
                        quad.background == watcher_surface(palette).into()
                            && quad.bounds == surface
                    }), "Watch header and body must share one continuous painted surface");
                });
                let interject_bar = visual
                    .debug_bounds("watcher-interject-bar")
                    .expect("combined Chat and Steer pill is visible without scrolling");
                let interject_trigger = visual.debug_bounds("watcher-back").unwrap();
                for button in [chat, interject_trigger] {
                    visual.simulate_mouse_move(button.center(), None, Default::default());
                    draw(visual);
                    visual.executor().advance_clock(Duration::from_millis(250));
                    draw(visual);
                    visual.update(|window, _| {
                        let bounds = button.scale(window.scale_factor());
                        assert!(!window.painted_quads().iter().any(|quad| {
                            quad.bounds == bounds
                                && quad.background.as_solid().is_some_and(|color| color.a > 0.)
                        }), "Watch action hover must not add a filled button surface");
                    });
                }
                visual.simulate_mouse_move(root.origin, None, Default::default());
                draw(visual);
                let separator = visual
                    .debug_bounds("watcher-mode-separator")
                    .expect("Chat and Steer separator");
                assert!(
                    (interject_bar.center().x - root.center().x).abs() <= gpui_kit::px(1.),
                    "Chat and Steer pill stays centered within its pane"
                );
                assert!(
                    (root.bottom() - interject_bar.bottom() - px(8.)).abs()
                        <= gpui_kit::px(1.),
                    "Interject overlays the bottom of Watch: bar {interject_bar:?}, pane {root:?}, dock {:?}",
                    visual.debug_bounds("watcher-interject-dock")
                );
                assert!(
                    chat.left() >= interject_bar.left()
                        && chat.right() <= separator.left() + gpui_kit::px(1.)
                        && separator.right() <= interject_trigger.left() + gpui_kit::px(1.)
                        && interject_trigger.right() <= interject_bar.right(),
                    "one pill orders Chat, separator and Steer: {chat:?}, {separator:?}, {interject_trigger:?} in {interject_bar:?}"
                );
                assert!(
                    interject_bar.left() >= root.left() && interject_bar.right() <= root.right(),
                    "the floating bar fits narrow panes"
                );
                assert!(
                    visual.debug_bounds("send").is_none(),
                    "chat composer is hidden in Watch mode"
                );
                assert!(
                    visual.debug_bounds("working-plan-toggle").is_none(),
                    "plan access moved out of status pill"
                );
                assert!(visual.debug_bounds("watcher-now").is_some());
                let update = visual
                    .debug_bounds("watcher-latest-update")
                    .expect("the update remains visible without a heading");
                let line = visual
                    .debug_bounds("watcher-live-line")
                    .expect("the live line remains visible");
                let plan = visual
                    .debug_bounds("watcher-plan")
                    .expect("watch plan slot");
                assert!(
                    line.top() >= update.bottom() - gpui_kit::px(1.),
                    "the pulsing line follows the update: {line:?} after {update:?}"
                );
                assert!(
                    plan.top() >= line.bottom(),
                    "the reserved plan slot starts after the update: {plan:?} after {line:?}"
                );
                assert!(visual.debug_bounds("watcher-live-bar").is_none());
                assert!(visual.debug_bounds("watcher-agents").is_some());
                assert!(visual.debug_bounds("watcher-activity").is_some());
                let body = visual.debug_bounds("watcher-body").expect("inset body");
                assert!(
                    body.left() >= root.left() + gpui_kit::px(43.),
                    "Watch content keeps the added outer inset: {body:?} in {root:?}"
                );
                assert!(visual.debug_bounds("watcher-agent-cards").is_some());
                if width >= 800. {
                    let cards = [
                        visual
                            .debug_bounds("watcher-agent-row-0")
                            .expect("agent card"),
                        visual
                            .debug_bounds("watcher-agent-row-1")
                            .expect("agent card"),
                        visual
                            .debug_bounds("watcher-agent-row-2")
                            .expect("agent card"),
                        visual
                            .debug_bounds("watcher-agent-row-3")
                            .expect("agent card"),
                    ];
                    assert!(cards[1].top() >= cards[0].bottom());
                    assert!(cards[2].top() >= cards[1].bottom());
                    assert!(cards[3].top() >= cards[2].bottom());
                }
                visual.update(|window, cx| {
                    pane.update(cx, |pane, cx| {
                        assert!(!pane.composer_keyboard_available(window, cx));
                    })
                });
                pane.read_with(visual, |pane, cx| {
                    assert_eq!(
                        pane.composer.read(cx).value().to_string(),
                        "Keep this draft"
                    )
                });
                // Completion arrives through the same sync path as a real native publication.
                snapshot["tasks"][0]["status"] = json!("idle");
                snapshot["workPlans"][0]["status"] = json!("closed");
                for session in snapshot["subagentSessions"].as_array_mut().unwrap() {
                    session["status"] = json!("completed");
                }
                pane.update(visual, |pane, cx| {
                    pane.sync(Rc::new(snapshot.clone()), Some("done".into()), cx)
                });
                draw(visual);
                pane.read_with(visual, |pane, cx| {
                    assert!(pane.watcher.enabled, "completion stays visible");
                    let messages: Vec<Rc<Value>> = pane
                        .transcripts
                        .get(&pane.task_id())
                        .map(|state| state.messages.iter().cloned().collect::<Vec<_>>())
                        .unwrap_or_default();
                    let state = WatcherProjection::new(
                        &pane.snapshot,
                        "watch",
                        &pane.companion.read(cx).records(),
                        &messages,
                    );
                    assert_eq!(state.status, "idle");
                    assert_eq!(state.plan.unwrap()["status"], "closed");
                    assert!(
                        !state.plan_active,
                        "a closed plan no longer occupies the plan card"
                    );
                    assert_eq!(state.finished_plan.unwrap()["id"], "plan");
                    assert_eq!(state.running_agents, 0);
                });
                let mut finished = visual
                    .debug_bounds("watcher-finished-agents")
                    .expect("completed agents are disclosed by a left-aligned link");
                if finished.bottom() > root.bottom() {
                    visual.simulate_event(gpui_kit::ScrollWheelEvent {
                        position: root.center(),
                        delta: gpui_kit::ScrollDelta::Pixels(gpui_kit::point(
                            gpui_kit::px(0.),
                            gpui_kit::px(-600.),
                        )),
                        ..Default::default()
                    });
                    draw(visual);
                    finished = visual
                        .debug_bounds("watcher-finished-agents")
                        .expect("completed-agent disclosure stays mounted after scrolling");
                }
                visual.simulate_click(finished.center(), Default::default());
                draw(visual);
                let first_finished = visual
                    .debug_bounds("watcher-agent-row-0")
                    .or_else(|| {
                        visual.simulate_event(gpui_kit::ScrollWheelEvent {
                            position: finished.center(),
                            delta: gpui_kit::ScrollDelta::Pixels(gpui_kit::point(
                                gpui_kit::px(0.),
                                gpui_kit::px(-600.),
                            )),
                            ..Default::default()
                        });
                        draw(visual);
                        visual.debug_bounds("watcher-agent-row-0")
                    })
                    .expect("finished agents render after their disclosure");
                assert!(
                    first_finished.top() >= finished.bottom() - gpui_kit::px(1.),
                    "finished rows stay below their disclosure: {first_finished:?} after {finished:?}"
                );
                let back = visual.debug_bounds("watcher-back").unwrap();
                assert_eq!(
                    visual.debug_bounds("watcher-interject-bar").unwrap(),
                    interject_bar,
                    "content scrolling and completion do not move the overlay bar"
                );
                assert_eq!(
                    back, interject_trigger,
                    "Interject remains available at the same position after scrolling"
                );
                visual.simulate_click(back.center(), Default::default());
                draw(visual);
                let menu = visual
                    .debug_bounds("watcher-interject-menu")
                    .expect("anchored interject menu");
                let composer = visual
                    .debug_bounds("watcher-interject-composer")
                    .expect("interject composer");
                assert!(visual.debug_bounds("chat-watcher").is_some());
                assert!(menu.bottom() <= back.top() + gpui_kit::px(1.));
                assert!(composer.left() >= menu.left() && composer.right() <= menu.right());
                visual.update(|window, cx| {
                    pane.update(cx, |pane, cx| {
                        assert!(pane.watcher.enabled);
                        assert!(pane.watcher.interject_open);
                        assert!(pane.composer_keyboard_available(window, cx));
                    })
                });
                pane.read_with(visual, |pane, cx| {
                    assert_eq!(
                        pane.composer.read(cx).value().to_string(),
                        "Keep this draft"
                    )
                });
                visual.simulate_keystrokes("space");
                draw(visual);
                assert!(
                    visual.debug_bounds("watcher-interject-menu").is_some(),
                    "Space is captured by the focused Steer composer instead of dismissing it"
                );
                pane.read_with(visual, |pane, cx| {
                    assert_eq!(
                        pane.composer.read(cx).value().to_string(),
                        " Keep this draft",
                        "Steer retains the typed space at the existing caret"
                    )
                });
                visual.simulate_keystrokes("backspace");
                visual.simulate_keystrokes("escape");
                draw(visual);
                assert!(visual.debug_bounds("watcher-interject-menu").is_none());
                assert!(
                    visual.debug_bounds("chat-watcher").is_some(),
                    "first Escape dismisses only the interject menu"
                );
                visual.simulate_keystrokes("escape");
                visual
                    .executor()
                    .advance_clock(std::time::Duration::from_millis(601));
                draw(visual);
                assert!(
                    visual.debug_bounds("chat-watcher").is_none(),
                    "Escape returns to chat"
                );
                snapshot["approvalRequests"] = json!([{"id":"request","taskId":"watch","status":"pending","title":"Run tests","createdAt":20}]);
                visual.update(|window, cx| {
                    pane.update(cx, |pane, cx| {
                        pane.sync(Rc::new(snapshot.clone()), Some("request".into()), cx);
                        pane.toggle_watcher(window, cx);
                    })
                });
                visual
                    .executor()
                    .advance_clock(std::time::Duration::from_millis(601));
                draw(visual);
                assert!(
                    visual.debug_bounds("watcher-attention").is_some(),
                    "pending request stays visible"
                );
                visual.update(|window, cx| {
                    pane.update(cx, |pane, cx| {
                        pane.review_notification_request("request".into(), window, cx);
                        assert!(pane.approval_popup_open);
                    })
                });
                visual
                    .executor()
                    .advance_clock(std::time::Duration::from_millis(601));
                draw(visual);
                pane.read_with(visual, |pane, _| {
                    assert!(!pane.watcher.enabled, "notification must reveal request UI");
                });
                snapshot["approvalRequests"] = json!([]);
                pane.update(visual, |pane, cx| {
                    pane.sync(Rc::new(snapshot.clone()), Some("clear".into()), cx)
                });
                draw(visual);
            }
        }
    }

    #[gpui_kit::test]
    fn watcher_task_summary_stands_in_for_the_plan_when_none_is_active(
        cx: &mut TestAppContext,
    ) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::surface_blend::initialize_for_ui_fixture();
            crate::native_motion::initialize_for_ui_fixture(cx);
        });
        for theme in ["light", "dark"] {
            let mut snapshot = json!({"settings":{"theme":theme,"interfaceFontSize":14},
                "tasks":[{"id":"watch","title":"Ship the release","agentId":"agent","provider":"codex",
                    "model":"gpt-5-codex","status":"running","createdAt":10,"updatedAt":20,"queue":[]}],
                "agents":[{"id":"agent","name":"Codex","provider":"codex"}],
                "nativeMessageContext":{"watch":{"latestUserAt":10}},
                "workPlans":[{"id":"plan","taskId":"watch","title":"Cutover plan","status":"closed",
                    "createdAt":10,"updatedAt":10,"items":[
                        {"id":"one","title":"Land the migration","status":"completed"},
                        {"id":"two","title":"Publish the release notes","status":"pending"}]}],
                "subagentSessions":[],
                "messages":[{"id":"request","taskId":"watch","role":"user",
                    "text":"Finish the cutover and ship the release notes","createdAt":12,
                    "streamStatus":"complete"}],
                "events":[{"id":"update","taskId":"watch","kind":"agent_progress",
                    "title":"Checking the implementation","createdAt":11}],
                "approvalRequests":[],"queuedMessages":[],"hosts":[],"channels":[]});
            let mut retained = None;
            let (_, visual) = cx.add_window_view(|window, cx| {
                let pane = cx.new(|cx| {
                    ChatPane::performance_fixture(
                        "watch".into(),
                        Rc::new(snapshot.clone()),
                        window,
                        cx,
                    )
                });
                pane.update(cx, |pane, cx| {
                    pane.reconcile_history(
                        snapshot["messages"].as_array().unwrap().clone(),
                        true,
                        cx,
                    )
                });
                retained = Some(pane.clone());
                gpui_kit::component::Root::new(pane, window, cx)
            });
            let pane = retained.unwrap();
            visual.simulate_resize(size(gpui_kit::px(900.), gpui_kit::px(1000.)));
            pane.update(visual, |pane, cx| {
                pane.sync(Rc::new(snapshot.clone()), Some("active".into()), cx)
            });
            draw(visual);
            let toggle = visual
                .debug_bounds("chat-watcher-toggle")
                .expect("header toggle");
            visual.simulate_click(toggle.center(), Default::default());
            visual
                .executor()
                .advance_clock(std::time::Duration::from_millis(601));
            draw(visual);

            let plan_slot = visual.debug_bounds("watcher-plan").expect("plan slot");
            let summary = visual
                .debug_bounds("watcher-task-summary")
                .expect("the task summary stands in for the plan card");
            assert!(
                summary.top() >= plan_slot.top() && summary.bottom() <= plan_slot.bottom(),
                "the summary occupies the plan slot: {summary:?} in {plan_slot:?}"
            );
            let request = visual
                .debug_bounds("watcher-task-request")
                .expect("the current request is summarised");
            let disclosure = visual
                .debug_bounds("watcher-last-plan-toggle")
                .expect("the finished plan stays reachable");
            assert!(
                disclosure.top() >= request.bottom() - gpui_kit::px(1.)
                    && disclosure.bottom() <= summary.bottom() + gpui_kit::px(1.),
                "the finished plan sits below the request and inside the summary: \
                 {disclosure:?} in {request:?} in {summary:?}"
            );
            assert!(
                visual.debug_bounds("watcher-last-plan-card").is_none(),
                "the finished plan starts collapsed"
            );

            visual.simulate_click(disclosure.center(), Default::default());
            draw(visual);
            let card = visual
                .debug_bounds("watcher-last-plan-card")
                .expect("the finished plan opens in place");
            assert!(
                card.top() >= disclosure.top(),
                "the opened plan renders below its disclosure: {card:?} in {disclosure:?}"
            );
            pane.read_with(visual, |pane, _| {
                assert!(pane.watcher.finished_plan_expanded);
                assert!(pane.watcher.enabled, "Watch stays mounted");
            });

            // Reopening the plan hands the slot back to the plan card.
            snapshot["workPlans"][0]["status"] = json!("active");
            pane.update(visual, |pane, cx| {
                pane.sync(Rc::new(snapshot.clone()), Some("reopened".into()), cx)
            });
            draw(visual);
            assert!(visual.debug_bounds("watcher-plan").is_some());
            assert!(
                visual.debug_bounds("watcher-task-summary").is_none(),
                "an active plan owns the card again"
            );
            assert!(visual.debug_bounds("watcher-last-plan-toggle").is_none());
            snapshot["workPlans"][0]["status"] = json!("closed");
            pane.update(visual, |pane, cx| {
                pane.sync(Rc::new(snapshot.clone()), Some("closed".into()), cx)
            });
            draw(visual);
            assert!(
                visual.debug_bounds("watcher-task-summary").is_some(),
                "closing the plan restores the summary"
            );
            assert!(
                visual.debug_bounds("watcher-last-plan-card").is_some(),
                "the disclosure keeps the state the watcher already reached"
            );
        }
    }
}
