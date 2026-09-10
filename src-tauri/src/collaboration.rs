//! Durable, local routing for per-turn Monitter collaboration grants.
//!
//! The loopback broker authenticates the originating task and calls [`Service::protocol`].
//! This module never accepts a caller task id from tool arguments.

use crate::{
    create_task_in_data,
    model::{id, now, Agent, Collaboration, CreateTaskInput, Message, RunEvent, Snapshot, Task},
    Service,
};
use serde_json::{json, Value};
use std::{
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

const MAX_REQUEST_ID: usize = 128;
const MAX_TEXT: usize = 64 * 1024;
const MAX_TITLE: usize = 240;
const MAX_DIRECTORY: usize = 12;
const MAX_LIST_MESSAGES: usize = 20;
const MAX_MESSAGE_RETURN: usize = 1_000;
const MAX_RESULT_RETURN: usize = 16 * 1024;
const MAX_DEPTH: usize = 4;
const MAX_PER_TASK: usize = 24;
const MAX_PER_ROOT: usize = 64;
const MAX_ACTIVE: usize = 4;
const DELIVERY_UNCERTAIN: &str =
    "Delivery was interrupted by Monitter restarting; it was not replayed because delivery is uncertain.";
const DELIVERED_TO_ACTIVE_TURN: &str =
    "Delivered to the recipient’s active turn via its Monitter inbox.";

impl Service {
    pub(crate) fn protocol(
        self: &Arc<Self>,
        caller_task: &str,
        tool: &str,
        args: Value,
    ) -> Result<Value, String> {
        let args = args
            .as_object()
            .ok_or_else(|| "Tool arguments must be an object.".to_string())?;
        self.require_collaboration_caller(caller_task)?;
        validate_tool_args(tool, args)?;
        match tool {
            "list_agents" => self.list_agents_protocol(caller_task, string_arg(args, "query")?),
            "delegate_task" => self.queue_protocol(caller_task, args, "delegation"),
            "send_message" => self.queue_protocol(caller_task, args, "message"),
            "get_task_result" => {
                self.get_result_protocol(caller_task, required_string(args, "collaboration_id")?)
            }
            "wait_for_task" => self.wait_protocol(caller_task, args),
            "list_messages" => self.list_messages_protocol(caller_task),
            "cancel_delegation" => {
                self.cancel_protocol(caller_task, required_string(args, "collaboration_id")?)
            }
            _ => Err("Unknown Monitter collaboration tool.".into()),
        }
    }

    fn require_collaboration_caller(&self, task_id: &str) -> Result<(Task, Agent), String> {
        let data = self
            .data
            .lock()
            .map_err(|_| "Monitter state lock failed.".to_string())?;
        let task = data
            .snapshot
            .tasks
            .iter()
            .find(|task| task.id == task_id)
            .cloned()
            .ok_or_else(|| "Collaboration caller task was not found.".to_string())?;
        if task.status != "running" {
            return Err("Collaboration is available only while this task is running.".into());
        }
        let agent = data
            .snapshot
            .agents
            .iter()
            .find(|agent| agent.id == task.agent_id)
            .cloned()
            .ok_or_else(|| "Collaboration caller agent was not found.".to_string())?;
        if !agent.collaboration_enabled {
            return Err("Collaboration is disabled for this agent.".into());
        }
        Ok((task, agent))
    }

    fn list_agents_protocol(
        &self,
        caller_task: &str,
        query: Option<&str>,
    ) -> Result<Value, String> {
        let needle = query.unwrap_or_default().trim().to_lowercase();
        let data = self
            .data
            .lock()
            .map_err(|_| "Monitter state lock failed.".to_string())?;
        let agents = data
            .snapshot
            .agents
            .iter()
            .filter(|agent| agent.collaboration_enabled)
            .filter(|agent| profile_matches(agent, &needle))
            .take(MAX_DIRECTORY)
            .map(|agent| {
                let host = data
                    .snapshot
                    .hosts
                    .iter()
                    .find(|host| host.id == agent.host_id);
                let current_task_id = data
                    .snapshot
                    .tasks
                    .iter()
                    .rev()
                    .find(|task| task.agent_id == agent.id && task.status == "running")
                    .map(|task| task.id.clone());
                json!({
                    "id": agent.id,
                    "name": truncate(&agent.name, 120),
                    "description": truncate(&agent.description, 320),
                    "expertise": compact_entries(&agent.expertise),
                    "responsibilities": compact_entries(&agent.responsibilities),
                    "skills": compact_entries(&agent.skills),
                    "provider": agent.provider,
                    "model": truncate(&agent.model, 120),
                    "host": host.map(|host| truncate(&host.name, 120)),
                    "configured": host.is_some(),
                    "current_task_id": current_task_id,
                })
            })
            .collect::<Vec<_>>();
        Ok(json!({"agents": agents, "caller_task_id": caller_task}))
    }

    fn queue_protocol(
        &self,
        caller_task: &str,
        args: &serde_json::Map<String, Value>,
        kind: &str,
    ) -> Result<Value, String> {
        let to_agent_id = required_string(args, "to_agent_id")?;
        let text = required_string(args, "message")?;
        let request_id = required_string(args, "request_id")?;
        let title = if kind == "delegation" {
            required_string(args, "title")?
        } else {
            "Peer message".to_string()
        };
        validate_request(&to_agent_id, &title, &text, &request_id)?;
        let target_task_arg = string_arg(args, "task_id")?.map(str::to_owned);

        let collaboration = self.mutate_data(Some(caller_task.to_string()), |data| {
            let caller = collaboration_caller(&data.snapshot, caller_task)?;
            let target_agent = data
                .snapshot
                .agents
                .iter()
                .find(|agent| agent.id == to_agent_id && agent.collaboration_enabled)
                .cloned()
                .ok_or_else(|| "Recipient agent is unavailable for collaboration.".to_string())?;
            if target_agent.id == caller.agent_id {
                return Err("An agent cannot delegate or message itself.".into());
            }
            if let Some(existing) = data
                .snapshot
                .collaborations
                .iter()
                .find(|item| item.from_task_id == caller_task && item.request_id == request_id)
                .cloned()
            {
                if existing.to_agent_id != to_agent_id
                    || existing.kind != kind
                    || existing.text != text
                    || (kind == "message"
                        && target_task_arg
                            .as_deref()
                            .is_some_and(|target| target != existing.to_task_id.as_str()))
                {
                    return Err(
                        "request_id was already used with different collaboration details.".into(),
                    );
                }
                return Ok(existing);
            }
            enforce_budget(&data.snapshot, caller_task)?;
            if target_task_arg.is_none()
                && is_agent_ancestor(&data.snapshot, caller_task, &target_agent.id)
            {
                return Err("This delegation would create an agent ancestry cycle.".into());
            }

            let to_task_id = match (kind, target_task_arg.as_deref()) {
                ("message", Some(task_id)) => {
                    let target = data
                        .snapshot
                        .tasks
                        .iter()
                        .find(|task| task.id == task_id)
                        .ok_or_else(|| "Recipient task was not found.".to_string())?;
                    if target.agent_id != target_agent.id
                        || !directly_connected(&data.snapshot, caller_task, task_id)
                    {
                        return Err(
                            "Messages may only target a directly connected collaboration task."
                                .into(),
                        );
                    }
                    target.id.clone()
                }
                (_, None) => {
                    create_task_in_data(
                        data,
                        CreateTaskInput {
                            agent_id: target_agent.id.clone(),
                            title: title.clone(),
                            native_session_id: None,
                            parent_task_id: Some(caller_task.to_string()),
                            channel_id: None,
                            project_id: caller.project_id.clone(),
                            model_settings: None,
                            sandbox: None,
                        },
                    )?
                    .id
                }
                _ => return Err("task_id is supported only for peer messages.".into()),
            };
            let timestamp = now();
            let collaboration = Collaboration {
                id: id(),
                kind: kind.to_string(),
                from_agent_id: caller.agent_id,
                from_task_id: caller.id,
                to_agent_id,
                to_task_id,
                text,
                request_id,
                status: "queued".into(),
                result: None,
                error: None,
                created_at: timestamp,
                updated_at: timestamp,
            };
            data.snapshot.collaborations.push(collaboration.clone());
            data.snapshot.events.push(RunEvent {
                id: id(),
                task_id: caller_task.into(),
                kind: "collaboration".into(),
                title: "Collaboration queued".into(),
                detail: collaboration.id.clone(),
                created_at: timestamp,
            });
            Ok(collaboration)
        })?;
        Ok(collaboration_value(&collaboration, false))
    }

    /// Marks queued deliveries running and starts their harness outside the persistence lock.
    pub(crate) fn dispatch_collaborations(self: &Arc<Self>) {
        loop {
            if !self.has_dispatchable_collaboration() {
                return;
            }
            let next = match self.prepare_next_collaboration() {
                Ok(next) => next,
                Err(_) => return,
            };
            let Some((task_id, prompt)) = next else {
                return;
            };
            if let Err(error) = self.launch(task_id.clone(), prompt) {
                if error.contains("active Monitter writer") || error.contains("active turn") {
                    let _ = self.requeue_busy_collaboration(&task_id);
                } else {
                    self.finish(&task_id, "error", Some(error));
                }
            }
        }
    }

    fn requeue_busy_collaboration(&self, task_id: &str) -> Result<(), String> {
        self.mutate(Some(task_id.into()), |snapshot| {
            let Some(item) = snapshot
                .collaborations
                .iter_mut()
                .find(|item| item.to_task_id == task_id && item.status == "running")
            else {
                return Ok(());
            };
            item.status = "queued".into();
            item.updated_at = now();
            if let Some(task) = snapshot
                .tasks
                .iter_mut()
                .find(|task| task.id == task_id && task.status == "running")
            {
                task.status = "idle".into();
                task.updated_at = now();
            }
            Ok(())
        })
    }

    fn has_dispatchable_collaboration(&self) -> bool {
        let Ok(data) = self.data.lock() else {
            return false;
        };
        let running = data
            .snapshot
            .collaborations
            .iter()
            .filter(|item| item.status == "running")
            .count();
        if running >= MAX_ACTIVE {
            return false;
        }
        data.snapshot
            .collaborations
            .iter()
            .filter(|item| item.status == "queued")
            .any(|item| {
                let Some(task) = data
                    .snapshot
                    .tasks
                    .iter()
                    .find(|task| task.id == item.to_task_id)
                else {
                    return true;
                };
                let enabled = data
                    .snapshot
                    .agents
                    .iter()
                    .any(|agent| agent.id == item.to_agent_id && agent.collaboration_enabled);
                !enabled
                    || task.archived
                    || (task.status != "running" && !self.run_is_active(&task.id))
            })
    }

    fn prepare_next_collaboration(&self) -> Result<Option<(String, String)>, String> {
        self.mutate_data(None, |data| {
            let snapshot = &mut data.snapshot;
            if snapshot
                .collaborations
                .iter()
                .filter(|item| item.status == "running")
                .count()
                >= MAX_ACTIVE
            {
                return Ok(None);
            }
            let terminal = snapshot.collaborations.iter().position(|item| {
                if item.status != "queued" {
                    return false;
                }
                let task = snapshot
                    .tasks
                    .iter()
                    .find(|task| task.id == item.to_task_id);
                task.is_none()
                    || task.is_some_and(|task| task.archived)
                    || !snapshot
                        .agents
                        .iter()
                        .any(|agent| agent.id == item.to_agent_id && agent.collaboration_enabled)
            });
            if let Some(index) = terminal {
                let item = snapshot.collaborations[index].clone();
                let reason = if !snapshot
                    .agents
                    .iter()
                    .any(|agent| agent.id == item.to_agent_id && agent.collaboration_enabled)
                {
                    "Recipient agent is unavailable for collaboration."
                } else if snapshot
                    .tasks
                    .iter()
                    .find(|task| task.id == item.to_task_id)
                    .is_none()
                {
                    "Recipient task was not found."
                } else {
                    "Recipient task is archived."
                };
                fail_queued_delivery(snapshot, index, reason);
                return Ok(None);
            }
            let Some(index) = snapshot.collaborations.iter().position(|item| {
                item.status == "queued"
                    && snapshot
                        .tasks
                        .iter()
                        .find(|task| task.id == item.to_task_id)
                        .is_some_and(|task| {
                            task.status != "running" && !self.run_is_active(&task.id)
                        })
            }) else {
                return Ok(None);
            };
            let item = snapshot.collaborations[index].clone();
            let task_index = snapshot
                .tasks
                .iter()
                .position(|task| task.id == item.to_task_id)
                .expect("validated queued task");
            let peer_name = snapshot
                .agents
                .iter()
                .find(|agent| agent.id == item.from_agent_id)
                .map(|agent| agent.name.clone())
                .unwrap_or_else(|| "A peer agent".into());
            let peer_context = format!("Peer context from {peer_name}:\n{}", item.text);
            let first_turn = !snapshot
                .messages
                .iter()
                .any(|message| message.task_id == item.to_task_id && message.role == "user");
            let instructions = if first_turn {
                snapshot
                    .messages
                    .iter()
                    .find(|message| message.task_id == item.to_task_id && message.role == "system")
                    .map(|message| message.text.clone())
            } else {
                None
            };
            if !snapshot
                .messages
                .iter()
                .any(|message| message.collaboration_id.as_deref() == Some(item.id.as_str()))
            {
                snapshot.messages.push(Message {
                    id: id(),
                    task_id: item.to_task_id.clone(),
                    role: "user".into(),
                    text: peer_context.clone(),
                    created_at: now(),
                    sender_agent_id: Some(item.from_agent_id.clone()),
                    collaboration_id: Some(item.id.clone()),
                    attachments: vec![],
                });
            }
            snapshot.tasks[task_index].status = "running".into();
            snapshot.tasks[task_index].updated_at = now();
            snapshot.collaborations[index].status = "running".into();
            snapshot.collaborations[index].updated_at = now();
            snapshot.events.push(RunEvent {
                id: id(),
                task_id: item.to_task_id.clone(),
                kind: "collaboration".into(),
                title: "Peer delivery started".into(),
                detail: item.id.clone(),
                created_at: now(),
            });
            Ok(Some((
                item.to_task_id,
                instructions
                    .map(|instructions| format!("{instructions}\n\n{peer_context}"))
                    .unwrap_or(peer_context),
            )))
        })
    }

    pub(crate) fn complete_collaborations(
        snapshot: &mut Snapshot,
        task_id: &str,
        status: &str,
        error: Option<&str>,
    ) {
        let finished = now();
        let matching = snapshot
            .collaborations
            .iter()
            .enumerate()
            .filter(|(_, item)| item.to_task_id == task_id && item.status == "running")
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        for index in matching {
            let item = snapshot.collaborations[index].clone();
            let result = snapshot
                .messages
                .iter()
                .position(|message| message.collaboration_id.as_deref() == Some(item.id.as_str()))
                .and_then(|marker| {
                    snapshot.messages[marker + 1..]
                        .iter()
                        .rev()
                        .find(|message| message.task_id == task_id && message.role == "assistant")
                })
                .map(|message| truncate(&message.text, MAX_RESULT_RETURN));
            let terminal = if status == "completed" {
                "completed"
            } else if status == "interrupted" {
                "interrupted"
            } else {
                "error"
            };
            let item_mut = &mut snapshot.collaborations[index];
            item_mut.status = terminal.into();
            item_mut.result = result.clone();
            item_mut.error = error.map(|error| truncate(error, MAX_RESULT_RETURN));
            item_mut.updated_at = finished;
            let detail = result
                .clone()
                .or_else(|| item_mut.error.clone())
                .unwrap_or_else(|| "No assistant result was produced.".into());
            snapshot.messages.push(Message {
                id: id(),
                task_id: item.from_task_id.clone(),
                role: "system".into(),
                text: format!("Collaboration {terminal} from {}: {detail}", item.id),
                created_at: finished,
                sender_agent_id: Some(item.to_agent_id.clone()),
                collaboration_id: Some(item.id.clone()),
                attachments: vec![],
            });
            snapshot.events.push(RunEvent {
                id: id(),
                task_id: item.from_task_id,
                kind: "collaboration".into(),
                title: format!("Collaboration {terminal}"),
                detail: item.id,
                created_at: finished,
            });
        }
    }

    pub(crate) fn recover_collaborations(snapshot: &mut Snapshot) {
        for item in &mut snapshot.collaborations {
            if item.status == "running" {
                item.status = "interrupted".into();
                item.error = Some(DELIVERY_UNCERTAIN.into());
                item.updated_at = now();
            }
        }
    }

    pub(crate) fn cancel_collaboration_children(&self, task_id: &str) {
        let children = self
            .data
            .lock()
            .ok()
            .map(|data| collaboration_descendants(&data.snapshot, task_id))
            .unwrap_or_default();
        for child in children.into_iter().take(MAX_PER_ROOT) {
            let _ = self.cancel(&child);
        }
        let _ = self.mutate(None, |snapshot| {
            let descendants = collaboration_descendants(snapshot, task_id);
            for item in &mut snapshot.collaborations {
                if item.from_task_id == task_id
                    || descendants.iter().any(|child| child == &item.from_task_id)
                {
                    if matches!(item.status.as_str(), "queued" | "running") {
                        item.status = "interrupted".into();
                        item.error = Some(
                            "Cancelled because the originating collaboration was cancelled.".into(),
                        );
                        item.updated_at = now();
                    }
                }
            }
            Ok(())
        });
    }

    fn get_result_protocol(
        &self,
        caller_task: &str,
        collaboration_id: String,
    ) -> Result<Value, String> {
        let item = self.read_owned_collaboration(caller_task, &collaboration_id)?;
        self.result_with_inbox(caller_task, &item)
    }

    fn result_with_inbox(&self, caller_task: &str, item: &Collaboration) -> Result<Value, String> {
        let mut value = collaboration_value(item, true);
        value["incoming_messages"] = Value::Array(self.incoming_messages(caller_task)?);
        Ok(value)
    }

    /// Acknowledge only queued peer messages for this already-running recipient. This avoids a
    /// duplicate follow-up harness run while retaining the durable collaboration record.
    fn incoming_messages(&self, caller_task: &str) -> Result<Vec<Value>, String> {
        self.mutate(Some(caller_task.into()), |snapshot| {
            let running = snapshot.tasks.iter().any(|task| task.id == caller_task && task.status == "running");
            if !running { return Err("Collaboration is available only while this task is running.".into()); }
            let queued = snapshot.collaborations.iter().enumerate().filter(|(_, item)| {
                item.to_task_id == caller_task && item.kind == "message" && item.status == "queued"
            }).map(|(index, _)| index).collect::<Vec<_>>();
            let newly_delivered = queued.iter().map(|index| snapshot.collaborations[*index].id.clone()).collect::<Vec<_>>();
            for index in queued {
                let item = snapshot.collaborations[index].clone();
                snapshot.collaborations[index].status = "completed".into();
                snapshot.collaborations[index].result = Some(DELIVERED_TO_ACTIVE_TURN.into());
                snapshot.collaborations[index].updated_at = now();
                if !snapshot.messages.iter().any(|message| message.collaboration_id.as_deref() == Some(item.id.as_str())) {
                    snapshot.messages.push(Message { id:id(), task_id:caller_task.into(), role:"user".into(), text:item.text.clone(), created_at:now(), sender_agent_id:Some(item.from_agent_id.clone()), collaboration_id:Some(item.id.clone()), attachments:vec![] });
                }
            }
            Ok(snapshot.collaborations.iter().filter(|item| {
                item.to_task_id == caller_task && item.kind == "message" && item.result.as_deref() == Some(DELIVERED_TO_ACTIVE_TURN)
            }).rev().take(MAX_LIST_MESSAGES).map(|item| json!({
                "collaboration_id":item.id, "from_agent_id":item.from_agent_id, "from_task_id":item.from_task_id,
                "text":truncate(&item.text, MAX_MESSAGE_RETURN), "created_at":item.created_at,
                "delivery_state":"delivered_to_active_turn",
                "newly_delivered":newly_delivered.iter().any(|id| id == &item.id),
            })).collect())
        })
    }

    fn wait_protocol(
        &self,
        caller_task: &str,
        args: &serde_json::Map<String, Value>,
    ) -> Result<Value, String> {
        let collaboration_id = required_string(args, "collaboration_id")?;
        let seconds = args
            .get("timeout_seconds")
            .and_then(Value::as_f64)
            .unwrap_or(20.0)
            .clamp(1.0, 20.0);
        let deadline = Instant::now() + Duration::from_millis((seconds * 1000.0) as u64);
        loop {
            self.require_collaboration_caller(caller_task)?;
            let item = self.read_owned_collaboration(caller_task, &collaboration_id)?;
            let response = self.result_with_inbox(caller_task, &item)?;
            let has_incoming = response["incoming_messages"]
                .as_array()
                .is_some_and(|messages| {
                    messages
                        .iter()
                        .any(|message| message["newly_delivered"].as_bool() == Some(true))
                });
            if !matches!(item.status.as_str(), "queued" | "running")
                || has_incoming
                || Instant::now() >= deadline
            {
                return Ok(response);
            }
            thread::sleep(Duration::from_millis(100));
        }
    }

    fn list_messages_protocol(&self, caller_task: &str) -> Result<Value, String> {
        let _ = self.incoming_messages(caller_task)?;
        let data = self
            .data
            .lock()
            .map_err(|_| "Monitter state lock failed.".to_string())?;
        let records = data
            .snapshot
            .collaborations
            .iter()
            .filter(|item| item.from_task_id == caller_task || item.to_task_id == caller_task)
            .rev()
            .take(MAX_LIST_MESSAGES)
            .map(collaboration_message_value)
            .collect::<Vec<_>>();
        Ok(json!({"messages": records}))
    }

    fn cancel_protocol(
        &self,
        caller_task: &str,
        collaboration_id: String,
    ) -> Result<Value, String> {
        let (target, cancel_child) = self.mutate(None, |snapshot| {
            collaboration_caller(snapshot, caller_task)?;
            let item = snapshot
                .collaborations
                .iter_mut()
                .find(|item| item.id == collaboration_id && item.from_task_id == caller_task)
                .ok_or_else(|| {
                    "Collaboration was not found or is not owned by this task.".to_string()
                })?;
            if item.kind != "delegation" {
                return Err("Only delegations can be cancelled.".into());
            }
            let was_running = item.status == "running";
            if matches!(item.status.as_str(), "queued" | "running") {
                item.status = "interrupted".into();
                item.error = Some("Cancelled by the originating agent.".into());
                item.updated_at = now();
            }
            let target = item.to_task_id.clone();
            let child = was_running
                && snapshot.tasks.iter().any(|task| {
                    task.id == target
                        && task.parent_task_id.as_deref() == Some(caller_task)
                        && task.status == "running"
                });
            Ok((item.clone(), child))
        })?;
        if cancel_child {
            let _ = self.cancel(&target.to_task_id);
        }
        Ok(collaboration_value(&target, true))
    }

    fn read_owned_collaboration(
        &self,
        caller_task: &str,
        collaboration_id: &str,
    ) -> Result<Collaboration, String> {
        let data = self
            .data
            .lock()
            .map_err(|_| "Monitter state lock failed.".to_string())?;
        data.snapshot
            .collaborations
            .iter()
            .find(|item| {
                item.id == collaboration_id
                    && (item.from_task_id == caller_task || item.to_task_id == caller_task)
            })
            .cloned()
            .ok_or_else(|| "Collaboration was not found or is not available to this task.".into())
    }
}

fn fail_queued_delivery(snapshot: &mut Snapshot, index: usize, error: &str) {
    let item = snapshot.collaborations[index].clone();
    let time = now();
    snapshot.collaborations[index].status = "error".into();
    snapshot.collaborations[index].error = Some(error.into());
    snapshot.collaborations[index].updated_at = time;
    snapshot.messages.push(Message {
        id: id(),
        task_id: item.from_task_id.clone(),
        role: "system".into(),
        text: format!("Collaboration error from {}: {error}", item.id),
        created_at: time,
        sender_agent_id: Some(item.to_agent_id.clone()),
        collaboration_id: Some(item.id.clone()),
        attachments: vec![],
    });
    snapshot.events.push(RunEvent {
        id: id(),
        task_id: item.from_task_id,
        kind: "collaboration".into(),
        title: "Collaboration error".into(),
        detail: item.id,
        created_at: time,
    });
}
fn validate_tool_args(tool: &str, args: &serde_json::Map<String, Value>) -> Result<(), String> {
    let allowed: &[&str] = match tool {
        "list_agents" => &["query"],
        "delegate_task" => &["to_agent_id", "title", "message", "request_id"],
        "send_message" => &["to_agent_id", "message", "request_id", "task_id"],
        "get_task_result" | "cancel_delegation" => &["collaboration_id"],
        "wait_for_task" => &["collaboration_id", "timeout_seconds"],
        "list_messages" => &[],
        _ => return Err("Unknown Monitter collaboration tool.".into()),
    };
    if args.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err("Tool arguments include unsupported fields.".into());
    }
    if let Some(timeout) = args.get("timeout_seconds") {
        if !timeout.as_f64().is_some_and(f64::is_finite) {
            return Err("timeout_seconds must be a finite number.".into());
        }
    }
    Ok(())
}
fn required_string(args: &serde_json::Map<String, Value>, name: &str) -> Result<String, String> {
    args.get(name)
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("{name} is required."))
}
fn string_arg<'a>(
    args: &'a serde_json::Map<String, Value>,
    name: &str,
) -> Result<Option<&'a str>, String> {
    match args.get(name) {
        None => Ok(None),
        Some(Value::String(value)) => Ok(Some(value)),
        _ => Err(format!("{name} must be a string.")),
    }
}
fn validate_request(to_agent: &str, title: &str, text: &str, request: &str) -> Result<(), String> {
    if to_agent.trim().is_empty()
        || request.trim().is_empty()
        || request.len() > MAX_REQUEST_ID
        || text.trim().is_empty()
        || text.len() > MAX_TEXT
        || title.trim().is_empty()
        || title.len() > MAX_TITLE
    {
        return Err("Collaboration request has invalid or oversized fields.".into());
    }
    Ok(())
}
fn collaboration_caller(snapshot: &Snapshot, task_id: &str) -> Result<Task, String> {
    let task = snapshot
        .tasks
        .iter()
        .find(|task| task.id == task_id)
        .cloned()
        .ok_or_else(|| "Collaboration caller task was not found.".to_string())?;
    let enabled = snapshot
        .agents
        .iter()
        .any(|agent| agent.id == task.agent_id && agent.collaboration_enabled);
    if task.status != "running" || !enabled {
        return Err("Collaboration caller is not eligible.".into());
    }
    Ok(task)
}
fn directly_connected(snapshot: &Snapshot, first: &str, second: &str) -> bool {
    snapshot.collaborations.iter().any(|item| {
        (item.from_task_id == first && item.to_task_id == second)
            || (item.from_task_id == second && item.to_task_id == first)
    }) || snapshot
        .tasks
        .iter()
        .any(|task| task.id == second && task.parent_task_id.as_deref() == Some(first))
}
fn root_task(snapshot: &Snapshot, task_id: &str) -> String {
    let mut current = task_id;
    for _ in 0..MAX_PER_ROOT {
        let Some(parent) = snapshot
            .tasks
            .iter()
            .find(|task| task.id == current)
            .and_then(|task| task.parent_task_id.as_deref())
        else {
            break;
        };
        current = parent;
    }
    current.to_string()
}
fn depth(snapshot: &Snapshot, task_id: &str) -> usize {
    let mut count = 0;
    let mut current = task_id;
    while let Some(parent) = snapshot
        .tasks
        .iter()
        .find(|task| task.id == current)
        .and_then(|task| task.parent_task_id.as_deref())
    {
        count += 1;
        current = parent;
        if count > MAX_DEPTH {
            break;
        }
    }
    count
}
fn enforce_budget(snapshot: &Snapshot, task_id: &str) -> Result<(), String> {
    if snapshot
        .collaborations
        .iter()
        .filter(|item| item.from_task_id == task_id)
        .count()
        >= MAX_PER_TASK
    {
        return Err("This task has reached its collaboration request limit.".into());
    }
    let root = root_task(snapshot, task_id);
    if snapshot
        .collaborations
        .iter()
        .filter(|item| root_task(snapshot, &item.from_task_id) == root)
        .count()
        >= MAX_PER_ROOT
    {
        return Err("This collaboration chain has reached its request limit.".into());
    }
    if depth(snapshot, task_id) >= MAX_DEPTH {
        return Err("This collaboration chain has reached its maximum depth.".into());
    }
    Ok(())
}
fn is_agent_ancestor(snapshot: &Snapshot, task_id: &str, agent_id: &str) -> bool {
    let mut current = Some(task_id);
    while let Some(task_id) = current {
        let Some(task) = snapshot.tasks.iter().find(|task| task.id == task_id) else {
            break;
        };
        if task.agent_id == agent_id {
            return true;
        }
        current = task.parent_task_id.as_deref();
    }
    false
}
fn collaboration_descendants(snapshot: &Snapshot, root: &str) -> Vec<String> {
    let mut output = Vec::new();
    let mut cursor = vec![root.to_string()];
    while let Some(parent) = cursor.pop() {
        for item in snapshot
            .collaborations
            .iter()
            .filter(|item| item.from_task_id == parent && item.kind == "delegation")
        {
            if output.len() >= MAX_PER_ROOT {
                return output;
            }
            if !output.contains(&item.to_task_id) {
                output.push(item.to_task_id.clone());
                cursor.push(item.to_task_id.clone());
            }
        }
    }
    output
}
fn profile_matches(agent: &Agent, needle: &str) -> bool {
    needle.is_empty()
        || [agent.name.as_str(), agent.description.as_str()]
            .into_iter()
            .chain(agent.expertise.iter().map(String::as_str))
            .chain(agent.responsibilities.iter().map(String::as_str))
            .chain(agent.skills.iter().map(String::as_str))
            .any(|value| value.to_lowercase().contains(needle))
}
fn truncate(value: &str, maximum: usize) -> String {
    if value.len() <= maximum {
        return value.into();
    }
    let mut end = 0;
    for (index, character) in value.char_indices() {
        if index + character.len_utf8() > maximum {
            break;
        }
        end = index + character.len_utf8();
    }
    value[..end].into()
}
fn compact_entries(entries: &[String]) -> Vec<String> {
    entries
        .iter()
        .take(4)
        .map(|entry| truncate(entry, 80))
        .collect()
}
fn collaboration_message_value(item: &Collaboration) -> Value {
    let mut value = collaboration_value(item, false);
    value["result"] = json!(item
        .result
        .as_deref()
        .map(|result| truncate(result, MAX_MESSAGE_RETURN)));
    value["error"] = json!(item
        .error
        .as_deref()
        .map(|error| truncate(error, MAX_MESSAGE_RETURN)));
    if item.result.as_deref() == Some(DELIVERED_TO_ACTIVE_TURN) {
        value["delivery_state"] = json!("delivered_to_active_turn");
    }
    value
}
fn collaboration_value(item: &Collaboration, include_result: bool) -> Value {
    let mut value = json!({"collaboration_id":item.id,"id":item.id,"kind":item.kind,"to_agent_id":item.to_agent_id,"to_task_id":item.to_task_id,"status":item.status,"created_at":item.created_at,"updated_at":item.updated_at});
    if include_result {
        value["result"] = json!(item
            .result
            .as_deref()
            .map(|result| truncate(result, MAX_RESULT_RETURN)));
        value["error"] = json!(item
            .error
            .as_deref()
            .map(|error| truncate(error, MAX_RESULT_RETURN)));
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Project, ProjectWorkspace};
    use std::{fs, path::PathBuf};

    fn temp_dir(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("monitter-collaboration-{label}-{}", id()))
    }
    fn service(label: &str) -> (Arc<Service>, PathBuf) {
        let dir = temp_dir(label);
        (Service::open(None, dir.clone()).unwrap(), dir)
    }
    fn add_agent(service: &Arc<Service>, name: &str) -> Agent {
        service
            .mutate(None, |snapshot| {
                let mut agent = snapshot.agents[0].clone();
                agent.id = id();
                agent.name = name.into();
                agent.description = format!("{name} profile");
                agent.expertise = vec!["Rust routing".into()];
                agent.responsibilities = vec!["Reviews".into()];
                agent.skills = vec!["testing".into()];
                snapshot.agents.push(agent.clone());
                Ok(agent)
            })
            .unwrap()
    }
    fn task(
        service: &Arc<Service>,
        agent: &Agent,
        title: &str,
        parent: Option<String>,
        project: Option<String>,
    ) -> Task {
        service
            .create_task(CreateTaskInput {
                agent_id: agent.id.clone(),
                title: title.into(),
                native_session_id: None,
                parent_task_id: parent,
                channel_id: None,
                project_id: project,
                model_settings: None,
                sandbox: None,
            })
            .unwrap()
    }
    fn running(service: &Arc<Service>, task_id: &str) {
        service
            .mutate(None, |snapshot| {
                let task = snapshot
                    .tasks
                    .iter_mut()
                    .find(|task| task.id == task_id)
                    .unwrap();
                task.status = "running".into();
                Ok(())
            })
            .unwrap();
    }
    fn delegate_args(agent: &Agent, request: &str, body: &str) -> Value {
        json!({"to_agent_id":agent.id,"title":"Review this","message":body,"request_id":request})
    }

    #[test]
    fn protocol_is_idempotent_and_rejects_mutated_or_unknown_requests() {
        let (service, dir) = service("idempotent");
        let recipient = add_agent(&service, "Reviewer");
        let caller = service.snapshot().unwrap().agents[0].clone();
        let root = task(&service, &caller, "Root", None, None);
        running(&service, &root.id);
        let first = service
            .protocol(
                &root.id,
                "delegate_task",
                delegate_args(&recipient, "r1", "check this"),
            )
            .unwrap();
        let duplicate = service
            .protocol(
                &root.id,
                "delegate_task",
                delegate_args(&recipient, "r1", "check this"),
            )
            .unwrap();
        assert_eq!(first["collaboration_id"], duplicate["collaboration_id"]);
        assert_eq!(service.snapshot().unwrap().collaborations.len(), 1);
        assert!(service
            .protocol(
                &root.id,
                "delegate_task",
                delegate_args(&recipient, "r1", "changed")
            )
            .is_err());
        assert!(service
            .protocol(&root.id, "list_agents", json!({"unexpected":true}))
            .is_err());
        assert!(service
            .protocol(
                &root.id,
                "wait_for_task",
                json!({"collaboration_id":first["collaboration_id"],"timeout_seconds":"no"})
            )
            .is_err());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn directory_hides_disabled_agents_and_delegation_uses_project_workspace() {
        let (service, dir) = service("project");
        let recipient = add_agent(&service, "Project reviewer");
        let caller = service.snapshot().unwrap().agents[0].clone();
        let host = service.snapshot().unwrap().hosts[0].clone();
        service
            .mutate(None, |snapshot| {
                snapshot.projects.push(Project {
                    id: "p".into(),
                    name: "Project".into(),
                    description: String::new(),
                    workspaces: vec![ProjectWorkspace {
                        host_id: host.id.clone(),
                        cwd: "/project-folder".into(),
                    }],
                });
                Ok(())
            })
            .unwrap();
        let root = task(&service, &caller, "Root", None, Some("p".into()));
        running(&service, &root.id);
        service
            .mutate(None, |snapshot| {
                snapshot
                    .agents
                    .iter_mut()
                    .find(|agent| agent.id == recipient.id)
                    .unwrap()
                    .collaboration_enabled = false;
                Ok(())
            })
            .unwrap();
        let directory = service
            .protocol(&root.id, "list_agents", json!({"query":"review"}))
            .unwrap();
        assert_eq!(directory["agents"].as_array().unwrap().len(), 0);
        assert!(service
            .protocol(
                &root.id,
                "delegate_task",
                delegate_args(&recipient, "off", "body")
            )
            .is_err());
        service
            .mutate(None, |snapshot| {
                snapshot
                    .agents
                    .iter_mut()
                    .find(|agent| agent.id == recipient.id)
                    .unwrap()
                    .collaboration_enabled = true;
                Ok(())
            })
            .unwrap();
        let record = service
            .protocol(
                &root.id,
                "delegate_task",
                delegate_args(&recipient, "project", "body"),
            )
            .unwrap();
        let created = service
            .snapshot()
            .unwrap()
            .tasks
            .into_iter()
            .find(|task| task.id == record["to_task_id"].as_str().unwrap())
            .unwrap();
        assert_eq!(created.project_id.as_deref(), Some("p"));
        assert_eq!(created.cwd, "/project-folder");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn busy_queue_head_does_not_block_an_independent_delivery() {
        let (service, dir) = service("queue");
        let first = add_agent(&service, "Busy");
        let second = add_agent(&service, "Ready");
        let caller = service.snapshot().unwrap().agents[0].clone();
        let root = task(&service, &caller, "Root", None, None);
        running(&service, &root.id);
        let one = service
            .protocol(
                &root.id,
                "delegate_task",
                delegate_args(&first, "one", "one"),
            )
            .unwrap();
        service
            .mutate(None, |snapshot| {
                snapshot
                    .tasks
                    .iter_mut()
                    .find(|task| task.id == one["to_task_id"].as_str().unwrap())
                    .unwrap()
                    .status = "running".into();
                Ok(())
            })
            .unwrap();
        let two = service
            .protocol(
                &root.id,
                "delegate_task",
                delegate_args(&second, "two", "two"),
            )
            .unwrap();
        let next = service.prepare_next_collaboration().unwrap().unwrap();
        assert_eq!(next.0, two["to_task_id"].as_str().unwrap());
        let snapshot = service.snapshot().unwrap();
        assert_eq!(
            snapshot
                .collaborations
                .iter()
                .find(|item| item.id == one["collaboration_id"].as_str().unwrap())
                .unwrap()
                .status,
            "queued"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn child_can_message_running_parent_and_cancel_is_delegation_only() {
        let (service, dir) = service("mailbox");
        let child_agent = add_agent(&service, "Child");
        let caller = service.snapshot().unwrap().agents[0].clone();
        let root = task(&service, &caller, "Root", None, None);
        running(&service, &root.id);
        let delegation = service
            .protocol(
                &root.id,
                "delegate_task",
                delegate_args(&child_agent, "delegate", "work"),
            )
            .unwrap();
        let child_id = delegation["to_task_id"].as_str().unwrap().to_string();
        running(&service, &child_id);
        let message=service.protocol(&child_id,"send_message",json!({"to_agent_id":caller.id,"task_id":root.id,"message":"need input","request_id":"peer"})).unwrap();
        assert!(service
            .protocol(
                &child_id,
                "cancel_delegation",
                json!({"collaboration_id":message["collaboration_id"]})
            )
            .is_err());
        let mailbox = service
            .protocol(&child_id, "list_messages", json!({}))
            .unwrap();
        assert!(mailbox["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["collaboration_id"] == message["collaboration_id"]));
        let cancelled = service
            .protocol(
                &root.id,
                "cancel_delegation",
                json!({"collaboration_id":delegation["collaboration_id"]}),
            )
            .unwrap();
        assert_eq!(cancelled["status"], "interrupted");
        running(&service, &child_id); // a later independent turn must survive a repeated cancel request.
        let _ = service
            .protocol(
                &root.id,
                "cancel_delegation",
                json!({"collaboration_id":delegation["collaboration_id"]}),
            )
            .unwrap();
        assert_eq!(
            service
                .snapshot()
                .unwrap()
                .tasks
                .iter()
                .find(|task| task.id == child_id)
                .unwrap()
                .status,
            "running"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn completion_uses_delivery_marker_and_recovery_preserves_queued_work() {
        let dir = temp_dir("recovery");
        let (store, mut snapshot, hosts, attachments) =
            crate::store::Store::open(dir.clone()).unwrap();
        snapshot.collaborations = vec![
            Collaboration {
                id: "run".into(),
                kind: "delegation".into(),
                from_agent_id: "a".into(),
                from_task_id: "source".into(),
                to_agent_id: "b".into(),
                to_task_id: "target".into(),
                text: "brief".into(),
                request_id: "run".into(),
                status: "running".into(),
                result: None,
                error: None,
                created_at: 1,
                updated_at: 1,
            },
            Collaboration {
                id: "queued".into(),
                kind: "delegation".into(),
                from_agent_id: "a".into(),
                from_task_id: "source".into(),
                to_agent_id: "b".into(),
                to_task_id: "later".into(),
                text: "brief".into(),
                request_id: "queued".into(),
                status: "queued".into(),
                result: None,
                error: None,
                created_at: 1,
                updated_at: 1,
            },
        ];
        snapshot.messages.push(Message {
            id: "old".into(),
            task_id: "target".into(),
            role: "assistant".into(),
            text: "old reply".into(),
            created_at: 1,
            sender_agent_id: None,
            collaboration_id: None,
            attachments: vec![],
        });
        snapshot.messages.push(Message {
            id: "marker".into(),
            task_id: "target".into(),
            role: "user".into(),
            text: "peer".into(),
            created_at: 2,
            sender_agent_id: Some("a".into()),
            collaboration_id: Some("run".into()),
            attachments: vec![],
        });
        snapshot.messages.push(Message {
            id: "new".into(),
            task_id: "target".into(),
            role: "assistant".into(),
            text: "new reply".into(),
            created_at: 3,
            sender_agent_id: None,
            collaboration_id: None,
            attachments: vec![],
        });
        Service::complete_collaborations(&mut snapshot, "target", "completed", None);
        assert_eq!(
            snapshot.collaborations[0].result.as_deref(),
            Some("new reply")
        );
        snapshot.collaborations[0].status = "running".into();
        snapshot.collaborations[0].result = None;
        store.save(&snapshot, &hosts, &attachments).unwrap();
        drop(store);
        let (_, recovered, _, _) = crate::store::Store::open(dir.clone()).unwrap();
        assert_eq!(recovered.collaborations[0].status, "interrupted");
        assert_eq!(recovered.collaborations[1].status, "queued");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn inbox_acknowledges_queued_peer_message_without_second_delivery_turn() {
        let (service, dir) = service("inbox-ack");
        let sender = add_agent(&service, "Sender");
        let receiver = service.snapshot().unwrap().agents[0].clone();
        let receiver_task = task(&service, &receiver, "Active", None, None);
        running(&service, &receiver_task.id);
        let sender_task = task(&service, &sender, "Sender turn", None, None);
        running(&service, &sender_task.id);
        service
            .mutate(None, |snapshot| {
                snapshot.collaborations.push(Collaboration {
                    id: "mail".into(),
                    kind: "message".into(),
                    from_agent_id: sender.id.clone(),
                    from_task_id: sender_task.id.clone(),
                    to_agent_id: receiver.id.clone(),
                    to_task_id: receiver_task.id.clone(),
                    text: "question".into(),
                    request_id: "mail".into(),
                    status: "queued".into(),
                    result: None,
                    error: None,
                    created_at: now(),
                    updated_at: now(),
                });
                Ok(())
            })
            .unwrap();
        let listed = service
            .protocol(&receiver_task.id, "list_messages", json!({}))
            .unwrap();
        assert_eq!(listed["messages"][0]["status"], "completed");
        assert_eq!(listed["messages"][0]["result"], DELIVERED_TO_ACTIVE_TURN);
        let inbox = service.incoming_messages(&receiver_task.id).unwrap();
        assert_eq!(inbox[0]["from_task_id"], sender_task.id);
        assert_eq!(inbox[0]["newly_delivered"], false);
        assert_eq!(
            service
                .snapshot()
                .unwrap()
                .messages
                .iter()
                .filter(|message| message.collaboration_id.as_deref() == Some("mail"))
                .count(),
            1
        );
        assert!(!service.has_dispatchable_collaboration());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn chain_limits_cycles_and_response_text_are_bounded() {
        let mut snapshot = crate::model::default_snapshot();
        let agent = snapshot.agents[0].clone();
        let root = crate::model::task_from_agent(
            &agent,
            &CreateTaskInput {
                agent_id: agent.id.clone(),
                title: "root".into(),
                native_session_id: None,
                parent_task_id: None,
                channel_id: None,
                project_id: None,
                model_settings: None,
                sandbox: None,
            },
        );
        snapshot.tasks.push(root.clone());
        let mut parent = root.id.clone();
        for index in 0..MAX_DEPTH {
            let mut child = crate::model::task_from_agent(
                &agent,
                &CreateTaskInput {
                    agent_id: agent.id.clone(),
                    title: index.to_string(),
                    native_session_id: None,
                    parent_task_id: Some(parent.clone()),
                    channel_id: None,
                    project_id: None,
                    model_settings: None,
                    sandbox: None,
                },
            );
            child.agent_id = format!("agent-{index}");
            parent = child.id.clone();
            snapshot.tasks.push(child);
        }
        assert_eq!(depth(&snapshot, &parent), MAX_DEPTH);
        assert!(is_agent_ancestor(&snapshot, &parent, &agent.id));
        assert!(enforce_budget(&snapshot, &parent).is_err());
        for index in 0..MAX_PER_TASK {
            snapshot.collaborations.push(Collaboration {
                id: index.to_string(),
                kind: "delegation".into(),
                from_agent_id: "a".into(),
                from_task_id: root.id.clone(),
                to_agent_id: "b".into(),
                to_task_id: "x".into(),
                text: String::new(),
                request_id: index.to_string(),
                status: "queued".into(),
                result: None,
                error: None,
                created_at: 0,
                updated_at: 0,
            });
        }
        assert!(enforce_budget(&snapshot, &root.id).is_err());
        let crab = "🦀".repeat(1_000);
        assert!(truncate(&crab, 999).len() <= 999);
        assert_eq!(compact_entries(&vec!["x".repeat(1_000); 12]).len(), 4);
    }
}
