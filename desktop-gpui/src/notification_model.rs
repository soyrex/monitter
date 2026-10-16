//! Session-local unread state. No backend decisions, I/O, or preference writes.
//! A first authoritative snapshot is a baseline, not a historical unread inbox.
use crate::notifications::{NotificationEntry, NotificationKind, NotificationTone};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::{Hash, Hasher};

const MAX_NOTIFICATIONS: usize = 100;
const MAX_SEEN_PER_SCOPE: usize = 64;
const MAX_DETAIL_CHARS: usize = 16_384;
type MessageKey = (i64, String);

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or("")
}
fn rows<'a>(value: &'a Value, key: &str) -> impl Iterator<Item = &'a Value> {
    value[key].as_array().into_iter().flatten()
}
fn nonempty(value: &Value, key: &str) -> Option<String> {
    let value = text(value, key);
    (!value.is_empty()).then(|| value.to_owned())
}
fn retained_detail(value: &str) -> String {
    let mut chars = value.chars();
    let mut result = chars.by_ref().take(MAX_DETAIL_CHARS).collect::<String>();
    if chars.next().is_some() {
        result.push('…');
    }
    result
}
fn agent_name<'a>(snapshot: &'a Value, id: &str) -> Option<&'a str> {
    rows(snapshot, "agents")
        .find(|agent| text(agent, "id") == id)
        .map(|agent| text(agent, "name"))
        .filter(|name| !name.trim().is_empty())
}
fn task_origin(snapshot: &Value, task: &Value) -> String {
    let agent = agent_name(snapshot, text(task, "agentId"))
        .map(str::to_owned)
        .unwrap_or_else(|| {
            let provider = rows(snapshot, "agents")
                .find(|agent| text(agent, "id") == text(task, "agentId"))
                .map(|agent| text(agent, "provider"))
                .filter(|provider| !provider.is_empty())
                .unwrap_or("Agent");
            provider.to_owned()
        });
    let title = if text(task, "title").trim().is_empty() {
        "Chat"
    } else {
        text(task, "title")
    };
    format!("{agent} | {title}")
}
fn visible_tasks<'a>(snapshot: &'a Value) -> HashMap<String, &'a Value> {
    let internal: HashSet<_> = rows(snapshot, "agents")
        .filter(|agent| agent["internal"].as_bool() == Some(true))
        .map(|agent| text(agent, "id"))
        .collect();
    rows(snapshot, "tasks")
        .filter(|task| {
            !text(task, "id").is_empty()
                && task["archived"].as_bool() != Some(true)
                && text(task, "channelId").is_empty()
                && !internal.contains(text(task, "agentId"))
        })
        .map(|task| (text(task, "id").to_owned(), task))
        .collect()
}
fn message_key(message: &Value) -> Option<MessageKey> {
    let id = nonempty(message, "id")?;
    Some((message["createdAt"].as_i64().unwrap_or(0), id))
}
fn final_assistant(message: &Value, running: bool) -> bool {
    text(message, "role") == "assistant"
        && !text(message, "text").trim().is_empty()
        && matches!(text(message, "phase"), "" | "final_answer")
        && matches!(text(message, "streamStatus"), "" | "complete")
        && (!running
            || (text(message, "phase") == "final_answer"
                && text(message, "streamStatus") == "complete"))
}
#[derive(Clone, Copy)]
struct SeenMessage {
    finalized: bool,
    fingerprint: u64,
}
#[derive(Default)]
struct Cursor {
    status: String,
    watermark: MessageKey,
    baseline_time: i64,
    seen: BTreeMap<MessageKey, SeenMessage>,
}
impl Cursor {
    /// Track small recent message identities and a nonrewinding watermark.
    /// A known streaming/interim row becoming final is new unread evidence even
    /// if its timestamp predates the initial baseline.
    fn observe(
        &mut self,
        messages: &[&Value],
        baseline: bool,
        running: bool,
    ) -> Option<(String, String)> {
        let mut messages: Vec<_> = messages
            .iter()
            .filter_map(|message| message_key(message).map(|key| (key, *message)))
            .collect();
        messages.sort_by(|a, b| a.0.cmp(&b.0));
        let mut latest = None;
        for (key, message) in messages {
            let final_message = final_assistant(message, running);
            let previous = self.seen.get(&key).copied();
            if baseline {
                self.baseline_time = self.baseline_time.max(key.0);
            }
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            text(message, "text").hash(&mut hasher);
            let fingerprint = hasher.finish();
            let legacy =
                text(message, "phase").is_empty() && text(message, "streamStatus").is_empty();
            let changed = previous.is_some_and(|previous| previous.fingerprint != fingerprint);
            if !baseline
                && final_message
                && (previous.is_some_and(|previous| !previous.finalized || (legacy && changed))
                    || (previous.is_none() && key.0 > self.baseline_time && key > self.watermark))
            {
                latest = Some((
                    text(message, "id").to_owned(),
                    text(message, "text").to_owned(),
                ));
            }
            // Existing final history does not become partial merely because a
            // new turn starts. Actual legacy text changes during that turn do.
            let finalized = if baseline
                || (previous.is_none() && (key.0 <= self.baseline_time || key <= self.watermark))
            {
                final_assistant(message, false)
            } else if running && legacy && changed {
                false
            } else {
                final_message || previous.is_some_and(|previous| previous.finalized)
            };
            self.watermark = self.watermark.clone().max(key.clone());
            self.seen.insert(
                key,
                SeenMessage {
                    finalized,
                    fingerprint,
                },
            );
            while self.seen.len() > MAX_SEEN_PER_SCOPE {
                self.seen.pop_first();
            }
        }
        latest
    }
}
/// Worker-prepared scope selection. Message bodies are serialized only for
/// changed IDs, except a baseline/reset/status transition which needs history.
#[derive(Default)]
pub(crate) struct NotificationProjection {
    tasks: HashMap<String, String>,
    channels: HashSet<String>,
    internal_agents: HashSet<String>,
}
pub(crate) struct PreparedNotifications {
    pub projection: std::sync::Arc<NotificationProjection>,
    tasks: HashMap<String, Vec<Value>>,
    channels: HashSet<String>,
    baseline_message_time: Option<i64>,
}
pub(crate) fn prepare_notifications(
    snapshot: &Value,
    index: &monitter_gpui_desktop::native_publication::PublicationIndex,
    updates: &HashMap<String, Vec<Value>>,
    changes: &monitter_gpui_desktop::native_publication::PublicationChanges,
    previous: Option<&NotificationProjection>,
) -> PreparedNotifications {
    let tasks = visible_tasks(snapshot);
    let projection = NotificationProjection {
        tasks: tasks
            .iter()
            .map(|(id, task)| (id.clone(), text(task, "status").to_owned()))
            .collect(),
        channels: rows(snapshot, "channels")
            .map(|channel| text(channel, "id").to_owned())
            .collect(),
        internal_agents: rows(snapshot, "agents")
            .filter(|agent| agent["internal"] == true)
            .map(|agent| text(agent, "id").to_owned())
            .collect(),
    };
    let mut scoped = HashMap::new();
    for (id, task) in &tasks {
        let full = changes.reset
            || previous.is_none_or(|previous| {
                previous.tasks.get(id).map(String::as_str) != Some(text(task, "status"))
            })
            || changes.task_page_boundaries.contains_key(id)
            || (changes.message_tasks.contains(id) && changes.message_ids.is_empty());
        if full {
            scoped.insert(id.clone(), index.task_history(id));
        } else if let Some(messages) = updates.get(id) {
            scoped.insert(id.clone(), messages.clone());
        }
    }
    let all_channels = changes.reset
        || previous.is_none_or(|previous| previous.internal_agents != projection.internal_agents);
    let channels = projection
        .channels
        .iter()
        .filter(|id| {
            all_channels
                || changes.changed_channels.contains(*id)
                || changes.reset_channels.contains(*id)
                || changes.channel_page_boundaries.contains_key(*id)
                || previous.is_none_or(|previous| !previous.channels.contains(*id))
        })
        .cloned()
        .collect();
    // The initial global watermark includes invisible history too, matching
    // the former complete snapshot baseline without serializing those bodies.
    let baseline_message_time = previous.is_none().then(|| {
        index
            .messages
            .iter()
            .map(|message| message.created_at)
            .max()
            .unwrap_or(0)
    });
    PreparedNotifications {
        projection: std::sync::Arc::new(projection),
        tasks: scoped,
        channels,
        baseline_message_time,
    }
}
struct Alert {
    entry: NotificationEntry,
    sequence: u64,
}
#[derive(Default)]
pub struct NotificationModel {
    initialized: bool,
    baseline_time: i64,
    tasks: HashMap<String, Cursor>,
    channels: HashMap<String, Cursor>,
    summary_attempts: HashSet<String>,
    alerts: HashMap<String, Alert>,
    sequence: u64,
    #[cfg(test)]
    observed_messages: usize,
}
impl NotificationModel {
    fn put(&mut self, key: String, entry: NotificationEntry) {
        let mut entry = entry;
        entry.created_at = chrono::Utc::now().timestamp_millis();
        self.sequence = self.sequence.saturating_add(1);
        self.alerts.insert(
            key,
            Alert {
                entry,
                sequence: self.sequence,
            },
        );
        while self.alerts.len() > MAX_NOTIFICATIONS {
            let oldest = self
                .alerts
                .iter()
                .min_by_key(|(_, alert)| alert.sequence)
                .map(|(id, _)| id.clone());
            if let Some(id) = oldest {
                self.alerts.remove(&id);
            } else {
                break;
            }
        }
    }
    pub fn sync(&mut self, snapshot: &Value) {
        let mut tasks: HashMap<String, Vec<Value>> = visible_tasks(snapshot)
            .keys()
            .map(|id| (id.clone(), Vec::new()))
            .collect();
        for message in rows(snapshot, "messages") {
            if let Some(messages) = tasks.get_mut(text(message, "taskId")) {
                messages.push(message.clone());
            }
        }
        let prepared = PreparedNotifications {
            projection: Default::default(),
            tasks,
            baseline_message_time: rows(snapshot, "messages")
                .map(|message| message["createdAt"].as_i64().unwrap_or(0))
                .max(),
            channels: rows(snapshot, "channels")
                .map(|channel| text(channel, "id").to_owned())
                .collect(),
        };
        self.sync_prepared(snapshot, &prepared);
    }
    pub(crate) fn sync_prepared(&mut self, snapshot: &Value, prepared: &PreparedNotifications) {
        // Loading placeholders are not authoritative baselines.
        if !snapshot["tasks"].is_array() {
            return;
        }
        let first = !self.initialized;
        if first {
            self.baseline_time = rows(snapshot, "tasks")
                .map(|task| task["updatedAt"].as_i64().unwrap_or(0))
                .chain(prepared.baseline_message_time)
                .chain(
                    prepared
                        .tasks
                        .values()
                        .flatten()
                        .map(|message| message["createdAt"].as_i64().unwrap_or(0)),
                )
                .chain(rows(snapshot, "channels").flat_map(|channel| {
                    rows(channel, "messages")
                        .map(|message| message["createdAt"].as_i64().unwrap_or(0))
                }))
                .max()
                .unwrap_or(0);
            self.initialized = true;
        }
        let tasks = visible_tasks(snapshot);
        let channels: HashMap<_, _> = rows(snapshot, "channels")
            .filter(|channel| !text(channel, "id").is_empty())
            .map(|channel| (text(channel, "id").to_owned(), channel))
            .collect();
        self.tasks.retain(|id, _| tasks.contains_key(id));
        self.channels.retain(|id, _| channels.contains_key(id));
        self.alerts.retain(|_, alert| {
            if let Some(channel) = &alert.entry.channel_id {
                channels.contains_key(channel)
            } else {
                tasks.contains_key(&alert.entry.task_id)
            }
        });
        let mut ordered_tasks: Vec<_> = prepared
            .tasks
            .keys()
            .filter_map(|id| tasks.get_key_value(id))
            .collect();
        ordered_tasks
            .sort_by_key(|(id, task)| (task["updatedAt"].as_i64().unwrap_or(0), (*id).clone()));
        for (id, task) in ordered_tasks {
            let Some(messages) = prepared.tasks.get(id) else {
                continue;
            };
            let messages: Vec<_> = messages.iter().collect();
            #[cfg(test)]
            {
                self.observed_messages += messages.len();
            }
            let new_scope = !self.tasks.contains_key(id);
            let cursor = self.tasks.entry(id.clone()).or_insert_with(|| Cursor {
                baseline_time: task["updatedAt"].as_i64().unwrap_or(self.baseline_time),
                watermark: (
                    task["updatedAt"].as_i64().unwrap_or(self.baseline_time),
                    String::new(),
                ),
                ..Cursor::default()
            });
            let previous = cursor.status.clone();
            let status = text(task, "status").to_owned();
            cursor.status = status.clone();
            let latest = cursor.observe(&messages, first || new_scope, status == "running");
            let ended = !first
                && previous == "running"
                && matches!(status.as_str(), "completed" | "error" | "interrupted");
            let origin = task_origin(snapshot, task);
            let key = format!("task:{id}");
            let final_message = messages
                .iter()
                .copied()
                .filter(|message| final_assistant(message, status == "running"))
                .max_by_key(|message| message_key(message));
            if let Some((message_id, detail)) = latest {
                self.put(
                    key.clone(),
                    NotificationEntry {
                        id: key.clone(),
                        created_at: 0,
                        task_id: id.clone(),
                        channel_id: None,
                        origin: origin.clone(),
                        title: String::new(),
                        detail,
                        message_id: Some(message_id),
                        summary: None,
                        kind: NotificationKind::Message,
                        tone: NotificationTone::Info,
                    },
                );
            } else if ended && status == "completed" && !self.alerts.contains_key(&key) {
                self.put(
                    key.clone(),
                    NotificationEntry {
                        id: key.clone(),
                        created_at: 0,
                        task_id: id.clone(),
                        channel_id: None,
                        origin: origin.clone(),
                        title: "Completed".into(),
                        detail: final_message.map(|message| text(message, "text").to_owned()).unwrap_or_else(|| "No final agent message was received; open the chat to inspect its outcome.".into()),
                        message_id: final_message.map(|message| text(message, "id").to_owned()), summary: None,
                        kind: NotificationKind::Attention,
                        tone: NotificationTone::Success,
                    },
                );
            }
            // Error/interruption outranks a reply arriving in the same snapshot.
            if ended && status != "completed" {
                self.put(
                    key.clone(),
                    NotificationEntry {
                        id: key,
                        created_at: 0,
                        task_id: id.clone(),
                        channel_id: None,
                        origin,
                        title: if status == "error" {
                            "Chat failed"
                        } else {
                            "Chat interrupted"
                        }
                        .into(),
                        detail: if text(task, "error").is_empty() {
                            "Review the chat for details.".into()
                        } else {
                            retained_detail(text(task, "error"))
                        },
                        message_id: None,
                        summary: None,
                        kind: NotificationKind::Attention,
                        tone: if status == "error" {
                            NotificationTone::Danger
                        } else {
                            NotificationTone::Warning
                        },
                    },
                );
            }
        }
        let internal: HashSet<_> = rows(snapshot, "agents")
            .filter(|agent| agent["internal"].as_bool() == Some(true))
            .map(|agent| text(agent, "id"))
            .collect();
        for (id, channel) in channels {
            if !prepared.channels.contains(&id) {
                continue;
            }
            let new_scope = !self.channels.contains_key(&id);
            let cursor = self.channels.entry(id.clone()).or_insert_with(|| Cursor {
                baseline_time: self.baseline_time,
                watermark: (self.baseline_time, String::new()),
                ..Cursor::default()
            });
            let messages: Vec<_> = rows(channel, "messages")
                .filter(|message| !internal.contains(text(message, "agentId")))
                .collect();
            if let Some((_, detail)) = cursor.observe(&messages, first || new_scope, false) {
                let key = format!("channel:{id}");
                self.put(
                    key.clone(),
                    NotificationEntry {
                        id: key,
                        created_at: 0,
                        task_id: String::new(),
                        channel_id: Some(id),
                        origin: {
                            let channel_name = if text(channel, "name").trim().is_empty() {
                                "Channel"
                            } else {
                                text(channel, "name")
                            };
                            let agent = rows(channel, "agentIds")
                                .filter_map(Value::as_str)
                                .find_map(|id| agent_name(snapshot, id));
                            match agent {
                                Some(agent) => format!("{channel_name} | {agent}"),
                                None => channel_name.to_owned(),
                            }
                        },
                        title: String::new(),
                        detail,
                        message_id: None,
                        summary: None,
                        kind: NotificationKind::Message,
                        tone: NotificationTone::Info,
                    },
                );
            }
        }
    }
    pub fn entries(&self, snapshot: &Value) -> Vec<NotificationEntry> {
        let tasks = visible_tasks(snapshot);
        let channels: HashSet<_> = rows(snapshot, "channels")
            .map(|channel| text(channel, "id"))
            .collect();
        let mut pending: Vec<_> = rows(snapshot, "approvalRequests")
            .filter(|request| {
                text(request, "status") == "pending"
                    && !text(request, "id").is_empty()
                    && tasks.contains_key(text(request, "taskId"))
            })
            .collect();
        pending
            .sort_by_key(|request| std::cmp::Reverse(request["createdAt"].as_i64().unwrap_or(0)));
        let mut entries = vec![];
        let mut request_ids = HashSet::new();
        for request in pending {
            if entries.len() == MAX_NOTIFICATIONS {
                break;
            }
            if !request_ids.insert(text(request, "id")) {
                continue;
            }
            let task = tasks[text(request, "taskId")];
            let input = request.get("input").is_some_and(|input| !input.is_null());
            entries.push(NotificationEntry {
                id: format!("approval:{}", text(request, "id")),
                created_at: request["createdAt"]
                    .as_i64()
                    .unwrap_or_else(|| chrono::Utc::now().timestamp_millis()),
                task_id: text(request, "taskId").into(),
                channel_id: None,
                origin: task_origin(snapshot, task),
                title: if input {
                    "Input needed"
                } else {
                    "Approval needed"
                }
                .into(),
                detail: retained_detail(text(request, "summary")),
                message_id: None,
                summary: None,
                kind: NotificationKind::Approval {
                    request: request.clone(),
                },
                tone: NotificationTone::Warning,
            });
        }
        let mut alerts: Vec<_> = self
            .alerts
            .values()
            .filter(|alert| {
                if let Some(channel) = &alert.entry.channel_id {
                    channels.contains(channel.as_str())
                } else {
                    tasks.contains_key(&alert.entry.task_id)
                }
            })
            .collect();
        alerts.sort_by_key(|alert| std::cmp::Reverse(alert.sequence));
        let remaining = MAX_NOTIFICATIONS.saturating_sub(entries.len());
        entries.extend(
            alerts
                .into_iter()
                .take(remaining)
                .map(|alert| alert.entry.clone()),
        );
        entries.truncate(MAX_NOTIFICATIONS);
        entries
    }
    pub fn take_summary_requests(&mut self) -> Vec<(String, String)> {
        let attempts = &mut self.summary_attempts;
        self.alerts
            .values()
            .filter_map(|alert| {
                let entry = &alert.entry;
                if entry.detail.trim().chars().count() <= 120 {
                    return None;
                }
                let message = entry.message_id.as_ref()?;
                attempts
                    .insert(message.clone())
                    .then(|| (entry.task_id.clone(), message.clone()))
            })
            .collect()
    }
    pub fn apply_summary(&mut self, task_id: &str, message_id: &str, summary: String) {
        if let Some(alert) = self.alerts.get_mut(&format!("task:{task_id}")) {
            if alert.entry.message_id.as_deref() == Some(message_id) && !summary.trim().is_empty() {
                alert.entry.summary = Some(summary);
            }
        }
    }
    pub fn dismiss(&mut self, id: &str) {
        self.alerts.retain(|_, alert| alert.entry.id != id);
    }
    pub fn mark_task_read(&mut self, task_id: &str) {
        self.alerts
            .retain(|_, alert| alert.entry.task_id != task_id);
    }
    pub fn mark_channel_read(&mut self, channel_id: &str) {
        self.alerts
            .retain(|_, alert| alert.entry.channel_id.as_deref() != Some(channel_id));
    }
}

#[cfg(test)]
mod tests {
    use super::NotificationModel;
    use crate::notifications::{NotificationKind, NotificationTone};
    use serde_json::{Value, json};
    fn snapshot() -> Value {
        json!({"agents":[{"id":"agent","name":"Codex","provider":"codex","internal":false},{"id":"internal","internal":true}],"tasks":[{"id":"task","agentId":"agent","title":"New layout","status":"running","archived":false,"createdAt":1,"updatedAt":10}],"messages":[],"channels":[{"id":"channel","name":"Channel","agentIds":["agent"],"messages":[]}],"approvalRequests":[]})
    }
    fn message(id: &str, time: i64, status: &str, phase: &str) -> Value {
        json!({"id":id,"taskId":"task","role":"assistant","text":"A useful answer","createdAt":time,"streamStatus":status,"phase":phase})
    }
    #[derive(Default)]
    struct Differential {
        full: NotificationModel,
        incremental: NotificationModel,
        previous: Value,
        projection: Option<std::sync::Arc<super::NotificationProjection>>,
    }
    impl Differential {
        fn sync(&mut self, source: &Value, reset: bool) {
            use monitter_gpui_desktop::native_publication::{PublicationChanges, PublicationIndex};
            let mut index = PublicationIndex::default();
            index.messages = source["messages"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|row| {
                    serde_json::from_value::<monitter_lib::model::Message>(row.clone()).unwrap()
                })
                .collect::<Vec<_>>()
                .into();
            let mut changes = PublicationChanges {
                reset,
                ..Default::default()
            };
            let mut updates: std::collections::HashMap<String, Vec<Value>> = Default::default();
            for row in source["messages"].as_array().into_iter().flatten() {
                if !self.previous["messages"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|old| old == row)
                {
                    let id = super::text(row, "taskId").to_owned();
                    changes.message_tasks.insert(id.clone());
                    changes
                        .message_ids
                        .insert(super::text(row, "id").to_owned());
                    updates.entry(id).or_default().push(row.clone());
                }
            }
            for old in self.previous["messages"].as_array().into_iter().flatten() {
                if !source["messages"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|row| super::text(row, "id") == super::text(old, "id"))
                {
                    changes
                        .removed_message_ids
                        .insert(super::text(old, "id").to_owned());
                    changes
                        .message_tasks
                        .insert(super::text(old, "taskId").to_owned());
                }
            }
            for row in source["channels"].as_array().into_iter().flatten() {
                if !self.previous["channels"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|old| old == row)
                {
                    changes
                        .changed_channels
                        .insert(super::text(row, "id").to_owned());
                }
            }
            let mut compact = source.clone();
            compact["messages"] = json!([]);
            let prepared = super::prepare_notifications(
                &compact,
                &index,
                &updates,
                &changes,
                self.projection.as_deref(),
            );
            self.incremental.sync_prepared(&compact, &prepared);
            self.full.sync(source);
            // The two models observe the same event at different wall-clock instants.
            // Compare semantic projection; retain authoritative approval timestamps.
            let stable = |mut entries: Vec<crate::notifications::NotificationEntry>| {
                for entry in &mut entries {
                    if !matches!(entry.kind, NotificationKind::Approval { .. }) { entry.created_at = 0; }
                }
                entries
            };
            assert_eq!(stable(self.incremental.entries(&compact)), stable(self.full.entries(source)));
            self.projection = Some(prepared.projection);
            self.previous = source.clone();
        }
    }
    #[::core::prelude::v1::test]
    fn notification_incremental_matches_baseline_finalization_legacy_dismissal_reset_and_removal() {
        let mut state = snapshot();
        state["messages"] = json!([message("old", 5, "streaming", "final_answer")]);
        let mut models = Differential::default();
        models.sync(&state, true);
        state["messages"][0]["streamStatus"] = json!("complete");
        models.sync(&state, false);
        models.incremental.dismiss("task:task");
        models.full.dismiss("task:task");
        models.sync(&state, false);
        state["messages"].as_array_mut().unwrap().push(message(
            "new",
            20,
            "streaming",
            "final_answer",
        ));
        models.sync(&state, false);
        state["messages"][1]["streamStatus"] = json!("complete");
        models.sync(&state, false);
        state["tasks"][0]["status"] = json!("error");
        state["tasks"][0]["error"] = json!("Fixture failure");
        models.sync(&state, false);
        models.incremental.mark_task_read("task");
        models.full.mark_task_read("task");
        state["tasks"][0]["status"] = json!("running");
        state["messages"].as_array_mut().unwrap().push(json!({"id":"legacy","taskId":"task","role":"assistant","createdAt":25,"text":"Partial"}));
        models.sync(&state, false);
        state["messages"][2]["text"] = json!("Final legacy reply");
        state["tasks"][0]["status"] = json!("completed");
        models.sync(&state, false);
        models.incremental.dismiss("task:task");
        models.full.dismiss("task:task");
        // A full refresh and an older page must not resurrect dismissed replies.
        models.sync(&state, true);
        state["messages"]
            .as_array_mut()
            .unwrap()
            .insert(0, message("paged", 1, "complete", "final_answer"));
        models.sync(&state, false);
        state["channels"][0]["messages"] = json!([{"id":"channel-reply","role":"assistant","agentId":"agent","text":"Channel answer","createdAt":30}]);
        models.sync(&state, false);
        state["tasks"] = json!([]);
        state["messages"] = json!([]);
        state["channels"] = json!([]);
        models.sync(&state, false);
    }
    #[::core::prelude::v1::test]
    fn notification_incremental_observes_one_changed_body_and_zero_unrelated_history_rows() {
        let mut state = snapshot();
        state["messages"] = json!(
            (0..1000)
                .map(|i| message(&format!("old-{i}"), i - 1000, "complete", "final_answer"))
                .collect::<Vec<_>>()
        );
        state["messages"].as_array_mut().unwrap().push(message(
            "stream",
            20,
            "streaming",
            "final_answer",
        ));
        let mut models = Differential::default();
        models.sync(&state, true);
        models.incremental.observed_messages = 0;
        state["messages"][1000]["text"] = json!("Changed streaming body");
        models.sync(&state, false);
        assert_eq!(models.incremental.observed_messages, 1);
        models.incremental.observed_messages = 0;
        state["tasks"][0]["updatedAt"] = json!(40);
        state["settings"] = json!({"theme":"light"});
        models.sync(&state, false);
        assert_eq!(models.incremental.observed_messages, 0);
        state["messages"][1000]["streamStatus"] = json!("complete");
        models.sync(&state, false);
        assert_eq!(models.incremental.observed_messages, 1);
    }
    #[::core::prelude::v1::test]
    fn notification_historical_page_at_the_baseline_timestamp_stays_read() {
        let mut state = snapshot();
        let mut model = NotificationModel::default();
        model.sync(&state);
        state["messages"] = json!([message("paged-history", 10, "complete", "final_answer")]);
        model.sync(&state);
        assert!(model.entries(&state).is_empty());
    }
    #[::core::prelude::v1::test]
    fn notification_historical_legacy_reply_is_not_reannounced_when_a_running_task_completes() {
        let mut state = snapshot();
        state["messages"] = json!([{"id":"old","taskId":"task","role":"assistant","text":"Historical answer","createdAt":5}]);
        let mut model = NotificationModel::default();
        model.sync(&state);
        model.sync(&state);
        state["tasks"][0]["status"] = json!("completed");
        model.sync(&state);
        let entries = model.entries(&state);
        assert_eq!(entries.len(), 1);
        assert!(matches!(entries[0].kind, NotificationKind::Attention));
    }
    #[::core::prelude::v1::test]
    fn notification_first_authoritative_snapshot_baselines_history_but_keeps_pending_requests() {
        let mut model = NotificationModel::default();
        model.sync(&Value::Null);
        let mut state = snapshot();
        state["tasks"][0]["status"] = json!("completed");
        state["messages"] = json!([message("old", 5, "complete", "final_answer")]);
        state["approvalRequests"] = json!([{"id":"permission","taskId":"task","status":"pending","summary":"Run a command","createdAt":11}]);
        model.sync(&state);
        let entries = model.entries(&state);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "approval:permission");
        assert!(
            matches!(&entries[0].kind,NotificationKind::Approval {request} if request["id"]=="permission")
        );
    }
    #[::core::prelude::v1::test]
    fn notification_streaming_and_commentary_wait_for_same_id_final_transition() {
        let mut state = snapshot();
        let mut model = NotificationModel::default();
        model.sync(&state);
        state["messages"] = json!([
            message("answer", 20, "streaming", "final_answer"),
            message("comment", 21, "complete", "commentary")
        ]);
        model.sync(&state);
        assert!(model.entries(&state).is_empty());
        state["messages"][0]["streamStatus"] = json!("complete");
        model.sync(&state);
        let entries = model.entries(&state);
        assert_eq!(entries.len(), 1);
        assert!(matches!(entries[0].kind, NotificationKind::Message));
        assert!(entries[0].title.is_empty());
        assert_eq!(entries[0].origin, "Codex | New layout");
        assert_eq!(entries[0].tone, NotificationTone::Info);
        model.sync(&state);
        assert_eq!(model.entries(&state).len(), 1);
        model.dismiss("task:task");
        model.sync(&state);
        assert!(model.entries(&state).is_empty());
    }
    #[::core::prelude::v1::test]
    fn notification_legacy_unmarked_assistant_waits_until_task_finishes() {
        let mut state = snapshot();
        let mut model = NotificationModel::default();
        model.sync(&state);
        state["messages"] = json!([{"id":"legacy","taskId":"task","role":"assistant","text":"Partial","createdAt":20}]);
        model.sync(&state);
        assert!(model.entries(&state).is_empty());
        state["messages"][0]["text"] = json!("Final answer");
        state["tasks"][0]["status"] = json!("completed");
        model.sync(&state);
        let entries = model.entries(&state);
        assert_eq!(entries.len(), 1);
        assert!(matches!(entries[0].kind, NotificationKind::Message));
        assert_eq!(entries[0].detail, "Final answer");
    }
    #[::core::prelude::v1::test]
    fn notification_initial_stream_can_finalize_without_a_new_timestamp() {
        let mut state = snapshot();
        state["messages"] = json!([message("active", 5, "streaming", "final_answer")]);
        let mut model = NotificationModel::default();
        model.sync(&state);
        state["messages"][0]["streamStatus"] = json!("complete");
        model.sync(&state);
        assert_eq!(model.entries(&state).len(), 1);
    }
    #[::core::prelude::v1::test]
    fn notification_unread_survives_cache_eviction_and_history_does_not_reannounce() {
        let mut state = snapshot();
        let mut model = NotificationModel::default();
        model.sync(&state);
        state["messages"] = json!([message("new", 20, "complete", "final_answer")]);
        model.sync(&state);
        state["messages"] = json!([]);
        model.sync(&state);
        assert_eq!(model.entries(&state).len(), 1);
        model.mark_task_read("task");
        state["messages"] = json!([
            message("new", 20, "complete", "final_answer"),
            message("older-page", 4, "complete", "final_answer")
        ]);
        model.sync(&state);
        assert!(model.entries(&state).is_empty());
    }
    #[::core::prelude::v1::test]
    fn notification_completed_error_and_interrupted_are_actual_running_transitions() {
        for status in ["completed", "error", "interrupted"] {
            let mut state = snapshot();
            let mut model = NotificationModel::default();
            model.sync(&state);
            state["tasks"][0]["status"] = json!(status);
            model.sync(&state);
            let entries = model.entries(&state);
            assert_eq!(entries.len(), 1);
            assert!(matches!(entries[0].kind, NotificationKind::Attention));
            model.mark_task_read("task");
            model.sync(&state);
            assert!(model.entries(&state).is_empty());
        }
    }
    #[::core::prelude::v1::test]
    fn notification_current_requests_resolve_and_input_preserves_the_real_request() {
        let mut state = snapshot();
        state["approvalRequests"] = json!([{"id":"question","taskId":"task","status":"pending","summary":"Choose a folder","input":{"kind":"questions","questions":[{"id":"q","question":"Where?"}]}},{"id":"missing","taskId":"not-present","status":"pending"}]);
        let mut model = NotificationModel::default();
        model.sync(&state);
        let entries = model.entries(&state);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].title.starts_with("Input needed"));
        assert_eq!(entries[0].title, "Input needed");
        assert_eq!(entries[0].origin, "Codex | New layout");
        assert!(
            matches!(&entries[0].kind,NotificationKind::Approval {request} if request["input"]["kind"]=="questions")
        );
        model.dismiss("approval:question");
        model.mark_task_read("task");
        assert_eq!(model.entries(&state).len(), 1);
        state["approvalRequests"][0]["status"] = json!("approved");
        assert!(model.entries(&state).is_empty());
        model.sync(&state);
        assert!(model.entries(&state).is_empty());
    }
    #[::core::prelude::v1::test]
    fn notification_archived_deleted_internal_and_channel_tasks_do_not_leave_alerts() {
        for field in ["archived", "deleted", "internal", "channel"] {
            let mut state = snapshot();
            let mut model = NotificationModel::default();
            model.sync(&state);
            state["messages"] = json!([message("reply", 20, "complete", "final_answer")]);
            model.sync(&state);
            assert_eq!(model.entries(&state).len(), 1);
            match field {
                "archived" => state["tasks"][0]["archived"] = json!(true),
                "deleted" => state["tasks"] = json!([]),
                "internal" => state["tasks"][0]["agentId"] = json!("internal"),
                _ => state["tasks"][0]["channelId"] = json!("channel"),
            };
            assert!(model.entries(&state).is_empty());
            model.sync(&state);
            assert!(model.alerts.is_empty());
        }
    }
    #[::core::prelude::v1::test]
    fn notification_channel_assistant_rows_are_unread_once_and_channel_read_is_scoped() {
        let mut state = snapshot();
        let mut model = NotificationModel::default();
        model.sync(&state);
        state["channels"][0]["messages"] = json!([{"id":"user","role":"user","text":"Hi","createdAt":11},{"id":"reply","role":"assistant","agentId":"agent","text":"Channel answer","createdAt":20}]);
        state["messages"] = json!([message("chat", 21, "complete", "final_answer")]);
        model.sync(&state);
        assert_eq!(model.entries(&state).len(), 2);
        model.mark_channel_read("channel");
        model.sync(&state);
        let entries = model.entries(&state);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].task_id, "task");
        state["channels"][0]["messages"] = json!([]);
        model.sync(&state);
        assert_eq!(model.entries(&state).len(), 1);
    }
    #[::core::prelude::v1::test]
    fn notification_detail_keeps_long_unicode_text_for_expansion() {
        let mut state = snapshot();
        let mut model = NotificationModel::default();
        model.sync(&state);
        let full = format!("{}終", "界".repeat(240));
        state["messages"] = json!([{"id":"long","taskId":"task","role":"assistant","text":full,"createdAt":20,"streamStatus":"complete","phase":"final_answer"}]);
        model.sync(&state);
        let entries = model.entries(&state);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].detail, full);
    }
    #[::core::prelude::v1::test]
    fn notification_attention_is_dismissible_but_pending_approval_is_retained() {
        let mut state = snapshot();
        let mut model = NotificationModel::default();
        model.sync(&state);
        state["tasks"][0]["status"] = json!("error");
        model.sync(&state);
        let attention = model.entries(&state).pop().unwrap();
        assert_eq!(attention.title, "Chat failed");
        assert_eq!(attention.tone, NotificationTone::Danger);
        model.dismiss(&attention.id);
        state["tasks"][0]["status"] = json!("running");
        state["approvalRequests"] = json!([{"id":"pending","taskId":"task","status":"pending","summary":"Confirm","createdAt":30}]);
        assert_eq!(model.entries(&state).len(), 1);
        model.dismiss("approval:pending");
        assert_eq!(model.entries(&state).len(), 1);
    }
    #[::core::prelude::v1::test]
    fn notification_latest_per_scope_and_retained_cards_are_bounded() {
        let mut state = snapshot();
        state["tasks"]=json!((0..120).map(|index|json!({"id":format!("t{index}"),"agentId":"agent","title":"Chat","status":"running","updatedAt":10})).collect::<Vec<_>>());
        let mut model = NotificationModel::default();
        model.sync(&state);
        state["messages"]=json!((0..120).map(|index|json!({"id":format!("m{index}"),"taskId":format!("t{index}"),"role":"assistant","text":"Answer","phase":"final_answer","streamStatus":"complete","createdAt":20})).collect::<Vec<_>>());
        model.sync(&state);
        assert_eq!(model.entries(&state).len(), 100);
        assert_eq!(model.alerts.len(), 100);
        state["messages"]=json!((0..200).map(|index|json!({"id":format!("message{index:03}"),"taskId":"t119","role":"assistant","text":format!("Answer {index}"),"phase":"final_answer","streamStatus":"complete","createdAt":30+index})).collect::<Vec<_>>());
        model.sync(&state);
        assert_eq!(model.alerts.len(), 100);
        assert!(model.tasks.values().all(|cursor| cursor.seen.len() <= 64));
        let latest = model
            .entries(&state)
            .into_iter()
            .find(|entry| entry.task_id == "t119")
            .unwrap();
        assert_eq!(latest.detail, "Answer 199");
        model.mark_task_read("t119");
        model.sync(&state);
        assert!(
            !model
                .entries(&state)
                .iter()
                .any(|entry| entry.task_id == "t119")
        );
    }
    #[::core::prelude::v1::test]
    fn notification_summary_is_requested_once_and_stale_response_cannot_replace_new_final() {
        let mut state = snapshot();
        let mut model = NotificationModel::default();
        model.sync(&state);
        state["messages"] = json!([message("first", 20, "complete", "final_answer")]);
        state["messages"][0]["text"] = json!("Observed result and validation detail. ".repeat(8));
        model.sync(&state);
        assert_eq!(
            model.take_summary_requests(),
            vec![("task".into(), "first".into())]
        );
        assert!(model.take_summary_requests().is_empty());
        let full = model.entries(&state)[0].detail.clone();
        model.apply_summary(
            "task",
            "first",
            "The agent reported the observed result.".into(),
        );
        assert_eq!(model.entries(&state)[0].detail, full);
        assert_eq!(
            model.entries(&state)[0].summary.as_deref(),
            Some("The agent reported the observed result.")
        );
        state["messages"] = json!([message("second", 30, "complete", "final_answer")]);
        state["messages"][0]["text"] = json!("New result and validation detail. ".repeat(8));
        model.sync(&state);
        model.apply_summary("task", "first", "Stale result.".into());
        assert!(model.entries(&state)[0].summary.is_none());
        assert_eq!(
            model.take_summary_requests(),
            vec![("task".into(), "second".into())]
        );
        model.dismiss("task:task");
        model.apply_summary("task", "second", "Already dismissed.".into());
        assert!(model.entries(&state).is_empty());
    }

    #[::core::prelude::v1::test]
    fn notification_short_final_does_not_request_admin_summary() {
        let mut state = snapshot();
        let mut model = NotificationModel::default();
        model.sync(&state);
        state["messages"] = json!([message("short", 20, "complete", "final_answer")]);
        model.sync(&state);
        assert_eq!(model.entries(&state).len(), 1);
        assert!(model.take_summary_requests().is_empty());
    }

    #[::core::prelude::v1::test]
    fn notification_initial_watermark_includes_invisible_task_history() {
        let mut state = snapshot();
        state["messages"] = json!([{"id":"hidden-history", "taskId":"archived-task", "role":"assistant", "text":"Old", "createdAt":100}]);
        let mut models = Differential::default();
        models.sync(&state, true);
        assert_eq!(models.full.baseline_time, 100);
        assert_eq!(models.incremental.baseline_time, 100);
        state["channels"][0]["messages"] = json!([{"id":"late-page", "role":"assistant", "agentId":"agent", "text":"Historical channel reply", "createdAt":50}]);
        models.sync(&state, false);
        assert!(models.incremental.entries(&state).is_empty());
    }

    #[::core::prelude::v1::test]
    fn notification_incremental_channels_follow_finalization_read_visibility_and_recreation() {
        let mut state = snapshot();
        let mut models = Differential::default();
        models.sync(&state, true);
        state["channels"][0]["messages"] = json!([{"id":"channel-stream", "role":"assistant", "agentId":"agent", "text":"Partial", "streamStatus":"streaming", "phase":"final_answer", "createdAt":20}]);
        models.sync(&state, false);
        assert!(models.incremental.entries(&state).is_empty());
        state["channels"][0]["messages"][0]["streamStatus"] = json!("complete");
        models.sync(&state, false);
        assert_eq!(models.incremental.entries(&state).len(), 1);
        models.incremental.mark_channel_read("channel");
        models.full.mark_channel_read("channel");
        models.sync(&state, false);
        state["channels"][0]["messages"].as_array_mut().unwrap().push(json!({"id":"internal-reply", "role":"assistant", "agentId":"internal", "text":"Hidden until agent becomes visible", "createdAt":30}));
        models.sync(&state, false);
        assert!(models.incremental.entries(&state).is_empty());
        state["agents"][1]["internal"] = json!(false);
        models.sync(&state, false);
        assert_eq!(models.incremental.entries(&state).len(), 1);
        let channel = state["channels"][0].clone();
        state["channels"] = json!([]);
        models.sync(&state, false);
        assert!(models.incremental.entries(&state).is_empty());
        state["channels"] = json!([channel]);
        models.sync(&state, false);
        assert!(
            models.incremental.entries(&state).is_empty(),
            "recreated scope baselines retained history"
        );
        models.sync(&state, true);
    }
}
