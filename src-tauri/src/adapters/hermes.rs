use crate::{model::Task, runner::Parsed};
use serde_json::Value;

/// The Hermes TUI gateway is a full-duplex JSON-RPC service. This small
/// standard-library bridge adapts it to Monitter's one-prompt stdin / JSONL
/// stdout runner contract without changing the user's Hermes configuration.
pub const BRIDGE: &str = include_str!("hermes_bridge.py");
pub const BRIDGE_PROGRAM: &str = "python3";

/// Build arguments for `python3`. `hermes_path` is the configured Hermes CLI
/// path, or `hermes` when the host uses its PATH default.
pub fn args(task: &Task, hermes_path: &str) -> Vec<String> {
    let mut args = vec![
        "-u".into(),
        "-c".into(),
        BRIDGE.into(),
        "--hermes".into(),
        if hermes_path.trim().is_empty() {
            "hermes".into()
        } else {
            hermes_path.trim().into()
        },
        "--cwd".into(),
        task.cwd.clone(),
    ];
    if let Some(session_id) = task
        .native_session_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        args.extend(["--session".into(), session_id.into()]);
    }
    if !task.model.trim().is_empty() {
        args.extend(["--model".into(), task.model.trim().into()]);
    }
    args
}

fn session_id(value: &Value) -> Option<String> {
    value
        .get("session_id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn json_detail(value: Option<&Value>) -> String {
    value
        .map(|value| match value {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        })
        .unwrap_or_default()
}

fn event(
    native_session_id: Option<String>,
    kind: &str,
    title: &str,
    detail: String,
    failed: bool,
) -> Parsed {
    Parsed {
        native_session_id,
        assistant: None,
        event: Some((kind.into(), title.into(), detail)),
        failed,
    }
}

/// Convert the bridge's normalized JSONL records into Monitter run events.
/// Interactive requests are visible error rows but remain recoverable: Hermes
/// receives an explicit deny/empty response and may continue the turn.
pub fn parse_event(value: &Value) -> Vec<Parsed> {
    let native_session_id = session_id(value);
    match value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
    {
        "session" => native_session_id
            .map(|native_session_id| Parsed {
                native_session_id: Some(native_session_id),
                assistant: None,
                event: None,
                failed: false,
            })
            .into_iter()
            .collect(),
        "output" => {
            let text = value
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            if text.trim().is_empty() {
                Vec::new()
            } else {
                vec![Parsed {
                    native_session_id,
                    assistant: Some(text.clone()),
                    event: Some(("output".into(), "Assistant response".into(), text)),
                    failed: false,
                }]
            }
        }
        "reasoning" => vec![event(
            native_session_id,
            "reasoning",
            "Reasoning",
            value
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .into(),
            false,
        )],
        "tool" => {
            let name = value
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("Tool activity");
            let phase = value
                .get("phase")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let title = if phase.is_empty() {
                name.to_owned()
            } else {
                format!("{name} · {phase}")
            };
            let is_computer = name == "computer_use";
            let detail = if is_computer {
                let payload = value.pointer("/detail/payload").unwrap_or(&Value::Null);
                serde_json::json!({
                    "id": payload.get("tool_id").cloned().unwrap_or(Value::Null),
                    "phase": phase,
                    "tool": name,
                    "summary": payload
                        .get("summary")
                        .or_else(|| payload.get("context"))
                        .map(|item| match item {
                            Value::String(text) => text.clone(),
                            other => other.to_string(),
                        })
                        .unwrap_or_default(),
                })
                .to_string()
            } else {
                json_detail(value.get("detail"))
            };
            let mut events = vec![event(
                native_session_id.clone(),
                "tool",
                &title,
                json_detail(value.get("detail")),
                false,
            )];
            if is_computer {
                events.push(event(
                    native_session_id,
                    "computer",
                    "Computer activity",
                    detail,
                    false,
                ));
            }
            events
        }
        "usage" => vec![event(
            native_session_id,
            "usage",
            "Usage updated",
            json_detail(value.get("detail")),
            false,
        )],
        "status" => {
            let status_kind = value
                .get("status_kind")
                .and_then(Value::as_str)
                .unwrap_or("status");
            let is_goal = status_kind == "goal";
            vec![event(
                native_session_id,
                if is_goal { "goal" } else { "status" },
                if is_goal {
                    "Hermes goal"
                } else if status_kind == "todo" {
                    "Hermes plan updated"
                } else {
                    "Hermes status"
                },
                json_detail(value.get("detail")),
                false,
            )]
        }
        "permission" => {
            let request = value
                .get("request")
                .and_then(Value::as_str)
                .unwrap_or("request");
            vec![event(
                native_session_id,
                "error",
                &format!("Hermes {request} denied"),
                json_detail(value.get("detail")),
                false,
            )]
        }
        "log" => vec![event(
            native_session_id,
            "log",
            "Hermes gateway",
            value
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .into(),
            false,
        )],
        "error" => vec![event(
            native_session_id,
            "error",
            "Hermes error",
            value
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("Hermes reported an error")
                .into(),
            value.get("fatal").and_then(Value::as_bool).unwrap_or(true),
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
            provider: "hermes".into(),
            model: model.into(),
            sandbox: "read-only".into(),
            project_id: None,
        }
    }

    #[test]
    fn bridge_args_keep_prompt_off_argv_and_do_not_bypass_approval() {
        let args = args(
            &task(Some("session-123"), "model-name"),
            "~/.local/bin/hermes",
        );
        assert_eq!(args[0], "-u");
        assert_eq!(args[1], "-c");
        assert_eq!(
            args[3..],
            [
                "--hermes",
                "~/.local/bin/hermes",
                "--cwd",
                "/tmp",
                "--session",
                "session-123",
                "--model",
                "model-name"
            ]
        );
        assert!(!args
            .iter()
            .any(|arg| matches!(arg.as_str(), "--yolo" | "--auto")));
    }

    #[test]
    fn parses_durable_session_output_computer_goal_and_recoverable_denial() {
        let session = parse_event(&serde_json::json!({
            "type": "session", "session_id": "session-123"
        }));
        assert_eq!(session[0].native_session_id.as_deref(), Some("session-123"));

        let output = parse_event(&serde_json::json!({
            "type": "output", "session_id": "session-123", "text": "Done."
        }));
        assert_eq!(output[0].assistant.as_deref(), Some("Done."));

        let computer = parse_event(&serde_json::json!({
            "type": "tool", "session_id": "session-123", "name": "computer_use",
            "phase": "completed", "detail": {
                "event": "tool.complete",
                "payload": {
                    "tool_id": "tool-1", "name": "computer_use",
                    "args": {"action": "click"}, "summary": "clicked"
                }
            }
        }));
        assert_eq!(computer.len(), 2);
        assert_eq!(computer[0].event.as_ref().unwrap().0, "tool");
        assert!(computer[0].event.as_ref().unwrap().2.contains("action"));
        assert_eq!(computer[1].event.as_ref().unwrap().0, "computer");
        let metadata: Value = serde_json::from_str(&computer[1].event.as_ref().unwrap().2).unwrap();
        assert_eq!(metadata["id"], "tool-1");
        assert_eq!(metadata["phase"], "completed");
        assert_eq!(metadata["tool"], "computer_use");
        assert_eq!(metadata["summary"], "clicked");

        let goal = parse_event(&serde_json::json!({
            "type": "status", "session_id": "session-123", "status_kind": "goal",
            "detail": {"text": "Goal active"}
        }));
        assert_eq!(goal[0].event.as_ref().unwrap().0, "goal");
        assert_eq!(goal[0].event.as_ref().unwrap().1, "Hermes goal");

        let denial = parse_event(&serde_json::json!({
            "type": "permission", "session_id": "session-123", "request": "approval",
            "decision": "denied", "detail": {"command": "rm -rf example"}
        }));
        assert!(!denial[0].failed);
        assert_eq!(denial[0].event.as_ref().unwrap().0, "error");
    }
}
