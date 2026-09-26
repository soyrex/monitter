//! Compact, polling-friendly projections of the durable workspace snapshot.
//!
//! The full event transcript stays in the local store.  LAN and responsive
//! clients get only the activity that is useful while a task is live; older
//! diagnostics are explicitly paged when the user opens run detail.

use crate::model::{RunEvent, Snapshot};
use serde::Serialize;
use serde_json::{json, Value};
use std::{collections::HashMap, sync::Arc};

pub const MAX_LIVE_EVENTS_PER_TASK: usize = 60;
pub const MAX_LIVE_EVENTS: usize = 300;
pub const MAX_LIVE_DETAIL: usize = 1_000;
pub const MAX_ERROR_DETAIL: usize = 300;
pub const MAX_EVENT_PAGE: usize = 100;
pub const MAX_EVENT_PAGE_DETAIL_BYTES: usize = 128 * 1024;
pub const MAX_EVENT_DETAIL_CHUNK_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UiSnapshot {
    pub revision: String,
    pub snapshot: Option<Snapshot>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskEventsPage {
    pub events: Vec<RunEvent>,
    pub next_before: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventDetailChunk {
    pub chunk: String,
    pub next_offset: Option<usize>,
    pub total_bytes: usize,
}

pub fn event_detail_chunk(
    snapshot: &Snapshot,
    task_id: &str,
    event_id: &str,
    offset: Option<usize>,
    limit: Option<usize>,
) -> Result<EventDetailChunk, String> {
    let event = snapshot
        .events
        .iter()
        .find(|event| event.task_id == task_id && event.id == event_id)
        .ok_or_else(|| "Task event was not found.".to_string())?;
    let start = offset.unwrap_or(0).min(event.detail.len());
    if !event.detail.is_char_boundary(start) {
        return Err("Invalid event detail offset.".into());
    }
    let limit = limit
        .unwrap_or(32 * 1024)
        .clamp(1_024, MAX_EVENT_DETAIL_CHUNK_BYTES);
    let mut end = start.saturating_add(limit).min(event.detail.len());
    while end > start && !event.detail.is_char_boundary(end) {
        end -= 1;
    }
    Ok(EventDetailChunk {
        chunk: event.detail[start..end].to_string(),
        next_offset: (end < event.detail.len()).then_some(end),
        total_bytes: event.detail.len(),
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct Accepted {
    pub accepted: bool,
}

fn truncate_bytes(value: &str, max: usize) -> String {
    const SUFFIX: &str = "… [truncated]";
    if value.len() <= max {
        return value.into();
    }
    if max <= SUFFIX.len() {
        // ASCII stays byte-bounded even when the page has only one byte left.
        return "[truncated]".chars().take(max).collect();
    }
    let mut end = max - SUFFIX.len();
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{}", &value[..end], SUFFIX)
}

fn compact_detail(event: &RunEvent) -> String {
    if event.kind == "error" {
        truncate_bytes(event.detail.lines().next().unwrap_or(""), MAX_ERROR_DETAIL)
    } else if event.kind == "tool" {
        compact_acp_activity_detail(&event.detail)
    } else if event.kind == "jevDecision" {
        compact_jev_decision_detail(&event.detail)
    } else {
        truncate_bytes(&event.detail, MAX_LIVE_DETAIL)
    }
}

/// Keep Jev decision cards renderable in the 1 KiB live projection. The full
/// locally stored event remains available from the on-demand detail endpoint.
fn compact_jev_decision_detail(detail: &str) -> String {
    let Ok(mut card) = serde_json::from_str::<Value>(detail) else {
        return truncate_bytes(detail, MAX_LIVE_DETAIL);
    };
    let Some(object) = card.as_object_mut() else {
        return truncate_bytes(detail, MAX_LIVE_DETAIL);
    };
    if let Some(question) = object
        .get("question")
        .and_then(Value::as_str)
        .map(str::to_owned)
    {
        object.insert(
            "question".into(),
            Value::String(truncate_bytes(&question, 220)),
        );
    }
    let tool = object
        .get("toolName")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let selected = object
        .get("response")
        .and_then(|response| response.get("candidateId"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    if tool == "jev_choose" {
        let mut kept_probability = None;
        if let Some(response) = object.get_mut("response").and_then(Value::as_object_mut) {
            if let Some(selected_id) = selected.as_deref() {
                if let Some(probabilities) = response
                    .get_mut("probabilities")
                    .and_then(Value::as_object_mut)
                {
                    kept_probability = probabilities.get(selected_id).and_then(Value::as_f64);
                    let mut compact = serde_json::Map::new();
                    if let Some(value) = kept_probability {
                        compact.insert(selected_id.to_string(), json!(value));
                    }
                    *probabilities = compact;
                    if let Some(value) = kept_probability {
                        response.insert("otherProbability".into(), json!((1.0 - value).max(0.0)));
                    }
                }
            }
        }
        if let Some(labels) = object.get_mut("labels").and_then(Value::as_object_mut) {
            let selected_label = selected.as_deref().and_then(|id| labels.get(id).cloned());
            labels.clear();
            if let (Some(id), Some(label)) = (selected.as_deref(), selected_label) {
                labels.insert(
                    id.to_string(),
                    Value::String(truncate_bytes(label.as_str().unwrap_or(""), 100)),
                );
            }
        }
    } else if tool == "jev_assess" {
        if let Some(labels) = object.get_mut("labels").and_then(Value::as_object_mut) {
            for label in labels.values_mut() {
                if let Some(text) = label.as_str() {
                    *label = Value::String(truncate_bytes(text, 70));
                }
            }
        }
    }
    object.insert("compact".into(), Value::Bool(true));
    let fallback = {
        let object = card.as_object().expect("decision card remains an object");
        let mut response = serde_json::Map::new();
        if let Some(source) = object.get("response").and_then(Value::as_object) {
            for key in [
                "status",
                "kind",
                "candidateId",
                "confidence",
                "probabilityYes",
                "score",
                "probabilities",
                "otherProbability",
                "outcome",
                "modelTier",
                "reasoningLevel",
                "selectedModel",
                "applied",
                "modelChanged",
                "modelStatus",
                "appliedReasoning",
                "reasoningStatus",
                "autoEligible",
                "taskKind",
                "permissionTier",
            ] {
                if let Some(value) = source.get(key) {
                    response.insert(key.into(), value.clone());
                }
            }
        }
        let tool = object
            .get("toolName")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let selected = object
            .get("response")
            .and_then(|value| value.get("candidateId"))
            .and_then(Value::as_str);
        let mut labels = serde_json::Map::new();
        if let Some(source_labels) = object.get("labels").and_then(Value::as_object) {
            if tool == "jev_assess"
                && object
                    .get("response")
                    .and_then(|value| value.get("kind"))
                    .and_then(Value::as_str)
                    == Some("score")
            {
                for (id, label) in source_labels.iter().take(6) {
                    if let Some(label) = label.as_str() {
                        labels.insert(id.clone(), Value::String(truncate_bytes(label, 52)));
                    }
                }
            } else if let Some(id) = selected {
                if let Some(label) = source_labels.get(id).and_then(Value::as_str) {
                    labels.insert(id.to_string(), Value::String(truncate_bytes(label, 64)));
                }
            }
        }
        json!({
            "compact": true,
            "taskId": object.get("taskId"),
            "toolName": truncate_bytes(tool, 40),
            "question": object.get("question").and_then(Value::as_str).map(|value| truncate_bytes(value, 150)).unwrap_or_default(),
            "labels": labels,
            "response": response,
            "provider": object.get("provider").and_then(Value::as_str).map(|value| truncate_bytes(value, 40)),
            "model": object.get("model").and_then(Value::as_str).map(|value| truncate_bytes(value, 48)),
            "latencyMs": object.get("latencyMs"),
            "inputTokens": object.get("inputTokens"),
            "outputTokens": object.get("outputTokens"),
            "costUsd": object.get("costUsd"),
            "createdAt": object.get("createdAt"),
        })
    };
    let compact = card.to_string();
    if compact.len() <= MAX_LIVE_DETAIL {
        return compact;
    }
    let fallback = fallback.to_string();
    if fallback.len() <= MAX_LIVE_DETAIL {
        fallback
    } else {
        "{\"compact\":true,\"question\":\"Jev decision\",\"response\":{\"status\":\"ok\"}}".into()
    }
}

fn compact_acp_value(value: Option<&Value>, limit: usize) -> Value {
    let Some(value) = value else {
        return Value::Null;
    };
    match value {
        Value::String(text) => Value::String(truncate_bytes(text, limit)),
        _ if value.to_string().len() <= limit => value.clone(),
        _ => Value::String(truncate_bytes(&value.to_string(), limit)),
    }
}

/// Live snapshots cannot carry unbounded diagnostics, but truncating the raw
/// JSON would make ACP activity unparsable and prevent lifecycle coalescing.
/// Retain the identifier and readable state envelope; full input/output stays
/// available from the on-demand event-detail endpoint.
fn compact_acp_update(update: &serde_json::Map<String, Value>, include_detail: bool) -> Value {
    let mut compact = serde_json::Map::new();
    for (key, limit) in [
        ("sessionUpdate", 80),
        ("toolCallId", 160),
        ("title", 180),
        ("toolName", 180),
        ("kind", 100),
        ("status", 100),
    ] {
        if let Some(value) = update.get(key) {
            compact.insert(key.into(), compact_acp_value(Some(value), limit));
        }
    }
    if include_detail {
        for (key, limit) in [
            ("content", 260),
            ("rawInput", 220),
            ("rawOutput", 220),
            ("error", 220),
            ("locations", 160),
        ] {
            if let Some(value) = update.get(key) {
                compact.insert(key.into(), compact_acp_value(Some(value), limit));
            }
        }
    }
    Value::Object(compact)
}

fn compact_acp_activity_detail(detail: &str) -> String {
    let Ok(value) = serde_json::from_str::<Value>(detail) else {
        return truncate_bytes(detail, MAX_LIVE_DETAIL);
    };
    let Some(session_id) = value.get("sessionId").and_then(Value::as_str) else {
        return truncate_bytes(detail, MAX_LIVE_DETAIL);
    };
    let Some(update) = value.get("update").and_then(Value::as_object) else {
        return truncate_bytes(detail, MAX_LIVE_DETAIL);
    };
    let compact = json!({
        "sessionId": session_id,
        "update": compact_acp_update(update, true),
    });
    let compact = compact.to_string();
    if compact.len() <= MAX_LIVE_DETAIL {
        compact
    } else {
        let metadata = json!({
            "sessionId": session_id,
            "update": compact_acp_update(update, false),
        })
        .to_string();
        if metadata.len() <= MAX_LIVE_DETAIL {
            metadata
        } else {
            // Do not truncate an identifier: a shortened session or tool ID
            // could collide with a different lifecycle. The full local event
            // remains available on demand.
            json!({"acpActivityTruncated": true}).to_string()
        }
    }
}

fn compact_event(event: &RunEvent) -> RunEvent {
    RunEvent {
        id: event.id.clone(),
        task_id: event.task_id.clone(),
        kind: event.kind.clone(),
        title: event.title.clone(),
        detail: compact_detail(event).into(),
        created_at: event.created_at,
    }
}

/// All non-event state is preserved. Events are chronological, with at most
/// the newest useful activity per task and no unbounded diagnostic detail.
pub fn compact_snapshot(snapshot: &Snapshot) -> Snapshot {
    let mut retained = HashMap::<&str, usize>::new();
    let mut newest_first = Vec::new();
    for event in snapshot.events.iter().rev() {
        // Assistant content is already represented by messages. Provider logs
        // and streamed output belong in the explicit diagnostic timeline.
        if matches!(event.kind.as_str(), "log" | "output") {
            continue;
        }
        let count = retained.entry(&event.task_id).or_default();
        if *count < MAX_LIVE_EVENTS_PER_TASK && newest_first.len() < MAX_LIVE_EVENTS {
            *count += 1;
            newest_first.push(compact_event(event));
            if newest_first.len() == MAX_LIVE_EVENTS {
                break;
            }
        }
    }
    newest_first.reverse();
    Snapshot {
        hosts: snapshot.hosts.clone(),
        agents: snapshot.agents.clone(),
        tasks: snapshot.tasks.clone(),
        messages: snapshot.messages.clone(),
        mail_batches: snapshot.mail_batches.clone(),
        work_plans: snapshot.work_plans.clone(),
        events: newest_first.into_iter().map(Arc::new).collect(),
        channels: snapshot.channels.clone(),
        project_board_messages: snapshot.project_board_messages.clone(),
        projects: snapshot.projects.clone(),
        settings: snapshot.settings.clone(),
        collaborations: snapshot.collaborations.clone(),
        subagent_sessions: snapshot.subagent_sessions.clone(),
        subagent_transcripts: snapshot.subagent_transcripts.clone(),
        queued_messages: snapshot.queued_messages.clone(),
        approval_requests: snapshot.approval_requests.clone(),
        approval_rules: snapshot.approval_rules.clone(),
        schedules: snapshot.schedules.clone(),
        schedule_runs: snapshot.schedule_runs.clone(),
        pending_throwaway_task_ids: snapshot.pending_throwaway_task_ids.clone(),
    }
}

/// Pages are newest-first. `before` is an exclusive index into this task's
/// chronological event list, supplied from the previous page's `nextBefore`.
pub fn task_events(
    snapshot: &Snapshot,
    task_id: &str,
    before: Option<i64>,
    limit: Option<u32>,
) -> TaskEventsPage {
    let limit = limit.unwrap_or(60).clamp(1, MAX_EVENT_PAGE as u32) as usize;
    let mut used = 0usize;
    let mut events = Vec::new();
    let mut remaining = false;
    let task_events = snapshot
        .events
        .iter()
        .filter(|event| event.task_id == task_id)
        .collect::<Vec<_>>();
    let before = before
        .unwrap_or(task_events.len() as i64)
        .clamp(0, task_events.len() as i64) as usize;
    for (index, event) in task_events[..before].iter().enumerate().rev() {
        if events.len() >= limit || used >= MAX_EVENT_PAGE_DETAIL_BYTES {
            remaining = true;
            break;
        }
        let available = MAX_EVENT_PAGE_DETAIL_BYTES.saturating_sub(used);
        let detail = if event.detail.len() > available {
            truncate_bytes(&event.detail, available).into()
        } else {
            event.detail.clone()
        };
        let event = RunEvent {
            id: event.id.clone(),
            task_id: event.task_id.clone(),
            kind: event.kind.clone(),
            title: event.title.clone(),
            detail: detail.into(),
            created_at: event.created_at,
        };
        used += event.detail.len();
        events.push((index, event));
    }
    let next_before = if remaining {
        events.last().map(|(index, _)| *index as i64)
    } else {
        None
    };
    TaskEventsPage {
        events: events.into_iter().map(|(_, event)| event).collect(),
        next_before,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::default_snapshot;

    fn event(task_id: &str, created_at: i64, kind: &str, detail: String) -> Arc<RunEvent> {
        Arc::new(RunEvent {
            id: format!("{task_id}-{created_at}"),
            task_id: task_id.into(),
            kind: kind.into(),
            title: "Event".into(),
            detail: detail.into(),
            created_at,
        })
    }

    #[test]
    fn compact_snapshot_keeps_recent_events_and_compacts_details() {
        let mut snapshot = default_snapshot();
        snapshot.events = (0..62)
            .map(|n| event("a", n, "tool", "x".repeat(2_100)))
            .collect();
        snapshot.events.push(event(
            "b",
            1,
            "error",
            format!("{}\nsecret diagnostic", "e".repeat(400)),
        ));
        let compact = compact_snapshot(&snapshot);
        assert_eq!(
            compact
                .events
                .iter()
                .filter(|event| event.task_id == "a")
                .count(),
            60
        );
        assert_eq!(compact.events.first().unwrap().created_at, 2);
        assert!(compact
            .events
            .iter()
            .filter(|event| event.kind == "tool")
            .all(|event| event.detail.chars().count() <= 1_015));
        assert!(compact.events.last().unwrap().detail.len() <= MAX_ERROR_DETAIL);
        assert!(compact
            .events
            .last()
            .unwrap()
            .detail
            .ends_with("[truncated]"));
        assert_eq!(snapshot.events.len(), 63);
    }

    #[test]
    fn compact_acp_activity_keeps_valid_grouping_metadata() {
        let detail = json!({
            "sessionId": "claude-session",
            "update": {
                "sessionUpdate": "tool_call",
                "toolCallId": "tool-123",
                "title": "Read an oversized file",
                "kind": "read",
                "status": "failed",
                "rawInput": {"path": "x".repeat(5_000)},
                "rawOutput": {"error": "permission denied", "body": "y".repeat(5_000)}
            }
        })
        .to_string();
        let compact = compact_acp_activity_detail(&detail);
        assert!(compact.len() <= MAX_LIVE_DETAIL);
        let compact: Value = serde_json::from_str(&compact).expect("valid ACP JSON");
        assert_eq!(compact["sessionId"], "claude-session");
        assert_eq!(compact["update"]["toolCallId"], "tool-123");
        assert_eq!(compact["update"]["status"], "failed");
        assert_eq!(compact["update"]["title"], "Read an oversized file");
    }

    #[test]
    fn event_page_is_bounded_and_newest_first() {
        let mut snapshot = default_snapshot();
        snapshot.events = (0..130).map(|n| event("a", n, "log", "x".into())).collect();
        let page = task_events(&snapshot, "a", None, Some(500));
        assert_eq!(page.events.len(), 100);
        assert_eq!(page.events[0].created_at, 129);
        assert_eq!(page.next_before, Some(30));
    }

    #[test]
    fn event_page_stays_byte_bounded_for_unicode_diagnostics() {
        let mut snapshot = default_snapshot();
        snapshot
            .events
            .push(event("a", 1, "log", "🚀".repeat(200_000)));
        let page = task_events(&snapshot, "a", None, Some(1));
        assert!(page.events[0].detail.len() <= MAX_EVENT_PAGE_DETAIL_BYTES);
        assert!(page.events[0].detail.ends_with("[truncated]"));
    }

    #[test]
    fn tiny_remaining_page_budget_never_expands_utf8() {
        let mut snapshot = default_snapshot();
        snapshot.events.push(event("a", 1, "log", "🚀".repeat(10)));
        snapshot.events.push(event(
            "a",
            2,
            "log",
            "x".repeat(MAX_EVENT_PAGE_DETAIL_BYTES - 1),
        ));
        let page = task_events(&snapshot, "a", None, Some(2));
        assert_eq!(
            page.events
                .iter()
                .map(|event| event.detail.len())
                .sum::<usize>(),
            MAX_EVENT_PAGE_DETAIL_BYTES
        );
    }

    #[test]
    fn event_detail_is_lazy_chunked_and_task_scoped() {
        let mut snapshot = default_snapshot();
        snapshot
            .events
            .push(event("a", 1, "tool", "🚀".repeat(40_000)));
        let first = event_detail_chunk(&snapshot, "a", "a-1", None, Some(32 * 1024)).unwrap();
        assert!(first.chunk.len() <= 32 * 1024);
        assert_eq!(first.total_bytes, "🚀".repeat(40_000).len());
        let offset = first.next_offset.expect("large detail has another chunk");
        let second =
            event_detail_chunk(&snapshot, "a", "a-1", Some(offset), Some(32 * 1024)).unwrap();
        assert!(!second.chunk.is_empty());
        assert!(event_detail_chunk(&snapshot, "other", "a-1", None, None).is_err());
        assert!(event_detail_chunk(&snapshot, "a", "missing", None, None).is_err());
        assert!(event_detail_chunk(&snapshot, "a", "a-1", Some(1), None).is_err());
    }
}
