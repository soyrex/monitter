//! Minimal resident ACP v1 stdio transport.
//!
//! This module intentionally owns one child per Monitter task.  It never uses
//! a shell, never retries an uncertain prompt, and performs one bounded,
//! prompt-free session reload after an unexpected transport EOF.
//! Protocol details are deliberately conservative: unsupported server requests
//! receive a JSON-RPC error instead of being mistaken for an approval.

use crate::{
    ApprovalDecision, CreateApprovalRequest, Service, acp_protocol,
    model::AssistantResponseMetadata,
    runner::{self, Parsed, RunControl},
};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    io::{BufReader, Write},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

const INITIALIZE_ID: i64 = 1;
const SESSION_ID: i64 = 2;
const FIRST_PROMPT_ID: i64 = 3;
const MODEL_CONFIG_ID: i64 = 4;
const PERMISSION_CONFIG_ID: i64 = 5;
// OpenCode may initialize its configured providers/plugins before answering
// ACP initialize (the installed CLI takes >20 seconds on a cold launch).
// Keep a fixed deadline, without mistaking healthy cold startup for failure.
pub(crate) const INITIALIZE_TIMEOUT: Duration = Duration::from_secs(60);
pub(crate) const SESSION_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_PERMISSION_HISTORY: usize = 4096;
const MAX_PENDING_PERMISSIONS: usize = 16;
const MAX_TOOL_ACTIVITY_ITEMS: usize = 1024;
const MAX_TOOL_ACTIVITY_BYTES: usize = 2 * 1024 * 1024;
const MAX_TOOL_EVENT_BYTES: usize = 512 * 1024;
const MAX_TOOL_FIELD_BYTES: usize = 64 * 1024;
const MAX_ROUTER_TRACE_BYTES: usize = 64 * 1024;
const MAX_RESPONSE_METADATA_MODEL_BYTES: usize = 256;
const MAX_RESPONSE_METADATA_RATIONALE_BYTES: usize = 4096;
const MAX_RESPONSE_METADATA_REQUESTED_MODEL_BYTES: usize = 256;
const MAX_RESPONSE_METADATA_REQUESTED_EFFORT_BYTES: usize = 64;
const MAX_RESPONSE_METADATA_ERROR_BYTES: usize = 1024;
const MAX_RESPONSE_METADATA_TOKENS: u64 = 1_000_000_000;
const TOOL_DETAIL_TRUNCATED: &str = "[ACP tool detail truncated]";

fn normalized_stop_outcome(stop_reason: &str) -> (&'static str, Option<String>) {
    match stop_reason {
        // `stop` is the successful finish reason used by OpenAI-compatible
        // providers. ACP agents should translate it to `end_turn`, but accept
        // it here as a compatibility boundary so a completed reply is not
        // persisted as a transport failure.
        "end_turn" | "completed" | "stop" => ("completed", None),
        "cancelled" => ("interrupted", None),
        "refusal" => ("error", Some("ACP agent refused this prompt.".into())),
        "max_tokens" => (
            "error",
            Some("ACP agent reached its token limit before completing this prompt.".into()),
        ),
        other => ("error", Some(format!("ACP prompt stopped with {other}."))),
    }
}

struct PermissionSlot(Arc<AtomicUsize>);
impl Drop for PermissionSlot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_updates_keep_identity_and_merge_lifecycle_fields() {
        let mut updates = HashMap::new();
        let first = json!({
            "sessionId": "claude-session",
            "update": {
                "sessionUpdate": "tool_call",
                "toolCallId": "call-1",
                "title": "Read package manifest",
                "kind": "read",
                "status": "pending",
                "rawInput": {"path": "package.json"}
            }
        });
        let (_, first) = normalized_activity(&first, "tool_call", &mut updates)
            .unwrap()
            .expect("tool update");
        assert_eq!(
            serde_json::from_str::<Value>(&first).unwrap()["update"]["title"],
            "Read package manifest"
        );

        let final_update = json!({
            "sessionId": "claude-session",
            "update": {
                "sessionUpdate": "tool_call_update",
                "toolCallId": "call-1",
                "status": "failed",
                "rawOutput": {"error": "permission denied"}
            }
        });
        let (title, detail) = normalized_activity(&final_update, "tool_call_update", &mut updates)
            .unwrap()
            .expect("tool update");
        let detail: Value = serde_json::from_str(&detail).unwrap();
        assert_eq!(title, "Read package manifest");
        assert_eq!(detail["sessionId"], "claude-session");
        assert_eq!(detail["update"]["toolCallId"], "call-1");
        assert_eq!(detail["update"]["status"], "failed");
        assert_eq!(detail["update"]["rawInput"]["path"], "package.json");
        assert_eq!(detail["update"]["rawOutput"]["error"], "permission denied");
    }

    #[test]
    fn plan_is_a_distinct_normalized_activity() {
        let mut updates = HashMap::new();
        let (title, detail) = normalized_activity(
            &json!({"sessionId":"claude-session", "update":{"content":"Check the manifest"}}),
            "plan",
            &mut updates,
        )
        .unwrap()
        .expect("plan");
        let detail: Value = serde_json::from_str(&detail).unwrap();
        assert_eq!(title, "ACP plan");
        assert_eq!(detail["update"]["sessionUpdate"], "plan");
        assert_eq!(detail["update"]["content"], "Check the manifest");
    }

    #[test]
    fn many_tool_outputs_keep_the_turn_running() {
        let mut updates = HashMap::new();
        for index in 0..64 {
            let params = json!({
                "sessionId": "long-turn",
                "update": {
                    "toolCallId": format!("call-{index}"),
                    "title": format!("Read file {index}"),
                    "kind": "read",
                    "status": "completed",
                    "rawOutput": {"text": "x".repeat(64 * 1024)}
                }
            });
            let (_, detail) = normalized_activity(&params, "tool_call", &mut updates)
                .unwrap()
                .expect("tool update");
            let detail: Value = serde_json::from_str(&detail).unwrap();
            assert_eq!(detail["update"]["status"], "completed");
            assert_eq!(detail["update"]["title"], format!("Read file {index}"));
        }
        assert_eq!(updates.len(), 64);
        assert!(retained_tool_activity_bytes(&updates) <= MAX_TOOL_ACTIVITY_BYTES);
    }

    #[test]
    fn oversized_tool_output_is_truncated_without_hiding_failure() {
        let mut updates = HashMap::new();
        let params = json!({
            "sessionId": "large-output",
            "update": {
                "toolCallId": "call-1",
                "title": "Run command",
                "status": "failed",
                "rawOutput": {"stderr": "x".repeat(MAX_TOOL_EVENT_BYTES)}
            }
        });
        let (title, detail) = normalized_activity(&params, "tool_call_update", &mut updates)
            .unwrap()
            .expect("tool update");
        assert_eq!(title, "Run command");
        assert!(detail.len() <= MAX_TOOL_EVENT_BYTES);
        let detail: Value = serde_json::from_str(&detail).unwrap();
        assert_eq!(detail["update"]["status"], "failed");
        assert!(
            detail["update"]["rawOutput"]
                .as_str()
                .unwrap()
                .contains(TOOL_DETAIL_TRUNCATED)
        );
    }

    #[test]
    fn parallel_tool_output_pressure_compacts_retained_state() {
        let mut updates = HashMap::new();
        for index in 0..64 {
            let params = json!({
                "sessionId": "parallel-turn",
                "update": {
                    "toolCallId": format!("call-{index}"),
                    "title": format!("Search {index}"),
                    "status": "in_progress",
                    "content": "x".repeat(48 * 1024)
                }
            });
            normalized_activity(&params, "tool_call", &mut updates)
                .unwrap()
                .expect("tool update");
        }
        assert!(retained_tool_activity_bytes(&updates) <= MAX_TOOL_ACTIVITY_BYTES);
        assert_eq!(updates.len(), 64);
        let final_update = json!({
            "sessionId": "parallel-turn",
            "update": {"toolCallId": "call-0", "status": "failed", "error": "permission denied"}
        });
        let (title, detail) = normalized_activity(&final_update, "tool_call_update", &mut updates)
            .unwrap()
            .expect("tool update");
        assert_eq!(title, "Search 0");
        let detail: Value = serde_json::from_str(&detail).unwrap();
        assert_eq!(detail["update"]["status"], "failed");
        assert_eq!(detail["update"]["error"], "permission denied");
    }

    #[test]
    fn router_trace_reports_requested_and_actual_route() {
        let params = json!({
            "sessionId": "mona-session",
            "update": {
                "sessionUpdate": "router_trace",
                "trace": {
                    "traceId": "trace-1",
                    "applied": true,
                    "requestedModel": "gpt-6-astra",
                    "requestedEffort": "max",
                    "newModel": "gpt-6-astra",
                    "newEffort": "xhigh",
                    "rationale": "frontier task",
                    "prompt": "must not be persisted",
                    "apiKey": "must not be persisted"
                }
            }
        });
        let (title, detail) = normalized_router_trace(&params).unwrap().unwrap();
        assert_eq!(title, "Jev route applied · gpt-6-astra · xhigh");
        let detail: Value = serde_json::from_str(&detail).unwrap();
        assert_eq!(detail["trace"]["requestedEffort"], "max");
        assert_eq!(detail["trace"]["newEffort"], "xhigh");
        assert!(detail["trace"].get("prompt").is_none());
        assert!(detail["trace"].get("apiKey").is_none());
    }

    #[test]
    fn router_trace_rejects_oversized_or_unstructured_payloads() {
        assert!(
            normalized_router_trace(&json!({
                "sessionId": "mona-session",
                "update": {"trace": "not-an-object"}
            }))
            .is_err()
        );
        assert!(normalized_router_trace(&json!({
            "sessionId": "mona-session",
            "update": {"trace": {"applied": false, "rationale": "x".repeat(MAX_ROUTER_TRACE_BYTES)}}
        }))
        .is_err());
    }

    #[test]
    fn mona_prompt_result_projects_only_bounded_completion_metadata() {
        let metadata = normalized_mona_response_metadata(&json!({
            "stopReason": "end_turn",
            "model": "MiniMax-M2.7",
            "usage": {"inputTokens": 120, "outputTokens": 45},
            "routing": {
                "rationale": "The task benefits from a longer reasoning budget.",
                "requestedModel": "gpt-6-astra",
                "requestedEffort": "xhigh",
                "confidence": 0.92,
                "applied": true,
                "applicationError": null,
                "rawPrompt": "must not be retained",
                "providerCredential": "must not be retained"
            }
        }))
        .unwrap()
        .expect("Mona metadata");
        assert_eq!(metadata.model, "MiniMax-M2.7");
        assert_eq!(metadata.input_tokens, 120);
        assert_eq!(metadata.output_tokens, 45);
        assert_eq!(metadata.jev_rationale.as_deref(), Some("The task benefits from a longer reasoning budget."));
        assert_eq!(metadata.requested_model.as_deref(), Some("gpt-6-astra"));
        assert_eq!(metadata.confidence, Some(0.92));
        assert_eq!(metadata.route_applied, Some(true));
        let serialized = serde_json::to_value(metadata).unwrap();
        assert!(serialized.get("rawPrompt").is_none());
        assert!(serialized.get("providerCredential").is_none());
    }

    #[test]
    fn mona_prompt_result_rejects_partial_or_oversized_metadata() {
        assert!(normalized_mona_response_metadata(&json!({
            "model": "mona", "usage": {"inputTokens": 1}
        }))
        .is_err());
        assert!(normalized_mona_response_metadata(&json!({
            "model": "mona", "usage": {"inputTokens": 1, "outputTokens": 1},
            "routing": {"rationale": "x".repeat(MAX_RESPONSE_METADATA_RATIONALE_BYTES + 1)}
        }))
        .is_err());
        assert_eq!(normalized_mona_response_metadata(&json!({"stopReason": "end_turn"})).unwrap(), None);
    }

    #[test]
    fn provider_native_stop_is_a_successful_acp_completion() {
        assert_eq!(normalized_stop_outcome("stop"), ("completed", None));
        assert_eq!(normalized_stop_outcome("end_turn"), ("completed", None));
        assert_eq!(normalized_stop_outcome("cancelled"), ("interrupted", None));
        assert_eq!(
            normalized_stop_outcome("tool_use"),
            (
                "error",
                Some("ACP prompt stopped with tool_use.".to_string())
            )
        );
    }
}

struct OwnedRun {
    service: Arc<Service>,
    task_id: String,
    control: Arc<RunControl>,
}

impl Drop for OwnedRun {
    fn drop(&mut self) {
        // The collector owns a planned retirement through reaping and the
        // pointer-checked registry release. Do not turn that deliberate EOF
        // into ordinary cancellation or let this stale reader release a
        // successor that is waking the same task.
        if self.control.is_planned_retirement() {
            return;
        }
        self.control.terminate_owned();
        self.service
            .release_app_server_run(&self.task_id, &self.control);
    }
}

pub(crate) fn start(
    service: Arc<Service>,
    task_id: String,
    prompt: String,
    control: Arc<RunControl>,
) {
    thread::spawn(move || run(service, task_id, Some(prompt), control));
}

/// Reconnect an owned ACP task after its stdio transport ended.  This starts
/// only the initialize + load/resume handshake: a user prompt is never copied
/// into this path because an accepted frame might already have reached ACP.
pub(crate) fn recover(service: Arc<Service>, task_id: String, control: Arc<RunControl>) {
    thread::spawn(move || run(service, task_id, None, control));
}

pub(crate) fn send_turn(
    control: &RunControl,
    prompt: &str,
    task: &crate::model::Task,
) -> Result<(), String> {
    control.send_acp_turn(prompt, task)
}

fn fail(
    service: &Arc<Service>,
    task_id: &str,
    control: &Arc<RunControl>,
    detail: impl Into<String>,
) {
    control.clear_acp_turn_reservation();
    let detail = detail.into();
    if !service.complete_app_server_turn(task_id, control, None, "error", Some(detail.clone())) {
        // Recovery starts from a completed/interrupted durable turn, so its
        // handshake failure cannot use the ordinary running-turn transition.
        // Preserve an actionable, redacted activity entry instead of silently
        // dropping the replacement owner.
        service.record(
            task_id,
            "error",
            "ACP connection could not be restored",
            detail,
        );
    }
}

/// A pipe closure is not evidence that a queued prompt was not received.  A
/// single replacement transport is therefore started without any prompt.  A
/// recovery start which itself fails goes through `fail` and is not retried:
/// this bounds one recovery episode and avoids an invisible restart loop.
fn recover_transport(
    service: &Arc<Service>,
    task_id: &str,
    control: &Arc<RunControl>,
    detail: impl Into<String>,
) {
    // The collector has already fenced this owner and is deliberately closing
    // its stdio. A planned retirement is neither a provider crash nor an
    // ambiguous user turn, so it must never start a replacement transport or
    // write a reconnect warning into the chat.
    if control.is_planned_retirement() {
        return;
    }
    let mut detail = detail.into();
    if let Some(exit) = control.acp_exit_diagnostic() {
        detail = format!("{detail} {exit}");
    }
    let active = control.has_app_server_turn_request();
    if active {
        // A prompt request may have been written before transport loss. Keep
        // the durable user message, interrupt partial output and approvals,
        // and make the uncertainty explicit; never replay its text.
        let _ = service.complete_app_server_turn(
            task_id,
            control,
            None,
            "interrupted",
            Some(format!(
                "{detail} Monitter is reconnecting the ACP session but did not replay this turn."
            )),
        );
    }
    control.clear_acp_turn_reservation();
    control.retire_acp_transport();
    match service.replace_acp_transport(task_id, control) {
        Ok(replacement) => {
            // Stop the old child after the registry CAS. Its owned Drop only
            // releases a matching Arc, so it cannot evict this replacement.
            control.cancel();
            recover(service.clone(), task_id.into(), replacement);
        }
        Err(error) => {
            // Idle recovery is quiet when it succeeds. Only an unavailable
            // saved-session recovery needs an actionable activity entry.
            let terminalized = if !active {
                service.complete_app_server_turn(
                    task_id,
                    control,
                    None,
                    "error",
                    Some(error.clone()),
                )
            } else {
                false
            };
            if active || (!terminalized && !control.is_cancelled()) {
                service.record(
                    task_id,
                    "error",
                    "ACP connection could not be restored",
                    error,
                );
            }
            control.cancel();
        }
    }
}

fn send(control: &RunControl, value: Value) -> Result<(), String> {
    control.send_control(&value.to_string())
}

fn content_text(value: &Value) -> Option<&str> {
    value
        .get("text")
        .and_then(Value::as_str)
        .or_else(|| value.get("content").and_then(Value::as_str))
        .or_else(|| value.pointer("/content/text").and_then(Value::as_str))
}

fn update_text(value: &Value) -> Option<&str> {
    content_text(value)
        .or_else(|| value.pointer("/params/update/text").and_then(Value::as_str))
        .or_else(|| {
            value
                .pointer("/params/update/content/text")
                .and_then(Value::as_str)
        })
}

fn session_id(value: &Value) -> Option<&str> {
    value
        .pointer("/result/sessionId")
        .and_then(Value::as_str)
        .or_else(|| value.pointer("/result/session/id").and_then(Value::as_str))
}

/// ACP sends a `tool_call` followed by partial `tool_call_update` patches.
/// Keep the provider's identifiers and merge those patches before recording
/// them, so every activity row remains useful on its own while the UI groups
/// the lifecycle by `sessionId` and `toolCallId`.
fn merge_activity_patch(current: &mut Value, patch: &Value) {
    if current.is_object() && patch.is_object() {
        let current = current.as_object_mut().expect("checked object");
        let patch = patch.as_object().expect("checked object");
        for (key, value) in patch {
            if value.is_null() {
                continue;
            }
            match current.get_mut(key) {
                Some(existing) => merge_activity_patch(existing, value),
                None => {
                    current.insert(key.clone(), value.clone());
                }
            }
        }
    } else if !patch.is_null() {
        *current = patch.clone();
    }
}

fn activity_title(update: &Value, fallback: &str) -> String {
    [
        update.get("title"),
        update.get("toolName"),
        update.pointer("/toolCall/title"),
        update.pointer("/toolCall/toolName"),
        update.get("kind"),
    ]
    .into_iter()
    .flatten()
    .filter_map(Value::as_str)
    .map(str::trim)
    .find(|value| !value.is_empty())
    .unwrap_or(fallback)
    .to_string()
}

fn activity_preview(value: &Value, max_bytes: usize) -> Value {
    let text = value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string());
    if text.len() <= max_bytes {
        return value.clone();
    }
    let mut end = max_bytes.saturating_sub(TOOL_DETAIL_TRUNCATED.len() + 1);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    Value::String(format!("{} {TOOL_DETAIL_TRUNCATED}", &text[..end]))
}

fn compact_tool_activity(update: &Value) -> Value {
    let mut compact = serde_json::Map::new();
    for field in [
        "sessionUpdate",
        "toolCallId",
        "title",
        "toolName",
        "kind",
        "status",
    ] {
        if let Some(value) = update.get(field) {
            let max_bytes = match field {
                "toolCallId" => 512,
                "title" | "toolName" => 256,
                _ => 64,
            };
            compact.insert(field.into(), activity_preview(value, max_bytes));
        }
    }
    Value::Object(compact)
}

fn retained_tool_activity_bytes(updates: &HashMap<(String, String), Value>) -> usize {
    updates.values().map(|value| value.to_string().len()).sum()
}

fn normalized_activity(
    params: &Value,
    kind: &str,
    tool_updates: &mut HashMap<(String, String), Value>,
) -> Result<Option<(String, String)>, String> {
    let Some(session) = params.get("sessionId").and_then(Value::as_str) else {
        return Ok(None);
    };
    let session = session.to_string();
    let Some(patch) = params.get("update") else {
        return Ok(None);
    };
    if kind == "plan" {
        let mut update = patch.clone();
        let Some(object) = update.as_object_mut() else {
            return Ok(None);
        };
        object.insert("sessionUpdate".into(), Value::String("plan".into()));
        let title = activity_title(&update, "ACP plan");
        return Ok(Some((
            title,
            json!({"sessionId": session, "update": update}).to_string(),
        )));
    }

    let tool_call_id = patch
        .get("toolCallId")
        .or_else(|| patch.pointer("/toolCall/id"))
        .and_then(Value::as_str)
        .filter(|id| !id.trim().is_empty());
    let Some(tool_call_id) = tool_call_id else {
        return Ok(None);
    };
    let tool_call_id = tool_call_id.to_string();
    let key = (session.clone(), tool_call_id.clone());
    if !tool_updates.contains_key(&key) && tool_updates.len() >= MAX_TOOL_ACTIVITY_ITEMS {
        return Err("Too many ACP tool calls in one turn.".into());
    }
    let (title, detail, terminal) = {
        let update = tool_updates.entry(key).or_insert_with(|| {
            json!({
                "sessionUpdate": "tool_call",
                "toolCallId": tool_call_id,
            })
        });
        merge_activity_patch(update, patch);
        let Some(object) = update.as_object_mut() else {
            return Ok(None);
        };
        object.insert("sessionUpdate".into(), Value::String("tool_call".into()));
        object.insert("toolCallId".into(), Value::String(tool_call_id));
        let mut detail = json!({"sessionId": session, "update": update}).to_string();
        if detail.len() > MAX_TOOL_EVENT_BYTES {
            let mut bounded = compact_tool_activity(update);
            for field in ["rawInput", "rawOutput", "content", "error"] {
                if let Some(value) = update.get(field) {
                    bounded[field] = activity_preview(value, MAX_TOOL_FIELD_BYTES);
                }
            }
            bounded["detailTruncated"] = Value::Bool(true);
            detail = json!({"sessionId": session, "update": bounded}).to_string();
            if detail.len() > MAX_TOOL_EVENT_BYTES {
                bounded = compact_tool_activity(update);
                bounded["content"] = Value::String(TOOL_DETAIL_TRUNCATED.into());
                detail = json!({"sessionId": session, "update": bounded}).to_string();
            }
            *update = bounded;
        }
        let title = activity_title(update, "ACP tool action");
        let terminal = matches!(
            update.get("status").and_then(Value::as_str),
            Some("completed" | "failed" | "cancelled" | "canceled")
        );
        if terminal {
            *update = compact_tool_activity(update);
        }
        (title, detail, terminal)
    };
    if !terminal && retained_tool_activity_bytes(tool_updates) > MAX_TOOL_ACTIVITY_BYTES {
        // Full updates have already been recorded as diagnostic events. Keep
        // only mergeable identity/status metadata in memory for later patches.
        for update in tool_updates.values_mut() {
            *update = compact_tool_activity(update);
        }
    }
    Ok(Some((title, detail)))
}

fn handle_permission_request(
    service: Arc<Service>,
    task_id: String,
    control: Arc<RunControl>,
    id: Value,
    params: Value,
    turn: String,
    pending: Arc<AtomicUsize>,
    auto_approve_once: bool,
) {
    // A permission request itself proves the provider has reached a tool
    // boundary, even if its options are invalid or later denied.
    control.mark_tool_work_observed();
    // Validate the opaque one-time options before showing a control. Invalid
    // or perpetual-only requests are cancelled rather than broadened.
    if acp_protocol::permission_outcome(&params, false).is_err() {
        let _ = send(
            &control,
            acp_protocol::response(id, acp_protocol::cancelled_permission()),
        );
        return;
    }
    if auto_approve_once {
        // An explicit YOLO selection still cannot manufacture broader
        // authority. Use only the agent's opaque one-time option, after the
        // same validation that protects the interactive approval path.
        let outcome = match acp_protocol::permission_outcome(&params, true) {
            Ok(outcome) => {
                service.record(
                    &task_id,
                    "tool",
                    "ACP YOLO permission",
                    "Monitter selected this agent's advertised one-time permission option because YOLO is enabled.".into(),
                );
                outcome
            }
            Err(_) => {
                service.record(
                    &task_id,
                    "error",
                    "ACP YOLO permission cancelled",
                    "This ACP permission request did not offer an allow_once option, so Monitter cancelled it.".into(),
                );
                acp_protocol::cancelled_permission()
            }
        };
        let _ = send(&control, acp_protocol::response(id, outcome));
        return;
    }
    if pending
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
            (count < MAX_PENDING_PERMISSIONS).then_some(count + 1)
        })
        .is_err()
    {
        let _ = send(
            &control,
            acp_protocol::error_response(id, -32000, "Too many pending ACP permission requests."),
        );
        return;
    }
    let slot = PermissionSlot(pending);
    control.acp_permission_wait_started();
    thread::spawn(move || {
        let _slot = slot;
        let tool = params
            .pointer("/toolCall/title")
            .and_then(Value::as_str)
            .or_else(|| params.pointer("/toolCall/toolName").and_then(Value::as_str))
            .or_else(|| params.get("title").and_then(Value::as_str))
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("ACP tool action")
            .to_string();
        let raw_input = params
            .pointer("/toolCall/rawInput")
            .or_else(|| params.pointer("/toolCall/input"))
            .filter(|value| !value.is_null())
            .cloned();
        let detail = params
            .pointer("/toolCall/rawInput")
            .or_else(|| params.pointer("/toolCall/input"))
            .map(Value::to_string)
            .filter(|detail| detail.len() <= 64 * 1024)
            .unwrap_or_else(|| "ACP supplied a one-time permission request.".into());
        // A title alone is never a remembered action. Preserve the full
        // provider context (including rawInput) for exact matching; the UI
        // detail remains independently bounded above.
        let action = raw_input.as_ref().map(|raw| serde_json::json!({
            "kind": params.pointer("/toolCall/kind"), "toolName": params.pointer("/toolCall/toolName"),
            "title": params.pointer("/toolCall/title"), "locations": params.pointer("/toolCall/locations"),
            "rawInput": raw,
        }));
        let request = service.create_approval_request(CreateApprovalRequest {
            task_id: task_id.clone(),
            provider: "acp".into(),
            run_id: format!("acp-permission:{}", id),
            summary: format!("Allow {tool}?"),
            tool,
            detail,
            risk: "unknown".into(),
            raw_input: action,
        });
        if let Ok(request) = &request {
            if let Ok(mut owners) = service.app_server_approvals.lock() {
                owners.insert(request.id.clone(), Arc::downgrade(&control));
            }
        }
        let outcome = match request.and_then(|request| {
            service.wait_for_approval(&request.id, || {
                !control.is_cancelled() && control.matches_app_server_turn(&turn)
            })
        }) {
            Ok(ApprovalDecision::ApproveOnce)
            | Ok(ApprovalDecision::ApproveSession)
            | Ok(ApprovalDecision::ApproveAlways) => {
                acp_protocol::permission_outcome(&params, true)
                    .unwrap_or_else(|_| acp_protocol::cancelled_permission())
            }
            Ok(ApprovalDecision::Deny) => acp_protocol::permission_outcome(&params, false)
                .unwrap_or_else(|_| acp_protocol::cancelled_permission()),
            Err(_) => acp_protocol::cancelled_permission(),
        };
        let _ = send(&control, acp_protocol::response(id, outcome));
        control.acp_permission_wait_finished();
    });
}

fn notification_kind(value: &Value) -> &str {
    value
        .pointer("/params/update/sessionUpdate")
        .and_then(Value::as_str)
        .or_else(|| value.pointer("/params/update/type").and_then(Value::as_str))
        .unwrap_or_default()
}

/// Validate and normalize Mona's negotiated `router_trace` extension into one
/// bounded diagnostic event. The event keeps both the requested and actual
/// configuration, but never contains a raw prompt or provider credential.
fn normalized_router_trace(params: &Value) -> Result<Option<(String, String)>, String> {
    let Some(session_id) = params.get("sessionId").and_then(Value::as_str) else {
        return Ok(None);
    };
    let trace = params.pointer("/update/trace").unwrap_or(&Value::Null);
    let Some(trace_object) = trace.as_object() else {
        return Err("ACP router_trace omitted its trace object.".into());
    };

    let applied = trace_object
        .get("applied")
        .and_then(Value::as_bool)
        .ok_or_else(|| "ACP router_trace omitted its applied flag.".to_string())?;
    let bounded_string = |key: &str, max: usize| -> Result<Option<String>, String> {
        let Some(value) = trace_object.get(key) else {
            return Ok(None);
        };
        let value = value
            .as_str()
            .ok_or_else(|| format!("ACP router_trace field {key} was not text."))?
            .trim();
        if value.len() > max {
            return Err(format!(
                "ACP router_trace field {key} exceeded its safety limit."
            ));
        }
        Ok((!value.is_empty()).then(|| value.to_string()))
    };
    let actual_model = bounded_string("newModel", 256)?;
    let actual_effort = bounded_string("newEffort", 64)?;
    let application_error = bounded_string("applicationError", 1024)?;

    // Store only the documented routing fields. Unknown provider data is not
    // copied into Monitter, so a malformed extension cannot smuggle a prompt,
    // token, or credential into the durable event log.
    let mut projected = serde_json::Map::new();
    projected.insert("applied".into(), Value::Bool(applied));
    for (key, max) in [
        ("traceId", 128),
        ("trigger", 64),
        ("rationale", 4096),
        ("oldModel", 256),
        ("newModel", 256),
        ("oldEffort", 64),
        ("newEffort", 64),
        ("promptFingerprint", 256),
        ("proposedTier", 64),
        ("proposedEffort", 64),
        ("requestedModel", 256),
        ("requestedEffort", 64),
        ("applicationError", 1024),
    ] {
        if let Some(value) = bounded_string(key, max)? {
            projected.insert(key.into(), Value::String(value));
        }
    }
    for key in ["confidence", "occurredAt"] {
        if let Some(value) = trace_object.get(key) {
            if !value.is_number() {
                return Err(format!("ACP router_trace field {key} was not numeric."));
            }
            projected.insert(key.into(), value.clone());
        }
    }
    let detail = json!({"sessionId": session_id, "trace": projected}).to_string();
    if detail.len() > MAX_ROUTER_TRACE_BYTES {
        return Err("ACP router_trace exceeded 64 KiB and was ignored.".into());
    }

    let mut parts = vec![if applied {
        "Jev route applied".to_string()
    } else if application_error.is_some() {
        "Jev route rolled back".to_string()
    } else {
        "Jev route kept current runtime".to_string()
    }];
    if let Some(model) = actual_model {
        parts.push(model);
    }
    if let Some(effort) = actual_effort {
        parts.push(effort);
    }
    Ok(Some((parts.join(" · "), detail)))
}

/// Project Mona's authoritative `session/prompt` result into the small,
/// transcript-safe completion record. A result without Mona's fields is a
/// normal generic ACP result; a partial or malformed Mona result is rejected
/// rather than persisting ambiguous usage or unbounded provider data.
fn normalized_mona_response_metadata(
    result: &Value,
) -> Result<Option<AssistantResponseMetadata>, String> {
    let has_mona_fields = result.get("model").is_some()
        || result.get("usage").is_some()
        || result.get("routing").is_some();
    if !has_mona_fields {
        return Ok(None);
    }
    let bounded_required = |key: &str, max: usize| -> Result<String, String> {
        let value = result
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("Mona prompt result omitted text {key}."))?;
        if value.len() > max {
            return Err(format!("Mona prompt result field {key} exceeded its safety limit."));
        }
        Ok(value.to_string())
    };
    let model = bounded_required("model", MAX_RESPONSE_METADATA_MODEL_BYTES)?;
    let usage = result
        .get("usage")
        .and_then(Value::as_object)
        .ok_or_else(|| "Mona prompt result omitted its usage object.".to_string())?;
    let tokens = |key: &str| -> Result<u64, String> {
        let value = usage
            .get(key)
            .and_then(Value::as_u64)
            .filter(|value| *value <= MAX_RESPONSE_METADATA_TOKENS)
            .ok_or_else(|| format!("Mona prompt result usage.{key} was not a supported token count."))?;
        Ok(value)
    };
    let input_tokens = tokens("inputTokens")?;
    let output_tokens = tokens("outputTokens")?;

    let routing = match result.get("routing") {
        None | Some(Value::Null) => None,
        Some(Value::Object(routing)) => Some(routing),
        Some(_) => return Err("Mona prompt result routing was not an object.".into()),
    };
    let routing_text = |key: &str, max: usize| -> Result<Option<String>, String> {
        let Some(routing) = routing else {
            return Ok(None);
        };
        let Some(value) = routing.get(key) else {
            return Ok(None);
        };
        if value.is_null() {
            return Ok(None);
        }
        let value = value
            .as_str()
            .ok_or_else(|| format!("Mona prompt result routing.{key} was not text."))?
            .trim();
        if value.len() > max {
            return Err(format!(
                "Mona prompt result routing.{key} exceeded its safety limit."
            ));
        }
        Ok((!value.is_empty()).then(|| value.to_string()))
    };
    let confidence = match routing.and_then(|routing| routing.get("confidence")) {
        None | Some(Value::Null) => None,
        Some(value) => {
            let confidence = value
                .as_f64()
                .filter(|value| value.is_finite() && (0.0..=1.0).contains(value))
                .ok_or_else(|| {
                    "Mona prompt result routing.confidence was outside 0..=1.".to_string()
                })?;
            Some(confidence)
        }
    };
    let route_applied = match routing.and_then(|routing| routing.get("applied")) {
        None | Some(Value::Null) => None,
        Some(Value::Bool(applied)) => Some(*applied),
        Some(_) => return Err("Mona prompt result routing.applied was not boolean.".into()),
    };

    Ok(Some(AssistantResponseMetadata {
        model,
        input_tokens,
        output_tokens,
        jev_rationale: routing_text("rationale", MAX_RESPONSE_METADATA_RATIONALE_BYTES)?,
        requested_model: routing_text("requestedModel", MAX_RESPONSE_METADATA_REQUESTED_MODEL_BYTES)?,
        requested_effort: routing_text("requestedEffort", MAX_RESPONSE_METADATA_REQUESTED_EFFORT_BYTES)?,
        confidence,
        route_applied,
        application_error: routing_text("applicationError", MAX_RESPONSE_METADATA_ERROR_BYTES)?,
    }))
}

fn flush_reasoning(
    service: &Arc<Service>,
    task_id: &str,
    control: &Arc<RunControl>,
    turn: &str,
    text: &mut String,
) -> Result<(), String> {
    if text.is_empty() {
        return Ok(());
    }
    service.app_server_event(
        task_id,
        control,
        Some(turn),
        Parsed {
            native_session_id: None,
            assistant: None,
            event: Some(("reasoning".into(), "Reasoning".into(), std::mem::take(text))),
            failed: false,
        },
    )
}

fn run(
    service: Arc<Service>,
    task_id: String,
    initial_prompt: Option<String>,
    control: Arc<RunControl>,
) {
    // A replacement may not itself start another replacement until it has
    // completed a subsequent real prompt. This prevents rapid EOF reload
    // loops while allowing an established chat to recover again later.
    let mut recovery_budget_exhausted = initial_prompt.is_none();
    let _owned = OwnedRun {
        service: service.clone(),
        task_id: task_id.clone(),
        control: control.clone(),
    };
    let (task, host) = match service.task_and_host(&task_id) {
        Ok(value) => value,
        Err(error) => {
            fail(&service, &task_id, &control, error);
            return;
        }
    };
    let Some(launch) = task.acp.as_ref() else {
        fail(
            &service,
            &task_id,
            &control,
            "ACP task has no immutable launch configuration.",
        );
        return;
    };
    let mut extensions = match service.runtime_extensions_for_agent(&task.agent_id) {
        Ok(extensions) => extensions,
        Err(error) => {
            fail(&service, &task_id, &control, error);
            return;
        }
    };
    if let Err(error) = extensions.validate_for("acp", &host.kind) {
        fail(&service, &task_id, &control, error);
        return;
    }
    control.set_mcp_fingerprint(extensions.mcp_fingerprint());
    let initial_prompt = initial_prompt.map(|prompt| extensions.prompt(&prompt));
    let grant = match if initial_prompt.is_some() {
        service.collaboration_grant(&task_id)
    } else {
        service.existing_collaboration_grant(&task_id)
    } {
        Ok(grant) => grant,
        Err(error) => {
            fail(&service, &task_id, &control, error);
            return;
        }
    };
    let mut remote_collaboration = match (host.kind.as_str(), grant.as_ref()) {
        ("ssh", Some(grant)) => {
            match runner::prepare_remote_collaboration(&host, &grant.endpoint, &control) {
                Ok(remote) => Some(remote),
                Err(error) => {
                    fail(&service, &task_id, &control, error);
                    return;
                }
            }
        }
        _ => None,
    };
    // SSH's reverse forward allocates a remote loopback port. The ACP agent
    // must receive that endpoint, never the desktop-only broker address.
    let session_grant = grant.as_ref().map(|grant| {
        let mut grant = grant.clone();
        if let Some(remote) = remote_collaboration.as_ref() {
            grant.endpoint = remote.endpoint.clone();
        }
        grant
    });
    let mut mcp_servers = crate::acp_collaboration::mcp_servers(session_grant.as_ref());
    let managed_mcp = match extensions.acp_servers() {
        Ok(Value::Array(servers)) => servers,
        Ok(_) => Vec::new(),
        Err(error) => {
            fail(&service, &task_id, &control, error);
            return;
        }
    };
    if let Some(target) = mcp_servers.as_array_mut() {
        target.extend(managed_mcp);
    }
    let mut command = match crate::acp_transport::command(&host, launch, &task.cwd) {
        Ok(command) => command,
        Err(error) => {
            if let Some(remote) = remote_collaboration.take() {
                runner::abort_remote_collaboration(remote);
            }
            fail(&service, &task_id, &control, error);
            return;
        }
    };
    if let Err(error) =
        service.apply_environment_secrets_to_local_user_command(&task_id, &host, &mut command)
    {
        if let Some(remote) = remote_collaboration.take() {
            runner::abort_remote_collaboration(remote);
        }
        fail(&service, &task_id, &control, error);
        return;
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            if let Some(remote) = remote_collaboration.take() {
                runner::abort_remote_collaboration(remote);
            }
            fail(
                &service,
                &task_id,
                &control,
                format!("Could not start ACP agent: {error}"),
            );
            return;
        }
    };
    let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        runner::terminate_bounded(&mut child);
        if let Some(remote) = remote_collaboration.take() {
            runner::abort_remote_collaboration(remote);
        }
        fail(
            &service,
            &task_id,
            &control,
            "Could not open ACP agent stdio.",
        );
        return;
    };
    let stderr = child.stderr.take();
    let (control_tx, control_rx) = mpsc::sync_channel(64);
    if let Err((mut child, _)) = control.install(child, None) {
        runner::terminate_bounded(&mut child);
        if let Some(remote) = remote_collaboration.take() {
            runner::abort_remote_collaboration(remote);
        }
        let _ = service.complete_app_server_turn(&task_id, &control, None, "interrupted", None);
        return;
    }
    if let Some(remote) = remote_collaboration.take() {
        runner::attach_remote_collaboration(remote, &control);
    }
    control.mark_acp_transport();
    control.set_acp_control(control_tx);
    thread::spawn(move || {
        let mut stdin = stdin;
        while let Ok(control_frame) = control_rx.recv() {
            let result = stdin
                .write_all(control_frame.frame.as_bytes())
                .and_then(|_| stdin.write_all(b"\n"))
                .and_then(|_| stdin.flush())
                .map_err(|error| format!("Could not flush ACP control frame: {error}"));
            if let Some(flushed) = control_frame.flushed {
                let _ = flushed.send(result.as_ref().map(|_| ()).map_err(Clone::clone));
            }
            if result.is_err() {
                break;
            }
        }
    });
    control.mark_resident();
    if let Some(stderr) = stderr {
        let service = service.clone();
        let task = task_id.clone();
        let control = control.clone();
        thread::spawn(move || {
            let mut emitted = false;
            let mut reader = BufReader::new(stderr);
            while let Ok(Some(line)) = acp_protocol::read_diagnostic_line(&mut reader) {
                if !emitted {
                    if let Some(detail) = runner::provider_stderr_diagnostic(&line) {
                        emitted = true;
                        let _ = service.app_server_event(
                            &task,
                            &control,
                            None,
                            Parsed {
                                native_session_id: None,
                                assistant: None,
                                event: Some(("error".into(), "ACP diagnostic".into(), detail)),
                                failed: false,
                            },
                        );
                    }
                }
            }
        });
    }
    let (frames_tx, frames_rx) = mpsc::sync_channel(64);
    thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            match acp_protocol::read_frame(&mut reader) {
                Ok(Some(frame)) => {
                    if frames_tx.send(Ok(frame)).is_err() {
                        break;
                    }
                }
                Ok(None) => break,
                Err(error) => {
                    let _ = frames_tx.send(Err(error));
                    break;
                }
            }
        }
    });
    let mut phase_deadline = Instant::now() + INITIALIZE_TIMEOUT;
    let mut phase = if host.kind == "ssh" {
        "bootstrap"
    } else {
        "initialize"
    };
    if host.kind != "ssh" {
        if let Err(error) = send(
            &control,
            acp_protocol::request(
                json!(INITIALIZE_ID),
                "initialize",
                acp_protocol::initialize_params(),
            ),
        ) {
            fail(&service, &task_id, &control, error);
            return;
        }
    }
    let mut turn = String::new();
    let mut messages = Vec::<(String, String)>::new();
    let mut dirty_messages = HashSet::<String>::new();
    let mut last_message_flush = Instant::now();
    let mut pending_reasoning = String::new();
    let mut reasoning_bytes = 0usize;
    let mut turn_images = 0usize;
    let mut tool_updates = HashMap::<(String, String), Value>::new();
    // ACP permits message chunks without an item identifier. Those chunks are
    // one assistant item until a tool/plan boundary, after which they must not
    // be appended to the narration that preceded the activity.
    let mut anonymous_message_item = 0usize;
    let mut last_reasoning_flush = Instant::now();
    // Populated from `subagent_spawned` announcements (see `acp_protocol::
    // initialize_params`'s `subagents` capability). Nested tool/message
    // notifications tagged with one of these session IDs, rather than the
    // root thread, belong to that subagent's own inline transcript.
    let mut known_subagents = HashSet::<String>::new();
    let mut resolved_cwd = task.cwd.clone();
    let mut seen_permission_ids = HashSet::<String>::new();
    let pending_permissions = Arc::new(AtomicUsize::new(0));
    let mut yolo_uses_one_time_permissions = false;
    loop {
        // Retirement only wins after the lifecycle guard proved there is no
        // turn, configuration request, approval, or owned background work.
        // Return before ordinary cancellation/EOF paths can repair it.
        if control.is_planned_retirement() {
            return;
        }
        if control.is_cancelled() {
            // Approval waiters must first write their explicit cancelled
            // outcomes. Only then cancel the provider session, and wait until
            // its writer has actually flushed that frame before teardown.
            let _ = control.wait_for_acp_permission_waits(Duration::from_secs(1));
            if let Some(session) = control.current_app_server_thread() {
                let frame =
                    acp_protocol::notification("session/cancel", json!({"sessionId": session}));
                let _ =
                    control.send_acp_control_flushed(&frame.to_string(), Duration::from_secs(1));
            }
            let _ = service.complete_app_server_turn(&task_id, &control, None, "interrupted", None);
            return;
        }
        if !dirty_messages.is_empty() && last_message_flush.elapsed() >= Duration::from_millis(100)
        {
            for (item, text) in &messages {
                if dirty_messages.contains(item) {
                    if let Err(error) = service
                        .app_server_message(&task_id, &control, &turn, item, text, None, false, None)
                    {
                        fail(&service, &task_id, &control, error);
                        return;
                    }
                }
            }
            dirty_messages.clear();
            last_message_flush = Instant::now();
        }
        if last_reasoning_flush.elapsed() >= Duration::from_millis(100) {
            if let Err(error) =
                flush_reasoning(&service, &task_id, &control, &turn, &mut pending_reasoning)
            {
                fail(&service, &task_id, &control, error);
                return;
            }
            last_reasoning_flush = Instant::now();
        }
        if phase != "idle" && Instant::now() >= phase_deadline {
            fail(
                &service,
                &task_id,
                &control,
                format!("ACP {phase} request timed out."),
            );
            return;
        }
        let wait = if phase == "idle" {
            Duration::from_millis(100)
        } else {
            phase_deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(100))
        };
        let received = frames_rx.recv_timeout(wait);
        let Some(_event) = control.begin_event_processing() else {
            return;
        };
        let value = match received {
            Ok(Ok(value)) => value,
            Ok(Err(error)) => {
                if control.is_planned_retirement() {
                    return;
                }
                if !recovery_budget_exhausted {
                    recover_transport(&service, &task_id, &control, error);
                } else {
                    fail(&service, &task_id, &control, error);
                }
                return;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                if control.is_planned_retirement() {
                    return;
                }
                if !recovery_budget_exhausted {
                    recover_transport(&service, &task_id, &control, "ACP output closed.");
                } else {
                    fail(&service, &task_id, &control, "ACP recovery output closed.");
                }
                return;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
        };
        if value.get("method").is_some() && value.get("id").is_some() {
            let method = value
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if matches!(
                method,
                "session/request_permission" | "session/requestPermission"
            ) {
                let params = value.get("params").cloned().unwrap_or_else(|| json!({}));
                let session_ok = params
                    .get("sessionId")
                    .and_then(Value::as_str)
                    .is_some_and(|id| control.matches_app_server_thread(id));
                let id_key = value["id"].to_string();
                if let Some(active_turn) = control.current_app_server_turn() {
                    turn = active_turn;
                }
                if seen_permission_ids.len() >= MAX_PERMISSION_HISTORY {
                    fail(
                        &service,
                        &task_id,
                        &control,
                        "ACP permission request history exceeded its safety limit; reconnect this chat.",
                    );
                    return;
                }
                if !session_ok
                    || !control.matches_app_server_turn(&turn)
                    || !seen_permission_ids.insert(id_key)
                {
                    let _ = send(
                        &control,
                        acp_protocol::error_response(
                            value["id"].clone(),
                            -32600,
                            "ACP permission request is stale or duplicated.",
                        ),
                    );
                } else {
                    handle_permission_request(
                        service.clone(),
                        task_id.clone(),
                        control.clone(),
                        value["id"].clone(),
                        params,
                        turn.clone(),
                        pending_permissions.clone(),
                        yolo_uses_one_time_permissions,
                    );
                }
            } else {
                let _ = send(
                    &control,
                    acp_protocol::error_response(
                        value["id"].clone(),
                        -32601,
                        "Monitter ACP client does not support this request.",
                    ),
                );
            }
            continue;
        }
        if phase == "bootstrap" {
            if value.get("method").and_then(Value::as_str)
                != Some(crate::acp_transport::REMOTE_READY_METHOD)
            {
                fail(
                    &service,
                    &task_id,
                    &control,
                    "ACP SSH transport did not provide its private ready notification.",
                );
                return;
            }
            let cwd = value
                .pointer("/params/cwd")
                .and_then(Value::as_str)
                .filter(|cwd| cwd.starts_with('/') && !cwd.contains('\0'));
            if cwd.is_none() {
                fail(
                    &service,
                    &task_id,
                    &control,
                    "ACP SSH transport returned an invalid remote working folder.",
                );
                return;
            }
            resolved_cwd = cwd.unwrap().into();
            if let Err(error) = send(
                &control,
                acp_protocol::request(
                    json!(INITIALIZE_ID),
                    "initialize",
                    acp_protocol::initialize_params(),
                ),
            ) {
                fail(&service, &task_id, &control, error);
                return;
            }
            phase = "initialize";
            phase_deadline = Instant::now() + INITIALIZE_TIMEOUT;
            continue;
        }
        // A namespaced ACP steering response is a non-terminal auxiliary
        // request. A rejected steer must return its durable message to FIFO,
        // not fail the active ACP turn.
        if value.pointer("/error/message").is_some() {
            if let Some(id) = value.get("id").and_then(Value::as_i64) {
                if let Some(steer) = control.take_acp_steer_request(id) {
                    let code = value.pointer("/error/code").and_then(Value::as_i64);
                    if code == Some(-32001)
                        && steer.attempts < runner::ACP_STEER_READY_RETRY_LIMIT
                        && control.current_app_server_turn().as_deref()
                            == Some(steer.expected_turn_id.as_str())
                    {
                        let retry_service = Arc::clone(&service);
                        let retry_control = Arc::clone(&control);
                        let retry_task_id = task_id.clone();
                        let queued_message_id = steer.queued_message_id.clone();
                        thread::spawn(move || {
                            thread::sleep(Duration::from_millis(200));
                            if let Err(error) = retry_control.retry_acp_steer(steer) {
                                retry_service.acp_steer_rejected(
                                    &retry_task_id,
                                    &retry_control,
                                    &queued_message_id,
                                    &error,
                                );
                            }
                        });
                        continue;
                    }
                    let detail = value
                        .pointer("/error/message")
                        .and_then(Value::as_str)
                        .unwrap_or("The ACP harness rejected the live steering request.");
                    service.acp_steer_rejected(
                        &task_id,
                        &control,
                        &steer.queued_message_id,
                        detail,
                    );
                    continue;
                }
            }
        }
        if let Some(error) = value.pointer("/error/message").and_then(Value::as_str) {
            fail(
                &service,
                &task_id,
                &control,
                format!("ACP {phase} failed: {error}"),
            );
            return;
        }
        if let Some(id) = value.get("id").and_then(Value::as_i64) {
            if id == INITIALIZE_ID {
                if phase != "initialize" {
                    fail(
                        &service,
                        &task_id,
                        &control,
                        "ACP initialize response was out of order.",
                    );
                    return;
                }
                let capabilities = match acp_protocol::Capabilities::from_initialize(
                    value.get("result").unwrap_or(&Value::Null),
                ) {
                    Ok(capabilities) => capabilities,
                    Err(error) => {
                        fail(&service, &task_id, &control, error);
                        return;
                    }
                };
                control.set_acp_extensions(value.get("result").unwrap_or(&Value::Null));
                // A cold-resume promise is valid only when this exact ACP
                // transport advertised a protocol recovery method. Persist
                // that negotiation so the collector never guesses from the
                // provider label alone.
                let recovery_method = capabilities.recovery_method();
                control.set_resume_supported(recovery_method.is_ok());
                if (grant.is_some() || extensions.has_http())
                    && value
                        .pointer("/result/agentCapabilities/mcpCapabilities/http")
                        .and_then(Value::as_bool)
                        != Some(true)
                {
                    fail(
                        &service,
                        &task_id,
                        &control,
                        "This ACP agent does not advertise HTTP MCP support; Monitter could not attach its collaboration server and did not silently ignore it.",
                    );
                    return;
                }
                let (method, params) = if let Some(native) = task.native_session_id.as_deref() {
                    match recovery_method {
                        Ok(method) => (
                            method,
                            json!({"cwd":resolved_cwd,"mcpServers":mcp_servers,"sessionId": native}),
                        ),
                        Err(error) => {
                            fail(&service, &task_id, &control, error);
                            return;
                        }
                    }
                } else {
                    (
                        "session/new",
                        json!({"cwd":resolved_cwd,"mcpServers":mcp_servers}),
                    )
                };
                if let Err(error) = send(
                    &control,
                    acp_protocol::request(json!(SESSION_ID), method, params),
                ) {
                    fail(&service, &task_id, &control, error);
                    return;
                }
                phase = method;
                phase_deadline = Instant::now() + SESSION_TIMEOUT;
                continue;
            }
            if id == SESSION_ID {
                if !matches!(phase, "session/new" | "session/load" | "session/resume") {
                    fail(
                        &service,
                        &task_id,
                        &control,
                        "ACP session response was out of order.",
                    );
                    return;
                }
                let session = session_id(&value)
                    .map(str::to_owned)
                    .or_else(|| task.native_session_id.clone());
                let Some(session) = session else {
                    fail(
                        &service,
                        &task_id,
                        &control,
                        "ACP session/new omitted sessionId.",
                    );
                    return;
                };
                let session_result = value.get("result").cloned().unwrap_or(Value::Null);
                control.set_acp_session_result(session_result.clone());
                // Publish readiness only once both the session identity and
                // its model/configuration advertisement are available to a
                // concurrently accepted explicit send.
                control.set_app_server_thread(session.clone());
                // Capture the process tree before any prompt can create
                // provider-owned background work. The collector only retires
                // an ACP owner when later descendants still match this safe
                // baseline.
                // Process inspection only governs whether GC may retire this
                // owner. It must never turn a usable ACP session into a
                // failed turn on hosts where process-tree sampling is
                // unavailable or transiently fails.
                let _ = control.capture_runtime_process_baseline();
                // A prompt-free transport repair has restored the saved
                // native session, but has not started a model turn. Return
                // the replacement to Idle so it remains reusable and can be
                // collected again; never manufacture or replay a prompt.
                if initial_prompt.is_none() {
                    service.mark_runtime_idle_if_current(&task_id, &control);
                }
                if initial_prompt.is_some() {
                    if let Err(error) = service.app_server_event(
                        &task_id,
                        &control,
                        None,
                        Parsed {
                            native_session_id: Some(session.clone()),
                            assistant: None,
                            event: None,
                            failed: false,
                        },
                    ) {
                        fail(&service, &task_id, &control, error);
                        return;
                    }
                }
                if initial_prompt.is_some()
                    && task.model_settings.as_ref().is_some_and(|settings| {
                        settings.fast_mode.is_some() || settings.reasoning_effort.is_some()
                    })
                {
                    fail(
                        &service,
                        &task_id,
                        &control,
                        "ACP has not advertised support for saved fast mode or reasoning effort settings.",
                    );
                    return;
                }
                let permission = match crate::acp_session_config::configured_permission_request(
                    &session_result,
                    &session,
                    &task.sandbox,
                ) {
                    Ok(request) => request,
                    Err(error) => {
                        fail(&service, &task_id, &control, error);
                        return;
                    }
                };
                if let crate::acp_session_config::PermissionConfiguration::UnavailableYolo(reason) =
                    &permission
                {
                    yolo_uses_one_time_permissions = true;
                    service.record(
                        &task_id,
                        "tool",
                        "ACP YOLO compatibility",
                        format!(
                            "{reason} This agent will receive only its advertised allow_once permission option for each request."
                        ),
                    );
                }
                if let crate::acp_session_config::PermissionConfiguration::Request(method, params) =
                    permission
                {
                    if let Err(error) = send(
                        &control,
                        acp_protocol::request(json!(PERMISSION_CONFIG_ID), method, params),
                    ) {
                        fail(&service, &task_id, &control, error);
                        return;
                    }
                    phase = "session/permissions";
                    phase_deadline = Instant::now() + SESSION_TIMEOUT;
                    continue;
                }
                if initial_prompt.is_some() {
                    let configured = match crate::acp_session_config::configured_model_request(
                        &session_result,
                        &session,
                        &task.model,
                    ) {
                        Ok(request) => request,
                        Err(error) => {
                            fail(&service, &task_id, &control, error);
                            return;
                        }
                    };
                    if let Some((method, params)) = configured {
                        if let Err(error) = send(
                            &control,
                            acp_protocol::request(json!(MODEL_CONFIG_ID), method, params),
                        ) {
                            fail(&service, &task_id, &control, error);
                            return;
                        }
                        phase = "session/model";
                        phase_deadline = Instant::now() + SESSION_TIMEOUT;
                        continue;
                    }
                }
                let Some(prompt) = initial_prompt.as_deref() else {
                    // Recovery loaded the exact saved session and has no
                    // prompt to replay.  Leave the durable interrupted or
                    // completed turn untouched; the next user send uses this
                    // new resident transport.
                    phase = "idle";
                    phase_deadline = Instant::now() + Duration::from_secs(24 * 60 * 60);
                    continue;
                };
                turn = format!("acp:{FIRST_PROMPT_ID}");
                control.set_app_server_turn(turn.clone());
                if let Err(error) = control.mark_app_server_turn_request(FIRST_PROMPT_ID) {
                    fail(&service, &task_id, &control, error);
                    return;
                }
                if let Err(error) = send(
                    &control,
                    acp_protocol::request(
                        json!(FIRST_PROMPT_ID),
                        "session/prompt",
                        json!({"sessionId":session,"prompt":[{"type":"text","text":prompt}]}),
                    ),
                ) {
                    fail(&service, &task_id, &control, error);
                    return;
                }
                phase = "prompt";
                // A prompt response is completion, not an acknowledgement;
                // valid model/tool activity must not time out an active turn.
                phase_deadline = Instant::now() + Duration::from_secs(24 * 60 * 60);
                continue;
            }
            if id == PERMISSION_CONFIG_ID {
                if phase != "session/permissions" {
                    fail(
                        &service,
                        &task_id,
                        &control,
                        "ACP permission response was out of order.",
                    );
                    return;
                }
                let session_result = value.get("result").cloned().unwrap_or(Value::Null);
                if let Some(options) = session_result.get("configOptions") {
                    if let Err(error) = control.update_acp_config_options(options) {
                        fail(&service, &task_id, &control, error);
                        return;
                    }
                }
                let session = control.current_app_server_thread().unwrap_or_default();
                if session.is_empty() {
                    fail(
                        &service,
                        &task_id,
                        &control,
                        "ACP session was lost before permission configuration.",
                    );
                    return;
                }
                if initial_prompt.is_some() {
                    let configured = match crate::acp_session_config::configured_model_request(
                        &session_result,
                        &session,
                        &task.model,
                    ) {
                        Ok(request) => request,
                        Err(error) => {
                            fail(&service, &task_id, &control, error);
                            return;
                        }
                    };
                    if let Some((method, params)) = configured {
                        if let Err(error) = send(
                            &control,
                            acp_protocol::request(json!(MODEL_CONFIG_ID), method, params),
                        ) {
                            fail(&service, &task_id, &control, error);
                            return;
                        }
                        phase = "session/model";
                        phase_deadline = Instant::now() + SESSION_TIMEOUT;
                        continue;
                    }
                }
                let Some(prompt) = initial_prompt.as_deref() else {
                    phase = "idle";
                    phase_deadline = Instant::now() + Duration::from_secs(24 * 60 * 60);
                    continue;
                };
                turn = format!("acp:{FIRST_PROMPT_ID}");
                control.set_app_server_turn(turn.clone());
                if let Err(error) = control.mark_app_server_turn_request(FIRST_PROMPT_ID).and_then(|_| send(&control, acp_protocol::request(json!(FIRST_PROMPT_ID), "session/prompt", json!({"sessionId":session,"prompt":[{"type":"text","text":prompt}]})))) { fail(&service,&task_id,&control,error); return; }
                phase = "prompt";
                phase_deadline = Instant::now() + Duration::from_secs(24 * 60 * 60);
                continue;
            }
            if id == MODEL_CONFIG_ID {
                if phase != "session/model" {
                    fail(
                        &service,
                        &task_id,
                        &control,
                        "ACP model response was out of order.",
                    );
                    return;
                }
                if let Some(options) = value.pointer("/result/configOptions") {
                    if let Err(error) = control.update_acp_config_options(options) {
                        fail(&service, &task_id, &control, error);
                        return;
                    }
                }
                let Some(prompt) = initial_prompt.as_deref() else {
                    phase = "idle";
                    phase_deadline = Instant::now() + Duration::from_secs(24 * 60 * 60);
                    continue;
                };
                turn = format!("acp:{FIRST_PROMPT_ID}");
                control.set_app_server_turn(turn.clone());
                // The model request only precedes prompt setup; session id is still held by control.
                let session = control.current_app_server_thread().unwrap_or_default();
                if session.is_empty() {
                    fail(
                        &service,
                        &task_id,
                        &control,
                        "ACP session was lost before model configuration.",
                    );
                    return;
                }
                if let Err(error) = control.mark_app_server_turn_request(FIRST_PROMPT_ID).and_then(|_| send(&control, acp_protocol::request(json!(FIRST_PROMPT_ID), "session/prompt", json!({"sessionId":session,"prompt":[{"type":"text","text":prompt}]})))) { fail(&service,&task_id,&control,error); return; }
                phase = "prompt";
                phase_deadline = Instant::now() + Duration::from_secs(24 * 60 * 60);
                continue;
            }
            if let Some(steer) = control.take_acp_steer_request(id) {
                let result = value.get("result").unwrap_or(&Value::Null);
                let returned_turn_matches = result.get("turnId").and_then(Value::as_str)
                    == Some(steer.expected_turn_id.as_str());
                let client_request_matches =
                    match result.get("clientRequestId").and_then(Value::as_str) {
                        Some(client_request_id) => client_request_id == steer.client_request_id,
                        None => steer.method == "mcode/session/steer",
                    };
                let accepted = result.get("mode").and_then(Value::as_str) == Some("steered")
                    && returned_turn_matches
                    && client_request_matches
                    && control.current_app_server_turn().as_deref()
                        == Some(steer.expected_turn_id.as_str());
                if accepted {
                    service.acp_steer_accepted(
                        &task_id,
                        &control,
                        &steer.expected_turn_id,
                        &steer.queued_message_id,
                    );
                } else {
                    service.acp_steer_rejected(
                        &task_id,
                        &control,
                        &steer.queued_message_id,
                        "The ACP harness returned an invalid or stale live-steering acknowledgement.",
                    );
                }
                continue;
            }
            let requested_turn = control.take_app_server_turn_request(id);
            if control.take_acp_config_request(id) {
                if let Some(options) = value.pointer("/result/configOptions") {
                    if let Err(error) = control.update_acp_config_options(options) {
                        fail(&service, &task_id, &control, error);
                        return;
                    }
                }
                let Some(prompt_frame) = control.take_acp_prompt_after_config() else {
                    fail(
                        &service,
                        &task_id,
                        &control,
                        "ACP model configuration response had no reserved prompt.",
                    );
                    return;
                };
                if let Err(error) = control.send_control(&prompt_frame) {
                    fail(&service, &task_id, &control, error);
                    return;
                }
                continue;
            }
            if requested_turn {
                turn = format!("acp:{id}");
                control.set_app_server_turn(turn.clone());
                if let Err(error) =
                    flush_reasoning(&service, &task_id, &control, &turn, &mut pending_reasoning)
                {
                    fail(&service, &task_id, &control, error);
                    return;
                }
                reasoning_bytes = 0;
                turn_images = 0;
                let stop_reason = value
                    .pointer("/result/stopReason")
                    .and_then(Value::as_str)
                    .unwrap_or("end_turn");
                let (status, error) = normalized_stop_outcome(stop_reason);
                let response_metadata = if status == "completed" {
                    match normalized_mona_response_metadata(
                        value.get("result").unwrap_or(&Value::Null),
                    ) {
                        Ok(metadata) => metadata,
                        Err(error) => {
                            service.record(
                                &task_id,
                                "error",
                                "Mona response metadata ignored",
                                error,
                            );
                            None
                        }
                    }
                } else {
                    None
                };
                let metadata_item = messages.last().map(|(item, _)| item.as_str());
                for (item, text) in &messages {
                    let metadata = (metadata_item == Some(item.as_str()))
                        .then(|| response_metadata.clone())
                        .flatten();
                    if let Err(error) = service
                        .app_server_message(
                            &task_id,
                            &control,
                            &turn,
                            item,
                            text,
                            None,
                            true,
                            metadata,
                        )
                    {
                        fail(&service, &task_id, &control, error);
                        return;
                    }
                }
                // Release only the reservation owned by this completed
                // response before publishing the durable completed state.
                // `complete_app_server_turn` can wake queued work; clearing
                // afterwards lets that later send reserve itself and then be
                // incorrectly cleared by this old response.
                control.clear_acp_turn_reservation();
                // A matching prompt response is the authoritative completion boundary.
                let _ = service.complete_app_server_turn(
                    &task_id,
                    &control,
                    Some(&turn),
                    status,
                    error,
                );
                if status == "completed" {
                    recovery_budget_exhausted = false;
                }
                messages.clear();
                dirty_messages.clear();
                tool_updates.clear();
                anonymous_message_item = 0;
                phase = "idle";
                phase_deadline = Instant::now() + Duration::from_secs(24 * 60 * 60);
                continue;
            }
            fail(
                &service,
                &task_id,
                &control,
                format!("ACP returned unknown response ID {id}."),
            );
            return;
        }
        if value.get("id").is_some() {
            fail(
                &service,
                &task_id,
                &control,
                "ACP returned a response ID that is not one of Monitter's numeric requests.",
            );
            return;
        }
        if value.get("method").and_then(Value::as_str) == Some("session/update") {
            if let Some(active_turn) = control.current_app_server_turn() {
                turn = active_turn;
            }
            let params = value.get("params").unwrap_or(&Value::Null);
            let Some(notification_session) = params.get("sessionId").and_then(Value::as_str) else {
                continue;
            };
            if control.matches_app_server_thread(notification_session)
                && notification_kind(&value) == "config_option_update"
            {
                if let Err(error) =
                    control.update_acp_config_options(&params["update"]["configOptions"])
                {
                    fail(&service, &task_id, &control, error);
                    return;
                }
                continue;
            }
            if control.matches_app_server_thread(notification_session)
                && notification_kind(&value) == "available_commands_update"
            {
                if let Err(error) =
                    control.replace_acp_slash_commands(&params["update"]["availableCommands"])
                {
                    fail(&service, &task_id, &control, error);
                    return;
                }
                continue;
            }
            let is_root_session = control.matches_app_server_thread(notification_session);
            let is_subagent_session =
                !is_root_session && known_subagents.contains(notification_session);
            if (!is_root_session && !is_subagent_session) || !control.matches_app_server_turn(&turn)
            {
                continue;
            }
            let kind = notification_kind(&value);
            if kind == "router_trace" {
                match normalized_router_trace(params) {
                    Ok(Some((title, detail))) => {
                        if let Err(error) = service.app_server_event(
                            &task_id,
                            &control,
                            (!turn.is_empty()).then_some(turn.as_str()),
                            Parsed {
                                native_session_id: None,
                                assistant: None,
                                event: Some(("status".into(), title, detail)),
                                failed: false,
                            },
                        ) {
                            fail(&service, &task_id, &control, error);
                            return;
                        }
                    }
                    Ok(None) => {}
                    Err(error) => {
                        service.record(&task_id, "error", "Mona routing update ignored", error)
                    }
                }
                continue;
            }
            if kind == "usage_update" {
                // Mona's extension reports per-turn token counts, not the
                // Codex-style context `used`/`size` pair. This notification
                // is useful live telemetry only; the prompt result below is
                // the authoritative completion record.
                let update = &params["update"];
                let usage = update.get("usage").unwrap_or(update);
                let input = usage.get("inputTokens").and_then(Value::as_i64);
                let output = usage.get("outputTokens").and_then(Value::as_i64);
                if !input.is_some_and(|value| value >= 0)
                    || !output.is_some_and(|value| value >= 0)
                {
                    service.record(
                        &task_id,
                        "error",
                        "Usage capture warning",
                        "Mona usage_update omitted numeric inputTokens or outputTokens; the run continued."
                            .into(),
                    );
                } else {
                    let detail = json!({
                        "providerTurnId": turn,
                        "classification": "delta",
                        "input": input,
                        "output": output,
                        "cost": usage.get("cost")
                    })
                    .to_string();
                    if let Err(error) = service.app_server_event(
                        &task_id,
                        &control,
                        (!turn.is_empty()).then_some(turn.as_str()),
                        Parsed {
                            native_session_id: None,
                            assistant: None,
                            event: Some(("usage".into(), "Usage updated".into(), detail)),
                            failed: false,
                        },
                    ) {
                        service.record(&task_id, "error", "Usage capture warning", error);
                    }
                }
                continue;
            }
            if matches!(kind, "subagent_spawned" | "subagent_state_update") {
                let update = &params["update"];
                if kind == "subagent_spawned" {
                    if let Some(child_id) = update.get("subagentSessionId").and_then(Value::as_str)
                    {
                        if !child_id.trim().is_empty() {
                            known_subagents.insert(child_id.to_string());
                        }
                    }
                }
                let title = if kind == "subagent_spawned" {
                    "Subagent spawned"
                } else {
                    "Subagent state update"
                };
                let result = service.app_server_event(
                    &task_id,
                    &control,
                    (!turn.is_empty()).then_some(turn.as_str()),
                    Parsed {
                        native_session_id: None,
                        assistant: None,
                        event: Some(("subagent".into(), title.into(), update.to_string())),
                        failed: false,
                    },
                );
                if let Err(error) = result {
                    fail(&service, &task_id, &control, error);
                    return;
                }
                if kind == "subagent_spawned" {
                    if let Some(child_id) = update.get("subagentSessionId").and_then(Value::as_str)
                    {
                        if let Some(task_text) = update.get("task").and_then(Value::as_str) {
                            let result = service.append_subagent_transcript_entry(
                                &task_id,
                                &control,
                                (!turn.is_empty()).then_some(turn.as_str()),
                                &format!("acp:{child_id}"),
                                "user",
                                task_text.to_string(),
                            );
                            if let Err(error) = result {
                                fail(&service, &task_id, &control, error);
                                return;
                            }
                        }
                    }
                }
                continue;
            }
            if is_subagent_session {
                // This subagent's own activity is captured into its inline
                // transcript rather than mixed into the parent task's
                // messages/events; the visor reads it back on demand.
                let subagent_id = format!("acp:{notification_session}");
                match kind {
                    "agent_message_chunk" | "agent_message" => {
                        if let Some(text) = update_text(&value) {
                            let result = service.append_subagent_transcript_entry(
                                &task_id,
                                &control,
                                (!turn.is_empty()).then_some(turn.as_str()),
                                &subagent_id,
                                "assistant",
                                text.to_string(),
                            );
                            if let Err(error) = result {
                                fail(&service, &task_id, &control, error);
                                return;
                            }
                        }
                    }
                    "agent_thought_chunk" => {
                        if let Some(text) = update_text(&value) {
                            let result = service.append_subagent_transcript_entry(
                                &task_id,
                                &control,
                                (!turn.is_empty()).then_some(turn.as_str()),
                                &subagent_id,
                                "reasoning",
                                text.to_string(),
                            );
                            if let Err(error) = result {
                                fail(&service, &task_id, &control, error);
                                return;
                            }
                        }
                    }
                    "tool_call" | "tool_call_update" | "plan" => {
                        control.mark_tool_work_observed();
                        let params = value.get("params").unwrap_or(&Value::Null);
                        let normalized = match normalized_activity(params, kind, &mut tool_updates)
                        {
                            Ok(value) => value,
                            Err(error) => {
                                fail(&service, &task_id, &control, error);
                                return;
                            }
                        };
                        if let Some((title, _detail)) = normalized {
                            let result = service.append_subagent_transcript_entry(
                                &task_id,
                                &control,
                                (!turn.is_empty()).then_some(turn.as_str()),
                                &subagent_id,
                                "activity",
                                title,
                            );
                            if let Err(error) = result {
                                fail(&service, &task_id, &control, error);
                                return;
                            }
                        }
                    }
                    _ => {}
                }
            } else if kind == "agent_message_done" {
                // An ACP turn can contain several completed assistant messages,
                // separated by tool work. Preserve each explicit message boundary
                // immediately: if a later provider request fails during a transient
                // network outage, only the still-streaming message is interrupted.
                if let Some((item, text)) = messages.last() {
                    if !text.is_empty() && !turn.is_empty() {
                        if let Err(error) = service.app_server_message(
                            &task_id, &control, &turn, item, text, None, true, None,
                        ) {
                            fail(&service, &task_id, &control, error);
                            return;
                        }
                        dirty_messages.remove(item);
                    }
                }
                // Mona emits this boundary without a provider item ID. Give the
                // next anonymous delta a distinct durable message.
                anonymous_message_item = anonymous_message_item.saturating_add(1).max(1);
            } else if matches!(kind, "agent_message_chunk" | "agent_message") {
                let content = &value["params"]["update"]["content"];
                let content_type = content["type"].as_str().unwrap_or("text");
                let placeholder = match content_type {
                    "image" => "",
                    "audio" => "\n[The ACP agent returned audio; playback is not supported yet.]\n",
                    "resource_link" => {
                        "\n[The ACP agent returned a resource link; automatic fetching is disabled.]\n"
                    }
                    "resource" => "\n[The ACP agent returned embedded resource content.]\n",
                    _ => "\n[The ACP agent returned unsupported content.]\n",
                };
                let delta = update_text(&value).unwrap_or(placeholder);
                {
                    let item = value
                        .pointer("/params/update/messageId")
                        .or_else(|| value.pointer("/params/update/id"))
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .unwrap_or_else(|| {
                            if anonymous_message_item == 0 {
                                anonymous_message_item = 1;
                            }
                            format!("anonymous-message-{anonymous_message_item}")
                        });
                    let is_new = !messages.iter().any(|(id, _)| id == &item);
                    if is_new {
                        if messages.len() >= 1024 {
                            fail(
                                &service,
                                &task_id,
                                &control,
                                "Too many ACP message items in one turn.",
                            );
                            return;
                        }
                        messages.push((item.clone(), String::new()));
                    }
                    let turn_bytes: usize = messages.iter().map(|(_, text)| text.len()).sum();
                    if turn_bytes.saturating_add(delta.len()) > 8 * 1024 * 1024 {
                        fail(
                            &service,
                            &task_id,
                            &control,
                            "ACP turn exceeds 8 MiB of text.",
                        );
                        return;
                    }
                    let Some((_, text)) = messages.iter_mut().find(|(id, _)| id == &item) else {
                        continue;
                    };
                    text.push_str(delta);
                    if text.len() > 2 * 1024 * 1024 {
                        fail(&service, &task_id, &control, "ACP reply exceeds 2 MiB.");
                        return;
                    }
                    if !turn.is_empty() {
                        if is_new || content_type == "image" {
                            if let Err(error) = service.app_server_message(
                                &task_id, &control, &turn, &item, text, None, false, None,
                            ) {
                                fail(&service, &task_id, &control, error);
                                return;
                            }
                            dirty_messages.remove(&item);
                        } else {
                            dirty_messages.insert(item.clone());
                        }
                        if content_type == "image" {
                            turn_images += 1;
                            if turn_images > 16 {
                                fail(
                                    &service,
                                    &task_id,
                                    &control,
                                    "Too many images in one ACP turn.",
                                );
                                return;
                            }
                            let result = content["data"]
                                .as_str()
                                .ok_or_else(|| "ACP image data is missing.".to_string())
                                .and_then(|data| {
                                    service
                                        .acp_message_image(&task_id, &control, &turn, &item, data)
                                });
                            if let Err(error) = result {
                                fail(&service, &task_id, &control, error);
                                return;
                            }
                        }
                    }
                }
            } else if kind == "agent_thought_chunk" {
                if let Some(delta) = update_text(&value) {
                    reasoning_bytes = reasoning_bytes.saturating_add(delta.len());
                    if reasoning_bytes > 2 * 1024 * 1024 {
                        fail(&service, &task_id, &control, "ACP reasoning exceeds 2 MiB.");
                        return;
                    }
                    pending_reasoning.push_str(delta);
                }
            } else if matches!(kind, "tool_call" | "tool_call_update" | "plan") {
                if matches!(kind, "tool_call" | "tool_call_update") {
                    control.mark_tool_work_observed();
                }
                anonymous_message_item = anonymous_message_item.saturating_add(1).max(1);
                let params = value.get("params").unwrap_or(&Value::Null);
                let normalized = match normalized_activity(params, kind, &mut tool_updates) {
                    Ok(value) => value,
                    Err(error) => {
                        fail(&service, &task_id, &control, error);
                        return;
                    }
                };
                let Some((title, detail)) = normalized else {
                    // A malformed activity update must remain visible rather
                    // than becoming a misleading generic tool row.
                    let detail = params.to_string();
                    let title = if kind == "plan" {
                        "ACP plan"
                    } else {
                        "ACP tool activity"
                    };
                    let result = service.app_server_event(
                        &task_id,
                        &control,
                        (!turn.is_empty()).then_some(turn.as_str()),
                        Parsed {
                            native_session_id: None,
                            assistant: None,
                            event: Some(("tool".into(), title.into(), detail)),
                            failed: false,
                        },
                    );
                    if let Err(error) = result {
                        fail(&service, &task_id, &control, error);
                        return;
                    }
                    continue;
                };
                let result = service.app_server_event(
                    &task_id,
                    &control,
                    (!turn.is_empty()).then_some(turn.as_str()),
                    Parsed {
                        native_session_id: None,
                        assistant: None,
                        event: Some(("tool".into(), title, detail)),
                        failed: false,
                    },
                );
                if let Err(error) = result {
                    fail(&service, &task_id, &control, error);
                    return;
                }
            }
        }
    }
}
