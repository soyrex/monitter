//! Minimal resident ACP v1 stdio transport.
//!
//! This module intentionally owns one child per Monitter task.  It never uses
//! a shell, never retries an uncertain prompt, and performs one bounded,
//! prompt-free session reload after an unexpected transport EOF.
//! Protocol details are deliberately conservative: unsupported server requests
//! receive a JSON-RPC error instead of being mistaken for an approval.

use crate::{
    acp_protocol,
    runner::{self, Parsed, RunControl},
    ApprovalDecision, CreateApprovalRequest, Service,
};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    io::{BufReader, Write},
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant},
};

const INITIALIZE_ID: i64 = 1;
const SESSION_ID: i64 = 2;
const FIRST_PROMPT_ID: i64 = 3;
const MODEL_CONFIG_ID: i64 = 4;
const PERMISSION_CONFIG_ID: i64 = 5;
const INITIALIZE_TIMEOUT: Duration = Duration::from_secs(20);
const SESSION_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_PERMISSION_HISTORY: usize = 4096;
const MAX_PENDING_PERMISSIONS: usize = 16;
const MAX_TOOL_ACTIVITY_ITEMS: usize = 1024;
const MAX_TOOL_ACTIVITY_BYTES: usize = 2 * 1024 * 1024;

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
        assert_eq!(serde_json::from_str::<Value>(&first).unwrap()["update"]["title"], "Read package manifest");

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
}

struct OwnedRun {
    service: Arc<Service>,
    task_id: String,
    control: Arc<RunControl>,
}

impl Drop for OwnedRun {
    fn drop(&mut self) {
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
        return Ok(Some((title, json!({"sessionId": session, "update": update}).to_string())));
    }

    let tool_call_id = patch
        .get("toolCallId")
        .or_else(|| patch.pointer("/toolCall/id"))
        .and_then(Value::as_str)
        .filter(|id| !id.trim().is_empty());
    let Some(tool_call_id) = tool_call_id else {
        return Ok(None);
    };
    let tool_call_id = tool_call_id
        .to_string();
    let key = (session.clone(), tool_call_id.clone());
    if !tool_updates.contains_key(&key) && tool_updates.len() >= MAX_TOOL_ACTIVITY_ITEMS {
        return Err("Too many ACP tool calls in one turn.".into());
    }
    let (title, detail) = {
        let update = tool_updates
            .entry(key)
            .or_insert_with(|| json!({
                "sessionUpdate": "tool_call",
                "toolCallId": tool_call_id,
            }));
        merge_activity_patch(update, patch);
        let Some(object) = update.as_object_mut() else {
            return Ok(None);
        };
        object.insert("sessionUpdate".into(), Value::String("tool_call".into()));
        object.insert("toolCallId".into(), Value::String(tool_call_id));
        let title = activity_title(update, "ACP tool action");
        let detail = json!({"sessionId": session, "update": update}).to_string();
        (title, detail)
    };
    let activity_bytes: usize = tool_updates.values().map(|value| value.to_string().len()).sum();
    if activity_bytes > MAX_TOOL_ACTIVITY_BYTES {
        return Err("ACP tool activity exceeds 2 MiB in one turn.".into());
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
) {
    // Validate the opaque one-time options before showing a control. Invalid
    // or perpetual-only requests are cancelled rather than broadened.
    if acp_protocol::permission_outcome(&params, false).is_err() {
        let _ = send(
            &control,
            acp_protocol::response(id, acp_protocol::cancelled_permission()),
        );
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
    });
}

fn notification_kind(value: &Value) -> &str {
    value
        .pointer("/params/update/sessionUpdate")
        .and_then(Value::as_str)
        .or_else(|| value.pointer("/params/update/type").and_then(Value::as_str))
        .unwrap_or_default()
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
    let mut extensions = match service.extension_config().map(|config| {
        crate::extensions_runtime::RuntimeExtensions::for_agent(&config, &task.agent_id)
    }) {
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
    let helper = match grant.as_ref() {
        Some(_) => match service.collaboration_helper() {
            Ok(helper) => Some(helper),
            Err(error) => {
                fail(&service, &task_id, &control, error);
                return;
            }
        },
        None => None,
    };
    let mut remote_collaboration = match (host.kind.as_str(), grant.as_ref(), helper.as_ref()) {
        ("ssh", Some(grant), Some(helper)) => {
            match runner::prepare_remote_collaboration(&host, &grant.endpoint, helper, &control) {
                Ok(remote) => Some(remote),
                Err(error) => {
                    fail(&service, &task_id, &control, error);
                    return;
                }
            }
        }
        _ => None,
    };
    let helper_for_session = remote_collaboration
        .as_ref()
        .map(|remote| remote.helper_path.as_str())
        .or_else(|| helper.as_ref().and_then(|path| path.to_str()));
    // SSH's reverse forward allocates a remote loopback port. The ACP agent
    // must receive that endpoint, never the desktop-only broker address.
    let session_grant = grant.as_ref().map(|grant| {
        let mut grant = grant.clone();
        if let Some(remote) = remote_collaboration.as_ref() {
            grant.endpoint = remote.endpoint.clone();
        }
        grant
    });
    let mut mcp_servers =
        crate::acp_collaboration::mcp_servers(helper_for_session, session_grant.as_ref());
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
        while let Ok(frame) = control_rx.recv() {
            if stdin
                .write_all(frame.as_bytes())
                .and_then(|_| stdin.write_all(b"\n"))
                .and_then(|_| stdin.flush())
                .is_err()
            {
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
    let mut resolved_cwd = task.cwd.clone();
    let mut seen_permission_ids = HashSet::<String>::new();
    let pending_permissions = Arc::new(AtomicUsize::new(0));
    loop {
        if control.is_cancelled() {
            thread::sleep(Duration::from_millis(150));
            let _ = service.complete_app_server_turn(&task_id, &control, None, "interrupted", None);
            return;
        }
        if !dirty_messages.is_empty() && last_message_flush.elapsed() >= Duration::from_millis(100)
        {
            for (item, text) in &messages {
                if dirty_messages.contains(item) {
                    if let Err(error) = service
                        .app_server_message(&task_id, &control, &turn, item, text, None, false)
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
        let value = match frames_rx.recv_timeout(wait) {
            Ok(Ok(value)) => value,
            Ok(Err(error)) => {
                if !recovery_budget_exhausted {
                    recover_transport(&service, &task_id, &control, error);
                } else {
                    fail(&service, &task_id, &control, error);
                }
                return;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
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
                    fail(&service, &task_id, &control,
                        "ACP permission request history exceeded its safety limit; reconnect this chat.");
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
                if extensions.has_http()
                    && value
                        .pointer("/result/agentCapabilities/mcpCapabilities/http")
                        .and_then(Value::as_bool)
                        != Some(true)
                {
                    fail(&service, &task_id, &control, "This ACP agent does not advertise HTTP MCP support; Monitter did not ignore the assigned server.");
                    return;
                }
                let (method, params) = if let Some(native) = task.native_session_id.as_deref() {
                    match capabilities.recovery_method() {
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
                    fail(&service, &task_id, &control, "ACP has not advertised support for saved fast mode or reasoning effort settings.");
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
                if let Some((method, params)) = permission {
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
                for (item, text) in &messages {
                    if let Err(error) = service
                        .app_server_message(&task_id, &control, &turn, item, text, None, true)
                    {
                        fail(&service, &task_id, &control, error);
                        return;
                    }
                }
                let stop_reason = value
                    .pointer("/result/stopReason")
                    .and_then(Value::as_str)
                    .unwrap_or("end_turn");
                let (status, error) = match stop_reason {
                    "end_turn" | "completed" => ("completed", None),
                    "cancelled" => ("interrupted", None),
                    "refusal" => ("error", Some("ACP agent refused this prompt.".into())),
                    "max_tokens" => (
                        "error",
                        Some(
                            "ACP agent reached its token limit before completing this prompt."
                                .into(),
                        ),
                    ),
                    other => ("error", Some(format!("ACP prompt stopped with {other}."))),
                };
                // A matching prompt response is the authoritative completion boundary.
                let _ = service.complete_app_server_turn(
                    &task_id,
                    &control,
                    Some(&turn),
                    status,
                    error,
                );
                control.clear_acp_turn_reservation();
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
            if !control.matches_app_server_thread(notification_session)
                || !control.matches_app_server_turn(&turn)
            {
                continue;
            }
            let kind = notification_kind(&value);
            if kind == "usage_update" {
                let usage = &params["update"];
                let used = usage.get("used").and_then(Value::as_i64);
                let size = usage.get("size").and_then(Value::as_i64);
                if used.is_none() || size.is_none() {
                    service.record(
                        &task_id,
                        "error",
                        "Usage capture warning",
                        "ACP usage_update omitted numeric used or size; the run continued.".into(),
                    );
                } else {
                    let detail = json!({"providerTurnId": turn, "used": used, "size": size, "cost": usage.get("cost")}).to_string();
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
            if matches!(kind, "agent_message_chunk" | "agent_message") {
                let content = &value["params"]["update"]["content"];
                let content_type = content["type"].as_str().unwrap_or("text");
                let placeholder = match content_type {
                    "image" => "",
                    "audio" => "\n[The ACP agent returned audio; playback is not supported yet.]\n",
                    "resource_link" => "\n[The ACP agent returned a resource link; automatic fetching is disabled.]\n",
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
                                &task_id, &control, &turn, &item, text, None, false,
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
                                    service.acp_message_image(&task_id, &control, &turn, &item, data)
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
                    let title = if kind == "plan" { "ACP plan" } else { "ACP tool activity" };
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
                        event: Some((
                            "tool".into(),
                            title,
                            detail,
                        )),
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
