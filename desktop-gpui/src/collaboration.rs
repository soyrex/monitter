//! Native view of real child-agent sessions for the selected task.
//! Snapshot records are the authority; transcript reads use the existing
//! scoped `get_subagent_transcript` service command.
use crate::design::ui_scaled_px as px;
use futures::channel::oneshot;
use gpui_kit::base::{TextSelectionHandle, TextSelectionRegistration, TextSelectionRun};
use gpui_kit::{
    Bounds, Element, ElementId, GlobalElementId, Hitbox, InspectorElementId, LayoutId, StyledText,
};
use gpui_kit::{
    assets::__private as icon_assets,
    component::{Icon, Sizable, button::*},
    prelude::*,
    *,
};
use monitter_lib::experimental_desktop::DesktopService;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::{rc::Rc, sync::Arc};

#[path = "completion_feedback.rs"]
pub(crate) mod completion_feedback;
use completion_feedback::CompletionMarker;

const TRANSCRIPT_WORKER_STACK: usize = 8 * 1024 * 1024;

/// Plain selectable text for the child output log. This deliberately avoids
/// Markdown interpretation so the transcript reads like terminal output.
struct SelectableLogLine {
    selection: TextSelectionHandle,
    text: SharedString,
    styled_text: StyledText,
    document_order: u64,
}

impl SelectableLogLine {
    fn new(selection: TextSelectionHandle, text: impl Into<SharedString>, order: u64) -> Self {
        let text = text.into();
        Self {
            selection,
            styled_text: StyledText::new(text.clone()),
            text,
            document_order: order,
        }
    }
}

impl IntoElement for SelectableLogLine {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for SelectableLogLine {
    type RequestLayoutState = ();
    type PrepaintState = Hitbox;

    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        self.styled_text
            .request_layout(id, inspector_id, window, cx)
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Hitbox {
        self.styled_text
            .prepaint(id, inspector_id, bounds, &mut (), window, cx);
        let hitbox = window.insert_hitbox(bounds, gpui::HitboxBehavior::Normal);
        self.selection.register(
            TextSelectionRegistration::new(hitbox.clone(), bounds)
                .with_document_order(self.document_order)
                .with_text_bounds(vec![bounds]),
            window,
            cx,
        );
        hitbox
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        _: &mut Hitbox,
        window: &mut Window,
        cx: &mut App,
    ) {
        let layout = self.styled_text.layout().clone();
        self.selection.update_runs(
            &[TextSelectionRun::new(self.text.clone(), layout, bounds)
                .with_document_order(self.document_order)],
            cx,
        );
        self.styled_text
            .paint(id, inspector_id, bounds, &mut (), &mut (), window, cx);
    }
}

fn output_log_line(entry: &Value) -> String {
    let role = entry["role"]
        .as_str()
        .unwrap_or("activity")
        .to_ascii_uppercase();
    let timestamp = entry["createdAt"]
        .as_i64()
        .map(format_time)
        .unwrap_or_default();
    let text = entry["text"].as_str().unwrap_or("");
    if timestamp.is_empty() {
        format!("{role}  {text}\n")
    } else {
        format!("{timestamp}  {role}  {text}\n")
    }
}

#[cfg(test)]
mod output_log_tests {
    use super::output_log_line;
    use serde_json::json;

    #[test]
    fn output_projection_keeps_markdown_as_literal_transcript_text() {
        let line = output_log_line(&json!({"role":"assistant", "text":"**done**\n`code`"}));
        assert_eq!(line, "ASSISTANT  **done**\n`code`\n");
    }

    #[test]
    fn output_projection_labels_non_message_activity_without_inventing_body() {
        let line = output_log_line(&json!({"role":"tool", "text":""}));
        assert_eq!(line, "TOOL  \n");
    }
}

fn field(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}
fn rows(v: &Value, key: &str) -> Vec<Value> {
    v.get(key)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}
fn rows_ref<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}
pub(crate) fn records(snapshot: &Value, parent: &str) -> Vec<Value> {
    let sessions = rows_ref(snapshot, "subagentSessions");
    let collaborations = rows_ref(snapshot, "collaborations");
    let collab_by_id: HashMap<String, &Value> = collaborations
        .iter()
        .filter(|item| field(item, "kind") == "delegation")
        .map(|item| (field(item, "id"), item))
        .collect();
    let mut result = Vec::new();
    let mut visited_parents = std::collections::HashSet::from([parent.to_owned()]);
    let mut represented = std::collections::HashSet::new();
    let mut visited_sessions = std::collections::HashSet::new();
    let mut pending = vec![parent.to_owned()];

    while let Some(parent_id) = pending.pop() {
        for session in sessions
            .iter()
            .filter(|session| field(session, "parentTaskId") == parent_id)
        {
            let session_id = field(session, "id");
            if !visited_sessions.insert(session_id.clone()) {
                continue;
            }
            let collaboration_id = field(session, "collaborationId");
            if !collaboration_id.is_empty() {
                represented.insert(collaboration_id.clone());
            }
            let collaboration = collab_by_id.get(&collaboration_id);
            let mut item = session.clone();
            if item["prompt"].is_null() {
                if let Some(value) = collaboration {
                    item["prompt"] = value["text"].clone();
                }
            }
            if item["result"].is_null() {
                if let Some(value) = collaboration {
                    item["result"] = value["result"].clone();
                }
            }
            if item["error"].is_null() {
                if let Some(value) = collaboration {
                    item["error"] = value["error"].clone();
                }
            }
            item["taskId"] = collaboration
                .map(|value| value["toTaskId"].clone())
                .unwrap_or(Value::Null);
            item["agentId"] = collaboration
                .map(|value| value["toAgentId"].clone())
                .unwrap_or(Value::Null);
            if let Some(child_task) = item["taskId"].as_str().filter(|value| !value.is_empty()) {
                if visited_parents.insert(child_task.to_owned()) {
                    pending.push(child_task.to_owned());
                }
            }
            result.push(item);
        }
        for collaboration in collaborations {
            if field(collaboration, "kind") != "delegation"
                || field(collaboration, "fromTaskId") != parent_id
                || represented.contains(&field(collaboration, "id"))
            {
                continue;
            }
            let child_task = field(collaboration, "toTaskId");
            if !child_task.is_empty() && visited_parents.insert(child_task.clone()) {
                pending.push(child_task.clone());
            }
            represented.insert(field(collaboration, "id"));
            result.push(json!({
                "id":format!("collaboration:{}",field(collaboration,"id")),
                "source":"collaboration", "parentTaskId":parent_id,
                "collaborationId":collaboration["id"], "prompt":collaboration["text"],
                "result":collaboration["result"], "error":collaboration["error"],
                "status":collaboration["status"], "createdAt":collaboration["createdAt"],
                "updatedAt":collaboration["updatedAt"], "taskId":collaboration["toTaskId"],
                "agentId":collaboration["toAgentId"]
            }));
        }
    }

    let tasks = rows_ref(snapshot, "tasks");
    let agents = rows_ref(snapshot, "agents");
    let projects = rows_ref(snapshot, "projects");
    let events = rows_ref(snapshot, "events");
    let messages = rows_ref(snapshot, "messages");
    for item in &mut result {
        let task = tasks
            .iter()
            .find(|task| field(task, "id") == field(item, "taskId"));
        let collaboration = collab_by_id.get(&field(item, "collaborationId"));
        let agent_id = task
            .map(|task| field(task, "agentId"))
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| field(item, "agentId"));
        let agent = agents.iter().find(|agent| field(agent, "id") == agent_id);
        let agent_name = agent
            .and_then(|agent| {
                agent["name"]
                    .as_str()
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned)
            })
            .or_else(|| item["agentPath"].as_str().map(human_name))
            .unwrap_or_else(|| "Subagent".into());
        item["agentName"] = Value::String(agent_name.clone());
        let initials = agent_name
            .split_whitespace()
            .filter_map(|part| part.chars().next())
            .take(2)
            .flat_map(char::to_uppercase)
            .collect::<String>();
        item["agentInitials"] = Value::String(if initials.is_empty() {
            "S".into()
        } else {
            initials
        });
        item["title"] = task
            .map(|task| task["title"].clone())
            .filter(|value| value.as_str().is_some_and(|title| !title.is_empty()))
            .unwrap_or_else(|| {
                item["prompt"]
                    .as_str()
                    .map(|prompt| {
                        Value::String(
                            prompt
                                .lines()
                                .next()
                                .unwrap_or(prompt)
                                .chars()
                                .take(70)
                                .collect(),
                        )
                    })
                    .unwrap_or_else(|| Value::String(agent_name.clone()))
            });

        let thread_id = field(item, "agentThreadId");
        let native_event = events
            .iter()
            .filter(|event| {
                if field(event, "taskId") != field(item, "parentTaskId")
                    || field(event, "kind") != "subagent"
                    || thread_id.is_empty()
                {
                    return false;
                }
                let detail: Value =
                    serde_json::from_str(&field(event, "detail")).unwrap_or(Value::Null);
                let activity = detail.get("activity").unwrap_or(&detail);
                let primary = activity
                    .get("agentThreadId")
                    .and_then(Value::as_str)
                    .or_else(|| activity.get("agent_thread_id").and_then(Value::as_str));
                let receivers = activity
                    .get("receiverThreadIds")
                    .or_else(|| activity.get("receiver_thread_ids"))
                    .and_then(Value::as_array);
                primary == Some(thread_id.as_str())
                    || receivers.is_some_and(|ids| {
                        ids.iter()
                            .any(|value| value.as_str() == Some(thread_id.as_str()))
                    })
            })
            .max_by_key(|event| event["createdAt"].as_i64().unwrap_or_default());
        let task_event = task.and_then(|task| {
            events
                .iter()
                .filter(|event| {
                    field(event, "taskId") == field(task, "id")
                        && matches!(field(event, "kind").as_str(), "tool" | "reasoning")
                })
                .max_by_key(|event| event["createdAt"].as_i64().unwrap_or_default())
        });
        let latest_event = native_event.or(task_event);
        item["activity"] = latest_event
            .and_then(|event| {
                event["title"]
                    .as_str()
                    .filter(|value| !value.trim().is_empty())
                    .map(|value| Value::String(value.to_owned()))
            })
            .unwrap_or(Value::Null);

        let reported_model = task.and_then(|task| {
            messages
                .iter()
                .filter(|message| {
                    field(message, "taskId") == field(task, "id")
                        && field(message, "role") == "assistant"
                })
                .filter_map(|message| {
                    message["responseMetadata"]["model"]
                        .as_str()
                        .filter(|model| !model.trim().is_empty())
                        .map(|model| {
                            (
                                message["createdAt"].as_i64().unwrap_or_default(),
                                model.to_owned(),
                            )
                        })
                })
                .max_by_key(|(created_at, _)| *created_at)
                .map(|(_, model)| model)
        });
        let configured_model = field(item, "model");
        let task_model = task.map(|task| field(task, "model")).unwrap_or_default();
        let agent_model = agent.map(|agent| field(agent, "model")).unwrap_or_default();
        let model = [
            reported_model.unwrap_or_default(),
            configured_model,
            task_model,
            agent_model,
        ]
        .into_iter()
        .find(|value| {
            !value.trim().is_empty()
                && !matches!(value.to_lowercase().as_str(), "default" | "harness default")
        });
        item["model"] = model.map(Value::String).unwrap_or(Value::Null);
        if item["reasoningEffort"].is_null() {
            item["reasoningEffort"] = task
                .map(|task| task["modelSettings"]["reasoningEffort"].clone())
                .unwrap_or(Value::Null);
        }

        let project_id = task
            .map(|task| field(task, "projectId"))
            .filter(|value| !value.is_empty())
            .or_else(|| {
                tasks
                    .iter()
                    .find(|task| field(task, "id") == parent)
                    .map(|task| field(task, "projectId"))
            });
        item["projectName"] = project_id
            .and_then(|project_id| {
                projects
                    .iter()
                    .find(|project| field(project, "id") == project_id)
                    .map(|project| project["name"].clone())
            })
            .unwrap_or(Value::Null);
        item["launchedAt"] = item["createdAt"]
            .as_i64()
            .map(Value::from)
            .or_else(|| task.map(|task| task["createdAt"].clone()))
            .unwrap_or(Value::from(0));
        let update_time = [
            item["updatedAt"].as_i64(),
            task.and_then(|task| task["updatedAt"].as_i64()),
            collaboration.and_then(|value| value["updatedAt"].as_i64()),
            latest_event.and_then(|event| event["createdAt"].as_i64()),
        ]
        .into_iter()
        .flatten()
        .max()
        .unwrap_or_default();
        item["updatedAt"] = Value::from(update_time);
    }
    result.sort_by(|a, b| {
        let active = |item: &Value| matches!(field(item, "status").as_str(), "queued" | "running");
        active(b)
            .cmp(&active(a))
            .then_with(|| b["updatedAt"].as_i64().cmp(&a["updatedAt"].as_i64()))
            .then_with(|| field(a, "title").cmp(&field(b, "title")))
    });
    result
}

pub(crate) fn prepare_child_transcripts(
    snapshot: &Value, items: &[Value],
    previous: Option<&monitter_gpui_desktop::native_publication::PreparedChildTranscripts>,
) -> monitter_gpui_desktop::native_publication::PreparedChildTranscripts {
    let mut transcripts = HashMap::new();
    for item in items {
        let id = field(item, "id");
        let entries = if let Some(task_id) = item["taskId"].as_str() {
            let mut entries: Vec<Value> = rows_ref(snapshot,"messages").iter()
                .filter(|m| field(m,"taskId") == task_id && matches!(field(m,"role").as_str(),"user"|"assistant"))
                .map(|m| json!({"id":m["id"],"role":m["role"],"text":m["text"],"createdAt":m["createdAt"]})).collect();
            entries.extend(rows_ref(snapshot, "events").iter().filter_map(|event| {
                if field(&event, "taskId") != task_id { return None; }
                match field(&event, "kind").as_str() {
                    "reasoning" => {
                        let summary = crate::transcript::reasoning_summary(event);
                        (!summary.is_empty()).then(|| json!({"id":event["id"],"role":"reasoning","text":summary,"createdAt":event["createdAt"]}))
                    }
                    "tool" => {
                        let title = event["title"].as_str().filter(|value| !value.is_empty()).unwrap_or("Tool activity");
                        Some(json!({"id":event["id"],"role":"activity","text":title,"createdAt":event["createdAt"]}))
                    }
                    _ => None,
                }
            }));
            entries.sort_by(|a, b| a["createdAt"].as_i64().cmp(&b["createdAt"].as_i64()));
            entries
        } else if let Some(entries) = snapshot["subagentTranscripts"].get(&id).and_then(Value::as_array) {
            entries.clone()
        } else { continue; };
        let retained = previous.and_then(|old| old.get(&id)).filter(|old| old.as_ref() == &entries);
        transcripts.insert(id, retained.cloned().unwrap_or_else(|| Arc::new(entries)));
    }
    Arc::new(transcripts)
}

/// Layout animation shared by the real visor and its native regression fixture.
pub(crate) fn visor_slide<T: 'static>(
    key: SharedString,
    open: bool,
    expanded_height: f32,
    motion_allowed: bool,
    window: &mut Window,
    cx: &mut Context<T>,
) -> f32 {
    // Seed a newly arriving auto-open visor from the collapsed position too.
    // Otherwise keyed transitions initialize at their first target and blink on.
    let seeded =
        window.use_keyed_state(SharedString::from(format!("{key}-seed")), cx, |_, _| false);
    if !*seeded.read(cx) {
        gpui_kit::base::motion::transition(
            key.clone(),
            0.0_f32,
            gpui_kit::base::motion::Transition::new(std::time::Duration::ZERO),
            window,
            cx,
        );
        seeded.update(cx, |seeded, _| *seeded = true);
    }
    let motion = gpui_kit::base::motion::transition_with_status(
        key,
        if open { expanded_height } else { 0.0 },
        gpui_kit::base::motion::Transition::new(std::time::Duration::from_millis(
            if motion_allowed { 250 } else { 0 },
        )),
        window,
        cx,
    );
    if matches!(
        motion.status,
        gpui_kit::base::motion::MotionStatus::Running
            | gpui_kit::base::motion::MotionStatus::Delayed
    ) {
        // Notify the observed child while moving so NativePanel's content
        // cache invalidates. Stop requesting work when the slide settles.
        let owner = cx.weak_entity();
        window.on_next_frame(move |_, cx| {
            let _ = owner.update(cx, |_, cx| cx.notify());
        });
    }
    motion.value.max(0.0)
}

pub(crate) fn clipped_visor_body(height: f32, body: impl IntoElement) -> AnyElement {
    div()
        .w_full()
        .relative()
        .h(px(height))
        .flex_shrink_0()
        .overflow_hidden()
        .child(body.into_any_element())
        .into_any_element()
}

pub struct SubagentPane {
    service: DesktopService,
    snapshot: Rc<Value>,
    parent_task: Option<String>,
    items: Arc<Vec<Value>>,
    selected: Option<String>,
    transcript: Arc<Vec<Value>>,
    prepared_transcripts: monitter_gpui_desktop::native_publication::PreparedChildTranscripts,
    transcript_list: ListState,
    generation: u64,
    transcript_loading: bool,
    native_read_in_flight: bool,
    transcript_load_key: Option<String>,
    error: Option<String>,
    visor_open: bool,
    compact_header: bool,
    composed_palette: Option<crate::design::Palette>,
    completion_markers: HashMap<(String, bool), Entity<CompletionMarker>>,
    sidebar_visible: bool,
    visor_visible: bool,
    suppress_completion: bool,
}

impl SubagentPane {
    pub fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        self.set_surface_visibility(visible, false, cx);
    }
    pub(crate) fn set_surface_visibility(&mut self, sidebar: bool, visor: bool, _cx: &mut Context<Self>) {
        if (self.sidebar_visible, self.visor_visible) == (sidebar, visor) { return; }
        self.sidebar_visible = sidebar;
        self.visor_visible = visor;
        for ((_, is_visor), marker) in &self.completion_markers {
            marker.update(_cx, |marker, cx| marker.set_visible(if *is_visor { visor } else { sidebar }, cx));
        }
    }
    /// Full snapshots and reconnects seed final status rather than replaying events.
    pub(crate) fn suppress_completion_motion(&mut self, cx: &mut Context<Self>) {
        self.suppress_completion = true;
        for marker in self.completion_markers.values() { marker.update(cx, |marker, cx| marker.suppress(cx)); }
    }
    fn marker(&mut self, id: &str, status: &str, visor: bool, palette: crate::design::Palette, size: f32, cx: &mut Context<Self>) -> Entity<CompletionMarker> {
        let visible = if visor { self.visor_visible } else { self.sidebar_visible };
        let marker = self.completion_markers.entry((id.to_owned(), visor))
            .or_insert_with(|| cx.new(|_| CompletionMarker::new(id, status)));
        let (icon, color) = status_marker(status, palette);
        marker.update(cx, |marker, cx| {
            marker.set_visible(visible, cx);
            marker.set_appearance(icon, color, size, cx);
        });
        marker.clone()
    }
    /// Count only authoritative records for this pane's current parent task.
    /// The badge performs no transcript read or backend request.
    pub fn record_count(&self) -> usize {
        self.items.len()
    }

    /// The parent chat uses this to decide whether to mount the child visor below its composer.
    pub fn visor_open(&self) -> bool {
        self.visor_open && !self.items.is_empty()
    }

    pub fn new(
        service: DesktopService,
        snapshot: Rc<Value>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Self {
        Self {
            service,
            snapshot,
            parent_task: None,
            items: Arc::new(vec![]),
            selected: None,
            transcript: Arc::new(vec![]),
            prepared_transcripts: Arc::new(HashMap::new()),
            transcript_list: ListState::new(0, ListAlignment::Bottom, px(220.)),
            generation: 0,
            transcript_loading: false,
            native_read_in_flight: false,
            transcript_load_key: None,
            error: None,
            visor_open: false,
            compact_header: false,
            composed_palette: None,
            completion_markers: HashMap::new(),
            sidebar_visible: false,
            visor_visible: false,
            suppress_completion: true,
        }
    }

    /// Hide the page title when this retained pane is embedded under a shell
    /// rail header. The pane keeps its normal standalone presentation by default.
    pub fn set_compact_header(&mut self, compact: bool, cx: &mut Context<Self>) {
        if self.compact_header != compact {
            self.compact_header = compact;
            cx.notify();
        }
    }

    /// Use the owning shell's composed appearance without losing standalone
    /// snapshot-based rendering. Repeated shell renders must not notify again.
    pub fn set_composed_palette(
        &mut self,
        palette: crate::design::Palette,
        cx: &mut Context<Self>,
    ) {
        let unchanged = self.composed_palette.is_some_and(|current| {
            current.dark == palette.dark
                && current.paper == palette.paper
                && current.sidebar == palette.sidebar
                && current.panel == palette.panel
                && current.card == palette.card
                && current.ink == palette.ink
                && current.muted == palette.muted
                && current.line == palette.line
                && current.soft == palette.soft
                && current.code == palette.code
                && current.accent == palette.accent
        });
        if !unchanged {
            self.composed_palette = Some(palette);
            cx.notify();
        }
    }

    /// User-driven context switches use the legacy immediate preparation;
    /// routine state publication supplies records prepared on its worker.
    pub fn set_context(&mut self, snapshot: Rc<Value>, task_id: &str, cx: &mut Context<Self>) {
        let items = Arc::new(records(&snapshot, task_id));
        let transcripts = prepare_child_transcripts(&snapshot, &items, Some(&self.prepared_transcripts));
        self.set_prepared_context(snapshot, task_id, items, transcripts, cx);
    }
    /// Publication dependency comparison proved this parent's prepared child
    /// records/transcripts unchanged. Release the older global source without
    /// rebuilding records, fencing native reads, or notifying the visor.
    pub(crate) fn advance_unaffected_snapshot(&mut self, snapshot: Rc<Value>) {
        self.snapshot = snapshot;
    }
    pub fn set_prepared_context(
        &mut self, snapshot: Rc<Value>, task_id: &str, items: Arc<Vec<Value>>,
        transcripts: monitter_gpui_desktop::native_publication::PreparedChildTranscripts, cx: &mut Context<Self>,
    ) {
        let changed = self.parent_task.as_deref() != Some(task_id);
        self.snapshot = snapshot;
        self.parent_task = Some(task_id.to_owned());
        if changed { self.completion_markers.clear(); }
        let live = !changed && !self.suppress_completion;
        self.completion_markers.retain(|(id, _), _| items.iter().any(|item| field(item, "id") == *id));
        for ((id, visor), marker) in &self.completion_markers {
            let Some(item) = items.iter().find(|item| field(item, "id") == *id) else { continue; };
            let visible = if *visor { self.visor_visible } else { self.sidebar_visible };
            marker.update(cx, |marker, cx| marker.set_status(id, &field(item, "status"), live && visible, cx));
        }
        self.suppress_completion = false;
        self.items = items;
        self.prepared_transcripts = transcripts;
        if changed
            || (self.selected.is_none() && !self.items.is_empty())
            || self
                .selected
                .as_ref()
                .is_some_and(|id| !self.items.iter().any(|v| field(v, "id") == *id))
        {
            self.selected = self.items.first().map(|v| field(v, "id"));
            self.visor_open = should_open_initial_visor(&self.items);
            self.transcript_loading = false;
            self.transcript_load_key = None;
            self.replace_transcript(Arc::new(vec![]));
            self.generation += 1;
            self.load_transcript(cx);
        } else if self
            .items
            .iter()
            .find(|item| self.selected.as_ref() == Some(&field(item, "id")))
            .is_some_and(|item| {
                !item["taskId"].is_null() || (field(item, "source") == "codex" && self.visor_open)
            })
        {
            // Delegation transcripts live in the ordinary snapshot and follow
            // its incremental refresh cadence. Native Codex threads are read
            // only when their selected record changes, avoiding duplicate reads.
            self.load_transcript(cx);
        }
        cx.notify();
    }

    fn select(&mut self, id: String, cx: &mut Context<Self>) {
        if self.selected.as_deref() == Some(&id) {
            if !self.visor_open {
                self.visor_open = true;
                self.load_transcript(cx);
                cx.notify();
            }
            return;
        }
        self.selected = Some(id);
        self.visor_open = true;
        self.transcript_loading = false;
        self.transcript_load_key = None;
        self.replace_transcript(Arc::new(vec![]));
        self.generation += 1;
        self.load_transcript(cx);
        cx.notify();
    }

    fn replace_transcript(&mut self, entries: Arc<Vec<Value>>) {
        if Arc::ptr_eq(&self.transcript, &entries) {
            return;
        }
        self.transcript_list.reset(entries.len());
        self.transcript = entries;
    }

    fn load_transcript(&mut self, cx: &mut Context<Self>) {
        self.error = None;
        let Some(id) = self.selected.clone() else {
            return;
        };
        let Some(root_parent) = self.parent_task.clone() else {
            return;
        };
        let item = self.items.iter().find(|v| field(v, "id") == id).cloned();
        let Some(item) = item else {
            return;
        };
        let parent = field(&item, "parentTaskId");
        let parent = if parent.is_empty() {
            root_parent
        } else {
            parent
        };
        // Task-backed delegations already have transcript messages/events in
        // the snapshot. Codex native subagent threads are queried by the
        // existing backend command; ACP inline transcripts are snapshot data.
        if let Some(entries) = self.prepared_transcripts.get(&id).cloned() {
            self.transcript_loading = false;
            self.transcript_load_key = None;
            self.replace_transcript(entries);
            return;
        }
        if field(&item, "source") != "codex" {
            self.transcript_loading = false;
            return;
        }
        if !self.visor_open {
            return;
        }
        let load_key = format!(
            "{parent}:{id}:{}:{}",
            item["updatedAt"].as_i64().unwrap_or_default(),
            field(&item, "status")
        );
        if self.native_read_in_flight {
            self.transcript_loading = true;
            return;
        }
        if self.transcript_load_key.as_deref() == Some(load_key.as_str()) {
            return;
        }
        self.transcript_load_key = Some(load_key);
        let service = self.service.clone();
        let generation = self.generation;
        let subagent_id = id;
        let (sender, receiver) = oneshot::channel();
        let worker_task_id = parent.clone();
        let worker_subagent_id = subagent_id.clone();
        let spawned = std::thread::Builder::new()
            .name("monitter-subagent-transcript".into())
            .stack_size(TRANSCRIPT_WORKER_STACK)
            .spawn(move || {
                let result = service.invoke(
                    "get_subagent_transcript",
                    json!({"taskId":worker_task_id,"subagentId":worker_subagent_id}),
                );
                let _ = sender.send(result.map(|entries| Arc::new(entries.as_array().cloned().unwrap_or_default())));
            });
        if let Err(error) = spawned {
            self.transcript_loading = false;
            self.transcript_load_key = None;
            self.error = Some(format!("Could not start transcript reader: {error}"));
            return;
        }
        self.transcript_loading = true;
        self.native_read_in_flight = true;
        self.error = None;
        cx.spawn(async move |pane, cx| {
            let result = receiver
                .await
                .unwrap_or_else(|_| Err("Transcript reader stopped before replying.".into()));
            let _ = pane.update(cx, |pane, cx| {
                pane.native_read_in_flight = false;
                pane.transcript_loading = false;
                let snapshot_backed = pane.selected.as_ref().is_some_and(|selected| {
                    pane.items.iter().any(|item| {
                        field(item, "id") == *selected && item["taskId"].as_str().is_some()
                    }) || pane.snapshot["subagentTranscripts"]
                        .get(selected)
                        .is_some_and(Value::is_array)
                });
                if pane.generation != generation || snapshot_backed {
                    // Rapid child changes queue only the newest selection;
                    // they never launch overlapping Codex lookup processes.
                    // A child promoted to snapshot authority also wins over
                    // an older native read of the same selection.
                    pane.load_transcript(cx);
                    cx.notify();
                    return;
                }
                match result {
                    Ok(entries) => {
                        pane.replace_transcript(entries)
                    }
                    Err(error) => {
                        pane.transcript_load_key = None;
                        pane.error = Some(error);
                    }
                }
                // A newer child record may have arrived while this bounded
                // read was in flight. The load key prevents duplicate reads.
                if pane.error.is_none() && pane.visor_open {
                    pane.load_transcript(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }
}

impl SubagentPane {
    /// Build the selected-child visor separately from the sidebar. The parent chat
    /// should place this element immediately below its composer.
    pub fn render_visor(
        &mut self,
        palette: crate::design::Palette,
        pane_height: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.items.is_empty() {
            return div().into_any_element();
        }
        let selected = self
            .selected
            .clone()
            .or_else(|| self.items.first().map(|item| field(item, "id")));
        let entries = self.transcript.clone();
        let task_id = self.parent_task.clone().unwrap_or_default();
        let tab_scroll = window.use_keyed_state(format!("subagent-visor-overflow-{task_id}"), cx,
            |_, _| ScrollHandle::new()).read(cx).clone();
        let mut tabs = div()
            .id(SharedString::from(format!("subagent-visor-tabs-{task_id}")))
            .role(Role::TabList)
            .aria_label("Subagent tasks")
            .flex()
            .items_center()
            .gap(px(5.))
            .min_w_0()
            .overflow_x_scroll()
            .track_scroll(&tab_scroll)
            .px(px(10.))
            .py(px(5.))
            .bg(crate::surface_blend::background(crate::surface_blend::Area::Pane, palette.paper));
        for (index, item) in self.items.clone().iter().cloned().enumerate() {
            let id = field(&item, "id");
            let active_tab = selected.as_deref() == Some(id.as_str());
            let title = item["title"]
                .as_str()
                .unwrap_or("Untitled subagent")
                .to_owned();
            let initials = item["agentInitials"].as_str().unwrap_or("S").to_owned();
            let selected_id = id.clone();
            let status = field(&item, "status");
            let (_, status_color) = status_marker(&status, palette);
            let status_marker = self.marker(&id, &status, true, palette, 9., cx);
            let mut tab = div()
                .id(SharedString::from(format!(
                    "subagent-visor-tab-{task_id}-{index}"
                )))
                .role(Role::Tab)
                .aria_label(format!(
                    "{title}, {}",
                    status_label(&field(&item, "status"))
                ))
                .aria_selected(active_tab)
                .flex_none()
                .max_w(px(260.))
                .flex()
                .items_center()
                .gap(px(6.))
                .px(px(8.))
                .py(px(5.))
                .rounded_md()
                .border_1()
                .border_color(rgb(if active_tab {
                    palette.accent
                } else {
                    palette.paper
                }))
                .bg(if active_tab {
                    crate::surface_blend::background(crate::surface_blend::Area::Pane, palette.soft)
                } else {
                    rgba(0x00000000).into()
                })
                .cursor_pointer()
                .child(
                    div()
                        .text_size(px(9.))
                        .font_family(crate::design::FONT_MONO)
                        .text_color(rgb(palette.accent))
                        .child(initials),
                )
                .child(div().min_w_0().truncate().text_size(px(11.)).child(title))
                .child(
                    div()
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .text_size(px(9.))
                        .text_color(rgb(status_color))
                        .child(status_marker)
                        .child(status_label(&status)),
                )
                .on_click(cx.listener(move |pane, _, _, cx| pane.select(selected_id.clone(), cx)));
            if self.visor_open && active_tab {
                tab = tab.border_color(rgb(palette.accent));
            }
            tabs = tabs.child(tab);
        }
        let mut tab_bar = div()
            .w_full()
            .min_w_0()
            .flex()
            .items_center()
            .border_t_1()
            .border_color(rgb(palette.line))
            .border_blend_mode(crate::surface_blend::current(crate::surface_blend::Area::Separators).shader())
            .child(ScrollEdgeFade::new(tabs.flex_1(), tab_scroll, Axis::Horizontal, gpui_kit::px(20.)));
        if self.visor_open {
            tab_bar = tab_bar.child(
                Button::new("subagent-visor-close")
                    .flex_shrink_0()
                    .mr(px(8.))
                    .compact()
                    .text()
                    .accessibility_label("Close subagent activity")
                    .icon(Icon::default().data(icon_assets::X.1).with_size(px(14.)))
                    .on_click(cx.listener(|pane, _, _, cx| {
                        pane.visor_open = false;
                        cx.notify();
                    })),
            );
        }
        let Some(item) = self
            .items
            .iter()
            .find(|item| selected.as_deref() == Some(field(item, "id").as_str()))
            .cloned()
        else {
            return tab_bar.into_any_element();
        };
        let expanded_height = if pane_height > 0.0 {
            (pane_height * 0.42).min(280.0)
        } else {
            240.0
        };
        let slide_height = visor_slide(
            SharedString::from(format!(
                "subagent-visor-slide-{task_id}-{:?}",
                cx.entity_id()
            )),
            self.visor_open,
            expanded_height,
            crate::native_motion::motion_allowed(),
            window,
            cx,
        );
        let mut visor = div()
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .border_t_1()
            .border_color(rgb(palette.line))
            .child(tab_bar);
        if slide_height > 0.1 {
            let agent = item["agentName"].as_str().unwrap_or("Subagent").to_owned();
            let status = status_label(&field(&item, "status"));
            let model = item["model"]
                .as_str()
                .filter(|value| !value.is_empty())
                .unwrap_or("Model not reported");
            let project = item["projectName"].as_str().unwrap_or("No project");
            let launched = item["launchedAt"]
                .as_i64()
                .filter(|value| *value > 0)
                .map(format_time)
                .unwrap_or_else(|| "Launch time unavailable".into());
            let meta = format!("{project} · {model} · Launched {launched}");
            let output_surface = if palette.dark {
                let channel = |shift: u32| ((palette.paper >> shift) & 0xff) * 3 / 5;
                (channel(16) << 16) | (channel(8) << 8) | channel(0)
            } else {
                palette.code
            };
            let mut body = div()
                .id(SharedString::from(format!(
                    "subagent-visor-panel-{task_id}"
                )))
                .role(Role::TabPanel)
                .flex()
                .flex_col()
                .gap(px(7.))
                .h(px(expanded_height))
                .min_h_0()
                .max_h(px(expanded_height))
                .overflow_y_scroll()
                .p(px(10.))
                .bg(crate::design::surface_color(output_surface))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(8.))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(7.))
                                .child(div().font_weight(FontWeight::SEMIBOLD).child(agent))
                                .child(status_chip(&status, palette)),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_size(px(10.))
                                .text_color(rgb(palette.muted))
                                .child(meta),
                        ),
                );
            if self.transcript.is_empty() {
                body = body.child(
                    div()
                        .p(px(8.))
                        .font_family(crate::design::FONT_MONO)
                        .text_size(px(9.))
                        .text_color(rgb(palette.muted))
                        .child("Waiting for child output…"),
                );
            } else {
                let messages = entries.clone();
                body = body.child(crate::transcript_edge_fade::TranscriptEdgeFade::for_list(
                    list(self.transcript_list.clone(), move |ix, window, cx| {
                        let entry = &messages[ix];
                        let text = output_log_line(entry);
                        let selection = window.use_keyed_state(
                            SharedString::from(format!(
                                "subagent-output-selection-{}",
                                field(entry, "id")
                            )),
                            cx,
                            |_, cx| TextSelectionHandle::new("", cx),
                        );
                        selection
                            .read(cx)
                            .clone()
                            .set_fallback_copy_text(text.clone(), cx);
                        div()
                            .w_full()
                            .min_w_0()
                            .py(px(3.))
                            .font_family(crate::design::FONT_MONO)
                            .text_size(px(9.))
                            .text_color(rgb(palette.ink))
                            .child(SelectableLogLine::new(
                                selection.read(cx).clone(),
                                text,
                                ix as u64,
                            ))
                            .into_any_element()
                    })
                    .h(px((expanded_height - 75.0).max(20.0)))
                    .max_h(px((expanded_height - 75.0).max(20.0))),
                    self.transcript_list.clone(),
                    gpui_kit::px(20.),
                ));
            }
            if self.transcript_loading {
                body = body.child(
                    div()
                        .text_size(px(10.))
                        .text_color(rgb(palette.muted))
                        .child("Updating transcript…"),
                );
            }
            if let Some(error) = &self.error {
                body = body.child(
                    div()
                        .text_size(px(10.))
                        .text_color(rgb(0xdc2626))
                        .child(format!("Transcript unavailable: {error}")),
                );
            }
            body = body.child(
                div()
                    .flex()
                    .justify_between()
                    .text_size(px(9.))
                    .font_family(crate::design::FONT_MONO)
                    .text_color(rgb(palette.muted))
                    .child({
                        let model = item["model"].as_str().unwrap_or("");
                        let effort = item["reasoningEffort"].as_str().unwrap_or("");
                        if effort.is_empty() {
                            model.to_owned()
                        } else {
                            format!("{model} · {effort}")
                        }
                    })
                    .child(format!(
                        "{} {}",
                        entries.len(),
                        if entries.len() == 1 {
                            "entry"
                        } else {
                            "entries"
                        }
                    )),
            );
            visor = visor.child(clipped_visor_body(
                slide_height,
                body.absolute().bottom_0().left_0().w_full(),
            ));
        }
        visor.into_any_element()
    }
}

/// Shared status projection for sidebar rows and the horizontal child tabs.
/// These are session states, independent of which child is currently selected.
fn status_marker(status: &str, palette: crate::design::Palette) -> (&'static str, u32) {
    match status {
        "completed" => ("✓", 0x6f9278),
        "running" => ("◌", palette.accent),
        "queued" => ("◷", palette.accent),
        "error" => ("!", 0xdc6262),
        "interrupted" => ("■", palette.muted),
        "idle" => ("◷", palette.muted),
        _ => ("?", palette.muted),
    }
}

fn status_label(status: &str) -> String {
    match status {
        "queued" => "Starting",
        "running" => "Working",
        "completed" => "Done",
        "error" => "Needs attention",
        "interrupted" => "Stopped",
        "idle" => "Waiting",
        _ => "Unknown",
    }
    .to_owned()
}

fn should_open_initial_visor(items: &[Value]) -> bool {
    items
        .first()
        .is_some_and(|item| matches!(field(item, "status").as_str(), "queued" | "running"))
}

fn human_name(path: &str) -> String {
    let leaf = path
        .split('/')
        .filter(|part| !part.is_empty())
        .next_back()
        .unwrap_or_default();
    let value = leaf
        .strip_prefix("agent-")
        .or_else(|| leaf.strip_prefix("agent_"))
        .unwrap_or(leaf);
    let mut result = String::new();
    let mut capitalize = true;
    for character in value.chars() {
        if character == '-' || character == '_' {
            if !result.ends_with(' ') && !result.is_empty() {
                result.push(' ');
            }
            capitalize = true;
        } else if capitalize {
            result.extend(character.to_uppercase());
            capitalize = false;
        } else {
            result.push(character);
        }
    }
    if result.is_empty() {
        "Subagent".into()
    } else {
        result
    }
}

fn format_time(milliseconds: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(milliseconds)
        .map(|time| {
            time.with_timezone(&chrono::Local)
                .format("%-I:%M %p")
                .to_string()
        })
        .unwrap_or_default()
}

fn status_chip(status: &str, palette: crate::design::Palette) -> AnyElement {
    let color = match status {
        "Done" => 0x6f9278,
        "Needs attention" => 0xdc6262,
        _ => palette.accent,
    };
    div()
        .flex()
        .items_center()
        .gap(px(4.))
        .text_size(px(9.))
        .font_family(crate::design::FONT_MONO)
        .text_color(rgb(palette.muted))
        .child(div().size(px(6.)).rounded_full().bg(rgb(color)))
        .child(status.to_owned())
        .into_any_element()
}

impl Render for SubagentPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self.composed_palette.unwrap_or_else(|| {
            crate::design::palette(&self.snapshot["settings"], window.appearance())
        });
        let font_ratio = self.snapshot["settings"]["interfaceFontSize"]
            .as_f64()
            .filter(|size| size.is_finite() && *size > 0.)
            .unwrap_or(14.) as f32
            / 14.;
        let mut active: Vec<Value> = self
            .items
            .iter()
            .filter(|item| matches!(field(item, "status").as_str(), "queued" | "running"))
            .cloned()
            .collect();
        let mut recent: Vec<Value> = self
            .items
            .iter()
            .filter(|item| !matches!(field(item, "status").as_str(), "queued" | "running"))
            .cloned()
            .collect();
        recent.sort_by(|a, b| {
            b["updatedAt"]
                .as_i64()
                .cmp(&a["updatedAt"].as_i64())
                .then_with(|| field(a, "title").cmp(&field(b, "title")))
        });
        recent.truncate(3);
        let sidebar_scroll = window.use_keyed_state(format!("subagent-sidebar-overflow-{}", cx.entity_id()), cx,
            |_, _| ScrollHandle::new()).read(cx).clone();
        let mut root = div()
            .size_full()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(crate::design::space::SECTION))
            .p(px(crate::design::space::INSET))
            .id("subagent-sidebar-scroll")
            .overflow_y_scroll()
            .track_scroll(&sidebar_scroll)
            .bg(if self.compact_header {
                rgba(0x00000000)
            } else {
                crate::design::surface_color(palette.sidebar)
            })
            .text_color(rgb(palette.ink));
        if !self.compact_header {
            root = root.child(
                div()
                    .text_size(px(14. * font_ratio))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Subagents"),
            );
        }
        for (heading, items) in [("ACTIVE", active), ("RECENT", recent)] {
            let mut section = div().flex().flex_col().gap(px(3.)).min_w_0().child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .mx(px(4.))
                    .mb(px(5.))
                    .font_family(crate::design::FONT_MONO)
                    .text_size(px(10. * font_ratio))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(palette.muted))
                    .child(heading)
                    .child(
                        div()
                            .font_weight(FontWeight::NORMAL)
                            .child(items.len().to_string()),
                    ),
            );
            if items.is_empty() {
                let empty = if heading == "ACTIVE" {
                    "No active subagents."
                } else {
                    "No recent subagent work."
                };
                section = section.child(
                    div()
                        .py(px(7.))
                        .px(px(9.))
                        .text_size(px(11. * font_ratio))
                        .line_height(px(15.4 * font_ratio))
                        .text_color(rgb(palette.muted))
                        .child(empty),
                );
            } else {
                for item in items {
                    let id = field(&item, "id");
                    let selected = self.selected.as_deref() == Some(id.as_str());
                    let title = item["title"]
                        .as_str()
                        .filter(|title| !title.is_empty())
                        .unwrap_or("Untitled subagent")
                        .to_owned();
                    let agent = item["agentName"].as_str().unwrap_or("Subagent").to_owned();
                    let initials = item["agentInitials"].as_str().unwrap_or("S").to_owned();
                    let activity = item["activity"]
                        .as_str()
                        .filter(|value| !value.is_empty())
                        .map(|value| format!("{agent} · {value}"))
                        .unwrap_or_else(|| agent.clone());
                    let status = field(&item, "status");
                    let label = status_label(&status);
                    let selected_id = id.clone();
                    let accessible_name = format!("{title}, {agent}, {label}");
                    let (status_icon, status_color) = status_marker(&status, palette);
                    let completion_marker = self.marker(&id, &status, false, palette, 12., cx);
                    let hover_motion = gpui_kit::component::hover_motion::HoverMotion::new(
                        SharedString::from(format!("subagent-sidebar-hover-{id}")), !selected, window, cx,
                    );
                    let hover_background = hover_motion.color(
                        "background", rgba(0x00000000).into(),
                        crate::design::surface_color(palette.soft).into(), window, cx,
                    );
                    let mut row = div()
                        .id(SharedString::from(format!(
                            "subagent-sidebar-row-{}",
                            id.chars()
                                .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
                                .collect::<String>()
                        )))
                        .role(Role::Button)
                        .aria_label(accessible_name)
                        .w_full()
                        .min_w_0()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .px(px(9.))
                        .py(px(8.))
                        .mb(px(crate::design::space::INSET))
                        .rounded_md()
                        .border_1()
                        .border_color(if selected {
                            rgb(palette.accent)
                        } else {
                            rgba(0x00000000)
                        })
                        .bg(if selected {
                            crate::design::surface_color(palette.soft)
                        } else {
                            hover_background.into()
                        })
                        .cursor_pointer()
                        .on_hover(move |hovered, _, cx| hover_motion.on_hover(*hovered, cx))
                        .child(
                            div()
                                .size(px(22.))
                                .flex_none()
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_md()
                                .bg(rgb(palette.accent))
                                .text_size(px(9.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .font_family(crate::design::FONT_MONO)
                                .text_color(rgb(0xffffff))
                                .child(initials),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap(px(2.))
                                .child(
                                    div()
                                        .min_w_0()
                                        .flex()
                                        .items_center()
                                        .gap(px(7.))
                                        .child(
                                            div()
                                                .min_w_0()
                                                .flex_1()
                                                .truncate()
                                                .text_size(px(12. * font_ratio))
                                                .font_weight(FontWeight::MEDIUM)
                                                .child(title),
                                        )
                                        .child(
                                            div()
                                                .flex_none()
                                                .flex()
                                                .items_center()
                                                .gap(px(4.))
                                                .font_family(crate::design::FONT_MONO)
                                                .text_size(px(9. * font_ratio))
                                                .text_color(rgb(palette.muted))
                                                .child(
                                                    div()
                                                        .size(px(5.))
                                                        .rounded_full()
                                                        .bg(rgb(status_color)),
                                                )
                                                .child(label),
                                        ),
                                )
                                .child(
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .text_size(px(10. * font_ratio))
                                        .text_color(rgb(palette.muted))
                                        .child(activity),
                                ),
                        )
                        .when(!status_icon.is_empty(), |row| {
                            row.child(
                                completion_marker,
                            )
                        })
                        .on_click(
                            cx.listener(move |pane, _, _, cx| pane.select(selected_id.clone(), cx)),
                        );
                    section = section.child(row);
                }
            }
            root = root.child(section);
        }
        ScrollEdgeFade::new(root, sidebar_scroll, Axis::Vertical, gpui_kit::px(20.))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn sidebar_and_visor_share_session_status_markers() {
        let palette = crate::design::palette(&Value::Null, WindowAppearance::Dark);
        assert_eq!(status_marker("completed", palette), ("✓", 0x6f9278));
        assert_eq!(status_label("completed"), "Done");
        assert_eq!(status_marker("running", palette), ("◌", palette.accent));
        assert_eq!(status_label("running"), "Working");
        assert_eq!(status_marker("queued", palette), ("◷", palette.accent));
        assert_eq!(status_label("queued"), "Starting");
        assert_eq!(status_marker("error", palette), ("!", 0xdc6262));
        assert_eq!(status_label("error"), "Needs attention");
    }

    #[::core::prelude::v1::test]
    fn native_session_projects_human_title_and_completed_defaults_closed() {
        let snapshot = json!({
            "subagentSessions": [{
                "id": "native-1", "source": "codex", "parentTaskId": "parent",
                "agentThreadId": "thread-1", "agentPath": "/agents/agent-research_helper",
                "status": "completed", "prompt": "Summarize the test results\nwith details",
                "createdAt": 100, "updatedAt": 200
            }],
            "tasks": [], "agents": [], "collaborations": [], "events": [], "messages": [], "projects": []
        });
        let items = records(&snapshot, "parent");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["title"], "Summarize the test results");
        assert_eq!(items[0]["agentName"], "Research Helper");
        assert_eq!(items[0]["agentInitials"], "RH");
        assert!(!should_open_initial_visor(&items));
    }

    #[::core::prelude::v1::test]
    fn follows_nested_delegations_and_prefers_reported_model() {
        let snapshot = json!({
            "subagentSessions": [
                {"id":"session-1","source":"delegation","parentTaskId":"parent","collaborationId":"c1","status":"running","createdAt":100,"updatedAt":110},
                {"id":"session-2","source":"delegation","parentTaskId":"child-1","collaborationId":"c2","status":"completed","createdAt":120,"updatedAt":130}
            ],
            "collaborations": [
                {"id":"c1","kind":"delegation","fromTaskId":"parent","toTaskId":"child-1","toAgentId":"agent-1","text":"First task","status":"running","createdAt":100,"updatedAt":110},
                {"id":"c2","kind":"delegation","fromTaskId":"child-1","toTaskId":"child-2","toAgentId":"agent-2","text":"Nested task","status":"completed","createdAt":120,"updatedAt":130}
            ],
            "tasks": [
                {"id":"child-1","title":"First child","agentId":"agent-1","model":"default","createdAt":100,"updatedAt":110},
                {"id":"child-2","title":"Nested child","agentId":"agent-2","model":"default","createdAt":120,"updatedAt":130}
            ],
            "agents": [
                {"id":"agent-1","name":"Research","model":"fallback-model"},
                {"id":"agent-2","name":"Writer","model":"writer-model"}
            ],
            "messages": [{"id":"m1","taskId":"child-1","role":"assistant","createdAt":150,"responseMetadata":{"model":"reported-model"}}],
            "projects": [], "events": []
        });
        let items = records(&snapshot, "parent");
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["title"], "First child");
        assert_eq!(items[0]["model"], "reported-model");
        assert_eq!(items[1]["title"], "Nested child");
        assert_eq!(items[1]["agentName"], "Writer");
        assert!(should_open_initial_visor(&items));
    }
    #[::core::prelude::v1::test]
    fn prepared_child_transcripts_scope_sort_and_retain_unchanged_rows() {
        let items = vec![json!({"id":"child-record","taskId":"child"})];
        let mut snapshot = json!({
            "messages":[
                {"id":"unrelated","taskId":"other","role":"assistant","text":"ignore","createdAt":0},
                {"id":"reply","taskId":"child","role":"assistant","text":"reply","createdAt":20},
                {"id":"tool-message","taskId":"child","role":"tool","text":"ignore role","createdAt":5}
            ],
            "events":[{"id":"tool","taskId":"child","kind":"tool","title":"Read file","createdAt":10}],
            "subagentTranscripts":{}
        });
        let first = prepare_child_transcripts(&snapshot, &items, None);
        assert_eq!(first["child-record"].len(), 2);
        assert_eq!(first["child-record"][0]["id"], "tool");
        assert_eq!(first["child-record"][1]["id"], "reply");
        let same = prepare_child_transcripts(&snapshot, &items, Some(&first));
        assert!(Arc::ptr_eq(&first["child-record"], &same["child-record"]));
        snapshot["messages"][1]["text"] = json!("new reply");
        let changed = prepare_child_transcripts(&snapshot, &items, Some(&same));
        assert!(!Arc::ptr_eq(&same["child-record"], &changed["child-record"]));
        assert_eq!(changed["child-record"][1]["text"], "new reply");
        snapshot["messages"] = json!([]);
        snapshot["events"] = json!([]);
        let empty = prepare_child_transcripts(&snapshot, &items, Some(&changed));
        assert!(empty["child-record"].is_empty());
    }

}

#[cfg(test)]
#[path = "collaboration_visor_tests.rs"]
mod visor_tests;
