//! Confirm lost acknowledgements using authority reads; never replay a mutation.
use serde_json::Value;

pub const RECONNECT_EXHAUSTED: &str = "Could not reconnect. Your draft is kept; check the chat after reconnecting before trying again.";
/// Read-only acknowledgement recovery tolerates a brief backend restart.
pub const READ_RETRY_DELAYS_MS: [u64; 8] = [0, 150, 450, 1_000, 2_000, 4_000, 8_000, 8_000];

pub fn transient(error: &str) -> bool {
    error == RECONNECT_EXHAUSTED || matches!(error,
        "Private IPC connection lost; delivery may be ambiguous. Refresh state before retrying a mutation."
        | "Private IPC response incomplete; delivery may be ambiguous. Refresh state before retrying a mutation."
        | "Private IPC connection unavailable. Reconnect and try again."
        | "Private IPC replay capacity is temporarily full; no command was dispatched."
        | "Private IPC replay receipt expired or backend changed. Refresh state before retrying."
        | "Private IPC replay reservation expired; no command was dispatched."
        | "Private IPC command completed but its replay receipt is too large. Refresh state before retrying."
        | "Private IPC command remains in flight. Refresh state before retrying."
        | "Private IPC is temporarily unavailable after 3 attempts. Refresh or reconnect, then try again.")
}

/// Transport recovery stays quiet; backend validation and authentication errors stay visible.
pub fn visible_error(error: &str) -> Option<&str> {
    (!transient(error) && !matches!(error,
        "Could not reconnect to Monitter. Your agents may still be running; try again shortly."
        | "Reconnected, but could not confirm the change. Check the chat before trying again. Your draft is kept."
    )).then_some(error)
}

fn rows<'a>(snapshot: &'a Value, key: &str) -> impl Iterator<Item = &'a Value> {
    snapshot[key].as_array().into_iter().flatten()
}

pub fn confirmed(command: &str, args: &Value, before: &Value, after: &Value) -> bool {
    match command {
        "send_message" => {
            let ids = args["attachmentIds"].as_array().cloned().unwrap_or_default();
            let matches = |row: &&Value, queue: bool| {
                let key = if queue { "queuedMessages" } else { "messages" };
                row["taskId"] == args["taskId"] && row["text"] == args["text"]
                    && row["id"].as_str().is_some()
                    && before["recoveryStartedAt"].as_i64().is_none_or(|started|
                        row["createdAt"].as_i64().is_some_and(|created| created >= started))
                    && !rows(before, key).any(|old| old["id"] == row["id"])
                    && if queue {
                        row["attachmentIds"].as_array().cloned().unwrap_or_default() == ids
                            && row["senderAgentId"].is_null()
                    } else {
                        row["role"] == "user"
                            && rows(row, "attachments").map(|a| a["id"].clone()).collect::<Vec<_>>() == ids
                    }
            };
            rows(after, "messages").filter(|row| matches(row, false)).count()
                + rows(after, "queuedMessages").filter(|row| matches(row, true)).count() == 1
        }
        "edit_queued_message" => rows(after, "queuedMessages").any(|row|
            row["id"] == args["id"] && row["text"] == args["text"]),
        // A missing queued item may have been delivered, so absence alone does
        // not confirm cancellation.
        "resolve_input" | "resolve_approval" | "resolve_mcp_approval" => rows(after, "approvalRequests").any(|row|
            row["id"] == args["approvalId"] && matches!(row["status"].as_str(), Some("approved" | "denied"))
                && if command == "resolve_input" { row["response"] == args["response"] }
                   else { row["decision"] == args["decision"] }),
        "cancel_task" => rows(after, "tasks").any(|row| row["id"] == args["taskId"]
            && matches!(row["status"].as_str(), Some("interrupted" | "idle" | "completed"))),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn presentation_hides_only_exact_transport_failures() {
        assert!(visible_error("Private IPC connection lost; delivery may be ambiguous. Refresh state before retrying a mutation.").is_none());
        assert!(visible_error("Private IPC connection unavailable. Reconnect and try again.").is_none());
        assert!(visible_error("Could not reconnect to Monitter. Your agents may still be running; try again shortly.").is_none());
        assert!(visible_error(RECONNECT_EXHAUSTED).is_none());
        assert!(visible_error("Private IPC command completed but its replay receipt is too large. Refresh state before retrying.").is_none());
        assert!(visible_error("Private IPC replay capacity is temporarily full; no command was dispatched.").is_none());
        assert!(transient(RECONNECT_EXHAUSTED));
        assert!(visible_error("Reconnected, but could not confirm the change. Check the chat before trying again. Your draft is kept.").is_none());
        for error in ["Private IPC connection lost inside a backend validation error", "Task is active", "Authentication required", "Attachment no longer exists", "Could not reconnect while validating this draft"] {
            assert_eq!(visible_error(error), Some(error));
        }
    }
    #[test]
    fn send_requires_new_exact_authoritative_message() {
        let args = json!({"taskId":"t", "text":"hello", "attachmentIds":["a"]});
        let old = json!({"messages":[{"id":"old","taskId":"t","role":"user","text":"hello","attachments":[{"id":"a"}]}]});
        assert!(!confirmed("send_message", &args, &old, &old));
        let mut after = old.clone();
        after["messages"][0]["id"] = json!("new");
        assert!(confirmed("send_message", &args, &old, &after));
        after["messages"][0]["attachments"] = json!([]);
        assert!(!confirmed("send_message", &args, &old, &after));
    }
    #[test]
    fn queued_send_and_semantic_errors_are_distinct() {
        let args = json!({"taskId":"t","text":"hello","attachmentIds":[]});
        let after = json!({"queuedMessages":[{"id":"q","taskId":"t","text":"hello","attachmentIds":[]}]});
        assert!(confirmed("send_message", &args, &json!({}), &after));
        assert!(!transient("Private IPC connection lost inside a backend validation error"));
        assert!(!confirmed("unknown_mutation", &args, &json!({}), &after));
    }
    #[test]
    fn ambiguity_and_different_decisions_are_not_acknowledgements() {
        let args = json!({"taskId":"t","text":"hello","attachmentIds":[]});
        let after = json!({"messages":[
            {"id":"a","taskId":"t","role":"user","text":"hello"},
            {"id":"b","taskId":"t","role":"user","text":"hello"}
        ]});
        assert!(!confirmed("send_message", &args, &json!({}), &after));
        let decision = json!({"approvalId":"a","decision":"deny"});
        let approved = json!({"approvalRequests":[{"id":"a","status":"approved","decision":"approve_once"}]});
        assert!(!confirmed("resolve_approval", &decision, &json!({}), &approved));
        let denied = json!({"approvalRequests":[{"id":"a","status":"denied","decision":"deny"}]});
        assert!(confirmed("resolve_approval", &decision, &json!({}), &denied));
        assert!(!confirmed("cancel_queued_message", &json!({"id":"q"}),
            &json!({"queuedMessages":[{"id":"q"}]}), &json!({})));
    }
    #[test]
    fn unsynced_older_prompt_cannot_clear_the_current_draft() {
        let args = json!({"taskId":"t","text":"hello","attachmentIds":[]});
        let before = json!({"recoveryStartedAt":100});
        let mut after = json!({"messages":[{"id":"older-unloaded","taskId":"t","role":"user","text":"hello","createdAt":99}]});
        assert!(!confirmed("send_message", &args, &before, &after));
        after["messages"][0]["createdAt"] = json!(101);
        assert!(confirmed("send_message", &args, &before, &after));
    }
}
