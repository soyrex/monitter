use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

pub fn id() -> String {
    Uuid::new_v4().to_string()
}
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Host {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub address: String,
    pub user: String,
    pub port: u16,
    pub identity_file: String,
    pub default_cwd: String,
    pub codex_path: String,
    #[serde(default)]
    pub claude_path: String,
    pub opencode_path: String,
    pub hermes_path: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Agent {
    pub id: String,
    pub name: String,
    pub description: String,
    pub instructions: String,
    pub provider: String,
    pub model: String,
    pub host_id: String,
    pub cwd: String,
    pub color: String,
    pub sandbox: String,
    #[serde(default)]
    pub avatar: Option<String>,
    #[serde(default)]
    pub expertise: Vec<String>,
    #[serde(default)]
    pub responsibilities: Vec<String>,
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default = "default_collaboration_enabled")]
    pub collaboration_enabled: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub agent_id: String,
    pub title: String,
    pub native_session_id: Option<String>,
    pub status: String,
    #[serde(default)]
    pub archived: bool,
    pub created_at: i64,
    pub updated_at: i64,
    pub parent_task_id: Option<String>,
    pub channel_id: Option<String>,
    pub host_id: String,
    pub cwd: String,
    pub provider: String,
    pub model: String,
    pub sandbox: String,
    #[serde(default)]
    pub project_id: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: String,
    pub task_id: String,
    pub role: String,
    pub text: String,
    pub created_at: i64,
    #[serde(default)]
    pub sender_agent_id: Option<String>,
    #[serde(default)]
    pub collaboration_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Collaboration {
    pub id: String,
    pub kind: String,
    pub from_agent_id: String,
    pub from_task_id: String,
    pub to_agent_id: String,
    pub to_task_id: String,
    pub text: String,
    pub request_id: String,
    pub status: String,
    pub result: Option<String>,
    pub error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

pub fn default_collaboration_enabled() -> bool {
    true
}

pub fn agent_instructions(agent: &Agent) -> String {
    let mut parts = vec![format!("Monitter agent: {}", agent.name)];
    if !agent.description.trim().is_empty() {
        parts.push(format!("Purpose: {}", agent.description.trim()));
    }
    for (name, entries) in [
        ("Expertise", &agent.expertise),
        ("Responsibilities", &agent.responsibilities),
        ("Skills", &agent.skills),
    ] {
        if !entries.is_empty() {
            parts.push(format!("{name}:\n- {}", entries.join("\n- ")));
        }
    }
    if !agent.instructions.trim().is_empty() {
        parts.push(agent.instructions.trim().into());
    }
    parts.join("\n\n")
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RunEvent {
    pub id: String,
    pub task_id: String,
    pub kind: String,
    pub title: String,
    pub detail: String,
    pub created_at: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChannelMessage {
    pub id: String,
    pub role: String,
    pub agent_id: Option<String>,
    pub text: String,
    pub created_at: i64,
    pub task_id: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Channel {
    pub id: String,
    pub name: String,
    pub description: String,
    pub agent_ids: Vec<String>,
    pub messages: Vec<ChannelMessage>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub accent: String,
    pub theme: String,
    #[serde(default = "default_interface_scale")]
    pub interface_scale: u8,
    #[serde(default = "default_show_tool_activity")]
    pub show_tool_activity: bool,
    #[serde(default = "default_show_reasoning_summaries")]
    pub show_reasoning_summaries: bool,
    #[serde(default)]
    pub send_with_enter: bool,
    #[serde(default = "default_sidebar_view")]
    pub sidebar_view: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectWorkspace {
    pub host_id: String,
    pub cwd: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub workspaces: Vec<ProjectWorkspace>,
}

fn default_interface_scale() -> u8 {
    125
}

fn default_show_tool_activity() -> bool {
    true
}

fn default_show_reasoning_summaries() -> bool {
    true
}

fn default_sidebar_view() -> String {
    "standard".into()
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub hosts: Vec<Host>,
    pub agents: Vec<Agent>,
    pub tasks: Vec<Task>,
    pub messages: Vec<Message>,
    pub events: Vec<RunEvent>,
    pub channels: Vec<Channel>,
    #[serde(default)]
    pub projects: Vec<Project>,
    pub settings: Settings,
    #[serde(default)]
    pub collaborations: Vec<Collaboration>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTaskInput {
    pub agent_id: String,
    pub title: String,
    pub native_session_id: Option<String>,
    pub parent_task_id: Option<String>,
    pub channel_id: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
}
#[derive(Debug, Clone, Serialize)]
pub struct ProbeResult {
    pub ok: bool,
    pub versions: HashMap<String, String>,
    pub message: String,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SmokeResult {
    pub ok: bool,
    pub task_id: String,
    pub native_session_id: Option<String>,
    pub final_status: String,
    pub output_count: usize,
    pub persisted: bool,
    pub last_assistant_text: Option<String>,
    pub message: String,
}

pub fn home() -> String {
    std::env::var("HOME").unwrap_or_else(|_| "/".into())
}
pub fn valid_sandbox(v: &str) -> bool {
    matches!(v, "read-only" | "workspace-write")
}

pub fn valid_sandbox_for_provider(provider: &str, sandbox: &str) -> bool {
    match provider {
        "codex" => valid_sandbox(sandbox),
        "claude" | "opencode" | "hermes" => sandbox == "harness-configured",
        _ => false,
    }
}

pub fn known_provider(provider: &str) -> bool {
    matches!(provider, "codex" | "claude" | "opencode" | "hermes")
}
pub fn default_snapshot() -> Snapshot {
    let host_id = id();
    let agent_id = id();
    let cwd = home();
    Snapshot {
        hosts: vec![Host {
            id: host_id.clone(),
            name: "This Mac".into(),
            kind: "local".into(),
            address: "localhost".into(),
            user: String::new(),
            port: 0,
            identity_file: String::new(),
            default_cwd: cwd.clone(),
            codex_path: String::new(),
            claude_path: String::new(),
            opencode_path: String::new(),
            hermes_path: String::new(),
        }],
        agents: vec![Agent {
            id: agent_id,
            name: "Codex".into(),
            description: "Local Codex CLI".into(),
            instructions: String::new(),
            provider: "codex".into(),
            model: String::new(),
            host_id,
            cwd,
            color: "#3f9d6a".into(),
            sandbox: "read-only".into(),
            avatar: None,
            expertise: vec![],
            responsibilities: vec![],
            skills: vec![],
            collaboration_enabled: true,
        }],
        tasks: vec![],
        messages: vec![],
        events: vec![],
        channels: vec![],
        projects: vec![],
        collaborations: vec![],
        settings: Settings {
            accent: "#3f9d6a".into(),
            theme: "system".into(),
            interface_scale: default_interface_scale(),
            show_tool_activity: default_show_tool_activity(),
            show_reasoning_summaries: default_show_reasoning_summaries(),
            send_with_enter: false,
            sidebar_view: default_sidebar_view(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_settings_deserialize_with_appearance_defaults() {
        let settings: Settings =
            serde_json::from_str(r##"{"accent":"#3f9d6a","theme":"system"}"##).unwrap();

        assert_eq!(settings.interface_scale, 125);
        assert!(settings.show_tool_activity);
        assert!(settings.show_reasoning_summaries);
        assert!(!settings.send_with_enter);
        assert_eq!(settings.sidebar_view, "standard");
    }

    #[test]
    fn settings_serialize_new_appearance_fields_in_camel_case() {
        let value = serde_json::to_value(default_snapshot().settings).unwrap();

        assert_eq!(value["interfaceScale"], 125);
        assert_eq!(value["showToolActivity"], true);
        assert_eq!(value["showReasoningSummaries"], true);
        assert_eq!(value["sendWithEnter"], false);
        assert_eq!(value["sidebarView"], "standard");
    }

    #[test]
    fn settings_round_trip_preserves_disabled_display_toggles() {
        let settings: Settings = serde_json::from_str(
            r##"{"accent":"#3f9d6a","theme":"dark","interfaceScale":125,"showToolActivity":false,"showReasoningSummaries":false,"sendWithEnter":true}"##,
        )
        .unwrap();

        let restored: Settings =
            serde_json::from_value(serde_json::to_value(&settings).unwrap()).unwrap();
        assert_eq!(restored, settings);
        assert!(!restored.show_tool_activity);
        assert!(!restored.show_reasoning_summaries);
        assert!(restored.send_with_enter);
    }

    #[test]
    fn old_agent_defaults_to_no_avatar_and_round_trips_data_url() {
        let mut agent: Agent = serde_json::from_value(serde_json::json!({
            "id":"a", "name":"Agent", "description":"", "instructions":"", "provider":"codex",
            "model":"", "hostId":"h", "cwd":"/tmp", "color":"#000", "sandbox":"read-only"
        }))
        .unwrap();
        assert_eq!(agent.avatar, None);
        agent.avatar = Some("data:image/png;base64,iVBORw0KGgo=".into());
        let value = serde_json::to_value(&agent).unwrap();
        assert_eq!(value["avatar"], "data:image/png;base64,iVBORw0KGgo=");
        assert_eq!(serde_json::from_value::<Agent>(value).unwrap(), agent);
    }
}
#[cfg(test)]
mod task_migration_tests {
    use super::*;
    #[test]
    fn old_task_defaults_to_unarchived() {
        let task: Task = serde_json::from_value(serde_json::json!({
            "id":"t", "agentId":"a", "title":"x", "nativeSessionId":null, "status":"idle", "createdAt":1, "updatedAt":1, "parentTaskId":null, "channelId":null, "hostId":"h", "cwd":"/tmp", "provider":"codex", "model":"", "sandbox":"read-only"
        })).unwrap();
        assert!(!task.archived);
        assert_eq!(task.project_id, None);
    }

    #[test]
    fn old_snapshot_defaults_projects_and_sidebar_view() {
        let snapshot: Snapshot = serde_json::from_value(serde_json::json!({
            "hosts": [], "agents": [], "tasks": [], "messages": [], "events": [], "channels": [],
            "settings": {"accent":"#3f9d6a", "theme":"system"}
        }))
        .unwrap();
        assert!(snapshot.projects.is_empty());
        assert_eq!(snapshot.settings.sidebar_view, "standard");
    }

    #[test]
    fn projects_and_task_links_round_trip_in_camel_case() {
        let mut snapshot = default_snapshot();
        let project_id = "project-1".to_string();
        snapshot.projects.push(Project {
            id: project_id.clone(),
            name: "Project".into(),
            description: "Description".into(),
            workspaces: vec![ProjectWorkspace {
                host_id: snapshot.hosts[0].id.clone(),
                cwd: "/workspace".into(),
            }],
        });
        snapshot.tasks.push(task_from_agent(
            &snapshot.agents[0],
            &CreateTaskInput {
                agent_id: snapshot.agents[0].id.clone(),
                title: "Task".into(),
                native_session_id: None,
                parent_task_id: None,
                channel_id: None,
                project_id: Some(project_id),
            },
        ));
        let value = serde_json::to_value(&snapshot).unwrap();
        assert_eq!(
            value["projects"][0]["workspaces"][0]["hostId"],
            snapshot.hosts[0].id
        );
        assert_eq!(value["tasks"][0]["projectId"], "project-1");
        let restored: Snapshot = serde_json::from_value(value).unwrap();
        assert_eq!(restored, snapshot);
    }
}

pub fn task_from_agent(agent: &Agent, input: &CreateTaskInput) -> Task {
    let time = now();
    Task {
        id: id(),
        agent_id: agent.id.clone(),
        title: input.title.trim().into(),
        native_session_id: input.native_session_id.clone(),
        status: "idle".into(),
        archived: false,
        created_at: time,
        updated_at: time,
        parent_task_id: input.parent_task_id.clone(),
        channel_id: input.channel_id.clone(),
        host_id: agent.host_id.clone(),
        cwd: agent.cwd.clone(),
        provider: agent.provider.clone(),
        model: agent.model.clone(),
        sandbox: agent.sandbox.clone(),
        project_id: input.project_id.clone(),
    }
}
