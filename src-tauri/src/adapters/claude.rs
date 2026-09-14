use crate::{model::Task, runner::Parsed};
use serde_json::Value;

/// Build Claude Code's headless, bidirectional transport. In stream-json mode
/// the process deliberately remains alive after a result so its native context
/// is available to the next user message on stdin.
pub fn args(task: &Task) -> Vec<String> {
    let mut args = vec![
        "--print".into(),
        "--input-format".into(),
        "stream-json".into(),
        "--output-format".into(),
        "stream-json".into(),
        "--verbose".into(),
        "--permission-prompts".into(),
        "host".into(),
    ];
    if task.sandbox == "yolo" {
        // Claude Code's documented non-interactive permission bypass.
        args.push("--dangerously-skip-permissions".into());
    }
    if let Some(session_id) = &task.native_session_id {
        args.extend(["--resume".into(), session_id.clone()]);
    }
    if !task.model.trim().is_empty() {
        args.extend(["--model".into(), task.model.clone()]);
    }
    args
}

/// A new user turn on Claude Code's stream-json stdin protocol. `parent_tool_use_id`
/// is required by the CLI schema even for ordinary top-level user turns.
pub fn user_frame(prompt: &str) -> Value {
    serde_json::json!({
        "type": "user",
        "message": { "role": "user", "content": prompt },
        "parent_tool_use_id": Value::Null,
    })
}

/// Answer the CLI-owned `can_use_tool` control request. The CLI validates both
/// shapes strictly; classify the result so its audit telemetry reflects the
/// actual desktop action rather than an inferred default.
pub fn permission_response(request_id: &str, allow: bool, input: &Value) -> Value {
    let response = if allow {
        serde_json::json!({
            "behavior": "allow",
            "updatedInput": input,
            "decisionClassification": "user_temporary",
        })
    } else {
        serde_json::json!({
            "behavior": "deny",
            "message": "Denied by the Monitter user.",
            "decisionClassification": "user_reject",
        })
    };
    serde_json::json!({
        "type": "control_response",
        "response": {
            "subtype": "success",
            "request_id": request_id,
            "response": response,
        }
    })
}

fn session_id(value: &Value) -> Option<String> {
    value
        .get("session_id")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn event(
    session_id: Option<String>,
    kind: &str,
    title: &str,
    detail: String,
    failed: bool,
) -> Parsed {
    Parsed {
        native_session_id: session_id,
        assistant: None,
        event: Some((kind.into(), title.into(), detail)),
        failed,
    }
}

fn content_events(value: &Value, session_id: Option<String>) -> Vec<Parsed> {
    let Some(content) = value.pointer("/message/content").and_then(Value::as_array) else {
        return Vec::new();
    };

    content
        .iter()
        .filter_map(|block| {
            let block_type = block.get("type").and_then(Value::as_str)?;
            match block_type {
                "text" => {
                    let text = block.get("text").and_then(Value::as_str)?.trim();
                    (!text.is_empty()).then(|| Parsed {
                        native_session_id: session_id.clone(),
                        assistant: Some(text.into()),
                        event: Some(("output".into(), "Assistant response".into(), text.into())),
                        failed: false,
                    })
                }
                "thinking" => Some(event(
                    session_id.clone(),
                    "reasoning",
                    "Reasoning",
                    block
                        .get("thinking")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .into(),
                    false,
                )),
                "tool_use" => Some(event(
                    session_id.clone(),
                    "tool",
                    block
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("Tool activity"),
                    block
                        .get("input")
                        .cloned()
                        .unwrap_or(Value::Null)
                        .to_string(),
                    false,
                )),
                _ => None,
            }
        })
        .collect()
}

/// Convert Claude Code `--output-format stream-json --verbose` records into
/// Monitter events. The final `result` envelope carries usage only: its `result`
/// text is intentionally ignored because the same assistant text already arrives
/// in an earlier `assistant` record.
pub fn parse_event(value: &Value) -> Vec<Parsed> {
    let session_id = session_id(value);
    match value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
    {
        "system" if value.get("subtype").and_then(Value::as_str) == Some("init") => session_id
            .map(|native_session_id| Parsed {
                native_session_id: Some(native_session_id),
                assistant: None,
                event: None,
                failed: false,
            })
            .into_iter()
            .collect(),
        "assistant" => content_events(value, session_id),
        "user" => value
            .pointer("/message/content")
            .and_then(Value::as_array)
            .map(|content| {
                content
                    .iter()
                    .filter_map(|block| {
                        (block.get("type").and_then(Value::as_str) == Some("tool_result")).then(
                            || {
                                event(
                                    session_id.clone(),
                                    "tool",
                                    "Tool result",
                                    block
                                        .get("content")
                                        .cloned()
                                        .unwrap_or(Value::Null)
                                        .to_string(),
                                    false,
                                )
                            },
                        )
                    })
                    .collect()
            })
            .unwrap_or_default(),
        "result" => {
            let failed = value
                .get("is_error")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let mut usage = serde_json::Map::new();
            for key in [
                "total_cost_usd",
                "duration_ms",
                "duration_api_ms",
                "num_turns",
                "stop_reason",
            ] {
                if let Some(item) = value.get(key) {
                    usage.insert(key.into(), item.clone());
                }
            }
            // Stream-json places token accounting under `usage`; retain only
            // fields actually present rather than manufacturing a total.
            if let Some(item) = value.get("usage") { usage.insert("usage".into(), item.clone()); }
            let mut events = vec![event(
                session_id.clone(),
                "usage",
                "Usage updated",
                Value::Object(usage).to_string(),
                false,
            )];
            if failed {
                let detail = value
                    .get("errors")
                    .and_then(Value::as_array)
                    .and_then(|errors| errors.first())
                    .and_then(Value::as_str)
                    .unwrap_or("Claude Code reported an error")
                    .into();
                events.push(event(
                    session_id,
                    "error",
                    "Claude Code error",
                    detail,
                    true,
                ));
            }
            events
        }
        "error" => vec![event(
            session_id,
            "error",
            "Claude Code error",
            value
                .get("error")
                .and_then(Value::as_str)
                .or_else(|| value.get("message").and_then(Value::as_str))
                .unwrap_or("Claude Code reported an error")
                .into(),
            true,
        )],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{id, now};

    fn task(native_session_id: Option<&str>, model: &str) -> Task {
        Task {
            id: id(),
            agent_id: id(),
            title: "test".into(),
            native_session_id: native_session_id.map(str::to_owned),
            status: "idle".into(),
            archived: false,
            created_at: now(),
            updated_at: now(),
            parent_task_id: None,
            channel_id: None,
            host_id: id(),
            cwd: "/tmp".into(),
            provider: "claude".into(),
            model: model.into(),
            model_settings: None,
            sandbox: "read-only".into(),
            project_id: None,
            acp: None,
        }
    }

    #[test]
    fn args_stream_stdin_and_resume_without_permission_bypass() {
        let args = args(&task(Some("abc123"), "sonnet"));

        assert_eq!(
            args,
            vec![
                "--print",
                "--input-format",
                "stream-json",
                "--output-format",
                "stream-json",
                "--verbose",
                "--permission-prompts",
                "host",
                "--resume",
                "abc123",
                "--model",
                "sonnet",
            ]
        );
        assert!(!args.iter().any(|arg| arg.contains("skip-permissions")));
        assert!(!args.iter().any(|arg| arg == "acceptEdits"));
    }

    #[test]
    fn frames_preserve_user_text_and_exact_permission_input() {
        let user = user_frame("Continue this exact context.");
        assert_eq!(user["type"], "user");
        assert_eq!(user["message"]["content"], "Continue this exact context.");
        assert!(user["parent_tool_use_id"].is_null());

        let input = serde_json::json!({"command": "git status --short"});
        let allow = permission_response("request-1", true, &input);
        assert_eq!(allow["response"]["request_id"], "request-1");
        assert_eq!(allow["response"]["response"]["behavior"], "allow");
        assert_eq!(allow["response"]["response"]["updatedInput"], input);

        let deny = permission_response("request-2", false, &input);
        assert_eq!(deny["response"]["response"]["behavior"], "deny");
        assert_eq!(
            deny["response"]["response"]["decisionClassification"],
            "user_reject"
        );
    }

    #[test]
    fn yolo_uses_claudes_documented_permission_bypass() {
        let mut task = task(None, "sonnet");
        task.sandbox = "yolo".into();
        assert!(args(&task)
            .iter()
            .any(|arg| arg == "--dangerously-skip-permissions"));
    }

    #[test]
    fn parses_assistant_content_and_result_usage_without_duplicate_final_reply() {
        let assistant = parse_event(&serde_json::json!({
            "type": "assistant", "session_id": "abc123",
            "message": { "content": [
                { "type": "thinking", "thinking": "Inspecting" },
                { "type": "tool_use", "name": "Read", "input": { "file_path": "README.md" } },
                { "type": "text", "text": "Done." }
            ] }
        }));
        assert_eq!(assistant.len(), 3);
        assert_eq!(assistant[0].event.as_ref().unwrap().0, "reasoning");
        assert_eq!(assistant[1].event.as_ref().unwrap().1, "Read");
        assert_eq!(assistant[2].assistant.as_deref(), Some("Done."));

        let result = parse_event(&serde_json::json!({
            "type": "result", "session_id": "abc123", "is_error": false,
            "result": "Done.", "total_cost_usd": 0.01, "num_turns": 1,
            "usage": {"input_tokens": 12, "output_tokens": 3, "cache_read_input_tokens": 2, "cache_creation_input_tokens": 1}
        }));
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].event.as_ref().unwrap().0, "usage");
        assert!(result[0].assistant.is_none());
        let usage: Value = serde_json::from_str(&result[0].event.as_ref().unwrap().2).unwrap();
        assert_eq!(usage["usage"]["cache_read_input_tokens"], 2);

        let tool_result = parse_event(&serde_json::json!({
            "type": "user", "session_id": "abc123", "message": { "content": [
                { "type": "tool_result", "content": "permission denied", "is_error": true }
            ] }
        }));
        assert_eq!(tool_result[0].event.as_ref().unwrap().0, "tool");
        assert!(!tool_result[0].failed);
    }

    #[test]
    fn parses_init_session_and_failed_result() {
        let init = parse_event(&serde_json::json!({
            "type": "system", "subtype": "init", "session_id": "abc123"
        }));
        assert_eq!(init[0].native_session_id.as_deref(), Some("abc123"));

        let failed = parse_event(&serde_json::json!({
            "type": "result", "session_id": "abc123", "is_error": true,
            "errors": ["permission denied"]
        }));
        assert_eq!(failed.len(), 2);
        assert!(failed[1].failed);
        assert_eq!(failed[1].event.as_ref().unwrap().2, "permission denied");
    }
}
