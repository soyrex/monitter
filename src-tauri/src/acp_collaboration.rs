//! Scoped collaboration configuration for ACP stdio sessions.
//!
//! The token travels only in the ACP session's stdio MCP configuration. It is
//! never put in a launch argument, saved in a task, or included in a
//! diagnostic.

use crate::collaboration_transport::SessionGrant;
use serde_json::{json, Value};

pub(crate) fn mcp_servers(helper: Option<&str>, grant: Option<&SessionGrant>) -> Value {
    match (helper, grant) {
        (Some(helper), Some(grant)) => json!([{
            "name": "monitter",
            "command": "python3",
            "args": [helper],
            "env": [
                {"name": "MONITTER_ENDPOINT", "value": grant.endpoint},
                {"name": "MONITTER_TOKEN", "value": grant.token},
            ]
        }]),
        _ => json!([]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stdio_server_keeps_grant_out_of_arguments_and_disabled_is_empty() {
        let grant = SessionGrant {
            endpoint: "http://127.0.0.1:4444/rpc".into(),
            token: "private-token".into(),
        };
        assert_eq!(mcp_servers(None, None), json!([]));
        let servers = mcp_servers(Some("/private/helper.py"), Some(&grant));
        assert_eq!(
            servers.pointer("/0/command").and_then(Value::as_str),
            Some("python3")
        );
        assert_eq!(
            servers.pointer("/0/args/0").and_then(Value::as_str),
            Some("/private/helper.py")
        );
        assert_eq!(
            servers.pointer("/0/env/1/value").and_then(Value::as_str),
            Some("private-token")
        );
    }
}
