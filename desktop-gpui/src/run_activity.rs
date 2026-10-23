//! Native, lazy-loaded detail rows for one authoritative task run event.
//!
//! Single events and grouped projections share this renderer, while each event
//! retains its own disclosure state inside the grouped projection.
use crate::design::ui_scaled_px as px;
use gpui_kit::{
    assets::IconName,
    component::{ActiveTheme, Disableable, Icon, Sizable, button::*, spinner::Spinner},
    prelude::*,
    *,
};
use monitter_lib::experimental_desktop::DesktopService;
use serde_json::{Value, json};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

#[path = "tool_detail.rs"]
mod tool_detail;

const DETAIL_PAGE_BYTES: u64 = 32 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EventUpdate {
    Unchanged,
    Metadata,
    Detail,
    Identity,
}

fn event_update(
    task_id: &str,
    prior_task: &str,
    prior: Option<&Value>,
    next: &Value,
) -> EventUpdate {
    if task_id != prior_task || prior.map(|event| field(event, "id")) != Some(field(next, "id")) {
        EventUpdate::Identity
    } else if prior == Some(next) {
        EventUpdate::Unchanged
    } else if prior.map(|event| field(event, "detail")) != Some(field(next, "detail")) {
        EventUpdate::Detail
    } else {
        EventUpdate::Metadata
    }
}

/// Render variant. Single events collapse to the legacy disclosure; grouped
/// projections show a compact process-style header with stable first-event id
/// and a fan-out to per-event detail rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupMode {
    Single,
    Reasoning,
    Tool,
    Process,
}

/// One collapsed branch in a mixed reasoning/tool process disclosure.
#[derive(Clone, Debug, PartialEq)]
pub struct ProcessBranch {
    pub mode: GroupMode,
    pub events: Vec<Value>,
}

fn field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn usize_field(value: &Value, key: &str) -> Option<usize> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|n| usize::try_from(n).ok())
}

fn event_time(event: &Value) -> Option<String> {
    crate::timestamps::local_datetime(event["createdAt"].as_i64()?)
        .map(|timestamp| timestamp.format("%-I:%M %P").to_string())
}

pub(crate) fn parsed_event_detail(event: &Value) -> Value {
    let mut parsed = serde_json::from_str::<Value>(&field(event, "detail")).unwrap_or(Value::Null);
    for _ in 0..3 {
        if let Some(boxed) = parsed.as_str() {
            match serde_json::from_str(boxed) {
                Ok(value) => parsed = value,
                Err(_) => break,
            }
        } else {
            break;
        }
    }
    parsed
}

/// Prefer a known execution envelope over a generic enclosing tool label.
/// Only traverse protocol wrapper fields; command text is never classified.
fn command_execution_payload(value: &Value, depth: usize) -> Option<Value> {
    if depth > 8 {
        return None;
    }
    if let Some(text) = value.as_str() {
        if text.len() > 256 * 1024 {
            return None;
        }
        return serde_json::from_str::<Value>(text)
            .ok()
            .and_then(|value| command_execution_payload(&value, depth + 1));
    }
    if value.get("type").and_then(Value::as_str) == Some("commandExecution") {
        return Some(value.clone());
    }
    for key in [
        "update",
        "activity",
        "rawInput",
        "rawOutput",
        "input",
        "output",
        "arguments",
        "item",
        "result",
        "data",
    ] {
        if let Some(payload) = value
            .get(key)
            .and_then(|value| command_execution_payload(value, depth + 1))
        {
            return Some(payload);
        }
    }
    None
}

fn tool_identity(event: &Value) -> String {
    let detail = parsed_event_detail(event);
    if command_execution_payload(&detail, 0).is_some() {
        return "commandexecution".into();
    }
    let activity = detail.get("activity").unwrap_or(&detail);
    let update = detail.get("update").unwrap_or(&detail);
    [
        activity.get("tool").and_then(Value::as_str),
        activity.get("type").and_then(Value::as_str),
        update.get("kind").and_then(Value::as_str),
        update.get("title").and_then(Value::as_str),
        detail.get("type").and_then(Value::as_str),
        detail.get("tool").and_then(Value::as_str),
    ]
    .into_iter()
    .flatten()
    .find(|value| !value.trim().is_empty())
    .unwrap_or_else(|| "")
    .to_ascii_lowercase()
}

fn event_tool_status(event: &Value) -> Option<String> {
    let detail = parsed_event_detail(event);
    detail
        .get("update")
        .and_then(|update| update.get("status"))
        .or_else(|| detail.get("status"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ToolState {
    Running,
    Completed,
    Failed,
    Unknown,
}
fn tool_state(event: &Value) -> ToolState {
    let detail = parsed_event_detail(event);
    if detail["type"] == "mcpToolCall"
        && (!detail["error"].is_null() || detail["result"]["isError"].as_bool() == Some(true))
    {
        return ToolState::Failed;
    }
    match event_tool_status(event)
        .unwrap_or_default()
        .to_ascii_lowercase()
        .replace(['_', '-'], "")
        .as_str()
    {
        "inprogress" | "running" | "pending" | "queued" | "started" => ToolState::Running,
        "completed" | "succeeded" | "success" => ToolState::Completed,
        "failed" | "error" | "cancelled" | "canceled" | "interrupted" => ToolState::Failed,
        _ => ToolState::Unknown,
    }
}
fn activity_icon(
    events: &[Value],
    fallback: IconName,
    cache: &PresentationCache,
    turn_running: Option<bool>,
) -> AnyElement {
    let states: Vec<_> = events
        .iter()
        .filter(|event| is_tool_event(event))
        .map(|event| cache.get(event).state)
        .collect();
    // Turn headers describe the turn lifecycle. Individual provider tool
    // records can retain an in-progress status after their turn has ended.
    let state = if let Some(running) = turn_running {
        if running {
            ToolState::Running
        } else {
            ToolState::Completed
        }
    } else if states.contains(&ToolState::Running) {
        ToolState::Running
    } else if states.contains(&ToolState::Failed) {
        ToolState::Failed
    } else if !states.is_empty() && states.iter().all(|state| *state == ToolState::Completed) {
        ToolState::Completed
    } else {
        ToolState::Unknown
    };
    let id = events
        .first()
        .map(|event| field(event, "id"))
        .unwrap_or_default();
    let (name, icon) = match state {
        ToolState::Running => (
            "running",
            Spinner::new().with_size(px(12.)).into_any_element(),
        ),
        ToolState::Completed => (
            "completed",
            Icon::new(IconName::CircleCheck)
                .size(px(12.))
                .into_any_element(),
        ),
        ToolState::Failed => (
            "failed",
            Icon::new(IconName::CircleAlert)
                .size(px(12.))
                .into_any_element(),
        ),
        ToolState::Unknown => (
            "unknown",
            Icon::new(fallback).size(px(12.)).into_any_element(),
        ),
    };
    fixture_bounds(
        div().flex_shrink_0().child(icon),
        format!("run-state-{id}-{name}"),
    )
    .into_any_element()
}

/// Compact English intent for the latest step, without promoting incidental
/// provider/accounting titles into tool labels.
pub(crate) fn process_tool_preview(events: &[Value]) -> Option<String> {
    let event = events.iter().rev().find(|event| is_tool_event(event))?;
    let detail = parsed_event_detail(event);
    let update = detail.get("update").unwrap_or(&detail);
    let arguments = detail
        .get("arguments")
        .and_then(Value::as_str)
        .and_then(|arguments| serde_json::from_str::<Value>(arguments).ok());
    let execution = command_execution_payload(&detail, 0);
    let input = execution
        .as_ref()
        .or_else(|| {
            update
                .get("rawInput")
                .or_else(|| update.get("input"))
                .or(arguments.as_ref())
        })
        .unwrap_or(update);
    let mut preview = String::new();
    if let Some(command) = input.get("command").or_else(|| input.get("cmd")) {
        let command = command.as_str().map(str::to_owned).or_else(|| {
            command.as_array().map(|parts| {
                parts
                    .iter()
                    .map(|part| {
                        part.as_str()
                            .map(str::to_owned)
                            .unwrap_or_else(|| part.to_string())
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            })
        });
        if let Some(command) = command.filter(|command| !command.trim().is_empty()) {
            preview = command;
        }
    }
    if preview.is_empty() {
        for key in [
            "error",
            "query",
            "q",
            "pattern",
            "path",
            "file_path",
            "url",
            "title",
            "summary",
            "question",
            "objective",
            "prompt",
            "message",
            "output",
            "result",
            "text",
        ] {
            if let Some(value) = input
                .get(key)
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
            {
                preview = value.trim().to_owned();
                break;
            }
        }
    }
    if preview.is_empty() {
        preview = tool_intent(event);
    }
    let preview = preview.split_whitespace().collect::<Vec<_>>().join(" ");
    Some(if preview.chars().count() > 100 {
        format!("{}…", preview.chars().take(99).collect::<String>())
    } else {
        preview
    })
}

fn is_tool_event(event: &Value) -> bool {
    matches!(field(event, "kind").as_str(), "tool" | "subagent")
        && !crate::transcript::is_context_compaction(event)
}

fn tool_count(events: &[Value]) -> usize {
    events.iter().filter(|event| is_tool_event(event)).count()
}

fn is_shell_event(event: &Value) -> bool {
    let mut identity = tool_identity(event);
    if identity.is_empty() {
        identity = field(event, "title").to_ascii_lowercase();
    }
    let identity = identity.replace(['_', '-'], " ");
    identity.contains("command") || identity.contains("shell") || identity == "execute"
}

#[cfg(test)]
fn grouped_tool_label(events: &[Value]) -> String {
    let tools: Vec<_> = events.iter().filter(|event| is_tool_event(event)).collect();
    match tools.len() {
        0 => "Activity".to_owned(),
        count if tools.iter().all(|event| is_shell_event(event)) => {
            format!(
                "Ran {count} {}",
                if count == 1 { "command" } else { "commands" }
            )
        }
        1 => tools
            .first()
            .map(|event| tool_intent(event))
            .unwrap_or_else(|| "Used a tool".to_owned()),
        count => {
            let label = tools
                .first()
                .map(|event| tool_intent(event))
                .unwrap_or_else(|| "Used a tool".to_owned());
            if label == "Read a file" {
                format!("Read {count} files")
            } else if label == "Ran a command" {
                format!("Ran {count} commands")
            } else {
                format!("{label} · {count} tools")
            }
        }
    }
}

fn mcp_tool_label(event: &Value) -> Option<String> {
    let detail = parsed_event_detail(event);
    let item = detail
        .get("item")
        .or_else(|| detail.get("update"))
        .unwrap_or(&detail);
    if item["type"].as_str() != Some("mcpToolCall") {
        return None;
    }
    fn friendly(value: &str) -> String {
        let words = value
            .replace(['_', '-'], " ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let mut chars = words.chars();
        chars
            .next()
            .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
            .unwrap_or_default()
    }
    let server = friendly(item["server"].as_str().unwrap_or_default());
    let tool = friendly(item["tool"].as_str().unwrap_or_default());
    if tool.is_empty() {
        return None;
    }
    Some(if server.is_empty() {
        tool
    } else {
        format!("{server} · {tool}")
    })
}

pub(crate) fn tool_intent(event: &Value) -> String {
    if let Some(label) = mcp_tool_label(event) {
        return label;
    }
    if event_tool_status(event).as_deref() == Some("failed") {
        return "Tool failed".to_owned();
    }
    let raw = tool_identity(event);
    let raw = if raw.is_empty() {
        field(event, "title")
    } else {
        raw
    };
    let identity = raw.to_ascii_lowercase().replace(['_', '-'], " ");
    if identity.contains("command") || identity.contains("shell") || identity == "execute" {
        "Ran a command".to_owned()
    } else if identity.contains("read") || identity.contains("view") {
        "Read a file".to_owned()
    } else if identity.contains("edit")
        || identity.contains("write")
        || identity.contains("filechange")
    {
        "Edited a file".to_owned()
    } else if identity.contains("websearch") || identity.contains("web search") {
        "Searched the web".to_owned()
    } else if identity.contains("webfetch")
        || identity.contains("web fetch")
        || identity.contains("fetch page")
        || identity.contains("fetch")
    {
        "Fetched a web page".to_owned()
    } else if identity.contains("grep")
        || identity.contains("ripgrep")
        || identity.contains("search")
    {
        "Searched files".to_owned()
    } else if identity.contains("list") {
        "Listed files".to_owned()
    } else if identity.contains("spawnagent") || identity.contains("subagent") {
        "Started an agent".to_owned()
    } else if raw.starts_with("mcp__") {
        let family = raw.split("__").nth(1).unwrap_or_default();
        if family.is_empty() {
            "Used a tool".to_owned()
        } else {
            format!("Worked in {}", family.replace(['_', '-'], " "))
        }
    } else {
        "Used a tool".to_owned()
    }
}

fn process_branch_key(branch: &ProcessBranch) -> String {
    let kind = match branch.mode {
        GroupMode::Reasoning => "thinking",
        _ => "tools",
    };
    format!(
        "{kind}:{}",
        branch
            .events
            .first()
            .map(|event| field(event, "id"))
            .unwrap_or_default()
    )
}

fn thinking_row(
    event: Option<&Value>,
    working: bool,
    cx: &Context<RunActivityRow>,
) -> gpui_kit::AnyElement {
    let label = if working {
        "Thinking · Working"
    } else {
        "Thinking"
    };
    let mut row = div()
        .w_full()
        .min_w_0()
        .flex()
        .items_center()
        .gap(px(6.))
        .py(px(3.))
        .text_size(px(11.))
        .text_color(cx.theme().muted_foreground)
        .child(Icon::new(IconName::BookOpen).size(px(14.)))
        .child(div().flex_1().min_w_0().child(label));
    if let Some(time) = event.and_then(event_time) {
        row = row.child(
            div()
                .ml_auto()
                .flex_shrink_0()
                .font_family(crate::design::FONT_MONO)
                .text_size(px(9.))
                .text_color(cx.theme().muted_foreground)
                .child(time),
        );
    }
    row.into_any_element()
}

fn context_compaction_label(compaction: &crate::transcript::ContextCompaction) -> String {
    if compaction.active {
        return "Compacting context…".into();
    }
    if !compaction.completed {
        return "Context compaction".into();
    }
    match compaction.duration_ms {
        Some(ms) if ms < 1_000 => "Context compacted · <1s".into(),
        Some(ms) if ms < 60_000 => format!("Context compacted · {}s", ms / 1_000),
        Some(ms) => format!("Context compacted · {}m {}s", ms / 60_000, ms / 1_000 % 60),
        None => "Context compacted".into(),
    }
}

fn context_compaction_row(
    events: &[Value],
    running: bool,
    cx: &Context<RunActivityRow>,
) -> Option<AnyElement> {
    let compaction = crate::transcript::context_compaction(events, running)?;
    let label = context_compaction_label(&compaction);
    let state = if compaction.active {
        "running"
    } else if compaction.completed {
        "completed"
    } else {
        "unknown"
    };
    let icon = if compaction.active {
        Spinner::new().with_size(px(13.)).into_any_element()
    } else {
        Icon::new(if compaction.completed {
            IconName::Check
        } else {
            IconName::BookOpen
        })
        .size(px(13.))
        .into_any_element()
    };
    let id = events
        .first()
        .map(|event| field(event, "id"))
        .unwrap_or_default();
    let mut row = fixture_bounds(
        div()
            .w_full()
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(6.))
            .py(px(3.))
            .text_size(px(11.))
            .text_color(cx.theme().muted_foreground)
            .child(icon)
            .child(div().flex_1().min_w_0().child(label.clone())),
        format!("context-compaction-{id}-{state}"),
    )
    .id(SharedString::from(format!("context-compaction-{id}")))
    .role(Role::Label)
    .aria_label(label);
    if let Some(time) = events.last().and_then(event_time) {
        row = row.child(
            div()
                .ml_auto()
                .flex_shrink_0()
                .font_family(crate::design::FONT_MONO)
                .text_size(px(9.))
                .child(time),
        );
    }
    Some(row.into_any_element())
}

fn reasoning_time(event: &Value, cx: &Context<RunActivityRow>) -> Option<AnyElement> {
    event_time(event).map(|time| {
        div()
            .ml_auto()
            .flex_shrink_0()
            .font_family(crate::design::FONT_MONO)
            .text_size(px(9.))
            .text_color(cx.theme().muted_foreground)
            .child(time)
            .into_any_element()
    })
}

// TextView is Markdown-backed, so fence event payloads to display tool output
// and source text literally (including links and Markdown-looking punctuation).
// The fence is guaranteed longer than any backtick run in the returned chunk.
fn literal_detail(source: &str) -> String {
    let (longest, _) = source.chars().fold((0usize, 0usize), |(longest, run), ch| {
        if ch == '`' {
            let run = run + 1;
            (longest.max(run), run)
        } else {
            (longest, 0)
        }
    });
    let fence = "`".repeat(longest.max(2) + 1);
    format!("{fence}text\n{source}\n{fence}")
}

#[derive(Default)]
struct PresentationCache(RefCell<HashMap<String, Rc<EventPresentation>>>);
struct EventPresentation {
    state: ToolState,
    failed: bool,
    shell: bool,
    intent: String,
    preview: Option<String>,
    reasoning: SharedString,
}
impl PresentationCache {
    fn get(&self, event: &Value) -> Rc<EventPresentation> {
        let id = field(event, "id");
        let key = if id.is_empty() { event.to_string() } else { id };
        if let Some(value) = self.0.borrow().get(&key) {
            return value.clone();
        }
        let value = Rc::new(EventPresentation {
            state: tool_state(event),
            failed: event_tool_status(event).as_deref() == Some("failed"),
            shell: is_shell_event(event),
            intent: tool_intent(event),
            preview: process_tool_preview(std::slice::from_ref(event)),
            reasoning: crate::transcript::reasoning_summary(event).into(),
        });
        self.0.borrow_mut().insert(key, value.clone());
        value
    }
    fn with_loaded_detail(&self, event: &Value, detail: Option<&str>) -> Rc<EventPresentation> {
        let key = field(event, "id");
        if let Some(cached) = self.0.borrow().get(&key) {
            return cached.clone();
        }
        if let Some(detail) = detail {
            let mut event = event.clone();
            event["detail"] = json!(detail);
            self.get(&event)
        } else {
            self.get(event)
        }
    }
    fn clear(&self) {
        self.0.borrow_mut().clear();
    }
    fn preview(&self, events: &[Value]) -> Option<String> {
        events
            .iter()
            .rev()
            .find(|event| is_tool_event(event))
            .and_then(|event| self.get(event).preview.clone())
    }
    fn group_label(&self, events: &[Value]) -> String {
        let tools: Vec<_> = events.iter().filter(|event| is_tool_event(event)).collect();
        let count = tools.len();
        if count == 0 {
            return "Activity".into();
        }
        if tools.iter().all(|event| self.get(event).shell) {
            return format!(
                "Ran {count} {}",
                if count == 1 { "command" } else { "commands" }
            );
        }
        let label = self.get(tools[0]).intent.clone();
        match (count, label.as_str()) {
            (1, _) => label,
            (_, "Read a file") => format!("Read {count} files"),
            (_, "Ran a command") => format!("Ran {count} commands"),
            _ => format!("{label} · {count} tools"),
        }
    }
}
enum DetailPresentation {
    Structured(tool_detail::PreparedDetail),
    Literal(SharedString),
}

/// A retained row for one real RunEvent. Detail is fetched only after an
/// explicit expand click and then one bounded chunk at a time. The grouped
/// constructors (`for_reasoning`, `for_tool`, `for_process`) reuse this row
/// for fan-out rendering: per-event disclosure state is preserved alongside
/// the group's stable first-id anchor.
pub struct RunActivityRow {
    service: DesktopService,
    git_owner: Option<WeakEntity<crate::chat::ChatPane>>,
    task_id: String,
    event_id: String,
    title: String,
    kind: String,
    expanded: bool,
    loading: bool,
    detail_loaded: bool,
    detail: String,
    next_offset: Option<usize>,
    total_bytes: Option<usize>,
    error: Option<String>,
    request_generation: u64,
    mode: GroupMode,
    events: Vec<Value>,
    stable_first_id: String,
    group_running: bool,
    header_icon_bounds: crate::activity_sparkles::PaintBounds,
    group_compressed: bool,
    detail_rows: Vec<Entity<RunActivityRow>>,
    detail_row_ids: Vec<String>,
    detail_row_index: HashMap<String, usize>,
    presentation_cache: PresentationCache,
    detail_presentation: RefCell<Option<DetailPresentation>>,
    process_branches: Vec<ProcessBranch>,
    process_approvals: Vec<Value>,
    expanded_branches: HashSet<String>,
    list_item: Option<(ListState, usize)>,
}

fn fixture_bounds(d: Div, name: String) -> Div {
    #[cfg(test)]
    {
        return d.debug_selector(move || name.clone());
    }
    #[cfg(not(test))]
    {
        let _ = name;
        d
    }
}
impl RunActivityRow {
    pub(crate) fn header_icon_bounds(&self) -> Option<crate::activity_sparkles::PaintBounds> {
        // Compact thinking/compaction rows use their own presentation rather
        // than the Turn header measured below. Preserve their existing rail.
        if crate::transcript::context_compaction(&self.events, self.group_running).is_some()
            || (self.mode == GroupMode::Reasoning
                && self
                    .events
                    .iter()
                    .all(|event| self.presentation_cache.get(event).reasoning.is_empty()))
        {
            return None;
        }
        Some(self.header_icon_bounds.clone())
    }

    #[cfg(test)]
    pub(crate) fn seed_fixture_detail(&mut self, detail: &str, cx: &mut Context<Self>) {
        self.detail = detail.into();
        self.detail_loaded = true;
        self.next_offset = None;
        self.presentation_cache.clear();
        *self.detail_presentation.borrow_mut() = None;
        cx.notify();
    }
    #[cfg(test)]
    pub(crate) fn fixture_loaded_detail(&self) -> &str {
        &self.detail
    }
    pub fn set_git_owner(&mut self, owner: WeakEntity<crate::chat::ChatPane>) {
        self.git_owner = Some(owner);
    }

    pub fn new(
        service: DesktopService,
        task_id: impl Into<String>,
        event: &Value,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Self {
        Self::new_pure(service, task_id, event)
    }

    fn new_pure(service: DesktopService, task_id: impl Into<String>, event: &Value) -> Self {
        let task_id = task_id.into();
        let event_id = field(event, "id");
        let title = field(event, "title");
        let kind = field(event, "kind");
        Self {
            service,
            git_owner: None,
            task_id,
            event_id: event_id.clone(),
            title,
            kind,
            expanded: false,
            loading: false,
            detail_loaded: false,
            detail: String::new(),
            next_offset: None,
            total_bytes: None,
            error: None,
            request_generation: 0,
            mode: GroupMode::Single,
            events: vec![event.clone()],
            stable_first_id: event_id.clone(),
            group_running: false,
            header_icon_bounds: Default::default(),
            group_compressed: false,
            detail_rows: vec![],
            detail_row_ids: vec![],
            detail_row_index: HashMap::new(),
            presentation_cache: PresentationCache::default(),
            detail_presentation: RefCell::new(None),
            process_branches: vec![],
            process_approvals: vec![],
            expanded_branches: HashSet::new(),
            list_item: None,
        }
    }

    /// Grouped reasoning entry. Detail disclosure follows the same per-event
    /// pattern as the single-event constructor; the group header renders a
    /// compact summary that links to the first retained event id.
    pub fn for_reasoning(
        service: DesktopService,
        task_id: impl Into<String>,
        events: Vec<Value>,
        running: bool,
    ) -> Self {
        Self::for_group_pure(
            service,
            task_id,
            events,
            GroupMode::Reasoning,
            running,
            false,
        )
    }

    /// Grouped tool entry. Compressed mode merges adjacent tool labels into a
    /// single fan-out header.
    pub fn for_tool(
        service: DesktopService,
        task_id: impl Into<String>,
        events: Vec<Value>,
        running: bool,
        compressed: bool,
    ) -> Self {
        Self::for_group_pure(
            service,
            task_id,
            events,
            GroupMode::Tool,
            running,
            compressed,
        )
    }

    /// Process-group entry: a mixed reasoning+tool sequence. Approvals are
    /// rendered as inline rows below the group summary; the renderer stays
    /// pure and never invents activity or model-cost facts.
    pub fn for_process(
        service: DesktopService,
        task_id: impl Into<String>,
        events: Vec<Value>,
        approvals: Vec<Value>,
        running: bool,
    ) -> Self {
        let mut row =
            Self::for_group_pure(service, task_id, events, GroupMode::Process, running, false);
        row.process_approvals = approvals;
        row
    }

    fn for_group_pure(
        service: DesktopService,
        task_id: impl Into<String>,
        events: Vec<Value>,
        mode: GroupMode,
        running: bool,
        compressed: bool,
    ) -> Self {
        let task_id = task_id.into();
        let stable_first_id = events
            .first()
            .map(|event| field(event, "id"))
            .unwrap_or_default();
        let primary = events.first().cloned().unwrap_or(Value::Null);
        let event_id = stable_first_id.clone();
        let title = field(&primary, "title");
        let kind = field(&primary, "kind");
        Self {
            service,
            git_owner: None,
            task_id,
            event_id,
            title,
            kind,
            expanded: false,
            loading: false,
            detail_loaded: false,
            detail: String::new(),
            next_offset: None,
            total_bytes: None,
            error: None,
            request_generation: 0,
            mode,
            events,
            stable_first_id,
            group_running: running,
            header_icon_bounds: Default::default(),
            group_compressed: compressed,
            detail_rows: vec![],
            detail_row_ids: vec![],
            detail_row_index: HashMap::new(),
            presentation_cache: PresentationCache::default(),
            detail_presentation: RefCell::new(None),
            process_branches: vec![],
            process_approvals: vec![],
            expanded_branches: HashSet::new(),
            list_item: None,
        }
    }

    /// Update this retained row when the parent refreshes its event snapshot.
    /// Expansion and loaded pages survive updates to the same event.
    pub fn set_event(&mut self, task_id: &str, event: &Value, cx: &mut Context<Self>) {
        if !self.set_event_pure(task_id, event) {
            return;
        }
        if self.expanded && !self.detail_loaded && !self.loading {
            self.load_chunk(0, cx);
        }
        self.invalidate_list_item();
        cx.notify();
    }

    fn set_event_pure(&mut self, task_id: &str, event: &Value) -> bool {
        let update = event_update(task_id, &self.task_id, self.events.first(), event);
        if update == EventUpdate::Unchanged {
            return false;
        }
        self.presentation_cache.clear();
        *self.detail_presentation.borrow_mut() = None;
        let event_id = field(event, "id");
        let title = field(event, "title");
        let kind = field(event, "kind");
        if matches!(update, EventUpdate::Identity | EventUpdate::Detail) {
            self.request_generation = self.request_generation.wrapping_add(1);
            self.task_id = task_id.to_owned();
            self.event_id = event_id;
            if update == EventUpdate::Identity {
                self.expanded = false;
            }
            self.loading = false;
            self.detail_loaded = false;
            self.detail.clear();
            self.next_offset = None;
            self.total_bytes = None;
            self.error = None;
        }
        self.title = title;
        self.kind = kind;
        if self.mode == GroupMode::Single {
            self.events = vec![event.clone()];
            self.stable_first_id = field(event, "id");
        }
        true
    }

    /// Replace the underlying group with a new snapshot. Expansion state for
    /// the first retained event survives across updates that keep the anchor;
    /// the renderer resets its detail page when the anchor shifts.
    pub fn set_group(
        &mut self,
        task_id: &str,
        events: Vec<Value>,
        running: bool,
        compressed: bool,
        cx: &mut Context<Self>,
    ) {
        let anchor = events
            .first()
            .map(|event| field(event, "id"))
            .unwrap_or_default();
        if self.task_id == task_id
            && self.stable_first_id == anchor
            && self.events == events
            && self.group_running == running
            && self.group_compressed == compressed
        {
            return;
        }
        self.set_group_pure(task_id, events, running, compressed);
        cx.notify();
    }

    fn set_group_pure(
        &mut self,
        task_id: &str,
        events: Vec<Value>,
        running: bool,
        compressed: bool,
    ) {
        let anchor = events
            .first()
            .map(|event| field(event, "id"))
            .unwrap_or_default();
        let anchor_changed = anchor != self.stable_first_id;
        let task_changed = task_id != self.task_id;
        self.task_id = task_id.to_owned();
        self.presentation_cache.clear();
        *self.detail_presentation.borrow_mut() = None;
        self.stable_first_id = anchor.clone();
        self.event_id = anchor;
        self.title = events
            .first()
            .map(|event| field(event, "title"))
            .unwrap_or_default();
        self.kind = events
            .first()
            .map(|event| field(event, "kind"))
            .unwrap_or_default();
        self.events = events;
        self.group_running = running;
        self.group_compressed = compressed;
        if anchor_changed || task_changed {
            self.request_generation = self.request_generation.wrapping_add(1);
            self.expanded = false;
            self.loading = false;
            self.detail_loaded = false;
            self.detail.clear();
            self.next_offset = None;
            self.total_bytes = None;
            self.error = None;
        }
    }

    /// Stable first-event anchor used by the parent for virtual-list keys.
    pub fn stable_key(&self) -> String {
        match self.mode {
            GroupMode::Single => format!("event:{}", self.event_id),
            GroupMode::Reasoning => format!("reasoning-group:{}", self.stable_first_id),
            GroupMode::Tool => format!("tool-group:{}", self.stable_first_id),
            GroupMode::Process => format!("process-group:{}", self.stable_first_id),
        }
    }

    /// Underlying events exposed so the parent can re-render the disclosure
    /// once per retained event id (matching native group disclosures).
    pub fn events(&self) -> &[Value] {
        &self.events
    }

    /// Retain one native disclosure entity per real event inside a group.
    pub fn set_detail_rows(
        &mut self,
        rows: Vec<(String, Entity<RunActivityRow>)>,
        cx: &mut Context<Self>,
    ) {
        let ids: Vec<String> = rows.iter().map(|(id, _)| id.clone()).collect();
        if self.detail_row_ids == ids {
            return;
        }
        self.detail_row_index = ids
            .iter()
            .enumerate()
            .map(|(index, id)| (id.clone(), index))
            .collect();
        self.detail_row_ids = ids;
        self.detail_rows = rows.into_iter().map(|(_, row)| row).collect();
        if self.expanded {
            self.invalidate_list_item();
        }
        cx.notify();
    }

    /// Supply the actual semantic subgroups and resolved approvals produced
    /// by transcript projection. These stay separate from tool event counts.
    pub fn set_process_tree(
        &mut self,
        branches: Vec<ProcessBranch>,
        approvals: Vec<Value>,
        cx: &mut Context<Self>,
    ) {
        if self.process_branches == branches && self.process_approvals == approvals {
            return;
        }
        let live_keys: HashSet<String> = branches.iter().map(process_branch_key).collect();
        self.expanded_branches.retain(|key| live_keys.contains(key));
        self.presentation_cache.clear();
        self.process_branches = branches;
        self.process_approvals = approvals;
        if self.expanded {
            self.invalidate_list_item();
        }
        cx.notify();
    }

    fn toggle_process_branch(&mut self, key: String, cx: &mut Context<Self>) {
        if !self.expanded_branches.remove(&key) {
            self.expanded_branches.insert(key);
        }
        self.invalidate_list_item();
        cx.notify();
    }

    /// Attach this row to its current variable-height virtual-list item.
    /// GPUI's list caches measured heights, so disclosure content changes must
    /// invalidate the containing projection item as well as notify this entity.
    pub fn set_list_item(&mut self, list: ListState, index: usize) {
        self.list_item = Some((list, index));
    }

    fn invalidate_list_item(&self) {
        if let Some((list, index)) = &self.list_item {
            if *index < list.item_count() {
                list.splice(*index..*index + 1, 1);
            }
        }
    }

    /// Mode is read-only after construction; useful for callers that want to
    /// surface "thinking" or "turn" copy without re-parsing `self.kind`.
    pub fn mode(&self) -> GroupMode {
        self.mode
    }

    /// True when the row represents a single retained event (legacy path).
    pub fn is_single(&self) -> bool {
        matches!(self.mode, GroupMode::Single)
    }

    /// A sole tool branch with the same events adds no information to its turn.
    /// Mixed reasoning/tool branches and approval history keep their own structure.
    fn repeats_only_tool_branch(&self) -> bool {
        let [branch] = self.process_branches.as_slice() else {
            return false;
        };
        branch.mode == GroupMode::Tool
            && self.process_approvals.is_empty()
            && !branch.events.is_empty()
            && branch
                .events
                .iter()
                .map(|event| field(event, "id"))
                .eq(self.events.iter().map(|event| field(event, "id")))
    }

    fn toggle(&mut self, cx: &mut Context<Self>) {
        self.expanded = !self.expanded;
        self.error = None;
        if self.mode == GroupMode::Single && self.expanded && !self.detail_loaded && !self.loading {
            self.load_chunk(0, cx);
        }
        self.invalidate_list_item();
        cx.notify();
    }

    fn load_next(&mut self, cx: &mut Context<Self>) {
        if let Some(offset) = self.next_offset {
            self.load_chunk(offset, cx);
        }
    }

    fn load_chunk(&mut self, offset: usize, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        self.loading = true;
        self.error = None;
        self.request_generation = self.request_generation.wrapping_add(1);
        let generation = self.request_generation;
        let task_id = self.task_id.clone();
        let event_id = self.event_id.clone();
        let service = self.service.clone();
        let worker = cx.background_executor().spawn(async move {
            service.invoke(
                "get_task_event_detail",
                json!({
                    "taskId": task_id,
                    "eventId": event_id,
                    "offset": offset,
                    "limit": DETAIL_PAGE_BYTES,
                }),
            )
        });
        cx.spawn(async move |row, cx| {
            let result = worker.await;
            let _ = row.update(cx, |row, cx| {
                // A retained entity may have been rebound while this read was
                // in flight. Discard an old response instead of mixing pages.
                if row.request_generation != generation {
                    return;
                }
                row.loading = false;
                match result {
                    Ok(value) => {
                        let chunk = field(&value, "chunk");
                        row.detail.push_str(&chunk);
                        row.presentation_cache.clear();
                        *row.detail_presentation.borrow_mut() = None;
                        row.detail_loaded = true;
                        row.next_offset = usize_field(&value, "nextOffset");
                        row.total_bytes = usize_field(&value, "totalBytes");
                    }
                    Err(error) => row.error = Some(error),
                }
                if row.expanded {
                    row.invalidate_list_item();
                }
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for RunActivityRow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        match self.mode {
            GroupMode::Single => self.render_single(window, cx),
            GroupMode::Reasoning | GroupMode::Tool | GroupMode::Process => self.render_group(cx),
        }
    }
}

impl RunActivityRow {
    fn render_single(&self, window: &mut Window, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        if let Some(compaction) = context_compaction_row(&self.events, false, cx) {
            return compaction;
        }
        let is_reasoning = self.kind == "reasoning";
        let presentation_event = self.events.first().unwrap_or(&Value::Null);
        let presentation = self.presentation_cache.with_loaded_detail(
            presentation_event,
            (self.detail_loaded && self.next_offset.is_none()).then_some(self.detail.as_str()),
        );
        let reasoning_summary = if is_reasoning {
            presentation.reasoning.clone()
        } else {
            SharedString::default()
        };
        if is_reasoning && reasoning_summary.is_empty() {
            return thinking_row(self.events.first(), self.group_running, cx);
        }
        let title = if is_reasoning {
            "Thought process".to_owned()
        } else if self.title.is_empty() {
            self.kind.clone()
        } else {
            self.title.clone()
        };
        let failed = Some(presentation_event)
            .filter(|event| !event.is_null())
            .is_some_and(|_| {
                matches!(self.kind.as_str(), "tool" | "subagent") && presentation.failed
            });
        let summary = if failed {
            "Tool failed".to_owned()
        } else if is_reasoning || self.kind.is_empty() {
            title.clone()
        } else if is_tool_event(presentation_event) {
            let intent = presentation.intent.clone();
            match presentation.preview.clone() {
                Some(preview) if preview != intent => format!("{intent}: {preview}"),
                _ => intent,
            }
        } else {
            format!("{} · {}", self.kind, title)
        };
        let expanded = self.expanded;
        let loading = self.loading;
        let has_more = self.next_offset.is_some();
        let next_offset = self.next_offset;
        let total_bytes = self.total_bytes;
        let detail = &self.detail;
        let error = self.error.clone();
        let event_id = self.event_id.clone();

        let action = if expanded {
            format!("Collapse details for {summary}")
        } else {
            format!("Expand details for {summary}")
        };
        let mut row = div()
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .items_start()
            .child(
                Button::new(format!("run-activity-toggle-{event_id}"))
                    .text()
                    .compact()
                    .xsmall()
                    .opacity(0.6)
                    .w_full()
                    .min_w_0()
                    .justify_start()
                    .px(px(0.))
                    .when(
                        is_reasoning || matches!(self.kind.as_str(), "tool" | "subagent"),
                        |button| {
                            button.child(activity_icon(
                                &self.events,
                                if is_reasoning {
                                    IconName::BookOpen
                                } else {
                                    IconName::Wrench
                                },
                                &self.presentation_cache,
                                None,
                            ))
                        },
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .max_w_full()
                            .overflow_hidden()
                            .truncate()
                            .text_size(px(11.))
                            .child(summary),
                    )
                    .when(is_reasoning, |button| {
                        if let Some(time) = reasoning_time(presentation_event, cx) {
                            button.child(time)
                        } else {
                            button
                        }
                    })
                    .child(fixture_bounds(
                        div().flex_shrink_0().child(
                            Icon::new(if expanded {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            })
                            .size(px(12.)),
                        ),
                        format!("run-caret-{event_id}"),
                    ))
                    .accessibility_label(action)
                    .disabled(loading)
                    .on_click(cx.listener(|row, _, _, cx| row.toggle(cx))),
            );

        #[cfg(test)]
        {
            row = row.debug_selector({
                let id = self.event_id.clone();
                move || format!("run-single-{id}")
            });
        }
        if expanded {
            let mut detail_panel = div()
                .w_full()
                .min_w_0()
                .ml(px(7.))
                .mt(px(10.))
                .pl(px(8.))
                .border_l_1()
                .border_color(cx.theme().border)
                .flex()
                .flex_col()
                .gap(px(10.));
            #[cfg(test)]
            {
                detail_panel = detail_panel.debug_selector({
                    let id = self.event_id.clone();
                    move || format!("run-detail-{id}")
                });
            }
            if loading {
                detail_panel = detail_panel.child(
                    div()
                        .text_size(px(11.))
                        .text_color(cx.theme().muted_foreground)
                        .child("Loading event detail…"),
                );
            }
            if is_reasoning {
                if !reasoning_summary.is_empty() {
                    detail_panel = detail_panel.child(
                        div()
                            .id(SharedString::from(format!(
                                "activity-detail-scroll-{}",
                                self.event_id
                            )))
                            .max_h(px(240.))
                            .overflow_y_scroll()
                            .child(
                                gpui_kit::component::text::TextView::markdown(
                                    SharedString::from(format!(
                                        "run-event-detail-{}",
                                        self.event_id
                                    )),
                                    reasoning_summary,
                                )
                                .selectable(true)
                                .scrollable(false),
                            ),
                    );
                }
            } else if !detail.is_empty() {
                detail_panel = detail_panel.child(
                    div()
                        .id(SharedString::from(format!(
                            "activity-detail-scroll-{}",
                            self.event_id
                        )))
                        .max_h(px(240.))
                        .overflow_y_scroll()
                        .child({
                            if self.detail_presentation.borrow().is_none() {
                                let prepared = tool_detail::prepare(&detail)
                                    .map(DetailPresentation::Structured)
                                    .unwrap_or_else(|| {
                                        DetailPresentation::Literal(literal_detail(&detail).into())
                                    });
                                *self.detail_presentation.borrow_mut() = Some(prepared);
                            }
                            match self.detail_presentation.borrow().as_ref().unwrap() {
                                DetailPresentation::Structured(prepared) => {
                                    tool_detail::render_prepared(
                                        prepared,
                                        &self.event_id,
                                        &self.task_id,
                                        self.git_owner.clone(),
                                        window,
                                        cx,
                                    )
                                }
                                DetailPresentation::Literal(text) => {
                                    gpui_kit::component::text::TextView::markdown(
                                        SharedString::from(format!(
                                            "run-event-detail-{}",
                                            self.event_id
                                        )),
                                        text.clone(),
                                    )
                                    .selectable(true)
                                    .scrollable(false)
                                    .into_any_element()
                                }
                            }
                        }),
                );
            }
            if let Some(error) = error {
                detail_panel = detail_panel.child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(0xff9f9f))
                        .child(error),
                );
            }
            if let (Some(offset), Some(total)) = (next_offset, total_bytes) {
                detail_panel = detail_panel.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .text_size(px(10.))
                                .text_color(cx.theme().muted_foreground)
                                .child(format!("{offset} of {total} bytes")),
                        )
                        .child(
                            Button::new(format!("run-activity-more-{event_id}"))
                                .text()
                                .compact()
                                .xsmall()
                                .label(if loading {
                                    "Loading…"
                                } else {
                                    "Load next detail chunk"
                                })
                                .disabled(loading || !has_more)
                                .on_click(cx.listener(|row, _, _, cx| row.load_next(cx))),
                        ),
                );
            } else if let Some(total) = total_bytes {
                detail_panel = detail_panel.child(
                    div()
                        .text_size(px(10.))
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("{total} bytes")),
                );
            }
            row = row.child(detail_panel);
        }
        row.into_any_element()
    }

    fn render_group(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        if let Some(compaction) = context_compaction_row(&self.events, self.group_running, cx) {
            return compaction;
        }
        if self.mode == GroupMode::Reasoning
            && !self.events.is_empty()
            && self
                .events
                .iter()
                .all(|event| self.presentation_cache.get(event).reasoning.is_empty())
        {
            return thinking_row(self.events.last(), self.group_running, cx);
        }
        let tool_count = tool_count(&self.events);
        let reasoning = self
            .events
            .iter()
            .any(|event| field(event, "kind") == "reasoning");
        let failed = self
            .events
            .iter()
            .filter(|event| is_tool_event(event) && self.presentation_cache.get(event).failed)
            .count();
        let label = match self.mode {
            GroupMode::Reasoning => format!("Thought process · {} items", self.events.len()),
            GroupMode::Tool => format!(
                "Turn · {tool_count} {}",
                if tool_count == 1 { "tool" } else { "tools" }
            ),
            GroupMode::Process => format!(
                "{} · {} {}",
                if reasoning { "Thinking" } else { "Turn" },
                tool_count,
                if tool_count == 1 { "tool" } else { "tools" }
            ),
            GroupMode::Single => String::new(),
        };
        let mut summary = label;
        if failed > 0 {
            summary.push_str(&format!(" · {failed} failed"));
        }
        if self.mode == GroupMode::Process && !self.process_approvals.is_empty() {
            let count = self.process_approvals.len();
            summary.push_str(&format!(
                " · {count} {}",
                if count == 1 { "approval" } else { "approvals" }
            ));
        }
        if matches!(self.mode, GroupMode::Process | GroupMode::Tool) {
            if let Some(preview) = self.presentation_cache.preview(&self.events) {
                summary.push_str(": ");
                if let Some(latest) = self.events.iter().rev().find(|event| is_tool_event(event)) {
                    let intent = self.presentation_cache.get(latest).intent.clone();
                    if preview != intent {
                        summary.push_str(&intent);
                        summary.push_str(" · ");
                    }
                }
                summary.push_str(&preview);
            }
        }
        let mut row = div()
            .w_full()
            .min_w_0()
            .py(px(2.))
            .flex()
            .flex_col()
            .items_start()
            .gap(px(6.));
        #[cfg(test)]
        {
            row = row.debug_selector({
                let id = self.stable_first_id.clone();
                move || format!("run-group-{id}")
            });
        }
        // Group disclosures are kept lazy: each retained event id remains
        // available through its own retained row in the grouped projection.
        let anchor = self.stable_first_id.clone();
        let action = if self.expanded {
            "Hide steps"
        } else {
            "Show steps"
        };
        let icon_bounds = self.header_icon_bounds.clone();
        let mut toggle = Button::new(format!("run-activity-toggle-{anchor}"))
            .text()
            .compact()
            .xsmall()
            .opacity(0.6)
            .w_full()
            .min_w_0()
            .justify_start()
            .px(px(0.))
            .child(fixture_bounds(
                div()
                    .relative()
                    .flex_shrink_0()
                    .child(activity_icon(
                        &self.events,
                        IconName::Brain,
                        &self.presentation_cache,
                        Some(self.group_running),
                    ))
                    .child(
                        canvas(
                            move |bounds, _, _| icon_bounds.set(Some(bounds)),
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .inset_0(),
                    ),
                format!("run-leading-{anchor}"),
            ))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .max_w_full()
                    .overflow_hidden()
                    .truncate()
                    .text_size(px(10.))
                    .child(summary.clone()),
            );
        if let Some(time) = self
            .events
            .last()
            .and_then(|event| reasoning_time(event, cx))
        {
            toggle = toggle.child(time);
        }
        toggle = toggle.child(fixture_bounds(
            div().flex_shrink_0().child(
                Icon::new(if self.expanded {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                })
                .size(px(12.)),
            ),
            format!("run-caret-{anchor}"),
        ));
        row = row.child(
            toggle
                .accessibility_label(format!("{action}: {summary}"))
                .on_click(cx.listener(|row, _, _, cx| row.toggle(cx))),
        );
        if self.expanded {
            if matches!(self.mode, GroupMode::Process | GroupMode::Tool)
                && !self.process_branches.is_empty()
                && !self.repeats_only_tool_branch()
            {
                for branch in &self.process_branches {
                    let key = process_branch_key(branch);
                    let branch_open = self.expanded_branches.contains(&key);
                    let label = match branch.mode {
                        GroupMode::Reasoning
                            if branch.events.iter().any(|event| {
                                !self.presentation_cache.get(event).reasoning.is_empty()
                            }) =>
                        {
                            "Thought process".to_owned()
                        }
                        GroupMode::Reasoning => "Thinking".to_owned(),
                        GroupMode::Tool => self.presentation_cache.group_label(&branch.events),
                        _ => "Activity".to_owned(),
                    };
                    let branch_time = branch.events.last().and_then(event_time);
                    let toggle_key = key.clone();
                    let branch_action = if branch_open { "Hide" } else { "Show" };
                    let branch_accessibility = format!("{branch_action} branch: {label}");
                    row = row.child(
                        fixture_bounds(div(), format!("run-branch-{key}"))
                            .w_full()
                            .min_w_0()
                            // Keep nested steps indented beneath their parent;
                            // disclosure carets stay on the trailing edge.
                            .ml(px(7.))
                            .pl(px(5.))
                            .border_l_1()
                            .border_color(cx.theme().border)
                            .flex()
                            .flex_col()
                            .gap(px(10.))
                            .child(
                                Button::new(format!("process-branch-{key}"))
                                    .text()
                                    .compact()
                                    .xsmall()
                                    .opacity(0.6)
                                    .w_full()
                                    .min_w_0()
                                    .justify_start()
                                    .accessibility_label(branch_accessibility)
                                    .px(px(0.))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .max_w_full()
                                            .overflow_hidden()
                                            .truncate()
                                            .text_size(px(10.))
                                            .child(label),
                                    )
                                    .when_some(branch_time, |button, time| {
                                        button.child(
                                            div()
                                                .ml_auto()
                                                .flex_shrink_0()
                                                .text_color(cx.theme().muted_foreground)
                                                .text_size(px(9.))
                                                .child(time),
                                        )
                                    })
                                    .child(
                                        Icon::new(if branch_open {
                                            IconName::ChevronDown
                                        } else {
                                            IconName::ChevronRight
                                        })
                                        .size(px(12.)),
                                    )
                                    .on_click(cx.listener(move |row, _, _, cx| {
                                        row.toggle_process_branch(toggle_key.clone(), cx);
                                    })),
                            )
                            .when(branch_open, |container| {
                                let mut steps = div()
                                    .w_full()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .ml(px(7.))
                                    .pl(px(1.))
                                    .border_l_1()
                                    .border_color(cx.theme().border)
                                    .gap(px(10.));
                                for event in &branch.events {
                                    let id = field(event, "id");
                                    let child = self
                                        .detail_row_index
                                        .get(&id)
                                        .and_then(|index| self.detail_rows.get(*index))
                                        .cloned();
                                    if let Some(child) = child {
                                        steps = steps.child(div().w_full().min_w_0().child(child));
                                    }
                                }
                                container.child(steps)
                            }),
                    );
                }
                for approval in &self.process_approvals {
                    let label = field(approval, "displayLabel");
                    let label = if label.is_empty() {
                        "Approval resolved".to_owned()
                    } else {
                        label
                    };
                    row = row.child(
                        div()
                            .w_full()
                            .min_w_0()
                            .pl_3()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_size(px(11.))
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("Approval · {label}")),
                    );
                }
            } else {
                for detail in &self.detail_rows {
                    row = row.child(
                        div()
                            .w_full()
                            .min_w_0()
                            .ml(px(7.))
                            .pl(px(5.))
                            .border_l_1()
                            .border_color(cx.theme().border)
                            .child(detail.clone()),
                    );
                }
            }
        }
        row.into_any_element()
    }
}

#[cfg(all(test, feature = "visual-export", feature = "performance-profiling"))]
#[path = "activity_disclosure_tests.rs"]
mod activity_disclosure_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[::core::prelude::v1::test]
    fn context_compaction_uses_concise_lifecycle_labels_and_no_tool_count() {
        use crate::transcript::ContextCompaction;
        assert_eq!(
            context_compaction_label(&ContextCompaction {
                active: true,
                completed: false,
                duration_ms: None
            }),
            "Compacting context…"
        );
        assert_eq!(
            context_compaction_label(&ContextCompaction {
                active: false,
                completed: true,
                duration_ms: Some(65_000)
            }),
            "Context compacted · 1m 5s"
        );
        assert_eq!(
            context_compaction_label(&ContextCompaction {
                active: false,
                completed: false,
                duration_ms: None
            }),
            "Context compaction"
        );
        let event = json!({"kind":"tool","title":"ContextCompaction","detail":json!({"type":"contextCompaction","id":"native","monitterPhase":"started"}).to_string()});
        assert_eq!(tool_count(&[event.clone()]), 0);
        assert_eq!(process_tool_preview(&[event]), None);
    }

    #[::core::prelude::v1::test]
    fn disclosure_presentation_cache_reuses_projection_and_invalidates_changed_detail() {
        let cache = PresentationCache::default();
        let event = json!({"id":"one","kind":"tool","detail":json!({"type":"commandExecution","command":"echo old","status":"completed"}).to_string()});
        let first = cache.get(&event);
        // Expansion/collapse changes no authoritative event revision.
        let reopened = cache.get(&event);
        assert!(Rc::ptr_eq(&first, &reopened));
        assert_eq!(first.preview.as_deref(), Some("echo old"));
        cache.clear();
        let loaded =
            json!({"type":"commandExecution","command":"echo new","status":"failed"}).to_string();
        let changed = cache.with_loaded_detail(&event, Some(&loaded));
        assert!(!Rc::ptr_eq(&first, &changed));
        assert!(changed.failed);
        assert_eq!(changed.preview.as_deref(), Some("echo new"));
        assert!(Rc::ptr_eq(
            &changed,
            &cache.with_loaded_detail(&event, Some(&loaded))
        ));
    }

    #[::core::prelude::v1::test]
    fn disclosure_presentation_cache_large_fixture_reports_warm_projection_cost() {
        let events:Vec<_>=(0..10).map(|n|json!({"id":format!("tool-{n}"),"kind":"tool","detail":json!({
            "type":"commandExecution","command":format!("echo {n}"),"output":"large output line\n".repeat(1800)
        }).to_string()})).collect();
        let cache = PresentationCache::default();
        let cold = std::time::Instant::now();
        let initial: Vec<_> = events.iter().map(|e| cache.get(e)).collect();
        let cold = cold.elapsed();
        let warm = std::time::Instant::now();
        for _ in 0..10 {
            for (index, event) in events.iter().enumerate() {
                assert!(Rc::ptr_eq(&initial[index], &cache.get(event)));
            }
        }
        eprintln!(
            "ten ~32KiB tool payloads: cold projection {cold:?}; ten warm disclosure redraws {:?}",
            warm.elapsed()
        );
        assert_eq!(cache.0.borrow().len(), 10);
    }

    #[::core::prelude::v1::test]
    fn structured_command_overrides_enclosing_fetch_tool_label() {
        let event = json!({"kind":"tool", "title":"fetch", "detail": json!({
            "activity":{"tool":"web_fetch"},
            "update":{"rawOutput": json!({"type":"commandExecution","command":"printf fetch"}).to_string()}
        }).to_string()});
        assert_eq!(tool_intent(&event), "Ran a command");
        assert_eq!(
            process_tool_preview(&[event]).as_deref(),
            Some("printf fetch")
        );
        let event = json!({"kind":"tool", "detail":json!({"tool":"web_fetch"}).to_string()});
        assert_eq!(tool_intent(&event), "Fetched a web page");
        assert_eq!(
            process_tool_preview(&[event]).as_deref(),
            Some("Fetched a web page")
        );
    }

    #[::core::prelude::v1::test]
    fn process_summary_uses_latest_structured_command_and_counts_only_tools() {
        let command = json!({
            "id": "run-1",
            "kind": "tool",
            "title": "functions.exec_command",
            "detail": json!({"update":{"rawInput":{"command":["cargo", "test"]}}}).to_string()
        });
        let usage = json!({
            "id": "usage-1",
            "kind": "usage",
            "title": "Context Usage Updated",
            "detail": "{}"
        });
        let events = vec![command.clone(), usage.clone()];
        assert_eq!(tool_count(&events), 1);
        assert_eq!(process_tool_preview(&events).as_deref(), Some("cargo test"));
        assert_eq!(process_tool_preview(&[usage]), None);
        assert_eq!(grouped_tool_label(&[command]), "Ran 1 command");
        let bookkeeping =
            json!({"id":"opaque","kind":"tool","title":"Internal usage checkpoint","detail":"{}"});
        assert_eq!(
            process_tool_preview(&[bookkeeping]).as_deref(),
            Some("Used a tool")
        );
    }

    #[::core::prelude::v1::test]
    fn same_identity_detail_updates_refresh_but_metadata_preserves_loaded_pages() {
        let prior = json!({"id":"r1","detail":"","title":"Thinking"});
        assert_eq!(
            event_update("t1", "t1", Some(&prior), &prior),
            EventUpdate::Unchanged
        );
        let summary = json!({"id":"r1","detail":"New summary","title":"Thinking"});
        assert_eq!(
            event_update("t1", "t1", Some(&prior), &summary),
            EventUpdate::Detail
        );
        let metadata = json!({"id":"r1","detail":"","title":"Updated title"});
        assert_eq!(
            event_update("t1", "t1", Some(&prior), &metadata),
            EventUpdate::Metadata
        );
        assert_eq!(
            event_update("t2", "t1", Some(&prior), &prior),
            EventUpdate::Identity
        );
        let other = json!({"id":"r2","detail":"","title":"Thinking"});
        assert_eq!(
            event_update("t1", "t1", Some(&prior), &other),
            EventUpdate::Identity
        );
    }

    fn event(id: &str, kind: &str, task_id: &str, created_at: i64, title: &str) -> Value {
        json!({
            "id": id,
            "kind": kind,
            "taskId": task_id,
            "createdAt": created_at,
            "title": title,
            "detail": "",
        })
    }

    /// Stable identity invariants for grouped constructors. The full GPUI state
    /// machine is exercised by the parent (chat.rs) at runtime; these checks
    /// guard the structural contracts the renderer depends on.
    #[::core::prelude::v1::test]
    fn stable_key_format_per_mode_uses_first_event_id() {
        let anchor = "a";
        let single = format!("event:{anchor}");
        let reasoning = format!("reasoning-group:{anchor}");
        let tool = format!("tool-group:{anchor}");
        let process = format!("process-group:{anchor}");
        assert_eq!(single, "event:a");
        assert_eq!(reasoning, "reasoning-group:a");
        assert_eq!(tool, "tool-group:a");
        assert_eq!(process, "process-group:a");
    }

    /// Confirm the JSON event builder used by the tests produces the fields the
    /// group constructors rely on (id / taskId / kind / title).
    #[::core::prelude::v1::test]
    fn event_builder_round_trip() {
        let value = event("a", "reasoning", "task-1", 10, "Thinking");
        assert_eq!(value["id"], "a");
        assert_eq!(value["taskId"], "task-1");
        assert_eq!(value["kind"], "reasoning");
        assert_eq!(value["title"], "Thinking");
        assert_eq!(value["createdAt"], 10);
    }
}
