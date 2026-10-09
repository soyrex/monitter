//! Bounded, dormant import of an already-mapped Monitter history bundle.
//!
//! This module only validates and persists records. It does not open native
//! sessions, register a runtime, enqueue a prompt, or start a provider.

use crate::{
    attachments,
    history::{History, HistoryRecord},
    model::{self, Agent, Host, Message, Project, RunEvent, Task},
    Service, ServiceData,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    io::{self, Write},
    path::{Component, PathBuf},
    sync::Arc,
};
use uuid::Uuid;

const MAX_HOSTS: usize = 200;
const MAX_AGENTS: usize = 200;
const MAX_PROJECTS: usize = 200;
const MAX_TASKS: usize = 500;
const MAX_MESSAGES: usize = 50_000;
const MAX_EVENTS: usize = 50_000;
const MAX_ATTACHMENTS: usize = 2_000;
const MAX_AGGREGATE_BYTES: u64 = 24 * 1024 * 1024;
const MAX_GENERAL_TEXT_BYTES: usize = 256 * 1024;
const MAX_MODEL_BYTES: usize = 1024;
const MAX_NATIVE_SESSION_ID_BYTES: usize = 4096;

/// Records must already have destination UUIDs mapped by the trusted import
/// adapter. There is no source database selector or arbitrary source path.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct GpuiHistoryImport {
    pub hosts: Vec<Host>,
    pub agents: Vec<Agent>,
    pub projects: Vec<Project>,
    pub tasks: Vec<Task>,
    pub messages: Vec<Message>,
    pub events: Vec<RunEvent>,
    /// Existing attachment metadata only. Import never reads or writes these
    /// paths; later reads remain fenced by host, cwd, and managed directory.
    #[serde(default)]
    pub attachments: Vec<attachments::StoredAttachment>,
    /// Exact host snapshot for each imported task. Must have exactly the same
    /// task keys as `tasks`, and each host ID must match `Task.host_id`.
    pub task_hosts: HashMap<String, Host>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GpuiHistoryImportReport {
    pub hosts_added: usize,
    pub agents_added: usize,
    pub projects_added: usize,
    pub tasks_added: usize,
    pub messages_added: usize,
    pub events_added: usize,
    pub attachments_added: usize,
}

#[derive(Default)]
struct AddCounts {
    hosts: usize,
    agents: usize,
    projects: usize,
    tasks: usize,
    messages: usize,
    events: usize,
    attachments: usize,
}

impl AddCounts {
    fn report(self) -> GpuiHistoryImportReport {
        GpuiHistoryImportReport {
            hosts_added: self.hosts,
            agents_added: self.agents,
            projects_added: self.projects,
            tasks_added: self.tasks,
            messages_added: self.messages,
            events_added: self.events,
            attachments_added: self.attachments,
        }
    }
}

impl Service {
    /// Atomically append imported, dormant history after validating the entire
    /// bundle against both itself and current state. Same-ID exact matches are
    /// idempotent; any divergent collision rejects the whole import.
    pub(crate) fn import_gpui_history(
        &self,
        mut input: GpuiHistoryImport,
    ) -> Result<GpuiHistoryImportReport, String> {
        validate_bundle_bounds(&input)?;
        normalize_import_messages(&mut input.messages)?;
        validate_bundle_records(&input)?;

        self.mutate_data(None, |data| {
            let validation = self.import_validation_data(&input, data)?;
            validate_against_current(&input, &validation)?;
            let mut counts = AddCounts::default();

            append_vec(
                &input.hosts,
                &mut data.snapshot.hosts,
                |item| &item.id,
                &mut counts.hosts,
            );
            append_vec(
                &input.agents,
                &mut data.snapshot.agents,
                |item| &item.id,
                &mut counts.agents,
            );
            append_vec(
                &input.projects,
                &mut data.snapshot.projects,
                |item| &item.id,
                &mut counts.projects,
            );
            append_vec(
                &input.tasks,
                &mut data.snapshot.tasks,
                |item| &item.id,
                &mut counts.tasks,
            );
            append_history(
                &input.messages,
                &mut data.snapshot.messages,
                |item| &item.id,
                &mut counts.messages,
            );

            let mut event_ids = data
                .snapshot
                .events
                .iter()
                .map(|event| event.id.clone())
                .collect::<HashSet<_>>();
            for event in &input.events {
                if event_ids.insert(event.id.clone()) {
                    data.snapshot.events.push(Arc::new(event.clone()));
                    counts.events += 1;
                }
            }
            for (task_id, host) in &input.task_hosts {
                data.task_hosts
                    .entry(task_id.clone())
                    .or_insert_with(|| host.clone());
            }
            for attachment in &input.attachments {
                if !data.attachments.contains_key(&attachment.attachment.id) {
                    data.attachments
                        .insert(attachment.attachment.id.clone(), attachment.clone());
                    counts.attachments += 1;
                }
            }
            Ok(counts.report())
        })
    }

    /// Hydrate only colliding IDs for exact durable comparison after eviction.
    fn import_validation_data(&self, input: &GpuiHistoryImport, data: &ServiceData) -> Result<ServiceData, String> {
        let mut validation = data.clone();
        for incoming in &input.messages {
            if validation.snapshot.messages.payload_is_cold(&incoming.id) {
                if let Some(index) = validation.snapshot.messages.iter().position(|row| row.id == incoming.id) {
                    let task_id = validation.snapshot.messages[index].task_id.clone();
                    validation.snapshot.messages[index] = self.store.task_message(&task_id, &incoming.id)?;
                }
            }
        }
        for incoming in &input.events {
            if validation.snapshot.events.payload_is_cold(&incoming.id) {
                if let Some(index) = validation.snapshot.events.iter().position(|row| row.id == incoming.id) {
                    let task_id = validation.snapshot.events[index].task_id.clone();
                    validation.snapshot.events[index] = Arc::new(self.store.task_event(&task_id, &incoming.id)?);
                }
            }
        }
        Ok(validation)
    }

    /// Read-only preflight for a prepared chunk. Uses the exact mutation
    /// validator without changing the snapshot, registries, or revision.
    pub(crate) fn validate_gpui_history(&self, mut input: GpuiHistoryImport) -> Result<(), String> {
        validate_bundle_bounds(&input)?;
        normalize_import_messages(&mut input.messages)?;
        validate_bundle_records(&input)?;
        let _writer = self.state_writes.lock().map_err(|_| "State writer lock failed.")?;
        let data = self.committed_data()?;
        let validation = self.import_validation_data(&input, &data)?;
        validate_against_current(&input, &validation)
    }
}

fn append_history<T: HistoryRecord + Clone>(
    incoming: &[T],
    existing: &mut History<T>,
    id: impl Fn(&T) -> &str,
    added: &mut usize,
) {
    let existing_ids = existing
        .iter()
        .map(|item| item.id().to_owned())
        .collect::<HashSet<_>>();
    for item in incoming {
        if !existing_ids.contains(id(item)) {
            existing.push(item.clone());
            *added += 1;
        }
    }
}

// The Snapshot's high-volume collections are History<T>; small metadata
// collections use Vec<T>. Separate helpers keep append order explicit.
fn append_vec<T: Clone>(
    incoming: &[T],
    existing: &mut Vec<T>,
    id: impl Fn(&T) -> &str,
    added: &mut usize,
) {
    let existing_ids = existing
        .iter()
        .map(|item| id(item).to_owned())
        .collect::<HashSet<_>>();
    for item in incoming {
        if !existing_ids.contains(id(item)) {
            existing.push(item.clone());
            *added += 1;
        }
    }
}

fn ensure_vec_collisions<T: PartialEq>(
    incoming: &[T],
    existing: &[T],
    id: impl Fn(&T) -> &str,
    label: &str,
) -> Result<(), String> {
    let by_id = existing
        .iter()
        .map(|item| (id(item).to_owned(), item))
        .collect::<HashMap<_, _>>();
    for item in incoming {
        if by_id.get(id(item)).is_some_and(|old| **old != *item) {
            return Err(format!(
                "History import {label} ID collides with different saved data."
            ));
        }
    }
    Ok(())
}

fn ensure_history_collisions<T: HistoryRecord + PartialEq>(
    incoming: &[T],
    existing: &History<T>,
    label: &str,
) -> Result<(), String> {
    let by_id = existing
        .iter()
        .map(|item| (item.id().to_owned(), item))
        .collect::<HashMap<_, _>>();
    for item in incoming {
        if by_id.get(item.id()).is_some_and(|old| **old != *item) {
            return Err(format!(
                "History import {label} ID collides with different saved data."
            ));
        }
    }
    Ok(())
}

fn validate_bundle_bounds(input: &GpuiHistoryImport) -> Result<(), String> {
    for (name, actual, limit) in [
        ("hosts", input.hosts.len(), MAX_HOSTS),
        ("agents", input.agents.len(), MAX_AGENTS),
        ("projects", input.projects.len(), MAX_PROJECTS),
        ("tasks", input.tasks.len(), MAX_TASKS),
        ("messages", input.messages.len(), MAX_MESSAGES),
        ("events", input.events.len(), MAX_EVENTS),
        ("attachments", input.attachments.len(), MAX_ATTACHMENTS),
    ] {
        if actual > limit {
            return Err(format!("History import has too many {name}."));
        }
    }
    if input.task_hosts.len() != input.tasks.len() {
        return Err("History import requires exactly one saved host for each task.".into());
    }
    let mut counter = BoundedCounter { bytes: 0 };
    serde_json::to_writer(&mut counter, input)
        .map_err(|_| "History import exceeds the 24 MiB size limit.".to_string())?;
    Ok(())
}

struct BoundedCounter {
    bytes: u64,
}

impl Write for BoundedCounter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let next = self.bytes.saturating_add(bytes.len() as u64);
        if next > MAX_AGGREGATE_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "aggregate import limit exceeded",
            ));
        }
        self.bytes = next;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn normalize_import_messages(messages: &mut [Message]) -> Result<(), String> {
    for message in messages {
        if message.stream_status.as_deref() == Some("streaming") {
            message.stream_status = Some("interrupted".into());
        }
    }
    Ok(())
}

fn validate_uuid(value: &str, label: &str) -> Result<(), String> {
    let parsed = Uuid::parse_str(value)
        .map_err(|_| format!("History import has an invalid {label} UUID."))?;
    if parsed.to_string() != value {
        return Err(format!(
            "History import {label} UUID must use canonical lowercase form."
        ));
    }
    Ok(())
}

fn validate_text(value: &str, label: &str, limit: usize, allow_empty: bool) -> Result<(), String> {
    if (!allow_empty && value.trim().is_empty()) || value.len() > limit || value.contains('\0') {
        return Err(format!("History import has invalid or oversized {label}."));
    }
    Ok(())
}

fn validate_attachment_record(attachment: &attachments::Attachment) -> Result<(), String> {
    validate_uuid(&attachment.id, "attachment")?;
    let safe_name = attachments::safe_name(&attachment.name)?;
    if safe_name != attachment.name {
        return Err("History import attachment name is not in canonical safe form.".into());
    }
    validate_text(&attachment.mime_type, "attachment MIME type", 256, false)?;
    if attachment.size > attachments::MAX_BYTES {
        return Err("History import attachment exceeds the 20 MiB limit.".into());
    }
    if let Some(source_id) = attachment.source_id.as_deref() {
        validate_uuid(source_id, "attachment source")?;
    }
    Ok(())
}

fn validate_message_attachment(attachment: &attachments::Attachment) -> Result<(), String> {
    validate_attachment_record(attachment)?;
    match attachment.path.as_str() {
        "Generated by Codex" | "Generated by ACP agent" => {
            let preview = attachment
                .preview_data_url
                .as_deref()
                .ok_or("Generated image preview is missing.")?;
            if preview.len() > attachments::MAX_GENERATED_IMAGE_DATA_URL_BYTES {
                return Err("History import generated image exceeds the inline limit.".into());
            }
            let payload = [
                "data:image/png;base64,",
                "data:image/jpeg;base64,",
                "data:image/webp;base64,",
            ]
            .iter()
            .find_map(|prefix| preview.strip_prefix(prefix))
            .ok_or("Generated image preview has an unsupported data URL.")?;
            let validated = attachments::generated_image(payload)?;
            if attachment.mime_type != validated.mime_type
                || attachment.size != validated.size
                || attachment.name != validated.name
                || attachment.source_id.is_some()
            {
                return Err("Generated image metadata does not match its bounded preview.".into());
            }
            Ok(())
        }
        _ => attachments::validate_preview(attachment.preview_data_url.as_deref()),
    }
}

fn normalize_import_path(value: &str, label: &str) -> Result<PathBuf, String> {
    validate_text(value, label, MAX_GENERAL_TEXT_BYTES, false)?;
    let expanded = if value == "~" || value.starts_with("~/") {
        let home =
            std::env::var_os("HOME").ok_or("Could not validate attachment workspace path.")?;
        let home = PathBuf::from(home);
        if value == "~" {
            home
        } else {
            home.join(&value[2..])
        }
    } else if value.starts_with('~') {
        return Err("History import path uses an unsupported home-directory form.".into());
    } else {
        PathBuf::from(value)
    };
    if !expanded.is_absolute() {
        return Err("History import attachment path must be absolute.".into());
    }
    let mut normalized = PathBuf::new();
    for component in expanded.components() {
        match component {
            Component::RootDir => normalized.push("/"),
            Component::Normal(part) => normalized.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::Prefix(_) => {
                return Err("History import path contains an unsafe component.".into());
            }
        }
    }
    Ok(normalized)
}

fn validate_stored_attachment(stored: &attachments::StoredAttachment) -> Result<(), String> {
    validate_attachment_record(&stored.attachment)?;
    validate_uuid(&stored.host_id, "stored attachment host")?;
    validate_text(
        &stored.cwd,
        "stored attachment folder",
        MAX_GENERAL_TEXT_BYTES,
        false,
    )?;
    let name = attachments::safe_name(&stored.attachment.name)?;
    let path = normalize_import_path(&stored.attachment.path, "stored attachment path")?;
    if !managed_attachment_directory(path.parent()) {
        return Err(
            "History import attachment path is outside a managed attachment folder.".into(),
        );
    }
    let filename = path
        .file_name()
        .and_then(|part| part.to_str())
        .ok_or("History import attachment filename is invalid.")?;
    let prefix = filename
        .as_bytes()
        .get(..36)
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
        .ok_or("History import attachment filename has no generated UUID prefix.")?;
    validate_uuid(prefix, "stored filename")?;
    let uuid_len = 36;
    if filename.len() <= uuid_len + 1
        || filename.as_bytes().get(uuid_len) != Some(&b'-')
        || &filename[uuid_len + 1..] != name
    {
        return Err("History import attachment filename does not match its managed name.".into());
    }
    attachments::validate_preview(stored.attachment.preview_data_url.as_deref())
}

fn managed_attachment_directory(path: Option<&std::path::Path>) -> bool {
    let Some(path) = path else { return false };
    path.file_name().is_some_and(|part| part == "attachments")
        && path
            .parent()
            .is_some_and(|parent| parent.file_name().is_some_and(|part| part == ".monitter"))
}

fn validate_stored_attachment_scope(
    stored: &attachments::StoredAttachment,
    host: &Host,
) -> Result<(), String> {
    let path = normalize_import_path(&stored.attachment.path, "stored attachment path")?;
    if host.kind == "local" {
        let cwd = normalize_import_path(&stored.cwd, "stored attachment folder")?;
        let expected = cwd.join(".monitter").join("attachments");
        if path.parent() != Some(expected.as_path()) {
            return Err(
                "History import local attachment path does not match its task folder.".into(),
            );
        }
    } else if stored.cwd.starts_with('~') {
        // The remote account's home directory is not available locally. Its
        // reader still resolves the path only beneath realpath(cwd)/.monitter.
        if !managed_attachment_directory(path.parent()) {
            return Err(
                "History import remote attachment path is outside its managed folder.".into(),
            );
        }
    } else {
        let cwd = normalize_import_path(&stored.cwd, "stored attachment folder")?;
        let expected = cwd.join(".monitter").join("attachments");
        if path.parent() != Some(expected.as_path()) {
            return Err(
                "History import remote attachment path does not match its task folder.".into(),
            );
        }
    }
    Ok(())
}

fn ensure_unique_ids<T>(
    items: &[T],
    get_id: impl Fn(&T) -> &str,
    label: &str,
) -> Result<(), String> {
    let mut seen = HashSet::with_capacity(items.len());
    for item in items {
        let value = get_id(item);
        validate_uuid(value, label)?;
        if !seen.insert(value) {
            return Err(format!("History import contains duplicate {label} IDs."));
        }
    }
    Ok(())
}

fn validate_bundle_records(input: &GpuiHistoryImport) -> Result<(), String> {
    ensure_unique_ids(&input.hosts, |host| &host.id, "host")?;
    ensure_unique_ids(&input.agents, |agent| &agent.id, "agent")?;
    ensure_unique_ids(&input.projects, |project| &project.id, "project")?;
    ensure_unique_ids(&input.tasks, |task| &task.id, "task")?;
    ensure_unique_ids(&input.messages, |message| &message.id, "message")?;
    ensure_unique_ids(&input.events, |event| &event.id, "event")?;
    ensure_unique_ids(&input.attachments, |item| &item.attachment.id, "attachment")?;

    let task_ids = input
        .tasks
        .iter()
        .map(|item| item.id.as_str())
        .collect::<HashSet<_>>();

    for host in &input.hosts {
        validate_host(host)?;
    }
    for agent in &input.agents {
        validate_agent(agent)?;
        validate_uuid(&agent.host_id, "agent host")?;
        if agent.internal {
            return Err("History import cannot add the internal Monitter Admin agent.".into());
        }
    }
    for project in &input.projects {
        validate_project_record(project)?;
        for workspace in &project.workspaces {
            validate_uuid(&workspace.host_id, "project workspace host")?;
        }
    }
    for task in &input.tasks {
        validate_task(task)?;
        validate_uuid(&task.agent_id, "task agent")?;
        validate_uuid(&task.host_id, "task host")?;
        if let Some(parent) = task.parent_task_id.as_deref() {
            validate_uuid(parent, "parent task")?;
        }
        if let Some(channel) = task.channel_id.as_deref() {
            validate_uuid(channel, "channel")?;
        }
        if let Some(project) = task.project_id.as_deref() {
            validate_uuid(project, "task project")?;
        }

        let saved_host = input
            .task_hosts
            .get(&task.id)
            .ok_or("History import task host is missing.")?;
        if saved_host.id != task.host_id {
            return Err("History import task host ID does not match the task snapshot.".into());
        }
        validate_host(saved_host)?;
    }
    for message in &input.messages {
        validate_uuid(&message.task_id, "message task")?;
        if !task_ids.contains(message.task_id.as_str()) {
            return Err("History import message task is missing from the task definitions.".into());
        }
        validate_text(
            &message.text,
            "message text",
            MAX_AGGREGATE_BYTES as usize,
            true,
        )?;
        if !matches!(message.role.as_str(), "user" | "assistant" | "system") {
            return Err("History import has an unsupported message role.".into());
        }
        if let Some(id) = message.sender_agent_id.as_deref() {
            validate_uuid(id, "message sender")?;
        }
        if let Some(id) = message.collaboration_id.as_deref() {
            validate_uuid(id, "message collaboration")?;
        }
        if let Some(phase) = message.phase.as_deref() {
            if !matches!(phase, "commentary" | "final_answer") {
                return Err("History import has an unsupported assistant phase.".into());
            }
        }
        if message
            .stream_status
            .as_deref()
            .is_some_and(|status| !matches!(status, "complete" | "interrupted"))
        {
            return Err("History import has an unsupported stream status.".into());
        }
        if message.attachments.len() > 64 {
            return Err("History import message has too many attachments.".into());
        }
        for attachment in &message.attachments {
            validate_message_attachment(attachment)?;
        }
    }
    for attachment in &input.attachments {
        validate_stored_attachment(attachment)?;
    }
    for event in &input.events {
        validate_uuid(&event.task_id, "event task")?;
        if !task_ids.contains(event.task_id.as_str()) {
            return Err("History import event task is missing from the task definitions.".into());
        }
        validate_text(&event.kind, "event kind", 256, false)?;
        // Existing tool events can include multi-line command titles. Retain
        // those historical records within the bounded import frame.
        validate_text(&event.title, "event title", 8192, true)?;
        validate_text(
            &event.detail,
            "event detail",
            MAX_AGGREGATE_BYTES as usize,
            true,
        )?;
    }
    for (task_id, host) in &input.task_hosts {
        validate_uuid(task_id, "task host key")?;
        if !task_ids.contains(task_id.as_str())
            || host.id
                != input
                    .tasks
                    .iter()
                    .find(|task| task.id == *task_id)
                    .unwrap()
                    .host_id
        {
            return Err("History import has an unexpected or mismatched saved task host.".into());
        }
    }
    validate_import_parent_cycles(&input.tasks)?;
    Ok(())
}

fn validate_host(host: &Host) -> Result<(), String> {
    if !matches!(host.kind.as_str(), "local" | "ssh") {
        return Err("History import host kind must be local or ssh.".into());
    }
    for (value, label) in [
        (&host.name, "host name"),
        (&host.address, "host address"),
        (&host.user, "host user"),
        (&host.identity_file, "host identity path"),
        (&host.default_cwd, "host default folder"),
        (&host.codex_path, "Codex executable path"),
        (&host.claude_path, "Claude executable path"),
        (&host.opencode_path, "OpenCode executable path"),
        (&host.hermes_path, "Hermes executable path"),
    ] {
        validate_text(value, label, MAX_GENERAL_TEXT_BYTES, true)?;
    }
    Ok(())
}

fn validate_agent(agent: &Agent) -> Result<(), String> {
    validate_text(&agent.name, "agent name", 240, false)?;
    validate_text(&agent.description, "agent description", 8192, true)?;
    validate_text(
        &agent.instructions,
        "agent instructions",
        MAX_GENERAL_TEXT_BYTES,
        true,
    )?;
    validate_text(&agent.model, "agent model", MAX_MODEL_BYTES, true)?;
    validate_text(&agent.cwd, "agent folder", MAX_GENERAL_TEXT_BYTES, false)?;
    if let Some(codex_home) = agent.codex_home.as_deref() {
        validate_text(
            codex_home,
            "agent Codex home",
            MAX_GENERAL_TEXT_BYTES,
            false,
        )?;
    }
    if !model::known_provider(&agent.provider)
        || !model::valid_sandbox_for_provider(&agent.provider, &agent.sandbox)
    {
        return Err("History import agent has an unsupported provider/sandbox combination.".into());
    }
    if agent.provider == "acp" {
        if !agent.acp.as_ref().is_some_and(model::valid_acp_launch) {
            return Err("History import ACP agent has an invalid launch definition.".into());
        }
    } else if agent.acp.is_some() {
        return Err("History import non-ACP agent has an ACP launch definition.".into());
    }
    crate::validate_agent_avatar(agent.avatar.as_deref())?;
    crate::validate_collaboration_profile(agent)?;
    Ok(())
}

fn validate_project_record(project: &Project) -> Result<(), String> {
    validate_text(&project.name, "project name", 512, false)?;
    validate_text(&project.description, "project description", 8192, true)?;
    if project.workspaces.len() > MAX_HOSTS {
        return Err("History import project has too many workspaces.".into());
    }
    let color = project.color.trim();
    if color.len() != 7
        || !color.starts_with('#')
        || !color[1..].bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("History import project color is invalid.".into());
    }
    if !matches!(
        project.icon.as_str(),
        "folder"
            | "briefcase"
            | "code"
            | "rocket"
            | "globe"
            | "palette"
            | "database"
            | "wrench"
            | "layers"
    ) {
        return Err("History import project icon is unsupported.".into());
    }
    if let Some(text) = project.board_guidance_text.as_deref() {
        validate_text(text, "project guidance", 64 * 1024, true)?;
    }
    for workspace in &project.workspaces {
        validate_text(
            &workspace.cwd,
            "project workspace folder",
            MAX_GENERAL_TEXT_BYTES,
            false,
        )?;
    }
    Ok(())
}

fn validate_task(task: &Task) -> Result<(), String> {
    if matches!(task.status.as_str(), "running" | "queued" | "sending") {
        return Err("History import cannot restore queued or running work.".into());
    }
    if !matches!(
        task.status.as_str(),
        "idle" | "completed" | "error" | "interrupted"
    ) {
        return Err("History import has an unsupported task status.".into());
    }
    validate_text(&task.title, "task title", 4096, false)?;
    validate_text(&task.cwd, "task folder", MAX_GENERAL_TEXT_BYTES, false)?;
    validate_text(&task.model, "task model", MAX_MODEL_BYTES, true)?;
    if let Some(codex_home) = task.codex_home.as_deref() {
        validate_text(codex_home, "task Codex home", MAX_GENERAL_TEXT_BYTES, false)?;
    }
    validate_text(&task.provider, "task provider", 64, false)?;
    if !model::known_provider(&task.provider)
        || !model::valid_sandbox_for_provider(&task.provider, &task.sandbox)
    {
        return Err("History import task has an unsupported provider/sandbox combination.".into());
    }
    if task.provider == "acp" {
        if !task.acp.as_ref().is_some_and(model::valid_acp_launch) {
            return Err("History import ACP task has an invalid saved launch definition.".into());
        }
    } else if task.acp.is_some() {
        return Err("History import non-ACP task has an ACP launch definition.".into());
    }
    if task.provider != "codex" && task.codex_home.is_some() {
        return Err("History import non-Codex task contains a Codex home.".into());
    }
    if task.native_session_id.as_deref().is_some_and(|id| {
        id.is_empty() || id.len() > MAX_NATIVE_SESSION_ID_BYTES || id.contains('\0')
    }) {
        return Err("History import native session reference is invalid.".into());
    }
    Ok(())
}

fn validate_import_parent_cycles(tasks: &[Task]) -> Result<(), String> {
    let parents = tasks
        .iter()
        .map(|task| (task.id.as_str(), task.parent_task_id.as_deref()))
        .collect::<HashMap<_, _>>();
    for task in tasks {
        let mut visited = HashSet::new();
        let mut cursor = Some(task.id.as_str());
        while let Some(id) = cursor {
            if !visited.insert(id) {
                return Err("History import parent-task links contain a cycle.".into());
            }
            cursor = parents.get(id).copied().flatten();
        }
    }
    Ok(())
}

fn validate_against_current(input: &GpuiHistoryImport, data: &ServiceData) -> Result<(), String> {
    let snapshot = &data.snapshot;
    ensure_vec_collisions(&input.hosts, &snapshot.hosts, |item| &item.id, "host")?;
    ensure_vec_collisions(&input.agents, &snapshot.agents, |item| &item.id, "agent")?;
    ensure_vec_collisions(
        &input.projects,
        &snapshot.projects,
        |item| &item.id,
        "project",
    )?;
    ensure_vec_collisions(&input.tasks, &snapshot.tasks, |item| &item.id, "task")?;
    ensure_history_collisions(&input.messages, &snapshot.messages, "message")?;
    for stored in &input.attachments {
        if data
            .attachments
            .get(&stored.attachment.id)
            .is_some_and(|existing| existing != stored)
        {
            return Err("History import attachment ID collides with different saved data.".into());
        }
        let host = input
            .hosts
            .iter()
            .find(|host| host.id == stored.host_id)
            .or_else(|| snapshot.hosts.iter().find(|host| host.id == stored.host_id))
            .ok_or("History import attachment refers to an unknown host.")?;
        validate_stored_attachment_scope(stored, host)?;
    }
    let event_ids = snapshot
        .events
        .iter()
        .map(|item| (item.id.as_str(), item.as_ref()))
        .collect::<HashMap<_, _>>();
    for event in &input.events {
        if event_ids
            .get(event.id.as_str())
            .is_some_and(|old| **old != *event)
        {
            return Err("History import event ID collides with different saved data.".into());
        }
    }

    let imported_hosts = input
        .hosts
        .iter()
        .map(|item| item.id.as_str())
        .collect::<HashSet<_>>();
    let imported_agents = input
        .agents
        .iter()
        .map(|item| item.id.as_str())
        .collect::<HashSet<_>>();
    let imported_projects = input
        .projects
        .iter()
        .map(|item| item.id.as_str())
        .collect::<HashSet<_>>();
    let imported_tasks = input
        .tasks
        .iter()
        .map(|item| item.id.as_str())
        .collect::<HashSet<_>>();
    let current_hosts = snapshot
        .hosts
        .iter()
        .map(|item| item.id.as_str())
        .collect::<HashSet<_>>();
    let current_agents = snapshot
        .agents
        .iter()
        .map(|item| item.id.as_str())
        .collect::<HashSet<_>>();
    let current_projects = snapshot
        .projects
        .iter()
        .map(|item| item.id.as_str())
        .collect::<HashSet<_>>();
    let current_tasks = snapshot
        .tasks
        .iter()
        .map(|item| item.id.as_str())
        .collect::<HashSet<_>>();
    let current_channels = snapshot
        .channels
        .iter()
        .map(|item| item.id.as_str())
        .collect::<HashSet<_>>();
    let current_collaborations = snapshot
        .collaborations
        .iter()
        .map(|item| item.id.as_str())
        .collect::<HashSet<_>>();

    for agent in &input.agents {
        if !imported_hosts.contains(agent.host_id.as_str())
            && !current_hosts.contains(agent.host_id.as_str())
        {
            return Err("History import agent refers to an unknown host.".into());
        }
    }
    for project in &input.projects {
        for workspace in &project.workspaces {
            if !imported_hosts.contains(workspace.host_id.as_str())
                && !current_hosts.contains(workspace.host_id.as_str())
            {
                return Err("History import project refers to an unknown host.".into());
            }
        }
    }
    for task in &input.tasks {
        let agent = input
            .agents
            .iter()
            .find(|agent| agent.id == task.agent_id)
            .or_else(|| {
                snapshot
                    .agents
                    .iter()
                    .find(|agent| agent.id == task.agent_id)
            });
        if agent.is_none() && !(task.archived && task.archived_agent_name.is_some()) {
            return Err("History import task refers to an unknown agent.".into());
        }
        if agent.is_some_and(|agent| agent.internal) {
            return Err(
                "History import task cannot target the internal Monitter Admin agent.".into(),
            );
        }
        if !imported_hosts.contains(task.host_id.as_str())
            && !current_hosts.contains(task.host_id.as_str())
        {
            return Err("History import task refers to an unknown host.".into());
        }
        let saved_host = input
            .task_hosts
            .get(&task.id)
            .expect("task host completeness was validated");
        if !imported_hosts.contains(saved_host.id.as_str())
            && !current_hosts.contains(saved_host.id.as_str())
        {
            return Err("History import saved task host refers to an unknown host.".into());
        }
        if let Some(project_id) = task.project_id.as_deref() {
            if !imported_projects.contains(project_id) && !current_projects.contains(project_id) {
                return Err("History import task refers to an unknown project.".into());
            }
        }
        if let Some(parent) = task.parent_task_id.as_deref() {
            if !imported_tasks.contains(parent) && !current_tasks.contains(parent) {
                return Err("History import task refers to an unknown parent task.".into());
            }
        }
        if let Some(channel) = task.channel_id.as_deref() {
            if !current_channels.contains(channel) {
                return Err(
                    "History import task refers to a channel that is not already present.".into(),
                );
            }
        }
        if task.codex_home.is_some()
            && input
                .task_hosts
                .get(&task.id)
                .is_some_and(|host| host.kind != "local")
        {
            return Err("History import Codex account home is only valid for a local host.".into());
        }
        let imported_host = saved_host;
        if let Some(existing_task) = snapshot
            .tasks
            .iter()
            .find(|existing| existing.id == task.id)
        {
            let existing_host = data
                .task_hosts
                .get(&existing_task.id)
                .ok_or("Existing task host mapping is missing.")?;
            if existing_host != imported_host {
                return Err("History import task ID has a different saved host mapping.".into());
            }
        } else if data.task_hosts.contains_key(&task.id) {
            return Err(
                "History import task host mapping collides with an unrelated record.".into(),
            );
        }
    }
    for message in &input.messages {
        if let Some(agent_id) = message.sender_agent_id.as_deref() {
            if !imported_agents.contains(agent_id) && !current_agents.contains(agent_id) {
                return Err("History import message refers to an unknown sender agent.".into());
            }
        }
        if let Some(collaboration_id) = message.collaboration_id.as_deref() {
            if !current_collaborations.contains(collaboration_id) {
                return Err(
                    "History import message refers to a collaboration that is not already present."
                        .into(),
                );
            }
        }
        let task = input
            .tasks
            .iter()
            .find(|task| task.id == message.task_id)
            .or_else(|| {
                snapshot
                    .tasks
                    .iter()
                    .find(|task| task.id == message.task_id)
            })
            .ok_or("History import message refers to an unknown task.")?;
        let task_host = input
            .task_hosts
            .get(&task.id)
            .or_else(|| data.task_hosts.get(&task.id))
            .ok_or("History import message task has no saved host mapping.")?;
        if task_host.id != task.host_id {
            return Err("History import message task host mapping does not match its task.".into());
        }
        for attachment in &message.attachments {
            if is_generated_image(attachment) {
                continue;
            }
            let stored = input
                .attachments
                .iter()
                .find(|item| item.attachment.id == attachment.id)
                .or_else(|| data.attachments.get(&attachment.id))
                .ok_or("History import message attachment has no saved registry record.")?;
            if stored.attachment != *attachment {
                return Err(
                    "History import message attachment differs from its registry record.".into(),
                );
            }
            if stored.host_id != task_host.id || stored.cwd != task.cwd {
                return Err(
                    "History import attachment is not owned by this task host and folder.".into(),
                );
            }
        }
    }
    for stored in &input.attachments {
        let referenced = input.messages.iter().any(|message| {
            message
                .attachments
                .iter()
                .any(|item| item.id == stored.attachment.id)
        });
        if !referenced {
            return Err(
                "History import contains an unreferenced attachment registry record.".into(),
            );
        }
    }
    for event in &input.events {
        if !imported_tasks.contains(event.task_id.as_str())
            && !current_tasks.contains(event.task_id.as_str())
        {
            return Err("History import event refers to an unknown task.".into());
        }
    }
    Ok(())
}

fn is_generated_image(attachment: &attachments::Attachment) -> bool {
    matches!(
        attachment.path.as_str(),
        "Generated by Codex" | "Generated by ACP agent"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::id;

    fn bundle(service: &Service, new_host: bool) -> GpuiHistoryImport {
        let snapshot = service.snapshot().unwrap();
        let agent = snapshot
            .agents
            .iter()
            .find(|agent| !agent.internal)
            .unwrap()
            .clone();
        let mut host = snapshot.hosts[0].clone();
        if new_host {
            host.id = id();
        }
        let task_id = id();
        let task = Task {
            id: task_id.clone(),
            agent_id: agent.id.clone(),
            title: "Imported chat".into(),
            native_session_id: Some("saved-native-session".into()),
            status: "completed".into(),
            archived: false,
            created_at: 1,
            updated_at: 2,
            parent_task_id: None,
            channel_id: None,
            host_id: host.id.clone(),
            cwd: host.default_cwd.clone(),
            provider: agent.provider.clone(),
            codex_home: None,
            model: agent.model.clone(),
            model_settings: None,
            sandbox: agent.sandbox.clone(),
            project_id: None,
            acp: agent.acp.clone(),
            archived_agent_name: None,
        };
        let mut imported_agent = agent;
        if new_host {
            imported_agent.id = id();
            imported_agent.host_id = host.id.clone();
        }
        let message = Message {
            id: id(),
            task_id: task_id.clone(),
            role: "assistant".into(),
            text: "Saved answer".into(),
            created_at: 2,
            sender_agent_id: Some(imported_agent.id.clone()),
            collaboration_id: None,
            attachments: vec![],
            stream_status: Some("streaming".into()),
            phase: Some("final_answer".into()),
            response_metadata: None,
        };
        GpuiHistoryImport {
            hosts: vec![host.clone()],
            agents: vec![imported_agent],
            projects: vec![],
            tasks: vec![task.clone()],
            messages: vec![message],
            events: vec![RunEvent {
                id: id(),
                task_id: task_id.clone(),
                kind: "status".into(),
                title: "turn.completed".into(),
                detail: Arc::from("{}"),
                created_at: 3,
            }],
            attachments: vec![],
            task_hosts: HashMap::from([(task_id, host)]),
        }
    }

    fn test_service() -> (Arc<Service>, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("gpui-history-import-{}", id()));
        let service = Service::open(None, dir.clone()).unwrap();
        (service, dir)
    }

    #[test]
    fn import_is_dormant_idempotent_and_normalizes_streaming_messages() {
        let (service, dir) = test_service();
        let input = bundle(&service, false);
        let task_id = input.tasks[0].id.clone();
        let native_id = input.tasks[0].native_session_id.clone();
        let first = service.import_gpui_history(input.clone()).unwrap();
        assert_eq!(first.tasks_added, 1);
        assert_eq!(first.messages_added, 1);
        let second = service.import_gpui_history(input).unwrap();
        assert_eq!(second.tasks_added, 0);
        assert_eq!(second.messages_added, 0);
        let snapshot = service.snapshot().unwrap();
        let task = snapshot
            .tasks
            .iter()
            .find(|task| task.id == task_id)
            .unwrap();
        assert_eq!(task.status, "completed");
        assert_eq!(task.native_session_id, native_id);
        assert_eq!(
            snapshot
                .messages
                .iter()
                .find(|message| message.task_id == task_id)
                .unwrap()
                .stream_status
                .as_deref(),
            Some("interrupted")
        );
        assert!(!service.runs.lock().unwrap().tasks.contains_key(&task_id));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn cold_history_preflight_and_repeat_import_compare_durable_bodies() {
        let (service, dir) = test_service();
        let mut input = bundle(&service, false);
        let message = input.messages[0].clone();
        input.messages = (0..70).map(|index| {
            let mut row = message.clone(); row.id = id(); row.created_at += index;
            row.text = format!("Durable answer {index}"); row
        }).collect();
        let event = input.events[0].clone();
        input.events = (0..140).map(|index| {
            let mut row = event.clone(); row.id = id(); row.created_at += index;
            row.detail = Arc::from("Durable diagnostic body ".repeat(100)); row
        }).collect();
        service.import_gpui_history(input.clone()).unwrap();
        assert!(service.snapshot().unwrap().messages.payload_is_cold(&input.messages[0].id));
        service.validate_gpui_history(input.clone()).unwrap();
        let report = service.import_gpui_history(input.clone()).unwrap();
        assert_eq!(report.messages_added, 0);
        assert_eq!(report.events_added, 0);
        input.messages[0].text.push_str(" conflicting");
        assert!(service.validate_gpui_history(input.clone()).is_err());
        assert!(service.import_gpui_history(input).is_err());
        drop(service); let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn conflicting_record_rejects_all_new_records_before_persisting() {
        let (service, dir) = test_service();
        let mut input = bundle(&service, false);
        input.agents[0].name.push_str(" divergent");
        let mut extra_host = input.hosts[0].clone();
        extra_host.id = id();
        input.hosts.push(extra_host);
        let before = service.snapshot().unwrap();
        assert!(service.import_gpui_history(input).is_err());
        let after = service.snapshot().unwrap();
        assert_eq!(before.hosts, after.hosts);
        assert_eq!(before.agents, after.agents);
        assert_eq!(before.tasks.len(), after.tasks.len());
        assert!(service.runs.lock().unwrap().tasks.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn invalid_reference_rejects_import_without_partial_changes() {
        let (service, dir) = test_service();
        let mut input = bundle(&service, true);
        input.tasks[0].channel_id = Some(id());
        let before = service.snapshot().unwrap();
        assert!(service
            .import_gpui_history(input)
            .unwrap_err()
            .contains("channel"));
        let after = service.snapshot().unwrap();
        assert_eq!(before.hosts, after.hosts);
        assert_eq!(before.tasks.len(), after.tasks.len());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn metadata_only_chunk_preflight_is_read_only_and_valid() {
        let (service, dir) = test_service();
        let input = GpuiHistoryImport {
            hosts: vec![],
            agents: vec![],
            projects: vec![],
            tasks: vec![],
            messages: vec![],
            events: vec![],
            attachments: vec![],
            task_hosts: HashMap::new(),
        };
        let before = service.snapshot().unwrap();
        service.validate_gpui_history(input).unwrap();
        let after = service.snapshot().unwrap();
        assert_eq!(before.hosts, after.hosts);
        assert_eq!(before.agents, after.agents);
        assert_eq!(before.tasks, after.tasks);
        assert_eq!(before.messages.len(), after.messages.len());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn generated_inline_images_validate_but_unmanaged_file_paths_do_not() {
        let mut generated = attachments::generated_image("/9j/2Q==").unwrap();
        generated.path = "Generated by Codex".into();
        validate_message_attachment(&generated).unwrap();

        let stored = attachments::StoredAttachment {
            attachment: attachments::Attachment {
                id: id(),
                name: "notes.txt".into(),
                mime_type: "text/plain".into(),
                size: 1,
                path: "/tmp/outside.txt".into(),
                preview_data_url: None,
                source_id: None,
            },
            host_id: id(),
            cwd: "/tmp/work".into(),
        };
        assert!(validate_stored_attachment(&stored).is_err());
    }

    /// Explicit local migration rehearsal. It never opens the source or live
    /// destination profile; only prepared record files and a fresh scratch DB.
    #[test]
    #[ignore = "requires an explicitly prepared private import manifest"]
    fn prepared_history_rehearsal_is_dormant_and_idempotent() {
        let path = std::env::var("GPUI_HISTORY_IMPORT_MANIFEST").expect("prepared manifest path");
        let manifest: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let (service, dir) = test_service();
        let files = manifest["files"].as_array().unwrap();
        let read = |file: &serde_json::Value| -> GpuiHistoryImport {
            let bytes = std::fs::read(file["path"].as_str().unwrap()).unwrap();
            assert_eq!(bytes.len() as u64, file["bytes"].as_u64().unwrap());
            use sha2::Digest;
            assert_eq!(format!("{:x}", sha2::Sha256::digest(&bytes)), file["sha256"].as_str().unwrap());
            serde_json::from_slice(&bytes).unwrap()
        };
        for (i, file) in files.iter().enumerate() {
            service.validate_gpui_history(read(file)).unwrap_or_else(|error| panic!("preflight batch {i}: {error}"));
        }
        let mut totals = [0usize; 5];
        for (i, file) in files.iter().enumerate() {
            let report = service.import_gpui_history(read(file)).unwrap_or_else(|error| panic!("import batch {i}: {error}"));
            totals[0] += report.projects_added;
            totals[1] += report.agents_added;
            totals[2] += report.tasks_added;
            totals[3] += report.messages_added;
            totals[4] += report.events_added;
        }
        for (i, key) in ["projects", "agents", "tasks", "messages", "events"].iter().enumerate() {
            assert_eq!(totals[i] as u64, manifest["counts"][key].as_u64().unwrap());
        }
        for file in files {
            let report = service.import_gpui_history(read(file)).unwrap();
            assert_eq!(report.tasks_added + report.messages_added + report.events_added + report.agents_added + report.projects_added + report.attachments_added, 0);
        }
        assert!(service.runs.lock().unwrap().tasks.is_empty());
        assert!(service.snapshot().unwrap().queued_messages.is_empty());
        eprintln!("Prepared history rehearsal passed: projects={}, agents={}, tasks={}, messages={}, events={}.", totals[0], totals[1], totals[2], totals[3], totals[4]);
        drop(service);
        let reopened = Service::open(None, dir.clone()).unwrap();
        let saved = reopened.snapshot().unwrap();
        assert_eq!(saved.messages.len() as u64, manifest["counts"]["messages"].as_u64().unwrap());
        assert_eq!(saved.events.len() as u64, manifest["counts"]["events"].as_u64().unwrap());
        assert!(reopened.runs.lock().unwrap().tasks.is_empty());
        drop(reopened);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
