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

/// An explicit ACP stdio launcher. Arguments deliberately remain an argv
/// vector: ACP agents are never launched through a shell.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AcpLaunch {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

pub fn valid_acp_launch(launch: &AcpLaunch) -> bool {
    // Keep launch data bounded before it reaches state.json or a process API.
    // NUL is rejected because it cannot be represented in an argv element.
    !launch.command.trim().is_empty()
        && launch.command.len() <= 4096
        && !launch.command.contains('\0')
        && launch.args.len() <= 128
        && launch
            .args
            .iter()
            .all(|arg| arg.len() <= 16 * 1024 && !arg.contains('\0'))
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
    /// Optional local Codex account home. `None` retains the process default
    /// for future chats; task creation snapshots that default for local Codex.
    #[serde(default)]
    pub codex_home: Option<String>,
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
    /// ACP is intentionally an explicit launcher rather than a growing list
    /// of provider-specific executable fields. It is copied into new tasks.
    #[serde(default)]
    pub acp: Option<AcpLaunch>,
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
    /// Immutable local Codex account-home snapshot. Legacy tasks without this
    /// field retain the process default when resumed.
    #[serde(default)]
    pub codex_home: Option<String>,
    pub model: String,
    #[serde(default)]
    pub model_settings: Option<ModelSettings>,
    pub sandbox: String,
    #[serde(default)]
    pub project_id: Option<String>,
    /// Immutable ACP launch snapshot. Editing an agent must not alter an
    /// existing native ACP session or its resumed transport.
    #[serde(default)]
    pub acp: Option<AcpLaunch>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    /// Only live app-server replies use this field; older/provider messages remain unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream_status: Option<String>,
    /// Codex distinguishes interim commentary from the turn's final answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
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

pub fn agent_instructions(agent: &Agent, user_name: &str) -> String {
    let mut identity = format!(
        "You are acting as {}. You are an agent running inside the Monitter harness.",
        agent.name.trim()
    );
    if !user_name.trim().is_empty() {
        // Treat the display name as a quoted value, including embedded quotes.
        identity.push_str(&format!(
            " Your user is {}.",
            serde_json::json!(user_name.trim())
        ));
    }
    let mut parts = vec![];
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
    if parts.is_empty() {
        identity
    } else {
        format!(
            "{identity}\n\nAgent settings and instructions:\n\n{}",
            parts.join("\n\n")
        )
    }
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

/// Normalized, durable observation from one provider turn. `classification` is
/// either `delta` (add it once) or `cumulative` (newest snapshot wins).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RunUsageSample {
    pub sample_id: String,
    pub run_id: String,
    pub task_id: String,
    pub provider: String,
    pub configured_model: Option<String>,
    pub started_at: i64,
    pub observed_at: i64,
    pub final_sample: bool,
    pub classification: String,
    #[serde(default)]
    pub provider_turn_id: Option<String>,
    pub tokens: UsageTokens,
    pub cost_usd: Option<f64>,
    pub duration_ms: Option<i64>,
    pub api_duration_ms: Option<i64>,
    pub provider_turns: Option<i64>,
    pub context: Option<UsageContext>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct UsageTokens {
    pub input: Option<i64>, pub output: Option<i64>, pub cache_read: Option<i64>,
    pub cache_write: Option<i64>, pub reasoning: Option<i64>, pub total: Option<i64>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UsageContext { pub used: i64, pub size: i64 }
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RunUsageSummary {
    pub run_id: String, pub task_id: String, pub provider: String, pub configured_model: Option<String>,
    pub started_at: i64, pub finished_at: Option<i64>, #[serde(rename = "final")] pub final_: bool, pub tokens: UsageTokens,
    pub cost_usd: Option<f64>, pub duration_ms: Option<i64>, pub api_duration_ms: Option<i64>,
    pub provider_turns: Option<i64>, pub context: Option<UsageContext>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RunUsageAggregate {
    pub provider: String, pub runs: i64, pub final_runs: i64, pub tokens: RequiredUsageTokens,
    pub cost_usd: Option<f64>, pub duration_ms: Option<i64>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct RequiredUsageTokens { pub input: i64, pub output: i64, pub cache_read: i64, pub cache_write: i64, pub reasoning: i64, pub total: i64 }
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AllowanceWindow {
    pub key: String, pub label: String, pub metric: String, pub used_percent: Option<f64>, pub used: Option<f64>, pub limit: Option<f64>, pub unit: String, pub resets_at: Option<i64>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AllowanceBalance { pub key: String, pub label: String, pub unit: String, pub remaining: Option<f64>, pub limit: Option<f64>, pub resets_at: Option<i64> }
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionUsageSource {
    pub provider: String, pub host_id: String, pub source: String,
    #[serde(default)] pub codex_home: Option<String>,
    #[serde(default)] pub account_label: Option<String>,
    pub state: String, pub plan_type: Option<String>, pub fetched_at: Option<i64>, pub stale_after: Option<i64>, pub last_attempt_at: i64, pub windows: Vec<AllowanceWindow>, pub balances: Vec<AllowanceBalance>, pub error: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UsageOverview { pub generated_at: i64, pub captured_since: Option<i64>, pub subscriptions: Vec<SubscriptionUsageSource>, pub provider_totals: Vec<RunUsageAggregate>, pub recent_runs: Vec<RunUsageSummary> }
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

/// A durable user decision requested by a harness tool. Runtime waiters are
/// intentionally kept outside the snapshot: a restarted app has no process
/// left that can safely resume an in-flight tool invocation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRequest {
    pub id: String,
    pub task_id: String,
    pub provider: String,
    pub run_id: String,
    pub tool: String,
    pub summary: String,
    pub detail: String,
    pub risk: String,
    pub status: String,
    pub created_at: i64,
    #[serde(default)]
    pub resolved_at: Option<i64>,
    #[serde(default)]
    pub decision: Option<String>,
    /// Whether this concrete tool action has enough stable, non-interaction
    /// input to be safely remembered.  Missing fields from older records are
    /// intentionally treated as false.
    #[serde(default)]
    pub rememberable: bool,
    /// A provider-normalized class of actions that can be approved for the
    /// lifetime of this live harness session. Session grants are runtime-only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_scope: Option<String>,
    #[serde(default)]
    pub rule_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) approval_scope: Option<crate::ApprovalScope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<InteractionInput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response: Option<serde_json::Value>,
}

/// A user-owned, revocable exact-action approval.  The scope fingerprints are
/// persisted for matching, but the UI only needs the small descriptive fields.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRule {
    pub id: String,
    pub agent_id: String,
    pub host_id: String,
    pub provider: String,
    pub cwd: String,
    pub tool: String,
    pub summary: String,
    pub detail: String,
    pub created_at: i64,
    #[serde(default)]
    pub last_used_at: Option<i64>,
    #[serde(default)]
    pub use_count: u64,
    /// Hash-like canonical scope data. Kept private to the app state and never
    /// treated as a provider permission grant.
    #[serde(default)]
    pub host_fingerprint: String,
    #[serde(default)]
    pub launcher_fingerprint: String,
    #[serde(default)]
    pub action_fingerprint: String,
    #[serde(default)]
    pub sandbox: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InteractionInput {
    pub kind: String,
    #[serde(default)]
    pub questions: Vec<InputQuestion>,
    #[serde(default)]
    pub schema: Option<serde_json::Value>,
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InputQuestion {
    pub id: String,
    pub header: String,
    pub question: String,
    #[serde(default)]
    pub is_secret: bool,
    #[serde(default)]
    pub options: Vec<InputOption>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InputOption {
    pub label: String,
    pub description: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(default)]
    pub user_name: String,
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
    #[serde(default)]
    pub auto_hide_tabs: bool,
    #[serde(default = "default_tab_style")]
    pub tab_style: String,
    #[serde(default = "default_interface_density")]
    pub interface_density: String,
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
fn default_tab_style() -> String {
    "classic".into()
}
fn default_interface_density() -> String {
    "normal".into()
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
    #[serde(default)]
    pub approval_requests: Vec<ApprovalRequest>,
    #[serde(default)]
    pub approval_rules: Vec<ApprovalRule>,
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
    #[serde(default)]
    pub codex_home: Option<String>,
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
        "acp" => sandbox == "harness-configured" || sandbox == "yolo",
        _ => false,
    }
}

pub fn known_provider(provider: &str) -> bool {
    matches!(provider, "codex" | "claude" | "opencode" | "hermes" | "acp")
}

/// True when the provider carries per-turn model/sandbox overrides on the wire
/// so the next `turn/start` (or equivalent) reflects the latest task snapshot.
/// Currently only the Codex app-server does — other harnesses bake launch
/// flags into the resident process and need a restart to apply changes.
pub fn supports_live_model_change(provider: &str) -> bool {
    provider == "codex"
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
            codex_home: None,
            color: "#3f9d6a".into(),
            sandbox: "read-only".into(),
            avatar: None,
            expertise: vec![],
            responsibilities: vec![],
            skills: vec![],
            collaboration_enabled: true,
            acp: None,
        }],
        tasks: vec![],
        messages: vec![],
        events: vec![],
        channels: vec![],
        projects: vec![],
        collaborations: vec![],
        queued_messages: vec![],
        approval_requests: vec![],
        approval_rules: vec![],
        settings: Settings {
            user_name: String::new(),
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
            auto_hide_tabs: false,
            tab_style: default_tab_style(),
            interface_density: default_interface_density(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acp_launch_requires_bounded_shell_free_argv_data() {
        assert!(valid_acp_launch(&AcpLaunch {
            command: "agent-acp".into(),
            args: vec!["--stdio".into()],
        }));
        assert!(!valid_acp_launch(&AcpLaunch {
            command: " ".into(),
            args: vec![],
        }));
        assert!(!valid_acp_launch(&AcpLaunch {
            command: "agent\0acp".into(),
            args: vec![],
        }));
    }

    #[test]
    fn old_settings_deserialize_with_appearance_defaults() {
        let settings: Settings =
            serde_json::from_str(r##"{"accent":"#3f9d6a","theme":"system"}"##).unwrap();

        assert!(settings.user_name.is_empty());
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
        assert_eq!(settings.tab_style, "classic");
        assert_eq!(settings.interface_density, "normal");
    }

    #[test]
    fn settings_serialize_new_appearance_fields_in_camel_case() {
        let value = serde_json::to_value(default_snapshot().settings).unwrap();

        assert_eq!(value["userName"], "");
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
        assert_eq!(value["tabStyle"], "classic");
        assert_eq!(value["interfaceDensity"], "normal");
    }

    #[test]
    fn profile_name_round_trips_and_is_quoted_without_guessing_a_default() {
        let mut snapshot = default_snapshot();
        snapshot.settings.user_name = "Álex \"Al\"".into();
        let value = serde_json::to_value(&snapshot.settings).unwrap();
        assert_eq!(value["userName"], "Álex \"Al\"");
        assert_eq!(
            serde_json::from_value::<Settings>(value).unwrap(),
            snapshot.settings
        );

        let agent = &mut snapshot.agents[0];
        agent.name = "Claudine".into();
        agent.description.clear();
        agent.expertise.clear();
        agent.responsibilities.clear();
        agent.skills.clear();
        agent.instructions.clear();
        let identity =
            "You are acting as Claudine. You are an agent running inside the Monitter harness.";
        assert_eq!(agent_instructions(agent, ""), identity);
        assert_eq!(agent_instructions(agent, "  "), identity);
        assert_eq!(
            agent_instructions(agent, &snapshot.settings.user_name),
            format!("{identity} Your user is \"Álex \\\"Al\\\"\".")
        );
    }

    #[test]
    fn tab_style_round_trips_through_settings_json() {
        let mut settings = default_snapshot().settings;
        settings.tab_style = "modern".into();
        let serialized = serde_json::to_string(&settings).unwrap();
        assert!(serialized.contains("\"tabStyle\":\"modern\""));
        assert_eq!(
            serde_json::from_str::<Settings>(&serialized).unwrap(),
            settings
        );
    }

    #[test]
    fn interface_density_round_trips_through_settings_json() {
        let mut settings = default_snapshot().settings;
        settings.interface_density = "tight".into();
        let serialized = serde_json::to_string(&settings).unwrap();
        assert!(serialized.contains("\"interfaceDensity\":\"tight\""));
        assert_eq!(
            serde_json::from_str::<Settings>(&serialized).unwrap(),
            settings
        );
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
        assert!(valid_sandbox_for_provider("acp", "yolo"));
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
        assert!(snapshot.approval_requests.is_empty());
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
    fn legacy_agent_and_task_without_codex_home_deserialize() {
        let agent: Agent = serde_json::from_value(serde_json::json!({
            "id":"a", "name":"Codex", "description":"", "instructions":"",
            "provider":"codex", "model":"", "hostId":"h", "cwd":"/tmp",
            "color":"#000", "sandbox":"read-only"
        })).unwrap();
        let task: Task = serde_json::from_value(serde_json::json!({
            "id":"t", "agentId":"a", "title":"Legacy", "nativeSessionId":null,
            "status":"idle", "createdAt":1, "updatedAt":1, "parentTaskId":null,
            "channelId":null, "hostId":"h", "cwd":"/tmp", "provider":"codex",
            "model":"", "sandbox":"read-only", "projectId":null
        })).unwrap();
        assert_eq!(agent.codex_home, None);
        assert_eq!(task.codex_home, None);
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
        codex_home: None,
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
        acp: (agent.provider == "acp")
            .then(|| agent.acp.clone())
            .flatten(),
    }
}
