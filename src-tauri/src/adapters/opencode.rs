use crate::{model::Task, runner::Parsed};
use serde_json::Value;

/// Build a non-interactive OpenCode command. The prompt is deliberately read
/// from stdin by `opencode run` when stdin is not a TTY; it is not passed as an
/// argv argument so the shared transport can handle local and SSH prompts alike.
pub fn args(task: &Task) -> Vec<String> {
    let mut args = vec![
        "run".into(),
        "--format".into(),
        "json".into(),
        "--thinking".into(),
        // OpenCode uses PWD while selecting its local project instance. A
        // desktop-launched child can inherit a different PWD even when its
        // process current directory is correct, so make the task folder an
        // explicit CLI input.
        "--dir".into(),
        task.cwd.clone(),
    ];
    if let Some(session_id) = &task.native_session_id {
        args.extend(["--session".into(), session_id.clone()]);
    }
    // OpenCode requires its provider-qualified `provider/model` identifier.
    // Older Monitter tasks stored a bare model name (for example `mimo-v2.5`),
    // which OpenCode rejects before it can start a session. In that case, omit
    // the flag and let the user's configured OpenCode default select a model.
    let model = task.model.trim();
    if model
        .split_once('/')
        .is_some_and(|(provider, name)| !provider.is_empty() && !name.is_empty())
    {
        args.extend(["--model".into(), model.into()]);
    }
    args
}

fn session_id(value: &Value) -> Option<String> {
    value
        .get("sessionID")
        .and_then(Value::as_str)
        .or_else(|| value.pointer("/part/sessionID").and_then(Value::as_str))
        .map(str::to_owned)
}

fn part_text(part: &Value) -> String {
    part.get("text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
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

/// Convert OpenCode's `opencode run --format json` events into Monitter events.
/// The CLI emits only completed text and reasoning parts, and completed/error
/// tool parts, so this intentionally does not fabricate partial activity.
pub fn parse_event(value: &Value) -> Vec<Parsed> {
    let ty = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let session_id = session_id(value);
    let part = value.get("part").unwrap_or(value);
    let part_type = part.get("type").and_then(Value::as_str).unwrap_or_default();

    match ty {
        "text" if part_type == "text" => {
            let text = part_text(part);
            if text.trim().is_empty() {
                return Vec::new();
            }
            vec![Parsed {
                native_session_id: session_id,
                assistant: Some(text.clone()),
                event: Some(("output".into(), "Assistant response".into(), text)),
                failed: false,
            }]
        }
        "reasoning" if part_type == "reasoning" => vec![event(
            session_id,
            "reasoning",
            "Reasoning",
            part_text(part),
            false,
        )],
        "tool_use" if part_type == "tool" => {
            let title = part
                .get("tool")
                .and_then(Value::as_str)
                .unwrap_or("Tool activity");
            let state = part.get("state").cloned().unwrap_or(Value::Null);
            // Tool errors are part of a recoverable agent turn; only a top-level
            // OpenCode error marks the process run as failed.
            let failed = false;
            vec![event(
                session_id,
                "tool",
                title,
                if state.is_null() {
                    String::new()
                } else {
                    state.to_string()
                },
                failed,
            )]
        }
        "step_finish" if part_type == "step-finish" => {
            let mut usage = serde_json::Map::new();
            if let Some(tokens) = part.get("tokens") {
                usage.insert("tokens".into(), tokens.clone());
            }
            if let Some(cost) = part.get("cost") {
                usage.insert("cost".into(), cost.clone());
            }
            let parsed = event(
                session_id,
                "usage",
                "Usage updated",
                Value::Object(usage).to_string(),
                false,
            );
            // `step-finish` is a completed step, so its reported accounting is
            // a delta, not a session snapshot.
            vec![parsed]
        }
        "error" => {
            let error = value.get("error").unwrap_or(value);
            let detail = error
                .get("message")
                .and_then(Value::as_str)
                .or_else(|| error.pointer("/data/message").and_then(Value::as_str))
                .unwrap_or("OpenCode reported an error")
                .to_owned();
            vec![event(session_id, "error", "OpenCode error", detail, true)]
        }
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
            provider: "opencode".into(),
            codex_home: None,
            model: model.into(),
            model_settings: None,
            sandbox: "read-only".into(),
            project_id: None,
            acp: None,
        }
    }

    #[test]
    fn args_use_task_directory_stdin_json_thinking_and_resume_without_auto_approval() {
        let args = args(&task(Some("ses_123"), "openai/gpt-5"));

        assert_eq!(
            args,
            vec![
                "run",
                "--format",
                "json",
                "--thinking",
                "--dir",
                "/tmp",
                "--session",
                "ses_123",
                "--model",
                "openai/gpt-5",
            ]
        );
        assert!(!args.iter().any(|arg| arg == "--auto"));
    }

    #[test]
    fn args_omit_legacy_unqualified_model_name() {
        let args = args(&task(None, "mimo-v2.5"));

        assert!(!args.iter().any(|arg| arg == "--model"));
    }

    #[test]
    fn parses_text_reasoning_tool_usage_and_errors() {
        let text = parse_event(&serde_json::json!({
            "type": "text", "sessionID": "ses_123",
            "part": { "type": "text", "text": "Answer" }
        }));
        assert_eq!(text[0].native_session_id.as_deref(), Some("ses_123"));
        assert_eq!(text[0].assistant.as_deref(), Some("Answer"));

        let reasoning = parse_event(&serde_json::json!({
            "type": "reasoning", "sessionID": "ses_123",
            "part": { "type": "reasoning", "text": "Thinking" }
        }));
        assert_eq!(reasoning[0].event.as_ref().unwrap().0, "reasoning");

        let tool = parse_event(&serde_json::json!({
            "type": "tool_use", "sessionID": "ses_123",
            "part": { "type": "tool", "tool": "bash", "state": { "status": "completed" } }
        }));
        assert_eq!(tool[0].event.as_ref().unwrap().1, "bash");

        let usage = parse_event(&serde_json::json!({
            "type": "step_finish", "sessionID": "ses_123",
            "part": { "type": "step-finish", "tokens": { "input": 4 }, "cost": 0 }
        }));
        assert_eq!(usage[0].event.as_ref().unwrap().0, "usage");

        let failure = parse_event(&serde_json::json!({
            "type": "error", "sessionID": "ses_123",
            "error": { "data": { "message": "denied" } }
        }));
        assert!(failure[0].failed);
        assert_eq!(failure[0].event.as_ref().unwrap().2, "denied");
    }
}
