//! Pure semantic projection over a chronological native transcript.
//!
//! The native renderer holds a flat [`TimelineRow`] sequence in `transcript.rs`.
//! This module re-derives a grouped view — message / activity / reasoning-group /
//! tool-group / process-group / approval — that mirrors the Svelte
//! `ConversationActivityItem` shape. Group identity uses the **first retained
//! event id**, which keeps virtual-list keys stable across live updates,
//! pagination, and journal resets.
//!
//! The projection never invents activity or model cost facts: blank reasoning
//! (no `reasoningSummary`) is filtered out, transport-artifact activities are
//! dropped, and approval rows are emitted only when the request is resolved.
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use super::TimelineRow;

/// User-cancellation title used by the native cancellation path. Kept here so
/// the projection can short-circuit stale status titles without recomputing the
/// public surface each render.
pub const CANCELLATION_EVENT_TITLE: &str = "You cancelled this run.";

fn field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn field_opt<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn as_record(value: &Value) -> Option<&serde_json::Map<String, Value>> {
    value.as_object()
}

trait EventTitle {
    fn title(&self) -> Option<&str>;
}
impl EventTitle for Value {
    fn title(&self) -> Option<&str> {
        self.get("title").and_then(Value::as_str)
    }
}

fn parse_detail(detail: &str) -> Option<Value> {
    let mut current = detail.trim().to_owned();
    // Some harness adapters retain the provider payload as a JSON string
    // inside the event JSON. Unwrap up to 2 transport layers so the
    // projection can recognise the activity type.
    for _ in 0..3 {
        if current.is_empty() {
            return None;
        }
        match serde_json::from_str::<Value>(&current) {
            Ok(Value::String(boxed)) => current = boxed.trim().to_owned(),
            Ok(value) => return Some(value),
            Err(_) => return None,
        }
    }
    None
}

/// Native collaboration items are kept distinct from Monitter-routed delegations.
/// Both `collabAgentToolCall` and `subAgentActivity` are recognised by Codex.
#[derive(Clone, Debug, PartialEq)]
pub struct NativeSubagentActivity {
    pub kind: &'static str,
    pub id: String,
    pub action: String,
    pub phase: String,
    pub agent_path: Option<String>,
    pub agent_thread_id: Option<String>,
    pub receiver_thread_ids: Vec<String>,
    pub prompt: Option<String>,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
}

fn native_subagent_activity(event: &Value) -> Option<NativeSubagentActivity> {
    let kind = field(event, "kind");
    if kind != "tool" && kind != "subagent" {
        return None;
    }
    let detail = field(event, "detail");
    let parsed = parse_detail(&detail)?;
    let record = as_record(&parsed)?;
    let activity = match record.get("activity").and_then(as_record) {
        Some(activity) => Value::Object(activity.clone()),
        None => parsed.clone(),
    };
    let activity_record = match as_record(&activity) {
        Some(record) => record,
        None => return None,
    };
    let kind = match activity_record.get("type").and_then(Value::as_str) {
        Some("collabAgentToolCall") => "collabAgentToolCall",
        Some("subAgentActivity") => "subAgentActivity",
        Some(_) => {
            if activity_record.contains_key("agentThreadId")
                || activity_record.contains_key("agent_thread_id")
            {
                "subAgentActivity"
            } else {
                return None;
            }
        }
        None => return None,
    };
    let id = field(&activity, "id");
    let id = if id.is_empty() {
        field(event, "id")
    } else {
        id
    };
    let action = field(&activity, "tool");
    let action = if action.is_empty() {
        field(&activity, "kind")
    } else {
        action
    };
    let phase = field(&activity, "status");
    let agent_path = field_opt(&activity, "agentPath").map(str::to_owned);
    let agent_thread_id = field_opt(&activity, "agentThreadId")
        .or_else(|| field_opt(&activity, "agent_thread_id"))
        .map(str::to_owned);
    let receivers = activity_record
        .get("receiverThreadIds")
        .or_else(|| activity_record.get("receiver_thread_ids"));
    let receiver_thread_ids = match receivers.and_then(Value::as_array) {
        Some(values) => values
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        None => Vec::new(),
    };
    let prompt = field_opt(&activity, "prompt")
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let model = field_opt(&activity, "model")
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let reasoning_effort = field_opt(&activity, "reasoningEffort")
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    Some(NativeSubagentActivity {
        kind,
        id,
        action,
        phase,
        agent_path,
        agent_thread_id,
        receiver_thread_ids,
        prompt,
        model,
        reasoning_effort,
    })
}

fn acp_update(event: &Value) -> Option<Value> {
    let parsed = parse_detail(&field(event, "detail"))?;
    let record = as_record(&parsed)?;
    let update = record.get("update")?.clone();
    let update_record = as_record(&update)?;
    let session_update = update_record
        .get("sessionUpdate")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if matches!(session_update, "tool_call" | "tool_call_update" | "plan") {
        Some(update)
    } else {
        None
    }
}

fn tool_identity(event: &Value) -> String {
    if let Some(update) = acp_update(event) {
        if let Some(record) = as_record(&update) {
            let session_update = record
                .get("sessionUpdate")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if session_update == "plan" {
                return "plan".to_owned();
            }
            let acp_kind = record
                .get("kind")
                .and_then(Value::as_str)
                .map(normalize_tool_label);
            if let Some(kind) = acp_kind {
                return kind;
            }
            if let Some(title) = record
                .get("title")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                return title.to_lowercase();
            }
            return "tool".to_owned();
        }
    }
    let title = normalize_tool_label(event.title().unwrap_or_default());
    let title_lower = title.trim().to_lowercase();
    let specific = !matches!(
        title_lower.as_str(),
        "tool activity" | "acp tool activity" | "tool result" | "tool" | "tool_result" | ""
    );
    if let Some(detail) = parse_detail(&field(event, "detail")) {
        if let Some(record) = as_record(&detail) {
            if let Some(kind) = record.get("type").and_then(Value::as_str) {
                let kind = normalize_tool_label(kind);
                if matches!(
                    kind.as_str(),
                    "command_execution" | "web_search" | "file_change"
                ) {
                    return kind;
                }
            }
            if specific {
                return title_lower;
            }
            if let Some(tool) = record.get("tool").and_then(Value::as_str) {
                let tool = tool.trim();
                if !tool.is_empty() {
                    return normalize_tool_label(tool);
                }
            }
        }
    } else if specific {
        return title_lower;
    }
    if specific {
        title_lower
    } else {
        format!("event:{}", field(event, "id"))
    }
}

fn normalize_tool_label(value: &str) -> String {
    let trimmed = value.trim().to_lowercase();
    let collapsed = trimmed.replace(['-', '_', ' '], "");
    match collapsed.as_str() {
        "websearch" => "web_search".to_owned(),
        "commandexecution" | "runcommand" => "command_execution".to_owned(),
        "filechange" => "file_change".to_owned(),
        _ => trimmed,
    }
}

/// A family is the connector or tool namespace preceding the method. Two tool
/// calls share a family when their identity and provider stay constant; family
/// survives transient thinking rows in the same turn.
fn tool_family(identity: &str) -> String {
    if let Some(captures) = connector_capture(identity) {
        return captures;
    }
    if let Some((family, _)) = identity.split_once(['.', '/']).and_then(|(family, rest)| {
        if !rest.is_empty() {
            Some((family, rest))
        } else {
            None
        }
    }) {
        return family.to_owned();
    }
    identity.to_owned()
}

fn connector_capture(identity: &str) -> Option<String> {
    let stripped = identity.strip_prefix("mcp__").unwrap_or(identity);
    let mut parts = stripped.split("__");
    let first = parts.next()?;
    if first.is_empty() {
        return None;
    }
    let second = parts.next()?;
    if second.is_empty() {
        return None;
    }
    Some(first.to_owned())
}

fn is_shell_activity(event: &Value) -> bool {
    let identity = tool_identity(event);
    if matches!(
        identity.as_str(),
        "command_execution"
            | "bash"
            | "shell"
            | "shell_command"
            | "exec_command"
            | "functions.exec_command"
    ) {
        return true;
    }
    let title = field(event, "title");
    let trimmed = title.trim();
    if let Some(rest) = trimmed
        .strip_prefix('/')
        .and_then(|rest| rest.split_whitespace().next())
    {
        let leaf = rest.rsplit('/').next().unwrap_or(rest);
        let lower = leaf.to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "bash" | "zsh" | "fish" | "ksh" | "dash" | "sh"
        ) {
            return true;
        }
    }
    let lower = trimmed.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "bash" | "zsh" | "fish" | "ksh" | "dash" | "sh"
    ) || lower.starts_with("bash ")
        || lower.starts_with("zsh ")
        || lower.starts_with("fish ")
        || lower.starts_with("ksh ")
        || lower.starts_with("dash ")
        || lower.starts_with("sh ")
}

fn normalize_compaction_label(value: &str) -> String {
    value
        .chars()
        .filter(|ch| !ch.is_whitespace() && *ch != '_' && *ch != '-')
        .flat_map(|ch| ch.to_lowercase())
        .collect()
}

fn context_compaction_id(event: &Value) -> Option<String> {
    let detail = parse_detail(&field(event, "detail"));
    let type_label = detail
        .as_ref()
        .and_then(|value| as_record(value).and_then(|record| record.get("type")))
        .and_then(Value::as_str)
        .map(normalize_compaction_label)
        .unwrap_or_else(|| normalize_compaction_label(&field(event, "title")));
    if type_label != "contextcompaction" {
        return None;
    }
    let id = detail
        .as_ref()
        .and_then(|value| as_record(value))
        .and_then(|record| record.get("id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    Some(id.to_owned())
}

fn context_compaction_turn(event: &Value) -> Option<String> {
    parse_detail(&field(event, "detail"))
        .and_then(|detail| detail["monitterTurnId"].as_str().map(str::to_owned))
        .filter(|turn| !turn.trim().is_empty())
}

fn context_compaction_phase(event: &Value) -> Option<String> {
    parse_detail(&field(event, "detail"))
        .and_then(|detail| detail["monitterPhase"].as_str().map(str::to_owned))
}

/// Person and peer user inputs may start/steer a run. Local system notices
/// and finished local answers are conservative boundaries too: the UI has no
/// authoritative active-turn DTO. Peer result/output notices do not start turns.
pub(crate) fn context_compaction_boundary_at(message: &Value) -> Option<i64> {
    let local = field(message, "senderAgentId").is_empty()
        && field(message, "collaborationId").is_empty();
    (field(message, "role") == "user"
        || (local && field(message, "role") == "system")
        || (local && field(message, "role") == "assistant"
            && field(message, "phase") == "final_answer"
            && field(message, "streamStatus") != "streaming"))
        .then(|| message["createdAt"].as_i64()).flatten()
}

pub(crate) fn is_context_compaction(event: &Value) -> bool {
    if normalize_compaction_label(&field(event, "title")) == "contextcompaction" {
        return true;
    }
    parse_detail(&field(event, "detail"))
        .as_ref()
        .and_then(|value| as_record(value))
        .and_then(|record| record.get("type"))
        .and_then(Value::as_str)
        .map(normalize_compaction_label)
        .map(|label| label == "contextcompaction")
        .unwrap_or(false)
}

/// Explicit provider lifecycle, independent of a task later becoming idle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ContextCompaction {
    pub active: bool,
    pub completed: bool,
    pub duration_ms: Option<i64>,
}

pub(crate) fn context_compaction(events: &[Value], running: bool) -> Option<ContextCompaction> {
    let first = events.first()?;
    if !events.iter().all(is_context_compaction) { return None; }
    let identity = context_compaction_id(first);
    let same_identity = identity.is_some() && events.iter().all(|event|
        field(event, "taskId") == field(first, "taskId")
            && context_compaction_id(event) == identity
            && context_compaction_turn(event) == context_compaction_turn(first));
    let phase = context_compaction_phase;
    let completed = events.iter().any(|event| phase(event).as_deref() == Some("completed"));
    let started = events.iter().any(|event| phase(event).as_deref() == Some("started"));
    let start = events.iter().filter(|event| phase(event).as_deref() == Some("started"))
        .filter_map(|event| event["createdAt"].as_i64()).min();
    // Repeated start/completion notifications do not inflate elapsed time.
    // An inverted pair cannot establish a duration.
    let end = start.and_then(|start| events.iter()
        .filter(|event| phase(event).as_deref() == Some("completed"))
        .filter_map(|event| event["createdAt"].as_i64())
        .filter(|end| *end >= start).min());
    Some(ContextCompaction {
        active: running && same_identity && started && !completed,
        completed,
        duration_ms: if same_identity { start.zip(end).map(|(start, end)| end - start) } else { None },
    })
}

/// Historical unmatched starts must not animate after later conversation input
/// (including peer delivery) or a completed final answer. Missing scope is neutral.
pub(crate) fn context_compaction_running(
    running: bool,
    events: &[Rc<Value>],
    latest_input_at: Option<i64>,
) -> bool {
    running && (!events.iter().any(|event| is_context_compaction(event))
        || latest_input_at.is_some_and(|input_at| events.first()
            .and_then(|event| event["createdAt"].as_i64())
            .is_some_and(|event_at| event_at > input_at)))
}

/// Displayable summary text for a reasoning event. Reasoning summaries may be
/// delivered as plain text or as a JSON object containing a `summary`/`text`/
/// block. Returns an empty string when no text is recoverable.
pub(crate) fn reasoning_summary(event: &Value) -> String {
    let trimmed = field(event, "detail").trim().to_owned();
    if trimmed.is_empty() {
        return String::new();
    }
    // Structured reasoning with no text is a Thinking marker, not a raw
    // payload disclosure. This follows the Tauri reasoningSummary contract.
    match serde_json::from_str::<Value>(&trimmed) {
        Ok(value) => extract_text(&value, 0),
        Err(_) => trimmed,
    }
}

fn extract_text(value: &Value, depth: usize) -> String {
    if depth > 8 {
        return String::new();
    }
    match value {
        Value::String(text) => text.trim().to_owned(),
        Value::Array(items) => items
            .iter()
            .map(|item| extract_text(item, depth + 1))
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n"),
        Value::Object(record) => {
            let summary = record
                .get("summary")
                .map(|raw| extract_text(raw, depth + 1))
                .unwrap_or_default();
            if !summary.is_empty() {
                return summary;
            }
            let text = record
                .get("text")
                .map(|raw| extract_text(raw, depth + 1))
                .unwrap_or_default();
            if !text.is_empty() {
                return text;
            }
            record
                .get("content")
                .map(|raw| extract_text(raw, depth + 1))
                .unwrap_or_default()
        }
        _ => String::new(),
    }
}

fn is_blank_reasoning(event: &Value) -> bool {
    field(event, "kind") == "reasoning" && reasoning_summary(event).is_empty()
}

/// Codex app-server lifecycle echoes for conversation items are not tool work.
/// Their completed counterparts are persisted as ordinary bubbles by the
/// adapter; only a structured native item type is safe to hide here.
fn is_native_message_transport_artifact(event: &Value) -> bool {
    if field(event, "kind") != "tool" {
        return false;
    }
    let parsed = parse_detail(&field(event, "detail"));
    let detail_type = parsed
        .as_ref()
        .and_then(|value| as_record(value))
        .and_then(|record| record.get("type"))
        .and_then(Value::as_str)
        .map(str::to_lowercase);
    matches!(
        detail_type.as_deref(),
        Some("usermessage") | Some("agentmessage")
    )
}

fn is_cancellation_event(event: &Value) -> bool {
    field(event, "kind") == "status" && field(event, "title") == CANCELLATION_EVENT_TITLE
}

/// Match the events AppSurface exposes to its conversation grouping. Status,
/// usage and other accounting telemetry belongs in the diagnostic timeline,
/// never in the visible tool count or process tree. Keep the user-facing
/// cancellation marker, which has its own transcript presentation.
fn is_displayable_conversation_activity(event: &Value) -> bool {
    matches!(
        field(event, "kind").as_str(),
        "tool" | "reasoning" | "collaboration" | "subagent"
    ) || is_cancellation_event(event)
}

/// Approval rows passed into the projection. Only non-pending requests are
/// shown as inline history; pending approvals live in the request list.
#[derive(Clone, Debug, PartialEq)]
pub struct ApprovalRow {
    pub id: String,
    pub task_id: String,
    pub status: String,
    pub created_at: i64,
    pub resolved_at: Option<i64>,
    pub rule_id: Option<String>,
    pub summary: String,
    pub tool: String,
}

impl ApprovalRow {
    pub fn at(&self) -> i64 {
        self.resolved_at.unwrap_or(self.created_at)
    }
}

/// The semantic view the parent will eventually render. The discriminated union
/// preserves the structural contract of the Svelte `ConversationActivityItem`.
#[derive(Clone, Debug, PartialEq)]
pub enum ConversationItem {
    Message(Rc<Value>),
    Activity(Rc<Value>),
    Approval(ApprovalRow),
    ExchangeGroup { members: Vec<ConversationItem>, peer_key: String, exchange_count: usize },
    ReasoningGroup(Vec<Rc<Value>>),
    ToolGroup(Vec<Rc<Value>>),
    ProcessGroup {
        values: Vec<Rc<Value>>,
        approvals: Vec<ApprovalRow>,
        groups: Vec<NestedGroup>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum NestedGroup {
    Reasoning(Vec<Rc<Value>>),
    Tool(Vec<Rc<Value>>),
}

impl ConversationItem {
    /// Leaves retain source identity while their list index belongs to the run.
    pub fn leaves(&self) -> Vec<&ConversationItem> {
        match self { Self::ExchangeGroup {members,..} => members.iter().flat_map(Self::leaves).collect(), _=>vec![self] }
    }
    pub fn contains_source(&self, key: &str) -> bool {
        self.leaves().iter().any(|row| {
            row.stable_key() == key || match row {
                Self::ToolGroup(values) | Self::ReasoningGroup(values) | Self::ProcessGroup { values, .. } =>
                    values.iter().any(|row| key == format!("activity:{}", field(row,"id"))),
                _ => false,
            }
        })
    }
    pub fn exchange_disclosure_key(&self) -> String {
        match self {
            Self::Message(row) => field(row,"id"),
            Self::Activity(row) => format!("event:{}",field(row,"id")),
            _ => self.stable_key(),
        }
    }
    pub fn activity_tool_keys(&self) -> HashSet<String> { self.activity_tool_keys_matching(false) }
    pub fn activity_failed_tool_keys(&self) -> HashSet<String> { self.activity_tool_keys_matching(true) }
    fn activity_tool_keys_matching(&self, failed_only: bool) -> HashSet<String> {
        let values = match self {
            Self::ToolGroup(values) | Self::ReasoningGroup(values) | Self::ProcessGroup { values, .. } => values,
            _ => return HashSet::new(),
        };
        values.iter().filter(|row| matches!(field(row,"kind").as_str(),"tool" | "subagent") && !is_context_compaction(row))
            .filter(|row| !failed_only || parse_detail(&field(row,"detail")).is_some_and(|d| d["update"]["status"].as_str().or_else(||d["status"].as_str()) == Some("failed")))
            .map(|row| {
                let detail = parse_detail(&field(row,"detail"));
                detail.as_ref().and_then(|d| d["toolCallId"].as_str().or_else(||d["item"]["id"].as_str()).or_else(||d["id"].as_str()))
                    .map(str::to_owned).unwrap_or_else(||field(row,"id"))
            }).collect::<HashSet<_>>()
    }
    pub fn at(&self) -> i64 {
        match self {
            Self::Message(value) | Self::Activity(value) => {
                value["createdAt"].as_i64().unwrap_or(0)
            }
            Self::Approval(row) => row.at(),
            Self::ExchangeGroup { members, .. } => members.first().map(Self::at).unwrap_or(0),
            Self::ReasoningGroup(values)
            | Self::ToolGroup(values)
            | Self::ProcessGroup { values, .. } => values
                .first()
                .and_then(|value| value["createdAt"].as_i64())
                .unwrap_or(0),
        }
    }
    /// Stable virtual-list identity. The first retained event/message id never
    /// shifts under append, pagination, or reset; it survives `before_id` and
    /// scroll anchor restarts.
    pub fn stable_key(&self) -> String {
        match self {
            Self::Message(value) => format!("message:{}", field(value, "id")),
            Self::Activity(value) => format!("activity:{}", field(value, "id")),
            Self::Approval(row) => format!("approval:{}", row.id),
            Self::ExchangeGroup { members, .. } => members.first().map(Self::stable_key).unwrap_or_default(),
            Self::ReasoningGroup(values) => {
                format!("reasoning-group:{}", first_id(values))
            }
            Self::ToolGroup(values) => format!("tool-group:{}", first_id(values)),
            Self::ProcessGroup { values, .. } => {
                format!("process-group:{}", first_id(values))
            }
        }
    }
    /// Number of underlying rows this item spans. Used by the parent to keep
    /// held rows, page merges, and jump anchors aligned with the projection.
    pub fn row_count(&self) -> usize {
        match self {
            Self::Message(_) | Self::Activity(_) | Self::Approval(_) => 1,
            Self::ExchangeGroup { members, .. } => members.iter().map(Self::row_count).sum(),
            Self::ReasoningGroup(values)
            | Self::ToolGroup(values)
            | Self::ProcessGroup { values, .. } => values.len(),
        }
    }
}

fn first_id(values: &[Rc<Value>]) -> String {
    values
        .first()
        .map(|value| field(value, "id"))
        .unwrap_or_default()
}

/// Coalesce native subagent start/completion envelopes that share an item id.
fn coalesce_native_subagent(events: Vec<Rc<Value>>) -> Vec<Rc<Value>> {
    let mut result: Vec<Rc<Value>> = Vec::with_capacity(events.len());
    let mut positions: HashMap<String, usize> = HashMap::new();
    for event in events {
        let activity = match native_subagent_activity(&event) {
            Some(activity) => activity,
            None => {
                result.push(event);
                continue;
            }
        };
        let key = format!(
            "{}:{}:{}",
            field(&event, "taskId"),
            activity.kind,
            activity.id
        );
        match positions.get(&key).copied() {
            Some(index) => {
                if let Some(existing) = result.get_mut(index) {
                    let prior_at = existing["createdAt"].as_i64().unwrap_or(0);
                    let prior_id = existing["id"].clone();
                    let mut next = event;
                    if let Some(map) = Rc::make_mut(&mut next).as_object_mut() {
                        map.insert("id".to_owned(), prior_id);
                        let replacement_at = if prior_at != 0 {
                            prior_at
                        } else {
                            map.get("createdAt").and_then(Value::as_i64).unwrap_or(0)
                        };
                        map.insert("createdAt".to_owned(), Value::Number(replacement_at.into()));
                    }
                    *existing = next;
                }
            }
            None => {
                positions.insert(key, result.len());
                result.push(event);
            }
        }
    }
    result
}

fn acp_tool_call_id(event: &Value) -> Option<String> {
    let update = acp_update(event)?;
    let record = as_record(&update)?;
    if record.get("sessionUpdate").and_then(Value::as_str) == Some("plan") {
        return Some("plan".to_owned());
    }
    let id = record.get("toolCallId").and_then(Value::as_str)?;
    let id = id.trim();
    if id.is_empty() {
        None
    } else {
        Some(id.to_owned())
    }
}

/// Collapse old lifecycle echoes without rewriting the diagnostic journal.
fn coalesce_acp_activity(events: Vec<Rc<Value>>, messages: &[Rc<Value>]) -> Vec<Rc<Value>> {
    let mut result: Vec<Rc<Value>> = Vec::with_capacity(events.len());
    let mut positions: HashMap<String, usize> = HashMap::new();
    let boundaries: Vec<(String, i64)> = messages
        .iter()
        .filter(|message| matches!(field(message, "role").as_str(), "user" | "system"))
        .map(|message| {
            (
                field(message, "taskId"),
                message["createdAt"].as_i64().unwrap_or(0),
            )
        })
        .collect();
    for event in events {
        let update = match acp_update(&event) {
            Some(update) => update,
            None => {
                result.push(event);
                continue;
            }
        };
        let call_id = match acp_tool_call_id(&event) {
            Some(id) => id,
            None => {
                result.push(event);
                continue;
            }
        };
        let session_id = parse_detail(&field(&event, "detail"))
            .as_ref()
            .and_then(|value| as_record(value))
            .and_then(|record| record.get("sessionId"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let task_id = field(&event, "taskId");
        let event_at = event["createdAt"].as_i64().unwrap_or(0);
        let boundary = boundaries
            .iter()
            .filter(|(tid, at)| tid == &task_id && *at <= event_at)
            .map(|(_, at)| *at)
            .fold(-1i64, i64::max);
        let is_plan = call_id == "plan";
        let key = format!(
            "{}|{}|{}|{}|{}",
            task_id,
            session_id,
            boundary,
            if is_plan { "plan" } else { "tool" },
            call_id
        );
        match positions.get(&key).copied() {
            Some(index) => {
                let update_record = as_record(&update).cloned().unwrap_or_default();
                let prior_at = result
                    .get(index)
                    .and_then(|value| value["createdAt"].as_i64())
                    .unwrap_or(0);
                let prior_title = result
                    .get(index)
                    .map(|value| field(value, "title"))
                    .unwrap_or_default();
                let prior_update_record = result
                    .get(index)
                    .and_then(|value| acp_update(value))
                    .and_then(|value| as_record(&value).cloned())
                    .unwrap_or_default();
                let mut merged = serde_json::Map::new();
                for (key, value) in prior_update_record {
                    if !value.is_null() {
                        merged.insert(key, value);
                    }
                }
                for (key, value) in update_record {
                    if !value.is_null() {
                        merged.insert(key, value);
                    }
                }
                let title = merged
                    .get("title")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .unwrap_or(prior_title);
                let envelope_object = parse_detail(&field(&event, "detail"))
                    .unwrap_or(Value::Null)
                    .as_object()
                    .cloned()
                    .unwrap_or_default();
                let mut envelope_value = Value::Object(envelope_object);
                if let Some(map) = envelope_value.as_object_mut() {
                    map.insert("update".to_owned(), Value::Object(merged));
                }
                let detail = envelope_value.to_string();
                let mut replacement = event.clone();
                if let Some(map) = Rc::make_mut(&mut replacement).as_object_mut() {
                    if prior_at != 0 {
                        map.insert("createdAt".to_owned(), Value::Number(prior_at.into()));
                    }
                    map.insert("title".to_owned(), Value::String(title));
                    map.insert("detail".to_owned(), Value::String(detail));
                }
                if let Some(slot) = result.get_mut(index) {
                    *slot = replacement;
                }
            }
            None => {
                positions.insert(key, result.len());
                result.push(event);
            }
        }
    }
    result
}

/// Build a flat, chronologically-sorted source list from the native timeline.
/// The grouped view is a pure projection of this list.
pub fn projection<'a>(
    rows: impl IntoIterator<Item = &'a TimelineRow>,
    approvals: &[ApprovalRow],
) -> Vec<ConversationItem> {
    projection_with_options(rows, approvals, true)
}

/// Project the transcript while optionally batching adjacent same-family tool
/// calls into one disclosure. Reasoning and mixed process grouping stay intact.
pub fn projection_with_options<'a>(
    rows: impl IntoIterator<Item = &'a TimelineRow>,
    approvals: &[ApprovalRow],
    compress_tools: bool,
) -> Vec<ConversationItem> {
    let mut messages: Vec<Rc<Value>> = Vec::new();
    let mut events: Vec<Rc<Value>> = Vec::new();
    for row in rows {
        match row {
            TimelineRow::Message(value) => messages.push(value.clone()),
            TimelineRow::Activity(value) => events.push(value.clone()),
        }
    }
    // Keep the same user-facing kind filter as Tauri's AppSurface. In
    // particular, usage/context accounting must not become a visible tool
    // leaf or inflate process-tree counts. Preserve the cancellation marker.
    let events: Vec<Rc<Value>> = events
        .into_iter()
        .filter(|event| is_displayable_conversation_activity(event))
        .filter(|event| !is_native_message_transport_artifact(event))
        .collect();
    let events = coalesce_acp_activity(coalesce_native_subagent(events), &messages);
    let mut ordered: Vec<OrderedItem> =
        Vec::with_capacity(messages.len() + events.len() + approvals.len());
    for value in &messages {
        ordered.push(OrderedItem::Message(value.clone()));
    }
    for value in &events {
        ordered.push(OrderedItem::Activity(value.clone()));
    }
    for row in approvals {
        if row.status == "pending"
            || (row.status == "approved"
                && row.rule_id.as_ref().is_some_and(|id| !id.trim().is_empty()))
        {
            continue;
        }
        ordered.push(OrderedItem::Approval(row.clone()));
    }
    ordered.sort_by_key(OrderedItem::at);

    // A finished final answer is the visual end of that turn: keep any
    // immediately following reasoning from the same task above it without
    // rewriting history.
    let display_ordered = reorder_final_answer(ordered);

    let mut grouped: Vec<ConversationItem> = Vec::new();
    let mut compaction_positions: HashMap<(String, String, Option<String>, String), usize> = HashMap::new();
    let mut compaction_boundaries: HashMap<String, String> = HashMap::new();
    for item in display_ordered {
        if let OrderedItem::Message(message) = &item {
            if context_compaction_boundary_at(message).is_some() {
                compaction_boundaries.insert(field(message, "taskId"), field(message, "id"));
            }
        }
        match item {
            OrderedItem::Activity(value) if is_context_compaction(&value) => {
                let task_id = field(&value, "taskId");
                let key = context_compaction_id(&value).map(|id| (
                    task_id.clone(), id, context_compaction_turn(&value),
                    if context_compaction_turn(&value).is_some() { String::new() }
                    else { compaction_boundaries.get(&task_id).cloned().unwrap_or_default() },
                ));
                let existing = key.as_ref().and_then(|key| compaction_positions.get(key)).copied()
                    .filter(|index| {
                        // A new start after a completed legacy item is another
                        // lifecycle even if the provider reused its native ID.
                        context_compaction_turn(&value).is_some()
                            || context_compaction_phase(&value).as_deref() != Some("started")
                            || !matches!(&grouped[*index], ConversationItem::ToolGroup(values)
                                if values.iter().any(|event| context_compaction_phase(event).as_deref() == Some("completed")))
                    });
                if let Some(index) = existing {
                    if let Some(ConversationItem::ToolGroup(values)) = grouped.get_mut(index) {
                        values.push(value);
                    }
                } else {
                    if let Some(key) = key { compaction_positions.insert(key, grouped.len()); }
                    grouped.push(ConversationItem::ToolGroup(vec![value]));
                }
            }
            OrderedItem::Activity(value) if field(&value, "kind") == "reasoning" => {
                let previous = grouped.last();
                if let Some(ConversationItem::ReasoningGroup(values)) = previous {
                    if first_task(values) == field(&value, "taskId") {
                        let mut next = values.clone();
                        next.push(value);
                        if let Some(ConversationItem::ReasoningGroup(slot)) = grouped.last_mut() {
                            *slot = next;
                        }
                        continue;
                    }
                }
                grouped.push(ConversationItem::ReasoningGroup(vec![value]));
            }
            OrderedItem::Activity(value)
                if matches!(field(&value, "kind").as_str(), "tool" | "subagent") =>
            {
                let tool_value = value;
                let previous = grouped.last();
                let merge = compress_tools
                    && match previous {
                        Some(ConversationItem::ToolGroup(values)) => {
                            let last = values.last().cloned().unwrap_or_else(|| tool_value.clone());
                            let last_compaction = is_context_compaction(&last);
                            let current_compaction = is_context_compaction(&tool_value);
                            let last_id = context_compaction_id(&last);
                            let current_id = context_compaction_id(&tool_value);
                            let compaction = last_compaction || current_compaction;
                            let last_subagent = native_subagent_activity(&last);
                            let current_subagent = native_subagent_activity(&tool_value);
                            if last_subagent.is_some() || current_subagent.is_some() {
                                false
                            } else if compaction {
                                last_id.is_some() && last_id == current_id
                            } else {
                                let failed_last = acp_update(&last)
                                    .as_ref()
                                    .and_then(as_record)
                                    .is_some_and(|r| {
                                        r.get("status").and_then(Value::as_str) == Some("failed")
                                    });
                                let failed_now = acp_update(&tool_value)
                                    .as_ref()
                                    .and_then(as_record)
                                    .is_some_and(|r| {
                                        r.get("status").and_then(Value::as_str) == Some("failed")
                                    });
                                tool_family(&tool_identity(&last))
                                    == tool_family(&tool_identity(&tool_value))
                                    && failed_last == failed_now
                            }
                        }
                        _ => false,
                    };
                if merge {
                    if let Some(ConversationItem::ToolGroup(slot)) = grouped.last_mut() {
                        slot.push(tool_value);
                    }
                } else {
                    grouped.push(ConversationItem::ToolGroup(vec![tool_value]));
                }
            }
            OrderedItem::Activity(value) => {
                grouped.push(ConversationItem::Activity(value));
            }
            OrderedItem::Message(value) => {
                grouped.push(ConversationItem::Message(value));
            }
            OrderedItem::Approval(row) => {
                grouped.push(ConversationItem::Approval(row));
            }
        }
    }

    // Empty reasoning is a temporary waiting indicator, not transcript content.
    let latest_replacement_at = latest_replacement_at(&messages, &events, approvals);
    let visible: Vec<ConversationItem> = grouped
        .into_iter()
        .filter(|item| match item {
            ConversationItem::ReasoningGroup(values) => {
                values.iter().any(|value| !is_blank_reasoning(value))
                    || values.last().map_or(false, |value| {
                        let task_id = field(value, "taskId");
                        latest_replacement_at
                            .get(&task_id)
                            .copied()
                            .unwrap_or(i64::MIN)
                            < value["createdAt"].as_i64().unwrap_or(0)
                    })
            }
            _ => true,
        })
        .collect();

    combine_process_groups(visible, compress_tools)
}

fn latest_replacement_at(
    messages: &[Rc<Value>],
    events: &[Rc<Value>],
    approvals: &[ApprovalRow],
) -> HashMap<String, i64> {
    let mut latest: HashMap<String, i64> = HashMap::new();
    for message in messages {
        let text = field(message, "text");
        let attachments = message["attachments"]
            .as_array()
            .map(|values| !values.is_empty())
            .unwrap_or(false);
        if !text.trim().is_empty() || attachments {
            let task_id = field(message, "taskId");
            let at = message["createdAt"].as_i64().unwrap_or(0);
            let entry = latest.entry(task_id).or_insert(i64::MIN);
            if at > *entry {
                *entry = at;
            }
        }
    }
    for event in events {
        if is_native_message_transport_artifact(event) || is_blank_reasoning(event) {
            continue;
        }
        let task_id = field(event, "taskId");
        let at = event["createdAt"].as_i64().unwrap_or(0);
        let entry = latest.entry(task_id).or_insert(i64::MIN);
        if at > *entry {
            *entry = at;
        }
    }
    for row in approvals {
        if row.status == "pending" {
            continue;
        }
        let at = row.at();
        let entry = latest.entry(row.task_id.clone()).or_insert(i64::MIN);
        if at > *entry {
            *entry = at;
        }
    }
    latest
}

fn first_task(values: &[Rc<Value>]) -> String {
    values
        .first()
        .map(|value| field(value, "taskId"))
        .unwrap_or_default()
}

#[derive(Clone, Debug)]
enum OrderedItem {
    Message(Rc<Value>),
    Activity(Rc<Value>),
    Approval(ApprovalRow),
}

impl OrderedItem {
    fn at(&self) -> i64 {
        match self {
            Self::Message(value) | Self::Activity(value) => {
                value["createdAt"].as_i64().unwrap_or(0)
            }
            Self::Approval(row) => row.at(),
        }
    }
}

fn reorder_final_answer(ordered: Vec<OrderedItem>) -> Vec<OrderedItem> {
    let mut result: Vec<OrderedItem> = Vec::with_capacity(ordered.len());
    let mut index = 0;
    while index < ordered.len() {
        let item = ordered[index].clone();
        let mut advance = true;
        if let OrderedItem::Message(value) = &item {
            if field(value, "role") == "assistant"
                && field(value, "phase") == "final_answer"
                && field(value, "streamStatus") != "streaming"
            {
                let task_id = field(value, "taskId");
                let mut next = index + 1;
                while next < ordered.len() {
                    match &ordered[next] {
                        OrderedItem::Activity(value)
                            if field(value, "kind") == "reasoning"
                                && field(value, "taskId") == task_id => {}
                        _ => break,
                    }
                    next += 1;
                }
                if next > index + 1 {
                    result.extend(ordered[index + 1..next].iter().cloned());
                    result.push(item.clone());
                    index = next;
                    advance = false;
                }
            }
        }
        if advance {
            result.push(item);
            index += 1;
        }
    }
    result
}

/// Combine adjacent reasoning/tool groups that share a task id into a single
/// process group. Approvals that match the leading task are folded into the
/// process group when they appear between its sub-groups.
fn combine_process_groups(
    items: Vec<ConversationItem>,
    compress_tools: bool,
) -> Vec<ConversationItem> {
    let mut result: Vec<ConversationItem> = Vec::with_capacity(items.len());
    let mut pending: Option<ProcessBuilder> = None;
    for item in items {
        match item {
            ConversationItem::Approval(row) => {
                if let Some(builder) = pending.as_mut() {
                    let first_task = builder.task_id.clone();
                    if row.task_id == first_task {
                        builder.approvals.push(row);
                        continue;
                    }
                }
                flush(&mut pending, &mut result, compress_tools);
                result.push(ConversationItem::Approval(row));
            }
            ConversationItem::ReasoningGroup(values) => {
                let task_id = first_task(&values);
                match pending.as_mut() {
                    Some(builder) if builder.task_id == task_id => {
                        builder.groups.push(NestedGroup::Reasoning(values.clone()));
                        builder.values.extend(values);
                        continue;
                    }
                    _ => {}
                }
                flush(&mut pending, &mut result, compress_tools);
                pending = Some(ProcessBuilder::new(task_id));
                if let Some(builder) = pending.as_mut() {
                    builder.groups.push(NestedGroup::Reasoning(values.clone()));
                    builder.values.extend(values);
                }
            }
            ConversationItem::ToolGroup(values) if values.iter().any(|event| is_context_compaction(event)) => {
                flush(&mut pending, &mut result, compress_tools);
                result.push(ConversationItem::ToolGroup(values));
            }
            ConversationItem::ToolGroup(values) => {
                let task_id = first_task(&values);
                if !pending
                    .as_ref()
                    .is_some_and(|builder| builder.task_id == task_id)
                {
                    flush(&mut pending, &mut result, compress_tools);
                    pending = Some(ProcessBuilder::new(task_id));
                }
                if let Some(builder) = pending.as_mut() {
                    builder.groups.push(NestedGroup::Tool(values.clone()));
                    builder.values.extend(values);
                }
            }
            other => {
                flush(&mut pending, &mut result, compress_tools);
                result.push(other);
            }
        }
    }
    flush(&mut pending, &mut result, compress_tools);
    result
}

fn flush(
    builder: &mut Option<ProcessBuilder>,
    sink: &mut Vec<ConversationItem>,
    compress_tools: bool,
) {
    if let Some(builder) = builder.take() {
        let ProcessBuilder {
            values,
            approvals,
            groups,
            ..
        } = builder;
        let has_reasoning = values
            .iter()
            .any(|value| field(value, "kind") == "reasoning" && !is_blank_reasoning(value));
        let has_tools = values
            .iter()
            .any(|value| matches!(field(value, "kind").as_str(), "tool" | "subagent"));
        if has_reasoning && has_tools {
            sink.push(ConversationItem::ProcessGroup {
                values,
                approvals: if approvals.is_empty() {
                    Vec::new()
                } else {
                    approvals
                },
                groups,
            });
        } else if groups.len() > 1
            && (compress_tools || matches!(groups.first(), Some(NestedGroup::Reasoning(_))))
        {
            let start = sink.len();
            // Multiple adjacent same-type groups still warrant a single disclosure.
            if let Some(first) = groups.first() {
                match first {
                    NestedGroup::Reasoning(_) => {
                        sink.push(ConversationItem::ReasoningGroup(values));
                    }
                    NestedGroup::Tool(_) => {
                        sink.push(ConversationItem::ToolGroup(values));
                    }
                }
            }
            sink.extend(approvals.into_iter().map(ConversationItem::Approval));
            sink[start..].sort_by_key(ConversationItem::at);
        } else {
            let start = sink.len();
            for group in groups {
                match group {
                    NestedGroup::Reasoning(values) => {
                        sink.push(ConversationItem::ReasoningGroup(values));
                    }
                    NestedGroup::Tool(values) => {
                        sink.push(ConversationItem::ToolGroup(values));
                    }
                }
            }
            sink.extend(approvals.into_iter().map(ConversationItem::Approval));
            sink[start..].sort_by_key(ConversationItem::at);
        }
    }
}

struct ProcessBuilder {
    task_id: String,
    values: Vec<Rc<Value>>,
    approvals: Vec<ApprovalRow>,
    groups: Vec<NestedGroup>,
}

impl ProcessBuilder {
    fn new(task_id: String) -> Self {
        Self {
            task_id,
            values: Vec::new(),
            approvals: Vec::new(),
            groups: Vec::new(),
        }
    }
}

/// Test-only deterministic family used to compare nested projections across
/// runs. The hash mirrors the field selection the runtime uses to detect
/// family changes; this is a documentation aid and not part of the public
/// projection contract.
#[doc(hidden)]
pub fn family_signature(event: &Value) -> String {
    let identity = tool_identity(event);
    let failed = acp_update(event)
        .as_ref()
        .and_then(as_record)
        .and_then(|record| record.get("status"))
        .and_then(Value::as_str)
        .map(|status| status == "failed")
        .unwrap_or(false);
    let compaction = is_context_compaction(event);
    if compaction {
        let id = context_compaction_id(event).unwrap_or_default();
        format!("compaction:{id}:{failed}")
    } else {
        format!("{}:{}", tool_family(&identity), failed)
    }
}

/// Filter that mirrors `showThinkingFallback` in the Svelte reference. The
/// projection does not consult task status directly; the caller passes
/// `working`/`awaiting` facts in.
pub fn show_thinking_fallback(items: &[ConversationItem], working: bool, awaiting: bool) -> bool {
    if !working || awaiting {
        return false;
    }
    match items.last() {
        Some(ConversationItem::ReasoningGroup(_)) | Some(ConversationItem::ProcessGroup { .. }) => {
            false
        }
        Some(ConversationItem::Message(value)) => field(value, "role") != "assistant",
        _ => true,
    }
}

/// Active-output detector: returns true when the latest visible item is a
/// message or reasoning/tool group that still expects more output.
pub fn has_active_output(items: &[ConversationItem]) -> bool {
    match items.last() {
        Some(ConversationItem::Message(value)) => field(value, "role") == "assistant",
        Some(ConversationItem::ReasoningGroup(_))
        | Some(ConversationItem::ToolGroup(_))
        | Some(ConversationItem::ProcessGroup { .. }) => true,
        Some(ConversationItem::Activity(value)) => matches!(
            field(value, "kind").as_str(),
            "reasoning" | "tool" | "process"
        ),
        _ => false,
    }
}

/// True when the latest retained item matches a request boundary marker
/// (cancellation status, context cleared system message, fork boundary, or
/// settings change). Used by the editor to decide whether a held view should
/// fall back to the live cursor.
pub fn is_request_boundary(item: &ConversationItem) -> bool {
    match item {
        ConversationItem::Activity(value) => is_cancellation_event(value),
        ConversationItem::Message(value) => {
            let role = field(value, "role");
            let text = field(value, "text");
            let sender = field(value, "senderAgentId");
            let attachments = value["attachments"]
                .as_array()
                .map(|values| !values.is_empty())
                .unwrap_or(false);
            role == "system"
                && sender.is_empty()
                && !attachments
                && (text == CANCELLATION_EVENT_TITLE
                    || text == "Context Cleared"
                    || text.starts_with("[Monitter fork] ")
                    || text.starts_with("[Monitter settings change] "))
        }
        _ => false,
    }
}

/// Helper used by `transcript.rs` to expose the projection at the call site.
/// Owns no state; the caller passes the existing flat rows in.
pub fn project<'a>(
    rows: impl IntoIterator<Item = &'a TimelineRow>,
    approvals: &[ApprovalRow],
) -> Vec<ConversationItem> {
    projection(rows, approvals)
}

/// Public options-aware counterpart to `project`; the legacy wrapper keeps
/// the historical compressed-tool projection for existing callers.
pub fn project_with_options<'a>(
    rows: impl IntoIterator<Item = &'a TimelineRow>,
    approvals: &[ApprovalRow],
    compress_tools: bool,
) -> Vec<ConversationItem> {
    projection_with_options(rows, approvals, compress_tools)
}

/// Stable set of event ids already covered by the projection. Used by the
/// renderer to detach retained native entities on authoritative removal.
pub fn visible_event_ids<'a>(
    items: impl IntoIterator<Item = &'a ConversationItem>,
) -> HashSet<String> {
    let mut ids: HashSet<String> = HashSet::new();
    for item in items {
        match item {
            ConversationItem::Activity(value) => {
                ids.insert(field(value, "id"));
            }
            ConversationItem::Approval(_) => {}
            ConversationItem::Message(_) => {}
            ConversationItem::ExchangeGroup { members, .. } => ids.extend(visible_event_ids(members)),
            ConversationItem::ReasoningGroup(values)
            | ConversationItem::ToolGroup(values)
            | ConversationItem::ProcessGroup { values, .. } => {
                for value in values {
                    ids.insert(field(value, "id"));
                }
            }
        }
    }
    ids
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn event(
        id: &str,
        kind: &str,
        task_id: &str,
        created_at: i64,
        title: &str,
        detail: &str,
    ) -> Value {
        json!({
            "id": id,
            "kind": kind,
            "taskId": task_id,
            "createdAt": created_at,
            "title": title,
            "detail": detail,
        })
    }

    fn row_message(id: &str, created_at: i64, role: &str, task_id: &str) -> TimelineRow {
        TimelineRow::Message(Rc::new(json!({
            "id": id,
            "createdAt": created_at,
            "role": role,
            "taskId": task_id,
            "text": format!("{role} message"),
        })))
    }

    #[::core::prelude::v1::test]
    fn projection_shares_large_message_payload_with_source_row() {
        let message = Rc::new(
            json!({"id":"large", "role":"user", "taskId":"task", "createdAt":1, "text":"x".repeat(1024 * 1024)}),
        );
        let source = vec![TimelineRow::Message(message.clone())];
        let projection = project(&source, &[]);
        let ConversationItem::Message(projected) = &projection[0] else {
            panic!("expected message")
        };
        assert!(Rc::ptr_eq(&message, projected));
        let cloned = projection.clone();
        let ConversationItem::Message(cloned_message) = &cloned[0] else {
            panic!("expected message")
        };
        assert!(Rc::ptr_eq(&message, cloned_message));
    }

    fn row_event(value: Value) -> TimelineRow {
        TimelineRow::Activity(value.into())
    }

    #[::core::prelude::v1::test]
    fn reasoning_group_collapses_consecutive_same_task_reasoning_and_preserves_first_id() {
        let rows = vec![
            row_message("msg-1", 10, "user", "task-1"),
            row_event(event(
                "a",
                "reasoning",
                "task-1",
                20,
                "Thinking",
                "{\"summary\":\"one\"}",
            )),
            row_event(event(
                "b",
                "reasoning",
                "task-1",
                25,
                "Thinking",
                "{\"summary\":\"two\"}",
            )),
            row_event(event(
                "c",
                "reasoning",
                "task-1",
                30,
                "Thinking",
                "{\"summary\":\"three\"}",
            )),
        ];
        let items = projection(&rows, &[]);
        assert_eq!(items.len(), 2);
        match &items[1] {
            ConversationItem::ReasoningGroup(values) => {
                assert_eq!(
                    values.iter().map(|v| field(v, "id")).collect::<Vec<_>>(),
                    vec!["a", "b", "c"]
                );
            }
            other => panic!("expected reasoning-group, found {other:?}"),
        }
        assert_eq!(items[1].stable_key(), "reasoning-group:a");
        assert_eq!(items[1].row_count(), 3);
    }

    #[::core::prelude::v1::test]
    fn tool_group_keeps_consecutive_same_family_calls_and_separates_across_messages() {
        let rows = vec![
            row_event(event(
                "a",
                "tool",
                "task-1",
                10,
                "Read a file",
                "{\"kind\":\"read\"}",
            )),
            row_event(event(
                "b",
                "tool",
                "task-1",
                20,
                "Read a file",
                "{\"kind\":\"read\"}",
            )),
            row_message("m1", 25, "assistant", "task-1"),
            row_event(event(
                "c",
                "tool",
                "task-1",
                30,
                "Search files",
                "{\"kind\":\"grep\"}",
            )),
        ];
        let items = projection(&rows, &[]);
        assert_eq!(items.len(), 3);
        match &items[0] {
            ConversationItem::ToolGroup(values) => assert_eq!(values.len(), 2),
            other => panic!("expected tool-group, found {other:?}"),
        }
        assert_eq!(items[0].stable_key(), "tool-group:a");
        match &items[1] {
            ConversationItem::Message(value) => assert_eq!(field(value, "id"), "m1"),
            other => panic!("expected message, found {other:?}"),
        }
        match &items[2] {
            ConversationItem::ToolGroup(values) => {
                assert_eq!(
                    values.iter().map(|v| field(v, "id")).collect::<Vec<_>>(),
                    vec!["c"]
                );
                assert_eq!(items[2].stable_key(), "tool-group:c");
            }
            other => panic!("expected tool-group, found {other:?}"),
        }
    }

    #[::core::prelude::v1::test]
    fn tool_compression_option_controls_same_family_batching() {
        let rows = vec![
            row_event(event(
                "a",
                "tool",
                "task-1",
                10,
                "Read a file",
                "{\"kind\":\"read\"}",
            )),
            row_event(event(
                "b",
                "tool",
                "task-1",
                20,
                "Read another file",
                "{\"kind\":\"read\"}",
            )),
        ];

        let expanded = project_with_options(&rows, &[], false);
        assert_eq!(expanded.len(), 2);
        assert!(
            expanded.iter().all(
                |item| matches!(item, ConversationItem::ToolGroup(values) if values.len() == 1)
            )
        );
        assert_eq!(expanded[0].stable_key(), "tool-group:a");
        assert_eq!(expanded[1].stable_key(), "tool-group:b");

        let compressed = project_with_options(&rows, &[], true);
        assert_eq!(compressed.len(), 1);
        match &compressed[0] {
            ConversationItem::ToolGroup(values) => assert_eq!(values.len(), 2),
            other => panic!("expected compressed tool group, found {other:?}"),
        }
    }

    #[::core::prelude::v1::test]
    fn disabling_tool_compression_preserves_mixed_process_disclosure() {
        let rows = vec![
            row_event(event(
                "r",
                "reasoning",
                "task-1",
                10,
                "Thinking",
                "{\"summary\":\"Checking files\"}",
            )),
            row_event(event(
                "a",
                "tool",
                "task-1",
                20,
                "Read a file",
                "{\"kind\":\"read\"}",
            )),
            row_event(event(
                "b",
                "tool",
                "task-1",
                30,
                "Read another file",
                "{\"kind\":\"read\"}",
            )),
        ];

        let items = project_with_options(&rows, &[], false);
        assert_eq!(items.len(), 1);
        match &items[0] {
            ConversationItem::ProcessGroup { values, groups, .. } => {
                assert_eq!(values.len(), 3);
                assert_eq!(groups.len(), 3);
            }
            other => panic!("expected mixed process group, found {other:?}"),
        }
    }

    #[::core::prelude::v1::test]
    fn process_group_combines_reasoning_then_tools_under_same_task() {
        let rows = vec![
            row_event(event(
                "a",
                "reasoning",
                "task-1",
                10,
                "Thinking",
                "{\"summary\":\"one\"}",
            )),
            row_event(event(
                "b",
                "tool",
                "task-1",
                20,
                "Read a file",
                "{\"kind\":\"read\"}",
            )),
            row_event(event(
                "c",
                "reasoning",
                "task-1",
                25,
                "Thinking",
                "{\"summary\":\"two\"}",
            )),
            row_event(event(
                "d",
                "tool",
                "task-1",
                30,
                "Read a file",
                "{\"kind\":\"read\"}",
            )),
        ];
        let items = projection(&rows, &[]);
        assert_eq!(items.len(), 1);
        match &items[0] {
            ConversationItem::ProcessGroup {
                values,
                groups,
                approvals,
            } => {
                assert_eq!(values.len(), 4);
                assert_eq!(groups.len(), 4);
                assert!(approvals.is_empty());
            }
            other => panic!("expected process-group, found {other:?}"),
        }
        assert_eq!(items[0].stable_key(), "process-group:a");
    }

    #[::core::prelude::v1::test]
    fn different_tasks_open_separate_groups() {
        let rows = vec![
            row_event(event(
                "a",
                "reasoning",
                "task-1",
                10,
                "Thinking",
                "{\"summary\":\"one\"}",
            )),
            row_event(event(
                "b",
                "reasoning",
                "task-2",
                15,
                "Thinking",
                "{\"summary\":\"two\"}",
            )),
        ];
        let items = projection(&rows, &[]);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].stable_key(), "reasoning-group:a");
        assert_eq!(items[1].stable_key(), "reasoning-group:b");
    }

    #[::core::prelude::v1::test]
    fn blank_reasoning_is_filtered_after_a_visible_replacement() {
        let rows = vec![
            row_event(event("a", "reasoning", "task-1", 10, "Thinking", "")),
            row_message("m1", 20, "assistant", "task-1"),
        ];
        let items = projection(&rows, &[]);
        assert_eq!(items.len(), 1);
        match &items[0] {
            ConversationItem::Message(value) => assert_eq!(field(value, "id"), "m1"),
            other => panic!("expected message, found {other:?}"),
        }
    }

    #[::core::prelude::v1::test]
    fn trailing_blank_reasoning_stays_for_thinking_fallback() {
        let rows = vec![
            row_message("m1", 10, "user", "task-1"),
            row_event(event("a", "reasoning", "task-1", 20, "Thinking", "")),
        ];
        let items = projection(&rows, &[]);
        assert_eq!(items.len(), 2);
        assert!(matches!(items[1], ConversationItem::ReasoningGroup(_)));
        assert!(!show_thinking_fallback(&items, true, false));
    }

    #[::core::prelude::v1::test]
    fn structured_empty_reasoning_has_no_displayable_summary() {
        for detail in [
            r#"{"type":"reasoning","id":"r1","summary":[],"content":[]}"#,
            r#"{"summary":[{"text":" "}],"content":[]}"#,
            "null",
            "[]",
        ] {
            assert!(
                reasoning_summary(&event("r", "reasoning", "task-1", 10, "Thinking", detail))
                    .is_empty(),
                "{detail}"
            );
        }
        assert_eq!(
            reasoning_summary(&event(
                "r",
                "reasoning",
                "task-1",
                10,
                "Thinking",
                "  Plain reasoning  "
            )),
            "Plain reasoning"
        );
        assert_eq!(
            reasoning_summary(&event(
                "r",
                "reasoning",
                "task-1",
                10,
                "Thinking",
                r#"{"summary":[{"text":"First"},{"text":"Second"}],"content":[{"text":"Ignored"}]}"#
            )),
            "First\n\nSecond"
        );
    }

    #[::core::prelude::v1::test]
    fn empty_structured_reasoning_is_replaced_by_assistant_but_retained_while_trailing() {
        let reasoning = row_event(event(
            "r",
            "reasoning",
            "task-1",
            10,
            "Thinking",
            r#"{"type":"reasoning","summary":[],"content":[]}"#,
        ));
        let trailing = projection(
            &[row_message("u", 5, "user", "task-1"), reasoning.clone()],
            &[],
        );
        assert!(matches!(
            trailing.last(),
            Some(ConversationItem::ReasoningGroup(_))
        ));
        let completed = projection(
            &[reasoning, row_message("a", 20, "assistant", "task-1")],
            &[],
        );
        assert_eq!(completed.len(), 1);
        assert!(
            matches!(&completed[0], ConversationItem::Message(value) if field(value, "id") == "a")
        );
    }

    #[::core::prelude::v1::test]
    fn message_breaks_a_tool_group_and_starts_a_new_one() {
        let rows = vec![
            row_event(event(
                "a",
                "tool",
                "task-1",
                10,
                "Read",
                "{\"kind\":\"read\"}",
            )),
            row_message("m1", 15, "assistant", "task-1"),
            row_event(event(
                "b",
                "tool",
                "task-1",
                20,
                "Read",
                "{\"kind\":\"read\"}",
            )),
        ];
        let items = projection(&rows, &[]);
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].stable_key(), "tool-group:a");
        assert!(matches!(items[1], ConversationItem::Message(_)));
        assert_eq!(items[2].stable_key(), "tool-group:b");
    }

    #[::core::prelude::v1::test]
    fn transport_artifacts_and_pending_approvals_are_dropped() {
        let rows = vec![
            row_event(event(
                "a",
                "tool",
                "task-1",
                10,
                "Lifecycle",
                "{\"type\":\"userMessage\"}",
            )),
            row_event(event(
                "b",
                "tool",
                "task-1",
                20,
                "Real tool",
                "{\"kind\":\"read\"}",
            )),
        ];
        let approvals = vec![ApprovalRow {
            id: "apr-1".to_owned(),
            task_id: "task-1".to_owned(),
            status: "pending".to_owned(),
            created_at: 25,
            resolved_at: None,
            rule_id: None,
            summary: "Run a command".to_owned(),
            tool: "command_execution".to_owned(),
        }];
        let items = projection(&rows, &approvals);
        assert_eq!(items.len(), 1);
        match &items[0] {
            ConversationItem::ToolGroup(values) => assert_eq!(field(&values[0], "id"), "b"),
            other => panic!("expected tool-group, found {other:?}"),
        }
    }

    #[::core::prelude::v1::test]
    fn usage_accounting_is_not_a_tool_step_but_cancellation_and_failed_tools_survive() {
        let mut failed = event(
            "tool-failed",
            "tool",
            "task-1",
            30,
            "Run command",
            r#"{"update":{"sessionUpdate":"tool_call_update","toolCallId":"call-1","status":"failed"}}"#,
        );
        failed["detail"] = serde_json::json!({
            "update": {
                "sessionUpdate": "tool_call_update",
                "toolCallId": "call-1",
                "status": "failed"
            }
        })
        .to_string()
        .into();
        let rows = vec![
            row_event(event(
                "usage",
                "usage",
                "task-1",
                20,
                "Context Usage Updated",
                "{}",
            )),
            row_event(failed),
            row_event(event(
                "cancel",
                "status",
                "task-1",
                40,
                CANCELLATION_EVENT_TITLE,
                "{}",
            )),
        ];
        let items = projection(&rows, &[]);
        assert_eq!(items.len(), 2, "usage telemetry must not become a row");
        assert!(matches!(&items[0], ConversationItem::ToolGroup(values)
            if values.len() == 1
                && field(&values[0], "id") == "tool-failed"
                && acp_update(&values[0]).as_ref().and_then(as_record)
                    .and_then(|record| record.get("status"))
                    .and_then(Value::as_str) == Some("failed")));
        assert!(matches!(&items[1], ConversationItem::Activity(value)
            if field(value, "id") == "cancel"));
    }

    #[::core::prelude::v1::test]
    fn resolved_approval_becomes_inline_history_in_chronological_order() {
        let rows = vec![row_event(event(
            "a",
            "tool",
            "task-1",
            10,
            "Read",
            "{\"kind\":\"read\"}",
        ))];
        let approvals = vec![ApprovalRow {
            id: "apr-1".to_owned(),
            task_id: "task-1".to_owned(),
            status: "approved".to_owned(),
            created_at: 5,
            resolved_at: Some(12),
            rule_id: None,
            summary: "Read a file".to_owned(),
            tool: "read".to_owned(),
        }];
        let items = projection(&rows, &approvals);
        assert_eq!(items.len(), 2);
        assert!(matches!(items[0], ConversationItem::ToolGroup(_)));
        assert!(matches!(items[1], ConversationItem::Approval(_)));
    }

    #[::core::prelude::v1::test]
    fn saved_rule_approvals_stay_out_of_chat_but_explicit_decisions_remain() {
        let saved = ApprovalRow {
            id: "saved".into(),
            task_id: "task-1".into(),
            status: "approved".into(),
            created_at: 5,
            resolved_at: Some(12),
            rule_id: Some("rule-1".into()),
            summary: "Read a file".into(),
            tool: "read".into(),
        };
        let explicit = ApprovalRow {
            id: "explicit".into(),
            rule_id: None,
            ..saved.clone()
        };
        let denied = ApprovalRow {
            id: "denied".into(),
            status: "denied".into(),
            ..saved.clone()
        };
        let items = projection(&[], &[saved, explicit, denied]);
        assert_eq!(items.len(), 2);
        assert!(
            items
                .iter()
                .all(|item| matches!(item, ConversationItem::Approval(row) if row.id != "saved"))
        );
    }

    #[::core::prelude::v1::test]
    fn live_update_appends_to_open_group_without_changing_first_id() {
        let mut rows = vec![
            row_event(event(
                "a",
                "reasoning",
                "task-1",
                10,
                "Thinking",
                "{\"summary\":\"one\"}",
            )),
            row_event(event(
                "b",
                "reasoning",
                "task-1",
                15,
                "Thinking",
                "{\"summary\":\"two\"}",
            )),
        ];
        let before = projection(&rows, &[]);
        assert_eq!(before.len(), 1);
        rows.push(row_event(event(
            "c",
            "reasoning",
            "task-1",
            18,
            "Thinking",
            "{\"summary\":\"three\"}",
        )));
        let after = projection(&rows, &[]);
        assert_eq!(after[0].stable_key(), before[0].stable_key());
        assert_eq!(after[0].row_count(), 3);
    }

    #[::core::prelude::v1::test]
    fn pagination_extends_first_id_when_older_rows_join_existing_projection() {
        let mut rows = vec![row_event(event(
            "newest",
            "reasoning",
            "task-1",
            30,
            "Thinking",
            "{\"summary\":\"latest\"}",
        ))];
        let after_first = projection(&rows, &[]);
        rows.insert(
            0,
            row_event(event(
                "older",
                "reasoning",
                "task-1",
                20,
                "Thinking",
                "{\"summary\":\"prior\"}",
            )),
        );
        rows.insert(
            0,
            row_event(event(
                "oldest",
                "reasoning",
                "task-1",
                10,
                "Thinking",
                "{\"summary\":\"first\"}",
            )),
        );
        let after_page = projection(&rows, &[]);
        // Pre-existing first id is preserved even when older rows precede it.
        assert_eq!(after_first[0].stable_key(), "reasoning-group:newest");
        assert_eq!(after_page[0].stable_key(), "reasoning-group:oldest");
        assert_eq!(after_page[0].row_count(), 3);
    }

    #[::core::prelude::v1::test]
    fn reconcile_reset_replaces_projection_with_only_retained_event_ids() {
        let rows = vec![
            row_event(event(
                "a",
                "reasoning",
                "task-1",
                10,
                "Thinking",
                "{\"summary\":\"one\"}",
            )),
            row_event(event(
                "b",
                "tool",
                "task-1",
                20,
                "Read",
                "{\"kind\":\"read\"}",
            )),
        ];
        let retained_ids = visible_event_ids(&projection(&rows, &[]));
        assert!(retained_ids.contains("a"));
        assert!(retained_ids.contains("b"));
        let reset_rows = vec![row_event(event(
            "c",
            "reasoning",
            "task-1",
            5,
            "Thinking",
            "{\"summary\":\"only\"}",
        ))];
        let reset_projection = projection(&reset_rows, &[]);
        let reset_ids = visible_event_ids(&reset_projection);
        assert_eq!(reset_ids.len(), 1);
        assert!(reset_ids.contains("c"));
        assert!(!reset_ids.contains("a"));
        assert!(!reset_ids.contains("b"));
    }

    #[::core::prelude::v1::test]
    fn compaction_lifecycle_keeps_started_and_completed_together() {
        let rows = vec![
            row_event(event(
                "start",
                "tool",
                "task-1",
                10,
                "Context Compaction",
                "{\"type\":\"contextCompaction\",\"id\":\"comp-1\",\"monitterPhase\":\"started\"}",
            )),
            row_event(event(
                "end",
                "tool",
                "task-1",
                20,
                "Context Compaction",
                "{\"type\":\"contextCompaction\",\"id\":\"comp-1\",\"monitterPhase\":\"completed\"}",
            )),
        ];
        let items = projection(&rows, &[]);
        assert_eq!(items.len(), 1);
        match &items[0] {
            ConversationItem::ToolGroup(values) => {
                assert_eq!(
                    values.iter().map(|v| field(v, "id")).collect::<Vec<_>>(),
                    vec!["start", "end"]
                );
            }
            other => panic!("expected tool-group, found {other:?}"),
        }
    }

    fn compaction_event(id: &str, native: &str, phase: Option<&str>, at: i64) -> Value {
        event(id, "tool", "task-1", at, "ContextCompaction",
            &json!({"type":"contextCompaction","id":native,"monitterPhase":phase}).to_string())
    }

    #[::core::prelude::v1::test]
    fn compaction_is_separate_from_tool_processes_in_both_compression_modes() {
        for compress in [true, false] {
            let rows = vec![
                row_event(event("reason", "reasoning", "task-1", 1, "Thinking", "Planning")),
                row_event(event("tool", "tool", "task-1", 2, "Read", "{\"kind\":\"read\"}")),
                row_event(compaction_event("start", "c1", Some("started"), 3)),
                row_event(compaction_event("end", "c1", Some("completed"), 4)),
                row_event(event("next", "tool", "task-1", 5, "Read", "{\"kind\":\"read\"}")),
            ];
            let items = projection_with_options(&rows, &[], compress);
            assert_eq!(items.len(), 3);
            assert!(matches!(&items[0], ConversationItem::ProcessGroup { .. }));
            assert!(matches!(&items[1], ConversationItem::ToolGroup(values) if values.len() == 2));
            assert!(items[1].activity_tool_keys().is_empty());
            assert_eq!(visible_event_ids(&items).len(), 5);
            assert_eq!(items[1].stable_key(), "tool-group:start");
        }
    }

    #[::core::prelude::v1::test]
    fn compaction_completion_matches_across_intervening_activity_with_stable_ids() {
        let rows = vec![
            row_event(compaction_event("start", "c1", Some("started"), 1_000)),
            row_event(event("tool", "tool", "task-1", 2_000, "Read", "{}")),
            row_event(compaction_event("end", "c1", Some("completed"), 4_000)),
        ];
        let items = projection_with_options(&rows, &[], false);
        assert_eq!(items.len(), 2);
        let ConversationItem::ToolGroup(values) = &items[0] else { panic!("compaction group"); };
        assert_eq!(values.iter().map(|event| field(event, "id")).collect::<Vec<_>>(), vec!["start", "end"]);
        let values: Vec<_> = values.iter().map(|event| (**event).clone()).collect();
        assert!(!context_compaction(&values, true).unwrap().active);
        assert_eq!(items[0].stable_key(), "tool-group:start");
        assert_eq!(visible_event_ids(&items).len(), 3);
    }

    #[::core::prelude::v1::test]
    fn compaction_lifecycle_handles_duplicates_and_out_of_order_notifications() {
        let start = compaction_event("s1", "c1", Some("started"), 1_000);
        assert_eq!(context_compaction(&[start.clone()], true), Some(ContextCompaction {
            active: true, completed: false, duration_ms: None,
        }));
        let end = compaction_event("e1", "c1", Some("completed"), 4_000);
        let duplicate_start = compaction_event("s2", "c1", Some("started"), 2_000);
        let duplicate_end = compaction_event("e2", "c1", Some("completed"), 5_000);
        assert_eq!(context_compaction(&[end.clone(), duplicate_start, start.clone(), duplicate_end], true), Some(ContextCompaction {
            active: false, completed: true, duration_ms: Some(3_000),
        }));
        let inverted_end = compaction_event("early", "c1", Some("completed"), 500);
        assert_eq!(context_compaction(&[inverted_end, start.clone()], true).unwrap().duration_ms, None);
        assert_eq!(context_compaction(&[end], false).unwrap(), ContextCompaction {
            active: false, completed: true, duration_ms: None,
        });
        assert_eq!(context_compaction(&[start], false).unwrap(), ContextCompaction {
            active: false, completed: false, duration_ms: None,
        });
        let legacy = compaction_event("legacy", "c1", None, 1_000);
        assert_eq!(context_compaction(&[legacy], true).unwrap(), ContextCompaction {
            active: false, completed: false, duration_ms: None,
        });
        let mismatch = compaction_event("other", "c2", Some("completed"), 4_000);
        assert_eq!(context_compaction(&[compaction_event("s", "c1", Some("started"), 1_000), mismatch], true).unwrap().duration_ms, None);
    }

    #[::core::prelude::v1::test]
    fn compaction_animation_requires_the_current_user_turn() {
        let events = vec![Rc::new(compaction_event("s", "c1", Some("started"), 1_000))];
        assert!(context_compaction_running(true, &events, Some(900)));
        assert!(!context_compaction_running(true, &events, Some(1_001)));
        assert!(!context_compaction_running(true, &events, None));
        assert!(!context_compaction_running(false, &events, Some(900)));
        let tool = vec![Rc::new(event("tool", "tool", "task-1", 1, "Read", "{}"))];
        assert!(context_compaction_running(true, &tool, Some(2_000)));
    }

    fn scoped_compaction(id: &str, phase: &str, at: i64, turn: &str) -> Value {
        let mut event = compaction_event(id, "reused-item", Some(phase), at);
        let mut detail = parse_detail(&field(&event, "detail")).unwrap();
        detail["monitterTurnId"] = json!(turn);
        event["detail"] = json!(detail.to_string());
        event
    }

    #[::core::prelude::v1::test]
    fn compaction_peer_input_starts_current_scope_and_retires_prior_peer_scope() {
        let peer = json!({"role":"user","senderAgentId":"peer","collaborationId":"delivery-1","createdAt":100});
        let old_start = vec![Rc::new(compaction_event("old", "native-1", Some("started"), 110))];
        assert_eq!(context_compaction_boundary_at(&peer), Some(100));
        assert!(context_compaction_running(true, &old_start, context_compaction_boundary_at(&peer)));
        let next_peer = json!({"role":"user","senderAgentId":"peer","collaborationId":"delivery-2","createdAt":200});
        assert!(!context_compaction_running(true, &old_start, context_compaction_boundary_at(&next_peer)));
        let current = vec![Rc::new(compaction_event("current", "native-2", Some("started"), 210))];
        assert!(context_compaction_running(true, &current, context_compaction_boundary_at(&next_peer)));
        assert!(!context_compaction_running(true, &old_start, Some(110)), "timestamp ties cannot prove ordering");
    }

    #[::core::prelude::v1::test]
    fn compaction_local_final_is_boundary_but_peer_output_is_not() {
        let local_final = json!({"role":"assistant","phase":"final_answer","streamStatus":"complete","createdAt":200});
        assert_eq!(context_compaction_boundary_at(&local_final), Some(200));
        let old_start = vec![Rc::new(compaction_event("old", "native", Some("started"), 110))];
        assert!(!context_compaction_running(true, &old_start, context_compaction_boundary_at(&local_final)));
        for role in ["assistant", "system"] {
            let output = json!({"role":role,"phase":"final_answer","streamStatus":"complete","senderAgentId":"peer","collaborationId":"delivery","createdAt":200});
            assert_eq!(context_compaction_boundary_at(&output), None);
        }
    }

    #[::core::prelude::v1::test]
    fn compaction_native_item_reuse_is_partitioned_by_provider_turn_or_legacy_input() {
        let scoped_rows = vec![
            row_event(scoped_compaction("old", "started", 10, "turn-1")),
            row_event(scoped_compaction("new", "completed", 20, "turn-2")),
        ];
        let items = projection(&scoped_rows, &[]);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].stable_key(), "tool-group:old");
        assert_eq!(items[1].stable_key(), "tool-group:new");
        let legacy_rows = vec![
            row_event(compaction_event("old", "reused", Some("started"), 10)),
            TimelineRow::Message(Rc::new(json!({"id":"next-input","taskId":"task-1","role":"user","senderAgentId":"peer","createdAt":15}))),
            row_event(compaction_event("new", "reused", Some("completed"), 20)),
        ];
        let items = projection(&legacy_rows, &[]);
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].stable_key(), "tool-group:old");
        assert_eq!(items[2].stable_key(), "tool-group:new");
        let rows = vec![
            row_event(compaction_event("s1", "reused", Some("started"), 10)),
            row_event(compaction_event("e1", "reused", Some("completed"), 20)),
            row_event(compaction_event("s2", "reused", Some("started"), 30)),
        ];
        assert_eq!(projection(&rows, &[]).len(), 2, "a completed legacy lifecycle cannot swallow a new start");
    }

    #[::core::prelude::v1::test]
    fn compaction_scoped_lifecycle_pairs_across_person_or_peer_steering() {
        for sender in [Value::Null, json!("peer")] {
            let rows = vec![
                row_event(scoped_compaction("start", "started", 10, "turn-1")),
                TimelineRow::Message(Rc::new(json!({"id":"steering","taskId":"task-1","role":"user","senderAgentId":sender,"createdAt":15}))),
                row_event(scoped_compaction("end", "completed", 20, "turn-1")),
            ];
            let items = projection(&rows, &[]);
            assert_eq!(items.len(), 2);
            let ConversationItem::ToolGroup(values) = &items[0] else { panic!("compaction group"); };
            assert_eq!(values.len(), 2);
            let events: Vec<_> = values.iter().map(|event| (**event).clone()).collect();
            assert_eq!(context_compaction(&events, false).unwrap().duration_ms, Some(10));
        }
    }

    #[::core::prelude::v1::test]
    fn native_subagent_envelopes_share_one_group_entry() {
        let rows = vec![
            row_event(event(
                "a",
                "tool",
                "task-1",
                10,
                "Spawn",
                "{\"activity\":{\"type\":\"collabAgentToolCall\",\"id\":\"x\",\"tool\":\"spawnAgent\",\"status\":\"inProgress\"}}",
            )),
            row_event(event(
                "b",
                "tool",
                "task-1",
                20,
                "Spawn",
                "{\"activity\":{\"type\":\"collabAgentToolCall\",\"id\":\"x\",\"tool\":\"spawnAgent\",\"status\":\"completed\"}}",
            )),
        ];
        let items = projection(&rows, &[]);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].stable_key(), "tool-group:a");
    }

    #[::core::prelude::v1::test]
    fn is_request_boundary_matches_known_markers() {
        let items = vec![
            ConversationItem::Activity(event(
                "a",
                "status",
                "task-1",
                10,
                CANCELLATION_EVENT_TITLE,
                "",
            ).into()),
            ConversationItem::Message(
                json!({"id":"m1","createdAt":20,"role":"system","taskId":"task-1","text":"Context Cleared"}).into(),
            ),
        ];
        assert!(is_request_boundary(&items[0]));
        assert!(is_request_boundary(&items[1]));
    }

    #[::core::prelude::v1::test]
    fn has_active_output_returns_true_for_trailing_reasoning_or_tool() {
        let items = vec![
            ConversationItem::Message(
                json!({"id":"m1","createdAt":10,"role":"user","taskId":"task-1","text":"hi"})
                    .into(),
            ),
            ConversationItem::ReasoningGroup(vec![
                event(
                    "a",
                    "reasoning",
                    "task-1",
                    15,
                    "Thinking",
                    "{\"summary\":\"x\"}",
                )
                .into(),
            ]),
        ];
        assert!(has_active_output(&items));
        let items = vec![ConversationItem::Message(
            json!({"id":"m1","createdAt":10,"role":"user","taskId":"task-1","text":"hi"}).into(),
        )];
        assert!(!has_active_output(&items));
    }

    #[::core::prelude::v1::test]
    fn tool_family_matches_across_spaces_and_underscores() {
        let a = tool_family(&tool_identity(&event(
            "a",
            "tool",
            "task-1",
            10,
            "Run command",
            "{\"kind\":\"command_execution\"}",
        )));
        let b = tool_family(&tool_identity(&event(
            "b",
            "tool",
            "task-1",
            20,
            "Run command",
            "{\"kind\":\"command_execution\"}",
        )));
        assert_eq!(a, b);
        assert_eq!(a, "command_execution");
    }

    #[::core::prelude::v1::test]
    fn connector_family_groups_mcp_namespaces() {
        let identity = tool_identity(&event(
            "c",
            "tool",
            "task-1",
            30,
            "Tool",
            "{\"tool\":\"mcp__gbrain__search_pages\"}",
        ));
        let family = tool_family(&identity);
        assert_eq!(family, "gbrain");
    }
}
