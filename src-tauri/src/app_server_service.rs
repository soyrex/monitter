//! Durable, ownership-checked boundary for local and SSH Codex app-server transports.
use crate::{
    model::*,
    runner::{Parsed, RunControl},
    *,
};
use serde_json::Value;
use std::sync::{Arc, Weak};

impl Service {
    fn app_server_mutate<R>(
        &self,
        task_id: &str,
        control: &Arc<RunControl>,
        turn_id: Option<&str>,
        f: impl FnOnce(&mut ServiceData, &mut RunRegistry) -> Result<R, String>,
    ) -> Result<R, String> {
        self.mutate_data(Some(task_id.into()), |data| {
            let mut runs = self.runs.lock().map_err(|_| "Run registry unavailable")?;
            if control.is_cancelled()
                || turn_id.is_some_and(|turn| !control.matches_app_server_turn(turn))
                || !runs
                    .tasks
                    .get(task_id)
                    .is_some_and(|current| Arc::ptr_eq(current, control))
                || !data
                    .snapshot
                    .tasks
                    .iter()
                    .any(|task| task.id == task_id && task.status == "running")
            {
                return Err("Codex turn no longer owns this chat.".into());
            }
            f(data, &mut runs)
        })
    }

    pub(crate) fn app_server_message(
        &self,
        task_id: &str,
        control: &Arc<RunControl>,
        turn_id: &str,
        item_id: &str,
        text: &str,
        complete: bool,
    ) -> Result<(), String> {
        if text.len() > 2 * 1024 * 1024 || item_id.len() > 512 {
            return Err("Codex reply exceeds the supported size.".into());
        }
        self.app_server_mutate(task_id, control, Some(turn_id), |data, _| {
            // Keep provider identifiers runtime-only. Persist normal local UUIDs,
            // including in messages projected to shared visitors.
            let message_id = {
                let mut ids = self
                    .app_server_message_ids
                    .lock()
                    .map_err(|_| "Message identities unavailable")?;
                let key = (task_id.into(), turn_id.into(), item_id.into());
                if !ids.contains_key(&key)
                    && ids.keys().filter(|(task, _, _)| task == task_id).count() >= 1024
                {
                    return Err("Too many message items in one Codex turn.".into());
                }
                ids.entry(key).or_insert_with(id).clone()
            };
            if data
                .snapshot
                .messages
                .iter()
                .any(|m| m.id == message_id && m.stream_status.as_deref() == Some("complete"))
            {
                return Ok(());
            }
            let images = if complete {
                self.pending_codex_images
                    .lock()
                    .map_err(|_| "Image state unavailable")?
                    .get(task_id)
                    .cloned()
                    .unwrap_or_default()
            } else {
                vec![]
            };
            let message = if let Some(message) = data
                .snapshot
                .messages
                .iter_mut()
                .find(|m| m.id == message_id)
            {
                message.text = text.into();
                message.stream_status =
                    Some(if complete { "complete" } else { "streaming" }.into());
                if complete {
                    message.attachments = images;
                }
                message.clone()
            } else {
                let message = Message {
                    id: message_id,
                    task_id: task_id.into(),
                    role: "assistant".into(),
                    text: text.into(),
                    created_at: now(),
                    sender_agent_id: None,
                    collaboration_id: None,
                    attachments: images,
                    stream_status: Some(if complete { "complete" } else { "streaming" }.into()),
                };
                data.snapshot.messages.push(message.clone());
                message
            };
            let task = data
                .snapshot
                .tasks
                .iter_mut()
                .find(|task| task.id == task_id)
                .ok_or("Task not found")?;
            task.updated_at = now();
            if complete && !data.blocked_channel_deliveries.contains(task_id) {
                if let Some(channel) = data.snapshot.channels.iter_mut().find(|channel| {
                    Some(&channel.id) == task.channel_id.as_ref()
                        && channel.agent_ids.contains(&task.agent_id)
                }) {
                    if !channel.messages.iter().any(|m| m.id == message.id) {
                        channel.messages.push(ChannelMessage {
                            id: message.id,
                            role: "assistant".into(),
                            agent_id: Some(task.agent_id.clone()),
                            text: message.text,
                            created_at: message.created_at,
                            task_id: Some(task_id.into()),
                        });
                    }
                }
            }
            Ok(())
        })?;
        if complete {
            self.pending_codex_images
                .lock()
                .map_err(|_| "Image state unavailable")?
                .remove(task_id);
        }
        Ok(())
    }

    pub(crate) fn app_server_event(
        &self,
        task_id: &str,
        control: &Arc<RunControl>,
        turn_id: Option<&str>,
        parsed: Parsed,
    ) -> Result<(), String> {
        self.app_server_mutate(task_id, control, turn_id, |data, runs| {
            let task = data
                .snapshot
                .tasks
                .iter_mut()
                .find(|task| task.id == task_id)
                .ok_or("Task not found")?;
            if let Some(native) = parsed.native_session_id {
                if task
                    .native_session_id
                    .as_ref()
                    .is_some_and(|existing| existing != &native)
                {
                    return Err("Codex changed this chat's native thread ID.".into());
                }
                let host = data.task_hosts.get(task_id).ok_or("Task host not found")?;
                let key = native_session_key(task, host, &native);
                if runs
                    .native_sessions
                    .get(&key)
                    .is_some_and(|owner| owner != task_id)
                {
                    return Err("Another chat owns this Codex thread.".into());
                }
                runs.native_sessions.insert(key, task_id.into());
                task.native_session_id = Some(native);
            }
            if let Some((kind, title, detail)) = parsed.event {
                if kind == "computer_image" {
                    let image = attachments::generated_image(&detail)?;
                    let mut pending = self
                        .pending_codex_images
                        .lock()
                        .map_err(|_| "Image state unavailable")?;
                    let images = pending.entry(task_id.into()).or_default();
                    if images.len() >= 4 {
                        return Err("Too many pending Codex images.".into());
                    }
                    images.push(image);
                } else {
                    data.snapshot.events.push(RunEvent {
                        id: id(),
                        task_id: task_id.into(),
                        kind,
                        title,
                        detail,
                        created_at: now(),
                    });
                }
            }
            Ok(())
        })
    }

    pub(crate) fn complete_app_server_turn(
        self: &Arc<Self>,
        task_id: &str,
        control: &Arc<RunControl>,
        turn_id: Option<&str>,
        status: &str,
        error: Option<String>,
    ) -> bool {
        let result =
            self.app_server_mutate(task_id, control, turn_id, |data, _| {
                let task = data
                    .snapshot
                    .tasks
                    .iter_mut()
                    .find(|task| task.id == task_id)
                    .ok_or("Task not found")?;
                task.status = if status == "completed" {
                    "completed"
                } else if status == "interrupted" {
                    "interrupted"
                } else {
                    "error"
                }
                .into();
                task.updated_at = now();
                let final_status = task.status.clone();
                Service::complete_collaborations(
                    &mut data.snapshot,
                    task_id,
                    &final_status,
                    error.as_deref(),
                );
                control.clear_app_server_turn();
                self.pending_codex_images
                    .lock()
                    .map_err(|_| "Image state unavailable")?
                    .remove(task_id);
                self.app_server_message_ids
                    .lock()
                    .map_err(|_| "Message identities unavailable")?
                    .retain(|(task, _, _), _| task != task_id);
                for message in data.snapshot.messages.iter_mut().filter(|m| {
                    m.task_id == task_id && m.stream_status.as_deref() == Some("streaming")
                }) {
                    message.stream_status = Some("interrupted".into());
                }
                let expired = expire_inputs(&mut data.snapshot, task_id);
                if let Some(detail) = error {
                    data.snapshot.events.push(RunEvent {
                        id: id(),
                        task_id: task_id.into(),
                        kind: "error".into(),
                        title: "Codex turn failed".into(),
                        detail,
                        created_at: now(),
                    });
                }
                let routes = if status == "completed" {
                    match prepare_channel_mention_routes(data, task_id) {
                        Ok(routes) => routes,
                        Err(detail) => {
                            data.snapshot.events.push(RunEvent {
                                id: id(),
                                task_id: task_id.into(),
                                kind: "error".into(),
                                title: "Channel peer routing unavailable".into(),
                                detail,
                                created_at: now(),
                            });
                            vec![]
                        }
                    }
                } else {
                    vec![]
                };
                Ok((expired, routes))
            });
        let Ok((expired, routes)) = result else {
            return false;
        };
        for request in expired {
            self.notify_approval_waiters(&request, Err("Codex turn ended.".into()));
            self.notify_input_waiter(&request, Err("Codex turn ended.".into()));
        }
        if status == "completed" {
            self.dispatch_queued(task_id);
            for (target, _) in routes {
                self.dispatch_queued(&target);
            }
        }
        true
    }

    pub(crate) fn create_app_server_approval(
        &self,
        control: &Arc<RunControl>,
        turn_id: &str,
        input: CreateApprovalRequest,
        interaction: Option<InteractionInput>,
    ) -> Result<ApprovalRequest, String> {
        let task_id = input.task_id.clone();
        self.app_server_mutate(&task_id, control, Some(turn_id), |data, _| {
            if let Some(existing) = data
                .snapshot
                .approval_requests
                .iter()
                .find(|r| r.task_id == task_id && r.run_id == input.run_id)
            {
                return Ok(existing.clone());
            }
            let request = ApprovalRequest {
                id: id(),
                task_id: task_id.clone(),
                provider: "codex".into(),
                run_id: input.run_id,
                tool: input.tool,
                summary: input.summary,
                detail: input.detail,
                risk: input.risk,
                status: "pending".into(),
                created_at: now(),
                resolved_at: None,
                decision: None,
                input: interaction,
                response: None,
            };
            self.app_server_approvals
                .lock()
                .map_err(|_| "Approval ownership unavailable")?
                .insert(request.id.clone(), Arc::downgrade(control));
            data.snapshot.approval_requests.push(request.clone());
            Ok(request)
        })
    }

    /// Called while the durable state lock is held; mirrors data -> runs lock order.
    pub(crate) fn release_app_server_run(&self, task_id: &str, control: &Arc<RunControl>) {
        if let Ok(mut runs) = self.runs.lock() {
            if runs
                .tasks
                .get(task_id)
                .is_some_and(|current| Arc::ptr_eq(current, control))
            {
                // Revoke while still reserving the task so a new process cannot
                // acquire a grant that this old reader then revokes.
                self.revoke_collaboration_grant(task_id);
                runs.tasks.remove(task_id);
                runs.native_sessions.retain(|_, owner| owner != task_id);
                if let Ok(mut ids) = self.app_server_message_ids.lock() {
                    ids.retain(|(task, _, _), _| task != task_id);
                }
            }
        }
        if let Ok(mut owners) = self.app_server_approvals.lock() {
            owners.retain(|_, owner| {
                owner
                    .upgrade()
                    .is_some_and(|owner| !Arc::ptr_eq(&owner, control))
            });
        }
    }

    pub(crate) fn validate_app_server_approval(
        &self,
        request: &ApprovalRequest,
        snapshot: &Snapshot,
    ) -> Result<(), String> {
        if request.provider != "codex" {
            return Ok(());
        }
        let runs = self.runs.lock().map_err(|_| "Run registry unavailable")?;
        let owners = self
            .app_server_approvals
            .lock()
            .map_err(|_| "Approval ownership unavailable")?;
        let owner = owners
            .get(&request.id)
            .and_then(Weak::upgrade)
            .ok_or("This Codex request no longer has a live response channel.")?;
        if owner.is_cancelled()
            || !runs
                .tasks
                .get(&request.task_id)
                .is_some_and(|current| Arc::ptr_eq(current, &owner))
            || !snapshot
                .tasks
                .iter()
                .any(|task| task.id == request.task_id && task.status == "running")
        {
            return Err("This Codex request has expired.".into());
        }
        Ok(())
    }

    pub(crate) fn wait_for_input<F: Fn() -> bool>(
        &self,
        approval_id: &str,
        keep_waiting: F,
    ) -> Result<Value, String> {
        let (sender, receiver) = mpsc::channel();
        {
            let mut waiters = self
                .input_waiters
                .lock()
                .map_err(|_| "Input waiters unavailable")?;
            let data = self.data.lock().map_err(|_| "State unavailable")?;
            let request = data
                .snapshot
                .approval_requests
                .iter()
                .find(|r| r.id == approval_id)
                .ok_or("Input request not found")?;
            if request.status != "pending" {
                return request
                    .response
                    .clone()
                    .ok_or("Input request is no longer pending".into());
            }
            waiters.insert(approval_id.into(), sender);
        }
        let result = loop {
            if !keep_waiting() {
                break Err("Input request interrupted".into());
            }
            match receiver.recv_timeout(Duration::from_millis(250)) {
                Ok(result) => break result,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => break Err("Input channel closed".into()),
            }
        };
        self.input_waiters
            .lock()
            .ok()
            .map(|mut waiters| waiters.remove(approval_id));
        result
    }

    pub(crate) fn notify_input_waiter(&self, id: &str, result: Result<Value, String>) {
        if let Some(waiter) = self
            .input_waiters
            .lock()
            .ok()
            .and_then(|mut waiters| waiters.remove(id))
        {
            let _ = waiter.send(result);
        }
    }

    pub(crate) fn resolve_input_request(
        &self,
        approval_id: &str,
        response: Value,
    ) -> Result<Snapshot, String> {
        if response.to_string().len() > 64 * 1024 {
            return Err("Input response is too large.".into());
        }
        let snapshot = self.mutate(None, |snapshot| {
            let index = snapshot
                .approval_requests
                .iter()
                .position(|r| r.id == approval_id)
                .ok_or("Input request not found")?;
            let request = &snapshot.approval_requests[index];
            if request.status != "pending" {
                return Err("Input request is no longer pending.".into());
            }
            self.validate_app_server_approval(request, snapshot)?;
            let input = request
                .input
                .as_ref()
                .ok_or("This request needs an approval decision, not input.")?;
            validate_response(input, &response)?;
            let request = &mut snapshot.approval_requests[index];
            request.status = "approved".into();
            request.decision = Some("approve_once".into());
            request.response = Some(response.clone());
            request.resolved_at = Some(now());
            Ok(snapshot.clone())
        })?;
        self.notify_input_waiter(approval_id, Ok(response));
        Ok(snapshot)
    }
}

fn expire_inputs(snapshot: &mut Snapshot, task_id: &str) -> Vec<String> {
    snapshot
        .approval_requests
        .iter_mut()
        .filter(|r| r.task_id == task_id && r.status == "pending")
        .map(|request| {
            request.status = "expired".into();
            request.resolved_at = Some(now());
            request.id.clone()
        })
        .collect()
}

fn validate_response(input: &InteractionInput, response: &Value) -> Result<(), String> {
    match input.kind.as_str() {
        "questions" => {
            let answers = response
                .get("answers")
                .and_then(Value::as_object)
                .ok_or("Expected question answers")?;
            if answers.len() != input.questions.len() {
                return Err("Answer each question, without adding unknown questions.".into());
            }
            for question in &input.questions {
                let values = answers
                    .get(&question.id)
                    .and_then(|v| v.get("answers"))
                    .and_then(Value::as_array)
                    .ok_or("Missing question answer")?;
                if values.is_empty()
                    || values.iter().any(|value| {
                        value
                            .as_str()
                            .is_none_or(|text| text.trim().is_empty() || text.len() > 16_384)
                    })
                {
                    return Err("Each answer must contain text.".into());
                }
            }
            Ok(())
        }
        "form" => validate_form(
            input.schema.as_ref().ok_or("Missing form schema")?,
            response,
        ),
        "url" if response == &Value::Bool(true) => Ok(()),
        _ => Err("Unsupported input response.".into()),
    }
}

/// MCP elicitation supports flat primitive forms. Reject unsupported schemas, never
/// pretend a free-text blob meets a schema the client does not understand.
fn validate_form(schema: &Value, value: &Value) -> Result<(), String> {
    let values = value.as_object().ok_or("Expected a form object")?;
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .ok_or("Unsupported form schema")?;
    if values.keys().any(|key| !properties.contains_key(key)) {
        return Err("Unknown form field.".into());
    }
    for name in schema
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        if !values.contains_key(name) {
            return Err(format!("Missing required field: {name}"));
        }
    }
    for (name, value) in values {
        let field = &properties[name];
        let valid = match field.get("type").and_then(Value::as_str) {
            Some("string") => value.is_string(),
            Some("boolean") => value.is_boolean(),
            Some("integer") => value.is_i64() || value.is_u64(),
            Some("number") => value.is_number(),
            _ => false,
        };
        if !valid {
            return Err(format!("Invalid form field: {name}"));
        }
        if field
            .get("enum")
            .and_then(Value::as_array)
            .is_some_and(|options| !options.contains(value))
        {
            return Err(format!("Invalid choice: {name}"));
        }
    }
    Ok(())
}
