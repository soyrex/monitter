//! Scoped collaboration configuration for ACP sessions.
//!
//! The token travels only in the ACP session's HTTP MCP headers. It is never
//! put in a URL or launch argument, saved in a task, or included in a
//! diagnostic.

use crate::collaboration_transport::SessionGrant;
use serde_json::{json, Value};

pub(crate) fn mcp_servers(grant: Option<&SessionGrant>) -> Value {
    match grant {
        Some(grant) => json!([{
            "type": "http",
            "name": "monitter",
            "url": grant.endpoint,
            "headers": [{"name": "Authorization", "value": format!("Bearer {}", grant.token)}]
        }]),
        None => json!([]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_server_keeps_grant_in_header_and_disabled_is_empty() {
        let grant = SessionGrant {
            endpoint: "http://127.0.0.1:4444/mcp".into(),
            token: "private-token".into(),
        };
        assert_eq!(mcp_servers(None), json!([]));
        let servers = mcp_servers(Some(&grant));
        assert_eq!(
            servers.pointer("/0/type").and_then(Value::as_str),
            Some("http")
        );
        assert_eq!(
            servers.pointer("/0/url").and_then(Value::as_str),
            Some("http://127.0.0.1:4444/mcp")
        );
        assert_eq!(
            servers
                .pointer("/0/headers/0/value")
                .and_then(Value::as_str),
            Some("Bearer private-token")
        );
        assert!(!servers.to_string().contains("command"));
    }
}
