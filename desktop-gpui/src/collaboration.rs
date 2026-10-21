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
    component::{ActiveTheme as _, button::*},
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

#[path = "build_queue.rs"]
pub(crate) mod build_queue;

const TRANSCRIPT_WORKER_STACK: usize = 8 * 1024 * 1024;
pub(super) const PANE_STATUS_BAR_HEIGHT: f32 = 34.;

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
    let native_thread_by_task: HashMap<&str, &str> = rows_ref(snapshot, "tasks")
        .iter()
        .filter_map(|task| Some((task["id"].as_str()?, task["nativeSessionId"].as_str()?)))
        .collect();
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
            // Older backends may have persisted the parent's own Codex thread
            // as a child. Compare exact thread identity against its actual
            // parent task; paths and elapsed time do not establish ownership.
            if session["source"] == "codex"
                && session["agentThreadId"].as_str().filter(|thread| !thread.is_empty())
                    .is_some_and(|thread| native_thread_by_task.get(parent_id.as_str()) == Some(&thread))
            {
                continue;
            }
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

        let reported_model = task
            .and_then(|task| {
                snapshot["nativeMessageContext"][field(task, "id")]["latestReportedModel"]
                    .as_str()
                    .map(str::to_owned)
            })
            .or_else(|| {
                task.and_then(|task| {
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
                })
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
        let native_terminal = matches!(field(item, "source").as_str(), "codex" | "acp")
            && matches!(field(item, "status").as_str(), "completed" | "error" | "interrupted");
        // A later interaction notice is not additional execution time for a
        // finished native child. Keep the backend's terminal timestamp.
        let update_time = if native_terminal {
            item["updatedAt"].as_i64().unwrap_or_default()
        } else {
            [
                item["updatedAt"].as_i64(),
                task.and_then(|task| task["updatedAt"].as_i64()),
                collaboration.and_then(|value| value["updatedAt"].as_i64()),
                latest_event.and_then(|event| event["createdAt"].as_i64()),
            ]
            .into_iter()
            .flatten()
            .max()
            .unwrap_or_default()
        };
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
    snapshot: &Value,
    items: &[Value],
    previous: Option<&monitter_gpui_desktop::native_publication::PreparedChildTranscripts>,
) -> monitter_gpui_desktop::native_publication::PreparedChildTranscripts {
    prepare_child_transcripts_inner(snapshot, None, items, previous)
}
pub(crate) fn prepare_child_transcripts_indexed(
    snapshot: &Value,
    index: &monitter_gpui_desktop::native_publication::PublicationIndex,
    items: &[Value],
    previous: Option<&monitter_gpui_desktop::native_publication::PreparedChildTranscripts>,
) -> monitter_gpui_desktop::native_publication::PreparedChildTranscripts {
    prepare_child_transcripts_inner(snapshot, Some(index), items, previous)
}
fn prepare_child_transcripts_inner(
    snapshot: &Value,
    index: Option<&monitter_gpui_desktop::native_publication::PublicationIndex>,
    items: &[Value],
    previous: Option<&monitter_gpui_desktop::native_publication::PreparedChildTranscripts>,
) -> monitter_gpui_desktop::native_publication::PreparedChildTranscripts {
    let mut transcripts = HashMap::new();
    for item in items {
        let id = field(item, "id");
        let entries = if let Some(task_id) = item["taskId"].as_str() {
            let owned = index.map(|index| index.task_history(task_id));
            let messages = owned
                .as_deref()
                .unwrap_or_else(|| rows_ref(snapshot, "messages"));
            let mut entries: Vec<Value> = messages.iter()
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
        } else if let Some(entries) = snapshot["subagentTranscripts"]
            .get(&id)
            .and_then(Value::as_array)
        {
            entries.clone()
        } else {
            continue;
        };
        let retained = previous
            .and_then(|old| old.get(&id))
            .filter(|old| old.as_ref() == &entries);
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

fn has_pane_work_plan(snapshot: &Value, task_id: Option<&str>) -> bool {
    task_id.is_some_and(|task_id| {
        snapshot["workPlans"].as_array().is_some_and(|plans| {
            plans
                .iter()
                .any(|plan| plan["taskId"].as_str() == Some(task_id))
        })
    })
}

#[derive(Default)]
struct VisorMenus {
    subagents: bool,
    plan: bool,
    detail: bool,
    last_plan: bool,
    builds: bool,
    last_builds: bool,
}

impl VisorMenus {
    fn toggle_subagents(&mut self) {
        self.builds = false;
        self.last_builds = false;
        self.subagents = !self.subagents;
        self.plan = false;
        self.last_plan = false;
        self.detail = false;
    }
    fn toggle_plan(&mut self) {
        self.builds = false;
        self.last_builds = false;
        self.plan = !self.plan;
        self.subagents = false;
        self.last_plan = true;
    }
    fn select_detail(&mut self) {
        self.builds = false;
        self.last_builds = false;
        self.plan = false;
        self.last_plan = false;
        self.detail = true;
    }
    fn toggle_builds(&mut self) {
        self.builds = !self.builds;
        self.last_builds = true;
        self.plan = false;
        self.last_plan = false;
        self.subagents = false;
        self.detail = false;
    }
    fn reset_task(&mut self) {
        *self = Self::default();
    }
}

pub enum SubagentPaneEvent {
    ShowGraph,
}

impl EventEmitter<SubagentPaneEvent> for SubagentPane {}

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
    menus: VisorMenus,
    drawer_fraction: f32,
    drawer_drag: Option<(f32, f32)>,
    drawer_revision: u64,
    drawer_save_error: Option<String>,
    build_recovery_confirm: Option<String>,
    build_recovery_error: Option<String>,
    build_recovery_busy: bool,
    work_plan: Entity<crate::plans::WorkPlanPane>,
    build_queue_scroll: ScrollHandle,
    work_plan_scroll: ScrollHandle,
    subagent_scroll: ScrollHandle,
    compact_header: bool,
    composed_palette: Option<crate::design::Palette>,
    completion_markers: HashMap<(String, bool), Entity<CompletionMarker>>,
    sidebar_visible: bool,
    visor_visible: bool,
    suppress_completion: bool,
}

impl SubagentPane {
    fn save_drawer_height(&mut self, cx: &mut Context<Self>) {
        let Some(task) = self.parent_task.clone() else {
            return;
        };
        let fraction = self.drawer_fraction;
        let revision = self.drawer_revision;
        let version = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
            .min(i64::MAX as u128) as i64;
        self.drawer_save_error = None;
        let (sender, receiver) = oneshot::channel();
        std::thread::spawn(move || {
            let _ = sender.send(crate::drawer_preferences::save_versioned(
                &task, fraction, version,
            ));
        });
        cx.spawn(async move |view, cx| {
            let result = receiver
                .await
                .unwrap_or_else(|_| Err("Drawer preference writer stopped.".into()));
            let _ = view.update(cx, |view, cx| {
                if view.drawer_revision != revision {
                    return;
                }
                view.drawer_save_error = result.err();
                cx.notify();
            });
        })
        .detach();
    }
    pub fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        self.set_surface_visibility(visible, false, cx);
    }
    pub(crate) fn set_surface_visibility(
        &mut self,
        sidebar: bool,
        visor: bool,
        _cx: &mut Context<Self>,
    ) {
        if (self.sidebar_visible, self.visor_visible) == (sidebar, visor) {
            return;
        }
        self.sidebar_visible = sidebar;
        self.visor_visible = visor;
        for ((_, is_visor), marker) in &self.completion_markers {
            marker.update(_cx, |marker, cx| {
                marker.set_visible(if *is_visor { visor } else { sidebar }, cx)
            });
        }
    }
    /// Full snapshots and reconnects seed final status rather than replaying events.
    pub(crate) fn suppress_completion_motion(&mut self, cx: &mut Context<Self>) {
        self.suppress_completion = true;
        for marker in self.completion_markers.values() {
            marker.update(cx, |marker, cx| marker.suppress(cx));
        }
    }
    fn marker(
        &mut self,
        id: &str,
        status: &str,
        visor: bool,
        palette: crate::design::Palette,
        size: f32,
        cx: &mut Context<Self>,
    ) -> Entity<CompletionMarker> {
        let visible = if visor {
            self.visor_visible
        } else {
            self.sidebar_visible
        };
        let marker = self
            .completion_markers
            .entry((id.to_owned(), visor))
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
    pub fn records(&self) -> Arc<Vec<Value>> {
        self.items.clone()
    }

    fn recover_build(&mut self, run_id: String, cx: &mut Context<Self>) {
        if self.build_recovery_busy {
            return;
        }
        self.build_recovery_busy = true;
        self.build_recovery_error = None;
        let service = self.service.clone();
        let (sender, receiver) = oneshot::channel();
        std::thread::spawn(move || {
            let _ = sender.send(service.invoke("recover_project_build",
            json!({"buildRunId":run_id,"processStopped":true,"summary":"User confirmed the build process has stopped; prior result remains unknown."})));
        });
        cx.spawn(async move |view, cx| {
            let result = receiver
                .await
                .unwrap_or_else(|_| Err("Build recovery worker stopped.".into()));
            let _ = view.update(cx, |view, cx| {
                view.build_recovery_busy = false;
                view.build_recovery_confirm = None;
                view.build_recovery_error = result.err();
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub fn dismiss_drawer(&mut self, cx: &mut Context<Self>) -> bool {
        let open = self.visor_open();
        self.menus.subagents = false;
        self.menus.plan = false;
        self.menus.builds = false;
        if open {
            cx.notify();
        }
        open
    }

    pub fn record_count(&self) -> usize {
        self.items.len()
    }

    /// The parent chat uses this to decide whether to mount the child visor below its composer.
    pub fn visor_open(&self) -> bool {
        (self.menus.builds
            && build_queue::project(&self.snapshot, self.parent_task.as_deref()).is_some())
            || (self.menus.subagents && !self.items.is_empty())
            || (self.menus.plan && has_pane_work_plan(&self.snapshot, self.parent_task.as_deref()))
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
            menus: VisorMenus::default(),
            drawer_fraction: 0.5,
            drawer_drag: None,
            drawer_revision: 0,
            drawer_save_error: None,
            build_recovery_confirm: None,
            build_recovery_error: None,
            build_recovery_busy: false,
            work_plan: _cx.new(crate::plans::WorkPlanPane::new),
            build_queue_scroll: ScrollHandle::new(),
            work_plan_scroll: ScrollHandle::new(),
            subagent_scroll: ScrollHandle::new(),
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
        let transcripts =
            prepare_child_transcripts(&snapshot, &items, Some(&self.prepared_transcripts));
        self.set_prepared_context(snapshot, task_id, items, transcripts, cx);
    }
    /// Publication dependency comparison proved this parent's prepared child
    /// records/transcripts unchanged. Release the older global source without
    /// rebuilding records, fencing native reads, or notifying the visor.
    pub(crate) fn advance_unaffected_snapshot(&mut self, snapshot: Rc<Value>) {
        self.snapshot = snapshot;
        self.reconcile_menus();
    }
    fn reconcile_menus(&mut self) {
        if build_queue::project(&self.snapshot, self.parent_task.as_deref()).is_none() {
            self.menus.builds = false;
        }
        if self.items.is_empty() {
            self.menus.subagents = false;
            self.menus.detail = false;
        }
        if !has_pane_work_plan(&self.snapshot, self.parent_task.as_deref()) {
            self.menus.plan = false;
        }
    }
    pub fn set_prepared_context(
        &mut self,
        snapshot: Rc<Value>,
        task_id: &str,
        items: Arc<Vec<Value>>,
        transcripts: monitter_gpui_desktop::native_publication::PreparedChildTranscripts,
        cx: &mut Context<Self>,
    ) {
        let changed = self.parent_task.as_deref() != Some(task_id);
        self.snapshot = snapshot;
        self.parent_task = Some(task_id.to_owned());
        if changed {
            self.completion_markers.clear();
            self.menus.reset_task();
            self.drawer_fraction = 0.5;
            self.drawer_drag = None;
            self.drawer_revision += 1;
            self.drawer_save_error = None;
            let revision = self.drawer_revision;
            let task = task_id.to_owned();
            let (sender, receiver) = oneshot::channel();
            std::thread::spawn(move || {
                let _ = sender.send(crate::drawer_preferences::load(&task));
            });
            cx.spawn(async move |view, cx| {
                let result = receiver
                    .await
                    .unwrap_or_else(|_| Err("Drawer preference reader stopped.".into()));
                let _ = view.update(cx, |view, cx| {
                    if view.drawer_revision != revision {
                        return;
                    }
                    match result {
                        Ok(Some(fraction)) => view.drawer_fraction = fraction,
                        Ok(None) => {}
                        Err(error) => view.drawer_save_error = Some(error),
                    }
                    cx.notify();
                });
            })
            .detach();
        }
        let live = !changed && !self.suppress_completion;
        self.completion_markers
            .retain(|(id, _), _| items.iter().any(|item| field(item, "id") == *id));
        for ((id, visor), marker) in &self.completion_markers {
            let Some(item) = items.iter().find(|item| field(item, "id") == *id) else {
                continue;
            };
            let visible = if *visor {
                self.visor_visible
            } else {
                self.sidebar_visible
            };
            marker.update(cx, |marker, cx| {
                marker.set_status(id, &field(item, "status"), live && visible, cx)
            });
        }
        self.suppress_completion = false;
        self.items = items;
        self.reconcile_menus();
        self.prepared_transcripts = transcripts;
        if changed
            || (self.selected.is_none() && !self.items.is_empty())
            || self
                .selected
                .as_ref()
                .is_some_and(|id| !self.items.iter().any(|v| field(v, "id") == *id))
        {
            self.menus.detail = false;
            self.selected = self.items.first().map(|v| field(v, "id"));
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
                !item["taskId"].is_null()
                    || (field(item, "source") == "codex"
                        && self.menus.subagents
                        && self.menus.detail)
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
        self.menus.select_detail();
        if self.selected.as_deref() == Some(&id) {
            self.menus.subagents = true;
            self.load_transcript(cx);
            cx.notify();
            return;
        }
        self.selected = Some(id);
        self.menus.subagents = true;
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
        if !self.menus.subagents || !self.menus.detail {
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
                let _ = sender.send(
                    result.map(|entries| Arc::new(entries.as_array().cloned().unwrap_or_default())),
                );
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
                    Ok(entries) => pane.replace_transcript(entries),
                    Err(error) => {
                        pane.transcript_load_key = None;
                        pane.error = Some(error);
                    }
                }
                // A newer child record may have arrived while this bounded
                // read was in flight. The load key prevents duplicate reads.
                if pane.error.is_none() && pane.menus.subagents && pane.menus.detail {
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
        pane_width: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.render_visor_mode(palette, pane_height, pane_width, false, window, cx)
    }

    pub fn render_graph_bar(
        &mut self,
        palette: crate::design::Palette,
        pane_height: f32,
        pane_width: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.render_visor_mode(palette, pane_height, pane_width, true, window, cx)
    }

    fn render_visor_mode(
        &mut self,
        palette: crate::design::Palette,
        pane_height: f32,
        pane_width: f32,
        _graph_open: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let entries = self.transcript.clone();
        let task_id = self.parent_task.clone().unwrap_or_default();
        let selected = self.selected.clone();
        let item = self
            .items
            .iter()
            .find(|item| selected.as_deref() == Some(field(item, "id").as_str()))
            .cloned();
        let pane_height = if pane_height > 0.0 {
            pane_height
        } else {
            480.0
        };
        let expanded_height =
            (pane_height * self.drawer_fraction).min((pane_height - 180.).max(0.));
        let slide_height = visor_slide(
            SharedString::from(format!(
                "subagent-visor-slide-{task_id}-{:?}",
                cx.entity_id()
            )),
            self.visor_open(),
            expanded_height,
            self.drawer_drag.is_none() && crate::native_motion::motion_allowed(),
            window,
            cx,
        );
        self.reconcile_menus();
        let has_subagents = !self.items.is_empty();
        let has_plan = has_pane_work_plan(&self.snapshot, self.parent_task.as_deref());
        let build_project = build_queue::project(&self.snapshot, self.parent_task.as_deref())
            .map(|project| field(project, "id"));
        if !has_subagents && !has_plan && build_project.is_none() {
            return div().into_any_element();
        }
        // Use the owning pane's logical pixel width, independent of window size
        // and interface scale. Start compact until its first layout measurement.
        let icon_only = pane_width < 700.;
        let action_button = |id: &'static str, label: String, icon: gpui_kit::assets::IconName| {
            Button::new(id)
                .icon(icon)
                .tooltip(label.clone())
                .accessibility_label(label.clone())
                .when(!icon_only, |button| button.label(label))
        };
        let pill = |id: &'static str, button: Button, selected: bool| {
            div()
                .id(id)
                .rounded(px(4.))
                .border_1()
                .border_color(
                    rgb(if selected {
                        palette.accent
                    } else {
                        palette.line
                    })
                    .opacity(if selected { 1. } else { 0. }),
                )
                .bg(rgb(palette.accent).opacity(if selected { 0.10 } else { 0. }))
                .hover(|style| {
                    style
                        .border_color(rgb(palette.accent))
                        .bg(rgb(palette.accent).opacity(0.10))
                })
                .child(
                    button
                        .compact()
                        .text()
                        .rounded(px(4.))
                        .px(px(8.))
                        .py(px(4.))
                        .bg(rgb(palette.accent).opacity(0.))
                        .text_color(rgb(if selected { palette.ink } else { palette.muted }))
                        .text_size((cx.theme().font_size - px(4.)).max(px(8.))),
                )
        };
        let mut bar = div()
            .w_full()
            .h(px(PANE_STATUS_BAR_HEIGHT))
            .flex()
            .items_center()
            .gap(px(5.))
            .border_t_1()
            .border_color(rgb(palette.line))
            .px(px(10.))
            .bg(crate::surface_blend::background(
                crate::surface_blend::Area::Pane,
                palette.paper,
            ));
        if has_subagents {
            bar = bar.child(pill(
                "subagent-menu-pill",
                action_button("subagent-menu-toggle", format!("Subagents ({})", self.items.len()), gpui_kit::assets::IconName::Users)
                    .toggled(self.menus.subagents)
                    .on_click(cx.listener(|pane, _, _, cx| {
                        pane.menus.toggle_subagents();
                        cx.notify();
                    })),
                self.menus.subagents,
            ));
        }
        if let Some(project_id) = &build_project {
            let count = build_queue::pending_count(&self.snapshot, project_id);
            bar = bar.child(div().flex_1());
            bar = bar.child(pill(
                "build-queue-menu-pill",
                action_button("build-queue-menu-toggle", if count > 0 {
                        format!("Build Queue ({count})")
                    } else {
                        "Build Queue".into()
                    }, gpui_kit::assets::IconName::Wrench)
                    .toggled(self.menus.builds)
                    .on_click(cx.listener(|pane, _, _, cx| {
                        pane.menus.toggle_builds();
                        cx.notify();
                    })),
                self.menus.builds,
            ));
        }
        // Window-level capture keeps resizing responsive across scrollable
        // drawer children and while the pointer leaves the grab band.
        let move_owner = cx.weak_entity();
        let release_owner = move_owner.clone();
        let input = canvas(
            |_, _, _| (),
            move |_, _, window, _| {
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                    if !phase.capture() || event.pressed_button != Some(MouseButton::Left) {
                        return;
                    }
                    let _ = move_owner.update(cx, |pane, cx| {
                        if let Some((start_y, start_fraction)) = pane.drawer_drag {
                            let y = event.position.y / crate::design::ui_scaled_px(1.);
                            pane.drawer_fraction =
                                (start_fraction + (start_y - y) / pane_height).clamp(0.1, 0.8);
                            cx.notify();
                            cx.stop_propagation();
                        }
                    });
                });
                window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                    if !phase.capture() || event.button != MouseButton::Left {
                        return;
                    }
                    let _ = release_owner.update(cx, |pane, cx| {
                        if pane.drawer_drag.take().is_none() {
                            return;
                        }
                        pane.save_drawer_height(cx);
                        cx.notify();
                        cx.stop_propagation();
                    });
                });
            },
        )
        .absolute()
        .inset_0();
        bar = bar
            .relative()
            .flex_shrink_0()
            .debug_selector(|| "pane-drawer-toolbar".into())
            .child(input);
        if slide_height > 0.1 {
            bar = bar.child(
                div()
                    .id("pane-drawer-resize")
                    .debug_selector(|| "pane-drawer-resize".into())
                    .absolute()
                    .top_0()
                    .left_0()
                    .w_full()
                    .h(px(6.))
                    .cursor(CursorStyle::ResizeUpDown)
                    .flex()
                    .justify_center()
                    .items_center()
                    .child(
                        div()
                            .w(px(28.))
                            .h(px(2.))
                            .rounded(px(1.))
                            .bg(rgb(palette.muted).opacity(0.5)),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |pane, event: &MouseDownEvent, _, cx| {
                            pane.drawer_revision += 1;
                            pane.drawer_drag = Some((
                                event.position.y / crate::design::ui_scaled_px(1.),
                                expanded_height / pane_height,
                            ));
                            cx.notify();
                            cx.stop_propagation();
                        }),
                    ),
            );
        }
        let mut visor = div()
            .w_full()
            .min_w_0()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .child(bar);
        if slide_height > 0.1 && self.menus.last_builds {
            if let Some(project_id) = &build_project {
                let mut queue = build_queue::content(&self.snapshot, project_id, palette);
                for run in build_queue::stalled_runs(&self.snapshot, project_id) {
                    let run_id = field(run, "id");
                    let confirmed = self.build_recovery_confirm.as_deref() == Some(run_id.as_str());
                    let action_id = run_id.clone();
                    queue = queue.child(div().mt(px(8.)).text_size(px(11.))
                        .child("This build stopped reporting. Its slot stays reserved until its process is confirmed stopped.")
                        .child(Button::new(SharedString::from(format!("recover-build-{run_id}")))
                            .compact().text().label(if confirmed {"I have stopped this build — release slot"} else {"Release stalled build…"})
                            .on_click(cx.listener(move |view,_,_,cx| {
                                if confirmed {view.recover_build(action_id.clone(),cx);} else {view.build_recovery_confirm=Some(action_id.clone());cx.notify();}
                            }))));
                }
                if let Some(error) = &self.build_recovery_error {
                    queue = queue.child(error.clone());
                }
                let scroll = self.build_queue_scroll.clone();
                visor = visor.child(clipped_visor_body(
                    slide_height,
                    crate::transcript_edge_fade::TranscriptEdgeFade::for_scroll(
                        div()
                            .absolute()
                            .bottom_0()
                            .left_0()
                            .w_full()
                            .h(px(expanded_height))
                            .id("build-queue-drawer")
                            .overflow_y_scroll()
                            .track_scroll(&scroll)
                            .p(px(10.))
                            .bg(crate::design::surface_color(palette.paper))
                            .child(queue),
                        scroll,
                        px(18.),
                    ),
                ));
            }
        } else if slide_height > 0.1 && self.menus.last_plan {
            self.work_plan.update(cx, |plan, cx| {
                plan.set_current_only(true, cx);
                plan.set_context(&self.snapshot, &task_id, palette, 1., cx);
            });
            let scroll = self.work_plan_scroll.clone();
            visor = visor.child(clipped_visor_body(
                slide_height,
                crate::transcript_edge_fade::TranscriptEdgeFade::for_scroll(
                    div()
                        .absolute()
                        .bottom_0()
                        .left_0()
                        .w_full()
                        .h(px(expanded_height))
                        .id("work-plan-drawer")
                        .overflow_y_scroll()
                        .track_scroll(&scroll)
                        .p(px(10.))
                        .bg(crate::design::surface_color(palette.paper))
                        .child(self.work_plan.clone()),
                    scroll,
                    px(18.),
                ),
            ));
        } else if slide_height > 0.1 && (!self.menus.detail || item.is_none()) {
            let mut body = div()
                .id("subagent-drawer-list")
                .absolute()
                .bottom_0()
                .left_0()
                .w_full()
                .h(px(expanded_height))
                .overflow_y_scroll()
                .track_scroll(&self.subagent_scroll)
                .p(px(10.))
                .bg(crate::design::surface_color(palette.paper))
                .flex()
                .flex_col()
                .gap(px(5.));
            if self.items.is_empty() {
                body = body.child(
                    div()
                        .text_color(rgb(palette.muted))
                        .child("No subagents for this task."),
                );
            }
            let mut last_active = None;
            for (index, item) in self.items.clone().iter().enumerate() {
                let id = field(item, "id");
                let status = field(item, "status");
                let active = matches!(status.as_str(), "running" | "queued");
                if last_active != Some(active) {
                    body = body.child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(palette.muted))
                            .child(if active { "Running & queued" } else { "Recent" }),
                    );
                    last_active = Some(active);
                }
                let marker = self.marker(&id, &status, true, palette, 10., cx);
                body = body.child(
                    div()
                        .id(SharedString::from(format!("subagent-list-row-{index}")))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .px(px(8.))
                        .py(px(7.))
                        .rounded_md()
                        .cursor_pointer()
                        .child(marker)
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .child(field(item, "title")),
                        )
                        .child(
                            div()
                                .text_size(px(10.))
                                .text_color(rgb(palette.muted))
                                .child(status_label(&status)),
                        )
                        .on_click(cx.listener(move |pane, _, _, cx| pane.select(id.clone(), cx))),
                );
            }
            let scroll = self.subagent_scroll.clone();
            visor = visor.child(clipped_visor_body(
                slide_height,
                crate::transcript_edge_fade::TranscriptEdgeFade::for_scroll(body, scroll, px(18.)),
            ));
        } else if slide_height > 0.1 {
            let item = item.expect("detail requires an available subagent");
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
            body = body.child(
                Button::new("subagent-back-to-list")
                    .compact()
                    .text()
                    .label("← Subagents")
                    .on_click(cx.listener(|pane, _, _, cx| {
                        pane.menus.detail = false;
                        cx.notify();
                    })),
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
        if let Some(error) = &self.drawer_save_error {
            visor = visor.child(
                div()
                    .text_size(px(10.))
                    .child(format!("Drawer size: {error}")),
            );
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
        let sidebar_scroll = window
            .use_keyed_state(
                format!("subagent-sidebar-overflow-{}", cx.entity_id()),
                cx,
                |_, _| ScrollHandle::new(),
            )
            .read(cx)
            .clone();
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
                        SharedString::from(format!("subagent-sidebar-hover-{id}")),
                        !selected,
                        window,
                        cx,
                    );
                    let hover_background = hover_motion.color(
                        "background",
                        rgba(0x00000000).into(),
                        crate::design::surface_color(palette.soft).into(),
                        window,
                        cx,
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
                        .when(!status_icon.is_empty(), |row| row.child(completion_marker))
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
    fn native_session_projects_human_title() {
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
    }

    #[::core::prelude::v1::test]
    fn codex_parent_root_is_hidden_without_hiding_true_children_or_other_sources() {
        let mut snapshot = json!({
            "tasks":[{"id":"parent","nativeSessionId":"parent-thread"},{"id":"other-parent","nativeSessionId":"child-thread"}],
            "subagentSessions":[
                {"id":"self","source":"codex","parentTaskId":"parent","agentThreadId":"parent-thread","agentPath":"/root","status":"running"},
                {"id":"child","source":"codex","parentTaskId":"parent","agentThreadId":"child-thread","agentPath":"/root","status":"running"},
                {"id":"acp","source":"acp","parentTaskId":"parent","agentThreadId":"parent-thread","status":"running"},
                {"id":"delegated","source":"collaboration","parentTaskId":"parent","agentThreadId":"parent-thread","status":"completed"},
                {"id":"unknown-thread","source":"codex","parentTaskId":"parent","agentThreadId":"","status":"completed"}
            ], "collaborations":[],"agents":[],"events":[],"messages":[],"projects":[]
        });
        let items = records(&snapshot,"parent");
        assert_eq!(items.len(),4);
        assert!(!items.iter().any(|item| item["id"] == "self"));
        for id in ["child","acp","delegated","unknown-thread"] {
            assert!(items.iter().any(|item| item["id"] == id), "preserve {id}");
        }
        snapshot["tasks"][0]["nativeSessionId"] = Value::Null;
        assert_eq!(records(&snapshot,"parent").len(),5,"missing parent identity is not evidence of a self row");
        snapshot["tasks"][0]["nativeSessionId"] = json!("");
        assert_eq!(records(&snapshot,"parent").len(),5,"empty thread IDs never establish self ownership");
    }

    #[::core::prelude::v1::test]
    fn terminal_native_subagent_time_ignores_later_interaction_notices() {
        let mut snapshot = json!({
            "subagentSessions": [{
                "id":"codex:child", "source":"codex", "parentTaskId":"parent",
                "agentThreadId":"child", "agentPath":"/root/codex_steering",
                "status":"running", "createdAt":100, "updatedAt":200
            }],
            "events": [{"id":"late", "taskId":"parent", "kind":"subagent",
                "title":"Sub-agent activity", "createdAt":9000,
                "detail":json!({"type":"subAgentActivity", "kind":"interacted",
                    "agentThreadId":"child"}).to_string()}],
            "tasks":[], "agents":[], "collaborations":[], "messages":[], "projects":[]
        });
        assert_eq!(records(&snapshot, "parent")[0]["updatedAt"], 9000);
        for source in ["codex", "acp"] {
            snapshot["subagentSessions"][0]["source"] = json!(source);
            for status in ["completed", "error", "interrupted"] {
                snapshot["subagentSessions"][0]["status"] = json!(status);
                let rows = records(&snapshot, "parent");
                assert_eq!(rows[0]["status"], status);
                assert_eq!(rows[0]["updatedAt"], 200);
                assert_eq!(rows[0]["createdAt"], 100);
            }
        }
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
        assert!(!Arc::ptr_eq(
            &same["child-record"],
            &changed["child-record"]
        ));
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

#[cfg(test)]
mod visor_menu_state_tests {
    use super::{VisorMenus, has_pane_work_plan};

    #[test]
    fn work_plan_button_keeps_completed_plans_for_current_task() {
        let snapshot = serde_json::json!({"workPlans": [
            {"taskId": "current", "status": "closed"},
            {"taskId": "other", "status": "active"}
        ]});
        assert!(has_pane_work_plan(&snapshot, Some("current")));
        assert!(!has_pane_work_plan(&snapshot, Some("missing")));
        assert!(!has_pane_work_plan(&snapshot, None));
        assert!(has_pane_work_plan(&snapshot, Some("other")));
        assert!(!has_pane_work_plan(&serde_json::json!({}), Some("current")));
    }

    #[test]
    fn toggles_are_exclusive_and_close_on_repeat() {
        let mut menus = VisorMenus::default();
        assert!(!menus.subagents && !menus.plan && !menus.detail);
        menus.toggle_builds();
        assert!(menus.builds && !menus.plan && !menus.subagents);
        menus.toggle_builds();
        assert!(!menus.builds);
        menus.toggle_subagents();
        assert!(menus.subagents && !menus.plan && !menus.detail && !menus.builds);
        menus.toggle_plan();
        assert!(menus.plan && !menus.subagents && menus.last_plan);
        menus.toggle_plan();
        assert!(!menus.plan && !menus.subagents && menus.last_plan);
        menus.toggle_subagents();
        assert!(menus.subagents && !menus.plan && !menus.last_plan);
        menus.toggle_subagents();
        assert!(!menus.subagents && !menus.plan);
    }

    #[test]
    fn list_selection_and_task_switch_clear_other_surfaces() {
        let mut menus = VisorMenus::default();
        menus.toggle_plan();
        menus.select_detail();
        assert!(menus.detail && !menus.plan && !menus.last_plan);
        menus.subagents = true;
        menus.toggle_subagents();
        assert!(!menus.detail && !menus.subagents);
        menus.toggle_plan();
        menus.reset_task();
        assert!(!menus.subagents && !menus.plan && !menus.detail && !menus.last_plan);
    }
}
