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
    #[serde(default)]
    pub model_settings: Option<ModelSettings>,
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
    #[serde(default)]
    pub attachments: Vec<crate::attachments::Attachment>,
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
    #[serde(default)]
    pub agent_conversation_enabled: bool,
    #[serde(default = "default_agent_conversation_turn_limit")]
    pub agent_conversation_turn_limit: u32,
    #[serde(default)]
    pub agent_conversation_turns_used: u32,
    #[serde(default)]
    pub agent_conversation_paused: bool,
}

pub fn default_agent_conversation_turn_limit() -> u32 {
    6
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct QueuedMessage {
    pub id: String,
    pub task_id: String,
    pub channel_id: Option<String>,
    pub text: String,
    pub attachment_ids: Vec<String>,
    pub created_at: i64,
    pub status: String,
    #[serde(default)]
    pub error: Option<String>,
    /// A channel peer delivery is never represented as user-authored input.
    #[serde(default)]
    pub sender_agent_id: Option<String>,
    #[serde(default)]
    pub origin: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(default = "default_terminal_font_size")]
    pub terminal_font_size: u8,
    #[serde(default = "default_chat_font_size")]
    pub chat_font_size: u8,
    #[serde(default = "default_interface_font_size")]
    pub interface_font_size: u8,
    #[serde(default = "default_chat_line_height")]
    pub chat_line_height: f64,
    #[serde(default = "default_terminal_line_height")]
    pub terminal_line_height: f64,
    #[serde(default)]
    pub terminal_font: String,
    #[serde(default)]
    pub chat_font: String,
    #[serde(default)]
    pub interface_font: String,
    pub accent: String,
    pub theme: String,
    #[serde(default = "default_interface_scale")]
    pub interface_scale: u8,
    #[serde(default = "default_show_tool_activity")]
    pub show_tool_activity: bool,
    #[serde(default = "default_show_reasoning_summaries")]
    pub show_reasoning_summaries: bool,
    #[serde(default)]
    pub tint_user_messages: bool,
    #[serde(default)]
    pub compress_tool_calls: bool,
    #[serde(default)]
    pub send_with_enter: bool,
    #[serde(default = "default_sidebar_view")]
    pub sidebar_view: String,
    #[serde(default = "default_dim_inactive_panes")]
    pub dim_inactive_panes: bool,
    #[serde(default = "default_inactive_pane_opacity")]
    pub inactive_pane_opacity: f64,
    #[serde(default)]
    pub focus_follows_mouse: bool,
    #[serde(default = "default_busy_message_mode")]
    pub busy_message_mode: String,
    #[serde(default = "default_shortcut_mode")]
    pub shortcut_mode: String,
    #[serde(default = "default_show_tab_close_buttons")]
    pub show_tab_close_buttons: bool,
}
impl Eq for Settings {}
fn default_terminal_font_size() -> u8 {
    14
}
pub fn default_shortcut_mode() -> String {
    "standard".into()
}
fn default_show_tab_close_buttons() -> bool {
    true
}
fn default_chat_font_size() -> u8 {
    13
}
fn default_interface_font_size() -> u8 {
    14
}
fn default_chat_line_height() -> f64 {
    1.65
}
fn default_terminal_line_height() -> f64 {
    1.0
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
    #[serde(default = "default_project_icon")]
    pub icon: String,
    #[serde(default = "default_project_color")]
    pub color: String,
    #[serde(default)]
    pub workspaces: Vec<ProjectWorkspace>,
}

fn default_project_icon() -> String {
    "folder".into()
}
fn default_project_color() -> String {
    "#3f9d6a".into()
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
fn default_dim_inactive_panes() -> bool {
    true
}
fn default_inactive_pane_opacity() -> f64 {
    0.6
}
fn default_busy_message_mode() -> String {
    "queue".into()
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
    #[serde(default)]
    pub queued_messages: Vec<QueuedMessage>,
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
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub model_settings: Option<ModelSettings>,
    #[serde(default)]
    pub sandbox: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelSettings {
    pub model: String,
    pub reasoning_effort: Option<String>,
    pub fast_mode: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCatalogTarget {
    #[serde(default)]
    pub task_id: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReasoningEffortOption {
    pub id: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogModel {
    pub id: String,
    pub name: String,
    pub description: String,
    pub reasoning_efforts: Vec<ReasoningEffortOption>,
    pub default_effort: Option<String>,
    pub supports_fast: bool,
    pub fast_description: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCatalogCurrent {
    pub model: String,
    pub reasoning_effort: Option<String>,
    pub fast_mode: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCatalog {
    pub models: Vec<CatalogModel>,
    pub current: ModelCatalogCurrent,
    pub source: String,
    pub warning: Option<String>,
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
        "codex" => valid_sandbox(sandbox) || sandbox == "yolo",
        "claude" => sandbox == "harness-configured" || sandbox == "yolo",
        "opencode" | "hermes" => sandbox == "harness-configured",
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
        queued_messages: vec![],
        settings: Settings {
            terminal_font_size: default_terminal_font_size(),
            chat_font_size: default_chat_font_size(),
            interface_font_size: default_interface_font_size(),
            chat_line_height: default_chat_line_height(),
            terminal_line_height: default_terminal_line_height(),
            terminal_font: String::new(),
            chat_font: String::new(),
            interface_font: String::new(),
            accent: "#3f9d6a".into(),
            theme: "system".into(),
            interface_scale: default_interface_scale(),
            show_tool_activity: default_show_tool_activity(),
            show_reasoning_summaries: default_show_reasoning_summaries(),
            tint_user_messages: false,
            compress_tool_calls: false,
            send_with_enter: false,
            sidebar_view: default_sidebar_view(),
            dim_inactive_panes: default_dim_inactive_panes(),
            inactive_pane_opacity: default_inactive_pane_opacity(),
            focus_follows_mouse: false,
            busy_message_mode: default_busy_message_mode(),
            shortcut_mode: default_shortcut_mode(),
            show_tab_close_buttons: default_show_tab_close_buttons(),
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
        assert_eq!(settings.chat_line_height, 1.65);
        assert_eq!(settings.terminal_line_height, 1.0);
        assert!(settings.show_tool_activity);
        assert!(settings.show_reasoning_summaries);
        assert!(!settings.send_with_enter);
        assert!(!settings.tint_user_messages);
        assert!(!settings.compress_tool_calls);
        assert_eq!(settings.sidebar_view, "standard");
        assert_eq!(settings.shortcut_mode, "standard");
        assert!(settings.show_tab_close_buttons);
    }

    #[test]
    fn settings_serialize_new_appearance_fields_in_camel_case() {
        let value = serde_json::to_value(default_snapshot().settings).unwrap();

        assert_eq!(value["interfaceScale"], 125);
        assert_eq!(value["chatLineHeight"], 1.65);
        assert_eq!(value["terminalLineHeight"], 1.0);
        assert_eq!(value["showToolActivity"], true);
        assert_eq!(value["showReasoningSummaries"], true);
        assert_eq!(value["sendWithEnter"], false);
        assert_eq!(value["tintUserMessages"], false);
        assert_eq!(value["compressToolCalls"], false);
        assert_eq!(value["sidebarView"], "standard");
        assert_eq!(value["shortcutMode"], "standard");
        assert_eq!(value["showTabCloseButtons"], true);
    }

    #[test]
    fn settings_round_trip_preserves_disabled_display_toggles() {
        let settings: Settings = serde_json::from_str(
            r##"{"accent":"#3f9d6a","theme":"dark","interfaceScale":125,"showToolActivity":false,"showReasoningSummaries":false,"sendWithEnter":true,"tintUserMessages":true,"compressToolCalls":true}"##,
        )
        .unwrap();

        let restored: Settings =
            serde_json::from_value(serde_json::to_value(&settings).unwrap()).unwrap();
        assert_eq!(restored, settings);
        assert!(!restored.show_tool_activity);
        assert!(!restored.show_reasoning_summaries);
        assert!(restored.send_with_enter);
        assert!(restored.tint_user_messages);
        assert!(restored.compress_tool_calls);
    }

    #[test]
    fn yolo_is_limited_to_providers_with_a_documented_bypass() {
        assert!(valid_sandbox_for_provider("codex", "yolo"));
        assert!(valid_sandbox_for_provider("claude", "yolo"));
        assert!(!valid_sandbox_for_provider("opencode", "yolo"));
        assert!(!valid_sandbox_for_provider("hermes", "yolo"));
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
        assert_eq!(task.model_settings, None);
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
        assert!(snapshot.settings.dim_inactive_panes);
        assert_eq!(snapshot.settings.inactive_pane_opacity, 0.6);
        assert!(!snapshot.settings.focus_follows_mouse);
        assert!(snapshot.settings.terminal_font.is_empty());
        assert!(snapshot.settings.chat_font.is_empty());
        assert!(snapshot.settings.interface_font.is_empty());
        assert_eq!(snapshot.settings.terminal_font_size, 14);
        assert_eq!(snapshot.settings.chat_font_size, 13);
        assert_eq!(snapshot.settings.interface_font_size, 14);
    }

    #[test]
    fn old_messages_default_to_no_attachments() {
        let message: Message = serde_json::from_value(serde_json::json!({
            "id":"m", "taskId":"t", "role":"user", "text":"hello", "createdAt":1
        }))
        .unwrap();
        assert!(message.attachments.is_empty());
    }

    #[test]
    fn task_from_agent_uses_explicit_working_folder() {
        let snapshot = default_snapshot();
        let task = task_from_agent(
            &snapshot.agents[0],
            &CreateTaskInput {
                agent_id: snapshot.agents[0].id.clone(),
                title: "Task".into(),
                native_session_id: None,
                parent_task_id: None,
                channel_id: None,
                project_id: None,
                cwd: Some("/chosen/folder".into()),
                model_settings: None,
                sandbox: None,
            },
        );
        assert_eq!(task.cwd, "/chosen/folder");
    }

    #[test]
    fn old_projects_default_to_folder_icon_and_accent_colour() {
        let project: Project =
            serde_json::from_str(r##"{"id":"p","name":"Project","description":""}"##).unwrap();
        assert_eq!(project.icon, "folder");
        assert_eq!(project.color, "#3f9d6a");
    }

    #[test]
    fn projects_and_task_links_round_trip_in_camel_case() {
        let mut snapshot = default_snapshot();
        let project_id = "project-1".to_string();
        snapshot.projects.push(Project {
            id: project_id.clone(),
            name: "Project".into(),
            description: "Description".into(),
            icon: "folder".into(),
            color: "#3f9d6a".into(),
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
                cwd: None,
                model_settings: None,
                sandbox: None,
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
        cwd: input.cwd.clone().unwrap_or_else(|| agent.cwd.clone()),
        provider: agent.provider.clone(),
        model: input
            .model_settings
            .as_ref()
            .map(|settings| settings.model.clone())
            .unwrap_or_else(|| agent.model.clone()),
        model_settings: input.model_settings.clone(),
        sandbox: input
            .sandbox
            .clone()
            .unwrap_or_else(|| agent.sandbox.clone()),
        project_id: input.project_id.clone(),
    }
}
