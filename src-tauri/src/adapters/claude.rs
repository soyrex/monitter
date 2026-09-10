use crate::{model::Task, runner::Parsed};
use serde_json::Value;

/// Build a non-interactive Claude Code command. With `-p`, Claude Code accepts
/// a text prompt from a non-TTY stdin stream, so the shared local/SSH transport
/// can supply the prompt without placing it in argv.
pub fn args(task: &Task) -> Vec<String> {
    let mut args = vec![
        "--print".into(),
        "--output-format".into(),
        "stream-json".into(),
        "--verbose".into(),
    ];
    if let Some(session_id) = &task.native_session_id {
        args.extend(["--resume".into(), session_id.clone()]);
    }
    if !task.model.trim().is_empty() {
        args.extend(["--model".into(), task.model.clone()]);
    }
    args
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
        }
    }

    #[test]
    fn args_stream_stdin_and_resume_without_permission_bypass() {
        let args = args(&task(Some("abc123"), "sonnet"));

        assert_eq!(
            args,
            vec![
                "--print",
                "--output-format",
                "stream-json",
                "--verbose",
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
            "result": "Done.", "total_cost_usd": 0.01, "num_turns": 1
        }));
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].event.as_ref().unwrap().0, "usage");
        assert!(result[0].assistant.is_none());

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
