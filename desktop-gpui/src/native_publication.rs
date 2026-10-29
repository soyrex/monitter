//! Background preparation and bounded publication bookkeeping. No GPUI access.
use crate::native_state::ChangeSet;
use monitter_lib::{
    history::History,
    model::{Message, Snapshot},
};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

/// Hold the previous source across consumer rebinding, then transfer its owned
/// JSON to a background destructor only when no consumer still references it.
/// Shared sources are never cloned or removed from their remaining owners.
pub fn take_unshared_snapshot(snapshot: Rc<Value>) -> Option<Value> {
    Rc::try_unwrap(snapshot).ok()
}

#[derive(Default, Clone)]
pub struct PublicationChanges {
    pub reset: bool,
    pub removed_tasks: HashSet<String>,
    pub reset_channels: HashSet<String>,
    pub message_tasks: HashSet<String>,
    pub trim_tasks: HashSet<String>,
    pub message_ids: HashSet<String>,
    pub removed_message_ids: HashSet<String>,
    pub changed_channels: HashSet<String>,
    /// Source ordering survives stale-worker coalescing and paused retries.
    pub reset_generation: Option<u64>,
    pub channel_reset_generations: HashMap<String, u64>,
    pub task_page_boundaries: HashMap<String, PageBoundary>,
    pub channel_page_boundaries: HashMap<String, PageBoundary>,
}
impl PublicationChanges {
    pub fn from_changes(reset: bool, changes: ChangeSet) -> Self {
        Self {
            reset,
            message_ids: changes.changed_message_ids.into_iter().collect(),
            removed_message_ids: changes.removed_message_ids.into_iter().collect(),
            changed_channels: changes.changed_channel_ids.into_iter().collect(),
            removed_tasks: changes.removed_task_ids.into_iter().collect(),
            reset_channels: changes.reset_channel_ids.into_iter().collect(),
            message_tasks: changes.changed_task_message_ids.into_iter().collect(),
            ..Default::default()
        }
    }
    pub fn merge(&mut self, other: Self) {
        self.message_ids.extend(other.message_ids);
        self.removed_message_ids.extend(other.removed_message_ids);
        self.changed_channels.extend(other.changed_channels);
        self.reset |= other.reset;
        self.removed_tasks.extend(other.removed_tasks);
        self.reset_channels.extend(other.reset_channels);
        self.message_tasks.extend(other.message_tasks);
        self.trim_tasks.extend(other.trim_tasks);
        self.reset_generation = self.reset_generation.max(other.reset_generation);
        for (id, generation) in other.channel_reset_generations {
            self.channel_reset_generations
                .entry(id)
                .and_modify(|old| *old = (*old).max(generation))
                .or_insert(generation);
        }
        fn newest(into: &mut HashMap<String, PageBoundary>, next: HashMap<String, PageBoundary>) {
            for (id, boundary) in next {
                if into
                    .get(&id)
                    .is_none_or(|old| old.generation < boundary.generation)
                {
                    into.insert(id, boundary);
                }
            }
        }
        newest(&mut self.task_page_boundaries, other.task_page_boundaries);
        newest(
            &mut self.channel_page_boundaries,
            other.channel_page_boundaries,
        );
    }
    pub fn task_boundary<'a>(
        &'a self,
        id: &str,
        revision: Option<&str>,
        snapshot: &Value,
    ) -> Option<&'a PageBoundary> {
        let boundary = self.task_page_boundaries.get(id)?;
        if !boundary.after_reset(revision, self.reset_generation)
            || !rows(snapshot, "tasks").any(|task| task["id"].as_str() == Some(id))
            || boundary.next_before_id.as_deref().is_some_and(|cursor| {
                !rows(snapshot, "messages")
                    .any(|row| task_id(row) == id && row["id"].as_str() == Some(cursor))
            })
        {
            return None;
        }
        Some(boundary)
    }
    pub fn task_boundary_index<'a>(
        &'a self,
        id: &str,
        revision: Option<&str>,
        snapshot: &Value,
        index: &PublicationIndex,
    ) -> Option<&'a PageBoundary> {
        let boundary = self.task_page_boundaries.get(id)?;
        if !boundary.after_reset(revision, self.reset_generation)
            || !rows(snapshot, "tasks").any(|task| task["id"].as_str() == Some(id))
            || boundary.next_before_id.as_deref().is_some_and(|cursor| {
                index
                    .messages
                    .get_by_id(cursor)
                    .ok()
                    .flatten()
                    .is_none_or(|row| row.task_id != id)
            })
        {
            return None;
        }
        Some(boundary)
    }
    pub fn channel_boundary<'a>(
        &'a self,
        id: &str,
        revision: Option<&str>,
        snapshot: &Value,
    ) -> Option<&'a PageBoundary> {
        let boundary = self.channel_page_boundaries.get(id)?;
        let reset = self
            .reset_generation
            .max(self.channel_reset_generations.get(id).copied());
        let channel = rows(snapshot, "channels").find(|row| row["id"].as_str() == Some(id))?;
        if !boundary.after_reset(revision, reset)
            || boundary.next_before_id.as_deref().is_some_and(|cursor| {
                !rows(channel, "messages").any(|row| row["id"].as_str() == Some(cursor))
            })
        {
            return None;
        }
        Some(boundary)
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageBoundary {
    pub revision: String,
    pub next_before_id: Option<String>,
    generation: u64,
}
impl PageBoundary {
    pub fn new(revision: String, next_before_id: Option<String>) -> Self {
        Self {
            revision,
            next_before_id,
            generation: 0,
        }
    }
    fn after_reset(&self, revision: Option<&str>, reset: Option<u64>) -> bool {
        revision == Some(self.revision.as_str()) && self.generation > reset.unwrap_or(0)
    }
}
/// At most one preparation runs and one set of changes waits. Invalidation
/// includes cache mutations at the same backend revision (history and eviction).
#[derive(Default)]
pub struct PublicationQueue {
    pub generation: u64,
    pub busy: bool,
    pending: Option<PublicationChanges>,
    failed: bool,
}
impl PublicationQueue {
    pub fn enqueue(&mut self, mut changes: PublicationChanges) {
        self.generation = self.generation.wrapping_add(1);
        self.failed = false;
        if changes.reset && changes.reset_generation.is_none() {
            changes.reset_generation = Some(self.generation);
        }
        for id in &changes.reset_channels {
            changes
                .channel_reset_generations
                .entry(id.clone())
                .or_insert(self.generation);
        }
        for boundary in changes
            .task_page_boundaries
            .values_mut()
            .chain(changes.channel_page_boundaries.values_mut())
        {
            if boundary.generation == 0 {
                boundary.generation = self.generation;
            }
        }
        self.pending
            .get_or_insert_with(Default::default)
            .merge(changes);
    }
    pub fn start(&mut self) -> Option<(u64, PublicationChanges)> {
        if self.busy || self.failed {
            return None;
        }
        let changes = self.pending.take()?;
        self.busy = true;
        Some((self.generation, changes))
    }
    pub fn finish(&mut self, generation: u64, changes: PublicationChanges) -> bool {
        self.busy = false;
        if generation == self.generation {
            return true;
        }
        // The rejected result's removals/reset scopes must survive coalescing.
        self.pending
            .get_or_insert_with(Default::default)
            .merge(changes);
        false
    }
    /// Keep a failed publication dirty without starting another worker. A new
    /// change or an explicit successful refresh may resume it exactly once.
    pub fn fail(&mut self, changes: PublicationChanges) {
        self.pending
            .get_or_insert_with(Default::default)
            .merge(changes);
        self.failed = true;
    }
    pub fn resume_failed(&mut self) -> bool {
        if !self.failed {
            return false;
        }
        self.failed = false;
        true
    }
}
/// Retry only a local cache race, once. A newer backend revision is a semantic
/// stale-page boundary and may never be replayed into the replacement state.
pub fn rebase_history_once(
    requested_generation: u64,
    current_generation: u64,
    page_revision: Option<&str>,
    current_revision: Option<&str>,
    attempt: u8,
) -> bool {
    requested_generation != current_generation
        && attempt == 0
        && page_revision.is_some()
        && page_revision == current_revision
}
#[derive(Default)]
pub struct PublicationIndex {
    pub messages: History<Message>,
    message_context: HashMap<String, Value>,
    metadata: Value,
    /// Build queue and opt-in state are footer inputs scoped to a project.
    /// Keeping them out of global metadata prevents one project's queue from
    /// invalidating every open chat pane.
    build_management: HashMap<String, Value>,
    task_projects: HashMap<String, String>,
    task_activity: HashMap<String, Value>,
    task_statuses: Arc<HashMap<String, String>>,
    events: HashMap<String, Vec<Value>>,
}
impl PublicationIndex {
    pub fn picker_inputs_unchanged(&self, previous: &Self) -> bool {
        ["tasks", "agents", "projects"]
            .into_iter()
            .all(|key| self.metadata[key] == previous.metadata[key])
            && self.task_activity == previous.task_activity
    }
    pub fn task_history(&self, id: &str) -> Vec<Value> {
        self.messages
            .scope_indices(id)
            .iter()
            .filter_map(|index| {
                self.messages
                    .get(*index)
                    .map(|row| serde_json::to_value(row).expect("message serializes"))
            })
            .collect()
    }

    pub fn task_status(&self, id: &str) -> Option<&str> {
        self.task_statuses.get(id).map(String::as_str)
    }
    /// Tiny worker-prepared projection; reuse identifies body-only publications.
    pub fn task_statuses(&self) -> Arc<HashMap<String, String>> {
        self.task_statuses.clone()
    }
}
pub type PreparedChildTranscripts = Arc<HashMap<String, Arc<Vec<Value>>>>;
pub struct PreparedPublication {
    pub snapshot: Value,
    pub hidden_tasks: HashSet<String>,
    pub index: Arc<PublicationIndex>,
    pub sync_tasks: HashSet<String>,
    /// Own message changes still require row/body validation by the pane. This
    /// only proves its non-message transcript context is unchanged, allowing
    /// source identity to advance without invalidating retained preparation.
    pub transcript_context_unchanged_tasks: HashSet<String>,
    pub histories: HashMap<String, Vec<Value>>,
    pub message_updates: HashMap<String, Vec<Value>>,
    pub body_only_tasks: HashSet<String>,
    pub lazy_history_tasks: HashSet<String>,
    pub collaboration_records: HashMap<String, Arc<Vec<Value>>>,
    pub collaboration_transcripts: HashMap<String, PreparedChildTranscripts>,
}
fn rows<'a>(snapshot: &'a Value, key: &str) -> impl Iterator<Item = &'a Value> {
    snapshot[key].as_array().into_iter().flatten()
}
/// Transcript preparation does not read top-level channels/projects metadata.
/// Compare all other borrowed entries conservatively, without cloning the tree.
fn transcript_metadata_equal(previous: &Value, next: &Value) -> bool {
    let (Some(previous), Some(next)) = (previous.as_object(), next.as_object()) else {
        return previous == next;
    };
    let relevant_len = |object: &serde_json::Map<String, Value>| {
        object
            .keys()
            .filter(|key| !matches!(key.as_str(), "channels" | "projects"))
            .count()
    };
    relevant_len(previous) == relevant_len(next)
        && previous
            .iter()
            .filter(|(key, _)| !matches!(key.as_str(), "channels" | "projects"))
            .all(|(key, value)| next.get(key) == Some(value))
}
fn task_id(row: &Value) -> &str {
    row["taskId"].as_str().unwrap_or_default()
}
/// Conservative metadata dependencies retain correctness for cross-task fork
/// discovery, palettes, tracked labels, approvals, and nested collaborations.
/// Histories/events are scoped, avoiding unrelated transcript reconciliation.
pub fn prepare(
    snapshot: Snapshot,
    panes: HashSet<String>,
    changes: &PublicationChanges,
    previous: Option<Arc<PublicationIndex>>,
) -> Result<PreparedPublication, String> {
    let visible = panes.clone();
    prepare_visible(snapshot, panes, visible, changes, previous)
}
/// Hidden panes retain source/revision proofs without preparing their full JSON
/// history. The caller refreshes marked scopes from the current typed index on
/// reveal, so coalesced resets, removals and page merges cannot resurrect rows.
pub fn prepare_visible(
    mut snapshot: Snapshot,
    panes: HashSet<String>,
    visible: HashSet<String>,
    changes: &PublicationChanges,
    previous: Option<Arc<PublicationIndex>>,
) -> Result<PreparedPublication, String> {
    let messages = std::mem::take(&mut snapshot.messages);
    let mut message_updates: HashMap<String, Vec<Value>> = HashMap::new();
    let mut body_only_tasks = changes.message_tasks.clone();
    if changes.reset
        || !changes.removed_message_ids.is_empty()
        || !changes.task_page_boundaries.is_empty()
    {
        body_only_tasks.clear();
    }
    // A hidden pane's trim intent must not disable an unrelated active stream.
    // Its own scope remains conservative even if no rows needed eviction.
    body_only_tasks.retain(|id| !changes.trim_tasks.contains(id));
    for id in &changes.message_ids {
        let Some(next) = messages.get_by_id(id).ok().flatten() else {
            continue;
        };
        let safe = previous
            .as_ref()
            .and_then(|index| index.messages.get_by_id(id).ok().flatten())
            .is_some_and(|old| {
                // Compare all non-body fields without cloning historical text.
                old.id == next.id
                    && old.task_id == next.task_id
                    && old.created_at == next.created_at
                    && old.role == "assistant"
                    && next.role == "assistant"
                    && old.stream_status.as_deref() == Some("streaming")
                    && old.stream_status == next.stream_status
                    && old.phase == next.phase
                    && old.response_metadata == next.response_metadata
                    && old.sender_agent_id == next.sender_agent_id
                    && old.collaboration_id == next.collaboration_id
                    && old.attachments == next.attachments
                    && !old.text.trim().is_empty()
                    && !next.text.trim().is_empty()
            });
        if !safe {
            body_only_tasks.remove(&next.task_id);
        }
        message_updates
            .entry(next.task_id.clone())
            .or_default()
            .push(serde_json::to_value(next).map_err(|e| e.to_string())?);
    }
    body_only_tasks.retain(|id| message_updates.contains_key(id));
    let mut value = serde_json::to_value(snapshot).map_err(|error| error.to_string())?;
    let mut message_context = previous
        .as_ref()
        .filter(|_| !changes.reset)
        .map(|index| index.message_context.clone())
        .unwrap_or_default();
    let context_tasks: HashSet<String> = if previous.is_none() || changes.reset {
        messages.iter().map(|row| row.task_id.clone()).collect()
    } else {
        changes
            .message_tasks
            .iter()
            .filter(|id| !body_only_tasks.contains(*id))
            .cloned()
            .collect()
    };
    for id in context_tasks {
        let mut user = None;
        let mut cleared = None;
        let mut reported_model: Option<(i64, String)> = None;
        for index in messages.scope_indices(&id).iter() {
            if let Some(row) = messages.get(*index) {
                if row.role == "assistant" {
                    if let Some(response) = row
                        .response_metadata
                        .as_ref()
                        .filter(|response| !response.model.trim().is_empty())
                    {
                        if reported_model
                            .as_ref()
                            .is_none_or(|(time, _)| *time <= row.created_at)
                        {
                            reported_model = Some((row.created_at, response.model.clone()));
                        }
                    }
                }
                if row.role == "user" {
                    user = Some(user.map_or(row.created_at, |old: i64| old.max(row.created_at)));
                }
                if row.role == "system" && row.text == "Context Cleared" {
                    cleared =
                        Some(cleared.map_or(row.created_at, |old: i64| old.max(row.created_at)));
                }
            }
        }
        message_context.insert(
            id,
            serde_json::json!({"latestUserAt": user, "contextClearedAt": cleared, "latestReportedModel": reported_model.map(|(_, model)| model)}),
        );
    }
    for id in &changes.removed_tasks {
        message_context.remove(id);
    }
    value["nativeMessageContext"] =
        serde_json::to_value(&message_context).map_err(|e| e.to_string())?;
    let internal_agents: HashSet<_> = rows(&value, "agents")
        .filter(|row| row["internal"].as_bool() == Some(true))
        .filter_map(|row| row["id"].as_str())
        .map(str::to_owned)
        .collect();
    let hidden_tasks = rows(&value, "tasks")
        .filter(|row| {
            row["agentId"]
                .as_str()
                .is_some_and(|id| internal_agents.contains(id))
        })
        .filter_map(|row| row["id"].as_str())
        .map(str::to_owned)
        .collect();
    if let Some(tasks) = value["tasks"].as_array_mut() {
        tasks.retain(|row| {
            !row["agentId"]
                .as_str()
                .is_some_and(|id| internal_agents.contains(id))
        });
    }
    let mut metadata = Value::Object(
        value
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(key, _)| {
                !matches!(
                    key.as_str(),
                    "messages" | "events" | "buildRequests" | "buildRuns" | "nativeMessageContext"
                )
            })
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
    );
    // Message payloads are independently journaled; channel transcript bodies
    // must not invalidate every task's metadata context.
    if let Some(channels) = metadata["channels"].as_array_mut() {
        for channel in channels {
            if let Some(object) = channel.as_object_mut() {
                object.remove("messages");
            }
        }
    }
    let mut build_management: HashMap<String, Value> = HashMap::new();
    if let Some(projects) = metadata["projects"].as_array_mut() {
        for project in projects {
            if let Some(object) = project.as_object_mut() {
                let id = object.get("id").and_then(Value::as_str).map(str::to_owned);
                let enabled = object
                    .remove("buildManagementEnabled")
                    .unwrap_or(Value::Null);
                if let Some(id) = id {
                    build_management.entry(id).or_default()["enabled"] = enabled;
                }
            }
        }
    }
    for key in ["buildRequests", "buildRuns"] {
        for row in rows(&value, key) {
            if let Some(project_id) = row["projectId"].as_str() {
                let project = build_management.entry(project_id.to_owned()).or_default();
                if !project[key].is_array() {
                    project[key] = Value::Array(Vec::new());
                }
                project[key]
                    .as_array_mut()
                    .expect("initialized as array")
                    .push(row.clone());
            }
        }
    }
    let task_projects: HashMap<String, String> = rows(&value, "tasks")
        .filter_map(|task| {
            Some((
                task["id"].as_str()?.to_owned(),
                task["projectId"].as_str()?.to_owned(),
            ))
        })
        .collect();
    // Stream flushes advance Task.updatedAt about every 100 ms. That marker
    // affects the task's footer and its collaboration ancestors, but cannot
    // invalidate unrelated panes. Keep every other metadata field conservative.
    let mut task_activity = HashMap::new();
    let mut task_statuses = HashMap::new();
    if let Some(tasks) = metadata["tasks"].as_array_mut() {
        for task in tasks {
            if let Some(task) = task.as_object_mut() {
                if let Some(id) = task.get("id").and_then(Value::as_str).map(str::to_owned) {
                    if let Some(status) = task.get("status").and_then(Value::as_str) {
                        task_statuses.insert(id.clone(), status.to_owned());
                    }
                    if let Some(updated_at) = task.remove("updatedAt") {
                        task_activity.insert(id, updated_at);
                    }
                }
            }
        }
    }
    let task_statuses = previous
        .as_ref()
        .filter(|index| index.task_statuses.as_ref() == &task_statuses)
        .map(|index| index.task_statuses.clone())
        .unwrap_or_else(|| Arc::new(task_statuses));
    let dependencies: HashMap<String, HashSet<String>> = panes
        .iter()
        .map(|id| {
            let mut tasks = HashSet::from([id.clone()]);
            loop {
                let next: Vec<_> = rows(&value, "collaborations")
                    .filter(|row| {
                        row["fromTaskId"]
                            .as_str()
                            .is_some_and(|parent| tasks.contains(parent))
                    })
                    .filter_map(|row| row["toTaskId"].as_str())
                    .filter(|child| !child.is_empty() && !tasks.contains(*child))
                    .map(str::to_owned)
                    .collect();
                if next.is_empty() {
                    break;
                }
                tasks.extend(next);
            }
            (id.clone(), tasks)
        })
        .collect();
    let relevant_tasks: HashSet<_> = dependencies.values().flatten().cloned().collect();
    let mut events: HashMap<String, Vec<Value>> = HashMap::new();
    for row in rows(&value, "events") {
        if relevant_tasks.contains(task_id(row)) {
            events
                .entry(task_id(row).to_owned())
                .or_default()
                .push(row.clone());
        }
    }
    let metadata_changed = previous.as_ref().is_none_or(|old| old.metadata != metadata);
    // Full pane sync still observes channels/projects. If transcript
    // preparation gains dependencies on either, include them in this check.
    let transcript_metadata_changed = metadata_changed
        && previous
        .as_ref()
        .is_none_or(|old| !transcript_metadata_equal(&old.metadata, &metadata));
    let settings_changed = previous
        .as_ref()
        .is_none_or(|old| old.metadata["settings"] != metadata["settings"]);
    let mut sync_tasks = HashSet::new();
    let mut transcript_context_unchanged_tasks = HashSet::new();
    let mut history_tasks = HashSet::new();
    for id in &panes {
        let events_changed = previous
            .as_ref()
            .is_none_or(|old| old.events.get(id) != events.get(id));
        let messages_changed = changes.message_tasks.contains(id);
        let build_management_changed = previous.as_ref().is_some_and(|old| {
            let project_id = task_projects.get(id).or_else(|| old.task_projects.get(id));
            project_id.is_some_and(|project_id| {
                old.build_management.get(project_id) != build_management.get(project_id)
            })
        });
        let activity_changed = previous.as_ref().is_none_or(|old| {
            dependencies[id]
                .iter()
                .any(|task| old.task_activity.get(task) != task_activity.get(task))
        });
        // Nested collaboration histories affect ancestor sidebars, even when
        // no child pane is open. The cycle-safe closure retains those inputs.
        let child_changed = dependencies[id].iter().any(|child| {
            child != id
                && (changes.message_tasks.contains(child)
                    || previous
                        .as_ref()
                        .is_none_or(|old| old.events.get(child) != events.get(child)))
        });
        let descendant_activity_changed = previous.as_ref().is_none_or(|old| {
            dependencies[id].iter().any(|child| {
                child != id && old.task_activity.get(child) != task_activity.get(child)
            })
        });
        if !changes.reset
            && !transcript_metadata_changed
            && !events_changed
            && !child_changed
            && !descendant_activity_changed
        {
            // The owner's updatedAt drives its footer, not transcript rows.
            // Descendant timestamps drive retained collaboration records and
            // therefore remain a conservative context invalidation.
            transcript_context_unchanged_tasks.insert(id.clone());
        }
        if changes.reset
            || metadata_changed
            || activity_changed
            || events_changed
            || messages_changed
            || child_changed
            || build_management_changed
        {
            sync_tasks.insert(id.clone());
        }
        if changes.reset
            || settings_changed
            || events_changed
            || (messages_changed && !body_only_tasks.contains(id))
        {
            history_tasks.insert(id.clone());
        }
    }
    let lazy_history_tasks = history_tasks
        .iter()
        .chain(changes.message_tasks.iter())
        .filter(|id| panes.contains(*id) && !visible.contains(*id))
        .cloned()
        .collect();
    history_tasks.retain(|id| visible.contains(id));
    let mut histories: HashMap<String, Vec<Value>> = history_tasks
        .iter()
        .map(|id| (id.clone(), Vec::new()))
        .collect();
    for (id, history) in &mut histories {
        for index in messages.scope_indices(id).iter() {
            if let Some(row) = messages.get(*index) {
                history.push(serde_json::to_value(row).map_err(|e| e.to_string())?);
            }
        }
    }
    Ok(PreparedPublication {
        snapshot: value,
        message_updates,
        body_only_tasks,
        lazy_history_tasks,
        hidden_tasks,
        index: Arc::new(PublicationIndex {
            messages,
            message_context,
            metadata,
            build_management,
            task_projects,
            task_activity,
            task_statuses,
            events,
        }),
        sync_tasks,
        transcript_context_unchanged_tasks,
        histories,
        collaboration_records: HashMap::new(),
        collaboration_transcripts: HashMap::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use monitter_lib::model::{Message, default_snapshot};
    fn message(id: &str, task: &str) -> Message {
        serde_json::from_value(serde_json::json!({"id":id,"taskId":task,"role":"assistant","text":"body","createdAt":0})).unwrap()
    }
    #[test]
    fn hidden_trim_preserves_unrelated_sparse_body_publication() {
        let mut snapshot = default_snapshot();
        let mut stream = message("active-stream", "active");
        stream.stream_status = Some("streaming".into());
        snapshot.messages = vec![
            stream,
            message("old-hidden", "hidden"),
            message("latest-hidden", "hidden"),
        ]
        .into();
        let panes = HashSet::from(["active".into(), "hidden".into()]);
        let initial = prepare(
            snapshot.clone(),
            panes.clone(),
            &PublicationChanges {
                reset: true,
                ..Default::default()
            },
            None,
        )
        .unwrap();
        snapshot
            .messages
            .update_by_id("active-stream", |row| row.text.push_str(" update"))
            .unwrap();
        // Model an actual hidden eviction as well as its trim intent.
        snapshot.messages.retain(|row| row.id != "old-hidden");
        let next = prepare(
            snapshot,
            panes,
            &PublicationChanges {
                message_tasks: HashSet::from(["active".into(), "hidden".into()]),
                message_ids: HashSet::from(["active-stream".into()]),
                trim_tasks: HashSet::from(["hidden".into()]),
                ..Default::default()
            },
            Some(initial.index),
        )
        .unwrap();
        assert!(next.body_only_tasks.contains("active"));
        assert!(!next.body_only_tasks.contains("hidden"));
        assert!(!next.histories.contains_key("active"));
        assert_eq!(next.histories["hidden"].len(), 1);
        assert_eq!(next.message_updates["active"].len(), 1);
    }
    #[test]
    fn hidden_history_is_materialized_only_from_latest_index_on_reveal() {
        let mut snapshot = default_snapshot();
        snapshot.messages = vec![message("old", "a"), message("visible", "b")].into();
        let panes = HashSet::from(["a".into(), "b".into()]);
        let visible = HashSet::from(["b".into()]);
        let first = prepare_visible(
            snapshot.clone(),
            panes.clone(),
            visible.clone(),
            &PublicationChanges {
                reset: true,
                ..Default::default()
            },
            None,
        )
        .unwrap();
        assert!(!first.histories.contains_key("a"));
        assert!(first.lazy_history_tasks.contains("a"));
        assert_eq!(first.histories["b"].len(), 1);
        snapshot.messages.retain(|row| row.id != "old");
        snapshot.messages.push(message("latest", "a"));
        let next = prepare_visible(
            snapshot,
            panes,
            visible,
            &PublicationChanges {
                message_tasks: HashSet::from(["a".into()]),
                message_ids: HashSet::from(["latest".into()]),
                removed_message_ids: HashSet::from(["old".into()]),
                ..Default::default()
            },
            Some(first.index),
        )
        .unwrap();
        assert!(!next.histories.contains_key("a"));
        assert!(next.lazy_history_tasks.contains("a"));
        assert_eq!(next.index.task_history("a")[0]["id"], "latest");
    }
    #[test]
    fn indexed_stream_publication_serializes_only_changed_body() {
        let mut snapshot = default_snapshot();
        snapshot.messages = (0..1000)
            .map(|n| {
                let mut row = message(&format!("m{n}"), "a");
                row.stream_status = Some("streaming".into());
                row
            })
            .collect();
        let panes = HashSet::from(["a".into()]);
        let initial = prepare(
            snapshot.clone(),
            panes.clone(),
            &PublicationChanges {
                reset: true,
                ..Default::default()
            },
            None,
        )
        .unwrap();
        snapshot
            .messages
            .update_by_id("m500", |row| row.text.push_str(" new"))
            .unwrap();
        let next = prepare(
            snapshot,
            panes,
            &PublicationChanges {
                message_tasks: HashSet::from(["a".into()]),
                message_ids: HashSet::from(["m500".into()]),
                ..Default::default()
            },
            Some(initial.index),
        )
        .unwrap();
        assert!(next.snapshot["messages"].as_array().unwrap().is_empty());
        assert!(next.histories.is_empty());
        assert_eq!(next.message_updates["a"].len(), 1);
        assert!(next.body_only_tasks.contains("a"));
        assert_eq!(next.index.task_history("a").len(), 1000);
    }
    #[test]
    fn task_status_projection_reuses_body_only_index_and_tracks_success_and_removal() {
        let mut snapshot = default_snapshot();
        snapshot.tasks = vec![
            serde_json::from_value(serde_json::json!({
                "id":"task","agentId":"a","title":"task","status":"running",
                "createdAt":0,"updatedAt":0,"hostId":"h","cwd":"",
                "provider":"codex","model":"default","sandbox":"read-only"
            }))
            .unwrap(),
        ];
        let first = prepare(
            snapshot.clone(),
            HashSet::new(),
            &PublicationChanges::default(),
            None,
        )
        .unwrap();
        assert_eq!(first.index.task_status("task"), Some("running"));
        snapshot.tasks[0].updated_at = 100;
        let streamed = prepare(
            snapshot.clone(),
            HashSet::new(),
            &PublicationChanges::default(),
            Some(first.index.clone()),
        )
        .unwrap();
        assert!(Arc::ptr_eq(
            &first.index.task_statuses(),
            &streamed.index.task_statuses()
        ));
        let mut task = serde_json::to_value(&snapshot.tasks[0]).unwrap();
        task["status"] = serde_json::json!("completed");
        snapshot.tasks[0] = serde_json::from_value(task).unwrap();
        let completed = prepare(
            snapshot.clone(),
            HashSet::new(),
            &PublicationChanges::default(),
            Some(streamed.index.clone()),
        )
        .unwrap();
        assert_eq!(completed.index.task_status("task"), Some("completed"));
        assert!(!Arc::ptr_eq(
            &streamed.index.task_statuses(),
            &completed.index.task_statuses()
        ));
        snapshot.tasks.clear();
        let removed = prepare(
            snapshot,
            HashSet::new(),
            &PublicationChanges::default(),
            Some(completed.index),
        )
        .unwrap();
        assert_eq!(removed.index.task_status("task"), None);
    }
    #[test]
    fn newer_same_revision_history_invalidates_preparation_and_retains_reset() {
        let mut queue = PublicationQueue::default();
        queue.enqueue(PublicationChanges {
            reset: true,
            removed_tasks: HashSet::from(["deleted".into()]),
            ..Default::default()
        });
        let (old, changes) = queue.start().unwrap();
        queue.enqueue(PublicationChanges {
            message_tasks: HashSet::from(["history".into()]),
            ..Default::default()
        });
        assert!(queue.start().is_none());
        assert!(!queue.finish(old, changes));
        let (latest, changes) = queue.start().unwrap();
        assert!(changes.reset);
        assert!(changes.removed_tasks.contains("deleted"));
        assert!(changes.message_tasks.contains("history"));
        assert!(queue.finish(latest, changes));
        assert!(queue.start().is_none());
    }
    #[test]
    fn unrelated_history_does_not_refresh_inactive_pane() {
        let mut snapshot = default_snapshot();
        snapshot.messages = vec![message("a1", "a"), message("b1", "b")].into();
        let panes = HashSet::from(["a".into(), "b".into()]);
        let initial = prepare(
            snapshot.clone(),
            panes.clone(),
            &PublicationChanges {
                reset: true,
                ..Default::default()
            },
            None,
        )
        .unwrap();
        snapshot.messages.push(message("a2", "a"));
        let next = prepare(
            snapshot,
            panes,
            &PublicationChanges {
                message_tasks: HashSet::from(["a".into()]),
                ..Default::default()
            },
            Some(initial.index),
        )
        .unwrap();
        assert_eq!(next.sync_tasks, HashSet::from(["a".into()]));
        assert_eq!(
            next.histories.keys().cloned().collect::<HashSet<_>>(),
            HashSet::from(["a".into()])
        );
        assert_eq!(next.histories["a"].len(), 2);
    }
    #[test]
    fn build_queue_and_project_opt_in_refresh_only_panes_in_that_project() {
        let mut snapshot = default_snapshot();
        let mut value = serde_json::to_value(&snapshot).unwrap();
        value["tasks"] = serde_json::json!([
            {"id":"a","agentId":"agent","projectId":"p1","title":"a","status":"idle","createdAt":0,"updatedAt":0,"hostId":"host","cwd":"/p1","provider":"codex","model":"default","sandbox":"read-only"},
            {"id":"b","agentId":"agent","projectId":"p2","title":"b","status":"idle","createdAt":0,"updatedAt":0,"hostId":"host","cwd":"/p2","provider":"codex","model":"default","sandbox":"read-only"}
        ]);
        value["projects"] = serde_json::json!([
            {"id":"p1","name":"Project 1","description":"","buildManagementEnabled":true},
            {"id":"p2","name":"Project 2","description":"","buildManagementEnabled":false}
        ]);
        value["buildRequests"] = serde_json::json!([]);
        value["buildRuns"] = serde_json::json!([]);
        snapshot = serde_json::from_value(value).unwrap();
        let panes = HashSet::from(["a".into(), "b".into()]);
        let initial = prepare(
            snapshot.clone(),
            panes.clone(),
            &PublicationChanges::default(),
            None,
        )
        .unwrap();

        let mut value = serde_json::to_value(&snapshot).unwrap();
        value["buildRequests"] = serde_json::json!([
            {"id":"request-1","projectId":"p1","taskId":"a","agentId":"agent","hostId":"host","requestId":"r1","title":"Build","command":"cargo build","cwd":"/p1","targetKey":"debug","status":"ready","buildRunId":null,"createdAt":1,"updatedAt":1}
        ]);
        let queued: Snapshot = serde_json::from_value(value).unwrap();
        let next = prepare(
            queued,
            panes.clone(),
            &PublicationChanges::default(),
            Some(initial.index),
        )
        .unwrap();
        assert_eq!(next.sync_tasks, HashSet::from(["a".into()]));
        assert_eq!(next.transcript_context_unchanged_tasks, panes);

        let mut value = serde_json::to_value(&snapshot).unwrap();
        value["projects"][0]["buildManagementEnabled"] = serde_json::json!(false);
        let opted_out: Snapshot = serde_json::from_value(value).unwrap();
        let toggled = prepare(
            opted_out,
            panes,
            &PublicationChanges::default(),
            Some(next.index),
        )
        .unwrap();
        assert_eq!(toggled.sync_tasks, HashSet::from(["a".into()]));
    }
    #[test]
    fn streaming_task_activity_refreshes_its_pane_and_ancestors_only() {
        let mut snapshot = default_snapshot();
        snapshot.tasks = ["parent", "child", "unrelated"]
            .into_iter()
            .map(|id| {
                serde_json::from_value(serde_json::json!({
                    "id":id,"agentId":"a","title":id,"status":"running",
                    "createdAt":0,"updatedAt":0,"hostId":"h","cwd":"",
                    "provider":"codex","model":"default","sandbox":"read-only"
                }))
                .unwrap()
            })
            .collect();
        snapshot.collaborations = serde_json::from_value(serde_json::json!([
            {"id":"c1","kind":"delegation","fromAgentId":"a","fromTaskId":"parent","toTaskId":"child","toAgentId":"a","requestId":"r","text":"child","status":"running","createdAt":0,"updatedAt":0}
        ])).unwrap();
        let panes = HashSet::from(["parent".into(), "child".into(), "unrelated".into()]);
        let initial = prepare(
            snapshot.clone(),
            panes.clone(),
            &PublicationChanges::default(),
            None,
        )
        .unwrap();
        snapshot.tasks[1].updated_at = 100;
        let activity = prepare(
            snapshot.clone(),
            panes.clone(),
            &PublicationChanges::default(),
            Some(initial.index),
        )
        .unwrap();
        assert_eq!(
            activity.sync_tasks,
            HashSet::from(["parent".into(), "child".into()])
        );
        assert!(activity.histories.is_empty());
        assert_eq!(activity.snapshot["tasks"][1]["updatedAt"], 100);
        assert!(
            activity
                .transcript_context_unchanged_tasks
                .contains("child")
        );
        assert!(
            !activity
                .transcript_context_unchanged_tasks
                .contains("parent")
        );
        // Titles remain conservative because cross-task links can show them.
        snapshot.tasks[1].title = "renamed child".into();
        let renamed = prepare(
            snapshot,
            panes.clone(),
            &PublicationChanges::default(),
            Some(activity.index),
        )
        .unwrap();
        assert_eq!(renamed.sync_tasks, panes);
        assert!(renamed.transcript_context_unchanged_tasks.is_empty());
    }
    #[test]
    fn message_removal_reconciles_empty_authoritative_history() {
        let mut snapshot = default_snapshot();
        snapshot.messages = vec![message("gone", "a")].into();
        let panes = HashSet::from(["a".into()]);
        let initial = prepare(
            snapshot.clone(),
            panes.clone(),
            &PublicationChanges {
                reset: true,
                ..Default::default()
            },
            None,
        )
        .unwrap();
        snapshot.messages.clear();
        let next = prepare(
            snapshot,
            panes,
            &PublicationChanges {
                message_tasks: HashSet::from(["a".into()]),
                ..Default::default()
            },
            Some(initial.index),
        )
        .unwrap();
        assert_eq!(next.histories["a"].len(), 0);
        assert!(next.sync_tasks.contains("a"));
    }
    #[test]
    fn nested_collaboration_output_refreshes_ancestor_without_reconciling_its_history() {
        let mut snapshot = default_snapshot();
        snapshot.collaborations = serde_json::from_value(serde_json::json!([
            {"id":"c1","kind":"delegation","fromAgentId":"a","fromTaskId":"parent","toTaskId":"child","toAgentId":"a","requestId":"r1","text":"first","status":"running","createdAt":0,"updatedAt":0},
            {"id":"c2","kind":"delegation","fromAgentId":"a","fromTaskId":"child","toTaskId":"grandchild","toAgentId":"a","requestId":"r2","text":"next","status":"running","createdAt":0,"updatedAt":0},
            {"id":"c3","kind":"delegation","fromAgentId":"a","fromTaskId":"grandchild","toTaskId":"parent","toAgentId":"a","requestId":"r3","text":"cycle","status":"running","createdAt":0,"updatedAt":0}
        ])).unwrap();
        let panes = HashSet::from(["parent".into(), "unrelated".into()]);
        let first = prepare(
            snapshot.clone(),
            panes.clone(),
            &PublicationChanges {
                reset: true,
                ..Default::default()
            },
            None,
        )
        .unwrap();
        snapshot.messages.push(message("output", "grandchild"));
        let next = prepare(
            snapshot,
            panes,
            &PublicationChanges {
                message_tasks: HashSet::from(["grandchild".into()]),
                ..Default::default()
            },
            Some(first.index),
        )
        .unwrap();
        assert_eq!(next.sync_tasks, HashSet::from(["parent".into()]));
        assert!(next.histories.is_empty());
        assert!(!next.transcript_context_unchanged_tasks.contains("parent"));
    }
    #[test]
    fn own_message_changes_reuse_context_but_reset_metadata_and_events_do_not() {
        let mut snapshot = default_snapshot();
        snapshot.messages = vec![message("stream", "a")].into();
        let panes = HashSet::from(["a".into()]);
        let first = prepare(
            snapshot.clone(),
            panes.clone(),
            &PublicationChanges::default(),
            None,
        )
        .unwrap();
        assert!(first.transcript_context_unchanged_tasks.is_empty());
        snapshot
            .messages
            .update_by_id("stream", |message| message.text = "new body".into())
            .unwrap();
        let changes = PublicationChanges {
            message_tasks: panes.clone(),
            ..Default::default()
        };
        let streamed =
            prepare(snapshot.clone(), panes.clone(), &changes, Some(first.index)).unwrap();
        assert_eq!(streamed.transcript_context_unchanged_tasks, panes);
        assert_eq!(streamed.histories["a"][0]["text"], "new body");
        let reset = prepare(
            snapshot.clone(),
            panes.clone(),
            &PublicationChanges {
                reset: true,
                ..Default::default()
            },
            Some(streamed.index.clone()),
        )
        .unwrap();
        assert!(reset.transcript_context_unchanged_tasks.is_empty());
        let mut settings_changed = serde_json::to_value(snapshot.clone()).unwrap();
        settings_changed["settings"]["chatFontSize"] = serde_json::json!(27);
        let settings_changed = prepare(
            serde_json::from_value(settings_changed).unwrap(),
            panes.clone(),
            &changes,
            Some(streamed.index.clone()),
        )
        .unwrap();
        assert!(
            settings_changed
                .transcript_context_unchanged_tasks
                .is_empty()
        );
        snapshot.events = serde_json::from_value(serde_json::json!([{"id":"event","taskId":"a","kind":"status","title":"status changed","detail":"detail","createdAt":0}])).unwrap();
        let event_changed = prepare(snapshot, panes, &changes, Some(streamed.index)).unwrap();
        assert!(event_changed.transcript_context_unchanged_tasks.is_empty());
    }
    #[test]
    fn retiring_snapshot_keeps_json_until_consumers_rebind_without_cloning() {
        let retired = Rc::new(serde_json::json!({"messages":[{"text":"retained body"}]}));
        let weak = Rc::downgrade(&retired);
        let mut pane = retired.clone();
        let mut sidebar = retired.clone();
        let next = Rc::new(serde_json::json!({"messages":[]}));
        pane = next.clone();
        assert!(weak.upgrade().is_some());
        // A remaining sidebar owner prevents transfer. Its separately held
        // retirement reference protects the eventual final consumer release.
        assert!(take_unshared_snapshot(retired).is_none());
        let sidebar_retired = std::mem::replace(&mut sidebar, next);
        assert!(Rc::ptr_eq(&pane, &sidebar));
        let owned = take_unshared_snapshot(sidebar_retired).unwrap();
        assert!(weak.upgrade().is_none());
        assert_eq!(owned["messages"][0]["text"], "retained body");
        // `owned` now contains the original payload and is Send, so production
        // can destroy it on its background executor after this UI transfer.
        std::thread::spawn(move || drop(owned)).join().unwrap();
    }
    #[test]
    fn local_history_race_rebases_once_but_reset_revision_never_replays() {
        assert!(rebase_history_once(
            4,
            5,
            Some("epoch:7"),
            Some("epoch:7"),
            0
        ));
        assert!(!rebase_history_once(
            4,
            5,
            Some("epoch:7"),
            Some("epoch:7"),
            1
        ));
        assert!(!rebase_history_once(
            4,
            5,
            Some("epoch:7"),
            Some("epoch:8"),
            0
        ));
        assert!(!rebase_history_once(
            4,
            5,
            Some("epoch:7"),
            Some("reset:0"),
            0
        ));
        assert!(!rebase_history_once(
            4,
            4,
            Some("epoch:7"),
            Some("epoch:7"),
            0
        ));
    }
    #[test]
    fn rebased_history_candidate_preserves_another_tasks_same_revision_page() {
        use crate::native_state::{ApplyResult, NativeState};
        use monitter_lib::experimental_desktop::{TaskMessagesPage, UiDeltaResponse};
        let mut snapshot = default_snapshot();
        for id in ["a", "b"] {
            snapshot.tasks.push(serde_json::from_value(serde_json::json!({
                "id":id,"agentId":"agent","title":id,"status":"idle","createdAt":0,"updatedAt":0,
                "hostId":"host","cwd":"/tmp","provider":"codex","model":"test","sandbox":"read-only"
            })).unwrap());
        }
        let mut current = NativeState::default();
        assert!(matches!(
            current.apply(UiDeltaResponse {
                revision: "epoch:7".into(),
                snapshot: Some(snapshot),
                delta: None
            }),
            ApplyResult::FullReset(_)
        ));
        let a_page = TaskMessagesPage {
            revision: "epoch:7".into(),
            messages: vec![message("older-a", "a")],
            next_before_id: None,
        };
        let mut stale_candidate = current.clone();
        stale_candidate
            .merge_task_page("a", a_page.clone())
            .unwrap();
        current
            .merge_task_page(
                "b",
                TaskMessagesPage {
                    revision: "epoch:7".into(),
                    messages: vec![message("older-b", "b")],
                    next_before_id: None,
                },
            )
            .unwrap();
        assert!(rebase_history_once(
            3,
            4,
            Some(&a_page.revision),
            current.revision(),
            0
        ));
        // Retry starts from the latest cache; the first worker's candidate is
        // never installed and cannot erase B's already accepted older page.
        let mut rebased_candidate = current.clone();
        rebased_candidate.merge_task_page("a", a_page).unwrap();
        let ids: HashSet<_> = rebased_candidate
            .snapshot()
            .unwrap()
            .messages
            .iter()
            .map(|row| row.id.as_str())
            .collect();
        assert_eq!(ids, HashSet::from(["older-a", "older-b"]));
        assert!(
            !stale_candidate
                .snapshot()
                .unwrap()
                .messages
                .iter()
                .any(|row| row.id == "older-b")
        );
    }
    fn boundary_snapshot() -> Value {
        serde_json::json!({"tasks":[{"id":"t"}], "messages":[{"id":"older","taskId":"t"}], "channels":[{"id":"c","messages":[{"id":"channel-older"}]}]})
    }
    fn accepted_boundaries() -> PublicationChanges {
        PublicationChanges {
            message_tasks: HashSet::from(["t".into()]),
            task_page_boundaries: HashMap::from([(
                "t".into(),
                PageBoundary::new("epoch:7".into(), Some("older".into())),
            )]),
            channel_page_boundaries: HashMap::from([(
                "c".into(),
                PageBoundary::new("epoch:7".into(), None),
            )]),
            ..Default::default()
        }
    }
    #[test]
    fn page_after_pending_reset_keeps_more_and_complete_boundaries_at_current_revision() {
        let mut queue = PublicationQueue::default();
        queue.enqueue(PublicationChanges {
            reset: true,
            reset_channels: HashSet::from(["c".into()]),
            ..Default::default()
        });
        let (old_generation, reset) = queue.start().unwrap();
        queue.enqueue(accepted_boundaries());
        assert!(!queue.finish(old_generation, reset));
        let (_, changes) = queue.start().unwrap();
        let snapshot = boundary_snapshot();
        assert!(changes.reset);
        assert_eq!(
            changes
                .task_boundary("t", Some("epoch:7"), &snapshot)
                .unwrap()
                .next_before_id
                .as_deref(),
            Some("older")
        );
        assert_eq!(
            changes
                .channel_boundary("c", Some("epoch:7"), &snapshot)
                .unwrap()
                .next_before_id,
            None
        );
        // A new epoch can never reuse the accepted old-epoch boundary.
        assert!(
            changes
                .task_boundary("t", Some("reset:0"), &snapshot)
                .is_none()
        );
        assert!(
            changes
                .channel_boundary("c", Some("reset:0"), &snapshot)
                .is_none()
        );
    }
    #[test]
    fn reset_after_page_does_not_resurrect_boundary_even_at_same_backend_revision() {
        let mut queue = PublicationQueue::default();
        queue.enqueue(accepted_boundaries());
        let (old_generation, page) = queue.start().unwrap();
        queue.enqueue(PublicationChanges {
            reset: true,
            ..Default::default()
        });
        assert!(!queue.finish(old_generation, page));
        let (_, changes) = queue.start().unwrap();
        let snapshot = boundary_snapshot();
        assert!(
            changes
                .task_boundary("t", Some("epoch:7"), &snapshot)
                .is_none()
        );
        assert!(
            changes
                .channel_boundary("c", Some("epoch:7"), &snapshot)
                .is_none()
        );
    }
    #[test]
    fn scoped_channel_reset_and_deleted_cursor_invalidate_only_current_boundary_scope() {
        let mut queue = PublicationQueue::default();
        queue.enqueue(accepted_boundaries());
        queue.enqueue(PublicationChanges {
            reset_channels: HashSet::from(["c".into()]),
            ..Default::default()
        });
        let (_, changes) = queue.start().unwrap();
        let mut snapshot = boundary_snapshot();
        assert!(
            changes
                .channel_boundary("c", Some("epoch:7"), &snapshot)
                .is_none()
        );
        assert!(
            changes
                .task_boundary("t", Some("epoch:7"), &snapshot)
                .is_some()
        );
        snapshot["messages"] = serde_json::json!([]);
        assert!(
            changes
                .task_boundary("t", Some("epoch:7"), &snapshot)
                .is_none()
        );
        snapshot["tasks"] = serde_json::json!([]);
        assert!(
            changes
                .task_boundary("t", Some("epoch:7"), &snapshot)
                .is_none()
        );
    }
    #[test]
    fn coalesced_old_worker_cannot_replace_newest_accepted_boundary() {
        let mut queue = PublicationQueue::default();
        queue.enqueue(accepted_boundaries());
        let (old_generation, old_page) = queue.start().unwrap();
        let mut newest = accepted_boundaries();
        newest
            .task_page_boundaries
            .insert("t".into(), PageBoundary::new("epoch:7".into(), None));
        queue.enqueue(newest);
        assert!(!queue.finish(old_generation, old_page));
        let (_, changes) = queue.start().unwrap();
        assert_eq!(
            changes
                .task_boundary("t", Some("epoch:7"), &boundary_snapshot())
                .unwrap()
                .next_before_id,
            None
        );
    }
    #[test]
    fn failed_preparation_stays_dirty_until_unchanged_refresh_or_new_source_change() {
        use crate::native_state::{ApplyResult, NativeState};
        use monitter_lib::experimental_desktop::UiDeltaResponse;
        let mut native = NativeState::default();
        let ApplyResult::FullReset(changes) = native.apply(UiDeltaResponse {
            revision: "epoch:7".into(),
            snapshot: Some(default_snapshot()),
            delta: None,
        }) else {
            panic!("reset expected");
        };
        let mut queue = PublicationQueue::default();
        let mut reset_changes = PublicationChanges::from_changes(true, changes);
        reset_changes.removed_tasks.insert("deleted-task".into());
        reset_changes.reset_channels.insert("c".into());
        queue.enqueue(reset_changes);
        let (generation, changes) = queue.start().unwrap();
        assert!(queue.finish(generation, changes.clone()));
        queue.fail(changes);
        assert!(queue.start().is_none());
        assert!(queue.start().is_none()); // No completion-driven retry loop.
        assert_eq!(
            native.apply(UiDeltaResponse {
                revision: "epoch:7".into(),
                snapshot: None,
                delta: None
            }),
            ApplyResult::Unchanged
        );
        assert!(queue.resume_failed());
        assert!(!queue.resume_failed());
        let (retry_generation, retry) = queue.start().unwrap();
        assert_eq!(retry_generation, generation);
        assert!(retry.reset);
        assert_eq!(retry.reset_generation, Some(generation));
        assert!(retry.removed_tasks.contains("deleted-task"));
        assert!(retry.reset_channels.contains("c"));
        assert!(queue.finish(retry_generation, retry.clone()));
        queue.fail(retry);
        assert!(queue.start().is_none());
        // A genuinely new source change also resumes the same retained reset.
        queue.enqueue(PublicationChanges {
            message_tasks: HashSet::from(["new-output".into()]),
            ..Default::default()
        });
        let (new_generation, changed) = queue.start().unwrap();
        assert!(new_generation > generation);
        assert!(changed.reset);
        assert!(changed.message_tasks.contains("new-output"));
    }
}
