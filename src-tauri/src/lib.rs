mod adapters;
mod attachments;
mod collaboration;
mod collaboration_runtime;
mod collaboration_transport;
mod deletion;
mod git;
mod goals;
pub mod model;
mod runner;
mod store;

use model::*;
use runner::Parsed;
use std::{
    collections::HashMap,
    io::Read,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager, State};

#[derive(Clone)]
struct AppState(Arc<Service>);

#[derive(Clone, PartialEq, Eq)]
struct ServiceData {
    snapshot: Snapshot,
    task_hosts: HashMap<String, Host>,
    attachments: HashMap<String, attachments::StoredAttachment>,
}

#[derive(Default)]
struct RunRegistry {
    tasks: HashMap<String, Arc<runner::RunControl>>,
    native_sessions: HashMap<String, String>,
}

pub(crate) struct Service {
    app: Option<AppHandle>,
    store: store::Store,
    data: Mutex<ServiceData>,
    runs: Mutex<RunRegistry>,
    collaboration: Mutex<Option<collaboration_transport::Broker>>,
    collaboration_grants: Mutex<HashMap<String, collaboration_transport::SessionGrant>>,
    collaboration_started: std::sync::atomic::AtomicBool,
    stopping: std::sync::atomic::AtomicBool,
    runtime_dir: PathBuf,
}

fn native_session_key(task: &Task, host: &Host, native: &str) -> String {
    format!("{}:{}:{}", task.provider, host.id, native)
}

impl Service {
    fn open(app: Option<AppHandle>, dir: PathBuf) -> Result<Arc<Self>, String> {
        let (store, snapshot, task_hosts, attachments) = store::Store::open(dir.clone())?;
        Ok(Arc::new(Self {
            app,
            store,
            data: Mutex::new(ServiceData {
                snapshot,
                task_hosts,
                attachments,
            }),
            runs: Mutex::new(RunRegistry::default()),
            collaboration: Mutex::new(None),
            collaboration_grants: Mutex::new(HashMap::new()),
            collaboration_started: std::sync::atomic::AtomicBool::new(false),
            stopping: std::sync::atomic::AtomicBool::new(false),
            runtime_dir: dir.join("runtime"),
        }))
    }

    fn snapshot(&self) -> Result<Snapshot, String> {
        self.data
            .lock()
            .map_err(|_| "Monitter state lock failed.".to_string())
            .map(|data| data.snapshot.clone())
    }

    fn changed(&self, task_id: Option<String>) {
        if let Some(app) = &self.app {
            let _ = app.emit("monitter:changed", serde_json::json!({ "taskId": task_id }));
        }
    }

    fn mutate_data<R>(
        &self,
        task_id: Option<String>,
        f: impl FnOnce(&mut ServiceData) -> Result<R, String>,
    ) -> Result<R, String> {
        let (output, changed) = {
            let mut data = self
                .data
                .lock()
                .map_err(|_| "Monitter state lock failed.".to_string())?;
            let mut candidate = data.clone();
            let output = f(&mut candidate)?;
            let changed = candidate != *data;
            if changed {
                self.store.save(
                    &candidate.snapshot,
                    &candidate.task_hosts,
                    &candidate.attachments,
                )?;
                *data = candidate;
            }
            (output, changed)
        };
        if changed {
            self.changed(task_id);
        }
        Ok(output)
    }

    fn mutate<R>(
        &self,
        task_id: Option<String>,
        f: impl FnOnce(&mut Snapshot) -> Result<R, String>,
    ) -> Result<R, String> {
        self.mutate_data(task_id, |data| f(&mut data.snapshot))
    }

    fn task_and_host(&self, id: &str) -> Result<(Task, Host), String> {
        let data = self
            .data
            .lock()
            .map_err(|_| "Monitter state lock failed.".to_string())?;
        let task = data
            .snapshot
            .tasks
            .iter()
            .find(|task| task.id == id)
            .cloned()
            .ok_or_else(|| "Task was not found.".to_string())?;
        let host = data
            .task_hosts
            .get(id)
            .cloned()
            .ok_or_else(|| "Task's saved host settings were not found.".to_string())?;
        Ok((task, host))
    }

    fn store_attachment(
        &self,
        target: attachments::AttachmentTarget,
        filename: String,
        mime_type: String,
        data_base64: String,
        preview_data_url: Option<String>,
        source_id: Option<String>,
    ) -> Result<attachments::Attachment, String> {
        let (host, cwd) = {
            let data = self
                .data
                .lock()
                .map_err(|_| "Monitter state lock failed.")?;
            resolve_attachment_target(&data, &target)?
        };
        let attachment = attachments::store(
            &host,
            &cwd,
            &filename,
            &mime_type,
            &data_base64,
            preview_data_url,
            source_id,
        )?;
        self.mutate_data(None, |data| {
            if data.attachments.contains_key(&attachment.id) {
                return Err("Attachment ID collision.".into());
            }
            data.attachments.insert(
                attachment.id.clone(),
                attachments::StoredAttachment {
                    attachment: attachment.clone(),
                    host_id: host.id.clone(),
                    cwd: cwd.clone(),
                },
            );
            Ok(attachment.clone())
        })
    }

    fn reserve_run(&self, task_id: &str) -> Result<Arc<runner::RunControl>, String> {
        let (task, host) = self.task_and_host(task_id)?;
        if task.status != "running" {
            return Err("Task was cancelled before its process started.".into());
        }
        let mut runs = self
            .runs
            .lock()
            .map_err(|_| "Monitter run registry lock failed.".to_string())?;
        if runs.tasks.contains_key(task_id) {
            return Err("This task already has an active turn.".into());
        }
        if let Some(native) = &task.native_session_id {
            if let Some(owner) = runs
                .native_sessions
                .get(&native_session_key(&task, &host, native))
            {
                if owner != task_id {
                    return Err(
                        "This native Codex session already has an active Monitter writer.".into(),
                    );
                }
            }
        }
        let control = runner::RunControl::new(host.kind == "ssh");
        runs.tasks.insert(task_id.into(), control.clone());
        if let Some(native) = task.native_session_id.as_deref() {
            runs.native_sessions
                .insert(native_session_key(&task, &host, native), task_id.into());
        }
        Ok(control)
    }

    fn claim_native_session(
        &self,
        task_id: &str,
        task: &Task,
        host: &Host,
        native: &str,
    ) -> Result<(), String> {
        let key = native_session_key(task, host, native);
        let mut runs = self
            .runs
            .lock()
            .map_err(|_| "Monitter run registry lock failed.".to_string())?;
        if let Some(owner) = runs.native_sessions.get(&key) {
            if owner != task_id {
                if let Some(control) = runs.tasks.get(task_id) {
                    control.cancel();
                }
                return Err("This provider returned a native session that already has another active writer.".into());
            }
        }
        runs.native_sessions.insert(key, task_id.into());
        Ok(())
    }

    fn release_run(&self, task_id: &str) {
        self.revoke_collaboration_grant(task_id);
        if let Ok(mut runs) = self.runs.lock() {
            runs.tasks.remove(task_id);
            runs.native_sessions.retain(|_, owner| owner != task_id);
        }
    }

    fn set_task_archived(&self, task_id: &str, archived: bool) -> Result<Snapshot, String> {
        self.mutate(Some(task_id.into()), |snapshot| {
            let task = snapshot
                .tasks
                .iter_mut()
                .find(|task| task.id == task_id)
                .ok_or_else(|| "Task was not found.".to_string())?;
            if task.status == "running" {
                return Err("Cancel a running task before changing its archive state.".into());
            }
            task.archived = archived;
            task.updated_at = now();
            Ok(snapshot.clone())
        })
    }

    fn save_project(&self, mut project: Project) -> Result<Snapshot, String> {
        self.mutate(None, |snapshot| {
            if project.id.trim().is_empty() {
                project.id = id();
            }
            project.name = project.name.trim().into();
            validate_project(&project, snapshot)?;
            if let Some(current) = snapshot
                .projects
                .iter_mut()
                .find(|current| current.id == project.id)
            {
                *current = project;
            } else {
                snapshot.projects.push(project);
            }
            Ok(snapshot.clone())
        })
    }

    fn delete_project(&self, id: &str) -> Result<Snapshot, String> {
        self.mutate(None, |snapshot| {
            if !snapshot.projects.iter().any(|project| project.id == id) {
                return Err("Project was not found.".into());
            }
            snapshot.projects.retain(|project| project.id != id);
            for task in &mut snapshot.tasks {
                if task.project_id.as_deref() == Some(id) {
                    task.project_id = None;
                }
            }
            Ok(snapshot.clone())
        })
    }

    fn set_task_project(
        &self,
        task_id: &str,
        project_id: Option<String>,
    ) -> Result<Snapshot, String> {
        self.mutate(Some(task_id.into()), |snapshot| {
            if let Some(project_id) = &project_id {
                if !snapshot
                    .projects
                    .iter()
                    .any(|project| project.id == *project_id)
                {
                    return Err("Project was not found.".into());
                }
            }
            let task = snapshot
                .tasks
                .iter_mut()
                .find(|task| task.id == task_id)
                .ok_or_else(|| "Task was not found.".to_string())?;
            task.project_id = project_id;
            Ok(snapshot.clone())
        })
    }

    fn launch(self: &Arc<Self>, task_id: String, prompt: String) -> Result<(), String> {
        let control = self.reserve_run(&task_id)?;
        runner::start(self.clone(), task_id, prompt, control);
        Ok(())
    }

    pub(crate) fn record(&self, task: &str, kind: &str, title: &str, detail: String) {
        let _ = self.mutate(Some(task.into()), |state| {
            state.events.push(RunEvent {
                id: id(),
                task_id: task.into(),
                kind: kind.into(),
                title: title.into(),
                detail,
                created_at: now(),
            });
            Ok(())
        });
    }

    pub(crate) fn apply_event(&self, task_id: &str, parsed: Parsed) -> Result<(), String> {
        if let Some(native) = parsed.native_session_id.as_deref() {
            let (task, host) = self.task_and_host(task_id)?;
            self.claim_native_session(task_id, &task, &host, native)?;
        }
        self.mutate(Some(task_id.into()), |state| {
            let ix = state
                .tasks
                .iter()
                .position(|task| task.id == task_id)
                .ok_or_else(|| "Task was not found.".to_string())?;
            if let Some(native) = parsed.native_session_id {
                match state.tasks[ix].native_session_id.as_deref() {
                    Some(existing) if existing != native => {
                        return Err("Codex changed native session ID during a task.".into())
                    }
                    None => state.tasks[ix].native_session_id = Some(native),
                    _ => {}
                }
            }
            state.tasks[ix].updated_at = now();
            if let Some((kind, title, detail)) = parsed.event {
                state.events.push(RunEvent {
                    id: id(),
                    task_id: task_id.into(),
                    kind,
                    title,
                    detail,
                    created_at: now(),
                });
            }
            if let Some(text) = parsed.assistant.filter(|text| !text.trim().is_empty()) {
                state.messages.push(Message {
                    sender_agent_id: None,
                    collaboration_id: None,
                    id: id(),
                    task_id: task_id.into(),
                    role: "assistant".into(),
                    text: text.clone(),
                    created_at: now(),
                    attachments: vec![],
                });
                let channel_id = state.tasks[ix].channel_id.clone();
                let agent_id = state.tasks[ix].agent_id.clone();
                if let Some(channel) = channel_id.and_then(|channel_id| {
                    state
                        .channels
                        .iter_mut()
                        .find(|channel| channel.id == channel_id)
                }) {
                    channel.messages.push(ChannelMessage {
                        id: id(),
                        role: "assistant".into(),
                        agent_id: Some(agent_id),
                        text,
                        created_at: now(),
                        task_id: Some(task_id.into()),
                    });
                }
            }
            Ok(())
        })
    }

    pub(crate) fn finish(&self, task_id: &str, status: &str, error: Option<String>) {
        let _ = self.mutate(Some(task_id.into()), |state| {
            let ix = state
                .tasks
                .iter()
                .position(|task| task.id == task_id)
                .ok_or_else(|| "Task was not found.".to_string())?;
            if state.tasks[ix].status != "interrupted" || status == "interrupted" {
                state.tasks[ix].status = status.into();
            }
            state.tasks[ix].updated_at = now();
            let final_status = state.tasks[ix].status.clone();
            Service::complete_collaborations(state, task_id, &final_status, error.as_deref());
            if let Some(detail) = error {
                state.events.push(RunEvent {
                    id: id(),
                    task_id: task_id.into(),
                    kind: "error".into(),
                    title: "Codex process failed".into(),
                    detail,
                    created_at: now(),
                });
            }
            Ok(())
        });
        self.release_run(task_id);
    }

    fn create_task(&self, input: CreateTaskInput) -> Result<Task, String> {
        self.mutate_data(None, |data| create_task_in_data(data, input))
    }

    fn send(
        self: &Arc<Self>,
        task_id: String,
        text: String,
        attachment_ids: Vec<String>,
    ) -> Result<Snapshot, String> {
        if text.trim().is_empty() && attachment_ids.is_empty() {
            return Err("Message cannot be empty.".into());
        }
        let user_text = text.trim().to_string();
        let execution_prompt = self.mutate_data(Some(task_id.clone()), |data| {
            let state = &mut data.snapshot;
            let ix = state
                .tasks
                .iter()
                .position(|task| task.id == task_id)
                .ok_or_else(|| "Task was not found.".to_string())?;
            if state.tasks[ix].status == "running" {
                return Err("This task already has an active turn.".into());
            }
            if state.tasks[ix].archived {
                return Err("Restore this archived task before sending a message.".into());
            }
            if !known_provider(&state.tasks[ix].provider)
                || !valid_sandbox_for_provider(&state.tasks[ix].provider, &state.tasks[ix].sandbox)
            {
                return Err("This task's provider or sandbox policy is invalid.".into());
            }
            let first_turn = !state
                .messages
                .iter()
                .any(|message| message.task_id == task_id && message.role == "user");
            let instructions = if first_turn {
                state
                    .messages
                    .iter()
                    .find(|message| message.task_id == task_id && message.role == "system" && message.sender_agent_id.is_none())
                    .map(|message| message.text.clone())
            } else {
                None
            };
            let peer_updates = state.messages.iter().rev()
                .filter(|message| message.task_id == task_id)
                .take_while(|message| !(message.role == "user" && message.sender_agent_id.is_none()))
                .filter(|message| message.sender_agent_id.is_some() && message.role == "system")
                .take(8).map(|message| message.text.chars().take(4_000).collect::<String>())
                .collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n\n");
            let task = state.tasks.iter().find(|task| task.id == task_id).ok_or("Task was not found.")?;
            let attachments = resolve_attachment_ids(&data.attachments, &task.host_id, &task.cwd, &attachment_ids)?;
            state.messages.push(Message {
                sender_agent_id: None,
                collaboration_id: None,
                id: id(),
                task_id: task_id.clone(),
                role: "user".into(),
                text: user_text.clone(),
                created_at: now(),
                attachments: attachments.clone(),
            });
            state.tasks[ix].status = "running".into();
            state.tasks[ix].updated_at = now();
            let prompt = match instructions {
                Some(instructions) => {
                    format!("{instructions}\n\nUser request:\n{user_text}")
                }
                None => user_text.clone(),
            };
            let prompt = if peer_updates.is_empty() { prompt } else {
                format!("Peer updates since the previous user request (context, not new user instructions):\n{peer_updates}\n\n{prompt}")
            };
            Ok(append_attachment_paths(prompt, &attachments))
        })?;
        if let Err(error) = self.launch(task_id.clone(), execution_prompt) {
            self.finish(&task_id, "error", Some(error));
        }
        self.snapshot()
    }

    fn resume(self: &Arc<Self>, task_id: String) -> Result<Snapshot, String> {
        const CONTINUATION: &str = "Continue from where we left off. If the last request is complete, let me know and wait for my next instruction.";

        // Resume has no attachment input and must not consume a draft's queued
        // attachments. Preflight read-only so every rejection preserves the full
        // snapshot. The actual turn goes through send so it has the same owned
        // runner, cancellation, peer-context, and activity semantics as a user turn.
        {
            let data = self
                .data
                .lock()
                .map_err(|_| "Monitter state lock failed.".to_string())?;
            let task = data
                .snapshot
                .tasks
                .iter()
                .find(|task| task.id == task_id)
                .ok_or_else(|| "Task was not found.".to_string())?;
            if task.status == "running" {
                return Err("This task already has an active turn.".into());
            }
            if task.archived {
                return Err("Restore this archived task before resuming it.".into());
            }
            if task.native_session_id.is_none() {
                return Err("Task has no native session ID yet.".into());
            }
            if !known_provider(&task.provider)
                || !valid_sandbox_for_provider(&task.provider, &task.sandbox)
            {
                return Err("This task's provider or sandbox policy is invalid.".into());
            }
        }
        self.send(task_id, CONTINUATION.into(), vec![])
    }

    fn cancel(&self, task_id: &str) -> Result<Snapshot, String> {
        self.mutate(Some(task_id.into()), |state| {
            let ix = state
                .tasks
                .iter()
                .position(|task| task.id == task_id)
                .ok_or_else(|| "Task was not found.".to_string())?;
            if state.tasks[ix].status != "running" {
                return Err("Task is not running.".into());
            }
            state.tasks[ix].status = "interrupted".into();
            state.tasks[ix].updated_at = now();
            state.events.push(RunEvent {
                id: id(),
                task_id: task_id.into(),
                kind: "status".into(),
                title: "Cancellation requested".into(),
                detail: String::new(),
                created_at: now(),
            });
            Ok(state.clone())
        })?;
        if let Ok(runs) = self.runs.lock() {
            if let Some(control) = runs.tasks.get(task_id) {
                control.cancel();
            }
        }
        self.cancel_collaboration_children(task_id);
        self.snapshot()
    }

    fn cleanup(&self) {
        self.stopping
            .store(true, std::sync::atomic::Ordering::Release);
        if let Ok(mut broker) = self.collaboration.lock() {
            broker.take();
        }
        let controls = self
            .runs
            .lock()
            .map(|runs| runs.tasks.values().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        for control in controls {
            control.cancel();
        }
    }

    fn run_is_active(&self, task_id: &str) -> bool {
        self.runs
            .lock()
            .map(|runs| runs.tasks.contains_key(task_id))
            .unwrap_or(true)
    }

    fn abort_run(&self, task_id: &str) {
        if let Ok(runs) = self.runs.lock() {
            if let Some(control) = runs.tasks.get(task_id) {
                control.cancel();
            }
        }
    }
}

fn create_task_in_data(data: &mut ServiceData, input: CreateTaskInput) -> Result<Task, String> {
    let state = &mut data.snapshot;
    if input.title.trim().is_empty() {
        return Err("Task title is required.".into());
    }
    let agent = state
        .agents
        .iter()
        .find(|agent| agent.id == input.agent_id)
        .cloned()
        .ok_or_else(|| "Agent was not found.".to_string())?;
    if !known_provider(&agent.provider)
        || !valid_sandbox_for_provider(&agent.provider, &agent.sandbox)
    {
        return Err("Agent provider or sandbox policy is invalid.".into());
    }
    let host = state
        .hosts
        .iter()
        .find(|host| host.id == agent.host_id)
        .cloned()
        .ok_or_else(|| "Agent host was not found.".to_string())?;
    if let Some(parent) = &input.parent_task_id {
        if !state.tasks.iter().any(|task| &task.id == parent) {
            return Err("Parent task was not found.".into());
        }
    }
    if let Some(channel) = &input.channel_id {
        if !state.channels.iter().any(|item| &item.id == channel) {
            return Err("Channel was not found.".into());
        }
    }
    let project = input
        .project_id
        .as_deref()
        .map(|project_id| {
            state
                .projects
                .iter()
                .find(|project| project.id == project_id)
                .cloned()
                .ok_or_else(|| "Project was not found.".to_string())
        })
        .transpose()?;
    let mut task = task_from_agent(&agent, &input);
    if let Some(workspace) = project.as_ref().and_then(|project| {
        project
            .workspaces
            .iter()
            .find(|workspace| workspace.host_id == agent.host_id)
    }) {
        task.cwd = workspace.cwd.clone();
    }
    if task.cwd.trim().is_empty() {
        task.cwd = host.default_cwd.clone();
    }
    if task.cwd.trim().is_empty() {
        return Err("Agent or host must specify a task folder.".into());
    }
    if !agent_instructions(&agent).trim().is_empty() {
        state.messages.push(Message {
            sender_agent_id: None,
            collaboration_id: None,
            id: id(),
            task_id: task.id.clone(),
            role: "system".into(),
            text: agent_instructions(&agent),
            created_at: now(),
            attachments: vec![],
        });
    }
    data.task_hosts.insert(task.id.clone(), host);
    state.tasks.push(task.clone());
    Ok(task)
}

fn resolve_attachment_target(
    data: &ServiceData,
    target: &attachments::AttachmentTarget,
) -> Result<(Host, String), String> {
    if let Some(task_id) = target.task_id.as_deref() {
        if target.agent_id.is_some() || target.project_id.is_some() {
            return Err("Attachment target must be a task or a draft agent/project.".into());
        }
        let task = data
            .snapshot
            .tasks
            .iter()
            .find(|task| task.id == task_id)
            .ok_or("Task was not found.")?;
        let host = data
            .task_hosts
            .get(task_id)
            .cloned()
            .ok_or("Task's saved host settings were not found.")?;
        return Ok((host, task.cwd.clone()));
    }
    let agent_id = target
        .agent_id
        .as_deref()
        .ok_or("Attachment target needs a task or agent.")?;
    let agent = data
        .snapshot
        .agents
        .iter()
        .find(|agent| agent.id == agent_id)
        .ok_or("Agent was not found.")?;
    let host = data
        .snapshot
        .hosts
        .iter()
        .find(|host| host.id == agent.host_id)
        .cloned()
        .ok_or("Agent host was not found.")?;
    let cwd = if let Some(project_id) = target.project_id.as_deref() {
        let project = data
            .snapshot
            .projects
            .iter()
            .find(|project| project.id == project_id)
            .ok_or("Project was not found.")?;
        project
            .workspaces
            .iter()
            .find(|workspace| workspace.host_id == agent.host_id)
            .map(|workspace| workspace.cwd.clone())
            .unwrap_or_else(|| agent.cwd.clone())
    } else {
        agent.cwd.clone()
    };
    let cwd = if cwd.trim().is_empty() {
        host.default_cwd.clone()
    } else {
        cwd
    };
    if cwd.trim().is_empty() {
        return Err("Agent or host must specify a task folder.".into());
    }
    Ok((host, cwd))
}

fn resolve_attachment_ids(
    registry: &HashMap<String, attachments::StoredAttachment>,
    host_id: &str,
    cwd: &str,
    ids: &[String],
) -> Result<Vec<attachments::Attachment>, String> {
    let mut seen = std::collections::HashSet::new();
    ids.iter()
        .map(|id| {
            if !seen.insert(id) {
                return Err("Attachment IDs must not repeat.".into());
            }
            let stored = registry.get(id).ok_or("Attachment was not found.")?;
            if stored.host_id != host_id || stored.cwd != cwd {
                return Err("Attachment belongs to a different task host or folder.".into());
            }
            Ok(stored.attachment.clone())
        })
        .collect()
}

fn matching_attachments(
    registry: &HashMap<String, attachments::StoredAttachment>,
    host_id: &str,
    cwd: &str,
    ids: &[String],
) -> Result<(Vec<attachments::Attachment>, Vec<String>), String> {
    let mut seen = std::collections::HashSet::new();
    let mut attachments = Vec::new();
    let mut matched = Vec::new();
    for id in ids {
        if !seen.insert(id) {
            return Err("Attachment IDs must not repeat.".into());
        }
        let stored = registry.get(id).ok_or("Attachment was not found.")?;
        if stored.host_id == host_id && stored.cwd == cwd {
            attachments.push(stored.attachment.clone());
            matched.push(id.clone());
        }
    }
    Ok((attachments, matched))
}

fn append_attachment_paths(prompt: String, attachments: &[attachments::Attachment]) -> String {
    if attachments.is_empty() {
        return prompt;
    }
    let paths = attachments
        .iter()
        .map(|attachment| {
            serde_json::to_string(&attachment.path).unwrap_or_else(|_| "\"attachment\"".into())
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!("{prompt}\n\nAttached files are available at these host paths:\n{paths}")
}

#[tauri::command]
fn get_snapshot(state: State<'_, AppState>) -> Result<Snapshot, String> {
    state.0.snapshot()
}

#[tauri::command]
fn read_attachment_file(source_path: String) -> Result<attachments::ReadAttachmentFile, String> {
    attachments::read_attachment_file(&source_path)
}

#[tauri::command]
fn store_attachment(
    state: State<'_, AppState>,
    target: attachments::AttachmentTarget,
    filename: String,
    mime_type: String,
    data_base64: String,
    preview_data_url: Option<String>,
    source_id: Option<String>,
) -> Result<attachments::Attachment, String> {
    state.0.store_attachment(
        target,
        filename,
        mime_type,
        data_base64,
        preview_data_url,
        source_id,
    )
}

#[tauri::command]
fn save_host(state: State<'_, AppState>, mut host: Host) -> Result<Snapshot, String> {
    state.0.mutate(None, |snapshot| {
        if host.id.trim().is_empty() {
            host.id = id();
        }
        if host.name.trim().is_empty() {
            return Err("Host name is required.".into());
        }
        if !matches!(host.kind.as_str(), "local" | "ssh") {
            return Err("Host kind must be local or ssh.".into());
        }
        if host.kind == "ssh" && host.address.trim().is_empty() {
            return Err("SSH host needs an address or configured alias.".into());
        }
        if let Some(current) = snapshot
            .hosts
            .iter_mut()
            .find(|current| current.id == host.id)
        {
            *current = host;
        } else {
            snapshot.hosts.push(host);
        }
        Ok(snapshot.clone())
    })
}

#[tauri::command]
fn delete_host(state: State<'_, AppState>, id: String) -> Result<Snapshot, String> {
    state.0.mutate(None, |snapshot| {
        let host = snapshot
            .hosts
            .iter()
            .find(|host| host.id == id)
            .ok_or_else(|| "Host was not found.".to_string())?;
        let default_local = snapshot
            .hosts
            .iter()
            .find(|host| host.kind == "local")
            .map(|host| host.id.as_str());
        if host.kind == "local" && default_local == Some(id.as_str()) {
            return Err("The default local host cannot be deleted.".into());
        }
        if snapshot.agents.iter().any(|agent| agent.host_id == id)
            || snapshot.tasks.iter().any(|task| task.host_id == id)
        {
            return Err("Host is referenced by an agent or task.".into());
        }
        if snapshot.projects.iter().any(|project| {
            project
                .workspaces
                .iter()
                .any(|workspace| workspace.host_id == id)
        }) {
            return Err("Host is referenced by a project workspace.".into());
        }
        snapshot.hosts.retain(|host| host.id != id);
        Ok(snapshot.clone())
    })
}

const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

fn probe_output(command: std::process::Command) -> Result<String, String> {
    probe_output_with_timeout(command, PROBE_TIMEOUT)
}

fn probe_output_with_timeout(
    mut command: std::process::Command,
    timeout: Duration,
) -> Result<String, String> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| format!("Could not run CLI probe: {error}"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Could not read CLI probe output.".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "Could not read CLI probe errors.".to_string())?;
    let stdout_reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = std::io::BufReader::new(stdout).read_to_end(&mut bytes);
        bytes
    });
    let stderr_reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = std::io::BufReader::new(stderr).read_to_end(&mut bytes);
        bytes
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("Could not wait for CLI probe: {error}"))?
        {
            break status;
        }
        if Instant::now() >= deadline {
            #[cfg(unix)]
            unsafe {
                let _ = libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            return Err(format!(
                "CLI probe timed out after {} seconds.",
                timeout.as_secs_f32()
            ));
        }
        thread::sleep(Duration::from_millis(20));
    };
    let stdout = stdout_reader.join().unwrap_or_default();
    let stderr = stderr_reader.join().unwrap_or_default();
    if status.success() {
        Ok(String::from_utf8_lossy(&stdout).trim().into())
    } else {
        let stderr = String::from_utf8_lossy(&stderr).trim().to_string();
        Err(if stderr.is_empty() {
            format!("CLI probe exited with {status}.")
        } else {
            stderr
        })
    }
}

fn probe_host_blocking(host: Host) -> Result<ProbeResult, String> {
    let mut versions = HashMap::new();
    let mut failures = Vec::new();
    for provider in ["codex", "claude", "opencode", "hermes"] {
        match runner::build_probe_command(&host, provider).and_then(probe_output) {
            Ok(version) => {
                versions.insert(provider.into(), version);
            }
            Err(error) => failures.push(format!("{provider}: {error}")),
        }
    }
    if versions.is_empty() {
        Ok(ProbeResult {
            ok: false,
            versions,
            message: format!("No supported CLI is reachable. {}", failures.join(" ")),
        })
    } else {
        Ok(ProbeResult {
            ok: true,
            versions,
            message: if failures.is_empty() {
                "Configured CLI adapters are reachable.".into()
            } else {
                format!("Some adapters are unavailable: {}", failures.join(" "))
            },
        })
    }
}

#[tauri::command]
async fn probe_host(host: Host) -> Result<ProbeResult, String> {
    tauri::async_runtime::spawn_blocking(move || probe_host_blocking(host))
        .await
        .map_err(|error| format!("CLI probe worker failed: {error}"))?
}

#[tauri::command]
fn save_agent(state: State<'_, AppState>, mut agent: Agent) -> Result<Snapshot, String> {
    state.0.mutate(None, |snapshot| {
        if agent.id.trim().is_empty() {
            agent.id = id();
        }
        if agent.name.trim().is_empty() {
            return Err("Agent name is required.".into());
        }
        validate_agent_avatar(agent.avatar.as_deref())?;
        validate_collaboration_profile(&agent)?;
        if !known_provider(&agent.provider) {
            return Err(format!(
                "Provider '{}' is not implemented in Monitter.",
                agent.provider
            ));
        }
        if !valid_sandbox_for_provider(&agent.provider, &agent.sandbox) {
            return Err(if agent.provider == "codex" {
                "Codex sandbox must be read-only or workspace-write.".into()
            } else {
                "This provider must use the harness-configured sandbox policy.".into()
            });
        }
        if !snapshot.hosts.iter().any(|host| host.id == agent.host_id) {
            return Err("Agent host was not found.".into());
        }
        if let Some(current) = snapshot
            .agents
            .iter_mut()
            .find(|current| current.id == agent.id)
        {
            *current = agent;
        } else {
            snapshot.agents.push(agent);
        }
        Ok(snapshot.clone())
    })
}

const MAX_AVATAR_DATA_URL_BYTES: usize = 3 * 1024 * 1024;

fn validate_collaboration_profile(agent: &Agent) -> Result<(), String> {
    if agent.name.len() > 240 || agent.description.len() > 8_192 {
        return Err("Agent name or description is too long.".into());
    }
    for entries in [&agent.expertise, &agent.responsibilities, &agent.skills] {
        if entries.len() > 40
            || entries
                .iter()
                .any(|entry| entry.trim().is_empty() || entry.len() > 512)
        {
            return Err(
                "Use up to 40 non-empty profile entries, each no longer than 512 bytes.".into(),
            );
        }
    }
    Ok(())
}

fn validate_agent_avatar(avatar: Option<&str>) -> Result<(), String> {
    let Some(avatar) = avatar else {
        return Ok(());
    };
    if avatar.len() > MAX_AVATAR_DATA_URL_BYTES {
        return Err("Agent avatar is too large.".into());
    }
    let encoded = [
        "data:image/png;base64,",
        "data:image/jpeg;base64,",
        "data:image/webp;base64,",
    ]
    .iter()
    .find_map(|prefix| avatar.strip_prefix(prefix))
    .ok_or_else(|| "Agent avatar must be a PNG, JPEG, or WebP data URL.".to_string())?;
    if encoded.is_empty() || encoded.len() % 4 != 0 {
        return Err("Agent avatar contains invalid base64 data.".into());
    }
    let padding = encoded
        .bytes()
        .rev()
        .take_while(|byte| *byte == b'=')
        .count();
    if padding > 2
        || encoded[..encoded.len() - padding]
            .bytes()
            .any(|byte| !byte.is_ascii_alphanumeric() && byte != b'+' && byte != b'/')
    {
        return Err("Agent avatar contains invalid base64 data.".into());
    }
    Ok(())
}

#[tauri::command]
fn delete_agent(state: State<'_, AppState>, id: String) -> Result<Snapshot, String> {
    state.0.mutate(None, |snapshot| {
        if snapshot.tasks.iter().any(|task| task.agent_id == id) {
            return Err("Agent has tasks and cannot be deleted.".into());
        }
        if !snapshot.agents.iter().any(|agent| agent.id == id) {
            return Err("Agent was not found.".into());
        }
        snapshot.agents.retain(|agent| agent.id != id);
        for channel in &mut snapshot.channels {
            channel.agent_ids.retain(|agent| agent != &id);
        }
        Ok(snapshot.clone())
    })
}

#[tauri::command]
fn create_task(state: State<'_, AppState>, input: CreateTaskInput) -> Result<Task, String> {
    state.0.create_task(input)
}

fn validate_project(project: &Project, snapshot: &Snapshot) -> Result<(), String> {
    if project.name.trim().is_empty() {
        return Err("Project name is required.".into());
    }
    let mut workspace_hosts = std::collections::HashSet::new();
    for workspace in &project.workspaces {
        if workspace.cwd.trim().is_empty() {
            return Err("Project workspace folder is required.".into());
        }
        if !snapshot
            .hosts
            .iter()
            .any(|host| host.id == workspace.host_id)
        {
            return Err("Project workspace host was not found.".into());
        }
        if !workspace_hosts.insert(&workspace.host_id) {
            return Err("Project can have only one workspace per host.".into());
        }
    }
    Ok(())
}

#[tauri::command]
fn save_project(state: State<'_, AppState>, project: Project) -> Result<Snapshot, String> {
    state.0.save_project(project)
}

#[tauri::command]
fn delete_project(state: State<'_, AppState>, id: String) -> Result<Snapshot, String> {
    state.0.delete_project(&id)
}

#[tauri::command]
fn set_task_project(
    state: State<'_, AppState>,
    task_id: String,
    project_id: Option<String>,
) -> Result<Snapshot, String> {
    state.0.set_task_project(&task_id, project_id)
}

#[tauri::command]
fn rename_task(state: State<'_, AppState>, id: String, title: String) -> Result<Snapshot, String> {
    state.0.mutate(Some(id.clone()), |snapshot| {
        if title.trim().is_empty() {
            return Err("Task title is required.".into());
        }
        let task = snapshot
            .tasks
            .iter_mut()
            .find(|task| task.id == id)
            .ok_or_else(|| "Task was not found.".to_string())?;
        task.title = title.trim().into();
        task.updated_at = now();
        Ok(snapshot.clone())
    })
}

#[tauri::command]
fn delete_task(state: State<'_, AppState>, id: String) -> Result<Snapshot, String> {
    state.0.mutate_data(Some(id.clone()), |data| {
        if data.snapshot.collaborations.iter().any(|delivery| {
            (delivery.from_task_id == id || delivery.to_task_id == id)
                && matches!(delivery.status.as_str(), "queued" | "running")
        }) {
            return Err(
                "Finish or cancel this chat's pending collaborations before deleting it.".into(),
            );
        }
        let task = data
            .snapshot
            .tasks
            .iter()
            .find(|task| task.id == id)
            .ok_or_else(|| "Task was not found.".to_string())?;
        if !task.archived {
            return Err("Archive this chat before permanently deleting it.".into());
        }
        if task.status == "running" {
            return Err("Cancel a running task before deleting it.".into());
        }
        data.snapshot.tasks.retain(|task| task.id != id);
        data.snapshot
            .messages
            .retain(|message| message.task_id != id);
        data.snapshot.events.retain(|event| event.task_id != id);
        data.task_hosts.remove(&id);
        Ok(data.snapshot.clone())
    })
}

#[tauri::command]
fn preview_task_deletion(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<deletion::DeletionPreview, String> {
    let (task, host) = state.0.task_and_host(&task_id)?;
    Ok(deletion::preview(&state.0.snapshot()?, &task, &host))
}

#[tauri::command]
fn delete_archived_task(
    state: State<'_, AppState>,
    task_id: String,
    remove_native_files: bool,
) -> Result<Snapshot, String> {
    let (task, host) = state.0.task_and_host(&task_id)?;
    let snapshot = state.0.snapshot()?;
    if !task.archived {
        return Err("Archive this chat before permanently deleting it.".into());
    }
    if task.status == "running" {
        return Err("Cancel this running chat before permanently deleting it.".into());
    }
    if remove_native_files {
        deletion::remove_verified(&snapshot, &task, &host)?;
    }
    delete_task(state, task_id)
}

#[tauri::command]
fn set_task_archived(
    state: State<'_, AppState>,
    task_id: String,
    archived: bool,
) -> Result<Snapshot, String> {
    state.0.set_task_archived(&task_id, archived)
}

#[tauri::command]
fn send_message(
    state: State<'_, AppState>,
    task_id: String,
    text: String,
    attachment_ids: Option<Vec<String>>,
) -> Result<Snapshot, String> {
    state
        .0
        .send(task_id, text, attachment_ids.unwrap_or_default())
}

#[tauri::command]
fn resume_task(state: State<'_, AppState>, task_id: String) -> Result<Snapshot, String> {
    state.0.resume(task_id)
}

#[tauri::command]
fn cancel_task(state: State<'_, AppState>, task_id: String) -> Result<Snapshot, String> {
    state.0.cancel(&task_id)
}

fn validate_settings(settings: &Settings) -> Result<(), String> {
    if !matches!(settings.theme.as_str(), "light" | "dark" | "system") {
        return Err("Theme must be light, dark, or system.".into());
    }
    if settings.accent.trim().is_empty() {
        return Err("Accent colour is required.".into());
    }
    if !(80..=200).contains(&settings.interface_scale) {
        return Err("Interface scale must be between 80% and 200%.".into());
    }
    if !matches!(
        settings.sidebar_view.as_str(),
        "standard" | "activity" | "projects"
    ) {
        return Err("Sidebar view must be standard, activity, or projects.".into());
    }
    Ok(())
}

#[tauri::command]
fn save_settings(state: State<'_, AppState>, settings: Settings) -> Result<Snapshot, String> {
    state.0.mutate(None, |snapshot| {
        validate_settings(&settings)?;
        snapshot.settings = settings;
        Ok(snapshot.clone())
    })
}

#[tauri::command]
fn save_channel(state: State<'_, AppState>, mut channel: Channel) -> Result<Snapshot, String> {
    state.0.mutate(None, |snapshot| {
        if channel.id.trim().is_empty() {
            channel.id = id();
        }
        if channel.name.trim().is_empty() {
            return Err("Channel name is required.".into());
        }
        channel.agent_ids.sort();
        channel.agent_ids.dedup();
        if channel
            .agent_ids
            .iter()
            .any(|id| !snapshot.agents.iter().any(|agent| agent.id == *id))
        {
            return Err("Channel contains an unknown agent.".into());
        }
        if let Some(current) = snapshot
            .channels
            .iter_mut()
            .find(|current| current.id == channel.id)
        {
            channel.messages = current.messages.clone();
            *current = channel;
        } else {
            channel.messages.clear();
            snapshot.channels.push(channel);
        }
        Ok(snapshot.clone())
    })
}

#[tauri::command]
fn send_channel_message(
    state: State<'_, AppState>,
    channel_id: String,
    text: String,
    mut agent_ids: Vec<String>,
    attachment_ids: Option<Vec<String>>,
) -> Result<Snapshot, String> {
    let attachment_ids = attachment_ids.unwrap_or_default();
    if (text.trim().is_empty() && attachment_ids.is_empty()) || agent_ids.is_empty() {
        return Err("Choose at least one recipient and enter a message.".into());
    }
    agent_ids.sort();
    agent_ids.dedup();
    let service = state.0.clone();
    let user_text = text.trim().to_string();
    let runs = service.mutate_data(None, |data| {
        let state = &mut data.snapshot;
        let channel_ix = state
            .channels
            .iter()
            .position(|channel| channel.id == channel_id)
            .ok_or_else(|| "Channel was not found.".to_string())?;
        if agent_ids
            .iter()
            .any(|id| !state.channels[channel_ix].agent_ids.contains(id))
        {
            return Err("Recipients must be explicitly selected channel agents.".into());
        }
        for agent_id in &agent_ids {
            if let Some(task) = state.tasks.iter().rev().find(|task| {
                task.channel_id.as_deref() == Some(&channel_id)
                    && &task.agent_id == agent_id
                    && !task.archived
            }) {
                if task.status == "running" {
                    return Err(format!(
                        "{} already has an active turn in this channel.",
                        state
                            .agents
                            .iter()
                            .find(|agent| &agent.id == agent_id)
                            .map(|agent| agent.name.as_str())
                            .unwrap_or("An agent")
                    ));
                }
            }
        }

        state.channels[channel_ix].messages.push(ChannelMessage {
            id: id(),
            role: "user".into(),
            agent_id: None,
            text: user_text.clone(),
            created_at: now(),
            task_id: None,
        });
        let context = state.channels[channel_ix]
            .messages
            .iter()
            .rev()
            .take(12)
            .rev()
            .map(|message| {
                let speaker = message
                    .agent_id
                    .as_ref()
                    .and_then(|id| state.agents.iter().find(|agent| &agent.id == id))
                    .map(|agent| agent.name.as_str())
                    .unwrap_or("User");
                format!("{speaker}: {}", message.text)
            })
            .collect::<Vec<_>>()
            .join("\n");
        let channel_name = state.channels[channel_ix].name.clone();
        let mut runs = Vec::new();
        let mut matched_attachment_ids = std::collections::HashSet::new();
        for agent_id in &agent_ids {
            let agent = state
                .agents
                .iter()
                .find(|agent| &agent.id == agent_id)
                .cloned()
                .ok_or_else(|| "Agent was not found.".to_string())?;
            if !known_provider(&agent.provider)
                || !valid_sandbox_for_provider(&agent.provider, &agent.sandbox)
            {
                return Err("Selected agent has an invalid provider or sandbox policy.".into());
            }
            let existing_ix = state.tasks.iter().rposition(|task| {
                task.channel_id.as_deref() == Some(&channel_id)
                    && task.agent_id == agent.id
                    && !task.archived
            });
            let task_id = if let Some(ix) = existing_ix {
                state.tasks[ix].status = "running".into();
                state.tasks[ix].updated_at = now();
                state.tasks[ix].id.clone()
            } else {
                let input = CreateTaskInput {
                    agent_id: agent.id.clone(),
                    title: format!(
                        "{channel_name}: {}",
                        user_text.chars().take(48).collect::<String>()
                    ),
                    native_session_id: None,
                    parent_task_id: None,
                    channel_id: Some(channel_id.clone()),
                    project_id: None,
                };
                let host = state
                    .hosts
                    .iter()
                    .find(|host| host.id == agent.host_id)
                    .cloned()
                    .ok_or_else(|| "Agent host was not found.".to_string())?;
                let mut task = task_from_agent(&agent, &input);
                if task.cwd.trim().is_empty() {
                    task.cwd = host.default_cwd.clone();
                }
                if task.cwd.trim().is_empty() {
                    return Err("Agent or host must specify a task folder.".into());
                }
                task.status = "running".into();
                if !agent_instructions(&agent).trim().is_empty() {
                    state.messages.push(Message {
                        sender_agent_id: None,
                        collaboration_id: None,
                        id: id(),
                        task_id: task.id.clone(),
                        role: "system".into(),
                        text: agent_instructions(&agent),
                        created_at: now(),
                        attachments: vec![],
                    });
                }
                data.task_hosts.insert(task.id.clone(), host);
                let id = task.id.clone();
                state.tasks.push(task);
                id
            };
            let task = state
                .tasks
                .iter()
                .find(|task| task.id == task_id)
                .ok_or("Task was not found.")?;
            let (attachments, matched) =
                matching_attachments(&data.attachments, &task.host_id, &task.cwd, &attachment_ids)?;
            matched_attachment_ids.extend(matched);
            state.messages.push(Message {
                sender_agent_id: None,
                collaboration_id: None,
                id: id(),
                task_id: task_id.clone(),
                role: "user".into(),
                text: user_text.clone(),
                created_at: now(),
                attachments: attachments.clone(),
            });
            let task_prompt = if existing_ix.is_none() && !agent.instructions.trim().is_empty() {
                format!(
                    "{}\n\nChannel context:\n{context}\n\nNew message:\n{user_text}",
                    agent.instructions.trim()
                )
            } else {
                format!("Channel context:\n{context}\n\nNew message:\n{user_text}")
            };
            runs.push((task_id, append_attachment_paths(task_prompt, &attachments)));
        }
        if attachment_ids
            .iter()
            .any(|id| !matched_attachment_ids.contains(id))
        {
            return Err(
                "An attachment does not belong to any selected recipient host and folder.".into(),
            );
        }
        Ok(runs)
    })?;
    for (task_id, prompt) in runs {
        if let Err(error) = service.launch(task_id.clone(), prompt) {
            service.finish(&task_id, "error", Some(error));
        }
    }
    service.snapshot()
}

#[tauri::command]
fn get_resume_command(state: State<'_, AppState>, task_id: String) -> Result<String, String> {
    let (task, host) = state.0.task_and_host(&task_id)?;
    if task.status == "running" {
        return Err("A running task cannot be resumed from another terminal.".into());
    }
    let native = task
        .native_session_id
        .as_ref()
        .ok_or_else(|| "Task has no native session ID yet.".to_string())?;
    runner::resume_command(&host, &task, native)
}

pub fn smoke_sequence(
    state_dir: &Path,
    host: Option<Host>,
    cwd: &Path,
    first: &str,
    second: Option<&str>,
    cancel: bool,
    expected_status: &str,
) -> Result<SmokeResult, String> {
    smoke_provider_sequence(
        state_dir,
        host,
        cwd,
        first,
        second,
        cancel,
        expected_status,
        "codex",
        None,
    )
}

pub fn smoke_provider_sequence(
    state_dir: &Path,
    host: Option<Host>,
    cwd: &Path,
    first: &str,
    second: Option<&str>,
    cancel: bool,
    expected_status: &str,
    provider: &str,
    model: Option<&str>,
) -> Result<SmokeResult, String> {
    if !known_provider(provider) {
        return Err(format!("Unsupported smoke provider: {provider}."));
    }
    if !matches!(expected_status, "completed" | "interrupted" | "error") {
        return Err("--expect-status must be completed, interrupted, or error.".into());
    }
    let service = Service::open(None, state_dir.to_path_buf())?;
    service.mutate(None, |snapshot| {
        if let Some(host) = host.clone() {
            if host.id.trim().is_empty() {
                return Err("Smoke host JSON needs a stable id.".into());
            }
            let old = snapshot.hosts[0].id.clone();
            snapshot.hosts[0] = host.clone();
            for agent in &mut snapshot.agents {
                if agent.host_id == old {
                    agent.host_id = host.id.clone();
                }
            }
        }
        let agent = snapshot
            .agents
            .first_mut()
            .ok_or_else(|| "No Codex agent exists.".to_string())?;
        agent.cwd = cwd.display().to_string();
        agent.provider = provider.into();
        agent.model = model.unwrap_or_default().into();
        agent.sandbox = if provider == "codex" {
            "read-only"
        } else {
            "harness-configured"
        }
        .into();
        Ok(())
    })?;
    let agent = service
        .snapshot()?
        .agents
        .into_iter()
        .next()
        .ok_or_else(|| "No Codex agent exists.".to_string())?;
    let task = service.create_task(CreateTaskInput {
        agent_id: agent.id,
        title: "Monitter smoke".into(),
        native_session_id: None,
        parent_task_id: None,
        channel_id: None,
        project_id: None,
    })?;
    service.send(task.id.clone(), first.into(), vec![])?;
    if cancel {
        wait_until_tool(&service, &task.id, 90)?;
        if service
            .snapshot()?
            .tasks
            .iter()
            .find(|item| item.id == task.id)
            .map(|item| item.status.as_str())
            == Some("running")
        {
            service.cancel(&task.id)?;
        }
    }
    wait_idle(&service, &task.id, 180)?;
    if let Some(second) = second {
        if service
            .snapshot()?
            .tasks
            .iter()
            .find(|item| item.id == task.id)
            .map(|item| item.status.as_str())
            == Some("completed")
        {
            service.send(task.id.clone(), second.into(), vec![])?;
            wait_idle(&service, &task.id, 180)?;
        }
    }
    let before_restart = service.snapshot()?;
    let before_task = before_restart
        .tasks
        .iter()
        .find(|item| item.id == task.id)
        .cloned()
        .ok_or_else(|| "Smoke task disappeared.".to_string())?;
    let output_count = before_restart
        .messages
        .iter()
        .filter(|message| message.task_id == task.id && message.role == "assistant")
        .count();
    let last_assistant_text = before_restart
        .messages
        .iter()
        .rev()
        .find(|message| message.task_id == task.id && message.role == "assistant")
        .map(|message| message.text.clone());
    drop(service);

    let reopened = Service::open(None, state_dir.to_path_buf())?;
    let after_restart = reopened.snapshot()?;
    let persisted = after_restart
        .tasks
        .iter()
        .find(|item| item.id == task.id)
        .map(|item| {
            item.status == before_task.status
                && item.native_session_id == before_task.native_session_id
        })
        .unwrap_or(false)
        && after_restart
            .messages
            .iter()
            .filter(|message| message.task_id == task.id && message.role == "assistant")
            .count()
            == output_count;
    let expected_outputs = 1 + usize::from(second.is_some());
    let completed_evidence = expected_status != "completed"
        || (before_task.native_session_id.is_some()
            && output_count >= expected_outputs
            && last_assistant_text.is_some());
    let ok = before_task.status == expected_status && persisted && completed_evidence;
    let message = if ok {
        format!("Expected {expected_status} result persisted through a native service restart.")
    } else {
        format!(
            "Expected {expected_status}; got {} with {output_count} assistant output(s), native session {}, persistence {persisted}.",
            before_task.status,
            before_task
                .native_session_id
                .as_deref()
                .unwrap_or("missing")
        )
    };
    Ok(SmokeResult {
        ok,
        task_id: task.id,
        native_session_id: before_task.native_session_id,
        final_status: before_task.status,
        output_count,
        persisted,
        last_assistant_text,
        message,
    })
}

fn wait_until_tool(service: &Service, task_id: &str, seconds: u64) -> Result<(), String> {
    for _ in 0..seconds * 10 {
        let snapshot = service.snapshot()?;
        let task = snapshot
            .tasks
            .iter()
            .find(|task| task.id == task_id)
            .ok_or_else(|| "Smoke task disappeared.".to_string())?;
        if snapshot
            .events
            .iter()
            .any(|event| event.task_id == task_id && event.kind == "tool")
        {
            return Ok(());
        }
        if task.status != "running" {
            return Err("Codex ended before starting the cancellation test tool.".into());
        }
        thread::sleep(Duration::from_millis(100));
    }
    let _ = service.cancel(task_id);
    service.abort_run(task_id);
    wait_for_run_exit(service, task_id, 10);
    Err("Timed out waiting for Codex to start the cancellation test tool.".into())
}

fn wait_idle(service: &Service, task_id: &str, seconds: u64) -> Result<(), String> {
    for _ in 0..seconds * 10 {
        if service
            .snapshot()?
            .tasks
            .iter()
            .find(|task| task.id == task_id)
            .map(|task| task.status.as_str())
            != Some("running")
            && !service.run_is_active(task_id)
        {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }
    let _ = service.cancel(task_id);
    service.abort_run(task_id);
    wait_for_run_exit(service, task_id, 10);
    Err("Timed out waiting for Monitter task.".into())
}

fn wait_for_run_exit(service: &Service, task_id: &str, seconds: u64) {
    for _ in 0..seconds * 10 {
        if !service.run_is_active(task_id) {
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }
}

#[tauri::command]
async fn get_task_goal(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<Option<serde_json::Value>, String> {
    let (task, host) = state.0.task_and_host(&task_id)?;
    if task.provider != "codex" {
        return Ok(None);
    }
    tauri::async_runtime::spawn_blocking(move || goals::read_goal(&host, &task))
        .await
        .map_err(|error| format!("Goal lookup worker failed: {error}"))?
}

#[tauri::command]
async fn get_task_git_status(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<git::GitStatus, String> {
    let (task, host) = state.0.task_and_host(&task_id)?;
    tauri::async_runtime::spawn_blocking(move || git::status(&host, &task.cwd))
        .await
        .map_err(|error| format!("Git status worker failed: {error}"))?
}

#[tauri::command]
async fn get_task_git_diff(
    state: State<'_, AppState>,
    task_id: String,
    path: String,
    scope: String,
) -> Result<git::GitDiff, String> {
    let (task, host) = state.0.task_and_host(&task_id)?;
    tauri::async_runtime::spawn_blocking(move || git::diff(&host, &task.cwd, &path, &scope))
        .await
        .map_err(|error| format!("Git diff worker failed: {error}"))?
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let dir = app
                .path()
                .app_data_dir()
                .map_err(|e| format!("Cannot resolve app data folder: {e}"))?;
            let service = Service::open(Some(app.handle().clone()), dir)?;
            service.initialize_collaboration()?;
            app.manage(AppState(service));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            read_attachment_file,
            store_attachment,
            save_host,
            delete_host,
            probe_host,
            save_agent,
            delete_agent,
            create_task,
            save_project,
            delete_project,
            set_task_project,
            rename_task,
            delete_task,
            preview_task_deletion,
            delete_archived_task,
            set_task_archived,
            send_message,
            resume_task,
            cancel_task,
            save_settings,
            save_channel,
            send_channel_message,
            get_resume_command,
            get_task_goal,
            get_task_git_status,
            get_task_git_diff
        ])
        .build(tauri::generate_context!())
        .expect("error while running Monitter");
    app.run(|handle, event| {
        if matches!(
            event,
            tauri::RunEvent::Exit | tauri::RunEvent::ExitRequested { .. }
        ) {
            handle.state::<AppState>().0.cleanup();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("monitter-{name}-{}", id()))
    }

    #[test]
    fn attachment_ids_are_bound_to_the_exact_host_and_folder() {
        let attachment = attachments::Attachment {
            id: "attachment".into(),
            name: "x.txt".into(),
            mime_type: "text/plain".into(),
            size: 1,
            path: "/work/.monitter/attachments/x".into(),
            preview_data_url: None,
            source_id: None,
        };
        let mut registry = HashMap::new();
        registry.insert(
            attachment.id.clone(),
            attachments::StoredAttachment {
                attachment,
                host_id: "host-a".into(),
                cwd: "/work".into(),
            },
        );
        assert!(
            resolve_attachment_ids(&registry, "host-a", "/work", &["attachment".into()]).is_ok()
        );
        assert!(
            resolve_attachment_ids(&registry, "host-b", "/work", &["attachment".into()]).is_err()
        );
        assert!(
            resolve_attachment_ids(&registry, "host-a", "/other", &["attachment".into()]).is_err()
        );
        assert!(resolve_attachment_ids(&registry, "host-a", "/work", &["spoofed".into()]).is_err());
    }

    #[test]
    fn probe_deadline_reaps_hung_process() {
        let mut command = std::process::Command::new("sh");
        command.args(["-c", "sleep 1"]);
        let error = probe_output_with_timeout(command, Duration::from_millis(20)).unwrap_err();
        assert!(error.starts_with("CLI probe timed out after"));
    }

    #[test]
    fn probe_exit_error_is_provider_neutral() {
        let mut command = std::process::Command::new("sh");
        command.args(["-c", "exit 7"]);
        assert_eq!(
            probe_output_with_timeout(command, Duration::from_secs(1)).unwrap_err(),
            "CLI probe exited with exit status: 7."
        );
    }

    #[test]
    fn settings_validation_accepts_interface_scale_bounds() {
        let mut settings = default_snapshot().settings;
        settings.interface_scale = 80;
        assert!(validate_settings(&settings).is_ok());
        settings.interface_scale = 200;
        assert!(validate_settings(&settings).is_ok());
    }

    #[test]
    fn settings_validation_rejects_interface_scale_outside_bounds() {
        let mut settings = default_snapshot().settings;
        settings.interface_scale = 79;
        assert_eq!(
            validate_settings(&settings),
            Err("Interface scale must be between 80% and 200%.".into())
        );
        settings.interface_scale = 201;
        assert_eq!(
            validate_settings(&settings),
            Err("Interface scale must be between 80% and 200%.".into())
        );
    }

    #[test]
    fn task_snapshots_host_and_agent_instructions() {
        let dir = temp_dir("task-snapshot");
        let service = Service::open(None, dir.clone()).unwrap();
        let snapshot = service.snapshot().unwrap();
        let agent_id = snapshot.agents[0].id.clone();
        let host_id = snapshot.hosts[0].id.clone();
        service
            .mutate(None, |snapshot| {
                snapshot.agents[0].instructions = "Keep this instruction".into();
                snapshot.agents[0].cwd.clear();
                snapshot.hosts[0].default_cwd = "/first".into();
                Ok(())
            })
            .unwrap();
        let task = service
            .create_task(CreateTaskInput {
                agent_id,
                title: "snapshot".into(),
                native_session_id: None,
                parent_task_id: None,
                channel_id: None,
                project_id: None,
            })
            .unwrap();
        service
            .mutate(None, |snapshot| {
                snapshot.hosts[0].default_cwd = "/changed".into();
                Ok(())
            })
            .unwrap();
        let (stored_task, stored_host) = service.task_and_host(&task.id).unwrap();
        assert_eq!(stored_task.host_id, host_id);
        assert_eq!(stored_task.cwd, "/first");
        assert_eq!(stored_host.default_cwd, "/first");
        assert_eq!(
            service
                .snapshot()
                .unwrap()
                .messages
                .iter()
                .find(|message| message.task_id == task.id && message.role == "system")
                .unwrap()
                .text,
            "Monitter agent: Codex\n\nPurpose: Local Codex CLI\n\nKeep this instruction"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn one_writer_per_native_session() {
        let dir = temp_dir("single-writer");
        let service = Service::open(None, dir.clone()).unwrap();
        let agent_id = service.snapshot().unwrap().agents[0].id.clone();
        let make = || {
            service.create_task(CreateTaskInput {
                agent_id: agent_id.clone(),
                title: "attached".into(),
                native_session_id: Some("same-native".into()),
                parent_task_id: None,
                channel_id: None,
                project_id: None,
            })
        };
        let first = make().unwrap();
        let second = make().unwrap();
        service
            .mutate(None, |snapshot| {
                for task in &mut snapshot.tasks {
                    task.status = "running".into();
                }
                Ok(())
            })
            .unwrap();
        service.reserve_run(&first.id).unwrap();
        assert!(service.reserve_run(&second.id).is_err());
        service.release_run(&first.id);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn resume_rejections_leave_history_and_task_state_unchanged() {
        let cases = [
            (
                None,
                false,
                "completed",
                "Task has no native session ID yet.",
            ),
            (
                Some("native"),
                true,
                "completed",
                "Restore this archived task before resuming it.",
            ),
            (
                Some("native"),
                false,
                "running",
                "This task already has an active turn.",
            ),
        ];
        for (native, archived, status, expected) in cases {
            let dir = temp_dir("resume-rejection");
            let service = Service::open(None, dir.clone()).unwrap();
            let agent_id = service.snapshot().unwrap().agents[0].id.clone();
            let task = service
                .create_task(CreateTaskInput {
                    agent_id,
                    title: "resume rejection".into(),
                    native_session_id: native.map(str::to_owned),
                    parent_task_id: None,
                    channel_id: None,
                    project_id: None,
                })
                .unwrap();
            service
                .mutate(None, |snapshot| {
                    let task = snapshot
                        .tasks
                        .iter_mut()
                        .find(|item| item.id == task.id)
                        .unwrap();
                    task.archived = archived;
                    task.status = status.into();
                    Ok(())
                })
                .unwrap();
            let before = service.snapshot().unwrap();
            assert_eq!(service.resume(task.id.clone()), Err(expected.into()));
            assert_eq!(service.snapshot().unwrap(), before);
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn resume_reuses_native_session_and_records_the_continuation_turn() {
        let dir = temp_dir("resume-runner");
        std::fs::create_dir_all(&dir).unwrap();
        let executable = dir.join("fake-codex");
        let arguments = dir.join("arguments");
        let prompt = dir.join("prompt");
        std::fs::write(
            &executable,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\ncat > '{}'\nprintf '%s\\n' '{{\"type\":\"item.completed\",\"item\":{{\"type\":\"agent_message\",\"text\":\"resumed\"}}}}'\n",
                arguments.display(),
                prompt.display()
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        }

        let service = Service::open(None, dir.clone()).unwrap();
        let agent_id = service.snapshot().unwrap().agents[0].id.clone();
        let task = service
            .create_task(CreateTaskInput {
                agent_id,
                title: "resume runner".into(),
                native_session_id: Some("native-session".into()),
                parent_task_id: None,
                channel_id: None,
                project_id: None,
            })
            .unwrap();
        service
            .mutate_data(None, |data| {
                data.task_hosts.get_mut(&task.id).unwrap().codex_path =
                    executable.display().to_string();
                Ok(())
            })
            .unwrap();
        let before = service.snapshot().unwrap();
        service.resume(task.id.clone()).unwrap();
        wait_idle(&service, &task.id, 5).unwrap();

        let after = service.snapshot().unwrap();
        let resumed = after.tasks.iter().find(|item| item.id == task.id).unwrap();
        assert_eq!(resumed.id, task.id);
        assert_eq!(resumed.native_session_id.as_deref(), Some("native-session"));
        assert_eq!(resumed.status, "completed");
        assert_eq!(
            after.messages.len(),
            before.messages.len() + 2,
            "Resume adds its visible continuation and the real harness response."
        );
        assert_eq!(
            &after.messages[..before.messages.len()],
            before.messages.as_slice(),
            "Resume must preserve earlier transcript entries."
        );
        let continuation = &after.messages[before.messages.len()];
        assert_eq!(continuation.role, "user");
        assert_eq!(
            continuation.text,
            "Continue from where we left off. If the last request is complete, let me know and wait for my next instruction."
        );
        assert_eq!(after.messages.last().unwrap().role, "assistant");
        assert_eq!(after.messages.last().unwrap().text, "resumed");
        let arguments = std::fs::read_to_string(arguments)
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        assert_eq!(arguments[0], "exec");
        assert!(arguments.windows(2).any(|pair| pair == ["-s", "read-only"]));
        assert_eq!(
            &arguments[arguments.len() - 4..],
            ["resume", "--json", "native-session", "-"]
        );
        assert!(std::fs::read_to_string(prompt)
            .unwrap()
            .ends_with("User request:\nContinue from where we left off. If the last request is complete, let me know and wait for my next instruction.\n"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn archive_preserves_task_history_and_rejects_running_tasks() {
        let dir = temp_dir("archive");
        let service = Service::open(None, dir.clone()).unwrap();
        let agent_id = service.snapshot().unwrap().agents[0].id.clone();
        let task = service
            .create_task(CreateTaskInput {
                agent_id,
                title: "archive".into(),
                native_session_id: Some("native".into()),
                parent_task_id: None,
                channel_id: None,
                project_id: None,
            })
            .unwrap();
        service
            .apply_event(
                &task.id,
                Parsed {
                    native_session_id: None,
                    assistant: Some("saved".into()),
                    event: Some(("output".into(), "Assistant response".into(), "saved".into())),
                    failed: false,
                },
            )
            .unwrap();
        let before_archive = service.snapshot().unwrap();
        service.set_task_archived(&task.id, true).unwrap();
        let archived = service.snapshot().unwrap();
        assert!(
            archived
                .tasks
                .iter()
                .find(|item| item.id == task.id)
                .unwrap()
                .archived
        );
        assert_eq!(
            service.send(task.id.clone(), "hidden".into(), vec![]),
            Err("Restore this archived task before sending a message.".into())
        );
        assert_eq!(archived.messages, before_archive.messages);
        service
            .mutate(None, |snapshot| {
                snapshot
                    .tasks
                    .iter_mut()
                    .find(|item| item.id == task.id)
                    .unwrap()
                    .status = "running".into();
                Ok(())
            })
            .unwrap();
        let result = service.set_task_archived(&task.id, false);
        assert!(result.is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn session_keys_include_provider_and_host() {
        let snapshot = default_snapshot();
        let host = &snapshot.hosts[0];
        let mut task = task_from_agent(
            &snapshot.agents[0],
            &CreateTaskInput {
                agent_id: snapshot.agents[0].id.clone(),
                title: "one".into(),
                native_session_id: None,
                parent_task_id: None,
                channel_id: None,
                project_id: None,
            },
        );
        let codex = native_session_key(&task, host, "same");
        task.provider = "claude".into();
        assert_ne!(codex, native_session_key(&task, host, "same"));
        let mut other_host = host.clone();
        other_host.id = "other".into();
        assert_ne!(
            codex,
            native_session_key(
                &task_from_agent(
                    &snapshot.agents[0],
                    &CreateTaskInput {
                        agent_id: snapshot.agents[0].id.clone(),
                        title: "two".into(),
                        native_session_id: None,
                        parent_task_id: None,
                        channel_id: None,
                        project_id: None,
                    }
                ),
                &other_host,
                "same"
            )
        );
    }

    #[test]
    fn restart_marks_running_task_interrupted() {
        let dir = temp_dir("restart");
        let service = Service::open(None, dir.clone()).unwrap();
        let agent_id = service.snapshot().unwrap().agents[0].id.clone();
        let task = service
            .create_task(CreateTaskInput {
                agent_id,
                title: "running".into(),
                native_session_id: None,
                parent_task_id: None,
                channel_id: None,
                project_id: None,
            })
            .unwrap();
        service
            .mutate(None, |snapshot| {
                snapshot
                    .tasks
                    .iter_mut()
                    .find(|item| item.id == task.id)
                    .unwrap()
                    .status = "running".into();
                Ok(())
            })
            .unwrap();
        drop(service);
        let reopened = Service::open(None, dir.clone()).unwrap();
        assert_eq!(
            reopened
                .snapshot()
                .unwrap()
                .tasks
                .iter()
                .find(|item| item.id == task.id)
                .unwrap()
                .status,
            "interrupted"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn identical_assistant_text_from_separate_turns_is_preserved() {
        let dir = temp_dir("repeat-output");
        let service = Service::open(None, dir.clone()).unwrap();
        let agent_id = service.snapshot().unwrap().agents[0].id.clone();
        let task = service
            .create_task(CreateTaskInput {
                agent_id,
                title: "repeated output".into(),
                native_session_id: None,
                parent_task_id: None,
                channel_id: None,
                project_id: None,
            })
            .unwrap();
        for _ in 0..2 {
            service
                .apply_event(
                    &task.id,
                    Parsed {
                        native_session_id: None,
                        assistant: Some("same answer".into()),
                        event: Some((
                            "output".into(),
                            "Assistant response".into(),
                            "same answer".into(),
                        )),
                        failed: false,
                    },
                )
                .unwrap();
        }
        assert_eq!(
            service
                .snapshot()
                .unwrap()
                .messages
                .iter()
                .filter(|message| message.task_id == task.id && message.role == "assistant")
                .count(),
            2
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn failed_mutation_does_not_change_live_or_persisted_state() {
        let dir = temp_dir("transaction");
        let service = Service::open(None, dir.clone()).unwrap();
        let before = service.snapshot().unwrap();
        let result: Result<(), String> = service.mutate(None, |snapshot| {
            snapshot.settings.theme = "light".into();
            Err("deliberate failure".into())
        });
        assert!(result.is_err());
        assert_eq!(service.snapshot().unwrap(), before);
        drop(service);
        assert_eq!(
            Service::open(None, dir.clone())
                .unwrap()
                .snapshot()
                .unwrap(),
            before
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    fn project(id: &str, host_id: &str, cwd: &str) -> Project {
        Project {
            id: id.into(),
            name: "  Product work  ".into(),
            description: "Shared work area".into(),
            workspaces: vec![ProjectWorkspace {
                host_id: host_id.into(),
                cwd: cwd.into(),
            }],
        }
    }

    fn task_input(agent_id: String, title: &str, project_id: Option<&str>) -> CreateTaskInput {
        CreateTaskInput {
            agent_id,
            title: title.into(),
            native_session_id: None,
            parent_task_id: None,
            channel_id: None,
            project_id: project_id.map(str::to_owned),
        }
    }

    #[test]
    fn project_workspaces_apply_to_new_tasks_without_retroactive_changes() {
        let dir = temp_dir("project-workspaces");
        let service = Service::open(None, dir.clone()).unwrap();
        let before = service.snapshot().unwrap();
        let host_id = before.hosts[0].id.clone();
        let agent_a = before.agents[0].id.clone();
        let mut agent_b = before.agents[0].clone();
        agent_b.id = id();
        agent_b.name = "Second agent".into();
        service
            .mutate(None, |snapshot| {
                snapshot.agents.push(agent_b.clone());
                Ok(())
            })
            .unwrap();
        let saved = service
            .save_project(project("project", &host_id, "/project-a"))
            .unwrap();
        assert_eq!(saved.projects[0].name, "Product work");

        let first = service
            .create_task(task_input(agent_a, "first", Some("project")))
            .unwrap();
        let second = service
            .create_task(task_input(agent_b.id, "second", Some("project")))
            .unwrap();
        assert_eq!(first.cwd, "/project-a");
        assert_eq!(second.cwd, "/project-a");

        service
            .mutate(None, |snapshot| {
                let task = snapshot
                    .tasks
                    .iter_mut()
                    .find(|task| task.id == first.id)
                    .unwrap();
                task.status = "running".into();
                task.native_session_id = Some("native-session".into());
                snapshot.messages.push(Message {
                    sender_agent_id: None,
                    collaboration_id: None,
                    id: id(),
                    task_id: first.id.clone(),
                    role: "assistant".into(),
                    text: "kept transcript".into(),
                    created_at: now(),
                    attachments: vec![],
                });
                Ok(())
            })
            .unwrap();
        let before_move = service.snapshot().unwrap();
        let original = before_move
            .tasks
            .iter()
            .find(|task| task.id == first.id)
            .unwrap()
            .clone();
        service.set_task_project(&first.id, None).unwrap();
        service
            .set_task_project(&first.id, Some("project".into()))
            .unwrap();
        let moved = service.snapshot().unwrap();
        let after_move = moved.tasks.iter().find(|task| task.id == first.id).unwrap();
        assert_eq!(after_move.cwd, original.cwd);
        assert_eq!(after_move.host_id, original.host_id);
        assert_eq!(after_move.status, original.status);
        assert_eq!(after_move.native_session_id, original.native_session_id);
        assert!(moved
            .messages
            .iter()
            .any(|message| message.task_id == first.id && message.text == "kept transcript"));

        service
            .save_project(project("project", &host_id, "/project-b"))
            .unwrap();
        let future = service
            .create_task(task_input(
                before.agents[0].id.clone(),
                "future",
                Some("project"),
            ))
            .unwrap();
        assert_eq!(future.cwd, "/project-b");
        assert_eq!(
            service
                .snapshot()
                .unwrap()
                .tasks
                .iter()
                .find(|task| task.id == first.id)
                .unwrap()
                .cwd,
            "/project-a"
        );

        let deleted = service.delete_project("project").unwrap();
        assert!(deleted.projects.is_empty());
        assert!(deleted
            .tasks
            .iter()
            .filter(|task| task.id == first.id || task.id == second.id || task.id == future.id)
            .all(|task| task.project_id.is_none()));
        assert!(deleted
            .messages
            .iter()
            .any(|message| message.task_id == first.id && message.text == "kept transcript"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn project_validation_rejects_unknown_or_duplicate_workspace_hosts_and_unknown_ids() {
        let dir = temp_dir("project-validation");
        let service = Service::open(None, dir.clone()).unwrap();
        let snapshot = service.snapshot().unwrap();
        let host_id = snapshot.hosts[0].id.clone();
        assert_eq!(
            service.save_project(project("unknown-host", "missing", "/work")),
            Err("Project workspace host was not found.".into())
        );
        let mut duplicate = project("duplicate", &host_id, "/one");
        duplicate.workspaces.push(ProjectWorkspace {
            host_id,
            cwd: "/two".into(),
        });
        assert_eq!(
            service.save_project(duplicate),
            Err("Project can have only one workspace per host.".into())
        );
        assert_eq!(
            service.create_task(task_input(
                snapshot.agents[0].id.clone(),
                "missing",
                Some("missing")
            )),
            Err("Project was not found.".into())
        );
        assert_eq!(
            service.set_task_project("missing", None),
            Err("Task was not found.".into())
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn agent_avatar_validation_allows_supported_data_urls_and_removal() {
        assert!(validate_agent_avatar(None).is_ok());
        for avatar in [
            "data:image/png;base64,AA==",
            "data:image/jpeg;base64,/9j/",
            "data:image/webp;base64,UklGRg==",
        ] {
            assert!(validate_agent_avatar(Some(avatar)).is_ok());
        }
    }

    #[test]
    fn agent_avatar_validation_rejects_unsupported_malformed_or_oversized_values() {
        assert_eq!(
            validate_agent_avatar(Some("https://example.com/avatar.png")),
            Err("Agent avatar must be a PNG, JPEG, or WebP data URL.".into())
        );
        assert_eq!(
            validate_agent_avatar(Some("data:image/svg+xml;base64,PHN2Zz4=")),
            Err("Agent avatar must be a PNG, JPEG, or WebP data URL.".into())
        );
        assert_eq!(
            validate_agent_avatar(Some("data:image/png;base64,not base64!")),
            Err("Agent avatar contains invalid base64 data.".into())
        );
        let oversized = format!(
            "data:image/png;base64,{}",
            "A".repeat(MAX_AVATAR_DATA_URL_BYTES)
        );
        assert_eq!(
            validate_agent_avatar(Some(&oversized)),
            Err("Agent avatar is too large.".into())
        );
    }
}
